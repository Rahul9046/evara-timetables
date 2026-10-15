/**
 * Section 9 — Subjects.
 *
 * ## Why this exists one stage before anything schedules it
 *
 * Nothing in Evara reads a subject yet. Stage C's lessons do — `course.subject_id` —
 * and they cannot be designed around a table that does not exist, which is why the
 * roadmap puts the curriculum prerequisites here rather than inside the stage that
 * consumes them. What a school enters now is referenced by identifier later, and those
 * identifiers are stable (UUIDv7, never renumbered), so this is work a school does once.
 *
 * ## Two schools' Mathematics are two subjects
 *
 * This is the whole multi-school point of the screen and the one thing it must not get
 * wrong. Northgate's Mathematics and Southmoor's Mathematics are separate records with
 * separate identifiers; they may share a name *and* a code without colliding, because the
 * uniqueness rules are `UNIQUE (school_id, name)` and a partial index on
 * `(school_id, code)`. Nothing merges them on a matching name, and nothing should: they may
 * sit in different departments and, from Stage C, carry different courses.
 *
 * Switching schools therefore re-reads the list rather than filtering one — the shell
 * remounts the section (ADR 0023 §2) and every call here names `school.id`.
 *
 * ## The department picker offers this school's departments, and only those
 *
 * A subject's department is optional and must belong to the same school. Both halves are
 * enforced three deep: this picker is built from `listDepartments(school.id)`, the
 * repository refuses a department from elsewhere by name, and `V14`'s trigger pair refuses
 * it at the storage layer. A school with no departments is told so and offered the section,
 * rather than blocked — the field is optional, so advice is the right response.
 *
 * ## Who can teach it
 *
 * A per-subject panel lists the teachers qualified for it. That list is **project-wide in
 * the people it shows and school-scoped in the subject it asks about**: a visiting teacher
 * qualified for this school's Chemistry appears, because she is. Adding and removing
 * qualifications is done from the Teachers screen, where the person is the subject of the
 * screen rather than the object of it — see
 * [ADR 0025](../../../../../docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md).
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Department,
  QualifiedTeacher,
  School,
  Subject,
  SubjectDependents,
  SubjectInput,
} from "@evara/domain";
import {
  createSubject,
  deleteSubject,
  listDepartments,
  listSubjectTeachers,
  listSubjects,
  subjectDependents,
  updateSubject,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner, EmptyState, Loading } from "../ui/Shell";
import { problems, required, text, textOrNull, type Draft } from "../validation";
import type { SetupContext } from "./SchoolSetup";
import { useList } from "./useList";

export function Subjects(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const openSection = props.context?.openSection;

  const loadDepartments = useCallback(() => listDepartments(school.id), [school.id]);
  const departments = useList<Department>(loadDepartments);

  /** Which subject the qualified-teachers panel is about. */
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [listNonce, setListNonce] = useState(0);

  /**
   * How many teachers are qualified for each subject.
   *
   * Read per row as the table renders, for the same reason Departments reads its subject
   * counts up front: `deleteBlockedBy` must answer synchronously, because the confirmation
   * decides from it whether to offer a destructive button at all.
   */
  const [reports, setReports] = useState<ReadonlyMap<string, SubjectDependents>>(new Map());

  const noteDependents = useCallback((id: string) => {
    void subjectDependents(id)
      .then((report) => setReports((current) => new Map(current).set(id, report)))
      .catch(() => undefined);
  }, []);

  const departmentName = useMemo(
    () => new Map((departments.items ?? []).map((row) => [row.id, row.name])),
    [departments.items],
  );

  const departmentChoices: readonly Choice[] = useMemo(
    () => (departments.items ?? []).map((row) => ({ value: row.id, label: row.name })),
    [departments.items],
  );

  const spec: CrudSpec<Subject, SubjectInput> = {
    noun: "subject",
    plural: "subjects",
    title: "Subjects",
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Code", render: (row) => row.code ?? "—" },
      {
        header: "Department",
        render: (row) =>
          row.departmentId === null ? (
            <span className="absent">No department</span>
          ) : (
            (departmentName.get(row.departmentId) ?? (
              <span className="absent">Another school&rsquo;s</span>
            ))
          ),
      },
      {
        header: "Qualified",
        numeric: true,
        render: (row) => String(reports.get(row.id)?.qualifiedTeachers ?? 0),
      },
    ],
    fields: [
      {
        kind: "text",
        key: "name",
        label: "Subject name",
        required: true,
        placeholder: "Mathematics",
      },
      {
        kind: "text",
        key: "code",
        label: "Subject code",
        hint: "Optional. Unique within this school when given — another school in this campus file may use the same code for its own subject.",
      },
      {
        kind: "select",
        key: "departmentId",
        label: "Department",
        emptyLabel: "No department",
        choices: departmentChoices,
        hint:
          departmentChoices.length === 0
            ? "This school has no departments yet. A subject does not need one — leave this as it is, or add departments in section 8 first."
            : "Optional, and only this school's departments are offered. A subject cannot be taught by another school's department.",
      },
      {
        kind: "textarea",
        key: "description",
        label: "Description",
        hint: "Optional.",
      },
    ],
    describe: (row) => row.name,
    validate: (draft: Draft) =>
      problems([["name", required(text(draft, "name"), "Subject name")]]),
    toDraft: (row) => ({
      name: row?.name ?? "",
      code: row?.code ?? "",
      departmentId: row?.departmentId ?? null,
      description: row?.description ?? "",
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      departmentId: textOrNull(draft, "departmentId"),
      name: text(draft, "name"),
      code: textOrNull(draft, "code"),
      description: textOrNull(draft, "description"),
    }),
    list: () =>
      listSubjects(school.id).then((rows) => {
        for (const row of rows) noteDependents(row.id);
        return rows;
      }),
    create: (input) => createSubject(input),
    update: (row, input) => updateSubject(row.id, row.rev, input),
    remove: (row) => deleteSubject(school.id, row.id, row.rev),
    emptyHint: (
      <>
        No subjects yet. Add what this school teaches — Mathematics, Physics, English
        Literature, History.
        <br />
        They belong to this school alone, so another school in this campus file keeps its own
        list; the two may use the same names and codes without colliding. Stage C&rsquo;s
        lessons are taught <em>of</em> a subject, so this is the list they will be chosen
        from.
      </>
    ),
    rowActions: [{ label: "Qualified teachers", onClick: (row) => setChosenId(row.id) }],
    deleteBlockedBy: (row) => {
      const qualified = reports.get(row.id)?.qualifiedTeachers ?? 0;
      if (qualified === 0) return null;
      return (
        <>
          <p>
            {qualified === 1
              ? "1 teacher is qualified to teach it"
              : `${qualified} teachers are qualified to teach it`}
            .
          </p>
          <p className="muted">
            A qualification is a record about a person, so deleting the subject will not
            quietly discard it. Remove the qualifications under <strong>Teachers</strong> —
            each one explicitly — and the delete is then offered.
          </p>
        </>
      );
    },
  };

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Subjects</h2>
        <p>
          What this school teaches. A subject is <strong>this school&rsquo;s own</strong>: a
          campus file with two schools holds two lists, and &ldquo;Mathematics&rdquo; in one
          is a different record from &ldquo;Mathematics&rdquo; in the other, even with the
          same code.
        </p>
        <p className="muted">
          Nothing is scheduled against a subject yet. Stage C&rsquo;s lessons reference them
          by identifier, and those identifiers never change — so a school can enter its
          curriculum now and have it still mean the same thing then.
        </p>
      </header>

      {departments.error !== null && (
        <Banner tone="warning">
          This school&rsquo;s departments could not be loaded, so the department picker is
          empty: {departments.error}
        </Banner>
      )}

      {departments.items !== null && departments.items.length === 0 && (
        <p className="panelNote" role="note">
          This school has no departments yet, so every subject will be added without one.
          That is a complete answer — a department is optional.{" "}
          {openSection !== undefined && (
            <button
              type="button"
              className="linkish"
              onClick={() => openSection("departments")}
            >
              Add departments first
            </button>
          )}
        </p>
      )}

      <CrudPanel
        spec={spec}
        reloadKey={school.id}
        onChanged={() => setListNonce((value) => value + 1)}
      />

      {/* Keyed by the chosen subject and by the list's own nonce, so the panel re-reads
          when the selection moves and when the subject list changes underneath it — a
          subject that has just been deleted must not go on offering its qualified staff. */}
      <QualifiedTeachers
        key={`${chosenId ?? "none"}:${listNonce}`}
        school={school}
        subject={chosenId}
        openSection={openSection}
      />
    </div>
  );
}

/**
 * Who can teach one subject.
 *
 * Read-only on purpose. Qualifying somebody is an edit to a *person*, and the screen where
 * a person is the subject of the sentence is Teachers — which is also the only screen that
 * can show the whole picture of what one teacher can teach, across every school. Offering
 * the same write from both places would mean two flows to keep honest about membership,
 * authority and what survives a departure.
 */
function QualifiedTeachers(props: {
  school: School;
  subject: string | null;
  openSection?: (id: string) => void;
}) {
  const { school, subject, openSection } = props;

  const load = useCallback(
    () =>
      subject === null
        ? Promise.resolve<QualifiedTeacher[]>([])
        : listSubjectTeachers(school.id, subject),
    [school.id, subject],
  );
  const qualified = useList<QualifiedTeacher>(load);

  if (subject === null) {
    return (
      <div className="crudPanel">
        <div className="panelHead">
          <h3>Qualified teachers</h3>
        </div>
        <EmptyState title="No subject chosen">
          Choose <strong>Qualified teachers</strong> on a subject above to see who can teach
          it.
        </EmptyState>
      </div>
    );
  }

  return (
    <div className="crudPanel">
      <div className="panelHead">
        <h3>Qualified teachers</h3>
      </div>

      {qualified.error !== null && <Banner tone="error">{qualified.error}</Banner>}

      {qualified.items === null ? (
        <Loading what="qualified teachers" />
      ) : qualified.items.length === 0 ? (
        <EmptyState title="Nobody is qualified for this subject yet">
          <p>
            A qualification says that a particular person can teach this particular subject.
            Nothing is implied by a name: somebody qualified for another school&rsquo;s
            Mathematics is not qualified for this one.
          </p>
          <p className="muted">
            Add one from <strong>Teachers</strong>, under &ldquo;Subject
            qualifications&rdquo; on the teacher it is about.
            {openSection !== undefined && (
              <>
                {" "}
                <button
                  type="button"
                  className="linkish"
                  onClick={() => openSection("teachers")}
                >
                  Open Teachers
                </button>
              </>
            )}
          </p>
        </EmptyState>
      ) : (
        <table className="dataTable">
          <caption className="srOnly">Teachers qualified to teach this subject</caption>
          <thead>
            <tr>
              <th scope="col">Last name</th>
              <th scope="col">First name</th>
              <th scope="col">Staff code</th>
              <th scope="col">Status</th>
            </tr>
          </thead>
          <tbody>
            {qualified.items.map((entry) => (
              <tr key={entry.qualification.id}>
                <td>{entry.teacher.familyName}</td>
                <td>{entry.teacher.givenName}</td>
                <td>{entry.teacher.code ?? "—"}</td>
                <td>
                  {entry.teacher.isActive ? (
                    "Active"
                  ) : (
                    <span title="Archived in this campus file. Their qualification is kept — see ADR 0025.">
                      Archived
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <p className="panelNote" role="note">
        Qualifications are added and removed on the <strong>Teachers</strong> screen, where
        the person is what is being edited. A qualification outlives employment, so somebody
        who has left this school may still appear here.
      </p>
    </div>
  );
}
