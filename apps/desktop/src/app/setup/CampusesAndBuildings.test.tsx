/**
 * Campuses and Buildings, with a building shared across campuses.
 *
 * The repository tests in `evara-db` own the rules — which shares are legal, what blocks a
 * withdrawal, what the change feed records. This file owns the things only the interface
 * can get wrong:
 *
 * - the same building is listed by every campus it serves, and reads as shared
 * - the multi-select sends campus *identifiers*, and only the extra ones
 * - withdrawing a share is confirmed before it is sent, the way a delete is
 * - a refusal from the backend is shown rather than swallowed, and nothing is lost
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Building, BuildingWithCampuses, Campus, School } from "@evara/domain";

const listCampuses = vi.fn<() => Promise<Campus[]>>();
const listBuildings = vi.fn<(campusId: string) => Promise<BuildingWithCampuses[]>>();
const createBuilding = vi.fn();
const updateBuilding = vi.fn();
const shareBuilding = vi.fn();
const unshareBuilding = vi.fn();
const deleteBuilding = vi.fn();

vi.mock("../../ipc/setup", () => ({
  listCampuses: () => listCampuses(),
  createCampus: vi.fn(() => Promise.resolve({})),
  updateCampus: vi.fn(() => Promise.resolve({})),
  deleteCampus: vi.fn(() => Promise.resolve()),
  listBuildings: (campusId: string) => listBuildings(campusId),
  createBuilding: (input: unknown, shared: unknown) => createBuilding(input, shared),
  updateBuilding: (id: string, input: unknown) => updateBuilding(id, input),
  shareBuilding: (buildingId: string, campusId: string) =>
    shareBuilding(buildingId, campusId),
  unshareBuilding: (schoolId: string, buildingId: string, campusId: string) =>
    unshareBuilding(schoolId, buildingId, campusId),
  deleteBuilding: (id: string) => deleteBuilding(id),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" && error !== null && "field" in error
      ? String((error as { field: unknown }).field)
      : null,
}));

const { CampusesAndBuildings } = await import("./CampusesAndBuildings");

const SCHOOL = { id: "school-1" } as School;

function campus(id: string, name: string, code: string): Campus {
  return {
    id,
    schoolId: "school-1",
    name,
    code,
    address: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as Campus;
}

const NORTH = campus("north", "North", "N");
const SOUTH = campus("south", "South", "S");
const EAST = campus("east", "East", "E");

function building(id: string, homeCampusId: string, name: string): Building {
  return {
    id,
    campusId: homeCampusId,
    name,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    rev: 1,
  } as Building;
}

/** The sports centre: home on North, shared with South. */
const SPORTS = building("b-sports", "north", "Sports Centre");
const SHARED_SPORTS: BuildingWithCampuses = {
  building: SPORTS,
  campusIds: ["north", "south"],
};
/** The library: North only. */
const LIBRARY = building("b-library", "north", "Library");
const PLAIN_LIBRARY: BuildingWithCampuses = {
  building: LIBRARY,
  campusIds: ["north"],
};

beforeEach(() => {
  vi.clearAllMocks();
  listCampuses.mockResolvedValue([NORTH, SOUTH, EAST]);
  listBuildings.mockResolvedValue([]);
  createBuilding.mockImplementation((input: { campusId: string; name: string }) =>
    Promise.resolve(building("b-new", input.campusId, input.name)),
  );
  updateBuilding.mockImplementation((id: string, input: { name: string }) =>
    Promise.resolve(building(id, "north", input.name)),
  );
  shareBuilding.mockResolvedValue(undefined);
  unshareBuilding.mockResolvedValue(undefined);
  deleteBuilding.mockResolvedValue(undefined);
});

/** Renders the screen and waits for the first campus to be selected automatically. */
async function open() {
  render(<CampusesAndBuildings school={SCHOOL} />);
  // ParentPicker selects the first campus itself, which triggers the building read.
  await waitFor(() => expect(listBuildings).toHaveBeenCalled());
}

/** The buildings table, as opposed to the campuses table above it. */
function buildingsTable(): HTMLElement {
  return screen.getByRole("table", { name: /Buildings on this campus/i });
}

async function openBuildingForm(name: string) {
  const row = within(buildingsTable()).getByText(name).closest("tr");
  await userEvent.click(within(row as HTMLElement).getByRole("button", { name: /Edit/i }));
}

/**
 * Opens the new-building form.
 *
 * `CrudPanel` deliberately offers "Add building" in two places — the panel header and the
 * empty state — and the form's own submit button carries the same words, so these have to
 * be told apart by position rather than by name.
 */
async function startNewBuilding() {
  const buttons = screen.getAllByRole("button", { name: "Add building" });
  await userEvent.click(buttons[0]!);
}

/** The open form's submit button, as opposed to either button that opens it. */
function submitButton(label: string): HTMLElement {
  const form = document.querySelector("form.entityForm");
  return within(form as HTMLElement).getByRole("button", { name: label });
}

/** A campus chip in the picker, as opposed to a row in the campuses table. */
function pickCampus(name: RegExp) {
  const picker = screen.getByRole("group", { name: "Campus" });
  return userEvent.click(within(picker).getByRole("button", { name }));
}

const sharedWith = () => screen.getByRole("combobox", { name: /Shared with campuses/ });

describe("viewing a shared building", () => {
  it("lists it on its home campus and marks it shared", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();

    const table = buildingsTable();
    expect(await within(table).findByText("Sports Centre")).toBeTruthy();
    expect(within(table).getByText("shared")).toBeTruthy();
    // The campus it is shared with is named, not just counted.
    expect(within(table).getByText("South")).toBeTruthy();
  });

  it("lists the identical building when viewing the campus it is shared with", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();

    // Switch to South: the same building id comes back from the same command.
    await pickCampus(/South/);

    await waitFor(() => expect(listBuildings).toHaveBeenLastCalledWith("south"));
    expect(await within(buildingsTable()).findByText("Sports Centre")).toBeTruthy();
  });

  it("says plainly when a building is not shared at all", async () => {
    listBuildings.mockResolvedValue([PLAIN_LIBRARY]);
    await open();

    expect(await within(buildingsTable()).findByText("This campus only")).toBeTruthy();
    expect(within(buildingsTable()).queryByText("shared")).toBeNull();
  });
});

describe("creating a building", () => {
  it("creates it for the selected campus with no shares by default", async () => {
    await open();
    await startNewBuilding();
    await userEvent.type(screen.getByLabelText(/Building name/), "Science Block");
    await userEvent.click(submitButton("Add building"));

    await waitFor(() => expect(createBuilding).toHaveBeenCalled());
    expect(createBuilding).toHaveBeenCalledWith(
      { campusId: "north", name: "Science Block" },
      [],
    );
  });

  it("offers the other campuses, never the one being viewed", async () => {
    await open();
    await startNewBuilding();
    await userEvent.click(sharedWith());

    // In the order the backend listed them — `campuses()` already sorts by name, so
    // re-sorting here would be a second opinion about ordering.
    const labels = screen.getAllByRole("option").map((o) => o.textContent);
    expect(labels).toEqual(["South (S)", "East (E)"]);
    expect(labels.some((label) => label?.includes("North"))).toBe(false);
  });

  it("sends the chosen campuses as identifiers, not labels", async () => {
    await open();
    await startNewBuilding();
    await userEvent.type(screen.getByLabelText(/Building name/), "Sports Centre");

    await userEvent.type(sharedWith(), "south");
    await userEvent.keyboard("{Enter}");
    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");

    await userEvent.click(submitButton("Add building"));

    await waitFor(() => expect(createBuilding).toHaveBeenCalled());
    expect(createBuilding).toHaveBeenCalledWith(
      { campusId: "north", name: "Sports Centre" },
      ["south", "east"],
    );
  });

  it("explains that a single-campus school has nothing to share with", async () => {
    listCampuses.mockResolvedValue([NORTH]);
    await open();
    await startNewBuilding();

    expect(screen.getByText(/nothing to share with/i)).toBeTruthy();
  });

  it("still requires a name", async () => {
    await open();
    await startNewBuilding();
    await userEvent.click(submitButton("Add building"));

    expect(await screen.findByText("Building name is required.")).toBeTruthy();
    expect(createBuilding).not.toHaveBeenCalled();
  });
});

describe("editing the share set", () => {
  it("opens with the campuses it is already shared with, home excluded", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    // South is a chip; North is the home campus and is not an editable share.
    expect(screen.getByRole("button", { name: "Remove South (S)" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Remove North/ })).toBeNull();
  });

  it("adds a campus without touching the existing one", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareBuilding).toHaveBeenCalled());
    expect(shareBuilding).toHaveBeenCalledWith("b-sports", "east");
    expect(unshareBuilding).not.toHaveBeenCalled();
  });

  it("renames without disturbing the shares", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    const name = screen.getByLabelText(/Building name/);
    await userEvent.clear(name);
    await userEvent.type(name, "Sports Hall");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateBuilding).toHaveBeenCalled());
    expect(updateBuilding).toHaveBeenCalledWith("b-sports", {
      campusId: "north",
      name: "Sports Hall",
    });
    expect(shareBuilding).not.toHaveBeenCalled();
    expect(unshareBuilding).not.toHaveBeenCalled();
  });
});

describe("withdrawing a share", () => {
  it("asks before sending it, naming the campus that loses the building", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    // Nothing is sent until the question is answered.
    expect(unshareBuilding).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/no longer be available to/i)).toBeTruthy();
    expect(within(dialog).getByText("South")).toBeTruthy();
    expect(within(dialog).getByText(/No rooms are deleted or detached/i)).toBeTruthy();
  });

  it("sends it once confirmed", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareBuilding).toHaveBeenCalled());
    // The school doing the withdrawing travels with it: either the school losing
    // access or the one that owns the building may do this, and a third may not.
    expect(unshareBuilding).toHaveBeenCalledWith("school-1", "b-sports", "south");
  });

  it("abandons it on cancel, with the edit still on screen", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /Cancel/i }));

    expect(unshareBuilding).not.toHaveBeenCalled();
    expect(updateBuilding).not.toHaveBeenCalled();
    // The form is still open with the removal still pending, so nothing was lost.
    expect(screen.getByLabelText(/Building name/)).toBeTruthy();
  });

  it("does not ask when nothing is being removed", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareBuilding).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the backend's refusal rather than swallowing it", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    unshareBuilding.mockRejectedValue({
      kind: "invalid",
      message:
        "3 rooms are still in Sports Centre on South. Move them to another building, or clear their building, before removing this campus.",
    });
    await open();
    await openBuildingForm("Sports Centre");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText(/3 rooms are still in Sports Centre/)).toBeTruthy();
    expect(screen.getByText(/Move them to another building/)).toBeTruthy();
  });
});

describe("deleting a shared building", () => {
  it("warns that it goes from every campus, not just this one", async () => {
    listBuildings.mockResolvedValue([SHARED_SPORTS]);
    await open();

    const row = within(buildingsTable()).getByText("Sports Centre").closest("tr");
    await userEvent.click(
      within(row as HTMLElement).getByRole("button", { name: /Delete/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/removes it from all of them/i)).toBeTruthy();
    // The note's "not" is wrapped in <strong>, so match the surrounding sentence.
    expect(within(dialog).getByText(/stop naming a building/i)).toBeTruthy();
  });

  it("does not warn about sharing for a building only one campus has", async () => {
    listBuildings.mockResolvedValue([PLAIN_LIBRARY]);
    await open();

    const row = within(buildingsTable()).getByText("Library").closest("tr");
    await userEvent.click(
      within(row as HTMLElement).getByRole("button", { name: /Delete/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByText(/removes it from all of them/i)).toBeNull();
  });
});
