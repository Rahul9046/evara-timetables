/**
 * Section 11 — Student groups and membership.
 *
 * ## One table for every kind of audience
 *
 * A class, a cohort, a homeroom, an elective set and a teaching set are one entity with a
 * `kind`, not five entities — `DATA-MODEL.md` §4 is explicit about it. The payoff shows up
 * the moment a lesson is taught to something: Phase 3 attaches an audience to an activity
 * once, rather than five times with four `if`s.
 *
 * ## Membership is a list, never a field on the student
 *
 * A student is in a homeroom **and** a maths set **and** an elective. The panel below is
 * therefore per-group, and the same student appears in as many groups as they belong to.
 * Anything shaped like "this student's group" would be wrong the first time a school
 * offers an option, which is to say immediately.
 *
 * ## Size and membership answer different questions
 *
 * A group's declared size is what lets a school timetable without entering students at all
 * ([ADR 0006](../../../../../docs/adr/0006-group-based-scheduling-first.md)). It is
 * therefore **not** derived from the member count and is not kept in step with it: a group
 * of 28 with four members listed is a school that has started entering students, not an
 * inconsistency to repair. The panel shows both numbers and lets them disagree, because
 * reconciling them would mean choosing which one to silently overwrite.
 *
 * ## Removing a member asks first
 *
 * Membership is the *only* record that a student was ever in a group: `enrolment` arrives
 * with Phase 3, and the change feed stores which row changed, never what it held. So a
 * mis-clicked "Remove" destroys history that nothing in the file can reconstruct, and the
 * removal is confirmed for the same reason a delete is. It reuses {@link ConfirmDialog} so
 * that what a destructive action asks is worded and behaved the same here as in
 * {@link CrudPanel}.
 *
 * ## The hierarchy is what clash detection will read
 *
 * `parentGroupId` is how a cohort and the sets it was split into are known to be the same
 * students — the sets clash with the cohort, not with each other. Nothing reads it yet;
 * it is collected now because inferring it later from names would be guesswork.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  Campus,
  GroupKind,
  School,
  Student,
  StudentGroup,
  StudentGroupInput,
  YearLevel,
} from "@evara/domain";
import {
  addGroupMember,
  createStudentGroup,
  deleteStudentGroup,
  describeSetupError,
  listCampuses,
  listGroupMembers,
  listStudentGroups,
  listStudents,
  listYearLevels,
  removeGroupMember,
  updateStudentGroup,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner, ConfirmDialog, EmptyState } from "../ui/Shell";
import {
  number,
  text,
  textOrNull,
  validateStudentGroup,
} from "../validation";
import { AvailabilityPanel } from "./AvailabilityGrid";
import { useList } from "./useList";

/** The five audience kinds, labelled for people rather than for the schema. */
const KINDS: readonly { readonly value: GroupKind; readonly label: string }[] = [
  { value: "CLASS", label: "Class" },
  { value: "COHORT", label: "Cohort — a whole year or band" },
  { value: "HOMEROOM", label: "Homeroom — pastoral registration" },
  { value: "ELECTIVE_SET", label: "Elective set — those who chose an option" },
  { value: "TEACHING_SET", label: "Teaching set — formed for one subject" },
];

function kindLabel(kind: GroupKind): string {
  // The short half of the label; the explanation belongs in the form, not the table.
  const found = KINDS.find((entry) => entry.value === kind)?.label ?? kind;
  return found.split(" — ")[0] ?? kind;
}

export function StudentGroups(props: { school: School }) {
  const { school } = props;
  const [selected, setSelected] = useState<StudentGroup | null>(null);

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

  const loadYearLevels = useCallback(() => listYearLevels(), []);
  const yearLevels = useList<YearLevel>(loadYearLevels);

  const loadGroups = useCallback(() => listStudentGroups(), []);
  const groups = useList<StudentGroup>(loadGroups);

  const campusChoices: readonly Choice[] = useMemo(
    () => (campuses.items ?? []).map((campus) => ({ value: campus.id, label: campus.name })),
    [campuses.items],
  );

  const yearChoices: readonly Choice[] = useMemo(
    () => (yearLevels.items ?? []).map((year) => ({ value: year.id, label: year.name })),
    [yearLevels.items],
  );

  /**
   * Every group, as parent choices.
   *
   * Not filtered to exclude the one being edited. Filtering would catch only the one-step
   * loop, which the schema's `CHECK` already refuses, while the real rule — no cycle
   * anywhere in the ancestry — is walked by the repository before any write. Half a rule
   * in the view layer is the second copy `CLAUDE.md` forbids, and the half it leaves out
   * is the half that matters.
   */
  const parentChoices: readonly Choice[] = useMemo(
    () => (groups.items ?? []).map((group) => ({ value: group.id, label: group.name })),
    [groups.items],
  );

  const spec: CrudSpec<StudentGroup, StudentGroupInput> = {
    noun: "student group",
    plural: "student groups",
    title: "Student groups",
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Kind", render: (row) => kindLabel(row.kind) },
      { header: "Code", render: (row) => row.code ?? "—" },
      {
        header: "Year",
        render: (row) =>
          (yearLevels.items ?? []).find((year) => year.id === row.yearLevelId)?.name ?? "—",
      },
      {
        header: "Declared size",
        render: (row) => (row.size === null ? "From members" : String(row.size)),
        numeric: true,
      },
    ],
    fields: [
      { kind: "text", key: "name", label: "Name", required: true, placeholder: "Grade 8A" },
      {
        kind: "select",
        key: "kind",
        label: "Kind",
        required: true,
        choices: KINDS.map((entry) => ({ value: entry.value, label: entry.label })),
      },
      { kind: "text", key: "code", label: "Code", hint: "Optional, unique within the school." },
      {
        kind: "select",
        key: "yearLevelId",
        label: "Year level",
        emptyLabel: "No year level",
        choices: yearChoices,
      },
      {
        kind: "select",
        key: "parentGroupId",
        label: "Split out of",
        emptyLabel: "Not a subdivision",
        hint: "The larger group these students also belong to — a cohort, when this is one of its teaching sets.",
        choices: parentChoices,
      },
      {
        kind: "number",
        key: "size",
        label: "Declared size",
        min: 0,
        hint: "Leave blank to use the member list. A number lets you timetable this group without entering students.",
      },
      {
        kind: "select",
        key: "campusId",
        label: "Campus",
        emptyLabel: "No campus",
        choices: campusChoices,
      },
    ],
    describe: (row) => row.name,
    validate: validateStudentGroup,
    toDraft: (row) => ({
      name: row?.name ?? "",
      kind: row?.kind ?? "CLASS",
      code: row?.code ?? "",
      yearLevelId: row?.yearLevelId ?? null,
      parentGroupId: row?.parentGroupId ?? null,
      size: row?.size === null || row?.size === undefined ? "" : String(row.size),
      campusId: row?.campusId ?? null,
    }),
    toInput: (draft) => ({
      schoolId: school.id,
      name: text(draft, "name"),
      kind: text(draft, "kind") as GroupKind,
      code: textOrNull(draft, "code"),
      yearLevelId: textOrNull(draft, "yearLevelId"),
      parentGroupId: textOrNull(draft, "parentGroupId"),
      size: text(draft, "size").length === 0 ? null : number(draft, "size"),
      campusId: textOrNull(draft, "campusId"),
    }),
    list: () => listStudentGroups(),
    create: (input) => createStudentGroup(input),
    update: (id, input) => updateStudentGroup(id, input),
    remove: (id) => deleteStudentGroup(id),
    emptyHint:
      "No groups yet. A group is whoever a lesson is taught to — a class, a cohort, a homeroom or an elective set.",
    deleteNote: () =>
      "The students themselves are kept; only their membership of this group goes.",
  };

  return (
    <section className="sectionBody">
      <p className="muted">
        A group is whoever a lesson is taught to. One student can be in as many as they
        belong to — a homeroom, a teaching set and an elective are three groups, not a
        choice between them.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      <CrudPanel spec={spec} onChanged={groups.reload} />
      <AvailabilityPanel
        school={school}
        entityKind="STUDENT_GROUP"
        noun="group"
        options={(groups.items ?? []).map((group) => ({
          id: group.id,
          name: group.name,
        }))}
      />
      <GroupMembers
        groups={groups.items ?? []}
        selected={selected}
        onSelect={setSelected}
      />
    </section>
  );
}

/**
 * The membership editor for one group.
 *
 * Deliberately a separate panel rather than a field on the group form. Membership is the
 * one part of this screen that is a *relationship* and not an attribute, it is edited far
 * more often than the group itself, and a multi-select of two thousand students inside a
 * form would be unusable at exactly the schools that need it most.
 */
function GroupMembers(props: {
  groups: readonly StudentGroup[];
  selected: StudentGroup | null;
  onSelect: (group: StudentGroup | null) => void;
}) {
  const { groups, selected, onSelect } = props;

  const loadStudents = useCallback(() => listStudents(), []);
  const students = useList<Student>(loadStudents);

  const [members, setMembers] = useState<readonly Student[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState<string>("");
  const [pending, setPending] = useState<Student | null>(null);

  const refresh = useCallback((groupId: string) => {
    void listGroupMembers(groupId)
      .then((rows) => {
        setMembers(rows);
        setError(null);
      })
      .catch((cause: unknown) => {
        setMembers([]);
        setError(describeSetupError(cause));
      });
  }, []);

  const choose = useCallback(
    (groupId: string) => {
      const group = groups.find((entry) => entry.id === groupId) ?? null;
      onSelect(group);
      setMembers(null);
      setError(null);
      if (group) refresh(group.id);
    },
    [groups, onSelect, refresh],
  );

  const add = useCallback(() => {
    if (!selected || adding.length === 0) return;
    setBusy(true);
    void addGroupMember({ studentGroupId: selected.id, studentId: adding })
      .then(() => {
        setAdding("");
        refresh(selected.id);
      })
      .catch((cause: unknown) => setError(describeSetupError(cause)))
      .finally(() => setBusy(false));
  }, [adding, refresh, selected]);

  const remove = useCallback(() => {
    if (!selected || pending === null) return;
    const studentId = pending.id;
    setBusy(true);
    void removeGroupMember(selected.id, studentId)
      .then(() => {
        setPending(null);
        refresh(selected.id);
      })
      // The banner is on the page rather than in the dialogue, which is closing either
      // way — the same choice CrudPanel makes for a delete that was refused.
      .catch((cause: unknown) => {
        setPending(null);
        setError(describeSetupError(cause));
      })
      .finally(() => setBusy(false));
  }, [pending, refresh, selected]);

  // Only students not already in the group, so adding one is never a no-op that fails.
  const addable = useMemo(() => {
    const present = new Set((members ?? []).map((student) => student.id));
    return (students.items ?? []).filter((student) => !present.has(student.id));
  }, [members, students.items]);

  if (groups.length === 0) return null;

  return (
    <div className="panel">
      <div className="panelHead">
        <h3>Members</h3>
      </div>

      <label className="field">
        <span className="fieldLabel">Group</span>
        <select
          value={selected?.id ?? ""}
          onChange={(event) => choose(event.target.value)}
          aria-label="Group to edit members of"
        >
          <option value="">Choose a group…</option>
          {groups.map((group) => (
            <option key={group.id} value={group.id}>
              {group.name}
            </option>
          ))}
        </select>
      </label>

      {error && <Banner tone="error">{error}</Banner>}

      {selected === null ? (
        <EmptyState title="No group chosen">Choose a group to see and edit who is in it.</EmptyState>
      ) : (
        <>
          <p className="muted">
            {selected.size === null
              ? "This group has no declared size, so its member list is its size."
              : `This group declares a size of ${selected.size}. The member list below is kept separately and the two are allowed to differ.`}
          </p>

          {(students.items ?? []).length === 0 ? (
            <EmptyState title="No students entered">
              Add students under “Students and year levels” first — or leave this group's
              declared size set and timetable without them.
            </EmptyState>
          ) : (
            <div className="actions">
              <label className="field">
                <span className="fieldLabel">Add a student</span>
                <select
                  value={adding}
                  onChange={(event) => setAdding(event.target.value)}
                  disabled={busy || addable.length === 0}
                  aria-label="Student to add"
                >
                  <option value="">
                    {addable.length === 0 ? "Everyone is already in this group" : "Choose a student…"}
                  </option>
                  {addable.map((student) => (
                    <option key={student.id} value={student.id}>
                      {student.familyName}, {student.givenName}
                      {student.isActive ? "" : " (left)"}
                    </option>
                  ))}
                </select>
              </label>
              <button type="button" onClick={add} disabled={busy || adding.length === 0}>
                Add to group
              </button>
            </div>
          )}

          {members === null ? null : members.length === 0 ? (
            <EmptyState title="No members yet">Nobody has been added to this group.</EmptyState>
          ) : (
            <table className="dataTable">
              <caption className="srOnly">Students in {selected.name}</caption>
              <thead>
                <tr>
                  <th scope="col">Last name</th>
                  <th scope="col">First name</th>
                  <th scope="col">Status</th>
                  <th scope="col">
                    <span className="srOnly">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {members.map((student) => (
                  <tr key={student.id}>
                    <td>{student.familyName}</td>
                    <td>{student.givenName}</td>
                    <td>{student.isActive ? "Enrolled" : "Left"}</td>
                    <td>
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => setPending(student)}
                      >
                        Remove
                        <span className="srOnly">
                          {" "}
                          {student.givenName} {student.familyName} from {selected.name}
                        </span>
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}

      <ConfirmDialog
        open={pending !== null && selected !== null}
        title="Remove from group"
        confirmLabel="Remove from group"
        tone="danger"
        busy={busy}
        onConfirm={remove}
        onCancel={() => setPending(null)}
      >
        {pending !== null && selected !== null && (
          <>
            <p>
              {pending.givenName} {pending.familyName} stops being a member of{" "}
              {selected.name}. The student record itself, and every other group they belong
              to, are untouched.
            </p>
            <p>
              This membership is the only record that they were ever in this group — Evara
              keeps no enrolment history yet — so removing it cannot be undone. To record
              that a student has left the school, clear “Currently enrolled” on them
              instead and their groups are kept.
            </p>
          </>
        )}
      </ConfirmDialog>
    </div>
  );
}
