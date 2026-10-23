//! What a solve returns: a score, an outcome, and the counts that keep a timetable honest
//! about what was checked.
//!
//! # Declared in M2, produced since M6
//!
//! Nothing in this module solves anything and nothing here reads a clock. These are the
//! **shapes** an engine returns; `evara_solver::ConstructionSolver` is what returns them, and
//! M2 declared them before it existed so that M3's diagnostics, M7's persistence and M9's
//! review would be written against one contract rather than three.
//!
//! M6 added three things to the shape and changed none of it: a [`Termination`] beside the
//! status, a [`SolveStatistics`] of deterministic counters, and an [`UnplacedCause`] on each
//! unplaced occurrence. The five statuses are untouched — see [`Termination`] for why "the
//! limit was reached" is not a sixth.
//!
//! # Why there is no `success: bool`
//!
//! Five outcomes are genuinely different, and a boolean collapses at least three of them
//! into the same answer. "The problem is impossible", "the user pressed cancel" and "the
//! engine panicked" are not one state, and a user told only that generation failed cannot
//! tell which happened — which is the failure mode `SOLVER.md` §8 opens by naming: *"A bare
//! 'no solution found' is the failure mode this product exists to avoid."*
//! [`SolveStatus`] is that enum.
//!
//! # A timetable is never described as conflict-free
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4, which is the reason
//! [`UnverifiedCounts`] is a mandatory field of an outcome rather than an optional extra. A
//! generated timetable is described as having **no confirmed clash within the school
//! solved**, with the unverified counts beside it — from shared entities (ADR 0032),
//! `Indeterminate` group pairs (ADR 0035) and unchecked capacities (ADR 0034).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::diagnostics::{Diagnostic, DiagnosticReport};
use super::index::ActivityIx;
use super::placement::Assignment;

/// A timetable's cost, at three levels compared lexicographically.
///
/// # Integers, never floats
///
/// `CLAUDE.md` is explicit: *"Scores are integers. Three levels, compared
/// lexicographically. Weights become integer penalties at compile time, not at scoring
/// time."* A constraint's weight is a `REAL` in the database and is multiplied into an
/// integer penalty when the constraint is compiled, so comparison and equality are exact and
/// reproducible across platforms — which is what makes a benchmark baseline and a
/// byte-for-byte reproducible solve possible at all.
///
/// Penalties are negative-going, so a better score is a greater score and
/// [`Ord`] is the comparison the solver wants with no wrapper.
///
/// # `MEDIUM` is not decoration
///
/// `SOLVER.md` §3: it is the level for *"must be satisfied unless the alternative is an
/// infeasible timetable"* — unscheduled activities, unmet period counts, unstaffed classes.
/// It is what lets a solve return a usable **partial** timetable and say exactly what it
/// could not place, instead of returning nothing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    /// Hard penalties. Zero, and only zero, is a feasible timetable.
    pub hard: i64,
    /// Medium penalties: things that must hold unless the alternative is infeasibility.
    pub medium: i64,
    /// Soft penalties: preferences. Every soft constraint is deferred past the MVP.
    pub soft: i64,
}

impl Score {
    /// A timetable with no penalty at any level.
    pub const PERFECT: Self = Self {
        hard: 0,
        medium: 0,
        soft: 0,
    };

    /// Builds a score.
    #[must_use]
    pub const fn new(hard: i64, medium: i64, soft: i64) -> Self {
        Self { hard, medium, soft }
    }

    /// Whether no hard constraint is violated.
    ///
    /// The only definition of a *valid* timetable. A feasible score says nothing about
    /// quality, and — per ADR 0032 §5 rule 4 — it does not say the timetable is
    /// clash-free either: it says nothing was **confirmed** broken within the school solved.
    #[must_use]
    pub const fn is_feasible(&self) -> bool {
        self.hard == 0
    }

    /// Adds two scores level by level.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self {
            hard: self.hard + other.hard,
            medium: self.medium + other.medium,
            soft: self.soft + other.soft,
        }
    }
}

/// The seed a run's randomness came from.
///
/// # Determinism is non-negotiable
///
/// `SOLVER.md` §5: *"The RNG is explicitly seeded and stored on `solve_run.seed`; the same
/// seed, problem and config must reproduce the same timetable byte-for-byte."* The seed is
/// therefore part of the **result**, not only of the request — a run whose seed was not
/// recorded cannot be reproduced, which makes a regression undebuggable and a benchmark
/// baseline meaningless.
///
/// `CLAUDE.md` adds the other half of the rule: no `Instant::now()` and no unseeded
/// randomness inside the solver's decisions. A clock reading is not a seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seed(u64);

impl Seed {
    /// Builds a seed from a stored or chosen value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The value, for recording on a run or re-seeding an identical one.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How a run ended.
///
/// Five states, deliberately. See this module's header for why a boolean will not do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SolveStatus {
    /// Every activity is placed and no hard constraint is violated.
    ///
    /// The best available answer, and still not "conflict-free" — see [`UnverifiedCounts`].
    Complete,
    /// Some activities could not be placed, and what was placed breaks no hard constraint.
    ///
    /// A usable result, which is what `MEDIUM` exists to make possible. The unplaced
    /// activities are listed with a reason each.
    Partial,
    /// The best timetable found still violates a hard constraint.
    ///
    /// Reported with the violations and full entity references so the user can click through
    /// to the exact clash — `SOLVER.md` §8's Layer 2. Note that this is a statement about
    /// what the search found, **not** a proof that no timetable exists; a counting argument
    /// that *does* prove impossibility is a pre-solve
    /// [`Diagnostic`](super::Diagnostic) instead.
    Infeasible,
    /// The user stopped it.
    ///
    /// Whatever had been found is returned. Not a failure, and it must not be reported as
    /// one.
    Cancelled,
    /// The run could not complete.
    ///
    /// An engine error, distinct from an impossible problem. A user told only that
    /// generation "failed" cannot tell these apart, which is the whole reason they are
    /// separate variants.
    Failed,
}

impl SolveStatus {
    /// Whether a timetable worth looking at came back.
    ///
    /// [`SolveStatus::Complete`] and [`SolveStatus::Partial`]. An infeasible or cancelled run
    /// may still carry placements worth showing, but it does not carry a result the user can
    /// adopt as it stands.
    #[must_use]
    pub const fn is_usable(self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }

    /// Whether the run reached its own end rather than being stopped or breaking.
    #[must_use]
    pub const fn ran_to_completion(self) -> bool {
        matches!(self, Self::Complete | Self::Partial | Self::Infeasible)
    }
}

/// Why one activity was not placed.
///
/// Distinct from a [`Diagnostic`](super::Diagnostic): a diagnostic is about the *problem*
/// and is produced before a solve, while this is about the *run* and is produced by one. A
/// pre-solve diagnostic may well explain it, which is why
/// [`UnscheduledActivity::diagnostic`] can carry one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnscheduledActivity {
    /// Which occurrence.
    pub activity: ActivityIx,
    /// Its stored identity, so a report need not hold the problem to name it.
    pub id: Uuid,
    /// What the section is called, so a report can name the lesson rather than a number.
    pub section: String,
    /// Why, as a cause a screen can group and act on.
    ///
    /// The structured half of the answer. [`UnplacedCause::is_impossible`] is what separates
    /// *"no timetable can place this"* from *"this arrangement did not"*, and a report that
    /// read only [`UnscheduledActivity::reason`] could not tell them apart.
    pub cause: UnplacedCause,
    /// Why, in prose the engine authored.
    pub reason: String,
    /// The pre-solve finding that already explained it, where there is one.
    ///
    /// `None` means the search simply ran out of room for it — a difficulty rather than an
    /// impossibility. `Some(_)` means it was conclusively impossible and pre-flight had
    /// already said so, which is a different message for the user.
    pub diagnostic: Option<Diagnostic>,
}

/// One hard constraint, and how many times the result breaks it.
///
/// A summary rather than the violations themselves: the full list with entity references is
/// `SOLVER.md` §8's Layer 2 report, and a score breakdown needs only the counts. Keeping
/// them apart is what stops a summary from growing to the size of the timetable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardViolationCount {
    /// The constraint's stable `type_key`, as registered in `evara-constraints`.
    ///
    /// A plain `String` rather than an enum, because the registry is open: a constraint type
    /// is addressed by a stable key and `evara-core` must not have to be edited to add one.
    pub type_key: String,
    /// How many violations of it the result carries.
    pub violations: u32,
}

/// What a timetable was **not** able to check.
///
/// # Mandatory, because of what it prevents
///
/// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4: *"A timetable is never described as
/// conflict-free. It is described as having no confirmed clash within the school solved,
/// with the unverified counts beside it."* Every figure here is a real hole that three
/// accepted records each chose to make visible rather than to close:
///
/// - [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
///   rule 3 — a shared teacher double-booked across two schools is **not detected**, only
///   announced. "This is a real hole and this record does not hide it."
/// - [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
///   §6 — until a school answers the disjointness question, two groups that genuinely
///   overlap may be timetabled together.
/// - [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
///   §5 — the first solver can place a class of 32 in a room for 20.
///
/// All zeroes is the only state in which a timetable has nothing to disclose, and even then
/// the correct phrasing is "no confirmed clash", because the solver checked what it could
/// check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UnverifiedCounts {
    /// Pairs of concurrently placed activities whose audiences could not be compared.
    ///
    /// Their group overlap was [`Overlap::Indeterminate`](super::Overlap::Indeterminate), so
    /// no clash was enforced and none was proved absent.
    pub indeterminate_student_pairs: u32,
    /// Activities for which room size was not checked.
    ///
    /// Their section's expected attendance is
    /// [`ExpectedAttendance::Unknown`](super::ExpectedAttendance::Unknown), so `room-capacity`
    /// was **not evaluated** — which is a third outcome, and must be rendered as one rather
    /// than as zero violations.
    pub unchecked_capacities: u32,
    /// Entities in scope that another school also holds.
    ///
    /// Each was announced before the solve and none was compared across the boundary.
    pub shared_entities: u32,
    /// Stored availability restrictions this engine does not enforce.
    ///
    /// `evara_db::Completeness::RestrictionsNotEnforced`'s count, carried onto the result. A
    /// `RESOURCE` or `SUBJECT` availability rule is recorded by the school and read by no
    /// evaluator, so a timetable generated over it has been checked against **less** than the
    /// school entered.
    ///
    /// The fourth hole, and it belongs beside the other three for the same reason they are
    /// here: a timetable described as clean when a stored rule was never applied is the
    /// failure `V11`'s store-only-exceptions design exists to prevent. `evara-core` cannot
    /// name the database type, so the figure crosses as a count.
    pub unenforced_restrictions: u32,
}

impl UnverifiedCounts {
    /// Nothing left unchecked.
    pub const NONE: Self = Self {
        indeterminate_student_pairs: 0,
        unchecked_capacities: 0,
        shared_entities: 0,
        unenforced_restrictions: 0,
    };

    /// Whether the timetable has anything to disclose.
    ///
    /// Even when this is `false`, the timetable is described as having no **confirmed**
    /// clash within the school solved — never as conflict-free.
    #[must_use]
    pub const fn has_anything_to_disclose(self) -> bool {
        self.indeterminate_student_pairs > 0
            || self.unchecked_capacities > 0
            || self.shared_entities > 0
            || self.unenforced_restrictions > 0
    }
}

/// Everything one run produced.
///
/// # The shape M2 declared, filled in by M6
///
/// `evara_solver::generate` returns one. Every field M2 declared is still here and means what
/// it meant; M6 added [`SolveOutcome::termination`] and [`SolveOutcome::statistics`], which are
/// about the **run** rather than about the timetable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveOutcome {
    /// Which problem this is a result for.
    pub problem: ProblemIdentity,
    /// The seed the run's randomness came from, so it can be reproduced exactly.
    pub seed: Seed,
    /// How it ended: what came back.
    pub status: SolveStatus,
    /// How it ended: why the engine stopped looking.
    ///
    /// Beside the status rather than inside it. A `Partial` result that was exhausted and one
    /// that hit a budget are the same timetable and two different things to tell a user — see
    /// [`Termination`].
    pub termination: Termination,
    /// What was placed where.
    ///
    /// Carried for every status, including [`SolveStatus::Cancelled`] and
    /// [`SolveStatus::Infeasible`]: a partial or infeasible result is still worth showing,
    /// and throwing it away would make the Layer-2 violation report impossible to render.
    pub assignment: Assignment,
    /// What could not be placed, and why.
    pub unscheduled: Vec<UnscheduledActivity>,
    /// The best score reached.
    pub score: Score,
    /// Which hard constraints the result breaks, and how many times each.
    ///
    /// Empty when [`Score::is_feasible`] holds, and the two must agree.
    pub hard_violations: Vec<HardViolationCount>,
    /// What was not checked. Never omitted — see [`UnverifiedCounts`].
    pub unverified: UnverifiedCounts,
    /// Findings from before the solve, carried onto the result.
    ///
    /// So a timetable's provenance can state the limits it was generated under, rather than
    /// a user having to remember a warning they saw on a previous screen.
    pub preflight: DiagnosticReport,
    /// What the run did, counted. Deterministic — see [`SolveStatistics`].
    pub statistics: SolveStatistics,
}

impl SolveOutcome {
    /// Whether the result is a timetable the user could adopt as it stands.
    ///
    /// Both halves are required: the run must have produced a usable status **and** the
    /// score must be feasible. They can disagree — an infeasible status with a feasible
    /// score would be a bug — and asking for both is how that bug surfaces rather than
    /// being papered over.
    ///
    /// A **partial** result can be adoptable, and that is the point of `MEDIUM`: a school may
    /// adopt eleven of twelve lessons and place the twelfth by hand. Completeness is
    /// [`SolveOutcome::is_complete`], asked separately.
    #[must_use]
    pub fn is_adoptable(&self) -> bool {
        self.status.is_usable() && self.score.is_feasible()
    }

    /// Whether every occurrence was placed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unscheduled.is_empty() && self.assignment.is_complete()
    }

    /// How many occurrences were placed.
    #[must_use]
    pub fn placed_count(&self) -> usize {
        self.assignment.scheduled_count()
    }

    /// The unplaced occurrences this run proved no timetable can place.
    ///
    /// A subset of [`SolveOutcome::unscheduled`], and usually empty: the gate refuses the
    /// inputs that make it non-empty before a placement is attempted. Separated because the
    /// remedy differs — one is a setup change, the other is "try again differently" — and
    /// because reporting the second as the first is the overclaim
    /// `evara_core::lessons`' rule forbids.
    pub fn impossible(&self) -> impl Iterator<Item = &UnscheduledActivity> + '_ {
        self.unscheduled.iter().filter(|u| u.cause.is_impossible())
    }
}

/// Which problem a result belongs to.
///
/// Identifiers rather than the whole [`Problem`](super::Problem): an outcome is persisted and
/// rendered long after the problem it came from was dropped, and an outcome that carried the
/// problem would be a second copy of it that could drift.
///
/// One school, one cycle, one term — ADR 0032 rule 1. **A timetable row is never
/// project-wide.**
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemIdentity {
    /// The school solved.
    pub school: Uuid,
    /// The cycle.
    pub cycle: Uuid,
    /// The term, or `None` for the grid that applies all year.
    pub term: Option<Uuid>,
}

/// Why a run stopped, beside *what it ended up with*.
///
/// # Not a sixth [`SolveStatus`]
///
/// The two answer different questions and a school needs both. `SolveStatus` says **what you
/// have** — a complete timetable, a partial one, an infeasible one. This says **why the engine
/// stopped looking**, and the pairing is what makes a partial result actionable: a `Partial`
/// run that stopped at [`Termination::Exhausted`] has been told every remaining occurrence was
/// tried and refused, while one that stopped at [`Termination::CandidateLimitReached`] has been
/// told nothing of the kind and should be run again with a larger budget.
///
/// Collapsing the two would force exactly the wrong report. `SolveStatus`'s own header is
/// explicit that *"a user told only that generation failed cannot tell which happened"*, and
/// adding `LimitReached` to that enum would make a timetable's completeness and the engine's
/// budget the same field.
///
/// # Every reason here is deterministic except one
///
/// A limit is a **count** — candidate evaluations, construction steps, retry passes — so the
/// same problem, seed and limits stop at the same place on every machine.
/// [`Termination::Cancelled`] is the exception and is the only reason a caller's clock can
/// produce, because cancellation is the caller's to decide: `evara-solver` reads no clock
/// (`CLAUDE.md`), and a wall-clock budget reaches it as a cancellation it cannot tell from a
/// user pressing stop. A run that ended this way is **not reproducible**, and that is a
/// property of the request rather than of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Termination {
    /// Every occurrence is placed, so there was nothing left to do.
    Complete,
    /// Every unplaced occurrence was tried against every candidate and refused.
    ///
    /// The engine ran out of **moves**, not of budget. Still not a proof that no complete
    /// timetable exists: a greedy construction that painted itself into a corner reports
    /// exactly this, and `SOLVER.md` §8's counting diagnostics are the only thing in Evara
    /// that can prove impossibility.
    Exhausted,
    /// The cap on candidate evaluations was reached.
    CandidateLimitReached,
    /// The cap on construction steps was reached.
    StepLimitReached,
    /// The cap on retry passes was reached while a pass was still placing things.
    PassLimitReached,
    /// The caller stopped it — a user, or a wall-clock budget the caller was enforcing.
    Cancelled,
    /// Pre-solve validation refused the inputs, so no placement was attempted.
    ///
    /// Distinct from [`Termination::Exhausted`]: nothing was tried. The remedy is in
    /// [`SolveOutcome::preflight`] and is a field the user can fill in.
    InputRejected,
    /// The engine's own final check found a hard violation in what it had accepted.
    ///
    /// A defect in Evara rather than a property of the school's data. The result is returned
    /// rather than discarded so the violation can be reported, and the status is
    /// [`SolveStatus::Failed`].
    InternalCheckFailed,
}

impl Termination {
    /// Whether a larger budget could plausibly place more.
    ///
    /// `true` for the three limits and for a cancellation. Used to phrase a partial result
    /// honestly: *"the constructor stopped at its budget"* rather than *"these lessons cannot
    /// be placed"*.
    #[must_use]
    pub const fn was_cut_short(self) -> bool {
        matches!(
            self,
            Self::CandidateLimitReached
                | Self::StepLimitReached
                | Self::PassLimitReached
                | Self::Cancelled
        )
    }

    /// Whether the same problem, configuration and seed reproduce this run exactly.
    ///
    /// `false` only for [`Termination::Cancelled`] — see this type's header.
    #[must_use]
    pub const fn is_reproducible(self) -> bool {
        !matches!(self, Self::Cancelled)
    }

    /// The stable key, for a structured report.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Exhausted => "exhausted",
            Self::CandidateLimitReached => "candidate-limit-reached",
            Self::StepLimitReached => "step-limit-reached",
            Self::PassLimitReached => "pass-limit-reached",
            Self::Cancelled => "cancelled",
            Self::InputRejected => "input-rejected",
            Self::InternalCheckFailed => "internal-check-failed",
        }
    }
}

/// What a run did, counted.
///
/// # Counters, and deliberately no elapsed time
///
/// Every figure here is a count of decisions, so two runs of the same problem at the same seed
/// produce **identical** statistics and a baseline can be committed against them. There is no
/// `elapsed` field, and its absence is the decision: `CLAUDE.md` forbids `Instant::now()`
/// inside the solver, a duration recorded on the outcome would make [`SolveOutcome`]'s
/// [`PartialEq`] machine-dependent, and the determinism tests compare whole outcomes.
///
/// A caller that wants wall-clock time measures it around the call and reports it beside the
/// outcome, which is what `evara generate` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveStatistics {
    /// Candidate placements whose legality was tested.
    ///
    /// The engine's unit of work, and what [`Termination::CandidateLimitReached`] counts.
    pub candidate_evaluations: u64,
    /// Candidates that were tested and refused for breaking a hard rule.
    pub candidates_rejected: u64,
    /// Placements committed to the working assignment.
    ///
    /// Greater than the number of placed occurrences when a later pass replaced an earlier
    /// placement.
    pub placements_committed: u64,
    /// Occurrences taken off the working assignment again.
    pub placements_withdrawn: u64,
    /// Occurrences the constructor attempted to place, counting retries.
    pub construction_steps: u64,
    /// Sweeps over the unplaced occurrences, including the first.
    pub passes: u32,
}

/// Why one occurrence was not placed, as a cause rather than a sentence.
///
/// # Structured, because a sentence cannot be acted on
///
/// `SOLVER.md` §6's rule for an [`Explanation`](super::Explanation) — *"structured, not a
/// string"* — applies here for the same reason: a Generate screen has to group unplaced
/// lessons by remedy, and the remedy for "no room of the required type exists" is not the
/// remedy for "a teacher is already busy in every slot that would fit".
///
/// # The two halves of the list, and why they must not be confused
///
/// The first four causes are properties of the **problem** and hold whatever else was placed:
/// the occurrence had no legal candidate at all. The next two are properties of this **run**:
/// the occurrence had candidates and every one of them clashed with something already placed,
/// or the engine stopped before reaching it.
///
/// That distinction is `evara_core::lessons`' rule applied to a solver result. *"Never report
/// that a demand can be scheduled"* has a mirror — never report that one cannot — and
/// [`UnplacedCause::is_impossible`] is where the difference is stated once. A greedy
/// constructor leaving a lesson unplaced is **not** a proof that no complete timetable exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnplacedCause {
    /// Its section resolves to no single bell schedule, so it has no value range.
    ///
    /// A hard pre-flight error under ADR 0033 §3; a run reaching this means the gate was
    /// bypassed, which only a caller that ignored [`SolveOutcome::preflight`] can do.
    NoResolvedBellSchedule,
    /// No run of that many consecutive teaching periods exists in its schedule.
    ///
    /// Arithmetic, and the one cause that proves impossibility from the problem alone — ADR
    /// 0033 §5. M3's `duration-exceeds-longest-run` says it before a solve.
    NoContiguousRun,
    /// Its section requires a kind of room and no room is a candidate.
    ///
    /// M3's `no-room-of-required-type`, met again at placement time.
    NoCandidateRoom,
    /// Every slot that would fit is one a stored rule marks `UNAVAILABLE`.
    ///
    /// For the teachers, the audience, or every candidate room — the occurrence has a value
    /// range and availability empties it.
    AllCandidatesUnavailable,
    /// Every remaining candidate clashes with an occurrence already placed.
    ///
    /// A property of this arrangement. A different order, a different seed or a later repair
    /// pass may well place it, and nothing here says a complete timetable is impossible.
    BlockedByPlacements,
    /// The engine stopped at a limit before this occurrence was placed.
    ///
    /// Nothing is claimed about it at all. [`Termination`] says which limit.
    NotReached,
    /// Generation was refused before it began, because the inputs carry a blocking finding.
    ///
    /// Also nothing claimed about this occurrence in particular — the refusal is about the
    /// school's setup. Distinct from [`UnplacedCause::NotReached`] because the remedies are
    /// opposites: that one says *run again with a larger budget*, and this one says *correct
    /// the input first*, and offering the first for the second sends a user to retry something
    /// that cannot work yet.
    InputRejected,
}

impl UnplacedCause {
    /// Whether this cause proves no timetable can place the occurrence.
    ///
    /// `true` only for a property of the problem that holds whatever else is placed. A cause
    /// about *this run* is never impossibility — see this type's header.
    #[must_use]
    pub const fn is_impossible(self) -> bool {
        matches!(
            self,
            Self::NoResolvedBellSchedule
                | Self::NoContiguousRun
                | Self::NoCandidateRoom
                | Self::AllCandidatesUnavailable
        )
    }

    /// The stable key, for a structured report and a test that pins the vocabulary.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::NoResolvedBellSchedule => "no-resolved-bell-schedule",
            Self::NoContiguousRun => "no-contiguous-run",
            Self::NoCandidateRoom => "no-candidate-room",
            Self::AllCandidatesUnavailable => "all-candidates-unavailable",
            Self::BlockedByPlacements => "blocked-by-placements",
            Self::NotReached => "not-reached",
            Self::InputRejected => "input-rejected",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Score, Seed, SolveStatistics, SolveStatus, Termination, UnplacedCause, UnverifiedCounts,
    };

    #[test]
    fn only_a_zero_hard_score_is_feasible() {
        assert!(Score::PERFECT.is_feasible());
        assert!(Score::new(0, -40, -900).is_feasible());
        assert!(!Score::new(-1, 0, 0).is_feasible());
    }

    #[test]
    fn scores_compare_lexicographically_with_hard_dominating() {
        // The ordering the solver relies on: one hard violation is worse than any amount of
        // soft unpleasantness, however large.
        assert!(Score::new(0, -1_000_000, -1_000_000) > Score::new(-1, 0, 0));
        assert!(Score::new(0, 0, -1) > Score::new(0, -1, 0));
        assert!(Score::PERFECT > Score::new(0, 0, -1));
    }

    #[test]
    fn a_score_is_the_sum_of_its_parts_level_by_level() {
        assert_eq!(
            Score::new(-1, -2, -3).plus(Score::new(0, -4, -5)),
            Score::new(-1, -6, -8)
        );
    }

    #[test]
    fn the_default_score_is_perfect_rather_than_arbitrary() {
        // An empty timetable has broken nothing yet, so accumulation starts from zero.
        assert_eq!(Score::default(), Score::PERFECT);
    }

    #[test]
    fn a_seed_round_trips_so_a_run_can_be_reproduced() {
        assert_eq!(Seed::new(0xDEAD_BEEF).get(), 0xDEAD_BEEF);
    }

    #[test]
    fn the_five_outcomes_stay_distinct() {
        // "Impossible", "the user pressed cancel" and "the engine broke" are not one state,
        // and a boolean would make them one.
        let all = [
            SolveStatus::Complete,
            SolveStatus::Partial,
            SolveStatus::Infeasible,
            SolveStatus::Cancelled,
            SolveStatus::Failed,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn only_complete_and_partial_are_usable_results() {
        assert!(SolveStatus::Complete.is_usable());
        assert!(SolveStatus::Partial.is_usable());
        assert!(!SolveStatus::Infeasible.is_usable());
        assert!(
            !SolveStatus::Cancelled.is_usable(),
            "a cancelled run is not a failure and is also not an answer"
        );
        assert!(!SolveStatus::Failed.is_usable());
    }

    #[test]
    fn a_cancelled_run_is_distinguished_from_one_that_finished() {
        assert!(SolveStatus::Infeasible.ran_to_completion());
        assert!(!SolveStatus::Cancelled.ran_to_completion());
        assert!(!SolveStatus::Failed.ran_to_completion());
    }

    #[test]
    fn nothing_unverified_still_is_not_a_claim_of_being_conflict_free() {
        // The type cannot enforce the wording, so this test pins the data: all zeroes means
        // there is nothing to disclose, and the four holes remain separately countable.
        assert!(!UnverifiedCounts::NONE.has_anything_to_disclose());
        assert_eq!(UnverifiedCounts::default(), UnverifiedCounts::NONE);

        for counts in [
            UnverifiedCounts {
                indeterminate_student_pairs: 1,
                ..UnverifiedCounts::NONE
            },
            UnverifiedCounts {
                unchecked_capacities: 1,
                ..UnverifiedCounts::NONE
            },
            UnverifiedCounts {
                shared_entities: 1,
                ..UnverifiedCounts::NONE
            },
            UnverifiedCounts {
                unenforced_restrictions: 1,
                ..UnverifiedCounts::NONE
            },
        ] {
            assert!(counts.has_anything_to_disclose());
        }
    }

    // ------------------------------------------------------- what M6 added to the shape

    #[test]
    fn a_termination_is_not_a_status_and_the_two_cross_freely() {
        // The pairing is the point: the same timetable with two different reasons for
        // stopping is two different things to tell a school.
        for termination in [Termination::Exhausted, Termination::CandidateLimitReached] {
            assert!(SolveStatus::Partial.is_usable());
            assert_ne!(termination.key(), "", "every termination has a key");
        }
        assert_ne!(
            Termination::Exhausted.was_cut_short(),
            Termination::CandidateLimitReached.was_cut_short(),
            "a budget and a dead end must not read the same"
        );
    }

    #[test]
    fn every_limit_says_a_bigger_budget_might_help_and_a_dead_end_does_not() {
        for cut_short in [
            Termination::CandidateLimitReached,
            Termination::StepLimitReached,
            Termination::PassLimitReached,
            Termination::Cancelled,
        ] {
            assert!(cut_short.was_cut_short(), "{cut_short:?}");
        }
        for finished in [
            Termination::Complete,
            Termination::Exhausted,
            Termination::InputRejected,
            Termination::InternalCheckFailed,
        ] {
            assert!(!finished.was_cut_short(), "{finished:?}");
        }
    }

    #[test]
    fn only_a_cancelled_run_is_not_reproducible() {
        // The one reason a caller's clock can produce. Everything else is a count, so the
        // same seed stops in the same place on every machine.
        assert!(!Termination::Cancelled.is_reproducible());
        for deterministic in [
            Termination::Complete,
            Termination::Exhausted,
            Termination::CandidateLimitReached,
            Termination::StepLimitReached,
            Termination::PassLimitReached,
            Termination::InputRejected,
            Termination::InternalCheckFailed,
        ] {
            assert!(deterministic.is_reproducible(), "{deterministic:?}");
        }
    }

    #[test]
    fn termination_keys_are_distinct_kebab_case() {
        let all = [
            Termination::Complete,
            Termination::Exhausted,
            Termination::CandidateLimitReached,
            Termination::StepLimitReached,
            Termination::PassLimitReached,
            Termination::Cancelled,
            Termination::InputRejected,
            Termination::InternalCheckFailed,
        ];
        let mut keys: Vec<&str> = all.iter().map(|t| t.key()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "two terminations share a key");
        for key in keys {
            assert!(
                key.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{key} is not kebab-case"
            );
        }
    }

    #[test]
    fn an_impossible_cause_is_about_the_problem_and_the_others_are_about_the_run() {
        // The distinction `evara_core::lessons`' rule turns into a mirror: never report that
        // a demand cannot be scheduled when all you know is that this arrangement did not.
        for proven in [
            UnplacedCause::NoResolvedBellSchedule,
            UnplacedCause::NoContiguousRun,
            UnplacedCause::NoCandidateRoom,
            UnplacedCause::AllCandidatesUnavailable,
        ] {
            assert!(proven.is_impossible(), "{proven:?}");
        }
        for merely_this_run in [
            UnplacedCause::BlockedByPlacements,
            UnplacedCause::NotReached,
            UnplacedCause::InputRejected,
        ] {
            assert!(
                !merely_this_run.is_impossible(),
                "{merely_this_run:?} is a property of one arrangement, not of the problem"
            );
        }
    }

    #[test]
    fn unplaced_cause_keys_are_distinct() {
        let all = [
            UnplacedCause::NoResolvedBellSchedule,
            UnplacedCause::NoContiguousRun,
            UnplacedCause::NoCandidateRoom,
            UnplacedCause::AllCandidatesUnavailable,
            UnplacedCause::BlockedByPlacements,
            UnplacedCause::NotReached,
            UnplacedCause::InputRejected,
        ];
        let mut keys: Vec<&str> = all.iter().map(|c| c.key()).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before);
    }

    #[test]
    fn statistics_start_at_zero_and_carry_no_clock() {
        // Counters only. A duration here would make a whole-outcome equality assertion —
        // which is how the determinism tests are written — machine-dependent.
        let fresh = SolveStatistics::default();
        assert_eq!(fresh.candidate_evaluations, 0);
        assert_eq!(fresh.placements_committed, 0);
        assert_eq!(fresh.passes, 0);
        let serialised = serde_json::to_string(&fresh).expect("deterministic statistics serialise");
        assert!(
            !serialised.contains("elapsed") && !serialised.contains("millis"),
            "{serialised} must carry no time"
        );
    }

    #[test]
    fn the_new_vocabulary_serialises_in_the_wire_casing() {
        assert_eq!(
            serde_json::to_string(&Termination::CandidateLimitReached).expect("serialises"),
            r#""candidateLimitReached""#
        );
        assert_eq!(
            serde_json::to_string(&UnplacedCause::BlockedByPlacements).expect("serialises"),
            r#""blockedByPlacements""#
        );
    }

    #[test]
    fn the_status_serialises_in_the_wire_casing() {
        assert_eq!(
            serde_json::to_string(&SolveStatus::Infeasible).expect("a status serialises"),
            r#""infeasible""#
        );
    }
}
