//! `room-type-required`: a lesson that needs a laboratory gets one.
//!
//! # The candidate predicate is called, not restated
//!
//! [`RoomFact::is_candidate_for`](evara_core::scheduling::RoomFact::is_candidate_for) is the
//! **one** implementation of the room candidate domain — bookable, reachable from the
//! section's campus, of the required type. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.1 names
//! this as the requirement M5 must not lose, because *"a domain the manual editor could bypass
//! is a rule living in two places."*
//!
//! So this evaluator asks that function and reports its answer. It does not compare a room's
//! `room_type` to a section's `required_room_type` itself, which would be the second
//! implementation — and which would miss the other two conditions, letting an unbookable room
//! or a room on the wrong campus pass.
//!
//! M6's candidate generator is the third caller of the same function, which is what makes
//! *"the generator never offers a room this rule would refuse"* a property of one predicate
//! rather than of two that happen to agree.
//!
//! # Three ways to fail, one rule
//!
//! A required type with no room assigned, a room that is not a candidate, and a section whose
//! candidate set is empty are all this rule. The last is also M3's `no-room-of-required-type`,
//! reported before a solve from the problem alone — the overlap is deliberate and the two do
//! not conflict: M3 says *"this can never work"*, and this says *"this placement does not"*.
//!
//! # Capacity is not here
//!
//! `room-capacity` is not in the first constraint set — ADR 0034 defers it because nothing
//! supplies an evaluable left-hand side. A room of unverified size is a caveat on a timetable,
//! and this rule says nothing about how many people fit.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, EntityKind, EntityRef,
    Explanation, Problem, RoomFact, RoomTypeIx, SectionFact, Violation, ViolationSink,
};

use crate::occupancy::{Occupancy, Placed};
use crate::{ConstraintEvaluator, explanation};

/// An assigned room must satisfy the section's room requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomTypeRequired;

impl ConstraintEvaluator for RoomTypeRequired {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::RoomTypeRequired
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::RoomTypeRequired)
    }

    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for placed in occupancy.all() {
            check_one(problem, occupancy, constraint, placed, out);
        }
    }

    /// The same check, for one occurrence.
    ///
    /// Trivially narrowed: whether *this* occurrence's room suits *this* section depends on
    /// nothing else in the timetable, which is the same fact [`RoomTypeRequired::affected`]
    /// reports. Both drivers call the same `check_one`.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        if let Some(placed) = occupancy.at(activity) {
            check_one(problem, occupancy, constraint, placed, out);
        }
    }

    fn affected(
        &self,
        _problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        // Entirely local: whether *this* occurrence's room suits *this* section depends on
        // nothing else in the timetable. Moving one lesson cannot make another's room wrong.
        AffectedSet::of(vec![changed])
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        explanation(
            problem,
            violation,
            format!("{}: {}.", violation.subject.label, violation.detail),
        )
    }
}

/// Whether one placed occurrence's room satisfies its section's requirement.
///
/// The body both drivers share, and this rule's only call of the candidate predicate.
fn check_one(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    placed: &Placed,
    out: &mut ViolationSink,
) {
    let Some(section) = occupancy.section_of(placed.activity) else {
        return;
    };
    let subject = EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone());
    let required = section.required_room_type;

    let found = match placed.span.room {
        // No room booked. Only a problem where the section requires a type: a lesson with no
        // requirement and no room is an unroomed lesson, which ADR 0034 §4 prefers to a lesson
        // in a room of unverified size and which this rule has no opinion about.
        None => {
            if required.is_none() {
                return;
            }
            let kind = type_label(problem, required);
            (
                format!("no room is booked for it, and it needs {kind}"),
                Vec::new(),
            )
        }
        Some(room) => {
            let Some(fact) = problem.rooms.get(room.get()) else {
                return;
            };
            // The one candidate predicate. Three conditions, asked once.
            if fact.is_candidate_for(section.campus, required) {
                return;
            }
            (
                why_not(problem, fact, section, required),
                vec![EntityRef::new(EntityKind::Room, fact.id, fact.name.clone())],
            )
        }
    };

    let (detail, related) = found;
    out.push(Violation {
        constraint_type: ConstraintType::RoomTypeRequired,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![placed.activity],
        subject,
        related,
        cycle_day: Some(placed.span.cycle_day),
        span: Some(placed.span.interval),
        detail,
    });
}

/// Which of the three conditions the room fails, named so the remedy is obvious.
///
/// Asked only after `is_candidate_for` has said no, so exactly one of these is the answer and
/// the order matches the predicate's own.
fn why_not(
    problem: &Problem,
    room: &RoomFact,
    section: &SectionFact,
    required: Option<RoomTypeIx>,
) -> String {
    if !room.is_bookable {
        return format!("{} is not bookable", room.name);
    }
    if let Some(campus) = section.campus
        && !room.campuses.contains(&campus)
    {
        let place = problem
            .context
            .campuses
            .get(campus.get())
            .map_or_else(|| String::from("its campus"), |c| c.name.clone());
        return format!("{} is not reachable from {place}", room.name);
    }
    let wanted = type_label(problem, required);
    let has = type_label(problem, room.room_type);
    format!("it needs {wanted} and {} is {has}", room.name)
}

/// A room type's name, or an honest phrase for the absence of one.
fn type_label(problem: &Problem, room_type: Option<RoomTypeIx>) -> String {
    match room_type {
        None => String::from("no particular kind of room"),
        Some(ix) => problem.room_types.get(ix.get()).map_or_else(
            || String::from("a kind of room this project no longer holds"),
            |t| format!("a {}", t.name),
        ),
    }
}
