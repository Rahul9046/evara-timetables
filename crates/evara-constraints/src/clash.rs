//! The three clash rules: a teacher, an audience, a room, each in two places at once.
//!
//! # One sweep, three keys
//!
//! All three ask the same question — *do these two occurrences occupy overlapping minutes?* —
//! and differ only in which pairs are worth asking about. So all three narrow first and then
//! call [`Occupancy::overlapping_pairs`], which is the one interval sweep.
//!
//! | Rule | Narrowed by |
//! | --- | --- |
//! | `teacher-clash` | the teacher: only the occurrences one person teaches can clash for them |
//! | `room-clash` | the room: only occurrences booked into the same `room_id` |
//! | `student-clash` | the cycle day, then filtered by mask: a pair clashes when one's `clash_mask` meets the other's `audience_mask` |
//!
//! # Minutes, never timeslot identifiers
//!
//! [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md) rule
//! 2 and `CONSTRAINTS.md`'s note on shared rooms: *"comparing `timeslot_id` is not the same as
//! comparing time: the obvious implementation is wrong exactly when sharing matters, and wrong
//! silently."* A school running two campuses on two bell schedules holds two disjoint slot
//! sets over the same minutes, so a shared hall booked at 09:00 by each has two different
//! `timeslot_id`s and one physical double-booking. The sweep compares intervals, and
//! [`PlacementSpan::clashes_in_time_with`](evara_core::scheduling::PlacementSpan) is the one
//! implementation of that test.
//!
//! Adjacent lessons do not clash: the intervals are half-open, `[start, end)`.
//!
//! # Each rule has two drivers and one body
//!
//! `evaluate` sweeps the whole timetable; `evaluate_involving` asks the same question about one
//! occurrence, for the incremental scorer. Both build their violations through the same
//! `*_violation` function, so the narrowed answer cannot differ in wording, ordering or
//! penalty from the swept one. The pair is always ordered `(lower index, higher index)` at the
//! point the violation is built, which is what makes the two routes produce an identical
//! [`Violation`] for the same clash — and therefore an identical
//! [`ViolationKey`](evara_core::scheduling::ViolationKey) to deduplicate on.

use evara_core::scheduling::{
    ActivityIx, AffectedSet, CompiledConstraint, ConstraintType, EntityKind, EntityRef,
    Explanation, Problem, RoomIx, SectionFact, TeacherIx, Violation, ViolationSink,
};

use crate::occupancy::{Occupancy, day_label};
use crate::{ConstraintEvaluator, explanation};

/// A teacher may not be in two activities at overlapping times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TeacherClash;

/// Two activities may not overlap when their audiences confirmedly overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StudentClash;

/// A room may not host two activities at overlapping wall-clock times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomClash;

impl ConstraintEvaluator for TeacherClash {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::TeacherClash
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::TeacherClash)
    }

    /// # Several teachers per section is ordinary
    ///
    /// A section's staffing is a list, and every one of them is in the room — so an occurrence
    /// appears in the subset of each teacher who teaches its section, and two occurrences
    /// sharing two teachers produce two violations, one per person. That is right: each is a
    /// separate thing to fix, and `PRE-SOLVE-VALIDATION.md` §5's rule against suppressing
    /// distinct actionable problems applies here too.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for (at, _) in problem.teachers.iter().enumerate() {
            let teacher = TeacherIx::new(u32::try_from(at).unwrap_or(u32::MAX));
            let mine = occupancy.taught_by(teacher);
            if mine.len() < 2 {
                continue;
            }
            for (a, b) in occupancy.overlapping_pairs(mine) {
                teacher_violation(problem, occupancy, constraint, teacher, a, b, out);
            }
        }
    }

    /// Narrowed to the teachers of this occurrence's own section.
    ///
    /// Moving one lesson cannot create a clash for somebody who does not teach it, so the
    /// outer loop over every teacher becomes a loop over the two or three who are in the room.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        let Some(section) = occupancy.section_of(activity) else {
            return;
        };
        for teacher in &section.teachers {
            for other in occupancy.overlapping_with(activity, occupancy.taught_by(*teacher)) {
                let (a, b) = ordered(activity, other);
                teacher_violation(problem, occupancy, constraint, *teacher, a, b, out);
            }
        }
    }

    fn affected(
        &self,
        problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        // Moving an occurrence can only change this rule's verdict about occurrences taught by
        // one of *its* teachers. Everything else is untouched.
        let Some(section) = problem.section_of(changed) else {
            return AffectedSet::nothing();
        };
        AffectedSet::of(
            problem
                .activity_indices()
                .filter(|other| {
                    problem.section_of(*other).is_some_and(|theirs| {
                        theirs.teachers.iter().any(|t| section.teachers.contains(t))
                    })
                })
                .collect(),
        )
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        let others = named(&violation.related);
        explanation(
            problem,
            violation,
            format!(
                "{} is timetabled to teach {others} at the same time, {}. One person cannot \
                 be in two rooms at once.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

impl ConstraintEvaluator for StudentClash {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::StudentClash
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::StudentClash)
    }

    /// # The relation is read, never re-derived
    ///
    /// `clash_mask(A) ∩ audience_mask(B) ≠ ∅` — ADR 0035 §5's test, over the masks the loader
    /// compiled from the relation `evara_core::scheduling::overlap` produced. This evaluator
    /// contains no part of the seven rules, and that is deliberate: rule 3 precedes rule 6, so
    /// a second implementation that ordered them differently would honour a clash here and
    /// ignore it in the loader.
    ///
    /// **An `Indeterminate` pair is not a violation.** `clash_mask` admits only `Overlaps`, so
    /// an undecided pair never reaches this test — ADR 0035 §6: *"enforce what is proved, and
    /// make what is not proved impossible to miss."* The making-it-impossible-to-miss is M3's
    /// `group-overlap-indeterminate` warning and the unverified count a generated timetable
    /// carries; it is not this rule's job, and inventing a violation here would mean a year
    /// group could hold one lesson at a time.
    ///
    /// **Overlap inside one section is not a clash.** A section taught to a cohort *and* to
    /// one of its sets has an internally overlapping audience; those people are in one room
    /// together, which is what a combined class is. The relation is only consulted between two
    /// occurrences of *different* sections.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        let all: Vec<ActivityIx> = occupancy.all().map(|placed| placed.activity).collect();
        for (a, b) in occupancy.overlapping_pairs(&all) {
            student_violation(problem, occupancy, constraint, a, b, out);
        }
    }

    /// Narrowed to this occurrence's own cycle position.
    ///
    /// Sound because a span never crosses a cycle position and the interval test requires the
    /// days to be equal, so nothing off this day can clash — see
    /// [`Occupancy::on_day`](crate::Occupancy). The audience test that follows is unchanged and
    /// is applied in the same `(lower, higher)` order the sweep uses, because `clash_mask` and
    /// `audience_mask` are two different masks and the pair order decides which is read from
    /// which side.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        let Some(placed) = occupancy.at(activity) else {
            return;
        };
        let same_day = occupancy.on_day(placed.span.cycle_day).to_vec();
        for other in occupancy.overlapping_with(activity, &same_day) {
            let (a, b) = ordered(activity, other);
            student_violation(problem, occupancy, constraint, a, b, out);
        }
    }

    fn affected(
        &self,
        problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        let Some(section) = problem.section_of(changed) else {
            return AffectedSet::nothing();
        };
        AffectedSet::of(
            problem
                .activity_indices()
                .filter(|other| {
                    problem.section_of(*other).is_some_and(|theirs| {
                        section.clash_mask.intersects(&theirs.audience_mask)
                            || theirs.clash_mask.intersects(&section.audience_mask)
                    })
                })
                .collect(),
        )
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        let groups: Vec<&EntityRef> = violation
            .related
            .iter()
            .filter(|e| e.kind == EntityKind::StudentGroup)
            .collect();
        let because = if groups.is_empty() {
            String::from("their audiences share students")
        } else {
            format!(
                "{} attends both",
                groups
                    .iter()
                    .map(|g| g.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" and ")
            )
        };
        let other = violation
            .related
            .iter()
            .find(|e| e.kind == EntityKind::ClassSection)
            .map_or_else(|| String::from("another lesson"), |e| e.label.clone());

        explanation(
            problem,
            violation,
            format!(
                "{} and {other} are timetabled at the same time, {}, and {because}. Those \
                 students cannot be in both rooms.",
                violation.subject.label, violation.detail
            ),
        )
    }
}

impl ConstraintEvaluator for RoomClash {
    fn constraint_type(&self) -> ConstraintType {
        ConstraintType::RoomClash
    }

    fn param_schema(&self) -> &'static str {
        crate::params::schema_of(ConstraintType::RoomClash)
    }

    /// # Keyed per `room_id`, never per campus
    ///
    /// ADR 0014 makes a shared room **one** room with one identity, and `CONSTRAINTS.md` is
    /// explicit that this rule *"must never be evaluated per campus or with a campus term in
    /// its key: if Campus A has booked a shared hall, Campus B cannot book it, and a
    /// per-campus evaluation would let both succeed and produce a timetable that double-books
    /// a physical space while reporting no violation."*
    ///
    /// So the key is the room index, which is one per `room.id`, and the campus is not in it.
    /// An occurrence with no room booked is not compared: a roomless lesson occupies no room.
    fn evaluate(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        out: &mut ViolationSink,
    ) {
        for at in 0..problem.rooms.len() {
            let room = RoomIx::new(u32::try_from(at).unwrap_or(u32::MAX));
            let booked = occupancy.booked_into(room);
            if booked.len() < 2 {
                continue;
            }
            for (a, b) in occupancy.overlapping_pairs(booked) {
                room_violation(problem, occupancy, constraint, room, a, b, out);
            }
        }
    }

    /// Narrowed to the one room this occurrence is booked into, or nothing where it has none.
    ///
    /// A roomless lesson occupies no room, so it can neither cause nor suffer a room clash —
    /// which is the same reason `evaluate` skips it.
    fn evaluate_involving(
        &self,
        problem: &Problem,
        occupancy: &Occupancy<'_>,
        constraint: &CompiledConstraint,
        activity: ActivityIx,
        out: &mut ViolationSink,
    ) {
        let Some(room) = occupancy.at(activity).and_then(|placed| placed.span.room) else {
            return;
        };
        for other in occupancy.overlapping_with(activity, occupancy.booked_into(room)) {
            let (a, b) = ordered(activity, other);
            room_violation(problem, occupancy, constraint, room, a, b, out);
        }
    }

    fn affected(
        &self,
        problem: &Problem,
        _constraint: &CompiledConstraint,
        changed: ActivityIx,
    ) -> AffectedSet {
        // A room is a planning variable, so a move can change which room is involved. Every
        // occurrence that could be placed in a room this one could also be placed in is
        // affected, which is its section's candidate set.
        let Some(section) = problem.section_of(changed) else {
            return AffectedSet::nothing();
        };
        AffectedSet::of(
            problem
                .activity_indices()
                .filter(|other| {
                    problem.section_of(*other).is_some_and(|theirs| {
                        theirs
                            .room_candidates
                            .iter()
                            .any(|room| section.room_candidates.contains(room))
                    })
                })
                .collect(),
        )
    }

    fn explain(&self, problem: &Problem, violation: &Violation) -> Explanation {
        explanation(
            problem,
            violation,
            format!(
                "{} is booked for {} at the same time, {}. One room holds one lesson.",
                violation.subject.label,
                named(&violation.related),
                violation.detail
            ),
        )
    }
}

/// The pair, lower activity index first.
///
/// Both drivers of every clash rule order the pair here, so the violation they build — and the
/// [`ViolationKey`](evara_core::scheduling::ViolationKey) it deduplicates on — is the same
/// whichever side the question was asked from.
const fn ordered(a: ActivityIx, b: ActivityIx) -> (ActivityIx, ActivityIx) {
    if a.raw() <= b.raw() { (a, b) } else { (b, a) }
}

/// One teacher double-booked, as a violation.
///
/// Shared by both drivers. `a` is the lower activity index, which is where the reported span
/// and day come from — so the message is the same whichever occurrence moved.
fn teacher_violation(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    teacher: TeacherIx,
    a: ActivityIx,
    b: ActivityIx,
    out: &mut ViolationSink,
) {
    let Some(fact) = problem.teachers.get(teacher.get()) else {
        return;
    };
    let span = occupancy.at(a).map(|p| p.span.clone());
    out.push(Violation {
        constraint_type: ConstraintType::TeacherClash,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![a, b],
        subject: EntityRef::new(EntityKind::Teacher, fact.id, fact.name.clone()),
        related: sections_of(occupancy, a, b),
        cycle_day: span.as_ref().map(|s| s.cycle_day),
        span: span.as_ref().map(|s| s.interval),
        detail: when(problem, occupancy, a),
    });
}

/// One room double-booked, as a violation.
fn room_violation(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    room: RoomIx,
    a: ActivityIx,
    b: ActivityIx,
    out: &mut ViolationSink,
) {
    let Some(fact) = problem.rooms.get(room.get()) else {
        return;
    };
    let span = occupancy.at(a).map(|p| p.span.clone());
    out.push(Violation {
        constraint_type: ConstraintType::RoomClash,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![a, b],
        subject: EntityRef::new(EntityKind::Room, fact.id, fact.name.clone()),
        related: sections_of(occupancy, a, b),
        cycle_day: span.as_ref().map(|s| s.cycle_day),
        span: span.as_ref().map(|s| s.interval),
        detail: when(problem, occupancy, a),
    });
}

/// Two overlapping occurrences whose audiences are proved to share students, as a violation.
///
/// Applies ADR 0035 §5's test and the two exclusions — a section does not clash with itself,
/// and an undecided pair is not a violation because `clash_mask` admits only `Overlaps`. Does
/// nothing where the test does not hold, so both drivers can offer it any overlapping pair.
fn student_violation(
    problem: &Problem,
    occupancy: &Occupancy<'_>,
    constraint: &CompiledConstraint,
    a: ActivityIx,
    b: ActivityIx,
    out: &mut ViolationSink,
) {
    let (Some(left), Some(right)) = (occupancy.section_of(a), occupancy.section_of(b)) else {
        return;
    };
    // Two occurrences of one section are the same people in one room; a section does not
    // clash with itself.
    if left.id == right.id {
        return;
    }
    if !left.clash_mask.intersects(&right.audience_mask) {
        return;
    }

    let shared = proved_shared(problem, left, right);
    let span = occupancy.at(a).map(|p| p.span.clone());
    out.push(Violation {
        constraint_type: ConstraintType::StudentClash,
        level: constraint.level,
        constraint_id: constraint.id,
        penalty: -constraint.weight,
        activities: vec![a, b],
        subject: EntityRef::new(EntityKind::ClassSection, left.id, left.name.clone()),
        related: std::iter::once(EntityRef::new(
            EntityKind::ClassSection,
            right.id,
            right.name.clone(),
        ))
        .chain(shared)
        .collect(),
        cycle_day: span.as_ref().map(|s| s.cycle_day),
        span: span.as_ref().map(|s| s.interval),
        detail: when(problem, occupancy, a),
    });
}

/// The two sections involved in a clash, as related references.
fn sections_of(occupancy: &Occupancy<'_>, a: ActivityIx, b: ActivityIx) -> Vec<EntityRef> {
    [a, b]
        .iter()
        .filter_map(|at| occupancy.section_of(*at))
        .map(|section| EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()))
        .collect()
}

/// A group proved to attend both sections, where the relation names one.
///
/// Carried so `explain` can say *which* students cannot be in both rooms rather than only
/// that some can't. Reads the compiled relation; derives nothing.
fn proved_shared(problem: &Problem, left: &SectionFact, right: &SectionFact) -> Vec<EntityRef> {
    let mut found = Vec::new();
    for theirs in &right.audience {
        if left.clash_mask.contains(*theirs)
            && let Some(group) = problem.groups.get(theirs.get())
        {
            found.push(EntityRef::new(
                EntityKind::StudentGroup,
                group.id,
                group.name.clone(),
            ));
        }
    }
    found
}

/// When an occurrence runs, as a phrase a sentence can end with.
fn when(problem: &Problem, occupancy: &Occupancy<'_>, activity: ActivityIx) -> String {
    occupancy.at(activity).map_or_else(
        || String::from("at the same time"),
        |placed| {
            format!(
                "{} on {}",
                interval_label(placed.span.interval),
                day_label(problem, placed.span.cycle_day)
            )
        },
    )
}

/// `09:00–10:00`, for a message.
fn interval_label(interval: evara_core::scheduling::TimeInterval) -> String {
    format!(
        "{:02}:{:02}\u{2013}{:02}:{:02}",
        interval.start.minutes() / 60,
        interval.start.minutes() % 60,
        interval.end.minutes() / 60,
        interval.end.minutes() % 60,
    )
}

/// The section names among a violation's related references, as an English list.
fn named(related: &[EntityRef]) -> String {
    let names: Vec<&str> = related
        .iter()
        .filter(|e| e.kind == EntityKind::ClassSection)
        .map(|e| e.label.as_str())
        .collect();
    match names.as_slice() {
        [] => String::from("two lessons"),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} and {b}"),
        _ => names.join(", "),
    }
}
