/**
 * One student's enrolment, history, and transfer.
 *
 * Teacher membership is a yes/no about now; an enrolment is **history**
 * ([ADR 0021](../../../../../docs/adr/0021-project-level-people-and-school-membership.md)
 * §5). A spell at a school is the thing a school is actually asked about — "when did he
 * join, leave, come back?" — so a transfer writes a new spell and keeps the old one, and
 * nothing on this screen edits a past spell into being a different spell.
 *
 * ## Transfer and concurrent enrolment are different things, and are not one control
 *
 * A **transfer** ends the spell here and opens one there. It is one fact about two
 * institutions, so `transfer_student` names both schools and does both halves in one
 * transaction — which is the whole reason it exists rather than a leave followed by an
 * enrol. Splitting it across two frontend calls would let a real student end up enrolled
 * nowhere, and this panel never does that.
 *
 * **Concurrent enrolment** keeps both: a student at a trust's senior school who also
 * attends its junior programme has two current spells, two admission numbers and two year
 * levels, and `student_enrolment_one_current` permits exactly that because two schools are
 * two keys. It is reached from the other school's Students screen, with "Enrol an existing
 * student" — never from here, so the two can never be confused with each other.
 *
 * ## Dates are not invented
 *
 * `enrolled_on` and `left_on` are nullable and the form leaves them blank. A school that
 * does not record the day somebody joined is the common case, not an incomplete one, and a
 * transfer with no date ends the old spell with none either — the `LEFT` status is the
 * fact, the date is optional.
 *
 * ## Group membership is not enrolment, and is never silently removed
 *
 * A student's groups are listed here, read-only, with that said plainly. Membership of
 * 9-MAT-A is the only record it ever happened, and a transfer leaves every one of them
 * alone — which means a group can legitimately list somebody who has left, and that is
 * history rather than an inconsistency to repair.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type {
  Campus,
  EnrolmentStatus,
  School,
  Student,
  StudentEnrolment,
  StudentEnrolmentInput,
  StudentGroup,
  YearLevel,
} from "@evara/domain";
import {
  describeSetupError,
  enrolStudent,
  leaveSchool,
  listCampuses,
  listEnrolmentHistory,
  listGroupsForStudent,
  listYearLevels,
  transferStudent,
  updateEnrolment,
} from "../../ipc/setup";
import { EntityForm, type Choice, type FieldSpec } from "../ui/Field";
import { Banner, ConfirmDialog, EmptyState, Facts, Loading, Panel } from "../ui/Shell";
import {
  isValid,
  text,
  textOrNull,
  validateEnrolment,
  type Draft,
  type Problems,
} from "../validation";

/** The three statuses, labelled for people rather than for the schema. */
const STATUSES: readonly { readonly value: EnrolmentStatus; readonly label: string }[] = [
  { value: "ENROLLED", label: "Enrolled — attending" },
  { value: "PROSPECTIVE", label: "Prospective — recorded and placed, not yet attending" },
];

function statusLabel(status: EnrolmentStatus): string {
  switch (status) {
    case "ENROLLED":
      return "Enrolled";
    case "PROSPECTIVE":
      return "Prospective";
    case "LEFT":
      return "Left";
  }
}

/** Which form, if any, is open. */
type Mode = "none" | "edit" | "enrol" | "transfer";

export function StudentEnrolments(props: {
  /** The school whose screen this is. Every mutation but a transfer names only it. */
  school: School;
  /** Every school in the project, so a transfer has somewhere to go. */
  schools: readonly School[];
  /** The student being looked at, or `null` when none is chosen. */
  student: Student | null;
  /** This school's year levels, for the placement selects. */
  yearLevels: readonly YearLevel[];
  /** This school's campuses. */
  campuses: readonly Campus[];
  /** Called after any change, so the students table above re-reads. */
  onChanged: () => void;
}) {
  const { school, schools, student, yearLevels, campuses, onChanged } = props;

  const [history, setHistory] = useState<readonly StudentEnrolment[] | null>(null);
  const [groups, setGroups] = useState<readonly StudentGroup[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [mode, setMode] = useState<Mode>("none");
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [leaving, setLeaving] = useState(false);
  const [leftOn, setLeftOn] = useState("");
  const [summary, setSummary] = useState<StudentEnrolmentInput | null>(null);

  const studentId = student?.id ?? null;

  const refresh = useCallback((id: string) => {
    let live = true;
    void Promise.all([listEnrolmentHistory(id), listGroupsForStudent(id)])
      .then(([spells, inGroups]) => {
        if (!live) return;
        setHistory(spells);
        setGroups(inGroups);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!live) return;
        setHistory([]);
        setGroups([]);
        setError(describeSetupError(cause));
      });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    setMode("none");
    setError(null);
    if (studentId === null) {
      setHistory(null);
      setGroups(null);
      return undefined;
    }
    setHistory(null);
    setGroups(null);
    return refresh(studentId);
  }, [studentId, refresh]);

  /** The student's current spell at **this** school, which is what this panel edits. */
  const current = useMemo(
    () =>
      (history ?? []).find(
        (spell) => spell.schoolId === school.id && spell.status !== "LEFT",
      ) ?? null,
    [history, school.id],
  );

  /** Every school this student currently attends, for the concurrent-enrolment note. */
  const alsoAt = useMemo(
    () =>
      (history ?? [])
        .filter((spell) => spell.status !== "LEFT" && spell.schoolId !== school.id)
        .map((spell) => spell.schoolId),
    [history, school.id],
  );

  const schoolName = useMemo(
    () => new Map(schools.map((candidate) => [candidate.id, candidate.name])),
    [schools],
  );

  const run = useCallback(
    (action: () => Promise<unknown>) => {
      if (studentId === null) return;
      setBusy(true);
      setError(null);
      void action()
        .then(() => {
          setMode("none");
          setLeaving(false);
          setSummary(null);
          refresh(studentId);
          onChanged();
        })
        .catch((cause: unknown) => {
          // The forms stay open with their values intact; only the confirmations close,
          // because they are dialogues and the message belongs on the page behind them.
          setLeaving(false);
          setSummary(null);
          setError(describeSetupError(cause));
        })
        .finally(() => setBusy(false));
    },
    [studentId, refresh, onChanged],
  );

  if (student === null) {
    return (
      <Panel title="Enrolment">
        <EmptyState title="No student chosen">
          Choose a student above with <strong>Enrolment</strong> to see this school&rsquo;s
          placement for them, their whole history, and to transfer them.
        </EmptyState>
      </Panel>
    );
  }

  const named = `${student.givenName} ${student.familyName}`;
  const yearName = (id: string | null): string =>
    id === null ? "Not set" : (yearLevels.find((year) => year.id === id)?.name ?? "Not set");
  const campusName = (id: string | null): string =>
    id === null ? "Not set" : (campuses.find((campus) => campus.id === id)?.name ?? "Not set");

  const openEdit = () => {
    if (!current) return;
    setDraft({
      admissionNo: current.admissionNo ?? "",
      yearLevelId: current.yearLevelId ?? null,
      campusId: current.campusId ?? null,
      status: current.status === "PROSPECTIVE" ? "PROSPECTIVE" : "ENROLLED",
      enrolledOn: current.enrolledOn ?? "",
      leftOn: "",
    });
    setProblems({});
    setError(null);
    setMode("edit");
  };

  const openEnrol = () => {
    setDraft({
      admissionNo: "",
      yearLevelId: null,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: "",
      leftOn: "",
    });
    setProblems({});
    setError(null);
    setMode("enrol");
  };

  const openTransfer = () => {
    setDraft({
      toSchoolId: "",
      admissionNo: "",
      yearLevelId: null,
      campusId: null,
      enrolledOn: "",
      leftOn: "",
    });
    setProblems({});
    setError(null);
    setMode("transfer");
  };

  /** Builds the spell this form describes, for this school. */
  const spellFrom = (schoolId: string): StudentEnrolmentInput => ({
    studentId: student.id,
    schoolId,
    admissionNo: textOrNull(draft, "admissionNo"),
    yearLevelId: textOrNull(draft, "yearLevelId"),
    campusId: textOrNull(draft, "campusId"),
    status: (text(draft, "status") || "ENROLLED") as EnrolmentStatus,
    enrolledOn: textOrNull(draft, "enrolledOn"),
    // Only a `LEFT` spell may carry a leaving date, and nothing here writes one: leaving
    // is its own operation, below.
    leftOn: null,
  });

  const submitPlacement = () => {
    const found = validateEnrolment(draft);
    setProblems(found);
    if (!isValid(found)) return;
    if (mode === "edit" && current) {
      run(() => updateEnrolment(school.id, current.id, spellFrom(school.id)));
    } else {
      run(() => enrolStudent(spellFrom(school.id)));
    }
  };

  const submitTransfer = () => {
    const found = validateEnrolment(draft);
    const destination = text(draft, "toSchoolId");
    const all = destination.length === 0
      ? { ...found, toSchoolId: "Choose the school they are transferring to." }
      : found;
    setProblems(all);
    if (!isValid(all)) return;
    setSummary({ ...spellFrom(destination), status: "ENROLLED" });
  };

  return (
    <Panel
      title={`Enrolment — ${named}`}
      note={
        <>
          One person, one identifier, however many schools they attend. The placement below
          is <strong>this school&rsquo;s</strong>: its own admission number, its own year
          level, its own campus. Another school&rsquo;s numbers for the same student are its
          own and neither is &ldquo;the&rdquo; one.
        </>
      }
    >
      {error && <Banner tone="error">{error}</Banner>}

      {history === null ? (
        <Loading what="this student's enrolment" />
      ) : (
        <>
          <h4 className="subHead">At {school.name}</h4>
          {current === null ? (
            <EmptyState
              title="Not currently enrolled here"
              action={{ label: "Enrol at this school", onClick: openEnrol, disabled: busy }}
            >
              Every spell {named} has had at this school is in the past. Their history is
              kept below; enrolling them again opens a new spell and leaves it untouched.
            </EmptyState>
          ) : (
            <>
              <Facts
                items={[
                  ["Status", statusLabel(current.status)],
                  ["Admission number", current.admissionNo ?? "Not set"],
                  ["Year level", yearName(current.yearLevelId)],
                  ["Campus", campusName(current.campusId)],
                  ["Joined", current.enrolledOn ?? "Not recorded"],
                  ["Revision", String(current.rev)],
                ]}
              />
              <div className="actions">
                <button type="button" disabled={busy} onClick={openEdit}>
                  Edit this enrolment
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => {
                    setLeftOn("");
                    setLeaving(true);
                  }}
                >
                  Mark as left
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy || schools.length < 2}
                  title={
                    schools.length < 2
                      ? "A transfer needs two schools. This campus file describes one."
                      : undefined
                  }
                  onClick={openTransfer}
                >
                  Transfer to another school…
                </button>
              </div>
            </>
          )}

          {alsoAt.length > 0 && (
            <Banner tone="info">
              {named} is <strong>also currently enrolled</strong> at{" "}
              {alsoAt.map((id) => schoolName.get(id) ?? "another school").join(", ")}. That
              is concurrent enrolment, not a transfer: each school holds its own placement,
              and ending one does not affect the other.
            </Banner>
          )}

          {(mode === "edit" || mode === "enrol") && (
            <EntityForm
              idPrefix="form-enrolment"
              legend={
                mode === "edit"
                  ? `Edit this school's enrolment of ${named}`
                  : `Enrol ${named} at ${school.name}`
              }
              fields={placementFields(yearLevels, campuses)}
              draft={draft}
              problems={problems}
              busy={busy}
              submitLabel={mode === "edit" ? "Save enrolment" : "Enrol"}
              onChange={(key, value) => {
                setDraft((d) => ({ ...d, [key]: value }));
                setProblems(({ [key]: _removed, ...rest }) => rest);
              }}
              onSubmit={submitPlacement}
              onCancel={() => setMode("none")}
            />
          )}

          {mode === "transfer" && (
            <TransferForm
              student={student}
              from={school}
              schools={schools}
              history={history}
              draft={draft}
              problems={problems}
              busy={busy}
              onChange={(key, value) => {
                setDraft((d) => ({ ...d, [key]: value }));
                setProblems(({ [key]: _removed, ...rest }) => rest);
              }}
              onSubmit={submitTransfer}
              onCancel={() => setMode("none")}
            />
          )}

          <h4 className="subHead">Enrolment history</h4>
          <p className="muted">
            Every spell {named} has ever had, at every school, oldest first. A transfer keeps
            the previous spell rather than rewriting it, which is what makes &ldquo;was she
            ever at Northgate?&rdquo; answerable years later. Past spells are history and are
            not edited here.
          </p>
          {history.length === 0 ? (
            <EmptyState title="No spells recorded">
              This student has no enrolment anywhere in this campus file, which should not be
              possible — a student is given a spell at the school that enters them.
            </EmptyState>
          ) : (
            <div className="tableScroll">
              <table className="dataTable">
                <caption className="srOnly">Enrolment history for {named}</caption>
                <thead>
                  <tr>
                    <th scope="col">School</th>
                    <th scope="col">Status</th>
                    <th scope="col">Admission no.</th>
                    <th scope="col">Joined</th>
                    <th scope="col">Left</th>
                  </tr>
                </thead>
                <tbody>
                  {history.map((spell) => (
                    <tr key={spell.id} className={spell.id === current?.id ? "editing" : undefined}>
                      <td>
                        {schoolName.get(spell.schoolId) ?? "A school no longer in this file"}
                        {spell.schoolId === student.schoolId && (
                          <span className="muted" title="The school that entered this student">
                            {" "}
                            (identity)
                          </span>
                        )}
                      </td>
                      <td>{statusLabel(spell.status)}</td>
                      <td>{spell.admissionNo ?? "—"}</td>
                      <td>{spell.enrolledOn ?? "—"}</td>
                      <td>{spell.leftOn ?? "—"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          <h4 className="subHead">Groups</h4>
          {groups === null || groups.length === 0 ? (
            <p className="muted">
              {named} is in no student group. A timetable does not need them to be —
              ADR 0006&rsquo;s group sizes are enough.
            </p>
          ) : (
            <>
              <p className="muted">
                {groups.map((group) => group.name).join(", ")}
              </p>
              <p className="muted">
                Group membership is <strong>not</strong> enrolment. These are kept exactly as
                they are by a transfer or a departure, because a membership is the only
                record that it ever happened — so a group may list somebody who has left,
                and that is history rather than something to repair. Memberships are edited
                under <strong>Student groups</strong>.
              </p>
            </>
          )}
        </>
      )}

      <ConfirmDialog
        open={leaving}
        title={`Record that ${named} has left ${school.name}?`}
        confirmLabel="Mark as left"
        tone="danger"
        busy={busy}
        onConfirm={() =>
          run(() => leaveSchool(school.id, student.id, leftOn.trim().length === 0 ? null : leftOn.trim()))
        }
        onCancel={() => setLeaving(false)}
      >
        <p>
          This school&rsquo;s spell becomes a past one. Nothing is deleted: the admission
          number, the year level and the campus stay on the row, because &ldquo;which year
          was she in when she left?&rdquo; is a question a school asks.
        </p>
        <div className="field">
          <label htmlFor="leaving-date">Leaving date</label>
          <input
            id="leaving-date"
            type="text"
            value={leftOn}
            placeholder="YYYY-MM-DD"
            onChange={(event) => setLeftOn(event.currentTarget.value)}
          />
          <p className="hint">
            Optional. Leave blank if the school does not record one — the fact that they have
            left is the status, not the date, and Evara does not invent one.
          </p>
        </div>
        <p className="muted">
          Their groups and every other school&rsquo;s enrolment are untouched. Enrolling them
          here again later opens a new spell and keeps this one.
        </p>
      </ConfirmDialog>

      <ConfirmDialog
        open={summary !== null}
        title="Transfer this student?"
        confirmLabel="Transfer student"
        busy={busy}
        onConfirm={() => {
          if (summary === null) return;
          run(() => transferStudent(school.id, summary.schoolId, student.id, summary));
        }}
        onCancel={() => setSummary(null)}
      >
        {summary !== null && (
          <>
            <p>
              <strong>{named}</strong> moves from <strong>{school.name}</strong> to{" "}
              <strong>{schoolName.get(summary.schoolId) ?? "the chosen school"}</strong>.
            </p>
            <ul className="planList">
              <li>Their spell at {school.name} becomes a past one and is kept.</li>
              <li>
                A new spell opens at{" "}
                {schoolName.get(summary.schoolId) ?? "the chosen school"}
                {summary.admissionNo === null
                  ? " with no admission number"
                  : ` as ${summary.admissionNo}`}
                {summary.enrolledOn === null ? "" : `, from ${summary.enrolledOn}`}.
              </li>
              <li>
                Their identifier, their groups and every other spell they have ever had are
                unchanged.
              </li>
            </ul>
            <p className="muted">
              Both halves happen in one transaction, so {named} is never enrolled twice and
              never enrolled nowhere. If they should attend <em>both</em> schools, this is the
              wrong action: cancel, and use &ldquo;Enrol an existing student&rdquo; on the
              other school&rsquo;s Students screen instead.
            </p>
          </>
        )}
      </ConfirmDialog>
    </Panel>
  );
}

/** The fields of a spell at one school. */
function placementFields(
  yearLevels: readonly YearLevel[],
  campuses: readonly Campus[],
): readonly FieldSpec[] {
  return [
    {
      kind: "text",
      key: "admissionNo",
      label: "Admission number",
      hint: "Optional, and this school's own. Unique among its current enrolments when given; a school that reissues a retired number has not done anything wrong.",
    },
    {
      kind: "select",
      key: "yearLevelId",
      label: "Year level",
      emptyLabel: "No year level",
      choices: toChoices(yearLevels),
      hint: "This school's year levels. Another school's are its own, even where both call it Grade 8.",
    },
    {
      kind: "select",
      key: "campusId",
      label: "Campus",
      emptyLabel: "No campus",
      choices: campuses.map((campus) => ({ value: campus.id, label: campus.name })),
    },
    {
      kind: "select",
      key: "status",
      label: "Status",
      required: true,
      choices: STATUSES.map((entry) => ({ value: entry.value, label: entry.label })),
      hint: "“Left” is not offered here: a departure is its own action, so a spell cannot be ended by editing a dropdown.",
    },
    {
      kind: "date",
      key: "enrolledOn",
      label: "Joining date",
      hint: "Optional. Leave blank if the school does not record one.",
    },
  ];
}

function toChoices(yearLevels: readonly YearLevel[]): Choice[] {
  return yearLevels.map((year) => ({ value: year.id, label: year.name }));
}

/**
 * The transfer form.
 *
 * The destination's year levels and campuses are read when the destination is chosen,
 * because they are *that school's* and this screen holds only its own. A placement naming a
 * third school's year level is refused by name, so offering one would be offering a
 * guaranteed failure.
 */
function TransferForm(props: {
  student: Student;
  from: School;
  schools: readonly School[];
  history: readonly StudentEnrolment[];
  draft: Draft;
  problems: Problems;
  busy: boolean;
  onChange: (key: string, value: string | boolean | string[]) => void;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  const { student, from, schools, history, draft, problems, busy } = props;
  const destination = text(draft, "toSchoolId");

  const [years, setYears] = useState<readonly YearLevel[]>([]);
  const [sites, setSites] = useState<readonly Campus[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (destination.length === 0) {
      setYears([]);
      setSites([]);
      return undefined;
    }
    let live = true;
    setLoadError(null);
    void Promise.all([listYearLevels(destination), listCampuses(destination)])
      .then(([yearLevels, campuses]) => {
        if (!live) return;
        setYears(yearLevels);
        setSites(campuses);
      })
      .catch((cause: unknown) => {
        if (!live) return;
        setYears([]);
        setSites([]);
        setLoadError(describeSetupError(cause));
      });
    return () => {
      live = false;
    };
  }, [destination]);

  /** Schools this student could be transferred to: not this one, and not one they attend. */
  const already = new Set(
    history.filter((spell) => spell.status !== "LEFT").map((spell) => spell.schoolId),
  );
  const choices: Choice[] = schools
    .filter((school) => school.id !== from.id)
    .map((school) => ({
      value: school.id,
      label: already.has(school.id)
        ? `${school.name} — already enrolled there`
        : school.name,
    }));

  const destinationField: FieldSpec = {
    kind: "select",
    key: "toSchoolId",
    label: "Transfer to",
    required: true,
    emptyLabel: "Choose a school…",
    choices,
    hint: "A transfer needs two different schools. A school they already attend is refused — that is concurrent enrolment, not a transfer.",
  };

  // The placement fields only once a destination is chosen, because they are *that*
  // school's year levels and campuses. `status` is dropped: a transfer arrives as an
  // enrolment and the backend refuses anything else, so offering the choice would be
  // offering a guaranteed failure.
  const fields: readonly FieldSpec[] = [
    destinationField,
    ...(destination.length === 0
      ? []
      : placementFields(years, sites).filter((field) => field.key !== "status")),
  ];

  return (
    <>
      {loadError && (
        <Banner tone="error">
          The receiving school&rsquo;s year levels and campuses could not be read:{" "}
          {loadError}
        </Banner>
      )}
      <EntityForm
        idPrefix="form-transfer"
        legend={`Transfer ${student.givenName} ${student.familyName} from ${from.name}`}
        fields={fields}
        draft={draft}
        problems={problems}
        busy={busy}
        submitLabel="Review transfer"
        onChange={props.onChange}
        onSubmit={props.onSubmit}
        onCancel={props.onCancel}
      />
    </>
  );
}
