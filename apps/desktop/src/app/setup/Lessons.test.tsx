/**
 * Stage C: the Lessons screen — courses, class sections, staffing, audience and demand.
 *
 * ## What is worth testing here
 *
 * The CRUD is `CrudPanel`'s and is already covered. What this screen adds, and what only a
 * test can hold in place, is **the five things it would be worst to get wrong**:
 *
 * 1. **The subject picker is scoped to the selected school.** A course of another school's
 *    Mathematics would be a course of that school, and the backend refuses it — but a
 *    picker that offered it would make that refusal look like a bug.
 * 2. **An unqualified teacher can be assigned, and is warned about.** Both halves, because
 *    either alone is the wrong behaviour: a refusal would be answered by inventing a
 *    qualification, and silence would make the record meaningless. ADR 0028.
 * 3. **The group population and the named roster are never added.** A section taught to a
 *    group of 28 that also names four children has an audience of 28. ADR 0029 §4.
 * 4. **Two numbers make the demand, and the screen says which is which.** "2 periods per
 *    lesson, 3 times a cycle" is six period slots, and the panel states the product before
 *    the save rather than leaving a user to work it out.
 * 5. **Nothing claims a lesson can be timetabled.** The demand summary reports
 *    impossibility only, and says so in those words.
 *
 * Plus the two things every screen in this project has to hold: a revision conflict is
 * reported and not retried, and a stale asynchronous response never lands on the wrong
 * parent row.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  AcademicYear,
  Activity,
  AssignedTeacher,
  AttendingGroup,
  Campus,
  ClassSection,
  ClassSectionDependents,
  ClassSectionView,
  Course,
  CourseDependents,
  CourseView,
  RoomType,
  RosteredStudent,
  School,
  SectionDemand,
  Student,
  StudentGroup,
  StudentInSchool,
  Subject,
  Teacher,
  TeacherCandidate,
  TeacherSchool,
  Term,
  YearLevel,
} from "@evara/domain";

const listSubjects = vi.fn<(schoolId: string) => Promise<Subject[]>>();
const listYearLevels = vi.fn<(schoolId: string) => Promise<YearLevel[]>>();
const listRoomTypes = vi.fn<(schoolId: string) => Promise<RoomType[]>>();
const listAcademicYears = vi.fn<(schoolId: string) => Promise<AcademicYear[]>>();
const listTerms = vi.fn<(yearId: string) => Promise<Term[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();

const listCourses = vi.fn<(schoolId: string) => Promise<CourseView[]>>();
const createCourse = vi.fn<(schoolId: string, input: unknown) => Promise<Course>>();
const updateCourse =
  vi.fn<(schoolId: string, id: string, rev: number, input: unknown) => Promise<Course>>();
const deleteCourse = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const courseDependents = vi.fn<(id: string) => Promise<CourseDependents>>();

const listClassSections =
  vi.fn<(schoolId: string, courseId: string) => Promise<ClassSectionView[]>>();
const createClassSection =
  vi.fn<(schoolId: string, input: unknown) => Promise<ClassSection>>();
const updateClassSection =
  vi.fn<
    (schoolId: string, id: string, rev: number, input: unknown) => Promise<ClassSection>
  >();
const deleteClassSection =
  vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const classSectionDependents = vi.fn<(id: string) => Promise<ClassSectionDependents>>();

const listClassTeachers =
  vi.fn<(schoolId: string, sectionId: string) => Promise<AssignedTeacher[]>>();
const listTeacherCandidates =
  vi.fn<(schoolId: string, sectionId: string) => Promise<TeacherCandidate[]>>();
const addClassTeacher =
  vi.fn<(schoolId: string, sectionId: string, teacherId: string) => Promise<unknown>>();
const removeClassTeacher =
  vi.fn<
    (schoolId: string, sectionId: string, teacherId: string, rev: number) => Promise<void>
  >();

const listClassGroups =
  vi.fn<(schoolId: string, sectionId: string) => Promise<AttendingGroup[]>>();
const listStudentGroups = vi.fn<(schoolId: string) => Promise<StudentGroup[]>>();
const addClassGroup =
  vi.fn<(schoolId: string, sectionId: string, groupId: string) => Promise<unknown>>();
const removeClassGroup =
  vi.fn<
    (schoolId: string, sectionId: string, groupId: string, rev: number) => Promise<void>
  >();

const listClassEnrolments =
  vi.fn<(schoolId: string, sectionId: string) => Promise<RosteredStudent[]>>();
const listSchoolStudents = vi.fn<(schoolId: string) => Promise<StudentInSchool[]>>();
const addClassEnrolment =
  vi.fn<(schoolId: string, sectionId: string, studentId: string) => Promise<unknown>>();
const removeClassEnrolment =
  vi.fn<
    (schoolId: string, sectionId: string, studentId: string, rev: number) => Promise<void>
  >();

const listActivities =
  vi.fn<(schoolId: string, sectionId: string) => Promise<Activity[]>>();
const sectionDemand =
  vi.fn<(schoolId: string, sectionId: string) => Promise<SectionDemand>>();
const addActivities =
  vi.fn<
    (
      schoolId: string,
      sectionId: string,
      name: string | null,
      duration: number,
      count: number,
    ) => Promise<Activity[]>
  >();
const generateActivities =
  vi.fn<(schoolId: string, sectionId: string) => Promise<Activity[]>>();
const updateActivity =
  vi.fn<
    (
      schoolId: string,
      id: string,
      rev: number,
      name: string | null,
      duration: number,
    ) => Promise<Activity>
  >();
const deleteActivity = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();

vi.mock("../../ipc/setup", () => ({
  listSubjects: (schoolId: string) => listSubjects(schoolId),
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  listRoomTypes: (schoolId: string) => listRoomTypes(schoolId),
  listAcademicYears: (schoolId: string) => listAcademicYears(schoolId),
  listTerms: (yearId: string) => listTerms(yearId),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listCourses: (schoolId: string) => listCourses(schoolId),
  createCourse: (schoolId: string, input: unknown) => createCourse(schoolId, input),
  updateCourse: (schoolId: string, id: string, rev: number, input: unknown) =>
    updateCourse(schoolId, id, rev, input),
  deleteCourse: (schoolId: string, id: string, rev: number) =>
    deleteCourse(schoolId, id, rev),
  courseDependents: (id: string) => courseDependents(id),
  listClassSections: (schoolId: string, courseId: string) =>
    listClassSections(schoolId, courseId),
  createClassSection: (schoolId: string, input: unknown) =>
    createClassSection(schoolId, input),
  updateClassSection: (schoolId: string, id: string, rev: number, input: unknown) =>
    updateClassSection(schoolId, id, rev, input),
  deleteClassSection: (schoolId: string, id: string, rev: number) =>
    deleteClassSection(schoolId, id, rev),
  classSectionDependents: (id: string) => classSectionDependents(id),
  listClassTeachers: (schoolId: string, sectionId: string) =>
    listClassTeachers(schoolId, sectionId),
  listTeacherCandidates: (schoolId: string, sectionId: string) =>
    listTeacherCandidates(schoolId, sectionId),
  addClassTeacher: (schoolId: string, sectionId: string, teacherId: string) =>
    addClassTeacher(schoolId, sectionId, teacherId),
  removeClassTeacher: (
    schoolId: string,
    sectionId: string,
    teacherId: string,
    rev: number,
  ) => removeClassTeacher(schoolId, sectionId, teacherId, rev),
  listClassGroups: (schoolId: string, sectionId: string) =>
    listClassGroups(schoolId, sectionId),
  listStudentGroups: (schoolId: string) => listStudentGroups(schoolId),
  addClassGroup: (schoolId: string, sectionId: string, groupId: string) =>
    addClassGroup(schoolId, sectionId, groupId),
  removeClassGroup: (schoolId: string, sectionId: string, groupId: string, rev: number) =>
    removeClassGroup(schoolId, sectionId, groupId, rev),
  listClassEnrolments: (schoolId: string, sectionId: string) =>
    listClassEnrolments(schoolId, sectionId),
  listSchoolStudents: (schoolId: string) => listSchoolStudents(schoolId),
  addClassEnrolment: (schoolId: string, sectionId: string, studentId: string) =>
    addClassEnrolment(schoolId, sectionId, studentId),
  removeClassEnrolment: (
    schoolId: string,
    sectionId: string,
    studentId: string,
    rev: number,
  ) => removeClassEnrolment(schoolId, sectionId, studentId, rev),
  listActivities: (schoolId: string, sectionId: string) =>
    listActivities(schoolId, sectionId),
  sectionDemand: (schoolId: string, sectionId: string) =>
    sectionDemand(schoolId, sectionId),
  addActivities: (
    schoolId: string,
    sectionId: string,
    name: string | null,
    duration: number,
    count: number,
  ) => addActivities(schoolId, sectionId, name, duration, count),
  generateActivities: (schoolId: string, sectionId: string) =>
    generateActivities(schoolId, sectionId),
  updateActivity: (
    schoolId: string,
    id: string,
    rev: number,
    name: string | null,
    duration: number,
  ) => updateActivity(schoolId, id, rev, name, duration),
  deleteActivity: (schoolId: string, id: string, rev: number) =>
    deleteActivity(schoolId, id, rev),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "duplicate"
      ? ((error as { field: string }).field ?? null)
      : null,
}));

const { Lessons } = await import("./Lessons");

/** Not 1: an assertion that a write carried the row's revision must be able to fail. */
const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 4,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function subject(id: string, schoolId: string, name: string): Subject {
  return { id, schoolId, name, code: null, departmentId: null, description: null, ...STAMP };
}

const NORTH_MATHS = subject("s-n-maths", NORTH.id, "Mathematics");
const NORTH_PHYSICS = subject("s-n-physics", NORTH.id, "Physics");
// Same name, different school. Two records, and the picker must never offer this one.
const SOUTH_MATHS = subject("s-s-maths", SOUTH.id, "Mathematics");

function course(id: string, subj: Subject, name: string, periods: number | null): Course {
  return {
    id,
    subjectId: subj.id,
    yearLevelId: null,
    termId: null,
    name,
    code: null,
    description: null,
    periodsPerCycle: periods,
    defaultDurationPeriods: 1,
    requiredRoomTypeId: null,
    ...STAMP,
  };
}

function view(c: Course, subj: Subject, sections: number): CourseView {
  return {
    course: c,
    subject: subj,
    yearLevelName: null,
    termName: null,
    roomTypeName: null,
    sections,
  };
}

const MATHS_9 = course("c-maths-9", NORTH_MATHS, "Year 9 Mathematics", 4);
const PHYSICS_10 = course("c-physics-10", NORTH_PHYSICS, "Year 10 Physics", 3);

function sectionView(
  id: string,
  courseId: string,
  name: string,
  over: Partial<ClassSectionView> = {},
): ClassSectionView {
  return {
    section: { id, courseId, name, code: null, campusId: null, ...STAMP },
    campusName: null,
    teachers: 0,
    groups: 0,
    groupPopulation: 0,
    enrolledStudents: 0,
    demand: { occurrences: 0, periodsPerCycle: 0, longestOccurrence: 0 },
    declared: { kind: "notDeclared" },
    findings: [],
    ...over,
  };
}

const SECTION_A = sectionView("sec-a", MATHS_9.id, "A", {
  teachers: 1,
  groups: 1,
  groupPopulation: 28,
  enrolledStudents: 4,
  demand: { occurrences: 4, periodsPerCycle: 4, longestOccurrence: 1 },
  declared: { kind: "matches" },
});

function teacher(id: string, family: string, active = true): Teacher {
  return {
    id,
    schoolId: NORTH.id,
    code: null,
    givenName: "Ada",
    familyName: family,
    email: null,
    homeCampusId: null,
    ftePercent: 100,
    maxPeriodsPerDay: null,
    maxPeriodsPerCycle: null,
    maxConsecutivePeriods: null,
    notes: null,
    isActive: active,
    ...STAMP,
  };
}

function membership(teacherId: string, active = true): TeacherSchool {
  return { id: `ts-${teacherId}`, teacherId, schoolId: NORTH.id, isActive: active, ...STAMP };
}

function candidate(
  t: Teacher,
  over: Partial<TeacherCandidate["eligibility"]> = {},
  assigned = false,
): TeacherCandidate {
  return {
    teacher: t,
    membership: membership(t.id, over.employedAndActive ?? true),
    eligibility: {
      employedAndActive: true,
      campusEligible: true,
      qualified: true,
      ...over,
    },
    assigned,
  };
}

const QUALIFIED = teacher("t-qualified", "Qualified");
const UNQUALIFIED = teacher("t-unqualified", "Unqualified");
const ELSEWHERE = teacher("t-elsewhere", "JuniorOnly");

function group(id: string, name: string, size: number | null): StudentGroup {
  return {
    id,
    schoolId: NORTH.id,
    name,
    code: null,
    kind: "CLASS",
    yearLevelId: null,
    parentGroupId: null,
    size,
    campusId: null,
    ...STAMP,
  };
}

const NINE_A = group("g-9a", "9A", 28);

function student(id: string, family: string): Student {
  return {
    id,
    schoolId: NORTH.id,
    code: null,
    givenName: "Bea",
    familyName: family,
    email: null,
    yearLevelId: null,
    campusId: null,
    isActive: true,
    ...STAMP,
  };
}

const NAMED = student("st-named", "Named");

function activity(id: string, splitIndex: number, duration: number): Activity {
  return {
    id,
    classSectionId: SECTION_A.section.id,
    name: null,
    durationPeriods: duration,
    splitIndex,
    ...STAMP,
  };
}

/** A five-day week of six teaching periods, never more than three in a row. */
const ORDINARY_DEMAND: SectionDemand = {
  demand: { occurrences: 4, periodsPerCycle: 4, longestOccurrence: 1 },
  declared: { kind: "matches" },
  capacity: { cycleDays: 5, teachingPeriodsPerDay: 6, longestTeachingRun: 3 },
  findings: [],
};

const NO_COURSE_DEPENDENTS: CourseDependents = { total: 0, sections: 0 };
const NO_SECTION_DEPENDENTS: ClassSectionDependents = {
  total: 0,
  teachers: 0,
  groups: 0,
  enrolledStudents: 0,
  activities: 0,
};

beforeEach(() => {
  vi.clearAllMocks();

  listSubjects.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id ? [NORTH_MATHS, NORTH_PHYSICS] : [SOUTH_MATHS],
    ),
  );
  listYearLevels.mockResolvedValue([]);
  listRoomTypes.mockResolvedValue([]);
  listAcademicYears.mockResolvedValue([]);
  listTerms.mockResolvedValue([]);
  listCampuses.mockResolvedValue([]);

  listCourses.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id
        ? [view(MATHS_9, NORTH_MATHS, 1), view(PHYSICS_10, NORTH_PHYSICS, 0)]
        : [],
    ),
  );
  createCourse.mockResolvedValue(MATHS_9);
  updateCourse.mockResolvedValue(MATHS_9);
  deleteCourse.mockResolvedValue(undefined);
  courseDependents.mockResolvedValue(NO_COURSE_DEPENDENTS);

  listClassSections.mockResolvedValue([SECTION_A]);
  createClassSection.mockResolvedValue(SECTION_A.section);
  updateClassSection.mockResolvedValue(SECTION_A.section);
  deleteClassSection.mockResolvedValue(undefined);
  classSectionDependents.mockResolvedValue(NO_SECTION_DEPENDENTS);

  listClassTeachers.mockResolvedValue([]);
  listTeacherCandidates.mockResolvedValue([
    candidate(QUALIFIED),
    candidate(UNQUALIFIED, { qualified: false }),
  ]);
  addClassTeacher.mockResolvedValue({});
  removeClassTeacher.mockResolvedValue(undefined);

  listClassGroups.mockResolvedValue([]);
  listStudentGroups.mockResolvedValue([NINE_A]);
  addClassGroup.mockResolvedValue({});
  removeClassGroup.mockResolvedValue(undefined);

  listClassEnrolments.mockResolvedValue([]);
  listSchoolStudents.mockResolvedValue([]);
  addClassEnrolment.mockResolvedValue({});
  removeClassEnrolment.mockResolvedValue(undefined);

  listActivities.mockResolvedValue([]);
  sectionDemand.mockResolvedValue(ORDINARY_DEMAND);
  addActivities.mockResolvedValue([activity("a-1", 1, 1)]);
  generateActivities.mockResolvedValue([activity("a-1", 1, 1)]);
  updateActivity.mockResolvedValue(activity("a-1", 1, 2));
  deleteActivity.mockResolvedValue(undefined);
});

/** Opens the course's class sections, then the chosen section's detail panels. */
async function openSectionDetail(user: ReturnType<typeof userEvent.setup>) {
  render(<Lessons school={NORTH} />);
  await screen.findByText("Year 9 Mathematics");
  await user.click(
    within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
  );
  await screen.findByRole("heading", { name: /Class sections of Year 9 Mathematics/i });
  await user.click(within(rowIn(SECTIONS, "A")).getByRole("button", { name: /^Open / }));
  await screen.findByRole("heading", { name: /^Teachers$/ });
}

/**
 * The row of one named table whose first cell reads `label`.
 *
 * Scoped to a table, because the screen shows several at once — courses, sections,
 * teachers, groups, roster, occurrences — and "A" and "4" each occur in more than one.
 * Every table carries a visually-hidden caption, which is what names it here.
 */
function rowIn(caption: RegExp, label: string): HTMLElement {
  const table = screen
    .getAllByRole("table")
    .find((candidate) => caption.test(candidate.querySelector("caption")?.textContent ?? ""));
  if (!table) throw new Error(`no table whose caption matches ${String(caption)}`);
  const row = within(table)
    .getAllByRole("row")
    .find((candidate) => candidate.querySelector("td")?.textContent === label);
  if (!row) throw new Error(`no row in ${String(caption)} whose first cell reads "${label}"`);
  return row;
}

/**
 * The panel whose heading matches `name`.
 *
 * The section detail shows four at once and each has an Add or a Remove, so an unscoped
 * `getByRole("button", { name: /^Add$/ })` finds two and fails. Scoping by panel is also
 * how a reader of the test knows which panel an assertion is about.
 */
function panel(name: RegExp): HTMLElement {
  const heading = screen.getByRole("heading", { name });
  const found = heading.closest(".crudPanel");
  if (!found) throw new Error(`no panel around the heading ${String(name)}`);
  return found as HTMLElement;
}

/** The open confirmation dialogue, whose buttons share names with the rows'. */
function dialogue(): HTMLElement {
  return screen.getByRole("dialog");
}

/** Every cell of one row, in order, as the user reads them. */
function cellsOf(row: HTMLElement): Array<string | null> {
  return within(row)
    .getAllByRole("cell")
    .map((cell) => cell.textContent);
}

/** The table captions, as `rowIn` matches them. */
const COURSES = /^Courses$/;
const SECTIONS = /^Class sections of/;
const TEACHERS = /^Teachers assigned/;
const GROUPS = /^Student groups attending/;
const ROSTER = /^Students named/;
const OCCURRENCES = /^Lesson occurrences of/;

// ============================================================================ courses

describe("the Courses panel", () => {
  it("reads the selected school's courses and offers only its subjects", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);

    await screen.findByText("Year 9 Mathematics");
    expect(listCourses).toHaveBeenCalledWith(NORTH.id);
    expect(listSubjects).toHaveBeenCalledWith(NORTH.id);

    await user.click(screen.getByRole("button", { name: /^Add course$/ }));

    // The whole multi-school point of the picker: Southmoor's Mathematics is a different
    // subject, and a course of it would be a course of Southmoor.
    await user.click(screen.getByLabelText(/Subject/));
    const options = within(screen.getByRole("listbox")).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual(["Mathematics", "Physics"]);
  });

  it("re-reads rather than filters when the school changes", async () => {
    const { rerender } = render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    rerender(<Lessons school={SOUTH} />);
    await waitFor(() => expect(listCourses).toHaveBeenCalledWith(SOUTH.id));
    await waitFor(() => expect(listSubjects).toHaveBeenCalledWith(SOUTH.id));
  });

  it("creates a course under the selected school, with the subject it was given", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    await user.click(screen.getByRole("button", { name: /^Add course$/ }));
    await user.click(screen.getByLabelText(/Subject/));
    await user.click(screen.getByRole("option", { name: "Mathematics" }));
    await user.type(screen.getByLabelText(/Course name/), "Year 8 Mathematics");
    await user.clear(screen.getByLabelText(/Periods per cycle/));
    await user.type(screen.getByLabelText(/Periods per cycle/), "5");
    await user.click(screen.getByRole("button", { name: /^Add course$/ }));

    await waitFor(() => expect(createCourse).toHaveBeenCalled());
    const [schoolId, input] = createCourse.mock.calls[0] as [string, Record<string, unknown>];
    expect(schoolId).toBe(NORTH.id);
    expect(input["subjectId"]).toBe(NORTH_MATHS.id);
    expect(input["name"]).toBe("Year 8 Mathematics");
    expect(input["periodsPerCycle"]).toBe(5);
    // No `schoolId` on the input: the subject is the only truthful place for one.
    expect(input).not.toHaveProperty("schoolId");
  });

  it("will not save a course with no name", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    await user.click(screen.getByRole("button", { name: /^Add course$/ }));
    await user.click(screen.getByRole("button", { name: /^Add course$/ }));

    expect(await screen.findByText(/Course name is required/)).toBeTruthy();
    expect(createCourse).not.toHaveBeenCalled();
  });

  it("sends the row's own revision on an edit, and keeps the subject fixed", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    await user.click(within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Edit / }));

    // A course is an offering *of* a subject and `update_course` does not take one, so
    // offering a change here would promise something the backend ignores.
    expect(screen.getByText(/cannot be changed/)).toBeTruthy();

    await user.clear(screen.getByLabelText(/Course name/));
    await user.type(screen.getByLabelText(/Course name/), "Year 9 Core Mathematics");
    await user.click(screen.getByRole("button", { name: /Save changes/ }));

    await waitFor(() => expect(updateCourse).toHaveBeenCalled());
    expect(updateCourse.mock.calls[0]?.[0]).toBe(NORTH.id);
    expect(updateCourse.mock.calls[0]?.[1]).toBe(MATHS_9.id);
    expect(updateCourse.mock.calls[0]?.[2]).toBe(STAMP.rev);
  });

  it("explains a blocked delete before offering it, with the section count", async () => {
    const user = userEvent.setup();
    courseDependents.mockImplementation((id) =>
      Promise.resolve(
        id === MATHS_9.id ? { total: 2, sections: 2 } : NO_COURSE_DEPENDENTS,
      ),
    );
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    await waitFor(() => expect(courseDependents).toHaveBeenCalledWith(MATHS_9.id));

    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Delete / }),
    );

    expect(await screen.findByText(/2 class sections deliver it/)).toBeTruthy();
    // An explanation, not a question: no destructive button at all.
    expect(screen.queryByRole("button", { name: /^Delete course$/ })).toBeNull();
    expect(deleteCourse).not.toHaveBeenCalled();
  });

  it("reports a revision conflict without retrying and without discarding the edit", async () => {
    const user = userEvent.setup();
    updateCourse.mockRejectedValue({
      kind: "conflict",
      message: "Somebody else changed this course. Nothing was saved.",
    });
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    await user.click(within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Edit / }));
    await user.clear(screen.getByLabelText(/Course name/));
    await user.type(screen.getByLabelText(/Course name/), "Renamed");
    await user.click(screen.getByRole("button", { name: /Save changes/ }));

    expect(await screen.findByText(/Nothing was saved/)).toBeTruthy();
    // Not retried, and the typed value is still there to re-save or copy out.
    expect(updateCourse).toHaveBeenCalledTimes(1);
    expect((screen.getByLabelText(/Course name/) as HTMLInputElement).value).toBe("Renamed");
  });

  it("asks for a subject before a course when the school has none", async () => {
    listSubjects.mockResolvedValue([]);
    listCourses.mockResolvedValue([]);
    render(<Lessons school={NORTH} />);

    expect(
      await screen.findByText(/Create a subject before adding a course/),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Add course$/ })).toBeNull();
  });
});

// ====================================================================== class sections

describe("the Class sections panel", () => {
  it("says nothing is chosen until a course is", async () => {
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    expect(screen.getByText(/No course chosen/)).toBeTruthy();
    expect(listClassSections).not.toHaveBeenCalled();
  });

  it("reads the chosen course's sections", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");

    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );

    await waitFor(() =>
      expect(listClassSections).toHaveBeenCalledWith(NORTH.id, MATHS_9.id),
    );
    expect(
      await screen.findByRole("heading", { name: /Class sections of Year 9 Mathematics/i }),
    ).toBeTruthy();
  });

  it("shows the group audience and the named roster as separate figures", async () => {
    // ADR 0029 §4. 28 in one group, 4 named — and never 32 anywhere.
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );
    await screen.findByRole("heading", { name: /Class sections of/i });

    // By position, because "4" is also the per-cycle figure in the same row and
    // `getByText` cannot tell the two apart. Section, code, campus, teachers, audience,
    // named, per cycle.
    const cells = cellsOf(rowIn(SECTIONS, "A"));
    expect(cells[4]).toBe("28 in 1");
    expect(cells[5]).toBe("4");
    expect(cells.join("|")).not.toContain("32");
  });

  it("creates a section of the chosen course", async () => {
    const user = userEvent.setup();
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );
    await screen.findByRole("heading", { name: /Class sections of/i });

    await user.click(screen.getByRole("button", { name: /^Add class section$/ }));
    await user.type(screen.getByLabelText(/Section name/), "B");
    await user.click(screen.getByRole("button", { name: /^Add class section$/ }));

    await waitFor(() => expect(createClassSection).toHaveBeenCalled());
    const [schoolId, input] = createClassSection.mock.calls[0] as [
      string,
      Record<string, unknown>,
    ];
    expect(schoolId).toBe(NORTH.id);
    expect(input["courseId"]).toBe(MATHS_9.id);
    expect(input["name"]).toBe("B");
  });

  it("says what a section delete will take with it", async () => {
    const user = userEvent.setup();
    classSectionDependents.mockResolvedValue({
      total: 7,
      teachers: 1,
      groups: 1,
      enrolledStudents: 1,
      activities: 4,
    });
    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );
    await screen.findByRole("heading", { name: /Class sections of/i });
    await waitFor(() =>
      expect(classSectionDependents).toHaveBeenCalledWith(SECTION_A.section.id),
    );

    await user.click(within(rowIn(SECTIONS, "A")).getByRole("button", { name: /^Delete / }));

    // The counts explain; unlike a course they do not block, because a section owns them.
    expect(await screen.findByText(/1 teacher assignment/)).toBeTruthy();
    expect(screen.getByText(/4 lesson occurrences/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Delete class section$/ })).toBeTruthy();
  });

  it("does not let a slow response for one course land under another", async () => {
    // The stale-response guarantee `useList` and `CrudPanel` both make. Switching courses
    // faster than a round trip must not show the previous course's sections.
    const user = userEvent.setup();
    let releaseFirst: (rows: ClassSectionView[]) => void = () => undefined;
    listClassSections.mockImplementation((_school, courseId) =>
      courseId === MATHS_9.id
        ? new Promise((resolve) => {
            releaseFirst = resolve;
          })
        : Promise.resolve([sectionView("sec-p", PHYSICS_10.id, "Physics Lab")]),
    );

    render(<Lessons school={NORTH} />);
    await screen.findByText("Year 9 Mathematics");
    await user.click(
      within(rowIn(COURSES, "Year 9 Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );
    await user.click(
      within(rowIn(COURSES, "Year 10 Physics")).getByRole("button", { name: /^Class sections/ }),
    );
    await screen.findByText("Physics Lab");

    // The first course's answer arrives last, and must be ignored.
    releaseFirst([sectionView("sec-a", MATHS_9.id, "A")]);
    await waitFor(() => expect(screen.getByText("Physics Lab")).toBeTruthy());
    expect(screen.queryByRole("cell", { name: "A" })).toBeNull();
  });
});

// =========================================================================== teachers

describe("the Teachers panel", () => {
  it("offers this school's members and explains the ones it will not", async () => {
    const user = userEvent.setup();
    listTeacherCandidates.mockResolvedValue([
      candidate(QUALIFIED),
      candidate(ELSEWHERE, { campusEligible: false }),
      candidate(teacher("t-left", "Left"), { employedAndActive: false }),
    ]);
    await openSectionDetail(user);

    expect(
      within(screen.getByLabelText(/Teacher to assign/)).getByRole("option", {
        name: /Qualified/,
      }),
    ).toBeTruthy();

    // Refused, and shown with the reason rather than silently absent — ADR 0023 §5.
    expect(screen.getByText(/not eligible at this section's campus/)).toBeTruthy();
    expect(screen.getByText(/no longer teaching at this school/)).toBeTruthy();
  });

  it("warns about a missing qualification before the save, and still saves", async () => {
    // ADR 0028, both halves in one test because either alone is the wrong behaviour.
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.selectOptions(screen.getByLabelText(/Teacher to assign/), UNQUALIFIED.id);

    expect(
      await screen.findByText(/does not have a recorded qualification for Mathematics/),
    ).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /^Assign$/ }));

    await waitFor(() => expect(addClassTeacher).toHaveBeenCalled());
    expect(addClassTeacher).toHaveBeenCalledWith(
      NORTH.id,
      SECTION_A.section.id,
      UNQUALIFIED.id,
    );
  });

  it("does not warn about a teacher who is qualified", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.selectOptions(screen.getByLabelText(/Teacher to assign/), QUALIFIED.id);

    expect(screen.queryByText(/does not have a recorded qualification/)).toBeNull();
  });

  it("marks an assigned teacher with no recorded qualification", async () => {
    const user = userEvent.setup();
    listClassTeachers.mockResolvedValue([
      {
        assignment: {
          id: "ct-1",
          classSectionId: SECTION_A.section.id,
          teacherId: UNQUALIFIED.id,
          ...STAMP,
        },
        teacher: UNQUALIFIED,
        membership: membership(UNQUALIFIED.id),
        qualified: false,
      },
    ]);
    await openSectionDetail(user);

    const row = rowIn(TEACHERS, "Unqualified");
    expect(within(row).getByText("Not recorded")).toBeTruthy();
    expect(within(row).getByText("Currently teaching")).toBeTruthy();
  });

  it("removes an assignment at its own revision, after confirming", async () => {
    const user = userEvent.setup();
    listClassTeachers.mockResolvedValue([
      {
        assignment: {
          id: "ct-1",
          classSectionId: SECTION_A.section.id,
          teacherId: QUALIFIED.id,
          ...STAMP,
        },
        teacher: QUALIFIED,
        membership: membership(QUALIFIED.id),
        qualified: true,
      },
    ]);
    await openSectionDetail(user);

    await user.click(within(rowIn(TEACHERS, "Qualified")).getByRole("button", { name: /^Remove$/ }));
    expect(await screen.findByText(/every qualification they hold are untouched/)).toBeTruthy();
    await user.click(within(dialogue()).getByRole("button", { name: /^Remove$/ }));

    await waitFor(() => expect(removeClassTeacher).toHaveBeenCalled());
    expect(removeClassTeacher).toHaveBeenCalledWith(
      NORTH.id,
      SECTION_A.section.id,
      QUALIFIED.id,
      STAMP.rev,
    );
  });
});

// ===================================================================== groups and roster

describe("the Student groups panel", () => {
  it("adds one of this school's groups and reports the audience it contributes", async () => {
    const user = userEvent.setup();
    listClassGroups.mockResolvedValue([
      {
        association: {
          id: "cg-1",
          classSectionId: SECTION_A.section.id,
          studentGroupId: NINE_A.id,
          ...STAMP,
        },
        group: NINE_A,
        namedMembers: 4,
        population: 28,
      },
    ]);
    await openSectionDetail(user);

    expect(listStudentGroups).toHaveBeenCalledWith(NORTH.id);
    // Group, code, kind, declared size, members listed, counts as. The declared size is
    // what counts; the listed membership is shown beside it and never added to it.
    const cells = cellsOf(rowIn(GROUPS, "9A"));
    expect(cells[3]).toBe("28");
    expect(cells[4]).toBe("4");
    expect(cells[5]).toBe("28");
    expect(screen.getByText(/audience is/)).toBeTruthy();
  });

  it("adds a group that is not already attending", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.selectOptions(screen.getByLabelText(/Student group to add/), NINE_A.id);
    await user.click(within(panel(/^Student groups$/)).getByRole("button", { name: /^Add$/ }));

    await waitFor(() => expect(addClassGroup).toHaveBeenCalled());
    expect(addClassGroup).toHaveBeenCalledWith(NORTH.id, SECTION_A.section.id, NINE_A.id);
  });

  it("says a group-only section is complete rather than unfinished", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    // The Named students panel, with nothing in it.
    expect(screen.getByText(/No individuals are named/)).toBeTruthy();
    expect(screen.getByText(/most schools will never fill this in/)).toBeTruthy();
  });
});

describe("the Named students panel", () => {
  it("offers students through their enrolment at this school", async () => {
    const user = userEvent.setup();
    listSchoolStudents.mockResolvedValue([
      {
        student: NAMED,
        enrolment: {
          id: "sse-1",
          studentId: NAMED.id,
          schoolId: NORTH.id,
          admissionNo: null,
          yearLevelId: null,
          campusId: null,
          status: "ENROLLED",
          enrolledOn: null,
          leftOn: null,
          ...STAMP,
        },
      },
    ]);
    await openSectionDetail(user);

    // `listSchoolStudents`, not `listStudents`: a project-wide identity is not eligibility
    // to sit in this school's class.
    expect(listSchoolStudents).toHaveBeenCalledWith(NORTH.id);

    await user.selectOptions(screen.getByLabelText(/Student to add to the roster/), NAMED.id);
    await user.click(within(panel(/^Named students$/)).getByRole("button", { name: /^Add$/ }));

    await waitFor(() => expect(addClassEnrolment).toHaveBeenCalled());
    expect(addClassEnrolment).toHaveBeenCalledWith(
      NORTH.id,
      SECTION_A.section.id,
      NAMED.id,
    );
  });

  it("flags a named student no attending group covers", async () => {
    const user = userEvent.setup();
    listClassEnrolments.mockResolvedValue([
      {
        enrolment: {
          id: "ce-1",
          classSectionId: SECTION_A.section.id,
          studentId: NAMED.id,
          ...STAMP,
        },
        student: NAMED,
        coveredByGroup: false,
      },
    ]);
    await openSectionDetail(user);

    expect(within(rowIn(ROSTER, "Named")).getByText("No")).toBeTruthy();
    expect(
      screen.getByText(/is not in any group attending this section/),
    ).toBeTruthy();
  });
});

// ======================================================================== occurrences

describe("the Lesson occurrences panel", () => {
  it("states the product of the two numbers before the save", async () => {
    // The figures a user conflates: duration 2, three times a cycle, is six period slots.
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.click(
      within(panel(/^Lesson occurrences$/).querySelector(".panelHead") as HTMLElement).getByRole(
        "button",
        { name: /^Add lessons$/ },
      ),
    );
    const duration = screen.getByLabelText(/Periods per lesson/);
    await user.clear(duration);
    await user.type(duration, "2");
    const count = screen.getByLabelText(/Times per cycle/);
    await user.clear(count);
    await user.type(count, "3");

    expect(
      await screen.findByText(/3 lessons of 2 consecutive periods each — 6 periods a cycle/),
    ).toBeTruthy();
  });

  it("writes one row per occurrence", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.click(
      within(panel(/^Lesson occurrences$/).querySelector(".panelHead") as HTMLElement).getByRole(
        "button",
        { name: /^Add lessons$/ },
      ),
    );
    const count = screen.getByLabelText(/Times per cycle/);
    await user.clear(count);
    await user.type(count, "4");
    await user.click(
      within(screen.getByRole("group")).getByRole("button", { name: /^Add lessons$/ }),
    );

    await waitFor(() => expect(addActivities).toHaveBeenCalled());
    expect(addActivities).toHaveBeenCalledWith(
      NORTH.id,
      SECTION_A.section.id,
      null,
      1,
      4,
    );
  });

  it("refuses a duration or a count below one", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.click(
      within(panel(/^Lesson occurrences$/).querySelector(".panelHead") as HTMLElement).getByRole(
        "button",
        { name: /^Add lessons$/ },
      ),
    );
    const duration = screen.getByLabelText(/Periods per lesson/);
    await user.clear(duration);
    await user.type(duration, "0");
    await user.click(
      within(screen.getByRole("group")).getByRole("button", { name: /^Add lessons$/ }),
    );

    expect(
      await screen.findByText(/Periods per lesson cannot be less than 1/),
    ).toBeTruthy();
    expect(addActivities).not.toHaveBeenCalled();
  });

  it("numbers the rows it displays rather than showing the stored positions", async () => {
    // `splitIndex` is never renumbered, so deleting the second of four leaves 1, 3, 4 —
    // which the user must not be shown as a gap.
    const user = userEvent.setup();
    listActivities.mockResolvedValue([
      activity("a-1", 1, 1),
      activity("a-3", 3, 1),
      activity("a-4", 4, 2),
    ]);
    await openSectionDetail(user);

    const positions = screen
      .getAllByRole("table")
      .filter((candidate) =>
        OCCURRENCES.test(candidate.querySelector("caption")?.textContent ?? ""),
      )
      .flatMap((table) => within(table).getAllByRole("row").slice(1))
      .map((row) => row.querySelector("td")?.textContent);
    expect(positions).toEqual(["1", "2", "3"]);
  });

  it("offers to generate the course's declared lessons when there are none", async () => {
    const user = userEvent.setup();
    await openSectionDetail(user);

    await user.click(screen.getByRole("button", { name: /Create those lessons/ }));

    await waitFor(() => expect(generateActivities).toHaveBeenCalled());
    expect(generateActivities).toHaveBeenCalledWith(NORTH.id, SECTION_A.section.id);
  });

  it("says nothing can be generated when the course declares no demand", async () => {
    const user = userEvent.setup();
    listCourses.mockResolvedValue([
      view(course("c-open", NORTH_MATHS, "Open Mathematics", null), NORTH_MATHS, 1),
    ]);
    render(<Lessons school={NORTH} />);
    await screen.findByText("Open Mathematics");
    await user.click(
      within(rowIn(COURSES, "Open Mathematics")).getByRole("button", { name: /^Class sections/ }),
    );
    await screen.findByRole("heading", { name: /Class sections of/i });
    await user.click(within(rowIn(SECTIONS, "A")).getByRole("button", { name: /^Open / }));
    await screen.findByRole("heading", { name: /^Lesson occurrences$/ });

    expect(screen.getByText(/does not say how many periods a cycle it needs/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Create those lessons/ })).toBeNull();
  });

  it("deletes one occurrence at its own revision", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 2)]);
    await openSectionDetail(user);

    await user.click(within(rowIn(OCCURRENCES, "1")).getByRole("button", { name: /^Delete$/ }));
    expect(await screen.findByText(/2 fewer periods a cycle/)).toBeTruthy();
    await user.click(screen.getByRole("button", { name: /^Delete lesson$/ }));

    await waitFor(() => expect(deleteActivity).toHaveBeenCalled());
    expect(deleteActivity).toHaveBeenCalledWith(NORTH.id, "a-1", STAMP.rev);
  });

  it("reports a conflict on an occurrence without retrying", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 1)]);
    deleteActivity.mockRejectedValue({
      kind: "conflict",
      message: "Somebody else changed this lesson. Nothing was saved.",
    });
    await openSectionDetail(user);

    await user.click(within(rowIn(OCCURRENCES, "1")).getByRole("button", { name: /^Delete$/ }));
    await user.click(screen.getByRole("button", { name: /^Delete lesson$/ }));

    expect(await screen.findByText(/Nothing was saved/)).toBeTruthy();
    expect(deleteActivity).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /^Reload$/ })).toBeTruthy();
  });
});

// ====================================================================== demand summary

describe("the demand summary", () => {
  it("states the demand and refuses to promise it can be scheduled", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 1)]);
    await openSectionDetail(user);

    expect(await screen.findByText(/4 periods a cycle/)).toBeTruthy();
    // The restraint that matters: no finding is not a promise.
    expect(screen.getByText(/not a promise that it can be timetabled/)).toBeTruthy();
  });

  it("explains an occurrence longer than any unbroken run of teaching periods", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 4)]);
    sectionDemand.mockResolvedValue({
      demand: { occurrences: 1, periodsPerCycle: 4, longestOccurrence: 4 },
      declared: { kind: "short", declared: 4, shortfall: 0 },
      capacity: { cycleDays: 5, teachingPeriodsPerDay: 6, longestTeachingRun: 3 },
      findings: [{ kind: "occurrenceLongerThanAnyRun", needed: 4, longestRun: 3 }],
    });
    await openSectionDetail(user);

    expect(await screen.findByText(/No timetable can place it/)).toBeTruthy();
    expect(screen.getByText(/more than 3 teaching periods/)).toBeTruthy();
  });

  it("explains a total that exceeds every slot in the cycle", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 1)]);
    sectionDemand.mockResolvedValue({
      demand: { occurrences: 31, periodsPerCycle: 31, longestOccurrence: 1 },
      declared: { kind: "notDeclared" },
      capacity: { cycleDays: 5, teachingPeriodsPerDay: 6, longestTeachingRun: 3 },
      findings: [{ kind: "totalExceedsCycle", demanded: 31, available: 30 }],
    });
    await openSectionDetail(user);

    expect(await screen.findByText(/No timetable can place them all/)).toBeTruthy();
  });

  it("judges nothing before the school has a cycle and a bell schedule", async () => {
    // Lessons is section 10 and the time model is 14–17, so this is the expected order
    // through setup rather than a mistake to report.
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 4)]);
    sectionDemand.mockResolvedValue({
      demand: { occurrences: 1, periodsPerCycle: 4, longestOccurrence: 4 },
      declared: { kind: "notDeclared" },
      capacity: { cycleDays: 0, teachingPeriodsPerDay: 0, longestTeachingRun: 0 },
      findings: [],
    });
    await openSectionDetail(user);

    expect(
      await screen.findByText(/no cycle or no bell schedule yet/),
    ).toBeTruthy();
    expect(screen.queryByText(/No timetable can place/)).toBeNull();
  });

  it("reports a difference from the course's declaration as advice", async () => {
    const user = userEvent.setup();
    listActivities.mockResolvedValue([activity("a-1", 1, 1)]);
    sectionDemand.mockResolvedValue({
      demand: { occurrences: 3, periodsPerCycle: 3, longestOccurrence: 1 },
      declared: { kind: "short", declared: 4, shortfall: 1 },
      capacity: { cycleDays: 5, teachingPeriodsPerDay: 6, longestTeachingRun: 3 },
      findings: [],
    });
    await openSectionDetail(user);

    expect(await screen.findByText(/so this section is 1 short/)).toBeTruthy();
    expect(screen.getByText(/nothing here refuses it/)).toBeTruthy();
  });
});
