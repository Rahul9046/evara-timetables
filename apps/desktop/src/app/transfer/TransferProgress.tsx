/**
 * The progress of a running transfer, with the one button that stops it.
 *
 * Shared by both directions, because both report the same three things: which step, how far
 * through it, and whether a stop has been asked for.
 *
 * ## The announcement is polite and is the label, not the number
 *
 * `aria-live="polite"` on the step line, and the bar carries `aria-valuenow`. The backend
 * coalesces progress to an interval, so this is not a per-byte announcement — but a bar that
 * announced "41 per cent, 42 per cent, 43 per cent" would still be useless to listen to,
 * where "Writing the portable file" is the thing a user wants to hear.
 *
 * ## The stop button is not here
 *
 * It lives in the dialogue's footer, where every other action in this application's dialogues
 * lives, so a transfer has exactly one button that stops it. What this component does show is
 * that one has been **asked for**: cancellation is a flag the engine reads between steps, so
 * the session leaves `running` when the worker returns and not when the button is clicked.
 */
import type { TransferStep } from "@evara/domain";

import { bytes, percent, stepLabel } from "./labels";

export function TransferProgress(props: {
  /** Names the bar for a screen reader, and is the heading the panel already shows. */
  labelledBy: string;
  step: TransferStep | null;
  done: number;
  total: number;
  cancelling: boolean;
}) {
  const { labelledBy, step, done, total, cancelling } = props;
  const share = percent(done, total);

  return (
    <div className="xferProgress">
      <div
        className="xferBar"
        role="progressbar"
        aria-labelledby={labelledBy}
        aria-valuemin={0}
        {...(share === null
          ? // An indeterminate bar: no `aria-valuenow` at all, which is how a screen
            // reader is told the value is unknown. A `0` here would announce "nought per
            // cent" for work that has in fact started.
            {}
          : { "aria-valuemax": 100, "aria-valuenow": share })}
      >
        <div className="xferBarFill" style={{ width: share === null ? "0%" : `${share}%` }} />
      </div>

      <p className="xferBarText" aria-live="polite">
        {stepLabel(step)}
        {share === null ? "" : ` — ${share}%`}
        {total > 0 ? ` (${bytes(done)} of ${bytes(total)})` : ""}
        {cancelling ? " — stopping after the current step" : "…"}
      </p>
    </div>
  );
}
