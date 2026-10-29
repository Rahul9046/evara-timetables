//! Repair: a solve that starts from a frozen arrangement and may move only what it was given.
//!
//! # Why this is a sibling of `generate` and not a mode of it
//!
//! [`crate::generate_with`] gates on [`crate::status_of`], whose first line is
//!
//! ```text
//! if !report.is_readable() || !report.score.is_feasible() { return SolveStatus::Failed; }
//! ```
//!
//! and `Score::is_feasible` is `hard == 0` over the **whole arrangement**. A repaired
//! arrangement holds every frozen out-of-scope placement, pre-existing hard violations included,
//! so routing a repair through that gate would return [`SolveStatus::Failed`] with
//! [`Termination::InternalCheckFailed`] for every repair of a timetable that has a hard
//! violation outside the chosen scope — which is precisely the kind of timetable a repair exists
//! for. The repair would be correct, the arrangement better than the baseline, and the product
//! would report an internal defect and discard it.
//!
//! So the generation path is left bit-for-bit alone and repair verifies its own answer with a
//! **scope-aware** check that is *stronger* rather than more permissive. See
//! `M9B-REPAIR-PROPOSAL.md` §2.9.
//!
//! # The verification, in four parts
//!
//! The reference evaluator remains the only judge of legality — nothing here re-implements a
//! rule. What changes is the question asked of its report.
//!
//! | | Check | Catches |
//! | --- | --- | --- |
//! | **V0** | [`frozen_differences`] over the complete assignments, placement by placement | A frozen occurrence moved, dropped or invented. The **primary** proof, because a violation-set comparison cannot see a frozen swap or a move to an equally-clashing slot |
//! | **V1** | `!report.is_readable()` | An arrangement that does not describe the problem |
//! | **V2** | a HARD violation whose `activities` intersects the movable set | The genuine incremental-versus-reference disagreement: `probe` refused every candidate carrying a hard violation naming the occurrence |
//! | **V3** | the frozen-only HARD set differs from the baseline's, by `ViolationKey` | A frozen placement acquiring or losing a conflict. Independent of V0, and kept because the two fail differently |
//!
//! V0 is the safeguard the approval made mandatory, and it is deliberately not derived from V3:
//! two arrangements can carry identical violation sets and still differ. Both run, and either
//! failing is an internal consistency failure that is never applyable.
//!
//! A non-empty **frozen pre-existing** hard set is not a failure, is not a reason to withhold
//! apply, and is reported as what it is.

use evara_constraints::{EvaluationReport, evaluate};
use evara_core::scheduling::{
    ActivityIx, Assignment, CompiledConstraint, ConstraintLevel, DiagnosticReport, FrozenMismatch,
    MovableSet, Placement, Problem, ProblemIdentity, SolveOutcome, SolveStatistics, SolveStatus,
    Termination, UnplacedCause, Violation, ViolationKey, frozen_differences, locked_differences,
};

use crate::config::{Cancel, ProgressSink, SolveConfig};
use crate::construct::{self, Repair as ConstructRepair};
use crate::{disclose, explain, hard_counts};

/// Everything a repair run needs that is not the problem.
#[derive(Debug, Clone)]
pub struct RepairRequest<'a> {
    /// The compiled problem, as for a generation.
    pub problem: &'a Problem,
    /// Which school, cycle and term.
    pub identity: ProblemIdentity,
    /// The rules to judge placements by. Never a narrowed set.
    pub constraints: &'a [CompiledConstraint],
    /// What pre-solve validation found, carried onto the result.
    pub preflight: DiagnosticReport,
    /// Whether the inputs were fit to solve.
    pub may_solve: bool,
    /// Stored availability restrictions this engine does not enforce.
    pub unenforced_restrictions: u32,
    /// Seed and limits.
    pub config: SolveConfig,
    /// The arrangement as stored. Never mutated, and never the working arrangement.
    pub baseline: &'a Assignment,
    /// What may move. Everything else is frozen.
    pub movable: &'a MovableSet,
}

/// Why a repair was refused before it ran, or rejected after it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairDefect {
    /// V0 — a frozen occurrence's placement is not what it was.
    ///
    /// The mandatory safeguard. Carries every mismatch, not just the first, so the defect can be
    /// diagnosed rather than merely detected.
    FrozenPlacementsChanged(Vec<FrozenMismatch>),
    /// V0b — a locked occurrence is not at its pin.
    LockedPlacementsChanged(Vec<FrozenMismatch>),
    /// V1 — the arrangement does not describe the problem.
    NotReadable(Vec<String>),
    /// V2 — a hard violation involving an occurrence the repair placed.
    HardViolationInvolvingMovable(Vec<Violation>),
    /// V3 — the frozen-only hard violation set moved.
    FrozenViolationsChanged {
        /// Keys present after the repair and not before.
        appeared: Vec<ViolationKey>,
        /// Keys present before and not after.
        disappeared: Vec<ViolationKey>,
    },
}

impl RepairDefect {
    /// A stable key, for a message and for a test that pins the vocabulary.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::FrozenPlacementsChanged(_) => "frozen-placements-changed",
            Self::LockedPlacementsChanged(_) => "locked-placements-changed",
            Self::NotReadable(_) => "arrangement-not-readable",
            Self::HardViolationInvolvingMovable(_) => "hard-violation-involving-movable",
            Self::FrozenViolationsChanged { .. } => "frozen-violations-changed",
        }
    }
}

/// How a repair's hard violations divide, once the movable set is known.
///
/// A projection of two reference reports, by set membership. No rule is restated and no
/// violation re-judged — this attributes, it does not decide.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViolationPartition {
    /// Present before and after, naming no movable occurrence. Expected, and not the repair's.
    pub pre_existing_frozen: Vec<Violation>,
    /// Present after only, naming a movable occurrence. Must be empty at HARD level — V2.
    pub new_involving_movable: Vec<Violation>,
    /// Present before only. What the repair fixed.
    pub eliminated: Vec<Violation>,
}

/// What a repair produced.
#[derive(Debug, Clone)]
pub struct RepairOutcome {
    /// The outcome, in M6's vocabulary. `score` and `hard_violations` are the **whole**
    /// arrangement's, from the reference evaluator — never the in-scope subset.
    pub outcome: SolveOutcome,
    /// How the hard violations divide, for the preview's attribution.
    pub hard_partition: ViolationPartition,
    /// Every level's violations after the repair, for the stored findings and the preview.
    pub violations: Vec<Violation>,
    /// The baseline's score, so the preview can show before and after at three levels.
    pub baseline_score: evara_core::scheduling::Score,
    /// The baseline's violations, for the same reason.
    pub baseline_violations: Vec<Violation>,
    /// How many occurrences were frozen.
    pub frozen_count: usize,
    /// How many could move.
    pub movable_count: usize,
    /// How many movable occurrences ended placed.
    ///
    /// With `movable_count`, this is the repair's own completeness -- distinct from the
    /// arrangement's, which is what [`SolveOutcome::status`] reports.
    pub movable_placed: usize,
    /// How many frozen occurrences were placed, which a repair may never change.
    pub frozen_placed: usize,
    /// Whether this outcome may be persisted. `false` whenever `defect` is `Some`.
    pub applyable: bool,
    /// The internal consistency failure, where there was one.
    pub defect: Option<RepairDefect>,
}

/// Runs a repair with the default engine.
#[must_use]
pub fn repair(request: &RepairRequest<'_>) -> RepairOutcome {
    repair_with(request, &crate::NoProgress, &crate::NeverCancel)
}

/// Runs a repair, reporting progress and honouring cancellation.
///
/// Performs the seven steps of `M9B-REPAIR-PROPOSAL.md` §2.8 and the four verification checks of
/// §2.9. Writes nothing; `evara_db::Timetables::save_repaired` is what writes, and it repeats V0
/// inside its transaction.
#[must_use]
pub fn repair_with(
    request: &RepairRequest<'_>,
    progress: &dyn ProgressSink,
    cancel: &dyn Cancel,
) -> RepairOutcome {
    let unverified = disclose(&crate::GenerateRequest {
        problem: request.problem,
        identity: request.identity.clone(),
        constraints: request.constraints,
        preflight: request.preflight.clone(),
        may_solve: request.may_solve,
        unenforced_restrictions: request.unenforced_restrictions,
        config: request.config,
    });

    let baseline_report = evaluate(request.problem, request.baseline, request.constraints);

    if !request.may_solve {
        return refused(
            request,
            unverified,
            &baseline_report,
            Termination::InputRejected,
        );
    }

    let scope = ConstructRepair {
        baseline: request.baseline,
        movable: request.movable,
    };
    let Some(finished) = construct::construct(
        request.problem,
        request.constraints,
        &request.config,
        cancel,
        progress,
        Some(&scope),
    ) else {
        // `IncrementalScorer::of` refused the starting arrangement.
        return refused(
            request,
            unverified,
            &baseline_report,
            Termination::InputRejected,
        );
    };

    let repaired = finished.scorer.assignment();
    let report = evaluate(request.problem, &repaired, request.constraints);

    let defect = verify(request, &baseline_report, &report, &repaired);
    let hard_partition = partition(request, &baseline_report, &report);

    let termination = if defect.is_some() {
        Termination::InternalCheckFailed
    } else {
        finished.termination
    };
    let status = status_of(finished.termination, defect.is_some(), &repaired, request);

    let outcome = SolveOutcome {
        problem: request.identity.clone(),
        seed: request.config.seed,
        status,
        termination,
        unscheduled: explain::unplaced_report(
            request.problem,
            &finished.unplaced,
            &request.preflight,
        ),
        // The WHOLE arrangement's figures. A preview showing only in-scope violations would be
        // describing a timetable the user is not about to save.
        score: report.score,
        hard_violations: hard_counts(&report),
        unverified,
        preflight: request.preflight.clone(),
        statistics: finished.statistics,
        assignment: repaired,
    };

    RepairOutcome {
        applyable: defect.is_none() && is_applyable(&outcome),
        outcome,
        hard_partition,
        violations: report.violations,
        baseline_score: baseline_report.score,
        baseline_violations: baseline_report.violations,
        frozen_count: request.movable.frozen_count(),
        movable_count: finished.attempted_total,
        movable_placed: finished.attempted_placed,
        frozen_placed: finished.frozen_placed_at_start,
        defect,
    }
}

/// The status of a repair, which is honest about the **arrangement** rather than about the run.
///
/// A timetable with out-of-scope unplaced lessons is `Partial` whatever the repair achieved, and
/// that is the truth about the timetable. The diff is what attributes it.
fn status_of(
    termination: Termination,
    defective: bool,
    repaired: &Assignment,
    request: &RepairRequest<'_>,
) -> SolveStatus {
    if defective {
        return SolveStatus::Failed;
    }
    if termination == Termination::Cancelled {
        return SolveStatus::Cancelled;
    }
    let every_movable_placed = request
        .movable
        .indices()
        .iter()
        .all(|at| repaired.placement(*at).is_some());
    if every_movable_placed && repaired.is_complete() {
        SolveStatus::Complete
    } else {
        SolveStatus::Partial
    }
}

/// Whether an outcome may be persisted.
///
/// A closed list, so nothing unsafe becomes applyable by omission. `Partial` **is** applyable: a
/// repair of a timetable with out-of-scope unplaced lessons is always partial, so refusing it
/// would refuse almost every real repair.
fn is_applyable(outcome: &SolveOutcome) -> bool {
    !matches!(
        outcome.termination,
        Termination::Cancelled | Termination::InputRejected | Termination::InternalCheckFailed
    ) && outcome.status != SolveStatus::Failed
        && outcome.status != SolveStatus::Cancelled
        && outcome.status != SolveStatus::Infeasible
}

/// V0, V0b, V1, V2 and V3. `None` means the outcome is internally consistent.
fn verify(
    request: &RepairRequest<'_>,
    baseline_report: &EvaluationReport,
    report: &EvaluationReport,
    repaired: &Assignment,
) -> Option<RepairDefect> {
    // V0 — the mandatory safeguard, and the first check because it is the strongest. The
    // complete assignments, placement by placement, slot and room, in both directions.
    let moved = frozen_differences(request.baseline, repaired, request.movable);
    if !moved.is_empty() {
        return Some(RepairDefect::FrozenPlacementsChanged(moved));
    }

    // V0b — the same question narrowed to locks, asked separately because the guarantee is
    // separate and a future change making a lock movable must fail loudly here.
    let unlocked = locked_differences(request.baseline, repaired, &request.problem.pinned);
    if !unlocked.is_empty() {
        return Some(RepairDefect::LockedPlacementsChanged(unlocked));
    }

    // V1 — unchanged from M6.
    if !report.is_readable() {
        return Some(RepairDefect::NotReadable(report.invalid_references.clone()));
    }

    // V2 — the real incremental-versus-reference disagreement.
    let involving: Vec<Violation> = hard(report)
        .filter(|found| touches_movable(found, request.movable))
        .cloned()
        .collect();
    if !involving.is_empty() {
        return Some(RepairDefect::HardViolationInvolvingMovable(involving));
    }

    // V3 — the frozen-only hard set, compared by identity rather than by count.
    let before: Vec<ViolationKey> = frozen_hard_keys(baseline_report, request.movable);
    let after: Vec<ViolationKey> = frozen_hard_keys(report, request.movable);
    let appeared: Vec<ViolationKey> = after
        .iter()
        .filter(|key| !before.contains(key))
        .cloned()
        .collect();
    let disappeared: Vec<ViolationKey> = before
        .iter()
        .filter(|key| !after.contains(key))
        .cloned()
        .collect();
    if !appeared.is_empty() || !disappeared.is_empty() {
        return Some(RepairDefect::FrozenViolationsChanged {
            appeared,
            disappeared,
        });
    }

    None
}

/// The four buckets, as set operations on two reference reports.
fn partition(
    request: &RepairRequest<'_>,
    baseline_report: &EvaluationReport,
    report: &EvaluationReport,
) -> ViolationPartition {
    let before: Vec<ViolationKey> = hard(baseline_report).map(Violation::key).collect();
    let after: Vec<ViolationKey> = hard(report).map(Violation::key).collect();

    ViolationPartition {
        pre_existing_frozen: hard(report)
            .filter(|found| !touches_movable(found, request.movable))
            .filter(|found| before.contains(&found.key()))
            .cloned()
            .collect(),
        new_involving_movable: hard(report)
            .filter(|found| touches_movable(found, request.movable))
            .filter(|found| !before.contains(&found.key()))
            .cloned()
            .collect(),
        eliminated: hard(baseline_report)
            .filter(|found| !after.contains(&found.key()))
            .cloned()
            .collect(),
    }
}

fn hard(report: &EvaluationReport) -> impl Iterator<Item = &Violation> + '_ {
    report
        .violations
        .iter()
        .filter(|found| found.level == ConstraintLevel::Hard)
}

/// Whether a violation names an occurrence the repair was allowed to move.
///
/// `Violation::activities` holds at most two entries — `CLAUDE.md` §4 makes that a precondition
/// of the incremental design and `differential.rs` asserts it — so this is a one- or two-element
/// membership test rather than a set intersection.
fn touches_movable(found: &Violation, movable: &MovableSet) -> bool {
    found
        .activities
        .iter()
        .any(|at| movable.contains(ActivityIx::new(at.raw())))
}

fn frozen_hard_keys(report: &EvaluationReport, movable: &MovableSet) -> Vec<ViolationKey> {
    hard(report)
        .filter(|found| !touches_movable(found, movable))
        .map(Violation::key)
        .collect()
}

/// The outcome for a repair that never ran.
fn refused(
    request: &RepairRequest<'_>,
    unverified: evara_core::scheduling::UnverifiedCounts,
    baseline_report: &EvaluationReport,
    termination: Termination,
) -> RepairOutcome {
    // The baseline, untouched. A refused repair changes nothing, so the arrangement it reports
    // is the one that is still stored — not an empty one, which would read as "everything was
    // unplaced" for a timetable that is intact.
    let assignment = request.baseline.clone();
    let unplaced: Vec<(ActivityIx, UnplacedCause)> = request
        .movable
        .indices()
        .iter()
        .filter(|at| assignment.placement(**at).is_none())
        .map(|at| (*at, UnplacedCause::InputRejected))
        .collect();

    RepairOutcome {
        outcome: SolveOutcome {
            problem: request.identity.clone(),
            seed: request.config.seed,
            status: SolveStatus::Failed,
            termination,
            unscheduled: explain::unplaced_report(request.problem, &unplaced, &request.preflight),
            score: baseline_report.score,
            hard_violations: hard_counts(baseline_report),
            unverified,
            preflight: request.preflight.clone(),
            statistics: SolveStatistics::default(),
            assignment,
        },
        hard_partition: ViolationPartition::default(),
        violations: baseline_report.violations.clone(),
        baseline_score: baseline_report.score,
        baseline_violations: baseline_report.violations.clone(),
        frozen_count: request.movable.frozen_count(),
        movable_count: request.movable.len(),
        movable_placed: 0,
        frozen_placed: request
            .movable
            .frozen()
            .filter(|at| request.baseline.placement(*at).is_some())
            .count(),
        applyable: false,
        defect: None,
    }
}

/// Where a movable occurrence ended up, against where it started.
///
/// Exposed for the report crate's diff, so the five categories are computed from one place.
#[must_use]
pub fn placement_change(
    baseline: &Assignment,
    repaired: &Assignment,
    activity: ActivityIx,
) -> (Option<Placement>, Option<Placement>) {
    (baseline.placement(activity), repaired.placement(activity))
}
