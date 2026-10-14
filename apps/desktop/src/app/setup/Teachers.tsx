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
 *
 * ## One teacher, possibly two schools
 *
 * Since Stage 0B a teacher is a project-level person with a membership per school, and the
 * list below is built from membership rather than from `teacher.schoolId`. So a teacher
 * shared with another school of a trust appears here *and* there, as one record with one
 * identifier — which is what makes their availability a single fact rather than one per
 * school.
 *
 * Two consequences the screen has to be honest about, because the backend enforces both:
 *
 * - **"Active" is two facts.** `teacher.isActive` says whether they are in the project at
 *   all; the membership says whether *this* school employs them. The Status column shows
 *   the conjunction, and distinguishes the two, because a teacher who has left Southmoor
 *   and stayed at Northgate must not read as "Active" on Southmoor's screen.
 * - **A visiting teacher is not this school's to edit or delete.** Their details belong to
 *   the school that created them, and `updateTeacher` is refused with a wrong-school error
 *   anywhere else. Rather than let the user find that out by pressing Save, such a row is
 *   marked and the restriction is stated. The full school-assignment interface is Stage
 *   0C; this screen does only what keeps it correct in the meantime.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Campus,
  School,
  Teacher,
  TeacherInSchool,
  TeacherInput,
} from "@evara/domain";
import {
  createTeacher,
  deleteTeacher,
  listCampuses,
  listSchoolTeachers,
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

/**
 * Shown in the Status column, so the states read as states rather than as a blank.
 *
 * Three answers, not two, because there are two flags. "Archived" is the project-level
 * one and outranks the membership: an archived person is offered to nobody, whatever any
 * school's employment record says. "Not teaching here" is the per-school one, and is the
 * ordinary state for somebody who has left one school of a trust and stayed at another.
 */
function statusLabel(teacher: Teacher, membership: TeacherInSchool | undefined): string {
  if (!teacher.isActive) return "Archived";
  // No membership row should be impossible -- the list is built from membership -- but
  // saying "Active" on the strength of a missing record would be the one wrong answer.
  if (membership && !membership.membership.isActive) return "Not teaching here";
  return "Active";
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

  // This school's own view of each teacher's employment, which the plain teacher record
  // cannot carry: `isActive` on it is the project-level flag.
  const loadMemberships = useCallback(() => listSchoolTeachers(school.id), [school.id]);
  const memberships = useList<TeacherInSchool>(loadMemberships);

  const membershipById: ReadonlyMap<string, TeacherInSchool> = useMemo(
    () => new Map((memberships.items ?? []).map((row) => [row.teacher.id, row])),
    [memberships.items],
  );

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
      {
        header: "Status",
        render: (row) => statusLabel(row, membershipById.get(row.id)),
      },
      {
        header: "Belongs to",
        render: (row) =>
          row.schoolId === school.id ? "This school" : "Another school (visiting)",
      },
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
    remove: (id) => deleteTeacher(school.id, id),
    emptyHint: "No teachers yet. Add the staff who will appear on the timetable.",
    confirmSubmit: (_draft, row) =>
      row !== null && row.schoolId !== school.id
        ? "This teacher belongs to another school, which holds their details. Saving here will be refused. Stage 0C adds the screen for editing a shared teacher from the school that owns them."
        : null,
    deleteNote: (row) => {
      if (row.schoolId !== school.id) {
        return "This teacher belongs to another school, so deleting them here will be refused. Removing them from this school is a Stage 0C action; it does not delete the person.";
      }
      return row.isActive
        ? "If this teacher has simply left, clearing “Currently teaching” keeps them and their history instead. Deleting is also refused while another school still employs them."
        : "Deleting is refused while another school still employs them.";
    },
  };

  return (
    <section className="sectionBody">
      <p className="muted">
        The staff a timetable can be built around. A teacher keeps the same identity across
        terms and academic years, so editing one here never breaks anything that already
        names them.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      {memberships.error && (
        <p className="muted">
          This school’s employment records could not be loaded: {memberships.error}
        </p>
      )}
      <CrudPanel
        spec={spec}
        reloadKey={`${school.id}:${(memberships.items ?? []).length}`}
        onChanged={() => {
          teachers.reload();
          memberships.reload();
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
