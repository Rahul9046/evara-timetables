//! What a placement occupies, and what the resolver says when it cannot occupy it.
//!
//! These were `evara-constraints`' tests of `Occupancy::resolve` in everything but name: the
//! evaluators asserted the defects through the violations they produced. Now the measure has a
//! caller that is not a rule — M7's reader, which has to decide whether a *stored* timetable
//! still describes something placeable — the defects are asserted directly, here, where they
//! live.

use crate::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use crate::scheduling::{
    ActivityIx, BellScheduleResolution, CycleKind, GroupKind, Overlap, PeriodKind, Placement,
    Problem, SectionIx, TimeslotIx, resolve_activity_span, resolve_span,
};

/// Four teaching periods with a break between the second and third.
///
/// The break is what makes a broken run testable: a day of nothing but teaching periods cannot
/// demonstrate one.
fn standard_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("1", "09:00", "10:00"),
        PeriodSpec::teaching("2", "10:00", "11:00"),
        PeriodSpec::non_teaching("Break", "11:00", "11:30", PeriodKind::Break),
        PeriodSpec::teaching("3", "11:30", "12:30"),
        PeriodSpec::teaching("4", "13:30", "14:30"),
    ]
}

/// One school, one day, one section, and `durations.len()` occurrences.
fn school(durations: &[u16]) -> (Problem, Vec<TimeslotIx>) {
    let mut builder = SchoolBuilder::new(5, "Northgate", "Week", CycleKind::Custom);
    let campus = builder.add_campus("Main");
    builder.add_cycle_days(&["Day 1"], 1);
    let schedule = builder.add_bell_schedule("Standard Day", Some(campus), &standard_day());
    let slots = builder.materialise(schedule);
    builder.add_room(RoomFactSpec::new("Room 1", 30).on_campus(campus));
    let teacher = builder.add_teacher(TeacherFactSpec::new("A. Teacher"));
    let group = builder.add_group(GroupSpec::new("7A", GroupKind::Class).sized(26));
    builder.declare_overlap(group, group, Overlap::Overlaps);
    let section = builder.add_section(
        SectionSpec::new("Mathematics 7A")
            .schedule(BellScheduleResolution::Resolved { schedule })
            .taught_by(teacher)
            .taught_to(group),
    );
    builder.add_activities(section, durations);
    (builder.finish(), slots)
}

fn a(ix: u32) -> ActivityIx {
    ActivityIx::new(ix)
}

#[test]
fn a_single_period_occurrence_occupies_the_slot_it_was_placed_in() {
    let (problem, slots) = school(&[1]);
    let resolved = resolve_activity_span(&problem, a(0), Placement::unroomed(slots[0]))
        .expect("the slot is in the grid");

    assert_eq!(resolved.span.slots, vec![slots[0]]);
    assert_eq!(resolved.span.slot_count(), 1);
    assert!(resolved.defect.is_sound(), "{:?}", resolved.defect);
}

#[test]
fn a_double_period_occupies_two_consecutive_slots_and_the_interval_spans_both() {
    let (problem, slots) = school(&[2]);
    let resolved = resolve_activity_span(&problem, a(0), Placement::unroomed(slots[0]))
        .expect("the slot is in the grid");

    assert_eq!(resolved.span.slots, vec![slots[0], slots[1]]);
    assert_eq!(resolved.span.interval.start.minutes(), 9 * 60);
    assert_eq!(resolved.span.interval.end.minutes(), 11 * 60);
    assert!(resolved.defect.is_sound());
}

#[test]
fn a_double_reaching_into_a_break_is_resolved_and_the_defect_says_so() {
    // Permissive resolution: the span is still built. `duration-contiguous` is what refuses it,
    // and a reader that only wanted the slots gets them.
    let (problem, slots) = school(&[2]);
    let resolved = resolve_activity_span(&problem, a(0), Placement::unroomed(slots[1]))
        .expect("the slot is in the grid");

    assert_eq!(resolved.span.slot_count(), 2, "the span is built anyway");
    assert!(resolved.defect.crosses_non_teaching);
    assert!(!resolved.defect.is_sound());
    assert!(
        resolved.defect.is_complete(),
        "two slots were available; what is wrong is which ones"
    );
}

#[test]
fn a_run_that_falls_off_the_end_of_the_day_is_short_rather_than_refused() {
    let (problem, slots) = school(&[2]);
    let last = *slots.last().expect("the day has slots");
    let resolved = resolve_activity_span(&problem, a(0), Placement::unroomed(last))
        .expect("the slot is in the grid");

    assert_eq!(resolved.span.slot_count(), 1);
    assert!(!resolved.defect.is_complete());
    assert_eq!(resolved.defect.wanted, 2);
    assert_eq!(resolved.defect.got, 1);
}

#[test]
fn a_lesson_starting_in_a_break_is_resolved_and_flagged() {
    let (problem, slots) = school(&[1]);
    // Index 2 of the day is the break.
    let resolved = resolve_activity_span(&problem, a(0), Placement::unroomed(slots[2]))
        .expect("the slot is in the grid");

    assert!(resolved.defect.starts_outside_teaching);
    assert!(!resolved.defect.is_sound());
}

#[test]
fn a_timeslot_outside_the_grid_is_the_one_unresolvable_case() {
    // The only `None`. Everything else is a span with a defect, which is what keeps a damaged
    // timetable reportable instead of unreadable.
    let (problem, _) = school(&[1]);
    let past_the_end = TimeslotIx::new(u32::try_from(problem.timeslots.len()).unwrap_or(0));
    assert!(resolve_activity_span(&problem, a(0), Placement::unroomed(past_the_end)).is_none());
}

#[test]
fn an_activity_outside_the_problem_resolves_to_nothing() {
    let (problem, slots) = school(&[1]);
    let past_the_end = ActivityIx::new(u32::try_from(problem.activities.len()).unwrap_or(0));
    assert!(resolve_activity_span(&problem, past_the_end, Placement::unroomed(slots[0])).is_none());
}

#[test]
fn the_activity_form_and_the_explicit_form_agree() {
    // `resolve_activity_span` exists so no caller looks up the duration and the section itself
    // and none can look up the wrong ones. It must be the same answer.
    let (problem, slots) = school(&[2]);
    let fact = &problem.activities[0];
    assert_eq!(
        resolve_activity_span(&problem, a(0), Placement::unroomed(slots[0])),
        resolve_span(
            &problem,
            a(0),
            fact.section,
            fact.duration.as_usize(),
            Placement::unroomed(slots[0])
        )
    );
}

#[test]
fn a_zero_period_run_resolves_to_nothing_rather_than_an_empty_span() {
    // `PlacementSpan::over` refuses an empty slot list, which is a caller bug rather than a
    // data state. Asserted here because M7's reader passes a stored `duration_periods` through.
    let (problem, slots) = school(&[1]);
    assert!(
        resolve_span(
            &problem,
            a(0),
            SectionIx::new(0),
            0,
            Placement::unroomed(slots[0])
        )
        .is_none()
    );
}

#[test]
fn a_sound_defect_is_exactly_the_four_flags_being_clear() {
    use crate::scheduling::SpanDefect;

    assert!(
        SpanDefect {
            wanted: 2,
            got: 2,
            ..SpanDefect::default()
        }
        .is_sound()
    );
    for broken in [
        SpanDefect {
            wanted: 2,
            got: 1,
            ..SpanDefect::default()
        },
        SpanDefect {
            wanted: 1,
            got: 1,
            crosses_non_teaching: true,
            ..SpanDefect::default()
        },
        SpanDefect {
            wanted: 1,
            got: 1,
            starts_outside_teaching: true,
            ..SpanDefect::default()
        },
        SpanDefect {
            wanted: 1,
            got: 1,
            wrong_bell_schedule: true,
            ..SpanDefect::default()
        },
    ] {
        assert!(!broken.is_sound(), "{broken:?}");
    }
}
