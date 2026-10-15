/**
 * Stage B: a teacher's departments and subject qualifications.
 *
 * ## What is worth testing here
 *
 * The panel's whole job is to keep two kinds of fact visibly apart, and the distinction is
 * the thing a future change is most likely to erode. So what is asserted is the semantics
 * rather than the markup:
 *
 * - a **department** is a narrowing of one school's employment, and only that school may
 *   change it;
 * - a **qualification** is a property of the person, so the list is project-wide, grouped
 *   by the school that owns the subject, and a row at another school is shown and *not*
 *   silently editable;
 * - the subject selector offers only this school's subjects, because a subject of the same
 *   name elsewhere is a different record;
 * - a removal is explicit, confirmed, and carries the row's own revision;
 * - and the confirmation says what is *not* changed, because that is the half a user has
 *   no other way to find out.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Department,
  QualifiedSubject,
  School,
  Subject,
  Teacher,
  TeacherDepartmentView,
} from "@evara/domain";

const listDepartments = vi.fn<(schoolId: string) => Promise<Department[]>>();
const listSubjects = vi.fn<(schoolId: string) => Promise<Subject[]>>();
const listTeacherDepartments = vi.fn<(teacherId: string) => Promise<TeacherDepartmentView[]>>();
const listQualifications = vi.fn<(teacherId: string) => Promise<QualifiedSubject[]>>();
const addTeacherDepartment =
  vi.fn<(schoolId: string, teacherId: string, departmentId: string) => Promise<unknown>>();
const removeTeacherDepartment =
  vi.fn<
    (schoolId: string, teacherId: string, departmentId: string, rev: number) => Promise<void>
  >();
const addQualification =
  vi.fn<(schoolId: string, teacherId: string, subjectId: string) => Promise<unknown>>();
const removeQualification =
  vi.fn<
    (schoolId: string, teacherId: string, subjectId: string, rev: number) => Promise<void>
  >();

vi.mock("../../ipc/setup", () => ({
  listDepartments: (schoolId: string) => listDepartments(schoolId),
  listSubjects: (schoolId: string) => listSubjects(schoolId),
  listTeacherDepartments: (teacherId: string) => listTeacherDepartments(teacherId),
  listQualifications: (teacherId: string) => listQualifications(teacherId),
  addTeacherDepartment: (schoolId: string, teacherId: string, departmentId: string) =>
    addTeacherDepartment(schoolId, teacherId, departmentId),
  removeTeacherDepartment: (
    schoolId: string,
    teacherId: string,
    departmentId: string,
    rev: number,
  ) => removeTeacherDepartment(schoolId, teacherId, departmentId, rev),
  addQualification: (schoolId: string, teacherId: string, subjectId: string) =>
    addQualification(schoolId, teacherId, subjectId),
  removeQualification: (
    schoolId: string,
    teacherId: string,
    subjectId: string,
    rev: number,
  ) => removeQualification(schoolId, teacherId, subjectId, rev),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
}));

const { TeacherCurriculum } = await import("./TeacherCurriculum");

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 4,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;
const SCHOOLS = [NORTH, SOUTH];

const ADA = {
  id: "t-ada",
  schoolId: NORTH.id,
  givenName: "Ada",
  familyName: "Okafor",
  code: "AO1",
  isActive: true,
  ...STAMP,
} as Teacher;

function department(id: string, schoolId: string, name: string, code: string | null) {
  return { id, schoolId, name, code, description: null, ...STAMP } as Department;
}

function subject(id: string, schoolId: string, name: string, code: string | null) {
  return {
    id,
    schoolId,
    name,
    code,
    departmentId: null,
    description: null,
    ...STAMP,
  } as Subject;
}

const NORTH_MATHS_DEPT = department("d-n-maths", NORTH.id, "Mathematics", "MAT");
const NORTH_PHYSICS_DEPT = department("d-n-physics", NORTH.id, "Physics", null);
const SOUTH_SCIENCE_DEPT = department("d-s-science", SOUTH.id, "Science", null);

const NORTH_MATHS = subject("s-n-maths", NORTH.id, "Mathematics", "MAT");
const NORTH_CHEM = subject("s-n-chem", NORTH.id, "Chemistry", "CHM");
// Same name, same code, at the other school. A different record, by design.
const SOUTH_MATHS = subject("s-s-maths", SOUTH.id, "Mathematics", "MAT");

function held(view: { id: string; department: Department; rev?: number }) {
  return {
    association: { id: view.id, teacherId: ADA.id, departmentId: view.department.id, ...STAMP },
    department: view.department,
  } as TeacherDepartmentView;
}

function qualified(id: string, of: Subject, departmentName: string | null = null) {
  return {
    qualification: { id, teacherId: ADA.id, subjectId: of.id, ...STAMP },
    subject: of,
    departmentName,
  } as QualifiedSubject;
}

beforeEach(() => {
  vi.clearAllMocks();
  listDepartments.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id ? [NORTH_MATHS_DEPT, NORTH_PHYSICS_DEPT] : [SOUTH_SCIENCE_DEPT],
    ),
  );
  listSubjects.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_MATHS, NORTH_CHEM] : [SOUTH_MATHS]),
  );
  listTeacherDepartments.mockResolvedValue([]);
  listQualifications.mockResolvedValue([]);
  addTeacherDepartment.mockResolvedValue({});
  removeTeacherDepartment.mockResolvedValue(undefined);
  addQualification.mockResolvedValue({});
  removeQualification.mockResolvedValue(undefined);
});

function mount(school: School = NORTH, teacher: Teacher | null = ADA) {
  return render(
    <TeacherCurriculum
      school={school}
      schools={SCHOOLS}
      teacher={teacher}
      onSelectSchool={vi.fn()}
      onChanged={vi.fn()}
    />,
  );
}

// ============================================================================ the shell

describe("before a teacher is chosen", () => {
  it("says what to do rather than showing two empty tables", async () => {
    mount(NORTH, null);

    expect(await screen.findByText("Choose a teacher above")).toBeTruthy();
    // And asks for nothing, because there is nobody to ask about.
    expect(listTeacherDepartments).not.toHaveBeenCalled();
    expect(listQualifications).not.toHaveBeenCalled();
  });
});

describe("the panel's framing", () => {
  it("says plainly that the two kinds of fact are different", async () => {
    mount();

    expect(
      await screen.findByText(/says where they sit in one school/i),
    ).toBeTruthy();
    expect(screen.getByText(/what they can teach and outlives it/i)).toBeTruthy();
  });
});

// ========================================================================= departments

describe("a teacher's departments", () => {
  it("reads them project-wide, for the person rather than the school", async () => {
    mount();

    await waitFor(() => expect(listTeacherDepartments).toHaveBeenCalledWith(ADA.id));
    // Not scoped by school: the whole point is that Mathematics at one and Science at
    // another are both true.
    expect(listTeacherDepartments).toHaveBeenCalledTimes(1);
  });

  it("shows this school's and another school's, labelled differently", async () => {
    listTeacherDepartments.mockResolvedValue([
      held({ id: "td-1", department: NORTH_MATHS_DEPT }),
      held({ id: "td-2", department: SOUTH_SCIENCE_DEPT }),
    ]);
    mount();

    const table = await screen.findByRole("table", {
      name: /Departments this teacher belongs to/i,
    });
    expect(within(table).getByText("Mathematics")).toBeTruthy();
    expect(within(table).getByText("Science")).toBeTruthy();
    expect(within(table).getByText("This school")).toBeTruthy();
    expect(within(table).getByText("Southmoor College")).toBeTruthy();
  });

  it("offers Remove on this school's row and a switch on another school's", async () => {
    listTeacherDepartments.mockResolvedValue([
      held({ id: "td-1", department: NORTH_MATHS_DEPT }),
      held({ id: "td-2", department: SOUTH_SCIENCE_DEPT }),
    ]);
    mount();
    await screen.findByText("Science");

    expect(screen.getAllByRole("button", { name: "Remove" })).toHaveLength(1);
    expect(
      screen.getByRole("button", { name: /Go to Southmoor College/ }),
    ).toBeTruthy();
  });

  it("offers only this school's departments, and not ones already held", async () => {
    listTeacherDepartments.mockResolvedValue([
      held({ id: "td-1", department: NORTH_MATHS_DEPT }),
    ]);
    mount();
    await screen.findByText("Mathematics");

    const picker = screen.getByLabelText("Department to add") as HTMLSelectElement;
    const offered = [...picker.options].map((option) => option.value).filter(Boolean);

    expect(offered).toEqual([NORTH_PHYSICS_DEPT.id]);
    expect(offered).not.toContain(SOUTH_SCIENCE_DEPT.id);
  });

  it("adds one, naming the acting school", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Not in any department");

    await user.selectOptions(screen.getByLabelText("Department to add"), NORTH_MATHS_DEPT.id);
    await user.click(screen.getByRole("button", { name: "Add to department" }));

    await waitFor(() => expect(addTeacherDepartment).toHaveBeenCalled());
    expect(addTeacherDepartment).toHaveBeenCalledWith(
      NORTH.id,
      ADA.id,
      NORTH_MATHS_DEPT.id,
    );
  });

  it("removes one after confirming, with the association's own revision", async () => {
    const user = userEvent.setup();
    listTeacherDepartments.mockResolvedValue([
      held({ id: "td-1", department: NORTH_MATHS_DEPT }),
    ]);
    mount();
    await screen.findByText("Mathematics");

    await user.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    // The confirmation says what is *not* changed, which is the half a user cannot
    // otherwise discover.
    expect(within(dialog).getByText(/every subject they are qualified to teach is untouched/i))
      .toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Remove from department" }));

    await waitFor(() => expect(removeTeacherDepartment).toHaveBeenCalled());
    expect(removeTeacherDepartment).toHaveBeenCalledWith(
      NORTH.id,
      ADA.id,
      NORTH_MATHS_DEPT.id,
      4,
    );
  });

  it("says when the school has no departments to offer", async () => {
    listDepartments.mockResolvedValue([]);
    mount();

    expect(await screen.findByText(/has no departments yet/)).toBeTruthy();
  });

  it("surfaces a backend refusal rather than appearing to have worked", async () => {
    const user = userEvent.setup();
    addTeacherDepartment.mockRejectedValue({
      kind: "invalid",
      message: "Add this teacher to the school before putting them in one of its departments.",
    });
    mount();
    await screen.findByText("Not in any department");

    await user.selectOptions(screen.getByLabelText("Department to add"), NORTH_MATHS_DEPT.id);
    await user.click(screen.getByRole("button", { name: "Add to department" }));

    expect(
      await screen.findByText(/Add this teacher to the school before/),
    ).toBeTruthy();
  });

  it("says that an association needs employment, and that only this school's are offered", async () => {
    mount();

    expect(
      await screen.findByText(/must already employ them/i),
    ).toBeTruthy();
  });
});

// ===================================================================== qualifications

describe("a teacher's subject qualifications", () => {
  it("reads them project-wide and groups them by the school that owns the subject", async () => {
    listQualifications.mockResolvedValue([
      qualified("q-1", NORTH_MATHS, "Mathematics"),
      qualified("q-2", SOUTH_MATHS),
    ]);
    mount();

    await waitFor(() => expect(listQualifications).toHaveBeenCalledWith(ADA.id));
    // Two groups, each naming its school — so a qualification at Southmoor is never shown
    // under Northgate's heading, however identical the subject's name.
    expect(await screen.findByText(/Northgate Secondary — this school/)).toBeTruthy();
    expect(screen.getByText("Southmoor College")).toBeTruthy();
  });

  it("puts this school's group first, whatever order the read returned", async () => {
    listQualifications.mockResolvedValue([
      qualified("q-2", SOUTH_MATHS),
      qualified("q-1", NORTH_MATHS),
    ]);
    mount();

    const headings = (await screen.findAllByRole("heading", { level: 5 })).map(
      (node) => node.textContent ?? "",
    );
    expect(headings[0]).toContain("Northgate Secondary");
  });

  it("offers Remove on this school's subject and a switch on another school's", async () => {
    listQualifications.mockResolvedValue([
      qualified("q-1", NORTH_MATHS),
      qualified("q-2", SOUTH_MATHS),
    ]);
    mount();
    await screen.findByRole("heading", { level: 5, name: /Southmoor College/ });

    // One Remove, for Northgate's Mathematics. Southmoor's is visible and not editable —
    // ADR 0023's rule, applied to a row this school can see but does not own.
    expect(screen.getAllByRole("button", { name: "Remove" })).toHaveLength(1);
    expect(screen.getByRole("button", { name: /Go to Southmoor College/ })).toBeTruthy();
  });

  it("shows the department a subject sits in, and says so when it has none", async () => {
    listQualifications.mockResolvedValue([
      qualified("q-1", NORTH_MATHS, "Mathematics"),
      qualified("q-2", NORTH_CHEM, null),
    ]);
    mount();

    expect(await screen.findByText("Chemistry")).toBeTruthy();
    expect(screen.getByText("No department")).toBeTruthy();
  });

  it("offers only this school's subjects, and not ones already qualified for", async () => {
    listQualifications.mockResolvedValue([qualified("q-1", NORTH_MATHS)]);
    mount();
    await screen.findByText(/Northgate Secondary — this school/);

    const picker = screen.getByLabelText("Subject to qualify for") as HTMLSelectElement;
    const offered = [...picker.options].map((option) => option.value).filter(Boolean);

    expect(offered).toEqual([NORTH_CHEM.id]);
    // Southmoor's Mathematics shares a name and a code and is still not on offer here.
    expect(offered).not.toContain(SOUTH_MATHS.id);
  });

  it("filters the selector by name, code or department", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("No subjects recorded yet");

    await user.type(screen.getByLabelText("Find a subject"), "chm");

    const picker = screen.getByLabelText("Subject to qualify for") as HTMLSelectElement;
    const offered = [...picker.options].map((option) => option.value).filter(Boolean);
    expect(offered).toEqual([NORTH_CHEM.id]);
  });

  it("says when nothing matches the search", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("No subjects recorded yet");

    await user.type(screen.getByLabelText("Find a subject"), "Astrophysics");

    expect(await screen.findByText(/No subject of Northgate Secondary matches/)).toBeTruthy();
  });

  it("adds one, naming the acting school", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("No subjects recorded yet");

    await user.selectOptions(screen.getByLabelText("Subject to qualify for"), NORTH_MATHS.id);
    await user.click(screen.getByRole("button", { name: "Add qualification" }));

    await waitFor(() => expect(addQualification).toHaveBeenCalled());
    expect(addQualification).toHaveBeenCalledWith(NORTH.id, ADA.id, NORTH_MATHS.id);
  });

  it("removes one only after an explicit confirmation, with its own revision", async () => {
    const user = userEvent.setup();
    listQualifications.mockResolvedValue([qualified("q-1", NORTH_MATHS)]);
    mount();
    await screen.findByText(/Northgate Secondary — this school/);

    await user.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/not their employment/i)).toBeTruthy();
    expect(
      within(dialog).getByText(/including one for a subject of the same name/i),
    ).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Remove qualification" }));

    await waitFor(() => expect(removeQualification).toHaveBeenCalled());
    expect(removeQualification).toHaveBeenCalledWith(NORTH.id, ADA.id, NORTH_MATHS.id, 4);
  });

  it("does not remove anything if the confirmation is cancelled", async () => {
    const user = userEvent.setup();
    listQualifications.mockResolvedValue([qualified("q-1", NORTH_MATHS)]);
    mount();
    await screen.findByText(/Northgate Secondary — this school/);

    await user.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^Cancel$/ }));

    expect(removeQualification).not.toHaveBeenCalled();
  });

  it("says that a qualification outlives employment, and lists everything that does not clear it", async () => {
    mount();

    // The sentence is broken up by a `<strong>`, so the assertion is on the whole note
    // rather than on the fragment the text matcher happens to land in.
    const fragment = await screen.findByText(/outlives one/i);
    const note = fragment.closest("p") ?? fragment;
    expect(note.textContent).toMatch(/deactivating this teacher/i);
    expect(note.textContent).toMatch(/removing a school from them/i);
    expect(note.textContent).toMatch(/changing their campuses/i);
    expect(note.textContent).toMatch(/editing their name/i);
  });

  it("shows a qualification at a school the teacher no longer works at", async () => {
    // The row somebody would otherwise enter a second time. It is shown, grouped under the
    // school that owns the subject, and is not editable from here.
    listQualifications.mockResolvedValue([qualified("q-2", SOUTH_MATHS)]);
    mount();

    expect(
      await screen.findByRole("heading", { level: 5, name: /Southmoor College/ }),
    ).toBeTruthy();
    // `queryAll`, not `getAll`: the expected answer is *none*, and `getAll` throws on
    // none, which would pass this test for the wrong reason if it were ever inverted.
    expect(screen.queryAllByRole("button", { name: "Remove" })).toHaveLength(0);
  });

  it("explains that nothing is implied by a name when there are none", async () => {
    mount();

    expect(await screen.findByText(/No subjects recorded yet/)).toBeTruthy();
    expect(screen.getByText(/Nothing is implied by a name or by a department/)).toBeTruthy();
  });

  it("surfaces a duplicate refusal rather than appearing to have worked", async () => {
    const user = userEvent.setup();
    addQualification.mockRejectedValue({
      kind: "duplicate",
      entity: "qualification",
      field: "subject",
      message: "Another qualification already uses that subject.",
    });
    mount();
    await screen.findByText("No subjects recorded yet");

    await user.selectOptions(screen.getByLabelText("Subject to qualify for"), NORTH_MATHS.id);
    await user.click(screen.getByRole("button", { name: "Add qualification" }));

    expect(await screen.findByText(/already uses that subject/)).toBeTruthy();
  });
});
