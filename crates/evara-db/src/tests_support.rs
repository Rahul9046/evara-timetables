//! Helpers for tests written when a project held exactly one school.
//!
//! # Why these exist
//!
//! Before `V12` every list was project-wide, because a project described one school. The
//! repositories now take an explicit `school_id`, and that is the whole point — an
//! unscoped list would hand one institution another's rows. But most of the existing
//! suites are *about* something else entirely: the working week, the logo, the cycle-day
//! limit, the grid. Rewriting each of them to thread a school identifier through would
//! bury what they actually assert.
//!
//! So each helper here resolves "this project's one school" and **asserts that there is
//! exactly one**. A test that grows a second school fails loudly rather than quietly
//! reading whichever row came back first — which is precisely the failure mode that
//! deleting `Structure::school()` was meant to make impossible.
//!
//! Nothing here is compiled into the application. The scoped repository calls are the only
//! API; these are a test-side convenience over them, kept out of the repositories so no
//! production call site can reach for one by accident.

use uuid::Uuid;

use crate::Database;
use crate::error::Result;
use crate::repo::Expected;
use crate::repo::availability::{AvailabilityEntity, AvailabilityRule};
use crate::repo::people::{Student, Teacher};
use crate::repo::structure::{Campus, RoomType, School, SchoolLogo};
use crate::repo::time_model::{
    Cycle, MaterialisationRequest, NonWorkingInstructionalDay, PlanFingerprint,
};
use evara_core::calendar::WorkingDays;

/// The project's one school, or `None` if setup has not started.
///
/// # Panics
///
/// If the project holds more than one school. A test using this helper is asserting, by
/// using it, that there is only one.
pub(crate) fn only_school(db: &mut Database) -> Option<School> {
    let schools = db.structure().schools().expect("read schools");
    assert!(
        schools.len() <= 1,
        "this helper means `the only school`, but the project holds {} -- \
         the test should name the school it means",
        schools.len()
    );
    schools.into_iter().next()
}

/// Identifier of the project's one school.
///
/// # Panics
///
/// If there is not exactly one.
pub(crate) fn only_school_id(db: &mut Database) -> Uuid {
    only_school(db).expect("a school exists").id
}

/// Campuses of the project's one school.
pub(crate) fn campuses_here(db: &mut Database) -> Vec<Campus> {
    let school = only_school_id(db);
    db.structure().campuses(school).expect("list campuses")
}

/// Room types of the project's one school.
pub(crate) fn room_types_here(db: &mut Database) -> Vec<RoomType> {
    let school = only_school_id(db);
    db.structure().room_types(school).expect("list room types")
}

/// The one school's logo, or `None` when it uses Evara's.
pub(crate) fn logo_here(db: &mut Database) -> Option<SchoolLogo> {
    let school = only_school_id(db);
    db.structure().school_logo(school).expect("read logo")
}

/// Removes the one school's logo. Removing nothing succeeds.
pub(crate) fn clear_logo_here(db: &mut Database) {
    let school = only_school_id(db);
    db.structure()
        .clear_school_logo(school)
        .expect("clear logo");
}

/// Cycles of the project's one school.
pub(crate) fn cycles_here(db: &mut Database) -> Vec<Cycle> {
    let school = only_school_id(db);
    db.time_model().cycles(school).expect("list cycles")
}

/// Calendar days of the project's one school, over an inclusive date range.
pub(crate) fn calendar_days_here(
    db: &mut Database,
    from: &str,
    to: &str,
) -> Vec<crate::repo::time_model::CalendarDay> {
    let school = only_school_id(db);
    db.time_model()
        .calendar_days(school, from, to)
        .expect("list calendar days")
}

/// Mapped teaching dates of the one school that fall outside a given working week.
pub(crate) fn non_working_here(
    db: &mut Database,
    week: WorkingDays,
) -> Vec<NonWorkingInstructionalDay> {
    let school = only_school_id(db);
    db.time_model()
        .non_working_instructional_days(school, week)
        .expect("read findings")
}

/// Teachers of the project's one school.
pub(crate) fn teachers_here(db: &mut Database) -> Vec<Teacher> {
    let school = only_school_id(db);
    db.people().teachers(school).expect("list teachers")
}

/// Students of the project's one school.
pub(crate) fn students_here(db: &mut Database) -> Vec<Student> {
    let school = only_school_id(db);
    db.people().students(school).expect("list students")
}

/// Availability rules for one entity, in the project's one school.
pub(crate) fn rules_here(
    db: &mut Database,
    entity_kind: AvailabilityEntity,
    entity_id: Uuid,
) -> Vec<AvailabilityRule> {
    let school = only_school_id(db);
    db.availability()
        .rules_for(school, entity_kind, entity_id)
        .expect("list rules")
}

/// How many availability rules one entity has, in the project's one school.
pub(crate) fn count_here(
    db: &mut Database,
    entity_kind: AvailabilityEntity,
    entity_id: Uuid,
) -> i64 {
    let school = only_school_id(db);
    db.availability()
        .count_for(school, entity_kind, entity_id)
        .expect("count rules")
}

// ------------------------------------------------------------------------- deletions
//
// Only the ones the older suites actually call. The rest of the destructive paths --
// resources, terms, cycles, bell schedules, calendar days -- are exercised in
// `tests_multi_school`, which names its schools explicitly because the school is the thing
// it is testing.
//
// Destructive operations take the school they act for, so an identifier alone is not
// authority to delete anything. These resolve the project's one school and pass it, and —
// unlike the readers above — they return the `Result` so each suite keeps asserting the
// outcome it was written to assert: a refusal, a `StillReferenced`, a corruption report.

/// Deletes a campus of the project's one school.
pub(crate) fn delete_campus_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure().delete_campus(school, id, Expected::Any)
}

/// Deletes a room type of the project's one school.
pub(crate) fn delete_room_type_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure().delete_room_type(school, id, Expected::Any)
}

/// Deletes a building of the project's one school.
pub(crate) fn delete_building_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure().delete_building(school, id, Expected::Any)
}

/// Deletes a room of the project's one school.
pub(crate) fn delete_room_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure().delete_room(school, id, Expected::Any)
}

/// Deletes an academic year of the project's one school.
pub(crate) fn delete_academic_year_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure()
        .delete_academic_year(school, id, Expected::Any)
}

/// Withdraws a building share, acting as the project's one school.
pub(crate) fn unshare_building_here(
    db: &mut Database,
    building_id: Uuid,
    campus_id: Uuid,
) -> Result<()> {
    let school = only_school_id(db);
    db.structure()
        .unshare_building(school, building_id, campus_id)
}

/// Withdraws a room share, acting as the project's one school.
pub(crate) fn unshare_room_here(db: &mut Database, room_id: Uuid, campus_id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.structure().unshare_room(school, room_id, campus_id)
}

/// Deletes a cycle day of the project's one school.
pub(crate) fn delete_cycle_day_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.time_model().delete_cycle_day(school, id, Expected::Any)
}

/// Deletes a period of the project's one school.
pub(crate) fn delete_period_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.time_model().delete_period(school, id, Expected::Any)
}

/// Deletes timeslots of the project's one school.
pub(crate) fn delete_timeslots_here(db: &mut Database, ids: &[Uuid]) -> Result<()> {
    let school = only_school_id(db);
    db.time_model().delete_timeslots(school, ids)
}

/// Releases the orphans of a reviewed plan, acting as the project's one school.
pub(crate) fn release_orphans_here(
    db: &mut Database,
    request: &MaterialisationRequest,
    reviewed: &PlanFingerprint,
) -> Result<Vec<Uuid>> {
    let school = only_school_id(db);
    db.time_model().release_orphans(school, request, reviewed)
}

/// Deletes a year level of the project's one school.
pub(crate) fn delete_year_level_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.people().delete_year_level(school, id, Expected::Any)
}

/// Deletes a teacher of the project's one school.
pub(crate) fn delete_teacher_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.people().delete_teacher(school, id, Expected::Any)
}

/// Deletes a student of the project's one school.
pub(crate) fn delete_student_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.people().delete_student(school, id, Expected::Any)
}

/// Deletes a student group of the project's one school.
pub(crate) fn delete_student_group_here(db: &mut Database, id: Uuid) -> Result<()> {
    let school = only_school_id(db);
    db.people().delete_student_group(school, id, Expected::Any)
}

/// Removes one group membership in the project's one school.
pub(crate) fn remove_group_member_here(
    db: &mut Database,
    student_group_id: Uuid,
    student_id: Uuid,
) -> Result<()> {
    let school = only_school_id(db);
    db.people()
        .remove_student_group_member(school, student_group_id, student_id)
}
