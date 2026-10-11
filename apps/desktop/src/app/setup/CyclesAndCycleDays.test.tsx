/**
 * Cycles and Cycle Days, after two fields were retired from its forms.
 *
 * "Calendar weeks spanned" (`weekCount`) and "This is the school's default cycle"
 * (`isDefault`) are both gone from the forms and the table. Each leaves the same pair of
 * claims to make good:
 *
 * 1. the user is never asked for it again, on either form or in the table, and
 * 2. **no existing cycle record changes because of that.** Both columns are still
 *    `NOT NULL` and `CycleInput` still carries both, so every save has to send a value;
 *    the value it sends is the row's own.
 *
 * The second is the one worth testing. A field removed from a form usually takes its value
 * with it, and here that would silently rewrite every cycle a user touched -- demoting a
 * school's default cycle, in the case of `isDefault`, with nothing on screen to show it.
 *
 * `isDefault` has a third claim of its own: a new cycle is never promoted to default. The
 * old form ticked the box automatically for the first cycle in a project.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Cycle, CycleCoverage, CycleDay, School } from "@evara/domain";

const listCycles = vi.fn<() => Promise<Cycle[]>>();
const createCycle = vi.fn();
const updateCycle = vi.fn();
const deleteCycle = vi.fn();
const listCycleDays = vi.fn<(cycleId: string) => Promise<CycleDay[]>>();
const createCycleDay = vi.fn();
const updateCycleDay = vi.fn();
const deleteCycleDay = vi.fn();
const cycleCoverage = vi.fn<(cycleId: string) => Promise<CycleCoverage>>();

vi.mock("../../ipc/setup", () => ({
  listCycles: () => listCycles(),
  createCycle: (input: unknown) => createCycle(input),
  updateCycle: (id: string, input: unknown) => updateCycle(id, input),
  deleteCycle: (id: string) => deleteCycle(id),
  listCycleDays: (cycleId: string) => listCycleDays(cycleId),
  createCycleDay: (input: unknown) => createCycleDay(input),
  updateCycleDay: (id: string, input: unknown) => updateCycleDay(id, input),
  deleteCycleDay: (id: string) => deleteCycleDay(id),
  cycleCoverage: (cycleId: string) => cycleCoverage(cycleId),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: () => null,
}));

const { CyclesAndCycleDays } = await import("./CyclesAndCycleDays");

const SCHOOL = { id: "school-1", workingDays: [1, 2, 3, 4, 5] } as School;

function cycle(overrides: Partial<Cycle> = {}): Cycle {
  return {
    id: "cycle-1",
    schoolId: "school-1",
    name: "Two-week rotation",
    dayCount: 10,
    // A value a user once entered. Nothing asks for it any more, and nothing may change
    // it either.
    weekCount: 2,
    isDefault: true,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 4,
    ...overrides,
  } as Cycle;
}

beforeEach(() => {
  vi.clearAllMocks();
  listCycles.mockResolvedValue([cycle()]);
  listCycleDays.mockResolvedValue([]);
  cycleCoverage.mockResolvedValue({
    declaredDayCount: 10,
    present: 0,
    missingOrdinals: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    beyondDeclared: [],
  });
  createCycle.mockResolvedValue(cycle());
  updateCycle.mockResolvedValue(cycle());
});

async function shown() {
  render(<CyclesAndCycleDays school={SCHOOL} />);
  await screen.findByRole("button", { name: "Edit Two-week rotation" });
}

/** The input the last cycle save sent. */
function sent(mock: typeof updateCycle): Record<string, unknown> {
  return mock.mock.calls.at(-1)?.at(-1) as Record<string, unknown>;
}

describe("the weeks-spanned field", () => {
  it("is not on the new-cycle form", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(screen.queryByLabelText(/Calendar weeks spanned/)).toBeNull();
    expect(screen.queryByLabelText(/weeks/i)).toBeNull();
    // The only fields that remain.
    expect(screen.getByLabelText(/Cycle name/)).toBeTruthy();
    expect(screen.getByLabelText(/Days in the cycle/)).toBeTruthy();
    // Two, not three: the name and the day count. The day count is a text input with
    // `inputMode="numeric"`, so it carries the textbox role as well -- see ui/Field.
    expect(screen.getAllByRole("textbox")).toHaveLength(2);
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("is not on the edit form either", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));

    expect(screen.queryByLabelText(/Calendar weeks spanned/)).toBeNull();
    expect(screen.queryByLabelText(/weeks/i)).toBeNull();
  });

  it("takes its helper text with it", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(screen.queryByText(/For display and grouping only/)).toBeNull();
    expect(screen.queryByText(/fortnight usually spans/)).toBeNull();
  });

  it("is gone from the table", async () => {
    await shown();

    expect(screen.queryByRole("columnheader", { name: "Weeks" })).toBeNull();
    expect(screen.getByRole("columnheader", { name: "Days" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Name" })).toBeTruthy();
  });
});

describe("what the forms still do", () => {
  it("creates a cycle from just a name and a day count", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await userEvent.type(screen.getByLabelText(/Cycle name/), "Six-day rotation");
    await userEvent.clear(screen.getByLabelText(/Days in the cycle/));
    await userEvent.type(screen.getByLabelText(/Days in the cycle/), "6");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toEqual({
      schoolId: "school-1",
      name: "Six-day rotation",
      dayCount: 6,
      // Supplied rather than asked for: the schema requires it and 1 is its own default.
      weekCount: 1,
      isDefault: false,
    });
  });

  it("still rejects a cycle of zero days", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await userEvent.type(screen.getByLabelText(/Cycle name/), "Nothing");
    await userEvent.clear(screen.getByLabelText(/Days in the cycle/));
    await userEvent.type(screen.getByLabelText(/Days in the cycle/), "0");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(await screen.findByText(/Days in the cycle cannot be less than 1/)).toBeTruthy();
    expect(createCycle).not.toHaveBeenCalled();
  });

  it("still requires a name", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(await screen.findByText(/Cycle name is required/)).toBeTruthy();
    expect(createCycle).not.toHaveBeenCalled();
  });
});

describe("the default-cycle checkbox", () => {
  it("is not on the new-cycle form", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(screen.queryByLabelText(/default cycle/)).toBeNull();
    expect(screen.queryByLabelText(/default/i)).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("is not on the edit form either", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));

    expect(screen.queryByLabelText(/default/i)).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("takes its helper text with it", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(screen.queryByText(/At most one cycle can be the default/)).toBeNull();
    expect(screen.queryByText(/clears the previous one/)).toBeNull();
  });

  it("is gone from the table, along with the DEFAULT column", async () => {
    await shown();

    expect(screen.queryByRole("columnheader", { name: "Default" })).toBeNull();
    expect(
      screen.getAllByRole("columnheader").map((header) => header.textContent),
    ).toEqual(["Name", "Days", "Actions"]);
  });

  it("leaves no default badge on the cycle picker", async () => {
    listCycles.mockResolvedValue([cycle({ isDefault: true })]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await screen.findByRole("button", { name: "Edit Two-week rotation" });

    const picker = await screen.findByRole("group", { name: "Cycle" });
    expect(within(picker).queryByText("default")).toBeNull();
  });
});

describe("the stored default flag", () => {
  it("is never set on a cycle being created", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await userEvent.type(screen.getByLabelText(/Cycle name/), "Weekly");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toMatchObject({ isDefault: false });
  });

  it("is still false for the very first cycle in a project", async () => {
    // The old form ticked the box automatically here, which is the behaviour requirement
    // 7 forbids: nothing may acquire a default nobody chose.
    listCycles.mockResolvedValue([]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await userEvent.click(await screen.findByRole("button", { name: "Add cycle" }));

    await userEvent.type(screen.getByLabelText(/Cycle name/), "Weekly");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toMatchObject({ isDefault: false });
  });

  it("is preserved when a cycle that is the default is edited", async () => {
    listCycles.mockResolvedValue([cycle({ isDefault: true })]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit Two-week rotation" }),
    );

    await userEvent.clear(screen.getByLabelText(/Cycle name/));
    await userEvent.type(screen.getByLabelText(/Cycle name/), "Fortnight");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycle).toHaveBeenCalled());
    // Still the default. Sending `false` here would demote the school's default cycle
    // because somebody corrected a typo in its name.
    expect(sent(updateCycle)).toMatchObject({ isDefault: true });
  });

  it("is preserved when a cycle that is not the default is edited", async () => {
    listCycles.mockResolvedValue([cycle({ isDefault: false })]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit Two-week rotation" }),
    );

    await userEvent.clear(screen.getByLabelText(/Cycle name/));
    await userEvent.type(screen.getByLabelText(/Cycle name/), "Fortnight");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycle).toHaveBeenCalled());
    expect(sent(updateCycle)).toMatchObject({ isDefault: false });
  });
});

describe("existing cycle records", () => {
  it("keep their weeks-spanned value when edited", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));

    await userEvent.clear(screen.getByLabelText(/Cycle name/));
    await userEvent.type(screen.getByLabelText(/Cycle name/), "Fortnight");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycle).toHaveBeenCalled());
    expect(updateCycle).toHaveBeenCalledWith("cycle-1", {
      schoolId: "school-1",
      name: "Fortnight",
      dayCount: 10,
      // The row's own 2, not a 1 invented by the form. This is the assertion that keeps
      // "do not silently change existing cycle records" true.
      weekCount: 2,
      // Likewise: the fixture cycle is the default, and editing its name must not change
      // that. "the stored default flag" below covers this on its own.
      isDefault: true,
    });
  });

  it("keep an unusual value too", async () => {
    listCycles.mockResolvedValue([cycle({ weekCount: 5, name: "Five-week drift" })]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Edit Five-week drift" }),
    );

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycle).toHaveBeenCalled());
    expect(sent(updateCycle)).toMatchObject({ weekCount: 5 });
  });

  it("fall back to 1 rather than NaN if the stored value is unusable", async () => {
    // Not reachable through the interface — the column is `NOT NULL CHECK (>= 1)` — but a
    // save rejected by a constraint on a field the user cannot see would be unfixable
    // from the screen, so the conversion refuses to produce one.
    listCycles.mockResolvedValue([
      cycle({ weekCount: undefined as unknown as number, name: "Damaged" }),
    ]);
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await userEvent.click(await screen.findByRole("button", { name: "Edit Damaged" }));

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycle).toHaveBeenCalled());
    expect(sent(updateCycle)).toMatchObject({ weekCount: 1 });
  });
});

describe("the cycle is still not a week", () => {
  it("offers no weekday template and treats no day count as special", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    // Removing "weeks spanned" must not have been an invitation to add weekday thinking
    // here. The working-days setting lives in School Details and is a calendar fact; a
    // cycle day's ordinal remains its whole scheduling identity.
    for (const weekday of ["Monday", "Tuesday", "Saturday", "Sunday"]) {
      expect(screen.queryByText(new RegExp(weekday))).toBeNull();
    }
    expect(
      screen.getByText(/There is no assumption of a five-day week/),
    ).toBeTruthy();
  });
});
