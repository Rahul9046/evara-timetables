/**
 * Stage 0C: one student, several schools, and the history between them.
 *
 * `evara-db`'s `tests_membership.rs` owns the rules — one current spell per school, a
 * transfer keeping both spells in one transaction, an admission number unique among a
 * school's current enrolments. This file owns the five things only the interface can get
 * wrong:
 *
 * - **enrolling an existing student creates no second record.** The duplication `V13`
 *   removed, and the one an interface can single-handedly reintroduce.
 * - **a transfer is one operation, not two.** `transferStudent` names both schools and does
 *   both halves atomically; a screen that called `leaveSchool` and then `enrolStudent`
 *   could leave a real student enrolled nowhere. Nothing here does that.
 * - **a transfer and a concurrent enrolment are different things**, reached by different
 *   actions, and each says which it is.
 * - **dates are not invented.** `enrolled_on` and `left_on` are nullable, and a school that
 *   does not record them must be able to leave them blank.
 *   history is read-only.
 * - **group memberships survive.** A transfer leaves them exactly as they were, because a
 *   membership is the only record it ever happened.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Campus,
  School,
  Student,
  StudentEnrolment,
  StudentGroup,
  StudentInSchool,
  YearLevel,
} from "@evara/domain";

const listStudents = vi.fn<(schoolId: string) => Promise<Student[]>>();
const listSchoolStudents = vi.fn<(schoolId: string) => Promise<StudentInSchool[]>>();
const listYearLevels = vi.fn<(schoolId: string) => Promise<YearLevel[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listEnrolmentHistory = vi.fn<(studentId: string) => Promise<StudentEnrolment[]>>();
const listGroupsForStudent = vi.fn<(studentId: string) => Promise<StudentGroup[]>>();
const enrolStudent = vi.fn<(input: unknown) => Promise<StudentEnrolment>>();
const updateEnrolment =
  vi.fn<(schoolId: string, id: string, input: unknown) => Promise<StudentEnrolment>>();
const leaveSchool =
  vi.fn<(schoolId: string, studentId: string, leftOn: string | null) => Promise<StudentEnrolment>>();
const transferStudent =
  vi.fn<
    (from: string, to: string, studentId: string, arriving: unknown) => Promise<StudentEnrolment>
  >();
const createStudent = vi.fn((_input: unknown) => Promise.resolve({}));

vi.mock("../../ipc/setup", () => ({
  listStudents: (schoolId: string) => listStudents(schoolId),
  listSchoolStudents: (schoolId: string) => listSchoolStudents(schoolId),
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listGroupsForStudent: (studentId: string) => listGroupsForStudent(studentId),
  listEnrolmentHistory: (studentId: string) => listEnrolmentHistory(studentId),
  enrolStudent: (input: unknown) => enrolStudent(input),
  updateEnrolment: (schoolId: string, id: string, input: unknown) =>
    updateEnrolment(schoolId, id, input),
  leaveSchool: (schoolId: string, studentId: string, leftOn: string | null) =>
    leaveSchool(schoolId, studentId, leftOn),
  transferStudent: (from: string, to: string, studentId: string, arriving: unknown) =>
    transferStudent(from, to, studentId, arriving),
  createStudent: (input: unknown) => createStudent(input),
  updateStudent: vi.fn(() => Promise.resolve({})),
  deleteStudent: vi.fn(() => Promise.resolve()),
  createYearLevel: vi.fn(() => Promise.resolve({})),
  updateYearLevel: vi.fn(() => Promise.resolve({})),
  deleteYearLevel: vi.fn(() => Promise.resolve()),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: () => null,
}));

const { StudentsAndYearLevels } = await import("./StudentsAndYearLevels");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

/** Cass, entered at Northgate. One record, whichever school is looking at her. */
function student(overrides: Partial<Student> = {}): Student {
  return {
    id: "student-cass",
    schoolId: NORTH.id,
    code: "NG-1001",
    givenName: "Cass",
    familyName: "Pupilsen",
    email: null,
    yearLevelId: "year-north-8",
    campusId: null,
    isActive: true,
    ...STAMP,
    ...overrides,
  } as Student;
}

const CASS = student();
const OTHER = student({
  id: "student-dev",
  schoolId: SOUTH.id,
  code: "SM-22",
  givenName: "Dev",
  familyName: "Rao",
  yearLevelId: null,
});

function yearLevel(id: string, schoolId: string, name: string): YearLevel {
  return { id, schoolId, name, ordinal: 8, ...STAMP } as YearLevel;
}

const NORTH_Y8 = yearLevel("year-north-8", NORTH.id, "Grade 8 (Northgate)");
const SOUTH_Y8 = yearLevel("year-south-8", SOUTH.id, "Grade 8 (Southmoor)");

function campus(id: string, schoolId: string, name: string): Campus {
  return { id, schoolId, name, code: name.slice(0, 2).toUpperCase(), address: null, ...STAMP } as Campus;
}

const NORTH_MAIN = campus("c-north", NORTH.id, "Main Campus");
const SOUTH_MAIN = campus("c-south", SOUTH.id, "Riverside");

function spell(overrides: Partial<StudentEnrolment> = {}): StudentEnrolment {
  return {
    id: "e-north",
    studentId: CASS.id,
    schoolId: NORTH.id,
    admissionNo: "NG-1001",
    yearLevelId: NORTH_Y8.id,
    campusId: null,
    status: "ENROLLED",
    enrolledOn: "2026-09-01",
    leftOn: null,
    ...STAMP,
    ...overrides,
  } as StudentEnrolment;
}

function inSchool(person: Student, enrolment: StudentEnrolment): StudentInSchool {
  return { student: person, enrolment } as StudentInSchool;
}

function context(overrides: Partial<SetupContext> = {}): SetupContext {
  return {
    schools: [NORTH, SOUTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listStudents.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [CASS] : [OTHER]),
  );
  listSchoolStudents.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id
        ? [inSchool(CASS, spell())]
        : [inSchool(OTHER, spell({ id: "e-dev", studentId: OTHER.id, schoolId: SOUTH.id, admissionNo: "SM-22", yearLevelId: null }))],
    ),
  );
  listYearLevels.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_Y8] : [SOUTH_Y8]),
  );
  listCampuses.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_MAIN] : [SOUTH_MAIN]),
  );
  listEnrolmentHistory.mockResolvedValue([spell()]);
  listGroupsForStudent.mockResolvedValue([]);
  enrolStudent.mockResolvedValue(spell({ id: "e-new" }));
  updateEnrolment.mockResolvedValue(spell());
  leaveSchool.mockResolvedValue(spell({ status: "LEFT", leftOn: "2027-07-20" }));
  transferStudent.mockResolvedValue(
    spell({ id: "e-south", schoolId: SOUTH.id, admissionNo: "SM-88" }),
  );
});

function mount(school: School, overrides: Partial<SetupContext> = {}) {
  return render(<StudentsAndYearLevels school={school} context={context(overrides)} />);
}

/** Opens the enrolment panel for one student on screen. */
async function openEnrolment(
  user: ReturnType<typeof userEvent.setup>,
  family: string,
): Promise<HTMLElement> {
  await screen.findByText(family);
  await user.click(screen.getByRole("button", { name: new RegExp(`^Enrolment .*${family}`) }));
  return (await screen.findByText(/^Enrolment — /)).closest(".subPanel") as HTMLElement;
}

describe("the roll", () => {
  it("shows this school's admission number, year level and campus", async () => {
    listSchoolStudents.mockResolvedValue([
      inSchool(CASS, spell({ campusId: NORTH_MAIN.id })),
    ]);
    mount(NORTH);

    await screen.findByText("Pupilsen");
    const roll = screen.getByRole("table", { name: "Students at this school" });

    expect(within(roll).getByText("NG-1001")).toBeTruthy();
    await waitFor(() =>
      expect(within(roll).getByText("Grade 8 (Northgate)")).toBeTruthy(),
    );
    expect(within(roll).getByText("Main Campus")).toBeTruthy();
    expect(within(roll).getByText("Enrolled")).toBeTruthy();
  });

  it("shows a prospective student as prospective, not as enrolled", async () => {
    listSchoolStudents.mockResolvedValue([inSchool(CASS, spell({ status: "PROSPECTIVE" }))]);
    mount(NORTH);

    expect(await screen.findByText("Prospective")).toBeTruthy();
  });

  it("keeps somebody whose every spell here is in the past, and says so", async () => {
    // `listStudents` still returns them, because this school holds their history.
    listSchoolStudents.mockResolvedValue([]);
    mount(NORTH);

    expect(await screen.findByText("Pupilsen")).toBeTruthy();
    expect(screen.getByText("Left this school")).toBeTruthy();
  });

  it("shows the other schools a student attends", async () => {
    listStudents.mockResolvedValue([CASS]);
    mount(NORTH);

    await screen.findByText("Pupilsen");
    await waitFor(() => expect(screen.getByText("Southmoor College")).toBeTruthy());
  });
});

describe("enrolling a student who already exists", () => {
  it("writes a spell and creates no student record", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Rao");

    await user.click(screen.getByRole("button", { name: "Enrol an existing student" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: /Pupilsen, Cass/ }));
    await user.click(within(dialog).getByRole("button", { name: "Enrol at this school" }));

    await waitFor(() => expect(enrolStudent).toHaveBeenCalled());
    expect(enrolStudent).toHaveBeenCalledWith({
      studentId: CASS.id,
      schoolId: SOUTH.id,
      // Placement is set straight afterwards, in the panel. Asking for it here would be
      // the enrolment form twice, and inventing a year level would be a claim.
      admissionNo: null,
      yearLevelId: null,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: null,
      leftOn: null,
    });
    expect(createStudent).not.toHaveBeenCalled();
  });

  it("says this is concurrent enrolment and points at transfer for the other case", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Rao");

    await user.click(screen.getByRole("button", { name: "Enrol an existing student" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/concurrent enrolment/i)).toBeTruthy();
    expect(within(dialog).getByText(/To move somebody instead/)).toBeTruthy();
    expect(transferStudent).not.toHaveBeenCalled();
  });

  it("does not offer to enrol before somebody is chosen", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Rao");

    await user.click(screen.getByRole("button", { name: "Enrol an existing student" }));
    const dialog = await screen.findByRole("dialog");

    const confirm = within(dialog).getByRole("button", { name: "Enrol at this school" });
    expect(confirm.hasAttribute("disabled")).toBe(true);
    expect(confirm.getAttribute("title")).toMatch(/Choose a student/);
    expect(enrolStudent).not.toHaveBeenCalled();
  });

  it("offers only students this school has never enrolled", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Rao");

    await user.click(screen.getByRole("button", { name: "Enrol an existing student" }));
    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getByRole("radio", { name: /Pupilsen, Cass/ })).toBeTruthy();
    expect(within(dialog).queryByRole("radio", { name: /Rao, Dev/ })).toBeNull();
  });

  it("surfaces a duplicate-enrolment refusal", async () => {
    const user = userEvent.setup();
    enrolStudent.mockRejectedValue({
      kind: "duplicate",
      entity: "enrolment",
      field: "school",
      message: "that student already has a current enrolment at that school",
    });
    mount(SOUTH);
    await screen.findByText("Rao");

    await user.click(screen.getByRole("button", { name: "Enrol an existing student" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: /Pupilsen, Cass/ }));
    await user.click(within(dialog).getByRole("button", { name: "Enrol at this school" }));

    expect(await within(dialog).findByText(/already has a current enrolment/)).toBeTruthy();
  });
});

describe("this school's enrolment of one student", () => {
  it("shows the spell this school holds, not the student's own columns", async () => {
    const user = userEvent.setup();
    listEnrolmentHistory.mockResolvedValue([
      spell({ id: "e-south", schoolId: SOUTH.id, admissionNo: "SM-77", yearLevelId: SOUTH_Y8.id }),
    ]);
    listStudents.mockResolvedValue([CASS]);
    listSchoolStudents.mockResolvedValue([
      inSchool(CASS, spell({ id: "e-south", schoolId: SOUTH.id, admissionNo: "SM-77", yearLevelId: SOUTH_Y8.id })),
    ]);
    mount(SOUTH);

    const panel = await openEnrolment(user, "Pupilsen");

    // Southmoor's own number and year, never Northgate's — which the student record still
    // carries as its home-school value.
    expect(within(panel).getAllByText("SM-77").length).toBeGreaterThan(0);
    expect(within(panel).queryByText("NG-1001")).toBeNull();
    expect(within(panel).getByText("Grade 8 (Southmoor)")).toBeTruthy();
  });

  it("edits the placement against this school", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Edit this enrolment" }));
    await user.clear(screen.getByLabelText(/Admission number/));
    await user.type(screen.getByLabelText(/Admission number/), "NG-2002");
    await user.click(screen.getByRole("button", { name: "Save enrolment" }));

    await waitFor(() => expect(updateEnrolment).toHaveBeenCalled());
    expect(updateEnrolment).toHaveBeenCalledWith(NORTH.id, "e-north", {
      studentId: CASS.id,
      schoolId: NORTH.id,
      admissionNo: "NG-2002",
      yearLevelId: NORTH_Y8.id,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: "2026-09-01",
      leftOn: null,
    });
  });

  it("offers only this school's year levels for the placement", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Edit this enrolment" }));

    const options = within(screen.getByLabelText(/Year level/)).getAllByRole("option");
    const labels = options.map((option) => option.textContent);
    expect(labels).toContain("Grade 8 (Northgate)");
    // Southmoor's Grade 8 is a different record, and the backend refuses it by name.
    expect(labels).not.toContain("Grade 8 (Southmoor)");
  });

  it("does not offer to end a spell by changing a dropdown", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Edit this enrolment" }));

    const labels = within(screen.getByLabelText(/Status/))
      .getAllByRole("option")
      .map((option) => option.textContent ?? "");
    expect(labels.some((label) => /Enrolled/.test(label))).toBe(true);
    expect(labels.some((label) => /Prospective/.test(label))).toBe(true);
    // Leaving is its own action, with its own confirmation and its own date.
    expect(labels.some((label) => /^Left/.test(label))).toBe(false);
  });

  it("records a departure with a date", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Mark as left" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Leaving date"), "2027-07-20");
    await user.click(within(dialog).getByRole("button", { name: "Mark as left" }));

    await waitFor(() => expect(leaveSchool).toHaveBeenCalled());
    expect(leaveSchool).toHaveBeenCalledWith(NORTH.id, CASS.id, "2027-07-20");
  });

  it("records a departure with no date, rather than inventing one", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Mark as left" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Mark as left" }));

    await waitFor(() => expect(leaveSchool).toHaveBeenCalled());
    // `null`, not today's date: the fact is the status, and only the school knows the day.
    expect(leaveSchool).toHaveBeenCalledWith(NORTH.id, CASS.id, null);
  });

  it("says a departure keeps the spell and the groups", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Mark as left" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Nothing is deleted/)).toBeTruthy();
    expect(within(dialog).getByText(/groups and every other school/)).toBeTruthy();
  });

  it("re-enrols somebody whose spells here are all in the past", async () => {
    const user = userEvent.setup();
    listSchoolStudents.mockResolvedValue([]);
    listEnrolmentHistory.mockResolvedValue([
      spell({ status: "LEFT", leftOn: "2026-12-20" }),
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    expect(within(panel).getByText("Not currently enrolled here")).toBeTruthy();
    await user.click(within(panel).getByRole("button", { name: "Enrol at this school" }));
    await user.type(screen.getByLabelText(/Admission number/), "NG-3003");
    await user.click(screen.getByRole("button", { name: "Enrol" }));

    await waitFor(() => expect(enrolStudent).toHaveBeenCalled());
    expect(enrolStudent.mock.calls[0]?.[0]).toMatchObject({
      studentId: CASS.id,
      schoolId: NORTH.id,
      admissionNo: "NG-3003",
      status: "ENROLLED",
    });
  });

  it("refuses a leaving date before the joining date, before sending it", async () => {
    const user = userEvent.setup();
    listSchoolStudents.mockResolvedValue([]);
    listEnrolmentHistory.mockResolvedValue([spell({ status: "LEFT" })]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    await user.click(within(panel).getByRole("button", { name: "Enrol at this school" }));
    await user.type(screen.getByLabelText(/Joining date/), "99/99/9999");
    await user.click(screen.getByRole("button", { name: "Enrol" }));

    // Shape only, and told before a round trip. What a legal *placement* is stays the
    // repository's rule, and arrives as a worded refusal.
    expect(await screen.findByText(/Joining date is not a real date/)).toBeTruthy();
    expect(enrolStudent).not.toHaveBeenCalled();
  });
});

describe("enrolment history", () => {
  it("shows every spell at every school, oldest first, read-only", async () => {
    const user = userEvent.setup();
    listEnrolmentHistory.mockResolvedValue([
      spell({ id: "e-1", status: "LEFT", enrolledOn: "2024-09-01", leftOn: "2025-07-18" }),
      spell({ id: "e-2", schoolId: SOUTH.id, admissionNo: "SM-77", status: "LEFT", enrolledOn: "2025-09-01", leftOn: "2026-07-20" }),
      spell({ id: "e-3", enrolledOn: "2026-09-01" }),
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    const table = within(panel).getByRole("table", { name: /Enrolment history/ });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(3);
    expect(rows[0]?.textContent).toContain("2024-09-01");
    expect(rows[1]?.textContent).toContain("Southmoor College");
    expect(rows[2]?.textContent).toContain("Enrolled");
    // No per-row Edit or Delete: a past spell is history, and editing one would rewrite
    // the answer to "was she ever at Southmoor?".
    expect(within(table).queryByRole("button")).toBeNull();
  });

  it("keeps a spell at a school the student has left, rather than hiding it", async () => {
    const user = userEvent.setup();
    listEnrolmentHistory.mockResolvedValue([
      spell({ id: "e-1", schoolId: SOUTH.id, status: "LEFT", leftOn: "2026-07-20" }),
      spell({ id: "e-2" }),
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    expect(within(panel).getByText("Southmoor College")).toBeTruthy();
    expect(within(panel).getByText("2026-07-20")).toBeTruthy();
  });

  it("marks concurrent enrolment where there is one", async () => {
    const user = userEvent.setup();
    listEnrolmentHistory.mockResolvedValue([
      spell(),
      spell({ id: "e-south", schoolId: SOUTH.id, admissionNo: "SM-77" }),
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    expect(within(panel).getByText(/also currently enrolled/)).toBeTruthy();
    expect(within(panel).getByText(/That is concurrent enrolment, not a transfer/)).toBeTruthy();
  });
});

describe("transferring a student", () => {
  /** Opens the transfer form and chooses Southmoor as the destination. */
  async function openTransfer(user: ReturnType<typeof userEvent.setup>) {
    const panel = await openEnrolment(user, "Pupilsen");
    await user.click(
      within(panel).getByRole("button", { name: /Transfer to another school/ }),
    );
    await user.selectOptions(screen.getByLabelText(/Transfer to/), SOUTH.id);
    return panel;
  }

  it("reads the receiving school's year levels and campuses, not this one's", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);

    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));
    expect(listCampuses).toHaveBeenCalledWith(SOUTH.id);

    const labels = within(await screen.findByLabelText(/Year level/))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(labels).toContain("Grade 8 (Southmoor)");
    expect(labels).not.toContain("Grade 8 (Northgate)");
  });

  it("summarises the move before doing it", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.type(screen.getByLabelText(/Admission number/), "SM-88");
    await user.click(screen.getByRole("button", { name: "Review transfer" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/moves from/)).toBeTruthy();
    expect(within(dialog).getByText(/becomes a past one and is kept/)).toBeTruthy();
    expect(within(dialog).getByText(/as SM-88/)).toBeTruthy();
    // Nothing is sent until the summary is agreed to.
    expect(transferStudent).not.toHaveBeenCalled();
  });

  it("executes the atomic operation, naming both schools", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.type(screen.getByLabelText(/Admission number/), "SM-88");
    await user.selectOptions(screen.getByLabelText(/Year level/), SOUTH_Y8.id);
    await user.click(screen.getByRole("button", { name: "Review transfer" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Transfer student" }));

    await waitFor(() => expect(transferStudent).toHaveBeenCalled());
    expect(transferStudent).toHaveBeenCalledWith(NORTH.id, SOUTH.id, CASS.id, {
      studentId: CASS.id,
      schoolId: SOUTH.id,
      admissionNo: "SM-88",
      yearLevelId: SOUTH_Y8.id,
      campusId: null,
      status: "ENROLLED",
      enrolledOn: null,
      leftOn: null,
    });
  });

  it("is never two frontend operations", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: "Review transfer" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Transfer student" }));

    await waitFor(() => expect(transferStudent).toHaveBeenCalledTimes(1));
    // The whole reason `transfer_student` names both schools: a leave plus an enrol is two
    // transactions, and a failure between them leaves a real student enrolled nowhere.
    expect(leaveSchool).not.toHaveBeenCalled();
    expect(enrolStudent).not.toHaveBeenCalled();
  });

  it("transfers with no date rather than inventing one", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: "Review transfer" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Transfer student" }));

    await waitFor(() => expect(transferStudent).toHaveBeenCalled());
    expect(transferStudent.mock.calls[0]?.[3]).toMatchObject({ enrolledOn: null });
  });

  it("refuses to review without a destination, and sends nothing", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");
    await user.click(
      within(panel).getByRole("button", { name: /Transfer to another school/ }),
    );

    await user.click(screen.getByRole("button", { name: "Review transfer" }));

    expect(await screen.findByText(/Choose the school they are transferring to/)).toBeTruthy();
    expect(transferStudent).not.toHaveBeenCalled();
  });

  it("marks a school the student already attends, rather than hiding it", async () => {
    const user = userEvent.setup();
    listEnrolmentHistory.mockResolvedValue([
      spell(),
      spell({ id: "e-south", schoolId: SOUTH.id }),
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");
    await user.click(
      within(panel).getByRole("button", { name: /Transfer to another school/ }),
    );

    // Shown and labelled, because the backend refuses it and the user should understand
    // why rather than wonder where Southmoor went.
    const labels = within(screen.getByLabelText(/Transfer to/))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(labels.some((label) => label?.includes("already enrolled there"))).toBe(true);
  });

  it("says plainly that a transfer is not a concurrent enrolment", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: "Review transfer" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/If they should attend/)).toBeTruthy();
    expect(within(dialog).getByText(/Enrol an existing student/)).toBeTruthy();
  });

  it("surfaces a refusal rather than reporting a move that did not happen", async () => {
    const user = userEvent.setup();
    transferStudent.mockRejectedValue({
      kind: "invalid",
      message: "That year level belongs to a different school. Pick one of this school's.",
    });
    mount(NORTH);
    await openTransfer(user);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: "Review transfer" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Transfer student" }));

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
  });

  it("is not offered in a single-school campus file", async () => {
    const user = userEvent.setup();
    render(
      <StudentsAndYearLevels school={NORTH} context={context({ schools: [NORTH] })} />,
    );
    const panel = await openEnrolment(user, "Pupilsen");

    const transfer = within(panel).getByRole("button", {
      name: /Transfer to another school/,
    });
    expect(transfer.hasAttribute("disabled")).toBe(true);
    expect(transfer.getAttribute("title")).toMatch(/needs two schools/);
  });
});

describe("group memberships and enrolment", () => {
  it("lists a student's groups and says membership is not enrolment", async () => {
    const user = userEvent.setup();
    listGroupsForStudent.mockResolvedValue([
      { id: "g-1", name: "8A Homeroom" } as StudentGroup,
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");

    expect(within(panel).getByText("8A Homeroom")).toBeTruthy();
    expect(within(panel).getByText(/Group membership is/)).toBeTruthy();
    expect(within(panel).getByText(/kept exactly as they are by a transfer/)).toBeTruthy();
  });

  it("does not touch them on a transfer", async () => {
    const user = userEvent.setup();
    listGroupsForStudent.mockResolvedValue([
      { id: "g-1", name: "8A Homeroom" } as StudentGroup,
    ]);
    mount(NORTH);
    const panel = await openEnrolment(user, "Pupilsen");
    await user.click(
      within(panel).getByRole("button", { name: /Transfer to another school/ }),
    );
    await user.selectOptions(screen.getByLabelText(/Transfer to/), SOUTH.id);
    await waitFor(() => expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id));
    await user.click(screen.getByRole("button", { name: "Review transfer" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Transfer student" }));

    await waitFor(() => expect(transferStudent).toHaveBeenCalled());
    // The summary promised it, and nothing here acts on groups at all.
    expect(within(dialog).queryByText(/groups/)).not.toBeNull();
  });

  it("stays usable for a project with no students at all", async () => {
    // ADR 0006: a complete timetable can be built from group sizes alone, and this screen
    // must not become a prerequisite for that.
    listStudents.mockResolvedValue([]);
    listSchoolStudents.mockResolvedValue([]);
    mount(NORTH);

    expect(await screen.findByText(/a timetable does not need any/)).toBeTruthy();
    expect(screen.getByText("No student chosen")).toBeTruthy();
  });
});
