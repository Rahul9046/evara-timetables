//! Stage B: two windows on one project, and what must happen when they disagree.
//!
//! # The failure these tests exist to prevent
//!
//! Until Stage B no write took a precondition. Two windows could read the same room, both
//! change its name, and the second save would win **silently** — the first person's edit
//! gone, with nothing on either screen to say so. `ROADMAP.md` called fixing it *"the last
//! thing between this phase and done"*.
//!
//! The fix is a revision predicate in the statement itself, which is what makes these
//! tests meaningful rather than decorative. A read-compare-write version would pass a test
//! that writes sequentially and still lose an update under a real race, so what is asserted
//! here is the shape of the mechanism as well as its outcome:
//!
//! - the stale save **fails**, with a typed [`DbError::Conflict`] distinguishable from
//!   `NotFound`;
//! - the row still holds the **first** writer's values;
//! - the revision did **not** advance;
//! - and the change feed gained **no entry**, because no trigger fired.
//!
//! The last two are the ones a plausible-but-wrong implementation gets wrong: a version
//! that wrote first and then noticed would leave a bumped `rev` and a change-log row
//! describing an edit that was rolled back.
//!
//! # Where a revision is deliberately *not* the contract
//!
//! ADR 0027 §4 classifies every mutation, and three categories carry something else. Those
//! are asserted here too, because "we decided not to" and "we forgot" look identical from
//! the outside:
//!
//! - **a relationship** either exists or does not, so its contract is duplicate-on-add and
//!   not-found-on-remove;
//! - **an atomic transfer** stays one call with one precondition;
//! - **the grid** is guarded by a plan fingerprint, which is strictly stronger.

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::curriculum::{DepartmentInput, SubjectInput};
use crate::repo::membership::{EnrolmentStatus, StudentEnrolmentInput, TeacherSchoolInput};
use crate::repo::people::{StudentInput, TeacherInput, YearLevelInput};
use crate::repo::structure::{AcademicYearInput, CampusInput, RoomTypeInput, SchoolInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

// ========================================================================= the fixture

struct Project {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
}

impl Project {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let settings = dir.path().join("settings.sqlite");
        let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
        let path = dir.path().join(format!("two-windows.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");

        let mut this = Self {
            _dir: dir,
            workspace,
            north: Uuid::nil(),
            south: Uuid::nil(),
        };
        this.north = this.school("Northgate Secondary");
        this.south = this.school("Southmoor College");
        this
    }

    fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    fn school(&mut self, name: &str) -> Uuid {
        self.db()
            .structure()
            .create_school(&SchoolInput {
                name: name.to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("create school")
            .id
    }

    fn room_type(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .structure()
            .create_room_type(&RoomTypeInput {
                school_id: school,
                name: name.to_owned(),
                code: code.to_owned(),
            })
            .expect("create room type")
            .id
    }

    fn campus(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .structure()
            .create_campus(&CampusInput {
                school_id: school,
                name: name.to_owned(),
                code: code.to_owned(),
                address: None,
            })
            .expect("create campus")
            .id
    }

    fn teacher(&mut self, school: Uuid, family: &str) -> Uuid {
        self.db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: school,
                code: None,
                given_name: "Ada".to_owned(),
                family_name: family.to_owned(),
                email: None,
                home_campus_id: None,
                fte_percent: 100,
                max_periods_per_day: None,
                max_periods_per_cycle: None,
                max_consecutive_periods: None,
                notes: None,
                is_active: true,
            })
            .expect("create teacher")
            .id
    }

    fn student(&mut self, school: Uuid, family: &str) -> Uuid {
        self.db()
            .people()
            .create_student(&StudentInput {
                school_id: school,
                code: None,
                given_name: "Cass".to_owned(),
                family_name: family.to_owned(),
                email: None,
                year_level_id: None,
                campus_id: None,
                is_active: true,
            })
            .expect("create student")
            .id
    }

    /// How many entries the change feed holds. Takes no arguments on purpose: the number
    /// on its own is what a conflict test needs, before and after.
    fn feed_length(&mut self) -> usize {
        self.db().changes_since(0, 100_000).expect("feed").len()
    }
}

fn room_type_input(school: Uuid, name: &str, code: &str) -> RoomTypeInput {
    RoomTypeInput {
        school_id: school,
        name: name.to_owned(),
        code: code.to_owned(),
    }
}

/// Asserts a conflict and returns the revisions it reported.
#[track_caller]
fn expect_conflict<T: std::fmt::Debug>(
    outcome: crate::error::Result<T>,
    entity: &'static str,
) -> (i64, i64) {
    match outcome {
        Err(DbError::Conflict {
            entity: found,
            expected,
            actual,
        }) => {
            assert_eq!(
                found, entity,
                "the conflict must name the entity it was about"
            );
            (expected, actual)
        }
        other => panic!("expected a {entity} conflict, got {other:?}"),
    }
}

// ============================================================== the two-window scenario

/// The whole point, written out in the order it happens.
#[test]
fn the_second_window_to_save_is_refused_rather_than_winning() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");

    // Two windows read the same row. Both now hold revision 1.
    let first_window = project
        .db()
        .structure()
        .room_type(lab)
        .expect("read")
        .expect("room type");
    let second_window = project
        .db()
        .structure()
        .room_type(lab)
        .expect("read")
        .expect("room type");
    assert_eq!(first_window.rev, second_window.rev);
    assert_eq!(first_window.rev, 1);

    // The first saves. Ordinary.
    let saved = project
        .db()
        .structure()
        .update_room_type(
            lab,
            Expected::Rev(first_window.rev),
            &room_type_input(north, "Science Laboratory", "LAB"),
        )
        .expect("the first save succeeds");
    assert_eq!(saved.rev, 2);

    let before = project.feed_length();

    // The second saves, still holding revision 1. This is the lost update.
    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(second_window.rev),
        &room_type_input(north, "Chemistry Lab", "CHEM"),
    );
    let (expected, actual) = expect_conflict(outcome, "room type");
    assert_eq!(
        expected, 1,
        "the conflict reports what the caller was holding"
    );
    assert_eq!(actual, 2, "and what the row is actually at");

    // The first writer's edit survives, byte for byte.
    let now = project
        .db()
        .structure()
        .room_type(lab)
        .expect("read")
        .expect("room type");
    assert_eq!(now.name, "Science Laboratory", "the first save must stand");
    assert_eq!(now.code, "LAB");
    assert_eq!(now.rev, 2, "a refused write consumes no revision");
    assert_eq!(
        now.updated_at, saved.updated_at,
        "nor touches the timestamp"
    );

    // And nothing describing the refused write reached the change feed. This is the
    // assertion a write-then-notice implementation fails.
    assert_eq!(
        project.feed_length(),
        before,
        "a conflict must append no change-log entry"
    );
}

#[test]
fn a_conflict_is_distinguishable_from_a_record_that_is_gone() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");

    // Changed by somebody else: a conflict, and the record is still there.
    project
        .db()
        .structure()
        .update_room_type(lab, Expected::Rev(1), &room_type_input(north, "Lab", "LAB"))
        .expect("first save");
    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(1),
        &room_type_input(north, "Other", "LAB"),
    );
    assert!(
        matches!(outcome, Err(DbError::Conflict { .. })),
        "{outcome:?}"
    );

    // Deleted by somebody else: not found, whatever revision is offered. Telling the user
    // their copy is out of date would send them looking for something that has gone.
    project
        .db()
        .structure()
        .delete_room_type(north, lab, Expected::Rev(2))
        .expect("delete");
    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(2),
        &room_type_input(north, "Other", "LAB"),
    );
    assert!(
        matches!(
            outcome,
            Err(DbError::NotFound {
                entity: "room type"
            })
        ),
        "expected NotFound for a row that is gone, got {outcome:?}"
    );
}

#[test]
fn a_delete_at_a_stale_revision_is_refused() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");

    // One window renames it; the other still has the delete dialogue open on revision 1.
    project
        .db()
        .structure()
        .update_room_type(
            lab,
            Expected::Rev(1),
            &room_type_input(north, "Science Laboratory", "LAB"),
        )
        .expect("rename");

    let before = project.feed_length();
    let outcome = project
        .db()
        .structure()
        .delete_room_type(north, lab, Expected::Rev(1));
    let (expected, actual) = expect_conflict(outcome, "room type");
    assert_eq!((expected, actual), (1, 2));

    assert!(
        project
            .db()
            .structure()
            .room_type(lab)
            .expect("read")
            .is_some(),
        "a delete built on what the row used to say must not destroy what it says now"
    );
    assert_eq!(project.feed_length(), before, "and must log nothing");
}

#[test]
fn the_wrong_school_is_refused_before_the_revision_is_even_considered() {
    let mut project = Project::new();
    let north = project.north;
    let south = project.south;
    let lab = project.room_type(north, "Laboratory", "LAB");

    // A correct revision does not buy authority over another school's row, and the error
    // says which boundary held — switching school is the remedy, not reloading.
    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(1),
        &room_type_input(south, "Hijacked", "LAB"),
    );
    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "room type"
            })
        ),
        "expected a wrong-school refusal rather than a conflict, got {outcome:?}"
    );

    let outcome = project
        .db()
        .structure()
        .delete_room_type(south, lab, Expected::Rev(1));
    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "room type"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );
}

#[test]
fn a_successful_save_advances_the_revision_exactly_once() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");

    let mut rev = 1;
    for name in ["A", "B", "C", "D"] {
        let saved = project
            .db()
            .structure()
            .update_room_type(
                lab,
                Expected::Rev(rev),
                &room_type_input(north, name, "LAB"),
            )
            .expect("save");
        rev += 1;
        assert_eq!(
            saved.rev, rev,
            "each save must advance the revision by exactly one"
        );
    }
}

// ======================================================= coverage across the surface
//
// The two-window scenario above is written out once, in full, for one entity. Repeating
// every assertion for twenty would bury the shape of the mechanism in noise — so what the
// rest of the surface is checked for is that it *carries* the precondition at all, which
// is the thing a missed command would get wrong.

/// Every single-record update, each offered a revision that is one behind.
#[test]
fn every_single_record_update_refuses_a_stale_revision() {
    let mut project = Project::new();
    let north = project.north;

    // --- structure
    let school_rev = project
        .db()
        .structure()
        .school(north)
        .expect("read")
        .expect("school")
        .rev;
    let outcome = project.db().structure().update_school(
        north,
        Expected::Rev(school_rev + 7),
        &SchoolInput {
            name: "Renamed".to_owned(),
            code: None,
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
            working_days: WorkingDays::MONDAY_TO_FRIDAY,
        },
    );
    expect_conflict(outcome, "school");

    let campus = project.campus(north, "Main", "MAIN");
    let outcome = project.db().structure().update_campus(
        campus,
        Expected::Rev(99),
        &CampusInput {
            school_id: north,
            name: "Moved".to_owned(),
            code: "MAIN".to_owned(),
            address: None,
        },
    );
    expect_conflict(outcome, "campus");

    let lab = project.room_type(north, "Laboratory", "LAB");
    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(99),
        &room_type_input(north, "Other", "LAB"),
    );
    expect_conflict(outcome, "room type");

    let year = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: north,
            name: "2027/2028".to_owned(),
            starts_on: "2027-09-01".to_owned(),
            ends_on: "2028-07-14".to_owned(),
        })
        .expect("academic year")
        .id;
    let outcome = project.db().structure().update_academic_year(
        year,
        Expected::Rev(99),
        &AcademicYearInput {
            school_id: north,
            name: "2028/2029".to_owned(),
            starts_on: "2028-09-01".to_owned(),
            ends_on: "2029-07-14".to_owned(),
        },
    );
    expect_conflict(outcome, "academic year");

    // --- people
    let level = project
        .db()
        .people()
        .create_year_level(&YearLevelInput {
            school_id: north,
            name: "Grade 8".to_owned(),
            ordinal: 8,
        })
        .expect("year level")
        .id;
    let outcome = project.db().people().update_year_level(
        level,
        Expected::Rev(99),
        &YearLevelInput {
            school_id: north,
            name: "Year 9".to_owned(),
            ordinal: 9,
        },
    );
    expect_conflict(outcome, "year level");

    let teacher = project.teacher(north, "Okafor");
    let outcome = project.db().people().update_teacher(
        teacher,
        Expected::Rev(99),
        &TeacherInput {
            school_id: north,
            code: None,
            given_name: "Ada".to_owned(),
            family_name: "Renamed".to_owned(),
            email: None,
            home_campus_id: None,
            fte_percent: 100,
            max_periods_per_day: None,
            max_periods_per_cycle: None,
            max_consecutive_periods: None,
            notes: None,
            is_active: true,
        },
    );
    expect_conflict(outcome, "teacher");

    let student = project.student(north, "Bellweather");
    let outcome = project.db().people().update_student(
        student,
        Expected::Rev(99),
        &StudentInput {
            school_id: north,
            code: None,
            given_name: "Cass".to_owned(),
            family_name: "Renamed".to_owned(),
            email: None,
            year_level_id: None,
            campus_id: None,
            is_active: true,
        },
    );
    expect_conflict(outcome, "student");

    // --- curriculum, the tables this stage adds
    let department = project
        .db()
        .curriculum()
        .create_department(&DepartmentInput {
            school_id: north,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("department")
        .id;
    let outcome = project.db().curriculum().update_department(
        department,
        Expected::Rev(99),
        &DepartmentInput {
            school_id: north,
            name: "Renamed".to_owned(),
            code: None,
            description: None,
        },
    );
    expect_conflict(outcome, "department");

    let subject = project
        .db()
        .curriculum()
        .create_subject(&SubjectInput {
            school_id: north,
            department_id: None,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("subject")
        .id;
    let outcome = project.db().curriculum().update_subject(
        subject,
        Expected::Rev(99),
        &SubjectInput {
            school_id: north,
            department_id: None,
            name: "Renamed".to_owned(),
            code: None,
            description: None,
        },
    );
    expect_conflict(outcome, "subject");
}

/// Every single-record delete, each offered a revision that is wrong.
#[test]
fn every_single_record_delete_refuses_a_stale_revision() {
    let mut project = Project::new();
    let north = project.north;

    let campus = project.campus(north, "Main", "MAIN");
    expect_conflict(
        project
            .db()
            .structure()
            .delete_campus(north, campus, Expected::Rev(99)),
        "campus",
    );

    let lab = project.room_type(north, "Laboratory", "LAB");
    expect_conflict(
        project
            .db()
            .structure()
            .delete_room_type(north, lab, Expected::Rev(99)),
        "room type",
    );

    let year = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: north,
            name: "2027/2028".to_owned(),
            starts_on: "2027-09-01".to_owned(),
            ends_on: "2028-07-14".to_owned(),
        })
        .expect("academic year")
        .id;
    expect_conflict(
        project
            .db()
            .structure()
            .delete_academic_year(north, year, Expected::Rev(99)),
        "academic year",
    );

    let level = project
        .db()
        .people()
        .create_year_level(&YearLevelInput {
            school_id: north,
            name: "Grade 8".to_owned(),
            ordinal: 8,
        })
        .expect("year level")
        .id;
    expect_conflict(
        project
            .db()
            .people()
            .delete_year_level(north, level, Expected::Rev(99)),
        "year level",
    );

    let teacher = project.teacher(north, "Okafor");
    expect_conflict(
        project
            .db()
            .people()
            .delete_teacher(north, teacher, Expected::Rev(99)),
        "teacher",
    );

    let student = project.student(north, "Bellweather");
    expect_conflict(
        project
            .db()
            .people()
            .delete_student(north, student, Expected::Rev(99)),
        "student",
    );

    let department = project
        .db()
        .curriculum()
        .create_department(&DepartmentInput {
            school_id: north,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("department")
        .id;
    expect_conflict(
        project
            .db()
            .curriculum()
            .delete_department(north, department, Expected::Rev(99)),
        "department",
    );

    let subject = project
        .db()
        .curriculum()
        .create_subject(&SubjectInput {
            school_id: north,
            department_id: None,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("subject")
        .id;
    expect_conflict(
        project
            .db()
            .curriculum()
            .delete_subject(north, subject, Expected::Rev(99)),
        "subject",
    );
}

/// A delete that clears children must check the parent's revision **before** clearing any
/// of them, or a refusal leaves the project part-way through.
#[test]
fn a_refused_delete_clears_none_of_the_rows_it_would_have_taken() {
    let mut project = Project::new();
    let north = project.north;
    let department = project
        .db()
        .curriculum()
        .create_department(&DepartmentInput {
            school_id: north,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("department")
        .id;
    let teacher = project.teacher(north, "Okafor");
    project
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, department)
        .expect("assign");

    // A teacher with a department association and a stale revision. The delete is refused
    // for the *reference*, which is checked first.
    let outcome = project
        .db()
        .people()
        .delete_teacher(north, teacher, Expected::Rev(99));
    assert!(outcome.is_err(), "{outcome:?}");

    assert_eq!(
        project
            .db()
            .curriculum()
            .departments_of_teacher(teacher)
            .expect("list")
            .len(),
        1,
        "a refused delete must not have cleared the association on the way to refusing"
    );
    assert!(
        project
            .db()
            .people()
            .teacher(teacher)
            .expect("read")
            .is_some(),
        "and the teacher is still there"
    );
}

// ========================================================= the specialised contracts

/// A membership's own revision, not the teacher's. The row that changes is the one with
/// the precondition on it.
#[test]
fn activating_a_membership_checks_the_memberships_revision() {
    let mut project = Project::new();
    let north = project.north;
    let south = project.south;
    let teacher = project.teacher(north, "Okafor");
    project
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: teacher,
            school_id: south,
            is_active: true,
        })
        .expect("employ at Southmoor");

    let membership = project
        .db()
        .membership()
        .teacher_memberships(teacher)
        .expect("memberships")
        .into_iter()
        .find(|row| row.school_id == south)
        .expect("Southmoor's membership");

    // One window marks her as left.
    project
        .db()
        .membership()
        .set_teacher_membership_active(south, teacher, Expected::Rev(membership.rev), false)
        .expect("deactivate");

    // The other, still holding the old revision, tries to mark her as teaching.
    let outcome = project.db().membership().set_teacher_membership_active(
        south,
        teacher,
        Expected::Rev(membership.rev),
        true,
    );
    expect_conflict(outcome, "school membership");

    let now = project
        .db()
        .membership()
        .teacher_memberships(teacher)
        .expect("memberships")
        .into_iter()
        .find(|row| row.school_id == south)
        .expect("membership");
    assert!(!now.is_active, "the first window's decision must stand");
}

/// An enrolment's own revision, for the same reason.
#[test]
fn editing_a_spell_checks_the_spells_revision() {
    let mut project = Project::new();
    let north = project.north;
    let student = project.student(north, "Bellweather");

    let spell = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read")
        .expect("the home spell create_student wrote");

    let input = StudentEnrolmentInput {
        student_id: student,
        school_id: north,
        admission_no: Some("A1".to_owned()),
        year_level_id: None,
        campus_id: None,
        status: EnrolmentStatus::Enrolled,
        enrolled_on: None,
        left_on: None,
    };
    project
        .db()
        .membership()
        .update_enrolment(north, spell.id, Expected::Rev(spell.rev), &input)
        .expect("the first save succeeds");

    let outcome = project.db().membership().update_enrolment(
        north,
        spell.id,
        Expected::Rev(spell.rev),
        &StudentEnrolmentInput {
            admission_no: Some("A2".to_owned()),
            ..input
        },
    );
    expect_conflict(outcome, "enrolment");

    let now = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read")
        .expect("spell");
    assert_eq!(
        now.admission_no.as_deref(),
        Some("A1"),
        "the first window's admission number must stand"
    );
}

/// A departure also rewrites the spell, so it carries the spell's revision.
#[test]
fn recording_a_departure_checks_the_spells_revision() {
    let mut project = Project::new();
    let north = project.north;
    let student = project.student(north, "Bellweather");
    let spell = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read")
        .expect("spell");

    let outcome = project.db().membership().leave_school(
        north,
        student,
        Expected::Rev(spell.rev + 5),
        Some("2027-07-20"),
    );
    expect_conflict(outcome, "enrolment");

    let now = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read");
    assert!(
        now.is_some(),
        "a refused departure must leave the student enrolled"
    );
}

/// A transfer stays **one** call across two schools, with one precondition: the leaving
/// spell, which is the only existing row it rewrites. ADR 0021 §7 refused to split it, and
/// ADR 0027 §4 refuses to split it now merely to give each half a revision.
#[test]
fn a_transfer_is_still_atomic_and_still_takes_one_precondition() {
    let mut project = Project::new();
    let north = project.north;
    let south = project.south;
    let student = project.student(north, "Bellweather");
    let leaving = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read")
        .expect("spell");

    let arriving = StudentEnrolmentInput {
        student_id: student,
        school_id: south,
        admission_no: None,
        year_level_id: None,
        campus_id: None,
        status: EnrolmentStatus::Enrolled,
        enrolled_on: Some("2027-09-01".to_owned()),
        left_on: None,
    };

    // A stale precondition refuses the whole thing, both halves.
    let outcome = project.db().membership().transfer_student(
        north,
        south,
        student,
        Expected::Rev(leaving.rev + 3),
        &arriving,
    );
    expect_conflict(outcome, "enrolment");

    let still_here = project
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read");
    assert!(
        still_here.is_some(),
        "the leaving half must not have been applied"
    );
    assert!(
        project
            .db()
            .membership()
            .current_enrolment_at(south, student)
            .expect("read")
            .is_none(),
        "nor the arriving half: a student must never be enrolled twice or nowhere"
    );

    // With the right revision it goes through, and both halves land together.
    project
        .db()
        .membership()
        .transfer_student(north, south, student, Expected::Rev(leaving.rev), &arriving)
        .expect("transfer");
    assert!(
        project
            .db()
            .membership()
            .current_enrolment_at(north, student)
            .expect("read")
            .is_none()
    );
    assert!(
        project
            .db()
            .membership()
            .current_enrolment_at(south, student)
            .expect("read")
            .is_some()
    );
    // One student, as ever.
    assert_eq!(
        project
            .db()
            .membership()
            .student_enrolments(student)
            .expect("history")
            .len(),
        2,
        "a transfer keeps the previous spell rather than rewriting it"
    );
}

/// A relationship has no mutable domain state, so a revision would be a precondition on
/// nothing. Its contract is duplicate-on-add and not-found-on-remove, and neither outcome
/// is a lost update: there is no previous value to lose.
#[test]
fn a_relationship_carries_existence_rather_than_a_revision() {
    let mut project = Project::new();
    let north = project.north;
    let teacher = project.teacher(north, "Okafor");
    let department = project
        .db()
        .curriculum()
        .create_department(&DepartmentInput {
            school_id: north,
            name: "Mathematics".to_owned(),
            code: None,
            description: None,
        })
        .expect("department")
        .id;

    // Two windows both add it. The second is a duplicate, not a silent re-write.
    project
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, department)
        .expect("the first add succeeds");
    let outcome = project
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, department);
    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { .. })),
        "expected a duplicate rather than a conflict, got {outcome:?}"
    );

    // Two windows both remove it. The second finds nothing, which is the truth.
    let held = project
        .db()
        .curriculum()
        .departments_of_teacher(teacher)
        .expect("list");
    let rev = held[0].association.rev;
    project
        .db()
        .curriculum()
        .remove_teacher_department(north, teacher, department, Expected::Rev(rev))
        .expect("the first removal succeeds");
    let outcome = project.db().curriculum().remove_teacher_department(
        north,
        teacher,
        department,
        Expected::Rev(rev),
    );
    assert!(
        matches!(
            outcome,
            Err(DbError::NotFound {
                entity: "department association"
            })
        ),
        "expected NotFound rather than a conflict, got {outcome:?}"
    );
}

/// Availability is the documented exception: one cell, set by direct manipulation in a
/// grid, last write wins. A precondition on a grid cell would make a two-window conflict
/// unresolvable without a reload the user never asked for, and nothing a cell holds is a
/// value somebody composed.
#[test]
fn an_availability_cell_is_last_write_wins_by_design() {
    use crate::repo::availability::{
        AvailabilityEntity, AvailabilityRuleInput, AvailabilityStatus,
    };
    use crate::repo::time_model::{CycleInput, CycleKind};

    let mut project = Project::new();
    let north = project.north;
    let teacher = project.teacher(north, "Okafor");
    let cycle = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: north,
            name: "Rotation".to_owned(),
            kind: CycleKind::Custom,
            day_count: 2,
            week_count: 1,
            is_default: true,
        })
        .expect("cycle")
        .id;
    let day = project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("days")
        .first()
        .expect("a custom cycle has days")
        .id;

    let cell = |status| AvailabilityRuleInput {
        school_id: north,
        entity_kind: AvailabilityEntity::Teacher,
        entity_id: teacher,
        timeslot_id: None,
        cycle_day_id: Some(day),
        period_id: None,
        term_id: None,
        status,
        weight: None,
        reason: None,
        effective_from: None,
        effective_to: None,
    };

    let first = project
        .db()
        .availability()
        .set(&cell(AvailabilityStatus::Unavailable))
        .expect("set");
    let second = project
        .db()
        .availability()
        .set(&cell(AvailabilityStatus::Discouraged))
        .expect("the second set is not refused");

    assert_eq!(
        first.id, second.id,
        "the same scope is one rule, updated in place rather than duplicated"
    );
    assert_eq!(second.rev, first.rev + 1);
    assert_eq!(
        second.status,
        AvailabilityStatus::Discouraged,
        "last write wins, deliberately — ADR 0027 §4"
    );
}

// ====================================================== the mechanism, not the outcome

/// `Expected::Any` must mean *no predicate*, not *any value that happens to match*.
///
/// This is what the cascades inside another operation's transaction rely on: they are
/// writing rows whose revisions no user ever saw, and a helper that quietly compared
/// against 1 would break the first time a row had been edited twice.
#[test]
fn an_unchecked_write_succeeds_whatever_the_row_is_at() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");

    for name in ["A", "B", "C"] {
        let rev = project
            .db()
            .structure()
            .room_type(lab)
            .expect("read")
            .expect("room type")
            .rev;
        project
            .db()
            .structure()
            .update_room_type(
                lab,
                Expected::Rev(rev),
                &room_type_input(north, name, "LAB"),
            )
            .expect("save");
    }

    // Revision 4 by now, and an unchecked write still lands.
    project
        .db()
        .structure()
        .update_room_type(lab, Expected::Any, &room_type_input(north, "D", "LAB"))
        .expect("an unchecked write must not depend on the revision");
    assert_eq!(
        project
            .db()
            .structure()
            .room_type(lab)
            .expect("read")
            .expect("room type")
            .rev,
        5
    );
}

/// A constraint failure must leave the revision alone too, which is the same guarantee
/// from the other direction: nothing is half-written.
#[test]
fn a_rejected_value_consumes_no_revision_either() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");
    project.room_type(north, "Gymnasium", "GYM");

    let outcome = project.db().structure().update_room_type(
        lab,
        Expected::Rev(1),
        // "GYM" is taken, so the statement aborts.
        &room_type_input(north, "Laboratory", "GYM"),
    );
    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { .. })),
        "{outcome:?}"
    );

    let now = project
        .db()
        .structure()
        .room_type(lab)
        .expect("read")
        .expect("room type");
    assert_eq!(now.code, "LAB");
    assert_eq!(
        now.rev, 1,
        "an aborted statement must leave the revision alone"
    );
}

/// Two conflicts in a row must both report the same current revision — a conflict that
/// advanced it would make the second window's reload show a version nobody wrote.
#[test]
fn repeated_conflicts_report_the_same_revision() {
    let mut project = Project::new();
    let north = project.north;
    let lab = project.room_type(north, "Laboratory", "LAB");
    project
        .db()
        .structure()
        .update_room_type(lab, Expected::Rev(1), &room_type_input(north, "Lab", "LAB"))
        .expect("save");

    for _ in 0..3 {
        let outcome = project.db().structure().update_room_type(
            lab,
            Expected::Rev(1),
            &room_type_input(north, "Other", "LAB"),
        );
        let (expected, actual) = expect_conflict(outcome, "room type");
        assert_eq!((expected, actual), (1, 2));
    }
}
