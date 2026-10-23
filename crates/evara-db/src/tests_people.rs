//! Phase 2 tests: teachers, students, groups and membership.
//!
//! Real temporary project files throughout, never `:memory:` — foreign keys, triggers and
//! the partial unique indexes are all properties of a real database file.
//!
//! The theme running through most of these is **what survives**. A school's people outlive
//! any one year of its timetable, so the questions worth testing are less "can I add a
//! teacher" than "is the teacher still the same teacher after an edit", "does a student
//! who leaves keep their history", and "can the database lose a membership without anyone
//! being told". The first is one assertion; the rest are the reason this file is long.

use std::path::PathBuf;

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::people::{
    GroupKind, StudentGroupInput, StudentGroupMemberInput, StudentInput, TeacherInput,
    YearLevelInput,
};
use crate::repo::structure::{CampusInput, SchoolInput};
use crate::tests_support;
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// Plenty for these tests, which make a handful of writes each.
const CHANGE_FEED_LIMIT: i64 = 1_000;

/// An open project in its own temporary directory.
struct Project {
    _dir: TempDir,
    workspace: Workspace,
    path: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let mut workspace =
            Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
                .expect("workspace");
        let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");
        Self {
            _dir: dir,
            workspace,
            path,
        }
    }

    fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    /// A school and one campus, the usual starting point.
    fn with_school(&mut self) -> (Uuid, Uuid) {
        let school = self
            .db()
            .structure()
            .create_school(&SchoolInput {
                name: "Northgate Secondary".to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::from_bits(0b001_1111).expect("weekdays"),
            })
            .expect("create school");

        let campus = self
            .db()
            .structure()
            .create_campus(&CampusInput {
                school_id: school.id,
                name: "Main".to_owned(),
                code: "MAIN".to_owned(),
                address: None,
            })
            .expect("create campus");

        (school.id, campus.id)
    }

    /// Closes and reopens the project file.
    ///
    /// The point of several tests below: a local-first application's real persistence test
    /// is not "is it in the struct", it is "is it still there tomorrow".
    fn reopen(&mut self) {
        assert!(
            self.workspace.close_project(),
            "the project must have been open"
        );
        self.workspace.open_project(&self.path).expect("reopen");
    }
}

/// A teacher input with everything optional left unset.
fn teacher(school_id: Uuid, given: &str, family: &str) -> TeacherInput {
    TeacherInput {
        school_id,
        code: None,
        given_name: given.to_owned(),
        family_name: family.to_owned(),
        email: None,
        home_campus_id: None,
        fte_percent: 100,
        max_periods_per_day: None,
        max_periods_per_cycle: None,
        max_consecutive_periods: None,
        notes: None,
        is_active: true,
    }
}

/// A student input with everything optional left unset.
fn student(school_id: Uuid, given: &str, family: &str) -> StudentInput {
    StudentInput {
        school_id,
        code: None,
        given_name: given.to_owned(),
        family_name: family.to_owned(),
        email: None,
        year_level_id: None,
        campus_id: None,
        is_active: true,
    }
}

/// A group input with everything optional left unset.
fn group(school_id: Uuid, name: &str, kind: GroupKind) -> StudentGroupInput {
    StudentGroupInput {
        school_id,
        name: name.to_owned(),
        code: None,
        kind,
        year_level_id: None,
        parent_group_id: None,
        size: None,
        campus_id: None,
        children_are_disjoint: None,
    }
}

// ------------------------------------------------------------------------- teachers

#[test]
fn a_teacher_can_be_created_edited_and_deactivated() {
    let mut project = Project::new();
    let (school_id, campus_id) = project.with_school();

    let created = project
        .db()
        .people()
        .create_teacher(&TeacherInput {
            code: Some("T-001".to_owned()),
            email: Some("a.okafor@example.invalid".to_owned()),
            home_campus_id: Some(campus_id),
            fte_percent: 80,
            max_periods_per_day: Some(5),
            ..teacher(school_id, "Amara", "Okafor")
        })
        .expect("create teacher");

    assert_eq!(created.rev, 1);
    assert_eq!(created.created_at, created.updated_at);
    assert!(created.is_active);

    let edited = project
        .db()
        .people()
        .update_teacher(
            created.id,
            Expected::Any,
            &TeacherInput {
                code: Some("T-001".to_owned()),
                family_name: "Okafor-Bell".to_owned(),
                is_active: false,
                ..teacher(school_id, "Amara", "Okafor")
            },
        )
        .expect("update teacher");

    // The whole point of a stable identity: the same person after the edit.
    assert_eq!(
        edited.id, created.id,
        "editing must not re-identify a teacher"
    );
    assert_eq!(edited.rev, 2);
    assert_eq!(edited.family_name, "Okafor-Bell");
    assert!(!edited.is_active);
    assert_eq!(edited.created_at, created.created_at);
}

#[test]
fn an_inactive_teacher_is_still_listed() {
    // Deactivation is not deletion, and a repository that hid inactive people would also
    // hide them from a timetable that names one. Who to *offer* is the interface's call.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    project
        .db()
        .people()
        .create_teacher(&TeacherInput {
            is_active: false,
            ..teacher(school_id, "Priya", "Raman")
        })
        .expect("create teacher");

    let listed = tests_support::teachers_here(project.db());
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_active);
}

#[test]
fn a_staff_code_is_unique_only_when_it_is_given() {
    // The partial index earns its place here: two teachers with no staff number is the
    // common case for a small school, and a COALESCE-based rule would have refused it.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    project
        .db()
        .people()
        .create_teacher(&teacher(school_id, "Nia", "Fletcher"))
        .expect("first teacher without a code");
    project
        .db()
        .people()
        .create_teacher(&teacher(school_id, "Tom", "Ellery"))
        .expect("a second teacher without a code is fine");

    project
        .db()
        .people()
        .create_teacher(&TeacherInput {
            code: Some("T-007".to_owned()),
            ..teacher(school_id, "Ada", "Shaw")
        })
        .expect("create with a code");

    let clash = project.db().people().create_teacher(&TeacherInput {
        code: Some("T-007".to_owned()),
        ..teacher(school_id, "Other", "Person")
    });

    assert!(
        matches!(clash, Err(DbError::DuplicateValue { .. })),
        "a repeated staff code must be refused, got {clash:?}"
    );
}

// ------------------------------------------------------------------------- students

#[test]
fn a_student_can_be_created_edited_and_deactivated() {
    let mut project = Project::new();
    let (school_id, campus_id) = project.with_school();

    let year = project
        .db()
        .people()
        .create_year_level(&YearLevelInput {
            school_id,
            name: "Year 8".to_owned(),
            ordinal: 8,
        })
        .expect("create year level");

    let created = project
        .db()
        .people()
        .create_student(&StudentInput {
            code: Some("A-2301".to_owned()),
            year_level_id: Some(year.id),
            campus_id: Some(campus_id),
            ..student(school_id, "Luca", "Moretti")
        })
        .expect("create student");

    assert!(created.is_active);
    assert_eq!(created.year_level_id, Some(year.id));

    let left = project
        .db()
        .people()
        .update_student(
            created.id,
            Expected::Any,
            &StudentInput {
                code: Some("A-2301".to_owned()),
                year_level_id: Some(year.id),
                is_active: false,
                ..student(school_id, "Luca", "Moretti")
            },
        )
        .expect("deactivate student");

    assert_eq!(
        left.id, created.id,
        "editing must not re-identify a student"
    );
    assert!(!left.is_active);
    assert_eq!(left.rev, 2);
}

#[test]
fn a_student_identity_outlives_the_year_level_it_was_classified_by() {
    // "Stable identities independent of academic-year enrolments", tested the only way
    // that means anything: destroy the classification and check the person survives it.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let year = project
        .db()
        .people()
        .create_year_level(&YearLevelInput {
            school_id,
            name: "Year 11".to_owned(),
            ordinal: 11,
        })
        .expect("create year level");

    let created = project
        .db()
        .people()
        .create_student(&StudentInput {
            year_level_id: Some(year.id),
            ..student(school_id, "Sofia", "Haddad")
        })
        .expect("create student");

    tests_support::delete_year_level_here(project.db(), year.id).expect("delete year level");

    let after = project
        .db()
        .people()
        .student(created.id)
        .expect("read student")
        .expect("the student must still exist");

    assert_eq!(after.id, created.id);
    assert_eq!(after.year_level_id, None, "the classification is cleared");
    assert_eq!(
        after.rev, 2,
        "detaching is a real edit and must consume a revision, or the change feed lies"
    );
}

// --------------------------------------------------------------------------- groups

#[test]
fn a_group_can_be_created_and_sized_without_any_students() {
    // ADR 0006: a school must be able to build a full timetable using only the declared
    // size, without entering a single student. If this breaks, that promise breaks.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let created = project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            code: Some("8A".to_owned()),
            size: Some(28),
            ..group(school_id, "Grade 8A", GroupKind::Class)
        })
        .expect("create group");

    assert_eq!(created.size, Some(28));
    assert_eq!(created.kind, GroupKind::Class);
    assert_eq!(
        project
            .db()
            .people()
            .student_group_member_count(created.id)
            .expect("count"),
        0,
        "a declared size is not a membership"
    );
}

#[test]
fn a_student_can_be_in_several_groups_at_once() {
    // A homeroom *and* a maths set *and* an elective. Anything that assumed one group per
    // student would pass every other test in this file and fail this one.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Ines", "Varga"))
        .expect("create student");

    let mut ids = Vec::new();
    for (name, kind) in [
        ("8A Homeroom", GroupKind::Homeroom),
        ("Grade 8 Mathematics", GroupKind::TeachingSet),
        ("French elective", GroupKind::ElectiveSet),
    ] {
        let created = project
            .db()
            .people()
            .create_student_group(&group(school_id, name, kind))
            .expect("create group");
        project
            .db()
            .people()
            .add_student_group_member(&StudentGroupMemberInput {
                student_group_id: created.id,
                student_id: pupil.id,
            })
            .expect("add member");
        ids.push(created.id);
    }

    let groups = project
        .db()
        .people()
        .groups_for_student(pupil.id)
        .expect("groups for student");

    assert_eq!(groups.len(), 3);
    for id in ids {
        assert!(groups.iter().any(|g| g.id == id));
    }
}

#[test]
fn two_students_in_one_homeroom_can_take_different_electives() {
    // The case the brief singles out, and the one a single-group model gets wrong.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let homeroom = project
        .db()
        .people()
        .create_student_group(&group(school_id, "8B Homeroom", GroupKind::Homeroom))
        .expect("homeroom");
    let french = project
        .db()
        .people()
        .create_student_group(&group(school_id, "French", GroupKind::ElectiveSet))
        .expect("french");
    let computing = project
        .db()
        .people()
        .create_student_group(&group(
            school_id,
            "Computer Science",
            GroupKind::ElectiveSet,
        ))
        .expect("computing");

    let first = project
        .db()
        .people()
        .create_student(&student(school_id, "Mateo", "Reyes"))
        .expect("first student");
    let second = project
        .db()
        .people()
        .create_student(&student(school_id, "Yuki", "Tanaka"))
        .expect("second student");

    for (student_id, elective) in [(first.id, french.id), (second.id, computing.id)] {
        for group_id in [homeroom.id, elective] {
            project
                .db()
                .people()
                .add_student_group_member(&StudentGroupMemberInput {
                    student_group_id: group_id,
                    student_id,
                })
                .expect("add member");
        }
    }

    assert_eq!(
        project
            .db()
            .people()
            .student_group_members(homeroom.id)
            .expect("homeroom members")
            .len(),
        2
    );

    let french_members = project
        .db()
        .people()
        .student_group_members(french.id)
        .expect("french members");
    assert_eq!(french_members.len(), 1);
    assert_eq!(french_members[0].id, first.id);
}

#[test]
fn a_member_can_be_added_and_removed_without_touching_the_student() {
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Ravi", "Menon"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Set 1", GroupKind::TeachingSet))
        .expect("create group");

    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: set.id,
            student_id: pupil.id,
        })
        .expect("add member");

    tests_support::remove_group_member_here(project.db(), set.id, pupil.id).expect("remove member");

    assert_eq!(
        project
            .db()
            .people()
            .student_group_members(set.id)
            .expect("members"),
        Vec::new(),
        "the group must be empty once its only member is removed"
    );
    assert!(
        project
            .db()
            .people()
            .student(pupil.id)
            .expect("read student")
            .is_some(),
        "removing a membership must not remove the student"
    );
}

#[test]
fn a_student_who_leaves_keeps_every_group_they_were_in() {
    // The whole argument for `is_active` existing. If deactivation dropped memberships it
    // would be a soft delete wearing a different name, and the record it was meant to
    // preserve -- the only evidence of who was taught in which set, until Phase 3's
    // `enrolment` -- would be the exact thing it destroyed.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Ines", "Varga"))
        .expect("create student");

    for (name, kind) in [
        ("8A Homeroom", GroupKind::Homeroom),
        ("French elective", GroupKind::ElectiveSet),
    ] {
        let created = project
            .db()
            .people()
            .create_student_group(&group(school_id, name, kind))
            .expect("create group");
        project
            .db()
            .people()
            .add_student_group_member(&StudentGroupMemberInput {
                student_group_id: created.id,
                student_id: pupil.id,
            })
            .expect("add member");
    }

    let left = project
        .db()
        .people()
        .update_student(
            pupil.id,
            Expected::Any,
            &StudentInput {
                is_active: false,
                ..student(school_id, "Ines", "Varga")
            },
        )
        .expect("deactivate student");
    assert!(!left.is_active);
    assert_eq!(left.id, pupil.id, "deactivating must not re-identify");

    assert_eq!(
        project
            .db()
            .people()
            .groups_for_student(pupil.id)
            .expect("groups for student")
            .len(),
        2,
        "deactivation must not take the memberships with it"
    );

    // And still there tomorrow, which is the half a struct-level assertion cannot see.
    project.reopen();

    let reopened = project
        .db()
        .people()
        .student(pupil.id)
        .expect("read student")
        .expect("the student must survive a reopen");
    assert!(!reopened.is_active, "the state must survive the reopen too");
    assert_eq!(
        project
            .db()
            .people()
            .groups_for_student(pupil.id)
            .expect("groups for student")
            .len(),
        2
    );

    // An inactive student is still listed, for the reason an inactive teacher is: a
    // timetable that names them is still a timetable.
    let listed = tests_support::students_here(project.db());
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_active);
}

#[test]
fn the_same_student_cannot_be_added_to_one_group_twice() {
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Omar", "Aziz"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Set 2", GroupKind::TeachingSet))
        .expect("create group");

    let input = StudentGroupMemberInput {
        student_group_id: set.id,
        student_id: pupil.id,
    };
    project
        .db()
        .people()
        .add_student_group_member(&input)
        .expect("add member");

    let repeat = project.db().people().add_student_group_member(&input);
    assert!(
        matches!(repeat, Err(DbError::DuplicateValue { .. })),
        "a repeated membership must be refused, got {repeat:?}"
    );
}

// ----------------------------------------------------------------- safe destruction

#[test]
fn deleting_a_student_who_is_still_in_a_group_is_refused() {
    // The brief's "do not silently delete group memberships", at its sharpest: the student
    // is the one record whose removal would take history with it, so it is refused and
    // deactivation is what the interface offers instead.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Hana", "Okada"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Set 3", GroupKind::TeachingSet))
        .expect("create group");
    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: set.id,
            student_id: pupil.id,
        })
        .expect("add member");

    let refused = tests_support::delete_student_here(project.db(), pupil.id);
    assert!(
        matches!(refused, Err(DbError::StillReferenced { .. })),
        "expected a refusal, got {refused:?}"
    );

    assert!(
        project
            .db()
            .people()
            .student(pupil.id)
            .expect("read student")
            .is_some(),
        "the refused delete must leave the student in place"
    );
    assert_eq!(
        project
            .db()
            .people()
            .student_group_member_count(set.id)
            .expect("count"),
        1,
        "and must leave the membership in place too"
    );
}

#[test]
fn deleting_a_group_removes_its_memberships_but_not_its_students() {
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Elif", "Demir"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Set 4", GroupKind::TeachingSet))
        .expect("create group");
    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: set.id,
            student_id: pupil.id,
        })
        .expect("add member");

    // What the confirmation dialogue reads before it asks.
    assert_eq!(
        project
            .db()
            .people()
            .student_group_member_count(set.id)
            .expect("count"),
        1
    );

    tests_support::delete_student_group_here(project.db(), set.id).expect("delete group");

    assert!(
        project
            .db()
            .people()
            .student(pupil.id)
            .expect("read student")
            .is_some(),
        "deleting a group must never delete its students"
    );
    assert_eq!(
        project
            .db()
            .people()
            .groups_for_student(pupil.id)
            .expect("groups"),
        Vec::new(),
        "the student's membership of the deleted group must go with it"
    );
}

#[test]
fn deleting_a_group_that_was_split_into_others_is_refused() {
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let cohort = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Grade 9", GroupKind::Cohort))
        .expect("cohort");
    project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            parent_group_id: Some(cohort.id),
            ..group(school_id, "9 Set 1", GroupKind::TeachingSet)
        })
        .expect("child");

    let refused = tests_support::delete_student_group_here(project.db(), cohort.id);
    assert!(
        matches!(refused, Err(DbError::StillReferenced { .. })),
        "flattening a hierarchy is not a side effect; got {refused:?}"
    );
}

#[test]
fn a_refused_group_delete_destroys_nothing_on_its_way_to_refusing() {
    // `delete_student_group` refuses, clears availability, removes memberships and then
    // deletes -- four steps in one transaction, of which only the first can say no. The
    // property worth pinning is that the no happens *before* any of the destruction and
    // that the transaction would undo it anyway: a half-applied delete would leave a
    // cohort whose members had quietly gone.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let cohort = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Grade 9", GroupKind::Cohort))
        .expect("cohort");
    project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            parent_group_id: Some(cohort.id),
            ..group(school_id, "9 Set 1", GroupKind::TeachingSet)
        })
        .expect("child");

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Ines", "Varga"))
        .expect("create student");
    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: cohort.id,
            student_id: pupil.id,
        })
        .expect("add member");

    let refused = tests_support::delete_student_group_here(project.db(), cohort.id);
    assert!(
        matches!(refused, Err(DbError::StillReferenced { .. })),
        "got {refused:?}"
    );

    assert_eq!(
        project
            .db()
            .people()
            .student_group_members(cohort.id)
            .expect("members")
            .len(),
        1,
        "a refused delete must not have taken the membership with it"
    );
    assert!(
        project
            .db()
            .people()
            .student_group(cohort.id)
            .expect("group")
            .is_some()
    );
}

#[test]
fn a_group_cannot_become_its_own_ancestor() {
    // SQLite can refuse the one-step case with a CHECK. The two-step case is why the
    // repository walks the chain: without it, resolving the hierarchy never terminates.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let top = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Upper School", GroupKind::Cohort))
        .expect("top");
    let middle = project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            parent_group_id: Some(top.id),
            ..group(school_id, "Grade 10", GroupKind::Cohort)
        })
        .expect("middle");

    let looped = project.db().people().update_student_group(
        top.id,
        Expected::Any,
        &StudentGroupInput {
            parent_group_id: Some(middle.id),
            ..group(school_id, "Upper School", GroupKind::Cohort)
        },
    );

    assert!(
        matches!(looped, Err(DbError::Invalid { .. })),
        "expected the cycle to be refused, got {looped:?}"
    );
}

// -------------------------------------------------------------------- what persists

#[test]
fn people_and_memberships_survive_closing_and_reopening_the_project() {
    // A local-first application's real persistence test. Everything above runs against one
    // open handle; this one proves the file, not the struct.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let taught_by = project
        .db()
        .people()
        .create_teacher(&TeacherInput {
            code: Some("T-100".to_owned()),
            fte_percent: 60,
            ..teacher(school_id, "Grace", "Abiola")
        })
        .expect("create teacher");
    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Noor", "Rahimi"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            size: Some(24),
            ..group(school_id, "10 History", GroupKind::TeachingSet)
        })
        .expect("create group");
    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: set.id,
            student_id: pupil.id,
        })
        .expect("add member");

    project.reopen();

    let teacher_after = project
        .db()
        .people()
        .teacher(taught_by.id)
        .expect("read teacher")
        .expect("teacher must survive");
    assert_eq!(teacher_after, taught_by, "every column must round-trip");

    let members = project
        .db()
        .people()
        .student_group_members(set.id)
        .expect("members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, pupil.id);

    let group_after = project
        .db()
        .people()
        .student_group(set.id)
        .expect("read group")
        .expect("group must survive");
    assert_eq!(group_after.size, Some(24));
    assert_eq!(group_after.kind, GroupKind::TeachingSet);
}

/// Writes a project exactly as a V9 build would have left it: a school and a campus, and
/// no People tables at all.
///
/// Raw SQL past the repositories, because that is the only way to produce rows a build
/// without V10 could have written. Same technique and justification as
/// `tests_cycle_kinds::write_a_pre_v9_project`.
fn write_a_pre_v10_project(
    path: &std::path::Path,
    site: Uuid,
    school: Uuid,
    campus: Uuid,
    stamp: &str,
) {
    let mut conn = crate::connection::open_configured(path, site).expect("open");

    let version = crate::migrations::run_up_to(&mut conn, 9).expect("migrate to V9");
    assert_eq!(version, 9, "the fixture must stop at the previous version");

    conn.execute(
        "INSERT INTO school (id, name, timezone, locale, working_days,
                             created_at, updated_at, rev)
         VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', 31, ?, ?, 3)",
        (school.to_string(), stamp, stamp),
    )
    .expect("write a school the old build could have written");
    conn.execute(
        "INSERT INTO campus (id, school_id, name, code, address, created_at, updated_at, rev)
         VALUES (?, ?, 'Main', 'MAIN', NULL, ?, ?, 2)",
        (campus.to_string(), school.to_string(), stamp, stamp),
    )
    .expect("write a campus the old build could have written");
}

/// A project written before V10 gains the People tables and loses nothing it had.
///
/// The one migration path every existing project will actually take, and the one that
/// cannot be corrected afterwards: a released migration is immutable, so a V10 that
/// rewrote a timestamp, consumed a revision or failed outright would be unfixable in
/// place. Every earlier phase has this test; the People group arrived without one, and
/// `a_project_made_before_the_people_group_existed_still_opens` below does not stand in
/// for it — that one creates the file at the *current* version and reopens it, which
/// proves idempotence rather than migration.
#[test]
fn a_project_written_before_v10_migrates_without_touching_what_it_had() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000010").expect("valid uuid");

    let school = Uuid::now_v7();
    let campus = Uuid::now_v7();
    let stamp = "2026-10-01T09:00:00.000Z";

    write_a_pre_v10_project(&path, site, school, campus, stamp);

    // Opening it with this build migrates it, in one transaction, all the way.
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("open project");
    assert_eq!(db.schema_version(), crate::supported_schema_version());

    let kept = db
        .structure()
        .school(school)
        .expect("read school")
        .expect("the school survived");
    assert_eq!(kept.id, school, "its identifier is the one it had");
    assert_eq!(kept.name, "Northgate Secondary");
    assert_eq!(kept.rev, 3, "the migration consumed no revision");
    assert_eq!(kept.created_at, stamp);
    assert_eq!(kept.updated_at, stamp, "and rewrote no timestamp");

    let campuses = db.structure().campuses(school).expect("read campuses");
    assert_eq!(campuses.len(), 1);
    assert_eq!(campuses[0].id, campus);
    assert_eq!(campuses[0].code, "MAIN");
    assert_eq!(campuses[0].rev, 2);

    // The new tables arrive empty rather than missing, and are writable at once.
    assert_eq!(db.people().teachers(school).expect("teachers"), Vec::new());
    assert_eq!(db.people().students(school).expect("students"), Vec::new());
    assert_eq!(
        db.people().student_groups(school).expect("student groups"),
        Vec::new()
    );

    let hired = db
        .people()
        .create_teacher(&teacher(school, "Amara", "Okafor"))
        .expect("the migrated project must accept a teacher");

    // And the change-log triggers were installed by the migration, not only by a create:
    // a table added to an existing file is the case where `change_log::install` is easiest
    // to forget, and the hole it leaves is silent.
    let tables: Vec<String> = db
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .into_iter()
        .map(|change| change.entity_table)
        .collect();
    assert!(
        tables.iter().any(|name| name == "teacher"),
        "the teacher write must reach the change feed, got {tables:?}"
    );
    assert_eq!(
        db.people()
            .teacher(hired.id)
            .expect("read back")
            .expect("the teacher is there")
            .rev,
        1
    );
}

#[test]
fn a_project_made_before_the_people_group_existed_still_opens() {
    // Existing-project compatibility. V10 only adds tables, so a file created by an
    // earlier build gains them on open and loses nothing it already had.
    let mut project = Project::new();
    let (school_id, campus_id) = project.with_school();

    project.reopen();

    let school = tests_support::only_school(project.db()).expect("a school exists");
    assert_eq!(school.id, school_id);
    assert_eq!(tests_support::campuses_here(project.db()).len(), 1);
    assert_eq!(
        project
            .db()
            .structure()
            .campus(campus_id)
            .expect("campus")
            .expect("campus must survive")
            .code,
        "MAIN"
    );

    // And the new tables are present and empty rather than missing.
    assert_eq!(
        tests_support::teachers_here(project.db()),
        Vec::new(),
        "the new tables must be present and empty, not missing"
    );
    assert_eq!(tests_support::students_here(project.db()), Vec::new());
}

#[test]
fn every_people_write_reaches_the_change_feed() {
    // The change log is what keeps future synchronisation possible, and a table whose
    // writes are absent from it is a silent hole. `student_group_member` is the one at
    // risk: it would have been a composite-key junction, which the triggers cannot see.
    let mut project = Project::new();
    let (school_id, _) = project.with_school();

    let before = project
        .db()
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .len();

    let pupil = project
        .db()
        .people()
        .create_student(&student(school_id, "Kofi", "Mensah"))
        .expect("create student");
    let set = project
        .db()
        .people()
        .create_student_group(&group(school_id, "Set 5", GroupKind::TeachingSet))
        .expect("create group");
    project
        .db()
        .people()
        .add_student_group_member(&StudentGroupMemberInput {
            student_group_id: set.id,
            student_id: pupil.id,
        })
        .expect("add member");
    tests_support::remove_group_member_here(project.db(), set.id, pupil.id).expect("remove member");

    let tables: Vec<String> = project
        .db()
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .into_iter()
        .skip(before)
        .map(|change| change.entity_table)
        .collect();

    assert!(tables.iter().any(|name| name == "student"));
    assert!(tables.iter().any(|name| name == "student_group"));
    assert_eq!(
        tables
            .iter()
            .filter(|name| *name == "student_group_member")
            .count(),
        2,
        "both the insert and the delete must be recorded, got {tables:?}"
    );
}
