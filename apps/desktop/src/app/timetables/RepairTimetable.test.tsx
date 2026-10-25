/**
 * The repair screen.
 *
 * The Rust tests own what a repair *is*: what may move, what stays frozen, which violations are
 * the repair's doing, what a lock means, which guard refuses a stale apply. None of that is
 * asserted here and none of it could be. This file owns what only the interface can get wrong,
 * and every case is one a reasonable implementation gets wrong in the obvious direction:
 *
 * - the scope is **never defaulted**, so Repair is unavailable until the user picks one;
 * - the scope picker is a real radio group with **arrow-key navigation**, because a list of
 *   buttons looks the same and is not the same;
 * - a **positional** scope warns that it cannot schedule unscheduled lessons, *before* the run;
 * - a **blocking** pre-flight finding disables Repair; an advisory one does not;
 * - the no-change result says *"this repair attempt made no changes"* and never
 *   *"nothing could be improved"*;
 * - a **pre-existing** violation outside the scope is labelled as such and does not stop Apply;
 * - the scope is never widened in place — the offer is to start a **separate** repair;
 * - Apply is explicit and named, Discard ends the session, and the three refusals get three
 *   different remedies;
 * - a scope violation is reported as **our defect**, not as the user's data to correct.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  RepairPreflight,
  RepairResult,
  RepairScopes,
  ScopeOption,
} from "@evara/domain";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

const repairScopes = vi.fn();
const repairPreflight = vi.fn();
const startRepair = vi.fn();
const repairResult = vi.fn();
const cancelRepair = vi.fn();
const discardRepair = vi.fn();
const applyRepair = vi.fn();
// Typed with their parameters, so the mock has the same shape as what it replaces: a bare
// `async () => ...` infers no arguments and then rejects the handler the screen passes.
const onRepairProgress =
  vi.fn<(runId: string, handler: (event: unknown) => void) => Promise<() => void>>();
const onRepairFinished =
  vi.fn<(runId: string, handler: (event: unknown) => void) => Promise<() => void>>();

vi.mock("../../ipc/repair", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/repair")>()),
  repairScopes: (...args: unknown[]) => repairScopes(...args),
  repairPreflight: (...args: unknown[]) => repairPreflight(...args),
  startRepair: (...args: unknown[]) => startRepair(...args),
  repairResult: (...args: unknown[]) => repairResult(...args),
  cancelRepair: (...args: unknown[]) => cancelRepair(...args),
  discardRepair: (...args: unknown[]) => discardRepair(...args),
  applyRepair: (...args: unknown[]) => applyRepair(...args),
  onRepairProgress: (runId: string, handler: (event: unknown) => void) =>
    onRepairProgress(runId, handler),
  onRepairFinished: (runId: string, handler: (event: unknown) => void) =>
    onRepairFinished(runId, handler),
}));

const { RepairTimetable } = await import("./RepairTimetable");

const SELECTION = { schoolId: "school", cycleId: "cycle", termId: null };

function option(over: Partial<ScopeOption> = {}): ScopeOption {
  return {
    scope: { kind: "whole" },
    key: "whole",
    label: "Whole timetable",
    movable: 12,
    frozen: 0,
    positional: false,
    unreachableUnplaced: 0,
    ...over,
  } as ScopeOption;
}

function scopes(options: ScopeOption[]): RepairScopes {
  return {
    timetableId: "tt",
    options,
    total: 12,
    unplaced: 2,
  } as RepairScopes;
}

function preflight(over: Partial<RepairPreflight> = {}): RepairPreflight {
  return {
    mayRepair: true,
    findings: [],
    movable: 12,
    frozen: 0,
    locked: 0,
    ...over,
  } as RepairPreflight;
}

function result(over: Partial<RepairResult["diff"]> = {}, top: Partial<RepairResult> = {}) {
  return {
    runId: "run",
    phase: "finished",
    scope: { kind: "whole" },
    notApplyable: null,
    termination: "complete",
    status: "COMPLETE",
    stoppedAtALimit: false,
    defect: null,
    appliedAs: null,
    diff: {
      changes: [],
      counts: {
        unchanged: 12,
        moved: 0,
        newlyScheduled: 0,
        unscheduled: 0,
        stillUnplaced: 0,
      },
      movableCount: 12,
      frozenCount: 0,
      scoreBefore: { hard: 0, medium: 0, soft: 0 },
      scoreAfter: { hard: 0, medium: 0, soft: 0 },
      improved: false,
      findingsBefore: [],
      findingsAfter: [],
      countsBefore: { hard: 0, medium: 0, soft: 0, unknown: 0 },
      countsAfter: { hard: 0, medium: 0, soft: 0, unknown: 0 },
      placedAfter: 12,
      required: 12,
      attribution: {
        preExistingFrozen: [],
        newInvolvingMovable: [],
        eliminated: [],
      },
      ...over,
    },
    ...top,
  } as unknown as RepairResult;
}

function mount(over: Partial<Parameters<typeof RepairTimetable>[0]> = {}) {
  return render(
    <RepairTimetable
      selection={SELECTION}
      timetableId="tt"
      timetableName="Draft A"
      schoolName="Northgate"
      onApplied={vi.fn()}
      onDone={vi.fn()}
      {...over}
    />,
  );
}

/** One finding, with every field the generated type requires. */
function finding(detail: string) {
  return {
    typeKey: "room-clash",
    knownType: true,
    level: "HARD",
    penalty: -1,
    message: detail,
    detail,
    cycleDayId: null,
    startsAt: null,
    endsAt: null,
    subject: null,
    related: [],
    activities: [],
    source: "evaluated",
  } as unknown as import("@evara/domain").FindingView;
}

/** Finishes a run: start, then fire the finished listener the screen registered. */
async function finishWith(found: RepairResult) {
  startRepair.mockResolvedValue({ runId: "run", movable: 12, frozen: 0 });
  repairResult.mockResolvedValue(found);
  let fire: (() => void) | null = null;
  onRepairFinished.mockImplementation(async (_runId, handler) => {
    fire = () => handler({ runId: "run", epoch: 0, hasResult: true });
    return () => undefined;
  });
  await userEvent.click(screen.getByRole("button", { name: "Repair" }));
  await waitFor(() => expect(fire).not.toBeNull());
  // Inside `act`, or React never processes the state the listener sets and the preview the
  // rest of the test is about is never rendered.
  await act(async () => {
    fire?.();
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  repairScopes.mockResolvedValue(scopes([option()]));
  repairPreflight.mockResolvedValue(preflight());
  onRepairProgress.mockResolvedValue(() => undefined);
  onRepairFinished.mockResolvedValue(() => undefined);
});

describe("choosing a scope", () => {
  it("says the repair makes a new timetable and leaves the original alone", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    expect(screen.getByText(/makes a/i).textContent).toMatch(/new/i);
    expect(screen.getByText(/left exactly as it is/i)).toBeTruthy();
  });

  it("defaults no scope, so Repair is unavailable until one is chosen", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());

    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", true);
    expect(repairPreflight).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", false);
  });

  it("offers the scopes as a real radio group", async () => {
    repairScopes.mockResolvedValue(
      scopes([
        option(),
        option({
          key: "teacher",
          label: "A. Teacher",
          scope: { kind: "teacher", teacherId: "t1" },
          movable: 4,
          frozen: 8,
        }),
      ]),
    );
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());

    const group = screen.getByRole("radiogroup");
    expect(within(group).getAllByRole("radio")).toHaveLength(2);
  });

  it("moves the selection with the arrow keys, which a list of buttons would not", async () => {
    repairScopes.mockResolvedValue(
      scopes([
        option(),
        option({
          key: "teacher",
          label: "A. Teacher",
          scope: { kind: "teacher", teacherId: "t1" },
        }),
      ]),
    );
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());

    const first = screen.getByRole("radio", { name: /Whole timetable/ });
    await userEvent.click(first);
    await waitFor(() => expect(repairPreflight).toHaveBeenCalledTimes(1));

    await userEvent.keyboard("{ArrowDown}");
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /A. Teacher/ })).toHaveProperty("checked", true),
    );
  });

  it("warns before the run that a positional scope cannot schedule unscheduled lessons", async () => {
    repairScopes.mockResolvedValue(
      scopes([
        option({
          key: "cycleDay",
          label: "Day 1",
          scope: { kind: "cycleDay", cycleDayId: "d1" },
          positional: true,
          unreachableUnplaced: 3,
          movable: 5,
          frozen: 7,
        }),
      ]),
    );
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Day 1/ }));

    // Stated before Repair is pressed, not discovered in the diff afterwards.
    await waitFor(() =>
      expect(screen.getByText(/cannot schedule your 3/i)).toBeTruthy(),
    );
    expect(screen.getByText(/in no room and on no day/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", false);
  });

  it("always warns that lessons outside the scope will not move", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(screen.getByText(/will not move/i)).toBeTruthy());
  });
});

describe("the pre-flight", () => {
  it("disables Repair on a blocking finding and shows its remedy", async () => {
    repairPreflight.mockResolvedValue(
      preflight({
        mayRepair: false,
        findings: [
          {
            code: "timeslot-not-in-problem",
            blocking: true,
            message: "This lock names a period that is no longer in the bell schedule.",
            remedy: "Unschedule or re-place that lesson, then lock it again.",
            activityId: null,
          },
        ],
      }),
    );
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));

    await waitFor(() =>
      expect(screen.getByText(/no longer in the bell schedule/i)).toBeTruthy(),
    );
    expect(screen.getByText(/then lock it again/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", true);
  });

  it("does not disable Repair for a pre-existing problem outside the scope", async () => {
    repairPreflight.mockResolvedValue(
      preflight({
        mayRepair: true,
        findings: [
          {
            code: "room-clash",
            blocking: false,
            message: "Already present, and outside the scope you chose: Room 3 is double-booked.",
            remedy: "This scope cannot reach it. Repair a wider scope to address it.",
            activityId: null,
          },
        ],
      }),
    );
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));

    await waitFor(() => expect(screen.getByText(/double-booked/i)).toBeTruthy());
    // Reported, and labelled as not the repair's doing -- but not a bar to running it.
    expect(screen.getByText(/do not stop you applying it/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", false);
  });
});

describe("the preview", () => {
  it("says the attempt made no changes, never that nothing could be improved", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(result());

    await waitFor(() =>
      expect(screen.getByText(/This repair attempt made no changes/i)).toBeTruthy(),
    );
    // The phrase appears once, and only as a disclaimer of what is NOT being claimed --
    // never as the headline. The headline is the attempt, not the possibility.
    expect(screen.getByText(/not a statement that nothing could be improved/i)).toBeTruthy();
    expect(screen.queryByText(/^Nothing could be improved/i)).toBeNull();
    expect(screen.queryByText(/optimal/i)).toBeNull();
    expect(screen.queryByText(/best possible/i)).toBeNull();
  });

  it("labels a pre-existing violation as not the repair's doing and still offers Apply", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(
      result({
        scoreAfter: { hard: -1, medium: 0, soft: 0 },
        countsAfter: { hard: 1, medium: 0, soft: 0, unknown: 0 },
        attribution: {
          preExistingFrozen: [finding("Room 3 is double-booked on Day 1 Period 2")],
          newInvolvingMovable: [],
          eliminated: [],
        },
      }),
    );

    await waitFor(() =>
      expect(screen.getByText(/Already present, outside the scope you chose/i)).toBeTruthy(),
    );
    expect(screen.getByText(/not a reason to withhold it/i)).toBeTruthy();
    // A hard violation in the arrangement is NOT a bar to applying: it was already there.
    expect(
      screen.getByRole("button", { name: /Apply as a new timetable/i }),
    ).toHaveProperty("disabled", false);
  });

  it("offers a separate repair rather than widening this one", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(
      result({
        attribution: {
          preExistingFrozen: [finding("Room 3 is double-booked")],
          newInvolvingMovable: [],
          eliminated: [],
        },
      }),
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Start a separate repair with a wider scope/i }),
      ).toBeTruthy(),
    );
    // Taking it returns to the picker rather than re-running with more movable -- a new
    // session, a new preview, a new approval.
    await userEvent.click(
      screen.getByRole("button", { name: /Start a separate repair with a wider scope/i }),
    );
    await waitFor(() => expect(screen.getByRole("radiogroup")).toBeTruthy());
    expect(screen.getByRole("button", { name: "Repair" })).toHaveProperty("disabled", true);
  });

  it("names a limit rather than presenting it as a conclusion", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(
      result({}, { stoppedAtALimit: true, termination: "candidate-limit-reached" }),
    );

    await waitFor(() => expect(screen.getByText(/A limit stopped this repair/i)).toBeTruthy());
    expect(screen.getByText(/not a conclusion about what is possible/i)).toBeTruthy();
    // Still applyable: an honest partial arrangement is a useful result.
    expect(
      screen.getByRole("button", { name: /Apply as a new timetable/i }),
    ).toHaveProperty("disabled", false);
  });

  it("shows every change as a word as well as a tint", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(
      result({
        counts: {
          unchanged: 10,
          moved: 1,
          newlyScheduled: 0,
          unscheduled: 0,
          stillUnplaced: 1,
        },
        changes: [
          {
            activityId: "a1",
            kind: "moved",
            inScope: true,
            isLocked: false,
            lesson: "9 Mathematics B",
            periods: 1,
            before: {
              timeslotId: "s1",
              day: "Day A",
              period: "1",
              room: "Room B12",
              roomId: "r1",
            },
            after: {
              timeslotId: "s2",
              day: "Day C",
              period: "4",
              room: "Room C04",
              roomId: "r2",
            },
            moved: { slot: true, room: true },
            cause: null,
          },
        ],
      }),
    );

    await waitFor(() => expect(screen.getByText("9 Mathematics B")).toBeTruthy());
    const row = screen.getByText("9 Mathematics B").closest("tr");
    expect(row).not.toBeNull();
    // The word is in the cell, so the tint is never the only signal.
    expect(within(row as HTMLElement).getByText("Moved")).toBeTruthy();
    expect(within(row as HTMLElement).getByText(/Day A 1, Room B12/)).toBeTruthy();
    expect(within(row as HTMLElement).getByText(/Day C 4, Room C04/)).toBeTruthy();
  });

  it("separates what the scope could not place from what it never attempted", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());

    await finishWith(
      result({
        counts: {
          unchanged: 10,
          moved: 0,
          newlyScheduled: 0,
          unscheduled: 0,
          stillUnplaced: 2,
        },
        changes: [
          {
            activityId: "a1",
            kind: "stillUnplaced",
            inScope: true,
            isLocked: false,
            lesson: "In scope, blocked",
            periods: 1,
            before: null,
            after: null,
            moved: null,
            cause: "blocked-by-placements",
          },
          {
            activityId: "a2",
            kind: "stillUnplaced",
            inScope: false,
            isLocked: false,
            lesson: "Never attempted",
            periods: 1,
            before: null,
            after: null,
            moved: null,
            cause: null,
          },
        ],
      }),
    );

    await waitFor(() =>
      expect(screen.getByText(/This scope could not place these/i)).toBeTruthy(),
    );
    expect(screen.getByText(/Unscheduled, and outside this scope/i)).toBeTruthy();
    // The remedies differ, which is the whole reason they are separated.
    expect(screen.getByText(/may be the reason, rather than the timetable/i)).toBeTruthy();
    expect(screen.getByText(/says nothing about them/i)).toBeTruthy();
  });
});

describe("applying", () => {
  it("applies under an explicit name and reports the new timetable", async () => {
    const onApplied = vi.fn();
    applyRepair.mockResolvedValue({
      timetableId: "new",
      basedOn: "tt",
      entries: 12,
    });
    mount({ onApplied });
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());

    const field = await screen.findByLabelText(/Name for the new timetable/i);
    expect(field).toHaveProperty("value", "Draft A (repaired)");
    await userEvent.clear(field);
    await userEvent.type(field, "Draft A fixed");
    await userEvent.click(screen.getByRole("button", { name: /Apply as a new timetable/i }));

    await waitFor(() => expect(applyRepair).toHaveBeenCalledWith("run", "Draft A fixed"));
    expect(onApplied).toHaveBeenCalledWith("new");
    expect(screen.getByText(/is unchanged/i)).toBeTruthy();
  });

  it("refuses to apply without a name", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());

    const field = await screen.findByLabelText(/Name for the new timetable/i);
    await userEvent.clear(field);
    expect(
      screen.getByRole("button", { name: /Apply as a new timetable/i }),
    ).toHaveProperty("disabled", true);
  });

  it("does not offer Apply for a result the backend will not accept", async () => {
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(
      result({}, { notApplyable: "the repair was cancelled, so there is nothing to apply" }),
    );

    await waitFor(() =>
      expect(screen.getByText(/This result cannot be applied/i)).toBeTruthy(),
    );
    expect(
      screen.getByRole("button", { name: /Apply as a new timetable/i }),
    ).toHaveProperty("disabled", true);
  });

  it("gives an edited baseline its own remedy, not a retry", async () => {
    applyRepair.mockRejectedValue({
      kind: "invalid",
      message: "the arrangement was edited since the repair began",
    });
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());
    await userEvent.click(screen.getByRole("button", { name: /Apply as a new timetable/i }));

    await waitFor(() =>
      expect(screen.getByText(/edited while you were repairing it/i)).toBeTruthy(),
    );
    expect(screen.getByText(/no longer exists/i)).toBeTruthy();
  });

  it("gives a moved input a different remedy from an edited baseline", async () => {
    applyRepair.mockRejectedValue({
      kind: "invalid",
      message: "a scheduling input moved since the problem was compiled",
    });
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());
    await userEvent.click(screen.getByRole("button", { name: /Apply as a new timetable/i }));

    await waitFor(() =>
      expect(screen.getByText(/Something the timetable depends on changed/i)).toBeTruthy(),
    );
  });

  it("reports a scope violation as our defect, not the user's data", async () => {
    applyRepair.mockRejectedValue({
      kind: "invalid",
      message:
        "the repair changed 2 placement(s) outside the scope it was given, so it was not saved",
    });
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());
    await userEvent.click(screen.getByRole("button", { name: /Apply as a new timetable/i }));

    await waitFor(() =>
      expect(screen.getByText(/defect in Evara, not a problem with your timetable/i)).toBeTruthy(),
    );
    expect(screen.getByText(/our bug to fix, not your data to correct/i)).toBeTruthy();
    expect(screen.getByText(/Your\s+timetable is unchanged/i)).toBeTruthy();
  });

  it("discards the session when the user leaves", async () => {
    const onDone = vi.fn();
    mount({ onDone });
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await finishWith(result());

    await userEvent.click(screen.getByRole("button", { name: /Discard this repair/i }));
    await waitFor(() => expect(discardRepair).toHaveBeenCalledWith("run"));
    expect(onDone).toHaveBeenCalled();
  });
});

describe("cancelling", () => {
  it("is honest that cancelling is a request", async () => {
    startRepair.mockResolvedValue({ runId: "run", movable: 12, frozen: 0 });
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Repair" }));

    await waitFor(() => expect(screen.getByText(/Repairing/i)).toBeTruthy());
    expect(screen.getByText(/may already have finished/i)).toBeTruthy();
    expect(screen.getByText(/nothing is written either way/i)).toBeTruthy();

    await userEvent.click(screen.getByRole("button", { name: /Cancel this repair/i }));
    await waitFor(() => expect(cancelRepair).toHaveBeenCalledWith("run"));
  });

  it("announces progress in a live region", async () => {
    startRepair.mockResolvedValue({ runId: "run", movable: 12, frozen: 0 });
    mount();
    await waitFor(() => expect(repairScopes).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("radio", { name: /Whole timetable/ }));
    await waitFor(() => expect(repairPreflight).toHaveBeenCalled());
    await userEvent.click(screen.getByRole("button", { name: "Repair" }));

    await waitFor(() => expect(screen.getByRole("progressbar")).toBeTruthy());
    const live = document.querySelector('[aria-live="polite"]');
    expect(live).not.toBeNull();
  });
});
