//! Finding the record a spreadsheet row is talking about.
//!
//! # Why these are here and not repository methods
//!
//! Every read in this file is *resolution policy*, not a general question a screen asks.
//! "Find the teacher this school knows as `T-0007`, case-insensitively, and tell me if more
//! than one answers to it" is a sentence only an import says; adding ten `*_by_code`
//! methods to the repositories would widen their surface for one caller and put the
//! policy — what counts as a match — in the wrong place.
//!
//! # Matching is within the selected school, and by code
//!
//! Two rules, both load-bearing, both from
//! [ADR 0031](../../../../docs/adr/0031-import-identity-matching.md):
//!
//! - **A code matches; a name does not.** A code is something the school chose as an
//!   identifier. A name is not: two children called Grace Hopper is ordinary, and treating
//!   a name match as an identity match would merge them. Where only names are available the
//!   planner reports a *conflict* and asks, rather than guessing either way.
//! - **School-owned entities are matched within the selected school.** Northgate's
//!   `MAT` and Southmoor's `MAT` are two subjects, and resolving across the boundary would
//!   be the multi-school failure every stage since `V12` has been built to prevent.
//!
//! Comparison is case-insensitive and whitespace-trimmed, because `mat`, `MAT` and `MAT `
//! are one school's one code typed three ways — but it is never *fuzzy*. `MATH` does not
//! match `MAT`.

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::repo::curriculum::{SUBJECT_COLUMNS, Subject};
use crate::repo::lessons::{CLASS_SECTION_COLUMNS, COURSE_COLUMNS, ClassSection, Course};
use crate::repo::people::{
    STUDENT_COLUMNS, STUDENT_GROUP_COLUMNS, Student, StudentGroup, TEACHER_COLUMNS, Teacher,
};
use crate::repo::row::text;

/// What a lookup found.
///
/// Three answers rather than two, because "more than one" is not "none" and must not be
/// collapsed into it: the first means *ask the user*, the second means *create*.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Found<T> {
    /// Nothing answers to that key.
    None,
    /// Exactly one does.
    One(T),
    /// Several do. The import stops and asks rather than picking.
    Many(Vec<T>),
}

impl<T> Found<T> {
    /// Collapses a list into an answer.
    fn of(mut rows: Vec<T>) -> Self {
        match rows.len() {
            0 => Self::None,
            1 => Self::One(rows.remove(0)),
            _ => Self::Many(rows),
        }
    }
}

/// Normalises a key for comparison: trimmed, case-folded.
///
/// Applied on both sides of every comparison in this file rather than in SQL, so the rule
/// is one function and cannot differ between a `WHERE` clause and a filter.
pub(super) fn key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Whether two keys are the same identifier typed two ways.
fn same(left: Option<&str>, right: &str) -> bool {
    left.is_some_and(|left| key(left) == key(right))
}

// ============================================================================= people

/// Teachers of one school whose staff code is `code`.
///
/// Scoped to the school because `teacher_code_unique` is `(school_id, code)` and ADR 0021
/// §4 deliberately did not widen it — a code at one school says nothing about the same code
/// at another, and treating it as an identity would merge two people.
///
/// Reads through `teacher_school` rather than `teacher.school_id`, so a teacher another
/// school created and this one employs is found. `teacher.school_id` is the *home* school
/// and nothing reads it to answer "which schools?".
pub(super) fn teachers_by_code(
    conn: &Connection,
    school_id: Uuid,
    code: &str,
) -> Result<Found<Teacher>> {
    let rows = teachers_of_school(conn, school_id)?
        .into_iter()
        .filter(|teacher| same(teacher.code.as_deref(), code))
        .collect();
    Ok(Found::of(rows))
}

/// Teachers of one school with both names matching.
///
/// Used only to *detect ambiguity*, never to match: see this module's header. The planner
/// turns any answer but `None` into a conflict the user resolves.
pub(super) fn teachers_by_name(
    conn: &Connection,
    school_id: Uuid,
    given: &str,
    family: &str,
) -> Result<Found<Teacher>> {
    let rows = teachers_of_school(conn, school_id)?
        .into_iter()
        .filter(|teacher| {
            key(&teacher.given_name) == key(given) && key(&teacher.family_name) == key(family)
        })
        .collect();
    Ok(Found::of(rows))
}

/// Teachers **anywhere in the project** whose code and both names match.
///
/// The one deliberately project-wide lookup, and the narrowest possible form of it: all
/// three of code, given name and family name must agree. It exists for the case the brief
/// is explicit about — *"importing a teacher into School B must not automatically create a
/// second teacher identity if that teacher already exists in the project"* — and it still
/// does not decide anything. A match is offered to the user as "employ the person Northgate
/// already has", because a shared staff code plus a shared name is strong evidence and not
/// proof.
pub(super) fn teachers_elsewhere(
    conn: &Connection,
    school_id: Uuid,
    code: &str,
    given: &str,
    family: &str,
) -> Result<Vec<Teacher>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TEACHER_COLUMNS} FROM teacher
         WHERE id NOT IN (SELECT teacher_id FROM teacher_school WHERE school_id = ?)
         ORDER BY family_name, given_name, id"
    ))?;
    let rows = stmt
        .query_map([text(school_id)], Teacher::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|teacher| {
            same(teacher.code.as_deref(), code)
                && key(&teacher.given_name) == key(given)
                && key(&teacher.family_name) == key(family)
        })
        .collect())
}

/// The schools one teacher is already employed at, by name.
///
/// Only for a message. "A teacher with that code exists at another school" is not an
/// answer a user can act on — deciding whether to employ the same person here means
/// knowing *which* school Evara found them at, and in a trust of six that is the whole
/// of the question. Project-wide on purpose, like `teachers_elsewhere` above.
pub(super) fn schools_of_teacher(conn: &Connection, teacher_id: Uuid) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT s.name FROM teacher_school ts
         JOIN school s ON s.id = ts.school_id
         WHERE ts.teacher_id = ?
         ORDER BY s.name",
    )?;
    let names = stmt
        .query_map([text(teacher_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

/// Every teacher one school employs, active or not.
fn teachers_of_school(conn: &Connection, school_id: Uuid) -> Result<Vec<Teacher>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM teacher_school ts
         JOIN teacher t ON t.id = ts.teacher_id
         WHERE ts.school_id = ?
         ORDER BY t.family_name, t.given_name, t.id",
        qualify(TEACHER_COLUMNS, "t"),
    ))?;
    stmt.query_map([text(school_id)], Teacher::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::from)
}

/// Whether a school's employment of a teacher is active, and whether it exists at all.
pub(super) fn membership_state(
    conn: &Connection,
    school_id: Uuid,
    teacher_id: Uuid,
) -> Result<Option<bool>> {
    conn.query_row(
        "SELECT is_active FROM teacher_school WHERE school_id = ? AND teacher_id = ?",
        [text(school_id), text(teacher_id)],
        |row| row.get(0),
    )
    .optional()
    .map_err(DbError::from)
}

/// Students of one school whose code matches.
///
/// A student's code lives in two places by `V13`'s compatibility policy —
/// `student.code` and `student_school_enrolment.admission_no`, the second being *this
/// school's* number for them. Both are compared, because a school that renumbered on
/// arrival has the number it uses in the second and a shared student may carry another
/// school's in the first.
pub(super) fn students_by_code(
    conn: &Connection,
    school_id: Uuid,
    code: &str,
) -> Result<Found<Student>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {}, e.admission_no FROM student_school_enrolment e
         JOIN student s ON s.id = e.student_id
         WHERE e.school_id = ?
         ORDER BY s.family_name, s.given_name, s.id",
        qualify(STUDENT_COLUMNS, "s"),
    ))?;
    let offset = STUDENT_COLUMNS.split(", ").count();
    let rows = stmt
        .query_map([text(school_id)], |row| {
            Ok((
                Student::from_row_at(row, 0)?,
                row.get::<_, Option<String>>(offset)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut matched: Vec<Student> = Vec::new();
    for (student, admission_no) in rows {
        let answers = same(admission_no.as_deref(), code) || same(student.code.as_deref(), code);
        if answers && !matched.iter().any(|seen| seen.id == student.id) {
            matched.push(student);
        }
    }
    Ok(Found::of(matched))
}

/// Students **anywhere in the project** whose code and both names match, excluding this
/// school.
///
/// The student counterpart of [`teachers_elsewhere`], and the same narrowest-possible form:
/// all three of code, given name and family name must agree. Concurrent enrolment at two
/// schools of a trust is ordinary and `V13` models it as one identity with two spells, so
/// creating a second child record would be the duplication that design exists to prevent —
/// but a shared code is evidence, not proof, so the planner offers and does not decide.
pub(super) fn students_elsewhere(
    conn: &Connection,
    school_id: Uuid,
    code: &str,
    given: &str,
    family: &str,
) -> Result<Vec<Student>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {STUDENT_COLUMNS} FROM student
         WHERE id NOT IN (SELECT student_id FROM student_school_enrolment WHERE school_id = ?)
         ORDER BY family_name, given_name, id"
    ))?;
    let rows = stmt
        .query_map([text(school_id)], Student::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|student| {
            same(student.code.as_deref(), code)
                && key(&student.given_name) == key(given)
                && key(&student.family_name) == key(family)
        })
        .collect())
}

/// Students of one school with both names matching. Ambiguity detection only.
pub(super) fn students_by_name(
    conn: &Connection,
    school_id: Uuid,
    given: &str,
    family: &str,
) -> Result<Found<Student>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM student_school_enrolment e
         JOIN student s ON s.id = e.student_id
         WHERE e.school_id = ?
         GROUP BY s.id
         ORDER BY s.family_name, s.given_name, s.id",
        qualify(STUDENT_COLUMNS, "s"),
    ))?;
    let rows = stmt
        .query_map([text(school_id)], Student::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Found::of(
        rows.into_iter()
            .filter(|student| {
                key(&student.given_name) == key(given) && key(&student.family_name) == key(family)
            })
            .collect(),
    ))
}

/// The student's current spell at one school, if they have one.
pub(super) fn current_spell(
    conn: &Connection,
    school_id: Uuid,
    student_id: Uuid,
) -> Result<Option<(Uuid, String, i64)>> {
    let found: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT id, status, rev FROM student_school_enrolment
             WHERE school_id = ? AND student_id = ? AND status <> 'LEFT'",
            [text(school_id), text(student_id)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    found
        .map(|(id, status, rev)| {
            crate::repo::row::parse_id_text("student_school_enrolment.id", &id)
                .map(|id| (id, status, rev))
        })
        .transpose()
}

// ========================================================================= curriculum

/// A department of one school, by code or by name.
///
/// Code first, because a code is an identifier and a name is a label. A school that codes
/// its departments and renames one keeps working; a school that does not code them is
/// matched on the name, which is what `UNIQUE (school_id, name)` makes safe.
pub(super) fn department_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<(Uuid, String)>> {
    let mut stmt = conn
        .prepare("SELECT id, name, code FROM department WHERE school_id = ? ORDER BY name, id")?;
    let rows = stmt
        .query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    collapse_keyed(rows, needle, "department.id")
}

/// One department as an input plus its revision, for the change comparison.
///
/// Returns the input form rather than the record because that is what the planner compares
/// against, and the revision because the update needs it as a precondition.
pub(super) fn department_row(
    conn: &Connection,
    id: Uuid,
) -> Result<(crate::repo::curriculum::DepartmentInput, i64)> {
    conn.query_row(
        "SELECT school_id, name, code, description, rev FROM department WHERE id = ?",
        [text(id)],
        |row| {
            Ok((
                crate::repo::curriculum::DepartmentInput {
                    school_id: crate::repo::row::FromColumn::read(row, 0)?,
                    name: row.get(1)?,
                    code: row.get(2)?,
                    description: row.get(3)?,
                },
                row.get(4)?,
            ))
        },
    )
    .map_err(DbError::from)
}

/// The subject a class section teaches, through its course.
///
/// Read so a class-teacher row can warn about a missing qualification without the planner
/// knowing how the chain is shaped.
pub(super) fn subject_of_section(
    conn: &Connection,
    class_section_id: Uuid,
) -> Result<Option<Uuid>> {
    let found: Option<String> = conn
        .query_row(
            "SELECT course.subject_id FROM class_section
             JOIN course ON course.id = class_section.course_id
             WHERE class_section.id = ?",
            [text(class_section_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| crate::repo::row::parse_id_text("course.subject_id", value))
        .transpose()
}

/// A subject of one school, by code or by name.
pub(super) fn subject_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<Subject>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SUBJECT_COLUMNS} FROM subject WHERE school_id = ? ORDER BY name, id"
    ))?;
    let rows = stmt
        .query_map([text(school_id)], Subject::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Found::of(
        rows.into_iter()
            .filter(|subject| {
                same(subject.code.as_deref(), needle) || key(&subject.name) == key(needle)
            })
            .collect(),
    ))
}

/// A course of one school, by code or by name.
pub(super) fn course_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<Course>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM course c
         JOIN subject s ON s.id = c.subject_id
         WHERE s.school_id = ?
         ORDER BY c.name, c.id",
        qualify(COURSE_COLUMNS, "c"),
    ))?;
    let rows = stmt
        .query_map([text(school_id)], Course::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Found::of(
        rows.into_iter()
            .filter(|course| {
                same(course.code.as_deref(), needle) || key(&course.name) == key(needle)
            })
            .collect(),
    ))
}

/// A class section of one course, by code or by name.
///
/// Scoped to the course rather than the school, because a section code is unique within its
/// course and `A` is a section of nearly every course a school runs.
pub(super) fn section_by_key(
    conn: &Connection,
    course_id: Uuid,
    needle: &str,
) -> Result<Found<ClassSection>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CLASS_SECTION_COLUMNS} FROM class_section
         WHERE course_id = ? ORDER BY name, id"
    ))?;
    let rows = stmt
        .query_map([text(course_id)], ClassSection::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Found::of(
        rows.into_iter()
            .filter(|section| {
                same(section.code.as_deref(), needle) || key(&section.name) == key(needle)
            })
            .collect(),
    ))
}

// ============================================================== structure and people

/// A year level of one school, by name.
///
/// Name only: a year level has no code column. `UNIQUE (school_id, name)` makes that safe.
pub(super) fn year_level_by_name(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<(Uuid, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, NULL FROM year_level WHERE school_id = ? ORDER BY ordinal, id",
    )?;
    let rows = stmt
        .query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    collapse_keyed(rows, needle, "year_level.id")
}

/// A campus of one school, by code or by name.
pub(super) fn campus_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<(Uuid, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, name, code FROM campus WHERE school_id = ? ORDER BY name, id")?;
    let rows = stmt
        .query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    collapse_keyed(rows, needle, "campus.id")
}

/// A room type of one school, by code or by name.
pub(super) fn room_type_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<(Uuid, String)>> {
    let mut stmt =
        conn.prepare("SELECT id, name, code FROM room_type WHERE school_id = ? ORDER BY name, id")?;
    let rows = stmt
        .query_map([text(school_id)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    collapse_keyed(rows, needle, "room_type.id")
}

/// A student group of one school, by code or by name.
pub(super) fn group_by_key(
    conn: &Connection,
    school_id: Uuid,
    needle: &str,
) -> Result<Found<StudentGroup>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {STUDENT_GROUP_COLUMNS} FROM student_group
         WHERE school_id = ? ORDER BY name, id"
    ))?;
    let rows = stmt
        .query_map([text(school_id)], StudentGroup::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Found::of(
        rows.into_iter()
            .filter(|group| same(group.code.as_deref(), needle) || key(&group.name) == key(needle))
            .collect(),
    ))
}

// ======================================================================= relationships

/// Whether a relationship row already exists, and its revision if it does.
///
/// One function for the three relationship tables, because the question is identical and
/// the only difference is which two columns name the ends. The table and column names are
/// `&'static str` from this crate, never caller input, and still go through the identifier
/// check before interpolation.
pub(super) fn relationship(
    conn: &Connection,
    table: &'static str,
    left_column: &'static str,
    left: Uuid,
    right_column: &'static str,
    right: Uuid,
) -> Result<Option<i64>> {
    let table = crate::repo::identifier(table)?;
    let left_column = crate::repo::identifier(left_column)?;
    let right_column = crate::repo::identifier(right_column)?;
    conn.query_row(
        &format!("SELECT rev FROM {table} WHERE {left_column} = ? AND {right_column} = ?"),
        [text(left), text(right)],
        |row| row.get(0),
    )
    .optional()
    .map_err(DbError::from)
}

/// How many occurrences a class section already has of one shape.
///
/// `(duration, label)` is the shape, and the count is the multiplicity. An activity has no
/// natural key — four identical single periods are four indistinguishable rows — so
/// idempotency is a question about *how many* rather than *which*, and
/// [ADR 0031](../../../../docs/adr/0031-import-identity-matching.md) §5 argues it.
pub(super) fn occurrences_of_shape(
    conn: &Connection,
    class_section_id: Uuid,
    duration_periods: i64,
    label: Option<&str>,
) -> Result<i64> {
    let mut stmt = conn
        .prepare("SELECT name FROM activity WHERE class_section_id = ? AND duration_periods = ?")?;
    let names = stmt
        .query_map(
            rusqlite::params![text(class_section_id), duration_periods],
            |row| row.get::<_, Option<String>>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let wanted = label.map(key);
    Ok(names
        .into_iter()
        .filter(|name| name.as_deref().map(key) == wanted)
        .count()
        .try_into()
        .unwrap_or(i64::MAX))
}

/// Whether a teacher holds a qualification for one subject.
///
/// Read so the planner can *warn*. ADR 0028 is explicit that a missing qualification does
/// not refuse an assignment, and an import must not be stricter than the screen.
pub(super) fn is_qualified(conn: &Connection, teacher_id: Uuid, subject_id: Uuid) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM teacher_qualification WHERE teacher_id = ? AND subject_id = ?",
        [text(teacher_id), text(subject_id)],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Whether a teacher is eligible at a class section's campus.
///
/// Reads the narrowing exactly as `V15`'s trigger and `Lessons::assign_class_teacher` do:
/// no eligibility recorded at this school means *not narrowed*, which is the state every
/// teacher is in until somebody says otherwise.
pub(super) fn campus_eligible(
    conn: &Connection,
    class_section_id: Uuid,
    teacher_id: Uuid,
) -> Result<bool> {
    conn.query_row(
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
    )
    .map_err(DbError::from)
}

// ============================================================================ helpers

/// Collapses `(id, name, code)` rows into an answer keyed by code or name.
fn collapse_keyed(
    rows: Vec<(String, String, Option<String>)>,
    needle: &str,
    column: &'static str,
) -> Result<Found<(Uuid, String)>> {
    let mut matched = Vec::new();
    for (id, name, code) in rows {
        if same(code.as_deref(), needle) || key(&name) == key(needle) {
            matched.push((crate::repo::row::parse_id_text(column, &id)?, name));
        }
    }
    Ok(Found::of(matched))
}

/// Prefixes every column in a `SELECT` list with a table alias.
///
/// Same three lines as `curriculum`'s and `lessons`', kept local for the same reason they
/// are: a cross-module `pub(crate)` for it would be a worse coupling than the duplication.
fn qualify(columns: &str, alias: &str) -> String {
    columns
        .split(", ")
        .map(|column| format!("{alias}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}
