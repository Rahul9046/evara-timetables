//! Structured, machine-readable findings about a scheduling problem.
//!
//! # The one rule the whole vocabulary encodes
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2 classifies every incomplete or unknown input
//! into one of four outcomes, and reading its table as a whole it says one thing:
//!
//! > **A hard error is reserved for an input whose remedy is a field the user can fill in;
//! > everything else is counted and announced; nothing is assumed.**
//!
//! That is why [`Severity::Blocking`] and [`Severity::Warning`] are not two shades of the
//! same thing. An unresolved bell schedule refuses the solve, because the remedy is one
//! field and without it there is no value range at all
//! ([ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §3). A teacher shared with another school only warns, because the remedy — coordinate two
//! schools' timetables — is the thing Evara cannot do
//! ([ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! §5). Refusing where the user can answer and warning where they cannot is one rule applied
//! throughout.
//!
//! # A warning the user learns to dismiss is worse than none
//!
//! Risk **B** of `SCHEDULING-ARCHITECTURE-DECISIONS.md` §8, recorded for three separate
//! warnings. The mitigation is that the wording is a deliverable rather than an
//! afterthought: a diagnostic must name the entity, state the consequence, and give the
//! remedy. That is what [`Diagnostic::remedy`] is for, and why [`Diagnostic::subject`] is a
//! structured reference rather than a name embedded in prose.
//!
//! # What this module does not do
//!
//! **No rule is implemented here.** M3 is the milestone that populates these; M2 declares
//! the contract so that M3 writes findings rather than inventing codes. Nothing in this
//! module inspects a [`Problem`](super::Problem).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The remedy is a field the user can fill in, and until they do the solve cannot
    /// produce a meaningful answer.
    ///
    /// Most blocking findings refuse the whole solve; a few — a duration longer than any
    /// teaching run, a required room type with no room — say that one activity cannot be
    /// placed **by any solver**, which is arithmetic rather than difficulty. Each code's
    /// documentation says which it is.
    Blocking,
    /// Something real is not being checked, or is not known.
    ///
    /// Never blocks. Always counted, and carried onto the generated timetable so that it is
    /// described as having no *confirmed* clash rather than as conflict-free.
    Warning,
    /// Worth saying, and not a problem.
    ///
    /// Where two stored figures disagree and both are legitimate — a course's declared
    /// periods against its section's actual demand (ADR 0029 §3), a stated attendance
    /// against a group's size (ADR 0034 §3) — the difference is advice. Nothing is
    /// overridden and nothing is refused.
    Info,
}

impl Severity {
    /// Whether a finding of this severity stops a solve from being worth running.
    #[must_use]
    pub const fn blocks_solve(self) -> bool {
        matches!(self, Self::Blocking)
    }
}

/// What kind of thing a finding is about.
///
/// One variant per table a diagnostic can name. Deliberately wider than the first
/// constraint set: a finding may be about a resource or a student even though neither is
/// scheduled in the MVP, because *"this stored rule names something nothing schedules"* is
/// precisely a thing worth reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    /// The school being solved.
    School,
    /// A campus of it.
    Campus,
    /// The cycle.
    Cycle,
    /// A position in the cycle.
    CycleDay,
    /// A term.
    Term,
    /// A bell schedule (`period_structure`).
    BellSchedule,
    /// A period of a bell schedule.
    Period,
    /// A materialised slot.
    Timeslot,
    /// A room.
    Room,
    /// A kind of room.
    RoomType,
    /// A teacher.
    Teacher,
    /// A student group.
    StudentGroup,
    /// An individual student.
    Student,
    /// A subject.
    Subject,
    /// A course.
    Course,
    /// A class section.
    ClassSection,
    /// One placeable occurrence.
    Activity,
    /// A stored availability rule.
    AvailabilityRule,
    /// A bookable resource.
    Resource,
    /// A year level.
    ///
    /// Appended rather than placed beside [`EntityKind::StudentGroup`], where it belongs by
    /// subject: the enum derives `Ord`, several reports sort by it, and a variant inserted in
    /// the middle would reorder findings that nothing had changed. Added for `V16`'s
    /// `constraint_ref`, whose `entity_kind` lists it.
    YearLevel,
}

/// A thing a finding names, by stored identity.
///
/// Structured rather than interpolated into prose, for two reasons. The interface makes
/// these clickable — `SOLVER.md` §6's `Explanation` carries `entities: Vec<EntityRef>` for
/// exactly that — and a count of findings grouped by subject is only possible if the subject
/// is a field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRef {
    /// Which table.
    pub kind: EntityKind,
    /// The row's stable identity.
    pub id: Uuid,
    /// What to call it, so a report need not re-read the database to render a name.
    pub label: String,
}

impl EntityRef {
    /// Builds a reference.
    #[must_use]
    pub fn new(kind: EntityKind, id: Uuid, label: impl Into<String>) -> Self {
        Self {
            kind,
            id,
            label: label.into(),
        }
    }
}

/// A stable identifier for a kind of finding.
///
/// # Stability is the contract
///
/// [`DiagnosticCode::key`] is a kebab-case string that is part of the contract between the
/// engine, the interface and any future test baseline. A renamed key silently invalidates a
/// user's dismissals and every assertion written against it, so these are treated like a
/// constraint's `type_key`: added to, never renamed.
///
/// Every variant below is grounded in an accepted record — `SCHEDULING-ARCHITECTURE-DECISIONS.md`
/// §3.2's classification table, or `SOLVER.md` §8's Layer-1 checks. None is speculative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticCode {
    // ---------------------------------------------------------------------------------
    // Blocking: the remedy is a field, and without it there is nothing to solve
    // ---------------------------------------------------------------------------------
    /// No grid has been materialised for this cycle and term.
    ///
    /// `G` is empty, so no activity has any candidate slot. **Refuses the solve, once**,
    /// before any activity is examined — not once per section (ADR 0033 §3, test 9).
    ///
    /// *Remedy: build a grid under Timetable Grid Preview before generating.*
    NoGridMaterialised,
    /// A class section resolves to no bell schedule.
    ///
    /// **Refuses the solve.** An activity that cannot be placed anywhere is not a
    /// scheduling difficulty, it is an unanswered setup question. The finding carries one of
    /// [`NoBellScheduleCause`](super::NoBellScheduleCause)'s three causes, because each has
    /// a different remedy (ADR 0033 §3).
    NoBellSchedule,
    /// A class section resolves to more than one bell schedule.
    ///
    /// **Refuses the solve.** Evara will not choose. The finding names **every** candidate
    /// by name, and offers two remedies: build the grid from one of them, or restrict the
    /// generation to one of them (ADR 0033 §3).
    AmbiguousBellSchedule,
    /// An occurrence needs more consecutive teaching periods than its resolved bell
    /// schedule offers in a row.
    ///
    /// Arithmetic, and conclusive: **no placement exists, in any timetable, by any solver.**
    /// A run never spans two bell schedules, so another schedule in the school offering a
    /// longer run is irrelevant (ADR 0033 §5, test 10).
    ///
    /// *Remedy: shorten the occurrence, or lengthen the run of teaching periods.*
    ///
    /// This is the same measure `evara_core::lessons::DemandFinding::OccurrenceLongerThanAnyRun`
    /// already reports per section, and the two agree by construction — but that one takes
    /// the school-wide **maximum** across schedules, so it can only under-report and will
    /// stay silent where this fires. That is correct and deliberate: never report that a
    /// demand *can* be scheduled.
    DurationExceedsLongestRun,
    /// A section requires a kind of room, and no room the section could use is of that kind.
    ///
    /// Conclusive for that activity. `course.required_room_type_id` is the only room
    /// requirement that exists, and the candidate set is rooms reachable from this school,
    /// narrowed to the section's campus where it names one, bookable, and of the required
    /// type — so an empty set is a counting fact rather than a difficulty.
    /// `SOLVER.md` §8 Layer 1, *room-type demand vs. supply*.
    ///
    /// *Remedy: add a room of that type at that campus, make an existing one bookable, or
    /// clear the requirement.*
    NoRoomOfRequiredType,
    /// More lessons need a kind of room than every room of that kind could hold.
    ///
    /// `SOLVER.md` §8 Layer 1, *room-type demand vs. supply*. A counting argument over
    /// period-slots: a room hosts at most one lesson per slot, so the supply of a room type is
    /// bounded by `eligible rooms × usable teaching slots`, and demand beyond that cannot be
    /// met by any timetable.
    ///
    /// Distinct from [`DiagnosticCode::NoRoomOfRequiredType`], which is the **degenerate**
    /// case of the same inequality — supply of zero. They are kept apart because the remedies
    /// differ: one needs a room of that kind to exist at all, the other needs more of them,
    /// fewer lessons requiring them, or more slots.
    ///
    /// **Added in M3.** M2 declared the degenerate case only, and the non-degenerate shortfall
    /// had no code to be reported under.
    RoomTypeDemandExceedsSupply,
    /// An activity's candidate set of `(timeslot, room)` pairs is empty for some other
    /// reason.
    ///
    /// `SOLVER.md` §8 Layer 1 names the usual cause: *"often a single bad availability
    /// entry"*. Distinct from [`DiagnosticCode::NoRoomOfRequiredType`] so the remedy can be
    /// specific rather than a shrug.
    EmptyCandidateSet,
    /// A teacher is assigned more periods per cycle than the grid offers them.
    ///
    /// `SOLVER.md` §8 Layer 1, *teacher load vs. available teaching slots, per teacher and
    /// per term*. A counting argument, so it catches a genuinely impossible setup rather
    /// than a hard one.
    TeacherLoadExceedsSlots,
    /// A student group's lessons demand more periods per cycle than the grid offers it.
    ///
    /// `SOLVER.md` §8 Layer 1, *group period budget vs. slots available to that group*, and
    /// the source of that section's example: *"Year 9 needs 42 periods per cycle but only 40
    /// teaching slots exist. Reduce a course's periods per cycle, or add a period."*
    GroupDemandExceedsSlots,
    /// A teacher is assigned to a section at a campus they are not eligible for.
    ///
    /// # This should be unreachable
    ///
    /// Employment and campus eligibility are **database invariants** under two trigger pairs
    /// (`V15:507`, [ADR 0028](../../../../docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md)),
    /// so the rows cannot exist. The code is declared so that a loader which encounters one
    /// anyway reports a damaged project instead of silently building a timetable on it —
    /// the same reasoning as `DbError::CorruptIdentifier`. It is **not** a check the solver
    /// performs in place of the triggers.
    TeacherNotEligibleForCampus,

    // ---------------------------------------------------------------------------------
    // Warning: something real is not being checked, and the remedy is a task or absent
    // ---------------------------------------------------------------------------------
    /// An entity in scope is also held by another school.
    ///
    /// A teacher with a second active membership, a room or resource whose campus
    /// associations reach two schools' campuses, a student with concurrent enrolments. ADR
    /// 0032 rule 3: **detect and report, never block and never guess.**
    ///
    /// The finding must name the entity, **both schools** and the consequence — *"Ms Okafor
    /// is employed by Northgate and by Southmoor. Evara cannot compare the two schools'
    /// timetables and will not detect a clash between them."* A warning that said
    /// "cross-school scheduling is unsupported" is one the user learns to dismiss.
    ///
    /// It is a property of the **sharing**, not of the other school's timetable, which is
    /// never read — whether it is absent, a draft, or several (ADR 0032 §5, tests 6 and 7).
    SharedEntityAcrossSchools,
    /// Two attending groups cannot be compared: nothing stored says whether they share a
    /// student.
    ///
    /// [`Overlap::Indeterminate`](super::Overlap::Indeterminate), and **not enforced**. ADR
    /// 0035 §6 requires the report to be actionable arithmetic rather than a list of pairs:
    /// a count, plus the number of answers that would clear it — *"431 pairs of this
    /// school's attending groups cannot be compared … Answering 'do these overlap?' on 7
    /// parent groups resolves 412 of them. Until then, lessons for those groups may be
    /// timetabled at the same time."*
    GroupOverlapIndeterminate,
    /// A member row contradicts a parent's disjointness declaration.
    ///
    /// A student is in two groups whose parent declares its children disjoint. **Facts win**:
    /// the pair is [`Overlap::Overlaps`](super::Overlap::Overlaps) — ADR 0035 §3 puts rule 3
    /// before rule 6 — and the contradiction is named with the student, both groups and the
    /// parent whose declaration they break. Silently honouring either side would hide a data
    /// error that matters.
    GroupOverlapContradiction,
    /// A class section has no attending group, so its students are not protected from
    /// clashes.
    ///
    /// `class_enrolment` has no scheduling meaning in Milestone 1, so a section taught to no
    /// group has an empty audience and `student-clash` has nothing to say about it — even
    /// when twenty students are named on its roster (ADR 0035 §7).
    ///
    /// *Remedy: add a student group.*
    SectionWithoutAttendingGroup,
    /// A class section has nobody assigned to teach it.
    ///
    /// Does not block: `MEDIUM` is the level for "must be satisfied unless the alternative
    /// is an infeasible timetable", and `SOLVER.md` §3 names unstaffed classes among exactly
    /// those. The lessons are still placed and the gap is still reported.
    SectionWithoutTeacher,
    /// A section's expected attendance is unknown, so room sizes will not be checked for it.
    ///
    /// `room-capacity` is **not evaluated** for such an activity — not satisfied, not
    /// violated (ADR 0034 §4). Pre-flight states it per section with the remedy: *"Enter an
    /// expected attendance, or declare a size for Year 9 Set 1 and Year 9 Set 2."*
    ExpectedAttendanceUnknown,
    /// A group declares no size but lists members, so only a floor on its population is
    /// available.
    ///
    /// The floor may raise a warning — *"Lab 2 seats 24 and at least 26 students are listed
    /// in this section's groups"* — and may **never** decide a hard constraint, because a
    /// bound that can only be too small cannot prove a room too small (ADR 0034 §3).
    ///
    /// *Remedy: declare the group's size.*
    GroupSizeUnknownWithMembers,
    /// A bookable room has a capacity of zero.
    ///
    /// The stored number is used **as written**: `0` means zero seats and is never
    /// reinterpreted as "not entered". This finding exists because a suspicious number is
    /// worth naming even when it is not reinterpreted — *"4 bookable rooms have a capacity
    /// of 0. If that is not deliberate, enter their capacity."* (ADR 0034 §5).
    BookableRoomZeroCapacity,
    /// A teacher teaches under more than one bell schedule in scope.
    ///
    /// Their period-level unavailability does not travel between schedules: a `period` row
    /// belongs to one `period_structure`, so "period 1, every day" says something about one
    /// schedule only and the loader widens nothing (ADR 0033 §7).
    ///
    /// *Remedy: enter the unavailability once per bell schedule they teach under.*
    TeacherUnderSeveralBellSchedules,
    /// An availability rule names an entity that no longer exists.
    ///
    /// `availability.entity_id` is polymorphic and so carries no foreign key — the one place
    /// in the schema where a reference cannot be enforced by the database (`V11`'s header).
    /// The repository sweeps for these; a solve reports any it finds rather than compiling a
    /// rule about nothing.
    OrphanAvailabilityReference,
    /// An availability rule names a kind of entity nothing schedules.
    ///
    /// `RESOURCE` and `SUBJECT` are stored by the column and compiled into no index: nothing
    /// links a resource to an activity (the audit's N4), and a subject-scoped rule has no
    /// agreed meaning for a placement. Reported rather than compiled, so the rule does not
    /// appear to be enforced when it is not.
    AvailabilityEntityKindNotScheduled,

    // ---------------------------------------------------------------------------------
    // Info: two legitimate stored figures disagree, and neither is overridden
    // ---------------------------------------------------------------------------------
    /// A course's declared periods per cycle differ from its section's actual demand.
    ///
    /// Advice, never a refusal. A section may legitimately need more or less than its
    /// course's default, and ADR 0029 §3 keeps the declaration as advice for exactly that
    /// reason.
    DeclaredPeriodsDisagree,
    /// A section's stated expected attendance differs from what its groups' sizes suggest.
    ///
    /// The explicit value **wins** and the two are never reconciled: a school that states 24
    /// for a section taught to a group of 28 has said something, and a derivation that
    /// overrode it would be refusing a true statement. The difference is reported as advice
    /// (ADR 0034 §3).
    ExpectedAttendanceDisagreesWithGroups,
}

impl DiagnosticCode {
    /// The stable kebab-case key.
    ///
    /// Part of the contract. Added to, never renamed — see this type's header.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::NoGridMaterialised => "no-grid-materialised",
            Self::NoBellSchedule => "no-bell-schedule",
            Self::AmbiguousBellSchedule => "ambiguous-bell-schedule",
            Self::DurationExceedsLongestRun => "duration-exceeds-longest-run",
            Self::NoRoomOfRequiredType => "no-room-of-required-type",
            Self::RoomTypeDemandExceedsSupply => "room-type-demand-exceeds-supply",
            Self::EmptyCandidateSet => "empty-candidate-set",
            Self::TeacherLoadExceedsSlots => "teacher-load-exceeds-slots",
            Self::GroupDemandExceedsSlots => "group-demand-exceeds-slots",
            Self::TeacherNotEligibleForCampus => "teacher-not-eligible-for-campus",
            Self::SharedEntityAcrossSchools => "shared-entity-across-schools",
            Self::GroupOverlapIndeterminate => "group-overlap-indeterminate",
            Self::GroupOverlapContradiction => "group-overlap-contradiction",
            Self::SectionWithoutAttendingGroup => "section-without-attending-group",
            Self::SectionWithoutTeacher => "section-without-teacher",
            Self::ExpectedAttendanceUnknown => "expected-attendance-unknown",
            Self::GroupSizeUnknownWithMembers => "group-size-unknown-with-members",
            Self::BookableRoomZeroCapacity => "bookable-room-zero-capacity",
            Self::TeacherUnderSeveralBellSchedules => "teacher-under-several-bell-schedules",
            Self::OrphanAvailabilityReference => "orphan-availability-reference",
            Self::AvailabilityEntityKindNotScheduled => "availability-entity-kind-not-scheduled",
            Self::DeclaredPeriodsDisagree => "declared-periods-disagree",
            Self::ExpectedAttendanceDisagreesWithGroups => {
                "expected-attendance-disagrees-with-groups"
            }
        }
    }

    /// The severity a finding of this code carries.
    ///
    /// Fixed per code rather than chosen per finding, because the classification is a
    /// decision (`SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2) and not a judgement call at
    /// the point of reporting. A caller that could pick its own severity could quietly
    /// downgrade a blocking finding.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::NoGridMaterialised
            | Self::NoBellSchedule
            | Self::AmbiguousBellSchedule
            | Self::DurationExceedsLongestRun
            | Self::NoRoomOfRequiredType
            | Self::RoomTypeDemandExceedsSupply
            | Self::EmptyCandidateSet
            | Self::TeacherLoadExceedsSlots
            | Self::GroupDemandExceedsSlots
            | Self::TeacherNotEligibleForCampus => Severity::Blocking,

            Self::SharedEntityAcrossSchools
            | Self::GroupOverlapIndeterminate
            | Self::GroupOverlapContradiction
            | Self::SectionWithoutAttendingGroup
            | Self::SectionWithoutTeacher
            | Self::ExpectedAttendanceUnknown
            | Self::GroupSizeUnknownWithMembers
            | Self::BookableRoomZeroCapacity
            | Self::TeacherUnderSeveralBellSchedules
            | Self::OrphanAvailabilityReference => Severity::Warning,

            Self::AvailabilityEntityKindNotScheduled
            | Self::DeclaredPeriodsDisagree
            | Self::ExpectedAttendanceDisagreesWithGroups => Severity::Info,
        }
    }

    /// Every code, in declaration order.
    ///
    /// Used by the test that asserts the keys are unique, which is the one way a renamed or
    /// copy-pasted key gets caught before it ships.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::NoGridMaterialised,
            Self::NoBellSchedule,
            Self::AmbiguousBellSchedule,
            Self::DurationExceedsLongestRun,
            Self::NoRoomOfRequiredType,
            Self::RoomTypeDemandExceedsSupply,
            Self::EmptyCandidateSet,
            Self::TeacherLoadExceedsSlots,
            Self::GroupDemandExceedsSlots,
            Self::TeacherNotEligibleForCampus,
            Self::SharedEntityAcrossSchools,
            Self::GroupOverlapIndeterminate,
            Self::GroupOverlapContradiction,
            Self::SectionWithoutAttendingGroup,
            Self::SectionWithoutTeacher,
            Self::ExpectedAttendanceUnknown,
            Self::GroupSizeUnknownWithMembers,
            Self::BookableRoomZeroCapacity,
            Self::TeacherUnderSeveralBellSchedules,
            Self::OrphanAvailabilityReference,
            Self::AvailabilityEntityKindNotScheduled,
            Self::DeclaredPeriodsDisagree,
            Self::ExpectedAttendanceDisagreesWithGroups,
        ]
    }
}

/// One finding about one thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Which kind of finding.
    pub code: DiagnosticCode,
    /// What it is about.
    pub subject: EntityRef,
    /// Anything else worth clicking through to — the other school in a sharing warning, the
    /// candidate schedules in an ambiguity, the student in a contradiction.
    pub related: Vec<EntityRef>,
    /// Prose a person can read.
    ///
    /// English, authored by the engine. `SOLVER.md` §6 settles the division of labour: *"The
    /// frontend renders and localises; it does not author the reason."* So this is not a UI
    /// string in the sense of being markup or a React concern — it is the engine's statement
    /// of what it found, and the structured fields beside it are what the interface uses to
    /// build anything richer.
    pub message: String,
    /// What the user can do about it, where there is something.
    ///
    /// `None` where no remedy exists — which is the honest answer for a cross-school sharing
    /// warning, and the reason that one does not block.
    pub remedy: Option<String>,
}

impl Diagnostic {
    /// Builds a finding. Severity comes from the code and is not a parameter.
    #[must_use]
    pub fn new(code: DiagnosticCode, subject: EntityRef, message: impl Into<String>) -> Self {
        Self {
            code,
            subject,
            related: Vec::new(),
            message: message.into(),
            remedy: None,
        }
    }

    /// Adds a remedy.
    #[must_use]
    pub fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }

    /// Adds a related reference.
    #[must_use]
    pub fn with_related(mut self, entity: EntityRef) -> Self {
        self.related.push(entity);
        self
    }

    /// How much this finding matters, from its code.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.code.severity()
    }
}

/// Everything pre-flight found, in one place.
///
/// Ordering is preserved as reported, so a caller that wants a stable presentation order
/// gets one without sorting.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    /// The findings, in the order they were reported.
    pub findings: Vec<Diagnostic>,
}

impl DiagnosticReport {
    /// An empty report.
    ///
    /// **Not a statement that a timetable exists.** Only a solver can say that, and ADR
    /// 0029 §6's restraint applies here too: a screen that says "this will work" and is then
    /// contradicted has spent the user's trust on a guess.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            findings: Vec::new(),
        }
    }

    /// Adds a finding.
    pub fn push(&mut self, finding: Diagnostic) {
        self.findings.push(finding);
    }

    /// Whether anything found refuses the solve.
    #[must_use]
    pub fn blocks_solve(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity() == Severity::Blocking)
    }

    /// Whether a solve may proceed.
    ///
    /// The one place a caller asks that question. It exists so no caller re-derives it by
    /// filtering severities itself — a caller that did could quietly admit a blocking finding
    /// by forgetting a variant.
    ///
    /// **A true answer is not a promise that a timetable exists.** It means pre-solve
    /// validation found no input it could prove impossible. Only a solver can say more, and
    /// `evara_core::lessons`' rule applies: never report that a demand *can* be scheduled.
    #[must_use]
    pub fn may_solve(&self) -> bool {
        !self.blocks_solve()
    }

    /// How many findings carry one severity.
    #[must_use]
    pub fn count_of(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity() == severity)
            .count()
    }

    /// How many findings carry one code.
    #[must_use]
    pub fn count_code(&self, code: DiagnosticCode) -> usize {
        self.findings.iter().filter(|f| f.code == code).count()
    }

    /// The findings of one severity, in report order.
    pub fn of_severity(&self, severity: Severity) -> impl Iterator<Item = &Diagnostic> + '_ {
        self.findings
            .iter()
            .filter(move |f| f.severity() == severity)
    }

    /// Whether any finding carries one code.
    #[must_use]
    pub fn has(&self, code: DiagnosticCode) -> bool {
        self.findings.iter().any(|f| f.code == code)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef, Severity};

    fn section() -> EntityRef {
        EntityRef::new(
            EntityKind::ClassSection,
            Uuid::from_u128(1),
            "Year 9 Mathematics A",
        )
    }

    #[test]
    fn every_code_has_a_distinct_stable_key() {
        // The one way a renamed or copy-pasted key gets caught before it ships. A key is
        // part of the contract with the interface and with any future test baseline.
        let mut keys: Vec<&str> = DiagnosticCode::all().iter().map(|c| c.key()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len(), "two diagnostic codes share a key");
    }

    #[test]
    fn every_key_is_kebab_case() {
        for code in DiagnosticCode::all() {
            let key = code.key();
            assert!(
                key.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{key} is not kebab-case"
            );
            assert!(!key.starts_with('-') && !key.ends_with('-'), "{key}");
        }
    }

    #[test]
    fn the_serialised_name_matches_the_key() {
        // Otherwise a finding read back from JSON would carry a different identifier from
        // the one a report printed, and the two would drift.
        for code in DiagnosticCode::all() {
            let json = serde_json::to_string(code).expect("a code serialises");
            assert_eq!(json, format!("\"{}\"", code.key()));
        }
    }

    #[test]
    fn the_bell_schedule_failures_block_and_the_sharing_warnings_do_not() {
        // SCHEDULING-ARCHITECTURE-DECISIONS.md §3.2's rule, asserted rather than assumed:
        // refuse where the user can answer, warn where they cannot.
        for code in [
            DiagnosticCode::NoGridMaterialised,
            DiagnosticCode::NoBellSchedule,
            DiagnosticCode::AmbiguousBellSchedule,
            DiagnosticCode::DurationExceedsLongestRun,
        ] {
            assert_eq!(code.severity(), Severity::Blocking, "{}", code.key());
        }
        for code in [
            DiagnosticCode::SharedEntityAcrossSchools,
            DiagnosticCode::GroupOverlapIndeterminate,
            DiagnosticCode::ExpectedAttendanceUnknown,
            DiagnosticCode::BookableRoomZeroCapacity,
        ] {
            assert_eq!(code.severity(), Severity::Warning, "{}", code.key());
        }
        for code in [
            DiagnosticCode::DeclaredPeriodsDisagree,
            DiagnosticCode::ExpectedAttendanceDisagreesWithGroups,
        ] {
            assert_eq!(code.severity(), Severity::Info, "{}", code.key());
        }
    }

    #[test]
    fn severity_comes_from_the_code_and_cannot_be_chosen() {
        // A caller that could pick its own severity could quietly downgrade a refusal.
        let finding = Diagnostic::new(DiagnosticCode::NoBellSchedule, section(), "…");
        assert_eq!(finding.severity(), Severity::Blocking);
        assert!(finding.severity().blocks_solve());
    }

    #[test]
    fn a_finding_carries_its_remedy_and_its_related_entities() {
        let finding = Diagnostic::new(
            DiagnosticCode::AmbiguousBellSchedule,
            section(),
            "Year 9 Mathematics A could be timetabled against two bell schedules.",
        )
        .with_related(EntityRef::new(
            EntityKind::BellSchedule,
            Uuid::from_u128(2),
            "Standard Day",
        ))
        .with_related(EntityRef::new(
            EntityKind::BellSchedule,
            Uuid::from_u128(3),
            "Northgate Senior Day",
        ))
        .with_remedy("Build this grid from one of them, or restrict this generation to one.");

        assert_eq!(finding.related.len(), 2, "every candidate is named");
        assert!(finding.remedy.is_some());
    }

    #[test]
    fn a_warning_may_legitimately_have_no_remedy() {
        // The cross-school case: the remedy — coordinate two schools' timetables — is the
        // thing Evara cannot do, which is also why it does not block.
        let finding = Diagnostic::new(
            DiagnosticCode::SharedEntityAcrossSchools,
            EntityRef::new(EntityKind::Teacher, Uuid::from_u128(9), "A. Okafor"),
            "Employed by Northgate and by Southmoor; Evara cannot compare the two.",
        );
        assert_eq!(finding.remedy, None);
        assert!(!finding.severity().blocks_solve());
    }

    #[test]
    fn an_empty_report_blocks_nothing_and_promises_nothing() {
        let report = DiagnosticReport::empty();
        assert!(!report.blocks_solve());
        assert_eq!(report.count_of(Severity::Blocking), 0);
        assert!(!report.has(DiagnosticCode::NoBellSchedule));
    }

    #[test]
    fn a_report_counts_by_severity_and_by_code() {
        let mut report = DiagnosticReport::empty();
        report.push(Diagnostic::new(
            DiagnosticCode::ExpectedAttendanceUnknown,
            section(),
            "…",
        ));
        report.push(Diagnostic::new(
            DiagnosticCode::ExpectedAttendanceUnknown,
            section(),
            "…",
        ));
        report.push(Diagnostic::new(
            DiagnosticCode::NoBellSchedule,
            section(),
            "…",
        ));

        assert_eq!(report.count_of(Severity::Warning), 2);
        assert_eq!(report.count_of(Severity::Blocking), 1);
        assert_eq!(
            report.count_code(DiagnosticCode::ExpectedAttendanceUnknown),
            2
        );
        assert!(report.blocks_solve());
        assert_eq!(report.of_severity(Severity::Blocking).count(), 1);
    }
}
