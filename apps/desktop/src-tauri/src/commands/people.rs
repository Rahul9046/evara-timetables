//! School Setup commands for the People group: teachers, students, groups, membership.
//!
//! Same shape as [`super::structure`], and for the same reason: lock the workspace, call
//! one repository method, convert the error. No validation, no defaulting, no ordering and
//! no uniqueness checking happens here — all of it is in `evara-db`, where the CLI and the
//! tests can reach it too. A rule implemented in a command handler would be unreachable
//! from `cargo run -p evara-cli`, which is the architecture bug `CLAUDE.md` names.
//!
//! # Why membership is removed by its two ends
//!
//! [`setup_student_group_member_remove`] takes a group and a student rather than the
//! membership's own identifier. That is what the interface has in hand — a row in a
//! group's member list — and asking it to carry a third identifier it never displays would
//! be making the contract fit the schema rather than the screen.
//!
//! # Counting before deleting
//!
//! [`setup_student_group_member_count`] exists so a confirmation dialogue can say *"this
//! removes 24 memberships"* before anything happens, and
//! [`setup_student_groups_for_student`] so the one for a student can say why the delete
//! will be refused. Both are reads the interface makes *before* it asks, not error text it
//! shows afterwards: a refusal arriving after the user pressed Delete is too late to be
//! the first time they hear about it.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    Expected, Student, StudentGroup, StudentGroupInput, StudentGroupMember,
    StudentGroupMemberInput, StudentInput, Teacher, TeacherInput, YearLevel, YearLevelInput,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ------------------------------------------------------------------------ year level

/// Creates a year level.
#[tauri::command]
pub fn setup_year_level_create(
    state: State<'_, AppState>,
    input: YearLevelInput,
) -> SetupResult<YearLevel> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.create_year_level(&input)?)
}

/// One year level by identifier, or `null`.
#[tauri::command]
pub fn setup_year_level_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<YearLevel>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.year_level(id)?)
}

/// Every year level of one school, in that school's own progression order.
#[tauri::command]
pub fn setup_year_level_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<YearLevel>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.year_levels(school_id)?)
}

/// Replaces a year level's details.
#[tauri::command]
pub fn setup_year_level_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: YearLevelInput,
    expected_rev: i64,
) -> SetupResult<YearLevel> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.update_year_level(id, expected, &input)?)
}

/// Deletes a year level, detaching anything classified by it.
#[tauri::command]
pub fn setup_year_level_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.delete_year_level(school_id, id, expected)?)
}

// --------------------------------------------------------------------------- teacher

/// Creates a teacher.
#[tauri::command]
pub fn setup_teacher_create(
    state: State<'_, AppState>,
    input: TeacherInput,
) -> SetupResult<Teacher> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.create_teacher(&input)?)
}

/// One teacher by identifier, or `null`.
#[tauri::command]
pub fn setup_teacher_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Teacher>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.teacher(id)?)
}

/// Every teacher of one school, active and inactive, ordered by name.
///
/// A teacher still belongs to exactly one school in Stage 0A. Membership of several is
/// Stage 0B (ADR 0021), and nothing here should be read as implying it yet.
#[tauri::command]
pub fn setup_teacher_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<Teacher>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.teachers(school_id)?)
}

/// Replaces a teacher's details. Their identity is unchanged.
#[tauri::command]
pub fn setup_teacher_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: TeacherInput,
    expected_rev: i64,
) -> SetupResult<Teacher> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.update_teacher(id, expected, &input)?)
}

/// Deletes a teacher.
#[tauri::command]
pub fn setup_teacher_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.delete_teacher(school_id, id, expected)?)
}

// --------------------------------------------------------------------------- student

/// Creates a student.
#[tauri::command]
pub fn setup_student_create(
    state: State<'_, AppState>,
    input: StudentInput,
) -> SetupResult<Student> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.create_student(&input)?)
}

/// One student by identifier, or `null`.
#[tauri::command]
pub fn setup_student_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Student>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.student(id)?)
}

/// Every student of one school, enrolled and left, ordered by name.
///
/// A student still belongs to exactly one school in Stage 0A. Enrolment history across
/// schools is Stage 0B (ADR 0021).
#[tauri::command]
pub fn setup_student_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<Student>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.students(school_id)?)
}

/// Replaces a student's details. Their identity is unchanged.
#[tauri::command]
pub fn setup_student_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: StudentInput,
    expected_rev: i64,
) -> SetupResult<Student> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.update_student(id, expected, &input)?)
}

/// Deletes a student who belongs to no group. Refused otherwise.
#[tauri::command]
pub fn setup_student_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.delete_student(school_id, id, expected)?)
}

// --------------------------------------------------------------------- student group

/// Creates a student group.
#[tauri::command]
pub fn setup_student_group_create(
    state: State<'_, AppState>,
    input: StudentGroupInput,
) -> SetupResult<StudentGroup> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.create_student_group(&input)?)
}

/// One student group by identifier, or `null`.
#[tauri::command]
pub fn setup_student_group_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<StudentGroup>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.student_group(id)?)
}

/// Every student group of one school, ordered by name.
#[tauri::command]
pub fn setup_student_group_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<StudentGroup>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.student_groups(school_id)?)
}

/// Replaces a group's details.
#[tauri::command]
pub fn setup_student_group_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: StudentGroupInput,
    expected_rev: i64,
) -> SetupResult<StudentGroup> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.update_student_group(id, expected, &input)?)
}

/// Deletes a group and the memberships that only existed because of it.
#[tauri::command]
pub fn setup_student_group_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.people()?.delete_student_group(school_id, id, expected)?)
}

// ------------------------------------------------------------------------ membership

/// Adds a student to a group.
#[tauri::command]
pub fn setup_student_group_member_add(
    state: State<'_, AppState>,
    input: StudentGroupMemberInput,
) -> SetupResult<StudentGroupMember> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.add_student_group_member(&input)?)
}

/// Removes one student from one group. The student is untouched.
#[tauri::command]
pub fn setup_student_group_member_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    student_group_id: Uuid,
    student_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .people()?
        .remove_student_group_member(school_id, student_group_id, student_id)?)
}

/// Every student in one group, ordered by name.
#[tauri::command]
pub fn setup_student_group_member_list(
    state: State<'_, AppState>,
    student_group_id: Uuid,
) -> SetupResult<Vec<Student>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.student_group_members(student_group_id)?)
}

/// How many students are in one group, for a confirmation that says so before it asks.
#[tauri::command]
pub fn setup_student_group_member_count(
    state: State<'_, AppState>,
    student_group_id: Uuid,
) -> SetupResult<i64> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.student_group_member_count(student_group_id)?)
}

/// Every group one student belongs to.
#[tauri::command]
pub fn setup_student_groups_for_student(
    state: State<'_, AppState>,
    student_id: Uuid,
) -> SetupResult<Vec<StudentGroup>> {
    let mut ws = workspace(&state)?;
    Ok(ws.people()?.groups_for_student(student_id)?)
}
