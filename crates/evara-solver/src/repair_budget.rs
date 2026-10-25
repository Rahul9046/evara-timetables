//! What a repair costs, measured and printed rather than asserted tightly.
//!
//! # Measured, not bounded
//!
//! The structural figures are asserted, because they are deterministic: how many occurrences
//! the school holds, how many may move, that the baseline really is legal, that every frozen
//! placement survived. The **wall-clock** figures are measured, printed and bounded only
//! loosely as hang detectors — a tight timing assertion on shared CI hardware is a flaky test,
//! and a flaky test is worse than a recorded number. Run with `--nocapture` to read them.
//!
//! **These are debug-build figures.** `cargo test` builds without optimisation, and that is the
//! number a developer sees. `M9B-DELIVERY-REPORT.md` quotes both, because the difference is
//! large and a reader deserves to know which one they are being given.
//!
//! # Three shapes, because the cost is dominated by what is wrong
//!
//! A repair's cost is not a function of the school's size alone. It builds a scorer over the
//! frozen arrangement, and a scorer builds the violation list — so a conflicted baseline costs
//! far more than a clean one of the same size, for the reason `editing_budget.rs` documents:
//! the time goes on allocating `Violation` structs with formatted `detail` strings, not on
//! comparing intervals.
//!
//! - **a clean whole-timetable repair** — every occurrence movable, nothing wrong. The common
//!   case, and the one the baseline-first probe should make nearly free.
//! - **a narrow scope** — one occurrence movable, the rest frozen. Should be *cheaper* than the
//!   whole, because the frozen set is placed into the scorer rather than searched.
//! - **a conflicted baseline** — the worst case for the violation list, which is what the
//!   proposal's §6.2 says to watch.
//!
//! This is the third place in the workspace that reads a clock on purpose, and for the same
//! reason as the first two: it is a test *about* cost, nothing it measures reaches a scheduling
//! decision, and `CLAUDE.md`'s rule that the engine reads no clock is untouched.

use std::time::Instant;

use evara_constraints::Registry;
use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleResolution, CompiledConstraint, CycleKind,
    DiagnosticReport, GroupKind, MovableSet, Overlap, PeriodKind, Placement, Problem,
    ProblemIdentity, RoomIx, TimeslotIx, frozen_differences,
};

use crate::{RepairRequest, SolveConfig, repair};

/// Eight periods a day, five days: the 40-slot grid `editing_budget` uses.
fn a_week() -> Vec<PeriodSpec> {
    (1..=8)
        .map(|n| {
            let start = 8 + n;
            PeriodSpec::teaching(
                &n.to_string(),
                &format!("{start:02}:00"),
                &format!("{:02}:00", start + 1),
            )
        })
        .collect()
}

/// A school of `sections` sections, each with one occurrence per entry in `per_section`.
///
/// 40 rooms and 40 teachers, so the grid has `40 × 40` cells and a legal one-per-cell
/// arrangement exists for up to 1 600 occurrences.
fn a_large_school(sections: usize, per_section: &[u16]) -> Problem {
    let mut school = SchoolBuilder::new(5, "Northgate", "Standard week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1", "Day 2", "Day 3", "Day 4", "Day 5"], 5);
    let schedule = school.add_bell_schedule("Standard Day", Some(campus), &a_week());
    school.materialise(schedule);

    for n in 0..40 {
        school.add_room(RoomFactSpec::new(&format!("Room {n}"), 30).on_campus(campus));
    }
    let teachers: Vec<_> = (0..40)
        .map(|n| school.add_teacher(TeacherFactSpec::new(&format!("Teacher {n}"))))
        .collect();

    for n in 0..sections {
        let group =
            school.add_group(GroupSpec::new(&format!("Class {n}"), GroupKind::Class).sized(26));
        school.declare_overlap(group, group, Overlap::Overlaps);
        let section = school.add_section(
            SectionSpec::new(&format!("Lesson {n}"))
                .schedule(BellScheduleResolution::Resolved { schedule })
                .taught_by(teachers[n % teachers.len()])
                .taught_to(group),
        );
        school.add_activities(section, per_section);
    }
    let _ = PeriodKind::Break;
    school.finish()
}

/// A genuinely legal arrangement for a school of `per_section` occurrences each.
///
/// # Why this is not "one per `(slot, room)` cell"
///
/// That simpler spread is legal only when every section has ONE occurrence. With eight each it
/// puts a section's eight into eight rooms of the **same slot** — the same teacher and the same
/// class in eight places at once — so the fixture would be a worst case pretending to be a
/// clean one, and the figure it printed would be about resolving 5 600 self-inflicted clashes.
///
/// So the mapping is derived from the construction instead. For activity `a`, with `p`
/// occurrences per section and `t` teachers, where `n = a / p` is its section:
///
/// ```text
/// k    = a % p          the occurrence's index within its section
/// who  = n % t          its teacher, as `a_large_school` assigns them
/// block= n / t          which run of `p` slots its section occupies
/// slot = block * p + k
/// room = who
/// ```
///
/// Every occurrence of one section lands in a distinct slot; every occurrence of one teacher
/// does too, because `(who, block, k)` determines `(slot, room)` injectively; and each
/// `(slot, room)` cell holds exactly one occurrence. Asserted by the caller, which runs the
/// reference evaluator over it before timing anything.
fn a_legal_arrangement(problem: &Problem, per_section: usize, teachers: usize) -> Assignment {
    let mut assignment = Assignment::empty(problem.activities.len());
    let per = per_section.max(1);
    let who_count = teachers.max(1);

    for a in 0..problem.activities.len() {
        let section = a / per;
        let k = a % per;
        let who = section % who_count;
        let block = section / who_count;
        let slot = block * per + k;
        if slot >= problem.timeslots.len() || who >= problem.rooms.len() {
            break;
        }
        assignment.place(
            ActivityIx::new(u32::try_from(a).expect("fits")),
            Placement {
                timeslot: TimeslotIx::new(u32::try_from(slot).expect("fits")),
                room: Some(RoomIx::new(u32::try_from(who).expect("fits"))),
            },
        );
    }
    assignment
}

/// Everything stacked into the first few slots: thousands of genuine clashes.
///
/// Not a timetable anybody would have, and labelled as one: a deliberate worst case for the
/// violation list.
fn a_conflicted_arrangement(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activities.len());
    let slots = problem.timeslots.len().max(1);
    for ix in 0..problem.activities.len() {
        assignment.place(
            ActivityIx::new(u32::try_from(ix).expect("fits")),
            Placement {
                timeslot: TimeslotIx::new(u32::try_from(ix % slots).expect("fits")),
                room: Some(RoomIx::new(0)),
            },
        );
    }
    assignment
}

fn rules() -> Vec<CompiledConstraint> {
    Registry::mandatory()
}

/// Runs one repair, prints what it cost, and asserts the scope guarantee held.
fn measure(
    problem: &Problem,
    baseline: &Assignment,
    movable: &MovableSet,
    what: &str,
) -> crate::RepairOutcome {
    let rules = rules();
    let started = Instant::now();
    let out = repair(&RepairRequest {
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
        config: SolveConfig::seeded(2026),
        baseline,
        movable,
    });
    let elapsed = started.elapsed();

    println!(
        "repair budget — {what}: {} occurrences, {} movable, {} frozen; {elapsed:?}; \
         {} placed, {} candidate evaluations; score {:?} -> {:?}",
        problem.activities.len(),
        movable.len(),
        movable.frozen_count(),
        out.movable_placed,
        out.outcome.statistics.candidate_evaluations,
        out.baseline_score,
        out.outcome.score,
    );

    // The guarantee, asserted on every measurement rather than only in the correctness tests:
    // a slow repair that also moved a frozen lesson must not read as a timing result.
    assert_eq!(out.defect, None, "{what}: {:?}", out.defect);
    assert_eq!(
        frozen_differences(baseline, &out.outcome.assignment, movable),
        vec![],
        "{what}: a frozen placement changed"
    );

    // A hang detector, not a performance assertion. Far above the printed figures on ordinary
    // hardware; a tight bound here would be a flaky test.
    assert!(
        elapsed.as_secs() < 120,
        "{what} took {elapsed:?}, which is long enough to be a hang rather than a cost"
    );
    out
}

#[test]
fn a_whole_timetable_repair_of_a_clean_baseline_is_within_budget() {
    // 1 600 occurrences over **200 sections of eight**, not 1 600 sections of one.
    //
    // The section count is the load-bearing choice. `construct::conflict_degrees` is
    // documented as quadratic in the SECTIONS -- "a 500-occurrence school has far fewer
    // sections, and every occurrence of a section has the same degree" -- so 1 600
    // single-occurrence sections is its worst case and no real school's shape.
    // `where_the_time_goes_at_1600_occurrences` measures what that costs and shows it is
    // M6's `prepare`, paid identically by a generation; this test uses the realistic shape so
    // the figure it prints is about the repair.
    let problem = a_large_school(200, &[1, 1, 1, 1, 1, 1, 1, 1]);
    assert_eq!(problem.activities.len(), 1_600);
    assert_eq!(problem.timeslots.len(), 40, "five days of eight periods");

    let baseline = a_legal_arrangement(&problem, 8, 40);
    // The whole point of this fixture is that nothing is wrong with it. Asserted by the
    // reference evaluator, before anything is timed: a "clean" baseline that was not clean
    // would make this a worst-case measurement wearing the wrong label.
    let report = evara_constraints::evaluate(&problem, &baseline, &rules());
    assert_eq!(
        report.score.hard,
        0,
        "the baseline must really be legal; found {:?}",
        report
            .violations
            .iter()
            .take(3)
            .map(|v| v.detail.clone())
            .collect::<Vec<_>>()
    );

    let movable = MovableSet::everything(problem.activities.len()).expect("movable");
    let out = measure(
        &problem,
        &baseline,
        &movable,
        "a clean whole-timetable repair",
    );

    assert_eq!(
        out.outcome.assignment, baseline,
        "a legal baseline must be reproduced exactly"
    );
    assert_eq!(
        out.outcome.statistics.candidate_evaluations, 1_600,
        "exactly one evaluation per occurrence: the baseline probe, and no walk"
    );
    assert_eq!(out.movable_placed, 1_600);
}

#[test]
fn a_narrow_scope_is_cheaper_than_the_whole_timetable() {
    // The frozen set is placed into the scorer rather than searched, so a one-occurrence scope
    // should cost a small fraction of the whole. Asserted as an *ordering* rather than a ratio,
    // because a ratio on CI hardware is the flaky assertion this file exists to avoid.
    let problem = a_large_school(50, &[1, 1, 1, 1, 1, 1, 1, 1]);
    let baseline = a_legal_arrangement(&problem, 8, 40);
    assert_eq!(
        evara_constraints::evaluate(&problem, &baseline, &rules())
            .score
            .hard,
        0,
        "the baseline must be legal for the comparison to mean anything"
    );

    let whole = MovableSet::everything(problem.activities.len()).expect("movable");
    let narrow = MovableSet::new(problem.activities.len(), &[ActivityIx::new(0)]).expect("movable");

    let wide_out = measure(
        &problem,
        &baseline,
        &whole,
        "whole timetable, 400 occurrences",
    );
    let narrow_out = measure(&problem, &baseline, &narrow, "one occurrence of 400");

    assert_eq!(narrow_out.movable_count, 1);
    assert_eq!(narrow_out.frozen_count, 399);
    assert!(
        narrow_out.outcome.statistics.candidate_evaluations
            < wide_out.outcome.statistics.candidate_evaluations,
        "a narrow scope must evaluate fewer candidates: {} vs {}",
        narrow_out.outcome.statistics.candidate_evaluations,
        wide_out.outcome.statistics.candidate_evaluations
    );
}

#[test]
fn a_conflicted_baseline_is_slow_and_says_so() {
    // The case §6.2 says to watch. The cost is the violation list, not the search, so this is
    // measured and printed rather than bounded: the fix is to make `Violation` defer its
    // formatted `detail`, and that is M5's surface rather than M9B's.
    let problem = a_large_school(100, &[1, 1, 1, 1, 1]);
    let baseline = a_conflicted_arrangement(&problem);
    let rules = rules();

    let report = evara_constraints::evaluate(&problem, &baseline, &rules);
    assert!(
        !report.score.is_feasible(),
        "the point of this fixture is that a great deal is wrong with it"
    );
    println!(
        "repair budget — the conflicted baseline holds {} violations, score {:?}",
        report.violations.len(),
        report.score
    );

    let movable = MovableSet::everything(problem.activities.len()).expect("movable");
    let out = measure(&problem, &baseline, &movable, "a conflicted baseline");

    // And it still repairs it, which is the point of the feature.
    assert!(
        out.outcome.score > out.baseline_score,
        "a repair of a badly broken timetable must improve it: {:?} -> {:?}",
        out.baseline_score,
        out.outcome.score
    );
}

#[test]
fn a_repair_of_a_conflicted_baseline_outside_the_scope_is_still_applyable() {
    // The blocking defect, at scale: the final arrangement holds thousands of hard violations
    // among frozen occurrences, and the repair must still be a usable result.
    let problem = a_large_school(100, &[1, 1, 1, 1, 1]);
    let baseline = a_conflicted_arrangement(&problem);

    // Only the first ten may move; the remaining 490 stay stacked and clashing.
    let indices: Vec<ActivityIx> = (0..10).map(ActivityIx::new).collect();
    let movable = MovableSet::new(problem.activities.len(), &indices).expect("movable");

    let out = measure(
        &problem,
        &baseline,
        &movable,
        "ten of 500, the rest conflicted",
    );

    let report = evara_constraints::evaluate(&problem, &out.outcome.assignment, &rules());
    assert!(
        !report.score.is_feasible(),
        "the frozen clashes must still be there, or this proves nothing"
    );
    assert!(
        out.applyable,
        "a correct repair of a conflicted timetable must be applyable"
    );
    assert!(
        !out.hard_partition.pre_existing_frozen.is_empty(),
        "and the frozen damage must be attributed rather than blamed on the repair"
    );
}

#[test]
// ~195 s in debug, almost all of it inside `prepare`. Kept because it is the evidence that the
// cost belongs to M6's `conflict_degrees` at an unrealistic section count rather than to the
// repair -- a generation over the same problem is measured here too and is just as slow -- and
// ignored because a three-minute test in the ordinary suite would be paid on every run by
// everyone. Run it with:
//
//     cargo test -p evara-solver --all-features --lib //         repair_budget::where_the_time_goes -- --ignored --nocapture
#[ignore = "a ~3-minute diagnostic; run it deliberately, not on every build"]
fn where_the_time_goes_at_1600_occurrences() {
    // A diagnostic, not a budget: it splits the clean 1 600-occurrence figure into the parts
    // that could own it, so a slow repair is attributed rather than guessed at.
    let problem = a_large_school(1_600, &[1]);
    let baseline = a_legal_arrangement(&problem, 1, 40);
    let rules = rules();

    let t = Instant::now();
    let _report = evara_constraints::evaluate(&problem, &baseline, &rules);
    let reference = t.elapsed();

    let t = Instant::now();
    let scorer =
        evara_constraints::IncrementalScorer::of(&problem, &baseline, &rules).expect("readable");
    let built = t.elapsed();

    // One probe of one occurrence, at its own baseline placement.
    let mut scorer = scorer;
    let at = ActivityIx::new(0);
    let where_it_is = baseline.placement(at).expect("placed");
    let t = Instant::now();
    let _verdict = scorer.probe(at, where_it_is);
    let one_probe = t.elapsed();

    // A generation over the same problem, for comparison: if this is as slow, the cost is
    // M6's at this fixture shape rather than the repair's.
    let t = Instant::now();
    let generated = crate::generate(&crate::GenerateRequest {
        problem: &problem,
        identity: ProblemIdentity {
            school: problem.context.school.id,
            cycle: problem.context.cycle.id,
            term: None,
        },
        constraints: &rules,
        preflight: DiagnosticReport::empty(),
        may_solve: true,
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(2026),
    });
    let generation = t.elapsed();

    println!(
        "repair budget — attribution at 1 600 occurrences / 1 600 sections:\n  \
         reference evaluate      {reference:?}\n  \
         build the scorer        {built:?}\n  \
         one probe               {one_probe:?}  (x1600 = {:?})\n  \
         a whole GENERATION      {generation:?}  ({} placed)",
        one_probe * 1600,
        generated.assignment.scheduled_count(),
    );
}
