/**
 * The timetable IPC contract.
 *
 * Generated types check the *shape* of what crosses the boundary. They cannot check the
 * command **name** or the argument **keys**, because both are strings on the Rust side too —
 * so a typo in either compiles cleanly and fails at runtime in front of a user. That is what
 * this file is for, and it is the same reason `setup.test.ts` exists.
 *
 * It also pins the four behaviours the wrappers themselves own: event filtering by run,
 * the three error predicates, and the scope lookup.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectedLesson, ScopeKind } from "@evara/domain";

const invoke = vi.fn();
const listen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listen(...args),
}));

const timetables = await import("./timetables");

const SELECTION = { schoolId: "sc-1", cycleId: "cy-1", termId: null };
const RUN = "run-1";
const ID = "tt-1";
const REV = 4;

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  listen.mockReset();
  listen.mockResolvedValue(() => undefined);
});

// ---------------------------------------------------------------- the command surface

describe("every command is called by the name and arguments the backend declares", () => {
  const cases: ReadonlyArray<[string, () => Promise<unknown>, unknown]> = [
    ["timetable_validate", () => timetables.validateInputs(SELECTION), { selection: SELECTION }],
    [
      "timetable_generate_start",
      () => timetables.startGeneration(SELECTION),
      { selection: SELECTION, settings: null },
    ],
    [
      "timetable_generate_status",
      () => timetables.generationStatus(RUN),
      { runId: RUN },
    ],
    [
      "timetable_generate_result",
      () => timetables.generationResult(RUN),
      { runId: RUN },
    ],
    [
      "timetable_generate_cancel",
      () => timetables.cancelGeneration(RUN),
      { runId: RUN },
    ],
    [
      "timetable_generate_discard",
      () => timetables.discardGeneration(RUN),
      { runId: RUN },
    ],
    [
      "timetable_generate_scope",
      () => timetables.generationScope(RUN, "teacher", "t-1"),
      { runId: RUN, scopeKind: "teacher", scopeId: "t-1" },
    ],
    [
      "timetable_save",
      () => timetables.saveGeneration(RUN, "Draft 1"),
      { runId: RUN, name: "Draft 1" },
    ],
    ["timetable_list", () => timetables.listTimetables("sc-1"), { schoolId: "sc-1" }],
    [
      "timetable_list_for_cycle",
      () => timetables.listTimetablesForCycle("cy-1", null),
      { cycleId: "cy-1", termId: null },
    ],
    ["timetable_open", () => timetables.openTimetable(ID), { id: ID }],
    [
      "timetable_scope",
      () => timetables.timetableScope(ID, "room", "r-1"),
      { id: ID, scopeKind: "room", scopeId: "r-1" },
    ],
    [
      "timetable_rename",
      () => timetables.renameTimetable(ID, REV, "Renamed"),
      { id: ID, expectedRev: REV, name: "Renamed" },
    ],
    [
      "timetable_delete",
      () => timetables.deleteTimetable(ID, REV),
      { id: ID, expectedRev: REV },
    ],
    [
      "timetable_revalidate",
      () => timetables.revalidateTimetable(ID, REV),
      { id: ID, expectedRev: REV },
    ],
  ];

  for (const [name, call, args] of cases) {
    it(name, async () => {
      await call();
      expect(invoke).toHaveBeenCalledWith(name, args);
    });
  }

  it("passes the settings through when a caller chooses limits", async () => {
    await timetables.startGeneration(SELECTION, { seed: 7, maxCandidates: null, maxSteps: null, maxPasses: null });
    expect(invoke).toHaveBeenCalledWith("timetable_generate_start", {
      selection: SELECTION,
      settings: { seed: 7, maxCandidates: null, maxSteps: null, maxPasses: null },
    });
  });

  it("carries expectedRev on every write that takes one, and on no read", async () => {
    // The three writes that take a revision, and the reads that must not invent one.
    const writes = ["timetable_rename", "timetable_delete", "timetable_revalidate"];
    const reads = ["timetable_list", "timetable_open", "timetable_scope"];
    for (const [name, call, args] of cases) {
      invoke.mockClear();
      await call();
      const sent = (args ?? {}) as Record<string, unknown>;
      if (writes.includes(name)) {
        expect(sent).toHaveProperty("expectedRev");
      }
      if (reads.includes(name)) {
        expect(sent).not.toHaveProperty("expectedRev");
      }
    }
  });
});

describe("saving", () => {
  it("has no overwrite or replace argument to pass", async () => {
    // A save always inserts, so there is deliberately nothing for a caller to set here.
    await timetables.saveGeneration(RUN, "Draft 1");
    const [, args] = invoke.mock.calls[0]!;
    expect(Object.keys(args as object).sort()).toEqual(["name", "runId"]);
  });
});

// ---------------------------------------------------------------- events

describe("solver events", () => {
  /** Captures the handler `listen` was given, so a payload can be pushed through it. */
  function captureHandler(): (event: { payload: unknown }) => void {
    const [, handler] = listen.mock.calls[0]!;
    return handler as (event: { payload: unknown }) => void;
  }

  it("subscribes to the documented event names", async () => {
    await timetables.onSolverProgress(RUN, () => undefined);
    expect(listen.mock.calls[0]![0]).toBe("solver://progress");

    listen.mockClear();
    await timetables.onSolverFinished(RUN, () => undefined);
    expect(listen.mock.calls[0]![0]).toBe("solver://finished");
  });

  it("delivers a progress event for this run", async () => {
    const seen = vi.fn();
    await timetables.onSolverProgress(RUN, seen);
    captureHandler()({
      payload: { runId: RUN, epoch: 1, placed: 3, total: 10, steps: 5, elapsedMs: 42 },
    });
    expect(seen).toHaveBeenCalledOnce();
  });

  it("ignores a progress event for another run", async () => {
    // The guarantee: a session the user has moved on from cannot drive the screen in front
    // of them. The backend emits to every window; the filter is here.
    const seen = vi.fn();
    await timetables.onSolverProgress(RUN, seen);
    captureHandler()({
      payload: { runId: "someone-else", epoch: 1, placed: 3, total: 10, steps: 5, elapsedMs: 42 },
    });
    expect(seen).not.toHaveBeenCalled();
  });

  it("ignores a finished event for another run", async () => {
    const seen = vi.fn();
    await timetables.onSolverFinished(RUN, seen);
    captureHandler()({
      payload: { runId: "someone-else", epoch: 1, hasResult: true, elapsedMs: 1 },
    });
    expect(seen).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- error predicates

describe("the refusals a screen has to tell apart", () => {
  it("recognises a stale-input refusal, whose remedy is to generate again", () => {
    expect(
      timetables.isInputsChanged({ kind: "inputsChanged", message: "Generate again." }),
    ).toBe(true);
    expect(timetables.isInputsChanged({ kind: "reviewRequired", message: "" })).toBe(false);
    expect(timetables.isInputsChanged(new Error("boom"))).toBe(false);
  });

  it("recognises a result that is not a timetable", () => {
    expect(timetables.isNotSaveable({ kind: "notSaveable", message: "" })).toBe(true);
    expect(timetables.isNotSaveable({ kind: "invalid", message: "" })).toBe(false);
  });

  it("recovers the timetable a repeated save already wrote", () => {
    expect(
      timetables.alreadySavedAs({
        kind: "alreadySaved",
        timetableId: "tt-9",
        message: "",
      }),
    ).toBe("tt-9");
    expect(timetables.alreadySavedAs({ kind: "invalid", message: "" })).toBeNull();
  });

  it("falls back to a plain sentence for anything it does not recognise", () => {
    expect(timetables.describeTimetableError(new Error("boom"))).toBe(
      "Something went wrong. Please try again.",
    );
  });
});

// ---------------------------------------------------------------- the scope lookup

describe("the scope lookup", () => {
  function lesson(over: Partial<ProjectedLesson> = {}): ProjectedLesson {
    return {
      activityId: "a1",
      sectionId: "s1",
      label: null,
      activityName: null,
      teacherIds: ["t1"],
      teachers: [],
      groupIds: [],
      groups: [],
      roomId: "r1",
      room: null,
      scheduleId: "bs1",
      cycleDayId: "d1",
      dayOrdinal: 1,
      occupies: [],
      startsAt: "09:00",
      endsAt: "10:00",
      durationPeriods: 1,
      defect: null,
      flags: { isLocked: false, isManual: false },
      ...over,
    };
  }

  const lessons = [
    lesson({ activityId: "a1", sectionId: "s1", teacherIds: ["t1"], roomId: "r1" }),
    lesson({ activityId: "a2", sectionId: "s2", teacherIds: ["t1", "t2"], roomId: null }),
    lesson({ activityId: "a3", sectionId: "s1", teacherIds: ["t2"], roomId: "r2" }),
  ];

  const found = (kind: ScopeKind, id: string | null, roster?: string[]) =>
    timetables.indexByScope(lessons, kind, id, roster);

  it("takes every lesson for the master scope", () => {
    expect(found("master", null)).toEqual([0, 1, 2]);
  });

  it("takes a section's own lessons", () => {
    expect(found("section", "s1")).toEqual([0, 2]);
  });

  it("puts a co-taught lesson on every teacher's list", () => {
    expect(found("teacher", "t1")).toEqual([0, 1]);
    expect(found("teacher", "t2")).toEqual([1, 2]);
  });

  it("leaves a roomless lesson out of every room's list", () => {
    expect(found("room", "r1")).toEqual([0]);
    expect(found("room", "r2")).toEqual([2]);
  });

  it("derives a student's lessons from the roster and nothing else", () => {
    expect(found("student", "st1", ["s2"])).toEqual([1]);
  });

  it("gives a student with no roster an empty list, never a group's timetable", () => {
    expect(found("student", "st1", [])).toEqual([]);
    expect(found("student", "st1")).toEqual([]);
  });

  it("filters the scopes of one kind, in the order the backend listed them", () => {
    const scopes = [
      { kind: "master" as ScopeKind, id: null, name: "All", secondary: null, lessonCount: 3, rosterDerived: false },
      { kind: "teacher" as ScopeKind, id: "t1", name: "One", secondary: null, lessonCount: 2, rosterDerived: false },
      { kind: "teacher" as ScopeKind, id: "t2", name: "Two", secondary: null, lessonCount: 2, rosterDerived: false },
    ];
    expect(timetables.scopesOfKind(scopes, "teacher").map((s) => s.id)).toEqual(["t1", "t2"]);
  });
});
