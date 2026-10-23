//! What the constructor places, what it refuses, and what it says it could not do.
//!
//! # The assertion that matters most
//!
//! [`no_fixture_is_generated_with_a_hard_violation`] runs the whole M2 catalogue through
//! generation and then re-evaluates every result with M5's **reference** scorer. A generated
//! timetable with a hard violation in it is the one failure a school cannot detect for itself,
//! so it is checked over every fixture, at several seeds, rather than on one happy path.
//!
//! Note what it does *not* assert: that every fixture is fully schedulable. Only `tiny` ships a
//! known-feasible arrangement, so only `tiny` is held to completeness — a test that demanded a
//! complete timetable from `infeasible-duration` would be asserting a falsehood, and one that
//! demanded it from the rest would be asserting something no record claims.
//!
//! # Built from fixtures, not from `Problem` literals
//!
//! `evara_core::scheduling::fixtures::builder` is public behind the `fixtures` feature, so a
//! test declares a school in a dozen lines and gets real indices, masks and a materialised grid.
//! A hand-built `Problem` would be a test that passes because its hand-built facts agree with
//! its hand-built expectations.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::fixtures::{FixtureKind, fixture};
use evara_core::scheduling::{
    ActivityIx, Assignment, AvailabilityEntity, AvailabilityStatus, BellScheduleResolution,
    CompiledConstraint, ConstraintLevel, ConstraintType, CycleKind, DiagnosticReport, GroupKind,
    Overlap, PeriodKind, Problem, ProblemIdentity, SolveOutcome, SolveStatus, Termination,
    UnplacedCause,
};
use uuid::Uuid;

use crate::{
    Cancel, ConstructionSolver, GenerateRequest, NeverCancel, NoProgress, SolveConfig, SolveLimits,
    SolverEngine, generate, generate_with, meets_completeness_requirement,
};

// =======================================================================================
// Scaffolding
// =======================================================================================

/// The mandatory set — what every project evaluates.
fn rules() -> Vec<CompiledConstraint> {
    Registry::mandatory()
}

/// A request over one problem, with nothing refused and nothing undisclosed.
fn request<'a>(problem: &'a Problem, constraints: &'a [CompiledConstraint]) -> GenerateRequest<'a> {
    GenerateRequest {
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
    }
}

/// Generates over one problem at one seed.
fn solve_at(problem: &Problem, seed: u64) -> SolveOutcome {
    let rules = rules();
    let mut asked = request(problem, &rules);
    asked.config = SolveConfig::seeded(seed);
    generate(&asked)
}

/// Four teaching periods with a break between the second and third.
fn standard_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("1", "09:00", "10:00"),
        PeriodSpec::teaching("2", "10:00", "11:00"),
        PeriodSpec::non_teaching("Break", "11:00", "11:30", PeriodKind::Break),
        PeriodSpec::teaching("3", "11:30", "12:30"),
        PeriodSpec::teaching("4", "13:30", "14:30"),
    ]
}

/// A school with one teacher, one room, one group and `durations.len()` occurrences.
///
/// The smallest thing that can demonstrate a clash: one teacher and one room means every pair
/// of occurrences competes for both.
fn one_of_everything(durations: &[u16]) -> Problem {
    let mut school = SchoolBuilder::new(5, "Northgate", "Standard week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1", "Day 2"], 2);
    let schedule = school.add_bell_schedule("Standard Day", Some(campus), &standard_day());
    school.materialise(schedule);

    let room = school.add_room(RoomFactSpec::new("Room 1", 30).on_campus(campus));
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
    let _ = room;
    school.finish()
}

/// A cancellation that fires after `after` questions.
///
/// Deterministic: the engine asks once per construction step, so the stop happens at a fixed
/// step rather than at a fixed time.
struct CancelAfter {
    after: std::cell::Cell<u32>,
}

impl CancelAfter {
    fn new(after: u32) -> Self {
        Self {
            after: std::cell::Cell::new(after),
        }
    }
}

impl Cancel for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let left = self.after.get();
        if left == 0 {
            return true;
        }
        self.after.set(left - 1);
        false
    }
}

// =======================================================================================
// The headline: nothing generated is ever hard-invalid
// =======================================================================================

#[test]
fn no_fixture_is_generated_with_a_hard_violation() {
    // The whole catalogue, several seeds, every result re-evaluated by the reference scorer.
    // `generate` already performs this check internally and reports a failure as an internal
    // defect; this asserts it from outside, so a bug in the internal check cannot hide a bug in
    // the construction.
    let rules = rules();
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        for (n, problem) in found.problems.iter().enumerate() {
            for seed in [1_u64, 2026, 99_991] {
                let outcome = solve_at(problem, seed);

                let report = evaluate(problem, &outcome.assignment, &rules);
                assert!(
                    report.is_readable(),
                    "{} problem {n} seed {seed}: the result does not describe its problem",
                    found.name
                );
                assert_eq!(
                    report.count_at(ConstraintLevel::Hard),
                    0,
                    "{} problem {n} seed {seed}: a generated timetable breaks a hard rule — {:?}",
                    found.name,
                    report
                        .violations
                        .iter()
                        .filter(|v| v.level == ConstraintLevel::Hard)
                        .map(|v| (v.constraint_type.key(), v.detail.clone()))
                        .collect::<Vec<_>>()
                );
                assert_ne!(
                    outcome.termination,
                    Termination::InternalCheckFailed,
                    "{} problem {n} seed {seed}: the engine's own check failed",
                    found.name
                );
                assert_ne!(outcome.status, SolveStatus::Failed);
            }
        }
    }
}

#[test]
fn the_tiny_fixture_is_scheduled_completely() {
    // The one fixture that ships a known-feasible arrangement, so the one fixture whose
    // completeness is a claim somebody made rather than a hope. The constructor does not have
    // to find *that* arrangement — only a feasible complete one.
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    let problem = found.problem();
    let outcome = solve_at(problem, 2026);

    assert_eq!(outcome.status, SolveStatus::Complete, "{outcome:#?}");
    assert_eq!(outcome.termination, Termination::Complete);
    assert!(outcome.is_complete());
    assert!(outcome.is_adoptable());
    assert_eq!(outcome.unscheduled.len(), 0, "nothing is left to place");
    assert_eq!(outcome.placed_count(), problem.activities.len());
    assert!(outcome.score.is_feasible());
    assert_eq!(outcome.score.medium, 0, "nothing left unplaced");
}

#[test]
fn a_known_feasible_fixture_is_scheduled_completely_at_every_seed() {
    // A seed changes which timetable is found and must not change whether one is found, for a
    // problem this small. A failure here is the construction stranding itself, which is the
    // thing a seeded candidate order exists to avoid.
    let found = fixture("tiny", 2026).expect("tiny is in the catalogue");
    for seed in [0_u64, 1, 2, 3, 7, 42, 2026, 123_456] {
        let outcome = solve_at(found.problem(), seed);
        assert_eq!(
            outcome.status,
            SolveStatus::Complete,
            "seed {seed} left {} unplaced",
            outcome.unscheduled.len()
        );
    }
}

// =======================================================================================
// Candidate generation
// =======================================================================================

#[test]
fn a_single_period_occurrence_is_placed_in_a_teaching_period() {
    let problem = one_of_everything(&[1]);
    let outcome = solve_at(&problem, 2026);

    assert_eq!(outcome.status, SolveStatus::Complete);
    let placement = outcome
        .assignment
        .placement(ActivityIx::new(0))
        .expect("it was placed");
    assert!(
        problem.timeslots[placement.timeslot.get()].is_teaching,
        "the generator must never offer a non-teaching slot"
    );
}

#[test]
fn a_multi_period_occurrence_lands_on_a_run_and_never_across_the_break() {
    // The day is 1, 2, Break, 3, 4 — so the only legal doubles are (1,2) and (3,4), and a
    // placement starting at period 2 would reach into the break.
    let problem = one_of_everything(&[2]);
    for seed in 0..12_u64 {
        let outcome = solve_at(&problem, seed);
        assert_eq!(outcome.status, SolveStatus::Complete, "seed {seed}");
        // `duration-contiguous` is what judges this, and the reference scorer is what reports
        // it — so the assertion is the evaluator's answer rather than a second reading of the
        // grid here.
        let report = evaluate(&problem, &outcome.assignment, &rules());
        assert!(
            !report.has(ConstraintType::DurationContiguous),
            "seed {seed}: a double period was placed across a break or a day boundary"
        );
    }
}

#[test]
fn an_occurrence_longer_than_any_run_is_impossible_and_says_so() {
    // The day's longest run of teaching periods is two, so a triple cannot be placed by any
    // timetable. Arithmetic, and the one claim of impossibility the solver is entitled to make.
    let problem = one_of_everything(&[3]);
    let outcome = solve_at(&problem, 2026);

    assert_eq!(outcome.status, SolveStatus::Partial);
    assert_eq!(outcome.unscheduled.len(), 1);
    let unplaced = &outcome.unscheduled[0];
    assert_eq!(unplaced.cause, UnplacedCause::NoContiguousRun);
    assert!(unplaced.cause.is_impossible());
    assert!(
        unplaced.reason.contains("no timetable can place it"),
        "{}",
        unplaced.reason
    );
    assert_eq!(outcome.impossible().count(), 1);
}

#[test]
fn a_required_room_type_with_no_room_of_that_type_is_impossible() {
    let mut school = SchoolBuilder::new(5, "Northgate", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    school.materialise(schedule);

    let laboratory = school.add_room_type("Science Laboratory");
    school.add_room(RoomFactSpec::new("Room 1", 30).on_campus(campus));
    let teacher = school.add_teacher(TeacherFactSpec::new("A. Teacher"));
    let group = school.add_group(GroupSpec::new("7A", GroupKind::Class).sized(26));
    school.declare_overlap(group, group, Overlap::Overlaps);
    let section = school.add_section(
        SectionSpec::new("Science 7A")
            .schedule(BellScheduleResolution::Resolved { schedule })
            .taught_by(teacher)
            .taught_to(group)
            .requires_room_type(laboratory),
    );
    school.add_activities(section, &[1]);
    let problem = school.finish();

    let outcome = solve_at(&problem, 2026);
    assert_eq!(outcome.unscheduled.len(), 1);
    assert_eq!(outcome.unscheduled[0].cause, UnplacedCause::NoCandidateRoom);
    assert!(outcome.unscheduled[0].cause.is_impossible());
}

#[test]
fn an_occurrence_needing_no_room_is_placed_whether_or_not_it_gets_one() {
    // A section with no required type may be timetabled without a room, and the roomless
    // candidate is the fallback that stops a full school from leaving lessons unplaced rather
    // than unroomed. Here there is one room and three occurrences of one teacher, so at most
    // one can hold the room at a time — but all three are placed, on different days or periods.
    let problem = one_of_everything(&[1, 1, 1]);
    let outcome = solve_at(&problem, 2026);
    assert_eq!(outcome.status, SolveStatus::Complete, "{outcome:#?}");
}

#[test]
fn an_unavailable_teacher_empties_the_value_range_and_is_reported_as_availability() {
    // One teacher, one group, one day, and the teacher is unavailable in every teaching slot.
    // Nothing else is placed, so this cannot be a clash: the cause must be availability.
    let mut school = SchoolBuilder::new(5, "Northgate", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
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
    school.add_activities(section, &[1]);
    for slot in &slots {
        school.restrict(
            AvailabilityEntity::Teacher { teacher },
            *slot,
            AvailabilityStatus::Unavailable,
        );
    }
    let problem = school.finish();

    let outcome = solve_at(&problem, 2026);
    assert_eq!(outcome.unscheduled.len(), 1);
    assert_eq!(
        outcome.unscheduled[0].cause,
        UnplacedCause::AllCandidatesUnavailable
    );
    assert!(
        outcome.unscheduled[0].reason.contains("marks unavailable"),
        "{}",
        outcome.unscheduled[0].reason
    );
}

#[test]
fn a_shared_room_across_two_bell_schedules_is_never_double_booked() {
    // The fixture where interval comparison and timeslot equality disagree: two campuses on
    // different timings sharing a hall and a teacher. A generator that compared slot
    // identifiers would double-book the hall and report nothing.
    let found = fixture("two-campuses-different-bells", 2026).expect("in the catalogue");
    for seed in [1_u64, 2026, 7] {
        let outcome = solve_at(found.problem(), seed);
        let report = evaluate(found.problem(), &outcome.assignment, &rules());
        assert!(
            !report.has(ConstraintType::RoomClash),
            "seed {seed}: a room was double-booked across two bell schedules"
        );
        assert!(!report.has(ConstraintType::TeacherClash));
    }
}

// =======================================================================================
// Construction
// =======================================================================================

#[test]
fn a_problem_with_no_activities_is_complete_immediately() {
    let mut school = SchoolBuilder::new(5, "Northgate", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    school.materialise(schedule);
    let problem = school.finish();

    let outcome = solve_at(&problem, 2026);
    assert_eq!(outcome.status, SolveStatus::Complete);
    assert_eq!(outcome.termination, Termination::Complete);
    assert!(outcome.is_complete());
    assert_eq!(outcome.statistics.candidate_evaluations, 0);
    assert_eq!(
        outcome.statistics.passes, 0,
        "there was nothing to sweep over"
    );
}

#[test]
fn a_teacher_cannot_be_given_two_lessons_at_once_however_many_there_are() {
    // One teacher, two days, four teaching periods each: eight slots, and the ninth occurrence
    // has nowhere to go. Every placed one must still be clash-free.
    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let outcome = solve_at(&problem, 2026);

    let report = evaluate(&problem, &outcome.assignment, &rules());
    assert!(!report.has(ConstraintType::TeacherClash));
    assert_eq!(outcome.placed_count(), 8, "{outcome:#?}");
    assert_eq!(outcome.unscheduled.len(), 1);
    assert_eq!(
        outcome.unscheduled[0].cause,
        UnplacedCause::BlockedByPlacements,
        "a slot ran out, which is about this arrangement and not about the problem"
    );
    assert!(
        !outcome.unscheduled[0].cause.is_impossible(),
        "the constructor must not claim a demand is impossible because it ran out of room"
    );
    assert!(
        outcome.unscheduled[0]
            .reason
            .contains("not a proof that no complete timetable exists"),
        "{}",
        outcome.unscheduled[0].reason
    );
}

#[test]
fn two_sections_sharing_students_are_never_placed_together() {
    let found = fixture("overlapping-group-membership", 2026).expect("in the catalogue");
    for seed in [1_u64, 2026, 5] {
        let outcome = solve_at(found.problem(), seed);
        let report = evaluate(found.problem(), &outcome.assignment, &rules());
        assert!(
            !report.has(ConstraintType::StudentClash),
            "seed {seed}: two sections with a shared member ran at once"
        );
    }
}

#[test]
fn an_indeterminate_pair_is_allowed_to_be_placed_together() {
    // ADR 0035 §6: enforce what is proved. A school that has answered nothing about its groups
    // must still get a timetable, which is the property group-first scheduling exists for — and
    // the hole is disclosed rather than closed.
    let found = fixture("unknown-group-overlap", 2026).expect("in the catalogue");
    let outcome = solve_at(found.problem(), 2026);

    assert!(outcome.status.is_usable(), "{outcome:#?}");
    assert!(
        outcome.unverified.indeterminate_student_pairs > 0,
        "the undecided pairs must be disclosed, not silently treated as disjoint"
    );
    assert!(outcome.unverified.has_anything_to_disclose());
}

#[test]
fn a_partial_result_is_returned_rather_than_nothing() {
    // The whole point of `activity-scheduled` being MEDIUM. Nine occurrences into eight slots:
    // eight lessons are a usable timetable and the ninth is a thing to fix.
    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let outcome = solve_at(&problem, 2026);

    assert_eq!(outcome.status, SolveStatus::Partial);
    assert!(outcome.status.is_usable(), "partial is a usable result");
    assert!(
        outcome.is_adoptable(),
        "nothing hard is broken, so the eight placed lessons can be adopted"
    );
    assert!(!outcome.is_complete());
    assert_eq!(outcome.termination, Termination::Exhausted);
    assert!(
        !outcome.termination.was_cut_short(),
        "it ran out of moves, not of budget"
    );
}

#[test]
fn an_unsatisfiable_fixture_still_returns_what_it_could_place() {
    for name in ["infeasible-duration", "infeasible-room-type"] {
        let found = fixture(name, 2026).unwrap_or_else(|| panic!("{name} is in the catalogue"));
        let outcome = solve_at(found.problem(), 2026);

        assert!(
            outcome.status.is_usable(),
            "{name}: an impossible occurrence must not fail the whole run — {outcome:#?}"
        );
        assert!(
            !outcome.unscheduled.is_empty(),
            "{name}: the impossible occurrence must be reported"
        );
        assert!(
            outcome.impossible().count() > 0,
            "{name}: and reported as impossible rather than as merely difficult"
        );
    }
}

#[test]
fn an_empty_grid_places_nothing_and_blames_the_grid() {
    // No slots at all. Every occurrence resolves to no bell schedule, which ADR 0033 §3 makes a
    // hard pre-flight error — so the honest cause is about the setup and not about the search.
    let found = fixture("empty-grid", 2026).expect("in the catalogue");
    let outcome = solve_at(found.problem(), 2026);

    assert_eq!(outcome.placed_count(), 0);
    for unplaced in &outcome.unscheduled {
        assert_eq!(unplaced.cause, UnplacedCause::NoResolvedBellSchedule);
        assert!(unplaced.cause.is_impossible());
    }
}

// =======================================================================================
// Determinism and seeds
// =======================================================================================

#[test]
fn the_same_problem_seed_and_configuration_reproduce_the_same_outcome() {
    // `SOLVER.md` §5 calls this non-negotiable. Whole-outcome equality, which is only possible
    // because `SolveStatistics` carries counters and no clock.
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        for problem in &found.problems {
            let first = solve_at(problem, 777);
            let again = solve_at(problem, 777);
            assert_eq!(
                first, again,
                "{}: two identical runs produced different results",
                found.name
            );
        }
    }
}

#[test]
fn a_different_seed_may_produce_a_different_timetable_and_is_not_required_to() {
    // Both halves are the contract. Over a school with real choice, at least one seed pair must
    // differ — otherwise the seed is decorative — but no individual pair is required to.
    let problem = one_of_everything(&[1, 1, 1, 1]);
    let baseline = solve_at(&problem, 1).assignment;
    let differs = (2..40_u64).any(|seed| solve_at(&problem, seed).assignment != baseline);
    assert!(
        differs,
        "no seed changed the timetable, so the seed is not reaching the decisions"
    );
}

#[test]
fn the_seed_is_recorded_on_the_result() {
    // A run whose seed was not recorded cannot be reproduced, which makes a regression
    // undebuggable and a baseline meaningless — `Seed`'s own documentation.
    let found = fixture("tiny", 2026).expect("in the catalogue");
    for seed in [0_u64, 7, u64::MAX] {
        assert_eq!(solve_at(found.problem(), seed).seed.get(), seed);
    }
}

#[test]
fn the_constrainedness_order_breaks_ties_on_activity_index() {
    // Four identical occurrences of one section: same duration, same value range, same conflict
    // degree. Only the index separates them, and without it the order would depend on whatever
    // order the loader happened to produce.
    let problem = one_of_everything(&[1, 1, 1, 1]);
    let first = solve_at(&problem, 11);
    for _ in 0..5 {
        assert_eq!(solve_at(&problem, 11), first);
    }
}

// =======================================================================================
// Execution limits
// =======================================================================================

#[test]
fn a_candidate_limit_stops_the_run_and_says_so() {
    let rules = rules();
    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1]);
    let mut asked = request(&problem, &rules);
    asked.config = SolveConfig::seeded(2026).with_limits(SolveLimits::at_most(2));

    let outcome = generate(&asked);
    assert_eq!(outcome.termination, Termination::CandidateLimitReached);
    assert!(outcome.termination.was_cut_short());
    assert!(outcome.statistics.candidate_evaluations <= 2);
    assert!(
        outcome.status.is_usable(),
        "whatever was placed is still worth returning"
    );
    // Nothing is claimed about the occurrences the run never reached.
    assert!(
        outcome
            .unscheduled
            .iter()
            .any(|u| u.cause == UnplacedCause::NotReached),
        "{outcome:#?}"
    );
    assert!(
        outcome
            .unscheduled
            .iter()
            .all(|u| u.cause != UnplacedCause::BlockedByPlacements || !u.cause.is_impossible())
    );
}

#[test]
fn a_cancelled_run_returns_its_work_and_is_not_a_failure() {
    let rules = rules();
    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1]);
    let asked = request(&problem, &rules);

    let outcome = generate_with(
        &ConstructionSolver,
        &asked,
        &NoProgress,
        &CancelAfter::new(2),
    );
    assert_eq!(outcome.status, SolveStatus::Cancelled);
    assert_eq!(outcome.termination, Termination::Cancelled);
    assert!(
        !outcome.termination.is_reproducible(),
        "a cancellation is the one termination a clock can cause, so it is not replayable"
    );
    assert_ne!(
        outcome.status,
        SolveStatus::Failed,
        "cancelled is not failed"
    );
    assert_eq!(outcome.placed_count(), 2, "the work it did is returned");
    assert_eq!(
        outcome.placed_count() + outcome.unscheduled.len(),
        problem.activities.len(),
        "and everything it did not place is reported: {outcome:#?}"
    );
}

#[test]
fn a_limit_is_reported_rather_than_silently_truncating() {
    // `CLAUDE.md`: "No silent caps: if a workflow bounds coverage, log what was dropped."
    // Reaching a limit must be distinguishable from finishing, which is what `Termination` is.
    let rules = rules();
    let problem = one_of_everything(&[1, 1, 1, 1]);
    let mut asked = request(&problem, &rules);
    asked.config = SolveConfig::seeded(3).with_limits(SolveLimits::at_most(1));
    let cut_short = generate(&asked);

    asked.config = SolveConfig::seeded(3);
    let complete = generate(&asked);

    assert_ne!(cut_short.termination, complete.termination);
    assert!(cut_short.termination.was_cut_short());
    assert!(!complete.termination.was_cut_short());
}

// =======================================================================================
// The gate, the disclosure and the verification
// =======================================================================================

#[test]
fn a_refused_problem_is_not_solved_at_all() {
    // `may_solve` of false means the inputs have no value range to search over. Searching anyway
    // would report an unanswered setup question as a scheduling difficulty.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let mut asked = request(found.problem(), &rules);
    asked.may_solve = false;

    let outcome = generate(&asked);
    assert_eq!(outcome.status, SolveStatus::Failed);
    assert_eq!(outcome.termination, Termination::InputRejected);
    assert_eq!(outcome.placed_count(), 0);
    assert_eq!(outcome.statistics.candidate_evaluations, 0);
    assert_eq!(
        outcome.unscheduled.len(),
        found.problem().activities.len(),
        "every occurrence is reported, and none of them as impossible"
    );
    assert_eq!(
        outcome.impossible().count(),
        0,
        "nothing was attempted, so nothing was established about any lesson"
    );
    assert!(
        outcome
            .unscheduled
            .iter()
            .all(|u| u.cause == UnplacedCause::InputRejected),
        "a refusal is its own cause: the remedy is to correct the input, not to retry with a          bigger budget, and `NotReached` says the second"
    );
}

#[test]
fn a_warning_never_blocks_a_solve() {
    // Indeterminate pairs, unknown attendance and shared entities are warnings, and
    // SCHEDULING-ARCHITECTURE-DECISIONS.md §3.2 reserves a refusal for an input whose remedy is
    // a field the user can fill in. Each of these fixtures carries warnings and must solve.
    for name in [
        "unknown-group-overlap",
        "unknown-attendance",
        "shared-teacher-across-schools",
        "roster-without-group",
    ] {
        let found = fixture(name, 2026).unwrap_or_else(|| panic!("{name} is in the catalogue"));
        let outcome = solve_at(found.problem(), 2026);
        assert!(
            outcome.status.is_usable(),
            "{name}: a warning blocked a solve — {:?}",
            outcome.termination
        );
    }
}

#[test]
fn preflight_findings_are_carried_onto_the_result() {
    use evara_core::scheduling::{Diagnostic, DiagnosticCode, EntityKind, EntityRef};

    let rules = rules();
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let mut preflight = DiagnosticReport::empty();
    preflight.push(Diagnostic::new(
        DiagnosticCode::ExpectedAttendanceUnknown,
        EntityRef::new(EntityKind::ClassSection, Uuid::from_u128(1), "A section"),
        "attendance is unknown",
    ));

    let mut asked = request(found.problem(), &rules);
    asked.preflight = preflight.clone();
    let outcome = generate(&asked);

    assert_eq!(
        outcome.preflight, preflight,
        "a timetable's provenance must state the limits it was generated under"
    );
}

#[test]
fn the_unverified_counts_disclose_every_hole_the_problem_has() {
    // ADR 0032 §5 rule 4: a timetable is never described as conflict-free. Every figure is a
    // real hole three accepted records chose to make visible rather than to close.
    let rules = rules();
    let found = fixture("shared-teacher-across-schools", 2026).expect("in the catalogue");
    let mut asked = request(found.problem(), &rules);
    asked.unenforced_restrictions = 4;

    let outcome = generate(&asked);
    assert!(outcome.unverified.shared_entities > 0, "{outcome:#?}");
    assert_eq!(
        outcome.unverified.unenforced_restrictions, 4,
        "the loader's count must reach the result"
    );
    assert!(outcome.unverified.has_anything_to_disclose());
}

#[test]
fn unchecked_capacities_are_counted_per_occurrence() {
    let found = fixture("unknown-attendance", 2026).expect("in the catalogue");
    let outcome = solve_at(found.problem(), 2026);
    assert!(
        outcome.unverified.unchecked_capacities > 0,
        "room-capacity was not evaluated, which is a third outcome and not zero violations"
    );
}

#[test]
fn the_hard_violation_list_and_the_score_agree() {
    // `SolveOutcome` documents that these two must agree. Over the whole catalogue, both say
    // "nothing broken", which is the only state this engine can produce.
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        for problem in &found.problems {
            let outcome = solve_at(problem, 2026);
            assert_eq!(
                outcome.hard_violations.is_empty(),
                outcome.score.is_feasible(),
                "{}: the violation list and the score disagree",
                found.name
            );
        }
    }
}

// =======================================================================================
// A raised `activity-scheduled`
// =======================================================================================

#[test]
fn raising_activity_scheduled_to_hard_keeps_the_partial_work_and_says_it_is_short() {
    // ADR 0039 lets a school raise a level. A partial result then fails the school's own
    // completeness requirement — and is still returned, because discarding it would leave a
    // school that raised the level strictly worse off than one that did not.
    let mut raised = rules();
    for rule in &mut raised {
        if rule.constraint_type == ConstraintType::ActivityScheduled {
            rule.level = ConstraintLevel::Hard;
        }
    }

    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let asked = request(&problem, &raised);
    let outcome = generate(&asked);

    assert_eq!(outcome.placed_count(), 8, "the work is kept");
    assert!(!meets_completeness_requirement(&outcome, &raised));
    assert!(
        !outcome.is_adoptable(),
        "at HARD, an unplaced lesson makes the result infeasible rather than incomplete"
    );
    assert_eq!(
        outcome.status,
        SolveStatus::Failed,
        "the reference scorer finds a hard violation, which is what raising the level means"
    );

    // The same run against the shipped level is adoptable, which is the comparison that shows
    // the level and not the construction decided it.
    let shipped = rules();
    let asked = request(&problem, &shipped);
    let relaxed = generate(&asked);
    assert!(relaxed.is_adoptable());
    assert!(meets_completeness_requirement(&relaxed, &shipped));
    assert_eq!(
        relaxed.placed_count(),
        outcome.placed_count(),
        "the level changed the verdict, not the timetable"
    );
}

#[test]
fn completeness_is_satisfied_at_the_shipped_level_even_when_partial() {
    let shipped = rules();
    let problem = one_of_everything(&[1, 1, 1, 1, 1, 1, 1, 1, 1]);
    let outcome = generate(&request(&problem, &shipped));
    assert!(!outcome.is_complete());
    assert!(
        meets_completeness_requirement(&outcome, &shipped),
        "at MEDIUM a partial timetable is an ordinary state, not a failed requirement"
    );
}

// =======================================================================================
// The engine trait
// =======================================================================================

#[test]
fn the_engine_can_be_driven_without_the_orchestration() {
    // `SolverEngine` is the reversibility seam `SOLVER.md` §9 asks for, so it has to be usable
    // on its own — and a caller that does must not get a verified result by accident.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let problem = found.problem();

    let solution = ConstructionSolver.solve(
        problem,
        None,
        &rules,
        &SolveConfig::seeded(2026),
        &NoProgress,
        &NeverCancel,
    );
    assert_eq!(solution.termination, Termination::Complete);
    assert_eq!(solution.unplaced.len(), 0, "nothing is left to place");
    assert!(solution.assignment.is_complete());
    assert!(solution.statistics.candidate_evaluations > 0);
}

#[test]
fn a_starting_arrangement_is_accepted_and_does_not_break_the_run() {
    // Repair is M-later, and the parameter exists now. Passing one must not make the result
    // worse or invalid, which is the property that keeps the seam honest until repair arrives.
    let rules = rules();
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let problem = found.problem();
    let start = found
        .expectations
        .feasible
        .as_ref()
        .expect("tiny ships one")
        .assignment(problem.activities.len());

    let solution = ConstructionSolver.solve(
        problem,
        Some(&start),
        &rules,
        &SolveConfig::seeded(2026),
        &NoProgress,
        &NeverCancel,
    );
    let report = evaluate(problem, &solution.assignment, &rules);
    assert!(report.is_usable());
    assert!(solution.assignment.is_complete());
}

#[test]
fn progress_is_reported_once_per_attempt() {
    use std::cell::RefCell;

    struct Recorder {
        seen: RefCell<Vec<(usize, usize)>>,
    }
    impl crate::ProgressSink for Recorder {
        fn placed(&self, placed: usize, total: usize, _steps: u64) {
            self.seen.borrow_mut().push((placed, total));
        }
    }

    let rules = rules();
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let recorder = Recorder {
        seen: RefCell::new(Vec::new()),
    };
    let outcome = generate_with(
        &ConstructionSolver,
        &request(found.problem(), &rules),
        &recorder,
        &NeverCancel,
    );

    let seen = recorder.seen.borrow();
    assert_eq!(
        seen.len(),
        found.problem().activities.len(),
        "one report per attempt"
    );
    assert_eq!(seen.last().copied(), Some((outcome.placed_count(), 10)));
    assert!(
        seen.windows(2).all(|w| w[0].0 <= w[1].0),
        "the placed count must never go backwards during construction"
    );
}

// =======================================================================================
// Scale
// =======================================================================================

#[test]
fn the_medium_fixture_schedules_at_the_documented_scale() {
    // 80 teachers, 1 000 students at group level, 500 occurrences — the realistic synthetic
    // school. The assertion is correctness at scale, not speed: every placement legal, and the
    // run finishing rather than hitting a limit. Figures are in
    // docs/TIMETABLE-CONSTRUCTION-SOLVER.md §14.
    let rules = rules();
    let found = fixture("medium", 2026).expect("medium is in the catalogue");
    let problem = found.problem();
    assert_eq!(problem.activities.len(), 500, "the documented scale");

    let outcome = solve_at(problem, 2026);
    let report = evaluate(problem, &outcome.assignment, &rules);
    assert_eq!(
        report.count_at(ConstraintLevel::Hard),
        0,
        "a 500-occurrence timetable with a hard violation in it"
    );
    assert!(
        !outcome.termination.was_cut_short(),
        "the shipped limits must not be what stops a realistic school: {:?}, {:?}",
        outcome.termination,
        outcome.statistics
    );
    assert!(
        outcome.placed_count() > 0,
        "nothing was placed at all: {outcome:#?}"
    );
}

/// A synthetic secondary school of roughly `sections` sections, each with four single periods.
///
/// Built here rather than added to M2's catalogue. `OPEN-DECISIONS.md` R5 names *"a
/// 2,000-activity secondary school with electives"* as the realistic upper end and asks for its
/// numbers to be treated as a budget; the catalogue's largest fixture is `medium` at 500, and
/// widening a documented 24-fixture catalogue is M2's surface rather than M6's. So the upper-end
/// measurement lives with the measurement.
///
/// Shaped so the problem is genuinely hard rather than merely large: teachers and rooms are
/// shared across many sections, so most candidates clash and the constraint engine does real
/// work per placement.
fn a_large_school(sections: usize) -> Problem {
    let days = 5;
    let periods_per_day = 8;
    let mut school =
        SchoolBuilder::new(99, "Largefield Academy", "Standard week", CycleKind::Weekly);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Mon", "Tue", "Wed", "Thu", "Fri"], days);

    let mut day_periods = Vec::new();
    for n in 0..periods_per_day {
        let start = 8 + n;
        day_periods.push(PeriodSpec::teaching(
            &format!("P{}", n + 1),
            &format!("{start:02}:00"),
            &format!("{:02}:00", start + 1),
        ));
    }
    let schedule = school.add_bell_schedule("Standard Day", Some(campus), &day_periods);
    school.materialise(schedule);

    // One room per concurrent lesson the grid could hold, so rooms are scarce but not the only
    // bottleneck.
    let rooms: Vec<_> = (0..40)
        .map(|n| school.add_room(RoomFactSpec::new(&format!("Room {n}"), 30).on_campus(campus)))
        .collect();
    let teachers: Vec<_> = (0..80)
        .map(|n| school.add_teacher(TeacherFactSpec::new(&format!("Teacher {n}"))))
        .collect();
    let groups: Vec<_> = (0..60)
        .map(|n| {
            school.add_group(GroupSpec::new(&format!("Class {n}"), GroupKind::Class).sized(28))
        })
        .collect();
    for group in &groups {
        school.declare_overlap(*group, *group, Overlap::Overlaps);
    }
    // Distinct classes are declared disjoint, so `student-clash` has real work and real
    // permission: an all-indeterminate school would let everything run at once.
    for (at, a) in groups.iter().enumerate() {
        for b in groups.iter().skip(at + 1) {
            school.declare_overlap(*a, *b, Overlap::Disjoint);
        }
    }

    for n in 0..sections {
        let section = school.add_section(
            SectionSpec::new(&format!("Course {n}"))
                .schedule(BellScheduleResolution::Resolved { schedule })
                .taught_by(teachers[n % teachers.len()])
                .taught_to(groups[n % groups.len()]),
        );
        school.add_activities(section, &[1, 1, 1, 1]);
    }
    let _ = rooms;
    school.finish()
}

#[test]
fn the_realistic_upper_end_schedules_without_hitting_a_limit() {
    // R5's budget: a 2 000-occurrence secondary school. The assertion is correctness and that
    // the shipped limits are not what stops it; the figures are recorded in
    // docs/TIMETABLE-CONSTRUCTION-SOLVER.md §14 and printed here under `--nocapture`.
    let rules = rules();
    let problem = a_large_school(500);
    assert_eq!(problem.activities.len(), 2_000, "the documented upper end");

    let outcome = solve_at(&problem, 2026);
    let report = evaluate(&problem, &outcome.assignment, &rules);

    eprintln!(
        "upper end: {} of {} placed, {} candidate evaluations, {} rejected, {} passes, \
         termination {:?}",
        outcome.placed_count(),
        problem.activities.len(),
        outcome.statistics.candidate_evaluations,
        outcome.statistics.candidates_rejected,
        outcome.statistics.passes,
        outcome.termination
    );

    assert_eq!(
        report.count_at(ConstraintLevel::Hard),
        0,
        "a 2 000-occurrence timetable with a hard violation in it"
    );
    assert!(
        !outcome.termination.was_cut_short(),
        "the shipped limits must not be the reason a realistic school fails: {:?} after {:?}",
        outcome.termination,
        outcome.statistics
    );
    assert!(
        outcome.placed_count() * 100 / problem.activities.len() >= 90,
        "only {} of {} placed, which is below the 90% this school's shape should reach",
        outcome.placed_count(),
        problem.activities.len()
    );
}

#[test]
fn a_crowded_assignment_round_trips_through_the_result() {
    // `SOLVER.md` §9 requires a JSON-serialisable `Problem`/`Assignment` pair, which is what
    // makes the headless CLI and a committed baseline possible.
    let found = fixture("tiny", 2026).expect("in the catalogue");
    let outcome = solve_at(found.problem(), 2026);

    let json = serde_json::to_string(&outcome.assignment).expect("an assignment serialises");
    let back: Assignment = serde_json::from_str(&json).expect("and parses back");
    assert_eq!(back, outcome.assignment);
}
