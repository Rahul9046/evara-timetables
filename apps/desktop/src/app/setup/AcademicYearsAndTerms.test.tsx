/**
 * Academic Years and Terms, as a screen.
 *
 * `evara-db` owns the rules — `ends_on >= starts_on`, what may be deleted, what is
 * unique. `../dates.test.ts` owns the arithmetic and `../ui/DateInput.test.tsx` owns the
 * control. What is left, and what this file is about, is the contract between them:
 *
 * - all four date fields are the same control, with the `DD/MM/YYYY` placeholder
 * - a stored date is read back as `DD/MM/YYYY`, and editing a row does not disturb it
 * - what reaches `create` and `update` is `YYYY-MM-DD`, whether typed or picked
 * - an impossible date is reported, not rounded into a real one
 * - a backend refusal is shown as it was written
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { AcademicYear, School, Term } from "@evara/domain";

const listAcademicYears = vi.fn<() => Promise<AcademicYear[]>>();
const createAcademicYear = vi.fn();
const updateAcademicYear = vi.fn();
const deleteAcademicYear = vi.fn();
const listTerms = vi.fn<(yearId: string) => Promise<Term[]>>();
const createTerm = vi.fn();
const updateTerm = vi.fn();
const deleteTerm = vi.fn();

vi.mock("../../ipc/setup", () => ({
  listAcademicYears: () => listAcademicYears(),
  createAcademicYear: (input: unknown) => createAcademicYear(input),
  updateAcademicYear: (id: string, rev: number, input: unknown) =>
    updateAcademicYear(id, rev, input),
  deleteAcademicYear: (id: string) => deleteAcademicYear(id),
  listTerms: (yearId: string) => listTerms(yearId),
  createTerm: (input: unknown) => createTerm(input),
  updateTerm: (id: string, rev: number, input: unknown) =>
    updateTerm(id, rev, input),
  deleteTerm: (id: string) => deleteTerm(id),
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
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" && error !== null && "field" in error
      ? String((error as { field: unknown }).field)
      : null,
}));

const { AcademicYearsAndTerms } = await import("./AcademicYearsAndTerms");

const SCHOOL = { id: "school-1" } as School;

function year(id: string, name: string, startsOn: string, endsOn: string): AcademicYear {
  return {
    id,
    schoolId: "school-1",
    name,
    startsOn,
    endsOn,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as AcademicYear;
}

function term(
  id: string,
  name: string,
  ordinal: number,
  startsOn: string,
  endsOn: string,
): Term {
  return {
    id,
    academicYearId: "y1",
    name,
    ordinal,
    startsOn,
    endsOn,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as Term;
}

const YEAR = year("y1", "2027/28", "2027-09-01", "2028-07-14");
const AUTUMN = term("t1", "Autumn", 1, "2027-09-01", "2027-12-17");

beforeEach(() => {
  vi.clearAllMocks();
  listAcademicYears.mockResolvedValue([YEAR]);
  listTerms.mockResolvedValue([AUTUMN]);
  createAcademicYear.mockResolvedValue(YEAR);
  updateAcademicYear.mockResolvedValue(YEAR);
  createTerm.mockResolvedValue(AUTUMN);
  updateTerm.mockResolvedValue(AUTUMN);
});

/** Waits for the first read to land, so the table is on screen. */
async function shown() {
  render(<AcademicYearsAndTerms school={SCHOOL} />);
  await waitFor(() => expect(listAcademicYears).toHaveBeenCalled());
  await screen.findByRole("button", { name: /^Edit 2027\/28$/ });
}

/** The open form, named by its legend — "New academic year", "Edit 2027/28". */
const form = () =>
  screen.getByRole("group", { name: /^(New|Edit)/ }) as HTMLFieldSetElement;

/**
 * One input of the open form.
 *
 * Anchored rather than exact, because a required field's label carries a ` *` marker,
 * and loose at the end rather than both, because "Year name" must not be found by the
 * calendar's own "Year" select.
 */
const field = (label: string) =>
  screen.getByLabelText(new RegExp(`^${label}`)) as HTMLInputElement;

/** The calendar button beside a date field. */
const calendarFor = (label: string) =>
  screen.getByRole("button", { name: new RegExp(`choose ${label} from a calendar`, "i") });

async function openNewYear(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Add academic year" }));
}

async function openEditYear(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Edit 2027/28" }));
}

describe("the date fields", () => {
  it("are date controls with a format placeholder, not example dates", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    for (const label of ["First day", "Last day"]) {
      const input = field(label);
      expect(input.getAttribute("placeholder")).toBe("DD/MM/YYYY");
      // The old field showed `2027-09-01`, which reads as a value and teaches the wrong
      // order. Nothing on screen may suggest a date in either direction.
      expect(input.value).toBe("");
      expect(calendarFor(label)).toBeTruthy();
    }
  });

  it("are the same control for a term as for a year", async () => {
    const user = userEvent.setup();
    await shown();
    // The term panel appears once the picker has settled on a year.
    await user.click(await screen.findByRole("button", { name: "Add term" }));

    for (const label of ["First day", "Last day"]) {
      expect(field(label).getAttribute("placeholder")).toBe("DD/MM/YYYY");
      expect(calendarFor(label)).toBeTruthy();
    }
  });

  it("read a stored date back in the display format", async () => {
    const user = userEvent.setup();
    await shown();
    await openEditYear(user);

    expect(field("First day").value).toBe("01/09/2027");
    expect(field("Last day").value).toBe("14/07/2028");
  });
});

describe("the tables", () => {
  it("show stored dates the way a person reads them, keeping the stored value in the markup", async () => {
    await shown();

    const cell = screen.getByText("01/09/2027");
    expect(cell.tagName).toBe("TIME");
    expect(cell.getAttribute("datetime")).toBe("2027-09-01");
    expect(screen.getByText("14/07/2028")).toBeTruthy();
  });

  it("show a term's dates the same way", async () => {
    await shown();
    await screen.findByRole("button", { name: /^Edit Autumn$/ });

    expect(screen.getByText("17/12/2027")).toBeTruthy();
  });

  it("show the year picker's range in the display format", async () => {
    await shown();

    const picker = screen.getByRole("group", { name: "Academic year" });
    expect(within(picker).getByText("01/09/2027 → 14/07/2028")).toBeTruthy();
  });
});

describe("saving", () => {
  it("sends the stored format for a typed date", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.type(field("First day"), "01/09/2028");
    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    await waitFor(() => expect(createAcademicYear).toHaveBeenCalled());
    expect(createAcademicYear).toHaveBeenCalledWith({
      schoolId: "school-1",
      name: "2028/29",
      startsOn: "2028-09-01",
      endsOn: "2029-07-13",
    });
  });

  it("sends the stored format for a picked date", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");

    await user.click(calendarFor("First day"));
    await user.selectOptions(
      within(screen.getByRole("dialog")).getByLabelText("Month"),
      "9",
    );
    await user.selectOptions(
      within(screen.getByRole("dialog")).getByLabelText("Year"),
      "2028",
    );
    await user.click(screen.getByRole("button", { name: "1 September 2028" }));

    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    await waitFor(() => expect(createAcademicYear).toHaveBeenCalled());
    // The same value typing produced above: the calendar is not a second format.
    expect(createAcademicYear).toHaveBeenCalledWith({
      schoolId: "school-1",
      name: "2028/29",
      startsOn: "2028-09-01",
      endsOn: "2029-07-13",
    });
  });

  it("sends an existing record's dates back unchanged when only its name is edited", async () => {
    const user = userEvent.setup();
    await shown();
    await openEditYear(user);

    await user.clear(field("Year name"));
    await user.type(field("Year name"), "2027/2028");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateAcademicYear).toHaveBeenCalled());
    expect(updateAcademicYear).toHaveBeenCalledWith("y1", 1, {
      schoolId: "school-1",
      name: "2027/2028",
      // Byte for byte what was read, with no normalising round trip through a Date.
      startsOn: "2027-09-01",
      endsOn: "2028-07-14",
    });
  });

  it("sends a term's dates in the stored format", async () => {
    const user = userEvent.setup();
    await shown();
    await screen.findByRole("button", { name: /^Edit Autumn$/ });
    await user.click(screen.getByRole("button", { name: "Edit Autumn" }));

    expect(field("First day").value).toBe("01/09/2027");

    await user.clear(field("Last day"));
    await user.type(field("Last day"), "18/12/2027");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateTerm).toHaveBeenCalled());
    expect(updateTerm).toHaveBeenCalledWith("t1", 1, {
      academicYearId: "y1",
      name: "Autumn",
      ordinal: 1,
      startsOn: "2027-09-01",
      endsOn: "2027-12-18",
    });
  });
});

describe("validation", () => {
  it("refuses a date that names no real day, and says so", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.type(field("First day"), "31/02/2026");
    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(await screen.findByText(/Start date is not a real date/)).toBeTruthy();
    // Nothing was sent, and nothing was silently turned into the 3rd of March.
    expect(createAcademicYear).not.toHaveBeenCalled();
    expect(field("First day").value).toBe("31/02/2026");
    expect(field("First day").getAttribute("aria-invalid")).toBe("true");
  });

  it("refuses a leap day in a year that has none, and accepts one in a year that does", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2027/28");
    await user.type(field("First day"), "29/02/2027");
    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(await screen.findByText(/Start date is not a real date/)).toBeTruthy();

    await user.clear(field("First day"));
    await user.type(field("First day"), "29/02/2028");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    await waitFor(() => expect(createAcademicYear).toHaveBeenCalled());
    expect(createAcademicYear).toHaveBeenCalledWith(
      expect.objectContaining({ startsOn: "2028-02-29" }),
    );
  });

  it("names the format when the field holds something that is not a date", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.type(field("First day"), "next Tuesday");
    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(await screen.findByText(/must be a date like DD\/MM\/YYYY/)).toBeTruthy();
    expect(createAcademicYear).not.toHaveBeenCalled();
  });

  it("refuses a range that ends before it starts", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.type(field("First day"), "01/09/2028");
    await user.type(field("Last day"), "31/08/2028");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(
      await screen.findByText(/The end date cannot be before the start date/),
    ).toBeTruthy();
    expect(createAcademicYear).not.toHaveBeenCalled();
  });

  it("accepts a single-day range, which the schema does too", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    // `CHECK (ends_on >= starts_on)` — equal ends are legal, so the form must not
    // invent a rule the model does not have.
    await user.type(field("Year name"), "Twilight");
    await user.type(field("First day"), "01/09/2028");
    await user.type(field("Last day"), "01/09/2028");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    await waitFor(() => expect(createAcademicYear).toHaveBeenCalled());
    expect(createAcademicYear).toHaveBeenCalledWith(
      expect.objectContaining({ startsOn: "2028-09-01", endsOn: "2028-09-01" }),
    );
  });

  it("requires both dates", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(await screen.findByText(/Start date is required/)).toBeTruthy();
    expect(screen.getByText(/End date is required/)).toBeTruthy();
    expect(createAcademicYear).not.toHaveBeenCalled();
  });

  it("does not complain while a date is still being typed", async () => {
    const user = userEvent.setup();
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));
    expect(await screen.findByText(/Start date is required/)).toBeTruthy();

    // Retyping clears the message for that field rather than leaving a stale one under
    // a box that now has something in it.
    await user.type(field("First day"), "01/0");
    expect(screen.queryByText(/Start date is required/)).toBeNull();
    expect(field("First day").value).toBe("01/0");
  });

  it("shows a refusal from the backend as it was written", async () => {
    const user = userEvent.setup();
    createAcademicYear.mockRejectedValue({
      kind: "invalid",
      message: "A year must not overlap another year in the same school.",
    });
    await shown();
    await openNewYear(user);

    await user.type(field("Year name"), "2028/29");
    await user.type(field("First day"), "01/09/2028");
    await user.type(field("Last day"), "13/07/2029");
    await user.click(screen.getByRole("button", { name: "Add academic year" }));

    expect(
      await screen.findByText("A year must not overlap another year in the same school."),
    ).toBeTruthy();
    // The form stays open with what was typed, so nothing has to be entered twice.
    expect(field("First day").value).toBe("01/09/2028");
  });
});

describe("the calendar on this screen", () => {
  it("opens on the stored date when an existing year is edited", async () => {
    const user = userEvent.setup();
    await shown();
    await openEditYear(user);

    await user.click(calendarFor("First day"));

    const dialog = screen.getByRole("dialog", { name: "First day calendar" });
    expect((within(dialog).getByLabelText("Month") as HTMLSelectElement).value).toBe("9");
    expect((within(dialog).getByLabelText("Year") as HTMLSelectElement).value).toBe("2027");
    expect(
      within(dialog).getByRole("button", { name: "1 September 2027" }).getAttribute(
        "aria-selected",
      ),
    ).toBe("true");
  });

  it("escapes the form's own box, so a scrolling section cannot clip it", async () => {
    const user = userEvent.setup();
    await shown();
    await openEditYear(user);

    await user.click(calendarFor("First day"));

    const dialog = screen.getByRole("dialog");
    expect(form().contains(dialog)).toBe(false);
    expect(dialog.parentElement).toBe(document.body);
  });

  it("does not submit the form when a day is chosen", async () => {
    const user = userEvent.setup();
    await shown();
    await openEditYear(user);

    await user.click(calendarFor("Last day"));
    await user.click(screen.getByRole("button", { name: "21 July 2028" }));

    expect(updateAcademicYear).not.toHaveBeenCalled();
    expect(field("Last day").value).toBe("21/07/2028");
  });
});
