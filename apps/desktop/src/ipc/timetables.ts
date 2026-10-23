/**
 * Generating, viewing and managing timetables, across the IPC boundary.
 *
 * Every timetable command is wrapped exactly once, here. Components call these functions and
 * never `invoke` directly, so the surface stays enumerable and mockable — and so the command
 * *name* and argument shape, the one part of the boundary generated types cannot check, is
 * asserted in exactly one place and covered by tests.
 *
 * ## Nothing here decides anything
 *
 * Every type comes from `@evara/domain`, generated from the Rust. This file declares no
 * shape of its own, computes no span, and judges no placement. The one piece of logic it
 * holds is {@link indexByScope}, which is a **lookup** — grouping already-resolved lessons
 * by an identifier they already carry — and which is documented there for why that is not a
 * scheduling rule.
 *
 * ## The two events
 *
 * `solver://progress` and `solver://finished` both carry `runId` **and** `epoch`.
 * {@link onSolverProgress} filters on the run so a session the user has moved on from cannot
 * drive the screen they are looking at now.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DeletedView,
  FinishedEvent,
  GenerateResult,
  GenerateSettings,
  GenerateStarted,
  GenerateStatus,
  ProgressEvent,
  ProjectedLesson,
  ProjectionScope,
  SavedView,
  ScopeGrid,
  ScopeKind,
  TimetableSummary,
  TimetableView,
  ValidationView,
} from "@evara/domain";

import { describeSetupError, isSetupError } from "./setup";

/** Which school, cycle and term a generation is about. All three are named; none defaults. */
export type Selection = {
  schoolId: string;
  cycleId: string;
  termId: string | null;
};

// ---------------------------------------------------------------- validation

/** Pre-solve validation for one selection. Writes nothing, so it is safe on every render. */
export async function validateInputs(selection: Selection): Promise<ValidationView> {
  return invoke("timetable_validate", { selection });
}

// ---------------------------------------------------------------- generation

/** Starts a generation on its own thread. Returns as soon as the thread is running. */
export async function startGeneration(
  selection: Selection,
  settings?: GenerateSettings,
): Promise<GenerateStarted> {
  return invoke("timetable_generate_start", { selection, settings: settings ?? null });
}

/** How a run is doing. Current even between coalesced progress events. */
export async function generationStatus(runId: string): Promise<GenerateStatus> {
  return invoke("timetable_generate_status", { runId });
}

/** The finished run and its preview, projected by the same Rust code a saved view uses. */
export async function generationResult(runId: string): Promise<GenerateResult> {
  return invoke("timetable_generate_result", { runId });
}

/**
 * Asks a run to stop.
 *
 * `false` means there was nothing still running — not a failure. A run that had already
 * finished cannot be stopped, and saying otherwise would tell the user their cancel did
 * something it did not.
 */
export async function cancelGeneration(runId: string): Promise<boolean> {
  return invoke("timetable_generate_cancel", { runId });
}

/** Abandons a run, cancelling it if it is still going. */
export async function discardGeneration(runId: string): Promise<boolean> {
  return invoke("timetable_generate_discard", { runId });
}

/**
 * One scope's laid-out grid, for a run still in memory.
 *
 * The layout — which cells hold what, and what `rowSpan` each may use — is decided in Rust.
 * This is a second call rather than part of the result because a large school has hundreds of
 * scopes and only one is on screen; see the Rust handler for why neither half of that is
 * negotiable.
 */
export async function generationScope(
  runId: string,
  scopeKind: ScopeKind,
  scopeId: string | null,
): Promise<ScopeGrid> {
  return invoke("timetable_generate_scope", { runId, scopeKind, scopeId });
}

/** One scope's laid-out grid, for a stored timetable. */
export async function timetableScope(
  id: string,
  scopeKind: ScopeKind,
  scopeId: string | null,
): Promise<ScopeGrid> {
  return invoke("timetable_scope", { id, scopeKind, scopeId });
}

/**
 * Saves a generated timetable. **The one call here that writes.**
 *
 * Always inserts: several timetables coexist for one cycle and term, and that coexistence is
 * the versioning. There is no overwrite, so no caller can ask for one.
 */
export async function saveGeneration(runId: string, name: string): Promise<SavedView> {
  return invoke("timetable_save", { runId, name });
}

// ---------------------------------------------------------------- saved timetables

/** Every timetable of one school, newest first. */
export async function listTimetables(schoolId: string): Promise<TimetableSummary[]> {
  return invoke("timetable_list", { schoolId });
}

/** Every timetable over one cycle and term, newest first. */
export async function listTimetablesForCycle(
  cycleId: string,
  termId: string | null,
): Promise<TimetableSummary[]> {
  return invoke("timetable_list_for_cycle", { cycleId, termId });
}

/**
 * One stored timetable, with its grid where one can be built.
 *
 * A `null` `projection` with an `unreadable` sentence is an ordinary answer, not a failure:
 * the timetable is still worth opening, and the screen says why there is no grid.
 */
export async function openTimetable(id: string): Promise<TimetableView> {
  return invoke("timetable_open", { id });
}

/** Renames one timetable, carrying the revision the screen was holding. */
export async function renameTimetable(
  id: string,
  expectedRev: number,
  name: string,
): Promise<TimetableSummary> {
  return invoke("timetable_rename", { id, expectedRev, name });
}

/** Deletes one timetable and everything belonging to it, reporting what it removed. */
export async function deleteTimetable(id: string, expectedRev: number): Promise<DeletedView> {
  return invoke("timetable_delete", { id, expectedRev });
}

/**
 * Re-scores one stored timetable and replaces its current findings.
 *
 * Carries `expectedRev` because the write **consumes** the timetable's revision: its current
 * findings are part of what it says about itself, so recording new ones moves it.
 */
export async function revalidateTimetable(id: string, expectedRev: number) {
  return invoke("timetable_revalidate", { id, expectedRev });
}

// ---------------------------------------------------------------- events

/**
 * Subscribes to one run's progress.
 *
 * Filters on `runId`, which is what stops a session the user has moved on from — a cancelled
 * run, a discarded one, one begun before the campus file was switched — driving the screen
 * in front of them. The backend sends every event to every window; deciding which are *this
 * screen's* is the screen's job and is done here, once.
 */
export async function onSolverProgress(
  runId: string,
  handler: (event: ProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProgressEvent>("solver://progress", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

/** Subscribes to one run's completion. Filters on `runId`, as progress does. */
export async function onSolverFinished(
  runId: string,
  handler: (event: FinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<FinishedEvent>("solver://finished", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

// ---------------------------------------------------------------- errors

/** A timetable failure, in the words the backend chose. */
export function describeTimetableError(error: unknown): string {
  return describeSetupError(error);
}

/**
 * Whether a save was refused because a scheduling input moved.
 *
 * The one refusal with a remedy the screen can offer: **generate again**. Nothing was saved,
 * and a screen that showed this as a plain error would make a correct safety check look like
 * a bug.
 */
export function isInputsChanged(error: unknown): boolean {
  return isSetupError(error) && error.kind === "inputsChanged";
}

/** Whether the result is not a timetable at all, so there is nothing to save. */
export function isNotSaveable(error: unknown): boolean {
  return isSetupError(error) && error.kind === "notSaveable";
}

/**
 * The timetable a repeated save found already written, if that is what happened.
 *
 * Not an error for the user: it already succeeded. The screen offers to open it rather than
 * reporting a failure.
 */
export function alreadySavedAs(error: unknown): string | null {
  return isSetupError(error) && error.kind === "alreadySaved" ? error.timetableId : null;
}

// ---------------------------------------------------------------- lookups

/**
 * The lessons of one scope, by index into the projection's flat list.
 *
 * ## Why this is a lookup and not a scheduling rule
 *
 * Every lesson already carries `teacherIds`, `roomId` and `sectionId`, resolved in Rust.
 * Asking *"which of these name this teacher?"* is a set membership test on data that is
 * already decided — it computes no span, judges no clash and reads no clock.
 *
 * The things that **would** be rules all happen before this: which slots a lesson occupies,
 * which bell schedule it belongs to, whether two lessons in one cell are a clash, and what
 * `rowSpan` a cell may use. Those arrive already answered, in
 * {@link ProjectedLesson.occupies} and in the `ScopeGrid` the backend lays out.
 *
 * Used for counts and for the lesson list beside a grid. The grid itself renders the
 * backend's `ScopeGrid`, never this.
 */
export function indexByScope(
  lessons: readonly ProjectedLesson[],
  kind: ScopeKind,
  id: string | null,
  rosterSections?: readonly string[],
): number[] {
  const matches = (lesson: ProjectedLesson): boolean => {
    switch (kind) {
      case "master":
        return true;
      case "section":
        return lesson.sectionId === id;
      case "teacher":
        return id !== null && lesson.teacherIds.includes(id);
      case "room":
        return lesson.roomId === id;
      // Explicit enrolment and nothing else. The sections come from the backend's label set,
      // so an absent roster yields an empty grid rather than a group's timetable.
      case "student":
        return rosterSections !== undefined && rosterSections.includes(lesson.sectionId);
      default:
        return false;
    }
  };
  return lessons.reduce<number[]>((found, lesson, index) => {
    if (matches(lesson)) found.push(index);
    return found;
  }, []);
}

/** The scopes of one kind, in the order the backend listed them. */
export function scopesOfKind(
  scopes: readonly ProjectionScope[],
  kind: ScopeKind,
): ProjectionScope[] {
  return scopes.filter((scope) => scope.kind === kind);
}
