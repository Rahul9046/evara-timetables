//! What a repair writes, what it refuses to write, and what it never touches.
//!
//! # The guarantees these tests exist for
//!
//! - **The scope check, repeated in the writing transaction.** `evara-solver` already compared
//!   the complete arrangements; [`a_repair_that_moved_a_frozen_placement_is_refused_by_the_write`]
//!   proves the database refuses one anyway, because an outcome that passed in memory and is then
//!   persisted from a different arrangement would write a timetable nobody checked.
//! - **One snapshot.** [`the_baseline_snapshot_is_internally_consistent`] reads everything from
//!   one transaction, so the stamp cannot be guarding an arrangement the repair did not use.
//! - **Immutable provenance.** [`a_repair_leaves_the_sources_run_and_scoring_byte_identical`].
//! - **Readable lineage refusal.** [`deleting_a_source_with_descendants_is_refused_readably`] —
//!   `DbError::HasDescendants`, never the foreign key's opaque failure.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{ActivityIx, Assignment, MovableSet, ProblemIdentity, Violation};
use evara_solver::{RepairRequest, SolveConfig, repair};
use uuid::Uuid;

use crate::scheduling::seed::{Project, School};
use crate::{
    ArrangementStamp, DbError, EditGuard, EntryFlags, RUN_REPAIR, RepairSave, RunProvenance,
    SavedTimetable, SchedulingSelection, UnexpressibleLockCause,
};

// =======================================================================================
// Scaffolding
// =======================================================================================

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

/// The same shape `tests_editing` uses, deliberately: a second builder would be a second place
/// for a fixture to drift from the schema.
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
        school.occurrences(project, section, 1, 2);
        school.occurrences(project, section, 2, 1);
    }
    school
}

fn a_second_school(project: &mut Project) -> School {
    let mut school = School::new(project, "Southmoor", &["Main"], 5);
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

fn generation_provenance() -> RunProvenance {
    RunProvenance {
        engine: "construction".to_owned(),
        settings: "{}".to_owned(),
        started_at: "2026-10-25T12:00:00.000Z".to_owned(),
        finished_at: "2026-10-25T12:00:01.000Z".to_owned(),
        kind: crate::RUN_GENERATION.to_owned(),
        scope: "{}".to_owned(),
    }
}

fn repair_provenance(scope: &str) -> RunProvenance {
    RunProvenance {
        engine: "construction".to_owned(),
        settings: "{}".to_owned(),
        started_at: "2026-10-25T13:00:00.000Z".to_owned(),
        finished_at: "2026-10-25T13:00:02.000Z".to_owned(),
        kind: RUN_REPAIR.to_owned(),
        scope: scope.to_owned(),
    }
}

/// Generates and saves one timetable for `selection`.
fn saved(project: &mut Project, selection: &SchedulingSelection, name: &str) -> SavedTimetable {
    let assembly = project.load(selection).expect("load the problem");
    let rules = Registry::mandatory();
    let report = crate::AssemblyReport::of(&assembly);
    let outcome = evara_solver::generate(&evara_solver::GenerateRequest {
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
    let violations: Vec<Violation> =
        evaluate(&assembly.problem, &outcome.assignment, &rules).violations;
    project
        .db()
        .timetables()
        .save_generated(
            &assembly,
            &outcome,
            &violations,
            &generation_provenance(),
            name,
        )
        .expect("save the generated timetable")
}

/// Locks or unlocks one entry, supplying the guard from the current state.
///
/// A test helper rather than a production shortcut: reading the stamp immediately before the
/// write is exactly what a *session* must not do, and is fine here because nothing else is
/// running.
fn set_lock_now(
    project: &mut Project,
    selection: &SchedulingSelection,
    timetable_id: Uuid,
    entry_id: Uuid,
    expected: i64,
    locked: bool,
) {
    let assembly = project.load(selection).expect("load");
    let watermark = assembly.watermark;
    let stamp = project
        .db()
        .timetables()
        .arrangement_stamp(timetable_id)
        .expect("stamp");
    project
        .db()
        .timetables()
        .set_lock(
            entry_id,
            expected,
            locked,
            EditGuard {
                watermark,
                stamp: &stamp,
            },
        )
        .expect("set the lock");
}

/// Everything a repair-and-save cycle needs, computed once.
struct Repaired {
    source: Uuid,
    source_rev: i64,
    /// Where the inputs stood when the repair compiled its problem.
    ///
    /// Captured here rather than re-read at save time: a fresh watermark would make
    /// `guard_freshness` compare the present against the present and pass vacuously, which is
    /// the check-then-act the guard exists to prevent.
    watermark: crate::change_log::Watermark,
    stamp: ArrangementStamp,
    baseline: Assignment,
    movable: MovableSet,
    flags: Vec<(Uuid, EntryFlags)>,
    outcome: evara_core::scheduling::SolveOutcome,
    violations: Vec<Violation>,
}

/// Repairs `source` over the whole timetable and returns everything the save needs.
fn repair_whole(project: &mut Project, selection: &SchedulingSelection, source: Uuid) -> Repaired {
    let assembly = project.load(selection).expect("load");
    let rules = Registry::mandatory();
    let base = project
        .db()
        .timetables()
        .repair_baseline(source, &assembly)
        .expect("baseline");

    let movable = MovableSet::everything(assembly.problem.activities.len()).expect("movable");
    let report = crate::AssemblyReport::of(&assembly);
    let out = repair(&RepairRequest {
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
        config: SolveConfig::seeded(99),
        baseline: &base.arrangement.assignment,
        movable: &movable,
    });

    Repaired {
        source,
        source_rev: base.source_rev,
        watermark: assembly.watermark,
        stamp: base.stamp,
        baseline: base.arrangement.assignment,
        movable,
        flags: base.flags,
        violations: out.violations,
        outcome: out.outcome,
    }
}

/// Saves a repair, with every guard supplied from the snapshot it was computed against.
fn save_repair(
    project: &mut Project,
    selection: &SchedulingSelection,
    r: &Repaired,
    name: &str,
) -> crate::error::Result<SavedTimetable> {
    let assembly = project.load(selection).expect("load");
    project.db().timetables().save_repaired(
        &assembly,
        &RepairSave {
            source: r.source,
            expected: r.source_rev,
            guard: EditGuard {
                watermark: r.watermark,
                stamp: &r.stamp,
            },
            outcome: &r.outcome,
            violations: &r.violations,
            baseline: &r.baseline,
            movable: &r.movable,
            flags: &r.flags,
            run: &repair_provenance(r#"{"kind":"whole"}"#),
            name,
        },
    )
}

// =======================================================================================
// The snapshot
// =======================================================================================

#[test]
fn the_baseline_snapshot_is_internally_consistent() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let assembly = project.load(&selection).expect("load");
    let base = project
        .db()
        .timetables()
        .repair_baseline(written.timetable.id, &assembly)
        .expect("baseline");

    // One read, so all five views agree.
    assert_eq!(base.arrangement.entries, written.entries);
    assert_eq!(base.stamp.len(), written.entries);
    assert_eq!(base.flags.len(), written.entries);
    assert_eq!(base.source_rev, written.timetable.rev);
    assert!(
        base.pins.unexpressible.is_empty(),
        "a freshly generated timetable has no locks at all"
    );
    assert_eq!(base.pins.locked, 0);

    // And it agrees with the separate readers it replaces.
    let separately = project
        .db()
        .timetables()
        .arrangement(written.timetable.id, &assembly)
        .expect("arrangement");
    assert_eq!(base.arrangement.assignment, separately.assignment);
    let stamp = project
        .db()
        .timetables()
        .arrangement_stamp(written.timetable.id)
        .expect("stamp");
    assert_eq!(base.stamp, stamp);
}

// =======================================================================================
// The write
// =======================================================================================

#[test]
fn a_repair_writes_a_new_version_and_leaves_the_source_alone() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let r = repair_whole(&mut project, &selection, written.timetable.id);
    let repaired = save_repair(&mut project, &selection, &r, "Repaired").expect("save");

    assert_ne!(repaired.timetable.id, written.timetable.id);
    assert_eq!(
        repaired.timetable.based_on_timetable_id,
        Some(written.timetable.id),
        "the lineage edge must point at the source"
    );
    assert_eq!(repaired.run.kind, RUN_REPAIR);
    assert!(
        repaired.run.scope.contains("whole"),
        "the scope is recorded: {}",
        repaired.run.scope
    );
    // The evaluation is a GENERATION: this IS the new version's original scoring.
    assert!(repaired.evaluation.is_generation());

    // Both versions coexist, and the source is byte-identical.
    let all = project
        .db()
        .timetables()
        .for_cycle(selection.cycle_id, selection.term_id)
        .expect("read");
    assert_eq!(all.len(), 2);
    let source_now = all
        .iter()
        .find(|t| t.id == written.timetable.id)
        .expect("the source survives");
    assert_eq!(
        *source_now, written.timetable,
        "a repair must not touch the source row"
    );
}

#[test]
fn a_repair_leaves_the_sources_run_and_scoring_byte_identical() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let before_run = project
        .db()
        .timetables()
        .run_for(written.timetable.id)
        .expect("run")
        .expect("there is one");
    let before_evaluations = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("evaluations");

    let r = repair_whole(&mut project, &selection, written.timetable.id);
    save_repair(&mut project, &selection, &r, "Repaired").expect("save");

    let after_run = project
        .db()
        .timetables()
        .run_for(written.timetable.id)
        .expect("run")
        .expect("still there");
    let after_evaluations = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("evaluations");

    assert_eq!(before_run, after_run, "historical provenance is immutable");
    assert_eq!(before_evaluations, after_evaluations);
}

#[test]
fn a_repair_preserves_lock_and_manual_flags_it_did_not_move() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    // Lock one entry, which also makes it frozen.
    let entries = project
        .db()
        .timetables()
        .entries(written.timetable.id)
        .expect("entries");
    let locked = entries.first().expect("an entry").clone();
    set_lock_now(
        &mut project,
        &selection,
        written.timetable.id,
        locked.id,
        locked.rev,
        true,
    );

    let source_rev = project
        .db()
        .timetables()
        .for_cycle(selection.cycle_id, selection.term_id)
        .expect("read")
        .into_iter()
        .find(|t| t.id == written.timetable.id)
        .expect("source")
        .rev;

    let assembly = project.load(&selection).expect("load");
    let base = project
        .db()
        .timetables()
        .repair_baseline(written.timetable.id, &assembly)
        .expect("baseline");
    assert_eq!(base.pins.locked, 1, "the lock was read as a pin");

    // Repair everything EXCEPT the locked occurrence, so the lock is frozen by scope too.
    let locked_ix = assembly
        .index
        .activity_index(locked.activity_id)
        .expect("the locked activity is in the problem");
    let movable: Vec<ActivityIx> = (0..assembly.problem.activities.len())
        .map(|n| ActivityIx::new(u32::try_from(n).expect("fits")))
        .filter(|at| *at != locked_ix)
        .collect();
    let movable = MovableSet::new(assembly.problem.activities.len(), &movable).expect("movable");

    let report = crate::AssemblyReport::of(&assembly);
    let mut problem = assembly.problem.clone();
    problem.pinned = base.pins.pinned.clone();
    let out = repair(&RepairRequest {
        problem: &problem,
        identity: ProblemIdentity {
            school: problem.context.school.id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &Registry::mandatory(),
        preflight: report.findings.clone(),
        may_solve: report.may_solve(),
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(5),
        baseline: &base.arrangement.assignment,
        movable: &movable,
    });
    assert_eq!(out.defect, None, "{:?}", out.defect);

    let r = Repaired {
        source: written.timetable.id,
        source_rev,
        watermark: assembly.watermark,
        stamp: base.stamp,
        baseline: base.arrangement.assignment,
        movable,
        flags: base.flags,
        violations: out.violations,
        outcome: out.outcome,
    };
    let repaired = save_repair(&mut project, &selection, &r, "Repaired").expect("save");

    let new_entries = project
        .db()
        .timetables()
        .entries(repaired.timetable.id)
        .expect("entries");
    let carried = new_entries
        .iter()
        .find(|e| e.activity_id == locked.activity_id)
        .expect("the locked occurrence is in the new version");
    assert!(carried.is_locked, "the lock must survive the repair");
    assert_eq!(
        carried.timeslot_id, locked.timeslot_id,
        "and so must its slot"
    );
    assert_eq!(carried.room_id, locked.room_id, "and its room");
}

#[test]
fn a_repair_that_moved_a_frozen_placement_is_refused_by_the_write() {
    // The mandatory safeguard, at the persistence layer. The outcome is doctored after the
    // solver verified it, which is exactly the case this second check exists for.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let mut r = repair_whole(&mut project, &selection, written.timetable.id);

    // Freeze everything, then move one of the frozen placements by hand.
    let assembly = project.load(&selection).expect("load");
    let total = assembly.problem.activities.len();
    r.movable = MovableSet::new(total, &[ActivityIx::new(0)]).expect("movable");
    let victim = ActivityIx::new(1);
    let somewhere_else = r
        .baseline
        .placement(ActivityIx::new(2))
        .expect("a second placement to steal");
    r.outcome.assignment.place(victim, somewhere_else);

    let error = save_repair(&mut project, &selection, &r, "Doctored").expect_err("refused");
    assert!(
        matches!(error, DbError::ScopeViolated { .. }),
        "expected a scope violation, got {error:?}"
    );

    // And nothing was written.
    let all = project
        .db()
        .timetables()
        .for_cycle(selection.cycle_id, selection.term_id)
        .expect("read");
    assert_eq!(all.len(), 1, "the refusal wrote no timetable");
}

#[test]
fn a_repair_whose_baseline_was_edited_is_refused() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let r = repair_whole(&mut project, &selection, written.timetable.id);

    // Somebody edits the baseline in another window.
    let entries = project
        .db()
        .timetables()
        .entries(written.timetable.id)
        .expect("entries");
    let one = entries.first().expect("an entry").clone();
    set_lock_now(
        &mut project,
        &selection,
        written.timetable.id,
        one.id,
        one.rev,
        true,
    );

    let error = save_repair(&mut project, &selection, &r, "Stale").expect_err("refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { .. }),
        "expected an arrangement change, got {error:?}"
    );
    assert_eq!(
        project
            .db()
            .timetables()
            .for_cycle(selection.cycle_id, selection.term_id)
            .expect("read")
            .len(),
        1,
        "and nothing was written"
    );
}

#[test]
fn a_repair_under_a_stale_source_revision_is_refused() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let mut r = repair_whole(&mut project, &selection, written.timetable.id);
    r.source_rev += 7;

    let error = save_repair(&mut project, &selection, &r, "Stale rev").expect_err("refused");
    assert!(
        matches!(error, DbError::Conflict { .. }),
        "expected a conflict, got {error:?}"
    );
}

#[test]
fn a_repair_whose_inputs_moved_is_refused() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let r = repair_whole(&mut project, &selection, written.timetable.id);

    // A scheduling input moves: a new room. Nothing referential breaks, which is why the
    // watermark is the only thing that can catch it.
    let mut school = school;
    school.room(&mut project, "Room 99", 0, None, 30, &[]);

    let error = save_repair(&mut project, &selection, &r, "Stale inputs").expect_err("refused");
    assert!(
        matches!(error, DbError::InputsChanged { .. }),
        "expected an input change, got {error:?}"
    );
}

#[test]
fn a_failed_repair_save_leaves_nothing_at_all() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let r = repair_whole(&mut project, &selection, written.timetable.id);
    // Take the name the scope already holds, which fails on the timetable insert itself.
    let error = save_repair(&mut project, &selection, &r, "Version 1").expect_err("refused");
    assert!(
        matches!(error, DbError::DuplicateValue { .. }),
        "expected a duplicate name, got {error:?}"
    );

    assert_eq!(
        project
            .db()
            .timetables()
            .for_cycle(selection.cycle_id, selection.term_id)
            .expect("read")
            .len(),
        1,
        "no timetable"
    );
    // And no orphaned run or evaluation: the whole transaction rolled back.
    let runs: i64 = project
        .db()
        .timetables()
        .descendant_count(written.timetable.id)
        .expect("count");
    assert_eq!(runs, 0, "no descendant was recorded");
}

// =======================================================================================
// Lineage
// =======================================================================================

#[test]
fn deleting_a_source_with_descendants_is_refused_readably() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let r = repair_whole(&mut project, &selection, written.timetable.id);
    let repaired = save_repair(&mut project, &selection, &r, "Repaired").expect("save");

    assert_eq!(
        project
            .db()
            .timetables()
            .descendant_count(written.timetable.id)
            .expect("count"),
        1
    );

    let source_rev = project
        .db()
        .timetables()
        .for_cycle(selection.cycle_id, selection.term_id)
        .expect("read")
        .into_iter()
        .find(|t| t.id == written.timetable.id)
        .expect("source")
        .rev;

    let error = project
        .db()
        .timetables()
        .delete(written.timetable.id, source_rev)
        .expect_err("refused");
    assert!(
        matches!(error, DbError::HasDescendants { timetables: 1 }),
        "expected a readable refusal, got {error:?}"
    );

    // Both survive, so the refusal cost nothing.
    assert_eq!(
        project
            .db()
            .timetables()
            .for_cycle(selection.cycle_id, selection.term_id)
            .expect("read")
            .len(),
        2
    );

    // Deleting the descendant first, then the source, works.
    project
        .db()
        .timetables()
        .delete(repaired.timetable.id, repaired.timetable.rev)
        .expect("delete the repair");
    project
        .db()
        .timetables()
        .delete(written.timetable.id, source_rev)
        .expect("then the source");
}

// =======================================================================================
// Unexpressible locks
// =======================================================================================

#[test]
fn a_lock_outside_the_compiled_problem_is_reported_with_its_cause() {
    // The dangerous case: the lock is ABSENT from `pinned`, so nothing would freeze it, and a
    // repair whose scope included the occurrence would MOVE it. Which is why a repair must
    // refuse rather than warn.
    //
    // Produced the way it actually happens -- by compiling a problem that does not contain the
    // locked lesson -- rather than by writing a bad row: `timetable_entry` is guarded by a
    // trigger pair that refuses a slot from another cycle, so the broken state cannot be
    // fabricated and does not need to be.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let north_selection = north.selection();
    let south_selection = south.selection();

    let south_written = saved(&mut project, &south_selection, "South 1");
    let entries = project
        .db()
        .timetables()
        .entries(south_written.timetable.id)
        .expect("entries");
    let one = entries.first().expect("an entry").clone();
    set_lock_now(
        &mut project,
        &south_selection,
        south_written.timetable.id,
        one.id,
        one.rev,
        true,
    );

    // Southmoor's locked timetable, read against NORTHGATE's compiled problem.
    let northgate = project.load(&north_selection).expect("load Northgate");
    let base = project
        .db()
        .timetables()
        .repair_baseline(south_written.timetable.id, &northgate)
        .expect("baseline");

    assert_eq!(
        base.pins.locked, 1,
        "the lock was counted, so it was seen at all"
    );
    assert_eq!(base.pins.unexpressible.len(), 1);
    let found = &base.pins.unexpressible[0];
    assert_eq!(found.activity_id, one.activity_id);
    assert_eq!(
        found.cause,
        UnexpressibleLockCause::ActivityNotInProblem,
        "the lesson is not part of the problem being repaired"
    );
    assert_eq!(found.cause.key(), "activity-not-in-problem");
    assert!(
        base.pins.pinned.iter().all(Option::is_none),
        "and it is absent from the pins, which is what makes it blocking"
    );
    assert_eq!(
        base.pins.not_in_problem,
        vec![one.activity_id],
        "the older field still reports it, so M9A's editor is unaffected"
    );
}

#[test]
fn an_unexpressible_lock_is_never_cleared_by_reading_it() {
    // A pre-flight must not unlock something to let itself proceed -- that would be the
    // silent discarding the blocking policy exists to prevent.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let north_selection = north.selection();
    let south_selection = south.selection();

    let south_written = saved(&mut project, &south_selection, "South 1");
    let entries = project
        .db()
        .timetables()
        .entries(south_written.timetable.id)
        .expect("entries");
    let one = entries.first().expect("an entry").clone();
    set_lock_now(
        &mut project,
        &south_selection,
        south_written.timetable.id,
        one.id,
        one.rev,
        true,
    );

    let northgate = project.load(&north_selection).expect("load Northgate");
    let _ = project
        .db()
        .timetables()
        .repair_baseline(south_written.timetable.id, &northgate)
        .expect("baseline");

    let after = project
        .db()
        .timetables()
        .entries(south_written.timetable.id)
        .expect("entries")
        .into_iter()
        .find(|e| e.id == one.id)
        .expect("still there");
    assert!(
        after.is_locked,
        "reading an unexpressible lock must not clear it"
    );
    assert_eq!(
        after.rev,
        one.rev + 1,
        "only the deliberate lock wrote to it"
    );
}

// =======================================================================================
// School isolation
// =======================================================================================

#[test]
fn a_repair_of_one_school_writes_nothing_of_anothers() {
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let north_selection = north.selection();
    let south_selection = south.selection();

    let north_written = saved(&mut project, &north_selection, "North 1");
    let south_written = saved(&mut project, &south_selection, "South 1");

    let r = repair_whole(&mut project, &north_selection, north_written.timetable.id);
    save_repair(&mut project, &north_selection, &r, "North repaired").expect("save");

    // Southmoor is untouched.
    let south_now = project
        .db()
        .timetables()
        .for_cycle(south_selection.cycle_id, south_selection.term_id)
        .expect("read");
    assert_eq!(south_now.len(), 1);
    assert_eq!(south_now[0], south_written.timetable);

    let north_now = project
        .db()
        .timetables()
        .for_cycle(north_selection.cycle_id, north_selection.term_id)
        .expect("read");
    assert_eq!(north_now.len(), 2);
}

#[test]
fn an_edit_to_another_schools_timetable_is_not_this_repairs_conflict() {
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let north_selection = north.selection();
    let south_selection = south.selection();

    let north_written = saved(&mut project, &north_selection, "North 1");
    let south_written = saved(&mut project, &south_selection, "South 1");

    let r = repair_whole(&mut project, &north_selection, north_written.timetable.id);

    // Southmoor's timetable is edited mid-repair. `timetable_entry` is in
    // SOLVE_OUTPUT_TABLES, so this moves no scheduling input, and the stamp is scoped to
    // Northgate's own entries -- so neither guard should fire.
    let south_entries = project
        .db()
        .timetables()
        .entries(south_written.timetable.id)
        .expect("entries");
    let one = south_entries.first().expect("an entry").clone();
    set_lock_now(
        &mut project,
        &south_selection,
        south_written.timetable.id,
        one.id,
        one.rev,
        true,
    );

    save_repair(&mut project, &north_selection, &r, "North repaired")
        .expect("another school's edit is not this repair's conflict");
}

// =======================================================================================
// Migration
// =======================================================================================

#[test]
fn a_v17_project_upgrades_to_v18_with_every_row_intact() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let before_timetable = written.timetable.clone();
    let before_entries = project
        .db()
        .timetables()
        .entries(written.timetable.id)
        .expect("entries");
    let before_run = project
        .db()
        .timetables()
        .run_for(written.timetable.id)
        .expect("run")
        .expect("there is one");
    let before_evaluations = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("evaluations");

    // A genuine rewind to V17: the index, then all three columns, then the history row --
    // so reopening re-runs `V18__timetable_repair.sql` against a project that already holds a
    // real timetable, a real run and a real scoring. `ALTER TABLE ... DROP COLUMN` is
    // available here because the index is gone first and nothing else references the columns.
    project
        .db()
        .write(|tx| {
            tx.execute_batch(
                "DROP INDEX IF EXISTS idx_timetable_based_on;
                 ALTER TABLE timetable DROP COLUMN based_on_timetable_id;
                 ALTER TABLE solve_run DROP COLUMN kind;
                 ALTER TABLE solve_run DROP COLUMN scope",
            )?;
            tx.execute("DELETE FROM refinery_schema_history WHERE version = 18", [])?;
            Ok(())
        })
        .expect("rewind to V17");

    // Confirm the rewind really happened, or the rest of this test proves nothing.
    let at_v17: i64 = project
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('timetable')
                 WHERE name = 'based_on_timetable_id'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("probe the schema");
    assert_eq!(at_v17, 0, "the project is genuinely back at V17");

    project.reopen();

    // Every row survived, byte for byte.
    let after_timetable = project
        .db()
        .timetables()
        .for_cycle(selection.cycle_id, selection.term_id)
        .expect("read")
        .into_iter()
        .find(|t| t.id == before_timetable.id)
        .expect("the timetable survived the upgrade");
    assert_eq!(after_timetable, before_timetable);
    assert_eq!(
        after_timetable.based_on_timetable_id, None,
        "an existing timetable has no lineage, and the default supplies it"
    );

    assert_eq!(
        project
            .db()
            .timetables()
            .entries(before_timetable.id)
            .expect("entries"),
        before_entries
    );
    assert_eq!(
        project
            .db()
            .timetables()
            .run_for(before_timetable.id)
            .expect("run")
            .expect("still there"),
        before_run
    );
    assert_eq!(
        project
            .db()
            .timetables()
            .evaluations(before_timetable.id)
            .expect("evaluations"),
        before_evaluations
    );
    assert_eq!(
        before_run.kind,
        crate::RUN_GENERATION,
        "and an existing run reads as a generation"
    );

    // And the upgraded project can still take a repair.
    let r = repair_whole(&mut project, &selection, before_timetable.id);
    save_repair(&mut project, &selection, &r, "Repaired after upgrade")
        .expect("repair works after the upgrade");
}
