//! The composition pass: every judgement a renderer would otherwise make separately.
//!
//! # Why it is here and not in a renderer
//!
//! Six judgements have to be the same in the CSV, the workbook and the printed page, and three
//! of them are the ones a renderer gets wrong: which panels and days are in the document, what
//! a cell's lines say, how a pinned cell is described, which lessons a merged range may cover,
//! what the header says, and what the appendices carry. Made once here, a renderer afterwards
//! is a traversal with no decisions in it — which is what makes *"the view, the CSV, the
//! workbook and the page agree"* a structural property rather than a test that happens to
//! pass. `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §4.2.
//!
//! # One projection per document, however many entities
//!
//! The caller compiles once, projects once, and this module lays out per selected entity from
//! that one projection through [`lay_out_many`] — which also builds the roster index once for
//! the batch (§14.2). A bulk export that went through the desktop's per-scope read path would
//! compile the problem once per teacher (§2.5), and that is the shape this avoids.
//!
//! # A document cannot contain another school's data
//!
//! Structurally, and not by a check here. A `Problem` is compiled for one
//! `SchedulingSelection { school_id, cycle_id, term_id }` and a `LabelSet` is scoped by the
//! same `school_id`, so the projection this module reads has seen nothing else — there is no
//! code path by which another school's rows could reach it (§9.1). What *is* done here is the
//! disclosure: a teacher employed at two schools has, in this document, only the lessons of
//! the school solved, and the sheet says so.

use std::collections::{BTreeMap, BTreeSet};

use evara_core::scheduling::display::LabelSet;
use evara_core::scheduling::{Problem, UnplacedCause};
use uuid::Uuid;

use crate::findings::FindingView;
use crate::projection::{
    CellContent, GridPanel, ProjectedDay, ProjectedLesson, ProjectedPeriod, ProjectionScope,
    ScopeGrid, ScopeKind, TimetableProjection, lay_out_many,
};

use super::{
    CellLesson, ConflictLine, DefectLine, Disclosure, DisclosureCode, DisclosureEntity,
    DocumentFinding, DocumentHeader, DocumentLogo, OutputCell, OutputCellKind, OutputDocument,
    OutputError, OutputOptions, OutputPanel, OutputSheet, RunProvenance, UnscheduledLine,
};

/// What only the caller knows about the document.
///
/// The school, the cycle and the term come from the projection, so they are not here: a caller
/// holding them apart from the grid could name one school's timetable with another's heading.
/// These seven are the ones a projection cannot answer — and `generated_at` in particular,
/// because this crate reads no clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentIdentity {
    /// What the timetable is called.
    pub timetable_name: String,
    /// Which timetable.
    pub timetable_id: Uuid,
    /// Its revision, read in the same transaction as the grid.
    pub timetable_rev: i64,
    /// How it came to be, where a run is recorded.
    pub provenance: Option<RunProvenance>,
    /// ISO-8601 UTC, supplied by the caller.
    pub generated_at: String,
    /// Which build composed it.
    pub app_version: String,
    /// The school's logo, where it has one.
    pub logo: Option<DocumentLogo>,
}

/// One occurrence the caller's run could not place.
///
/// Carries the cause as the **stored key** rather than a decided boolean, so that
/// [`UnplacedCause::is_impossible`] stays the one place the difference between *"no timetable
/// can place this"* and *"this arrangement did not"* is decided. A caller passing a flag could
/// pass the wrong one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnscheduledOccurrence {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// What the lesson is called, where the caller knew.
    pub label: Option<String>,
    /// `UnplacedCause::key()`, as stored or as the run reported it.
    pub cause: String,
    /// The pre-solve finding that already proved it, where there was one.
    pub diagnostic_code: Option<String>,
    /// What the engine said, for a result still in memory.
    pub reason: Option<String>,
}

/// Everything [`compose`] is given.
///
/// Grouped rather than passed as six arguments for the reason `ProjectionInput` is: the parts
/// are one answer about one timetable, and a caller holding them apart could compose one
/// timetable's grid with another's findings.
#[derive(Debug, Clone, Copy)]
pub struct CompositionInput<'a> {
    /// The compiled problem, for the cross-campus disclosure.
    ///
    /// The only thing read from it is `also_at` on the teachers and rooms: the fact that
    /// another school holds the same identity lives nowhere else, and the alternative — the
    /// caller flattening it first — would put the fact one indirection away from the sheet it
    /// has to appear on.
    pub problem: &'a Problem,
    /// The projection: axes, days, lessons and the scopes a reader can select.
    pub projection: &'a TimetableProjection,
    /// The display names, for the roster index and the scope labels.
    pub labels: &'a LabelSet,
    /// What only the caller knows.
    pub identity: &'a DocumentIdentity,
    /// What the run could not place. Empty where nothing is recorded.
    pub unscheduled: &'a [UnscheduledOccurrence],
    /// The findings, at whichever fidelity the caller had.
    pub findings: &'a [FindingView],
}

/// Composes one document from one projection.
///
/// # Errors
///
/// [`OutputError::UnknownScope`], [`OutputError::UnknownSchedule`] or
/// [`OutputError::UnknownDay`] where the options name something the projection does not offer;
/// [`OutputError::NoEnrolments`] where a per-student document is asked for and no student
/// carries an enrolment row; [`OutputError::EmptySelection`] where the filters leave nothing to
/// render at all.
pub fn compose(
    input: CompositionInput<'_>,
    options: &OutputOptions,
) -> Result<OutputDocument, OutputError> {
    let CompositionInput {
        problem,
        projection,
        labels,
        identity,
        unscheduled,
        findings,
    } = input;

    let scopes = select_scopes(projection, options)?;
    let days = select_days(projection, options)?;
    let schedules = select_schedules(projection, options)?;

    let grids = lay_out_many(projection, &scopes, labels);

    let shared = shared_identities(problem);
    let day_labels_by_id: BTreeMap<Uuid, String> = projection
        .days
        .iter()
        .map(|day| (day.cycle_day_id, day.label.clone()))
        .collect();

    let mut sheets = Vec::with_capacity(grids.len());
    let mut panels_omitted = 0_usize;
    let mut rows_omitted = 0_usize;
    for grid in &grids {
        let (sheet, dropped_panels, dropped_rows) = compose_sheet(
            projection,
            grid,
            &days,
            schedules.as_ref(),
            options,
            &shared,
        );
        panels_omitted += dropped_panels;
        rows_omitted += dropped_rows;
        sheets.push(sheet);
    }

    if sheets.iter().all(|sheet| sheet.panels.is_empty())
        && sheets.iter().any(|sheet| !sheet.is_empty)
    {
        // Every sheet that holds something had everything filtered away. An empty file would
        // read as "there was nothing" rather than "you filtered everything out".
        return Err(OutputError::EmptySelection {
            detail: "every panel of every selected entity was filtered out by the bell \
                     schedule or day selection"
                .to_owned(),
        });
    }

    let unscheduled_lines: Vec<UnscheduledLine> = if options.include_unscheduled {
        unscheduled.iter().map(unscheduled_line).collect()
    } else {
        Vec::new()
    };
    let finding_lines: Vec<DocumentFinding> = if options.include_findings {
        findings
            .iter()
            .map(|finding| document_finding(finding, &day_labels_by_id))
            .collect()
    } else {
        Vec::new()
    };

    let disclosures = document_disclosures(
        identity,
        options,
        &sheets,
        unscheduled.len(),
        findings.len(),
        projection.days.len() - days.len(),
        panels_omitted,
        rows_omitted,
    );

    Ok(OutputDocument {
        header: DocumentHeader {
            school_name: projection.school_name.clone(),
            cycle_name: projection.cycle_name.clone(),
            term_name: projection.term_name.clone(),
            timetable_name: identity.timetable_name.clone(),
            timetable_id: identity.timetable_id,
            timetable_rev: identity.timetable_rev,
            provenance: identity.provenance.clone(),
            generated_at: identity.generated_at.clone(),
            app_version: identity.app_version.clone(),
            logo: if options.include_logo {
                identity.logo.clone()
            } else {
                None
            },
        },
        sheets,
        unscheduled: unscheduled_lines,
        findings: finding_lines,
        disclosures,
        options: options.clone(),
    })
}

// ---------------------------------------------------------------------- the selection

/// Which entities the document is about, in the caller's order.
///
/// An empty `scope_ids` means *every* entity of the kind the projection offers — including
/// the ones with no lessons, because a school exporting every teacher needs to see which
/// teachers have nothing (§8.4).
fn select_scopes(
    projection: &TimetableProjection,
    options: &OutputOptions,
) -> Result<Vec<ProjectionScope>, OutputError> {
    // A per-student document is refused before a picker is offered, not answered blank. ADR
    // 0006 makes the roster optional, so "no rows" is an ordinary state — and the honest
    // answer to it is a sentence, because the only way to fill the gap would be to infer a
    // child's schedule from a group they might be in.
    if options.scope_kind == ScopeKind::Student && !projection.has_enrolments {
        return Err(OutputError::NoEnrolments);
    }

    let offered: Vec<&ProjectionScope> = projection
        .scopes
        .iter()
        .filter(|scope| scope.kind == options.scope_kind)
        .collect();

    if options.scope_ids.is_empty() {
        if offered.is_empty() {
            return Err(OutputError::EmptySelection {
                detail: format!(
                    "this timetable offers no {} to render",
                    kind_word(options.scope_kind)
                ),
            });
        }
        return Ok(offered.into_iter().cloned().collect());
    }

    // Named entities, in the caller's order, duplicates collapsed to their first position: two
    // identical sheets would be two identical worksheets, which Excel refuses by name anyway.
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let mut chosen = Vec::with_capacity(options.scope_ids.len());
    for id in &options.scope_ids {
        if !seen.insert(*id) {
            continue;
        }
        let found = offered.iter().find(|scope| scope.id == Some(*id)).ok_or(
            OutputError::UnknownScope {
                kind: options.scope_kind,
                id: *id,
            },
        )?;
        chosen.push((*found).clone());
    }
    Ok(chosen)
}

/// Which day columns, in the cycle's own `ordinal` order.
///
/// The caller's order is deliberately **not** preserved: a day's ordinal is its only
/// scheduling identity and the columns are the cycle's days in that order, so a document that
/// reordered them would be asserting something about the cycle that is not true.
fn select_days(
    projection: &TimetableProjection,
    options: &OutputOptions,
) -> Result<Vec<ProjectedDay>, OutputError> {
    if options.day_ordinals.is_empty() {
        if projection.days.is_empty() {
            return Err(OutputError::EmptySelection {
                detail: "this cycle holds no days".to_owned(),
            });
        }
        return Ok(projection.days.clone());
    }

    let mut wanted: BTreeSet<u16> = BTreeSet::new();
    for ordinal in &options.day_ordinals {
        if !projection.days.iter().any(|day| day.ordinal == *ordinal) {
            return Err(OutputError::UnknownDay { ordinal: *ordinal });
        }
        wanted.insert(*ordinal);
    }
    let days: Vec<ProjectedDay> = projection
        .days
        .iter()
        .filter(|day| wanted.contains(&day.ordinal))
        .cloned()
        .collect();
    if days.is_empty() {
        return Err(OutputError::EmptySelection {
            detail: "the day selection names no day of this cycle".to_owned(),
        });
    }
    Ok(days)
}

/// Which bell schedules' panels, or `None` for all of them.
fn select_schedules(
    projection: &TimetableProjection,
    options: &OutputOptions,
) -> Result<Option<BTreeSet<Uuid>>, OutputError> {
    if options.schedule_ids.is_empty() {
        return Ok(None);
    }
    let mut wanted = BTreeSet::new();
    for id in &options.schedule_ids {
        if !projection.axes.iter().any(|axis| axis.schedule_id == *id) {
            return Err(OutputError::UnknownSchedule { id: *id });
        }
        wanted.insert(*id);
    }
    Ok(Some(wanted))
}

/// The word a sentence uses for one scope kind.
const fn kind_word(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Section => "class sections",
        ScopeKind::Teacher => "teachers",
        ScopeKind::Room => "rooms",
        ScopeKind::Student => "students",
        ScopeKind::Master => "master grid",
    }
}

// ---------------------------------------------------------------------- one sheet

/// Composes one entity's sheet, and reports what the filters dropped.
///
/// Returns `(sheet, panels dropped, period rows dropped)`.
fn compose_sheet(
    projection: &TimetableProjection,
    grid: &ScopeGrid,
    days: &[ProjectedDay],
    schedules: Option<&BTreeSet<Uuid>>,
    options: &OutputOptions,
    shared: &BTreeMap<Uuid, Vec<String>>,
) -> (OutputSheet, usize, usize) {
    let day_ordinals: BTreeSet<u16> = days.iter().map(|day| day.ordinal).collect();

    // Everything this scope holds, before the filters — the denominator of the conservation
    // account below.
    let mut held: BTreeSet<Uuid> = BTreeSet::new();
    for panel in &grid.panels {
        for cell in &panel.cells {
            for ix in occupants(&cell.content) {
                if let Some(lesson) = projection.lessons.get(ix) {
                    held.insert(lesson.activity_id);
                }
            }
        }
    }

    let mut panels = Vec::new();
    let mut panels_dropped = 0_usize;
    let mut rows_dropped = 0_usize;
    let mut present: BTreeSet<Uuid> = BTreeSet::new();

    for panel in &grid.panels {
        if schedules.is_some_and(|wanted| !wanted.contains(&panel.schedule_id)) {
            panels_dropped += 1;
            continue;
        }
        let periods = select_periods(panel, options.include_non_teaching);
        rows_dropped += panel.periods.len() - periods.len();
        let kept_days: Vec<ProjectedDay> = panel
            .days
            .iter()
            .filter(|day| day_ordinals.contains(&day.ordinal))
            .cloned()
            .collect();
        if periods.is_empty() || kept_days.is_empty() {
            panels_dropped += 1;
            continue;
        }

        let cells = compose_cells(projection, panel, &kept_days, &periods, &mut present);
        panels.push(OutputPanel {
            schedule_id: panel.schedule_id,
            schedule_name: panel.schedule_name.clone(),
            campus_name: panel.campus_name.clone(),
            days: kept_days,
            periods,
            cells,
        });
    }

    let kept_schedules: BTreeSet<Uuid> = panels.iter().map(|p| p.schedule_id).collect();

    let sheet = OutputSheet {
        disclosures: sheet_disclosures(&grid.scope, shared),
        scope: grid.scope.clone(),
        is_empty: grid.panels.is_empty(),
        conflicts: conflict_lines(projection, grid, &kept_schedules, &day_ordinals),
        defective: defect_lines(projection, grid, &kept_schedules, &day_ordinals, days),
        lesson_count: present.len(),
        omitted_lesson_count: held.len().saturating_sub(present.len()),
        panels,
    };
    (sheet, panels_dropped, rows_dropped)
}

/// One sheet's clashes, by stored identifier, narrowed to the positions the document holds.
///
/// Narrowed rather than carried whole: a clash on a day the reader deselected is not in this
/// document, and an appendix naming a position the grid does not show would send them looking
/// for a cell that is not there. What the narrowing drops is counted in
/// [`OutputSheet::omitted_lesson_count`] and disclosed, so it is not a silent loss.
fn conflict_lines(
    projection: &TimetableProjection,
    grid: &ScopeGrid,
    kept_schedules: &BTreeSet<Uuid>,
    day_ordinals: &BTreeSet<u16>,
) -> Vec<ConflictLine> {
    grid.conflicts
        .iter()
        .filter(|conflict| {
            kept_schedules.contains(&conflict.schedule_id)
                && day_ordinals.contains(&conflict.day_ordinal)
        })
        .map(|conflict| {
            let involved: Vec<&ProjectedLesson> = conflict
                .lessons
                .iter()
                .filter_map(|ix| projection.lessons.get(*ix))
                .collect();
            ConflictLine {
                schedule_id: conflict.schedule_id,
                day_ordinal: conflict.day_ordinal,
                day_label: conflict.day_label.clone(),
                period_ordinal: conflict.period_ordinal,
                period_label: conflict.period_label.clone(),
                // Every occurrence involved, never the first.
                activity_ids: involved.iter().map(|lesson| lesson.activity_id).collect(),
                captions: involved
                    .iter()
                    .map(|lesson| lesson.lines.caption.clone())
                    .collect(),
            }
        })
        .collect()
}

/// One sheet's unsound runs, narrowed the same way.
fn defect_lines(
    projection: &TimetableProjection,
    grid: &ScopeGrid,
    kept_schedules: &BTreeSet<Uuid>,
    day_ordinals: &BTreeSet<u16>,
    days: &[ProjectedDay],
) -> Vec<DefectLine> {
    grid.defective
        .iter()
        .filter_map(|ix| projection.lessons.get(*ix))
        .filter(|lesson| {
            kept_schedules.contains(&lesson.schedule_id)
                && day_ordinals.contains(&lesson.day_ordinal)
        })
        .filter_map(|lesson| {
            lesson.defect.map(|defect| DefectLine {
                activity_id: lesson.activity_id,
                caption: lesson.lines.caption.clone(),
                // The span resolver's own verdict, carried rather than re-judged.
                defect,
                schedule_id: lesson.schedule_id,
                day_ordinal: lesson.day_ordinal,
                day_label: day_label(days, lesson.day_ordinal),
                occupied_periods: u16::try_from(lesson.occupies.len()).unwrap_or(u16::MAX),
                duration_periods: lesson.duration_periods,
            })
        })
        .collect()
}

/// Which period rows survive the non-teaching filter.
///
/// A break is a row by default, and **always** a row when it holds something. The second half
/// is not politeness: a stored timetable whose bell schedule moved can have an occurrence in a
/// period that is now a break, and a run can be *covered* by a span that starts above one. A
/// filter that dropped such a row would lose a placement and break the merge that crossed it,
/// which is the accidental data loss §4.4 forbids — so only an **entirely empty** non-teaching
/// row is omitted, and that omission is counted and disclosed.
fn select_periods(panel: &GridPanel, include_non_teaching: bool) -> Vec<ProjectedPeriod> {
    if include_non_teaching {
        return panel.periods.clone();
    }
    panel
        .periods
        .iter()
        .filter(|period| {
            period.is_teaching
                || panel.cells.iter().any(|cell| {
                    cell.period_ordinal == period.ordinal
                        && !matches!(cell.content, CellContent::Empty)
                })
        })
        .cloned()
        .collect()
}

/// Every cell of one panel, row-major, with its occupants resolved.
fn compose_cells(
    projection: &TimetableProjection,
    panel: &GridPanel,
    days: &[ProjectedDay],
    periods: &[ProjectedPeriod],
    present: &mut BTreeSet<Uuid>,
) -> Vec<OutputCell> {
    let mut cells = Vec::with_capacity(periods.len() * days.len());
    for period in periods {
        for day in days {
            let Some(source) = panel.cell(day.ordinal, period.ordinal) else {
                continue;
            };
            let (kind, indexes, rows) = match &source.content {
                CellContent::Empty => {
                    let kind = if period.is_teaching {
                        OutputCellKind::Empty
                    } else {
                        OutputCellKind::NonTeaching
                    };
                    (kind, Vec::new(), 1)
                }
                CellContent::Lesson { lesson, rows } => {
                    (OutputCellKind::Lesson, vec![*lesson], *rows)
                }
                CellContent::Pinned { lessons, reason } => (
                    OutputCellKind::Pinned { reason: *reason },
                    lessons.clone(),
                    1,
                ),
                // The covering occurrence, found by reading the panel's own span decision
                // upwards rather than by rebuilding an occupancy map — the span is already
                // decided and a second derivation of it could disagree.
                CellContent::Continuation => (
                    OutputCellKind::Continuation,
                    covering_lesson(panel, day.ordinal, period.ordinal)
                        .into_iter()
                        .collect(),
                    1,
                ),
            };

            let lessons: Vec<CellLesson> = indexes
                .iter()
                .filter_map(|ix| projection.lessons.get(*ix))
                .map(|lesson| {
                    present.insert(lesson.activity_id);
                    cell_lesson(lesson, day.ordinal, period.ordinal)
                })
                .collect();

            cells.push(OutputCell {
                day_ordinal: day.ordinal,
                period_ordinal: period.ordinal,
                kind,
                lessons,
                rows,
            });
        }
    }
    cells
}

/// Which occurrences a cell's content names.
fn occupants(content: &CellContent) -> Vec<usize> {
    match content {
        CellContent::Empty | CellContent::Continuation => Vec::new(),
        CellContent::Lesson { lesson, .. } => vec![*lesson],
        CellContent::Pinned { lessons, .. } => lessons.clone(),
    }
}

/// The occurrence whose span covers one continuation cell.
///
/// Walks the column upwards to the nearest [`CellContent::Lesson`] and checks that its `rows`
/// reach this row. Reading the panel's own decision, not re-deciding it.
fn covering_lesson(panel: &GridPanel, day_ordinal: u16, period_ordinal: u16) -> Option<usize> {
    let row = panel
        .periods
        .iter()
        .position(|period| period.ordinal == period_ordinal)?;
    for above in (0..row).rev() {
        let ordinal = panel.periods.get(above)?.ordinal;
        match panel.cell(day_ordinal, ordinal).map(|cell| &cell.content) {
            Some(CellContent::Continuation) => {}
            Some(CellContent::Lesson { lesson, rows }) => {
                return (above + usize::from(*rows) > row).then_some(*lesson);
            }
            _ => return None,
        }
    }
    None
}

/// One occurrence as this cell carries it.
fn cell_lesson(lesson: &ProjectedLesson, day_ordinal: u16, period_ordinal: u16) -> CellLesson {
    let parts = u16::try_from(lesson.occupies.len()).unwrap_or(u16::MAX);
    let part = lesson
        .occupies
        .iter()
        .position(|slot| slot.day_ordinal == day_ordinal && slot.period_ordinal == period_ordinal)
        .and_then(|at| u16::try_from(at + 1).ok())
        .unwrap_or(1);
    CellLesson {
        activity_id: lesson.activity_id,
        section_id: lesson.section_id,
        lines: lesson.lines.clone(),
        label: lesson.label.clone(),
        teacher_ids: lesson.teacher_ids.clone(),
        group_ids: lesson.group_ids.clone(),
        room_id: lesson.room_id,
        room: lesson.room.clone(),
        starts_at: lesson.starts_at.clone(),
        ends_at: lesson.ends_at.clone(),
        duration_periods: lesson.duration_periods,
        part,
        parts,
        flags: lesson.flags,
        defect: lesson.defect,
    }
}

/// One day's name, or its ordinal where the selection does not hold it.
fn day_label(days: &[ProjectedDay], ordinal: u16) -> String {
    days.iter()
        .find(|day| day.ordinal == ordinal)
        .map_or_else(|| format!("Day {ordinal}"), |day| day.label.clone())
}

// ---------------------------------------------------------------------- appendices

/// One unplaced occurrence, with impossibility decided where it is decided.
fn unscheduled_line(occurrence: &UnscheduledOccurrence) -> UnscheduledLine {
    let cause = UnplacedCause::from_key(&occurrence.cause);
    UnscheduledLine {
        activity_id: occurrence.activity_id,
        caption: occurrence.label.clone(),
        cause: occurrence.cause.clone(),
        known_cause: cause.is_some(),
        // Never true for a cause this build does not recognise. Overclaiming impossibility is
        // the failure the distinction exists to prevent.
        is_impossible: cause.is_some_and(UnplacedCause::is_impossible),
        diagnostic_code: occurrence.diagnostic_code.clone(),
        reason: occurrence.reason.clone(),
    }
}

/// One finding, in the rule's own words.
fn document_finding(finding: &FindingView, day_labels: &BTreeMap<Uuid, String>) -> DocumentFinding {
    DocumentFinding {
        type_key: finding.type_key.clone(),
        known_type: finding.known_type,
        level: finding.level.clone(),
        penalty: finding.penalty,
        // Verbatim. ADR 0043: the sentence belongs to the rule that produced it.
        message: finding.message.clone(),
        detail: finding.detail.clone(),
        day_label: finding
            .cycle_day_id
            .and_then(|id| day_labels.get(&id).cloned()),
        starts_at: finding.starts_at.clone(),
        ends_at: finding.ends_at.clone(),
        activity_ids: finding.activity_ids.clone(),
    }
}

// ---------------------------------------------------------------------- disclosures

/// Every teacher and room in the problem that another school also holds, by identifier.
///
/// Read straight off `TeacherFact::also_at` and `RoomFact::also_at`, which is where the fact
/// lives. ADR 0032 rule 3 requires the warning to name the entity, both schools and the
/// consequence, which is what [`sheet_disclosures`] composes from this.
fn shared_identities(problem: &Problem) -> BTreeMap<Uuid, Vec<String>> {
    let mut shared = BTreeMap::new();
    for teacher in &problem.teachers {
        if !teacher.also_at.is_empty() {
            shared.insert(
                teacher.id,
                teacher.also_at.iter().map(|s| s.name.clone()).collect(),
            );
        }
    }
    for room in &problem.rooms {
        if !room.also_at.is_empty() {
            shared.insert(
                room.id,
                room.also_at.iter().map(|s| s.name.clone()).collect(),
            );
        }
    }
    shared
}

/// The sentences one sheet must carry.
///
/// Two of them, and neither is suppressible by any option:
///
/// - the **cross-campus** sentence, when the selected entity is also held by another school. A
///   sheet headed with a teacher's name reads as *their timetable*, and for somebody employed
///   at two schools it is their timetable **here**. §9.3, and risk R-10: this is the one
///   disclosure whose absence causes a real scheduling mistake in a real school.
/// - the **roster-derived** caveat on a student's sheet, because the grid is `class_enrolment`
///   and nothing else, and is therefore potentially incomplete (ADR 0006).
fn sheet_disclosures(
    scope: &ProjectionScope,
    shared: &BTreeMap<Uuid, Vec<String>>,
) -> Vec<Disclosure> {
    let mut out = Vec::new();

    if let Some(id) = scope.id
        && let Some(others) = shared.get(&id)
    {
        let kind = match scope.kind {
            ScopeKind::Room => "room",
            _ => "teacher",
        };
        out.push(Disclosure {
            code: DisclosureCode::SharedEntity,
            message: format!(
                "{} is also held by {}. This timetable is for {} only: commitments at the \
                 other school are not shown here and have not been checked against these \
                 placements.",
                scope.name,
                others.join(" and "),
                kind_possessive(scope.kind),
            ),
            is_mandatory: true,
            entity: Some(DisclosureEntity {
                kind: kind.to_owned(),
                id,
                name: scope.name.clone(),
            }),
            count: Some(others.len()),
        });
    }

    if scope.roster_derived {
        out.push(Disclosure {
            code: DisclosureCode::RosterDerived,
            message: format!(
                "{}'s timetable is derived from their enrolment records alone. A lesson they \
                 attend as part of a group, with no enrolment record, is not shown.",
                scope.name
            ),
            is_mandatory: true,
            entity: scope.id.map(|id| DisclosureEntity {
                kind: "student".to_owned(),
                id,
                name: scope.name.clone(),
            }),
            count: None,
        });
    }

    out
}

/// How a sentence refers to the school this document is for.
const fn kind_possessive(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Room => "this school's bookings",
        _ => "this school",
    }
}

/// The document-wide disclosures, mandatory ones first.
#[allow(
    clippy::too_many_arguments,
    reason = "one list of what the reader must be told, assembled from the counts that \
              produce it; threading them through a struct would hide which filter produced \
              which sentence"
)]
fn document_disclosures(
    identity: &DocumentIdentity,
    options: &OutputOptions,
    sheets: &[OutputSheet],
    unscheduled: usize,
    findings: usize,
    days_omitted: usize,
    panels_omitted: usize,
    rows_omitted: usize,
) -> Vec<Disclosure> {
    let mut out = vec![Disclosure {
        code: DisclosureCode::Snapshot,
        message: format!(
            "This document is a snapshot of \"{}\" at revision {}, taken on {}. It is not \
             updated when the timetable changes.",
            identity.timetable_name, identity.timetable_rev, identity.generated_at
        ),
        is_mandatory: true,
        entity: None,
        count: None,
    }];

    // Every sheet's mandatory sentences reach the cover as well, deduplicated: §9.3 asks for
    // the cross-campus sentence on the cover "when any selected entity is shared", and a
    // reader who opens the cover and nothing else must still see it.
    let mut seen: BTreeSet<(DisclosureCode, Option<Uuid>)> = BTreeSet::new();
    for sheet in sheets {
        for disclosure in &sheet.disclosures {
            let key = (disclosure.code, disclosure.entity.as_ref().map(|e| e.id));
            if seen.insert(key) {
                out.push(disclosure.clone());
            }
        }
    }

    let omitted_lessons: usize = sheets.iter().map(|s| s.omitted_lesson_count).sum();
    if omitted_lessons > 0 {
        out.push(Disclosure {
            code: DisclosureCode::LessonsOmitted,
            message: format!(
                "{omitted_lessons} placed lesson(s) fall outside the days or bell schedules \
                 selected and are not shown."
            ),
            is_mandatory: true,
            entity: None,
            count: Some(omitted_lessons),
        });
    }
    if days_omitted > 0 {
        out.push(Disclosure {
            code: DisclosureCode::DaysOmitted,
            message: format!("{days_omitted} day(s) of the cycle were not selected."),
            is_mandatory: false,
            entity: None,
            count: Some(days_omitted),
        });
    }
    if panels_omitted > 0 {
        out.push(Disclosure {
            code: DisclosureCode::PanelsOmitted,
            message: format!("{panels_omitted} bell schedule panel(s) were not selected."),
            is_mandatory: false,
            entity: None,
            count: Some(panels_omitted),
        });
    }
    if rows_omitted > 0 {
        out.push(Disclosure {
            code: DisclosureCode::NonTeachingOmitted,
            message: format!(
                "{rows_omitted} empty non-teaching period row(s) were left out. A break that \
                 holds a lesson is always shown."
            ),
            is_mandatory: false,
            entity: None,
            count: Some(rows_omitted),
        });
    }
    if !options.include_unscheduled && unscheduled > 0 {
        out.push(Disclosure {
            code: DisclosureCode::UnscheduledOmitted,
            message: format!(
                "{unscheduled} occurrence(s) have no place in this timetable and were left \
                 out at your request."
            ),
            is_mandatory: true,
            entity: None,
            count: Some(unscheduled),
        });
    }
    if !options.include_findings && findings > 0 {
        out.push(Disclosure {
            code: DisclosureCode::FindingsOmitted,
            message: format!(
                "{findings} finding(s) about this timetable were left out at your request."
            ),
            is_mandatory: true,
            entity: None,
            count: Some(findings),
        });
    }

    out
}
