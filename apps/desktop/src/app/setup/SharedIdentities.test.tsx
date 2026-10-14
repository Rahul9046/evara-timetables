/**
 * Stage 0B: a shared person is one person on both schools' screens.
 *
 * The repository tests in `evara-db` own the model — one teacher row, one student row,
 * relationships per school. This file owns the three things only the interface can get
 * wrong, and which no backend test can catch:
 *
 * - a teacher belonging to two schools appears **once** in each school's list, as the same
 *   record, rather than twice in one or not at all in the other;
 * - the **school-specific** fact is what the row shows — this school's employment, this
 *   school's placement — never the project-level flag or the home school's year level
 *   standing in for it;
 * - an operation the backend is going to refuse is **disclosed before** it is attempted,
 *   because a refusal arriving after the user pressed Save explains nothing.
 *
 * The second is the one worth having. A visiting student's record carries the year level
 * of the school that created them, and rendering that under another school's name produces
 * a screen that is wrong in the most convincing possible way: populated, plausible, and
 * about a different institution's Grade 8.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";

import userEvent from "@testing-library/user-event";

import type {
  Campus,
  School,
  Student,
  StudentEnrolment,
  StudentInSchool,
  Teacher,
  TeacherInSchool,
  YearLevel,
} from "@evara/domain";

const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const listSchoolTeachers = vi.fn<(schoolId: string) => Promise<TeacherInSchool[]>>();
const listStudents = vi.fn<(schoolId: string) => Promise<Student[]>>();
const listSchoolStudents = vi.fn<(schoolId: string) => Promise<StudentInSchool[]>>();
const listYearLevels = vi.fn<(schoolId: string) => Promise<YearLevel[]>>();
const updateTeacher = vi.fn<(id: string, input: unknown) => Promise<Teacher>>();
const listEnrolmentHistory = vi.fn<(studentId: string) => Promise<StudentEnrolment[]>>();

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  listSchoolTeachers: (schoolId: string) => listSchoolTeachers(schoolId),
  createTeacher: vi.fn(() => Promise.resolve({})),
  updateTeacher: (id: string, input: unknown) => updateTeacher(id, input),
  deleteTeacher: vi.fn(() => Promise.resolve()),
  listStudents: (schoolId: string) => listStudents(schoolId),
  listSchoolStudents: (schoolId: string) => listSchoolStudents(schoolId),
  createStudent: vi.fn(() => Promise.resolve({})),
  updateStudent: vi.fn(() => Promise.resolve({})),
  deleteStudent: vi.fn(() => Promise.resolve()),
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  createYearLevel: vi.fn(() => Promise.resolve({})),
  updateYearLevel: vi.fn(() => Promise.resolve({})),
  deleteYearLevel: vi.fn(() => Promise.resolve()),
  listGroupsForStudent: vi.fn(() => Promise.resolve([])),
  listCampuses: vi.fn<(schoolId: string) => Promise<Campus[]>>(() => Promise.resolve([])),
  // The Stage 0C relationship surface. The assertions here are about what the two lists
  // *show* and what they refuse, so these are stubs — `MultiSchoolManagement.test.tsx`
  // covers the writes.
  addTeacherToSchool: vi.fn(() => Promise.resolve({})),
  listTeacherMemberships: vi.fn(() => Promise.resolve([])),
  setTeacherMembershipActive: vi.fn(() => Promise.resolve({})),
  removeTeacherFromSchool: vi.fn(() => Promise.resolve()),
  listTeacherCampuses: vi.fn(() => Promise.resolve([])),
  assignTeacherCampus: vi.fn(() => Promise.resolve({})),
  removeTeacherCampus: vi.fn(() => Promise.resolve()),
  enrolStudent: vi.fn(() => Promise.resolve({})),
  listEnrolmentHistory: (studentId: string) => listEnrolmentHistory(studentId),
  updateEnrolment: vi.fn(() => Promise.resolve({})),
  leaveSchool: vi.fn(() => Promise.resolve({})),
  transferStudent: vi.fn(() => Promise.resolve({})),
  listAvailability: vi.fn(() => Promise.resolve([])),
  countAvailability: vi.fn(() => Promise.resolve(0)),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  listCycles: vi.fn(() => Promise.resolve([])),
  listPeriodStructures: vi.fn(() => Promise.resolve([])),
  gridPreview: vi.fn(() => Promise.resolve(null)),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: () => null,
}));

const { Teachers } = await import("./Teachers");
const { StudentsAndYearLevels } = await import("./StudentsAndYearLevels");
type SetupContext = import("./SchoolSetup").SetupContext;

const NORTH = { id: "school-north", name: "Northgate Secondary" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College" } as School;

/**
 * What the setup shell hands a section.
 *
 * Both screens take it as optional, so a single-school project needs none — but a
 * *visiting* person can only be described properly with it: naming the school that holds
 * their identity needs the project's list of schools, and offering to go there needs
 * `selectSchool`.
 */
function context(overrides: Partial<SetupContext> = {}): SetupContext {
  return {
    schools: [NORTH, SOUTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
    ...overrides,
  };
}

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

/** Ada, created at Northgate. One record, whichever school is looking at her. */
const ADA = {
  id: "teacher-ada",
  schoolId: NORTH.id,
  code: null,
  givenName: "Ada",
  familyName: "Okafor",
  email: null,
  homeCampusId: null,
  ftePercent: 100,
  maxPeriodsPerDay: null,
  maxPeriodsPerCycle: null,
  maxConsecutivePeriods: null,
  notes: null,
  isActive: true,
  ...STAMP,
} as Teacher;

/** Cass, created at Northgate. */
const CASS = {
  id: "student-cass",
  schoolId: NORTH.id,
  code: "NG-1001",
  givenName: "Cass",
  familyName: "Pupilsen",
  email: null,
  yearLevelId: "year-north",
  campusId: null,
  isActive: true,
  ...STAMP,
} as Student;

function membership(schoolId: string, isActive: boolean): TeacherInSchool {
  return {
    teacher: ADA,
    membership: {
      id: `ts-${schoolId}`,
      teacherId: ADA.id,
      schoolId,
      isActive,
      ...STAMP,
    },
  } as TeacherInSchool;
}

function enrolment(schoolId: string, yearLevelId: string | null): StudentInSchool {
  return {
    student: CASS,
    enrolment: {
      id: `e-${schoolId}`,
      studentId: CASS.id,
      schoolId,
      admissionNo: schoolId === NORTH.id ? "NG-1001" : "SM-77",
      yearLevelId,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: null,
      leftOn: null,
      ...STAMP,
    } as StudentEnrolment,
  } as StudentInSchool;
}

function yearLevel(id: string, name: string, schoolId: string): YearLevel {
  return { id, schoolId, name, ordinal: 8, ...STAMP } as YearLevel;
}

beforeEach(() => {
  vi.clearAllMocks();
  updateTeacher.mockResolvedValue(ADA);
  listEnrolmentHistory.mockImplementation((studentId) =>
    Promise.resolve(studentId === CASS.id ? [enrolment(SOUTH.id, "year-south").enrolment] : []),
  );

  // Both schools list the same person, because both have a membership for her. This is
  // the shape the backend actually returns: `listTeachers` reads `teacher_school`.
  listTeachers.mockResolvedValue([ADA]);
  listStudents.mockResolvedValue([CASS]);
  listYearLevels.mockImplementation((schoolId: string) =>
    Promise.resolve(
      schoolId === NORTH.id
        ? [yearLevel("year-north", "Grade 8 (Northgate)", NORTH.id)]
        : [yearLevel("year-south", "Grade 8 (Southmoor)", SOUTH.id)],
    ),
  );
});

describe("a teacher shared between two schools", () => {
  it("appears once in each school's list, as the same record", async () => {
    listSchoolTeachers.mockImplementation((schoolId: string) =>
      Promise.resolve([membership(schoolId, true)]),
    );

    const view = render(<Teachers school={NORTH} />);
    expect(await screen.findByText("Okafor")).toBeTruthy();
    expect(screen.getAllByText("Okafor")).toHaveLength(1);
    expect(screen.getByText("This school")).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} />);

    await waitFor(() => expect(listSchoolTeachers).toHaveBeenCalledWith(SOUTH.id));
    expect(screen.getAllByText("Okafor")).toHaveLength(1);
  });

  it("shows this school's employment, not the project-level flag", async () => {
    // Active as a person, no longer employed by the school being looked at. Reading
    // `teacher.isActive` alone would say "Active", which is the one wrong answer.
    listSchoolTeachers.mockResolvedValue([membership(SOUTH.id, false)]);

    render(<Teachers school={SOUTH} />);

    expect(await screen.findByText("Okafor")).toBeTruthy();
    expect(screen.getByText("Not teaching here")).toBeTruthy();
    expect(screen.queryByText("Active")).toBeNull();
  });

  it("marks a visiting teacher, and names the school that holds their identity", async () => {
    listSchoolTeachers.mockResolvedValue([membership(SOUTH.id, true)]);

    render(<Teachers school={SOUTH} context={context()} />);

    expect(await screen.findByText("Okafor")).toBeTruthy();
    // Stage 0C names the school rather than saying "another": "visiting" alone does not
    // tell the user where to go to change anything.
    expect(screen.getByText(/Visiting from Northgate Secondary/)).toBeTruthy();
  });

  /**
   * Stage 0B *disclosed* that editing a visiting teacher would be refused, and let the
   * user send it anyway. Stage 0C does not offer the edit at all, and offers the thing
   * that works instead — which is the difference between a screen that warns and a screen
   * that is correct.
   */
  it("does not offer to edit a visiting teacher's identity here", async () => {
    listSchoolTeachers.mockResolvedValue([membership(SOUTH.id, true)]);

    render(<Teachers school={SOUTH} context={context()} />);
    expect(await screen.findByText("Okafor")).toBeTruthy();

    const edit = screen.getByRole("button", { name: "Edit Ada Okafor" });
    expect(edit.hasAttribute("disabled")).toBe(true);
    expect(edit.getAttribute("title")).toMatch(/Northgate Secondary/);
    expect(edit.getAttribute("title")).toMatch(/workload settings/);
    expect(updateTeacher).not.toHaveBeenCalled();
  });

  it("offers the school where that edit is allowed, and switches to it", async () => {
    const user = userEvent.setup();
    const selectSchool = vi.fn();
    listSchoolTeachers.mockResolvedValue([membership(SOUTH.id, true)]);

    render(<Teachers school={SOUTH} context={context({ selectSchool })} />);
    expect(await screen.findByText("Okafor")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /^Go to home school/ }));

    expect(selectSchool).toHaveBeenCalledWith(NORTH.id);
  });

  it("does not offer to delete a visiting teacher, and says why", async () => {
    const user = userEvent.setup();
    listSchoolTeachers.mockResolvedValue([membership(SOUTH.id, true)]);

    render(<Teachers school={SOUTH} context={context()} />);
    expect(await screen.findByText("Okafor")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /^Delete Ada Okafor/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/only the school that created somebody/i)).toBeTruthy();
    // No destructive button: the refusal is certain, so the dialogue explains instead.
    expect(within(dialog).queryByRole("button", { name: /^Delete teacher$/ })).toBeNull();
    expect(within(dialog).getByRole("button", { name: /^Close$/ })).toBeTruthy();
  });

  it("shows this school's own employment on the row, not the project-level flag", async () => {
    // Already covered above for the inactive case; this is the shared-identity version:
    // the same person, two schools, two different answers to "do you employ her?".
    listSchoolTeachers.mockImplementation((schoolId: string) =>
      Promise.resolve([membership(schoolId, schoolId === NORTH.id)]),
    );

    const view = render(<Teachers school={NORTH} context={context()} />);
    expect(await screen.findByText("Active")).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} context={context()} />);

    expect(await screen.findByText("Not teaching here")).toBeTruthy();
    expect(screen.queryByText("Active")).toBeNull();
  });
});

describe("a student enrolled in two schools", () => {
  it("is placed in the year level of the school being looked at", async () => {
    listSchoolStudents.mockImplementation((schoolId: string) =>
      Promise.resolve([
        enrolment(schoolId, schoolId === NORTH.id ? "year-north" : "year-south"),
      ]),
    );

    const view = render(<StudentsAndYearLevels school={NORTH} />);
    // Twice: the Year levels table that defines it, and the student's Year column.
    await waitFor(() =>
      expect(screen.getAllByText("Grade 8 (Northgate)")).toHaveLength(2),
    );

    view.rerender(<StudentsAndYearLevels school={SOUTH} />);

    // Southmoor's own year level, from Southmoor's enrolment -- not the "year-north" the
    // student record still carries as its home-school value.
    await waitFor(() =>
      expect(screen.getAllByText("Grade 8 (Southmoor)")).toHaveLength(2),
    );
    expect(screen.queryAllByText("Grade 8 (Northgate)")).toHaveLength(0);
  });

  it("shows the admission number the school being looked at gave them", async () => {
    listSchoolStudents.mockResolvedValue([enrolment(SOUTH.id, "year-south")]);

    render(<StudentsAndYearLevels school={SOUTH} />);

    expect(await screen.findByText("SM-77")).toBeTruthy();
    expect(screen.queryByText("NG-1001")).toBeNull();
  });

  it("says so when every spell at this school is in the past", async () => {
    // `listStudents` keeps returning a transferred-away student, because the school still
    // holds their history. With no current spell, "Enrolled" would be a lie.
    listSchoolStudents.mockResolvedValue([]);

    render(<StudentsAndYearLevels school={SOUTH} />);

    expect(await screen.findByText("Pupilsen")).toBeTruthy();
    expect(screen.getByText("Left this school")).toBeTruthy();
  });

  it("marks a visiting student, and names the school that holds their identity", async () => {
    listSchoolStudents.mockResolvedValue([enrolment(SOUTH.id, "year-south")]);

    render(<StudentsAndYearLevels school={SOUTH} context={context()} />);

    expect(await screen.findByText("Pupilsen")).toBeTruthy();
    expect(screen.getByText(/Visiting from Northgate Secondary/)).toBeTruthy();
  });

  it("does not offer to edit a visiting student's identity here", async () => {
    listSchoolStudents.mockResolvedValue([enrolment(SOUTH.id, "year-south")]);

    render(<StudentsAndYearLevels school={SOUTH} context={context()} />);
    expect(await screen.findByText("Pupilsen")).toBeTruthy();

    const edit = screen.getByRole("button", { name: "Edit Cass Pupilsen" });
    expect(edit.hasAttribute("disabled")).toBe(true);
    // And the thing this school *can* change is named in the same breath.
    expect(edit.getAttribute("title")).toMatch(/Northgate Secondary/);
    expect(edit.getAttribute("title")).toMatch(/Enrolment/);
  });

  it("offers this school's enrolment of them as the thing it can change", async () => {
    const user = userEvent.setup();
    listSchoolStudents.mockResolvedValue([enrolment(SOUTH.id, "year-south")]);

    render(<StudentsAndYearLevels school={SOUTH} context={context()} />);
    expect(await screen.findByText("Pupilsen")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /^Enrolment Cass Pupilsen/ }));

    // Southmoor's own spell, with Southmoor's own admission number — never Northgate's.
    const panel = (await screen.findByText(/Enrolment — Cass Pupilsen/)).closest(
      ".subPanel",
    ) as HTMLElement;
    // Twice: the current spell's summary, and the same spell in the history below it.
    expect(within(panel).getAllByText("SM-77")).toHaveLength(2);
    expect(within(panel).queryByText("NG-1001")).toBeNull();
  });
});

describe("a single-school project is unaffected", () => {
  it("still shows one school's own teacher as its own, and active", async () => {
    listSchoolTeachers.mockResolvedValue([membership(NORTH.id, true)]);

    render(<Teachers school={NORTH} />);

    expect(await screen.findByText("Okafor")).toBeTruthy();
    expect(screen.getByText("Active")).toBeTruthy();
    expect(screen.getByText("This school")).toBeTruthy();
    expect(screen.queryByText("Another school (visiting)")).toBeNull();
  });

  it("still shows its own student's placement from its own enrolment", async () => {
    listSchoolStudents.mockResolvedValue([enrolment(NORTH.id, "year-north")]);

    render(<StudentsAndYearLevels school={NORTH} />);

    expect(await screen.findByText("Pupilsen")).toBeTruthy();
    await waitFor(() =>
      expect(screen.getAllByText("Grade 8 (Northgate)")).toHaveLength(2),
    );
    expect(screen.getByText("Enrolled")).toBeTruthy();
  });
});
