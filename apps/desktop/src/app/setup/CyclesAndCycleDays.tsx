/**
 * Section 6 — Cycles and Cycle Days.
 *
 * ## The cycle is not a week
 *
 * A cycle day's `ordinal` is its whole scheduling identity. `label` is free text —
 * "Monday", "Day A", "Week 2 Thursday" — and `weekdayHint` is explicitly display only.
 * Nothing in this screen derives one from the other, offers a Monday–Friday template, or
 * treats five days as special, because nothing in the model does. A one-day cycle, a
 * five-day week, a six-day rotation and a ten-day fortnight differ here only in how many
 * rows there are.
 *
 * ## Declared length versus actual days
 *
 * `dayCount` is the *intended* length. ADR 0011 Q4 settled that the rows need not be
 * complete — adding days one at a time has to be legal — so an incomplete cycle is a
 * legal state rather than an error. It is also a state that must be impossible to miss,
 * because a grid built from a half-finished cycle is a half-finished grid. Hence
 * {@link CoveragePanel}: it names the positions that have no day, and the answer comes
 * from `evara_core::time::CycleCoverage` over IPC rather than from a count done here.
 *
 * ## Neither `weekCount` nor `isDefault` is asked for
 *
 * `cycle.is_default` was "the school's default cycle", offered as a checkbox and used to
 * pre-select a cycle on the Timetable Grid Preview. It is no longer collected: a default
 * is only worth having once something has to choose a cycle *without* asking, and nothing
 * does — the grid screen now requires the choice to be made. Picking a cycle on the user's
 * behalf from a flag they set once, months earlier, is how the wrong grid gets rebuilt.
 *
 * The column and the IPC contract are unchanged, so {@link CrudSpec.toDraft} carries the
 * stored flag through an edit untouched and sends `false` for a new cycle. **No existing
 * cycle record changes, and nothing is promoted to default automatically** — the old form
 * did exactly that for the first cycle in a project.
 *
 * ## `weekCount` is no longer asked for
 *
 * "Calendar weeks spanned" was display-and-grouping only, and nothing was ever grouped by
 * it — not the materialiser, not the grid, not a report. It asked every user to work out
 * and maintain a number the application did not consult, and it invited exactly the
 * inference this screen exists to avoid: that a cycle is a number of weeks.
 *
 * The field is gone from the forms and from the table. The column and the IPC contract are
 * not: `cycle.week_count` is `NOT NULL CHECK (week_count >= 1)`, a released migration is
 * immutable, and `CycleInput` still carries it. So {@link weekCountFor} supplies one — the
 * row's own value when editing, 1 for a new cycle — which means **no existing cycle record
 * changes**. See the Fix 6 report for why dropping the column was deferred.
 */
import { useCallback, useMemo, useState } from "react";

import type { Cycle, CycleCoverage, CycleDay, School } from "@evara/domain";
import {
  createCycle,
  createCycleDay,
  cycleCoverage,
  deleteCycle,
  deleteCycleDay,
  describeSetupError,
  listCycleDays,
  listCycles,
  updateCycle,
  updateCycleDay,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { Banner, Panel } from "../ui/Shell";
import {
  flag,
  number,
  text,
  textOrNull,
  validateCycle,
  validateCycleDay,
  type Draft,
} from "../validation";
import { Absent } from "./CampusesAndBuildings";
import { ParentPicker } from "./ParentPicker";
import { useList, useValue } from "./useList";

/**
 * The `weekCount` a save must send, for a field no longer on screen.
 *
 * The schema requires `week_count >= 1` and the IPC contract still carries it, so a save
 * has to name a value. The right value is whatever the row already held — preserving it is
 * what keeps "do not silently change existing cycle records" true — and 1 for a cycle being
 * created.
 *
 * Validated rather than trusted, even though the only writers are {@link CrudSpec.toDraft}
 * and this function. `number()` would hand `NaN` to the backend for anything unexpected,
 * and a save rejected by a `CHECK` on a field the user cannot see would be unfixable from
 * the interface. The fallback is 1, which is both the column's own default and the value a
 * new cycle gets.
 */
function weekCountFor(draft: Draft): number {
  const stored = Number(text(draft, "weekCount"));
  return Number.isInteger(stored) && stored >= 1 ? stored : 1;
}

export function CyclesAndCycleDays(props: { school: School }) {
  const { school } = props;
  const [cycleId, setCycleId] = useState<string | null>(null);

  const loadCycles = useCallback(() => listCycles(), []);
  const cycles = useList<Cycle>(loadCycles);

  const loadCoverage = useCallback(
    () =>
      cycleId
        ? cycleCoverage(cycleId)
        : Promise.resolve<CycleCoverage>({
            declaredDayCount: 0,
            present: 0,
            missingOrdinals: [],
            beyondDeclared: [],
          }),
    [cycleId],
  );
  const coverage = useValue<CycleCoverage>(loadCoverage);

  const selected = cycles.items?.find((c) => c.id === cycleId) ?? null;

  const cycleSpec = useMemo<CrudSpec<Cycle, Parameters<typeof createCycle>[0]>>(
    () => ({
      noun: "cycle",
      plural: "cycles",
      title: "Cycles",
      describe: (row) => row.name,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Days", numeric: true, render: (row) => row.dayCount },
      ],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Cycle name",
          required: true,
          placeholder: "Two-week rotation",
        },
        {
          kind: "number",
          key: "dayCount",
          label: "Days in the cycle",
          required: true,
          min: 1,
          hint: "The intended length: 1, 5, 6, 10 — whatever the school actually repeats. There is no assumption of a five-day week.",
        },
      ],
      validate: validateCycle,
      toDraft: (row) => ({
        name: row?.name ?? "",
        dayCount: row ? String(row.dayCount) : "5",
        // Both of these are carried in the draft with no field to render them, so an edit
        // sends each value back exactly as it was read and no input can change either.
        weekCount: row ? String(row.weekCount) : "1",
        // `false` for a new cycle, not "true if this is the first one". The old form made
        // the first cycle the default automatically; nothing does that now, because
        // nothing in the application should acquire a default nobody chose.
        isDefault: row ? row.isDefault : false,
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        name: text(draft, "name"),
        dayCount: number(draft, "dayCount"),
        weekCount: weekCountFor(draft),
        isDefault: flag(draft, "isDefault"),
      }),
      list: listCycles,
      create: createCycle,
      update: updateCycle,
      remove: deleteCycle,
      emptyHint:
        "A cycle is the pattern the timetable repeats. Most schools have one. Add it, then give it a day for each position in the pattern.",
      deleteNote: () => (
        <p className="muted">
          A cycle that still has days, or a materialised grid, cannot be deleted. Remove
          those first.
        </p>
      ),
    }),
    [school.id, cycles.items],
  );

  const daySpec = useMemo<CrudSpec<CycleDay, Parameters<typeof createCycleDay>[0]>>(
    () => ({
      noun: "cycle day",
      plural: "cycle days",
      title: selected ? `Days of ${selected.name}` : "Cycle days",
      describe: (row) => `${row.label} (position ${row.ordinal})`,
      columns: [
        { header: "Position", numeric: true, render: (row) => row.ordinal },
        { header: "Label", render: (row) => row.label },
        {
          header: "Weekday hint",
          render: (row) =>
            row.weekdayHint === null ? <Absent /> : <span>{row.weekdayHint}</span>,
        },
      ],
      fields: [
        {
          kind: "number",
          key: "ordinal",
          label: "Position",
          required: true,
          min: 1,
          max: selected?.dayCount,
          hint: positionHint(selected, coverage.value),
        },
        {
          kind: "text",
          key: "label",
          label: "Label",
          required: true,
          placeholder: "Day A",
          hint: "Whatever the school calls it. Display only — nothing in scheduling reads it.",
        },
        {
          kind: "number",
          key: "weekdayHint",
          label: "Weekday hint",
          min: 1,
          max: 7,
          hint: "Optional, 1–7, display only. Never used for scheduling, ordering or identity.",
        },
      ],
      validate: validateCycleDay,
      toDraft: (row) => ({
        ordinal: row ? String(row.ordinal) : String(nextPosition(coverage.value)),
        label: row?.label ?? "",
        weekdayHint: row?.weekdayHint === null ? "" : String(row?.weekdayHint ?? ""),
      }),
      toInput: (draft) => {
        const hint = textOrNull(draft, "weekdayHint");
        return {
          cycleId: cycleId ?? "",
          ordinal: number(draft, "ordinal"),
          label: text(draft, "label"),
          weekdayHint: hint === null ? null : Number(hint),
        };
      },
      list: () => (cycleId ? listCycleDays(cycleId) : Promise.resolve([])),
      create: createCycleDay,
      update: updateCycleDay,
      remove: deleteCycleDay,
      emptyHint:
        "A cycle needs one day per position in its pattern. Name them however the school does — the position is what the timetable uses.",
      deleteNote: () => (
        <p className="muted">
          A day still used by a timeslot or a mapped calendar date cannot be deleted. The
          Timetable Grid Preview can show you exactly what is in the way before you try.
        </p>
      ),
      blockedBy: cycleId === null ? "Select a cycle first." : null,
      // Not `blockedBy`: the table of days must stay on screen. The reason adding is
      // unavailable is that every one of those days exists.
      addBlockedBy: cycleFull(coverage.value),
    }),
    [cycleId, selected, coverage.value],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Cycles and cycle days</h2>
        <p>
          The cycle is the pattern the timetable repeats. It is not a week: a day's
          position is its identity, and its label is whatever the school calls it.
        </p>
      </header>

      <CrudPanel
        spec={cycleSpec}
        // Coverage as well as the list. Editing a cycle changes its `dayCount`, which is
        // what decides whether the cycle is full -- and `loadCoverage` is keyed on the
        // selected cycle's id, which an edit does not change. Without this, lengthening a
        // cycle left "all cycle positions are filled" on screen and the Add button
        // disabled, so the one instruction that message gives would not have worked.
        onChanged={() => {
          cycles.reload();
          coverage.reload();
        }}
      />

      <hr />

      <ParentPicker
        legend="Cycle"
        items={
          cycles.items?.map((cycle) => ({
            id: cycle.id,
            label: cycle.name,
            detail: `${cycle.dayCount} ${cycle.dayCount === 1 ? "day" : "days"}`,
          })) ?? null
        }
        error={cycles.error}
        selectedId={cycleId}
        onSelect={setCycleId}
        empty={{
          title: "No cycles yet",
          hint: "Add a cycle above, then give it a day for each position in the pattern.",
        }}
      >
        <CoveragePanel
          cycle={selected}
          coverage={coverage.value}
          error={coverage.error}
          onFilled={() => {
            coverage.reload();
            cycles.reload();
          }}
        />
        <CrudPanel
          spec={daySpec}
          reloadKey={cycleId ?? ""}
          onChanged={coverage.reload}
        />
      </ParentPicker>
    </div>
  );
}

/**
 * What the Position field says about the range it will accept.
 *
 * Names the free positions when some are free, because "from 1 to 10" is unhelpful on a
 * cycle whose only gap is 7. Presentation only: the authority on whether a position is
 * legal is `check_ordinal_within_cycle` in `evara-db`, and its refusal is what the form
 * shows if one slips through — which is why this never grows into a validator.
 */
function positionHint(cycle: Cycle | null, coverage: CycleCoverage | undefined): string {
  const identity = "This is the day's identity — everything scheduled points at it.";
  if (!cycle) return `From 1. ${identity}`;

  const free = coverage?.missingOrdinals ?? [];
  if (free.length === 0 || free.length === cycle.dayCount) {
    return `From 1 to ${cycle.dayCount}. ${identity}`;
  }
  const named = free.length <= 4 ? free.join(", ") : `${free.slice(0, 4).join(", ")} and more`;
  return `From 1 to ${cycle.dayCount}; still free: ${named}. ${identity}`;
}

/** The position a new day should default to: the first gap, or one past the end. */
function nextPosition(coverage: CycleCoverage | undefined): number {
  if (!coverage) return 1;
  return coverage.missingOrdinals[0] ?? coverage.present + 1;
}

/** What a full cycle says when asked for another day. */
export const CYCLE_FULL_MESSAGE =
  "All cycle positions are filled. Increase the cycle length to add more days.";

/**
 * Why no further day can be added, or `null` while one still can.
 *
 * Answered from {@link CycleCoverage.missingOrdinals}, which `evara_core::time` computed
 * and the backend handed over. That matters: "is this cycle full?" is a question about the
 * model, and deciding it here by comparing a row count against `dayCount` would be the
 * same rule implemented twice, in the layer `CLAUDE.md` forbids it in. The screen reads an
 * answer; it does not work one out.
 *
 * `undefined` coverage means it has not arrived yet, which is not the same as "full" —
 * disabling the button then would flash it off and on during every load.
 */
function cycleFull(coverage: CycleCoverage | undefined): string | null {
  if (!coverage) return null;
  return coverage.missingOrdinals.length === 0 ? CYCLE_FULL_MESSAGE : null;
}

/**
 * Says plainly whether the cycle is finished, and what is missing if it is not.
 *
 * This is the brief's "the UI must clearly identify [incomplete cycles]". It is a banner
 * rather than a quiet badge because the consequence is downstream and invisible: the grid
 * will build, it will simply be short of a day, and nothing later would say why.
 */
function CoveragePanel(props: {
  cycle: Cycle | null;
  coverage: CycleCoverage | undefined;
  error: string | null;
  onFilled: () => void;
}) {
  const { cycle, coverage, error, onFilled } = props;
  const [busy, setBusy] = useState(false);
  const [fillError, setFillError] = useState<string | null>(null);

  const fill = useCallback(async () => {
    if (!cycle || !coverage) return;
    setBusy(true);
    setFillError(null);
    try {
      // One command per day, in order, stopping at the first refusal. Deliberately not a
      // single bulk call: there is no bulk repository method, and inventing one in the
      // interface would put a loop with its own partial-failure semantics in the view
      // layer. A starting label the user will rename is a convenience, not a decision.
      for (const ordinal of coverage.missingOrdinals) {
        await createCycleDay({
          cycleId: cycle.id,
          ordinal,
          label: `Day ${ordinal}`,
          weekdayHint: null,
        });
      }
      onFilled();
    } catch (e: unknown) {
      setFillError(describeSetupError(e));
      // Some days may have been created before the failure, so the panel must re-read
      // rather than assume nothing happened.
      onFilled();
    } finally {
      setBusy(false);
    }
  }, [cycle, coverage, onFilled]);

  if (error) return <Banner tone="error">{error}</Banner>;
  if (!cycle || !coverage) return null;

  const complete =
    coverage.missingOrdinals.length === 0 &&
    coverage.beyondDeclared.length === 0 &&
    coverage.declaredDayCount >= 1;

  return (
    <Panel title="Cycle completeness">
      {fillError && <Banner tone="error">{fillError}</Banner>}

      {complete ? (
        <Banner tone="ok">
          This cycle is complete: {coverage.present} of {coverage.declaredDayCount}{" "}
          positions have a day. A timetable grid built from it will cover the whole
          pattern.
        </Banner>
      ) : (
        <Banner tone="warning">
          <strong>This cycle is incomplete.</strong> {coverage.present} of{" "}
          {coverage.declaredDayCount} positions have a day
          {coverage.missingOrdinals.length > 0 && (
            <>
              ; nothing is defined at{" "}
              {coverage.missingOrdinals.length === 1 ? "position" : "positions"}{" "}
              <strong>{coverage.missingOrdinals.join(", ")}</strong>
            </>
          )}
          . That is allowed while you are still building it, but a timetable grid will be
          short of those days until they exist.
          {coverage.beyondDeclared.length > 0 && (
            <>
              {" "}
              There {coverage.beyondDeclared.length === 1 ? "is a day" : "are days"} at{" "}
              <strong>{coverage.beyondDeclared.join(", ")}</strong>, past the declared
              length of {coverage.declaredDayCount} — either lengthen the cycle or remove
              them.
            </>
          )}
        </Banner>
      )}

      {coverage.missingOrdinals.length > 0 && (
        <div className="actions">
          <button type="button" className="secondary" disabled={busy} onClick={() => void fill()}>
            {busy
              ? "Adding…"
              : `Add the ${coverage.missingOrdinals.length} missing ${
                  coverage.missingOrdinals.length === 1 ? "position" : "positions"
                }`}
          </button>
          <span className="hint">
            Creates a day at each missing position, labelled “Day N”. Rename them to
            whatever the school uses.
          </span>
        </div>
      )}
    </Panel>
  );
}
