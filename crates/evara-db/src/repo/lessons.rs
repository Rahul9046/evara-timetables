//! Repositories for the Lessons group: courses, class sections, who teaches them, who
//! attends them, and the occurrences they demand.
//!
//! Same shape as [`super::curriculum`] and [`super::structure`] — callers pass an input
//! struct and receive a record, no `rusqlite` type appears in a signature, and every write
//! goes through [`super::update`] or [`super::delete`] so `rev` and `updated_at` cannot be
//! forgotten.
//!
//! # Every school-owned read and write resolves its school through the chain
//!
//! `subject -> course -> class_section -> activity`. Not one of those four tables carries a
//! `school_id`, and `V15`'s header argues it at length: `ROADMAP.md`'s Phase 3 notes
//! settled it for `course` specifically ("resolve the school through `subject.school_id`
//! rather than adding a second `school_id`"), and `CLAUDE.md` states it generally. So every
//! `update_*` and `delete_*` here calls [`super::require_school_owner`] with an owner query
//! that walks the chain, inside the same transaction as the write and before anything is
//! cleared.
//!
//! The queries are written out per table rather than composed, because the chain *is* the
//! domain statement — a course belongs to a school only by way of its subject, and an
//! activity only by way of its section and that section's course.
//!
//! # Three scopes meet here, and they are governed differently
//!
//! | | What it is | Rule for joining it to a section |
//! | --- | --- | --- |
//! | a **teacher** | a project-level identity | an **active** membership at the section's school, plus campus eligibility where the section is constrained |
//! | a **student group** | school-owned | must belong to the section's school |
//! | a **student** | a project-level identity | a **current** (non-`LEFT`) spell at the section's school |
//!
//! The teacher rule is the one with history behind it. ADR 0025 §4 wrote down a
//! *conjunction* of three things for Stage C to implement — qualification, active
//! membership, campus eligibility — and
//! [adr/0028](../../../docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md)
//! amends the first while keeping the other two. A missing qualification is reported by
//! [`Lessons::assignable_teachers`] and refuses nothing; a missing or inactive membership
//! is refused by the repository, by name, and by `V15`'s trigger pair underneath it.
//!
//! What is **not** weakened is ADR 0025's ownership model. A qualification is still a
//! property of the person, still carries no `school_id`, and is still removed only by an
//! explicit `unqualify_teacher`. Nothing in this module writes one.
//!
//! # Demand is rows, not a count
//!
//! One `activity` row is one placeable occurrence, and a class section needing four single
//! periods a cycle holds four of them.
//! [adr/0029](../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
//! argues it; the practical consequence for this module is that there is no
//! `occurrences_per_cycle` column to maintain and no multi-row update to make atomic.
//! Adding occurrences is a **creation** (ADR 0027 §5: no precondition, because there is no
//! previous version of a row that does not exist); editing one occurrence and deleting one
//! occurrence are ordinary single-record writes carrying that row's own revision.
//!
//! The arithmetic over those rows is [`evara_core::lessons`] and not SQL, so the solver and
//! the interface read the same answer from the same place.
//!
//! # Deletion
//!
//! Two different treatments, each chosen rather than defaulted, and `V15`'s header records
//! the reasoning:
//!
//! - [`Lessons::delete_course`] is **refused** while any class section exists, and
//!   [`Lessons::course_dependents`] says how many. A section is a substantial thing a
//!   school built, with staff and a roster on it. This is `delete_department`'s treatment
//!   of subjects.
//! - [`Lessons::delete_class_section`] **clears its own dependents** — teachers, groups,
//!   roster, activities — inside its transaction and through [`super::delete`], so each
//!   removal is a truthful change-log entry. A `class_teacher` row with no section is not
//!   history, it is a dangling fact. This is `delete_teacher`'s treatment of
//!   qualifications, and ADR 0025 §5's reasoning.

use evara_core::lessons::{CycleCapacity, DeclaredComparison, DemandFinding, LessonDemand};
use rusqlite::{Connection, OptionalExtension, Transaction, types::ToSql};
use uuid::Uuid;

use super::curriculum::{SUBJECT_COLUMNS, Subject};
use super::people::{
    STUDENT_COLUMNS, STUDENT_GROUP_COLUMNS, Student, StudentGroup, TEACHER_COLUMNS, Teacher,
};
use super::row::{entity, opt_text, parse_id_text, text};
use super::{Expected, Stamp, delete, require_school_owner, translate, update};
use crate::Database;
use crate::error::{DbError, Result};
use crate::repo::membership::{TEACHER_SCHOOL_COLUMNS, TeacherSchool};

/// Handle for reading and writing the Lessons group.
///
/// Obtained from [`Database::lessons`]. Holds the database mutably because most operations
/// write.
#[derive(Debug)]
pub struct Lessons<'a> {
    db: &'a mut Database,
}

impl<'a> Lessons<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

entity! {
    /// One school's offering of one subject: "Year 9 Mathematics".
    ///
    /// A *course* is what is taught; a [`Subject`] is what it is an offering *of*. The
    /// distinction earns its keep the moment a school teaches one subject at three year
    /// levels with three different period allocations, which is every school.
    ///
    /// There is no `school_id`: `subject.school_id` is the only answer, and a second copy
    /// of it would be a second place for it to be wrong. See this module's header.
    Course,
    /// Values for creating or replacing a course.
    CourseInput,
    {
        /// What is taught. The only source of this course's school.
        subject_id: Uuid,
        /// Who it is for, when the school says. Must be a year level of the same school.
        ///
        /// Optional, because a course may be open to several year levels or to none in
        /// particular, and because a school should not have to invent one to enter a
        /// course.
        year_level_id: Option<Uuid>,
        /// When it runs. `None` means all year, which is the common case.
        term_id: Option<Uuid>,
        /// Display name, unique within the subject.
        name: String,
        /// Optional short reference. Unique within the **school** when given.
        code: Option<String>,
        /// Free text for whatever the school needs to remember.
        description: Option<String>,
        /// Declared demand: how many period slots one section needs per cycle.
        ///
        /// `None` for a course that has not decided. The actual demand of a section is the
        /// sum over its activities, and the two are allowed to differ — the interface
        /// reports the difference as advice. ADR 0029 §3.
        periods_per_cycle: Option<i64>,
        /// How long one occurrence is when the generator expands the declaration.
        ///
        /// 4 periods per cycle at duration 1 is four single periods; at duration 2 it is
        /// two doubles.
        default_duration_periods: i64,
        /// What kind of room a lesson of this course needs. A requirement, not a booking.
        required_room_type_id: Option<Uuid>,
    }
}

entity! {
    /// One deliverable instance of a course: "Year 9 Mathematics A".
    ///
    /// Several per course is the normal case, several teachers per section is ordinary, and
    /// several attending groups per section is how a combined cohort is expressed. Its `id`
    /// is the durable identity Phase 6's timetable and Stage D's imports will address;
    /// names and codes may change without breaking a reference.
    ClassSection,
    /// Values for creating or replacing a class section.
    ClassSectionInput,
    {
        /// The offering this is an instance of. Supplies the school, through its subject.
        course_id: Uuid,
        /// Display name, unique within the course.
        name: String,
        /// Optional short reference. Unique within the **course** when given.
        code: Option<String>,
        /// Where it is taught, when that is decided.
        ///
        /// What makes the campus half of ADR 0028's rule mean anything: a teacher narrowed
        /// to the junior campus may not be given a section at the senior one. `None` means
        /// the section is not campus-constrained.
        campus_id: Option<Uuid>,
    }
}

entity! {
    /// That one teacher teaches one class section.
    ///
    /// Requires an **active** membership at the section's school, and campus eligibility
    /// where the section names a campus. Does **not** require a qualification for the
    /// subject — ADR 0028 — though the absence of one is reported.
    ClassTeacher,
    /// Values for assigning a teacher to a class section.
    ClassTeacherInput,
    {
        /// The section being staffed.
        class_section_id: Uuid,
        /// The person. Project-level, one row per human being.
        teacher_id: Uuid,
    }
}

entity! {
    /// That one student group attends one class section.
    ///
    /// The **primary scheduling population**. ADR 0006 requires that a school be able to
    /// build a full timetable from group sizes alone, so a section with groups and no named
    /// individuals is complete rather than half-finished.
    ClassGroup,
    /// Values for adding a student group to a class section.
    ClassGroupInput,
    {
        /// The section being attended.
        class_section_id: Uuid,
        /// The group attending it. Must belong to the section's school.
        student_group_id: Uuid,
    }
}

entity! {
    /// That one named student is on one class section's roster.
    ///
    /// Optional, and a **roster overlay** rather than a second scheduling population: a
    /// section taught to a group of 28 that also names four children has an audience of
    /// 28, because those four are among the 28. Nothing adds the two figures. ADR 0029 §4.
    ClassEnrolment,
    /// Values for putting a student on a class section's roster.
    ClassEnrolmentInput,
    {
        /// The section.
        class_section_id: Uuid,
        /// The person. Must have a current (non-`LEFT`) spell at the section's school.
        student_id: Uuid,
    }
}

entity! {
    /// **One placeable occurrence** of a class section's teaching.
    ///
    /// Four single periods a cycle is four of these. One double period a cycle is one of
    /// these with `durationPeriods = 2`. There is no occurrence count on the row, because
    /// the row *is* the occurrence — ADR 0029, and `V15`'s header for the three things that
    /// depend on it.
    ///
    /// An activity has no timeslot, no room and no cycle. It is demand, and nothing in
    /// Stage C claims it can be scheduled; only a solver can say that.
    Activity,
    /// The full column set of an activity.
    ///
    /// Carried by the repository rather than by a caller: `splitIndex` is assigned from
    /// `MAX(split_index) + 1` within the section and never renumbered, so there is no
    /// honest way for an interface to supply it. [`Lessons::add_activities`] and
    /// [`Lessons::update_activity`] take the fields a user actually chooses.
    ActivityInput,
    {
        /// The section this is an occurrence of. Supplies the school, through its course.
        class_section_id: Uuid,
        /// What to call this occurrence, when it needs its own name: "Laboratory".
        name: Option<String>,
        /// Consecutive periods one occurrence needs. A double period is 2.
        duration_periods: i64,
        /// Which occurrence this is within its section. Stable, never renumbered.
        split_index: i64,
    }
}

// ================================================================== read projections

/// A course with the names a listing needs, resolved in one query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CourseView {
    /// The course record. Its `rev` is the precondition for editing or deleting it.
    pub course: Course,
    /// What it is an offering of. Carries the owning `schoolId`.
    pub subject: Subject,
    /// Name of the year level it is for, when it names one.
    pub year_level_name: Option<String>,
    /// Name of the term it runs in, or `null` for a course that runs all year.
    pub term_name: Option<String>,
    /// Name of the room type it requires, when it requires one.
    pub room_type_name: Option<String>,
    /// How many class sections deliver it.
    pub sections: i64,
}

/// A class section with its staffing, audience and demand.
///
/// Every count and every finding is read here rather than by a call per row, for the reason
/// `Subjects` learned the hard way about dependent counts: the interface has to decide
/// *synchronously* whether to offer a destructive button, and a per-row round trip means
/// the answer arrives after the question.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClassSectionView {
    /// The section record. Its `rev` is the precondition for editing or deleting it.
    pub section: ClassSection,
    /// Name of the campus it is taught at, when it is campus-constrained.
    pub campus_name: Option<String>,
    /// How many teachers are assigned.
    pub teachers: i64,
    /// How many student groups attend.
    pub groups: i64,
    /// Declared size of those groups, summed.
    ///
    /// The scheduling population in Milestone 1 (ADR 0006). A group with no declared size
    /// contributes its counted membership instead, which is what
    /// `student_group.size IS NULL` means — see [`Lessons::class_groups`].
    pub group_population: i64,
    /// How many individuals are named on the roster. **Never added to
    /// [`Self::group_population`]** — ADR 0029 §4.
    pub enrolled_students: i64,
    /// What this section asks of a cycle, summed over its occurrences.
    pub demand: LessonDemand,
    /// How that compares with what the course declared. Advice, never a refusal.
    pub declared: DeclaredComparison,
    /// Impossibilities in the demand, if any. Empty does **not** mean schedulable.
    pub findings: Vec<DemandFinding>,
}

/// What a course still holds, counted per kind.
///
/// Read *before* a delete is offered, so the confirmation explains rather than the button
/// fails — the pattern `SchoolDependents` established in Stage 0A and ADR 0023 §4 made
/// ordinary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CourseDependents {
    /// Sum of the other fields. Zero means the course can be deleted.
    pub total: i64,
    /// Class sections delivering it.
    pub sections: i64,
}

/// What a class section holds, so a confirmation can say what deleting it takes with it.
///
/// Unlike [`CourseDependents`] these do **not** block the delete: the section owns every
/// one of them and clears them itself. The counts exist so the user is told what is about
/// to go, which is the difference between a confirmation and a trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ClassSectionDependents {
    /// Sum of the other fields.
    pub total: i64,
    /// Teacher assignments that will be removed with it.
    pub teachers: i64,
    /// Student group associations that will be removed with it.
    pub groups: i64,
    /// Roster rows that will be removed with it.
    pub enrolled_students: i64,
    /// Occurrences that will be removed with it.
    pub activities: i64,
}

/// One teacher assigned to a class section, with the two statuses the screen must show.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssignedTeacher {
    /// The assignment record. Its `rev` is the precondition for removing it.
    pub assignment: ClassTeacher,
    /// The shared identity. One record for one real person, however many schools.
    pub teacher: Teacher,
    /// This school's employment of them, which may since have been deactivated.
    ///
    /// An assignment made while the membership was active survives its deactivation —
    /// ADR 0018's "keeps every assignment they ever had, and simply stops being offered for
    /// new ones" — so this is shown rather than assumed.
    pub membership: TeacherSchool,
    /// Whether they hold a qualification for this course's subject.
    ///
    /// `false` is a **warning**, never a reason the row is invalid. ADR 0028.
    pub qualified: bool,
}

/// What [ADR 0028](../../../docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md)
/// says about giving one teacher one class section.
///
/// Three answers, and the whole value of the type is that they are **not the same kind of
/// answer**. Two of them refuse the assignment and one of them warns about it, which is
/// the amendment ADR 0028 makes to ADR 0025 §4's conjunction. A flat set of booleans would
/// let a screen treat the warning as a refusal, which is the mistake this stage exists to
/// avoid: a school covering an absent specialist on Tuesday must be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssignmentEligibility {
    /// Whether this school may give them new work: both `isActive` flags together.
    ///
    /// A **refusal**. The project-level flag is "archived in this campus file" and the
    /// membership's is "currently teaching here"; either one being off means no new
    /// classes, and ADR 0018's rule is that the classes they already have are untouched.
    pub employed_and_active: bool,
    /// Whether they are eligible at the section's campus.
    ///
    /// A **refusal**. `true` when the section names no campus, and `true` when the teacher
    /// has no campus eligibility recorded at this school — `teacher_campus` is a
    /// *narrowing*, so an empty set narrows nothing (ADR 0021 §3).
    pub campus_eligible: bool,
    /// Whether they hold a qualification for this course's subject.
    ///
    /// A **warning**, never a refusal. ADR 0028, and the reason this type exists.
    pub qualified: bool,
}

impl AssignmentEligibility {
    /// Whether the assignment would be accepted.
    ///
    /// The two refusals, and deliberately not the warning. Computed here so no caller has
    /// to remember which of the three fields is advisory — the question ADR 0028 is about.
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        self.employed_and_active && self.campus_eligible
    }
}

/// One teacher a class section could be given, and everything the picker must say about it.
///
/// The whole of ADR 0028's policy, computed once in the backend. A screen that worked any
/// of this out for itself would be a scheduling rule in the view layer.
///
/// Returns **every** member of the school, refusals included, so a picker can say why
/// somebody expected is unavailable rather than leaving them mysteriously absent —
/// ADR 0023 §5's rule: *visible, explained, not silently editable.*
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TeacherCandidate {
    /// The person.
    pub teacher: Teacher,
    /// This school's employment of them.
    pub membership: TeacherSchool,
    /// The three answers, two refusals and one warning.
    pub eligibility: AssignmentEligibility,
    /// Whether they already teach this section.
    pub assigned: bool,
}

/// One student group attending a class section, with the population it contributes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AttendingGroup {
    /// The association record. Its `rev` is the precondition for removing it.
    pub association: ClassGroup,
    /// The group, which carries the owning `schoolId` and its declared `size`.
    pub group: StudentGroup,
    /// How many students are named as members of it.
    ///
    /// Shown alongside `group.size` rather than instead of it, because they answer
    /// different questions: the declared size is what the school says the group holds, and
    /// this is how much of it has been entered. ADR 0006 requires the first to be enough.
    pub named_members: i64,
    /// What this group contributes to the section's audience.
    ///
    /// `group.size` when declared, the named membership otherwise — "NULL means membership
    /// is the truth", which is what `V10` says the nullable column means.
    pub population: i64,
}

/// One individual on a class section's roster.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RosteredStudent {
    /// The roster record. Its `rev` is the precondition for removing it.
    pub enrolment: ClassEnrolment,
    /// The shared identity.
    pub student: Student,
    /// Whether they are also a member of one of the section's attending groups.
    ///
    /// The answer to "am I being counted twice?", shown rather than silently resolved.
    /// Normally `true`: naming individuals of a group that already attends is what a
    /// register is. A `false` here means somebody is on the roster whom no attending group
    /// covers, which is worth seeing.
    pub covered_by_group: bool,
}

/// One class section one teacher teaches, named well enough to show in a list.
///
/// Read by the Teachers screen and by the refusal that explains why a membership cannot be
/// withdrawn, so it names the course and subject rather than only the section.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaughtSection {
    /// The assignment record. Its `rev` is the precondition for removing it.
    pub assignment: ClassTeacher,
    /// The section.
    pub section: ClassSection,
    /// Name of the course it delivers.
    pub course_name: String,
    /// Name of the subject that course teaches.
    pub subject_name: String,
    /// The school that owns all of it, so another school's rows can be named and disabled.
    pub school_id: Uuid,
}

pub(crate) const COURSE_COLUMNS: &str = "id, subject_id, year_level_id, term_id, name, code, description, \
     periods_per_cycle, default_duration_periods, required_room_type_id, \
     created_at, updated_at, rev";
const COURSE_ASSIGNMENTS: &str = "year_level_id = ?, term_id = ?, name = ?, code = ?, \
     description = ?, periods_per_cycle = ?, default_duration_periods = ?, \
     required_room_type_id = ?";

pub(crate) const CLASS_SECTION_COLUMNS: &str =
    "id, course_id, name, code, campus_id, created_at, updated_at, rev";
const CLASS_SECTION_ASSIGNMENTS: &str = "name = ?, code = ?, campus_id = ?";

const CLASS_TEACHER_COLUMNS: &str = "id, class_section_id, teacher_id, created_at, updated_at, rev";
const CLASS_GROUP_COLUMNS: &str =
    "id, class_section_id, student_group_id, created_at, updated_at, rev";
const CLASS_ENROLMENT_COLUMNS: &str =
    "id, class_section_id, student_id, created_at, updated_at, rev";

const ACTIVITY_COLUMNS: &str =
    "id, class_section_id, name, duration_periods, split_index, created_at, updated_at, rev";
const ACTIVITY_ASSIGNMENTS: &str = "name = ?, duration_periods = ?";

/// Resolves the school that owns a course, through its subject.
const COURSE_OWNER: &str = "SELECT subject.school_id FROM course
     JOIN subject ON subject.id = course.subject_id
     WHERE course.id = ?";

/// Resolves the school that owns a class section, through its course and subject.
const SECTION_OWNER: &str = "SELECT subject.school_id FROM class_section
     JOIN course  ON course.id  = class_section.course_id
     JOIN subject ON subject.id = course.subject_id
     WHERE class_section.id = ?";

/// Resolves the school that owns an activity, through its section, course and subject.
const ACTIVITY_OWNER: &str = "SELECT subject.school_id FROM activity
     JOIN class_section ON class_section.id = activity.class_section_id
     JOIN course        ON course.id        = class_section.course_id
     JOIN subject       ON subject.id       = course.subject_id
     WHERE activity.id = ?";

// ------------------------------------------------------------------------------ course

impl Lessons<'_> {
    /// Creates a course as an offering of one of the acting school's subjects.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the subject belongs to another school. A school may
    ///   not build a curriculum on an institution it does not run.
    /// - [`DbError::Invalid`] if the year level, term or room type belongs to another
    ///   school. Checked here as well as by `V15`'s trigger pairs, because a trigger cannot
    ///   name the remedy and a stale picker is exactly how this mistake happens.
    /// - [`DbError::DuplicateValue`] if the name is taken within the subject, or the code
    ///   within the school.
    pub fn create_course(&mut self, school_id: Uuid, input: &CourseInput) -> Result<Course> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_subject(tx, input.subject_id, school_id)?;
            require_course_references(tx, input, school_id)?;
            require_course_code_free(tx, school_id, input.code.as_deref(), None)?;

            insert_course(tx, &stamp, input)?;
            Ok(())
        })?;
        self.course(stamp.id)?
            .ok_or(DbError::NotFound { entity: "course" })
    }

    /// One course by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn course(&self, id: Uuid) -> Result<Option<Course>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {COURSE_COLUMNS} FROM course WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Course::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every course of one school, by subject then name, with the names a listing needs.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn courses(&self, school_id: Uuid) -> Result<Vec<CourseView>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}, y.name, t.name, rt.name,
                        (SELECT COUNT(*) FROM class_section cs WHERE cs.course_id = c.id)
                 FROM course c
                 JOIN subject s ON s.id = c.subject_id
                 LEFT JOIN year_level y ON y.id = c.year_level_id
                 LEFT JOIN term t       ON t.id = c.term_id
                 LEFT JOIN room_type rt ON rt.id = c.required_room_type_id
                 WHERE s.school_id = ?
                 ORDER BY s.name, c.name, c.id",
                qualify(COURSE_COLUMNS, "c"),
                qualify(SUBJECT_COLUMNS, "s"),
            ))?;
            let subject_at = COURSE_COLUMNS.split(", ").count();
            let rest_at = subject_at + SUBJECT_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(school_id)], |row| {
                Ok(CourseView {
                    course: Course::from_row_at(row, 0)?,
                    subject: Subject::from_row_at(row, subject_at)?,
                    year_level_name: row.get(rest_at)?,
                    term_name: row.get(rest_at + 1)?,
                    room_type_name: row.get(rest_at + 2)?,
                    sections: row.get(rest_at + 3)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a course's details.
    ///
    /// Its subject is not among them: a course is an *offering of* a subject, and
    /// repointing one would turn a Mathematics course into a Physics course while keeping
    /// its sections, staff and roster — which is a new course, not an edit.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the course belongs to another school.
    /// - [`DbError::Invalid`] if a reference belongs to another school.
    /// - [`DbError::Conflict`] if somebody else saved first.
    /// - [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_course(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
        input: &CourseInput,
    ) -> Result<Course> {
        self.db.write(|tx| {
            require_school_owner(tx, "course", COURSE_OWNER, id, school_id)?;
            require_course_references(tx, input, school_id)?;
            require_course_code_free(tx, school_id, input.code.as_deref(), Some(id))?;
            update_course_row(tx, id, expected, input)?;
            Ok(())
        })?;
        self.course(id)?
            .ok_or(DbError::NotFound { entity: "course" })
    }

    /// What still refers to a course, so a confirmation can explain before it asks.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn course_dependents(&self, id: Uuid) -> Result<CourseDependents> {
        self.db.read(|conn| {
            let sections: i64 = conn.query_row(
                "SELECT COUNT(*) FROM class_section WHERE course_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;
            Ok(CourseDependents {
                total: sections,
                sections,
            })
        })
    }

    /// Deletes a course that has no class sections.
    ///
    /// # Why this refuses rather than cascading
    ///
    /// A class section is a substantial thing a school built: it has staff on it, an
    /// audience, a roster and a cycle's worth of occurrences. Destroying all of that
    /// because somebody deleted the course above it is the mis-click ADR 0010 exists to
    /// prevent, and the error that replaces it names what is in the way.
    /// [`Lessons::course_dependents`] is what makes that explanation arrive before the
    /// button rather than after it.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if another school owns it.
    /// - [`DbError::StillReferenced`] while any class section delivers it.
    /// - [`DbError::Conflict`] if somebody else edited it since it was read.
    /// - [`DbError::NotFound`] if no such course exists.
    pub fn delete_course(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(tx, "course", COURSE_OWNER, id, school_id)?;
            let sections: i64 = tx.query_row(
                "SELECT COUNT(*) FROM class_section WHERE course_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;
            if sections > 0 {
                return Err(DbError::StillReferenced { entity: "course" });
            }
            delete(tx, "course", "course", id, expected)
        })
    }
}

/// Writes one course row.
///
/// Extracted from [`Lessons::create_course`] so an import can write several kinds of record
/// inside one transaction — see `super::import`. The public method and the importer go
/// through this one statement, so a new column cannot reach one path and miss the other.
pub(crate) fn insert_course(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    input: &CourseInput,
) -> Result<()> {
    tx.execute(
        "INSERT INTO course (id, subject_id, year_level_id, term_id, name, code,
             description, periods_per_cycle, default_duration_periods,
             required_room_type_id, created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(input.subject_id),
            opt_text(input.year_level_id),
            opt_text(input.term_id),
            &input.name,
            &input.code,
            &input.description,
            input.periods_per_cycle,
            input.default_duration_periods,
            opt_text(input.required_room_type_id),
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| translate(error, "course"))?;
    Ok(())
}

/// Writes one course's domain columns, at the revision the caller expects.
///
/// Extracted from [`Lessons::update_course`] so an import can compose several writes inside one
/// transaction — see `super::import`. Both paths go through this one statement, so a new
/// column cannot reach one and miss the other. The school check and anything the public
/// method does *around* the write stay there, because the importer has already done its own.
pub(crate) fn update_course_row(
    tx: &Transaction<'_>,
    id: Uuid,
    expected: Expected,
    input: &CourseInput,
) -> Result<()> {
    update(
        tx,
        "course",
        "course",
        id,
        expected,
        COURSE_ASSIGNMENTS,
        rusqlite::params![
            opt_text(input.year_level_id),
            opt_text(input.term_id),
            &input.name,
            &input.code,
            &input.description,
            input.periods_per_cycle,
            input.default_duration_periods,
            opt_text(input.required_room_type_id),
        ],
    )?;
    Ok(())
}
// ----------------------------------------------------------------------- class section

impl Lessons<'_> {
    /// Creates a class section of one of the acting school's courses.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the course belongs to another school.
    /// - [`DbError::Invalid`] if the campus belongs to another school.
    /// - [`DbError::DuplicateValue`] if the name or code is taken within the course.
    pub fn create_class_section(
        &mut self,
        school_id: Uuid,
        input: &ClassSectionInput,
    ) -> Result<ClassSection> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_school_owner(tx, "course", COURSE_OWNER, input.course_id, school_id)?;
            require_campus_in_school(tx, input.campus_id, school_id)?;

            insert_class_section(tx, &stamp, input)?;
            Ok(())
        })?;
        self.class_section(stamp.id)?.ok_or(DbError::NotFound {
            entity: "class section",
        })
    }

    /// One class section by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn class_section(&self, id: Uuid) -> Result<Option<ClassSection>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CLASS_SECTION_COLUMNS} FROM class_section WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], ClassSection::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every class section of one course, by name, with its staffing, audience and demand.
    ///
    /// The demand arithmetic is [`evara_core::lessons`] and the time-model capacity is read
    /// once for the whole listing, so every row's findings are measured against the same
    /// answer.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the course belongs to another school.
    pub fn class_sections(
        &self,
        school_id: Uuid,
        course_id: Uuid,
    ) -> Result<Vec<ClassSectionView>> {
        self.db.read(|conn| {
            require_school_owner(conn, "course", COURSE_OWNER, course_id, school_id)?;

            let declared: Option<i64> = conn.query_row(
                "SELECT periods_per_cycle FROM course WHERE id = ?",
                [text(course_id)],
                |row| row.get(0),
            )?;
            let capacity = capacity_of(conn, school_id)?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, cam.name,
                        (SELECT COUNT(*) FROM class_teacher ct WHERE ct.class_section_id = cs.id),
                        (SELECT COUNT(*) FROM class_group cg WHERE cg.class_section_id = cs.id),
                        (SELECT COALESCE(SUM(COALESCE(sg.size,
                             (SELECT COUNT(*) FROM student_group_member m
                              WHERE m.student_group_id = sg.id))), 0)
                         FROM class_group cg
                         JOIN student_group sg ON sg.id = cg.student_group_id
                         WHERE cg.class_section_id = cs.id),
                        (SELECT COUNT(*) FROM class_enrolment ce WHERE ce.class_section_id = cs.id)
                 FROM class_section cs
                 LEFT JOIN campus cam ON cam.id = cs.campus_id
                 WHERE cs.course_id = ?
                 ORDER BY cs.name, cs.id",
                qualify(CLASS_SECTION_COLUMNS, "cs"),
            ))?;
            let rest_at = CLASS_SECTION_COLUMNS.split(", ").count();
            let partial = stmt
                .query_map([text(course_id)], |row| {
                    Ok((
                        ClassSection::from_row_at(row, 0)?,
                        row.get::<_, Option<String>>(rest_at)?,
                        row.get::<_, i64>(rest_at + 1)?,
                        row.get::<_, i64>(rest_at + 2)?,
                        row.get::<_, i64>(rest_at + 3)?,
                        row.get::<_, i64>(rest_at + 4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            partial
                .into_iter()
                .map(
                    |(section, campus_name, teachers, groups, group_population, enrolled)| {
                        let demand = demand_of(conn, section.id)?;
                        Ok(ClassSectionView {
                            campus_name,
                            teachers,
                            groups,
                            group_population,
                            enrolled_students: enrolled,
                            declared: demand.against_declared(declared),
                            findings: DemandFinding::of(demand, capacity),
                            demand,
                            section,
                        })
                    },
                )
                .collect()
        })
    }

    /// Replaces a class section's details.
    ///
    /// Its course is not among them, for the reason a course's subject is not: moving a
    /// section between courses would change what is taught to the children on its roster.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if it belongs to another school.
    /// - [`DbError::Invalid`] if the new campus belongs to another school, or if moving the
    ///   section to a campus would strand a teacher who is not eligible there.
    /// - [`DbError::Conflict`] if somebody else saved first.
    /// - [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_class_section(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
        input: &ClassSectionInput,
    ) -> Result<ClassSection> {
        self.db.write(|tx| {
            require_school_owner(tx, "class section", SECTION_OWNER, id, school_id)?;
            require_campus_in_school(tx, input.campus_id, school_id)?;
            // The campus rule of ADR 0028 applies to an existing assignment as much as to a
            // new one, and moving the section is the other way it can be broken. `V15`'s
            // trigger pair fires on an INSERT or UPDATE of `class_teacher`, not of
            // `class_section`, so without this a school could move a section to a campus
            // its own staff are not eligible at and the database would permit it.
            if let Some(campus_id) = input.campus_id {
                let stranded = stranded_by_campus(tx, id, school_id, campus_id)?;
                if stranded > 0 {
                    return Err(DbError::Invalid {
                        message: format!(
                            "{stranded} of this class section's teachers are not eligible at \
                             that campus. Give them that campus on the Teachers screen, or \
                             remove them from this section first."
                        ),
                    });
                }
            }
            update_class_section_row(tx, id, expected, input)?;
            Ok(())
        })?;
        self.class_section(id)?.ok_or(DbError::NotFound {
            entity: "class section",
        })
    }

    /// What a class section holds, so a confirmation can say what deleting it takes.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn class_section_dependents(&self, id: Uuid) -> Result<ClassSectionDependents> {
        self.db.read(|conn| {
            let key = text(id);
            let count = |sql: &str| -> Result<i64> {
                conn.query_row(sql, [&key], |row| row.get(0))
                    .map_err(DbError::from)
            };
            let teachers = count("SELECT COUNT(*) FROM class_teacher WHERE class_section_id = ?")?;
            let groups = count("SELECT COUNT(*) FROM class_group WHERE class_section_id = ?")?;
            let students =
                count("SELECT COUNT(*) FROM class_enrolment WHERE class_section_id = ?")?;
            let activities = count("SELECT COUNT(*) FROM activity WHERE class_section_id = ?")?;
            Ok(ClassSectionDependents {
                total: teachers + groups + students + activities,
                teachers,
                groups,
                enrolled_students: students,
                activities,
            })
        })
    }

    /// Deletes a class section, taking its staffing, audience, roster and occurrences.
    ///
    /// # Why this cascades where [`Lessons::delete_course`] refuses
    ///
    /// Every one of those four things is *about* this section and means nothing without it.
    /// A `class_teacher` row with no section is not history, it is a dangling fact — the
    /// judgement ADR 0025 §5 makes about a qualification with nobody to hold it, and the
    /// one `delete_teacher` and `delete_student_group` already act on. Refusing instead
    /// would mean removing four teachers, two groups and six occurrences one at a time
    /// before a section could be deleted at all.
    ///
    /// Each removal goes through [`super::delete`], inside this transaction, so each is a
    /// truthful change-log entry rather than a silent orphan — which a schema-level
    /// `CASCADE` could not be, because it would not fire the trigger at all.
    ///
    /// [`Lessons::class_section_dependents`] exists so the confirmation says what is about
    /// to go. The counts do not block it.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if another school owns it.
    /// - [`DbError::Conflict`] if somebody else edited the section since it was read.
    /// - [`DbError::NotFound`] if no such class section exists.
    pub fn delete_class_section(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            // Before anything is cleared, not after: clearing a section's staffing and then
            // refusing the delete would be the worst of both outcomes. `delete_teacher`
            // establishes the ordering and the rule is the same here.
            require_school_owner(tx, "class section", SECTION_OWNER, id, school_id)?;

            // `Expected::Any` on each: these are cascades inside somebody else's
            // transaction and the user never saw these rows' revisions. ADR 0027 §4's
            // first documented kind.
            for (entity, table) in [
                ("class teacher", "class_teacher"),
                ("class group", "class_group"),
                ("class enrolment", "class_enrolment"),
                ("activity", "activity"),
            ] {
                for child in children_of_section(tx, table, id)? {
                    delete(tx, entity, table, child, Expected::Any)?;
                }
            }

            delete(tx, "class section", "class_section", id, expected)
        })
    }
}

/// Writes one class section row.
///
/// Extracted from [`Lessons::create_class_section`] so an import can write several kinds of record
/// inside one transaction — see `super::import`. The public method and the importer go
/// through this one statement, so a new column cannot reach one path and miss the other.
pub(crate) fn insert_class_section(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    input: &ClassSectionInput,
) -> Result<()> {
    tx.execute(
        "INSERT INTO class_section (id, course_id, name, code, campus_id,
             created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(input.course_id),
            &input.name,
            &input.code,
            opt_text(input.campus_id),
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| translate(error, "class section"))?;
    Ok(())
}

/// Writes one class section's domain columns, at the revision the caller expects.
///
/// Extracted from [`Lessons::update_class_section`] so an import can compose several writes inside one
/// transaction — see `super::import`. Both paths go through this one statement, so a new
/// column cannot reach one and miss the other. The school check and anything the public
/// method does *around* the write stay there, because the importer has already done its own.
pub(crate) fn update_class_section_row(
    tx: &Transaction<'_>,
    id: Uuid,
    expected: Expected,
    input: &ClassSectionInput,
) -> Result<()> {
    update(
        tx,
        "class section",
        "class_section",
        id,
        expected,
        CLASS_SECTION_ASSIGNMENTS,
        rusqlite::params![&input.name, &input.code, opt_text(input.campus_id)],
    )?;
    Ok(())
}
// --------------------------------------------------------------------- teacher staffing

impl Lessons<'_> {
    /// Every teacher assigned to one class section, by name, with both statuses.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn class_teachers(
        &self,
        school_id: Uuid,
        class_section_id: Uuid,
    ) -> Result<Vec<AssignedTeacher>> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}, {},
                        EXISTS (SELECT 1 FROM teacher_qualification q
                                JOIN class_section cs ON cs.id = ct.class_section_id
                                JOIN course c  ON c.id = cs.course_id
                                WHERE q.teacher_id = ct.teacher_id
                                  AND q.subject_id = c.subject_id)
                 FROM class_teacher ct
                 JOIN teacher t ON t.id = ct.teacher_id
                 JOIN teacher_school ts
                      ON ts.teacher_id = ct.teacher_id AND ts.school_id = ?
                 WHERE ct.class_section_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                qualify(CLASS_TEACHER_COLUMNS, "ct"),
                qualify(TEACHER_COLUMNS, "t"),
                qualify(TEACHER_SCHOOL_COLUMNS, "ts"),
            ))?;
            let teacher_at = CLASS_TEACHER_COLUMNS.split(", ").count();
            let membership_at = teacher_at + TEACHER_COLUMNS.split(", ").count();
            let qualified_at = membership_at + TEACHER_SCHOOL_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(school_id), text(class_section_id)], |row| {
                Ok(AssignedTeacher {
                    assignment: ClassTeacher::from_row_at(row, 0)?,
                    teacher: Teacher::from_row_at(row, teacher_at)?,
                    membership: TeacherSchool::from_row_at(row, membership_at)?,
                    qualified: row.get(qualified_at)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Every teacher this school could give the section, and why each one may or may not.
    ///
    /// The whole of ADR 0028's policy in one read. See [`TeacherCandidate`] and
    /// [`AssignmentEligibility`]: two of the three answers refuse the assignment and the
    /// third only warns about it, and keeping them distinct is the point of the stage.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn assignable_teachers(
        &self,
        school_id: Uuid,
        class_section_id: Uuid,
    ) -> Result<Vec<TeacherCandidate>> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {},
                        EXISTS (SELECT 1 FROM teacher_qualification q
                                JOIN class_section cs ON cs.id = ?
                                JOIN course c ON c.id = cs.course_id
                                WHERE q.teacher_id = t.id AND q.subject_id = c.subject_id),
                        EXISTS (SELECT 1 FROM class_teacher ct
                                WHERE ct.class_section_id = ? AND ct.teacher_id = t.id),
                        -- The narrowing, read exactly as `V15`'s trigger reads it: a
                        -- section with no campus constrains nothing, and a teacher with no
                        -- eligibility recorded at this school is not narrowed.
                        (SELECT cs.campus_id FROM class_section cs WHERE cs.id = ?) IS NULL
                        OR NOT EXISTS (SELECT 1 FROM teacher_campus tc
                                       JOIN campus cam ON cam.id = tc.campus_id
                                       WHERE tc.teacher_id = t.id AND cam.school_id = ?)
                        OR EXISTS (SELECT 1 FROM teacher_campus tc
                                   WHERE tc.teacher_id = t.id
                                     AND tc.campus_id = (SELECT cs.campus_id
                                                         FROM class_section cs
                                                         WHERE cs.id = ?))
                 FROM teacher_school ts
                 JOIN teacher t ON t.id = ts.teacher_id
                 WHERE ts.school_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                qualify(TEACHER_COLUMNS, "t"),
                qualify(TEACHER_SCHOOL_COLUMNS, "ts"),
            ))?;
            let membership_at = TEACHER_COLUMNS.split(", ").count();
            let rest_at = membership_at + TEACHER_SCHOOL_COLUMNS.split(", ").count();

            let section = text(class_section_id);
            let school = text(school_id);
            let rows = stmt.query_map(
                rusqlite::params![&section, &section, &section, &school, &section, &school],
                |row| {
                    let teacher = Teacher::from_row_at(row, 0)?;
                    let membership = TeacherSchool::from_row_at(row, membership_at)?;
                    Ok(TeacherCandidate {
                        eligibility: AssignmentEligibility {
                            employed_and_active: teacher.is_active && membership.is_active,
                            campus_eligible: row.get(rest_at + 2)?,
                            qualified: row.get(rest_at)?,
                        },
                        teacher,
                        membership,
                        assigned: row.get(rest_at + 1)?,
                    })
                },
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Gives one of the acting school's class sections to a teacher.
    ///
    /// # What this requires, and what it merely warns about
    ///
    /// ADR 0025 §4 asked Stage C for a conjunction of three things; ADR 0028 keeps two of
    /// them as requirements and makes the third advisory:
    ///
    /// - an **active** `teacher_school` membership at the section's school — **required**,
    ///   here and by `V15`'s trigger pair. Not merely a membership: an assignment is new
    ///   work, and ADR 0018's rule for a deactivated person is that they keep what they
    ///   have and stop being offered more.
    /// - **campus eligibility** where the section names a campus — **required**, and read
    ///   as a narrowing, so a teacher with no eligibility recorded is not narrowed.
    /// - a **qualification** for the course's subject — **not required**. A school covering
    ///   an absent specialist on Tuesday must be able to say so, and a database that
    ///   refuses will be answered by inventing a qualification, which destroys the record
    ///   everything else rests on. [`Lessons::assignable_teachers`] reports its absence so
    ///   the interface can warn.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::NotFound`] if no such teacher exists.
    /// - [`DbError::Invalid`] if they are not a member of the school, their membership is
    ///   inactive, they are archived, or they are not eligible at the section's campus.
    ///   Each message names its own remedy, because the four are fixed in four different
    ///   places.
    /// - [`DbError::DuplicateValue`] if they already teach it.
    pub fn assign_class_teacher(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        teacher_id: Uuid,
    ) -> Result<ClassTeacher> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            require_assignable_teacher(tx, school_id, class_section_id, teacher_id)?;

            if pair_id(
                tx,
                "SELECT id FROM class_teacher WHERE class_section_id = ? AND teacher_id = ?",
                "class_teacher.id",
                class_section_id,
                teacher_id,
            )?
            .is_some()
            {
                return Err(DbError::DuplicateValue {
                    entity: "class teacher",
                    field: "teacher",
                });
            }

            tx.execute(
                "INSERT INTO class_teacher (id, class_section_id, teacher_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(class_section_id),
                    text(teacher_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "class teacher"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| one_row(conn, CLASS_TEACHER_COLUMNS, "class_teacher", stamp.id))?
            .ok_or(DbError::NotFound {
                entity: "class teacher",
            })
    }

    /// Takes a teacher off one of the acting school's class sections.
    ///
    /// Their membership, their campus eligibility, their departments and every
    /// qualification they hold are untouched: this withdraws one staffing decision.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::Conflict`] if the assignment changed since it was read.
    /// - [`DbError::NotFound`] if they did not teach it.
    pub fn remove_class_teacher(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        teacher_id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            let Some(id) = pair_id(
                tx,
                "SELECT id FROM class_teacher WHERE class_section_id = ? AND teacher_id = ?",
                "class_teacher.id",
                class_section_id,
                teacher_id,
            )?
            else {
                return Err(DbError::NotFound {
                    entity: "class teacher",
                });
            };
            delete(tx, "class teacher", "class_teacher", id, expected)
        })
    }

    /// Every class section one teacher teaches, across every school.
    ///
    /// Project-wide for the reason `qualifications_of_teacher` is: a teacher shared by two
    /// schools of one trust teaches at both, and a listing showing one would hide the
    /// arrangement `V13` exists to express. The caller groups by `schoolId` and makes only
    /// the selected school's rows editable.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn sections_of_teacher(&self, teacher_id: Uuid) -> Result<Vec<TaughtSection>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}, c.name, s.name, s.school_id
                 FROM class_teacher ct
                 JOIN class_section cs ON cs.id = ct.class_section_id
                 JOIN course c  ON c.id = cs.course_id
                 JOIN subject s ON s.id = c.subject_id
                 WHERE ct.teacher_id = ?
                 ORDER BY s.school_id, s.name, c.name, cs.name, cs.id",
                qualify(CLASS_TEACHER_COLUMNS, "ct"),
                qualify(CLASS_SECTION_COLUMNS, "cs"),
            ))?;
            let section_at = CLASS_TEACHER_COLUMNS.split(", ").count();
            let rest_at = section_at + CLASS_SECTION_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(teacher_id)], |row| {
                Ok(TaughtSection {
                    assignment: ClassTeacher::from_row_at(row, 0)?,
                    section: ClassSection::from_row_at(row, section_at)?,
                    course_name: row.get(rest_at)?,
                    subject_name: row.get(rest_at + 1)?,
                    school_id: super::row::FromColumn::read(row, rest_at + 2)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }
}

// ------------------------------------------------------------------ attending groups

impl Lessons<'_> {
    /// Every student group attending one class section, by name, with its population.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn class_groups(
        &self,
        school_id: Uuid,
        class_section_id: Uuid,
    ) -> Result<Vec<AttendingGroup>> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {},
                        (SELECT COUNT(*) FROM student_group_member m
                         WHERE m.student_group_id = sg.id)
                 FROM class_group cg
                 JOIN student_group sg ON sg.id = cg.student_group_id
                 WHERE cg.class_section_id = ?
                 ORDER BY sg.name, sg.id",
                qualify(CLASS_GROUP_COLUMNS, "cg"),
                qualify(STUDENT_GROUP_COLUMNS, "sg"),
            ))?;
            let group_at = CLASS_GROUP_COLUMNS.split(", ").count();
            let members_at = group_at + STUDENT_GROUP_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(class_section_id)], |row| {
                let association = ClassGroup::from_row_at(row, 0)?;
                let group = StudentGroup::from_row_at(row, group_at)?;
                let named_members: i64 = row.get(members_at)?;
                Ok(AttendingGroup {
                    // "NULL means membership is the truth; a number means there are this
                    // many and I am not listing them" — V10's own words for the column.
                    population: group.size.unwrap_or(named_members),
                    association,
                    group,
                    named_members,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Adds one of the acting school's student groups to a class section.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if either the section or the group belongs to another
    ///   school. The group's case is reported as `WrongSchool` rather than `Invalid`
    ///   because the only way to reach it is a stale screen holding another school's
    ///   identifier, and switching schools *is* the remedy.
    /// - [`DbError::NotFound`] if no such group exists.
    /// - [`DbError::DuplicateValue`] if it already attends.
    pub fn assign_class_group(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        student_group_id: Uuid,
    ) -> Result<ClassGroup> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            super::require_same_school(
                tx,
                "student group",
                "student_group",
                student_group_id,
                school_id,
            )?;

            if pair_id(
                tx,
                "SELECT id FROM class_group
                 WHERE class_section_id = ? AND student_group_id = ?",
                "class_group.id",
                class_section_id,
                student_group_id,
            )?
            .is_some()
            {
                return Err(DbError::DuplicateValue {
                    entity: "class group",
                    field: "group",
                });
            }

            tx.execute(
                "INSERT INTO class_group (id, class_section_id, student_group_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(class_section_id),
                    text(student_group_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "class group"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| one_row(conn, CLASS_GROUP_COLUMNS, "class_group", stamp.id))?
            .ok_or(DbError::NotFound {
                entity: "class group",
            })
    }

    /// Takes a student group off one of the acting school's class sections.
    ///
    /// The group itself, its members, its size and its availability are untouched.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::Conflict`] if the association changed since it was read.
    /// - [`DbError::NotFound`] if it was not attending.
    pub fn remove_class_group(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        student_group_id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            let Some(id) = pair_id(
                tx,
                "SELECT id FROM class_group
                 WHERE class_section_id = ? AND student_group_id = ?",
                "class_group.id",
                class_section_id,
                student_group_id,
            )?
            else {
                return Err(DbError::NotFound {
                    entity: "class group",
                });
            };
            delete(tx, "class group", "class_group", id, expected)
        })
    }
}

// -------------------------------------------------------------- individual roster

impl Lessons<'_> {
    /// Every individual on one class section's roster, by name.
    ///
    /// `coveredByGroup` answers "is this child being counted twice?" visibly rather than
    /// resolving it silently. They are not: the group side is the scheduling population in
    /// Milestone 1 and nothing adds the two figures (ADR 0029 §4).
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn class_enrolments(
        &self,
        school_id: Uuid,
        class_section_id: Uuid,
    ) -> Result<Vec<RosteredStudent>> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {},
                        EXISTS (SELECT 1 FROM class_group cg
                                JOIN student_group_member m
                                     ON m.student_group_id = cg.student_group_id
                                WHERE cg.class_section_id = ce.class_section_id
                                  AND m.student_id = ce.student_id)
                 FROM class_enrolment ce
                 JOIN student st ON st.id = ce.student_id
                 WHERE ce.class_section_id = ?
                 ORDER BY st.family_name, st.given_name, st.id",
                qualify(CLASS_ENROLMENT_COLUMNS, "ce"),
                qualify(STUDENT_COLUMNS, "st"),
            ))?;
            let student_at = CLASS_ENROLMENT_COLUMNS.split(", ").count();
            let covered_at = student_at + STUDENT_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(class_section_id)], |row| {
                Ok(RosteredStudent {
                    enrolment: ClassEnrolment::from_row_at(row, 0)?,
                    student: Student::from_row_at(row, student_at)?,
                    covered_by_group: row.get(covered_at)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Puts one named student on a class section's roster.
    ///
    /// Optional throughout: ADR 0006 requires that a timetable be buildable from group
    /// sizes alone, so nothing anywhere treats an empty roster as incomplete.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::NotFound`] if no such student exists.
    /// - [`DbError::Invalid`] if they have no current spell at the section's school — which
    ///   includes the case ADR 0029 §4 is most careful about: a student whose
    ///   project-wide identity exists because *another* school enrolled them. The message
    ///   names enrolling them here as the remedy, because the database's own refusal
    ///   cannot.
    /// - [`DbError::DuplicateValue`] if they are already on it.
    pub fn enrol_in_class(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        student_id: Uuid,
    ) -> Result<ClassEnrolment> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            require_student_at_school(tx, school_id, student_id)?;

            if pair_id(
                tx,
                "SELECT id FROM class_enrolment WHERE class_section_id = ? AND student_id = ?",
                "class_enrolment.id",
                class_section_id,
                student_id,
            )?
            .is_some()
            {
                return Err(DbError::DuplicateValue {
                    entity: "class enrolment",
                    field: "student",
                });
            }

            tx.execute(
                "INSERT INTO class_enrolment (id, class_section_id, student_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(class_section_id),
                    text(student_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "class enrolment"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| one_row(conn, CLASS_ENROLMENT_COLUMNS, "class_enrolment", stamp.id))?
            .ok_or(DbError::NotFound {
                entity: "class enrolment",
            })
    }

    /// Takes one individual off a class section's roster.
    ///
    /// Their identity, their school enrolment and their group memberships are untouched.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::Conflict`] if the roster row changed since it was read.
    /// - [`DbError::NotFound`] if they were not on it.
    pub fn remove_from_class(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        student_id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            let Some(id) = pair_id(
                tx,
                "SELECT id FROM class_enrolment WHERE class_section_id = ? AND student_id = ?",
                "class_enrolment.id",
                class_section_id,
                student_id,
            )?
            else {
                return Err(DbError::NotFound {
                    entity: "class enrolment",
                });
            };
            delete(tx, "class enrolment", "class_enrolment", id, expected)
        })
    }
}

// -------------------------------------------------------------------------- activities

impl Lessons<'_> {
    /// Every occurrence of one class section, in its stable order.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn activities(&self, school_id: Uuid, class_section_id: Uuid) -> Result<Vec<Activity>> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            let mut stmt = conn.prepare(&format!(
                "SELECT {ACTIVITY_COLUMNS} FROM activity
                 WHERE class_section_id = ? ORDER BY split_index, id"
            ))?;
            let rows = stmt.query_map([text(class_section_id)], Activity::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// What a class section asks of a cycle, and anything about it that cannot work.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the section belongs to another school.
    pub fn section_demand(&self, school_id: Uuid, class_section_id: Uuid) -> Result<SectionDemand> {
        self.db.read(|conn| {
            require_school_owner(
                conn,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            let declared: Option<i64> = conn.query_row(
                "SELECT c.periods_per_cycle FROM class_section cs
                 JOIN course c ON c.id = cs.course_id
                 WHERE cs.id = ?",
                [text(class_section_id)],
                |row| row.get(0),
            )?;
            let capacity = capacity_of(conn, school_id)?;
            let demand = demand_of(conn, class_section_id)?;
            Ok(SectionDemand {
                declared: demand.against_declared(declared),
                findings: DemandFinding::of(demand, capacity),
                capacity,
                demand,
            })
        })
    }

    /// Adds `count` identical occurrences to one of the acting school's class sections.
    ///
    /// This is how "four times per cycle" is entered: four rows, each one placeable unit,
    /// each with its own identifier and its own revision. ADR 0029 §2.
    ///
    /// A **creation**, so it carries no revision precondition — ADR 0027 §5: there is no
    /// previous version of a row that does not exist. All `count` rows are written in one
    /// transaction, so a failure part-way leaves none of them rather than three.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::Invalid`] if `count` or `duration_periods` is not positive, or if
    ///   `count` would exceed what one cycle could ever hold. The second is arithmetic,
    ///   not a guess about difficulty — see [`DemandFinding`].
    pub fn add_activities(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
        name: Option<&str>,
        duration_periods: i64,
        count: i64,
    ) -> Result<Vec<Activity>> {
        if duration_periods < 1 {
            return Err(DbError::Invalid {
                message: "An occurrence needs at least one period.".to_owned(),
            });
        }
        if count < 1 {
            return Err(DbError::Invalid {
                message: "Add at least one occurrence.".to_owned(),
            });
        }
        // An upper bound on one call rather than on the section, and deliberately generous:
        // it is here so a mistyped "40" in the count box cannot write forty rows a school
        // then has to delete one at a time, not to express a scheduling rule. The real
        // judgement about whether the demand fits is `DemandFinding`, which reports and
        // refuses nothing.
        if count > MAX_OCCURRENCES_PER_CALL {
            return Err(DbError::Invalid {
                message: format!(
                    "That is more than {MAX_OCCURRENCES_PER_CALL} occurrences in one go. \
                     Add them in smaller batches if a class really needs that many."
                ),
            });
        }

        let written = self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;
            insert_occurrences(tx, class_section_id, name, duration_periods, count)
        })?;

        self.activities(school_id, class_section_id).map(|all| {
            all.into_iter()
                .filter(|a| written.contains(&a.id))
                .collect()
        })
    }

    /// Fills an empty class section with the occurrences its course declares.
    ///
    /// `ROADMAP.md`'s Phase 3 "activity generator": expands `periods_per_cycle` into
    /// occurrences of `default_duration_periods`. Four periods at duration 1 is four rows;
    /// four at duration 2 is two.
    ///
    /// Refuses a section that already has occurrences rather than adding to them, because
    /// "generate" pressed twice must not silently double a school's demand. Clearing the
    /// section first is an explicit act the user can see.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the section belongs to another school.
    /// - [`DbError::Invalid`] if the course declares no `periodsPerCycle` — nothing here
    ///   invents a number — if the section already has occurrences, or if the declaration
    ///   is not a whole multiple of the default duration, which would otherwise be silently
    ///   rounded.
    pub fn generate_activities(
        &mut self,
        school_id: Uuid,
        class_section_id: Uuid,
    ) -> Result<Vec<Activity>> {
        self.db.write(|tx| {
            require_school_owner(
                tx,
                "class section",
                SECTION_OWNER,
                class_section_id,
                school_id,
            )?;

            let existing: i64 = tx.query_row(
                "SELECT COUNT(*) FROM activity WHERE class_section_id = ?",
                [text(class_section_id)],
                |row| row.get(0),
            )?;
            if existing > 0 {
                return Err(DbError::Invalid {
                    message: "This class section already has occurrences. Remove them first \
                              if you want to generate them again from the course."
                        .to_owned(),
                });
            }

            let (declared, duration): (Option<i64>, i64) = tx.query_row(
                "SELECT c.periods_per_cycle, c.default_duration_periods
                 FROM class_section cs JOIN course c ON c.id = cs.course_id
                 WHERE cs.id = ?",
                [text(class_section_id)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let Some(declared) = declared else {
                return Err(DbError::Invalid {
                    message: "This course does not say how many periods per cycle it needs. \
                              Set that on the course, or add occurrences to this section by \
                              hand."
                        .to_owned(),
                });
            };
            if duration < 1 || declared % duration != 0 {
                return Err(DbError::Invalid {
                    message: format!(
                        "{declared} periods per cycle does not divide into occurrences of \
                         {duration}. Adjust the course's periods per cycle or its default \
                         duration."
                    ),
                });
            }

            insert_occurrences(tx, class_section_id, None, duration, declared / duration)?;
            Ok(())
        })?;

        self.activities(school_id, class_section_id)
    }

    /// Replaces one occurrence's name and duration.
    ///
    /// Its section is not among them: an occurrence belongs to the section it is an
    /// occurrence of, and its `splitIndex` is never renumbered.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if it belongs to another school.
    /// - [`DbError::Invalid`] if the duration is not positive.
    /// - [`DbError::Conflict`] if somebody else saved first.
    /// - [`DbError::NotFound`] if no such occurrence exists.
    pub fn update_activity(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
        name: Option<&str>,
        duration_periods: i64,
    ) -> Result<Activity> {
        if duration_periods < 1 {
            return Err(DbError::Invalid {
                message: "An occurrence needs at least one period.".to_owned(),
            });
        }
        self.db.write(|tx| {
            require_school_owner(tx, "activity", ACTIVITY_OWNER, id, school_id)?;
            update(
                tx,
                "activity",
                "activity",
                id,
                expected,
                ACTIVITY_ASSIGNMENTS,
                rusqlite::params![name, duration_periods],
            )
        })?;
        self.db
            .read(|conn| one_row(conn, ACTIVITY_COLUMNS, "activity", id))?
            .ok_or(DbError::NotFound { entity: "activity" })
    }

    /// Deletes one occurrence.
    ///
    /// The remaining occurrences keep their `splitIndex` values, so deleting the second of
    /// four leaves 1, 3, 4. Renumbering them would rewrite two rows the user did not touch
    /// and consume their revisions, which would make a concurrent edit to one of them fail
    /// for no reason the user could see.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if it belongs to another school.
    /// - [`DbError::Conflict`] if it changed since it was read.
    /// - [`DbError::NotFound`] if no such occurrence exists.
    pub fn delete_activity(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(tx, "activity", ACTIVITY_OWNER, id, school_id)?;
            delete(tx, "activity", "activity", id, expected)
        })
    }
}

// -------------------------------------------------------- what a lesson row blocks
//
// Two project-wide counts, read by the People screens *before* they offer a delete. They
// are reads about a student or a group rather than about a lesson, and they live here
// because the tables they count are this module's — the same arrangement `curriculum` has
// with `people`.

impl Lessons<'_> {
    /// How many class sections one student is on the roster of, across every school.
    ///
    /// Read by the Students screen so a blocked delete *explains* rather than fails:
    /// `delete_student` is refused while any remain, which is ADR 0018's treatment of
    /// group membership applied to a class roster.
    ///
    /// Project-wide, not school-scoped, because the delete it informs is about the person
    /// and a count that hid another school's rows would understate what is in the way.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn class_count_of_student(&self, student_id: Uuid) -> Result<i64> {
        self.db.read(|conn| rosters_of_student(conn, student_id))
    }

    /// How many class sections one student group attends.
    ///
    /// Read by the Student groups screen, for the same reason and with the same effect:
    /// `delete_student_group` is refused while any remain.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn class_count_of_group(&self, student_group_id: Uuid) -> Result<i64> {
        self.db
            .read(|conn| sections_of_group(conn, student_group_id))
    }
}

/// How much of a cycle one class section asks for, and what the cycle can hold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SectionDemand {
    /// Summed over the section's occurrences.
    pub demand: LessonDemand,
    /// How that stands against the course's declaration. Advice, never a refusal.
    pub declared: DeclaredComparison,
    /// What the school's cycle and bell schedules offer. Zeroes before they exist.
    pub capacity: CycleCapacity,
    /// Impossibilities in the demand. **Empty does not mean schedulable.**
    pub findings: Vec<DemandFinding>,
}

/// The most occurrences one `add_activities` call will write.
///
/// A typo guard, not a scheduling rule. See [`Lessons::add_activities`].
const MAX_OCCURRENCES_PER_CALL: i64 = 40;

// ============================================================================ helpers

/// Prefixes every column in a `SELECT` list with a table alias.
///
/// The record readers decode positionally in declaration order, so a joined query has to
/// name the same columns in the same order — writing them out twice is what drifts. Same
/// helper as [`super::curriculum`]'s and [`super::membership`]'s, kept local for the same
/// reason: it is three lines, and a cross-module `pub(crate)` for it would be a worse
/// coupling than the duplication.
fn qualify(columns: &str, alias: &str) -> String {
    columns
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reads one row of one table by identifier.
fn one_row<T>(conn: &Connection, columns: &str, table: &str, id: Uuid) -> Result<Option<T>>
where
    T: RowRecord,
{
    let table = super::identifier(table)?;
    let mut stmt = conn.prepare(&format!("SELECT {columns} FROM {table} WHERE id = ?"))?;
    let mut rows = stmt.query_map([text(id)], T::read)?;
    rows.next().transpose().map_err(DbError::from)
}

/// A record the [`entity!`] macro generated a positional reader for.
///
/// Exists only so [`one_row`] can be written once for the four relationship tables rather
/// than four times. Nothing outside this module implements it.
trait RowRecord: Sized {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

macro_rules! row_record {
    ($($ty:ty),* $(,)?) => {
        $(impl RowRecord for $ty {
            fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> { Self::from_row(row) }
        })*
    };
}

row_record!(ClassTeacher, ClassGroup, ClassEnrolment, Activity);

/// Refuses a subject that belongs to another school.
fn require_subject(tx: &Transaction<'_>, subject_id: Uuid, school_id: Uuid) -> Result<()> {
    super::require_same_school(tx, "subject", "subject", subject_id, school_id)
}

/// Refuses a course whose year level, term or required room type is another school's.
///
/// `Invalid` rather than `WrongSchool`, deliberately, and for the reason
/// `require_department_in_school` gives: the *write* is for the right school, and it is one
/// field within it that names somebody else's row. `WrongSchool` would send the user to
/// switch schools, which is the wrong advice when the fix is to pick a different year
/// level.
fn require_course_references(
    tx: &Transaction<'_>,
    input: &CourseInput,
    school_id: Uuid,
) -> Result<()> {
    if let Some(year_level_id) = input.year_level_id {
        require_owned_by(
            tx,
            "year level",
            "SELECT school_id FROM year_level WHERE id = ?",
            year_level_id,
            school_id,
            "That year level belongs to a different school. Choose one of this school's \
             year levels, or leave the course without one.",
        )?;
    }
    if let Some(term_id) = input.term_id {
        require_owned_by(
            tx,
            "term",
            "SELECT academic_year.school_id FROM term
             JOIN academic_year ON academic_year.id = term.academic_year_id
             WHERE term.id = ?",
            term_id,
            school_id,
            "That term belongs to a different school. Choose one of this school's terms, \
             or leave the course running all year.",
        )?;
    }
    if let Some(room_type_id) = input.required_room_type_id {
        require_owned_by(
            tx,
            "room type",
            "SELECT school_id FROM room_type WHERE id = ?",
            room_type_id,
            school_id,
            "That room type belongs to a different school. Choose one of this school's \
             room types, or leave the course without a room requirement.",
        )?;
    }
    Ok(())
}

/// Refuses a campus that belongs to another school, or accepts `None`.
fn require_campus_in_school(
    tx: &Transaction<'_>,
    campus_id: Option<Uuid>,
    school_id: Uuid,
) -> Result<()> {
    match campus_id {
        None => Ok(()),
        Some(campus_id) => require_owned_by(
            tx,
            "campus",
            "SELECT school_id FROM campus WHERE id = ?",
            campus_id,
            school_id,
            "That campus belongs to a different school. Choose one of this school's \
             campuses, or leave the class section without one.",
        ),
    }
}

/// Refuses a reference whose owner query returns a different school.
fn require_owned_by(
    tx: &Transaction<'_>,
    entity: &'static str,
    owner_query: &str,
    id: Uuid,
    school_id: Uuid,
    remedy: &str,
) -> Result<()> {
    let owner: Option<String> = tx
        .query_row(owner_query, [text(id)], |row| row.get(0))
        .optional()?;
    match owner {
        None => Err(DbError::NotFound { entity }),
        Some(raw) if raw == school_id.to_string() => Ok(()),
        Some(_) => Err(DbError::Invalid {
            message: remedy.to_owned(),
        }),
    }
}

/// Refuses a course code already used by another course of the same school.
///
/// `V15` holds this as a trigger pair rather than a partial unique index, because the scope
/// is the school and `course` deliberately has no `school_id`. This is the repository half
/// of that division: `repo::translate` reads SQLite's own constraint text and cannot see a
/// `RAISE` message, so without this the user would get an internal error instead of
/// "another course already uses that code" pinned to the Code field.
///
/// `exclude` is the course being updated, which must not collide with itself.
fn require_course_code_free(
    tx: &Transaction<'_>,
    school_id: Uuid,
    code: Option<&str>,
    exclude: Option<Uuid>,
) -> Result<()> {
    let Some(code) = code else { return Ok(()) };

    let taken: i64 = tx.query_row(
        "SELECT COUNT(*) FROM course c
         JOIN subject s ON s.id = c.subject_id
         WHERE s.school_id = ? AND c.code = ? AND c.id <> ?",
        rusqlite::params![
            text(school_id),
            code,
            exclude.map_or_else(String::new, text)
        ],
        |row| row.get(0),
    )?;

    if taken > 0 {
        return Err(DbError::DuplicateValue {
            entity: "course",
            field: "code",
        });
    }
    Ok(())
}

/// Refuses a teacher this school may not give new work to, naming which of four reasons.
///
/// Four separate messages rather than one, because the four are fixed in four different
/// places and a single "cannot be assigned" would send the user looking in the wrong one.
/// The trigger pair in `V15` enforces two of them again underneath; this is the half that
/// can name a remedy.
fn require_assignable_teacher(
    tx: &Transaction<'_>,
    school_id: Uuid,
    class_section_id: Uuid,
    teacher_id: Uuid,
) -> Result<()> {
    let found: Option<(bool, bool)> = tx
        .query_row(
            "SELECT t.is_active, ts.is_active FROM teacher t
             LEFT JOIN teacher_school ts
                  ON ts.teacher_id = t.id AND ts.school_id = ?
             WHERE t.id = ?",
            [text(school_id), text(teacher_id)],
            |row| Ok((row.get(0)?, row.get::<_, Option<bool>>(1)?.unwrap_or(false))),
        )
        .optional()?;

    let Some((person_active, membership_active)) = found else {
        return Err(DbError::NotFound { entity: "teacher" });
    };

    // Membership first: "not employed here" is a more fundamental answer than "archived",
    // and a visiting teacher this school has never employed is the common case.
    let employed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM teacher_school WHERE teacher_id = ? AND school_id = ?",
        [text(teacher_id), text(school_id)],
        |row| row.get(0),
    )?;
    if employed == 0 {
        return Err(DbError::Invalid {
            message: "Add this teacher to the school before giving them one of its classes. \
                      Being qualified for a subject is not the same as being employed here."
                .to_owned(),
        });
    }
    if !membership_active {
        return Err(DbError::Invalid {
            message: "This teacher is recorded as no longer teaching at this school, so they \
                      cannot be given new classes. The classes they already have are \
                      untouched. Set “Currently teaching” on the Teachers screen to assign \
                      them again."
                .to_owned(),
        });
    }
    if !person_active {
        return Err(DbError::Invalid {
            message: "This teacher is archived in this campus file, so they cannot be given \
                      new classes. Restore them on the Teachers screen first."
                .to_owned(),
        });
    }

    if !campus_eligible(tx, class_section_id, teacher_id)? {
        return Err(DbError::Invalid {
            message: "This teacher is not eligible at the campus this class section is \
                      taught at. Add that campus to them on the Teachers screen, or clear \
                      the section's campus."
                .to_owned(),
        });
    }
    Ok(())
}

/// Whether a teacher may be based where a class section is taught.
///
/// Reads the narrowing exactly as `V15`'s trigger does, and the middle condition is the
/// subtlety: `teacher_campus` is a *narrowing* of membership (ADR 0021 §3), so a teacher
/// with no eligibility recorded at this school is not narrowed and is eligible anywhere in
/// it. Reading an empty set as "eligible nowhere" would make every teacher in a
/// multi-campus school unassignable until somebody filled the table in.
fn campus_eligible(tx: &Transaction<'_>, class_section_id: Uuid, teacher_id: Uuid) -> Result<bool> {
    let eligible: bool = tx.query_row(
        "SELECT
             (SELECT cs.campus_id FROM class_section cs WHERE cs.id = ?) IS NULL
             OR NOT EXISTS (
                 SELECT 1 FROM teacher_campus tc
                 JOIN campus cam ON cam.id = tc.campus_id
                 JOIN class_section cs ON cs.id = ?
                 JOIN course c  ON c.id = cs.course_id
                 JOIN subject s ON s.id = c.subject_id
                 WHERE tc.teacher_id = ? AND cam.school_id = s.school_id)
             OR EXISTS (
                 SELECT 1 FROM teacher_campus tc
                 WHERE tc.teacher_id = ?
                   AND tc.campus_id = (SELECT cs.campus_id FROM class_section cs
                                       WHERE cs.id = ?))",
        rusqlite::params![
            text(class_section_id),
            text(class_section_id),
            text(teacher_id),
            text(teacher_id),
            text(class_section_id),
        ],
        |row| row.get(0),
    )?;
    Ok(eligible)
}

/// How many of a section's teachers would not be eligible at a given campus.
///
/// Asked before a section is moved to a campus, because `V15`'s trigger pair fires on a
/// write to `class_teacher` and not on a write to `class_section` — so the section is the
/// other way the campus rule can be broken.
fn stranded_by_campus(
    tx: &Transaction<'_>,
    class_section_id: Uuid,
    school_id: Uuid,
    campus_id: Uuid,
) -> Result<i64> {
    tx.query_row(
        "SELECT COUNT(*) FROM class_teacher ct
         WHERE ct.class_section_id = ?
           AND EXISTS (SELECT 1 FROM teacher_campus tc
                       JOIN campus cam ON cam.id = tc.campus_id
                       WHERE tc.teacher_id = ct.teacher_id AND cam.school_id = ?)
           AND NOT EXISTS (SELECT 1 FROM teacher_campus tc
                           WHERE tc.teacher_id = ct.teacher_id AND tc.campus_id = ?)",
        [text(class_section_id), text(school_id), text(campus_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// Refuses a student with no current spell at this school.
///
/// The multi-school safety ADR 0029 §4 is most careful about: a student's project-wide
/// identity exists the moment *any* school enrolled them, and that is not eligibility to
/// sit in this school's class.
fn require_student_at_school(
    tx: &Transaction<'_>,
    school_id: Uuid,
    student_id: Uuid,
) -> Result<()> {
    let exists: i64 = tx.query_row(
        "SELECT COUNT(*) FROM student WHERE id = ?",
        [text(student_id)],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Err(DbError::NotFound { entity: "student" });
    }

    let current: i64 = tx.query_row(
        "SELECT COUNT(*) FROM student_school_enrolment
         WHERE student_id = ? AND school_id = ? AND status <> 'LEFT'",
        [text(student_id), text(school_id)],
        |row| row.get(0),
    )?;
    if current == 0 {
        return Err(DbError::Invalid {
            message: "This student is not currently enrolled at this school, so they cannot \
                      be put in one of its classes. Enrol them on the Students screen first \
                      — a student another school has enrolled is not thereby a student here."
                .to_owned(),
        });
    }
    Ok(())
}

/// Writes `count` occurrences, numbering them from the section's current high-water mark.
///
/// Shared by [`Lessons::add_activities`] and [`Lessons::generate_activities`] so the
/// numbering rule lives in one place. `split_index` continues from `MAX + 1` and is never
/// reused, so an occurrence deleted and replaced does not inherit a number another row's
/// change-log history is about.
pub(crate) fn insert_occurrences(
    tx: &Transaction<'_>,
    class_section_id: Uuid,
    name: Option<&str>,
    duration_periods: i64,
    count: i64,
) -> Result<Vec<Uuid>> {
    let next: i64 = tx.query_row(
        "SELECT COALESCE(MAX(split_index), 0) + 1 FROM activity WHERE class_section_id = ?",
        [text(class_section_id)],
        |row| row.get(0),
    )?;

    let mut written = Vec::new();
    for offset in 0..count {
        let stamp = Stamp::new();
        tx.execute(
            "INSERT INTO activity (id, class_section_id, name, duration_periods, split_index,
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                text(stamp.id),
                text(class_section_id),
                name,
                duration_periods,
                next + offset,
                &stamp.created_at,
                &stamp.updated_at,
                stamp.rev,
            ],
        )
        .map_err(|error| translate(error, "activity"))?;
        written.push(stamp.id);
    }
    Ok(written)
}

/// Sums one section's occurrences into a demand.
fn demand_of(conn: &Connection, class_section_id: Uuid) -> Result<LessonDemand> {
    let mut stmt =
        conn.prepare("SELECT duration_periods FROM activity WHERE class_section_id = ?")?;
    let durations = stmt
        .query_map([text(class_section_id)], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(LessonDemand::of(&durations))
}

/// Measures what one school's cycle and bell schedules could hold.
///
/// Reads the flags; [`CycleCapacity::from_schedules`] does the counting, because "longest
/// unbroken run of teaching periods" is a rule about the model and belongs in `evara-core`
/// where the solver can reach the same answer.
///
/// The cycle taken is the school's **default** when it has one, and otherwise its longest —
/// an occurrence only has to fit somewhere, and a school part-way through setting up a
/// second cycle has not thereby made its lessons impossible.
fn capacity_of(conn: &Connection, school_id: Uuid) -> Result<CycleCapacity> {
    let cycle_days: i64 = conn
        .query_row(
            "SELECT COUNT(cd.id) FROM cycle c
             LEFT JOIN cycle_day cd ON cd.cycle_id = c.id
             WHERE c.school_id = ?
             GROUP BY c.id
             ORDER BY c.is_default DESC, COUNT(cd.id) DESC
             LIMIT 1",
            [text(school_id)],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);

    let mut stmt = conn.prepare(
        "SELECT p.period_structure_id, p.kind FROM period p
         JOIN period_structure ps ON ps.id = p.period_structure_id
         WHERE ps.school_id = ?
         ORDER BY p.period_structure_id, p.ordinal",
    )?;
    let flags = stmt
        .query_map([text(school_id)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    // Grouped in Rust rather than by a window function, because the rows arrive already
    // ordered by structure and then by ordinal — which is the order
    // `from_schedules` needs, and the order a run-length rule cannot be computed without.
    let mut schedules: Vec<Vec<bool>> = Vec::new();
    let mut current_structure: Option<String> = None;
    for (structure, kind) in flags {
        if current_structure.as_ref() != Some(&structure) {
            schedules.push(Vec::new());
            current_structure = Some(structure);
        }
        if let Some(last) = schedules.last_mut() {
            last.push(kind == "TEACHING");
        }
    }

    Ok(CycleCapacity::from_schedules(cycle_days, &schedules))
}

/// Reads one identifier addressed by a pair of identifiers.
fn pair_id(
    tx: &Transaction<'_>,
    sql: &str,
    column: &'static str,
    first: Uuid,
    second: Uuid,
) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(sql, [text(first), text(second)], |row| row.get(0))
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text(column, value))
        .transpose()
}

/// Every row of one child table belonging to one class section, by identifier.
fn children_of_section(
    tx: &Transaction<'_>,
    table: &str,
    class_section_id: Uuid,
) -> Result<Vec<Uuid>> {
    let table = super::identifier(table)?;
    ids(
        tx,
        &format!("SELECT id FROM {table} WHERE class_section_id = ? ORDER BY id"),
        "id",
        &[&text(class_section_id)],
    )
}

/// Reads a list of identifiers, failing on a damaged one rather than skipping it.
fn ids(
    tx: &Transaction<'_>,
    sql: &str,
    column: &'static str,
    params: &[&dyn ToSql],
) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map(params, |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .map(|value| parse_id_text(column, value))
        .collect()
}

// --------------------------------------------------------- cross-repository helpers
//
// Called by the repositories whose deletes this stage changed. They live here, next to the
// tables they are about, so the knowledge of what a lesson row means is in one place — the
// same arrangement `curriculum` already has with `people` and `membership`.

/// How many courses are offerings of one subject.
///
/// Used by `Curriculum::subject_dependents` and `delete_subject`, so a subject a school is
/// still teaching is *explained* rather than merely refused. `course.subject_id` is
/// `RESTRICT`, exactly as `ROADMAP.md`'s Phase 3 notes said it would be.
pub(crate) fn courses_of_subject(conn: &Connection, subject_id: Uuid) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM course WHERE subject_id = ?",
        [text(subject_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// Every class-section assignment one teacher holds, by identifier.
///
/// Used by `delete_teacher`. Destroying the person is the one operation that may take
/// these, for the reason ADR 0025 §5 gives about a qualification: an assignment with nobody
/// to hold it is not history, it is a dangling fact.
pub(crate) fn assignments_of_teacher(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Vec<Uuid>> {
    ids(
        tx,
        "SELECT id FROM class_teacher WHERE teacher_id = ? ORDER BY id",
        "class_teacher.id",
        &[&text(teacher_id)],
    )
}

/// How many of one school's class sections a teacher still teaches.
///
/// Used by `Membership::remove_teacher_from_school`, which **refuses** while any remain —
/// unlike campus eligibility and department associations, which it clears. `V15`'s
/// `teacher_school_delete_requires_no_class_assignment` is the schema half of that refusal;
/// this is the half that can report the count so the message names what is in the way.
pub(crate) fn assignments_in_school(
    conn: &Connection,
    teacher_id: Uuid,
    school_id: Uuid,
) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM class_teacher ct
         JOIN class_section cs ON cs.id = ct.class_section_id
         JOIN course c  ON c.id = cs.course_id
         JOIN subject s ON s.id = c.subject_id
         WHERE ct.teacher_id = ? AND s.school_id = ?",
        [text(teacher_id), text(school_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// How many class sections one student group attends.
///
/// Used by `People::student_group_dependents` and `delete_student_group`, which refuses
/// while any remain. A group with members, a parent, a size and availability on it is
/// substantial, and deleting one that three class sections are taught to is almost always
/// the wrong row.
pub(crate) fn sections_of_group(conn: &Connection, student_group_id: Uuid) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM class_group WHERE student_group_id = ?",
        [text(student_group_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// How many class rosters one student is on.
///
/// Used by `delete_student`, which refuses while any remain — the treatment ADR 0018
/// already gives group membership, on the ground that the dependent row *is* the history.
pub(crate) fn rosters_of_student(conn: &Connection, student_id: Uuid) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM class_enrolment WHERE student_id = ?",
        [text(student_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}
