//! The projection budget at the realistic upper end.
//!
//! [OPEN-DECISIONS.md](../../../../docs/OPEN-DECISIONS.md) R5 names *"a 2,000-activity
//! secondary school with electives"* as the realistic upper end, asks for its numbers to be
//! treated as a budget, and records that two of its three figures were still unmeasured after
//! M6: load time, and **grid rendering, which had no grid to render until Phase 7**. This
//! file is the projection half of that debt.
//!
//! # What is asserted, and what is only measured
//!
//! The **structural** figures are asserted, because they are deterministic: how many lessons
//! a projection holds, how many cells a panel has, that every panel is well-formed, and that
//! the flat lesson list is serialised once however many grids address it.
//!
//! The **wall-clock** figure is measured and printed, and bounded only loosely. A tight
//! timing assertion on shared CI hardware is a flaky test, and a flaky test is worse than a
//! recorded number — so the bound here is a hang detector and the figure itself is reported
//! for `TIMETABLE-VIEWS.md` to quote. Run with `--nocapture` to read it.
//!
//! This is also the one place in the workspace that reads a clock on purpose. It is a test
//! *about* cost; nothing it measures reaches a scheduling decision, so `CLAUDE.md`'s rule
//! that the engine reads no clock is untouched.

use std::collections::BTreeMap;
use std::time::Instant;

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel};
use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleResolution, CycleKind, GroupKind, Overlap, Placement,
    Problem,
};

use super::*;

/// A synthetic secondary school of roughly `sections` sections, each with four single periods.
///
/// The same shape `evara-solver`'s own upper-end measurement uses, and built here for the
/// same reason that one is built there: the catalogue's largest fixture is `medium` at 500,
/// and widening a documented 24-fixture catalogue is M2's surface rather than M8's. So the
/// upper-end measurement lives with the measurement.
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

/// Labels for every section and room, as a real load would supply.
fn labels_for(problem: &Problem) -> LabelSet {
    LabelSet {
        sections: problem
            .sections
            .iter()
            .enumerate()
            .map(|(n, section)| SectionLabel {
                section_id: section.id,
                subject: format!("Subject {}", n % 12),
                course: section.name.clone(),
                section: section.name.clone(),
                year_level: Some(format!("{}", 7 + (n % 6))),
                campus: Some("Main".to_owned()),
                short: SectionLabel::compose_short(
                    Some(&format!("{}", 7 + (n % 6))),
                    &format!("Subject {}", n % 12),
                    &section.name,
                ),
            })
            .collect(),
        rooms: problem
            .rooms
            .iter()
            .map(|room| RoomLabel {
                room_id: room.id,
                name: room.name.clone(),
                building: Some("Main Block".to_owned()),
            })
            .collect(),
        students: Vec::new(),
    }
}

/// Fills the grid round-robin, so almost every cell is occupied.
///
/// Not a solve — a deliberately dense arrangement, which is the worst case for layout: every
/// cell has to be decided and most of them hold something.
fn a_dense_arrangement(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activity_count());
    let slots: Vec<_> = problem
        .timeslots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.is_teaching)
        .map(|(ix, _)| evara_core::scheduling::TimeslotIx::new(u32::try_from(ix).unwrap_or(0)))
        .collect();
    let rooms: Vec<_> = problem
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| room.is_bookable)
        .map(|(ix, _)| evara_core::scheduling::RoomIx::new(u32::try_from(ix).unwrap_or(0)))
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
                // Rooms round-robin too, so the room grids have bookings to lay out. Both
                // cycles wrap, so plenty of cells end up shared — which is the worst case for
                // layout and exactly what should be measured.
                room: rooms.get(ix % rooms.len().max(1)).copied(),
            },
        );
    }
    assignment
}

#[test]
fn the_realistic_upper_end_projects_within_budget() {
    let problem = a_large_school(500);
    assert_eq!(
        problem.activities.len(),
        2_000,
        "R5's documented upper end: a 2 000-occurrence secondary school"
    );

    let labels = labels_for(&problem);
    let assignment = a_dense_arrangement(&problem);
    let flags: BTreeMap<_, _> = BTreeMap::new();

    let started = Instant::now();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let projected = started.elapsed();

    // Structural, and therefore asserted.
    assert_eq!(
        projection.lessons.len(),
        2_000,
        "every placed occurrence is projected exactly once"
    );
    assert_eq!(projection.axes.len(), 1, "one bell schedule, one axis");
    assert_eq!(projection.days.len(), 5);
    assert_eq!(
        projection.scopes.len(),
        1 + 500 + 80 + 40,
        "master, then every section, teacher and room"
    );

    // The four grids a reader actually opens, laid out from the one projection. No reload,
    // and no second serialisation of the 2 000 lessons.
    let started = Instant::now();
    let mut panels = 0_usize;
    let mut cells = 0_usize;
    for kind in [
        ScopeKind::Master,
        ScopeKind::Section,
        ScopeKind::Teacher,
        ScopeKind::Room,
    ] {
        let scope = projection
            .scopes
            .iter()
            .find(|s| s.kind == kind && s.lesson_count > 0)
            .unwrap_or_else(|| panic!("a {kind:?} scope with lessons"))
            .clone();
        let grid = lay_out(&projection, &scope, &labels);
        assert!(
            grid.is_well_formed(),
            "{kind:?}: {:?}",
            grid.panels.first().map(GridPanel::malformations)
        );
        panels += grid.panels.len();
        cells += grid.panels.iter().map(|p| p.cells.len()).sum::<usize>();
    }
    let laid_out = started.elapsed();

    // A single scope grid is 5 days x 8 periods = 40 cells, which is what makes the
    // per-scope views cheap however large the school is. Four grids, one panel each.
    assert_eq!(panels, 4, "one panel per grid, one schedule in play");
    assert_eq!(cells, 4 * 5 * 8, "40 cells per grid");

    // The master grid is the only large one, and it is still one panel of 40 cells — the
    // lessons stack inside them rather than multiplying the cell count. That is why
    // pagination, not virtualisation, is the master grid's answer.
    let master = lay_out(&projection, &projection.scopes[0], &labels);
    assert_eq!(master.panels[0].cells.len(), 40);
    let stacked: usize = master.panels[0]
        .cells
        .iter()
        .filter_map(|c| match &c.content {
            CellContent::Pinned { lessons, .. } => Some(lessons.len()),
            CellContent::Lesson { .. } => Some(1),
            _ => None,
        })
        .sum();
    assert_eq!(
        stacked, 2_000,
        "every lesson is reachable from the master grid, none discarded"
    );

    // Measured, not asserted tightly. Printed so `TIMETABLE-VIEWS.md` can quote a figure a
    // reader can reproduce rather than take on trust.
    println!(
        "R5 projection budget: 2000 occurrences projected in {projected:?}; \
         four scope grids laid out in {laid_out:?} ({cells} cells)"
    );
    assert!(
        projected.as_secs() < 30 && laid_out.as_secs() < 30,
        "a hang detector, not a performance bar: projected {projected:?}, laid out {laid_out:?}"
    );
}

#[test]
fn a_scope_grid_cost_does_not_grow_with_the_school() {
    // The claim the desktop design rests on: switching scope is cheap because a scope grid is
    // days x periods, whatever the school's size. Asserted by building two schools an order
    // of magnitude apart and comparing the cell counts.
    let small = a_large_school(50);
    let large = a_large_school(500);
    assert_eq!(small.activities.len(), 200);
    assert_eq!(large.activities.len(), 2_000);

    let mut counts = Vec::new();
    for problem in [&small, &large] {
        let labels = labels_for(problem);
        let assignment = a_dense_arrangement(problem);
        let flags: BTreeMap<_, _> = BTreeMap::new();
        let projection = project(ProjectionInput {
            problem,
            assignment: &assignment,
            labels: &labels,
            flags: &flags,
        });
        let scope = projection
            .scopes
            .iter()
            .find(|s| s.kind == ScopeKind::Teacher && s.lesson_count > 0)
            .expect("a teacher with lessons")
            .clone();
        let grid = lay_out(&projection, &scope, &labels);
        assert!(grid.is_well_formed());
        counts.push(grid.panels.iter().map(|p| p.cells.len()).sum::<usize>());
    }
    assert_eq!(
        counts[0], counts[1],
        "a teacher's grid is the same size in a small school and a large one"
    );
}
