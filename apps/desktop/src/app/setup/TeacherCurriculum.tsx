/**
 * One teacher's departments and subject qualifications.
 *
 * Sits below {@link TeacherSchools} on the Teachers screen and is about the same person,
 * but about two different kinds of fact — and keeping them apart is the whole of
 * [ADR 0025](../../../../../docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md).
 *
 * ## Four things, deliberately not conflated
 *
 * | | What it says | Scope |
 * | --- | --- | --- |
 * | Identity | who this person is | the project |
 * | School membership | which schools employ them | per school |
 * | Campus eligibility | which sites they may work at | a narrowing of membership |
 * | **Department** | which of a school's units they work in | a narrowing of membership |
 * | **Qualification** | what they can teach | the person |
 *
 * The first three are {@link TeacherSchools}. This panel is the last two, and the line
 * between them is the one a user has to be able to see:
 *
 * - A **department association** is a statement about one school's internal organisation.
 *   It requires membership of that school, and withdrawing the membership withdraws it.
 * - A **qualification** is a statement about the person. It requires no membership, it
 *   survives a departure, and nothing short of deleting the person removes it — least of
 *   all deactivating them, changing their campuses or editing their name.
 *
 * ## Both lists are project-wide, and only this school's rows are editable
 *
 * A teacher in Mathematics at Northgate and Science at Southmoor has two true answers, and
 * a panel showing one of them would hide the arrangement the table exists to express. The
 * same goes for qualifications, including at schools she has left — those are precisely the
 * rows somebody would otherwise enter a second time.
 *
 * So both are grouped by owning school, and rows belonging to another school are shown with
 * their controls disabled and an offer to switch. ADR 0023 §5's rule: *visible, explained,
 * not silently editable.*
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Department,
  QualifiedSubject,
  School,
  Subject,
  Teacher,
  TeacherDepartmentView,
} from "@evara/domain";
import {
  addQualification,
  addTeacherDepartment,
  describeSetupError,
  listDepartments,
  listSubjects,
  listTeacherDepartments,
  listQualifications,
  removeQualification,
  removeTeacherDepartment,
} from "../../ipc/setup";
import { Banner, ConfirmDialog, EmptyState, Loading, Panel } from "../ui/Shell";
import { useList } from "./useList";

/** A pending removal, held so the confirmation and the call cannot disagree. */
type Pending =
  | { kind: "department"; row: TeacherDepartmentView }
  | { kind: "qualification"; row: QualifiedSubject };

export function TeacherCurriculum(props: {
  /** The school whose authority the edits are made under. */
  school: School;
  /** Every school in the project, so another school's rows can be named. */
  schools: readonly School[];
  /** Whose curriculum this is, or `null` when no teacher is chosen. */
  teacher: Teacher | null;
  /** Switches the setup shell to another school. */
  onSelectSchool?: (id: string) => void;
  /** Called after any change, so the Teachers list can re-read. */
  onChanged?: () => void;
}) {
  const { school, schools, teacher, onSelectSchool, onChanged } = props;

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);

  const teacherId = teacher?.id ?? null;

  const loadDepartments = useCallback(
    () =>
      teacherId === null
        ? Promise.resolve<TeacherDepartmentView[]>([])
        : listTeacherDepartments(teacherId),
    [teacherId],
  );
  const held = useList<TeacherDepartmentView>(loadDepartments);

  const loadQualifications = useCallback(
    () =>
      teacherId === null
        ? Promise.resolve<QualifiedSubject[]>([])
        : listQualifications(teacherId),
    [teacherId],
  );
  const qualifications = useList<QualifiedSubject>(loadQualifications);

  const loadOffered = useCallback(() => listDepartments(school.id), [school.id]);
  const offered = useList<Department>(loadOffered);

  const loadSubjects = useCallback(() => listSubjects(school.id), [school.id]);
  const subjects = useList<Subject>(loadSubjects);

  const schoolName = useMemo(
    () => new Map(schools.map((candidate) => [candidate.id, candidate.name])),
    [schools],
  );
  const nameOf = (id: string) => schoolName.get(id) ?? "another school";

  const departmentNames = useMemo(
    () => new Map((offered.items ?? []).map((row) => [row.id, row.name])),
    [offered.items],
  );

  /** Re-reads everything this panel shows, plus whatever the parent depends on. */
  const refresh = useCallback(() => {
    held.reload();
    qualifications.reload();
    onChanged?.();
  }, [held, qualifications, onChanged]);

  const run = useCallback(
    (work: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      void work()
        .then(() => {
          setPending(null);
          refresh();
        })
        .catch((cause: unknown) => {
          setPending(null);
          setError(describeSetupError(cause));
        })
        .finally(() => setBusy(false));
    },
    [refresh],
  );

  if (teacher === null) {
    return (
      <Panel title="Departments and subject qualifications">
        {/* Deliberately not "No teacher chosen", which is what the two panels either side
            of this one say. Three identical boxes stacked down a screen tell the reader
            nothing about which selection each is waiting for. */}
        <EmptyState title="Choose a teacher above">
          <strong>Curriculum</strong> on any row shows which departments they work in and
          which subjects they are qualified to teach — at every school, not only this one.
        </EmptyState>
      </Panel>
    );
  }

  const named = `${teacher.givenName} ${teacher.familyName}`;
  /** Departments of this school the teacher is not already in. */
  const addableDepartments = (offered.items ?? []).filter(
    (candidate) => !(held.items ?? []).some((row) => row.department.id === candidate.id),
  );
  /** Subjects of this school they are not already qualified for. */
  const addableSubjects = (subjects.items ?? []).filter(
    (candidate) => !(qualifications.items ?? []).some((row) => row.subject.id === candidate.id),
  );

  return (
    <Panel
      title={`Departments and subject qualifications — ${named}`}
      note={
        <>
          Two different kinds of fact about the same person, kept apart on purpose. A{" "}
          <strong>department</strong> says where they sit in one school&rsquo;s organisation
          and needs that school&rsquo;s employment; a <strong>qualification</strong> says
          what they can teach and outlives it.
        </>
      }
    >
      {error !== null && <Banner tone="error">{error}</Banner>}

      <Departments
        school={school}
        named={named}
        held={held.items}
        error={held.error}
        addable={addableDepartments}
        departmentNames={departmentNames}
        nameOf={nameOf}
        busy={busy}
        onAdd={(departmentId) =>
          run(() => addTeacherDepartment(school.id, teacher.id, departmentId))
        }
        onRemove={(row) => setPending({ kind: "department", row })}
        onSelectSchool={onSelectSchool}
      />

      <Qualifications
        school={school}
        named={named}
        rows={qualifications.items}
        error={qualifications.error}
        addable={addableSubjects}
        departmentNames={departmentNames}
        nameOf={nameOf}
        busy={busy}
        onAdd={(subjectId) => run(() => addQualification(school.id, teacher.id, subjectId))}
        onRemove={(row) => setPending({ kind: "qualification", row })}
        onSelectSchool={onSelectSchool}
      />

      <ConfirmDialog
        open={pending !== null}
        title={
          pending?.kind === "department"
            ? `Take ${named} out of ${pending.row.department.name}?`
            : pending?.kind === "qualification"
              ? `Remove ${named}'s qualification for ${pending.row.subject.name}?`
              : ""
        }
        confirmLabel={
          pending?.kind === "department" ? "Remove from department" : "Remove qualification"
        }
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          if (pending.kind === "department") {
            run(() =>
              removeTeacherDepartment(
                school.id,
                teacher.id,
                pending.row.department.id,
                pending.row.association.rev,
              ),
            );
          } else {
            run(() =>
              removeQualification(
                school.id,
                teacher.id,
                pending.row.subject.id,
                pending.row.qualification.rev,
              ),
            );
          }
        }}
        onCancel={() => setPending(null)}
      >
        {pending?.kind === "department" && (
          <>
            <p>
              <strong>{named}</strong> stops being part of{" "}
              <strong>{pending.row.department.name}</strong>. They still work at{" "}
              <strong>{nameOf(pending.row.department.schoolId)}</strong>, and every subject
              they are qualified to teach is untouched.
            </p>
            <p className="muted">
              A department association says where somebody sits in a school&rsquo;s
              organisation. What they can teach is a separate record and is not changed by
              this.
            </p>
          </>
        )}
        {pending?.kind === "qualification" && (
          <>
            <p>
              <strong>{named}</strong> is recorded as no longer qualified to teach{" "}
              <strong>{pending.row.subject.name}</strong> at{" "}
              <strong>{nameOf(pending.row.subject.schoolId)}</strong>.
            </p>
            <p className="muted">
              Nothing else changes: not their employment, not their campuses, not their
              departments, and not any qualification at another school &mdash; including one
              for a subject of the same name.
            </p>
          </>
        )}
      </ConfirmDialog>
    </Panel>
  );
}

/** The departments half. */
function Departments(props: {
  school: School;
  named: string;
  held: readonly TeacherDepartmentView[] | null;
  error: string | null;
  addable: readonly Department[];
  departmentNames: ReadonlyMap<string, string>;
  nameOf: (id: string) => string;
  busy: boolean;
  onAdd: (departmentId: string) => void;
  onRemove: (row: TeacherDepartmentView) => void;
  onSelectSchool?: (id: string) => void;
}) {
  const { school, named, held, error, addable, nameOf, busy, onAdd, onRemove } = props;
  const [picked, setPicked] = useState("");

  return (
    <section className="subSection">
      <h4 className="subHead">Departments</h4>
      <p className="muted">
        Which of a school&rsquo;s units they work in. A teacher may be in more than one
        &mdash; Mathematics and Physics is ordinary &mdash; and in different ones at
        different schools.
      </p>

      {error !== null && <Banner tone="error">{error}</Banner>}

      {held === null ? (
        <Loading what="departments" />
      ) : held.length === 0 ? (
        <EmptyState title="Not in any department">
          <p>
            <strong>{named}</strong> is not recorded in a department at any school. That is a
            complete answer: a department association is optional, and nothing is blocked by
            its absence.
          </p>
        </EmptyState>
      ) : (
        <table className="dataTable">
          <caption className="srOnly">Departments this teacher belongs to</caption>
          <thead>
            <tr>
              <th scope="col">Department</th>
              <th scope="col">School</th>
              <th scope="col">Code</th>
              <th scope="col" className="rowActions">
                <span className="srOnly">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {held.map((row) => {
              const isThis = row.department.schoolId === school.id;
              return (
                <tr key={row.association.id}>
                  <td>{row.department.name}</td>
                  <td>
                    {isThis ? (
                      "This school"
                    ) : (
                      <span className="shareList">
                        <span className="chipBadge">other</span>
                        <span className="shareName">{nameOf(row.department.schoolId)}</span>
                      </span>
                    )}
                  </td>
                  <td>{row.department.code ?? "—"}</td>
                  <td className="rowActions">
                    {isThis ? (
                      <button
                        type="button"
                        className="linkish danger"
                        disabled={busy}
                        onClick={() => onRemove(row)}
                      >
                        Remove
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="linkish"
                        disabled={props.onSelectSchool === undefined}
                        title={`This is ${nameOf(row.department.schoolId)}'s own organisation, so only that school may change it.`}
                        onClick={() => props.onSelectSchool?.(row.department.schoolId)}
                      >
                        Go to {nameOf(row.department.schoolId)}
                      </button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      <div className="actions">
        <label className="srOnly" htmlFor="teacher-department-add">
          Department to add
        </label>
        <select
          id="teacher-department-add"
          value={picked}
          disabled={busy || addable.length === 0}
          onChange={(event) => setPicked(event.target.value)}
        >
          <option value="">Choose a department…</option>
          {addable.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>
              {candidate.code === null
                ? candidate.name
                : `${candidate.name} (${candidate.code})`}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="secondary"
          disabled={busy || picked === ""}
          onClick={() => {
            onAdd(picked);
            setPicked("");
          }}
        >
          Add to department
        </button>
      </div>
      {addable.length === 0 && (
        <p className="panelNote" role="note">
          {(props.departmentNames.size === 0
            ? `${school.name} has no departments yet. Add some in the Departments section first.`
            : `${named} is already in every department ${school.name} has.`)}
        </p>
      )}
      <p className="panelNote" role="note">
        Only <strong>{school.name}</strong>&rsquo;s departments are offered, and the school
        must already employ them &mdash; a department association is a narrowing of
        employment, never a claim on its own.
      </p>
    </section>
  );
}

/** The qualifications half. */
function Qualifications(props: {
  school: School;
  named: string;
  rows: readonly QualifiedSubject[] | null;
  error: string | null;
  addable: readonly Subject[];
  departmentNames: ReadonlyMap<string, string>;
  nameOf: (id: string) => string;
  busy: boolean;
  onAdd: (subjectId: string) => void;
  onRemove: (row: QualifiedSubject) => void;
  onSelectSchool?: (id: string) => void;
}) {
  const { school, named, rows, error, addable, departmentNames, nameOf, busy } = props;
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState("");

  /**
   * The subjects the search offers.
   *
   * A searchable list rather than a plain select, because a secondary school's subject list
   * runs to dozens and because the thing a user is looking for is a name they already know.
   */
  const matches = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle.length === 0) return addable;
    return addable.filter((candidate) => {
      const department = candidate.departmentId === null
        ? ""
        : (departmentNames.get(candidate.departmentId) ?? "");
      return (
        candidate.name.toLowerCase().includes(needle) ||
        (candidate.code ?? "").toLowerCase().includes(needle) ||
        department.toLowerCase().includes(needle)
      );
    });
  }, [addable, query, departmentNames]);

  /** Grouped by the school that owns the subject, so the two scopes read as two scopes. */
  const grouped = useMemo(() => {
    const groups = new Map<string, QualifiedSubject[]>();
    for (const row of rows ?? []) {
      const list = groups.get(row.subject.schoolId) ?? [];
      list.push(row);
      groups.set(row.subject.schoolId, list);
    }
    // This school first, whatever the read order was: it is the one that can be edited.
    return [...groups.entries()].sort(([a], [b]) =>
      a === school.id ? -1 : b === school.id ? 1 : a.localeCompare(b),
    );
  }, [rows, school.id]);

  return (
    <section className="subSection">
      <h4 className="subHead">Subject qualifications</h4>
      <p className="muted">
        What <strong>{named}</strong> can teach. A qualification names one school&rsquo;s own
        subject record, so somebody who teaches Mathematics at two schools holds{" "}
        <strong>two</strong> qualifications and remains <strong>one</strong> person.
      </p>

      {error !== null && <Banner tone="error">{error}</Banner>}

      {rows === null ? (
        <Loading what="qualifications" />
      ) : rows.length === 0 ? (
        <EmptyState title="No subjects recorded yet">
          <p>
            Nothing is implied by a name or by a department: a qualification is an explicit
            statement that this person can teach this particular subject.
          </p>
          <p className="muted">
            Choose a subject below to record one. Stage C will ask for a qualified teacher
            when a lesson is assigned, so this is the list it will read.
          </p>
        </EmptyState>
      ) : (
        grouped.map(([schoolId, group]) => {
          const isThis = schoolId === school.id;
          return (
            <div key={schoolId} className="schoolGroup">
              <h5 className="subHead">
                {isThis ? `${school.name} — this school` : nameOf(schoolId)}
                {!isThis && <span className="chipBadge">other</span>}
              </h5>
              <table className="dataTable">
                <caption className="srOnly">
                  Subjects of {isThis ? school.name : nameOf(schoolId)} this teacher is
                  qualified to teach
                </caption>
                <thead>
                  <tr>
                    <th scope="col">Subject</th>
                    <th scope="col">Code</th>
                    <th scope="col">Department</th>
                    <th scope="col" className="rowActions">
                      <span className="srOnly">Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {group.map((row) => (
                    <tr key={row.qualification.id}>
                      <td>{row.subject.name}</td>
                      <td>{row.subject.code ?? "—"}</td>
                      <td>
                        {row.departmentName ?? (
                          <span className="absent">No department</span>
                        )}
                      </td>
                      <td className="rowActions">
                        {isThis ? (
                          <button
                            type="button"
                            className="linkish danger"
                            disabled={busy}
                            onClick={() => props.onRemove(row)}
                          >
                            Remove
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="linkish"
                            disabled={props.onSelectSchool === undefined}
                            title={`${nameOf(schoolId)} owns this subject, so only that school may change this qualification.`}
                            onClick={() => props.onSelectSchool?.(schoolId)}
                          >
                            Go to {nameOf(schoolId)}
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          );
        })
      )}

      <div className="field">
        <label htmlFor="qualification-search">Find a subject</label>
        <input
          id="qualification-search"
          type="text"
          value={query}
          placeholder="Name, code or department"
          disabled={busy}
          onChange={(event) => setQuery(event.currentTarget.value)}
        />
        <p className="hint">
          Only <strong>{school.name}</strong>&rsquo;s subjects are offered. A subject of the
          same name at another school is a different record and has to be chosen there.
        </p>
      </div>

      <div className="actions">
        <label className="srOnly" htmlFor="qualification-add">
          Subject to qualify for
        </label>
        <select
          id="qualification-add"
          value={picked}
          disabled={busy || matches.length === 0}
          onChange={(event) => setPicked(event.target.value)}
        >
          <option value="">Choose a subject…</option>
          {matches.map((candidate) => {
            const department =
              candidate.departmentId === null
                ? null
                : (departmentNames.get(candidate.departmentId) ?? null);
            const parts = [
              candidate.code === null ? null : candidate.code,
              department,
              school.name,
            ].filter((part): part is string => part !== null);
            return (
              <option key={candidate.id} value={candidate.id}>
                {`${candidate.name} — ${parts.join(" · ")}`}
              </option>
            );
          })}
        </select>
        <button
          type="button"
          className="secondary"
          disabled={busy || picked === ""}
          onClick={() => {
            props.onAdd(picked);
            setPicked("");
            setQuery("");
          }}
        >
          Add qualification
        </button>
      </div>

      {matches.length === 0 && (
        <p className="panelNote" role="note">
          {addable.length === 0
            ? `${named} is already qualified for every subject ${school.name} has.`
            : `No subject of ${school.name} matches “${query.trim()}”.`}
        </p>
      )}

      <p className="panelNote" role="note">
        A qualification is a fact about the person, so it needs no school membership and{" "}
        <strong>outlives one</strong>: deactivating this teacher, removing a school from
        them, changing their campuses or editing their name all leave every row above
        exactly as it is. Removing one is always the explicit action here.
      </p>
    </section>
  );
}
