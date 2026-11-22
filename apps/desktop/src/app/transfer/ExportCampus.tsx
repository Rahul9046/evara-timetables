/**
 * Export the open campus file to a portable `.evara`.
 *
 * ## What the user is agreeing to
 *
 * One file, carrying a verified copy of everything in the open campus file: every school,
 * timetable version, manual edit, lock, repair lineage edge and change-feed row. The campus
 * itself is **not** touched — the backend opens it read-only, which is why this dialogue can
 * say so without hedging.
 *
 * ## There is no overwrite, and that is a decision
 *
 * An occupied destination comes back as a refusal and this screen offers another name. It
 * never offers to replace: an archive is a document somebody may still need, the remedy costs
 * one dialogue, and the backend claims the name atomically anyway, so a file that appears
 * while the archive is being written is refused rather than destroyed.
 *
 * ## Why a dialogue and not a screen
 *
 * The flow is short-lived, modal by nature — it must not be navigated away from half-done —
 * and the application has no router to give it a route. The existing `<dialog>` element is
 * what School Setup already confirms destructive work with, so this looks like the rest of
 * the application rather than like a new one.
 */
import { useCallback, useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";

import type { ExportReportView } from "@evara/domain";

import { describeError } from "../../ipc/project";
import {
  archiveExtension,
  cancelExport,
  discardExport,
  exportStatus,
  isDestinationConflict,
  startExport,
} from "../../ipc/transfer";
import { Banner, Facts, Modal } from "../ui/Shell";
import { TransferProgress } from "./TransferProgress";
import { bytes, instant } from "./labels";
import { useTransferSession } from "./useTransferSession";

export function ExportCampus(props: {
  open: boolean;
  /** The open campus file's display name, for the sentence that says what travels. */
  campusName: string;
  onClose: () => void;
}) {
  const { open, campusName, onClose } = props;

  const session = useTransferSession<ExportReportView>({
    status: exportStatus,
    cancel: cancelExport,
    discard: discardExport,
  });
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [choosing, setChoosing] = useState(false);

  const busy = choosing || session.starting || session.running;

  const chooseAndStart = useCallback(async () => {
    if (busy) return;
    setError(null);
    setConflict(false);
    setChoosing(true);
    try {
      // The extension comes from the backend, which owns both of them and keeps the live
      // campus file and the portable one from being confused for each other.
      const extension = await archiveExtension();
      const path = await saveDialog({
        title: "Export campus to a portable file",
        defaultPath: `${campusName}.${extension}`,
        filters: [{ name: "Evara portable campus", extensions: [extension] }],
      });
      if (typeof path !== "string") return;
      await session.begin(() => startExport(path));
    } catch (e: unknown) {
      // A destination the backend will not write to is an offer, not an apology: the user
      // chooses another name. Everything else is shown as it was reported.
      setConflict(isDestinationConflict(e));
      setError(describeError(e));
    } finally {
      setChoosing(false);
    }
  }, [busy, campusName, session]);

  /**
   * Escape, and the Close button.
   *
   * A running export is asked to **stop** rather than dismissed, so the dialogue cannot be
   * left behind with a worker nobody is watching. Once a stop has been asked for the
   * dialogue closes on the next try: a cancel is a flag the engine reads between steps, and
   * a modal a user cannot get out of while that happens would be worse than leaving the
   * worker to finish on its own — which it does safely, writing no archive.
   */
  const dismiss = useCallback(() => {
    if (session.running && !session.cancelling) {
      void session.requestCancel();
      return;
    }
    void session.reset();
    setError(null);
    setConflict(false);
    onClose();
  }, [session, onClose]);

  const report = session.report;
  const phase = session.phase;

  return (
    <Modal open={open} labelledBy="xferExportTitle" className="xfer" onCancel={dismiss}>
      <h3 id="xferExportTitle">Export campus</h3>

      <div className="confirmBody">
        {phase === null && (
          <>
            <p>
              This writes <strong>{campusName}</strong> to one portable file ending in{" "}
              <code className="mono">.evara</code>, which you can copy, send or keep as an
              archive. Everything travels: every school, timetable version, manual edit, lock
              and the change history.
            </p>
            <p className="muted">
              The campus file itself is opened read-only and is not changed, and whatever you
              have open stays open.
            </p>
          </>
        )}

        {session.running && (
          <>
            <p>
              Writing <strong>{campusName}</strong> to a portable file.
            </p>
            <TransferProgress
              labelledBy="xferExportTitle"
              step={session.step}
              done={session.done}
              total={session.total}
              cancelling={session.cancelling}
            />
          </>
        )}

        {phase === "cancelled" && (
          <Banner tone="info">
            The export was stopped. Nothing was written — there is no archive and no
            half-finished file to clear up.
          </Banner>
        )}

        {phase === "failed" && (
          <Banner tone="error">
            {session.failure === null
              ? "The export did not finish."
              : session.failure.message}
          </Banner>
        )}

        {phase === "finished" && report !== null && (
          <>
            <Banner tone="ok">
              <strong>{report.fileName}</strong> is ready.
            </Banner>
            <Facts
              items={[
                ["Portable file", <span className="mono">{report.path}</span>],
                [
                  "Size",
                  `${bytes(report.archiveBytes)} (from ${bytes(
                    report.payloadBytes,
                  )}, ${report.compressionSavingPercent.toFixed(1)}% smaller)`,
                ],
                [
                  "Schools",
                  report.schools.length === 0
                    ? "none"
                    : report.schools
                        .map((school) => `${school.name} (${school.campuses} campuses)`)
                        .join(", "),
                ],
                ["Schema", `v${report.schemaVersion}`],
                ["Change history", `${report.changeLogEntries} entries`],
                ["Written", instant(report.exportedAt)],
              ]}
            />
            <p className="muted">
              The identity inside the file is the same one this campus has, so a copy imported
              elsewhere is recognisably the same campus rather than a new one.
            </p>
          </>
        )}

        {error !== null && (
          <>
            <Banner tone="error">{error}</Banner>
            {conflict && (
              <p className="muted">
                Evara will not overwrite a portable file you may still need. Choose another
                name.
              </p>
            )}
          </>
        )}
      </div>

      <div className="actions">
        {!session.running && phase !== "finished" && (
          <button type="button" onClick={() => void chooseAndStart()} disabled={busy}>
            {/* After a refusal the offer is a *different* name, which is the remedy the
                backend's own sentence names. A second button labelled as the first would
                read as "try that again". */}
            {error === null && phase === null ? "Choose a destination…" : "Try another name…"}
          </button>
        )}
        <button type="button" className="secondary" onClick={dismiss}>
          {session.running ? (session.cancelling ? "Close anyway" : "Stop") : "Close"}
        </button>
      </div>
    </Modal>
  );
}
