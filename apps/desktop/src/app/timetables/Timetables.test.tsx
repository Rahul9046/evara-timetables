/**
 * Saved timetables: the list, and one of them opened.
 *
 * Every case here is a statement a record requires the interface to make, and each is one a
 * reasonable implementation gets wrong in the obvious direction:
 *
 * - a list row is a **version**, so there is no "current" badge and no version number;
 * - a delete says what it will remove before it removes it;
 * - a rename and a revalidation carry the revision the screen was holding, never one they
 *   read for themselves;
 * - a timetable whose grid cannot be rebuilt still opens, shows its findings, and says why
 *   there is no grid — and shows **no** grid rather than a partial one;
 * - a timetable that is readable but incomplete says so, because a short grid shown as whole
 *   is the one failure a school cannot detect for itself;
 * - this screen stays read-only and **hands over** to the editor: no drag affordance here, no
 *   move, lock or repair action, and editing reached only by opening a session of its own.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { TimetableSummary, TimetableView } from "@evara/domain";

const listTimetables = vi.fn<() => Promise<TimetableSummary[]>>();
const openTimetable = vi.fn<() => Promise<TimetableView>>();
const timetableScope = vi.fn();
const renameTimetable = vi.fn();
const deleteTimetable = vi.fn();
const revalidateTimetable = vi.fn();

vi.mock("../../ipc/timetables", () => ({
  listTimetables: () => listTimetables(),
  openTimetable: () => openTimetable(),
  timetableScope: (...args: unknown[]) => timetableScope(...args),
  renameTimetable: (...args: unknown[]) => renameTimetable(...args),
  deleteTimetable: (...args: unknown[]) => deleteTimetable(...args),
  revalidateTimetable: (...args: unknown[]) => revalidateTimetable(...args),
  describeTimetableError: () => "Something went wrong. Please try again.",
  isInputsChanged: (error: unknown) =>
    typeof error === "object" && error !== null && "kind" in error
      ? (error as { kind: string }).kind === "inputsChanged"
      : false,
}));

// Only the one command this screen's hand-over has to reach. The rest of `ipc/edit` stays real,
// `verdictAt` among them, because the grid calls it on every render.
const openForEditing = vi.fn();

vi.mock("../../ipc/edit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/edit")>()),
  openForEditing: (...args: unknown[]) => openForEditing(...args),
  closeEditing: vi.fn(async () => true),
}));

const { Timetables } = await import("./Timetables");

const NOTHING = {
  indeterminateStudentPairs: 0,
  uncheckedCapacities: 0,
  sharedEntities: 0,
  unenforcedRestrictions: 0,
};

function aSummary(over: Partial<TimetableSummary> = {}): TimetableSummary {
  return {
    id: "tt-1",
    cycleId: "cy-1",
    termId: null,
    name: "Draft 1",
    generatedAt: "2026-10-23T14:02:00.000Z",
    rev: 4,
    unverified: NOTHING,
    ...over,
  };
}

function aView(over: Partial<TimetableView> = {}): TimetableView {
  return {
    timetable: aSummary(),
    scorings: [
      {
        kind: "GENERATION",
        isGeneration: true,
        evaluatedAt: "2026-10-23T14:02:00.000Z",
        hard: 0,
        medium: 0,
        soft: 0,
        placed: 6,
        required: 6,
      },
    ],
    unscheduled: [],
    shared: [],
    projection: {
      schoolName: "Northgate",
      cycleName: "Standard week",
      termName: null,
      days: [],
      axes: [],
      lessons: [],
      scopes: [
        {
          kind: "master",
          id: null,
          name: "All lessons",
          secondary: null,
          lessonCount: 0,
          rosterDerived: false,
        },
      ],
      hasEnrolments: false,
    },
    unreadable: null,
    notInProblem: [],
    storedEntries: 6,
    findings: [],
    counts: { hard: 0, medium: 0, soft: 0, unknown: 0 },
    run: null,
    ...over,
  };
}

beforeEach(() => {
  for (const fn of [
    listTimetables,
    openTimetable,
    timetableScope,
    renameTimetable,
    deleteTimetable,
    revalidateTimetable,
  ]) {
    fn.mockReset();
  }
  listTimetables.mockResolvedValue([aSummary()]);
  openTimetable.mockResolvedValue(aView());
  timetableScope.mockResolvedValue({
    scope: {
      kind: "master",
      id: null,
      name: "All lessons",
      secondary: null,
      lessonCount: 0,
      rosterDerived: false,
    },
    panels: [],
    conflicts: [],
    defective: [],
  });
  renameTimetable.mockResolvedValue(aSummary({ name: "Renamed" }));
  deleteTimetable.mockResolvedValue({
    entries: 6,
    runs: 1,
    unscheduled: 0,
    evaluations: 1,
    violations: 0,
  });
  revalidateTimetable.mockResolvedValue({
    kind: "REVALIDATION",
    isGeneration: false,
    evaluatedAt: "2026-10-23T16:41:00.000Z",
    hard: 0,
    medium: 0,
    soft: 0,
    placed: 6,
    required: 6,
  });
});

function renderList(openId: string | null = null, onOpen = vi.fn()) {
  render(
    <Timetables
      schoolId="sc-1"
      schoolName="Northgate"
      openId={openId}
      onOpen={onOpen}
      onGenerate={vi.fn()}
    />,
  );
  return { onOpen };
}

// ---------------------------------------------------------------- the list

describe("the list", () => {
  it("shows each timetable as its own version, with no current badge", async () => {
    renderList();
    expect(await screen.findByRole("button", { name: "Draft 1" })).toBeTruthy();
    expect(screen.queryByText(/current/i)).toBeNull();
    expect(screen.queryByText(/version 1|v1/i)).toBeNull();
  });

  it("says a save adds rather than replaces, when there is nothing yet", async () => {
    listTimetables.mockResolvedValue([]);
    renderList();
    expect(
      await screen.findByText(/every timetable you save is kept alongside the others/),
    ).toBeTruthy();
  });

  it("names the section that fixes an absent school", async () => {
    const user = userEvent.setup();
    const onOpenSection = vi.fn();
    render(
      <Timetables
        schoolId={null}
        schoolName="Northgate"
        openId={null}
        onOpen={vi.fn()}
        onGenerate={vi.fn()}
        onOpenSection={onOpenSection}
      />,
    );
    await user.click(screen.getByRole("button", { name: /create one under Schools/ }));
    expect(onOpenSection).toHaveBeenCalledWith("schools");
  });
});

// ---------------------------------------------------------------- an open timetable

describe("an opened timetable", () => {
  it("shows both scorings and the stored disclosure", async () => {
    renderList("tt-1");
    expect(await screen.findByText("As generated")).toBeTruthy();
    expect(screen.getByText("What this timetable could not check")).toBeTruthy();
  });

  it("carries the revision it was holding into a rename", async () => {
    const user = userEvent.setup();
    renderList("tt-1");
    await user.click(await screen.findByRole("button", { name: "Rename" }));
    const input = screen.getByLabelText("Name");
    await user.clear(input);
    await user.type(input, "Renamed");
    await user.click(screen.getByRole("button", { name: "Save name" }));
    // Revision 4 is the one the view carried: read-compare-write is not available here.
    await waitFor(() => expect(renameTimetable).toHaveBeenCalledWith("tt-1", 4, "Renamed"));
  });

  it("carries the revision into a revalidation, which consumes it", async () => {
    const user = userEvent.setup();
    renderList("tt-1");
    await user.click(
      await screen.findByRole("button", { name: /Re-score against today's inputs/ }),
    );
    await waitFor(() => expect(revalidateTimetable).toHaveBeenCalledWith("tt-1", 4));
  });

  it("says what a delete will remove before removing it", async () => {
    const user = userEvent.setup();
    renderList("tt-1");
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    expect(screen.getByText(/remove 6 placed lessons/)).toBeTruthy();
    expect(screen.getByText(/cannot be undone/)).toBeTruthy();
    expect(deleteTimetable).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteTimetable).toHaveBeenCalledWith("tt-1", 4));
  });

  it("lets a confirmation be declined without deleting anything", async () => {
    const user = userEvent.setup();
    renderList("tt-1");
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Keep" }));
    expect(deleteTimetable).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- degraded reads

describe("a timetable whose grid cannot be rebuilt", () => {
  it("opens, says why there is no grid, and draws none", async () => {
    openTimetable.mockResolvedValue(
      aView({
        projection: null,
        unreadable: "the cycle this timetable was built over is no longer in the campus file",
        findings: [
          {
            typeKey: "teacher-clash",
            knownType: true,
            level: "HARD",
            penalty: -1000,
            message: null,
            detail: "two lessons at once",
            cycleDayId: null,
            startsAt: null,
            endsAt: null,
            activityIds: [],
            entities: [],
            source: "stored",
          },
        ],
        counts: { hard: 1, medium: 0, soft: 0, unknown: 0 },
      }),
    );
    renderList("tt-1");

    expect(await screen.findByText("No grid could be built")).toBeTruthy();
    expect(screen.getByText(/no longer in the campus file/)).toBeTruthy();
    // The stored findings are still there — that is what they are kept for.
    expect(screen.getByText("two lessons at once")).toBeTruthy();
    // And no table at all, rather than a partial one.
    expect(screen.queryByRole("table", { name: /Days across/ })).toBeNull();
    expect(timetableScope).not.toHaveBeenCalled();
  });

  it("says when a timetable is readable but incomplete", async () => {
    openTimetable.mockResolvedValue(aView({ notInProblem: ["a1", "a2"], storedEntries: 6 }));
    renderList("tt-1");
    expect(
      await screen.findByText("This timetable is readable but incomplete"),
    ).toBeTruthy();
    expect(
      screen.getByText(/What you are looking at is less than what was saved/),
    ).toBeTruthy();
  });

  it("admits it cannot name a shared entity without a compiled problem", async () => {
    openTimetable.mockResolvedValue(
      aView({
        projection: null,
        unreadable: "the cycle is gone",
        timetable: aSummary({ unverified: { ...NOTHING, sharedEntities: 2 } }),
      }),
    );
    renderList("tt-1");
    expect(await screen.findByText(/open this timetable to see which/)).toBeTruthy();
  });
});

// ---------------------------------------------------------------- read-only, and the hand-over

describe("this screen stays read-only", () => {
  it("offers no drag affordance anywhere", async () => {
    renderList("tt-1");
    await screen.findByText("As generated");
    expect(document.querySelectorAll("[draggable='true']")).toHaveLength(0);
    expect(document.querySelectorAll("[aria-dropeffect]")).toHaveLength(0);
  });

  it("performs no scheduling action of its own", async () => {
    renderList("tt-1");
    await screen.findByText("As generated");
    // Still read-only: nothing here moves a lesson, locks one or drags one. Since M9B it
    // hands over to a *repair session* as it already hands over to the editor -- which is
    // the opposite of becoming writable, because the session compiles its own problem and
    // takes its own baseline snapshot.
    for (const forbidden of [/^move/i, /lock/i, /drag/i]) {
      expect(screen.queryByRole("button", { name: forbidden })).toBeNull();
    }
    expect(
      screen.getByRole("button", { name: "Repair this timetable" }),
    ).toBeTruthy();
  });
});

describe("the hand-over to the editor", () => {
  it("is offered on an opened timetable", async () => {
    renderList("tt-1");
    expect(
      await screen.findByRole("button", { name: "Edit this timetable" }),
    ).toHaveProperty("disabled", false);
  });

  it("opens a session of its own rather than making this screen writable", async () => {
    const user = userEvent.setup();
    openForEditing.mockRejectedValue({ kind: "invalid", message: "Not now." });
    renderList("tt-1");
    await user.click(await screen.findByRole("button", { name: "Edit this timetable" }));
    // The identifier, not a view this screen already loaded: the session compiles its own
    // problem and takes its own cross-session stamp, from one snapshot.
    await waitFor(() => expect(openForEditing).toHaveBeenCalledWith("tt-1"));
  });

  it("comes back to the read-only view, which re-reads the timetable", async () => {
    const user = userEvent.setup();
    openForEditing.mockRejectedValue({ kind: "invalid", message: "Not now." });
    renderList("tt-1");
    await user.click(await screen.findByRole("button", { name: "Edit this timetable" }));
    await user.click(await screen.findByRole("button", { name: /Back to this timetable/ }));
    expect(await screen.findByText("As generated")).toBeTruthy();
  });

  it("is refused, with the reason, when no grid could be built", async () => {
    openTimetable.mockResolvedValue(
      aView({ projection: null, unreadable: "This timetable's cycle no longer exists" }),
    );
    renderList("tt-1");
    expect(await screen.findByRole("button", { name: "Edit this timetable" })).toHaveProperty(
      "disabled",
      true,
    );
    // Repair needs the same compiled problem, and is refused for the same reason.
    expect(screen.getByRole("button", { name: "Repair this timetable" })).toHaveProperty(
      "disabled",
      true,
    );
    expect(
      screen.getByText(/there is nothing to check an edit or a\s+repair against/),
    ).toBeTruthy();
  });
});
