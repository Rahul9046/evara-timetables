/**
 * The words and numbers a transfer screen shows, in one place.
 *
 * ## What is here and what is deliberately not
 *
 * Here: the label for each {@link TransferStep}, a byte size a person can read, and the
 * sentence that distinguishes a cancellation from a failure.
 *
 * Not here: **any wording that explains a refusal.** Every `ProjectError` carries its own
 * `message`, written beside the rule that produced it, and the screens show that string. A
 * second sentence composed here would drift from the check — which is the mistake ADR 0043
 * records for violation explanations and the same mistake in a different layer.
 *
 * The step labels are the one place this file names something the backend also names.
 * `TransferStep` is a seven-variant enum whose Rust `as_str` is a lower-case word for a log
 * line (`"snapshotting"`); what a dialogue needs is a sentence fragment in the interface's own
 * vocabulary (*"Taking a copy of the campus"*). Mapping it once, here, is what keeps the two
 * from being invented per screen.
 */
import type { TransferStep } from "@evara/domain";

/** What each step of the work is called on screen. */
const STEP_LABELS: Record<TransferStep, string> = {
  snapshotting: "Taking a copy of the campus",
  hashing: "Checking the copy",
  compressing: "Writing the portable file",
  staging: "Reading the portable file",
  verifying: "Verifying what arrived",
  migrating: "Bringing the campus up to date",
  finalising: "Finishing",
};

/**
 * The label for a step, or a neutral one before the first event.
 *
 * Never a percentage on its own: the steps take wildly different times, so a single bar
 * spends most of its life inside one of them and the label is what actually tells a user
 * where the work has got to.
 */
export function stepLabel(step: TransferStep | null): string {
  return step === null ? "Starting" : STEP_LABELS[step];
}

/**
 * A byte count in units a person reads.
 *
 * Binary units, because that is what a file manager on either platform shows for a file this
 * size, and a number that disagrees with the one beside it in Explorer or Finder reads as a
 * bug. One decimal place above a kibibyte; none below, since "1.0 B" is noise.
 */
export function bytes(count: number): string {
  if (!Number.isFinite(count) || count < 0) return "—";
  if (count < 1024) return `${Math.round(count)} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = count / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/**
 * A percentage for the bar, or `null` when there is nothing to divide by.
 *
 * `total` is documented as `0` until it is known, and a bar at `NaN%` is worse than no bar.
 */
export function percent(done: number, total: number): number | null {
  if (total <= 0) return null;
  return Math.max(0, Math.min(100, Math.round((done / total) * 100)));
}

/**
 * An ISO-8601 instant as a local date and time, or the raw string if it will not parse.
 *
 * The raw string rather than a guess: an archive's `exportedAt` comes from whatever wrote it,
 * and showing an unparseable value as "Invalid Date" tells the user nothing about their file.
 */
export function instant(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  return at.toLocaleString();
}
