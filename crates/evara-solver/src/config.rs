//! What a solve was asked to do, and when it must stop.
//!
//! # Every limit is a count, and that is the whole point
//!
//! `CLAUDE.md`: *"Solving is deterministic. Seeded RNG, recorded on the run. No
//! `Instant::now()` or unseeded randomness inside the solver's decisions."* A time limit
//! violates that twice over — it reads a clock, and the place it stops depends on how fast the
//! machine is, so the same seed produces different timetables on a laptop and in CI. A
//! benchmark baseline committed against such a run is meaningless.
//!
//! So every limit here is a **count of decisions**: candidate evaluations, construction steps,
//! retry passes. The same problem, seed and limits stop at the same place everywhere.
//!
//! # A wall clock is still supported, and it is the caller's
//!
//! A desktop user who has waited long enough wants to stop, and that is a real requirement.
//! [`Cancel`] is how: the solver asks `is_cancelled()` between steps and the **caller** decides
//! what the answer depends on — a button, a deadline, anything. The solver reads no clock, the
//! cancellation shows up as [`Termination::Cancelled`](evara_core::scheduling::Termination), and
//! that termination is the one this workspace documents as **not reproducible**. Keeping the
//! clock outside is what lets the engine stay honest about which of its results can be replayed.

use evara_core::scheduling::Seed;

/// Deterministic caps on how much work a solve may do.
///
/// # Defaults are generous, not tight
///
/// The defaults exist so that a limit is never the reason a realistic school fails to schedule;
/// they are a backstop against a pathological problem, not a budget to tune. A 500-occurrence
/// school with 40 rooms and 30 runs a cycle has roughly 600 000 candidates in total and
/// construction touches a small fraction of them, so [`SolveLimits::DEFAULT`] is about two
/// orders of magnitude above what such a school needs — and reaching one is reported, never
/// silent (`CLAUDE.md`: *"No silent caps"*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolveLimits {
    /// The most candidate placements whose legality may be tested.
    ///
    /// The engine's unit of work, so this is the limit that actually bounds the run.
    pub max_candidate_evaluations: u64,
    /// The most occurrences the constructor may attempt, retries included.
    pub max_construction_steps: u64,
    /// The most sweeps over the unplaced occurrences.
    ///
    /// A pass that places nothing ends the run on its own, so this only bites on a problem
    /// where every pass keeps making progress for an unreasonable number of passes. Bounded
    /// because an unbounded loop is a hang, and a hang is a worse failure than a partial
    /// timetable.
    pub max_passes: u32,
}

impl SolveLimits {
    /// The shipped limits.
    pub const DEFAULT: Self = Self {
        max_candidate_evaluations: 50_000_000,
        max_construction_steps: 1_000_000,
        max_passes: 8,
    };

    /// Limits that stop almost immediately, for testing the stopping itself.
    ///
    /// A test that wanted to see [`Termination::CandidateLimitReached`] by building a huge
    /// problem would be slow and would stop being a test of the limit the moment the problem
    /// got easier.
    #[must_use]
    pub const fn at_most(candidates: u64) -> Self {
        Self {
            max_candidate_evaluations: candidates,
            ..Self::DEFAULT
        }
    }
}

impl Default for SolveLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Everything about a solve that is not the problem itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolveConfig {
    /// The seed every ordering decision derives from.
    ///
    /// Recorded on the result, because *"a run whose seed was not recorded cannot be
    /// reproduced"* — [`Seed`]'s own documentation.
    pub seed: Seed,
    /// When to stop.
    pub limits: SolveLimits,
}

impl SolveConfig {
    /// A configuration at one seed, with the shipped limits.
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self {
            seed: Seed::new(seed),
            limits: SolveLimits::DEFAULT,
        }
    }

    /// The same configuration with different limits.
    #[must_use]
    pub const fn with_limits(self, limits: SolveLimits) -> Self {
        Self { limits, ..self }
    }
}

impl Default for SolveConfig {
    /// Seed `2026`, matching `evara fixture`, `evara validate` and `evara evaluate`.
    ///
    /// A default seed rather than no seed: there is no unseeded mode, because an unseeded solve
    /// is one nobody can reproduce or benchmark.
    fn default() -> Self {
        Self::seeded(2026)
    }
}

/// Whether the caller wants the run to stop.
///
/// Asked between construction steps. See this module's header for why the clock lives on the
/// caller's side of this trait and not inside the engine.
pub trait Cancel {
    /// Whether to stop now.
    ///
    /// Called often, so it must be cheap. An implementation that read a file or took a lock
    /// would make the solver's speed depend on something other than the problem.
    fn is_cancelled(&self) -> bool;
}

/// A cancellation that never fires.
///
/// What the headless CLI and every test pass. Named rather than anonymous so a caller that
/// genuinely wants no cancellation says so, instead of an `Option<&dyn Cancel>` that reads as
/// an oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NeverCancel;

impl Cancel for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Where a run reports its progress.
///
/// `SOLVER.md` §5 streams best score, step count and elapsed time to the frontend as
/// `solver://progress` events. The engine supplies the first two; elapsed time is the caller's,
/// for the reason the clock is.
pub trait ProgressSink {
    /// How far the construction has got.
    ///
    /// `placed` of `total`, after `steps` attempts. Called once per committed placement and
    /// once per occurrence given up on, so a caller can drive a progress bar without polling.
    fn placed(&self, placed: usize, total: usize, steps: u64);
}

/// A sink that discards everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn placed(&self, _placed: usize, _total: usize, _steps: u64) {}
}
