//! The generation session: one solve's lifecycle, as an explicit state machine.
//!
//! # Why a state machine and not a few booleans
//!
//! A generate-then-save workflow has five ways to go wrong that no single flag catches, and
//! every one of them is reachable from an ordinary interface:
//!
//! | Hazard | What goes wrong without a state | Guarded by |
//! | --- | --- | --- |
//! | **Saving an unfinished run** | the worker is still placing lessons; a save would write half an arrangement | [`State::Running`] refuses |
//! | **Saving a refused or broken run** | `Termination::InputRejected` attempted nothing and `InternalCheckFailed` produced an arrangement the reference scorer rejected — neither is a timetable | [`Session::saveable`] |
//! | **Two saves from two clicks** | both pass a `finished?` check, both write, two timetables appear | [`State::Saving`], claimed by a compare-and-swap |
//! | **A cancel racing a finish** | cancel arrives as the worker commits its last placement | [`Session::request_cancel`] sets a flag and *never* sets a state |
//! | **A save into a different project** | the project was closed and another opened while the solve ran | [`Session::epoch`] |
//!
//! # The states, and the only transitions
//!
//! ```text
//!                  ┌──────────────────────── discard ──────────────┐
//!                  v                                               │
//!   start ──► Running ──► Finished ──► Saving ──► Saved            │
//!                            ^           │                        │
//!                            └─ failed ──┘  (the write is retryable)
//! ```
//!
//! `Saved` and `Discarded` are terminal. **Nothing leaves `Saved`**, which is what makes a
//! duplicate save impossible rather than unlikely.
//!
//! # Cancellation is a request, not a transition
//!
//! [`Session::request_cancel`] sets an `AtomicBool` the engine reads between construction
//! steps and changes no state at all. The worker then finishes normally and reports
//! `Termination::Cancelled`, so there is exactly one path out of `Running` and no race
//! between a cancel and a completion. A cancel after the run has finished returns `false`
//! and changes nothing — which is the honest answer, because there was nothing left to stop.
//!
//! # No database lock is held while the engine runs
//!
//! The command thread takes the workspace lock, compiles an [`Assembly`], reads the labels,
//! compiles the rules, **releases the lock**, and moves the owned assembly to a dedicated
//! thread. `ARCHITECTURE.md` §3.3 is what makes that possible — the solver *"receives a fully
//! materialised problem in memory"* and never opens a database — and
//! `evara_db::scheduling`'s `send_assertions` test is what keeps it possible.
//!
//! Holding the lock for the ~900 ms a 2 000-occurrence solve takes would freeze every other
//! command in the application, and `Scheduling` borrows `&Database` so the borrow could not
//! cross the thread boundary in any case.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use evara_core::scheduling::display::LabelSet;
use evara_core::scheduling::{CompiledConstraint, SolveOutcome, SolveStatus, Termination};
use evara_db::scheduling::{Assembly, SchedulingSelection};
use evara_solver::{Cancel, ProgressSink};
use serde::Serialize;
use uuid::Uuid;

/// Monotonic counter identifying which project a session belongs to.
///
/// Bumped whenever a project is created, opened or closed. A session records the value it
/// started under, and every later command compares: a solve begun against Northgate's file
/// cannot be saved into the one the user opened afterwards.
///
/// A counter rather than the project's path, deliberately. A path can be re-opened and would
/// then compare equal, and *"the same file re-opened"* is still a different database handle
/// with a different change-log position — so the watermark the outcome was built against is
/// no longer the one a save would be checked against.
#[derive(Debug, Default)]
pub struct ProjectEpoch(AtomicU64);

impl ProjectEpoch {
    /// The current epoch.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Moves to a new epoch, invalidating every session of the previous one.
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// How far a session has got.
///
/// Kept apart from [`SolveOutcome`] because they answer different questions: an outcome says
/// what the engine found, and a state says what the *application* may still do about it.
#[derive(Debug)]
enum State {
    /// The worker is running. Nothing may be saved.
    Running,
    /// The worker finished. What it found is here, whatever it says.
    Finished(Box<SolveOutcome>),
    /// A save has been claimed by one caller and is in flight.
    ///
    /// The compare-and-swap that makes a second concurrent save impossible. Holds the outcome
    /// so a failed write can put it back.
    Saving(Box<SolveOutcome>),
    /// Saved, as this timetable. Terminal.
    Saved {
        /// The timetable that was written.
        timetable_id: Uuid,
        /// What was saved, so the preview still renders after the save.
        outcome: Box<SolveOutcome>,
    },
    /// Abandoned by the user, or by the project closing. Terminal.
    Discarded,
}

/// What a session is, as the interface sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum SessionPhase {
    /// The engine is working.
    Running,
    /// It finished. Whether the result is saveable is a separate question — see
    /// `GenerateStatus::saveable`.
    Finished,
    /// A save is in flight.
    Saving,
    /// Saved.
    Saved,
    /// Abandoned.
    Discarded,
}

/// How far the engine has got, as the progress events report it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    /// Occurrences placed so far.
    pub placed: usize,
    /// Occurrences the problem holds.
    pub total: usize,
    /// Attempts made, retries included.
    pub steps: u64,
}

/// A session's lifecycle, with no scheduling context attached.
///
/// Split out of [`Session`] on purpose: the hazards in this module's header are all about
/// *transitions*, and a transition needs only a [`SolveOutcome`]. Keeping the state machine
/// free of the `Assembly` means its tests are pure — no project file, no migration, no solve
/// — so every one of the five hazards is covered by a test that runs in microseconds.
#[derive(Debug)]
pub struct Lifecycle {
    cancel: Arc<AtomicBool>,
    progress: Mutex<Progress>,
    state: Mutex<State>,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Mutex::new(Progress::default()),
            state: Mutex::new(State::Running),
        }
    }
}

impl Lifecycle {
    /// A handle the engine can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        CancelFlag(Arc::clone(&self.cancel))
    }

    /// Asks the run to stop. See [`Session::request_cancel`].
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

    /// Records what the worker found. Only from [`State::Running`].
    pub fn finish(&self, outcome: SolveOutcome) {
        let mut state = self.lock_state();
        if matches!(&*state, State::Running) {
            *state = State::Finished(Box::new(outcome));
        }
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> SessionPhase {
        match &*self.lock_state() {
            State::Running => SessionPhase::Running,
            State::Finished(_) => SessionPhase::Finished,
            State::Saving(_) => SessionPhase::Saving,
            State::Saved { .. } => SessionPhase::Saved,
            State::Discarded => SessionPhase::Discarded,
        }
    }

    /// The timetable this session was saved as, where it was.
    pub fn saved_as(&self) -> Option<Uuid> {
        match &*self.lock_state() {
            State::Saved { timetable_id, .. } => Some(*timetable_id),
            _ => None,
        }
    }

    /// The outcome, for a session that has one.
    pub fn outcome(&self) -> Option<SolveOutcome> {
        match &*self.lock_state() {
            State::Finished(outcome) | State::Saving(outcome) | State::Saved { outcome, .. } => {
                Some((**outcome).clone())
            }
            State::Running | State::Discarded => None,
        }
    }

    /// Abandons the session. See [`Session::discard`].
    pub fn discard(&self) -> bool {
        self.cancel.store(true, Ordering::SeqCst);
        let mut state = self.lock_state();
        if matches!(&*state, State::Discarded) {
            return false;
        }
        *state = State::Discarded;
        true
    }

    /// Claims the right to save, exactly once. See [`Session::claim_save`].
    pub fn claim_save(&self) -> Result<SolveOutcome, SaveRefusal> {
        let mut state = self.lock_state();
        match &*state {
            State::Running => Err(SaveRefusal::StillRunning),
            State::Saving(_) => Err(SaveRefusal::AlreadySaving),
            State::Saved { timetable_id, .. } => Err(SaveRefusal::AlreadySaved {
                timetable_id: *timetable_id,
            }),
            State::Discarded => Err(SaveRefusal::Discarded),
            State::Finished(outcome) => {
                if let Some(reason) = unsaveable_reason(outcome) {
                    return Err(SaveRefusal::NotSaveable { reason });
                }
                let outcome = outcome.clone();
                *state = State::Saving(outcome.clone());
                Ok(*outcome)
            }
        }
    }

    /// Records a completed save. Terminal.
    pub fn save_succeeded(&self, timetable_id: Uuid) {
        let mut state = self.lock_state();
        if let State::Saving(outcome) = &*state {
            *state = State::Saved {
                timetable_id,
                outcome: outcome.clone(),
            };
        }
    }

    /// Returns a claimed save to [`State::Finished`] so it can be retried.
    pub fn save_failed(&self) {
        let mut state = self.lock_state();
        if let State::Saving(outcome) = &*state {
            *state = State::Finished(outcome.clone());
        }
    }

    /// Why this session's result cannot be saved, or `None` if it can.
    pub fn saveable(&self) -> Option<&'static str> {
        match &*self.lock_state() {
            State::Finished(outcome) | State::Saving(outcome) => unsaveable_reason(outcome),
            State::Saved { .. } => None,
            State::Running => Some("the run has not finished"),
            State::Discarded => Some("the run was discarded"),
        }
    }

    /// Takes the state lock, recovering from poisoning.
    ///
    /// A poisoned lock here means a previous command panicked while holding it. The state is
    /// still readable and the alternative — refusing every later call about this session —
    /// would strand a result the user can see on screen. The workspace lock is the one that
    /// treats poisoning as fatal, because that one guards a database.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One generation, from start to saved or discarded.
///
/// The scheduling context, plus a [`Lifecycle`]. Every lifecycle method is re-exposed here by
/// delegation rather than by making the field public, so a caller cannot reach a transition
/// without going through the session that owns it.
///
/// Holds the `Assembly` because the save needs **that** one: `save_generated` refuses
/// `DbError::Invalid` when the outcome's `ProblemIdentity` does not match the assembly it is
/// offered with, and that check exists because an outcome written against a different
/// assembly's index would translate every dense index to the wrong identifier and store a
/// complete, plausible, wrong timetable (`TIMETABLE-PERSISTENCE.md` §9). Reloading the
/// assembly at save time would defeat it.
pub struct Session {
    /// This run's identity, carried on every event.
    pub run_id: Uuid,
    /// Which school, cycle and term.
    pub selection: SchedulingSelection,
    /// The problem the outcome was built from, kept for the save.
    pub assembly: Arc<Assembly>,
    /// The display names, kept so the preview needs no second read.
    pub labels: Arc<LabelSet>,
    /// The rules the run was judged by.
    pub constraints: Arc<Vec<CompiledConstraint>>,
    /// Which project this belongs to.
    pub epoch: u64,
    /// When the caller started it, ISO-8601 UTC.
    pub started_at: String,
    /// Where in its lifecycle it is.
    ///
    /// Behind an `Arc` so the worker thread and the progress sink can hold it without
    /// holding the scheduling context — which is also what makes the state machine's tests
    /// pure, since a transition needs only a `SolveOutcome`.
    life: Arc<Lifecycle>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("run_id", &self.run_id)
            .field("epoch", &self.epoch)
            .field("life", &self.life)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts a session in `Running`.
    pub fn new(
        run_id: Uuid,
        selection: SchedulingSelection,
        assembly: Arc<Assembly>,
        labels: Arc<LabelSet>,
        constraints: Arc<Vec<CompiledConstraint>>,
        epoch: u64,
        started_at: String,
    ) -> Self {
        Self {
            run_id,
            selection,
            assembly,
            labels,
            constraints,
            epoch,
            started_at,
            life: Arc::new(Lifecycle::default()),
        }
    }

    /// The lifecycle, for the worker thread and the progress sink.
    pub fn lifecycle(&self) -> Arc<Lifecycle> {
        Arc::clone(&self.life)
    }

    /// A handle the engine can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        self.life.cancellation()
    }

    /// Asks the run to stop.
    ///
    /// Returns whether the request was made *while something was still running*. `false` for
    /// a run that had already finished, which is not an error: there was nothing to stop, and
    /// reporting success would tell the user their cancel did something it did not.
    ///
    /// Sets no state. The worker is the only thing that leaves `Running`, so a cancel
    /// arriving as the last placement commits simply loses the race and the run reports
    /// whatever it found — deterministically, with no window in which the session is both
    /// running and finished.
    pub fn request_cancel(&self) -> bool {
        self.life.request_cancel()
    }

    /// Whether a cancel has been requested.
    pub fn cancel_requested(&self) -> bool {
        self.life.cancel_requested()
    }

    /// Records progress from the worker.
    pub fn report(&self, progress: Progress) {
        self.life.report(progress);
    }

    /// The latest progress reported.
    pub fn progress(&self) -> Progress {
        self.life.progress()
    }

    /// Records what the worker found.
    ///
    /// Only from `Running`. A session discarded while the worker ran stays discarded: the
    /// user said they did not want it, and a result arriving afterwards must not resurrect it.
    pub fn finish(&self, outcome: SolveOutcome) {
        self.life.finish(outcome);
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> SessionPhase {
        self.life.phase()
    }

    /// The timetable this session was saved as, where it was.
    pub fn saved_as(&self) -> Option<Uuid> {
        self.life.saved_as()
    }

    /// The outcome, for a session that has one.
    ///
    /// Available once finished and after a save, because a preview must keep rendering rather
    /// than emptying the screen at the moment of success.
    pub fn outcome(&self) -> Option<SolveOutcome> {
        self.life.outcome()
    }

    /// Abandons the session, cancelling the run if it is still going.
    pub fn discard(&self) -> bool {
        self.life.discard()
    }

    /// Claims the right to save, exactly once.
    ///
    /// # The compare-and-swap that stops a double click writing twice
    ///
    /// Two concurrent `timetable_save` calls both pass a *"has it finished?"* test, and the
    /// workspace mutex only serialises them — it does not make the second one notice the
    /// first. So the claim and the state change happen together, under this session's own
    /// lock, before any database work begins.
    ///
    /// On success the caller **must** call [`Session::save_succeeded`] or
    /// [`Session::save_failed`]; the latter puts the outcome back so a write refused for a
    /// reason the user can fix is retryable.
    pub fn claim_save(&self) -> Result<SolveOutcome, SaveRefusal> {
        self.life.claim_save()
    }

    /// Records a completed save. Terminal.
    pub fn save_succeeded(&self, timetable_id: Uuid) {
        self.life.save_succeeded(timetable_id);
    }

    /// Returns a claimed save to `Finished` so it can be retried.
    pub fn save_failed(&self) {
        self.life.save_failed();
    }

    /// Why this session's result cannot be saved, or `None` if it can.
    pub fn saveable(&self) -> Option<&'static str> {
        self.life.saveable()
    }
}

/// Every generation this process has started, by run.
///
/// # Why they are kept after they finish
///
/// A finished session holds the preview the user is looking at and the `Assembly` a save
/// needs. Dropping it on completion would mean the result could be rendered once and never
/// saved. They are dropped when the user discards one, and all of them when the project
/// closes — see [`GenerationSessions::clear`].
///
/// A `BTreeMap` rather than a `HashMap`: nothing in this workspace iterates a hash map, and
/// [`GenerationSessions::running`] is read by a test that asserts an order.
#[derive(Debug, Default)]
pub struct GenerationSessions {
    sessions: Mutex<std::collections::BTreeMap<Uuid, Arc<Session>>>,
}

impl GenerationSessions {
    /// Records a new session.
    pub fn insert(&self, session: Arc<Session>) {
        if let Ok(mut held) = self.sessions.lock() {
            held.insert(session.run_id, session);
        }
    }

    /// One session by run.
    pub fn get(&self, run_id: Uuid) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .ok()
            .and_then(|held| held.get(&run_id).map(Arc::clone))
    }

    /// Abandons one session and forgets it. Returns whether there was one.
    ///
    /// The session is discarded **before** being removed, so a worker still running sees the
    /// cancel flag even though nothing holds the map entry any more — the thread holds its
    /// own `Arc`.
    pub fn discard(&self, run_id: Uuid) -> bool {
        let Ok(mut held) = self.sessions.lock() else {
            return false;
        };
        match held.remove(&run_id) {
            Some(session) => {
                session.discard();
                true
            }
            None => false,
        }
    }

    /// How many runs are still going.
    pub fn running(&self) -> usize {
        self.sessions.lock().map_or(0, |held| {
            held.values()
                .filter(|session| session.phase() == SessionPhase::Running)
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
    /// Abandons every session and forgets them all.
    ///
    /// Called when the project closes. A session names a school of the project being closed
    /// and holds an `Assembly` describing a database that is about to be shut; neither can
    /// survive it. Every one is **discarded** first, so a worker mid-solve stops rather than
    /// continuing to compute an answer nobody can save.
    ///
    /// Takes no `Result`, for the reason `ImportSessions::clear` does not: a poisoned lock
    /// here means a previous command panicked, and refusing to close the project because of
    /// it would trap the user in the state they are trying to leave.
    pub fn clear(&self) {
        if let Ok(mut held) = self.sessions.lock() {
            for session in held.values() {
                session.discard();
            }
            held.clear();
        }
    }
}

/// Why a save was refused, before any database work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveRefusal {
    /// The engine is still working.
    StillRunning,
    /// Another save of this session is in flight.
    AlreadySaving,
    /// This session has already been saved.
    AlreadySaved {
        /// What it was saved as.
        timetable_id: Uuid,
    },
    /// The session was abandoned.
    Discarded,
    /// The outcome is not a timetable.
    NotSaveable {
        /// Which of the two cases, in words.
        reason: &'static str,
    },
}

/// Whether an outcome is a timetable at all, and why not when it is not.
///
/// The two cases `TIMETABLE-PERSISTENCE.md` §9 names, checked here so the interface refuses
/// before a transaction opens rather than translating a `DbError` afterwards. The repository
/// refuses them too — this is the earlier of two guards, not the only one.
fn unsaveable_reason(outcome: &SolveOutcome) -> Option<&'static str> {
    match outcome.termination {
        // Pre-solve validation refused the inputs, so nothing was attempted. The findings
        // belong to the validation report, not to a timetable.
        Termination::InputRejected => Some(
            "generation was refused because the inputs are not fit to solve, \
             so there is no timetable to save",
        ),
        // The reference scorer rejected what the engine built. This is a defect in Evara and
        // is reported as one — never as a timetable.
        Termination::InternalCheckFailed => Some(
            "the arrangement failed Evara's own final check. This is a defect in Evara \
             and the timetable is deliberately not saveable",
        ),
        _ => match outcome.status {
            SolveStatus::Failed => Some("the run failed, so there is no timetable to save"),
            SolveStatus::Infeasible => {
                Some("the run produced no timetable, so there is nothing to save")
            }
            SolveStatus::Complete | SolveStatus::Partial | SolveStatus::Cancelled => None,
        },
    }
}

/// A [`Cancel`] backed by the session's flag.
#[derive(Debug, Clone)]
pub struct CancelFlag(Arc<AtomicBool>);

impl Cancel for CancelFlag {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A [`ProgressSink`] that records into the session and coalesces its events.
///
/// # Why it coalesces
///
/// `ProgressSink::placed` is called once per committed placement and once per occurrence given
/// up on — about 2 000 times for the realistic upper end. Emitting 2 000 Tauri events is
/// affordable and wasteful, and it floods an `aria-live` region a screen reader is reading.
/// So an event is emitted at most once per [`Self::INTERVAL_MS`], **plus** the final one,
/// which the caller emits after the worker returns.
///
/// The clock read here is the *caller's*, which is the division `SOLVER.md` §5 requires: the
/// engine reads no clock, so the same seed still stops in the same place.
pub struct CoalescingSink<F> {
    life: Arc<Lifecycle>,
    emit: F,
    last: Mutex<Option<std::time::Instant>>,
}

impl<F: Fn(Progress)> CoalescingSink<F> {
    /// The shortest gap between two emitted progress events.
    pub const INTERVAL_MS: u128 = 50;

    /// Wraps a lifecycle and an emitter.
    pub fn new(life: Arc<Lifecycle>, emit: F) -> Self {
        Self {
            life,
            emit,
            last: Mutex::new(None),
        }
    }
}

impl<F: Fn(Progress)> ProgressSink for CoalescingSink<F> {
    fn placed(&self, placed: usize, total: usize, steps: u64) {
        let progress = Progress {
            placed,
            total,
            steps,
        };
        // Recorded on every call, so `generate_status` is always current even between events.
        self.life.report(progress);

        let now = std::time::Instant::now();
        let due = match self.last.lock() {
            Ok(mut last) => {
                let due =
                    last.is_none_or(|at| now.duration_since(at).as_millis() >= Self::INTERVAL_MS);
                if due {
                    *last = Some(now);
                }
                due
            }
            // A poisoned lock must not silence progress entirely; emitting is harmless.
            Err(_) => true,
        };
        if due {
            (self.emit)(progress);
        }
    }
}

#[cfg(test)]
mod tests;
