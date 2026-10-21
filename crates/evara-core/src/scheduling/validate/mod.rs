//! Pre-solve validation: what is wrong with a scheduling problem, before anything solves it.
//!
//! # The one rule
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2 classifies every incomplete or unknown input,
//! and read as a whole it says one thing. This module exists to make it true:
//!
//! > **A hard error is reserved for an input whose remedy is a field the user can fill in;
//! > everything else is counted and announced; nothing is assumed.**
//!
//! So an unresolved bell schedule refuses the solve — the remedy is one field, and without it
//! there is no value range at all. A teacher shared with another school only warns, because
//! the remedy is to coordinate two schools' timetables and that is the thing Evara cannot do.
//!
//! # What it never claims
//!
//! An empty report means **nothing was proved impossible**. It does not mean a timetable
//! exists. `evara_core::lessons`' rule binds this module exactly as it binds that one:
//!
//! > Never report that a demand *can* be scheduled.
//!
//! [`DiagnosticReport::may_solve`] is therefore named for what it decides — whether a solve is
//! worth starting — and not for feasibility, which only M6 can speak to.
//!
//! # Pure, and that is the whole design
//!
//! [`validate`] takes a [`Problem`] and returns findings. It opens no database, reads no file,
//! reads no clock, needs no Tauri runtime, mutates nothing, and is deterministic. That is why
//! it lives in `evara-core` beside the vocabulary rather than in `evara-constraints`:
//! `ARCHITECTURE.md` §3 rule 2 reserves that crate for `type_key`-addressed evaluators scored
//! against a *candidate timetable*, and a rule with an answer **before any placement exists**
//! is not one of those. `evara_core::lessons`' module header makes the same argument for
//! `DemandFinding`, and this is the same kind of thing at a larger scale.
//!
//! # What it does not do
//!
//! - **No constraint is evaluated.** There is no placement to evaluate one against.
//! - **No overlap is computed.** [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
//!   §3's seven ordered rules are M5's. This module *reads* the relation a problem carries and
//!   counts what it does not settle — see [`uncertainty`].
//! - **No database row is inspected.** Two declared codes are about stored rows a compiled
//!   problem has already dropped, and they belong to M4's loader. See
//!   [ADR 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md).
//!
//! # Ordering
//!
//! Findings come out in a fixed order designed so a reader fixes causes before symptoms:
//!
//! 1. **Scheduling context** — no grid, no bell schedule, an ambiguous one. Nothing downstream
//!    is meaningful until these are answered.
//! 2. **Structure and duration** — an occurrence longer than any run, a required room type
//!    nothing satisfies, a candidate set availability has emptied.
//! 3. **Proven aggregate impossibility** — the counting arguments.
//! 4. **Uncertainty warnings** — what will not be checked, and what is shared.
//! 5. **Informational discrepancies** — two legitimate figures disagreeing.
//!
//! Within a stage, entities are visited in **index order**, which is the problem's canonical
//! order and the order the interface lists them in. That is the stable secondary key: it is
//! deterministic, it is reproducible across runs, and it is more useful to a reader than
//! sorting by a UUID would be.

mod counting;
mod schedule;
mod uncertainty;

use super::context::BellScheduleResolution;
use super::diagnostics::{Diagnostic, DiagnosticReport, Severity};
use super::index::{BellScheduleIx, SectionIx, TimeslotIx};
use super::problem::Problem;
use super::resolve::{MaterialisedGrid, teaching_slots};

/// Validates a scheduling problem.
///
/// Returns every finding, in the order this module's header defines. Pure: the problem is not
/// modified and nothing outside the returned report is touched.
///
/// # What an empty report means
///
/// That no input could be **proved** to prevent scheduling. See this module's header.
#[must_use]
pub fn validate(problem: &Problem) -> DiagnosticReport {
    let pass = Pass::new(problem);
    let mut report = DiagnosticReport::empty();

    // Stage 1: the scheduling context. A missing grid short-circuits the per-section
    // findings, because with no slots every section would report the same cause.
    let grid_missing = schedule::context_findings(&pass, &mut report);
    if !grid_missing {
        schedule::resolution_findings(&pass, &mut report);
    }

    // Stage 2: structure and duration, for the sections that resolved.
    schedule::structure_findings(&pass, &mut report);

    // Stage 3: the counting arguments.
    counting::findings(&pass, &mut report);

    // Stage 4 and 5: what is not known, and what merely disagrees.
    uncertainty::warnings(&pass, &mut report);
    uncertainty::advice(&pass, &mut report);

    report
}

/// Everything the checks share, computed once.
///
/// Resolution and the per-schedule teaching slots are each needed by several checks, and
/// recomputing them per section would make validation quadratic in a school's size for no
/// reason.
struct Pass<'p> {
    problem: &'p Problem,
    /// `G`, derived from the materialised slots.
    grid: MaterialisedGrid,
    /// The computed resolution for each section, in section-index order.
    ///
    /// **Computed, never read from the section.** A problem's recorded resolution is an
    /// answer somebody else produced; a disagreement between it and the relation is a
    /// compiled-problem defect reported by
    /// [`Problem::integrity`](super::Problem::integrity), not a finding a user can act on.
    /// Driving the findings from the computed value means a wrong declaration cannot produce
    /// a wrong finding.
    resolved: Vec<BellScheduleResolution>,
    /// The teaching slots of each bell schedule, indexed by schedule index.
    ///
    /// Empty for a schedule with no slots, which is every schedule outside `G`.
    teaching: Vec<Vec<TimeslotIx>>,
}

impl<'p> Pass<'p> {
    fn new(problem: &'p Problem) -> Self {
        let grid = MaterialisedGrid::of(problem);
        let resolved = problem
            .sections
            .iter()
            .map(|section| grid.resolve(&problem.context, section.campus))
            .collect();
        let teaching = (0..problem.context.bell_schedules.len())
            .map(|ix| {
                let schedule = BellScheduleIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
                teaching_slots(problem, schedule)
            })
            .collect();
        Self {
            problem,
            grid,
            resolved,
            teaching,
        }
    }

    /// The schedule a section resolved to, or `None` where it did not resolve uniquely.
    fn schedule_of(&self, section: SectionIx) -> Option<BellScheduleIx> {
        self.resolved
            .get(section.get())
            .and_then(BellScheduleResolution::schedule)
    }

    /// The teaching slots of one schedule.
    fn teaching_slots(&self, schedule: BellScheduleIx) -> &[TimeslotIx] {
        self.teaching
            .get(schedule.get())
            .map_or(&[], |slots| slots.as_slice())
    }

    /// Sections in index order, with their index.
    fn sections(&self) -> impl Iterator<Item = (SectionIx, &super::problem::SectionFact)> + '_ {
        self.problem
            .sections
            .iter()
            .enumerate()
            .map(|(ix, s)| (SectionIx::new(u32::try_from(ix).unwrap_or(u32::MAX)), s))
    }
}

/// A count of what pre-solve validation found and what it could not check.
///
/// # Pre-solve, and deliberately not [`UnverifiedCounts`](super::UnverifiedCounts)
///
/// `UnverifiedCounts` counts *concurrently placed* activity pairs whose audiences could not be
/// compared, so it needs an assignment and belongs to M6 and M9. These figures are the
/// pre-solve analogues and are about the **inputs**: how many group pairs nothing settles, how
/// many declarations are unanswered, how many sections will not have their room size checked.
///
/// They are what a Generate screen shows before a solve, and what ADR 0035 §6's *"actionable
/// arithmetic"* is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ValidationSummary {
    /// Findings that refuse the solve.
    pub blocking: usize,
    /// Findings that are reported and do not block.
    pub warnings: usize,
    /// Findings that report a legitimate disagreement.
    pub informational: usize,
    /// Unordered pairs of this school's groups whose overlap nothing settles.
    pub indeterminate_group_pairs: usize,
    /// Parent groups with children and no disjointness answer.
    pub unanswered_disjointness: usize,
    /// Sections whose room size will not be checked, because attendance is unknown.
    pub unchecked_attendance: usize,
    /// Entities in scope that another school also holds.
    pub shared_entities: usize,
}

impl ValidationSummary {
    /// Summarises a report, reading the problem for the figures a count of findings cannot
    /// give.
    ///
    /// A finding count is not the same as a pair count: one
    /// [`GroupOverlapIndeterminate`](super::DiagnosticCode::GroupOverlapIndeterminate) finding
    /// stands for however many pairs it aggregates, which is the whole point of aggregating
    /// it.
    #[must_use]
    pub fn of(problem: &Problem, report: &DiagnosticReport) -> Self {
        Self {
            blocking: report.count_of(Severity::Blocking),
            warnings: report.count_of(Severity::Warning),
            informational: report.count_of(Severity::Info),
            indeterminate_group_pairs: problem.overlap.indeterminate_pairs(),
            unanswered_disjointness: uncertainty::unanswered_declarations(problem).len(),
            unchecked_attendance: problem
                .sections
                .iter()
                .filter(|s| !s.expected_attendance.is_known())
                .count(),
            shared_entities: uncertainty::shared_entity_count(problem),
        }
    }

    /// Whether a solve may proceed.
    #[must_use]
    pub const fn may_solve(&self) -> bool {
        self.blocking == 0
    }

    /// Whether anything at all was found.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.blocking == 0 && self.warnings == 0 && self.informational == 0
    }
}

/// Pushes a finding, keeping the call sites in the check modules short.
fn push(report: &mut DiagnosticReport, finding: Diagnostic) {
    report.push(finding);
}

#[cfg(test)]
mod tests;
