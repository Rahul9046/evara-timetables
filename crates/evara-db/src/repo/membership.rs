//! School membership, campus eligibility and enrolment history for project-level people.
//!
//! `V13` (ADR 0021) separated **who someone is** from **where they belong**. A teacher and
//! a student are one row each, for one real person, for the life of the project; which
//! schools employ or enrol them is a relationship, and relationships are what this module
//! reads and writes.
//!
//! # The three tables, and why they are shaped differently
//!
//! | Table | Shape | Because |
//! | --- | --- | --- |
//! | `teacher_school` | one row per (teacher, school), deactivated rather than duplicated | "Has she left?" is a yes/no about now |
//! | `teacher_campus` | one row per (teacher, campus), a *narrowing* of membership | "Junior campus only" is not expressible by membership alone |
//! | `student_school_enrolment` | one row per **spell**, history kept | "When did he join, leave, come back?" is what a school is asked |
//!
//! # Two levels of "active", and why both are needed
//!
//! - [`Teacher::is_active`](super::people::Teacher::is_active) — in the project at all. An
//!   archive flag.
//! - [`TeacherSchool::is_active`] — employed by *this* school.
//!
//! Effective eligibility is the **conjunction**, which [`Membership::teaches_at`] is the
//! one place that computes. A teacher active in the project but inactive at Southmoor is
//! not available to Southmoor, and nothing may read the project-level flag alone and
//! conclude otherwise — that is the mistake this split exists to make impossible.
//!
//! # Authority
//!
//! Every mutation here names the school it acts for, and refuses an identifier belonging
//! to another one, inside the same transaction as the write. That is `V12`'s rule, and a
//! shared identity makes it matter more rather than less: Northgate holding a shared
//! teacher's identifier is now completely ordinary, and must still not be authority to
//! touch Southmoor's employment of them.
//!
//! [`Membership::transfer_student`] is the single deliberate exception, and it is explicit
//! about it: both schools are named in the signature, because a transfer is a
//! project-level act and splitting it in two would allow a student to end up enrolled
//! nowhere if the second half failed. See its own documentation.
//!
//! # What a school may not do by letting go
//!
//! Removing a membership is not deleting a person. The three operations are kept distinct,
//! the way ADR 0019 kept them distinct for a shared room:
//!
//! | Operation | Effect |
//! | --- | --- |
//! | Deactivate a membership | the person stays, keeps everything, stops being offered here |
//! | Remove a membership | this school's relationship goes; the person and every other school's relationship stay |
//! | Delete the identity | only the home school, and only while nobody else holds a relationship |
//!
//! The last of those lives in [`super::people`], next to the other deletions.

use rusqlite::{OptionalExtension, Transaction, types::ToSql};
use uuid::Uuid;

use super::people::{Student, Teacher};
use super::row::{entity, opt_text, parse_id_text, text};
use super::{Stamp, delete, require_same_school, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing memberships, eligibility and enrolment.
///
/// Obtained from [`Database::membership`]. Holds the database mutably because most
/// operations write.
#[derive(Debug)]
pub struct Membership<'a> {
    db: &'a mut Database,
}

impl<'a> Membership<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

// ================================================================== teacher membership

entity! {
    /// One teacher's employment by one school.
    ///
    /// The authoritative answer to "which schools does this teacher belong to?".
    /// `teacher.school_id` is the **home** school — the one they were created under — and
    /// is never the whole answer once a project holds more than one school.
    TeacherSchool,
    /// Values for adding a teacher to a school.
    TeacherSchoolInput,
    {
        /// The teacher.
        teacher_id: Uuid,
        /// The school employing them.
        school_id: Uuid,
        /// Whether they are currently employed **by this school**.
        ///
        /// Distinct from [`Teacher::is_active`](super::people::Teacher::is_active), which
        /// says whether they are in the project at all. A teacher may be active in the
        /// project and inactive here, and that combination is the ordinary one for
        /// somebody who has left one school of a trust and stayed at another.
        is_active: bool,
    }
}

const TEACHER_SCHOOL_COLUMNS: &str =
    "id, teacher_id, school_id, is_active, created_at, updated_at, rev";

/// A teacher as one school sees them: the shared identity, and this school's view of it.
///
/// Returned by [`Membership::school_teachers`] rather than a bare [`Teacher`] so a listing
/// can show per-school employment without a follow-up question per row. The teacher inside
/// is **one** teacher: two schools listing them are listing the same `teacher.id`, which
/// is what makes their availability a single fact rather than one per school.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TeacherInSchool {
    /// The shared identity. Its `isActive` is the project-level flag.
    pub teacher: Teacher,
    /// This school's employment of them.
    pub membership: TeacherSchool,
}

impl TeacherInSchool {
    /// Whether this school may offer them new work.
    ///
    /// The conjunction of both flags, computed in one place so no caller has to remember
    /// that there are two. See the module documentation.
    #[must_use]
    pub const fn is_effectively_active(&self) -> bool {
        self.teacher.is_active && self.membership.is_active
    }
}

impl Membership<'_> {
    /// Adds an existing teacher to another school.
    ///
    /// No teacher row is written: that is the point. The same person gains a second
    /// employer, keeps their identifier, their availability and every assignment they
    /// already had.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if that school already employs them. Refused rather
    ///   than quietly ignored: reaching this point means the caller's idea of the
    ///   membership set is stale, and it should re-read rather than believe a success it
    ///   did not cause.
    /// - [`DbError::NotFound`] shape errors if either end does not exist.
    pub fn add_teacher_to_school(&mut self, input: &TeacherSchoolInput) -> Result<TeacherSchool> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            if membership_of(tx, input.teacher_id, input.school_id)?.is_some() {
                return Err(DbError::DuplicateValue {
                    entity: "school membership",
                    field: "school",
                });
            }
            insert_membership(
                tx,
                &stamp,
                input.teacher_id,
                input.school_id,
                input.is_active,
            )
        })?;
        self.teacher_membership(stamp.id)?.ok_or(DbError::NotFound {
            entity: "school membership",
        })
    }

    /// One membership by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teacher_membership(&self, id: Uuid) -> Result<Option<TeacherSchool>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TEACHER_SCHOOL_COLUMNS} FROM teacher_school WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], TeacherSchool::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every school one teacher belongs to, by school name.
    ///
    /// The complete answer. `teacher.school_id` names only the home school and is never
    /// consulted here.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teacher_memberships(&self, teacher_id: Uuid) -> Result<Vec<TeacherSchool>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM teacher_school ts
                 JOIN school s ON s.id = ts.school_id
                 WHERE ts.teacher_id = ? ORDER BY s.name, s.id",
                qualify(TEACHER_SCHOOL_COLUMNS, "ts")
            ))?;
            let rows = stmt.query_map([text(teacher_id)], TeacherSchool::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Every teacher one school employs, with that school's view of them, by name.
    ///
    /// Reads membership, not `teacher.school_id`. A teacher shared with another school
    /// appears here as the same row with the same identifier they have there — which is
    /// what makes one person show up in two staff lists without being duplicated.
    ///
    /// Inactive memberships are included, for the reason
    /// [`People::teachers`](super::people::People::teachers) includes inactive teachers:
    /// deciding who to *offer* is the interface's business, and a list that hid them would
    /// also hide somebody a timetable already names.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn school_teachers(&self, school_id: Uuid) -> Result<Vec<TeacherInSchool>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {} FROM teacher t
                 JOIN teacher_school ts ON ts.teacher_id = t.id
                 WHERE ts.school_id = ?
                 ORDER BY t.family_name, t.given_name, t.id",
                qualify(super::people::TEACHER_COLUMNS, "t"),
                qualify(TEACHER_SCHOOL_COLUMNS, "ts"),
            ))?;
            let offset = column_count(super::people::TEACHER_COLUMNS);
            let rows = stmt.query_map([text(school_id)], |row| {
                Ok(TeacherInSchool {
                    teacher: Teacher::from_row(row)?,
                    membership: TeacherSchool::from_row_at(row, offset)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Whether one school may offer one teacher new work.
    ///
    /// The conjunction of the project-level and per-school flags, and the only place that
    /// conjunction is written. A teacher with no membership of this school is not
    /// eligible, which is a different statement from "inactive" and gives the same answer.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn teaches_at(&self, school_id: Uuid, teacher_id: Uuid) -> Result<bool> {
        self.db.read(|conn| {
            let found: i64 = conn.query_row(
                "SELECT COUNT(*) FROM teacher_school ts
                 JOIN teacher t ON t.id = ts.teacher_id
                 WHERE ts.teacher_id = ? AND ts.school_id = ?
                   AND ts.is_active = 1 AND t.is_active = 1",
                [text(teacher_id), text(school_id)],
                |row| row.get(0),
            )?;
            Ok(found > 0)
        })
    }

    /// Activates or deactivates one school's employment of one teacher.
    ///
    /// Nothing else changes: not the person, not their availability, not any other
    /// school's membership, and not their eligibility for this school's campuses — a
    /// teacher who has left keeps the record of which campuses they worked at, the way an
    /// inactive teacher keeps every assignment they ever had.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if that school does not employ them.
    pub fn set_teacher_membership_active(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        is_active: bool,
    ) -> Result<TeacherSchool> {
        let id = self.db.write(|tx| {
            let id = require_membership(tx, teacher_id, school_id)?;
            update(
                tx,
                "school membership",
                "teacher_school",
                id,
                "is_active = ?",
                &[&is_active as &dyn ToSql],
            )?;
            Ok(id)
        })?;
        self.teacher_membership(id)?.ok_or(DbError::NotFound {
            entity: "school membership",
        })
    }

    /// Removes one school's employment of a teacher entirely.
    ///
    /// The teacher is untouched. Every other school keeps them, their availability stays,
    /// and their identifier never changes — this is withdrawing a relationship, not
    /// deleting a person, and the two must never be reachable by the same action.
    ///
    /// The teacher's eligibility for **this school's** campuses goes with it, in the same
    /// transaction and through [`super::delete`] so each removal is a truthful change-log
    /// entry. Eligibility that outlived its membership would be exactly the drift
    /// `teacher_campus_requires_school_membership` exists to forbid.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if that school does not employ them.
    /// - [`DbError::Invalid`] if it is the teacher's home school. Withdrawing that would
    ///   leave the person belonging to no school and visible on no screen; deactivating
    ///   the membership is what was meant, and the message says so.
    pub fn remove_teacher_from_school(&mut self, school_id: Uuid, teacher_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            let id = require_membership(tx, teacher_id, school_id)?;

            if teacher_home_school(tx, teacher_id)? == school_id {
                return Err(DbError::Invalid {
                    message: "This teacher was created in this school, so this school \
                              cannot be removed from them. Clear “Currently teaching” to \
                              record that they have left, or delete the teacher."
                        .to_owned(),
                });
            }

            for eligibility in eligibility_in_school(tx, teacher_id, school_id)? {
                delete(tx, "campus eligibility", "teacher_campus", eligibility)?;
            }
            delete(tx, "school membership", "teacher_school", id)
        })
    }
}

// ================================================================= campus eligibility

entity! {
    /// One teacher's eligibility to be based at one campus.
    ///
    /// A **narrowing** of membership, never an independent claim: the schema refuses a row
    /// naming a campus of a school the teacher does not belong to.
    TeacherCampus,
    /// Values for making a teacher eligible at a campus.
    TeacherCampusInput,
    {
        /// The teacher.
        teacher_id: Uuid,
        /// The campus they may be based at.
        campus_id: Uuid,
    }
}

const TEACHER_CAMPUS_COLUMNS: &str = "id, teacher_id, campus_id, created_at, updated_at, rev";

impl Membership<'_> {
    /// Every campus one teacher is eligible at, by campus name.
    ///
    /// Across every school they belong to. A teacher at two schools of one trust may
    /// legitimately be eligible at campuses of both, and the caller filters by school if
    /// that is what it is showing.
    ///
    /// # Errors
    ///
    /// Propagates any database failure, or [`DbError::CorruptIdentifier`].
    pub fn teacher_campuses(&self, teacher_id: Uuid) -> Result<Vec<Uuid>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT tc.campus_id FROM teacher_campus tc
                 JOIN campus c ON c.id = tc.campus_id
                 WHERE tc.teacher_id = ? ORDER BY c.name, c.id",
            )?;
            let raw = stmt
                .query_map([text(teacher_id)], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            raw.iter()
                .map(|value| parse_id_text("teacher_campus.campus_id", value))
                .collect()
        })
    }

    /// Makes a teacher eligible at one of the acting school's campuses.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the campus belongs to another school. A school may
    ///   not decide who works at an institution it does not run, even for a teacher it
    ///   shares.
    /// - [`DbError::Invalid`] if the teacher does not belong to that campus's school. The
    ///   message says to add the membership first, because that is the one ordering
    ///   mistake this rule actually produces — the database's own refusal cannot name the
    ///   remedy.
    /// - [`DbError::DuplicateValue`] if they are already eligible there.
    pub fn assign_teacher_campus(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        campus_id: Uuid,
    ) -> Result<TeacherCampus> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_same_school(tx, "campus", "campus", campus_id, school_id)?;
            require_teacher(tx, teacher_id)?;

            if membership_of(tx, teacher_id, school_id)?.is_none() {
                return Err(DbError::Invalid {
                    message: "Add this teacher to the school before giving them one of its \
                              campuses."
                        .to_owned(),
                });
            }
            if eligibility_of(tx, teacher_id, campus_id)?.is_some() {
                return Err(DbError::DuplicateValue {
                    entity: "campus eligibility",
                    field: "campus",
                });
            }

            tx.execute(
                "INSERT INTO teacher_campus (id, teacher_id, campus_id, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(teacher_id),
                    text(campus_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "campus eligibility"))?;
            Ok(())
        })?;

        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {TEACHER_CAMPUS_COLUMNS} FROM teacher_campus WHERE id = ?"
                ))?;
                let mut rows = stmt.query_map([text(stamp.id)], TeacherCampus::from_row)?;
                rows.next().transpose().map_err(DbError::from)
            })?
            .ok_or(DbError::NotFound {
                entity: "campus eligibility",
            })
    }

    /// Withdraws a teacher's eligibility for one of the acting school's campuses.
    ///
    /// The membership is untouched: they still belong to the school, simply not to that
    /// site.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the campus belongs to another school.
    /// - [`DbError::NotFound`] if they were not eligible there.
    pub fn remove_teacher_campus(
        &mut self,
        school_id: Uuid,
        teacher_id: Uuid,
        campus_id: Uuid,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "campus", "campus", campus_id, school_id)?;
            let Some(id) = eligibility_of(tx, teacher_id, campus_id)? else {
                return Err(DbError::NotFound {
                    entity: "campus eligibility",
                });
            };
            delete(tx, "campus eligibility", "teacher_campus", id)
        })
    }
}

// ================================================================== student enrolment

/// Where a student stands in one spell at one school.
///
/// Carried as a string in the schema and checked there too, so a project opened by an
/// older build fails loudly rather than reading an unknown status as a plausible one —
/// the same rule [`GroupKind`](super::people::GroupKind) follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EnrolmentStatus {
    /// Attending.
    Enrolled,
    /// A past spell. History, kept for ever, and the only status a `leftOn` date may
    /// accompany.
    Left,
    /// An applicant: recorded and placed, not yet attending.
    Prospective,
}

impl EnrolmentStatus {
    /// The value stored in `student_school_enrolment.status`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Enrolled => "ENROLLED",
            Self::Left => "LEFT",
            Self::Prospective => "PROSPECTIVE",
        }
    }

    /// Whether this spell is the student's standing at the school *now*.
    ///
    /// The schema's `student_enrolment_one_current` index permits exactly one such row per
    /// (student, school); every other spell is `LEFT`.
    #[must_use]
    pub const fn is_current(self) -> bool {
        !matches!(self, Self::Left)
    }
}

impl super::row::FromColumn for EnrolmentStatus {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "ENROLLED" => Ok(Self::Enrolled),
            "LEFT" => Ok(Self::Left),
            "PROSPECTIVE" => Ok(Self::Prospective),
            // A status the schema's CHECK should have refused is corruption, not input.
            _ => Err(rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(crate::error::CorruptId {
                    column: "student_school_enrolment.status".to_owned(),
                    value: raw,
                }),
            )),
        }
    }
}

entity! {
    /// One student's spell at one school.
    ///
    /// A transfer writes a new one and keeps the old; it never edits a spell into being a
    /// different spell. That is what makes "was she ever at Northgate?" answerable years
    /// later, and it is why this is modelled as history where teacher membership is not.
    StudentEnrolment,
    /// Values for enrolling a student at a school.
    StudentEnrolmentInput,
    {
        /// The student. Their identity is unchanged by every operation here.
        student_id: Uuid,
        /// The school they are enrolled at.
        school_id: Uuid,
        /// This school's own number for them. Unique among the school's current
        /// enrolments when given; never project-wide, because two schools may legitimately
        /// number the same person differently.
        admission_no: Option<String>,
        /// Which of **this school's** year levels they sit in.
        year_level_id: Option<Uuid>,
        /// Which of **this school's** campuses they attend.
        campus_id: Option<Uuid>,
        /// Where this spell stands.
        status: EnrolmentStatus,
        /// ISO-8601 date they joined, if the school records it.
        enrolled_on: Option<String>,
        /// ISO-8601 date they left. Only a `LEFT` spell may carry one.
        left_on: Option<String>,
    }
}

const ENROLMENT_COLUMNS: &str = "id, student_id, school_id, admission_no, year_level_id, \
     campus_id, status, enrolled_on, left_on, created_at, updated_at, rev";

const ENROLMENT_ASSIGNMENTS: &str = "admission_no = ?, year_level_id = ?, campus_id = ?, \
     status = ?, enrolled_on = ?, left_on = ?";

/// A student as one school sees them: the shared identity, and this school's spell.
///
/// The student inside is **one** student. Two schools listing them are listing the same
/// `student.id`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StudentInSchool {
    /// The shared identity.
    pub student: Student,
    /// This school's current spell for them.
    pub enrolment: StudentEnrolment,
}

impl Membership<'_> {
    /// Enrols an existing student at a school.
    ///
    /// No student row is written. The same person gains a place, keeps their identifier,
    /// and keeps every group membership and every past spell they already had.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if they already have a current spell at that school.
    ///   Two current spells at one school is not a thing that can be true, and the schema
    ///   refuses it as well.
    /// - [`DbError::Invalid`] if the year level or campus belongs to another school.
    /// - [`DbError::NotFound`] shape errors if either end does not exist.
    pub fn enrol_student(&mut self, input: &StudentEnrolmentInput) -> Result<StudentEnrolment> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_enrolment_placement(tx, input)?;
            if input.status.is_current()
                && current_enrolment(tx, input.student_id, input.school_id)?.is_some()
            {
                return Err(DbError::DuplicateValue {
                    entity: "enrolment",
                    field: "school",
                });
            }
            insert_enrolment(tx, &stamp, input)
        })?;
        self.enrolment(stamp.id)?.ok_or(DbError::NotFound {
            entity: "enrolment",
        })
    }

    /// One enrolment by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn enrolment(&self, id: Uuid) -> Result<Option<StudentEnrolment>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENROLMENT_COLUMNS} FROM student_school_enrolment WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], StudentEnrolment::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every spell one student has ever had, oldest first.
    ///
    /// The whole history, across every school. Ordered by `created_at` then `id` rather
    /// than by `enrolled_on`, because that date is optional and a history that reordered
    /// itself when somebody filled one in would be worse than useless.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn student_enrolments(&self, student_id: Uuid) -> Result<Vec<StudentEnrolment>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENROLMENT_COLUMNS} FROM student_school_enrolment
                 WHERE student_id = ? ORDER BY created_at, id"
            ))?;
            let rows = stmt.query_map([text(student_id)], StudentEnrolment::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// One student's current spell at one school, if they have one.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn current_enrolment_at(
        &self,
        school_id: Uuid,
        student_id: Uuid,
    ) -> Result<Option<StudentEnrolment>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ENROLMENT_COLUMNS} FROM student_school_enrolment
                 WHERE student_id = ? AND school_id = ? AND status <> 'LEFT'"
            ))?;
            let mut rows = stmt.query_map(
                [text(student_id), text(school_id)],
                StudentEnrolment::from_row,
            )?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every student currently at one school, with that school's placement for them.
    ///
    /// Current spells only — `ENROLLED` and `PROSPECTIVE`. Past spells are history and are
    /// read through [`Membership::student_enrolments`]; a roll call that listed everybody
    /// who had ever attended would be a different question.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn school_students(&self, school_id: Uuid) -> Result<Vec<StudentInSchool>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {}, {} FROM student s
                 JOIN student_school_enrolment e ON e.student_id = s.id
                 WHERE e.school_id = ? AND e.status <> 'LEFT'
                 ORDER BY s.family_name, s.given_name, s.id",
                qualify(super::people::STUDENT_COLUMNS, "s"),
                qualify(ENROLMENT_COLUMNS, "e"),
            ))?;
            let offset = column_count(super::people::STUDENT_COLUMNS);
            let rows = stmt.query_map([text(school_id)], |row| {
                Ok(StudentInSchool {
                    student: Student::from_row(row)?,
                    enrolment: StudentEnrolment::from_row_at(row, offset)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces one spell's placement, status and dates.
    ///
    /// The student and the school are not among them: a spell is *at* a school, and
    /// repointing one would rewrite history rather than record it. Moving a student
    /// between schools is [`Membership::transfer_student`].
    ///
    /// When this is the student's current spell at their **home** school, the matching
    /// columns on `student` are kept in step in the same transaction — see
    /// [`super::people::People::update_student`] for the whole compatibility policy and
    /// why there is deliberately one authority rather than two.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the spell belongs to another school.
    /// - [`DbError::NotFound`] if there is no such spell.
    /// - [`DbError::Invalid`] if the year level or campus belongs to another school.
    /// - [`DbError::DuplicateValue`] if making this spell current would give the student
    ///   two current spells at one school, or if the admission number is taken.
    pub fn update_enrolment(
        &mut self,
        school_id: Uuid,
        id: Uuid,
        input: &StudentEnrolmentInput,
    ) -> Result<StudentEnrolment> {
        self.db.write(|tx| {
            require_same_school(tx, "enrolment", "student_school_enrolment", id, school_id)?;
            require_enrolment_placement(tx, input)?;
            require_room_for_this_spell(tx, id, input)?;
            write_enrolment(tx, id, input)?;
            mirror_enrolment_onto_student(tx, id)
        })?;
        self.enrolment(id)?.ok_or(DbError::NotFound {
            entity: "enrolment",
        })
    }

    /// Records that a student has left one school, keeping the spell.
    ///
    /// The spell becomes `LEFT` and gains its end date. Nothing is deleted: the admission
    /// number, the year level and the campus stay on the row, because "which year was she
    /// in when she left?" is a question a school asks.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if they have no current spell at that school.
    /// - [`DbError::Invalid`] if `left_on` is not an ISO-8601 date, or is before the date
    ///   they joined.
    pub fn leave_school(
        &mut self,
        school_id: Uuid,
        student_id: Uuid,
        left_on: Option<&str>,
    ) -> Result<StudentEnrolment> {
        let id = self.db.write(|tx| {
            let id = require_current_enrolment(tx, student_id, school_id)?;
            end_spell(tx, id, left_on)?;
            Ok(id)
        })?;
        self.enrolment(id)?.ok_or(DbError::NotFound {
            entity: "enrolment",
        })
    }

    /// Moves a student from one school to another, preserving both the identity and the
    /// history.
    ///
    /// The old spell becomes `LEFT`; a new spell is written at the receiving school. The
    /// student row is never touched, so `student.id` — and every group membership, every
    /// past spell, every timetable that names them — survives unchanged. That is the
    /// structural guarantee: there is no code path here that could produce a second
    /// student record.
    ///
    /// The old spell ends on `arriving.enrolled_on`, because that is the date the transfer
    /// happened and the only one the caller has. A transfer with no date recorded ends the
    /// old spell with none either, rather than inventing one — the `LEFT` status is the
    /// fact, and the date is optional for the reason `V13` made it optional.
    ///
    /// # Why this names two schools
    ///
    /// Every other mutation in this module acts for exactly one school and refuses
    /// anything belonging to another. A transfer cannot: it is one fact about two
    /// institutions. Splitting it into [`Membership::leave_school`] plus
    /// [`Membership::enrol_student`] would be two transactions, and a failure between them
    /// leaves a real student enrolled nowhere — the outcome the atomicity exists to
    /// prevent.
    ///
    /// So the authority is made **explicit in the signature** instead of implied by a
    /// single acting school: a caller must name both ends, and the Stage 0C interface must
    /// offer this only where the user has standing in both. ADR 0021 §7 records the
    /// decision.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if the student has no current spell at `from_school_id`.
    /// - [`DbError::DuplicateValue`] if they already have a current spell at
    ///   `to_school_id`. Transferring somewhere they already are is a mistake, not a
    ///   no-op.
    /// - [`DbError::Invalid`] if the new placement belongs to a third school.
    pub fn transfer_student(
        &mut self,
        from_school_id: Uuid,
        to_school_id: Uuid,
        student_id: Uuid,
        arriving: &StudentEnrolmentInput,
    ) -> Result<StudentEnrolment> {
        if from_school_id == to_school_id {
            return Err(DbError::Invalid {
                message: "A transfer needs two different schools.".to_owned(),
            });
        }
        if arriving.student_id != student_id || arriving.school_id != to_school_id {
            return Err(DbError::Invalid {
                message: "The arriving enrolment must name the student and the school being \
                          transferred to."
                    .to_owned(),
            });
        }
        if !arriving.status.is_current() {
            return Err(DbError::Invalid {
                message: "A transfer arrives as an enrolment, not as a past one.".to_owned(),
            });
        }

        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_enrolment_placement(tx, arriving)?;

            let leaving = require_current_enrolment(tx, student_id, from_school_id)?;
            if current_enrolment(tx, student_id, to_school_id)?.is_some() {
                return Err(DbError::DuplicateValue {
                    entity: "enrolment",
                    field: "school",
                });
            }

            // The old spell is ended rather than repointed. Both halves are in this one
            // transaction, so the student is never enrolled twice and never enrolled
            // nowhere, whichever of them fails.
            end_spell(tx, leaving, arriving.enrolled_on.as_deref())?;
            insert_enrolment(tx, &stamp, arriving)
        })?;
        self.enrolment(stamp.id)?.ok_or(DbError::NotFound {
            entity: "enrolment",
        })
    }
}

// ============================================================================ helpers

/// Prefixes every column in a `SELECT` list with a table alias.
///
/// The record readers decode positionally in declaration order, so a joined query has to
/// name the same columns in the same order — writing them out twice is what drifts.
fn qualify(columns: &str, alias: &str) -> String {
    columns
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How many columns one record's `SELECT` list occupies.
///
/// Derived from the list rather than written out, so a column added to an entity cannot
/// leave a joined read decoding the next record from the wrong offset — which would not
/// fail loudly, it would silently read one field as another.
fn column_count(columns: &str) -> usize {
    columns.split(", ").count()
}

/// The membership's identifier, or `None` when that school does not employ them.
fn membership_of(tx: &Transaction<'_>, teacher_id: Uuid, school_id: Uuid) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM teacher_school WHERE teacher_id = ? AND school_id = ?",
            [text(teacher_id), text(school_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text("teacher_school.id", value))
        .transpose()
}

/// The membership's identifier, or [`DbError::NotFound`].
fn require_membership(tx: &Transaction<'_>, teacher_id: Uuid, school_id: Uuid) -> Result<Uuid> {
    membership_of(tx, teacher_id, school_id)?.ok_or(DbError::NotFound {
        entity: "school membership",
    })
}

/// Writes one membership row.
pub(crate) fn insert_membership(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    teacher_id: Uuid,
    school_id: Uuid,
    is_active: bool,
) -> Result<()> {
    tx.execute(
        "INSERT INTO teacher_school (id, teacher_id, school_id, is_active,
                                     created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(teacher_id),
            text(school_id),
            is_active,
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| translate(error, "school membership"))?;
    Ok(())
}

/// The school a teacher was created in, or [`DbError::NotFound`].
fn teacher_home_school(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Uuid> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT school_id FROM teacher WHERE id = ?",
            [text(teacher_id)],
            |row| row.get(0),
        )
        .optional()?;
    let raw = raw.ok_or(DbError::NotFound { entity: "teacher" })?;
    parse_id_text("teacher.school_id", &raw)
}

/// Refuses a teacher identifier that is not there.
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

/// One eligibility row's identifier, or `None`.
fn eligibility_of(tx: &Transaction<'_>, teacher_id: Uuid, campus_id: Uuid) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM teacher_campus WHERE teacher_id = ? AND campus_id = ?",
            [text(teacher_id), text(campus_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text("teacher_campus.id", value))
        .transpose()
}

/// Identifiers of one teacher's eligibility rows within one school.
pub(crate) fn eligibility_in_school(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    school_id: Uuid,
) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare(
        "SELECT tc.id FROM teacher_campus tc
         JOIN campus c ON c.id = tc.campus_id
         WHERE tc.teacher_id = ? AND c.school_id = ? ORDER BY tc.id",
    )?;
    let raw = stmt
        .query_map([text(teacher_id), text(school_id)], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("teacher_campus.id", value))
        .collect()
}

/// Identifiers of every eligibility row of one teacher.
pub(crate) fn eligibility_of_teacher(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare("SELECT id FROM teacher_campus WHERE teacher_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(teacher_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("teacher_campus.id", value))
        .collect()
}

/// Identifiers of every eligibility row at one campus.
pub(crate) fn eligibility_at_campus(tx: &Transaction<'_>, campus_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare("SELECT id FROM teacher_campus WHERE campus_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(campus_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("teacher_campus.id", value))
        .collect()
}

/// Identifiers of every membership of one teacher.
pub(crate) fn memberships_of_teacher(tx: &Transaction<'_>, teacher_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare("SELECT id FROM teacher_school WHERE teacher_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(teacher_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("teacher_school.id", value))
        .collect()
}

/// How many schools other than `school_id` employ this teacher.
pub(crate) fn other_employers(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    school_id: Uuid,
) -> Result<i64> {
    tx.query_row(
        "SELECT COUNT(*) FROM teacher_school WHERE teacher_id = ? AND school_id <> ?",
        [text(teacher_id), text(school_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// The identifier of a student's current spell at one school, or `None`.
fn current_enrolment(
    tx: &Transaction<'_>,
    student_id: Uuid,
    school_id: Uuid,
) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM student_school_enrolment
             WHERE student_id = ? AND school_id = ? AND status <> 'LEFT'",
            [text(student_id), text(school_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text("student_school_enrolment.id", value))
        .transpose()
}

/// The identifier of a current spell, or [`DbError::NotFound`].
fn require_current_enrolment(
    tx: &Transaction<'_>,
    student_id: Uuid,
    school_id: Uuid,
) -> Result<Uuid> {
    current_enrolment(tx, student_id, school_id)?.ok_or(DbError::NotFound {
        entity: "enrolment",
    })
}

/// Identifiers of every spell of one student.
pub(crate) fn enrolments_of_student(tx: &Transaction<'_>, student_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt =
        tx.prepare("SELECT id FROM student_school_enrolment WHERE student_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(student_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("student_school_enrolment.id", value))
        .collect()
}

/// How many schools other than `school_id` hold a spell for this student.
pub(crate) fn other_schools_attended(
    tx: &Transaction<'_>,
    student_id: Uuid,
    school_id: Uuid,
) -> Result<i64> {
    tx.query_row(
        "SELECT COUNT(*) FROM student_school_enrolment WHERE student_id = ? AND school_id <> ?",
        [text(student_id), text(school_id)],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

/// Refuses a placement that names another school's year level or campus.
///
/// The schema refuses it too. This runs first so the message can say *which field* is
/// wrong, which a trigger cannot — the division of labour `V12` set out.
fn require_enrolment_placement(tx: &Transaction<'_>, input: &StudentEnrolmentInput) -> Result<()> {
    if let Some(year_level_id) = input.year_level_id {
        let owner: Option<String> = tx
            .query_row(
                "SELECT school_id FROM year_level WHERE id = ?",
                [text(year_level_id)],
                |row| row.get(0),
            )
            .optional()?;
        match owner {
            None => {
                return Err(DbError::NotFound {
                    entity: "year level",
                });
            }
            Some(raw) if raw == text(input.school_id) => {}
            Some(_) => {
                return Err(DbError::Invalid {
                    message: "That year level belongs to a different school. Pick one of this \
                              school's."
                        .to_owned(),
                });
            }
        }
    }

    if let Some(campus_id) = input.campus_id {
        let owner: Option<String> = tx
            .query_row(
                "SELECT school_id FROM campus WHERE id = ?",
                [text(campus_id)],
                |row| row.get(0),
            )
            .optional()?;
        match owner {
            None => return Err(DbError::NotFound { entity: "campus" }),
            Some(raw) if raw == text(input.school_id) => {}
            Some(_) => {
                return Err(DbError::Invalid {
                    message: "That campus belongs to a different school. Pick one of this \
                              school's."
                        .to_owned(),
                });
            }
        }
    }

    Ok(())
}

/// Writes one enrolment row.
pub(crate) fn insert_enrolment(
    tx: &Transaction<'_>,
    stamp: &Stamp,
    input: &StudentEnrolmentInput,
) -> Result<()> {
    tx.execute(
        "INSERT INTO student_school_enrolment
             (id, student_id, school_id, admission_no, year_level_id, campus_id,
              status, enrolled_on, left_on, created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            text(stamp.id),
            text(input.student_id),
            text(input.school_id),
            &input.admission_no,
            opt_text(input.year_level_id),
            opt_text(input.campus_id),
            input.status.as_str(),
            &input.enrolled_on,
            &input.left_on,
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(enrolment_error)?;
    Ok(())
}

/// Refuses an edit that would make a spell collide with one the student already has.
///
/// Both rules are partial unique indexes in the schema, so this is not what makes them
/// true. It is what makes them *sayable*: `repo::update` routes a unique failure through
/// `repo::translate`, which cannot tell the one-current-spell index from the admission
/// number and would report either as "another enrolment already uses that name".
fn require_room_for_this_spell(
    tx: &Transaction<'_>,
    id: Uuid,
    input: &StudentEnrolmentInput,
) -> Result<()> {
    if !input.status.is_current() {
        return Ok(());
    }

    if let Some(current) = current_enrolment(tx, input.student_id, input.school_id)?
        && current != id
    {
        return Err(DbError::DuplicateValue {
            entity: "enrolment",
            field: "school",
        });
    }

    if let Some(number) = input.admission_no.as_deref() {
        let taken: Option<String> = tx
            .query_row(
                "SELECT id FROM student_school_enrolment
                 WHERE school_id = ? AND admission_no = ? AND status <> 'LEFT' AND id <> ?",
                [&text(input.school_id), number, &text(id)],
                |row| row.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(DbError::DuplicateValue {
                entity: "enrolment",
                field: "admission number",
            });
        }
    }

    Ok(())
}

/// Replaces one enrolment's own columns.
fn write_enrolment(
    tx: &Transaction<'_>,
    id: Uuid,
    input: &StudentEnrolmentInput,
) -> Result<(String, i64)> {
    let year_level = opt_text(input.year_level_id);
    let campus = opt_text(input.campus_id);
    update(
        tx,
        "enrolment",
        "student_school_enrolment",
        id,
        ENROLMENT_ASSIGNMENTS,
        &[
            &input.admission_no as &dyn ToSql,
            &year_level,
            &campus,
            &input.status.as_str(),
            &input.enrolled_on,
            &input.left_on,
        ],
    )
}

/// Ends one spell, keeping everything it recorded.
fn end_spell(tx: &Transaction<'_>, id: Uuid, left_on: Option<&str>) -> Result<()> {
    if let Some(date) = left_on {
        let valid: Option<String> = tx.query_row("SELECT date(?)", [date], |row| row.get(0))?;
        if valid.is_none() {
            return Err(DbError::Invalid {
                message: format!("“{date}” is not a date Evara can read. Use YYYY-MM-DD."),
            });
        }
    }
    update(
        tx,
        "enrolment",
        "student_school_enrolment",
        id,
        "status = ?, left_on = ?",
        &[&EnrolmentStatus::Left.as_str() as &dyn ToSql, &left_on],
    )
    .map(|_| ())
    .map_err(|error| match error {
        // The schema's `left_on >= enrolled_on` CHECK is the authority on ordering; it
        // arrives here as a worded refusal rather than as "the project database reported
        // an error".
        DbError::Invalid { .. } => DbError::Invalid {
            message: "A student cannot leave before the date they joined.".to_owned(),
        },
        other => other,
    })
}

// ------------------------------------------------------------- the one placement fact
//
// `V13` made the enrolment row the authority on where a student is placed, and left
// `student.code`, `student.year_level_id` and `student.campus_id` in place as the
// home-school values -- the same treatment `room.campus_id` got in ADR 0014. Two writable
// copies of one fact drift, so these two functions keep them from being independent: an
// edit through either door writes both sides in one transaction, through `repo::update`,
// so each row's `rev` and each change-log entry stay truthful.
//
// Only the HOME school's current spell participates. A spell at any other school must not
// be mirrored, and could not be: `V12`'s trigger holds `student.year_level_id` to belong
// to `student.school_id`, so copying Southmoor's year level onto a Northgate-created
// student would be refused -- rightly, because it would be a claim about the wrong
// institution.
//
// `is_active` and `status` are deliberately NOT mirrored. One is the project-level flag,
// the other the school's own record, exactly as `teacher.is_active` and
// `teacher_school.is_active` are kept apart. ADR 0021 section 6.

/// Copies a spell's placement onto `student`, when that spell is the home-school one.
fn mirror_enrolment_onto_student(tx: &Transaction<'_>, id: Uuid) -> Result<()> {
    type Placement = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    let row: Option<Placement> = tx
        .query_row(
            "SELECT student_id, school_id, admission_no, year_level_id, campus_id
             FROM student_school_enrolment WHERE id = ? AND status <> 'LEFT'",
            [text(id)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;

    let Some((student_raw, school_raw, admission_no, year_level, campus)) = row else {
        return Ok(());
    };
    if student_home_school_text(tx, &student_raw)?.as_deref() != Some(school_raw.as_str()) {
        return Ok(());
    }

    let student_id = parse_id_text("student_school_enrolment.student_id", &student_raw)?;
    update(
        tx,
        "student",
        "student",
        student_id,
        "code = ?, year_level_id = ?, campus_id = ?",
        &[&admission_no as &dyn ToSql, &year_level, &campus],
    )?;
    Ok(())
}

/// Copies `student`'s placement onto its home-school spell, when it has a current one.
///
/// The other door. A student who has transferred away from the school that created them
/// has no current spell there, so this writes nothing and the `student` row is edited
/// alone -- which is correct: those columns are then a record of where they last were.
pub(crate) fn mirror_student_onto_enrolment(tx: &Transaction<'_>, student_id: Uuid) -> Result<()> {
    type Placement = (String, Option<String>, Option<String>, Option<String>);

    let row: Option<Placement> = tx
        .query_row(
            "SELECT school_id, code, year_level_id, campus_id FROM student WHERE id = ?",
            [text(student_id)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;

    let Some((school_raw, code, year_level, campus)) = row else {
        return Ok(());
    };
    let school_id = parse_id_text("student.school_id", &school_raw)?;
    let Some(enrolment) = current_enrolment(tx, student_id, school_id)? else {
        return Ok(());
    };

    update(
        tx,
        "enrolment",
        "student_school_enrolment",
        enrolment,
        "admission_no = ?, year_level_id = ?, campus_id = ?",
        &[&code as &dyn ToSql, &year_level, &campus],
    )
    .map_err(|error| match error {
        // The admission-number index is the only unique rule this write can break, and it
        // breaks it only when another current student at the same school already has that
        // number -- which `student_code_unique` would have refused first. Named anyway, so
        // a future divergence between the two indexes does not read as "name taken".
        DbError::DuplicateValue { .. } => DbError::DuplicateValue {
            entity: "student",
            field: "admission number",
        },
        other => other,
    })?;
    Ok(())
}

/// The school a student was created in, as stored text.
fn student_home_school_text(tx: &Transaction<'_>, student_id: &str) -> Result<Option<String>> {
    tx.query_row(
        "SELECT school_id FROM student WHERE id = ?",
        [student_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(DbError::from)
}

/// Turns an enrolment constraint failure into something the interface can act on.
///
/// `repo::translate` guesses "name" for an unrecognised unique index, which for this table
/// is always wrong: the two it can hit are the one-current-spell rule and the admission
/// number, and neither is a name.
fn enrolment_error(error: rusqlite::Error) -> DbError {
    use rusqlite::ErrorCode;

    let rusqlite::Error::SqliteFailure(failure, ref detail) = error else {
        return translate(error, "enrolment");
    };
    if failure.code != ErrorCode::ConstraintViolation {
        return translate(error, "enrolment");
    }

    let detail = detail.as_deref().unwrap_or_default().to_lowercase();
    if detail.contains("admission_no") {
        DbError::DuplicateValue {
            entity: "enrolment",
            field: "admission number",
        }
    } else if detail.contains("unique") {
        DbError::DuplicateValue {
            entity: "enrolment",
            field: "school",
        }
    } else {
        translate(error, "enrolment")
    }
}
