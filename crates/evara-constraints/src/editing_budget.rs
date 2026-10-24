//! What a pick-up costs at the realistic upper end.
//!
//! [OPEN-DECISIONS.md](../../../docs/OPEN-DECISIONS.md) R5 names *"a 2,000-activity secondary
//! school with electives"* as the realistic upper end and asks for its numbers to be treated as
//! a budget. M8 paid the projection half of that debt in `evara-report`'s `budget.rs`; this file
//! pays the **editing** half, which M9A is the first milestone to owe.
//!
//! # What an editor actually costs, and why it is measured here
//!
//! `edit_candidates` answers a whole panel in one call and does exactly two things:
//!
//! 1. builds one [`IncrementalScorer`](crate::IncrementalScorer) over the current arrangement —
//!    one reference evaluation of the whole timetable, and **no database access**;
//! 2. calls [`probe`](crate::IncrementalScorer::probe) once per slot of the lesson's own bell
//!    schedule.
//!
//! Both live in this crate, so both are measurable with no project file, no IPC and no session.
//! What is *not* measured here is the IPC round trip and the session's own bookkeeping, neither
//! of which is this crate's and neither of which scales with the school.
//!
//! # Two arrangements, because the cost is dominated by what is wrong
//!
//! The scorer's cost is **not** a function of the school's size alone. It builds the violation
//! list, so an arrangement with tens of thousands of clashes costs far more than a clean one of
//! the same size — and the two numbers are different enough that quoting one as *"the editor's
//! cost"* would be misleading whichever one was chosen. So both are measured:
//!
//! - [`a_legal_arrangement`] — 1 600 occurrences, one per `(slot, room)`, **no clash of any
//!   kind**, asserted by a full `evaluate` before anything is timed. What a school that has just
//!   generated a timetable actually opens.
//! - [`a_dense_arrangement`] — 2 000 occurrences over the same 40 slots and 40 rooms, which is
//!   **over-subscribed by construction**: 50 occurrences per slot against 40 rooms, so it holds
//!   thousands of real room clashes. Not a timetable anybody would have; a deliberate worst case
//!   for the violation list, and labelled as one.
//!
//! The dense case is therefore a *cost* fixture and never a *legality* fixture. Nothing here
//! asserts what is legal — `differential.rs` owns that, over feasible arrangements.
//!
//! # What is asserted, and what is only measured
//!
//! The **structural** figures are asserted, because they are deterministic: how many activities
//! the school holds, how many slots a panel has, that the legal arrangement really is legal, and
//! that a probe returns a verdict for every cell. The **wall-clock** figures are measured,
//! printed and bounded only loosely — a tight timing assertion on shared CI hardware is a flaky
//! test, and a flaky test is worse than a recorded number. Run with `--nocapture` to read them.
//!
//! **These are debug-build figures.** `cargo test` builds without optimisation, and that is the
//! number a developer sees; `TIMETABLE-EDITING.md` quotes both, because the difference between
//! them is large and a reader deserves to know which one they are being given.
//!
//! This is the second place in the workspace that reads a clock on purpose, and for the same
//! reason as the first: it is a test *about* cost, nothing it measures reaches a scheduling
//! decision, and `CLAUDE.md`'s rule that the engine reads no clock is untouched.

use std::time::Instant;

use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleResolution, CycleKind, GroupKind, Overlap, Placement,
    Problem, RoomIx, TimeslotIx,
};

use crate::{IncrementalScorer, Registry};

/// A synthetic secondary school of roughly `sections` sections, each with four single periods.
///
/// The same shape `evara-report`'s `budget.rs` and `evara-solver`'s own upper-end measurement
/// use, and built here for the same reason both of those are built where they are: the
/// catalogue's largest fixture is `medium` at 500 occurrences, and widening a documented
/// 24-fixture catalogue is M2's surface rather than M9A's. So the upper-end school lives with
/// the measurement that needs it.
fn a_large_school(sections: usize, per_section: &[u16]) -> Problem {
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

    for n in 0..40 {
        school.add_room(RoomFactSpec::new(&format!("Room {n}"), 30).on_campus(campus));
    }
    let teachers: Vec<_> = (0..80)
        .map(|n| school.add_teacher(TeacherFactSpec::new(&format!("Teacher {n}"))))
        .collect();
    let groups: Vec<_> = (0..60)
        .map(|n| {
            school.add_group(GroupSpec::new(&format!("Class {n}"), GroupKind::Class).sized(28))
        })
        .collect();
    // A group shares students with itself and with nothing else that is stated, so
    // `student-clash` has real work to do and the indeterminate pairs stay indeterminate.
    for group in &groups {
        school.declare_overlap(*group, *group, Overlap::Overlaps);
    }

    for n in 0..sections {
        let section = school.add_section(
            SectionSpec::new(&format!("Course {n}"))
                .schedule(BellScheduleResolution::Resolved { schedule })
                .taught_by(teachers[n % teachers.len()])
                .taught_to(groups[n % groups.len()]),
        );
        school.add_activities(section, per_section);
    }
    school.finish()
}

/// A legal arrangement: one occurrence per `(slot, room)`, and no clash of any kind.
///
/// Activity `i` goes to slot `i / rooms` and room `i % rooms`, which makes every `(slot, room)`
/// pair unique and — because the occurrences in one slot are 40 **consecutive** indices — gives
/// that slot 40 distinct teachers of 80 and 40 distinct groups of 60. Nothing is double-booked.
///
/// # Why the school must have one occurrence per section
///
/// The first version of this function placed activity `i` at slot `i % slots`, over a school of
/// 400 sections with four occurrences each. That double-books every teacher: a section's four
/// occurrences share a teacher *and* a group, and `i / 4` is constant across four consecutive
/// indices, so the four landed in four slots — but in each of those slots the other occupants
/// were a teacher-cycle apart and collided. The measurement reported 2 400 hard penalties on a
/// fixture whose whole purpose was to have none.
///
/// Giving each section one occurrence makes the section index and the activity index the same
/// number, which is what the consecutive-indices argument above needs. A section's occurrences
/// are not what this file measures; the size of the violation list is.
///
/// This is what a school opens for editing the moment after it saves a generated timetable, and
/// it is the figure to quote.
fn a_legal_arrangement(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activity_count());
    let slots: Vec<TimeslotIx> = problem
        .timeslots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.is_teaching)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap_or(0)))
        .collect();
    let rooms: Vec<RoomIx> = problem
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| room.is_bookable)
        .map(|(ix, _)| RoomIx::new(u32::try_from(ix).unwrap_or(0)))
        .collect();
    assert!(
        problem.activities.len() <= slots.len() * rooms.len(),
        "a legal arrangement needs a cell per occurrence: {} occurrences, {} slots, {} rooms",
        problem.activities.len(),
        slots.len(),
        rooms.len()
    );
    for ix in 0..problem.activities.len() {
        let activity = ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
        assignment.place(
            activity,
            Placement {
                timeslot: slots[ix / rooms.len()],
                room: Some(rooms[ix % rooms.len()]),
            },
        );
    }
    assignment
}

/// Fills the grid round-robin, so almost every cell is occupied **several times over**.
///
/// Not a solve and not legal: 2 000 occurrences over 40 slots and 40 rooms is 50 per slot against
/// 40 rooms, so this holds thousands of genuine room clashes. That is the point — it is the worst
/// case for the violation list the scorer builds, and the upper bound on what an editor could
/// ever be asked to score. Never used to assert anything about legality.
fn a_dense_arrangement(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activity_count());
    let slots: Vec<TimeslotIx> = problem
        .timeslots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.is_teaching)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap_or(0)))
        .collect();
    let rooms: Vec<RoomIx> = problem
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| room.is_bookable)
        .map(|(ix, _)| RoomIx::new(u32::try_from(ix).unwrap_or(0)))
        .collect();
    if slots.is_empty() {
        return assignment;
    }
    for ix in 0..problem.activities.len() {
        let activity = ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
        assignment.place(
            activity,
            Placement {
                timeslot: slots[ix % slots.len()],
                room: rooms.get(ix % rooms.len().max(1)).copied(),
            },
        );
    }
    assignment
}

/// Builds a scorer and probes every slot, reporting both costs.
///
/// Returned rather than asserted, so the two arrangements are measured by one body and the
/// difference between them is the thing on screen.
fn measure(problem: &Problem, assignment: &Assignment, what: &str) -> (usize, usize) {
    let rules = Registry::mandatory();

    let started = Instant::now();
    let mut scorer =
        IncrementalScorer::of(problem, assignment, &rules).expect("the arrangement is readable");
    let built = started.elapsed();
    let violations = scorer.score();

    let subject = ActivityIx::new(0);
    let room = problem
        .rooms
        .iter()
        .position(|room| room.is_bookable)
        .map(|ix| RoomIx::new(u32::try_from(ix).unwrap_or(0)));

    let slots = problem.timeslots.len();
    let started = Instant::now();
    let mut answered = 0;
    let mut allowed = 0;
    for ix in 0..slots {
        let verdict = scorer.probe(
            subject,
            Placement {
                timeslot: TimeslotIx::new(u32::try_from(ix).unwrap_or(0)),
                room,
            },
        );
        answered += 1;
        if verdict.allowed {
            allowed += 1;
        }
    }
    let panel = started.elapsed();

    println!(
        "editing budget — {what}: {} occurrences, score {violations:?}; scorer {built:?};          panel of {slots} cells {panel:?} ({:?} per cell); {allowed} legal",
        problem.activities.len(),
        panel / u32::try_from(slots.max(1)).unwrap_or(1),
    );

    // Hang detectors, not performance assertions. Both are far above the figures above on
    // ordinary hardware; a tight bound here would be a flaky test.
    assert!(built.as_secs() < 60, "building the scorer took {built:?}");
    assert!(panel.as_secs() < 60, "the panel took {panel:?}");
    (answered, allowed)
}

#[test]
fn a_candidate_panel_is_answered_within_budget_over_a_clean_timetable() {
    // The figure to quote. 1 600 sections of one single period each, arranged legally over a
    // 5 × 8 grid with 40 rooms — see `a_legal_arrangement` for why one each.
    let problem = a_large_school(1_600, &[1]);
    assert_eq!(problem.activities.len(), 1_600);
    assert_eq!(problem.timeslots.len(), 40, "five days of eight periods");

    let assignment = a_legal_arrangement(&problem);
    let rules = Registry::mandatory();
    let report = crate::evaluate(&problem, &assignment, &rules);
    assert_eq!(
        report.score.hard, 0,
        "the point of this fixture is that nothing is wrong with it: {:?}",
        report.violations
    );

    let (answered, allowed) = measure(&problem, &assignment, "a clean timetable");
    assert_eq!(answered, 40, "every cell of the panel gets a verdict");
    assert!(
        allowed > 0,
        "and a clean timetable has somewhere to put a lesson"
    );
}

#[test]
fn a_candidate_panel_is_answered_within_budget_at_the_documented_worst_case() {
    let problem = a_large_school(500, &[1, 1, 1, 1]);
    let assignment = a_dense_arrangement(&problem);

    // Structural, and deterministic. 500 sections × four single periods, over a 5 × 8 grid.
    assert_eq!(problem.activities.len(), 2_000, "the documented upper end");
    let slots = problem.timeslots.len();
    assert_eq!(slots, 40, "five days of eight periods");

    let (answered, _) = measure(&problem, &assignment, "an over-subscribed arrangement");
    assert_eq!(answered, slots, "every cell of the panel gets a verdict");
}

#[test]
fn a_probe_does_not_report_an_occurrence_as_clashing_with_itself_at_scale() {
    // The property that makes `probe` a move validator rather than a placement validator,
    // asserted at the upper end where it would be easiest to get wrong. Probing the slot an
    // occurrence is **already in** must come back allowed, because `probe` withdraws it first.
    //
    // Over the **legal** arrangement, necessarily. The first version of this test used the dense
    // one and failed — correctly: probing activity 0's own cell there still clashes with the
    // forty-nine *other* occupants of that room-slot, because that fixture is over-subscribed by
    // construction. The failure was the fixture's, not the code's, and it is why the two
    // arrangements are now separate functions with the dense one labelled a cost fixture.
    let problem = a_large_school(1_600, &[1]);
    let rules = Registry::mandatory();
    let assignment = a_legal_arrangement(&problem);
    let mut scorer = IncrementalScorer::of(&problem, &assignment, &rules).expect("readable");

    let subject = ActivityIx::new(0);
    let where_it_is = assignment
        .placement(subject)
        .expect("the arrangement places everything");
    let verdict = scorer.probe(subject, where_it_is);
    assert!(
        verdict.allowed,
        "a lesson does not clash with itself: {:?}",
        verdict.blocking
    );
    assert_eq!(
        verdict.delta,
        evara_core::scheduling::Score::PERFECT,
        "and putting it back where it is costs nothing"
    );
}
