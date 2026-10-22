//! The scheduling context and the grid: which school, cycle and term, and the slots that
//! exist in them.
//!
//! # What "in scope" means here
//!
//! One school, one cycle, one term —
//! [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! rule 1. All three are named by the caller and none is guessed: a solve that picked a
//! cycle would be building a timetable over a grid the user did not choose, and
//! `GridPreviewSection.tsx` already refuses to default it.
//!
//! The three are also *checked against each other* before anything else is read. A cycle
//! belonging to another school, or a term belonging to another school's academic year, is
//! [`DbError::WrongSchool`] — an execution failure and not a scheduling diagnostic, because
//! no amount of editing the school's data would make the request sensible.
//!
//! # `bell_schedules` is every schedule the school has **defined**
//!
//! Not `G`. [ADR 0036](../../../../docs/adr/0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md)
//! §5 corrected M2's documentation of this field: `G` is the schedules with *materialised
//! slots*, and it is derived from
//! [`Problem::timeslots`](evara_core::scheduling::Problem) by
//! [`MaterialisedGrid::of`](evara_core::scheduling::MaterialisedGrid). This field holds the
//! defined set, because a schedule's campus and name have to be readable even for one that
//! was never materialised — and because resolving against the defined set would report an
//! ambiguity at a correctly configured school, which ADR 0033 test 8 exists to forbid.

use evara_core::scheduling::clock::{TimeInterval, TimeOfDay};
use evara_core::scheduling::{
    BellScheduleFact, CampusFact, CycleDayFact, CycleFact, CycleKind, PeriodFact, PeriodKind,
    SchedulingContext, SchoolFact, TermFact, TimeslotFact,
};
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use super::index::ProblemIndex;
use super::{SchedulingSelection, corrupt, require_index};
use crate::error::{DbError, Result};
use crate::repo::row::{opt_text, text};

/// Resolves the school that owns a term, through its academic year.
const TERM_OWNER: &str = "SELECT academic_year.school_id FROM term
     JOIN academic_year ON academic_year.id = term.academic_year_id
     WHERE term.id = ?";

/// Resolves the school that owns a cycle.
const CYCLE_OWNER: &str = "SELECT school_id FROM cycle WHERE id = ?";

/// Reads the context, and the grid it is about, into facts and dense indices.
///
/// Returns the context and the slots separately because a [`Problem`] holds them in two
/// fields, and because the slots are what the bell-schedule resolver derives `G` from.
pub(super) fn load(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<(SchedulingContext, Vec<TimeslotFact>)> {
    let school = school_fact(conn, selection.school_id)?;
    let cycle = cycle_fact(conn, selection)?;
    let term = term_fact(conn, selection)?;

    let campuses = campuses(conn, selection.school_id, index)?;
    let cycle_days = cycle_days(conn, selection.cycle_id, index)?;
    let (bell_schedules, periods) = bells(conn, selection.school_id, index)?;

    let context = SchedulingContext {
        school,
        cycle,
        term,
        campuses,
        cycle_days,
        bell_schedules,
        periods,
    };
    let timeslots = timeslots(conn, selection, &context, index)?;
    Ok((context, timeslots))
}

/// The school being solved.
///
/// # Errors
///
/// [`DbError::NotFound`] if the project holds no such school. A project holds several since
/// `V12`, so there is no "the school" to fall back on — a `LIMIT 1` read would hand the
/// caller an arbitrary institution and silently scope everything after it.
fn school_fact(conn: &Connection, school_id: Uuid) -> Result<SchoolFact> {
    conn.query_row(
        "SELECT name, timezone FROM school WHERE id = ?",
        [text(school_id)],
        |row| {
            Ok(SchoolFact {
                id: school_id,
                name: row.get(0)?,
                timezone: row.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or(DbError::NotFound { entity: "school" })
}

/// The cycle the grid is defined over, refusing one that belongs elsewhere.
fn cycle_fact(conn: &Connection, selection: &SchedulingSelection) -> Result<CycleFact> {
    crate::repo::require_school_owner(
        conn,
        "cycle",
        CYCLE_OWNER,
        selection.cycle_id,
        selection.school_id,
    )?;

    conn.query_row(
        "SELECT name, kind FROM cycle WHERE id = ?",
        [text(selection.cycle_id)],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()?
    .ok_or(DbError::NotFound { entity: "cycle" })
    .and_then(|(name, kind)| {
        Ok(CycleFact {
            id: selection.cycle_id,
            name,
            kind: cycle_kind(&kind)?,
        })
    })
}

/// The term, when one was named, refusing one from another school's academic year.
///
/// `None` is an ordinary selection and not a missing value: `timeslot.term_id` is nullable
/// and a school may materialise one grid for the whole year.
fn term_fact(conn: &Connection, selection: &SchedulingSelection) -> Result<Option<TermFact>> {
    let Some(term_id) = selection.term_id else {
        return Ok(None);
    };
    crate::repo::require_school_owner(conn, "term", TERM_OWNER, term_id, selection.school_id)?;

    let name: Option<String> = conn
        .query_row(
            "SELECT name FROM term WHERE id = ?",
            [text(term_id)],
            |row| row.get(0),
        )
        .optional()?;
    name.map(|name| TermFact { id: term_id, name })
        .ok_or(DbError::NotFound { entity: "term" })
        .map(Some)
}

/// The school's campuses, in the order the interface lists them.
fn campuses(
    conn: &Connection,
    school_id: Uuid,
    index: &mut ProblemIndex,
) -> Result<Vec<CampusFact>> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM campus WHERE school_id = ? ORDER BY name, id")?;
    let rows = stmt.query_map([text(school_id)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut facts = Vec::new();
    for row in rows {
        let (raw, name) = row?;
        let id = crate::repo::row::parse_id_text("campus.id", &raw)?;
        index.campuses.intern(id);
        facts.push(CampusFact { id, name });
    }
    Ok(facts)
}

/// The cycle's positions, in ordinal order.
///
/// `ordinal` is a day's only scheduling identity — nothing here reads `weekday_hint`, and
/// nothing downstream may.
fn cycle_days(
    conn: &Connection,
    cycle_id: Uuid,
    index: &mut ProblemIndex,
) -> Result<Vec<CycleDayFact>> {
    let mut stmt = conn
        .prepare("SELECT id, ordinal, label FROM cycle_day WHERE cycle_id = ? ORDER BY ordinal")?;
    let rows = stmt.query_map([text(cycle_id)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut facts = Vec::new();
    for row in rows {
        let (raw, ordinal, label) = row?;
        let id = crate::repo::row::parse_id_text("cycle_day.id", &raw)?;
        index.cycle_days.intern(id);
        facts.push(CycleDayFact {
            id,
            ordinal: ordinal_of("cycle_day.ordinal", ordinal)?,
            label,
        });
    }
    Ok(facts)
}

/// Every bell schedule the school has defined, and every period of them.
///
/// Both tables are read in one pass each, and the periods are ordered by their schedule's
/// position and then by their own ordinal — so a schedule's `periods` list is contiguous in
/// index order, which is what lets a reader diff two loads by eye.
fn bells(
    conn: &Connection,
    school_id: Uuid,
    index: &mut ProblemIndex,
) -> Result<(Vec<BellScheduleFact>, Vec<PeriodFact>)> {
    let mut schedules = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, campus_id, name FROM period_structure
             WHERE school_id = ? ORDER BY name, id",
        )?;
        let rows = stmt.query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (raw, campus, name) = row?;
            let id = crate::repo::row::parse_id_text("period_structure.id", &raw)?;
            index.bell_schedules.intern(id);
            let campus = campus
                .map(|raw| crate::repo::row::parse_id_text("period_structure.campus_id", &raw))
                .transpose()?
                .map(|campus| require_index(&index.campuses, campus, "period_structure.campus_id"))
                .transpose()?;
            schedules.push(BellScheduleFact {
                id,
                name,
                campus,
                periods: Vec::new(),
            });
        }
    }

    let mut periods = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT p.id, p.period_structure_id, p.ordinal, p.label, p.starts_at, p.ends_at,
                    p.kind
             FROM period p
             JOIN period_structure ps ON ps.id = p.period_structure_id
             WHERE ps.school_id = ?
             ORDER BY ps.name, ps.id, p.ordinal",
        )?;
        let rows = stmt.query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        for row in rows {
            let (raw, raw_schedule, ordinal, label, starts_at, ends_at, kind) = row?;
            let id = crate::repo::row::parse_id_text("period.id", &raw)?;
            let schedule_id =
                crate::repo::row::parse_id_text("period.period_structure_id", &raw_schedule)?;
            let bell_schedule = require_index(
                &index.bell_schedules,
                schedule_id,
                "period.period_structure_id",
            )?;
            let at = index.periods.intern(id);
            schedules[bell_schedule.get()].periods.push(at);
            periods.push(PeriodFact {
                id,
                bell_schedule,
                ordinal: ordinal_of("period.ordinal", ordinal)?,
                label,
                interval: interval(&starts_at, &ends_at)?,
                kind: period_kind(&kind)?,
            });
        }
    }

    Ok((schedules, periods))
}

/// The materialised grid of `(cycle, term)`: the solver's value range.
///
/// Non-teaching slots are **kept**. The loader restricts an activity's candidates to the
/// teaching ones, and `no-teaching-in-period` stays a registered constraint because manual
/// editing can name a non-teaching slot — ADR 0033 §4, and both read the one stored
/// `timeslot.is_teaching` rather than re-deriving it from the period's kind.
///
/// A slot's wall-clock interval comes from its period, because a timeslot has no times of
/// its own. That interval is the clash key for shared occupancy (ADR 0032 rule 2); a
/// `timeslot_id` is not, because a school running two bell schedules holds two disjoint slot
/// sets over the same minutes.
fn timeslots(
    conn: &Connection,
    selection: &SchedulingSelection,
    context: &SchedulingContext,
    index: &mut ProblemIndex,
) -> Result<Vec<TimeslotFact>> {
    let mut stmt = conn.prepare(
        "SELECT ts.id, ts.cycle_day_id, ts.period_id, ts.campus_id, ts.is_teaching
         FROM timeslot ts
         JOIN period      p  ON p.id  = ts.period_id
         JOIN period_structure ps ON ps.id = p.period_structure_id
         JOIN cycle_day   cd ON cd.id = ts.cycle_day_id
         WHERE ts.cycle_id = ? AND ts.term_id IS ?
         ORDER BY cd.ordinal, ps.name, ps.id, p.ordinal",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.cycle_id), opt_text(selection.term_id)],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, bool>(4)?,
            ))
        },
    )?;

    let mut facts = Vec::new();
    for row in rows {
        let (raw, raw_day, raw_period, raw_campus, is_teaching) = row?;
        let id = crate::repo::row::parse_id_text("timeslot.id", &raw)?;
        let day = crate::repo::row::parse_id_text("timeslot.cycle_day_id", &raw_day)?;
        let period = crate::repo::row::parse_id_text("timeslot.period_id", &raw_period)?;

        let cycle_day = require_index(&index.cycle_days, day, "timeslot.cycle_day_id")?;
        let period = require_index(&index.periods, period, "timeslot.period_id")?;
        let campus = raw_campus
            .map(|raw| crate::repo::row::parse_id_text("timeslot.campus_id", &raw))
            .transpose()?
            .map(|campus| require_index(&index.campuses, campus, "timeslot.campus_id"))
            .transpose()?;

        let fact = context
            .periods
            .get(period.get())
            .ok_or_else(|| corrupt("timeslot.period_id", raw_period.clone()))?;

        index.timeslots.intern(id);
        facts.push(TimeslotFact {
            id,
            cycle_day,
            period,
            bell_schedule: fact.bell_schedule,
            campus,
            is_teaching,
            interval: fact.interval,
        });
    }
    Ok(facts)
}

/// Reads a stored `'HH:MM'` pair into a comparable interval.
///
/// The schema `CHECK`s both ends and `ends_at > starts_at`, so a value that will not parse
/// is corruption rather than input — the rule `parse_id` follows, and for the same reason:
/// an invented default makes a damaged file look like a working one, and a lesson placed
/// against a guessed bell time is exactly the wrong answer this product exists to avoid.
fn interval(starts_at: &str, ends_at: &str) -> Result<TimeInterval> {
    let start =
        TimeOfDay::parse(starts_at).ok_or_else(|| corrupt("period.starts_at", starts_at.into()))?;
    let end = TimeOfDay::parse(ends_at).ok_or_else(|| corrupt("period.ends_at", ends_at.into()))?;
    TimeInterval::new(start, end).ok_or_else(|| corrupt("period.ends_at", ends_at.into()))
}

/// Reads `period.kind`.
fn period_kind(raw: &str) -> Result<PeriodKind> {
    match raw {
        "TEACHING" => Ok(PeriodKind::Teaching),
        "BREAK" => Ok(PeriodKind::Break),
        "REGISTRATION" => Ok(PeriodKind::Registration),
        "OTHER" => Ok(PeriodKind::Other),
        other => Err(corrupt("period.kind", other.into())),
    }
}

/// Reads `cycle.kind`.
fn cycle_kind(raw: &str) -> Result<CycleKind> {
    match raw {
        "WEEKLY" => Ok(CycleKind::Weekly),
        "CUSTOM" => Ok(CycleKind::Custom),
        other => Err(corrupt("cycle.kind", other.into())),
    }
}

/// Narrows a stored ordinal to the width the facts use.
///
/// `CHECK (ordinal >= 1)` holds in the schema, so anything outside `u16` is corruption and
/// not a large school.
fn ordinal_of(column: &'static str, value: i64) -> Result<u16> {
    u16::try_from(value).map_err(|_| corrupt(column, value.to_string()))
}
