//! The transfer state machine, tested against the four hazards its header names.
//!
//! These run on a bare [`TransferLifecycle`] and a hand-built registry: no archive, no campus
//! file, no disk. That is the point of splitting the state machine out of [`TransferSession`]
//! — a transition needs only an outcome, so every hazard gets a test that runs in
//! microseconds and that fails for exactly one reason.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use evara_core::scheduling::Cancel;
use evara_project::{Phase, TransferProgress};
use uuid::Uuid;

use super::*;
use crate::commands::transfer::ExportReportView;

/// A plausible export report, for a test that only cares that *a* report arrived.
fn a_report() -> TransferOutcome {
    TransferOutcome::Exported(Box::new(ExportReportView {
        path: String::from("/tmp/Northgate.evara"),
        file_name: String::from("Northgate.evara"),
        archive_bytes: 1_024,
        payload_bytes: 4_096,
        payload_sha256: String::from("abc"),
        compression_saving_percent: 75.0,
        project_id: Uuid::from_bytes([1; 16]),
        schema_version: 18,
        schools: Vec::new(),
        capabilities: Vec::new(),
        change_log_entries: 0,
        change_log_watermark: 0,
        exported_at: String::from("2026-11-13T00:00:00.000Z"),
    }))
}

/// A session of `kind`, writing to `destination`.
fn a_session(kind: TransferKind, destination: &str) -> Arc<TransferSession> {
    Arc::new(TransferSession::new(
        Uuid::now_v7(),
        kind,
        0,
        None,
        PathBuf::from(destination),
        4_096,
        String::from("2026-11-13T00:00:00.000Z"),
    ))
}

// =======================================================================================
// Hazard 1 — reading the result of an unfinished run
// =======================================================================================

#[test]
fn a_new_session_is_running_and_has_no_report() {
    let life = TransferLifecycle::default();
    assert_eq!(life.phase(), TransferPhase::Running);
    assert!(life.outcome().is_none(), "nothing has been produced yet");
    assert!(life.failure().is_none());
    assert!(!life.is_terminal());
}

#[test]
fn a_report_is_available_only_once_the_worker_has_finished() {
    let life = TransferLifecycle::default();
    life.finish(a_report());

    assert_eq!(life.phase(), TransferPhase::Finished);
    assert!(life.outcome().is_some());
    assert!(life.is_terminal());
}

// =======================================================================================
// Hazard 2 — two workers for one session
// =======================================================================================

#[test]
fn the_right_to_run_is_claimed_exactly_once() {
    let life = TransferLifecycle::default();
    assert!(life.claim_start(), "the first caller claims it");
    assert!(
        !life.claim_start(),
        "a second caller must not get a second worker writing the same file"
    );
}

#[test]
fn two_sessions_cannot_share_one_identifier() {
    let sessions = TransferSessions::default();
    let first = a_session(TransferKind::Export, "/tmp/a.evara");
    // The same identifier, deliberately: two entries under one key would make every later
    // status, cancel and discard ambiguous.
    let second = Arc::new(TransferSession::new(
        first.run_id,
        TransferKind::Export,
        0,
        None,
        PathBuf::from("/tmp/b.evara"),
        0,
        String::from("2026-11-13T00:00:00.000Z"),
    ));

    assert!(sessions.insert(Arc::clone(&first)));
    assert!(!sessions.insert(second), "the duplicate must be refused");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions
            .get(first.run_id)
            .expect("the first session is still there")
            .destination,
        PathBuf::from("/tmp/a.evara"),
        "the first session must not have been replaced"
    );
}

#[test]
fn a_destination_already_being_written_is_visible_before_any_work_starts() {
    let sessions = TransferSessions::default();
    let session = a_session(TransferKind::Export, "/tmp/Northgate.evara");
    sessions.insert(Arc::clone(&session));

    assert!(sessions.is_writing_to(Path::new("/tmp/Northgate.evara")));
    assert!(!sessions.is_writing_to(Path::new("/tmp/Southmoor.evara")));

    // A session that has finished is no longer writing anywhere, so a second export to the
    // same name is a question about the file rather than about the session.
    session.lifecycle().finish(a_report());
    assert!(!sessions.is_writing_to(Path::new("/tmp/Northgate.evara")));
}

// =======================================================================================
// Hazard 3 — a cancel racing a finish
// =======================================================================================

#[test]
fn a_cancel_sets_a_flag_and_no_state() {
    let life = TransferLifecycle::default();
    assert!(life.request_cancel(), "something was running");

    assert!(life.cancel_requested());
    assert_eq!(
        life.phase(),
        TransferPhase::Running,
        "the worker is the only thing that leaves Running"
    );
}

#[test]
fn a_cancel_arriving_after_the_work_finished_changes_nothing_and_says_so() {
    let life = TransferLifecycle::default();
    life.finish(a_report());

    assert!(
        !life.request_cancel(),
        "there was nothing left to stop, and saying otherwise would tell the user their \
         cancel did something it did not"
    );
    assert_eq!(life.phase(), TransferPhase::Finished);
    assert!(
        life.outcome().is_some(),
        "the report must survive a late cancel"
    );
}

#[test]
fn a_worker_that_wins_the_race_still_reports_what_it_found() {
    // The cancel flag is set, and the worker commits its last step anyway. The session must
    // report the finish, because that is what actually happened on disk.
    let life = TransferLifecycle::default();
    life.request_cancel();
    life.finish(a_report());

    assert_eq!(life.phase(), TransferPhase::Finished);
    assert!(
        life.cancel_requested(),
        "the request itself is still visible"
    );
}

#[test]
fn a_cancelled_run_is_not_a_failure() {
    let life = TransferLifecycle::default();
    life.request_cancel();
    life.cancelled();

    assert_eq!(life.phase(), TransferPhase::Cancelled);
    assert!(
        life.failure().is_none(),
        "the user pressed Cancel; presenting a working control as a fault is how a safe \
         operation acquires a reputation for being unreliable"
    );
}

// =======================================================================================
// Terminality
// =======================================================================================

#[test]
fn nothing_leaves_a_terminal_state() {
    for reach in [
        &(|life: &TransferLifecycle| life.finish(a_report())) as &dyn Fn(&TransferLifecycle),
        &|life| {
            life.fail(ProjectError::Failed {
                message: String::new(),
            });
        },
        &|life| life.cancelled(),
    ] {
        let life = TransferLifecycle::default();
        reach(&life);
        let settled = life.phase();

        // Every other transition, applied afterwards.
        life.finish(a_report());
        life.fail(ProjectError::Failed {
            message: String::from("later"),
        });
        life.cancelled();

        assert_eq!(
            life.phase(),
            settled,
            "a terminal state must not be reachable out of"
        );
    }
}

#[test]
fn a_session_discarded_mid_run_stays_discarded_when_the_result_arrives() {
    let life = TransferLifecycle::default();
    assert!(life.discard());
    assert!(
        life.cancel_requested(),
        "discarding must also stop the worker rather than leaving it writing"
    );

    life.finish(a_report());
    assert_eq!(
        life.phase(),
        TransferPhase::Discarded,
        "the user said they did not want it; a result arriving afterwards must not \
         resurrect it"
    );
    assert!(life.outcome().is_none());
}

#[test]
fn discarding_twice_reports_the_second_as_a_no_op() {
    let life = TransferLifecycle::default();
    assert!(life.discard());
    assert!(!life.discard());
}

// =======================================================================================
// Hazard 4 — the wrong kind, and the registry's own rules
// =======================================================================================

#[test]
fn a_session_is_reachable_only_as_the_kind_it_is() {
    let sessions = TransferSessions::default();
    let export = a_session(TransferKind::Export, "/tmp/a.evara");
    sessions.insert(Arc::clone(&export));

    assert!(
        sessions
            .get_of_kind(export.run_id, TransferKind::Export)
            .is_some()
    );
    assert!(
        sessions
            .get_of_kind(export.run_id, TransferKind::Import)
            .is_none(),
        "an import command handed an export's identifier must not reach its cancel flag"
    );
}

#[test]
fn discarding_with_the_wrong_kind_changes_nothing() {
    let sessions = TransferSessions::default();
    let import = a_session(TransferKind::Import, "/tmp/a.evaraproj");
    sessions.insert(Arc::clone(&import));

    assert!(!sessions.discard(import.run_id, TransferKind::Export));
    assert_eq!(sessions.len(), 1, "the session must still be held");
    assert!(
        !import.cancel_requested(),
        "and it must not have been stopped by a command that could not name it"
    );

    assert!(sessions.discard(import.run_id, TransferKind::Import));
    assert_eq!(sessions.len(), 0);
    assert!(import.cancel_requested());
}

#[test]
fn clearing_stops_every_session_and_forgets_them_all() {
    let sessions = TransferSessions::default();
    let export = a_session(TransferKind::Export, "/tmp/a.evara");
    let import = a_session(TransferKind::Import, "/tmp/b.evaraproj");
    sessions.insert(Arc::clone(&export));
    sessions.insert(Arc::clone(&import));
    assert_eq!(sessions.running(), 2);

    sessions.clear();

    assert!(sessions.is_empty());
    for session in [&export, &import] {
        assert!(
            session.cancel_requested(),
            "a worker still running must see the flag even though nothing holds the entry"
        );
        assert_eq!(session.phase(), TransferPhase::Discarded);
    }
}

#[test]
fn shutdown_asks_every_running_transfer_to_stop_without_forgetting_any() {
    let sessions = TransferSessions::default();
    let running = a_session(TransferKind::Export, "/tmp/a.evara");
    let done = a_session(TransferKind::Import, "/tmp/b.evaraproj");
    done.lifecycle().cancelled();
    sessions.insert(Arc::clone(&running));
    sessions.insert(Arc::clone(&done));

    assert_eq!(
        sessions.request_cancel_all(),
        1,
        "only the one that was still going counts as stopped"
    );
    assert!(running.cancel_requested());
    assert_eq!(
        sessions.len(),
        2,
        "shutdown asks; it does not discard, because the reports are still readable"
    );
}

#[test]
fn history_is_bounded_and_a_running_session_is_never_evicted() {
    let sessions = TransferSessions::default();

    // One running session, inserted first so it is the oldest key and would be the first
    // candidate for eviction if the cap did not skip it.
    let running = a_session(TransferKind::Export, "/tmp/live.evara");
    sessions.insert(Arc::clone(&running));

    let mut finished = Vec::new();
    for index in 0..TransferSessions::MAX_RETAINED + 5 {
        let session = a_session(TransferKind::Export, &format!("/tmp/{index}.evara"));
        session.lifecycle().finish(a_report());
        sessions.insert(Arc::clone(&session));
        finished.push(session);
    }

    assert!(
        sessions.len() <= TransferSessions::MAX_RETAINED,
        "an unbounded session history is a leak: {} held",
        sessions.len()
    );
    assert!(
        sessions.get(running.run_id).is_some(),
        "the running session must survive every eviction"
    );
    // The newest finished sessions are the ones kept.
    let newest = finished.last().expect("at least one");
    assert!(sessions.get(newest.run_id).is_some());
}

#[test]
fn a_registry_of_running_sessions_grows_past_the_cap_rather_than_silencing_one() {
    // The cap bounds *history*, not concurrency. Seventeen simultaneous exports is absurd
    // and is still not a reason to stop tracking one of them.
    let sessions = TransferSessions::default();
    for index in 0..=TransferSessions::MAX_RETAINED {
        sessions.insert(a_session(
            TransferKind::Export,
            &format!("/tmp/{index}.evara"),
        ));
    }
    assert_eq!(sessions.len(), TransferSessions::MAX_RETAINED + 1);
    assert_eq!(sessions.running(), TransferSessions::MAX_RETAINED + 1);
}

// =======================================================================================
// Cancellation reaches the protocol
// =======================================================================================

#[test]
fn the_cancellation_handle_reads_the_sessions_own_flag() {
    let session = a_session(TransferKind::Import, "/tmp/a.evaraproj");
    let cancel = session.cancellation();

    assert!(!cancel.is_cancelled());
    session.request_cancel();
    assert!(
        cancel.is_cancelled(),
        "the handle the protocol holds must see the request, or a cancel would do nothing"
    );
}

// =======================================================================================
// Coalescing, and the transition it never drops
// =======================================================================================

/// A sink that counts what it emitted and in what order.
struct Recorder {
    emitted: std::sync::Mutex<Vec<(Phase, u64)>>,
    calls: AtomicUsize,
}

impl Recorder {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            emitted: std::sync::Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn phases(&self) -> Vec<Phase> {
        self.emitted
            .lock()
            .expect("recorder")
            .iter()
            .map(|(phase, _)| *phase)
            .collect()
    }
}

#[test]
fn a_flood_within_one_phase_is_coalesced() {
    let life = Arc::new(TransferLifecycle::default());
    let recorder = Recorder::new();
    let seen = Arc::clone(&recorder);
    let sink = CoalescingTransferSink::new(Arc::clone(&life), move |work: Work| {
        seen.calls.fetch_add(1, Ordering::SeqCst);
        seen.emitted
            .lock()
            .expect("recorder")
            .push((work.step, work.done));
    });

    for done in 0..2_000_u64 {
        sink.progress(Phase::Staging, done, 2_000);
    }

    let emitted = recorder.calls.load(Ordering::SeqCst);
    assert!(
        emitted < 50,
        "two thousand buffer reports must not become two thousand events; got {emitted}"
    );
    assert!(emitted >= 1, "the first report is always due");
    assert_eq!(
        life.work().done,
        1_999,
        "every call is recorded, so a status poll is current even between events"
    );
}

#[test]
fn every_phase_transition_is_emitted_however_fast_they_arrive() {
    // The exception that is the point of the sink: the phases take wildly different times, so
    // the label is what carries the information and dropping a transition to save an event
    // would be dropping the only part worth sending.
    let life = Arc::new(TransferLifecycle::default());
    let recorder = Recorder::new();
    let seen = Arc::clone(&recorder);
    let sink = CoalescingTransferSink::new(life, move |work: Work| {
        seen.emitted
            .lock()
            .expect("recorder")
            .push((work.step, work.done));
    });

    let order = [
        Phase::Staging,
        Phase::Verifying,
        Phase::Migrating,
        Phase::Finalising,
    ];
    for phase in order {
        // Several reports per phase, all within one coalescing interval.
        for done in 0..10_u64 {
            sink.progress(phase, done, 10);
        }
    }

    let phases = recorder.phases();
    for phase in order {
        assert!(
            phases.contains(&phase),
            "{phase:?} was coalesced away; got {phases:?}"
        );
    }
    // And they arrived in the order the protocol ran them.
    let first_seen: Vec<Phase> = {
        let mut seen = Vec::new();
        for phase in phases {
            if seen.last() != Some(&phase) {
                seen.push(phase);
            }
        }
        seen
    };
    assert_eq!(
        first_seen,
        order.to_vec(),
        "progress must not be reordered or interleaved"
    );
}

#[test]
fn the_interval_is_the_one_the_solver_sink_uses() {
    // One implementation, two callers. A second copy of the arithmetic is the duplication
    // the proposal asks this sink to avoid.
    assert_eq!(
        Coalescer::INTERVAL_MS,
        crate::commands::session::CoalescingSink::<fn(crate::commands::session::Progress)>::INTERVAL_MS
    );
}

#[test]
fn a_coalescer_lets_the_first_call_through_and_then_holds() {
    let rate = Coalescer::new();
    assert!(rate.due(), "nothing has been emitted, so the first is due");
    assert!(!rate.due(), "and the next is not, a microsecond later");
}
