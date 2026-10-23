/**
 * Choosing whose timetable to look at, and fetching the grid the backend lays out for it.
 *
 * ## Five scopes, and one of them is labelled differently
 *
 * | Scope | Is |
 * | --- | --- |
 * | All lessons | every occurrence, ungrouped. Several per cell is ordinary here |
 * | Class | one class section's own timetable |
 * | Teacher | one teacher's load |
 * | Room | one room's bookings |
 * | Student | a **roster-derived** view, and said to be one |
 *
 * The student view is the one that needs a caveat on screen rather than only in a record.
 * ADR 0006 makes a full timetable buildable with no student records at all, so
 * `class_enrolment` is optional and a student's grid shows **exactly** the sections an
 * enrolment row names. An empty one may mean nobody entered a roster rather than that the
 * child has no lessons, and the picker says so instead of letting a blank grid imply the
 * second.
 *
 * Nothing here substitutes a group's timetable for a student's. That would present a guess
 * as a child's confirmed schedule, which is the one thing this view must never do.
 *
 * ## Day navigation
 *
 * The columns are the cycle's days in `ordinal` order — never a week, and nothing here reads
 * a weekday. A cycle of more than {@link WHOLE_CYCLE_LIMIT} days defaults to one day at a
 * time, because a sixteen-column table is unreadable before it is wrong. That threshold is a
 * display preference and not a rule.
 */
import { useEffect, useMemo, useState } from "react";

import type { ProjectedDay, ProjectionScope, ScopeGrid, ScopeKind } from "@evara/domain";

import { describeTimetableError } from "../../ipc/timetables";

/** Above this many cycle days, one day at a time is the default view. */
export const WHOLE_CYCLE_LIMIT = 10;

/** The scope kinds a picker offers, in the order a timetabler asks for them. */
const KINDS: ReadonlyArray<{ kind: ScopeKind; label: string; plural: string }> = [
  { kind: "master", label: "All lessons", plural: "All lessons" },
  { kind: "section", label: "Class", plural: "Classes" },
  { kind: "teacher", label: "Teacher", plural: "Teachers" },
  { kind: "room", label: "Room", plural: "Rooms" },
  { kind: "student", label: "Student", plural: "Students" },
];

export function ScopePicker(props: {
  scopes: readonly ProjectionScope[];
  kind: ScopeKind;
  id: string | null;
  hasEnrolments: boolean;
  onChange: (kind: ScopeKind, id: string | null) => void;
}) {
  const { scopes, kind, id, hasEnrolments, onChange } = props;

  const available = useMemo(
    () =>
      KINDS.filter(
        (entry) => entry.kind === "master" || scopes.some((s) => s.kind === entry.kind),
      ),
    [scopes],
  );
  const options = useMemo(
    () => scopes.filter((scope) => scope.kind === kind),
    [scopes, kind],
  );

  return (
    <div className="ttScopePicker">
      <div className="ttScopeKinds" role="group" aria-label="What to show">
        {available.map((entry) => (
          <button
            key={entry.kind}
            type="button"
            className={`ttScopeKind${kind === entry.kind ? " ttScopeKind--on" : ""}`}
            aria-pressed={kind === entry.kind}
            onClick={() => {
              const first = scopes.find((scope) => scope.kind === entry.kind);
              onChange(entry.kind, entry.kind === "master" ? null : (first?.id ?? null));
            }}
          >
            {entry.plural}
          </button>
        ))}
      </div>

      {kind === "master" ? null : (
        <label className="ttScopeChoose">
          <span>{KINDS.find((entry) => entry.kind === kind)?.label ?? "Which"}</span>
          <select
            value={id ?? ""}
            onChange={(event) => onChange(kind, event.target.value || null)}
          >
            {options.map((scope) => (
              <option key={scope.id ?? "none"} value={scope.id ?? ""}>
                {scope.name}
                {scope.secondary === null ? "" : ` — ${scope.secondary}`}
                {` (${scope.lessonCount} lesson${scope.lessonCount === 1 ? "" : "s"})`}
              </option>
            ))}
          </select>
        </label>
      )}

      {kind === "student" ? (
        <p className="ttRosterNote" role="note">
          {hasEnrolments
            ? "A student's timetable is built from class enrolment records only. If a roster" +
              " is incomplete, so is this view — it never fills a gap from a class group."
            : "No class enrolment records have been entered, so no student timetable can be" +
              " derived. A class group's timetable is not shown in a student's place."}
        </p>
      ) : null}
    </div>
  );
}

/** How one day of a panel is chosen, when the whole cycle is too wide to read. */
export function DayStrip(props: {
  days: readonly ProjectedDay[];
  shown: number | null;
  onChange: (ordinal: number | null) => void;
}) {
  const { days, shown, onChange } = props;
  if (days.length === 0) return null;

  return (
    <div className="ttDayStrip" role="group" aria-label="Which day to show">
      <button
        type="button"
        className={`ttDay${shown === null ? " ttDay--on" : ""}`}
        aria-current={shown === null ? "true" : undefined}
        onClick={() => onChange(null)}
      >
        Whole cycle
      </button>
      {days.map((day) => (
        <button
          key={day.cycleDayId}
          type="button"
          className={`ttDay${shown === day.ordinal ? " ttDay--on" : ""}`}
          aria-current={shown === day.ordinal ? "true" : undefined}
          onClick={() => onChange(day.ordinal)}
        >
          {day.label}
        </button>
      ))}
    </div>
  );
}

/** What loading one scope's grid produced. */
export type ScopeLoad = {
  grid: ScopeGrid | null;
  loading: boolean;
  error: string | null;
};

/**
 * Fetches the grid for one scope, through whichever command the caller supplies.
 *
 * The loader is a parameter because a generated preview and a saved timetable are addressed
 * differently — by run and by timetable — while the grid they produce is the same type laid
 * out by the same Rust function. Keeping the fetch here means neither screen reimplements
 * the request, and keeping the *layout* in Rust means neither reimplements the `rowSpan`.
 */
export function useScopeLoad(
  load: ((kind: ScopeKind, id: string | null) => Promise<ScopeGrid>) | null,
  kind: ScopeKind,
  id: string | null,
): ScopeLoad {
  const [grid, setGrid] = useState<ScopeGrid | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (load === null) {
      setGrid(null);
      return;
    }
    let live = true;
    setLoading(true);
    setError(null);
    load(kind, id)
      .then((found) => {
        if (live) setGrid(found);
      })
      .catch((e: unknown) => {
        if (live) {
          setGrid(null);
          setError(describeTimetableError(e));
        }
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [load, kind, id]);

  return { grid, loading, error };
}
