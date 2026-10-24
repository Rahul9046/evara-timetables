/**
 * The editing IPC contract.
 *
 * Generated types check the *shape* of what crosses the boundary. They cannot check the command
 * **name** or the argument **keys**, because both are strings on the Rust side too — so a typo in
 * either compiles cleanly and fails at runtime in front of a user. That is what this file is for,
 * and it is the same reason `timetables.test.ts` and `setup.test.ts` exist.
 *
 * It also pins the two things the wrappers themselves own, and nothing else, because nothing else
 * is theirs: the four error predicates, and `verdictAt`'s lookup. `verdictAt` is the only function
 * in `edit.ts` with any logic at all, and the point of the cases below is that it is an **index
 * into an answer** — it never decides that a cell is legal, and a cell the backend did not send a
 * verdict for comes back `null` rather than permitted.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CandidateCell, CandidateGrid, ProposedMove } from "@evara/domain";

const invoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const edit = await import("./edit");

const SESSION = "se-1";
const ACTIVITY = "ac-1";
const TIMETABLE = "tt-1";
const MOVE: ProposedMove = { activityId: ACTIVITY, timeslotId: "ts-1", roomId: "r-1" };

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
});

// ---------------------------------------------------------------- the command surface

describe("every editing command is called by the name and arguments the backend declares", () => {
  const cases: ReadonlyArray<[string, () => Promise<unknown>, unknown]> = [
    ["edit_open", () => edit.openForEditing(TIMETABLE), { timetableId: TIMETABLE }],
    ["edit_close", () => edit.closeEditing(SESSION), { sessionId: SESSION }],
    ["edit_state", () => edit.editState(SESSION), { sessionId: SESSION }],
    [
      "edit_scope",
      () => edit.editScope(SESSION, "teacher", "t-1"),
      { sessionId: SESSION, scopeKind: "teacher", scopeId: "t-1" },
    ],
    [
      "edit_candidates",
      () => edit.candidatesFor(SESSION, ACTIVITY, "r-1"),
      { sessionId: SESSION, activityId: ACTIVITY, roomId: "r-1" },
    ],
    [
      "edit_validate",
      () => edit.validateMove(SESSION, MOVE),
      { sessionId: SESSION, proposed: MOVE },
    ],
    [
      "edit_apply",
      () => edit.applyMove(SESSION, MOVE, 7),
      { sessionId: SESSION, proposed: MOVE, expectedRev: 7 },
    ],
    [
      "edit_unschedule",
      () => edit.unscheduleLesson(SESSION, ACTIVITY, 7),
      { sessionId: SESSION, activityId: ACTIVITY, expectedRev: 7 },
    ],
    [
      "edit_set_lock",
      () => edit.setLessonLock(SESSION, ACTIVITY, true, 7),
      { sessionId: SESSION, activityId: ACTIVITY, locked: true, expectedRev: 7 },
    ],
    ["edit_undo", () => edit.undoEdit(SESSION), { sessionId: SESSION }],
    ["edit_redo", () => edit.redoEdit(SESSION), { sessionId: SESSION }],
    ["edit_history", () => edit.editHistory(SESSION), { sessionId: SESSION }],
    [
      "timetable_duplicate",
      () => edit.duplicateTimetable(TIMETABLE, 3, "Draft B"),
      { id: TIMETABLE, expectedRev: 3, name: "Draft B" },
    ],
  ];

  for (const [name, call, args] of cases) {
    it(`calls ${name}`, async () => {
      await call();
      expect(invoke).toHaveBeenCalledWith(name, args);
    });
  }

  it("covers every command the backend registers", () => {
    // The thirteen in `lib.rs`'s `generate_handler!`. A fourteenth added there and not wrapped
    // here would otherwise be invisible until a screen needed it.
    expect(cases).toHaveLength(13);
  });
});

describe("a placement carries no revision precondition", () => {
  it("sends null for an occurrence that is not yet placed", async () => {
    await edit.applyMove(SESSION, MOVE, null);
    expect(invoke).toHaveBeenCalledWith("edit_apply", {
      sessionId: SESSION,
      proposed: MOVE,
      expectedRev: null,
    });
  });
});

// ---------------------------------------------------------------- the four refusals

describe("the refusals a screen has to tell apart", () => {
  const locked = { kind: "locked", message: "9 Mathematics B is locked." };
  const changed = {
    kind: "arrangementChanged",
    entriesChanged: 2,
    message: "Another window changed this timetable.",
  };
  const gone = { kind: "notFound", entity: "edit session", message: "No such session." };
  const conflict = {
    kind: "conflict",
    entity: "timetable entry",
    expectedRev: 3,
    actualRev: 4,
    message: "This placement moved.",
  };

  it("recognises a locked placement", () => {
    expect(edit.isLocked(locked)).toBe(true);
    expect(edit.isLocked(conflict)).toBe(false);
  });

  it("recognises another window's change to a different placement", () => {
    expect(edit.isArrangementChanged(changed)).toBe(true);
    // The distinction the whole `ArrangementStamp` exists for: a revision conflict is about
    // *this* placement and a retry can answer it; an arrangement change is about another one and
    // the session has to be reopened.
    expect(edit.isArrangementChanged(conflict)).toBe(false);
  });

  it("recognises a session that is gone", () => {
    expect(edit.isSessionGone(gone)).toBe(true);
    expect(edit.isSessionGone(locked)).toBe(false);
  });

  it("uses the backend's own sentence, never one of its own", () => {
    expect(edit.describeEditError(locked)).toBe("9 Mathematics B is locked.");
  });

  it("says something honest about a failure it does not recognise", () => {
    expect(edit.describeEditError(new Error("boom"))).toBe(
      "Something went wrong. Please try again.",
    );
  });
});

// ---------------------------------------------------------------- the one lookup

describe("verdictAt is an index into the backend's answer", () => {
  function cell(over: Partial<CandidateCell>): CandidateCell {
    return {
      timeslotId: "ts-1",
      dayOrdinal: 1,
      periodOrdinal: 1,
      allowed: true,
      isCurrent: false,
      blocking: [],
      mediumDelta: 0,
      ...over,
    };
  }

  const grid: CandidateGrid = {
    activityId: ACTIVITY,
    scheduleId: "bs-1",
    roomId: "r-1",
    rooms: [],
    roomlessAllowed: false,
    cells: [
      cell({ timeslotId: "ts-1", dayOrdinal: 1, periodOrdinal: 1, allowed: true }),
      cell({
        timeslotId: "ts-2",
        dayOrdinal: 2,
        periodOrdinal: 3,
        allowed: false,
        blocking: ["Break is not a teaching period."],
      }),
    ],
    unavailable: null,
  };

  it("finds the cell at one position", () => {
    expect(edit.verdictAt(grid, 1, 1)?.timeslotId).toBe("ts-1");
    expect(edit.verdictAt(grid, 2, 3)?.blocking).toEqual(["Break is not a teaching period."]);
  });

  it("returns null for a position the backend sent no verdict for", () => {
    // Not `allowed: false`, and emphatically not `allowed: true`. No verdict is a *missing
    // answer*, and a grid that rendered it as either would be deciding legality here.
    expect(edit.verdictAt(grid, 1, 2)).toBeNull();
  });

  it("returns null when nothing is in hand", () => {
    expect(edit.verdictAt(null, 1, 1)).toBeNull();
  });
});
