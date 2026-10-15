/**
 * Stage A: the four workload fields are off the teacher form, and their values are not.
 *
 * This is the one test in the stage that guards against a *silent* loss. Hiding a field
 * from a form that submits a whole record is how a value gets quietly replaced by a
 * default or a `null`: `toDraft` stops reading it, or `toInput` stops writing it, and
 * nothing fails — the save succeeds, the screen looks right, and a school's part-time
 * contracts have become full-time ones. The repository cannot catch it, because what
 * arrives is a valid record.
 *
 * So every assertion here is about the **payload**, and the fixture carries four
 * non-default values on purpose:
 *
 * | Field | Stored | Why this value |
 * | --- | --- | --- |
 * | `ftePercent` | `60` | not the schema's `DEFAULT 100`, so a reset is visible |
 * | `maxPeriodsPerDay` | `4` | a real ceiling, so a `null` is visible |
 * | `maxPeriodsPerCycle` | `18` | likewise |
 * | `maxConsecutivePeriods` | `2` | likewise |
 *
 * The audit flagged this pass-through as the risk of the stage and kept flagging it. It is
 * now asserted rather than hoped for.
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
  TeacherInput,
  TeacherSchool,
} from "@evara/domain";

const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const listSchoolTeachers = vi.fn<(schoolId: string) => Promise<TeacherInSchool[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const createTeacher = vi.fn<(input: TeacherInput) => Promise<Teacher>>();
const updateTeacher =
  vi.fn<(id: string, rev: number, input: TeacherInput) => Promise<Teacher>>();
const listTeacherMemberships = vi.fn<(teacherId: string) => Promise<TeacherSchool[]>>();
const listTeacherCampuses = vi.fn<(teacherId: string) => Promise<string[]>>();
const assignTeacherCampus =
  vi.fn<(schoolId: string, teacherId: string, campusId: string) => Promise<TeacherCampus>>();
const removeTeacherCampus =
  vi.fn<(schoolId: string, teacherId: string, campusId: string) => Promise<void>>();

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  listSchoolTeachers: (schoolId: string) => listSchoolTeachers(schoolId),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  createTeacher: (input: TeacherInput) => createTeacher(input),
  updateTeacher: (id: string, rev: number, input: TeacherInput) =>
    updateTeacher(id, rev, input),
  deleteTeacher: vi.fn(() => Promise.resolve()),
  listTeacherMemberships: (teacherId: string) => listTeacherMemberships(teacherId),
  listTeacherCampuses: (teacherId: string) => listTeacherCampuses(teacherId),
  addTeacherToSchool: vi.fn(() => Promise.resolve({})),
  setTeacherMembershipActive: vi.fn(() => Promise.resolve({})),
  removeTeacherFromSchool: vi.fn(() => Promise.resolve()),
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

const NORTH_MAIN = {
  id: "c-north-main",
  schoolId: NORTH.id,
  name: "Main Campus",
  code: "NM",
  address: null,
  ...STAMP,
} as Campus;

const NORTH_JUNIOR = {
  id: "c-north-junior",
  schoolId: NORTH.id,
  name: "Junior Campus",
  code: "NJ",
  address: null,
  ...STAMP,
} as Campus;

/** Ada, part-time, with every ceiling set to something that is not a default. */
function teacher(overrides: Partial<Teacher> = {}): Teacher {
  return {
    id: "teacher-ada",
    schoolId: NORTH.id,
    code: "N-17",
    givenName: "Ada",
    familyName: "Okafor",
    email: "ada@example.invalid",
    homeCampusId: NORTH_MAIN.id,
    ftePercent: 60,
    maxPeriodsPerDay: 4,
    maxPeriodsPerCycle: 18,
    maxConsecutivePeriods: 2,
    notes: "Mondays off",
    isActive: true,
    ...STAMP,
    ...overrides,
  } as Teacher;
}

const ADA = teacher();

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

function context(overrides: Partial<SetupContext> = {}): SetupContext {
  return {
    schools: [NORTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listTeachers.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [ADA] : []),
  );
  listSchoolTeachers.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [inSchool(ADA, NORTH.id)] : []),
  );
  listCampuses.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_MAIN, NORTH_JUNIOR] : []),
  );
  listTeacherMemberships.mockResolvedValue([inSchool(ADA, NORTH.id).membership]);
  listTeacherCampuses.mockResolvedValue([NORTH_MAIN.id]);
  createTeacher.mockImplementation((input) =>
    Promise.resolve(teacher({ id: "teacher-new", ...(input as Partial<Teacher>) })),
  );
  updateTeacher.mockImplementation((id, input) =>
    Promise.resolve(teacher({ id, ...(input as Partial<Teacher>) })),
  );
  assignTeacherCampus.mockResolvedValue({
    id: "tc-1",
    teacherId: ADA.id,
    campusId: NORTH_JUNIOR.id,
    ...STAMP,
  } as TeacherCampus);
  removeTeacherCampus.mockResolvedValue(undefined);
});

function mount(school: School = NORTH, overrides: Partial<SetupContext> = {}) {
  return render(<Teachers school={school} context={context(overrides)} />);
}

/** Opens the edit form on the one teacher on screen. */
async function openEdit(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByText("Okafor");
  await user.click(screen.getByRole("button", { name: /^Edit Ada Okafor/ }));
}

describe("the four workload fields are not on the form", () => {
  it.each([
    ["full-time equivalent", /Full-time equivalent/i],
    ["most periods per day", /Most periods per day/i],
    ["most periods per cycle", /Most periods per cycle/i],
    ["most periods in a row", /Most periods in a row/i],
  ])("does not ask for %s when editing", async (_name, label) => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    expect(screen.queryByLabelText(label)).toBeNull();
  });

  it.each([
    ["full-time equivalent", /Full-time equivalent/i],
    ["most periods per day", /Most periods per day/i],
    ["most periods per cycle", /Most periods per cycle/i],
    ["most periods in a row", /Most periods in a row/i],
  ])("does not ask for %s when creating", async (_name, label) => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Okafor");
    await user.click(screen.getByRole("button", { name: "Add teacher" }));

    expect(screen.queryByLabelText(label)).toBeNull();
  });

  it("still asks for the identity fields, which is the point of the form", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    expect(screen.getByLabelText(/First name/)).toBeTruthy();
    expect(screen.getByLabelText(/Last name/)).toBeTruthy();
    expect(screen.getByLabelText(/Staff code/)).toBeTruthy();
    expect(screen.getByLabelText(/Email/)).toBeTruthy();
    expect(screen.getByLabelText(/Notes/)).toBeTruthy();
  });

  it("says once, visibly, that the values are kept rather than gone", async () => {
    // Somebody who set a ceiling in an earlier version and cannot find the field needs to
    // know the value survives; somebody who never set one needs to know they have not
    // skipped a step.
    mount();

    const note = await screen.findByText(/Workload limits/);
    expect(note.textContent).toMatch(/kept exactly as they are/);
    expect(note.textContent).toMatch(/never changed by editing a teacher/);
  });

  it("still shows the stored FTE in the table, because the value is real", async () => {
    mount();

    await screen.findByText("Okafor");
    expect(screen.getByText("60%")).toBeTruthy();
  });
});

describe("editing a teacher preserves every stored workload value", () => {
  it("sends all four back unchanged when only the name is edited", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.clear(screen.getByLabelText(/First name/));
    await user.type(screen.getByLabelText(/First name/), "Adaeze");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    const [id, , input] = updateTeacher.mock.calls[0] as [string, number, TeacherInput];
    expect(id).toBe(ADA.id);
    expect(input.givenName).toBe("Adaeze");
    // The whole point of the stage's riskiest change, asserted exactly.
    expect(input.ftePercent).toBe(60);
    expect(input.maxPeriodsPerDay).toBe(4);
    expect(input.maxPeriodsPerCycle).toBe(18);
    expect(input.maxConsecutivePeriods).toBe(2);
  });

  it("does not reset FTE to the schema default", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    // `100` is `V10`'s `DEFAULT`, and is what a form that rebuilt the draft from nothing
    // would send. It is the single most likely wrong answer here.
    expect(updateTeacher.mock.calls[0]?.[2].ftePercent).not.toBe(100);
    expect(updateTeacher.mock.calls[0]?.[2].ftePercent).toBe(60);
  });

  it("does not null the three ceilings", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    const input = updateTeacher.mock.calls[0]?.[2] as TeacherInput;
    // `null` is the other likely wrong answer: a hidden field read as "blank" means "no
    // ceiling", which would silently lift a real limit.
    expect(input.maxPeriodsPerDay).not.toBeNull();
    expect(input.maxPeriodsPerCycle).not.toBeNull();
    expect(input.maxConsecutivePeriods).not.toBeNull();
  });

  it("keeps a teacher who genuinely has no ceilings without inventing any", async () => {
    const user = userEvent.setup();
    const uncapped = teacher({
      ftePercent: 100,
      maxPeriodsPerDay: null,
      maxPeriodsPerCycle: null,
      maxConsecutivePeriods: null,
    });
    listSchoolTeachers.mockResolvedValue([inSchool(uncapped, NORTH.id)]);
    mount();
    await openEdit(user);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    const input = updateTeacher.mock.calls[0]?.[2] as TeacherInput;
    // A blank ceiling means *no ceiling*, never zero. Sending `0` would forbid them from
    // teaching at all.
    expect(input.maxPeriodsPerDay).toBeNull();
    expect(input.maxPeriodsPerCycle).toBeNull();
    expect(input.maxConsecutivePeriods).toBeNull();
    expect(input.ftePercent).toBe(100);
  });

  it("sends every key the command expects, so serde cannot reject the payload", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    // Omitting a key would be indistinguishable from "leave it alone", which this command
    // never means — and `TeacherInput`'s non-optional fields would fail to deserialise.
    expect(Object.keys(updateTeacher.mock.calls[0]?.[2] ?? {}).sort()).toEqual([
      "code",
      "email",
      "familyName",
      "ftePercent",
      "givenName",
      "homeCampusId",
      "isActive",
      "maxConsecutivePeriods",
      "maxPeriodsPerCycle",
      "maxPeriodsPerDay",
      "notes",
      "schoolId",
    ]);
  });

  it("survives a second edit, so the values are not lost one round trip later", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateTeacher).toHaveBeenCalledTimes(1));

    await openEdit(user);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalledTimes(2));
    expect(updateTeacher.mock.calls[1]?.[2]).toMatchObject({
      ftePercent: 60,
      maxPeriodsPerDay: 4,
      maxPeriodsPerCycle: 18,
      maxConsecutivePeriods: 2,
    });
  });
});

describe("creating a teacher", () => {
  it("uses the schema's own defaults rather than inventing any", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add teacher" }));
    await user.type(screen.getByLabelText(/First name/), "Bea");
    await user.type(screen.getByLabelText(/Last name/), "Nwosu");
    await user.click(screen.getByRole("button", { name: "Add teacher" }));

    await waitFor(() => expect(createTeacher).toHaveBeenCalled());
    // `V10`: `fte_percent INTEGER NOT NULL DEFAULT 100`, and three nullable ceilings. The
    // frontend matches the schema rather than choosing its own numbers.
    expect(createTeacher.mock.calls[0]?.[0]).toMatchObject({
      givenName: "Bea",
      familyName: "Nwosu",
      ftePercent: 100,
      maxPeriodsPerDay: null,
      maxPeriodsPerCycle: null,
      maxConsecutivePeriods: null,
    });
  });

  it("is not blocked by a validator for a field nobody can see", async () => {
    // The trap this stage could have walked into: validating a hidden numeric field leaves
    // an unsubmittable form with nothing on screen to explain why.
    const user = userEvent.setup();
    mount();
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add teacher" }));
    await user.type(screen.getByLabelText(/First name/), "Bea");
    await user.type(screen.getByLabelText(/Last name/), "Nwosu");
    await user.click(screen.getByRole("button", { name: "Add teacher" }));

    await waitFor(() => expect(createTeacher).toHaveBeenCalled());
    expect(screen.queryByText(/Full-time equivalent/)).toBeNull();
  });

  it("still refuses a teacher with no name", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: "Add teacher" }));
    await user.click(screen.getByRole("button", { name: "Add teacher" }));

    expect(await screen.findByText("First name is required.")).toBeTruthy();
    expect(createTeacher).not.toHaveBeenCalled();
  });
});

describe("home campus and campus eligibility stay different things", () => {
  it("offers a home campus, described as where they are based rather than where they may work", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    const field = screen.getByLabelText(/Home campus/);
    expect(field).toBeTruthy();
    const hint = screen.getByText(/only where they are normally based/);
    expect(hint.textContent).toMatch(/not the set of campuses they may work at/);
    expect(hint.textContent).toMatch(/campus eligibility/i);
  });

  it("keeps the stored home campus through an edit", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    expect(updateTeacher.mock.calls[0]?.[2].homeCampusId).toBe(NORTH_MAIN.id);
  });

  it("does not touch the home campus when eligibility changes", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: /^Schools Ada Okafor/ }));
    const panel = (await screen.findByText(/^Schools and campuses — /)).closest(
      ".subPanel",
    ) as HTMLElement;
    await user.click(within(panel).getByRole("checkbox", { name: "Junior Campus" }));

    await waitFor(() => expect(assignTeacherCampus).toHaveBeenCalled());
    expect(assignTeacherCampus).toHaveBeenCalledWith(NORTH.id, ADA.id, NORTH_JUNIOR.id);
    // Eligibility is its own table. A screen that wrote the teacher record here would be
    // changing where somebody is *based* because they gained a site they may work at.
    expect(updateTeacher).not.toHaveBeenCalled();
  });

  it("does not change eligibility when the teacher record is edited", async () => {
    const user = userEvent.setup();
    mount();
    await openEdit(user);

    await user.clear(screen.getByLabelText(/First name/));
    await user.type(screen.getByLabelText(/First name/), "Adaeze");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTeacher).toHaveBeenCalled());
    // The mirror image, and the one the brief asks for by name.
    expect(assignTeacherCampus).not.toHaveBeenCalled();
    expect(removeTeacherCampus).not.toHaveBeenCalled();
  });

  it("keeps the multi-campus eligibility panel reachable", async () => {
    const user = userEvent.setup();
    mount(NORTH, { schools: [NORTH, SOUTH] });
    await screen.findByText("Okafor");

    await user.click(screen.getByRole("button", { name: /^Schools Ada Okafor/ }));

    // The Stage 0C control, still the user-facing mechanism for where somebody may work.
    expect(await screen.findByText(/^Schools and campuses — /)).toBeTruthy();
    expect(screen.getByRole("checkbox", { name: "Main Campus" })).toBeTruthy();
    expect(screen.getByRole("checkbox", { name: "Junior Campus" })).toBeTruthy();
  });
});
