/**
 * Stage 0A: no screen shows one school's records under another school.
 *
 * The repository tests in `evara-db` own the rule — a scoped query returns one school's
 * rows and no more. This file owns the two things only the interface can get wrong, and
 * which no backend test can catch:
 *
 * - a list is **asked for** with the active school's identifier, not with none
 * - switching schools **re-asks**, rather than leaving the previous school's rows on
 *   screen under the new school's name
 *
 * The second is the one worth having. Every screen takes `school` as a prop and most
 * fetch in a `useCallback` whose dependency list decides whether a switch re-reads; a
 * missing `school.id` there produces a screen that is wrong in the most convincing
 * possible way — populated, plausible, and about a different institution.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import type { Campus, School, Teacher } from "@evara/domain";

const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listAvailability = vi.fn(() => Promise.resolve([]));
const countAvailability = vi.fn(() => Promise.resolve(0));

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  createTeacher: vi.fn(() => Promise.resolve({})),
  updateTeacher: vi.fn(() => Promise.resolve({})),
  deleteTeacher: vi.fn(() => Promise.resolve()),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listAvailability: () => listAvailability(),
  countAvailability: () => countAvailability(),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  listCycles: vi.fn(() => Promise.resolve([])),
  listPeriodStructures: vi.fn(() => Promise.resolve([])),
  gridPreview: vi.fn(() => Promise.resolve(null)),
  describeSetupError: (error: unknown) => String(error),
  offendingField: () => null,
}));

const { Teachers } = await import("./Teachers");

const NORTH = { id: "school-north", name: "Northgate Secondary" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College" } as School;

function teacher(id: string, family: string): Teacher {
  return {
    id,
    schoolId: "school-north",
    code: null,
    givenName: "Ada",
    familyName: family,
    email: null,
    homeCampusId: null,
    ftePercent: 100,
    maxPeriodsPerDay: null,
    maxPeriodsPerCycle: null,
    maxConsecutivePeriods: null,
    notes: null,
    isActive: true,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as Teacher;
}

beforeEach(() => {
  vi.clearAllMocks();
  listCampuses.mockResolvedValue([]);
  listAvailability.mockResolvedValue([]);
  countAvailability.mockResolvedValue(0);
  listTeachers.mockImplementation((schoolId: string) =>
    Promise.resolve(
      schoolId === NORTH.id
        ? [teacher("t-north", "Northwood")]
        : [teacher("t-south", "Southwell")],
    ),
  );
});

describe("a school-owned listing", () => {
  it("is asked for with the active school's identifier", async () => {
    render(<Teachers school={NORTH} />);

    await waitFor(() => expect(listTeachers).toHaveBeenCalled());
    expect(listTeachers).toHaveBeenCalledWith(NORTH.id);
    // Every list on the screen carries it, not just the main one.
    expect(listCampuses).toHaveBeenCalledWith(NORTH.id);
  });

  it("never asks without one", async () => {
    render(<Teachers school={NORTH} />);

    await waitFor(() => expect(listTeachers).toHaveBeenCalled());
    for (const call of listTeachers.mock.calls) {
      expect(call[0]).toBe(NORTH.id);
    }
  });
});

describe("switching the active school", () => {
  it("re-reads, and shows no trace of the school left behind", async () => {
    const view = render(<Teachers school={NORTH} />);
    expect(await screen.findByText("Northwood")).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} />);

    expect(await screen.findByText("Southwell")).toBeTruthy();
    expect(listTeachers).toHaveBeenCalledWith(SOUTH.id);
    await waitFor(() => expect(screen.queryByText("Northwood")).toBeNull());
  });

  it("re-reads the supporting lists too, not only the main one", async () => {
    const view = render(<Teachers school={NORTH} />);
    await waitFor(() => expect(listCampuses).toHaveBeenCalledWith(NORTH.id));

    view.rerender(<Teachers school={SOUTH} />);

    await waitFor(() => expect(listCampuses).toHaveBeenCalledWith(SOUTH.id));
  });
});
