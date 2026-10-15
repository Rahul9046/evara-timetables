/**
 * Stage A: year levels on their own screen.
 *
 * The functionality is not new — it moved out of the Students screen — so the thing worth
 * testing is that **nothing was lost on the way** and that the one property the move could
 * plausibly have broken still holds: a year level belongs to one school, and
 * [ADR 0019](../../../../../docs/adr/0019-multiple-schools-in-one-project.md) is explicit
 * that it is *never* shared. A screen that read the project's year levels instead of the
 * selected school's would show Northgate's Grade 8 under Southmoor's heading, which is the
 * failure the whole multi-school stage exists to prevent.
 *
 * Uniqueness and ordinal bounds are the repository's and the schema's — `UNIQUE (school_id,
 * name)` from `V10` — so what is asserted here is that a refusal *arrives* and is shown,
 * not that it is computed.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School, YearLevel, YearLevelInput } from "@evara/domain";

const listYearLevels = vi.fn<(schoolId: string) => Promise<YearLevel[]>>();
const createYearLevel = vi.fn<(input: YearLevelInput) => Promise<YearLevel>>();
const updateYearLevel =
  vi.fn<(id: string, rev: number, input: YearLevelInput) => Promise<YearLevel>>();
const deleteYearLevel = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();

vi.mock("../../ipc/setup", () => ({
  listYearLevels: (schoolId: string) => listYearLevels(schoolId),
  createYearLevel: (input: YearLevelInput) => createYearLevel(input),
  updateYearLevel: (id: string, rev: number, input: YearLevelInput) =>
    updateYearLevel(id, rev, input),
  deleteYearLevel: (schoolId: string, id: string, rev: number) =>
    deleteYearLevel(schoolId, id, rev),
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
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "duplicate"
      ? ((error as { field: string }).field ?? null)
      : null,
}));

const { YearLevels } = await import("./YearLevels");

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  // Not 1: an assertion that a write carried the row's revision must fail if the
  // screen sent a default instead.
  rev: 4,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function yearLevel(id: string, schoolId: string, name: string, ordinal: number): YearLevel {
  return { id, schoolId, name, ordinal, ...STAMP } as YearLevel;
}

/** Northgate numbers its stages 7–9; Southmoor calls its own Grade 1. */
const NORTH_LEVELS = [
  yearLevel("y-n-7", NORTH.id, "Year 7", 7),
  yearLevel("y-n-8", NORTH.id, "Year 8", 8),
  yearLevel("y-n-9", NORTH.id, "Year 9", 9),
];
const SOUTH_LEVELS = [yearLevel("y-s-1", SOUTH.id, "Grade 1", 1)];

beforeEach(() => {
  vi.clearAllMocks();
  listYearLevels.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? NORTH_LEVELS : SOUTH_LEVELS),
  );
  createYearLevel.mockImplementation((input) =>
    Promise.resolve(yearLevel("y-new", input.schoolId, input.name, input.ordinal)),
  );
  updateYearLevel.mockImplementation((id, _rev, input) =>
    Promise.resolve(yearLevel(id, input.schoolId, input.name, input.ordinal)),
  );
  deleteYearLevel.mockResolvedValue(undefined);
});

/** The year levels table's data rows, in the order they are rendered. */
function rows(): string[] {
  const table = screen.getByRole("table", { name: "Year levels" });
  return within(table)
    .getAllByRole("row")
    .slice(1)
    .map((row) => row.textContent ?? "");
}

describe("listing one school's year levels", () => {
  it("asks for them with the selected school's identifier", async () => {
    render(<YearLevels school={NORTH} />);

    await waitFor(() => expect(listYearLevels).toHaveBeenCalled());
    expect(listYearLevels).toHaveBeenCalledWith(NORTH.id);
    for (const call of listYearLevels.mock.calls) {
      expect(call[0]).toBe(NORTH.id);
    }
  });

  it("shows each level's name and position", async () => {
    render(<YearLevels school={NORTH} />);

    expect(await screen.findByText("Year 7")).toBeTruthy();
    expect(screen.getByText("Year 9")).toBeTruthy();
    const table = screen.getByRole("table", { name: "Year levels" });
    expect(within(table).getByText("7")).toBeTruthy();
    expect(within(table).getByText("9")).toBeTruthy();
  });

  it("preserves the order the repository returned", async () => {
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    // `year_level_list` orders by ordinal; the screen must not re-sort by name, which
    // would put "Year 10" before "Year 7".
    expect(rows().map((row) => row.split(/\d/)[0]?.trim())).toEqual([
      "Year",
      "Year",
      "Year",
    ]);
    expect(rows()[0]).toContain("Year 7");
    expect(rows()[2]).toContain("Year 9");
  });

  it("offers a first level rather than an empty table", async () => {
    listYearLevels.mockResolvedValue([]);
    render(<YearLevels school={NORTH} />);

    // Twice — the panel head and the empty state's own heading, which is the pattern
    // every other section's first run follows.
    expect(await screen.findAllByText(/No year levels yet/)).toHaveLength(2);
    expect(
      screen.getAllByRole("button", { name: "Add year level" }).length,
    ).toBeGreaterThan(0);
  });

  it("says they belong to this school alone", async () => {
    listYearLevels.mockResolvedValue([]);
    render(<YearLevels school={NORTH} />);

    expect(await screen.findByText(/belong to this school alone/)).toBeTruthy();
  });
});

describe("one school's levels never appear under another's", () => {
  it("re-reads on a switch and shows no trace of the school left behind", async () => {
    const view = render(<YearLevels school={NORTH} />);
    expect(await screen.findByText("Year 7")).toBeTruthy();

    view.rerender(<YearLevels school={SOUTH} />);

    expect(await screen.findByText("Grade 1")).toBeTruthy();
    expect(listYearLevels).toHaveBeenCalledWith(SOUTH.id);
    await waitFor(() => expect(screen.queryByText("Year 7")).toBeNull());
    expect(screen.queryByText("Year 9")).toBeNull();
  });

  it("lets two schools each hold a stage at the same position", async () => {
    // ADR 0019's actual requirement: "Grade 1" at ordinal 1 in both schools is two
    // records, and neither collides with the other.
    listYearLevels.mockImplementation((schoolId) =>
      Promise.resolve([yearLevel(`y-${schoolId}`, schoolId, "Grade 1", 1)]),
    );
    const view = render(<YearLevels school={NORTH} />);
    expect(await screen.findByText("Grade 1")).toBeTruthy();

    view.rerender(<YearLevels school={SOUTH} />);

    expect(await screen.findByText("Grade 1")).toBeTruthy();
    expect(screen.getAllByText("Grade 1")).toHaveLength(1);
  });
});

describe("creating a year level", () => {
  it("names the selected school and the position typed", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    await user.click(screen.getByRole("button", { name: "Add year level" }));
    await user.type(screen.getByLabelText(/Name/), "Year 10");
    await user.type(screen.getByLabelText(/Position/), "10");
    await user.click(screen.getByRole("button", { name: "Add year level" }));

    await waitFor(() => expect(createYearLevel).toHaveBeenCalled());
    expect(createYearLevel).toHaveBeenCalledWith({
      schoolId: NORTH.id,
      name: "Year 10",
      ordinal: 10,
    });
  });

  it("refuses a level with no name, before sending it", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    await user.click(screen.getByRole("button", { name: "Add year level" }));
    await user.type(screen.getByLabelText(/Position/), "10");
    await user.click(screen.getByRole("button", { name: "Add year level" }));

    expect(await screen.findByText("Year level name is required.")).toBeTruthy();
    expect(createYearLevel).not.toHaveBeenCalled();
  });

  it("refuses a position that is not a whole number", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    await user.click(screen.getByRole("button", { name: "Add year level" }));
    await user.type(screen.getByLabelText(/Name/), "Year 10");
    await user.type(screen.getByLabelText(/Position/), "tenth");
    await user.click(screen.getByRole("button", { name: "Add year level" }));

    expect(await screen.findByText(/Position must be a whole number/)).toBeTruthy();
    expect(createYearLevel).not.toHaveBeenCalled();
  });

  it("shows a duplicate name on the field, keeping what was typed", async () => {
    // `UNIQUE (school_id, name)` is the authority; the screen's job is to show the refusal
    // against the box that caused it rather than losing the typed row.
    const user = userEvent.setup();
    createYearLevel.mockRejectedValue({
      kind: "duplicate",
      entity: "year level",
      field: "name",
      message: "another year level already uses that name",
    });
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    await user.click(screen.getByRole("button", { name: "Add year level" }));
    await user.type(screen.getByLabelText(/Name/), "Year 8");
    await user.type(screen.getByLabelText(/Position/), "8");
    await user.click(screen.getByRole("button", { name: "Add year level" }));

    expect(await screen.findByText(/already uses that name/)).toBeTruthy();
    expect((screen.getByLabelText(/Name/) as HTMLInputElement).value).toBe("Year 8");
    expect((screen.getByLabelText(/Position/) as HTMLInputElement).value).toBe("8");
  });

  it("says position is the school's own order, not an age", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 7");

    await user.click(screen.getByRole("button", { name: "Add year level" }));

    const hint = screen.getByText(/The school's own order, lowest first/);
    expect(hint.textContent).toMatch(/Not an age/);
  });
});

describe("editing a year level", () => {
  it("renames it, keeping its position", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Edit Year 8" }));
    await user.clear(screen.getByLabelText(/Name/));
    await user.type(screen.getByLabelText(/Name/), "Grade 8");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateYearLevel).toHaveBeenCalled());
    expect(updateYearLevel).toHaveBeenCalledWith("y-n-8", 4, {
      schoolId: NORTH.id,
      name: "Grade 8",
      ordinal: 8,
    });
  });

  it("reorders it without touching its name", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Edit Year 8" }));
    await user.clear(screen.getByLabelText(/Position/));
    await user.type(screen.getByLabelText(/Position/), "12");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateYearLevel).toHaveBeenCalled());
    expect(updateYearLevel.mock.calls[0]?.[2]).toMatchObject({
      name: "Year 8",
      ordinal: 12,
    });
  });

  it("opens the form on the stored values", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Edit Year 8" }));

    expect((screen.getByLabelText(/Name/) as HTMLInputElement).value).toBe("Year 8");
    expect((screen.getByLabelText(/Position/) as HTMLInputElement).value).toBe("8");
  });
});

describe("deleting a year level", () => {
  it("names the school with the identifier", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Delete Year 8" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete year level" }));

    await waitFor(() => expect(deleteYearLevel).toHaveBeenCalled());
    expect(deleteYearLevel).toHaveBeenCalledWith(NORTH.id, "y-n-8", 4);
  });

  it("says the students in it are kept", async () => {
    const user = userEvent.setup();
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Delete Year 8" }));

    const dialog = await screen.findByRole("dialog");
    // The repository unclassifies through a real update rather than a database `SET NULL`,
    // so this is a promise the model actually keeps.
    expect(within(dialog).getByText(/Students and groups in this year level are kept/)).toBeTruthy();
  });

  it("shows a refusal rather than reporting a delete that did not happen", async () => {
    const user = userEvent.setup();
    deleteYearLevel.mockRejectedValue({
      kind: "wrongSchool",
      entity: "year level",
      message: "that year level belongs to a different school",
    });
    render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    await user.click(screen.getByRole("button", { name: "Delete Year 8" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete year level" }));

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
    // And the row is still there, because nothing was deleted.
    expect(screen.getByText("Year 8")).toBeTruthy();
  });

  it("never deletes against the school left behind", async () => {
    const user = userEvent.setup();
    const view = render(<YearLevels school={NORTH} />);
    await screen.findByText("Year 8");

    view.rerender(<YearLevels school={SOUTH} />);
    await screen.findByText("Grade 1");

    await user.click(screen.getByRole("button", { name: "Delete Grade 1" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete year level" }));

    await waitFor(() => expect(deleteYearLevel).toHaveBeenCalled());
    expect(deleteYearLevel).toHaveBeenCalledWith(SOUTH.id, "y-s-1", 4);
    for (const call of deleteYearLevel.mock.calls) {
      expect(call[0]).not.toBe(NORTH.id);
    }
  });
});
