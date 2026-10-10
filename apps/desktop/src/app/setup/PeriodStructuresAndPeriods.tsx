/**
 * Section 7 — Period Structures and Periods.
 *
 * A period structure is a bell schedule. The brief asks for "multiple documented period
 * structures", which the model supports by scoping each one to a campus — or to no campus
 * at all, meaning school-wide. That is also what makes two grids over one cycle work: a
 * school with two campuses on different bells has two schedules, and materialising one
 * never reports the other's slots as orphans.
 *
 * Periods are ordered by `ordinal`, which is their identity within the day, not by their
 * start time. The two usually agree; when they do not, the table says so rather than
 * silently re-sorting, because the model's order is the one the grid is built from.
 */
import { useCallback, useMemo, useState } from "react";

import type { Campus, Period, PeriodStructure, School } from "@evara/domain";
import {
  createPeriod,
  createPeriodStructure,
  deletePeriod,
  deletePeriodStructure,
  listCampuses,
  listPeriodStructures,
  listPeriods,
  updatePeriod,
  updatePeriodStructure,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { Banner } from "../ui/Shell";
import {
  flag,
  number,
  text,
  textOrNull,
  validatePeriod,
  validatePeriodStructure,
} from "../validation";
import { Absent } from "./CampusesAndBuildings";
import { ParentPicker } from "./ParentPicker";
import { useList } from "./useList";

/** The period kinds, with wording a user recognises. Mirrors `PeriodKind` in Rust. */
const KIND_CHOICES = [
  { value: "TEACHING", label: "Teaching — lessons may be scheduled" },
  { value: "BREAK", label: "Break — recess or lunch" },
  { value: "REGISTRATION", label: "Registration — form time, roll call" },
  { value: "OTHER", label: "Other" },
] as const;

const KIND_SHORT: Record<string, string> = {
  TEACHING: "Teaching",
  BREAK: "Break",
  REGISTRATION: "Registration",
  OTHER: "Other",
};

export function PeriodStructuresAndPeriods(props: { school: School }) {
  const { school } = props;
  const [structureId, setStructureId] = useState<string | null>(null);

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

  const loadStructures = useCallback(() => listPeriodStructures(), []);
  const structures = useList<PeriodStructure>(loadStructures);

  const loadPeriods = useCallback(
    () => (structureId ? listPeriods(structureId) : Promise.resolve([])),
    [structureId],
  );
  const periods = useList<Period>(loadPeriods);

  const selected = structures.items?.find((s) => s.id === structureId) ?? null;

  const structureSpec = useMemo<
    CrudSpec<PeriodStructure, Parameters<typeof createPeriodStructure>[0]>
  >(() => {
    const choices = (campuses.items ?? []).map((c) => ({ value: c.id, label: c.name }));
    const campusName = new Map(choices.map((c) => [c.value, c.label]));

    return {
      noun: "bell schedule",
      plural: "bell schedules",
      title: "Bell schedules",
      describe: (row) => row.name,
      columns: [
        { header: "Name", render: (row) => row.name },
        {
          header: "Applies to",
          render: (row) =>
            row.campusId ? (
              (campusName.get(row.campusId) ?? <Absent />)
            ) : (
              <span className="muted">Whole school</span>
            ),
        },
        {
          header: "Default",
          render: (row) => (row.isDefault ? <span className="yes">Yes</span> : <Absent />),
        },
      ],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Schedule name",
          required: true,
          placeholder: "Standard day",
        },
        {
          kind: "select",
          key: "campusId",
          label: "Applies to",
          emptyLabel: "Whole school",
          choices,
          hint: "Scope it to a campus when sites ring different bells. Each scope has its own grid over the cycle.",
        },
        {
          kind: "checkbox",
          key: "isDefault",
          label: "Default for its scope",
          hint: "At most one default per campus, and school-wide counts as its own scope.",
        },
      ],
      validate: validatePeriodStructure,
      toDraft: (row) => ({
        name: row?.name ?? "",
        campusId: row?.campusId ?? "",
        isDefault: row ? row.isDefault : (structures.items?.length ?? 0) === 0,
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        campusId: textOrNull(draft, "campusId"),
        name: text(draft, "name"),
        isDefault: flag(draft, "isDefault"),
      }),
      list: listPeriodStructures,
      create: createPeriodStructure,
      update: updatePeriodStructure,
      remove: deletePeriodStructure,
      emptyHint:
        "A bell schedule is the shape of a single day: period 1, period 2, break, and so on. Add one, then its periods — the timetable grid is the cycle's days crossed with this schedule's periods.",
      deleteNote: () => (
        <p className="muted">A schedule that still has periods cannot be deleted.</p>
      ),
    };
  }, [school.id, campuses.items, structures.items]);

  const periodSpec = useMemo<CrudSpec<Period, Parameters<typeof createPeriod>[0]>>(
    () => ({
      noun: "period",
      plural: "periods",
      title: selected ? `Periods of ${selected.name}` : "Periods",
      describe: (row) => `${row.label} (${row.startsAt}–${row.endsAt})`,
      columns: [
        { header: "Position", numeric: true, render: (row) => row.ordinal },
        { header: "Label", render: (row) => row.label },
        { header: "Starts", numeric: true, render: (row) => <time>{row.startsAt}</time> },
        { header: "Ends", numeric: true, render: (row) => <time>{row.endsAt}</time> },
        {
          header: "Kind",
          render: (row) => (
            <span className={row.kind === "TEACHING" ? "kind kind--teaching" : "kind"}>
              {KIND_SHORT[row.kind] ?? row.kind}
            </span>
          ),
        },
        {
          header: "Counts as load",
          render: (row) => (row.countsAsLoad ? <span className="yes">Yes</span> : <Absent />),
        },
      ],
      fields: [
        {
          kind: "number",
          key: "ordinal",
          label: "Position",
          required: true,
          min: 1,
          hint: "Order within the day, from 1. This is the period's identity; inserting one renumbers the grid without changing any slot's identifier.",
        },
        {
          kind: "text",
          key: "label",
          label: "Label",
          required: true,
          placeholder: "P1",
        },
        { kind: "time", key: "startsAt", label: "Starts at", required: true },
        { kind: "time", key: "endsAt", label: "Ends at", required: true },
        {
          kind: "select",
          key: "kind",
          label: "Kind",
          required: true,
          choices: KIND_CHOICES,
          hint: "Only teaching periods become schedulable slots in the grid.",
        },
        {
          kind: "checkbox",
          key: "countsAsLoad",
          label: "Counts towards a teacher's load",
        },
      ],
      validate: validatePeriod,
      toDraft: (row) => ({
        ordinal: row ? String(row.ordinal) : String((periods.items?.length ?? 0) + 1),
        label: row?.label ?? "",
        startsAt: row?.startsAt ?? "",
        endsAt: row?.endsAt ?? "",
        kind: row?.kind ?? "TEACHING",
        countsAsLoad: row ? row.countsAsLoad : true,
      }),
      toInput: (draft) => ({
        periodStructureId: structureId ?? "",
        ordinal: number(draft, "ordinal"),
        label: text(draft, "label"),
        startsAt: text(draft, "startsAt"),
        endsAt: text(draft, "endsAt"),
        // The select's values are exactly the stored spellings, so no mapping is needed
        // and none is invented. The cast is narrowing a string to the generated union.
        kind: text(draft, "kind") as Period["kind"],
        countsAsLoad: flag(draft, "countsAsLoad"),
      }),
      list: loadPeriods,
      create: createPeriod,
      update: updatePeriod,
      remove: deletePeriod,
      emptyHint:
        "Add every period of the day, breaks included. Breaks keep the grid honest about when the school is actually running, even though nothing is scheduled into them.",
      deleteNote: () => (
        <p className="muted">
          A period a timeslot was built from cannot be deleted. Use the Timetable Grid
          Preview to see exactly which slots stand in the way first.
        </p>
      ),
      blockedBy: structureId === null ? "Select a bell schedule first." : null,
    }),
    [structureId, selected, periods.items, loadPeriods],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Bell schedules and periods</h2>
        <p>
          A bell schedule is one day's shape. Scope it to a campus when sites differ, or
          leave it school-wide.
        </p>
      </header>

      <CrudPanel spec={structureSpec} onChanged={structures.reload} />

      <hr />

      <ParentPicker
        legend="Bell schedule"
        items={
          structures.items?.map((structure) => ({
            id: structure.id,
            label: structure.name,
            isDefault: structure.isDefault,
          })) ?? null
        }
        error={structures.error}
        selectedId={structureId}
        onSelect={setStructureId}
        empty={{
          title: "No bell schedules yet",
          hint: "Add a schedule above, then its periods can be recorded here.",
        }}
      >
        <OrderNote periods={periods.items} />
        <CrudPanel
          spec={periodSpec}
          reloadKey={structureId ?? ""}
          onChanged={periods.reload}
        />
      </ParentPicker>
    </div>
  );
}

/**
 * Points out when the declared order and the clock disagree.
 *
 * Not an error — a school may genuinely number a period before an earlier-starting one,
 * and the model does not care. But it is almost always a typo, and finding out from a
 * wrong timetable later is much worse than being asked about it now.
 */
function OrderNote(props: { periods: readonly Period[] | null }) {
  const periods = props.periods;
  if (!periods || periods.length < 2) return null;

  const outOfStep = periods.some((period, index) => {
    const previous = periods[index - 1];
    return previous !== undefined && period.startsAt < previous.startsAt;
  });
  if (!outOfStep) return null;

  return (
    <Banner tone="warning">
      These periods are not in clock order: a later position starts earlier in the day. The
      timetable grid is built from the <strong>position</strong>, not the time, so this
      will work — but check it is what you meant.
    </Banner>
  );
}
