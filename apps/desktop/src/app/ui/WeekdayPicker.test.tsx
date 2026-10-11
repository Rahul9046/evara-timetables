/**
 * The weekday toggles show their abbreviation and nothing else.
 *
 * ## Why this test reads the stylesheet
 *
 * The defect it exists for was a stray `::after { content: ... }` on the selected day,
 * which put two junk characters after every chosen abbreviation. **No DOM test could have
 * caught it.** `styles.css` is imported by `main.tsx` and is never loaded under Vitest,
 * jsdom does not compute generated content, and `textContent` would not include it even
 * if it did.
 *
 * So the markup half is asserted against the DOM, and the generated-content half against
 * the stylesheet source, imported with `?raw`. The second half is the one that regresses.
 *
 * ## This file's source is deliberately all ASCII below the comments
 *
 * Every character being asserted about is built from its code point. The original mistake
 * was an escape mangled by a patch script, which is invisible in review precisely because
 * the result still looks like a plausible character; an assertion written with the
 * literal character, or with a `\u` escape, is vulnerable to exactly that.
 */
import { beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { WEEKDAYS } from "../setup/weekdays";
import type { Draft } from "../validation";
import { Field, type FieldSpec } from "./Field";

const SPEC: FieldSpec = {
  kind: "weekdays",
  key: "workingDays",
  label: "School working days",
  required: true,
};

/** The picker on its own, with no backend and no form around it. */
function renderPicker(values: string[]) {
  const draft: Draft = { workingDays: values };
  return render(
    <Field
      spec={SPEC}
      draft={draft}
      problems={{}}
      disabled={false}
      onChange={vi.fn()}
      idPrefix="form-school"
    />,
  );
}

/** Every toggle's visible text, in display order. */
function visibleLabels(): string[] {
  return screen
    .getAllByRole("checkbox")
    .map((box) => box.closest("label")?.querySelector("[aria-hidden]")?.textContent ?? "");
}

/** The two-letter form shown on one toggle. */
function shownOn(box: Element): string {
  return box.closest("label")?.querySelector("[aria-hidden]")?.textContent ?? "";
}

describe("the visible label", () => {
  it("is exactly the two-letter abbreviation, for every day", () => {
    renderPicker(["1", "2", "3", "4", "5"]);

    expect(visibleLabels()).toEqual(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]);
  });

  it("is the same whether the day is selected or not", async () => {
    const user = userEvent.setup();
    // Monday to Friday selected, Saturday and Sunday not, so both states are on screen in
    // one render. That is where an inconsistency between them would show.
    renderPicker(["1", "2", "3", "4", "5"]);

    const boxes = screen.getAllByRole("checkbox");
    const selected = boxes.filter((box) => (box as HTMLInputElement).checked);
    const unselected = boxes.filter((box) => !(box as HTMLInputElement).checked);
    expect(selected).toHaveLength(5);
    expect(unselected).toHaveLength(2);

    for (const box of boxes) {
      expect(shownOn(box)).toMatch(/^(Mo|Tu|We|Th|Fr|Sa|Su)$/);
    }

    // And selecting one does not change what it reads.
    await user.click(screen.getByRole("checkbox", { name: "Saturday" }));
    expect(visibleLabels()).toEqual(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]);
  });

  it("carries no superscript, tick, bullet or other decoration", () => {
    renderPicker(["1", "2", "3", "4", "5", "6", "7"]);

    // Superscript one, two and three; a tick; a middot. The first of these is what the
    // reported defect actually rendered.
    const decorations = [0x00b9, 0x00b2, 0x00b3, 0x2713, 0x00b7].map((code) =>
      String.fromCodePoint(code),
    );

    for (const shown of visibleLabels()) {
      expect(shown).toHaveLength(2);
      expect(shown).toMatch(/^[A-Za-z]{2}$/);
      for (const bad of [...decorations, "*"]) {
        expect(shown).not.toContain(bad);
      }
    }
  });

  it("still names each toggle by its full weekday for a screen reader", () => {
    renderPicker([]);

    for (const day of WEEKDAYS) {
      const box = screen.getByRole("checkbox", { name: day.label });
      expect(box).toBeTruthy();
      // The two-letter form is hidden from the accessibility tree, so it is never read out
      // as the control's name. That is why the label holds both forms.
      expect(box.closest("label")?.querySelector(".srOnly")?.textContent).toBe(day.label);
      expect(shownOn(box)).toBe(day.short);
    }
  });
});

/** The slice of `node:fs` this file needs. */
interface FsModule {
  readFileSync: (path: string, encoding: string) => string;
}

describe("the stylesheet", () => {
  /**
   * The stylesheet source, read off disk.
   *
   * Not `import css from "../../styles.css?raw"`: Vitest stubs CSS modules, and `?raw`
   * and `?inline` both resolve to the empty string under it. That would have made every
   * assertion below pass by checking nothing, which is why
   * "was parsed into something worth asserting against" exists.
   *
   * The specifier is held in a variable so TypeScript does not try to resolve it. That
   * keeps `@types/node` out of a frontend package whose `tsconfig` deliberately offers no
   * Node types — nothing in `src` may assume a Node runtime, and this is a test asking the
   * host to read a file rather than production code doing so.
   */
  let css = "";

  beforeAll(async () => {
    const specifier = "node:fs";
    const fs = (await import(/* @vite-ignore */ specifier)) as unknown as FsModule;
    // Relative to the Vite root, which is `apps/desktop`. A wrong root fails the guard
    // test below rather than silently passing the two real ones.
    css = fs.readFileSync("src/styles.css", "utf8");
  });

  /**
   * Every rule block, as `{ selector, body }`.
   *
   * A deliberately crude split on braces. It is enough because this stylesheet is flat,
   * with no nesting, and its only at-rules are `@media` blocks whose inner rules split
   * the same way. A real CSS parser would be a dependency for one assertion.
   */
  const rulesOf = (source: string) =>
    source
      .split("}")
      .map((chunk) => {
        const brace = chunk.indexOf("{");
        return brace < 0
          ? null
          : { selector: chunk.slice(0, brace).trim(), body: chunk.slice(brace + 1) };
      })
      .filter((rule): rule is { selector: string; body: string } => rule !== null);

  it("was parsed into something worth asserting against", () => {
    const rules = rulesOf(css);
    // Guards the crude split above. If this stylesheet ever stops splitting sensibly, the
    // two tests below would pass by finding nothing rather than by finding nothing wrong.
    expect(rules.length).toBeGreaterThan(100);
    expect(rules.some((rule) => rule.selector.includes(".weekdayToggle"))).toBe(true);
  });

  it("gives the weekday toggles no generated content in any state", () => {
    const offenders = rulesOf(css)
      .filter((rule) => rule.selector.includes("weekday"))
      .filter((rule) => /(^|[\s;{])content\s*:/.test(rule.body))
      .map((rule) => rule.selector);

    // The assertion the defect would have failed. A weekday label's text comes from
    // `WEEKDAYS[].short` in the markup and from nowhere else; a `::before` or `::after`
    // here puts characters on screen that no DOM test can see.
    expect(offenders).toEqual([]);
  });

  it("has no content declaration anywhere carrying a superscript artefact", () => {
    // The root cause was a CSS escape for a tick, U+2713, written into a patch script
    // whose own escape was mangled: what reached the file was U+00B9 followed by a literal
    // "3". A superscript is the signature of that class of mistake, so this guards the
    // whole stylesheet rather than only the weekday rules.
    //
    // Narrow on purpose. An earlier version of this test rejected the entire Latin-1
    // punctuation range and failed on `content: "\00b7"` — a middot used deliberately as
    // a separator elsewhere in this file, and a perfectly reasonable thing to generate. A
    // guard that cries wolf over legitimate characters gets deleted rather than heeded.
    const suspect = (character: string): boolean => {
      const code = character.codePointAt(0) ?? 0;
      // Superscript one, two and three: the Latin-1 strays a mangled escape lands on.
      const latin1Superscripts = code === 0x00b9 || code === 0x00b2 || code === 0x00b3;
      // The superscripts and subscripts block in full.
      const superscriptBlock = code >= 0x2070 && code <= 0x209f;
      return latin1Superscripts || superscriptBlock;
    };

    const suspicious = [...css.matchAll(/content\s*:\s*([^;]*);/g)]
      .map((match) => (match[1] ?? "").trim())
      .filter((value) => [...value].some(suspect));

    expect(suspicious).toEqual([]);
  });
});
