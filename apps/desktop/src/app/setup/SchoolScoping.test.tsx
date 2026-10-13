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
import { render, screen, waitFor, within } from "@testing-library/react";

import userEvent from "@testing-library/user-event";

import type { Campus, School, Teacher } from "@evara/domain";

const listTeachers = vi.fn<(schoolId: string) => Promise<Teacher[]>>();
const deleteTeacher = vi.fn<(schoolId: string, id: string) => Promise<void>>();
const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listAvailability = vi.fn(() => Promise.resolve([]));
const countAvailability = vi.fn(() => Promise.resolve(0));

vi.mock("../../ipc/setup", () => ({
  listTeachers: (schoolId: string) => listTeachers(schoolId),
  createTeacher: vi.fn(() => Promise.resolve({})),
  updateTeacher: vi.fn(() => Promise.resolve({})),
  deleteTeacher: (schoolId: string, id: string) => deleteTeacher(schoolId, id),
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listAvailability: () => listAvailability(),
  countAvailability: () => countAvailability(),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  listCycles: vi.fn(() => Promise.resolve([])),
  listPeriodStructures: vi.fn(() => Promise.resolve([])),
  gridPreview: vi.fn(() => Promise.resolve(null)),
  // Mirrors the real wrapper closely enough for these tests: the typed `wrongSchool`
  // error carries the message the user is shown.
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
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
  deleteTeacher.mockResolvedValue(undefined);
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

describe("deleting is scoped too", () => {
  /** Opens the delete confirmation for the one teacher on screen. */
  async function confirmDelete() {
    await userEvent.click(await screen.findByRole("button", { name: /Delete/ }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(
      within(dialog).getByRole("button", { name: /^Delete|Delete teacher/ }),
    );
  }

  it("sends the active school with the identifier", async () => {
    render(<Teachers school={NORTH} />);
    expect(await screen.findByText("Northwood")).toBeTruthy();

    await confirmDelete();

    await waitFor(() => expect(deleteTeacher).toHaveBeenCalled());
    expect(deleteTeacher).toHaveBeenCalledWith(NORTH.id, "t-north");
  });

  /**
   * The stale-context case, which is the reason the backend check exists.
   *
   * A confirmation opened under one school must not, once the school has changed,
   * delete against the school it was opened under. The panel re-reads on a switch, so
   * what is on screen afterwards is the new school's row — and that is what the delete
   * names.
   */
  it("never deletes against the school left behind", async () => {
    const view = render(<Teachers school={NORTH} />);
    expect(await screen.findByText("Northwood")).toBeTruthy();

    view.rerender(<Teachers school={SOUTH} />);
    expect(await screen.findByText("Southwell")).toBeTruthy();

    await confirmDelete();

    await waitFor(() => expect(deleteTeacher).toHaveBeenCalled());
    expect(deleteTeacher).toHaveBeenCalledWith(SOUTH.id, "t-south");
    for (const call of deleteTeacher.mock.calls) {
      expect(call[0]).not.toBe(NORTH.id);
    }
  });

  it("shows a wrong-school refusal rather than swallowing it", async () => {
    deleteTeacher.mockRejectedValue({
      kind: "wrongSchool",
      message: "that teacher belongs to a different school",
    });

    render(<Teachers school={NORTH} />);
    expect(await screen.findByText("Northwood")).toBeTruthy();

    await confirmDelete();

    expect(
      await screen.findByText(/belongs to a different school/i),
    ).toBeTruthy();
    // And the row is still there, because nothing was deleted.
    expect(screen.getByText("Northwood")).toBeTruthy();
  });
});
