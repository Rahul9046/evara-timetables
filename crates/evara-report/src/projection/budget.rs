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

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
use evara_core::scheduling::fixtures::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use evara_core::scheduling::{
    ActivityIx, Assignment, BellScheduleResolution, CycleKind, GroupKind, Overlap, Placement,
    Problem,
};
use uuid::Uuid;

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

// =======================================================================================
// The §14.2 index fix, and the cost it removes
// =======================================================================================

/// `count` synthetic students, each enrolled in six sections.
///
/// Every name is invented — `CLAUDE.md` rule 3 — and the enrolment is round-robin, which is
/// the shape that matters: a roster whose students each hold a handful of sections is what a
/// real `class_enrolment` looks like, and it is the one the old linear scan was worst at.
fn a_roster(problem: &Problem, count: usize) -> Vec<StudentLabel> {
    if problem.sections.is_empty() {
        return Vec::new();
    }
    (0..count)
        .map(|n| {
            let sections = (0..6)
                .map(|k| problem.sections[(n * 7 + k * 83) % problem.sections.len()].id)
                .collect();
            StudentLabel {
                student_id: Uuid::from_u128(0xEEEE_0000_0000_0000_0000_0000_0000_0000 + n as u128),
                name: format!("Student {n}"),
                code: Some(format!("S{n:05}")),
                sections,
            }
        })
        .collect()
}

/// The scan `in_scope` used before M11, kept here as the measurement's *before*.
///
/// **Not** a second implementation of membership: nothing calls it but these measurements, and
/// the test below asserts it agrees with [`in_scope_indexed`] on every pair it is asked about.
/// It exists so the figure in the completion report is a measurement rather than an argument.
fn membership_by_scan(lesson: &ProjectedLesson, student_id: Uuid, labels: &LabelSet) -> bool {
    labels
        .students
        .iter()
        .find(|s| s.student_id == student_id)
        .is_some_and(|s| s.sections.contains(&lesson.section_id))
}

#[test]
fn every_student_grid_of_a_large_school_lays_out_within_budget() {
    // R5's upper end for the timetable, and `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §14.2's upper
    // end for the roster: 2 000 occurrences and 3 000 students. Before the index this was
    // `O(students² × lessons)` — about 1.8 × 10¹⁰ comparisons, which is minutes to hours
    // rather than a slow export, and is why the fix is a Stage 1 deliverable rather than an
    // optimisation.
    let problem = a_large_school(500);
    assert_eq!(problem.activities.len(), 2_000);

    let mut labels = labels_for(&problem);
    labels.students = a_roster(&problem, 3_000);
    assert_eq!(labels.students.len(), 3_000);
    assert!(labels.has_enrolments());

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

    // `build_scopes` is on this path, and it used to answer every entity's lesson count with
    // its own pass over the 2 000 lessons — 3 620 passes, of which 3 000 were the roster's.
    let students: Vec<ProjectionScope> = projection
        .scopes
        .iter()
        .filter(|scope| scope.kind == ScopeKind::Student)
        .cloned()
        .collect();
    assert_eq!(
        students.len(),
        3_000,
        "one selectable grid per enrolled student"
    );
    assert_eq!(
        projection.scopes.len(),
        1 + 500 + 80 + 40 + 3_000,
        "master, then every section, teacher, room and student"
    );

    let started = Instant::now();
    let grids = lay_out_many(&projection, &students, &labels);
    let laid_out = started.elapsed();

    assert_eq!(grids.len(), 3_000, "every selected student gets a grid");
    let cells: usize = grids
        .iter()
        .flat_map(|grid| &grid.panels)
        .map(|panel| panel.cells.len())
        .sum();
    for grid in &grids {
        assert!(
            grid.is_well_formed(),
            "{:?}",
            grid.panels.first().map(GridPanel::malformations)
        );
    }

    println!(
        "M11 §14.2 bulk student layout: 3000 students, 500 sections, 2000 placed occurrences, \
         3000 scopes laid out; projected in {projected:?}, laid out in {laid_out:?} \
         ({cells} cells)"
    );
    // A hang detector, not a performance bar — and one the quadratic version could not pass:
    // 1.8 × 10¹⁰ comparisons is minutes at best. The figure printed above is three orders of
    // magnitude inside this bound, which is what makes a loose wall-clock assertion a real
    // regression test here rather than a flaky one.
    assert!(
        projected.as_secs() < 60 && laid_out.as_secs() < 60,
        "projected {projected:?}, laid out {laid_out:?}"
    );
}

#[test]
fn the_index_answers_membership_exactly_as_the_scan_did() {
    // The correctness half of the §14.2 change: the index is a faster route to the **same**
    // answer. Asserted over every (student, lesson) pair of a school small enough to compare
    // exhaustively — 200 students × 400 occurrences is 80 000 pairs.
    let problem = a_large_school(100);
    let mut labels = labels_for(&problem);
    labels.students = a_roster(&problem, 200);
    let assignment = a_dense_arrangement(&problem);
    let flags: BTreeMap<_, _> = BTreeMap::new();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });

    let index = StudentIndex::of(&labels);
    assert_eq!(index.len(), labels.students.len());
    assert!(!index.is_empty());

    let mut pairs = 0_usize;
    let mut matches = 0_usize;
    for scope in projection
        .scopes
        .iter()
        .filter(|scope| scope.kind == ScopeKind::Student)
    {
        let id = scope.id.expect("a student has an id");
        for lesson in &projection.lessons {
            let by_scan = membership_by_scan(lesson, id, &labels);
            assert_eq!(
                in_scope_indexed(lesson, scope, &index),
                by_scan,
                "the index and the scan must agree"
            );
            // And the public wrapper, which builds its own index, agrees with both.
            assert_eq!(in_scope(lesson, scope, &labels), by_scan);
            pairs += 1;
            if by_scan {
                matches += 1;
            }
        }
    }
    assert_eq!(pairs, 200 * 400, "every pair was compared");
    assert!(
        matches > 0,
        "some student is enrolled in some placed lesson"
    );

    // And the counts `build_scopes` computes in one pass are the counts the per-entity scans
    // produced — the same property for sections, teachers and rooms.
    for scope in &projection.scopes {
        let counted = match (scope.kind, scope.id) {
            (ScopeKind::Master, _) => projection.lessons.len(),
            (ScopeKind::Section, Some(id)) => projection
                .lessons
                .iter()
                .filter(|l| l.section_id == id)
                .count(),
            (ScopeKind::Teacher, Some(id)) => projection
                .lessons
                .iter()
                .filter(|l| l.teacher_ids.contains(&id))
                .count(),
            (ScopeKind::Room, Some(id)) => projection
                .lessons
                .iter()
                .filter(|l| l.room_id == Some(id))
                .count(),
            (ScopeKind::Student, Some(id)) => projection
                .lessons
                .iter()
                .filter(|l| membership_by_scan(l, id, &labels))
                .count(),
            (_, None) => 0,
        };
        assert_eq!(
            scope.lesson_count, counted,
            "{:?} {:?}: the one-pass tally is the per-entity scan's answer",
            scope.kind, scope.name
        );
    }
}

#[test]
fn a_students_grid_holds_only_the_sections_their_enrolment_names() {
    // The property the index must not weaken, asserted at scale: a student's grid is
    // `class_enrolment` and nothing else — never a group's membership, never a group's size.
    let problem = a_large_school(200);
    let mut labels = labels_for(&problem);
    labels.students = a_roster(&problem, 400);
    let assignment = a_dense_arrangement(&problem);
    let flags: BTreeMap<_, _> = BTreeMap::new();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let students: Vec<ProjectionScope> = projection
        .scopes
        .iter()
        .filter(|scope| scope.kind == ScopeKind::Student)
        .cloned()
        .collect();
    let grids = lay_out_many(&projection, &students, &labels);

    let mut shown = 0_usize;
    for grid in &grids {
        let id = grid.scope.id.expect("a student has an id");
        for cell in grid.panels.iter().flat_map(|panel| &panel.cells) {
            let occupants = match &cell.content {
                CellContent::Lesson { lesson, .. } => vec![*lesson],
                CellContent::Pinned { lessons, .. } => lessons.clone(),
                _ => Vec::new(),
            };
            for ix in occupants {
                assert!(
                    membership_by_scan(&projection.lessons[ix], id, &labels),
                    "a section nobody enrolled them in reached a student's grid"
                );
                shown += 1;
            }
        }
    }
    assert!(shown > 0, "the roster reaches some placed lessons");
}

#[test]
fn the_quadratic_scan_grows_faster_than_the_index_as_the_roster_grows() {
    // The *before* figure, measured rather than argued, at three roster sizes so the **shape**
    // of the cost is visible and not only its size: the scan is quadratic in the roster and
    // the index is linear, so doubling the roster roughly quadruples one and doubles the
    // other. R5's full 3 000-student roster is deliberately not scanned here — at that size
    // the scan alone is seconds, and this file runs on every `npm run check`.
    //
    // Every figure is printed. The ratios between them are the finding; none is asserted
    // tightly, because a wall-clock assertion on shared CI hardware is a flaky test.
    let problem = a_large_school(500);
    let base = labels_for(&problem);
    let assignment = a_dense_arrangement(&problem);
    let flags: BTreeMap<_, _> = BTreeMap::new();

    for roster in [300_usize, 600, 1_200] {
        let mut labels = base.clone();
        labels.students = a_roster(&problem, roster);
        let projection = project(ProjectionInput {
            problem: &problem,
            assignment: &assignment,
            labels: &labels,
            flags: &flags,
        });
        let students: Vec<ProjectionScope> = projection
            .scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Student)
            .cloned()
            .collect();
        assert_eq!(students.len(), roster);

        let started = Instant::now();
        let mut by_scan = 0_usize;
        for scope in &students {
            let id = scope.id.expect("a student has an id");
            for lesson in &projection.lessons {
                if membership_by_scan(lesson, id, &labels) {
                    by_scan += 1;
                }
            }
        }
        let scanned = started.elapsed();

        let index = StudentIndex::of(&labels);
        let started = Instant::now();
        let mut by_index = 0_usize;
        for scope in &students {
            for lesson in &projection.lessons {
                if in_scope_indexed(lesson, scope, &index) {
                    by_index += 1;
                }
            }
        }
        let indexed = started.elapsed();

        assert_eq!(by_scan, by_index, "the same answer, both ways");
        println!(
            "M11 §14.2 membership, {roster} students x {} occurrences \
             ({by_scan} memberships): scan {scanned:?}, index {indexed:?}",
            projection.lessons.len(),
        );
        assert!(
            scanned.as_secs() < 60 && indexed.as_secs() < 60,
            "a hang detector only"
        );
    }
}

#[test]
fn the_shared_caption_costs_what_the_proposal_priced_it_at() {
    // §14.1 priced `CellLines` at "roughly 100–200 bytes per lesson — about 0.2–0.4 MB at
    // 2 000 lessons", and risk R-9 says that if it matters the field becomes conditional, but
    // that a conditional field is worse than 300 KB and should not be reached for first. This
    // is the measurement that turns both into facts.
    //
    // The figure asserted is a **ceiling**, loose enough not to be a maintenance burden and
    // tight enough that a field quietly growing by an order of magnitude is a red test.
    let problem = a_large_school(500);
    let labels = labels_for(&problem);
    let assignment = a_dense_arrangement(&problem);
    let flags: BTreeMap<_, _> = BTreeMap::new();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 2_000);

    let whole = serde_json::to_string(&projection)
        .expect("a projection serialises")
        .len();
    let lines: Vec<&CellLines> = projection
        .lessons
        .iter()
        .map(|lesson| &lesson.lines)
        .collect();
    let captions = serde_json::to_string(&lines)
        .expect("the captions serialise")
        .len();
    let per_lesson = captions / projection.lessons.len();

    println!(
        "M11 §14.1 IPC payload: a 2000-lesson projection is {whole} bytes of JSON, of which \
         {captions} bytes ({per_lesson} per lesson) are the shared captions"
    );
    assert!(
        per_lesson < 400,
        "the shared caption is {per_lesson} bytes per lesson, which is past what §14.1 priced"
    );
    assert!(
        captions < 1_000_000,
        "the shared caption adds {captions} bytes at the documented upper end"
    );
}
