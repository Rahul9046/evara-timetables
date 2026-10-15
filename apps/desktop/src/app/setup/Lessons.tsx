/**
 * Section 10 — Lessons.
 *
 * The section the navigation has been announcing since Stage A, and the first screen in
 * Evara where scheduling *demand* is entered rather than the structure it will be placed
 * into.
 *
 * ## Three things, and the user has to be able to see which is which
 *
 * | | What it is | Example |
 * | --- | --- | --- |
 * | **Subject** | what is taught | Mathematics |
 * | **Course** | *this school's* offering of a subject | Year 9 Mathematics |
 * | **Class section** | one deliverable instance of that offering | Year 9 Mathematics A |
 * | **Lesson occurrence** | one placeable unit of that instance | a single period of it |
 *
 * The section is called **Lessons** because that is the word a school administrator uses,
 * and the brief asks for it. What it must *not* do is hide the distinctions underneath:
 * a school with two Year 9 Mathematics sections taught by different people to different
 * children has three objects, not one, and a screen that flattened them would make the
 * second section inexpressible. So the hierarchy is visible and navigated —
 * **Course → Class section → occurrences** — and each level says what it is for.
 *
 * ## Four periods a cycle is four rows
 *
 * The single most important thing on this screen, and the one most likely to be read
 * wrongly. An occurrence is **one placeable unit**: a class meeting four times a cycle for
 * one period holds four of them, and a class meeting once for a double holds one with a
 * duration of 2.
 *
 * So the Occurrences panel asks two numbers — *periods per lesson* and *times per cycle* —
 * and writes `count` rows. Their product is the demand, and the panel says so out loud,
 * because "duration" and "total load" are the two figures a user is most likely to
 * conflate. [ADR 0029](../../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
 * argues why rows rather than a count, and `evara_core::lessons` does the arithmetic so
 * that this screen never performs any.
 *
 * ## Nothing here says a lesson can be timetabled
 *
 * The demand summary reports **impossibility** and never feasibility. "This occurrence
 * needs four consecutive periods and no day has more than three in a row" is arithmetic.
 * "Twenty-nine periods in a thirty-period cycle" is probably infeasible and is *not*
 * reported, because only a solver knows and there is not one yet. A screen that promised
 * otherwise would spend the user's trust on a guess.
 *
 * ## A missing qualification warns; a missing membership refuses
 *
 * The teacher picker is where [ADR 0028](../../../../../docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md)
 * becomes visible. Three answers, two of which are refusals and one of which is not, and
 * all three computed by the backend — a screen that worked any of them out for itself
 * would be a scheduling rule in the view layer.
 *
 * ## Groups are the audience; the roster is a register
 *
 * A section taught to a group of 28 that also names four children has an audience of
 * **28**, because those four are among the 28. Nothing adds the two numbers, and the panel
 * says which is which rather than leaving a user to assume a total. ADR 0006 requires that
 * a school be able to timetable from group sizes alone, so the roster is optional
 * everywhere and nothing treats an empty one as unfinished.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  AcademicYear,
  Course,
  CourseDependents,
  CourseInput,
  CourseView,
  RoomType,
  School,
  Subject,
  Term,
  YearLevel,
} from "@evara/domain";
import {
  courseDependents,
  createCourse,
  deleteCourse,
  listAcademicYears,
  listCourses,
  listRoomTypes,
  listSubjects,
  listTerms,
  listYearLevels,
  updateCourse,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice, FieldSpec } from "../ui/Field";
import { Banner, EmptyState } from "../ui/Shell";
import { flag, number, numberOrNull, text, textOrNull, validateCourse, type Draft } from "../validation";
import { ClassSections } from "./ClassSections";
import type { SetupContext } from "./SchoolSetup";
import { useList } from "./useList";

/**
 * One row of the courses table.
 *
 * The course's own fields lifted alongside the names its listing needs, because
 * `CrudPanel` keys rows by `id` and `CourseView` keeps it one level down. The same
 * adapter `RoomsAndRoomTypes` uses for `RoomWithCampuses`, and for the same reason.
 *
 * `subject` stays nested: it is a whole record rather than a label, the panels below need
 * its name and its identifier, and flattening it would collide with the course's own
 * `name` and `code`.
 */
export type CourseRow = Course & {
  readonly subject: Subject;
  readonly yearLevelName: string | null;
  readonly termName: string | null;
  readonly roomTypeName: string | null;
  readonly sections: number;
};

function toRow(view: CourseView): CourseRow {
  return {
    ...view.course,
    subject: view.subject,
    yearLevelName: view.yearLevelName,
    termName: view.termName,
    roomTypeName: view.roomTypeName,
    sections: view.sections,
  };
}

export function Lessons(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const openSection = props.context?.openSection;

  const loadSubjects = useCallback(() => listSubjects(school.id), [school.id]);
  const subjects = useList<Subject>(loadSubjects);

  const loadYearLevels = useCallback(() => listYearLevels(school.id), [school.id]);
  const yearLevels = useList<YearLevel>(loadYearLevels);

  const loadRoomTypes = useCallback(() => listRoomTypes(school.id), [school.id]);
  const roomTypes = useList<RoomType>(loadRoomTypes);

  const loadYears = useCallback(() => listAcademicYears(school.id), [school.id]);
  const years = useList<AcademicYear>(loadYears);

  /** Which course the class-section panel below is about. */
  const [chosen, setChosen] = useState<CourseRow | null>(null);

  /**
   * How many class sections each course has, read as the table renders.
   *
   * `CourseView.sections` already carries the figure, so this exists for one reason:
   * `deleteBlockedBy` must answer **synchronously**, because the confirmation decides from
   * it whether to offer a destructive button at all. The same arrangement Subjects and
   * Departments arrived at, for the same reason.
   */
  const [reports, setReports] = useState<ReadonlyMap<string, CourseDependents>>(new Map());

  const noteDependents = useCallback((id: string) => {
    void courseDependents(id)
      .then((report) => setReports((current) => new Map(current).set(id, report)))
      .catch(() => undefined);
  }, []);

  const subjectChoices: readonly Choice[] = useMemo(
    () =>
      (subjects.items ?? []).map((row) => ({
        value: row.id,
        label: row.code ? `${row.name} (${row.code})` : row.name,
      })),
    [subjects.items],
  );

  const yearLevelChoices: readonly Choice[] = useMemo(
    () => (yearLevels.items ?? []).map((row) => ({ value: row.id, label: row.name })),
    [yearLevels.items],
  );

  const roomTypeChoices: readonly Choice[] = useMemo(
    () =>
      (roomTypes.items ?? []).map((row) => ({
        value: row.id,
        label: `${row.name} (${row.code})`,
      })),
    [roomTypes.items],
  );

  /**
   * Every term of every one of this school's academic years, labelled by both.
   *
   * Read through the years rather than from a single call, because a term belongs to a
   * school only by way of its academic year — the same chain the repository's owner query
   * walks, and the reason "Term 1" on its own would be ambiguous in a school that has
   * configured two years.
   */
  const loadTerms = useCallback(() => {
    const list = years.items ?? [];
    return Promise.all(
      list.map((year) =>
        listTerms(year.id).then((terms: Term[]) =>
          terms.map((term) => ({
            value: term.id,
            label: `${year.name} — ${term.name}`,
          })),
        ),
      ),
    ).then((pairs) => pairs.flat());
  }, [years.items]);
  const terms = useList<Choice>(loadTerms);

  // `useList` holds its rows in state, so this identity is stable between loads and the
  // memoised field list below can depend on it.
  const termChoices: readonly Choice[] = terms.items ?? [];

  const noSubjects = subjects.items !== null && subjects.items.length === 0;

  /**
   * The course form, as a function of its own draft.
   *
   * The one reason `CrudSpec.fields` accepts a function: a course's **subject never
   * changes**. `update_course` does not take one, because repointing an offering would
   * turn a Mathematics course into a Physics course while keeping its sections, staff and
   * roster — which is a new course rather than an edit. So on an edit the picker shows the
   * stored subject and refuses to change it, with a hint saying why, rather than offering
   * a change the backend would ignore.
   *
   * `existing` is set by {@link CrudSpec.toDraft}, which is the only place that knows
   * whether a row is being edited or created.
   */
  const fields = useCallback(
    (draft: Draft): readonly FieldSpec[] => [
      {
        kind: "combobox",
        key: "subjectId",
        label: "Subject",
        required: true,
        choices: subjectChoices,
        placeholder: "Search this school's subjects…",
        emptyLabel: "No subject of this school matches",
        hint: flag(draft, "existing")
          ? "A course is an offering of one subject, and that cannot be changed. Create a separate course under the other subject instead."
          : "Only this school's subjects are offered. Another school's Mathematics is a different subject, and a course of it would be a course of that school.",
      },
      {
        kind: "text",
        key: "name",
        label: "Course name",
        required: true,
        placeholder: "Year 9 Mathematics",
        hint: "Unique within the subject. Two offerings of Mathematics may not both be called “Year 9”; a Mathematics course and a Physics course may both be called “Core”.",
      },
      {
        kind: "text",
        key: "code",
        label: "Course code",
        hint: "Optional. Unique within this school when given, across every subject — a code like “9MAT” is quoted on its own, so it has to mean one course.",
      },
      {
        kind: "select",
        key: "yearLevelId",
        label: "Year level",
        emptyLabel: "No particular year level",
        choices: yearLevelChoices,
        hint:
          yearLevelChoices.length === 0
            ? "This school has no year levels yet. A course does not need one — leave this as it is, or add them in section 7 first."
            : "Optional, and only this school's year levels are offered. A course open to several years is left without one.",
      },
      {
        kind: "select",
        key: "termId",
        label: "Term",
        emptyLabel: "Runs all year",
        choices: termChoices,
        hint:
          termChoices.length === 0
            ? "This school has no terms yet, so every course will run all year. Add academic years and terms in section 6 if a course runs only part of the year."
            : "Leave as “Runs all year” unless this course runs only in one term.",
      },
      {
        kind: "number",
        key: "periodsPerCycle",
        label: "Periods per cycle",
        min: 1,
        hint: "Optional. How many period slots one section of this course needs in a cycle — the figure the occurrence generator expands. Leave blank if the school has not decided; nothing will guess.",
      },
      {
        kind: "number",
        key: "defaultDurationPeriods",
        label: "Periods per lesson",
        required: true,
        min: 1,
        hint: "How long one lesson is by default. 4 periods a cycle at 1 period per lesson is four single lessons; at 2 it is two doubles.",
      },
      {
        kind: "select",
        key: "requiredRoomTypeId",
        label: "Required room type",
        emptyLabel: "No room requirement",
        choices: roomTypeChoices,
        hint:
          roomTypeChoices.length === 0
            ? "This school has no room types yet. Add them in section 4 if a course needs a particular kind of room."
            : "Optional. A requirement, not a booking — nothing here reserves a room, and room allocation is a later phase.",
      },
      { kind: "textarea", key: "description", label: "Description", hint: "Optional." },
    ],
    [roomTypeChoices, subjectChoices, termChoices, yearLevelChoices],
  );

  const spec: CrudSpec<CourseRow, CourseInput> = {
    noun: "course",
    plural: "courses",
    title: "Courses",
    columns: [
      { header: "Course", render: (row) => row.name },
      { header: "Code", render: (row) => row.code ?? "—" },
      { header: "Subject", render: (row) => row.subject.name },
      {
        header: "Year level",
        render: (row) =>
          row.yearLevelName ?? <span className="absent">Any year</span>,
      },
      {
        header: "Term",
        render: (row) => row.termName ?? <span className="absent">All year</span>,
      },
      {
        header: "Per cycle",
        numeric: true,
        render: (row) =>
          row.periodsPerCycle === null ? (
            <span className="absent">Not set</span>
          ) : (
            describeDeclaration(row)
          ),
      },
      { header: "Sections", numeric: true, render: (row) => String(row.sections) },
    ],
    fields,
    describe: (row) => row.name,
    validate: validateCourse,
    toDraft: (row) => ({
      // Set for an edit and absent for a create, which is the only signal `fields` has for
      // "this course already exists and its subject is settled".
      existing: row !== null,
      subjectId: row?.subject.id ?? "",
      name: row?.name ?? "",
      code: row?.code ?? "",
      yearLevelId: row?.yearLevelId ?? null,
      termId: row?.termId ?? null,
      periodsPerCycle:
        row === null || row.periodsPerCycle === null ? "" : String(row.periodsPerCycle),
      defaultDurationPeriods: String(row?.defaultDurationPeriods ?? 1),
      requiredRoomTypeId: row?.requiredRoomTypeId ?? null,
      description: row?.description ?? "",
    }),
    toInput: (draft) => ({
      subjectId: text(draft, "subjectId"),
      yearLevelId: textOrNull(draft, "yearLevelId"),
      termId: textOrNull(draft, "termId"),
      name: text(draft, "name"),
      code: textOrNull(draft, "code"),
      description: textOrNull(draft, "description"),
      periodsPerCycle: numberOrNull(draft, "periodsPerCycle"),
      defaultDurationPeriods: number(draft, "defaultDurationPeriods"),
      requiredRoomTypeId: textOrNull(draft, "requiredRoomTypeId"),
    }),
    list: () =>
      listCourses(school.id).then((views) => {
        for (const view of views) noteDependents(view.course.id);
        return views.map(toRow);
      }),
    // The row a create resolves to is never rendered — `CrudPanel` re-reads the list
    // straight afterwards — so this fills the projection fields from what the form already
    // had rather than inventing a second source of truth. `RoomsAndRoomTypes.toNewRow`
    // makes the same move for the same reason.
    create: (input) =>
      createCourse(school.id, input).then((created) => ({
        ...created,
        subject:
          (subjects.items ?? []).find((row) => row.id === created.subjectId) ??
          ({ id: created.subjectId, name: "" } as Subject),
        yearLevelName: labelOf(yearLevelChoices, created.yearLevelId),
        termName: labelOf(termChoices, created.termId),
        roomTypeName: labelOf(roomTypeChoices, created.requiredRoomTypeId),
        sections: 0,
      })),
    update: (row, input) =>
      updateCourse(school.id, row.id, row.rev, input).then((saved) => ({
        ...row,
        ...saved,
      })),
    remove: (row) => deleteCourse(school.id, row.id, row.rev),
    blockedBy: noSubjects
      ? "Create a subject before adding a course. A course is an offering of a subject, and this school has none yet."
      : null,
    emptyHint: (
      <>
        No courses have been configured for this school. A course is this school&rsquo;s
        offering of a subject — &ldquo;Year 9 Mathematics&rdquo; is a course, and
        &ldquo;Mathematics&rdquo; is the subject it is an offering of.
        <br />
        One course can have several class sections, which is how a year group split into two
        or three classes is expressed.
      </>
    ),
    rowActions: [
      { label: "Class sections", onClick: (row) => setChosen(row) },
    ],
    deleteBlockedBy: (row) => {
      const sections = reports.get(row.id)?.sections ?? row.sections;
      if (sections === 0) return null;
      return (
        <>
          <p>
            {sections === 1
              ? "1 class section delivers it"
              : `${sections} class sections deliver it`}
            .
          </p>
          <p className="muted">
            A class section carries its own teachers, audience, roster and lesson
            occurrences, so deleting the course will not quietly take all of that with it.
            Delete the sections first — each one explicitly — and the course delete is then
            offered.
          </p>
        </>
      );
    },
  };

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Lessons</h2>
        <p>
          What this school teaches to whom, how often, and for how long at a time. Three
          levels, each about a different decision: a <strong>course</strong> is this
          school&rsquo;s offering of a subject, a <strong>class section</strong> is one
          deliverable instance of that course, and a <strong>lesson occurrence</strong> is
          one period — or one double period — that has to find a place in the timetable.
        </p>
        <p className="muted">
          Nothing here is scheduled. These are the requirements a timetable will later be
          built to satisfy, and nothing on this screen claims that any of them can be met —
          only a solver can say that.
        </p>
      </header>

      {subjects.error !== null && (
        <Banner tone="warning">
          This school&rsquo;s subjects could not be loaded, so the subject picker is empty:{" "}
          {subjects.error}
        </Banner>
      )}

      {noSubjects && (
        <EmptyState title="Create a subject before adding a course">
          <p>
            A course is an offering <em>of</em> a subject, so there has to be one to offer.
            This school has no subjects yet.
          </p>
          <p className="muted">
            Subjects belong to this school alone — another school in this campus file keeps
            its own list, and a course of its Mathematics would be a course of that school.
            {openSection !== undefined && (
              <>
                {" "}
                <button
                  type="button"
                  className="linkish"
                  onClick={() => openSection("subjects")}
                >
                  Open Subjects
                </button>
              </>
            )}
          </p>
        </EmptyState>
      )}

      {!noSubjects && (
        <CrudPanel
          spec={spec}
          reloadKey={school.id}
          onChanged={() => {
            // The chosen course's own figures may have moved — its section count, its
            // declared demand — and the panel below renders from the row it was handed.
            // Re-reading it here keeps the two from disagreeing; dropping the selection
            // instead would throw away the user's place in the hierarchy on every edit.
            if (chosen !== null) {
              void listCourses(school.id)
                .then((views) => {
                  const found = views.find((view) => view.course.id === chosen.id);
                  setChosen(found === undefined ? null : toRow(found));
                })
                .catch(() => undefined);
            }
          }}
        />
      )}

      {!noSubjects && (
        <ClassSections
          key={chosen?.id ?? "none"}
          school={school}
          course={chosen}
          openSection={openSection}
        />
      )}
    </div>
  );
}

/**
 * The label a choice list holds for one value, or `null`.
 *
 * Used only to fill the projection fields of the row a create resolves to, which nothing
 * renders — see `CrudSpec.create` above.
 */
function labelOf(choices: readonly Choice[], value: string | null): string | null {
  if (value === null) return null;
  return choices.find((choice) => choice.value === value)?.label ?? null;
}

/**
 * How a course's declaration reads in one cell.
 *
 * `4 × 1` rather than `4`, because the two numbers are the ones a user conflates and the
 * column is where they first meet. The product is the demand.
 */
function describeDeclaration(course: CourseRow): string {
  const periods = course.periodsPerCycle ?? 0;
  const duration = course.defaultDurationPeriods;
  if (duration === 1) return `${periods}`;
  return `${periods} (${duration}-period lessons)`;
}
