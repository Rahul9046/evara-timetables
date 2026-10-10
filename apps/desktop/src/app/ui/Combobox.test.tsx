/**
 * The searchable select.
 *
 * Two of its properties are load-bearing and are what most of this file is about:
 *
 * - **the value changes only on an explicit selection.** Typing is searching. A form
 *   that let a half-typed query become the stored timezone would let a stray keystroke
 *   silently repoint every period time in a school.
 * - **it is operable from the keyboard alone**, with the ARIA wiring a screen reader
 *   needs to follow along — which is structural here, not decorative, so it is asserted
 *   rather than assumed.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Combobox, matchesQuery, withExistingValue } from "./Combobox";

const OPTIONS = [
  { value: "Asia/Kolkata", label: "India Standard Time — Asia/Kolkata" },
  { value: "Europe/London", label: "United Kingdom Time — Europe/London" },
  { value: "America/New_York", label: "Eastern Time — America/New_York" },
  { value: "Australia/Sydney", label: "Australian Eastern Time — Australia/Sydney" },
];

function setup(value = "", options = OPTIONS) {
  const onSelect = vi.fn();
  render(
    <Combobox id="tz" value={value} options={options} onSelect={onSelect} />,
  );
  return { onSelect, input: screen.getByRole("combobox") };
}

const listbox = () => screen.getByRole("listbox", { hidden: true });
const options = () => screen.getAllByRole("option");

describe("presentation", () => {
  it("shows the selected option's label, not its identifier", () => {
    const { input } = setup("Asia/Kolkata");

    expect((input as HTMLInputElement).value).toBe("India Standard Time — Asia/Kolkata");
  });

  it("starts closed, with the list hidden", () => {
    const { input } = setup("Asia/Kolkata");

    expect(input.getAttribute("aria-expanded")).toBe("false");
    expect(listbox().hasAttribute("hidden")).toBe(true);
  });

  it("marks the stored value as the selected option", async () => {
    const { input } = setup("Europe/London");
    await userEvent.click(input);

    const selected = options().filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0]?.textContent).toContain("Europe/London");
  });
});

describe("searching", () => {
  it("filters to options matching the query", async () => {
    const { input } = setup();
    await userEvent.type(input, "kolkata");

    expect(options()).toHaveLength(1);
    expect(options()[0]?.textContent).toContain("Asia/Kolkata");
  });

  it("matches the human-readable name as well as the identifier", async () => {
    const { input } = setup();
    await userEvent.type(input, "india standard");

    expect(options()).toHaveLength(1);
    expect(options()[0]?.textContent).toContain("Asia/Kolkata");
  });

  it("finds a city without the continent prefix, underscore and all", async () => {
    const { input } = setup();
    await userEvent.type(input, "new york");

    expect(options()).toHaveLength(1);
    expect(options()[0]?.textContent).toContain("America/New_York");
  });

  it("says so when nothing matches, rather than showing an empty box", async () => {
    const { input } = setup();
    await userEvent.type(input, "atlantis");

    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("No matches")).toBeTruthy();
  });

  it("shows the whole list when opened by click, not just the selected row", async () => {
    const { input } = setup("Asia/Kolkata");
    await userEvent.click(input);

    // The input reads "India Standard Time — Asia/Kolkata"; if that were treated as a
    // query the list would hold exactly one row and browsing would be impossible.
    expect(options()).toHaveLength(OPTIONS.length);
  });

  it("announces how many options the query left", async () => {
    const { input } = setup();
    await userEvent.type(input, "kolkata");

    expect(screen.getByRole("status").textContent).toBe("1 of 4 options");
  });
});

describe("typing never changes the value", () => {
  it("does not select while the user types", async () => {
    const { input, onSelect } = setup("Europe/London");
    await userEvent.type(input, "kolkata");

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("does not select even when the query leaves exactly one match", async () => {
    const { input, onSelect } = setup();
    await userEvent.type(input, "Asia/Kolkata");

    expect(options()).toHaveLength(1);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("restores the stored selection when the query is abandoned with Escape", async () => {
    const { input, onSelect } = setup("Europe/London");
    await userEvent.type(input, "kolkata");
    await userEvent.keyboard("{Escape}");

    expect(onSelect).not.toHaveBeenCalled();
    expect((input as HTMLInputElement).value).toBe("United Kingdom Time — Europe/London");
    expect(input.getAttribute("aria-expanded")).toBe("false");
  });

  it("restores the stored selection when focus leaves", async () => {
    const { input, onSelect } = setup("Europe/London");
    await userEvent.type(input, "sydney");
    await userEvent.tab();

    expect(onSelect).not.toHaveBeenCalled();
    expect((input as HTMLInputElement).value).toBe("United Kingdom Time — Europe/London");
  });
});

/**
 * `userEvent.type` clicks the element before typing, and a click opens the list — which
 * is correct behaviour but makes "what happens on a *closed* box" untestable through it.
 * Focusing directly gives a focused, still-closed combobox to press keys at.
 */
function focusClosed(input: HTMLElement): void {
  input.focus();
}

describe("keyboard", () => {
  it("opens on ArrowDown and lands on the current selection", async () => {
    const { input } = setup("America/New_York");
    focusClosed(input);
    await userEvent.keyboard("{ArrowDown}");

    expect(input.getAttribute("aria-expanded")).toBe("true");
    const active = document.getElementById(input.getAttribute("aria-activedescendant")!);
    expect(active?.textContent).toContain("America/New_York");
  });

  it("opens on ArrowUp as well", async () => {
    const { input } = setup("Europe/London");
    focusClosed(input);
    await userEvent.keyboard("{ArrowUp}");

    expect(input.getAttribute("aria-expanded")).toBe("true");
  });

  it("selects the active option with Enter", async () => {
    const { input, onSelect } = setup();
    // Click opens on the current selection — index 0 while nothing is chosen.
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowDown}{Enter}");

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(OPTIONS[1]?.value);
  });

  it("selects a searched-for option with Enter, storing the identifier", async () => {
    const { input, onSelect } = setup();
    await userEvent.type(input, "sydney{Enter}");

    expect(onSelect).toHaveBeenCalledWith("Australia/Sydney");
  });

  it("moves with ArrowUp and wraps at the ends", async () => {
    const { input, onSelect } = setup();
    await userEvent.click(input);
    // From the first row, back past the top onto the last.
    await userEvent.keyboard("{ArrowUp}{Enter}");

    expect(onSelect).toHaveBeenCalledWith("Australia/Sydney");
  });

  it("wraps forward off the end onto the first", async () => {
    const { input, onSelect } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{End}{ArrowDown}{Enter}");

    expect(onSelect).toHaveBeenCalledWith("Asia/Kolkata");
  });

  it("jumps to the first and last options with Home and End", async () => {
    const { input, onSelect } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{End}{Enter}");
    expect(onSelect).toHaveBeenLastCalledWith("Australia/Sydney");

    await userEvent.click(input);
    await userEvent.keyboard("{Home}{Enter}");
    expect(onSelect).toHaveBeenLastCalledWith("Asia/Kolkata");
  });

  it("closes on Enter so one keypress does not both select and resubmit", async () => {
    const { input } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{Enter}");

    expect(input.getAttribute("aria-expanded")).toBe("false");
  });

  it("leaves Enter alone when closed, so the form can still submit", async () => {
    const submit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={submit}>
        <Combobox id="in-form" value="" options={OPTIONS} onSelect={vi.fn()} />
      </form>,
    );

    const input = screen.getByRole("combobox");
    focusClosed(input);
    await userEvent.keyboard("{Enter}");

    expect(submit).toHaveBeenCalled();
  });

  it("swallows Enter while open, so selecting does not submit the form", async () => {
    const submit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const onSelect = vi.fn();
    render(
      <form onSubmit={submit}>
        <Combobox id="in-form" value="" options={OPTIONS} onSelect={onSelect} />
      </form>,
    );

    const input = screen.getByRole("combobox");
    await userEvent.click(input);
    await userEvent.keyboard("{Enter}");

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(submit).not.toHaveBeenCalled();
  });

  it("keeps the active row inside the list as the query narrows it", async () => {
    const { input, onSelect } = setup();
    await userEvent.click(input);
    // Move to the last row, then type a query that leaves only the first.
    await userEvent.keyboard("{End}");
    await userEvent.type(input, "kolkata{Enter}");

    // Had the index stayed at 3 there would have been nothing to commit.
    expect(onSelect).toHaveBeenCalledWith("Asia/Kolkata");
  });
});

describe("mouse", () => {
  it("selects the clicked option and stores its identifier", async () => {
    const { input, onSelect } = setup();
    await userEvent.click(input);
    await userEvent.click(screen.getByText("Eastern Time — America/New_York"));

    expect(onSelect).toHaveBeenCalledWith("America/New_York");
  });
});

describe("accessibility wiring", () => {
  it("declares itself a combobox controlling its listbox", () => {
    const { input } = setup();

    expect(input.getAttribute("role")).toBe("combobox");
    expect(input.getAttribute("aria-autocomplete")).toBe("list");
    expect(input.getAttribute("aria-controls")).toBe(listbox().getAttribute("id"));
  });

  it("points aria-activedescendant at a real option while open", async () => {
    const { input } = setup();
    await userEvent.type(input, "{ArrowDown}");

    const id = input.getAttribute("aria-activedescendant");
    expect(id).toBeTruthy();
    const active = document.getElementById(id!);
    expect(active?.getAttribute("role")).toBe("option");
  });

  it("drops aria-activedescendant when closed", () => {
    const { input } = setup("Asia/Kolkata");

    expect(input.getAttribute("aria-activedescendant")).toBeNull();
  });

  it("carries the error wiring the field gave it", () => {
    render(
      <Combobox
        id="tz"
        value=""
        options={OPTIONS}
        onSelect={vi.fn()}
        aria-invalid={true}
        aria-errormessage="tz-error"
        aria-describedby="tz-hint"
      />,
    );

    const input = screen.getByRole("combobox");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-errormessage")).toBe("tz-error");
    // The field's hint, plus the combobox's own result-count status.
    expect(input.getAttribute("aria-describedby")).toContain("tz-hint");
  });
});

describe("matchesQuery", () => {
  const choice = OPTIONS[0]!;

  it("accepts everything for an empty query", () => {
    expect(matchesQuery(choice, "")).toBe(true);
    expect(matchesQuery(choice, "   ")).toBe(true);
  });

  it("ignores term order", () => {
    expect(matchesQuery(choice, "kolkata india")).toBe(true);
    expect(matchesQuery(choice, "india kolkata")).toBe(true);
  });

  it("requires every term to match", () => {
    expect(matchesQuery(choice, "india london")).toBe(false);
  });

  it("is case-insensitive without being locale-sensitive", () => {
    // Were this `toLocaleLowerCase`, a Turkish locale would dot-strip the I and this
    // would stop matching.
    expect(matchesQuery(choice, "INDIA")).toBe(true);
  });
});

describe("withExistingValue", () => {
  it("leaves the list alone when the value is already in it", () => {
    expect(withExistingValue(OPTIONS, "Asia/Kolkata")).toBe(OPTIONS);
  });

  it("leaves the list alone when there is no value yet", () => {
    expect(withExistingValue(OPTIONS, "")).toBe(OPTIONS);
    expect(withExistingValue(OPTIONS, null)).toBe(OPTIONS);
    expect(withExistingValue(OPTIONS, undefined)).toBe(OPTIONS);
  });

  it("prepends an unknown value, labelled as what it is", () => {
    const withLegacy = withExistingValue(OPTIONS, "Mars/Olympus_Mons");

    expect(withLegacy).toHaveLength(OPTIONS.length + 1);
    expect(withLegacy[0]?.value).toBe("Mars/Olympus_Mons");
    expect(withLegacy[0]?.label).toBe(
      "Mars/Olympus_Mons — existing value, not in the list",
    );
  });

  it("lets the caller build the label, since it knows more about the value", () => {
    const withLegacy = withExistingValue(
      OPTIONS,
      "Mars/Olympus_Mons",
      (value) => `${value} (saved earlier)`,
    );

    expect(withLegacy[0]?.label).toBe("Mars/Olympus_Mons (saved earlier)");
  });
});
