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
use super::row::{entity, opt_text, text};
use super::{Stamp, delete, require_same_school, translate, update};
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
    pub fn update_year_level(&mut self, id: Uuid, input: &YearLevelInput) -> Result<YearLevel> {
        self.db.write(|tx| {
            require_same_school(tx, "year level", "year_level", id, input.school_id)?;
            update(
                tx,
                "year level",
                "year_level",
                id,
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
    pub fn delete_year_level(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            detach_year_level(tx, "student", id)?;
            detach_year_level(tx, "student_group", id)?;
            delete(tx, "year level", "year_level", id)
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
            "year_level_id = ?",
            &[&None::<String>],
        )?;
    }
    Ok(())
}

// --------------------------------------------------------------------------- teacher

const TEACHER_COLUMNS: &str = "id, school_id, code, given_name, family_name, email, \
     home_campus_id, fte_percent, max_periods_per_day, max_periods_per_cycle, \
     max_consecutive_periods, notes, is_active, created_at, updated_at, rev";

/// The assignments and bindings shared by teacher insert and update.
const TEACHER_ASSIGNMENTS: &str = "code = ?, given_name = ?, family_name = ?, email = ?, \
     home_campus_id = ?, fte_percent = ?, max_periods_per_day = ?, max_periods_per_cycle = ?, \
     max_consecutive_periods = ?, notes = ?, is_active = ?";

impl People<'_> {
    /// Creates a teacher.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken within the school.
    pub fn create_teacher(&mut self, input: &TeacherInput) -> Result<Teacher> {
        let stamp = Stamp::new();
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
            Ok(())
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

    /// Every teacher, active and inactive, ordered by name.
    ///
    /// Inactive teachers are included. Filtering them is the interface's decision, and a
    /// repository that hid them would also hide them from a timetable that names one.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teachers(&self, school_id: Uuid) -> Result<Vec<Teacher>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TEACHER_COLUMNS} FROM teacher WHERE school_id = ? \
                 ORDER BY family_name, given_name, id"
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
    pub fn update_teacher(&mut self, id: Uuid, input: &TeacherInput) -> Result<Teacher> {
        self.db.write(|tx| {
            require_same_school(tx, "teacher", "teacher", id, input.school_id)?;
            update(
                tx,
                "teacher",
                "teacher",
                id,
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

    /// Deletes a teacher, and the availability rules that were about them.
    ///
    /// Their availability goes because it is *theirs* — a rule saying "not on Friday
    /// afternoons" means nothing once the person it describes is gone, and `entity_id` is
    /// polymorphic so no foreign key would have caught it. The rows are removed through
    /// `repo::delete` so each is a truthful change-log entry rather than a silent orphan.
    ///
    /// Nothing else references a teacher yet — activities arrive in Phase 3 — so this
    /// otherwise succeeds today. When `activity_teacher` exists its RESTRICT edge makes
    /// this refuse for a teacher who is timetabled, which is the behaviour the schema is
    /// already shaped for.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if no such teacher exists.
    pub fn delete_teacher(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            clear_for_entity(tx, AvailabilityEntity::Teacher, id)?;
            delete(tx, "teacher", "teacher", id)
        })
    }
}

// --------------------------------------------------------------------------- student

const STUDENT_COLUMNS: &str = "id, school_id, code, given_name, family_name, email, \
     year_level_id, campus_id, is_active, created_at, updated_at, rev";

const STUDENT_ASSIGNMENTS: &str = "code = ?, given_name = ?, family_name = ?, email = ?, \
     year_level_id = ?, campus_id = ?, is_active = ?";

impl People<'_> {
    /// Creates a student.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the admission number is taken within the school.
    pub fn create_student(&mut self, input: &StudentInput) -> Result<Student> {
        let stamp = Stamp::new();
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
            Ok(())
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

    /// Every student, enrolled and left, ordered by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn students(&self, school_id: Uuid) -> Result<Vec<Student>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {STUDENT_COLUMNS} FROM student WHERE school_id = ? \
                 ORDER BY family_name, given_name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], Student::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a student's details. Their school never changes, and neither does their id.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_student(&mut self, id: Uuid, input: &StudentInput) -> Result<Student> {
        self.db.write(|tx| {
            require_same_school(tx, "student", "student", id, input.school_id)?;
            update(
                tx,
                "student",
                "student",
                id,
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
            )
        })?;
        self.student(id)?
            .ok_or(DbError::NotFound { entity: "student" })
    }

    /// Deletes a student who belongs to no group.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if they are still in one. That is deliberate and is
    /// the difference between a record entered by mistake and a student who has left:
    /// the first has no memberships and goes, the second has history worth keeping and
    /// should be marked inactive instead. The message says which.
    pub fn delete_student(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
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

            delete(tx, "student", "student", id)
        })
    }
}

// --------------------------------------------------------------------- student group

const STUDENT_GROUP_COLUMNS: &str = "id, school_id, name, code, kind, year_level_id, \
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
    pub fn delete_student_group(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
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
                delete(tx, "group membership", "student_group_member", member_id)?;
            }

            delete(tx, "student group", "student_group", id)
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
    /// Adds a student to a group.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if they are already in it, or [`DbError::NotFound`]
    /// shape errors if either end does not exist.
    pub fn add_student_group_member(
        &mut self,
        input: &StudentGroupMemberInput,
    ) -> Result<StudentGroupMember> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
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
        student_group_id: Uuid,
        student_id: Uuid,
    ) -> Result<()> {
        self.db.write(|tx| {
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
            delete(tx, "group membership", "student_group_member", id)
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
