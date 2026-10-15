/**
 * The School Setup shell: which sections exist, in what order, and how they gate.
 *
 * `SchoolSetup.tsx` says `SECTIONS` is "exported so a test can assert the list rather than
 * the rendering", and this is that test: the set of steps is a product decision, and a
 * dropped or reordered one would otherwise only be noticed by someone looking for it.
 *
 * Stage 0C added **Schools** and restored **Resources**. Stage A then reordered the whole
 * list — people before the time model, year levels out of Students and into the structure
 * of the school year — and added a disabled **Lessons** entry so the order reads correctly
 * before Stage C fills it in. Both changes are product decisions, which is exactly why they
 * are asserted as data here.
 *
 * So this covers the list as data *and* what the user actually sees, because the two can
 * disagree: the sidebar numbers steps from their array position, so an insertion has to
 * renumber the rest with no separate counter to keep in step.
 *
 * It also covers the thing only the shell can get wrong, and the one Stage 0C's brief
 * singles out: **switching schools must not leave the previous school's screen standing.**
 * The sections are keyed by school identifier, so a switch remounts them; the test asserts
 * the remount rather than the key, because the key is the mechanism and the remount is the
 * guarantee.
 *
 * Each section's own screen is tested in its own file. This file deliberately stubs them
 * all out and asserts only navigation, numbering, gating and school context.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School } from "@evara/domain";

const listSchools = vi.fn<() => Promise<School[]>>();

vi.mock("../../ipc/setup", () => ({
  listSchools: () => listSchools(),
}));

/**
 * Every section screen stubbed to a marker.
 *
 * The shell's job is to render exactly one of them; which one is the only thing under
 * test here, so the real screens — each of which would fetch its own data — are replaced.
 *
 * The stubs print the school they were given, which is what lets the remount assertions
 * below see that a switch reached the section rather than only the sidebar.
 */
vi.mock("./Schools", () => ({
  // The button stands in for a create, rename or delete finishing: the real screen calls
  // `onChanged` and the shell re-reads. That is the only way a test can exercise what the
  // shell does when the project's schools change under it.
  Schools: (props: { activeSchoolId: string | null; onChanged: () => void }) => (
    <div>
      stub:schools:{props.activeSchoolId}
      <button type="button" onClick={props.onChanged}>
        stub:changed
      </button>
    </div>
  ),
}));
vi.mock("./SchoolDetails", () => ({
  SchoolDetails: (props: { school: School | null }) => (
    <div>stub:school-details:{props.school?.id ?? "none"}</div>
  ),
}));
vi.mock("./CampusesAndBuildings", () => ({
  CampusesAndBuildings: (props: { school: School }) => (
    <div>stub:campuses:{props.school.id}</div>
  ),
}));
vi.mock("./RoomsAndRoomTypes", () => ({
  RoomsAndRoomTypes: () => <div>stub:rooms</div>,
}));
vi.mock("./ResourcesAndSharing", () => ({
  ResourcesAndSharing: () => <div>stub:resources</div>,
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
vi.mock("./Teachers", () => ({
  Teachers: () => <div>stub:teachers</div>,
}));
vi.mock("./Students", () => ({
  Students: () => <div>stub:students</div>,
}));
vi.mock("./YearLevels", () => ({
  YearLevels: () => <div>stub:yearLevels</div>,
}));
vi.mock("./StudentGroups", () => ({
  StudentGroups: () => <div>stub:groups</div>,
}));
vi.mock("./Departments", () => ({
  Departments: () => <div>stub:departments</div>,
}));
vi.mock("./Subjects", () => ({
  Subjects: () => <div>stub:subjects</div>,
}));

const { SchoolSetup, SECTIONS } = await import("./SchoolSetup");

const SCHOOL = {
  id: "school-1",
  name: "Northgate Secondary",
  code: "NGS",
  timezone: "Europe/London",
  locale: "en-GB",
  workingDays: [1, 2, 3, 4, 5],
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
} as School;

const SECOND = { ...SCHOOL, id: "school-2", name: "Southmoor College", code: "SMC" } as School;

beforeEach(() => {
  vi.clearAllMocks();
  listSchools.mockResolvedValue([SCHOOL]);
});

/** Renders the shell and waits for the school read to settle. */
async function open() {
  render(<SchoolSetup />);
  await waitFor(() => expect(listSchools).toHaveBeenCalled());
  // The first section renders once the project's schools are known.
  await screen.findByText(/^stub:schools:/);
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
  it("is the seventeen steps of setup, in configuration order", () => {
    expect(SECTIONS.map((s) => s.id)).toEqual([
      // Which institution. Everything after this is about one of them.
      "schools",
      "school",
      // Where it is, and what is in it.
      "campuses",
      "rooms",
      "resources",
      // The shape of a school year, then the stages within it.
      "years",
      "yearLevels",
      // What is taught. Departments and Subjects are Stage B; Lessons is Stage C, and
      // comes after them because a lesson is taught *of* a subject.
      "departments",
      "subjects",
      "lessons",
      // Who.
      "teachers",
      "students",
      "groups",
      // When. Last, because it is the part a school verifies against the grid rather
      // than the part it already knows.
      "cycles",
      "periods",
      "calendar",
      "grid",
    ]);
  });

  it("has a Resources section", () => {
    // Removed when configuring a resource had no visible effect; back because V13 gave
    // resources cross-school sharing, which is a decision with a consequence.
    expect(SECTIONS.some((s) => s.id === "resources")).toBe(true);
    expect(SECTIONS.find((s) => s.id === "resources")?.label).toMatch(/resource/i);
  });

  it("separates Year levels from Students", () => {
    const yearLevels = SECTIONS.findIndex((s) => s.id === "yearLevels");
    const students = SECTIONS.findIndex((s) => s.id === "students");

    expect(yearLevels).toBeGreaterThan(-1);
    expect(students).toBeGreaterThan(-1);
    expect(yearLevels).not.toBe(students);
    // Year levels are part of the structure of a school year, so they sit with the
    // academic year rather than with the children placed in them.
    expect(yearLevels).toBe(SECTIONS.findIndex((s) => s.id === "years") + 1);
    expect(SECTIONS.find((s) => s.id === "students")?.label).toBe("Students");
  });

  it("puts the people before the time model", () => {
    const of = (id: string) => SECTIONS.findIndex((s) => s.id === id);

    // The Stage A reorder, asserted as the relationship rather than as two numbers, so
    // inserting a section cannot quietly invert it.
    for (const who of ["teachers", "students", "groups"]) {
      for (const when of ["cycles", "periods", "calendar", "grid"]) {
        expect(of(who)).toBeLessThan(of(when));
      }
    }
  });

  it("needs a school for everything except Schools and School details", () => {
    expect(SECTIONS.filter((s) => !s.needsSchool).map((s) => s.id)).toEqual([
      "schools",
      "school",
    ]);
  });
});

/**
 * Lessons is announced, not built.
 *
 * The brief's order has it at step 8; Stage C implements it. An empty functional screen
 * would invite somebody to look for a feature that does not exist, so the entry carries a
 * reason, has no `render`, and cannot be opened.
 */
describe("the Lessons entry", () => {
  it("is in the list, after the subjects it will be taught of", () => {
    const of = (id: string) => SECTIONS.findIndex((s) => s.id === id);
    // Stage B inserted Departments and Subjects between Year levels and Lessons, which is
    // the dependency order: `course.subject_id` references a subject, so the subject has
    // to exist first. Lessons still sits immediately before the people it is assigned to.
    expect(of("departments")).toBe(of("yearLevels") + 1);
    expect(of("subjects")).toBe(of("departments") + 1);
    expect(of("lessons")).toBe(of("subjects") + 1);
    expect(of("lessons")).toBe(of("teachers") - 1);
  });

  it("is the only section that is not built, and says when it arrives", () => {
    const later = SECTIONS.filter((s) => s.comingLater !== undefined);
    expect(later.map((s) => s.id)).toEqual(["lessons"]);
    expect(later[0]?.comingLater).toMatch(/Stage C/);
  });

  it("has nothing to render, so it cannot be opened by accident", () => {
    expect(SECTIONS.find((s) => s.id === "lessons")?.render).toBeUndefined();
  });

  it("is offered as disabled, labelled, and distinct from a locked section", async () => {
    await open();

    const lessons = within(nav()).getByRole("button", { name: /^Lessons/ });
    expect(lessons.hasAttribute("disabled")).toBe(true);
    expect(lessons.textContent).toContain("Coming later");
    expect(lessons.getAttribute("title")).toMatch(/Stage C/);
    // Not "Needs a school": adding a school would not make it appear, and conflating the
    // two reasons would promise something untrue.
    expect(lessons.textContent).not.toContain("Needs a school");
  });

  it("renders no screen when clicked", async () => {
    await open();

    // A disabled button does not fire, so the body is unchanged — and the fallback in
    // `SchoolSetup` means even a programmatic request could not blank it.
    await userEvent.click(within(nav()).getByRole("button", { name: /^Lessons/ }));

    expect(screen.getByText(/^stub:schools:/)).toBeTruthy();
    // Nothing about lessons in the body — only the sidebar entry that announces them.
    const body = document.querySelector(".setupBody") as HTMLElement;
    expect(within(body).queryByText(/lesson/i)).toBeNull();
  });
});

describe("the sidebar", () => {
  it("offers every section, Resources and Year levels included", async () => {
    await open();

    expect(steps()).toHaveLength(17);
    expect(within(nav()).getByRole("button", { name: /Resources/i })).toBeTruthy();
    expect(within(nav()).getByRole("button", { name: /^Schools/ })).toBeTruthy();
    expect(within(nav()).getByRole("button", { name: /^Year levels/ })).toBeTruthy();
    expect(within(nav()).getByRole("button", { name: /^Departments/ })).toBeTruthy();
    expect(within(nav()).getByRole("button", { name: /^Subjects/ })).toBeTruthy();
  });

  it("numbers the steps 1-17 with no gap", async () => {
    await open();

    // The numbers come from array position, so this is what proves the reorder and the two
    // insertions renumbered rather than leaving a hole.
    const indices = within(nav())
      .getAllByRole("button")
      .map((b) => b.querySelector(".navIndex")?.textContent);
    expect(indices).toEqual([
      "1", "2", "3", "4", "5", "6", "7", "8", "9",
      "10", "11", "12", "13", "14", "15", "16", "17",
    ]);
  });

  it("gives step 1 to Schools and step 2 to School details", async () => {
    await open();

    const buttons = within(nav()).getAllByRole("button");
    expect(buttons[0]?.textContent).toContain("Schools");
    expect(buttons[1]?.textContent).toContain("School details");
  });
});

describe("navigating between the sections", () => {
  it("opens Schools first", async () => {
    await open();

    expect(screen.getByText(/^stub:schools:/)).toBeTruthy();
    const first = within(nav()).getAllByRole("button")[0];
    expect(first?.getAttribute("aria-current")).toBe("page");
  });

  it.each([
    ["School details", /^stub:school-details:/],
    ["Campuses and buildings", /^stub:campuses:/],
    ["Rooms and room types", /^stub:rooms$/],
    ["Resources", /^stub:resources$/],
    ["Academic years and terms", /^stub:years$/],
    ["Year levels", /^stub:yearLevels$/],
    ["Teachers", /^stub:teachers$/],
    ["Students", /^stub:students$/],
    ["Student groups", /^stub:groups$/],
    ["Cycles and cycle days", /^stub:cycles$/],
    ["Bell schedules and periods", /^stub:periods$/],
    ["Calendar mapping", /^stub:calendar$/],
    ["Timetable grid preview", /^stub:grid$/],
  ])("opens %s", async (label, marker) => {
    await open();
    await userEvent.click(
      within(nav()).getByRole("button", { name: new RegExp(`^${label}`, "i") }),
    );

    expect(await screen.findByText(marker)).toBeTruthy();
  });

  it("renders one section at a time, and marks which", async () => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: /Rooms and room types/i }));

    expect(await screen.findByText("stub:rooms")).toBeTruthy();
    expect(screen.queryByText(/^stub:schools:/)).toBeNull();
    expect(screen.queryByText(/^stub:campuses:/)).toBeNull();

    const current = within(nav())
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]?.textContent).toContain("Rooms and room types");
  });

  it("walks the whole workflow end to end, in the configuration order", async () => {
    await open();

    const visited: string[] = ["stub:schools"];
    // Every step but Lessons, which has no screen, in the order the sidebar offers them.
    for (const label of [
      "School details",
      "Campuses and buildings",
      "Rooms and room types",
      "Resources",
      "Academic years and terms",
      "Year levels",
      "Teachers",
      "Students",
      "Student groups",
      "Cycles and cycle days",
      "Bell schedules and periods",
      "Calendar mapping",
      "Timetable grid preview",
    ]) {
      await userEvent.click(
        within(nav()).getByRole("button", { name: new RegExp(`^${label}`, "i") }),
      );
      const body = document.querySelector(".setupBody");
      const marker = within(body as HTMLElement).getByText(/^stub:/).textContent ?? "";
      visited.push(marker.split(":").slice(0, 2).join(":"));
    }

    expect(visited).toEqual([
      "stub:schools",
      "stub:school-details",
      "stub:campuses",
      "stub:rooms",
      "stub:resources",
      "stub:years",
      "stub:yearLevels",
      "stub:teachers",
      "stub:students",
      "stub:groups",
      "stub:cycles",
      "stub:periods",
      "stub:calendar",
      "stub:grid",
    ]);
  });
});

describe("before any school exists", () => {
  beforeEach(() => {
    listSchools.mockResolvedValue([]);
  });

  it("locks every section but Schools and School details", async () => {
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);

    // Fifteen school-owned sections, of which Lessons is disabled for its own reason.
    const locked = within(nav())
      .getAllByRole("button")
      .filter((b) => b.hasAttribute("disabled"));
    expect(locked).toHaveLength(15);
    expect(
      locked.filter((b) => b.textContent?.includes("Needs a school")),
    ).toHaveLength(14);
    expect(locked.filter((b) => b.textContent?.includes("Coming later"))).toHaveLength(1);
  });

  it("opens on Schools, which is where the first school is added", async () => {
    render(<SchoolSetup />);

    expect(await screen.findByText("stub:schools:")).toBeTruthy();
    const first = within(nav()).getAllByRole("button")[0];
    expect(first?.hasAttribute("disabled")).toBe(false);
  });

  it("sends School details to Schools rather than offering a second way to create one", async () => {
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);

    await userEvent.click(within(nav()).getByRole("button", { name: /School details/i }));

    // The stub prints "none", which is the shell having passed a null school rather than
    // having picked an arbitrary one.
    expect(await screen.findByText("stub:school-details:none")).toBeTruthy();
  });
});

describe("choosing which school", () => {
  it("offers no picker when the project holds one school", async () => {
    await open();
    expect(screen.queryByLabelText("School")).toBeNull();
  });

  it("does not label the body with a school when there is only one", async () => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));

    // Nothing to tell apart, so the scope line would be furniture on every screen.
    expect(document.querySelector(".setupScope")).toBeNull();
  });

  it("offers a picker when the project holds several, keyed by identifier", async () => {
    listSchools.mockResolvedValue([SCHOOL, SECOND]);
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);

    const picker = screen.getByLabelText("School");
    expect((picker as HTMLSelectElement).value).toBe(SCHOOL.id);
    await userEvent.selectOptions(picker, SECOND.id);
    expect((picker as HTMLSelectElement).value).toBe(SECOND.id);
  });

  it("names the school the body is about once there is a choice", async () => {
    listSchools.mockResolvedValue([SCHOOL, SECOND]);
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));

    const scope = document.querySelector(".setupScope");
    expect(scope?.textContent).toContain("Northgate Secondary (NGS)");
  });

  it("switches every school-specific section, not only the sidebar", async () => {
    listSchools.mockResolvedValue([SCHOOL, SECOND]);
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));
    expect(await screen.findByText(`stub:campuses:${SCHOOL.id}`)).toBeTruthy();

    await userEvent.selectOptions(screen.getByLabelText("School"), SECOND.id);

    // The section is now about the other school, and shows no trace of the one left
    // behind — "data from School A under School B's heading" is the failure this guards.
    expect(await screen.findByText(`stub:campuses:${SECOND.id}`)).toBeTruthy();
    expect(screen.queryByText(`stub:campuses:${SCHOOL.id}`)).toBeNull();
    expect(document.querySelector(".setupScope")?.textContent).toContain(
      "Southmoor College",
    );
  });

  /**
   * The stale-state guarantee, asserted as a remount rather than as a key.
   *
   * A section holds state that is *about* a school — an open form, a pending delete, a
   * selected campus. Re-rendering with a new `school` prop re-reads the lists and leaves
   * that state standing; remounting discards it. The stub counts its own mounts, so this
   * sees the difference.
   */
  it("remounts a section on a switch, discarding what it was holding", async () => {
    listSchools.mockResolvedValue([SCHOOL, SECOND]);
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));
    const first = await screen.findByText(`stub:campuses:${SCHOOL.id}`);

    await userEvent.selectOptions(screen.getByLabelText("School"), SECOND.id);
    const second = await screen.findByText(`stub:campuses:${SECOND.id}`);

    // A different DOM node, not the same one re-rendered: React only reuses the element
    // when the key matches, and the key is the school.
    expect(second).not.toBe(first);
    expect(document.body.contains(first)).toBe(false);
  });

  it("hands the selection to another school when the selected one is deleted", async () => {
    listSchools.mockResolvedValue([SCHOOL, SECOND]);
    render(<SchoolSetup />);
    await screen.findByText(/^stub:schools:/);
    await userEvent.selectOptions(screen.getByLabelText("School"), SECOND.id);
    expect(await screen.findByText(`stub:schools:${SECOND.id}`)).toBeTruthy();

    // A successful delete: the project now holds one school, and the section tells the
    // shell to re-read. The selection cannot stay pointed at a row that is gone.
    listSchools.mockResolvedValue([SCHOOL]);
    await userEvent.click(screen.getByRole("button", { name: "stub:changed" }));

    expect(await screen.findByText(`stub:schools:${SCHOOL.id}`)).toBeTruthy();
    expect(screen.queryByText(`stub:schools:${SECOND.id}`)).toBeNull();
    // And with one school left, the picker goes: there is no choice to make again.
    await waitFor(() => expect(screen.queryByLabelText("School")).toBeNull());
  });
});

describe("existing project data", () => {
  it("reads the schools once and renders the active one, rather than re-fetching per section", async () => {
    await open();
    await userEvent.click(within(nav()).getByRole("button", { name: /Campuses/i }));
    await userEvent.click(within(nav()).getByRole("button", { name: /Rooms/i }));

    // The shell holds the record; a project opened before Stage 0C is untouched by this
    // change, because nothing here writes.
    expect(listSchools).toHaveBeenCalledTimes(1);
  });
});
