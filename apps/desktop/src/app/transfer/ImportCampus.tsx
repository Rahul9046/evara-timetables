/**
 * Import a portable `.evara` into a campus file.
 *
 * ## Inspect, then confirm — in that order, always
 *
 * Choosing a file does not import it. `project_import_inspect` reads the archive's manifest
 * and its digest record and **writes nothing**, so a user decides whether to go on while
 * knowing what the file claims to be, who wrote it, which schools are inside and whether this
 * build can read it at all.
 *
 * ## What an inspection does not prove, said on the screen
 *
 * The inspection does **not** read the payload. Streaming and hashing the campus inside is
 * step 9 of the reader's verification order and belongs to the import, which is where a
 * damaged one is caught — before anything is written. So a readable summary means *this build
 * understands the container*, never *the campus inside is intact*, and this dialogue says so
 * rather than letting a green tick imply the stronger claim.
 *
 * And a clean verification says the bytes arrived as the archive described them; it says
 * nothing about who produced them. `integrityNotice` carries that sentence, written in Rust so
 * there is one source of the wording, and it is shown every time.
 *
 * ## Nothing here overrides a refusal
 *
 * `readable: false` removes the import, and the backend's `refusal.message` is what is shown.
 * This screen forms no opinion about a version, a capability or a digest — an opinion here
 * could tell a user an archive was fine that the import then refused.
 *
 * ## Replacement is a second, explicit click
 *
 * The first attempt is always `"refuse"`. An occupied destination comes back with the
 * backend's own sentence, which names the campus and the name the current copy will be kept
 * under, and only then is Replace offered. Nothing is deleted on either path: the previous
 * campus file is kept beside the new one, and no code path in Evara removes it.
 *
 * ## It does not open what it imported
 *
 * D9. Auto-opening would close whatever was open and discard an edit session as a side effect
 * of an import. The imported campus is remembered instead, and "Open now" is the user's own
 * click.
 */
import { useCallback, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import type { ArchiveSummaryView, ImportReportView, RecentProjectView } from "@evara/domain";

import { describeError, isProjectError, projectExtension } from "../../ipc/project";
import {
  archiveExtension,
  cancelImport,
  discardImport,
  importStatus,
  inspectArchive,
  isArchiveDamaged,
  isDestinationConflict,
  isTooOldToRead,
  startImport,
} from "../../ipc/transfer";
import { Banner, Facts, Modal } from "../ui/Shell";
import { TransferProgress } from "./TransferProgress";
import { bytes, instant } from "./labels";
import { useTransferSession } from "./useTransferSession";

/** A destination the backend refused as occupied, with the sentence it refused it in. */
interface Occupied {
  readonly path: string;
  readonly message: string;
}

export function ImportCampus(props: {
  open: boolean;
  onClose: () => void;
  /** Opens the imported campus file. Only ever called from the user's own click — D9. */
  onOpenImported: (path: string) => void;
  /** Called once an import has succeeded, so the recent list reflects it. */
  onImported: () => void;
}) {
  const { open, onClose, onOpenImported, onImported } = props;

  const session = useTransferSession<ImportReportView>({
    status: importStatus,
    cancel: cancelImport,
    discard: discardImport,
  });

  const [summary, setSummary] = useState<ArchiveSummaryView | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [occupied, setOccupied] = useState<Occupied | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** How to describe the error: a conflict, an old build, a damaged file, or none of those. */
  const [remedy, setRemedy] = useState<"conflict" | "update" | "fresh" | null>(null);

  const busy = inspecting || choosing || session.starting || session.running;
  const phase = session.phase;
  const report = session.report;

  const clearError = () => {
    setError(null);
    setRemedy(null);
  };

  /** Records a rejection with the remedy its predicate chooses, and never rewords it. */
  const show = useCallback((e: unknown) => {
    setError(describeError(e));
    setRemedy(
      isDestinationConflict(e)
        ? "conflict"
        : isTooOldToRead(e)
          ? "update"
          : isArchiveDamaged(e)
            ? "fresh"
            : null,
    );
  }, []);

  const chooseArchive = useCallback(async () => {
    if (busy) return;
    clearError();
    setOccupied(null);
    setInspecting(true);
    try {
      const extension = await archiveExtension();
      const picked = await openDialog({
        title: "Choose a portable campus file",
        multiple: false,
        directory: false,
        filters: [{ name: "Evara portable campus", extensions: [extension] }],
      });
      if (typeof picked !== "string") return;
      setSummary(await inspectArchive(picked));
    } catch (e: unknown) {
      setSummary(null);
      show(e);
    } finally {
      setInspecting(false);
    }
  }, [busy, show]);

  /** Starts an import at `path`, refusing an occupied destination unless `replace`. */
  const begin = useCallback(
    async (path: string, replace: boolean) => {
      if (summary === null) return;
      clearError();
      try {
        await session.begin(() => startImport(summary.path, path, replace ? "replace" : "refuse"));
        setOccupied(null);
      } catch (e: unknown) {
        // An occupied destination is the one refusal with a second offer. The sentence is
        // the backend's: it names the campus and the keep-name, and composing one here
        // would be a second source of wording for one rule.
        if (!replace && isProjectError(e) && e.kind === "alreadyExists") {
          setOccupied({ path, message: e.message });
          return;
        }
        setOccupied(null);
        show(e);
      }
    },
    [summary, session, show],
  );

  const chooseDestination = useCallback(async () => {
    if (busy || summary === null) return;
    clearError();
    setChoosing(true);
    try {
      const extension = await projectExtension();
      const path = await saveDialog({
        title: "Save the imported campus as",
        defaultPath: summary.suggestedFileName,
        filters: [{ name: "Evara campus", extensions: [extension] }],
      });
      if (typeof path !== "string") return;
      await begin(path, false);
    } catch (e: unknown) {
      show(e);
    } finally {
      setChoosing(false);
    }
  }, [busy, summary, begin, show]);

  /**
   * Escape, and the Close button.
   *
   * A running import is asked to **stop** rather than dismissed, so the dialogue cannot be
   * left behind with a worker nobody is watching. Once a stop has been asked for the
   * dialogue closes on the next try — a cancel is a flag read between steps, and a
   * cancelled import leaves no campus file and nothing beside it either way.
   */
  const dismiss = useCallback(() => {
    if (session.running && !session.cancelling) {
      void session.requestCancel();
      return;
    }
    void session.reset();
    setSummary(null);
    setOccupied(null);
    clearError();
    onClose();
  }, [session, onClose]);

  return (
    <Modal open={open} labelledBy="xferImportTitle" className="xfer" onCancel={dismiss}>
      <h3 id="xferImportTitle">Import a campus</h3>

      <div className="confirmBody">
        {summary === null && phase === null && (
          <p>
            Choose a portable campus file — one ending in <code className="mono">.evara</code>{" "}
            — and Evara will read it, tell you what is inside, and only then ask where to save
            it. Nothing is written until you say so.
          </p>
        )}

        {summary !== null && phase === null && (
          <ArchiveDetails summary={summary} />
        )}

        {occupied !== null && (
          <section className="xferReplace" aria-labelledby="xferReplaceTitle">
            <h4 id="xferReplaceTitle">That file already exists</h4>
            {/* The backend's own sentence. When the destination is the *same* campus it says
                so; when it is a different one it says that instead — and either way it names
                what the current copy will be kept as. */}
            <p>{occupied.message}</p>
            <p className="muted">
              Nothing is deleted. Your current copy is kept beside the new one, and no part of
              Evara removes it afterwards.
            </p>
          </section>
        )}

        {session.running && (
          <>
            <p>
              Importing <strong>{summary?.fileName}</strong>.
            </p>
            <TransferProgress
              labelledBy="xferImportTitle"
              step={session.step}
              done={session.done}
              total={session.total}
              cancelling={session.cancelling}
            />
          </>
        )}

        {phase === "cancelled" && (
          <Banner tone="info">
            The import was stopped. No campus file was written and nothing was left behind
            beside it.
          </Banner>
        )}

        {phase === "failed" && (
          <Banner tone="error">
            {session.failure === null ? "The import did not finish." : session.failure.message}
          </Banner>
        )}

        {phase === "finished" && report !== null && <ImportResult report={report} />}

        {error !== null && (
          <>
            <Banner tone="error">{error}</Banner>
            {remedy === "conflict" && (
              <p className="muted">Choose another name, or close the campus file first.</p>
            )}
            {remedy === "update" && (
              <p className="muted">
                Update Evara and try again. Nothing was written, so the file is unchanged.
              </p>
            )}
            {remedy === "fresh" && (
              <p className="muted">
                Ask whoever sent it for a fresh copy. Nothing was written, so there is nothing
                to undo.
              </p>
            )}
          </>
        )}
      </div>

      <div className="actions">
        {summary === null && phase === null && (
          <button type="button" onClick={() => void chooseArchive()} disabled={busy}>
            {inspecting ? "Reading…" : "Choose a portable file…"}
          </button>
        )}

        {/* An unreadable archive offers no import. The refusal is the backend's and this
            screen does not get a vote on it. */}
        {summary !== null && summary.readable && phase === null && occupied === null && (
          <button type="button" onClick={() => void chooseDestination()} disabled={busy}>
            {choosing ? "Choosing…" : "Choose where to save…"}
          </button>
        )}

        {occupied !== null && (
          <>
            <button
              type="button"
              className="danger"
              disabled={busy}
              onClick={() => void begin(occupied.path, true)}
            >
              Replace and keep a copy
            </button>
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => void chooseDestination()}
            >
              Choose another name…
            </button>
          </>
        )}

        {summary !== null && phase === null && occupied === null && (
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => void chooseArchive()}
          >
            Choose a different file…
          </button>
        )}

        {phase === "finished" && report !== null && (
          <button
            type="button"
            onClick={() => {
              const path = report.path;
              void session.reset();
              setSummary(null);
              clearError();
              onClose();
              onOpenImported(path);
            }}
          >
            Open now
          </button>
        )}

        {(phase === "cancelled" || phase === "failed") && summary !== null && (
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => void session.reset()}
          >
            Try again
          </button>
        )}

        <button
          type="button"
          className="secondary"
          onClick={() => {
            if (phase === "finished") onImported();
            dismiss();
          }}
        >
          {session.running ? (session.cancelling ? "Close anyway" : "Stop") : "Close"}
        </button>
      </div>
    </Modal>
  );
}

/** What the archive says about itself, and what that does and does not establish. */
function ArchiveDetails(props: { summary: ArchiveSummaryView }) {
  const { summary } = props;

  return (
    <>
      <Facts
        items={[
          ["Portable file", <span className="mono">{summary.fileName}</span>],
          ["Size", `${bytes(summary.archiveBytes)} (${bytes(summary.payloadBytes)} inside)`],
          ["Written by", `${summary.generatorApp} ${summary.generatorVersion}`],
          ["Exported", instant(summary.exportedAt)],
          [
            "Schools",
            summary.schools.length === 0
              ? "none"
              : summary.schools
                  .map((school) => `${school.name} (${school.campuses} campuses)`)
                  .join(", "),
          ],
          ["Schema", `v${summary.schemaVersion}`],
          ["Rows", `${summary.totalRows} in ${summary.tablesWithRows} tables`],
          ["Change history", `${summary.changeLogEntries} entries`],
        ]}
      />

      {summary.capabilities.length > 0 && (
        <p className="muted">
          Needs support for: {summary.capabilities.join(", ")}.
        </p>
      )}

      {summary.unknownEntries.length > 0 && (
        <p className="muted">
          This file also carries {summary.unknownEntries.join(", ")}, which this version of
          Evara does not use and will ignore.
        </p>
      )}

      {/* D13's first sentence, verbatim from the backend, before anything is staged. */}
      {summary.identityNotice !== null && (
        <Banner tone="warning">{summary.identityNotice}</Banner>
      )}
      {summary.identityConflicts.length > 0 && (
        <IdentityConflicts entries={summary.identityConflicts} />
      )}

      {summary.readable ? (
        <Banner tone="info">
          Evara can read this file: the container, its description and its checksum record are
          all ones this version understands. The campus inside is checked against that record
          during the import — not now — so a damaged one is caught then, and refused before
          anything is written.
        </Banner>
      ) : (
        <Banner tone="error">
          {summary.refusal === null
            ? "This version of Evara cannot read that file."
            : summary.refusal.message}
        </Banner>
      )}

      {/* Said every time, at the point somebody decides whether to trust the file. */}
      <Banner tone="warning">{summary.integrityNotice}</Banner>
    </>
  );
}

/**
 * The campus files already on this machine carrying this identity — D13.
 *
 * Listed rather than summarised, because "which one" is the question a user has next. Nothing
 * here offers to merge, delete or renumber: Evara cannot merge two copies, and regenerating
 * the identity would destroy the only thing that says they are the same campus.
 */
function IdentityConflicts(props: { entries: readonly RecentProjectView[] }) {
  return (
    <section className="xferConflicts" aria-labelledby="xferConflictsTitle">
      <h4 id="xferConflictsTitle">Already on this machine</h4>
      <ul>
        {props.entries.map((entry) => (
          <li key={entry.path}>
            <strong>{entry.displayName}</strong>{" "}
            <span className="muted">
              {entry.exists ? entry.folder : "file not found"}
            </span>
          </li>
        ))}
      </ul>
      <p className="muted">
        Two campus files can legitimately be the same campus — that is what copying one or
        importing it twice produces. Evara keeps both and tells you they are the same; it never
        merges them, deletes one or gives one a new identity.
      </p>
    </section>
  );
}

/** What arrived, including the copy that was kept when something was replaced. */
function ImportResult(props: { report: ImportReportView }) {
  const { report } = props;

  return (
    <>
      <Banner tone="ok">
        <strong>{report.fileName}</strong> is ready.
      </Banner>
      <Facts
        items={[
          ["Campus file", <span className="mono">{report.path}</span>],
          ["Rows", `${report.totalRows} in ${report.tablesWithRows} tables`],
          [
            "Schema",
            report.migrated
              ? `brought up to date: v${report.schemaVersionFound} → v${report.schemaVersionNow}`
              : `v${report.schemaVersionNow}`,
          ],
        ]}
      />

      {report.replacedCopyName !== null && (
        <Banner tone="info">
          Your previous campus file is kept as <strong>{report.replacedCopyName}</strong>
          {report.replacedCopyKeptAt === null ? null : (
            <>
              {" "}
              (<span className="mono">{report.replacedCopyKeptAt}</span>)
            </>
          )}
          . Evara never deletes it.
        </Banner>
      )}

      {report.countDisagreements.length > 0 && (
        <Banner tone="warning">
          This file described itself inaccurately:{" "}
          {report.countDisagreements
            .map((row) => `${row.table} said ${row.declared}, holds ${row.found}`)
            .join("; ")}
          . The campus itself is what arrived and is what Evara read.
        </Banner>
      )}

      {!report.remembered && (
        <p className="muted">
          The campus is on disk and correct, but it could not be added to the recent list. Use
          “Open campus…” to reach it.
        </p>
      )}
    </>
  );
}
