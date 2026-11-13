/**
 * Portable campus transfer — the `.evara` export, inspect and import surface.
 *
 * Wrapped exactly once, here, for the reason {@link ./repair} is: the command *name* and
 * argument shape are the one part of the boundary generated types cannot check, so they are
 * asserted in one place and covered by tests.
 *
 * ## Nothing here decides anything
 *
 * Every type comes from `@evara/domain`, generated from the Rust. This file declares no shape
 * of its own, reads no archive, compares no digest and judges no schema. In particular it does
 * **not** work out whether an archive is importable: `ArchiveSummaryView.readable` and its
 * `refusal` are decided by the reader that owns the verification order, and an opinion formed
 * here could tell a user an archive was fine that the import then refused.
 *
 * ## The two objects, and they are never confused
 *
 * | Extension | What it is |
 * | --- | --- |
 * | `.evaraproj` | the **live campus**: what Evara opens, migrates and writes |
 * | `.evara` | the **portable file**: a verified snapshot of one, for sending somewhere |
 *
 * {@link archiveExtension} and {@link projectExtension} are the two sources a dialog filter is
 * built from. Neither string is ever written out in this package.
 *
 * ## What an inspection proves, and what it does not
 *
 * {@link inspectArchive} reads an archive's manifest and its digest record. It does **not**
 * read the payload — that is verified during the import, which is where a damaged one is
 * caught and refused before anything is written. So a readable summary means *this build
 * understands the container*, never *the campus inside is intact*.
 *
 * And a clean verification says the bytes arrived intact; it says nothing about who produced
 * them. {@link ArchiveSummaryView.integrityNotice} carries that sentence, written in Rust so
 * there is one source of the wording, and a screen offering an import must show it.
 *
 * ## The two events, and why they are not the solver's
 *
 * `transfer://progress` and `transfer://finished` are distinct from `solver://` and
 * `repair://`. Each listener filters on `runId` as well, so a transfer the user has moved on
 * from — or one belonging to a campus they have since closed — cannot drive the screen in
 * front of them.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ArchiveSummaryView,
  ExportStarted,
  ExportStatus,
  ImportStarted,
  ImportStatus,
  OnConflict,
  ProjectError,
  RecoveredView,
  TransferFinishedEvent,
  TransferProgressEvent,
} from "@evara/domain";

import { isProjectError } from "./project";

// ---------------------------------------------------------------- the extension

/**
 * The extension a portable campus file must use, so dialog filters have one source.
 *
 * Distinct from `projectExtension()`, which is the live campus. Using one where the other
 * belongs is the confusion the two commands exist to prevent.
 */
export function archiveExtension(): Promise<string> {
  return invoke<string>("project_archive_extension");
}

// ---------------------------------------------------------------- export

/**
 * Snapshots the open campus and writes a portable file, on a worker thread.
 *
 * Returns as soon as the copy has been taken, which is why `totalBytes` is exact. The archive
 * itself is written afterwards, with no database lock held.
 *
 * Refuses a destination that already exists. There is deliberately no overwrite: the remedy is
 * another name, which costs nothing and cannot destroy a file somebody may still need.
 */
export function startExport(path: string): Promise<ExportStarted> {
  return invoke<ExportStarted>("project_export_start", { path });
}

/** How an export is doing, and its report once it has one. */
export function exportStatus(runId: string): Promise<ExportStatus> {
  return invoke<ExportStatus>("project_export_status", { runId });
}

/**
 * Asks an export to stop.
 *
 * Resolves to whether something was still running. `false` is not an error: there was nothing
 * to stop, and reporting success would say the cancel did something it did not. A cancelled
 * export leaves no archive and no partial file.
 */
export function cancelExport(runId: string): Promise<boolean> {
  return invoke<boolean>("project_export_cancel", { runId });
}

/** Forgets an export session, stopping it if it is still going. */
export function discardExport(runId: string): Promise<void> {
  return invoke<void>("project_export_discard", { runId });
}

// ---------------------------------------------------------------- import

/**
 * What an archive says about itself. Writes nothing, stages nothing, imports nothing.
 *
 * A version or capability problem is **not** a rejection here — it comes back as
 * `readable: false` with a `refusal`, so a screen can say what is wrong without catching
 * anything.
 */
export function inspectArchive(path: string): Promise<ArchiveSummaryView> {
  return invoke<ArchiveSummaryView>("project_import_inspect", { path });
}

/**
 * Reads an archive into a campus file, on a worker thread.
 *
 * `onConflict` is `"refuse"` or `"replace"` — one vocabulary, in the Rust enum, the command
 * and here. **Always offer `"refuse"` first**: the refusal names the campus and says where the
 * current copy will be kept, and a replacement nobody confirmed is not a replacement anybody
 * asked for.
 *
 * Does not open what it imported. The imported campus is added to the recent list instead, so
 * the interface can offer "Open now" as the user's own decision.
 */
export function startImport(
  archivePath: string,
  destinationPath: string,
  onConflict: OnConflict,
): Promise<ImportStarted> {
  return invoke<ImportStarted>("project_import_start", {
    archivePath,
    destinationPath,
    onConflict,
  });
}

/** How an import is doing, and its report once it has one. */
export function importStatus(runId: string): Promise<ImportStatus> {
  return invoke<ImportStatus>("project_import_status", { runId });
}

/**
 * Asks an import to stop.
 *
 * A cancelled import leaves no campus file and no half-finished work beside it.
 */
export function cancelImport(runId: string): Promise<boolean> {
  return invoke<boolean>("project_import_cancel", { runId });
}

/** Forgets an import session, stopping it if it is still going. */
export function discardImport(runId: string): Promise<void> {
  return invoke<void>("project_import_discard", { runId });
}

// ---------------------------------------------------------------- recovery

/**
 * What Evara did about interrupted imports when it started.
 *
 * Read back, never re-run: recovery happens before any campus is open, because the campus
 * that needs recovering may be the one that will not open. An entry with `resolved: false`
 * means Evara changed **nothing** and left the record for a person to look at.
 */
export function recoveryReport(): Promise<RecoveredView[]> {
  return invoke<RecoveredView[]>("project_recovery_report");
}

// ---------------------------------------------------------------- events

/** Subscribes to one transfer's progress. Filters on `runId`. */
export async function onTransferProgress(
  runId: string,
  handler: (event: TransferProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<TransferProgressEvent>("transfer://progress", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

/** Subscribes to one transfer's completion. Filters on `runId`, as progress does. */
export async function onTransferFinished(
  runId: string,
  handler: (event: TransferFinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<TransferFinishedEvent>("transfer://finished", ({ payload }) => {
    if (payload.runId === runId) handler(payload);
  });
}

// ---------------------------------------------------------------- errors

/**
 * Whether a refusal is one the user can act on by choosing differently.
 *
 * Separated from the rest because the screen's response is an offer rather than an apology:
 * another name, or Replace. Every one of these leaves what is already on disk untouched.
 */
export function isDestinationConflict(error: unknown): error is ProjectError {
  return (
    isProjectError(error) &&
    (error.kind === "alreadyExists" ||
      error.kind === "destinationIsOpen" ||
      error.kind === "transferBlocked")
  );
}

/**
 * Whether Evara is too old to read this archive.
 *
 * Both of the two version axes: `formatTooNew` is the container and `schemaTooNew` is the data
 * inside it. One remedy — update Evara — and they are kept apart in the backend because they
 * are different problems, so a screen that wants to explain *which* still can.
 */
export function isTooOldToRead(error: unknown): error is ProjectError {
  return (
    isProjectError(error) && (error.kind === "formatTooNew" || error.kind === "schemaTooNew")
  );
}

/**
 * Whether the archive arrived damaged.
 *
 * The remedy is always available and is always the same: ask whoever sent it for a fresh copy.
 * Nothing was written, so there is nothing for the user to undo.
 */
export function isArchiveDamaged(error: unknown): error is ProjectError {
  return isProjectError(error) && error.kind === "archiveDamaged";
}
