//! Composition tests, driven by M2's deterministic synthetic catalogue.
//!
//! The suite is built around one property — **conservation** — because the requirement on this
//! module is not *"the common case composes"* but *"nothing is lost on the way out"*. A
//! renderer that drops a lesson produces a timetable a school will act on, and the lost lesson
//! is invisible precisely because the document looks complete.
//!
//! Every fixture is composed at every scope kind it offers. Nothing here asserts a rendered
//! byte: there is no renderer yet, and Stage 1's exit criterion is that a document composes
//! for all 24 fixtures and conservation holds.

use std::collections::{BTreeMap, BTreeSet};

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
use evara_core::scheduling::fixtures::{all_fixtures, fixture};
use evara_core::scheduling::{
    ActivityIx, Assignment, CycleKind, Placement, Problem, RoomIx, TimeslotIx,
};
use uuid::Uuid;

use crate::findings::{FindingEntity, FindingView, StoredFinding};
use crate::projection::{
    CellContent, ProjectedCellRef, ProjectionInput, ProjectionScope, ScopeKind,
    TimetableProjection, lay_out, lay_out_many, no_flags, project,
};

use super::*;

/// The seed every test here uses, matching the rest of the workspace.
const SEED: u64 = 2026;

/// Every scope kind, so a sweep cannot quietly skip one.
const EVERY_KIND: [ScopeKind; 5] = [
    ScopeKind::Master,
    ScopeKind::Section,
    ScopeKind::Teacher,
    ScopeKind::Room,
    ScopeKind::Student,
];

// =======================================================================================
// Scaffolding
// =======================================================================================

/// A label set covering every section and room, and a synthetic roster.
///
/// Built from the problem so the tests exercise the join rather than a hand-written table. The
/// roster is **invented here** — `CLAUDE.md` rule 3, and every name below belongs to nobody —
/// because the catalogue's `LabelSet` is the loader's and the fixtures carry no enrolment rows:
/// a student scope has to be exercised, and the only honest way is explicit synthetic rows.
fn labels_for(problem: &Problem, students: usize) -> LabelSet {
    LabelSet {
        sections: problem
            .sections
            .iter()
            .enumerate()
            .map(|(n, section)| SectionLabel {
                section_id: section.id,
                subject: format!("Subject {}", n % 5),
                course: format!("Course for {}", section.name),
                section: section.name.clone(),
                year_level: Some(format!("{}", 7 + (n % 6))),
                campus: None,
                short: SectionLabel::compose_short(
                    Some(&format!("{}", 7 + (n % 6))),
                    &format!("Subject {}", n % 5),
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
        students: a_roster(problem, students),
    }
}

/// `count` synthetic students, each enrolled in one section, round-robin.
fn a_roster(problem: &Problem, count: usize) -> Vec<StudentLabel> {
    if problem.sections.is_empty() {
        return Vec::new();
    }
    (0..count)
        .map(|n| StudentLabel {
            student_id: Uuid::from_bytes([
                0xEE,
                0xEE,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                u8::try_from(n >> 24 & 0xFF).unwrap_or(0),
                u8::try_from(n >> 16 & 0xFF).unwrap_or(0),
                u8::try_from(n >> 8 & 0xFF).unwrap_or(0),
                u8::try_from(n & 0xFF).unwrap_or(0),
            ]),
            name: format!("Student {n}"),
            code: Some(format!("S{n:04}")),
            sections: vec![problem.sections[n % problem.sections.len()].id],
        })
        .collect()
}

/// Places every occurrence round-robin over the teaching slots **of its own bell schedule**.
///
/// Not a solve. Deliberately dense, and deliberately clash-ridden where the slots run out,
/// because that is the arrangement a stored timetable acquires when its inputs move — and the
/// one a document must carry rather than tidy.
///
/// It respects two things a solver respects, and only those two: the section's resolved bell
/// schedule, and its room candidate list. The first matters here because a projection reads
/// the **slot's** schedule, so an arrangement that ignored it would put both campuses of a
/// two-campus school on one panel and quietly make every multi-campus assertion vacuous.
fn a_dense_arrangement(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activity_count());

    // Teaching slots per bell schedule, in index order.
    let mut by_schedule: BTreeMap<usize, Vec<TimeslotIx>> = BTreeMap::new();
    for (ix, slot) in problem.timeslots.iter().enumerate() {
        if !slot.is_teaching {
            continue;
        }
        if let Ok(n) = u32::try_from(ix) {
            by_schedule
                .entry(slot.bell_schedule.get())
                .or_default()
                .push(TimeslotIx::new(n));
        }
    }
    if by_schedule.is_empty() {
        return assignment;
    }
    let anywhere: Vec<TimeslotIx> = by_schedule.values().flatten().copied().collect();
    let every_room: Vec<RoomIx> = problem
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| room.is_bookable)
        .filter_map(|(ix, _)| u32::try_from(ix).ok().map(RoomIx::new))
        .collect();

    for (ix, fact) in problem.activities.iter().enumerate() {
        let Ok(n) = u32::try_from(ix) else { continue };
        let section = problem.sections.get(fact.section.get());
        let slots = section
            .and_then(|s| s.bell_schedule.schedule())
            .and_then(|schedule| by_schedule.get(&schedule.get()))
            .unwrap_or(&anywhere);
        if slots.is_empty() {
            continue;
        }
        let rooms = section
            .map(|s| s.room_candidates.as_slice())
            .filter(|candidates| !candidates.is_empty())
            .unwrap_or(&every_room);
        assignment.place(
            ActivityIx::new(n),
            Placement {
                timeslot: slots[ix % slots.len()],
                room: rooms.get(ix % rooms.len().max(1)).copied(),
            },
        );
    }
    assignment
}

/// One fixture, projected: its own feasible arrangement where it ships one, a dense one
/// otherwise.
struct Projected {
    problem: Problem,
    labels: LabelSet,
    projection: TimetableProjection,
}

impl Projected {
    /// Composes with the given options.
    fn compose(&self, options: &OutputOptions) -> Result<OutputDocument, OutputError> {
        compose(self.input(&[], &[]), options)
    }

    /// The composition input, with the appendices the caller wants.
    fn input<'a>(
        &'a self,
        unscheduled: &'a [UnscheduledOccurrence],
        findings: &'a [FindingView],
    ) -> CompositionInput<'a> {
        CompositionInput {
            problem: &self.problem,
            projection: &self.projection,
            labels: &self.labels,
            identity: an_identity(),
            unscheduled,
            findings,
        }
    }

    /// Every scope of one kind the projection offers.
    fn scopes(&self, kind: ScopeKind) -> Vec<ProjectionScope> {
        self.projection
            .scopes
            .iter()
            .filter(|scope| scope.kind == kind)
            .cloned()
            .collect()
    }
}

/// What only a caller knows, fixed so a document is byte-stable across a test run.
fn an_identity() -> &'static DocumentIdentity {
    use std::sync::OnceLock;
    static IDENTITY: OnceLock<DocumentIdentity> = OnceLock::new();
    IDENTITY.get_or_init(|| DocumentIdentity {
        timetable_name: "Spring 2027 Draft 3".to_owned(),
        timetable_id: Uuid::from_bytes([7; 16]),
        timetable_rev: 4,
        provenance: Some(RunProvenance {
            engine: "construction".to_owned(),
            seed: 42,
            status: "PARTIAL".to_owned(),
            termination: "Converged".to_owned(),
        }),
        // A fixed stamp: this crate reads no clock, and a test that minted one would be
        // asserting against a moving value.
        generated_at: "2026-11-24T09:00:00.000Z".to_owned(),
        app_version: "0.1.0-test".to_owned(),
        logo: None,
    })
}

/// Projects one fixture by name.
fn projected(name: &str, students: usize) -> Projected {
    let found = fixture(name, SEED).unwrap_or_else(|| panic!("{name} is in the catalogue"));
    project_fixture(&found, students)
}

/// Projects one fixture's first problem.
fn project_fixture(
    found: &evara_core::scheduling::fixtures::SchedulingFixture,
    students: usize,
) -> Projected {
    let (index, assignment) = match found.expectations.feasible.as_ref() {
        Some(feasible) => (
            feasible.problem,
            feasible.assignment(found.problems[feasible.problem].activity_count()),
        ),
        None => (0, a_dense_arrangement(&found.problems[0])),
    };
    let problem = found.problems[index].clone();
    let labels = labels_for(&problem, students);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    Projected {
        problem,
        labels,
        projection,
    }
}

/// Every occurrence a document's sheets carry, by identifier.
fn in_document(document: &OutputDocument) -> BTreeSet<Uuid> {
    document
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.panels)
        .flat_map(|panel| &panel.cells)
        .flat_map(|cell| &cell.lessons)
        .map(|lesson| lesson.activity_id)
        .collect()
}

/// Every `(entity, day, period, activity)` position a document carries.
///
/// The tuple set §15.2's consistency tests compare across formats. Asserted here between the
/// projection and the document, which is the half Stage 1 can prove.
fn positions(document: &OutputDocument) -> BTreeSet<(Option<Uuid>, Uuid, u16, u16, Uuid)> {
    let mut out = BTreeSet::new();
    for sheet in &document.sheets {
        for panel in &sheet.panels {
            for cell in &panel.cells {
                for lesson in &cell.lessons {
                    out.insert((
                        sheet.scope.id,
                        panel.schedule_id,
                        cell.day_ordinal,
                        cell.period_ordinal,
                        lesson.activity_id,
                    ));
                }
            }
        }
    }
    out
}

/// Every way one output panel would render as a broken table. Empty for a sound one.
///
/// The same four rules `GridPanel::malformations` states, asked of the composed panel — which
/// is the thing a renderer will actually walk. Deliberately a test helper rather than a method
/// on [`OutputPanel`]: the rule has one implementation, in `lay_out`, and this only checks that
/// composition did not break what `lay_out` decided.
fn malformations(panel: &OutputPanel) -> Vec<String> {
    let mut problems = Vec::new();
    let days: Vec<u16> = panel.days.iter().map(|d| d.ordinal).collect();
    let periods: Vec<u16> = panel.periods.iter().map(|p| p.ordinal).collect();

    if panel.cells.len() != days.len() * periods.len() {
        problems.push(format!(
            "{} cells for a {}x{} grid",
            panel.cells.len(),
            periods.len(),
            days.len()
        ));
    }
    let mut seen: BTreeSet<(u16, u16)> = BTreeSet::new();
    for cell in &panel.cells {
        if !seen.insert((cell.period_ordinal, cell.day_ordinal)) {
            problems.push(format!(
                "two cells for day {} period {}",
                cell.day_ordinal, cell.period_ordinal
            ));
        }
    }
    for period in &periods {
        for day in &days {
            if !seen.contains(&(*period, *day)) {
                problems.push(format!("no cell for day {day} period {period}"));
            }
        }
    }

    let mut covered: BTreeSet<(u16, u16)> = BTreeSet::new();
    for cell in &panel.cells {
        if cell.kind != OutputCellKind::Lesson {
            continue;
        }
        let Some(from) = periods.iter().position(|p| *p == cell.period_ordinal) else {
            continue;
        };
        let rows = usize::from(cell.rows);
        if rows == 0 {
            problems.push("a lesson spans no rows".to_owned());
            continue;
        }
        if from + rows > periods.len() {
            problems.push(format!(
                "a span of {rows} rows from row {from} runs past the end"
            ));
            continue;
        }
        for period in periods.iter().skip(from + 1).take(rows - 1) {
            if !covered.insert((*period, cell.day_ordinal)) {
                problems.push(format!(
                    "day {} period {period} is covered twice",
                    cell.day_ordinal
                ));
            }
            match panel.cell(cell.day_ordinal, *period).map(|c| c.kind) {
                Some(OutputCellKind::Continuation) => {}
                other => problems.push(format!(
                    "day {} period {period} is covered by a span but holds {other:?}",
                    cell.day_ordinal
                )),
            }
        }
    }
    for cell in &panel.cells {
        if cell.kind == OutputCellKind::Continuation
            && !covered.contains(&(cell.period_ordinal, cell.day_ordinal))
        {
            problems.push(format!(
                "day {} period {} continues nothing",
                cell.day_ordinal, cell.period_ordinal
            ));
        }
    }
    problems
}

// =======================================================================================
// T-1 — every fixture composes, at every scope kind
// =======================================================================================

#[test]
fn every_fixture_composes_at_every_scope_kind() {
    let mut composed = 0_usize;
    let mut fixtures = 0_usize;
    for found in all_fixtures(SEED) {
        fixtures += 1;
        let p = project_fixture(&found, 6);
        for kind in EVERY_KIND {
            let options = OutputOptions::for_kind(kind);
            match p.compose(&options) {
                Ok(document) => {
                    // One sheet per entity the projection offers, in that order.
                    assert_eq!(
                        document.sheets.len(),
                        p.scopes(kind).len(),
                        "{} / {kind:?}: a sheet per selected entity",
                        found.name
                    );
                    for panel in document.sheets.iter().flat_map(|s| &s.panels) {
                        assert!(
                            malformations(panel).is_empty(),
                            "{} / {kind:?}: {:?}",
                            found.name,
                            malformations(panel)
                        );
                    }
                    composed += 1;
                }
                // The two refusals a fixture may legitimately produce, and nothing else.
                Err(OutputError::NoEnrolments) => {
                    assert_eq!(kind, ScopeKind::Student, "{}", found.name);
                    assert!(!p.projection.has_enrolments, "{}", found.name);
                }
                Err(OutputError::EmptySelection { detail }) => {
                    // A fixture with no sections, rooms or days of its own. The refusal names
                    // which part of the selection emptied it.
                    assert!(!detail.is_empty(), "{} / {kind:?}", found.name);
                }
                Err(other) => panic!("{} / {kind:?} refused unexpectedly: {other}", found.name),
            }
        }
    }
    assert_eq!(fixtures, 24, "the documented catalogue is 24 fixtures");
    assert!(composed > 0, "nothing composed at all");
}

#[test]
fn every_fixture_composes_a_master_grid() {
    // The one scope every fixture with a cycle offers, so the sweep has a floor that is not
    // satisfied by refusals.
    let mut composed = 0_usize;
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 0);
        match p.compose(&OutputOptions::for_kind(ScopeKind::Master)) {
            Ok(document) => {
                assert_eq!(document.sheets.len(), 1, "{}", found.name);
                assert_eq!(document.header.school_name, p.projection.school_name);
                composed += 1;
            }
            Err(OutputError::EmptySelection { .. }) => {
                // `empty-grid` has no days at all, which is the fixture's whole point.
                assert!(
                    p.projection.days.is_empty() || p.projection.axes.is_empty(),
                    "{} refused a master grid with days and axes present",
                    found.name
                );
            }
            Err(other) => panic!("{}: {other}", found.name),
        }
    }
    assert!(
        composed >= 20,
        "only {composed} fixtures composed a master grid"
    );
}

// =======================================================================================
// T-2 — conservation
// =======================================================================================

#[test]
fn nothing_the_grid_holds_is_lost_on_the_way_out() {
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 6);
        for kind in EVERY_KIND {
            let Ok(document) = p.compose(&OutputOptions::for_kind(kind)) else {
                continue;
            };

            // The denominator: every occurrence the scope grids hold, straight from `lay_out`.
            let scopes = p.scopes(kind);
            let grids = lay_out_many(&p.projection, &scopes, &p.labels);
            let mut expected: BTreeSet<Uuid> = BTreeSet::new();
            for grid in &grids {
                for cell in grid.panels.iter().flat_map(|panel| &panel.cells) {
                    let indexes = match &cell.content {
                        CellContent::Empty | CellContent::Continuation => Vec::new(),
                        CellContent::Lesson { lesson, .. } => vec![*lesson],
                        CellContent::Pinned { lessons, .. } => lessons.clone(),
                    };
                    for ix in indexes {
                        expected.insert(p.projection.lessons[ix].activity_id);
                    }
                }
            }

            assert_eq!(
                in_document(&document),
                expected,
                "{} / {kind:?}: the document must carry exactly what the grids hold",
                found.name
            );
            assert_eq!(
                document.omitted_lessons(),
                0,
                "{} / {kind:?}: the default options exclude nothing",
                found.name
            );
        }
    }
}

#[test]
fn a_cells_occupied_positions_are_all_carried_even_where_the_span_is_one_cell() {
    // The distinction the proposal insists on: a cell's *visual span* and the *positions* an
    // occurrence occupies are different things, and a pinned or broken run has the second
    // without the first. A document that carried only the span would lose the rest.
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 0);
        let Ok(document) = p.compose(&OutputOptions::for_kind(ScopeKind::Master)) else {
            continue;
        };
        for sheet in &document.sheets {
            for panel in &sheet.panels {
                for cell in &panel.cells {
                    for lesson in &cell.lessons {
                        let source = p
                            .projection
                            .lessons
                            .iter()
                            .find(|l| l.activity_id == lesson.activity_id)
                            .expect("the occurrence is in the projection");
                        assert!(
                            source.occupies.iter().any(|slot| {
                                slot.day_ordinal == cell.day_ordinal
                                    && slot.period_ordinal == cell.period_ordinal
                            }),
                            "{}: a cell carries an occurrence that does not occupy it",
                            found.name
                        );
                        assert_eq!(
                            usize::from(lesson.parts),
                            source.occupies.len(),
                            "{}: `parts` is the run's length",
                            found.name
                        );
                        assert!(
                            lesson.part >= 1 && lesson.part <= lesson.parts,
                            "{}: part {} of {}",
                            found.name,
                            lesson.part,
                            lesson.parts
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_occupied_position_of_every_run_appears_in_the_document() {
    // The other direction of conservation: not only is nothing invented, nothing is dropped.
    // Counted over the master grid, where every placed occurrence is in scope.
    let p = projected("tiny", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("tiny composes");
    let mut expected: BTreeSet<(Uuid, u16, u16)> = BTreeSet::new();
    for lesson in &p.projection.lessons {
        for slot in &lesson.occupies {
            expected.insert((lesson.activity_id, slot.day_ordinal, slot.period_ordinal));
        }
    }
    let mut got: BTreeSet<(Uuid, u16, u16)> = BTreeSet::new();
    for cell in document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|p| &p.cells)
    {
        for lesson in &cell.lessons {
            got.insert((lesson.activity_id, cell.day_ordinal, cell.period_ordinal));
        }
    }
    assert_eq!(
        got, expected,
        "every occupied slot is addressable in the document"
    );
}

#[test]
fn a_conflicting_cell_keeps_every_occupant() {
    let p = projected("medium", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Teacher))
        .expect("medium composes");

    let shared: Vec<&OutputCell> = document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|p| &p.cells)
        .filter(|cell| cell.occupants() > 1)
        .collect();
    assert!(
        !shared.is_empty(),
        "the dense arrangement puts two lessons in one teacher's cell"
    );
    for cell in shared {
        assert_eq!(
            cell.state_key(),
            "conflict",
            "a teacher expects exclusive use"
        );
        assert_eq!(cell.rows, 1, "a pinned cell never carries a span");
        let mut ids: Vec<Uuid> = cell.lessons.iter().map(|l| l.activity_id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "no occupant is listed twice");
    }

    // And the clash reaches the appendix, naming every occurrence by stored identifier.
    let conflicts: Vec<&ConflictLine> = document.sheets.iter().flat_map(|s| &s.conflicts).collect();
    assert!(!conflicts.is_empty(), "the clashes reach the appendix");
    for conflict in conflicts {
        assert!(
            conflict.activity_ids.len() > 1,
            "a clash names every occurrence"
        );
        assert_eq!(conflict.activity_ids.len(), conflict.captions.len());
    }
}

#[test]
fn several_lessons_in_one_master_cell_are_shared_and_not_a_conflict() {
    let p = projected("medium", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("medium composes");
    let stacked: Vec<&OutputCell> = document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|p| &p.cells)
        .filter(|cell| cell.occupants() > 1)
        .collect();
    assert!(
        !stacked.is_empty(),
        "a master grid stacks lessons in one cell"
    );
    for cell in stacked {
        assert_eq!(
            cell.state_key(),
            "shared",
            "several lessons at once is the ordinary state of a master grid"
        );
    }
    assert!(
        document.sheets.iter().all(|s| s.conflicts.is_empty()),
        "a master grid reports no conflicts, or the word would be useless"
    );
}

// =======================================================================================
// T-3 — weekly and custom rotation cycles
// =======================================================================================

#[test]
fn a_custom_rotation_is_columns_in_ordinal_order_and_names_no_weekday() {
    let found = fixture("custom-rotating-cycle", SEED).expect("in the catalogue");
    assert_eq!(found.problem().context.cycle.kind, CycleKind::Custom);
    let p = project_fixture(&found, 4);

    for kind in EVERY_KIND {
        let Ok(document) = p.compose(&OutputOptions::for_kind(kind)) else {
            continue;
        };
        for panel in document.sheets.iter().flat_map(|s| &s.panels) {
            let ordinals: Vec<u16> = panel.days.iter().map(|d| d.ordinal).collect();
            let mut sorted = ordinals.clone();
            sorted.sort_unstable();
            assert_eq!(ordinals, sorted, "{kind:?}: columns are in ordinal order");
        }

        // The negative, proved over the whole serialised document rather than field by field:
        // nothing in scheduling may read a weekday, so no weekday name may reach the output.
        let json = serde_json::to_string(&document).expect("a document serialises");
        for weekday in [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ] {
            assert!(
                !json.contains(weekday),
                "{kind:?}: {weekday} reached a document composed from a rotation cycle"
            );
        }
        assert!(
            !json.contains("weekdayHint") && !json.contains("weekday_hint"),
            "{kind:?}: a weekday hint is display on a cycle day and is never an output"
        );
    }
}

#[test]
fn a_weekly_cycle_keeps_its_day_labels_and_its_ordinals() {
    let p = projected("tiny", 0);
    assert_eq!(p.problem.context.cycle.kind, CycleKind::Weekly);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("tiny composes");
    let panel = &document.sheets[0].panels[0];
    assert_eq!(
        panel.days.iter().map(|d| d.ordinal).collect::<Vec<_>>(),
        p.projection
            .days
            .iter()
            .map(|d| d.ordinal)
            .collect::<Vec<_>>(),
        "the whole cycle, in the cycle's order"
    );
    for (composed, source) in panel.days.iter().zip(&p.projection.days) {
        assert_eq!(
            composed.label, source.label,
            "a label is carried, not composed"
        );
    }
}

// =======================================================================================
// T-31 — ScopeGrid to OutputSheet consistency
// =======================================================================================

#[test]
fn one_output_cell_per_timetable_cell_in_the_same_order() {
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 4);
        for kind in EVERY_KIND {
            let scopes = p.scopes(kind);
            let Ok(document) = p.compose(&OutputOptions::for_kind(kind)) else {
                continue;
            };
            let grids = lay_out_many(&p.projection, &scopes, &p.labels);
            assert_eq!(document.sheets.len(), grids.len(), "{}", found.name);

            for (sheet, grid) in document.sheets.iter().zip(&grids) {
                assert_eq!(sheet.scope, grid.scope, "{}", found.name);
                assert_eq!(sheet.panels.len(), grid.panels.len(), "{}", found.name);
                for (out, source) in sheet.panels.iter().zip(&grid.panels) {
                    assert_eq!(out.schedule_id, source.schedule_id);
                    assert_eq!(out.schedule_name, source.schedule_name);
                    assert_eq!(out.campus_name, source.campus_name);
                    assert_eq!(out.days, source.days, "{}", found.name);
                    assert_eq!(out.periods, source.periods, "{}", found.name);
                    assert_eq!(out.cells.len(), source.cells.len(), "{}", found.name);
                    for (cell, from) in out.cells.iter().zip(&source.cells) {
                        assert_eq!(cell.day_ordinal, from.day_ordinal);
                        assert_eq!(cell.period_ordinal, from.period_ordinal);
                        // The content kind, mapped and nothing else — with `Empty` split into
                        // "nothing is placed" and "nothing may be", which the period row
                        // already said.
                        let expected = match &from.content {
                            CellContent::Empty => {
                                let teaching = out
                                    .periods
                                    .iter()
                                    .find(|p| p.ordinal == cell.period_ordinal)
                                    .is_some_and(|p| p.is_teaching);
                                if teaching {
                                    OutputCellKind::Empty
                                } else {
                                    OutputCellKind::NonTeaching
                                }
                            }
                            CellContent::Lesson { .. } => OutputCellKind::Lesson,
                            CellContent::Continuation => OutputCellKind::Continuation,
                            CellContent::Pinned { reason, .. } => {
                                OutputCellKind::Pinned { reason: *reason }
                            }
                        };
                        assert_eq!(cell.kind, expected, "{}", found.name);
                        // The span is `lay_out`'s decision, carried verbatim.
                        let rows = match from.content {
                            CellContent::Lesson { rows, .. } => rows,
                            _ => 1,
                        };
                        assert_eq!(cell.rows, rows, "{}", found.name);
                    }
                }
            }
        }
    }
}

#[test]
fn a_continuation_cell_carries_the_occurrence_that_covers_it() {
    let found = all_fixtures(SEED)
        .into_iter()
        .map(|found| project_fixture(&found, 0))
        .find(|p| {
            p.compose(&OutputOptions::for_kind(ScopeKind::Section))
                .is_ok_and(|document| {
                    document
                        .sheets
                        .iter()
                        .flat_map(|s| &s.panels)
                        .flat_map(|p| &p.cells)
                        .any(|cell| cell.kind == OutputCellKind::Continuation)
                })
        });
    let Some(p) = found else {
        // No fixture in the catalogue places a multi-period run whose cell earns a span under
        // a dense arrangement. Said rather than passed silently.
        panic!("no fixture produced a continuation cell; the assertion below never ran");
    };
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Section))
        .expect("composes");
    let mut checked = 0_usize;
    for panel in document.sheets.iter().flat_map(|s| &s.panels) {
        for cell in &panel.cells {
            if cell.kind != OutputCellKind::Continuation {
                continue;
            }
            assert_eq!(
                cell.lessons.len(),
                1,
                "a continuation is covered by exactly one span"
            );
            let carried = &cell.lessons[0];
            assert!(carried.part > 1, "a continuation is never part 1");
            assert!(carried.parts > 1);
            // The covering cell is above it in the same column and reaches this row.
            let covering = panel
                .cells
                .iter()
                .find(|c| {
                    c.day_ordinal == cell.day_ordinal
                        && c.kind == OutputCellKind::Lesson
                        && c.lessons
                            .first()
                            .is_some_and(|l| l.activity_id == carried.activity_id)
                })
                .expect("the covering lesson cell");
            assert!(covering.period_ordinal < cell.period_ordinal);
            checked += 1;
        }
    }
    assert!(checked > 0);
}

// =======================================================================================
// T-28 — CellLines
// =======================================================================================

#[test]
fn a_caption_is_the_sections_own_short_label() {
    let p = projected("tiny", 0);
    assert!(
        !p.projection.lessons.is_empty(),
        "tiny places lessons, or the assertions below are vacuous"
    );
    for lesson in &p.projection.lessons {
        let label = p
            .labels
            .section(lesson.section_id)
            .expect("every section is labelled in this set");
        assert_eq!(
            lesson.lines.caption, label.short,
            "the caption is `SectionLabel::short`, not a second composition"
        );
    }
}

#[test]
fn an_unlabelled_section_falls_back_to_its_identifier_and_not_a_placeholder() {
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    // A label set that holds nothing: the ordinary state of a set loaded for another scope.
    let labels = LabelSet::default();
    let assignment = found.expectations.feasible.as_ref().map_or_else(
        || a_dense_arrangement(&problem),
        |f| f.assignment(problem.activity_count()),
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert!(
        !projection.lessons.is_empty(),
        "the arrangement places lessons, or the assertions below are vacuous"
    );
    for lesson in &projection.lessons {
        assert!(lesson.label.is_none());
        assert_eq!(
            lesson.lines.caption,
            lesson.section_id.to_string(),
            "the identifier, which is visibly an identifier"
        );
        assert!(
            !lesson.lines.caption.contains("Unknown") && !lesson.lines.caption.contains("Untitled"),
            "never a placeholder a reader could mistake for a name"
        );
    }
}

#[test]
fn a_booked_room_with_no_label_is_named_by_its_identifier_rather_than_left_blank() {
    // A blank room reads as "no room was booked", which is a different fact. The caption
    // already takes this decision for a section; the room takes the same one.
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    let labels = LabelSet::default();
    let assignment = a_dense_arrangement(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let booked: Vec<_> = projection
        .lessons
        .iter()
        .filter(|l| l.room_id.is_some())
        .collect();
    assert!(!booked.is_empty(), "the dense arrangement books rooms");
    for lesson in booked {
        assert!(lesson.room.is_none(), "the set holds no room labels");
        assert_eq!(
            lesson.lines.room.as_deref(),
            Some(lesson.room_id.expect("booked").to_string()).as_deref()
        );
    }
    // And where no room was booked at all, the line is absent rather than an identifier.
    for lesson in projection.lessons.iter().filter(|l| l.room_id.is_none()) {
        assert!(lesson.lines.room.is_none());
    }
}

#[test]
fn the_lines_carry_as_many_teachers_and_groups_as_the_section_has() {
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 0);
        let Ok(document) = p.compose(&OutputOptions::for_kind(ScopeKind::Master)) else {
            continue;
        };
        for cell_lesson in document
            .sheets
            .iter()
            .flat_map(|s| &s.panels)
            .flat_map(|p| &p.cells)
            .flat_map(|c| &c.lessons)
        {
            assert_eq!(
                cell_lesson.lines.teachers.len(),
                cell_lesson.teacher_ids.len(),
                "{}: every teacher, never only the first",
                found.name
            );
            assert_eq!(
                cell_lesson.lines.groups.len(),
                cell_lesson.group_ids.len(),
                "{}: every attending group",
                found.name
            );
        }
    }
}

#[test]
fn a_co_taught_section_carries_both_teachers() {
    // No fixture in M2's catalogue gives one section two teachers — every `SectionSpec` names
    // exactly one — so the case is constructed here rather than asserted of a fixture that
    // does not cover it. Two teachers on an activity **is** representable (unlike two rooms),
    // and §4.3 requires every one of them to reach the output.
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let mut problem = found.problem().clone();
    assert!(problem.teachers.len() >= 2, "tiny employs two people");
    let second = problem
        .sections
        .iter()
        .flat_map(|s| s.teachers.iter())
        .copied()
        .find(|ix| *ix != problem.sections[0].teachers[0])
        .expect("a second teacher teaches somewhere");
    problem.sections[0].teachers.push(second);

    let labels = labels_for(&problem, 0);
    let assignment = a_dense_arrangement(&problem);
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let section_id = problem.sections[0].id;
    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_entity(ScopeKind::Section, section_id))
        .expect("composes");

    let co_taught: Vec<&CellLesson> = document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|p| &p.cells)
        .flat_map(|c| &c.lessons)
        .filter(|lesson| lesson.section_id == section_id)
        .collect();
    assert!(
        !co_taught.is_empty(),
        "the co-taught section reaches the grid"
    );
    for lesson in co_taught {
        assert_eq!(lesson.teacher_ids.len(), 2);
        assert_eq!(
            lesson.lines.teachers.len(),
            2,
            "both names, joined in order"
        );
        assert!(
            lesson
                .lines
                .one_line(CellDetail::Standard)
                .contains(&lesson.lines.teachers.join(", ")),
            "a one-line caption carries both as well"
        );
    }

    // And both teachers' own grids hold the lesson.
    for teacher in &p.problem.sections[0].teachers.clone() {
        let id = p.problem.teachers[teacher.get()].id;
        let theirs = p
            .compose(&OutputOptions::for_entity(ScopeKind::Teacher, id))
            .expect("composes");
        assert!(
            in_document(&theirs)
                .iter()
                .any(|activity| co_taught_holds(&document, *activity)),
            "a co-taught lesson is in both teachers' timetables"
        );
    }
}

/// Whether a document carries one occurrence.
fn co_taught_holds(document: &OutputDocument, activity: Uuid) -> bool {
    in_document(document).contains(&activity)
}

/// The first fixture problem holding a double or triple period, and that occurrence.
///
/// Searched rather than named, so that a fixture whose durations change does not quietly turn
/// a span assertion into a vacuous one.
fn a_problem_with_a_multi_period_occurrence() -> (Problem, ActivityIx) {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            if let Some(activity) = problem
                .activities
                .iter()
                .enumerate()
                .find(|(_, fact)| fact.duration.as_usize() > 1)
                .and_then(|(ix, _)| u32::try_from(ix).ok().map(ActivityIx::new))
            {
                return (problem.clone(), activity);
            }
        }
    }
    panic!("no fixture in the catalogue holds a multi-period occurrence");
}

#[test]
fn a_one_line_caption_grows_with_the_detail_asked_for() {
    let lines = CellLines {
        caption: "9 Mathematics B".to_owned(),
        activity_name: Some("Double period".to_owned()),
        teachers: vec!["A. Okafor".to_owned(), "B. Fenwick".to_owned()],
        groups: vec!["9A".to_owned()],
        room: Some("Room 12".to_owned()),
        time: "09:00\u{2013}10:00".to_owned(),
    };
    assert_eq!(lines.one_line(CellDetail::Compact), "9 Mathematics B");
    assert_eq!(
        lines.one_line(CellDetail::Standard),
        "9 Mathematics B \u{b7} A. Okafor, B. Fenwick \u{b7} Room 12"
    );
    assert_eq!(
        lines.one_line(CellDetail::Full),
        "9 Mathematics B \u{b7} Double period \u{b7} A. Okafor, B. Fenwick \u{b7} 9A \
         \u{b7} Room 12 \u{b7} 09:00\u{2013}10:00"
    );
}

#[test]
fn a_one_line_caption_leaves_no_empty_separator_where_a_part_is_missing() {
    let lines = CellLines {
        caption: "Art A".to_owned(),
        activity_name: None,
        teachers: Vec::new(),
        groups: Vec::new(),
        room: None,
        time: "09:00\u{2013}10:00".to_owned(),
    };
    for detail in CellDetail::all() {
        let line = lines.one_line(*detail);
        assert!(!line.contains("\u{b7} \u{b7}"), "{detail:?}: {line:?}");
        assert!(!line.starts_with(' ') && !line.ends_with(' '), "{line:?}");
    }
    assert_eq!(lines.one_line(CellDetail::Standard), "Art A");
    assert_eq!(
        lines.one_line(CellDetail::Full),
        "Art A \u{b7} 09:00\u{2013}10:00"
    );
}

#[test]
fn the_time_line_is_the_span_the_resolver_produced() {
    let p = projected("tiny", 0);
    for lesson in &p.projection.lessons {
        assert_eq!(
            lesson.lines.time,
            format!("{}\u{2013}{}", lesson.starts_at, lesson.ends_at),
            "derived from the periods covered, never from a duration"
        );
    }
}

#[test]
fn a_cell_detail_key_round_trips() {
    for detail in CellDetail::all() {
        assert_eq!(CellDetail::from_key(detail.key()), Some(*detail));
    }
    assert_eq!(CellDetail::from_key("verbose"), None);
}

// =======================================================================================
// Scope selection, empty scopes and invalid selections
// =======================================================================================

#[test]
fn a_selected_teacher_with_no_lessons_is_present_and_marked_empty() {
    // The distinction §8.4 insists on: a missing sheet reads as "this teacher was not
    // exported", which is the opposite of what a school exporting everybody wants to learn.
    //
    // The idle teacher is produced deliberately. Under a dense arrangement every teacher who
    // holds a section has lessons, so the state is reached by placing **one** occurrence and
    // leaving the rest unplaced — which is also exactly what a partial solve produces.
    let found = fixture("medium", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    let labels = labels_for(&problem, 0);
    let mut assignment = Assignment::empty(problem.activity_count());
    let first_slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, slot)| slot.is_teaching)
        .and_then(|(ix, _)| u32::try_from(ix).ok())
        .map(TimeslotIx::new)
        .expect("the fixture has a teaching slot");
    assignment.place(
        ActivityIx::new(0),
        Placement {
            timeslot: first_slot,
            room: None,
        },
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let p = Projected {
        problem,
        labels,
        projection,
    };

    let idle: Vec<ProjectionScope> = p
        .scopes(ScopeKind::Teacher)
        .into_iter()
        .filter(|scope| scope.lesson_count == 0)
        .collect();
    let busy = p
        .scopes(ScopeKind::Teacher)
        .into_iter()
        .find(|scope| scope.lesson_count > 0)
        .expect("the one placed occurrence has a teacher");

    let chosen: Vec<Uuid> = idle
        .iter()
        .take(2)
        .chain(std::iter::once(&busy))
        .filter_map(|scope| scope.id)
        .collect();
    assert_eq!(chosen.len(), 3, "two idle teachers and one with a lesson");

    let document = p
        .compose(&OutputOptions {
            scope_ids: chosen.clone(),
            ..OutputOptions::for_kind(ScopeKind::Teacher)
        })
        .expect("composes");
    assert_eq!(document.sheets.len(), chosen.len(), "every selected entity");
    assert_eq!(
        document
            .sheets
            .iter()
            .filter_map(|s| s.scope.id)
            .collect::<Vec<_>>(),
        chosen,
        "in the caller's order"
    );
    for sheet in &document.sheets {
        if sheet.scope.lesson_count == 0 {
            assert!(sheet.is_empty, "{:?}", sheet.scope.name);
            assert!(
                sheet.panels.is_empty(),
                "a scope holding nothing has no panel to draw"
            );
            assert_eq!(sheet.lesson_count, 0);
            assert_eq!(sheet.omitted_lesson_count, 0, "nothing was filtered away");
        } else {
            assert!(!sheet.is_empty);
            assert!(
                !sheet.panels.is_empty(),
                "a scope holding something has a panel"
            );
        }
    }
}

#[test]
fn an_unselected_entity_has_no_sheet_at_all() {
    let p = projected("medium", 0);
    let scopes = p.scopes(ScopeKind::Room);
    assert!(scopes.len() > 1);
    let one = scopes[0].id.expect("a room has an id");
    let document = p
        .compose(&OutputOptions::for_entity(ScopeKind::Room, one))
        .expect("composes");
    assert_eq!(document.sheets.len(), 1);
    assert_eq!(document.sheets[0].scope.id, Some(one));
}

#[test]
fn a_scope_the_projection_does_not_offer_is_refused_by_name() {
    let p = projected("tiny", 0);
    let nobody = Uuid::from_bytes([0xAB; 16]);
    let error = p
        .compose(&OutputOptions::for_entity(ScopeKind::Teacher, nobody))
        .expect_err("an unknown teacher is refused");
    assert_eq!(
        error,
        OutputError::UnknownScope {
            kind: ScopeKind::Teacher,
            id: nobody
        }
    );
    assert!(error.to_string().contains(&nobody.to_string()));
}

#[test]
fn a_master_grid_names_no_entity_and_refuses_one() {
    let p = projected("tiny", 0);
    let anything = Uuid::from_bytes([3; 16]);
    assert!(matches!(
        p.compose(&OutputOptions::for_entity(ScopeKind::Master, anything)),
        Err(OutputError::UnknownScope {
            kind: ScopeKind::Master,
            ..
        })
    ));
}

#[test]
fn a_duplicated_selection_produces_one_sheet() {
    let p = projected("tiny", 0);
    let one = p.scopes(ScopeKind::Section)[0].id.expect("an id");
    let document = p
        .compose(&OutputOptions {
            scope_ids: vec![one, one, one],
            ..OutputOptions::for_kind(ScopeKind::Section)
        })
        .expect("composes");
    assert_eq!(
        document.sheets.len(),
        1,
        "two identical sheets are one sheet"
    );
}

#[test]
fn an_unknown_bell_schedule_is_refused() {
    let p = projected("tiny", 0);
    let nothing = Uuid::from_bytes([0xCD; 16]);
    assert_eq!(
        p.compose(&OutputOptions {
            schedule_ids: vec![nothing],
            ..OutputOptions::for_kind(ScopeKind::Master)
        }),
        Err(OutputError::UnknownSchedule { id: nothing })
    );
}

#[test]
fn a_day_the_cycle_does_not_hold_is_refused_rather_than_dropped() {
    let p = projected("tiny", 0);
    let beyond = u16::try_from(p.projection.days.len()).unwrap_or(0) + 50;
    assert_eq!(
        p.compose(&OutputOptions {
            day_ordinals: vec![beyond],
            ..OutputOptions::for_kind(ScopeKind::Master)
        }),
        Err(OutputError::UnknownDay { ordinal: beyond })
    );
}

#[test]
fn a_student_document_is_refused_when_no_enrolment_row_exists() {
    // T-26. Not an empty picker and not a blank grid: a sentence, because the only way to fill
    // the gap would be to infer a child's schedule from a group they might be in.
    let p = projected("tiny", 0);
    assert!(!p.projection.has_enrolments);
    let error = p
        .compose(&OutputOptions::for_kind(ScopeKind::Student))
        .expect_err("refused");
    assert_eq!(error, OutputError::NoEnrolments);
    assert!(error.to_string().contains("enrolment"));
}

#[test]
fn a_student_grid_is_exactly_the_sections_the_roster_names() {
    // T-27: the one test whose failure is a child being shown a guess.
    let p = projected("tiny", 4);
    assert!(p.projection.has_enrolments);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Student))
        .expect("composes");
    assert_eq!(document.sheets.len(), 4);

    for sheet in &document.sheets {
        let id = sheet.scope.id.expect("a student has an id");
        let enrolled: BTreeSet<Uuid> = p
            .labels
            .students
            .iter()
            .find(|s| s.student_id == id)
            .expect("in the roster")
            .sections
            .iter()
            .copied()
            .collect();
        let shown: BTreeSet<Uuid> = sheet
            .panels
            .iter()
            .flat_map(|panel| &panel.cells)
            .flat_map(|cell| &cell.lessons)
            .map(|lesson| lesson.section_id)
            .collect();
        assert!(
            shown.is_subset(&enrolled),
            "{}: a section nobody enrolled them in reached their timetable",
            sheet.scope.name
        );
        assert!(sheet.scope.roster_derived);
        assert!(
            sheet
                .disclosures
                .iter()
                .any(|d| d.code == DisclosureCode::RosterDerived && d.is_mandatory),
            "{}: the caveat is mandatory",
            sheet.scope.name
        );
    }
}

#[test]
fn a_student_timetable_is_never_derived_from_a_group() {
    // The negative: a group every student in the fixture attends, and a student enrolled in
    // nothing, must still have an empty grid.
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    let mut labels = labels_for(&problem, 1);
    labels.students.push(StudentLabel {
        student_id: Uuid::from_bytes([0xDD; 16]),
        name: "Student with no record".to_owned(),
        code: None,
        // No enrolment row, though the groups of this school are full of children.
        sections: Vec::new(),
    });
    let assignment = found.expectations.feasible.as_ref().map_or_else(
        || a_dense_arrangement(&problem),
        |f| f.assignment(problem.activity_count()),
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_entity(
            ScopeKind::Student,
            Uuid::from_bytes([0xDD; 16]),
        ))
        .expect("composes");
    assert_eq!(document.sheets.len(), 1);
    assert!(
        document.sheets[0].is_empty,
        "no enrolment row means an empty grid, never a group's timetable"
    );
    assert_eq!(in_document(&document).len(), 0);
}

// =======================================================================================
// Multi-campus and school isolation
// =======================================================================================

#[test]
fn two_campuses_on_two_bell_schedules_are_two_panels() {
    let p = projected("two-campuses-different-bells", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let panels = &document.sheets[0].panels;
    assert_eq!(panels.len(), 2, "one panel per bell schedule, never merged");

    let campuses: BTreeSet<&str> = panels
        .iter()
        .filter_map(|panel| panel.campus_name.as_deref())
        .collect();
    assert_eq!(campuses.len(), 2, "each panel names its own campus");
    let schedules: BTreeSet<Uuid> = panels.iter().map(|panel| panel.schedule_id).collect();
    assert_eq!(schedules.len(), 2);

    // The merged axis ADR 0033 forbids would show as one panel holding both schedules'
    // periods. Asserted by the period identifiers being disjoint between the two.
    let first: BTreeSet<Uuid> = panels[0].periods.iter().map(|p| p.period_id).collect();
    let second: BTreeSet<Uuid> = panels[1].periods.iter().map(|p| p.period_id).collect();
    assert!(
        first.is_disjoint(&second),
        "two campuses' period rows are not one row"
    );
}

#[test]
fn identical_timings_are_still_two_panels() {
    // Identical periods are not evidence of one schedule: the two campuses hold different
    // `timeslot` rows over the same minutes.
    let p = projected("two-campuses-identical-bells", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    assert_eq!(document.sheets[0].panels.len(), 2);
}

#[test]
fn a_campus_is_selected_by_filtering_panels_and_not_by_a_scope_kind() {
    let p = projected("two-campuses-different-bells", 0);
    let all = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let one = all.sheets[0].panels[0].schedule_id;

    let narrowed = p
        .compose(&OutputOptions {
            schedule_ids: vec![one],
            ..OutputOptions::for_kind(ScopeKind::Master)
        })
        .expect("composes");
    assert_eq!(narrowed.sheets[0].panels.len(), 1);
    assert_eq!(narrowed.sheets[0].panels[0].schedule_id, one);

    // What the narrowing left out is counted and disclosed, not silently absent.
    assert!(narrowed.omitted_lessons() > 0);
    assert!(
        narrowed
            .disclosures
            .iter()
            .any(|d| d.code == DisclosureCode::LessonsOmitted && d.is_mandatory),
        "an intentional exclusion is still disclosed"
    );
    assert!(
        narrowed
            .disclosures
            .iter()
            .any(|d| d.code == DisclosureCode::PanelsOmitted),
    );
    assert!(
        in_document(&narrowed).is_subset(&in_document(&all)),
        "narrowing never invents a lesson"
    );
}

#[test]
fn a_shared_teacher_gets_a_disclosure_no_option_can_remove() {
    let p = projected("shared-teacher-across-schools", 0);
    let shared = p
        .problem
        .teachers
        .iter()
        .find(|t| !t.also_at.is_empty())
        .expect("the fixture shares a teacher");
    let other_school = shared.also_at[0].name.clone();

    // Every combination of the switches a caller has. None of them may remove it.
    for include_non_teaching in [true, false] {
        for include_logo in [true, false] {
            for include_unscheduled in [true, false] {
                for include_findings in [true, false] {
                    for detail in CellDetail::all() {
                        let document = p
                            .compose(&OutputOptions {
                                include_non_teaching,
                                include_logo,
                                include_unscheduled,
                                include_findings,
                                detail: *detail,
                                ..OutputOptions::for_entity(ScopeKind::Teacher, shared.id)
                            })
                            .expect("composes");
                        let sheet = &document.sheets[0];
                        let disclosure = sheet
                            .disclosures
                            .iter()
                            .find(|d| d.code == DisclosureCode::SharedEntity)
                            .expect("the cross-campus sentence is on the sheet");
                        assert!(disclosure.is_mandatory);
                        assert!(
                            disclosure.message.contains(&other_school),
                            "ADR 0032 rule 3: name the other school — {:?}",
                            disclosure.message
                        );
                        assert!(
                            disclosure.message.contains(&shared.name),
                            "and name the entity"
                        );
                        assert!(
                            document
                                .disclosures
                                .iter()
                                .any(|d| d.code == DisclosureCode::SharedEntity),
                            "and it reaches the cover as well"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_teacher_who_works_at_one_school_gets_no_cross_campus_sentence() {
    let p = projected("shared-teacher-across-schools", 0);
    let local = p
        .problem
        .teachers
        .iter()
        .find(|t| t.also_at.is_empty())
        .expect("the fixture has a local teacher");
    let document = p
        .compose(&OutputOptions::for_entity(ScopeKind::Teacher, local.id))
        .expect("composes");
    assert!(
        !document.sheets[0]
            .disclosures
            .iter()
            .any(|d| d.code == DisclosureCode::SharedEntity),
        "a warning a user learns to dismiss is worse than none"
    );
}

#[test]
fn a_document_carries_nothing_of_the_other_school_but_its_name_in_the_disclosure() {
    let found = fixture("shared-teacher-across-schools", SEED).expect("in the catalogue");
    assert_eq!(found.problems.len(), 2, "two schools, two problems");
    let p = project_fixture(&found, 0);
    let other = &found.problems[1];

    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let json = serde_json::to_string(&document).expect("serialises");

    assert_eq!(document.header.school_name, p.problem.context.school.name);
    assert_ne!(document.header.school_name, other.context.school.name);

    // Not one identifier from the other school's problem reaches the document — except the
    // teacher whose identity the two deliberately share, which is the fixture's whole point.
    let ours: BTreeSet<Uuid> = p.problem.sections.iter().map(|s| s.id).collect();
    for section in &other.sections {
        if ours.contains(&section.id) {
            continue;
        }
        assert!(
            !json.contains(&section.id.to_string()),
            "a section of the other school reached the document"
        );
    }
    for room in &other.rooms {
        assert!(!json.contains(&room.id.to_string()));
    }
    for slot in &other.timeslots {
        assert!(!json.contains(&slot.id.to_string()));
    }
}

// =======================================================================================
// Breaks, spans, broken runs, flags
// =======================================================================================

#[test]
fn a_break_is_a_row_by_default_and_is_marked_as_one() {
    let p = projected("tiny", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let panel = &document.sheets[0].panels[0];
    let breaks: Vec<&ProjectedPeriod> = panel.periods.iter().filter(|p| !p.is_teaching).collect();
    assert!(!breaks.is_empty(), "the fixture has a break");

    for period in breaks {
        let row: Vec<&OutputCell> = panel
            .cells
            .iter()
            .filter(|cell| cell.period_ordinal == period.ordinal)
            .collect();
        assert_eq!(row.len(), panel.days.len(), "a break is a complete row");
        for cell in row {
            assert_eq!(
                cell.state_key(),
                "non_teaching",
                "'nothing is scheduled' and 'nothing may be' must not look alike"
            );
        }
    }
}

#[test]
fn an_empty_non_teaching_row_is_omitted_only_when_asked_and_only_when_empty() {
    let p = projected("tiny", 0);
    let with = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let without = p
        .compose(&OutputOptions {
            include_non_teaching: false,
            ..OutputOptions::for_kind(ScopeKind::Master)
        })
        .expect("composes");

    let rows_with = with.sheets[0].panels[0].periods.len();
    let rows_without = without.sheets[0].panels[0].periods.len();
    assert!(rows_without < rows_with, "a break row is a row that can go");
    assert!(
        without.sheets[0].panels[0]
            .periods
            .iter()
            .all(|p| p.is_teaching),
        "the break rows that went were the empty ones"
    );

    // And nothing was lost with them: the omission is safe precisely because those rows held
    // nothing.
    assert_eq!(in_document(&without), in_document(&with));
    assert_eq!(without.omitted_lessons(), 0);
    assert!(
        without
            .disclosures
            .iter()
            .any(|d| d.code == DisclosureCode::NonTeachingOmitted),
    );
    for panel in without.sheets.iter().flat_map(|s| &s.panels) {
        assert!(
            malformations(panel).is_empty(),
            "{:?}",
            malformations(panel)
        );
    }
}

#[test]
fn a_non_teaching_row_that_holds_a_lesson_is_never_omitted() {
    // The case that makes the filter safe rather than lossy: a stored timetable whose bell
    // schedule moved can have an occurrence in a period that is now a break.
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    let labels = labels_for(&problem, 0);

    // Place one occurrence on a non-teaching slot, which is exactly what a moved grid does.
    let break_slot = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(_, slot)| !slot.is_teaching)
        .and_then(|(ix, _)| u32::try_from(ix).ok())
        .map(TimeslotIx::new)
        .expect("the fixture has a non-teaching slot");
    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        ActivityIx::new(0),
        Placement {
            timeslot: break_slot,
            room: None,
        },
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    let p = Projected {
        problem,
        labels,
        projection,
    };
    assert_eq!(p.projection.lessons.len(), 1);

    let document = p
        .compose(&OutputOptions {
            include_non_teaching: false,
            ..OutputOptions::for_kind(ScopeKind::Master)
        })
        .expect("composes");
    assert_eq!(
        in_document(&document).len(),
        1,
        "the occurrence on a break row survived the filter"
    );
    assert_eq!(document.omitted_lessons(), 0);
}

#[test]
fn a_multi_period_run_earns_a_span_equal_to_the_cells_it_covers() {
    let mut checked = 0_usize;
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 0);
        let Ok(document) = p.compose(&OutputOptions::for_kind(ScopeKind::Section)) else {
            continue;
        };
        for panel in document.sheets.iter().flat_map(|s| &s.panels) {
            for cell in panel.cells.iter().filter(|c| c.rows > 1) {
                assert_eq!(cell.kind, OutputCellKind::Lesson, "{}", found.name);
                assert_eq!(cell.lessons.len(), 1, "a span is earned only when alone");
                let lesson = &cell.lessons[0];
                assert_eq!(
                    usize::from(cell.rows),
                    usize::from(lesson.parts),
                    "{}: the span is the run",
                    found.name
                );
                assert_eq!(lesson.part, 1, "a span begins at its first part");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no fixture produced a merged cell");
}

#[test]
fn an_unsound_run_reaches_the_appendix_with_the_resolvers_own_verdict() {
    // Constructed, because no fixture in the catalogue ships a stored arrangement whose runs
    // have gone wrong — that is a state a *saved* timetable acquires after its grid moves, and
    // the catalogue describes inputs rather than stored timetables. A three-period occurrence
    // placed on the last teaching slot of a day is the simplest instance: the run is short.
    let (problem, long) = a_problem_with_a_multi_period_occurrence();
    let labels = labels_for(&problem, 0);
    // The last teaching slot of the first cycle day, so the run runs off the end.
    let last = problem
        .timeslots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.is_teaching && slot.cycle_day.get() == 0)
        .filter_map(|(ix, _)| u32::try_from(ix).ok())
        .next_back()
        .map(TimeslotIx::new)
        .expect("a teaching slot on the first day");

    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        long,
        Placement {
            timeslot: last,
            room: None,
        },
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 1);
    let defect = projection.lessons[0]
        .defect
        .expect("a run that ran off the end of the day is not sound");
    assert!(
        !defect.is_complete(),
        "got {} of {}",
        defect.got,
        defect.wanted
    );

    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Section))
        .expect("composes");

    let defective: Vec<&DefectLine> = document.sheets.iter().flat_map(|s| &s.defective).collect();
    assert_eq!(defective.len(), 1, "the unsound run reaches the appendix");
    let line = defective[0];
    assert!(!line.caption.is_empty(), "named, not numbered");
    assert_eq!(
        line.defect, defect,
        "the resolver's own verdict, carried rather than re-judged"
    );
    assert!(line.occupied_periods < line.duration_periods);
    assert!(!line.day_label.is_empty(), "the day is named, not numbered");

    // The occurrence is still in the grid, in every cell it actually occupies.
    assert_eq!(
        in_document(&document).len(),
        1,
        "a broken run is not discarded"
    );
}

#[test]
fn a_run_that_reaches_across_a_break_is_reported_and_still_rendered() {
    // A non-teaching period *inside* a run is a `SpanDefect` — ADR 0033 §5 — but it is not a
    // hole in the **axis**: the break is a row of the panel, so the occupied rows are still
    // consecutive and the lesson still earns a span. Both halves are asserted, because
    // conflating them is the easy mistake: the defect must be reported, and the table must
    // not be broken to report it.
    let (problem, activity) = a_problem_with_a_multi_period_occurrence();
    let labels = labels_for(&problem, 0);

    // A teaching slot immediately before a break on the same day.
    let before_break = problem
        .timeslots
        .iter()
        .enumerate()
        .find(|(ix, slot)| {
            slot.is_teaching
                && problem
                    .timeslots
                    .get(ix + 1)
                    .is_some_and(|next| !next.is_teaching && next.cycle_day == slot.cycle_day)
        })
        .and_then(|(ix, _)| u32::try_from(ix).ok())
        .map(TimeslotIx::new)
        .expect("a teaching slot immediately before a break on the same day");

    let mut assignment = Assignment::empty(problem.activity_count());
    assignment.place(
        activity,
        Placement {
            timeslot: before_break,
            room: None,
        },
    );
    let flags = no_flags();
    let projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 1);
    assert!(
        projection.lessons[0]
            .defect
            .is_some_and(|d| d.crosses_non_teaching),
        "a run reaching across a break is not sound: {:?}",
        projection.lessons[0].defect
    );

    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    assert_eq!(in_document(&document).len(), 1, "it is not discarded");
    assert_eq!(
        document.sheets[0].defective.len(),
        1,
        "and it is reported as unsound"
    );
    for panel in document.sheets.iter().flat_map(|s| &s.panels) {
        assert!(
            malformations(panel).is_empty(),
            "{:?}",
            malformations(panel)
        );
    }
}

#[test]
fn an_occurrence_whose_run_has_a_hole_in_the_axis_gets_no_span() {
    // The case `PinReason::BrokenSpan` exists for: a slot *inside* the run was never
    // materialised, so the occupied period rows are 1 and 3 and no single span describes
    // them. Reached by editing the projection's `occupies`, the technique `projection::tests`
    // already uses for it — a `SchoolBuilder` materialises a schedule whole, so the state
    // cannot be built from the catalogue.
    let (problem, activity) = a_problem_with_a_multi_period_occurrence();
    let labels = labels_for(&problem, 0);
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
    let flags = no_flags();
    let mut projection = project(ProjectionInput {
        problem: &problem,
        assignment: &assignment,
        labels: &labels,
        flags: &flags,
    });
    assert_eq!(projection.lessons.len(), 1);
    let axis = projection.axes.first().cloned().expect("one axis");
    assert!(axis.periods.len() >= 3, "the axis needs a row to skip");

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

    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");

    let occupied: Vec<&OutputCell> = document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|panel| &panel.cells)
        .filter(|cell| !cell.lessons.is_empty())
        .collect();
    assert_eq!(occupied.len(), 2, "each occupied row is a cell of its own");
    for cell in occupied {
        assert_eq!(cell.rows, 1, "a run with a hole in it earns no span");
        assert_eq!(cell.state_key(), "broken_run");
        assert_eq!(cell.lessons.len(), 1);
        assert_eq!(cell.lessons[0].parts, 2, "both parts are addressable");
    }
    assert!(
        document
            .sheets
            .iter()
            .flat_map(|s| &s.panels)
            .flat_map(|panel| &panel.cells)
            .all(|cell| cell.kind != OutputCellKind::Lesson),
        "no cell claims a span"
    );
    assert_eq!(in_document(&document).len(), 1, "and nothing is discarded");
    for panel in document.sheets.iter().flat_map(|s| &s.panels) {
        assert!(
            malformations(panel).is_empty(),
            "{:?}",
            malformations(panel)
        );
    }
}

#[test]
fn a_locked_or_manual_placement_reaches_the_document() {
    let found = fixture("tiny", SEED).expect("in the catalogue");
    let problem = found.problem().clone();
    let labels = labels_for(&problem, 0);
    let assignment = a_dense_arrangement(&problem);
    let first = problem.activities[0].id;
    let mut flags = BTreeMap::new();
    flags.insert(
        first,
        crate::projection::LessonFlags {
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
    let p = Projected {
        problem,
        labels,
        projection,
    };
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    let carried = document
        .sheets
        .iter()
        .flat_map(|s| &s.panels)
        .flat_map(|p| &p.cells)
        .flat_map(|c| &c.lessons)
        .find(|lesson| lesson.activity_id == first)
        .expect("the occurrence is in the master grid");
    assert!(carried.flags.is_locked);
    assert!(carried.flags.is_manual);
}

// =======================================================================================
// Appendices
// =======================================================================================

#[test]
fn an_unplaced_occurrence_is_a_section_of_the_document() {
    let p = projected("infeasible-duration", 0);
    let unscheduled = vec![
        UnscheduledOccurrence {
            activity_id: Uuid::from_bytes([1; 16]),
            label: Some("9 Mathematics B".to_owned()),
            cause: "no-contiguous-run".to_owned(),
            diagnostic_code: Some("duration-exceeds-longest-run".to_owned()),
            reason: Some("no run of 6 consecutive teaching periods exists".to_owned()),
        },
        UnscheduledOccurrence {
            activity_id: Uuid::from_bytes([2; 16]),
            label: None,
            cause: "blocked-by-placements".to_owned(),
            diagnostic_code: None,
            reason: None,
        },
        UnscheduledOccurrence {
            activity_id: Uuid::from_bytes([3; 16]),
            label: None,
            cause: "a-cause-from-a-newer-build".to_owned(),
            diagnostic_code: None,
            reason: None,
        },
    ];
    let document = compose(
        p.input(&unscheduled, &[]),
        &OutputOptions::for_kind(ScopeKind::Master),
    )
    .expect("composes");

    assert_eq!(document.unscheduled.len(), 3);

    // A property of the problem: impossible.
    assert!(document.unscheduled[0].is_impossible);
    assert!(document.unscheduled[0].known_cause);
    assert_eq!(
        document.unscheduled[0].diagnostic_code.as_deref(),
        Some("duration-exceeds-longest-run")
    );
    // A property of this arrangement: not impossible.
    assert!(!document.unscheduled[1].is_impossible);
    assert!(document.unscheduled[1].known_cause);
    // A cause this build does not know: reported, and never called impossible.
    assert!(!document.unscheduled[2].known_cause);
    assert!(
        !document.unscheduled[2].is_impossible,
        "overclaiming impossibility is the failure the distinction exists to prevent"
    );
}

#[test]
fn leaving_the_unplaced_occurrences_out_is_disclosed() {
    let p = projected("tiny", 0);
    let unscheduled = vec![UnscheduledOccurrence {
        activity_id: Uuid::from_bytes([1; 16]),
        label: None,
        cause: "not-reached".to_owned(),
        diagnostic_code: None,
        reason: None,
    }];
    let document = compose(
        p.input(&unscheduled, &[]),
        &OutputOptions {
            include_unscheduled: false,
            ..OutputOptions::for_kind(ScopeKind::Master)
        },
    )
    .expect("composes");
    assert!(
        document.unscheduled.is_empty(),
        "the caller asked for the section to be left out"
    );
    let disclosure = document
        .disclosures
        .iter()
        .find(|d| d.code == DisclosureCode::UnscheduledOmitted)
        .expect("an absent section reads as a feature that was not run");
    assert!(disclosure.is_mandatory);
    assert_eq!(disclosure.count, Some(1));
}

#[test]
fn a_finding_carries_the_rules_own_sentence_and_nothing_composed_here() {
    let p = projected("tiny", 0);
    let day = p.projection.days[0].cycle_day_id;
    let findings = vec![FindingView {
        type_key: "teacher-clash".to_owned(),
        known_type: true,
        level: "HARD".to_owned(),
        penalty: -1_000,
        message: Some("A. Okafor is in two places at once on Day 1.".to_owned()),
        detail: "two lessons at once".to_owned(),
        cycle_day_id: Some(day),
        starts_at: Some("09:00".to_owned()),
        ends_at: Some("10:00".to_owned()),
        activity_ids: vec![Uuid::from_bytes([4; 16])],
        entities: vec![FindingEntity {
            role: "subject".to_owned(),
            kind: "teacher".to_owned(),
            id: Uuid::from_bytes([5; 16]).to_string(),
        }],
        source: crate::findings::FindingSource::Explained,
    }];
    let document = compose(
        p.input(&[], &findings),
        &OutputOptions::for_kind(ScopeKind::Master),
    )
    .expect("composes");

    assert_eq!(document.findings.len(), 1);
    let line = &document.findings[0];
    assert_eq!(
        line.message.as_deref(),
        Some("A. Okafor is in two places at once on Day 1."),
        "ADR 0043: verbatim, never re-worded"
    );
    assert_eq!(line.level, "HARD");
    assert_eq!(line.penalty, -1_000);
    assert_eq!(
        line.day_label.as_deref(),
        Some(p.projection.days[0].label.as_str())
    );
    assert_eq!(line.activity_ids, vec![Uuid::from_bytes([4; 16])]);
}

#[test]
fn a_stored_finding_reaches_the_document_with_no_sentence_invented_for_it() {
    let p = projected("tiny", 0);
    let findings = vec![FindingView::from_stored(StoredFinding {
        type_key: "room-clash",
        level: "HARD",
        penalty: -1_000,
        detail: "two lessons in Room 1",
        cycle_day_id: None,
        starts_at: None,
        ends_at: None,
        entities: Vec::new(),
    })];
    let document = compose(
        p.input(&[], &findings),
        &OutputOptions::for_kind(ScopeKind::Master),
    )
    .expect("composes");
    assert_eq!(document.findings.len(), 1);
    assert!(
        document.findings[0].message.is_none(),
        "the stored tier renders the fact and not prose"
    );
    assert_eq!(document.findings[0].detail, "two lessons in Room 1");
}

#[test]
fn an_unknown_rule_is_reported_rather_than_dropped() {
    let p = projected("tiny", 0);
    let findings = vec![FindingView::from_stored(StoredFinding {
        type_key: "a-rule-from-a-newer-build",
        level: "HARD",
        penalty: -1,
        detail: "",
        cycle_day_id: None,
        starts_at: None,
        ends_at: None,
        entities: Vec::new(),
    })];
    let document = compose(
        p.input(&[], &findings),
        &OutputOptions::for_kind(ScopeKind::Master),
    )
    .expect("composes");
    assert_eq!(document.findings.len(), 1);
    assert!(!document.findings[0].known_type);
    assert_eq!(document.findings[0].type_key, "a-rule-from-a-newer-build");
}

// =======================================================================================
// Header, disclosures and determinism
// =======================================================================================

#[test]
fn the_header_names_the_school_the_grid_is_for_and_the_revision_it_was_read_at() {
    let p = projected("tiny", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Master))
        .expect("composes");
    assert_eq!(document.header.school_name, p.projection.school_name);
    assert_eq!(document.header.cycle_name, p.projection.cycle_name);
    assert_eq!(document.header.term_name, p.projection.term_name);
    assert_eq!(document.header.timetable_rev, 4);
    assert_eq!(document.header.generated_at, "2026-11-24T09:00:00.000Z");
    assert_eq!(
        document.header.provenance.as_ref().map(|p| p.seed),
        Some(42),
        "the seed, so the run can be reproduced"
    );
}

#[test]
fn every_document_says_it_is_a_snapshot() {
    let p = projected("tiny", 0);
    for kind in [ScopeKind::Master, ScopeKind::Section, ScopeKind::Teacher] {
        let document = p.compose(&OutputOptions::for_kind(kind)).expect("composes");
        let snapshot = document
            .disclosures
            .iter()
            .find(|d| d.code == DisclosureCode::Snapshot)
            .expect("a file on somebody's desk needs to say what it is");
        assert!(snapshot.is_mandatory);
        assert!(snapshot.message.contains("Spring 2027 Draft 3"));
        assert!(snapshot.message.contains("2026-11-24T09:00:00.000Z"));
    }
}

#[test]
fn the_logo_is_carried_only_when_the_caller_asked_for_it() {
    let p = projected("tiny", 0);
    let identity = DocumentIdentity {
        logo: Some(DocumentLogo {
            bytes: vec![0x89, 0x50, 0x4E, 0x47],
            media_type: "image/png".to_owned(),
            width: 64,
            height: 64,
        }),
        ..an_identity().clone()
    };
    let input = CompositionInput {
        problem: &p.problem,
        projection: &p.projection,
        labels: &p.labels,
        identity: &identity,
        unscheduled: &[],
        findings: &[],
    };
    let with = compose(input, &OutputOptions::for_kind(ScopeKind::Master)).expect("composes");
    assert!(with.header.logo.is_some());

    let without = compose(
        input,
        &OutputOptions {
            include_logo: false,
            ..OutputOptions::for_kind(ScopeKind::Master)
        },
    )
    .expect("composes");
    assert!(without.header.logo.is_none());
}

#[test]
fn composing_the_same_request_twice_produces_the_same_document() {
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 3);
        for kind in EVERY_KIND {
            let options = OutputOptions::for_kind(kind);
            let (Ok(first), Ok(second)) = (p.compose(&options), p.compose(&options)) else {
                continue;
            };
            assert_eq!(
                first, second,
                "{} / {kind:?} must be deterministic",
                found.name
            );
            assert_eq!(
                serde_json::to_string(&first).expect("serialises"),
                serde_json::to_string(&second).expect("serialises"),
                "{} / {kind:?}: byte-identical over the wire too",
                found.name
            );
        }
    }
}

#[test]
fn the_options_are_carried_so_a_format_can_print_its_own_provenance() {
    let p = projected("tiny", 4);
    let options = OutputOptions {
        format: Some(OutputFormat::Csv),
        detail: CellDetail::Full,
        include_non_teaching: false,
        page: PageSetup {
            paper: PaperSize::Letter,
            orientation: Orientation::Portrait,
        },
        ..OutputOptions::for_kind(ScopeKind::Student)
    };
    let document = p.compose(&options).expect("composes");
    assert_eq!(document.options, options);
}

// =======================================================================================
// Single-scope and bulk equivalence
// =======================================================================================

#[test]
fn laying_out_one_scope_and_a_batch_give_the_same_grids() {
    for found in all_fixtures(SEED) {
        let p = project_fixture(&found, 8);
        for kind in EVERY_KIND {
            let scopes = p.scopes(kind);
            let bulk = lay_out_many(&p.projection, &scopes, &p.labels);
            let one_at_a_time: Vec<_> = scopes
                .iter()
                .map(|scope| lay_out(&p.projection, scope, &p.labels))
                .collect();
            assert_eq!(
                bulk, one_at_a_time,
                "{} / {kind:?}: bulk layout is the same layout",
                found.name
            );
        }
    }
}

#[test]
fn a_document_of_one_entity_matches_that_entitys_part_of_the_bulk_document() {
    let p = projected("medium", 0);
    let bulk = p
        .compose(&OutputOptions::for_kind(ScopeKind::Teacher))
        .expect("composes");
    assert!(bulk.sheets.len() > 1);

    for sheet in &bulk.sheets {
        let id = sheet.scope.id.expect("a teacher has an id");
        let single = p
            .compose(&OutputOptions::for_entity(ScopeKind::Teacher, id))
            .expect("composes");
        assert_eq!(single.sheets.len(), 1);
        assert_eq!(
            &single.sheets[0], sheet,
            "{}: the individual case is the bulk case with one entity",
            sheet.scope.name
        );
    }
}

#[test]
fn the_position_tuples_are_the_projections_own() {
    // The half of §15.2's consistency property Stage 1 can prove: the document's
    // `(entity, schedule, day, period, activity)` set is exactly the projection's. Stage 2
    // adds the CSV-versus-XLSX half on top of this one.
    let p = projected("medium", 0);
    let document = p
        .compose(&OutputOptions::for_kind(ScopeKind::Room))
        .expect("composes");
    let mut expected: BTreeSet<(Option<Uuid>, Uuid, u16, u16, Uuid)> = BTreeSet::new();
    for scope in p.scopes(ScopeKind::Room) {
        let grid = lay_out(&p.projection, &scope, &p.labels);
        for panel in &grid.panels {
            for lesson in grid
                .panels
                .iter()
                .flat_map(|panel| &panel.cells)
                .flat_map(|cell| match &cell.content {
                    CellContent::Empty | CellContent::Continuation => Vec::new(),
                    CellContent::Lesson { lesson, .. } => vec![*lesson],
                    CellContent::Pinned { lessons, .. } => lessons.clone(),
                })
                .collect::<BTreeSet<_>>()
            {
                let source = &p.projection.lessons[lesson];
                if source.schedule_id != panel.schedule_id {
                    continue;
                }
                for slot in &source.occupies {
                    expected.insert((
                        scope.id,
                        panel.schedule_id,
                        slot.day_ordinal,
                        slot.period_ordinal,
                        source.activity_id,
                    ));
                }
            }
        }
    }
    assert_eq!(positions(&document), expected);
}

// =======================================================================================
// Vocabulary
// =======================================================================================

#[test]
fn the_cell_state_vocabulary_is_the_csv_columns_own() {
    // The six values §6.1 column 13 documents, plus `continuation` for the flat form's part
    // rows. Pinned at one place so a renderer cannot invent a seventh.
    let states: Vec<&str> = [
        OutputCellKind::Empty,
        OutputCellKind::NonTeaching,
        OutputCellKind::Lesson,
        OutputCellKind::Continuation,
        OutputCellKind::Pinned {
            reason: PinReason::Conflict,
        },
        OutputCellKind::Pinned {
            reason: PinReason::Shared,
        },
        OutputCellKind::Pinned {
            reason: PinReason::BrokenSpan,
        },
    ]
    .into_iter()
    .map(|kind| {
        OutputCell {
            day_ordinal: 1,
            period_ordinal: 1,
            kind,
            lessons: Vec::new(),
            rows: 1,
        }
        .state_key()
    })
    .collect();
    assert_eq!(
        states,
        vec![
            "empty",
            "non_teaching",
            "lesson",
            "continuation",
            "conflict",
            "shared",
            "broken_run"
        ]
    );
}

#[test]
fn a_format_key_round_trips() {
    for format in OutputFormat::all() {
        assert_eq!(OutputFormat::from_key(format.key()), Some(*format));
    }
    assert_eq!(
        OutputFormat::from_key("pdf"),
        None,
        "Stage 3 renders no bytes"
    );
}

#[test]
fn the_page_vocabulary_round_trips() {
    for paper in PaperSize::all() {
        assert_eq!(PaperSize::from_key(paper.key()), Some(*paper));
    }
    for orientation in Orientation::all() {
        assert_eq!(Orientation::from_key(orientation.key()), Some(*orientation));
    }
    assert_eq!(PaperSize::default(), PaperSize::A4);
    assert_eq!(Orientation::default(), Orientation::Landscape);
}

#[test]
fn the_defaults_are_the_request_that_needs_no_configuration() {
    let options = OutputOptions::default();
    assert_eq!(options.scope_kind, ScopeKind::Master);
    assert!(options.scope_ids.is_empty(), "every entity of the kind");
    assert!(options.schedule_ids.is_empty(), "every panel");
    assert!(options.day_ordinals.is_empty(), "the whole cycle");
    assert!(options.include_non_teaching);
    assert!(options.include_logo);
    assert!(options.include_unscheduled);
    assert!(options.include_findings);
    assert!(!options.include_empty);
    assert_eq!(options.detail, CellDetail::Standard);
    assert_eq!(options.page, PageSetup::default());
    assert!(
        options.format.is_none(),
        "the print view takes the document, not a format"
    );
}
