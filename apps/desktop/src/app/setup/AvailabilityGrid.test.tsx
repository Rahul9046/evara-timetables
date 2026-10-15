/**
 * The availability editor.
 *
 * The repository tests own the rules — what collides, what a delete refuses, what reaches
 * the change feed. This file owns what only the interface can get wrong, and every case
 * here is one a reasonable implementation gets wrong in the obvious direction:
 *
 * - a blank cell is **available**, and clearing one *removes* a rule rather than storing
 *   an `AVAILABLE` status that does not exist
 * - the columns are the cycle's days, never Monday–Friday, on any cycle
 * - a cell is reachable and operable from the keyboard alone
 * - each cell says who, when and what it currently is, because its position on screen is
 *   not available to someone listening to it
 * - rules the grid cannot show are reported, not silently hidden
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { AvailabilityRule, GridPreview, School } from "@evara/domain";

const listCycles = vi.fn();
const listPeriodStructures = vi.fn();
const previewGrid = vi.fn<(request: unknown) => Promise<GridPreview>>();
const listAvailability =
  vi.fn<(kind: unknown, id: unknown) => Promise<AvailabilityRule[]>>();
const setAvailability = vi.fn();
const clearAvailability = vi.fn();

vi.mock("../../ipc/setup", () => ({
  listCycles: () => listCycles(),
  listPeriodStructures: () => listPeriodStructures(),
  previewGrid: (request: unknown) => previewGrid(request),
  listAvailability: (kind: unknown, id: unknown) => listAvailability(kind, id),
  setAvailability: (input: unknown) => setAvailability(input),
  clearAvailability: (scope: unknown) => clearAvailability(scope),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
}));

const { AvailabilityGrid } = await import("./AvailabilityGrid");

const SCHOOL = { id: "school-1" } as School;
const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

/** A grid preview with `days` cycle positions and two periods. */
function preview(dayLabels: readonly string[]): GridPreview {
  return {
    cycleId: "cycle-1",
    periodStructureId: "ps-1",
    termId: null,
    days: dayLabels.map((label, index) => ({
      id: `day-${index + 1}`,
      cycleId: "cycle-1",
      ordinal: index + 1,
      label,
      weekdayHint: null,
      ...STAMP,
    })),
    periods: [1, 2].map((ordinal) => ({
      id: `period-${ordinal}`,
      periodStructureId: "ps-1",
      ordinal,
      label: `P${ordinal}`,
      startsAt: `0${8 + ordinal}:00`,
      endsAt: `0${8 + ordinal}:50`,
      kind: "TEACHING",
      countsAsLoad: true,
      ...STAMP,
    })),
    cells: [],
    coverage: { declared: dayLabels.length, present: dayLabels.length, missingOrdinals: [] },
  } as unknown as GridPreview;
}

function rule(over: Partial<AvailabilityRule> = {}): AvailabilityRule {
  return {
    id: "rule-1",
    schoolId: "school-1",
    entityKind: "TEACHER",
    entityId: "t-1",
    timeslotId: null,
    cycleDayId: "day-1",
    periodId: "period-1",
    termId: null,
    status: "UNAVAILABLE",
    weight: null,
    reason: null,
    effectiveFrom: null,
    effectiveTo: null,
    ...STAMP,
    ...over,
  } as AvailabilityRule;
}

function renderGrid() {
  return render(
    <AvailabilityGrid
      school={SCHOOL}
      entityKind="TEACHER"
      entityId="t-1"
      entityName="Amara Okafor"
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  listCycles.mockResolvedValue([
    { id: "cycle-1", name: "Main cycle", isDefault: true, ...STAMP },
  ]);
  listPeriodStructures.mockResolvedValue([
    { id: "ps-1", name: "Standard day", isDefault: true, ...STAMP },
  ]);
  previewGrid.mockResolvedValue(preview(["Mon", "Tue"]));
  listAvailability.mockResolvedValue([]);
  setAvailability.mockResolvedValue(rule());
  clearAvailability.mockResolvedValue(true);
});

describe("the default state", () => {
  it("shows every cell as available when there are no rules", async () => {
    renderGrid();

    const cells = await screen.findAllByRole("button", { name: /available$/ });
    // Two days x two periods, all free.
    expect(cells).toHaveLength(4);
  });

  it("names the entity, the day, the period and the status on every cell", async () => {
    // A cell's meaning is carried entirely by its position and its colour, neither of which
    // reaches someone using a screen reader.
    listAvailability.mockResolvedValue([rule()]);
    renderGrid();

    expect(
      await screen.findByRole("button", {
        name: "Amara Okafor, Mon, P1: unavailable",
      }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Amara Okafor, Tue, P2: available" }),
    ).toBeTruthy();
  });
});

describe("editing", () => {
  it("makes an available cell unavailable", async () => {
    const user = userEvent.setup();
    renderGrid();

    await user.click(
      await screen.findByRole("button", { name: "Amara Okafor, Mon, P1: available" }),
    );

    await waitFor(() => expect(setAvailability).toHaveBeenCalled());
    expect(setAvailability.mock.calls[0]?.[0]).toMatchObject({
      entityKind: "TEACHER",
      entityId: "t-1",
      cycleDayId: "day-1",
      periodId: "period-1",
      timeslotId: null,
      termId: null,
      status: "UNAVAILABLE",
    });
    expect(clearAvailability).not.toHaveBeenCalled();
  });

  it("cycles through the statuses and back to available", async () => {
    const user = userEvent.setup();

    // Preferred is the last stop before available, so this is the step that must clear.
    listAvailability.mockResolvedValue([rule({ status: "PREFERRED" })]);
    renderGrid();

    await user.click(
      await screen.findByRole("button", { name: "Amara Okafor, Mon, P1: preferred" }),
    );

    await waitFor(() => expect(clearAvailability).toHaveBeenCalled());
    expect(
      setAvailability,
      "returning to available must remove the rule, not store an AVAILABLE status",
    ).not.toHaveBeenCalled();
  });

  it("steps unavailable to discouraged rather than straight back to available", async () => {
    const user = userEvent.setup();
    listAvailability.mockResolvedValue([rule({ status: "UNAVAILABLE" })]);
    renderGrid();

    await user.click(
      await screen.findByRole("button", { name: "Amara Okafor, Mon, P1: unavailable" }),
    );

    await waitFor(() => expect(setAvailability).toHaveBeenCalled());
    expect(setAvailability.mock.calls[0]?.[0]).toMatchObject({ status: "DISCOURAGED" });
  });
});

describe("the keyboard", () => {
  it("has one tab stop for the whole grid", async () => {
    // Fifty separate tab stops is technically reachable and practically unusable.
    renderGrid();
    await screen.findAllByRole("button", { name: /available$/ });

    const reachable = screen
      .getAllByRole("button")
      .filter((button) => button.getAttribute("tabindex") === "0");
    expect(reachable).toHaveLength(1);
  });

  it("moves between cells with the arrow keys", async () => {
    const user = userEvent.setup();
    renderGrid();

    const first = await screen.findByRole("button", {
      name: "Amara Okafor, Mon, P1: available",
    });
    first.focus();

    await user.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "Amara Okafor, Tue, P1: available" }),
    );

    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "Amara Okafor, Tue, P2: available" }),
    );

    await user.keyboard("{ArrowLeft}{ArrowUp}");
    expect(document.activeElement).toBe(first);
  });

  it("does not wrap or escape at the edges", async () => {
    const user = userEvent.setup();
    renderGrid();

    const first = await screen.findByRole("button", {
      name: "Amara Okafor, Mon, P1: available",
    });
    first.focus();

    await user.keyboard("{ArrowLeft}{ArrowUp}");
    expect(document.activeElement).toBe(first);
  });

  it("sets a cell from the keyboard alone", async () => {
    const user = userEvent.setup();
    renderGrid();

    const first = await screen.findByRole("button", {
      name: "Amara Okafor, Mon, P1: available",
    });
    first.focus();
    await user.keyboard("{Enter}");

    await waitFor(() => expect(setAvailability).toHaveBeenCalled());
    expect(setAvailability.mock.calls[0]?.[0]).toMatchObject({ status: "UNAVAILABLE" });
  });
});

describe("the cycle, not the week", () => {
  it("renders a six-day rotation with its own labels", async () => {
    // Nothing here may assume Monday–Friday. A rotation that drifts against the calendar
    // must render exactly as a week does.
    previewGrid.mockResolvedValue(preview(["Day 1", "Day 2", "Day 3", "Day 4", "Day 5", "Day 6"]));
    renderGrid();

    const table = await screen.findByRole("table");
    const headers = within(table)
      .getAllByRole("columnheader")
      .map((header) => header.textContent);

    expect(headers).toEqual(["Period", "Day 1", "Day 2", "Day 3", "Day 4", "Day 5", "Day 6"]);
  });

  it("renders custom day labels a school chose", async () => {
    previewGrid.mockResolvedValue(preview(["Red", "Amber", "Green"]));
    renderGrid();

    const table = await screen.findByRole("table");
    expect(within(table).getByRole("columnheader", { name: "Red" })).toBeTruthy();
    expect(within(table).getByRole("columnheader", { name: "Green" })).toBeTruthy();
  });
});

describe("rules the grid cannot show", () => {
  it("says so rather than hiding them", async () => {
    // A whole-day rule is real, is in effect, and is invisible in a cell grid. Silently
    // omitting it would make the screen a lie about what is configured.
    listAvailability.mockResolvedValue([
      rule({ id: "rule-2", periodId: null }),
      rule({ id: "rule-3", cycleDayId: null }),
    ]);
    renderGrid();

    expect(await screen.findByText(/2 rules are not shown above/i)).toBeTruthy();
  });

  it("stays quiet when every rule is a plain cell", async () => {
    listAvailability.mockResolvedValue([rule()]);
    renderGrid();

    await screen.findByRole("table");
    expect(screen.queryByText(/not shown above/i)).toBeNull();
  });
});

describe("a rule scoped to one term", () => {
  // The grid writes all-year rules only, so a term-scoped cell is a rule of another
  // granularity that happens to name a cell. Drawing it would claim an all-year
  // restriction that is not there, and the next click would write a second rule beside
  // it rather than editing the one on screen.
  it("is counted as not shown rather than drawn in its cell", async () => {
    listAvailability.mockResolvedValue([rule({ id: "rule-4", termId: "term-1" })]);
    renderGrid();

    expect(await screen.findByText(/1 rule is not shown above/i)).toBeTruthy();
    const table = await screen.findByRole("table");
    expect(
      within(table).getByRole("button", {
        name: "Amara Okafor, Mon, P1: available",
      }),
    ).toBeTruthy();
  });

  it("is left alone when the same cell is edited for the whole year", async () => {
    listAvailability.mockResolvedValue([rule({ id: "rule-4", termId: "term-1" })]);
    renderGrid();

    const table = await screen.findByRole("table");
    await userEvent.click(
      within(table).getByRole("button", { name: "Amara Okafor, Mon, P1: available" }),
    );

    await waitFor(() => expect(setAvailability).toHaveBeenCalledTimes(1));
    expect(setAvailability.mock.calls[0]?.[0]).toMatchObject({
      cycleDayId: "day-1",
      periodId: "period-1",
      termId: null,
      status: "UNAVAILABLE",
    });
    expect(clearAvailability).not.toHaveBeenCalled();
  });
});
