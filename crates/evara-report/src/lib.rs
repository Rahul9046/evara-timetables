//! Timetable view projections, and later the output renderers.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and [`evara_constraints`]. **Receives data; does not query for
//! it.** No SQLite, no file, no network: a projection is a pure function of a compiled
//! problem, an arrangement over it, and the display labels for its school.
//!
//! The `evara-constraints` edge exists for one reason and buys one thing:
//! [`findings::explain_all`] renders the sentence that belongs to the rule that produced a
//! violation, rather than composing one here.
//! [ADR 0043](../../../docs/adr/0043-a-report-renders-a-rules-own-explanation.md) records it.
//! `evara-constraints` has no I/O and no SQL, so the edge adds nothing this crate should not
//! have — and `CLAUDE.md` §2's dependency table says `evara-report -> core, constraints`.
//!
//! # What lives here (M8)
//!
//! | Module | Answers |
//! | --- | --- |
//! | [`projection`] | *what occupies Day A period 2, and in which grids?* |
//! | [`findings`] | *what is wrong with this arrangement, in words the rule chose?* |
//! | [`output`] | *what goes into a file, and what must a reader be told?* (M11) |
//!
//! # One projection, two callers
//!
//! `evara_report::projection::project` is called by the Generate screen's preview **and** by
//! the saved-timetable view. There is deliberately no second renderer, so a preview and the
//! timetable it becomes cannot disagree.
//!
//! # Three things this crate restates nowhere
//!
//! Each already has exactly one implementation, and a copy here would be `CLAUDE.md` §2's
//! most important rule broken:
//!
//! | Question | The one implementation |
//! | --- | --- |
//! | which slots does an occurrence occupy? | `evara_core::scheduling::span::resolve_activity_span` |
//! | what does this violation mean? | the rule's own `explain()`, via [`evara_constraints::evaluator_for`] |
//! | which bell schedule did it draw its slots from? | the starting slot's, as the resolver decided |
//!
//! # Not here, and when
//!
//! Phase 10 is what the header above was written for, and M11 collects on it in stages. The
//! shared artefact arrived first: [`output::OutputDocument`] is composed once and read by
//! every format, and [`output::DocumentRenderer`] is declared over the two formats Evara
//! renders itself. **Neither renderer is written yet** — the CSV and XLSX backends, the print
//! stylesheet and the print view are M11 Stage 2 and Stage 3. A trait with one implementation
//! and no second caller is a guess about the second, and a placeholder implementation added to
//! demonstrate a trait is the same guess with a test beside it, so the trait ships with none.
//!
//! What still does not live here, and will not: a file. A renderer returns **bytes**. Judging
//! a destination, claiming it and renaming onto it belongs to the command layer and the CLI,
//! which is where `DestinationClaim` already is.

pub mod findings;
pub mod output;
pub mod projection;
pub mod repair;

pub use findings::{
    FindingCounts, FindingEntity, FindingSource, FindingView, StoredFinding, explain_all,
};
pub use output::{
    CellLesson, CompositionInput, ConflictLine, DefectLine, Disclosure, DisclosureCode,
    DisclosureEntity, DocumentFinding, DocumentHeader, DocumentIdentity, DocumentLogo,
    DocumentRenderer, Orientation, OutputCell, OutputCellKind, OutputDocument, OutputError,
    OutputFormat, OutputOptions, OutputPanel, OutputSheet, PageSetup, PaperSize, RunProvenance,
    UnscheduledLine, UnscheduledOccurrence, compose,
};
pub use projection::{
    CellContent, CellDetail, CellLines, GridConflict, GridPanel, LessonFlags, PinReason,
    ProjectedCellRef, ProjectedDay, ProjectedLesson, ProjectedPeriod, ProjectionInput,
    ProjectionScope, ScheduleAxis, ScopeGrid, ScopeKind, StudentIndex, TimetableCell,
    TimetableProjection, in_scope, in_scope_indexed, lay_out, lay_out_many, no_flags, project,
};
pub use repair::{
    Attribution, ChangeCounts, ChangeKind, MovedParts, OccurrenceChange, PlacementView, RepairDiff,
    RepairDiffInput, diff,
};
