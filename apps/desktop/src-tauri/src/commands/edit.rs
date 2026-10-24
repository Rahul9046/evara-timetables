//! Manual editing: open a timetable, ask what is legal, apply one edit, undo it.
//!
//! # What this module is, and is not
//!
//! It **orchestrates**. It holds no scheduling rule, decides no legality and lays out no grid.
//! Every answer it gives comes from somewhere that already owns the question:
//!
//! ```text
//!  open        evara_db::Scheduling::load          -> Assembly        (read-only)
//!              evara_db::Timetables::{entries, arrangement_stamp, pins}
//!              evara_constraints::Registry::compile -> the rules
//!  candidates  IncrementalScorer::probe             -> PlacementVerdict, per cell
//!  validate    IncrementalScorer::probe             -> PlacementVerdict
//!  apply       evara_db::Timetables::{place, move_entry, unschedule, set_lock}  one txn each
//!  explain     evara_report::explain_all            -> the rule's own sentence
//!  lay out     evara_report::projection::lay_out    -> the rowSpan decision
//! ```
//!
//! # The frontend computes no legality, and is given no way to
//!
//! [`edit_candidates`] answers *"which cells may this lesson go in?"* for a whole panel in one
//! call, and every cell comes back with a verdict and, where it is refused, the **rule's own**
//! explanation. There is nothing left for the view layer to work out: no span, no clash, no
//! availability, no `rowSpan`. That is the architecture bug `CLAUDE.md` §2 names and the one the
//! readiness audit flagged as this milestone's defining risk.
//!
//! # `probe` was already the move validator
//!
//! This is the single largest thing M9A inherits rather than writes.
//! [`IncrementalScorer::probe`](evara_constraints::IncrementalScorer::probe) **withdraws the
//! occurrence first**, then places it at the candidate, then restores — so moving a lesson one
//! period later does not report it as clashing with itself, which is the thing a hand-written
//! move validator gets wrong. Its `allowed` is also stricter than `SOLVER.md` §6's wording in
//! exactly the way an editor needs: a drop target that was already broken is not a legal drop
//! target.
//!
//! # Lock order
//!
//! Every handler here takes the **session** lock and then the **workspace** lock, in that order
//! and never the other way. No other module takes a session lock, so there is no cycle to
//! deadlock on, and the session's own `Mutex` is what serialises two commands of one session —
//! which they must be, or their stamps and their undo stacks would interleave.

// See `project.rs` for why this lint is inapplicable to a module of command handlers.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;

use evara_constraints::{PlacementVerdict, Registry};
use evara_core::scheduling::{
    ActivityIx, BellScheduleIx, Placement, Problem, RoomIx, Score, SectionFact, TimeslotIx,
};
use evara_db::scheduling::SchedulingSelection;
use evara_db::{AppliedEdit, EditGuard, EntryPlacement, TimetableEntry};
use evara_report::findings::{FindingCounts, FindingView};
use evara_report::projection::{
    LessonFlags, ProjectionInput, ScopeGrid, ScopeKind, TimetableProjection,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use super::edit_session::{Applied, Command, EditRefusal, EditSession};
use super::error::{SetupError, SetupResult};
use super::project::AppState;
use super::views::{self, TimetableSummary};
use super::workspace;

// =======================================================================================
// What crosses the boundary
// =======================================================================================

/// One occurrence's stored placement, as the editor addresses it.
///
/// Carries the entry's revision, which the interface echoes back on the write. That is ADR 0027's
/// shape and the same thing `timetable_revalidate` does: the revision is the *interface's*, not
/// one the handler reads for itself, because reading it here would be read-compare-write — *"the
/// same race with a longer window"*.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EditLesson {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// The stored row.
    pub entry_id: Uuid,
    /// The revision a move, an unschedule or a lock must carry.
    pub rev: i64,
    /// Whether a repair must preserve this placement.
    pub is_locked: bool,
    /// Whether a human placed it.
    pub is_manual: bool,
}

impl EditLesson {
    /// Flattens one stored entry.
    fn of(entry: &TimetableEntry) -> Self {
        Self {
            activity_id: entry.activity_id,
            entry_id: entry.id,
            rev: entry.rev,
            is_locked: entry.is_locked,
            is_manual: entry.is_manual,
        }
    }
}

/// One occurrence this timetable does not place.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EditUnplaced {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// What to call it.
    pub label: String,
    /// How many periods it needs.
    pub duration_periods: u16,
    /// Which occurrence of its class this is.
    pub split_index: u16,
}

/// A score, at all three levels.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScoreView {
    /// Hard penalties. Negative-going.
    pub hard: i64,
    /// Medium penalties.
    pub medium: i64,
    /// Soft penalties.
    pub soft: i64,
}

impl ScoreView {
    /// Flattens one score.
    const fn of(score: Score) -> Self {
        Self {
            hard: score.hard,
            medium: score.medium,
            soft: score.soft,
        }
    }
}

/// What undo and redo would do next.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EditHistoryView {
    /// What undo would reverse, or `null` when there is nothing to undo.
    pub undo: Option<String>,
    /// What redo would reapply.
    pub redo: Option<String>,
    /// How many edits this session has applied and not undone.
    pub depth: usize,
}

/// Everything one open timetable looks like, after zero or more edits.
///
/// Returned by [`edit_open`], by every write, and by [`edit_state`]. One shape rather than three,
/// so a screen cannot end up rendering a lesson list from one moment and a score from another.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EditStateView {
    /// Which session this is.
    pub session_id: Uuid,
    /// The timetable being edited, with the revision a rename or a duplicate must carry.
    pub timetable: TimetableSummary,
    /// Bumped on every applied edit, so a grid can tell whether it needs re-fetching.
    pub version: u64,
    /// The grid data: axes, lessons, scopes. The same projection a read-only view uses.
    pub projection: TimetableProjection,
    /// The stored rows behind the placed lessons, with their revisions.
    pub lessons: Vec<EditLesson>,
    /// The occurrences this timetable does not place.
    pub unplaced: Vec<EditUnplaced>,
    /// What the rules find about the arrangement **now**.
    pub findings: Vec<FindingView>,
    /// Those findings, counted by level.
    pub counts: FindingCounts,
    /// The current score.
    pub score: ScoreView,
    /// Whether the arrangement breaks no hard rule.
    ///
    /// Deliberately not `is_valid`: a usable arrangement may still be incomplete, which is a
    /// different question and is answered by `unplaced`.
    pub is_usable: bool,
    /// How many placements are locked.
    pub locked: usize,
    /// What undo and redo would do.
    pub history: EditHistoryView,
    /// Stored placements naming something the campus file no longer holds.
    ///
    /// Non-empty means the timetable is **readable but incomplete**: those rows are not in the
    /// grid, and the screen says so rather than showing less than was saved without comment.
    pub not_in_problem: Vec<Uuid>,
    /// Locked placements naming something the campus file no longer holds.
    pub pins_not_in_problem: Vec<Uuid>,
}

/// Where the caller proposes to put an occurrence.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProposedMove {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// The slot it would **start** in. A multi-period occurrence's later slots are derived.
    pub timeslot_id: Uuid,
    /// The room it would book, or `null` for none.
    pub room_id: Option<Uuid>,
}

/// What one proposed placement would do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MoveVerdictView {
    /// Whether the edit would be accepted.
    pub allowed: bool,
    /// Why it is refused before any rule runs, where it is.
    ///
    /// A locked placement is the case: a lock is a decision the user already recorded, so it is
    /// refused here with its own sentence rather than reported as a rule being broken.
    pub refusal: Option<String>,
    /// The hard violations the placement would be in, explained by the rules that found them.
    ///
    /// Empty exactly when the placement is allowed by the rules.
    pub blocking: Vec<FindingView>,
    /// The medium and soft violations it would newly introduce. **Never a reason to refuse.**
    pub warnings: Vec<FindingView>,
    /// How the score would change. Negative-going, so a smaller delta is worse.
    pub delta: ScoreView,
}

/// One cell of a candidate grid, with the verdict for putting a lesson there.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CandidateCell {
    /// The slot this cell is.
    pub timeslot_id: Uuid,
    /// Which cycle position it is on.
    pub day_ordinal: u16,
    /// Which period row it is in.
    pub period_ordinal: u16,
    /// Whether the lesson may be dropped here.
    pub allowed: bool,
    /// Whether this is where the lesson already is.
    pub is_current: bool,
    /// What refuses it, in the rules' own words. Empty when it is allowed.
    pub blocking: Vec<String>,
    /// What it would cost at the medium level, where anything.
    pub medium_delta: i64,
}

/// Every cell of one lesson's own bell schedule, with a verdict each.
///
/// # One call rather than sixty
///
/// Tinting legal drop targets means asking about every cell of a panel — forty to sixty for a
/// realistic school — and sixty IPC round trips per drag is not a design. One call builds one
/// scorer and probes each candidate against it, which is microseconds apiece.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CandidateGrid {
    /// Which occurrence this is about.
    pub activity_id: Uuid,
    /// The bell schedule its class draws slots from, where one resolves.
    pub schedule_id: Option<Uuid>,
    /// The room the verdicts were computed with.
    pub room_id: Option<Uuid>,
    /// Every room the occurrence's class may use, in the order the loader resolved them.
    ///
    /// From `RoomFact::is_candidate_for`, through the compiled `room_candidates` — the one
    /// implementation of the room candidate domain, called rather than restated.
    pub rooms: Vec<CandidateRoom>,
    /// Whether no room at all is a legal choice for this class.
    pub roomless_allowed: bool,
    /// One verdict per slot of that schedule.
    pub cells: Vec<CandidateCell>,
    /// Why there are no candidates at all, where that is the answer.
    pub unavailable: Option<String>,
}

/// One room an occurrence may be given.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CandidateRoom {
    /// Which room.
    pub room_id: Uuid,
    /// What it is called.
    pub name: String,
}

// =======================================================================================
// Opening and closing
// =======================================================================================

/// Opens one stored timetable for editing.
///
/// # Editing is in place, and the screen says so
///
/// This does not fork a version. `is_manual` is a per-entry flag on **this** timetable, and
/// ADR 0042 §5 keeps the `GENERATION` scoring immutable precisely so that re-scoring can never
/// misrepresent how a timetable was generated — a guarantee that is only needed *because* the
/// arrangement beneath it can change. A user who wants to keep the generated arrangement
/// duplicates first, which is [`timetable_duplicate`].
///
/// # Errors
///
/// - [`SetupError::NotFound`] if the project holds no such timetable, or its cycle has gone.
/// - [`SetupError::Invalid`] if the problem cannot be compiled. Unlike a read-only view, there is
///   no degraded editing mode: an editor with no problem could validate nothing, and offering one
///   would be offering to write placements nothing had checked.
#[tauri::command]
pub fn edit_open(state: State<'_, AppState>, timetable_id: Uuid) -> SetupResult<EditStateView> {
    let epoch = state.epoch.get();
    let session = {
        let mut ws = workspace(&state)?;

        let timetable = ws.timetables()?.get(timetable_id)?;
        let school_id = ws
            .time_model()?
            .cycle(timetable.cycle_id)?
            .map(|cycle| cycle.school_id)
            .ok_or_else(|| SetupError::NotFound {
                entity: "cycle".to_owned(),
                message: "This timetable names a cycle the campus file no longer holds, so it \
                          cannot be edited."
                    .to_owned(),
            })?;
        let selection = SchedulingSelection {
            school_id,
            cycle_id: timetable.cycle_id,
            term_id: timetable.term_id,
        };

        let assembly = ws.scheduling()?.load(&selection)?;
        let labels = ws.scheduling()?.labels(&selection)?;
        let stored = ws.constraints()?.stored_for(school_id)?;
        let rules =
            Registry::compile(&stored, selection.term_id).map_err(|error| SetupError::Invalid {
                message: format!("this school's constraint configuration cannot be read: {error}"),
            })?;
        let entries = ws.timetables()?.entries(timetable_id)?;
        let stamp = ws.timetables()?.arrangement_stamp(timetable_id)?;
        let pins = ws.timetables()?.pins(timetable_id, &assembly)?;

        EditSession::new(
            Uuid::now_v7(),
            timetable_id,
            selection,
            epoch,
            assembly,
            labels,
            rules,
            entries,
            stamp,
            pins.pinned,
            pins.not_in_problem,
        )
    };

    let held = state.edits.insert(session);
    let session = held.lock().map_err(|_| poisoned())?;
    state_of(&state, &session)
}

/// Closes one editing session, discarding its undo stack.
///
/// Returns whether there was one. Not an error if there was not: a close arriving after a project
/// switch has nothing to act on, and that is the honest answer.
#[tauri::command]
pub fn edit_close(state: State<'_, AppState>, session_id: Uuid) -> SetupResult<bool> {
    Ok(state.edits.close(session_id))
}

/// The session's current state, without changing anything.
#[tauri::command]
pub fn edit_state(state: State<'_, AppState>, session_id: Uuid) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let session = held.lock().map_err(|_| poisoned())?;
    state_of(&state, &session)
}

/// One scope's laid-out grid, for the session's **current** arrangement.
///
/// The layout — which cells hold what, and what `rowSpan` each may use — is Rust's, through the
/// same `evara-report` call the read-only view uses. So an edited grid and a saved one cannot
/// look different because a different code path drew them.
#[tauri::command]
pub fn edit_scope(
    state: State<'_, AppState>,
    session_id: Uuid,
    scope_kind: ScopeKind,
    scope_id: Option<Uuid>,
) -> SetupResult<ScopeGrid> {
    let held = live(&state, session_id)?;
    let session = held.lock().map_err(|_| poisoned())?;
    let projection = project_session(&session);
    views::lay_out_scope(&projection, session.labels(), scope_kind, scope_id)
}

// =======================================================================================
// Asking what is legal
// =======================================================================================

/// Every cell of one lesson's own bell schedule, with a verdict each.
///
/// # Why the non-teaching and the unavailable cells are **included**
///
/// A pre-filter would leave them out, which is right for a generator — it is choosing what to
/// try — and wrong for an editor, which is answering a user who can see the cell. So every slot
/// of the schedule is probed and the refusals come back with the rule's own sentence: *"Break is
/// not a teaching period"*, *"A. Okafor is unavailable on Day 2"*. `Problem::timeslots` keeps the
/// non-teaching slots for exactly this (ADR 0033 §4).
///
/// # Errors
///
/// [`SetupError::Invalid`] if the occurrence is not in the compiled problem.
#[tauri::command]
pub fn edit_candidates(
    state: State<'_, AppState>,
    session_id: Uuid,
    activity_id: Uuid,
    room_id: Option<Uuid>,
) -> SetupResult<CandidateGrid> {
    let held = live(&state, session_id)?;
    let session = held.lock().map_err(|_| poisoned())?;
    let problem = session.problem();

    let activity = session
        .assembly()
        .index
        .activity_index(activity_id)
        .ok_or_else(|| SetupError::Invalid {
            message: "That lesson is not part of this timetable's problem.".to_owned(),
        })?;
    let fact = problem
        .activities
        .get(activity.get())
        .ok_or_else(|| SetupError::Invalid {
            message: "That lesson is not part of this timetable's problem.".to_owned(),
        })?;
    let section = problem
        .sections
        .get(fact.section.get())
        .ok_or_else(|| SetupError::Invalid {
            message: "That lesson's class is not part of this timetable's problem.".to_owned(),
        })?;

    let rooms = candidate_rooms(problem, section);
    let roomless_allowed = section.required_room_type.is_none();

    // Which room the verdicts are about: the caller's choice, or where the lesson already is.
    let chosen = room_id.or_else(|| session.entry_of(activity_id).and_then(|e| e.room_id));
    let room = chosen.and_then(|id| session.assembly().index.room_index(id));
    if chosen.is_some() && room.is_none() {
        return Err(SetupError::Invalid {
            message: "That room is not one this campus file holds for this school.".to_owned(),
        });
    }

    // ADR 0033 §3: anything other than a single resolved bell schedule is a pre-flight refusal,
    // and an occurrence in that state has no value range at all rather than a small one.
    let Some(schedule) = section.bell_schedule.schedule() else {
        return Ok(CandidateGrid {
            activity_id,
            schedule_id: None,
            room_id: chosen,
            rooms,
            roomless_allowed,
            cells: Vec::new(),
            unavailable: Some(
                "This class does not resolve to a single bell schedule, so Evara cannot say \
                 where its lessons may go. Give its campus one bell schedule, or name the \
                 schedule on the class."
                    .to_owned(),
            ),
        });
    };

    let cells = probe_schedule(&session, activity, activity_id, schedule, room)?;

    Ok(CandidateGrid {
        activity_id,
        schedule_id: problem
            .context
            .bell_schedules
            .get(schedule.get())
            .map(|s| s.id),
        room_id: chosen,
        rooms,
        roomless_allowed,
        cells,
        unavailable: None,
    })
}

/// Every room one class may use, named.
///
/// `room_candidates` is the loader's record of `RoomFact::is_candidate_for`, which is the **one**
/// implementation of the room candidate domain. Read, never restated.
fn candidate_rooms(problem: &Problem, section: &SectionFact) -> Vec<CandidateRoom> {
    section
        .room_candidates
        .iter()
        .filter_map(|room| {
            let fact = problem.rooms.get(room.get())?;
            Some(CandidateRoom {
                room_id: fact.id,
                name: fact.name.clone(),
            })
        })
        .collect()
}

/// Probes every slot of one bell schedule for one occurrence.
///
/// # One scorer, every cell
///
/// The scorer is built once and probed per candidate, which is what makes a whole panel one call
/// rather than sixty. `probe` commits nothing and is observably side-effect free, so the order the
/// cells are probed in cannot change any of their answers.
///
/// # Errors
///
/// [`SetupError::Invalid`] if the stored arrangement cannot be read against the problem. A scorer
/// over an unreadable arrangement would maintain a number about nothing, so nothing is validated
/// against one.
fn probe_schedule(
    session: &EditSession,
    activity: ActivityIx,
    activity_id: Uuid,
    schedule: BellScheduleIx,
    room: Option<RoomIx>,
) -> SetupResult<Vec<CandidateCell>> {
    let problem = session.problem();
    let mut scorer = session.scorer().ok_or_else(|| SetupError::Invalid {
        message: UNREADABLE.to_owned(),
    })?;

    let current = session.entry_of(activity_id).map(|entry| entry.timeslot_id);
    // A locked lesson has no legal target but its own place, and the registry says so too:
    // `activity-fixed` refuses every other cell. The flag is read here as well so the grid offers
    // no drop target at all rather than one that fails on release.
    let locked = session.is_locked(activity_id);
    let mut cells = Vec::new();

    for (at, slot) in problem.timeslots.iter().enumerate() {
        if slot.bell_schedule != schedule {
            continue;
        }
        let timeslot = TimeslotIx::new(u32::try_from(at).unwrap_or(u32::MAX));
        let verdict = scorer.probe(activity, Placement { timeslot, room });
        cells.push(CandidateCell {
            timeslot_id: slot.id,
            day_ordinal: problem
                .context
                .cycle_days
                .get(slot.cycle_day.get())
                .map_or(0, |day| day.ordinal),
            period_ordinal: problem
                .context
                .periods
                .get(slot.period.get())
                .map_or(0, |period| period.ordinal),
            allowed: verdict.allowed && !locked,
            is_current: current == Some(slot.id),
            blocking: reasons(problem, &verdict),
            medium_delta: verdict.delta.medium,
        });
    }
    Ok(cells)
}

/// What one proposed placement would do. **Writes nothing.**
///
/// Takes any slot, not only one of the lesson's own schedule, so a drop onto another panel is
/// answered rather than ignored — and the answer is `duration-contiguous`'s, which is the rule
/// that owns *"this lesson cannot sit on a schedule its class does not use"*.
#[tauri::command]
pub fn edit_validate(
    state: State<'_, AppState>,
    session_id: Uuid,
    proposed: ProposedMove,
) -> SetupResult<MoveVerdictView> {
    let held = live(&state, session_id)?;
    let session = held.lock().map_err(|_| poisoned())?;
    verdict_for(&session, proposed)
}

// =======================================================================================
// Applying one edit
// =======================================================================================

/// Places or moves one occurrence. **The write.**
///
/// # One command, one transaction
///
/// A drag that lands is one command, and the repository applies it in one transaction with all
/// three guards as predicates on the write. A move is an `UPDATE` of the existing row, so the
/// placement keeps its identity, its lock and its creation time, and the change feed records one
/// entry for one user action.
///
/// `expected_rev` is the revision the interface was holding for the entry, and is required when
/// the occurrence is already placed. For a placement there is no row to be stale about, so it is
/// `null` — a creation carries no revision precondition (`CLAUDE.md` §3).
///
/// # Errors
///
/// - [`SetupError::Locked`] if the placement is locked. Refused before any rule runs.
/// - [`SetupError::Invalid`] if the move would break a hard rule, carrying the rules' own
///   sentences.
/// - [`SetupError::Conflict`] if the entry moved since `expected_rev`.
/// - [`SetupError::ArrangementChanged`] if another window changed this timetable.
/// - [`SetupError::InputsChanged`] if a scheduling input moved since the session opened.
#[tauri::command]
pub fn edit_apply(
    state: State<'_, AppState>,
    session_id: Uuid,
    proposed: ProposedMove,
    expected_rev: Option<i64>,
) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let mut session = held.lock().map_err(|_| poisoned())?;
    require_epoch(&state, &session)?;

    // The verdict first, so a refusal carries the rule's own words and nothing is attempted.
    let verdict = verdict_for(&session, proposed)?;
    if let Some(refusal) = verdict.refusal {
        return Err(SetupError::Locked { message: refusal });
    }
    if !verdict.allowed {
        return Err(SetupError::Invalid {
            message: refusal_sentence(&verdict.blocking),
        });
    }

    let to = EntryPlacement {
        timeslot_id: proposed.timeslot_id,
        room_id: proposed.room_id,
    };
    let command = match session.stored_placement_of(proposed.activity_id) {
        Some(from) => Command::Move {
            activity: proposed.activity_id,
            from,
            to,
        },
        None => Command::Place {
            activity: proposed.activity_id,
            to,
        },
    };

    let label = describe(&session, command);
    let applied = run(&state, &session, command, expected_rev)?;
    record(&mut session, command, &applied, label, Push::Forward);
    state_of(&state, &session)
}

/// Takes one occurrence off the timetable.
///
/// Always permitted when it is not locked, and it always costs: `activity-scheduled` is MEDIUM,
/// so an unplaced lesson makes a timetable **incomplete** rather than unusable. A rule that
/// refused this would make the obvious first step of a manual rearrangement impossible.
#[tauri::command]
pub fn edit_unschedule(
    state: State<'_, AppState>,
    session_id: Uuid,
    activity_id: Uuid,
    expected_rev: i64,
) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let mut session = held.lock().map_err(|_| poisoned())?;
    require_epoch(&state, &session)?;

    if session.is_locked(activity_id) {
        return Err(SetupError::Locked {
            message: locked_sentence(&session, activity_id),
        });
    }
    let from = session
        .stored_placement_of(activity_id)
        .ok_or_else(|| refused(&EditRefusal::NotPlaced))?;

    let command = Command::Unschedule {
        activity: activity_id,
        from,
    };
    let label = describe(&session, command);
    let applied = run(&state, &session, command, Some(expected_rev))?;
    record(&mut session, command, &applied, label, Push::Forward);
    state_of(&state, &session)
}

/// Locks or unlocks one placement.
///
/// A lock means *"in this timetable, this placement is fixed"*. It reaches the constraint engine
/// as a `Problem::pinned` entry judged by `activity-fixed`, so the editor and a later repair
/// cannot disagree about what it means — and it is recomputed on the session the moment this
/// returns, so unlocking a lesson makes its drop targets legal immediately.
#[tauri::command]
pub fn edit_set_lock(
    state: State<'_, AppState>,
    session_id: Uuid,
    activity_id: Uuid,
    locked: bool,
    expected_rev: i64,
) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let mut session = held.lock().map_err(|_| poisoned())?;
    require_epoch(&state, &session)?;

    if session.entry_of(activity_id).is_none() {
        return Err(refused(&EditRefusal::NotPlaced));
    }
    let command = Command::SetLock {
        activity: activity_id,
        locked,
    };
    let label = describe(&session, command);
    let applied = run(&state, &session, command, Some(expected_rev))?;
    record(&mut session, command, &applied, label, Push::Forward);
    state_of(&state, &session)
}

/// Reverses the last applied edit.
///
/// # An undo is a new transaction, not a rollback
///
/// The inverse command is applied as its own write, so the change feed records **both** the move
/// and its reversal — which is what an audit trail means. A rollback would leave no trace that
/// the user had done and undone something.
///
/// It can therefore fail, and failing is the honest answer: on a revision conflict, on moved
/// inputs, or on another window's edit. When it does, **both stacks are cleared** — every entry
/// below the one that failed was computed against a state that no longer holds.
#[tauri::command]
pub fn edit_undo(state: State<'_, AppState>, session_id: Uuid) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let mut session = held.lock().map_err(|_| poisoned())?;
    require_epoch(&state, &session)?;
    step(&state, &mut session, Push::Undo)
}

/// Reapplies the last undone edit.
#[tauri::command]
pub fn edit_redo(state: State<'_, AppState>, session_id: Uuid) -> SetupResult<EditStateView> {
    let held = live(&state, session_id)?;
    let mut session = held.lock().map_err(|_| poisoned())?;
    require_epoch(&state, &session)?;
    step(&state, &mut session, Push::Redo)
}

/// What undo and redo would do next.
#[tauri::command]
pub fn edit_history(state: State<'_, AppState>, session_id: Uuid) -> SetupResult<EditHistoryView> {
    let held = live(&state, session_id)?;
    let session = held.lock().map_err(|_| poisoned())?;
    let history = session.history();
    Ok(EditHistoryView {
        undo: history.undo,
        redo: history.redo,
        depth: history.depth,
    })
}

/// Copies one timetable and its arrangement, claiming none of its provenance.
///
/// The affordance that makes in-place editing safe: a user who wants to keep the generated
/// arrangement duplicates before editing. The copy carries no `solve_run` and no evaluation,
/// because it was not generated and a cloned `GENERATION` row would be a claim about a run that
/// did not produce it.
#[tauri::command]
pub fn timetable_duplicate(
    state: State<'_, AppState>,
    id: Uuid,
    expected_rev: i64,
    name: String,
) -> SetupResult<TimetableSummary> {
    let mut ws = workspace(&state)?;
    let copy = ws.timetables()?.duplicate(id, expected_rev, name.trim())?;
    Ok(TimetableSummary::of(&copy.timetable))
}

// =======================================================================================
// Helpers
// =======================================================================================

/// Which stack an applied command goes onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Push {
    /// A new edit: onto the undo stack, clearing redo.
    Forward,
    /// An undo: the inverse was applied, so it goes onto the redo stack.
    Undo,
    /// A redo: back onto the undo stack.
    Redo,
}

/// Applies one undo or redo step.
fn step(
    state: &State<'_, AppState>,
    session: &mut EditSession,
    direction: Push,
) -> SetupResult<EditStateView> {
    let taken = match direction {
        Push::Undo => session.take_undo(),
        Push::Redo => session.take_redo(),
        Push::Forward => None,
    };
    let Some(entry) = taken else {
        return Err(refused(&EditRefusal::NothingToDo));
    };

    let command = match direction {
        // Undoing applies the inverse; redoing applies the command again. The stack entry's own
        // `produced` revision is the precondition in both directions, because that is the
        // revision this session's last write left the row at.
        Push::Undo => entry.command.inverse(),
        _ => entry.command,
    };
    let expected = entry.produced.map(|(_, rev)| rev);
    let label = entry.label.clone();

    match run(state, session, command, expected) {
        Ok(applied) => {
            let next = Applied {
                command,
                produced: applied.entry.as_ref().map(|entry| (entry.id, entry.rev)),
                label,
            };
            session.observe(
                command.activity(),
                applied.entry.clone(),
                applied.stamp.clone(),
            );
            match direction {
                Push::Undo => session.undone(next),
                _ => session.redone(next),
            }
            state_of(state, session)
        }
        Err(error) => {
            // Everything below this entry was computed against a state that no longer holds.
            session.forget_history();
            Err(error)
        }
    }
}

/// Runs one command against the database, under the workspace lock.
fn run(
    state: &State<'_, AppState>,
    session: &EditSession,
    command: Command,
    expected_rev: Option<i64>,
) -> SetupResult<AppliedEdit> {
    let guard = EditGuard {
        watermark: session.assembly().watermark,
        stamp: session.stamp(),
    };
    let entry_id = session.entry_of(command.activity()).map(|entry| entry.id);
    let needs_rev = |what: &str| SetupError::Invalid {
        message: format!(
            "Evara cannot {what} this lesson without the revision it was read at. Reopen the \
             timetable and try again."
        ),
    };

    let mut ws = workspace(state)?;
    let mut timetables = ws.timetables()?;
    let applied = match command {
        Command::Place { activity, to } => {
            timetables.place(session.timetable_id, activity, to, guard)?
        }
        Command::Move { to, .. } => {
            let id = entry_id.ok_or_else(|| refused(&EditRefusal::NotPlaced))?;
            let rev = expected_rev.ok_or_else(|| needs_rev("move"))?;
            timetables.move_entry(id, rev, to, guard)?
        }
        Command::Unschedule { .. } => {
            let id = entry_id.ok_or_else(|| refused(&EditRefusal::NotPlaced))?;
            let rev = expected_rev.ok_or_else(|| needs_rev("unschedule"))?;
            timetables.unschedule(id, rev, guard)?
        }
        Command::SetLock { locked, .. } => {
            let id = entry_id.ok_or_else(|| refused(&EditRefusal::NotPlaced))?;
            let rev = expected_rev.ok_or_else(|| needs_rev("lock"))?;
            timetables.set_lock(id, rev, locked, guard)?
        }
    };
    Ok(applied)
}

/// Records what a forward command did on the session.
fn record(
    session: &mut EditSession,
    command: Command,
    applied: &AppliedEdit,
    label: String,
    direction: Push,
) {
    session.observe(
        command.activity(),
        applied.entry.clone(),
        applied.stamp.clone(),
    );
    let entry = Applied {
        command,
        produced: applied.entry.as_ref().map(|entry| (entry.id, entry.rev)),
        label,
    };
    match direction {
        Push::Forward => session.did(entry),
        Push::Undo => session.undone(entry),
        Push::Redo => session.redone(entry),
    }
}

/// The verdict for one proposed placement, lock refusal included.
fn verdict_for(session: &EditSession, proposed: ProposedMove) -> SetupResult<MoveVerdictView> {
    let problem = session.problem();
    let activity = session
        .assembly()
        .index
        .activity_index(proposed.activity_id)
        .ok_or_else(|| refused(&EditRefusal::NotInProblem))?;
    let timeslot = session
        .assembly()
        .index
        .timeslot_index(proposed.timeslot_id)
        .ok_or_else(|| SetupError::Invalid {
            message: "That period is not part of this timetable's grid.".to_owned(),
        })?;
    let room =
        match proposed.room_id {
            None => None,
            Some(id) => Some(session.assembly().index.room_index(id).ok_or_else(|| {
                SetupError::Invalid {
                    message: "That room is not one this campus file holds for this school."
                        .to_owned(),
                }
            })?),
        };

    if session.is_locked(proposed.activity_id) {
        return Ok(MoveVerdictView {
            allowed: false,
            refusal: Some(locked_sentence(session, proposed.activity_id)),
            blocking: Vec::new(),
            warnings: Vec::new(),
            delta: ScoreView::of(Score::PERFECT),
        });
    }

    let mut scorer = session.scorer().ok_or_else(|| SetupError::Invalid {
        message: UNREADABLE.to_owned(),
    })?;
    let verdict = scorer.probe(activity, Placement { timeslot, room });

    Ok(MoveVerdictView {
        allowed: verdict.allowed,
        refusal: None,
        blocking: evara_report::explain_all(problem, &verdict.blocking),
        warnings: evara_report::explain_all(problem, &verdict.warnings),
        delta: ScoreView::of(verdict.delta),
    })
}

/// The rules' own sentences for a refused candidate.
fn reasons(problem: &Problem, verdict: &PlacementVerdict) -> Vec<String> {
    evara_report::explain_all(problem, &verdict.blocking)
        .into_iter()
        .filter_map(|finding| finding.message)
        .collect()
}

/// One sentence for a refused drop, from the rules that refused it.
fn refusal_sentence(blocking: &[FindingView]) -> String {
    if blocking.is_empty() {
        return "That move cannot be made.".to_owned();
    }
    let said: Vec<String> = blocking
        .iter()
        .filter_map(|finding| finding.message.clone())
        .collect();
    if said.is_empty() {
        // Every rule implements `explain`, so an empty list here means the findings came back at
        // the stored fidelity — which cannot happen for a live probe. Reported rather than
        // papered over with a cheerful sentence.
        return "That move cannot be made, and Evara could not say why. Please report this."
            .to_owned();
    }
    said.join(" ")
}

/// What to say about a locked placement.
fn locked_sentence(session: &EditSession, activity_id: Uuid) -> String {
    format!(
        "{} is locked in this timetable, so it cannot be moved or unscheduled. Unlock it first.",
        name_of(session, activity_id)
    )
}

/// What to call one occurrence.
fn name_of(session: &EditSession, activity_id: Uuid) -> String {
    let problem = session.problem();
    let Some(activity) = session.assembly().index.activity_index(activity_id) else {
        return "That lesson".to_owned();
    };
    let Some(fact) = problem.activities.get(activity.get()) else {
        return "That lesson".to_owned();
    };
    let section = problem.sections.get(fact.section.get());
    let label = section.map_or_else(
        || "an unnamed class".to_owned(),
        |section| {
            session
                .labels()
                .section(section.id)
                .map_or_else(|| section.name.clone(), |label| label.short.clone())
        },
    );
    match &fact.name {
        Some(name) => format!("{label} ({name})"),
        None => label,
    }
}

/// What to call one command in an "Undo …" control.
fn describe(session: &EditSession, command: Command) -> String {
    let what = name_of(session, command.activity());
    match command {
        Command::Place { to, .. } => format!("place {what} at {}", where_of(session, to)),
        Command::Move { to, .. } => format!("move {what} to {}", where_of(session, to)),
        Command::Unschedule { .. } => format!("unschedule {what}"),
        Command::SetLock { locked, .. } => {
            if locked {
                format!("lock {what}")
            } else {
                format!("unlock {what}")
            }
        }
    }
}

/// Where a placement is, in the words a school wrote.
///
/// Day and period, never a weekday: `cycle_day.ordinal` is a day's only scheduling identity and
/// `label` is what a reader recognises on their own grid.
fn where_of(session: &EditSession, at: EntryPlacement) -> String {
    let problem = session.problem();
    let Some(timeslot) = session.assembly().index.timeslot_index(at.timeslot_id) else {
        return "another period".to_owned();
    };
    let Some(slot) = problem.timeslots.get(timeslot.get()) else {
        return "another period".to_owned();
    };
    let day = problem
        .context
        .cycle_days
        .get(slot.cycle_day.get())
        .map_or_else(|| "a day".to_owned(), |day| day.label.clone());
    let period = problem
        .context
        .periods
        .get(slot.period.get())
        .map_or_else(|| "a period".to_owned(), |period| period.label.clone());
    format!("{day} {period}")
}

/// Projects the session's current arrangement.
fn project_session(session: &EditSession) -> TimetableProjection {
    let flags: BTreeMap<Uuid, LessonFlags> = session
        .entries()
        .iter()
        .map(|(activity, entry)| {
            (
                *activity,
                LessonFlags {
                    is_locked: entry.is_locked,
                    is_manual: entry.is_manual,
                },
            )
        })
        .collect();
    evara_report::projection::project(ProjectionInput {
        problem: session.problem(),
        assignment: &session.assignment(),
        labels: session.labels(),
        flags: &flags,
    })
}

/// The whole state of one session, as the interface renders it.
fn state_of(state: &State<'_, AppState>, session: &EditSession) -> SetupResult<EditStateView> {
    let projection = project_session(session);
    let assignment = session.assignment();
    let report = evara_constraints::evaluate(session.problem(), &assignment, session.rules());
    let findings = evara_report::explain_all(session.problem(), &report.violations);

    // The timetable row, for the revision a rename, a delete or a duplicate must carry. Read
    // rather than cached: a rename in another window must not make the button in this one stale.
    let timetable = {
        let mut ws = workspace(state)?;
        ws.timetables()?.get(session.timetable_id)?
    };

    let placed: Vec<Uuid> = session.entries().keys().copied().collect();
    let unplaced = unplaced_of(session, &placed);
    let history = session.history();

    Ok(EditStateView {
        session_id: session.session_id,
        timetable: TimetableSummary::of(&timetable),
        version: session.version,
        lessons: session.entries().values().map(EditLesson::of).collect(),
        unplaced,
        counts: FindingCounts::of(&findings),
        findings,
        score: ScoreView::of(report.score),
        is_usable: report.is_usable(),
        locked: session
            .entries()
            .values()
            .filter(|entry| entry.is_locked)
            .count(),
        history: EditHistoryView {
            undo: history.undo,
            redo: history.redo,
            depth: history.depth,
        },
        not_in_problem: session.not_in_problem.clone(),
        pins_not_in_problem: session.pins_not_in_problem.clone(),
        projection,
    })
}

/// Every occurrence of the problem this timetable does not place.
///
/// In the problem's own order, which is what makes two renders of one timetable agree. An
/// unplaced occurrence is an ordinary state and not an error: `activity-scheduled` is MEDIUM.
fn unplaced_of(session: &EditSession, placed: &[Uuid]) -> Vec<EditUnplaced> {
    session
        .problem()
        .activities
        .iter()
        .filter(|fact| !placed.contains(&fact.id))
        .map(|fact| EditUnplaced {
            activity_id: fact.id,
            label: name_of(session, fact.id),
            duration_periods: u16::try_from(fact.duration.as_usize()).unwrap_or(u16::MAX),
            split_index: fact.split_index,
        })
        .collect()
}

/// The session for one identifier, if there is one.
fn live(
    state: &State<'_, AppState>,
    session_id: Uuid,
) -> SetupResult<std::sync::Arc<std::sync::Mutex<EditSession>>> {
    state
        .edits
        .get(session_id)
        .ok_or_else(|| SetupError::NotFound {
            entity: "timetable".to_owned(),
            message: "This editing session is no longer open. It may have been closed, or the \
                  campus file may have been switched since. Open the timetable again."
                .to_owned(),
        })
}

/// Refuses a session that belongs to a campus file that is no longer open.
///
/// A session holds an `Assembly` describing one database. Writing it into another would offer one
/// file's arrangement to another file's writer, which the repository would refuse — but only
/// after a transaction had opened and only if the identifiers happened not to collide. Refusing
/// here is the earlier and clearer of the two.
fn require_epoch(state: &State<'_, AppState>, session: &EditSession) -> SetupResult<()> {
    if session.epoch == state.epoch.get() {
        return Ok(());
    }
    Err(SetupError::NotFound {
        entity: "timetable".to_owned(),
        message: "This timetable was opened from a campus file that is no longer open, so \
                  nothing was changed. Open it again."
            .to_owned(),
    })
}

/// A refusal that happens before any rule runs.
fn refused(refusal: &EditRefusal) -> SetupError {
    match refusal {
        EditRefusal::Locked { label } => SetupError::Locked {
            message: format!(
                "{label} is locked in this timetable, so it cannot be moved or unscheduled. \
                 Unlock it first."
            ),
        },
        EditRefusal::NotInProblem => SetupError::Invalid {
            message: "That lesson is not part of this timetable's problem.".to_owned(),
        },
        EditRefusal::AlreadyPlaced => SetupError::Invalid {
            message: "That lesson is already placed in this timetable.".to_owned(),
        },
        EditRefusal::NotPlaced => SetupError::Invalid {
            message: "That lesson is not placed in this timetable, so there is nothing to move."
                .to_owned(),
        },
        EditRefusal::NothingToDo => SetupError::Invalid {
            message: "There is nothing to undo.".to_owned(),
        },
    }
}

/// What to say when the stored arrangement cannot be read against the compiled problem.
///
/// One sentence rather than two, because the candidate pass and the single-cell validator are
/// refusing the same thing and a user who met both wordings would reasonably think they were
/// different problems.
const UNREADABLE: &str = "This timetable's stored placements cannot be read against the campus \
                          file as it now stands, so nothing can be validated. Reopen it.";

/// A poisoned session lock.
fn poisoned() -> SetupError {
    SetupError::Failed {
        message: "Evara is in an inconsistent state. Please restart the application.".to_owned(),
    }
}
