//! Repositories for the People group: year levels, teachers, students and groups.
//!
//! Same shape as [`super::structure`] — callers pass an input struct and receive a record,
//! no `rusqlite` type appears in a signature, and every write goes through
//! [`super::update`] or [`super::delete`] so `rev` and `updated_at` cannot be forgotten.
//!
//! # Deletion, and why it differs between a group and a student
//!
//! Both are RESTRICT in the schema, and both are cleared explicitly here rather than by a
//! foreign-key action, so every removal is a real change-log entry. What differs is the
//! answer to "did the user mean this?":
//!
//! - **Deleting a group** removes its own membership rows. A membership has no meaning
//!   without the group on one end of it, and [`People::student_group_member_count`] exists
//!   so the interface can say *"this removes 24 memberships; the students are not
//!   deleted"* before anything happens. Told, then done — not silent.
//! - **Deleting a student** who is still in a group is refused. That is the case where
//!   the record is history: the student was in 9-MAT-A last year, and a school clearing
//!   out leavers would otherwise erase the only evidence of it. [`StudentInput::is_active`]
//!   is what they actually wanted, and the error says so.
//!
//! An inactive teacher or student is **not** a deleted one. They keep every row they ever
//! had and simply stop being offered for new work; nothing here filters them out, because
//! deciding who to offer is the interface's business and a timetable that already names
//! them must keep resolving.
//!
//! # Group hierarchy
//!
//! `student_group.parent_group_id` is what lets clash detection know that a cohort and the
//! teaching sets it was split into are the same students. SQLite can refuse a group that
//! is its own parent with a `CHECK`, but it cannot express "no cycles anywhere", so
//! [`People::update_student_group`] walks the ancestry before writing. A cycle would make
//! that resolution non-terminating, which is a hang rather than an error message.

use rusqlite::{OptionalExtension, Transaction};
use uuid::Uuid;

use super::availability::{AvailabilityEntity, clear_for_entity};
use super::curriculum::{associations_of, qualifications_of};
use super::lessons::{assignments_of_teacher, rosters_of_student, sections_of_group};
use super::membership::{
    EnrolmentStatus, StudentEnrolmentInput, eligibility_of_teacher, enrolments_of_student,
    insert_enrolment, insert_membership, memberships_of_teacher, mirror_student_onto_enrolment,
    other_employers, other_schools_attended,
};
use super::row::{entity, opt_text, text};
use super::{Expected, Stamp, delete, require_same_school, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing People entities.
///
/// Obtained from [`Database::people`]. Holds the database mutably because most operations
/// write.
#[derive(Debug)]
pub struct People<'a> {
    db: &'a mut Database,
}

impl<'a> People<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

entity! {
    /// A stage of progression through the school: "Year 9", "Grade 8", "Sixth Form".
    YearLevel,
    /// Values for creating or replacing a year level.
    YearLevelInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name, unique within the school.
        name: String,
        /// The school's own progression order, lowest first.
        ///
        /// Not an age and not a date. Nothing derives a year from a birthday, for the same
        /// reason nothing derives a cycle day from a weekday.
        ordinal: i64,
    }
}

entity! {
    /// Someone who teaches. Identity is stable across terms and academic years.
    Teacher,
    /// Values for creating or replacing a teacher.
    TeacherInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Optional staff or payroll reference. Unique within the school when given.
        code: Option<String>,
        /// Given (first) name.
        given_name: String,
        /// Family (last) name.
        family_name: String,
        /// Optional contact address. Never a credential — Evara has no accounts.
        email: Option<String>,
        /// Campus they are normally based at, if the school has more than one.
        home_campus_id: Option<Uuid>,
        /// Full-time equivalent as a percentage: `100` is full time, `50` a half timetable.
        ///
        /// An integer rather than a fraction because the Phase 4 load constraints compare
        /// and sum it, and this crate keeps that arithmetic exact.
        fte_percent: i64,
        /// Load ceiling per cycle day, if the school sets one.
        max_periods_per_day: Option<i64>,
        /// Load ceiling per whole cycle, if the school sets one.
        max_periods_per_cycle: Option<i64>,
        /// How many periods in a row they may be timetabled for.
        max_consecutive_periods: Option<i64>,
        /// Free text for whatever the school needs to remember.
        notes: Option<String>,
        /// Whether they are currently teaching.
        ///
        /// Employment state, not a soft delete. An inactive teacher keeps every assignment
        /// they ever had; they are simply not offered for new ones.
        is_active: bool,
    }
}

entity! {
    /// A student. Identity is stable and independent of any one year's enrolment.
    Student,
    /// Values for creating or replacing a student.
    StudentInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Optional admission or roll number. Unique within the school when given.
        code: Option<String>,
        /// Given (first) name.
        given_name: String,
        /// Family (last) name.
        family_name: String,
        /// Optional contact address. Never a credential — Evara has no accounts.
        email: Option<String>,
        /// Which year they are in, if the school tracks that.
        year_level_id: Option<Uuid>,
        /// Campus they attend, if the school has more than one.
        campus_id: Option<Uuid>,
        /// Whether they are currently enrolled.
        ///
        /// A student who has left keeps their group history.
        is_active: bool,
    }
}

entity! {
    /// An audience a lesson can be taught to: a class, a cohort, a homeroom, an elective set.
    ///
    /// Deliberately one table rather than five. See `DATA-MODEL.md` §4.
    StudentGroup,
    /// Values for creating or replacing a student group.
    StudentGroupInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name, unique within the school.
        name: String,
        /// Optional short code, unique within the school when given.
        code: Option<String>,
        /// What sort of audience this is.
        kind: GroupKind,
        /// Which year it belongs to, if any.
        year_level_id: Option<Uuid>,
        /// The group this one was split out of, if any.
        ///
        /// Three teaching sets under one cohort clash with the cohort but not with each
        /// other. This is the edge that says so.
        parent_group_id: Option<Uuid>,
        /// Declared head count, for a school that does not list students.
        ///
        /// `None` means membership is the truth. ADR 0006 requires a school to be able to
        /// timetable without entering a single student, which is what this is for.
        size: Option<i64>,
        /// Campus it is taught on, if the school has more than one.
        campus_id: Option<Uuid>,
    }
}

/// What sort of audience a [`StudentGroup`] is.
///
/// Carried as a string in the schema and checked there too, so a project opened by an
/// older build fails loudly rather than reading an unknown kind as a plausible one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum GroupKind {
    /// A taught class.
    Class,
    /// A whole year or band.
    Cohort,
    /// A pastoral registration group.
    Homeroom,
    /// The students who chose one option.
    ElectiveSet,
    /// A set formed for one subject, typically by attainment.
    TeachingSet,
}

impl GroupKind {
    /// The value stored in `student_group.kind`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "CLASS",
            Self::Cohort => "COHORT",
            Self::Homeroom => "HOMEROOM",
            Self::ElectiveSet => "ELECTIVE_SET",
            Self::TeachingSet => "TEACHING_SET",
        }
    }
}

impl super::row::FromColumn for GroupKind {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "CLASS" => Ok(Self::Class),
            "COHORT" => Ok(Self::Cohort),
            "HOMEROOM" => Ok(Self::Homeroom),
            "ELECTIVE_SET" => Ok(Self::ElectiveSet),
            "TEACHING_SET" => Ok(Self::TeachingSet),
            // A kind the schema's CHECK should have refused is corruption, not input —
            // the same rule identifiers follow. Guessing "probably a class" would make a
            // damaged file look like a working one.
            _ => Err(rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(crate::error::CorruptId {
                    column: "student_group.kind".to_owned(),
                    value: raw,
                }),
            )),
        }
    }
}

// ------------------------------------------------------------------------ year level

const YEAR_LEVEL_COLUMNS: &str = "id, school_id, name, ordinal, created_at, updated_at, rev";

impl People<'_> {
    /// Creates a year level.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name or the ordinal is taken within the school.
    pub fn create_year_level(&mut self, input: &YearLevelInput) -> Result<YearLevel> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO year_level (id, school_id, name, ordinal, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    input.ordinal,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "year level"))?;
            Ok(())
        })?;
        self.year_level(stamp.id)?.ok_or(DbError::NotFound {
            entity: "year level",
        })
    }

    /// One year level by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn year_level(&self, id: Uuid) -> Result<Option<YearLevel>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {YEAR_LEVEL_COLUMNS} FROM year_level WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], YearLevel::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every year level of one school, in that school's own progression order.
    ///
    /// Scoped to a school since V12, and this is the list where it matters most: year
    /// levels are never shared, and `UNIQUE (school_id, name)` means two schools may each
    /// hold their own "Grade 1" at their own ordinal 1. An unscoped list would interleave
    /// them and make both meaningless.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn year_levels(&self, school_id: Uuid) -> Result<Vec<YearLevel>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {YEAR_LEVEL_COLUMNS} FROM year_level WHERE school_id = ? \
                 ORDER BY ordinal, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], YearLevel::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a year level's details. Its school never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_year_level(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &YearLevelInput,
    ) -> Result<YearLevel> {
        self.db.write(|tx| {
            require_same_school(tx, "year level", "year_level", id, input.school_id)?;
            update(
                tx,
                "year level",
                "year_level",
                id,
                expected,
                "name = ?, ordinal = ?",
                &[&input.name, &input.ordinal],
            )
        })?;
        self.year_level(id)?.ok_or(DbError::NotFound {
            entity: "year level",
        })
    }

    /// Deletes a year level.
    ///
    /// Students and groups classified by it are detached first, so the SET NULL path in
    /// the schema stays a backstop rather than the normal route — a database-level
    /// SET NULL changes a row without incrementing `rev`, which would make the change feed
    /// lie about what happened.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if no such year level exists.
    pub fn delete_year_level(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "year level", "year_level", id, school_id)?;
            detach_year_level(tx, "student", id)?;
            detach_year_level(tx, "student_group", id)?;
            // V13 added a third table that classifies by year level, and it is the
            // authoritative one. Leaving it to the schema's SET NULL would change rows
            // without incrementing `rev`, which is the thing this helper exists to avoid.
            detach_year_level(tx, "student_school_enrolment", id)?;
            // `V15`'s. A course may name the year level it is for, and losing the year
            // level leaves a complete course that is simply no longer labelled — so this is
            // ADR 0010's "optional classification, therefore SET NULL" case, cleared here
            // through `update` first so each affected course's `rev` stays truthful. It is
            // deliberately *not* treated the way `subject.department_id` is: ADR 0025 §6
            // departed from ADR 0010 there partly because refusing is what a school wants
            // from a department still in use, and that argument does not carry over.
            detach_year_level(tx, "course", id)?;
            delete(tx, "year level", "year_level", id, expected)
        })
    }
}

/// Clears `year_level_id` on every row of one table that names this year level.
///
/// Goes through [`update`] per row rather than one bulk `UPDATE`, so each row's `rev` is
/// incremented and each change-log entry is truthful.
fn detach_year_level(tx: &Transaction<'_>, table: &'static str, year_level_id: Uuid) -> Result<()> {
    let ids = {
        let mut stmt = tx.prepare(&format!(
            "SELECT id FROM {} WHERE year_level_id = ?",
            super::identifier(table)?
        ))?;
        let rows = stmt.query_map([text(year_level_id)], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for raw in ids {
        let id = super::row::parse_id_text("id", &raw)?;
        update(
            tx,
            table,
            table,
            id,
            Expected::Any,
            "year_level_id = ?",
            &[&None::<String>],
        )?;
    }
    Ok(())
}

// --------------------------------------------------------------------------- teacher

pub(crate) const TEACHER_COLUMNS: &str = "id, school_id, code, given_name, family_name, email, \
     home_campus_id, fte_percent, max_periods_per_day, max_periods_per_cycle, \
     max_consecutive_periods, notes, is_active, created_at, updated_at, rev";

/// The assignments and bindings shared by teacher insert and update.
const TEACHER_ASSIGNMENTS: &str = "code = ?, given_name = ?, family_name = ?, email = ?, \
     home_campus_id = ?, fte_percent = ?, max_periods_per_day = ?, max_periods_per_cycle = ?, \
     max_consecutive_periods = ?, notes = ?, is_active = ?";

impl People<'_> {
    /// Creates a teacher, and the membership of their home school.
    ///
    /// Both in one transaction, because a teacher with no membership would be in a state
    /// the rest of the model treats as impossible: `teacher_school` is the authority on
    /// which schools employ somebody, so a teacher without the row would be employed by
    /// nobody and appear in no staff list -- including the one of the school that had just
    /// created them.
    ///
    /// The teacher row is written first out of necessity: the membership references it.
    /// That ordering is why `teacher_references_a_member_school_insert` still accepts the
    /// home school directly, and the migration says so.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken within the school.
    pub fn create_teacher(&mut self, input: &TeacherInput) -> Result<Teacher> {
        let stamp = Stamp::new();
        let membership = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO teacher (id, school_id, code, given_name, family_name, email,
                     home_campus_id, fte_percent, max_periods_per_day, max_periods_per_cycle,
                     max_consecutive_periods, notes, is_active, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    &input.code,
                    &input.given_name,
                    &input.family_name,
                    &input.email,
                    opt_text(input.home_campus_id),
                    input.fte_percent,
                    input.max_periods_per_day,
                    input.max_periods_per_cycle,
                    input.max_consecutive_periods,
                    &input.notes,
                    input.is_active,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "teacher"))?;

            // The membership inherits the project-level flag, which is what the V13
            // backfill did for every teacher that already existed: before memberships,
            // `is_active` WAS the per-school answer, and a teacher entered as inactive is
            // being recorded rather than employed. The two flags diverge from here --
            // `update_teacher` never touches this row again, and eligibility is their
            // conjunction.
            insert_membership(tx, &membership, stamp.id, input.school_id, input.is_active)
        })?;
        self.teacher(stamp.id)?
            .ok_or(DbError::NotFound { entity: "teacher" })
    }

    /// One teacher by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teacher(&self, id: Uuid) -> Result<Option<Teacher>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TEACHER_COLUMNS} FROM teacher WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Teacher::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every teacher one school employs, active and inactive, ordered by name.
    ///
    /// Reads `teacher_school`, **not** `teacher.school_id`. Since `V13` the home school is
    /// one membership among possibly several, and a listing built on the column would miss
    /// every shared teacher the school had been given -- which is the whole thing ADR 0021
    /// exists to make work. A teacher belonging to two schools appears in both lists, once
    /// each, as the same row with the same identifier.
    ///
    /// Inactive teachers and inactive memberships are both included. Filtering them is the
    /// interface's decision, and a repository that hid them would also hide them from a
    /// timetable that names one. [`Membership::school_teachers`] carries each school's own
    /// view of employment alongside the person, for a screen that needs to show it.
    ///
    /// [`Membership::school_teachers`]: super::membership::Membership::school_teachers
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teachers(&self, school_id: Uuid) -> Result<Vec<Teacher>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM teacher t
                 JOIN teacher_school ts ON ts.teacher_id = t.id
                 WHERE ts.school_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                TEACHER_COLUMNS
                    .split(", ")
                    .map(|column| format!("t.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt.query_map([text(school_id)], Teacher::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a teacher's details. Their school never changes, and neither does their id.
    ///
    /// Identity surviving an edit is the point: a teacher who changes name, campus or
    /// status is the same teacher, and every assignment that names them still resolves.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_teacher(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &TeacherInput,
    ) -> Result<Teacher> {
        self.db.write(|tx| {
            require_same_school(tx, "teacher", "teacher", id, input.school_id)?;
            update(
                tx,
                "teacher",
                "teacher",
                id,
                expected,
                TEACHER_ASSIGNMENTS,
                rusqlite::params![
                    &input.code,
                    &input.given_name,
                    &input.family_name,
                    &input.email,
                    opt_text(input.home_campus_id),
                    input.fte_percent,
                    input.max_periods_per_day,
                    input.max_periods_per_cycle,
                    input.max_consecutive_periods,
                    &input.notes,
                    input.is_active,
                ],
            )
        })?;
        self.teacher(id)?
            .ok_or(DbError::NotFound { entity: "teacher" })
    }

    /// Deletes a teacher nobody else employs, and the rows that were only about them.
    ///
    /// # Deleting a person is not the same as letting go of them
    ///
    /// Since `V13` a teacher may be employed by several schools, and this is the operation
    /// that destroys the person rather than a relationship. So it takes the authority a
    /// shared facility takes in ADR 0019, for the same reason: a hall created on
    /// Northgate's campus and shared with Southmoor's is a hall Southmoor is timetabling
    /// into, and Northgate deleting it would take Southmoor's room away without Southmoor
    /// being asked.
    ///
    /// - Only the **home** school may delete a teacher -- [`require_same_school`] reads
    ///   `teacher.school_id`, which is exactly that.
    /// - And only **while nobody else employs them**: another school's membership makes
    ///   this [`DbError::StillReferenced`], and the way forward is for that school to
    ///   remove its own membership first -- an operation that exists, and that the other
    ///   school can see.
    ///
    /// A school merely finished with a shared teacher wants
    /// `Membership::remove_teacher_from_school`; a school whose teacher has left wants
    /// `Membership::set_teacher_membership_active`. Neither of them can reach this.
    ///
    /// # What goes with them
    ///
    /// Their home membership and every campus eligibility, in this transaction and through
    /// [`super::delete`] so each is a truthful change-log entry rather than a silent
    /// orphan. Their availability goes for the reason it always did -- a rule saying "not
    /// on Friday afternoons" means nothing once the person it describes is gone, and
    /// `entity_id` is polymorphic so no foreign key would have caught it.
    ///
    /// Nothing else references a teacher yet -- activities arrive in Phase 3. When
    /// `activity_teacher` exists its RESTRICT edge makes this refuse for a teacher who is
    /// timetabled, which is the behaviour the schema is already shaped for.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if no such teacher exists.
    /// - [`DbError::WrongSchool`] if another school created them.
    /// - [`DbError::StillReferenced`] if another school still employs them.
    pub fn delete_teacher(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            // Both checks before anything is cleared, not after: these rows are the
            // teacher's, and clearing a shared teacher's availability and *then* refusing
            // the delete would be the worst of both outcomes. V12 made that mistake
            // impossible for the school check and the rule is the same for this one.
            require_same_school(tx, "teacher", "teacher", id, school_id)?;
            if other_employers(tx, id, school_id)? > 0 {
                return Err(DbError::StillReferenced { entity: "teacher" });
            }

            clear_for_entity(tx, AvailabilityEntity::Teacher, id)?;
            // V14's two. A qualification and a department association both have the person
            // on one end of them, so neither means anything once that person is gone --
            // the same judgement `delete_student_group` makes about its own memberships.
            // Through `super::delete`, so each removal is a truthful change-log entry
            // rather than a silent orphan; `Expected::Any` because these are cascades
            // inside somebody else's transaction and the user never saw their revisions.
            for qualification in qualifications_of(tx, id)? {
                delete(
                    tx,
                    "qualification",
                    "teacher_qualification",
                    qualification,
                    Expected::Any,
                )?;
            }
            for association in associations_of(tx, id)? {
                delete(
                    tx,
                    "department association",
                    "teacher_department",
                    association,
                    Expected::Any,
                )?;
            }
            for eligibility in eligibility_of_teacher(tx, id)? {
                delete(
                    tx,
                    "campus eligibility",
                    "teacher_campus",
                    eligibility,
                    Expected::Any,
                )?;
            }
            // `V15`'s. A class assignment has this person on one end of it, so it means
            // nothing once they are gone — the same judgement the two rows above and the
            // qualifications before them get. Note the asymmetry with
            // `remove_teacher_from_school`, which *refuses* while an assignment remains:
            // withdrawing one school's employment is an edit, and destroying the person is
            // not. ADR 0025 §5 draws that line and ADR 0029 §7 extends it here.
            for assignment in assignments_of_teacher(tx, id)? {
                delete(
                    tx,
                    "class teacher",
                    "class_teacher",
                    assignment,
                    Expected::Any,
                )?;
            }
            for membership in memberships_of_teacher(tx, id)? {
                delete(
                    tx,
                    "school membership",
                    "teacher_school",
                    membership,
                    Expected::Any,
                )?;
            }
            delete(tx, "teacher", "teacher", id, expected)
        })
    }
}

// --------------------------------------------------------------------------- student

pub(crate) const STUDENT_COLUMNS: &str = "id, school_id, code, given_name, family_name, email, \
     year_level_id, campus_id, is_active, created_at, updated_at, rev";

const STUDENT_ASSIGNMENTS: &str = "code = ?, given_name = ?, family_name = ?, email = ?, \
     year_level_id = ?, campus_id = ?, is_active = ?";

impl People<'_> {
    /// Creates a student, and their enrolment at their home school.
    ///
    /// Both in one transaction, for the reason [`People::create_teacher`] writes a
    /// membership: `student_school_enrolment` is the authority on where a student is
    /// placed, and a student with no spell would appear on no roll -- including the one of
    /// the school that had just entered them.
    ///
    /// The spell carries the placement and the code the caller gave, which is what makes
    /// the two representations agree from the first moment rather than after a later edit.
    /// `is_active` decides its status: a student entered as having left becomes a `LEFT`
    /// spell, because that flag has meant "has not left" since `V10`.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the admission number is taken within the school.
    pub fn create_student(&mut self, input: &StudentInput) -> Result<Student> {
        let stamp = Stamp::new();
        let enrolment = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO student (id, school_id, code, given_name, family_name, email,
                     year_level_id, campus_id, is_active, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    &input.code,
                    &input.given_name,
                    &input.family_name,
                    &input.email,
                    opt_text(input.year_level_id),
                    opt_text(input.campus_id),
                    input.is_active,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "student"))?;

            insert_enrolment(
                tx,
                &enrolment,
                &StudentEnrolmentInput {
                    student_id: stamp.id,
                    school_id: input.school_id,
                    admission_no: input.code.clone(),
                    year_level_id: input.year_level_id,
                    campus_id: input.campus_id,
                    status: if input.is_active {
                        EnrolmentStatus::Enrolled
                    } else {
                        EnrolmentStatus::Left
                    },
                    enrolled_on: None,
                    left_on: None,
                },
            )
        })?;
        self.student(stamp.id)?
            .ok_or(DbError::NotFound { entity: "student" })
    }

    /// One student by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student(&self, id: Uuid) -> Result<Option<Student>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {STUDENT_COLUMNS} FROM student WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Student::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every student one school has ever enrolled, current and past, ordered by name.
    ///
    /// Reads `student_school_enrolment`, **not** `student.school_id`, for the reason
    /// [`People::teachers`] reads membership: the column names only the school that
    /// entered them, and a roll built on it would miss every student shared with or
    /// transferred into this school.
    ///
    /// Past spells are included, which keeps this method meaning exactly what it meant
    /// before `V13` -- "enrolled and left" -- and is what stops a transferred-away student
    /// vanishing from the school that still holds their history.
    /// `Membership::school_students` is the current roll, with each student's placement
    /// alongside them.
    ///
    /// `DISTINCT` because a student who left and rejoined has more than one spell here and
    /// is still one student.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn students(&self, school_id: Uuid) -> Result<Vec<Student>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT DISTINCT {} FROM student s
                 JOIN student_school_enrolment e ON e.student_id = s.id
                 WHERE e.school_id = ?
                 ORDER BY s.family_name, s.given_name, s.id",
                STUDENT_COLUMNS
                    .split(", ")
                    .map(|column| format!("s.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt.query_map([text(school_id)], Student::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a student's details. Their school never changes, and neither does their id.
    ///
    /// # Two representations, one authority
    ///
    /// `V13` made `student_school_enrolment` the authority on where a student is placed,
    /// and left `student.code`, `student.year_level_id` and `student.campus_id` in place
    /// as the **home-school** values -- the same treatment `room.campus_id` got in ADR
    /// 0014. Two writable copies of one fact is exactly the drift the brief for this stage
    /// forbids, so they are not independent: changing them here rewrites the student's
    /// current spell at their home school, in this transaction, through [`super::update`]
    /// so the spell's `rev` and the change feed stay truthful.
    ///
    /// What is *not* mirrored is `is_active`. That is the project-level flag -- "in the
    /// project at all" -- and the per-school answer is the spell's status, exactly as
    /// `teacher.is_active` and `teacher_school.is_active` are kept apart. Recording that a
    /// student has left one school of a trust is
    /// `Membership::leave_school`, and it must not be reachable by editing a shared
    /// person's details.
    ///
    /// A student with no current spell at their home school -- one who has transferred
    /// away -- has nothing to mirror into, and this edits only the `student` row. ADR 0021
    /// section 6 records the whole policy.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`], or [`DbError::Invalid`] if the
    /// placement belongs to another school.
    pub fn update_student(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &StudentInput,
    ) -> Result<Student> {
        self.db.write(|tx| {
            require_same_school(tx, "student", "student", id, input.school_id)?;
            update(
                tx,
                "student",
                "student",
                id,
                expected,
                STUDENT_ASSIGNMENTS,
                rusqlite::params![
                    &input.code,
                    &input.given_name,
                    &input.family_name,
                    &input.email,
                    opt_text(input.year_level_id),
                    opt_text(input.campus_id),
                    input.is_active,
                ],
            )?;
            mirror_student_onto_enrolment(tx, id)
        })?;
        self.student(id)?
            .ok_or(DbError::NotFound { entity: "student" })
    }

    /// Deletes a student who belongs to no group and no other school.
    ///
    /// Two refusals, for two different reasons, and both are deliberate:
    ///
    /// - **Still in a group.** The difference between a record entered by mistake and a
    ///   student who has left: the first has no memberships and goes, the second has
    ///   history worth keeping and should be marked inactive instead.
    /// - **Still enrolled somewhere else.** Since `V13` a student may attend two schools,
    ///   and this operation destroys the person rather than one school's relationship with
    ///   them. Only the home school may reach it, and only while no other school holds a
    ///   spell -- the authority a shared facility takes in ADR 0019, applied to a shared
    ///   person. `Membership::leave_school` is what a school finished with a shared
    ///   student wants, and it cannot reach this.
    ///
    /// Their own spells go with them, through [`super::delete`] so each is a truthful
    /// change-log entry.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] in either case; [`DbError::WrongSchool`] if another
    /// school created them.
    pub fn delete_student(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "student", "student", id, school_id)?;
            if other_schools_attended(tx, id, school_id)? > 0 {
                return Err(DbError::StillReferenced { entity: "student" });
            }
            let groups: i64 = tx.query_row(
                "SELECT COUNT(*) FROM student_group_member WHERE student_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;

            if groups > 0 {
                // Typed rather than a worded `Invalid`, so the interface's existing
                // handling for a blocked delete applies. The count the confirmation
                // dialogue shows comes from `groups_for_student` before it asks, not from
                // this message — a refusal arriving after the user pressed Delete is too
                // late to be the first time they hear about it.
                return Err(DbError::StillReferenced { entity: "student" });
            }

            // `V15`'s, and refused for exactly the reason group membership is: "she was in
            // 9MAT-A" is history, and ADR 0018's rule is that where the dependent row *is*
            // the history, the delete is refused and the remedy is made visible rather than
            // performed. `Lessons::class_count_of_student` is what the confirmation reads
            // first, so this message is the backstop and not the explanation.
            if rosters_of_student(tx, id)? > 0 {
                return Err(DbError::StillReferenced { entity: "student" });
            }

            for enrolment in enrolments_of_student(tx, id)? {
                delete(
                    tx,
                    "enrolment",
                    "student_school_enrolment",
                    enrolment,
                    Expected::Any,
                )?;
            }
            delete(tx, "student", "student", id, expected)
        })
    }
}

// --------------------------------------------------------------------- student group

pub(crate) const STUDENT_GROUP_COLUMNS: &str = "id, school_id, name, code, kind, year_level_id, \
     parent_group_id, size, campus_id, created_at, updated_at, rev";

const STUDENT_GROUP_ASSIGNMENTS: &str = "name = ?, code = ?, kind = ?, year_level_id = ?, \
     parent_group_id = ?, size = ?, campus_id = ?";

impl People<'_> {
    /// Creates a student group.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name or code is taken; [`DbError::Invalid`]
    /// if the parent does not exist or would form a cycle.
    pub fn create_student_group(&mut self, input: &StudentGroupInput) -> Result<StudentGroup> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            // A brand-new group cannot be its own ancestor, but its parent chain can still
            // be checked for an existing cycle — cheap, and it fails here rather than in
            // the first thing that tries to resolve the hierarchy.
            if let Some(parent) = input.parent_group_id {
                ensure_group_exists(tx, parent)?;
                ensure_no_cycle(tx, stamp.id, parent)?;
            }

            tx.execute(
                "INSERT INTO student_group (id, school_id, name, code, kind, year_level_id,
                     parent_group_id, size, campus_id, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    &input.code,
                    input.kind.as_str(),
                    opt_text(input.year_level_id),
                    opt_text(input.parent_group_id),
                    input.size,
                    opt_text(input.campus_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "student group"))?;
            Ok(())
        })?;
        self.student_group(stamp.id)?.ok_or(DbError::NotFound {
            entity: "student group",
        })
    }

    /// One student group by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student_group(&self, id: Uuid) -> Result<Option<StudentGroup>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {STUDENT_GROUP_COLUMNS} FROM student_group WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], StudentGroup::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every student group, ordered by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student_groups(&self, school_id: Uuid) -> Result<Vec<StudentGroup>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {STUDENT_GROUP_COLUMNS} FROM student_group WHERE school_id = ? \
                 ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], StudentGroup::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a group's details. Its school never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`], or [`DbError::Invalid`] if
    /// the parent does not exist or the change would make the hierarchy cyclic.
    pub fn update_student_group(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &StudentGroupInput,
    ) -> Result<StudentGroup> {
        self.db.write(|tx| {
            require_same_school(tx, "student group", "student_group", id, input.school_id)?;
            if let Some(parent) = input.parent_group_id {
                ensure_group_exists(tx, parent)?;
                ensure_no_cycle(tx, id, parent)?;
            }

            update(
                tx,
                "student group",
                "student_group",
                id,
                expected,
                STUDENT_GROUP_ASSIGNMENTS,
                rusqlite::params![
                    &input.name,
                    &input.code,
                    input.kind.as_str(),
                    opt_text(input.year_level_id),
                    opt_text(input.parent_group_id),
                    input.size,
                    opt_text(input.campus_id),
                ],
            )
        })?;
        self.student_group(id)?.ok_or(DbError::NotFound {
            entity: "student group",
        })
    }

    /// How many students are in one group.
    ///
    /// Exists so the interface can say what deleting it will remove, before it happens.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student_group_member_count(&self, student_group_id: Uuid) -> Result<i64> {
        self.db.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM student_group_member WHERE student_group_id = ?",
                [text(student_group_id)],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// Deletes a group and the memberships that only existed because of it.
    ///
    /// The students themselves are untouched. Child groups are not: a group that is still
    /// a parent is refused, because flattening a hierarchy is not a side effect anyone
    /// asked for.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], or [`DbError::StillReferenced`] if it still has child groups.
    pub fn delete_student_group(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "student group", "student_group", id, school_id)?;
            let children: i64 = tx.query_row(
                "SELECT COUNT(*) FROM student_group WHERE parent_group_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;

            if children > 0 {
                return Err(DbError::StillReferenced {
                    entity: "student group",
                });
            }

            // `V15`'s. A group attending three class sections is the audience of three
            // lessons, and deleting it would take the audience out from under them — so
            // this refuses, unlike the group's own memberships below, which it clears.
            // `class_group.student_group_id` is `RESTRICT` for the reason every edge to a
            // substantial record is, and `Lessons::class_count_of_group` reports the count
            // so the confirmation explains before the button rather than after it.
            if sections_of_group(tx, id)? > 0 {
                return Err(DbError::StillReferenced {
                    entity: "student group",
                });
            }

            // The group's own availability goes with it, for the same reason a teacher's
            // does: it is a statement about this group and means nothing without it.
            clear_for_entity(tx, AvailabilityEntity::StudentGroup, id)?;

            // Through repo::delete per row, so each removal is its own change-log entry.
            // A CASCADE would not fire the trigger at all.
            let members = {
                let mut stmt = tx.prepare(
                    "SELECT id FROM student_group_member WHERE student_group_id = ? ORDER BY id",
                )?;
                let rows = stmt.query_map([text(id)], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for raw in members {
                let member_id = super::row::parse_id_text("student_group_member.id", &raw)?;
                delete(
                    tx,
                    "group membership",
                    "student_group_member",
                    member_id,
                    Expected::Any,
                )?;
            }

            delete(tx, "student group", "student_group", id, expected)
        })
    }
}

/// Fails unless the group exists.
///
/// A missing parent would otherwise surface as a foreign-key failure, which says nothing
/// about which end was wrong.
fn ensure_group_exists(tx: &Transaction<'_>, id: Uuid) -> Result<()> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM student_group WHERE id = ?",
            [text(id)],
            |row| row.get(0),
        )
        .optional()?;

    if found.is_none() {
        return Err(DbError::NotFound {
            entity: "student group",
        });
    }
    Ok(())
}

/// Refuses a parent that would make the hierarchy cyclic.
///
/// Walks from the proposed parent upwards. If `group` appears, the edge would close a
/// loop. The step limit is a backstop against a cycle that already exists in a damaged
/// file: without it this walk is what hangs, and a hang is much worse to diagnose than an
/// error.
fn ensure_no_cycle(tx: &Transaction<'_>, group: Uuid, parent: Uuid) -> Result<()> {
    let mut current = Some(parent);
    let mut steps = 0usize;

    while let Some(id) = current {
        if id == group {
            return Err(DbError::Invalid {
                message: "a group cannot be inside itself, directly or through its parents"
                    .to_owned(),
            });
        }

        steps += 1;
        if steps > MAX_GROUP_DEPTH {
            return Err(DbError::Invalid {
                message: "the group hierarchy is too deep, or already contains a loop".to_owned(),
            });
        }

        let raw: Option<Option<String>> = tx
            .query_row(
                "SELECT parent_group_id FROM student_group WHERE id = ?",
                [text(id)],
                |row| row.get(0),
            )
            .optional()?;

        current = match raw.flatten() {
            Some(value) => Some(super::row::parse_id_text("parent_group_id", &value)?),
            None => None,
        };
    }
    Ok(())
}

/// How far the parent walk will go before calling the hierarchy broken.
///
/// Schools nest a few levels — school, cohort, band, set. Anything beyond this is a
/// mistake or a loop, and either way the user needs to be told rather than kept waiting.
const MAX_GROUP_DEPTH: usize = 64;

// -------------------------------------------------------------------- group membership

entity! {
    /// One student's place in one group.
    ///
    /// A first-class row with its own identity, not a bare junction: that is what keeps it
    /// inside the change feed. See the migration for the full reason.
    StudentGroupMember,
    /// Values for adding a student to a group.
    StudentGroupMemberInput,
    {
        /// The group.
        student_group_id: Uuid,
        /// The student.
        student_id: Uuid,
    }
}

const MEMBER_COLUMNS: &str = "id, student_group_id, student_id, created_at, updated_at, rev";

impl People<'_> {
    /// Adds a student to a group of a school they have been enrolled in.
    ///
    /// The enrolment requirement is new in `V13` and closes a hole `V12` could not: a
    /// membership carries no school of its own, so nothing stopped Northgate's student
    /// being put into Southmoor's cohort -- and a cohort containing another institution's
    /// students is not an audience anything can be timetabled for.
    ///
    /// Stated against enrolment rather than against `student.school_id`, which is what
    /// makes it correct under sharing: a student enrolled in both schools may legitimately
    /// be in a group in either. Any spell counts, past ones included, for the reason ADR
    /// 0018 gives throughout -- membership of 9-MAT-A last year is the only evidence it
    /// happened, and a student who has left must not have their history refused.
    ///
    /// ADR 0006 is untouched: a school that timetables with `student_group.size` and no
    /// student records at all never reaches this method.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if they are already in it, [`DbError::Invalid`] if they
    /// have never been enrolled in the group's school, or [`DbError::NotFound`] shape
    /// errors if either end does not exist.
    pub fn add_student_group_member(
        &mut self,
        input: &StudentGroupMemberInput,
    ) -> Result<StudentGroupMember> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            // Checked here as well as by the trigger, because the trigger can say the rule
            // but not the remedy, and "enrol them in this school first" is the remedy.
            let enrolled: i64 = tx.query_row(
                "SELECT COUNT(*) FROM student_group g
                 JOIN student_school_enrolment e ON e.school_id = g.school_id
                 WHERE g.id = ? AND e.student_id = ?",
                [text(input.student_group_id), text(input.student_id)],
                |row| row.get(0),
            )?;
            if enrolled == 0 {
                return Err(DbError::Invalid {
                    message: "This student is not enrolled in the group's school. Enrol them \
                              there first."
                        .to_owned(),
                });
            }

            tx.execute(
                "INSERT INTO student_group_member
                     (id, student_group_id, student_id, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.student_group_id),
                    text(input.student_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "group membership"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {MEMBER_COLUMNS} FROM student_group_member WHERE id = ?"
                ))?;
                let mut rows = stmt.query_map([text(stamp.id)], StudentGroupMember::from_row)?;
                rows.next().transpose().map_err(DbError::from)
            })?
            .ok_or(DbError::NotFound {
                entity: "group membership",
            })
    }

    /// Removes one student from one group.
    ///
    /// Addressed by the pair rather than by the membership's own identifier, because that
    /// is what the interface has in hand: a row in a group's member list.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if that student is not in that group.
    pub fn remove_student_group_member(
        &mut self,
        school_id: Uuid,
        student_group_id: Uuid,
        student_id: Uuid,
    ) -> Result<()> {
        self.db.write(|tx| {
            // A membership has no `school_id` of its own and is not given one: the group
            // owns it, and the group names its school. Checking the group is therefore
            // checking the membership, with no second copy of the same fact to go stale.
            require_same_school(
                tx,
                "student group",
                "student_group",
                student_group_id,
                school_id,
            )?;
            let raw: Option<String> = tx
                .query_row(
                    "SELECT id FROM student_group_member
                     WHERE student_group_id = ? AND student_id = ?",
                    [text(student_group_id), text(student_id)],
                    |row| row.get(0),
                )
                .optional()?;

            let raw = raw.ok_or(DbError::NotFound {
                entity: "group membership",
            })?;
            let id = super::row::parse_id_text("student_group_member.id", &raw)?;
            delete(
                tx,
                "group membership",
                "student_group_member",
                id,
                Expected::Any,
            )
        })
    }

    /// Every student in one group, ordered by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student_group_members(&self, student_group_id: Uuid) -> Result<Vec<Student>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM student s
                 JOIN student_group_member m ON m.student_id = s.id
                 WHERE m.student_group_id = ?
                 ORDER BY s.family_name, s.given_name, s.id",
                STUDENT_COLUMNS
                    .split(", ")
                    .map(|column| format!("s.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt.query_map([text(student_group_id)], Student::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Every group one student belongs to, ordered by name.
    ///
    /// The plural is the point: a student is in a homeroom *and* a teaching set *and* an
    /// elective, and anything that assumed one group would be wrong the moment electives
    /// exist.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn groups_for_student(&self, student_id: Uuid) -> Result<Vec<StudentGroup>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM student_group g
                 JOIN student_group_member m ON m.student_group_id = g.id
                 WHERE m.student_id = ?
                 ORDER BY g.name, g.id",
                STUDENT_GROUP_COLUMNS
                    .split(", ")
                    .map(|column| format!("g.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt.query_map([text(student_id)], StudentGroup::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }
}
