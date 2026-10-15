//! School Setup commands for the Curriculum prerequisites.
//!
//! Departments, subjects, teacher qualifications and teacher department associations. Same
//! shape as [`super::structure`] and for the same reason: lock the workspace, call one
//! repository method, convert the error. No validation, no defaulting, no ordering and no
//! uniqueness checking happens here — all of it is in `evara-db`, where the CLI and the
//! tests reach it too.
//!
//! # Every mutation names the school it acts for
//!
//! A department and a subject are school-owned, so each read takes a `schoolId` and each
//! write refuses an identifier belonging to another school. The two relationship surfaces
//! take one as well, even though the rows they write carry no `school_id`: the authority to
//! record a qualification comes from owning the *subject*, and the authority to put
//! somebody in a department comes from owning the *department*. Passing the acting school
//! explicitly is what lets the backend refuse a stale screen, rather than trusting the
//! interface to have scoped itself.
//!
//! # Every mutation of a row names the revision it expects
//!
//! `expectedRev` on every update and every delete, including the two relationship
//! removals, because a relationship row has a revision like anything else. The creations
//! take none: there is no prior version of a row that does not exist yet. See
//! [adr/0027](../../../../../docs/adr/0027-revisions-are-checked-on-write.md) §4 for the
//! contract each category carries.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    Department, DepartmentDependents, DepartmentInput, Expected, QualifiedSubject,
    QualifiedTeacher, Subject, SubjectDependents, SubjectInput, Teacher, TeacherDepartment,
    TeacherDepartmentView, TeacherQualification,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ------------------------------------------------------------------------- department

/// Creates a department in one school.
#[tauri::command]
pub fn setup_department_create(
    state: State<'_, AppState>,
    input: DepartmentInput,
) -> SetupResult<Department> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.create_department(&input)?)
}

/// One department by identifier, or `null`.
#[tauri::command]
pub fn setup_department_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<Department>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.department(id)?)
}

/// Every department of one school, by name.
#[tauri::command]
pub fn setup_department_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<Department>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.departments(school_id)?)
}

/// Replaces a department's details.
#[tauri::command]
pub fn setup_department_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: DepartmentInput,
    expected_rev: i64,
) -> SetupResult<Department> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.curriculum()?.update_department(id, expected, &input)?)
}

/// What a department still holds, so a confirmation can explain before it asks.
#[tauri::command]
pub fn setup_department_dependents(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<DepartmentDependents> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.department_dependents(id)?)
}

/// Every teacher in one department, by name.
#[tauri::command]
pub fn setup_department_teachers(
    state: State<'_, AppState>,
    school_id: Uuid,
    department_id: Uuid,
) -> SetupResult<Vec<Teacher>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .curriculum()?
        .department_teachers(school_id, department_id)?)
}

/// Deletes a department nothing refers to.
///
/// Refused while any subject is classified by it or any teacher is in it. Call
/// `setup_department_dependents` first so the user is told which, rather than finding out
/// by pressing Delete.
#[tauri::command]
pub fn setup_department_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .curriculum()?
        .delete_department(school_id, id, expected)?)
}

// ---------------------------------------------------------------------------- subject

/// Creates a subject in one school, optionally in one of its departments.
#[tauri::command]
pub fn setup_subject_create(
    state: State<'_, AppState>,
    input: SubjectInput,
) -> SetupResult<Subject> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.create_subject(&input)?)
}

/// One subject by identifier, or `null`.
#[tauri::command]
pub fn setup_subject_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Subject>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.subject(id)?)
}

/// Every subject of one school, by name.
///
/// Two schools' Mathematics are two subjects with two identifiers, so this is the call
/// that must be re-made when the selected school changes — nothing here merges them.
#[tauri::command]
pub fn setup_subject_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<Subject>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.subjects(school_id)?)
}

/// Replaces a subject's details, including which department teaches it.
#[tauri::command]
pub fn setup_subject_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: SubjectInput,
    expected_rev: i64,
) -> SetupResult<Subject> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.curriculum()?.update_subject(id, expected, &input)?)
}

/// What a subject still holds.
#[tauri::command]
pub fn setup_subject_dependents(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<SubjectDependents> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.subject_dependents(id)?)
}

/// Every teacher qualified to teach one subject, by name.
#[tauri::command]
pub fn setup_subject_teachers(
    state: State<'_, AppState>,
    school_id: Uuid,
    subject_id: Uuid,
) -> SetupResult<Vec<QualifiedTeacher>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.subject_teachers(school_id, subject_id)?)
}

/// Deletes a subject nobody is qualified for.
///
/// Refused while anybody is, rather than discarding what a school recorded about its
/// staff. Call `setup_subject_dependents` first.
#[tauri::command]
pub fn setup_subject_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.curriculum()?.delete_subject(school_id, id, expected)?)
}

// --------------------------------------------------------------------- qualifications

/// Records that a teacher is qualified to teach one of this school's subjects.
///
/// Requires no school membership and creates none — see `Curriculum::qualify_teacher` and
/// ADR 0025 §4.
#[tauri::command]
pub fn setup_qualification_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    subject_id: Uuid,
) -> SetupResult<TeacherQualification> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .curriculum()?
        .qualify_teacher(school_id, teacher_id, subject_id)?)
}

/// Every subject one teacher is qualified to teach, across every school.
///
/// Project-wide, including schools they no longer work at. The interface groups by the
/// subject's owning school and makes only the selected school's rows editable.
#[tauri::command]
pub fn setup_qualification_list(
    state: State<'_, AppState>,
    teacher_id: Uuid,
) -> SetupResult<Vec<QualifiedSubject>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.qualifications_of_teacher(teacher_id)?)
}

/// Withdraws a qualification. Always explicit, never a side effect of anything else.
///
/// Addressed by its two ends rather than by the qualification's own identifier, which is
/// what the interface has in hand — a row in a teacher's subject list. Same reasoning as
/// `setup_student_group_member_remove`.
#[tauri::command]
pub fn setup_qualification_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    subject_id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .curriculum()?
        .unqualify_teacher(school_id, teacher_id, subject_id, expected)?)
}

// ---------------------------------------------------------- department associations

/// Puts a teacher in one of this school's departments.
///
/// Refused unless the school already employs them, with a message naming that remedy.
#[tauri::command]
pub fn setup_teacher_department_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    department_id: Uuid,
) -> SetupResult<TeacherDepartment> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .curriculum()?
        .assign_teacher_department(school_id, teacher_id, department_id)?)
}

/// Every department one teacher belongs to, across every school.
#[tauri::command]
pub fn setup_teacher_department_list(
    state: State<'_, AppState>,
    teacher_id: Uuid,
) -> SetupResult<Vec<TeacherDepartmentView>> {
    let mut ws = workspace(&state)?;
    Ok(ws.curriculum()?.departments_of_teacher(teacher_id)?)
}

/// Takes a teacher out of one of this school's departments.
///
/// Their membership, their campus eligibility and every qualification they hold are
/// untouched.
#[tauri::command]
pub fn setup_teacher_department_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    teacher_id: Uuid,
    department_id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.curriculum()?.remove_teacher_department(
        school_id,
        teacher_id,
        department_id,
        expected,
    )?)
}
