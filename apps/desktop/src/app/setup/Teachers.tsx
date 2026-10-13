/**
 * Section 9 — Teachers.
 *
 * ## Inactive is a state, not a filter on the truth
 *
 * The list shows every teacher, and an inactive one is marked rather than hidden. Hiding
 * them would be the obvious thing and the wrong one: a teacher who left mid-year is still
 * named by last term's timetable, and a screen that pretended otherwise would make the
 * timetable look corrupt. The repository returns them for the same reason.
 *
 * So "Active" is a column, and the only thing it changes is whether the teacher is offered
 * for *new* work — which is a decision Phase 3's lesson assignment makes, not this screen.
 *
 * ## Almost nothing is required
 *
 * Two names. Not a staff code, because plenty of schools do not issue one and a form that
 * insisted would be answered with invented numbers. Not an email, because Evara has no
 * accounts, sends nothing, and asking for contact details it will never use is asking a
 * school to type its staff directory into a timetabler for no return.
 *
 * The load ceilings are the same bargain in the other direction: blank means *no ceiling*,
 * not zero, and the hints say so. They are carried now and read by the Phase 4 load
 * constraints; a school that does not cap its teachers simply leaves them empty.
 */
import { useCallback, useMemo, useState } from "react";

import type { Campus, School, Teacher, TeacherInput } from "@evara/domain";
import {
  createTeacher,
  deleteTeacher,
  listCampuses,
  listTeachers,
  updateTeacher,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import {
  flag,
  number,
  numberOrNull,
  text,
  textOrNull,
  validateTeacher,
} from "../validation";
import { AvailabilityPanel } from "./AvailabilityGrid";
import { useList } from "./useList";

/** Shown in the Active column, so the two states read as states rather than as a blank. */
function activeLabel(teacher: Teacher): string {
  return teacher.isActive ? "Active" : "Inactive";
}

export function Teachers(props: { school: School }) {
  const { school } = props;
  // Bumped after any change to the teacher list, so the availability picker below cannot
  // go on offering someone who has just been deleted.
  const [listNonce, setListNonce] = useState(0);

  const loadTeachers = useCallback(() => listTeachers(school.id), [school.id]);
  const teachers = useList<Teacher>(loadTeachers);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  const campusChoices: readonly Choice[] = useMemo(
    () => (campuses.items ?? []).map((campus) => ({ value: campus.id, label: campus.name })),
    [campuses.items],
  );

  const spec: CrudSpec<Teacher, TeacherInput> = {
    noun: "teacher",
    plural: "teachers",
    title: "Teachers",
    columns: [
      { header: "Last name", render: (row) => row.familyName },
      { header: "First name", render: (row) => row.givenName },
      { header: "Staff code", render: (row) => row.code ?? "—" },
      { header: "FTE", render: (row) => `${row.ftePercent}%`, numeric: true },
      { header: "Status", render: activeLabel },
    ],
    fields: [
      { kind: "text", key: "givenName", label: "First name", required: true },
      { kind: "text", key: "familyName", label: "Last name", required: true },
      {
        kind: "text",
        key: "code",
        label: "Staff code",
        hint: "Optional. Must be unique within the school when given.",
      },
      {
        kind: "text",
        key: "email",
        label: "Email",
        hint: "Optional, and never used to sign in — Evara has no accounts and sends nothing.",
      },
      {
        kind: "select",
        key: "homeCampusId",
        label: "Home campus",
        emptyLabel: "No home campus",
        hint: "Where they are normally based. Does not restrict where they can teach.",
        choices: campusChoices,
      },
      {
        kind: "number",
        key: "ftePercent",
        label: "Full-time equivalent (%)",
        required: true,
        min: 0,
        max: 100,
        hint: "100 is a full timetable, 50 a half one.",
      },
      {
        kind: "number",
        key: "maxPeriodsPerDay",
        label: "Most periods per day",
        min: 0,
        hint: "Leave blank for no limit.",
      },
      {
        kind: "number",
        key: "maxPeriodsPerCycle",
        label: "Most periods per cycle",
        min: 0,
        hint: "Leave blank for no limit.",
      },
      {
        kind: "number",
        key: "maxConsecutivePeriods",
        label: "Most periods in a row",
        min: 0,
        hint: "Leave blank for no limit.",
      },
      { kind: "textarea", key: "notes", label: "Notes" },
      {
        kind: "checkbox",
        key: "isActive",
        label: "Currently teaching",
        hint: "Clearing this keeps the teacher and everything they are assigned to; they are simply not offered for new work.",
      },
    ],
    describe: (row) => `${row.givenName} ${row.familyName}`,
    validate: validateTeacher,
    toDraft: (row) => ({
      givenName: row?.givenName ?? "",
      familyName: row?.familyName ?? "",
      code: row?.code ?? "",
      email: row?.email ?? "",
      homeCampusId: row?.homeCampusId ?? null,
      ftePercent: String(row?.ftePercent ?? 100),
      maxPeriodsPerDay: row?.maxPeriodsPerDay === null || row?.maxPeriodsPerDay === undefined
        ? ""
        : String(row.maxPeriodsPerDay),
      maxPeriodsPerCycle:
        row?.maxPeriodsPerCycle === null || row?.maxPeriodsPerCycle === undefined
          ? ""
          : String(row.maxPeriodsPerCycle),
      maxConsecutivePeriods:
        row?.maxConsecutivePeriods === null || row?.maxConsecutivePeriods === undefined
          ? ""
          : String(row.maxConsecutivePeriods),
      notes: row?.notes ?? "",
      // A new teacher is active. The common case should not need a tick.
      isActive: row === null ? true : row.isActive,
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      givenName: text(draft, "givenName"),
      familyName: text(draft, "familyName"),
      code: textOrNull(draft, "code"),
      email: textOrNull(draft, "email"),
      homeCampusId: textOrNull(draft, "homeCampusId"),
      ftePercent: number(draft, "ftePercent"),
      maxPeriodsPerDay: numberOrNull(draft, "maxPeriodsPerDay"),
      maxPeriodsPerCycle: numberOrNull(draft, "maxPeriodsPerCycle"),
      maxConsecutivePeriods: numberOrNull(draft, "maxConsecutivePeriods"),
      notes: textOrNull(draft, "notes"),
      isActive: flag(draft, "isActive"),
    }),
    list: () => listTeachers(school.id),
    create: (input) => createTeacher(input),
    update: (id, input) => updateTeacher(id, input),
    remove: (id) => deleteTeacher(id),
    emptyHint: "No teachers yet. Add the staff who will appear on the timetable.",
    deleteNote: (row) =>
      row.isActive
        ? "If this teacher has simply left, clearing “Currently teaching” keeps them and their history instead."
        : null,
  };

  return (
    <section className="sectionBody">
      <p className="muted">
        The staff a timetable can be built around. A teacher keeps the same identity across
        terms and academic years, so editing one here never breaks anything that already
        names them.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      <CrudPanel
        spec={spec}
        reloadKey={school.id}
        onChanged={() => {
          teachers.reload();
          setListNonce((value) => value + 1);
        }}
      />
      <AvailabilityPanel
        key={listNonce}
        school={school}
        entityKind="TEACHER"
        noun="teacher"
        options={(teachers.items ?? []).map((teacher) => ({
          id: teacher.id,
          name: `${teacher.familyName}, ${teacher.givenName}${teacher.isActive ? "" : " (inactive)"}`,
        }))}
      />
    </section>
  );
}
