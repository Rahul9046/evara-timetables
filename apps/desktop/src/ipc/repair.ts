/**
 * Automated repair and partial regeneration, across the IPC boundary.
 *
 * Wrapped exactly once, here, for the reason {@link ./timetables} is: the command *name* and
 * argument shape are the one part of the boundary generated types cannot check, so they are
 * asserted in one place and covered by tests.
 *
 * ## Nothing here decides anything
 *
 * Every type comes from `@evara/domain`, generated from the Rust. This file declares no shape
 * of its own, computes no score, derives no scope and judges no placement. In particular it
 * does **not** work out whether a repair improved a timetable: `RepairDiff.improved` is
 * decided in Rust, because the comparison is lexicographic over three levels rather than a
 * sum, and an interface that added them up would rank one hard clash above several soft ones.
 *
 * ## The two events, and why they are not the solver's
 *
 * `repair://progress` and `repair://finished` are deliberately distinct from
 * `solver://progress` and `solver://finished`. Both run on their own worker threads and both
 * emit progress, so a screen watching a repair must not be woken by a generation. Each
 * listener filters on `runId` as well, so a session the user has moved on from cannot drive
 * the screen in front of them.
 *
 * ## The scope is never widened here
 *
 * {@link startRepair} sends back exactly the scope {@link repairScopes} handed out. Where a
 * wider scope would plainly help, the screen offers to start a **separate** repair — a new
 * session, a new baseline, a new preview and a new approval. Nothing in this file grows a
 * movable set.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  GenerateSettings,
  RepairApplied,
  RepairFinishedEvent,
  RepairPreflight,
  RepairProgressEvent,
  RepairResult,
  RepairScope,
  RepairScopes,
  RepairStarted,
  RepairStatus,
} from "@evara/domain";

import { describeSetupError, isSetupError } from "./setup";
import type { Selection } from "./timetables";

// ---------------------------------------------------------------- reading

/** What scopes this timetable offers, with the count each would make movable. Writes nothing. */
export async function repairScopes(
  selection: Selection,
  timetableId: string,
): Promise<RepairScopes> {
  return invoke("repair_scopes", { selection, timetableId });
}

/**
 * Whether a repair may proceed over one scope, and what stands in the way.
 *
 * Writes nothing, so it is safe to call whenever the chosen scope changes.
 */
export async function repairPreflight(
  selection: Selection,
  timetableId: string,
  scope: RepairScope,
): Promise<RepairPreflight> {
  return invoke("repair_preflight", { selection, timetableId, scope });
}

// ---------------------------------------------------------------- running

/** Starts a repair on a worker thread and returns immediately. Writes nothing. */
export async function startRepair(
  selection: Selection,
  timetableId: string,
  scope: RepairScope,
  settings?: GenerateSettings,
): Promise<RepairStarted> {
  return invoke("repair_start", { selection, timetableId, scope, settings });
}

/** How a repair is doing. */
export async function repairStatus(runId: string): Promise<RepairStatus> {
  return invoke("repair_status", { runId });
}

/** The finished repair, with its diff. */
export async function repairResult(runId: string): Promise<RepairResult> {
  return invoke("repair_result", { runId });
}

/**
 * Asks a repair to stop.
 *
 * Returns whether something was still running. `false` is not an error: there was nothing to
 * stop, and reporting success would tell the user their cancel did something it did not.
 */
export async function cancelRepair(runId: string): Promise<boolean> {
  return invoke("repair_cancel", { runId });
}

/** Forgets a repair session. */
export async function discardRepair(runId: string): Promise<boolean> {
  return invoke("repair_discard", { runId });
}

/**
 * Applies a repair, as a **new** timetable. The one command here that writes.
 *
 * Never an overwrite: the timetable being repaired is untouched, and the result names it as
 * its source.
 */
export async function applyRepair(runId: string, name: string): Promise<RepairApplied> {
  return invoke("repair_apply", { runId, name });
}

// ---------------------------------------------------------------- events

/** Subscribes to one repair's progress. Filters on `runId`. */
export async function onRepairProgress(
  runId: string,
  handler: (event: RepairProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<RepairProgressEvent>("repair://progress", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

/** Subscribes to one repair's completion. Filters on `runId`, as progress does. */
export async function onRepairFinished(
  runId: string,
  handler: (event: RepairFinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<RepairFinishedEvent>("repair://finished", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

// ---------------------------------------------------------------- errors

/** A repair failure, in the words the backend chose. */
export function describeRepairError(error: unknown): string {
  return describeSetupError(error);
}

/**
 * Whether the baseline was edited since the repair began.
 *
 * Its own question because the remedy is specific and is not a retry: the diff the user is
 * looking at describes a timetable that no longer exists, so the repair has to be run again
 * from the arrangement as it now stands.
 */
export function isArrangementChanged(error: unknown): boolean {
  return isSetupError(error) && /edited|arrangement/i.test(error.message ?? "");
}

/**
 * Whether a scheduling input moved since the problem was compiled.
 *
 * Distinct from {@link isArrangementChanged}: this one means a *rule or resource* changed, so
 * every verdict the repair computed is untrue, not merely out of date.
 */
export function isInputsChanged(error: unknown): boolean {
  return isSetupError(error) && /input/i.test(error.message ?? "");
}

/**
 * Whether the repair was refused because it disturbed a frozen placement.
 *
 * A **defect in Evara**, not a problem with the timetable, and the screen says so in those
 * words: the alternative is a user trying to fix their own data in response to our bug.
 */
export function isScopeViolation(error: unknown): boolean {
  return isSetupError(error) && /outside the scope it was given/i.test(error.message ?? "");
}
