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

use evara_core::time::CycleCoverage;
use rusqlite::types::{ToSql, ToSqlOutput};
use rusqlite::{Row, Transaction};
use serde::{Deserialize, Serialize};
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
// The JSON and TypeScript spelling is the *stored* spelling, so the schema's CHECK
// values, the Rust variants, the wire format and the generated TypeScript union are all
// one vocabulary. The renames below are a second copy of the list in `stored_as!`, which
// `kinds_match_their_stored_spelling` exists to catch if the two ever drift. Deliberately
// a plain comment: it is a note to a maintainer, not something to publish into the
// generated TypeScript a frontend reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PeriodKind {
    /// Lessons may be scheduled into it.
    #[serde(rename = "TEACHING")]
    Teaching,
    /// Break, lunch, recess.
    #[serde(rename = "BREAK")]
    Break,
    /// Form time, roll call, assembly.
    #[serde(rename = "REGISTRATION")]
    Registration,
    /// Anything else the school names.
    #[serde(rename = "OTHER")]
    Other,
}

/// What a calendar date is.
// Spelled as stored, for the reason above. Note `PD` rather than
// `PROFESSIONAL_DEVELOPMENT`: the column's CHECK says `PD`, and the wire format follows
// the column rather than the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CalendarDayKind {
    /// A normal teaching day.
    #[serde(rename = "SCHOOL")]
    School,
    /// Closed.
    #[serde(rename = "HOLIDAY")]
    Holiday,
    /// Examinations.
    #[serde(rename = "EXAM")]
    Exam,
    /// A whole-school event.
    #[serde(rename = "EVENT")]
    Event,
    /// Staff professional development.
    #[serde(rename = "PD")]
    ProfessionalDevelopment,
}

/// Declares the stored spelling of an enum, both directions, in one place.
///
/// The spellings are the `CHECK` constraint's own values, so drift between the schema and
/// the Rust type would be a compile-time-visible edit to one list rather than two.
macro_rules! stored_as {
    ($ty:ty, $entity:expr, { $($variant:path => $text:literal),* $(,)? }) => {
        impl $ty {
            /// Every variant, in declaration order.
            ///
            /// The interface renders a picker from this rather than hardcoding a list that
            /// would quietly fall behind when a kind is added.
            pub const ALL: &'static [Self] = &[$($variant,)*];

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

    /// Which of a cycle's declared positions actually have a day.
    ///
    /// Reads the ordinals and hands them to [`CycleCoverage::of`], which holds the rule.
    /// The rule is in `evara-core` rather than in this query because "is this cycle
    /// finished?" is a question about the model, and the interface must not be the only
    /// place that knows the answer.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such cycle. Reported rather than returning an
    /// empty coverage, which would read as "declared 0 days" and look deliberately empty.
    pub fn cycle_coverage(&self, cycle_id: Uuid) -> Result<CycleCoverage> {
        let declared = self
            .cycle(cycle_id)?
            .ok_or(DbError::NotFound { entity: "cycle" })?
            .day_count;

        self.db.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT ordinal FROM cycle_day WHERE cycle_id = ? ORDER BY ordinal")?;
            let ordinals: Vec<i64> = stmt
                .query_map([text(cycle_id)], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(CycleCoverage::of(declared, &ordinals))
        })
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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

/// An opaque summary of what a plan would do, so a confirmation can be proved current.
///
/// # Why a fingerprint rather than passing the plan back
///
/// [ADR 0011](../../../docs/adr/0011-stable-timeslot-identity.md) §5 settled that
/// [`TimeModel::apply_materialisation`] recomputes the plan inside its own transaction,
/// because a plan made earlier describes a world that may have moved on. That makes
/// applying *safe* — it can never act on tuples that no longer exist — but on its own it
/// does not make applying *what the user agreed to*. A human shown "12 new slots, nothing
/// orphaned" who clicks Apply a minute after someone deleted a period would silently get
/// a different outcome.
///
/// So the caller sends back a fingerprint of the plan it displayed, the repository
/// recomputes the plan inside the transaction and compares, and a mismatch is
/// [`DbError::ReviewRequired`] with nothing written. The comparison has to happen inside
/// the transaction to mean anything; doing it in the command handler would be a
/// check-then-act race, and a scheduling decision in the wrong layer besides.
///
/// # What counts as a change
///
/// Which tuples would be created, which slots would be preserved, and which would be
/// orphaned. **Not** `ordinal`: it is a derived projection that ADR 0011 §1 says is
/// expected to change, and a renumbering alters no slot's fate. Treating it as material
/// would make routine reordering demand a pointless second confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlanFingerprint(String);

impl PlanFingerprint {
    /// The fingerprint as it travels to the interface and back.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Builds one from an arbitrary string.
    ///
    /// Test-only, and deliberately so: outside a test the only legitimate source of a
    /// fingerprint is [`MaterialisationPlan::fingerprint`], and a public constructor
    /// would let a caller fabricate the agreement the guard exists to check. (It would
    /// still be safe — the apply path recomputes the plan regardless — but it would make
    /// the staleness check trivially bypassable, which is worse than useless.)
    #[cfg(test)]
    pub(crate) fn from_str_for_test(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for PlanFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// FNV-1a, 64-bit.
///
/// Spelled out rather than taken from `DefaultHasher` because this value crosses the IPC
/// boundary and comes back: it has to mean the same thing on both sides of a round trip,
/// and `DefaultHasher`'s output is explicitly not guaranteed stable between Rust releases.
/// FNV is four lines and fixed forever. It is not a security primitive and nothing here
/// treats it as one — a forged fingerprint can only cause a spurious re-review, never an
/// unreviewed write, because the apply path recomputes the plan regardless.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl MaterialisationPlan {
    /// A fingerprint of this plan, to send to a human and receive back on confirmation.
    ///
    /// Deterministic for a given set of created tuples, preserved slots and orphans,
    /// regardless of the order they happen to come out of the database in.
    #[must_use]
    pub fn fingerprint(&self) -> PlanFingerprint {
        let term = |id: Option<Uuid>| id.map_or_else(|| String::from("-"), |t| t.to_string());

        // Created slots have no identifier yet, so they are keyed by the logical tuple
        // that identifies them. Preserved and orphaned slots have one, and it is the
        // thing the user is being asked about.
        let mut lines: Vec<String> =
            Vec::with_capacity(self.created.len() + self.preserved.len() + self.orphaned.len());
        for slot in &self.created {
            lines.push(format!(
                "c:{}:{}:{}",
                slot.cycle_day_id,
                slot.period_id,
                term(slot.term_id)
            ));
        }
        for (tag, slots) in [("p", &self.preserved), ("o", &self.orphaned)] {
            for slot in slots {
                lines.push(format!(
                    "{tag}:{}",
                    slot.id
                        .map_or_else(|| String::from("?"), |id| id.to_string())
                ));
            }
        }
        lines.sort_unstable();

        PlanFingerprint(format!("{:016x}", fnv1a(lines.join("\n").as_bytes())))
    }

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

/// One cell of the grid preview: a `(cycle day, period)` pair and whether it exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GridCell {
    /// Which day of the cycle.
    pub cycle_day_id: Uuid,
    /// Which period of the bell schedule.
    pub period_id: Uuid,
    /// The materialised slot, or `None` when the grid has never been built here.
    ///
    /// `None` is what "missing timeslot" means, and it is the difference between a grid
    /// the solver can use and one it cannot.
    pub timeslot_id: Option<Uuid>,
    /// The stored slot's derived order, when the slot exists.
    pub ordinal: Option<i64>,
    /// Whether lessons could be scheduled here, from the period's kind.
    ///
    /// Reported for cells that do not exist yet as well, so the preview can distinguish a
    /// missing teaching slot from a missing break.
    pub is_teaching: bool,
}

/// The state of one grid, for the Timetable Grid Preview screen.
///
/// A read-only projection. It answers, in one round trip, every question the screen asks:
/// what the axes are, which cells exist, which do not, what rebuilding would do, and what
/// rebuilding would strand. Assembling it here rather than in the command handler keeps
/// the queries in the crate that owns SQL and keeps the handler a translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GridPreview {
    /// The cycle forming one axis.
    pub cycle_id: Uuid,
    /// The bell schedule forming the other.
    pub period_structure_id: Uuid,
    /// The term, or `None` for a year-wide grid.
    pub term_id: Option<Uuid>,
    /// The cycle's days, in `ordinal` order. The axis, and the only day identity there is.
    pub days: Vec<CycleDay>,
    /// The bell schedule's periods, in `ordinal` order.
    pub periods: Vec<Period>,
    /// Every `(day, period)` pair, in `(day.ordinal, period.ordinal)` order.
    pub cells: Vec<GridCell>,
    /// Whether the cycle itself is finished, and which positions are missing if not.
    pub coverage: CycleCoverage,
    /// What a rebuild would preserve, create and strand.
    pub counts: MaterialisationCounts,
    /// Slots a rebuild would strand. Never touched by anything in this projection.
    pub orphaned: Vec<PlannedSlot>,
    /// Fingerprint of the plan these counts came from.
    ///
    /// The screen sends it back with a rebuild or an orphan release, which is what proves
    /// the confirmation belongs to the figures that were displayed. See
    /// [`PlanFingerprint`].
    pub fingerprint: PlanFingerprint,
}

impl GridPreview {
    /// Whether the grid is fully materialised and the cycle behind it is finished.
    ///
    /// Three things have to hold, and the screen should say which one does not: the cycle
    /// has every declared day, the bell schedule has at least one period, and no cell is
    /// missing. A grid with no periods is trivially "not missing anything", which is why
    /// the period check is explicit rather than implied by the cell count.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.coverage.is_complete()
            && !self.periods.is_empty()
            && self.cells.iter().all(|cell| cell.timeslot_id.is_some())
    }
}

impl TimeModel<'_> {
    /// The state of one grid: axes, existing and missing cells, and what a rebuild would
    /// do.
    ///
    /// Writes nothing. The plan inside it is a snapshot, which is exactly why the
    /// [`PlanFingerprint`] travels with it.
    ///
    /// # Errors
    ///
    /// As [`TimeModel::plan_materialisation`]: [`DbError::NotFound`] for an unknown cycle,
    /// bell schedule or term, [`DbError::Invalid`] if the cycle and the bell schedule
    /// belong to different schools.
    pub fn grid_preview(&mut self, request: &MaterialisationRequest) -> Result<GridPreview> {
        let coverage = self.cycle_coverage(request.cycle_id)?;
        let days = self.cycle_days(request.cycle_id)?;
        let periods = self.periods(request.period_structure_id)?;
        let plan = self.plan_materialisation(request)?;

        // Scoped to this bell schedule's periods, per ADR 0011 §2: a school with two
        // campuses on different schedules has two grids over the same cycle, and one must
        // not show the other's slots.
        let wanted: HashMap<Uuid, PeriodKind> = periods.iter().map(|p| (p.id, p.kind)).collect();
        let existing: HashMap<(Uuid, Uuid), Timeslot> = self
            .timeslots(request.cycle_id, request.term_id)?
            .into_iter()
            .filter(|slot| wanted.contains_key(&slot.period_id))
            .map(|slot| ((slot.cycle_day_id, slot.period_id), slot))
            .collect();

        let mut cells = Vec::with_capacity(days.len() * periods.len());
        for day in &days {
            for period in &periods {
                let slot = existing.get(&(day.id, period.id));
                cells.push(GridCell {
                    cycle_day_id: day.id,
                    period_id: period.id,
                    timeslot_id: slot.map(|s| s.id),
                    ordinal: slot.map(|s| s.ordinal),
                    is_teaching: period.kind == PeriodKind::Teaching,
                });
            }
        }

        Ok(GridPreview {
            cycle_id: request.cycle_id,
            period_structure_id: request.period_structure_id,
            term_id: request.term_id,
            days,
            periods,
            coverage,
            counts: plan.counts(),
            orphaned: plan.orphaned.clone(),
            fingerprint: plan.fingerprint(),
            cells,
        })
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
        self.db.write(|tx| apply(tx, request, None))
    }

    /// [`TimeModel::apply_materialisation`], but only if the plan is still the one a human
    /// reviewed.
    ///
    /// `reviewed` is the [`MaterialisationPlan::fingerprint`] of the plan that was shown.
    /// The plan is recomputed inside the write transaction and compared against it, so
    /// there is no window between the check and the write.
    ///
    /// Prefer this over [`TimeModel::apply_materialisation`] anywhere a person pressed a
    /// button: the unreviewed form is for the CLI and for tests, which have no preview to
    /// go stale.
    ///
    /// # Errors
    ///
    /// - [`DbError::ReviewRequired`] if the grid moved since the preview. **Nothing is
    ///   written**; the caller re-reads the plan and asks again.
    /// - Otherwise as [`TimeModel::apply_materialisation`].
    pub fn apply_materialisation_reviewed(
        &mut self,
        request: &MaterialisationRequest,
        reviewed: &PlanFingerprint,
    ) -> Result<MaterialisationPlan> {
        self.db.write(|tx| apply(tx, request, Some(reviewed)))
    }

    /// Deletes exactly the slots this proposal strands, once a human has agreed to lose
    /// them.
    ///
    /// The guarded counterpart to [`TimeModel::delete_timeslots`], and the one the
    /// interface uses. It differs in the way that matters: it recomputes the plan inside
    /// the write transaction, refuses unless the plan still matches `reviewed`, and then
    /// deletes **the orphans that plan names** rather than a list of identifiers supplied
    /// from outside.
    ///
    /// That closes a real hole. `delete_timeslots` cannot verify orphan-hood — ADR 0011 §4
    /// explains why it is a property of a proposal, not of a row — so handing it
    /// identifiers captured from an earlier preview could delete a slot that has since
    /// become wanted again. Here the identifiers and the agreement are checked against the
    /// same snapshot.
    ///
    /// Deliberately **not** combined with materialisation. Creating and destroying stay
    /// separate calls so no flag can turn the safe one into the destructive one.
    ///
    /// Returns the identifiers removed, in plan order. An empty result means the proposal
    /// stranded nothing — not a failure.
    ///
    /// # Errors
    ///
    /// - [`DbError::ReviewRequired`] if the grid moved since the preview. Nothing is
    ///   deleted.
    /// - [`DbError::StillReferenced`] if anything — a future timetable entry — refers to
    ///   one of the slots. Nothing is deleted: the whole call rolls back, so no subset
    ///   disappears.
    pub fn release_orphans(
        &mut self,
        request: &MaterialisationRequest,
        reviewed: &PlanFingerprint,
    ) -> Result<Vec<Uuid>> {
        self.db.write(|tx| {
            let plan = plan(tx, request)?;
            require_unchanged(&plan, reviewed)?;

            let ids = plan.orphaned_ids();
            for id in &ids {
                delete(tx, "timeslot", "timeslot", *id)?;
            }
            Ok(ids)
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

/// Refuses to proceed when the recomputed plan is not the one that was reviewed.
///
/// Called **inside** the write transaction that is about to act, which is the whole point:
/// a comparison made before the transaction opened could be invalidated before the write
/// lands.
fn require_unchanged(plan: &MaterialisationPlan, reviewed: &PlanFingerprint) -> Result<()> {
    let current = plan.fingerprint();
    if current == *reviewed {
        return Ok(());
    }
    Err(DbError::ReviewRequired {
        reviewed: reviewed.as_str().to_owned(),
        current: current.as_str().to_owned(),
    })
}

/// Creates the missing slots and refreshes the surviving ones, inside a caller's
/// transaction.
///
/// Shared by [`TimeModel::apply_materialisation`] and
/// [`TimeModel::apply_materialisation_reviewed`] so there is exactly one implementation of
/// cases A and B. `reviewed` is the only difference between them, and it is checked after
/// the plan is computed and before anything is written.
///
/// Never performs case C: orphans are reported in the returned plan and left alone.
fn apply(
    tx: &Transaction<'_>,
    request: &MaterialisationRequest,
    reviewed: Option<&PlanFingerprint>,
) -> Result<MaterialisationPlan> {
    let mut plan = plan(tx, request)?;
    if let Some(reviewed) = reviewed {
        require_unchanged(&plan, reviewed)?;
    }

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
