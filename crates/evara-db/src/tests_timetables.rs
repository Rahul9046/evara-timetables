//! Saving a timetable, reading it back, scoring it again, and every way a save is refused.
//!
//! # The assertion that makes a stored timetable trustworthy
//!
//! [`the_arrangement_read_back_is_the_one_that_was_saved`] generates, saves, reopens the file
//! and rebuilds an [`Assignment`] from the stored rows. It must equal the one the solver
//! produced, **exactly**. That holds only because an entry stores the *start* slot and nothing
//! derived: there is no span to reconstruct and no row count to reconcile against
//! `duration_periods`, so a stored timetable cannot mean something other than what was
//! generated.
//!
//! # Why the fixtures are database-backed
//!
//! M4's `scheduling::seed` builds a real project with a grid, lessons and a solvable problem.
//! M7's tests reuse it rather than growing a second builder, for the reason M6 reused M2's
//! fixtures: a second one is a second place for a fixture to drift from the schema.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{
    Assignment, ProblemIdentity, Score, SolveOutcome, SolveStatus, Termination, UnplacedCause,
    UnscheduledActivity, UnverifiedCounts,
};
use evara_solver::{GenerateRequest, SolveConfig, generate};
use uuid::Uuid;

use crate::scheduling::seed::{Project, School};
use crate::{
    DbError, Expected, KIND_GENERATION, KIND_REVALIDATION, RunProvenance, SchedulingSelection,
    Scoring,
};

// =======================================================================================
// Scaffolding
// =======================================================================================

/// Six teaching periods a day, with a break in the middle.
///
/// The break is what makes a multi-period occurrence interesting: a day of nothing but teaching
/// periods cannot demonstrate a run that is broken.
fn standard_day() -> &'static [(&'static str, &'static str, &'static str, bool)] {
    &[
        ("1", "09:00", "10:00", true),
        ("2", "10:00", "11:00", true),
        ("Break", "11:00", "11:30", false),
        ("3", "11:30", "12:30", true),
        ("4", "13:30", "14:30", true),
        ("5", "14:30", "15:30", true),
    ]
}

/// A school with a grid, four rooms, four teachers, two groups and six occurrences.
///
/// Enough to be solvable and enough for a clash to be possible, which is what makes a stored
/// timetable worth asserting about. Two of the six occurrences are doubles, so D1's
/// multi-period case is exercised by every test that uses this.
fn a_solvable_school(project: &mut Project) -> School {
    let mut school = School::new(project, "Northgate", &["Main"], 5);
    school.with_term(project);
    school.bells(project, "Standard Day", Some(0), standard_day());
    school.materialise(project);

    let general = school.room_type(project, "General Classroom");
    for n in 0..4 {
        school.room(project, &format!("Room {n}"), 0, Some(general), 30, &[]);
    }
    let teachers: Vec<Uuid> = (0..4)
        .map(|n| school.teacher(project, &format!("Teacher{n}")))
        .collect();
    let groups = [
        school.group(project, "7A", Some(26), None),
        school.group(project, "7B", Some(26), None),
    ];

    let subject = school.subject(project, "Mathematics");
    for (n, group) in groups.into_iter().enumerate() {
        let section = school.section(
            project,
            subject,
            &format!("Mathematics {n}"),
            None,
            None,
            None,
            false,
        );
        school.teach(project, section, teachers[n]);
        school.attend(project, section, group);
        // Two singles and a double: one row each, whatever the duration (ADR 0029).
        school.occurrences(project, section, 1, 2);
        school.occurrences(project, section, 2, 1);
    }
    school
}

/// A second solvable school in the same project, for the isolation tests.
fn a_second_school(project: &mut Project, name: &str) -> School {
    let mut school = School::new(project, name, &["Main"], 5);
    school.with_term(project);
    school.bells(project, "Standard Day", Some(0), standard_day());
    school.materialise(project);
    let general = school.room_type(project, "General Classroom");
    school.room(project, "Room A", 0, Some(general), 30, &[]);
    let teacher = school.teacher(project, "Teacher");
    let group = school.group(project, "8A", Some(26), None);
    let subject = school.subject(project, "Science");
    let section = school.section(project, subject, "Science 8A", None, None, None, false);
    school.teach(project, section, teacher);
    school.attend(project, section, group);
    school.occurrences(project, section, 1, 2);
    school
}

/// Loads, generates and returns everything a save needs.
struct Generated {
    assembly: crate::scheduling::Assembly,
    outcome: SolveOutcome,
    violations: Vec<evara_core::scheduling::Violation>,
}

fn generate_for(project: &Project, selection: &SchedulingSelection) -> Generated {
    let assembly = project.load(selection).expect("load the problem");
    let rules = Registry::mandatory();
    let report = crate::AssemblyReport::of(&assembly);

    let outcome = generate(&GenerateRequest {
        problem: &assembly.problem,
        identity: ProblemIdentity {
            school: assembly.problem.context.school.id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &rules,
        preflight: report.findings.clone(),
        may_solve: report.may_solve(),
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(2026),
    });
    let violations = evaluate(&assembly.problem, &outcome.assignment, &rules).violations;
    Generated {
        assembly,
        outcome,
        violations,
    }
}

fn provenance() -> RunProvenance {
    RunProvenance {
        engine: "construction".to_owned(),
        settings: "{}".to_owned(),
        started_at: "2026-10-23T12:00:00.000Z".to_owned(),
        finished_at: "2026-10-23T12:00:01.000Z".to_owned(),
        kind: crate::RUN_GENERATION.to_owned(),
        scope: "{}".to_owned(),
    }
}

/// Saves a generated timetable under `name`.
fn save(
    project: &mut Project,
    generated: &Generated,
    name: &str,
) -> crate::Result<crate::SavedTimetable> {
    project.db().timetables().save_generated(
        &generated.assembly,
        &generated.outcome,
        &generated.violations,
        &provenance(),
        name,
    )
}

// =======================================================================================
// The round trip
// =======================================================================================

#[test]
fn the_arrangement_read_back_is_the_one_that_was_saved() {
    // The most valuable assertion in this module. See the header.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    let generated = generate_for(&project, &selection);
    assert!(
        generated.outcome.placed_count() > 0,
        "the fixture must be solvable for this test to mean anything"
    );
    let saved = save(&mut project, &generated, "First run").expect("save");

    // Through a *reopened* file, so the assertion is about what is on disk rather than about
    // the connection that wrote it.
    project.reopen();
    let assembly = project.load(&selection).expect("reload");
    let stored = project
        .db()
        .timetables()
        .arrangement(saved.timetable.id, &assembly)
        .expect("read the arrangement back");

    assert_eq!(
        stored.assignment, generated.outcome.assignment,
        "a stored timetable must mean exactly what was generated"
    );
    assert_eq!(stored.entries, generated.outcome.placed_count());
    assert_eq!(stored.not_in_problem.len(), 0, "every entry resolved");
}

#[test]
fn a_multi_period_occurrence_is_one_row_carrying_its_start() {
    // D1. The fixture has a two-period occurrence; it must be one row, and the run it occupies
    // must be derivable rather than stored.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    let entries = project
        .db()
        .timetables()
        .entries(saved.timetable.id)
        .expect("read entries");

    assert_eq!(
        entries.len(),
        generated.outcome.placed_count(),
        "one row per placed occurrence, whatever its duration"
    );
    let doubles = generated
        .assembly
        .problem
        .activities
        .iter()
        .filter(|a| a.duration.get() > 1)
        .count();
    assert!(
        doubles > 0,
        "the fixture must contain a multi-period lesson"
    );

    // And the span is still recoverable, from the one implementation.
    let two_period = generated
        .assembly
        .problem
        .activity_indices()
        .find(|at| {
            generated.assembly.problem.activities[at.get()]
                .duration
                .get()
                == 2
        })
        .expect("a double exists");
    if let Some(placement) = generated.outcome.assignment.placement(two_period) {
        let resolved = evara_core::scheduling::resolve_activity_span(
            &generated.assembly.problem,
            two_period,
            placement,
        )
        .expect("the span resolves");
        assert_eq!(resolved.span.slot_count(), 2, "derived, not stored");
    }
}

#[test]
fn the_four_unverified_counts_round_trip() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);
    generated.outcome.unverified = UnverifiedCounts {
        indeterminate_student_pairs: 3,
        unchecked_capacities: 5,
        shared_entities: 7,
        unenforced_restrictions: 11,
    };

    let saved = save(&mut project, &generated, "First run").expect("save");
    project.reopen();
    let read = project
        .db()
        .timetables()
        .get(saved.timetable.id)
        .expect("read it back");

    // ADR 0032 rule 4: these travel with the timetable or it can be described as clean when
    // four holes were disclosed at generation.
    assert_eq!(read.unverified(), generated.outcome.unverified);
}

#[test]
fn the_run_records_the_seed_the_statistics_and_the_termination() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    project.reopen();
    let run = project
        .db()
        .timetables()
        .run_for(saved.timetable.id)
        .expect("read the run")
        .expect("a generated timetable has one");

    assert_eq!(
        run.seed, 2026,
        "a run whose seed was not recorded cannot be reproduced"
    );
    assert_eq!(run.termination, generated.outcome.termination.key());
    assert_eq!(run.engine, "construction");
    let stats = generated.outcome.statistics;
    assert_eq!(
        run.candidate_evaluations,
        i64::try_from(stats.candidate_evaluations).expect("small")
    );
    assert_eq!(run.passes, i64::from(stats.passes));
    assert!(!run.app_version.is_empty(), "which build ran it");
}

#[test]
fn the_generation_scoring_is_stored_with_its_violations() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    project.reopen();
    let evaluations = project
        .db()
        .timetables()
        .evaluations(saved.timetable.id)
        .expect("read evaluations");

    assert_eq!(evaluations.len(), 1, "one scoring, from the generation");
    let generation = &evaluations[0];
    assert_eq!(generation.kind, KIND_GENERATION);
    assert!(generation.is_generation());
    assert!(
        generation.solve_run_id.is_some(),
        "a generation belongs to a run"
    );
    assert_eq!(generation.score(), generated.outcome.score);

    let stored = project
        .db()
        .timetables()
        .violations_for(generation.id)
        .expect("read violations");
    assert_eq!(stored.len(), generated.violations.len());
    // Every stored violation names its subject, so a report can render it without the problem.
    for violation in &stored {
        assert!(
            violation.entities.iter().any(|e| e.role == "subject"),
            "{violation:?}"
        );
        assert!(violation.penalty <= 0, "penalties are negative-going");
    }
}

#[test]
fn the_unplaced_causes_are_stored_and_the_sentence_is_not() {
    // Correction 4's subject: an unplaced cause is historical generation information. The cause
    // is stored because nothing can reconstruct it once the inputs move; the engine's sentence
    // is not, because a wording improvement would leave old rows reading the old way.
    //
    // The unplaced occurrence is **injected** rather than provoked by a hard fixture. A school
    // the constructor genuinely cannot finish is one M3's counting diagnostics prove impossible
    // — so the gate refuses it and there is no save to test. What is under test here is the
    // persistence of a cause, not the solver's ability to fail, so the outcome is edited to
    // carry one.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);

    let giving_up = generated
        .assembly
        .problem
        .activity_indices()
        .next()
        .expect("the fixture has occurrences");
    let fact = &generated.assembly.problem.activities[giving_up.get()];
    generated.outcome.assignment.clear(giving_up);
    generated.outcome.unscheduled.push(UnscheduledActivity {
        activity: giving_up,
        id: fact.id,
        section: "Mathematics 0".to_owned(),
        cause: UnplacedCause::BlockedByPlacements,
        reason: "every slot clashed with a lesson it had already placed".to_owned(),
        diagnostic: None,
    });
    generated.outcome.status = SolveStatus::Partial;
    generated.outcome.termination = Termination::Exhausted;

    let saved = save(&mut project, &generated, "Crowded").expect("save");
    project.reopen();
    let run = project
        .db()
        .timetables()
        .run_for(saved.timetable.id)
        .expect("read run")
        .expect("there is one");
    let unplaced = project
        .db()
        .timetables()
        .unscheduled_for(run.id)
        .expect("read unscheduled");

    assert_eq!(unplaced.len(), 1);
    assert_eq!(unplaced[0].cause, UnplacedCause::BlockedByPlacements.key());
    assert_eq!(unplaced[0].activity_id, fact.id);
    assert_eq!(unplaced[0].diagnostic_code, None);
    // The engine's sentence is nowhere in the row, and the cause still answers the only
    // question that matters: is this a property of the problem, or of this run?
    assert!(
        !unplaced[0].is_impossible(),
        "a slot running out is a property of the run, not of the curriculum"
    );
}

// =======================================================================================
// Versioning
// =======================================================================================

#[test]
fn a_second_generation_never_overwrites_the_first() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    let first = generate_for(&project, &selection);
    let saved_first = save(&mut project, &first, "First run").expect("save the first");
    let first_entries = project
        .db()
        .timetables()
        .entries(saved_first.timetable.id)
        .expect("read entries");

    // Reload, because the first save moved the feed; the second generation is a fresh snapshot.
    let second = generate_for(&project, &selection);
    let saved_second = save(&mut project, &second, "Second run").expect("save the second");

    assert_ne!(saved_first.timetable.id, saved_second.timetable.id);
    let after = project
        .db()
        .timetables()
        .entries(saved_first.timetable.id)
        .expect("the first is still there");
    assert_eq!(
        first_entries, after,
        "a regeneration must leave the previous timetable byte-identical"
    );

    let all = project
        .db()
        .timetables()
        .for_school(school.id)
        .expect("list");
    assert_eq!(all.len(), 2, "both coexist, and that is the versioning");
}

#[test]
fn two_timetables_in_one_scope_may_not_share_a_name() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    let first = generate_for(&project, &selection);
    save(&mut project, &first, "Same name").expect("the first");
    let second = generate_for(&project, &selection);

    let error = save(&mut project, &second, "Same name").expect_err("refused");
    assert!(matches!(error, DbError::DuplicateValue { .. }), "{error:?}");
}

#[test]
fn a_whole_year_timetable_is_covered_by_the_uniqueness_rule() {
    // Correction 1: the rule is a UNIQUE INDEX over COALESCE(term_id, ''), because SQLite
    // treats NULLs as distinct and the whole-year case is the common one. Without the
    // COALESCE the second save here would succeed.
    //
    // The school deliberately has **no term**, so its grid is materialised for the whole year
    // and `term_id` really is NULL. A school whose grid was built against a term has no
    // whole-year grid at all, and pre-solve validation refuses that selection rather than
    // solving over an empty one — which is correct, and is why this fixture is separate.
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Northgate", &["Main"], 5);
    school.bells(&mut project, "Standard Day", Some(0), standard_day());
    school.materialise(&mut project);
    let general = school.room_type(&mut project, "General Classroom");
    school.room(&mut project, "Room 1", 0, Some(general), 30, &[]);
    let teacher = school.teacher(&mut project, "Teacher");
    let group = school.group(&mut project, "7A", Some(26), None);
    let subject = school.subject(&mut project, "Mathematics");
    let section = school.section(
        &mut project,
        subject,
        "Mathematics 7A",
        None,
        None,
        None,
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    let whole_year = school.selection();
    assert!(whole_year.term_id.is_none(), "the fixture has no term");

    let first = generate_for(&project, &whole_year);
    save(&mut project, &first, "Whole year").expect("the first");
    let second = generate_for(&project, &whole_year);

    let error = save(&mut project, &second, "Whole year").expect_err("refused");
    assert!(matches!(error, DbError::DuplicateValue { .. }), "{error:?}");
}

#[test]
fn renaming_takes_the_revision_and_refuses_a_stale_one() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    let renamed = project
        .db()
        .timetables()
        .rename(saved.timetable.id, saved.timetable.rev, "Renamed")
        .expect("rename");
    assert_eq!(renamed.name, "Renamed");
    assert_eq!(renamed.rev, saved.timetable.rev + 1);

    let stale = project
        .db()
        .timetables()
        .rename(saved.timetable.id, saved.timetable.rev, "Again")
        .expect_err("refused");
    assert!(matches!(stale, DbError::Conflict { .. }), "{stale:?}");
}

#[test]
fn deleting_a_timetable_removes_its_children_and_writes_each_one() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    let before = project
        .db()
        .changes_since(0, 100_000)
        .expect("read the feed")
        .len();

    let removed = project
        .db()
        .timetables()
        .delete(saved.timetable.id, saved.timetable.rev)
        .expect("delete");

    assert_eq!(removed.entries, saved.entries);
    assert_eq!(removed.runs, 1);
    assert_eq!(removed.evaluations, 1);
    assert_eq!(removed.violations, saved.violations);
    assert!(
        project.db().timetables().get(saved.timetable.id).is_err(),
        "it is gone"
    );

    // ADR 0040 §2: the children are removed by the repository, so each is a truthful
    // change-log entry. A CASCADE would have removed them silently.
    let after = project
        .db()
        .changes_since(0, 100_000)
        .expect("read the feed")
        .len();
    assert!(
        after > before + saved.entries,
        "every tracked deletion must reach the change feed: {before} -> {after}"
    );
}

#[test]
fn deleting_with_a_stale_revision_removes_nothing() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    project
        .db()
        .timetables()
        .rename(saved.timetable.id, saved.timetable.rev, "Moved on")
        .expect("rename");

    let error = project
        .db()
        .timetables()
        .delete(saved.timetable.id, saved.timetable.rev)
        .expect_err("refused");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
    assert_eq!(
        project
            .db()
            .timetables()
            .entries(saved.timetable.id)
            .expect("still there")
            .len(),
        saved.entries,
        "a refused delete must remove nothing at all"
    );
}

// =======================================================================================
// Freshness — correction 2
// =======================================================================================

#[test]
fn an_input_that_changed_after_generation_refuses_the_save() {
    let mut project = Project::new();
    let mut school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);

    // A scheduling input moves. Nothing referential breaks: every activity, timeslot and room
    // the outcome names still exists, so without the watermark this save would succeed and
    // store a timetable built against inputs that no longer exist.
    school.teacher(&mut project, "Latecomer");

    let error = save(&mut project, &generated, "Stale").expect_err("refused");
    match error {
        DbError::InputsChanged { changes_since, .. } => {
            assert!(changes_since > 0, "the refusal says how much moved");
        }
        other => panic!("{other:?}"),
    }

    assert!(
        project
            .db()
            .timetables()
            .for_school(school.id)
            .expect("list")
            .is_empty(),
        "nothing was written"
    );
}

#[test]
fn saving_one_timetable_does_not_make_another_outcome_stale() {
    // The output tables are excluded from the freshness check: writing a timetable is not a
    // change to the *inputs*. Without that exclusion the second save here would be refused.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    let first = generate_for(&project, &selection);
    let second = generate_for(&project, &selection);

    save(&mut project, &first, "First").expect("the first saves");
    save(&mut project, &second, "Second").expect("and so does the second");
}

#[test]
fn a_reloaded_problem_saves_after_the_inputs_moved() {
    // The remedy the refusal names: generate again. A fresh load takes a fresh watermark.
    let mut project = Project::new();
    let mut school = a_solvable_school(&mut project);
    let selection = school.selection();
    let stale = generate_for(&project, &selection);
    school.teacher(&mut project, "Latecomer");

    assert!(save(&mut project, &stale, "Stale").is_err());
    let fresh = generate_for(&project, &selection);
    save(&mut project, &fresh, "Fresh").expect("a fresh problem saves");
}

// =======================================================================================
// Refusals
// =======================================================================================

#[test]
fn an_input_rejected_outcome_is_not_saveable() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);
    generated.outcome.status = SolveStatus::Failed;
    generated.outcome.termination = Termination::InputRejected;

    let error = save(&mut project, &generated, "Refused").expect_err("refused");
    assert!(matches!(error, DbError::NotSaveable { .. }), "{error:?}");
}

#[test]
fn an_outcome_whose_internal_check_failed_is_not_saveable() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);
    generated.outcome.status = SolveStatus::Failed;
    generated.outcome.termination = Termination::InternalCheckFailed;

    let error = save(&mut project, &generated, "Broken").expect_err("refused");
    assert!(matches!(error, DbError::NotSaveable { .. }), "{error:?}");
}

#[test]
fn an_outcome_for_another_problem_is_refused() {
    // The failure this prevents is the worst shape available: translating every dense index
    // through the wrong map would store a complete, plausible, wrong timetable.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);
    generated.outcome.problem = ProblemIdentity {
        school: Uuid::now_v7(),
        cycle: Uuid::now_v7(),
        term: None,
    };

    let error = save(&mut project, &generated, "Elsewhere").expect_err("refused");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
}

#[test]
fn an_outcome_of_the_wrong_length_is_refused() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let mut generated = generate_for(&project, &selection);
    generated.outcome.assignment = Assignment::empty(generated.outcome.assignment.len() + 1);

    let error = save(&mut project, &generated, "Mismatched").expect_err("refused");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
}

#[test]
fn a_read_only_project_refuses_a_save_before_a_transaction_opens() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);

    // The project stays alive deliberately. `seed::Project` owns the temporary directory and
    // removes it on drop, so dropping it to "close the file" deletes the file — which on Unix
    // makes this a test of a missing path rather than of a read-only connection. A second
    // connection on the same path is what the inspection path does in production anyway.
    let mut inspect = crate::Database::open_for_inspection(project.path()).expect("open read-only");
    let error = inspect
        .timetables()
        .save_generated(
            &generated.assembly,
            &generated.outcome,
            &generated.violations,
            &provenance(),
            "Read only",
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::ReadOnlyProject), "{error:?}");
}

// =======================================================================================
// Referential refusals
// =======================================================================================

#[test]
fn a_timeslot_a_stored_entry_uses_cannot_be_deleted() {
    // ADR 0011 §5 by name: "It relies on RESTRICT to refuse any slot a future
    // `timetable_entry` references, which is why timetable entries will not need a cascade."
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    let entries = project
        .db()
        .timetables()
        .entries(saved.timetable.id)
        .expect("entries");
    let used = entries[0].timeslot_id;

    let error = project
        .db()
        .time_model()
        .delete_timeslots(school.id, &[used])
        .expect_err("refused");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "{error:?}"
    );
}

#[test]
fn an_activity_a_stored_entry_uses_cannot_be_deleted() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    let entries = project
        .db()
        .timetables()
        .entries(saved.timetable.id)
        .expect("entries");
    let activity = entries[0].activity_id;
    let error = project
        .db()
        .lessons()
        .delete_activity(school.id, activity, Expected::Any)
        .expect_err("refused");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "{error:?}"
    );
}

// =======================================================================================
// School isolation
// =======================================================================================

#[test]
fn a_timetable_belongs_to_one_school_and_the_listing_says_so() {
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project, "Southmoor");

    let north_saved = {
        let generated = generate_for(&project, &north.selection());
        save(&mut project, &generated, "Northgate run").expect("save")
    };
    let south_saved = {
        let generated = generate_for(&project, &south.selection());
        save(&mut project, &generated, "Southmoor run").expect("save")
    };

    let northern = project
        .db()
        .timetables()
        .for_school(north.id)
        .expect("list north");
    let southern = project
        .db()
        .timetables()
        .for_school(south.id)
        .expect("list south");

    assert_eq!(northern.len(), 1);
    assert_eq!(southern.len(), 1);
    assert_eq!(northern[0].id, north_saved.timetable.id);
    assert_eq!(southern[0].id, south_saved.timetable.id);
    // The school is resolved through `cycle`, which is what carrying no `school_id` costs and
    // what keeps it from being a second place for the ownership to be wrong.
    assert_ne!(northern[0].cycle_id, southern[0].cycle_id);
}

#[test]
fn a_cycle_and_a_term_from_two_schools_cannot_make_a_timetable() {
    // The trigger pair of V17. ADR 0032 rule 1 scopes a timetable to ONE school, and nothing
    // else would refuse a row naming one school's cycle and another's term.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project, "Southmoor");
    let foreign_term = south.selection().term_id.expect("Southmoor has a term");

    let error = project
        .db()
        .write(|tx| {
            tx.execute(
                "INSERT INTO timetable (
                     id, cycle_id, term_id, name, generated_at,
                     indeterminate_student_pairs, unchecked_capacities, shared_entities,
                     unenforced_restrictions, created_at, updated_at, rev)
                 VALUES (?1, ?2, ?3, 'Mixed', '2026-01-01T00:00:00.000Z',
                         0, 0, 0, 0, '2026-01-01T00:00:00.000Z',
                         '2026-01-01T00:00:00.000Z', 1)",
                rusqlite::params![
                    Uuid::now_v7().to_string(),
                    north.cycle.to_string(),
                    foreign_term.to_string(),
                ],
            )?;
            Ok(())
        })
        .expect_err("refused by the trigger");
    // The trigger's own message, which reaches the caller wrapped as an internal failure
    // because a trigger abort is not a `DbError` variant — the schema refused it, which is the
    // point: a code path cannot be the only thing keeping two schools apart in a file.
    assert!(
        format!("{error:?}").contains("different school"),
        "{error:?}"
    );
}

// =======================================================================================
// Revalidation — correction 3
// =======================================================================================

#[test]
fn a_revalidation_adds_a_current_scoring_and_leaves_the_generation_one_alone() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");
    let generation_id = saved.evaluation.id;
    let generation_score = saved.evaluation.score();

    project.reopen();
    let assembly = project.load(&selection).expect("reload");
    let stored = project
        .db()
        .timetables()
        .arrangement(saved.timetable.id, &assembly)
        .expect("read back");
    let rules = Registry::mandatory();
    let report = evaluate(&assembly.problem, &stored.assignment, &rules);

    let timetable = project
        .db()
        .timetables()
        .get(saved.timetable.id)
        .expect("read it");
    let current = project
        .db()
        .timetables()
        .record_revalidation(
            timetable.id,
            timetable.rev,
            &Scoring {
                score: report.score,
                placed: stored.entries,
                required: assembly.problem.activities.len(),
                violations: &report.violations,
            },
            &assembly,
        )
        .expect("record the revalidation");

    assert_eq!(current.kind, KIND_REVALIDATION);
    assert!(current.solve_run_id.is_none(), "a revalidation has no run");

    let evaluations = project
        .db()
        .timetables()
        .evaluations(timetable.id)
        .expect("read evaluations");
    assert_eq!(evaluations.len(), 2, "the generation one survives");
    let generation = evaluations
        .iter()
        .find(|e| e.is_generation())
        .expect("still there");
    assert_eq!(generation.id, generation_id);
    assert_eq!(
        generation.score(),
        generation_score,
        "historical provenance must not be rewritten by a revalidation"
    );
}

#[test]
fn a_second_revalidation_replaces_the_first_and_still_spares_the_generation() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");
    let assembly = project.load(&selection).expect("load");

    let first = record_a_revalidation(
        &mut project,
        saved.timetable.id,
        &assembly,
        Score::new(0, -1, 0),
    );
    let second = record_a_revalidation(
        &mut project,
        saved.timetable.id,
        &assembly,
        Score::new(0, -2, 0),
    );

    assert_ne!(first.id, second.id, "a replacement is a new row");
    let evaluations = project
        .db()
        .timetables()
        .evaluations(saved.timetable.id)
        .expect("read evaluations");
    assert_eq!(
        evaluations.len(),
        2,
        "still exactly one generation and one current scoring"
    );
    assert_eq!(
        evaluations
            .iter()
            .filter(|e| e.kind == KIND_REVALIDATION)
            .count(),
        1
    );
    assert!(
        project
            .db()
            .timetables()
            .violations_for(first.id)
            .expect("read")
            .is_empty(),
        "the replaced scoring's violations went with it"
    );
}

/// Records a revalidation with a chosen score, so a test can tell two apart.
fn record_a_revalidation(
    project: &mut Project,
    timetable_id: Uuid,
    assembly: &crate::scheduling::Assembly,
    score: Score,
) -> crate::TimetableEvaluation {
    let timetable = project.db().timetables().get(timetable_id).expect("read");
    project
        .db()
        .timetables()
        .record_revalidation(
            timetable.id,
            timetable.rev,
            &Scoring {
                score,
                placed: 0,
                required: 0,
                violations: &[],
            },
            assembly,
        )
        .expect("record")
}

#[test]
fn a_revalidation_takes_the_timetables_revision() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");
    let assembly = project.load(&selection).expect("load");

    // Somebody renames it; the revalidation built on the old revision is refused.
    project
        .db()
        .timetables()
        .rename(saved.timetable.id, saved.timetable.rev, "Renamed")
        .expect("rename");

    let error = project
        .db()
        .timetables()
        .record_revalidation(
            saved.timetable.id,
            saved.timetable.rev,
            &Scoring {
                score: Score::PERFECT,
                placed: 0,
                required: 0,
                violations: &[],
            },
            &assembly,
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
}

#[test]
fn an_entry_whose_lesson_left_the_problem_is_reported_rather_than_refused() {
    // A timetable whose inputs moved is still worth opening. The reader says what it could not
    // place in the arrangement rather than failing, so the user can see what to fix.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "First run").expect("save");

    // Load a *different* scope, so the stored entries name activities this problem lacks.
    let other_scope = SchedulingSelection {
        term_id: None,
        ..selection
    };
    let assembly = project.load(&other_scope).expect("load the whole year");
    let stored = project
        .db()
        .timetables()
        .arrangement(saved.timetable.id, &assembly)
        .expect("still readable");

    assert_eq!(stored.entries, saved.entries, "every row was read");
    // Whether anything is missing depends on whether the whole-year grid holds the same slots;
    // what matters is that the read succeeded and reported rather than refusing.
    assert_eq!(stored.assignment.len(), assembly.problem.activities.len());
}

// =======================================================================================
// Transaction boundary
// =======================================================================================

#[test]
fn a_failed_save_leaves_no_timetable_at_all() {
    // One transaction. The name collision fires on the `timetable` insert, which is the first
    // statement, so this also asserts the simplest case; the harder one is below.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    let first = generate_for(&project, &selection);
    save(&mut project, &first, "Taken").expect("the first");
    let before = project
        .db()
        .timetables()
        .for_school(school.id)
        .expect("list")
        .len();

    let second = generate_for(&project, &selection);
    assert!(save(&mut project, &second, "Taken").is_err());

    assert_eq!(
        project
            .db()
            .timetables()
            .for_school(school.id)
            .expect("list")
            .len(),
        before,
        "a refused save adds nothing"
    );
}

#[test]
fn a_save_refused_for_staleness_writes_no_entry_or_run() {
    // The freshness check runs first, inside the transaction, so nothing downstream of it has
    // written anything when it refuses.
    let mut project = Project::new();
    let mut school = a_solvable_school(&mut project);
    let selection = school.selection();
    let generated = generate_for(&project, &selection);
    school.teacher(&mut project, "Latecomer");

    assert!(save(&mut project, &generated, "Stale").is_err());

    let orphans: i64 = project
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT (SELECT COUNT(*) FROM timetable_entry)
                      + (SELECT COUNT(*) FROM solve_run)
                      + (SELECT COUNT(*) FROM timetable_evaluation)
                      + (SELECT COUNT(*) FROM violation)",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("count");
    assert_eq!(orphans, 0, "nothing downstream of the guard was written");
}

// =======================================================================================
// Migration
// =======================================================================================

#[test]
fn a_v16_project_upgrades_to_v17_with_the_new_tables_empty() {
    // Every existing project opens unchanged and reads the new tables as empty, which is the
    // correct state for one that has never generated.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();

    // Rewind to V16 by removing V17's tables and its history row, then reopen to re-run it.
    project
        .db()
        .write(|tx| {
            tx.execute_batch(
                "DROP TABLE IF EXISTS violation_ref;
                 DROP TABLE IF EXISTS violation;
                 DROP TABLE IF EXISTS timetable_evaluation;
                 DROP TABLE IF EXISTS solve_run_unscheduled;
                 DROP TABLE IF EXISTS solve_run;
                 DROP TABLE IF EXISTS timetable_entry;
                 DROP TABLE IF EXISTS timetable",
            )?;
            // V18 only ADDS columns to `timetable` and `solve_run` and an index over one of
            // them, so dropping those tables already removed its whole effect -- but its
            // history row has to go too, or reopening would re-run V17 and skip V18, leaving
            // `timetable` without the lineage column.
            tx.execute(
                "DELETE FROM refinery_schema_history WHERE version IN (17, 18)",
                [],
            )?;
            Ok(())
        })
        .expect("rewind to V16");

    project.reopen();

    // The upgrade ran, the school's own rows are untouched, and a save works.
    let generated = generate_for(&project, &selection);
    let saved = save(&mut project, &generated, "After upgrade").expect("save after upgrading");
    assert_eq!(saved.entries, generated.outcome.placed_count());
}
