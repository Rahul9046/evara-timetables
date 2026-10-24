//! An editing session: the compiled problem, the arrangement, and an undo stack in Rust.
//!
//! # What a session holds, and why it is not a cached scorer
//!
//! Opening a timetable for editing is expensive in exactly one way: it reads the database. The
//! compiled [`Assembly`], the display labels and the school's constraint rules are a fixed
//! number of statements each, and they are what this type keeps.
//!
//! What it deliberately does **not** keep is an [`IncrementalScorer`]. A scorer borrows the
//! problem it is about, so holding one beside the problem in the same struct would be
//! self-referential — and the reason not to is better than the borrow checker's: a cached
//! scorer is a second statement of what is placed, and the moment it disagrees with the
//! `timetable_entry` rows every verdict the editor gives is about a timetable nobody can see. So
//! a scorer is built per command from the entries this session holds, which costs one reference
//! evaluation and **no database access at all** — see [`EditSession::scorer`].
//!
//! # The three guards, and the one that is new
//!
//! | Guard | Catches | Held as |
//! | --- | --- | --- |
//! | `expectedRev` | somebody moved *this* placement | the interface's, echoed back per write |
//! | `ArrangementStamp` | somebody moved *another* placement of this timetable | [`EditSession::stamp`] |
//! | the input watermark | a scheduling *input* moved | `assembly.watermark` |
//! | [`ProjectEpoch`](super::session::ProjectEpoch) | the campus file was closed and another opened | [`EditSession::epoch`] |
//!
//! Every one of them is compared **inside the writing transaction** by `evara-db`, except the
//! epoch, which is this layer's and is checked before a transaction opens. Nothing here compares
//! a watermark or a stamp itself — ADR 0030's rule, which `CLAUDE.md` §3 restates.
//!
//! # Why the undo stack is here and not in TypeScript
//!
//! Three reasons, and the third is the one that settles it. An inverse command is a statement
//! about scheduling state. Its precondition is a revision the database owns. And a stack in the
//! view layer would be a *second* place that decides what reversing a move means — which is
//! `CLAUDE.md` §2's rule about a rule implemented twice, applied to the rule *"undoing a move
//! is the move back"*.
//!
//! The stack is in memory and does not survive the session, which is `OPEN-DECISIONS.md` D9's
//! accepted trade: *"in-memory inverse commands for M1, designed so a persisted log can back it
//! later"*. What **is** durable is `change_log`, which records the forward edit and its undo as
//! two entries — because an undo is a new transaction and not a rollback.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use evara_core::scheduling::display::LabelSet;
use evara_core::scheduling::{
    ActivityIx, Assignment, CompiledConstraint, Placement, Problem, Score,
};
use evara_db::scheduling::{Assembly, SchedulingSelection};
use evara_db::{ArrangementStamp, EntryPlacement, TimetableEntry};
use uuid::Uuid;

/// One manual edit, in the vocabulary an undo is written in.
///
/// Stored identifiers rather than dense indices: an inverse is handed straight back to the
/// repository, and translating twice would be two chances to translate differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Put an unplaced occurrence somewhere.
    Place {
        /// Which occurrence.
        activity: Uuid,
        /// Where it goes.
        to: EntryPlacement,
    },
    /// Move a placed occurrence.
    Move {
        /// Which occurrence.
        activity: Uuid,
        /// Where it was, so the inverse needs no lookup.
        from: EntryPlacement,
        /// Where it goes.
        to: EntryPlacement,
    },
    /// Take a placed occurrence off the timetable.
    Unschedule {
        /// Which occurrence.
        activity: Uuid,
        /// Where it was.
        from: EntryPlacement,
    },
    /// Lock or unlock a placement.
    SetLock {
        /// Which occurrence.
        activity: Uuid,
        /// What the flag becomes.
        locked: bool,
    },
}

impl Command {
    /// Which occurrence this is about.
    #[must_use]
    pub const fn activity(&self) -> Uuid {
        match self {
            Self::Place { activity, .. }
            | Self::Move { activity, .. }
            | Self::Unschedule { activity, .. }
            | Self::SetLock { activity, .. } => *activity,
        }
    }

    /// The command that undoes this one.
    ///
    /// Total: every edit this milestone can make has an inverse of the same four kinds, which is
    /// what makes undo a property of the vocabulary rather than a feature bolted onto it.
    #[must_use]
    pub const fn inverse(&self) -> Self {
        match *self {
            Self::Place { activity, to } => Self::Unschedule { activity, from: to },
            Self::Move { activity, from, to } => Self::Move {
                activity,
                from: to,
                to: from,
            },
            Self::Unschedule { activity, from } => Self::Place { activity, to: from },
            Self::SetLock { activity, locked } => Self::SetLock {
                activity,
                locked: !locked,
            },
        }
    }
}

/// One command that was applied, and what its own write left behind.
///
/// # The revision an inverse is preconditioned on
///
/// The naive stack records the revision read *before* the forward write, and an undo built on it
/// fails every time: the forward write consumed it. So each applied command records the revision
/// its **own** write produced, and the inverse's precondition is that. If the row has moved since,
/// something outside this stack changed it — and the undo is refused rather than applied over
/// somebody else's edit.
#[derive(Debug, Clone)]
pub struct Applied {
    /// What was done.
    pub command: Command,
    /// The entry and the revision the write left it at, or `None` for an unschedule.
    pub produced: Option<(Uuid, i64)>,
    /// What to call it in an "Undo …" control.
    pub label: String,
}

/// What the interface needs to render undo and redo controls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    /// What undo would reverse, or `None` when there is nothing.
    pub undo: Option<String>,
    /// What redo would reapply.
    pub redo: Option<String>,
    /// How deep the undo stack is.
    pub depth: usize,
}

/// The two stacks, and the four rules about moving entries between them.
///
/// # Why this is its own type
///
/// It holds no scheduling knowledge and needs no project, so its rules can be asserted with no
/// database, no workspace lock and no compiled problem. That is the whole reason: *"a new forward
/// edit clears redo"* and *"a refusal abandons both"* are the two properties most likely to be
/// got wrong and were the two this module's tests claimed to cover without covering — they could
/// not, because reaching them meant building an [`EditSession`], which means building an
/// `Assembly`, which means a database.
///
/// [`EditSession`] delegates to it and adds nothing.
#[derive(Debug, Default)]
pub struct Stack {
    /// Applied commands, newest last.
    undo: Vec<Applied>,
    /// Undone commands, newest last. Cleared by any new forward command.
    redo: Vec<Applied>,
}

impl Stack {
    /// Pushes an applied forward command, clearing the redo stack.
    ///
    /// Clearing is what every editor does and is not a nicety: a redo computed against a
    /// different future is not a redo.
    pub fn did(&mut self, applied: Applied) {
        self.undo.push(applied);
        self.redo.clear();
    }

    /// Takes the next command to undo.
    pub fn take_undo(&mut self) -> Option<Applied> {
        self.undo.pop()
    }

    /// Takes the next command to redo.
    pub fn take_redo(&mut self) -> Option<Applied> {
        self.redo.pop()
    }

    /// Records that a command was undone, so it can be redone.
    pub fn undone(&mut self, applied: Applied) {
        self.redo.push(applied);
    }

    /// Records that a command was redone, so it can be undone again.
    pub fn redone(&mut self, applied: Applied) {
        self.undo.push(applied);
    }

    /// Abandons both stacks.
    ///
    /// Called when a write is refused for a reason that means the world moved: every entry below
    /// the one that failed was computed against a state that no longer holds, so replaying any of
    /// them would apply an inverse to a row it was not computed for.
    pub fn forget(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// What undo would reverse next, without removing it.
    #[must_use]
    pub fn peek_undo(&self) -> Option<&Applied> {
        self.undo.last()
    }

    /// What redo would reapply next, without removing it.
    #[must_use]
    pub fn peek_redo(&self) -> Option<&Applied> {
        self.redo.last()
    }

    /// What the interface needs to render the two controls.
    #[must_use]
    pub fn history(&self) -> History {
        History {
            undo: self.undo.last().map(|a| a.label.clone()),
            redo: self.redo.last().map(|a| a.label.clone()),
            depth: self.undo.len(),
        }
    }
}

/// Why an edit was refused before any rule ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRefusal {
    /// The occurrence is locked in this timetable.
    ///
    /// Not a violation and not routed through the rule engine: a lock is a decision the user
    /// already recorded, and reporting it as a clash would make unlocking feel like overriding a
    /// safety check. `activity-fixed` is the registry's backstop for anything that tries anyway —
    /// ADR 0045 §5.
    Locked {
        /// What it is called, for the message.
        label: String,
    },
    /// The occurrence is not in the compiled problem.
    NotInProblem,
    /// The occurrence is already placed, and the caller asked to place it.
    AlreadyPlaced,
    /// The occurrence is not placed, and the caller asked to move or unschedule it.
    NotPlaced,
    /// There is nothing to undo, or nothing to redo.
    NothingToDo,
}

/// One timetable, open for editing.
///
/// Lives behind a `Mutex` in [`EditSessions`]: one user, one timetable, one command at a time.
/// That serialisation is deliberate rather than incidental — two concurrent commands on one
/// session would interleave their stamps and their stacks, and the result could not be undone
/// coherently.
pub struct EditSession {
    /// This session's identity, which every command carries.
    pub session_id: Uuid,
    /// Which timetable is open.
    pub timetable_id: Uuid,
    /// Which school, cycle and term.
    pub selection: SchedulingSelection,
    /// Which project this belongs to.
    pub epoch: u64,
    /// The compiled problem, with `pinned` kept current from the locks below.
    assembly: Assembly,
    /// The display names the grid is drawn with.
    labels: LabelSet,
    /// The rules every verdict comes from.
    rules: Vec<CompiledConstraint>,
    /// The arrangement, as stored rows, keyed by `activity_id`.
    ///
    /// **The single source of truth for what is placed.** The assignment, the pins and the
    /// scorer are all derived from this, so none of them can disagree with the database about a
    /// placement that was written.
    entries: BTreeMap<Uuid, TimetableEntry>,
    /// What this session believes the whole arrangement is.
    stamp: ArrangementStamp,
    /// Locked entries naming something the problem no longer holds.
    pub pins_not_in_problem: Vec<Uuid>,
    /// Entries naming something the problem no longer holds.
    pub not_in_problem: Vec<Uuid>,
    /// The undo and redo stacks. See [`Stack`].
    stack: Stack,
    /// Bumped on every applied edit, so a view can tell whether it is current.
    pub version: u64,
}

impl std::fmt::Debug for EditSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditSession")
            .field("session_id", &self.session_id)
            .field("timetable_id", &self.timetable_id)
            .field("epoch", &self.epoch)
            .field("entries", &self.entries.len())
            .field("history", &self.stack.history())
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl EditSession {
    /// Opens a session over a compiled problem and the arrangement it holds.
    ///
    /// `pinned` is written onto the problem here, which is the one place it happens: a lock is a
    /// `timetable_entry` column and the loader compiles inputs only (ADR 0045 §4).
    #[allow(
        clippy::too_many_arguments,
        reason = "every one of these is read from a \
        different place under one lock, and grouping them into a struct would only move the \
        argument list to the call site"
    )]
    pub fn new(
        session_id: Uuid,
        timetable_id: Uuid,
        selection: SchedulingSelection,
        epoch: u64,
        mut assembly: Assembly,
        labels: LabelSet,
        rules: Vec<CompiledConstraint>,
        entries: Vec<TimetableEntry>,
        stamp: ArrangementStamp,
        pinned: Vec<Option<Placement>>,
        pins_not_in_problem: Vec<Uuid>,
    ) -> Self {
        assembly.problem.pinned = pinned;
        let not_in_problem = entries
            .iter()
            .filter(|entry| assembly.index.activity_index(entry.activity_id).is_none())
            .map(|entry| entry.activity_id)
            .collect();
        Self {
            session_id,
            timetable_id,
            selection,
            epoch,
            assembly,
            labels,
            rules,
            entries: entries
                .into_iter()
                .map(|entry| (entry.activity_id, entry))
                .collect(),
            stamp,
            pins_not_in_problem,
            not_in_problem,
            stack: Stack::default(),
            version: 0,
        }
    }

    /// The compiled problem, pins included.
    #[must_use]
    pub const fn problem(&self) -> &Problem {
        &self.assembly.problem
    }

    /// The assembly, for a write that needs its watermark or its index.
    #[must_use]
    pub const fn assembly(&self) -> &Assembly {
        &self.assembly
    }

    /// The display names.
    #[must_use]
    pub const fn labels(&self) -> &LabelSet {
        &self.labels
    }

    /// The rules every verdict comes from.
    #[must_use]
    pub fn rules(&self) -> &[CompiledConstraint] {
        &self.rules
    }

    /// What this session believes the arrangement is.
    #[must_use]
    pub const fn stamp(&self) -> &ArrangementStamp {
        &self.stamp
    }

    /// The stored entries, by `activity_id`.
    #[must_use]
    pub const fn entries(&self) -> &BTreeMap<Uuid, TimetableEntry> {
        &self.entries
    }

    /// One occurrence's stored entry, if it is placed.
    #[must_use]
    pub fn entry_of(&self, activity_id: Uuid) -> Option<&TimetableEntry> {
        self.entries.get(&activity_id)
    }

    /// Whether one occurrence's placement is locked.
    #[must_use]
    pub fn is_locked(&self, activity_id: Uuid) -> bool {
        self.entries
            .get(&activity_id)
            .is_some_and(|entry| entry.is_locked)
    }

    /// The arrangement these entries describe, against the compiled problem.
    ///
    /// Built from the rows rather than maintained alongside them, so there is nothing to drift.
    /// An entry naming something the problem no longer holds is skipped and is reported in
    /// [`EditSession::not_in_problem`] — the same decision `Timetables::arrangement` makes, for
    /// the same reason.
    #[must_use]
    pub fn assignment(&self) -> Assignment {
        let mut assignment = Assignment::empty(self.assembly.problem.activities.len());
        for entry in self.entries.values() {
            if let Some(placement) = self.placement_of(entry)
                && let Some(activity) = self.assembly.index.activity_index(entry.activity_id)
            {
                assignment.place(activity, placement);
            }
        }
        assignment
    }

    /// The pins the current locks describe.
    ///
    /// Recomputed after a lock changes, because `Problem::pinned` is what `activity-fixed` reads
    /// and a stale copy would have the editor refusing a move the user had just unlocked.
    #[must_use]
    pub fn pins(&self) -> Vec<Option<Placement>> {
        let locked: Vec<(ActivityIx, Placement)> = self
            .entries
            .values()
            .filter(|entry| entry.is_locked)
            .filter_map(|entry| {
                let activity = self.assembly.index.activity_index(entry.activity_id)?;
                Some((activity, self.placement_of(entry)?))
            })
            .collect();
        if locked.is_empty() {
            return Vec::new();
        }
        let mut pinned = vec![None; self.assembly.problem.activities.len()];
        for (activity, placement) in locked {
            if let Some(slot) = pinned.get_mut(activity.get()) {
                *slot = Some(placement);
            }
        }
        pinned
    }

    /// One stored entry as a dense placement, where the problem holds everything it names.
    #[must_use]
    pub fn placement_of(&self, entry: &TimetableEntry) -> Option<Placement> {
        let timeslot = self.assembly.index.timeslot_index(entry.timeslot_id)?;
        let room = match entry.room_id {
            None => None,
            Some(room) => Some(self.assembly.index.room_index(room)?),
        };
        Some(Placement { timeslot, room })
    }

    /// Where an occurrence is, as stored identifiers.
    #[must_use]
    pub fn stored_placement_of(&self, activity_id: Uuid) -> Option<EntryPlacement> {
        self.entries.get(&activity_id).map(|entry| EntryPlacement {
            timeslot_id: entry.timeslot_id,
            room_id: entry.room_id,
        })
    }

    /// Records what one applied write did, and bumps the version.
    ///
    /// The entries map and the stamp move together, because they are two views of one write: a
    /// stamp that advanced while the map did not would make the next command's guard pass against
    /// an arrangement this session no longer describes.
    pub fn observe(
        &mut self,
        activity_id: Uuid,
        entry: Option<TimetableEntry>,
        stamp: ArrangementStamp,
    ) {
        match entry {
            Some(entry) => {
                self.entries.insert(activity_id, entry);
            }
            None => {
                self.entries.remove(&activity_id);
            }
        }
        self.stamp = stamp;
        self.assembly.problem.pinned = self.pins();
        self.version += 1;
    }

    /// Pushes an applied forward command, clearing the redo stack.
    pub fn did(&mut self, applied: Applied) {
        self.stack.did(applied);
    }

    /// What undo would reverse next, without removing it.
    #[must_use]
    pub fn peek_undo(&self) -> Option<&Applied> {
        self.stack.peek_undo()
    }

    /// What redo would reapply next, without removing it.
    #[must_use]
    pub fn peek_redo(&self) -> Option<&Applied> {
        self.stack.peek_redo()
    }

    /// Takes the next command to undo.
    pub fn take_undo(&mut self) -> Option<Applied> {
        self.stack.take_undo()
    }

    /// Takes the next command to redo.
    pub fn take_redo(&mut self) -> Option<Applied> {
        self.stack.take_redo()
    }

    /// Records that a command was undone, so it can be redone.
    pub fn undone(&mut self, applied: Applied) {
        self.stack.undone(applied);
    }

    /// Records that a command was redone, so it can be undone again.
    pub fn redone(&mut self, applied: Applied) {
        self.stack.redone(applied);
    }

    /// Abandons both stacks. See [`Stack::forget`].
    pub fn forget_history(&mut self) {
        self.stack.forget();
    }

    /// What the interface needs to render the two controls.
    #[must_use]
    pub fn history(&self) -> History {
        self.stack.history()
    }

    /// A scorer over the current arrangement.
    ///
    /// # Built per command, and why that is affordable
    ///
    /// One reference evaluation of the timetable and **no database access**, so it cannot block
    /// another command and cannot be stale. A drag asks for one of these and then probes every
    /// candidate cell against it, so the cost is paid once per interaction rather than once per
    /// cell — see `edit_candidates`.
    ///
    /// `None` when the arrangement cannot be read against the problem, which means the stored
    /// rows name something the compiled problem does not hold. That is reported rather than
    /// guessed at, because a scorer over an unreadable arrangement would maintain a number about
    /// nothing.
    #[must_use]
    pub fn scorer(&self) -> Option<evara_constraints::IncrementalScorer<'_>> {
        evara_constraints::IncrementalScorer::of(
            &self.assembly.problem,
            &self.assignment(),
            &self.rules,
        )
    }

    /// The score the current arrangement carries.
    #[must_use]
    pub fn score(&self) -> Score {
        self.scorer().map_or(Score::PERFECT, |s| s.score())
    }
}

/// Every editing session this process has open.
///
/// Its own lock, like `GenerationSessions` and for the same reasons: a session holds a compiled
/// problem and **no** connection, it outlives the command that opened it, and reading its history
/// must not block behind a write somewhere else in the application.
///
/// A `BTreeMap` rather than a `HashMap`, because nothing in this workspace iterates a hash map.
#[derive(Debug, Default)]
pub struct EditSessions {
    sessions: Mutex<BTreeMap<Uuid, std::sync::Arc<Mutex<EditSession>>>>,
}

impl EditSessions {
    /// Records a new session and returns the handle.
    pub fn insert(&self, session: EditSession) -> std::sync::Arc<Mutex<EditSession>> {
        let id = session.session_id;
        let held = std::sync::Arc::new(Mutex::new(session));
        self.held().insert(id, std::sync::Arc::clone(&held));
        held
    }

    /// One session by identifier.
    #[must_use]
    pub fn get(&self, session_id: Uuid) -> Option<std::sync::Arc<Mutex<EditSession>>> {
        self.held().get(&session_id).map(std::sync::Arc::clone)
    }

    /// Closes one session. Returns whether there was one.
    pub fn close(&self, session_id: Uuid) -> bool {
        self.held().remove(&session_id).is_some()
    }

    /// How many sessions are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.held().len()
    }

    /// Whether any session is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Closes every session and forgets them all.
    ///
    /// Called when the project closes. A session names a school of the file being closed and
    /// holds an `Assembly` describing a database about to be shut; neither can survive it, and an
    /// undo stack whose rows are in another file is worse than no undo stack.
    pub fn clear(&self) {
        self.held().clear();
    }

    /// The map, through a poisoned lock if necessary.
    ///
    /// A poisoned lock here means a previous command panicked. Refusing to close the project
    /// because of it would trap the user in the state they are trying to leave, which is the
    /// argument `ImportSessions::clear` and `GenerationSessions::clear` both make.
    fn held(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, std::sync::Arc<Mutex<EditSession>>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
