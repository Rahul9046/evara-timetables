//! Stage 0B: one person, several schools — and what sharing must never permit.
//!
//! # What these tests are for
//!
//! `V13` separated identity from belonging. Three things therefore have to be proved
//! rather than argued:
//!
//! 1. **One person stays one person.** A teacher at two schools, a student who transfers,
//!    a resource lent across a trust: the identifier never changes, the record is never
//!    duplicated, and both schools are looking at the same row. That is the whole reason
//!    the stage exists — two rows for one teacher means two availability records and two
//!    independent timetables, the failure ADR 0014 rejected for rooms.
//! 2. **Sharing is not a loophole.** Holding a shared person's identifier is now
//!    completely ordinary, and must still not be authority to touch another school's
//!    employment, enrolment, campuses or deletions.
//! 3. **Nothing an existing project held moved.** The backfill writes relationships for
//!    rows that already exist; it must not rewrite one of them or consume a revision.
//!
//! # The schema is tested as well as the repository
//!
//! Several rules are a trigger pair *and* a repository check, which `V12` explains: the
//! repository can say which field is wrong and name the remedy, the trigger is what makes
//! the invariant true for a future repository, the CLI, an import or a hand-repair. Both
//! halves are asserted here — the repository through its typed error, the trigger by
//! writing the row underneath it.

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::availability::{AvailabilityEntity, AvailabilityRuleInput, AvailabilityStatus};
use crate::repo::membership::{EnrolmentStatus, StudentEnrolmentInput, TeacherSchoolInput};
use crate::repo::people::{GroupKind, StudentGroupInput, StudentInput, TeacherInput};
use crate::repo::structure::{CampusInput, ResourceInput, SchoolInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// The SQL under test, read from the migration itself so the two cannot drift apart.
const V13_SQL: &str =
    include_str!("../migrations/V13__school_membership_and_shared_identities.sql");

// ================================================================== the trust fixture

/// Two schools in one project, each with a campus and a year level.
///
/// Named rather than numbered because every assertion below is about *which* school, and
/// `school_a`/`school_b` makes a wrong expectation read as plausible.
struct Trust {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
    north_campus: Uuid,
    south_campus: Uuid,
    north_year: Uuid,
    south_year: Uuid,
}

impl Trust {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let settings = dir.path().join("settings.sqlite");
        let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
        let path = dir.path().join(format!("trust.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");

        let mut this = Self {
            _dir: dir,
            workspace,
            north: Uuid::nil(),
            south: Uuid::nil(),
            north_campus: Uuid::nil(),
            south_campus: Uuid::nil(),
            north_year: Uuid::nil(),
            south_year: Uuid::nil(),
        };
        this.north = this.school("Northgate Secondary");
        this.south = this.school("Southmoor College");
        this.north_campus = this.campus(this.north, "Northgate Main", "NGM");
        this.south_campus = this.campus(this.south, "Southmoor Riverside", "SMR");
        this.north_year = this.year_level(this.north, "Grade 8", 8);
        this.south_year = this.year_level(this.south, "Grade 8", 8);
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

    fn year_level(&mut self, school: Uuid, name: &str, ordinal: i64) -> Uuid {
        self.db()
            .people()
            .create_year_level(&crate::repo::people::YearLevelInput {
                school_id: school,
                name: name.to_owned(),
                ordinal,
            })
            .expect("create year level")
            .id
    }

    /// A cycle with one day, which is the cheapest time an availability rule can name.
    fn cycle_day(&mut self, school: Uuid) -> Uuid {
        let cycle = self
            .db()
            .time_model()
            .create_cycle(&crate::repo::time_model::CycleInput {
                school_id: school,
                name: "Rotation".to_owned(),
                kind: crate::repo::time_model::CycleKind::Custom,
                day_count: 2,
                week_count: 1,
                is_default: true,
            })
            .expect("cycle")
            .id;
        // `create_cycle` lays the days out itself, so take the first rather than adding
        // another at position 1.
        self.db()
            .time_model()
            .cycle_days(cycle)
            .expect("cycle days")
            .first()
            .expect("a custom cycle has days")
            .id
    }

    /// A teacher created in one school, with no home campus unless one is given.
    fn teacher(&mut self, school: Uuid, family: &str, home_campus: Option<Uuid>) -> Uuid {
        self.db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: school,
                code: None,
                given_name: "Ada".to_owned(),
                family_name: family.to_owned(),
                email: None,
                home_campus_id: home_campus,
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

    fn group(&mut self, school: Uuid, name: &str, size: Option<i64>) -> Uuid {
        self.db()
            .people()
            .create_student_group(&StudentGroupInput {
                school_id: school,
                name: name.to_owned(),
                code: None,
                kind: GroupKind::Cohort,
                year_level_id: None,
                parent_group_id: None,
                size,
                campus_id: None,
            })
            .expect("create group")
            .id
    }

    fn resource(&mut self, school: Uuid, name: &str, code: &str, campus: Option<Uuid>) -> Uuid {
        self.db()
            .structure()
            .create_resource(&ResourceInput {
                school_id: school,
                campus_id: campus,
                name: name.to_owned(),
                code: code.to_owned(),
                quantity: 1,
            })
            .expect("create resource")
            .id
    }
}

/// An enrolment naming a school, with nothing else set.
fn enrolment(student: Uuid, school: Uuid) -> StudentEnrolmentInput {
    StudentEnrolmentInput {
        student_id: student,
        school_id: school,
        admission_no: None,
        year_level_id: None,
        campus_id: None,
        status: EnrolmentStatus::Enrolled,
        enrolled_on: None,
        left_on: None,
    }
}

// ============================================================== teacher membership

/// The requirement the stage exists for: one person, two employers, one identifier.
#[test]
fn one_teacher_belongs_to_two_schools_as_one_record() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);

    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("Southmoor employs her too");

    let northern = trust.db().people().teachers(north).expect("Northgate list");
    let southern = trust.db().people().teachers(south).expect("Southmoor list");

    assert_eq!(northern.len(), 1, "once in Northgate's list");
    assert_eq!(southern.len(), 1, "once in Southmoor's list");
    assert_eq!(
        northern[0].id, southern[0].id,
        "the same identifier in both: one person, not two records"
    );
    assert_eq!(northern[0].id, ada);

    let rows: i64 = trust
        .db()
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM teacher", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("count teachers");
    assert_eq!(rows, 1, "no teacher row was duplicated to achieve this");

    let memberships = trust
        .db()
        .membership()
        .teacher_memberships(ada)
        .expect("her memberships");
    assert_eq!(memberships.len(), 2, "membership is the complete answer");
}

/// `teacher.school_id` is the home school, not the membership set.
#[test]
fn the_home_school_column_is_not_the_answer_to_which_schools() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    let record = trust
        .db()
        .people()
        .teacher(ada)
        .expect("read")
        .expect("there");
    assert_eq!(
        record.school_id, north,
        "the column still names only the school that created her"
    );

    // And the listing is not built on it: Southmoor's list is populated entirely by
    // membership, and a query over the column would return nothing here.
    assert_eq!(
        trust.db().people().teachers(south).expect("list").len(),
        1,
        "Southmoor sees her because of the membership, not the column"
    );
}

/// Employment is per school. Leaving one is not leaving the other.
#[test]
fn a_membership_is_deactivated_without_touching_the_person_or_the_other_school() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    trust
        .db()
        .membership()
        .set_teacher_membership_active(south, ada, Expected::Any, false)
        .expect("she has left Southmoor");

    assert!(
        trust
            .db()
            .membership()
            .teaches_at(north, ada)
            .expect("north"),
        "Northgate still employs her"
    );
    assert!(
        !trust
            .db()
            .membership()
            .teaches_at(south, ada)
            .expect("south"),
        "Southmoor does not"
    );

    let person = trust
        .db()
        .people()
        .teacher(ada)
        .expect("read")
        .expect("there");
    assert!(
        person.is_active,
        "the project-level flag is untouched: she is still a person in this project"
    );
    assert_eq!(
        person.rev, 1,
        "deactivating a membership consumes no revision on the teacher"
    );

    // She is still in Southmoor's list, marked rather than hidden -- the ADR 0018 rule.
    assert_eq!(
        trust.db().people().teachers(south).expect("list").len(),
        1,
        "an inactive membership is a state, not a filter on the truth"
    );
}

/// The conjunction, which is the only thing that may decide availability for new work.
#[test]
fn eligibility_needs_both_flags_not_either_one() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    // Archived at project level, still employed by both on paper.
    let mut input = teacher_input_from(&mut trust, ada);
    input.is_active = false;
    trust
        .db()
        .people()
        .update_teacher(ada, Expected::Any, &input)
        .expect("archive her");

    for school in [north, south] {
        assert!(
            !trust
                .db()
                .membership()
                .teaches_at(school, ada)
                .expect("ask"),
            "an archived person is available to nobody, whatever the membership says"
        );
    }

    let view = trust
        .db()
        .membership()
        .school_teachers(south)
        .expect("Southmoor's view");
    assert_eq!(view.len(), 1);
    assert!(
        view[0].membership.is_active,
        "Southmoor's own record of employment is unchanged by a project-level edit"
    );
    assert!(
        !view[0].is_effectively_active(),
        "and the conjunction is what a screen must read"
    );
}

/// Campus eligibility may span schools — but only schools she belongs to.
#[test]
fn campus_eligibility_spans_member_schools_and_stops_at_the_boundary() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let ada = trust.teacher(north, "Okafor", None);

    trust
        .db()
        .membership()
        .assign_teacher_campus(north, ada, north_campus)
        .expect("her own school's campus");

    // Southmoor's campus, before Southmoor employs her.
    let refused = trust
        .db()
        .membership()
        .assign_teacher_campus(south, ada, south_campus)
        .expect_err("not a member yet");
    assert!(
        matches!(refused, DbError::Invalid { ref message } if message.contains("Add this teacher")),
        "the message has to name the remedy, which the trigger cannot: {refused:?}"
    );

    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("now she is");
    trust
        .db()
        .membership()
        .assign_teacher_campus(south, ada, south_campus)
        .expect("and now the campus is hers to have");

    let campuses = trust.db().membership().teacher_campuses(ada).expect("read");
    assert_eq!(campuses.len(), 2, "eligible at a campus of each school");
    assert!(campuses.contains(&north_campus) && campuses.contains(&south_campus));
}

/// The same rule, asserted against the schema rather than the repository.
#[test]
fn the_schema_itself_refuses_eligibility_without_membership() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south_campus = trust.south_campus;
    let ada = trust.teacher(north, "Okafor", None);

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO teacher_campus (id, teacher_id, campus_id, created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-10-14T09:00:00.000Z', '2026-10-14T09:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                ada.to_string(),
                south_campus.to_string(),
            ),
        )?;
        Ok(())
    });

    assert!(
        refused.is_err(),
        "a hand-written row must not be able to assert what the repository refuses"
    );
}

/// One school may not reach into another's employment of a shared person.
#[test]
fn a_school_cannot_act_on_another_schools_membership() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let south_campus = trust.south_campus;
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    // Northgate holds her identifier -- that is now ordinary -- and it is not authority.
    let wrong_campus = trust
        .db()
        .membership()
        .assign_teacher_campus(north, ada, south_campus)
        .expect_err("Southmoor's campus is not Northgate's to give");
    assert!(
        matches!(wrong_campus, DbError::WrongSchool { .. }),
        "{wrong_campus:?}"
    );

    let wrong_removal = trust
        .db()
        .membership()
        .remove_teacher_campus(north, ada, south_campus)
        .expect_err("nor to take away");
    assert!(
        matches!(wrong_removal, DbError::WrongSchool { .. }),
        "{wrong_removal:?}"
    );
}

/// Withdrawing one school's employment leaves everything else exactly as it was.
#[test]
fn removing_one_membership_preserves_the_other_and_the_person() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, ada, north_campus)
        .expect("northern campus");
    trust
        .db()
        .membership()
        .assign_teacher_campus(south, ada, south_campus)
        .expect("southern campus");

    trust
        .db()
        .membership()
        .remove_teacher_from_school(south, ada)
        .expect("Southmoor is finished with her");

    let person = trust.db().people().teacher(ada).expect("read");
    assert!(person.is_some(), "the person is not deleted by letting go");
    assert_eq!(
        person.expect("there").rev,
        1,
        "and no revision of hers was consumed"
    );

    let memberships = trust
        .db()
        .membership()
        .teacher_memberships(ada)
        .expect("read");
    assert_eq!(memberships.len(), 1, "Northgate's survives");
    assert_eq!(memberships[0].school_id, north);

    let campuses = trust.db().membership().teacher_campuses(ada).expect("read");
    assert_eq!(
        campuses,
        vec![north_campus],
        "the southern eligibility went with the southern membership, the northern did not"
    );
    assert_eq!(
        trust.db().people().teachers(south).expect("list").len(),
        0,
        "Southmoor no longer lists her"
    );
}

/// The home school's own membership is not removable, because nothing would be left.
#[test]
fn the_home_school_membership_cannot_be_withdrawn() {
    let mut trust = Trust::new();
    let north = trust.north;
    let ada = trust.teacher(north, "Okafor", None);

    let refused = trust
        .db()
        .membership()
        .remove_teacher_from_school(north, ada)
        .expect_err("that would leave her belonging to nobody");
    assert!(
        matches!(refused, DbError::Invalid { ref message } if message.contains("created in this school")),
        "{refused:?}"
    );
}

/// Availability is the teacher's, and sharing her does not disturb it.
#[test]
fn sharing_a_teacher_preserves_the_availability_she_already_had() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    let day_a = trust.cycle_day(north);

    trust
        .db()
        .availability()
        .set(&AvailabilityRuleInput {
            school_id: north,
            entity_kind: AvailabilityEntity::Teacher,
            entity_id: ada,
            timeslot_id: None,
            cycle_day_id: Some(day_a),
            period_id: None,
            term_id: None,
            status: AvailabilityStatus::Unavailable,
            weight: None,
            reason: Some("Fridays off".to_owned()),
            effective_from: None,
            effective_to: None,
        })
        .expect("a rule about her");

    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");
    trust
        .db()
        .membership()
        .set_teacher_membership_active(south, ada, Expected::Any, false)
        .expect("and un-shared again, by deactivation");
    trust
        .db()
        .membership()
        .remove_teacher_from_school(south, ada)
        .expect("and removed outright");

    let rules = trust
        .db()
        .availability()
        .rules_for(north, AvailabilityEntity::Teacher, ada)
        .expect("her rules");
    assert_eq!(
        rules.len(),
        1,
        "every membership operation left her availability alone"
    );
    assert_eq!(rules[0].reason.as_deref(), Some("Fridays off"));
}

/// Deleting the person is a different operation from letting go of them.
#[test]
fn a_shared_teacher_cannot_be_deleted_by_the_school_that_created_her() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    let refused = trust
        .db()
        .people()
        .delete_teacher(north, ada, Expected::Any)
        .expect_err("Southmoor is timetabling around her");
    assert!(
        matches!(refused, DbError::StillReferenced { .. }),
        "{refused:?}"
    );
    assert!(
        trust.db().people().teacher(ada).expect("read").is_some(),
        "and the refusal destroyed nothing on its way to refusing"
    );

    // The way forward exists, and the other school can see it.
    trust
        .db()
        .membership()
        .remove_teacher_from_school(south, ada)
        .expect("Southmoor withdraws");
    trust
        .db()
        .people()
        .delete_teacher(north, ada, Expected::Any)
        .expect("and now the home school may");
    assert!(trust.db().people().teacher(ada).expect("read").is_none());
}

/// A school that merely employs a shared teacher may not delete the person.
#[test]
fn a_borrowing_school_cannot_delete_a_shared_teacher() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");

    let refused = trust
        .db()
        .people()
        .delete_teacher(south, ada, Expected::Any)
        .expect_err("not Southmoor's person to destroy");
    assert!(
        matches!(refused, DbError::WrongSchool { .. }),
        "{refused:?}"
    );
}

/// A teacher may be based at a campus of any school she belongs to.
#[test]
fn a_home_campus_may_belong_to_any_member_school() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let south_campus = trust.south_campus;
    let ada = trust.teacher(north, "Okafor", None);

    let mut input = teacher_input_from(&mut trust, ada);
    input.home_campus_id = Some(south_campus);
    let refused = trust
        .db()
        .people()
        .update_teacher(ada, Expected::Any, &input)
        .expect_err("she does not work there");
    assert!(
        matches!(refused, DbError::Invalid { .. } | DbError::Internal(_)),
        "{refused:?}"
    );

    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("now she does");
    let updated = trust
        .db()
        .people()
        .update_teacher(ada, Expected::Any, &input)
        .expect("and Southmoor's campus is a place she can be based");
    assert_eq!(updated.home_campus_id, Some(south_campus));
}

// ================================================================= student enrolment

/// Two schools, one student, one identifier.
#[test]
fn a_student_is_enrolled_in_two_schools_concurrently_as_one_record() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");

    trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, south))
        .expect("Southmoor enrols her too");

    let northern = trust
        .db()
        .membership()
        .school_students(north)
        .expect("north roll");
    let southern = trust
        .db()
        .membership()
        .school_students(south)
        .expect("south roll");
    assert_eq!(northern.len(), 1);
    assert_eq!(southern.len(), 1);
    assert_eq!(
        northern[0].student.id, southern[0].student.id,
        "one person on two rolls"
    );
    assert_ne!(
        northern[0].enrolment.id, southern[0].enrolment.id,
        "two spells, which is what carries the school-specific placement"
    );

    let rows: i64 = trust
        .db()
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM student", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("count students");
    assert_eq!(rows, 1, "no student row was duplicated");
}

/// Two current spells at one school is not a thing that can be true.
#[test]
fn a_second_current_enrolment_at_the_same_school_is_refused() {
    let mut trust = Trust::new();
    let north = trust.north;
    let cass = trust.student(north, "Pupilsen");

    let refused = trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, north))
        .expect_err("she is already here");
    assert!(
        matches!(refused, DbError::DuplicateValue { .. }),
        "{refused:?}"
    );

    // And the schema refuses it underneath the repository too.
    let raw = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO student_school_enrolment
                 (id, student_id, school_id, status, created_at, updated_at, rev)
             VALUES (?, ?, ?, 'ENROLLED', '2026-10-14T09:00:00.000Z',
                     '2026-10-14T09:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                cass.to_string(),
                north.to_string(),
            ),
        )?;
        Ok(())
    });
    assert!(raw.is_err(), "the index is what makes this impossible");
}

/// A transfer keeps the person and keeps the history.
#[test]
fn a_transfer_preserves_the_identity_and_the_previous_spell() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_year, south_year) = (trust.north_year, trust.south_year);
    let cass = trust.student(north, "Pupilsen");

    // Placed at Northgate first, so there is something to preserve.
    let northern = trust
        .db()
        .membership()
        .current_enrolment_at(north, cass)
        .expect("read")
        .expect("her Northgate spell");
    trust
        .db()
        .membership()
        .update_enrolment(
            north,
            northern.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("NG-1001".to_owned()),
                year_level_id: Some(north_year),
                ..enrolment(cass, north)
            },
        )
        .expect("place her");

    let arriving = StudentEnrolmentInput {
        admission_no: Some("SM-77".to_owned()),
        year_level_id: Some(south_year),
        enrolled_on: Some("2027-01-05".to_owned()),
        ..enrolment(cass, south)
    };
    trust
        .db()
        .membership()
        .transfer_student(north, south, cass, Expected::Any, &arriving)
        .expect("she moves to Southmoor");

    assert!(
        trust.db().people().student(cass).expect("read").is_some(),
        "the same student, still there"
    );

    let history = trust
        .db()
        .membership()
        .student_enrolments(cass)
        .expect("history");
    assert_eq!(history.len(), 2, "the old spell was kept, not rewritten");

    let old = history
        .iter()
        .find(|row| row.school_id == north)
        .expect("Northgate's spell");
    assert_eq!(old.status, EnrolmentStatus::Left);
    assert_eq!(
        old.admission_no.as_deref(),
        Some("NG-1001"),
        "a past spell keeps the number the school gave her"
    );
    assert_eq!(
        old.year_level_id,
        Some(north_year),
        "and the year she was in when she left"
    );
    assert_eq!(old.left_on.as_deref(), Some("2027-01-05"));

    let new = history
        .iter()
        .find(|row| row.school_id == south)
        .expect("Southmoor's spell");
    assert_eq!(new.status, EnrolmentStatus::Enrolled);
    assert_eq!(new.year_level_id, Some(south_year));

    assert_eq!(
        trust
            .db()
            .membership()
            .school_students(north)
            .expect("roll")
            .len(),
        0,
        "Northgate's current roll no longer has her"
    );
    assert_eq!(
        trust
            .db()
            .people()
            .students(north)
            .expect("history list")
            .len(),
        1,
        "but Northgate still holds her history, which is the point of keeping the spell"
    );
}

/// Leaving and coming back is two spells, not a refused duplicate.
#[test]
fn a_student_may_leave_and_rejoin_the_same_school() {
    let mut trust = Trust::new();
    let north = trust.north;
    let cass = trust.student(north, "Pupilsen");

    trust
        .db()
        .membership()
        .leave_school(north, cass, Expected::Any, Some("2027-07-20"))
        .expect("she leaves");
    trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, north))
        .expect("and comes back, which no constraint may forbid");

    let history = trust
        .db()
        .membership()
        .student_enrolments(cass)
        .expect("history");
    assert_eq!(history.len(), 2, "two spells at one school");
    assert_eq!(
        history
            .iter()
            .filter(|row| row.status == EnrolmentStatus::Enrolled)
            .count(),
        1,
        "exactly one of them current"
    );
    assert_eq!(history[0].left_on.as_deref(), Some("2027-07-20"));
}

/// Undated spells must not collide, because most schools do not record the date.
#[test]
fn leaving_and_rejoining_works_without_any_dates_at_all() {
    let mut trust = Trust::new();
    let north = trust.north;
    let cass = trust.student(north, "Pupilsen");

    for _ in 0..3 {
        trust
            .db()
            .membership()
            .leave_school(north, cass, Expected::Any, None)
            .expect("leaves, no date recorded");
        trust
            .db()
            .membership()
            .enrol_student(&enrolment(cass, north))
            .expect("rejoins, no date recorded");
    }

    assert_eq!(
        trust
            .db()
            .membership()
            .student_enrolments(cass)
            .expect("history")
            .len(),
        4,
        "a COALESCE'd uniqueness rule on enrolled_on would have refused the second of these"
    );
}

/// An admission number is one school's, and may be reissued once the holder has left.
#[test]
fn admission_numbers_are_per_school_and_only_bind_current_spells() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    let drew = trust.student(north, "Quill");

    let cass_spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, cass)
        .expect("read")
        .expect("there");
    trust
        .db()
        .membership()
        .update_enrolment(
            north,
            cass_spell.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("1001".to_owned()),
                ..enrolment(cass, north)
            },
        )
        .expect("Cass is 1001 at Northgate");

    // Southmoor may use the same number for somebody else. It is their number, not ours.
    trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            admission_no: Some("1001".to_owned()),
            ..enrolment(drew, south)
        })
        .expect("not project-wide");

    // Northgate may not, while Cass still holds it.
    let drew_spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, drew)
        .expect("read")
        .expect("there");
    let clash = trust
        .db()
        .membership()
        .update_enrolment(
            north,
            drew_spell.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("1001".to_owned()),
                ..enrolment(drew, north)
            },
        )
        .expect_err("taken");
    assert!(
        matches!(
            clash,
            DbError::DuplicateValue {
                field: "admission number",
                ..
            }
        ),
        "{clash:?}"
    );

    let _ = drew_spell;
    let cass_history = trust
        .db()
        .membership()
        .student_enrolments(cass)
        .expect("history");
    assert_eq!(
        cass_history[0].admission_no.as_deref(),
        Some("1001"),
        "and Cass's own record of being 1001 is hers"
    );
}

/// A retired admission number may be reissued, because the index binds current spells only.
///
/// Set up at Southmoor for two students Northgate created, so the home-school mirror of
/// ADR 0021 section 6 is not involved and the rule under test is the only thing acting.
/// The home-school case is the next test, and it is more restricted.
#[test]
fn an_admission_number_is_free_again_once_its_holder_has_left() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    let drew = trust.student(north, "Quill");

    trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            admission_no: Some("1001".to_owned()),
            ..enrolment(cass, south)
        })
        .expect("Cass is 1001 at Southmoor");

    let taken = trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            admission_no: Some("1001".to_owned()),
            ..enrolment(drew, south)
        })
        .expect_err("not while Cass holds it");
    assert!(
        matches!(
            taken,
            DbError::DuplicateValue {
                field: "admission number",
                ..
            }
        ),
        "{taken:?}"
    );

    trust
        .db()
        .membership()
        .leave_school(south, cass, Expected::Any, Some("2027-07-20"))
        .expect("Cass leaves Southmoor");
    trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            admission_no: Some("1001".to_owned()),
            ..enrolment(drew, south)
        })
        .expect("and the number may be reissued");

    // Her Southmoor spell, not her Northgate one -- she has both, and the history is
    // ordered oldest first.
    let history = trust
        .db()
        .membership()
        .student_enrolments(cass)
        .expect("history");
    let southern = history
        .iter()
        .find(|row| row.school_id == south)
        .expect("her Southmoor spell");
    assert_eq!(
        southern.admission_no.as_deref(),
        Some("1001"),
        "constraining history would have refused this and lost the record"
    );
    assert_eq!(southern.status, EnrolmentStatus::Left);
}

/// At a student's HOME school the legacy column still binds, and that is V10's rule.
///
/// `student.code` carries `UNIQUE (school_id, code)` over every student a school has ever
/// entered, leavers included -- a rule from `V10` that `V13` neither widens nor relaxes.
/// The home-school mirror brings it to bear on `admission_no`, so reissuing a retired
/// number *at the school that created both students* is refused where the same operation
/// at any other school succeeds. Asserted rather than wished away; ADR 0021 section 8
/// records the forward path, which is a later migration relaxing that standalone index.
#[test]
fn reissuing_a_number_at_the_home_school_is_still_bound_by_the_v10_index() {
    let mut trust = Trust::new();
    let north = trust.north;
    let cass = trust.student(north, "Pupilsen");
    let drew = trust.student(north, "Quill");

    let cass_spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, cass)
        .expect("read")
        .expect("there");
    trust
        .db()
        .membership()
        .update_enrolment(
            north,
            cass_spell.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("1001".to_owned()),
                ..enrolment(cass, north)
            },
        )
        .expect("Cass is 1001");
    trust
        .db()
        .membership()
        .leave_school(north, cass, Expected::Any, None)
        .expect("Cass leaves");

    let drew_spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, drew)
        .expect("read")
        .expect("there");
    let refused = trust
        .db()
        .membership()
        .update_enrolment(
            north,
            drew_spell.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("1001".to_owned()),
                ..enrolment(drew, north)
            },
        )
        .expect_err("student_code_unique still holds Cass's 1001");
    assert!(
        matches!(refused, DbError::DuplicateValue { .. }),
        "{refused:?}"
    );

    // The refusal destroyed nothing, which matters more than the refusal itself.
    let after = trust
        .db()
        .membership()
        .enrolment(drew_spell.id)
        .expect("read")
        .expect("there");
    assert_eq!(after.admission_no, None);
    assert_eq!(after.rev, 1, "and consumed no revision");
}

/// A placement names its own school's structures, or it names nothing meaningful.
#[test]
fn an_enrolment_cannot_be_placed_in_another_schools_year_level_or_campus() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_year, north_campus) = (trust.north_year, trust.north_campus);
    let cass = trust.student(north, "Pupilsen");

    let wrong_year = trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            year_level_id: Some(north_year),
            ..enrolment(cass, south)
        })
        .expect_err("Northgate's Grade 8 is not Southmoor's");
    assert!(
        matches!(wrong_year, DbError::Invalid { ref message } if message.contains("year level")),
        "{wrong_year:?}"
    );

    let wrong_campus = trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            campus_id: Some(north_campus),
            ..enrolment(cass, south)
        })
        .expect_err("nor is the campus");
    assert!(
        matches!(wrong_campus, DbError::Invalid { ref message } if message.contains("campus")),
        "{wrong_campus:?}"
    );

    // The schema refuses it underneath, with no repository involved.
    let raw = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO student_school_enrolment
                 (id, student_id, school_id, year_level_id, status,
                  created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, 'ENROLLED', '2026-10-14T09:00:00.000Z',
                     '2026-10-14T09:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                cass.to_string(),
                south.to_string(),
                north_year.to_string(),
            ),
        )?;
        Ok(())
    });
    assert!(raw.is_err(), "the trigger is what makes the rule true");
}

/// A group is one school's audience, and so are its members.
#[test]
fn a_student_cannot_join_a_group_of_a_school_they_have_never_attended() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    let southern_cohort = trust.group(south, "Southmoor Cohort", None);

    let refused = trust
        .db()
        .people()
        .add_student_group_member(&crate::repo::people::StudentGroupMemberInput {
            student_group_id: southern_cohort,
            student_id: cass,
        })
        .expect_err("she is not a Southmoor student");
    assert!(
        matches!(refused, DbError::Invalid { ref message } if message.contains("not enrolled")),
        "{refused:?}"
    );

    trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, south))
        .expect("enrol her there");
    trust
        .db()
        .people()
        .add_student_group_member(&crate::repo::people::StudentGroupMemberInput {
            student_group_id: southern_cohort,
            student_id: cass,
        })
        .expect("and now she may join");
}

/// A transfer does not erase the groups the previous school put her in.
#[test]
fn group_history_survives_a_transfer() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    let northern_cohort = trust.group(north, "Northgate Cohort", None);

    trust
        .db()
        .people()
        .add_student_group_member(&crate::repo::people::StudentGroupMemberInput {
            student_group_id: northern_cohort,
            student_id: cass,
        })
        .expect("in a Northgate group");

    trust
        .db()
        .membership()
        .transfer_student(north, south, cass, Expected::Any, &enrolment(cass, south))
        .expect("transfer");

    let groups = trust.db().people().groups_for_student(cass).expect("read");
    assert_eq!(
        groups.len(),
        1,
        "membership of 9-MAT-A last year is the only evidence it happened"
    );
    assert_eq!(groups[0].id, northern_cohort);
}

/// ADR 0006 is untouched: a school may still timetable with nothing but a declared size.
#[test]
fn a_school_with_no_students_at_all_still_works() {
    let mut trust = Trust::new();
    let north = trust.north;
    let cohort = trust.group(north, "Year 8", Some(120));

    assert_eq!(
        trust.db().people().students(north).expect("list").len(),
        0,
        "no student records"
    );
    assert_eq!(
        trust
            .db()
            .membership()
            .school_students(north)
            .expect("roll")
            .len(),
        0,
        "and no enrolments"
    );
    let group = trust
        .db()
        .people()
        .student_group(cohort)
        .expect("read")
        .expect("there");
    assert_eq!(
        group.size,
        Some(120),
        "the declared size is the audience, exactly as before"
    );
}

/// Deleting a shared student is the home school's, and only when nobody else has them.
#[test]
fn a_student_enrolled_elsewhere_cannot_be_deleted() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, south))
        .expect("shared");

    let refused = trust
        .db()
        .people()
        .delete_student(north, cass, Expected::Any)
        .expect_err("Southmoor has her");
    assert!(
        matches!(refused, DbError::StillReferenced { .. }),
        "{refused:?}"
    );

    let not_yours = trust
        .db()
        .people()
        .delete_student(south, cass, Expected::Any)
        .expect_err("and Southmoor may not destroy the person either");
    assert!(
        matches!(not_yours, DbError::WrongSchool { .. }),
        "{not_yours:?}"
    );

    assert!(
        trust.db().people().student(cass).expect("read").is_some(),
        "two refusals, nothing destroyed"
    );
}

/// The compatibility policy: one placement, written through either door.
#[test]
fn editing_a_students_details_keeps_the_home_spell_in_step() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (north_year, north_campus) = (trust.north_year, trust.north_campus);
    let cass = trust.student(north, "Pupilsen");

    let mut input = student_input_from(&mut trust, cass);
    input.code = Some("NG-42".to_owned());
    input.year_level_id = Some(north_year);
    input.campus_id = Some(north_campus);
    trust
        .db()
        .people()
        .update_student(cass, Expected::Any, &input)
        .expect("edit through the student screen");

    let spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, cass)
        .expect("read")
        .expect("there");
    assert_eq!(spell.admission_no.as_deref(), Some("NG-42"));
    assert_eq!(spell.year_level_id, Some(north_year));
    assert_eq!(spell.campus_id, Some(north_campus));

    // And the other direction.
    trust
        .db()
        .membership()
        .update_enrolment(
            north,
            spell.id,
            Expected::Any,
            &StudentEnrolmentInput {
                admission_no: Some("NG-43".to_owned()),
                year_level_id: None,
                campus_id: None,
                ..enrolment(cass, north)
            },
        )
        .expect("edit through the enrolment");

    let person = trust
        .db()
        .people()
        .student(cass)
        .expect("read")
        .expect("there");
    assert_eq!(person.code.as_deref(), Some("NG-43"));
    assert_eq!(person.year_level_id, None);
    assert_eq!(person.campus_id, None);
}

/// A school-scoped edit is not authority to rewrite another school's placement.
#[test]
fn a_school_cannot_edit_another_schools_enrolment() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let cass = trust.student(north, "Pupilsen");
    let southern = trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, south))
        .expect("shared");

    let refused = trust
        .db()
        .membership()
        .update_enrolment(north, southern.id, Expected::Any, &enrolment(cass, south))
        .expect_err("not Northgate's spell");
    assert!(
        matches!(refused, DbError::WrongSchool { .. }),
        "{refused:?}"
    );
}

// =================================================================== shared resources

/// One trolley, two schools, one identifier.
#[test]
fn a_resource_is_shared_across_schools_as_one_record() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let trolley = trust.resource(north, "Projector trolley", "PROJ", Some(north_campus));

    trust
        .db()
        .structure()
        .share_resource(north, trolley, south_campus)
        .expect("Northgate lends it to Southmoor");

    let northern = trust.db().structure().resources(north).expect("north list");
    let southern = trust.db().structure().resources(south).expect("south list");
    assert_eq!(northern.len(), 1);
    assert_eq!(southern.len(), 1);
    assert_eq!(
        northern[0].id, southern[0].id,
        "the same resource identifier in both lists: one object, one occupancy record"
    );

    let campuses = trust
        .db()
        .structure()
        .resource_campuses(trolley)
        .expect("read");
    assert_eq!(campuses.len(), 2);
    assert!(campuses.contains(&north_campus) && campuses.contains(&south_campus));

    let rows: i64 = trust
        .db()
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM resource", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("count");
    assert_eq!(rows, 1, "nothing was duplicated to make it available");
}

/// A school-wide resource has no campus, and that is not an error.
#[test]
fn a_resource_with_no_campus_has_no_associations_and_can_still_be_shared() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let south_campus = trust.south_campus;
    let minibus = trust.resource(north, "Minibus", "BUS", None);

    assert_eq!(
        trust
            .db()
            .structure()
            .resource_campuses(minibus)
            .expect("read"),
        Vec::<Uuid>::new(),
        "unlike a room, a resource may serve no particular site"
    );

    trust
        .db()
        .structure()
        .share_resource(north, minibus, south_campus)
        .expect("and may still be lent");
    assert_eq!(
        trust.db().structure().resources(south).expect("list").len(),
        1
    );
}

/// Either end may withdraw a share. A third school may not.
#[test]
fn only_a_school_at_one_end_of_a_share_may_withdraw_it() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let third = trust.school("Eastbrook Free School");
    let trolley = trust.resource(north, "Projector trolley", "PROJ", Some(north_campus));
    trust
        .db()
        .structure()
        .share_resource(north, trolley, south_campus)
        .expect("lent");

    let refused = trust
        .db()
        .structure()
        .unshare_resource(third, trolley, south_campus)
        .expect_err("Eastbrook has no say in either end");
    assert!(
        matches!(refused, DbError::WrongSchool { .. }),
        "{refused:?}"
    );

    trust
        .db()
        .structure()
        .unshare_resource(south, trolley, south_campus)
        .expect("the borrower may hand it back");
    assert_eq!(
        trust.db().structure().resources(south).expect("list").len(),
        0
    );
    assert_eq!(
        trust.db().structure().resources(north).expect("list").len(),
        1,
        "and the owner still has it"
    );
}

/// A resource's home campus cannot be withdrawn, and the owner cannot destroy a lent one.
#[test]
fn a_shared_resource_is_protected_from_its_owner() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let trolley = trust.resource(north, "Projector trolley", "PROJ", Some(north_campus));
    trust
        .db()
        .structure()
        .share_resource(north, trolley, south_campus)
        .expect("lent");

    let home = trust
        .db()
        .structure()
        .unshare_resource(north, trolley, north_campus)
        .expect_err("that would contradict resource.campus_id");
    assert!(
        matches!(home, DbError::Invalid { ref message } if message.contains("based at")),
        "{home:?}"
    );

    let destroy = trust
        .db()
        .structure()
        .delete_resource(north, trolley, Expected::Any)
        .expect_err("Southmoor is timetabling into it");
    assert!(
        matches!(destroy, DbError::StillReferenced { .. }),
        "{destroy:?}"
    );

    trust
        .db()
        .structure()
        .unshare_resource(south, trolley, south_campus)
        .expect("withdraw first");
    trust
        .db()
        .structure()
        .delete_resource(north, trolley, Expected::Any)
        .expect("and now the owner may");
}

/// Moving a resource's home campus moves the association, and leaves the loans alone.
#[test]
fn rebasing_a_resource_keeps_the_shares_it_was_lent_under() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let annex = trust.campus(north, "Northgate Annex", "NGA");
    let trolley = trust.resource(north, "Projector trolley", "PROJ", Some(north_campus));
    trust
        .db()
        .structure()
        .share_resource(north, trolley, south_campus)
        .expect("lent");

    trust
        .db()
        .structure()
        .update_resource(
            trolley,
            Expected::Any,
            &ResourceInput {
                school_id: north,
                campus_id: Some(annex),
                name: "Projector trolley".to_owned(),
                code: "PROJ".to_owned(),
                quantity: 1,
            },
        )
        .expect("it lives in the annex now");

    let campuses = trust
        .db()
        .structure()
        .resource_campuses(trolley)
        .expect("read");
    assert!(campuses.contains(&annex), "the new home is associated");
    assert!(
        !campuses.contains(&north_campus),
        "the old home is not, because it is not where the thing is"
    );
    assert!(
        campuses.contains(&south_campus),
        "and Southmoor's loan is not Northgate's to revoke by editing a field"
    );
}

// ======================================================= a school's remaining people

/// A school that still shares somebody else's teacher cannot be deleted silently.
#[test]
fn school_dependents_counts_the_people_it_merely_borrows() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let ada = trust.teacher(north, "Okafor", None);
    let cass = trust.student(north, "Pupilsen");
    trust
        .db()
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ada,
            school_id: south,
            is_active: true,
        })
        .expect("shared");
    trust
        .db()
        .membership()
        .enrol_student(&enrolment(cass, south))
        .expect("shared");

    // Southmoor owns a campus and a year level too; empty those, and the shared people are
    // what is left in the way.
    let (south_campus, south_year) = (trust.south_campus, trust.south_year);
    trust
        .db()
        .structure()
        .delete_campus(south, south_campus, Expected::Any)
        .expect("its own campus");
    trust
        .db()
        .people()
        .delete_year_level(south, south_year, Expected::Any)
        .expect("its own year level");

    let dependents = trust
        .db()
        .structure()
        .school_dependents(south)
        .expect("read");
    assert_eq!(dependents.teachers, 0, "it created no teacher of its own");
    assert_eq!(dependents.students, 0, "nor any student");
    assert_eq!(dependents.shared_teachers, 1);
    assert_eq!(dependents.shared_students, 1);
    assert_eq!(
        dependents.total, 2,
        "and the total a confirmation reads is these two, counted once each"
    );

    let refused = trust
        .db()
        .structure()
        .delete_school(south, Expected::Any)
        .expect_err("a borrowed person is still something in the way");
    assert!(
        matches!(refused, DbError::StillReferenced { .. }),
        "{refused:?}"
    );
}

/// Deleting a campus clears what pointed at it, rather than refusing for ever.
#[test]
fn deleting_a_campus_clears_eligibility_and_loans_but_not_what_is_based_there() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let (north_campus, south_campus) = (trust.north_campus, trust.south_campus);
    let ada = trust.teacher(north, "Okafor", Some(north_campus));
    let trolley = trust.resource(south, "Trolley", "TR", Some(south_campus));

    trust
        .db()
        .structure()
        .share_resource(south, trolley, north_campus)
        .expect("Southmoor lends it to Northgate");

    // The backfill gives every teacher with a home campus an eligibility row, and
    // `create_teacher` does not -- so assign it explicitly, which is what a Stage 0C
    // screen will do.
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, ada, north_campus)
        .expect("eligible there");

    trust
        .db()
        .structure()
        .delete_campus(north, north_campus, Expected::Any)
        .expect("the campus closes");

    assert_eq!(
        trust.db().membership().teacher_campuses(ada).expect("read"),
        Vec::<Uuid>::new(),
        "a restriction naming somewhere that no longer exists is not information"
    );
    assert_eq!(
        trust
            .db()
            .structure()
            .resource_campuses(trolley)
            .expect("read"),
        vec![south_campus],
        "and Northgate's access went with Northgate's campus"
    );
    assert!(
        trust.db().people().teacher(ada).expect("read").is_some(),
        "the teacher is not taken with it"
    );
}

// ====================================================================== the migration

/// Everything a V12 project could hold, written as the previous build would have.
struct PreV13 {
    school: Uuid,
    other_school: Uuid,
    campus: Uuid,
    year_level: Uuid,
    teacher_based: Uuid,
    teacher_floating: Uuid,
    student_here: Uuid,
    student_left: Uuid,
    resource_on_campus: Uuid,
    resource_school_wide: Uuid,
}

const STAMP: &str = "2026-10-01T09:00:00.000Z";

/// Writes a project exactly as `V12` left it, with revisions above 1 so that "the
/// migration consumed no revision" is checkable rather than vacuous.
#[expect(
    clippy::too_many_lines,
    reason = "one fixture holding every kind of row a V12 project could have, in one place"
)]
fn write_a_pre_v13_project(path: &std::path::Path, site: Uuid) -> PreV13 {
    let ids = PreV13 {
        school: Uuid::now_v7(),
        other_school: Uuid::now_v7(),
        campus: Uuid::now_v7(),
        year_level: Uuid::now_v7(),
        teacher_based: Uuid::now_v7(),
        teacher_floating: Uuid::now_v7(),
        student_here: Uuid::now_v7(),
        student_left: Uuid::now_v7(),
        resource_on_campus: Uuid::now_v7(),
        resource_school_wide: Uuid::now_v7(),
    };

    let mut conn = crate::connection::open_configured(path, site).expect("open");
    let version = crate::migrations::run_up_to(&mut conn, 12).expect("migrate to V12");
    assert_eq!(version, 12, "the fixture must stop at the previous version");

    let tx = conn.transaction().expect("transaction");
    for (id, name) in [
        (ids.school, "Northgate Secondary"),
        (ids.other_school, "Southmoor College"),
    ] {
        tx.execute(
            "INSERT INTO school (id, name, timezone, locale, working_days,
                                 created_at, updated_at, rev)
             VALUES (?, ?, 'Europe/London', 'en-GB', 31, ?, ?, 3)",
            (id.to_string(), name, STAMP, STAMP),
        )
        .expect("school");
    }
    tx.execute(
        "INSERT INTO campus (id, school_id, name, code, address, created_at, updated_at, rev)
         VALUES (?, ?, 'Main', 'MAIN', NULL, ?, ?, 2)",
        (ids.campus.to_string(), ids.school.to_string(), STAMP, STAMP),
    )
    .expect("campus");
    tx.execute(
        "INSERT INTO year_level (id, school_id, name, ordinal, created_at, updated_at, rev)
         VALUES (?, ?, 'Grade 8', 8, ?, ?, 4)",
        (
            ids.year_level.to_string(),
            ids.school.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("year level");

    for (id, family, campus, active) in [
        (ids.teacher_based, "Okafor", Some(ids.campus), 1),
        (ids.teacher_floating, "Retiree", None, 0),
    ] {
        tx.execute(
            "INSERT INTO teacher (id, school_id, code, given_name, family_name, email,
                                  home_campus_id, fte_percent, is_active,
                                  created_at, updated_at, rev)
             VALUES (?, ?, NULL, 'Ada', ?, NULL, ?, 100, ?, ?, ?, 5)",
            rusqlite::params![
                id.to_string(),
                ids.school.to_string(),
                family,
                campus.map(|value| value.to_string()),
                active,
                STAMP,
                STAMP,
            ],
        )
        .expect("teacher");
    }

    for (id, family, code, active) in [
        (ids.student_here, "Pupilsen", Some("1001"), 1),
        (ids.student_left, "Leaver", None, 0),
    ] {
        tx.execute(
            "INSERT INTO student (id, school_id, code, given_name, family_name, email,
                                  year_level_id, campus_id, is_active,
                                  created_at, updated_at, rev)
             VALUES (?, ?, ?, 'Cass', ?, NULL, ?, ?, ?, ?, ?, 6)",
            rusqlite::params![
                id.to_string(),
                ids.school.to_string(),
                code,
                family,
                ids.year_level.to_string(),
                ids.campus.to_string(),
                active,
                STAMP,
                STAMP,
            ],
        )
        .expect("student");
    }

    for (id, code, campus) in [
        (ids.resource_on_campus, "PROJ", Some(ids.campus)),
        (ids.resource_school_wide, "BUS", None),
    ] {
        tx.execute(
            "INSERT INTO resource (id, school_id, campus_id, name, code, quantity,
                                   created_at, updated_at, rev)
             VALUES (?, ?, ?, 'Thing', ?, 1, ?, ?, 7)",
            rusqlite::params![
                id.to_string(),
                ids.school.to_string(),
                campus.map(|value| value.to_string()),
                code,
                STAMP,
                STAMP,
            ],
        )
        .expect("resource");
    }

    tx.commit().expect("commit the fixture");
    drop(conn);
    ids
}

/// The whole backfill, asserted row by row on a project the previous build wrote.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one migration, every guarantee it has to make, in the order it makes them"
)]
fn a_v12_project_upgrades_and_nothing_it_held_moved() {
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let path = dir.path().join(format!("trust.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000013").expect("valid uuid");

    let ids = write_a_pre_v13_project(&path, site);

    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    workspace.open_project(&path).expect("the upgrade runs");
    let db = workspace.open_database_for_test();
    assert_eq!(
        db.schema_version(),
        crate::migrations::supported_schema_version()
    );

    // ---- identities and revisions are exactly what they were.
    for (table, id, rev) in [
        ("school", ids.school, 3),
        ("campus", ids.campus, 2),
        ("year_level", ids.year_level, 4),
        ("teacher", ids.teacher_based, 5),
        ("teacher", ids.teacher_floating, 5),
        ("student", ids.student_here, 6),
        ("student", ids.student_left, 6),
        ("resource", ids.resource_on_campus, 7),
        ("resource", ids.resource_school_wide, 7),
    ] {
        let found: (String, i64, String) = db
            .read(|conn| {
                conn.query_row(
                    &format!("SELECT created_at, rev, updated_at FROM {table} WHERE id = ?"),
                    [id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(DbError::from)
            })
            .unwrap_or_else(|error| panic!("{table} {id} is gone: {error}"));
        assert_eq!(found.1, rev, "{table} {id} had its revision consumed");
        assert_eq!(found.0, STAMP, "{table} {id} was rewritten");
        assert_eq!(found.2, STAMP, "{table} {id} was rewritten");
    }

    // ---- one membership per teacher, naming the school they already belonged to.
    let memberships = db
        .membership()
        .teacher_memberships(ids.teacher_based)
        .expect("read");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].school_id, ids.school);
    assert!(memberships[0].is_active, "she was active and still is");
    assert_eq!(memberships[0].rev, 1, "a backfilled row starts at rev 1");
    assert_eq!(
        memberships[0].created_at, STAMP,
        "the relationship is as old as the fact it records"
    );
    assert_eq!(memberships[0].created_at, memberships[0].updated_at);
    assert_eq!(
        memberships[0].id.get_version_num(),
        7,
        "the hand-rolled identifier really is a UUIDv7"
    );

    let retired = db
        .membership()
        .teacher_memberships(ids.teacher_floating)
        .expect("read");
    assert_eq!(retired.len(), 1);
    assert!(
        !retired[0].is_active,
        "reading is_active as 1 regardless would silently re-employ every leaver"
    );

    // ---- campus eligibility from home_campus_id, and nothing invented for the teacher
    // who had none.
    assert_eq!(
        db.membership()
            .teacher_campuses(ids.teacher_based)
            .expect("read"),
        vec![ids.campus]
    );
    assert_eq!(
        db.membership()
            .teacher_campuses(ids.teacher_floating)
            .expect("read"),
        Vec::<Uuid>::new(),
        "no restriction recorded is not the same claim as restricted to nowhere"
    );

    // ---- one enrolment per student, carrying the placement and the number they had.
    let here = db
        .membership()
        .student_enrolments(ids.student_here)
        .expect("read");
    assert_eq!(here.len(), 1);
    assert_eq!(here[0].school_id, ids.school);
    assert_eq!(here[0].status, EnrolmentStatus::Enrolled);
    assert_eq!(here[0].admission_no.as_deref(), Some("1001"));
    assert_eq!(here[0].year_level_id, Some(ids.year_level));
    assert_eq!(here[0].campus_id, Some(ids.campus));
    assert_eq!(here[0].rev, 1);
    assert_eq!(here[0].created_at, STAMP);

    let gone = db
        .membership()
        .student_enrolments(ids.student_left)
        .expect("read");
    assert_eq!(gone.len(), 1);
    assert_eq!(
        gone[0].status,
        EnrolmentStatus::Left,
        "a student the school had marked as having left is a past spell"
    );
    assert_eq!(
        gone[0].left_on, None,
        "and no date is invented, because none was ever recorded"
    );

    // ---- resource associations for the one that had a campus, none for the school-wide.
    assert_eq!(
        db.structure()
            .resource_campuses(ids.resource_on_campus)
            .expect("read"),
        vec![ids.campus]
    );
    assert_eq!(
        db.structure()
            .resource_campuses(ids.resource_school_wide)
            .expect("read"),
        Vec::<Uuid>::new()
    );

    // ---- the project is referentially sound, and the new tables are in the change feed.
    let orphans: i64 = db
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .map_err(DbError::from)
        })
        .expect("foreign_key_check");
    assert_eq!(orphans, 0);

    for table in [
        "teacher_school",
        "teacher_campus",
        "student_school_enrolment",
        "resource_campus",
    ] {
        let triggers: i64 = db
            .read(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
                     AND name LIKE 'trg_' || ?1 || '_changelog%'",
                    [table],
                    |row| row.get(0),
                )
                .map_err(DbError::from)
            })
            .expect("count triggers");
        assert_eq!(triggers, 3, "{table} must be in the change feed");
    }

    // ---- the backfill wrote no change-log entries, because a schema change is not an
    // entity change. install_all runs after the migration, so the triggers did not exist
    // while the rows were inserted.
    let logged: i64 = db
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM change_log
                 WHERE entity_table IN ('teacher_school', 'teacher_campus',
                                        'student_school_enrolment', 'resource_campus')",
                [],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("count entries");
    assert_eq!(logged, 0);

    // ---- and the usual operations work on the upgraded project.
    let moved = db
        .membership()
        .add_teacher_to_school(&TeacherSchoolInput {
            teacher_id: ids.teacher_based,
            school_id: ids.other_school,
            is_active: true,
        })
        .expect("the point of the upgrade");
    assert_eq!(moved.rev, 1);
    assert_eq!(
        db.people().teachers(ids.other_school).expect("list").len(),
        1
    );
}

/// Reopening an upgraded project changes nothing and applies nothing.
#[test]
fn an_upgraded_project_reopens_cleanly() {
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let path = dir.path().join(format!("trust.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000015").expect("valid uuid");
    let ids = write_a_pre_v13_project(&path, site);

    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    workspace.open_project(&path).expect("upgrade");
    let backfilled = workspace
        .open_database_for_test()
        .membership()
        .teacher_memberships(ids.teacher_based)
        .expect("read");
    assert!(workspace.close_project(), "a project was open to close");

    workspace.open_project(&path).expect("reopen");
    let again = workspace
        .open_database_for_test()
        .membership()
        .teacher_memberships(ids.teacher_based)
        .expect("read");
    assert_eq!(
        backfilled, again,
        "a second open must not re-backfill or renumber anything"
    );
}

/// A V11 project still gets the Stage 0A safety copy on its way past the rebuild.
#[test]
fn the_stage_0a_backup_still_happens_for_a_project_that_predates_it() {
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let path = dir.path().join(format!("old.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000016").expect("valid uuid");

    {
        let mut conn = crate::connection::open_configured(&path, site).expect("open");
        crate::migrations::run_up_to(&mut conn, 11).expect("migrate to V11");
        conn.execute(
            "INSERT INTO school (id, only_one, name, timezone, locale, working_days,
                                 created_at, updated_at, rev)
             VALUES (?, 1, 'Northgate Secondary', 'Europe/London', 'en-GB', 31, ?, ?, 1)",
            (Uuid::now_v7().to_string(), STAMP, STAMP),
        )
        .expect("a school to protect");
    }

    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    workspace.open_project(&path).expect("upgrade V11 -> V13");
    let db = workspace.open_database_for_test();
    assert!(
        db.safety_backup().is_some(),
        "V13 is additive, but the route to it still crosses V12's rebuild"
    );
    assert_eq!(
        db.schema_version(),
        crate::migrations::supported_schema_version(),
        "and it arrives at the current schema in one open"
    );
}

/// A failure anywhere in the batch leaves the project exactly as it was.
#[test]
fn a_failure_in_the_batch_rolls_the_whole_backfill_back() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000017").expect("valid uuid");

    let ids = write_a_pre_v13_project(&path, site);
    let mut conn = crate::connection::open_configured(&path, site).expect("open");

    let tx = conn.transaction().expect("transaction");
    tx.execute_batch(V13_SQL).expect("the migration applies");
    let rows: i64 = tx
        .query_row("SELECT COUNT(*) FROM teacher_school", [], |row| row.get(0))
        .expect("count");
    assert_eq!(rows, 2, "the backfill really did run before the failure");

    let failure = tx.execute_batch(
        "INSERT INTO teacher_school (id, teacher_id, school_id, is_active,
                                     created_at, updated_at, rev)
         VALUES (NULL, NULL, NULL, NULL, NULL, NULL, NULL);",
    );
    assert!(
        failure.is_err(),
        "the deliberate failure must actually fail"
    );
    drop(tx); // No commit: this is what refinery does with a failed batch.

    for table in [
        "teacher_school",
        "teacher_campus",
        "student_school_enrolment",
        "resource_campus",
    ] {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("look for the table");
        assert_eq!(present, 0, "{table} went with the rollback");
    }

    // The V12 triggers the migration replaces are back, under their original names.
    for trigger in [
        "teacher_references_own_school_insert",
        "teacher_references_own_school_update",
    ] {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get(0),
            )
            .expect("look for the trigger");
        assert_eq!(present, 1, "{trigger} was restored");
    }

    let rev: i64 = conn
        .query_row(
            "SELECT rev FROM teacher WHERE id = ?",
            [ids.teacher_based.to_string()],
            |row| row.get(0),
        )
        .expect("the teacher is still there");
    assert_eq!(rev, 5, "and untouched");

    let orphans: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign_key_check");
    assert_eq!(orphans, 0);
}

/// Existing group memberships survive the migration, enrolment rule and all.
#[test]
fn a_group_membership_written_before_v13_is_not_invalidated_by_the_new_rule() {
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let path = dir.path().join(format!("trust.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000018").expect("valid uuid");

    let ids = write_a_pre_v13_project(&path, site);
    let group = Uuid::now_v7();
    {
        let mut conn = crate::connection::open_configured(&path, site).expect("open");
        let tx = conn.transaction().expect("tx");
        tx.execute(
            "INSERT INTO student_group (id, school_id, name, kind, created_at, updated_at, rev)
             VALUES (?, ?, 'Grade 8 Cohort', 'COHORT', ?, ?, 1)",
            (group.to_string(), ids.school.to_string(), STAMP, STAMP),
        )
        .expect("group");
        tx.execute(
            "INSERT INTO student_group_member (id, student_group_id, student_id,
                                               created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, 1)",
            (
                Uuid::now_v7().to_string(),
                group.to_string(),
                ids.student_here.to_string(),
                STAMP,
                STAMP,
            ),
        )
        .expect("membership");
        tx.commit().expect("commit");
    }

    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    workspace.open_project(&path).expect("upgrade");
    let db = workspace.open_database_for_test();

    let members = db.people().student_group_members(group).expect("read");
    assert_eq!(members.len(), 1, "the membership survived");
    assert_eq!(members[0].id, ids.student_here);

    // And the student the backfill marked as LEFT may still be added, because any spell
    // counts -- history must not be refused.
    db.people()
        .add_student_group_member(&crate::repo::people::StudentGroupMemberInput {
            student_group_id: group,
            student_id: ids.student_left,
        })
        .expect("a past spell is still a spell");
}

// =========================================================================== helpers

/// The teacher's current values as an input, for an edit that changes one of them.
fn teacher_input_from(trust: &mut Trust, id: Uuid) -> TeacherInput {
    let row = trust
        .db()
        .people()
        .teacher(id)
        .expect("read")
        .expect("there");
    TeacherInput {
        school_id: row.school_id,
        code: row.code,
        given_name: row.given_name,
        family_name: row.family_name,
        email: row.email,
        home_campus_id: row.home_campus_id,
        fte_percent: row.fte_percent,
        max_periods_per_day: row.max_periods_per_day,
        max_periods_per_cycle: row.max_periods_per_cycle,
        max_consecutive_periods: row.max_consecutive_periods,
        notes: row.notes,
        is_active: row.is_active,
    }
}

/// The student's current values as an input.
fn student_input_from(trust: &mut Trust, id: Uuid) -> StudentInput {
    let row = trust
        .db()
        .people()
        .student(id)
        .expect("read")
        .expect("there");
    StudentInput {
        school_id: row.school_id,
        code: row.code,
        given_name: row.given_name,
        family_name: row.family_name,
        email: row.email,
        year_level_id: row.year_level_id,
        campus_id: row.campus_id,
        is_active: row.is_active,
    }
}
