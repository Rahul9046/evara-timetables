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
//! The renderer trait and the HTML, CSV and PDF backends are ROADMAP Phase 10. A trait with
//! one implementation and no second caller is a guess about the second, so the projections
//! are shaped to be rendered without being shaped by a renderer — which is what the original
//! header promised and what Phase 10 collects on.

pub mod findings;
pub mod projection;
pub mod repair;

pub use findings::{
    FindingCounts, FindingEntity, FindingSource, FindingView, StoredFinding, explain_all,
};
pub use projection::{
    CellContent, GridConflict, GridPanel, LessonFlags, PinReason, ProjectedCellRef, ProjectedDay,
    ProjectedLesson, ProjectedPeriod, ProjectionInput, ProjectionScope, ScheduleAxis, ScopeGrid,
    ScopeKind, TimetableCell, TimetableProjection, in_scope, lay_out, no_flags, project,
};
pub use repair::{
    Attribution, ChangeCounts, ChangeKind, MovedParts, OccurrenceChange, PlacementView, RepairDiff,
    RepairDiffInput, diff,
};
