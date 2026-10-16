/**
 * Spreadsheet roster import, across the IPC boundary.
 *
 * Wrapped exactly once, here, like every other command — see `./setup.ts` for why, and
 * `./registration.test.ts` for the test that holds both sides in step.
 *
 * ## The file crosses as bytes, once
 *
 * {@link beginImport} sends the chosen file's name and contents and the backend keeps the
 * parsed table for the rest of the session. Every later call — {@link previewImport},
 * {@link applyImport} — sends only the mapping and the decisions. That is partly about not
 * shipping a workbook through the boundary on every change to the mapper, and mostly about
 * the file reviewed being the file applied: the fingerprint is computed over the *plan*, so
 * a re-sent file with the same resulting actions would have the same fingerprint and the
 * substitution would go unnoticed. The backend holding the table makes it unexpressible.
 *
 * ## The school is sent every time
 *
 * It is the user's current selection rather than something the session remembers, because
 * it can change between two previews of one file. ADR 0023 has the shell remount a section
 * when the school changes, and the backend refuses a fingerprint from another school
 * regardless — both halves are wanted, for the reason `SchoolSetup` gives.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  ColumnMapping,
  ImportFingerprint,
  ImportKind,
  ImportKindView,
  ImportPlan,
  ImportSourceView,
  ImportSummary,
  RowDecision,
  TemplateKindFormat,
} from "@evara/domain";

/** Every kind of import, in the order a school would do them. */
export function importKinds(): Promise<ImportKindView[]> {
  return invoke<ImportKindView[]>("import_kinds");
}

/**
 * Prepares a blank template and returns the name to suggest for it.
 *
 * Followed by {@link saveImportFile} once the user has chosen where it goes. The bytes
 * stay in the backend throughout — see {@link saveImportFile}.
 *
 * The workbook format is the one to offer by default: it formats the code columns as text,
 * which is what stops a spreadsheet turning student `00124` into `124` before Evara ever
 * sees the file.
 */
export function prepareImportTemplate(
  kind: ImportKind,
  format: TemplateKindFormat,
): Promise<string> {
  return invoke<string>("import_template", { kind, format });
}

/**
 * Reads a chosen file and starts a session, replacing any already in progress.
 *
 * `base64` is the file's bytes, without the `data:` prefix a `FileReader` adds. `sheet`
 * picks a worksheet; omitted, the backend takes the first.
 */
export function beginImport(
  kind: ImportKind,
  fileName: string,
  base64: string,
  sheet?: string,
): Promise<ImportSourceView> {
  return invoke<ImportSourceView>("import_begin", {
    kind,
    fileName,
    base64,
    sheet: sheet ?? null,
  });
}

/** Reads another worksheet of the file already chosen, without re-sending it. */
export function selectImportSheet(sheet: string): Promise<ImportSourceView> {
  return invoke<ImportSourceView>("import_sheet", { sheet });
}

/**
 * The session as it stands, or `null`.
 *
 * Read on mount so a school switch — which remounts the section — can restore the mapper
 * rather than asking the user for the file again.
 */
export function currentImport(): Promise<ImportSourceView | null> {
  return invoke<ImportSourceView | null>("import_current");
}

/**
 * Builds a preview. Writes nothing.
 *
 * `decisions` answers the rows the planner would not decide alone. Re-previewing with a
 * new answer is how a conflict is resolved: the plan is rebuilt, which is also what keeps
 * the fingerprint honest.
 */
export function previewImport(
  schoolId: string,
  mapping: ColumnMapping,
  decisions: readonly RowDecision[],
): Promise<ImportPlan> {
  return invoke<ImportPlan>("import_preview", { schoolId, mapping, decisions });
}

/**
 * Applies a previewed plan, in one transaction.
 *
 * `fingerprint` is the one the preview returned. A plan that has moved since is refused
 * with `kind: "reviewRequired"` and **nothing is written** — the interface re-previews and
 * asks again.
 */
export function applyImport(
  schoolId: string,
  mapping: ColumnMapping,
  decisions: readonly RowDecision[],
  fingerprint: ImportFingerprint,
): Promise<ImportSummary> {
  return invoke<ImportSummary>("import_apply", {
    schoolId,
    mapping,
    decisions,
    fingerprint,
  });
}

/**
 * Prepares the report for the import just applied, and returns the name to suggest.
 *
 * Rendered by the backend from its own record of what happened, not from the summary this
 * window is holding, so the file a school keeps cannot disagree with what Evara did.
 */
export function prepareImportReport(): Promise<string> {
  return invoke<string>("import_report");
}

/**
 * Writes whatever was last prepared, to a path the user chose in a save dialogue.
 *
 * Sends **no content**: the bytes are the ones the backend generated, and there is no way
 * for this window to make it write anything else. The path is the only authority the write
 * has, and it came from an OS dialogue the page cannot script — the same arrangement
 * `createProject` has.
 *
 * Returns the name of the file written, for the confirmation message.
 */
export function saveImportFile(path: string): Promise<string> {
  return invoke<string>("import_save", { path });
}

/** Ends the session and forgets the file. Called when the workflow closes. */
export function cancelImport(): Promise<boolean> {
  return invoke<boolean>("import_cancel");
}
