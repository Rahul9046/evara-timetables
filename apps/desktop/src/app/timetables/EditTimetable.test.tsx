/**
 * The editor screen.
 *
 * The Rust tests own what an edit *is*: whether a move is legal, what undoing one means, which
 * revision a write carries, what a lock does to the constraint engine. None of that is asserted
 * here and none of it could be — this file owns what only the interface can get wrong, and every
 * case is one a reasonable implementation gets wrong in the obvious direction:
 *
 * - the session is opened once on mount and **closed on unmount**, so leaving the screen does not
 *   leak a compiled problem and an undo stack per visit;
 * - the room is sent with the proposal and a different room **re-asks** the backend, because a
 *   drop's legality depends on it;
 * - the four refusals get four different remedies, and the two stale ones stop offering writes
 *   rather than inviting a retry that would answer with the old snapshot;
 * - a refused undo **re-reads** the state, because the session cleared both stacks;
 * - the unplaced tray is a pick-up source, since an unplaced occurrence is in no cell;
 * - **Duplicate** is offered before the first edit, with the sentence that says why.
 *
 * Every assertion about *which* cells are legal is in `TimetableGrid.test.tsx`; this file hands
 * the grid a `CandidateGrid` and checks what was asked for, never what was decided.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  CandidateGrid,
  EditStateView,
  ProjectedDay,
  ProjectedLesson,
  ProjectedPeriod,
  ScopeGrid,
  TimetableProjection,
  TimetableSummary,
} from "@evara/domain";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

// The commands. The predicates and `verdictAt` stay real: they are the contract this screen is
// written against, and mocking them would test the mock.
const openForEditing = vi.fn();
const closeEditing = vi.fn();
const editState = vi.fn();
const editScope = vi.fn();
const candidatesFor = vi.fn();
const applyMove = vi.fn();
const unscheduleLesson = vi.fn();
const setLessonLock = vi.fn();
const undoEdit = vi.fn();
const redoEdit = vi.fn();
const duplicateTimetable = vi.fn();

vi.mock("../../ipc/edit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/edit")>()),
  openForEditing: (...a: unknown[]) => openForEditing(...a),
  closeEditing: (...a: unknown[]) => closeEditing(...a),
  editState: (...a: unknown[]) => editState(...a),
  editScope: (...a: unknown[]) => editScope(...a),
  candidatesFor: (...a: unknown[]) => candidatesFor(...a),
  applyMove: (...a: unknown[]) => applyMove(...a),
  unscheduleLesson: (...a: unknown[]) => unscheduleLesson(...a),
  setLessonLock: (...a: unknown[]) => setLessonLock(...a),
  undoEdit: (...a: unknown[]) => undoEdit(...a),
  redoEdit: (...a: unknown[]) => redoEdit(...a),
  duplicateTimetable: (...a: unknown[]) => duplicateTimetable(...a),
}));

const { EditTimetable } = await import("./EditTimetable");

// ---------------------------------------------------------------- fixtures

const DAYS: ProjectedDay[] = [
  { cycleDayId: "d1", ordinal: 1, label: "Day A" },
  { cycleDayId: "d2", ordinal: 2, label: "Day B" },
];

const PERIODS: ProjectedPeriod[] = [
  { periodId: "p1", ordinal: 1, label: "1", startsAt: "09:00", endsAt: "10:00", isTeaching: true },
  { periodId: "p2", ordinal: 2, label: "2", startsAt: "10:00", endsAt: "11:00", isTeaching: true },
];

const LESSON: ProjectedLesson = {
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
};

const SUMMARY: TimetableSummary = {
  id: "tt-1",
  cycleId: "cy-1",
  termId: null,
  name: "Draft A",
  generatedAt: "2026-10-24T09:00:00.000Z",
  rev: 2,
  unverified: {
    indeterminateStudentPairs: 0,
    uncheckedCapacities: 0,
    sharedEntities: 0,
    unenforcedRestrictions: 0,
  },
};

const PROJECTION: TimetableProjection = {
  schoolName: "Northgate",
  cycleName: "Two-week rotation",
  termName: null,
  days: DAYS,
  axes: [
    {
      scheduleId: "bs1",
      scheduleName: "Standard Day",
      campusId: null,
      campusName: "Main",
      periods: PERIODS,
    },
  ],
  lessons: [LESSON],
  scopes: [{ kind: "master", id: null, name: "All lessons", secondary: null, lessonCount: 1, rosterDerived: false }],
  hasEnrolments: false,
};

function aState(over: Partial<EditStateView> = {}): EditStateView {
  return {
    sessionId: "se-1",
    timetable: SUMMARY,
    version: 0,
    projection: PROJECTION,
    lessons: [{ activityId: "a1", entryId: "e1", rev: 5, isLocked: false, isManual: false }],
    unplaced: [],
    findings: [],
    counts: { hard: 0, medium: 0, soft: 0, unknown: 0 },
    score: { hard: 0, medium: 0, soft: 0 },
    isUsable: true,
    locked: 0,
    history: { undo: null, redo: null, depth: 0 },
    notInProblem: [],
    pinsNotInProblem: [],
    ...over,
  };
}

const GRID: ScopeGrid = {
  scope: {
    kind: "master",
    id: null,
    name: "All lessons",
    secondary: null,
    lessonCount: 1,
    rosterDerived: false,
  },
  panels: [
    {
      scheduleId: "bs1",
      scheduleName: "Standard Day",
      campusName: "Main",
      days: DAYS,
      periods: PERIODS,
      cells: [
        { dayOrdinal: 1, periodOrdinal: 1, content: { kind: "lesson", lesson: 0, rows: 1 } },
        { dayOrdinal: 2, periodOrdinal: 1, content: { kind: "empty" } },
        { dayOrdinal: 1, periodOrdinal: 2, content: { kind: "empty" } },
        { dayOrdinal: 2, periodOrdinal: 2, content: { kind: "empty" } },
      ],
    },
  ],
  conflicts: [],
  defective: [],
};

function aCandidateGrid(over: Partial<CandidateGrid> = {}): CandidateGrid {
  return {
    activityId: "a1",
    scheduleId: "bs1",
    roomId: "r1",
    rooms: [
      { roomId: "r1", name: "Room B12" },
      { roomId: "r2", name: "Room C04" },
    ],
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
        blocking: ["Room B12 is already booked."],
        mediumDelta: 0,
      },
    ],
    unavailable: null,
    ...over,
  };
}

/**
 * Renders the editor and waits for its session **and its grid**.
 *
 * Both, because the per-lesson Pick up / Lock / Unschedule buttons live in the grid's cells —
 * which is the point of them — so a test that acted on one before `edit_scope` resolved would be
 * asserting against a screen that is still loading.
 */
async function open(state: EditStateView = aState()) {
  openForEditing.mockResolvedValue(state);
  const onDone = vi.fn();
  const onChanged = vi.fn();
  const rendered = render(
    <EditTimetable id="tt-1" schoolName="Northgate" onDone={onDone} onChanged={onChanged} />,
  );
  await screen.findByRole("heading", { name: /Editing Draft A/ });
  await screen.findByRole("table");
  return { ...rendered, onDone, onChanged };
}

/** Picks the one placed lesson up, through the grid's own button. */
async function pickUp(user: ReturnType<typeof userEvent.setup>) {
  candidatesFor.mockResolvedValue(aCandidateGrid());
  await user.click(screen.getByRole("button", { name: "Pick up" }));
  await screen.findByText(/will take it/);
}

beforeEach(() => {
  for (const mock of [
    openForEditing,
    closeEditing,
    editState,
    editScope,
    candidatesFor,
    applyMove,
    unscheduleLesson,
    setLessonLock,
    undoEdit,
    redoEdit,
    duplicateTimetable,
  ]) {
    mock.mockReset();
  }
  closeEditing.mockResolvedValue(true);
  editScope.mockResolvedValue(GRID);
});

// ---------------------------------------------------------------- the session

describe("the session is this screen's lifetime", () => {
  it("opens one on mount, by timetable", async () => {
    await open();
    expect(openForEditing).toHaveBeenCalledWith("tt-1");
    expect(openForEditing).toHaveBeenCalledTimes(1);
  });

  it("closes it on unmount, so a visit does not leak a compiled problem", async () => {
    const { unmount } = await open();
    unmount();
    await waitFor(() => expect(closeEditing).toHaveBeenCalledWith("se-1"));
  });

  it("says there is no partial editing mode when the problem will not compile", async () => {
    openForEditing.mockRejectedValue({
      kind: "invalid",
      message: "This timetable's cycle has changed.",
    });
    render(
      <EditTimetable id="tt-1" schoolName="Northgate" onDone={vi.fn()} onChanged={vi.fn()} />,
    );
    const banner = await screen.findByRole("alert");
    expect(banner.textContent).toContain("This timetable's cycle has changed.");
    expect(banner.textContent).toContain("could check nothing");
  });

  it("asks for the grid through the session, not the timetable", async () => {
    await open();
    await waitFor(() => expect(editScope).toHaveBeenCalledWith("se-1", "master", null));
  });
});

// ---------------------------------------------------------------- picking up

describe("picking a lesson up", () => {
  it("asks the backend for the whole panel, with the room the lesson already has", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    expect(candidatesFor).toHaveBeenCalledWith("se-1", "a1", "r1");
  });

  it("reports how many cells will take it, counting the backend's verdicts", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    expect(screen.getByText(/1 of 2 cells on its bell schedule will take it/)).toBeTruthy();
  });

  it("says what the backend said when there are no candidates at all", async () => {
    const user = userEvent.setup();
    await open();
    candidatesFor.mockResolvedValue(
      aCandidateGrid({ cells: [], unavailable: "No bell schedule resolves for 9B." }),
    );
    await user.click(screen.getByRole("button", { name: "Pick up" }));
    expect(await screen.findByText("No bell schedule resolves for 9B.")).toBeTruthy();
  });

  it("re-asks the backend when the room changes, because a room is part of the proposal", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    candidatesFor.mockClear();
    candidatesFor.mockResolvedValue(aCandidateGrid({ roomId: "r2" }));
    await user.selectOptions(screen.getByLabelText("Room"), "r2");
    expect(candidatesFor).toHaveBeenCalledWith("se-1", "a1", "r2");
  });

  it("offers no room at all only when the backend allows one", async () => {
    const user = userEvent.setup();
    await open();
    candidatesFor.mockResolvedValue(aCandidateGrid({ roomlessAllowed: true }));
    await user.click(screen.getByRole("button", { name: "Pick up" }));
    const select = await screen.findByLabelText("Room");
    expect(within(select).getByRole("option", { name: "No room" })).toBeTruthy();
  });

  it("offers only the rooms the loader resolved for the class", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    const options = within(screen.getByLabelText("Room")).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual(["Room B12", "Room C04"]);
  });

  it("puts down without writing anything", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    // The panel's button, named for the lesson. The cell the lesson came from carries its own.
    await user.click(screen.getByRole("button", { name: "Put down 9 Mathematics B" }));
    expect(screen.getByText(/Pick a lesson up to see where it can go/)).toBeTruthy();
    expect(applyMove).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- writing

describe("applying one edit", () => {
  it("sends the proposal with the chosen room and the revision it was holding", async () => {
    const user = userEvent.setup();
    await open();
    await pickUp(user);
    applyMove.mockResolvedValue(aState({ version: 1 }));

    document
      .querySelector('[data-cell="0-1-0"]')!
      .dispatchEvent(new MouseEvent("drop", { bubbles: true, cancelable: true }));

    await waitFor(() =>
      expect(applyMove).toHaveBeenCalledWith(
        "se-1",
        { activityId: "a1", timeslotId: "ts-legal", roomId: "r1" },
        5,
      ),
    );
  });

  it("sends no revision for an occurrence that is not yet placed", async () => {
    const user = userEvent.setup();
    const state = aState({
      lessons: [],
      unplaced: [{ activityId: "a1", label: "9 Mathematics B", durationPeriods: 1, splitIndex: 0 }],
      projection: { ...PROJECTION, lessons: [] },
    });
    await open(state);
    candidatesFor.mockResolvedValue(aCandidateGrid());
    applyMove.mockResolvedValue(aState({ version: 1 }));
    await user.click(screen.getByRole("button", { name: "Pick up" }));
    await screen.findByText(/will take it/);

    document
      .querySelector('[data-cell="0-1-0"]')!
      .dispatchEvent(new MouseEvent("drop", { bubbles: true, cancelable: true }));

    await waitFor(() =>
      expect(applyMove).toHaveBeenCalledWith(
        "se-1",
        { activityId: "a1", timeslotId: "ts-legal", roomId: null },
        null,
      ),
    );
  });

  it("unschedules with the entry's revision", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockResolvedValue(aState({ version: 1, lessons: [] }));
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    expect(unscheduleLesson).toHaveBeenCalledWith("se-1", "a1", 5);
  });

  it("locks with the entry's revision, and asks for the state it is not in", async () => {
    const user = userEvent.setup();
    await open();
    setLessonLock.mockResolvedValue(aState({ version: 1, locked: 1 }));
    await user.click(screen.getByRole("button", { name: "Lock" }));
    expect(setLessonLock).toHaveBeenCalledWith("se-1", "a1", true, 5);
  });

  it("re-asks for the grid after an edit, so nothing stale is left on screen", async () => {
    const user = userEvent.setup();
    await open();
    editScope.mockClear();
    setLessonLock.mockResolvedValue(aState({ version: 1, locked: 1 }));
    await user.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(editScope).toHaveBeenCalled());
  });

  it("tells the list behind it that something changed", async () => {
    const user = userEvent.setup();
    const { onChanged } = await open();
    setLessonLock.mockResolvedValue(aState({ version: 1, locked: 1 }));
    await user.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });
});

// ---------------------------------------------------------------- the four refusals

describe("a refusal", () => {
  it("offers Unlock when the placement is locked, rather than describing the problem", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockRejectedValue({
      kind: "locked",
      message: "9 Mathematics B is locked in this timetable.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));

    const banner = await screen.findByText(/9 Mathematics B is locked in this timetable\./);
    expect(banner.textContent).toContain("not overriding a clash");
    setLessonLock.mockResolvedValue(aState({ version: 1 }));
    await user.click(within(banner).getByRole("button", { name: "Unlock it" }));
    expect(setLessonLock).toHaveBeenCalledWith("se-1", "a1", true, 5);
  });

  it("ends the session when another window changed a different placement", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockRejectedValue({
      kind: "arrangementChanged",
      entriesChanged: 1,
      message: "Another window changed this timetable.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));

    const banner = await screen.findByRole("alert");
    expect(banner.textContent).toContain("Another window changed this timetable.");
    expect(banner.textContent).toContain("Nothing was written.");
    expect(within(banner).getByRole("button", { name: "Reopen it" })).toBeTruthy();
  });

  it("ends the session when a scheduling input moved", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockRejectedValue({
      kind: "inputsChanged",
      message: "An availability rule changed since this timetable was opened.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByRole("button", { name: "Reopen it" })).toBeTruthy();
  });

  it("stops offering writes once the snapshot is stale", async () => {
    const user = userEvent.setup();
    await open(aState({ history: { undo: "move 9 Mathematics B", redo: null, depth: 1 } }));
    unscheduleLesson.mockRejectedValue({
      kind: "arrangementChanged",
      entriesChanged: 1,
      message: "Another window changed this timetable.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    await screen.findByRole("alert");
    // Undo was available a moment ago. With a stale snapshot it is not: the inverse command's
    // precondition was computed from a state that no longer holds.
    expect(screen.getByRole("button", { name: "Undo" })).toHaveProperty("disabled", true);
  });

  it("opens a new session when the user reopens, and closes the old one", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockRejectedValue({
      kind: "arrangementChanged",
      entriesChanged: 1,
      message: "Another window changed this timetable.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    const banner = await screen.findByRole("alert");

    openForEditing.mockResolvedValue(aState({ sessionId: "se-2" }));
    await user.click(within(banner).getByRole("button", { name: "Reopen it" }));
    await waitFor(() => expect(closeEditing).toHaveBeenCalledWith("se-1"));
    expect(openForEditing).toHaveBeenCalledTimes(2);
  });

  it("shows an ordinary refusal as an error and keeps the session", async () => {
    const user = userEvent.setup();
    await open();
    unscheduleLesson.mockRejectedValue({
      kind: "conflict",
      entity: "timetable entry",
      expectedRev: 5,
      actualRev: 6,
      message: "This placement moved while you were editing.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    expect(
      await screen.findByText("This placement moved while you were editing."),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Unschedule" })).toHaveProperty(
      "disabled",
      false,
    );
  });
});

// ---------------------------------------------------------------- undo and redo

describe("undo and redo", () => {
  it("name what they would do, in the session's own words", async () => {
    await open(
      aState({ history: { undo: "move 9 Mathematics B", redo: "lock 9 Science A", depth: 1 } }),
    );
    expect(screen.getByText(/Undo: move 9 Mathematics B\./)).toBeTruthy();
    expect(screen.getByText(/Redo: lock 9 Science A\./)).toBeTruthy();
  });

  it("are offered only when the session says there is something to do", async () => {
    await open();
    expect(screen.getByRole("button", { name: "Undo" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Redo" })).toHaveProperty("disabled", true);
  });

  it("call the session, which owns the inverse command", async () => {
    const user = userEvent.setup();
    await open(aState({ history: { undo: "move 9 Mathematics B", redo: null, depth: 1 } }));
    undoEdit.mockResolvedValue(aState({ version: 2 }));
    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(undoEdit).toHaveBeenCalledWith("se-1");
  });

  it("re-read the state when a step is refused, because both stacks were cleared", async () => {
    const user = userEvent.setup();
    await open(aState({ history: { undo: "move 9 Mathematics B", redo: null, depth: 1 } }));
    undoEdit.mockRejectedValue({
      kind: "conflict",
      entity: "timetable entry",
      expectedRev: 5,
      actualRev: 6,
      message: "That placement moved.",
    });
    editState.mockResolvedValue(aState({ history: { undo: null, redo: null, depth: 0 } }));

    await user.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(editState).toHaveBeenCalledWith("se-1"));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Undo" })).toHaveProperty("disabled", true),
    );
  });
});

// ---------------------------------------------------------------- the tray

describe("the unplaced tray", () => {
  const unplaced = () =>
    aState({
      unplaced: [
        { activityId: "a9", label: "9 Science A", durationPeriods: 2, splitIndex: 0 },
      ],
    });

  it("is where an unplaced occurrence is picked up, since it is in no cell", async () => {
    const user = userEvent.setup();
    await open(unplaced());
    candidatesFor.mockResolvedValue(aCandidateGrid({ activityId: "a9" }));
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    await user.click(within(tray).getByRole("button", { name: "Pick up" }));
    expect(candidatesFor).toHaveBeenCalledWith("se-1", "a9", null);
  });

  it("says how long it is and which occurrence of its class", async () => {
    await open(unplaced());
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    expect(within(tray).getByText(/2 periods · occurrence 1/)).toBeTruthy();
  });

  it("does not claim an unplaced lesson cannot be placed", async () => {
    await open(unplaced());
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    expect(tray.textContent).toContain("is not a statement that these cannot be placed");
  });

  it("says plainly when there is nothing in it", async () => {
    await open();
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    expect(within(tray).getByText(/Every occurrence this timetable covers is on the grid/))
      .toBeTruthy();
  });
});

// ---------------------------------------------------------------- duplicating

describe("keeping the generated arrangement", () => {
  it("offers a copy before the first edit, with the reason", async () => {
    await open();
    const panel = screen.getByRole("region", { name: "Keeping the generated arrangement" });
    expect(panel.textContent).toContain("Editing changes");
    expect(within(panel).getByRole("button", { name: "Copy this timetable first" })).toBeTruthy();
  });

  it("copies with the timetable's revision and the name the user gave", async () => {
    const user = userEvent.setup();
    await open();
    duplicateTimetable.mockResolvedValue({ ...SUMMARY, id: "tt-2", name: "Draft A (copy)" });
    await user.click(screen.getByRole("button", { name: "Copy this timetable first" }));
    await user.click(screen.getByRole("button", { name: "Make the copy" }));
    expect(duplicateTimetable).toHaveBeenCalledWith("tt-1", 2, "Draft A (copy)");
  });

  it("says the copy claims no generation record", async () => {
    const user = userEvent.setup();
    await open();
    duplicateTimetable.mockResolvedValue({ ...SUMMARY, id: "tt-2", name: "Draft A (copy)" });
    await user.click(screen.getByRole("button", { name: "Copy this timetable first" }));
    await user.click(screen.getByRole("button", { name: "Make the copy" }));
    expect(
      await screen.findByText(/claims no generation run and no generation score/),
    ).toBeTruthy();
  });
});

// ---------------------------------------------------------------- what it discloses

describe("what the screen says about itself", () => {
  it("says edits are written as they are made and the history is not kept", async () => {
    await open();
    expect(screen.getByText(/Leaving this screen discards the undo history/)).toBeTruthy();
  });

  it("counts locked, unplaced and hard findings", async () => {
    await open(
      aState({
        locked: 3,
        counts: { hard: 1, medium: 2, soft: 0, unknown: 0 },
        unplaced: [
          { activityId: "a9", label: "9 Science A", durationPeriods: 1, splitIndex: 0 },
        ],
      }),
    );
    expect(screen.getByText("3 locked · 1 unplaced · 1 hard")).toBeTruthy();
  });

  it("says a timetable with stored rows outside the problem is less than what was saved", async () => {
    await open(aState({ notInProblem: ["e9"], pinsNotInProblem: ["e9"] }));
    const panel = screen.getByRole("region", {
      name: "This timetable is readable but incomplete",
    });
    expect(panel.textContent).toContain("less than what was saved");
    expect(panel.textContent).toContain("1 of them is locked");
  });

  it("goes back without writing anything", async () => {
    const user = userEvent.setup();
    const { onDone } = await open();
    await user.click(screen.getByRole("button", { name: /Back to this timetable/ }));
    expect(onDone).toHaveBeenCalled();
    expect(applyMove).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- asynchronous correctness

/**
 * Old answers landing on new questions.
 *
 * An editor is a screen where the user changes their mind faster than a round trip completes, so
 * each sequence below is a *likely* one rather than a contrived one — and the failure mode is
 * always the same and is the worst kind: the grid renders a verdict Rust gave truthfully, about a
 * question nobody is asking any more.
 *
 * The fixtures use deferred promises rather than timers, so the arrival order is **stated** by
 * the test instead of raced for, and a settle is wrapped in `act` so React has actually
 * re-rendered before anything is asserted. Both matter: an assertion made one microtask after a
 * settle passes whatever the component does, which is how a worthless regression test looks.
 *
 * **What each case is worth, measured rather than assumed.** Each guard was removed and the suite
 * re-run, which is the only way to know. Four cases fail without their guard and are regression
 * tests in the real sense: the two candidate-swap cases, the put-down case, and the
 * unmount-while-writing case. The rest assert a *user-visible* guarantee whose underlying guard
 * is belt-and-braces — the panel is unmounted by the same action that clears `pickedUp`, so no
 * second symptom is left to observe — and each one says so in its own comment. They are kept
 * because the visible guarantee is what matters to a user; they are labelled because a reader
 * deserves to know which assertions have teeth.
 */
describe("a stale answer never lands", () => {
  /** A promise whose resolution the test controls. */
  function deferred<T>() {
    let settle: (value: T) => void = () => undefined;
    let fail: (reason: unknown) => void = () => undefined;
    const promise = new Promise<T>((resolve, reject) => {
      settle = resolve;
      fail = reject;
    });
    return { promise, settle, fail };
  }

  it("ignores lesson A's candidate panel when lesson B is picked up first", async () => {
    const user = userEvent.setup();
    const state = aState({
      lessons: [
        { activityId: "a1", entryId: "e1", rev: 5, isLocked: false, isManual: false },
      ],
      unplaced: [
        { activityId: "a9", label: "9 Science A", durationPeriods: 1, splitIndex: 0 },
      ],
    });
    await open(state);

    const first = deferred<CandidateGrid>();
    const second = deferred<CandidateGrid>();
    candidatesFor.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    // A from its cell, then B from the tray, before either answer arrives. Both are scoped,
    // because with an unplaced lesson on screen there are two buttons called "Pick up" — which
    // is the point of the tray.
    const grid = screen.getByRole("table");
    await user.click(within(grid).getByRole("button", { name: "Pick up" }));
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    await user.click(within(tray).getByRole("button", { name: "Pick up" }));

    // B's answer, then A's — the order the network chose, not the order they were asked in.
    await act(async () => {
      second.settle(aCandidateGrid({ activityId: "a9", cells: [] }));
    });
    expect(screen.getByText(/0 of 0 cells/)).toBeTruthy();
    expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);

    await act(async () => {
      first.settle(aCandidateGrid({ activityId: "a1" }));
    });

    // **The assertion with teeth.** A's grid has one allowed cell, so without the ticket guard
    // that cell is now a drop target — tinted, droppable and announced as legal — while the
    // lesson in hand is B. The user would drop B into a cell Rust approved for A.
    expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);
    expect(screen.getByText(/0 of 0 cells/)).toBeTruthy();
    expect(screen.queryByText(/1 of 2 cells/)).toBeNull();
  });

  it("ignores the previous room's panel when the room has already changed", async () => {
    const user = userEvent.setup();
    await open();

    // Both requests are in flight before either answers, which is the whole point: the pick-up
    // asks about Room B12 and the room change asks about Room C04, and B12's answer arrives
    // last. The room select is only on screen once a panel exists, so B12's answer has to land
    // first to open it — and then be superseded while a *second* B12-shaped answer is pending.
    const firstB12 = deferred<CandidateGrid>();
    const forC04 = deferred<CandidateGrid>();
    const lateB12 = deferred<CandidateGrid>();
    candidatesFor
      .mockReturnValueOnce(firstB12.promise)
      .mockReturnValueOnce(forC04.promise)
      .mockReturnValueOnce(lateB12.promise);

    await user.click(screen.getByRole("button", { name: "Pick up" }));
    await act(async () => {
      firstB12.settle(aCandidateGrid({ roomId: "r1" }));
    });
    expect(screen.getByText(/1 of 2 cells/)).toBeTruthy();

    // Change to C04, then back to B12, then let C04's answer land last. Three questions about
    // the same lesson, so the guard cannot be the lesson's identity — which is why it is the
    // request's.
    await user.selectOptions(screen.getByLabelText("Room"), "r2");
    await user.selectOptions(screen.getByLabelText("Room"), "r1");
    await act(async () => {
      lateB12.settle(aCandidateGrid({ roomId: "r1", cells: [] }));
    });
    expect(screen.getByText(/0 of 0 cells/)).toBeTruthy();

    await act(async () => {
      forC04.settle(aCandidateGrid({ roomId: "r2" }));
    });
    // Teeth: without the ticket, C04's two cells come back — one of them droppable — under a
    // room select reading "Room B12". Dropping there would book the wrong room on a verdict
    // Rust gave about a different one.
    expect(screen.getByText(/0 of 0 cells/)).toBeTruthy();
    expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);
    expect((screen.getByLabelText("Room") as HTMLSelectElement).value).toBe("r1");
  });

  it("discards a panel asked for before a put-down, even once something else is in hand", async () => {
    const user = userEvent.setup();
    const state = aState({
      unplaced: [{ activityId: "a9", label: "9 Science A", durationPeriods: 1, splitIndex: 0 }],
    });
    await open(state);

    const stale = deferred<CandidateGrid>();
    const fresh = deferred<CandidateGrid>();
    candidatesFor.mockReturnValueOnce(stale.promise).mockReturnValueOnce(fresh.promise);

    // Pick A up, put it down, pick B up. Three questions, and only the third has an answer the
    // screen is entitled to render.
    const grid = screen.getByRole("table");
    await user.click(within(grid).getByRole("button", { name: "Pick up" }));
    await user.click(screen.getByRole("button", { name: "Put down 9 Mathematics B" }));
    const tray = screen.getByRole("region", { name: "Unplaced lessons" });
    await user.click(within(tray).getByRole("button", { name: "Pick up" }));

    await act(async () => {
      fresh.settle(aCandidateGrid({ activityId: "a9", cells: [] }));
    });
    await act(async () => {
      stale.settle(aCandidateGrid({ activityId: "a1" }));
    });

    // A put-down bumps the ticket as well, so the first answer is dead twice over.
    expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);
    expect(screen.getByText(/0 of 0 cells/)).toBeTruthy();
  });

  it("survives a candidate panel arriving after the editor closed", async () => {
    // **A smoke test, not a regression test**, and said so rather than dressed up: after unmount
    // the tree is gone, so every query is null whatever the component did. What it proves is that
    // the late settle does not throw — which is all there is to prove, because React 19 discards
    // a state write to an unmounted tree silently. The `alive` guard it exercises earns its place
    // on the *write* path, where the next test has teeth.
    const user = userEvent.setup();
    const { unmount } = await open();
    const pending = deferred<CandidateGrid>();
    candidatesFor.mockReturnValueOnce(pending.promise);

    await user.click(screen.getByRole("button", { name: "Pick up" }));
    unmount();
    await act(async () => {
      pending.settle(aCandidateGrid());
    });
    expect(screen.queryByText(/will take it/)).toBeNull();
  });

  it("closes the session when the editor unmounts before it finished opening", async () => {
    const pending = deferred<EditStateView>();
    openForEditing.mockReturnValueOnce(pending.promise);
    const { unmount } = render(
      <EditTimetable id="tt-1" schoolName="Northgate" onDone={vi.fn()} onChanged={vi.fn()} />,
    );
    unmount();
    pending.settle(aState({ sessionId: "se-late" }));
    // The session opened after the cleanup ran, so the cleanup had nothing to close. It must be
    // closed here instead, or it leaks a compiled problem for the life of the process.
    await waitFor(() => expect(closeEditing).toHaveBeenCalledWith("se-late"));
  });

  it("writes no state when the editor closes while an edit is in flight", async () => {
    const user = userEvent.setup();
    const { unmount, onChanged } = await open();
    const pending = deferred<EditStateView>();
    setLessonLock.mockReturnValueOnce(pending.promise);

    await user.click(screen.getByRole("button", { name: "Lock" }));
    onChanged.mockClear();
    unmount();
    pending.settle(aState({ version: 1, locked: 1 }));
    await Promise.resolve();
    // `accept` would have called this. The write itself completed in Rust, which is correct —
    // what must not happen is this screen acting on it.
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("starts one write for a double-click", async () => {
    // **Covered by `busy`, not by the ref, and this test cannot tell them apart.** Testing
    // Library wraps every dispatch in `act`, so React has re-rendered and disabled the control
    // before the second click is delivered — with `fireEvent` as much as with `userEvent`. There
    // is therefore no DOM-level sequence that reaches the handler twice, and no honest way to
    // write a failing test for the `writing` ref from here.
    //
    // The ref is kept anyway: it costs one comparison, it covers the keyboard path and any
    // future caller that is not a click, and the alternative is relying on a render having
    // happened. What this test pins is the guarantee a user has — two clicks, one edit.
    await open();
    const pending = deferred<EditStateView>();
    setLessonLock.mockReturnValue(pending.promise);

    const button = screen.getByRole("button", { name: "Lock" });
    fireEvent.click(button);
    fireEvent.click(button);

    await act(async () => {
      pending.settle(aState({ version: 1, locked: 1 }));
    });
    // Two writes would both have read rev 5, and the second would come back a conflict — the
    // database protecting itself, reported to the user as their own mistake.
    expect(setLessonLock).toHaveBeenCalledTimes(1);
  });

  it("does not re-tint the grid after the session has been declared stale", async () => {
    const user = userEvent.setup();
    await open();
    const pending = deferred<CandidateGrid>();
    candidatesFor.mockReturnValueOnce(pending.promise);

    await user.click(screen.getByRole("button", { name: "Pick up" }));
    // A write refused while that panel is still in flight.
    unscheduleLesson.mockRejectedValue({
      kind: "arrangementChanged",
      entriesChanged: 1,
      message: "Another window changed this timetable.",
    });
    await user.click(screen.getByRole("button", { name: "Unschedule" }));
    await screen.findByRole("alert");

    await act(async () => {
      pending.settle(aCandidateGrid());
    });
    // The panel was computed from the snapshot that has just been declared stale, so landing it
    // would tint cells under a banner saying not to trust them. **Belt-and-braces**: going stale
    // clears `pickedUp` in the same action, so the grid would not tint even without the ticket
    // bump. Asserted because it is the guarantee a user depends on, not because the bump is the
    // only thing providing it.
    expect(screen.queryByText(/will take it/)).toBeNull();
    expect(document.querySelectorAll("[data-droppable]")).toHaveLength(0);
    expect(screen.getByRole("button", { name: "Reopen it" })).toBeTruthy();
  });

  it("refuses a second write while the first is still in flight", async () => {
    const user = userEvent.setup();
    await open();
    const pending = deferred<EditStateView>();
    setLessonLock.mockReturnValueOnce(pending.promise);
    await user.click(screen.getByRole("button", { name: "Lock" }));

    // What this proves is the **user-visible** guarantee: while a write is in flight the other
    // write affordances are disabled, including the grid's own per-lesson buttons, so a second
    // edit cannot be started by clicking. The ref behind it is proven by the same-tick test
    // above; this one is proven by `busy`, and both are wanted.
    await user.click(screen.getByRole("button", { name: "Unschedule" })).catch(() => undefined);
    expect(unscheduleLesson).not.toHaveBeenCalled();

    pending.settle(aState({ version: 1, locked: 1 }));
    await waitFor(() => expect(screen.getByText(/1 locked/)).toBeTruthy());
  });
});

// ---------------------------------------------------------------- provenance

describe("historical provenance", () => {
  it("says the findings are about the arrangement now, not about the generation", async () => {
    await open();
    expect(
      screen.getByText(/Everything below is about this arrangement as it stands now/),
    ).toBeTruthy();
    expect(screen.getByText(/separate, unchangeable record/)).toBeTruthy();
  });

  it("shows no generation score of its own, so there is none to mistake for the current one", async () => {
    await open();
    // The read-only screen owns the two-scorings panel; this one must not grow a second,
    // undated score beside the current findings.
    expect(screen.queryByText("As generated")).toBeNull();
    expect(screen.queryByRole("region", { name: "Scoring" })).toBeNull();
  });
});
