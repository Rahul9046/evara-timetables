//! The cycle's declared length, enforced at both ends.
//!
//! A cycle day's `ordinal` is its whole scheduling identity (ADR 0011 §6), and
//! `cycle.day_count` is the declared length the rows need not yet fill (ADR 0011 Q4). So
//! `1..=day_count` is the range a position may occupy, and this file is about the
//! repository holding that line whoever asks — the interface, `evara-cli`, or a direct
//! IPC call from a console.
//!
//! Three refusals, each with its own reason:
//!
//! - **below 1** — a position counts from 1; 0 is not a short cycle but a nonsensical one
//! - **above `day_count`** — a day beyond the declared end is a contradiction
//! - **already taken** — `UNIQUE (cycle_id, ordinal)`, reported against the position field
//!
//! And one refusal in the other direction: a cycle cannot be shortened past a day it
//! already has. The alternative would be deleting or renumbering days to fit, which would
//! silently invalidate every timeslot and calendar mapping pointing at them.
//!
//! Real temporary project files throughout, never `:memory:`.

use uuid::Uuid;

use crate::DbError;
use crate::repo::structure::SchoolInput;
use crate::repo::time_model::{
    CalendarDayInput, CalendarDayKind, CycleDay, CycleDayInput, CycleInput, MaterialisationRequest,
};
use crate::tests_time_model::Project;

/// Attempts a cycle day, returning the failure rather than panicking on it.
fn try_day(
    project: &mut Project,
    cycle: Uuid,
    ordinal: i64,
    label: &str,
) -> Result<CycleDay, DbError> {
    project.db().time_model().create_cycle_day(&CycleDayInput {
        cycle_id: cycle,
        ordinal,
        label: label.to_owned(),
        weekday_hint: None,
    })
}

/// A cycle's days as `(ordinal, id, label, rev)`, in cycle order.
///
/// Carries `id` and `rev` on purpose: "existing records are unchanged" has to mean the
/// rows were not rewritten, not merely that they still exist.
fn snapshot(project: &mut Project, cycle: Uuid) -> Vec<(i64, Uuid, String, i64)> {
    project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("read days")
        .into_iter()
        .map(|day| (day.ordinal, day.id, day.label, day.rev))
        .collect()
}

/// Sets a cycle's declared length, returning the failure rather than panicking.
fn try_resize(project: &mut Project, cycle: Uuid, day_count: i64) -> Result<(), DbError> {
    let school = project
        .db()
        .structure()
        .school()
        .expect("read school")
        .expect("a school exists")
        .id;
    project
        .db()
        .time_model()
        .update_cycle(
            cycle,
            &CycleInput {
                school_id: school,
                name: "Rotation".to_owned(),
                day_count,
                week_count: 1,
                is_default: false,
            },
        )
        .map(|_| ())
}

// ======================================================================= a full cycle

#[test]
fn a_five_day_cycle_with_five_days_refuses_a_sixth() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    for ordinal in 1..=5 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let before = snapshot(&mut project, cycle);

    let outcome = try_day(&mut project, cycle, 6, "Day 6");

    match outcome {
        Err(DbError::Invalid { ref message }) => {
            assert!(
                message.contains("5 days long") && message.contains("no day 6"),
                "the refusal must say which cycle and which day: {message}"
            );
        }
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }

    // And nothing was written on the way to being refused.
    assert_eq!(snapshot(&mut project, cycle), before);
    assert_eq!(before.len(), 5);
}

#[test]
fn a_full_cycle_refuses_every_position_it_already_has() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    for ordinal in 1..=5 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let before = snapshot(&mut project, cycle);

    // Every in-range position is taken, so the only answer left is "that one exists".
    for ordinal in 1..=5 {
        match try_day(&mut project, cycle, ordinal, "Another") {
            Err(DbError::DuplicateValue { entity, field }) => {
                assert_eq!(entity, "cycle day");
                assert_eq!(field, "position", "the form must mark the position field");
            }
            other => panic!("expected a duplicate refusal for {ordinal}, got {other:?}"),
        }
    }

    assert_eq!(snapshot(&mut project, cycle), before);
}

// ==================================================================== a cycle with a gap

#[test]
fn a_five_day_cycle_with_four_days_accepts_the_missing_one() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    // Deliberately leaving position 3 empty rather than position 5: a gap in the middle
    // is the case a "count the rows" implementation gets wrong.
    for ordinal in [1, 2, 4, 5] {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let before = snapshot(&mut project, cycle);

    let created = try_day(&mut project, cycle, 3, "Day 3").expect("the gap is fillable");

    assert_eq!(created.ordinal, 3);
    assert_eq!(created.rev, 1);

    let after = snapshot(&mut project, cycle);
    assert_eq!(after.len(), 5);
    // In cycle order, with the new day in its place and every existing row untouched.
    assert_eq!(
        after.iter().map(|day| day.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    for existing in &before {
        assert!(
            after.contains(existing),
            "existing day {existing:?} was altered by filling the gap"
        );
    }
}

#[test]
fn filling_the_last_gap_then_refuses_anything_further() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 3);
    project.day(cycle, 1, "Day 1");
    project.day(cycle, 2, "Day 2");

    try_day(&mut project, cycle, 3, "Day 3").expect("the last gap");

    for ordinal in [0, 4, 99] {
        assert!(
            try_day(&mut project, cycle, ordinal, "Extra").is_err(),
            "{ordinal} must be refused once the cycle is full"
        );
    }
    assert_eq!(snapshot(&mut project, cycle).len(), 3);
}

// ===================================================================== the range itself

#[test]
fn a_position_below_one_is_refused_with_a_reason_a_user_can_act_on() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);

    for ordinal in [0, -1, i64::MIN] {
        match try_day(&mut project, cycle, ordinal, "Nowhere") {
            Err(DbError::Invalid { ref message }) => {
                assert!(
                    message.contains("counts from 1"),
                    "the refusal must explain the rule, not merely refuse: {message}"
                );
                // The schema's `CHECK (ordinal >= 1)` would also have stopped this, but
                // `translate` turns any CHECK failure into "that cycle day has a value the
                // project does not allow" -- true, and useless. This is why the range is
                // checked before the insert as well.
                assert!(
                    !message.contains("does not allow"),
                    "the generic CHECK message reached the caller: {message}"
                );
            }
            other => panic!("expected an Invalid refusal for {ordinal}, got {other:?}"),
        }
    }

    assert_eq!(snapshot(&mut project, cycle), Vec::new());
}

#[test]
fn a_position_past_the_declared_length_is_refused() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);

    for ordinal in [6, 7, 100, i64::MAX] {
        match try_day(&mut project, cycle, ordinal, "Beyond") {
            Err(DbError::Invalid { .. }) => {}
            other => panic!("expected an Invalid refusal for {ordinal}, got {other:?}"),
        }
    }

    assert_eq!(snapshot(&mut project, cycle), Vec::new());
}

#[test]
fn a_one_day_cycle_accepts_exactly_one_position() {
    let mut project = Project::new();
    let cycle = project.cycle("Every day", 1);

    try_day(&mut project, cycle, 1, "The day").expect("position 1");
    assert!(try_day(&mut project, cycle, 2, "The other day").is_err());
    assert!(try_day(&mut project, cycle, 0, "No day").is_err());
    assert_eq!(snapshot(&mut project, cycle).len(), 1);
}

#[test]
fn an_edit_is_range_checked_too_not_only_a_create() {
    // Requirement 6 is about what a direct IPC call can do, and `setup_cycle_day_update`
    // is as direct a route to an out-of-range ordinal as `create` is.
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    let day = project.day(cycle, 2, "Day 2");

    for ordinal in [0, -5, 6, 99] {
        let outcome = project.db().time_model().update_cycle_day(
            day,
            &CycleDayInput {
                cycle_id: cycle,
                ordinal,
                label: "Moved".to_owned(),
                weekday_hint: None,
            },
        );
        assert!(
            matches!(outcome, Err(DbError::Invalid { .. })),
            "moving a day to {ordinal} must be refused, got {outcome:?}"
        );
    }

    // Untouched by any of those attempts: still position 2, still rev 1, still named.
    let stored = project
        .db()
        .time_model()
        .cycle_day(day)
        .expect("read")
        .expect("still there");
    assert_eq!(stored.ordinal, 2);
    assert_eq!(stored.label, "Day 2");
    assert_eq!(stored.rev, 1);
}

// ================================================================== changing the length

#[test]
fn increasing_the_length_opens_the_positions_it_adds() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    for ordinal in 1..=5 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let before = snapshot(&mut project, cycle);
    assert!(try_day(&mut project, cycle, 6, "Day 6").is_err());

    try_resize(&mut project, cycle, 6).expect("lengthening is always safe");

    let created = try_day(&mut project, cycle, 6, "Day 6").expect("position 6 is now real");
    assert_eq!(created.ordinal, 6);

    // The five that already existed are byte-for-byte what they were: lengthening a cycle
    // renumbers nothing.
    let after = snapshot(&mut project, cycle);
    assert_eq!(after.len(), 6);
    for existing in &before {
        assert!(after.contains(existing), "day {existing:?} was altered");
    }
    // And position 7 is still refused.
    assert!(try_day(&mut project, cycle, 7, "Day 7").is_err());
}

#[test]
fn reducing_the_length_below_an_occupied_position_is_refused() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 6);
    for ordinal in 1..=6 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let before = snapshot(&mut project, cycle);

    match try_resize(&mut project, cycle, 5) {
        Err(DbError::Invalid { ref message }) => {
            assert!(
                message.contains("day 6") && message.contains("cannot be reduced"),
                "the refusal must name the day in the way: {message}"
            );
        }
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }

    // Nothing was deleted and nothing was renumbered to make room -- requirement 9.
    assert_eq!(snapshot(&mut project, cycle), before);
    let cycle_row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("still there");
    assert_eq!(
        cycle_row.day_count, 6,
        "and the length did not change either"
    );
}

#[test]
fn reducing_the_length_to_the_highest_occupied_position_is_allowed() {
    // The boundary: a 6-day cycle whose days stop at 4 may be shortened to 4, but not 3.
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 6);
    for ordinal in 1..=4 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }

    try_resize(&mut project, cycle, 4).expect("shortening to the highest day is fine");
    assert!(try_resize(&mut project, cycle, 3).is_err());

    let cycle_row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("still there");
    assert_eq!(cycle_row.day_count, 4);
    // And the now-full cycle refuses a fifth day.
    assert!(try_day(&mut project, cycle, 5, "Day 5").is_err());
}

#[test]
fn an_empty_cycle_can_be_shortened_freely() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 10);

    try_resize(&mut project, cycle, 1).expect("no days, nothing in the way");
    assert!(try_day(&mut project, cycle, 2, "Day 2").is_err());
    try_day(&mut project, cycle, 1, "The day").expect("position 1");
}

// ============================================== what the limit must never disturb

#[test]
fn a_refused_day_disturbs_no_calendar_mapping_or_timeslot() {
    let mut project = Project::new();
    let (cycle, structure, days, _periods) = project.six_day_grid();
    // `six_day_grid` builds the two axes; the slots themselves have to be asked for.
    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, None))
        .expect("materialise");
    let school = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;

    // A real date pointing at a real cycle day, which is the thing a renumbering would
    // silently invalidate.
    project
        .db()
        .time_model()
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-09-01".to_owned(),
            term_id: None,
            campus_id: None,
            cycle_day_id: Some(days[0]),
            kind: CalendarDayKind::School,
            note: None,
        })
        .expect("calendar day");

    let days_before = snapshot(&mut project, cycle);
    let slots_before = project.slots(cycle, None);
    let calendar_before = project
        .db()
        .time_model()
        .calendar_days("0000-01-01", "9999-12-31")
        .expect("read calendar");
    assert_ne!(slots_before, Vec::new());

    // Every refusal this file is about, one after another.
    assert!(try_day(&mut project, cycle, 0, "No").is_err());
    assert!(try_day(&mut project, cycle, 7, "No").is_err());
    assert!(try_day(&mut project, cycle, 1, "No").is_err());
    assert!(try_resize(&mut project, cycle, 2).is_err());

    assert_eq!(snapshot(&mut project, cycle), days_before);
    assert_eq!(project.slots(cycle, None), slots_before);
    assert_eq!(
        project
            .db()
            .time_model()
            .calendar_days("0000-01-01", "9999-12-31")
            .expect("read calendar"),
        calendar_before
    );
}

#[test]
fn the_limit_is_per_cycle_not_per_school() {
    let mut project = Project::new();
    let short = project.cycle("Weekly", 2);
    let long = project.cycle("Fortnight", 10);

    project.day(short, 1, "Mon");
    project.day(short, 2, "Tue");
    assert!(try_day(&mut project, short, 3, "Wed").is_err());

    // The other cycle is unaffected by its neighbour being full.
    project.day(long, 3, "Day 3");
    try_day(&mut project, long, 10, "Day 10").expect("a different cycle, a different limit");
    assert!(try_day(&mut project, long, 11, "Day 11").is_err());

    assert_eq!(snapshot(&mut project, short).len(), 2);
    // Positions 3 and 10, deliberately leaving the rest of the fortnight empty: a cycle
    // with gaps is a legal intermediate state (ADR 0011 Q4).
    assert_eq!(snapshot(&mut project, long).len(), 2);
}

#[test]
fn a_cycle_day_against_a_cycle_that_does_not_exist_is_a_reference_failure() {
    // The range check reads `day_count` and finds no row. It must not invent a limit, and
    // must leave the foreign key to report what is actually wrong.
    let mut project = Project::new();
    let outcome = try_day(&mut project, Uuid::nil(), 1, "Orphan");
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "expected a reference failure, got {outcome:?}"
    );
}

#[test]
fn the_school_fixture_is_untouched_by_all_of_this() {
    // A guard on the helpers rather than on the rule: `try_resize` writes a cycle name, so
    // this makes sure the tests above are not quietly editing the school record too.
    let mut project = Project::new();
    let before = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");
    let cycle = project.cycle("Rotation", 2);
    project.day(cycle, 1, "Day 1");
    assert!(try_day(&mut project, cycle, 3, "Day 3").is_err());

    let after = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");
    assert_eq!(after.rev, before.rev);
    assert_eq!(after.updated_at, before.updated_at);
    let _ = SchoolInput {
        name: after.name.clone(),
        timezone: after.timezone.clone(),
        locale: after.locale.clone(),
        working_days: after.working_days,
    };
}
