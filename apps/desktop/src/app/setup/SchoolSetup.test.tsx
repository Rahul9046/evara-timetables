/**
 * The School Setup shell: which sections exist, in what order, and how they gate.
 *
 * `SchoolSetup.tsx` has always said `SECTIONS` was "exported so a test can assert the
 * list rather than the rendering" — there was no such test. Removing the Resources
 * section is the case that comment was describing: the set of steps is a product
 * decision, and a dropped or reordered one would otherwise only be noticed by someone
 * looking for it.
 *
 * So this covers the list as data *and* what the user actually sees, because the two can
 * disagree: the sidebar numbers steps from their array position, so a removal has to
 * renumber the rest with no separate counter to keep in step.
 *
 * Each section's own screen is tested in its own file. This file deliberately stubs them
 * all out and asserts only navigation, numbering and gating.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School } from "@evara/domain";

const getSchool = vi.fn<() => Promise<School | null>>();

vi.mock("../../ipc/setup", () => ({
  getSchool: () => getSchool(),
}));

/**
 * Every section screen stubbed to a marker.
 *
 * The shell's job is to render exactly one of them; which one is the only thing under
 * test here, so the real screens — each of which would fetch its own data — are replaced.
 */
vi.mock("./SchoolDetails", () => ({
  SchoolDetails: () => <div>stub:school-details</div>,
}));
vi.mock("./CampusesAndBuildings", () => ({
  CampusesAndBuildings: () => <div>stub:campuses</div>,
}));
vi.mock("./RoomsAndRoomTypes", () => ({
  RoomsAndRoomTypes: () => <div>stub:rooms</div>,
}));
vi.mock("./AcademicYearsAndTerms", () => ({
  AcademicYearsAndTerms: () => <div>stub:years</div>,
}));
vi.mock("./CyclesAndCycleDays", () => ({
  CyclesAndCycleDays: () => <div>stub:cycles</div>,
}));
vi.mock("./PeriodStructuresAndPeriods", () => ({
  PeriodStructuresAndPeriods: () => <div>stub:periods</div>,
}));
vi.mock("./CalendarMapping", () => ({
  CalendarMapping: () => <div>stub:calendar</div>,
}));
vi.mock("./GridPreviewSection", () => ({
  GridPreviewSection: () => <div>stub:grid</div>,
}));

const { SchoolSetup, SECTIONS } = await import("./SchoolSetup");

const SCHOOL = {
  id: "school-1",
  name: "Northgate Secondary",
  timezone: "Europe/London",
  locale: "en-GB",
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
} as School;

beforeEach(() => {
  vi.clearAllMocks();
  getSchool.mockResolvedValue(SCHOOL);
});

/** Renders the shell and waits for the school read to settle. */
async function open() {
  render(<SchoolSetup />);
  await waitFor(() => expect(getSchool).toHaveBeenCalled());
  // The first section renders once the school is known.
  await screen.findByText("stub:school-details");
}

function nav(): HTMLElement {
  return screen.getByRole("navigation", { name: /School setup sections/i });
}

/** The sidebar's steps, in order, as the user reads them. */
function steps(): string[] {
  return within(nav())
    .getAllByRole("button")
    .map((b) => b.textContent ?? "");
}

describe("the section list", () => {
  it("is the eight steps of setup, in dependency order", () => {
    expect(SECTIONS.map((s) => s.id)).toEqual([
      "school",
      "campuses",
      "rooms",
      "years",
      "cycles",
      "periods",
      "calendar",
      "grid",
    ]);
  });

  it("has no Resources section", () => {
    // Nothing schedules against a resource until constraints arrive, so the step was
    // configuration with no visible effect. The table and its commands remain.
    expect(SECTIONS.some((s) => s.id === "resources")).toBe(false);
    expect(SECTIONS.some((s) => /resource/i.test(s.label))).toBe(false);
  });

  it("needs a school for everything except School details", () => {
    expect(SECTIONS.filter((s) => !s.needsSchool).map((s) => s.id)).toEqual(["school"]);
  });
});

describe("the sidebar", () => {
  it("offers every section and no Resources entry", async () => {
    await open();

    expect(steps()).toHaveLength(8);
    expect(within(nav()).queryByRole("button", { name: /Resources/i })).toBeNull();
    expect(within(nav()).queryByText(/Shared equipment/i)).toBeNull();
  });

  it("numbers the steps 1-8 with no gap where Resources was", async () => {
    await open();

    // The numbers come from array position, so this is what proves the removal
    // renumbered rather than leaving a hole at 4.
    const indices = within(nav())
      .getAllByRole("button")
      .map((b) => b.querySelector(".navIndex")?.textContent);
    expect(indices).toEqual(["1", "2", "3", "4", "5", "6", "7", "8"]);
  });

  it("gives step 4 to academic years, which followed Resources", async () => {
    await open();

    const fourth = within(nav()).getAllByRole("button")[3];
    expect(fourth?.querySelector(".navIndex")?.textContent).toBe("4");
    expect(fourth?.textContent).toContain("Academic years and terms");
  });
});

describe("navigating between the remaining sections", () => {
  it("opens School details first", async () => {
    await open();

    expect(screen.getByText("stub:school-details")).toBeTruthy();
    const first = within(nav()).getAllByRole("button")[0];
    expect(first?.getAttribute("aria-current")).toBe("page");
  });

  it.each([
    ["Campuses and buildings", "stub:campuses"],
    ["Rooms and room types", "stub:rooms"],
    ["Academic years and terms", "stub:years"],
    ["Cycles and cycle days", "stub:cycles"],
    ["Bell schedules and periods", "stub:periods"],
    ["Calendar mapping", "stub:calendar"],
    ["Timetable grid preview", "stub:grid"],
  ])("opens %s", async (label, marker) => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: new RegExp(label, "i") }));

    expect(await screen.findByText(marker)).toBeTruthy();
  });

  it("renders one section at a time, and marks which", async () => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: /Rooms and room types/i }));

    expect(await screen.findByText("stub:rooms")).toBeTruthy();
    expect(screen.queryByText("stub:school-details")).toBeNull();
    expect(screen.queryByText("stub:campuses")).toBeNull();

    const current = within(nav())
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]?.textContent).toContain("Rooms and room types");
  });

  it("walks the whole workflow end to end without a Resources step", async () => {
    // "Setup can be completed without configuring resources": every step is reachable and
    // renders, and the walk never passes through a resources section.
    await open();

    const visited: string[] = ["stub:school-details"];
    for (const label of [
      "Campuses and buildings",
      "Rooms and room types",
      "Academic years and terms",
      "Cycles and cycle days",
      "Bell schedules and periods",
      "Calendar mapping",
      "Timetable grid preview",
    ]) {
      await userEvent.click(
        within(nav()).getByRole("button", { name: new RegExp(label, "i") }),
      );
      const body = document.querySelector(".setupBody");
      visited.push(within(body as HTMLElement).getByText(/^stub:/).textContent ?? "");
    }

    expect(visited).toEqual([
      "stub:school-details",
      "stub:campuses",
      "stub:rooms",
      "stub:years",
      "stub:cycles",
      "stub:periods",
      "stub:calendar",
      "stub:grid",
    ]);
    expect(visited.some((m) => /resource/i.test(m))).toBe(false);
  });
});

describe("before the school record exists", () => {
  beforeEach(() => {
    getSchool.mockResolvedValue(null);
  });

  it("locks every section but School details", async () => {
    render(<SchoolSetup />);
    await screen.findByText("stub:school-details");

    const locked = within(nav())
      .getAllByRole("button")
      .filter((b) => b.hasAttribute("disabled"));
    expect(locked).toHaveLength(7);
    expect(locked.every((b) => b.textContent?.includes("Needs the school record"))).toBe(
      true,
    );
  });

  it("still lets School details be used, so the first run has somewhere to start", async () => {
    render(<SchoolSetup />);

    expect(await screen.findByText("stub:school-details")).toBeTruthy();
    const first = within(nav()).getAllByRole("button")[0];
    expect(first?.hasAttribute("disabled")).toBe(false);
  });
});

describe("existing project data", () => {
  it("reads the school once and renders it, rather than re-fetching per section", async () => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));
    await userEvent.click(within(nav()).getByRole("button", { name: /Rooms/i }));

    // The shell holds the record; a project opened with resources already in it is
    // untouched by this change, because nothing here writes.
    expect(getSchool).toHaveBeenCalledTimes(1);
  });
});
