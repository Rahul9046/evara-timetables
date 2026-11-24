//! One output document, composed once and rendered many times.
//!
//! # What this module is for
//!
//! A timetable leaves Evara three ways: a machine-readable CSV, a human-readable Excel
//! workbook, and a printed page or PDF through the platform's own print pipeline. Those three
//! share almost nothing about *how* they are produced — CSV consumes a row stream, a workbook
//! consumes a grid with merges, and printing consumes a document in a webview — but they must
//! agree completely about *what* they say. So the shared artefact is one level lower than a
//! renderer: an [`OutputDocument`], built here, read by all three.
//! `M11-TIMETABLE-OUTPUT-PROPOSAL.md` §4 and §5.
//!
//! [`DocumentRenderer`] is declared only over the two formats Evara renders **itself**. The
//! print view is a third *consumer* of the same document over IPC, not an implementation of
//! the trait: it would have to return the bytes of an HTML document, and then the thing that
//! paginates, hyphenates and prints it is the webview — so the trait would be returning an
//! input to a renderer rather than a rendered artefact (§5.5).
//!
//! # Two rules the model enforces structurally
//!
//! **Nothing is omitted to make a format render.** Every occupant of a cell is carried, never
//! the first; a panel is never merged with another; an unplaced occurrence, a conflict, a
//! broken run and a finding are sections of the document rather than options. Where the
//! caller's own filters exclude something, the exclusion is **counted and disclosed**
//! ([`OutputSheet::omitted_lesson_count`], [`DisclosureCode::LessonsOmitted`]) so that
//! intentional narrowing and accidental data loss cannot look alike. §4.4.
//!
//! **Nothing is resolved twice.** [`compose`] calls nothing it could restate:
//!
//! | Question | The one implementation it calls |
//! | --- | --- |
//! | how tall is a merged cell? | [`CellContent::Lesson`]'s `rows`, decided by `lay_out` |
//! | which slots does an occurrence occupy? | [`ProjectedLesson::occupies`], from the span resolver |
//! | is this cell a clash? | [`ScopeGrid::conflicts`], found by `lay_out` |
//! | what is this lesson called? | [`CellLines`], composed by `project` |
//! | what does this violation mean? | the rule's own `explain()`, through [`FindingView`](crate::findings::FindingView) |
//! | does this cause prove impossibility? | [`UnplacedCause::is_impossible`](evara_core::scheduling::UnplacedCause::is_impossible) |
//!
//! # The crate boundary is unchanged
//!
//! `evara-report` *"receives data; does not query for it"*. This module adds no SQLite, no
//! file, no network and no dependency on `evara-project` or Tauri: a renderer produces
//! **bytes**, and choosing a destination, claiming it and renaming onto it belongs to the
//! caller (§2.1, §10.1). Nothing here reads a clock either — [`DocumentHeader::generated_at`]
//! is supplied by the caller.
//!
//! # A campus is an axis, not a scope
//!
//! There is no `ScopeKind::Campus` and there must not be one. A campus reaches a document as
//! [`OutputPanel::campus_name`], and *"the Northgate timetable"* is the master grid's
//! Northgate panel. Selecting one is [`OutputOptions::schedule_ids`] filtering panels. Merging
//! two campuses' Period 3 into one row is what ADR 0033 forbids. §3.1.

pub mod guard;
pub mod names;

mod compose;

#[cfg(test)]
mod tests;

use evara_core::scheduling::display::{RoomLabel, SectionLabel};
use evara_core::scheduling::span::SpanDefect;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::projection::{
    CellDetail, CellLines, LessonFlags, PinReason, ProjectedDay, ProjectedPeriod, ProjectionScope,
    ScopeKind,
};

pub use compose::{CompositionInput, DocumentIdentity, UnscheduledOccurrence, compose};

// =======================================================================================
// The document
// =======================================================================================

/// One output document: what is rendered, already decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputDocument {
    /// School, cycle, term, timetable, provenance and the caller's stamp.
    pub header: DocumentHeader,
    /// One per selected entity, in the caller's selection order.
    pub sheets: Vec<OutputSheet>,
    /// Occurrences with no placement, for the whole timetable.
    ///
    /// Never per sheet: an unplaced occurrence is in no scope's grid, because it has no
    /// position to be in one at. A document that ended at the grid would omit silently the
    /// lessons that have nowhere to be — §2.3 finding 1.
    pub unscheduled: Vec<UnscheduledLine>,
    /// Findings, in the words of the rules that produced them.
    ///
    /// Document-wide for the same reason as [`OutputDocument::unscheduled`]: a finding is
    /// about an arrangement, and several of them name no position at all. The document is one
    /// school's by construction (§9.1), so a document-wide appendix cannot reach another
    /// school's data.
    pub findings: Vec<DocumentFinding>,
    /// What a reader must see for the document to be honest.
    ///
    /// Not an option. Every entry with [`Disclosure::is_mandatory`] has to be rendered by
    /// every format; the cross-campus sentence in particular is unsuppressible, because an
    /// option to hide it would be an option to produce a misleading document (§9.3).
    pub disclosures: Vec<Disclosure>,
    /// What the options asked for, so a format can print its own provenance.
    pub options: OutputOptions,
}

impl OutputDocument {
    /// How many distinct occurrences the sheets carry, counting one that appears in two
    /// sheets once.
    #[must_use]
    pub fn distinct_lessons(&self) -> usize {
        let mut seen: std::collections::BTreeSet<Uuid> = std::collections::BTreeSet::new();
        for sheet in &self.sheets {
            for panel in &sheet.panels {
                for cell in &panel.cells {
                    for lesson in &cell.lessons {
                        seen.insert(lesson.activity_id);
                    }
                }
            }
        }
        seen.len()
    }

    /// How many occurrences the caller's own filters left out of every sheet.
    ///
    /// Zero for the default options. Non-zero means a day, a panel or a bell schedule was
    /// deselected, which is a narrowing the reader asked for — and is disclosed rather than
    /// left to be inferred from a short document.
    #[must_use]
    pub fn omitted_lessons(&self) -> usize {
        self.sheets.iter().map(|s| s.omitted_lesson_count).sum()
    }

    /// Every disclosure a format is obliged to render.
    #[must_use]
    pub fn mandatory_disclosures(&self) -> Vec<&Disclosure> {
        self.disclosures.iter().filter(|d| d.is_mandatory).collect()
    }
}

/// What the document is about, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DocumentHeader {
    /// The school, from the projection. One document, one school.
    pub school_name: String,
    /// The cycle its grid is over.
    pub cycle_name: String,
    /// The term, where it names one.
    pub term_name: Option<String>,
    /// What the timetable is called.
    pub timetable_name: String,
    /// Which timetable.
    pub timetable_id: Uuid,
    /// Its revision, so a file on somebody's desk can be matched to a version (§8.5).
    pub timetable_rev: i64,
    /// Engine, seed, status and termination. `None` when no run is recorded.
    pub provenance: Option<RunProvenance>,
    /// Supplied by the caller. This crate reads no clock.
    pub generated_at: String,
    /// Which build composed it.
    pub app_version: String,
    /// The school's logo, when it has one and the options asked for it.
    pub logo: Option<DocumentLogo>,
}

/// How the timetable in this document came to be.
///
/// A report's own flattening of the four facts a run records, not `evara-db`'s
/// `RunProvenance` — this crate may not see that crate, and the caller passes the four across.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunProvenance {
    /// Which engine ran.
    pub engine: String,
    /// The seed every ordering decision derived from.
    pub seed: i64,
    /// What came back: `COMPLETE`, `PARTIAL`, `INFEASIBLE`, `CANCELLED` or `FAILED`.
    pub status: String,
    /// Why it stopped looking.
    pub termination: String,
}

/// A school's logo, as bytes a renderer can embed.
///
/// Bytes rather than a path or a data URL, because `rust_xlsxwriter`'s `Image::new_from_buffer`
/// takes bytes and this crate opens no file. `evara_core::branding` has already sniffed the
/// format and bounded the dimensions, so a logo that reached the database is one a writer can
/// take; a format a writer cannot take is **omitted with a disclosure**, never substituted
/// (§6.2, §9.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DocumentLogo {
    /// The image, as stored.
    pub bytes: Vec<u8>,
    /// Its sniffed media type.
    pub media_type: String,
    /// Pixel width, as stored.
    pub width: u32,
    /// Pixel height, as stored.
    pub height: u32,
}

// =======================================================================================
// Sheets, panels and cells
// =======================================================================================

/// One selected entity's part of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputSheet {
    /// Which grid this is: the kind, the entity, its name and its secondary line.
    pub scope: ProjectionScope,
    /// One per bell schedule the scope touches and the options kept.
    ///
    /// Empty when [`OutputSheet::is_empty`] is true — the scope holds nothing — **or** when
    /// the options filtered every panel away, which [`OutputSheet::omitted_lesson_count`]
    /// distinguishes. Either way the sheet is *present*: a school exporting every teacher
    /// needs to see which teachers have nothing, and an absent sheet would read as *"this
    /// teacher was not exported"* (§8.4).
    pub panels: Vec<OutputPanel>,
    /// True when the scope holds no placed occurrence in this timetable at all.
    ///
    /// A state, not an absence. False with empty `panels` means the caller's filters removed
    /// everything this scope holds.
    pub is_empty: bool,
    /// Cells where more than one occurrence falls and this scope expects exclusive use.
    pub conflicts: Vec<ConflictLine>,
    /// Occurrences in this scope whose run is not sound.
    pub defective: Vec<DefectLine>,
    /// How many distinct occurrences the sheet's cells carry.
    pub lesson_count: usize,
    /// How many of the scope's occurrences the caller's filters left out.
    ///
    /// The difference between *"this teacher teaches nothing on the days you asked for"* and
    /// *"a lesson went missing"*. Zero for the default options.
    pub omitted_lesson_count: usize,
    /// Sentences this sheet must carry, over and above the document's.
    ///
    /// A teacher's sheet headed with their name reads as *their timetable*, and for somebody
    /// employed at two schools it is their timetable **here** — so the sentence belongs on the
    /// sheet as well as on the cover (§9.3).
    pub disclosures: Vec<Disclosure>,
}

/// One bell schedule's grid: days across, periods down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputPanel {
    /// The schedule this panel is for.
    pub schedule_id: Uuid,
    /// What it is called.
    pub schedule_name: String,
    /// The campus it serves, where it names one.
    ///
    /// Named above every grid block in every format: two panels in one document with no label
    /// is the one way this model can confuse a reader (§9.2).
    pub campus_name: Option<String>,
    /// The day columns the options kept, in `ordinal` order.
    pub days: Vec<ProjectedDay>,
    /// The period rows the options kept, in `ordinal` order.
    pub periods: Vec<ProjectedPeriod>,
    /// Row-major, `periods × days`, exactly as `GridPanel::cells`.
    pub cells: Vec<OutputCell>,
}

impl OutputPanel {
    /// The cell at one position, or `None` if the panel has no such position.
    #[must_use]
    pub fn cell(&self, day_ordinal: u16, period_ordinal: u16) -> Option<&OutputCell> {
        self.cells
            .iter()
            .find(|c| c.day_ordinal == day_ordinal && c.period_ordinal == period_ordinal)
    }
}

/// One `(day, period)` position of one panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutputCell {
    /// Which day column.
    pub day_ordinal: u16,
    /// Which period row.
    pub period_ordinal: u16,
    /// What is in it.
    pub kind: OutputCellKind,
    /// Every occurrence in this cell, in the panel's order. **Never truncated.**
    ///
    /// A [`OutputCellKind::Continuation`] carries the occurrence that covers it, so a flat
    /// format can emit one row per occupied slot without having to work out which lesson a
    /// merge belongs to.
    pub lessons: Vec<CellLesson>,
    /// Merge height. 1 for everything but a [`OutputCellKind::Lesson`] that earned a span.
    ///
    /// Taken from `CellContent::Lesson { rows }`, never recomputed: a renderer that worked out
    /// its own spans is where a broken table comes from, which is the whole reason `lay_out`
    /// decides this.
    pub rows: u16,
}

impl OutputCell {
    /// The stable `cell_state` key, as the CSV column contract spells it.
    ///
    /// The one implementation of the mapping, so the CSV, the workbook's formats and the
    /// printed page cannot disagree about what a cell is. §6.1 column 13.
    #[must_use]
    pub const fn state_key(&self) -> &'static str {
        match self.kind {
            OutputCellKind::Empty => "empty",
            OutputCellKind::NonTeaching => "non_teaching",
            OutputCellKind::Lesson => "lesson",
            OutputCellKind::Continuation => "continuation",
            OutputCellKind::Pinned {
                reason: PinReason::Conflict,
            } => "conflict",
            OutputCellKind::Pinned {
                reason: PinReason::Shared,
            } => "shared",
            OutputCellKind::Pinned {
                reason: PinReason::BrokenSpan,
            } => "broken_run",
        }
    }

    /// How many occurrences are in this cell. `2` is how a consumer finds a clash.
    #[must_use]
    pub fn occupants(&self) -> usize {
        self.lessons.len()
    }
}

/// What kind of thing one cell is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum OutputCellKind {
    /// Nothing is placed, and teaching is permitted here.
    Empty,
    /// Nothing is placed, and nothing may be: a break.
    ///
    /// Kept apart from [`OutputCellKind::Empty`] so that *"nothing is scheduled here"* and
    /// *"nothing may be scheduled here"* do not look alike, and **never dropped** — dropping a
    /// break would renumber the rows a reader counts (§4.3).
    NonTeaching,
    /// One occurrence, alone and contiguous. The only kind with `rows > 1`.
    Lesson,
    /// Covered by a [`OutputCellKind::Lesson`] above in the same column.
    ///
    /// A grid renderer skips it; a flat format still emits a row for it, carrying the covering
    /// occurrence with its part index.
    Continuation,
    /// Held to one cell, carrying every occupant. `rows` is always 1.
    Pinned {
        /// Why no span was given, carried verbatim from `lay_out`.
        reason: PinReason,
    },
}

/// One occurrence as a cell carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CellLesson {
    /// The stored occurrence.
    pub activity_id: Uuid,
    /// The section it belongs to.
    pub section_id: Uuid,
    /// The already-composed lines, in reading order. The one implementation (§5.2).
    pub lines: CellLines,
    /// The section's label, where the set holds it.
    ///
    /// Carried **beside** the composed lines, not instead of them: the CSV column contract
    /// wants `subject`, `course`, `section` and `year_level` unjoined, and a grid wants the
    /// joined caption. Both forms, one composition.
    pub label: Option<SectionLabel>,
    /// Who teaches it. All of them, in the section's order.
    pub teacher_ids: Vec<Uuid>,
    /// The student groups that attend.
    pub group_ids: Vec<Uuid>,
    /// The room booked, where one was.
    ///
    /// One room, because one room is what a `Placement` can express. "Multiple rooms on an
    /// activity" is not representable today and nothing here invents a representation for it
    /// (§2.3 finding 3).
    pub room_id: Option<Uuid>,
    /// The room's label, where the set holds it — for the `building` column.
    pub room: Option<RoomLabel>,
    /// `'HH:MM'` of the first slot the occurrence covers.
    pub starts_at: String,
    /// `'HH:MM'` of the last slot it covers.
    pub ends_at: String,
    /// How many periods the occurrence asked for.
    pub duration_periods: u16,
    /// 1-based position of **this cell** within the occurrence's run.
    ///
    /// Read off [`ProjectedLesson::occupies`], so it is right for a broken run as well as a
    /// contiguous one: a flat format emits `2 of 3` rather than guessing from a merge height
    /// that a broken run does not have.
    pub part: u16,
    /// How many slots the run occupies in total.
    pub parts: u16,
    /// Whether the entry is locked or placed by hand.
    pub flags: LessonFlags,
    /// What is wrong with the run, where anything is.
    pub defect: Option<SpanDefect>,
}

// =======================================================================================
// Appendices
// =======================================================================================

/// One clash a grid found.
///
/// Addresses its occurrences by **stored identifier**, where `GridConflict` addresses them by
/// index into `TimetableProjection::lessons`. A document is rendered without the projection
/// beside it and crosses the IPC boundary to the print view, and *"an index has no meaning
/// outside the problem that assigned it"* — the rule `findings::FindingView` already follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConflictLine {
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
    /// Every occurrence involved. **Never only the first.**
    pub activity_ids: Vec<Uuid>,
    /// Their captions, in the same order.
    pub captions: Vec<String>,
}

/// One occurrence whose run is not sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DefectLine {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// What it is called.
    pub caption: String,
    /// What is wrong with the run, as the span resolver reported it.
    pub defect: SpanDefect,
    /// Which schedule it drew its slots from.
    pub schedule_id: Uuid,
    /// Which day it runs on.
    pub day_ordinal: u16,
    /// That day, by name.
    pub day_label: String,
    /// How many slots it occupies, against how many periods it asked for.
    pub occupied_periods: u16,
    /// What the occurrence asked for.
    pub duration_periods: u16,
}

/// One occurrence with no placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UnscheduledLine {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// What the lesson is called, where the caller knew.
    pub caption: Option<String>,
    /// The cause key, as stored.
    pub cause: String,
    /// Whether this build recognises the cause at all.
    ///
    /// A key written by a newer build is reported as unknown rather than dropped, and is
    /// **not** reported as impossible.
    pub known_cause: bool,
    /// Whether this cause proves **no** timetable can place it.
    ///
    /// Decided by `UnplacedCause::is_impossible` through `UnplacedCause::from_key`, which is
    /// the one place the difference between *"no timetable can place this"* and *"this
    /// arrangement did not"* is decided. Never composed here, and never true for a cause this
    /// build does not recognise.
    pub is_impossible: bool,
    /// The pre-solve finding that already proved it, where there was one.
    pub diagnostic_code: Option<String>,
    /// What the engine said, where the caller had a sentence.
    pub reason: Option<String>,
}

/// One finding, in the words of the rule that produced it.
///
/// Named `DocumentFinding` rather than `FindingLine` because the command layer already owns
/// that name for a **pre-solve diagnostic** (`commands::solve::FindingLine`), which is a
/// different thing — a diagnostic is about the problem and this is about an arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DocumentFinding {
    /// The registry's stable `type_key`.
    pub type_key: String,
    /// Whether this build implements the rule that produced it.
    pub known_type: bool,
    /// `HARD`, `MEDIUM` or `SOFT`.
    pub level: String,
    /// Negative-going, as every penalty in this workspace is.
    pub penalty: i64,
    /// The rule's **own** sentence, where a problem was available to produce one.
    ///
    /// Carried verbatim from the evaluator's `explain()` and never re-worded: a report that
    /// composed its own wording would be a second implementation of every rule's reasoning
    /// (ADR 0043). `None` for a finding read from a stored row, which deliberately carries no
    /// sentence.
    pub message: Option<String>,
    /// The specific fact the rule recorded.
    pub detail: String,
    /// The cycle day it is about, by name, where it has a when.
    pub day_label: Option<String>,
    /// `'HH:MM'`.
    pub starts_at: Option<String>,
    /// `'HH:MM'`.
    pub ends_at: Option<String>,
    /// The occurrences it names, by stored identifier.
    pub activity_ids: Vec<Uuid>,
}

// =======================================================================================
// Disclosures
// =======================================================================================

/// Something a reader must be told for the document not to mislead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Disclosure {
    /// Which disclosure this is, so a format can style or place it.
    pub code: DisclosureCode,
    /// The sentence, composed here because it is about the *document* rather than about a
    /// scheduling rule — no evaluator owns a sentence for "this file is a snapshot".
    pub message: String,
    /// Whether every format is obliged to render it.
    ///
    /// True for the cross-campus sentence, the roster-derived caveat and the snapshot line.
    /// No [`OutputOptions`] value can clear a mandatory disclosure, because an option to hide
    /// one would be an option to produce a misleading document (§9.3).
    pub is_mandatory: bool,
    /// The entity it is about, where it is about one.
    pub entity: Option<DisclosureEntity>,
    /// How many things it is about, where that is the point of it.
    pub count: Option<usize>,
}

/// Which disclosure a [`Disclosure`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum DisclosureCode {
    /// The document is a snapshot of one revision at one moment.
    Snapshot,
    /// A selected teacher or room is also held by another school, whose commitments are not
    /// in this document and are not compared against it.
    SharedEntity,
    /// A student's timetable is derived from explicit enrolment rows and from nothing else.
    RosterDerived,
    /// The caller's day selection left days of the cycle out.
    DaysOmitted,
    /// The caller's bell-schedule selection left panels out.
    PanelsOmitted,
    /// Non-teaching period rows that held nothing were left out.
    NonTeachingOmitted,
    /// Occurrences this scope holds fall outside the caller's selection.
    LessonsOmitted,
    /// The caller asked for the unplaced occurrences to be left out, and there are some.
    UnscheduledOmitted,
    /// The caller asked for the findings to be left out, and there are some.
    FindingsOmitted,
}

/// The entity a disclosure is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DisclosureEntity {
    /// `teacher`, `room`, `section` or `student`.
    pub kind: String,
    /// Which one.
    pub id: Uuid,
    /// What it is called.
    pub name: String,
}

// =======================================================================================
// Options
// =======================================================================================

/// What the caller asked for.
///
/// Every field has a default chosen so the common request needs no configuration: the current
/// scope, every panel, every day, breaks shown, standard detail, the logo on, A4 landscape,
/// and both appendices included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[allow(
    clippy::struct_excessive_bools,
    reason = "five independent presentation toggles, not one multi-state fact encoded as \
              flags. `DestinationState` (evara-db) took the opposite decision because its \
              three booleans admitted combinations that cannot exist — \"absent but is a \
              project\" — and an enum removed them. Here every one of the 32 combinations is \
              legal and meaningful: showing breaks says nothing about showing the logo. An \
              enum or a flag set would make the IPC face of this type (§12.2's \
              `ReportRequest`, which carries the same four) harder to read and no safer."
)]
pub struct OutputOptions {
    /// Which format the document is destined for.
    ///
    /// `None` for the print view, which takes the document rather than a format (§5.5) —
    /// reconciling §5.3's field with §12.2's `ReportRequest`, whose `format` is already
    /// optional for exactly this reason.
    pub format: Option<OutputFormat>,
    /// Which kind of grid.
    pub scope_kind: ScopeKind,
    /// Which entities. Empty means every entity of this kind that the projection offers.
    pub scope_ids: Vec<Uuid>,
    /// Which bell schedules' panels. Empty means every panel the selection touches.
    pub schedule_ids: Vec<Uuid>,
    /// Which cycle days, by `ordinal`. Empty means the whole cycle, in `ordinal` order.
    pub day_ordinals: Vec<u16>,
    /// Whether a break is a row. Default true, and a break that holds something is always a
    /// row whatever this says.
    pub include_non_teaching: bool,
    /// How much a cell says.
    pub detail: CellDetail,
    /// Whether to carry the school's logo. Ignored where the school has none.
    pub include_logo: bool,
    /// Paper and orientation, honoured by the workbook and the printed page.
    pub page: PageSetup,
    /// Whether the unplaced occurrences are a section of the document.
    pub include_unscheduled: bool,
    /// Whether the findings are a section of the document.
    pub include_findings: bool,
    /// Whether a flat format emits a row for an empty teaching cell.
    ///
    /// Read by the CSV renderer and by nothing in [`compose`]: an empty cell is always a cell
    /// of the document, because the position has to exist for the grid to be a table. Default
    /// false, which is safe **only** because the day and period ordinals make every position
    /// addressable, so an absent row is unambiguous (§6.1).
    pub include_empty: bool,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            format: None,
            scope_kind: ScopeKind::Master,
            scope_ids: Vec::new(),
            schedule_ids: Vec::new(),
            day_ordinals: Vec::new(),
            include_non_teaching: true,
            detail: CellDetail::Standard,
            include_logo: true,
            page: PageSetup::default(),
            include_unscheduled: true,
            include_findings: true,
            include_empty: false,
        }
    }
}

impl OutputOptions {
    /// The default options for one kind of grid.
    #[must_use]
    pub fn for_kind(scope_kind: ScopeKind) -> Self {
        Self {
            scope_kind,
            ..Self::default()
        }
    }

    /// The same options narrowed to one entity.
    #[must_use]
    pub fn for_entity(scope_kind: ScopeKind, id: Uuid) -> Self {
        Self {
            scope_kind,
            scope_ids: vec![id],
            ..Self::default()
        }
    }
}

/// A format Evara renders itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum OutputFormat {
    /// Long form, one row per occupied slot.
    Csv,
    /// A readable grid per entity.
    Xlsx,
}

impl OutputFormat {
    /// The stable key, as a command argument spells it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }

    /// Every format, in the order a dialogue lists them.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Xlsx, Self::Csv]
    }

    /// The format one key names, or `None` where it names nothing.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|f| f.key() == key)
    }
}

/// What size paper the page is set up for.
///
/// Two values on purpose: the brief asks for A4 and for portrait and landscape, and a general
/// paper-size list is a report designer's feature (§3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PaperSize {
    /// A4. The default.
    #[default]
    A4,
    /// US Letter.
    Letter,
}

impl PaperSize {
    /// The stable key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
        }
    }

    /// Every value.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::A4, Self::Letter]
    }

    /// The size one key names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|p| p.key() == key)
    }
}

/// Which way up the page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Orientation {
    /// Taller than wide.
    Portrait,
    /// Wider than tall. The default, because a cycle is a wide thing.
    #[default]
    Landscape,
}

impl Orientation {
    /// The stable key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    /// Every value.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Landscape, Self::Portrait]
    }

    /// The orientation one key names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|o| o.key() == key)
    }
}

/// Paper and orientation together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PageSetup {
    /// What size paper.
    pub paper: PaperSize,
    /// Which way up.
    pub orientation: Orientation,
}

// =======================================================================================
// Errors
// =======================================================================================

/// Why a document could not be composed, or a renderer refused.
///
/// Names no path, because this crate never has one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    /// The options named a scope the projection does not offer.
    #[error("{kind:?} {id} is not in this timetable")]
    UnknownScope {
        /// Which kind of grid was asked for.
        kind: ScopeKind,
        /// The identifier that names nothing.
        id: Uuid,
    },
    /// The options named a bell schedule the selection does not touch.
    #[error("bell schedule {id} is not in this timetable")]
    UnknownSchedule {
        /// The identifier that names nothing.
        id: Uuid,
    },
    /// The options named a cycle day the cycle does not hold.
    ///
    /// A refusal rather than a silently dropped filter: a caller asking for day 7 of a five-day
    /// cycle has misunderstood something, and a document that quietly answered with five days
    /// would hide it. The mirror of [`OutputError::UnknownSchedule`].
    #[error("day {ordinal} is not in this cycle")]
    UnknownDay {
        /// The ordinal that names nothing.
        ordinal: u16,
    },
    /// A per-student document was asked for and no student carries an enrolment row.
    ///
    /// Refused with a sentence rather than answered with an empty picker or a blank grid. A
    /// student view is derived from `class_enrolment` and from nothing else, so with no rows
    /// there is nothing to derive — and filling it from a group's membership or a group's size
    /// would present a guess as a child's confirmed schedule (ADR 0006).
    #[error(
        "no student in this school carries an enrolment row, so a per-student timetable \
         cannot be derived"
    )]
    NoEnrolments,
    /// Every selected day or panel was filtered away, so the document would be blank.
    ///
    /// A refusal rather than an empty file, because an empty file reads as *"there was
    /// nothing"* rather than *"you filtered everything out"*.
    #[error("the selection leaves nothing to render: {detail}")]
    EmptySelection {
        /// Which part of the selection emptied it.
        detail: String,
    },
    /// The workbook writer refused. Carries its sentence.
    #[error("the workbook could not be written: {detail}")]
    Workbook {
        /// The writer's own words.
        detail: String,
    },
    /// Worksheet names could not be made unique within Excel's rules.
    #[error("worksheet names could not be made unique: {detail}")]
    SheetNames {
        /// Which names clashed.
        detail: String,
    },
}

// =======================================================================================
// The renderer contract
// =======================================================================================

/// A renderer that turns a composed document into a file's bytes.
///
/// # Declared narrowly, on purpose
///
/// Over the two Rust byte-producing formats and no further. CSV is a row stream and XLSX is a
/// grid, and this is the whole of what they share: a document in, bytes out, a media type and
/// an extension to name the result.
///
/// The print path is **not** an implementation. It would have to return the bytes of an HTML
/// document, and then the thing that paginates, hyphenates, measures and prints it is the
/// webview — so the trait would be returning an *input* to a renderer, and the one format the
/// user cares most about would be the one the trait described least. The print page is a third
/// consumer of [`OutputDocument`], over IPC, in the layer that owns the window. §5.5.
///
/// # Bytes, not a path
///
/// Because `evara-report` writes no files. Judging a destination, claiming it and renaming onto
/// it is the command layer's and the CLI's, which is where `DestinationClaim` already is
/// (§10.1).
///
/// # No implementation yet
///
/// Stage 1 declares the contract and stops there. A trait with one implementation and no
/// second caller is a guess about the second (`lib.rs`), and a placeholder implementation
/// added to demonstrate a trait is the same guess with a test beside it. The CSV and XLSX
/// renderers are Stage 2.
pub trait DocumentRenderer {
    /// A stable key for the format: `"csv"`, `"xlsx"`.
    fn key(&self) -> &'static str;

    /// The file extension, without a dot.
    fn extension(&self) -> &'static str;

    /// The media type a caller reports.
    fn media_type(&self) -> &'static str;

    /// Renders the whole document.
    ///
    /// # Errors
    ///
    /// [`OutputError::Workbook`] or [`OutputError::SheetNames`] where the underlying writer
    /// refuses. A renderer composes nothing: everything it needs was decided by [`compose`].
    fn render(&self, document: &OutputDocument) -> Result<Vec<u8>, OutputError>;
}
