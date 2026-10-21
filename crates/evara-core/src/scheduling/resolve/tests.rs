//! Tests for the one bell-schedule resolver.
//!
//! The compatibility relation is tested directly, as a matrix over the three clauses. The
//! resolution is tested both on hand-built contexts and over every fixture in the catalogue,
//! because the catalogue is where a real configuration appears.

use uuid::Uuid;

use super::{
    MaterialisedGrid, is_compatible, longest_teaching_run, only_campus, runs_on, teaching_slots,
};
use crate::scheduling::context::{
    BellScheduleFact, BellScheduleResolution, CampusFact, CycleDayFact, CycleFact, CycleKind,
    NoBellScheduleCause, SchedulingContext, SchoolFact,
};
use crate::scheduling::fixtures::{FixtureKind, all_fixtures};
use crate::scheduling::index::{BellScheduleIx, CampusIx, CycleDayIx};

/// Seeds the fixture-wide properties are checked at.
const SEEDS: [u64; 3] = [0, 7, 2026];

/// A single seed for the tests that need only one.
const SEED: u64 = 2026;

/// A context with `campuses` campuses and the given schedules, and no periods or days.
///
/// Enough to exercise the compatibility relation, which reads only campuses.
fn context(campuses: usize, schedules: &[Option<CampusIx>]) -> SchedulingContext {
    SchedulingContext {
        school: SchoolFact {
            id: Uuid::from_u128(1),
            name: "Northgate".into(),
            timezone: "Europe/London".into(),
        },
        cycle: CycleFact {
            id: Uuid::from_u128(2),
            name: "Standard week".into(),
            kind: CycleKind::Weekly,
        },
        term: None,
        campuses: (0..campuses)
            .map(|n| CampusFact {
                id: Uuid::from_u128(10 + n as u128),
                name: format!("Campus {n}"),
            })
            .collect(),
        cycle_days: vec![CycleDayFact {
            id: Uuid::from_u128(3),
            ordinal: 1,
            label: "Day 1".into(),
        }],
        bell_schedules: schedules
            .iter()
            .enumerate()
            .map(|(n, campus)| BellScheduleFact {
                id: Uuid::from_u128(20 + n as u128),
                name: format!("Schedule {n}"),
                campus: *campus,
                periods: Vec::new(),
            })
            .collect(),
        periods: Vec::new(),
    }
}

fn c(ix: u32) -> CampusIx {
    CampusIx::new(ix)
}

// =======================================================================================
// The compatibility relation — ADR 0033 §2
// =======================================================================================

#[test]
fn clause_one_a_school_wide_schedule_serves_every_section() {
    // `campus_id IS NULL` serves every campus by its own definition, so the section's campus
    // is irrelevant — including when it has none.
    let ctx = context(2, &[None]);
    let schedule = &ctx.bell_schedules[0];
    assert!(is_compatible(&ctx, schedule, None));
    assert!(is_compatible(&ctx, schedule, Some(c(0))));
    assert!(is_compatible(&ctx, schedule, Some(c(1))));
}

#[test]
fn clause_two_a_campus_schedule_serves_a_section_at_that_campus() {
    let ctx = context(2, &[Some(c(0))]);
    let schedule = &ctx.bell_schedules[0];
    assert!(is_compatible(&ctx, schedule, Some(c(0))));
    assert!(
        !is_compatible(&ctx, schedule, Some(c(1))),
        "a schedule for one campus does not serve a section at another"
    );
}

#[test]
fn clause_three_a_sole_campus_makes_an_unconstrained_section_compatible() {
    // The clause that makes the commonest configuration in the product work: one campus, one
    // campus-scoped schedule, a section that leaves `campus_id` null because there is nothing
    // to choose.
    let ctx = context(1, &[Some(c(0))]);
    assert!(is_compatible(&ctx, &ctx.bell_schedules[0], None));
}

#[test]
fn clause_three_does_not_generalise_to_a_multi_campus_school() {
    // ADR 0033 §2 is explicit. At a school with two campuses a section naming none is
    // compatible only with a school-wide schedule, because a campus-scoped schedule is a
    // statement about one campus and the section is not at a known one.
    let ctx = context(2, &[Some(c(0)), Some(c(1))]);
    assert!(!is_compatible(&ctx, &ctx.bell_schedules[0], None));
    assert!(!is_compatible(&ctx, &ctx.bell_schedules[1], None));
}

#[test]
fn the_sole_campus_is_only_a_value_when_there_is_exactly_one() {
    assert_eq!(only_campus(&context(1, &[])), Some(c(0)));
    assert_eq!(only_campus(&context(0, &[])), None, "none is not one");
    assert_eq!(only_campus(&context(2, &[])), None, "two is not one");
}

// =======================================================================================
// Resolution — ADR 0033 §3
// =======================================================================================

/// A grid over the given schedule indices, built without a problem.
fn grid_over(schedules: &[u32]) -> MaterialisedGrid {
    MaterialisedGrid::from_schedules(schedules.iter().copied().map(BellScheduleIx::new).collect())
}

#[test]
fn exactly_one_compatible_schedule_resolves() {
    let ctx = context(1, &[Some(c(0))]);
    assert_eq!(
        grid_over(&[0]).resolve(&ctx, None),
        BellScheduleResolution::Resolved {
            schedule: BellScheduleIx::new(0)
        }
    );
}

#[test]
fn an_empty_grid_is_the_grid_empty_cause_whatever_the_section_says() {
    // Raised before any campus reasoning: with no slots there is nothing to place a lesson
    // in, and the cause is about the solve rather than the section.
    let ctx = context(2, &[Some(c(0)), None]);
    for campus in [None, Some(c(0)), Some(c(1))] {
        assert_eq!(
            grid_over(&[]).resolve(&ctx, campus),
            BellScheduleResolution::None {
                cause: NoBellScheduleCause::GridEmpty
            }
        );
    }
}

#[test]
fn a_section_at_a_campus_with_no_schedule_for_it_names_that_cause() {
    let ctx = context(2, &[Some(c(0))]);
    assert_eq!(
        grid_over(&[0]).resolve(&ctx, Some(c(1))),
        BellScheduleResolution::None {
            cause: NoBellScheduleCause::NoScheduleForCampus
        }
    );
}

#[test]
fn a_campusless_section_at_a_multi_campus_school_names_that_cause() {
    // ADR 0033 test 6, and the cost that record takes deliberately: the school finishes
    // saying which campus teaches what rather than getting a timetable built on a guess.
    let ctx = context(2, &[Some(c(0)), Some(c(1))]);
    assert_eq!(
        grid_over(&[0, 1]).resolve(&ctx, None),
        BellScheduleResolution::None {
            cause: NoBellScheduleCause::SectionHasNoCampus
        }
    );
}

#[test]
fn a_school_wide_and_a_campus_schedule_together_are_ambiguous() {
    // ADR 0033 test 7: clause 1 matches the school-wide schedule and clause 2 the section's
    // own campus, so there are two candidates and Evara will not choose.
    let ctx = context(2, &[None, Some(c(0))]);
    assert_eq!(
        grid_over(&[0, 1]).resolve(&ctx, Some(c(0))),
        BellScheduleResolution::Ambiguous {
            candidates: vec![BellScheduleIx::new(0), BellScheduleIx::new(1)]
        }
    );
}

#[test]
fn candidates_are_listed_in_a_deterministic_order() {
    // A diagnostic that names its candidates must name them in a stable order, or two runs
    // produce two different messages.
    let ctx = context(2, &[None, Some(c(0)), None]);
    let BellScheduleResolution::Ambiguous { candidates } =
        grid_over(&[2, 0, 1]).resolve(&ctx, Some(c(0)))
    else {
        panic!("three compatible schedules is an ambiguity");
    };
    assert_eq!(
        candidates,
        vec![
            BellScheduleIx::new(0),
            BellScheduleIx::new(1),
            BellScheduleIx::new(2)
        ],
        "index order, whatever order the grid was built in"
    );
}

#[test]
fn a_schedule_outside_the_grid_is_never_a_candidate() {
    // ADR 0033 test 8, at the unit level: `G` is what is materialised. A school-wide schedule
    // the school defined and never built a grid from must not create an ambiguity.
    let ctx = context(1, &[Some(c(0)), None]);
    assert_eq!(
        grid_over(&[0]).resolve(&ctx, None),
        BellScheduleResolution::Resolved {
            schedule: BellScheduleIx::new(0)
        },
        "the school-wide schedule is defined but unmaterialised, so it is not in G"
    );
    assert_eq!(
        grid_over(&[0, 1]).resolve(&ctx, None),
        BellScheduleResolution::Ambiguous {
            candidates: vec![BellScheduleIx::new(0), BellScheduleIx::new(1)]
        },
        "and once it is materialised, the same school is ambiguous"
    );
}

// =======================================================================================
// `G` is derived from the grid
// =======================================================================================

#[test]
fn the_grid_is_derived_from_slots_and_not_from_defined_schedules() {
    // The property this whole module depends on. Three fixtures define a schedule they never
    // materialise; if `G` were read from `context.bell_schedules` each would report a false
    // ambiguity, which ADR 0033 test 8 forbids.
    for kind in [
        FixtureKind::UnmaterialisedBellSchedule,
        FixtureKind::InfeasibleDuration,
    ] {
        let found = kind.generate(SEED);
        let problem = found.problem();
        let grid = MaterialisedGrid::of(problem);
        assert_eq!(
            problem.context.bell_schedules.len(),
            2,
            "{}: two schedules are defined",
            kind.name()
        );
        assert_eq!(
            grid.len(),
            1,
            "{}: and only one is materialised",
            kind.name()
        );
    }
}

#[test]
fn an_unmaterialised_grid_is_empty_even_though_a_schedule_is_defined() {
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.context.bell_schedules.len(), 1);
    assert!(MaterialisedGrid::of(problem).is_empty());
}

// =======================================================================================
// Every fixture's declared resolution is the computed one
// =======================================================================================

#[test]
fn every_fixtures_declared_resolution_agrees_with_the_resolver() {
    // The gap M2 left: a fixture *states* its resolution and nothing checked the statement.
    // This is the check. A fixture can no longer declare a resolution the accepted rule would
    // not produce.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for (n, problem) in found.problems.iter().enumerate() {
                let grid = MaterialisedGrid::of(problem);
                for section in &problem.sections {
                    let computed = grid.resolve(&problem.context, section.campus);
                    assert_eq!(
                        section.bell_schedule, computed,
                        "{} problem {n} at seed {seed}: {} declares {:?} but the relation \
                         computes {:?}",
                        found.name, section.name, section.bell_schedule, computed
                    );
                }
            }
        }
    }
}

// =======================================================================================
// Runs of teaching periods — ADR 0033 §5
// =======================================================================================

#[test]
fn a_run_is_broken_by_a_non_teaching_period_between_two_teaching_ones() {
    // The tiny fixture's day is P1, P2, Break, P3, P4 — so two runs of two and no run of
    // three, however many teaching periods the day holds in total.
    let found = FixtureKind::Tiny.generate(SEED);
    let problem = found.problem();
    let schedule = BellScheduleIx::new(0);

    assert_eq!(longest_teaching_run(problem, schedule), 2);
    assert_eq!(teaching_slots(problem, schedule).len(), 4 * 5);

    let day = CycleDayIx::new(0);
    assert_eq!(runs_on(problem, schedule, day, 1).len(), 4, "four singles");
    assert_eq!(
        runs_on(problem, schedule, day, 2).len(),
        2,
        "P1-P2 and P3-P4, and never P2-P3 across the break"
    );
    assert!(
        runs_on(problem, schedule, day, 3).is_empty(),
        "the break makes a triple impossible"
    );
}

#[test]
fn a_run_lists_its_slots_first_to_last_and_consecutively() {
    let found = FixtureKind::Tiny.generate(SEED);
    let problem = found.problem();
    let runs = runs_on(problem, BellScheduleIx::new(0), CycleDayIx::new(0), 2);

    for run in &runs {
        assert_eq!(run.len(), 2);
        let ordinals: Vec<u16> = run
            .iter()
            .map(|ix| {
                let slot = &problem.timeslots[ix.get()];
                problem.context.periods[slot.period.get()].ordinal
            })
            .collect();
        assert_eq!(
            ordinals[1],
            ordinals[0] + 1,
            "a run is ordinal-consecutive, not merely adjacent in a filtered list"
        );
        for ix in run {
            assert!(problem.timeslots[ix.get()].is_teaching);
        }
    }
}

#[test]
fn a_zero_duration_has_no_runs() {
    let found = FixtureKind::Tiny.generate(SEED);
    assert!(
        runs_on(
            found.problem(),
            BellScheduleIx::new(0),
            CycleDayIx::new(0),
            0
        )
        .is_empty(),
        "an occurrence of no periods is not a lesson"
    );
}

#[test]
fn a_run_never_spans_two_cycle_days() {
    let found = FixtureKind::Tiny.generate(SEED);
    let problem = found.problem();
    for day in 0..problem.context.cycle_days.len() {
        let day = CycleDayIx::new(u32::try_from(day).expect("small"));
        for run in runs_on(problem, BellScheduleIx::new(0), day, 2) {
            for ix in run {
                assert_eq!(problem.timeslots[ix.get()].cycle_day, day);
            }
        }
    }
}

#[test]
fn the_longest_run_is_measured_per_schedule_and_never_across_them() {
    // ADR 0033 test 10's premise, and the measure is stricter than that test needs: the run
    // is counted over the schedule's **slots in this grid**, not over its period definitions.
    //
    // The infeasible fixture defines a schedule whose four teaching periods would hold a run
    // of four, and never materialises it. So the figure a refusal may use is 2 — and the
    // longer schedule contributes 0 rather than 4, because an unmaterialised schedule has
    // nothing to place a lesson in at all.
    let found = FixtureKind::InfeasibleDuration.generate(SEED);
    let problem = found.problem();
    let grid = MaterialisedGrid::of(problem);
    let resolved = grid.schedules()[0];

    assert_eq!(longest_teaching_run(problem, resolved), 2);

    let other = BellScheduleIx::new(1);
    assert_ne!(resolved, other);
    let defined_teaching = problem.context.bell_schedules[other.get()]
        .periods
        .iter()
        .filter(|p| problem.context.periods[p.get()].kind.is_teaching())
        .count();
    assert_eq!(
        defined_teaching, 4,
        "the other schedule's periods really would hold a run of four"
    );
    assert_eq!(
        longest_teaching_run(problem, other),
        0,
        "and it still offers no run, because it has no slots in this grid"
    );
}

#[test]
fn a_schedule_with_no_slots_offers_no_run_and_no_teaching_slot() {
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let problem = found.problem();
    let schedule = BellScheduleIx::new(0);
    assert_eq!(longest_teaching_run(problem, schedule), 0);
    assert_eq!(teaching_slots(problem, schedule).len(), 0);
}

#[test]
fn resolution_is_deterministic_over_the_whole_catalogue() {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            let first = MaterialisedGrid::of(problem);
            let second = MaterialisedGrid::of(problem);
            assert_eq!(first, second);
            for section in &problem.sections {
                assert_eq!(
                    first.resolve(&problem.context, section.campus),
                    second.resolve(&problem.context, section.campus)
                );
            }
        }
    }
}
