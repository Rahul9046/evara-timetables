//! The evaluation API: a problem, a candidate timetable, and what is wrong with it.
//!
//! # Two entry points, one implementation
//!
//! [`evaluate`] runs every constraint over the whole assignment. It is the reference
//! implementation `SOLVER.md` §4 calls *"simple, obviously correct"* and *"always correct,
//! never fast"*.
//!
//! [`evaluate_placement`] answers the question M6 and M11 actually ask — *would putting this
//! occurrence here break anything?* — by evaluating only the constraints whose
//! [`affected`](crate::ConstraintEvaluator::affected) set contains the occurrence that moved.
//! It is **incremental-friendly, not incremental**: it holds no cache and maintains no index,
//! so it cannot drift from the reference the way a hand-written delta can. `SOLVER.md` §4's
//! whole argument is that the cheap path keeps the fast path honest, and the cheap path has to
//! exist first.
//!
//! The brief asked not to build an elaborate cache until profiling demands one. Nothing here
//! does.
//!
//! # Partial and complete are different questions
//!
//! A partial assignment is a legitimate state, not an invalid input: M6 constructs
//! incrementally and every intermediate step has unplaced occurrences. So [`evaluate`] never
//! refuses one. What it does is *say which question was answered* —
//! [`EvaluationReport::completeness`] — so a caller cannot mistake a partial timetable for a
//! finished one.
//!
//! [`EvaluationReport::is_usable`] is `true` when no hard violation exists, and it is
//! deliberately not called `is_feasible` or `is_valid`: a usable *partial* timetable is still
//! partial, and `evara_core::lessons`' rule binds this crate too — never report that a demand
//! can be scheduled.

use evara_core::scheduling::{
    ActivityIx, Assignment, CompiledConstraint, ConstraintLevel, ConstraintType, Explanation,
    Problem, Score, Violation, ViolationSink,
};

use crate::occupancy::Occupancy;
use crate::registry::evaluator_for;

/// Whether every occurrence of the problem is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Completeness {
    /// Every occurrence is placed. The timetable is a candidate for adoption.
    Complete,
    /// Some occurrences are unplaced, which is ordinary during construction.
    Partial {
        /// How many are placed.
        placed: usize,
        /// How many the problem holds.
        required: usize,
    },
}

impl Completeness {
    /// Whether every occurrence is placed.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// What evaluating a candidate timetable found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationReport {
    /// Every violation, grouped by constraint type in registry order.
    pub violations: Vec<Violation>,
    /// The total score: hard, medium and soft penalties.
    pub score: Score,
    /// Whether the assignment placed everything.
    pub completeness: Completeness,
    /// References the problem does not hold.
    ///
    /// Non-empty means the assignment does not describe this problem, and the report describes
    /// nothing useful — see [`EvaluationReport::is_readable`].
    pub invalid_references: Vec<String>,
}

impl EvaluationReport {
    /// Whether the assignment could be read against this problem at all.
    ///
    /// `false` means an activity index or a timeslot the problem does not hold. A caller
    /// treats that as an execution failure, not as a timetable with violations: scoring an
    /// assignment that names rows from somewhere else would produce a number about nothing.
    #[must_use]
    pub fn is_readable(&self) -> bool {
        self.invalid_references.is_empty()
    }

    /// Whether no hard rule is broken.
    ///
    /// **Not a claim that the timetable is finished.** A partial assignment with no clashes is
    /// usable so far and incomplete; [`EvaluationReport::completeness`] is the other half of
    /// the answer and a caller needs both.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.is_readable() && self.score.is_feasible()
    }

    /// Whether this is a complete timetable with nothing hard broken.
    ///
    /// The strongest thing M5 can say, and it is still not *"a good timetable"* — no soft
    /// constraint is implemented, so nothing here has an opinion about quality.
    #[must_use]
    pub fn is_adoptable(&self) -> bool {
        self.is_usable() && self.completeness.is_complete()
    }

    /// How many violations at one level.
    #[must_use]
    pub fn count_at(&self, level: ConstraintLevel) -> usize {
        self.violations.iter().filter(|v| v.level == level).count()
    }

    /// How many violations of one type.
    #[must_use]
    pub fn count_of(&self, constraint_type: ConstraintType) -> usize {
        self.violations
            .iter()
            .filter(|v| v.constraint_type == constraint_type)
            .count()
    }

    /// Whether any violation of one type was found.
    #[must_use]
    pub fn has(&self, constraint_type: ConstraintType) -> bool {
        self.count_of(constraint_type) > 0
    }

    /// The violations of one level, in report order.
    pub fn at_level(&self, level: ConstraintLevel) -> impl Iterator<Item = &Violation> + '_ {
        self.violations.iter().filter(move |v| v.level == level)
    }

    /// Every violation explained, in report order.
    ///
    /// Each explanation comes from the evaluator that produced the violation, so a rule's
    /// wording lives with the rule and `explain()` cannot drift from `evaluate()`.
    #[must_use]
    pub fn explanations(&self, problem: &Problem) -> Vec<Explanation> {
        self.violations
            .iter()
            .map(|violation| evaluator_for(violation.constraint_type).explain(problem, violation))
            .collect()
    }
}

/// Evaluates every constraint over a whole candidate timetable.
///
/// # Deterministic by construction
///
/// Constraints run in [`ConstraintType::all`]'s order, each evaluator visits entities in index
/// order, and the pair sweep sorts by `(cycle day, start, activity)`. Nothing iterates a hash
/// map. Same problem, same assignment, same configuration, identical report — which
/// `SOLVER.md` §5 calls non-negotiable, because a seed over a varying order is not a seed.
///
/// `constraints` is what [`Registry::compile`](crate::Registry::compile) produced. A caller
/// that has configured nothing passes [`Registry::mandatory`](crate::Registry::mandatory),
/// and every rule still runs.
#[must_use]
pub fn evaluate(
    problem: &Problem,
    assignment: &Assignment,
    constraints: &[CompiledConstraint],
) -> EvaluationReport {
    let occupancy = Occupancy::of(problem, assignment);
    let invalid_references = occupancy.invalid_references();

    let mut sink = ViolationSink::new();
    // An unreadable assignment is not scored. Running the rules over it would produce
    // violations about activities that are not in this problem, which is worse than silence
    // because it looks like an answer.
    if invalid_references.is_empty() {
        for constraint_type in ConstraintType::all() {
            for constraint in constraints
                .iter()
                .filter(|c| c.constraint_type == *constraint_type)
            {
                evaluator_for(*constraint_type)
                    .evaluate(problem, &occupancy, constraint, &mut sink);
            }
        }
        sink.deduplicate();
    }

    let placed = occupancy.placed_count();
    let required = problem.activities.len();
    EvaluationReport {
        score: sink.score(),
        violations: sink.into_violations(),
        completeness: if placed == required {
            Completeness::Complete
        } else {
            Completeness::Partial { placed, required }
        },
        invalid_references,
    }
}

/// Evaluates only the rules a change to `moved` can affect.
///
/// # What this is for, and what it is not
///
/// M6's search and M11's `validate_move` both ask *"does this placement break anything?"* and
/// neither can afford a full sweep per candidate. This asks each evaluator whether `moved` is
/// in its [`affected`](crate::ConstraintEvaluator::affected) set and runs only those — which
/// skips, for instance, `room-type-required` for every other occurrence, because that rule is
/// entirely local.
///
/// It still evaluates each selected rule in full rather than computing a delta. That is the
/// deliberate stopping point: a hand-written delta is *"the single most bug-prone part of any
/// solver"*, and `SOLVER.md` §4's mitigation is to build the reference first and gate the
/// incremental one on a property test against it. M6 writes the delta; M5 writes the thing it
/// will be checked against.
///
/// The returned report describes the **whole** assignment for the rules it ran, so a caller
/// comparing before and after gets a comparable figure.
#[must_use]
pub fn evaluate_placement(
    problem: &Problem,
    assignment: &Assignment,
    constraints: &[CompiledConstraint],
    moved: ActivityIx,
) -> EvaluationReport {
    let relevant: Vec<CompiledConstraint> = constraints
        .iter()
        .filter(|constraint| {
            evaluator_for(constraint.constraint_type)
                .affected(problem, constraint, moved)
                .contains(moved)
        })
        .cloned()
        .collect();
    evaluate(problem, assignment, &relevant)
}
