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
//! Nine types — eight `HARD` and `activity-scheduled` at `MEDIUM`, which is
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's split in full. `activity-fixed` is the eighth
//! hard type of that record and M9A implements it: locks exist as `timetable_entry.is_locked`,
//! the caller that opened a timetable reads them into `Problem::pinned`, and the rule therefore
//! has something to fire on. See [`pinning`] for why an *unplaced* pinned occurrence is
//! `activity-scheduled`'s business and not its.
//!
//! Nothing here searches, optimises or writes. `evaluate` is the reference implementation
//! `SOLVER.md` §4 describes as *"always correct, never fast"*, and [`IncrementalScorer`] is the
//! fast one it keeps honest.
//!
//! # Why the incremental scorer is here and not in `evara-solver`
//!
//! It is the one piece of M6 that lives in this crate, and
//! [ADR 0041](../../../docs/adr/0041-the-incremental-scorer-lives-with-the-evaluators.md) is
//! why. A scorer that must agree with these eight rules to the violation needs the span
//! resolver they share ([`Occupancy`]) and each rule's own narrowed query; reaching those from
//! another crate would mean either a second copy of the predicates — `CLAUDE.md` §2's most
//! important rule, broken in the exact place a solver usually breaks it — or a public API wide
//! enough to make the copy easy for somebody else to write later.
//!
//! `evara-solver` *searches*: it decides what to try, in what order, and when to stop.
//! Deciding whether a placement is legal was never its job.

mod availability;
mod clash;
mod completeness;
mod facilities;
mod incremental;
mod occupancy;
mod params;
mod pinning;
mod registry;
mod timing;

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, Explanation, Problem, Violation,
    ViolationSink,
};

pub use evaluation::{Completeness, EvaluationReport, evaluate, evaluate_placement};
pub use incremental::{IncrementalScorer, PlacementVerdict};
pub use occupancy::Occupancy;
pub use params::schema_of;
pub use registry::{Registry, evaluator_for};

mod evaluation;

#[cfg(test)]
mod differential;
/// What a candidate panel costs at the realistic upper end — R5's editing half.
#[cfg(test)]
mod editing_budget;
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

    /// The violations of this rule that **name** one occurrence.
    ///
    /// # Why this is the whole of incremental scoring
    ///
    /// Every violation any implemented rule produces names at most two occurrences — two for a
    /// clash, one for everything else — which is a property of the eight, not an accident: a
    /// [`Violation`] carries an `activities` list and nothing in the MVP fills it with three.
    /// So a violation that does not name `activity` cannot be changed by moving `activity`,
    /// and
    ///
    /// ```text
    /// score(after) - score(before)
    ///     = involving(after, a) - involving(before, a)
    /// ```
    ///
    /// is **exact** rather than an approximation. That identity is what
    /// [`IncrementalScorer`](crate::IncrementalScorer) maintains a running score with, and it
    /// is why there is no hand-written per-rule delta anywhere in this crate —
    /// `SOLVER.md` §4 calls such a delta *"the single most bug-prone part of any solver"*.
    ///
    /// # Two drivers, one body
    ///
    /// Each implementation shares its violation-building code with [`ConstraintEvaluator::evaluate`]
    /// rather than restating the check. `evaluate` sweeps everything and this narrows first;
    /// neither owns a predicate the other does not. The property that keeps them honest is
    /// asserted over every fixture and every rule:
    ///
    /// > the union of `evaluate_involving` over every occurrence, deduplicated, is exactly what
    /// > `evaluate` produces.
    ///
    /// A rule that narrowed too far fails it by omission, and one that narrowed wrongly fails it
    /// by producing a violation `evaluate` does not. There is no third outcome, which is what
    /// makes the test a gate rather than a smoke check.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
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
