/**
 * The date field.
 *
 * `../dates.test.ts` owns the arithmetic. This file owns what only the control can get
 * wrong: that the two ways of entering a date produce the same value, that a half-typed
 * one is neither rewritten nor thrown away, and that the calendar is usable without a
 * mouse.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";

import { DateInput } from "./DateInput";

/**
 * A controlled host, the way `Field` uses it.
 *
 * Controlled on purpose: the value the component reports is fed straight back to it, so
 * these tests exercise the same round trip the real form does rather than a one-way
 * version of it that could not show a value being clobbered.
 */
function Harness(props: { initial?: string; onChange?: (value: string) => void }) {
  const [value, setValue] = useState(props.initial ?? "");
  return (
    <>
      <DateInput
        id="f-startsOn"
        label="First day"
        value={value}
        onChange={(next) => {
          setValue(next);
          props.onChange?.(next);
        }}
      />
      {/* Somewhere for focus to go, so blur and outside clicks can be tested. */}
      <button type="button">elsewhere</button>
      <div id="stored">{value}</div>
    </>
  );
}

const box = () => screen.getByRole("textbox") as HTMLInputElement;
const trigger = () =>
  screen.getByRole("button", { name: /choose first day from a calendar/i });
const dialog = () => screen.getByRole("dialog");
const monthSelect = () => within(dialog()).getByLabelText("Month") as HTMLSelectElement;
const yearSelect = () => within(dialog()).getByLabelText("Year") as HTMLSelectElement;
const focusedDay = () => document.activeElement?.getAttribute("aria-label");

/** What the harness is holding — the value a save would send. */
const stored = () => document.getElementById("stored")?.textContent ?? "";

describe("the placeholder", () => {
  it("names the format rather than showing an example date", () => {
    render(<Harness />);
    expect(box().getAttribute("placeholder")).toBe("DD/MM/YYYY");
  });

  it("leaves an empty field empty, so the placeholder is what shows", () => {
    render(<Harness />);
    expect(box().value).toBe("");
  });
});

describe("an existing record", () => {
  it("is shown in the display format", () => {
    render(<Harness initial="2027-09-01" />);
    expect(box().value).toBe("01/09/2027");
  });

  it("is not altered by being opened", () => {
    const onChange = vi.fn();
    render(<Harness initial="2027-09-01" onChange={onChange} />);
    // Nothing is normalised, re-derived or re-emitted on mount: a form opened and
    // cancelled cannot have moved a date.
    expect(onChange).not.toHaveBeenCalled();
    expect(stored()).toBe("2027-09-01");
  });

  it("survives the calendar being opened and dismissed", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Harness initial="2027-09-01" onChange={onChange} />);

    await user.click(trigger());
    await user.keyboard("{Escape}");

    expect(onChange).not.toHaveBeenCalled();
    expect(stored()).toBe("2027-09-01");
    expect(box().value).toBe("01/09/2027");
  });

  it("is replaced, not merged, when a new value arrives from outside", () => {
    // Re-rendering with a different value is what opening a second row for editing does.
    const { rerender } = render(
      <DateInput id="d" label="First day" value="2027-09-01" onChange={() => {}} />,
    );
    expect(box().value).toBe("01/09/2027");

    rerender(<DateInput id="d" label="First day" value="2026-12-25" onChange={() => {}} />);
    expect(box().value).toBe("25/12/2026");
  });
});

describe("typing a date", () => {
  it("stores the canonical form", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "01/09/2027");

    expect(stored()).toBe("2027-09-01");
  });

  it("does not reformat under the caret while the date is being typed", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "1/9/2027");

    // Still exactly what was typed. Padding it now would move the caret.
    expect(box().value).toBe("1/9/2027");
    expect(stored()).toBe("2027-09-01");
  });

  it("tidies a parsable value once the field is left", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "1/9/2027");
    await user.tab();

    expect(box().value).toBe("01/09/2027");
    expect(stored()).toBe("2027-09-01");
  });

  it("keeps a half-typed value instead of discarding it", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "01/09/202");

    expect(box().value).toBe("01/09/202");
    // Passed through rather than dropped or rounded, so the validator can object to it
    // and the user can still see what they typed.
    expect(stored()).toBe("01/09/202");
  });

  it("passes an impossible date through rather than correcting it", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "31/02/2026");

    // Emphatically not 2026-03-03, which is where a Date would have rolled it.
    expect(stored()).toBe("31/02/2026");
    expect(box().value).toBe("31/02/2026");
  });

  it("leaves an impossible date alone on blur too", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "31/02/2026");
    await user.tab();

    expect(box().value).toBe("31/02/2026");
    expect(stored()).toBe("31/02/2026");
  });

  it("accepts a leap day in a leap year", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "29/02/2028");

    expect(stored()).toBe("2028-02-29");
  });

  it("refuses a leap day in a year that has none", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(box(), "29/02/2027");

    // Not converted, so `isoDate` rejects it rather than the 1st of March being saved.
    expect(stored()).toBe("29/02/2027");
  });

  it("clears the stored value when the field is emptied", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.clear(box());

    expect(stored()).toBe("");
  });

  it("accepts a stored date pasted in as it is spelled", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(box());
    await user.paste("2027-09-01");
    await user.tab();

    expect(stored()).toBe("2027-09-01");
    expect(box().value).toBe("01/09/2027");
  });
});

describe("the calendar", () => {
  it("opens from the calendar button and says which field it belongs to", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(trigger().getAttribute("aria-expanded")).toBe("false");

    await user.click(trigger());

    expect(screen.getByRole("dialog", { name: "First day calendar" })).toBeTruthy();
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
    expect(trigger().getAttribute("aria-controls")).toBe(dialog().getAttribute("id"));
  });

  it("opens on the month of the date in the field", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());

    expect(monthSelect().value).toBe("9");
    expect(yearSelect().value).toBe("2027");
    expect(within(dialog()).getByRole("button", { name: "1 September 2027" })).toBeTruthy();
  });

  it("stores the canonical date when a day is clicked", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "17 September 2027" }));

    expect(stored()).toBe("2027-09-17");
    expect(box().value).toBe("17/09/2027");
    // Picking closes it and hands focus back, so the next keystroke goes to the field.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(box());
  });

  it("produces exactly the value typing the same date produces", async () => {
    const user = userEvent.setup();

    const first = render(<Harness initial="2027-09-15" />);
    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "1 September 2027" }));
    const fromCalendar = stored();
    first.unmount();

    render(<Harness />);
    await user.type(box(), "01/09/2027");
    const fromTyping = stored();

    expect(fromCalendar).toBe("2027-09-01");
    expect(fromTyping).toBe(fromCalendar);
  });

  it("marks the chosen day as selected", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());

    expect(
      screen.getByRole("button", { name: "1 September 2027" }).getAttribute("aria-selected"),
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: "2 September 2027" }).getAttribute("aria-selected"),
    ).toBe("false");
  });

  it("shows the days either side of the month, and they are selectable", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());
    // 1 September 2027 is a Wednesday, so the grid opens on Monday 30 August.
    await user.click(screen.getByRole("button", { name: "30 August 2027" }));

    expect(stored()).toBe("2027-08-30");
  });

  it("steps a month and a year at a time", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());

    await user.click(within(dialog()).getByRole("button", { name: "Next month" }));
    expect(monthSelect().value).toBe("10");

    await user.click(within(dialog()).getByRole("button", { name: "Previous month" }));
    expect(monthSelect().value).toBe("9");

    await user.click(within(dialog()).getByRole("button", { name: "Next year" }));
    expect(yearSelect().value).toBe("2028");

    await user.click(within(dialog()).getByRole("button", { name: "Previous year" }));
    expect(yearSelect().value).toBe("2027");
  });

  it("jumps to a month and a year with the selects", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    await user.selectOptions(monthSelect(), "2");
    await user.selectOptions(yearSelect(), "2030");

    expect(within(dialog()).getByRole("button", { name: "15 February 2030" })).toBeTruthy();
  });

  it("clamps the day when the month it jumps to is shorter", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-03-31" />);

    await user.click(trigger());
    await user.selectOptions(monthSelect(), "2");

    // Not the 3rd of March, which is where an unclamped jump would land.
    expect(within(dialog()).getByRole("button", { name: "28 February 2027" })).toBeTruthy();
    expect(monthSelect().value).toBe("2");
  });

  it("does not take focus off a select that is being used", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    const month = monthSelect();
    await user.selectOptions(month, "2");

    // Stepping through months with the keyboard would be impossible if choosing one
    // threw focus into the grid.
    expect(document.activeElement).toBe(month);
  });

  it("closes without changing anything when dismissed with Escape", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(stored()).toBe("2027-09-01");
    expect(document.activeElement).toBe(box());
  });

  it("closes when something outside it is clicked", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "elsewhere" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(stored()).toBe("2027-09-01");
  });

  it("is rendered outside the field, so no scrolling box can clip it", async () => {
    const user = userEvent.setup();
    const { container } = render(<Harness initial="2027-09-01" />);

    await user.click(trigger());

    expect(container.contains(dialog())).toBe(false);
    expect(dialog().parentElement).toBe(document.body);
  });

  it("is taken down when the form is disabled mid-edit", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <DateInput id="d" label="First day" value="2027-09-01" onChange={() => {}} />,
    );

    await user.click(screen.getByRole("button", { name: /choose first day/i }));
    expect(screen.getByRole("dialog")).toBeTruthy();

    // What a save in flight does: the fieldset disables, but the calendar is portalled
    // and would otherwise stay open and clickable over a form that is busy.
    rerender(
      <DateInput id="d" label="First day" value="2027-09-01" disabled onChange={() => {}} />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("keyboard operation", () => {
  it("opens with Alt+ArrowDown and puts focus on the date in the field", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-01" />);

    box().focus();
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(focusedDay()).toBe("1 September 2027");
  });

  it("moves a day at a time with the arrow keys", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    await user.keyboard("{ArrowRight}");
    expect(focusedDay()).toBe("16 September 2027");

    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(focusedDay()).toBe("14 September 2027");
  });

  it("moves a week at a time with ArrowUp and ArrowDown", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    await user.keyboard("{ArrowDown}");
    expect(focusedDay()).toBe("22 September 2027");

    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(focusedDay()).toBe("8 September 2027");
  });

  it("moves to the ends of the week with Home and End", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    // 15 September 2027 is a Wednesday, and the week starts on Monday.
    await user.keyboard("{Home}");
    expect(focusedDay()).toBe("13 September 2027");

    await user.keyboard("{End}");
    expect(focusedDay()).toBe("19 September 2027");
  });

  it("moves a month at a time with PageUp and PageDown, and a year with Shift", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());

    await user.keyboard("{PageDown}");
    expect(focusedDay()).toBe("15 October 2027");

    await user.keyboard("{PageUp}{PageUp}");
    expect(focusedDay()).toBe("15 August 2027");

    await user.keyboard("{Shift>}{PageDown}{/Shift}");
    expect(focusedDay()).toBe("15 August 2028");

    await user.keyboard("{Shift>}{PageUp}{/Shift}");
    expect(focusedDay()).toBe("15 August 2027");
  });

  it("crosses a month boundary by arrowing, and the grid follows", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-30" />);

    await user.click(trigger());
    await user.keyboard("{ArrowRight}");

    expect(focusedDay()).toBe("1 October 2027");
    expect(monthSelect().value).toBe("10");
  });

  it("selects the focused day with Enter", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    await user.keyboard("{ArrowRight}{Enter}");

    expect(stored()).toBe("2027-09-16");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("selects the focused day with Space", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());
    await user.keyboard("{ArrowDown} ");

    expect(stored()).toBe("2027-09-22");
  });

  it("is one tab stop, not forty-two", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());

    const days = [...screen.getByRole("grid").querySelectorAll("button")];
    const reachable = days.filter((day) => day.getAttribute("tabindex") === "0");
    expect(days).toHaveLength(42);
    expect(reachable).toHaveLength(1);
    expect(reachable[0]?.getAttribute("aria-label")).toBe("15 September 2027");
  });

  it("names every weekday heading in full", async () => {
    const user = userEvent.setup();
    render(<Harness initial="2027-09-15" />);

    await user.click(trigger());

    expect(
      screen.getAllByRole("columnheader").map((header) => header.getAttribute("aria-label")),
    ).toEqual([
      "Monday",
      "Tuesday",
      "Wednesday",
      "Thursday",
      "Friday",
      "Saturday",
      "Sunday",
    ]);
  });

  it("carries the field's error wiring onto the box a screen reader reads", () => {
    render(
      <DateInput
        id="d"
        label="First day"
        value="31/02/2026"
        aria-invalid
        aria-errormessage="d-error"
        aria-describedby="d-error"
        onChange={() => {}}
      />,
    );

    expect(box().getAttribute("aria-invalid")).toBe("true");
    expect(box().getAttribute("aria-errormessage")).toBe("d-error");
    expect(box().value).toBe("31/02/2026");
  });
});
