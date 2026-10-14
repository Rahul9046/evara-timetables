/**
 * Section 12 — Students and year levels.
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
 * ## Placement is the enrolment's, not the student's
 *
 * Since Stage 0B a student is a project-level person with an enrolment per school, and that
 * enrolment is where the school-specific admission number, year level and campus live. The
 * list is built from enrolment, so a student shared with another school of a trust appears
 * here *and* there, as one record with one identifier.
 *
 * The Admission number, Year, Campus and Status columns therefore read **this school's**
 * enrolment rather than the columns on the student record, which name only the school that
 * created them. Showing the home school's year on another school's roll would be a
 * confident wrong answer.
 *
 * ## Stage 0C: enrolment is editable, and nobody is entered twice
 *
 * - **"Enrol an existing student"** searches the whole project and writes a spell. Nothing
 *   here can produce a second student record for one real person.
 * - {@link StudentEnrolments} below edits this school's spell, records a departure,
 *   re-enrols, shows the whole history across every school, and transfers.
 * - A visiting student's *identity* belongs to the school that created them and
 *   `updateStudent` is refused anywhere else, so Edit is unavailable on such a row and the
 *   enrolment panel — which is what this school can change — is offered instead.
 *
 * ## Deleting a student is refused, not silently widened
 *
 * A student in any group, or enrolled at another school, cannot be deleted; the backend
 * refuses. Rather than let the user find that out by pressing Delete, this screen reads
 * both facts first and explains, offering what they almost certainly meant — which is to
 * record that the student has left. Membership is the only record that a student was ever
 * in 9-MAT-A, and deleting the person is the one action that destroys it.
 *
 * ## Year levels sit here rather than in their own section
 *
 * They are what a student is classified by, they are four rows in most schools, and the
 * rest of School Setup already pairs an entity with the small table that classifies it —
 * rooms with room types, cycles with cycle days. Deleting one unclassifies its students
 * through a real repository update rather than a database-level `SET NULL`, so the
 * student's revision stays truthful.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type {
  Campus,
  School,
  Student,
  StudentGroup,
  StudentInSchool,
  StudentInput,
  YearLevel,
  YearLevelInput,
} from "@evara/domain";
import {
  createStudent,
  createYearLevel,
  deleteStudent,
  deleteYearLevel,
  describeSetupError,
  enrolStudent,
  listCampuses,
  listGroupsForStudent,
  listSchoolStudents,
  listStudents,
  listYearLevels,
  updateStudent,
  updateYearLevel,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner, ConfirmDialog } from "../ui/Shell";
import {
  flag,
  number,
  text,
  textOrNull,
  validateStudent,
  validateYearLevel,
} from "../validation";
import { PersonSearch, type Candidate } from "./PersonSearch";
import type { SetupContext } from "./SchoolSetup";
import { StudentEnrolments } from "./StudentEnrolments";
import { useList } from "./useList";

/**
 * One row of the students table: the shared identity with this school's spell beside it.
 *
 * `enrolment` is `null` for somebody every one of whose spells here is in the past —
 * which `listStudents` keeps returning, because the school still holds their history, and
 * which "Enrolled" would be a lie about.
 */
type StudentRow = Student & { readonly enrolment: StudentInSchool["enrolment"] | null };

export function StudentsAndYearLevels(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const schools = props.context?.schools ?? [school];
  const selectSchool = props.context?.selectSchool;

  /** Whose enrolment the panel at the bottom is about. */
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  const loadYearLevels = useCallback(() => listYearLevels(school.id), [school.id]);
  const yearLevels = useList<YearLevel>(loadYearLevels);

  /**
   * Everybody this school has ever enrolled, with the current spell where there is one.
   *
   * Two reads rather than one: `listStudents` is every student with any spell here, past
   * ones included, and `listSchoolStudents` is the current roll with each placement. The
   * roll alone would drop a transferred-away student the school still holds history for;
   * the list alone would carry only the home school's columns.
   */
  const loadRoll = useCallback(
    () =>
      Promise.all([listStudents(school.id), listSchoolStudents(school.id)]).then(
        ([people, roll]): StudentRow[] => {
          const current = new Map(roll.map((entry) => [entry.student.id, entry.enrolment]));
          return people.map((person) => ({
            ...person,
            enrolment: current.get(person.id) ?? null,
          }));
        },
      ),
    [school.id],
  );
  const roll = useList<StudentRow>(loadRoll);

  /**
   * Which schools hold a spell for each student in the project.
   *
   * One read per school rather than one per student, for the reason the Teachers screen
   * fans out the same way: `listStudents` is enrolment-backed, so the union over every
   * school is the whole attendance graph. It answers whether a delete will be refused, and
   * which schools to show beside a candidate in the search below.
   */
  const [attended, setAttended] = useState<ReadonlyMap<string, readonly string[]>>(new Map());
  const [known, setKnown] = useState<readonly Student[] | null>(null);
  const [graphError, setGraphError] = useState<string | null>(null);
  const [graphNonce, setGraphNonce] = useState(0);

  const schoolKey = schools.map((candidate) => candidate.id).join(",");

  useEffect(() => {
    let live = true;
    setKnown(null);
    void Promise.all(
      schools.map((candidate) =>
        listStudents(candidate.id).then((rows) => [candidate.id, rows] as const),
      ),
    )
      .then((pairs) => {
        if (!live) return;
        const bySchool = new Map<string, string[]>();
        const people = new Map<string, Student>();
        for (const [schoolId, rows] of pairs) {
          for (const row of rows) {
            if (!people.has(row.id)) people.set(row.id, row);
            const list = bySchool.get(row.id) ?? [];
            list.push(schoolId);
            bySchool.set(row.id, list);
          }
        }
        setAttended(bySchool);
        setKnown([...people.values()]);
        setGraphError(null);
      })
      .catch((cause: unknown) => {
        if (!live) return;
        setKnown([]);
        setGraphError(describeSetupError(cause));
      });
    return () => {
      live = false;
    };
    // `schoolKey` rather than `schools`, whose array identity changes on every render.
  }, [schoolKey, graphNonce]);

  /**
   * Which groups each student is in, filled in as rows arrive.
   *
   * Read for the rows on screen in one pass rather than one call per row at confirmation
   * time: the count is what makes the delete explanation worth reading, and a dialogue
   * that has to wait for a round trip either flickers or asks before it knows.
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

  const refreshAll = useCallback(() => {
    roll.reload();
    yearLevels.reload();
    setGraphNonce((value) => value + 1);
  }, [roll, yearLevels]);

  const campusChoices: readonly Choice[] = useMemo(
    () => (campuses.items ?? []).map((campus) => ({ value: campus.id, label: campus.name })),
    [campuses.items],
  );

  const yearChoices: readonly Choice[] = useMemo(
    () => (yearLevels.items ?? []).map((year) => ({ value: year.id, label: year.name })),
    [yearLevels.items],
  );

  const schoolName = useMemo(
    () => new Map(schools.map((candidate) => [candidate.id, candidate.name])),
    [schools],
  );

  const homeOf = (row: Student): string =>
    schoolName.get(row.schoolId) ?? "another school";

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
    list: () => listYearLevels(school.id),
    create: (input) => createYearLevel(input),
    update: (id, input) => updateYearLevel(id, input),
    remove: (id) => deleteYearLevel(school.id, id),
    emptyHint:
      "No year levels yet. Add the years this school teaches — they are this school's own, so two schools in one campus file may each hold their own Grade 1.",
    deleteNote: () =>
      "Students and groups in this year level are kept. They simply stop naming a year.",
  };

  const studentSpec: CrudSpec<StudentRow, StudentInput> = {
    noun: "student",
    plural: "students",
    title: "Students at this school",
    columns: [
      { header: "Last name", render: (row) => row.familyName },
      { header: "First name", render: (row) => row.givenName },
      {
        header: "Admission no.",
        render: (row) => row.enrolment?.admissionNo ?? "—",
      },
      {
        header: "Year",
        render: (row) => {
          const placed = row.enrolment?.yearLevelId ?? null;
          return (yearLevels.items ?? []).find((year) => year.id === placed)?.name ?? "—";
        },
      },
      {
        header: "Campus",
        render: (row) => {
          const placed = row.enrolment?.campusId ?? null;
          return (campuses.items ?? []).find((campus) => campus.id === placed)?.name ?? "—";
        },
      },
      {
        header: "Status",
        render: (row) => {
          if (!row.isActive) return "Archived";
          // No current spell here means every spell at this school is in the past, which
          // is exactly what `listStudents` keeps returning them for.
          if (row.enrolment === null) return "Left this school";
          return row.enrolment.status === "PROSPECTIVE" ? "Prospective" : "Enrolled";
        },
      },
      {
        header: "Belongs to",
        render: (row) =>
          row.schoolId === school.id ? (
            "This school"
          ) : (
            <span title="Their identity is held by the school that entered them">
              Visiting from {homeOf(row)}
            </span>
          ),
      },
      {
        header: "Also at",
        render: (row) => {
          const others = (attended.get(row.id) ?? []).filter((id) => id !== school.id);
          if (others.length === 0) return <span className="absent">This school only</span>;
          return (
            <span className="shareList">
              <span className="chipBadge">shared</span>
              {others.map((id) => (
                <span key={id} className="shareName">
                  {schoolName.get(id) ?? "another school"}
                </span>
              ))}
            </span>
          );
        },
      },
    ],
    fields: [
      { kind: "text", key: "givenName", label: "First name", required: true },
      { kind: "text", key: "familyName", label: "Last name", required: true },
      {
        kind: "text",
        key: "code",
        label: "Admission number",
        hint: "This school's own, and optional. Editing it here rewrites this school's current enrolment too, so the two cannot disagree.",
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
        label: "In this campus file",
        hint: "The project-level flag — clearing it archives the person everywhere. Whether this school still has them is their enrolment, below, and the two are deliberately different questions.",
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
      loadRoll().then((rows) => {
        for (const row of rows) noteGroups(row.id);
        return rows;
      }),
    // The panel re-reads the list straight afterwards, so the returned row is never what
    // gets rendered; the spell is supplied so `create` and `update` satisfy the spec
    // without inventing a second source of truth. `create_student` writes a home spell
    // in the same transaction, and `update_student` mirrors onto the existing one.
    create: (input) => createStudent(input).then((row) => ({ ...row, enrolment: null })),
    update: (id, input) =>
      updateStudent(id, input).then((row) => ({
        ...row,
        enrolment: (roll.items ?? []).find((known) => known.id === id)?.enrolment ?? null,
      })),
    remove: (id) => deleteStudent(school.id, id),
    rowActions: [
      { label: "Enrolment", onClick: (row) => setChosenId(row.id) },
      {
        label: "Go to home school",
        onClick: (row) => selectSchool?.(row.schoolId),
        blockedBy: (row) =>
          row.schoolId === school.id
            ? "This school holds their identity, so Edit above is the way to change it."
            : selectSchool === undefined
              ? "Switching schools is unavailable here."
              : null,
      },
    ],
    editBlockedBy: (row) =>
      row.schoolId === school.id
        ? null
        : `${row.givenName} ${row.familyName} belongs to ${homeOf(row)}, which holds their name and contact details. This school's placement for them is editable under “Enrolment” below.`,
    deleteBlockedBy: (row) => {
      if (row.schoolId !== school.id) {
        return (
          <>
            <p>
              Their identity belongs to <strong>{homeOf(row)}</strong>, and only the school
              that entered somebody may delete them.
            </p>
            <p className="muted">
              What this school can do is end its own enrolment: <strong>Enrolment</strong>{" "}
              below records that they have left, and keeps the spell as history. The person
              stays with the school that holds their identity.
            </p>
          </>
        );
      }
      const others = (attended.get(row.id) ?? []).filter((id) => id !== school.id);
      const groups = groupsByStudent[row.id] ?? [];
      if (others.length === 0 && groups.length === 0) return null;
      return (
        <>
          {others.length > 0 && (
            <p>
              {others.length === 1
                ? "Another school in this campus file holds a spell for them"
                : `${others.length} other schools in this campus file hold a spell for them`}
              :{" "}
              <strong>
                {others.map((id) => schoolName.get(id) ?? "another school").join(", ")}
              </strong>
              . Each of those schools lets go of its own enrolment first, and the delete is
              then offered.
            </p>
          )}
          {groups.length > 0 && (
            <p>
              They are in {groups.length} group{groups.length === 1 ? "" : "s"} (
              {groups.map((group) => group.name).join(", ")}). A membership is the only
              record that they were ever in one, so it has to be removed deliberately under{" "}
              <strong>Student groups</strong> rather than taken by a delete.
            </p>
          )}
          <p className="muted">
            If they have simply left, <strong>Enrolment</strong> below records it and keeps
            everything — which is almost always what was meant.
          </p>
        </>
      );
    },
    emptyHint:
      "No students yet — and a timetable does not need any. Add them when you want per-student timetables; if somebody already attends another school in this campus file, enrol them rather than entering them twice.",
    deleteNote: (row) =>
      row.isActive
        ? "If this student has left, “Mark as left” under Enrolment keeps them and their history instead. Their own spells go with them if you delete."
        : "Their enrolment spells go with them.",
  };

  /** Students in the project this school has never enrolled. */
  const candidates: readonly Candidate[] | null = useMemo(() => {
    if (known === null) return null;
    const here = new Set((roll.items ?? []).map((row) => row.id));
    return known
      .filter((person) => !here.has(person.id))
      .map((person) => ({
        id: person.id,
        name: `${person.familyName}, ${person.givenName}`,
        details: [
          person.code === null ? null : `Admission number ${person.code}`,
          person.email,
          `Identity held by ${schoolName.get(person.schoolId) ?? "another school"}`,
          `Spells at ${(attended.get(person.id) ?? [])
            .map((id) => schoolName.get(id) ?? "another school")
            .join(", ")}`,
          person.isActive ? null : "Archived in this campus file",
        ].filter((detail): detail is string => detail !== null && detail.length > 0),
      }));
  }, [known, roll.items, attended, schoolName]);

  const [picked, setPicked] = useState<string | null>(null);
  const [addError, setAddError] = useState<string | null>(null);
  const [addBusy, setAddBusy] = useState(false);

  const closeAdd = useCallback(() => {
    setAdding(false);
    setPicked(null);
    setAddError(null);
  }, []);

  const confirmAdd = useCallback(() => {
    if (picked === null) return;
    setAddBusy(true);
    setAddError(null);
    void enrolStudent({
      studentId: picked,
      schoolId: school.id,
      // Placement is left for the enrolment panel, which opens straight afterwards. A
      // dialogue that asked for an admission number, a year level and a campus before it
      // would be the enrolment form twice.
      admissionNo: null,
      yearLevelId: null,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: null,
      leftOn: null,
    })
      .then(() => {
        closeAdd();
        refreshAll();
        setChosenId(picked);
      })
      .catch((cause: unknown) => setAddError(describeSetupError(cause)))
      .finally(() => setAddBusy(false));
  }, [picked, school.id, closeAdd, refreshAll]);

  const chosen = (roll.items ?? []).find((row) => row.id === chosenId) ?? null;
  const visiting = (roll.items ?? []).filter((row) => row.schoolId !== school.id).length;

  return (
    <section className="sectionBody">
      <p className="muted">
        Students are <strong>optional</strong>. A complete timetable can be built from group
        sizes alone, so enter students only when you want per-student timetables. One
        student attending two schools of a trust is{" "}
        <strong>one record with one identifier</strong> and two enrolments.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      {roll.error && (
        <Banner tone="error">
          This school&rsquo;s enrolments could not be loaded: {roll.error}
        </Banner>
      )}
      {graphError && (
        <Banner tone="warning">
          Other schools&rsquo; rolls could not be read, so enrolling an existing student and
          the delete warnings are unavailable: {graphError}
        </Banner>
      )}

      <CrudPanel
        spec={yearSpec}
        reloadKey={school.id}
        onChanged={() => {
          yearLevels.reload();
          roll.reload();
        }}
      />

      {schools.length > 1 && (
        <div className="actions">
          <button
            type="button"
            className="secondary"
            disabled={candidates === null || candidates.length === 0}
            title={
              candidates !== null && candidates.length === 0
                ? "Every student in this campus file already has a spell at this school."
                : undefined
            }
            onClick={() => setAdding(true)}
          >
            Enrol an existing student
          </button>
          <span className="panelNote" role="note">
            Somebody already in this campus file gains a second enrolment — concurrent
            enrolment, not a transfer, and not a second student record.
          </span>
        </div>
      )}

      {visiting > 0 && (
        <Banner tone="info">
          {visiting === 1
            ? "One student on this roll was entered by another school"
            : `${visiting} students on this roll were entered by another school`}
          . Their names and contact details belong to that school; this school&rsquo;s
          placement for them is editable under <strong>Enrolment</strong> below.
        </Banner>
      )}

      <CrudPanel
        spec={studentSpec}
        reloadKey={`${school.id}:${(yearLevels.items ?? []).length}:${(campuses.items ?? []).length}`}
        onChanged={refreshAll}
      />

      <StudentEnrolments
        school={school}
        schools={schools}
        student={chosen}
        yearLevels={yearLevels.items ?? []}
        campuses={campuses.items ?? []}
        onChanged={refreshAll}
      />

      <ConfirmDialog
        open={adding}
        title="Enrol an existing student at this school"
        confirmLabel="Enrol at this school"
        busy={addBusy}
        confirmBlockedBy={picked === null ? "Choose a student from the list first." : null}
        onConfirm={confirmAdd}
        onCancel={closeAdd}
      >
        <p>
          This opens a spell at <strong>{school.name}</strong> for somebody already in this
          campus file. No second student record is created: they keep their identifier, their
          groups, and every other school&rsquo;s enrolment of them.
        </p>
        <p className="muted">
          This is <strong>concurrent enrolment</strong> — they will attend both schools. To
          move somebody instead, cancel and use <strong>Transfer</strong> under Enrolment on
          the school they are leaving.
        </p>
        {addError && <Banner tone="error">{addError}</Banner>}
        <PersonSearch
          legend="Students already in this campus file"
          candidates={candidates}
          error={graphError}
          selectedId={picked}
          onSelect={setPicked}
          disabled={addBusy}
          emptyTitle="Nobody to enrol"
          emptyHint="Every student in this campus file already has a spell at this school. Use “Add student” above to enter somebody new."
        />
        <p className="muted">
          The admission number, year level and campus are set straight afterwards, under
          Enrolment. Not finding them? Evara never merges two records because they share a
          name — use <strong>Add student</strong> for somebody new.
        </p>
      </ConfirmDialog>
    </section>
  );
}
