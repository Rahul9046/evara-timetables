//! M5 tests: a configured rule is stored, scoped to its school, and refused when it would
//! weaken a safety check.
//!
//! Real temporary project files throughout, never `:memory:` — the RESTRICT edges, the
//! same-school trigger pair on `term_id` and the delete trigger that insists scope rows go
//! first are all properties of a real database file.
//!
//! The questions worth asking are not "can I store a rule":
//!
//! - does a row that would disable or weaken a mandatory rule get refused, rather than stored
//!   and then refused by the engine when the next solve starts
//! - does an instance of one school stay invisible and untouchable from another
//! - does a revision check actually stop a second window's save
//! - do scope rows go with their instance, each as its own change-log entry
//! - can a constraint make `school_dependents` disagree with what a delete does

use std::path::PathBuf;

use evara_core::calendar::WorkingDays;
use evara_core::scheduling::{ConstraintLevel, ConstraintType, EntityKind, ScopeRef};
use tempfile::TempDir;
use uuid::Uuid;

use crate::repo::Expected;
use crate::repo::constraints::{ConstraintInstanceInput, ScopeEntity, ScopeRole};
use crate::repo::structure::{AcademicYearInput, SchoolInput, TermInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";
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

    fn reopen(&mut self) {
        assert!(
            self.workspace.close_project(),
            "the project must have been open"
        );
        self.workspace.open_project(&self.path).expect("reopen");
    }

    /// A school, with nothing else.
    fn school(&mut self, name: &str) -> Uuid {
        self.db()
            .structure()
            .create_school(&SchoolInput {
                name: name.to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::from_bits(0b001_1111).expect("weekdays"),
            })
            .expect("create school")
            .id
    }

    /// A term of one school, for the scoping tests.
    fn term(&mut self, school_id: Uuid, name: &str) -> Uuid {
        let year = self
            .db()
            .structure()
            .create_academic_year(&AcademicYearInput {
                school_id,
                name: format!("{name} year"),
                starts_on: "2026-09-01".to_owned(),
                ends_on: "2027-07-20".to_owned(),
            })
            .expect("create academic year");
        self.db()
            .structure()
            .create_term(&TermInput {
                academic_year_id: year.id,
                name: name.to_owned(),
                ordinal: 1,
                starts_on: "2026-09-01".to_owned(),
                ends_on: "2026-12-18".to_owned(),
            })
            .expect("create term")
            .id
    }
}

/// A tuning of one rule: the shipped level, weight 1, no parameters.
fn tuning(school_id: Uuid, constraint_type: ConstraintType) -> ConstraintInstanceInput {
    ConstraintInstanceInput {
        school_id,
        type_key: constraint_type.key().to_owned(),
        level: constraint_type.default_level().stored().to_owned(),
        weight: 1,
        enabled: true,
        params: "{}".to_owned(),
        term_id: None,
    }
}

// =======================================================================================
// Storing and reading
// =======================================================================================

#[test]
fn a_configured_rule_is_stored_and_survives_a_reopen() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.weight = 25;
    let created = project
        .db()
        .constraints()
        .create(&input)
        .expect("create a constraint");

    assert_eq!(created.type_key, "teacher-clash");
    assert_eq!(created.level, "HARD");
    assert_eq!(created.weight, 25);
    assert!(created.enabled);
    assert_eq!(created.params, "{}");
    assert_eq!(created.rev, 1, "a new row starts at revision 1");
    assert_eq!(created.created_at, created.updated_at);

    project.reopen();
    let held = project
        .db()
        .constraints()
        .instance(created.id)
        .expect("read back")
        .expect("still there");
    assert_eq!(held, created);
}

#[test]
fn a_tuned_parameter_is_stored_as_written() {
    // The shape `evara-constraints` parses. This crate stores the text and never interprets
    // it — the schema lives beside the evaluator, in a crate `evara-db` may not depend on.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut input = tuning(school, ConstraintType::DurationContiguous);
    input.params = "{\"allowBreakSpan\": true}".to_owned();
    let created = project.db().constraints().create(&input).expect("create");

    assert_eq!(created.params, "{\"allowBreakSpan\": true}");
}

#[test]
fn a_rule_may_be_raised_but_not_weakened() {
    // ADR 0005's whole point — "schools genuinely disagree about which rules are hard" —
    // bounded by ADR 0039: the disagreement runs upwards only.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut raised = tuning(school, ConstraintType::ActivityScheduled);
    raised.level = ConstraintLevel::Hard.stored().to_owned();
    let created = project
        .db()
        .constraints()
        .create(&raised)
        .expect("a school may insist every lesson is placed");
    assert_eq!(created.level, "HARD");

    let mut weakened = tuning(school, ConstraintType::TeacherClash);
    weakened.level = ConstraintLevel::Soft.stored().to_owned();
    let error = project
        .db()
        .constraints()
        .create(&weakened)
        .expect_err("a teacher in two rooms at once is not a preference");
    let message = error.to_string();
    assert!(message.contains("teacher-clash"), "{message}");
    assert!(message.contains("HARD"), "{message}");
    assert!(message.contains("SOFT"), "{message}");
}

#[test]
fn a_mandatory_rule_cannot_be_turned_off() {
    // The refusal ADR 0039 exists for. A row that disabled a safety rule would be a timetable
    // that claims to be checked and is not, and the remedy — remove the configuration — is
    // in the message because "cannot be disabled" alone leaves somebody stuck.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    for constraint_type in ConstraintType::all() {
        let mut input = tuning(school, *constraint_type);
        input.enabled = false;
        let error = project
            .db()
            .constraints()
            .create(&input)
            .expect_err("every implemented type is mandatory");
        let message = error.to_string();
        assert!(message.contains(constraint_type.key()), "{message}");
        assert!(
            message.contains("remove the configuration"),
            "{} must name the remedy: {message}",
            constraint_type.key()
        );
    }
}

#[test]
fn an_unknown_rule_is_refused_and_the_message_lists_the_known_ones() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.type_key = "room-capacity".to_owned();
    let error = project
        .db()
        .constraints()
        .create(&input)
        .expect_err("ADR 0034 deferred room-capacity");
    let message = error.to_string();
    assert!(message.contains("room-capacity"), "{message}");
    assert!(
        message.contains("teacher-clash") && message.contains("activity-scheduled"),
        "the message has to say what it does know: {message}"
    );
}

#[test]
fn a_level_that_is_not_a_level_is_refused() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.level = "CRITICAL".to_owned();
    let message = project
        .db()
        .constraints()
        .create(&input)
        .expect_err("three levels, and that is not one")
        .to_string();
    assert!(message.contains("HARD, MEDIUM or SOFT"), "{message}");
}

#[test]
fn parameters_that_are_not_a_json_object_are_refused() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    for (params, expected) in [
        ("not json", "must be JSON"),
        ("[]", "must be a JSON object"),
        ("7", "must be a JSON object"),
        ("null", "must be a JSON object"),
    ] {
        let mut input = tuning(school, ConstraintType::DurationContiguous);
        input.params = params.to_owned();
        let message = project
            .db()
            .constraints()
            .create(&input)
            .expect_err("params is checked")
            .to_string();
        assert!(message.contains(expected), "{params}: {message}");
    }
}

#[test]
fn a_negative_weight_is_refused_by_name() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.weight = -5;
    let message = project
        .db()
        .constraints()
        .create(&input)
        .expect_err("a penalty cannot be negative")
        .to_string();
    assert!(
        message.contains("weight"),
        "the message must name the field rather than reporting a disallowed value: {message}"
    );
}

// =======================================================================================
// Schools
// =======================================================================================

#[test]
fn a_rule_belongs_to_one_school_and_is_invisible_from_the_other() {
    let mut project = Project::new();
    let north = project.school("Northgate Secondary");
    let south = project.school("Southmoor High");

    let theirs = project
        .db()
        .constraints()
        .create(&tuning(north, ConstraintType::RoomClash))
        .expect("create");

    assert_eq!(
        project.db().constraints().list(south).expect("list"),
        Vec::new(),
        "a project holds several schools and every school-owned read says which"
    );
    assert_eq!(
        project
            .db()
            .constraints()
            .list(north)
            .expect("list")
            .iter()
            .map(|held| held.id)
            .collect::<Vec<_>>(),
        vec![theirs.id]
    );
}

#[test]
fn another_school_may_not_retune_or_remove_this_school_s_rule() {
    let mut project = Project::new();
    let north = project.school("Northgate Secondary");
    let south = project.school("Southmoor High");

    let held = project
        .db()
        .constraints()
        .create(&tuning(north, ConstraintType::RoomClash))
        .expect("create");

    let mut theirs = tuning(south, ConstraintType::RoomClash);
    theirs.weight = 99;
    let error = project
        .db()
        .constraints()
        .update(held.id, Expected::Rev(held.rev), &theirs)
        .expect_err("an identifier from another school is not authority");
    assert!(matches!(error, DbError::WrongSchool { .. }), "{error:?}");

    let error = project
        .db()
        .constraints()
        .delete(south, held.id, Expected::Rev(held.rev))
        .expect_err("nor for a delete");
    assert!(matches!(error, DbError::WrongSchool { .. }), "{error:?}");

    assert!(
        project
            .db()
            .constraints()
            .instance(held.id)
            .expect("read")
            .is_some(),
        "and nothing happened to it"
    );
}

#[test]
fn a_term_from_another_school_is_refused() {
    // V16's trigger pair. `term` reaches its school through `academic_year`, so no foreign
    // key can say this.
    let mut project = Project::new();
    let north = project.school("Northgate Secondary");
    let south = project.school("Southmoor High");
    let theirs = project.term(south, "Autumn");

    let mut input = tuning(north, ConstraintType::TeacherClash);
    input.term_id = Some(theirs);
    let message = project
        .db()
        .constraints()
        .create(&input)
        .expect_err("their term, not ours")
        .to_string();
    assert!(
        message.contains("different school"),
        "a sentence naming the remedy, not the database's own words: {message}"
    );
    assert!(message.contains("terms"), "{message}");
}

#[test]
fn a_term_of_the_schools_own_is_stored() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let autumn = project.term(school, "Autumn");

    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.term_id = Some(autumn);
    let created = project.db().constraints().create(&input).expect("create");
    assert_eq!(created.term_id, Some(autumn));
}

#[test]
fn a_configured_rule_blocks_deleting_its_school_and_the_count_says_so() {
    // `constraint_instance.school_id` is RESTRICT, so without the count the screen would
    // report nothing in the way and the delete would then fail.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::TeacherClash))
        .expect("create");

    let dependents = project
        .db()
        .structure()
        .school_dependents(school)
        .expect("count dependents");
    assert_eq!(dependents.constraints, 1);
    assert!(dependents.total >= 1);

    let error = project
        .db()
        .structure()
        .delete_school(school, Expected::Rev(1))
        .expect_err("a dependent row refuses it");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "{error:?}"
    );
}

// =======================================================================================
// Retuning
// =======================================================================================

#[test]
fn retuning_advances_the_revision_and_keeps_the_identity() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::StudentClash))
        .expect("create");

    let mut changed = tuning(school, ConstraintType::StudentClash);
    changed.weight = 40;
    let updated = project
        .db()
        .constraints()
        .update(created.id, Expected::Rev(created.rev), &changed)
        .expect("retune");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.weight, 40);
    assert_eq!(updated.rev, 2);
    assert_eq!(updated.created_at, created.created_at);
}

#[test]
fn a_stale_revision_is_a_conflict_and_writes_nothing() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::StudentClash))
        .expect("create");

    let mut first = tuning(school, ConstraintType::StudentClash);
    first.weight = 10;
    project
        .db()
        .constraints()
        .update(created.id, Expected::Rev(1), &first)
        .expect("the first window saves");

    let mut second = tuning(school, ConstraintType::StudentClash);
    second.weight = 20;
    let error = project
        .db()
        .constraints()
        .update(created.id, Expected::Rev(1), &second)
        .expect_err("the second window must not overwrite the first");
    assert!(
        matches!(
            error,
            DbError::Conflict {
                expected: 1,
                actual: 2,
                ..
            }
        ),
        "{error:?}"
    );

    let held = project
        .db()
        .constraints()
        .instance(created.id)
        .expect("read")
        .expect("there");
    assert_eq!(held.weight, 10, "the first save stands");
    assert_eq!(held.rev, 2, "and the conflict consumed no revision");
}

#[test]
fn a_rules_type_cannot_be_changed_in_place() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::RoomClash))
        .expect("create");

    let message = project
        .db()
        .constraints()
        .update(
            created.id,
            Expected::Rev(created.rev),
            &tuning(school, ConstraintType::TeacherClash),
        )
        .expect_err("an instance is a configuration of one rule")
        .to_string();
    assert!(message.contains("room-clash"), "{message}");
    assert!(message.contains("teacher-clash"), "{message}");
}

#[test]
fn retuning_refuses_everything_creating_refuses() {
    // The same validation on both paths, so a row cannot be edited into a state it could not
    // have been created in.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::TeacherClash))
        .expect("create");

    let mut disabled = tuning(school, ConstraintType::TeacherClash);
    disabled.enabled = false;
    assert!(
        project
            .db()
            .constraints()
            .update(created.id, Expected::Rev(created.rev), &disabled)
            .is_err()
    );

    let mut weakened = tuning(school, ConstraintType::TeacherClash);
    weakened.level = "MEDIUM".to_owned();
    assert!(
        project
            .db()
            .constraints()
            .update(created.id, Expected::Rev(created.rev), &weakened)
            .is_err()
    );

    let held = project
        .db()
        .constraints()
        .instance(created.id)
        .expect("read")
        .expect("there");
    assert_eq!(held.rev, 1, "neither attempt consumed a revision");
}

// =======================================================================================
// Removing
// =======================================================================================

#[test]
fn removing_a_configuration_returns_the_school_to_the_shipped_rule() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let mut input = tuning(school, ConstraintType::TeacherClash);
    input.weight = 80;
    let created = project.db().constraints().create(&input).expect("create");

    project
        .db()
        .constraints()
        .delete(school, created.id, Expected::Rev(created.rev))
        .expect("remove it");

    assert_eq!(
        project.db().constraints().list(school).expect("list"),
        Vec::new()
    );
    assert_eq!(
        project.db().constraints().stored_for(school).expect("read"),
        Vec::new(),
        "and the engine is back to the mandatory set, which it always evaluated anyway"
    );
}

#[test]
fn a_stale_revision_refuses_a_delete() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::RoomTypeRequired))
        .expect("create");
    let mut changed = tuning(school, ConstraintType::RoomTypeRequired);
    changed.weight = 3;
    project
        .db()
        .constraints()
        .update(created.id, Expected::Rev(1), &changed)
        .expect("somebody edits it");

    let error = project
        .db()
        .constraints()
        .delete(school, created.id, Expected::Rev(1))
        .expect_err("a delete is checked as carefully as an update");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
    assert!(
        project
            .db()
            .constraints()
            .instance(created.id)
            .expect("read")
            .is_some()
    );
}

#[test]
fn scope_rows_go_with_their_instance_and_each_is_a_change_log_entry() {
    // The behaviour V16's deviation 2 argues for. A `CASCADE` would have taken these with no
    // entry in the change feed, because SQLite fires a DELETE trigger for a foreign-key
    // action only under `recursive_triggers`, which this project never enables.
    //
    // The scope rows are written here with SQL rather than through the repository, which
    // refuses every narrowing in M5 — this is what a project written by a newer build, or a
    // future narrowable type, leaves behind for the delete to clear.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::EntityUnavailable))
        .expect("create");
    let teacher = Uuid::now_v7();

    let scope = write_scope_row(project.db(), created.id, ScopeEntity::Teacher, teacher);
    assert_eq!(
        project
            .db()
            .constraints()
            .scope_of(created.id)
            .expect("read scope")
            .len(),
        1
    );

    let before = change_count(project.db(), "constraint_ref");
    project
        .db()
        .constraints()
        .delete(school, created.id, Expected::Rev(created.rev))
        .expect("remove the instance and its scope");

    assert_eq!(
        project
            .db()
            .constraints()
            .scope_of(created.id)
            .expect("read scope"),
        Vec::new()
    );
    assert_eq!(
        change_count(project.db(), "constraint_ref"),
        before + 1,
        "the scope row's removal is in the change feed"
    );
    let _ = scope;
}

#[test]
fn a_conflicted_delete_leaves_the_scope_rows_alone() {
    // The scope deletions share the instance's transaction, so a revision conflict on the
    // instance rolls them back. Without that, a failed delete would have silently unscoped a
    // rule.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::EntityUnavailable))
        .expect("create");
    write_scope_row(project.db(), created.id, ScopeEntity::Room, Uuid::now_v7());

    let error = project
        .db()
        .constraints()
        .delete(school, created.id, Expected::Rev(99))
        .expect_err("that is not its revision");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
    assert_eq!(
        project
            .db()
            .constraints()
            .scope_of(created.id)
            .expect("read scope")
            .len(),
        1,
        "the rule is still scoped to what it was scoped to"
    );
}

// =======================================================================================
// Scope
// =======================================================================================

#[test]
fn narrowing_a_rule_this_build_implements_is_refused_by_name() {
    // None of the eight can be narrowed, and the refusal is here rather than at compile time
    // so that one scope row somebody added cannot make a school's whole configuration
    // uncompilable.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::TeacherClash))
        .expect("create");

    let message = project
        .db()
        .constraints()
        .add_scope(
            school,
            &crate::repo::constraints::ConstraintScopeEntryInput {
                constraint_id: created.id,
                role: ScopeRole::AppliesTo,
                entity_kind: ScopeEntity::Teacher,
                entity_id: Uuid::now_v7(),
            },
        )
        .expect_err("a clash is a clash wherever it happens")
        .to_string();
    assert!(message.contains("teacher-clash"), "{message}");
    assert!(message.contains("whole school"), "{message}");
}

#[test]
fn another_school_may_not_narrow_this_school_s_rule() {
    let mut project = Project::new();
    let north = project.school("Northgate Secondary");
    let south = project.school("Southmoor High");
    let created = project
        .db()
        .constraints()
        .create(&tuning(north, ConstraintType::TeacherClash))
        .expect("create");

    let error = project
        .db()
        .constraints()
        .add_scope(
            south,
            &crate::repo::constraints::ConstraintScopeEntryInput {
                constraint_id: created.id,
                role: ScopeRole::AppliesTo,
                entity_kind: ScopeEntity::Teacher,
                entity_id: Uuid::now_v7(),
            },
        )
        .expect_err("ownership is checked before anything else");
    assert!(matches!(error, DbError::WrongSchool { .. }), "{error:?}");
}

#[test]
fn a_scope_row_naming_a_row_that_is_gone_is_found_by_the_sweep() {
    // `entity_id` is polymorphic, so no foreign key covers it. This is the third of the three
    // things that stand in for one, and the only one M5 wires: nothing here can create a
    // scope row, so anything the sweep finds came from elsewhere.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::EntityUnavailable))
        .expect("create");
    let missing = Uuid::now_v7();
    write_scope_row(project.db(), created.id, ScopeEntity::Teacher, missing);

    let orphans = project.db().constraints().orphans().expect("sweep");
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].entity_id, missing);
    assert_eq!(orphans[0].entity_kind, ScopeEntity::Teacher);
}

// =======================================================================================
// The hand-over to the registry
// =======================================================================================

#[test]
fn the_stored_form_carries_what_the_registry_reads_and_interprets_nothing() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let autumn = project.term(school, "Autumn");

    let mut input = tuning(school, ConstraintType::DurationContiguous);
    input.params = "{\"allowBreakSpan\": true}".to_owned();
    input.weight = 7;
    input.term_id = Some(autumn);
    let created = project.db().constraints().create(&input).expect("create");

    let stored = project
        .db()
        .constraints()
        .stored_for(school)
        .expect("hand over");
    assert_eq!(stored.len(), 1);
    let row = &stored[0];
    assert_eq!(row.id, created.id);
    assert_eq!(row.type_key, "duration-contiguous");
    assert_eq!(row.level, "HARD", "the raw string, not an interpretation");
    assert_eq!(row.weight, 7);
    assert!(row.enabled);
    assert_eq!(row.params, "{\"allowBreakSpan\": true}");
    assert_eq!(row.term, Some(autumn));
    assert_eq!(row.applies_to, Vec::new());
}

#[test]
fn a_scope_row_reaches_the_registry_as_a_typed_reference() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::EntityUnavailable))
        .expect("create");
    let room = Uuid::now_v7();
    write_scope_row(project.db(), created.id, ScopeEntity::Room, room);

    let stored = project
        .db()
        .constraints()
        .stored_for(school)
        .expect("hand over");
    assert_eq!(
        stored[0].applies_to,
        vec![ScopeRef {
            kind: EntityKind::Room,
            id: room
        }],
        "one vocabulary for an entity reference across the whole scheduling path"
    );
}

#[test]
fn the_hand_over_is_ordered_so_the_registrys_first_row_wins_predictably() {
    // `Registry::compile` keeps the first row of a type. That is only deterministic because
    // this is ordered by `(type_key, id)` rather than by whatever the planner returned.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");
    for constraint_type in [
        ConstraintType::RoomClash,
        ConstraintType::ActivityScheduled,
        ConstraintType::TeacherClash,
    ] {
        project
            .db()
            .constraints()
            .create(&tuning(school, constraint_type))
            .expect("create");
    }

    let keys: Vec<String> = project
        .db()
        .constraints()
        .stored_for(school)
        .expect("hand over")
        .into_iter()
        .map(|row| row.type_key)
        .collect();
    assert_eq!(keys, ["activity-scheduled", "room-clash", "teacher-clash"]);
}

#[test]
fn an_unconfigured_school_hands_over_nothing() {
    // And is still fully checked, because the eight types are mandatory in the registry. The
    // assertion worth making here is that this is *empty* rather than a seeded eight: a seeded
    // row is a row somebody can delete.
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    assert_eq!(
        project.db().constraints().stored_for(school).expect("read"),
        Vec::new()
    );
    assert_eq!(
        project.db().constraints().list(school).expect("read"),
        Vec::new()
    );
}

// =======================================================================================
// The change feed
// =======================================================================================

#[test]
fn every_constraint_write_reaches_the_change_feed() {
    let mut project = Project::new();
    let school = project.school("Northgate Secondary");

    let created = project
        .db()
        .constraints()
        .create(&tuning(school, ConstraintType::TeacherClash))
        .expect("create");
    let mut changed = tuning(school, ConstraintType::TeacherClash);
    changed.weight = 12;
    project
        .db()
        .constraints()
        .update(created.id, Expected::Rev(1), &changed)
        .expect("retune");
    project
        .db()
        .constraints()
        .delete(school, created.id, Expected::Rev(2))
        .expect("remove");

    let ops: Vec<String> = project
        .db()
        .changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .into_iter()
        .filter(|change| change.entity_table == "constraint_instance")
        .map(|change| change.op)
        .collect();
    assert_eq!(ops, ["INSERT", "UPDATE", "DELETE"]);
}

// =======================================================================================
// Helpers that reach past the repository, deliberately
// =======================================================================================

/// Writes a `constraint_ref` row directly.
///
/// The repository refuses every narrowing in M5, so there is no public way to create one —
/// and the delete path, the sweep and the hand-over all have to work when a row exists,
/// because a project written by a newer build can hold one. This is that project, in a test.
fn write_scope_row(
    db: &mut Database,
    constraint_id: Uuid,
    entity_kind: ScopeEntity,
    entity_id: Uuid,
) -> Uuid {
    let id = Uuid::now_v7();
    let now = "2026-10-22T09:00:00.000Z";
    db.write(|tx| {
        tx.execute(
            "INSERT INTO constraint_ref (id, constraint_id, role, entity_kind, entity_id,
                 created_at, updated_at, rev)
             VALUES (?, ?, 'appliesTo', ?, ?, ?, ?, 1)",
            rusqlite::params![
                id.to_string(),
                constraint_id.to_string(),
                match entity_kind {
                    ScopeEntity::Teacher => "TEACHER",
                    ScopeEntity::Room => "ROOM",
                    _ => unreachable!("the tests only need two kinds"),
                },
                entity_id.to_string(),
                now,
                now,
            ],
        )?;
        Ok(())
    })
    .expect("write a scope row");
    id
}

/// How many change-feed entries one table has.
fn change_count(db: &mut Database, table: &str) -> usize {
    db.changes_since(0, CHANGE_FEED_LIMIT)
        .expect("changes")
        .into_iter()
        .filter(|change| change.entity_table == table)
        .count()
}
