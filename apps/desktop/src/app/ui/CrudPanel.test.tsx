/**
 * The shared CRUD panel.
 *
 * Eight of the nine setup sections are this component with different fields, so the
 * brief's requirements about forms and deletes are mostly requirements about *this*:
 *
 * - forms populate from persisted data
 * - required fields and obvious input errors are validated
 * - database constraint errors become useful messages
 * - destructive actions require confirmation
 * - RESTRICT failures explain that dependents must be removed or reassigned first
 * - empty states guide the user toward the next action
 *
 * Testing it once covers all eight. The fixture below is a deliberately plain entity so
 * the assertions stay about the interaction rather than about any one section's fields.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { CrudPanel, type CrudSpec } from "./CrudPanel";
import { problems, required, text, type Draft } from "../validation";

interface Widget {
  id: string;
  name: string;
  code: string;
}

interface WidgetInput {
  name: string;
  code: string;
}

const ONE: Widget = { id: "w-1", name: "Main", code: "MAIN" };

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

beforeEach(() => {
  vi.clearAllMocks();
});

describe("listing", () => {
  it("shows the rows the backend returned", async () => {
    render(<CrudPanel spec={spec()} />);

    expect(await screen.findByText("Main")).toBeTruthy();
    expect(screen.getByText("MAIN")).toBeTruthy();
  });

  it("guides the user toward the next action when there is nothing yet", async () => {
    render(<CrudPanel spec={spec({ list: () => Promise.resolve([]) })} />);

    expect(await screen.findByText("No campuses yet")).toBeTruthy();
    expect(screen.getByText(/needs at least one campus/)).toBeTruthy();
    // The empty state offers the action, not just a description of the emptiness.
    expect(screen.getAllByRole("button", { name: /Add campus/i }).length).toBeGreaterThan(0);
  });

  it("shows a prerequisite instead of an empty table when it is blocked", async () => {
    const list = vi.fn(() => Promise.resolve([]));
    render(<CrudPanel spec={spec({ list, blockedBy: "Select a campus first." })} />);

    expect(await screen.findByText("Select a campus first.")).toBeTruthy();
    expect(list).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Add campus/i })).toBeNull();
  });

  it("reports a failed load rather than looking empty", async () => {
    render(
      <CrudPanel
        spec={spec({
          list: () => Promise.reject({ kind: "failed", message: "The project is in use." }),
        })}
      />,
    );

    expect(await screen.findByText("The project is in use.")).toBeTruthy();
  });
});

describe("creating", () => {
  it("validates required fields before sending anything", async () => {
    const create = vi.fn(() => Promise.resolve(ONE));
    render(<CrudPanel spec={spec({ create })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    expect(await screen.findByText("Campus name is required.")).toBeTruthy();
    expect(screen.getByText("Code is required.")).toBeTruthy();
    expect(create).not.toHaveBeenCalled();
  });

  it("marks the invalid input itself, not just a banner", async () => {
    render(<CrudPanel spec={spec()} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    const input = await screen.findByLabelText(/Campus name/);
    await waitFor(() => expect(input.getAttribute("aria-invalid")).toBe("true"));
    expect(input.getAttribute("aria-errormessage")).toBeTruthy();
  });

  it("clears a field's message as it is retyped", async () => {
    render(<CrudPanel spec={spec()} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));
    expect(await screen.findByText("Campus name is required.")).toBeTruthy();

    await userEvent.type(screen.getByLabelText(/Campus name/), "Annexe");
    expect(screen.queryByText("Campus name is required.")).toBeNull();
    // The other field's message is untouched: only what changed was reconsidered.
    expect(screen.getByText("Code is required.")).toBeTruthy();
  });

  it("sends the converted input and re-reads the list", async () => {
    const create = vi.fn(() => Promise.resolve(ONE));
    const list = vi.fn(() => Promise.resolve([ONE]));
    render(<CrudPanel spec={spec({ create, list })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.type(screen.getByLabelText(/Campus name/), "Annexe");
    await userEvent.type(screen.getByLabelText(/^Code/), "ANX");
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    await waitFor(() => {
      expect(create).toHaveBeenCalledWith({ name: "Annexe", code: "ANX" });
    });
    // Re-read rather than patched: a write can have side effects on other rows.
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  });

  it("trims whitespace rather than storing it", async () => {
    const create = vi.fn(() => Promise.resolve(ONE));
    render(<CrudPanel spec={spec({ create })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.type(screen.getByLabelText(/Campus name/), "  Annexe  ");
    await userEvent.type(screen.getByLabelText(/^Code/), "ANX");
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    await waitFor(() => expect(create).toHaveBeenCalledWith({ name: "Annexe", code: "ANX" }));
  });

  it("notifies the parent so dependent sections can refresh", async () => {
    const onChanged = vi.fn();
    render(<CrudPanel spec={spec()} onChanged={onChanged} />);

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.type(screen.getByLabelText(/Campus name/), "Annexe");
    await userEvent.type(screen.getByLabelText(/^Code/), "ANX");
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });
});

describe("editing", () => {
  it("populates the form from the persisted record", async () => {
    render(<CrudPanel spec={spec()} />);

    await userEvent.click(await screen.findByRole("button", { name: /Edit Main/i }));

    expect(screen.getByLabelText(/Campus name/)).toHaveProperty("value", "Main");
    expect(screen.getByLabelText(/^Code/)).toHaveProperty("value", "MAIN");
  });

  it("sends the identifier alongside the input", async () => {
    const update = vi.fn(() => Promise.resolve(ONE));
    render(<CrudPanel spec={spec({ update })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Edit Main/i }));
    const name = screen.getByLabelText(/Campus name/);
    await userEvent.clear(name);
    await userEvent.type(name, "Main Site");
    await userEvent.click(screen.getByRole("button", { name: /Save changes/i }));

    await waitFor(() => {
      expect(update).toHaveBeenCalledWith("w-1", { name: "Main Site", code: "MAIN" });
    });
  });

  it("abandons nothing on cancel", async () => {
    const update = vi.fn(() => Promise.resolve(ONE));
    render(<CrudPanel spec={spec({ update })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Edit Main/i }));
    await userEvent.click(screen.getByRole("button", { name: /^Cancel$/i }));

    expect(update).not.toHaveBeenCalled();
    expect(screen.queryByLabelText(/Campus name/)).toBeNull();
  });
});

describe("constraint failures from the backend", () => {
  it("marks the field a duplicate collided on", async () => {
    render(
      <CrudPanel
        spec={spec({
          create: () =>
            Promise.reject({
              kind: "duplicate",
              entity: "campus",
              field: "code",
              message: "Another campus already uses that code.",
            }),
        })}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.type(screen.getByLabelText(/Campus name/), "Annexe");
    await userEvent.type(screen.getByLabelText(/^Code/), "MAIN");
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    expect(await screen.findByText("Another campus already uses that code.")).toBeTruthy();
    const code = screen.getByLabelText(/^Code/);
    await waitFor(() => expect(code.getAttribute("aria-invalid")).toBe("true"));
    // The name field is not blamed for the code's collision.
    expect(screen.getByLabelText(/Campus name/).getAttribute("aria-invalid")).toBeNull();
  });

  it("maps the backend's `position` onto the form's `ordinal` field", async () => {
    render(
      <CrudPanel
        spec={spec({
          fields: [
            { kind: "number", key: "ordinal", label: "Position", required: true },
            { kind: "text", key: "label", label: "Label", required: true },
          ],
          validate: () => ({}),
          toDraft: () => ({ ordinal: "1", label: "Day A" }),
          create: () =>
            Promise.reject({
              kind: "duplicate",
              entity: "cycle day",
              field: "position",
              message: "Another cycle day already uses that position.",
            }),
        })}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    const ordinal = await screen.findByLabelText(/Position/);
    await waitFor(() => expect(ordinal.getAttribute("aria-invalid")).toBe("true"));
  });

  it("keeps the typed values when a save is rejected", async () => {
    render(
      <CrudPanel
        spec={spec({
          create: () => Promise.reject({ kind: "failed", message: "Could not save." }),
        })}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Add campus/i }));
    await userEvent.type(screen.getByLabelText(/Campus name/), "Annexe");
    await userEvent.type(screen.getByLabelText(/^Code/), "ANX");
    await userEvent.click(screen.getByRole("button", { name: /^Add campus$/i }));

    expect(await screen.findByText("Could not save.")).toBeTruthy();
    expect(screen.getByLabelText(/Campus name/)).toHaveProperty("value", "Annexe");
    expect(screen.getByLabelText(/^Code/)).toHaveProperty("value", "ANX");
  });
});

describe("deleting", () => {
  it("asks before deleting anything", async () => {
    const remove = vi.fn(() => Promise.resolve());
    render(<CrudPanel spec={spec({ remove })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Delete Main/i }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/cannot be undone/i)).toBeTruthy();
    expect(within(dialog).getByText("Main (MAIN)")).toBeTruthy();
    expect(remove).not.toHaveBeenCalled();
  });

  it("deletes nothing when the confirmation is cancelled", async () => {
    const remove = vi.fn(() => Promise.resolve());
    render(<CrudPanel spec={spec({ remove })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Delete Main/i }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Cancel$/i }));

    expect(remove).not.toHaveBeenCalled();
  });

  it("deletes and re-reads once confirmed", async () => {
    const remove = vi.fn(() => Promise.resolve());
    const list = vi.fn(() => Promise.resolve([ONE]));
    render(<CrudPanel spec={spec({ remove, list })} />);

    await userEvent.click(await screen.findByRole("button", { name: /Delete Main/i }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Delete campus$/i }));

    await waitFor(() => expect(remove).toHaveBeenCalledWith("w-1"));
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  });

  it("shows the extra warning a row deserves", async () => {
    render(
      <CrudPanel
        spec={spec({
          deleteNote: () => <p>Rooms in this building are not deleted.</p>,
        })}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Delete Main/i }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Rooms in this building are not deleted/)).toBeTruthy();
  });

  it("explains a RESTRICT refusal, including what to do about it", async () => {
    render(
      <CrudPanel
        spec={spec({
          remove: () =>
            Promise.reject({
              kind: "stillReferenced",
              entity: "campus",
              message:
                "This campus cannot be deleted while other records still depend on it. " +
                "Remove or reassign those records first, then try again.",
            }),
        })}
      />,
    );

    await userEvent.click(await screen.findByRole("button", { name: /Delete Main/i }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /^Delete campus$/i }));

    const error = await screen.findByText(/Remove or reassign those records first/);
    expect(error).toBeTruthy();
    // The dialogue closes: the explanation belongs on the page, not in a modal the user
    // has to dismiss before they can act on it.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // And the row is still there, because nothing was deleted.
    expect(screen.getByText("Main")).toBeTruthy();
  });
});

describe("parent selection", () => {
  it("re-reads when the parent row changes", async () => {
    const list = vi.fn(() => Promise.resolve([ONE]));
    const { rerender } = render(
      <CrudPanel spec={spec({ list })} reloadKey="campus-1" />,
    );
    await screen.findByText("Main");
    expect(list).toHaveBeenCalledTimes(1);

    rerender(<CrudPanel spec={spec({ list })} reloadKey="campus-2" />);
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  });
});
