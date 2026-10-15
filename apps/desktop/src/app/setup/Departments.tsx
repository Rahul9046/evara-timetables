/**
 * Section 8 — Departments.
 *
 * ## What a department is for, and what it is not
 *
 * It is how one school organises the teaching of its own curriculum: Mathematics,
 * Humanities, Design and Technology. Nothing in scheduling reads it yet — Phase 4's
 * constraints will — so it exists here for two reasons that are already real: a subject can
 * say which unit teaches it, and a teacher can say which units they work in.
 *
 * It is **not** a division of the school in the structural sense. That question was asked
 * and settled: [ADR 0023](../../../../../docs/adr/0023-the-interface-scopes-to-one-school.md)
 * §7 rejected a `program` entity below the school, and a department is not one by the back
 * door. It classifies teaching; it owns no campus, no year level and no timetable.
 *
 * ## School-owned, and never shared
 *
 * Two schools in one campus file may each hold a Mathematics Department, as two records
 * with two identifiers, and neither sees the other's. That is the same treatment a year
 * level gets and for the same reason — this says how *one* institution is arranged — so
 * every call here names `school.id` explicitly and `UNIQUE (school_id, name)` keeps a
 * duplicate within one school refused.
 *
 * ## Deleting one is refused rather than performed quietly
 *
 * A department with subjects in it, or staff assigned to it, cannot be deleted. The counts
 * are read *before* Delete is offered — `departmentDependents` — so the confirmation
 * explains what is in the way instead of the button failing, which is the pattern Stage 0C
 * made ordinary (ADR 0023 §4).
 *
 * This departs from ADR 0010's "optional classification, therefore `SET NULL`" rule, which
 * a department otherwise looks like. [ADR
 * 0025](../../../../../docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md)
 * §6 argues it: a database-level `SET NULL` would rewrite every affected subject without
 * incrementing a single revision, which is R10 in OPEN-DECISIONS and contradicts the
 * contract the rest of this stage installs.
 */
import { useCallback, useMemo, useState } from "react";

import type { Department, DepartmentDependents, DepartmentInput, School } from "@evara/domain";
import {
  createDepartment,
  deleteDepartment,
  departmentDependents,
  listDepartments,
  listSubjects,
  updateDepartment,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { problems, required, text, textOrNull, type Draft } from "../validation";
import type { SetupContext } from "./SchoolSetup";
import { useList } from "./useList";

/** A department, with what it holds. */
export function Departments(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const openSection = props.context?.openSection;

  /**
   * How many subjects each department teaches.
   *
   * One read of the school's subjects rather than a dependent report per row: the Subjects
   * column is on screen for every department at once, and thirteen round trips to render
   * one table is the kind of thing that looks fine with three departments and is noticeable
   * with thirty. The authoritative count still comes from `departmentDependents`, which is
   * read for the one row a delete is about.
   */
  const loadSubjects = useCallback(() => listSubjects(school.id), [school.id]);
  const subjects = useList(loadSubjects);

  const subjectCount = useMemo(() => {
    const counts = new Map<string, number>();
    for (const subject of subjects.items ?? []) {
      if (subject.departmentId === null) continue;
      counts.set(subject.departmentId, (counts.get(subject.departmentId) ?? 0) + 1);
    }
    return counts;
  }, [subjects.items]);

  /**
   * The dependent report for each row a delete has been offered on.
   *
   * Read when the row is rendered rather than when Delete is pressed, because
   * `deleteBlockedBy` has to answer synchronously — the dialogue decides whether to offer a
   * destructive button from it.
   *
   * A row whose report has not arrived yet falls back to the subject count above, which
   * knows nothing about teacher associations. So in that window a delete can be *offered*
   * and then refused by the backend, with its own message. That is the ordinary pre-Stage-B
   * behaviour rather than a regression, the window is one round trip from the list read
   * beside it, and the alternative — blocking Delete until a count arrives — would make a
   * delete look impossible whenever a read was slow. Every list read re-reads every row's
   * report, so the fallback is never used for a row the user has been looking at.
   */
  const [reports, setReports] = useState<ReadonlyMap<string, DepartmentDependents>>(new Map());

  const noteDependents = useCallback((id: string) => {
    void departmentDependents(id)
      .then((report) => setReports((current) => new Map(current).set(id, report)))
      // A failed count is not worth a banner: the delete still refuses, with its own
      // message, and the only thing lost is the explanation arriving a moment earlier.
      .catch(() => undefined);
  }, []);

  const spec: CrudSpec<Department, DepartmentInput> = {
    noun: "department",
    plural: "departments",
    title: "Departments",
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Code", render: (row) => row.code ?? "—" },
      {
        header: "Subjects",
        numeric: true,
        render: (row) => String(subjectCount.get(row.id) ?? 0),
      },
      {
        header: "Description",
        render: (row) =>
          row.description === null ? <span className="absent">—</span> : row.description,
      },
    ],
    fields: [
      {
        kind: "text",
        key: "name",
        label: "Name",
        required: true,
        placeholder: "Mathematics",
      },
      {
        kind: "text",
        key: "code",
        label: "Code",
        hint: "Optional. Unique within this school when given — a school that does not code its departments can leave every one of these blank.",
      },
      {
        kind: "textarea",
        key: "description",
        label: "Description",
        hint: "Optional. Whatever this school needs to remember about the department.",
      },
    ],
    describe: (row) => row.name,
    validate: (draft: Draft) =>
      problems([["name", required(text(draft, "name"), "Department name")]]),
    toDraft: (row) => ({
      name: row?.name ?? "",
      code: row?.code ?? "",
      description: row?.description ?? "",
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      name: text(draft, "name"),
      code: textOrNull(draft, "code"),
      description: textOrNull(draft, "description"),
    }),
    list: () =>
      listDepartments(school.id).then((rows) => {
        for (const row of rows) noteDependents(row.id);
        return rows;
      }),
    create: (input) => createDepartment(input),
    update: (row, input) => updateDepartment(row.id, row.rev, input),
    remove: (row) => deleteDepartment(school.id, row.id, row.rev),
    emptyHint: (
      <>
        No departments yet. Add the units this school organises its teaching into —
        Mathematics, Science, Humanities, whatever this school calls them.
        <br />
        They belong to this school alone, so another school in this campus file can hold its
        own Mathematics Department without colliding. A department is optional: subjects can
        be added without one.
      </>
    ),
    deleteBlockedBy: (row) => {
      const report = reports.get(row.id);
      const subjectsHere = report?.subjects ?? subjectCount.get(row.id) ?? 0;
      const teachersHere = report?.teachers ?? 0;
      if (subjectsHere + teachersHere === 0) return null;
      return (
        <>
          <p>
            {[
              subjectsHere === 0
                ? null
                : subjectsHere === 1
                  ? "1 subject is taught by it"
                  : `${subjectsHere} subjects are taught by it`,
              teachersHere === 0
                ? null
                : teachersHere === 1
                  ? "1 teacher belongs to it"
                  : `${teachersHere} teachers belong to it`,
            ]
              .filter((part): part is string => part !== null)
              .join(", and ")}
            .
          </p>
          <p className="muted">
            Nothing is unclassified or removed on your behalf. Change those subjects&rsquo;
            department in <strong>Subjects</strong>, and take those teachers out of it under{" "}
            <strong>Teachers</strong>, and the delete is then offered.
          </p>
          {openSection !== undefined && subjectsHere > 0 && (
            <p>
              <button
                type="button"
                className="linkish"
                onClick={() => openSection("subjects")}
              >
                Open Subjects
              </button>
            </p>
          )}
        </>
      );
    },
  };

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Departments</h2>
        <p>
          How this school organises the teaching of its curriculum. A department groups
          subjects, and staff can belong to one or more of them — which is what Phase
          4&rsquo;s constraints will read when they balance load across a faculty.
        </p>
        <p className="muted">
          Departments are <strong>this school&rsquo;s own</strong> and are never shared. They
          are also optional: a school that does not think in departments can go straight to{" "}
          <strong>Subjects</strong> and leave every one of them unassigned.
        </p>
      </header>

      {subjects.error !== null && (
        <p className="muted">
          Subject counts could not be loaded, so the Subjects column may be out of date:{" "}
          {subjects.error}
        </p>
      )}

      <CrudPanel
        spec={spec}
        reloadKey={school.id}
        onChanged={() => {
          subjects.reload();
        }}
      />
    </div>
  );
}
