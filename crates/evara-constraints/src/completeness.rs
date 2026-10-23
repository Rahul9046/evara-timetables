//! `activity-scheduled`: every occurrence placed, exactly once.
//!
//! # MEDIUM, and that is the point of it
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 puts this at **MEDIUM** while the other seven are
//! HARD, and `CONSTRAINTS.md` §1 says what the level is for: *"It lets the solver return a
//! usable partial timetable and tell the user exactly what it could not place, instead of
//! returning nothing."*
//!
//! So an unplaced occurrence makes a timetable **incomplete**, not infeasible. M6 constructs
//! incrementally, and every intermediate state has unplaced activities; promoting this to HARD
//! would make the first of them infeasible and every one after it, which is the same as having
//! no construction phase at all. The original M5 brief classified it as hard; the accepted
//! record does not, and [`ConstraintType::default_level`] is where that is settled.
//!
//! A school that will not accept an unplaced lesson can still raise its own instance to HARD —
//! ADR 0005 stores the level per instance for exactly that disagreement. M6 honours a raise and
//! still returns its partial work: `evara_solver::generate` reports the best arrangement it
//! reached **and** that the arrangement does not meet the school's completeness requirement,
//! because discarding the work would leave a school with a raised level strictly worse off than
//! one without.
//!
//! # Partial and complete are different questions, asked separately
//!
//! This evaluator reports what is unplaced whatever mode it is called in, because *"12 lessons
//! are not placed"* is true and useful in both. What differs is the caller's conclusion, and
//! [`Occupancy::is_complete`](crate::Occupancy) is what answers it — see
//! [`crate::EvaluationReport`]. Nothing here decides that a partial assignment is acceptable;
//! it reports the gap and lets the report say which question was being asked.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, EntityKind, EntityRef,
    Explanation, Problem, Violation, ViolationSink,
};

use crate::occupancy::Occupancy;
use crate::{ConstraintEvaluator, explanation};

/// Every activity in scope must be placed, exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivityScheduled;

impl ConstraintEvaluator for ActivityScheduled {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::ActivityScheduled
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::ActivityScheduled)
    }

    /// # One violation per unplaced occurrence, not one for the school
    ///
    /// Each is a separate thing to place, and a solver repairing a timetable needs to know
    /// which. That is the opposite of M3's aggregate `group-overlap-indeterminate`, and the
    /// difference is whether the remedy is per row: here it is.
    ///
    /// **Duplicate placement cannot be represented**, and that is worth stating rather than
    /// checking for. An [`Assignment`](evara_core::scheduling::Assignment) is one
    /// `Option<Placement>` per activity index, so an occurrence has one placement or none —
    /// there is no way to give it two. The duplicate the brief asks about is prevented by the
    /// type rather than detected by this rule, and an *assignment* that named one activity
    /// twice would be rejected earlier, by [`Occupancy`]'s invalid-reference check.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for activity in problem.activity_indices() {
            check_one(problem, occupancy, constraint, activity, out);
        }
    }

    /// The same check, for one occurrence.
    ///
    /// # The one rule whose `affected` set is everything and whose violations are still local
    ///
    /// Placing anything changes *how many* occurrences are unplaced, which is why
    /// [`ActivityScheduled::affected`] returns [`AffectedSet::everything`]. But each violation
    /// this rule produces names exactly **one** occurrence, so the violation about `A` is
    /// unchanged by a move of `B` and the narrowed form is still exact — the two statements are
    /// about different things, and the incremental identity in
    /// [`ConstraintEvaluator::evaluate_involving`] depends on the second.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        check_one(problem, occupancy, constraint, activity, out);
    }

    fn affected(
        &self,
        _problem: &Problem,
        _constraint: &CompiledConstraint,
        _changed: ActivityIx,
    ) -> AffectedSet {
        // The one rule in the MVP that is about the timetable as a whole: placing anything
        // changes how many things are unplaced, so every occurrence's contribution can move.
        AffectedSet::everything()
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        explanation(
            problem,
            violation,
            format!(
                "{} is not placed anywhere — {}. The timetable is incomplete rather than \
                 unusable, so this does not refuse it.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

/// One violation if the occurrence is unplaced, and nothing if it is placed.
///
/// The body both drivers share.
fn check_one(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    activity: ActivityIx,
    out: &mut ViolationSink,
) {
    if occupancy.at(activity).is_some() {
        return;
    }
    let Some(fact) = problem.activities.get(activity.get()) else {
        return;
    };
    let section = occupancy.section_of(activity);
    let label = section.map_or_else(
        || String::from("an unknown class"),
        |section| section.name.clone(),
    );

    out.push(Violation {
        constraint_type: ConstraintType::ActivityScheduled,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![activity],
        subject: EntityRef::new(
            EntityKind::Activity,
            fact.id,
            format!("{label} (occurrence {})", fact.split_index),
        ),
        related: section
            .map(|section| {
                EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone())
            })
            .into_iter()
            .collect(),
        cycle_day: None,
        span: None,
        detail: format!(
            "{} period{} to place",
            fact.duration.get(),
            if fact.duration.get() == 1 { "" } else { "s" }
        ),
    });
}
