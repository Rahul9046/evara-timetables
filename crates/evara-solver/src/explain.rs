//! Why a lesson is not on the timetable, said in a way somebody can act on.
//!
//! # The claim this module is careful not to make
//!
//! `evara_core::lessons` carries a rule the whole workspace inherits: **never report that a
//! demand can be scheduled.** A solver needs its mirror. A greedy construction that leaves
//! twelve lessons unplaced has discovered that *its* arrangement had no room for them, and
//! nothing more — a different seed, a different order, or Phase B's local search may well place
//! every one. Telling a school *"this cannot be timetabled"* on that evidence would spend the
//! user's trust on a guess, which is the failure `SOLVER.md` §8 opens by naming.
//!
//! So each unplaced occurrence gets one of two kinds of sentence, and
//! [`UnplacedCause::is_impossible`] is the one place the difference is decided:
//!
//! | Kind | Phrasing | Example cause |
//! | --- | --- | --- |
//! | A property of the **problem** | *"cannot be placed by any timetable"* | no run of three consecutive teaching periods exists |
//! | A property of this **run** | *"the constructor could not place it"* | every candidate clashed with something already placed |
//!
//! # A pre-solve finding is attached where there is one
//!
//! [`UnscheduledActivity::diagnostic`] exists so an occurrence that was *already* known to be
//! impossible carries the finding that said so, rather than a second wording of it. M3's
//! validator and M4's loader name the section or the activity in the finding's subject, which is
//! what the match is on.
//!
//! The field's own documentation states what the two cases mean: `None` means *"the search
//! simply ran out of room for it — a difficulty rather than an impossibility"*, `Some(_)` means
//! pre-flight had already proved it. Honouring that is a property of the **match**, not of the
//! cause: a finding is attached only when it is one of the four codes that prove impossibility
//! **and** its subject is this lesson or its section. So a stray warning about the same section
//! can never be attached, and a lesson the constructor merely failed on gets `None` because
//! nothing proved anything about it.
//!
//! This is why a refused run can still attach one. `infeasible-duration`'s occurrence is
//! genuinely impossible and `duration-exceeds-longest-run` says so; the run was refused at the
//! gate, so its *cause* is [`UnplacedCause::InputRejected`] rather than
//! [`UnplacedCause::NoContiguousRun`] — the engine established nothing itself — and the finding
//! that did establish it is carried beside it.

use evara_core::scheduling::{
    ActivityIx, Diagnostic, DiagnosticCode, DiagnosticReport, Problem, Severity, UnplacedCause,
    UnscheduledActivity,
};
use uuid::Uuid;

/// Builds the unplaced list a result carries.
///
/// In activity-index order, matching the order the construction reports them in, so two runs of
/// the same solve produce byte-identical reports.
#[must_use]
pub fn unplaced_report(
    problem: &Problem,
    unplaced: &[(ActivityIx, UnplacedCause)],
    preflight: &DiagnosticReport,
) -> Vec<UnscheduledActivity> {
    unplaced
        .iter()
        .filter_map(|(activity, cause)| {
            let fact = problem.activities.get(activity.get())?;
            let section = problem.sections.get(fact.section.get());
            let section_name = section.map_or_else(
                || String::from("an unknown class"),
                |section| section.name.clone(),
            );

            Some(UnscheduledActivity {
                activity: *activity,
                id: fact.id,
                section: section_name.clone(),
                cause: *cause,
                reason: reason(&section_name, fact.duration.as_usize(), *cause),
                diagnostic: already_said(preflight, fact.id, section.map(|s| s.id)),
            })
        })
        .collect()
}

/// The sentence for one cause.
///
/// Each names the remedy where there is one, and the three that are about this run say so
/// explicitly rather than leaving the reader to infer it.
fn reason(section: &str, duration: usize, cause: UnplacedCause) -> String {
    let periods = if duration == 1 {
        String::from("one period")
    } else {
        format!("{duration} consecutive periods")
    };

    match cause {
        UnplacedCause::NoResolvedBellSchedule => format!(
            "{section} does not resolve to a single bell schedule, so this lesson has no slots \
             to be placed in at all. Name the section's campus, or make a bell schedule \
             school-wide."
        ),
        UnplacedCause::NoContiguousRun => format!(
            "{section} needs {periods} and no run that long exists anywhere in its bell \
             schedule, so no timetable can place it. Shorten the lesson, or add a period."
        ),
        UnplacedCause::NoCandidateRoom => format!(
            "{section} requires a kind of room that no bookable room of its campus is, so no \
             timetable can place it. Add a room of that type, make an existing one bookable, \
             or change what the course requires."
        ),
        UnplacedCause::AllCandidatesUnavailable => format!(
            "every slot that would fit {periods} is one a stored availability rule marks \
             unavailable for {section}'s teacher, its students or every room it could use. \
             Review those restrictions."
        ),
        UnplacedCause::BlockedByPlacements => format!(
            "the constructor could not place {section} here: every slot that would fit \
             {periods} clashed with a lesson it had already placed. This is not a proof that \
             no complete timetable exists — try another seed, or place it by hand."
        ),
        UnplacedCause::NotReached => format!(
            "generation stopped before reaching {section}. Nothing has been established about \
             whether it can be placed; run again with a larger budget."
        ),
        UnplacedCause::InputRejected => format!(
            "generation was refused before it began, because this school's scheduling inputs \
             carry a blocking finding, so nothing was attempted for {section}. Correct the \
             findings — `evara validate` lists them — and generate again."
        ),
    }
}

/// The pre-solve finding that already explained this occurrence, where there is one.
///
/// Matches on the finding's subject identifier — the activity's or its section's — because that
/// is what M3 and M4 put there, and on the code. Only the codes that prove impossibility are
/// considered: a warning about the same section is not an explanation of why a lesson went
/// unplaced, and attaching one would read as a cause.
fn already_said(
    preflight: &DiagnosticReport,
    activity: Uuid,
    section: Option<Uuid>,
) -> Option<Diagnostic> {
    preflight
        .findings
        .iter()
        .find(|finding| {
            finding.severity() == Severity::Blocking
                && proves_impossibility(finding.code)
                && (finding.subject.id == activity || Some(finding.subject.id) == section)
        })
        .cloned()
}

/// Whether a diagnostic code is one that proves an occurrence cannot be placed.
///
/// The four of M3's and M4's codes whose argument is *arithmetic or an absent value range*, as
/// opposed to a counting argument about the school as a whole. A `teacher-load-exceeds-slots`
/// finding proves **some** lesson cannot be placed and not **this** one, so it is not here: it
/// would attach a true statement to an occurrence it is not about, which
/// `PRE-SOLVE-VALIDATION.md` §5 treats as worse than no finding — it sends the user to fix
/// something that is not broken.
const fn proves_impossibility(code: DiagnosticCode) -> bool {
    matches!(
        code,
        DiagnosticCode::NoBellSchedule
            | DiagnosticCode::AmbiguousBellSchedule
            | DiagnosticCode::DurationExceedsLongestRun
            | DiagnosticCode::EmptyCandidateSet
    )
}
