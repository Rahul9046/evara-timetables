//! Repositories for the time half of the Structure group, and the timeslot materialiser.
//!
//! Cycle, cycle day, period structure, period, the materialised `timeslot` grid, and the
//! `calendar_day` projection of real dates onto cycle days. The static half — places and
//! the coarse academic calendar — is [`super::structure`].
//!
//! # The model knows nothing about weekdays
//!
//! [`CycleDay::ordinal`] is the whole of a day's scheduling identity. `label` ("Monday",
//! "Day A", "Day 1") is a display string and `weekday_hint` is documented as display only.
//! Nothing here reads either of them for ordering, identity or materialisation, so a
//! one-day cycle, a six-day rotation and a ten-day fortnight are the same code path with a
//! different row count.
//!
//! # Materialisation
//!
//! The rules are set out in
//! [ADR 0011](../../../docs/adr/0011-stable-timeslot-identity.md); the short version is
//! that a timeslot's logical identity is `(term_id, cycle_day_id, period_id)`, its `id` is
//! assigned once and never reassigned while that tuple exists, and nothing in this module
//! deletes a timeslot except [`TimeModel::delete_timeslots`], which has to be called
//! deliberately with explicit identifiers.
//!
//! Shape follows [`super::structure`] deliberately. Every write goes through
//! [`super::update`] or [`super::delete`], which own `rev` and `updated_at`.

use std::collections::HashMap;

use rusqlite::types::{ToSql, ToSqlOutput};
use rusqlite::{Row, Transaction};
use uuid::Uuid;

use super::row::{FromColumn, entity, opt_text, text};
use super::{Stamp, delete, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing the time model.
///
/// Obtained from [`Database::time_model`]. Holds the database mutably because most
/// operations write.
#[derive(Debug)]
pub struct TimeModel<'a> {
    db: &'a mut Database,
}

impl<'a> TimeModel<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

// =============================================================================== kinds

/// What a period is for.
///
/// Typed rather than stringly, because `is_teaching` on the materialised grid is derived
/// from it and a mistyped comparison would silently mark a whole column of the timetable
/// unschedulable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeriodKind {
    /// Lessons may be scheduled into it.
    Teaching,
    /// Break, lunch, recess.
    Break,
    /// Form time, roll call, assembly.
    Registration,
    /// Anything else the school names.
    Other,
}

/// What a calendar date is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalendarDayKind {
    /// A normal teaching day.
    School,
    /// Closed.
    Holiday,
    /// Examinations.
    Exam,
    /// A whole-school event.
    Event,
    /// Staff professional development.
    ProfessionalDevelopment,
}

/// Declares the stored spelling of an enum, both directions, in one place.
///
/// The spellings are the `CHECK` constraint's own values, so drift between the schema and
/// the Rust type would be a compile-time-visible edit to one list rather than two.
macro_rules! stored_as {
    ($ty:ty, $entity:expr, { $($variant:path => $text:literal),* $(,)? }) => {
        impl $ty {
            /// The value as stored.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $($variant => $text,)* }
            }

            fn parse(raw: &str) -> Option<Self> {
                match raw { $($text => Some($variant),)* _ => None }
            }
        }

        impl ToSql for $ty {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.as_str()))
            }
        }

        impl FromColumn for $ty {
            fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self> {
                let raw: String = row.get(index)?;
                // Unreachable behind the table's CHECK constraint, which permits exactly
                // these spellings. Still an error rather than a default: guessing which
                // kind a damaged row meant is how a break period becomes teachable.
                Self::parse(&raw).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        rusqlite::types::Type::Text,
                        format!("`{raw}` is not a known {} kind", $entity).into(),
                    )
                })
            }
        }
    };
}

stored_as!(PeriodKind, "period", {
    PeriodKind::Teaching => "TEACHING",
    PeriodKind::Break => "BREAK",
    PeriodKind::Registration => "REGISTRATION",
    PeriodKind::Other => "OTHER",
});

stored_as!(CalendarDayKind, "calendar day", {
    CalendarDayKind::School => "SCHOOL",
    CalendarDayKind::Holiday => "HOLIDAY",
    CalendarDayKind::Exam => "EXAM",
    CalendarDayKind::Event => "EVENT",
    CalendarDayKind::ProfessionalDevelopment => "PD",
});

// ============================================================================ entities

entity! {
    /// The repeating pattern a timetable is defined over.
    Cycle,
    /// Values for creating or replacing a cycle.
    CycleInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name, unique within the school.
        name: String,
        /// Declared length in days: 5 weekly, 10 for a fortnight, 6 for a rotation.
        ///
        /// The *declared* length. The materialiser uses the cycle days that exist, and a
        /// partly built cycle is legal — see ADR 0011 Q4.
        day_count: i64,
        /// How many calendar weeks the pattern spans, for display and grouping.
        week_count: i64,
        /// Whether this is the school's default cycle. At most one may be.
        is_default: bool,
    }
}

entity! {
    /// One day of the repeating pattern.
    CycleDay,
    /// Values for creating or replacing a cycle day.
    CycleDayInput,
    {
        /// Owning cycle.
        cycle_id: Uuid,
        /// Position in the cycle, from 1. **This is the day's identity.**
        ordinal: i64,
        /// What a human reads: "Monday", "Day A", "Day 1". Unique within the cycle.
        label: String,
        /// 1..7, for display only. Never read by scheduling, ordering or materialisation.
        weekday_hint: Option<i64>,
    }
}

entity! {
    /// A bell schedule.
    PeriodStructure,
    /// Values for creating or replacing a period structure.
    PeriodStructureInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Campus it applies to, or `None` for school-wide.
        campus_id: Option<Uuid>,
        /// Display name, unique within the school.
        name: String,
        /// Whether this is the default for its campus. At most one per campus may be.
        is_default: bool,
    }
}

entity! {
    /// One period of a bell schedule.
    Period,
    /// Values for creating or replacing a period.
    PeriodInput,
    {
        /// Owning period structure.
        period_structure_id: Uuid,
        /// Position in the day, from 1. **This is the period's identity.**
        ordinal: i64,
        /// What a human reads: "1", "P3", "Registration". Unique within the structure.
        label: String,
        /// Local wall-clock start, `HH:MM`.
        starts_at: String,
        /// Local wall-clock end, `HH:MM`. Must be after `starts_at`.
        ends_at: String,
        /// What the period is for.
        kind: PeriodKind,
        /// Whether it counts towards a teacher's load.
        counts_as_load: bool,
    }
}

entity! {
    /// One cell of the materialised grid: the solver's value range.
    ///
    /// Created only by [`TimeModel::apply_materialisation`]. `school_id`, `cycle_id`,
    /// `campus_id`, `ordinal` and `is_teaching` are derived projections, refreshed on
    /// rematerialisation; only `id` is stable, and only `(term_id, cycle_day_id,
    /// period_id)` decides which slot a row is.
    Timeslot,
    /// The insert payload the materialiser builds.
    ///
    /// There is deliberately no `create_timeslot`: a slot that did not come from
    /// materialisation would have no logical identity to preserve.
    TimeslotInput,
    {
        /// Owning school. Derived from the cycle.
        school_id: Uuid,
        /// Owning cycle. Derived from the cycle day.
        cycle_id: Uuid,
        /// Which day of the cycle. **Identity.**
        cycle_day_id: Uuid,
        /// Which period of the bell schedule. **Identity.**
        period_id: Uuid,
        /// Campus, derived from the period structure. `None` for school-wide.
        campus_id: Option<Uuid>,
        /// Term the grid belongs to, or `None` for the whole year. **Identity.**
        term_id: Option<Uuid>,
        /// Global order within the cycle, from 1. Derived, and expected to change.
        ordinal: i64,
        /// Whether lessons may be scheduled here. Derived from the period's kind.
        is_teaching: bool,
    }
}

entity! {
    /// A real date, and which cycle day it is.
    CalendarDay,
    /// Values for creating or replacing a calendar day.
    CalendarDayInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// The date, `YYYY-MM-DD`.
        date: String,
        /// Term the date falls in, when known.
        term_id: Option<Uuid>,
        /// Campus this entry is for, or `None` for the whole school.
        campus_id: Option<Uuid>,
        /// Which cycle day the date is. `None` means no lessons that day.
        cycle_day_id: Option<Uuid>,
        /// What kind of day it is.
        kind: CalendarDayKind,
        /// Free text.
        note: Option<String>,
    }
}

// =============================================================================== cycle

const CYCLE_COLUMNS: &str =
    "id, school_id, name, day_count, week_count, is_default, created_at, updated_at, rev";

impl TimeModel<'_> {
    /// Creates a cycle.
    ///
    /// Marking it default clears any existing default for the school in the same
    /// transaction, with a proper revision bump, so the schema's single-default index is a
    /// backstop that never fires in normal operation (risk R10).
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if the name is taken in that school.
    /// - [`DbError::Invalid`] if `day_count` or `week_count` is below 1.
    pub fn create_cycle(&mut self, input: &CycleInput) -> Result<Cycle> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            if input.is_default {
                clear_default_cycle(tx, input.school_id)?;
            }
            tx.execute(
                "INSERT INTO cycle
                     (id, school_id, name, day_count, week_count, is_default,
                      created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    input.day_count,
                    input.week_count,
                    input.is_default,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "cycle"))?;
            Ok(())
        })?;
        self.cycle(stamp.id)?
            .ok_or(DbError::NotFound { entity: "cycle" })
    }

    /// One cycle by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn cycle(&self, id: Uuid) -> Result<Option<Cycle>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {CYCLE_COLUMNS} FROM cycle WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Cycle::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every cycle, by name then identifier so the order never varies.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn cycles(&self) -> Result<Vec<Cycle>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CYCLE_COLUMNS} FROM cycle ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], Cycle::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a cycle's details. Its school never changes.
    ///
    /// Shortening `day_count` below an existing cycle day's ordinal is refused: the days
    /// are the reality, and silently leaving a day outside the declared length would make
    /// the two disagree with nothing noticing.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_cycle(&mut self, id: Uuid, input: &CycleInput) -> Result<Cycle> {
        self.db.write(|tx| {
            let highest = highest_cycle_day_ordinal(tx, id)?;
            if highest > input.day_count {
                return Err(DbError::Invalid {
                    message: format!(
                        "this cycle already has a day {highest}, so its length cannot be \
                         reduced to {}",
                        input.day_count
                    ),
                });
            }
            if input.is_default {
                clear_default_cycle(tx, input.school_id)?;
            }
            update(
                tx,
                "cycle",
                "cycle",
                id,
                "name = ?, day_count = ?, week_count = ?, is_default = ?",
                &[
                    &input.name,
                    &input.day_count,
                    &input.week_count,
                    &input.is_default,
                ],
            )?;
            Ok(())
        })?;
        self.cycle(id)?.ok_or(DbError::NotFound { entity: "cycle" })
    }

    /// Deletes a cycle.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if it still has cycle days or timeslots. Ownership is
    /// RESTRICT: the user removes those deliberately.
    pub fn delete_cycle(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "cycle", "cycle", id))
    }
}

/// Clears the school's current default cycle through the revision contract.
fn clear_default_cycle(tx: &Transaction<'_>, school_id: Uuid) -> Result<()> {
    for id in default_ids(
        tx,
        "SELECT id FROM cycle WHERE school_id = ? AND is_default = 1",
        &[&text(school_id)],
    )? {
        update(tx, "cycle", "cycle", id, "is_default = ?", &[&false])?;
    }
    Ok(())
}

/// The largest ordinal among a cycle's days, or 0 when it has none.
fn highest_cycle_day_ordinal(tx: &Transaction<'_>, cycle_id: Uuid) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT COALESCE(MAX(ordinal), 0) FROM cycle_day WHERE cycle_id = ?",
        [text(cycle_id)],
        |row| row.get(0),
    )?)
}

/// Reads identifiers for a "which row is currently the default" query.
fn default_ids(tx: &Transaction<'_>, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare(sql)?;
    let raw: Vec<String> = stmt
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.iter()
        .map(|value| super::row::parse_id_text("id", value))
        .collect()
}

// =========================================================================== cycle day

const CYCLE_DAY_COLUMNS: &str =
    "id, cycle_id, ordinal, label, weekday_hint, created_at, updated_at, rev";

impl TimeModel<'_> {
    /// Adds a day to a cycle.
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] if the ordinal exceeds the cycle's declared `day_count`.
    /// - [`DbError::DuplicateValue`] if that ordinal or label is already used in the cycle.
    /// - [`DbError::StillReferenced`] if the cycle does not exist.
    pub fn create_cycle_day(&mut self, input: &CycleDayInput) -> Result<CycleDay> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            check_ordinal_within_cycle(tx, input.cycle_id, input.ordinal)?;
            tx.execute(
                "INSERT INTO cycle_day
                     (id, cycle_id, ordinal, label, weekday_hint, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.cycle_id),
                    input.ordinal,
                    &input.label,
                    input.weekday_hint,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "cycle day"))?;
            Ok(())
        })?;
        self.cycle_day(stamp.id)?.ok_or(DbError::NotFound {
            entity: "cycle day",
        })
    }

    /// One cycle day by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn cycle_day(&self, id: Uuid) -> Result<Option<CycleDay>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CYCLE_DAY_COLUMNS} FROM cycle_day WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], CycleDay::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// A cycle's days, in cycle order.
    ///
    /// Ordered by `ordinal` alone, which is unique within the cycle, so the order is
    /// total and deterministic without a tiebreak.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn cycle_days(&self, cycle_id: Uuid) -> Result<Vec<CycleDay>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CYCLE_DAY_COLUMNS} FROM cycle_day WHERE cycle_id = ? ORDER BY ordinal"
            ))?;
            let rows = stmt.query_map([text(cycle_id)], CycleDay::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a cycle day's details. Its cycle never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_cycle_day(&mut self, id: Uuid, input: &CycleDayInput) -> Result<CycleDay> {
        self.db.write(|tx| {
            check_ordinal_within_cycle(tx, input.cycle_id, input.ordinal)?;
            update(
                tx,
                "cycle day",
                "cycle_day",
                id,
                "ordinal = ?, label = ?, weekday_hint = ?",
                &[&input.ordinal, &input.label, &input.weekday_hint],
            )?;
            Ok(())
        })?;
        self.cycle_day(id)?.ok_or(DbError::NotFound {
            entity: "cycle day",
        })
    }

    /// Deletes a cycle day.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if a timeslot or a calendar day still points at it.
    /// That is deliberate: see [`TimeModel::plan_materialisation`] for how to find out
    /// what is in the way before trying.
    pub fn delete_cycle_day(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "cycle day", "cycle_day", id))
    }
}

/// Refuses an ordinal outside the cycle's declared length.
///
/// ADR 0011 Q4: `day_count` is the declared length and the rows need not be complete, but
/// a day beyond the declared end is a contradiction rather than an intermediate state.
fn check_ordinal_within_cycle(tx: &Transaction<'_>, cycle_id: Uuid, ordinal: i64) -> Result<()> {
    let declared: Option<i64> = tx
        .query_row(
            "SELECT day_count FROM cycle WHERE id = ?",
            [text(cycle_id)],
            |row| row.get(0),
        )
        .ok();

    // A missing cycle is left to the foreign key, which reports it as a reference failure.
    if let Some(declared) = declared
        && ordinal > declared
    {
        return Err(DbError::Invalid {
            message: format!("this cycle is {declared} days long, so it has no day {ordinal}"),
        });
    }
    Ok(())
}

// ==================================================================== period structure

const PERIOD_STRUCTURE_COLUMNS: &str =
    "id, school_id, campus_id, name, is_default, created_at, updated_at, rev";

impl TimeModel<'_> {
    /// Creates a period structure.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name is taken in that school.
    pub fn create_period_structure(
        &mut self,
        input: &PeriodStructureInput,
    ) -> Result<PeriodStructure> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            if input.is_default {
                clear_default_period_structure(tx, input.school_id, input.campus_id)?;
            }
            tx.execute(
                "INSERT INTO period_structure
                     (id, school_id, campus_id, name, is_default, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    opt_text(input.campus_id),
                    &input.name,
                    input.is_default,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "period structure"))?;
            Ok(())
        })?;
        self.period_structure(stamp.id)?.ok_or(DbError::NotFound {
            entity: "period structure",
        })
    }

    /// One period structure by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn period_structure(&self, id: Uuid) -> Result<Option<PeriodStructure>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {PERIOD_STRUCTURE_COLUMNS} FROM period_structure WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], PeriodStructure::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every period structure, by name then identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn period_structures(&self) -> Result<Vec<PeriodStructure>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {PERIOD_STRUCTURE_COLUMNS} FROM period_structure ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], PeriodStructure::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a period structure's details. Its school never changes.
    ///
    /// Moving it to another campus changes the `campus_id` of every slot materialised from
    /// it, which the next rematerialisation refreshes; the slots keep their identifiers.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_period_structure(
        &mut self,
        id: Uuid,
        input: &PeriodStructureInput,
    ) -> Result<PeriodStructure> {
        self.db.write(|tx| {
            if input.is_default {
                clear_default_period_structure(tx, input.school_id, input.campus_id)?;
            }
            update(
                tx,
                "period structure",
                "period_structure",
                id,
                "campus_id = ?, name = ?, is_default = ?",
                &[&opt_text(input.campus_id), &input.name, &input.is_default],
            )?;
            Ok(())
        })?;
        self.period_structure(id)?.ok_or(DbError::NotFound {
            entity: "period structure",
        })
    }

    /// Deletes a period structure.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if it still has periods.
    pub fn delete_period_structure(&mut self, id: Uuid) -> Result<()> {
        self.db
            .write(|tx| delete(tx, "period structure", "period_structure", id))
    }
}

/// Clears the current default structure for one campus bucket.
///
/// `IS` rather than `=`, so the school-wide rows (`campus_id IS NULL`) form their own
/// bucket exactly as the unique index's `COALESCE` does.
fn clear_default_period_structure(
    tx: &Transaction<'_>,
    school_id: Uuid,
    campus_id: Option<Uuid>,
) -> Result<()> {
    for id in default_ids(
        tx,
        "SELECT id FROM period_structure
         WHERE school_id = ? AND campus_id IS ? AND is_default = 1",
        &[&text(school_id), &opt_text(campus_id)],
    )? {
        update(
            tx,
            "period structure",
            "period_structure",
            id,
            "is_default = ?",
            &[&false],
        )?;
    }
    Ok(())
}

// ============================================================================== period

const PERIOD_COLUMNS: &str = "id, period_structure_id, ordinal, label, starts_at, ends_at, \
                              kind, counts_as_load, created_at, updated_at, rev";

impl TimeModel<'_> {
    /// Adds a period to a bell schedule.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if that ordinal or label is already used.
    /// - [`DbError::Invalid`] if the times are malformed or end before they start.
    /// - [`DbError::StillReferenced`] if the period structure does not exist.
    pub fn create_period(&mut self, input: &PeriodInput) -> Result<Period> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO period
                     (id, period_structure_id, ordinal, label, starts_at, ends_at, kind,
                      counts_as_load, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.period_structure_id),
                    input.ordinal,
                    &input.label,
                    &input.starts_at,
                    &input.ends_at,
                    input.kind,
                    input.counts_as_load,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "period"))?;
            Ok(())
        })?;
        self.period(stamp.id)?
            .ok_or(DbError::NotFound { entity: "period" })
    }

    /// One period by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn period(&self, id: Uuid) -> Result<Option<Period>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {PERIOD_COLUMNS} FROM period WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Period::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// A bell schedule's periods, in order.
    ///
    /// Ordered by `ordinal` alone, which is unique within the structure.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn periods(&self, period_structure_id: Uuid) -> Result<Vec<Period>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {PERIOD_COLUMNS} FROM period
                 WHERE period_structure_id = ? ORDER BY ordinal"
            ))?;
            let rows = stmt.query_map([text(period_structure_id)], Period::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a period's details. Its structure never changes.
    ///
    /// Changing `kind` changes `is_teaching` on every slot built from it, which the next
    /// rematerialisation refreshes without disturbing any identifier.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_period(&mut self, id: Uuid, input: &PeriodInput) -> Result<Period> {
        self.db.write(|tx| {
            update(
                tx,
                "period",
                "period",
                id,
                "ordinal = ?, label = ?, starts_at = ?, ends_at = ?, kind = ?, \
                 counts_as_load = ?",
                &[
                    &input.ordinal,
                    &input.label,
                    &input.starts_at,
                    &input.ends_at,
                    &input.kind,
                    &input.counts_as_load,
                ],
            )?;
            Ok(())
        })?;
        self.period(id)?
            .ok_or(DbError::NotFound { entity: "period" })
    }

    /// Deletes a period.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if a timeslot was materialised from it. Use
    /// [`TimeModel::plan_materialisation`] with the period excluded to see exactly which
    /// slots stand in the way before removing anything.
    pub fn delete_period(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "period", "period", id))
    }
}

// ======================================================================== calendar day

const CALENDAR_DAY_COLUMNS: &str = "id, school_id, date, term_id, campus_id, cycle_day_id, \
                                    kind, note, created_at, updated_at, rev";

impl TimeModel<'_> {
    /// Records what one real date is.
    ///
    /// This is the only bridge between dates and the scheduling model, and it is an
    /// explicit, stored mapping. Phase 1D does not generate it: what Day follows a
    /// holiday, and whether a closure consumes a rotation slot, is policy DATA-MODEL.md
    /// does not state, and a guess would be indistinguishable from a decision.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if that date is already recorded for that campus.
    /// - [`DbError::Invalid`] if the date is not a real `YYYY-MM-DD`.
    /// - [`DbError::StillReferenced`] if the school, term, campus or cycle day is unknown.
    pub fn create_calendar_day(&mut self, input: &CalendarDayInput) -> Result<CalendarDay> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO calendar_day
                     (id, school_id, date, term_id, campus_id, cycle_day_id, kind, note,
                      created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.date,
                    opt_text(input.term_id),
                    opt_text(input.campus_id),
                    opt_text(input.cycle_day_id),
                    input.kind,
                    &input.note,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "calendar day"))?;
            Ok(())
        })?;
        self.calendar_day(stamp.id)?.ok_or(DbError::NotFound {
            entity: "calendar day",
        })
    }

    /// One calendar day by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn calendar_day(&self, id: Uuid) -> Result<Option<CalendarDay>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CALENDAR_DAY_COLUMNS} FROM calendar_day WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], CalendarDay::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Calendar days in a date range, inclusive, in date order.
    ///
    /// Text comparison is correct here because the column is a canonical `YYYY-MM-DD`,
    /// enforced by a `CHECK`.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn calendar_days(&self, from: &str, to: &str) -> Result<Vec<CalendarDay>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CALENDAR_DAY_COLUMNS} FROM calendar_day
                 WHERE date BETWEEN ? AND ? ORDER BY date, COALESCE(campus_id, ''), id"
            ))?;
            let rows = stmt.query_map([from, to], CalendarDay::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a calendar day's details. Its school never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_calendar_day(
        &mut self,
        id: Uuid,
        input: &CalendarDayInput,
    ) -> Result<CalendarDay> {
        self.db.write(|tx| {
            update(
                tx,
                "calendar day",
                "calendar_day",
                id,
                "date = ?, term_id = ?, campus_id = ?, cycle_day_id = ?, kind = ?, note = ?",
                &[
                    &input.date,
                    &opt_text(input.term_id),
                    &opt_text(input.campus_id),
                    &opt_text(input.cycle_day_id),
                    &input.kind,
                    &input.note,
                ],
            )?;
            Ok(())
        })?;
        self.calendar_day(id)?.ok_or(DbError::NotFound {
            entity: "calendar day",
        })
    }

    /// Deletes a calendar day.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn delete_calendar_day(&mut self, id: Uuid) -> Result<()> {
        self.db
            .write(|tx| delete(tx, "calendar day", "calendar_day", id))
    }
}

// ===================================================================== materialisation

const TIMESLOT_COLUMNS: &str = "id, school_id, cycle_id, cycle_day_id, period_id, campus_id, \
                                term_id, ordinal, is_teaching, created_at, updated_at, rev";

/// Which grid to materialise, and what configuration to materialise it against.
///
/// With both exclusion lists empty — the normal case — the desired grid is whatever the
/// cycle and the period structure currently say. The exclusions exist because
/// `timeslot.cycle_day_id` and `timeslot.period_id` are `RESTRICT`: a materialised grid
/// blocks the deletion of a day or a period, so an orphan can never be found by deleting
/// first and rematerialising afterwards. Orphan-hood is a property of a *proposal*, and
/// this struct is how a caller states one. See ADR 0011 §4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialisationRequest {
    /// The cycle whose days form one axis of the grid.
    pub cycle_id: Uuid,
    /// The bell schedule whose periods form the other axis.
    pub period_structure_id: Uuid,
    /// The term the grid belongs to, or `None` for the whole year.
    pub term_id: Option<Uuid>,
    /// Cycle days to leave out of the desired grid, to preview their removal.
    pub without_cycle_days: Vec<Uuid>,
    /// Periods to leave out of the desired grid, to preview their removal.
    pub without_periods: Vec<Uuid>,
}

impl MaterialisationRequest {
    /// A request for the grid exactly as the cycle and period structure currently stand.
    #[must_use]
    pub fn new(cycle_id: Uuid, period_structure_id: Uuid, term_id: Option<Uuid>) -> Self {
        Self {
            cycle_id,
            period_structure_id,
            term_id,
            without_cycle_days: Vec::new(),
            without_periods: Vec::new(),
        }
    }

    /// Asks what the grid would be if this cycle day were gone.
    #[must_use]
    pub fn without_cycle_day(mut self, id: Uuid) -> Self {
        self.without_cycle_days.push(id);
        self
    }

    /// Asks what the grid would be if this period were gone.
    #[must_use]
    pub fn without_period(mut self, id: Uuid) -> Self {
        self.without_periods.push(id);
        self
    }
}

/// One logical slot in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSlot {
    /// The slot's stable identifier.
    ///
    /// `None` only in [`MaterialisationPlan::created`] from
    /// [`TimeModel::plan_materialisation`], where the slot does not exist yet. After
    /// [`TimeModel::apply_materialisation`] every entry has one.
    pub id: Option<Uuid>,
    /// Identity: which day of the cycle.
    pub cycle_day_id: Uuid,
    /// Identity: which period.
    pub period_id: Uuid,
    /// Identity: which term, or `None` for the whole year.
    pub term_id: Option<Uuid>,
    /// Derived global order within the cycle. For an orphan, its order as stored.
    pub ordinal: i64,
}

/// How many slots a plan touches, for a confirmation prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialisationCounts {
    /// Slots that already exist and keep their identifiers.
    pub preserved: usize,
    /// Slots that would be, or were, created.
    pub created: usize,
    /// Slots the proposal no longer wants. Never touched.
    pub orphaned: usize,
    /// Preserved slots whose derived columns changed, so a revision was consumed.
    ///
    /// Always 0 from [`TimeModel::plan_materialisation`], which writes nothing.
    pub refreshed: usize,
}

/// What a rematerialisation would do, or did.
///
/// The orphan list is the point of the type: a caller can see that rematerialising would
/// strand slots *before* anything is removed, and nothing in this module removes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialisationPlan {
    /// Slots that already exist and are still wanted. Identifiers preserved.
    pub preserved: Vec<PlannedSlot>,
    /// Logical tuples the proposal wants that do not exist yet.
    pub created: Vec<PlannedSlot>,
    /// Slots that exist but the proposal no longer wants.
    pub orphaned: Vec<PlannedSlot>,
    /// Preserved slots whose derived columns were refreshed.
    pub refreshed: usize,
}

impl MaterialisationPlan {
    /// Whether applying this proposal would strand any slot.
    #[must_use]
    pub fn would_orphan(&self) -> bool {
        !self.orphaned.is_empty()
    }

    /// The counts, for a confirmation prompt.
    #[must_use]
    pub fn counts(&self) -> MaterialisationCounts {
        MaterialisationCounts {
            preserved: self.preserved.len(),
            created: self.created.len(),
            orphaned: self.orphaned.len(),
            refreshed: self.refreshed,
        }
    }

    /// The identifiers of the stranded slots, ready for
    /// [`TimeModel::delete_timeslots`] once a human has agreed.
    #[must_use]
    pub fn orphaned_ids(&self) -> Vec<Uuid> {
        self.orphaned.iter().filter_map(|slot| slot.id).collect()
    }
}

/// The derived values every slot in one grid shares.
struct GridContext {
    school_id: Uuid,
    campus_id: Option<Uuid>,
}

impl TimeModel<'_> {
    /// What rematerialising this grid would do. Writes nothing.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if the cycle, period structure or term does not exist, and
    /// [`DbError::Invalid`] if the cycle and the period structure belong to different
    /// schools.
    pub fn plan_materialisation(
        &mut self,
        request: &MaterialisationRequest,
    ) -> Result<MaterialisationPlan> {
        // A read, but it runs in a transaction so the several queries it makes see one
        // consistent snapshot of the grid.
        self.db.write(|tx| plan(tx, request))
    }

    /// Creates the slots this grid is missing and refreshes the ones it already has.
    ///
    /// Transactional, and **never deletes**. Orphans are reported in the returned plan and
    /// left exactly as they were; removing them is a separate, explicit call to
    /// [`TimeModel::delete_timeslots`].
    ///
    /// The plan is recomputed inside the write transaction rather than taken as an
    /// argument: a plan made earlier is a snapshot of a world that may have moved on, and
    /// applying a stale one would act on tuples that no longer describe anything.
    ///
    /// Idempotent. A second run with the same configuration finds every tuple present and
    /// every derived value correct, so it writes nothing at all and consumes no revisions.
    ///
    /// # Errors
    ///
    /// As [`TimeModel::plan_materialisation`], plus any constraint failure — in which case
    /// the whole rematerialisation is rolled back and the grid is exactly as it was.
    pub fn apply_materialisation(
        &mut self,
        request: &MaterialisationRequest,
    ) -> Result<MaterialisationPlan> {
        self.db.write(|tx| {
            let mut plan = plan(tx, request)?;
            let context = grid_context(tx, request)?;
            let teaching = teaching_periods(tx, request.period_structure_id)?;

            for slot in &mut plan.created {
                let stamp = Stamp::new();
                tx.execute(
                    "INSERT INTO timeslot
                         (id, school_id, cycle_id, cycle_day_id, period_id, campus_id,
                          term_id, ordinal, is_teaching, created_at, updated_at, rev)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        text(stamp.id),
                        text(context.school_id),
                        text(request.cycle_id),
                        text(slot.cycle_day_id),
                        text(slot.period_id),
                        opt_text(context.campus_id),
                        opt_text(slot.term_id),
                        slot.ordinal,
                        *teaching.get(&slot.period_id).unwrap_or(&false),
                        &stamp.created_at,
                        &stamp.updated_at,
                        stamp.rev,
                    ),
                )
                .map_err(|error| translate(error, "timeslot"))?;
                slot.id = Some(stamp.id);
            }

            plan.refreshed = refresh_preserved(tx, request, &context, &teaching, &plan.preserved)?;
            Ok(plan)
        })
    }

    /// Every slot of one grid, in cycle order.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn timeslots(&self, cycle_id: Uuid, term_id: Option<Uuid>) -> Result<Vec<Timeslot>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TIMESLOT_COLUMNS} FROM timeslot
                 WHERE cycle_id = ? AND term_id IS ? ORDER BY ordinal, id"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![text(cycle_id), opt_text(term_id)],
                Timeslot::from_row,
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// One timeslot by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn timeslot(&self, id: Uuid) -> Result<Option<Timeslot>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TIMESLOT_COLUMNS} FROM timeslot WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Timeslot::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Deletes the named timeslots. **The only destructive call in this module.**
    ///
    /// Intended to be given [`MaterialisationPlan::orphaned_ids`] after a human has agreed
    /// to lose them. It takes explicit identifiers rather than a grid, and it is a separate
    /// function from [`TimeModel::apply_materialisation`] rather than a flag on it, so
    /// destruction cannot be reached by accident.
    ///
    /// It does not verify orphan-hood, and does not pretend to: per ADR 0011 §4 that is a
    /// property of a proposal, not of a row.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if any identifier matches no slot.
    /// - [`DbError::StillReferenced`] if anything — a future timetable entry — refers to
    ///   one of them. That refusal is why timetable entries will never need a cascade.
    ///
    /// Either way the whole call is rolled back: no subset is deleted.
    pub fn delete_timeslots(&mut self, ids: &[Uuid]) -> Result<()> {
        self.db.write(|tx| {
            for id in ids {
                delete(tx, "timeslot", "timeslot", *id)?;
            }
            Ok(())
        })
    }
}

/// Computes the plan for one grid against one proposal.
fn plan(tx: &Transaction<'_>, request: &MaterialisationRequest) -> Result<MaterialisationPlan> {
    // For its validation, not its values: planning must report an unknown cycle, bell
    // schedule or term rather than quietly returning an empty grid.
    grid_context(tx, request)?;

    let days = desired_cycle_days(tx, request)?;
    let periods = desired_periods(tx, request)?;
    let mut existing = existing_slots(tx, request)?;

    let mut preserved = Vec::new();
    let mut created = Vec::new();
    let mut ordinal = 0i64;

    // Order is (cycle_day.ordinal, period.ordinal), which is what makes `ordinal`
    // deterministic for a given configuration.
    for day in &days {
        for period in &periods {
            ordinal += 1;
            let slot = PlannedSlot {
                id: None,
                cycle_day_id: *day,
                period_id: *period,
                term_id: request.term_id,
                ordinal,
            };
            match existing.remove(&(*day, *period)) {
                // Case A: the tuple survives, so its identifier survives with it.
                Some(found) => preserved.push(PlannedSlot {
                    id: Some(found.id),
                    ..slot
                }),
                // Case B.
                None => created.push(slot),
            }
        }
    }

    // Case C: whatever the proposal no longer wants. Reported, never touched.
    let mut orphaned: Vec<PlannedSlot> = existing
        .into_values()
        .map(|found| PlannedSlot {
            id: Some(found.id),
            cycle_day_id: found.cycle_day_id,
            period_id: found.period_id,
            term_id: found.term_id,
            ordinal: found.ordinal,
        })
        .collect();
    // A HashMap's drain order is arbitrary; the report must not be.
    orphaned.sort_by_key(|slot| (slot.ordinal, slot.id));

    Ok(MaterialisationPlan {
        preserved,
        created,
        orphaned,
        refreshed: 0,
    })
}

/// Resolves the derived values the whole grid shares, validating the request on the way.
fn grid_context(tx: &Transaction<'_>, request: &MaterialisationRequest) -> Result<GridContext> {
    let school_id: String = tx
        .query_row(
            "SELECT school_id FROM cycle WHERE id = ?",
            [text(request.cycle_id)],
            |row| row.get(0),
        )
        .map_err(|_| DbError::NotFound { entity: "cycle" })?;
    let school_id = super::row::parse_id_text("cycle.school_id", &school_id)?;

    let (structure_school, campus_id): (String, Option<String>) = tx
        .query_row(
            "SELECT school_id, campus_id FROM period_structure WHERE id = ?",
            [text(request.period_structure_id)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| DbError::NotFound {
            entity: "period structure",
        })?;
    let structure_school =
        super::row::parse_id_text("period_structure.school_id", &structure_school)?;

    if structure_school != school_id {
        return Err(DbError::Invalid {
            message: "a cycle and a bell schedule from different schools cannot form a grid"
                .to_owned(),
        });
    }

    if let Some(term_id) = request.term_id {
        let known: i64 = tx.query_row(
            "SELECT COUNT(*) FROM term WHERE id = ?",
            [text(term_id)],
            |row| row.get(0),
        )?;
        if known == 0 {
            return Err(DbError::NotFound { entity: "term" });
        }
    }

    let campus_id = match campus_id {
        Some(raw) => Some(super::row::parse_id_text(
            "period_structure.campus_id",
            &raw,
        )?),
        None => None,
    };

    Ok(GridContext {
        school_id,
        campus_id,
    })
}

/// The cycle days the proposal wants, in cycle order.
fn desired_cycle_days(tx: &Transaction<'_>, request: &MaterialisationRequest) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare("SELECT id FROM cycle_day WHERE cycle_id = ? ORDER BY ordinal")?;
    let ids = stmt
        .query_map([text(request.cycle_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    keep_wanted(&ids, &request.without_cycle_days, "cycle_day.id")
}

/// The periods the proposal wants, in bell order.
fn desired_periods(tx: &Transaction<'_>, request: &MaterialisationRequest) -> Result<Vec<Uuid>> {
    let mut stmt =
        tx.prepare("SELECT id FROM period WHERE period_structure_id = ? ORDER BY ordinal")?;
    let ids = stmt
        .query_map([text(request.period_structure_id)], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    keep_wanted(&ids, &request.without_periods, "period.id")
}

/// Parses stored identifiers and drops the ones the proposal excludes.
fn keep_wanted(raw: &[String], excluded: &[Uuid], column: &'static str) -> Result<Vec<Uuid>> {
    raw.iter()
        .map(|value| super::row::parse_id_text(column, value))
        .filter(|parsed| match parsed {
            Ok(id) => !excluded.contains(id),
            Err(_) => true,
        })
        .collect()
}

/// The slots already stored for this grid, keyed by the identity tuple's variable half.
///
/// `term_id` is fixed by the request, so the key is only `(cycle_day_id, period_id)`.
/// Scoped by cycle, term and period structure so a sibling grid over the same cycle — a
/// second campus on a different bell schedule — is never seen, let alone reported as
/// orphaned.
fn existing_slots(
    tx: &Transaction<'_>,
    request: &MaterialisationRequest,
) -> Result<HashMap<(Uuid, Uuid), Timeslot>> {
    let mut stmt = tx.prepare(&format!(
        "SELECT {TIMESLOT_COLUMNS} FROM timeslot
         WHERE cycle_id = ?
           AND term_id IS ?
           AND period_id IN (SELECT id FROM period WHERE period_structure_id = ?)"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![
            text(request.cycle_id),
            opt_text(request.term_id),
            text(request.period_structure_id)
        ],
        Timeslot::from_row,
    )?;

    let mut found = HashMap::new();
    for slot in rows {
        let slot = slot?;
        found.insert((slot.cycle_day_id, slot.period_id), slot);
    }
    Ok(found)
}

/// Which of a bell schedule's periods admit lessons.
///
/// Read once per rematerialisation rather than once per slot: a ten-day cycle with twelve
/// periods is 120 slots, and asking the same twelve questions ten times over is pure waste.
fn teaching_periods(
    tx: &Transaction<'_>,
    period_structure_id: Uuid,
) -> Result<HashMap<Uuid, bool>> {
    let mut stmt = tx.prepare("SELECT id, kind FROM period WHERE period_structure_id = ?")?;
    let rows = stmt.query_map([text(period_structure_id)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut kinds = HashMap::new();
    for row in rows {
        let (id, kind) = row?;
        kinds.insert(
            super::row::parse_id_text("period.id", &id)?,
            kind == PeriodKind::Teaching.as_str(),
        );
    }
    Ok(kinds)
}

/// Brings the derived columns of surviving slots back in line.
///
/// Only writes where something actually differs. That is what makes "running
/// materialisation twice is idempotent" a statement about revisions rather than about row
/// counts — and revisions are what a future synchronisation will merge on.
fn refresh_preserved(
    tx: &Transaction<'_>,
    request: &MaterialisationRequest,
    context: &GridContext,
    teaching: &HashMap<Uuid, bool>,
    preserved: &[PlannedSlot],
) -> Result<usize> {
    let mut refreshed = 0usize;
    let mut read = tx.prepare(&format!(
        "SELECT {TIMESLOT_COLUMNS} FROM timeslot WHERE id = ?"
    ))?;

    for slot in preserved {
        let Some(id) = slot.id else { continue };
        let stored: Timeslot = {
            let mut rows = read.query_map([text(id)], Timeslot::from_row)?;
            match rows.next() {
                Some(row) => row?,
                None => return Err(DbError::NotFound { entity: "timeslot" }),
            }
        };

        let teaching = *teaching.get(&slot.period_id).unwrap_or(&false);
        let unchanged = stored.school_id == context.school_id
            && stored.cycle_id == request.cycle_id
            && stored.campus_id == context.campus_id
            && stored.ordinal == slot.ordinal
            && stored.is_teaching == teaching;
        if unchanged {
            continue;
        }

        update(
            tx,
            "timeslot",
            "timeslot",
            id,
            "school_id = ?, cycle_id = ?, campus_id = ?, ordinal = ?, is_teaching = ?",
            &[
                &text(context.school_id),
                &text(request.cycle_id),
                &opt_text(context.campus_id),
                &slot.ordinal,
                &teaching,
            ],
        )?;
        refreshed += 1;
    }

    Ok(refreshed)
}
