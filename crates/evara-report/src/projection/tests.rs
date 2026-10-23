//! Projection tests, driven by M2's deterministic synthetic catalogue.
//!
//! The suite is built around one invariant — [`GridPanel::is_well_formed`] — because the
//! requirement on this module is not *"the common case renders"* but *"no arrangement
//! renders as a broken table"*, and the arrangements that break it are the ones a stored
//! timetable acquires after its inputs move.

use std::collections::{BTreeMap, BTreeSet};

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
use evara_core::scheduling::fixtures::{all_fixtures, fixture, fixture_names};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleIx, CycleDayIx, Placement, Problem, TimeslotIx,
};

use super::*;

/// The seed every test here uses, matching the rest of the workspace.
const SEED: u64 = 2026;

/// A label set covering every section and room of a problem.
///
/// Built from the problem so the tests exercise the join rather than a hand-written table,
/// and so a section the loader interned cannot be missing from the labels by accident.
fn labels_for(problem: &Problem) -> LabelSet {
    LabelSet {
        sections: problem
            .sections
            .iter()
            .map(|section| SectionLabel {
                section_id: section.id,
                subject: "Subject".to_owned(),
                course: format!("Course for {}", section.name),
                section: section.name.clone(),
                year_level: Some("9".to_owned()),
                campus: None,
                short: SectionLabel::compose_short(Some("9"), "Subject", &section.name),
            })
            .collect(),
        rooms: problem
            .rooms
            .iter()
            .map(|room| RoomLabel {
                room_id: room.id,
                name: room.name.clone(),
                building: Some("Main".to_owned()),
            })
            .collect(),
        students: Vec::new(),
    }
}

/// Projects a fixture's own known-feasible arrangement, where it ships one.
fn project_feasible(name: &str) -> Option<(Problem, LabelSet, TimetableProjection)> {
    let found = fixture(name, SEED)?;
    let feasible = found.expectations.feasible.as_ref()?;
    let problem = found.problems.get(feasible.problem)?.clone();
    let assignment = feasible.assignment(problem.activity_count());
    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    Some((problem, labels, projection))
}

/// Lays out every scope of a projection, for the invariant sweeps.
fn every_grid(projection: &TimetableProjection, labels: &LabelSet) -> Vec<ScopeGrid> {
    projection
        .scopes
        .iter()
        .map(|scope| lay_out(projection, scope, labels))
        .collect()
}

// ---------------------------------------------------------------- the invariant

#[test]
fn every_scope_of_every_fixture_renders_a_well_formed_table() {
    let mut checked = 0_usize;
    for found in all_fixtures(SEED) {
        let Some(feasible) = found.expectations.feasible.as_ref() else {
            continue;
        };
        let problem = &found.problems[feasible.problem];
        let assignment = feasible.assignment(problem.activity_count());
        let labels = labels_for(problem);
        let flags = no_flags();
        let projection = project(ProjectionInput {
            problem,
            assignment: &assignment,
            labels: &labels,
            flags: &flags,
        });
        for grid in every_grid(&projection, &labels) {
            for panel in &grid.panels {
                let problems = panel.malformations();
                assert!(
                    problems.is_empty(),
                    "{} / {:?} {:?}: {problems:?}",
                    found.name,
                    grid.scope.kind,
                    grid.scope.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no fixture ships a feasible arrangement");
}

#[test]
fn a_cell_exists_for_every_day_and_period_of_a_panel() {
    let (_, labels, projection) = project_feasible("tiny").expect("tiny is feasible");
    let grid = lay_out(&projection, &projection.scopes[0], &labels);
    for panel in &grid.panels {
        assert_eq!(
            panel.cells.len(),
            panel.days.len() * panel.periods.len(),
            "a panel is a complete rectangle"
        );
        for day in &panel.days {
            for period in &panel.periods {
                assert!(
                    panel.cell(day.ordinal, period.ordinal).is_some(),
                    "day {} period {} is missing",
                    day.ordinal,
                    period.ordinal
                );
            }
        }
    }
}

// ---------------------------------------------------------------- conflicts

/// Places two occurrences of the same problem in the same slot, whatever the fixture said.
///
/// The arrangement a stored timetable acquires when its inputs move: nothing referential
/// breaks, every identifier still resolves, and two lessons now sit on top of one another.
fn two_lessons_in_one_slot(problem: &Problem) -> Option<(Assignment, ActivityIx, ActivityIx)> {
    let slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, s)| s.is_teaching)
        .and_then(|(ix, _)| u32::try_from(ix).ok())
        .map(TimeslotIx::new)?;
    // Two single-period occurrences, so the overlap is exact rather than partial.
    let mut singles = problem
        .activities
        .iter()
        .enumerate()
        .filter(|(_, a)| a.duration.as_usize() == 1)
        .map(|(ix, _)| ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX)));
    let first = singles.next()?;
    let second = singles.next()?;
    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        first,
        Placement {
            timeslot: slot,
            room: None,
        },
    );
    assignment.place(
        second,
        Placement {
            timeslot: slot,
            room: None,
        },
    );
    Some((assignment, first, second))
}

#[test]
fn two_single_period_lessons_in_one_slot_are_pinned_and_neither_is_discarded() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let (assignment, _, _) =
        two_lessons_in_one_slot(&problem).expect("tiny has two single-period occurrences");
    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 2, "both lessons are projected");

    // The master grid expects several lessons per cell, so this is Shared, not a conflict.
    let master = lay_out(&projection, &projection.scopes[0], &labels);
    assert!(master.is_well_formed());
    assert!(
        master.conflicts.is_empty(),
        "the master grid does not call a shared cell a clash"
    );
    let pinned: Vec<_> = master
        .panels
        .iter()
        .flat_map(|p| &p.cells)
        .filter_map(|c| match &c.content {
            CellContent::Pinned { lessons, reason } => Some((lessons.clone(), *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(pinned.len(), 1, "one cell holds both");
    assert_eq!(pinned[0].0.len(), 2, "both lessons are reachable from it");
    assert_eq!(pinned[0].1, PinReason::Shared);
}

#[test]
fn a_room_double_booking_is_a_conflict_and_the_table_stays_well_formed() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let room = problem
        .rooms
        .iter()
        .enumerate()
        .find(|(_, r)| r.is_bookable)
        .map(|(ix, r)| {
            (
                evara_core::scheduling::RoomIx::new(u32::try_from(ix).unwrap()),
                r.id,
            )
        })
        .expect("tiny has a bookable room");
    let slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, s)| s.is_teaching)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap()))
        .expect("tiny has a teaching slot");

    let mut singles = problem
        .activities
        .iter()
        .enumerate()
        .filter(|(_, a)| a.duration.as_usize() == 1)
        .map(|(ix, _)| ActivityIx::new(u32::try_from(ix).unwrap()));
    let mut assignment = Assignment::empty(problem.activity_count());
    for activity in [
        singles.next().expect("one"),
        singles.next().expect("another"),
    ] {
        assignment.place(
            activity,
            Placement {
                timeslot: slot,
                room: Some(room.0),
            },
        );
    }

    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Room && s.id == Some(room.1))
        .expect("the room is a scope")
        .clone();
    let grid = lay_out(&projection, &scope, &labels);

    assert!(
        grid.is_well_formed(),
        "{:?}",
        grid.panels[0].malformations()
    );
    assert_eq!(grid.conflicts.len(), 1, "one clash is reported");
    assert_eq!(
        grid.conflicts[0].lessons.len(),
        2,
        "both lessons are named by the clash"
    );
    assert!(
        !grid.conflicts[0].day_label.is_empty() && !grid.conflicts[0].period_label.is_empty(),
        "a clash names its position in words"
    );
    let pinned = grid.panels.iter().flat_map(|p| &p.cells).any(|c| {
        matches!(
            &c.content,
            CellContent::Pinned {
                reason: PinReason::Conflict,
                ..
            }
        )
    });
    assert!(pinned, "the cell is pinned as a conflict");
}

#[test]
fn a_multi_period_lesson_overlapping_a_single_one_pins_every_cell_it_touches() {
    // A `d > 1` occurrence and a single one placed inside its run. The multi-period lesson
    // must lose its rowSpan entirely: a span across a pinned cell is the broken table this
    // whole design exists to prevent.
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();

    let multi = problem
        .activities
        .iter()
        .enumerate()
        .find(|(_, a)| a.duration.as_usize() > 1)
        .map(|(ix, a)| {
            (
                ActivityIx::new(u32::try_from(ix).unwrap()),
                a.duration.as_usize(),
            )
        });
    let Some((multi, duration)) = multi else {
        // Not every fixture ships a double period; the dedicated multi-period fixture does.
        return;
    };
    let single = problem
        .activities
        .iter()
        .enumerate()
        .find(|(_, a)| a.duration.as_usize() == 1)
        .map(|(ix, _)| ActivityIx::new(u32::try_from(ix).unwrap()))
        .expect("tiny has a single-period occurrence");

    // Place the long one at the first teaching slot of a day, then put the single one on the
    // second slot of the same run.
    let day = problem
        .timeslots
        .first()
        .map(|s| s.cycle_day)
        .expect("a slot");
    let schedule = problem
        .timeslots
        .first()
        .map(|s| s.bell_schedule)
        .expect("a slot");
    let run = problem.slots_of(schedule, day);
    if run.len() <= duration {
        return;
    }
    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        multi,
        Placement {
            timeslot: run[0],
            room: None,
        },
    );
    assignment.place(
        single,
        Placement {
            timeslot: run[1],
            room: None,
        },
    );

    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let master = lay_out(&projection, &projection.scopes[0], &labels);
    assert!(
        master.is_well_formed(),
        "{:?}",
        master.panels.first().map(GridPanel::malformations)
    );
    let spans_anything = master
        .panels
        .iter()
        .flat_map(|p| &p.cells)
        .any(|c| matches!(c.content, CellContent::Lesson { rows, .. } if rows > 1));
    assert!(
        !spans_anything,
        "an overlapped multi-period lesson keeps no rowSpan"
    );
}

#[test]
fn a_teacher_in_two_places_at_once_is_a_conflict_in_their_own_grid() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    // Two occurrences of sections that share a teacher, placed in one slot.
    let mut by_teacher: BTreeMap<_, Vec<ActivityIx>> = BTreeMap::new();
    for (ix, activity) in problem.activities.iter().enumerate() {
        if activity.duration.as_usize() != 1 {
            continue;
        }
        let Some(section) = problem.sections.get(activity.section.get()) else {
            continue;
        };
        for teacher in &section.teachers {
            by_teacher
                .entry(*teacher)
                .or_default()
                .push(ActivityIx::new(u32::try_from(ix).unwrap()));
        }
    }
    let Some((index, activities)) = by_teacher.iter().find(|(_, v)| v.len() >= 2) else {
        return;
    };
    let teacher_id = problem.teachers[index.get()].id;
    let slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, s)| s.is_teaching)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap()))
        .expect("a teaching slot");

    let mut assignment = Assignment::empty(problem.activity_count());
    for activity in activities.iter().take(2) {
        assignment.place(
            *activity,
            Placement {
                timeslot: slot,
                room: None,
            },
        );
    }

    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Teacher && s.id == Some(teacher_id))
        .expect("the teacher is a scope")
        .clone();
    let grid = lay_out(&projection, &scope, &labels);
    assert!(grid.is_well_formed());
    assert_eq!(grid.conflicts.len(), 1);
    assert_eq!(grid.conflicts[0].lessons.len(), 2);
}

// ---------------------------------------------------------------- spans

#[test]
fn a_sound_multi_period_lesson_spans_exactly_its_duration() {
    for found in all_fixtures(SEED) {
        let Some(feasible) = found.expectations.feasible.as_ref() else {
            continue;
        };
        let problem = &found.problems[feasible.problem];
        let assignment = feasible.assignment(problem.activity_count());
        let labels = labels_for(problem);
        let flags = no_flags();
        let projection = project(ProjectionInput {
            problem,
            assignment: &assignment,
            labels: &labels,
            flags: &flags,
        });
        for lesson in &projection.lessons {
            if lesson.defect.is_some() {
                continue;
            }
            assert_eq!(
                lesson.occupies.len(),
                usize::from(lesson.duration_periods),
                "{}: a sound run covers its duration",
                found.name
            );
        }
    }
}

#[test]
fn the_wall_clock_span_comes_from_the_periods_covered() {
    let (problem, _, projection) = project_feasible("tiny").expect("tiny is feasible");
    for lesson in &projection.lessons {
        let first = lesson.occupies.first().expect("a run has a first slot");
        let last = lesson.occupies.last().expect("a run has a last slot");
        let period_of = |ordinal: u16| {
            problem
                .context
                .periods
                .iter()
                .find(|p| p.ordinal == ordinal)
                .expect("the period is in the context")
        };
        assert_eq!(
            lesson.starts_at,
            period_of(first.period_ordinal).interval.start.to_string(),
            "the start is the first period's, not a computed offset"
        );
        assert_eq!(
            lesson.ends_at,
            period_of(last.period_ordinal).interval.end.to_string(),
            "the end is the last period's"
        );
    }
}

#[test]
fn a_lesson_whose_run_is_not_contiguous_in_the_axis_is_pinned_rather_than_spanned() {
    // One lesson only, so the pin reason can be nothing but the broken run. With a full
    // arrangement the neighbouring lessons would share the cells and the reason would be
    // `Shared`, which is a different finding.
    //
    // Reachable in practice when a slot inside the run was never materialised — the state a
    // stored timetable reaches after its grid is rebuilt without one of its periods.
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let activity = ActivityIx::new(0);
    let slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, s)| s.is_teaching)
        .and_then(|(ix, _)| u32::try_from(ix).ok())
        .map(TimeslotIx::new)
        .expect("a teaching slot");
    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        activity,
        Placement {
            timeslot: slot,
            room: None,
        },
    );

    let labels = labels_for(&problem);
    let flags = no_flags();
    let mut projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 1, "exactly one lesson is placed");

    let axis = projection.axes.first().cloned().expect("one axis");
    if axis.periods.len() < 3 {
        return;
    }
    let lesson = &mut projection.lessons[0];
    lesson.schedule_id = axis.schedule_id;
    let day = lesson.day_ordinal;
    let timeslot = lesson.occupies[0].timeslot_id;
    lesson.occupies = vec![
        ProjectedCellRef {
            timeslot_id: timeslot,
            day_ordinal: day,
            period_ordinal: axis.periods[0].ordinal,
        },
        ProjectedCellRef {
            timeslot_id: timeslot,
            day_ordinal: day,
            period_ordinal: axis.periods[2].ordinal,
        },
    ];
    lesson.duration_periods = 2;

    let scope = projection.scopes[0].clone();
    let grid = lay_out(&projection, &scope, &labels);
    assert!(
        grid.is_well_formed(),
        "{:?}",
        grid.panels[0].malformations()
    );
    let broken = grid
        .panels
        .iter()
        .flat_map(|p| &p.cells)
        .filter(|c| {
            matches!(
                &c.content,
                CellContent::Pinned {
                    reason: PinReason::BrokenSpan,
                    ..
                }
            )
        })
        .count();
    assert_eq!(broken, 2, "each occupied row is pinned on its own");
    let spans = grid
        .panels
        .iter()
        .flat_map(|p| &p.cells)
        .any(|c| matches!(c.content, CellContent::Lesson { .. }));
    assert!(!spans, "a broken run earns no rowSpan at all");
}

// ---------------------------------------------------------------- axes

#[test]
fn a_cross_campus_fixture_keeps_two_bell_schedules_on_two_axes() {
    // The axes do not depend on any placement, so this runs over the **empty** arrangement
    // and therefore reaches the fixtures that ship no feasible one — which is where the
    // two-bell-schedule cases live (`two-campuses-different-bells` and its siblings).
    let mut saw_two = false;
    for name in fixture_names() {
        let Some(found) = fixture(name, SEED) else {
            continue;
        };
        for problem in &found.problems {
            if problem.context.bell_schedules.len() < 2 {
                continue;
            }
            saw_two = true;
            let labels = labels_for(problem);
            let flags = no_flags();
            let assignment = Assignment::empty(problem.activity_count());
            let projection = project(ProjectionInput {
                problem,
                assignment: &assignment,
                labels: &labels,
                flags: &flags,
            });
            assert_eq!(
                projection.axes.len(),
                problem.context.bell_schedules.len(),
                "{name}: every bell schedule is its own axis, never a merged one"
            );
            // No axis may contain a period belonging to another schedule.
            for axis in &projection.axes {
                let schedule = problem
                    .context
                    .bell_schedules
                    .iter()
                    .find(|s| s.id == axis.schedule_id)
                    .expect("the axis names a schedule");
                let allowed: Vec<_> = schedule
                    .periods
                    .iter()
                    .filter_map(|ix| problem.context.periods.get(ix.get()))
                    .map(|p| p.id)
                    .collect();
                for period in &axis.periods {
                    assert!(
                        allowed.contains(&period.period_id),
                        "{name}: a period of another schedule reached an axis"
                    );
                }
            }
        }
    }
    assert!(
        saw_two,
        "the catalogue should ship a multi-bell-schedule fixture"
    );
}

#[test]
fn a_scope_spanning_two_bell_schedules_gets_two_panels() {
    // Placed by hand, one occurrence per resolved schedule, so the case is reached whether or
    // not the fixture ships a feasible arrangement. The assertion is the panel count: two
    // schedules whose period rows do not align are never merged into one axis.
    let mut proved = false;
    for name in fixture_names() {
        let Some(found) = fixture(name, SEED) else {
            continue;
        };
        for problem in &found.problems {
            if problem.context.bell_schedules.len() < 2 {
                continue;
            }
            // One activity per distinct resolved schedule — the section's, as ADR 0033 says.
            let mut by_schedule: BTreeMap<usize, ActivityIx> = BTreeMap::new();
            for (ix, activity) in problem.activities.iter().enumerate() {
                let Some(section) = problem.sections.get(activity.section.get()) else {
                    continue;
                };
                let Some(schedule) = section.bell_schedule.schedule() else {
                    continue;
                };
                by_schedule
                    .entry(schedule.get())
                    .or_insert_with(|| ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX)));
            }
            if by_schedule.len() < 2 {
                continue;
            }

            let day = CycleDayIx::new(0);
            let mut assignment = Assignment::empty(problem.activity_count());
            for (schedule, activity) in &by_schedule {
                let run = problem.slots_of(
                    BellScheduleIx::new(u32::try_from(*schedule).unwrap_or(u32::MAX)),
                    day,
                );
                let duration = problem.activities[activity.get()].duration.as_usize();
                let Some(start) = run
                    .iter()
                    .position(|slot| {
                        problem
                            .timeslots
                            .get(slot.get())
                            .is_some_and(|s| s.is_teaching)
                    })
                    .filter(|from| from + duration <= run.len())
                else {
                    continue;
                };
                assignment.place(
                    *activity,
                    Placement {
                        timeslot: run[start],
                        room: None,
                    },
                );
            }

            let labels = labels_for(problem);
            let flags = no_flags();
            let projection = project(ProjectionInput {
                problem,
                assignment: &assignment,
                labels: &labels,
                flags: &flags,
            });
            let used: BTreeSet<_> = projection.lessons.iter().map(|l| l.schedule_id).collect();
            if used.len() < 2 {
                continue;
            }
            let master = lay_out(&projection, &projection.scopes[0], &labels);
            assert_eq!(
                master.panels.len(),
                used.len(),
                "{name}: one panel per schedule actually used, never a merged axis"
            );
            for panel in &master.panels {
                assert!(
                    panel.is_well_formed(),
                    "{name}: {:?}",
                    panel.malformations()
                );
            }
            proved = true;
        }
    }
    assert!(
        proved,
        "the catalogue should let one arrangement touch two bell schedules"
    );
}

#[test]
fn a_break_period_is_a_row_and_is_marked_as_non_teaching() {
    let mut saw_break = false;
    for name in fixture_names() {
        let Some((_, _, projection)) = project_feasible(name) else {
            continue;
        };
        for axis in &projection.axes {
            if axis.periods.iter().any(|p| !p.is_teaching) {
                saw_break = true;
                assert!(
                    axis.periods.iter().any(|p| p.is_teaching),
                    "an axis of only breaks would be a setup error, not a grid"
                );
            }
        }
        if saw_break {
            break;
        }
    }
    assert!(saw_break, "the catalogue should ship a non-teaching period");
}

// ---------------------------------------------------------------- cycle kinds

#[test]
fn a_weekly_and_a_custom_cycle_take_the_same_path() {
    use evara_core::scheduling::CycleKind;
    let mut kinds: Vec<CycleKind> = Vec::new();
    for name in fixture_names() {
        let Some((problem, labels, projection)) = project_feasible(name) else {
            continue;
        };
        if !kinds.contains(&problem.context.cycle.kind) {
            kinds.push(problem.context.cycle.kind);
        }
        // The columns are the cycle's days in ordinal order, whatever the kind.
        let expected: Vec<u16> = {
            let mut ordinals: Vec<u16> = problem
                .context
                .cycle_days
                .iter()
                .map(|d| d.ordinal)
                .collect();
            ordinals.sort_unstable();
            ordinals
        };
        assert_eq!(
            projection
                .days
                .iter()
                .map(|d| d.ordinal)
                .collect::<Vec<_>>(),
            expected,
            "{name}: the day axis is the cycle's ordinals"
        );
        for grid in every_grid(&projection, &labels) {
            for panel in &grid.panels {
                assert_eq!(
                    panel.days.len(),
                    problem.context.cycle_days.len(),
                    "{name}: every day is a column"
                );
            }
        }
    }
    assert!(
        kinds.contains(&CycleKind::Weekly) || kinds.contains(&CycleKind::Custom),
        "the catalogue should ship at least one cycle kind"
    );
}

// ---------------------------------------------------------------- scopes

#[test]
fn a_co_taught_lesson_is_on_every_teachers_grid() {
    for name in fixture_names() {
        let Some((_, labels, projection)) = project_feasible(name) else {
            continue;
        };
        for (ix, lesson) in projection.lessons.iter().enumerate() {
            if lesson.teacher_ids.len() < 2 {
                continue;
            }
            for teacher in &lesson.teacher_ids {
                let scope = projection
                    .scopes
                    .iter()
                    .find(|s| s.kind == ScopeKind::Teacher && s.id == Some(*teacher))
                    .expect("every teacher of a lesson is a scope");
                let grid = lay_out(&projection, scope, &labels);
                let present = grid.panels.iter().flat_map(|p| &p.cells).any(|c| {
                    matches!(c.content, CellContent::Lesson { lesson, .. } if lesson == ix)
                        || matches!(&c.content, CellContent::Pinned { lessons, .. } if lessons.contains(&ix))
                });
                assert!(present, "{name}: a co-taught lesson reaches both grids");
            }
        }
    }
}

#[test]
fn the_master_grid_holds_every_lesson_and_the_scopes_partition_none_away() {
    let (_, labels, projection) = project_feasible("tiny").expect("tiny is feasible");
    let master = &projection.scopes[0];
    assert_eq!(master.kind, ScopeKind::Master);
    assert_eq!(master.lesson_count, projection.lessons.len());

    // The union of the section scopes is every lesson: a lesson belongs to exactly one.
    let mut seen = vec![false; projection.lessons.len()];
    for scope in projection
        .scopes
        .iter()
        .filter(|s| s.kind == ScopeKind::Section)
    {
        for (ix, lesson) in projection.lessons.iter().enumerate() {
            if in_scope(lesson, scope, &labels) {
                assert!(!seen[ix], "a lesson is in two section scopes");
                seen[ix] = true;
            }
        }
    }
    assert!(
        seen.iter().all(|s| *s),
        "every lesson belongs to a section scope"
    );
}

#[test]
fn a_lesson_with_no_room_is_in_no_room_scope_but_keeps_its_section_one() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let activity = ActivityIx::new(0);
    let slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, s)| s.is_teaching)
        .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap()))
        .expect("a teaching slot");
    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        activity,
        Placement {
            timeslot: slot,
            room: None,
        },
    );

    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 1);
    assert!(projection.lessons[0].room_id.is_none());
    for scope in projection
        .scopes
        .iter()
        .filter(|s| s.kind == ScopeKind::Room)
    {
        assert_eq!(scope.lesson_count, 0, "a roomless lesson books no room");
    }
    let section = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Section && s.id == Some(projection.lessons[0].section_id))
        .expect("its section is a scope");
    assert_eq!(section.lesson_count, 1);
}

// ---------------------------------------------------------------- students

#[test]
fn a_student_grid_is_derived_from_enrolment_rows_and_nothing_else() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let feasible = found
        .expectations
        .feasible
        .as_ref()
        .expect("tiny is feasible");
    let assignment = feasible.assignment(problem.activity_count());

    let enrolled_section = problem.sections[0].id;
    let student = uuid::Uuid::from_bytes([42; 16]);
    let mut labels = labels_for(&problem);
    labels.students = vec![StudentLabel {
        student_id: student,
        name: "A. Student".to_owned(),
        code: Some("S001".to_owned()),
        sections: vec![enrolled_section],
    }];
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });

    let scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Student)
        .expect("the enrolled student is a scope")
        .clone();
    assert!(
        scope.roster_derived,
        "a student grid is labelled as roster-derived"
    );
    assert!(projection.has_enrolments);

    // Exactly the lessons of the enrolled section, and no others — never a group's timetable.
    let expected = projection
        .lessons
        .iter()
        .filter(|l| l.section_id == enrolled_section)
        .count();
    assert_eq!(scope.lesson_count, expected);
    assert!(expected > 0, "the enrolled section has lessons");

    let grid = lay_out(&projection, &scope, &labels);
    assert!(grid.is_well_formed());
    for panel in &grid.panels {
        for cell in &panel.cells {
            let ixs: Vec<usize> = match &cell.content {
                CellContent::Lesson { lesson, .. } => vec![*lesson],
                CellContent::Pinned { lessons, .. } => lessons.clone(),
                _ => Vec::new(),
            };
            for ix in ixs {
                assert_eq!(
                    projection.lessons[ix].section_id, enrolled_section,
                    "only the enrolled section reaches a student's grid"
                );
            }
        }
    }
}

#[test]
fn a_project_with_no_enrolment_rows_offers_no_student_scope() {
    let (_, _, projection) = project_feasible("tiny").expect("tiny is feasible");
    assert!(
        !projection.has_enrolments,
        "the default label set has no roster"
    );
    assert!(
        !projection
            .scopes
            .iter()
            .any(|s| s.kind == ScopeKind::Student),
        "with no rows there is nothing to derive a student grid from"
    );
}

#[test]
fn a_student_enrolled_in_nothing_has_an_empty_grid_rather_than_a_groups() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let feasible = found.expectations.feasible.as_ref().expect("feasible");
    let assignment = feasible.assignment(problem.activity_count());

    let student = uuid::Uuid::from_bytes([43; 16]);
    let mut labels = labels_for(&problem);
    labels.students = vec![StudentLabel {
        student_id: student,
        name: "B. Student".to_owned(),
        code: None,
        sections: Vec::new(),
    }];
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert!(
        !projection.has_enrolments,
        "a student with no sections is not an enrolment"
    );
    let scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Student)
        .expect("the student is still listed")
        .clone();
    assert_eq!(
        scope.lesson_count, 0,
        "no row means no lesson, never a group's timetable"
    );
    let grid = lay_out(&projection, &scope, &labels);
    assert!(
        grid.panels.is_empty(),
        "an empty roster produces no panel to mistake for a schedule"
    );
}

#[test]
fn a_partial_roster_shows_only_what_was_entered() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let feasible = found.expectations.feasible.as_ref().expect("feasible");
    let assignment = feasible.assignment(problem.activity_count());
    if problem.sections.len() < 2 {
        return;
    }
    let one = problem.sections[0].id;
    let student = uuid::Uuid::from_bytes([44; 16]);
    let mut labels = labels_for(&problem);
    labels.students = vec![StudentLabel {
        student_id: student,
        name: "C. Student".to_owned(),
        code: None,
        sections: vec![one],
    }];
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let total = projection.lessons.len();
    let scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Student)
        .expect("a scope")
        .clone();
    assert!(
        scope.lesson_count < total,
        "a partial roster is partial, and is not completed from elsewhere"
    );
}

// ---------------------------------------------------------------- determinism and labels

#[test]
fn the_same_inputs_project_to_the_same_value() {
    let first = project_feasible("tiny").expect("tiny is feasible");
    let second = project_feasible("tiny").expect("tiny is feasible");
    assert_eq!(first.2, second.2, "a projection is deterministic");
    let a = lay_out(&first.2, &first.2.scopes[0], &first.1);
    let b = lay_out(&second.2, &second.2.scopes[0], &second.1);
    assert_eq!(a, b, "so is a layout");
}

#[test]
fn a_missing_label_leaves_none_rather_than_a_substituted_name() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let feasible = found.expectations.feasible.as_ref().expect("feasible");
    let assignment = feasible.assignment(problem.activity_count());
    let empty = LabelSet::default();
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &empty,
        flags: &flags,
    });
    let placed = projection.lessons.len();
    assert!(placed > 0, "the fixture places something to label");
    for lesson in &projection.lessons {
        assert!(
            lesson.label.is_none(),
            "an absent label is absent, not invented"
        );
        assert!(lesson.room.is_none() || lesson.room_id.is_none());
    }
    // The scope still names the section, falling back to the problem's own name.
    let section_scope = projection
        .scopes
        .iter()
        .find(|s| s.kind == ScopeKind::Section)
        .expect("a section scope");
    let named = section_scope.name.len();
    assert!(
        named > 0,
        "a section scope still names itself, falling back to the problem's own name"
    );
}

#[test]
fn stored_flags_reach_the_projection() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let feasible = found.expectations.feasible.as_ref().expect("feasible");
    let assignment = feasible.assignment(problem.activity_count());
    let labels = labels_for(&problem);
    let first = problem.activities[0].id;
    let mut flags = BTreeMap::new();
    flags.insert(
        first,
        LessonFlags {
            is_locked: true,
            is_manual: true,
        },
    );
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let lesson = projection
        .lessons
        .iter()
        .find(|l| l.activity_id == first)
        .expect("the first occurrence is placed");
    assert!(lesson.flags.is_locked);
    assert!(lesson.flags.is_manual);
    for other in projection.lessons.iter().filter(|l| l.activity_id != first) {
        assert!(!other.flags.is_locked, "a flag is not spread");
    }
}

#[test]
fn an_empty_arrangement_projects_no_lesson_and_still_offers_every_axis() {
    let found = fixture("tiny", SEED).expect("tiny exists");
    let problem = found.problem().clone();
    let assignment = Assignment::empty(problem.activity_count());
    let labels = labels_for(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let placed = projection.lessons.len();
    let axes = projection.axes.len();
    let days = projection.days.len();
    assert_eq!(placed, 0, "nothing is placed");
    assert!(axes > 0, "the grid still has a shape");
    assert!(days > 0, "and the cycle still has its days");
    let grid = lay_out(&projection, &projection.scopes[0], &labels);
    assert!(
        grid.panels.is_empty(),
        "no lesson means no panel, which an empty state explains"
    );
}
