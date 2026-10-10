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
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { School } from "@evara/domain";

const getSchool = vi.fn<() => Promise<School | null>>();
const createSchool = vi.fn();
const updateSchool = vi.fn();

vi.mock("../../ipc/setup", () => ({
  getSchool: () => getSchool(),
  createSchool: (input: unknown) => createSchool(input),
  updateSchool: (id: string, input: unknown) => updateSchool(id, input),
  describeSetupError: (error: unknown) => String(error),
}));

const { SchoolDetails } = await import("./SchoolDetails");

function school(overrides: Partial<School> = {}): School {
  return {
    id: "school-1",
    name: "Northgate Secondary",
    timezone: "Europe/London",
    locale: "en-GB",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-02T00:00:00.000Z",
    rev: 3,
    ...overrides,
  } as School;
}

beforeEach(() => {
  vi.clearAllMocks();
  getSchool.mockResolvedValue(null);
  createSchool.mockImplementation((input: Record<string, string>) =>
    Promise.resolve(school(input)),
  );
  updateSchool.mockImplementation((_id: string, input: Record<string, string>) =>
    Promise.resolve(school(input)),
  );
});

/** Opens the edit form on whatever the backend returned. */
async function openForm() {
  render(<SchoolDetails onChanged={vi.fn()} />);
  const button = await screen.findByRole("button", {
    name: /Describe the school|Edit details/,
  });
  await userEvent.click(button);
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

    await userEvent.type(screen.getByLabelText(/School name/), "Northgate");
    await userEvent.click(screen.getByRole("button", { name: "Create school" }));

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    expect(createSchool.mock.calls[0]?.[0]).toMatchObject({
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

    await userEvent.type(screen.getByLabelText(/School name/), "Northgate");
    await userEvent.click(screen.getByRole("button", { name: "Create school" }));

    // ...and the backend is sent an identifier.
    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    const input = createSchool.mock.calls[0]?.[0] as Record<string, string>;
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

    await userEvent.type(screen.getByLabelText(/School name/), "Northgate");
    await userEvent.click(screen.getByRole("button", { name: "Create school" }));

    await waitFor(() => expect(createSchool).toHaveBeenCalled());
    expect(createSchool.mock.calls[0]?.[0]).toMatchObject({ locale: "en-IN" });
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
    getSchool.mockResolvedValue(school());
    await openForm();

    expect((timezoneBox() as HTMLInputElement).value).toBe(
      "United Kingdom Time — Europe/London",
    );
    expect((localeBox() as HTMLInputElement).value).toBe(
      "English (United Kingdom) — en-GB",
    );
  });

  it("saves the same values back untouched when nothing was changed", async () => {
    getSchool.mockResolvedValue(school());
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool).toHaveBeenCalledWith("school-1", {
      name: "Northgate Secondary",
      timezone: "Europe/London",
      locale: "en-GB",
    });
  });

  it("summarises the saved values with readable labels", async () => {
    getSchool.mockResolvedValue(school());
    render(<SchoolDetails onChanged={vi.fn()} />);

    expect(
      await screen.findByText("United Kingdom Time — Europe/London"),
    ).toBeTruthy();
    expect(screen.getByText("English (United Kingdom) — en-GB")).toBeTruthy();
  });
});

describe("a stored value the list does not offer", () => {
  it("shows a hand-typed timezone as an existing value rather than blanking it", async () => {
    // What a project written while these were free-text boxes can hold.
    getSchool.mockResolvedValue(school({ timezone: "Mars/Olympus_Mons" }));
    await openForm();

    const shown = (timezoneBox() as HTMLInputElement).value;
    expect(shown).toContain("Mars/Olympus_Mons");
    expect(shown).toContain("existing value, not in this list");
  });

  it("keeps that value on save unless the user picks another", async () => {
    getSchool.mockResolvedValue(school({ timezone: "Mars/Olympus_Mons" }));
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      timezone: "Mars/Olympus_Mons",
    });
  });

  it("does not change it merely because the user typed a search", async () => {
    getSchool.mockResolvedValue(school({ timezone: "Mars/Olympus_Mons" }));
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata");
    // Typed a query, then thought better of it.
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      timezone: "Mars/Olympus_Mons",
    });
  });

  it("replaces it only on a deliberate selection", async () => {
    getSchool.mockResolvedValue(school({ timezone: "Mars/Olympus_Mons" }));
    await openForm();

    const box = timezoneBox();
    await userEvent.clear(box);
    await userEvent.type(box, "kolkata{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      timezone: "Asia/Kolkata",
    });
  });

  it("keeps a retired IANA spelling, explaining the current name", async () => {
    // Asia/Calcutta still resolves to Indian time. Rewriting it on open would be a
    // silent write that bumps rev and teaches the user nothing.
    getSchool.mockResolvedValue(school({ timezone: "Asia/Calcutta" }));
    await openForm();

    const shown = (timezoneBox() as HTMLInputElement).value;
    expect(shown).toContain("Asia/Calcutta");
    expect(shown).toContain("renamed to Asia/Kolkata");

    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateSchool).toHaveBeenCalled());
    expect(updateSchool.mock.calls[0]?.[1]).toMatchObject({
      timezone: "Asia/Calcutta",
    });
  });

  it("shows an unrecognised locale the same way", async () => {
    getSchool.mockResolvedValue(school({ locale: "xx-ZZ" }));
    await openForm();

    expect((localeBox() as HTMLInputElement).value).toBe(
      "xx-ZZ (existing value, not in this list)",
    );
  });
});

describe("required fields", () => {
  it("refuses to create a school with no name", async () => {
    await openForm();

    await userEvent.click(screen.getByRole("button", { name: "Create school" }));

    expect(await screen.findByText("School name is required.")).toBeTruthy();
    expect(createSchool).not.toHaveBeenCalled();
  });

  it("refuses to save a cleared timezone", async () => {
    getSchool.mockResolvedValue(school());
    await openForm();

    // There is no way to type a value into the picker, but there is a way to end up
    // with none: an existing project whose timezone is somehow blank.
    getSchool.mockResolvedValue(school({ timezone: "" }));
    render(<SchoolDetails onChanged={vi.fn()} />);
    const buttons = await screen.findAllByRole("button", { name: "Edit details" });
    await userEvent.click(buttons[buttons.length - 1]!);

    const saves = screen.getAllByRole("button", { name: "Save changes" });
    await userEvent.click(saves[saves.length - 1]!);

    expect(await screen.findByText("Time zone is required.")).toBeTruthy();
  });

  it("marks the offending picker as invalid for a screen reader", async () => {
    getSchool.mockResolvedValue(school({ locale: "" }));
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
    getSchool.mockResolvedValue(school({ locale: "" }));
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
