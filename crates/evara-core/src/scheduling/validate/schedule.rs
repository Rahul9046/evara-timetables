//! Stage 1 and 2: the scheduling context, and whether an occurrence has anywhere to go.
//!
//! Every finding here is **blocking**, and every one of them has a remedy that is a field the
//! user can fill in. That is what §3.2 reserves a hard error for.

use super::{Pass, push};
use crate::scheduling::context::{BellScheduleResolution, NoBellScheduleCause};
use crate::scheduling::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef,
};
use crate::scheduling::index::ActivityIx;
use crate::scheduling::resolve::runs_on;

/// Whether the grid is missing, reporting it once if so.
///
/// # Once for the solve, not once per section
///
/// ADR 0033 §3's first cause is *"`G` is empty"*, and test 9 requires it *"once for the solve
/// rather than once per section"*. With no slots there is nothing section-specific to say: a
/// per-section finding would produce one identical message per section for a single missing
/// step, which is the kind of report a user scrolls past.
///
/// So the subject is the **school**, and the caller skips the per-section resolution findings
/// when this returns `true`.
pub(super) fn context_findings(pass: &Pass<'_>, report: &mut DiagnosticReport) -> bool {
    if !pass.grid.is_empty() {
        return false;
    }

    let school = &pass.problem.context.school;
    let cycle = &pass.problem.context.cycle;
    let term = pass
        .problem
        .context
        .term
        .as_ref()
        .map_or_else(|| "the whole year".to_owned(), |t| t.name.clone());

    let mut finding = Diagnostic::new(
        DiagnosticCode::NoGridMaterialised,
        EntityRef::new(EntityKind::School, school.id, school.name.clone()),
        format!(
            "No timetable grid has been built for {}'s {} in {term}, so there is nowhere to \
             place a lesson. Nothing else about this school's timetable can be checked until \
             there is one.",
            school.name, cycle.name
        ),
    )
    .with_remedy(
        "Build the grid for this cycle and term under Timetable Grid Preview, then generate \
         again.",
    );
    finding = finding.with_related(EntityRef::new(
        EntityKind::Cycle,
        cycle.id,
        cycle.name.clone(),
    ));
    if let Some(term) = &pass.problem.context.term {
        finding =
            finding.with_related(EntityRef::new(EntityKind::Term, term.id, term.name.clone()));
    }
    push(report, finding);
    true
}

/// Sections that resolve to no bell schedule, or to more than one.
///
/// Both are hard: an activity that cannot be placed anywhere is not a scheduling difficulty,
/// it is an unanswered setup question.
pub(super) fn resolution_findings(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (ix, section) in pass.sections() {
        let subject = EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone());
        match &pass.resolved[ix.get()] {
            BellScheduleResolution::Resolved { .. } => {}
            BellScheduleResolution::None { cause } => {
                push(report, no_bell_schedule(pass, section, subject, *cause));
            }
            BellScheduleResolution::Ambiguous { candidates } => {
                let names: Vec<String> = candidates
                    .iter()
                    .map(|ix| {
                        let schedule = &pass.problem.context.bell_schedules[ix.get()];
                        match schedule.campus {
                            None => format!("{} (school-wide)", schedule.name),
                            Some(campus) => format!(
                                "{} ({})",
                                schedule.name,
                                pass.problem.context.campuses[campus.get()].name
                            ),
                        }
                    })
                    .collect();

                let mut finding = Diagnostic::new(
                    DiagnosticCode::AmbiguousBellSchedule,
                    subject,
                    format!(
                        "{} could be timetabled against {} bell schedules: {}. Evara will not \
                         choose between them.",
                        section.name,
                        candidates.len(),
                        names.join(" and ")
                    ),
                )
                .with_remedy(
                    "Build this grid from one of them, or restrict this generation to one of \
                     them.",
                );
                // Every candidate, by name. A message that says "ambiguous" without saying
                // between what is one the user cannot act on.
                for candidate in candidates {
                    let schedule = &pass.problem.context.bell_schedules[candidate.get()];
                    finding = finding.with_related(EntityRef::new(
                        EntityKind::BellSchedule,
                        schedule.id,
                        schedule.name.clone(),
                    ));
                }
                push(report, finding);
            }
        }
    }
}

/// One of ADR 0033 §3's three sentences, chosen by cause.
///
/// Three causes, three remedies. Merging them produces a message the user cannot act on, so
/// each gets its own wording.
fn no_bell_schedule(
    pass: &Pass<'_>,
    section: &crate::scheduling::problem::SectionFact,
    subject: EntityRef,
    cause: NoBellScheduleCause,
) -> Diagnostic {
    let (message, remedy) = match cause {
        // Reachable per section only when the caller did not short-circuit; the solve-level
        // form is `NoGridMaterialised`.
        NoBellScheduleCause::GridEmpty => (
            format!(
                "No timetable grid has been built for this cycle and term, so {} has nowhere \
                 to be placed.",
                section.name
            ),
            "Build one under Timetable Grid Preview before generating.".to_owned(),
        ),
        NoBellScheduleCause::NoScheduleForCampus => {
            let campus = section.campus.map_or_else(
                || "its campus".to_owned(),
                |ix| pass.problem.context.campuses[ix.get()].name.clone(),
            );
            (
                format!(
                    "{} is taught at {campus}, and no bell schedule in this grid applies \
                     there.",
                    section.name
                ),
                format!(
                    "Build the grid from {campus}'s bell schedule, or make a schedule \
                     school-wide."
                ),
            )
        }
        NoBellScheduleCause::SectionHasNoCampus => (
            format!(
                "{} does not say which campus it is taught at, and this school's bell \
                 schedules are each tied to one campus.",
                section.name
            ),
            "Give the section a campus, or make a bell schedule school-wide.".to_owned(),
        ),
    };

    let mut finding =
        Diagnostic::new(DiagnosticCode::NoBellSchedule, subject, message).with_remedy(remedy);
    if let Some(campus) = section.campus {
        let campus = &pass.problem.context.campuses[campus.get()];
        finding = finding.with_related(EntityRef::new(
            EntityKind::Campus,
            campus.id,
            campus.name.clone(),
        ));
    }
    finding
}

/// Occurrences and sections with nowhere to go, for a reason other than resolution.
///
/// Three findings, in a deliberate precedence. All three would fire together on a badly set
/// up section, and reporting the most specific one is what makes the message actionable:
///
/// 1. [`DiagnosticCode::DurationExceedsLongestRun`] — the occurrence is too long for its
///    schedule. Arithmetic, and the remedy is the duration or the bell schedule.
/// 2. [`DiagnosticCode::NoRoomOfRequiredType`] — a room type is required and nothing
///    satisfies it. Also a counting fact, and the remedy is a room.
/// 3. [`DiagnosticCode::EmptyCandidateSet`] — runs exist and rooms exist, and availability
///    has removed every combination. The residual case, and `SOLVER.md` §8 names its usual
///    cause: *"often a single bad availability entry"*.
pub(super) fn structure_findings(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    unsatisfiable_room_requirements(pass, report);
    unplaceable_occurrences(pass, report);
}

/// Sections requiring a kind of room that nothing can satisfy.
///
/// One finding per **section** rather than per occurrence: the requirement lives on the
/// section's course and every occurrence of it fails identically, so a per-occurrence report
/// would repeat one actionable fact four times.
fn unsatisfiable_room_requirements(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (_, section) in pass.sections() {
        if let Some(required) = section.required_room_type
            && section.room_candidates.is_empty()
        {
            let room_type = &pass.problem.room_types[required.get()];
            let bookable_of_type = pass
                .problem
                .rooms
                .iter()
                .filter(|room| room.room_type == Some(required))
                .count();
            let reason = match bookable_of_type {
                0 => format!("this school has no room classified as {}", room_type.name),
                1 => format!(
                    "this school's only {} is either not bookable or not at this section's \
                     campus",
                    room_type.name
                ),
                n => format!(
                    "none of this school's {n} {} rooms is both bookable and at this \
                     section's campus",
                    room_type.name
                ),
            };
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::NoRoomOfRequiredType,
                    EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
                    format!(
                        "{} needs a {} and {reason}, so no lesson of it can be placed.",
                        section.name, room_type.name
                    ),
                )
                .with_remedy(format!(
                    "Add a bookable {} at this section's campus, or clear the room \
                     requirement on its course.",
                    room_type.name
                ))
                .with_related(EntityRef::new(
                    EntityKind::RoomType,
                    room_type.id,
                    room_type.name.clone(),
                )),
            );
        }
    }
}

/// Occurrences too long for their schedule, or whose every option availability has removed.
fn unplaceable_occurrences(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for ix in pass.problem.activity_indices() {
        let activity = &pass.problem.activities[ix.get()];
        let Some(section) = pass.problem.section_of(ix) else {
            continue;
        };
        let Some(schedule) = pass.schedule_of(activity.section) else {
            // Unresolved: already reported against the section, and there is no schedule to
            // measure a run in.
            continue;
        };

        let needed = activity.duration.as_usize();
        let runs = all_runs(pass, schedule, needed);

        if runs.is_empty() {
            let longest = crate::scheduling::resolve::longest_teaching_run(pass.problem, schedule);
            let bells = &pass.problem.context.bell_schedules[schedule.get()];
            let label = occurrence_label(pass, ix);
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::DurationExceedsLongestRun,
                    EntityRef::new(EntityKind::Activity, activity.id, label.clone()),
                    format!(
                        "{label} needs {needed} consecutive teaching periods, and {} never \
                         offers more than {longest} in a row. No timetable can place it.",
                        bells.name
                    ),
                )
                .with_remedy(format!(
                    "Shorten the lesson to {longest} period{}, or lengthen the run of \
                     teaching periods in {}.",
                    if longest == 1 { "" } else { "s" },
                    bells.name
                ))
                .with_related(EntityRef::new(
                    EntityKind::ClassSection,
                    section.id,
                    section.name.clone(),
                ))
                .with_related(EntityRef::new(
                    EntityKind::BellSchedule,
                    bells.id,
                    bells.name.clone(),
                )),
            );
            continue;
        }

        // Runs exist. Does availability leave any of them usable, with a room where one is
        // required?
        if section.required_room_type.is_some() && section.room_candidates.is_empty() {
            // Already reported, more specifically, against the section.
            continue;
        }
        if runs.iter().any(|run| run_is_usable(pass, section, run)) {
            continue;
        }
        let residual = occurrence_label(pass, ix);
        push(
            report,
            Diagnostic::new(
                DiagnosticCode::EmptyCandidateSet,
                EntityRef::new(EntityKind::Activity, activity.id, residual.clone()),
                format!(
                    "Every possible time for {residual} is ruled out by an availability rule \
                     on its teacher, its student group or its rooms, so it cannot be placed."
                ),
            )
            .with_remedy(
                "Review the availability entered for this section's teacher, groups and \
                 rooms — a single unavailable entry can remove every option."
                    .to_owned(),
            )
            .with_related(EntityRef::new(
                EntityKind::ClassSection,
                section.id,
                section.name.clone(),
            )),
        );
    }
}

/// Every run of `needed` consecutive teaching periods in one schedule, across every day.
fn all_runs(
    pass: &Pass<'_>,
    schedule: crate::scheduling::index::BellScheduleIx,
    needed: usize,
) -> Vec<Vec<crate::scheduling::index::TimeslotIx>> {
    let mut runs = Vec::new();
    for day in 0..pass.problem.context.cycle_days.len() {
        let day = crate::scheduling::index::CycleDayIx::new(u32::try_from(day).unwrap_or(u32::MAX));
        runs.extend(runs_on(pass.problem, schedule, day, needed));
    }
    runs
}

/// Whether one run survives the availability rules the section's resources carry.
///
/// # Scope, deliberately narrow
///
/// This reads **only** hard [`SlotAvailability::Unavailable`](crate::scheduling::SlotAvailability)
/// rules on the section's teachers, its attending groups and — where a room is required — its
/// candidate rooms. It is not constraint evaluation: it does not consider other activities, it
/// does not consider clashes, and it never decides that a run *is* a legal placement.
///
/// It answers one question soundly in one direction: *is every option already removed?* If
/// any run survives, no finding is produced, which is the conservative answer. M5's
/// `entity-unavailable` is the evaluator that enforces the same stored rules against a
/// candidate timetable, and both read the one compiled index.
fn run_is_usable(
    pass: &Pass<'_>,
    section: &crate::scheduling::problem::SectionFact,
    run: &[crate::scheduling::index::TimeslotIx],
) -> bool {
    use crate::scheduling::availability::{AvailabilityEntity, SlotAvailability};

    let blocked_for = |entity: AvailabilityEntity| {
        run.iter().any(|slot| {
            pass.problem.unavailable.status(entity, *slot) == SlotAvailability::Unavailable
        })
    };

    for teacher in &section.teachers {
        if blocked_for(AvailabilityEntity::Teacher { teacher: *teacher }) {
            return false;
        }
    }
    for group in &section.audience {
        if blocked_for(AvailabilityEntity::Group { group: *group }) {
            return false;
        }
    }
    if section.required_room_type.is_some() {
        // At least one candidate room must be free for the whole run.
        let any_room = section
            .room_candidates
            .iter()
            .any(|room| !blocked_for(AvailabilityEntity::Room { room: *room }));
        if !any_room {
            return false;
        }
    }
    true
}

/// What to call one occurrence in a message.
///
/// `activity.name` where the school gave it one — "Laboratory", "Double" — and otherwise the
/// section's name with the occurrence number, because "activity 7" means nothing to a reader.
fn occurrence_label(pass: &Pass<'_>, ix: ActivityIx) -> String {
    let activity = &pass.problem.activities[ix.get()];
    match (&activity.name, pass.problem.section_of(ix)) {
        (Some(name), Some(section)) => format!("{} — {name}", section.name),
        (Some(name), None) => name.clone(),
        (None, Some(section)) => {
            format!("{} (lesson {})", section.name, activity.split_index)
        }
        (None, None) => format!("lesson {}", activity.split_index),
    }
}
