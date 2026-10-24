//! What each rule finds, and — more often the point — what it does not.
//!
//! # Built with M2's builder, not with hand-written facts
//!
//! `evara_core::scheduling::fixtures::builder` is public behind the `fixtures` feature, so a
//! test can declare a school in a dozen lines and get a real [`Problem`] with real indices,
//! masks and a materialised grid. Constructing a `Problem` literal instead would mean a test
//! that passes because its hand-built masks agree with its hand-built expectations.
//!
//! The one place a *catalogue* fixture is used is the headline test: `tiny` ships a
//! known-feasible arrangement, and the strongest single assertion in this file is that
//! evaluating it finds nothing hard. That arrangement was built by M2 against the ADRs and
//! verified there with `PlacementSpan::clashes_in_time_with`; agreeing with it is agreeing with
//! an independent reading of the same records.

use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::fixtures::fixture;
use evara_core::scheduling::{
    ActivityIx, Assignment, AvailabilityEntity, AvailabilityStatus, BellScheduleResolution,
    CompiledConstraint, ConstraintLevel, ConstraintType, CycleKind, GroupIx, GroupKind, Overlap,
    Placement, Problem, RoomIx, TimeslotIx,
};

use crate::evaluation::Completeness;
use crate::{Registry, evaluate, evaluate_placement, evaluator_for};

// =======================================================================================
// A school to break
// =======================================================================================

/// Four teaching periods with a break between the second and third, five days.
///
/// The break is what makes `duration-contiguous` and `no-teaching-in-period` testable: a day
/// of nothing but teaching periods cannot demonstrate either.
fn standard_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("1", "09:00", "10:00"),
        PeriodSpec::teaching("2", "10:00", "11:00"),
        PeriodSpec::non_teaching(
            "Break",
            "11:00",
            "11:30",
            evara_core::scheduling::PeriodKind::Break,
        ),
        PeriodSpec::teaching("3", "11:30", "12:30"),
        PeriodSpec::teaching("4", "13:30", "14:30"),
    ]
}

/// A school with two sections that share a teacher, a room and nothing else.
///
/// Returns the problem and the slot indices of day 1, so a test can place things precisely.
fn two_sections() -> (Problem, Vec<TimeslotIx>) {
    let mut school = SchoolBuilder::new(7, "Northgate", "Standard week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1", "Day 2"], 2);
    let schedule = school.add_bell_schedule("Standard Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);

    let general = school.add_room_type("General Teaching");
    let lab = school.add_room_type("Science Laboratory");
    school.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    school.add_room(
        RoomFactSpec::new("Lab 1", 24)
            .on_campus(campus)
            .of_type(lab),
    );

    let teacher = school.add_teacher(TeacherFactSpec::new("A. Abara"));
    let nine_a = school.add_group(GroupSpec::new("9A", GroupKind::Class).sized(28));
    let nine_b = school.add_group(GroupSpec::new("9B", GroupKind::Class).sized(26));
    // Declared disjoint, so concurrency between them is legal and provable.
    school.declare_overlap(nine_a, nine_b, Overlap::Disjoint);

    let maths = school.add_section(
        SectionSpec::new("9A Mathematics")
            .at(campus)
            .taught_by(teacher)
            .taught_to(nine_a),
    );
    school.add_activities(maths, &[1, 1]);

    let science = school.add_section(
        SectionSpec::new("9B Science")
            .at(campus)
            .taught_by(teacher)
            .taught_to(nine_b)
            .requires_room_type(lab),
    );
    school.add_activities(science, &[1]);

    (school.finish(), slots)
}

/// The slots of one cycle day, in period order.
fn day(problem: &Problem, ordinal: u16) -> Vec<TimeslotIx> {
    let day = problem
        .context
        .cycle_days
        .iter()
        .position(|d| d.ordinal == ordinal)
        .map(|at| evara_core::scheduling::CycleDayIx::new(u32::try_from(at).expect("small")))
        .expect("the day");
    let schedule = problem.timeslots[0].bell_schedule;
    problem.slots_of(schedule, day)
}

/// An assignment with nothing placed.
fn nothing(problem: &Problem) -> Assignment {
    Assignment::empty(problem.activities.len())
}

/// The mandatory set — what every project evaluates.
fn rules() -> Vec<CompiledConstraint> {
    Registry::mandatory()
}

fn a(ix: u32) -> ActivityIx {
    ActivityIx::new(ix)
}

// =======================================================================================
// The headline: M2's known-feasible arrangement breaks nothing
// =======================================================================================

#[test]
fn the_known_feasible_arrangement_of_tiny_has_no_hard_violation() {
    // The strongest single assertion here. M2 built this arrangement against ADRs 0032-0035
    // and verified its properties independently; the eight evaluators agreeing with it is two
    // readings of the same records agreeing.
    let found = fixture("tiny", 2026).expect("tiny exists");
    let problem = found.problem();
    let feasible = found
        .expectations
        .feasible
        .as_ref()
        .expect("tiny ships a known-feasible arrangement");
    let assignment = feasible.assignment(problem.activities.len());

    let report = evaluate(problem, &assignment, &rules());

    assert!(report.is_readable(), "{:?}", report.invalid_references);
    assert_eq!(
        report.count_at(ConstraintLevel::Hard),
        0,
        "hard violations in a known-feasible arrangement: {:?}",
        report
            .at_level(ConstraintLevel::Hard)
            .map(|v| (v.constraint_type.key(), v.detail.clone()))
            .collect::<Vec<_>>()
    );
    assert!(report.is_usable());
    assert_eq!(report.completeness, Completeness::Complete);
    assert!(report.is_adoptable(), "and it is a complete timetable");
}

#[test]
fn evaluating_the_same_candidate_twice_gives_an_identical_report() {
    let found = fixture("tiny", 2026).expect("tiny exists");
    let problem = found.problem();
    let assignment = found
        .expectations
        .feasible
        .as_ref()
        .expect("an arrangement")
        .assignment(problem.activities.len());

    assert_eq!(
        evaluate(problem, &assignment, &rules()),
        evaluate(problem, &assignment, &rules()),
        "a report must be reproducible — SOLVER.md §5"
    );
}

// =======================================================================================
// Partial and complete
// =======================================================================================

#[test]
fn an_empty_assignment_is_partial_usable_and_medium_only() {
    // The state every M6 construction starts in. Nothing is placed, so nothing can clash:
    // the report is usable and incomplete, and promoting `activity-scheduled` to hard would
    // make this — and every step after it — infeasible.
    let (problem, _) = two_sections();
    let report = evaluate(&problem, &nothing(&problem), &rules());

    assert_eq!(
        report.completeness,
        Completeness::Partial {
            placed: 0,
            required: 3
        }
    );
    assert_eq!(report.count_at(ConstraintLevel::Hard), 0);
    assert_eq!(report.count_of(ConstraintType::ActivityScheduled), 3);
    assert!(report.is_usable(), "nothing placed breaks no hard rule");
    assert!(!report.is_adoptable(), "but it is not a timetable yet");
    assert_eq!(report.score.hard, 0);
    assert_eq!(report.score.medium, -3, "negative-going, per SOLVER.md §3");
}

#[test]
fn a_partly_placed_assignment_reports_only_what_is_missing() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(
        report.completeness,
        Completeness::Partial {
            placed: 1,
            required: 3
        }
    );
    assert_eq!(report.count_of(ConstraintType::ActivityScheduled), 2);
    assert!(report.is_usable());
}

#[test]
fn every_occurrence_placed_once_is_complete_and_clean() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    // Three occurrences, three different periods, two rooms. The science section needs the
    // laboratory.
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[1], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(slots[3], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert!(report.is_adoptable());
}

// =======================================================================================
// teacher-clash
// =======================================================================================

#[test]
fn one_teacher_in_two_overlapping_lessons_is_a_hard_violation() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(slots[0], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::TeacherClash), 1);
    assert!(!report.is_usable());

    let violation = report
        .at_level(ConstraintLevel::Hard)
        .find(|v| v.constraint_type == ConstraintType::TeacherClash)
        .expect("the clash");
    assert_eq!(violation.activities, vec![a(0), a(2)], "sorted, low first");
    assert_eq!(violation.subject.label, "A. Abara");
    assert_eq!(violation.related.len(), 2, "both sections are named");

    let message = evaluator_for(ConstraintType::TeacherClash)
        .explain(&problem, violation)
        .message;
    assert!(message.contains("A. Abara"), "{message}");
    assert!(message.contains("9A Mathematics"), "{message}");
    assert!(message.contains("9B Science"), "{message}");
}

#[test]
fn adjacent_lessons_do_not_clash() {
    // The half-open interval, from the top. 09:00–10:00 and 10:00–11:00 share no minute, and
    // a closed comparison would make every consecutive pair of lessons a violation.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(slots[1], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::TeacherClash), 0);
    assert_eq!(report.count_of(ConstraintType::RoomClash), 0);
}

#[test]
fn the_same_teacher_on_different_days_does_not_clash() {
    let (problem, _) = two_sections();
    let first = day(&problem, 1);
    let second = day(&problem, 2);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(first[0], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(second[0], RoomIx::new(1)));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::TeacherClash),
        0,
        "a cycle position is the day axis"
    );
}

#[test]
fn a_clash_across_two_bell_schedules_is_found_on_the_minutes() {
    // The case `CONSTRAINTS.md` says the obvious implementation gets wrong, "and wrong
    // silently": two campuses, two schedules whose period 1 differ, one shared teacher and
    // one shared hall. The two placements have different `timeslot_id`s and overlapping
    // minutes.
    let mut school = SchoolBuilder::new(11, "Fairmead", "Week", CycleKind::Custom);
    let lower = school.add_campus("Lower");
    let upper = school.add_campus("Upper");
    school.add_cycle_days(&["Day 1"], 1);

    // Lower teaches 09:00–10:00; Upper teaches 09:30–10:30. They overlap by half an hour and
    // are period 1 of two different schedules.
    let lower_day = school.add_bell_schedule(
        "Lower Day",
        Some(lower),
        &[PeriodSpec::teaching("1", "09:00", "10:00")],
    );
    let upper_day = school.add_bell_schedule(
        "Upper Day",
        Some(upper),
        &[PeriodSpec::teaching("1", "09:30", "10:30")],
    );
    let lower_slots = school.materialise(lower_day);
    let upper_slots = school.materialise(upper_day);

    // One physical hall, reachable from both campuses — ADR 0014.
    let hall = school.add_room(
        RoomFactSpec::new("Shared Hall", 200)
            .on_campus(lower)
            .on_campus(upper),
    );
    let teacher = school.add_teacher(TeacherFactSpec::new("B. Benson"));
    let lower_group = school.add_group(GroupSpec::new("7A", GroupKind::Class).sized(20));
    let upper_group = school.add_group(GroupSpec::new("12A", GroupKind::Class).sized(15));
    school.declare_overlap(lower_group, upper_group, Overlap::Disjoint);

    let lower_section = school.add_section(
        SectionSpec::new("7A Games")
            .at(lower)
            .schedule(BellScheduleResolution::Resolved {
                schedule: lower_day,
            })
            .taught_by(teacher)
            .taught_to(lower_group),
    );
    school.add_activities(lower_section, &[1]);
    let upper_section = school.add_section(
        SectionSpec::new("12A Games")
            .at(upper)
            .schedule(BellScheduleResolution::Resolved {
                schedule: upper_day,
            })
            .taught_by(teacher)
            .taught_to(upper_group),
    );
    school.add_activities(upper_section, &[1]);

    let problem = school.finish();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(lower_slots[0], hall));
    assignment.place(a(1), Placement::in_room(upper_slots[0], hall));

    assert_ne!(
        lower_slots[0], upper_slots[0],
        "two different timeslots, which is exactly why ids cannot be the key"
    );
    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(
        report.count_of(ConstraintType::TeacherClash),
        1,
        "B. Benson is in two rooms from 09:30 to 10:00"
    );
    assert_eq!(
        report.count_of(ConstraintType::RoomClash),
        1,
        "and the hall is double-booked — one room, one identity"
    );
}

// =======================================================================================
// room-clash
// =======================================================================================

#[test]
fn one_room_holding_two_overlapping_lessons_is_a_hard_violation() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    // Two occurrences of the *same* section, so no teacher or student clash confuses the
    // result — one teacher cannot clash with themselves across one section's occurrences...
    // which they can, so this asserts the room finding specifically.
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::RoomClash), 1);
    let violation = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::RoomClash)
        .expect("the clash");
    assert_eq!(violation.subject.label, "Room 1");
    assert!(
        evaluator_for(ConstraintType::RoomClash)
            .explain(&problem, violation)
            .message
            .contains("Room 1")
    );
}

#[test]
fn two_different_rooms_at_the_same_time_do_not_clash() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(1)));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::RoomClash),
        0
    );
}

#[test]
fn an_unroomed_lesson_occupies_no_room() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::unroomed(slots[0]));
    assignment.place(a(1), Placement::unroomed(slots[0]));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::RoomClash),
        0,
        "two roomless lessons do not double-book a room"
    );
}

// =======================================================================================
// student-clash
// =======================================================================================

/// A cohort and one of its classes, each taught its own section.
fn cohort_and_class() -> (Problem, Vec<TimeslotIx>) {
    let mut school = SchoolBuilder::new(13, "Marlowe", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
    school.add_room(RoomFactSpec::new("Room 1", 40).on_campus(campus));
    school.add_room(RoomFactSpec::new("Room 2", 40).on_campus(campus));

    let cohort = school.add_group(GroupSpec::new("Year 9", GroupKind::Cohort).sized(120));
    let class = school.add_group(GroupSpec::new("9A", GroupKind::Class).sized(28));
    // Rule 4: a parent contains its child, so the pair overlaps.
    school.declare_overlap(cohort, class, Overlap::Overlaps);

    let first = school.add_teacher(TeacherFactSpec::new("C. Carrow"));
    let second = school.add_teacher(TeacherFactSpec::new("D. Danforth"));

    let assembly = school.add_section(
        SectionSpec::new("Year 9 Assembly")
            .at(campus)
            .taught_by(first)
            .taught_to(cohort),
    );
    school.add_activities(assembly, &[1]);
    let maths = school.add_section(
        SectionSpec::new("9A Mathematics")
            .at(campus)
            .taught_by(second)
            .taught_to(class),
    );
    school.add_activities(maths, &[1]);

    (school.finish(), slots)
}

#[test]
fn a_parent_and_its_child_cannot_be_taught_at_once() {
    // ADR 0035 future acceptance test 3, and the one `student-clash` exists for.
    let (problem, slots) = cohort_and_class();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::StudentClash), 1);
    assert!(!report.is_usable());

    let violation = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::StudentClash)
        .expect("the clash");
    let message = evaluator_for(ConstraintType::StudentClash)
        .explain(&problem, violation)
        .message;
    assert!(message.contains("Year 9 Assembly"), "{message}");
    assert!(message.contains("9A Mathematics"), "{message}");
    assert!(
        message.contains("Year 9") || message.contains("9A"),
        "it names the group that attends both: {message}"
    );
}

#[test]
fn provably_disjoint_groups_may_be_taught_at_once() {
    // ADR 0035 future acceptance test 6: concurrency is legal and produces no violation and
    // no warning.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    // 9B Science shares the teacher, so swap the teacher out of the comparison by checking
    // only the student finding.
    assignment.place(a(2), Placement::in_room(slots[0], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(
        report.count_of(ConstraintType::StudentClash),
        0,
        "9A and 9B are declared disjoint"
    );
}

#[test]
fn an_indeterminate_pair_is_not_a_violation() {
    // ADR 0035 §6: "enforce what is proved". An undecided pair is neither a clash nor a
    // licence, and inventing a violation here would mean a year group could hold one lesson
    // at a time — the failure that record rejects by name.
    let mut school = SchoolBuilder::new(17, "Brackwell", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
    school.add_room(RoomFactSpec::new("Room 1", 40).on_campus(campus));
    school.add_room(RoomFactSpec::new("Room 2", 40).on_campus(campus));

    // Two sibling groups with nothing declared: Indeterminate, which the builder leaves as
    // the default for any pair it does not mention.
    let x = school.add_group(GroupSpec::new("Set X", GroupKind::TeachingSet).sized(28));
    let y = school.add_group(GroupSpec::new("Set Y", GroupKind::TeachingSet).sized(25));
    let first = school.add_teacher(TeacherFactSpec::new("E. Everly"));
    let second = school.add_teacher(TeacherFactSpec::new("F. Fenwick"));

    let left = school.add_section(
        SectionSpec::new("Set X Maths")
            .at(campus)
            .taught_by(first)
            .taught_to(x),
    );
    school.add_activities(left, &[1]);
    let right = school.add_section(
        SectionSpec::new("Set Y Maths")
            .at(campus)
            .taught_by(second)
            .taught_to(y),
    );
    school.add_activities(right, &[1]);

    let problem = school.finish();
    assert_eq!(
        problem.overlap.get(GroupIx::new(0), GroupIx::new(1)),
        Overlap::Indeterminate
    );

    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::StudentClash), 0);
    assert!(
        report.is_usable(),
        "an unproven pair must not refuse a timetable"
    );
}

#[test]
fn two_occurrences_of_one_section_do_not_clash_with_each_other() {
    // A section's audience overlaps itself by definition. Two of its occurrences in one slot
    // is a room clash and a teacher clash, and emphatically not a student clash: those are
    // the same people, and the rule is only consulted between different sections.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(1)));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::StudentClash),
        0
    );
}

// =======================================================================================
// room-type-required
// =======================================================================================

#[test]
fn a_lesson_needing_a_laboratory_may_not_sit_in_a_classroom() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    // Activity 2 is the science section, which requires the laboratory (room index 1).
    assignment.place(a(2), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::RoomTypeRequired), 1);
    let violation = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::RoomTypeRequired)
        .expect("the finding");
    let message = evaluator_for(ConstraintType::RoomTypeRequired)
        .explain(&problem, violation)
        .message;
    assert!(message.contains("Science Laboratory"), "{message}");
    assert!(message.contains("Room 1"), "{message}");
}

#[test]
fn the_right_room_type_satisfies_the_requirement() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(2), Placement::in_room(slots[0], RoomIx::new(1)));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::RoomTypeRequired),
        0
    );
}

#[test]
fn a_required_room_type_with_no_room_booked_is_reported() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(2), Placement::unroomed(slots[0]));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::RoomTypeRequired), 1);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.detail.contains("no room is booked"))
    );
}

#[test]
fn a_lesson_with_no_requirement_and_no_room_is_not_reported() {
    // ADR 0034 §4: a lesson with no room is preferable to one in a room of unverified size,
    // and this rule has no opinion about a section that requires nothing.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::unroomed(slots[0]));

    assert_eq!(
        evaluate(&problem, &assignment, &rules()).count_of(ConstraintType::RoomTypeRequired),
        0
    );
}

#[test]
fn an_unbookable_room_satisfies_nothing() {
    let mut school = SchoolBuilder::new(19, "Oakhanger", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
    let studio = school.add_room_type("Drama Studio");
    let closed = school.add_room(
        RoomFactSpec::new("Studio 2 (closed)", 30)
            .on_campus(campus)
            .of_type(studio)
            .not_bookable(),
    );
    let teacher = school.add_teacher(TeacherFactSpec::new("G. Garrick"));
    let group = school.add_group(GroupSpec::new("9A", GroupKind::Class).sized(28));
    let section = school.add_section(
        SectionSpec::new("9A Drama")
            .at(campus)
            .taught_by(teacher)
            .taught_to(group)
            .requires_room_type(studio),
    );
    school.add_activities(section, &[1]);

    let problem = school.finish();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], closed));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::RoomTypeRequired), 1);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.detail.contains("not bookable")),
        "{:?}",
        report.violations
    );
}

// =======================================================================================
// entity-unavailable
// =======================================================================================

/// `two_sections`, with the teacher unavailable in the first slot and the room in the second.
fn with_restrictions() -> (Problem, Vec<TimeslotIx>) {
    let mut school = SchoolBuilder::new(23, "Rookwood", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
    let room = school.add_room(RoomFactSpec::new("Room 1", 30).on_campus(campus));
    school.add_room(RoomFactSpec::new("Room 2", 30).on_campus(campus));
    let teacher = school.add_teacher(TeacherFactSpec::new("H. Hallam"));
    let group = school.add_group(GroupSpec::new("8A", GroupKind::Class).sized(26));
    let section = school.add_section(
        SectionSpec::new("8A Music")
            .at(campus)
            .taught_by(teacher)
            .taught_to(group),
    );
    school.add_activities(section, &[1, 1, 1]);

    school.restrict(
        AvailabilityEntity::Teacher { teacher },
        slots[0],
        AvailabilityStatus::Unavailable,
    );
    school.restrict(
        AvailabilityEntity::Room { room },
        slots[1],
        AvailabilityStatus::Unavailable,
    );
    school.restrict(
        AvailabilityEntity::Group { group },
        slots[3],
        AvailabilityStatus::Unavailable,
    );
    // Soft, and therefore not this rule's business.
    school.restrict(
        AvailabilityEntity::Teacher { teacher },
        slots[4],
        AvailabilityStatus::Discouraged,
    );

    (school.finish(), slots)
}

#[test]
fn an_unavailable_teacher_group_or_room_is_a_hard_violation() {
    let (problem, slots) = with_restrictions();

    for (slot, expected) in [
        (slots[0], "H. Hallam"),
        (slots[1], "Room 1"),
        (slots[3], "8A"),
    ] {
        let mut assignment = nothing(&problem);
        assignment.place(a(0), Placement::in_room(slot, RoomIx::new(0)));
        let report = evaluate(&problem, &assignment, &rules());
        let found: Vec<&str> = report
            .violations
            .iter()
            .filter(|v| v.constraint_type == ConstraintType::EntityUnavailable)
            .map(|v| v.subject.label.as_str())
            .collect();
        assert!(
            found.contains(&expected),
            "expected {expected} to be reported unavailable, got {found:?}"
        );
    }
}

#[test]
fn a_discouraged_slot_is_legal() {
    // "Do not treat DISCOURAGED as hard unavailable." It is soft, and the soft half of
    // availability — `preferred-time` — is not in the MVP.
    let (problem, slots) = with_restrictions();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[4], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::EntityUnavailable), 0);
    assert!(report.is_usable());
}

#[test]
fn an_entity_with_no_rule_is_available_everywhere() {
    // `V11` stores only exceptions, so an absent rule is not a restriction.
    let (problem, slots) = with_restrictions();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[3], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    let rooms: Vec<&str> = report
        .violations
        .iter()
        .filter(|v| v.constraint_type == ConstraintType::EntityUnavailable)
        .map(|v| v.subject.label.as_str())
        .collect();
    assert!(
        !rooms.contains(&"Room 1"),
        "Room 1 is only restricted in period 2: {rooms:?}"
    );
}

// =======================================================================================
// duration-contiguous and no-teaching-in-period
// =======================================================================================

/// A school whose only section needs a double period.
fn double_period() -> (Problem, Vec<TimeslotIx>) {
    let mut school = SchoolBuilder::new(29, "Eastvale", "Week", CycleKind::Custom);
    let campus = school.add_campus("Main");
    school.add_cycle_days(&["Day 1"], 1);
    let schedule = school.add_bell_schedule("Day", Some(campus), &standard_day());
    let slots = school.materialise(schedule);
    school.add_room(RoomFactSpec::new("Hall", 60).on_campus(campus));
    let teacher = school.add_teacher(TeacherFactSpec::new("I. Ingram"));
    let group = school.add_group(GroupSpec::new("10A", GroupKind::Class).sized(30));
    let section = school.add_section(
        SectionSpec::new("10A Games")
            .at(campus)
            .taught_by(teacher)
            .taught_to(group),
    );
    school.add_activities(section, &[2]);
    (school.finish(), slots)
}

#[test]
fn a_double_period_in_two_consecutive_teaching_periods_is_legal() {
    let (problem, slots) = double_period();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::DurationContiguous), 0);
    assert!(report.is_adoptable(), "{:?}", report.violations);
}

#[test]
fn a_double_period_may_not_span_a_break() {
    // ADR 0033 §5: a run is broken by any non-teaching period between two teaching ones.
    // Period 2 is 10:00–11:00 and the break follows it, so starting at period 2 reaches
    // across it.
    let (problem, slots) = double_period();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[1], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::DurationContiguous), 1);
    let violation = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::DurationContiguous)
        .expect("the finding");
    assert!(violation.detail.contains("Break"), "{}", violation.detail);
    assert!(!report.is_usable());
}

#[test]
fn a_school_that_permits_a_break_span_gets_no_violation() {
    // The MVP's one parameter, doing what it says. `CONSTRAINTS.md`: "not spanning a break
    // unless permitted".
    let (problem, slots) = double_period();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[1], RoomIx::new(0)));

    let mut permissive = rules();
    for rule in &mut permissive {
        if rule.constraint_type == ConstraintType::DurationContiguous {
            rule.options.allow_break_span = true;
        }
    }
    assert_eq!(
        evaluate(&problem, &assignment, &permissive).count_of(ConstraintType::DurationContiguous),
        0
    );
}

#[test]
fn a_double_period_may_not_run_off_the_end_of_the_day() {
    let (problem, slots) = double_period();
    let mut assignment = nothing(&problem);
    // Period 4 is the last of the day, so there is no second slot to take.
    assignment.place(a(0), Placement::in_room(slots[4], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::DurationContiguous), 1);
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.detail.contains("only 1 remain")),
        "{:?}",
        report.violations
    );
}

#[test]
fn a_lesson_in_a_break_is_refused_and_a_lesson_in_a_teaching_period_is_not() {
    let (problem, _) = two_sections();
    let day_slots = day(&problem, 1);

    let mut in_break = nothing(&problem);
    in_break.place(a(0), Placement::in_room(day_slots[2], RoomIx::new(0)));
    let report = evaluate(&problem, &in_break, &rules());
    assert_eq!(report.count_of(ConstraintType::NoTeachingInPeriod), 1);
    let violation = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::NoTeachingInPeriod)
        .expect("the finding");
    let message = evaluator_for(ConstraintType::NoTeachingInPeriod)
        .explain(&problem, violation)
        .message;
    assert!(message.contains("Break"), "{message}");
    assert!(message.contains("not a teaching period"), "{message}");

    let mut teaching = nothing(&problem);
    teaching.place(a(0), Placement::in_room(day_slots[0], RoomIx::new(0)));
    assert_eq!(
        evaluate(&problem, &teaching, &rules()).count_of(ConstraintType::NoTeachingInPeriod),
        0
    );
}

// =======================================================================================
// Invalid references
// =======================================================================================

#[test]
fn an_assignment_naming_an_activity_the_problem_does_not_hold_is_unreadable() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = Assignment::empty(problem.activities.len() + 4);
    assignment.place(a(5), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert!(!report.is_readable());
    assert!(
        !report.is_usable(),
        "an unreadable assignment is not usable"
    );
    assert!(
        report.violations.is_empty(),
        "and it is not scored — a number about nothing looks like an answer"
    );
    assert!(report.invalid_references[0].contains("activity #5"));
}

#[test]
fn an_assignment_naming_a_timeslot_the_grid_does_not_hold_is_unreadable() {
    let (problem, _) = two_sections();
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::unroomed(TimeslotIx::new(9_999)));

    let report = evaluate(&problem, &assignment, &rules());
    assert!(!report.is_readable());
    assert!(report.invalid_references[0].contains("timeslot"));
}

// =======================================================================================
// The incremental-friendly entry point
// =======================================================================================

#[test]
fn evaluating_one_placement_runs_only_the_rules_that_depend_on_it() {
    // `room-type-required` is entirely local, so moving one lesson cannot change its verdict
    // about another. `activity-scheduled` depends on everything.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    // Activity 2 is in the wrong room; activity 0 is fine.
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(slots[1], RoomIx::new(0)));

    let full = evaluate(&problem, &assignment, &rules());
    assert_eq!(full.count_of(ConstraintType::RoomTypeRequired), 1);

    // Asking about activity 0 still surfaces the room-type finding, because the rule it
    // belongs to was selected and then evaluated in full — the documented stopping point.
    // What it must not do is miss a rule that *does* depend on activity 0.
    let about_zero = evaluate_placement(&problem, &assignment, &rules(), a(0));
    assert!(
        about_zero.count_of(ConstraintType::TeacherClash) == 0,
        "the two are adjacent, so there is no clash to find"
    );
    assert!(
        about_zero.has(ConstraintType::ActivityScheduled),
        "a rule about the whole timetable is always selected"
    );
}

#[test]
fn a_rule_that_cannot_be_affected_is_not_selected() {
    let (problem, _) = two_sections();
    // `room-type-required`'s affected set is the moved activity alone, so asking about a
    // different activity must not select it.
    let local = evaluator_for(ConstraintType::RoomTypeRequired).affected(
        &problem,
        &CompiledConstraint::mandatory(ConstraintType::RoomTypeRequired),
        a(0),
    );
    assert!(local.contains(a(0)));
    assert!(!local.contains(a(2)));

    // `activity-scheduled` depends on every occurrence.
    let global = evaluator_for(ConstraintType::ActivityScheduled).affected(
        &problem,
        &CompiledConstraint::mandatory(ConstraintType::ActivityScheduled),
        a(0),
    );
    assert!(global.contains(a(2)));
}

#[test]
fn teacher_clash_is_affected_only_by_occurrences_sharing_a_teacher() {
    let (problem, _) = two_sections();
    let affected = evaluator_for(ConstraintType::TeacherClash).affected(
        &problem,
        &CompiledConstraint::mandatory(ConstraintType::TeacherClash),
        a(0),
    );
    // All three sections here share one teacher, so all three are affected.
    assert!(affected.contains(a(0)));
    assert!(affected.contains(a(2)));
}

// =======================================================================================
// Levels, scoring and explanations
// =======================================================================================

#[test]
fn a_hard_violation_scores_hard_and_an_unplaced_activity_scores_medium() {
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));
    assignment.place(a(1), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert!(report.score.hard < 0, "a room and teacher clash");
    assert_eq!(report.score.medium, -1, "one occurrence still unplaced");
    assert_eq!(report.score.soft, 0, "nothing soft is implemented");
    assert!(
        report.score < evara_core::scheduling::Score::PERFECT,
        "a broken arrangement must order below a clean one, which is what makes `Ord` the          comparison a search can use"
    );
}

#[test]
fn every_violation_can_explain_itself_and_names_its_subject() {
    // The discipline M3's wording tests found four real defects with, applied to all eight
    // rules at once: a message that does not name what it is about cannot be acted on.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[2], RoomIx::new(1)));
    assignment.place(a(2), Placement::in_room(slots[2], RoomIx::new(1)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_ne!(
        report.violations,
        Vec::new(),
        "the arrangement above breaks three rules at once"
    );
    for explanation in report.explanations(&problem) {
        assert!(
            explanation.message.len() > 25,
            "{}: {}",
            explanation.constraint_type.key(),
            explanation.message
        );
        let subject = &explanation.entities[0].label;
        assert!(
            explanation.message.contains(subject.as_str()),
            "{} does not name its subject {subject:?}: {}",
            explanation.constraint_type.key(),
            explanation.message
        );
        assert!(
            !explanation.conflicting_activities.is_empty(),
            "{} highlights nothing",
            explanation.constraint_type.key()
        );
    }
}

#[test]
fn violations_are_reported_in_registry_order() {
    // Determinism a reader can rely on: causes before consequences, and the same order twice.
    let (problem, _) = two_sections();
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[2], RoomIx::new(0)));
    assignment.place(a(2), Placement::in_room(slots[2], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    let order: Vec<usize> = report
        .violations
        .iter()
        .map(|v| {
            ConstraintType::all()
                .iter()
                .position(|k| *k == v.constraint_type)
                .expect("a registered type")
        })
        .collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "{order:?}");
}

// =======================================================================================
// activity-fixed
// =======================================================================================

/// `two_sections`, with activity 0 pinned to the first slot of day 1 in Room 1.
///
/// Pins are one entry per activity, which is what `Problem::integrity` requires of a non-empty
/// list — a half-length list would make the rule's lookup silently miss the tail.
fn with_a_pin(problem: &mut Problem, at: Placement) {
    problem.pinned = vec![None; problem.activities.len()];
    problem.pinned[0] = Some(at);
}

#[test]
fn a_pinned_occurrence_placed_where_it_is_pinned_breaks_nothing() {
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    let pin = Placement::in_room(slots[0], RoomIx::new(0));
    with_a_pin(&mut problem, pin);

    let mut assignment = nothing(&problem);
    assignment.place(a(0), pin);

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(
        report.count_of(ConstraintType::ActivityFixed),
        0,
        "the pin is honoured"
    );
}

#[test]
fn a_pinned_occurrence_placed_elsewhere_is_a_hard_violation() {
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    with_a_pin(&mut problem, Placement::in_room(slots[0], RoomIx::new(0)));

    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[1], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::ActivityFixed), 1);
    let found = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::ActivityFixed)
        .expect("the violation");
    assert_eq!(found.level, ConstraintLevel::Hard);
    assert_eq!(found.activities, vec![a(0)], "exactly one occurrence named");
    assert!(found.detail.contains("locked to"), "{}", found.detail);
}

#[test]
fn a_pin_names_the_room_as_well_as_the_slot() {
    // A lock protects a `Placement`, which is the unit the solver moves. A repair that kept
    // the period and took the room away has moved the lesson.
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    with_a_pin(&mut problem, Placement::in_room(slots[0], RoomIx::new(0)));

    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::unroomed(slots[0]));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(
        report.count_of(ConstraintType::ActivityFixed),
        1,
        "same slot, no room, is not where it was pinned"
    );
}

#[test]
fn an_unplaced_pinned_occurrence_is_incomplete_and_not_infeasible() {
    // The decision in this rule's shape. If a pin fired on an unplaced occurrence, the empty
    // arrangement of a pinned problem would be infeasible and so would every intermediate
    // construction state — which is the failure `activity-scheduled`'s MEDIUM level exists to
    // prevent, and a pin is not a licence to reintroduce it.
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    with_a_pin(&mut problem, Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &nothing(&problem), &rules());
    assert_eq!(report.count_of(ConstraintType::ActivityFixed), 0);
    assert!(
        report.is_usable(),
        "nothing placed breaks no hard rule, pins included"
    );
    assert_eq!(
        report.count_of(ConstraintType::ActivityScheduled),
        problem.activities.len(),
        "being nowhere is the completeness rule's business"
    );
}

#[test]
fn an_unpinned_occurrence_is_not_judged_by_the_pin_rule() {
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    with_a_pin(&mut problem, Placement::in_room(slots[0], RoomIx::new(0)));

    let mut assignment = nothing(&problem);
    // Activity 1 carries no pin, so wherever it goes this rule has no opinion.
    assignment.place(a(1), Placement::in_room(slots[3], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::ActivityFixed), 0);
}

#[test]
fn a_problem_with_no_pins_never_produces_the_rule() {
    // The common case — a generation from scratch — and the reason registering the rule costs
    // its callers nothing.
    let (problem, _) = two_sections();
    assert_eq!(problem.pinned.len(), 0, "the builder pins nothing");
    let slots = day(&problem, 1);
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[0], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    assert_eq!(report.count_of(ConstraintType::ActivityFixed), 0);
}

#[test]
fn the_pin_rule_is_affected_only_by_the_occurrence_that_moved() {
    let (problem, _) = two_sections();
    let affected = evaluator_for(ConstraintType::ActivityFixed).affected(
        &problem,
        &CompiledConstraint::mandatory(ConstraintType::ActivityFixed),
        a(0),
    );
    assert!(affected.contains(a(0)));
    assert!(!affected.contains(a(1)), "a pin is about one occurrence");
}

#[test]
fn the_pin_rule_explains_itself_by_naming_both_places() {
    let (mut problem, _) = two_sections();
    let slots = day(&problem, 1);
    with_a_pin(&mut problem, Placement::in_room(slots[0], RoomIx::new(0)));
    let mut assignment = nothing(&problem);
    assignment.place(a(0), Placement::in_room(slots[1], RoomIx::new(0)));

    let report = evaluate(&problem, &assignment, &rules());
    let found = report
        .violations
        .iter()
        .find(|v| v.constraint_type == ConstraintType::ActivityFixed)
        .expect("the violation");
    let said = evaluator_for(ConstraintType::ActivityFixed).explain(&problem, found);

    assert!(said.message.contains("locked"), "{}", said.message);
    assert!(said.message.contains("Day 1"), "{}", said.message);
    assert!(
        said.message.contains('1') && said.message.contains('2'),
        "both periods are named: {}",
        said.message
    );
    assert_eq!(
        said.conflicting_activities,
        vec![problem.activities[0].id],
        "the occurrence a screen must highlight"
    );
}
