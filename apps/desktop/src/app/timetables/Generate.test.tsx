/**
 * The Generate screen.
 *
 * The Rust tests own the session state machine and the save's refusals. This file owns what
 * only the interface can get wrong:
 *
 * - a blocking finding **disables** Generate, because searching over inputs that were refused
 *   reports a setup question as an unplaceable lesson;
 * - a warning does not;
 * - the grid's existing timetable count is on screen *before* generating, because a save
 *   adds rather than replaces;
 * - progress is announced politely and the bar reflects it;
 * - an unsaveable result shows why and offers no name box;
 * - a stale-input refusal shows **Generate again**, not an error.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { GenerateResult, ValidationView } from "@evara/domain";

const validateInputs = vi.fn<() => Promise<ValidationView>>();
const startGeneration = vi.fn();
const generationResult = vi.fn<() => Promise<GenerateResult>>();
const generationScope = vi.fn();
const cancelGeneration = vi.fn();
const discardGeneration = vi.fn();
const saveGeneration = vi.fn();
const onSolverProgress = vi.fn();
const onSolverFinished = vi.fn();

vi.mock("../../ipc/timetables", () => ({
  validateInputs: () => validateInputs(),
  startGeneration: (selection: unknown) => startGeneration(selection),
  generationResult: () => generationResult(),
  generationScope: (...args: unknown[]) => generationScope(...args),
  cancelGeneration: (runId: unknown) => cancelGeneration(runId),
  discardGeneration: (runId: unknown) => discardGeneration(runId),
  saveGeneration: (runId: unknown, name: unknown) => saveGeneration(runId, name),
  onSolverProgress: (runId: unknown, handler: unknown) => onSolverProgress(runId, handler),
  onSolverFinished: (runId: unknown, handler: unknown) => onSolverFinished(runId, handler),
  describeTimetableError: () => "Something went wrong. Please try again.",
  isInputsChanged: (error: unknown) =>
    typeof error === "object" && error !== null && (error as { kind?: string }).kind === "inputsChanged",
  alreadySavedAs: (error: unknown) =>
    typeof error === "object" && error !== null && (error as { kind?: string }).kind === "alreadySaved"
      ? (error as { timetableId: string }).timetableId
      : null,
}));

const { Generate } = await import("./Generate");

const SELECTION = { schoolId: "sc-1", cycleId: "cy-1", termId: null };

function aValidation(over: Partial<ValidationView> = {}): ValidationView {
  return {
    maySolve: true,
    findings: [],
    occurrences: 6,
    unenforcedRestrictions: 0,
    existingTimetables: 0,
    ...over,
  };
}

function aResult(over: Partial<GenerateResult> = {}): GenerateResult {
  return {
    runId: "run-1",
    phase: "finished",
    outcome: {
      status: "COMPLETE",
      termination: "Exhausted",
      seed: 2026,
      hard: 0,
      medium: 0,
      soft: 0,
      placed: 6,
      required: 6,
      hardViolations: 0,
      unscheduled: [],
      unverified: {
        indeterminateStudentPairs: 0,
        uncheckedCapacities: 0,
        sharedEntities: 0,
        unenforcedRestrictions: 0,
      },
      wasCancelled: false,
    },
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
    notSaveable: null,
    savedAs: null,
    ...over,
  };
}

/** Pushes the worker's completion through whatever handler the screen registered. */
async function finishTheRun() {
  const [, handler] = onSolverFinished.mock.calls.at(-1)!;
  (handler as (event: unknown) => void)({
    runId: "run-1",
    epoch: 1,
    hasResult: true,
    elapsedMs: 120,
  });
}

beforeEach(() => {
  for (const fn of [
    validateInputs,
    startGeneration,
    generationResult,
    generationScope,
    cancelGeneration,
    discardGeneration,
    saveGeneration,
    onSolverProgress,
    onSolverFinished,
  ]) {
    fn.mockReset();
  }
  validateInputs.mockResolvedValue(aValidation());
  startGeneration.mockResolvedValue({ runId: "run-1", total: 6 });
  generationResult.mockResolvedValue(aResult());
  generationScope.mockResolvedValue({
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
  cancelGeneration.mockResolvedValue(true);
  discardGeneration.mockResolvedValue(true);
  onSolverProgress.mockResolvedValue(() => undefined);
  onSolverFinished.mockResolvedValue(() => undefined);
});

function renderScreen(onOpenSaved = vi.fn()) {
  render(
    <Generate selection={SELECTION} schoolName="Northgate" onOpenSaved={onOpenSaved} />,
  );
  return { onOpenSaved };
}

// ---------------------------------------------------------------- the gate

describe("pre-solve validation", () => {
  it("disables Generate when a finding blocks, and says why", async () => {
    validateInputs.mockResolvedValue(
      aValidation({
        maySolve: false,
        findings: [
          {
            code: "no-bell-schedule",
            severity: "blocking",
            message: "9 Mathematics B resolves to no bell schedule.",
            remedy: "Give the section a campus, or give the school one schedule.",
            blocks: true,
          },
        ],
      }),
    );
    renderScreen();

    const button = await screen.findByRole("button", { name: "Generate" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("These have to be fixed first")).toBeTruthy();
    expect(screen.getByText(/Give the section a campus/)).toBeTruthy();
    expect(
      screen.getByText(/unavailable until the items above are corrected/),
    ).toBeTruthy();
  });

  it("leaves Generate available for a warning, and files it separately", async () => {
    validateInputs.mockResolvedValue(
      aValidation({
        findings: [
          {
            code: "student-overlap-unknown",
            severity: "warning",
            message: "Two groups have not been declared disjoint.",
            remedy: null,
            blocks: false,
          },
        ],
      }),
    );
    renderScreen();

    const button = await screen.findByRole("button", { name: "Generate" });
    expect((button as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText("These have to be fixed first")).toBeNull();
    expect(screen.getByText(/1 thing worth knowing/)).toBeTruthy();
  });

  it("says how many timetables the grid already holds, before anything is generated", async () => {
    validateInputs.mockResolvedValue(aValidation({ existingTimetables: 3 }));
    renderScreen();
    expect(
      await screen.findByText(/already holds 3 timetables — saving adds another/),
    ).toBeTruthy();
  });

  it("says plainly when the grid holds none", async () => {
    renderScreen();
    expect(await screen.findByText(/holds no timetable yet/)).toBeTruthy();
  });
});

// ---------------------------------------------------------------- running

describe("while a run is going", () => {
  it("announces progress politely and reflects it in the bar", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));

    await waitFor(() => expect(onSolverProgress).toHaveBeenCalled());
    const [, handler] = onSolverProgress.mock.calls.at(-1)!;
    (handler as (event: unknown) => void)({
      runId: "run-1",
      epoch: 1,
      placed: 3,
      total: 6,
      steps: 7,
      elapsedMs: 40,
    });

    const said = await screen.findByText(/3 of 6 lessons placed/);
    expect(said.getAttribute("aria-live")).toBe("polite");
    const bar = screen.getByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("3");
    expect(bar.getAttribute("aria-valuemax")).toBe("6");
  });

  it("offers Stop, and reports that stopping is under way", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));

    const stop = await screen.findByRole("button", { name: "Stop" });
    await user.click(stop);
    expect(cancelGeneration).toHaveBeenCalledWith("run-1");
    expect(await screen.findByText(/stopping after the current step/)).toBeTruthy();
  });
});

// ---------------------------------------------------------------- the result

describe("a finished run", () => {
  it("describes itself as having no confirmed clash within the school", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();

    expect(
      await screen.findByText("No confirmed clash within Northgate."),
    ).toBeTruthy();
    expect(screen.getByText(/Every lesson is placed/)).toBeTruthy();
  });

  it("offers a name and says that saving adds rather than replaces", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();

    expect(await screen.findByLabelText("Name this timetable")).toBeTruthy();
    expect(
      screen.getByText("Saving adds a new timetable. Nothing existing is replaced."),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /overwrite|replace/i })).toBeNull();
  });

  it("shows no name box for a result that is not a timetable, and says which case it is", async () => {
    const user = userEvent.setup();
    generationResult.mockResolvedValue(
      aResult({
        notSaveable:
          "The arrangement failed Evara's own final check. This is a defect in Evara.",
      }),
    );
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();

    expect(await screen.findByText("Nothing to save")).toBeTruthy();
    expect(screen.getByText(/defect in Evara/)).toBeTruthy();
    expect(screen.queryByLabelText("Name this timetable")).toBeNull();
  });

  it("offers no seed for a cancelled run, because that one is not reproducible", async () => {
    const user = userEvent.setup();
    const result = aResult();
    generationResult.mockResolvedValue({
      ...result,
      outcome: {
        ...result.outcome,
        status: "CANCELLED",
        termination: "Cancelled",
        wasCancelled: true,
        placed: 4,
      },
    });
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();

    expect(
      await screen.findByText(/the one result Evara cannot reproduce from its seed/),
    ).toBeTruthy();
    expect(screen.queryByText(/seed 2026/)).toBeNull();
  });
});

// ---------------------------------------------------------------- saving

describe("saving", () => {
  it("opens the new timetable, which is what the user asked for", async () => {
    const user = userEvent.setup();
    saveGeneration.mockResolvedValue({
      timetable: {
        id: "tt-9",
        cycleId: "cy-1",
        termId: null,
        name: "Draft 1",
        generatedAt: "2026-10-23T14:02:00.000Z",
        rev: 1,
        unverified: {
          indeterminateStudentPairs: 0,
          uncheckedCapacities: 0,
          sharedEntities: 0,
          unenforcedRestrictions: 0,
        },
      },
      entries: 6,
      unscheduled: 0,
      violations: 0,
    });
    const { onOpenSaved } = renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();

    await user.click(await screen.findByRole("button", { name: "Save" }));
    await waitFor(() => expect(onOpenSaved).toHaveBeenCalledWith("tt-9"));
  });

  it("shows Generate again — not an error — when an input moved", async () => {
    const user = userEvent.setup();
    saveGeneration.mockRejectedValue({
      kind: "inputsChanged",
      message: "A scheduling input changed… Generate again.",
    });
    renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();
    await user.click(await screen.findByRole("button", { name: "Save" }));

    expect(await screen.findByText("Nothing was saved")).toBeTruthy();
    expect(screen.getByText(/could store a wrong timetable/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Generate again" })).toBeTruthy();
  });

  it("opens the timetable a repeated save already wrote rather than reporting a failure", async () => {
    const user = userEvent.setup();
    saveGeneration.mockRejectedValue({
      kind: "alreadySaved",
      timetableId: "tt-7",
      message: "This timetable has already been saved.",
    });
    const { onOpenSaved } = renderScreen();
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(onSolverFinished).toHaveBeenCalled());
    await finishTheRun();
    await user.click(await screen.findByRole("button", { name: "Save" }));

    await waitFor(() => expect(onOpenSaved).toHaveBeenCalledWith("tt-7"));
  });
});

// ---------------------------------------------------------------- empty state

describe("with no cycle chosen", () => {
  it("names the section that fixes it and offers to open it", async () => {
    const user = userEvent.setup();
    const onOpenSection = vi.fn();
    render(
      <Generate
        selection={null}
        schoolName="Northgate"
        onOpenSaved={vi.fn()}
        onOpenSection={onOpenSection}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: /create one under Cycles and cycle days/ }),
    );
    expect(onOpenSection).toHaveBeenCalledWith("cycles");
  });
});
