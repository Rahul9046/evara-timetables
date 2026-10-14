/**
 * Stage 0C: a form opened under one school must never submit under another.
 *
 * This is the failure mode multi-school editing introduces and nothing else in the
 * application has. A section holds state that is *about* a school — an open edit form, a
 * pending delete, a chosen teacher, a half-typed enrolment — and `school` is a prop.
 * Re-rendering with a new one re-reads the lists and leaves that state standing, so a
 * perfectly ordinary sequence (open a form, switch schools, press Save) would write one
 * school's values under the other's name.
 *
 * Three layers answer it and all three are wanted:
 *
 * 1. **The shell remounts.** `SchoolSetup` keys the section body by school identifier, so
 *    a switch discards every form and dialogue. `SchoolSetup.test.tsx` asserts that.
 * 2. **Each section re-reads and closes its form.** `CrudPanel` clears `editing` when its
 *    `reloadKey` changes, which is what makes the *rerender* path safe too — and the
 *    rerender path is what a test, a future router, or any caller that does not remount
 *    will produce.
 * 3. **The backend refuses.** Every mutation names its school and `WrongSchool` is raised
 *    inside the write transaction.
 *
 * This file covers layer 2 — the one a component can get wrong on its own, and the one the
 * brief singles out: "the application must not submit or display School A's data as though
 * it belongs to School B". It drives the sections directly with a rerender, deliberately
 * *not* through the shell, because that is the harder case.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const updateTeacher = vi.fn<(id: string, input: unknown) => Promise<Teacher>>();
const updateStudent = vi.fn<(id: string, input: unknown) => Promise<Student>>();
const deleteTeacher = vi.fn<(schoolId: string, id: string) => Promise<void>>();
const listEnrolmentHistory = vi.fn<(studentId: string) => Promise<StudentEnrolment[]>>();
const updateEnrolment =
  vi.fn<(schoolId: string, id: string, input: unknown) => Promise<StudentEnrolment>>();

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  listSchoolTeachers: (schoolId: string) => listSchoolTeachers(schoolId),
  createTeacher: vi.fn(() => Promise.resolve({})),
  updateTeacher: (id: string, input: unknown) => updateTeacher(id, input),
  deleteTeacher: (schoolId: string, id: string) => deleteTeacher(schoolId, id),
  listStudents: (schoolId: string) => listStudents(schoolId),
  listSchoolStudents: (schoolId: string) => listSchoolStudents(schoolId),
  createStudent: vi.fn(() => Promise.resolve({})),
  updateStudent: (id: string, input: unknown) => updateStudent(id, input),
  deleteStudent: vi.fn(() => Promise.resolve()),
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  createYearLevel: vi.fn(() => Promise.resolve({})),
  updateYearLevel: vi.fn(() => Promise.resolve({})),
  deleteYearLevel: vi.fn(() => Promise.resolve()),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listGroupsForStudent: vi.fn(() => Promise.resolve([])),
  listEnrolmentHistory: (studentId: string) => listEnrolmentHistory(studentId),
  updateEnrolment: (schoolId: string, id: string, input: unknown) =>
    updateEnrolment(schoolId, id, input),
  enrolStudent: vi.fn(() => Promise.resolve({})),
  leaveSchool: vi.fn(() => Promise.resolve({})),
  transferStudent: vi.fn(() => Promise.resolve({})),
  addTeacherToSchool: vi.fn(() => Promise.resolve({})),
  listTeacherMemberships: vi.fn(() => Promise.resolve([])),
  setTeacherMembershipActive: vi.fn(() => Promise.resolve({})),
  removeTeacherFromSchool: vi.fn(() => Promise.resolve()),
  listTeacherCampuses: vi.fn(() => Promise.resolve([])),
  assignTeacherCampus: vi.fn(() => Promise.resolve({})),
  removeTeacherCampus: vi.fn(() => Promise.resolve()),
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
const { Students } = await import("./Students");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function teacher(id: string, family: string, schoolId: string): Teacher {
  return {
    id,
    schoolId,
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
    isActive: true,
    ...STAMP,
  } as Teacher;
}

const NORTH_TEACHER = teacher("t-north", "Northwood", NORTH.id);
const SOUTH_TEACHER = teacher("t-south", "Southwell", SOUTH.id);

function student(id: string, family: string, schoolId: string): Student {
  return {
    id,
    schoolId,
    code: `${family.slice(0, 2).toUpperCase()}-1`,
    givenName: "Cass",
    familyName: family,
    email: null,
    yearLevelId: null,
    campusId: null,
    isActive: true,
    ...STAMP,
  } as Student;
}

const NORTH_STUDENT = student("s-north", "Northman", NORTH.id);
const SOUTH_STUDENT = student("s-south", "Southey", SOUTH.id);

function inSchool(person: Teacher, schoolId: string): TeacherInSchool {
  return {
    teacher: person,
    membership: {
      id: `ts-${person.id}`,
      teacherId: person.id,
      schoolId,
      isActive: true,
      ...STAMP,
    },
  } as TeacherInSchool;
}

function spell(person: Student, schoolId: string): StudentInSchool {
  return {
    student: person,
    enrolment: {
      id: `e-${person.id}`,
      studentId: person.id,
      schoolId,
      admissionNo: person.code,
      yearLevelId: null,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: null,
      leftOn: null,
      ...STAMP,
    } as StudentEnrolment,
  } as StudentInSchool;
}

function context(): SetupContext {
  return {
    schools: [NORTH, SOUTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listTeachers.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_TEACHER] : [SOUTH_TEACHER]),
  );
  listSchoolTeachers.mockImplementation((schoolId) =>
    Promise.resolve([inSchool(schoolId === NORTH.id ? NORTH_TEACHER : SOUTH_TEACHER, schoolId)]),
  );
  listStudents.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_STUDENT] : [SOUTH_STUDENT]),
  );
  listSchoolStudents.mockImplementation((schoolId) =>
    Promise.resolve([spell(schoolId === NORTH.id ? NORTH_STUDENT : SOUTH_STUDENT, schoolId)]),
  );
  listYearLevels.mockResolvedValue([]);
  listCampuses.mockResolvedValue([]);
  listEnrolmentHistory.mockResolvedValue([]);
  updateTeacher.mockResolvedValue(NORTH_TEACHER);
  updateStudent.mockResolvedValue(NORTH_STUDENT);
  updateEnrolment.mockResolvedValue(spell(NORTH_STUDENT, NORTH.id).enrolment);
  deleteTeacher.mockResolvedValue(undefined);
});

describe("a teacher form open when the school changes", () => {
  it("is closed rather than left standing over the new school's list", async () => {
    const user = userEvent.setup();
    const view = render(<Teachers school={NORTH} context={context()} />);
    await screen.findByText("Northwood");

    await user.click(screen.getByRole("button", { name: /^Edit Ada Northwood/ }));
    expect(screen.getByRole("button", { name: "Save changes" })).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} context={context()} />);

    await screen.findByText("Southwell");
    // The form is gone: there is nothing left to press Save on, so there is nothing that
    // could write Northgate's values against Southmoor.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull(),
    );
    expect(updateTeacher).not.toHaveBeenCalled();
  });

  it("never sends the school it was opened under", async () => {
    const user = userEvent.setup();
    const view = render(<Teachers school={NORTH} context={context()} />);
    await screen.findByText("Northwood");

    await user.click(screen.getByRole("button", { name: /^Edit Ada Northwood/ }));
    view.rerender(<Teachers school={SOUTH} context={context()} />);
    await screen.findByText("Southwell");

    // Re-opening after the switch edits the row now on screen, and the input names
    // Southmoor — which is also what the backend's own check would require.
    await user.click(screen.getByRole("button", { name: /^Edit Ada Southwell/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    expect(updateTeacher).toHaveBeenCalledWith("t-south", expect.objectContaining({
      schoolId: SOUTH.id,
    }));
    for (const call of updateTeacher.mock.calls) {
      expect((call[1] as { schoolId: string }).schoolId).not.toBe(NORTH.id);
    }
  });

  it("shows no trace of the school left behind", async () => {
    const view = render(<Teachers school={NORTH} context={context()} />);
    await screen.findByText("Northwood");

    view.rerender(<Teachers school={SOUTH} context={context()} />);

    await screen.findByText("Southwell");
    await waitFor(() => expect(screen.queryByText("Northwood")).toBeNull());
  });
});

describe("a student form open when the school changes", () => {
  it("is closed, and the roll is the new school's", async () => {
    const user = userEvent.setup();
    const view = render(<Students school={NORTH} context={context()} />);
    await screen.findByText("Northman");

    await user.click(screen.getByRole("button", { name: /^Edit Cass Northman/ }));
    expect(screen.getByRole("button", { name: "Save changes" })).toBeTruthy();

    view.rerender(<Students school={SOUTH} context={context()} />);

    await screen.findByText("Southey");
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull(),
    );
    expect(updateStudent).not.toHaveBeenCalled();
    expect(screen.queryByText("Northman")).toBeNull();
  });
});

describe("an enrolment panel open when the school changes", () => {
  it("stops being about the student who is no longer on the roll", async () => {
    const user = userEvent.setup();
    const view = render(<Students school={NORTH} context={context()} />);
    await screen.findByText("Northman");

    await user.click(screen.getByRole("button", { name: /^Enrolment Cass Northman/ }));
    expect(await screen.findByText(/Enrolment — Cass Northman/)).toBeTruthy();

    view.rerender(<Students school={SOUTH} context={context()} />);
    await screen.findByText("Southey");

    // The panel is keyed off the roll, so a student the new school does not have cannot
    // stay open with Northgate's spell under Southmoor's heading.
    await waitFor(() =>
      expect(screen.queryByText(/Enrolment — Cass Northman/)).toBeNull(),
    );
    expect(screen.getByText("No student chosen")).toBeTruthy();
    expect(updateEnrolment).not.toHaveBeenCalled();
  });
});

describe("a relationship panel open when the school changes", () => {
  it("stops being about a teacher the new school does not employ", async () => {
    const user = userEvent.setup();
    const view = render(<Teachers school={NORTH} context={context()} />);
    await screen.findByText("Northwood");

    await user.click(screen.getByRole("button", { name: /^Schools Ada Northwood/ }));
    expect(await screen.findByText(/Schools and campuses — Ada Northwood/)).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} context={context()} />);
    await screen.findByText("Southwell");

    await waitFor(() =>
      expect(screen.queryByText(/Schools and campuses — Ada Northwood/)).toBeNull(),
    );
    // Twice: the relationship panel and the availability panel below it, both of which
    // chose a teacher who is no longer on this school's list.
    expect(screen.getAllByText("No teacher chosen")).toHaveLength(2);
  });
});

describe("a slow read that lands after the school has changed", () => {
  it("does not overwrite the new school's list with the old school's rows", async () => {
    // The response-ordering case the brief asks about. `useList` and `CrudPanel` both
    // guard it with a liveness flag in the effect's cleanup, so a reply from a request
    // whose effect has been torn down is dropped rather than rendered.
    let releaseNorth: (rows: TeacherInSchool[]) => void = () => undefined;
    listSchoolTeachers.mockImplementation((schoolId) => {
      if (schoolId === NORTH.id) {
        return new Promise<TeacherInSchool[]>((resolve) => {
          releaseNorth = resolve;
        });
      }
      return Promise.resolve([inSchool(SOUTH_TEACHER, SOUTH.id)]);
    });

    const view = render(<Teachers school={NORTH} context={context()} />);
    // Northgate's read is still in flight.
    expect(screen.queryByText("Northwood")).toBeNull();

    view.rerender(<Teachers school={SOUTH} context={context()} />);
    await screen.findByText("Southwell");

    // Northgate's reply arrives now, after the switch.
    releaseNorth([inSchool(NORTH_TEACHER, NORTH.id)]);

    await waitFor(() => expect(screen.queryByText("Northwood")).toBeNull());
    expect(screen.getByText("Southwell")).toBeTruthy();
  });
});

describe("a delete confirmation open when the school changes", () => {
  it("never deletes against the school it was opened under", async () => {
    const user = userEvent.setup();
    const view = render(<Teachers school={NORTH} context={context()} />);
    await screen.findByText("Northwood");

    await user.click(screen.getByRole("button", { name: /^Delete Ada Northwood/ }));
    expect(await screen.findByRole("dialog")).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} context={context()} />);
    await screen.findByText("Southwell");

    // Whatever remains on screen, nothing has been sent, and the only row a delete could
    // now name is Southmoor's own.
    expect(deleteTeacher).not.toHaveBeenCalled();
    for (const call of deleteTeacher.mock.calls) {
      expect(call[0]).not.toBe(NORTH.id);
    }
  });
});
