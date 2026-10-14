/**
 * Stage 0C: managing the schools a campus file describes.
 *
 * `evara-db`'s tests own the model — `UNIQUE (name)`, a partial unique index on `code`,
 * `ON DELETE RESTRICT` on every school-owned table, and `school_dependents` counting what
 * is in the way. This file owns the four things only this screen can get wrong:
 *
 * - a **second** school can be added, and gets none of the first one's configuration;
 * - the optional **code** is collected, and is neither invented nor silently cleared;
 * - a school with anything in it is **explained**, not offered a button that fails —
 *   including a borrowed person, whose remedy is in a different screen;
 * - an **empty** school can be deleted, with one confirmation.
 *
 * The last two are the pair worth having. A refusal that arrives after the user pressed
 * Delete explains nothing, and a refusal that is never explained at all leaves somebody
 * emptying a school at random to find out what is holding it.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School, SchoolDependents } from "@evara/domain";

const listSchools = vi.fn<() => Promise<School[]>>();
const createSchool = vi.fn<(input: unknown) => Promise<School>>();
const updateSchool = vi.fn<(id: string, input: unknown) => Promise<School>>();
const deleteSchool = vi.fn<(id: string) => Promise<void>>();
const schoolDependents = vi.fn<(id: string) => Promise<SchoolDependents>>();

vi.mock("../../ipc/setup", () => ({
  listSchools: () => listSchools(),
  createSchool: (input: unknown) => createSchool(input),
  updateSchool: (id: string, input: unknown) => updateSchool(id, input),
  deleteSchool: (id: string) => deleteSchool(id),
  schoolDependents: (id: string) => schoolDependents(id),
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

const { Schools } = await import("./Schools");

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-02T00:00:00.000Z",
  rev: 1,
};

function school(overrides: Partial<School> = {}): School {
  return {
    id: "school-north",
    name: "Northgate Secondary",
    code: "NGS",
    timezone: "Europe/London",
    locale: "en-GB",
    workingDays: [1, 2, 3, 4, 5],
    ...STAMP,
    ...overrides,
  } as School;
}

const NORTH = school();
const SOUTH = school({ id: "school-south", name: "Southmoor College", code: "SMC" });

/** Nothing in the way: `total` is zero, so the school can be deleted. */
function empty(): SchoolDependents {
  return {
    total: 0,
    campuses: 0,
    roomTypes: 0,
    resources: 0,
    academicYears: 0,
    cycles: 0,
    periodStructures: 0,
    timeslots: 0,
    calendarDays: 0,
    yearLevels: 0,
    teachers: 0,
    students: 0,
    studentGroups: 0,
    availabilityRules: 0,
    sharedTeachers: 0,
    sharedStudents: 0,
  };
}

/** A dependent report with the given counts, and a `total` that agrees with them. */
function holding(counts: Partial<SchoolDependents>): SchoolDependents {
  const found = { ...empty(), ...counts };
  const { total: _ignored, ...kinds } = found;
  return {
    ...found,
    total: Object.values(kinds).reduce((sum, count) => sum + count, 0),
  };
}

function mount(props: Partial<Parameters<typeof Schools>[0]> = {}) {
  const onSelect = vi.fn();
  const onOpenSection = vi.fn();
  const onChanged = vi.fn();
  const view = render(
    <Schools
      activeSchoolId={NORTH.id}
      onSelect={onSelect}
      onOpenSection={onOpenSection}
      onChanged={onChanged}
      {...props}
    />,
  );
  return { view, onSelect, onOpenSection, onChanged };
}

beforeEach(() => {
  vi.clearAllMocks();
  listSchools.mockResolvedValue([NORTH]);
  createSchool.mockImplementation((input) => Promise.resolve(school(input as Partial<School>)));
  updateSchool.mockImplementation((id, input) =>
    Promise.resolve(school({ id, ...(input as Partial<School>) })),
  );
  deleteSchool.mockResolvedValue(undefined);
  schoolDependents.mockResolvedValue(empty());
});

/** Opens the Add form and fills the two required-and-unprefilled fields. */
async function addSchool(user: ReturnType<typeof userEvent.setup>, name: string, code?: string) {
  await user.click(screen.getByRole("button", { name: "Add school" }));
  await user.type(screen.getByLabelText(/School name/), name);
  if (code !== undefined) await user.type(screen.getByLabelText(/School code/), code);
  await user.click(screen.getByRole("button", { name: "Add school" }));
}

describe("listing the schools in a campus file", () => {
  it("shows one school's name, code, zone, locale and week", async () => {
    mount();

    expect(await screen.findByText("Northgate Secondary")).toBeTruthy();
    expect(screen.getByText("NGS")).toBeTruthy();
    expect(screen.getByText("United Kingdom Time — Europe/London")).toBeTruthy();
    expect(screen.getByText("English (United Kingdom) — en-GB")).toBeTruthy();
    expect(screen.getByText("Monday to Friday")).toBeTruthy();
  });

  it("marks which school the rest of setup is about", async () => {
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    mount({ activeSchoolId: SOUTH.id });

    await screen.findByText("Southmoor College");
    const row = screen.getByText("Southmoor College").closest("tr") as HTMLElement;
    expect(within(row).getByText("selected")).toBeTruthy();
  });

  it("says a school is empty, so a delete is known to be available before it is pressed", async () => {
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    schoolDependents.mockImplementation((id) =>
      Promise.resolve(id === NORTH.id ? empty() : holding({ campuses: 2, cycles: 1 })),
    );
    mount();

    expect(await screen.findByText("Empty")).toBeTruthy();
    // Three rows' worth of counting, read up front rather than when Delete is pressed.
    await waitFor(() => expect(screen.getByText("3")).toBeTruthy());
  });

  it("offers a first school rather than an empty table", async () => {
    listSchools.mockResolvedValue([]);
    mount({ activeSchoolId: null });

    expect(await screen.findByText(/Nothing has been set up/)).toBeTruthy();
    // Twice: the panel head's action and the empty state's own, which is the pattern every
    // other section's first run follows.
    expect(screen.getAllByRole("button", { name: "Add school" })).toHaveLength(2);
  });
});

describe("adding a school", () => {
  it("creates a second one alongside the first", async () => {
    const user = userEvent.setup();
    const { onChanged } = mount();
    await screen.findByText("Northgate Secondary");

    await addSchool(user, "Southmoor College", "SMC");

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    expect(createSchool.mock.calls[0]?.[0]).toMatchObject({
      name: "Southmoor College",
      code: "SMC",
    });
    // The shell re-reads, so the selector and every section see the new school.
    expect(onChanged).toHaveBeenCalled();
  });

  it("copies nothing from the school that already exists", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Northgate Secondary");

    await addSchool(user, "Southmoor College");

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    const input = createSchool.mock.calls[0]?.[0] as Record<string, unknown>;
    // `create_school` writes a school and nothing else: no campuses, no years, no cycle.
    // The form carries only the school's own fields, and this is what proves nothing of
    // Northgate's configuration is smuggled along with it.
    expect(Object.keys(input).sort()).toEqual([
      "code",
      "locale",
      "name",
      "timezone",
      "workingDays",
    ]);
    expect(input["name"]).toBe("Southmoor College");
    expect(input["name"]).not.toBe(NORTH.name);
  });

  it("starts a new school on Monday to Friday, not on the other school's week", async () => {
    const user = userEvent.setup();
    listSchools.mockResolvedValue([school({ workingDays: [7, 1, 2, 3, 4] })]);
    mount();
    await screen.findByText("Northgate Secondary");

    await addSchool(user, "Southmoor College");

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    expect(createSchool.mock.calls[0]?.[0]).toMatchObject({
      workingDays: [1, 2, 3, 4, 5],
    });
  });

  it("sends no code when the box is left blank", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Northgate Secondary");

    await addSchool(user, "Southmoor College");

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    // `code` is unique when given through a partial index, so "" would collide between
    // two schools that have no code at all.
    expect(createSchool.mock.calls[0]?.[0]).toMatchObject({ code: null });
  });

  it("shows a duplicate name on the field, keeping what was typed", async () => {
    const user = userEvent.setup();
    createSchool.mockRejectedValue({
      kind: "duplicate",
      entity: "school",
      field: "name",
      message: "another school already uses that name",
    });
    mount();
    await screen.findByText("Northgate Secondary");

    await addSchool(user, "Northgate Secondary");

    expect(await screen.findByText(/already uses that name/)).toBeTruthy();
    expect((screen.getByLabelText(/School name/) as HTMLInputElement).value).toBe(
      "Northgate Secondary",
    );
  });
});

describe("editing a school", () => {
  it("renames it without touching anything else", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Northgate Secondary");

    await user.click(screen.getByRole("button", { name: /^Edit Northgate Secondary/ }));
    await user.clear(screen.getByLabelText(/School name/));
    await user.type(screen.getByLabelText(/School name/), "Northgate Academy");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool).toHaveBeenCalledWith(NORTH.id, {
      name: "Northgate Academy",
      code: "NGS",
      timezone: "Europe/London",
      locale: "en-GB",
      workingDays: [1, 2, 3, 4, 5],
    });
  });

  it("preserves the code when only the working days change", async () => {
    // The regression the Stage 0A workaround existed to prevent, now that the field is
    // real: a save must never clear a code it was not asked about.
    const user = userEvent.setup();
    mount();
    await screen.findByText("Northgate Secondary");

    await user.click(screen.getByRole("button", { name: /^Edit Northgate Secondary/ }));
    await user.click(screen.getByRole("checkbox", { name: "Saturday" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      code: "NGS",
      workingDays: [1, 2, 3, 4, 5, 6],
    });
  });

  it("edits the code itself", async () => {
    const user = userEvent.setup();
    mount();
    await screen.findByText("Northgate Secondary");

    await user.click(screen.getByRole("button", { name: /^Edit Northgate Secondary/ }));
    await user.clear(screen.getByLabelText(/School code/));
    await user.type(screen.getByLabelText(/School code/), "NGA");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({ code: "NGA" });
  });

  it("keeps a timezone this build does not offer", async () => {
    // A school saved while these were free-text boxes, or in a zone added since. Editing
    // its name must not rewrite its zone to something plausible.
    const user = userEvent.setup();
    listSchools.mockResolvedValue([school({ timezone: "Mars/Olympus_Mons" })]);
    mount();
    await screen.findByText("Northgate Secondary");

    await user.click(screen.getByRole("button", { name: /^Edit Northgate Secondary/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      timezone: "Mars/Olympus_Mons",
    });
  });
});

describe("choosing which school the rest of setup is about", () => {
  it("selects another school", async () => {
    const user = userEvent.setup();
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    const { onSelect } = mount();
    await screen.findByText("Southmoor College");

    await user.click(screen.getByRole("button", { name: /^Select Southmoor College/ }));

    expect(onSelect).toHaveBeenCalledWith(SOUTH.id);
  });

  it("does not offer to select the school already selected", async () => {
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    mount({ activeSchoolId: SOUTH.id });
    await screen.findByText("Southmoor College");

    const already = screen.getByRole("button", { name: /^Select Southmoor College/ });
    expect(already.hasAttribute("disabled")).toBe(true);
    expect(already.getAttribute("title")).toMatch(/already the one/);
  });

  it("opens one school's details, selecting it on the way", async () => {
    const user = userEvent.setup();
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    const { onSelect, onOpenSection } = mount();
    await screen.findByText("Southmoor College");

    await user.click(screen.getByRole("button", { name: /^Open details Southmoor College/ }));

    // Both, in that order: opening another school's details without selecting it would
    // show one school's record under the other's heading.
    expect(onSelect).toHaveBeenCalledWith(SOUTH.id);
    expect(onOpenSection).toHaveBeenCalledWith("school");
  });
});

describe("deleting a school", () => {
  it("deletes an empty one after one confirmation", async () => {
    const user = userEvent.setup();
    listSchools.mockResolvedValue([NORTH, SOUTH]);
    const { onChanged } = mount();
    await screen.findByText("Southmoor College");
    await waitFor(() => expect(schoolDependents).toHaveBeenCalledWith(SOUTH.id));

    await user.click(screen.getByRole("button", { name: /^Delete Southmoor College/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/holds nothing/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Delete school" }));

    await waitFor(() => expect(deleteSchool).toHaveBeenCalledWith(SOUTH.id));
    expect(onChanged).toHaveBeenCalled();
  });

  it("explains what is in the way instead of offering a delete that fails", async () => {
    const user = userEvent.setup();
    schoolDependents.mockResolvedValue(
      holding({ campuses: 2, yearLevels: 7, timeslots: 240 }),
    );
    mount();
    await screen.findByText("Northgate Secondary");
    await waitFor(() => expect(schoolDependents).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: /^Delete Northgate Secondary/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("2 campuses")).toBeTruthy();
    expect(within(dialog).getByText("7 year levels")).toBeTruthy();
    expect(within(dialog).getByText("240 timetable slots")).toBeTruthy();
    // No destructive button at all, and nothing is sent.
    expect(within(dialog).queryByRole("button", { name: "Delete school" })).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeTruthy();
    expect(deleteSchool).not.toHaveBeenCalled();
  });

  it("never offers to delete a school and everything in it", async () => {
    const user = userEvent.setup();
    schoolDependents.mockResolvedValue(holding({ campuses: 1 }));
    mount();
    await screen.findByText("Northgate Secondary");
    await waitFor(() => expect(schoolDependents).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: /^Delete Northgate Secondary/ }));

    const dialog = await screen.findByRole("dialog");
    // ADR 0019's rule, stated to the user: a school is years of configuration and one
    // mis-click must not take a campus, a grid and a staff list with it.
    expect(within(dialog).getByText(/never deletes a school and everything in it/)).toBeTruthy();
    expect(within(dialog).getByText(/section by section/)).toBeTruthy();
  });

  it("explains a borrowed person separately, because the remedy is elsewhere", async () => {
    const user = userEvent.setup();
    schoolDependents.mockResolvedValue(holding({ sharedTeachers: 2, sharedStudents: 1 }));
    mount();
    await screen.findByText("Northgate Secondary");
    await waitFor(() => expect(schoolDependents).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: /^Delete Northgate Secondary/ }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("2 teachers shared from another school"),
    ).toBeTruthy();
    expect(
      within(dialog).getByText("1 student enrolled from another school"),
    ).toBeTruthy();
    // And what to do about it, which is not "empty this school": the person stays with
    // the school that holds their identity, and this school lets go of its membership.
    expect(
      within(dialog).getByText(/without deleting the person/),
    ).toBeTruthy();
  });

  it("refuses to offer a delete it could not count, rather than guessing", async () => {
    const user = userEvent.setup();
    schoolDependents.mockRejectedValue({ kind: "failed", message: "disk is on fire" });
    mount();
    await screen.findByText("Northgate Secondary");
    expect(await screen.findByText(/could not be counted/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /^Delete Northgate Secondary/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/could not count what this school still holds/)).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete school" })).toBeNull();
  });

  it("shows a refusal from the backend rather than swallowing it", async () => {
    // The dependent counts can be stale: another window, or a write between the read and
    // the press. The backend is the authority and its refusal has to reach the screen.
    const user = userEvent.setup();
    deleteSchool.mockRejectedValue({
      kind: "stillReferenced",
      entity: "school",
      message: "that school still owns records",
    });
    mount();
    await screen.findByText("Northgate Secondary");
    await waitFor(() => expect(schoolDependents).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: /^Delete Northgate Secondary/ }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete school" }));

    expect(await screen.findByText(/still owns records/)).toBeTruthy();
    // And the row is still there, because nothing was deleted.
    expect(screen.getByText("Northgate Secondary")).toBeTruthy();
  });
});
