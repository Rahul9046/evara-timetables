//! Turning a problem and an arrangement into grids a reader can scan.
//!
//! # One projection, two callers
//!
//! The generated preview and the saved-timetable view go through **this** code. That is what
//! makes *"what I previewed is what I saved"* a structural property rather than a hope: there
//! is no second renderer that could disagree with the first.
//!
//! # What this module does not do
//!
//! It resolves no span and judges no rule of its own.
//! [`span::resolve_activity_span`](evara_core::scheduling::span::resolve_activity_span) is
//! the one implementation of *"which slots does this occurrence occupy?"*
//! ([ADR 0042](../../../../docs/adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
//! D2), and this module calls it once per placed occurrence. A second copy of that measure is
//! `CLAUDE.md` §2's most important rule broken.
//!
//! # Nothing here can read a weekday
//!
//! Not as a discipline — as a fact about the inputs. `CycleDayFact` carries `id`, `ordinal`
//! and `label` and no weekday at all, so the columns are the cycle's days in `ordinal` order
//! and there is nothing else they could be. `CLAUDE.md` §3, and ADR 0017.
//!
//! # One axis per bell schedule, never a merged one
//!
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md):
//! an occurrence draws its slots from exactly one resolved bell schedule. Two campuses on two
//! schedules hold different `timeslot` rows over the same minutes
//! ([ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! rule 2), so their period rows do not align and a merged axis would have to assert that
//! Northgate's Period 3 and Southmoor's Period 3 are one row. They are not. A scope touching
//! two schedules gets [`ScopeGrid::panels`] of length two.
//!
//! # A cell holds a list, and `rowSpan` is decided here
//!
//! The hard requirement on this module is that **the cell grid is always a well-formed
//! table**, including for a timetable saved last term whose grid has moved underneath it.
//! Two things make that fail if a renderer is left to work them out:
//!
//! - **Two occurrences in one cell.** Ordinary in a master grid, a clash in a teacher's.
//!   Either way, two overlapping `rowSpan`s in one column produce a broken table and a lost
//!   lesson.
//! - **A run that is not contiguous in the axis.** If a slot of the run was never
//!   materialised, the occupied periods can be 2 and 4, and no single `rowSpan` describes
//!   that.
//!
//! So every `(day, period)` of every panel gets exactly one [`TimetableCell`], a lesson gets a
//! `rowSpan` **only** when it is alone in every cell it occupies and contiguous in the axis,
//! and everything else is [`CellContent::Pinned`] at `rowSpan` 1 — carrying every lesson
//! involved, so none is discarded. [`GridPanel::is_well_formed`] asserts the invariant and a
//! test runs it over every fixture.

use std::collections::{BTreeMap, BTreeSet};

use evara_core::scheduling::display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
use evara_core::scheduling::span::{SpanDefect, resolve_activity_span};
use evara_core::scheduling::{ActivityIx, Assignment, PeriodKind, Problem};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One period row of a bell schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectedPeriod {
    /// The stored period.
    pub period_id: Uuid,
    /// Its position in the schedule. The row's only identity.
    pub ordinal: u16,
    /// What it is called.
    pub label: String,
    /// `'HH:MM'`, as stored.
    pub starts_at: String,
    /// `'HH:MM'`, as stored.
    pub ends_at: String,
    /// Whether teaching may happen in it.
    ///
    /// A break is rendered as a row and marked unavailable, so that *"nothing is scheduled
    /// here"* and *"nothing may be scheduled here"* do not collapse into one appearance.
    pub is_teaching: bool,
}

/// One day column of the cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectedDay {
    /// The stored cycle day.
    pub cycle_day_id: Uuid,
    /// Its position in the cycle. A day's **only** scheduling identity.
    pub ordinal: u16,
    /// What it is called — display only.
    pub label: String,
}

/// One slot an occurrence occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectedCellRef {
    /// The materialised slot.
    pub timeslot_id: Uuid,
    /// The day it falls on.
    pub day_ordinal: u16,
    /// The period row it falls in.
    pub period_ordinal: u16,
}

/// Whether a stored entry was locked or placed by hand.
///
/// Both are columns the generator writes as `0`
/// ([TIMETABLE-PERSISTENCE.md](../../../../docs/TIMETABLE-PERSISTENCE.md) §12). M8 renders
/// them and writes neither, so that when Phase 8 begins writing them the view already tells
/// the truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LessonFlags {
    /// The entry is pinned and a later repair must not move it.
    pub is_locked: bool,
    /// The entry was placed by hand rather than by a solve.
    pub is_manual: bool,
}

/// How much of a cell's caption a reader wants.
///
/// Read by [`CellLines::one_line`] and carried on
/// [`OutputOptions::detail`](crate::output::OutputOptions::detail). Three values rather than a
/// free-form template: a report designer is out of scope
/// (`M11-TIMETABLE-OUTPUT-PROPOSAL.md` §3.2), and the three are what the Download dialogue's
/// *"Cell shows"* control offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CellDetail {
    /// The caption alone — what fits in a narrow printed column.
    Compact,
    /// The caption, the teachers and the room. The screen's own cell, and the default.
    #[default]
    Standard,
    /// Everything the lines hold, including the groups, the occurrence's name and the time.
    Full,
}

impl CellDetail {
    /// The stable key, for a command argument and a test that pins the vocabulary.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Full => "full",
        }
    }

    /// Every value, in increasing order of detail.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Compact, Self::Standard, Self::Full]
    }

    /// The detail one key names, or `None` where it names nothing.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|d| d.key() == key)
    }
}

/// What a cell says about one occurrence, composed once.
///
/// # Why this exists, and what it replaces
///
/// Four consumers have to agree about what a lesson is called: the on-screen grid, the CSV,
/// the workbook and the printed page. Before M11 the screen composed its own caption, teacher
/// list and room name in TypeScript (`TimetableGrid.tsx`), which was three lines of joining
/// and so not a scheduling rule in the view layer — but it was also the only composition, and
/// *"the screen says one thing and the printout says another"* is the drift this milestone
/// must not introduce. So the composition moves to Rust, where it is done once and read by all
/// four. `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §5.2 and §5.6.
///
/// # Nothing here is substituted
///
/// [`CellLines::caption`] is [`SectionLabel::short`] — composed by
/// [`SectionLabel::compose_short`], which is already the one implementation — or the section's
/// stored identifier where the label set does not hold a label. Never a placeholder name that
/// a reader could mistake for data: `evara_core::scheduling::display`'s header is the rule, and
/// an identifier is visibly an identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CellLines {
    /// [`SectionLabel::short`], or the section identifier where the set holds no label.
    pub caption: String,
    /// The occurrence's own name, where one was given.
    pub activity_name: Option<String>,
    /// Every teacher, in the section's order. Never only the first.
    pub teachers: Vec<String>,
    /// Every attending student group, in the section's order.
    pub groups: Vec<String>,
    /// The room's name, where a room was booked.
    ///
    /// The identifier where a room was booked and the label set holds no label for it: a
    /// booked room rendering as blank would be a placement silently omitted, which §4.4 of the
    /// proposal forbids. `None` means **no room was booked**, which is a different fact.
    pub room: Option<String>,
    /// `'HH:MM–HH:MM'`, derived from the periods the occurrence covers.
    ///
    /// Copied from [`ProjectedLesson::starts_at`] and [`ProjectedLesson::ends_at`], which the
    /// span resolver produced from the slots. Never a duration multiplied by a period length —
    /// `CLAUDE.md` §3.
    pub time: String,
}

impl CellLines {
    /// The whole caption on one line, at the detail asked for.
    ///
    /// What a CSV field and a narrow workbook column use; the structured fields are what the
    /// printed page and a wide cell use. One implementation, four consumers.
    #[must_use]
    pub fn one_line(&self, detail: CellDetail) -> String {
        let teachers = self.teachers.join(", ");
        let groups = self.groups.join(", ");
        let mut parts: Vec<&str> = vec![self.caption.as_str()];
        if detail == CellDetail::Full
            && let Some(name) = self.activity_name.as_deref()
        {
            parts.push(name);
        }
        if detail != CellDetail::Compact {
            parts.push(teachers.as_str());
        }
        if detail == CellDetail::Full {
            parts.push(groups.as_str());
        }
        if detail != CellDetail::Compact
            && let Some(room) = self.room.as_deref()
        {
            parts.push(room);
        }
        if detail == CellDetail::Full {
            parts.push(self.time.as_str());
        }
        parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }
}

/// One placed occurrence, resolved and labelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectedLesson {
    /// The stored occurrence.
    pub activity_id: Uuid,
    /// The section it belongs to.
    pub section_id: Uuid,
    /// What it is called, where the label set holds it.
    ///
    /// `None` is an ordinary state for a label set loaded for a different scope, and the view
    /// says so rather than substituting a name.
    pub label: Option<SectionLabel>,
    /// The occurrence's own name, where one was given.
    pub activity_name: Option<String>,
    /// Who teaches it.
    pub teacher_ids: Vec<Uuid>,
    /// Their names, in the same order.
    pub teachers: Vec<String>,
    /// The student groups that attend.
    pub group_ids: Vec<Uuid>,
    /// Their names, in the same order.
    pub groups: Vec<String>,
    /// The room booked, where one was.
    pub room_id: Option<Uuid>,
    /// Its label, where the label set holds it.
    pub room: Option<RoomLabel>,
    /// The bell schedule the run was taken from — the **slot's**, not the section's.
    pub schedule_id: Uuid,
    /// The cycle day it runs on.
    pub cycle_day_id: Uuid,
    /// That day's ordinal.
    pub day_ordinal: u16,
    /// Every slot it occupies, first to last.
    pub occupies: Vec<ProjectedCellRef>,
    /// `'HH:MM'` of the first slot it occupies.
    ///
    /// Derived from the periods covered, never from a duration multiplied by a period
    /// length — `CLAUDE.md` §3.
    pub starts_at: String,
    /// `'HH:MM'` of the last slot it occupies.
    pub ends_at: String,
    /// How many periods the occurrence asked for.
    pub duration_periods: u16,
    /// What is wrong with the run, where anything is.
    ///
    /// Carried and rendered, never swallowed: a stored timetable whose grid has moved
    /// produces short, broken or misplaced runs, and the whole reason the resolver is
    /// permissive is so that a reader can still see what to fix.
    pub defect: Option<SpanDefect>,
    /// Whether the entry is locked or manual.
    pub flags: LessonFlags,
    /// What a cell says about it, composed once here and read by every consumer.
    ///
    /// Additive, and the fields it joins are all still carried beside it — a CSV column
    /// contract wants `subject` and `course` unjoined and a grid wants them joined, so both
    /// forms are present and only one of them is *composed*. §5.6 of the M11 proposal.
    pub lines: CellLines,
}

/// Which kind of thing a grid is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ScopeKind {
    /// One class section's own timetable.
    Section,
    /// One teacher's load.
    Teacher,
    /// One room's bookings.
    Room,
    /// One student's roster-derived timetable.
    Student,
    /// Every occurrence, ungrouped.
    Master,
}

impl ScopeKind {
    /// Whether two occurrences in one cell are a clash for this kind of grid.
    ///
    /// A teacher, a room, a section and a student can each be in one place at a time, so a
    /// second occupant is a conflict. The **master** grid is every occurrence at once, where
    /// several in one cell is the ordinary state of a working timetable and calling it a
    /// conflict would make the word useless.
    #[must_use]
    pub const fn expects_exclusive_use(self) -> bool {
        !matches!(self, Self::Master)
    }
}

/// One selectable grid: which teacher, room, section or student.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectionScope {
    /// What kind of grid.
    pub kind: ScopeKind,
    /// Which one. `None` for [`ScopeKind::Master`], which is about nothing in particular.
    pub id: Option<Uuid>,
    /// What to call it.
    pub name: String,
    /// A second line — a building, a campus, a subject — where there is one.
    pub secondary: Option<String>,
    /// How many placed occurrences fall in it.
    pub lesson_count: usize,
    /// Whether this scope's contents come from an explicit roster rather than from
    /// scheduling data.
    ///
    /// True only for [`ScopeKind::Student`]. A student grid is derived from
    /// `class_enrolment` rows and from nothing else, so it is **potentially incomplete** and
    /// is labelled as such wherever it is shown — ADR 0006, and
    /// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §7.
    pub roster_derived: bool,
}

/// Why a cell could not be given a `rowSpan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PinReason {
    /// Two or more occurrences occupy it and this scope expects exclusive use.
    ///
    /// A clash to be shown as one. Every occurrence involved is in
    /// [`CellContent::Pinned::lessons`], so none is lost.
    Conflict,
    /// Two or more occurrences occupy it and that is ordinary for this scope.
    ///
    /// The master grid. Not a finding.
    Shared,
    /// The occurrence's run is not contiguous in this axis, so no single `rowSpan`
    /// describes it.
    ///
    /// Reachable when a slot inside the run was never materialised. The occurrence is shown
    /// in each cell it occupies, with its part number.
    BrokenSpan,
}

/// What is in one cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CellContent {
    /// Nothing is placed here.
    Empty,
    /// One occurrence begins here and covers `rows` period rows.
    ///
    /// The only variant carrying a `rowSpan`, and it is produced only when the occurrence is
    /// alone in every cell it occupies and contiguous in this axis.
    Lesson {
        /// Index into [`TimetableProjection::lessons`].
        lesson: usize,
        /// The `rowSpan` to render. Always at least 1.
        rows: u16,
    },
    /// Covered by a [`CellContent::Lesson`] above in the same column. Render nothing.
    Continuation,
    /// One or more occurrences held to this single cell. `rowSpan` is always 1.
    Pinned {
        /// Indexes into [`TimetableProjection::lessons`], in a deterministic order.
        lessons: Vec<usize>,
        /// Why no `rowSpan` was given.
        reason: PinReason,
    },
}

/// One cell of one panel — one `(day, period)` position of one bell schedule's grid.
///
/// Named `TimetableCell` rather than `GridCell` because `evara-db`'s materialisation preview
/// already owns that name for a *slot* of the grid being built. These are different things:
/// that one is about whether a `timeslot` row exists, this one is about what occupies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TimetableCell {
    /// Which day column.
    pub day_ordinal: u16,
    /// Which period row.
    pub period_ordinal: u16,
    /// What is in it.
    pub content: CellContent,
}

/// One bell schedule's grid: days across, periods down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GridPanel {
    /// The schedule this panel is for.
    pub schedule_id: Uuid,
    /// What it is called.
    pub schedule_name: String,
    /// The campus it serves, where it names one.
    pub campus_name: Option<String>,
    /// The day columns, in `ordinal` order.
    pub days: Vec<ProjectedDay>,
    /// The period rows, in `ordinal` order.
    pub periods: Vec<ProjectedPeriod>,
    /// Every cell, in `(period, day)` order — row-major, as a table is read.
    pub cells: Vec<TimetableCell>,
}

impl GridPanel {
    /// The cell at one position, or `None` if the panel has no such position.
    #[must_use]
    pub fn cell(&self, day_ordinal: u16, period_ordinal: u16) -> Option<&TimetableCell> {
        self.cells
            .iter()
            .find(|c| c.day_ordinal == day_ordinal && c.period_ordinal == period_ordinal)
    }

    /// Whether this panel renders as a well-formed table.
    ///
    /// # The invariant, and why it is checked rather than assumed
    ///
    /// Four things have to hold, and a renderer that worked out its own `rowSpan`s would
    /// break one of them the first time a stored timetable's grid moved:
    ///
    /// 1. there is exactly one cell per `(day, period)`, and no others;
    /// 2. every `rowSpan` fits inside the panel;
    /// 3. the cells a `rowSpan` covers are all [`CellContent::Continuation`];
    /// 4. no [`CellContent::Continuation`] is uncovered.
    ///
    /// Together those say: summing one row's rendered cells gives the day count, for every
    /// row — which is the definition of a table that is not broken.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.malformations().is_empty()
    }

    /// Every way this panel would render as a broken table. Empty for a sound one.
    ///
    /// Separate from [`GridPanel::is_well_formed`] so a failing test says *which* rule broke
    /// rather than only that one did.
    #[must_use]
    pub fn malformations(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let days: Vec<u16> = self.days.iter().map(|d| d.ordinal).collect();
        let periods: Vec<u16> = self.periods.iter().map(|p| p.ordinal).collect();

        // 1. Exactly one cell per position.
        if self.cells.len() != days.len() * periods.len() {
            problems.push(format!(
                "{} cells for a {}x{} grid",
                self.cells.len(),
                periods.len(),
                days.len()
            ));
        }
        let mut seen: BTreeSet<(u16, u16)> = BTreeSet::new();
        for cell in &self.cells {
            if !seen.insert((cell.period_ordinal, cell.day_ordinal)) {
                problems.push(format!(
                    "two cells for day {} period {}",
                    cell.day_ordinal, cell.period_ordinal
                ));
            }
            if !days.contains(&cell.day_ordinal) || !periods.contains(&cell.period_ordinal) {
                problems.push(format!(
                    "a cell at day {} period {} is outside the axes",
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

        // 2 and 3. Spans fit, and what they cover is a continuation.
        let mut covered: BTreeSet<(u16, u16)> = BTreeSet::new();
        for cell in &self.cells {
            let CellContent::Lesson { rows, lesson } = cell.content else {
                continue;
            };
            let Some(from) = periods.iter().position(|p| *p == cell.period_ordinal) else {
                continue;
            };
            let rows = usize::from(rows);
            if rows == 0 {
                problems.push(format!("lesson {lesson} spans no rows"));
                continue;
            }
            if from + rows > periods.len() {
                problems.push(format!(
                    "lesson {lesson} spans {rows} rows from row {from}, past the end",
                ));
                continue;
            }
            for period in periods.iter().skip(from + 1).take(rows - 1) {
                let covers = (*period, cell.day_ordinal);
                if !covered.insert(covers) {
                    problems.push(format!(
                        "day {} period {period} is covered twice",
                        cell.day_ordinal
                    ));
                }
                match self.cell(cell.day_ordinal, *period).map(|c| &c.content) {
                    Some(CellContent::Continuation) => {}
                    other => problems.push(format!(
                        "day {} period {period} is covered by lesson {lesson} but holds {other:?}",
                        cell.day_ordinal
                    )),
                }
            }
        }

        // 4. No continuation is uncovered.
        for cell in &self.cells {
            if matches!(cell.content, CellContent::Continuation)
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
}

/// One clash a grid found, for the findings panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GridConflict {
    /// Which schedule's panel.
    pub schedule_id: Uuid,
    /// Which day column.
    pub day_ordinal: u16,
    /// Which day, by name.
    pub day_label: String,
    /// Which period row.
    pub period_ordinal: u16,
    /// Which period, by name.
    pub period_label: String,
    /// Every occurrence involved, as indexes into [`TimetableProjection::lessons`].
    pub lessons: Vec<usize>,
}

/// One scope's grids and what is wrong with them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScopeGrid {
    /// Which grid this is.
    pub scope: ProjectionScope,
    /// One per bell schedule the scope touches, in a deterministic order.
    ///
    /// Two means the scope spans two bell schedules, whose period rows do not align. A
    /// reader is told that the two cannot be read as one week rather than being shown a
    /// merged axis that asserts they can.
    pub panels: Vec<GridPanel>,
    /// Cells where more than one occurrence falls and this scope expects exclusive use.
    pub conflicts: Vec<GridConflict>,
    /// Occurrences in this scope whose run is not sound.
    pub defective: Vec<usize>,
}

impl ScopeGrid {
    /// Whether every panel renders as a well-formed table.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.panels.iter().all(GridPanel::is_well_formed)
    }
}

/// Everything a view needs about one arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TimetableProjection {
    /// The school the arrangement belongs to.
    pub school_name: String,
    /// The cycle its grid is over.
    pub cycle_name: String,
    /// The term, where it names one.
    pub term_name: Option<String>,
    /// The cycle's days, in `ordinal` order. The column axis of every panel.
    pub days: Vec<ProjectedDay>,
    /// Every bell schedule of the school, with its period rows.
    pub axes: Vec<ScheduleAxis>,
    /// Every placed occurrence, resolved and labelled.
    ///
    /// Flat and sent once. Scope grids address these by index, so no occurrence is
    /// serialised twice however many grids it appears in.
    pub lessons: Vec<ProjectedLesson>,
    /// Every grid a reader can select, grouped by kind.
    pub scopes: Vec<ProjectionScope>,
    /// Whether any student carries an enrolment row.
    ///
    /// `false` means the per-student view has nothing to derive from, which a screen says
    /// plainly rather than offering a picker of blank timetables.
    pub has_enrolments: bool,
}

/// One bell schedule's period rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScheduleAxis {
    /// The stored bell schedule.
    pub schedule_id: Uuid,
    /// What it is called.
    pub schedule_name: String,
    /// The campus it serves, where it names one.
    pub campus_id: Option<Uuid>,
    /// That campus's name.
    pub campus_name: Option<String>,
    /// The period rows, in `ordinal` order.
    pub periods: Vec<ProjectedPeriod>,
}

/// What [`project`] is given.
///
/// Grouped rather than passed as four arguments because the four are one answer: a problem,
/// an arrangement over *that* problem, the labels for *that* school, and the stored flags of
/// *those* entries. A caller holding them apart could project one timetable's arrangement
/// against another's labels.
#[derive(Debug, Clone, Copy)]
pub struct ProjectionInput<'a> {
    /// The compiled problem.
    pub problem: &'a Problem,
    /// The arrangement over it.
    pub assignment: &'a Assignment,
    /// The display names.
    pub labels: &'a LabelSet,
    /// Per-occurrence stored flags, keyed by `activity_id`.
    ///
    /// Empty for a generated preview, which has no stored rows yet.
    pub flags: &'a BTreeMap<Uuid, LessonFlags>,
}

/// An empty flag map, for a caller with none.
#[must_use]
pub fn no_flags() -> BTreeMap<Uuid, LessonFlags> {
    BTreeMap::new()
}

/// Projects one arrangement into axes, lessons and selectable scopes.
///
/// Calls [`resolve_activity_span`] once per placed occurrence and resolves nothing itself.
/// An occurrence whose starting slot is not in the problem is skipped: that is an invalid
/// reference, which `Problem::integrity` reports and a grid cannot draw.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one pass over one problem, read top to \
    bottom; splitting it would hide the order the facts have to be read in"
)]
pub fn project(input: ProjectionInput<'_>) -> TimetableProjection {
    let ProjectionInput {
        problem,
        assignment,
        labels,
        flags,
    } = input;
    let context = &problem.context;

    let days: Vec<ProjectedDay> = context
        .cycle_days
        .iter()
        .map(|day| ProjectedDay {
            cycle_day_id: day.id,
            ordinal: day.ordinal,
            label: day.label.clone(),
        })
        .collect();

    let axes: Vec<ScheduleAxis> = context
        .bell_schedules
        .iter()
        .map(|schedule| ScheduleAxis {
            schedule_id: schedule.id,
            schedule_name: schedule.name.clone(),
            campus_id: schedule
                .campus
                .and_then(|ix| context.campuses.get(ix.get()))
                .map(|campus| campus.id),
            campus_name: schedule
                .campus
                .and_then(|ix| context.campuses.get(ix.get()))
                .map(|campus| campus.name.clone()),
            periods: schedule
                .periods
                .iter()
                .filter_map(|ix| context.periods.get(ix.get()))
                .map(|period| ProjectedPeriod {
                    period_id: period.id,
                    ordinal: period.ordinal,
                    label: period.label.clone(),
                    starts_at: period.interval.start.to_string(),
                    ends_at: period.interval.end.to_string(),
                    is_teaching: matches!(period.kind, PeriodKind::Teaching),
                })
                .collect(),
        })
        .collect();

    let sections = labels.section_index();
    let rooms = labels.room_index();

    let mut lessons: Vec<ProjectedLesson> = Vec::new();
    for (ix, fact) in problem.activities.iter().enumerate() {
        let activity = ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
        let Some(placement) = assignment.placement(activity) else {
            continue;
        };
        let Some(resolved) = resolve_activity_span(problem, activity, placement) else {
            // The starting slot is not in the problem. `Problem::integrity` reports that as a
            // loader bug; a grid has no position to draw it at.
            continue;
        };
        let Some(start_slot) = problem.timeslots.get(resolved.span.start().get()) else {
            continue;
        };
        let Some(section) = problem.sections.get(fact.section.get()) else {
            continue;
        };
        let Some(day) = context.cycle_days.get(resolved.span.cycle_day.get()) else {
            continue;
        };
        let Some(schedule) = context.bell_schedules.get(start_slot.bell_schedule.get()) else {
            continue;
        };

        let occupies: Vec<ProjectedCellRef> = resolved
            .span
            .slots
            .iter()
            .filter_map(|slot| problem.timeslots.get(slot.get()))
            .filter_map(|slot| {
                context
                    .periods
                    .get(slot.period.get())
                    .map(|period| ProjectedCellRef {
                        timeslot_id: slot.id,
                        day_ordinal: day.ordinal,
                        period_ordinal: period.ordinal,
                    })
            })
            .collect();
        if occupies.is_empty() {
            continue;
        }

        let teachers: Vec<_> = section
            .teachers
            .iter()
            .filter_map(|ix| problem.teachers.get(ix.get()))
            .collect();
        let groups: Vec<_> = section
            .audience
            .iter()
            .filter_map(|ix| problem.groups.get(ix.get()))
            .collect();
        let room_id = resolved
            .span
            .room
            .and_then(|ix| problem.rooms.get(ix.get()))
            .map(|room| room.id);

        let label = sections.get(&section.id).map(|l| (*l).clone());
        let room = room_id.and_then(|id| rooms.get(&id).map(|r| (*r).clone()));
        let teacher_names: Vec<String> = teachers.iter().map(|t| t.name.clone()).collect();
        let group_names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
        let starts_at = resolved.span.interval.start.to_string();
        let ends_at = resolved.span.interval.end.to_string();
        let lines = CellLines {
            // The label's own short form, or the stored identifier. Never a placeholder.
            caption: label
                .as_ref()
                .map_or_else(|| section.id.to_string(), |l| l.short.clone()),
            activity_name: fact.name.clone(),
            teachers: teacher_names.clone(),
            groups: group_names.clone(),
            // A booked room the label set does not name is named by its identifier rather
            // than left blank: a blank reads as "no room was booked", a different fact.
            room: room.as_ref().map_or_else(
                || room_id.map(|id| id.to_string()),
                |r| Some(r.name.clone()),
            ),
            time: format!("{starts_at}\u{2013}{ends_at}"),
        };

        lessons.push(ProjectedLesson {
            activity_id: fact.id,
            section_id: section.id,
            label,
            activity_name: fact.name.clone(),
            teacher_ids: teachers.iter().map(|t| t.id).collect(),
            teachers: teacher_names,
            group_ids: groups.iter().map(|g| g.id).collect(),
            groups: group_names,
            room_id,
            room,
            schedule_id: schedule.id,
            cycle_day_id: day.id,
            day_ordinal: day.ordinal,
            occupies,
            starts_at,
            ends_at,
            duration_periods: u16::try_from(fact.duration.as_usize()).unwrap_or(u16::MAX),
            defect: Some(resolved.defect).filter(|d| !d.is_sound()),
            flags: flags.get(&fact.id).copied().unwrap_or_default(),
            lines,
        });
    }

    let scopes = build_scopes(problem, labels, &lessons);

    TimetableProjection {
        school_name: context.school.name.clone(),
        cycle_name: context.cycle.name.clone(),
        term_name: context.term.as_ref().map(|t| t.name.clone()),
        days,
        axes,
        lessons,
        scopes,
        has_enrolments: labels.has_enrolments(),
    }
}

/// Every grid a reader can select, in a deterministic order.
///
/// # Counted in one pass, not one pass per entity
///
/// Every `lesson_count` here is a count of the same flat list, so the obvious shape — a
/// `filter().count()` per section, per teacher, per room and per student — is
/// `O(entities × lessons)` on a path that runs on **every** projection. At the documented
/// upper end (500 sections, 80 teachers, 40 rooms) that is millions of comparisons before a
/// single grid is laid out, and the student loop is quadratic in the roster on its own. So the
/// lessons are walked **once** into count maps and each entity reads its own number, and the
/// label lookups go through [`LabelSet::section_index`] / [`LabelSet::room_index`] rather than
/// through the linear `section()` / `room()`. `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §14.2.
///
/// The output is unchanged: the same scopes, in the same order, with the same counts.
fn build_scopes(
    problem: &Problem,
    labels: &LabelSet,
    lessons: &[ProjectedLesson],
) -> Vec<ProjectionScope> {
    // One pass over the lessons, three tallies. `BTreeMap` because every output of this
    // module has to be deterministic and nothing in this workspace iterates a hash map.
    let mut by_section: BTreeMap<Uuid, usize> = BTreeMap::new();
    let mut by_teacher: BTreeMap<Uuid, usize> = BTreeMap::new();
    let mut by_room: BTreeMap<Uuid, usize> = BTreeMap::new();
    for lesson in lessons {
        *by_section.entry(lesson.section_id).or_default() += 1;
        for teacher in &lesson.teacher_ids {
            *by_teacher.entry(*teacher).or_default() += 1;
        }
        if let Some(room) = lesson.room_id {
            *by_room.entry(room).or_default() += 1;
        }
    }

    let sections = labels.section_index();
    let rooms = labels.room_index();

    let mut scopes = vec![ProjectionScope {
        kind: ScopeKind::Master,
        id: None,
        name: "All lessons".to_owned(),
        secondary: None,
        lesson_count: lessons.len(),
        roster_derived: false,
    }];

    // Sections, in the loader's interning order, which is `subject, course, section` —
    // already the order a reader expects.
    for section in &problem.sections {
        let label = sections.get(&section.id).copied();
        scopes.push(ProjectionScope {
            kind: ScopeKind::Section,
            id: Some(section.id),
            name: label.map_or_else(|| section.name.clone(), |l| l.short.clone()),
            secondary: label.map(|l| l.course.clone()),
            lesson_count: by_section.get(&section.id).copied().unwrap_or(0),
            roster_derived: false,
        });
    }

    for teacher in &problem.teachers {
        scopes.push(ProjectionScope {
            kind: ScopeKind::Teacher,
            id: Some(teacher.id),
            name: teacher.name.clone(),
            secondary: None,
            lesson_count: by_teacher.get(&teacher.id).copied().unwrap_or(0),
            roster_derived: false,
        });
    }

    for room in &problem.rooms {
        let label = rooms.get(&room.id).copied();
        scopes.push(ProjectionScope {
            kind: ScopeKind::Room,
            id: Some(room.id),
            name: room.name.clone(),
            secondary: label.and_then(|l| l.building.clone()),
            lesson_count: by_room.get(&room.id).copied().unwrap_or(0),
            roster_derived: false,
        });
    }

    // Students come from the label set, which holds only those with an explicit
    // `class_enrolment` row. A student with no row is absent rather than present with a blank
    // timetable: ADR 0006 makes the roster optional, so its absence is an ordinary state and
    // not a gap to be filled from a group's size.
    //
    // A student's count is the sum of their enrolled sections' counts — the same number the
    // per-lesson scan produced, at `O(their sections)` rather than `O(lessons)`. The sections
    // are deduplicated first, because a roster naming one section twice must not count its
    // lessons twice; `contains` had that property and a sum does not.
    for student in &labels.students {
        let mut enrolled: Vec<Uuid> = student.sections.clone();
        enrolled.sort_unstable();
        enrolled.dedup();
        let count = enrolled
            .iter()
            .filter_map(|section| by_section.get(section))
            .sum();
        scopes.push(ProjectionScope {
            kind: ScopeKind::Student,
            id: Some(student.student_id),
            name: student.name.clone(),
            secondary: student.code.clone(),
            lesson_count: count,
            roster_derived: true,
        });
    }

    scopes
}

/// The roster, indexed, so *"is this student in this lesson?"* is a map lookup.
///
/// # What it is for
///
/// Scope membership is asked once per lesson per grid. For the four scheduling scopes the
/// answer is on the lesson itself — a section identifier, a teacher list, a room — but for a
/// student it is in the **roster**, and answering it by scanning [`LabelSet::students`] makes
/// one student's grid `O(students × lessons)` and every student's `O(students² × lessons)`.
/// At 3 000 students and 2 000 lessons that is about 1.8 × 10¹⁰ comparisons: minutes to hours,
/// not a slow export. `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §14.2.
///
/// Built once per batch by [`lay_out_many`] and once per call by [`lay_out`] — which is the
/// right trade for a single grid and the wrong one for four hundred, which is why both exist.
///
/// # Still one definition of membership
///
/// This type holds **no** rule. [`in_scope_indexed`] is the one implementation of *"does this
/// lesson belong in this scope?"* and [`in_scope`] delegates to it, so a scope's grid and its
/// lesson count cannot disagree however the roster is reached.
#[derive(Debug, Clone)]
pub struct StudentIndex<'a> {
    /// Every student the label set holds, by identifier.
    by_id: BTreeMap<Uuid, &'a StudentLabel>,
}

impl<'a> StudentIndex<'a> {
    /// Indexes one label set's roster.
    #[must_use]
    pub fn of(labels: &'a LabelSet) -> Self {
        Self {
            by_id: labels.student_index(),
        }
    }

    /// How many students the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the index holds no student.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Whether this student's enrolment names this section.
    ///
    /// Explicit enrolment and nothing else: never a group's membership, never a group's size.
    /// A student the index does not hold is enrolled in nothing, which is already what a
    /// `LabelSet` carrying only students with rows means.
    #[must_use]
    pub fn enrolled_in(&self, student_id: Uuid, section_id: Uuid) -> bool {
        self.by_id
            .get(&student_id)
            .is_some_and(|student| student.sections.contains(&section_id))
    }
}

/// Whether one lesson belongs in one scope.
///
/// The **only** implementation of scope membership, so a scope's grid and its lesson count
/// cannot disagree.
///
/// # Cost
///
/// Builds a [`StudentIndex`] per call, which is right for a single question and wrong inside a
/// loop. [`lay_out`] and [`lay_out_many`] therefore call [`in_scope_indexed`] with an index
/// they built once — this wrapper exists so a caller with a `LabelSet` and one question need
/// not know about the index at all.
#[must_use]
pub fn in_scope(lesson: &ProjectedLesson, scope: &ProjectionScope, labels: &LabelSet) -> bool {
    in_scope_indexed(lesson, scope, &StudentIndex::of(labels))
}

/// Whether one lesson belongs in one scope, against an index built once.
///
/// This is the body [`in_scope`] delegates to, and so the one definition of membership. The
/// only difference is how the roster is reached: a map rather than a scan, which is what makes
/// laying out every student's grid linear rather than quadratic (§14.2).
#[must_use]
pub fn in_scope_indexed(
    lesson: &ProjectedLesson,
    scope: &ProjectionScope,
    students: &StudentIndex<'_>,
) -> bool {
    match (scope.kind, scope.id) {
        (ScopeKind::Master, _) => true,
        (ScopeKind::Section, Some(id)) => lesson.section_id == id,
        (ScopeKind::Teacher, Some(id)) => lesson.teacher_ids.contains(&id),
        (ScopeKind::Room, Some(id)) => lesson.room_id == Some(id),
        // Explicit enrolment and nothing else. Never a group's membership, never a group's
        // size: substituting either would present a guess as a child's confirmed schedule.
        (ScopeKind::Student, Some(id)) => students.enrolled_in(id, lesson.section_id),
        (_, None) => false,
    }
}

/// Lays out one scope's lessons into well-formed tables.
///
/// # The layout decision, in one place
///
/// A lesson earns a `rowSpan` only when both hold: it is **alone** in every cell it occupies,
/// and its occupied periods are **contiguous** in this panel's axis. Otherwise every cell it
/// touches is [`CellContent::Pinned`] at `rowSpan` 1, carrying every lesson in that cell.
///
/// Doing this in the renderer is where a broken table comes from, which is why
/// [`GridPanel::is_well_formed`] exists and why a test runs it over every fixture and over
/// deliberately overlapping arrangements.
#[must_use]
pub fn lay_out(
    projection: &TimetableProjection,
    scope: &ProjectionScope,
    labels: &LabelSet,
) -> ScopeGrid {
    lay_out_indexed(projection, scope, &StudentIndex::of(labels))
}

/// Lays out many scopes from one projection, building the roster index once.
///
/// # Why this exists
///
/// It is the same function as [`lay_out`] applied to each scope, and it must stay that way —
/// both go through [`lay_out_indexed`], so a bulk document and a single screen grid cannot
/// disagree. What it saves is the index: [`lay_out`] builds a [`StudentIndex`] per call, so a
/// bulk output of 3 000 students would build 3 000 of them, which is the second half of
/// §14.2's cost. Here it is built once for the batch.
///
/// The result is in the caller's order, one [`ScopeGrid`] per scope given, including for a
/// scope with nothing in it — an empty grid is a state and an absent one would read as *"this
/// teacher was not exported"* (§8.4 of the proposal).
#[must_use]
pub fn lay_out_many(
    projection: &TimetableProjection,
    scopes: &[ProjectionScope],
    labels: &LabelSet,
) -> Vec<ScopeGrid> {
    let students = StudentIndex::of(labels);
    scopes
        .iter()
        .map(|scope| lay_out_indexed(projection, scope, &students))
        .collect()
}

/// The body both [`lay_out`] and [`lay_out_many`] run.
///
/// One implementation, so the single-scope and bulk paths are the same path.
#[must_use]
fn lay_out_indexed(
    projection: &TimetableProjection,
    scope: &ProjectionScope,
    students: &StudentIndex<'_>,
) -> ScopeGrid {
    let mine: Vec<usize> = projection
        .lessons
        .iter()
        .enumerate()
        .filter(|(_, lesson)| in_scope_indexed(lesson, scope, students))
        .map(|(ix, _)| ix)
        .collect();

    let mut panels = Vec::new();
    let mut conflicts = Vec::new();

    for axis in &projection.axes {
        let here: Vec<usize> = mine
            .iter()
            .copied()
            .filter(|ix| projection.lessons[*ix].schedule_id == axis.schedule_id)
            .collect();
        if here.is_empty() {
            continue;
        }
        let (panel, found) = lay_out_panel(projection, scope, axis, &here);
        panels.push(panel);
        conflicts.extend(found);
    }

    let defective = mine
        .iter()
        .copied()
        .filter(|ix| projection.lessons[*ix].defect.is_some())
        .collect();

    ScopeGrid {
        scope: scope.clone(),
        panels,
        conflicts,
        defective,
    }
}

/// Lays out one bell schedule's panel, and reports the clashes it found.
///
/// # The decision, per cell
///
/// | Occupants | Lesson state | Content |
/// | --- | --- | --- |
/// | 0 | — | [`CellContent::Empty`] |
/// | 1 | alone everywhere **and** contiguous here | [`CellContent::Lesson`] at its start row, [`CellContent::Continuation`] after |
/// | 1 | pinned for either reason | [`CellContent::Pinned`], `rowSpan` 1 |
/// | 2+ | — | [`CellContent::Pinned`], `rowSpan` 1, carrying **every** occupant |
///
/// A lesson earns a `rowSpan` only in the second row of that table. Everything else is held
/// to one cell, which is what makes [`GridPanel::is_well_formed`] hold for an arrangement
/// whose grid has moved underneath it.
fn lay_out_panel(
    projection: &TimetableProjection,
    scope: &ProjectionScope,
    axis: &ScheduleAxis,
    here: &[usize],
) -> (GridPanel, Vec<GridConflict>) {
    // Who occupies each position. A `BTreeMap` because every output of this module has to be
    // deterministic and nothing in this workspace iterates a hash map.
    let mut occupancy: BTreeMap<(u16, u16), Vec<usize>> = BTreeMap::new();
    for ix in here {
        for cell in &projection.lessons[*ix].occupies {
            occupancy
                .entry((cell.period_ordinal, cell.day_ordinal))
                .or_default()
                .push(*ix);
        }
    }
    for occupants in occupancy.values_mut() {
        occupants.sort_unstable();
        occupants.dedup();
    }

    let ordinals: Vec<u16> = axis.periods.iter().map(|p| p.ordinal).collect();
    let shared_reason = if scope.kind.expects_exclusive_use() {
        PinReason::Conflict
    } else {
        PinReason::Shared
    };

    // A lesson is pinned when it shares a cell, or when its run is not contiguous here.
    let mut pinned: BTreeMap<usize, PinReason> = BTreeMap::new();
    for ix in here {
        let lesson = &projection.lessons[*ix];
        let shares = lesson.occupies.iter().any(|cell| {
            occupancy
                .get(&(cell.period_ordinal, cell.day_ordinal))
                .is_some_and(|o| o.len() > 1)
        });
        if shares {
            pinned.insert(*ix, shared_reason);
        } else if !is_contiguous(lesson, &ordinals) {
            pinned.insert(*ix, PinReason::BrokenSpan);
        }
    }

    let mut conflicts = Vec::new();
    let mut cells = Vec::with_capacity(axis.periods.len() * projection.days.len());
    for period in &axis.periods {
        for day in &projection.days {
            let occupants = occupancy
                .get(&(period.ordinal, day.ordinal))
                .cloned()
                .unwrap_or_default();
            let content = match occupants.len() {
                0 => CellContent::Empty,
                1 => single(projection, &pinned, occupants, period.ordinal),
                _ => {
                    if shared_reason == PinReason::Conflict {
                        conflicts.push(GridConflict {
                            schedule_id: axis.schedule_id,
                            day_ordinal: day.ordinal,
                            day_label: day.label.clone(),
                            period_ordinal: period.ordinal,
                            period_label: period.label.clone(),
                            lessons: occupants.clone(),
                        });
                    }
                    CellContent::Pinned {
                        lessons: occupants,
                        reason: shared_reason,
                    }
                }
            };
            cells.push(TimetableCell {
                day_ordinal: day.ordinal,
                period_ordinal: period.ordinal,
                content,
            });
        }
    }

    (
        GridPanel {
            schedule_id: axis.schedule_id,
            schedule_name: axis.schedule_name.clone(),
            campus_name: axis.campus_name.clone(),
            days: projection.days.clone(),
            periods: axis.periods.clone(),
            cells,
        },
        conflicts,
    )
}

/// The content of a cell with exactly one occupant.
///
/// The only place a `rowSpan` is produced, and it is produced only when the lesson is not
/// pinned — so a span can never cross a cell somebody else is in.
fn single(
    projection: &TimetableProjection,
    pinned: &BTreeMap<usize, PinReason>,
    occupants: Vec<usize>,
    period_ordinal: u16,
) -> CellContent {
    let only = occupants[0];
    if let Some(reason) = pinned.get(&only) {
        return CellContent::Pinned {
            lessons: occupants,
            reason: *reason,
        };
    }
    let lesson = &projection.lessons[only];
    let starts_here = lesson
        .occupies
        .first()
        .is_some_and(|c| c.period_ordinal == period_ordinal);
    if starts_here {
        CellContent::Lesson {
            lesson: only,
            rows: u16::try_from(lesson.occupies.len()).unwrap_or(1).max(1),
        }
    } else {
        CellContent::Continuation
    }
}

/// Whether a lesson's occupied periods are consecutive rows of this axis.
///
/// Not the same question as *"are the slots ordinal-consecutive?"*: the resolver takes the
/// run from the slots that were **materialised**, so a slot the grid never built leaves a
/// hole, and a `rowSpan` over a hole is a broken table.
fn is_contiguous(lesson: &ProjectedLesson, ordinals: &[u16]) -> bool {
    let mut rows = lesson
        .occupies
        .iter()
        .map(|cell| ordinals.iter().position(|o| *o == cell.period_ordinal));
    let Some(Some(first)) = rows.next() else {
        return false;
    };
    let mut expected = first;
    for row in rows {
        expected += 1;
        if row != Some(expected) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod budget;
#[cfg(test)]
mod tests;
