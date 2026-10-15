/**
 * Stage B: the Departments and Subjects screens.
 *
 * ## What is worth testing here
 *
 * The CRUD is `CrudPanel`'s and is already covered. What these two screens add, and what
 * only a test can hold in place, is the **scoping**:
 *
 * - every read names the selected school, and switching schools re-reads rather than
 *   filters — a screen that showed Northgate's subjects under Southmoor's heading is the
 *   failure the whole multi-school work exists to prevent;
 * - two schools may each hold "Mathematics" with the same code, as two records, and
 *   nothing merges them on a matching name;
 * - the department picker offers *this* school's departments and no others;
 * - a delete that is going to be refused is **explained before it is offered**, with the
 *   counts read from the backend rather than guessed in React.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Department,
  DepartmentDependents,
  QualifiedTeacher,
  School,
  Subject,
  SubjectDependents,
} from "@evara/domain";

const listDepartments = vi.fn<(schoolId: string) => Promise<Department[]>>();
const createDepartment = vi.fn<(input: unknown) => Promise<Department>>();
const updateDepartment =
  vi.fn<(id: string, rev: number, input: unknown) => Promise<Department>>();
const deleteDepartment = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const departmentDependents = vi.fn<(id: string) => Promise<DepartmentDependents>>();

const listSubjects = vi.fn<(schoolId: string) => Promise<Subject[]>>();
const createSubject = vi.fn<(input: unknown) => Promise<Subject>>();
const updateSubject = vi.fn<(id: string, rev: number, input: unknown) => Promise<Subject>>();
const deleteSubject = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const subjectDependents = vi.fn<(id: string) => Promise<SubjectDependents>>();
const listSubjectTeachers =
  vi.fn<(schoolId: string, subjectId: string) => Promise<QualifiedTeacher[]>>();

vi.mock("../../ipc/setup", () => ({
  listDepartments: (schoolId: string) => listDepartments(schoolId),
  createDepartment: (input: unknown) => createDepartment(input),
  updateDepartment: (id: string, rev: number, input: unknown) =>
    updateDepartment(id, rev, input),
  deleteDepartment: (schoolId: string, id: string, rev: number) =>
    deleteDepartment(schoolId, id, rev),
  departmentDependents: (id: string) => departmentDependents(id),
  listDepartmentTeachers: vi.fn(() => Promise.resolve([])),
  listSubjects: (schoolId: string) => listSubjects(schoolId),
  createSubject: (input: unknown) => createSubject(input),
  updateSubject: (id: string, rev: number, input: unknown) => updateSubject(id, rev, input),
  deleteSubject: (schoolId: string, id: string, rev: number) =>
    deleteSubject(schoolId, id, rev),
  subjectDependents: (id: string) => subjectDependents(id),
  listSubjectTeachers: (schoolId: string, subjectId: string) =>
    listSubjectTeachers(schoolId, subjectId),
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

const { Departments } = await import("./Departments");
const { Subjects } = await import("./Subjects");

/** Not 1: an assertion that a write carried the row's revision must be able to fail. */
const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 4,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function department(id: string, schoolId: string, name: string, code: string | null) {
  return { id, schoolId, name, code, description: null, ...STAMP } as Department;
}

function subject(
  id: string,
  schoolId: string,
  name: string,
  code: string | null,
  departmentId: string | null,
) {
  return { id, schoolId, name, code, departmentId, description: null, ...STAMP } as Subject;
}

const NORTH_MATHS_DEPT = department("d-n-maths", NORTH.id, "Mathematics", "MAT");
const NORTH_SCIENCE_DEPT = department("d-n-science", NORTH.id, "Science", null);
const SOUTH_MATHS_DEPT = department("d-s-maths", SOUTH.id, "Mathematics", "MAT");

const NORTH_SUBJECTS = [
  subject("s-n-maths", NORTH.id, "Mathematics", "MAT", NORTH_MATHS_DEPT.id),
  subject("s-n-physics", NORTH.id, "Physics", "PHY", NORTH_SCIENCE_DEPT.id),
  subject("s-n-history", NORTH.id, "History", null, null),
];
// Same name, same code, different school. Two records, by design.
const SOUTH_SUBJECTS = [subject("s-s-maths", SOUTH.id, "Mathematics", "MAT", null)];

const NOTHING: DepartmentDependents = { total: 0, subjects: 0, teachers: 0 };

beforeEach(() => {
  vi.clearAllMocks();
  listDepartments.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id ? [NORTH_MATHS_DEPT, NORTH_SCIENCE_DEPT] : [SOUTH_MATHS_DEPT],
    ),
  );
  createDepartment.mockImplementation((input) =>
    Promise.resolve(
      department(
        "d-new",
        (input as { schoolId: string }).schoolId,
        (input as { name: string }).name,
        (input as { code: string | null }).code,
      ),
    ),
  );
  updateDepartment.mockImplementation((id, _rev, input) =>
    Promise.resolve(
      department(
        id,
        (input as { schoolId: string }).schoolId,
        (input as { name: string }).name,
        (input as { code: string | null }).code,
      ),
    ),
  );
  deleteDepartment.mockResolvedValue(undefined);
  departmentDependents.mockResolvedValue(NOTHING);

  listSubjects.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? NORTH_SUBJECTS : SOUTH_SUBJECTS),
  );
  createSubject.mockImplementation((input) =>
    Promise.resolve(
      subject(
        "s-new",
        (input as { schoolId: string }).schoolId,
        (input as { name: string }).name,
        (input as { code: string | null }).code,
        (input as { departmentId: string | null }).departmentId,
      ),
    ),
  );
  updateSubject.mockImplementation((id, _rev, input) =>
    Promise.resolve(
      subject(
        id,
        (input as { schoolId: string }).schoolId,
        (input as { name: string }).name,
        (input as { code: string | null }).code,
        (input as { departmentId: string | null }).departmentId,
      ),
    ),
  );
  deleteSubject.mockResolvedValue(undefined);
  subjectDependents.mockResolvedValue({ total: 0, qualifiedTeachers: 0 });
  listSubjectTeachers.mockResolvedValue([]);
});

// ========================================================================= departments

describe("the Departments screen", () => {
  it("asks for the selected school's departments and no others", async () => {
    render(<Departments school={NORTH} />);

    await waitFor(() => expect(listDepartments).toHaveBeenCalled());
    for (const call of listDepartments.mock.calls) {
      expect(call[0]).toBe(NORTH.id);
    }
  });

  it("shows each department's name, code and subject count", async () => {
    render(<Departments school={NORTH} />);

    expect(await screen.findByText("Mathematics")).toBeTruthy();
    const table = screen.getByRole("table", { name: "Departments" });
    expect(within(table).getByText("MAT")).toBeTruthy();
    // Mathematics teaches one of Northgate's three subjects; Science teaches one too.
    const maths = within(table).getAllByRole("row")[1];
    expect(maths?.textContent).toContain("1");
  });

  it("re-reads on a switch and shows no trace of the school left behind", async () => {
    const view = render(<Departments school={NORTH} />);
    expect(await screen.findByText("Science")).toBeTruthy();

    view.rerender(<Departments school={SOUTH} />);

    await waitFor(() => expect(listDepartments).toHaveBeenCalledWith(SOUTH.id));
    await waitFor(() => expect(screen.queryByText("Science")).toBeNull());
  });

  it("names the selected school when a department is created", async () => {
    const user = userEvent.setup();
    render(<Departments school={NORTH} />);
    await screen.findByText("Mathematics");

    await user.click(screen.getByRole("button", { name: "Add department" }));
    await user.type(screen.getByLabelText(/^Name/), "Humanities");
    await user.type(screen.getByLabelText(/^Code/), "HUM");
    await user.click(screen.getByRole("button", { name: /^Add department$/ }));

    await waitFor(() => expect(createDepartment).toHaveBeenCalled());
    expect(createDepartment).toHaveBeenCalledWith({
      schoolId: NORTH.id,
      name: "Humanities",
      code: "HUM",
      description: null,
    });
  });

  it("sends a blank code as null rather than an empty string", async () => {
    const user = userEvent.setup();
    render(<Departments school={NORTH} />);
    await screen.findByText("Mathematics");

    await user.click(screen.getByRole("button", { name: "Add department" }));
    await user.type(screen.getByLabelText(/^Name/), "Humanities");
    await user.click(screen.getByRole("button", { name: /^Add department$/ }));

    await waitFor(() => expect(createDepartment).toHaveBeenCalled());
    // A partial unique index means `null` and `""` are not the same thing: an empty string
    // would collide with the next codeless department.
    const input = createDepartment.mock.calls[0]?.[0] as { code: unknown };
    expect(input.code).toBeNull();
  });

  it("requires a name and nothing else", async () => {
    const user = userEvent.setup();
    render(<Departments school={NORTH} />);
    await screen.findByText("Mathematics");

    await user.click(screen.getByRole("button", { name: "Add department" }));
    await user.click(screen.getByRole("button", { name: /^Add department$/ }));

    expect(await screen.findByText(/Department name is required/i)).toBeTruthy();
    expect(createDepartment).not.toHaveBeenCalled();
  });

  it("sends the row's revision when one is edited", async () => {
    const user = userEvent.setup();
    render(<Departments school={NORTH} />);
    await screen.findByText("Mathematics");

    await user.click(screen.getByRole("button", { name: "Edit Mathematics" }));
    await user.clear(screen.getByLabelText(/^Name/));
    await user.type(screen.getByLabelText(/^Name/), "Maths and Statistics");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateDepartment).toHaveBeenCalled());
    expect(updateDepartment).toHaveBeenCalledWith("d-n-maths", 4, {
      schoolId: NORTH.id,
      name: "Maths and Statistics",
      code: "MAT",
      description: null,
    });
  });

  it("explains a blocked delete before offering it, with the backend's counts", async () => {
    const user = userEvent.setup();
    departmentDependents.mockImplementation((id) =>
      Promise.resolve(
        id === NORTH_MATHS_DEPT.id
          ? { total: 5, subjects: 3, teachers: 2 }
          : NOTHING,
      ),
    );
    render(<Departments school={NORTH} />);
    await screen.findByText("Mathematics");
    await waitFor(() => expect(departmentDependents).toHaveBeenCalledWith("d-n-maths"));

    await user.click(screen.getByRole("button", { name: "Delete Mathematics" }));
    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getByText(/cannot be deleted/i)).toBeTruthy();
    expect(within(dialog).getByText(/3 subjects are taught by it/)).toBeTruthy();
    expect(within(dialog).getByText(/2 teachers belong to it/)).toBeTruthy();
    // Explained, not offered: no destructive button to press.
    expect(
      within(dialog).queryByRole("button", { name: /^Delete department$/ }),
    ).toBeNull();
    expect(deleteDepartment).not.toHaveBeenCalled();
  });

  it("offers the delete, with the revision, when nothing refers to it", async () => {
    const user = userEvent.setup();
    render(<Departments school={NORTH} />);
    await screen.findByText("Science");

    await user.click(screen.getByRole("button", { name: "Delete Science" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^Delete department$/ }));

    await waitFor(() => expect(deleteDepartment).toHaveBeenCalled());
    expect(deleteDepartment).toHaveBeenCalledWith(NORTH.id, "d-n-science", 4);
  });

  it("says departments are this school's own, and optional", async () => {
    listDepartments.mockResolvedValue([]);
    render(<Departments school={NORTH} />);

    expect(await screen.findByText(/belong to this school alone/)).toBeTruthy();
    expect(screen.getByText(/A department is optional/)).toBeTruthy();
  });
});

// ============================================================================ subjects

describe("the Subjects screen", () => {
  it("asks for the selected school's subjects and departments", async () => {
    render(<Subjects school={NORTH} />);

    await waitFor(() => expect(listSubjects).toHaveBeenCalled());
    expect(listSubjects).toHaveBeenCalledWith(NORTH.id);
    expect(listDepartments).toHaveBeenCalledWith(NORTH.id);
    for (const call of [...listSubjects.mock.calls, ...listDepartments.mock.calls]) {
      expect(call[0]).toBe(NORTH.id);
    }
  });

  it("shows each subject's name, code and department", async () => {
    render(<Subjects school={NORTH} />);

    expect(await screen.findByText("Physics")).toBeTruthy();
    const table = screen.getByRole("table", { name: "Subjects" });
    expect(within(table).getByText("PHY")).toBeTruthy();
    expect(within(table).getByText("Science")).toBeTruthy();
    // A subject with no department says so rather than showing a blank cell.
    expect(within(table).getByText("No department")).toBeTruthy();
  });

  it("keeps two schools' Mathematics as two separate subjects", async () => {
    const view = render(<Subjects school={NORTH} />);
    expect(await screen.findByText("Physics")).toBeTruthy();

    view.rerender(<Subjects school={SOUTH} />);

    // Southmoor's own Mathematics, with the same code, and nothing of Northgate's.
    await waitFor(() => expect(screen.queryByText("Physics")).toBeNull());
    expect(await screen.findByText("Mathematics")).toBeTruthy();
    expect(screen.getAllByText("Mathematics")).toHaveLength(1);
    expect(listSubjects).toHaveBeenCalledWith(SOUTH.id);
  });

  it("offers only this school's departments in the picker", async () => {
    const user = userEvent.setup();
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Add subject" }));
    const picker = screen.getByLabelText(/^Department/) as HTMLSelectElement;
    const offered = [...picker.options].map((option) => option.value);

    expect(offered).toContain(NORTH_MATHS_DEPT.id);
    expect(offered).toContain(NORTH_SCIENCE_DEPT.id);
    expect(offered).not.toContain(SOUTH_MATHS_DEPT.id);
    // And "no department" is a real choice, because the field is optional.
    expect(offered).toContain("");
  });

  it("creates a subject with no department when none is chosen", async () => {
    const user = userEvent.setup();
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Add subject" }));
    await user.type(screen.getByLabelText(/Subject name/), "Chemistry");
    await user.click(screen.getByRole("button", { name: /^Add subject$/ }));

    await waitFor(() => expect(createSubject).toHaveBeenCalled());
    expect(createSubject).toHaveBeenCalledWith({
      schoolId: NORTH.id,
      departmentId: null,
      name: "Chemistry",
      code: null,
      description: null,
    });
  });

  it("assigns a department when one is chosen", async () => {
    const user = userEvent.setup();
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Add subject" }));
    await user.type(screen.getByLabelText(/Subject name/), "Chemistry");
    await user.selectOptions(screen.getByLabelText(/^Department/), NORTH_SCIENCE_DEPT.id);
    await user.click(screen.getByRole("button", { name: /^Add subject$/ }));

    await waitFor(() => expect(createSubject).toHaveBeenCalled());
    const input = createSubject.mock.calls[0]?.[0] as { departmentId: unknown };
    expect(input.departmentId).toBe(NORTH_SCIENCE_DEPT.id);
  });

  it("lets a department be cleared back to none", async () => {
    const user = userEvent.setup();
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Edit Physics" }));
    await user.selectOptions(screen.getByLabelText(/^Department/), "");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSubject).toHaveBeenCalled());
    expect(updateSubject).toHaveBeenCalledWith("s-n-physics", 4, {
      schoolId: NORTH.id,
      departmentId: null,
      name: "Physics",
      code: "PHY",
      description: null,
    });
  });

  it("advises rather than blocks when the school has no departments", async () => {
    const user = userEvent.setup();
    listDepartments.mockResolvedValue([]);
    render(<Subjects school={NORTH} context={{ openSection: vi.fn() } as never} />);
    await screen.findByText("Physics");

    expect(screen.getByText(/no departments yet/i)).toBeTruthy();
    expect(screen.getByText(/a department is optional/i)).toBeTruthy();

    // The table is not hidden and a subject can still be added, which is the difference
    // between advice and a prerequisite.
    await user.click(screen.getByRole("button", { name: "Add subject" }));
    await user.type(screen.getByLabelText(/Subject name/), "Chemistry");
    await user.click(screen.getByRole("button", { name: /^Add subject$/ }));
    await waitFor(() => expect(createSubject).toHaveBeenCalled());
  });

  it("refuses a delete while somebody is qualified, and says how many", async () => {
    const user = userEvent.setup();
    subjectDependents.mockImplementation((id) =>
      Promise.resolve(
        id === "s-n-maths"
          ? { total: 4, qualifiedTeachers: 4 }
          : { total: 0, qualifiedTeachers: 0 },
      ),
    );
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");
    await waitFor(() => expect(subjectDependents).toHaveBeenCalledWith("s-n-maths"));

    await user.click(screen.getByRole("button", { name: "Delete Mathematics" }));
    const dialog = await screen.findByRole("dialog");

    expect(within(dialog).getByText(/4 teachers are qualified to teach it/)).toBeTruthy();
    expect(within(dialog).getByText(/record about a person/)).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: /^Delete subject$/ })).toBeNull();
  });

  it("shows who is qualified for a chosen subject", async () => {
    const user = userEvent.setup();
    listSubjectTeachers.mockResolvedValue([
      {
        teacher: {
          id: "t-1",
          givenName: "Ada",
          familyName: "Okafor",
          code: "AO1",
          isActive: true,
        },
        qualification: { id: "q-1", rev: 1 },
      } as unknown as QualifiedTeacher,
    ]);
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Qualified teachers Mathematics" }));

    await waitFor(() =>
      expect(listSubjectTeachers).toHaveBeenCalledWith(NORTH.id, "s-n-maths"),
    );
    expect(await screen.findByText("Okafor")).toBeTruthy();
    expect(screen.getByText("AO1")).toBeTruthy();
  });

  it("explains where to add a qualification when nobody is qualified", async () => {
    const user = userEvent.setup();
    render(<Subjects school={NORTH} />);
    await screen.findByText("Physics");

    await user.click(screen.getByRole("button", { name: "Qualified teachers Mathematics" }));

    expect(
      await screen.findByText(/Nobody is qualified for this subject yet/),
    ).toBeTruthy();
    // And says plainly that a matching name implies nothing, which is the multi-school
    // misunderstanding this screen is most likely to produce.
    expect(screen.getByText(/Nothing is implied by a name/)).toBeTruthy();
  });

  it("says subjects belong to this school alone when there are none", async () => {
    listSubjects.mockResolvedValue([]);
    render(<Subjects school={NORTH} />);

    expect(await screen.findByText(/belong to this school alone/)).toBeTruthy();
  });
});
