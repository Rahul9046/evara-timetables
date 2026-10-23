//! The two rules about *when* a lesson may sit where it has been put.
//!
//! `duration-contiguous` — a multi-period occurrence occupies one unbroken run of teaching
//! periods on one day — and `no-teaching-in-period`, which refuses a lesson in a break.
//!
//! # The measure is `runs_on`, not a second count
//!
//! [`runs_on`](evara_core::scheduling::runs_on) is ADR 0033 §5's measure and the one
//! implementation of *"`d` ordinal-consecutive `TEACHING` periods of its own resolved
//! schedule, on one cycle day"*. ADR 0036 §2 lists `duration-contiguous` as one of its four
//! callers, so this rule asks it whether the run the occurrence actually occupies is one of the
//! legal ones — it does not re-derive what "consecutive" means.
//!
//! Three things that measure already knows and this rule therefore inherits: a non-teaching
//! period between two teaching ones breaks a run; a mere wall-clock gap between consecutive
//! bells does **not**, because a gap between bells is a gap between bells; and a run never
//! spans two schedules even where their minutes are adjacent, because two schedules are two
//! grids and an occurrence lives in one.
//!
//! M6's candidate generator is the fourth caller of the same function: it *enumerates* the
//! legal runs rather than guessing at slots and letting this rule refuse them, which is why a
//! generated timetable never carries a `duration-contiguous` violation and a manually edited
//! one can.
//!
//! # Why `no-teaching-in-period` still exists
//!
//! The loader restricts an activity's candidate slots to the teaching ones, so a generated
//! timetable should never place a lesson in a break. ADR 0033 §4 keeps the constraint anyway,
//! *"because manual editing can name a non-teaching slot and the editor must refuse for the
//! same reason the generator never offered it"* — and both read the one stored
//! `timeslot.is_teaching` rather than inferring teachability from a period's label.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, EntityKind, EntityRef,
    Explanation, Problem, TimeslotIx, Violation, ViolationSink, runs_on,
};

use crate::occupancy::{Occupancy, Placed, day_label};
use crate::{ConstraintEvaluator, explanation};

/// A multi-period occurrence occupies consecutive teaching periods of one day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurationContiguous;

/// A lesson may not occupy a period that is not for teaching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NoTeachingInPeriod;

impl ConstraintEvaluator for DurationContiguous {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::DurationContiguous
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::DurationContiguous)
    }

    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for placed in occupancy.all() {
            check_run(problem, occupancy, constraint, placed, out);
        }
    }

    /// The same check, for one occurrence.
    ///
    /// Trivially narrowed: a run is a property of where this occurrence sits in its own
    /// schedule, and no other placement can make it broken or whole.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        if let Some(placed) = occupancy.at(activity) {
            check_run(problem, occupancy, constraint, placed, out);
        }
    }

    fn affected(
        &self,
        _problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
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

impl ConstraintEvaluator for NoTeachingInPeriod {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::NoTeachingInPeriod
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::NoTeachingInPeriod)
    }

    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for placed in occupancy.all() {
            check_start(problem, occupancy, constraint, placed, out);
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
        if let Some(placed) = occupancy.at(activity) {
            check_start(problem, occupancy, constraint, placed, out);
        }
    }

    fn affected(
        &self,
        _problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        AffectedSet::of(vec![changed])
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        explanation(
            problem,
            violation,
            format!(
                "{} is timetabled into {}, which is not a teaching period.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

/// Whether one placed occurrence's run of slots is a legal one.
///
/// The body both of `duration-contiguous`'s drivers share.
///
/// # Four ways to fail, and the order they are reported in
///
/// 1. **The wrong bell schedule.** The occurrence sits in a slot of a schedule its section
///    did not resolve to, so no run of its own schedule contains it. Reported first because
///    every other answer would be computed against the wrong grid.
/// 2. **The day ran out.** Fewer than `d` slots exist from where it starts.
/// 3. **A break interrupts it.** The run covers a non-teaching period.
/// 4. **It is not one of the legal runs.** `runs_on` enumerates them and this one is not among
///    them — the general case that catches anything the first three miss.
///
/// A single-period occurrence is skipped entirely: one period is trivially a run of one, and
/// `no-teaching-in-period` is what judges whether that period may hold a lesson.
fn check_run(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    placed: &Placed,
    out: &mut ViolationSink,
) {
    let Some(activity) = problem.activities.get(placed.activity.get()) else {
        return;
    };
    let wanted = activity.duration.as_usize();
    if wanted <= 1 {
        return;
    }
    let Some(section) = occupancy.section_of(placed.activity) else {
        return;
    };

    let detail = if placed.defect.wrong_bell_schedule {
        Some(String::from(
            "it is placed in a slot of a bell schedule its class does not use, so no run of \
             its own schedule holds it",
        ))
    } else if !placed.defect.is_complete() {
        Some(format!(
            "it needs {wanted} consecutive periods and only {} remain on {} from where it \
             starts",
            placed.defect.got,
            day_label(problem, placed.span.cycle_day)
        ))
    } else if placed.defect.crosses_non_teaching && !constraint.options.allow_break_span {
        Some(format!(
            "it spans {}, which is not a teaching period",
            non_teaching_label(problem, &placed.span.slots)
        ))
    } else if !is_a_legal_run(problem, placed, wanted, constraint.options.allow_break_span) {
        Some(format!(
            "the {wanted} periods it occupies are not a consecutive run of teaching periods \
             on {}",
            day_label(problem, placed.span.cycle_day)
        ))
    } else {
        None
    };

    if let Some(detail) = detail {
        out.push(Violation {
            constraint_type: ConstraintType::DurationContiguous,
            level: constraint.level,
            constraint_id: constraint.id,
            penalty: -constraint.weight,
            activities: vec![placed.activity],
            subject: EntityRef::new(
                EntityKind::Activity,
                activity.id,
                format!("{} ({} periods)", section.name, wanted),
            ),
            related: vec![EntityRef::new(
                EntityKind::ClassSection,
                section.id,
                section.name.clone(),
            )],
            cycle_day: Some(placed.span.cycle_day),
            span: Some(placed.span.interval),
            detail,
        });
    }
}

/// Whether one placed occurrence starts in a period a lesson may occupy.
///
/// The body both of `no-teaching-in-period`'s drivers share.
///
/// # The starting slot, read from the stored flag
///
/// `timeslot.is_teaching` is derived at materialisation from `period.kind` and from nothing
/// else, so this reads one stored boolean. Nothing infers teachability from a period's label —
/// a period called "Study" may well be a teaching period, and a period called "Period 4" may
/// not be.
///
/// Only the *starting* slot is this rule's business. A double period that reaches into a break
/// is `duration-contiguous`'s finding, and reporting it twice under two codes with two remedies
/// is the duplication `PRE-SOLVE-VALIDATION.md` §5 avoids by giving the three
/// unplaceable-occurrence codes a precedence.
fn check_start(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    placed: &Placed,
    out: &mut ViolationSink,
) {
    if !placed.defect.starts_outside_teaching {
        return;
    }
    let Some(section) = occupancy.section_of(placed.activity) else {
        return;
    };
    let start = placed.span.slots.first().copied();

    out.push(Violation {
        constraint_type: ConstraintType::NoTeachingInPeriod,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![placed.activity],
        subject: EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
        related: period_reference(problem, start).into_iter().collect(),
        cycle_day: Some(placed.span.cycle_day),
        span: Some(placed.span.interval),
        detail: format!(
            "{} on {}",
            start.map_or_else(
                || String::from("that period"),
                |slot| period_label(problem, slot)
            ),
            day_label(problem, placed.span.cycle_day)
        ),
    });
}

/// Whether the span is one of the runs `runs_on` enumerates.
///
/// The general check, asked after the specific ones so a message can be specific. With
/// `allow_break_span` the enumeration is not the right question — a permitted span is by
/// definition not one of the unbroken runs — so the check is skipped and the earlier clauses
/// stand as the whole rule.
fn is_a_legal_run(
    problem: &Problem,
    placed: &Placed,
    wanted: usize,
    allow_break_span: bool,
) -> bool {
    if allow_break_span {
        return true;
    }
    let Some(schedule) = problem
        .timeslots
        .get(placed.span.slots[0].get())
        .map(|slot| slot.bell_schedule)
    else {
        return false;
    };
    runs_on(problem, schedule, placed.span.cycle_day, wanted)
        .into_iter()
        .any(|run| run == placed.span.slots)
}

/// The first non-teaching period inside a run, named.
fn non_teaching_label(problem: &Problem, slots: &[TimeslotIx]) -> String {
    slots
        .iter()
        .skip(1)
        .find(|slot| {
            problem
                .timeslots
                .get(slot.get())
                .is_some_and(|s| !s.is_teaching)
        })
        .map_or_else(
            || String::from("a non-teaching period"),
            |slot| period_label(problem, *slot),
        )
}

/// A period's label, as a school wrote it.
fn period_label(problem: &Problem, slot: TimeslotIx) -> String {
    problem
        .timeslots
        .get(slot.get())
        .and_then(|s| problem.context.periods.get(s.period.get()))
        .map_or_else(|| String::from("that period"), |p| p.label.clone())
}

/// The period a slot belongs to, as a clickable reference.
fn period_reference(problem: &Problem, slot: Option<TimeslotIx>) -> Option<EntityRef> {
    let slot = slot?;
    let period = problem
        .context
        .periods
        .get(problem.timeslots.get(slot.get())?.period.get())?;
    Some(EntityRef::new(
        EntityKind::Period,
        period.id,
        period.label.clone(),
    ))
}
