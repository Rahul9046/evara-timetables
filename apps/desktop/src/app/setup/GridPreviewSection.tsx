/**
 * Section 9 — Timetable Grid Preview.
 *
 * Not a lesson timetable. This is the *value range* a timetable will later be placed
 * into: every `(cycle day, period)` pair of one grid, and whether a slot exists for it.
 *
 * ## Why this screen is built the way it is
 *
 * A timeslot's identifier is what a future lesson points at. If rebuilding the grid
 * reassigned identifiers, every placement would silently point somewhere else — the
 * timetable would still load, it would just be wrong, and no migration could repair it
 * because the information needed was the identifier. That is why the whole interaction is
 * preview → confirm → apply, and why destruction is a different button.
 *
 * Three guarantees, each visible in the code below:
 *
 * 1. **Rebuilding never deletes.** {@link rebuildGrid} creates missing slots and
 *    refreshes surviving ones. Stranded slots are reported and left alone.
 * 2. **A confirmation belongs to the figures that were shown.** Every preview carries a
 *    fingerprint; the confirm sends it back and the backend recomputes the plan inside its
 *    write transaction and refuses if it has changed. The comparison is not done here —
 *    doing it in the interface would be a check-then-act race.
 * 3. **Releasing stranded slots is a separate, explicitly confirmed action** that lists
 *    what will be lost. It is never bundled into a rebuild.
 *
 * ## Orphans are a property of a proposal, not of a row
 *
 * A stored slot is never *intrinsically* stranded: its day and its period still exist,
 * and the grid's `RESTRICT` references are what stop them being deleted. So orphans can
 * only be discovered by asking "what would this grid be if that day were gone?" — which
 * is what the "plan without" toggles do. Without them the orphan list would always be
 * empty and the removal workflow would be unreachable.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Cycle,
  GridPreview,
  MaterialisationRequest,
  PeriodStructure,
  School,
  Term,
} from "@evara/domain";
import {
  describeSetupError,
  isReviewRequired,
  listAcademicYears,
  listCycles,
  listPeriodStructures,
  listTerms,
  previewGrid,
  rebuildGrid,
  releaseOrphans,
} from "../../ipc/setup";
import { Banner, ConfirmDialog, EmptyState, Loading, Panel } from "../ui/Shell";
import { useList, useValue } from "./useList";

/** Shortens an identifier for display. The full value is in the cell's title. */
function shortId(id: string): string {
  return id.slice(0, 8);
}

export function GridPreviewSection(_props: { school: School }) {
  const [cycleId, setCycleId] = useState<string | null>(null);
  const [structureId, setStructureId] = useState<string | null>(null);
  const [termId, setTermId] = useState<string | null>(null);
  const [withoutDays, setWithoutDays] = useState<readonly string[]>([]);
  const [withoutPeriods, setWithoutPeriods] = useState<readonly string[]>([]);

  const [message, setMessage] = useState<{ tone: "ok" | "warning" | "error"; text: string } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState<"rebuild" | "release" | null>(null);

  const loadCycles = useCallback(() => listCycles(), []);
  const cycles = useList<Cycle>(loadCycles);

  const loadStructures = useCallback(() => listPeriodStructures(), []);
  const structures = useList<PeriodStructure>(loadStructures);

  const loadTerms = useCallback(async () => {
    const years = await listAcademicYears();
    const lists = await Promise.all(years.map((year) => listTerms(year.id)));
    return lists.flat();
  }, []);
  const terms = useList<Term>(loadTerms);

  // Default to the school's own default cycle and schedule: they are what the user means.
  const effectiveCycleId = cycleId ?? pick(cycles.items, (c) => c.isDefault);
  const effectiveStructureId = structureId ?? pick(structures.items, (s) => s.isDefault);

  const request: MaterialisationRequest | null = useMemo(() => {
    if (!effectiveCycleId || !effectiveStructureId) return null;
    return {
      cycleId: effectiveCycleId,
      periodStructureId: effectiveStructureId,
      termId,
      withoutCycleDays: [...withoutDays],
      withoutPeriods: [...withoutPeriods],
    };
  }, [effectiveCycleId, effectiveStructureId, termId, withoutDays, withoutPeriods]);

  const loadPreview = useCallback(
    () => (request ? previewGrid(request) : Promise.resolve(null)),
    [request],
  );
  const preview = useValue<GridPreview | null>(loadPreview);

  /** Re-reads the plan and tells the user why, when a confirmation went stale. */
  const refreshAfterStaleness = useCallback(() => {
    preview.reload();
    setMessage({
      tone: "warning",
      text:
        "The grid changed while this preview was open, so nothing was applied. " +
        "The figures below have been re-read — review them and confirm again.",
    });
  }, [preview]);

  const doRebuild = useCallback(async () => {
    const grid = preview.value;
    if (!request || !grid) return;
    setBusy(true);
    setMessage(null);
    try {
      const plan = await rebuildGrid(request, grid.fingerprint);
      const counts = plan.preserved.length;
      setConfirming(null);
      preview.reload();
      setMessage({
        tone: "ok",
        text:
          `Grid rebuilt. ${plan.created.length} slot${plan.created.length === 1 ? "" : "s"} created, ` +
          `${counts} preserved with their existing identifiers, ` +
          `${plan.refreshed} refreshed. ` +
          `${plan.orphaned.length} stranded slot${plan.orphaned.length === 1 ? "" : "s"} left untouched.`,
      });
    } catch (e: unknown) {
      setConfirming(null);
      if (isReviewRequired(e)) {
        refreshAfterStaleness();
      } else {
        setMessage({ tone: "error", text: describeSetupError(e) });
      }
    } finally {
      setBusy(false);
    }
  }, [request, preview, refreshAfterStaleness]);

  const doRelease = useCallback(async () => {
    const grid = preview.value;
    if (!request || !grid) return;
    setBusy(true);
    setMessage(null);
    try {
      const removed = await releaseOrphans(request, grid.fingerprint);
      setConfirming(null);
      preview.reload();
      setMessage({
        tone: "ok",
        text:
          `${removed.length} stranded slot${removed.length === 1 ? "" : "s"} deleted. ` +
          "The cycle days and periods you excluded can now be removed in their own sections.",
      });
    } catch (e: unknown) {
      setConfirming(null);
      if (isReviewRequired(e)) {
        refreshAfterStaleness();
      } else {
        setMessage({ tone: "error", text: describeSetupError(e) });
      }
    } finally {
      setBusy(false);
    }
  }, [request, preview, refreshAfterStaleness]);

  const toggle = useCallback(
    (list: readonly string[], set: (next: readonly string[]) => void, id: string) => {
      set(list.includes(id) ? list.filter((x) => x !== id) : [...list, id]);
      setMessage(null);
    },
    [],
  );

  if (cycles.error) return <Banner tone="error">{cycles.error}</Banner>;
  if (cycles.items === null || structures.items === null) {
    return <Loading what="the timetable grid" />;
  }

  if (cycles.items.length === 0 || structures.items.length === 0) {
    return (
      <div className="section">
        <GridIntro />
        <EmptyState title="Not enough defined yet">
          A grid is a cycle's days crossed with a bell schedule's periods.
          {cycles.items.length === 0 && " Add a cycle in Cycles and Cycle Days."}
          {structures.items.length === 0 &&
            " Add a bell schedule in Bell Schedules and Periods."}
        </EmptyState>
      </div>
    );
  }

  const grid = preview.value;
  const excluding = withoutDays.length > 0 || withoutPeriods.length > 0;

  return (
    <div className="section">
      <GridIntro />

      <Panel title="Which grid">
        <div className="gridPickers">
          <Selector
            id="grid-cycle"
            label="Cycle"
            value={effectiveCycleId ?? ""}
            options={cycles.items.map((c) => ({
              value: c.id,
              label: c.isDefault ? `${c.name} (default)` : c.name,
            }))}
            onChange={(value) => {
              setCycleId(value);
              setWithoutDays([]);
              setMessage(null);
            }}
          />
          <Selector
            id="grid-structure"
            label="Bell schedule"
            value={effectiveStructureId ?? ""}
            options={structures.items.map((s) => ({
              value: s.id,
              label: s.isDefault ? `${s.name} (default)` : s.name,
            }))}
            onChange={(value) => {
              setStructureId(value);
              setWithoutPeriods([]);
              setMessage(null);
            }}
          />
          <Selector
            id="grid-term"
            label="Scope"
            value={termId ?? ""}
            emptyLabel="Whole year"
            options={(terms.items ?? []).map((t) => ({ value: t.id, label: t.name }))}
            onChange={(value) => {
              setTermId(value);
              setMessage(null);
            }}
          />
        </div>
        <p className="hint">
          A term-scoped grid and a year-wide grid are different grids. Build a per-term one
          only if the bell schedule changes during the year.
        </p>
      </Panel>

      {message && <Banner tone={message.tone}>{message.text}</Banner>}
      {preview.error && <Banner tone="error">{preview.error}</Banner>}

      {grid === undefined ? (
        <Loading what="the grid" />
      ) : grid === null ? null : (
        <>
          <StatusPanel grid={grid} excluding={excluding} />

          <Panel
            title="The grid"
            note={
              <>
                Rows are cycle days in position order; columns are periods in position
                order. A cell shows the identifier of the slot that exists, or that none
                does. Use <strong>Plan without</strong> to see what removing a day or a
                period would strand, before trying to remove it.
              </>
            }
          >
            <GridTable
              grid={grid}
              withoutDays={withoutDays}
              withoutPeriods={withoutPeriods}
              onToggleDay={(id) => toggle(withoutDays, setWithoutDays, id)}
              onTogglePeriod={(id) => toggle(withoutPeriods, setWithoutPeriods, id)}
            />
          </Panel>

          <Panel title="Rebuild">
            <PlanSummary grid={grid} />
            <div className="actions">
              <button
                type="button"
                disabled={busy || grid.counts.created === 0}
                onClick={() => setConfirming("rebuild")}
              >
                Rebuild Timetable Grid
              </button>
              <button type="button" className="secondary" disabled={busy} onClick={preview.reload}>
                Re-read the plan
              </button>
            </div>
            {grid.counts.created === 0 && (
              <p className="hint">
                Nothing to create: every cell this proposal wants already exists. Rebuilding
                would write nothing and consume no revisions.
              </p>
            )}
          </Panel>

          {grid.orphaned.length > 0 && (
            <Panel title="Stranded slots">
              <Banner tone="warning">
                <strong>
                  {grid.orphaned.length} existing slot
                  {grid.orphaned.length === 1 ? "" : "s"} would be stranded by this
                  proposal.
                </strong>{" "}
                Rebuilding will <em>not</em> touch them — it never deletes. Deleting them is
                a separate, deliberate action, and it is what finally allows the excluded
                day or period to be removed.
              </Banner>
              <ul className="orphanList">
                {grid.orphaned.map((slot) => (
                  <li key={slot.id ?? `${slot.cycleDayId}:${slot.periodId}`}>
                    <code>{slot.id ? shortId(slot.id) : "?"}</code>
                    <span>
                      {labelOfDay(grid, slot.cycleDayId)} ·{" "}
                      {labelOfPeriod(grid, slot.periodId)}
                    </span>
                  </li>
                ))}
              </ul>
              <div className="actions">
                <button
                  type="button"
                  className="danger"
                  disabled={busy}
                  onClick={() => setConfirming("release")}
                >
                  Delete {grid.orphaned.length} stranded slot
                  {grid.orphaned.length === 1 ? "" : "s"}…
                </button>
              </div>
            </Panel>
          )}

          <ConfirmDialog
            open={confirming === "rebuild"}
            title="Rebuild the timetable grid?"
            confirmLabel="Rebuild grid"
            busy={busy}
            onConfirm={() => void doRebuild()}
            onCancel={() => setConfirming(null)}
          >
            <p>This will:</p>
            <ul className="planList">
              <li>
                <strong>Create {grid.counts.created}</strong> slot
                {grid.counts.created === 1 ? "" : "s"} that do not exist yet
              </li>
              <li>
                <strong>Keep {grid.counts.preserved}</strong> existing slot
                {grid.counts.preserved === 1 ? "" : "s"}, each with its current identifier
                unchanged
              </li>
              <li>
                <strong>Delete nothing.</strong>{" "}
                {grid.orphaned.length > 0
                  ? `${grid.orphaned.length} stranded slot${
                      grid.orphaned.length === 1 ? "" : "s"
                    } will be left exactly as they are.`
                  : "Nothing would be stranded by this proposal."}
              </li>
            </ul>
            <p className="hint">
              The plan is recomputed as it is applied. If the grid has changed since this
              preview, nothing will be written and you will be asked to review again.
            </p>
          </ConfirmDialog>

          <ConfirmDialog
            open={confirming === "release"}
            title={`Permanently delete ${grid.orphaned.length} slot${
              grid.orphaned.length === 1 ? "" : "s"
            }?`}
            confirmLabel="Delete slots"
            tone="danger"
            busy={busy}
            onConfirm={() => void doRelease()}
            onCancel={() => setConfirming(null)}
          >
            <p>
              These slots will be removed from the project. This cannot be undone, and it is
              the step that makes the excluded cycle day or period deletable.
            </p>
            <ul className="orphanList">
              {grid.orphaned.map((slot) => (
                <li key={slot.id ?? `${slot.cycleDayId}:${slot.periodId}`}>
                  <code>{slot.id ? shortId(slot.id) : "?"}</code>
                  <span>
                    {labelOfDay(grid, slot.cycleDayId)} · {labelOfPeriod(grid, slot.periodId)}
                  </span>
                </li>
              ))}
            </ul>
            <p className="hint">
              If anything already refers to one of these slots, the whole deletion is
              refused and nothing is removed.
            </p>
          </ConfirmDialog>
        </>
      )}
    </div>
  );
}

function GridIntro() {
  return (
    <header className="sectionIntro">
      <h2>Timetable grid preview</h2>
      <p>
        Every slot a lesson could later occupy: the cycle's days crossed with a bell
        schedule's periods. No lessons are placed here — this is the space they will be
        placed into.
      </p>
    </header>
  );
}

/** Materialisation status, in words before numbers. */
function StatusPanel(props: { grid: GridPreview; excluding: boolean }) {
  const { grid, excluding } = props;
  const missing = grid.cells.filter((cell) => cell.timeslotId === null).length;
  const existing = grid.cells.length - missing;
  const cycleComplete =
    grid.coverage.missingOrdinals.length === 0 &&
    grid.coverage.beyondDeclared.length === 0 &&
    grid.coverage.declaredDayCount >= 1;
  const built = missing === 0 && grid.periods.length > 0;

  return (
    <Panel title="Status">
      {excluding && (
        <Banner tone="info">
          You are previewing a <strong>proposal</strong>, not the current configuration:
          one or more days or periods are excluded. Nothing has been changed.
        </Banner>
      )}

      {grid.periods.length === 0 ? (
        <Banner tone="warning">
          This bell schedule has no periods, so the grid has no columns and nothing can be
          scheduled. Add periods in Bell Schedules and Periods.
        </Banner>
      ) : built && cycleComplete ? (
        <Banner tone="ok">
          <strong>This grid is complete.</strong> All {existing} slots exist, and the cycle
          has a day at every one of its {grid.coverage.declaredDayCount} declared positions.
        </Banner>
      ) : (
        <Banner tone="warning">
          <strong>This grid is not finished.</strong>
          {missing > 0 && (
            <>
              {" "}
              {missing} of {grid.cells.length} cells have no slot — rebuild to create them.
            </>
          )}
          {!cycleComplete && (
            <>
              {" "}
              The cycle itself is incomplete
              {grid.coverage.missingOrdinals.length > 0 && (
                <>
                  {" "}
                  (nothing at position{grid.coverage.missingOrdinals.length === 1 ? "" : "s"}{" "}
                  {grid.coverage.missingOrdinals.join(", ")})
                </>
              )}
              , so even a fully built grid would be short of those days.
            </>
          )}
        </Banner>
      )}

      <dl className="counts">
        <div>
          <dt>Cycle days</dt>
          <dd>{grid.days.length}</dd>
        </div>
        <div>
          <dt>Periods</dt>
          <dd>{grid.periods.length}</dd>
        </div>
        <div>
          <dt>Slots existing</dt>
          <dd>{existing}</dd>
        </div>
        <div>
          <dt>Slots missing</dt>
          <dd className={missing > 0 ? "bad" : undefined}>{missing}</dd>
        </div>
      </dl>
    </Panel>
  );
}

/** Preserved / new / orphaned, as the plan reports them. */
function PlanSummary(props: { grid: GridPreview }) {
  const { counts } = props.grid;
  return (
    <dl className="counts counts--plan">
      <div>
        <dt>Would be preserved</dt>
        <dd>{counts.preserved}</dd>
        <p className="hint">Identifiers unchanged</p>
      </div>
      <div>
        <dt>Would be created</dt>
        <dd className={counts.created > 0 ? "new" : undefined}>{counts.created}</dd>
        <p className="hint">New slots, new identifiers</p>
      </div>
      <div>
        <dt>Would be stranded</dt>
        <dd className={counts.orphaned > 0 ? "bad" : undefined}>{counts.orphaned}</dd>
        <p className="hint">Never deleted by a rebuild</p>
      </div>
    </dl>
  );
}

/** The grid itself, with per-axis exclusion toggles. */
function GridTable(props: {
  grid: GridPreview;
  withoutDays: readonly string[];
  withoutPeriods: readonly string[];
  onToggleDay: (id: string) => void;
  onTogglePeriod: (id: string) => void;
}) {
  const { grid, withoutDays, withoutPeriods, onToggleDay, onTogglePeriod } = props;

  if (grid.days.length === 0 || grid.periods.length === 0) {
    return (
      <p className="muted">
        The grid has no {grid.days.length === 0 ? "rows" : "columns"} yet.
      </p>
    );
  }

  const cellAt = new Map(
    grid.cells.map((cell) => [`${cell.cycleDayId}:${cell.periodId}`, cell]),
  );

  return (
    <div className="tableScroll">
      <table className="gridTable">
        <caption className="srOnly">
          Cycle days by period, showing which timeslots exist
        </caption>
        <thead>
          <tr>
            <th scope="col">Cycle day</th>
            {grid.periods.map((period) => (
              <th
                key={period.id}
                scope="col"
                className={withoutPeriods.includes(period.id) ? "excluded" : undefined}
              >
                <span className="axisLabel">{period.label}</span>
                <span className="axisDetail">
                  #{period.ordinal} · {period.startsAt}–{period.endsAt}
                </span>
                <span className="axisDetail">
                  {period.kind === "TEACHING" ? "teaching" : period.kind.toLowerCase()}
                </span>
                <button
                  type="button"
                  className="linkish"
                  aria-pressed={withoutPeriods.includes(period.id)}
                  onClick={() => onTogglePeriod(period.id)}
                >
                  {withoutPeriods.includes(period.id) ? "Include" : "Plan without"}
                </button>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {grid.days.map((day) => {
            const dayExcluded = withoutDays.includes(day.id);
            return (
              <tr key={day.id} className={dayExcluded ? "excluded" : undefined}>
                <th scope="row">
                  <span className="axisLabel">{day.label}</span>
                  <span className="axisDetail">position {day.ordinal}</span>
                  <button
                    type="button"
                    className="linkish"
                    aria-pressed={dayExcluded}
                    onClick={() => onToggleDay(day.id)}
                  >
                    {dayExcluded ? "Include" : "Plan without"}
                  </button>
                </th>
                {grid.periods.map((period) => {
                  const cell = cellAt.get(`${day.id}:${period.id}`);
                  const excluded = dayExcluded || withoutPeriods.includes(period.id);
                  const exists = cell?.timeslotId != null;
                  const classes = [
                    "slot",
                    exists ? "slot--present" : "slot--missing",
                    cell?.isTeaching ? "slot--teaching" : "slot--nonteaching",
                    excluded ? "slot--excluded" : "",
                  ]
                    .filter(Boolean)
                    .join(" ");

                  return (
                    <td key={period.id} className={classes}>
                      {exists ? (
                        <span title={`Timeslot ${cell?.timeslotId ?? ""}`}>
                          <code>{shortId(cell?.timeslotId ?? "")}</code>
                        </span>
                      ) : (
                        <span className="missingMark" title="No timeslot exists for this cell">
                          missing
                        </span>
                      )}
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function labelOfDay(grid: GridPreview, id: string): string {
  const day = grid.days.find((d) => d.id === id);
  return day ? `${day.label} (position ${day.ordinal})` : "an unknown day";
}

function labelOfPeriod(grid: GridPreview, id: string): string {
  const period = grid.periods.find((p) => p.id === id);
  return period ? `${period.label} (#${period.ordinal})` : "an unknown period";
}

function pick<T extends { id: string }>(
  items: readonly T[] | null,
  isPreferred: (item: T) => boolean,
): string | null {
  if (!items || items.length === 0) return null;
  return (items.find(isPreferred) ?? items[0])?.id ?? null;
}

/** A labelled select. Plain enough not to need the form machinery. */
function Selector(props: {
  id: string;
  label: string;
  value: string;
  emptyLabel?: string;
  options: ReadonlyArray<{ value: string; label: string }>;
  onChange: (value: string | null) => void;
}) {
  return (
    <div className="field">
      <label htmlFor={props.id}>{props.label}</label>
      <select
        id={props.id}
        value={props.value}
        onChange={(e) => props.onChange(e.currentTarget.value || null)}
      >
        {props.emptyLabel !== undefined && <option value="">{props.emptyLabel}</option>}
        {props.options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  );
}
