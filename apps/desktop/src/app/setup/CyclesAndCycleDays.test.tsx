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

import type {
  Cycle,
  CycleCoverage,
  CycleDay,
  CycleShape,
  School,
  WeeklyCycleDay,
} from "@evara/domain";

const listCycles = vi.fn<() => Promise<Cycle[]>>();
const createCycle = vi.fn();
const updateCycle = vi.fn();
const deleteCycle = vi.fn();
const listCycleDays = vi.fn<(cycleId: string) => Promise<CycleDay[]>>();
const createCycleDay = vi.fn();
const updateCycleDay = vi.fn();
const deleteCycleDay = vi.fn();
const cycleShape =
  vi.fn<(cycleId: string, workingDays: readonly number[]) => Promise<CycleShape>>();

vi.mock("../../ipc/setup", () => ({
  listCycles: () => listCycles(),
  createCycle: (input: unknown) => createCycle(input),
  updateCycle: (id: string, input: unknown) => updateCycle(id, input),
  deleteCycle: (id: string) => deleteCycle(id),
  listCycleDays: (cycleId: string) => listCycleDays(cycleId),
  createCycleDay: (input: unknown) => createCycleDay(input),
  updateCycleDay: (id: string, input: unknown) => updateCycleDay(id, input),
  deleteCycleDay: (id: string) => deleteCycleDay(id),
  cycleShape: (cycleId: string, workingDays: readonly number[]) =>
    cycleShape(cycleId, workingDays),
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
    // Every cycle written before the type existed is a custom rotation, and so is
    // every cycle in this file unless it says otherwise.
    kind: "CUSTOM",
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

/**
 * A custom cycle's shape, as `setup_cycle_shape` would answer it.
 *
 * The screen asks one question and reads the answer: is this cycle weekly or custom, how
 * does its rotation run, which positions are empty, and which weekdays apply. Everything
 * in it is computed in Rust, which is the point — nothing below may work any of it out.
 */
function shapeOf(coverage: CycleCoverage, cycleId = "cycle-1"): CycleShape {
  const sequence = Array.from(
    { length: coverage.declaredDayCount },
    (_, index) => index + 1,
  );
  return {
    cycleId,
    kind: "CUSTOM",
    dayCount: coverage.declaredDayCount,
    rotation: {
      dayCount: coverage.declaredDayCount,
      sequence,
      wrapsTo: sequence[0] ?? null,
    },
    coverage,
    // Empty for a custom cycle: it has positions, not weekdays.
    weekdays: [],
  };
}

/** A weekly position, as the backend reports it for one working week. */
function weekdayOf(ordinal: number, applies: boolean): WeeklyCycleDay {
  const names = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
  ];
  const canonical = names[ordinal - 1] as string;
  return {
    cycleDayId: `day-${ordinal}`,
    ordinal,
    weekday: ordinal,
    label: canonical,
    canonicalLabel: canonical,
    applies,
  };
}

/**
 * A weekly cycle's shape for a school keeping `week`.
 *
 * All seven positions are present whatever the week is — that is the requirement the whole
 * design exists for — and `applies` is the comparison the backend made.
 */
function weeklyShapeOf(week: readonly number[]): CycleShape {
  const sequence = [1, 2, 3, 4, 5, 6, 7];
  return {
    cycleId: "cycle-1",
    kind: "WEEKLY",
    dayCount: 7,
    rotation: { dayCount: 7, sequence, wrapsTo: 1 },
    coverage: coverageOf(7, []),
    weekdays: sequence.map((ordinal) => weekdayOf(ordinal, week.includes(ordinal))),
  };
}

/** A weekly cycle row. */
function weeklyCycle(overrides: Partial<Cycle> = {}): Cycle {
  return cycle({ name: "School week", kind: "WEEKLY", dayCount: 7, ...overrides });
}

/** The seven weekday rows a weekly cycle's day list returns. */
function weeklyDayRows(): CycleDay[] {
  return [1, 2, 3, 4, 5, 6, 7].map((ordinal) => ({
    ...dayRow(ordinal),
    label: weekdayOf(ordinal, true).canonicalLabel,
    weekdayHint: ordinal,
  }));
}

beforeEach(() => {
  vi.clearAllMocks();
  listCycles.mockResolvedValue([cycle()]);
  listCycleDays.mockResolvedValue([]);
  cycleShape.mockResolvedValue(
    shapeOf(coverageOf(10, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10])),
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
  cycleShape.mockResolvedValue(shapeOf(coverageOf(dayCount, missing)));

  render(<CyclesAndCycleDays school={SCHOOL} />);
  await screen.findByRole("button", { name: "Add cycle day" });
  await waitFor(() => expect(cycleShape).toHaveBeenCalled());
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
    // The only fields that remain.
    expect(screen.getByLabelText(/Cycle name/)).toBeTruthy();
    expect(screen.getByLabelText(/Rotation length/)).toBeTruthy();
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
    await userEvent.clear(screen.getByLabelText(/Rotation length/));
    await userEvent.type(screen.getByLabelText(/Rotation length/), "6");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toEqual({
      schoolId: "school-1",
      name: "Six-day rotation",
      // The type the select starts on. A rotation is the safe default: it asserts nothing
      // about a week.
      kind: "CUSTOM",
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
    await userEvent.clear(screen.getByLabelText(/Rotation length/));
    await userEvent.type(screen.getByLabelText(/Rotation length/), "0");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    expect(await screen.findByText(/Rotation length cannot be less than 1/)).toBeTruthy();
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
    ).toEqual(["Name", "Type", "Days", "Actions"]);
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
      // Read from the row and sent straight back. The backend refuses a change, and the
      // select is disabled, so this is the only value an edit can carry.
      kind: "CUSTOM",
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
      "All cycle positions are filled. Increase the rotation length to add more days.",
    );
    // Visible as well as on the title: a disabled button is not focusable, so a tooltip
    // alone would be unreachable by keyboard and by touch.
    expect(
      screen.getByText(
        "All cycle positions are filled. Increase the rotation length to add more days.",
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
    let release: (value: CycleShape) => void = () => {};
    cycleShape.mockReturnValue(
      new Promise<CycleShape>((resolve) => {
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

    release(shapeOf(coverageOf(5, [])));
    await waitFor(() => expect(addingEnabled()).toBe(false));
  });

  it("re-enables adding once the cycle is lengthened", async () => {
    await withCoverage(5, []);
    await screen.findByText(/All cycle positions are filled/);
    expect(addingEnabled()).toBe(false);

    // What editing the cycle to six days looks like from here: the backend reports a free
    // position, and the panel follows.
    cycleShape.mockResolvedValue(shapeOf(coverageOf(6, [6])));
    listCycles.mockResolvedValue([cycle({ dayCount: 6 })]);
    updateCycle.mockResolvedValue(cycle({ dayCount: 6 }));

    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));
    await userEvent.clear(screen.getByLabelText(/Rotation length/));
    await userEvent.type(screen.getByLabelText(/Rotation length/), "6");
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
    // The length field talks about positions the school repeats, never about a week.
    expect(
      screen.getByText(/whatever the school actually repeats/),
    ).toBeTruthy();
  });
});

// ===================================================================== the cycle type

/** Renders with a weekly cycle selected, for a school keeping `week`. */
async function withWeeklyCycle(week: readonly number[] = [1, 2, 3, 4, 5]) {
  listCycles.mockResolvedValue([weeklyCycle()]);
  listCycleDays.mockResolvedValue(weeklyDayRows());
  cycleShape.mockResolvedValue(weeklyShapeOf(week));

  render(<CyclesAndCycleDays school={{ ...SCHOOL, workingDays: [...week] } as School} />);
  await screen.findByRole("button", { name: "Edit School week" });
  await waitFor(() => expect(cycleShape).toHaveBeenCalled());
  await waitFor(() => expect(screen.queryByText(/Loading cycle days/)).toBeNull());
}

describe("choosing the cycle type", () => {
  it("offers both kinds on the new-cycle form, custom as the starting value", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    const select = screen.getByLabelText(/Cycle type/) as HTMLSelectElement;
    expect(select.disabled).toBe(false);
    expect([...select.options].map((option) => option.value)).toEqual([
      "WEEKLY",
      "CUSTOM",
    ]);
    // A rotation is the safe starting value: it asserts nothing about a week.
    expect(select.value).toBe("CUSTOM");
  });

  it("hides the rotation length once Weekly is chosen", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));
    expect(screen.getByLabelText(/Rotation length/)).toBeTruthy();

    await userEvent.selectOptions(screen.getByLabelText(/Cycle type/), "WEEKLY");

    // A week is seven days. Offering the field would invite a number the backend ignores.
    expect(screen.queryByLabelText(/Rotation length/)).toBeNull();
    // And switching back brings it straight back.
    await userEvent.selectOptions(screen.getByLabelText(/Cycle type/), "CUSTOM");
    expect(screen.getByLabelText(/Rotation length/)).toBeTruthy();
  });

  it("creates a weekly cycle from a name alone", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await userEvent.selectOptions(screen.getByLabelText(/Cycle type/), "WEEKLY");
    await userEvent.type(screen.getByLabelText(/Cycle name/), "School week");
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    await waitFor(() => expect(createCycle).toHaveBeenCalled());
    expect(sent(createCycle)).toEqual({
      schoolId: "school-1",
      name: "School week",
      kind: "WEEKLY",
      // Carried because the column is `CHECK (day_count >= 1)` and the input has no
      // optional length. The backend derives the stored value from the type regardless.
      dayCount: 7,
      weekCount: 1,
      isDefault: false,
    });
  });

  it("never asks a weekly cycle for a length, even when refusing the save", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));
    await userEvent.selectOptions(screen.getByLabelText(/Cycle type/), "WEEKLY");

    // Submitting with no name must complain about the name only -- never about a length
    // the form does not show, which would be a message pinned to nothing.
    await userEvent.click(screen.getByRole("button", { name: "Add cycle" }));
    expect(await screen.findByText(/Cycle name is required/)).toBeTruthy();
    expect(screen.queryByText(/Rotation length is required/)).toBeNull();
    expect(createCycle).not.toHaveBeenCalled();
  });

  it("will not let an existing cycle change type, and says why", async () => {
    await shown();
    await userEvent.click(screen.getByRole("button", { name: "Edit Two-week rotation" }));

    const select = screen.getByLabelText(/Cycle type/) as HTMLSelectElement;
    expect(select.value).toBe("CUSTOM");
    expect(select.disabled).toBe(true);
    // The reason is visible text, not a tooltip: a disabled control is not focusable.
    expect(screen.getByText(/A cycle's type is fixed when it is created/)).toBeTruthy();
  });

  it("shows the type in the table and in the picker", async () => {
    listCycles.mockResolvedValue([weeklyCycle(), cycle({ id: "cycle-2" })]);
    cycleShape.mockResolvedValue(weeklyShapeOf([1, 2, 3, 4, 5]));
    render(<CyclesAndCycleDays school={SCHOOL} />);
    await screen.findByRole("button", { name: "Edit School week" });

    expect(screen.getByRole("columnheader", { name: "Type" })).toBeTruthy();
    const picker = await screen.findByRole("group", { name: "Cycle" });
    expect(within(picker).getByText(/Weekly . 7 days/)).toBeTruthy();
    expect(within(picker).getByText(/Custom . 10 days/)).toBeTruthy();
  });
});

describe("a weekly cycle", () => {
  it("says its days are managed automatically and offers no way to add one", async () => {
    await withWeeklyCycle();

    expect(screen.getByText(/managed automatically/)).toBeTruthy();
    expect(addingEnabled()).toBe(false);
    expect(addDayButton().title).toBe(
      "A weekly cycle has the seven days of the week, created with it. There is no position to add.",
    );
    expect(
      screen.getByText(
        "A weekly cycle has the seven days of the week, created with it. There is no position to add.",
      ),
    ).toBeTruthy();
  });

  it("holds all seven days and marks the five this school teaches on", async () => {
    await withWeeklyCycle([1, 2, 3, 4, 5]);

    const table = screen.getByRole("table", {
      name: /seven days of this weekly cycle/i,
    });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(7);
    expect(rows.map((row) => within(row).getAllByRole("cell")[3]?.textContent)).toEqual([
      "Yes",
      "Yes",
      "Yes",
      "Yes",
      "Yes",
      "Not a working day here",
      "Not a working day here",
    ]);
    expect(screen.getByText(/5 of the 7/)).toBeTruthy();
  });

  /** The requirement the whole design exists for, from the screen's side. */
  it("holds the same seven days for a school that works Sunday to Thursday", async () => {
    await withWeeklyCycle([7, 1, 2, 3, 4]);

    const table = screen.getByRole("table", {
      name: /seven days of this weekly cycle/i,
    });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(7);
    expect(rows.map((row) => within(row).getAllByRole("cell")[3]?.textContent)).toEqual([
      "Yes",
      "Yes",
      "Yes",
      "Yes",
      "Not a working day here",
      "Not a working day here",
      "Yes",
    ]);
    // The cycle is unchanged by being asked. Only a read happened.
    expect(updateCycle).not.toHaveBeenCalled();
    expect(updateCycleDay).not.toHaveBeenCalled();
    expect(createCycleDay).not.toHaveBeenCalled();
    expect(deleteCycleDay).not.toHaveBeenCalled();
  });

  it("sends the school's working week with the question", async () => {
    await withWeeklyCycle([1, 2, 3, 4, 5, 6]);
    expect(cycleShape).toHaveBeenCalledWith("cycle-1", [1, 2, 3, 4, 5, 6]);
  });

  it("shows no start date and no holiday policy", async () => {
    await withWeeklyCycle();

    // Section 3 of the brief: these belong to Calendar Mapping, per school, and must not
    // appear on a shared definition.
    for (const forbidden of [
      /start date/i,
      /starting day/i,
      /holiday/i,
      /first day of term/i,
    ]) {
      expect(screen.queryByLabelText(forbidden)).toBeNull();
    }
    // No control of any kind collects them, which is stronger than "no such label":
    // the only inputs on screen belong to the date-free panels above.
    expect(screen.queryByRole("textbox", { name: /date|holiday|start/i })).toBeNull();
    // And the screen says so, rather than leaving their absence to be noticed.
    expect(
      screen.getByText(/it holds no dates, no term start and no holidays/),
    ).toBeTruthy();
  });

  it("lets a weekday be renamed but not repositioned", async () => {
    await withWeeklyCycle();
    await userEvent.click(screen.getByRole("button", { name: "Edit Monday" }));

    // Only the label: a weekly day's position and weekday are fixed, and the schema
    // refuses a day whose weekday disagrees with its position.
    expect(screen.getByLabelText(/Label/)).toBeTruthy();
    expect(screen.queryByLabelText(/Position/)).toBeNull();
    expect(screen.queryByLabelText(/Weekday hint/)).toBeNull();

    await userEvent.clear(screen.getByLabelText(/Label/));
    await userEvent.type(screen.getByLabelText(/Label/), "Lundi");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateCycleDay).toHaveBeenCalled());
    expect(updateCycleDay).toHaveBeenCalledWith("day-1", {
      cycleId: "cycle-1",
      ordinal: 1,
      label: "Lundi",
      weekdayHint: 1,
    });
  });

  it("names a missing weekday rather than quietly holding six days", async () => {
    listCycles.mockResolvedValue([weeklyCycle()]);
    listCycleDays.mockResolvedValue(weeklyDayRows().slice(0, 6));
    const shape = weeklyShapeOf([1, 2, 3, 4, 5]);
    cycleShape.mockResolvedValue({
      ...shape,
      weekdays: shape.weekdays.map((day) =>
        day.ordinal === 7 ? { ...day, cycleDayId: null, label: null } : day,
      ),
    });

    render(<CyclesAndCycleDays school={SCHOOL} />);
    await screen.findByRole("button", { name: "Edit School week" });

    expect(await screen.findByText(/One day of the week is missing/)).toBeTruthy();
  });

  it("shows no completeness panel and no rotation preview", async () => {
    await withWeeklyCycle();

    // Those are the custom cycle's panels. A weekly cycle's positions are not the user's
    // to fill, and "The week" says everything a weekly cycle has to say.
    expect(screen.queryByText("Cycle completeness")).toBeNull();
    expect(screen.queryByText("Rotation")).toBeNull();
  });
});

describe("a custom cycle", () => {
  it("previews the rotation and where it comes back round", async () => {
    listCycles.mockResolvedValue([cycle({ dayCount: 5 })]);
    listCycleDays.mockResolvedValue([1, 2, 3, 4, 5].map(dayRow));
    cycleShape.mockResolvedValue(shapeOf(coverageOf(5, [])));

    render(<CyclesAndCycleDays school={SCHOOL} />);
    await screen.findByRole("button", { name: "Edit Two-week rotation" });

    const preview = await screen.findByText(/comes back round to/);
    expect(preview.textContent).toMatch(
      /After Day 5 the rotation comes back round to Day 1/,
    );
    // And it says where dates come from, because they do not come from here.
    expect(preview.textContent).toMatch(/set in Calendar Mapping/);
  });

  it("renders the rotation in the labels the user chose", async () => {
    listCycles.mockResolvedValue([cycle({ dayCount: 3 })]);
    listCycleDays.mockResolvedValue([
      { ...dayRow(1), label: "Red" },
      { ...dayRow(2), label: "Amber" },
      { ...dayRow(3), label: "Green" },
    ]);
    cycleShape.mockResolvedValue(shapeOf(coverageOf(3, [])));

    render(<CyclesAndCycleDays school={SCHOOL} />);
    await screen.findByRole("button", { name: "Edit Two-week rotation" });

    const preview = await screen.findByText(/comes back round to/);
    expect(preview.textContent).toMatch(
      /After Green the rotation comes back round to Red/,
    );
  });

  it("shows no weekday column and no week table", async () => {
    await withCoverage(5, []);

    expect(screen.queryByRole("columnheader", { name: "Weekday" })).toBeNull();
    expect(screen.queryByText("The week")).toBeNull();
    expect(screen.queryByText(/Not a working day here/)).toBeNull();
  });
});
