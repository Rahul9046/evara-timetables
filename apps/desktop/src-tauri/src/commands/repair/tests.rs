//! What the command layer shapes, without a Tauri state to drive.
//!
//! The behaviour that needs a project file is covered where it belongs — `evara-db`'s
//! `tests_repair` for the writes and the guards, `evara-solver`'s `tests_repair` for the
//! verification, and [`super::super::repair_session::tests`] for the six hazards. What is left
//! here is this module's own arithmetic and wording, which is where a wrong remedy or a dropped
//! field would otherwise reach a screen unnoticed.

use evara_core::scheduling::{RepairScope, ScopeError, SolveStatus, Termination};
use evara_db::UnexpressibleLockCause;
use uuid::Uuid;

use super::{
    PreflightFinding, RepairPreflight, blocking_message, describe_unexpressible, scope_message,
    status_key, stopped_at_a_limit,
};

#[test]
fn every_unexpressible_cause_names_a_remedy() {
    for cause in [
        UnexpressibleLockCause::ActivityNotInProblem,
        UnexpressibleLockCause::TimeslotNotInProblem {
            timeslot_id: Uuid::nil(),
        },
        UnexpressibleLockCause::RoomNotInProblem {
            room_id: Uuid::nil(),
        },
    ] {
        let (message, remedy) = describe_unexpressible(cause);
        assert!(!message.is_empty(), "{cause:?} needs a message");
        assert!(
            !remedy.is_empty(),
            "{cause:?} needs a remedy — a blocking finding with no remedy strands the user"
        );
        // The remedy must be an action, which is the point of blocking rather than warning.
        assert!(
            remedy.contains("Repair")
                || remedy.contains("Unschedule")
                || remedy.contains("Re-place"),
            "{cause:?}: the remedy must say what to do, got {remedy}"
        );
        // And nothing offers to clear the lock on the user's behalf.
        assert!(
            !remedy.to_lowercase().contains("we will"),
            "{cause:?}: nothing in the repair path unlocks anything for the user"
        );
    }
}

#[test]
fn a_scope_error_reads_as_a_sentence() {
    assert!(
        scope_message(&ScopeError::NotInProblem {
            entity: "room",
            id: Uuid::nil()
        })
        .contains("room"),
        "the entity must be named"
    );
    let empty = scope_message(&ScopeError::Empty { scope: "room" });
    assert!(
        empty.contains("nothing") && empty.contains("room"),
        "an empty scope says so plainly: {empty}"
    );
}

#[test]
fn a_limit_is_distinguished_from_a_finished_search() {
    assert!(stopped_at_a_limit(Termination::CandidateLimitReached));
    assert!(stopped_at_a_limit(Termination::StepLimitReached));
    assert!(stopped_at_a_limit(Termination::PassLimitReached));

    // These are not limits, and reporting them as such would present a conclusion as a budget
    // or a budget as a conclusion.
    assert!(!stopped_at_a_limit(Termination::Complete));
    assert!(!stopped_at_a_limit(Termination::Exhausted));
    assert!(!stopped_at_a_limit(Termination::Cancelled));
    assert!(!stopped_at_a_limit(Termination::InputRejected));
    assert!(!stopped_at_a_limit(Termination::InternalCheckFailed));
}

#[test]
fn the_status_keys_match_what_the_schema_stores() {
    // `solve_run.status`' CHECK, verbatim.
    assert_eq!(status_key(SolveStatus::Complete), "COMPLETE");
    assert_eq!(status_key(SolveStatus::Partial), "PARTIAL");
    assert_eq!(status_key(SolveStatus::Infeasible), "INFEASIBLE");
    assert_eq!(status_key(SolveStatus::Cancelled), "CANCELLED");
    assert_eq!(status_key(SolveStatus::Failed), "FAILED");
}

#[test]
fn the_blocking_message_carries_only_blocking_findings() {
    let checked = RepairPreflight {
        may_repair: false,
        findings: vec![
            PreflightFinding {
                code: "timeslot-not-in-problem".to_owned(),
                blocking: true,
                message: "A lock names a period that is gone.".to_owned(),
                remedy: "Re-place it.".to_owned(),
                activity_id: None,
            },
            PreflightFinding {
                code: "room-clash".to_owned(),
                blocking: false,
                message: "Already present, and outside the scope.".to_owned(),
                remedy: "Repair a wider scope.".to_owned(),
                activity_id: None,
            },
        ],
        movable: 3,
        frozen: 9,
        locked: 1,
    };

    let message = blocking_message(&checked);
    assert!(
        message.contains("period that is gone"),
        "the blocking finding must be in it: {message}"
    );
    assert!(
        !message.contains("outside the scope"),
        "a non-blocking finding must not be, or the refusal names things that did not cause it: \
         {message}"
    );
}

#[test]
fn a_scope_serialises_with_its_kind_tag() {
    // The interface switches on `kind`, so the tag has to be there and has to be camelCase.
    let json = serde_json::to_string(&RepairScope::Whole).expect("serialise");
    assert_eq!(json, r#"{"kind":"whole"}"#);

    let json = serde_json::to_string(&RepairScope::ClassSection {
        section_id: Uuid::nil(),
    })
    .expect("serialise");
    assert!(json.contains(r#""kind":"classSection""#), "got {json}");
    assert!(json.contains("sectionId"), "got {json}");

    let json = serde_json::to_string(&RepairScope::CycleDay {
        cycle_day_id: Uuid::nil(),
    })
    .expect("serialise");
    assert!(json.contains(r#""kind":"cycleDay""#), "got {json}");
    assert!(json.contains("cycleDayId"), "got {json}");
}

#[test]
fn a_scope_round_trips_through_json() {
    // `repair_start` takes back what `repair_scopes` handed out, so the two have to agree.
    for scope in [
        RepairScope::Whole,
        RepairScope::ClassSection {
            section_id: Uuid::now_v7(),
        },
        RepairScope::Teacher {
            teacher_id: Uuid::now_v7(),
        },
        RepairScope::Room {
            room_id: Uuid::now_v7(),
        },
        RepairScope::CycleDay {
            cycle_day_id: Uuid::now_v7(),
        },
        RepairScope::Occurrences {
            activity_ids: vec![Uuid::now_v7(), Uuid::now_v7()],
        },
    ] {
        let json = serde_json::to_string(&scope).expect("serialise");
        let back: RepairScope = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, scope, "round trip failed for {}", scope.key());
    }
}

#[test]
fn the_event_names_are_distinct_from_the_generation_events() {
    // A screen listening for a repair must not be woken by a generation, and the other way
    // round -- both run on their own worker threads and both emit progress.
    assert_eq!(super::PROGRESS_EVENT, "repair://progress");
    assert_eq!(super::FINISHED_EVENT, "repair://finished");
    assert_ne!(
        super::PROGRESS_EVENT,
        crate::commands::solve::PROGRESS_EVENT
    );
    assert_ne!(
        super::FINISHED_EVENT,
        crate::commands::solve::FINISHED_EVENT
    );
}
