/**
 * Section 1 — Schools.
 *
 * The project-level screen Stage 0A deferred and Stage 0C owes:
 * [ADR 0019](../../../../../docs/adr/0019-multiple-schools-in-one-project.md) made a
 * campus file able to hold several institutions, and until now the only way to add one was
 * a repository call. This is where they are created, renamed, coded, retimed and removed.
 *
 * ## Project, school, campus — three different things
 *
 * The file is a **campus file** (see `App.tsx` for why that word, and why the extra noun).
 * It holds **schools**, each an institution. Each school holds **campuses**, its physical
 * sites. "Campus" never means the file here, and this screen is the one place all three
 * are visible at once, so the copy says it plainly.
 *
 * ## A new school is empty, and that is the point
 *
 * `create_school` copies nothing: no campuses, no academic years, no year levels, no
 * cycle, no bell schedule. Two schools of a trust usually *do* run different years and
 * different rotations, which is the whole reason they are two schools rather than two
 * campuses — so a "copy from" button would be offering a shortcut that is wrong more often
 * than it is right. The empty state after a create says what to do first instead.
 *
 * ## Deleting is refused, explained, and never widened
 *
 * Every school-owned table references `school (id)` with `ON DELETE RESTRICT` and ADR 0019
 * kept it that way on purpose: a school is years of configuration and one mis-click must
 * not take a campus, a timetable grid and a staff list with it. There is no
 * "delete everything in it".
 *
 * So the dependent counts are read **before** the action is offered — the pattern
 * `student_group_member_count` established — and a school with anything in it gets an
 * explanation naming what is in the way rather than a button that fails. Shared people are
 * counted separately by `setup_school_dependents` and are explained separately here,
 * because "two teachers another school entered" is a different remedy from "two teachers
 * of your own".
 */
import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { School, SchoolDependents, SchoolInput } from "@evara/domain";
import {
  createSchool,
  deleteSchool,
  listSchools,
  schoolDependents,
  updateSchool,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { Banner } from "../ui/Shell";
import { validateSchool } from "../validation";
import { localeLabel } from "./locales";
import { schoolDraft, schoolFields, schoolInput } from "./schoolForm";
import { timezoneLabel } from "./timezones";
import { describeWeek } from "./weekdays";

/** One dependent kind, as the confirmation spells it out. */
interface Blocker {
  readonly count: number;
  readonly label: string;
}

/**
 * What is standing in the way of deleting this school, longest-first.
 *
 * Reads the report rather than guessing: `total` is the backend's own sum, so "can this be
 * deleted?" is one field and "what is in the way?" is the rest. Shared people are last
 * because their remedy lives in another school's screen, and the user should read their own
 * configuration first.
 */
function blockers(found: SchoolDependents): readonly Blocker[] {
  const kinds: ReadonlyArray<readonly [number, string, string]> = [
    [found.campuses, "campus", "campuses"],
    [found.roomTypes, "room type", "room types"],
    [found.resources, "resource", "resources"],
    [found.academicYears, "academic year", "academic years"],
    [found.cycles, "cycle", "cycles"],
    [found.periodStructures, "bell schedule", "bell schedules"],
    [found.timeslots, "timetable slot", "timetable slots"],
    [found.calendarDays, "calendar mapping", "calendar mappings"],
    [found.yearLevels, "year level", "year levels"],
    [found.teachers, "teacher", "teachers"],
    [found.students, "student", "students"],
    [found.studentGroups, "student group", "student groups"],
    [found.availabilityRules, "availability rule", "availability rules"],
    [found.constraints, "configured constraint", "configured constraints"],
    [found.sharedTeachers, "teacher shared from another school", "teachers shared from another school"],
    [found.sharedStudents, "student enrolled from another school", "students enrolled from another school"],
  ];
  return kinds
    .filter(([count]) => count > 0)
    .map(([count, one, many]) => ({ count, label: count === 1 ? one : many }));
}

export function Schools(props: {
  /** Which school the rest of setup is about, so this screen can mark it. */
  activeSchoolId: string | null;
  /** Makes a school the active one. */
  onSelect: (id: string) => void;
  /** Opens another setup section — used to send the user on to School details. */
  onOpenSection: (id: string) => void;
  /** Called after any change, so the shell re-reads the project's schools. */
  onChanged: () => void;
}) {
  const { activeSchoolId, onSelect, onOpenSection, onChanged } = props;

  const [schools, setSchools] = useState<readonly School[]>([]);

  /**
   * What each school still owns.
   *
   * Read per school up front rather than when Delete is pressed. A confirmation that has
   * to wait for a round trip either flickers or asks before it knows, and the counts are
   * the whole reason the dialogue is worth reading — the same argument `StudentGroups`
   * makes for its member counts.
   */
  const [dependents, setDependents] = useState<Readonly<Record<string, SchoolDependents>>>(
    {},
  );
  const [countError, setCountError] = useState<string | null>(null);

  useEffect(() => {
    if (schools.length === 0) return undefined;
    let live = true;
    void Promise.all(
      schools.map((school) =>
        schoolDependents(school.id).then(
          (found) => [school.id, found] as const,
          () => null,
        ),
      ),
    ).then((pairs) => {
      if (!live) return;
      const read = pairs.filter((pair) => pair !== null);
      setDependents(Object.fromEntries(read));
      setCountError(
        read.length === schools.length
          ? null
          : "Some schools' contents could not be counted, so deleting them is not offered here.",
      );
    });
    return () => {
      live = false;
    };
  }, [schools]);

  const spec: CrudSpec<School, SchoolInput> = useMemo(
    () => ({
      noun: "school",
      plural: "schools",
      title: "Schools in this campus file",
      describe: (row) => (row.code ? `${row.name} (${row.code})` : row.name),
      columns: [
        {
          header: "Name",
          render: (row) => (
            <>
              {row.name}
              {row.id === activeSchoolId && (
                <span className="chipBadge" title="The school the rest of setup is about">
                  {" "}
                  selected
                </span>
              )}
            </>
          ),
        },
        { header: "Code", render: (row) => (row.code ? <code>{row.code}</code> : "—") },
        { header: "Time zone", render: (row) => timezoneLabel(row.timezone) },
        { header: "Locale", render: (row) => localeLabel(row.locale) },
        { header: "Working days", render: (row) => describeWeek(row.workingDays) },
        {
          header: "Contents",
          numeric: true,
          render: (row) => {
            const found = dependents[row.id];
            if (found === undefined) return "—";
            return found.total === 0 ? "Empty" : String(found.total);
          },
        },
      ],
      // Every school's stored values, not just the row being edited: the panel resolves
      // `fields` from the draft and does not say which row it was opened on, and offering
      // a value already in the list costs nothing. See `schoolFields`.
      fields: schoolFields(schools),
      validate: validateSchool,
      toDraft: schoolDraft,
      toInput: schoolInput,
      list: () =>
        listSchools().then((rows) => {
          setSchools(rows);
          return rows;
        }),
      create: (input) => createSchool(input),
      update: (row, input) => updateSchool(row.id, row.rev, input),
      remove: (row) => deleteSchool(row.id, row.rev),
      rowActions: [
        {
          label: "Select",
          onClick: (row) => onSelect(row.id),
          blockedBy: (row) =>
            row.id === activeSchoolId
              ? "This school is already the one the rest of setup is about."
              : null,
        },
        {
          label: "Open details",
          onClick: (row) => {
            onSelect(row.id);
            onOpenSection("school");
          },
        },
      ],
      deleteBlockedBy: (row) => {
        const found = dependents[row.id];
        if (found === undefined) {
          return (
            <p>
              Evara could not count what this school still holds, so it will not offer to
              delete it. Reopen this campus file and try again.
            </p>
          );
        }
        if (found.total === 0) return null;
        return <DeleteBlocked school={row} dependents={found} />;
      },
      emptyHint:
        "Nothing has been set up in this campus file yet. Add the first school — give it a name, a time zone, a locale and its working days, and the rest of setup opens up.",
      deleteNote: () => (
        <p className="muted">
          This school holds nothing, so nothing goes with it. Its logo, if it has one, is
          removed too — a logo is a picture of the school rather than something it owns.
        </p>
      ),
    }),
    [activeSchoolId, dependents, onOpenSection, onSelect, schools],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Schools</h2>
        <p>
          A campus file can describe several schools — a trust, a group, or a school running
          a separate junior programme. Each one owns its own campuses, academic years, year
          levels, cycle and bell schedules, and they <strong>share</strong> staff, students,
          rooms and equipment rather than duplicating them.
        </p>
        <p className="muted">
          A <strong>school</strong> is an institution; a <strong>campus</strong> is one of
          its physical sites. Schools with nothing in common gain nothing from sharing a
          campus file — use a second file for those.
        </p>
      </header>

      {countError && <Banner tone="warning">{countError}</Banner>}

      <CrudPanel
        spec={spec}
        onChanged={() => {
          onChanged();
          // The counts are per school and a create or delete changes which schools exist,
          // so the map is rebuilt from the new list rather than patched.
          setDependents({});
        }}
      />

      {schools.length > 1 && (
        <p className="muted">
          <strong>Selected:</strong>{" "}
          {spec.describe(
            schools.find((school) => school.id === activeSchoolId) ?? schools[0]!,
          )}
          . Every section after this one is about that school. Use{" "}
          <strong>Select</strong> above, or the School selector in the sidebar, to change
          it.
        </p>
      )}
    </div>
  );
}

/** Why this school cannot be deleted, and what to do about it. */
function DeleteBlocked(props: { school: School; dependents: SchoolDependents }): ReactNode {
  const { school, dependents } = props;
  const inTheWay = blockers(dependents);
  const sharedPeople = dependents.sharedTeachers + dependents.sharedStudents;

  return (
    <>
      <p>
        <strong>{school.name}</strong> still holds {dependents.total} record
        {dependents.total === 1 ? "" : "s"}. Evara never deletes a school and everything in
        it: a school is years of configuration, and one mis-click must not be able to take a
        campus, a timetable grid and a staff list with it.
      </p>
      <ul className="planList">
        {inTheWay.map((blocker) => (
          <li key={blocker.label}>
            {blocker.count} {blocker.label}
          </li>
        ))}
      </ul>
      <p className="muted">
        Empty the school section by section — each step confirms before it removes anything
        — and this school can then be deleted.
      </p>
      {sharedPeople > 0 && (
        <p className="warning">
          {sharedPeople === 1
            ? "One of those people was entered by another school"
            : `${sharedPeople} of those people were entered by another school`}
          . Removing this school&rsquo;s membership or enrolment of them under{" "}
          <strong>Teachers</strong> or <strong>Students</strong> lets go of them here
          without deleting the person, who stays with the school that holds their identity.
        </p>
      )}
    </>
  );
}
