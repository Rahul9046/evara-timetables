/**
 * Teachers, students and student groups — the things only the interface can get wrong.
 *
 * The repository tests in `evara-db` own the rules: what a delete refuses, how a
 * membership reaches the change feed, whether an identity survives an edit. This file owns
 * the decisions that exist only here, and each of them is one a reasonable implementation
 * gets wrong in the obvious direction:
 *
 * - an inactive teacher is **shown**, not filtered out of the list
 * - deactivating is an edit to `isActive`, never a delete
 * - a new person starts active without the user ticking anything
 * - a blank load ceiling is sent as `null`, not `0`
 * - a blank group size is sent as `null`, so "use the member list" survives the round trip
 * - a student in groups is warned *before* Delete is pressed, not refused after
 * - membership is per-group and plural, and removing one does not touch the student
 * - removing a membership asks first, because nothing else records it ever existed
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School, Student, StudentGroup, Teacher, YearLevel } from "@evara/domain";

const listCampuses = vi.fn();
const listYearLevels = vi.fn();
const listTeachers = vi.fn<() => Promise<Teacher[]>>();
const createTeacher = vi.fn();
const updateTeacher = vi.fn();
const deleteTeacher = vi.fn();
const listStudents = vi.fn<() => Promise<Student[]>>();
const createStudent = vi.fn();
const updateStudent = vi.fn();
const deleteStudent = vi.fn();
const listStudentGroups = vi.fn<() => Promise<StudentGroup[]>>();
const createStudentGroup = vi.fn();
const updateStudentGroup = vi.fn();
const deleteStudentGroup = vi.fn();
const listGroupMembers = vi.fn<(groupId: string) => Promise<Student[]>>();
const addGroupMember = vi.fn();
const countGroupMembers = vi.fn<(groupId: string) => Promise<number>>();
const removeGroupMember = vi.fn();
const listGroupsForStudent = vi.fn<(studentId: string) => Promise<StudentGroup[]>>();

vi.mock("../../ipc/setup", () => ({
  listCampuses: () => listCampuses(),
  listYearLevels: () => listYearLevels(),
  createYearLevel: vi.fn(() => Promise.resolve({})),
  updateYearLevel: vi.fn(() => Promise.resolve({})),
  deleteYearLevel: vi.fn(() => Promise.resolve()),
  listTeachers: () => listTeachers(),
  // Stage 0B made these the authority on who a school employs and enrols, and Stage 0C
  // builds the two lists from them rather than from the project-level tables. So they are
  // derived from the same fixtures here: one person, one record, with this school's own
  // view of them beside it. Returning `[]` would now mean an empty roll, not a roll
  // without per-school detail.
  listSchoolTeachers: () =>
    listTeachers().then((rows) =>
      rows.map((teacher) => ({
        teacher,
        membership: {
          id: `ts-${teacher.id}`,
          teacherId: teacher.id,
          schoolId: SCHOOL.id,
          isActive: teacher.isActive,
          ...STAMP,
        },
      })),
    ),
  listSchoolStudents: () =>
    listStudents().then((rows) =>
      rows
        .filter((student) => student.isActive)
        .map((student) => ({
          student,
          enrolment: {
            id: `e-${student.id}`,
            studentId: student.id,
            schoolId: SCHOOL.id,
            admissionNo: student.code,
            yearLevelId: student.yearLevelId,
            campusId: student.campusId,
            status: "ENROLLED",
            enrolledOn: null,
            leftOn: null,
            ...STAMP,
          },
        })),
    ),
  // The Stage 0C relationship surface. These screens reach for it only once a person is
  // chosen or a second school exists, neither of which these tests do, so stubs are
  // enough — but the module surface has to be complete or the import fails.
  addTeacherToSchool: vi.fn(() => Promise.resolve({})),
  listTeacherMemberships: vi.fn(() => Promise.resolve([])),
  setTeacherMembershipActive: vi.fn(() => Promise.resolve({})),
  removeTeacherFromSchool: vi.fn(() => Promise.resolve()),
  listTeacherCampuses: vi.fn(() => Promise.resolve([])),
  assignTeacherCampus: vi.fn(() => Promise.resolve({})),
  removeTeacherCampus: vi.fn(() => Promise.resolve()),
  enrolStudent: vi.fn(() => Promise.resolve({})),
  listEnrolmentHistory: vi.fn(() => Promise.resolve([])),
  updateEnrolment: vi.fn(() => Promise.resolve({})),
  leaveSchool: vi.fn(() => Promise.resolve({})),
  transferStudent: vi.fn(() => Promise.resolve({})),
  createTeacher: (input: unknown) => createTeacher(input),
  updateTeacher: (id: string, rev: number, input: unknown) =>
    updateTeacher(id, rev, input),
  deleteTeacher: (id: string) => deleteTeacher(id),
  listStudents: () => listStudents(),
  createStudent: (input: unknown) => createStudent(input),
  updateStudent: (id: string, rev: number, input: unknown) =>
    updateStudent(id, rev, input),
  deleteStudent: (id: string) => deleteStudent(id),
  listStudentGroups: () => listStudentGroups(),
  createStudentGroup: (input: unknown) => createStudentGroup(input),
  updateStudentGroup: (id: string, rev: number, input: unknown) =>
    updateStudentGroup(id, rev, input),
  deleteStudentGroup: (id: string) => deleteStudentGroup(id),
  listGroupMembers: (groupId: string) => listGroupMembers(groupId),
  countGroupMembers: (groupId: string) => countGroupMembers(groupId),
  addGroupMember: (input: unknown) => addGroupMember(input),
  removeGroupMember: (schoolId: string, groupId: string, studentId: string) =>
    removeGroupMember(schoolId, groupId, studentId),
  listGroupsForStudent: (studentId: string) => listGroupsForStudent(studentId),
  // Stage B. Empty by default: a suite that is about something else should not have
  // to describe a curriculum, and one that is about this panel overrides them.
  listDepartments: vi.fn(() => Promise.resolve([])),
  listSubjects: vi.fn(() => Promise.resolve([])),
  listTeacherDepartments: vi.fn(() => Promise.resolve([])),
  listQualifications: vi.fn(() => Promise.resolve([])),
  listSubjectTeachers: vi.fn(() => Promise.resolve([])),
  listDepartmentTeachers: vi.fn(() => Promise.resolve([])),
  departmentDependents: vi.fn(() =>
    Promise.resolve({ total: 0, subjects: 0, teachers: 0 }),
  ),
  subjectDependents: vi.fn(() => Promise.resolve({ total: 0, qualifiedTeachers: 0 })),
  addQualification: vi.fn(() => Promise.resolve({})),
  removeQualification: vi.fn(() => Promise.resolve()),
  addTeacherDepartment: vi.fn(() => Promise.resolve({})),
  removeTeacherDepartment: vi.fn(() => Promise.resolve()),
  createDepartment: vi.fn(() => Promise.resolve({})),
  updateDepartment: vi.fn(() => Promise.resolve({})),
  deleteDepartment: vi.fn(() => Promise.resolve()),
  createSubject: vi.fn(() => Promise.resolve({})),
  updateSubject: vi.fn(() => Promise.resolve({})),
  deleteSubject: vi.fn(() => Promise.resolve()),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" && error !== null && "field" in error
      ? String((error as { field: unknown }).field)
      : null,
}));

const { Teachers } = await import("./Teachers");
const { Students } = await import("./Students");
const { StudentGroups } = await import("./StudentGroups");

const SCHOOL = { id: "school-1" } as School;
const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

function teacher(over: Partial<Teacher> = {}): Teacher {
  return {
    id: "t-1",
    schoolId: "school-1",
    code: null,
    givenName: "Amara",
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
    ...over,
  } as Teacher;
}

function student(over: Partial<Student> = {}): Student {
  return {
    id: "s-1",
    schoolId: "school-1",
    code: null,
    givenName: "Ines",
    familyName: "Varga",
    email: null,
    yearLevelId: null,
    campusId: null,
    isActive: true,
    ...STAMP,
    ...over,
  } as Student;
}

function group(over: Partial<StudentGroup> = {}): StudentGroup {
  return {
    id: "g-1",
    schoolId: "school-1",
    name: "8A Homeroom",
    code: null,
    kind: "HOMEROOM",
    yearLevelId: null,
    parentGroupId: null,
    size: null,
    campusId: null,
    ...STAMP,
    ...over,
  } as StudentGroup;
}


/**
 * Submits the open entity form.
 *
 * The toolbar's "Add teacher" button and the form's own submit share a label — the form
 * reuses the noun — so this picks the one inside the `<form>` rather than guessing by
 * position.
 */
async function submitForm(user: ReturnType<typeof userEvent.setup>, label: RegExp) {
  const form = document.querySelector("form.entityForm");
  if (!form) throw new Error("no entity form is open");
  await user.click(within(form as HTMLElement).getByRole("button", { name: label }));
}

beforeEach(() => {
  vi.clearAllMocks();
  listCampuses.mockResolvedValue([]);
  listYearLevels.mockResolvedValue([] as YearLevel[]);
  listTeachers.mockResolvedValue([]);
  listStudents.mockResolvedValue([]);
  listStudentGroups.mockResolvedValue([]);
  listGroupMembers.mockResolvedValue([]);
  countGroupMembers.mockResolvedValue(0);
  listGroupsForStudent.mockResolvedValue([]);
});

describe("Teachers", () => {
  it("lists an inactive teacher rather than hiding them", async () => {
    // A teacher who left mid-year is still named by last term's timetable. Filtering them
    // out of this list would make that timetable look like it referenced a ghost.
    //
    // Since Stage 0B the label is "Archived": `teacher.isActive` is the project-level flag
    // and the per-school one lives on the membership, so the column has to say which of
    // the two it is reporting. See the next test for the other.
    listTeachers.mockResolvedValue([
      teacher({ id: "t-1", familyName: "Raman", givenName: "Priya", isActive: false }),
    ]);

    render(<Teachers school={SCHOOL} />);

    expect(await screen.findByText("Raman")).toBeTruthy();
    expect(screen.getByText("Archived")).toBeTruthy();
  });

  it("creates a teacher as active without the user ticking anything", async () => {
    const user = userEvent.setup();
    createTeacher.mockResolvedValue(teacher());

    render(<Teachers school={SCHOOL} />);
    await user.click(await screen.findByRole("button", { name: /add teacher/i }));

    await user.type(screen.getByLabelText(/first name/i), "Amara");
    await user.type(screen.getByLabelText(/last name/i), "Okafor");
    await submitForm(user, /add teacher/i);

    await waitFor(() => expect(createTeacher).toHaveBeenCalled());
    expect(createTeacher.mock.calls[0]?.[0]).toMatchObject({
      givenName: "Amara",
      familyName: "Okafor",
      isActive: true,
    });
  });

  it("sends a blank load ceiling as null rather than zero", async () => {
    // "No limit" and "may teach nothing" are different claims about a person, and
    // defaulting the first to the second would write a rule the school never asked for.
    const user = userEvent.setup();
    createTeacher.mockResolvedValue(teacher());

    render(<Teachers school={SCHOOL} />);
    await user.click(await screen.findByRole("button", { name: /add teacher/i }));
    await user.type(screen.getByLabelText(/first name/i), "Nia");
    await user.type(screen.getByLabelText(/last name/i), "Fletcher");
    await submitForm(user, /add teacher/i);

    await waitFor(() => expect(createTeacher).toHaveBeenCalled());
    const sent = createTeacher.mock.calls[0]?.[0] as Record<string, unknown>;
    expect(sent["maxPeriodsPerDay"]).toBeNull();
    expect(sent["maxPeriodsPerCycle"]).toBeNull();
    expect(sent["maxConsecutivePeriods"]).toBeNull();
    expect(sent["code"]).toBeNull();
    expect(sent["email"]).toBeNull();
  });

  it("deactivates through an edit, never through a delete", async () => {
    const user = userEvent.setup();
    listTeachers.mockResolvedValue([teacher()]);
    updateTeacher.mockResolvedValue(teacher({ isActive: false }));

    render(<Teachers school={SCHOOL} />);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: /edit/i }));
    // Stage 0C renames this flag to what ADR 0021 says it means: "in the project at all",
    // an archive flag. Whether *this school* employs them is the membership, and is
    // changed under Schools and campuses rather than by this checkbox.
    await user.click(screen.getByLabelText(/in this campus file/i));
    await submitForm(user, /save changes/i);

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    expect(updateTeacher.mock.calls[0]?.[2]).toMatchObject({ isActive: false });
    expect(deleteTeacher).not.toHaveBeenCalled();
  });
});

describe("Students", () => {
  it("says a timetable can be built without any students", async () => {
    // ADR 0006's promise, visible where someone is deciding whether to type two thousand
    // names. If this copy goes, the promise is invisible and nobody benefits from it.
    render(<Students school={SCHOOL} />);

    expect(await screen.findByText(/optional/i)).toBeTruthy();
  });

  it("warns before Delete that a student in groups cannot be deleted", async () => {
    // The point: the refusal is explained while the user is still deciding, and the thing
    // they almost certainly meant is offered. Discovering it from a rejected call would be
    // too late to be useful.
    const user = userEvent.setup();
    listStudents.mockResolvedValue([student()]);
    listGroupsForStudent.mockResolvedValue([
      group({ id: "g-1", name: "8A Homeroom" }),
      group({ id: "g-2", name: "French elective" }),
    ]);

    render(<Students school={SCHOOL} />);
    await screen.findByText("Varga");
    await waitFor(() => expect(listGroupsForStudent).toHaveBeenCalledWith("s-1"));

    await user.click(screen.getByRole("button", { name: /delete/i }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/8A Homeroom/)).toBeTruthy();
    expect(within(dialog).getByText(/French elective/)).toBeTruthy();
    // And the thing they almost certainly meant is offered.
    expect(within(dialog).getByText(/records it and keeps everything/i)).toBeTruthy();
    // Stage 0C goes one step further than explaining: the destructive button is not
    // offered at all, because the answer is already known and asking a user to agree to a
    // refusal explains nothing. See `CrudSpec.deleteBlockedBy`.
    expect(within(dialog).queryByRole("button", { name: /^Delete student$/ })).toBeNull();
    expect(within(dialog).getByRole("button", { name: /^Close$/ })).toBeTruthy();
  });
});

describe("Student groups", () => {
  it("sends a blank size as null, so the member list stays the authority", async () => {
    const user = userEvent.setup();
    createStudentGroup.mockResolvedValue(group());

    render(<StudentGroups school={SCHOOL} />);
    await user.click(await screen.findByRole("button", { name: /add student group/i }));
    await user.type(screen.getByLabelText(/^name/i), "8A Homeroom");
    await submitForm(user, /add student group/i);

    await waitFor(() => expect(createStudentGroup).toHaveBeenCalled());
    expect((createStudentGroup.mock.calls[0]?.[0] as Record<string, unknown>)["size"]).toBeNull();
  });

  it("adds and removes a member without touching the student", async () => {
    const user = userEvent.setup();
    listStudentGroups.mockResolvedValue([group()]);
    listStudents.mockResolvedValue([student()]);
    listGroupMembers.mockResolvedValue([]);
    addGroupMember.mockResolvedValue({});

    render(<StudentGroups school={SCHOOL} />);

    await user.selectOptions(
      await screen.findByLabelText(/group to edit members of/i),
      "g-1",
    );
    await user.selectOptions(await screen.findByLabelText(/student to add/i), "s-1");
    await user.click(screen.getByRole("button", { name: /add to group/i }));

    await waitFor(() => expect(addGroupMember).toHaveBeenCalled());
    expect(addGroupMember.mock.calls[0]?.[0]).toEqual({
      studentGroupId: "g-1",
      studentId: "s-1",
    });
    expect(updateStudent).not.toHaveBeenCalled();
    expect(deleteStudent).not.toHaveBeenCalled();
  });

  /** Opens the member panel on `8A Homeroom` with Ines Varga in it. */
  async function openMembers(user: ReturnType<typeof userEvent.setup>) {
    listStudentGroups.mockResolvedValue([group()]);
    listStudents.mockResolvedValue([student()]);
    listGroupMembers.mockResolvedValue([student()]);

    render(<StudentGroups school={SCHOOL} />);
    await user.selectOptions(
      await screen.findByLabelText(/group to edit members of/i),
      "g-1",
    );
    // The row's button says "Remove" and spells out the rest for a screen reader, and the
    // accessible name is matched unnormalised — hence the loose leading whitespace.
    return screen.findByRole("button", { name: /Ines Varga from 8A Homeroom/i });
  }

  it("removes a member by the pair, leaving the student alone", async () => {
    const user = userEvent.setup();
    removeGroupMember.mockResolvedValue(undefined);

    await user.click(await openMembers(user));
    await user.click(screen.getByRole("button", { name: "Remove from group" }));

    await waitFor(() =>
      expect(removeGroupMember).toHaveBeenCalledWith("school-1", "g-1", "s-1"),
    );
    expect(deleteStudent).not.toHaveBeenCalled();
  });

  it("says how many students a group delete will unlist", async () => {
    // `countGroupMembers` exists for this dialogue and nothing else -- the command's own
    // documentation says so. Without it the confirmation says what is kept and never what
    // goes, which is the half the user needs.
    const user = userEvent.setup();
    listStudentGroups.mockResolvedValue([group()]);
    countGroupMembers.mockResolvedValue(23);

    render(<StudentGroups school={SCHOOL} />);
    await user.click(await screen.findByRole("button", { name: /^Delete/ }));

    expect(await screen.findByText(/23 students are listed in this group/i)).toBeTruthy();
    expect(screen.getByText(/nothing else records them/i)).toBeTruthy();
    expect(deleteStudentGroup).not.toHaveBeenCalled();
  });

  it("asks before removing a member, because nothing else records they were in it", async () => {
    // Membership is the only record a student was ever in a group until Phase 3's
    // enrolment arrives, and the change feed stores which row changed rather than what it
    // held. An unconfirmed click therefore destroys history irrecoverably.
    const user = userEvent.setup();

    await user.click(await openMembers(user));

    expect(removeGroupMember).not.toHaveBeenCalled();
    expect(screen.getByText(/cannot be undone/i)).toBeTruthy();
    expect(screen.getByText(/currently enrolled/i)).toBeTruthy();
  });

  it("removes nothing when the confirmation is dismissed", async () => {
    const user = userEvent.setup();

    await user.click(await openMembers(user));
    await user.click(screen.getByRole("button", { name: /cancel/i }));

    expect(removeGroupMember).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Remove from group" })).toBeNull();
  });

  it("offers only students who are not already in the group", async () => {
    // Otherwise adding someone twice is a round trip whose only outcome is an error the
    // interface could have prevented.
    listStudentGroups.mockResolvedValue([group()]);
    listStudents.mockResolvedValue([
      student({ id: "s-1", familyName: "Varga" }),
      student({ id: "s-2", familyName: "Tanaka", givenName: "Yuki" }),
    ]);
    listGroupMembers.mockResolvedValue([student({ id: "s-1", familyName: "Varga" })]);

    const user = userEvent.setup();
    render(<StudentGroups school={SCHOOL} />);
    await user.selectOptions(
      await screen.findByLabelText(/group to edit members of/i),
      "g-1",
    );

    const picker = await screen.findByLabelText(/student to add/i);
    const options = within(picker).getAllByRole("option").map((o) => o.textContent ?? "");
    expect(options.some((label) => label.includes("Tanaka"))).toBe(true);
    expect(options.some((label) => label.includes("Varga"))).toBe(false);
  });

  it("shows the declared size and the member list as separate facts", async () => {
    // A group of 28 with four names listed is a school part-way through entering students,
    // not an inconsistency. Reconciling them would mean silently overwriting one.
    listStudentGroups.mockResolvedValue([group({ size: 28 })]);
    listGroupMembers.mockResolvedValue([student()]);

    const user = userEvent.setup();
    render(<StudentGroups school={SCHOOL} />);
    await user.selectOptions(
      await screen.findByLabelText(/group to edit members of/i),
      "g-1",
    );

    expect(await screen.findByText(/declares a size of 28/i)).toBeTruthy();
  });
});
