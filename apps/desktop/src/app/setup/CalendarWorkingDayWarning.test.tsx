/**
 * The working-day warning on Calendar Mapping.
 *
 * `tests_working_days.rs` owns the rule and proves the repository only reads. This file
 * owns the interface's half of the same promise:
 *
 * - a mapped teaching date on a non-working weekday is reported, by date and by weekday
 * - the warning says plainly that nothing was changed, and offers both ways out
 * - **no mapping is created, deleted or updated** by any of it — asserted by the write
 *   mocks never being called
 * - a six-day cycle on a five-day week produces no warning, because the two are unrelated
 * - the advisory is asked again when the school's week changes, not cached
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import type {
  AcademicYear,
  CalendarDay,
  Campus,
  Cycle,
  CycleDay,
  NonWorkingInstructionalDay,
  School,
  Term,
} from "@evara/domain";

const listAcademicYears = vi.fn<() => Promise<AcademicYear[]>>();
const listCampuses = vi.fn<() => Promise<Campus[]>>();
const listCycles = vi.fn<() => Promise<Cycle[]>>();
const listCycleDays = vi.fn<(cycleId: string) => Promise<CycleDay[]>>();
const listTerms = vi.fn<(yearId: string) => Promise<Term[]>>();
const listCalendarDays =
  vi.fn<(schoolId: string, from: string, to: string) => Promise<CalendarDay[]>>();
// Both carry the school they are about since V12: a calendar mapping and the findings
// over it belong to one school, and another school's week is read against its own.
const nonWorkingInstructionalDays =
  vi.fn<
    (schoolId: string, week: readonly number[]) => Promise<NonWorkingInstructionalDay[]>
  >();

// The three writes. Every test asserts these were never called: the working week must not
// mutate a calendar mapping, and a projection that did would be the subtler version of
// the same bug.
const createCalendarDay = vi.fn();
const updateCalendarDay = vi.fn();
const deleteCalendarDay = vi.fn();

vi.mock("../../ipc/setup", () => ({
  listAcademicYears: () => listAcademicYears(),
  listCampuses: () => listCampuses(),
  listCycles: () => listCycles(),
  listCycleDays: (cycleId: string) => listCycleDays(cycleId),
  listTerms: (yearId: string) => listTerms(yearId),
  listCalendarDays: (schoolId: string, from: string, to: string) =>
    listCalendarDays(schoolId, from, to),
  nonWorkingInstructionalDays: (schoolId: string, week: readonly number[]) =>
    nonWorkingInstructionalDays(schoolId, week),
  createCalendarDay: (input: unknown) => createCalendarDay(input),
  updateCalendarDay: (id: string, rev: number, input: unknown) =>
    updateCalendarDay(id, rev, input),
  deleteCalendarDay: (id: string) => deleteCalendarDay(id),
  // Stage B. Empty by default: a suite that is about something else should not have
  // to describe a curriculum, and one that is about this panel overrides them.
  listDepartments: vi.fn(() => Promise.resolve([])),
  listSubjects: vi.fn(() => Promise.resolve([])),
  listTeacherDepartments: vi.fn(() => Promise.resolve([])),
  listQualifications: vi.fn(() => Promise.resolve([])),
  listSubjectTeachers: vi.fn(() => Promise.resolve([])),
  listDepartmentTeachers: vi.fn(() => Promise.resolve([])),
  departmentDependents: vi.fn(() =>
    Promise.resolve({ total: 0, subjects: 0, teachers: 0 }),
  ),
  subjectDependents: vi.fn(() => Promise.resolve({ total: 0, qualifiedTeachers: 0 })),
  addQualification: vi.fn(() => Promise.resolve({})),
  removeQualification: vi.fn(() => Promise.resolve()),
  addTeacherDepartment: vi.fn(() => Promise.resolve({})),
  removeTeacherDepartment: vi.fn(() => Promise.resolve()),
  createDepartment: vi.fn(() => Promise.resolve({})),
  updateDepartment: vi.fn(() => Promise.resolve({})),
  deleteDepartment: vi.fn(() => Promise.resolve()),
  createSubject: vi.fn(() => Promise.resolve({})),
  updateSubject: vi.fn(() => Promise.resolve({})),
  deleteSubject: vi.fn(() => Promise.resolve()),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) => String(error),
  offendingField: () => null,
}));

const { CalendarMapping } = await import("./CalendarMapping");

function school(workingDays: number[]): School {
  return {
    id: "school-1",
    name: "Northgate Secondary",
    timezone: "Europe/London",
    locale: "en-GB",
    workingDays,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as School;
}

function finding(date: string, weekday: number): NonWorkingInstructionalDay {
  return { date, kind: "SCHOOL", weekday };
}

beforeEach(() => {
  vi.clearAllMocks();
  listAcademicYears.mockResolvedValue([
    {
      id: "year-1",
      schoolId: "school-1",
      name: "2027/28",
      startsOn: "2027-09-01",
      endsOn: "2028-07-20",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
      rev: 1,
    } as AcademicYear,
  ]);
  listCampuses.mockResolvedValue([]);
  listCycles.mockResolvedValue([]);
  listCycleDays.mockResolvedValue([]);
  listTerms.mockResolvedValue([]);
  listCalendarDays.mockResolvedValue([]);
  nonWorkingInstructionalDays.mockResolvedValue([]);
});

/** Nothing on this screen may write a calendar mapping as a result of the working week. */
function assertNothingWritten() {
  expect(createCalendarDay).not.toHaveBeenCalled();
  expect(updateCalendarDay).not.toHaveBeenCalled();
  expect(deleteCalendarDay).not.toHaveBeenCalled();
}

describe("when a mapped teaching date falls outside the working days", () => {
  it("names the date and the weekday it landed on", async () => {
    nonWorkingInstructionalDays.mockResolvedValue([finding("2027-09-04", 6)]);
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    const banner = await screen.findByText(/falls outside the school's working days/);
    expect(banner.textContent).toMatch(/04\/09\/2027/);
    expect(banner.textContent).toMatch(/Saturday/);
    expect(banner.textContent).toMatch(/Monday to Friday/);
    assertNothingWritten();
  });

  it("says plainly that nothing was changed, and offers both ways out", async () => {
    nonWorkingInstructionalDays.mockResolvedValue([finding("2027-09-04", 6)]);
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    const banner = await screen.findByText(/falls outside the school's working days/);
    expect(banner.textContent).toMatch(/Nothing has been changed/);
    expect(banner.textContent).toMatch(/Edit a date below/);
    expect(banner.textContent).toMatch(/change the working days in School Details/);
  });

  it("counts them when there is more than one", async () => {
    nonWorkingInstructionalDays.mockResolvedValue([
      finding("2027-09-04", 6),
      finding("2027-09-05", 7),
    ]);
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    const banner = await screen.findByText(/fall outside the school's working days/);
    expect(banner.textContent).toMatch(/^2 mapped teaching dates/);
    expect(banner.textContent).toMatch(/Saturday/);
    expect(banner.textContent).toMatch(/Sunday/);
    assertNothingWritten();
  });

  it("names a handful and summarises the rest rather than listing a year of them", async () => {
    nonWorkingInstructionalDays.mockResolvedValue(
      Array.from({ length: 12 }, (_, index) =>
        finding(`2027-09-${String(index + 1).padStart(2, "0")}`, 6),
      ),
    );
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    const banner = await screen.findByText(/fall outside the school's working days/);
    expect(banner.textContent).toMatch(/^12 mapped teaching dates/);
    expect(banner.textContent).toMatch(/and 4 more/);
    assertNothingWritten();
  });

  it("warns a Sunday-to-Thursday school about its Fridays", async () => {
    nonWorkingInstructionalDays.mockResolvedValue([finding("2027-09-03", 5)]);
    render(<CalendarMapping school={school([7, 1, 2, 3, 4])} />);

    const banner = await screen.findByText(/falls outside the school's working days/);
    expect(banner.textContent).toMatch(/Friday/);
    expect(banner.textContent).toMatch(/Monday, Tuesday, Wednesday, Thursday and Sunday/);
    assertNothingWritten();
  });
});

describe("when nothing is out of place", () => {
  it("shows no warning at all", async () => {
    nonWorkingInstructionalDays.mockResolvedValue([]);
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalled());
    expect(screen.queryByText(/outside the school's working days/)).toBeNull();
    assertNothingWritten();
  });

  it("stays silent for a six-day cycle on a five-day week", async () => {
    // The brief's case. The backend returns nothing because the two numbers are
    // unrelated; this asserts the screen does not invent a warning of its own from
    // `dayCount` either.
    listCycles.mockResolvedValue([
      {
        id: "cycle-1",
        schoolId: "school-1",
        name: "Six-day rotation",
        dayCount: 6,
        weekCount: 2,
        isDefault: true,
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-01T00:00:00.000Z",
        rev: 1,
      } as Cycle,
    ]);
    nonWorkingInstructionalDays.mockResolvedValue([]);
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalled());
    expect(screen.queryByText(/outside the school's working days/)).toBeNull();
    expect(screen.queryByText(/six/i)).toBeNull();
    assertNothingWritten();
  });

  it("says nothing when the advisory itself fails", async () => {
    // The screen's real job is the table. An advisory that could not be computed is
    // better silent than alarming.
    nonWorkingInstructionalDays.mockRejectedValue({ kind: "internal", message: "nope" });
    render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);

    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalled());
    expect(screen.queryByText(/outside the school's working days/)).toBeNull();
    expect(screen.queryByText(/nope/)).toBeNull();
    assertNothingWritten();
  });
});

describe("what the advisory is asked", () => {
  it("is asked with the school's own week, as ISO weekday numbers", async () => {
    render(<CalendarMapping school={school([7, 1, 2, 3, 4])} />);

    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalled());
    expect(nonWorkingInstructionalDays).toHaveBeenCalledWith(expect.any(String), [7, 1, 2, 3, 4]);
  });

  it("is asked again when the school's week changes", async () => {
    const { rerender } = render(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);
    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalledTimes(1));

    nonWorkingInstructionalDays.mockResolvedValue([finding("2027-09-03", 5)]);
    rerender(<CalendarMapping school={school([7, 1, 2, 3, 4])} />);

    // Not cached: a week edited in School Details must change what this says.
    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalledTimes(2));
    expect(nonWorkingInstructionalDays).toHaveBeenLastCalledWith(
      expect.any(String),
      [7, 1, 2, 3, 4],
    );
    expect(await screen.findByText(/falls outside the school's working days/)).toBeTruthy();
    assertNothingWritten();
  });

  it("is not re-asked on an unrelated re-render", async () => {
    const same = school([1, 2, 3, 4, 5]);
    const { rerender } = render(<CalendarMapping school={same} />);
    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalledTimes(1));

    rerender(<CalendarMapping school={school([1, 2, 3, 4, 5])} />);
    await waitFor(() => expect(nonWorkingInstructionalDays).toHaveBeenCalledTimes(1));
  });
});
