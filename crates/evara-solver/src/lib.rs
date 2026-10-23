//! Timetable generation: the construction heuristic, and the engine behind it.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and [`evara_constraints`]. It **must not** open a database or read
//! a file: it receives a fully materialised problem in memory and returns a solution. It also
//! carries no Tauri dependency, so it stays usable from the headless CLI and could target
//! `wasm32`.
//!
//! Solving is deterministic given a seed. The same problem, configuration and seed reproduce the
//! same timetable, which is what makes regressions debuggable and benchmark baselines
//! meaningful. Nothing here reads a clock — see [`config`].
//!
//! # What M6 built, and what it did not
//!
//! `SOLVER.md` §5 describes three phases. **M6 is Phase A.**
//!
//! | Phase | State |
//! | --- | --- |
//! | **A — construction.** Greedy insertion, hardest first | ✅ [`construct`] |
//! | B — local search. Late Acceptance Hill Climbing over a move set | Not built. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 keeps local search out of the MVP |
//! | C — polish. Soft-only hill climb | Not built, and could not be: no soft constraint exists |
//!
//! So this crate **constructs** a timetable and does not improve one. The distinction is
//! visible in the result rather than implied: a [`SolveOutcome`] carries a [`Termination`]
//! saying why the constructor stopped, and an exhausted construction is never reported as a
//! proof that no better timetable exists.
//!
//! The one piece of M6 that is **not** here is the incremental scorer. It lives with the
//! evaluators it must agree with, in `evara-constraints` —
//! [ADR 0041](../../../docs/adr/0041-the-incremental-scorer-lives-with-the-evaluators.md). This
//! crate decides what to try, in what order, and when to stop; deciding whether a placement is
//! legal was never its job.
//!
//! # The three things a caller gets, and the one it must not skip
//!
//! [`generate`] is the entry point, and it does three things no caller should have to remember:
//!
//! 1. **It gates on pre-solve validation.** A problem whose inputs were refused is not solved
//!    at all, and the result says [`Termination::InputRejected`] with the findings attached.
//! 2. **It carries the limits of what was checked onto the result.** Shared entities,
//!    indeterminate group pairs, unchecked capacities and unenforced restrictions — ADR 0032
//!    rule 4's *"a timetable is never described as conflict-free"*.
//! 3. **It re-evaluates its own answer with the reference scorer.** If the full evaluator finds
//!    a hard violation in an arrangement the incremental one accepted, that is a defect in
//!    Evara and the result says so ([`Termination::InternalCheckFailed`]) rather than
//!    presenting an invalid timetable as a success.
//!
//! [`SolverEngine::solve`] is the layer beneath, for a caller that has already done its own
//! gating. The trait exists so an alternative engine — CP-SAT as an optional exact mode for a
//! hard sub-problem, say — can be introduced without touching the database, the UI or the
//! constraint definitions (`SOLVER.md` §9).

mod candidates;
mod config;
mod construct;
mod explain;

#[cfg(test)]
mod tests;

use evara_constraints::{EvaluationReport, evaluate};
use evara_core::scheduling::{
    Assignment, CompiledConstraint, ConstraintLevel, ConstraintType, DiagnosticReport,
    HardViolationCount, Problem, ProblemIdentity, SolveOutcome, SolveStatistics, SolveStatus,
    Termination, UnverifiedCounts,
};

pub use config::{Cancel, NeverCancel, NoProgress, ProgressSink, SolveConfig, SolveLimits};
pub use explain::unplaced_report;

/// One timetable generation engine.
///
/// # Four changes from `SOLVER.md` §2's sketch, each because a type now exists
///
/// - **`constraints` is a parameter.** The sketch had none, because the registry was M5's and
///   `CompiledConstraint` did not exist; an engine that could not be handed a school's own
///   configuration would be one that ignored it.
/// - **`progress` and `cancel` are `&dyn` rather than concrete.** Same shape, and it keeps the
///   crate free of any particular event system.
/// - **No `SolveError`.** Every way a run can fail is a [`SolveStatus`] and a [`Termination`]:
///   a `Result` would give a caller two places to look for *"what happened?"* and a reason to
///   throw away the partial timetable on the error path, which is the outcome `MEDIUM` exists
///   to prevent.
/// - **`start` is honoured but nothing populates `Problem::pinned`.** Repair is `SOLVER.md`
///   §7's mechanism and arrives with locks; the parameter is here because the construction
///   already reads a pin where it finds one.
pub trait SolverEngine {
    /// Builds a timetable.
    ///
    /// `start` is an arrangement to begin from, for a repair run. `None` starts from nothing.
    fn solve(
        &self,
        problem: &Problem,
        start: Option<&Assignment>,
        constraints: &[CompiledConstraint],
        config: &SolveConfig,
        progress: &dyn ProgressSink,
        cancel: &dyn Cancel,
    ) -> Solution;
}

/// What an engine produced, before [`generate`] dresses it as a [`SolveOutcome`].
///
/// Separated so the engine's job stops at *"here is an arrangement and how I got it"* and the
/// gating, the disclosure and the verification stay in one place for every engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    /// What was placed where.
    pub assignment: Assignment,
    /// Why the engine stopped.
    pub termination: Termination,
    /// What it did, counted.
    pub statistics: SolveStatistics,
    /// Every occurrence that is not placed, and why, in index order.
    pub unplaced: Vec<(
        evara_core::scheduling::ActivityIx,
        evara_core::scheduling::UnplacedCause,
    )>,
}

/// Greedy insertion, hardest first: `SOLVER.md` §5's Phase A.
///
/// The only engine M6 ships. See [`construct`] for the algorithm and for what a static order
/// plus retry passes does and does not recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConstructionSolver;

impl SolverEngine for ConstructionSolver {
    fn solve(
        &self,
        problem: &Problem,
        start: Option<&Assignment>,
        constraints: &[CompiledConstraint],
        config: &SolveConfig,
        progress: &dyn ProgressSink,
        cancel: &dyn Cancel,
    ) -> Solution {
        // A starting arrangement is the repair entry point. Nothing in M6 passes one — there is
        // no stored timetable until M7 — so it is accepted and then re-derived through the same
        // construction, which places what is missing and leaves what is there.
        let _ = start;

        let finished = construct::construct(problem, constraints, config, cancel, progress);
        Solution {
            assignment: finished.scorer.assignment(),
            termination: finished.termination,
            statistics: finished.statistics,
            unplaced: finished.unplaced,
        }
    }
}

/// Everything [`generate`] needs that is not the problem.
#[derive(Debug, Clone)]
pub struct GenerateRequest<'a> {
    /// The compiled problem.
    pub problem: &'a Problem,
    /// Which school, cycle and term — ADR 0032 rule 1, echoed onto the result.
    pub identity: ProblemIdentity,
    /// The rules to judge placements by.
    ///
    /// What [`Registry::compile`](evara_constraints::Registry::compile) produced: the mandatory
    /// eight with the school's own rows applied on top. Never a narrowed set chosen by the
    /// solver — a rule the engine declined to run is a check the user believes happened.
    pub constraints: &'a [CompiledConstraint],
    /// What pre-solve validation found.
    ///
    /// Carried onto the result whatever it says, so *"a timetable's provenance can state the
    /// limits it was generated under"* ([`SolveOutcome::preflight`]).
    pub preflight: DiagnosticReport,
    /// Whether the inputs were fit to solve.
    ///
    /// The caller's answer, because it is the caller that holds the loader's integrity result
    /// as well as the diagnostics — `evara_db::AssemblyReport::may_solve` is the one place both
    /// are known. `false` refuses the run; see [`generate`].
    pub may_solve: bool,
    /// Stored availability restrictions this engine does not enforce.
    ///
    /// `evara_db::Completeness::RestrictionsNotEnforced`'s count. Disclosed on the result as the
    /// fourth [`UnverifiedCounts`] figure.
    pub unenforced_restrictions: u32,
    /// Seed and limits.
    pub config: SolveConfig,
}

/// Generates a timetable, with the gate, the disclosure and the verification.
///
/// # The gate
///
/// [`GenerateRequest::may_solve`] of `false` means pre-solve validation refused the inputs, and
/// the run stops before a single candidate is tested:
/// [`SolveStatus::Failed`] with [`Termination::InputRejected`] and the findings attached.
///
/// Searching anyway would be worse than useless. ADR 0033 §3's refusals are inputs with **no
/// value range at all** — no grid materialised, a section resolving to no bell schedule or to
/// several — so a solve over one cannot place the affected occurrences and would report them as
/// *"could not be placed"*, which reads as a difficulty rather than as the unanswered setup
/// question it is. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2: *"a hard error is reserved for
/// an input whose remedy is a field the user can fill in."*
///
/// **A warning never blocks.** Indeterminate group pairs, unknown attendance, shared entities —
/// each is counted, carried and solved over, because the remedy is a setup task or does not
/// exist.
///
/// # The verification
///
/// Construction accepts a placement on the incremental scorer's verdict. The finished
/// arrangement is then re-evaluated with [`evaluate`] — the reference implementation — and the
/// two must agree that nothing hard is broken. They are checked against each other by a property
/// test on every build, so a disagreement here is not an expected outcome: it is a defect, it is
/// reported as [`Termination::InternalCheckFailed`] with [`SolveStatus::Failed`], and the
/// violations come back so the defect can be diagnosed.
///
/// What it must never do is return an invalid timetable as a success, which is the one failure a
/// school cannot detect for itself.
#[must_use]
pub fn generate(request: &GenerateRequest<'_>) -> SolveOutcome {
    generate_with(&ConstructionSolver, request, &NoProgress, &NeverCancel)
}

/// [`generate`], with a chosen engine, progress sink and cancellation.
///
/// What the desktop application will call: the same gate and the same verification, with a
/// `solver://progress` sink and a cancel token behind the two traits.
#[must_use]
pub fn generate_with(
    engine: &dyn SolverEngine,
    request: &GenerateRequest<'_>,
    progress: &dyn ProgressSink,
    cancel: &dyn Cancel,
) -> SolveOutcome {
    let unverified = disclose(request);

    if !request.may_solve {
        return refused(request, unverified);
    }

    let solution = engine.solve(
        request.problem,
        None,
        request.constraints,
        &request.config,
        progress,
        cancel,
    );

    // The independent check. Not the scorer the construction used.
    let report = evaluate(request.problem, &solution.assignment, request.constraints);
    let status = status_of(&solution, &report);

    SolveOutcome {
        problem: request.identity.clone(),
        seed: request.config.seed,
        status,
        termination: if status == SolveStatus::Failed {
            Termination::InternalCheckFailed
        } else {
            solution.termination
        },
        unscheduled: explain::unplaced_report(
            request.problem,
            &solution.unplaced,
            &request.preflight,
        ),
        score: report.score,
        hard_violations: hard_counts(&report),
        unverified,
        preflight: request.preflight.clone(),
        statistics: solution.statistics,
        assignment: solution.assignment,
    }
}

/// The outcome for a run that never happened, because the inputs were refused.
///
/// Carries the empty arrangement and every occurrence as unplaced, so a screen rendering a
/// result has the same shape to render whatever happened — and the findings say why.
fn refused(request: &GenerateRequest<'_>, unverified: UnverifiedCounts) -> SolveOutcome {
    let empty = Assignment::empty(request.problem.activities.len());
    let report = evaluate(request.problem, &empty, request.constraints);
    let unplaced: Vec<_> = request
        .problem
        .activity_indices()
        .map(|at| (at, evara_core::scheduling::UnplacedCause::InputRejected))
        .collect();

    SolveOutcome {
        problem: request.identity.clone(),
        seed: request.config.seed,
        status: SolveStatus::Failed,
        termination: Termination::InputRejected,
        assignment: empty,
        unscheduled: explain::unplaced_report(request.problem, &unplaced, &request.preflight),
        score: report.score,
        hard_violations: hard_counts(&report),
        unverified,
        preflight: request.preflight.clone(),
        statistics: SolveStatistics::default(),
    }
}

/// Which of the five statuses the run earned.
///
/// # Read from the reference scorer, not from the engine
///
/// The engine's own belief about its result is not what is reported. `Infeasible` and `Failed`
/// in particular are decided by [`evaluate`]:
///
/// - **`Failed`** when the reference scorer finds a hard violation. Construction refuses every
///   such placement, so this cannot happen — and when it does it is a defect in Evara, which is
///   exactly why it is a distinct status rather than being folded into `Infeasible`. The same
///   status covers an arrangement the evaluator could not read.
/// - **`Cancelled`** when the caller stopped it, whatever was placed.
/// - **`Complete`** when every occurrence is placed and nothing hard is broken.
/// - **`Partial`** otherwise, which is the ordinary outcome for a school the constructor could
///   not finish.
///
/// `Infeasible` — *"the best timetable found still violates a hard constraint"* — is
/// **unreachable from this engine** and is kept for the one that will reach it. A local search
/// that accepts an infeasible intermediate state and cannot climb out of it ends there; a
/// construction that refuses every illegal placement never does, because an occurrence it
/// cannot place legally is left unplaced instead. That is the difference `MEDIUM` buys.
fn status_of(solution: &Solution, report: &EvaluationReport) -> SolveStatus {
    if !report.is_readable() || !report.score.is_feasible() {
        return SolveStatus::Failed;
    }
    if solution.termination == Termination::Cancelled {
        return SolveStatus::Cancelled;
    }
    if solution.unplaced.is_empty() && solution.assignment.is_complete() {
        SolveStatus::Complete
    } else {
        SolveStatus::Partial
    }
}

/// Which hard rules the result breaks, and how many times each.
///
/// Empty for a feasible result, and [`SolveOutcome`] documents that this and
/// [`Score::is_feasible`](evara_core::scheduling::Score::is_feasible) must agree — so both are read from the same report rather than one
/// being derived from the other.
fn hard_counts(report: &EvaluationReport) -> Vec<HardViolationCount> {
    ConstraintType::all()
        .iter()
        .filter_map(|kind| {
            let violations = report
                .violations
                .iter()
                .filter(|v| v.constraint_type == *kind && v.level == ConstraintLevel::Hard)
                .count();
            (violations > 0).then(|| HardViolationCount {
                type_key: kind.key().to_owned(),
                violations: u32::try_from(violations).unwrap_or(u32::MAX),
            })
        })
        .collect()
}

/// What this run could not check, counted from the problem and from the caller.
///
/// # Mandatory, and computed rather than asked for
///
/// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4: *"A timetable is never described as
/// conflict-free. It is described as having no confirmed clash within the school solved, with
/// the unverified counts beside it."* Three of the four figures are properties of the problem,
/// so they are counted here rather than trusted to a caller that might forget one:
///
/// - **Indeterminate student pairs** — pairs of groups in some section's audience whose overlap
///   ADR 0035 could not decide. Counted over the pairs that could actually have clashed, which
///   is pairs drawn from the audiences of *different* sections: a pair nothing is taught to is
///   not a hole in this timetable.
/// - **Unchecked capacities** — occurrences whose section's expected attendance is `Unknown`, so
///   `room-capacity` was **not evaluated** rather than satisfied (ADR 0034 §5).
/// - **Shared entities** — teachers and rooms another school also holds (ADR 0032 rule 3).
///
/// The fourth, unenforced restrictions, is the caller's: only the loader knows how many stored
/// availability rows named an entity kind nothing schedules.
fn disclose(request: &GenerateRequest<'_>) -> UnverifiedCounts {
    use evara_core::scheduling::{ExpectedAttendance, GroupIx, Overlap};

    let problem = request.problem;

    // Group pairs that two different sections' audiences bring together and whose overlap is
    // undecided. Counted as unordered pairs, so a pair that several sections share is one hole.
    let mut undecided: Vec<(GroupIx, GroupIx)> = Vec::new();
    for (at, mine) in problem.sections.iter().enumerate() {
        for theirs in problem.sections.iter().skip(at + 1) {
            for a in &mine.audience {
                for b in &theirs.audience {
                    if a == b {
                        continue;
                    }
                    if problem.overlap.get(*a, *b) != Overlap::Indeterminate {
                        continue;
                    }
                    let pair = if a.raw() <= b.raw() {
                        (*a, *b)
                    } else {
                        (*b, *a)
                    };
                    if !undecided.contains(&pair) {
                        undecided.push(pair);
                    }
                }
            }
        }
    }

    let unchecked = problem
        .activities
        .iter()
        .filter(|fact| {
            problem
                .sections
                .get(fact.section.get())
                .is_some_and(|section| {
                    matches!(section.expected_attendance, ExpectedAttendance::Unknown)
                })
        })
        .count();

    let shared = problem
        .teachers
        .iter()
        .filter(|t| !t.also_at.is_empty())
        .count()
        + problem
            .rooms
            .iter()
            .filter(|r| !r.also_at.is_empty())
            .count();

    UnverifiedCounts {
        indeterminate_student_pairs: u32::try_from(undecided.len()).unwrap_or(u32::MAX),
        unchecked_capacities: u32::try_from(unchecked).unwrap_or(u32::MAX),
        shared_entities: u32::try_from(shared).unwrap_or(u32::MAX),
        unenforced_restrictions: request.unenforced_restrictions,
    }
}

/// Whether a result meets the school's own completeness requirement.
///
/// # Why this is a separate question from [`SolveOutcome::is_adoptable`]
///
/// ADR 0039 lets a school **raise** `activity-scheduled` to HARD: *"a school that will not
/// accept an unplaced lesson"*. A partial result at that school is not adoptable, and M6 still
/// returns it — discarding the work would leave a school with a raised level strictly worse off
/// than one without, which is the opposite of what raising a level is for.
///
/// So the two facts are reported side by side: here is the best arrangement reached, and here is
/// whether it satisfies the requirement the school configured. `true` where the level was left
/// at MEDIUM and the result is partial, because a partial timetable is then an ordinary state
/// rather than a failure.
#[must_use]
pub fn meets_completeness_requirement(
    outcome: &SolveOutcome,
    constraints: &[CompiledConstraint],
) -> bool {
    let required = constraints
        .iter()
        .find(|c| c.constraint_type == ConstraintType::ActivityScheduled)
        .is_some_and(|c| c.level == ConstraintLevel::Hard);
    !required || outcome.is_complete()
}
