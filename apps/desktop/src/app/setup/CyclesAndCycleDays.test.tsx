/**
 * Cycles and Cycle Days, after "Calendar weeks spanned" was retired.
 *
 * The point of this file is the pair of claims that make removing a field safe:
 *
 * 1. the user is never asked for `weekCount` again, on the forms or in the table, and
 * 2. **no existing cycle record changes because of that.** The column is `NOT NULL CHECK
 *    (week_count >= 1)` and `CycleInput` still carries it, so an edit has to send a value;
 *    the value it sends is the row's own.
 *
 * The second is the one worth a test. A field removed from a form usually takes its value
 * with it, and here that would silently rewrite every cycle a user touched.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
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
    // The fields that remain.
    expect(screen.getByLabelText(/Cycle name/)).toBeTruthy();
    expect(screen.getByLabelText(/Days in the cycle/)).toBeTruthy();
    expect(screen.getByLabelText(/default cycle/)).toBeTruthy();
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
  it("creates a cycle with a name, a day count and a default flag", async () => {
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

  it("still sets the default-cycle flag", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await userEvent.type(screen.getByLabelText(/Cycle name/), "Weekly");
    await userEvent.click(screen.getByLabelText(/default cycle/));
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toMatchObject({ isDefault: true });
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
