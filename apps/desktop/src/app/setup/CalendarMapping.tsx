/**
 * Section 8 — Calendar Mapping.
 *
 * ## Explicit, one date at a time
 *
 * This is the only bridge between real dates and the scheduling model, and every entry is
 * a mapping a human made. Evara does **not** infer weekends, does not skip holidays, and
 * does not decide what day follows a closure — the brief forbids inventing those policies
 * and the data model does not state them, so a guess here would be indistinguishable from
 * a decision. What Day follows a snow day is a real question with different answers at
 * different schools; it waits for a phase where someone decides it.
 *
 * ## A date with no cycle day is a day with no lessons
 *
 * `cycleDayId` is nullable and `null` is meaningful, so the select offers "No lessons" as
 * a named choice. That is how a non-teaching date is expressed, and it is why the model
 * refuses to delete a cycle day that dates still point at — reinterpreting those dates as
 * non-teaching would be a change of meaning, not a detached reference (ADR 0011 Q2).
 *
 * The `kind` column records *why*: a holiday, an exam day, an event, staff development.
 * Both halves matter and neither implies the other — an exam day may well have lessons.
 *
 * ## Working days are advice, and these rows are the authority
 *
 * {@link WorkingDayWarning} reports mapped teaching dates that landed on a weekday the
 * school says it does not work. It is a warning and nothing else: the school's working
 * days are a *default calendar pattern*, and an explicit row here outranks them, because a
 * school really might teach one Saturday for an open day. Changing the working days in
 * School Details creates, deletes and rewrites nothing on this screen — it only changes
 * what the warning says. The user resolves it by editing a date or the working week, and
 * the application never resolves it for them.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  AcademicYear,
  CalendarDay,
  Campus,
  Cycle,
  CycleDay,
  NonWorkingInstructionalDay,
  School,
  Term,
} from "@evara/domain";
import {
  createCalendarDay,
  deleteCalendarDay,
  listAcademicYears,
  listCalendarDays,
  listCampuses,
  listCycleDays,
  listCycles,
  listTerms,
  nonWorkingInstructionalDays,
  updateCalendarDay,
} from "../../ipc/setup";
import { toDisplay } from "../dates";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { DateInput } from "../ui/DateInput";
import { Banner, Panel } from "../ui/Shell";
import { describeWeek, weekdayName } from "./weekdays";
import { text, textOrNull, validateCalendarDay } from "../validation";
import { Absent } from "./CampusesAndBuildings";
import { useList } from "./useList";

/** Mirrors `CalendarDayKind` in Rust. The values are the stored spellings. */
const KIND_CHOICES = [
  { value: "SCHOOL", label: "School day" },
  { value: "HOLIDAY", label: "Holiday — closed" },
  { value: "EXAM", label: "Examinations" },
  { value: "EVENT", label: "Whole-school event" },
  { value: "PD", label: "Staff development" },
] as const;

const KIND_SHORT: Record<string, string> = {
  SCHOOL: "School day",
  HOLIDAY: "Holiday",
  EXAM: "Exams",
  EVENT: "Event",
  PD: "Staff development",
};

export function CalendarMapping(props: { school: School }) {
  const { school } = props;

  const loadYears = useCallback(() => listAcademicYears(school.id), [school.id]);
  const years = useList<AcademicYear>(loadYears);

  const loadCycles = useCallback(() => listCycles(school.id), [school.id]);
  const cycles = useList<Cycle>(loadCycles);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  // Every cycle day of every cycle, because a date names a day directly and the screen
  // must be able to label whichever one it names.
  const [days, setDays] = useState<readonly CycleDay[]>([]);
  const [terms, setTerms] = useState<readonly Term[]>([]);

  const cycleItems = cycles.items;
  const yearItems = years.items;

  // Default range: the first academic year, or the current calendar year if there is none.
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const defaultFrom = yearItems?.[0]?.startsOn ?? "";
  const defaultTo = yearItems?.[0]?.endsOn ?? "";
  const activeFrom = from || defaultFrom;
  const activeTo = to || defaultTo;

  const loadDependencies = useCallback(async () => {
    const [dayLists, termLists] = await Promise.all([
      Promise.all((cycleItems ?? []).map((cycle) => listCycleDays(cycle.id))),
      Promise.all((yearItems ?? []).map((year) => listTerms(year.id))),
    ]);
    setDays(dayLists.flat());
    setTerms(termLists.flat());
    return [] as const;
  }, [cycleItems, yearItems]);

  // Loaded through `useList` purely to reuse its cancellation and error handling; the
  // values land in state above because they are two lists, not one.
  const dependencies = useList(loadDependencies);

  const loadCalendar = useCallback(
    () =>
      activeFrom && activeTo
        ? listCalendarDays(school.id, activeFrom, activeTo)
        : Promise.resolve([]),
    [school.id, activeFrom, activeTo],
  );

  const spec = useMemo<CrudSpec<CalendarDay, Parameters<typeof createCalendarDay>[0]>>(() => {
    const cycleName = new Map((cycleItems ?? []).map((c) => [c.id, c.name]));
    const dayChoices = days.map((day) => ({
      value: day.id,
      label:
        (cycleItems ?? []).length > 1
          ? `${cycleName.get(day.cycleId) ?? "Cycle"} — ${day.label} (position ${day.ordinal})`
          : `${day.label} (position ${day.ordinal})`,
    }));
    const dayLabel = new Map(dayChoices.map((c) => [c.value, c.label]));

    const termChoices = terms.map((term) => ({ value: term.id, label: term.name }));
    const termLabel = new Map(termChoices.map((c) => [c.value, c.label]));

    const campusChoices = (campuses.items ?? []).map((c) => ({ value: c.id, label: c.name }));
    const campusLabel = new Map(campusChoices.map((c) => [c.value, c.label]));

    return {
      noun: "date",
      plural: "dates",
      title:
        activeFrom && activeTo
          ? `Dates from ${toDisplay(activeFrom)} to ${toDisplay(activeTo)}`
          : "Dates",
      describe: (row) => toDisplay(row.date),
      columns: [
        {
          header: "Date",
          render: (row) => <time dateTime={row.date}>{toDisplay(row.date)}</time>,
        },
        {
          header: "Cycle day",
          render: (row) =>
            row.cycleDayId ? (
              (dayLabel.get(row.cycleDayId) ?? <Absent />)
            ) : (
              <span className="muted">No lessons</span>
            ),
        },
        { header: "Kind", render: (row) => KIND_SHORT[row.kind] ?? row.kind },
        {
          header: "Term",
          render: (row) => (row.termId ? (termLabel.get(row.termId) ?? <Absent />) : <Absent />),
        },
        {
          header: "Campus",
          render: (row) =>
            row.campusId ? (
              (campusLabel.get(row.campusId) ?? <Absent />)
            ) : (
              <span className="muted">Whole school</span>
            ),
        },
        { header: "Note", render: (row) => row.note ?? <Absent /> },
      ],
      fields: [
        {
          kind: "date",
          key: "date",
          label: "Date",
          required: true,
          hint: "One entry per date per campus.",
        },
        {
          kind: "select",
          key: "cycleDayId",
          label: "Cycle day",
          emptyLabel: "No lessons this day",
          choices: dayChoices,
          hint:
            dayChoices.length === 0
              ? "No cycle days exist yet. Add them in Cycles and Cycle Days, or record this date as having no lessons."
              : "Which day of the repeating pattern this date is. Leave as “no lessons” for a closure.",
        },
        {
          kind: "select",
          key: "kind",
          label: "Kind of day",
          required: true,
          choices: KIND_CHOICES,
          hint: "Records why. Independent of the cycle day — an exam day may still have lessons.",
        },
        {
          kind: "select",
          key: "termId",
          label: "Term",
          emptyLabel: "Not recorded",
          choices: termChoices,
          hint: "Optional.",
        },
        {
          kind: "select",
          key: "campusId",
          label: "Campus",
          emptyLabel: "Whole school",
          choices: campusChoices,
          hint: "Scope the entry to one campus when sites close on different days.",
        },
        { kind: "textarea", key: "note", label: "Note", hint: "Optional." },
      ],
      validate: validateCalendarDay,
      toDraft: (row) => ({
        date: row?.date ?? "",
        cycleDayId: row?.cycleDayId ?? "",
        kind: row?.kind ?? "SCHOOL",
        termId: row?.termId ?? "",
        campusId: row?.campusId ?? "",
        note: row?.note ?? "",
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        date: text(draft, "date"),
        termId: textOrNull(draft, "termId"),
        campusId: textOrNull(draft, "campusId"),
        cycleDayId: textOrNull(draft, "cycleDayId"),
        kind: text(draft, "kind") as CalendarDay["kind"],
        note: textOrNull(draft, "note"),
      }),
      list: loadCalendar,
      create: createCalendarDay,
      update: (row, input) => updateCalendarDay(row.id, row.rev, input),
      remove: (row) => deleteCalendarDay(school.id, row.id, row.rev),
      emptyHint:
        "No dates are recorded in this range. Add the teaching days you want mapped onto cycle days, and the closures you want marked. Evara records only what you enter — it does not assume weekends or holidays.",
      blockedBy:
        activeFrom && activeTo
          ? null
          : "Set a date range above — or add an academic year, and its dates will be used.",
    };
  }, [school.id, days, terms, cycleItems, campuses.items, activeFrom, activeTo, loadCalendar]);

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Calendar mapping</h2>
        <p>
          Which real dates are which cycle day. Every entry is explicit: Evara never infers
          weekends, holidays or where a rotation resumes after a closure.
        </p>
      </header>

      {dependencies.error && <Banner tone="error">{dependencies.error}</Banner>}

      <Panel
        title="Date range"
        note="Calendar entries are read a range at a time, so a long year stays readable."
      >
        <div className="rangeRow">
          <div className="field">
            <label htmlFor="cal-from">From</label>
            <DateInput
              id="cal-from"
              label="From"
              value={from}
              onChange={setFrom}
            />
          </div>
          <div className="field">
            <label htmlFor="cal-to">To</label>
            <DateInput id="cal-to" label="To" value={to} onChange={setTo} />
          </div>
          {(defaultFrom || defaultTo) && (from || to) && (
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setFrom("");
                setTo("");
              }}
            >
              Use the academic year
            </button>
          )}
        </div>
        {(defaultFrom || defaultTo) && !from && !to && (
          <p className="hint">
            {`Showing the academic year: ${toDisplay(defaultFrom)} to ${toDisplay(defaultTo)}. Enter a range above to narrow it.`}
          </p>
        )}
        {yearItems !== null && yearItems.length === 0 && (
          <Banner tone="info">
            No academic year exists yet, so there is no default range. Add one in Academic
            Years and Terms, or type a range above.
          </Banner>
        )}
      </Panel>

      <WorkingDayWarning school={school} />

      <CrudPanel spec={spec} reloadKey={`${school.id}:${activeFrom}:${activeTo}`} />
    </div>
  );
}

/** How many offending dates are named before the rest are summarised. */
const NAMED_LIMIT = 8;

/**
 * Warns when a mapped teaching date sits on a weekday the school does not work.
 *
 * A warning, never a correction. The backend only reads — see
 * `TimeModel::non_working_instructional_days` — so nothing here or there alters a mapping,
 * and the user decides whether the date is a typo or a deliberate Saturday.
 *
 * Deliberately *not* scoped to the range above: a date entered a day out is worth knowing
 * about whether or not it happens to be on screen, and a warning that disappeared when you
 * narrowed the range would be worse than no warning. The count is over the whole of this
 * school's calendar -- the active school's, since V12, because another school's mapping is
 * read against its own working week and is none of this screen's business.
 */
function WorkingDayWarning(props: { school: School }) {
  const { school } = props;
  const workingDays = school.workingDays;

  // The week is the only thing this depends on. Deliberately *not* the date range: the
  // finding is project-wide, so re-reading when the range above changes would be a round
  // trip for an answer that cannot have moved.
  //
  // Keyed on the joined week rather than the array, because the array arrives from JSON
  // and is a fresh object on every load — its identity would re-fetch forever.
  const key = workingDays.join(",");
  const load = useCallback(
    () => nonWorkingInstructionalDays(school.id, workingDays),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [school.id, key],
  );
  const found = useList<NonWorkingInstructionalDay>(load);

  // A failure here is not worth a banner of its own: the screen's real job is the table
  // below, and an advisory that could not be computed is better silent than alarming.
  if (found.error !== null || found.items === null || found.items.length === 0) return null;

  const days = found.items;
  const named = days.slice(0, NAMED_LIMIT);
  const rest = days.length - named.length;

  return (
    <Banner tone="warning">
      {days.length === 1
        ? "One mapped teaching date falls outside the school's working days"
        : `${days.length} mapped teaching dates fall outside the school's working days`}{" "}
      ({describeWeek(workingDays)}):{" "}
      {named
        .map((day) => `${toDisplay(day.date)} (${weekdayName(day.weekday)})`)
        .join(", ")}
      {rest > 0 && ` and ${rest} more`}. Nothing has been changed — these mappings are
      what the timetable uses. Edit a date below if it is wrong, or change the working days
      in School Details if the school really does teach then.
    </Banner>
  );
}
