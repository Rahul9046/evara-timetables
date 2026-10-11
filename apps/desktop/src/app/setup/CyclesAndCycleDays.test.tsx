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
  offendingField: (error: unknown) =>
    typeof error === "object" && error !== null && "field" in error
      ? String((error as { field: unknown }).field)
      : null,
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

/**
 * A coverage answer, as `evara_core::time::CycleCoverage` would compute it.
 *
 * The screen asks the backend whether a cycle is full rather than counting rows itself, so
 * this is what the tests below vary. `missing` is the positions still free.
 */
function coverageOf(declaredDayCount: number, missing: number[]): CycleCoverage {
  return {
    declaredDayCount,
    present: declaredDayCount - missing.length,
    missingOrdinals: missing,
    beyondDeclared: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listCycles.mockResolvedValue([cycle()]);
  listCycleDays.mockResolvedValue([]);
  cycleCoverage.mockResolvedValue(
    coverageOf(10, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
  );
  createCycle.mockResolvedValue(cycle());
  updateCycle.mockResolvedValue(cycle());
});

/** A cycle day row, for a list the screen will render. */
function dayRow(ordinal: number): CycleDay {
  return {
    id: `day-${ordinal}`,
    cycleId: "cycle-1",
    ordinal,
    label: `Day ${ordinal}`,
    weekdayHint: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as CycleDay;
}

/**
 * Every route to adding a cycle day.
 *
 * Two of them when the list is empty -- the panel head and the empty state both offer the
 * button -- and one when there are rows. Asserted as a set rather than picking the first,
 * because "adding is disabled" has to hold for every route, not just whichever one a test
 * happened to query.
 */
const addDayButtons = () =>
  screen.getAllByRole("button", { name: "Add cycle day" }) as HTMLButtonElement[];

/** Whether adding is offered at all, by every route that offers it. */
function addingEnabled(): boolean {
  const buttons = addDayButtons();
  expect(buttons.length).toBeGreaterThan(0);
  return buttons.every((button) => !button.disabled);
}

/** The panel head's own button, for the assertions that are about it specifically. */
const addDayButton = () => addDayButtons()[0] as HTMLButtonElement;

/**
 * Renders with a cycle of `dayCount` whose free positions are `missing`.
 *
 * Waits for the day panel to have *loaded*, not merely to exist. The Add button is
 * disabled while the list is still arriving, so returning as soon as it appears would read
 * that loading state as though it were the limit -- which is how the first version of this
 * helper made three tests pass for the wrong reason and two fail for the right one.
 */
async function withCoverage(dayCount: number, missing: number[]) {
  const present = Array.from({ length: dayCount }, (_, index) => index + 1).filter(
    (ordinal) => !missing.includes(ordinal),
  );
  listCycles.mockResolvedValue([cycle({ dayCount })]);
  listCycleDays.mockResolvedValue(present.map(dayRow));
  cycleCoverage.mockResolvedValue(coverageOf(dayCount, missing));

  render(<CyclesAndCycleDays school={SCHOOL} />);
  await screen.findByRole("button", { name: "Add cycle day" });
  await waitFor(() => expect(cycleCoverage).toHaveBeenCalled());
  await waitFor(() => expect(screen.queryByText(/Loading cycle days/)).toBeNull());
}

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

describe("the cycle day limit", () => {
  it("disables adding when every position is filled, and says why", async () => {
    await withCoverage(5, []);

    // Waiting for the note rather than asserting `disabled` straight away: the button is
    // also disabled while the list loads, and this has to be about the limit.
    await screen.findByText(/All cycle positions are filled/);
    expect(addingEnabled()).toBe(false);
    const button = addDayButton();
    expect(button.disabled).toBe(true);
    expect(button.title).toBe(
      "All cycle positions are filled. Increase the cycle length to add more days.",
    );
    // Visible as well as on the title: a disabled button is not focusable, so a tooltip
    // alone would be unreachable by keyboard and by touch.
    expect(
      screen.getByText(
        "All cycle positions are filled. Increase the cycle length to add more days.",
      ),
    ).toBeTruthy();
  });

  it("keeps the table of days on screen while adding is disabled", async () => {
    await withCoverage(5, []);
    await screen.findByText(/All cycle positions are filled/);

    // Not `blockedBy`: the reason adding is unavailable is that all five of these exist,
    // so hiding them would be perverse.
    expect(addingEnabled()).toBe(false);
    for (const ordinal of [1, 2, 3, 4, 5]) {
      expect(screen.getByRole("button", { name: `Edit Day ${ordinal} (position ${ordinal})` }))
        .toBeTruthy();
    }
  });

  it("allows adding while a position is still free", async () => {
    await withCoverage(5, [3]);

    await waitFor(() => expect(addingEnabled()).toBe(true));
    expect(addDayButton().title).toBe("");
    expect(screen.queryByText(/All cycle positions are filled/)).toBeNull();
  });

  it("allows adding on a wholly empty cycle", async () => {
    await withCoverage(5, [1, 2, 3, 4, 5]);

    await waitFor(() => expect(addingEnabled()).toBe(true));
    expect(screen.queryByText(/All cycle positions are filled/)).toBeNull();
  });

  it("offers the missing position, not one past the end", async () => {
    await withCoverage(5, [3]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    // The gap, so the common case is one keystroke: name it and save.
    expect((screen.getByLabelText(/Position/) as HTMLInputElement).value).toBe("3");
  });

  it("names the free positions in the hint rather than the whole range", async () => {
    await withCoverage(10, [4, 7]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    const hint = screen.getByText(/still free/);
    expect(hint.textContent).toMatch(/From 1 to 10/);
    expect(hint.textContent).toMatch(/still free: 4, 7/);
  });

  it("gives the plain range when every position is free", async () => {
    await withCoverage(5, [1, 2, 3, 4, 5]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    const hint = screen.getByText(/This is the day's identity/);
    expect(hint.textContent).toMatch(/From 1 to 5/);
    expect(hint.textContent).not.toMatch(/still free/);
  });

  it("creates the missing day with the position and label given", async () => {
    await withCoverage(5, [3]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    await userEvent.type(screen.getByLabelText(/Label/), "Day C");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle day" }));

    await waitFor(() => expect(createCycleDay).toHaveBeenCalled());
    expect(createCycleDay).toHaveBeenCalledWith({
      cycleId: "cycle-1",
      ordinal: 3,
      label: "Day C",
      weekdayHint: null,
    });
  });

  it("shows the backend's refusal when a position slips through", async () => {
    // The screen does not re-implement the range check -- `check_ordinal_within_cycle` in
    // `evara-db` is the authority -- so what matters here is that its refusal is shown and
    // the typed values survive it.
    createCycleDay.mockRejectedValue({
      kind: "invalid",
      message: "this cycle is 5 days long, so it has no day 6",
    });
    await withCoverage(5, [3]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    await userEvent.clear(screen.getByLabelText(/Position/));
    await userEvent.type(screen.getByLabelText(/Position/), "6");
    await userEvent.type(screen.getByLabelText(/Label/), "Day F");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle day" }));

    expect(
      await screen.findByText("this cycle is 5 days long, so it has no day 6"),
    ).toBeTruthy();
    expect((screen.getByLabelText(/Position/) as HTMLInputElement).value).toBe("6");
  });

  it("marks the position field when the backend reports it as taken", async () => {
    createCycleDay.mockRejectedValue({
      kind: "duplicate",
      field: "position",
      message: "another cycle day already uses that position",
    });
    await withCoverage(5, [3]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    await userEvent.type(screen.getByLabelText(/Label/), "Day C");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle day" }));

    const position = (await screen.findByLabelText(/Position/)) as HTMLInputElement;
    await waitFor(() => expect(position.getAttribute("aria-invalid")).toBe("true"));
    expect(
      screen.getByText("another cycle day already uses that position"),
    ).toBeTruthy();
  });

  it("still refuses a position below 1 before the round trip", async () => {
    await withCoverage(5, [3]);
    await waitFor(() => expect(addingEnabled()).toBe(true));
    await userEvent.click(addDayButton());

    await userEvent.clear(screen.getByLabelText(/Position/));
    await userEvent.type(screen.getByLabelText(/Position/), "0");
    await userEvent.type(screen.getByLabelText(/Label/), "Nowhere");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle day" }));

    // `validateCycleDay` already restates the column's own `CHECK (ordinal >= 1)`, which
    // is a field-shape rule and therefore allowed in the frontend.
    expect(await screen.findByText(/Position cannot be less than 1/)).toBeTruthy();
    expect(createCycleDay).not.toHaveBeenCalled();
  });

  it("does not disable adding while coverage is still loading", async () => {
    // `undefined` coverage is "not known yet", not "full". Treating it as full would
    // flash the button off and on during every load.
    let release: (value: CycleCoverage) => void = () => {};
    cycleCoverage.mockReturnValue(
      new Promise<CycleCoverage>((resolve) => {
        release = resolve;
      }),
    );
    listCycles.mockResolvedValue([cycle({ dayCount: 5 })]);
    listCycleDays.mockResolvedValue([1, 2, 3, 4, 5].map(dayRow));

    render(<CyclesAndCycleDays school={SCHOOL} />);
    // Waiting on a row rather than on the loading text: until the picker settles on a
    // cycle the day panel is blocked entirely and has no Add button to read.
    await screen.findByRole("button", { name: "Edit Day 1 (position 1)" });
    // Rows have arrived; coverage has not. Nothing is known to be full yet.
    expect(addingEnabled()).toBe(true);
    expect(screen.queryByText(/All cycle positions are filled/)).toBeNull();

    release(coverageOf(5, []));
    await waitFor(() => expect(addingEnabled()).toBe(false));
  });

  it("re-enables adding once the cycle is lengthened", async () => {
    await withCoverage(5, []);
    await screen.findByText(/All cycle positions are filled/);
    expect(addingEnabled()).toBe(false);

    // What editing the cycle to six days looks like from here: the backend reports a free
    // position, and the panel follows.
    cycleCoverage.mockResolvedValue(coverageOf(6, [6]));
    listCycles.mockResolvedValue([cycle({ dayCount: 6 })]);
    updateCycle.mockResolvedValue(cycle({ dayCount: 6 }));

    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));
    await userEvent.clear(screen.getByLabelText(/Days in the cycle/));
    await userEvent.type(screen.getByLabelText(/Days in the cycle/), "6");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(addingEnabled()).toBe(true));
    expect(screen.queryByText(/All cycle positions are filled/)).toBeNull();
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
