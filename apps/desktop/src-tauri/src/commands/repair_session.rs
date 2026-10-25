//! A repair session: one baseline, one scope, and the six hazards that guard applying it.
//!
//! # Copied from the generation session, not extended
//!
//! [`super::session::Lifecycle`] already solves five of the six hazards, and the shape here is
//! deliberately the same one with `Saving`/`Saved` renamed. It is a sibling rather than a reuse
//! because a repair session carries a baseline, a movable set, an arrangement stamp and a source
//! revision that a generation has no field for, and because its outcome is a
//! [`RepairOutcome`] rather than a bare `SolveOutcome` — the attribution and the defect have to
//! survive into the preview.
//!
//! ```text
//!                    ┌──────────────────── discard ───────────────────┐
//!                    v                                                │
//!   start ──► Running ──► Finished ──► Applying ──► Applied           │
//!                            ^            │                          │
//!                            └─ failed ───┘   (the write is retryable)
//! ```
//!
//! `Applied` and `Discarded` are terminal, and **nothing leaves `Applied`** — which is what
//! makes a duplicate apply impossible rather than unlikely.
//!
//! # The six hazards
//!
//! | Hazard | Guard |
//! | --- | --- |
//! | Applying an unfinished repair | [`RepairLifecycle::claim_apply`] refuses from `Running` |
//! | Applying a cancelled, refused or defective repair | [`RepairOutcome::applyable`], computed by the solver |
//! | Two applies from two clicks | a compare-and-swap into `Applying` **before** any database work |
//! | A cancel racing a finish | `request_cancel` sets an `AtomicBool` and **no state**; the worker is the only thing that leaves `Running` |
//! | Applying into a different project | [`RepairSession::epoch`], compared before a transaction opens |
//! | **Applying over an arrangement that has since been edited** | the [`ArrangementStamp`] taken with the baseline, compared *inside* the writing transaction. New in M9B: a generation has no baseline to be stale about |
//!
//! Nothing here compares a watermark or a stamp itself — `evara-db` does, inside the
//! transaction that writes. ADR 0030's rule, which `CLAUDE.md` §3 restates.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evara_core::scheduling::{Assignment, CompiledConstraint, LabelSet, MovableSet, RepairScope};
use evara_db::{ArrangementStamp, Assembly, EntryFlags, SchedulingSelection};
use evara_solver::RepairOutcome;
use serde::Serialize;
use uuid::Uuid;

use super::session::{CancelFlag, Progress, ProgressReporter};

/// How far a repair session has got.
#[derive(Debug)]
enum State {
    /// The worker is running. Nothing may be applied.
    Running,
    /// The worker finished. What it found is here, whatever it says.
    Finished(Box<RepairOutcome>),
    /// An apply has been claimed by one caller and is in flight.
    Applying(Box<RepairOutcome>),
    /// Applied, as this timetable. Terminal.
    Applied {
        /// The timetable that was written.
        timetable_id: Uuid,
        /// What was applied, so the preview still renders afterwards.
        outcome: Box<RepairOutcome>,
    },
    /// Abandoned by the user, or by the project closing. Terminal.
    Discarded,
}

/// What a repair session is, as the interface sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RepairPhase {
    /// The engine is working.
    Running,
    /// It finished. Whether the result may be applied is a separate question.
    Finished,
    /// An apply is in flight.
    Applying,
    /// Applied.
    Applied,
    /// Abandoned.
    Discarded,
}

/// Why an apply was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// The worker has not finished.
    StillRunning,
    /// Another caller already claimed the apply.
    AlreadyApplying,
    /// This session was already applied, as this timetable.
    AlreadyApplied {
        /// The timetable it became.
        timetable_id: Uuid,
    },
    /// The session was discarded.
    Discarded,
    /// The outcome is not a timetable.
    NotApplyable {
        /// What was wrong, in a sentence.
        reason: String,
    },
}

/// A repair session's lifecycle, with no scheduling context attached.
///
/// Split out for the reason [`super::session::Lifecycle`] is: every hazard is about a
/// *transition*, and a transition needs only the outcome. Keeping the state machine free of the
/// `Assembly` means its tests are pure — no project file, no migration, no solve.
#[derive(Debug)]
pub struct RepairLifecycle {
    cancel: Arc<AtomicBool>,
    progress: Mutex<Progress>,
    state: Mutex<State>,
}

impl Default for RepairLifecycle {
    fn default() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Mutex::new(Progress::default()),
            state: Mutex::new(State::Running),
        }
    }
}

impl ProgressReporter for RepairLifecycle {
    fn report(&self, progress: Progress) {
        Self::report(self, progress);
    }
}

impl RepairLifecycle {
    /// A handle the engine can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        CancelFlag::from_flag(Arc::clone(&self.cancel))
    }

    /// Asks the run to stop.
    ///
    /// Returns whether the request was made while something was still running. Sets **no
    /// state**: the worker is the only thing that leaves `Running`, so a cancel arriving as the
    /// last placement commits simply loses the race and the run reports what it found.
    pub fn request_cancel(&self) -> bool {
        let running = matches!(&*self.lock_state(), State::Running);
        self.cancel.store(true, Ordering::SeqCst);
        running
    }

    /// Whether a cancel has been requested.
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Records progress from the worker.
    pub fn report(&self, progress: Progress) {
        if let Ok(mut held) = self.progress.lock() {
            *held = progress;
        }
    }

    /// The latest progress reported.
    pub fn progress(&self) -> Progress {
        self.progress.lock().map(|p| *p).unwrap_or_default()
    }

    /// Records what the worker found. Only from `Running`.
    pub fn finish(&self, outcome: RepairOutcome) {
        let mut state = self.lock_state();
        if matches!(&*state, State::Running) {
            *state = State::Finished(Box::new(outcome));
        }
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> RepairPhase {
        match &*self.lock_state() {
            State::Running => RepairPhase::Running,
            State::Finished(_) => RepairPhase::Finished,
            State::Applying(_) => RepairPhase::Applying,
            State::Applied { .. } => RepairPhase::Applied,
            State::Discarded => RepairPhase::Discarded,
        }
    }

    /// The timetable this session became, where it was applied.
    pub fn applied_as(&self) -> Option<Uuid> {
        match &*self.lock_state() {
            State::Applied { timetable_id, .. } => Some(*timetable_id),
            _ => None,
        }
    }

    /// The outcome, for a session that has one.
    pub fn outcome(&self) -> Option<RepairOutcome> {
        match &*self.lock_state() {
            State::Finished(outcome)
            | State::Applying(outcome)
            | State::Applied { outcome, .. } => Some((**outcome).clone()),
            State::Running | State::Discarded => None,
        }
    }

    /// Abandons the session.
    pub fn discard(&self) -> bool {
        self.cancel.store(true, Ordering::SeqCst);
        let mut state = self.lock_state();
        if matches!(&*state, State::Discarded) {
            return false;
        }
        *state = State::Discarded;
        true
    }

    /// Claims the right to apply, exactly once.
    ///
    /// The compare-and-swap into `Applying` happens **here**, before any database work, because
    /// two clicks otherwise both pass a "has it finished?" test and both write.
    pub fn claim_apply(&self) -> Result<RepairOutcome, ApplyRefusal> {
        let mut state = self.lock_state();
        match &*state {
            State::Running => Err(ApplyRefusal::StillRunning),
            State::Applying(_) => Err(ApplyRefusal::AlreadyApplying),
            State::Applied { timetable_id, .. } => Err(ApplyRefusal::AlreadyApplied {
                timetable_id: *timetable_id,
            }),
            State::Discarded => Err(ApplyRefusal::Discarded),
            State::Finished(outcome) => {
                if let Some(reason) = unapplyable_reason(outcome) {
                    return Err(ApplyRefusal::NotApplyable { reason });
                }
                let outcome = outcome.clone();
                *state = State::Applying(outcome.clone());
                Ok(*outcome)
            }
        }
    }

    /// Records a completed apply. Terminal.
    pub fn apply_succeeded(&self, timetable_id: Uuid) {
        let mut state = self.lock_state();
        if let State::Applying(outcome) = &*state {
            *state = State::Applied {
                timetable_id,
                outcome: outcome.clone(),
            };
        }
    }

    /// Returns a claimed apply to `Finished` so it can be retried.
    pub fn apply_failed(&self) {
        let mut state = self.lock_state();
        if let State::Applying(outcome) = &*state {
            *state = State::Finished(outcome.clone());
        }
    }

    /// Why this session's result cannot be applied, or `None` if it can.
    pub fn applyable(&self) -> Option<String> {
        match &*self.lock_state() {
            State::Finished(outcome) | State::Applying(outcome) => unapplyable_reason(outcome),
            State::Applied { .. } => None,
            State::Running => Some("the repair has not finished".to_owned()),
            State::Discarded => Some("the repair was discarded".to_owned()),
        }
    }

    /// Takes the state lock, recovering from poisoning.
    ///
    /// A poisoned lock means a previous command panicked while holding it. The state is still
    /// readable, and refusing every later call would strand a result the user can see on screen.
    /// The workspace lock is the one that treats poisoning as fatal, because that guards a
    /// database.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Why an outcome is not a timetable, or `None` if it is.
///
/// Reads [`RepairOutcome::applyable`] rather than re-deriving the rule: the solver computed it
/// from the closed list in `M9B-REPAIR-PROPOSAL.md` §2.5, and a second opinion here is exactly
/// the kind of divergence that lets an unsafe result through.
fn unapplyable_reason(outcome: &RepairOutcome) -> Option<String> {
    if outcome.applyable {
        return None;
    }
    if let Some(defect) = &outcome.defect {
        return Some(format!(
            "this repair is internally inconsistent ({}), which is a defect in Evara rather \
             than a problem with the timetable",
            defect.key()
        ));
    }
    Some(format!(
        "the repair {}, so there is nothing to apply",
        outcome.outcome.termination.key()
    ))
}

/// One repair, from start to applied or discarded.
///
/// Holds the `Assembly` the outcome was built from, for the reason
/// [`super::session::Session`] does: `save_repaired` refuses an outcome whose identity does not
/// match the assembly it is offered with, and reloading the assembly at apply time would defeat
/// that check.
pub struct RepairSession {
    /// This run's identity, carried on every event.
    pub run_id: Uuid,
    /// Which school, cycle and term.
    pub selection: SchedulingSelection,
    /// The timetable being repaired. Never modified.
    pub source: Uuid,
    /// Its revision when the baseline was read.
    pub source_rev: i64,
    /// The scope the user chose, as recorded on the run.
    pub scope: RepairScope,
    /// The problem the outcome was built from, kept for the apply.
    pub assembly: Arc<Assembly>,
    /// The display names, kept so the preview needs no second read.
    pub labels: Arc<LabelSet>,
    /// The rules the run was judged by.
    pub constraints: Arc<Vec<CompiledConstraint>>,
    /// The arrangement as stored, which the apply re-checks against.
    pub baseline: Arc<Assignment>,
    /// What the repair was allowed to move.
    pub movable: Arc<MovableSet>,
    /// The exact `(entry id, rev)` list the baseline was read at.
    pub stamp: Arc<ArrangementStamp>,
    /// The flags to carry into the new version.
    pub flags: Arc<Vec<(Uuid, EntryFlags)>>,
    /// Which project this belongs to.
    pub epoch: u64,
    /// When the caller started it, ISO-8601 UTC.
    pub started_at: String,
    life: Arc<RepairLifecycle>,
}

impl std::fmt::Debug for RepairSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepairSession")
            .field("run_id", &self.run_id)
            .field("source", &self.source)
            .field("scope", &self.scope.key())
            .field("epoch", &self.epoch)
            .field("life", &self.life)
            .finish_non_exhaustive()
    }
}

/// Everything a session needs at construction, bundled so the call is readable.
pub struct RepairSessionInput {
    /// This run's identity.
    pub run_id: Uuid,
    /// Which school, cycle and term.
    pub selection: SchedulingSelection,
    /// The timetable being repaired.
    pub source: Uuid,
    /// Its revision when the baseline was read.
    pub source_rev: i64,
    /// The scope the user chose.
    pub scope: RepairScope,
    /// The compiled problem.
    pub assembly: Arc<Assembly>,
    /// The display names.
    pub labels: Arc<LabelSet>,
    /// The rules.
    pub constraints: Arc<Vec<CompiledConstraint>>,
    /// The stored arrangement.
    pub baseline: Arc<Assignment>,
    /// What may move.
    pub movable: Arc<MovableSet>,
    /// The arrangement stamp.
    pub stamp: Arc<ArrangementStamp>,
    /// The flags to preserve.
    pub flags: Arc<Vec<(Uuid, EntryFlags)>>,
    /// The project epoch.
    pub epoch: u64,
    /// When the caller started it.
    pub started_at: String,
}

impl RepairSession {
    /// Starts a session in `Running`.
    #[must_use]
    pub fn new(input: RepairSessionInput) -> Self {
        Self {
            run_id: input.run_id,
            selection: input.selection,
            source: input.source,
            source_rev: input.source_rev,
            scope: input.scope,
            assembly: input.assembly,
            labels: input.labels,
            constraints: input.constraints,
            baseline: input.baseline,
            movable: input.movable,
            stamp: input.stamp,
            flags: input.flags,
            epoch: input.epoch,
            started_at: input.started_at,
            life: Arc::new(RepairLifecycle::default()),
        }
    }

    /// The lifecycle, for the worker thread and the progress sink.
    pub fn lifecycle(&self) -> Arc<RepairLifecycle> {
        Arc::clone(&self.life)
    }

    /// A handle the engine can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        self.life.cancellation()
    }

    /// Asks the run to stop.
    pub fn request_cancel(&self) -> bool {
        self.life.request_cancel()
    }

    /// Whether a cancel has been requested.
    pub fn cancel_requested(&self) -> bool {
        self.life.cancel_requested()
    }

    /// Records what the worker found.
    pub fn finish(&self, outcome: RepairOutcome) {
        self.life.finish(outcome);
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> RepairPhase {
        self.life.phase()
    }

    /// The latest progress reported.
    pub fn progress(&self) -> Progress {
        self.life.progress()
    }

    /// The outcome, for a session that has one.
    pub fn outcome(&self) -> Option<RepairOutcome> {
        self.life.outcome()
    }

    /// The timetable this session became.
    pub fn applied_as(&self) -> Option<Uuid> {
        self.life.applied_as()
    }

    /// Why the result cannot be applied, or `None`.
    pub fn applyable(&self) -> Option<String> {
        self.life.applyable()
    }

    /// Claims the right to apply, exactly once.
    pub fn claim_apply(&self) -> Result<RepairOutcome, ApplyRefusal> {
        self.life.claim_apply()
    }

    /// Records a completed apply.
    pub fn apply_succeeded(&self, timetable_id: Uuid) {
        self.life.apply_succeeded(timetable_id);
    }

    /// Returns a claimed apply to `Finished`.
    pub fn apply_failed(&self) {
        self.life.apply_failed();
    }

    /// Abandons the session.
    pub fn discard(&self) -> bool {
        self.life.discard()
    }
}

/// Every repair session this process holds.
///
/// Cleared when a project closes, for the reason the generation registry is: a session's
/// `Assembly` describes a database that is no longer open.
#[derive(Debug, Default)]
pub struct RepairSessions {
    sessions: Mutex<std::collections::BTreeMap<Uuid, Arc<RepairSession>>>,
}

impl RepairSessions {
    /// Registers a session.
    pub fn insert(&self, session: Arc<RepairSession>) {
        if let Ok(mut held) = self.sessions.lock() {
            held.insert(session.run_id, session);
        }
    }

    /// One session, if this process still holds it.
    pub fn get(&self, run_id: Uuid) -> Option<Arc<RepairSession>> {
        self.sessions
            .lock()
            .ok()
            .and_then(|held| held.get(&run_id).map(Arc::clone))
    }

    /// Forgets one session, having asked it to stop.
    pub fn remove(&self, run_id: Uuid) -> bool {
        let Ok(mut held) = self.sessions.lock() else {
            return false;
        };
        held.remove(&run_id).is_some_and(|session| {
            session.discard();
            true
        })
    }

    /// How many sessions are still running.
    pub fn running(&self) -> usize {
        self.sessions.lock().map_or(0, |held| {
            held.values()
                .filter(|session| session.phase() == RepairPhase::Running)
                .count()
        })
    }

    /// How many sessions are held at all.
    pub fn len(&self) -> usize {
        self.sessions.lock().map_or(0, |held| held.len())
    }

    /// Whether any session is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Discards and forgets every session.
    ///
    /// Takes no `Result` for the reason the generation registry's `clear` does not: a poisoned
    /// lock must not stop a project from closing.
    pub fn clear(&self) {
        if let Ok(mut held) = self.sessions.lock() {
            for session in held.values() {
                session.discard();
            }
            held.clear();
        }
    }
}

#[cfg(test)]
mod tests;
