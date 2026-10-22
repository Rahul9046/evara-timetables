//! The three diagnostics that belong to whatever reads rows.
//!
//! # Why these three are the loader's and not the validator's
//!
//! [ADR 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md): *"A
//! finding about a row belongs to whatever reads rows."* M3 validates a compiled
//! [`Problem`](evara_core::scheduling::Problem), and a `Problem` holds availability rules
//! already compiled onto the slots they name — so a rule naming a deleted teacher compiles
//! onto nothing and a rule naming a resource has no index to compile into. By the time a
//! problem exists, both rows are gone. The information is not missing from M3 by oversight;
//! it is not there to report from.
//!
//! `Problem` was deliberately **not** widened to carry the rejected rows. It is *"everything
//! a solve needs, and nothing else"*, and a list of things the solve must ignore is the
//! opposite of that.
//!
//! # One vocabulary, two producers
//!
//! Every finding here is an ordinary [`Diagnostic`] with a code from the same
//! [`DiagnosticCode`] enum M3 draws from, and the two producers' code sets are **disjoint**
//! — ADR 0037 §1's table. A user reading a pre-flight report should not be able to tell
//! which stage found what, and a caller assembling a report should not have to merge two
//! shapes. There is no parallel diagnostic type here and there must never be one.

use evara_core::scheduling::{
    AvailabilityEntityKind, Diagnostic, DiagnosticCode, EntityKind, EntityRef, Problem,
};

use super::rules::Orphan;

/// Builds the loader's findings, most severe first.
///
/// Returned as a plain `Vec` rather than a report, because the caller merges them with M3's
/// by severity — see [`super::report`] for the merge rule and why it is not a concatenation.
pub(super) fn of(problem: &Problem, orphans: &[Orphan], unscheduled: &[Orphan]) -> Vec<Diagnostic> {
    let mut findings = Vec::new();
    findings.extend(ineligible_teachers(problem));
    findings.extend(orphans.iter().map(|row| orphan(problem, row)));
    findings.extend(unscheduled.iter().map(|row| not_scheduled(problem, row)));
    findings
}

/// Every teacher given a section at a campus their eligibility excludes.
///
/// # A violated storage invariant, and why it is reported rather than enforced
///
/// ADR 0037 §4 is explicit that this is reported **only** as a damaged project, and that the
/// loader does not check eligibility "as a matter of course": ADR 0028 makes employment and
/// campus eligibility database invariants under two trigger pairs, and a solver-side
/// duplicate would be a second implementation of a rule the database owns.
///
/// This check honours that in the way that matters. It issues **no query of its own** — the
/// section's campus, its staffing and each teacher's eligibility are already loaded because
/// the compiled problem needs all three — so it adds no enforcement path and no cost. It is
/// a consistency assertion over facts in hand, exactly as
/// [`Problem::integrity`](evara_core::scheduling::Problem::integrity) is.
///
/// # This is reachable, and M2's documentation of it is wrong
///
/// M2 documented the code as *"unreachable — trigger pairs prevent it"*, and ADR 0037 §4
/// repeats that a consistent database cannot contain one. Neither is true at `V15`.
/// `class_teacher_respects_campus_eligibility` fires on an INSERT or UPDATE of
/// `class_teacher` and **never on a change to `teacher_campus`**, and neither
/// `Membership::assign_teacher_campus` nor `Membership::remove_teacher_campus` consults
/// `class_teacher`. So two ordinary supported calls reach it:
///
/// 1. Give a section to an **unnarrowed** teacher — permitted, because no `teacher_campus`
///    row means "not narrowed" — and then narrow them to a different campus. The first
///    `assign_teacher_campus` call turns an empty eligibility set into `{B}`, and the
///    existing assignment at campus A is now excluded.
/// 2. Remove the one eligibility row an existing assignment relies on.
///
/// `Lessons::update_class_section` closes the third route — moving a section to a campus its
/// staff are not eligible at is refused, and its comment says why — so the gap is on the
/// `teacher_campus` side only. That makes this finding load-bearing rather than theoretical,
/// and the remedy it names is one the user owns.
///
/// One finding per `(section, teacher)` pair, in index order. Not one per teacher: the
/// remedy differs per section — remove them from *this* section, or grant the campus once —
/// and `PRE-SOLVE-VALIDATION.md` §5 is explicit that distinct actionable problems are never
/// suppressed for sharing a code.
fn ineligible_teachers(problem: &Problem) -> Vec<Diagnostic> {
    let mut findings = Vec::new();
    for section in &problem.sections {
        let Some(campus) = section.campus else {
            // Not campus-constrained, so there is nothing to be ineligible for. The same
            // first condition `V15`'s trigger reads.
            continue;
        };
        let Some(place) = problem.context.campuses.get(campus.get()) else {
            continue;
        };
        for teacher in &section.teachers {
            let Some(person) = problem.teachers.get(teacher.get()) else {
                continue;
            };
            // `None` means unnarrowed, which permits every campus. Only a stated narrowing
            // that omits this campus is a violation — reading an empty narrowing as
            // "eligible nowhere" is the mistake the trigger's own comment warns against.
            let Some(eligible) = person.eligible_campuses.as_deref() else {
                continue;
            };
            if eligible.contains(&campus) {
                continue;
            }
            findings.push(
                Diagnostic::new(
                    DiagnosticCode::TeacherNotEligibleForCampus,
                    EntityRef::new(EntityKind::Teacher, person.id, person.name.clone()),
                    format!(
                        "{} is given {} at {}, and is not recorded as eligible for that \
                         campus. The project should not be able to hold this, so something \
                         has narrowed their campuses since the class was assigned.",
                        person.name, section.name, place.name
                    ),
                )
                .with_remedy(format!(
                    "Give {} the campus {} on the Teachers screen, or remove them from {}.",
                    person.name, place.name, section.name
                ))
                .with_related(EntityRef::new(
                    EntityKind::ClassSection,
                    section.id,
                    section.name.clone(),
                ))
                .with_related(EntityRef::new(
                    EntityKind::Campus,
                    place.id,
                    place.name.clone(),
                )),
            );
        }
    }
    findings
}

/// One availability rule whose entity no longer exists.
///
/// `availability.entity_id` is polymorphic and so carries no foreign key — `V11`'s header
/// calls it *"the one place in this schema where a reference cannot be enforced by the
/// database"*. Three things stand in for that foreign key, the third being
/// [`Availability::orphans`](crate::Availability::orphans)'s sweep, and this finding is what
/// makes a failure of the first two visible at the moment it would matter.
///
/// The subject is the **rule**, not its vanished entity: the row is what exists and what the
/// user can remove, and a finding about an entity that is not there would be unclickable.
fn orphan(problem: &Problem, row: &Orphan) -> Diagnostic {
    let noun = noun(row.kind);
    Diagnostic::new(
        DiagnosticCode::OrphanAvailabilityReference,
        EntityRef::new(
            EntityKind::AvailabilityRule,
            row.id,
            format!("availability rule for a {noun}"),
        ),
        format!(
            "An availability rule at {} is about a {noun} ({}) that is no longer in the \
             project, so it restricts nothing. It was most likely left behind when the \
             {noun} was removed.",
            problem.context.school.name, row.entity_id
        ),
    )
    .with_remedy(format!(
        "Delete this rule. Nothing is being enforced by it, and no {noun} is affected."
    ))
    .with_related(school_ref(problem))
}

/// One availability rule about a kind of thing nothing schedules.
///
/// A `RESOURCE` or a `SUBJECT`. The compiled index models four of the column's six kinds, and
/// the asymmetry is deliberate: `resource-clash` is not in the first constraint set, and
/// `Availability::set` refuses to write a `SUBJECT` rule at all, so such a row can only be
/// legacy.
///
/// Informational rather than a warning, matching the severity M2 fixed for the code. It
/// states plainly that the rule is **not being enforced**, because the alternative — a rule
/// that silently does nothing — is indistinguishable from one that was never entered, and
/// those are different states. That is the whole reason `V11` stores only exceptions.
fn not_scheduled(problem: &Problem, row: &Orphan) -> Diagnostic {
    let noun = noun(row.kind);
    Diagnostic::new(
        DiagnosticCode::AvailabilityEntityKindNotScheduled,
        EntityRef::new(
            EntityKind::AvailabilityRule,
            row.id,
            format!("availability rule for a {noun}"),
        ),
        format!(
            "An availability rule at {} is about a {noun} ({}). Evara does not yet timetable \
             around a {noun}, so this rule is recorded and is not being enforced.",
            problem.context.school.name, row.entity_id
        ),
    )
    .with_related(school_ref(problem))
}

/// The school, for a finding that has to say which one it is about.
fn school_ref(problem: &Problem) -> EntityRef {
    EntityRef::new(
        EntityKind::School,
        problem.context.school.id,
        problem.context.school.name.clone(),
    )
}

/// What to call an entity kind in a sentence.
const fn noun(kind: AvailabilityEntityKind) -> &'static str {
    match kind {
        AvailabilityEntityKind::Teacher => "teacher",
        AvailabilityEntityKind::Room => "room",
        AvailabilityEntityKind::StudentGroup => "student group",
        AvailabilityEntityKind::Resource => "resource",
        AvailabilityEntityKind::Campus => "campus",
        AvailabilityEntityKind::Subject => "subject",
    }
}

/// The diagnostic codes the loader owns, as [ADR
/// 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md) §1 assigns
/// them.
///
/// Public because a caller rendering a pre-flight report has a legitimate reason to ask which
/// stage owns a code — and because naming the set once means a test can assert the two
/// producers are disjoint rather than trusting it, and adding a fourth loader finding cannot
/// leave that assertion behind.
pub const PRODUCED: &[DiagnosticCode] = &[
    DiagnosticCode::TeacherNotEligibleForCampus,
    DiagnosticCode::OrphanAvailabilityReference,
    DiagnosticCode::AvailabilityEntityKindNotScheduled,
];
