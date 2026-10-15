/**
 * The import screen's safety behaviour, tested at the component level.
 *
 * `tests_import.rs` already proves the backend cannot be made to import the wrong thing.
 * These tests prove the *screen* does not undo any of it, because four of the guarantees
 * are only half in Rust:
 *
 * - **Preview is a separate button from Import**, and nothing is sent to `import_apply`
 *   until the user presses the second one.
 * - **Apply is unavailable while any row is unanswered.** The backend refuses a blocked
 *   plan, but a button that offers an action certain to fail is a lie, and the reason has
 *   to be reachable — so it is in the button's `title` and not only in a colour.
 * - **A conflicted row is re-previewed when it is answered**, not patched locally: an
 *   answer can change another row's outcome, and the fingerprint must belong to the plan
 *   the backend computed.
 * - **A `reviewRequired` refusal re-reads the plan and says nothing was imported**, rather
 *   than reporting a failure the user cannot act on.
 *
 * Two more are about what the screen *sends*: the school is the selected one on every
 * call, and the fingerprint applied is the one the preview handed over.
 *
 * `invoke` is mocked; there is no Tauri runtime under Vitest. `plugin-dialog` is mocked
 * too — a save dialogue has no headless form.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const saveDialog = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveDialog }));

const { ImportData } = await import("./ImportData");

const SCHOOL = {
  id: "school-1",
  name: "Northgate Secondary",
  code: null,
  timezone: "Europe/London",
  locale: "en-GB",
  workingDays: [1, 2, 3, 4, 5],
  createdAt: "2027-01-01T00:00:00.000Z",
  updatedAt: "2027-01-01T00:00:00.000Z",
  rev: 1,
};

const TEACHER_FIELDS = [
  { key: "code", label: "Teacher Code", required: false, hint: "Optional, and what makes an import repeatable." },
  { key: "givenName", label: "First Name", required: true, hint: "Required." },
  { key: "familyName", label: "Last Name", required: true, hint: "Required." },
];

const KINDS = [
  {
    kind: "teachers",
    label: "Teachers",
    dependsOn: null,
    fields: TEACHER_FIELDS,
  },
  {
    kind: "classEnrolments",
    label: "Class enrolments",
    dependsOn: "class sections, and students enrolled here",
    fields: [
      { key: "courseCode", label: "Course Code", required: true, hint: "Required." },
      { key: "sectionCode", label: "Section Code", required: true, hint: "Required." },
      { key: "studentCode", label: "Student Code", required: true, hint: "Required." },
    ],
  },
];

/** A read file, as `import_begin` answers. */
function source(overrides: Record<string, unknown> = {}) {
  return {
    kind: "teachers",
    fileName: "staff.xlsx",
    sheet: "Staff",
    availableSheets: ["Staff"],
    headers: ["Teacher Code", "First Name", "Last Name"],
    rowCount: 2,
    truncatedAt: null,
    fields: TEACHER_FIELDS,
    suggestedMapping: { code: 0, givenName: 1, familyName: 2 },
    sample: [
      { line: 2, cells: ["T-0007", "Ada", "Lovelace"] },
      { line: 3, cells: ["T-0008", "Grace", "Hopper"] },
    ],
    ...overrides,
  };
}

/** A clean two-row plan. */
function cleanPlan(fingerprint = "aaaaaaaaaaaaaaaa") {
  return {
    kind: "teachers",
    schoolId: "school-1",
    rows: [
      { line: 2, action: "create", sourceLabel: "T-0007 — Ada Lovelace", targetLabel: null, findings: [] },
      { line: 3, action: "create", sourceLabel: "T-0008 — Grace Hopper", targetLabel: null, findings: [] },
    ],
    counts: {
      create: 2,
      update: 0,
      unchanged: 0,
      skipped: 0,
      conflict: 0,
      invalid: 0,
      warnings: 0,
    },
    fingerprint,
  };
}

/** A plan with one row the planner would not decide alone. */
function conflictedPlan(fingerprint = "bbbbbbbbbbbbbbbb") {
  return {
    kind: "teachers",
    schoolId: "school-1",
    rows: [
      { line: 2, action: "create", sourceLabel: "T-0007 — Ada Lovelace", targetLabel: null, findings: [] },
      {
        line: 3,
        action: "conflict",
        sourceLabel: "Grace Hopper",
        targetLabel: "Grace Hopper",
        findings: [
          {
            severity: "error",
            field: null,
            message: "a teacher of this school already has this name, and the row has no staff code.",
          },
        ],
      },
    ],
    counts: {
      create: 1,
      update: 0,
      unchanged: 0,
      skipped: 0,
      conflict: 1,
      invalid: 0,
      warnings: 0,
    },
    fingerprint,
  };
}

/** Routes each mocked command to a canned answer. */
function route(handlers: Record<string, unknown>) {
  invoke.mockImplementation((command: string, args?: unknown) => {
    const handler = handlers[command];
    if (typeof handler === "function") {
      return Promise.resolve((handler as (a?: unknown) => unknown)(args));
    }
    if (handler === undefined) return Promise.resolve(null);
    return Promise.resolve(handler);
  });
}

const BASE = {
  import_kinds: KINDS,
  import_current: null,
  import_cancel: false,
};

beforeEach(() => {
  invoke.mockReset();
  saveDialog.mockReset();
});

/** Renders, waits for the type chooser, and hands over a file. */
async function withFile(handlers: Record<string, unknown> = {}) {
  route({ ...BASE, import_begin: () => source(), ...handlers });
  const user = userEvent.setup();
  render(<ImportData school={SCHOOL} />);
  await screen.findByLabelText("Record type");

  const input = document.querySelector<HTMLInputElement>("#import-file");
  if (!input) throw new Error("expected a file input");
  await user.upload(input, new File(["Teacher Code,First Name,Last Name\n"], "staff.xlsx"));
  await screen.findByText(/rows on sheet/);
  return user;
}

/** The arguments of the one call to a command. */
function callTo(command: string): Record<string, unknown> {
  const call = invoke.mock.calls.find(([name]) => name === command);
  if (!call) throw new Error(`${command} was never called`);
  return (call[1] ?? {}) as Record<string, unknown>;
}

describe("before a file is chosen", () => {
  it("offers every import type and says what each one needs first", async () => {
    route(BASE);
    const user = userEvent.setup();
    render(<ImportData school={SCHOOL} />);

    const chooser = await screen.findByLabelText("Record type");
    expect(within(chooser).getByRole("option", { name: "Teachers" })).toBeTruthy();

    await user.selectOptions(chooser, "classEnrolments");
    // A school importing in the wrong order should be told, not left to read two hundred
    // identical row errors.
    expect(await screen.findByText(/students enrolled here/)).toBeTruthy();
  });

  it("does not offer a preview, because there is nothing to preview", async () => {
    route(BASE);
    render(<ImportData school={SCHOOL} />);
    await screen.findByLabelText("Record type");

    expect(screen.queryByRole("button", { name: /^Preview/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Import \d/ })).toBeNull();
  });

  it("promises in words that nothing is written and nothing is overwritten", async () => {
    route(BASE);
    render(<ImportData school={SCHOOL} />);
    await screen.findByLabelText("Record type");

    // These are the two questions a school asks before trying this at all, so they are
    // answered before the first button rather than in a tooltip somewhere.
    expect(screen.getByText(/Nothing is written until you say so/)).toBeTruthy();
    expect(screen.getByText(/never overwritten silently/)).toBeTruthy();
    expect(screen.getByText(/All or nothing/)).toBeTruthy();
  });

  it("saves a template through a save dialogue, sending no content of its own", async () => {
    route({ ...BASE, import_template: "evara-teachers-template.xlsx", import_save: "evara-teachers-template.xlsx" });
    saveDialog.mockResolvedValue("C:/somewhere/evara-teachers-template.xlsx");
    const user = userEvent.setup();
    render(<ImportData school={SCHOOL} />);
    await screen.findByLabelText("Record type");

    await user.click(screen.getByRole("button", { name: /template \(Excel\)/ }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_save", expect.anything()));
    // The path and nothing else: the bytes are the backend's own, so there is no way for
    // this window to make it write something else.
    expect(callTo("import_save")).toEqual({ path: "C:/somewhere/evara-teachers-template.xlsx" });
    expect(await screen.findByText(/evara-teachers-template\.xlsx saved/)).toBeTruthy();
  });

  it("writes nothing when the save dialogue is dismissed", async () => {
    route({ ...BASE, import_template: "evara-teachers-template.csv" });
    saveDialog.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<ImportData school={SCHOOL} />);
    await screen.findByLabelText("Record type");

    await user.click(screen.getByRole("button", { name: /template \(CSV\)/ }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_template", expect.anything()));
    expect(invoke.mock.calls.some(([name]) => name === "import_save")).toBe(false);
  });
});

describe("choosing a file", () => {
  it("sends the bytes once and shows what was read", async () => {
    await withFile();

    expect(callTo("import_begin")).toMatchObject({
      kind: "teachers",
      fileName: "staff.xlsx",
    });
    expect(typeof callTo("import_begin")["base64"]).toBe("string");
    expect(screen.getByText(/2 rows on sheet/)).toBeTruthy();
    // The mapper's sample, so a wrong mapping is visible before two hundred rows are
    // planned against it.
    expect(screen.getByText("T-0007")).toBeTruthy();
  });

  it("suggests a mapping from the headings and lets every field be unmapped", async () => {
    await withFile();

    const code = screen.getByLabelText("Column for Teacher Code") as HTMLSelectElement;
    expect(code.value).toBe("0");
    // "Not in this file" is the answer that means "leave this field alone", which is
    // different from mapping a blank column. Both have to be expressible.
    expect(within(code).getByRole("option", { name: "Not in this file" })).toBeTruthy();
  });

  it("says so when the file was longer than Evara reads, naming the row", async () => {
    await withFile({ import_begin: () => source({ truncatedAt: 20_001, rowCount: 20_000 }) });

    // Silent truncation would be the worst possible failure of this feature: the school
    // would believe it had imported everybody.
    const banner = await screen.findByText(/stops at row 20001/);
    expect(banner.textContent).toMatch(/Split it/);
  });

  it("offers the other worksheets of a workbook without re-sending the file", async () => {
    const user = await withFile({
      import_begin: () => source({ availableSheets: ["Staff", "Notes"] }),
      import_sheet: () => source({ availableSheets: ["Staff", "Notes"], sheet: "Notes" }),
    });

    await user.selectOptions(await screen.findByLabelText("Worksheet"), "Notes");

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_sheet", { sheet: "Notes" }));
    // `import_begin` is what carries the bytes, so it must not be called again.
    expect(invoke.mock.calls.filter(([name]) => name === "import_begin")).toHaveLength(1);
  });

  it("forgets the file when the type changes, because a mapping is of one type's fields", async () => {
    const user = await withFile();

    await user.selectOptions(screen.getByLabelText("Record type"), "classEnrolments");

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_cancel"));
    expect(screen.queryByText(/rows on sheet/)).toBeNull();
  });
});

describe("previewing", () => {
  it("writes nothing: apply is not called until the second button is pressed", async () => {
    const user = await withFile({ import_preview: () => cleanPlan() });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await screen.findByText(/to create/);

    expect(invoke.mock.calls.some(([name]) => name === "import_apply")).toBe(false);
    expect(callTo("import_preview")).toMatchObject({ schoolId: "school-1" });
  });

  it("shows the tally the backend computed, and does not compute one of its own", async () => {
    const user = await withFile({
      import_preview: () =>
        cleanPlan() &&
        ({
          ...cleanPlan(),
          // Deliberately inconsistent with `rows`: the screen must render the backend's
          // answer rather than counting the rows itself, because the backend's is the one
          // the fingerprint and the Apply refusal are computed from.
          counts: {
            create: 7,
            update: 3,
            unchanged: 1,
            skipped: 0,
            conflict: 0,
            invalid: 0,
            warnings: 2,
          },
        } as unknown),
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));

    const counts = await screen.findByText(/to create/);
    expect(counts.textContent).toContain("7");
    expect(screen.getByRole("button", { name: /^Import 10 records/ })).toBeTruthy();
  });

  it("re-previews when a conflicted row is answered, rather than patching it locally", async () => {
    const plans = [conflictedPlan(), cleanPlan("cccccccccccccccc")];
    const user = await withFile({
      import_preview: () => plans.shift() ?? cleanPlan(),
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await screen.findByRole("heading", { name: /Rows that need a decision/ });
    await user.click(screen.getByRole("button", { name: "Skip this row" }));

    await waitFor(() =>
      expect(invoke.mock.calls.filter(([name]) => name === "import_preview")).toHaveLength(2),
    );
    const second = invoke.mock.calls.filter(([name]) => name === "import_preview")[1];
    expect((second?.[1] as { decisions: unknown }).decisions).toEqual([
      { line: 3, choice: "skip" },
    ]);
  });

  it("offers all four answers to a conflicted row, with none of them preferred", async () => {
    const user = await withFile({ import_preview: () => conflictedPlan() });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await screen.findByRole("heading", { name: /Rows that need a decision/ });

    // Evara has no opinion about which is right, so none is a default and none is styled
    // as the expected answer.
    for (const label of [
      "Use this file",
      "Keep what Evara has",
      "Create a separate record",
      "Skip this row",
    ]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
    // And the reason is shown next to the row, not in a banner about the file.
    expect(screen.getByText(/already has this name/)).toBeTruthy();
  });

  it("refuses to offer Import while a row is unanswered, and says why", async () => {
    const user = await withFile({ import_preview: () => conflictedPlan() });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));

    const apply = await screen.findByRole("button", { name: /^Import 1 record/ });
    expect(apply.hasAttribute("disabled")).toBe(true);
    // In the title, so it is reachable by keyboard and by a screen reader: a colour on a
    // count is not an explanation.
    expect(apply.getAttribute("title")).toMatch(/Answer the rows/);
  });

  it("says there is nothing to do rather than offering an empty import", async () => {
    const user = await withFile({
      import_preview: () => ({
        ...cleanPlan(),
        rows: [
          {
            line: 2,
            action: "unchanged",
            sourceLabel: "T-0007 — Ada Lovelace",
            targetLabel: "Ada Lovelace",
            findings: [],
          },
        ],
        counts: {
          create: 0,
          update: 0,
          unchanged: 1,
          skipped: 0,
          conflict: 0,
          invalid: 0,
          warnings: 0,
        },
      }),
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));

    expect(await screen.findByText(/every row already matches/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Import 0 records/ }).hasAttribute("disabled")).toBe(
      true,
    );
  });
});

describe("importing", () => {
  it("applies the fingerprint the preview handed over", async () => {
    const user = await withFile({
      import_preview: () => cleanPlan("deadbeefdeadbeef"),
      import_apply: () => ({
        kind: "teachers",
        created: 2,
        updated: 0,
        unchanged: 0,
        skipped: 0,
        warnings: 0,
        rowsProcessed: 2,
        rows: [],
      }),
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await user.click(await screen.findByRole("button", { name: /^Import 2 records/ }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_apply", expect.anything()));
    expect(callTo("import_apply")).toMatchObject({
      schoolId: "school-1",
      fingerprint: "deadbeefdeadbeef",
    });
    expect(await screen.findByText(/2 created/)).toBeTruthy();
  });

  it("re-reads the plan and says nothing was imported when it had moved", async () => {
    const user = await withFile({
      import_preview: () => cleanPlan(),
      import_apply: () => {
        throw { kind: "reviewRequired", message: "the plan moved" };
      },
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await user.click(await screen.findByRole("button", { name: /^Import 2 records/ }));

    // Not an error: nothing was written, and the next step is to look again rather than to
    // report a problem.
    const banner = await screen.findByText(/nothing was imported/);
    expect(banner.textContent).toMatch(/review it and apply again/i);
    await waitFor(() =>
      expect(invoke.mock.calls.filter(([name]) => name === "import_preview")).toHaveLength(2),
    );
  });

  it("reports any other refusal in the backend's own words", async () => {
    const user = await withFile({
      import_preview: () => cleanPlan(),
      import_apply: () => {
        throw { kind: "duplicate", entity: "teacher", field: "code", message: "Another teacher already uses that code." };
      },
    });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await user.click(await screen.findByRole("button", { name: /^Import 2 records/ }));

    expect(await screen.findByText("Another teacher already uses that code.")).toBeTruthy();
  });

  it("offers the report afterwards, rendered by the backend from what happened", async () => {
    const user = await withFile({
      import_preview: () => cleanPlan(),
      import_apply: () => ({
        kind: "teachers",
        created: 1,
        updated: 1,
        unchanged: 0,
        skipped: 0,
        warnings: 1,
        rowsProcessed: 2,
        rows: [],
      }),
      import_report: "evara-teachers-import-report.csv",
      import_save: "evara-teachers-import-report.csv",
    });
    saveDialog.mockResolvedValue("C:/somewhere/report.csv");

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await user.click(await screen.findByRole("button", { name: /^Import 2 records/ }));
    await user.click(await screen.findByRole("button", { name: "Save the report" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("import_report"));
    // No summary is sent back: the report is rendered from the backend's own record, so
    // the file a school keeps cannot disagree with what Evara did.
    expect(callTo("import_report")).toEqual({});
    expect(callTo("import_save")).toEqual({ path: "C:/somewhere/report.csv" });
  });
});

describe("the school this imports into", () => {
  it("is the selected one on every call, and is never read from the file", async () => {
    const user = await withFile({ import_preview: () => cleanPlan() });

    await user.click(screen.getByRole("button", { name: /^Preview/ }));
    await screen.findByText(/to create/);

    for (const [name, args] of invoke.mock.calls) {
      if (name === "import_preview" || name === "import_apply") {
        expect((args as { schoolId: string }).schoolId).toBe("school-1");
      }
    }
    // And nothing in the file-reading call names a school at all: `import_begin` is about
    // bytes, and a column saying "Southmoor" must not scope anything.
    expect(callTo("import_begin")).not.toHaveProperty("schoolId");
  });

  it("restores the file after a remount, but not the plan", async () => {
    // What a school switch does: `SchoolSetup` keys the body on the school, so this
    // component is rebuilt. The file is not about a school and comes back; the plan is.
    route({
      ...BASE,
      import_current: source(),
      import_preview: () => cleanPlan(),
    });
    render(<ImportData school={SCHOOL} />);

    expect(await screen.findByText(/rows on sheet/)).toBeTruthy();
    expect(screen.queryByText(/to create/)).toBeNull();
    expect(screen.getByRole("button", { name: "Preview" })).toBeTruthy();
  });
});
