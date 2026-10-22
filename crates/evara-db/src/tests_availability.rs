//! Phase 2 tests: availability.
//!
//! Real temporary project files throughout, never `:memory:` — the RESTRICT edges, the
//! `COALESCE` identity index and the `CHECK` that a rule must name some time are all
//! properties of a real database file.
//!
//! The questions worth asking here are not "can I store a rule" but:
//!
//! - does *available* stay the default, so "unconfigured" and "never free" differ
//! - does editing a cell keep the rule's identity, or quietly make a new one
//! - do two rules meaning the same thing collide, given every scope column is nullable
//! - can a cycle-day deletion take availability with it without anyone being told
//! - does a polymorphic reference, which no foreign key covers, actually stay honest

use std::path::PathBuf;

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::availability::{AvailabilityEntity, AvailabilityRuleInput, AvailabilityStatus};
use crate::repo::people::{GroupKind, StudentGroupInput, TeacherInput};
use crate::repo::structure::{CampusInput, RoomInput, SchoolInput};
use crate::repo::time_model::{CycleInput, PeriodInput, PeriodStructureInput};
use crate::tests_support;
use crate::workspace::Workspace;
use crate::{CycleKind, Database, DbError, PROJECT_EXTENSION, PeriodKind};

const APP_VERSION: &str = "0.1.0-test";
const CHANGE_FEED_LIMIT: i64 = 1_000;

/// An open project in its own temporary directory.
struct Project {
    _dir: TempDir,
    workspace: Workspace,
    path: PathBuf,
}

/// The identifiers a test needs to write a rule.
struct Fixture {
    school_id: Uuid,
    campus_id: Uuid,
    teacher_id: Uuid,
    cycle_day_ids: Vec<Uuid>,
    period_ids: Vec<Uuid>,
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

    fn reopen(&mut self) {
        assert!(
            self.workspace.close_project(),
            "the project must have been open"
        );
        self.workspace.open_project(&self.path).expect("reopen");
    }

    /// A school, a teacher, and a cycle and bell schedule to hang rules on.
    ///
    /// `kind` decides the cycle: a `WEEKLY` cycle lays out its own seven positions, a
    /// `CUSTOM` one takes the length given. Both are exercised, because availability must
    /// not care which the school chose.
    fn with_timetable_shape(&mut self, kind: CycleKind, day_count: i64) -> Fixture {
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

        let teacher = self
            .db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: school.id,
                code: None,
                given_name: "Amara".to_owned(),
                family_name: "Okafor".to_owned(),
                email: None,
                home_campus_id: Some(campus.id),
                fte_percent: 100,
                max_periods_per_day: None,
                max_periods_per_cycle: None,
                max_consecutive_periods: None,
                notes: None,
                is_active: true,
            })
            .expect("create teacher");

        let cycle = self
            .db()
            .time_model()
            .create_cycle(&CycleInput {
                school_id: school.id,
                name: "Main cycle".to_owned(),
                kind,
                day_count,
                week_count: 1,
                is_default: true,
            })
            .expect("create cycle");

        let cycle_day_ids = self
            .db()
            .time_model()
            .cycle_days(cycle.id)
            .expect("cycle days")
            .into_iter()
            .map(|day| day.id)
            .collect::<Vec<_>>();

        let structure = self
            .db()
            .time_model()
            .create_period_structure(&PeriodStructureInput {
                school_id: school.id,
                campus_id: None,
                name: "Standard day".to_owned(),
                is_default: true,
            })
            .expect("create period structure");

        let mut period_ids = Vec::new();
        for ordinal in 1..=3 {
            let period = self
                .db()
                .time_model()
                .create_period(&PeriodInput {
                    period_structure_id: structure.id,
                    ordinal,
                    label: format!("P{ordinal}"),
                    starts_at: format!("{:02}:00", 8 + ordinal),
                    ends_at: format!("{:02}:50", 8 + ordinal),
                    kind: PeriodKind::Teaching,
                    counts_as_load: true,
                })
                .expect("create period");
            period_ids.push(period.id);
        }

        Fixture {
            school_id: school.id,
            campus_id: campus.id,
            teacher_id: teacher.id,
            cycle_day_ids,
            period_ids,
        }
    }
}

/// A rule about one recurring cell — what the grid editor writes.
fn cell(
    fixture: &Fixture,
    day: usize,
    period: usize,
    status: AvailabilityStatus,
) -> AvailabilityRuleInput {
    AvailabilityRuleInput {
        school_id: fixture.school_id,
        entity_kind: AvailabilityEntity::Teacher,
        entity_id: fixture.teacher_id,
        timeslot_id: None,
        cycle_day_id: Some(fixture.cycle_day_ids[day]),
        period_id: Some(fixture.period_ids[period]),
        term_id: None,
        status,
        weight: None,
        reason: None,
        effective_from: None,
        effective_to: None,
    }
}

// ------------------------------------------------------------------ the default state

#[test]
fn an_entity_with_no_rules_is_available_everywhere() {
    // The design's load-bearing claim: available is the absence of a row. If this ever
    // became "no rules means no availability", every unconfigured teacher would become
    // unschedulable the moment the solver arrives.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Weekly, 7);

    assert_eq!(
        tests_support::rules_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        Vec::new()
    );
    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        0
    );
}

// ------------------------------------------------------------------ writing and editing

#[test]
fn a_cell_can_be_made_unavailable_and_cleared_again() {
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 6);

    let rule = project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("set rule");

    assert_eq!(rule.status, AvailabilityStatus::Unavailable);
    assert_eq!(rule.rev, 1);
    assert_eq!(rule.term_id, None, "a rule with no term applies all year");

    let removed = project
        .db()
        .availability()
        .clear(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("clear");
    assert!(removed);

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        0
    );
}

#[test]
fn clearing_a_cell_that_was_already_available_is_not_an_error() {
    // The grid sends the same request whichever way a cell was toggled. "There was nothing
    // to remove" is the right outcome, not a failure the user has to read.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let removed = project
        .db()
        .availability()
        .clear(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("clear");

    assert!(!removed, "nothing was there, and that is not a failure");
}

#[test]
fn editing_a_cell_keeps_the_rule_rather_than_replacing_it() {
    // Delete-then-insert would have been simpler and would make every edit look like a new
    // rule in the change feed, which is the opposite of what a change feed is for.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let first = project
        .db()
        .availability()
        .set(&cell(&fixture, 1, 2, AvailabilityStatus::Unavailable))
        .expect("set rule");

    let second = project
        .db()
        .availability()
        .set(&cell(&fixture, 1, 2, AvailabilityStatus::Discouraged))
        .expect("update rule");

    assert_eq!(second.id, first.id, "the same rule, edited");
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.rev, 2, "an edit consumes a revision");
    assert_eq!(second.status, AvailabilityStatus::Discouraged);

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        1,
        "editing must not leave a second rule behind"
    );
}

#[test]
fn the_three_granularities_are_separate_rules() {
    // "All of Monday", "period 1 every day" and one exact cell are different statements,
    // and the identity index must not fold them together.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let whole_day = AvailabilityRuleInput {
        period_id: None,
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    };
    let whole_period = AvailabilityRuleInput {
        cycle_day_id: None,
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    };

    project
        .db()
        .availability()
        .set(&whole_day)
        .expect("day rule");
    project
        .db()
        .availability()
        .set(&whole_period)
        .expect("period rule");
    project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("cell rule");

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        3
    );

    // And clearing the cell is a statement about the cell alone. The grid editor writes
    // and clears this one granularity, so if a clear reached wider than its own scope it
    // would silently discard rules no screen in this phase can even display.
    assert!(
        project
            .db()
            .availability()
            .clear(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
            .expect("clear the cell")
    );

    let left = tests_support::rules_here(
        project.db(),
        AvailabilityEntity::Teacher,
        fixture.teacher_id,
    );
    assert_eq!(left.len(), 2, "only the cell was cleared");
    assert!(
        left.iter()
            .any(|rule| rule.period_id.is_none() && rule.cycle_day_id.is_some()),
        "the whole-day rule must survive"
    );
    assert!(
        left.iter()
            .any(|rule| rule.cycle_day_id.is_none() && rule.period_id.is_some()),
        "the whole-period rule must survive"
    );
}

#[test]
fn two_rules_meaning_the_same_thing_are_one_rule() {
    // Every scope column is nullable, and SQLite treats NULLs as distinct. Without the
    // COALESCE identity index this constraint would silently fail for exactly the common
    // case — a rule with no term and no timeslot.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let whole_day = AvailabilityRuleInput {
        period_id: None,
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    };

    let first = project.db().availability().set(&whole_day).expect("first");
    let again = project
        .db()
        .availability()
        .set(&AvailabilityRuleInput {
            status: AvailabilityStatus::Preferred,
            ..whole_day
        })
        .expect("second");

    assert_eq!(again.id, first.id);
    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        1
    );
}

// -------------------------------------------------------------------- invalid input

#[test]
fn a_rule_that_names_no_time_is_refused() {
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let nothing = AvailabilityRuleInput {
        cycle_day_id: None,
        period_id: None,
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    };

    let refused = project.db().availability().set(&nothing);
    assert!(
        matches!(refused, Err(DbError::Invalid { .. })),
        "a rule about no time at all is not a rule; got {refused:?}"
    );
}

#[test]
fn a_rule_about_an_entity_that_does_not_exist_is_refused() {
    // `entity_id` is polymorphic, so no foreign key covers it. This check is what stands in
    // for one, and without it an orphan would be created by the write itself.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let refused = project.db().availability().set(&AvailabilityRuleInput {
        entity_id: Uuid::now_v7(),
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    });

    assert!(
        matches!(refused, Err(DbError::NotFound { .. })),
        "expected a refusal, got {refused:?}"
    );
}

#[test]
fn a_rule_about_a_subject_is_refused_until_subjects_exist() {
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let refused = project.db().availability().set(&AvailabilityRuleInput {
        entity_kind: AvailabilityEntity::Subject,
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    });

    assert!(
        matches!(refused, Err(DbError::Invalid { .. })),
        "accepting this would create an orphan by construction; got {refused:?}"
    );
}

#[test]
fn a_rule_naming_a_cycle_day_from_another_project_is_refused() {
    // The time references *are* enforced, unlike entity_id. A bad one must fail at the
    // foreign key rather than being stored and resolving to nothing later.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let refused = project.db().availability().set(&AvailabilityRuleInput {
        cycle_day_id: Some(Uuid::now_v7()),
        ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
    });

    assert!(refused.is_err(), "expected a refusal, got {refused:?}");
}

// --------------------------------------------------------------- historical safety

#[test]
fn deleting_a_cycle_day_that_availability_depends_on_is_refused() {
    // The brief's "avoid deleting availability silently when cycle configuration changes",
    // held by the schema rather than by a check someone has to remember to write.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("set rule");

    let refused = tests_support::delete_cycle_day_here(project.db(), fixture.cycle_day_ids[0]);

    assert!(
        matches!(refused, Err(DbError::StillReferenced { .. })),
        "expected the day's deletion to be refused, got {refused:?}"
    );

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        1,
        "and the rule must still be there"
    );
}

#[test]
fn deactivating_a_teacher_keeps_their_availability() {
    // Deactivation is not deletion. A teacher who returns next year should find their
    // Friday afternoons still blocked out.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    project
        .db()
        .availability()
        .set(&cell(&fixture, 2, 1, AvailabilityStatus::Unavailable))
        .expect("set rule");

    let teacher = project
        .db()
        .people()
        .teacher(fixture.teacher_id)
        .expect("read teacher")
        .expect("teacher");

    project
        .db()
        .people()
        .update_teacher(
            teacher.id,
            Expected::Any,
            &TeacherInput {
                school_id: fixture.school_id,
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
                is_active: false,
            },
        )
        .expect("deactivate");

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        1,
        "deactivation must not touch availability"
    );
}

#[test]
fn deleting_a_teacher_takes_their_availability_and_leaves_no_orphan() {
    // The polymorphic reference has no foreign key, so this is the one case where the code
    // is the only thing standing between a delete and a dangling row.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("set rule");
    project
        .db()
        .availability()
        .set(&cell(&fixture, 1, 1, AvailabilityStatus::Discouraged))
        .expect("set rule");

    tests_support::delete_teacher_here(project.db(), fixture.teacher_id).expect("delete teacher");

    assert_eq!(
        tests_support::count_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        0
    );
    assert_eq!(
        project.db().availability().orphans().expect("orphans"),
        Vec::new(),
        "the integrity sweep must find nothing"
    );
}

// --------------------------------------------------------------------- what persists

#[test]
fn availability_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 6);

    let stored = project
        .db()
        .availability()
        .set(&AvailabilityRuleInput {
            weight: Some(40),
            reason: Some("Part-time Wednesdays".to_owned()),
            ..cell(&fixture, 2, 0, AvailabilityStatus::Discouraged)
        })
        .expect("set rule");

    project.reopen();

    let after = tests_support::rules_here(
        project.db(),
        AvailabilityEntity::Teacher,
        fixture.teacher_id,
    );

    assert_eq!(after.len(), 1);
    assert_eq!(after[0], stored, "every column must round-trip");
    assert_eq!(after[0].weight, Some(40));
    assert_eq!(after[0].reason.as_deref(), Some("Part-time Wednesdays"));
}

#[test]
fn availability_works_the_same_on_a_weekly_and_a_custom_cycle() {
    // Nothing in availability may read a weekday. A rule is about a cycle *position*, and
    // the two cycle kinds differ only in who chose those positions.
    for (kind, day_count, expected_days) in [(CycleKind::Weekly, 7, 7), (CycleKind::Custom, 6, 6)] {
        let mut project = Project::new();
        let fixture = project.with_timetable_shape(kind, day_count);

        assert_eq!(
            fixture.cycle_day_ids.len(),
            expected_days,
            "the fixture itself must reflect the cycle kind"
        );

        for day in 0..expected_days {
            project
                .db()
                .availability()
                .set(&cell(&fixture, day, 0, AvailabilityStatus::Unavailable))
                .expect("set rule");
        }

        assert_eq!(
            tests_support::count_here(
                project.db(),
                AvailabilityEntity::Teacher,
                fixture.teacher_id
            ),
            i64::try_from(expected_days).expect("small"),
            "one rule per position, whichever kind of cycle it is"
        );
    }
}

#[test]
fn a_project_made_before_availability_existed_still_opens() {
    // V11 only adds a table, so a file created by an earlier build gains it on open and
    // loses nothing it already had.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    project.reopen();

    assert!(
        project
            .db()
            .people()
            .teacher(fixture.teacher_id)
            .expect("read teacher")
            .is_some(),
        "everything from before must survive"
    );
    assert_eq!(
        tests_support::rules_here(
            project.db(),
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        Vec::new(),
        "and the new table is present and empty rather than missing"
    );
}

#[test]
fn every_availability_write_reaches_the_change_feed() {
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);

    let before = project
        .db()
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .len();

    project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Unavailable))
        .expect("insert");
    project
        .db()
        .availability()
        .set(&cell(&fixture, 0, 0, AvailabilityStatus::Preferred))
        .expect("update");
    project
        .db()
        .availability()
        .clear(&cell(&fixture, 0, 0, AvailabilityStatus::Preferred))
        .expect("delete");

    let ops: Vec<String> = project
        .db()
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .into_iter()
        .skip(before)
        .filter(|change| change.entity_table == "availability")
        .map(|change| change.op)
        .collect();

    assert_eq!(
        ops,
        vec![
            "INSERT".to_owned(),
            "UPDATE".to_owned(),
            "DELETE".to_owned()
        ],
        "the insert, the edit and the removal must each be recorded"
    );
}

// --------------------------------------------------------------- the other two kinds

/// A room on the fixture's campus.
fn room_on(project: &mut Project, fixture: &Fixture) -> Uuid {
    project
        .db()
        .structure()
        .create_room(
            &RoomInput {
                campus_id: fixture.campus_id,
                building_id: None,
                room_type_id: None,
                name: "Science 1".to_owned(),
                code: "S1".to_owned(),
                capacity: 30,
                is_bookable: true,
                notes: None,
            },
            &[],
        )
        .expect("create room")
        .id
}

/// A class group in the fixture's school.
fn group_in(project: &mut Project, fixture: &Fixture) -> Uuid {
    project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            school_id: fixture.school_id,
            name: "Grade 8A".to_owned(),
            code: None,
            kind: GroupKind::Class,
            year_level_id: None,
            parent_group_id: None,
            size: None,
            campus_id: None,
            children_are_disjoint: None,
        })
        .expect("create group")
        .id
}

#[test]
fn a_room_and_a_group_carry_their_own_rules_and_take_them_when_deleted() {
    // Every other test in this file is about a teacher, and the interface wires the same
    // grid to Teachers, Rooms and Student groups. The three share one table and one
    // repository, so what is worth testing is not that each can store a rule but that the
    // *polymorphic* reference keeps them apart -- and that the two deletes which had to
    // learn to clear it do, since no foreign key will do it for them.
    let mut project = Project::new();
    let fixture = project.with_timetable_shape(CycleKind::Custom, 5);
    let room_id = room_on(&mut project, &fixture);
    let group_id = group_in(&mut project, &fixture);

    // The same cell, for all three kinds. Three rules, not one: the identity index leads
    // on entity_kind, and a room being busy says nothing about a teacher.
    let everyone = [
        (AvailabilityEntity::Teacher, fixture.teacher_id),
        (AvailabilityEntity::Room, room_id),
        (AvailabilityEntity::StudentGroup, group_id),
    ];
    for (kind, id) in everyone {
        project
            .db()
            .availability()
            .set(&AvailabilityRuleInput {
                entity_kind: kind,
                entity_id: id,
                ..cell(&fixture, 0, 0, AvailabilityStatus::Unavailable)
            })
            .expect("set rule");
    }

    project.reopen();

    for (kind, id) in everyone {
        assert_eq!(
            count(&mut project, kind, id),
            1,
            "{kind:?} must keep its own rule across a reopen"
        );
    }

    // Deleting either one is where a polymorphic reference goes wrong silently: the row
    // stays, points at nothing, and the solver later reads it as a restriction on an
    // entity that no longer exists.
    tests_support::delete_room_here(project.db(), room_id).expect("delete room");
    tests_support::delete_student_group_here(project.db(), group_id).expect("delete group");

    assert_eq!(count(&mut project, AvailabilityEntity::Room, room_id), 0);
    assert_eq!(
        count(&mut project, AvailabilityEntity::StudentGroup, group_id),
        0
    );
    assert_eq!(
        count(
            &mut project,
            AvailabilityEntity::Teacher,
            fixture.teacher_id
        ),
        1,
        "and the teacher's rule is none of their business"
    );
    assert!(
        project
            .db()
            .availability()
            .orphans()
            .expect("orphans")
            .is_empty(),
        "no rule may outlive the thing it is about"
    );
}

/// How many rules one entity has.
fn count(project: &mut Project, kind: AvailabilityEntity, id: Uuid) -> i64 {
    tests_support::count_here(project.db(), kind, id)
}
