//! Stage 4 and 5: what will not be checked, what is shared, and what merely disagrees.
//!
//! # None of this blocks
//!
//! Every finding here is a [`Severity::Warning`](crate::scheduling::Severity) or a
//! [`Severity::Info`](crate::scheduling::Severity), and the reason is §3.2's rule: a hard
//! error is reserved for an input whose remedy is a field the user can fill in. The remedies
//! here are a setup task, or — for cross-school sharing — nothing at all.
//!
//! # A warning the user learns to dismiss is worse than none
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §8 risk **B**, recorded for three warnings and
//! naming their wording as a deliverable of this milestone rather than an afterthought of it.
//! The three are [`shared_entities`], [`indeterminate_overlap`] and [`unknown_attendance`],
//! and each ADR specifies the shape its message must take. They are written to that shape:
//! the entity by name, the consequence in terms of what will not be checked, and the remedy —
//! or, where there is none, no remedy rather than an invented one.
//!
//! # The M5 boundary
//!
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) §3's
//! seven ordered rules are **not** implemented here. [`indeterminate_overlap`] reads the
//! three-state relation the problem already carries and counts what it does not settle;
//! [`unanswered_declarations`] walks the parent edge to count unanswered questions. Neither
//! decides whether two groups share a student, and neither decides whether two placements
//! clash. See [ADR 0038](../../../../docs/adr/0038-counting-unanswered-questions-is-not-evaluating-overlap.md).

use crate::scheduling::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef,
};
use crate::scheduling::index::GroupIx;
use crate::scheduling::problem::Problem;

use super::super::hierarchy::Hierarchy;
use super::{Pass, push};

/// Every warning, in a fixed order.
pub(super) fn warnings(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    shared_entities(pass, report);
    indeterminate_overlap(pass, report);
    staffing_and_audience(pass, report);
    unknown_attendance(pass, report);
    group_size_floors(pass, report);
    zero_capacity_rooms(pass, report);
    teachers_under_several_schedules(pass, report);
}

/// Every informational finding.
pub(super) fn advice(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    attendance_disagreement(pass, report);
    declared_periods(pass, report);
}

// =======================================================================================
// Cross-school sharing — ADR 0032 rule 3
// =======================================================================================

/// Entities in scope that another school also holds.
///
/// # The wording this record demands
///
/// ADR 0032 rule 3 gives the sentence: *"Ms Okafor is employed by Northgate and by Southmoor.
/// Evara cannot compare the two schools' timetables and will not detect a clash between
/// them."* It names the person, **both schools**, and the consequence — and its consequences
/// section says in as many words that a warning reading "cross-school scheduling is
/// unsupported" is one the user learns to dismiss.
///
/// # No remedy, deliberately
///
/// There is none. The remedy a block would demand — coordinate two schools' timetables — is
/// the thing Evara cannot do, which is also exactly why this does not block. Inventing a
/// remedy would be worse than omitting one.
///
/// # The other school's timetable is never read
///
/// This is a property of the **sharing** — a second active membership, a campus association
/// spanning two schools — and not of any output. ADR 0032 §5: a warning that appeared only
/// once a second timetable existed would be absent exactly while the first school was
/// generating, which is when it is needed. Nothing here could read another school's timetable
/// even if it wanted to: a [`Problem`] holds one school.
fn shared_entities(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    let this_school = &pass.problem.context.school.name;

    for teacher in &pass.problem.teachers {
        if teacher.also_at.is_empty() {
            continue;
        }
        let mut finding = Diagnostic::new(
            DiagnosticCode::SharedEntityAcrossSchools,
            EntityRef::new(EntityKind::Teacher, teacher.id, teacher.name.clone()),
            format!(
                "{} is employed by {this_school} and by {}. Evara cannot compare the two \
                 schools' timetables and will not detect a clash between them.",
                teacher.name,
                and_list(
                    &teacher
                        .also_at
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                )
            ),
        );
        for other in &teacher.also_at {
            finding = finding.with_related(EntityRef::new(
                EntityKind::School,
                other.id,
                other.name.clone(),
            ));
        }
        push(report, finding);
    }

    for room in &pass.problem.rooms {
        if room.also_at.is_empty() {
            continue;
        }
        let mut finding = Diagnostic::new(
            DiagnosticCode::SharedEntityAcrossSchools,
            EntityRef::new(EntityKind::Room, room.id, room.name.clone()),
            format!(
                "{} is shared by {this_school} and by {}. Evara cannot compare the two \
                 schools' timetables and will not detect a double booking between them. \
                 Bookings made by {this_school} are still checked against each other.",
                room.name,
                and_list(
                    &room
                        .also_at
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                )
            ),
        );
        for other in &room.also_at {
            finding = finding.with_related(EntityRef::new(
                EntityKind::School,
                other.id,
                other.name.clone(),
            ));
        }
        push(report, finding);
    }
}

/// How many entities this problem shares with another school.
pub(super) fn shared_entity_count(problem: &Problem) -> usize {
    problem
        .teachers
        .iter()
        .filter(|t| !t.also_at.is_empty())
        .count()
        + problem
            .rooms
            .iter()
            .filter(|r| !r.also_at.is_empty())
            .count()
}

// =======================================================================================
// Indeterminate group overlap — ADR 0035 §6
// =======================================================================================

/// Group pairs nothing settles, reported as arithmetic.
///
/// # One finding, not one per pair
///
/// ADR 0035 §6 is explicit that a list of pairs is the wrong report — a modest school has
/// hundreds — and that the right one is *"a count plus the number of answers that would clear
/// it"*. So this is a single aggregate finding about the school.
///
/// # What the second number is, and what it is not
///
/// The ADR's example reads *"Answering 'do these overlap?' on 7 parent groups … resolves 412
/// of them."* Computing **412** means deciding which pairs rule 6 would settle, which is the
/// evaluator M5 owns. So this reports the number of unanswered questions and does **not**
/// claim how many pairs they resolve:
///
/// > Answering it on 7 parent groups would narrow this.
///
/// rather than
///
/// > Answering it on 7 parent groups resolves 412 of them.
///
/// That is the honest form available without implementing the evaluator, and it is what the
/// brief requires when the precise count cannot be established. The message also says that
/// membership information may still be needed, because a declaration is not the only way a
/// pair becomes decided.
fn indeterminate_overlap(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    let pairs = pass.problem.overlap.indeterminate_pairs();
    if pairs == 0 {
        return;
    }

    let unanswered = unanswered_declarations(pass.problem);
    let school = &pass.problem.context.school;

    let arithmetic = if unanswered.is_empty() {
        // Every parent with children has answered, and pairs remain. Those pairs have no
        // common parent to declare about — ADR 0035 truth table 17: two groups in different
        // year groups share no ancestor, and *unrelated is not disjoint*. So no declaration
        // can settle them and saying "answer a declaration" would send the user to a field
        // that cannot help.
        "Every parent group has already answered, so the pairs that remain are ones with no \
         common parent to declare about. Only the memberships themselves would settle those."
            .to_owned()
    } else {
        let names: Vec<String> = unanswered
            .iter()
            .take(4)
            .map(|ix| pass.problem.groups[ix.get()].name.clone())
            .collect();
        let more = unanswered.len().saturating_sub(names.len());
        let listed = if more == 0 {
            and_list(&names)
        } else {
            format!("{}, and {more} more", names.join(", "))
        };
        // The arithmetic ADR 0038 deferred, now that rule 6 exists to produce it. Still
        // "would settle" rather than "resolves": the figure is exact for the hypothesis that
        // every open declaration answers *disjoint*, and a parent that answers *may overlap*
        // has answered without settling anything.
        let settled = crate::scheduling::overlap::resolvable_by_declaration(
            &pass.problem.groups,
            &pass.problem.overlap,
        );
        let effect = if settled == 0 {
            String::from(
                "would narrow this; some pairs may still need membership information as well.",
            )
        } else if settled == pairs {
            String::from("would settle all of them.")
        } else {
            format!(
                "would settle {settled} of them; the other {} have no common parent to \
                 declare about, so only the memberships themselves will settle those.",
                pairs - settled
            )
        };
        format!(
            "Answering \"do these groups share students?\" on {} parent group{} — {listed} — \
             {effect}",
            unanswered.len(),
            if unanswered.len() == 1 { "" } else { "s" }
        )
    };

    let mut finding = Diagnostic::new(
        DiagnosticCode::GroupOverlapIndeterminate,
        EntityRef::new(EntityKind::School, school.id, school.name.clone()),
        format!(
            "{pairs} pair{} of {}'s student groups cannot be compared: Evara does not know \
             whether they share students. Until they can be, lessons for those groups may be \
             timetabled at the same time. {arithmetic}",
            if pairs == 1 { "" } else { "s" },
            school.name
        ),
    )
    .with_remedy(if unanswered.is_empty() {
        // No parent is left to answer, so the only lever is the memberships themselves.
        "Record the memberships of the groups involved, so Evara can see which students \
         they share."
            .to_owned()
    } else {
        "Answer whether each parent group's children share students, or record the \
         memberships that settle it."
            .to_owned()
    });
    for ix in unanswered.iter().take(8) {
        let group = &pass.problem.groups[ix.get()];
        finding = finding.with_related(EntityRef::new(
            EntityKind::StudentGroup,
            group.id,
            group.name.clone(),
        ));
    }
    push(report, finding);
}

/// Parent groups that have children and have not answered whether those children overlap.
///
/// # This is hierarchy information, not overlap evaluation
///
/// It reads two stored facts: the parent edge, and
/// [`GroupFact::children_are_disjoint`](crate::scheduling::GroupFact). It counts **distinct
/// unanswered declarations**, which is a property of a *group*, and never asks whether two
/// groups share a student, which is a property of a *pair* and is ADR 0035 §3's function.
///
/// A group with fewer than two children is excluded: a declaration about one child settles no
/// pair, so asking for it would send the user to a field that cannot help.
///
/// Returned in group-index order so a message built from it is deterministic. M5 may reuse
/// this rather than restating the walk.
pub(super) fn unanswered_declarations(problem: &Problem) -> Vec<GroupIx> {
    // Reads the one shared hierarchy rather than counting children here. ADR 0038 §3 asks
    // M5's rule 6 to reuse this walk, and the way to honour that is for both to read one
    // implementation — "two readings of a hierarchy are two places for a MAX_GROUP_DEPTH
    // assumption to differ".
    Hierarchy::of(&problem.groups).unanswered_parents(&problem.groups)
}

// =======================================================================================
// Staffing and audience
// =======================================================================================

/// Sections with nobody to teach them, or nobody to teach.
fn staffing_and_audience(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (_, section) in pass.sections() {
        let subject = || EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone());

        if section.teachers.is_empty() {
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::SectionWithoutTeacher,
                    subject(),
                    format!(
                        "{} has nobody assigned to teach it. Its lessons can still be placed, \
                         and no teacher's time is reserved for them.",
                        section.name
                    ),
                )
                .with_remedy(format!("Assign a teacher to {}.", section.name)),
            );
        }

        // ADR 0035 §7: `class_enrolment` has no scheduling meaning, so a section taught to no
        // group has an empty audience and nothing protects its students from clashes — even
        // when the roster names twenty of them.
        if section.audience.is_empty() {
            let roster = if section.individually_enrolled > 0 {
                format!(
                    " It names {} student{} individually, and an individual roster carries no \
                     scheduling meaning.",
                    section.individually_enrolled,
                    if section.individually_enrolled == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            } else {
                String::new()
            };
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::SectionWithoutAttendingGroup,
                    subject(),
                    format!(
                        "{} has no attending student group, so its students are not protected \
                         from being timetabled against their other lessons.{roster}",
                        section.name
                    ),
                )
                .with_remedy(format!("Add a student group to {}.", section.name)),
            );
        }
    }
}

// =======================================================================================
// Attendance — ADR 0034
// =======================================================================================

/// Sections whose room size will not be checked.
///
/// # The wording this record demands
///
/// ADR 0034 §5 gives the shape: *"12 class sections have no expected attendance and no
/// complete set of declared group sizes. Room sizes will not be checked for them. Enter an
/// expected attendance, or declare a size for Year 9 Set 1 and Year 9 Set 2."*
///
/// Reported **per section** rather than aggregated, because the remedy is per section and the
/// fixtures record one expectation per affected section. The message names which groups are
/// missing a size where that is the reason, so the second half of the remedy is specific.
///
/// # Not evaluated is a third outcome
///
/// ADR 0034 §4: `room-capacity` is not evaluated for such an activity — not satisfied, not
/// violated. The message says what will not be checked rather than implying a pass, and
/// nothing here treats a missing attendance as zero.
fn unknown_attendance(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (_, section) in pass.sections() {
        if section.expected_attendance.is_known() {
            continue;
        }

        let sizeless: Vec<String> = section
            .audience
            .iter()
            .filter(|ix| pass.problem.groups[ix.get()].declared_size.is_none())
            .map(|ix| pass.problem.groups[ix.get()].name.clone())
            .collect();

        let because = if section.audience.is_empty() {
            "it has no attending group to derive one from".to_owned()
        } else if sizeless.is_empty() {
            "its groups' sizes cannot be added together — Evara cannot prove they do not \
             share students"
                .to_owned()
        } else {
            format!("no size is declared for {}", and_list(&sizeless))
        };

        let remedy = if sizeless.is_empty() {
            format!("Enter an expected attendance for {}.", section.name)
        } else {
            format!(
                "Enter an expected attendance for {}, or declare a size for {}.",
                section.name,
                and_list(&sizeless)
            )
        };

        let mut finding = Diagnostic::new(
            DiagnosticCode::ExpectedAttendanceUnknown,
            EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
            format!(
                "How many people attend {} is not known, because {because}. Room sizes will \
                 not be checked for its lessons.",
                section.name
            ),
        )
        .with_remedy(remedy);

        if section.attendance_floor.is_informative() {
            finding = Diagnostic::new(
                DiagnosticCode::ExpectedAttendanceUnknown,
                EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
                format!(
                    "How many people attend {} is not known, because {because}. At least {} \
                     are listed in its groups, which is a floor and not a measurement. Room \
                     sizes will not be checked for its lessons.",
                    section.name, section.attendance_floor.at_least
                ),
            )
            .with_remedy(
                finding
                    .remedy
                    .clone()
                    .unwrap_or_else(|| "Enter an expected attendance.".to_owned()),
            );
        }
        push(report, finding);
    }
}

/// Groups that declare no size but list members, so only a floor is available.
///
/// ADR 0034 §3: the floor *"may raise a warning and may never decide a hard constraint"*,
/// because a bound that can only be too small cannot prove a room too small.
fn group_size_floors(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for group in &pass.problem.groups {
        if group.declared_size.is_some() || group.effective_members == 0 {
            continue;
        }
        push(
            report,
            Diagnostic::new(
                DiagnosticCode::GroupSizeUnknownWithMembers,
                EntityRef::new(EntityKind::StudentGroup, group.id, group.name.clone()),
                format!(
                    "{} declares no size, and {} student{} listed in it. That is a floor \
                     rather than a count, so it can raise a warning about a room and cannot \
                     decide one.",
                    group.name,
                    group.effective_members,
                    if group.effective_members == 1 {
                        " is"
                    } else {
                        "s are"
                    }
                ),
            )
            .with_remedy(format!("Declare a size for {}.", group.name)),
        );
    }
}

/// Bookable rooms recorded as holding nobody.
///
/// ADR 0034 §5: *"`room.capacity = 0` means zero seats, and a `Known` check against it
/// fails. Nothing reads 0 as 'not entered'."* So this names a suspicious number and does not
/// reinterpret it — the stored value stands.
fn zero_capacity_rooms(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for room in &pass.problem.rooms {
        if !room.is_bookable || room.capacity != 0 {
            continue;
        }
        push(
            report,
            Diagnostic::new(
                DiagnosticCode::BookableRoomZeroCapacity,
                EntityRef::new(EntityKind::Room, room.id, room.name.clone()),
                format!(
                    "{} is bookable and recorded as holding 0 people. Evara uses the number \
                     as entered and will not treat it as unset.",
                    room.name
                ),
            )
            .with_remedy(format!(
                "If that is not deliberate, enter {}'s capacity.",
                room.name
            )),
        );
    }
}

/// Teachers who teach under more than one bell schedule in scope.
///
/// ADR 0033 §7's named limitation: a `period_id`-scoped availability rule belongs to one
/// schedule, so *"a shared teacher's period-level unavailability must be entered once per
/// bell schedule they teach under"*, and the omission has to be visible before a solve rather
/// than discovered in the finished timetable.
fn teachers_under_several_schedules(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (ix, teacher) in pass.problem.teachers.iter().enumerate() {
        let teacher_ix =
            crate::scheduling::index::TeacherIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
        let mut schedules: Vec<crate::scheduling::index::BellScheduleIx> = Vec::new();
        for (section_ix, section) in pass.sections() {
            if !section.teachers.contains(&teacher_ix) {
                continue;
            }
            if let Some(schedule) = pass.schedule_of(section_ix)
                && !schedules.contains(&schedule)
            {
                schedules.push(schedule);
            }
        }
        if schedules.len() < 2 {
            continue;
        }

        let names: Vec<String> = schedules
            .iter()
            .map(|s| pass.problem.context.bell_schedules[s.get()].name.clone())
            .collect();
        let mut finding = Diagnostic::new(
            DiagnosticCode::TeacherUnderSeveralBellSchedules,
            EntityRef::new(EntityKind::Teacher, teacher.id, teacher.name.clone()),
            format!(
                "{} teaches under {} bell schedules: {}. An unavailability entered against a \
                 period of one of them does not apply to the others.",
                teacher.name,
                schedules.len(),
                and_list(&names)
            ),
        )
        .with_remedy(format!(
            "Enter {}'s unavailability once per bell schedule they teach under, or enter it \
             for whole cycle days, which applies to all of them.",
            teacher.name
        ));
        for schedule in &schedules {
            let bells = &pass.problem.context.bell_schedules[schedule.get()];
            finding = finding.with_related(EntityRef::new(
                EntityKind::BellSchedule,
                bells.id,
                bells.name.clone(),
            ));
        }
        push(report, finding);
    }
}

// =======================================================================================
// Informational: two legitimate figures that disagree
// =======================================================================================

/// A stated attendance that the section's groups' sizes do not add up to.
///
/// ADR 0034 §3: the explicit value **wins** and the two are never reconciled — *"a school
/// that states 24 for a section taught to a group of 28 has said something … and a derivation
/// that silently overrode it would be refusing a true statement."* So this is advice, nothing
/// is overridden, and nothing is refused.
///
/// Only reported where the comparison is meaningful: every attending group declares a size,
/// so there is a figure to compare against.
fn attendance_disagreement(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (_, section) in pass.sections() {
        let Some(stated) = section.expected_attendance.people() else {
            continue;
        };
        if section.audience.is_empty() {
            continue;
        }
        let mut declared_total: i64 = 0;
        for ix in &section.audience {
            match pass.problem.groups[ix.get()].declared_size {
                Some(size) => declared_total += size,
                None => return_if_unsized(&mut declared_total),
            }
        }
        // A group with no declared size makes the sum meaningless; `return_if_unsized` marks
        // it with a sentinel the comparison then skips.
        if declared_total < 0 || declared_total == stated {
            continue;
        }

        let groups: Vec<String> = section
            .audience
            .iter()
            .map(|ix| pass.problem.groups[ix.get()].name.clone())
            .collect();
        let difference = (declared_total - stated).abs();
        let direction = if stated < declared_total {
            "fewer"
        } else {
            "more"
        };

        push(
            report,
            Diagnostic::new(
                DiagnosticCode::ExpectedAttendanceDisagreesWithGroups,
                EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
                format!(
                    "{} expects {stated} people, which is {difference} {direction} than the \
                     {declared_total} declared across {}. Evara uses the figure you entered; \
                     this is only worth knowing.",
                    section.name,
                    and_list(&groups)
                ),
            ),
        );
    }
}

/// Marks a sum as meaningless by driving it negative.
///
/// A sentinel rather than an `Option` because the loop that fills it has to keep going to
/// build the group list, and an early `return` from the caller would skip the rest of the
/// sections.
fn return_if_unsized(total: &mut i64) {
    *total = -1;
}

/// A course's declared periods per cycle against its section's actual demand.
///
/// ADR 0029 §3 keeps the declaration as **advice**: *"a section may legitimately need more or
/// less than its course's default"*, and the interface reports the difference rather than
/// refusing it.
fn declared_periods(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (section_ix, section) in pass.sections() {
        let Some(declared) = section.declared_periods_per_cycle else {
            continue;
        };
        let actual: i64 = pass
            .problem
            .activities
            .iter()
            .filter(|a| a.section == section_ix)
            .map(|a| i64::from(a.duration.get()))
            .sum();
        if actual == declared || actual == 0 {
            continue;
        }
        let (word, difference) = if actual > declared {
            ("more", actual - declared)
        } else {
            ("fewer", declared - actual)
        };
        push(
            report,
            Diagnostic::new(
                DiagnosticCode::DeclaredPeriodsDisagree,
                EntityRef::new(EntityKind::ClassSection, section.id, section.name.clone()),
                format!(
                    "{} has {actual} periods a cycle, {difference} {word} than the \
                     {declared} its course declares. Both are allowed; the lessons entered \
                     are what will be timetabled.",
                    section.name
                ),
            ),
        );
    }
}

/// Joins names into a readable English list: `"A"`, `"A and B"`, `"A, B and C"`.
///
/// A diagnostic that reads as a sentence has to join its lists like one. Used by the three
/// load-bearing warnings, whose wording is a deliverable.
fn and_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
