//! School Setup commands for shared identities: membership, campus eligibility, enrolment.
//!
//! Same shape as [`super::people`], and for the same reason: lock the workspace, call one
//! repository method, convert the error. No validation, no defaulting, no ordering and no
//! uniqueness checking happens here — all of it is in `evara-db`, where the CLI and the
//! tests can reach it too.
//!
//! # Why these are separate from `people`
//!
//! A person is one row for one real human being. Which schools employ or enrol them is a
//! relationship, and since `V13` a project may hold several of those per person. Keeping
//! the two apart in the command surface is what stops a screen scoped to one school
//! treating a shared identity as its own property — the mistake ADR 0021 exists to make
//! structurally impossible.
//!
//! # Every mutation names the school it acts for
//!
//! `V12`'s rule, and sharing makes it matter more rather than less: holding a shared
//! teacher's identifier is now completely ordinary, and must still not be authority to
//! touch another school's employment of them. The one exception is
//! [`setup_student_transfer`], which names **both** schools, because a transfer is one
//! fact about two institutions and splitting it would let a student end up enrolled
//! nowhere. ADR 0021 §7.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    StudentEnrolment, StudentEnrolmentInput, StudentInSchool, TeacherCampus, TeacherInSchool,
    TeacherSchool, TeacherSchoolInput,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ------------------------------------------------------------------ school membership

/// Adds an existing teacher to another school. No teacher record is created.
#[tauri::command]
pub fn setup_teacher_membership_add(
    state: State<'_, AppState>,
    input: TeacherSchoolInput,
) -> SetupResult<TeacherSchool> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.add_teacher_to_school(&input)?)
}

/// Every school one teacher belongs to. The complete answer, not `teacher.schoolId`.
#[tauri::command]
pub fn setup_teacher_membership_list(
    state: State<'_, AppState>,
    teacher_id: Uuid,
) -> SetupResult<Vec<TeacherSchool>> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.teacher_memberships(teacher_id)?)
}

/// Every teacher one school employs, with that school's own view of their employment.
///
/// A teacher shared with another school appears here as the same record, with the same
/// identifier, that the other school sees. One person, two listings.
#[tauri::command]
pub fn setup_school_teacher_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<TeacherInSchool>> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.school_teachers(school_id)?)
}

/// Activates or deactivates one school's employment of a teacher.
///
/// Not a soft delete: they keep the person, their availability, their campuses and every
/// other school's membership.
#[tauri::command]
pub fn setup_teacher_membership_set_active(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    is_active: bool,
) -> SetupResult<TeacherSchool> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .set_teacher_membership_active(school_id, teacher_id, is_active)?)
}

/// Withdraws one school's employment of a teacher. The person is untouched.
#[tauri::command]
pub fn setup_teacher_membership_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .remove_teacher_from_school(school_id, teacher_id)?)
}

// ----------------------------------------------------------------- campus eligibility

/// Every campus one teacher may be based at, across every school they belong to.
#[tauri::command]
pub fn setup_teacher_campus_list(
    state: State<'_, AppState>,
    teacher_id: Uuid,
) -> SetupResult<Vec<Uuid>> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.teacher_campuses(teacher_id)?)
}

/// Makes a teacher eligible at one of the acting school's campuses.
#[tauri::command]
pub fn setup_teacher_campus_assign(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<TeacherCampus> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .assign_teacher_campus(school_id, teacher_id, campus_id)?)
}

/// Withdraws a teacher's eligibility for one campus. Their membership is untouched.
#[tauri::command]
pub fn setup_teacher_campus_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .remove_teacher_campus(school_id, teacher_id, campus_id)?)
}

// ----------------------------------------------------------------- student enrolment

/// Enrols an existing student at a school. No student record is created.
#[tauri::command]
pub fn setup_enrolment_create(
    state: State<'_, AppState>,
    input: StudentEnrolmentInput,
) -> SetupResult<StudentEnrolment> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.enrol_student(&input)?)
}

/// Every student currently at one school, with that school's placement for them.
#[tauri::command]
pub fn setup_school_student_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<StudentInSchool>> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.school_students(school_id)?)
}

/// Every spell one student has ever had, at every school, oldest first.
#[tauri::command]
pub fn setup_enrolment_history(
    state: State<'_, AppState>,
    student_id: Uuid,
) -> SetupResult<Vec<StudentEnrolment>> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.student_enrolments(student_id)?)
}

/// One student's current spell at one school, or `null`.
#[tauri::command]
pub fn setup_enrolment_current(
    state: State<'_, AppState>,
    school_id: Uuid,
    student_id: Uuid,
) -> SetupResult<Option<StudentEnrolment>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .current_enrolment_at(school_id, student_id)?)
}

/// Replaces one spell's placement, status and dates.
#[tauri::command]
pub fn setup_enrolment_update(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    input: StudentEnrolmentInput,
) -> SetupResult<StudentEnrolment> {
    let mut ws = workspace(&state)?;
    Ok(ws.membership()?.update_enrolment(school_id, id, &input)?)
}

/// Records that a student has left one school. The spell is kept, not deleted.
#[tauri::command]
pub fn setup_enrolment_leave(
    state: State<'_, AppState>,
    school_id: Uuid,
    student_id: Uuid,
    left_on: Option<String>,
) -> SetupResult<StudentEnrolment> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .leave_school(school_id, student_id, left_on.as_deref())?)
}

/// Moves a student between two schools, preserving the identity and both spells.
///
/// Names both schools deliberately: see the module documentation.
#[tauri::command]
pub fn setup_student_transfer(
    state: State<'_, AppState>,
    from_school_id: Uuid,
    to_school_id: Uuid,
    student_id: Uuid,
    arriving: StudentEnrolmentInput,
) -> SetupResult<StudentEnrolment> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .membership()?
        .transfer_student(from_school_id, to_school_id, student_id, &arriving)?)
}

// ----------------------------------------------------------------- resource sharing

/// Every campus one resource serves. Possibly empty: a resource may be school-wide.
#[tauri::command]
pub fn setup_resource_campus_list(
    state: State<'_, AppState>,
    resource_id: Uuid,
) -> SetupResult<Vec<Uuid>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.resource_campuses(resource_id)?)
}

/// Shares a resource with a campus, which may belong to another school.
#[tauri::command]
pub fn setup_resource_share(
    state: State<'_, AppState>,
    school_id: Uuid,
    resource_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .share_resource(school_id, resource_id, campus_id)?)
}

/// Stops sharing a resource with a campus. Nothing is moved or duplicated.
#[tauri::command]
pub fn setup_resource_unshare(
    state: State<'_, AppState>,
    school_id: Uuid,
    resource_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .unshare_resource(school_id, resource_id, campus_id)?)
}
