/**
 * Section 5 — Academic Years and Terms.
 *
 * A term is a division of a year, ordered by `ordinal` rather than by its dates — which
 * matters because a term is also a scope for the timetable grid: a school whose bell
 * schedule changes after Christmas materialises one grid per term, and a school whose
 * does not materialises one grid for the whole year.
 *
 * Dates are plain `YYYY-MM-DD` text. Nothing in scheduling reads them; they exist so the
 * calendar section can say which term a mapped date falls in.
 */
import { useCallback, useMemo, useState } from "react";

import type { AcademicYear, School, Term } from "@evara/domain";
import {
  createAcademicYear,
  createTerm,
  deleteAcademicYear,
  deleteTerm,
  listAcademicYears,
  listTerms,
  updateAcademicYear,
  updateTerm,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { number, text, validateAcademicYear, validateTerm } from "../validation";
import { ParentPicker } from "./ParentPicker";
import { useList } from "./useList";

export function AcademicYearsAndTerms(props: { school: School }) {
  const { school } = props;
  const [yearId, setYearId] = useState<string | null>(null);

  const loadYears = useCallback(() => listAcademicYears(), []);
  const years = useList<AcademicYear>(loadYears);

  const yearSpec = useMemo<CrudSpec<AcademicYear, Parameters<typeof createAcademicYear>[0]>>(
    () => ({
      noun: "academic year",
      plural: "academic years",
      title: "Academic years",
      describe: (row) => row.name,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Starts", render: (row) => <time>{row.startsOn}</time> },
        { header: "Ends", render: (row) => <time>{row.endsOn}</time> },
      ],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Year name",
          required: true,
          placeholder: "2027/28",
        },
        { kind: "date", key: "startsOn", label: "First day", required: true },
        { kind: "date", key: "endsOn", label: "Last day", required: true },
      ],
      validate: validateAcademicYear,
      toDraft: (row) => ({
        name: row?.name ?? "",
        startsOn: row?.startsOn ?? "",
        endsOn: row?.endsOn ?? "",
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        name: text(draft, "name"),
        startsOn: text(draft, "startsOn"),
        endsOn: text(draft, "endsOn"),
      }),
      list: listAcademicYears,
      create: createAcademicYear,
      update: updateAcademicYear,
      remove: deleteAcademicYear,
      emptyHint:
        "Add the year you are timetabling. Terms go inside it, and a term can scope the timetable grid when the bell schedule changes partway through the year.",
      deleteNote: () => (
        <p className="muted">A year that still has terms cannot be deleted.</p>
      ),
    }),
    [school.id],
  );

  const termSpec = useMemo<CrudSpec<Term, Parameters<typeof createTerm>[0]>>(
    () => ({
      noun: "term",
      plural: "terms",
      title: "Terms in this year",
      describe: (row) => row.name,
      columns: [
        { header: "Position", numeric: true, render: (row) => row.ordinal },
        { header: "Name", render: (row) => row.name },
        { header: "Starts", render: (row) => <time>{row.startsOn}</time> },
        { header: "Ends", render: (row) => <time>{row.endsOn}</time> },
      ],
      fields: [
        {
          kind: "number",
          key: "ordinal",
          label: "Position",
          required: true,
          min: 1,
          hint: "Teaching order within the year, from 1. This is how terms are sorted, not their dates.",
        },
        {
          kind: "text",
          key: "name",
          label: "Term name",
          required: true,
          placeholder: "Autumn",
        },
        { kind: "date", key: "startsOn", label: "First day", required: true },
        { kind: "date", key: "endsOn", label: "Last day", required: true },
      ],
      validate: validateTerm,
      toDraft: (row) => ({
        ordinal: row ? String(row.ordinal) : "1",
        name: row?.name ?? "",
        startsOn: row?.startsOn ?? "",
        endsOn: row?.endsOn ?? "",
      }),
      toInput: (draft) => ({
        academicYearId: yearId ?? "",
        name: text(draft, "name"),
        ordinal: number(draft, "ordinal"),
        startsOn: text(draft, "startsOn"),
        endsOn: text(draft, "endsOn"),
      }),
      list: () => (yearId ? listTerms(yearId) : Promise.resolve([])),
      create: createTerm,
      update: updateTerm,
      remove: deleteTerm,
      emptyHint:
        "Terms are optional. Add them if the timetable changes during the year, or if you want the calendar to record which term a date falls in.",
      deleteNote: () => (
        <p className="muted">
          A term still named by a timeslot or a calendar date cannot be deleted.
        </p>
      ),
      blockedBy: yearId === null ? "Select an academic year first." : null,
    }),
    [yearId],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Academic years and terms</h2>
        <p>
          Terms are ordered by position, not by date. A term is also the optional scope of
          a timetable grid — see Timetable Grid Preview.
        </p>
      </header>

      <CrudPanel spec={yearSpec} onChanged={years.reload} />

      <hr />

      <ParentPicker
        legend="Academic year"
        items={
          years.items?.map((year) => ({
            id: year.id,
            label: year.name,
            detail: `${year.startsOn} → ${year.endsOn}`,
          })) ?? null
        }
        error={years.error}
        selectedId={yearId}
        onSelect={setYearId}
        empty={{
          title: "No academic years yet",
          hint: "Add a year above, then its terms can be recorded here.",
        }}
      >
        <CrudPanel spec={termSpec} reloadKey={yearId ?? ""} />
      </ParentPicker>
    </div>
  );
}
