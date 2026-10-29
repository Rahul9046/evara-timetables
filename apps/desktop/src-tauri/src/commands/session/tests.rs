//! The session state machine, tested against the five hazards its header names.
//!
//! These run on a bare [`Lifecycle`] and a hand-built [`SolveOutcome`]: no project file, no
//! migration, no solve. That is the point of splitting the state machine out of [`Session`] —
//! a transition needs only an outcome, so every hazard gets a test that runs in microseconds
//! and that fails for exactly one reason.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use evara_core::scheduling::{
    Assignment, DiagnosticReport, ProblemIdentity, Score, Seed, SolveOutcome, SolveStatistics,
    SolveStatus, Termination, UnverifiedCounts,
};
use evara_solver::{Cancel, ProgressSink};
use uuid::Uuid;

use super::*;

/// An outcome with the status and termination a test is about, and plausible everything else.
fn an_outcome(status: SolveStatus, termination: Termination) -> SolveOutcome {
    SolveOutcome {
        problem: ProblemIdentity {
            school: Uuid::from_bytes([1; 16]),
            cycle: Uuid::from_bytes([2; 16]),
            term: None,
        },
        seed: Seed::new(2026),
        status,
        termination,
        assignment: Assignment::empty(4),
        unscheduled: Vec::new(),
        score: Score::PERFECT,
        hard_violations: Vec::new(),
        unverified: UnverifiedCounts::default(),
        preflight: DiagnosticReport::default(),
        statistics: SolveStatistics::default(),
    }
}

/// A complete, saveable outcome.
fn a_good_outcome() -> SolveOutcome {
    an_outcome(SolveStatus::Complete, Termination::Exhausted)
}

/// A lifecycle that has finished with a saveable result.
fn finished() -> Lifecycle {
    let life = Lifecycle::default();
    life.finish(a_good_outcome());
    life
}

// =======================================================================================
// Hazard 1 — saving an unfinished run
// =======================================================================================

#[test]
fn a_new_session_is_running_and_refuses_a_save() {
    let life = Lifecycle::default();
    assert_eq!(life.phase(), SessionPhase::Running);
    assert_eq!(life.claim_save(), Err(SaveRefusal::StillRunning));
    assert_eq!(
        life.saveable(),
        Some("the run has not finished"),
        "and it says why, so the button can explain itself"
    );
}

#[test]
fn a_running_session_has_no_outcome_to_show() {
    let life = Lifecycle::default();
    assert!(life.outcome().is_none());
    assert!(life.saved_as().is_none());
}

// =======================================================================================
// Hazard 2 — saving a refused or broken run
// =======================================================================================

#[test]
fn an_input_rejected_run_is_not_saveable_and_names_the_findings() {
    let life = Lifecycle::default();
    life.finish(an_outcome(SolveStatus::Failed, Termination::InputRejected));
    let refusal = life.claim_save().expect_err("nothing was attempted");
    let SaveRefusal::NotSaveable { reason } = refusal else {
        panic!("expected NotSaveable, got {refusal:?}");
    };
    assert!(
        reason.contains("not fit to solve"),
        "the remedy is the validation report, and the wording says so: {reason}"
    );
    assert_eq!(
        life.phase(),
        SessionPhase::Finished,
        "a refused save leaves the session where it was"
    );
}

#[test]
fn an_internal_check_failure_is_reported_as_a_defect_in_evara() {
    let life = Lifecycle::default();
    life.finish(an_outcome(
        SolveStatus::Failed,
        Termination::InternalCheckFailed,
    ));
    let refusal = life
        .claim_save()
        .expect_err("the reference scorer rejected it");
    let SaveRefusal::NotSaveable { reason } = refusal else {
        panic!("expected NotSaveable, got {refusal:?}");
    };
    assert!(
        reason.contains("defect in Evara"),
        "TIMETABLE-PERSISTENCE §9 and generate() both call this a defect, never a timetable: \
         {reason}"
    );
}

#[test]
fn a_partial_timetable_is_saveable_because_partial_is_an_ordinary_state() {
    let life = Lifecycle::default();
    life.finish(an_outcome(SolveStatus::Partial, Termination::Exhausted));
    assert!(
        life.saveable().is_none(),
        "a partial timetable is usable, with lessons unplaced — not a failure"
    );
    assert!(life.claim_save().is_ok());
}

#[test]
fn a_cancelled_run_is_saveable_because_what_it_placed_is_real() {
    let life = Lifecycle::default();
    life.finish(an_outcome(SolveStatus::Cancelled, Termination::Cancelled));
    assert!(
        life.saveable().is_none(),
        "a cancelled run produced a partial arrangement, and the user may want to keep it"
    );
}

#[test]
fn an_infeasible_run_has_no_timetable_to_save() {
    let life = Lifecycle::default();
    life.finish(an_outcome(SolveStatus::Infeasible, Termination::Exhausted));
    let refusal = life.claim_save().expect_err("there is no timetable");
    assert!(matches!(refusal, SaveRefusal::NotSaveable { .. }));
}

// =======================================================================================
// Hazard 3 — two saves from two clicks
// =======================================================================================

#[test]
fn a_second_claim_while_the_first_is_in_flight_is_refused() {
    let life = finished();
    assert!(life.claim_save().is_ok(), "the first claim wins");
    assert_eq!(
        life.phase(),
        SessionPhase::Saving,
        "the claim moves the session, before any database work"
    );
    assert_eq!(
        life.claim_save(),
        Err(SaveRefusal::AlreadySaving),
        "the second click finds the save already claimed"
    );
}

#[test]
fn a_claim_after_a_successful_save_names_the_timetable_already_written() {
    let life = finished();
    let id = Uuid::from_bytes([9; 16]);
    life.claim_save().expect("claim");
    life.save_succeeded(id);
    assert_eq!(life.phase(), SessionPhase::Saved);
    assert_eq!(life.saved_as(), Some(id));
    assert_eq!(
        life.claim_save(),
        Err(SaveRefusal::AlreadySaved { timetable_id: id }),
        "so the interface can offer to open it rather than reporting an error"
    );
}

#[test]
fn nothing_leaves_the_saved_state() {
    let life = finished();
    let id = Uuid::from_bytes([9; 16]);
    life.claim_save().expect("claim");
    life.save_succeeded(id);

    // Every transition, tried against a saved session.
    life.finish(an_outcome(SolveStatus::Partial, Termination::Exhausted));
    life.save_failed();
    life.save_succeeded(Uuid::from_bytes([7; 16]));
    assert_eq!(life.phase(), SessionPhase::Saved, "saved is terminal");
    assert_eq!(
        life.saved_as(),
        Some(id),
        "and it is still the timetable that was actually written"
    );
}

#[test]
fn a_failed_write_returns_the_session_so_the_save_can_be_retried() {
    // A write refused for a reason the user can act on — a duplicate name, a read-only
    // project — must not strand a result they can see on screen.
    let life = finished();
    life.claim_save().expect("claim");
    life.save_failed();
    assert_eq!(life.phase(), SessionPhase::Finished);
    assert!(
        life.claim_save().is_ok(),
        "and the retry is allowed to claim again"
    );
}

#[test]
fn the_outcome_survives_a_save_so_the_preview_keeps_rendering() {
    let life = finished();
    life.claim_save().expect("claim");
    assert!(life.outcome().is_some(), "while saving");
    life.save_succeeded(Uuid::from_bytes([9; 16]));
    assert!(
        life.outcome().is_some(),
        "and after, rather than the screen emptying at the moment of success"
    );
}

#[test]
fn concurrent_claims_from_many_threads_yield_exactly_one_winner() {
    // The hazard stated as the race it actually is: several IPC calls in flight at once.
    let life = Arc::new(finished());
    let winners = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..16 {
        let life = Arc::clone(&life);
        let winners = Arc::clone(&winners);
        handles.push(std::thread::spawn(move || {
            if life.claim_save().is_ok() {
                winners.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("no thread panicked");
    }
    assert_eq!(
        winners.load(Ordering::SeqCst),
        1,
        "exactly one claim may win, whatever the interleaving"
    );
    assert_eq!(life.phase(), SessionPhase::Saving);
}

// =======================================================================================
// Hazard 4 — a cancel racing a finish
// =======================================================================================

#[test]
fn a_cancel_while_running_is_reported_as_having_had_something_to_stop() {
    let life = Lifecycle::default();
    assert!(life.request_cancel(), "something was running");
    assert!(life.cancel_requested(), "and the engine will see the flag");
    assert_eq!(
        life.phase(),
        SessionPhase::Running,
        "a cancel is a request, not a transition: only the worker leaves Running"
    );
}

#[test]
fn a_cancel_after_the_run_finished_changes_nothing_and_says_so() {
    let life = finished();
    assert!(
        !life.request_cancel(),
        "there was nothing left to stop, and reporting success would be a lie"
    );
    assert_eq!(life.phase(), SessionPhase::Finished);
    assert!(
        life.saveable().is_none(),
        "and the result is still saveable — the cancel lost the race, which is deterministic"
    );
}

#[test]
fn a_worker_finishing_after_a_cancel_still_records_what_it_found() {
    // The real interleaving: the user cancels, the engine notices between steps, and the run
    // returns `Cancelled` with whatever it had placed.
    let life = Lifecycle::default();
    life.request_cancel();
    life.finish(an_outcome(SolveStatus::Cancelled, Termination::Cancelled));
    assert_eq!(life.phase(), SessionPhase::Finished);
    let outcome = life.outcome().expect("the partial arrangement is kept");
    assert_eq!(outcome.termination, Termination::Cancelled);
}

#[test]
fn the_cancel_flag_is_what_the_engine_reads() {
    let life = Lifecycle::default();
    let flag = life.cancellation();
    assert!(!flag.is_cancelled());
    life.request_cancel();
    assert!(
        flag.is_cancelled(),
        "the handle the engine holds sees the request"
    );
}

// =======================================================================================
// Discard, including while a worker is running
// =======================================================================================

#[test]
fn discarding_a_running_session_cancels_it_and_is_terminal() {
    let life = Lifecycle::default();
    assert!(life.discard());
    assert_eq!(life.phase(), SessionPhase::Discarded);
    assert!(
        life.cancel_requested(),
        "discarding stops the thread rather than leaving it working on an answer \
         nobody will read"
    );
    assert!(!life.discard(), "discarding twice changes nothing");
}

#[test]
fn a_result_arriving_after_a_discard_does_not_resurrect_the_session() {
    let life = Lifecycle::default();
    life.discard();
    life.finish(a_good_outcome());
    assert_eq!(
        life.phase(),
        SessionPhase::Discarded,
        "the user said they did not want it"
    );
    assert!(life.outcome().is_none());
    assert_eq!(life.claim_save(), Err(SaveRefusal::Discarded));
}

#[test]
fn a_discard_after_a_save_still_marks_the_session_discarded_without_unsaving_it() {
    // Closing the project after a save: the session is abandoned, and the timetable it wrote
    // is already on disk and unaffected.
    let life = finished();
    let id = Uuid::from_bytes([9; 16]);
    life.claim_save().expect("claim");
    life.save_succeeded(id);
    assert!(life.discard());
    assert_eq!(life.phase(), SessionPhase::Discarded);
}

// =======================================================================================
// Hazard 5 — a save into a different project
// =======================================================================================

#[test]
fn the_project_epoch_moves_on_every_lifecycle_event() {
    let epoch = ProjectEpoch::default();
    let before = epoch.get();
    epoch.bump();
    assert_ne!(epoch.get(), before, "a close invalidates every session");
    let between = epoch.get();
    epoch.bump();
    assert_ne!(epoch.get(), between, "and so does the next open");
    assert!(
        epoch.get() > before,
        "monotonic, so a stale epoch can never compare equal to a later one"
    );
}

// =======================================================================================
// Progress
// =======================================================================================

#[test]
fn progress_is_recorded_on_every_call_even_between_emitted_events() {
    // The coalescing is about the *channel*, not about what `generate_status` can answer: a
    // poll between two events must still see the latest figures.
    let life = Lifecycle::default();
    life.report(Progress {
        placed: 3,
        total: 10,
        steps: 7,
    });
    let seen = life.progress();
    assert_eq!(seen.placed, 3);
    assert_eq!(seen.total, 10);
    assert_eq!(seen.steps, 7);
}

#[test]
fn the_sink_emits_the_first_event_and_coalesces_the_flood_behind_it() {
    let life = Arc::new(Lifecycle::default());
    let emitted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&emitted);
    let sink = CoalescingSink::new(
        Arc::clone(&life) as Arc<dyn super::ProgressReporter>,
        move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        },
    );

    // 2 000 calls in a tight loop: the realistic upper end's flood.
    for n in 0..2_000 {
        sink.placed(n, 2_000, n as u64);
    }
    let count = emitted.load(Ordering::SeqCst);
    assert!(count >= 1, "the first placement is always announced");
    assert!(
        count < 2_000,
        "2 000 placements must not become 2 000 events: got {count}"
    );
    assert_eq!(
        life.progress().placed,
        1_999,
        "and the lifecycle still holds the latest figure"
    );
}
