/**
 * Stage 0C: one teacher, several schools, several campuses.
 *
 * `evara-db`'s `tests_membership.rs` owns the rules — eligibility refused outside
 * membership, the home membership refused removal, a delete refused while another school
 * employs them. This file owns the five things only the interface can get wrong, and
 * which no backend test can catch:
 *
 * - **adding an existing person creates no second record.** The whole point of `V13`, and
 *   the one thing an interface can single-handedly undo: a screen that offered no way to
 *   add somebody already in the project would force the user to type them again, and two
 *   teacher rows is one human being with two timetables.
 * - **deactivating and removing are different buttons**, and deleting is a third, in a
 *   different panel. ADR 0021 §8 keeps the three apart; a screen that merged any two of
 *   them would destroy a person somebody meant to stop employing.
 * - **membership is written before campus eligibility.** The database refuses the other
 *   order and names the remedy, so the screen must not need to be told twice.
 * - **another school's employment is visible but not editable here.** Seeing it is the
 *   point; changing it is `WrongSchool`.
 * - **names are never merged.** Two people with one name are two people.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Campus,
  School,
  Teacher,
  TeacherCampus,
  TeacherInSchool,
  TeacherSchool,
} from "@evara/domain";

const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const listSchoolTeachers = vi.fn<(schoolId: string) => Promise<TeacherInSchool[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listTeacherMemberships = vi.fn<(teacherId: string) => Promise<TeacherSchool[]>>();
const listTeacherCampuses = vi.fn<(teacherId: string) => Promise<string[]>>();
const addTeacherToSchool = vi.fn<(input: unknown) => Promise<TeacherSchool>>();
const setTeacherMembershipActive =
  vi.fn<
    (schoolId: string, teacherId: string, rev: number, isActive: boolean) => Promise<TeacherSchool>
  >();
const removeTeacherFromSchool = vi.fn<(schoolId: string, teacherId: string) => Promise<void>>();
const assignTeacherCampus =
  vi.fn<(schoolId: string, teacherId: string, campusId: string) => Promise<TeacherCampus>>();
const removeTeacherCampus =
  vi.fn<(schoolId: string, teacherId: string, campusId: string) => Promise<void>>();
const createTeacher = vi.fn((_input: unknown) => Promise.resolve({}));

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  listSchoolTeachers: (schoolId: string) => listSchoolTeachers(schoolId),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  createTeacher: (input: unknown) => createTeacher(input),
  updateTeacher: vi.fn(() => Promise.resolve({})),
  deleteTeacher: vi.fn(() => Promise.resolve()),
  listTeacherMemberships: (teacherId: string) => listTeacherMemberships(teacherId),
  listTeacherCampuses: (teacherId: string) => listTeacherCampuses(teacherId),
  addTeacherToSchool: (input: unknown) => addTeacherToSchool(input),
  setTeacherMembershipActive: (
    schoolId: string,
    teacherId: string,
    rev: number,
    isActive: boolean,
  ) => setTeacherMembershipActive(schoolId, teacherId, rev, isActive),
  removeTeacherFromSchool: (schoolId: string, teacherId: string) =>
    removeTeacherFromSchool(schoolId, teacherId),
  assignTeacherCampus: (schoolId: string, teacherId: string, campusId: string) =>
    assignTeacherCampus(schoolId, teacherId, campusId),
  removeTeacherCampus: (schoolId: string, teacherId: string, campusId: string) =>
    removeTeacherCampus(schoolId, teacherId, campusId),
  listAvailability: vi.fn(() => Promise.resolve([])),
  countAvailability: vi.fn(() => Promise.resolve(0)),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  listCycles: vi.fn(() => Promise.resolve([])),
  listPeriodStructures: vi.fn(() => Promise.resolve([])),
  gridPreview: vi.fn(() => Promise.resolve(null)),
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
  offendingField: () => null,
}));

const { Teachers } = await import("./Teachers");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

/** Ada, created at Northgate. One record, whichever school is looking at her. */
function teacher(overrides: Partial<Teacher> = {}): Teacher {
  return {
    id: "teacher-ada",
    schoolId: NORTH.id,
    code: "N-17",
    givenName: "Ada",
    familyName: "Okafor",
    email: "ada@example.invalid",
    homeCampusId: null,
    ftePercent: 100,
    maxPeriodsPerDay: null,
    maxPeriodsPerCycle: null,
    maxConsecutivePeriods: null,
    notes: null,
    isActive: true,
    ...STAMP,
    ...overrides,
  } as Teacher;
}

const ADA = teacher();
/** Somebody with the same family name, who is a different person. */
const OTHER_OKAFOR = teacher({
  id: "teacher-chi",
  schoolId: SOUTH.id,
  code: "S-04",
  givenName: "Chidi",
  email: null,
});

function campus(id: string, schoolId: string, name: string, code: string): Campus {
  return { id, schoolId, name, code, address: null, ...STAMP } as Campus;
}

const NORTH_MAIN = campus("c-north-main", NORTH.id, "Main Campus", "NM");
const NORTH_JUNIOR = campus("c-north-junior", NORTH.id, "Junior Campus", "NJ");
const SOUTH_MAIN = campus("c-south-main", SOUTH.id, "Riverside", "SR");

function inSchool(person: Teacher, schoolId: string, isActive = true): TeacherInSchool {
  return {
    teacher: person,
    membership: {
      id: `ts-${person.id}-${schoolId}`,
      teacherId: person.id,
      schoolId,
      isActive,
      ...STAMP,
    },
  } as TeacherInSchool;
}

function memberOf(person: Teacher, schoolId: string, isActive = true): TeacherSchool {
  return inSchool(person, schoolId, isActive).membership;
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
  // Northgate employs Ada; Southmoor employs Chidi. Nobody is shared to begin with.
  listTeachers.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [ADA] : [OTHER_OKAFOR]),
  );
  listSchoolTeachers.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id ? [inSchool(ADA, NORTH.id)] : [inSchool(OTHER_OKAFOR, SOUTH.id)],
    ),
  );
  listCampuses.mockImplementation((schoolId) =>
    Promise.resolve(
      schoolId === NORTH.id ? [NORTH_MAIN, NORTH_JUNIOR] : [SOUTH_MAIN],
    ),
  );
  listTeacherMemberships.mockResolvedValue([memberOf(ADA, NORTH.id)]);
  listTeacherCampuses.mockResolvedValue([]);
  addTeacherToSchool.mockResolvedValue(memberOf(ADA, SOUTH.id));
  setTeacherMembershipActive.mockResolvedValue(memberOf(ADA, NORTH.id, false));
  removeTeacherFromSchool.mockResolvedValue(undefined);
  assignTeacherCampus.mockResolvedValue({
    id: "tc-1",
    teacherId: ADA.id,
    campusId: NORTH_MAIN.id,
    ...STAMP,
  } as TeacherCampus);
  removeTeacherCampus.mockResolvedValue(undefined);
});

/** Renders the Teachers screen for one school, with the project's schools in context. */
function mount(school: School, overrides: Partial<SetupContext> = {}) {
  return render(<Teachers school={school} context={context(overrides)} />);
}

/** Opens the relationship panel for the one teacher on screen. */
async function openSchoolsPanel(user: ReturnType<typeof userEvent.setup>, family: string) {
  await screen.findByText(family);
  await user.click(screen.getByRole("button", { name: new RegExp(`^Schools .*${family}`) }));
  return (await screen.findByText(/^Schools and campuses — /)).closest(
    ".subPanel",
  ) as HTMLElement;
}

describe("adding a teacher who already exists", () => {
  it("writes a membership and creates no teacher record", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Okafor"); // Chidi, Southmoor's own

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: /Okafor, Ada/ }));
    await user.click(within(dialog).getByRole("button", { name: "Add to this school" }));

    await waitFor(() => expect(addTeacherToSchool).toHaveBeenCalled());
    expect(addTeacherToSchool).toHaveBeenCalledWith({
      teacherId: ADA.id,
      schoolId: SOUTH.id,
      isActive: true,
    });
    // The thing `V13` exists to make unnecessary, and only an interface could reintroduce.
    expect(createTeacher).not.toHaveBeenCalled();
  });

  it("offers only people this school does not already employ", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");

    // Ada is offered; Chidi, whom Southmoor already employs, is not — adding somebody
    // twice is a duplicate the backend refuses, so offering it would be offering a
    // guaranteed failure.
    expect(within(dialog).getByRole("radio", { name: /Okafor, Ada/ })).toBeTruthy();
    expect(within(dialog).queryByRole("radio", { name: /Okafor, Chidi/ })).toBeNull();
  });

  it("shows enough detail to tell two people with one surname apart", async () => {
    const user = userEvent.setup();
    // Both at Northgate, so Southmoor sees two candidates called Okafor.
    listTeachers.mockImplementation((schoolId) =>
      Promise.resolve(schoolId === NORTH.id ? [ADA, teacher({ id: "teacher-ada-2", code: "N-91", givenName: "Ada", email: null })] : []),
    );
    listSchoolTeachers.mockImplementation((schoolId) =>
      Promise.resolve(schoolId === NORTH.id ? [inSchool(ADA, NORTH.id)] : []),
    );
    mount(SOUTH);

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");

    // Two radios, not one merged row. Names alone cannot distinguish them, so the code,
    // the email and the holding school are shown.
    const options = within(dialog).getAllByRole("radio");
    expect(options).toHaveLength(2);
    expect(within(dialog).getByText(/Staff code N-17/)).toBeTruthy();
    expect(within(dialog).getByText(/Staff code N-91/)).toBeTruthy();
    expect(within(dialog).getAllByText(/Identity held by Northgate Secondary/)).toHaveLength(2);
  });

  it("filters the list by any detail, not only the name", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Search"), "N-17");

    expect(within(dialog).getByRole("radio", { name: /Okafor, Ada/ })).toBeTruthy();
  });

  it("does not offer to add before somebody is chosen", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");

    // A button that silently does nothing is worse than one that is visibly unavailable
    // and says why.
    const confirm = within(dialog).getByRole("button", { name: "Add to this school" });
    expect(confirm.hasAttribute("disabled")).toBe(true);
    expect(confirm.getAttribute("title")).toMatch(/Choose a teacher/);

    await user.click(within(dialog).getByRole("radio", { name: /Okafor, Ada/ }));
    expect(
      within(dialog)
        .getByRole("button", { name: "Add to this school" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });

  it("says plainly that it never merges two records with one name", async () => {
    const user = userEvent.setup();
    mount(SOUTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/never merges two records/)).toBeTruthy();
  });

  it("surfaces a refusal and keeps the dialogue open", async () => {
    const user = userEvent.setup();
    addTeacherToSchool.mockRejectedValue({
      kind: "duplicate",
      entity: "school membership",
      field: "school",
      message: "that school already employs them",
    });
    mount(SOUTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add an existing teacher" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: /Okafor, Ada/ }));
    await user.click(within(dialog).getByRole("button", { name: "Add to this school" }));

    expect(await within(dialog).findByText(/already employs them/)).toBeTruthy();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("is not offered at all in a single-school campus file", async () => {
    render(<Teachers school={NORTH} context={context({ schools: [NORTH] })} />);
    await screen.findByText("Okafor");

    // Nobody to add from: the only school is this one, and the button would be furniture.
    expect(screen.queryByRole("button", { name: "Add an existing teacher" })).toBeNull();
  });
});

describe("a teacher's schools", () => {
  it("lists every school that employs them, marking the home one", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([
      memberOf(ADA, NORTH.id),
      memberOf(ADA, SOUTH.id, false),
    ]);
    mount(NORTH);

    const panel = await openSchoolsPanel(user, "Okafor");
    // Scoped to the memberships table: the school names also appear as the legends of the
    // campus-eligibility groups below, which is a different statement about them.
    const table = within(panel).getByRole("table", { name: /Schools employing/ });

    expect(within(table).getByText("Northgate Secondary")).toBeTruthy();
    expect(within(table).getByText("Southmoor College")).toBeTruthy();
    expect(within(panel).getByText("Home school")).toBeTruthy();
    expect(within(panel).getByText("Shared")).toBeTruthy();
    // Each school's own answer to "do you employ her?", side by side.
    expect(within(panel).getByText("Teaching here")).toBeTruthy();
    expect(within(panel).getByText("Not teaching here")).toBeTruthy();
  });

  it("deactivates this school's employment without deleting anything", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("button", { name: "Mark as left" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Nothing is deleted/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Mark as left" }));

    await waitFor(() => expect(setTeacherMembershipActive).toHaveBeenCalled());
    // The MEMBERSHIP's revision, not the teacher's: `teacher_school` is the row that
    // changes, and nothing about the person does.
    expect(setTeacherMembershipActive).toHaveBeenCalledWith(
      NORTH.id,
      ADA.id,
      expect.any(Number),
      false,
    );
    expect(removeTeacherFromSchool).not.toHaveBeenCalled();
  });

  it("reactivates without a confirmation, because nothing is being taken away", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([memberOf(ADA, NORTH.id, false)]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("button", { name: "Mark as teaching" }));

    await waitFor(() => expect(setTeacherMembershipActive).toHaveBeenCalled());
    expect(setTeacherMembershipActive).toHaveBeenCalledWith(
      NORTH.id,
      ADA.id,
      expect.any(Number),
      true,
    );
  });

  it("removes a non-home membership, and says what survives it", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([
      memberOf(ADA, NORTH.id),
      memberOf(ADA, SOUTH.id),
    ]);
    listSchoolTeachers.mockImplementation((schoolId) =>
      Promise.resolve([inSchool(ADA, schoolId)]),
    );
    mount(SOUTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(
      within(panel).getByRole("button", { name: "Remove from this school" }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/every other school keeps them/)).toBeTruthy();
    await user.click(
      within(dialog).getByRole("button", { name: "Remove from this school" }),
    );

    await waitFor(() => expect(removeTeacherFromSchool).toHaveBeenCalled());
    expect(removeTeacherFromSchool).toHaveBeenCalledWith(SOUTH.id, ADA.id);
  });

  it("refuses to remove the home membership, and names the remedy", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    const remove = within(panel).getByRole("button", { name: "Remove from this school" });
    expect(remove.hasAttribute("disabled")).toBe(true);
    expect(remove.getAttribute("title")).toMatch(/created in this school/);
    // The remedy, named where the refusal is: deactivate, or delete the person.
    expect(remove.getAttribute("title")).toMatch(/Mark as left/);
    expect(removeTeacherFromSchool).not.toHaveBeenCalled();
  });

  it("shows another school's employment but does not offer to change it", async () => {
    const user = userEvent.setup();
    const selectSchool = vi.fn();
    listTeacherMemberships.mockResolvedValue([
      memberOf(ADA, NORTH.id),
      memberOf(ADA, SOUTH.id),
    ]);
    mount(NORTH, { selectSchool });
    const panel = await openSchoolsPanel(user, "Okafor");

    // Southmoor's row offers no activate and no remove — only a way to go and do it there.
    const table = within(panel).getByRole("table", { name: /Schools employing/ });
    const row = within(table).getByText("Southmoor College").closest("tr") as HTMLElement;
    expect(within(row).queryByRole("button", { name: /^Mark as/ })).toBeNull();
    expect(within(row).queryByRole("button", { name: /^Remove/ })).toBeNull();

    await user.click(within(row).getByRole("button", { name: /Manage at Southmoor/ }));
    expect(selectSchool).toHaveBeenCalledWith(SOUTH.id);
  });

  it("shows a refusal from the backend rather than reporting success", async () => {
    const user = userEvent.setup();
    setTeacherMembershipActive.mockRejectedValue({
      kind: "wrongSchool",
      entity: "school membership",
      message: "that membership belongs to a different school",
    });
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("button", { name: "Mark as left" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Mark as left" }));

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
  });

  it("adds a membership for another school from here, active by default", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.selectOptions(
      within(panel).getByLabelText("School to add this teacher to"),
      SOUTH.id,
    );
    await user.click(within(panel).getByRole("button", { name: "Add to school" }));

    await waitFor(() => expect(addTeacherToSchool).toHaveBeenCalled());
    expect(addTeacherToSchool).toHaveBeenCalledWith({
      teacherId: ADA.id,
      schoolId: SOUTH.id,
      isActive: true,
    });
  });
});

describe("campus eligibility", () => {
  it("groups campuses by school, with each school's membership state", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([memberOf(ADA, NORTH.id)]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    const groups = within(panel).getAllByRole("group");
    const named = groups.map((group) => group.querySelector("legend")?.textContent ?? "");
    expect(named.some((text) => text.includes("Northgate Secondary") && text.includes("member"))).toBe(true);
    expect(
      named.some((text) => text.includes("Southmoor College") && text.includes("not a member")),
    ).toBe(true);
  });

  it("assigns a campus of the acting school", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("checkbox", { name: "Junior Campus" }));

    await waitFor(() => expect(assignTeacherCampus).toHaveBeenCalled());
    expect(assignTeacherCampus).toHaveBeenCalledWith(NORTH.id, ADA.id, NORTH_JUNIOR.id);
  });

  it("assigns several campuses, one call each", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("checkbox", { name: "Main Campus" }));
    await waitFor(() => expect(assignTeacherCampus).toHaveBeenCalledTimes(1));
    await user.click(within(panel).getByRole("checkbox", { name: "Junior Campus" }));

    await waitFor(() => expect(assignTeacherCampus).toHaveBeenCalledTimes(2));
    expect(assignTeacherCampus.mock.calls.map((call) => call[2])).toEqual([
      NORTH_MAIN.id,
      NORTH_JUNIOR.id,
    ]);
  });

  it("offers no campus of a school the teacher does not belong to", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([memberOf(ADA, NORTH.id)]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    // The one ordering mistake the database's rule actually produces, made unreachable:
    // Southmoor's campus is shown so the user can see it exists, and is not selectable.
    const riverside = within(panel).getByRole("checkbox", { name: "Riverside" });
    expect((riverside as HTMLInputElement).disabled).toBe(true);
    expect(assignTeacherCampus).not.toHaveBeenCalled();
  });

  it("names the remedy for that, rather than leaving the user to guess", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    expect(
      within(panel).getByText(/Add this teacher to Southmoor College before giving them/),
    ).toBeTruthy();
  });

  it("offers no campus of another school even where the teacher is a member", async () => {
    const user = userEvent.setup();
    // Ada belongs to both, but Northgate still may not decide who works at Southmoor's
    // sites: `assign_teacher_campus` refuses it with `WrongSchool`.
    listTeacherMemberships.mockResolvedValue([
      memberOf(ADA, NORTH.id),
      memberOf(ADA, SOUTH.id),
    ]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    const riverside = within(panel).getByRole("checkbox", { name: "Riverside" });
    expect((riverside as HTMLInputElement).disabled).toBe(true);
    expect(
      within(panel).getByText(/Only Southmoor College can decide who works at its campuses/),
    ).toBeTruthy();
  });

  it("withdraws one campus after confirming, leaving the membership alone", async () => {
    const user = userEvent.setup();
    listTeacherCampuses.mockResolvedValue([NORTH_MAIN.id]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("checkbox", { name: "Main Campus" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/employment by Northgate Secondary is untouched/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Withdraw campus" }));

    await waitFor(() => expect(removeTeacherCampus).toHaveBeenCalled());
    expect(removeTeacherCampus).toHaveBeenCalledWith(NORTH.id, ADA.id, NORTH_MAIN.id);
    expect(removeTeacherFromSchool).not.toHaveBeenCalled();
  });

  it("shows the eligibility a teacher already has, across schools", async () => {
    const user = userEvent.setup();
    listTeacherMemberships.mockResolvedValue([
      memberOf(ADA, NORTH.id),
      memberOf(ADA, SOUTH.id),
    ]);
    listTeacherCampuses.mockResolvedValue([NORTH_JUNIOR.id, SOUTH_MAIN.id]);
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    // Both are shown as held, even though only Northgate's is editable here. Hiding
    // Southmoor's would make the screen say she is not eligible there, which is false.
    expect(
      (within(panel).getByRole("checkbox", { name: "Junior Campus" }) as HTMLInputElement)
        .checked,
    ).toBe(true);
    expect(
      (within(panel).getByRole("checkbox", { name: "Riverside" }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("surfaces a backend refusal rather than appearing to have worked", async () => {
    const user = userEvent.setup();
    assignTeacherCampus.mockRejectedValue({
      kind: "wrongSchool",
      entity: "campus",
      message: "that campus belongs to a different school",
    });
    mount(NORTH);
    const panel = await openSchoolsPanel(user, "Okafor");

    await user.click(within(panel).getByRole("checkbox", { name: "Main Campus" }));

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
  });
});

describe("the panel before a teacher is chosen", () => {
  it("says what to do rather than showing an empty table", async () => {
    mount(NORTH);

    expect(await screen.findByText("No teacher chosen")).toBeTruthy();
  });
});

describe("deleting a teacher another school employs", () => {
  it("is explained rather than offered", async () => {
    const user = userEvent.setup();
    // Ada works at both, so Northgate deleting her would take a teacher away from
    // Southmoor without Southmoor being asked.
    listTeachers.mockResolvedValue([ADA]);
    listSchoolTeachers.mockImplementation((schoolId) =>
      Promise.resolve([inSchool(ADA, schoolId)]),
    );
    mount(NORTH);
    await screen.findByText("Okafor");
    await waitFor(() => expect(listTeachers).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: /^Delete Ada Okafor/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Southmoor College/)).toBeTruthy();
    expect(within(dialog).getByText(/removes its own employment first/)).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete teacher" })).toBeNull();
  });

  it("is offered for a teacher only this school employs", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Okafor");
    await waitFor(() => expect(listTeachers).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: /^Delete Ada Okafor/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Delete teacher" })).toBeTruthy();
  });
});

describe("the list itself", () => {
  it("shows the other schools a teacher works at", async () => {
    listTeachers.mockResolvedValue([ADA]);
    listSchoolTeachers.mockImplementation((schoolId) =>
      Promise.resolve([inSchool(ADA, schoolId)]),
    );
    mount(NORTH);

    await screen.findByText("Okafor");
    await waitFor(() => expect(screen.getByText("Southmoor College")).toBeTruthy());
    expect(screen.getByText("shared")).toBeTruthy();
  });

  it("says plainly when a teacher works only here", async () => {
    mount(NORTH);

    await screen.findByText("Okafor");
    await waitFor(() => expect(screen.getByText("This school only")).toBeTruthy());
  });

  it("keeps the notes field, which is still asked for", async () => {
    // The four workload values moved off the form in Stage A, and their round trip is
    // covered by `TeacherFormFields.test.tsx`. Notes stayed, and is the one free-text
    // field whose round trip nothing else here exercises.
    const user = userEvent.setup();
    listSchoolTeachers.mockResolvedValue([
      inSchool(teacher({ notes: "Mondays off" }), NORTH.id),
    ]);
    mount(NORTH);
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: /^Edit Ada Okafor/ }));

    expect((screen.getByLabelText(/Notes/) as HTMLTextAreaElement).value).toBe("Mondays off");
  });
});
