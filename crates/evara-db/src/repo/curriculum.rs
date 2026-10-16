//! Repositories for the Curriculum prerequisites: departments, subjects, qualifications
//! and department membership.
//!
//! Same shape as [`super::structure`] and [`super::people`] — callers pass an input struct
//! and receive a record, no `rusqlite` type appears in a signature, and every write goes
//! through [`super::update`] or [`super::delete`] so `rev` and `updated_at` cannot be
//! forgotten.
//!
//! # Two scopes, kept apart on purpose
//!
//! A department and a subject are **school-owned**: every read takes an explicit
//! `school_id` and every write refuses an identifier belonging to another school, exactly
//! as `V12` requires. A teacher is **project-level** and may be employed by several
//! schools. The two meet here, and the whole of
//! [adr/0025](../../../docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md)
//! is about keeping the meeting honest:
//!
//! - A **qualification** is a fact about a person. It names one school-owned subject, so
//!   the subject supplies the school; the qualification itself carries no `school_id` and
//!   requires no membership, and nothing short of deleting the person removes it.
//! - A **department association** is a fact about one school's internal organisation. It
//!   is a narrowing of school membership, modelled on `teacher_campus` from `V13` down to
//!   the trigger names, and it goes when that membership is withdrawn.
//!
//! The practical consequence is worth stating plainly, because it looks like duplication
//! and is not: a teacher who teaches Mathematics at two schools holds **two**
//! qualifications, one per subject record, and remains **one** teacher. The subjects are
//! two different curricula owned by two different institutions.
//!
//! # Deletion
//!
//! Both deletes are **refusals with counts**, not cascades:
//!
//! - [`Curriculum::delete_department`] is refused while any subject is classified by it or
//!   any teacher is associated with it. [`Curriculum::department_dependents`] exists so the
//!   confirmation says which, before the user presses anything.
//! - [`Curriculum::delete_subject`] is refused while anybody is qualified for it.
//!   [`Curriculum::subject_dependents`] says how many.
//!
//! ADR 0025 §6 records why a department is not treated as ADR 0010's "optional
//! classification, therefore `SET NULL`" case. The short version is R10 in
//! `OPEN-DECISIONS`: a database-level `SET NULL` rewrites rows without incrementing their
//! revision, which is precisely the contract Stage B exists to enforce.

use rusqlite::{OptionalExtension, Transaction, types::ToSql};
use uuid::Uuid;

use super::people::{TEACHER_COLUMNS, Teacher};
use super::row::{entity, opt_text, parse_id_text, text};
use super::{Expected, Stamp, delete, require_same_school, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing Curriculum entities.
///
/// Obtained from [`Database::curriculum`]. Holds the database mutably because most
/// operations write.
#[derive(Debug)]
pub struct Curriculum<'a> {
    db: &'a mut Database,
}

impl<'a> Curriculum<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

entity! {
    /// An organisational unit inside one school: "Mathematics", "Humanities".
    ///
    /// Never shared. Two schools in one campus file may each hold a Mathematics
    /// department, as two records with two identifiers — the same treatment a year level
    /// gets, and for the same reason: this says how one institution organises itself.
    Department,
    /// Values for creating or replacing a department.
    DepartmentInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name, unique within the school.
        name: String,
        /// Optional short reference. Unique within the school when given.
        code: Option<String>,
        /// Free text for whatever the school needs to remember.
        description: Option<String>,
    }
}

entity! {
    /// Something a school teaches: "Mathematics", "English Literature", "Chemistry".
    ///
    /// School-owned. Two schools' Mathematics are two subjects with two identifiers, and
    /// they may share a name and a code without colliding. Nothing merges them on a
    /// matching name, because they may have different departments and, from Stage C,
    /// different courses.
    Subject,
    /// Values for creating or replacing a subject.
    SubjectInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Which of this school's departments teaches it, if the school says.
        ///
        /// Optional: a school that has not organised itself into departments still has
        /// subjects, and being made to invent a department before entering Mathematics
        /// would be asking the wrong question first.
        department_id: Option<Uuid>,
        /// Display name, unique within the school.
        name: String,
        /// Optional short reference. Unique within the school when given.
        code: Option<String>,
        /// Free text.
        description: Option<String>,
    }
}

entity! {
    /// That one teacher is qualified to teach one school-owned subject.
    ///
    /// A property of the **person**, which is why there is no `school_id` here: the
    /// subject names the school. It outlives employment — a teacher who leaves does not
    /// become unqualified — and only deleting the person removes it.
    TeacherQualification,
    /// Values for recording a qualification.
    TeacherQualificationInput,
    {
        /// The person. Project-level, one row per human being.
        teacher_id: Uuid,
        /// The subject they can teach. Its school owns the curriculum context.
        subject_id: Uuid,
    }
}

entity! {
    /// That one teacher belongs to one school's department.
    ///
    /// A **narrowing of school membership**, like `teacher_campus`: a trigger pair refuses
    /// a department of a school the teacher does not belong to, and withdrawing the
    /// membership withdraws this with it.
    TeacherDepartment,
    /// Values for recording a department association.
    TeacherDepartmentInput,
    {
        /// The person.
        teacher_id: Uuid,
        /// The department. Its school is reached through `department.school_id`.
        department_id: Uuid,
    }
}

/// What a department still holds, counted per kind.
///
/// Read *before* a delete is offered, so the confirmation explains rather than the button
/// fails — the pattern `SchoolDependents` established in Stage 0A and ADR 0023 §4 made
/// ordinary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DepartmentDependents {
    /// Sum of the other fields. Zero means the department can be deleted.
    pub total: i64,
    /// Subjects classified by this department.
    pub subjects: i64,
    /// Teachers associated with this department.
    pub teachers: i64,
}

/// What a subject still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SubjectDependents {
    /// Sum of the other fields. Zero means the subject can be deleted.
    pub total: i64,
    /// Teachers qualified to teach it.
    pub qualified_teachers: i64,
    /// Courses that are offerings of it.
    ///
    /// Added in Stage C. `ROADMAP.md` said in advance that this is where the new count
    /// goes — *"`course.subject_id` is `RESTRICT`, like every ownership edge, and
    /// `Curriculum::subject_dependents` is the place to add the new count so a refused
    /// subject delete goes on explaining itself."*
    pub courses: i64,
}

/// One of a teacher's qualifications, with the subject it names.
///
/// Returned instead of a bare [`TeacherQualification`] because the identifier alone tells a
/// screen nothing it can show: the whole point of this listing is "what can she teach, and
/// where?", and the answer lives on the subject. `departmentName` is resolved here rather
/// than by a call per row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QualifiedSubject {
    /// The qualification record. Its `rev` is the precondition for removing it.
    pub qualification: TeacherQualification,
    /// The subject, which carries the owning `schoolId`.
    pub subject: Subject,
    /// Name of the subject's department, when it has one.
    pub department_name: Option<String>,
}

/// One teacher qualified to teach a subject.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QualifiedTeacher {
    /// The shared identity. One record for one real person, however many schools.
    pub teacher: Teacher,
    /// The qualification joining them to the subject.
    pub qualification: TeacherQualification,
}

/// One of a teacher's department associations, with the department it names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TeacherDepartmentView {
    /// The association record. Its `rev` is the precondition for removing it.
    pub association: TeacherDepartment,
    /// The department, which carries the owning `schoolId`.
    pub department: Department,
}

const DEPARTMENT_COLUMNS: &str =
    "id, school_id, name, code, description, created_at, updated_at, rev";
const DEPARTMENT_ASSIGNMENTS: &str = "name = ?, code = ?, description = ?";

pub(crate) const SUBJECT_COLUMNS: &str =
    "id, school_id, department_id, name, code, description, created_at, updated_at, rev";
const SUBJECT_ASSIGNMENTS: &str = "department_id = ?, name = ?, code = ?, description = ?";

const QUALIFICATION_COLUMNS: &str = "id, teacher_id, subject_id, created_at, updated_at, rev";
const TEACHER_DEPARTMENT_COLUMNS: &str =
    "id, teacher_id, department_id, created_at, updated_at, rev";

// ------------------------------------------------------------------------- department

impl Curriculum<'_> {
    /// Creates a department in one school.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if the name, or the code when given, is already used
    ///   within that school.
    /// - [`DbError::StillReferenced`] if no such school exists — the foreign key's own
    ///   report, left as it is everywhere else in this crate.
    pub fn create_department(&mut self, input: &DepartmentInput) -> Result<Department> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            insert_department(tx, &stamp, input)?;
            Ok(())
        })?;
        self.department(stamp.id)?.ok_or(DbError::NotFound {
            entity: "department",
        })
    }

    /// One department by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn department(&self, id: Uuid) -> Result<Option<Department>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {DEPARTMENT_COLUMNS} FROM department WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Department::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every department of one school, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn departments(&self, school_id: Uuid) -> Result<Vec<Department>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {DEPARTMENT_COLUMNS} FROM department WHERE school_id = ?
                 ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], Department::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a department's details. Its school never changes, and neither does its id.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if it belongs to another school.
    /// - [`DbError::Conflict`] if somebody else saved first.
    /// - [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_department(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &DepartmentInput,
    ) -> Result<Department> {
        self.db.write(|tx| {
            require_same_school(tx, "department", "department", id, input.school_id)?;
            update_department_row(tx, id, expected, input)?;
            Ok(())
        })?;
        self.department(id)?.ok_or(DbError::NotFound {
            entity: "department",
        })
    }

    /// What still refers to a department, so a confirmation can explain before it asks.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn department_dependents(&self, id: Uuid) -> Result<DepartmentDependents> {
        self.db.read(|conn| {
            let key = text(id);
            let count = |sql: &str| -> Result<i64> {
                conn.query_row(sql, [&key], |row| row.get(0))
                    .map_err(DbError::from)
            };
            let subjects = count("SELECT COUNT(*) FROM subject WHERE department_id = ?")?;
            let teachers =
                count("SELECT COUNT(*) FROM teacher_department WHERE department_id = ?")?;
            Ok(DepartmentDependents {
                total: subjects + teachers,
                subjects,
                teachers,
            })
        })
    }

    /// Deletes a department nothing refers to.
    ///
    /// # Why this refuses rather than unclassifying
    ///
    /// ADR 0010 says optional classification references are `SET NULL`, and a department is
    /// shaped like one — a subject exists perfectly well without one. It is nevertheless a
    /// refusal, for two reasons recorded in ADR 0025 §6. A database-level `SET NULL` would
    /// rewrite every affected subject without incrementing a single revision, which is R10
    /// in `OPEN-DECISIONS` and flatly contradicts the contract the rest of this stage
    /// installs. And a school deleting a department that still has subjects and staff in it
    /// has almost certainly mistaken which row it is on; the useful answer names what is in
    /// the way.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if another school owns it.
    /// - [`DbError::StillReferenced`] while any subject or teacher refers to it. Call
    ///   [`Curriculum::department_dependents`] first to say which.
    /// - [`DbError::Conflict`] if somebody else edited it since it was read.
    /// - [`DbError::NotFound`] if no such department exists.
    pub fn delete_department(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            // Both checks before anything else, so a refusal leaves the project untouched
            // rather than part-way through. V12 established the ordering for the school
            // check and the rule is the same for this one.
            require_same_school(tx, "department", "department", id, school_id)?;
            if department_is_referenced(tx, id)? {
                return Err(DbError::StillReferenced {
                    entity: "department",
                });
            }
            delete(tx, "department", "department", id, expected)
        })
    }
}

/// Writes one department row.
///
/// Extracted from [`Curriculum::create_department`] so an import can write several kinds of record
/// inside one transaction — see `super::import`. The public method and the importer go
/// through this one statement, so a new column cannot reach one path and miss the other.
pub(crate) fn insert_department(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    input: &DepartmentInput,
) -> Result<()> {
    tx.execute(
        "INSERT INTO department (id, school_id, name, code, description,
             created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(input.school_id),
            &input.name,
            &input.code,
            &input.description,
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| translate(error, "department"))?;
    Ok(())
}

/// Writes one department's domain columns, at the revision the caller expects.
///
/// Extracted from [`Curriculum::update_department`] so an import can compose several writes inside one
/// transaction — see `super::import`. Both paths go through this one statement, so a new
/// column cannot reach one and miss the other. The school check and anything the public
/// method does *around* the write stay there, because the importer has already done its own.
pub(crate) fn update_department_row(
    tx: &Transaction<'_>,
    id: Uuid,
    expected: Expected,
    input: &DepartmentInput,
) -> Result<()> {
    update(
        tx,
        "department",
        "department",
        id,
        expected,
        DEPARTMENT_ASSIGNMENTS,
        rusqlite::params![&input.name, &input.code, &input.description],
    )?;
    Ok(())
}

/// Whether anything still refers to a department.
///
/// Inside the transaction that is about to delete it, so the answer cannot go stale between
/// the question and the write — which is the whole difference between this and the public
/// [`Curriculum::department_dependents`], whose job is to inform a dialogue.
fn department_is_referenced(tx: &Transaction<'_>, id: Uuid) -> Result<bool> {
    let key = text(id);
    let subjects: i64 = tx.query_row(
        "SELECT COUNT(*) FROM subject WHERE department_id = ?",
        [&key],
        |row| row.get(0),
    )?;
    let teachers: i64 = tx.query_row(
        "SELECT COUNT(*) FROM teacher_department WHERE department_id = ?",
        [&key],
        |row| row.get(0),
    )?;
    Ok(subjects + teachers > 0)
}

// ---------------------------------------------------------------------------- subject

impl Curriculum<'_> {
    /// Creates a subject in one school, optionally in one of its departments.
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] if the department belongs to another school. Checked here as
    ///   well as by the schema's trigger pair, because a trigger cannot name the remedy and
    ///   a stale department picker is exactly how this mistake happens.
    /// - [`DbError::DuplicateValue`] if the name, or the code when given, is taken within
    ///   the school.
    pub fn create_subject(&mut self, input: &SubjectInput) -> Result<Subject> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_department_in_school(tx, input.department_id, input.school_id)?;
            insert_subject(tx, &stamp, input)?;
            Ok(())
        })?;
        self.subject(stamp.id)?
            .ok_or(DbError::NotFound { entity: "subject" })
    }

    /// One subject by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn subject(&self, id: Uuid) -> Result<Option<Subject>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SUBJECT_COLUMNS} FROM subject WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Subject::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every subject of one school, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn subjects(&self, school_id: Uuid) -> Result<Vec<Subject>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SUBJECT_COLUMNS} FROM subject WHERE school_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], Subject::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a subject's details, including which department teaches it.
    ///
    /// The school is not among them: a subject is *of* a school, and repointing one would
    /// move a curriculum between institutions rather than edit it.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if it belongs to another school.
    /// - [`DbError::Invalid`] if the new department belongs to another school.
    /// - [`DbError::Conflict`] if somebody else saved first.
    /// - [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_subject(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &SubjectInput,
    ) -> Result<Subject> {
        self.db.write(|tx| {
            require_same_school(tx, "subject", "subject", id, input.school_id)?;
            require_department_in_school(tx, input.department_id, input.school_id)?;
            update_subject_row(tx, id, expected, input)?;
            Ok(())
        })?;
        self.subject(id)?
            .ok_or(DbError::NotFound { entity: "subject" })
    }

    /// What still refers to a subject.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn subject_dependents(&self, id: Uuid) -> Result<SubjectDependents> {
        self.db.read(|conn| {
            let qualified: i64 = conn.query_row(
                "SELECT COUNT(*) FROM teacher_qualification WHERE subject_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;
            let courses = super::lessons::courses_of_subject(conn, id)?;
            Ok(SubjectDependents {
                total: qualified + courses,
                qualified_teachers: qualified,
                courses,
            })
        })
    }

    /// Every teacher qualified to teach one of this school's subjects, by name.
    ///
    /// Project-wide in the people it returns and school-scoped in the subject it asks
    /// about, which is the asymmetry ADR 0025 is about: a visiting teacher qualified for
    /// this school's Chemistry appears here, because she is.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the subject belongs to another school.
    pub fn subject_teachers(
        &self,
        school_id: Uuid,
        subject_id: Uuid,
    ) -> Result<Vec<QualifiedTeacher>> {
        self.db.read(|conn| {
            // The refusal belongs in the read, not in a write transaction opened to ask a
            // question — `require_same_school` takes a connection for exactly this.
            require_same_school(conn, "subject", "subject", subject_id, school_id)?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}
                 FROM teacher_qualification q
                 JOIN teacher t ON t.id = q.teacher_id
                 WHERE q.subject_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                qualify(TEACHER_COLUMNS, "t"),
                qualify(QUALIFICATION_COLUMNS, "q"),
            ))?;
            let offset = TEACHER_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(subject_id)], |row| {
                Ok(QualifiedTeacher {
                    teacher: Teacher::from_row_at(row, 0)?,
                    qualification: TeacherQualification::from_row_at(row, offset)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Deletes a subject nobody is qualified for.
    ///
    /// # Why a qualification blocks this
    ///
    /// Because it is a record about a person, and a school clearing out a subject it no
    /// longer offers must not take with it the fact that four of its staff were qualified
    /// to teach it. That is the same judgement `delete_student` makes about group
    /// membership in ADR 0018: where the dependent row *is* the history, the delete is
    /// refused and the remedy is made visible rather than performed silently.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if another school owns it.
    /// - [`DbError::StillReferenced`] while anybody is qualified for it. Call
    ///   [`Curriculum::subject_dependents`] first to say how many.
    /// - [`DbError::Conflict`] if somebody else edited it since it was read.
    /// - [`DbError::NotFound`] if no such subject exists.
    pub fn delete_subject(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "subject", "subject", id, school_id)?;
            let qualified: i64 = tx.query_row(
                "SELECT COUNT(*) FROM teacher_qualification WHERE subject_id = ?",
                [text(id)],
                |row| row.get(0),
            )?;
            // Stage C's addition, and the one `ROADMAP.md` named in advance. A course is an
            // offering *of* this subject, so deleting the subject underneath it would leave
            // a course teaching nothing — and with it the sections, staffing and roster
            // built on that course. `course.subject_id` is `RESTRICT` for the reason every
            // ownership edge is.
            let courses = super::lessons::courses_of_subject(tx, id)?;
            if qualified + courses > 0 {
                return Err(DbError::StillReferenced { entity: "subject" });
            }
            delete(tx, "subject", "subject", id, expected)
        })
    }
}

/// Writes one subject row.
///
/// Extracted from [`Curriculum::create_subject`] so an import can write several kinds of record
/// inside one transaction — see `super::import`. The public method and the importer go
/// through this one statement, so a new column cannot reach one path and miss the other.
pub(crate) fn insert_subject(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    input: &SubjectInput,
) -> Result<()> {
    tx.execute(
        "INSERT INTO subject (id, school_id, department_id, name, code, description,
             created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(input.school_id),
            opt_text(input.department_id),
            &input.name,
            &input.code,
            &input.description,
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| translate(error, "subject"))?;
    Ok(())
}

/// Writes one subject's domain columns, at the revision the caller expects.
///
/// Extracted from [`Curriculum::update_subject`] so an import can compose several writes inside one
/// transaction — see `super::import`. Both paths go through this one statement, so a new
/// column cannot reach one and miss the other. The school check and anything the public
/// method does *around* the write stay there, because the importer has already done its own.
pub(crate) fn update_subject_row(
    tx: &Transaction<'_>,
    id: Uuid,
    expected: Expected,
    input: &SubjectInput,
) -> Result<()> {
    update(
        tx,
        "subject",
        "subject",
        id,
        expected,
        SUBJECT_ASSIGNMENTS,
        rusqlite::params![
            opt_text(input.department_id),
            &input.name,
            &input.code,
            &input.description,
        ],
    )?;
    Ok(())
}

/// Refuses a department that belongs to another school, or accepts `None`.
///
/// `Invalid` rather than `WrongSchool`, deliberately: the *write* is for the right school,
/// and it is one field within it that names somebody else's row. `WrongSchool` is reserved
/// for "you are editing another school's record", which would send the user to switch
/// schools — the wrong advice here, where the fix is to pick a different department.
fn require_department_in_school(
    tx: &Transaction<'_>,
    department_id: Option<Uuid>,
    school_id: Uuid,
) -> Result<()> {
    let Some(department_id) = department_id else {
        return Ok(());
    };
    let owner: Option<String> = tx
        .query_row(
            "SELECT school_id FROM department WHERE id = ?",
            [text(department_id)],
            |row| row.get(0),
        )
        .optional()?;

    match owner {
        None => Err(DbError::NotFound {
            entity: "department",
        }),
        Some(raw) if raw == school_id.to_string() => Ok(()),
        Some(_) => Err(DbError::Invalid {
            message: "That department belongs to a different school. Choose one of this \
                      school's departments, or leave the subject without one."
                .to_owned(),
        }),
    }
}

// --------------------------------------------------------------------- qualifications

impl Curriculum<'_> {
    /// Records that a teacher is qualified to teach one of the acting school's subjects.
    ///
    /// # What this does not require
    ///
    /// **Not** that the teacher currently belongs to the school. ADR 0025 §4 argues it: a
    /// qualification is a statement about a person's competence, and tying it to current
    /// employment would make it expire when the employment did — so a school re-engaging
    /// somebody would have to re-enter what it already knew. Whether they may actually be
    /// *given* a lesson is a different question, and Stage C answers it with qualification
    /// **and** active membership **and** campus eligibility together.
    ///
    /// **Not** that this creates a membership either. Qualifying somebody is not employing
    /// them, and a call that quietly did both would make the Teachers screen's careful
    /// distinction between the two meaningless.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the subject belongs to another school. A school may
    ///   not record qualifications against an institution it does not run.
    /// - [`DbError::NotFound`] if no such teacher exists.
    /// - [`DbError::DuplicateValue`] if they are already qualified for it. Saying it twice
    ///   says nothing more, and is a mistake rather than a no-op.
    pub fn qualify_teacher(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        subject_id: Uuid,
    ) -> Result<TeacherQualification> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_same_school(tx, "subject", "subject", subject_id, school_id)?;
            require_teacher(tx, teacher_id)?;

            if qualification_of(tx, teacher_id, subject_id)?.is_some() {
                return Err(DbError::DuplicateValue {
                    entity: "qualification",
                    field: "subject",
                });
            }

            tx.execute(
                "INSERT INTO teacher_qualification (id, teacher_id, subject_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(teacher_id),
                    text(subject_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "qualification"))?;
            Ok(())
        })?;

        self.qualification(stamp.id)?.ok_or(DbError::NotFound {
            entity: "qualification",
        })
    }

    /// One qualification by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn qualification(&self, id: Uuid) -> Result<Option<TeacherQualification>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {QUALIFICATION_COLUMNS} FROM teacher_qualification WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], TeacherQualification::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every subject one teacher is qualified to teach, across every school.
    ///
    /// Project-wide on purpose. A shared teacher's qualifications at the school she visits
    /// are as real as the ones at the school that created her, and a screen that showed
    /// only the selected school's would make the other set invisible — including the ones
    /// at a school she has left, which are exactly the ones somebody would otherwise enter
    /// again. The caller groups them by `subject.schoolId`.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn qualifications_of_teacher(&self, teacher_id: Uuid) -> Result<Vec<QualifiedSubject>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}, d.name
                 FROM teacher_qualification q
                 JOIN subject s ON s.id = q.subject_id
                 LEFT JOIN department d ON d.id = s.department_id
                 WHERE q.teacher_id = ?
                 ORDER BY s.school_id, s.name, s.id",
                qualify(QUALIFICATION_COLUMNS, "q"),
                qualify(SUBJECT_COLUMNS, "s"),
            ))?;
            let offset = QUALIFICATION_COLUMNS.split(", ").count();
            let name_at = offset + SUBJECT_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(teacher_id)], |row| {
                Ok(QualifiedSubject {
                    qualification: TeacherQualification::from_row_at(row, 0)?,
                    subject: Subject::from_row_at(row, offset)?,
                    department_name: row.get(name_at)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Withdraws a qualification for one of the acting school's subjects.
    ///
    /// # Always explicit, never a side effect
    ///
    /// This is the **only** operation that removes a qualification short of deleting the
    /// person. Deactivating a teacher, withdrawing a school membership, changing campus
    /// eligibility, editing a name and changing a department association all leave every
    /// qualification exactly where it was. ADR 0025 §5 lists them, and the tests assert
    /// each one, because "we would never do that" is not a guarantee.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the subject belongs to another school.
    /// - [`DbError::Conflict`] if the qualification changed since it was read.
    /// - [`DbError::NotFound`] if they were not qualified for it.
    pub fn unqualify_teacher(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        subject_id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "subject", "subject", subject_id, school_id)?;
            let Some(id) = qualification_of(tx, teacher_id, subject_id)? else {
                return Err(DbError::NotFound {
                    entity: "qualification",
                });
            };
            delete(tx, "qualification", "teacher_qualification", id, expected)
        })
    }
}

// ----------------------------------------------------------- department associations

impl Curriculum<'_> {
    /// Puts a teacher in one of the acting school's departments.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the department belongs to another school. A school may
    ///   not decide the internal organisation of an institution it does not run, even for a
    ///   teacher it shares.
    /// - [`DbError::Invalid`] if the teacher does not belong to that department's school.
    ///   The message says to add the membership first, because that is the one ordering
    ///   mistake this rule actually produces and the database's own refusal cannot name the
    ///   remedy. This mirrors `assign_teacher_campus` exactly.
    /// - [`DbError::DuplicateValue`] if they are already in it.
    pub fn assign_teacher_department(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        department_id: Uuid,
    ) -> Result<TeacherDepartment> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_same_school(tx, "department", "department", department_id, school_id)?;
            require_teacher(tx, teacher_id)?;

            if !is_member(tx, teacher_id, school_id)? {
                return Err(DbError::Invalid {
                    message: "Add this teacher to the school before putting them in one of \
                              its departments."
                        .to_owned(),
                });
            }
            if association_of(tx, teacher_id, department_id)?.is_some() {
                return Err(DbError::DuplicateValue {
                    entity: "department association",
                    field: "department",
                });
            }

            tx.execute(
                "INSERT INTO teacher_department (id, teacher_id, department_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(teacher_id),
                    text(department_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "department association"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {TEACHER_DEPARTMENT_COLUMNS} FROM teacher_department WHERE id = ?"
                ))?;
                let mut rows = stmt.query_map([text(stamp.id)], TeacherDepartment::from_row)?;
                rows.next().transpose().map_err(DbError::from)
            })?
            .ok_or(DbError::NotFound {
                entity: "department association",
            })
    }

    /// Every department one teacher belongs to, across every school.
    ///
    /// Project-wide for the same reason as the qualifications listing: a teacher in
    /// Mathematics at one school and Science at another has two true answers, and showing
    /// one of them would hide the arrangement this table exists to express. The caller
    /// groups by `department.schoolId` and makes only the selected school's editable.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn departments_of_teacher(&self, teacher_id: Uuid) -> Result<Vec<TeacherDepartmentView>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {}
                 FROM teacher_department td
                 JOIN department d ON d.id = td.department_id
                 WHERE td.teacher_id = ?
                 ORDER BY d.school_id, d.name, d.id",
                qualify(TEACHER_DEPARTMENT_COLUMNS, "td"),
                qualify(DEPARTMENT_COLUMNS, "d"),
            ))?;
            let offset = TEACHER_DEPARTMENT_COLUMNS.split(", ").count();
            let rows = stmt.query_map([text(teacher_id)], |row| {
                Ok(TeacherDepartmentView {
                    association: TeacherDepartment::from_row_at(row, 0)?,
                    department: Department::from_row_at(row, offset)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Every teacher in one of the acting school's departments, by name.
    ///
    /// # Errors
    ///
    /// [`DbError::WrongSchool`] if the department belongs to another school.
    pub fn department_teachers(
        &self,
        school_id: Uuid,
        department_id: Uuid,
    ) -> Result<Vec<Teacher>> {
        self.db.read(|conn| {
            require_same_school(conn, "department", "department", department_id, school_id)?;

            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM teacher_department td
                 JOIN teacher t ON t.id = td.teacher_id
                 WHERE td.department_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                qualify(TEACHER_COLUMNS, "t"),
            ))?;
            let rows = stmt.query_map([text(department_id)], Teacher::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Takes a teacher out of one of the acting school's departments.
    ///
    /// Their school membership, their campus eligibility and every qualification they hold
    /// are untouched: this withdraws one organisational fact, not a relationship with the
    /// school and not a statement about what they can teach.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the department belongs to another school.
    /// - [`DbError::Conflict`] if the association changed since it was read.
    /// - [`DbError::NotFound`] if they were not in it.
    pub fn remove_teacher_department(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        department_id: Uuid,
        expected: Expected,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "department", "department", department_id, school_id)?;
            let Some(id) = association_of(tx, teacher_id, department_id)? else {
                return Err(DbError::NotFound {
                    entity: "department association",
                });
            };
            delete(
                tx,
                "department association",
                "teacher_department",
                id,
                expected,
            )
        })
    }
}

// ============================================================================ helpers

/// Prefixes every column in a `SELECT` list with a table alias.
///
/// The record readers decode positionally in declaration order, so a joined query has to
/// name the same columns in the same order — writing them out twice is what drifts. Same
/// helper as [`super::membership`]'s, kept local rather than shared because it is three
/// lines and a cross-module `pub(crate)` for it would be a worse coupling than the
/// duplication.
fn qualify(columns: &str, alias: &str) -> String {
    columns
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Refuses a teacher who is not in the project.
///
/// Reported before the insert so the message is about the person rather than about whichever
/// foreign key SQLite checked first.
fn require_teacher(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<()> {
    let found: i64 = tx.query_row(
        "SELECT COUNT(*) FROM teacher WHERE id = ?",
        [text(teacher_id)],
        |row| row.get(0),
    )?;
    if found == 0 {
        return Err(DbError::NotFound { entity: "teacher" });
    }
    Ok(())
}

/// Whether a school employs a teacher at all, active or not.
///
/// Membership rather than *active* membership, matching V13's eligibility rule: a teacher
/// who has left keeps the record of which departments they worked in, and re-employing them
/// should not require re-entering it.
fn is_member(tx: &Transaction<'_>, teacher_id: Uuid, school_id: Uuid) -> Result<bool> {
    let found: i64 = tx.query_row(
        "SELECT COUNT(*) FROM teacher_school WHERE teacher_id = ? AND school_id = ?",
        [text(teacher_id), text(school_id)],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

/// The qualification's identifier, or `None`.
fn qualification_of(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    subject_id: Uuid,
) -> Result<Option<Uuid>> {
    one_id(
        tx,
        "SELECT id FROM teacher_qualification WHERE teacher_id = ? AND subject_id = ?",
        "teacher_qualification.id",
        teacher_id,
        subject_id,
    )
}

/// The department association's identifier, or `None`.
fn association_of(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    department_id: Uuid,
) -> Result<Option<Uuid>> {
    one_id(
        tx,
        "SELECT id FROM teacher_department WHERE teacher_id = ? AND department_id = ?",
        "teacher_department.id",
        teacher_id,
        department_id,
    )
}

/// Reads one identifier addressed by a pair of identifiers.
fn one_id(
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

// --------------------------------------------------------- cross-repository helpers
//
// Called by `people` when it destroys a teacher and by `membership` when it withdraws a
// school's employment of one. They live here, next to the tables they are about, so the
// knowledge of what a curriculum row means is in one place — the same arrangement
// `membership` already has with `people`.

/// Every qualification one teacher holds, by identifier.
///
/// Used by `delete_teacher`. Deleting the person is the one operation that may take these:
/// a qualification with nobody to hold it is not history, it is a dangling fact.
pub(crate) fn qualifications_of(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Vec<Uuid>> {
    ids(
        tx,
        "SELECT id FROM teacher_qualification WHERE teacher_id = ?",
        "teacher_qualification.id",
        &[&text(teacher_id)],
    )
}

/// Every department association one teacher holds, by identifier.
///
/// Used by `delete_teacher`.
pub(crate) fn associations_of(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Vec<Uuid>> {
    ids(
        tx,
        "SELECT id FROM teacher_department WHERE teacher_id = ?",
        "teacher_department.id",
        &[&text(teacher_id)],
    )
}

/// One teacher's department associations within one school, by identifier.
///
/// Used by `remove_teacher_from_school`, which clears them in the same transaction. The
/// schema has a trigger refusing a membership removal that would leave one behind — this is
/// the path that makes the removal possible, and doing it through `repo::delete` is what
/// makes each one a truthful change-log entry.
pub(crate) fn associations_in_school(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    school_id: Uuid,
) -> Result<Vec<Uuid>> {
    ids(
        tx,
        "SELECT td.id FROM teacher_department td
         JOIN department d ON d.id = td.department_id
         WHERE td.teacher_id = ? AND d.school_id = ?",
        "teacher_department.id",
        &[&text(teacher_id), &text(school_id)],
    )
}

/// How many departments and subjects one school owns.
///
/// Used by `Structure::school_dependents`, so a school that still describes a curriculum is
/// *explained* rather than merely refused.
pub(crate) fn curriculum_counts(
    conn: &rusqlite::Connection,
    school_id: Uuid,
) -> Result<(i64, i64)> {
    let key = text(school_id);
    let departments: i64 = conn.query_row(
        "SELECT COUNT(*) FROM department WHERE school_id = ?",
        [&key],
        |row| row.get(0),
    )?;
    let subjects: i64 = conn.query_row(
        "SELECT COUNT(*) FROM subject WHERE school_id = ?",
        [&key],
        |row| row.get(0),
    )?;
    Ok((departments, subjects))
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
