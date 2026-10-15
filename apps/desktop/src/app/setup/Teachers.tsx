/**
 * Section 11 — Teachers.
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
 * ## The four workload fields are not on this form, and are not gone
 *
 * Stage A removes **full-time equivalent**, **most periods per day**, **most periods per
 * cycle** and **most periods in a row** from the form. Nothing else about them changes:
 * the columns, the record, the generated types, `TeacherInput` and the command signature
 * are all untouched, and the Phase 4 load constraints will read exactly the values that
 * are stored today.
 *
 * They went because they are the wrong question at the wrong time. A school setting itself
 * up is entering a staff list; being asked, per person, for four load ceilings before it
 * has described a single period is being asked to tune a solver it has not run. Phase 4
 * owns workload, and will ask on a screen that can show what the answers do.
 *
 * **The values survive an edit exactly**, and the mechanism is deliberate rather than
 * incidental: `toDraft` reads every stored value into the draft and `toInput` writes every
 * one of them back, whether or not a field renders it. So an edit sends the stored number,
 * not a default and not a `null` — which is the whole risk of hiding a field from a form
 * that submits a whole record. `TeacherFormFields.test.tsx` asserts it against a teacher
 * with four non-default values, because "it should still be in the draft" is exactly the
 * kind of thing that stays true until somebody tidies up.
 *
 * A new teacher gets the schema's own defaults — `fte_percent DEFAULT 100` and three
 * nullable ceilings, from `V10` — rather than anything this screen invents.
 *
 * ## One teacher, possibly several schools
 *
 * Since Stage 0B a teacher is a project-level person with a membership per school, and the
 * list is built from `listSchoolTeachers` — the relationship — rather than from
 * `teacher.schoolId`. So a teacher shared with another school of a trust appears here *and*
 * there, as one record with one identifier, which is what makes their availability a single
 * fact rather than one per school.
 *
 * Stage 0C adds the three things that were missing:
 *
 * - **Adding somebody who already exists.** "Add an existing teacher" searches the whole
 *   project and writes a membership. Nothing on this screen can produce a second record
 *   for one real person, which is the duplication `V13` removed and only an interface
 *   could reintroduce.
 * - **Managing the relationship.** {@link TeacherSchools} below edits the schools they
 *   belong to and the campuses they may be based at, keeping deactivation, removal and
 *   deletion three distinct operations (ADR 0021 §8).
 * - **An edit that is actually allowed.** A visiting teacher's identity belongs to the
 *   school that created them and `updateTeacher` is refused anywhere else. Rather than
 *   letting the user discover that by pressing Save, Edit is unavailable on such a row and
 *   the panel offers to switch to the school where it is permitted.
 *
 * "Active" remains two facts: `teacher.isActive` says whether they are in the project at
 * all, the membership says whether *this* school employs them. The Status column shows the
 * conjunction and distinguishes the two, because a teacher who has left Southmoor and
 * stayed at Northgate must not read as "Active" on Southmoor's screen.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type {
  Campus,
  School,
  Teacher,
  TeacherInSchool,
  TeacherInput,
} from "@evara/domain";
import {
  addTeacherToSchool,
  createTeacher,
  deleteTeacher,
  describeSetupError,
  listCampuses,
  listSchoolTeachers,
  listTeachers,
  updateTeacher,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner, ConfirmDialog } from "../ui/Shell";
import {
  flag,
  number,
  numberOrNull,
  text,
  textOrNull,
  validateTeacherIdentity,
} from "../validation";
import { AvailabilityPanel } from "./AvailabilityGrid";
import { PersonSearch, type Candidate } from "./PersonSearch";
import type { SetupContext } from "./SchoolSetup";
import { TeacherSchools } from "./TeacherSchools";
import { useList } from "./useList";

/**
 * One row of the teachers table.
 *
 * The shared identity with this school's view of it lifted alongside, because `CrudPanel`
 * keys rows by `id` and `TeacherInSchool` keeps the teacher one level down. The `id` is
 * the **teacher's**, not the membership's: it is what every other screen, and every future
 * timetable, names them by.
 */
type TeacherRow = Teacher & { readonly membershipActive: boolean };

function toRow(entry: TeacherInSchool): TeacherRow {
  return { ...entry.teacher, membershipActive: entry.membership.isActive };
}

/**
 * Shown in the Status column, so the states read as states rather than as a blank.
 *
 * Three answers, not two, because there are two flags. "Archived" is the project-level one
 * and outranks the membership: an archived person is offered to nobody, whatever any
 * school's employment record says. "Not teaching here" is the per-school one, and is the
 * ordinary state for somebody who has left one school of a trust and stayed at another.
 */
function statusLabel(row: TeacherRow): string {
  if (!row.isActive) return "Archived";
  if (!row.membershipActive) return "Not teaching here";
  return "Active";
}

export function Teachers(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const schools = props.context?.schools ?? [school];
  const selectSchool = props.context?.selectSchool;

  // Bumped after any change to the teacher list, so the availability picker below cannot
  // go on offering someone who has just been deleted.
  const [listNonce, setListNonce] = useState(0);
  /** Whose relationships the panel at the bottom is about. */
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  // This school's own view of each teacher's employment, which the plain teacher record
  // cannot carry: `isActive` on it is the project-level flag.
  const loadMemberships = useCallback(
    () => listSchoolTeachers(school.id).then((rows) => rows.map(toRow)),
    [school.id],
  );
  const staff = useList<TeacherRow>(loadMemberships);

  /**
   * Which schools employ each teacher in the project.
   *
   * One read per school rather than one per teacher: `listTeachers` is membership-backed,
   * so the union of its results over every school is the whole employment graph. It
   * answers two questions at once — whether a delete will be refused because somebody else
   * still employs them, and which schools to show beside a candidate in the search below.
   */
  const [employers, setEmployers] = useState<ReadonlyMap<string, readonly string[]>>(
    new Map(),
  );
  const [known, setKnown] = useState<readonly Teacher[] | null>(null);
  const [graphError, setGraphError] = useState<string | null>(null);
  const [graphNonce, setGraphNonce] = useState(0);

  const schoolKey = schools.map((candidate) => candidate.id).join(",");

  useEffect(() => {
    let live = true;
    setKnown(null);
    void Promise.all(
      schools.map((candidate) =>
        listTeachers(candidate.id).then((rows) => [candidate.id, rows] as const),
      ),
    )
      .then((pairs) => {
        if (!live) return;
        const byTeacher = new Map<string, string[]>();
        const people = new Map<string, Teacher>();
        for (const [schoolId, rows] of pairs) {
          for (const row of rows) {
            if (!people.has(row.id)) people.set(row.id, row);
            const list = byTeacher.get(row.id) ?? [];
            list.push(schoolId);
            byTeacher.set(row.id, list);
          }
        }
        setEmployers(byTeacher);
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

  const refreshAll = useCallback(() => {
    staff.reload();
    setGraphNonce((value) => value + 1);
    setListNonce((value) => value + 1);
  }, [staff]);

  const campusChoices: readonly Choice[] = useMemo(
    () => (campuses.items ?? []).map((campus) => ({ value: campus.id, label: campus.name })),
    [campuses.items],
  );

  const schoolName = useMemo(
    () => new Map(schools.map((candidate) => [candidate.id, candidate.name])),
    [schools],
  );

  const chosen = (staff.items ?? []).find((row) => row.id === chosenId) ?? null;

  /** How a row's home school reads in a message. */
  const homeOf = (row: TeacherRow): string =>
    schoolName.get(row.schoolId) ?? "another school";

  const spec: CrudSpec<TeacherRow, TeacherInput> = {
    noun: "teacher",
    plural: "teachers",
    title: "Teachers at this school",
    columns: [
      { header: "Last name", render: (row) => row.familyName },
      { header: "First name", render: (row) => row.givenName },
      { header: "Staff code", render: (row) => row.code ?? "—" },
      // Shown, though no longer asked for. The value is real and Phase 4's load
      // constraints read it, so a school that set one must be able to see it — and a
      // column is a far better place for a number nobody needs to change than a form
      // field on every create.
      { header: "FTE", render: (row) => `${row.ftePercent}%`, numeric: true },
      { header: "Status", render: statusLabel },
      {
        header: "Belongs to",
        render: (row) =>
          row.schoolId === school.id ? (
            "This school"
          ) : (
            <span title="Their identity is held by the school that created them">
              Visiting from {homeOf(row)}
            </span>
          ),
      },
      {
        header: "Also at",
        render: (row) => {
          const others = (employers.get(row.id) ?? []).filter((id) => id !== school.id);
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
        label: "Staff code",
        hint: "Optional. Must be unique within the school when given — a staff number is the employer's identifier, so two schools of a trust may legitimately number the same person differently.",
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
        hint: "Optional, and only where they are normally based — not the set of campuses they may work at. That is campus eligibility, under “Schools and campuses” below, and a teacher can be eligible at several sites or at none.",
        choices: campusChoices,
      },
      // Full-time equivalent and the three load ceilings are deliberately absent. They
      // are still read into the draft by `toDraft` and written back by `toInput`, so an
      // edit preserves them exactly; see this module's header for why they are not asked
      // for here.
      { kind: "textarea", key: "notes", label: "Notes" },
      {
        kind: "checkbox",
        key: "isActive",
        label: "In this campus file",
        hint: "The project-level flag — clearing it archives the person everywhere. Whether this particular school employs them is “Schools and campuses” below, and the two are deliberately different questions.",
      },
    ],
    describe: (row) => `${row.givenName} ${row.familyName}`,
    // Only the fields on screen. The four workload values are carried through rather than
    // collected, so validating them as input would be validating something the user cannot
    // see or fix — and an error pinned to an absent field is an unsubmittable form with no
    // visible reason. Their `CHECK` constraints remain the authority, as they always were.
    validate: validateTeacherIdentity,
    toDraft: (row) => ({
      givenName: row?.givenName ?? "",
      familyName: row?.familyName ?? "",
      code: row?.code ?? "",
      email: row?.email ?? "",
      homeCampusId: row?.homeCampusId ?? null,
      ftePercent: String(row?.ftePercent ?? 100),
      maxPeriodsPerDay:
        row?.maxPeriodsPerDay === null || row?.maxPeriodsPerDay === undefined
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
    list: loadMemberships,
    // The panel re-reads the list straight afterwards, so the returned row is never
    // what gets rendered; the membership flag is supplied so `create` and `update`
    // satisfy the spec without inventing a second source of truth. A newly created
    // teacher is employed here, because `create_teacher` writes the membership from the
    // same flag in the same transaction.
    create: (input) =>
      createTeacher(input).then((row) => ({ ...row, membershipActive: row.isActive })),
    update: (row, input) =>
      updateTeacher(row.id, row.rev, input).then((saved) => ({
        ...saved,
        membershipActive:
          (staff.items ?? []).find((known) => known.id === row.id)?.membershipActive ??
          saved.isActive,
      })),
    remove: (row) => deleteTeacher(school.id, row.id, row.rev),
    emptyHint:
      "No teachers at this school yet. Add the staff who will appear on the timetable — or, if somebody already works at another school in this campus file, add them with “Add an existing teacher” rather than entering them twice.",
    rowActions: [
      {
        label: "Schools",
        onClick: (row) => setChosenId(row.id),
      },
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
        : `${row.givenName} ${row.familyName} belongs to ${homeOf(row)}, which holds their name, code, email and workload settings. Use “Go to home school” to change them there.`,
    deleteBlockedBy: (row) => {
      if (row.schoolId !== school.id) {
        return (
          <>
            <p>
              Their identity belongs to <strong>{homeOf(row)}</strong>, and only the school
              that created somebody may delete them.
            </p>
            <p className="muted">
              What this school can do is let go of them: <strong>Schools</strong> below
              removes this school&rsquo;s employment, or marks them as having left. Neither
              touches the person, and every other school keeps them.
            </p>
          </>
        );
      }
      const others = (employers.get(row.id) ?? []).filter((id) => id !== school.id);
      if (others.length === 0) return null;
      return (
        <>
          <p>
            {others.length === 1
              ? "Another school in this campus file still employs them"
              : `${others.length} other schools in this campus file still employ them`}
            :{" "}
            <strong>
              {others.map((id) => schoolName.get(id) ?? "another school").join(", ")}
            </strong>
            .
          </p>
          <p className="muted">
            Deleting the person would take a teacher away from a school that is timetabling
            around them. Each of those schools removes its own employment first — an
            operation it can see and perform — and the delete is then offered.
          </p>
        </>
      );
    },
    deleteNote: (row) =>
      row.isActive
        ? "If this teacher has simply left, clearing “In this campus file” archives them, and “Mark as left” under Schools records that this one school no longer employs them. Either keeps them and their history."
        : "Their availability, their campus eligibility and their school memberships go with them.",
  };

  /** Teachers in the project this school does not already employ. */
  const candidates: readonly Candidate[] | null = useMemo(() => {
    if (known === null) return null;
    const here = new Set((staff.items ?? []).map((row) => row.id));
    return known
      .filter((person) => !here.has(person.id))
      .map((person) => ({
        id: person.id,
        name: `${person.familyName}, ${person.givenName}`,
        details: [
          person.code === null ? null : `Staff code ${person.code}`,
          person.email,
          `Identity held by ${schoolName.get(person.schoolId) ?? "another school"}`,
          `Already at ${(employers.get(person.id) ?? [])
            .map((id) => schoolName.get(id) ?? "another school")
            .join(", ")}`,
          person.isActive ? null : "Archived in this campus file",
        ].filter((detail): detail is string => detail !== null && detail.length > 0),
      }));
  }, [known, staff.items, employers, schoolName]);

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
    void addTeacherToSchool({ teacherId: picked, schoolId: school.id, isActive: true })
      .then(() => {
        closeAdd();
        refreshAll();
        // Opening their relationships straight away: somebody has just been added to this
        // school and the next thing to decide is which of its campuses they work at.
        setChosenId(picked);
      })
      .catch((cause: unknown) => setAddError(describeSetupError(cause)))
      .finally(() => setAddBusy(false));
  }, [picked, school.id, closeAdd, refreshAll]);

  const visiting = (staff.items ?? []).filter((row) => row.schoolId !== school.id).length;

  return (
    <section className="sectionBody">
      <p className="muted">
        The staff a timetable can be built around. A teacher keeps the same identity across
        terms, academic years and schools, so one person shared between two schools of a
        trust is <strong>one record with one identifier</strong> — which is what makes
        &ldquo;is she free at ten?&rdquo; have a single answer.
      </p>
      {campuses.error && <p className="muted">Campuses could not be loaded: {campuses.error}</p>}
      {staff.error && (
        <Banner tone="error">
          This school&rsquo;s employment records could not be loaded: {staff.error}
        </Banner>
      )}
      {graphError && (
        <Banner tone="warning">
          Other schools&rsquo; staff lists could not be read, so adding an existing teacher
          and the delete warnings are unavailable: {graphError}
        </Banner>
      )}

      {/* Said once, visibly, rather than left as a surprise. Somebody who set a ceiling in
          an earlier version and cannot find the field needs to know the value is still
          there — and somebody who never set one needs to know they are not missing a step.
          `role="note"` rather than a banner: a standing explanation, not something that
          just happened. */}
      <p className="panelNote" role="note">
        Workload limits — full-time equivalent and the per-day, per-cycle and consecutive
        period ceilings — are no longer asked for here. Any values already stored are kept
        exactly as they are and are never changed by editing a teacher; FTE is shown in the
        table. The screen that sets them arrives with the load constraints.
      </p>

      {schools.length > 1 && (
        <div className="actions">
          <button
            type="button"
            className="secondary"
            disabled={candidates === null || candidates.length === 0}
            title={
              candidates !== null && candidates.length === 0
                ? "Every teacher in this campus file already works at this school."
                : undefined
            }
            onClick={() => setAdding(true)}
          >
            Add an existing teacher
          </button>
          <span className="panelNote" role="note">
            Somebody who already teaches at another school in this campus file is added
            here, never entered again.
          </span>
        </div>
      )}

      {visiting > 0 && (
        <Banner tone="info">
          {visiting === 1
            ? "One teacher on this list is shared from another school"
            : `${visiting} teachers on this list are shared from another school`}
          . Their names, codes and workload settings belong to the school that created them
          and are not editable here; this school&rsquo;s employment of them is, under{" "}
          <strong>Schools and campuses</strong> below.
        </Banner>
      )}

      <CrudPanel
        spec={spec}
        reloadKey={`${school.id}:${(staff.items ?? []).length}`}
        onChanged={refreshAll}
      />

      <TeacherSchools
        school={school}
        schools={schools}
        teacher={chosen}
        onSelectSchool={(id) => selectSchool?.(id)}
        onChanged={refreshAll}
      />

      <AvailabilityPanel
        key={listNonce}
        school={school}
        entityKind="TEACHER"
        noun="teacher"
        onOpenSection={props.context?.openSection}
        options={(staff.items ?? []).map((row) => ({
          id: row.id,
          name: `${row.familyName}, ${row.givenName}${statusLabel(row) === "Active" ? "" : ` (${statusLabel(row).toLowerCase()})`}`,
        }))}
      />

      <ConfirmDialog
        open={adding}
        title="Add an existing teacher to this school"
        confirmLabel="Add to this school"
        busy={addBusy}
        confirmBlockedBy={picked === null ? "Choose a teacher from the list first." : null}
        onConfirm={confirmAdd}
        onCancel={closeAdd}
      >
        <p>
          This adds an existing person to <strong>{school.name}</strong>. No second teacher
          record is created: they keep their identifier, their availability and every other
          school&rsquo;s employment of them.
        </p>
        {addError && <Banner tone="error">{addError}</Banner>}
        <PersonSearch
          legend="Teachers already in this campus file"
          candidates={candidates}
          error={graphError}
          selectedId={picked}
          onSelect={setPicked}
          disabled={addBusy}
          emptyTitle="Nobody to add"
          emptyHint="Every teacher in this campus file already works at this school. Use “Add teacher” above to enter somebody new."
        />
        <p className="muted">
          Not finding them? They may genuinely be a different person with a similar name.
          Evara never merges two records because they share one — use{" "}
          <strong>Add teacher</strong> for somebody new.
        </p>
      </ConfirmDialog>
    </section>
  );
}
