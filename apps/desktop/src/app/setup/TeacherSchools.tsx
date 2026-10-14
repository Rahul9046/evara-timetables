/**
 * One teacher's schools and campuses.
 *
 * The panel [ADR 0021](../../../../../docs/adr/0021-project-level-people-and-school-membership.md)
 * was written for. A teacher is one person for the life of the project; which schools
 * employ them and which of those schools' campuses they may be based at are relationships,
 * and this is where they are edited.
 *
 * ## Three operations, deliberately not one control
 *
 * ADR 0021 §8 keeps them apart and so does this screen, because conflating them is how a
 * school ends up deleting a person it only meant to stop employing:
 *
 * | Action here | What it does |
 * | --- | --- |
 * | **Deactivate** | they stay, keep everything, stop being offered for new work here |
 * | **Remove from this school** | this school's employment goes; the person and every other school's employment stay |
 * | Delete the teacher (the Teachers table above) | the person — only from their home school, only while nobody else employs them |
 *
 * The home school's membership cannot be removed at all: it is what guarantees a teacher
 * can never reach zero schools and vanish from every screen. The backend refuses it with a
 * message naming the remedy, and this panel says so before the button is pressed rather
 * than after.
 *
 * ## Only the selected school's relationship is editable here
 *
 * Holding a shared teacher's identifier is ordinary now, and is **not** authority to touch
 * another school's employment of them — `require_same_school` refuses it. So another
 * school's row is shown, because seeing it is the point, and its controls are not offered:
 * the way to change Southmoor's employment of somebody is to select Southmoor. The panel
 * offers that as an action rather than leaving the user to find the sidebar.
 *
 * ## Membership first, campus second
 *
 * `assign_teacher_campus` refuses a campus of a school the teacher does not belong to, and
 * the message names the remedy. The ordering is therefore not reimplemented here — the
 * campus list simply only offers campuses of schools the teacher is already a member of,
 * and "add the membership first" is stated where a user would otherwise look for the
 * missing campus. When a user asks for both at once, membership is written first.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type { School, Teacher, TeacherSchool } from "@evara/domain";
import {
  addTeacherToSchool,
  assignTeacherCampus,
  describeSetupError,
  listTeacherCampuses,
  listTeacherMemberships,
  removeTeacherCampus,
  removeTeacherFromSchool,
  setTeacherMembershipActive,
} from "../../ipc/setup";
import { Banner, ConfirmDialog, EmptyState, Loading, Panel } from "../ui/Shell";
import { useCampusIndex, useProjectCampuses, type CampusOfSchool } from "./project";

/** What the panel has to ask before it does it. */
type Pending =
  | { readonly kind: "remove"; readonly membership: TeacherSchool }
  | { readonly kind: "deactivate"; readonly membership: TeacherSchool }
  | { readonly kind: "unassign"; readonly campus: CampusOfSchool };

export function TeacherSchools(props: {
  /** The school whose screen this is. Every mutation names it. */
  school: School;
  /** Every school in the project, so another school's row can be named. */
  schools: readonly School[];
  /** The teacher being looked at, or `null` when none is chosen. */
  teacher: Teacher | null;
  /** Switches the selected school, for an edit this one may not make. */
  onSelectSchool: (id: string) => void;
  /** Called after any change, so the teachers table above re-reads. */
  onChanged: () => void;
}) {
  const { school, schools, teacher, onSelectSchool, onChanged } = props;

  const [memberships, setMemberships] = useState<readonly TeacherSchool[] | null>(null);
  const [eligible, setEligible] = useState<readonly string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Pending | null>(null);

  const campuses = useProjectCampuses(schools);
  const campusOf = useCampusIndex(campuses.all);

  const teacherId = teacher?.id ?? null;

  /**
   * Re-reads both relationships together.
   *
   * One function rather than two, because every write here changes both in principle:
   * removing a membership clears that school's eligibility in the same transaction, and a
   * screen that re-read only what it thought it had changed would show eligibility for a
   * school the teacher no longer belongs to.
   */
  const refresh = useCallback((id: string) => {
    let live = true;
    void Promise.all([listTeacherMemberships(id), listTeacherCampuses(id)])
      .then(([rows, campusIds]) => {
        if (!live) return;
        setMemberships(rows);
        setEligible(campusIds);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (!live) return;
        setMemberships([]);
        setEligible([]);
        setError(describeSetupError(cause));
      });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    if (teacherId === null) {
      setMemberships(null);
      setEligible(null);
      return undefined;
    }
    setMemberships(null);
    setEligible(null);
    return refresh(teacherId);
  }, [teacherId, refresh]);

  /** Runs one relationship change, surfacing a refusal rather than losing it. */
  const run = useCallback(
    (action: () => Promise<unknown>) => {
      if (teacherId === null) return;
      setBusy(true);
      setError(null);
      void action()
        .then(() => {
          setPending(null);
          refresh(teacherId);
          onChanged();
        })
        .catch((cause: unknown) => {
          // The dialogue is closing either way, so the message belongs on the page — the
          // same choice `CrudPanel` makes for a delete that was refused.
          setPending(null);
          setError(describeSetupError(cause));
        })
        .finally(() => setBusy(false));
    },
    [teacherId, refresh, onChanged],
  );

  const memberSchoolIds = useMemo(
    () => new Set((memberships ?? []).map((row) => row.schoolId)),
    [memberships],
  );

  /** The schools this teacher does not yet belong to. */
  const joinable = useMemo(
    () => schools.filter((candidate) => !memberSchoolIds.has(candidate.id)),
    [schools, memberSchoolIds],
  );

  if (teacher === null) {
    return (
      <Panel title="Schools and campuses">
        <EmptyState title="No teacher chosen">
          Choose a teacher above with <strong>Schools</strong> to see which schools employ
          them and which campuses they may be based at.
        </EmptyState>
      </Panel>
    );
  }

  const homeSchool = schools.find((candidate) => candidate.id === teacher.schoolId);
  const isVisiting = teacher.schoolId !== school.id;

  return (
    <Panel
      title={`Schools and campuses — ${teacher.givenName} ${teacher.familyName}`}
      note={
        <>
          One person, one identifier, however many schools employ them. Their availability,
          their load ceilings and their timetable are a single fact; the rows below are the
          relationships, not copies of the person.
        </>
      }
    >
      {error && <Banner tone="error">{error}</Banner>}
      {campuses.error && (
        <Banner tone="warning">
          Some campuses could not be read, so the list below may be incomplete:{" "}
          {campuses.error}
        </Banner>
      )}

      {isVisiting && (
        <Banner tone="info">
          <strong>{teacher.givenName} {teacher.familyName}</strong>&rsquo;s identity — their
          name, staff code, email and load ceilings — is held by{" "}
          <strong>{homeSchool?.name ?? "the school that created them"}</strong> and can only
          be edited there. This school can change its own employment of them, below.
          {homeSchool && (
            <>
              {" "}
              <button
                type="button"
                className="linkish"
                onClick={() => onSelectSchool(homeSchool.id)}
              >
                Switch to {homeSchool.name}
              </button>
            </>
          )}
        </Banner>
      )}

      <h4 className="subHead">Schools</h4>
      {memberships === null ? (
        <Loading what="this teacher's schools" />
      ) : (
        <div className="tableScroll">
          <table className="dataTable">
            <caption className="srOnly">
              Schools employing {teacher.givenName} {teacher.familyName}
            </caption>
            <thead>
              <tr>
                <th scope="col">School</th>
                <th scope="col">Employment</th>
                <th scope="col">Role</th>
                <th scope="col" className="rowActions">
                  <span className="srOnly">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {memberships.map((row) => {
                const named = schools.find((candidate) => candidate.id === row.schoolId);
                const isHome = row.schoolId === teacher.schoolId;
                const isThis = row.schoolId === school.id;
                return (
                  <tr key={row.id} className={isThis ? "editing" : undefined}>
                    <td>{named?.name ?? "A school no longer in this file"}</td>
                    <td>
                      {row.isActive ? (
                        <span className="yes">Teaching here</span>
                      ) : (
                        <span className="no">Not teaching here</span>
                      )}
                    </td>
                    <td>
                      {isHome ? (
                        <span title="The school that created this teacher and holds their identity">
                          Home school
                        </span>
                      ) : (
                        "Shared"
                      )}
                    </td>
                    <td className="rowActions">
                      {isThis ? (
                        <>
                          <button
                            type="button"
                            className="linkish"
                            disabled={busy}
                            onClick={() =>
                              row.isActive
                                ? setPending({ kind: "deactivate", membership: row })
                                : run(() =>
                                    setTeacherMembershipActive(school.id, teacher.id, true),
                                  )
                            }
                          >
                            {row.isActive ? "Mark as left" : "Mark as teaching"}
                          </button>
                          <button
                            type="button"
                            className="linkish danger"
                            disabled={busy || isHome}
                            title={
                              isHome
                                ? "This teacher was created in this school, so this school cannot be removed from them. Use “Mark as left” to record that they have left, or delete the teacher."
                                : undefined
                            }
                            onClick={() => setPending({ kind: "remove", membership: row })}
                          >
                            Remove from this school
                          </button>
                        </>
                      ) : (
                        <button
                          type="button"
                          className="linkish"
                          disabled={busy || named === undefined}
                          title="A school's employment of a teacher is only editable from that school."
                          onClick={() => named && onSelectSchool(named.id)}
                        >
                          Manage at {named?.name ?? "that school"}
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {joinable.length > 0 && (
        <div className="actions">
          <label className="field">
            <span className="fieldLabel">Add to another school</span>
            <AddToSchool
              schools={joinable}
              busy={busy}
              onAdd={(schoolId) =>
                run(() =>
                  addTeacherToSchool({
                    teacherId: teacher.id,
                    schoolId,
                    // The ordinary intent of adding somebody is that they teach there.
                    // The per-school flag is separate from the project-level one on
                    // purpose, and "Mark as left" is how it is cleared afterwards.
                    isActive: true,
                  }),
                )
              }
            />
          </label>
        </div>
      )}

      <h4 className="subHead">Campus eligibility</h4>
      <p className="muted">
        A <strong>narrowing</strong> of membership, for the part-time arrangement membership
        alone cannot express — &ldquo;Southmoor, junior campus only&rdquo;. A teacher with no
        campus chosen is not restricted; they are simply not tied to a site. Only campuses
        of schools they belong to can be offered, because the database refuses the rest.
      </p>

      {eligible === null || campuses.all === null ? (
        <Loading what="campus eligibility" />
      ) : (
        <CampusEligibility
          schools={schools}
          campuses={campuses.all}
          memberSchoolIds={memberSchoolIds}
          actingSchoolId={school.id}
          eligible={new Set(eligible)}
          busy={busy}
          onAssign={(campusId) =>
            run(() => assignTeacherCampus(school.id, teacher.id, campusId))
          }
          onRemove={(campusId) => {
            const entry = campusOf.get(campusId);
            if (entry) setPending({ kind: "unassign", campus: entry });
          }}
          onSelectSchool={onSelectSchool}
        />
      )}

      <ConfirmDialog
        open={pending !== null}
        title={titleOf(pending)}
        confirmLabel={confirmLabelOf(pending)}
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          if (pending.kind === "remove") {
            run(() => removeTeacherFromSchool(school.id, teacher.id));
          } else if (pending.kind === "deactivate") {
            run(() => setTeacherMembershipActive(school.id, teacher.id, false));
          } else {
            run(() => removeTeacherCampus(school.id, teacher.id, pending.campus.campus.id));
          }
        }}
        onCancel={() => setPending(null)}
      >
        {pending?.kind === "remove" && (
          <>
            <p>
              <strong>{school.name}</strong> stops employing{" "}
              {teacher.givenName} {teacher.familyName}. The person is untouched: every other
              school keeps them, their identifier never changes, and their availability
              stays.
            </p>
            <p className="muted">
              Their eligibility for this school&rsquo;s campuses goes with the membership —
              eligibility for a school they no longer belong to is exactly the drift the
              database forbids. If they have simply left,{" "}
              <strong>Mark as left</strong> keeps the record instead.
            </p>
          </>
        )}
        {pending?.kind === "deactivate" && (
          <>
            <p>
              {teacher.givenName} {teacher.familyName} stops being offered for new work at{" "}
              <strong>{school.name}</strong>.
            </p>
            <p className="muted">
              Nothing is deleted. They keep the person, their availability, the campuses
              they worked at, and every assignment they already have — the way an inactive
              teacher always has. Every other school&rsquo;s employment of them is
              unaffected.
            </p>
          </>
        )}
        {pending?.kind === "unassign" && (
          <p>
            {teacher.givenName} {teacher.familyName} stops being eligible at{" "}
            <strong>{pending.campus.campus.name}</strong>. Their employment by{" "}
            {pending.campus.school.name} is untouched — they still belong to the school,
            simply not to that site.
          </p>
        )}
      </ConfirmDialog>
    </Panel>
  );
}

function titleOf(pending: Pending | null): string {
  switch (pending?.kind) {
    case "remove":
      return "Remove this teacher from this school?";
    case "deactivate":
      return "Record that this teacher has left this school?";
    case "unassign":
      return "Withdraw this campus?";
    default:
      return "";
  }
}

function confirmLabelOf(pending: Pending | null): string {
  switch (pending?.kind) {
    case "remove":
      return "Remove from this school";
    case "deactivate":
      return "Mark as left";
    case "unassign":
      return "Withdraw campus";
    default:
      return "Confirm";
  }
}

/** A select plus a button, because adding a membership is one deliberate act. */
function AddToSchool(props: {
  schools: readonly School[];
  busy: boolean;
  onAdd: (schoolId: string) => void;
}) {
  const [chosen, setChosen] = useState("");

  return (
    <span className="rangeRow">
      <select
        value={chosen}
        disabled={props.busy}
        aria-label="School to add this teacher to"
        onChange={(event) => setChosen(event.currentTarget.value)}
      >
        <option value="">Choose a school…</option>
        {props.schools.map((school) => (
          <option key={school.id} value={school.id}>
            {school.code ? `${school.name} (${school.code})` : school.name}
          </option>
        ))}
      </select>
      <button
        type="button"
        disabled={props.busy || chosen.length === 0}
        onClick={() => {
          props.onAdd(chosen);
          setChosen("");
        }}
      >
        Add to school
      </button>
    </span>
  );
}

/**
 * Campuses grouped by school, with each school's membership state beside it.
 *
 * Grouped rather than flat because the grouping *is* the explanation: a campus the teacher
 * cannot be given is unavailable because of the school it belongs to, and showing that
 * school's membership state next to it answers "why not" without a tooltip. A school this
 * teacher does not belong to is listed with its campuses disabled and the remedy named,
 * which is the one ordering mistake the rule actually produces.
 */
function CampusEligibility(props: {
  schools: readonly School[];
  campuses: readonly CampusOfSchool[];
  memberSchoolIds: ReadonlySet<string>;
  actingSchoolId: string;
  eligible: ReadonlySet<string>;
  busy: boolean;
  onAssign: (campusId: string) => void;
  onRemove: (campusId: string) => void;
  onSelectSchool: (id: string) => void;
}) {
  const {
    schools,
    campuses,
    memberSchoolIds,
    actingSchoolId,
    eligible,
    busy,
    onAssign,
    onRemove,
    onSelectSchool,
  } = props;

  const bySchool = schools.map((school) => ({
    school,
    campuses: campuses.filter((entry) => entry.school.id === school.id),
  }));

  if (bySchool.every((group) => group.campuses.length === 0)) {
    return (
      <EmptyState title="No campuses yet">
        Add a campus under <strong>Campuses and buildings</strong> first. A teacher does not
        need one — leaving this empty means they are not tied to a site.
      </EmptyState>
    );
  }

  return (
    <ul className="eligibilityGroups">
      {bySchool
        .filter((group) => group.campuses.length > 0)
        .map((group) => {
          const isMember = memberSchoolIds.has(group.school.id);
          const isActing = group.school.id === actingSchoolId;
          return (
            <li key={group.school.id}>
              <fieldset className="eligibilityGroup" disabled={busy || !isMember || !isActing}>
                <legend>
                  {group.school.name}
                  {isMember ? (
                    <span className="chipBadge"> member</span>
                  ) : (
                    <span className="chipBadge"> not a member</span>
                  )}
                </legend>

                {!isMember && (
                  <p className="hint">
                    Add this teacher to {group.school.name} before giving them one of its
                    campuses — the database refuses eligibility without the membership, and
                    &ldquo;Add to another school&rdquo; above is where it is granted.
                  </p>
                )}
                {isMember && !isActing && (
                  <p className="hint">
                    Only {group.school.name} can decide who works at its campuses.{" "}
                    <button
                      type="button"
                      className="linkish"
                      onClick={() => onSelectSchool(group.school.id)}
                    >
                      Switch to {group.school.name}
                    </button>{" "}
                    to change these.
                  </p>
                )}

                <div className="weekdayRow">
                  {group.campuses.map((entry) => {
                    const on = eligible.has(entry.campus.id);
                    return (
                      <label
                        key={entry.campus.id}
                        className="weekdayToggle weekdayToggle--wide"
                        htmlFor={`eligible-${entry.campus.id}`}
                      >
                        <input
                          id={`eligible-${entry.campus.id}`}
                          type="checkbox"
                          checked={on}
                          disabled={busy || !isMember || !isActing}
                          onChange={(event) =>
                            event.currentTarget.checked
                              ? onAssign(entry.campus.id)
                              : onRemove(entry.campus.id)
                          }
                        />
                        <span>{entry.campus.name}</span>
                      </label>
                    );
                  })}
                </div>
              </fieldset>
            </li>
          );
        })}
    </ul>
  );
}
