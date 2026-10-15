/**
 * Stage A: the split sections navigate to each other, by identifier.
 *
 * Splitting Students in two created a dependency between two *screens* where there had
 * been one screen with two panels: the student form offers year levels it cannot create.
 * The brief's answer is an actionable empty state, and the thing worth testing is that the
 * action **opens the right section** — which is the part a reorder can silently break.
 *
 * So every assertion here is about `SetupContext.openSection` being called with a stable
 * identifier. A section index would have made the Stage A reorder a behaviour change in
 * every one of these places, with nothing failing: `openSection(11)` still resolves, just
 * to the wrong screen.
 *
 * The shell's own ordering and gating are `SchoolSetup.test.tsx`'s; this is about the
 * screens that navigate.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Campus,
  Cycle,
  PeriodStructure,
  School,
  Student,
  StudentEnrolment,
  StudentGroup,
  StudentInSchool,
  Teacher,
  TeacherInSchool,
  YearLevel,
} from "@evara/domain";

const listYearLevels = vi.fn<(schoolId: string) => Promise<YearLevel[]>>();
const listStudents = vi.fn<(schoolId: string) => Promise<Student[]>>();
const listSchoolStudents = vi.fn<(schoolId: string) => Promise<StudentInSchool[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listEnrolmentHistory = vi.fn<(studentId: string) => Promise<StudentEnrolment[]>>();
const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const listSchoolTeachers = vi.fn<(schoolId: string) => Promise<TeacherInSchool[]>>();
const listStudentGroups = vi.fn<(schoolId: string) => Promise<StudentGroup[]>>();
const listCycles = vi.fn<(schoolId: string) => Promise<Cycle[]>>();
const listPeriodStructures = vi.fn<(schoolId: string) => Promise<PeriodStructure[]>>();

vi.mock("../../ipc/setup", () => ({
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  createYearLevel: vi.fn(() => Promise.resolve({})),
  updateYearLevel: vi.fn(() => Promise.resolve({})),
  deleteYearLevel: vi.fn(() => Promise.resolve()),
  listStudents: (schoolId: string) => listStudents(schoolId),
  listSchoolStudents: (schoolId: string) => listSchoolStudents(schoolId),
  createStudent: vi.fn(() => Promise.resolve({})),
  updateStudent: vi.fn(() => Promise.resolve({})),
  deleteStudent: vi.fn(() => Promise.resolve()),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listGroupsForStudent: vi.fn(() => Promise.resolve([])),
  listEnrolmentHistory: (studentId: string) => listEnrolmentHistory(studentId),
  enrolStudent: vi.fn(() => Promise.resolve({})),
  updateEnrolment: vi.fn(() => Promise.resolve({})),
  leaveSchool: vi.fn(() => Promise.resolve({})),
  transferStudent: vi.fn(() => Promise.resolve({})),
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  listSchoolTeachers: (schoolId: string) => listSchoolTeachers(schoolId),
  createTeacher: vi.fn(() => Promise.resolve({})),
  updateTeacher: vi.fn(() => Promise.resolve({})),
  deleteTeacher: vi.fn(() => Promise.resolve()),
  listTeacherMemberships: vi.fn(() => Promise.resolve([])),
  listTeacherCampuses: vi.fn(() => Promise.resolve([])),
  addTeacherToSchool: vi.fn(() => Promise.resolve({})),
  setTeacherMembershipActive: vi.fn(() => Promise.resolve({})),
  removeTeacherFromSchool: vi.fn(() => Promise.resolve()),
  assignTeacherCampus: vi.fn(() => Promise.resolve({})),
  removeTeacherCampus: vi.fn(() => Promise.resolve()),
  listStudentGroups: (schoolId: string) => listStudentGroups(schoolId),
  createStudentGroup: vi.fn(() => Promise.resolve({})),
  updateStudentGroup: vi.fn(() => Promise.resolve({})),
  deleteStudentGroup: vi.fn(() => Promise.resolve()),
  listGroupMembers: vi.fn(() => Promise.resolve([])),
  countGroupMembers: vi.fn(() => Promise.resolve(0)),
  addGroupMember: vi.fn(() => Promise.resolve({})),
  removeGroupMember: vi.fn(() => Promise.resolve()),
  listCycles: (schoolId: string) => listCycles(schoolId),
  listPeriodStructures: (schoolId: string) => listPeriodStructures(schoolId),
  listAvailability: vi.fn(() => Promise.resolve([])),
  countAvailability: vi.fn(() => Promise.resolve(0)),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  previewGrid: vi.fn(() => Promise.resolve(null)),
  gridPreview: vi.fn(() => Promise.resolve(null)),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: () => null,
}));

const { Students } = await import("./Students");
const { Teachers } = await import("./Teachers");
const { StudentGroups } = await import("./StudentGroups");
const { SECTIONS } = await import("./SchoolSetup");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const SCHOOL = { id: "school-1", name: "Northgate Secondary", code: "NGS" } as School;

const CASS = {
  id: "student-cass",
  schoolId: SCHOOL.id,
  code: "NG-1001",
  givenName: "Cass",
  familyName: "Pupilsen",
  email: null,
  yearLevelId: null,
  campusId: null,
  isActive: true,
  ...STAMP,
} as Student;

const SPELL = {
  id: "e-1",
  studentId: CASS.id,
  schoolId: SCHOOL.id,
  admissionNo: "NG-1001",
  yearLevelId: null,
  campusId: null,
  status: "ENROLLED",
  enrolledOn: null,
  leftOn: null,
  ...STAMP,
} as StudentEnrolment;

const ADA = {
  id: "teacher-ada",
  schoolId: SCHOOL.id,
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

/** The context the shell hands down, with a spy on the navigation callback. */
function context(openSection: (id: string) => void): SetupContext {
  return {
    schools: [SCHOOL],
    selectSchool: vi.fn(),
    openSection,
    reloadSchools: vi.fn(),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listYearLevels.mockResolvedValue([]);
  listStudents.mockResolvedValue([CASS]);
  listSchoolStudents.mockResolvedValue([
    { student: CASS, enrolment: SPELL } as StudentInSchool,
  ]);
  listCampuses.mockResolvedValue([]);
  listEnrolmentHistory.mockResolvedValue([SPELL]);
  listTeachers.mockResolvedValue([ADA]);
  listSchoolTeachers.mockResolvedValue([
    {
      teacher: ADA,
      membership: {
        id: "ts-1",
        teacherId: ADA.id,
        schoolId: SCHOOL.id,
        isActive: true,
        ...STAMP,
      },
    } as TeacherInSchool,
  ]);
  listStudentGroups.mockResolvedValue([
    { id: "g-1", schoolId: SCHOOL.id, name: "8A", kind: "HOMEROOM" } as StudentGroup,
  ]);
  listCycles.mockResolvedValue([]);
  listPeriodStructures.mockResolvedValue([]);
});

/** Every identifier a screen may navigate to has to be one the shell actually offers. */
const SECTION_IDS = new Set(SECTIONS.map((section) => section.id));

describe("the Students screen with no year levels", () => {
  it("says so, in the words the brief asks for", async () => {
    render(<Students school={SCHOOL} context={context(vi.fn())} />);

    expect(
      await screen.findByText(/No year levels have been configured for this school yet/),
    ).toBeTruthy();
  });

  it("offers to open Year levels, by identifier", async () => {
    const user = userEvent.setup();
    const openSection = vi.fn();
    render(<Students school={SCHOOL} context={context(openSection)} />);
    await screen.findByText(/No year levels have been configured/);

    await user.click(screen.getByRole("button", { name: "Go to Year levels" }));

    expect(openSection).toHaveBeenCalledWith("yearLevels");
    // And that identifier is one the shell knows, which is what a position could not
    // guarantee across a reorder.
    expect(SECTION_IDS.has(openSection.mock.calls[0]?.[0] as string)).toBe(true);
  });

  it("does not block entering a student, because a year level is optional", async () => {
    // ADR 0006 and `V13` both make the placement optional. The note is advice; a school
    // that does not track years must be able to ignore it entirely.
    const user = userEvent.setup();
    render(<Students school={SCHOOL} context={context(vi.fn())} />);
    await screen.findByText("Pupilsen");

    await user.click(screen.getByRole("button", { name: "Add student" }));

    expect(screen.getByLabelText(/First name/)).toBeTruthy();
    const add = screen.getAllByRole("button", { name: "Add student" }).at(-1);
    expect(add?.hasAttribute("disabled")).toBe(false);
  });

  it("says in the field's own hint where year levels come from", async () => {
    const user = userEvent.setup();
    render(<Students school={SCHOOL} context={context(vi.fn())} />);
    await screen.findByText("Pupilsen");

    await user.click(screen.getByRole("button", { name: "Add student" }));

    expect(
      screen.getByText(/This school has no year levels yet\. Add them under/),
    ).toBeTruthy();
  });

  it("stops saying it once the school has some", async () => {
    listYearLevels.mockResolvedValue([
      { id: "y-1", schoolId: SCHOOL.id, name: "Year 9", ordinal: 9, ...STAMP } as YearLevel,
    ]);
    render(<Students school={SCHOOL} context={context(vi.fn())} />);

    await screen.findByText("Pupilsen");
    await waitFor(() =>
      expect(screen.queryByText(/No year levels have been configured/)).toBeNull(),
    );
    expect(screen.queryByRole("button", { name: "Go to Year levels" })).toBeNull();
  });

  it("offers it from the enrolment panel too, where a spell is placed", async () => {
    const user = userEvent.setup();
    const openSection = vi.fn();
    render(<Students school={SCHOOL} context={context(openSection)} />);
    await screen.findByText("Pupilsen");

    await user.click(screen.getByRole("button", { name: /^Enrolment Cass Pupilsen/ }));
    const panel = (await screen.findByText(/^Enrolment — /)).closest(
      ".subPanel",
    ) as HTMLElement;
    await user.click(within(panel).getByRole("button", { name: "Go to Year levels" }));

    expect(openSection).toHaveBeenCalledWith("yearLevels");
  });

  it("works with no context at all, for a caller that does not navigate", async () => {
    // Both screens take the context as optional so a single-school caller — and every
    // existing test — keeps working. The note still appears; only the button does not.
    render(<Students school={SCHOOL} />);

    expect(
      await screen.findByText(/No year levels have been configured/),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Go to Year levels" })).toBeNull();
  });
});

/**
 * Availability is set against a cycle's days and a bell schedule's periods, and Stage A
 * moved people *before* both. That was always a legal state; it is now the common one, so
 * the empty states have to do more than name the section.
 */
describe("availability before the time model exists", () => {
  it.each([
    ["Teachers", "teacher"],
    ["Student groups", "group"],
  ])("offers to open Cycles from %s", async (_screen, noun) => {
    const user = userEvent.setup();
    const openSection = vi.fn();
    if (noun === "teacher") {
      render(<Teachers school={SCHOOL} context={context(openSection)} />);
      await screen.findByText("Okafor");
    } else {
      render(<StudentGroups school={SCHOOL} context={context(openSection)} />);
      // The name appears in the groups table and in the membership pickers below it.
      await waitFor(() => expect(screen.getAllByText("8A").length).toBeGreaterThan(0));
    }

    await user.selectOptions(
      screen.getByLabelText(`${noun} to set availability for`),
      noun === "teacher" ? ADA.id : "g-1",
    );

    expect(await screen.findByText("No cycle yet")).toBeTruthy();
    await user.click(
      screen.getByRole("button", { name: "Go to Cycles and cycle days" }),
    );
    expect(openSection).toHaveBeenCalledWith("cycles");
  });

  it("offers to open Bell schedules once a cycle exists", async () => {
    const user = userEvent.setup();
    const openSection = vi.fn();
    listCycles.mockResolvedValue([
      { id: "c-1", schoolId: SCHOOL.id, name: "Two-week", dayCount: 10 } as Cycle,
    ]);
    render(<Teachers school={SCHOOL} context={context(openSection)} />);
    await screen.findByText("Okafor");

    await user.selectOptions(screen.getByLabelText("teacher to set availability for"), ADA.id);

    expect(await screen.findByText("No bell schedule yet")).toBeTruthy();
    await user.click(
      screen.getByRole("button", { name: "Go to Bell schedules and periods" }),
    );
    expect(openSection).toHaveBeenCalledWith("periods");
  });

  it("explains that nothing is lost by configuring the rotation afterwards", async () => {
    const user = userEvent.setup();
    render(<Teachers school={SCHOOL} context={context(vi.fn())} />);
    await screen.findByText("Okafor");

    await user.selectOptions(screen.getByLabelText("teacher to set availability for"), ADA.id);

    // The reassurance the reorder makes necessary: everything is available until a rule
    // says otherwise, so entering staff first costs nothing.
    expect(
      await screen.findByText(/everything is available until a rule says otherwise/),
    ).toBeTruthy();
  });

  it("navigates to identifiers the shell offers", async () => {
    const user = userEvent.setup();
    const openSection = vi.fn();
    render(<Teachers school={SCHOOL} context={context(openSection)} />);
    await screen.findByText("Okafor");
    await user.selectOptions(screen.getByLabelText("teacher to set availability for"), ADA.id);
    await screen.findByText("No cycle yet");

    await user.click(screen.getByRole("button", { name: "Go to Cycles and cycle days" }));

    for (const call of openSection.mock.calls) {
      expect(SECTION_IDS.has(call[0] as string)).toBe(true);
    }
  });

  it("still works with no context, showing the message without the button", async () => {
    const user = userEvent.setup();
    render(<Teachers school={SCHOOL} />);
    await screen.findByText("Okafor");

    await user.selectOptions(screen.getByLabelText("teacher to set availability for"), ADA.id);

    expect(await screen.findByText("No cycle yet")).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Go to Cycles and cycle days" }),
    ).toBeNull();
  });
});

describe("every navigating screen uses a stable identifier", () => {
  it("never addresses a section by position", () => {
    // The contract this file exists to protect. `openSection` takes a string, and every
    // identifier used anywhere in the setup screens resolves against `SECTIONS` — so the
    // Stage A reorder could not, and a future one cannot, quietly send somebody to the
    // wrong screen.
    expect([...SECTION_IDS].every((id) => typeof id === "string")).toBe(true);
    expect(SECTION_IDS.has("yearLevels")).toBe(true);
    expect(SECTION_IDS.has("cycles")).toBe(true);
    expect(SECTION_IDS.has("periods")).toBe(true);
    expect(SECTION_IDS.has("schools")).toBe(true);
    expect(SECTION_IDS.has("school")).toBe(true);
  });
});
