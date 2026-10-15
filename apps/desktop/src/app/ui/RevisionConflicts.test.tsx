/**
 * Stage B: what the interface does when somebody else saved first.
 *
 * ## Why this is its own suite
 *
 * The backend's half is covered in Rust: the write is refused, the row keeps the first
 * writer's values, no revision is consumed, no change-log entry is appended. None of that
 * is any use if the screen then behaves badly, and the ways a screen can behave badly here
 * are specific and tempting:
 *
 * | Tempting | Why it is wrong |
 * | --- | --- |
 * | Retry with the current revision | Completes the overwrite the check prevented, without asking |
 * | Re-read and replace the form | Destroys the user's edit, which now exists nowhere else |
 * | Show "saved" and close the form | The one outcome the whole mechanism exists to prevent |
 * | Say "that record no longer exists" | It does exist; the user goes looking for nothing |
 *
 * So each of those is asserted *not* to happen, which is the only way a promise of this
 * shape can be held. ADR 0027 §6 states the contract.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { CrudPanel, type CrudSpec } from "./CrudPanel";
import { problems, required, text, type Draft } from "../validation";

interface Widget {
  id: string;
  name: string;
  code: string;
  rev: number;
}

interface WidgetInput {
  name: string;
  code: string;
}

/** Revision 4, so a panel sending a default cannot pass by coincidence. */
const ONE: Widget = { id: "w-1", name: "Main", code: "MAIN", rev: 4 };

/** What the backend sends when somebody else saved first. */
const CONFLICT = {
  kind: "conflict",
  entity: "campus",
  expectedRev: 4,
  actualRev: 5,
  message:
    "This campus has changed since you opened it, so nothing was saved. " +
    "Reload the latest version before saving.",
};

function spec(overrides: Partial<CrudSpec<Widget, WidgetInput>> = {}): CrudSpec<
  Widget,
  WidgetInput
> {
  return {
    noun: "campus",
    plural: "campuses",
    title: "Campuses",
    describe: (row) => `${row.name} (${row.code})`,
    columns: [
      { header: "Name", render: (row) => row.name },
      { header: "Code", render: (row) => row.code },
    ],
    fields: [
      { kind: "text", key: "name", label: "Campus name", required: true },
      { kind: "text", key: "code", label: "Code", required: true },
    ],
    validate: (draft: Draft) =>
      problems([
        ["name", required(text(draft, "name"), "Campus name")],
        ["code", required(text(draft, "code"), "Code")],
      ]),
    toDraft: (row) => ({ name: row?.name ?? "", code: row?.code ?? "" }),
    toInput: (draft) => ({ name: text(draft, "name"), code: text(draft, "code") }),
    list: vi.fn(() => Promise.resolve([ONE])),
    create: vi.fn(() => Promise.resolve(ONE)),
    update: vi.fn(() => Promise.resolve(ONE)),
    remove: vi.fn(() => Promise.resolve()),
    emptyHint: "A school needs at least one campus before rooms can be recorded.",
    ...overrides,
  };
}

/** Opens the edit form on the one row and types a new name into it. */
async function editAndSave(user: ReturnType<typeof userEvent.setup>, newName: string) {
  await user.click(await screen.findByRole("button", { name: /Edit Main/i }));
  const name = screen.getByLabelText(/Campus name/);
  await user.clear(name);
  await user.type(name, newName);
  await user.click(screen.getByRole("button", { name: /Save changes/i }));
}

describe("a save that lost a race", () => {
  it("says that nothing was saved, in those words", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");

    // The message is the backend's, shown as written. A screen that worded it itself would
    // be a second place for the one promise that matters to drift.
    const banner = await screen.findByText(/nothing was saved/);
    expect(banner).toBeTruthy();
    expect(banner.textContent).toMatch(/changed since you opened it/);
  });

  it("keeps every value the user typed", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");
    await screen.findByText(/nothing was saved/);

    // Their edit now exists in no other place. Replacing it with the stored version to
    // "show them the latest" would destroy the one copy of their work.
    expect(screen.getByLabelText(/Campus name/)).toHaveProperty("value", "Main Site");
    expect(screen.getByLabelText(/^Code/)).toHaveProperty("value", "MAIN");
  });

  it("leaves the form open rather than closing as though it had worked", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");
    await screen.findByText(/nothing was saved/);

    expect(screen.getByRole("button", { name: /Save changes/i })).toBeTruthy();
  });

  it("never retries with a fresh revision", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");
    await screen.findByText(/nothing was saved/);

    // Exactly one attempt. A retry would complete the overwrite the check prevented, with
    // the user's agreement neither asked for nor given.
    expect(update).toHaveBeenCalledTimes(1);
  });

  it("offers to reload, and does not reload until asked", async () => {
    const user = userEvent.setup();
    const list = vi.fn(() => Promise.resolve([ONE]));
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ list, update })} />);

    await editAndSave(user, "Main Site");
    await screen.findByText(/nothing was saved/);

    // One read: the initial one. Whether to look at the other version is the user's
    // decision, because only they know whose change should survive.
    expect(list).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: /Reload campuses/i }));

    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    // And the warning goes with the re-read, because it is no longer true.
    await waitFor(() => expect(screen.queryByText(/nothing was saved/)).toBeNull());
  });

  it("does not pin the message to a form field", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.reject(CONFLICT));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");
    await screen.findByText(/nothing was saved/);

    // A conflict is not about a value, so neither input is marked wrong — unlike a
    // duplicate, which is.
    expect(screen.getByLabelText(/Campus name/).getAttribute("aria-invalid")).toBeNull();
    expect(screen.getByLabelText(/^Code/).getAttribute("aria-invalid")).toBeNull();
  });
});

describe("a delete that lost a race", () => {
  it("explains and leaves the row alone", async () => {
    const user = userEvent.setup();
    const remove = vi.fn(() => Promise.reject(CONFLICT));
    const list = vi.fn(() => Promise.resolve([ONE]));
    render(<CrudPanel spec={spec({ remove, list })} />);

    await user.click(await screen.findByRole("button", { name: /Delete Main/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^Delete campus$/i }));

    expect(await screen.findByText(/nothing was saved/)).toBeTruthy();
    // The row is still there, because nothing was deleted.
    expect(screen.getByText("Main")).toBeTruthy();
    expect(remove).toHaveBeenCalledTimes(1);
    expect(list).toHaveBeenCalledTimes(1);
  });
});

describe("other failures are still handled as they were", () => {
  it("a duplicate still marks the offending field", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() =>
      Promise.reject({
        kind: "duplicate",
        entity: "campus",
        field: "code",
        message: "Another campus already uses that code.",
      }),
    );
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");

    expect(await screen.findByText(/already uses that code/)).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByLabelText(/^Code/).getAttribute("aria-invalid")).toBe("true"),
    );
    // And no reload button, because reloading is not the remedy for a bad value.
    expect(screen.queryByRole("button", { name: /Reload campuses/i })).toBeNull();
  });

  it("a record that is genuinely gone is not described as out of date", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() =>
      Promise.reject({
        kind: "notFound",
        entity: "campus",
        message: "That campus no longer exists. It may have been deleted already.",
      }),
    );
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");

    expect(await screen.findByText(/no longer exists/)).toBeTruthy();
    expect(screen.queryByText(/nothing was saved/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Reload campuses/i })).toBeNull();
  });

  it("a wrong-school refusal says the boundary held, not that the copy is stale", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() =>
      Promise.reject({
        kind: "wrongSchool",
        entity: "campus",
        message:
          "That campus belongs to a different school, so nothing was changed. " +
          "Switch to the school it belongs to and try again.",
      }),
    );
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Reload campuses/i })).toBeNull();
  });
});

describe("a save that succeeds", () => {
  it("shows no warning and closes the form", async () => {
    const user = userEvent.setup();
    const update = vi.fn(() => Promise.resolve(ONE));
    render(<CrudPanel spec={spec({ update })} />);

    await editAndSave(user, "Main Site");

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /Save changes/i })).toBeNull(),
    );
    expect(screen.queryByText(/nothing was saved/)).toBeNull();
  });
});
