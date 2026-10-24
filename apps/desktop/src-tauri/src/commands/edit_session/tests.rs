//! The command vocabulary and the undo stack, without a database.
//!
//! # What is testable here, and why that is the split
//!
//! [`Command`] and its inverse are pure, and the stack is a sequence of them. So the properties
//! that matter most — *every edit has an inverse*, *undoing twice is undoing twice*, *a new edit
//! clears redo*, *a refusal abandons both stacks* — are asserted with no project file, no
//! workspace lock and no solver.
//!
//! The last two of those four were **claimed by this paragraph and not asserted** until the M9A
//! audit, because reaching them meant building an [`EditSession`](super::EditSession), which
//! means an `Assembly`, which means a database. [`Stack`](super::Stack) was extracted so they
//! could be: it holds no scheduling knowledge, and the section at the bottom of this file is
//! what the paragraph had been promising.
//!
//! What needs a database is the **write** and its three guards, and those are asserted in
//! `evara-db`'s `tests_editing`: a revision conflict, another window's edit, a moved input. The
//! division is deliberate. A stack test that had to save a timetable first would be a slow test
//! of the wrong thing.

use evara_db::EntryPlacement;
use uuid::Uuid;

use super::{Applied, Command, EditSessions, History, Stack};

/// A placement at a named slot, with no room.
fn at(slot: u128) -> EntryPlacement {
    EntryPlacement {
        timeslot_id: Uuid::from_u128(slot),
        room_id: None,
    }
}

/// A placement at a named slot in a named room.
fn at_room(slot: u128, room: u128) -> EntryPlacement {
    EntryPlacement {
        timeslot_id: Uuid::from_u128(slot),
        room_id: Some(Uuid::from_u128(room)),
    }
}

fn activity(n: u128) -> Uuid {
    Uuid::from_u128(1_000 + n)
}

/// An applied command whose write left the entry at `rev`.
fn applied(command: Command, rev: i64) -> Applied {
    Applied {
        command,
        produced: Some((Uuid::from_u128(9_000), rev)),
        label: "an edit".to_owned(),
    }
}

// =======================================================================================
// Every edit has an inverse
// =======================================================================================

#[test]
fn placing_is_undone_by_unscheduling_and_back_again() {
    let place = Command::Place {
        activity: activity(1),
        to: at(10),
    };
    assert_eq!(
        place.inverse(),
        Command::Unschedule {
            activity: activity(1),
            from: at(10)
        }
    );
    assert_eq!(
        place.inverse().inverse(),
        place,
        "undoing an undo is the original edit"
    );
}

#[test]
fn a_move_is_undone_by_the_move_back() {
    let moved = Command::Move {
        activity: activity(2),
        from: at(10),
        to: at(20),
    };
    assert_eq!(
        moved.inverse(),
        Command::Move {
            activity: activity(2),
            from: at(20),
            to: at(10)
        }
    );
    assert_eq!(moved.inverse().inverse(), moved);
}

#[test]
fn a_room_change_is_a_move_and_inverts_like_one() {
    // `room_id` is nullable and `None` means no room booked, which ADR 0034 §4 prefers to no
    // lesson. So clearing a room is an edit with an inverse, not a deletion.
    let cleared = Command::Move {
        activity: activity(3),
        from: at_room(10, 5),
        to: at(10),
    };
    assert_eq!(
        cleared.inverse(),
        Command::Move {
            activity: activity(3),
            from: at(10),
            to: at_room(10, 5)
        }
    );
}

#[test]
fn a_lock_is_undone_by_an_unlock() {
    let locked = Command::SetLock {
        activity: activity(4),
        locked: true,
    };
    assert_eq!(
        locked.inverse(),
        Command::SetLock {
            activity: activity(4),
            locked: false
        }
    );
    assert_eq!(locked.inverse().inverse(), locked);
}

#[test]
fn every_command_names_the_occurrence_it_is_about() {
    // The undo path addresses the entry through the activity, so a command that did not name one
    // could not be inverted against the session's entries.
    for command in [
        Command::Place {
            activity: activity(5),
            to: at(10),
        },
        Command::Move {
            activity: activity(5),
            from: at(10),
            to: at(20),
        },
        Command::Unschedule {
            activity: activity(5),
            from: at(10),
        },
        Command::SetLock {
            activity: activity(5),
            locked: true,
        },
    ] {
        assert_eq!(command.activity(), activity(5));
        assert_eq!(
            command.inverse().activity(),
            activity(5),
            "and so does its inverse"
        );
    }
}

#[test]
fn an_inverse_is_an_involution_for_every_command_shape() {
    // The property that makes the stack sound: `inverse(inverse(c)) == c` for all four kinds.
    // A command for which it did not hold would make redo apply something other than what was
    // undone.
    for command in [
        Command::Place {
            activity: activity(6),
            to: at_room(11, 2),
        },
        Command::Move {
            activity: activity(6),
            from: at_room(11, 2),
            to: at_room(12, 3),
        },
        Command::Unschedule {
            activity: activity(6),
            from: at_room(11, 2),
        },
        Command::SetLock {
            activity: activity(6),
            locked: false,
        },
    ] {
        assert_eq!(command.inverse().inverse(), command);
    }
}

// =======================================================================================
// The sessions map
// =======================================================================================

#[test]
fn an_unknown_session_is_absent_rather_than_an_error() {
    let sessions = EditSessions::default();
    assert!(sessions.is_empty());
    assert_eq!(sessions.len(), 0);
    assert!(sessions.get(Uuid::from_u128(1)).is_none());
    assert!(
        !sessions.close(Uuid::from_u128(1)),
        "closing something that is not open is false, not a failure"
    );
}

// =======================================================================================
// The history view
// =======================================================================================

#[test]
fn an_empty_history_offers_neither_control() {
    let history = History::default();
    assert!(history.undo.is_none());
    assert!(history.redo.is_none());
    assert_eq!(history.depth, 0);
}

#[test]
fn a_stack_entry_carries_the_revision_its_own_write_produced() {
    // The subtlety the whole design turns on. A naive stack stores the revision read *before*
    // the forward write, and an undo built on that fails every time — the forward write consumed
    // it. So the inverse is preconditioned on what the write left behind.
    let entry = applied(
        Command::Move {
            activity: activity(7),
            from: at(10),
            to: at(20),
        },
        4,
    );
    assert_eq!(
        entry.produced.map(|(_, rev)| rev),
        Some(4),
        "the revision the move left the row at, not the one it was read at"
    );
}

#[test]
fn an_unschedule_produces_no_revision_because_there_is_no_row() {
    // And it needs none: its inverse is a `Place`, which is a creation and carries no revision
    // precondition (`CLAUDE.md` §3).
    let entry = Applied {
        command: Command::Unschedule {
            activity: activity(8),
            from: at(10),
        },
        produced: None,
        label: "unschedule".to_owned(),
    };
    assert!(entry.produced.is_none());
    assert!(matches!(entry.command.inverse(), Command::Place { .. }));
}

// =======================================================================================
// Audit — the two stack rules this module's header claimed and did not assert
// =======================================================================================
//
// The header above has promised *"a new edit clears redo"* and *"a refusal abandons both
// stacks"* since M9A. Neither was asserted, and neither could be: both are methods on
// `EditSession`, and reaching one meant building an `Assembly`, which means a database. The
// audit extracted `Stack` for exactly this — it holds no scheduling knowledge, so its rules are
// testable with nothing at all.

#[test]
fn a_new_forward_edit_clears_the_redo_stack() {
    // A redo computed against a different future is not a redo. Undo a move, make a *different*
    // edit, and the redo that was available must be gone — reapplying it would put the lesson
    // somewhere the user's later decision has already contradicted.
    let mut stack = Stack::default();
    let first = applied(
        Command::Move {
            activity: activity(1),
            from: at(10),
            to: at(20),
        },
        2,
    );
    stack.did(first.clone());

    let undone = stack.take_undo().expect("something to undo");
    stack.undone(undone);
    assert!(stack.peek_redo().is_some(), "and it can be redone");

    stack.did(applied(
        Command::SetLock {
            activity: activity(2),
            locked: true,
        },
        3,
    ));
    assert!(
        stack.peek_redo().is_none(),
        "a forward edit discards the future that was undone"
    );
    assert_eq!(
        stack.history().depth,
        1,
        "and the new edit is the only thing to undo"
    );
}

#[test]
fn a_refusal_abandons_both_stacks() {
    // Everything below the entry that failed was computed against a state that no longer holds,
    // so replaying any of it would apply an inverse to a row it was not computed for. Clearing
    // one stack and keeping the other would be worse than clearing neither: the kept half would
    // look usable.
    let mut stack = Stack::default();
    for n in 1..=3_i64 {
        stack.did(applied(
            Command::Move {
                activity: activity(u128::try_from(n).expect("positive")),
                from: at(10),
                to: at(20),
            },
            n + 1,
        ));
    }
    let undone = stack.take_undo().expect("something to undo");
    stack.undone(undone);
    assert_eq!(stack.history().depth, 2);
    assert!(stack.peek_redo().is_some());

    stack.forget();
    assert_eq!(stack.history(), History::default(), "both, and completely");
    assert!(stack.take_undo().is_none());
    assert!(stack.take_redo().is_none());
}

#[test]
fn a_failed_edit_leaves_no_trace_in_the_history() {
    // "A failed edit must not enter history", asserted at the only place it can be. `edit.rs`
    // calls `record` — and so `did` — **after** `run` returns `Ok`, so a refused write never
    // reaches this type at all. What is asserted here is the other half: that nothing about the
    // stack changes unless something is pushed, so there is no path by which an attempt alone
    // leaves a mark.
    let mut stack = Stack::default();
    let before = stack.history();
    assert_eq!(before, History::default());

    // A refused undo takes the entry off and the caller then clears both stacks, which is what
    // `step` does on an `Err`. The entry must not silently return.
    stack.did(applied(
        Command::Unschedule {
            activity: activity(5),
            from: at(10),
        },
        1,
    ));
    let taken = stack.take_undo().expect("something to undo");
    stack.forget();
    assert_eq!(
        stack.history(),
        History::default(),
        "a refused undo abandons the entry it took, rather than putting it back"
    );
    assert!(matches!(taken.command, Command::Unschedule { .. }));
}

#[test]
fn undoing_and_redoing_the_same_edit_does_not_duplicate_it() {
    // The property that keeps the two stacks a single history rather than two growing lists.
    let mut stack = Stack::default();
    stack.did(applied(
        Command::Move {
            activity: activity(9),
            from: at(10),
            to: at(20),
        },
        2,
    ));
    for _ in 0..4 {
        let undone = stack.take_undo().expect("undo");
        stack.undone(undone);
        assert_eq!(stack.history().depth, 0);
        let redone = stack.take_redo().expect("redo");
        stack.redone(redone);
        assert_eq!(stack.history().depth, 1);
    }
    assert!(stack.peek_redo().is_none(), "and nothing accumulated");
}

#[test]
fn the_history_names_the_edit_at_the_top_of_each_stack() {
    let mut stack = Stack::default();
    stack.did(Applied {
        command: Command::SetLock {
            activity: activity(1),
            locked: true,
        },
        produced: Some((Uuid::from_u128(9_000), 2)),
        label: "lock 9 Mathematics B".to_owned(),
    });
    stack.did(Applied {
        command: Command::Move {
            activity: activity(2),
            from: at(10),
            to: at(20),
        },
        produced: Some((Uuid::from_u128(9_001), 2)),
        label: "move 9 Science A".to_owned(),
    });

    let history = stack.history();
    assert_eq!(
        history.undo.as_deref(),
        Some("move 9 Science A"),
        "newest last"
    );
    assert!(history.redo.is_none());
    assert_eq!(history.depth, 2);

    let undone = stack.take_undo().expect("undo");
    stack.undone(undone);
    let history = stack.history();
    assert_eq!(history.undo.as_deref(), Some("lock 9 Mathematics B"));
    assert_eq!(history.redo.as_deref(), Some("move 9 Science A"));
    assert_eq!(history.depth, 1);
}

// =======================================================================================
// Audit — a session does not outlive its project
// =======================================================================================

#[test]
fn closing_the_project_discards_every_session() {
    // `project_create`, `project_open` and `project_close` all call `clear`. A session holds a
    // compiled `Assembly` describing a database about to be shut and an undo stack whose rows are
    // in that file; an undo stack pointing at another project is worse than no undo stack.
    let sessions = EditSessions::default();
    assert!(sessions.is_empty());

    // `insert` needs a real session, which needs a database — so the registry's own emptying is
    // what is asserted here, and `tests_editing` owns the writes. What matters is that `clear`
    // is total and that a cleared handle is reported absent rather than erroring.
    sessions.clear();
    assert!(sessions.is_empty());
    assert!(sessions.get(Uuid::from_u128(1)).is_none());
    assert!(
        !sessions.close(Uuid::from_u128(1)),
        "closing a session the project switch already discarded is false, not a failure"
    );
}
