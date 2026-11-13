//! A transfer session: one export or one import, as an explicit state machine.
//!
//! # Why a state machine and not a few booleans
//!
//! D9 of [the transfer proposal](../../../../../docs/M10-PROJECT-TRANSFER-PROPOSAL.md) says a
//! transfer session is the generation session's state machine, and four of
//! [`super::session`]'s five hazards are transfer's too:
//!
//! | Hazard | What goes wrong without a state | Guarded by |
//! | --- | --- | --- |
//! | **Reading the result of an unfinished run** | the worker is still compressing; a report read now would describe an archive that does not exist yet | [`State::Running`] carries no report |
//! | **Two workers for one session** | both write the same archive, or both claim the same destination | [`TransferLifecycle::claim_start`], a compare-and-swap, and [`TransferSessions::insert`], which refuses an identifier it already holds |
//! | **A cancel racing a finish** | the cancel arrives as the last byte is renamed into place | [`TransferLifecycle::request_cancel`] sets a flag and sets **no** state |
//! | **Work aimed at a different project** | the project was closed and another opened while the export ran | [`TransferSession::epoch`] |
//!
//! ```text
//!                  ┌────────────── discard ──────────────┐
//!                  v                                     │
//!   start ──► Running ──┬──► Finished                    │
//!                       ├──► Failed                      │
//!                       └──► Cancelled ──────────────────┘
//! ```
//!
//! `Finished`, `Failed`, `Cancelled` and `Discarded` are terminal, and **nothing leaves
//! them**. A transfer has no second write to claim — unlike a generation, where the solve and
//! the save are two acts — so there is no `Saving` state here and no retryable middle.
//!
//! # Cancellation is a request, not a transition
//!
//! [`TransferLifecycle::request_cancel`] sets an `AtomicBool` that
//! `evara_core::scheduling::Cancel` reads between steps, and changes no state at all. The
//! worker is the only thing that leaves `Running`, so a cancel arriving as `seal` finalises
//! simply loses the race and the session reports what actually happened. That is also why the
//! protocol's own guarantees hold: a cancelled export leaves no `.evara` and no `.part`, and a
//! cancelled import leaves no destination and no staging, because `evara-project` checks the
//! flag at points it has chosen rather than being interrupted at an arbitrary one.
//!
//! # No workspace lock is held while the archive is written
//!
//! `ARCHITECTURE.md` §3.3's arrangement, which `evara_project::seal` is shaped for: it takes
//! an owned [`evara_db::transfer::Snapshot`] and **no** `&Database`, so the command thread
//! takes the lock, snapshots, releases the lock, and moves the snapshot to a worker. D8.
//! Compressing a multi-hundred-megabyte payload with the project held open would block every
//! other command for the whole of it.
//!
//! An import never holds the lock either: it writes a *different* file, and its journal is
//! reached through the installation's settings database at
//! [`evara_db::Workspace::settings_path`] rather than through this workspace's handle.
//!
//! # Retention is bounded
//!
//! A finished session holds a small report and nothing else — no connection, no compiled
//! problem, no archive bytes — so keeping one is cheap. Keeping an unbounded number of them
//! is still a leak, so [`TransferSessions`] evicts the oldest **terminal** session once it
//! holds more than [`TransferSessions::MAX_RETAINED`]. A running session is never evicted.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evara_project::Phase;
use serde::Serialize;
use uuid::Uuid;

use super::project::ProjectError;
use super::session::{CancelFlag, Coalescer};

/// Which of the two operations a session is.
///
/// Carried on the session rather than inferred from which command asked, so
/// `project_export_cancel` handed an *import*'s identifier refuses instead of stopping
/// something the caller did not name. That is the "only authorised operations can be
/// cancelled" rule, and it is a property of the registry rather than of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TransferKind {
    /// Writing a portable `.evara` archive from the open project.
    Export,
    /// Reading a portable `.evara` archive into a project file.
    Import,
}

impl TransferKind {
    /// What a human calls it, for a message.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Export => "export",
            Self::Import => "import",
        }
    }
}

/// Where a session is in its lifecycle.
///
/// Deliberately **not** the same thing as [`Phase`], which is where the *work* has got to.
/// `GenerateStatus` and `RepairStatus` already use `phase` for a session's lifecycle in this
/// crate, so a transfer status uses it the same way and names the work phase `step`. The
/// proposal's one-line event sketch wrote `phase` for the work phase; one word for two
/// things, on one screen, is the worse outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TransferPhase {
    /// The worker is running.
    Running,
    /// It finished, and there is a report.
    Finished,
    /// It did not finish, for a reason the status carries.
    Failed,
    /// The user stopped it. Nothing was written — the protocol's own guarantee.
    Cancelled,
    /// Forgotten by the user, or by the project closing.
    Discarded,
}

/// How far the work has got, as the progress events report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    /// Which part of the transfer is running.
    pub step: Phase,
    /// How much of it is done, in the unit `step` counts in.
    pub done: u64,
    /// How much there is, or `0` when it is not yet known.
    pub total: u64,
}

impl Default for Work {
    fn default() -> Self {
        Self {
            // The first phase of either direction: an export snapshots, an import stages.
            // Never a zero value of its own, because there is no "no phase" — a running
            // transfer is always doing one of the seven.
            step: Phase::Snapshotting,
            done: 0,
            total: 0,
        }
    }
}

/// What a finished transfer produced.
///
/// One enum rather than two session types, because every hazard above is about a transition
/// and a transition does not care which direction it was. The commands keep the two apart:
/// `project_export_status` refuses an import's identifier before it ever reaches this.
#[derive(Debug, Clone)]
pub enum TransferOutcome {
    /// An archive was written.
    Exported(Box<super::transfer::ExportReportView>),
    /// A project file was written.
    Imported(Box<super::transfer::ImportReportView>),
}

/// How far a session has got.
#[derive(Debug)]
enum State {
    /// The worker is running, or has not been spawned yet.
    Running,
    /// It finished. Terminal.
    Finished(Box<TransferOutcome>),
    /// It failed, for a reason already translated for the interface. Terminal.
    Failed(Box<ProjectError>),
    /// The user stopped it. Terminal.
    Cancelled,
    /// Forgotten. Terminal.
    Discarded,
}

/// A transfer session's lifecycle, with no transfer context attached.
///
/// Split out for the reason [`super::session::Lifecycle`] is: every hazard is about a
/// *transition*, and a transition needs only an outcome. Keeping the state machine free of
/// paths and reports means its tests are pure — no archive, no project file, no disk.
#[derive(Debug)]
pub struct TransferLifecycle {
    cancel: Arc<AtomicBool>,
    started: AtomicBool,
    work: Mutex<Work>,
    state: Mutex<State>,
}

impl Default for TransferLifecycle {
    fn default() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            started: AtomicBool::new(false),
            work: Mutex::new(Work::default()),
            state: Mutex::new(State::Running),
        }
    }
}

impl TransferLifecycle {
    /// A handle the protocol can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        CancelFlag::from_flag(Arc::clone(&self.cancel))
    }

    /// Claims the right to run this session's work, exactly once.
    ///
    /// Returns `false` if somebody already claimed it. A compare-and-swap rather than a
    /// check, because two callers that both read "not started" would both spawn a worker and
    /// both write the same archive — the same reason a save is claimed rather than tested.
    ///
    /// In the shipped application only `project_export_start` and `project_import_start` call
    /// this, each immediately after creating the session; the guard exists so that stays true
    /// by construction rather than by reading the two call sites.
    pub fn claim_start(&self) -> bool {
        self.started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Asks the run to stop.
    ///
    /// Returns whether the request was made *while something was still running*. `false` for
    /// a transfer that had already finished, which is not an error: there was nothing to
    /// stop, and reporting success would tell the user their cancel did something it did not.
    ///
    /// Sets **no state**. The worker is the only thing that leaves `Running`.
    pub fn request_cancel(&self) -> bool {
        let running = matches!(&*self.lock_state(), State::Running);
        self.cancel.store(true, Ordering::SeqCst);
        running
    }

    /// Whether a cancel has been requested.
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Records where the work has got to.
    pub fn report(&self, work: Work) {
        if let Ok(mut held) = self.work.lock() {
            *held = work;
        }
    }

    /// The latest progress reported.
    pub fn work(&self) -> Work {
        self.work.lock().map(|held| *held).unwrap_or_default()
    }

    /// Records what the worker produced. Only from `Running`.
    ///
    /// A session discarded while the worker ran stays discarded: the user said they did not
    /// want it, and a result arriving afterwards must not resurrect it.
    pub fn finish(&self, outcome: TransferOutcome) {
        let mut state = self.lock_state();
        if matches!(&*state, State::Running) {
            *state = State::Finished(Box::new(outcome));
        }
    }

    /// Records that the worker failed. Only from `Running`.
    pub fn fail(&self, error: ProjectError) {
        let mut state = self.lock_state();
        if matches!(&*state, State::Running) {
            *state = State::Failed(Box::new(error));
        }
    }

    /// Records that the run stopped because it was asked to. Only from `Running`.
    ///
    /// Distinct from [`Self::fail`] on purpose: a cancelled transfer wrote nothing and is not
    /// a failure the user needs to act on, so presenting it as one would make a working
    /// control look like a fault.
    pub fn cancelled(&self) {
        let mut state = self.lock_state();
        if matches!(&*state, State::Running) {
            *state = State::Cancelled;
        }
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> TransferPhase {
        match &*self.lock_state() {
            State::Running => TransferPhase::Running,
            State::Finished(_) => TransferPhase::Finished,
            State::Failed(_) => TransferPhase::Failed,
            State::Cancelled => TransferPhase::Cancelled,
            State::Discarded => TransferPhase::Discarded,
        }
    }

    /// The report, for a session that has one.
    pub fn outcome(&self) -> Option<TransferOutcome> {
        match &*self.lock_state() {
            State::Finished(outcome) => Some((**outcome).clone()),
            _ => None,
        }
    }

    /// Why it failed, for a session that did.
    pub fn failure(&self) -> Option<ProjectError> {
        match &*self.lock_state() {
            State::Failed(error) => Some((**error).clone()),
            _ => None,
        }
    }

    /// Abandons the session, asking the run to stop if it is still going.
    ///
    /// Returns whether this call was the one that abandoned it.
    pub fn discard(&self) -> bool {
        self.cancel.store(true, Ordering::SeqCst);
        let mut state = self.lock_state();
        if matches!(&*state, State::Discarded) {
            return false;
        }
        *state = State::Discarded;
        true
    }

    /// Whether this session has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        !matches!(&*self.lock_state(), State::Running)
    }

    /// Takes the state lock, recovering from poisoning.
    ///
    /// A poisoned lock means a previous command panicked while holding it. The state is still
    /// readable, and refusing every later call would strand a transfer the user can see on
    /// screen. The workspace lock is the one that treats poisoning as fatal, because that
    /// guards a database.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One transfer, from start to finished, failed, cancelled or discarded.
#[derive(Debug)]
pub struct TransferSession {
    /// This run's identity, carried on every event about it.
    pub run_id: Uuid,
    /// Which direction.
    pub kind: TransferKind,
    /// Which project was open when it began.
    pub epoch: u64,
    /// What the transfer reads: the archive, for an import. `None` for an export, whose
    /// source is whichever project was open — named by `epoch` rather than by a path.
    pub source: Option<PathBuf>,
    /// What the transfer writes: the `.evara` for an export, the `.evaraproj` for an import.
    pub destination: PathBuf,
    /// How many bytes the payload holds, known before the worker starts.
    pub total_bytes: u64,
    /// When the caller started it, ISO-8601 UTC.
    pub started_at: String,
    life: Arc<TransferLifecycle>,
}

impl TransferSession {
    /// Starts a session in `Running`.
    #[must_use]
    pub fn new(
        run_id: Uuid,
        kind: TransferKind,
        epoch: u64,
        source: Option<PathBuf>,
        destination: PathBuf,
        total_bytes: u64,
        started_at: String,
    ) -> Self {
        Self {
            run_id,
            kind,
            epoch,
            source,
            destination,
            total_bytes,
            started_at,
            life: Arc::new(TransferLifecycle::default()),
        }
    }

    /// The lifecycle, for the worker thread and the progress sink.
    pub fn lifecycle(&self) -> Arc<TransferLifecycle> {
        Arc::clone(&self.life)
    }

    /// A handle the protocol can ask whether to stop.
    pub fn cancellation(&self) -> CancelFlag {
        self.life.cancellation()
    }

    /// Claims the right to run this session's work, exactly once.
    pub fn claim_start(&self) -> bool {
        self.life.claim_start()
    }

    /// Asks the run to stop.
    pub fn request_cancel(&self) -> bool {
        self.life.request_cancel()
    }

    /// Whether a cancel has been requested.
    pub fn cancel_requested(&self) -> bool {
        self.life.cancel_requested()
    }

    /// Where the work has got to.
    pub fn work(&self) -> Work {
        self.life.work()
    }

    /// Which phase the interface should render.
    pub fn phase(&self) -> TransferPhase {
        self.life.phase()
    }

    /// The report, for a session that has one.
    pub fn outcome(&self) -> Option<TransferOutcome> {
        self.life.outcome()
    }

    /// Why it failed, for a session that did.
    pub fn failure(&self) -> Option<ProjectError> {
        self.life.failure()
    }

    /// Abandons the session.
    pub fn discard(&self) -> bool {
        self.life.discard()
    }

    /// Whether this session has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.life.is_terminal()
    }
}

/// Every transfer this process has started, by run.
///
/// A `BTreeMap` keyed by a UUIDv7, so iteration is in start order and the oldest session is
/// the first key — which is what makes [`Self::MAX_RETAINED`]'s eviction a first-entry scan
/// rather than a sort by a timestamp nobody would keep in step.
#[derive(Debug, Default)]
pub struct TransferSessions {
    sessions: Mutex<std::collections::BTreeMap<Uuid, Arc<TransferSession>>>,
}

impl TransferSessions {
    /// How many sessions this process keeps before it starts forgetting finished ones.
    ///
    /// Fixed rather than configurable, for the reason `MAX_RECENT_PROJECTS` is: a session
    /// history is a convenience, and an unbounded one becomes a list of every transfer the
    /// user has ever run. A **running** session is never evicted, so the cap can never
    /// silence something still in flight.
    pub const MAX_RETAINED: usize = 16;

    /// Records a new session.
    ///
    /// Returns `false` if this run identifier is already held, which is the duplicate the
    /// caller must not paper over: two sessions with one identity would make every later
    /// status, cancel and discard ambiguous.
    pub fn insert(&self, session: Arc<TransferSession>) -> bool {
        let Ok(mut held) = self.sessions.lock() else {
            return false;
        };
        if held.contains_key(&session.run_id) {
            return false;
        }
        held.insert(session.run_id, session);
        Self::evict_oldest_terminal(&mut held);
        true
    }

    /// One session by run, whatever kind it is.
    pub fn get(&self, run_id: Uuid) -> Option<Arc<TransferSession>> {
        self.sessions
            .lock()
            .ok()
            .and_then(|held| held.get(&run_id).map(Arc::clone))
    }

    /// One session by run, if it is of `kind`.
    ///
    /// `None` for an identifier belonging to the other direction. That is the refusal the
    /// "only authorised operations can be cancelled" rule needs: an export command handed an
    /// import's run must not reach the import's cancel flag.
    pub fn get_of_kind(&self, run_id: Uuid, kind: TransferKind) -> Option<Arc<TransferSession>> {
        self.get(run_id).filter(|session| session.kind == kind)
    }

    /// Abandons one session and forgets it. Returns whether there was one of that kind.
    ///
    /// The session is discarded **before** being removed, so a worker still running sees the
    /// cancel flag even though nothing holds the map entry any more — the thread holds its
    /// own `Arc`.
    pub fn discard(&self, run_id: Uuid, kind: TransferKind) -> bool {
        let Ok(mut held) = self.sessions.lock() else {
            return false;
        };
        if held.get(&run_id).is_none_or(|session| session.kind != kind) {
            return false;
        }
        match held.remove(&run_id) {
            Some(session) => {
                session.discard();
                true
            }
            None => false,
        }
    }

    /// Whether any session is still writing to `destination`.
    ///
    /// The second guard on a destination, and the cheap one. `evara-project` already refuses a
    /// second replacement of one path through the journal's uniqueness index, and `seal`'s
    /// `.part` claim is atomic — but both of those report a failure *after* work has begun,
    /// and a user who clicked twice is better told before anything is snapshotted.
    pub fn is_writing_to(&self, destination: &Path) -> bool {
        self.sessions.lock().is_ok_and(|held| {
            held.values().any(|session| {
                !session.is_terminal() && session.destination.as_path() == destination
            })
        })
    }

    /// How many transfers are still running.
    pub fn running(&self) -> usize {
        self.sessions.lock().map_or(0, |held| {
            held.values()
                .filter(|session| !session.is_terminal())
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

    /// Asks every running transfer to stop, without forgetting any of them.
    ///
    /// Called when the application is shutting down. A request rather than a wait: the worker
    /// is the only thing that leaves `Running`, and an import interrupted anyway is made safe
    /// by the replacement journal and by `recover_pending` at the next start — which is D11's
    /// mechanism, not something this method could improve on.
    pub fn request_cancel_all(&self) -> usize {
        self.sessions.lock().map_or(0, |held| {
            held.values()
                .filter(|session| session.request_cancel())
                .count()
        })
    }

    /// Abandons every session and forgets them all.
    ///
    /// Called when the project is created, opened or closed — D9, and exactly what `imports`,
    /// `generations`, `edits` and `repairs` do. A session records the epoch it began under,
    /// so one belonging to a project that is no longer open can neither be read nor cancelled
    /// honestly; discarding it also stops an export whose snapshot describes a campus the
    /// user has moved away from.
    ///
    /// Takes no `Result`, for the reason the other registries' `clear` does not: a poisoned
    /// lock means a previous command panicked, and refusing to close the project because of
    /// it would trap the user in the state they are trying to leave.
    pub fn clear(&self) {
        if let Ok(mut held) = self.sessions.lock() {
            for session in held.values() {
                session.discard();
            }
            held.clear();
        }
    }

    /// Drops the oldest terminal sessions until the map is within [`Self::MAX_RETAINED`].
    ///
    /// Running sessions are skipped, so a user who starts seventeen exports at once keeps all
    /// seventeen; the cap bounds *history*, not concurrency.
    fn evict_oldest_terminal(held: &mut std::collections::BTreeMap<Uuid, Arc<TransferSession>>) {
        while held.len() > Self::MAX_RETAINED {
            let Some(oldest) = held
                .iter()
                .find(|(_, session)| session.is_terminal())
                .map(|(run_id, _)| *run_id)
            else {
                // Every session is still running. Nothing may be evicted, and the cap waits.
                return;
            };
            held.remove(&oldest);
        }
    }
}

/// A [`evara_project::TransferProgress`] that records into a session and coalesces its events.
///
/// # Why it coalesces, and what it never coalesces away
///
/// `stream_payload` reports after every buffer, which for a 200 MB payload is thousands of
/// calls. Emitting thousands of Tauri events is affordable and wasteful, and it floods an
/// `aria-live` region a screen reader is reading. So an event is emitted at most once per
/// [`Coalescer::INTERVAL_MS`] — **except** that a change of [`Phase`] is always emitted, even
/// if one went by a millisecond ago.
///
/// That exception is the point. The phases take wildly different times, so a bar driven by
/// `done / total` alone tells a user very little; the label is what carries the information,
/// and dropping a transition to save an event would be dropping the only part worth sending.
/// The interval arithmetic itself is [`Coalescer`]'s, shared with the solver's sink.
pub struct CoalescingTransferSink<F> {
    life: Arc<TransferLifecycle>,
    emit: F,
    rate: Coalescer,
    last_phase: Mutex<Option<Phase>>,
}

impl<F: Fn(Work)> CoalescingTransferSink<F> {
    /// Wraps a lifecycle and an emitter.
    pub fn new(life: Arc<TransferLifecycle>, emit: F) -> Self {
        Self {
            life,
            emit,
            rate: Coalescer::new(),
            last_phase: Mutex::new(None),
        }
    }

    /// Whether this call is a phase transition, recording the new phase when it is.
    fn is_transition(&self, phase: Phase) -> bool {
        match self.last_phase.lock() {
            Ok(mut last) => {
                let changed = *last != Some(phase);
                if changed {
                    *last = Some(phase);
                }
                changed
            }
            // A poisoned lock answers "yes": an extra event is harmless, and a missed
            // transition is the one thing this sink must not lose.
            Err(_) => true,
        }
    }
}

impl<F: Fn(Work) + Send + Sync> evara_project::TransferProgress for CoalescingTransferSink<F> {
    fn progress(&self, phase: Phase, done: u64, total: u64) {
        let work = Work {
            step: phase,
            done,
            total,
        };
        // Recorded on every call, so a status poll is current even between events.
        self.life.report(work);

        if self.is_transition(phase) || self.rate.due() {
            (self.emit)(work);
        }
    }
}

#[cfg(test)]
mod tests;
