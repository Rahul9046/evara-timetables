/**
 * One class section in full: who teaches it, who attends it, who is named on its roster,
 * and how often it has to meet.
 *
 * Four panels rather than one form, for the reason `StudentGroups` separates membership
 * from the group: every one of these is a *relationship* and not an attribute, each is
 * edited far more often than the section itself, and a multi-select of two thousand
 * students inside a form would be unusable at exactly the schools that need it most.
 *
 * ## The teacher panel is where ADR 0028 is visible
 *
 * Three answers about giving a section to a teacher, and the whole point is that they are
 * not the same kind of answer:
 *
 * | Answer | Effect | Why |
 * | --- | --- | --- |
 * | not actively employed here | **refused** | an assignment is new work, and ADR 0018 says an inactive person keeps what they have and is offered no more |
 * | not eligible at the section's campus | **refused** | `teacher_campus` is a narrowing of membership, and a section at the senior campus is not work for somebody confined to the junior one |
 * | no qualification for the subject | **warned** | a school covering an absent specialist on Tuesday must be able to say so |
 *
 * Every one of those is computed by the backend and read here.
 * [ADR 0028](../../../../../docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md)
 * argues the third, which is the one that changed: ADR 0025 §4 originally asked Stage C for
 * a conjunction of all three.
 *
 * The refused candidates are still shown, disabled, with the reason beside them. Somebody
 * expected and silently missing from a picker is worse than somebody shown and explained —
 * ADR 0023 §5's rule.
 *
 * ## Groups are the audience; the roster is a register
 *
 * The two panels are deliberately unequal in prominence and in wording. ADR 0006 requires
 * a school to be able to timetable from group sizes alone, so **Student groups** is the
 * scheduling population and **Named students** is optional everywhere. Nothing adds the two
 * figures: a section taught to a group of 28 that also names four children has an audience
 * of 28, because those four are among the 28.
 *
 * That is also why each roster row says whether an attending group already covers it. A
 * `false` there is not an error — it means somebody is named whom no attending group
 * includes, which is worth seeing rather than silently resolving.
 *
 * ## Occurrences: two numbers, and their product
 *
 * The panel asks *periods per lesson* and *times per cycle* and writes one row per
 * occurrence. It then states the product, because those are the two figures a user is most
 * likely to conflate — "duration 2, three times a cycle" is six period slots, not two and
 * not three. [ADR 0029](../../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md).
 *
 * The findings below it report **impossibility only**. An empty list does not mean the
 * section can be timetabled.
 */
import { useCallback, useMemo, useState, type ReactNode } from "react";

import type {
  Activity,
  AssignedTeacher,
  AttendingGroup,
  DemandFinding,
  RosteredStudent,
  School,
  SectionDemand,
  StudentGroup,
  StudentInSchool,
  TeacherCandidate,
} from "@evara/domain";
import {
  addActivities,
  addClassEnrolment,
  addClassGroup,
  addClassTeacher,
  deleteActivity,
  describeSetupError,
  generateActivities,
  isConflict,
  listActivities,
  listClassEnrolments,
  listClassGroups,
  listClassTeachers,
  listSchoolStudents,
  listStudentGroups,
  listTeacherCandidates,
  removeClassEnrolment,
  removeClassGroup,
  removeClassTeacher,
  sectionDemand,
  updateActivity,
} from "../../ipc/setup";
import { EntityForm } from "../ui/Field";
import { Banner, ConfirmDialog, EmptyState, Loading } from "../ui/Shell";
import { isValid, number, text, textOrNull, validateOccurrences, type Draft, type Problems } from "../validation";
import type { SectionRow } from "./ClassSections";
import type { CourseRow } from "./Lessons";
import { useList, useValue } from "./useList";

export function ClassSectionDetail(props: {
  school: School;
  course: CourseRow;
  /** The chosen section, or `null` when none is. */
  section: SectionRow | null;
  openSection?: (id: string) => void;
  /** Called after any change, so the sections table can re-read its counts. */
  onChanged?: () => void;
}) {
  const { school, course, section, openSection, onChanged } = props;

  if (section === null) {
    return (
      <div className="crudPanel">
        <div className="panelHead">
          <h3>Class section detail</h3>
        </div>
        <EmptyState title="No class section chosen">
          Choose <strong>Open</strong> on a class section above to assign teachers, choose
          who attends, and say how often it meets.
        </EmptyState>
      </div>
    );
  }

  return (
    <>
      <p className="setupScope" role="note">
        <span className="eyebrow">CLASS SECTION</span>
        <strong>
          {course.name} — {section.name}
        </strong>
      </p>

      <SectionTeachers
        school={school}
        course={course}
        section={section}
        openSection={openSection}
        onChanged={onChanged}
      />
      <SectionGroups
        school={school}
        section={section}
        openSection={openSection}
        onChanged={onChanged}
      />
      <SectionRoster
        school={school}
        section={section}
        openSection={openSection}
        onChanged={onChanged}
      />
      <SectionOccurrences
        school={school}
        course={course}
        section={section}
        openSection={openSection}
        onChanged={onChanged}
      />
    </>
  );
}

/**
 * Runs one mutation, keeping a conflict distinguishable from everything else.
 *
 * The relationship panels do not go through `CrudPanel`, so they have to hold its promise
 * themselves: a stale revision is **reported with something to do about it** and is never
 * retried, because re-sending with the current revision would complete exactly the
 * overwrite the check prevented. ADR 0027 §6.
 */
function useMutation(refresh: () => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stale, setStale] = useState<string | null>(null);

  const run = useCallback(
    (work: () => Promise<unknown>, after?: () => void) => {
      setBusy(true);
      setError(null);
      void work()
        .then(() => {
          setStale(null);
          after?.();
          refresh();
        })
        .catch((cause: unknown) => {
          if (isConflict(cause)) setStale(describeSetupError(cause));
          else setError(describeSetupError(cause));
        })
        .finally(() => setBusy(false));
    },
    [refresh],
  );

  const banners = (
    <>
      {error !== null && <Banner tone="error">{error}</Banner>}
      {stale !== null && (
        <Banner tone="warning">
          <span>{stale}</span>{" "}
          <button
            type="button"
            className="linkish"
            onClick={() => {
              setStale(null);
              refresh();
            }}
          >
            Reload
          </button>
        </Banner>
      )}
    </>
  );

  return { busy, run, banners };
}

// ========================================================================== teachers

function SectionTeachers(props: {
  school: School;
  course: CourseRow;
  section: SectionRow;
  openSection?: (id: string) => void;
  onChanged?: () => void;
}) {
  const { school, course, section, openSection, onChanged } = props;
  const sectionId = section.id;

  const loadAssigned = useCallback(
    () => listClassTeachers(school.id, sectionId),
    [school.id, sectionId],
  );
  const assigned = useList<AssignedTeacher>(loadAssigned);

  const loadCandidates = useCallback(
    () => listTeacherCandidates(school.id, sectionId),
    [school.id, sectionId],
  );
  const candidates = useList<TeacherCandidate>(loadCandidates);

  const refresh = useCallback(() => {
    assigned.reload();
    candidates.reload();
    onChanged?.();
  }, [assigned, candidates, onChanged]);

  const { busy, run, banners } = useMutation(refresh);
  const [picked, setPicked] = useState("");
  const [pending, setPending] = useState<AssignedTeacher | null>(null);

  /**
   * The candidates that may actually be added, with the ones that may not kept separate.
   *
   * Both halves are rendered: the first in the picker, the second as an explained list
   * below it. Dropping the second would leave a user looking for a colleague who is in the
   * school and simply not offered, with nothing saying why.
   */
  const offerable = useMemo(
    () =>
      (candidates.items ?? []).filter(
        (entry) => !entry.assigned && entry.eligibility.employedAndActive && entry.eligibility.campusEligible,
      ),
    [candidates.items],
  );
  const refused = useMemo(
    () =>
      (candidates.items ?? []).filter(
        (entry) =>
          !entry.assigned &&
          !(entry.eligibility.employedAndActive && entry.eligibility.campusEligible),
      ),
    [candidates.items],
  );

  const chosen = offerable.find((entry) => entry.teacher.id === picked) ?? null;

  return (
    <div className="crudPanel">
      <div className="panelHead">
        <h3>Teachers</h3>
      </div>

      <p className="muted">
        A section may have several teachers — co-teaching is ordinary, and nothing here
        assumes one. Assigning somebody needs an <strong>active</strong> membership of this
        school
        {section.campusId === null
          ? ""
          : ", and eligibility at the campus this section is taught at"}
        . A missing <em>subject qualification</em> is a warning, not a refusal.
      </p>

      {banners}
      {assigned.error !== null && <Banner tone="error">{assigned.error}</Banner>}

      {assigned.items === null ? (
        <Loading what="assigned teachers" />
      ) : assigned.items.length === 0 ? (
        <EmptyState title="Nobody teaches this section yet">
          <p>
            A class section with no teacher is a complete record — a school often knows the
            class before it knows who will take it — but a timetable cannot be built from
            one.
          </p>
        </EmptyState>
      ) : (
        <table className="dataTable">
          <caption className="srOnly">Teachers assigned to this class section</caption>
          <thead>
            <tr>
              <th scope="col">Last name</th>
              <th scope="col">First name</th>
              <th scope="col">Staff code</th>
              <th scope="col">At this school</th>
              <th scope="col">{course.subject.name}</th>
              <th scope="col">
                <span className="srOnly">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {assigned.items.map((entry) => (
              <tr key={entry.assignment.id}>
                <td>{entry.teacher.familyName}</td>
                <td>{entry.teacher.givenName}</td>
                <td>{entry.teacher.code ?? "—"}</td>
                <td>
                  {entry.membership.isActive && entry.teacher.isActive ? (
                    "Currently teaching"
                  ) : (
                    <span title="Their existing assignments are kept — only new ones are refused. See ADR 0018.">
                      No longer teaching
                    </span>
                  )}
                </td>
                <td>
                  {entry.qualified ? (
                    "Qualified"
                  ) : (
                    <span
                      className="warnish"
                      title="No qualification for this subject is recorded. The assignment stands; record one on the Teachers screen if it should be there."
                    >
                      Not recorded
                    </span>
                  )}
                </td>
                <td className="rowActions">
                  <button
                    type="button"
                    className="linkish danger"
                    disabled={busy}
                    onClick={() => setPending(entry)}
                  >
                    Remove
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {(candidates.items ?? []).length === 0 ? (
        <EmptyState title="This school has no teachers yet">
          <p className="muted">
            Add staff under <strong>Teachers</strong> first. A teacher is a project-wide
            identity, so somebody another school in this campus file already employs can be
            given a membership here rather than entered twice.
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
        <div className="actions">
          <label className="field">
            <span className="fieldLabel">Assign a teacher</span>
            <select
              value={picked}
              onChange={(event) => setPicked(event.target.value)}
              disabled={busy || offerable.length === 0}
              aria-label="Teacher to assign"
            >
              <option value="">
                {offerable.length === 0
                  ? "Nobody eligible is left to assign"
                  : "Choose a teacher…"}
              </option>
              {offerable.map((entry) => (
                <option key={entry.teacher.id} value={entry.teacher.id}>
                  {entry.teacher.familyName}, {entry.teacher.givenName}
                  {entry.eligibility.qualified ? "" : " — no qualification recorded"}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy || chosen === null}
            onClick={() => {
              if (chosen === null) return;
              run(() => addClassTeacher(school.id, sectionId, chosen.teacher.id), () =>
                setPicked(""),
              );
            }}
          >
            Assign
          </button>
        </div>
      )}

      {/* The warning, before the save rather than after it. Shown as soon as the picker
          settles on somebody unqualified, so the user decides knowingly instead of being
          told what they have already done. */}
      {chosen !== null && !chosen.eligibility.qualified && (
        <Banner tone="warning">
          {chosen.teacher.givenName} {chosen.teacher.familyName} does not have a recorded
          qualification for {course.subject.name}. You can still assign them — cover,
          unusual staffing and incomplete records are all ordinary. Record the qualification
          under <strong>Teachers</strong> if it should be there.
        </Banner>
      )}

      {refused.length > 0 && (
        <p className="panelNote" role="note">
          Not offered:{" "}
          {refused.map((entry, index) => (
            <span key={entry.teacher.id}>
              {index > 0 && "; "}
              <strong>
                {entry.teacher.familyName}, {entry.teacher.givenName}
              </strong>{" "}
              — {refusalReason(entry)}
            </span>
          ))}
        </p>
      )}

      <ConfirmDialog
        open={pending !== null}
        title="Remove this teacher from the section?"
        confirmLabel="Remove"
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          run(
            () =>
              removeClassTeacher(
                school.id,
                sectionId,
                pending.teacher.id,
                pending.assignment.rev,
              ),
            () => setPending(null),
          );
        }}
        onCancel={() => setPending(null)}
      >
        {pending !== null && (
          <>
            <p>
              <strong>
                {pending.teacher.givenName} {pending.teacher.familyName}
              </strong>{" "}
              will no longer teach this section.
            </p>
            <p className="muted">
              Their membership of this school, their campus eligibility, their departments
              and every qualification they hold are untouched. This removes one staffing
              decision.
            </p>
          </>
        )}
      </ConfirmDialog>
    </div>
  );
}

/** Why a candidate is not offered, in the words the user needs to act on. */
function refusalReason(entry: TeacherCandidate): string {
  if (!entry.eligibility.employedAndActive) {
    return entry.teacher.isActive
      ? "recorded as no longer teaching at this school"
      : "archived in this campus file";
  }
  if (!entry.eligibility.campusEligible) {
    return "not eligible at this section's campus";
  }
  return "unavailable";
}

// ==================================================================== student groups

function SectionGroups(props: {
  school: School;
  section: SectionRow;
  openSection?: (id: string) => void;
  onChanged?: () => void;
}) {
  const { school, section, openSection, onChanged } = props;
  const sectionId = section.id;

  const loadAttending = useCallback(
    () => listClassGroups(school.id, sectionId),
    [school.id, sectionId],
  );
  const attending = useList<AttendingGroup>(loadAttending);

  const loadGroups = useCallback(() => listStudentGroups(school.id), [school.id]);
  const groups = useList<StudentGroup>(loadGroups);

  const refresh = useCallback(() => {
    attending.reload();
    onChanged?.();
  }, [attending, onChanged]);

  const { busy, run, banners } = useMutation(refresh);
  const [picked, setPicked] = useState("");
  const [pending, setPending] = useState<AttendingGroup | null>(null);

  const addable = useMemo(() => {
    const present = new Set((attending.items ?? []).map((entry) => entry.group.id));
    return (groups.items ?? []).filter((group) => !present.has(group.id));
  }, [attending.items, groups.items]);

  const total = (attending.items ?? []).reduce((sum, entry) => sum + entry.population, 0);

  return (
    <div className="crudPanel">
      <div className="panelHead">
        <h3>Student groups</h3>
      </div>

      <p className="muted">
        Who this section is taught to, and <strong>the audience a timetable is built
        around</strong>. A section can be taught to several groups — a combined cohort is
        ordinary — and a section with groups and no named students is complete rather than
        half-finished.
      </p>

      {banners}
      {attending.error !== null && <Banner tone="error">{attending.error}</Banner>}
      {groups.error !== null && (
        <Banner tone="warning">
          This school&rsquo;s student groups could not be loaded: {groups.error}
        </Banner>
      )}

      {attending.items === null ? (
        <Loading what="attending groups" />
      ) : attending.items.length === 0 ? (
        <EmptyState title="Nobody attends this section yet">
          <p>
            Add a student group to say who this class is taught to. A group&rsquo;s declared
            size is enough — individual students are optional throughout Evara.
          </p>
        </EmptyState>
      ) : (
        <>
          <table className="dataTable">
            <caption className="srOnly">Student groups attending this class section</caption>
            <thead>
              <tr>
                <th scope="col">Group</th>
                <th scope="col">Code</th>
                <th scope="col">Kind</th>
                <th scope="col" className="numeric">
                  Declared size
                </th>
                <th scope="col" className="numeric">
                  Members listed
                </th>
                <th scope="col" className="numeric">
                  Counts as
                </th>
                <th scope="col">
                  <span className="srOnly">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {attending.items.map((entry) => (
                <tr key={entry.association.id}>
                  <td>{entry.group.name}</td>
                  <td>{entry.group.code ?? "—"}</td>
                  <td>{entry.group.kind}</td>
                  <td className="numeric">
                    {entry.group.size === null ? (
                      <span className="absent">Not declared</span>
                    ) : (
                      String(entry.group.size)
                    )}
                  </td>
                  <td className="numeric">{entry.namedMembers}</td>
                  <td className="numeric">{entry.population}</td>
                  <td className="rowActions">
                    <button
                      type="button"
                      className="linkish danger"
                      disabled={busy}
                      onClick={() => setPending(entry)}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="panelNote" role="note">
            This section&rsquo;s audience is <strong>{total}</strong>. A group with no
            declared size contributes its listed membership instead — &ldquo;not
            declared&rdquo; means the member list is the truth.
          </p>
        </>
      )}

      {(groups.items ?? []).length === 0 ? (
        <EmptyState title="This school has no student groups yet">
          <p className="muted">
            Add them under <strong>Student groups</strong>. A declared size is all a group
            needs for a timetable to be built around it.
            {openSection !== undefined && (
              <>
                {" "}
                <button
                  type="button"
                  className="linkish"
                  onClick={() => openSection("groups")}
                >
                  Open Student groups
                </button>
              </>
            )}
          </p>
        </EmptyState>
      ) : (
        <div className="actions">
          <label className="field">
            <span className="fieldLabel">Add a student group</span>
            <select
              value={picked}
              onChange={(event) => setPicked(event.target.value)}
              disabled={busy || addable.length === 0}
              aria-label="Student group to add"
            >
              <option value="">
                {addable.length === 0
                  ? "Every group of this school already attends"
                  : "Choose a group…"}
              </option>
              {addable.map((group) => (
                <option key={group.id} value={group.id}>
                  {group.name}
                  {group.size === null ? "" : ` (${group.size})`}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy || picked.length === 0}
            onClick={() =>
              run(() => addClassGroup(school.id, sectionId, picked), () => setPicked(""))
            }
          >
            Add
          </button>
        </div>
      )}

      <ConfirmDialog
        open={pending !== null}
        title="Remove this group from the section?"
        confirmLabel="Remove"
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          run(
            () =>
              removeClassGroup(
                school.id,
                sectionId,
                pending.group.id,
                pending.association.rev,
              ),
            () => setPending(null),
          );
        }}
        onCancel={() => setPending(null)}
      >
        {pending !== null && (
          <>
            <p>
              <strong>{pending.group.name}</strong> will no longer attend this section.
            </p>
            <p className="muted">
              The group itself, its members, its declared size and its availability are
              untouched.
            </p>
          </>
        )}
      </ConfirmDialog>
    </div>
  );
}

// ===================================================================== named students

function SectionRoster(props: {
  school: School;
  section: SectionRow;
  openSection?: (id: string) => void;
  onChanged?: () => void;
}) {
  const { school, section, openSection, onChanged } = props;
  const sectionId = section.id;

  const loadRoster = useCallback(
    () => listClassEnrolments(school.id, sectionId),
    [school.id, sectionId],
  );
  const roster = useList<RosteredStudent>(loadRoster);

  /**
   * The students this school has, read **through their enrolment at this school**.
   *
   * `listSchoolStudents` rather than `listStudents`, deliberately: a student's identity is
   * project-wide and exists the moment any school enrolled them, which is not eligibility
   * to sit in this school's class. The backend refuses it anyway; using the enrolment-scoped
   * read means the picker never offers it in the first place.
   */
  const loadEligible = useCallback(() => listSchoolStudents(school.id), [school.id]);
  const eligible = useList<StudentInSchool>(loadEligible);

  const refresh = useCallback(() => {
    roster.reload();
    onChanged?.();
  }, [roster, onChanged]);

  const { busy, run, banners } = useMutation(refresh);
  const [picked, setPicked] = useState("");
  const [pending, setPending] = useState<RosteredStudent | null>(null);

  const addable = useMemo(() => {
    const present = new Set((roster.items ?? []).map((entry) => entry.student.id));
    return (eligible.items ?? []).filter(
      (entry) => !present.has(entry.student.id) && entry.enrolment.status !== "LEFT",
    );
  }, [eligible.items, roster.items]);

  const uncovered = (roster.items ?? []).filter((entry) => !entry.coveredByGroup).length;

  return (
    <div className="crudPanel">
      <div className="panelHead">
        <h3>Named students</h3>
      </div>

      <p className="muted">
        <strong>Optional.</strong> A register of which individuals attend, for a school that
        keeps one. It is <em>not</em> a second audience: the figure a timetable is built
        around is the attending groups&rsquo;, and naming four children of a group of 28
        does not make the class 32.
      </p>

      {banners}
      {roster.error !== null && <Banner tone="error">{roster.error}</Banner>}

      {roster.items === null ? (
        <Loading what="named students" />
      ) : roster.items.length === 0 ? (
        <EmptyState title="No individuals are named">
          <p>
            Nothing needs to be. A timetable can be built entirely from the student
            groups&rsquo; declared sizes, and most schools will never fill this in.
          </p>
        </EmptyState>
      ) : (
        <>
          <table className="dataTable">
            <caption className="srOnly">Students named on this class section's roster</caption>
            <thead>
              <tr>
                <th scope="col">Last name</th>
                <th scope="col">First name</th>
                <th scope="col">Admission number</th>
                <th scope="col">In an attending group</th>
                <th scope="col">
                  <span className="srOnly">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {roster.items.map((entry) => (
                <tr key={entry.enrolment.id}>
                  <td>{entry.student.familyName}</td>
                  <td>{entry.student.givenName}</td>
                  <td>{entry.student.code ?? "—"}</td>
                  <td>
                    {entry.coveredByGroup ? (
                      "Yes"
                    ) : (
                      <span
                        className="warnish"
                        title="No group attending this section has them as a member, so the audience figure does not include them."
                      >
                        No
                      </span>
                    )}
                  </td>
                  <td className="rowActions">
                    <button
                      type="button"
                      className="linkish danger"
                      disabled={busy}
                      onClick={() => setPending(entry)}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {uncovered > 0 && (
            <p className="panelNote" role="note">
              {uncovered === 1
                ? "1 named student is not in any group attending this section"
                : `${uncovered} named students are not in any group attending this section`}
              , so the audience figure above does not include them. That may be exactly
              right — or it may mean a group is missing from the panel above.
            </p>
          )}
        </>
      )}

      {(eligible.items ?? []).length === 0 ? (
        <EmptyState title="No students are enrolled at this school">
          <p className="muted">
            That is not a problem: individual students are optional throughout Evara, and a
            full timetable can be built without entering one.
            {openSection !== undefined && (
              <>
                {" "}
                <button
                  type="button"
                  className="linkish"
                  onClick={() => openSection("students")}
                >
                  Open Students
                </button>
              </>
            )}
          </p>
        </EmptyState>
      ) : (
        <div className="actions">
          <label className="field">
            <span className="fieldLabel">Add a student</span>
            <select
              value={picked}
              onChange={(event) => setPicked(event.target.value)}
              disabled={busy || addable.length === 0}
              aria-label="Student to add to the roster"
            >
              <option value="">
                {addable.length === 0
                  ? "Everyone enrolled here is already named"
                  : "Choose a student…"}
              </option>
              {addable.map((entry) => (
                <option key={entry.student.id} value={entry.student.id}>
                  {entry.student.familyName}, {entry.student.givenName}
                  {entry.enrolment.status === "PROSPECTIVE" ? " (applicant)" : ""}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy || picked.length === 0}
            onClick={() =>
              run(() => addClassEnrolment(school.id, sectionId, picked), () =>
                setPicked(""),
              )
            }
          >
            Add
          </button>
        </div>
      )}

      <ConfirmDialog
        open={pending !== null}
        title="Remove this student from the roster?"
        confirmLabel="Remove"
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          run(
            () =>
              removeClassEnrolment(
                school.id,
                sectionId,
                pending.student.id,
                pending.enrolment.rev,
              ),
            () => setPending(null),
          );
        }}
        onCancel={() => setPending(null)}
      >
        {pending !== null && (
          <>
            <p>
              <strong>
                {pending.student.givenName} {pending.student.familyName}
              </strong>{" "}
              will no longer be named on this section&rsquo;s roster.
            </p>
            <p className="muted">
              Their identity, their enrolment at this school and their group memberships are
              untouched.
            </p>
          </>
        )}
      </ConfirmDialog>
    </div>
  );
}

// ======================================================================= occurrences

function SectionOccurrences(props: {
  school: School;
  course: CourseRow;
  section: SectionRow;
  openSection?: (id: string) => void;
  onChanged?: () => void;
}) {
  const { school, course, section, openSection, onChanged } = props;
  const sectionId = section.id;

  const loadActivities = useCallback(
    () => listActivities(school.id, sectionId),
    [school.id, sectionId],
  );
  const activities = useList<Activity>(loadActivities);

  const loadDemand = useCallback(
    () => sectionDemand(school.id, sectionId),
    [school.id, sectionId],
  );
  const demand = useValue<SectionDemand>(loadDemand);
  const summary = demand.value ?? null;

  const refresh = useCallback(() => {
    activities.reload();
    demand.reload();
    onChanged?.();
  }, [activities, demand, onChanged]);

  const { busy, run, banners } = useMutation(refresh);

  /** The add form, or the row being edited, or nothing. */
  const [editing, setEditing] = useState<{ row: Activity | null } | null>(null);
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [pending, setPending] = useState<Activity | null>(null);

  const open = useCallback(
    (row: Activity | null) => {
      setEditing({ row });
      setProblems({});
      setDraft({
        name: row?.name ?? "",
        durationPeriods: String(
          row?.durationPeriods ?? course.defaultDurationPeriods,
        ),
        // Only an add takes a count; editing changes the one occurrence it was opened
        // on, and the field is not rendered.
        count: "1",
      });
    },
    [course.defaultDurationPeriods],
  );

  const submit = useCallback(() => {
    if (editing === null) return;
    const found = editing.row === null
      ? validateOccurrences(draft)
      : validateOccurrences({ ...draft, count: "1" });
    setProblems(found);
    if (!isValid(found)) return;

    const name = textOrNull(draft, "name");
    const duration = number(draft, "durationPeriods");
    if (editing.row === null) {
      run(
        () => addActivities(school.id, sectionId, name, duration, number(draft, "count")),
        () => setEditing(null),
      );
    } else {
      const row = editing.row;
      run(
        () => updateActivity(school.id, row.id, row.rev, name, duration),
        () => setEditing(null),
      );
    }
  }, [draft, editing, run, school.id, sectionId]);

  const declared = course.periodsPerCycle;
  const canGenerate =
    declared !== null && (activities.items ?? []).length === 0 && declared > 0;

  return (
    <div className="crudPanel">
      <div className="panelHead">
        <h3>Lesson occurrences</h3>
        {editing === null && (
          <button type="button" disabled={busy} onClick={() => open(null)}>
            Add lessons
          </button>
        )}
      </div>

      <p className="muted">
        How often this class has to meet, and for how long at a time. One row is{" "}
        <strong>one lesson</strong> that has to find a place in the timetable: a class
        meeting four times a cycle for one period has four rows, and a class meeting once
        for a double period has one row of two periods.
      </p>

      {banners}
      {activities.error !== null && <Banner tone="error">{activities.error}</Banner>}
      {/* Separate from the occurrences' own failure: the rows can load while the demand
          summary does not, and the panel is still usable without the summary. */}
      {demand.error !== null && (
        <Banner tone="warning">
          This section&rsquo;s demand could not be summarised, so the figures below the table
          are missing: {demand.error}
        </Banner>
      )}

      {editing !== null && (
        <EntityForm
          idPrefix="form-occurrence"
          legend={
            editing.row === null
              ? "Add lessons to this section"
              : `Edit lesson ${positionOf(activities.items, editing.row)}`
          }
          fields={[
            {
              kind: "number",
              key: "durationPeriods",
              label: "Periods per lesson",
              required: true,
              min: 1,
              hint: "Consecutive periods one lesson needs. 1 for a single, 2 for a double. A double is one lesson, not two — it cannot be split.",
            },
            ...(editing.row === null
              ? ([
                  {
                    kind: "number" as const,
                    key: "count",
                    label: "Times per cycle",
                    required: true,
                    min: 1,
                    hint: "How many lessons of that length the cycle needs. Periods per lesson × times per cycle is the total this class demands.",
                  },
                ] as const)
              : []),
            {
              kind: "text",
              key: "name",
              label: "Label",
              hint: "Optional. “Laboratory”, “Double” — only where an occurrence needs its own name. Most do not.",
            },
          ]}
          draft={draft}
          problems={problems}
          busy={busy}
          submitLabel={editing.row === null ? "Add lessons" : "Save changes"}
          onChange={(key, value) => {
            setDraft((current) => ({ ...current, [key]: value }));
            setProblems((current) => {
              if (current[key] === undefined) return current;
              const next = { ...current };
              delete next[key];
              return next;
            });
          }}
          onSubmit={submit}
          onCancel={() => {
            setEditing(null);
            setProblems({});
          }}
        />
      )}

      {editing?.row === null && (
        <p className="panelNote" role="note">
          That is{" "}
          <strong>
            {describeProduct(
              Number(text(draft, "durationPeriods")),
              Number(text(draft, "count")),
            )}
          </strong>{" "}
          added to this section.
        </p>
      )}

      {activities.items === null ? (
        <Loading what="lesson occurrences" />
      ) : activities.items.length === 0 ? (
        <EmptyState
          title="No lessons yet"
          action={
            editing === null ? { label: "Add lessons", onClick: () => open(null) } : undefined
          }
        >
          <p>
            Add an activity to define how often this class needs to meet. Two numbers:{" "}
            <em>periods per lesson</em> and <em>times per cycle</em>.
          </p>
          {canGenerate && (
            <p className="muted">
              {course.name} says it needs {declared} period
              {declared === 1 ? "" : "s"} a cycle at{" "}
              {course.defaultDurationPeriods} per lesson.{" "}
              <button
                type="button"
                className="linkish"
                disabled={busy}
                onClick={() => run(() => generateActivities(school.id, sectionId))}
              >
                Create those lessons
              </button>
            </p>
          )}
          {declared === null && (
            <p className="muted">
              {course.name} does not say how many periods a cycle it needs, so
              nothing can be generated from it. Set <em>Periods per cycle</em> on the
              course, or add the lessons here by hand — nothing will guess a number.
            </p>
          )}
        </EmptyState>
      ) : (
        <table className="dataTable">
          <caption className="srOnly">Lesson occurrences of this class section</caption>
          <thead>
            <tr>
              <th scope="col">Lesson</th>
              <th scope="col">Label</th>
              <th scope="col" className="numeric">
                Periods
              </th>
              <th scope="col">
                <span className="srOnly">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {activities.items.map((row, index) => (
              <tr key={row.id}>
                <td>{index + 1}</td>
                <td>{row.name ?? <span className="absent">Unnamed</span>}</td>
                <td className="numeric">{row.durationPeriods}</td>
                <td className="rowActions">
                  <button
                    type="button"
                    className="linkish"
                    disabled={busy}
                    onClick={() => open(row)}
                  >
                    Edit
                  </button>{" "}
                  <button
                    type="button"
                    className="linkish danger"
                    disabled={busy}
                    onClick={() => setPending(row)}
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {summary !== null && summary.demand.occurrences > 0 && (
        <DemandSummary
          summary={summary}
          courseName={course.name}
          openSection={openSection}
        />
      )}

      <ConfirmDialog
        open={pending !== null}
        title="Delete this lesson?"
        confirmLabel="Delete lesson"
        tone="danger"
        busy={busy}
        onConfirm={() => {
          if (pending === null) return;
          run(
            () => deleteActivity(school.id, pending.id, pending.rev),
            () => setPending(null),
          );
        }}
        onCancel={() => setPending(null)}
      >
        {pending !== null && (
          <p>
            One {pending.durationPeriods === 1 ? "single" : `${pending.durationPeriods}-period`}{" "}
            lesson will be removed from this section, so it will demand{" "}
            {pending.durationPeriods} fewer period
            {pending.durationPeriods === 1 ? "" : "s"} a cycle.
          </p>
        )}
      </ConfirmDialog>
    </div>
  );
}

/**
 * What this section asks of a cycle, and anything that makes it impossible.
 *
 * Reports impossibility **only**. "Twenty-nine periods in a thirty-period cycle" is
 * probably infeasible and is deliberately not reported, because only a solver can tell
 * difficulty from impossibility and there is not one yet — see this module's header.
 */
function DemandSummary(props: {
  summary: SectionDemand;
  courseName: string;
  openSection?: (id: string) => void;
}) {
  const { summary, courseName, openSection } = props;
  const { demand, declared, capacity, findings } = summary;

  return (
    <div className="panelSummary">
      <p>
        This section demands{" "}
        <strong>
          {demand.periodsPerCycle} period{demand.periodsPerCycle === 1 ? "" : "s"} a cycle
        </strong>
        , across {demand.occurrences} lesson{demand.occurrences === 1 ? "" : "s"}
        {demand.longestOccurrence > 1 && (
          <>
            , the longest being {demand.longestOccurrence} consecutive periods
          </>
        )}
        .
      </p>

      {declared.kind === "short" && (
        <p className="panelNote" role="note">
          {courseName} says it needs {declared.declared} periods a cycle, so this section is{" "}
          {declared.shortfall} short. That may be deliberate — a section can legitimately
          need less than its course&rsquo;s default — and nothing here refuses it.
        </p>
      )}
      {declared.kind === "over" && (
        <p className="panelNote" role="note">
          {courseName} says it needs {declared.declared} periods a cycle, so this section
          asks for {declared.excess} more. That may be deliberate, and nothing here refuses
          it.
        </p>
      )}

      {!capacity.cycleDays || !capacity.teachingPeriodsPerDay ? (
        <p className="panelNote" role="note">
          This school has no cycle or no bell schedule yet, so nothing can be said about
          whether the demand fits. That is the expected order — lessons come before the time
          model in this workflow.
          {openSection !== undefined && (
            <>
              {" "}
              <button
                type="button"
                className="linkish"
                onClick={() => openSection("cycles")}
              >
                Open Cycles
              </button>
            </>
          )}
        </p>
      ) : (
        findings.length === 0 && (
          <p className="panelNote" role="note">
            Nothing about this demand is impossible in a {capacity.cycleDays}-day cycle of
            up to {capacity.teachingPeriodsPerDay} teaching periods.{" "}
            <strong>That is not a promise that it can be timetabled</strong> — clashes,
            rooms and staff availability are only decided when a timetable is generated.
          </p>
        )
      )}

      {findings.map((finding, index) => (
        <Banner key={index} tone="error">
          {describeFinding(finding)}
        </Banner>
      ))}
    </div>
  );
}

/** What one impossibility means, and what to do about it. */
function describeFinding(finding: DemandFinding): ReactNode {
  switch (finding.kind) {
    case "occurrenceLongerThanAnyRun":
      return (
        <>
          One lesson needs {finding.needed} consecutive periods, and no day in this
          school&rsquo;s bell schedules has more than {finding.longestRun} teaching periods
          in a row. <strong>No timetable can place it.</strong> Shorten the lesson, or add
          periods to a bell schedule.
        </>
      );
    case "totalExceedsCycle":
      return (
        <>
          This section demands {finding.demanded} period slots a cycle and the cycle offers
          at most {finding.available}. <strong>No timetable can place them all.</strong>{" "}
          Reduce the number of lessons, or lengthen the cycle.
        </>
      );
    default:
      return "This section's demand cannot be met by the current time model.";
  }
}

/** Where one occurrence sits in the list the user is looking at, counting from 1. */
function positionOf(rows: readonly Activity[] | null, row: Activity): number {
  const index = (rows ?? []).findIndex((candidate) => candidate.id === row.id);
  return index < 0 ? 1 : index + 1;
}

/**
 * Duration times count, spelled out.
 *
 * The one arithmetic this screen does, and it is here rather than in the backend because it
 * describes what the *form in front of the user* would add rather than what the project
 * holds. Everything about stored demand comes from `evara_core::lessons`.
 */
function describeProduct(duration: number, count: number): string {
  if (!Number.isFinite(duration) || !Number.isFinite(count) || duration < 1 || count < 1) {
    return "nothing yet";
  }
  const lessons = count === 1 ? "1 lesson" : `${count} lessons`;
  const each = duration === 1 ? "1 period each" : `${duration} consecutive periods each`;
  const total = duration * count;
  return `${lessons} of ${each} — ${total} period${total === 1 ? "" : "s"} a cycle`;
}
