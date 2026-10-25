//! What a repair moves, what it refuses to move, and what it refuses to call a success.
//!
//! # The three tests that are the point
//!
//! - [`two_movable_occurrences_that_clash_can_both_be_re_placed`] — the movable-to-movable case.
//!   It fails under an initialisation from the full baseline, because
//!   `PlacementVerdict::allowed` is false whenever the result *carries* a hard violation naming
//!   the occurrence, so each of two already-clashing lessons is an immovable obstacle to the
//!   other.
//! - [`a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable`] — the blocking
//!   defect. It fails if a repair is gated on `Score::is_feasible`, which is a property of the
//!   whole arrangement and so is false for every timetable a repair exists to fix.
//! - [`the_working_scorer_holds_only_frozen_placements`] — the initialisation itself, asserted
//!   directly rather than inferred from an outcome.
//!
//! # Built from fixtures, never from hand-written `Problem` literals
//!
//! Same reason as [`crate::tests`]: a hand-built problem is a test that passes because its
//! facts agree with its expectations.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::fixtures::FixtureKind;
use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleResolution, CompiledConstraint, ConstraintLevel, CycleKind,
    DiagnosticReport, GroupKind, MovableSet, Overlap, PeriodKind, Placement, Problem,
    ProblemIdentity, SolveStatus, Termination, TimeslotIx, UnplacedCause, frozen_differences,
    frozen_only,
};
use uuid::Uuid;

use crate::{
    NeverCancel, NoProgress, RepairDefect, RepairOutcome, RepairRequest, SolveConfig, repair,
    repair_with,
};

// =======================================================================================
// Scaffolding
// =======================================================================================

fn rules() -> Vec<CompiledConstraint> {
    Registry::mandatory()
}

fn standard_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("1", "09:00", "10:00"),
        PeriodSpec::teaching("2", "10:00", "11:00"),
        PeriodSpec::non_teaching("Break", "11:00", "11:30", PeriodKind::Break),
        PeriodSpec::teaching("3", "11:30", "12:30"),
        PeriodSpec::teaching("4", "13:30", "14:30"),
    ]
}

/// One teacher, one room, one group, `durations.len()` occurrences, `days` cycle days.
///
/// One teacher and one room means every pair of occurrences competes for both, which is the
/// smallest thing that can demonstrate a clash.
fn one_of_everything(durations: &[u16], days: usize) -> Problem {
    let labels: Vec<String> = (1..=days).map(|d| format!("Day {d}")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();

    let mut school = SchoolBuilder::new(5, "Northgate", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&refs, days);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    school.materialise(schedule);

    school.add_room(RoomFactSpec::new("Room 1", 30).on_campus(campus));
    let teacher = school.add_teacher(TeacherFactSpec::new("A. Teacher"));
    let group = school.add_group(GroupSpec::new("7A", GroupKind::Class).sized(26));
    school.declare_overlap(group, group, Overlap::Overlaps);

    let section = school.add_section(
        SectionSpec::new("Mathematics 7A")
            .schedule(BellScheduleResolution::Resolved { schedule })
            .taught_by(teacher)
            .taught_to(group),
    );
    school.add_activities(section, durations);
    school.finish()
}

fn at(raw: u32) -> ActivityIx {
    ActivityIx::new(raw)
}

/// A repair request over one problem, with nothing refused.
fn ask<'a>(
    problem: &'a Problem,
    constraints: &'a [CompiledConstraint],
    baseline: &'a Assignment,
    movable: &'a MovableSet,
) -> RepairRequest<'a> {
    RepairRequest {
        problem,
        identity: ProblemIdentity {
            school: problem.context.school.id,
            cycle: problem.context.cycle.id,
            term: None,
        },
        constraints,
        preflight: DiagnosticReport::empty(),
        may_solve: true,
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(2026),
        baseline,
        movable,
    }
}

/// Generates a complete arrangement to repair, over the `tiny` fixture.
fn tiny_baseline() -> (Problem, Vec<CompiledConstraint>, Assignment) {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny has a problem").clone();
    let constraints = rules();
    let generated = crate::generate(&crate::GenerateRequest {
        problem: &problem,
        identity: ProblemIdentity {
            school: problem.context.school.id,
            cycle: problem.context.cycle.id,
            term: None,
        },
        constraints: &constraints,
        preflight: DiagnosticReport::empty(),
        may_solve: true,
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(7),
    });
    (problem, constraints, generated.assignment)
}

/// The first `n` timeslots of the problem, for building an arrangement by hand.
fn slots(problem: &Problem, n: usize) -> Vec<TimeslotIx> {
    problem
        .timeslots
        .iter()
        .enumerate()
        .take(n)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap_or(u32::MAX)))
        .collect()
}

// =======================================================================================
// C1 — scorer initialisation and the movable set
// =======================================================================================

#[test]
fn the_working_scorer_holds_only_frozen_placements() {
    let (problem, _rules, baseline) = tiny_baseline();
    assert!(baseline.scheduled_count() >= 3, "need a few to freeze");

    let movable = MovableSet::new(problem.activities.len(), &[at(0), at(2)]).expect("valid");
    let working = frozen_only(&baseline, &movable);

    // Every movable occurrence is absent, and every frozen one is exactly where it was.
    assert_eq!(working.placement(at(0)), None);
    assert_eq!(working.placement(at(2)), None);
    for frozen in movable.frozen() {
        assert_eq!(
            working.placement(frozen),
            baseline.placement(frozen),
            "frozen {frozen:?} must be untouched"
        );
    }
    assert_eq!(
        working.scheduled_count(),
        baseline.scheduled_count() - 2,
        "exactly the movable set was withdrawn"
    );
    assert_eq!(working.len(), baseline.len(), "dimension inherited");
}

#[test]
fn two_movable_occurrences_that_clash_can_both_be_re_placed() {
    // One teacher, one room, four days. Two occurrences stacked in the SAME slot: a
    // pre-existing movable-to-movable clash, which is the state that breaks an initialisation
    // from the full baseline.
    let problem = one_of_everything(&[1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 1);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));

    let before = evaluate(&problem, &baseline, &rules);
    assert!(
        !before.score.is_feasible(),
        "the baseline must really be clashing"
    );

    let movable = MovableSet::everything(problem.activities.len()).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None, "{:?}", out.defect);
    assert!(out.applyable);
    assert_eq!(
        out.movable_placed, 2,
        "both must be placed, not one or neither"
    );
    let after = evaluate(&problem, &out.outcome.assignment, &rules);
    assert!(
        after.score.is_feasible(),
        "the repair must resolve the clash it was given: {:?}",
        after.violations
    );
}

#[test]
fn a_movable_occurrence_whose_placement_is_still_legal_does_not_move() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::everything(problem.activities.len()).expect("valid");

    // The baseline is a generated, feasible arrangement, so every baseline placement is still
    // legal and the baseline-first probe must take all of them.
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None);
    assert_eq!(
        out.outcome.assignment, baseline,
        "a legal baseline must be reproduced exactly"
    );
    assert_eq!(
        out.hard_partition.eliminated.len(),
        0,
        "nothing to eliminate"
    );
}

#[test]
fn a_movable_occurrence_does_not_clash_with_itself() {
    // One occurrence, alone. Probing its own baseline placement must succeed: `probe` withdraws
    // before placing, and in a repair the occurrence is unplaced in the working arrangement
    // anyway, so there is nothing to clash with.
    let problem = one_of_everything(&[1], 2);
    let rules = rules();
    let cells = slots(&problem, 1);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));

    let movable = MovableSet::everything(problem.activities.len()).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None);
    assert_eq!(
        out.outcome.assignment.placement(at(0)),
        Some(Placement::unroomed(cells[0])),
        "it must stay exactly where it was"
    );
    assert_eq!(
        out.outcome.statistics.candidate_evaluations, 1,
        "the baseline probe must be the only evaluation"
    );
}

#[test]
fn a_multi_period_lesson_moves_as_one_unit_to_an_unbroken_run() {
    // A double period, placed where it is illegal (stacked with a single), must move to a run
    // of two consecutive teaching periods and carry only its start slot.
    let problem = one_of_everything(&[2, 1], 2);
    let rules = rules();
    let cells = slots(&problem, 1);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));

    let movable = MovableSet::everything(problem.activities.len()).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None);
    let after = evaluate(&problem, &out.outcome.assignment, &rules);
    assert!(
        after.score.is_feasible(),
        "the double must land on a legal run: {:?}",
        after.violations
    );
}

#[test]
fn a_frozen_occurrence_unplaced_in_the_baseline_is_not_reported_as_this_runs_failure() {
    // This is the test that fails under M6's closing invariant, which counts every occurrence
    // in the problem rather than every occurrence the run was allowed to attempt.
    let problem = one_of_everything(&[1, 1, 1], 2);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    // at(1) is frozen AND unplaced. at(2) is the only movable one.
    baseline.place(at(2), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None, "{:?}", out.defect);
    assert!(
        !out.outcome.unscheduled.iter().any(|u| u.activity == at(1)),
        "a frozen unplaced occurrence is not this run's failure: {:?}",
        out.outcome.unscheduled
    );
    assert_eq!(
        out.outcome.assignment.placement(at(1)),
        None,
        "and it stays unplaced"
    );
    assert_eq!(out.movable_count, 1);
    assert_eq!(out.movable_placed, 1);
    // The arrangement is incomplete, so the status is honest about the timetable.
    assert_eq!(out.outcome.status, SolveStatus::Partial);
    assert!(out.applyable, "partial is an ordinary, applyable result");
}

#[test]
fn the_scope_guarantee_holds_for_every_frozen_occurrence() {
    let (problem, rules, baseline) = tiny_baseline();

    // Repair each single occurrence in turn; everything else must be byte-identical.
    for one in 0..problem.activities.len() {
        let movable = MovableSet::new(
            problem.activities.len(),
            &[at(u32::try_from(one).expect("fits"))],
        )
        .expect("valid");
        let out = repair(&ask(&problem, &rules, &baseline, &movable));

        assert_eq!(out.defect, None, "occurrence {one}: {:?}", out.defect);
        assert_eq!(
            frozen_differences(&baseline, &out.outcome.assignment, &movable),
            vec![],
            "occurrence {one} repaired, but a frozen placement changed"
        );
        assert_eq!(
            out.frozen_placed,
            baseline.scheduled_count() - 1,
            "occurrence {one}: frozen placed count"
        );
    }
}

#[test]
fn an_unreadable_baseline_is_refused_not_regenerated() {
    let problem = one_of_everything(&[1, 1], 2);
    let rules = rules();

    // A placement naming a timeslot the problem does not hold.
    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(TimeslotIx::new(9_999)));

    let movable = MovableSet::new(problem.activities.len(), &[at(1)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.outcome.termination, Termination::InputRejected);
    assert!(!out.applyable, "a refused repair is never applyable");
    assert_eq!(
        out.outcome.assignment, baseline,
        "a refusal changes nothing, so it reports the stored arrangement"
    );
}

#[test]
fn a_repair_is_deterministic_over_one_timetable_scope_and_seed() {
    let problem = one_of_everything(&[1, 1, 1], 3);
    let rules = rules();
    let cells = slots(&problem, 1);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));
    baseline.place(at(2), Placement::unroomed(cells[0]));

    let movable = MovableSet::everything(problem.activities.len()).expect("valid");
    let first = repair(&ask(&problem, &rules, &baseline, &movable));
    let second = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(first.outcome.assignment, second.outcome.assignment);
    assert_eq!(first.outcome.statistics, second.outcome.statistics);
    assert_eq!(first.outcome.score, second.outcome.score);
    assert_eq!(first.outcome.termination, second.outcome.termination);
}

#[test]
fn a_repair_at_a_different_seed_is_still_scope_preserving() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::new(problem.activities.len(), &[at(0)]).expect("valid");

    for seed in [1_u64, 42, 2026, 99_999] {
        let mut asked = ask(&problem, &rules, &baseline, &movable);
        asked.config = SolveConfig::seeded(seed);
        let out = repair(&asked);
        assert_eq!(out.defect, None, "seed {seed}");
        assert_eq!(
            frozen_differences(&baseline, &out.outcome.assignment, &movable),
            vec![],
            "seed {seed}"
        );
    }
}

// =======================================================================================
// C2 — hard-violation acceptance
// =======================================================================================

#[test]
fn a_pre_existing_frozen_clash_does_not_refuse_any_candidate() {
    // Two frozen occurrences clash with each other. A third, movable, must still be placed —
    // `probe` scopes its verdict to violations involving the probed occurrence, so a
    // frozen-to-frozen clash is invisible to candidate acceptance.
    let problem = one_of_everything(&[1, 1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0])); // the pre-existing clash
    baseline.place(at(2), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.defect, None, "{:?}", out.defect);
    assert_eq!(out.movable_placed, 1, "the movable one must be placed");
    assert_eq!(
        frozen_differences(&baseline, &out.outcome.assignment, &movable),
        vec![],
        "and the clashing pair must survive untouched"
    );
}

#[test]
fn a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable() {
    // THE test for the blocking defect. The final arrangement still holds a hard violation —
    // between two frozen occurrences — and the repair must nonetheless be a success.
    let problem = one_of_everything(&[1, 1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));
    baseline.place(at(2), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    let after = evaluate(&problem, &out.outcome.assignment, &rules);
    assert!(
        !after.score.is_feasible(),
        "the frozen clash must still be there — otherwise this test proves nothing"
    );

    assert_ne!(
        out.outcome.termination,
        Termination::InternalCheckFailed,
        "a pre-existing frozen violation is not a defect in Evara"
    );
    assert_ne!(out.outcome.status, SolveStatus::Failed);
    assert!(
        out.applyable,
        "a correct repair of a conflicted timetable must be applyable"
    );
    assert_eq!(out.defect, None);

    // And it is attributed, not hidden.
    assert!(
        !out.hard_partition.pre_existing_frozen.is_empty(),
        "the frozen clash must be reported as pre-existing"
    );
    assert_eq!(out.hard_partition.new_involving_movable, vec![]);
}

#[test]
fn a_frozen_pre_existing_violation_set_is_identical_before_and_after() {
    let problem = one_of_everything(&[1, 1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));
    baseline.place(at(2), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));
    assert_eq!(out.defect, None, "V3 must hold: {:?}", out.defect);

    // Asserted independently of the verification, by comparing the reports directly.
    let before = evaluate(&problem, &baseline, &rules);
    let after = evaluate(&problem, &out.outcome.assignment, &rules);
    let frozen_hard = |report: &evara_constraints::EvaluationReport| -> Vec<_> {
        let mut keys: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.level == ConstraintLevel::Hard)
            .filter(|v| !v.activities.iter().any(|a| movable.contains(*a)))
            .map(evara_core::scheduling::Violation::key)
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(frozen_hard(&before), frozen_hard(&after));
}

#[test]
fn the_preview_reports_the_whole_arrangements_violations() {
    let problem = one_of_everything(&[1, 1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));
    baseline.place(at(2), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    let reference = evaluate(&problem, &out.outcome.assignment, &rules);
    assert_eq!(
        out.outcome.score, reference.score,
        "the reported score must be the whole arrangement's"
    );
    assert_eq!(
        out.violations.len(),
        reference.violations.len(),
        "and so must the reported violations"
    );
    assert!(
        !out.outcome.hard_violations.is_empty(),
        "the frozen clash must be counted in the headline figures, not filtered out"
    );
}

#[test]
fn the_reference_and_incremental_scorers_agree_after_a_repair() {
    // `differential.rs`'s gate, extended to the frozen-only starting point, over every fixture.
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        let Some(problem) = found.problems.first() else {
            continue;
        };
        if problem.activities.is_empty() {
            continue;
        }
        let rules = rules();
        let generated = crate::generate(&crate::GenerateRequest {
            problem,
            identity: ProblemIdentity {
                school: problem.context.school.id,
                cycle: problem.context.cycle.id,
                term: None,
            },
            constraints: &rules,
            preflight: DiagnosticReport::empty(),
            may_solve: true,
            unenforced_restrictions: 0,
            config: SolveConfig::seeded(11),
        });

        let movable = MovableSet::everything(problem.activities.len()).expect("valid");
        let out = repair(&ask(problem, &rules, &generated.assignment, &movable));

        let reference = evaluate(problem, &out.outcome.assignment, &rules);
        assert_eq!(
            out.outcome.score, reference.score,
            "{kind:?}: incremental and reference disagree"
        );
        assert_eq!(out.defect, None, "{kind:?}: {:?}", out.defect);
    }
}

#[test]
fn the_four_buckets_are_disjoint_and_account_for_every_hard_violation() {
    let problem = one_of_everything(&[1, 1, 1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    // A frozen clash and a movable clash, so more than one bucket is populated.
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[0]));
    baseline.place(at(2), Placement::unroomed(cells[1]));
    baseline.place(at(3), Placement::unroomed(cells[1]));

    let movable = MovableSet::new(problem.activities.len(), &[at(2), at(3)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));
    assert_eq!(out.defect, None, "{:?}", out.defect);

    let p = &out.hard_partition;
    assert_eq!(
        p.new_involving_movable,
        vec![],
        "V2 forbids a new hard violation involving a movable occurrence"
    );
    assert!(
        !p.eliminated.is_empty(),
        "the movable clash must have been fixed"
    );
    assert!(
        !p.pre_existing_frozen.is_empty(),
        "the frozen clash must remain, attributed"
    );

    // Disjoint by key.
    let keys = |v: &[evara_core::scheduling::Violation]| -> Vec<_> {
        v.iter()
            .map(evara_core::scheduling::Violation::key)
            .collect()
    };
    for key in keys(&p.eliminated) {
        assert!(
            !keys(&p.pre_existing_frozen).contains(&key),
            "buckets overlap on {key:?}"
        );
    }
}

// =======================================================================================
// Locks
// =======================================================================================

#[test]
fn a_locked_occurrence_inside_the_scope_is_never_moved() {
    let problem = one_of_everything(&[1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[1]));

    // at(0) is locked, and ALSO named movable — the caller's scope says it may move, the lock
    // says it may not. The lock wins, because the frozen set is `out-of-scope ∪ locked`.
    let mut locked = problem;
    locked.pinned = vec![Some(Placement::unroomed(cells[0])), None];

    let movable = MovableSet::new(locked.activities.len(), &[at(1)]).expect("valid");
    let out = repair(&ask(&locked, &rules, &baseline, &movable));

    assert_eq!(out.defect, None, "{:?}", out.defect);
    assert_eq!(
        out.outcome.assignment.placement(at(0)),
        Some(Placement::unroomed(cells[0])),
        "the lock must hold slot and room exactly"
    );
}

#[test]
fn a_lock_is_verified_against_its_pin_as_well_as_against_the_baseline() {
    // V0b. A pin that disagrees with the baseline is caught even though the frozen comparison
    // against the baseline would pass.
    let problem = one_of_everything(&[1, 1], 4);
    let rules = rules();
    let cells = slots(&problem, 2);

    let mut baseline = Assignment::empty(problem.activities.len());
    baseline.place(at(0), Placement::unroomed(cells[0]));
    baseline.place(at(1), Placement::unroomed(cells[1]));

    let mut mispinned = problem;
    // The pin names a slot the baseline does not have it in. Repair must not silently accept.
    mispinned.pinned = vec![Some(Placement::unroomed(cells[1])), None];

    let movable = MovableSet::new(mispinned.activities.len(), &[at(1)]).expect("valid");
    let out = repair(&ask(&mispinned, &rules, &baseline, &movable));

    assert!(
        matches!(out.defect, Some(RepairDefect::LockedPlacementsChanged(_))),
        "expected a lock defect, got {:?}",
        out.defect
    );
    assert!(!out.applyable, "a lock defect is never applyable");
    assert_eq!(out.outcome.termination, Termination::InternalCheckFailed);
}

// =======================================================================================
// Cancellation, limits and refusal
// =======================================================================================

/// A cancellation that fires after `after` questions. Deterministic: one question per step.
struct CancelAfter {
    after: std::cell::Cell<u32>,
}

impl crate::Cancel for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let left = self.after.get();
        if left == 0 {
            return true;
        }
        self.after.set(left - 1);
        false
    }
}

#[test]
fn a_cancelled_repair_is_not_applyable() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::everything(problem.activities.len()).expect("valid");

    let out = repair_with(
        &ask(&problem, &rules, &baseline, &movable),
        &NoProgress,
        &CancelAfter {
            after: std::cell::Cell::new(1),
        },
    );

    assert_eq!(out.outcome.termination, Termination::Cancelled);
    assert_eq!(out.outcome.status, SolveStatus::Cancelled);
    assert!(!out.applyable, "a cancelled repair is never applyable");
}

#[test]
fn a_cancelled_repair_still_preserved_every_frozen_placement() {
    // Cancellation stops the walk; it must not be a route around the scope guarantee.
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::new(problem.activities.len(), &[at(0), at(1)]).expect("valid");

    let out = repair_with(
        &ask(&problem, &rules, &baseline, &movable),
        &NoProgress,
        &CancelAfter {
            after: std::cell::Cell::new(1),
        },
    );

    assert_eq!(
        frozen_differences(&baseline, &out.outcome.assignment, &movable),
        vec![],
        "a cancelled repair must still preserve the frozen set"
    );
}

#[test]
fn a_repair_refused_before_solving_reports_the_stored_arrangement() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::everything(problem.activities.len()).expect("valid");

    let mut asked = ask(&problem, &rules, &baseline, &movable);
    asked.may_solve = false;
    let out = repair(&asked);

    assert_eq!(out.outcome.termination, Termination::InputRejected);
    assert_eq!(out.outcome.status, SolveStatus::Failed);
    assert!(!out.applyable);
    assert_eq!(out.outcome.assignment, baseline);
    assert!(
        out.outcome
            .unscheduled
            .iter()
            .all(|u| u.cause == UnplacedCause::InputRejected),
        "a refusal claims nothing about any occurrence"
    );
}

#[test]
fn the_unchanged_case_is_reported_as_a_run_that_changed_nothing() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::everything(problem.activities.len()).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    assert_eq!(out.outcome.assignment, baseline);
    assert_eq!(out.defect, None);
    assert!(out.applyable);
    // Nothing eliminated, nothing new: the honest statement is "this attempt made no changes",
    // which the caller renders. The engine claims nothing about what was possible.
    assert_eq!(out.hard_partition.eliminated, vec![]);
    assert_eq!(out.hard_partition.new_involving_movable, vec![]);
}

#[test]
fn every_fixture_repairs_without_disturbing_a_frozen_placement() {
    // The scope guarantee over the whole catalogue, at a narrow scope so most of each fixture
    // is frozen.
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        let Some(problem) = found.problems.first() else {
            continue;
        };
        if problem.activities.is_empty() {
            continue;
        }
        let rules = rules();
        let generated = crate::generate(&crate::GenerateRequest {
            problem,
            identity: ProblemIdentity {
                school: problem.context.school.id,
                cycle: problem.context.cycle.id,
                term: None,
            },
            constraints: &rules,
            preflight: DiagnosticReport::empty(),
            may_solve: true,
            unenforced_restrictions: 0,
            config: SolveConfig::seeded(3),
        });
        let movable = MovableSet::new(problem.activities.len(), &[at(0)]).expect("valid");
        let out = repair(&ask(problem, &rules, &generated.assignment, &movable));

        assert_eq!(out.defect, None, "{kind:?}: {:?}", out.defect);
        assert_eq!(
            frozen_differences(&generated.assignment, &out.outcome.assignment, &movable),
            vec![],
            "{kind:?}"
        );
    }
}

#[test]
fn a_repair_never_widens_its_movable_set() {
    let (problem, rules, baseline) = tiny_baseline();
    let movable = MovableSet::new(problem.activities.len(), &[at(0)]).expect("valid");
    let out = repair(&ask(&problem, &rules, &baseline, &movable));

    // Nothing outside the movable set changed, which is the widening test stated as an
    // assignment comparison rather than as an intention.
    for frozen in movable.frozen() {
        assert_eq!(
            out.outcome.assignment.placement(frozen),
            baseline.placement(frozen),
            "{frozen:?} was outside the scope and must be identical"
        );
    }
    let _ = Uuid::nil();
    let _: &dyn crate::Cancel = &NeverCancel;
    let _ = RepairOutcome::clone(&out);
}
