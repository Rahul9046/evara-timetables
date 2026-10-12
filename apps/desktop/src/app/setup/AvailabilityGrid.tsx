/**
 * The availability editor: one grid, shared by teachers, rooms and student groups.
 *
 * ## Available is the absence of a rule
 *
 * A blank cell is not "unknown" — it is available. That is the model's design and it is
 * what keeps "I have not configured this yet" and "this teacher is never free" from
 * collapsing into the same empty set. So clearing a cell **removes** a row rather than
 * storing an `AVAILABLE` status, which does not exist.
 *
 * ## The axes come from the cycle, never from a week
 *
 * Columns are the chosen cycle's days in `ordinal` order, rows are the chosen bell
 * schedule's periods. Nothing here reads a weekday: a six-day rotation that drifts against
 * the calendar renders exactly as a seven-day week does, which is the whole point of
 * `cycle_day.ordinal` being the only scheduling identity. `label` is display, and
 * `weekday_hint` is not consulted at all.
 *
 * The grid is read from {@link previewGrid}, the same projection the Timetable grid preview
 * section uses, so the cycle days and periods are never re-derived here. A second copy of
 * "what cells exist" is exactly the drift `CLAUDE.md` forbids. It writes nothing, which is
 * why an editor may call it freely.
 *
 * **A cell needs no materialised timeslot.** Availability is recurring and is stored
 * against `(cycle_day, period)`, so it can be configured before the grid has ever been
 * built and survives every rebuild. `GridCell.timeslotId` is deliberately ignored.
 *
 * ## Keyboard and screen readers
 *
 * A real `<table>` of real `<button>`s. Arrow keys move between cells, Enter and Space
 * cycle the status, and each button carries the full sentence — "Amara Okafor, Monday,
 * period 1: available" — because a cell's visual position is not available to someone
 * hearing it. A grid of `<div>`s with click handlers would have been fewer lines and
 * unusable by half the people who have to configure a school.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  AvailabilityEntity,
  AvailabilityRule,
  AvailabilityStatus,
  Cycle,
  GridPreview,
  PeriodStructure,
  School,
} from "@evara/domain";
import {
  clearAvailability,
  describeSetupError,
  listAvailability,
  listCycles,
  listPeriodStructures,
  previewGrid,
  setAvailability,
} from "../../ipc/setup";
import { Banner, EmptyState, Loading } from "../ui/Shell";
import { useList } from "./useList";

/**
 * What a cell cycles through on Enter, Space or a click.
 *
 * `null` is available — the absence of a rule — and it is first so that the common edit,
 * blocking one slot, is a single press from the default state.
 */
const ROTATION: readonly (AvailabilityStatus | null)[] = [
  null,
  "UNAVAILABLE",
  "DISCOURAGED",
  "PREFERRED",
];

/** What each status is called on screen. */
const STATUS_LABEL: Record<AvailabilityStatus, string> = {
  UNAVAILABLE: "unavailable",
  DISCOURAGED: "discouraged",
  PREFERRED: "preferred",
};

/** The class that colours a cell. */
function cellClass(status: AvailabilityStatus | null): string {
  if (status === null) return "availCell availCell--free";
  return `availCell availCell--${status.toLowerCase()}`;
}

/** The glyph inside a cell. Never the only carrier of meaning — the label says it too. */
function cellMark(status: AvailabilityStatus | null): string {
  switch (status) {
    case "UNAVAILABLE":
      return "✕";
    case "DISCOURAGED":
      return "−";
    case "PREFERRED":
      return "★";
    default:
      return "";
  }
}

/** Key for the rule map: one recurring cell. */
function cellKey(cycleDayId: string, periodId: string): string {
  return `${cycleDayId}|${periodId}`;
}

export function AvailabilityGrid(props: {
  school: School;
  entityKind: AvailabilityEntity;
  entityId: string;
  /** The person, room or group being configured, for the spoken labels. */
  entityName: string;
}) {
  const { school, entityKind, entityId, entityName } = props;

  const loadCycles = useCallback(() => listCycles(), []);
  const cycles = useList<Cycle>(loadCycles);

  const loadStructures = useCallback(() => listPeriodStructures(), []);
  const structures = useList<PeriodStructure>(loadStructures);

  const [cycleId, setCycleId] = useState<string | null>(null);
  const [structureId, setStructureId] = useState<string | null>(null);
  const [preview, setPreview] = useState<GridPreview | null>(null);
  const [rules, setRules] = useState<readonly AvailabilityRule[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [focused, setFocused] = useState<{ day: number; period: number }>({
    day: 0,
    period: 0,
  });

  const grid = useRef<HTMLTableElement | null>(null);

  // Default to the school's own default cycle and bell schedule, so the common case needs
  // no choosing. Only defaults — the user can still pick another.
  useEffect(() => {
    if (cycleId === null && cycles.items && cycles.items.length > 0) {
      setCycleId((cycles.items.find((c) => c.isDefault) ?? cycles.items[0])?.id ?? null);
    }
  }, [cycleId, cycles.items]);

  useEffect(() => {
    if (structureId === null && structures.items && structures.items.length > 0) {
      setStructureId(
        (structures.items.find((s) => s.isDefault) ?? structures.items[0])?.id ?? null,
      );
    }
  }, [structureId, structures.items]);

  const reloadRules = useCallback(() => {
    void listAvailability(entityKind, entityId)
      .then((rows) => {
        setRules(rows);
        setError(null);
      })
      .catch((cause: unknown) => {
        setRules([]);
        setError(describeSetupError(cause));
      });
  }, [entityKind, entityId]);

  useEffect(() => {
    reloadRules();
  }, [reloadRules]);

  useEffect(() => {
    if (cycleId === null || structureId === null) {
      setPreview(null);
      return;
    }
    let live = true;
    void previewGrid({
      cycleId,
      periodStructureId: structureId,
      termId: null,
      withoutCycleDays: [],
      withoutPeriods: [],
    })
      .then((value) => {
        if (live) setPreview(value);
      })
      .catch((cause: unknown) => {
        if (live) setError(describeSetupError(cause));
      });
    return () => {
      live = false;
    };
  }, [cycleId, structureId]);

  /** Only the recurring cell rules — what this grid can show and edit. */
  const byCell = useMemo(() => {
    const map = new Map<string, AvailabilityRule>();
    for (const rule of rules ?? []) {
      if (rule.cycleDayId !== null && rule.periodId !== null && rule.timeslotId === null) {
        map.set(cellKey(rule.cycleDayId, rule.periodId), rule);
      }
    }
    return map;
  }, [rules]);

  /**
   * Rules this grid is not showing: whole-day, whole-period, exact-slot or term-scoped.
   *
   * The schema supports them and the solver will read them; this editor writes only
   * recurring cells. Saying so is the difference between an editor that is simple and one
   * that silently misrepresents what is configured.
   */
  const otherRules = useMemo(
    () =>
      (rules ?? []).filter(
        (rule) =>
          rule.timeslotId !== null || rule.cycleDayId === null || rule.periodId === null,
      ),
    [rules],
  );

  const write = useCallback(
    (cycleDayId: string, periodId: string, next: AvailabilityStatus | null) => {
      setBusy(true);
      const scope = {
        schoolId: school.id,
        entityKind,
        entityId,
        timeslotId: null,
        cycleDayId,
        periodId,
        termId: null,
        status: next ?? "UNAVAILABLE",
        weight: null,
        reason: null,
        effectiveFrom: null,
        effectiveTo: null,
      } as const;

      const action =
        next === null ? clearAvailability({ ...scope }) : setAvailability({ ...scope });

      void action
        .then(() => reloadRules())
        .catch((cause: unknown) => setError(describeSetupError(cause)))
        .finally(() => setBusy(false));
    },
    [entityId, entityKind, reloadRules, school.id],
  );

  const advance = useCallback(
    (cycleDayId: string, periodId: string) => {
      const current = byCell.get(cellKey(cycleDayId, periodId))?.status ?? null;
      const index = ROTATION.indexOf(current);
      const next = ROTATION[(index + 1) % ROTATION.length] ?? null;
      write(cycleDayId, periodId, next);
    },
    [byCell, write],
  );

  const days = preview?.days ?? [];
  const periods = preview?.periods ?? [];

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent, day: number, period: number) => {
      const moves: Record<string, [number, number]> = {
        ArrowRight: [day + 1, period],
        ArrowLeft: [day - 1, period],
        ArrowDown: [day, period + 1],
        ArrowUp: [day, period - 1],
      };
      const move = moves[event.key];
      if (!move) return;

      event.preventDefault();
      const [nextDay, nextPeriod] = move;
      if (nextDay < 0 || nextDay >= days.length) return;
      if (nextPeriod < 0 || nextPeriod >= periods.length) return;

      setFocused({ day: nextDay, period: nextPeriod });
      const selector = `[data-cell="${nextDay}-${nextPeriod}"]`;
      grid.current?.querySelector<HTMLButtonElement>(selector)?.focus();
    },
    [days.length, periods.length],
  );

  if (cycles.items !== null && cycles.items.length === 0) {
    return (
      <EmptyState title="No cycle yet">
        Availability is set against a cycle's days. Create one under “Cycles and cycle days”
        first.
      </EmptyState>
    );
  }
  if (structures.items !== null && structures.items.length === 0) {
    return (
      <EmptyState title="No bell schedule yet">
        Availability is set against a bell schedule's periods. Create one under “Bell
        schedules and periods” first.
      </EmptyState>
    );
  }

  return (
    <div className="availability">
      <div className="availabilityControls">
        <label className="field">
          <span className="fieldLabel">Cycle</span>
          <select
            value={cycleId ?? ""}
            onChange={(event) => setCycleId(event.target.value || null)}
            aria-label="Cycle to show"
          >
            {(cycles.items ?? []).map((cycle) => (
              <option key={cycle.id} value={cycle.id}>
                {cycle.name}
              </option>
            ))}
          </select>
        </label>

        <label className="field">
          <span className="fieldLabel">Bell schedule</span>
          <select
            value={structureId ?? ""}
            onChange={(event) => setStructureId(event.target.value || null)}
            aria-label="Bell schedule to show"
          >
            {(structures.items ?? []).map((structure) => (
              <option key={structure.id} value={structure.id}>
                {structure.name}
              </option>
            ))}
          </select>
        </label>
      </div>

      <p className="muted">
        Every slot is available until you say otherwise. Choose a cell and press Enter or
        Space to move it through unavailable, discouraged and preferred, then back to
        available. Arrow keys move around the grid.
      </p>

      {error && <Banner tone="error">{error}</Banner>}

      {preview === null || rules === null ? (
        <Loading what="availability" />
      ) : days.length === 0 || periods.length === 0 ? (
        <EmptyState title="Nothing to show yet">
          This cycle has no days, or this bell schedule has no periods. Add them first and
          the grid will follow.
        </EmptyState>
      ) : (
        <div className="availabilityScroll">
          <table className="availabilityTable" ref={grid}>
            <caption className="srOnly">
              Availability for {entityName}. Rows are periods, columns are cycle days.
            </caption>
            <thead>
              <tr>
                <th scope="col">Period</th>
                {days.map((day) => (
                  <th key={day.id} scope="col">
                    {day.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {periods.map((period, periodIndex) => (
                <tr key={period.id}>
                  <th scope="row">
                    {period.label}
                    <span className="availTime">
                      {period.startsAt}–{period.endsAt}
                    </span>
                  </th>
                  {days.map((day, dayIndex) => {
                    const status = byCell.get(cellKey(day.id, period.id))?.status ?? null;
                    const isTabStop =
                      focused.day === dayIndex && focused.period === periodIndex;
                    return (
                      <td key={day.id}>
                        <button
                          type="button"
                          data-cell={`${dayIndex}-${periodIndex}`}
                          className={cellClass(status)}
                          disabled={busy}
                          // One tab stop for the whole grid; arrows move within it. A grid
                          // of fifty separate tab stops is technically reachable and
                          // practically unusable.
                          tabIndex={isTabStop ? 0 : -1}
                          aria-label={`${entityName}, ${day.label}, ${period.label}: ${
                            status === null ? "available" : STATUS_LABEL[status]
                          }`}
                          onFocus={() => setFocused({ day: dayIndex, period: periodIndex })}
                          onKeyDown={(event) => onKeyDown(event, dayIndex, periodIndex)}
                          onClick={() => advance(day.id, period.id)}
                        >
                          <span aria-hidden="true">{cellMark(status)}</span>
                        </button>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <ul className="availabilityKey">
        <li>
          <span className="availCell availCell--free" aria-hidden="true" /> Available
        </li>
        <li>
          <span className="availCell availCell--unavailable" aria-hidden="true">
            ✕
          </span>{" "}
          Unavailable
        </li>
        <li>
          <span className="availCell availCell--discouraged" aria-hidden="true">
            −
          </span>{" "}
          Discouraged
        </li>
        <li>
          <span className="availCell availCell--preferred" aria-hidden="true">
            ★
          </span>{" "}
          Preferred
        </li>
      </ul>

      {otherRules.length > 0 && (
        <Banner tone="warning">
          <strong>
            {otherRules.length} rule{otherRules.length === 1 ? "" : "s"} are not shown above.
          </strong>{" "}
          They cover a whole day, a whole period, one exact slot or a single term, which
          this editor does not write. They are still in effect and are left untouched by
          anything you do here.
        </Banner>
      )}
    </div>
  );
}

/**
 * The availability editor with its own "which one?" picker.
 *
 * Exists so Teachers, Rooms and Student groups each add availability with one element
 * rather than three, and so the choice of *who* is being configured is presented the same
 * way on all three screens. The grid itself knows nothing about picking.
 */
export function AvailabilityPanel(props: {
  school: School;
  entityKind: AvailabilityEntity;
  /** What this kind is called, singular and lowercase: "teacher". */
  noun: string;
  /** The rows that can be configured, already in display order. */
  options: readonly { readonly id: string; readonly name: string }[];
}) {
  const { school, entityKind, noun, options } = props;
  const [selected, setSelected] = useState<string>("");

  const chosen = options.find((option) => option.id === selected) ?? null;

  if (options.length === 0) return null;

  return (
    <div className="panel">
      <div className="panelHead">
        <h3>Availability</h3>
      </div>

      <label className="field">
        <span className="fieldLabel">{noun[0]?.toUpperCase()}{noun.slice(1)}</span>
        <select
          value={selected}
          onChange={(event) => setSelected(event.target.value)}
          aria-label={`${noun} to set availability for`}
        >
          <option value="">Choose a {noun}…</option>
          {options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.name}
            </option>
          ))}
        </select>
      </label>

      {chosen === null ? (
        <EmptyState title={`No ${noun} chosen`}>
          Choose a {noun} to see when they can be timetabled. Everything is available until
          you say otherwise, so there is nothing to configure unless there are exceptions.
        </EmptyState>
      ) : (
        <AvailabilityGrid
          school={school}
          entityKind={entityKind}
          entityId={chosen.id}
          entityName={chosen.name}
        />
      )}
    </div>
  );
}
