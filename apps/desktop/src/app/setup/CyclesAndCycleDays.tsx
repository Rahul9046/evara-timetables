/**
 * Section 6 — Cycles and Cycle Days.
 *
 * ## Two kinds of cycle, and one thing they have in common
 *
 * A cycle is **weekly** or **custom**, and the difference is who decides what its
 * positions are. A weekly cycle *is* the week: seven positions, Monday through Sunday,
 * created with the cycle and not managed by hand. A custom cycle is a rotation of whatever
 * length the school repeats, with labels the user chooses.
 *
 * What they have in common is the thing this screen exists to protect: **a cycle is a
 * reusable definition and holds nothing about any school's calendar.** No start date, no
 * starting rotation position, no working week, no holiday policy. Those are per-school
 * facts and they live in Calendar Mapping, which is the only bridge from real dates to the
 * model (ADR 0011). It is what lets one cycle be shared by two schools, or by two campuses
 * keeping different weeks, and it is why the weekly cycle stores all seven days rather
 * than the five this school happens to teach.
 *
 * Which of those seven *apply here* is a question, not a stored value:
 * {@link WeeklyWeekPanel} asks it, sending the school's working days with the question and
 * getting an answer back that changes nothing. Change the working days in School Details
 * and this screen says something different; no cycle, day, timeslot or mapping moves.
 *
 * ## The cycle is still not a week
 *
 * A cycle day's `ordinal` is its whole scheduling identity, and that is unchanged by the
 * weekly kind. A weekly position is *named* by the calendar — position 1 is Monday — but
 * nothing schedules by it: no date acquires a cycle day from a weekday, `weekdayHint`
 * remains display, and the grid, the materialiser and the solver read `ordinal` alone. See
 * `evara_core::calendar::WeeklyPosition` for the distinction written out in full.
 *
 * ## Nothing here works anything out
 *
 * Whether a cycle is complete, where its rotation wraps, which weekday a position is and
 * whether this school teaches on it all arrive from `setup_cycle_shape` in one round trip.
 * `CLAUDE.md` names a scheduling rule in TypeScript, or any rule implemented twice, as an
 * architecture bug; this screen renders answers.
 *
 * ## The type cannot be changed after creation
 *
 * The select is disabled on an edit and the backend refuses a change outright. Converting
 * a six-day rotation into a week would mean deleting days, which `timeslot` and
 * `calendar_day` hold with `RESTRICT` precisely so that removing one is a deliberate,
 * confirmed act. The honest answer is a new cycle, and the form says so rather than
 * silently snapping the picker back.
 *
 * ## Neither `weekCount` nor `isDefault` is asked for
 *
 * `cycle.is_default` was "the school's default cycle", offered as a checkbox and used to
 * pre-select a cycle on the Timetable Grid Preview. It is no longer collected: a default
 * is only worth having once something has to choose a cycle *without* asking, and nothing
 * does — the grid screen now requires the choice to be made. Picking a cycle on the user's
 * behalf from a flag they set once, months earlier, is how the wrong grid gets rebuilt.
 *
 * "Calendar weeks spanned" (`weekCount`) was display-and-grouping only, and nothing was
 * ever grouped by it — not the materialiser, not the grid, not a report. It asked every
 * user to maintain a number the application did not consult, and it invited exactly the
 * inference this screen exists to avoid: that a cycle is a number of weeks.
 *
 * Both columns remain: `cycle.week_count` is `NOT NULL CHECK (week_count >= 1)`, a
 * released migration is immutable, and `CycleInput` still carries both. So
 * {@link CrudSpec.toDraft} carries each stored value through an edit untouched and
 * {@link weekCountFor} supplies 1 for a new cycle, which means **no existing cycle record
 * changes** and nothing is promoted to default automatically — the old form did exactly
 * that for the first cycle in a project.
 */
import { useCallback, useMemo, useState } from "react";

import type { Cycle, CycleDay, CycleKind, CycleShape, School } from "@evara/domain";
import {
  createCycle,
  createCycleDay,
  cycleShape,
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
import { describeWeek, weekdayName } from "./weekdays";
import { useList, useValue } from "./useList";

/**
 * How long a weekly cycle is, for the one value a save must still name.
 *
 * `evara_core::calendar::WEEKLY_DAY_COUNT` is the authority and the backend derives the
 * stored length from the cycle's type, ignoring whatever a caller sends. But the column is
 * `CHECK (day_count >= 1)` and `CycleInput` has no optional length, so a save has to carry
 * *some* number. Carrying the right one means a future build that stopped deriving it
 * would still store seven rather than a placeholder.
 *
 * This is the same arrangement, for the same reason, as {@link weekCountFor}.
 */
const WEEKLY_DAY_COUNT = 7;

/** What the cycle-type select offers. The values are the stored spellings. */
const KIND_CHOICES: readonly { value: CycleKind; label: string }[] = [
  { value: "WEEKLY", label: "Weekly — the days of the week" },
  { value: "CUSTOM", label: "Custom — a numbered rotation" },
];

const KIND_SHORT: Record<CycleKind, string> = {
  WEEKLY: "Weekly",
  CUSTOM: "Custom",
};

/** Whether a draft describes a weekly cycle. */
function isWeekly(draft: Draft): boolean {
  return text(draft, "kind") === "WEEKLY";
}

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

  // The working week is part of the question, not something the backend looks up. Keyed on
  // the joined numbers rather than the array, because the array arrives from JSON and is a
  // fresh object on every load — its identity would re-fetch forever.
  const workingDays = school.workingDays;
  const weekKey = workingDays.join(",");
  const loadShape = useCallback(
    () => (cycleId ? cycleShape(cycleId, workingDays) : Promise.resolve(null)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cycleId, weekKey],
  );
  const shape = useValue<CycleShape | null>(loadShape);

  const selected = cycles.items?.find((c) => c.id === cycleId) ?? null;
  const current = shape.value ?? null;
  const weekly = selected?.kind === "WEEKLY";

  const cycleSpec = useMemo<CrudSpec<Cycle, Parameters<typeof createCycle>[0]>>(
    () => ({
      noun: "cycle",
      plural: "cycles",
      title: "Cycles",
      describe: (row) => row.name,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Type", render: (row) => KIND_SHORT[row.kind] ?? row.kind },
        {
          header: "Days",
          numeric: true,
          render: (row) => row.dayCount,
        },
      ],
      // A function of the draft: a weekly cycle has no length to ask for, and offering the
      // field would invite a number the backend is going to ignore. See `CrudSpec.fields`.
      fields: (draft) => {
        const locked = flag(draft, "locked");
        const base = [
          {
            kind: "text" as const,
            key: "name",
            label: "Cycle name",
            required: true,
            placeholder: isWeekly(draft) ? "School week" : "Six-day rotation",
          },
          {
            kind: "select" as const,
            key: "kind",
            label: "Cycle type",
            required: true,
            choices: KIND_CHOICES,
            disabled: locked,
            hint: locked
              ? "A cycle's type is fixed when it is created. To change it, add a new cycle — converting one would mean removing days that a timetable grid or a mapped date may already point at."
              : "Weekly gives you the seven days of the week, created for you. Custom gives you a numbered rotation of whatever length the school repeats.",
          },
        ];

        if (isWeekly(draft)) return base;

        return [
          ...base,
          {
            kind: "number" as const,
            key: "dayCount",
            label: "Rotation length",
            required: true,
            min: 1,
            hint: "How many positions the rotation has: 3, 6, 10 — whatever the school actually repeats. Each one is created for you as Day 1, Day 2 and so on, and you can rename them below.",
          },
        ];
      },
      validate: validateCycle,
      toDraft: (row) => ({
        name: row?.name ?? "",
        kind: row?.kind ?? "CUSTOM",
        dayCount: row ? String(row.dayCount) : "5",
        // Not a field: it tells `fields` whether this is an edit, which is the one thing
        // that decides whether the type may still be chosen.
        locked: row !== null,
        // Both of these are carried in the draft with no field to render them, so an edit
        // sends each value back exactly as it was read and no input can change either.
        weekCount: row ? String(row.weekCount) : "1",
        // `false` for a new cycle, not "true if this is the first one". The old form made
        // the first cycle the default automatically; nothing does that now, because
        // nothing in the application should acquire a default nobody chose.
        isDefault: row ? row.isDefault : false,
      }),
      toInput: (draft) => {
        const kind = text(draft, "kind") as CycleKind;
        return {
          schoolId: school.id,
          name: text(draft, "name"),
          kind,
          dayCount: kind === "WEEKLY" ? WEEKLY_DAY_COUNT : number(draft, "dayCount"),
          weekCount: weekCountFor(draft),
          isDefault: flag(draft, "isDefault"),
        };
      },
      list: listCycles,
      create: createCycle,
      update: updateCycle,
      remove: deleteCycle,
      emptyHint:
        "A cycle is the pattern the timetable repeats. Most schools have one. Choose Weekly for an ordinary school week, or Custom for a rotation — either way its days are created with it.",
      deleteNote: () => (
        <p className="muted">
          A cycle that still has days, or a materialised grid, cannot be deleted. Remove
          those first.
        </p>
      ),
    }),
    [school.id],
  );

  const daySpec = useMemo<CrudSpec<CycleDay, Parameters<typeof createCycleDay>[0]>>(
    () => ({
      noun: "cycle day",
      plural: "cycle days",
      title: selected ? `Days of ${selected.name}` : "Cycle days",
      describe: (row) =>
        weekly ? row.label : `${row.label} (position ${row.ordinal})`,
      columns: weekly
        ? [
            { header: "Position", numeric: true, render: (row) => row.ordinal },
            { header: "Weekday", render: (row) => weekdayLabel(row) },
            { header: "Label", render: (row) => row.label },
          ]
        : [
            { header: "Position", numeric: true, render: (row) => row.ordinal },
            { header: "Label", render: (row) => row.label },
            {
              header: "Weekday hint",
              render: (row) =>
                row.weekdayHint === null ? <Absent /> : <span>{row.weekdayHint}</span>,
            },
          ],
      // A weekly cycle's position and weekday are fixed, so only the label is editable.
      // Offering the other two would offer a change the schema refuses: a weekly day
      // carries the weekday of its position, enforced by a trigger.
      fields: weekly
        ? [
            {
              kind: "text",
              key: "label",
              label: "Label",
              required: true,
              hint: "What this weekday is called here. Display only — the position is what the timetable uses, and it does not move.",
            },
          ]
        : [
            {
              kind: "number",
              key: "ordinal",
              label: "Position",
              required: true,
              min: 1,
              max: selected?.dayCount,
              hint: positionHint(selected, current),
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
        ordinal: row ? String(row.ordinal) : String(nextPosition(current)),
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
      emptyHint: weekly
        ? "A weekly cycle is created with all seven days. If none are listed, edit the cycle above and they will be put back."
        : "A cycle needs one day per position in its pattern. Name them however the school does — the position is what the timetable uses.",
      deleteNote: (row) =>
        weekly ? (
          <p className="muted">
            <strong>{row.label}</strong> is one of the seven days of the week. Removing it
            leaves this weekly cycle short of a day; editing the cycle above will put it
            back, as a new day with a new identifier. You only need to remove days in order
            to delete the cycle itself.
          </p>
        ) : (
          <p className="muted">
            A day still used by a timeslot or a mapped calendar date cannot be deleted. The
            Timetable Grid Preview can show you exactly what is in the way before you try.
          </p>
        ),
      blockedBy: cycleId === null ? "Select a cycle first." : null,
      // Not `blockedBy`: the table of days must stay on screen. The reason adding is
      // unavailable is that every one of those days exists.
      addBlockedBy: weekly ? WEEKLY_FULL_MESSAGE : cycleFull(current),
    }),
    [cycleId, selected, weekly, current],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Cycles and cycle days</h2>
        <p>
          The cycle is the pattern the timetable repeats — a week, or a rotation of any
          length. It is a reusable definition: it holds no dates, no term start and no
          holidays, so the same cycle can be used by schools that keep different weeks.
        </p>
      </header>

      <CrudPanel
        spec={cycleSpec}
        // The shape as well as the list. Editing a cycle changes its `dayCount`, which is
        // what decides whether the cycle is full -- and `loadShape` is keyed on the
        // selected cycle's id, which an edit does not change. Without this, lengthening a
        // cycle left "all cycle positions are filled" on screen and the Add button
        // disabled, so the one instruction that message gives would not have worked.
        onChanged={() => {
          cycles.reload();
          shape.reload();
        }}
      />

      <hr />

      <ParentPicker
        legend="Cycle"
        items={
          cycles.items?.map((cycle) => ({
            id: cycle.id,
            label: cycle.name,
            detail:
              cycle.kind === "WEEKLY"
                ? "Weekly — 7 days"
                : `Custom — ${cycle.dayCount} ${cycle.dayCount === 1 ? "day" : "days"}`,
          })) ?? null
        }
        error={cycles.error}
        selectedId={cycleId}
        onSelect={setCycleId}
        empty={{
          title: "No cycles yet",
          hint: "Add a cycle above. Its days are created with it — weekly gives you Monday to Sunday, custom gives you Day 1 to Day N.",
        }}
      >
        {shape.error !== null ? (
          <Banner tone="error">{shape.error}</Banner>
        ) : selected !== null && current !== null ? (
          current.kind === "WEEKLY" ? (
            <WeeklyWeekPanel shape={current} workingDays={workingDays} />
          ) : (
            <>
              <RotationPanel cycle={selected} shape={current} />
              <CoveragePanel
                cycle={selected}
                shape={current}
                onFilled={() => {
                  shape.reload();
                  cycles.reload();
                }}
              />
            </>
          )
        ) : null}

        <CrudPanel
          spec={daySpec}
          reloadKey={cycleId ?? ""}
          onChanged={shape.reload}
        />
      </ParentPicker>
    </div>
  );
}

/** A weekly day's weekday, named. `weekdayHint` is the stored copy of its position. */
function weekdayLabel(row: CycleDay): React.ReactNode {
  return row.weekdayHint === null ? <Absent /> : <span>{weekdayName(row.weekdayHint)}</span>;
}

/** What a weekly cycle says when asked for another day. */
export const WEEKLY_FULL_MESSAGE =
  "A weekly cycle has the seven days of the week, created with it. There is no position to add.";

/**
 * The week a weekly cycle holds, and which of it this school teaches on.
 *
 * ## Why all seven are shown
 *
 * The cycle is shared. Storing only this school's five working days would bind the shared
 * definition to this school's settings, and the moment a second school with a six-day week
 * used it one of the two would be wrong with no record of which. So the definition is the
 * whole week and this panel marks the part that applies — which is a *comparison*, made by
 * `evara_core::calendar::WeeklyPosition::applies_to` against the working days sent with the
 * request, stored nowhere.
 *
 * Change the working days in School Details and only this panel changes. No cycle, no day,
 * no timeslot and no calendar mapping is created, removed or rewritten — the same rule
 * Calendar Mapping's working-day warning follows, and for the same reason: a school really
 * might teach one Saturday, and an application that tidied that away would destroy a
 * deliberate decision.
 */
function WeeklyWeekPanel(props: { shape: CycleShape; workingDays: readonly number[] }) {
  const { shape, workingDays } = props;
  const applying = shape.weekdays.filter((day) => day.applies);
  const missing = shape.weekdays.filter((day) => day.cycleDayId === null);

  return (
    <Panel title="The week">
      <Banner tone="info">
        This cycle's days are <strong>managed automatically</strong>: it holds all seven
        days of the week, Monday to Sunday, created with the cycle. There is nothing to add
        and no length to set.
      </Banner>

      <p>
        This school works <strong>{describeWeek(workingDays)}</strong>, so{" "}
        <strong>
          {applying.length} of the {shape.weekdays.length}
        </strong>{" "}
        days apply here. The other {shape.weekdays.length - applying.length} stay in the
        cycle so that a school or campus keeping a different week can use the same
        definition. To change which days apply, change the working days in School Details —
        nothing in this cycle is altered by it.
      </p>

      <div className="tableScroll">
        <table className="dataTable">
          <caption className="srOnly">
            The seven days of this weekly cycle, and whether this school teaches on each
          </caption>
          <thead>
            <tr>
              <th scope="col" className="num">
                Position
              </th>
              <th scope="col">Weekday</th>
              <th scope="col">Label</th>
              <th scope="col">Applies here</th>
            </tr>
          </thead>
          <tbody>
            {shape.weekdays.map((day) => (
              <tr key={day.ordinal}>
                <td className="num">{day.ordinal}</td>
                <td>{weekdayName(day.weekday)}</td>
                <td>{day.label ?? <Absent />}</td>
                <td>
                  {day.applies ? (
                    <span>Yes</span>
                  ) : (
                    <span className="muted">Not a working day here</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {missing.length > 0 && (
        <Banner tone="warning">
          <strong>
            {missing.length === 1
              ? "One day of the week is missing"
              : `${missing.length} days of the week are missing`}
            :
          </strong>{" "}
          {missing.map((day) => weekdayName(day.weekday)).join(", ")}. A weekly cycle should
          have all seven. Edit the cycle above and the missing days will be created again —
          as new days with new identifiers, so anything that pointed at the old ones will
          need remapping.
        </Banner>
      )}
    </Panel>
  );
}

/**
 * The rotation a custom cycle runs in, and where it comes back round.
 *
 * The sequence and the wrap come from `evara_core::time::CycleRotation` over IPC. That
 * matters for one small rule that is easy to get wrong in a renderer: what follows the last
 * day. Deciding it here — `(n % count) + 1`, probably — would be the second implementation
 * of it, in the layer `CLAUDE.md` forbids rules in.
 *
 * Labels come from the stored days, so a renamed Day 3 reads as what it is now called. A
 * position with no day yet is shown by number, because the position is what exists.
 */
function RotationPanel(props: { cycle: Cycle; shape: CycleShape }) {
  const { cycle, shape } = props;
  const { rotation } = shape;

  const loadDays = useCallback(() => listCycleDays(cycle.id), [cycle.id]);
  const days = useList<CycleDay>(loadDays);
  const labels = new Map((days.items ?? []).map((day) => [day.ordinal, day.label]));
  const nameOf = (ordinal: number) => labels.get(ordinal) ?? `Position ${ordinal}`;

  if (rotation.sequence.length === 0) return null;

  return (
    <Panel title="Rotation">
      <p className="rotationPreview">
        {rotation.sequence.map((ordinal, index) => (
          <span key={ordinal}>
            {index > 0 && <span aria-hidden="true"> → </span>}
            <span className={labels.has(ordinal) ? undefined : "muted"}>
              {nameOf(ordinal)}
            </span>
          </span>
        ))}
        {rotation.wrapsTo !== null && (
          <span>
            <span aria-hidden="true"> → </span>
            <span className="muted">{nameOf(rotation.wrapsTo)}</span>
          </span>
        )}
      </p>
      <p className="hint">
        {rotation.wrapsTo === null
          ? "This cycle has no positions yet."
          : `After ${nameOf(rotation.sequence[rotation.sequence.length - 1] as number)} the rotation comes back round to ${nameOf(rotation.wrapsTo)}. Which real date is which position is set in Calendar Mapping — this cycle holds no dates.`}
      </p>
    </Panel>
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
function positionHint(cycle: Cycle | null, shape: CycleShape | null): string {
  const identity = "This is the day's identity — everything scheduled points at it.";
  if (!cycle) return `From 1. ${identity}`;

  const free = shape?.coverage.missingOrdinals ?? [];
  if (free.length === 0 || free.length === cycle.dayCount) {
    return `From 1 to ${cycle.dayCount}. ${identity}`;
  }
  const named = free.length <= 4 ? free.join(", ") : `${free.slice(0, 4).join(", ")} and more`;
  return `From 1 to ${cycle.dayCount}; still free: ${named}. ${identity}`;
}

/** The position a new day should default to: the first gap, or one past the end. */
function nextPosition(shape: CycleShape | null): number {
  if (!shape) return 1;
  return shape.coverage.missingOrdinals[0] ?? shape.coverage.present + 1;
}

/** What a full cycle says when asked for another day. */
export const CYCLE_FULL_MESSAGE =
  "All cycle positions are filled. Increase the rotation length to add more days.";

/**
 * Why no further day can be added, or `null` while one still can.
 *
 * Answered from {@link CycleCoverage.missingOrdinals}, which `evara_core::time` computed
 * and the backend handed over. That matters: "is this cycle full?" is a question about the
 * model, and deciding it here by comparing a row count against `dayCount` would be the
 * same rule implemented twice, in the layer `CLAUDE.md` forbids it in. The screen reads an
 * answer; it does not work one out.
 *
 * `null` shape means it has not arrived yet, which is not the same as "full" — disabling
 * the button then would flash it off and on during every load.
 */
function cycleFull(shape: CycleShape | null): string | null {
  if (!shape) return null;
  return shape.coverage.missingOrdinals.length === 0 ? CYCLE_FULL_MESSAGE : null;
}

/**
 * Says plainly whether a custom cycle is finished, and what is missing if it is not.
 *
 * Creating a cycle now lays out every position, so a complete cycle is the normal case and
 * this usually reports success. It still earns its place: a project made before the cycle
 * type existed may have gaps, and a day removed later leaves one. The consequence is
 * downstream and invisible — the grid will build, it will simply be short of a day, and
 * nothing later would say why — so it is a banner rather than a quiet badge.
 *
 * Not shown for a weekly cycle: its positions are not the user's to fill, and
 * {@link WeeklyWeekPanel} names a missing weekday in the terms that apply to one.
 */
function CoveragePanel(props: {
  cycle: Cycle;
  shape: CycleShape;
  onFilled: () => void;
}) {
  const { cycle, shape, onFilled } = props;
  const coverage = shape.coverage;
  const [busy, setBusy] = useState(false);
  const [fillError, setFillError] = useState<string | null>(null);

  const fill = useCallback(async () => {
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
