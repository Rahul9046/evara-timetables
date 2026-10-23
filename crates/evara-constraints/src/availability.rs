//! `entity-unavailable`: a placement where a school has said no.
//!
//! # Only `UNAVAILABLE` is hard
//!
//! `availability.status` has three values and this rule enforces one.
//! [`SlotAvailability::permits_placement`](evara_core::scheduling::SlotAvailability) is the
//! test, and it admits everything except `Unavailable` — so a `DISCOURAGED` slot is **legal**
//! and merely worse, which is what makes it soft. The soft half is `preferred-time`, which is
//! not in the MVP; treating `DISCOURAGED` as hard here would refuse placements a school said
//! it would accept.
//!
//! **A missing rule is not a restriction.** `V11` stores only exceptions, so an entity with no
//! rows is free everywhere and [`SlotAvailability::Unconstrained`] is a distinct fourth state.
//! Nothing here reads an absent rule as a prohibition.
//!
//! # Already compiled onto slots, and not widened
//!
//! The index M4 built holds each rule against the slots it actually names — ADR 0033 §7, and
//! the loader *"widens nothing"*. A teacher unavailable "period 1, every day" at a two-schedule
//! school has said something about one schedule, and this rule inherits that: it asks the index
//! about the slots an occurrence occupies and never re-expands a rule's granularity.
//!
//! The consequent limitation is named rather than solved: M3's
//! `teacher-under-several-bell-schedules` is what makes the gap visible. And no date is read —
//! `effective_from` / `effective_to` are carried on a rule and consulted by nothing, because a
//! cycle carries no dates (ADR 0032 §5).
//!
//! # Three entity kinds, because those are the three that matter
//!
//! Teachers, groups and rooms: the entities an occurrence actually occupies. A `CAMPUS` rule is
//! compiled into the index too and is not consulted here, because an occurrence does not
//! occupy a campus — its room does, and the room's own availability is what applies. `RESOURCE`
//! and `SUBJECT` never reach the index at all; the loader reports them as not scheduled.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, AvailabilityEntity, CompiledConstraint, ConstraintType, EntityKind,
    EntityRef, Explanation, Problem, TimeslotIx, Violation, ViolationSink,
};

use crate::occupancy::{Occupancy, day_label};
use crate::{ConstraintEvaluator, explanation};

/// A placement must not fall where a hard availability rule forbids it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntityUnavailable;

impl ConstraintEvaluator for EntityUnavailable {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::EntityUnavailable
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::EntityUnavailable)
    }

    /// # Every slot of the span, not only the first
    ///
    /// A double period occupies two slots, and a teacher unavailable for the second of them is
    /// unavailable for the lesson. So the test runs over every slot the span covers, and the
    /// first offending one is reported — one violation per `(occurrence, entity)` rather than
    /// one per slot, because the remedy is the same for all of them.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for placed in occupancy.all() {
            let Some(section) = occupancy.section_of(placed.activity) else {
                continue;
            };

            let mut involved: Vec<(AvailabilityEntity, EntityRef)> = Vec::new();
            for teacher in &section.teachers {
                if let Some(fact) = problem.teachers.get(teacher.get()) {
                    involved.push((
                        AvailabilityEntity::Teacher { teacher: *teacher },
                        EntityRef::new(EntityKind::Teacher, fact.id, fact.name.clone()),
                    ));
                }
            }
            for group in &section.audience {
                if let Some(fact) = problem.groups.get(group.get()) {
                    involved.push((
                        AvailabilityEntity::Group { group: *group },
                        EntityRef::new(EntityKind::StudentGroup, fact.id, fact.name.clone()),
                    ));
                }
            }
            if let Some(room) = placed.span.room
                && let Some(fact) = problem.rooms.get(room.get())
            {
                involved.push((
                    AvailabilityEntity::Room { room },
                    EntityRef::new(EntityKind::Room, fact.id, fact.name.clone()),
                ));
            }

            for (entity, reference) in involved {
                let Some(slot) = first_forbidden(problem, &placed.span.slots, entity) else {
                    continue;
                };
                out.push(Violation {
                    constraint_type: ConstraintType::EntityUnavailable,
                    level: constraint.level,
                    constraint_id: constraint.id,
                    penalty: constraint.weight,
                    activities: vec![placed.activity],
                    subject: reference,
                    related: vec![EntityRef::new(
                        EntityKind::ClassSection,
                        section.id,
                        section.name.clone(),
                    )],
                    cycle_day: Some(placed.span.cycle_day),
                    span: Some(placed.span.interval),
                    detail: slot_label(problem, slot, placed.span.cycle_day),
                });
            }
        }
    }

    fn affected(
        &self,
        _problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        // Local: whether an occurrence sits where its people are free depends on where *it*
        // is and on nothing any other occurrence does.
        AffectedSet::of(vec![changed])
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        let section = violation
            .related
            .first()
            .map_or_else(|| String::from("this lesson"), |e| e.label.clone());
        explanation(
            problem,
            violation,
            format!(
                "{} is recorded as unavailable at {}, and {section} is timetabled then.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

/// The first slot of the span the entity may not be placed in.
fn first_forbidden(
    problem: &Problem,
    slots: &[TimeslotIx],
    entity: AvailabilityEntity,
) -> Option<TimeslotIx> {
    slots.iter().copied().find(|slot| {
        !problem
            .unavailable
            .status(entity, *slot)
            .permits_placement()
    })
}

/// A slot named the way a school would name it: the period's label and the day's.
fn slot_label(
    problem: &Problem,
    slot: TimeslotIx,
    day: evara_core::scheduling::CycleDayIx,
) -> String {
    let period = problem
        .timeslots
        .get(slot.get())
        .and_then(|s| problem.context.periods.get(s.period.get()))
        .map_or_else(|| String::from("that period"), |p| p.label.clone());
    format!("{period} on {}", day_label(problem, day))
}
