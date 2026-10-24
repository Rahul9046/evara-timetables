//! `activity-fixed`: a pinned occurrence must be where it is pinned.
//!
//! # The rule locks were waiting for
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's hard set is seven of the clash and timing rules
//! plus `activity-fixed` *"once locks exist"*, and `CONSTRAINT-ENGINE.md` §3 held it back in
//! those words: *"when locks land, `activity-fixed` arrives with its evaluator, its schema, its
//! tests and its row in CONSTRAINTS.md, and the hard set becomes eight."* They have landed.
//!
//! The pin itself is [`Problem::pinned`], one entry per activity, and **the caller populates
//! it**. It is not compiled by the loader and never will be: a lock is a `timetable_entry`
//! column, which is an *output* of a solve, and a `Problem` is compiled from inputs only. A
//! loader that read a timetable would make the compiled problem depend on which timetable
//! happened to be open — ADR 0045 §4.
//!
//! # Why an unplaced pinned occurrence is not a violation of this rule
//!
//! This is the decision in the rule's shape, and it is the same argument
//! [`ActivityScheduled`](crate::completeness) is MEDIUM for.
//!
//! A pin says *where* an occurrence must be. It does not say that it must be somewhere, because
//! `activity-scheduled` already says that — at MEDIUM, so that an incomplete arrangement is
//! *incomplete* rather than infeasible. If this rule also fired on an unplaced pinned
//! occurrence, then [`IncrementalScorer::empty`](crate::IncrementalScorer::empty) over a problem
//! with pins would start **infeasible**, and every intermediate state of a construction would
//! be too. That is exactly the failure M5 refused to introduce when it kept completeness at
//! MEDIUM, and the pin is not a licence to reintroduce it.
//!
//! So: placed somewhere other than the pin is a HARD violation; placed at the pin is silence;
//! not placed is silence here and a MEDIUM `activity-scheduled` violation there.
//!
//! # A pin names a slot **and** a room
//!
//! [`Placement`] is the unit the solver moves and the unit a lock protects, so both halves are
//! compared. A repair that kept a locked lesson's period and took its room away has moved it,
//! and a user who locked a lesson into the laboratory did not lock it into *a* period.
//!
//! # The cheapest of the nine, and the narrowest
//!
//! Every violation names exactly **one** occurrence, so the differential identity
//! (`crate::incremental`'s header) holds trivially and [`AffectedSet`] is the single occurrence.
//! The check is one comparison against one array entry: no sweep, no pair, no interval.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, EntityKind, EntityRef,
    Explanation, Placement, Problem, RoomIx, TimeslotIx, Violation, ViolationSink,
};

use crate::occupancy::{Occupancy, day_label};
use crate::{ConstraintEvaluator, explanation};

/// A pinned occurrence must be exactly where it is pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivityFixed;

impl ConstraintEvaluator for ActivityFixed {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::ActivityFixed
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::ActivityFixed)
    }

    /// # Swept over the pins, not over the timetable
    ///
    /// [`Problem::pinned`] is empty for every problem nobody has pinned anything in, which is
    /// the common case — a generation from scratch, every fixture in the catalogue — and the
    /// sweep is then a single length check. That is not an optimisation: it is what makes
    /// registering a mandatory rule free for the callers that have no pins.
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
        changed: ActivityIx,
    ) -> AffectedSet {
        // A pin is about one occurrence and nothing else. Moving `A` cannot change whether `B`
        // is where it was pinned.
        AffectedSet::of(vec![changed])
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        explanation(
            problem,
            violation,
            format!(
                "{} is locked, and {}. Unlock it before moving it, or move it back.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

/// One violation if the occurrence is placed away from its pin, and nothing otherwise.
///
/// The body both drivers share. Three silent cases, each for its own reason:
///
/// - **no pin** — nothing was asked of this occurrence;
/// - **not placed** — `activity-scheduled`'s question, at MEDIUM, for the reason in this
///   module's header;
/// - **placed at the pin** — the pin is honoured.
fn check_one(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    activity: ActivityIx,
    out: &mut ViolationSink,
) {
    let Some(Some(pin)) = problem.pinned.get(activity.get()) else {
        return;
    };
    let Some(placed) = occupancy.at(activity) else {
        return;
    };
    let at = Placement {
        timeslot: placed.span.slots.first().copied().unwrap_or(pin.timeslot),
        room: placed.span.room,
    };
    if at == *pin {
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
        constraint_type: ConstraintType::ActivityFixed,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![activity],
        subject: EntityRef::new(
            EntityKind::Activity,
            fact.id,
            format!("{label} (occurrence {})", fact.split_index),
        ),
        related: pin_reference(problem, pin.timeslot).into_iter().collect(),
        cycle_day: Some(placed.span.cycle_day),
        span: Some(placed.span.interval),
        detail: format!(
            "it is locked to {} and is placed at {}",
            describe(problem, *pin),
            describe(problem, at)
        ),
    });
}

/// Where a placement is, in the words a school wrote.
///
/// Day and period, never a weekday and never a slot identifier: `cycle_day.ordinal` is a day's
/// only scheduling identity (`CLAUDE.md` §3), and a user reading a refusal needs the label on
/// their own grid.
fn describe(problem: &Problem, placement: Placement) -> String {
    let where_ = match problem.timeslots.get(placement.timeslot.get()) {
        Some(slot) => format!(
            "{} {}",
            day_label(problem, slot.cycle_day),
            problem
                .context
                .periods
                .get(slot.period.get())
                .map_or_else(|| String::from("(unknown period)"), |p| p.label.clone())
        ),
        None => format!("an unknown slot (#{})", placement.timeslot.raw()),
    };
    match placement.room.and_then(|room| room_label(problem, room)) {
        Some(room) => format!("{where_} in {room}"),
        None => match placement.room {
            Some(room) => format!("{where_} in an unknown room (#{})", room.raw()),
            None => format!("{where_} with no room"),
        },
    }
}

/// A room's name, where the problem holds it.
fn room_label(problem: &Problem, room: RoomIx) -> Option<String> {
    problem.rooms.get(room.get()).map(|r| r.name.clone())
}

/// The period a pin names, as a reference a screen can act on.
fn pin_reference(problem: &Problem, timeslot: TimeslotIx) -> Option<EntityRef> {
    let slot = problem.timeslots.get(timeslot.get())?;
    let period = problem.context.periods.get(slot.period.get())?;
    Some(EntityRef::new(
        EntityKind::Period,
        period.id,
        period.label.clone(),
    ))
}
