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
use crate::repo::availability::{AvailabilityEntity, AvailabilityRule};
use crate::repo::people::{Student, Teacher};
use crate::repo::structure::{Campus, RoomType, School, SchoolLogo};
use crate::repo::time_model::{Cycle, NonWorkingInstructionalDay};
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
