//! What a solve returns: a score, an outcome, and the counts that keep a timetable honest
//! about what was checked.
//!
//! # No solver exists
//!
//! Nothing in this module solves anything, and nothing in this milestone does. There is no
//! `solve` function here, no engine, and no stub that returns a fabricated timetable — the
//! `SolverEngine` trait and its implementations are M6's, and `evara-solver` is still a
//! documentation-only crate. These are the **shapes** a future engine returns, declared now
//! so that M3's diagnostics, M7's persistence and M9's review are written against one
//! contract rather than three.
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
}

impl UnverifiedCounts {
    /// Nothing left unchecked.
    pub const NONE: Self = Self {
        indeterminate_student_pairs: 0,
        unchecked_capacities: 0,
        shared_entities: 0,
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
    }
}

/// Everything one run produced.
///
/// # Not produced by anything yet
///
/// No function in this workspace returns one. It is the declared result shape for M6's
/// engine, and declaring it now is what lets M7's persistence and M9's review be designed
/// against the real thing instead of against a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveOutcome {
    /// Which problem this is a result for.
    pub problem: ProblemIdentity,
    /// The seed the run's randomness came from, so it can be reproduced exactly.
    pub seed: Seed,
    /// How it ended.
    pub status: SolveStatus,
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
}

impl SolveOutcome {
    /// Whether the result is a timetable the user could adopt as it stands.
    ///
    /// Both halves are required: the run must have produced a usable status **and** the
    /// score must be feasible. They can disagree — an infeasible status with a feasible
    /// score would be a bug — and asking for both is how that bug surfaces rather than
    /// being papered over.
    #[must_use]
    pub fn is_adoptable(&self) -> bool {
        self.status.is_usable() && self.score.is_feasible()
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

#[cfg(test)]
mod tests {
    use super::{Score, Seed, SolveStatus, UnverifiedCounts};

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
        // there is nothing to disclose, and the three holes remain separately countable.
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
        ] {
            assert!(counts.has_anything_to_disclose());
        }
    }

    #[test]
    fn the_status_serialises_in_the_wire_casing() {
        assert_eq!(
            serde_json::to_string(&SolveStatus::Infeasible).expect("a status serialises"),
            r#""infeasible""#
        );
    }
}
