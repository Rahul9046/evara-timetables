/**
 * What Evara did about interrupted imports when it started — D11, read back.
 *
 * ## Read back, never re-run
 *
 * Recovery runs once, at startup, before any campus file is open — because the campus that
 * needs recovering may be the one that will not open. `project_recovery_report` reports what
 * it did. Nothing in this component retries, reconciles or deletes anything: an ambiguous
 * state is reported and acted on by nothing, and guessing is how the protocol would lose
 * somebody's campus.
 *
 * ## Resolved and unresolved are different, and are not styled the same
 *
 * `resolved: true` means Evara put the campus file back, or finished the import, and cleared
 * the record — information, and dismissible. `resolved: false` means Evara **changed nothing**
 * and left the record for a person to look at; it stays on screen, because a warning the
 * application quietly forgets is a warning nobody acts on.
 *
 * ## It blocks nothing
 *
 * Neither kind stops a campus file being created, opened or imported. The interrupted import
 * is about a path, not about the application, and an unresolved record is a thing to look at
 * rather than a reason to refuse everything else.
 */
import { useEffect, useState } from "react";

import type { RecoveredView } from "@evara/domain";

import { recoveryReport } from "../../ipc/transfer";
import { Banner } from "../ui/Shell";

export function RecoveryNotices() {
  const [entries, setEntries] = useState<RecoveredView[]>([]);
  const [dismissed, setDismissed] = useState<readonly string[]>([]);

  useEffect(() => {
    let live = true;
    recoveryReport()
      .then((found) => {
        if (live) setEntries(found);
      })
      .catch(() => {
        // A report that cannot be read is not worth a banner of its own: recovery has
        // already run or not run, and this is the record of it rather than the act.
      });
    return () => {
      live = false;
    };
  }, []);

  const unresolved = entries.filter((entry) => !entry.resolved);
  const resolved = entries.filter(
    (entry) => entry.resolved && !dismissed.includes(entry.destination),
  );
  if (unresolved.length === 0 && resolved.length === 0) return null;

  return (
    <section className="xferRecovery" aria-labelledby="xferRecoveryTitle">
      <h2 className="sectionHead" id="xferRecoveryTitle">
        Interrupted imports
      </h2>

      {unresolved.map((entry) => (
        <Banner key={entry.destination} tone="warning">
          <strong>{entry.fileName}</strong> — {entry.outcome}. Evara changed nothing here, and
          has kept the record so it can be looked at. The folder is{" "}
          <span className="mono">{entry.destination}</span>.
        </Banner>
      ))}

      {resolved.map((entry) => (
        <div key={entry.destination} className="xferRecoveryRow">
          <Banner tone="info">
            <strong>{entry.fileName}</strong> — {entry.outcome}.
          </Banner>
          <button
            type="button"
            className="linkish"
            onClick={() => setDismissed((was) => [...was, entry.destination])}
            aria-label={`Dismiss the note about ${entry.fileName}`}
          >
            Dismiss
          </button>
        </div>
      ))}
    </section>
  );
}
