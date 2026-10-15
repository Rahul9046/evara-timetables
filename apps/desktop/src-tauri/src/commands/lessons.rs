//! School Setup commands for the Lessons group.
//!
//! Courses, class sections, their staffing and audience, the optional individual roster,
//! and the occurrences they demand. Same shape as [`super::curriculum`] and for the same
//! reason: lock the workspace, call one repository method, convert the error. No
//! validation, no defaulting, no ordering and no uniqueness checking happens here — all of
//! it is in `evara-db`, where the CLI and the tests reach it too.
//!
//! # Every mutation names the school it acts for
//!
//! None of the six tables `V15` adds carries a `school_id`: a course resolves its school
//! through its subject, a section through its course, an activity through its section. So
//! `schoolId` is an explicit parameter on every read and every write here rather than
//! something lifted off the input — which is the one visible consequence of that design,
//! and the thing that lets the backend refuse a stale screen rather than trusting the
//! interface to have scoped itself. See
//! [adr/0029](../../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md).
//!
//! `setup_course_update` therefore takes `schoolId` *and* `input`, where
//! `setup_subject_update` takes only `input`. The difference is not an inconsistency: a
//! `SubjectInput` carries the school it is for, and a `CourseInput` has nowhere truthful to
//! put one.
//!
//! # Every mutation of a row names the revision it expects
//!
//! `expectedRev` on every update and every delete, including the three relationship
//! removals, because each of those rows is displayed in a list the user is acting on —
//! ADR 0027 §4's rule for when a relationship carries one. The creations take none: there
//! is no prior version of a row that does not exist yet, and that includes
//! `setup_activity_add`, which writes several rows in one transaction and is still a
//! creation (ADR 0027 §5).
//!
//! # Two reads that are about other screens
//!
//! `setup_student_class_count` and `setup_group_class_count` are called by the Students and
//! Student groups screens, not by Lessons. They are here because the rows they count are
//! this group's, and because a blocked delete has to *explain* before the button rather
//! than fail after it — ADR 0023 §4.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    Activity, AssignedTeacher, AttendingGroup, ClassEnrolment, ClassGroup, ClassSection,
    ClassSectionDependents, ClassSectionInput, ClassSectionView, ClassTeacher, Course,
    CourseDependents, CourseInput, CourseView, Expected, RosteredStudent, SectionDemand,
    TaughtSection, TeacherCandidate,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ------------------------------------------------------------------------------ course

/// Creates a course as an offering of one of this school's subjects.
#[tauri::command]
pub fn setup_course_create(
    state: State<'_, AppState>,
    school_id: Uuid,
    input: CourseInput,
) -> SetupResult<Course> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.create_course(school_id, &input)?)
}

/// One course by identifier, or `null`.
#[tauri::command]
pub fn setup_course_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Course>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.course(id)?)
}

/// Every course of one school, by subject then name, with the names a listing needs.
///
/// Two schools' Mathematics are two subjects, so their courses are different courses with
/// different identifiers — this is a call that must be re-made when the selected school
/// changes, and nothing here merges them.
#[tauri::command]
pub fn setup_course_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<CourseView>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.courses(school_id)?)
}

/// Replaces a course's details.
///
/// Its subject is not among them: a course is an offering *of* a subject, and repointing
/// one would turn a Mathematics course into a Physics course while keeping its sections,
/// staff and roster.
#[tauri::command]
pub fn setup_course_update(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    input: CourseInput,
    expected_rev: i64,
) -> SetupResult<Course> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .update_course(school_id, id, expected, &input)?)
}

/// What a course still holds, so a confirmation can explain before it asks.
#[tauri::command]
pub fn setup_course_dependents(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<CourseDependents> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.course_dependents(id)?)
}

/// Deletes a course that has no class sections.
///
/// Refused while any remain, because a section has staff and a roster on it. Call
/// `setup_course_dependents` first so the user is told how many, rather than finding out by
/// pressing Delete.
#[tauri::command]
pub fn setup_course_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.lessons()?.delete_course(school_id, id, expected)?)
}

// ----------------------------------------------------------------------- class section

/// Creates a class section of one of this school's courses.
#[tauri::command]
pub fn setup_class_section_create(
    state: State<'_, AppState>,
    school_id: Uuid,
    input: ClassSectionInput,
) -> SetupResult<ClassSection> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.create_class_section(school_id, &input)?)
}

/// One class section by identifier, or `null`.
#[tauri::command]
pub fn setup_class_section_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<ClassSection>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_section(id)?)
}

/// Every class section of one course, with its staffing, audience and demand.
#[tauri::command]
pub fn setup_class_section_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    course_id: Uuid,
) -> SetupResult<Vec<ClassSectionView>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_sections(school_id, course_id)?)
}

/// Replaces a class section's details.
///
/// Refused if moving it to a campus would strand a teacher who is not eligible there —
/// the other direction of ADR 0028's campus rule, which `V15`'s triggers cannot see because
/// they fire on a write to `class_teacher` rather than to `class_section`.
#[tauri::command]
pub fn setup_class_section_update(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    input: ClassSectionInput,
    expected_rev: i64,
) -> SetupResult<ClassSection> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .update_class_section(school_id, id, expected, &input)?)
}

/// What a class section holds, so a confirmation can say what deleting it takes with it.
///
/// Unlike a course's dependents these do **not** block the delete: the section owns every
/// one of them and clears them itself. The counts exist so the user is told what is about
/// to go.
#[tauri::command]
pub fn setup_class_section_dependents(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<ClassSectionDependents> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_section_dependents(id)?)
}

/// Deletes a class section, taking its staffing, audience, roster and occurrences.
#[tauri::command]
pub fn setup_class_section_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .delete_class_section(school_id, id, expected)?)
}

// --------------------------------------------------------------------- teacher staffing

/// Every teacher assigned to one class section, with their membership and qualification
/// status.
#[tauri::command]
pub fn setup_class_teacher_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<AssignedTeacher>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_teachers(school_id, class_section_id)?)
}

/// Every teacher this school could give the section, and why each one may or may not.
///
/// The whole of ADR 0028's policy in one read, including the members that would be
/// refused — so the picker can say why somebody expected is unavailable rather than
/// leaving them mysteriously absent.
#[tauri::command]
pub fn setup_class_teacher_candidates(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<TeacherCandidate>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .assignable_teachers(school_id, class_section_id)?)
}

/// Gives one of this school's class sections to a teacher.
///
/// Requires an **active** membership at the section's school and campus eligibility where
/// the section names a campus. Does **not** require a qualification for the subject —
/// ADR 0028 — though `setup_class_teacher_candidates` reports its absence so the interface
/// can warn before the save and after it.
#[tauri::command]
pub fn setup_class_teacher_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    teacher_id: Uuid,
) -> SetupResult<ClassTeacher> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .assign_class_teacher(school_id, class_section_id, teacher_id)?)
}

/// Takes a teacher off one of this school's class sections.
///
/// Their membership, campus eligibility, departments and every qualification they hold are
/// untouched. Addressed by its two ends rather than by the assignment's own identifier,
/// which is what the interface has in hand — same reasoning as
/// `setup_qualification_remove`.
#[tauri::command]
pub fn setup_class_teacher_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    teacher_id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .remove_class_teacher(school_id, class_section_id, teacher_id, expected)?)
}

/// Every class section one teacher teaches, across every school.
///
/// Project-wide, including other schools of the same trust. The interface groups by
/// `schoolId` and makes only the selected school's rows editable.
#[tauri::command]
pub fn setup_teacher_sections(
    state: State<'_, AppState>,
    teacher_id: Uuid,
) -> SetupResult<Vec<TaughtSection>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.sections_of_teacher(teacher_id)?)
}

// ------------------------------------------------------------------ attending groups

/// Every student group attending one class section, with the population it contributes.
#[tauri::command]
pub fn setup_class_group_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<AttendingGroup>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_groups(school_id, class_section_id)?)
}

/// Adds one of this school's student groups to a class section.
#[tauri::command]
pub fn setup_class_group_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    student_group_id: Uuid,
) -> SetupResult<ClassGroup> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .assign_class_group(school_id, class_section_id, student_group_id)?)
}

/// Takes a student group off one of this school's class sections.
///
/// The group itself, its members, its size and its availability are untouched.
#[tauri::command]
pub fn setup_class_group_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    student_group_id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .remove_class_group(school_id, class_section_id, student_group_id, expected)?)
}

// -------------------------------------------------------------- individual roster

/// Every individual on one class section's roster.
///
/// `coveredByGroup` answers "is this child being counted twice?" visibly. They are not:
/// the group side is the scheduling population in Milestone 1 and nothing adds the two
/// figures. ADR 0029 §4.
#[tauri::command]
pub fn setup_class_enrolment_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<RosteredStudent>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .class_enrolments(school_id, class_section_id)?)
}

/// Puts one named student on a class section's roster.
///
/// Optional throughout: ADR 0006 requires that a timetable be buildable from group sizes
/// alone, so nothing treats an empty roster as incomplete. Refused for a student with no
/// current spell at this school — including one whose identity exists only because another
/// school enrolled them.
#[tauri::command]
pub fn setup_class_enrolment_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    student_id: Uuid,
) -> SetupResult<ClassEnrolment> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .enrol_in_class(school_id, class_section_id, student_id)?)
}

/// Takes one individual off a class section's roster.
///
/// Their identity, their school enrolment and their group memberships are untouched.
#[tauri::command]
pub fn setup_class_enrolment_remove(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    student_id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .remove_from_class(school_id, class_section_id, student_id, expected)?)
}

// -------------------------------------------------------------------------- activities

/// Every occurrence of one class section, in its stable order.
#[tauri::command]
pub fn setup_activity_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<Activity>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.activities(school_id, class_section_id)?)
}

/// What a class section asks of a cycle, and anything about it that cannot work.
///
/// `findings` reports only **impossibility**. An empty list does not mean the section can
/// be timetabled — only a solver can say that, and there is not one yet.
#[tauri::command]
pub fn setup_section_demand(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<SectionDemand> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.section_demand(school_id, class_section_id)?)
}

/// Adds `count` identical occurrences to one of this school's class sections.
///
/// This is how "four times per cycle" is entered: four rows, each one placeable unit, each
/// with its own identifier and its own revision. A creation, so no `expectedRev` — all
/// `count` rows are written in one transaction, so a failure part-way leaves none.
#[tauri::command]
pub fn setup_activity_add(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
    name: Option<String>,
    duration_periods: i64,
    count: i64,
) -> SetupResult<Vec<Activity>> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.add_activities(
        school_id,
        class_section_id,
        name.as_deref(),
        duration_periods,
        count,
    )?)
}

/// Fills an empty class section with the occurrences its course declares.
///
/// `ROADMAP.md`'s Phase 3 activity generator. Refuses a section that already has
/// occurrences rather than adding to them, and invents no number for a course that
/// declares none.
#[tauri::command]
pub fn setup_activity_generate(
    state: State<'_, AppState>,
    school_id: Uuid,
    class_section_id: Uuid,
) -> SetupResult<Vec<Activity>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .lessons()?
        .generate_activities(school_id, class_section_id)?)
}

/// Replaces one occurrence's name and duration.
#[tauri::command]
pub fn setup_activity_update(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    name: Option<String>,
    duration_periods: i64,
    expected_rev: i64,
) -> SetupResult<Activity> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws
        .lessons()?
        .update_activity(school_id, id, expected, name.as_deref(), duration_periods)?)
}

/// Deletes one occurrence.
///
/// The remaining occurrences keep their `splitIndex` values, so deleting the second of four
/// leaves 1, 3, 4. The interface numbers the rows it displays.
#[tauri::command]
pub fn setup_activity_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    let expected = Expected::Rev(expected_rev);
    Ok(ws.lessons()?.delete_activity(school_id, id, expected)?)
}

// ------------------------------------------------- what a lesson row blocks elsewhere

/// How many class sections one student is on the roster of, across every school.
///
/// Read by the **Students** screen so a blocked delete explains rather than fails:
/// `setup_student_delete` is refused while any remain.
#[tauri::command]
pub fn setup_student_class_count(state: State<'_, AppState>, student_id: Uuid) -> SetupResult<i64> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_count_of_student(student_id)?)
}

/// How many class sections one student group attends.
///
/// Read by the **Student groups** screen, for the same reason and with the same effect.
#[tauri::command]
pub fn setup_group_class_count(
    state: State<'_, AppState>,
    student_group_id: Uuid,
) -> SetupResult<i64> {
    let mut ws = workspace(&state)?;
    Ok(ws.lessons()?.class_count_of_group(student_group_id)?)
}
