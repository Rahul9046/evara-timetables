/**
 * The grid screen's safety behaviour, tested at the component level.
 *
 * The repository tests already prove the backend cannot be made to do the wrong thing.
 * These tests prove the *screen* asks the right questions, because several of the
 * guarantees are only half in Rust:
 *
 * - a rebuild and an orphan release are separate buttons with separate confirmations
 * - the confirmation says, in words, that nothing will be deleted
 * - the orphan list is shown before it is agreed to, not merely counted
 * - a `reviewRequired` rejection re-reads the plan and says so, rather than reporting an
 *   error the user cannot act on
 * - the fingerprint the screen confirms with is the one the preview handed it
 *
 * `invoke` is mocked; there is no Tauri runtime under Vitest.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { GridPreviewSection } = await import("./GridPreviewSection");

const SCHOOL = {
  id: "school-1",
  name: "Northgate Secondary",
  timezone: "Europe/London",
  locale: "en-GB",
  createdAt: "2027-01-01T00:00:00.000Z",
  updatedAt: "2027-01-01T00:00:00.000Z",
  rev: 1,
};

const CYCLE = {
  id: "cycle-1",
  schoolId: "school-1",
  name: "Six-day rotation",
  dayCount: 2,
  weekCount: 1,
  isDefault: true,
  createdAt: "2027-01-01T00:00:00.000Z",
  updatedAt: "2027-01-01T00:00:00.000Z",
  rev: 1,
};

const STRUCTURE = {
  id: "bells-1",
  schoolId: "school-1",
  campusId: null,
  name: "Standard bells",
  isDefault: true,
  createdAt: "2027-01-01T00:00:00.000Z",
  updatedAt: "2027-01-01T00:00:00.000Z",
  rev: 1,
};

function day(id: string, ordinal: number, label: string) {
  return {
    id,
    cycleId: "cycle-1",
    ordinal,
    label,
    weekdayHint: null,
    createdAt: "2027-01-01T00:00:00.000Z",
    updatedAt: "2027-01-01T00:00:00.000Z",
    rev: 1,
  };
}

function period(id: string, ordinal: number, label: string) {
  return {
    id,
    periodStructureId: "bells-1",
    ordinal,
    label,
    startsAt: `0${8 + ordinal}:00`,
    endsAt: `0${8 + ordinal}:50`,
    kind: "TEACHING" as const,
    countsAsLoad: true,
    createdAt: "2027-01-01T00:00:00.000Z",
    updatedAt: "2027-01-01T00:00:00.000Z",
    rev: 1,
  };
}

/** A two-by-one grid, so the assertions stay about behaviour rather than arithmetic. */
function preview(overrides: Record<string, unknown> = {}) {
  return {
    cycleId: "cycle-1",
    periodStructureId: "bells-1",
    termId: null,
    days: [day("day-1", 1, "Day A"), day("day-2", 2, "Day B")],
    periods: [period("period-1", 1, "P1")],
    cells: [
      {
        cycleDayId: "day-1",
        periodId: "period-1",
        timeslotId: null,
        ordinal: null,
        isTeaching: true,
      },
      {
        cycleDayId: "day-2",
        periodId: "period-1",
        timeslotId: null,
        ordinal: null,
        isTeaching: true,
      },
    ],
    coverage: {
      declaredDayCount: 2,
      present: 2,
      missingOrdinals: [],
      beyondDeclared: [],
    },
    counts: { preserved: 0, created: 2, orphaned: 0, refreshed: 0 },
    orphaned: [],
    fingerprint: "aaaaaaaaaaaaaaaa",
    ...overrides,
  };
}

/**
 * The banner a matched fragment sits inside.
 *
 * `findByText` returns the innermost element containing the text, which for these
 * banners is the `<strong>` lead-in. Assertions about the rest of the sentence have to
 * read the whole banner.
 */
function bannerAround(element: HTMLElement): HTMLElement {
  const banner = element.closest(".banner");
  if (!banner) throw new Error("expected the match to be inside a banner");
  return banner as HTMLElement;
}

/** Routes each mocked command to a canned answer. */
function route(handlers: Record<string, unknown>) {
  invoke.mockImplementation((command: string) => {
    const handler = handlers[command];
    if (typeof handler === "function") return Promise.resolve(handler());
    if (handler === undefined) return Promise.resolve([]);
    return Promise.resolve(handler);
  });
}

const BASE = {
  setup_cycle_list: [CYCLE],
  setup_period_structure_list: [STRUCTURE],
  setup_academic_year_list: [],
  setup_grid_preview: () => preview(),
};

beforeEach(() => {
  invoke.mockReset();
});

describe("the grid preview", () => {
  it("shows the axes, and marks every cell that has no slot", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    expect(await screen.findByText("Day A")).toBeTruthy();
    expect(screen.getByText("Day B")).toBeTruthy();
    expect(screen.getByText("P1")).toBeTruthy();
    expect(screen.getAllByText("missing")).toHaveLength(2);
    expect(screen.getByText(/Slots missing/i)).toBeTruthy();
  });

  it("says the grid is not finished when cells are missing", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    const status = bannerAround(await screen.findByText(/This grid is not finished/i));
    expect(status.textContent).toMatch(/2 of 2 cells have no slot/);
  });

  it("identifies an incomplete cycle behind an otherwise-built grid", async () => {
    route({
      ...BASE,
      setup_grid_preview: () =>
        preview({
          cells: [
            {
              cycleDayId: "day-1",
              periodId: "period-1",
              timeslotId: "slot-1",
              ordinal: 1,
              isTeaching: true,
            },
            {
              cycleDayId: "day-2",
              periodId: "period-1",
              timeslotId: "slot-2",
              ordinal: 2,
              isTeaching: true,
            },
          ],
          coverage: {
            declaredDayCount: 6,
            present: 2,
            missingOrdinals: [3, 4, 5, 6],
            beyondDeclared: [],
          },
          counts: { preserved: 2, created: 0, orphaned: 0, refreshed: 0 },
        }),
    });
    render(<GridPreviewSection school={SCHOOL} />);

    const status = bannerAround(await screen.findByText(/This grid is not finished/i));
    expect(status.textContent).toMatch(/cycle itself is incomplete/);
    expect(status.textContent).toMatch(/3, 4, 5, 6/);
  });

  it("tells the user what to add first when there is no cycle or schedule", async () => {
    route({ ...BASE, setup_cycle_list: [], setup_period_structure_list: [] });
    render(<GridPreviewSection school={SCHOOL} />);

    const empty = await screen.findByText(/Not enough defined yet/i);
    expect(empty).toBeTruthy();
    expect(screen.getByText(/Add a cycle in Cycles and Cycle Days/)).toBeTruthy();
  });
});

describe("rebuilding", () => {
  it("confirms first, stating plainly that nothing will be deleted", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(await screen.findByRole("button", { name: /Rebuild Timetable Grid/i }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Delete nothing/i)).toBeTruthy();
    expect(within(dialog).getByText(/Create 2/i)).toBeTruthy();
    // Nothing may have been sent before the confirmation.
    expect(invoke).not.toHaveBeenCalledWith("setup_grid_rebuild", expect.anything());
  });

  it("sends the fingerprint the preview issued, not one of its own", async () => {
    route({
      ...BASE,
      setup_grid_preview: () => preview({ fingerprint: "deadbeefdeadbeef" }),
      setup_grid_rebuild: () => ({
        preserved: [],
        created: [{}, {}],
        orphaned: [],
        refreshed: 0,
      }),
    });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(await screen.findByRole("button", { name: /Rebuild Timetable Grid/i }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Rebuild grid$/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "setup_grid_rebuild",
        expect.objectContaining({ reviewed: "deadbeefdeadbeef" }),
      );
    });
  });

  it("offers no rebuild when there is nothing to create", async () => {
    route({
      ...BASE,
      setup_grid_preview: () =>
        preview({
          cells: [
            {
              cycleDayId: "day-1",
              periodId: "period-1",
              timeslotId: "slot-1",
              ordinal: 1,
              isTeaching: true,
            },
            {
              cycleDayId: "day-2",
              periodId: "period-1",
              timeslotId: "slot-2",
              ordinal: 2,
              isTeaching: true,
            },
          ],
          counts: { preserved: 2, created: 0, orphaned: 0, refreshed: 0 },
        }),
    });
    render(<GridPreviewSection school={SCHOOL} />);

    const button = await screen.findByRole("button", { name: /Rebuild Timetable Grid/i });
    expect(button).toHaveProperty("disabled", true);
    expect(screen.getByText(/Nothing to create/i)).toBeTruthy();
  });

  it("re-reads the plan and says so when the confirmation went stale", async () => {
    let previews = 0;
    route({
      ...BASE,
      setup_grid_preview: () => {
        previews += 1;
        return preview();
      },
      setup_grid_rebuild: () => {
        throw { kind: "reviewRequired", message: "the grid changed" };
      },
    });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(await screen.findByRole("button", { name: /Rebuild Timetable Grid/i }));
    const dialog = await screen.findByRole("dialog");
    const before = previews;
    await userEvent.click(within(dialog).getByRole("button", { name: /^Rebuild grid$/i }));

    const notice = bannerAround(await screen.findByText(/nothing was applied/i));
    expect(notice.textContent).toMatch(/review them and confirm again/i);
    await waitFor(() => expect(previews).toBeGreaterThan(before));
  });

  it("shows an ordinary failure as an error rather than a stale-plan notice", async () => {
    route({
      ...BASE,
      setup_grid_rebuild: () => {
        throw { kind: "failed", message: "The project could not be updated." };
      },
    });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(await screen.findByRole("button", { name: /Rebuild Timetable Grid/i }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Rebuild grid$/i }));

    expect(await screen.findByText(/The project could not be updated/)).toBeTruthy();
    expect(screen.queryByText(/review them and confirm again/i)).toBeNull();
  });
});

describe("stranded slots", () => {
  const WITH_ORPHANS = {
    ...BASE,
    setup_grid_preview: () =>
      preview({
        counts: { preserved: 1, created: 0, orphaned: 1, refreshed: 0 },
        orphaned: [
          {
            id: "slot-9",
            cycleDayId: "day-2",
            periodId: "period-1",
            termId: null,
            ordinal: 2,
          },
        ],
      }),
  };

  it("warns that a rebuild will not remove them", async () => {
    route(WITH_ORPHANS);
    render(<GridPreviewSection school={SCHOOL} />);

    const warning = bannerAround(await screen.findByText(/would be stranded by this proposal/i));
    expect(warning.textContent).toMatch(/it never deletes/i);
    expect(warning.textContent).toMatch(/separate, deliberate action/i);
  });

  it("lists which slots they are, not just how many", async () => {
    route(WITH_ORPHANS);
    render(<GridPreviewSection school={SCHOOL} />);

    expect(await screen.findByText("slot-9")).toBeTruthy();
    expect(screen.getByText(/Day B \(position 2\) · P1/)).toBeTruthy();
  });

  it("requires its own confirmation, separate from the rebuild", async () => {
    route(WITH_ORPHANS);
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(
      await screen.findByRole("button", { name: /Delete 1 stranded slot…/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/cannot be undone/i)).toBeTruthy();
    expect(within(dialog).getByText("slot-9")).toBeTruthy();
    expect(invoke).not.toHaveBeenCalledWith(
      "setup_grid_release_orphans",
      expect.anything(),
    );
  });

  it("deletes nothing if the confirmation is cancelled", async () => {
    route(WITH_ORPHANS);
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(
      await screen.findByRole("button", { name: /Delete 1 stranded slot…/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Cancel$/i }));

    expect(invoke).not.toHaveBeenCalledWith(
      "setup_grid_release_orphans",
      expect.anything(),
    );
  });

  it("releases with the reviewed fingerprint once confirmed", async () => {
    route({ ...WITH_ORPHANS, setup_grid_release_orphans: () => ["slot-9"] });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(
      await screen.findByRole("button", { name: /Delete 1 stranded slot…/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Delete slots$/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "setup_grid_release_orphans",
        expect.objectContaining({ reviewed: "aaaaaaaaaaaaaaaa" }),
      );
    });
    expect(await screen.findByText(/1 stranded slot deleted/i)).toBeTruthy();
  });

  it("re-reads rather than deleting when a release confirmation went stale", async () => {
    route({
      ...WITH_ORPHANS,
      setup_grid_release_orphans: () => {
        throw { kind: "reviewRequired", message: "the grid changed" };
      },
    });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(
      await screen.findByRole("button", { name: /Delete 1 stranded slot…/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Delete slots$/i }));

    expect(await screen.findByText(/nothing was applied/i)).toBeTruthy();
  });

  it("explains a refusal when something still refers to a slot", async () => {
    route({
      ...WITH_ORPHANS,
      setup_grid_release_orphans: () => {
        throw {
          kind: "stillReferenced",
          entity: "timeslot",
          message:
            "This timeslot cannot be deleted while other records still depend on it. " +
            "Remove or reassign those records first, then try again.",
        };
      },
    });
    render(<GridPreviewSection school={SCHOOL} />);

    await userEvent.click(
      await screen.findByRole("button", { name: /Delete 1 stranded slot…/i }),
    );
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Delete slots$/i }));

    const error = await screen.findByText(/Remove or reassign those records first/);
    expect(error).toBeTruthy();
  });

  it("offers no release panel when nothing would be stranded", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    await screen.findByText("Day A");
    expect(screen.queryByText(/Stranded slots/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /Delete.*stranded/i })).toBeNull();
  });
});

describe("previewing a removal", () => {
  it("asks the backend what the grid would be without an excluded day", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    await screen.findByText("Day A");
    const row = screen.getByText("Day A").closest("th");
    await userEvent.click(within(row as HTMLElement).getByRole("button", { name: /Plan without/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "setup_grid_preview",
        expect.objectContaining({
          request: expect.objectContaining({ withoutCycleDays: ["day-1"] }),
        }),
      );
    });
  });

  it("says plainly that an excluded preview is a proposal, not the current state", async () => {
    route(BASE);
    render(<GridPreviewSection school={SCHOOL} />);

    await screen.findByText("Day A");
    const row = screen.getByText("Day A").closest("th");
    await userEvent.click(within(row as HTMLElement).getByRole("button", { name: /Plan without/i }));

    const notice = bannerAround(await screen.findByText(/You are previewing a/i));
    expect(notice.textContent).toMatch(/Nothing has been changed/i);
  });
});
