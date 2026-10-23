//! Reading the display names a grid needs and a solve does not.
//!
//! # Why this is a second, opt-in read
//!
//! A compiled `Problem` is *"everything a solve needs, and nothing else"*, and
//! [ADR 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md) §2
//! refused to widen it with a field whose only consumer is a diagnostic. A field whose only
//! consumer is a **report** is the same field, so these names are not compiled into the
//! problem and are not carried on the `Assembly` either: a solve would pay for a vocabulary
//! it never reads, and M4's statement count per load is a figure worth keeping.
//!
//! So [`Scheduling::labels`](super::Scheduling::labels) is called by the viewing paths and by
//! nothing else, and the type it returns lives in `evara-core` because `evara-report` has to
//! read what this crate writes and neither can see the other.
//!
//! # School isolation
//!
//! Every query is scoped by `su.school_id` or `r.id IN (…)`, the same way the loader's are.
//! A section of another school is **absent** from the result rather than refused: this is a
//! read of one school's names, not a write that could cross a boundary, so the failure mode
//! to prevent is a name leaking in rather than an error being missed.
//!
//! # A roster is not a scheduling population
//!
//! [`student_labels`] reads `class_enrolment` and nothing else. ADR 0006 requires a full
//! timetable from `student_group.size` with no student records at all, so a student with no
//! enrolment row is **not in the result** — there is nothing to show them, and filling the
//! gap from a group's membership would present a guess as a child's confirmed schedule.

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
use rusqlite::Connection;
use uuid::Uuid;

use super::{IN_SCOPE_TERM, SchedulingSelection};
use crate::error::Result;
use crate::repo::row::{opt_text, parse_id_text, text};

/// The chain a section's name is assembled from, as the loader writes it.
const SECTION_CHAIN: &str = "JOIN course  co ON co.id = cs.course_id
         JOIN subject su ON su.id = co.subject_id";

/// Reads every display name in scope, in a deterministic order.
pub(super) fn labels(conn: &Connection, selection: &SchedulingSelection) -> Result<LabelSet> {
    Ok(LabelSet {
        sections: section_labels(conn, selection)?,
        rooms: room_labels(conn, selection)?,
        students: student_labels(conn, selection)?,
    })
}

/// One label per class section in scope, ordered as the Lessons screen presents them.
fn section_labels(conn: &Connection, selection: &SchedulingSelection) -> Result<Vec<SectionLabel>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT cs.id, su.name, co.name, cs.name, yl.name, ca.name
         FROM class_section cs
         {SECTION_CHAIN}
         LEFT JOIN year_level yl ON yl.id = co.year_level_id
         LEFT JOIN campus     ca ON ca.id = cs.campus_id
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}
         ORDER BY su.name, su.id, co.name, co.id, cs.name, cs.id"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    )?;

    let mut out = Vec::new();
    for row in rows {
        let (raw, subject, course, section, year_level, campus) = row?;
        let section_id = parse_id_text("class_section.id", &raw)?;
        // Composed by the one function that composes it, so a cell caption, a row header and
        // a findings entry cannot disagree about what a lesson is called.
        let short = SectionLabel::compose_short(year_level.as_deref(), &subject, &section);
        out.push(SectionLabel {
            section_id,
            subject,
            course,
            section,
            year_level,
            campus,
            short,
        });
    }
    Ok(out)
}

/// One label per room reachable from this school's campuses.
///
/// Scoped through `room_campus`, which is how a room reaches a school at all — a room carries
/// no `school_id`, and [ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md)
/// makes one physical room one identity however many campuses share it. `DISTINCT` because a
/// room shared by two of this school's campuses would otherwise be listed twice.
fn room_labels(conn: &Connection, selection: &SchedulingSelection) -> Result<Vec<RoomLabel>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.id, r.name, b.name
         FROM room r
         JOIN room_campus rc ON rc.room_id = r.id
         JOIN campus     ca ON ca.id = rc.campus_id
         LEFT JOIN building b ON b.id = r.building_id
         WHERE ca.school_id = ?1
         ORDER BY r.name, r.id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (raw, name, building) = row?;
        out.push(RoomLabel {
            room_id: parse_id_text("room.id", &raw)?,
            name,
            building,
        });
    }
    Ok(out)
}

/// One label per student with at least one `class_enrolment` row in scope.
///
/// # Explicit enrolment, and nothing else
///
/// A student absent from this list is a student no enrolment row names. That is an ordinary
/// state — ADR 0006 — and the remedy is to enter a roster, not to infer one. Nothing here
/// reads `student_group_member`, `student_group.size` or `class_group`: a group's timetable
/// is the group's, and presenting it as a named child's would be a claim nobody made.
fn student_labels(conn: &Connection, selection: &SchedulingSelection) -> Result<Vec<StudentLabel>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT s.id, s.given_name, s.family_name, s.code, cs.id
         FROM class_enrolment ce
         JOIN class_section cs ON cs.id = ce.class_section_id
         {SECTION_CHAIN}
         JOIN student s ON s.id = ce.student_id
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}
         ORDER BY s.family_name, s.given_name, s.id, cs.id"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;

    // Grouped in one pass, which the `ORDER BY` makes possible: every row of one student is
    // consecutive, so no map and no second sort is needed and the output order is the query's.
    let mut out: Vec<StudentLabel> = Vec::new();
    for row in rows {
        let (raw_student, given, family, code, raw_section) = row?;
        let student_id = parse_id_text("student.id", &raw_student)?;
        let section_id = parse_id_text("class_enrolment.class_section_id", &raw_section)?;
        match out.last_mut() {
            Some(last) if last.student_id == student_id => last.sections.push(section_id),
            _ => out.push(StudentLabel {
                student_id,
                name: display_name(&given, &family),
                code,
                sections: vec![section_id],
            }),
        }
    }
    Ok(out)
}

/// A student's name as a reader sees it.
///
/// `given family`, which is what the People screens show. Composed here rather than stored,
/// because a stored display name is a third copy of two columns.
fn display_name(given: &str, family: &str) -> String {
    let given = given.trim();
    let family = family.trim();
    if family.is_empty() {
        return given.to_owned();
    }
    if given.is_empty() {
        return family.to_owned();
    }
    format!("{given} {family}")
}

/// The stored flags of one timetable's entries, keyed by activity.
///
/// Read here rather than in [`crate::Timetables`] because it is a *view* concern: the writer
/// sets both to `0` and nothing in M8 changes them. Phase 8 is what writes them, and when it
/// does the grid already renders them.
pub(super) fn entry_flags(
    conn: &Connection,
    timetable_id: Uuid,
) -> Result<Vec<(Uuid, bool, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT activity_id, is_locked, is_manual
         FROM timetable_entry WHERE timetable_id = ?1 ORDER BY activity_id",
    )?;
    let rows = stmt.query_map([text(timetable_id)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (raw, locked, manual) = row?;
        out.push((
            parse_id_text("timetable_entry.activity_id", &raw)?,
            locked != 0,
            manual != 0,
        ));
    }
    Ok(out)
}
