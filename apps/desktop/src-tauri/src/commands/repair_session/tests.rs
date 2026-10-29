//! The six hazards, each as a test that runs in microseconds.
//!
//! Pure: no project file, no migration, no solve. A transition needs only an outcome, which is
//! why [`RepairLifecycle`] is split out of [`super::RepairSession`] — the same reason the
//! generation session is split.

use evara_core::scheduling::{
    Assignment, DiagnosticReport, ProblemIdentity, Score, Seed, SolveOutcome, SolveStatistics,
    SolveStatus, Termination, UnverifiedCounts,
};
use evara_solver::{RepairDefect, RepairOutcome, ViolationPartition};
use uuid::Uuid;

use super::{ApplyRefusal, RepairLifecycle, RepairPhase};

/// An outcome with the given termination, and nothing else interesting.
fn outcome_with(termination: Termination, status: SolveStatus, applyable: bool) -> RepairOutcome {
    RepairOutcome {
        outcome: SolveOutcome {
            problem: ProblemIdentity {
                school: Uuid::nil(),
                cycle: Uuid::nil(),
                term: None,
            },
            seed: Seed::new(1),
            status,
            termination,
            unscheduled: Vec::new(),
            score: Score::PERFECT,
            hard_violations: Vec::new(),
            unverified: UnverifiedCounts::default(),
            preflight: DiagnosticReport::empty(),
            statistics: SolveStatistics::default(),
            assignment: Assignment::empty(0),
        },
        hard_partition: ViolationPartition::default(),
        violations: Vec::new(),
        baseline_score: Score::PERFECT,
        baseline_violations: Vec::new(),
        frozen_count: 0,
        movable_count: 1,
        movable_placed: 1,
        frozen_placed: 0,
        applyable,
        defect: None,
    }
}

/// A clean, applyable outcome.
fn good() -> RepairOutcome {
    outcome_with(Termination::Complete, SolveStatus::Complete, true)
}

#[test]
fn a_new_session_is_running_and_nothing_may_be_applied() {
    let life = RepairLifecycle::default();
    assert_eq!(life.phase(), RepairPhase::Running);
    assert_eq!(
        life.claim_apply().expect_err("refused"),
        ApplyRefusal::StillRunning
    );
    assert_eq!(
        life.applyable().as_deref(),
        Some("the repair has not finished")
    );
    assert!(life.outcome().is_none());
}

#[test]
fn two_applies_from_two_clicks_leave_only_one_claim() {
    let life = RepairLifecycle::default();
    life.finish(good());

    assert!(life.claim_apply().is_ok(), "the first claim wins");
    assert_eq!(
        life.claim_apply().expect_err("refused"),
        ApplyRefusal::AlreadyApplying,
        "the second is refused rather than queued"
    );
    assert_eq!(life.phase(), RepairPhase::Applying);
}

#[test]
fn nothing_leaves_applied() {
    let life = RepairLifecycle::default();
    life.finish(good());
    let _ = life.claim_apply().expect("claimed");
    let written = Uuid::now_v7();
    life.apply_succeeded(written);

    assert_eq!(life.phase(), RepairPhase::Applied);
    assert_eq!(life.applied_as(), Some(written));
    assert_eq!(
        life.claim_apply().expect_err("refused"),
        ApplyRefusal::AlreadyApplied {
            timetable_id: written
        }
    );

    // And a late finish or a late failure changes nothing.
    life.finish(good());
    life.apply_failed();
    assert_eq!(life.phase(), RepairPhase::Applied, "terminal is terminal");
}

#[test]
fn a_failed_apply_can_be_retried() {
    let life = RepairLifecycle::default();
    life.finish(good());
    let _ = life.claim_apply().expect("claimed");
    life.apply_failed();

    assert_eq!(life.phase(), RepairPhase::Finished);
    assert!(
        life.claim_apply().is_ok(),
        "the write is retryable, so the claim is available again"
    );
}

#[test]
fn a_cancel_sets_a_flag_and_no_state() {
    let life = RepairLifecycle::default();
    assert!(life.request_cancel(), "it was running");
    assert!(life.cancel_requested());
    assert_eq!(
        life.phase(),
        RepairPhase::Running,
        "the worker is the only thing that leaves Running"
    );

    // The worker then reports what it found, which is the deterministic single outcome.
    life.finish(outcome_with(
        Termination::Cancelled,
        SolveStatus::Cancelled,
        false,
    ));
    assert_eq!(life.phase(), RepairPhase::Finished);
}

#[test]
fn a_cancel_after_a_finish_reports_that_nothing_was_stopped() {
    let life = RepairLifecycle::default();
    life.finish(good());
    assert!(
        !life.request_cancel(),
        "nothing was running, and saying otherwise would claim the cancel did something"
    );
    assert_eq!(life.phase(), RepairPhase::Finished);
}

#[test]
fn a_cancelled_outcome_is_never_applyable() {
    let life = RepairLifecycle::default();
    life.finish(outcome_with(
        Termination::Cancelled,
        SolveStatus::Cancelled,
        false,
    ));
    let refusal = life.claim_apply().expect_err("refused");
    assert!(
        matches!(refusal, ApplyRefusal::NotApplyable { .. }),
        "{refusal:?}"
    );
    assert!(life.applyable().is_some());
}

#[test]
fn a_refused_outcome_is_never_applyable() {
    let life = RepairLifecycle::default();
    life.finish(outcome_with(
        Termination::InputRejected,
        SolveStatus::Failed,
        false,
    ));
    assert!(matches!(
        life.claim_apply().expect_err("refused"),
        ApplyRefusal::NotApplyable { .. }
    ));
}

#[test]
fn a_defective_outcome_names_the_defect_and_is_never_applyable() {
    let life = RepairLifecycle::default();
    let mut broken = outcome_with(Termination::InternalCheckFailed, SolveStatus::Failed, false);
    broken.defect = Some(RepairDefect::FrozenPlacementsChanged(Vec::new()));
    life.finish(broken);

    let ApplyRefusal::NotApplyable { reason } = life.claim_apply().expect_err("refused") else {
        panic!("expected a NotApplyable refusal");
    };
    assert!(
        reason.contains("frozen-placements-changed"),
        "the defect must be named so it can be reported as a bug: {reason}"
    );
    assert!(
        reason.contains("defect in Evara"),
        "and framed as a defect rather than a timetable problem: {reason}"
    );
}

#[test]
fn a_partial_repair_is_applyable() {
    // §2.8: a repair of a timetable with out-of-scope unplaced lessons is ALWAYS partial, so
    // refusing partial would refuse almost every real repair.
    let life = RepairLifecycle::default();
    life.finish(outcome_with(
        Termination::Exhausted,
        SolveStatus::Partial,
        true,
    ));
    assert!(life.applyable().is_none());
    assert!(life.claim_apply().is_ok());
}

#[test]
fn a_budget_limited_repair_is_applyable() {
    // An honest partial arrangement. Applyable, with the limit named in the preview -- never
    // refused, which would present a limit as a conclusion.
    for limit in [
        Termination::CandidateLimitReached,
        Termination::StepLimitReached,
        Termination::PassLimitReached,
    ] {
        let life = RepairLifecycle::default();
        life.finish(outcome_with(limit, SolveStatus::Partial, true));
        assert!(life.applyable().is_none(), "{limit:?} must be applyable");
    }
}

#[test]
fn a_discarded_session_refuses_an_apply_and_stays_discarded() {
    let life = RepairLifecycle::default();
    life.finish(good());
    assert!(life.discard());
    assert!(!life.discard(), "discarding twice is not a second event");

    assert_eq!(life.phase(), RepairPhase::Discarded);
    assert_eq!(
        life.claim_apply().expect_err("refused"),
        ApplyRefusal::Discarded
    );
    assert!(
        life.cancel_requested(),
        "discarding also asks the worker to stop"
    );
    assert!(life.outcome().is_none());
}

#[test]
fn a_finish_from_a_terminal_state_is_ignored() {
    let life = RepairLifecycle::default();
    life.discard();
    life.finish(good());
    assert_eq!(
        life.phase(),
        RepairPhase::Discarded,
        "a late worker cannot resurrect a discarded session"
    );
}

#[test]
fn progress_is_readable_while_running() {
    let life = RepairLifecycle::default();
    life.report(super::Progress {
        placed: 7,
        total: 20,
        steps: 9,
    });
    let seen = life.progress();
    assert_eq!(seen.placed, 7);
    assert_eq!(seen.total, 20);
    assert_eq!(seen.steps, 9);
}

#[test]
fn the_outcome_survives_into_applied_so_the_preview_still_renders() {
    let life = RepairLifecycle::default();
    life.finish(good());
    let _ = life.claim_apply().expect("claimed");
    life.apply_succeeded(Uuid::now_v7());

    assert!(
        life.outcome().is_some(),
        "a user looking at the preview after applying must still see it"
    );
}

#[test]
fn the_registry_clears_every_session_when_a_project_closes() {
    use super::RepairSessions;

    let sessions = RepairSessions::default();
    assert!(sessions.is_empty());
    assert_eq!(sessions.running(), 0);

    // The registry's bookkeeping is exercised without a session, because building one needs
    // an `Assembly` and these tests are deliberately pure. `clear` on an empty registry must
    // still be safe, which is what a project close relies on.
    sessions.clear();
    assert!(sessions.is_empty());
    assert!(!sessions.remove(Uuid::now_v7()), "nothing to remove");
    assert!(sessions.get(Uuid::now_v7()).is_none());
}
