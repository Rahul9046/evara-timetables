/**
 * School Details, end to end through the form.
 *
 * The component tests in `ui/Combobox.test.tsx` cover the picker's own behaviour. This
 * file covers the thing that actually matters to a school: that **what reaches the
 * backend is the identifier the user chose**, and that opening an existing project does
 * not quietly alter what it already holds.
 *
 * The backend is mocked because none of what is asserted here is a backend rule. The
 * `CHECK (length(trim(timezone)) > 0)` in migration V3 is still the authority on what a
 * valid value is, and it is untouched by this change.
 *
 * **Stage 0C moved creation out.** A school is created in the Schools section, which is
 * about the project; this screen edits the one that is selected. `Schools.test.tsx` covers
 * the create path against the screen that performs it, and the form itself is one
 * definition in `schoolForm.ts`, so the fields asserted here are the fields asserted
 * there.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School } from "@evara/domain";

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
  updateSchool: (id: string, rev: number, input: unknown) =>
    updateSchool(id, rev, input),
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
    typeof error === "object" && error !== null && (error as { kind?: string }).kind === "duplicate"
      ? ((error as { field: string }).field ?? null)
      : null,
}));

const { SchoolDetails } = await import("./SchoolDetails");

/**
 * The school the screen is being shown for.
 *
 * Since V12 the active school is chosen by the setup shell and passed in as a prop, so
 * "given this stored school" is expressed by setting this rather than by mocking a read.
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
  updateSchool.mockImplementation((_id: string, input: Record<string, string>) =>
    Promise.resolve(school(input)),
  );
});

/** Opens the edit form on the selected school. */
async function openForm() {
  render(<SchoolDetails school={active} onChanged={vi.fn()} />);
  await userEvent.click(await screen.findByRole("button", { name: /Edit details/ }));
}

const timezoneBox = () => screen.getByRole("combobox", { name: /Time zone/ });
const localeBox = () => screen.getByRole("combobox", { name: /Locale/ });

describe("selecting a timezone", () => {
  it("searches by city and stores the IANA identifier", async () => {
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata");
    await userEvent.click(screen.getByText("India Standard Time — Asia/Kolkata"));

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      timezone: "Asia/Kolkata",
    });
  });

  it("stores the identifier even though the user only ever saw the label", async () => {
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    // Searched by identifier rather than by name on purpose: "India Standard Time" is
    // ambiguous — Asia/Colombo carries it too — which is exactly why the identifier is
    // part of the label and the only thing stored.
    await userEvent.type(box, "kolkata{Enter}");

    // The box shows a label...
    expect((box as HTMLInputElement).value).toBe("India Standard Time — Asia/Kolkata");

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    // ...and the backend is sent an identifier.
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    const input = updateSchool.mock.calls[0]?.[2] as Record<string, string>;
    expect(input.timezone).toBe("Asia/Kolkata");
    expect(input.timezone).not.toContain("—");
  });

  it("is chosen with the keyboard alone", async () => {
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "tokyo");
    await userEvent.keyboard("{ArrowDown}{Enter}");

    expect((box as HTMLInputElement).value).toContain("Asia/Tokyo");
  });
});

describe("selecting a locale", () => {
  it("searches by language and region and stores the tag", async () => {
    await openForm();

    const box = localeBox();
    await userEvent.clear(box);
    await userEvent.type(box, "english india{Enter}");

    expect((box as HTMLInputElement).value).toBe("English (India) — en-IN");

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({ locale: "en-IN" });
  });

  it("is not inferred from the timezone", async () => {
    await openForm();

    // Choosing an Indian timezone must not touch the locale. A British school in Dubai
    // reports in en-GB; geography is not audience.
    const before = (localeBox() as HTMLInputElement).value;

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata{Enter}");

    expect((localeBox() as HTMLInputElement).value).toBe(before);
  });
});

describe("reopening an existing project", () => {
  it("shows the saved timezone and locale", async () => {
    active = school();
    await openForm();

    expect((timezoneBox() as HTMLInputElement).value).toBe(
      "United Kingdom Time — Europe/London",
    );
    expect((localeBox() as HTMLInputElement).value).toBe(
      "English (United Kingdom) — en-GB",
    );
  });

  it("saves the same values back untouched when nothing was changed", async () => {
    active = school();
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool).toHaveBeenCalledWith("school-1", 3, {
      name: "Northgate Secondary",
      // A school with no code saves none. `code` is nullable and unique only when given,
      // so an empty box must reach the column as NULL rather than as "".
      code: null,
      timezone: "Europe/London",
      locale: "en-GB",
      workingDays: [1, 2, 3, 4, 5],
    });
  });

  it("summarises the saved values with readable labels", async () => {
    active = school();
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);

    expect(
      await screen.findByText("United Kingdom Time — Europe/London"),
    ).toBeTruthy();
    expect(screen.getByText("English (United Kingdom) — en-GB")).toBeTruthy();
  });
});

/**
 * The school code, exposed by Stage 0C.
 *
 * `V12` added it and nothing collected it, so `SchoolDetails` carried the stored value
 * through by hand to stop a save clearing one it had never shown. The field replaces that
 * workaround, and the thing worth testing is that it replaced it *without* reintroducing
 * the bug from the other side: editing the time zone must not clear a code, and clearing
 * the box must mean no code rather than an empty string, because the column is unique only
 * when given and `""` would collide between two un-coded schools.
 */
describe("the school code", () => {
  it("shows the stored code in the summary", async () => {
    active = school({ code: "NGS" });
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);

    expect(await screen.findByText("NGS")).toBeTruthy();
  });

  it("says so plainly when there is no code", async () => {
    active = school({ code: null });
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);

    expect(await screen.findByText("Not set")).toBeTruthy();
  });

  it("is editable, and reaches the backend as typed", async () => {
    active = school({ code: null });
    await openForm();

    await userEvent.type(screen.getByLabelText(/School code/), "NGS");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({ code: "NGS" });
  });

  it("is preserved when another field is edited", async () => {
    // The regression this field exists to make impossible, from the direction that used to
    // be handled by carrying the value through by hand.
    active = school({ code: "NGS" });
    await openForm();

    const box = screen.getByRole("combobox", { name: /Time zone/ });
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      code: "NGS",
      timezone: "Asia/Kolkata",
    });
  });

  it("sends null, never an empty string, when the box is cleared", async () => {
    active = school({ code: "NGS" });
    await openForm();

    await userEvent.clear(screen.getByLabelText(/School code/));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    // `code` is unique when given through a partial index, so two schools saved with ""
    // would collide on a value neither of them has.
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({ code: null });
  });

  it("marks the code field when the backend reports a duplicate", async () => {
    active = school({ code: null });
    updateSchool.mockRejectedValue({
      kind: "duplicate",
      entity: "school",
      field: "code",
      message: "another school already uses that code",
    });
    await openForm();

    await userEvent.type(screen.getByLabelText(/School code/), "SMC");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    // The form stays open with the typed values intact, which is the point: retyping a
    // whole record to fix one box is not a reasonable ask.
    expect(await screen.findByText(/already uses that code/i)).toBeTruthy();
    expect((screen.getByLabelText(/School code/) as HTMLInputElement).value).toBe("SMC");
  });
});

describe("a stored value the list does not offer", () => {
  it("shows a hand-typed timezone as an existing value rather than blanking it", async () => {
    // What a project written while these were free-text boxes can hold.
    active = school({ timezone: "Mars/Olympus_Mons" });
    await openForm();

    const shown = (timezoneBox() as HTMLInputElement).value;
    expect(shown).toContain("Mars/Olympus_Mons");
    expect(shown).toContain("existing value, not in this list");
  });

  it("keeps that value on save unless the user picks another", async () => {
    active = school({ timezone: "Mars/Olympus_Mons" });
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      timezone: "Mars/Olympus_Mons",
    });
  });

  it("does not change it merely because the user typed a search", async () => {
    active = school({ timezone: "Mars/Olympus_Mons" });
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata");
    // Typed a query, then thought better of it.
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      timezone: "Mars/Olympus_Mons",
    });
  });

  it("replaces it only on a deliberate selection", async () => {
    active = school({ timezone: "Mars/Olympus_Mons" });
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      timezone: "Asia/Kolkata",
    });
  });

  it("keeps a retired IANA spelling, explaining the current name", async () => {
    // Asia/Calcutta still resolves to Indian time. Rewriting it on open would be a
    // silent write that bumps rev and teaches the user nothing.
    active = school({ timezone: "Asia/Calcutta" });
    await openForm();

    const shown = (timezoneBox() as HTMLInputElement).value;
    expect(shown).toContain("Asia/Calcutta");
    expect(shown).toContain("renamed to Asia/Kolkata");

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[2]).toMatchObject({
      timezone: "Asia/Calcutta",
    });
  });

  it("shows an unrecognised locale the same way", async () => {
    active = school({ locale: "xx-ZZ" });
    await openForm();

    expect((localeBox() as HTMLInputElement).value).toBe(
      "xx-ZZ (existing value, not in this list)",
    );
  });
});

describe("required fields", () => {
  it("refuses to save a school with its name cleared", async () => {
    await openForm();

    await userEvent.clear(screen.getByLabelText(/School name/));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("School name is required.")).toBeTruthy();
    expect(updateSchool).not.toHaveBeenCalled();
  });

  it("refuses to save a cleared timezone", async () => {
    active = school();
    await openForm();

    // There is no way to type a value into the picker, but there is a way to end up
    // with none: an existing project whose timezone is somehow blank.
    active = school({ timezone: "" });
    render(<SchoolDetails school={active} onChanged={vi.fn()} />);
    const buttons = await screen.findAllByRole("button", { name: "Edit details" });
    await userEvent.click(buttons[buttons.length - 1]!);

    const saves = screen.getAllByRole("button", { name: "Save changes" });
    await userEvent.click(saves[saves.length - 1]!);

    expect(await screen.findByText("Time zone is required.")).toBeTruthy();
  });

  it("marks the offending picker as invalid for a screen reader", async () => {
    active = school({ locale: "" });
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(localeBox().getAttribute("aria-invalid")).toBe("true"),
    );
    const errorId = localeBox().getAttribute("aria-errormessage");
    expect(errorId).toBeTruthy();
    expect(document.getElementById(errorId!)?.textContent).toBe("Locale is required.");
  });

  it("clears the error once a value is chosen", async () => {
    active = school({ locale: "" });
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    expect(await screen.findByText("Locale is required.")).toBeTruthy();

    await userEvent.type(localeBox(), "english india{Enter}");

    await waitFor(() =>
      expect(screen.queryByText("Locale is required.")).toBeNull(),
    );
  });
});

describe("the pickers are offered at all", () => {
  it("replaces the old free-text inputs with comboboxes", async () => {
    await openForm();

    // The regression this fix is about: these were `<input type="text">`.
    expect(timezoneBox().getAttribute("role")).toBe("combobox");
    expect(localeBox().getAttribute("role")).toBe("combobox");
  });

  it("offers a long list without a network round trip", async () => {
    await openForm();

    await userEvent.click(timezoneBox());
    expect(screen.getAllByRole("option").length).toBeGreaterThan(50);
  });
});
