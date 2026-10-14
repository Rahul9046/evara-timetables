/**
 * School working days, through the School Details form.
 *
 * `weekdays.test.ts` owns the conversion and `evara_core::calendar` owns the rule. This
 * file owns what only the screen can get wrong:
 *
 * - an existing school shows the week it was saved with, never a default standing in for it
 * - any combination saves, including Sunday to Thursday and a six-day week
 * - an empty week is refused with a message, not sent and not quietly filled in
 * - the week reaching the backend is a list of ISO weekday numbers
 * - the picker is operable from the keyboard alone, and every toggle is named
 *
 * The backend is mocked. `working_days BETWEEN 1 AND 127` in migration V7 is still the
 * authority on what a legal week is; nothing asserted here is a backend rule.
 *
 * **Where the new-school default is covered.** Stage 0C moved school *creation* to the
 * Schools section, so "a new school starts on Monday to Friday" is asserted in
 * `Schools.test.tsx`, against the screen that actually creates one. This file is about
 * editing the selected school's week, which is what School Details now does.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School } from "@evara/domain";

import { weekdayName } from "./weekdays";

const updateSchool = vi.fn();

const getSchoolLogo = vi.fn();
const getSchoolLogoLimits = vi.fn();
const setSchoolLogo = vi.fn();
const clearSchoolLogo = vi.fn();

vi.mock("../../ipc/setup", () => ({
  getSchoolLogo: () => getSchoolLogo(),
  getSchoolLogoLimits: () => getSchoolLogoLimits(),
  setSchoolLogo: (id: string, data: string) => setSchoolLogo(id, data),
  clearSchoolLogo: () => clearSchoolLogo(),
  updateSchool: (id: string, input: unknown) => updateSchool(id, input),
  describeSetupError: (error: unknown) => String(error),
}));

const { SchoolDetails } = await import("./SchoolDetails");

/**
 * The school the screen is being shown for.
 *
 * Since V12 the active school is chosen by the setup shell and passed in as a prop, so
 * "given this stored school" is expressed by setting this rather than by mocking a read.
 * It is never `null` here: a campus file with no school opens on Schools, and School
 * Details has nothing to edit.
 */
let active: School = {} as School;

function school(overrides: Partial<School> = {}): School {
  return {
    id: "school-1",
    name: "Northgate Secondary",
    timezone: "Europe/London",
    locale: "en-GB",
    workingDays: [1, 2, 3, 4, 5],
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-02T00:00:00.000Z",
    rev: 3,
    ...overrides,
  } as School;
}

beforeEach(() => {
  vi.clearAllMocks();
  // School Details renders the logo panel, which reads both of these on mount. Neither is
  // what these tests are about; `null` is the ordinary "this school uses the Evara logo"
  // answer. See SchoolLogoPanel.test.tsx for the panel's own behaviour.
  getSchoolLogo.mockResolvedValue(null);
  getSchoolLogoLimits.mockResolvedValue({
    maxBytes: 2 * 1024 * 1024,
    maxEdge: 2048,
    minEdge: 16,
    mediaTypes: ["image/png", "image/jpeg", "image/webp"],
  });
  setSchoolLogo.mockResolvedValue(null);
  clearSchoolLogo.mockResolvedValue(undefined);
  active = school();
  updateSchool.mockResolvedValue(school());
});

async function openForm() {
  render(<SchoolDetails school={active} onChanged={vi.fn()} />);
  await userEvent.click(await screen.findByRole("button", { name: /Edit details/ }));
}

/** One weekday toggle, by its full name — which is what a screen reader announces. */
const day = (name: string) => screen.getByRole("checkbox", { name }) as HTMLInputElement;

/**
 * The weekdays currently selected, named, in display order.
 *
 * Read from each box's `value` rather than its rendered text: the label deliberately
 * carries both the visible two letters and the hidden full name, so `textContent` would
 * read "WeWednesday".
 */
function selected(): string[] {
  return screen
    .getAllByRole("checkbox")
    .filter((box) => (box as HTMLInputElement).checked)
    .map((box) => weekdayName(Number((box as HTMLInputElement).value)));
}

/** The week the last save sent. */
function savedWeek(mock: typeof updateSchool): number[] {
  const input = mock.mock.calls.at(-1)?.at(-1) as { workingDays: number[] };
  return input.workingDays;
}

describe("an existing school", () => {
  it("shows Monday to Friday where that is what it holds", async () => {
    await openForm();

    expect(day("Monday").checked).toBe(true);
    expect(day("Tuesday").checked).toBe(true);
    expect(day("Wednesday").checked).toBe(true);
    expect(day("Thursday").checked).toBe(true);
    expect(day("Friday").checked).toBe(true);
    expect(day("Saturday").checked).toBe(false);
    expect(day("Sunday").checked).toBe(false);
  });

  it("sends the week back untouched when nothing about it was changed", async () => {
    await openForm();
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 5]);
  });

  it("shows the week it was saved with, not the default", async () => {
    active = school({ workingDays: [7, 1, 2, 3, 4] });
    await openForm();

    expect(day("Sunday").checked).toBe(true);
    expect(day("Monday").checked).toBe(true);
    expect(day("Thursday").checked).toBe(true);
    expect(day("Friday").checked).toBe(false);
    expect(day("Saturday").checked).toBe(false);
  });

  it("shows a six-day week", async () => {
    active = school({ workingDays: [1, 2, 3, 4, 5, 6] });
    await openForm();

    expect(selected()).toHaveLength(6);
    expect(day("Saturday").checked).toBe(true);
    expect(day("Sunday").checked).toBe(false);
  });

  it("shows a one-day week", async () => {
    active = school({ workingDays: [3] });
    await openForm();

    expect(selected()).toEqual(["Wednesday"]);
  });

  it("sends its week back unchanged when only the name is edited", async () => {
    active = school({ workingDays: [7, 1, 2, 3, 4] });
    await openForm();

    await userEvent.clear(screen.getByLabelText(/School name/));
    await userEvent.type(screen.getByLabelText(/School name/), "Northgate Academy");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 7]);
  });

  it("summarises its week in prose once saved", async () => {
    active = school({ workingDays: [1, 2, 3, 4, 5] });
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);

    expect(await screen.findByText("Monday to Friday")).toBeTruthy();
  });

  it("summarises an unusual week without pretending it is a range", async () => {
    active = school({ workingDays: [7, 1, 2, 3, 4] });
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);

    expect(
      await screen.findByText("Monday, Tuesday, Wednesday, Thursday and Sunday"),
    ).toBeTruthy();
  });
});

describe("changing the week", () => {
  it("saves an added day", async () => {
    active = school();
    await openForm();

    await userEvent.click(day("Saturday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("saves a removed day", async () => {
    active = school();
    await openForm();

    await userEvent.click(day("Friday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4]);
  });

  it("saves a Sunday-to-Thursday week built from scratch", async () => {
    active = school();
    await openForm();

    await userEvent.click(day("Friday"));
    await userEvent.click(day("Sunday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    // Ascending ISO order, whatever order the boxes were clicked in.
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 7]);
  });

  it("saves a weekend-only week", async () => {
    active = school();
    await openForm();

    for (const name of ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]) {
      await userEvent.click(day(name));
    }
    await userEvent.click(day("Saturday"));
    await userEvent.click(day("Sunday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([6, 7]);
  });

  it("saves every day", async () => {
    active = school();
    await openForm();

    await userEvent.click(day("Saturday"));
    await userEvent.click(day("Sunday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 5, 6, 7]);
  });

  it("toggles back off, so a day can be added by mistake and removed", async () => {
    active = school();
    await openForm();

    await userEvent.click(day("Saturday"));
    expect(day("Saturday").checked).toBe(true);
    await userEvent.click(day("Saturday"));
    expect(day("Saturday").checked).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 2, 3, 4, 5]);
  });
});

describe("a week with no days", () => {
  it("is refused with a message, and nothing is sent", async () => {
    active = school();
    await openForm();

    for (const name of ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]) {
      await userEvent.click(day(name));
    }
    expect(selected()).toEqual([]);

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText(/Choose at least one working day/)).toBeTruthy();
    expect(updateSchool).not.toHaveBeenCalled();
  });

  it("is not quietly turned into every day, or into Monday to Friday", async () => {
    active = school();
    await openForm();

    for (const name of ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]) {
      await userEvent.click(day(name));
    }
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText(/Choose at least one working day/);

    // The form still shows the empty week the user made, so they can see what is wrong.
    expect(selected()).toEqual([]);
    expect(updateSchool).not.toHaveBeenCalled();
  });

  it("clears its message as soon as a day is chosen", async () => {
    active = school();
    await openForm();

    for (const name of ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]) {
      await userEvent.click(day(name));
    }
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText(/Choose at least one working day/);

    await userEvent.click(day("Wednesday"));
    expect(screen.queryByText(/Choose at least one working day/)).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([3]);
  });

  it("does not disable the last remaining day", async () => {
    active = school({ workingDays: [1] });
    await openForm();

    // Clearing the row to start again is a reasonable thing to do; a toggle that refuses
    // to switch off is confusing, and the message is what teaches the rule.
    expect(day("Monday").disabled).toBe(false);
    await userEvent.click(day("Monday"));
    expect(day("Monday").checked).toBe(false);
  });
});

describe("accessibility", () => {
  it("is a named group of seven checkboxes", async () => {
    await openForm();

    const group = screen.getByRole("group", { name: /School working days/ });
    expect(group.tagName).toBe("FIELDSET");
    expect(screen.getAllByRole("checkbox")).toHaveLength(7);
  });

  it("names every toggle by its full weekday, not its two letters", async () => {
    await openForm();

    for (const name of [
      "Monday",
      "Tuesday",
      "Wednesday",
      "Thursday",
      "Friday",
      "Saturday",
      "Sunday",
    ]) {
      expect(screen.getByRole("checkbox", { name })).toBeTruthy();
    }
    // "Mo" is shown but hidden from the accessibility tree, so it is never read out as
    // the control's name.
    expect(screen.queryByRole("checkbox", { name: "Mo" })).toBeNull();
  });

  it("is reachable and operable by keyboard alone", async () => {
    active = school();
    await openForm();

    day("Monday").focus();
    expect(document.activeElement).toBe(day("Monday"));

    // Space toggles a checkbox. This is the whole reason these are real checkboxes
    // rather than buttons with aria-pressed.
    await userEvent.keyboard(" ");
    expect(day("Monday").checked).toBe(false);

    await userEvent.keyboard(" ");
    expect(day("Monday").checked).toBe(true);

    // Tab walks to the next day rather than out of the group.
    await userEvent.tab();
    expect(document.activeElement).toBe(day("Tuesday"));
    await userEvent.keyboard(" ");
    expect(day("Tuesday").checked).toBe(false);

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(savedWeek(updateSchool)).toEqual([1, 3, 4, 5]);
  });

  it("marks every toggle invalid when the week is empty", async () => {
    active = school({ workingDays: [1] });
    await openForm();

    await userEvent.click(day("Monday"));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText(/Choose at least one working day/);

    const invalid = screen
      .getAllByRole("checkbox")
      .filter((box) => box.getAttribute("aria-invalid") === "true");
    expect(invalid).toHaveLength(7);
  });

  it("explains that working days are not the cycle's length", async () => {
    await openForm();

    const hint = screen.getByText(/weekdays this school normally teaches on/);
    expect(hint.textContent).toMatch(/does not set the cycle's length/);
    expect(hint.textContent).toMatch(/six-day rotation across a five-day week/);
  });
});
