//! What each kind of row means, and what to do about it.
//!
//! One function per import kind, all reached through [`plan_row`]. Each one does the same
//! four things in the same order, and the order matters:
//!
//! 1. **read** the mapped cells, recording a finding per bad cell rather than stopping at
//!    the first;
//! 2. **claim** the row's key, so a duplicate *within the file* is its own answer;
//! 3. **resolve** every reference within the selected school, so a missing one is an error
//!    about the reference rather than a foreign-key failure later;
//! 4. **match** against what the project holds, and propose.
//!
//! # Unmapped columns are left alone
//!
//! The quietest and most important rule here. A proposed update is built by taking the
//! record Evara holds and overlaying **only the fields the file actually mapped**. A roster
//! of names and codes therefore does not propose blanking everybody's email address, and a
//! school can import the two columns it has without losing the eight it does not.
//!
//! The consequence is that `Unchanged` means "the mapped fields already say this", which is
//! exactly what makes a second run of the same file a no-op.
//!
//! # A code matches; a name asks
//!
//! [ADR 0031](../../../../docs/adr/0031-import-identity-matching.md). Where a row carries a
//! code, it is resolved within the school and the outcome is a create or an update. Where it
//! carries only names and something similar exists, the outcome is a **conflict**: two
//! children called Grace Hopper is ordinary, and a name match is not evidence of identity.

use evara_core::import::{Choice, ImportKind, PlannedRow, RowAction, RowFinding};
use rusqlite::Connection;
use uuid::Uuid;

use super::lookup::{self, Found};
use super::{Built, Intent, Request, Row, SeenKeys};
use crate::error::Result;
use crate::repo::curriculum::{DepartmentInput, SubjectInput};
use crate::repo::lessons::{ClassSectionInput, Course, CourseInput};
use crate::repo::membership::{EnrolmentStatus, StudentEnrolmentInput};
use crate::repo::people::{StudentInput, TeacherInput};

/// Plans one row of whatever kind is being imported.
pub(super) fn plan_row(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    match request.kind {
        ImportKind::Teachers => teachers(conn, request, row, seen),
        ImportKind::Students => students(conn, request, row, seen),
        ImportKind::Departments => departments(conn, request, row, seen),
        ImportKind::Subjects => subjects(conn, request, row, seen),
        ImportKind::Courses => courses(conn, request, row, seen),
        ImportKind::ClassSections => class_sections(conn, request, row, seen),
        ImportKind::ClassTeachers => class_teachers(conn, request, row, seen),
        ImportKind::ClassGroups => class_groups(conn, request, row, seen),
        ImportKind::ClassEnrolments => class_enrolments(conn, request, row, seen),
        ImportKind::Activities => activities(conn, request, row, seen),
    }
}

/// Assembles the answer for one row.
fn finish(
    row: &mut Row<'_>,
    action: RowAction,
    source_label: String,
    target_label: Option<String>,
    intent: Option<Intent>,
) -> Built {
    let line = row.line;
    let findings = row.take_findings();
    // A row that would write nothing carries no intent, however it got there. Belt and
    // braces: the executor only runs what is here, so an `Unchanged` row that somehow
    // carried one would be a silent write.
    let intent = if action.writes() { intent } else { None };
    Built {
        row: PlannedRow {
            line,
            action,
            source_label,
            target_label,
            findings,
        },
        intent,
    }
}

/// The answer for a row that cannot be acted on.
fn invalid(row: &mut Row<'_>, label: String) -> Built {
    finish(row, RowAction::Invalid, label, None, None)
}

/// Records an in-file duplicate, which is its own kind of mistake.
///
/// Two rows claiming the same key is a mistake in the *file*. Applying both would make the
/// second silently overwrite the first, and the user would see one record where they wrote
/// two rows.
fn claim(row: &mut Row<'_>, seen: &mut SeenKeys, key: &str) -> bool {
    if let Some(earlier) = seen.claim(&lookup::key(key), row.line) {
        row.note(RowFinding::row_error(format!(
            "row {earlier} of this file already uses “{key}”. Two rows cannot describe the \
             same record — remove one, or give them different codes."
        )));
        return false;
    }
    true
}

/// Applies a decision to a conflict, or leaves it unresolved.
///
/// `allow_create_new` says whether "this is a different one" is a sensible answer. It is for
/// an *identity* conflict — two people with one name — and it is not for a code clash,
/// where creating a second record under the same code is not an answer to anything.
fn decide(row: &mut Row<'_>, allow_create_new: bool, ask: &str) -> Option<Choice> {
    match row.decision() {
        None => {
            row.note(RowFinding::row_error(ask.to_owned()));
            None
        }
        Some(Choice::CreateNew) if !allow_create_new => {
            row.note(RowFinding::row_error(
                "“Create a new one” is not an answer here: the code would collide with the \
                 record that already uses it. Update it, keep it, or skip the row."
                    .to_owned(),
            ));
            None
        }
        Some(choice) => Some(choice),
    }
}

// =========================================================================== teachers

/// Teachers: a project-wide identity, and this school's employment of it.
///
/// Split into the read, then one function per thing the code could have matched, because
/// the four outcomes are genuinely different decisions and reading them as one function was
/// reading four.
fn teachers(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let read = TeacherRow {
        code: row.code("code"),
        given: row.required_text("givenName"),
        family: row.required_text("familyName"),
        email: row.text("email"),
        email_mapped: row.cell_mapped("email"),
        active: row.flag("isActive"),
        department_keys: row.code_list("departmentCodes"),
        campus_keys: row.code_list("campusCodes"),
    };

    let label = describe_person(
        read.code.as_deref(),
        read.given.as_deref(),
        read.family.as_deref(),
    );
    let (Some(given), Some(family)) = (read.given.clone(), read.family.clone()) else {
        return Ok(invalid(row, label));
    };
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    if let Some(code) = read.code.as_deref()
        && !claim(row, seen, code)
    {
        return Ok(invalid(row, label));
    }

    // References first, so a missing department is an error about the department rather
    // than a foreign-key failure three steps later.
    let narrow = Pending {
        departments: resolve_all(
            conn,
            row,
            school_id,
            &read.department_keys,
            Reference::Department,
        )?,
        campuses: resolve_all(conn, row, school_id, &read.campus_keys, Reference::Campus)?,
    };
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    let here = match read.code.as_deref() {
        Some(code) => lookup::teachers_by_code(conn, school_id, code)?,
        None => Found::None,
    };
    match here {
        Found::Many(_) => {
            row.note(RowFinding::row_error(
                "more than one teacher at this school already uses that code, which should \
                 not be possible. Open the Teachers screen and correct it before importing."
                    .to_owned(),
            ));
            Ok(invalid(row, label))
        }
        Found::One(teacher) => {
            teacher_already_here(conn, row, school_id, &read, &narrow, &teacher, label)
        }
        Found::None => teacher_not_here(conn, request, row, &read, narrow, given, family, label),
    }
}

/// Every mapped cell of a teacher row.
struct TeacherRow {
    code: Option<String>,
    given: Option<String>,
    family: Option<String>,
    email: Option<String>,
    email_mapped: bool,
    active: Option<bool>,
    department_keys: Vec<String>,
    campus_keys: Vec<String>,
}

/// A teacher this school already employs under that code.
fn teacher_already_here(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    read: &TeacherRow,
    narrow: &Pending,
    teacher: &crate::repo::people::Teacher,
    label: String,
) -> Result<Built> {
    let proposed = overlay_teacher(
        school_id,
        Some(teacher),
        read.code.clone(),
        read.given.clone(),
        read.family.clone(),
        read.email.clone(),
        read.email_mapped,
    );
    let target = Some(describe_person(
        teacher.code.as_deref(),
        Some(&teacher.given_name),
        Some(&teacher.family_name),
    ));
    note_membership_drift(conn, row, school_id, teacher.id, read.active)?;
    let pending = pending_narrowings(conn, teacher.id, &narrow.campuses, &narrow.departments)?;

    if proposed == teacher_input_of(teacher) && pending.is_empty_pair() {
        return Ok(finish(row, RowAction::Unchanged, label, target, None));
    }
    let intent = Intent::UpdateTeacher {
        id: teacher.id,
        rev: teacher.rev,
        input: proposed,
        campuses: pending.campuses,
        departments: pending.departments,
    };
    Ok(settle_update(
        row,
        label,
        target,
        intent,
        "this teacher already exists here and the file disagrees with what Evara holds. \
         Choose whether to update the record or leave it as it is.",
    ))
}

/// Nobody here answers to that code — so either the project already holds the person, or a
/// same-named colleague makes the row ambiguous, or it is somebody new.
#[expect(
    clippy::too_many_arguments,
    reason = "the row's values and its two resolved reference lists; grouping them would \
              only move the list"
)]
fn teacher_not_here(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    read: &TeacherRow,
    narrow: Pending,
    given: String,
    family: String,
    label: String,
) -> Result<Built> {
    let school_id = request.school_id;

    // Before creating a second identity for somebody the project may already hold, look
    // for the narrowest possible match: same code *and* both names, at another school.
    if let Some(code) = read.code.as_deref() {
        let elsewhere = lookup::teachers_elsewhere(conn, school_id, code, &given, &family)?;
        if let Some(found) = elsewhere.first() {
            return teacher_shared(
                conn, row, read, narrow, found, label, given, family, school_id,
            );
        }
    }

    // No code at all. A name is not an identifier, so a same-name teacher is a question
    // rather than a match.
    if read.code.is_none() {
        match lookup::teachers_by_name(conn, school_id, &given, &family)? {
            Found::None => {
                row.note(RowFinding::warning(
                    "this row has no staff code, so importing the same file again will \
                     create a second record rather than recognising this one. A Teacher \
                     Code column is what makes an import repeatable."
                        .to_owned(),
                ));
            }
            Found::One(teacher) => {
                return teacher_same_name(conn, row, read, narrow, &teacher, label, school_id);
            }
            Found::Many(found) => {
                row.note(RowFinding::row_error(format!(
                    "{} teachers at this school already have this name, and the row has no \
                     staff code to say which. Add a Teacher Code column — Evara cannot tell \
                     them apart without one.",
                    found.len()
                )));
                return Ok(invalid(row, label));
            }
        }
    }

    Ok(finish(
        row,
        RowAction::Create,
        label,
        None,
        Some(Intent::CreateTeacher {
            input: overlay_teacher(
                school_id,
                None,
                read.code.clone(),
                Some(given),
                Some(family),
                read.email.clone(),
                true,
            ),
            campuses: narrow.campuses,
            departments: narrow.departments,
        }),
    ))
}

/// The project holds somebody with this code and this name, at another school.
#[expect(
    clippy::too_many_arguments,
    reason = "as `teacher_not_here`: the row's values plus the match it is about"
)]
fn teacher_shared(
    conn: &Connection,
    row: &mut Row<'_>,
    read: &TeacherRow,
    narrow: Pending,
    found: &crate::repo::people::Teacher,
    label: String,
    given: String,
    family: String,
    school_id: Uuid,
) -> Result<Built> {
    let target = Some(format!(
        "{} {} — already in this campus file",
        found.given_name, found.family_name
    ));
    // Named, because "at another school" is not something a user can act on and the
    // decision they are being asked for turns on which school it is.
    let elsewhere = lookup::schools_of_teacher(conn, found.id)?;
    let where_found = if elsewhere.is_empty() {
        "another school".to_owned()
    } else {
        elsewhere.join(", ")
    };
    let Some(choice) = decide(
        row,
        true,
        &format!(
            "this campus file already holds a teacher with that staff code and that name, \
             at {where_found}. Employ the same person here, or create a separate identity \
             — Evara will not decide which, because a shared code is strong evidence and \
             not proof."
        ),
    ) else {
        return Ok(finish(row, RowAction::Conflict, label, target, None));
    };
    Ok(match choice {
        Choice::Update => finish(
            row,
            RowAction::Update,
            label,
            target,
            Some(Intent::EmployTeacher {
                teacher_id: found.id,
                is_active: read.active.unwrap_or(true),
                campuses: narrow.campuses,
                departments: narrow.departments,
            }),
        ),
        Choice::CreateNew => finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateTeacher {
                input: overlay_teacher(
                    school_id,
                    None,
                    read.code.clone(),
                    Some(given),
                    Some(family),
                    read.email.clone(),
                    true,
                ),
                campuses: narrow.campuses,
                departments: narrow.departments,
            }),
        ),
        Choice::KeepExisting | Choice::Skip => finish(row, RowAction::Skipped, label, target, None),
    })
}

/// A teacher here has this name, and the row has no code to tell them apart.
fn teacher_same_name(
    conn: &Connection,
    row: &mut Row<'_>,
    read: &TeacherRow,
    narrow: Pending,
    teacher: &crate::repo::people::Teacher,
    label: String,
    school_id: Uuid,
) -> Result<Built> {
    let target = Some(format!(
        "{} {} — already at this school",
        teacher.given_name, teacher.family_name
    ));
    let Some(choice) = decide(
        row,
        true,
        "a teacher with this name is already at this school, and the row has no staff code \
         to tell them apart. Say whether this is the same person or a different one — two \
         teachers with one name is ordinary, so Evara will not assume.",
    ) else {
        return Ok(finish(row, RowAction::Conflict, label, target, None));
    };
    Ok(match choice {
        Choice::Update => {
            let proposed = overlay_teacher(
                school_id,
                Some(teacher),
                None,
                read.given.clone(),
                read.family.clone(),
                read.email.clone(),
                read.email_mapped,
            );
            let pending =
                pending_narrowings(conn, teacher.id, &narrow.campuses, &narrow.departments)?;
            finish(
                row,
                RowAction::Update,
                label,
                target,
                Some(Intent::UpdateTeacher {
                    id: teacher.id,
                    rev: teacher.rev,
                    input: proposed,
                    campuses: pending.campuses,
                    departments: pending.departments,
                }),
            )
        }
        Choice::CreateNew => finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateTeacher {
                input: overlay_teacher(
                    school_id,
                    None,
                    None,
                    read.given.clone(),
                    read.family.clone(),
                    read.email.clone(),
                    true,
                ),
                campuses: narrow.campuses,
                departments: narrow.departments,
            }),
        ),
        Choice::KeepExisting | Choice::Skip => finish(row, RowAction::Skipped, label, target, None),
    })
}

/// Warns when the file's employment flag disagrees with the membership Evara holds.
///
/// Reported rather than applied, deliberately. Withdrawing somebody's employment is a
/// decision the Teachers screen owns and has a confirmation for; a roster import silently
/// marking four people as no longer teaching — because a column the school barely looked at
/// said `No` — is exactly the "silently modifying existing records" this stage must not do.
/// ADR 0030 §7 records it as a known limitation rather than a gap.
fn note_membership_drift(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    teacher_id: Uuid,
    wanted: Option<bool>,
) -> Result<()> {
    let Some(wanted) = wanted else { return Ok(()) };
    let Some(current) = lookup::membership_state(conn, school_id, teacher_id)? else {
        return Ok(());
    };
    if current != wanted {
        row.note(RowFinding::warning(format!(
            "the file says this teacher is {} at this school and Evara has them as {}. The \
             import leaves employment alone — change it on the Teachers screen, where it \
             asks first.",
            if wanted { "active" } else { "not active" },
            if current { "active" } else { "not active" },
        )));
    }
    Ok(())
}

/// A teacher's campus and department references.
struct Pending {
    campuses: Vec<Uuid>,
    departments: Vec<Uuid>,
}

impl Pending {
    const fn is_empty_pair(&self) -> bool {
        self.campuses.is_empty() && self.departments.is_empty()
    }
}

/// Filters a row's campus and department references down to the ones that are new.
///
/// An association the teacher already holds is not a change, so a second run of the same
/// file proposes nothing — which is what keeps `Unchanged` honest for a teacher row that
/// carries them.
fn pending_narrowings(
    conn: &Connection,
    teacher_id: Uuid,
    campuses: &[Uuid],
    departments: &[Uuid],
) -> Result<Pending> {
    let mut wanted_campuses = Vec::new();
    for campus_id in campuses {
        if lookup::relationship(
            conn,
            "teacher_campus",
            "teacher_id",
            teacher_id,
            "campus_id",
            *campus_id,
        )?
        .is_none()
        {
            wanted_campuses.push(*campus_id);
        }
    }
    let mut wanted_departments = Vec::new();
    for department_id in departments {
        if lookup::relationship(
            conn,
            "teacher_department",
            "teacher_id",
            teacher_id,
            "department_id",
            *department_id,
        )?
        .is_none()
        {
            wanted_departments.push(*department_id);
        }
    }
    Ok(Pending {
        campuses: wanted_campuses,
        departments: wanted_departments,
    })
}

/// Settles an update that needs the user's agreement.
fn settle_update(
    row: &mut Row<'_>,
    label: String,
    target: Option<String>,
    intent: Intent,
    ask: &str,
) -> Built {
    match row.decision() {
        Some(Choice::Update) => finish(row, RowAction::Update, label, target, Some(intent)),
        Some(Choice::KeepExisting | Choice::Skip) => {
            finish(row, RowAction::Skipped, label, target, None)
        }
        Some(Choice::CreateNew) => {
            row.note(RowFinding::row_error(
                "“Create a new one” is not an answer here: the code would collide with the \
                 record that already uses it. Update it, keep it, or skip the row."
                    .to_owned(),
            ));
            finish(row, RowAction::Invalid, label, target, None)
        }
        None => {
            row.note(RowFinding::row_error(ask.to_owned()));
            finish(row, RowAction::Conflict, label, target, None)
        }
    }
}

/// Builds a teacher input from what exists, overlaid with what the file mapped.
///
/// Unmapped fields keep the record's current value — see this module's header. `email_mapped`
/// is passed separately because a *mapped and blank* email means "clear it" while an
/// *unmapped* one means "leave it".
fn overlay_teacher(
    school_id: Uuid,
    existing: Option<&crate::repo::people::Teacher>,
    code: Option<String>,
    given: Option<String>,
    family: Option<String>,
    email: Option<String>,
    email_mapped: bool,
) -> TeacherInput {
    let mut input = match existing {
        Some(teacher) => teacher_input_of(teacher),
        None => TeacherInput {
            school_id,
            code: None,
            given_name: String::new(),
            family_name: String::new(),
            email: None,
            home_campus_id: None,
            fte_percent: 100,
            max_periods_per_day: None,
            max_periods_per_cycle: None,
            max_consecutive_periods: None,
            notes: None,
            is_active: true,
        },
    };
    input.school_id = existing.map_or(school_id, |teacher| teacher.school_id);
    if let Some(code) = code {
        input.code = Some(code);
    }
    if let Some(given) = given {
        input.given_name = given;
    }
    if let Some(family) = family {
        input.family_name = family;
    }
    if email_mapped {
        input.email = email;
    }
    input
}

/// The input form of a stored teacher, for comparison.
fn teacher_input_of(teacher: &crate::repo::people::Teacher) -> TeacherInput {
    TeacherInput {
        school_id: teacher.school_id,
        code: teacher.code.clone(),
        given_name: teacher.given_name.clone(),
        family_name: teacher.family_name.clone(),
        email: teacher.email.clone(),
        home_campus_id: teacher.home_campus_id,
        fte_percent: teacher.fte_percent,
        max_periods_per_day: teacher.max_periods_per_day,
        max_periods_per_cycle: teacher.max_periods_per_cycle,
        max_consecutive_periods: teacher.max_consecutive_periods,
        notes: teacher.notes.clone(),
        is_active: teacher.is_active,
    }
}

// =========================================================================== students

/// Students: a project-wide identity, and this school's enrolment spell.
fn students(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let read = StudentRow {
        code: row.code("code"),
        given: row.required_text("givenName"),
        family: row.required_text("familyName"),
        email: row.text("email"),
        email_mapped: row.cell_mapped("email"),
        year_mapped: row.cell_mapped("yearLevel"),
        campus_mapped: row.cell_mapped("campusCode"),
        year_level_id: None,
        campus_id: None,
        status: EnrolmentStatus::Enrolled,
        enrolled_on: row.date("enrolledOn"),
    };
    let year_key = row.code("yearLevel");
    let campus_key = row.code("campusCode");
    let status_text = row.text("status");

    let label = describe_person(
        read.code.as_deref(),
        read.given.as_deref(),
        read.family.as_deref(),
    );
    let (Some(given), Some(family)) = (read.given.clone(), read.family.clone()) else {
        return Ok(invalid(row, label));
    };
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    if let Some(code) = read.code.as_deref()
        && !claim(row, seen, code)
    {
        return Ok(invalid(row, label));
    }

    let Some(status) = read_status(row, status_text.as_deref()) else {
        return Ok(invalid(row, label));
    };
    let mut read = read;
    read.status = status;
    read.year_level_id = resolve_one(
        conn,
        row,
        school_id,
        year_key.as_deref(),
        Reference::YearLevel,
    )?;
    read.campus_id = resolve_one(
        conn,
        row,
        school_id,
        campus_key.as_deref(),
        Reference::Campus,
    )?;
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    let here = match read.code.as_deref() {
        Some(code) => lookup::students_by_code(conn, school_id, code)?,
        None => Found::None,
    };
    match here {
        Found::Many(found) => {
            row.note(RowFinding::row_error(format!(
                "{} students at this school already use that code. Correct the duplicate on \
                 the Students screen before importing.",
                found.len()
            )));
            Ok(invalid(row, label))
        }
        Found::One(student) => student_already_here(conn, row, school_id, &read, &student, label),
        Found::None => student_not_here(conn, row, school_id, &read, given, family, label),
    }
}

/// Every mapped cell of a student row, plus the references it resolved to.
struct StudentRow {
    code: Option<String>,
    given: Option<String>,
    family: Option<String>,
    email: Option<String>,
    email_mapped: bool,
    year_mapped: bool,
    campus_mapped: bool,
    year_level_id: Option<Uuid>,
    campus_id: Option<Uuid>,
    status: EnrolmentStatus,
    enrolled_on: Option<String>,
}

/// Reads the enrolment status, defaulting and reporting a word nobody recognises.
fn read_status(row: &mut Row<'_>, text: Option<&str>) -> Option<EnrolmentStatus> {
    let Some(text) = text else {
        // A blank status column means the ordinary case, not a missing answer.
        return Some(EnrolmentStatus::Enrolled);
    };
    if let Some(status) = parse_status(text) {
        return Some(status);
    }
    row.note(RowFinding::error(
        "status",
        format!(
            "“{text}” is not an enrolment status. Use Enrolled, Prospective or Left, \
             or leave it blank for Enrolled."
        ),
    ));
    None
}

/// A student this school already has under that code.
fn student_already_here(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    read: &StudentRow,
    student: &crate::repo::people::Student,
    label: String,
) -> Result<Built> {
    let proposed = overlay_student(school_id, Some(student), read);
    let target = Some(describe_person(
        student.code.as_deref(),
        Some(&student.given_name),
        Some(&student.family_name),
    ));
    note_spell_drift(conn, row, school_id, student.id, read.status)?;

    if proposed == student_input_of(student) {
        return Ok(finish(row, RowAction::Unchanged, label, target, None));
    }
    let intent = Intent::UpdateStudent {
        id: student.id,
        rev: student.rev,
        input: proposed,
    };
    Ok(settle_update(
        row,
        label,
        target,
        intent,
        "this student already exists here and the file disagrees with what Evara holds. \
         Choose whether to update the record or leave it as it is.",
    ))
}

/// This school has no student with that code.
fn student_not_here(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    read: &StudentRow,
    given: String,
    family: String,
    label: String,
) -> Result<Built> {
    // Concurrent enrolment is ordinary in a trust, so before creating a second identity
    // look for the same child elsewhere in the project — by code *and* both names, which
    // is the narrowest evidence there is. Still asked rather than assumed.
    if let Some(code) = read.code.as_deref() {
        let elsewhere = lookup::students_elsewhere(conn, school_id, code, &given, &family)?;
        if let Some(found) = elsewhere.first() {
            let target = Some(format!(
                "{} {} — already in this campus file",
                found.given_name, found.family_name
            ));
            let Some(choice) = decide(
                row,
                true,
                "this campus file already holds a student with that code and that name, at \
                 another school. Enrol the same child here — which keeps one identity and \
                 both schools' spells — or create a separate record.",
            ) else {
                return Ok(finish(row, RowAction::Conflict, label, target, None));
            };
            return Ok(match choice {
                Choice::Update => finish(
                    row,
                    RowAction::Update,
                    label,
                    target,
                    Some(Intent::EnrolStudent {
                        spell: spell_for(found.id, school_id, read),
                    }),
                ),
                Choice::CreateNew => student_create(row, school_id, read, given, family, label),
                Choice::KeepExisting | Choice::Skip => {
                    finish(row, RowAction::Skipped, label, target, None)
                }
            });
        }
    }

    if read.code.is_none() {
        match lookup::students_by_name(conn, school_id, &given, &family)? {
            Found::None => {
                row.note(RowFinding::warning(
                    "this row has no student code, so importing the same file again will \
                     enrol a second child rather than recognising this one. A Student Code \
                     column is what makes an import repeatable."
                        .to_owned(),
                ));
            }
            Found::One(_) | Found::Many(_) => {
                row.note(RowFinding::row_error(
                    "a student with this name is already at this school and the row has no \
                     code to tell them apart. Two children with one name is ordinary, so \
                     Evara will not assume — add a Student Code column."
                        .to_owned(),
                ));
                return Ok(invalid(row, label));
            }
        }
    }
    Ok(student_create(row, school_id, read, given, family, label))
}

/// A student nothing in the project matches.
fn student_create(
    row: &mut Row<'_>,
    school_id: Uuid,
    read: &StudentRow,
    given: String,
    family: String,
    label: String,
) -> Built {
    let mut input = overlay_student(school_id, None, read);
    input.given_name = given;
    input.family_name = family;
    finish(
        row,
        RowAction::Create,
        label,
        None,
        Some(Intent::CreateStudent {
            input,
            // The executor replaces this with the identity it just created.
            spell: spell_for(Uuid::nil(), school_id, read),
        }),
    )
}

/// The enrolment spell a student row describes.
fn spell_for(student_id: Uuid, school_id: Uuid, read: &StudentRow) -> StudentEnrolmentInput {
    StudentEnrolmentInput {
        student_id,
        school_id,
        admission_no: read.code.clone(),
        year_level_id: read.year_level_id,
        campus_id: read.campus_id,
        status: read.status,
        enrolled_on: read.enrolled_on.clone(),
        left_on: None,
    }
}

/// Warns when the file's enrolment status disagrees with the spell Evara holds.
///
/// Not applied, and this one is more than caution. Changing a spell's status *is* leaving or
/// re-enrolling, which ADR 0021 §7 deliberately made one transactional operation with its
/// own history semantics — `leave_school` writes an end date, a return writes a new spell,
/// and a `LEFT` row is never rewritten. An import that set `status` directly would bypass
/// all of it and quietly destroy the history the model exists to keep.
fn note_spell_drift(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    student_id: Uuid,
    wanted: EnrolmentStatus,
) -> Result<()> {
    let Some((_, current, _)) = lookup::current_spell(conn, school_id, student_id)? else {
        row.note(RowFinding::warning(
            "this student has no current enrolment at this school — they have left. The \
             import leaves enrolment history alone; re-enrol them on the Students screen, \
             which keeps the previous spell."
                .to_owned(),
        ));
        return Ok(());
    };
    if current != wanted.stored_value() {
        row.note(RowFinding::warning(format!(
            "the file says this student is “{}” and Evara has them as “{}”. The import \
             leaves enrolment alone — leaving and re-enrolling keep a history that a \
             spreadsheet column cannot express, so do it on the Students screen.",
            wanted.stored_value().to_lowercase(),
            current.to_lowercase(),
        )));
    }
    Ok(())
}

/// Reads an enrolment status from a cell.
fn parse_status(value: &str) -> Option<EnrolmentStatus> {
    match value.trim().to_lowercase().as_str() {
        "enrolled" | "enroled" | "active" | "current" => Some(EnrolmentStatus::Enrolled),
        "prospective" | "applicant" | "pending" => Some(EnrolmentStatus::Prospective),
        "left" | "leaver" | "inactive" | "former" => Some(EnrolmentStatus::Left),
        _ => None,
    }
}

/// Builds a student input from what exists, overlaid with what the file mapped.
fn overlay_student(
    school_id: Uuid,
    existing: Option<&crate::repo::people::Student>,
    read: &StudentRow,
) -> StudentInput {
    let mut input = match existing {
        Some(student) => student_input_of(student),
        None => StudentInput {
            school_id,
            code: None,
            given_name: String::new(),
            family_name: String::new(),
            email: None,
            year_level_id: None,
            campus_id: None,
            is_active: true,
        },
    };
    input.school_id = existing.map_or(school_id, |student| student.school_id);
    if let Some(code) = read.code.clone() {
        input.code = Some(code);
    }
    if let Some(given) = read.given.clone() {
        input.given_name = given;
    }
    if let Some(family) = read.family.clone() {
        input.family_name = family;
    }
    if read.email_mapped {
        input.email.clone_from(&read.email);
    }
    if read.year_mapped || existing.is_none() {
        input.year_level_id = read.year_level_id;
    }
    if read.campus_mapped || existing.is_none() {
        input.campus_id = read.campus_id;
    }
    input
}

/// The input form of a stored student, for comparison.
fn student_input_of(student: &crate::repo::people::Student) -> StudentInput {
    StudentInput {
        school_id: student.school_id,
        code: student.code.clone(),
        given_name: student.given_name.clone(),
        family_name: student.family_name.clone(),
        email: student.email.clone(),
        year_level_id: student.year_level_id,
        campus_id: student.campus_id,
        is_active: student.is_active,
    }
}

// ========================================================================= curriculum

/// Departments of one school.
fn departments(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let code = row.code("code");
    let name = row.required_text("name");
    let description = row.text("description");
    let description_mapped = row.cell_mapped("description");

    let label = name.clone().unwrap_or_else(|| "(no name)".to_owned());
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    let Some(name) = name else {
        return Ok(invalid(row, label));
    };
    let claim_key = code.clone().unwrap_or_else(|| name.clone());
    if !claim(row, seen, &claim_key) {
        return Ok(invalid(row, label));
    }

    let found = lookup::department_by_key(conn, school_id, &claim_key)?;
    match found {
        Found::Many(_) => Ok(ambiguous(row, label, "department")),
        Found::None => Ok(finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateDepartment {
                input: DepartmentInput {
                    school_id,
                    name,
                    code,
                    description,
                },
            }),
        )),
        Found::One((id, existing_name)) => {
            let (current, rev) = lookup::department_row(conn, id)?;
            let proposed = DepartmentInput {
                school_id,
                name,
                code: code.or_else(|| current.code.clone()),
                description: if description_mapped {
                    description
                } else {
                    current.description.clone()
                },
            };
            Ok(unchanged_or_update(
                row,
                label,
                Some(existing_name),
                proposed == current,
                Intent::UpdateDepartment {
                    id,
                    rev,
                    input: proposed,
                },
                "a department with that code or name already exists here and the file \
                 disagrees with it.",
            ))
        }
    }
}

/// Subjects of one school.
fn subjects(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let code = row.code("code");
    let name = row.required_text("name");
    let description = row.text("description");
    let description_mapped = row.cell_mapped("description");
    let department_key = row.code("departmentCode");
    let department_mapped = row.cell_mapped("departmentCode");

    let label = name.clone().unwrap_or_else(|| "(no name)".to_owned());
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    let Some(name) = name else {
        return Ok(invalid(row, label));
    };
    let claim_key = code.clone().unwrap_or_else(|| name.clone());
    if !claim(row, seen, &claim_key) {
        return Ok(invalid(row, label));
    }

    let department_id = resolve_one(
        conn,
        row,
        school_id,
        department_key.as_deref(),
        Reference::Department,
    )?;
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    match lookup::subject_by_key(conn, school_id, &claim_key)? {
        Found::Many(_) => Ok(ambiguous(row, label, "subject")),
        Found::None => Ok(finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateSubject {
                input: SubjectInput {
                    school_id,
                    department_id,
                    name,
                    code,
                    description,
                },
            }),
        )),
        Found::One(subject) => {
            let current = SubjectInput {
                school_id: subject.school_id,
                department_id: subject.department_id,
                name: subject.name.clone(),
                code: subject.code.clone(),
                description: subject.description.clone(),
            };
            let proposed = SubjectInput {
                school_id,
                department_id: if department_mapped {
                    department_id
                } else {
                    subject.department_id
                },
                name,
                code: code.or_else(|| subject.code.clone()),
                description: if description_mapped {
                    description
                } else {
                    subject.description.clone()
                },
            };
            Ok(unchanged_or_update(
                row,
                label,
                Some(subject.name.clone()),
                proposed == current,
                Intent::UpdateSubject {
                    id: subject.id,
                    rev: subject.rev,
                    input: proposed,
                },
                "a subject with that code or name already exists here and the file \
                 disagrees with it.",
            ))
        }
    }
}

/// Courses, as offerings of this school's subjects.
fn courses(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let code = row.code("code");
    let name = row.required_text("name");
    let subject_key = row.required_code("subjectCode");
    let year_key = row.code("yearLevel");
    let year_mapped = row.cell_mapped("yearLevel");
    let periods = row.number("periodsPerCycle");
    let periods_mapped = row.cell_mapped("periodsPerCycle");
    let duration = row.number("defaultDurationPeriods");
    let room_key = row.code("roomType");
    let room_mapped = row.cell_mapped("roomType");

    let label = name.clone().unwrap_or_else(|| "(no name)".to_owned());
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    let (Some(name), Some(subject_key)) = (name, subject_key) else {
        return Ok(invalid(row, label));
    };
    let claim_key = code
        .clone()
        .unwrap_or_else(|| format!("{subject_key}/{name}"));
    if !claim(row, seen, &claim_key) {
        return Ok(invalid(row, label));
    }

    let Some(subject) = resolve_subject(conn, row, school_id, &subject_key)? else {
        return Ok(invalid(row, label));
    };
    let year_level_id = resolve_one(
        conn,
        row,
        school_id,
        year_key.as_deref(),
        Reference::YearLevel,
    )?;
    let room_type_id = resolve_one(
        conn,
        row,
        school_id,
        room_key.as_deref(),
        Reference::RoomType,
    )?;
    if duration.is_some_and(|duration| duration < 1) {
        row.note(RowFinding::error(
            "defaultDurationPeriods",
            "a lesson needs at least one period.",
        ));
    }
    if periods.is_some_and(|periods| periods < 1) {
        row.note(RowFinding::error(
            "periodsPerCycle",
            "a course needs at least one period a cycle, or leave the column blank.",
        ));
    }
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    match lookup::course_by_key(conn, school_id, &claim_key)? {
        Found::Many(_) => Ok(ambiguous(row, label, "course")),
        Found::None => Ok(finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateCourse {
                input: CourseInput {
                    subject_id: subject,
                    year_level_id,
                    term_id: None,
                    name,
                    code,
                    description: None,
                    periods_per_cycle: periods,
                    default_duration_periods: duration.unwrap_or(1),
                    required_room_type_id: room_type_id,
                },
            }),
        )),
        Found::One(course) => Ok(course_update(
            row,
            &course,
            label,
            CourseEdit {
                subject,
                name,
                code,
                year_level_id,
                year_mapped,
                periods,
                periods_mapped,
                duration,
                room_type_id,
                room_mapped,
            },
        )),
    }
}

/// What a course row asks to change, and which of those columns the file actually carried.
///
/// Carried as a struct because the update path needs all ten, and a function taking ten
/// positional arguments of three types is a function nobody can call correctly.
struct CourseEdit {
    subject: Uuid,
    name: String,
    code: Option<String>,
    year_level_id: Option<Uuid>,
    year_mapped: bool,
    periods: Option<i64>,
    periods_mapped: bool,
    duration: Option<i64>,
    room_type_id: Option<Uuid>,
    room_mapped: bool,
}

/// A course this school already offers under that code or name.
///
/// Split out of `courses` because this is where every "only if the column was mapped"
/// decision lives, and reading it inside the create path obscured both.
fn course_update(row: &mut Row<'_>, course: &Course, label: String, edit: CourseEdit) -> Built {
    let current = CourseInput {
        subject_id: course.subject_id,
        year_level_id: course.year_level_id,
        term_id: course.term_id,
        name: course.name.clone(),
        code: course.code.clone(),
        description: course.description.clone(),
        periods_per_cycle: course.periods_per_cycle,
        default_duration_periods: course.default_duration_periods,
        required_room_type_id: course.required_room_type_id,
    };
    if course.subject_id != edit.subject {
        row.note(RowFinding::row_error(
            "that course already exists here under a different subject. A course is \
                     an offering *of* one subject and cannot be moved between them — give \
                     the new one its own code and name."
                .to_owned(),
        ));
        return invalid(row, label);
    }
    let proposed = CourseInput {
        subject_id: course.subject_id,
        year_level_id: if edit.year_mapped {
            edit.year_level_id
        } else {
            course.year_level_id
        },
        term_id: course.term_id,
        name: edit.name,
        code: edit.code.or_else(|| course.code.clone()),
        description: course.description.clone(),
        periods_per_cycle: if edit.periods_mapped {
            edit.periods
        } else {
            course.periods_per_cycle
        },
        default_duration_periods: edit.duration.unwrap_or(course.default_duration_periods),
        required_room_type_id: if edit.room_mapped {
            edit.room_type_id
        } else {
            course.required_room_type_id
        },
    };
    unchanged_or_update(
        row,
        label,
        Some(course.name.clone()),
        proposed == current,
        Intent::UpdateCourse {
            id: course.id,
            rev: course.rev,
            input: proposed,
        },
        "a course with that code or name already exists here and the file disagrees with it.",
    )
}

/// Class sections of this school's courses.
fn class_sections(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let course_key = row.required_code("courseCode");
    let name = row.required_text("name");
    let code = row.code("code");
    let campus_key = row.code("campusCode");
    let campus_mapped = row.cell_mapped("campusCode");

    let label = format!(
        "{} — {}",
        course_key
            .clone()
            .unwrap_or_else(|| "(no course)".to_owned()),
        name.clone().unwrap_or_else(|| "(no name)".to_owned())
    );
    if row.has_error() {
        return Ok(invalid(row, label));
    }
    let (Some(course_key), Some(name)) = (course_key, name) else {
        return Ok(invalid(row, label));
    };
    let claim_key = format!(
        "{course_key}/{}",
        code.clone().unwrap_or_else(|| name.clone())
    );
    if !claim(row, seen, &claim_key) {
        return Ok(invalid(row, label));
    }

    let Some(course) = resolve_course(conn, row, school_id, &course_key)? else {
        return Ok(invalid(row, label));
    };
    let campus_id = resolve_one(
        conn,
        row,
        school_id,
        campus_key.as_deref(),
        Reference::Campus,
    )?;
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    let section_key = code.clone().unwrap_or_else(|| name.clone());
    match lookup::section_by_key(conn, course, &section_key)? {
        Found::Many(_) => Ok(ambiguous(row, label, "class section")),
        Found::None => Ok(finish(
            row,
            RowAction::Create,
            label,
            None,
            Some(Intent::CreateSection {
                input: ClassSectionInput {
                    course_id: course,
                    name,
                    code,
                    campus_id,
                },
            }),
        )),
        Found::One(section) => {
            let current = ClassSectionInput {
                course_id: section.course_id,
                name: section.name.clone(),
                code: section.code.clone(),
                campus_id: section.campus_id,
            };
            let proposed = ClassSectionInput {
                course_id: section.course_id,
                name,
                code: code.or_else(|| section.code.clone()),
                campus_id: if campus_mapped {
                    campus_id
                } else {
                    section.campus_id
                },
            };
            Ok(unchanged_or_update(
                row,
                label,
                Some(section.name.clone()),
                proposed == current,
                Intent::UpdateSection {
                    id: section.id,
                    rev: section.rev,
                    input: proposed,
                },
                "that class section already exists and the file disagrees with it.",
            ))
        }
    }
}

// ======================================================================== associations

/// Which teachers teach which class sections.
fn class_teachers(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let teacher_key = row.required_code("teacherCode");
    let Some((section, label)) =
        section_reference(conn, request, row, seen, teacher_key.as_deref())?
    else {
        let label = label_of_pair(row, "teacherCode");
        return Ok(invalid(row, label));
    };
    let Some(teacher_key) = teacher_key else {
        return Ok(invalid(row, label));
    };

    let teacher = match lookup::teachers_by_code(conn, school_id, &teacher_key)? {
        Found::One(teacher) => teacher,
        Found::None => {
            row.note(RowFinding::error(
                "teacherCode",
                format!(
                    "no teacher at this school has the code “{teacher_key}”. Import the \
                     teachers first, or correct the code."
                ),
            ));
            return Ok(invalid(row, label));
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "teacherCode",
                "more than one teacher at this school uses that code.",
            ));
            return Ok(invalid(row, label));
        }
    };

    // ADR 0028's two refusals and one warning, read exactly as the screen reads them.
    match lookup::membership_state(conn, school_id, teacher.id)? {
        None => {
            row.note(RowFinding::error(
                "teacherCode",
                "this school does not employ that teacher. Add them to the school before \
                 giving them a class.",
            ));
        }
        Some(false) => {
            row.note(RowFinding::error(
                "teacherCode",
                "that teacher is recorded as no longer teaching at this school, so they \
                 cannot be given new classes. The classes they already have are untouched.",
            ));
        }
        Some(true) if !teacher.is_active => {
            row.note(RowFinding::error(
                "teacherCode",
                "that teacher is archived in this campus file. Restore them on the Teachers \
                 screen first.",
            ));
        }
        Some(true) => {}
    }
    if !lookup::campus_eligible(conn, section, teacher.id)? {
        row.note(RowFinding::error(
            "teacherCode",
            "that teacher is not eligible at the campus this class section is taught at.",
        ));
    }
    if row.has_error() {
        return Ok(invalid(row, label));
    }

    if let Some(subject_id) = lookup::subject_of_section(conn, section)?
        && !lookup::is_qualified(conn, teacher.id, subject_id)?
    {
        {
            // A warning, never a refusal. ADR 0028: an import must not be stricter than
            // the screen, and the screen assigns and warns.
            row.note(RowFinding::warning(
                "no qualification for this subject is recorded for that teacher. The \
                 assignment stands — record one on the Teachers screen if it should be \
                 there."
                    .to_owned(),
            ));
        }
    }

    link_or_unchanged(
        conn,
        row,
        label,
        "class_teacher",
        "class_section_id",
        section,
        "teacher_id",
        teacher.id,
        "class teacher",
    )
}

/// Which student groups attend which class sections.
fn class_groups(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let group_key = row.required_code("groupCode");
    let Some((section, label)) = section_reference(conn, request, row, seen, group_key.as_deref())?
    else {
        let label = label_of_pair(row, "groupCode");
        return Ok(invalid(row, label));
    };
    let Some(group_key) = group_key else {
        return Ok(invalid(row, label));
    };

    let group = match lookup::group_by_key(conn, school_id, &group_key)? {
        Found::One(group) => group,
        Found::None => {
            row.note(RowFinding::error(
                "groupCode",
                format!(
                    "this school has no student group called “{group_key}”. Groups are not \
                     created by this import — add it on the Student groups screen first."
                ),
            ));
            return Ok(invalid(row, label));
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "groupCode",
                "more than one of this school's student groups answers to that code or name.",
            ));
            return Ok(invalid(row, label));
        }
    };

    link_or_unchanged(
        conn,
        row,
        label,
        "class_group",
        "class_section_id",
        section,
        "student_group_id",
        group.id,
        "class group",
    )
}

/// Which named individuals are on a class section's roster.
fn class_enrolments(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let school_id = request.school_id;
    let student_key = row.required_code("studentCode");
    let Some((section, label)) =
        section_reference(conn, request, row, seen, student_key.as_deref())?
    else {
        let label = label_of_pair(row, "studentCode");
        return Ok(invalid(row, label));
    };
    let Some(student_key) = student_key else {
        return Ok(invalid(row, label));
    };

    let student = match lookup::students_by_code(conn, school_id, &student_key)? {
        Found::One(student) => student,
        Found::None => {
            row.note(RowFinding::error(
                "studentCode",
                format!(
                    "no student enrolled at this school has the code “{student_key}”. A \
                     student another school enrolled is not thereby a student here — import \
                     or enrol them first."
                ),
            ));
            return Ok(invalid(row, label));
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "studentCode",
                "more than one student at this school uses that code.",
            ));
            return Ok(invalid(row, label));
        }
    };
    if lookup::current_spell(conn, school_id, student.id)?.is_none() {
        row.note(RowFinding::error(
            "studentCode",
            "that student has left this school, so they cannot be put in one of its \
             classes. Their past class rosters are kept.",
        ));
        return Ok(invalid(row, label));
    }

    link_or_unchanged(
        conn,
        row,
        label,
        "class_enrolment",
        "class_section_id",
        section,
        "student_id",
        student.id,
        "class enrolment",
    )
}

// =========================================================================== activities

/// How often a class section meets, expanded into one record per lesson.
fn activities(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
) -> Result<Built> {
    let duration = row.number("durationPeriods");
    let occurrences = row.number("occurrences");
    let name = row.text("name");
    let Some((section, base_label)) = section_reference(conn, request, row, seen, None)? else {
        let label = label_of_pair(row, "durationPeriods");
        return Ok(invalid(row, label));
    };

    let duration = duration.unwrap_or(0);
    let wanted = occurrences.unwrap_or(1);
    if duration < 1 {
        row.note(RowFinding::error(
            "durationPeriods",
            "a lesson needs at least one period. A double period is one lesson of 2, not \
             two lessons.",
        ));
    }
    if wanted < 1 {
        row.note(RowFinding::error(
            "occurrences",
            "a class that meets no times a cycle is not a class. Leave the column blank for \
             one lesson, or remove the row.",
        ));
    }
    if row.has_error() {
        return Ok(invalid(row, base_label));
    }

    let label = format!(
        "{base_label} — {wanted} × {duration} period{}",
        if duration == 1 { "" } else { "s" }
    );

    // ADR 0031 §5. An activity has no natural key — four identical single periods are four
    // indistinguishable rows — so idempotency is a question about *how many*: the row says
    // the section needs `wanted` lessons of this shape, and the plan proposes the
    // difference. A second run of the same file therefore adds nothing.
    let held = lookup::occurrences_of_shape(conn, section, duration, name.as_deref())?;
    let target = Some(format!("{held} already of this shape"));

    if held == wanted {
        return Ok(finish(row, RowAction::Unchanged, label, target, None));
    }
    if held > wanted {
        row.note(RowFinding::row_error(format!(
            "this class section already has {held} lessons of that shape and the file asks \
             for {wanted}. Removing lessons is destructive and this import never does it — \
             delete them on the Lessons screen, or correct the file."
        )));
        return Ok(finish(row, RowAction::Conflict, label, target, None));
    }

    let adding = wanted - held;
    row.note(RowFinding::warning(format!(
        "Evara stores one record per lesson, so this row becomes {adding} new record{} \
         ({held} of the {wanted} already exist).",
        if adding == 1 { "" } else { "s" }
    )));
    Ok(finish(
        row,
        RowAction::Create,
        label,
        target,
        Some(Intent::AddOccurrences {
            class_section_id: section,
            name,
            duration_periods: duration,
            count: adding,
        }),
    ))
}

// ============================================================================ helpers

/// Reads and resolves the `courseCode` + `sectionCode` pair every association row carries.
///
/// Returns the section and a label, or `None` having recorded why. `extra_key` joins the
/// in-file duplicate key, so two rows putting the same teacher in the same section are
/// caught as a file mistake rather than as a duplicate relationship.
fn section_reference(
    conn: &Connection,
    request: &Request<'_>,
    row: &mut Row<'_>,
    seen: &mut SeenKeys,
    extra_key: Option<&str>,
) -> Result<Option<(Uuid, String)>> {
    let course_key = row.required_code("courseCode");
    let section_key = row.required_code("sectionCode");
    let (Some(course_key), Some(section_key)) = (course_key, section_key) else {
        return Ok(None);
    };
    let label = format!("{course_key} / {section_key}");

    let claim_key = match extra_key {
        Some(extra) => format!("{course_key}/{section_key}/{extra}"),
        None => format!("{course_key}/{section_key}"),
    };
    if !claim(row, seen, &claim_key) {
        return Ok(None);
    }

    let Some(course) = resolve_course(conn, row, request.school_id, &course_key)? else {
        return Ok(None);
    };
    match lookup::section_by_key(conn, course, &section_key)? {
        Found::One(section) => Ok(Some((section.id, label))),
        Found::None => {
            row.note(RowFinding::error(
                "sectionCode",
                format!(
                    "“{course_key}” has no class section called “{section_key}”. Import the \
                     class sections first, or correct the code."
                ),
            ));
            Ok(None)
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "sectionCode",
                "more than one section of that course answers to that code or name.",
            ));
            Ok(None)
        }
    }
}

/// A best-effort label for a row that failed before it could be described properly.
fn label_of_pair(row: &mut Row<'_>, third: &str) -> String {
    let course = row.code("courseCode").unwrap_or_else(|| "?".to_owned());
    let section = row.code("sectionCode").unwrap_or_else(|| "?".to_owned());
    match row.code(third) {
        Some(extra) => format!("{course} / {section} / {extra}"),
        None => format!("{course} / {section}"),
    }
}

/// Creates a relationship, or reports that it is already there.
///
/// A relationship either exists or it does not, so there is no such thing as a conflicting
/// one — which is ADR 0027 §4's reasoning applied to an import. A second run of the same
/// file reports `Unchanged` for every row.
#[expect(
    clippy::too_many_arguments,
    reason = "five of the eight are one relationship's table and column names, which is \
              what lets three kinds share this function"
)]
fn link_or_unchanged(
    conn: &Connection,
    row: &mut Row<'_>,
    label: String,
    table: &'static str,
    left_column: &'static str,
    left: Uuid,
    right_column: &'static str,
    right: Uuid,
    entity: &'static str,
) -> Result<Built> {
    if lookup::relationship(conn, table, left_column, left, right_column, right)?.is_some() {
        return Ok(finish(row, RowAction::Unchanged, label, None, None));
    }
    Ok(finish(
        row,
        RowAction::Create,
        label,
        None,
        Some(Intent::Link {
            table,
            left_column,
            left,
            right_column,
            right,
            entity,
        }),
    ))
}

/// Either nothing changed, or the user has to agree to the change.
fn unchanged_or_update(
    row: &mut Row<'_>,
    label: String,
    target: Option<String>,
    same: bool,
    intent: Intent,
    ask: &str,
) -> Built {
    if same {
        return finish(row, RowAction::Unchanged, label, target, None);
    }
    settle_update(row, label, target, intent, ask)
}

/// The answer for a key that matches more than one record.
fn ambiguous(row: &mut Row<'_>, label: String, noun: &str) -> Built {
    row.note(RowFinding::row_error(format!(
        "more than one {noun} of this school answers to that code or name, so Evara cannot \
         tell which the row means. Give them distinct codes."
    )));
    finish(row, RowAction::Invalid, label, None, None)
}

/// What kind of thing a reference names, for the message when it is missing.
#[derive(Debug, Clone, Copy)]
enum Reference {
    Department,
    Campus,
    YearLevel,
    RoomType,
}

impl Reference {
    const fn noun(self) -> &'static str {
        match self {
            Self::Department => "department",
            Self::Campus => "campus",
            Self::YearLevel => "year level",
            Self::RoomType => "room type",
        }
    }

    const fn field(self) -> &'static str {
        match self {
            Self::Department => "departmentCode",
            Self::Campus => "campusCode",
            Self::YearLevel => "yearLevel",
            Self::RoomType => "roomType",
        }
    }
}

/// Resolves one optional reference within the school.
///
/// A missing reference is an **error**, never a creation. The brief is explicit and so is
/// ADR 0030 §4: *do not create placeholder entities for missing references.* A department
/// invented to satisfy a subject import is a department nobody chose, with a name taken
/// from whatever the spreadsheet happened to hold.
fn resolve_one(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    needle: Option<&str>,
    kind: Reference,
) -> Result<Option<Uuid>> {
    let Some(needle) = needle else {
        return Ok(None);
    };
    let found = match kind {
        Reference::Department => lookup::department_by_key(conn, school_id, needle)?,
        Reference::Campus => lookup::campus_by_key(conn, school_id, needle)?,
        Reference::YearLevel => lookup::year_level_by_name(conn, school_id, needle)?,
        Reference::RoomType => lookup::room_type_by_key(conn, school_id, needle)?,
    };
    match found {
        Found::One((id, _)) => Ok(Some(id)),
        Found::None => {
            row.note(RowFinding::error(
                kind.field(),
                format!(
                    "this school has no {} called “{needle}”. Nothing is created to fill a \
                     gap — add it first, or clear the cell.",
                    kind.noun()
                ),
            ));
            Ok(None)
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                kind.field(),
                format!(
                    "more than one of this school's {}s answers to “{needle}”.",
                    kind.noun()
                ),
            ));
            Ok(None)
        }
    }
}

/// Resolves a list of references, reporting each failure separately.
fn resolve_all(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    needles: &[String],
    kind: Reference,
) -> Result<Vec<Uuid>> {
    let mut found = Vec::new();
    for needle in needles {
        if let Some(id) = resolve_one(conn, row, school_id, Some(needle), kind)? {
            found.push(id);
        }
    }
    Ok(found)
}

/// Resolves a subject reference, reporting a miss against the subject column.
fn resolve_subject(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    needle: &str,
) -> Result<Option<Uuid>> {
    match lookup::subject_by_key(conn, school_id, needle)? {
        Found::One(subject) => Ok(Some(subject.id)),
        Found::None => {
            row.note(RowFinding::error(
                "subjectCode",
                format!(
                    "this school has no subject called “{needle}”. A course is an offering \
                     of a subject, and subjects are not created by a course import — import \
                     them first."
                ),
            ));
            Ok(None)
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "subjectCode",
                format!("more than one of this school's subjects answers to “{needle}”."),
            ));
            Ok(None)
        }
    }
}

/// Resolves a course reference, reporting a miss against the course column.
fn resolve_course(
    conn: &Connection,
    row: &mut Row<'_>,
    school_id: Uuid,
    needle: &str,
) -> Result<Option<Uuid>> {
    match lookup::course_by_key(conn, school_id, needle)? {
        Found::One(course) => Ok(Some(course.id)),
        Found::None => {
            row.note(RowFinding::error(
                "courseCode",
                format!(
                    "this school has no course called “{needle}”. Import the courses first, \
                     or correct the code."
                ),
            ));
            Ok(None)
        }
        Found::Many(_) => {
            row.note(RowFinding::error(
                "courseCode",
                format!("more than one of this school's courses answers to “{needle}”."),
            ));
            Ok(None)
        }
    }
}

/// How a person's row identifies itself in a preview and a report.
fn describe_person(code: Option<&str>, given: Option<&str>, family: Option<&str>) -> String {
    let name = match (given, family) {
        (Some(given), Some(family)) => format!("{given} {family}"),
        (Some(given), None) => given.to_owned(),
        (None, Some(family)) => family.to_owned(),
        (None, None) => "(no name)".to_owned(),
    };
    match code {
        Some(code) => format!("{code} — {name}"),
        None => name,
    }
}
