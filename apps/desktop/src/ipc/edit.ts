/**
 * Manual timetable editing, across the IPC boundary.
 *
 * Every editing command is wrapped exactly once, here. Components call these functions and never
 * `invoke` directly, so the surface stays enumerable and mockable — and so the command *name* and
 * argument shape, the one part of the boundary generated types cannot check, is asserted in
 * exactly one place.
 *
 * ## Nothing here decides whether an edit is legal
 *
 * Not one line. {@link candidatesFor} returns a verdict per cell, already decided by the
 * constraint registry in Rust, with the rule's own sentence for every refusal. {@link validateMove}
 * answers for one cell. {@link applyMove} refuses the write if the verdict changed between the two.
 *
 * The things that **would** be scheduling rules all happen on the other side: which slots a lesson
 * occupies, whether a drop clashes, whether a period may hold a lesson, what a lock means, and what
 * undoing a move is. A second implementation of any of them here is the architecture bug
 * `CLAUDE.md` §2 forbids in the strongest terms it contains.
 *
 * ## Three refusals the screen must tell apart
 *
 * | Refusal | Means | What the screen offers |
 * | --- | --- | --- |
 * | `locked` | the placement is pinned in this timetable | **Unlock** |
 * | `conflict` | somebody changed *this* placement | reload |
 * | `arrangementChanged` | somebody changed *another* placement of this timetable | **reopen** |
 * | `inputsChanged` | a scheduling input moved | reopen |
 *
 * Nothing was written in any of them, and each message says so first.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  CandidateGrid,
  EditHistoryView,
  EditStateView,
  MoveVerdictView,
  ProposedMove,
  ScopeGrid,
  ScopeKind,
  TimetableSummary,
} from "@evara/domain";

import { describeSetupError, isSetupError } from "./setup";

// ---------------------------------------------------------------- opening

/** Opens one stored timetable for editing. Compiles its problem; writes nothing. */
export async function openForEditing(timetableId: string): Promise<EditStateView> {
  return invoke("edit_open", { timetableId });
}

/**
 * Closes a session, discarding its undo stack.
 *
 * `false` means there was no session — not a failure. One arriving after a campus switch has
 * nothing to act on.
 */
export async function closeEditing(sessionId: string): Promise<boolean> {
  return invoke("edit_close", { sessionId });
}

/** The session's current state, unchanged. */
export async function editState(sessionId: string): Promise<EditStateView> {
  return invoke("edit_state", { sessionId });
}

/**
 * One scope's laid-out grid, for the session's **current** arrangement.
 *
 * The same Rust layout a saved timetable's view uses, so an edited grid and a saved one cannot
 * look different because a different code path drew them.
 */
export async function editScope(
  sessionId: string,
  scopeKind: ScopeKind,
  scopeId: string | null,
): Promise<ScopeGrid> {
  return invoke("edit_scope", { sessionId, scopeKind, scopeId });
}

// ---------------------------------------------------------------- asking

/**
 * Every cell of one lesson's own bell schedule, with a verdict each.
 *
 * One call per pick-up rather than one per cell: a realistic panel is forty to sixty cells, and
 * sixty round trips per drag is not a design.
 */
export async function candidatesFor(
  sessionId: string,
  activityId: string,
  roomId: string | null,
): Promise<CandidateGrid> {
  return invoke("edit_candidates", { sessionId, activityId, roomId });
}

/** What one proposed placement would do. Writes nothing. */
export async function validateMove(
  sessionId: string,
  proposed: ProposedMove,
): Promise<MoveVerdictView> {
  return invoke("edit_validate", { sessionId, proposed });
}

// ---------------------------------------------------------------- writing

/**
 * Places or moves one occurrence.
 *
 * `expectedRev` is the revision the screen was holding for the entry, and is `null` only when the
 * occurrence is not yet placed — a creation carries no revision precondition.
 */
export async function applyMove(
  sessionId: string,
  proposed: ProposedMove,
  expectedRev: number | null,
): Promise<EditStateView> {
  return invoke("edit_apply", { sessionId, proposed, expectedRev });
}

/** Takes one occurrence off the timetable. Always permitted unless it is locked. */
export async function unscheduleLesson(
  sessionId: string,
  activityId: string,
  expectedRev: number,
): Promise<EditStateView> {
  return invoke("edit_unschedule", { sessionId, activityId, expectedRev });
}

/** Locks or unlocks one placement. */
export async function setLessonLock(
  sessionId: string,
  activityId: string,
  locked: boolean,
  expectedRev: number,
): Promise<EditStateView> {
  return invoke("edit_set_lock", { sessionId, activityId, locked, expectedRev });
}

/** Reverses the last applied edit. A new transaction, not a rollback — so it can be refused. */
export async function undoEdit(sessionId: string): Promise<EditStateView> {
  return invoke("edit_undo", { sessionId });
}

/** Reapplies the last undone edit. */
export async function redoEdit(sessionId: string): Promise<EditStateView> {
  return invoke("edit_redo", { sessionId });
}

/** What undo and redo would do next. */
export async function editHistory(sessionId: string): Promise<EditHistoryView> {
  return invoke("edit_history", { sessionId });
}

/**
 * Copies one timetable and its arrangement under a new name.
 *
 * The copy claims no generation run and no generation scoring, because it was not generated. This
 * is the action that makes in-place editing safe: a user who wants to keep the generated
 * arrangement duplicates first.
 */
export async function duplicateTimetable(
  id: string,
  expectedRev: number,
  name: string,
): Promise<TimetableSummary> {
  return invoke("timetable_duplicate", { id, expectedRev, name });
}

// ---------------------------------------------------------------- errors

/** An editing failure, in the words the backend chose. */
export function describeEditError(error: unknown): string {
  return describeSetupError(error);
}

/** Whether the edit was refused because the placement is locked. */
export function isLocked(error: unknown): boolean {
  return isSetupError(error) && error.kind === "locked";
}

/**
 * Whether another window changed this timetable while it was open.
 *
 * The one refusal whose remedy is **reopen** rather than retry: the session's compiled problem,
 * its validation and its undo stack were all computed from one snapshot, so they are stale
 * together.
 */
export function isArrangementChanged(error: unknown): boolean {
  return isSetupError(error) && error.kind === "arrangementChanged";
}

/** Whether the session is gone — closed, or the campus file switched under it. */
export function isSessionGone(error: unknown): boolean {
  return isSetupError(error) && error.kind === "notFound";
}

// ---------------------------------------------------------------- lookups

/**
 * The verdict for one cell of a candidate grid, by position.
 *
 * ## Why this is a lookup and not a scheduling rule
 *
 * Every cell already carries `allowed`, decided in Rust by the constraint registry. Finding the
 * one at `(day, period)` is an index into an answer, not a judgement — it computes no span, reads
 * no availability and compares no interval.
 */
export function verdictAt(
  grid: CandidateGrid | null,
  dayOrdinal: number,
  periodOrdinal: number,
): CandidateGrid["cells"][number] | null {
  if (grid === null) return null;
  return (
    grid.cells.find(
      (cell) => cell.dayOrdinal === dayOrdinal && cell.periodOrdinal === periodOrdinal,
    ) ?? null
  );
}
