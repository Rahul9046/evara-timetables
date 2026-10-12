/**
 * Section 10 — Students and year levels.
 *
 * ## Students are optional, and the screen says so
 *
 * [ADR 0006](../../../../../docs/adr/0006-group-based-scheduling-first.md) requires a
 * school to be able to build a complete timetable without entering a single student, using
 * a group's declared size instead. That is not a limitation to apologise for — it is the
 * difference between a school trying Evara in an afternoon and one giving up before it
 * sees a timetable. The copy here says it plainly so nobody types two thousand names
 * believing they have to.
 *
 * Students become worth entering when a school wants per-student timetables, or when the
 * request-driven scheduling of Milestone 2 arrives. Until then the group is the audience.
 *
 * ## Deleting a student is refused, not silently widened
 *
 * A student in any group cannot be deleted; the backend refuses. Rather than let the user
 * find that out by pressing Delete, this screen reads the student's groups first and says
 * what will happen in the confirmation — and offers the thing they almost certainly meant,
 * which is to mark them as having left. Membership is the only record that a student was
 * ever in 9-MAT-A, and deleting the person is the one action that destroys it.
 *
 * ## Year levels sit here rather than in their own section
 *
 * They are what a student is classified by, they are four rows in most schools, and the
 * rest of School Setup already pairs an entity with the small table that classifies it —
 * rooms with room types, cycles with cycle days. Deleting one unclassifies its students
 * through a real repository update rather than a database-level `SET NULL`, so the
 * student's revision stays truthful.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Campus,
  School,
  Student,
  StudentGroup,
  StudentInput,
  YearLevel,
  YearLevelInput,
} from "@evara/domain";
import {
  createStudent,
  createYearLevel,
  deleteStudent,
  deleteYearLevel,
  listCampuses,
  listGroupsForStudent,
  listStudents,
  listYearLevels,
  updateStudent,
  updateYearLevel,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import {
  flag,
  number,
  text,
  textOrNull,
  validateStudent,
  validateYearLevel,
} from "../validation";
import { useList } from "./useList";

export function StudentsAndYearLevels(props: { school: School }) {
  const { school } = props;

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

  const loadYearLevels = useCallback(() => listYearLevels(), []);
  const yearLevels = useList<YearLevel>(loadYearLevels);

  /**
   * Which groups each student is in, filled in as rows are considered for deletion.
   *
   * Read lazily rather than up front: a school with two thousand students would otherwise
   * make two thousand calls to populate a warning almost nobody will see.
   */
  const [groupsByStudent, setGroupsByStudent] = useState<
    Readonly<Record<string, readonly StudentGroup[]>>
  >({});

  const noteGroups = useCallback((studentId: string) => {
    void listGroupsForStudent(studentId)
      .then((groups) => {
        setGroupsByStudent((current) => ({ ...current, [studentId]: groups }));
      })
      .catch(() => {
        // The confirmation still works without this; it simply loses the extra sentence,
        // and the backend refuses the delete anyway. Not worth a second error surface.
      });
  }, []);

  const campusChoices: readonly Choice[] = useMemo(
    () => (campuses.items ?? []).map((campus) => ({ value: campus.id, label: campus.name })),
    [campuses.items],
  );

  const yearChoices: readonly Choice[] = useMemo(
    () => (yearLevels.items ?? []).map((year) => ({ value: year.id, label: year.name })),
    [yearLevels.items],
  );

  const yearSpec: CrudSpec<YearLevel, YearLevelInput> = {
    noun: "year level",
    plural: "year levels",
    title: "Year levels",
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Position", render: (row) => String(row.ordinal), numeric: true },
    ],
    fields: [
      { kind: "text", key: "name", label: "Name", required: true, placeholder: "Year 9" },
      {
        kind: "number",
        key: "ordinal",
        label: "Position",
        required: true,
        min: 0,
        hint: "The school's own order, lowest first. Not an age.",
      },
    ],
    describe: (row) => row.name,
    validate: validateYearLevel,
    toDraft: (row) => ({
      name: row?.name ?? "",
      ordinal: row === null ? "" : String(row.ordinal),
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      name: text(draft, "name"),
      ordinal: number(draft, "ordinal"),
    }),
    list: () => listYearLevels(),
    create: (input) => createYearLevel(input),
    update: (id, input) => updateYearLevel(id, input),
    remove: (id) => deleteYearLevel(id),
    emptyHint: "No year levels yet. Add the years this school teaches.",
    deleteNote: () =>
      "Students and groups in this year level are kept. They simply stop naming a year.",
  };

  const studentSpec: CrudSpec<Student, StudentInput> = {
    noun: "student",
    plural: "students",
    title: "Students",
    columns: [
      { header: "Last name", render: (row) => row.familyName },
      { header: "First name", render: (row) => row.givenName },
      { header: "Admission no.", render: (row) => row.code ?? "—" },
      {
        header: "Year",
        render: (row) =>
          (yearLevels.items ?? []).find((year) => year.id === row.yearLevelId)?.name ?? "—",
      },
      { header: "Status", render: (row) => (row.isActive ? "Enrolled" : "Left") },
    ],
    fields: [
      { kind: "text", key: "givenName", label: "First name", required: true },
      { kind: "text", key: "familyName", label: "Last name", required: true },
      {
        kind: "text",
        key: "code",
        label: "Admission number",
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
        key: "yearLevelId",
        label: "Year level",
        emptyLabel: "No year level",
        choices: yearChoices,
      },
      {
        kind: "select",
        key: "campusId",
        label: "Campus",
        emptyLabel: "No campus",
        choices: campusChoices,
      },
      {
        kind: "checkbox",
        key: "isActive",
        label: "Currently enrolled",
        hint: "Clearing this keeps the student and every group they have been in.",
      },
    ],
    describe: (row) => `${row.givenName} ${row.familyName}`,
    validate: validateStudent,
    toDraft: (row) => {
      if (row !== null) noteGroups(row.id);
      return {
        givenName: row?.givenName ?? "",
        familyName: row?.familyName ?? "",
        code: row?.code ?? "",
        email: row?.email ?? "",
        yearLevelId: row?.yearLevelId ?? null,
        campusId: row?.campusId ?? null,
        isActive: row === null ? true : row.isActive,
      };
    },
    toInput: (draft) => ({
      schoolId: school.id,
      givenName: text(draft, "givenName"),
      familyName: text(draft, "familyName"),
      code: textOrNull(draft, "code"),
      email: textOrNull(draft, "email"),
      yearLevelId: textOrNull(draft, "yearLevelId"),
      campusId: textOrNull(draft, "campusId"),
      isActive: flag(draft, "isActive"),
    }),
    list: () =>
      listStudents().then((rows) => {
        // Populate the deletion warning for the rows now on screen, in one pass rather
        // than one call per row at confirmation time.
        for (const row of rows) noteGroups(row.id);
        return rows;
      }),
    create: (input) => createStudent(input),
    update: (id, input) => updateStudent(id, input),
    remove: (id) => deleteStudent(id),
    emptyHint:
      "No students yet — and a timetable does not need any. Add them when you want per-student timetables.",
    deleteNote: (row) => {
      const groups = groupsByStudent[row.id] ?? [];
      if (groups.length === 0) {
        return row.isActive
          ? "If this student has left, clearing “Currently enrolled” keeps them and their history instead."
          : null;
      }
      const names = groups.map((group) => group.name).join(", ");
      return `This student is in ${groups.length} group${groups.length === 1 ? "" : "s"} (${names}), so deleting them will be refused — a membership is the only record that they were ever in one. Remove them from those groups first, or clear “Currently enrolled” to keep the history.`;
    },
  };

  return (
    <section className="sectionBody">
      <p className="muted">
        Students are <strong>optional</strong>. A complete timetable can be built from group
        sizes alone, so enter students only when you want per-student timetables.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      <CrudPanel spec={yearSpec} onChanged={yearLevels.reload} />
      <CrudPanel spec={studentSpec} reloadKey={String((yearLevels.items ?? []).length)} />
    </section>
  );
}
