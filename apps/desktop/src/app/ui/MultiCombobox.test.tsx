/**
 * The searchable multi-select.
 *
 * Shares `matchesQuery` and the listbox mechanics with {@link Combobox}, whose own tests
 * cover those. What is specific here is that an option *toggles*, that the list stays open
 * while several are picked, and that what is already chosen can be removed from the
 * keyboard as well as the mouse.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MultiCombobox } from "./MultiCombobox";

const CAMPUSES = [
  { value: "north", label: "North (N)" },
  { value: "south", label: "South (S)" },
  { value: "east", label: "East (E)" },
];

function setup(values: string[] = []) {
  const onChange = vi.fn();
  render(
    <MultiCombobox
      id="shared"
      values={values}
      options={CAMPUSES}
      onChange={onChange}
      noneLabel="This campus only"
      placeholder="Search campuses"
    />,
  );
  return { onChange, input: screen.getByRole("combobox") };
}

const options = () => screen.getAllByRole("option");

describe("presentation", () => {
  it("says so plainly when nothing is chosen", () => {
    setup();
    expect(screen.getByText("This campus only")).toBeTruthy();
  });

  it("shows a chip per chosen value, labelled not raw", () => {
    setup(["north", "east"]);

    // Queried by the remove button's accessible name: the chip's own text also appears on
    // its option in the (closed but still rendered) listbox, so plain text is ambiguous.
    expect(screen.getByRole("button", { name: "Remove North (N)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove East (E)" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove South (S)" })).toBeNull();
    expect(screen.queryByText("This campus only")).toBeNull();
  });

  it("marks the chosen options as selected in the listbox", async () => {
    const { input } = setup(["south"]);
    await userEvent.click(input);

    const selected = options().filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0]?.textContent).toBe("South (S)");
  });

  it("declares the listbox multi-selectable", async () => {
    const { input } = setup();
    await userEvent.click(input);

    expect(screen.getByRole("listbox").getAttribute("aria-multiselectable")).toBe("true");
  });
});

describe("choosing", () => {
  it("adds a value on click, keeping the ones already there", async () => {
    const { input, onChange } = setup(["north"]);
    await userEvent.click(input);
    await userEvent.click(screen.getByText("South (S)"));

    expect(onChange).toHaveBeenCalledWith(["north", "south"]);
  });

  it("adds a value with Enter on the active option", async () => {
    const { input, onChange } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowDown}{Enter}");

    expect(onChange).toHaveBeenCalledWith(["south"]);
  });

  it("removes a value by picking it again", async () => {
    const { input, onChange } = setup(["north", "south"]);
    await userEvent.click(input);
    await userEvent.click(screen.getByRole("option", { name: "North (N)" }));

    expect(onChange).toHaveBeenCalledWith(["south"]);
  });

  it("keeps the list open after a pick, because picking several is the normal case", async () => {
    const { input } = setup();
    await userEvent.click(input);
    await userEvent.click(screen.getByText("South (S)"));

    expect(input.getAttribute("aria-expanded")).toBe("true");
  });

  it("clears the query after a pick so the next search starts fresh", async () => {
    const { input } = setup();
    await userEvent.type(input, "south");
    await userEvent.keyboard("{Enter}");

    expect((input as HTMLInputElement).value).toBe("");
  });

  it("searches, then picks what it found", async () => {
    const { input, onChange } = setup();
    await userEvent.type(input, "east");

    expect(options()).toHaveLength(1);
    await userEvent.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalledWith(["east"]);
  });

  it("says so when nothing matches", async () => {
    const { input } = setup();
    await userEvent.type(input, "west");

    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("No matches")).toBeTruthy();
  });
});

describe("removing", () => {
  it("removes a value from its chip, by a named button", async () => {
    const { onChange } = setup(["north", "south"]);

    // Not an unlabelled "×": a row of identical remove buttons is unusable by ear.
    await userEvent.click(screen.getByRole("button", { name: "Remove North (N)" }));

    expect(onChange).toHaveBeenCalledWith(["south"]);
  });

  it("removes the last value with Backspace on an empty query", async () => {
    const { input, onChange } = setup(["north", "south"]);
    input.focus();
    await userEvent.keyboard("{Backspace}");

    expect(onChange).toHaveBeenCalledWith(["north"]);
  });

  it("leaves Backspace alone while there is a query to edit", async () => {
    const { input, onChange } = setup(["north"]);
    await userEvent.type(input, "sou");
    await userEvent.keyboard("{Backspace}");

    expect(onChange).not.toHaveBeenCalled();
    expect((input as HTMLInputElement).value).toBe("so");
  });

  it("does nothing on Backspace when there is nothing chosen", async () => {
    const { input, onChange } = setup();
    input.focus();
    await userEvent.keyboard("{Backspace}");

    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("keyboard and accessibility", () => {
  it("opens on ArrowDown", async () => {
    const { input } = setup();
    input.focus();
    await userEvent.keyboard("{ArrowDown}");

    expect(input.getAttribute("aria-expanded")).toBe("true");
  });

  it("wraps at both ends", async () => {
    const { input, onChange } = setup();
    await userEvent.click(input);
    // Open on the first option, then step back past the top onto the last.
    await userEvent.keyboard("{ArrowUp}{Enter}");

    expect(onChange).toHaveBeenCalledWith(["east"]);
  });

  it("jumps with Home and End", async () => {
    const { input, onChange } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{End}{Enter}");

    expect(onChange).toHaveBeenLastCalledWith(["east"]);
  });

  it("closes on Escape without changing anything", async () => {
    const { input, onChange } = setup(["north"]);
    await userEvent.type(input, "south");
    await userEvent.keyboard("{Escape}");

    expect(onChange).not.toHaveBeenCalled();
    expect(input.getAttribute("aria-expanded")).toBe("false");
  });

  it("points aria-activedescendant at a real option while open", async () => {
    const { input } = setup();
    await userEvent.click(input);

    const id = input.getAttribute("aria-activedescendant");
    expect(id).toBeTruthy();
    expect(document.getElementById(id!)?.getAttribute("role")).toBe("option");
  });

  it("announces the counts for a screen reader", async () => {
    const { input } = setup(["north"]);
    await userEvent.type(input, "south");

    expect(screen.getByRole("status").textContent).toBe("1 of 3 options, 1 chosen");
  });

  it("leaves Enter as the form's submit key while closed", async () => {
    const submit = vi.fn((e: React.FormEvent) => e.preventDefault());
    render(
      <form onSubmit={submit}>
        <MultiCombobox id="in-form" values={[]} options={CAMPUSES} onChange={vi.fn()} />
      </form>,
    );

    const input = screen.getByRole("combobox");
    input.focus();
    await userEvent.keyboard("{Enter}");

    expect(submit).toHaveBeenCalled();
  });

  it("swallows Enter while open, so picking does not submit the form", async () => {
    const submit = vi.fn((e: React.FormEvent) => e.preventDefault());
    const onChange = vi.fn();
    render(
      <form onSubmit={submit}>
        <MultiCombobox id="in-form" values={[]} options={CAMPUSES} onChange={onChange} />
      </form>,
    );

    const input = screen.getByRole("combobox");
    await userEvent.click(input);
    await userEvent.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();
  });
});
