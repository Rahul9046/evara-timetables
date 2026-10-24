/**
 * The timetable grid.
 *
 * The Rust tests own the layout — which cell holds what, which lesson earns a `rowSpan`,
 * whether a panel is a complete rectangle. This file owns what only the interface can get
 * wrong, and every case here is one a reasonable implementation gets wrong in the obvious
 * direction:
 *
 * - a multi-period lesson is **one** `<td>` with the `rowSpan` the backend decided, and the
 *   positions it covers render **nothing** — emitting a `<td>` for a continuation is how a
 *   table silently loses a column;
 * - a pinned cell renders every lesson it holds, so a clash hides neither of them;
 * - two bell schedules are two tables and a sentence, never one merged axis;
 * - a break is a row and is marked, because an empty cell says "free" and a break is not;
 * - every cell says who, when and where out loud, because its position on screen is not
 *   available to somebody listening to it;
 * - the whole grid is reachable from the keyboard alone;
 * - a span defect is rendered rather than swallowed, which is the state a stored timetable
 *   reaches after its grid moves.
 *
 * The editing section at the bottom is held to the same division. Whether a cell may take a
 * lesson is Rust's answer and is asserted nowhere here; what is asserted is that the grid offers
 * a drop exactly where it was told one is legal, refuses one where it was not, renders the
 * refusing rule's own sentence, and reaches all of it from the keyboard.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  CandidateGrid,
  CellContent,
  GridPanel,
  ProjectedDay,
  ProjectedLesson,
  ProjectedPeriod,
  ScopeGrid,
  ScopeKind,
  SpanDefect,
  TimetableCell,
} from "@evara/domain";

import { TimetableGrid, type Editing } from "./TimetableGrid";

// ---------------------------------------------------------------- fixtures

const DAYS: ProjectedDay[] = [
  { cycleDayId: "d1", ordinal: 1, label: "Day A" },
  { cycleDayId: "d2", ordinal: 2, label: "Day B" },
];

/** Three teaching periods and a break, so a run can be broken and a row can be unavailable. */
const PERIODS: ProjectedPeriod[] = [
  { periodId: "p1", ordinal: 1, label: "1", startsAt: "09:00", endsAt: "10:00", isTeaching: true },
  { periodId: "p2", ordinal: 2, label: "2", startsAt: "10:00", endsAt: "11:00", isTeaching: true },
  {
    periodId: "pb",
    ordinal: 3,
    label: "Break",
    startsAt: "11:00",
    endsAt: "11:30",
    isTeaching: false,
  },
  { periodId: "p3", ordinal: 4, label: "3", startsAt: "11:30", endsAt: "12:30", isTeaching: true },
];

function aLesson(over: Partial<ProjectedLesson> = {}): ProjectedLesson {
  return {
    activityId: "a1",
    sectionId: "s1",
    label: {
      sectionId: "s1",
      subject: "Mathematics",
      course: "Mathematics Year 9",
      section: "B",
      yearLevel: "9",
      campus: null,
      short: "9 Mathematics B",
    },
    activityName: null,
    teacherIds: ["t1"],
    teachers: ["A. Okafor"],
    groupIds: ["g1"],
    groups: ["9B"],
    roomId: "r1",
    room: { roomId: "r1", name: "Room B12", building: "Main" },
    scheduleId: "bs1",
    cycleDayId: "d1",
    dayOrdinal: 1,
    occupies: [{ timeslotId: "ts1", dayOrdinal: 1, periodOrdinal: 1 }],
    startsAt: "09:00",
    endsAt: "10:00",
    durationPeriods: 1,
    defect: null,
    flags: { isLocked: false, isManual: false },
    ...over,
  };
}

/** A complete rectangle of cells, defaulting to empty, with the named overrides applied. */
function cells(
  periods: ProjectedPeriod[],
  days: ProjectedDay[],
  over: Record<string, CellContent> = {},
): TimetableCell[] {
  const out: TimetableCell[] = [];
  for (const period of periods) {
    for (const day of days) {
      out.push({
        dayOrdinal: day.ordinal,
        periodOrdinal: period.ordinal,
        content: over[`${day.ordinal}-${period.ordinal}`] ?? { kind: "empty" },
      });
    }
  }
  return out;
}

function aPanel(over: Partial<GridPanel> = {}): GridPanel {
  return {
    scheduleId: "bs1",
    scheduleName: "Standard Day",
    campusName: "Main",
    days: DAYS,
    periods: PERIODS,
    cells: cells(PERIODS, DAYS),
    ...over,
  };
}

function aGrid(over: Partial<ScopeGrid> = {}, kind: ScopeKind = "section"): ScopeGrid {
  return {
    scope: {
      kind,
      id: "s1",
      name: "9 Mathematics B",
      secondary: null,
      lessonCount: 1,
      rosterDerived: kind === "student",
      ...(over.scope ?? {}),
    },
    panels: [aPanel()],
    conflicts: [],
    defective: [],
    ...over,
  };
}

// ---------------------------------------------------------------- spans

describe("a multi-period lesson", () => {
  it("renders one cell with the rowSpan the backend decided, and nothing for what it covers", () => {
    const lesson = aLesson({
      durationPeriods: 2,
      endsAt: "11:00",
      occupies: [
        { timeslotId: "ts1", dayOrdinal: 1, periodOrdinal: 1 },
        { timeslotId: "ts2", dayOrdinal: 1, periodOrdinal: 2 },
      ],
    });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "lesson", lesson: 0, rows: 2 },
            "1-2": { kind: "continuation" },
          }),
        }),
      ],
    });

    render(<TimetableGrid grid={grid} lessons={[lesson]} />);

    const named = screen.getAllByText("9 Mathematics B");
    expect(named).toHaveLength(1);

    const cell = named[0]!.closest("td");
    expect(cell).not.toBeNull();
    // The rowSpan is the backend's number, not a count the component worked out.
    expect(cell!.getAttribute("rowspan")).toBe("2");

    // Period 1 has a two-row cell plus one empty for Day B; period 2 has only Day B's,
    // because the continuation renders nothing at all. A `<td>` there would push Day B's
    // cell into the wrong column.
    const rows = screen.getAllByRole("row");
    const periodOne = rows.find((row) => within(row).queryByText("1") !== null);
    const periodTwo = rows.find((row) => within(row).queryByText("2") !== null);
    expect(periodOne!.querySelectorAll("td")).toHaveLength(2);
    expect(periodTwo!.querySelectorAll("td")).toHaveLength(1);
  });

  it("announces the whole span once, with both clock times", () => {
    const lesson = aLesson({
      durationPeriods: 2,
      endsAt: "11:00",
      occupies: [
        { timeslotId: "ts1", dayOrdinal: 1, periodOrdinal: 1 },
        { timeslotId: "ts2", dayOrdinal: 1, periodOrdinal: 2 },
      ],
    });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "lesson", lesson: 0, rows: 2 },
            "1-2": { kind: "continuation" },
          }),
        }),
      ],
    });

    render(<TimetableGrid grid={grid} lessons={[lesson]} />);
    const labelled = screen.getByLabelText(/Day A, 2 periods from 1/);
    const label = labelled.getAttribute("aria-label") ?? "";
    expect(label).toContain("9 Mathematics B");
    expect(label).toContain("A. Okafor");
    expect(label).toContain("Room B12");
    expect(label).toContain("09:00 to 11:00");
  });
});

// ---------------------------------------------------------------- pinned cells

describe("a cell holding more than one lesson", () => {
  it("renders every lesson, so a clash hides neither of them", () => {
    const a = aLesson({ activityId: "a1" });
    const b = aLesson({
      activityId: "a2",
      sectionId: "s2",
      label: { ...aLesson().label!, sectionId: "s2", section: "C", short: "9 Mathematics C" },
    });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "pinned", lessons: [0, 1], reason: "conflict" },
          }),
        }),
      ],
      conflicts: [
        {
          scheduleId: "bs1",
          dayOrdinal: 1,
          dayLabel: "Day A",
          periodOrdinal: 1,
          periodLabel: "1",
          lessons: [0, 1],
        },
      ],
    });

    render(<TimetableGrid grid={grid} lessons={[a, b]} />);
    expect(screen.getByText("9 Mathematics B")).toBeTruthy();
    expect(screen.getByText("9 Mathematics C")).toBeTruthy();
  });

  it("gives a clash rowSpan 1, so no span can cross a cell somebody else is in", () => {
    const a = aLesson({ activityId: "a1" });
    const b = aLesson({ activityId: "a2" });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "pinned", lessons: [0, 1], reason: "conflict" },
          }),
        }),
      ],
    });

    render(<TimetableGrid grid={grid} lessons={[a, b]} />);
    const cell = screen.getAllByText("9 Mathematics B")[0]!.closest("td");
    // `null` or "1" both mean one row; what must not happen is a span.
    expect(["1", null]).toContain(cell!.getAttribute("rowspan"));
  });

  it("says a clash is a clash, in words and not only in colour", () => {
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "pinned", lessons: [0], reason: "conflict" },
          }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[aLesson()]} />);
    expect(screen.getByText("Clash")).toBeTruthy();
    expect(screen.getByLabelText(/^Clash\./)).toBeTruthy();
  });

  it("does not call a shared master-grid cell a clash", () => {
    const grid = aGrid(
      {
        panels: [
          aPanel({
            cells: cells(PERIODS, DAYS, {
              "1-1": { kind: "pinned", lessons: [0], reason: "shared" },
            }),
          }),
        ],
        scope: {
          kind: "master",
          id: null,
          name: "All lessons",
          secondary: null,
          lessonCount: 1,
          rosterDerived: false,
        },
      },
      "master",
    );
    render(<TimetableGrid grid={grid} lessons={[aLesson()]} />);
    expect(screen.queryByText("Clash")).toBeNull();
    expect(screen.getByText("Several lessons")).toBeTruthy();
  });

  it("marks a broken run and gives it no span", () => {
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, {
            "1-1": { kind: "pinned", lessons: [0], reason: "brokenSpan" },
            "1-4": { kind: "pinned", lessons: [0], reason: "brokenSpan" },
          }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[aLesson()]} />);
    expect(screen.getAllByText("Broken run")).toHaveLength(2);
  });
});

// ---------------------------------------------------------------- bell schedules

describe("two bell schedules", () => {
  it("render as two tables and say they cannot be read as one week", () => {
    const grid = aGrid({
      panels: [
        aPanel(),
        aPanel({
          scheduleId: "bs2",
          scheduleName: "Early Day",
          campusName: "Southside",
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[aLesson()]} />);

    expect(screen.getAllByRole("table")).toHaveLength(2);
    expect(screen.getByText("Standard Day")).toBeTruthy();
    expect(screen.getByText("Early Day")).toBeTruthy();
    expect(screen.getByText(/cannot be read as a single week/)).toBeTruthy();
  });

  it("says nothing of the kind when there is only one", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    expect(screen.queryByText(/cannot be read as a single week/)).toBeNull();
  });
});

// ---------------------------------------------------------------- the axes

describe("the axes", () => {
  it("are the cycle's days, never a week", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    const headers = screen.getAllByRole("columnheader").map((h) => h.textContent);
    expect(headers).toContain("Day A");
    expect(headers).toContain("Day B");
    expect(headers.join(" ")).not.toMatch(/Monday|Tuesday/);
  });

  it("render a break as a row, marked as having no teaching", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    expect(screen.getByText("Break")).toBeTruthy();
    expect(screen.getByText("(no teaching)")).toBeTruthy();
    // And the cell says so rather than reading as free.
    expect(screen.getAllByLabelText(/Break: not a teaching period/).length).toBeGreaterThan(0);
  });

  it("call an unoccupied teaching cell free", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    expect(screen.getAllByLabelText(/Day A, 1: free/).length).toBe(1);
  });

  it("render one column when a single day is chosen, and keep the table well formed", () => {
    // Showing one day is the only filtering this component does, and it is safe because it
    // removes whole cells rather than part of a span: every row still renders as many cells
    // as there are visible columns.
    const lesson = aLesson();
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[lesson]} shownDay={2} />);

    const headers = screen
      .getAllByRole("columnheader")
      .map((h) => h.textContent)
      .filter((text) => text !== "Period");
    expect(headers).toEqual(["Day B"]);

    for (const row of screen.getAllByRole("row").slice(1)) {
      expect(row.querySelectorAll("td")).toHaveLength(1);
    }
    // Day A's lesson is hidden, not relocated.
    expect(screen.queryByText("9 Mathematics B")).toBeNull();
  });

  it("stops the keyboard at the one visible column", async () => {
    const user = userEvent.setup();
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} shownDay={1} />);
    await user.tab();
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement!.getAttribute("data-cell")).toBe("0-0-0");
  });

  it("use a real table with row and column scopes", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    const table = screen.getAllByRole("table")[0]!;
    expect(table.querySelector("caption")).not.toBeNull();
    for (const header of table.querySelectorAll("thead th")) {
      expect(header.getAttribute("scope")).toBe("col");
    }
    for (const header of table.querySelectorAll("tbody th")) {
      expect(header.getAttribute("scope")).toBe("row");
    }
  });
});

// ---------------------------------------------------------------- keyboard

describe("the keyboard", () => {
  it("moves between cells with the arrow keys", async () => {
    const user = userEvent.setup();
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);

    await user.tab();
    const first = document.activeElement;
    expect(first).not.toBeNull();
    expect(first!.getAttribute("data-cell")).toBe("0-0-0");

    await user.keyboard("{ArrowRight}");
    expect(document.activeElement!.getAttribute("data-cell")).toBe("0-1-0");

    await user.keyboard("{ArrowDown}");
    expect(document.activeElement!.getAttribute("data-cell")).toBe("0-1-1");

    await user.keyboard("{Home}");
    expect(document.activeElement!.getAttribute("data-cell")).toBe("0-0-1");
  });

  it("does not move past the edge of the panel", async () => {
    const user = userEvent.setup();
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    await user.tab();
    await user.keyboard("{ArrowLeft}{ArrowUp}");
    expect(document.activeElement!.getAttribute("data-cell")).toBe("0-0-0");
  });

  it("keeps exactly one tab stop in the grid", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    const stops = Array.from(document.querySelectorAll("[data-cell]")).filter(
      (cell) => cell.getAttribute("tabindex") === "0",
    );
    expect(stops).toHaveLength(1);
  });
});

// ---------------------------------------------------------------- defects and flags

describe("a lesson whose run is not sound", () => {
  const defect = (over: Partial<SpanDefect> = {}): SpanDefect => ({
    wanted: 2,
    got: 1,
    crossesNonTeaching: false,
    startsOutsideTeaching: false,
    wrongBellSchedule: false,
    ...over,
  });

  it("is rendered with what is wrong, never silently shortened", () => {
    const lesson = aLesson({ durationPeriods: 2, defect: defect() });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[lesson]} />);
    expect(
      screen.getByText(/needs 2 periods but only 1 are available from here/),
    ).toBeTruthy();
  });

  it("names each kind of defect it carries", () => {
    const lesson = aLesson({
      defect: defect({
        got: 2,
        crossesNonTeaching: true,
        startsOutsideTeaching: true,
        wrongBellSchedule: true,
      }),
    });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[lesson]} />);
    const said = screen.getByText(/runs across a period with no teaching/).textContent ?? "";
    expect(said).toContain("starts in a period with no teaching");
    expect(said).toContain("sits on a bell schedule its class does not use");
  });
});

describe("stored flags", () => {
  it("are shown where set and nowhere else", () => {
    const lesson = aLesson({ flags: { isLocked: true, isManual: true } });
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[lesson]} />);
    expect(screen.getByText("Locked")).toBeTruthy();
    expect(screen.getByText("Placed by hand")).toBeTruthy();
  });

  it("are absent for an ordinary generated lesson", () => {
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(<TimetableGrid grid={grid} lessons={[aLesson()]} />);
    expect(screen.queryByText("Locked")).toBeNull();
    expect(screen.queryByText("Placed by hand")).toBeNull();
  });
});

// ---------------------------------------------------------------- empty states

describe("an empty grid", () => {
  it("says so for an ordinary scope", () => {
    render(<TimetableGrid grid={aGrid({ panels: [] })} lessons={[]} />);
    expect(screen.getByText(/Nothing is scheduled in this timetable for/)).toBeTruthy();
    expect(screen.queryByText(/class enrolment records/)).toBeNull();
  });

  it("warns that a student's empty grid may mean no roster was entered", () => {
    const grid = aGrid(
      {
        panels: [],
        scope: {
          kind: "student",
          id: "st1",
          name: "A. Student",
          secondary: null,
          lessonCount: 0,
          rosterDerived: true,
        },
      },
      "student",
    );
    render(<TimetableGrid grid={grid} lessons={[]} />);
    expect(
      screen.getByText(/may mean no enrolment has been entered rather than that there are no/),
    ).toBeTruthy();
  });
});

// ---------------------------------------------------------------- read-only

describe("the grid is read-only", () => {
  it("offers no drag affordance at all — not even a disabled one", () => {
    render(<TimetableGrid grid={aGrid()} lessons={[aLesson()]} />);
    expect(document.querySelectorAll("[draggable='true']")).toHaveLength(0);
    expect(document.querySelectorAll("[aria-grabbed]")).toHaveLength(0);
    expect(document.querySelectorAll("[aria-dropeffect]")).toHaveLength(0);
  });

  it("reports a selected lesson without changing anything", async () => {
    const user = userEvent.setup();
    const onSelectLesson = vi.fn();
    const grid = aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });
    render(
      <TimetableGrid grid={grid} lessons={[aLesson()]} onSelectLesson={onSelectLesson} />,
    );
    await user.click(screen.getByText("9 Mathematics B"));
    expect(onSelectLesson).toHaveBeenCalledWith(0);
  });
});

// ---------------------------------------------------------------- editing

/**
 * Editing, which decides nothing.
 *
 * Every case here is about the grid **rendering a verdict it was given** rather than forming one.
 * The fixtures therefore state the verdict explicitly — `allowed: true` on one cell and
 * `allowed: false` with the rule's sentence on another — and the assertions are that the grid
 * offered a drop where the backend allowed it, refused one where it did not, and said which rule
 * refused in words a screen reader reaches.
 *
 * The keyboard half is not a nicety. A grid that can only be edited by dragging is a grid half
 * the people who have to use it cannot edit, so every mouse affordance is asserted to have a
 * keyboard equivalent — including on the **second** lesson of a shared cell, which is the one a
 * single-accelerator design silently strands.
 */
describe("editing", () => {
  /** One verdict per cell, as the backend probed them. */
  function aCandidateGrid(over: Partial<CandidateGrid> = {}): CandidateGrid {
    return {
      activityId: "a1",
      scheduleId: "bs1",
      roomId: "r1",
      rooms: [{ roomId: "r1", name: "Room B12" }],
      roomlessAllowed: false,
      cells: [
        {
          timeslotId: "ts-legal",
          dayOrdinal: 2,
          periodOrdinal: 1,
          allowed: true,
          isCurrent: false,
          blocking: [],
          mediumDelta: 0,
        },
        {
          timeslotId: "ts-illegal",
          dayOrdinal: 2,
          periodOrdinal: 2,
          allowed: false,
          isCurrent: false,
          blocking: ["A. Okafor already teaches 9 Science A at this time."],
          mediumDelta: 0,
        },
      ],
      unavailable: null,
      ...over,
    };
  }

  function anEditing(over: Partial<Editing> = {}): Editing {
    return {
      pickedUp: null,
      candidates: null,
      busy: false,
      onPickUp: vi.fn(),
      onDrop: vi.fn(),
      onUnschedule: vi.fn(),
      onToggleLock: vi.fn(),
      ...over,
    };
  }

  /** The default fixture: one lesson at Day A, Period 1 — which is also the grid's tab stop. */
  const withLesson = () =>
    aGrid({
      panels: [
        aPanel({
          cells: cells(PERIODS, DAYS, { "1-1": { kind: "lesson", lesson: 0, rows: 1 } }),
        }),
      ],
    });

  const cellAt = (at: string) => document.querySelector(`[data-cell="${at}"]`)!;

  describe("a drop target", () => {
    it("is offered exactly where the backend allowed one", () => {
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
        />,
      );
      // Day B Period 1 is the allowed cell; Day B Period 2 is the refused one.
      expect(cellAt("0-1-0").getAttribute("data-droppable")).toBe("true");
      expect(cellAt("0-1-1").getAttribute("data-droppable")).toBeNull();
    });

    it("says why it refuses, in the rule's own words", () => {
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
        />,
      );
      expect(cellAt("0-1-1").getAttribute("aria-label")).toContain(
        "A. Okafor already teaches 9 Science A at this time.",
      );
      expect(cellAt("0-1-0").getAttribute("aria-label")).toContain("Legal drop target.");
    });

    it("is not offered for a cell the backend sent no verdict for", () => {
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
        />,
      );
      // Day A Period 2 is in no candidate cell. No verdict is a missing answer, so the cell is
      // neither a target nor marked as refused — it says nothing about dropping at all.
      const quiet = cellAt("0-0-1");
      expect(quiet.getAttribute("data-droppable")).toBeNull();
      expect(quiet.getAttribute("aria-label")).not.toContain("Cannot be dropped here");
    });

    it("is offered nowhere on a panel of a different bell schedule", () => {
      const grid = aGrid({
        panels: [
          aPanel({ scheduleId: "bs2", scheduleName: "Southmoor Day" }),
          aPanel({ scheduleId: "bs1" }),
        ],
      });
      render(
        <TimetableGrid
          grid={grid}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
        />,
      );
      // Panel 0 is Southmoor's; the candidates are Standard Day's. A lesson may only sit in slots
      // of the schedule its class resolved to, and the grid has no verdict rather than a guess.
      const onSouthmoor = Array.from(document.querySelectorAll("[data-droppable]")).filter(
        (cell) => cell.getAttribute("data-cell")?.startsWith("0-"),
      );
      const onStandard = Array.from(document.querySelectorAll("[data-droppable]")).filter(
        (cell) => cell.getAttribute("data-cell")?.startsWith("1-"),
      );
      expect(onSouthmoor).toHaveLength(0);
      expect(onStandard).toHaveLength(1);
    });

    it("is withdrawn while a write is in flight", () => {
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), busy: true })}
        />,
      );
      expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);
    });
  });

  describe("the mouse", () => {
    it("picks a lesson up and drops it in one slot", async () => {
      const user = userEvent.setup();
      const onPickUp = vi.fn();
      const onDrop = vi.fn();
      const { rerender } = render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ onPickUp })}
        />,
      );
      await user.click(screen.getByRole("button", { name: "Pick up" }));
      expect(onPickUp).toHaveBeenCalledWith("a1");

      rerender(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), onDrop })}
        />,
      );
      cellAt("0-1-0").dispatchEvent(new MouseEvent("drop", { bubbles: true, cancelable: true }));
      expect(onDrop).toHaveBeenCalledWith("ts-legal");
    });

    it("cannot drop onto a cell the backend refused", () => {
      const onDrop = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), onDrop })}
        />,
      );
      cellAt("0-1-1").dispatchEvent(new MouseEvent("drop", { bubbles: true, cancelable: true }));
      expect(onDrop).not.toHaveBeenCalled();
    });

    it("offers the held lesson a put-down rather than a second pick-up", () => {
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
        />,
      );
      expect(
        screen.getByRole("button", { name: "Put down" }).getAttribute("aria-pressed"),
      ).toBe("true");
    });
  });

  describe("the keyboard reaches everything the mouse does", () => {
    it("picks up with Enter and drops with Enter", async () => {
      const user = userEvent.setup();
      const onPickUp = vi.fn();
      const onDrop = vi.fn();
      const { rerender } = render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ onPickUp })}
        />,
      );
      await user.tab();
      expect(document.activeElement!.getAttribute("data-cell")).toBe("0-0-0");
      await user.keyboard("{Enter}");
      expect(onPickUp).toHaveBeenCalledWith("a1");

      rerender(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), onDrop })}
        />,
      );
      (cellAt("0-0-0") as HTMLElement).focus();
      await user.keyboard("{ArrowRight}{Enter}");
      expect(onDrop).toHaveBeenCalledWith("ts-legal");
    });

    it("does not drop onto a refused cell", async () => {
      const user = userEvent.setup();
      const onDrop = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), onDrop })}
        />,
      );
      (cellAt("0-1-1") as HTMLElement).focus();
      await user.keyboard("{Enter}");
      expect(onDrop).not.toHaveBeenCalled();
    });

    it("puts down with Escape, changing nothing", async () => {
      const user = userEvent.setup();
      const onPickUp = vi.fn();
      const onDrop = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid(), onPickUp, onDrop })}
        />,
      );
      (cellAt("0-1-0") as HTMLElement).focus();
      await user.keyboard("{Escape}");
      expect(onPickUp).toHaveBeenCalledWith(null);
      expect(onDrop).not.toHaveBeenCalled();
    });

    it("unschedules with Delete", async () => {
      const user = userEvent.setup();
      const onUnschedule = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ onUnschedule })}
        />,
      );
      await user.tab();
      await user.keyboard("{Delete}");
      expect(onUnschedule).toHaveBeenCalledWith("a1");
    });

    it("locks and unlocks with l", async () => {
      const user = userEvent.setup();
      const onToggleLock = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[aLesson()]}
          editing={anEditing({ onToggleLock })}
        />,
      );
      await user.tab();
      await user.keyboard("l");
      expect(onToggleLock).toHaveBeenCalledWith("a1");
    });

    it("still moves the focus when the key is not an accelerator", async () => {
      const user = userEvent.setup();
      render(
        <TimetableGrid grid={withLesson()} lessons={[aLesson()]} editing={anEditing()} />,
      );
      await user.tab();
      await user.keyboard("{ArrowDown}");
      expect(document.activeElement!.getAttribute("data-cell")).toBe("0-0-1");
    });

    it("reaches the second lesson of a shared cell, which an accelerator alone strands", async () => {
      const user = userEvent.setup();
      const onPickUp = vi.fn();
      const grid = aGrid({
        panels: [
          aPanel({
            cells: cells(PERIODS, DAYS, {
              "1-1": { kind: "pinned", lessons: [0, 1], reason: "conflict" },
            }),
          }),
        ],
      });
      const second = aLesson({
        activityId: "a2",
        label: {
          sectionId: "s2",
          subject: "Science",
          course: "Science Year 9",
          section: "A",
          yearLevel: "9",
          campus: null,
          short: "9 Science A",
        },
      });
      render(
        <TimetableGrid
          grid={grid}
          lessons={[aLesson(), second]}
          editing={anEditing({ onPickUp })}
        />,
      );
      // Two of each, one per lesson. The `l`/Enter/Delete accelerators all act on the first, so
      // without these buttons the second lesson of a clash would be mouse-only.
      const pickUps = screen.getAllByRole("button", { name: "Pick up" });
      expect(pickUps).toHaveLength(2);
      await user.click(pickUps[1]!);
      expect(onPickUp).toHaveBeenCalledWith("a2");
    });
  });

  describe("a locked placement", () => {
    const locked = () => aLesson({ flags: { isLocked: true, isManual: false } });

    it("offers no drag and no pick-up", () => {
      render(
        <TimetableGrid grid={withLesson()} lessons={[locked()]} editing={anEditing()} />,
      );
      expect(document.querySelectorAll("[draggable='true']")).toHaveLength(0);
      expect(screen.getByRole("button", { name: "Pick up" })).toHaveProperty("disabled", true);
      expect(screen.getByRole("button", { name: "Unschedule" })).toHaveProperty(
        "disabled",
        true,
      );
    });

    it("offers Unlock, because the lock is the thing the user can change", async () => {
      const user = userEvent.setup();
      const onToggleLock = vi.fn();
      render(
        <TimetableGrid
          grid={withLesson()}
          lessons={[locked()]}
          editing={anEditing({ onToggleLock })}
        />,
      );
      const button = screen.getByRole("button", { name: "Unlock" });
      expect(button).toHaveProperty("disabled", false);
      await user.click(button);
      expect(onToggleLock).toHaveBeenCalledWith("a1");
    });

    it("is still said to be locked in words, not only by a missing handle", () => {
      render(
        <TimetableGrid grid={withLesson()} lessons={[locked()]} editing={anEditing()} />,
      );
      expect(screen.getByText("Locked")).toBeTruthy();
    });
  });

  it("marks what is in hand", () => {
    render(
      <TimetableGrid
        grid={withLesson()}
        lessons={[aLesson()]}
        editing={anEditing({ pickedUp: "a1", candidates: aCandidateGrid() })}
      />,
    );
    expect(document.querySelectorAll(".ttLesson--held")).toHaveLength(1);
  });

  it("stops offering writes while one is in flight", () => {
    render(
      <TimetableGrid
        grid={withLesson()}
        lessons={[aLesson()]}
        editing={anEditing({ busy: true })}
      />,
    );
    for (const name of ["Pick up", "Lock", "Unschedule"]) {
      expect(screen.getByRole("button", { name })).toHaveProperty("disabled", true);
    }
    expect(document.querySelectorAll("[draggable='true']")).toHaveLength(0);
  });
});
