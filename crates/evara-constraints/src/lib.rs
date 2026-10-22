//! The constraint registry: evaluators, parameter schemas and explanations.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and nothing else. It **must not** touch SQL or the filesystem: a
//! constraint is evaluated against an in-memory problem, never a database.
//!
//! This crate is consumed by **both** `evara-solver` and the manual-move validator in the
//! desktop application, which is the whole reason it exists as a separate crate. There is
//! exactly one implementation of every scheduling rule, shared by:
//!
//! 1. automatic timetable generation
//! 2. timetable scoring
//! 3. manual move validation
//! 4. violation explanation
//!
//! A rule that exists in the UI, or twice anywhere, is an architecture bug.
//!
//! # What this crate does not own
//!
//! The **vocabulary** is `evara_core::scheduling::constraints`: [`ConstraintType`],
//! [`Violation`], [`Explanation`], [`CompiledConstraint`]. `evara-db` produces the last and
//! will persist the second in M7, and it may not depend on this crate, so those types sit
//! underneath both.
//!
//! Three rules that could plausibly live here deliberately do not, because each has an answer
//! **before any placement exists** and `ARCHITECTURE.md` §3 rule 2 reserves this crate for
//! rules scored against a candidate timetable:
//!
//! | Rule | Where it lives | Record |
//! | --- | --- | --- |
//! | Which bell schedule a section draws slots from | `evara_core::scheduling::resolve` | ADR 0036 |
//! | Which rooms a section may use | `RoomFact::is_candidate_for` | ADR 0014 |
//! | Whether two groups share a student | `evara_core::scheduling::overlap` | ADR 0035 |
//!
//! Every evaluator here **calls** those rather than restating them. `student-clash` in
//! particular reads the compiled [`Problem::overlap`] relation and never re-derives it:
//! rule 3 of ADR 0035 §3 precedes rule 6, and two implementations that disagreed would have
//! the product honouring a clash in one place and ignoring it in the other.
//!
//! # What is implemented
//!
//! Eight types — seven `HARD` and `activity-scheduled` at `MEDIUM`, which is
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's split. `activity-fixed` is the eighth hard type
//! of that record and is **not** implemented: `activity.is_locked` does not exist, so it would
//! be a registered rule that cannot fire.
//!
//! Nothing here searches, optimises or writes. `evaluate` is the reference implementation
//! `SOLVER.md` §4 describes as *"always correct, never fast"*, and M6's incremental scorer will
//! be kept honest against it.

mod availability;
mod clash;
mod completeness;
mod facilities;
mod occupancy;
mod params;
mod registry;
mod timing;

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, Explanation, Problem, Violation,
    ViolationSink,
};

pub use evaluation::{EvaluationReport, evaluate, evaluate_placement};
pub use occupancy::Occupancy;
pub use params::schema_of;
pub use registry::{Registry, evaluator_for};

mod evaluation;

#[cfg(test)]
mod tests;

/// One scheduling rule, evaluated against a candidate timetable.
///
/// The five members `SOLVER.md` §4 declares and ADR 0005's table names, with their signatures
/// adapted to the types that now exist:
///
/// - `changed` is an [`ActivityIx`] rather than a bare `u32`, for the reason M2 made the
///   indices newtypes: mixing two would be a plausible wrong answer rather than a crash.
/// - `evaluate` writes into a [`ViolationSink`] rather than returning a `Vec`, because an
///   evaluator sweeping a grid produces violations as it goes and the caller decides what to
///   do with them.
/// - `constraint_type` replaces `type_key`, which the enum now carries. `type_key()` is still
///   reachable as `self.constraint_type().key()`.
///
/// # Every implementation must be deterministic
///
/// Same problem, same assignment, same configuration, same violations in the same order. That
/// is not tidiness: `SOLVER.md` §5 makes reproducibility non-negotiable, and a report whose
/// order varied would make a solver's seed meaningless. No implementation here iterates a hash
/// map; everything is sorted by index or swept in time order.
pub trait ConstraintEvaluator: Send + Sync {
    /// Which rule this is.
    fn constraint_type(&self) -> ConstraintType;

    /// The JSON Schema for this type's parameters.
    ///
    /// Validates a write **and** generates the configuration form — ADR 0005 calls the schema
    /// *"load-bearing"* for exactly that double duty, which is why it lives beside the
    /// evaluator and is tested against it rather than written once in a form definition.
    fn param_schema(&self) -> &'static str;

    /// Full evaluation. The reference implementation — always correct, never fast.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    );

    /// Which occurrences this rule's verdict can change when `changed` moves.
    ///
    /// Scopes M6's incremental re-evaluation. Nothing in M5 maintains an incremental index —
    /// `SOLVER.md` §4's argument is that the reference implementation is what keeps the
    /// incremental one honest, so the reference one comes first — but the answer is computed
    /// here, because only the rule knows what it depends on.
    fn affected(
        &self,
        problem: &Problem,
        constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet;

    /// Why this specific violation happened, in a sentence that names its subjects.
    ///
    /// Mandatory, not optional. ADR 0005: *"A constraint that cannot say why it was violated
    /// cannot ship, because the product's differentiator is explanation."*
    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation;
}

/// Builds the explanation every evaluator shares the shape of.
///
/// A violation already carries its subject, its related entities and its detail, so an
/// explanation is mostly a re-shaping plus the sentence the rule writes. Centralised so the
/// `entities` and `conflicting_activities` lists are assembled the same way by all eight and a
/// new evaluator cannot forget one.
pub(crate) fn explanation(
    problem: &Problem,
    violation: &Violation,
    message: String,
) -> Explanation {
    let mut entities = vec![violation.subject.clone()];
    entities.extend(violation.related.iter().cloned());
    Explanation {
        constraint_type: violation.constraint_type,
        level: violation.level,
        message,
        entities,
        conflicting_activities: violation
            .activities
            .iter()
            .filter_map(|at| problem.activities.get(at.get()).map(|a| a.id))
            .collect(),
    }
}
