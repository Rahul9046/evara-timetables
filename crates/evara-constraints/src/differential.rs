//! The gate: the incremental scorer is only trusted because the cheap one keeps proving it
//! right.
//!
//! # Why this file is a deliverable and not a test file
//!
//! `SOLVER.md` §4 and `OPEN-DECISIONS.md` R1 — the highest-rated technical risk in the
//! project — both state the same condition, and R1 states it as a rule about shipping:
//!
//! > M6 inherits `IncrementalScorer` **and** the property test in the same milestone. Neither
//! > half may land without the other.
//!
//! *"A wrong delta score silently produces a wrong timetable, and it is the hardest part of any
//! solver to debug."* Nothing in a passing solver run would reveal it: the timetable would look
//! finished and be wrong. So the assertions here are the reason the fast path may be used at
//! all, and a failure in this module is never to be worked around by changing what it expects.
//!
//! # Three properties, in increasing strength
//!
//! **1. Each rule's narrowed query is complete** —
//! [`union_of_involving_is_the_full_evaluation`]. For every fixture and every configured rule,
//! the union of [`ConstraintEvaluator::evaluate_involving`](crate::ConstraintEvaluator) over
//! every occurrence, deduplicated, is exactly what [`evaluate`] produces. This is the property
//! the incremental identity rests on, and it is asserted per rule so a failure names the rule
//! rather than the total.
//!
//! **2. A mutated occupancy equals a rebuilt one** —
//! [`a_mutated_occupancy_equals_one_built_from_the_assignment`]. Place and withdraw are not
//! allowed to drift from [`Occupancy::of`](crate::Occupancy), because every narrowing above
//! reads the indexes they maintain.
//!
//! **3. The running score equals the reference score, after every single operation** —
//! [`the_running_score_agrees_with_the_reference_after_every_operation`]. A seeded sequence of
//! adds, removals, replacements and rejected probes over a real school, checked after each
//! step. This is `SOLVER.md`'s *"applies thousands of random moves to random synthetic problems
//! and asserts `IncrementalScorer == ReferenceScorer` after every single one"*.
//!
//! # Seeded, never random
//!
//! The sequences come from [`Rng`], the same SplitMix64 the fixtures and the solver use, so a
//! failure is reproducible from the seed printed in the assertion rather than being a flake
//! somebody reruns until it passes.

use evara_core::scheduling::fixtures::{all_fixtures, fixture};
use evara_core::scheduling::{
    ActivityIx, Assignment, CompiledConstraint, ConstraintType, Placement, Problem, Rng, RoomIx,
    Score, TimeslotIx, Violation, ViolationKey, ViolationSink, runs_on,
};

use crate::occupancy::Occupancy;
use crate::registry::evaluator_for;
use crate::{IncrementalScorer, Registry, evaluate};

/// The mandatory set — what every project evaluates.
fn rules() -> Vec<CompiledConstraint> {
    Registry::mandatory()
}

/// Every violation of one rule that names `activity`.
fn involving_one(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    activity: ActivityIx,
) -> Vec<Violation> {
    let mut sink = ViolationSink::new();
    evaluator_for(constraint.constraint_type)
        .evaluate_involving(problem, occupancy, constraint, activity, &mut sink);
    sink.into_violations()
}

/// Every violation of one rule, swept over the whole arrangement.
fn swept_one(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
) -> Vec<Violation> {
    let mut sink = ViolationSink::new();
    evaluator_for(constraint.constraint_type).evaluate(problem, occupancy, constraint, &mut sink);
    sink.deduplicate();
    sink.into_violations()
}

/// The deduplicated keys of a violation list, sorted so two lists compare as sets.
fn keys(violations: &[Violation]) -> Vec<ViolationKey> {
    let mut found: Vec<ViolationKey> = violations.iter().map(Violation::key).collect();
    found.sort();
    found.dedup();
    found
}

/// An arrangement that places every occurrence it can, legally or not.
///
/// Deliberately **not** a feasible arrangement: the properties here must hold over broken
/// timetables as well, and a generator that only produced legal ones would never exercise the
/// pair rules at all. Slots are chosen by a seeded walk over the day's runs, so clashes are
/// common and reproducible.
fn crowded(problem: &Problem, seed: u64) -> Assignment {
    let mut rng = Rng::new(seed);
    let mut assignment = Assignment::empty(problem.activities.len());
    for activity in problem.activity_indices() {
        if let Some(placement) = some_placement(problem, activity, &mut rng) {
            assignment.place(activity, placement);
        }
    }
    assignment
}

/// Any placement for one occurrence, chosen by a seeded draw. `None` where it has no run.
///
/// Uses [`runs_on`] for the slot so the span resolves to something, and picks a room from the
/// section's candidates — or no room — without caring whether the result is legal.
fn some_placement(problem: &Problem, activity: ActivityIx, rng: &mut Rng) -> Option<Placement> {
    let fact = problem.activities.get(activity.get())?;
    let section = problem.sections.get(fact.section.get())?;
    let schedule = section.bell_schedule.schedule()?;

    let days: Vec<usize> = (0..problem.context.cycle_days.len()).collect();
    let day = *rng.pick(&days)?;
    let day = evara_core::scheduling::CycleDayIx::new(u32::try_from(day).unwrap_or(0));
    let runs = runs_on(problem, schedule, day, fact.duration.as_usize());
    let run = rng.pick(&runs)?;
    let start: TimeslotIx = *run.first()?;

    // A room from the candidate set, or none at all. Both are states a real assignment reaches.
    let room: Option<RoomIx> = if section.room_candidates.is_empty() || rng.chance(1, 5) {
        None
    } else {
        rng.pick(&section.room_candidates).copied()
    };
    Some(Placement {
        timeslot: start,
        room,
    })
}

// =======================================================================================
// Property 1 — each rule's narrowed query is complete
// =======================================================================================

#[test]
fn union_of_involving_is_the_full_evaluation() {
    // The property the whole incremental identity rests on, asserted per rule and per fixture.
    // A rule that narrowed too far fails this by omission; one that narrowed wrongly fails it
    // by producing a violation the sweep does not. There is no third outcome.
    let rules = rules();
    for found in all_fixtures(2026) {
        for (n, problem) in found.problems.iter().enumerate() {
            let assignment = crowded(problem, 11);
            let occupancy = Occupancy::of(problem, &assignment);
            assert!(
                occupancy.invalid_references().is_empty(),
                "{} problem {n}: the generated arrangement must be readable",
                found.name
            );

            for constraint in &rules {
                let swept = keys(&swept_one(problem, &occupancy, constraint));

                let mut narrowed = Vec::new();
                for activity in problem.activity_indices() {
                    narrowed.extend(involving_one(problem, &occupancy, constraint, activity));
                }
                let narrowed = keys(&narrowed);

                assert_eq!(
                    narrowed,
                    swept,
                    "{} problem {n}, rule {}: the narrowed query and the sweep disagree",
                    found.name,
                    constraint.constraint_type.key()
                );
            }
        }
    }
}

#[test]
fn a_narrowed_query_names_only_the_occurrence_it_was_asked_about() {
    // The other half of the identity: a violation returned for `a` must actually name `a`,
    // or the delta would move a figure belonging to somebody else.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let assignment = crowded(problem, 3);
    let occupancy = Occupancy::of(problem, &assignment);

    for constraint in &rules {
        for activity in problem.activity_indices() {
            for violation in involving_one(problem, &occupancy, constraint, activity) {
                assert!(
                    violation.activities.contains(&activity),
                    "rule {} returned a violation about {:?} when asked about {activity:?}",
                    constraint.constraint_type.key(),
                    violation.activities
                );
            }
        }
    }
}

#[test]
fn every_violation_the_engine_produces_names_at_most_two_occurrences() {
    // Not a style check. The incremental identity is sound *because* a violation names at most
    // two occurrences; a three-occurrence rule would break it silently, so the assumption is
    // asserted over the whole catalogue rather than left in a comment.
    let rules = rules();
    for found in all_fixtures(2026) {
        for problem in &found.problems {
            let assignment = crowded(problem, 5);
            let report = evaluate(problem, &assignment, &rules);
            for violation in &report.violations {
                assert!(
                    violation.activities.len() <= 2,
                    "{}: {} names {} occurrences",
                    found.name,
                    violation.constraint_type.key(),
                    violation.activities.len()
                );
            }
        }
    }
}

// =======================================================================================
// Property 2 — a mutated occupancy equals a rebuilt one
// =======================================================================================

#[test]
fn a_mutated_occupancy_equals_one_built_from_the_assignment() {
    // Every narrowing in this crate reads the indexes `place` and `withdraw` maintain, so they
    // are not allowed to drift from `Occupancy::of`. Equality is on the whole structure,
    // indexes included, which is why `Occupancy` derives `PartialEq`.
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let mut rng = Rng::new(19);

    let mut built = Occupancy::empty(problem);
    let mut assignment = Assignment::empty(problem.activities.len());

    for step in 0..200 {
        let activities: Vec<ActivityIx> = problem.activity_indices().collect();
        let Some(activity) = rng.pick(&activities).copied() else {
            break;
        };

        if rng.chance(1, 3) {
            built.withdraw(activity);
            assignment.clear(activity);
        } else if let Some(placement) = some_placement(problem, activity, &mut rng) {
            assert!(
                built.place(activity, placement),
                "step {step}: a placement built from the problem must be readable"
            );
            assignment.place(activity, placement);
        }

        assert_eq!(
            built,
            Occupancy::of(problem, &assignment),
            "step {step}: a mutated occupancy drifted from a rebuilt one"
        );
    }
}

#[test]
fn an_unreadable_placement_leaves_the_occupancy_untouched() {
    // `place` returning false must change nothing at all, not even the previous placement —
    // otherwise a caller that offered a bad candidate would silently lose a good one.
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let mut occupancy = Occupancy::empty(problem);

    let good = Placement::unroomed(TimeslotIx::new(0));
    assert!(occupancy.place(ActivityIx::new(0), good));
    let before = occupancy.clone();

    let past_the_grid = TimeslotIx::new(u32::try_from(problem.timeslots.len()).unwrap_or(0));
    assert!(
        !occupancy.place(ActivityIx::new(0), Placement::unroomed(past_the_grid)),
        "a slot outside the grid is not readable"
    );
    assert_eq!(before, occupancy, "a refused placement changed something");

    let past_the_activities = ActivityIx::new(u32::try_from(problem.activities.len()).unwrap_or(0));
    assert!(!occupancy.place(past_the_activities, good));
    assert_eq!(before, occupancy);
}

// =======================================================================================
// Property 3 — the running score equals the reference score, always
// =======================================================================================

#[test]
fn the_running_score_agrees_with_the_reference_after_every_operation() {
    // `SOLVER.md` §4's property test. Several fixtures, several seeds, and a check after every
    // single operation rather than at the end — a delta that is wrong and then wrong the other
    // way would pass an end-only check.
    let rules = rules();
    for name in [
        "tiny",
        "two-campuses-different-bells",
        "overlapping-group-membership",
        "unknown-group-overlap",
        "missing-availability-context",
    ] {
        let found = fixture(name, 2026).unwrap_or_else(|| panic!("{name} is in the catalogue"));
        for seed in [1_u64, 7, 4_242] {
            let problem = found.problem();
            let mut scorer = IncrementalScorer::empty(problem, &rules);
            let mut rng = Rng::new(seed);

            for step in 0..120 {
                let taken = one_step(problem, &mut scorer, &mut rng);

                let reference = evaluate(problem, &scorer.assignment(), &rules);
                assert_eq!(
                    scorer.score(),
                    reference.score,
                    "{name} seed {seed} step {step} after {taken:?}: the running score left \
                     the reference behind"
                );
                for kind in ConstraintType::all() {
                    assert_eq!(
                        u64::from(scorer.count_of(*kind)),
                        reference.count_of(*kind) as u64,
                        "{name} seed {seed} step {step} after {taken:?}: the count of {} is \
                         wrong",
                        kind.key()
                    );
                }
                assert_eq!(
                    scorer.is_usable(),
                    reference.is_usable(),
                    "{name} seed {seed} step {step}: usability disagrees"
                );
            }
        }
    }
}

/// Applies one random operation and says what it was.
fn one_step(
    problem: &Problem,
    scorer: &mut IncrementalScorer<'_>,
    rng: &mut Rng,
) -> Option<String> {
    let activities: Vec<ActivityIx> = problem.activity_indices().collect();
    let activity = *rng.pick(&activities)?;

    // A removal, a probe that must change nothing, and a placement — so the sequence covers
    // adds, replacements of an already-placed occurrence, removals and rejected candidates.
    if rng.chance(1, 4) {
        scorer.withdraw(activity);
        return Some(format!("withdrew {activity:?}"));
    }
    let placement = some_placement(problem, activity, rng)?;
    if rng.chance(1, 4) {
        scorer.probe(activity, placement);
        return Some(format!("probed {activity:?} at {placement:?}"));
    }
    scorer.place(activity, placement);
    Some(format!("placed {activity:?} at {placement:?}"))
}

#[test]
fn a_probe_changes_nothing_a_caller_can_observe() {
    // A probe that leaked state would corrupt every placement after it, and the corruption
    // would look like a solver bug rather than a scorer bug.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let mut scorer = IncrementalScorer::empty(problem, &rules);
    let mut rng = Rng::new(31);

    // Place some things first, so the probes are against a non-empty arrangement.
    for activity in problem.activity_indices().take(5) {
        if let Some(placement) = some_placement(problem, activity, &mut rng) {
            scorer.place(activity, placement);
        }
    }

    let activities: Vec<ActivityIx> = problem.activity_indices().collect();
    for step in 0..60 {
        let activity = *rng.pick(&activities).expect("a problem has activities");
        let Some(placement) = some_placement(problem, activity, &mut rng) else {
            continue;
        };
        let before_score = scorer.score();
        let before_assignment = scorer.assignment();
        let before_placed = scorer.placed_count();

        scorer.probe(activity, placement);

        assert_eq!(
            scorer.score(),
            before_score,
            "step {step}: probe moved the score"
        );
        assert_eq!(
            scorer.assignment(),
            before_assignment,
            "step {step}: probe moved a placement"
        );
        assert_eq!(scorer.placed_count(), before_placed);
    }
}

#[test]
fn a_probe_predicts_exactly_what_the_commit_then_does() {
    // The property a construction solver's every refusal depends on: if a probe said a
    // placement was legal and the commit disagreed, the solver would build an invalid
    // timetable while believing it had checked.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let mut scorer = IncrementalScorer::empty(problem, &rules);
    let mut rng = Rng::new(97);
    let activities: Vec<ActivityIx> = problem.activity_indices().collect();

    for step in 0..80 {
        let activity = *rng.pick(&activities).expect("a problem has activities");
        let Some(placement) = some_placement(problem, activity, &mut rng) else {
            continue;
        };

        let verdict = scorer.probe(activity, placement);
        let committed = scorer
            .place(activity, placement)
            .expect("a placement built from the problem is readable");

        assert_eq!(
            verdict.delta, committed,
            "step {step}: the probe's delta and the commit's disagreed"
        );
        assert_eq!(
            verdict.allowed,
            verdict.blocking.is_empty(),
            "step {step}: `allowed` and `blocking` must be the same answer"
        );
    }
}

#[test]
fn a_scorer_built_over_an_arrangement_starts_at_the_reference_score() {
    // `IncrementalScorer::of` is the repair entry point. Starting it anywhere other than the
    // reference score would make every later delta exact and every later total wrong.
    let rules = rules();
    for found in all_fixtures(2026) {
        for (n, problem) in found.problems.iter().enumerate() {
            let assignment = crowded(problem, 13);
            let scorer = IncrementalScorer::of(problem, &assignment, &rules)
                .expect("a readable arrangement");
            let reference = evaluate(problem, &assignment, &rules);
            assert_eq!(
                scorer.score(),
                reference.score,
                "{} problem {n}: a fresh scorer disagreed with the reference",
                found.name
            );
        }
    }
}

#[test]
fn a_scorer_refuses_an_arrangement_it_cannot_read() {
    // Maintaining a number about activities the problem does not hold would be worse than
    // silence, because it looks like an answer — `EvaluationReport::is_readable`'s argument.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();

    let mut wrong = Assignment::empty(problem.activities.len() + 3);
    wrong.place(
        ActivityIx::new(u32::try_from(problem.activities.len()).unwrap_or(0)),
        Placement::unroomed(TimeslotIx::new(0)),
    );
    assert!(IncrementalScorer::of(problem, &wrong, &rules).is_none());
}

#[test]
fn the_empty_arrangement_scores_one_medium_violation_per_occurrence() {
    // The starting point of every construction solve, and the reason `activity-scheduled` is
    // MEDIUM: the empty arrangement is usable-so-far and maximally incomplete.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let scorer = IncrementalScorer::empty(problem, &rules);

    assert!(scorer.is_usable(), "nothing placed breaks no hard rule");
    assert_eq!(
        scorer.score(),
        Score::new(
            0,
            -i64::try_from(problem.activities.len()).expect("a small school"),
            0
        )
    );
    assert_eq!(
        u64::from(scorer.count_of(ConstraintType::ActivityScheduled)),
        problem.activities.len() as u64
    );
}
