/**
 * The class sections of one course, and the detail of whichever one is chosen.
 *
 * ## Why a course has sections rather than being one
 *
 * "Year 9 Mathematics" is a curriculum decision; "Year 9 Mathematics A" is a timetabling
 * object with a teacher, an audience and a place in the week. A year group split into three
 * classes is one course and three sections, and a model that assumed one of each would make
 * the second and third inexpressible — which is why nothing on this screen assumes a course
 * has exactly one section, or that a section has exactly one teacher.
 *
 * ## The section's identity is its `id`
 *
 * Names and codes may change — a school renaming "A" to "Set 1" mid-year is ordinary — and
 * nothing breaks, because every reference is by UUIDv7 and those are never renumbered. That
 * is what makes Stage D's imports able to find an existing section rather than creating a
 * duplicate.
 *
 * ## What each column is counting
 *
 * The audience figure is the **groups'**, never the groups' plus the roster's. A section
 * taught to a group of 28 that also names four children has an audience of 28, because
 * those four are among the 28; see
 * [ADR 0029](../../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
 * §4. The two are shown in separate columns with separate headings for that reason.
 */
import { useCallback, useMemo, useState, type ReactNode } from "react";

import type {
  Campus,
  ClassSection,
  ClassSectionDependents,
  ClassSectionInput,
  ClassSectionView,
  DeclaredComparison,
  DemandFinding,
  LessonDemand,
  School,
} from "@evara/domain";
import {
  classSectionDependents,
  createClassSection,
  deleteClassSection,
  listCampuses,
  listClassSections,
  updateClassSection,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner, EmptyState } from "../ui/Shell";
import { text, textOrNull, validateClassSection } from "../validation";
import { ClassSectionDetail } from "./ClassSectionDetail";
import type { CourseRow } from "./Lessons";
import { useList } from "./useList";

/**
 * One row of the class-sections table.
 *
 * The section's own fields lifted alongside its counts and its demand, because `CrudPanel`
 * keys rows by `id` and `ClassSectionView` keeps it one level down. Same adapter
 * `RoomsAndRoomTypes` uses for `RoomWithCampuses`.
 */
export type SectionRow = ClassSection & {
  readonly campusName: string | null;
  readonly teachers: number;
  readonly groups: number;
  readonly groupPopulation: number;
  readonly enrolledStudents: number;
  readonly demand: LessonDemand;
  readonly declared: DeclaredComparison;
  readonly findings: readonly DemandFinding[];
};

function toRow(view: ClassSectionView): SectionRow {
  return {
    ...view.section,
    campusName: view.campusName,
    teachers: view.teachers,
    groups: view.groups,
    groupPopulation: view.groupPopulation,
    enrolledStudents: view.enrolledStudents,
    demand: view.demand,
    declared: view.declared,
    findings: view.findings,
  };
}

export function ClassSections(props: {
  school: School;
  /** The chosen course, or `null` when none is. */
  course: CourseRow | null;
  openSection?: (id: string) => void;
}) {
  const { school, course, openSection } = props;

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  /** Which section the detail panels below are about. */
  const [chosen, setChosen] = useState<SectionRow | null>(null);
  const [reports, setReports] = useState<ReadonlyMap<string, ClassSectionDependents>>(
    new Map(),
  );

  const noteDependents = useCallback((id: string) => {
    void classSectionDependents(id)
      .then((report) => setReports((current) => new Map(current).set(id, report)))
      .catch(() => undefined);
  }, []);

  const campusChoices: readonly Choice[] = useMemo(
    () =>
      (campuses.items ?? []).map((row) => ({
        value: row.id,
        label: `${row.name} (${row.code})`,
      })),
    [campuses.items],
  );

  if (course === null) {
    return (
      <div className="crudPanel">
        <div className="panelHead">
          <h3>Class sections</h3>
        </div>
        <EmptyState title="No course chosen">
          Choose <strong>Class sections</strong> on a course above to see and edit the
          classes that deliver it.
        </EmptyState>
      </div>
    );
  }

  const courseId = course.id;

  const spec: CrudSpec<SectionRow, ClassSectionInput> = {
    noun: "class section",
    plural: "class sections",
    title: `Class sections of ${course.name}`,
    columns: [
      { header: "Section", render: (row) => row.name },
      { header: "Code", render: (row) => row.code ?? "—" },
      {
        header: "Campus",
        render: (row) =>
          row.campusName ?? <span className="absent">Not set</span>,
      },
      { header: "Teachers", numeric: true, render: (row) => String(row.teachers) },
      {
        header: "Audience",
        numeric: true,
        render: (row) =>
          row.groups === 0 ? (
            <span className="absent">No groups</span>
          ) : (
            `${row.groupPopulation} in ${row.groups}`
          ),
      },
      {
        header: "Named",
        numeric: true,
        render: (row) =>
          row.enrolledStudents === 0 ? (
            <span className="absent">—</span>
          ) : (
            String(row.enrolledStudents)
          ),
      },
      {
        header: "Per cycle",
        numeric: true,
        render: (row) => describeDemand(row),
      },
    ],
    fields: [
      {
        kind: "text",
        key: "name",
        label: "Section name",
        required: true,
        placeholder: "A",
        hint: "Unique within this course. “A”, “B”, “Set 1”, “Laboratory Group 1” — whatever this school calls its classes.",
      },
      {
        kind: "text",
        key: "code",
        label: "Section code",
        hint: "Optional. Unique within this course, not across the school: a section code is read next to its course, so “A” for Mathematics and “A” for Physics are both fine.",
      },
      {
        kind: "select",
        key: "campusId",
        label: "Campus",
        emptyLabel: "Not tied to a campus",
        choices: campusChoices,
        hint:
          campusChoices.length === 0
            ? "This school has no campuses yet. A section does not need one — add campuses in section 3 if its lessons happen at a particular site."
            : "Optional. Setting it means only teachers eligible at that campus can be given this section, so a section with no campus is the right answer unless the site matters.",
      },
    ],
    describe: (row) => `${course.name} — ${row.name}`,
    validate: validateClassSection,
    toDraft: (row) => ({
      name: row?.name ?? "",
      code: row?.code ?? "",
      campusId: row?.campusId ?? null,
    }),
    toInput: (draft) => ({
      courseId,
      name: text(draft, "name"),
      code: textOrNull(draft, "code"),
      campusId: textOrNull(draft, "campusId"),
    }),
    list: () =>
      listClassSections(school.id, courseId).then((views) => {
        for (const view of views) noteDependents(view.section.id);
        return views.map(toRow);
      }),
    // Never rendered: the panel re-reads the list straight afterwards. A brand-new section
    // has nothing on it, so the counts and the demand are honestly zero rather than
    // invented.
    create: (input) =>
      createClassSection(school.id, input).then((created) => ({
        ...created,
        campusName: null,
        teachers: 0,
        groups: 0,
        groupPopulation: 0,
        enrolledStudents: 0,
        demand: { occurrences: 0, periodsPerCycle: 0, longestOccurrence: 0 },
        declared: { kind: "notDeclared" } as DeclaredComparison,
        findings: [],
      })),
    update: (row, input) =>
      updateClassSection(school.id, row.id, row.rev, input).then((saved) => ({
        ...row,
        ...saved,
      })),
    remove: (row) => deleteClassSection(school.id, row.id, row.rev),
    emptyHint: (
      <>
        Add a class section to define who teaches this course and which students attend.
        <br />
        A course can have as many as the school needs — a year group split into three
        classes is one course and three sections, each with its own staff and audience.
      </>
    ),
    rowActions: [{ label: "Open", onClick: (row) => setChosen(row) }],
    /**
     * Deleting a section is confirmed with counts, but is never **blocked** by them.
     *
     * The opposite of a course, and deliberately: a section owns its staffing, audience,
     * roster and occurrences, so they go with it. Refusing instead would mean removing
     * four teachers, two groups and six occurrences one at a time before a section could
     * be deleted at all. The confirmation therefore says what is about to go rather than
     * refusing to go.
     */
    deleteNote: (row) => {
      const report = reports.get(row.id);
      const total = report?.total ?? 0;
      if (total === 0) {
        return (
          <p className="muted">
            Nothing is attached to this section yet, so nothing else goes with it.
          </p>
        );
      }
      return (
        <>
          <p>This will also remove, from this section only:</p>
          <ul>
            {report !== undefined && report.teachers > 0 && (
              <li>
                {report.teachers === 1
                  ? "1 teacher assignment"
                  : `${report.teachers} teacher assignments`}{" "}
                — the teachers themselves are untouched
              </li>
            )}
            {report !== undefined && report.groups > 0 && (
              <li>
                {report.groups === 1
                  ? "1 attending student group"
                  : `${report.groups} attending student groups`}{" "}
                — the groups and their members are untouched
              </li>
            )}
            {report !== undefined && report.enrolledStudents > 0 && (
              <li>
                {report.enrolledStudents === 1
                  ? "1 named student on the roster"
                  : `${report.enrolledStudents} named students on the roster`}{" "}
                — their school enrolment is untouched
              </li>
            )}
            {report !== undefined && report.activities > 0 && (
              <li>
                {report.activities === 1
                  ? "1 lesson occurrence"
                  : `${report.activities} lesson occurrences`}
              </li>
            )}
          </ul>
        </>
      );
    },
  };

  return (
    <>
      {campuses.error !== null && (
        <Banner tone="warning">
          This school&rsquo;s campuses could not be loaded, so the campus picker is empty:{" "}
          {campuses.error}
        </Banner>
      )}

      <CrudPanel
        spec={spec}
        reloadKey={courseId}
        onChanged={() => {
          if (chosen !== null) {
            void listClassSections(school.id, courseId)
              .then((views) => {
                const found = views.find((view) => view.section.id === chosen.id);
                setChosen(found === undefined ? null : toRow(found));
              })
              .catch(() => undefined);
          }
        }}
      />

      <ClassSectionDetail
        key={chosen?.id ?? "none"}
        school={school}
        course={course}
        section={chosen}
        openSection={openSection}
        onChanged={() => {
          void listClassSections(school.id, courseId)
            .then((views) => {
              for (const view of views) noteDependents(view.section.id);
              if (chosen !== null) {
                const found = views.find((view) => view.section.id === chosen.id);
                setChosen(found === undefined ? null : toRow(found));
              }
            })
            .catch(() => undefined);
        }}
      />
    </>
  );
}

/**
 * A section's demand in one cell.
 *
 * `6 in 5` reads as "six period slots, across five lessons", which is the pair of numbers
 * the model actually holds — four singles and a double is five occurrences and six periods,
 * and no single figure says that.
 */
function describeDemand(row: SectionRow): ReactNode {
  if (row.demand.occurrences === 0) {
    return <span className="absent">No lessons</span>;
  }
  const { periodsPerCycle, occurrences } = row.demand;
  const label =
    periodsPerCycle === occurrences
      ? String(periodsPerCycle)
      : `${periodsPerCycle} in ${occurrences}`;
  if (row.findings.length > 0) {
    return (
      <span title="This section's demand cannot be met by the current cycle and bell schedules. Open it to see why.">
        {label} ⚠
      </span>
    );
  }
  return label;
}
