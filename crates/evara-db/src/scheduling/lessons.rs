//! Class sections and the occurrences they demand.
//!
//! # One activity row is one placeable occurrence
//!
//! [ADR 0029](../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md):
//! four single periods a cycle is **four** `activity` rows and therefore four
//! [`ActivityFact`]s. There is no `occurrences_per_cycle` column and this loader invents no
//! substitute — it does not read `course.periods_per_cycle` and multiply, and it does not
//! split a `duration_periods = 2` row into two activities. The stored rows are the variable
//! set, one for one, and `split_index` travels with each because Phase 4's "spread these
//! across four days" has to address the occurrences individually.
//!
//! # No table in the chain carries a school
//!
//! `subject → course → class_section → activity`, and ownership is resolved through it —
//! never by adding a second `school_id` column. Every query here therefore joins up to
//! `subject`, which is the same chain `require_school_owner` walks for every write.
//!
//! # Which sections a term selects
//!
//! `course.term_id` is nullable and means *all year*. A section is in scope when its
//! course's term is `NULL` or is the selected term; and when **no** term is selected, every
//! course is in scope.
//!
//! That second branch is a deliberate choice rather than an oversight. With no term named
//! there is no basis on which to exclude a course, and excluding term-specific courses would
//! *under-count demand* — the one direction
//! [`PRE-SOLVE-VALIDATION.md`](../../../../docs/PRE-SOLVE-VALIDATION.md) §11 forbids, since a
//! counting check is only sound when demand is exact or under-stated and capacity exact or
//! over-stated. A school that materialises one year-long grid and also marks courses by term
//! will see all of that demand at once, and M3's counting findings will say so.

use evara_core::scheduling::{
    ActivityFact, AttendanceFloor, DurationPeriods, ExpectedAttendance, GroupFact, GroupIx,
    GroupMask, MaterialisedGrid, OverlapRelation, RoomFact, SchedulingContext, SectionFact,
    TeacherIx,
};
use rusqlite::Connection;
use uuid::Uuid;

use super::facilities;
use super::index::ProblemIndex;
use super::{IN_SCOPE_TERM, SchedulingSelection, corrupt, require_index};
use crate::error::Result;
use crate::repo::row::{opt_text, parse_id_text, text};

/// The ordering every read of the lesson chain shares.
///
/// Subject, then course, then section — the nesting the Lessons screen presents, and total
/// because `subject` is unique on `(school_id, name)`, `course` on `(subject_id, name)` and
/// `class_section` on `(course_id, name)`. So a section's index is its position in the list
/// the user is looking at, which is the stable secondary key `PRE-SOLVE-VALIDATION.md` §4
/// asks a diagnostic to be ordered by.
const SECTION_ORDER: &str = "su.name, su.id, co.name, co.id, cs.name, cs.id";

/// The chain every read joins, from `class_section` up to the school that owns it.
const SECTION_CHAIN: &str = "JOIN course  co ON co.id = cs.course_id
         JOIN subject su ON su.id = co.subject_id";

/// One class-section row, before its facts can be assembled.
///
/// Held between the two passes the loader makes: the indices have to exist before a
/// `SectionFact` can name a teacher, a group or a room, and the grid has to exist before a
/// bell schedule can be resolved.
pub(super) struct SectionRow {
    /// Stable identity, as stored.
    pub(super) id: Uuid,
    /// What the section is called.
    pub(super) name: String,
    /// The campus it is taught at, when the school has said.
    pub(super) campus: Option<evara_core::scheduling::CampusIx>,
    /// The room type its course requires.
    pub(super) required_room_type: Option<evara_core::scheduling::RoomTypeIx>,
    /// What the course declared it needs per cycle.
    pub(super) declared_periods_per_cycle: Option<i64>,
    /// How many students are named on the roster.
    pub(super) individually_enrolled: i64,
}

/// Reads the sections in scope, interning each in listing order.
pub(super) fn section_rows(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<Vec<SectionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT cs.id, cs.name, cs.campus_id, co.required_room_type_id, co.periods_per_cycle,
                (SELECT COUNT(*) FROM class_enrolment ce WHERE ce.class_section_id = cs.id)
         FROM class_section cs
         {SECTION_CHAIN}
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}
         ORDER BY {SECTION_ORDER}"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;

    let mut out = Vec::new();
    for row in rows {
        let (raw, name, raw_campus, raw_type, declared, enrolled) = row?;
        let id = parse_id_text("class_section.id", &raw)?;
        let campus = raw_campus
            .map(|raw| parse_id_text("class_section.campus_id", &raw))
            .transpose()?
            .map(|campus| require_index(&index.campuses, campus, "class_section.campus_id"))
            .transpose()?;
        let required_room_type = raw_type
            .map(|raw| parse_id_text("course.required_room_type_id", &raw))
            .transpose()?
            .map(|kind| require_index(&index.room_types, kind, "course.required_room_type_id"))
            .transpose()?;
        index.sections.intern(id);
        out.push(SectionRow {
            id,
            name,
            campus,
            required_room_type,
            declared_periods_per_cycle: declared,
            individually_enrolled: enrolled,
        });
    }
    Ok(out)
}

/// Who teaches each in-scope section, by section index.
///
/// **Fixed input, not a planning variable** (D7). An empty list is a section nobody has been
/// given, which M3 reports as `section-without-teacher`.
pub(super) fn staffing(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
) -> Result<Vec<Vec<TeacherIx>>> {
    let mut lists = vec![Vec::new(); index.sections.len()];
    let mut stmt = conn.prepare(&format!(
        "SELECT ct.class_section_id, ct.teacher_id
         FROM class_teacher ct
         JOIN class_section cs ON cs.id = ct.class_section_id
         {SECTION_CHAIN}
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    for row in rows {
        let (raw_section, raw_teacher) = row?;
        let section = parse_id_text("class_teacher.class_section_id", &raw_section)?;
        let teacher = parse_id_text("class_teacher.teacher_id", &raw_teacher)?;
        let at = require_index(&index.sections, section, "class_teacher.class_section_id")?;
        let teacher = require_index(&index.teachers, teacher, "class_teacher.teacher_id")?;
        lists[at.get()].push(teacher);
    }
    // Sorted by index rather than left in query order, because `SectionFact::teachers` is
    // documented as being in index order and the teacher table is built in two passes — a
    // deactivated teacher is interned after every employed one whatever their name.
    for list in &mut lists {
        list.sort_unstable();
        list.dedup();
    }
    Ok(lists)
}

/// Which groups attend each in-scope section, by section index.
///
/// A set of **groups**, never an expanded set of students (ADR 0035 §5). An empty list is a
/// section with no clash protection at all, which M3 reports with its remedy.
pub(super) fn audience(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
) -> Result<Vec<Vec<GroupIx>>> {
    let mut lists = vec![Vec::new(); index.sections.len()];
    let mut stmt = conn.prepare(&format!(
        "SELECT cg.class_section_id, cg.student_group_id
         FROM class_group cg
         JOIN class_section cs ON cs.id = cg.class_section_id
         {SECTION_CHAIN}
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    for row in rows {
        let (raw_section, raw_group) = row?;
        let section = parse_id_text("class_group.class_section_id", &raw_section)?;
        let group = parse_id_text("class_group.student_group_id", &raw_group)?;
        let at = require_index(&index.sections, section, "class_group.class_section_id")?;
        let group = require_index(&index.groups, group, "class_group.student_group_id")?;
        lists[at.get()].push(group);
    }
    for list in &mut lists {
        list.sort_unstable();
        list.dedup();
    }
    Ok(lists)
}

/// Reads the occurrences to be placed.
///
/// One row, one [`ActivityFact`]. Ordered by section and then by `split_index`, so an
/// activity's index follows its section's and the set is stable across loads.
pub(super) fn activities(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<Vec<ActivityFact>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id, a.class_section_id, a.name, a.duration_periods, a.split_index
         FROM activity a
         JOIN class_section cs ON cs.id = a.class_section_id
         {SECTION_CHAIN}
         WHERE su.school_id = ?1 AND {IN_SCOPE_TERM}
         ORDER BY {SECTION_ORDER}, a.split_index"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![text(selection.school_id), opt_text(selection.term_id)],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        },
    )?;

    let mut facts = Vec::new();
    for row in rows {
        let (raw, raw_section, name, duration, split) = row?;
        let id = parse_id_text("activity.id", &raw)?;
        let section = parse_id_text("activity.class_section_id", &raw_section)?;
        let section = require_index(&index.sections, section, "activity.class_section_id")?;
        let duration = u16::try_from(duration)
            .ok()
            .and_then(DurationPeriods::new)
            .ok_or_else(|| corrupt("activity.duration_periods", duration.to_string()))?;
        let split_index =
            u16::try_from(split).map_err(|_| corrupt("activity.split_index", split.to_string()))?;
        index.activities.intern(id);
        facts.push(ActivityFact {
            id,
            section,
            name,
            duration,
            split_index,
        });
    }
    Ok(facts)
}

/// Assembles the section facts, once everything they refer to exists.
///
/// # Why the bell schedule is resolved here and not read from a column
///
/// There is no column. `grid` is `G` derived from the materialised slots, and
/// [`MaterialisedGrid::resolve`] is
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §2–§3's one implementation, shared with M3's validator and M5's `duration-contiguous`.
/// This loader **calls** it; it does not reimplement the compatibility relation, which
/// [ADR 0036](../../../../docs/adr/0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md)
/// §2 requires and `Problem::integrity` would report if it drifted.
///
/// Resolution reads only the section's campus, so every occurrence of a section resolves
/// identically and the question is asked once per section rather than once per activity.
#[allow(clippy::too_many_arguments)]
pub(super) fn sections(
    rows: Vec<SectionRow>,
    context: &SchedulingContext,
    grid: &MaterialisedGrid,
    rooms: &[RoomFact],
    groups: &[GroupFact],
    overlap: &OverlapRelation,
    staffing: Vec<Vec<TeacherIx>>,
    audience: Vec<Vec<GroupIx>>,
) -> Vec<SectionFact> {
    rows.into_iter()
        .zip(staffing)
        .zip(audience)
        .map(|((row, teachers), attending)| {
            let mut audience_mask = GroupMask::with_capacity(groups.len());
            for group in &attending {
                audience_mask.insert(*group);
            }
            let clash_mask = overlap.clash_mask(&audience_mask);

            SectionFact {
                id: row.id,
                name: row.name,
                campus: row.campus,
                bell_schedule: grid.resolve(context, row.campus),
                teachers,
                expected_attendance: attendance(&attending, groups, overlap),
                attendance_floor: floor(&attending, groups),
                audience: attending,
                audience_mask,
                clash_mask,
                required_room_type: row.required_room_type,
                room_candidates: facilities::candidates(rooms, row.campus, row.required_room_type),
                individually_enrolled: row.individually_enrolled,
                declared_periods_per_cycle: row.declared_periods_per_cycle,
            }
        })
        .collect()
}

/// How many people a lesson of this section should be assumed to hold.
///
/// [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
/// §3's derivation, as far as `V15` permits it:
///
/// 1. **The explicit value wins** — and there is no explicit value yet.
///    `class_section.expected_attendance` is a FUTURE column (§2,
///    `SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1), so this clause reads nothing today and
///    becomes the first branch when the column lands.
/// 2. **No attending group** — `Unknown`.
/// 3. **Every group's size declared and the groups pairwise `Disjoint`** — `Known(Σ size)`.
/// 4. **Otherwise** — `Unknown`.
///
/// Clause 3 reaches every pair since M5, and that is the one thing that changed here.
///
/// M4 compiled no overlap answers at all, so no *pair* of groups was provably disjoint and
/// clause 3 could fire only where there were no pairs — a section taught to exactly one sized
/// group. The seven rules now exist (`population::overlap`), so a section taught to two groups
/// that are **proved** disjoint and both declare a size is `Known(Σ)`, which is clause 3 as
/// ADR 0034 §3 writes it. M4's §20 listed this as owed by M5.
///
/// `Overlap::Indeterminate` is **not** disjoint. A section whose groups might share students
/// stays `Unknown`, because summing the sizes would count a student twice and an over-stated
/// attendance is a room the school is told is too small. `Overlap::clashes` is not consulted
/// either: this needs the positive proof, so it reads `is_disjoint` and nothing else.
///
/// `Unknown` is never `0`: ADR 0034 §3's last clause makes it a third value that propagates,
/// and nothing in the first constraint set reads it, because `room-capacity` is not in that
/// set.
fn attendance(
    audience: &[GroupIx],
    groups: &[GroupFact],
    overlap: &OverlapRelation,
) -> ExpectedAttendance {
    if audience.is_empty() {
        return ExpectedAttendance::Unknown;
    }

    let mut total: i64 = 0;
    for group in audience {
        let Some(size) = groups.get(group.get()).and_then(|held| held.declared_size) else {
            return ExpectedAttendance::Unknown;
        };
        total = total.saturating_add(size);
    }

    // Pairwise disjoint, proved. Vacuously true for one group, which is why the common case
    // needed no pair logic before M5 and needs none now.
    for (at, a) in audience.iter().enumerate() {
        for b in &audience[at + 1..] {
            if !overlap.get(*a, *b).is_disjoint() {
                return ExpectedAttendance::Unknown;
            }
        }
    }

    ExpectedAttendance::Known { people: total }
}

/// A floor on the attendance, from member rows.
///
/// ADR 0034 §3: *"Where a group's `size` is `NULL` and it has `k > 0` member rows, `k` is a
/// floor on that group's population. The maximum floor across a section's groups is a sound
/// lower bound on its attendance."*
///
/// The maximum and not the sum, because the groups may share students and a sum over
/// possibly-overlapping groups is not a bound in either direction. It may raise a warning and
/// may never decide a hard constraint.
///
/// A group that *has* declared a size contributes no floor: its size is the better statement,
/// and `expected_attendance` is where it is read.
fn floor(audience: &[GroupIx], groups: &[GroupFact]) -> AttendanceFloor {
    let at_least = audience
        .iter()
        .filter_map(|group| groups.get(group.get()))
        .filter(|group| group.declared_size.is_none())
        .map(|group| group.effective_members)
        .max()
        .unwrap_or(0);
    AttendanceFloor { at_least }
}
