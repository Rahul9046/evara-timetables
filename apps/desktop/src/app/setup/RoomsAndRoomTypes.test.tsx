/**
 * Rooms and Room Types, with a room shared across campuses.
 *
 * The repository tests in `evara-db` own the rules — which shares are legal, what the
 * building has to do with it, what the change feed records. This file owns the things only
 * the interface can get wrong:
 *
 * - the same room is listed by every campus it serves, and reads as shared
 * - the multi-select sends campus *identifiers*, and only the extra ones
 * - a room's own attributes survive a share, rather than being dropped from the payload
 * - the three calls of an edit go out in the order that lets a legitimate edit succeed
 * - withdrawing a share is confirmed before it is sent, the way a delete is
 * - a refusal from the backend is shown rather than swallowed, and nothing is lost
 * - a room with no building is a normal, supported state throughout
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  Building,
  BuildingWithCampuses,
  Campus,
  Room,
  RoomType,
  RoomWithCampuses,
  School,
} from "@evara/domain";

const listCampuses = vi.fn<() => Promise<Campus[]>>();
const listBuildings = vi.fn<(campusId: string) => Promise<BuildingWithCampuses[]>>();
const listRoomTypes = vi.fn<() => Promise<RoomType[]>>();
const listRooms = vi.fn<(campusId: string) => Promise<RoomWithCampuses[]>>();
const createRoom = vi.fn();
const updateRoom = vi.fn();
const shareRoom = vi.fn();
const unshareRoom = vi.fn();
const deleteRoom = vi.fn();

/** Records the order calls were made in, which is the point of one test below. */
const order: string[] = [];

vi.mock("../../ipc/setup", () => ({
  listCampuses: () => listCampuses(),
  listBuildings: (campusId: string) => listBuildings(campusId),
  listRoomTypes: () => listRoomTypes(),
  listRooms: (campusId: string) => listRooms(campusId),
  createRoom: (input: unknown, shared: unknown) => createRoom(input, shared),
  updateRoom: (id: string, input: unknown) => {
    order.push("update");
    return updateRoom(id, input);
  },
  shareRoom: (roomId: string, campusId: string) => {
    order.push(`share:${campusId}`);
    return shareRoom(roomId, campusId);
  },
  unshareRoom: (roomId: string, campusId: string) => {
    order.push(`unshare:${campusId}`);
    return unshareRoom(roomId, campusId);
  },
  deleteRoom: (id: string) => deleteRoom(id),
  createRoomType: vi.fn(() => Promise.resolve({})),
  updateRoomType: vi.fn(() => Promise.resolve({})),
  deleteRoomType: vi.fn(() => Promise.resolve()),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" && error !== null && "field" in error
      ? String((error as { field: unknown }).field)
      : null,
}));

const { RoomsAndRoomTypes } = await import("./RoomsAndRoomTypes");

const SCHOOL = { id: "school-1" } as School;
const STAMP = "2026-01-01T00:00:00.000Z";

function campus(id: string, name: string, code: string): Campus {
  return {
    id,
    schoolId: "school-1",
    name,
    code,
    address: null,
    createdAt: STAMP,
    updatedAt: STAMP,
    rev: 1,
  } as Campus;
}

const NORTH = campus("north", "North", "N");
const SOUTH = campus("south", "South", "S");
const EAST = campus("east", "East", "E");

/** The sports centre: home on North, shared with South — so its rooms *may* be shared. */
const SPORTS: BuildingWithCampuses = {
  building: {
    id: "b-sports",
    campusId: "north",
    name: "Sports Centre",
    createdAt: STAMP,
    updatedAt: STAMP,
    rev: 1,
  } as Building,
  campusIds: ["north", "south"],
};

function room(
  id: string,
  homeCampusId: string,
  name: string,
  code: string,
  buildingId: string | null = null,
): Room {
  return {
    id,
    campusId: homeCampusId,
    buildingId,
    roomTypeId: null,
    name,
    code,
    capacity: 30,
    isBookable: true,
    notes: null,
    createdAt: STAMP,
    updatedAt: STAMP,
    rev: 1,
  } as Room;
}

/** The sports hall: home on North, shared with South, inside the shared building. */
const HALL = room("r-hall", "north", "Sports Hall", "HALL", "b-sports");
const SHARED_HALL: RoomWithCampuses = {
  room: HALL,
  campusIds: ["north", "south"],
};
/** A classroom on North only, with no building at all. */
const CLASSROOM = room("r-c1", "north", "Classroom 1", "C1", null);
const PLAIN_CLASSROOM: RoomWithCampuses = {
  room: CLASSROOM,
  campusIds: ["north"],
};

beforeEach(() => {
  vi.clearAllMocks();
  order.length = 0;
  listCampuses.mockResolvedValue([NORTH, SOUTH, EAST]);
  listRoomTypes.mockResolvedValue([]);
  listBuildings.mockResolvedValue([SPORTS]);
  listRooms.mockResolvedValue([]);
  createRoom.mockImplementation((input: { campusId: string; name: string; code: string }) =>
    Promise.resolve(room("r-new", input.campusId, input.name, input.code)),
  );
  updateRoom.mockImplementation((id: string, input: { name: string; code: string }) =>
    Promise.resolve(room(id, "north", input.name, input.code)),
  );
  shareRoom.mockResolvedValue(undefined);
  unshareRoom.mockResolvedValue(undefined);
  deleteRoom.mockResolvedValue(undefined);
});

/** Renders the screen and waits for the first campus to be selected automatically. */
async function open() {
  render(<RoomsAndRoomTypes school={SCHOOL} />);
  // ParentPicker selects the first campus itself, which triggers the room read.
  await waitFor(() => expect(listRooms).toHaveBeenCalled());
}

/** The rooms table, as opposed to the room types table above it. */
function roomsTable(): HTMLElement {
  return screen.getByRole("table", { name: /Rooms on this campus/i });
}

async function openRoomForm(name: string) {
  const row = within(roomsTable()).getByText(name).closest("tr");
  await userEvent.click(within(row as HTMLElement).getByRole("button", { name: /Edit/i }));
}

/**
 * Opens the new-room form.
 *
 * `CrudPanel` offers "Add room" in the panel header and in the empty state, and the form's
 * own submit button carries the same words, so these are told apart by position.
 */
async function startNewRoom() {
  const buttons = screen.getAllByRole("button", { name: "Add room" });
  await userEvent.click(buttons[0]!);
}

/** The open form's submit button, as opposed to either button that opens it. */
function submitButton(label: string): HTMLElement {
  const form = document.querySelector("form.entityForm");
  return within(form as HTMLElement).getByRole("button", { name: label });
}

/** A campus chip in the picker, as opposed to a row in a table. */
function pickCampus(name: RegExp) {
  const picker = screen.getByRole("group", { name: "Campus" });
  return userEvent.click(within(picker).getByRole("button", { name }));
}

const sharedWith = () => screen.getByRole("combobox", { name: /Shared with campuses/ });

describe("viewing a shared room", () => {
  it("lists it on its home campus and marks it shared", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();

    const table = roomsTable();
    expect(await within(table).findByText("Sports Hall")).toBeTruthy();
    expect(within(table).getByText("shared")).toBeTruthy();
    // The campus it is shared with is named, not just counted.
    expect(within(table).getByText("South")).toBeTruthy();
  });

  it("lists the identical room when viewing the campus it is shared with", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();

    await pickCampus(/South/);

    await waitFor(() => expect(listRooms).toHaveBeenLastCalledWith("south"));
    // The same row, from the same command — one room, listed twice, never duplicated.
    expect(await within(roomsTable()).findByText("Sports Hall")).toBeTruthy();
    expect(within(roomsTable()).getByText("HALL")).toBeTruthy();
  });

  it("says plainly when a room is not shared at all", async () => {
    listRooms.mockResolvedValue([PLAIN_CLASSROOM]);
    await open();

    expect(await within(roomsTable()).findByText("This campus only")).toBeTruthy();
    expect(within(roomsTable()).queryByText("shared")).toBeNull();
  });

  it("keeps room attributes visible alongside the sharing columns", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();

    const row = (await within(roomsTable()).findByText("Sports Hall")).closest("tr");
    const cells = within(row as HTMLElement);
    expect(cells.getByText("HALL")).toBeTruthy();
    expect(cells.getByText("Sports Centre (shared)")).toBeTruthy();
    expect(cells.getByText("30")).toBeTruthy();
    expect(cells.getByText("Yes")).toBeTruthy();
  });
});

describe("creating a room", () => {
  it("creates it for the selected campus with no shares by default", async () => {
    await open();
    await startNewRoom();
    await userEvent.type(screen.getByLabelText(/Room name/), "Classroom 1");
    await userEvent.type(screen.getByLabelText(/^Code/), "C1");
    await userEvent.click(submitButton("Add room"));

    await waitFor(() => expect(createRoom).toHaveBeenCalled());
    expect(createRoom).toHaveBeenCalledWith(
      {
        campusId: "north",
        buildingId: null,
        roomTypeId: null,
        name: "Classroom 1",
        code: "C1",
        capacity: 30,
        isBookable: true,
        notes: null,
      },
      [],
    );
  });

  it("offers the other campuses, never the one being viewed", async () => {
    await open();
    await startNewRoom();
    await userEvent.click(sharedWith());

    // Scoped to the share field's own listbox: the form's two native selects (Building,
    // Room type) also publish `option` elements, and a bare query would collect those.
    const labels = within(screen.getByRole("listbox", { name: "Options" }))
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(labels).toEqual(["South (S)", "East (E)"]);
    expect(labels.some((label) => label?.includes("North"))).toBe(false);
  });

  it("sends the chosen campuses as identifiers, not labels", async () => {
    await open();
    await startNewRoom();
    await userEvent.type(screen.getByLabelText(/Room name/), "Sports Hall");
    await userEvent.type(screen.getByLabelText(/^Code/), "HALL");

    await userEvent.type(sharedWith(), "south");
    await userEvent.keyboard("{Enter}");
    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");

    await userEvent.click(submitButton("Add room"));

    await waitFor(() => expect(createRoom).toHaveBeenCalled());
    expect(createRoom.mock.calls[0]?.[1]).toEqual(["south", "east"]);
  });

  it("carries capacity, bookability and notes through a shared create", async () => {
    // The share set is a second argument, not part of `RoomInput`. The risk in splitting
    // them is dropping the rest of the room on the way, so assert the whole payload.
    await open();
    await startNewRoom();
    await userEvent.type(screen.getByLabelText(/Room name/), "Store");
    await userEvent.type(screen.getByLabelText(/^Code/), "ST");
    const seats = screen.getByLabelText(/Seats/);
    await userEvent.clear(seats);
    await userEvent.type(seats, "0");
    await userEvent.click(screen.getByLabelText(/Lessons may be scheduled here/));
    await userEvent.type(screen.getByLabelText(/Notes/), "Caretaker only");
    await userEvent.type(sharedWith(), "south");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(submitButton("Add room"));

    await waitFor(() => expect(createRoom).toHaveBeenCalled());
    expect(createRoom).toHaveBeenCalledWith(
      {
        campusId: "north",
        buildingId: null,
        roomTypeId: null,
        name: "Store",
        code: "ST",
        capacity: 0,
        isBookable: false,
        notes: "Caretaker only",
      },
      ["south"],
    );
  });

  it("explains that a single-campus school has nothing to share with", async () => {
    listCampuses.mockResolvedValue([NORTH]);
    await open();
    await startNewRoom();

    expect(screen.getByText(/nothing to share with/i)).toBeTruthy();
  });

  it("still requires a name and a code", async () => {
    await open();
    await startNewRoom();
    await userEvent.click(submitButton("Add room"));

    expect(await screen.findByText("Room name is required.")).toBeTruthy();
    expect(createRoom).not.toHaveBeenCalled();
  });

  it("says that sharing is refused when the building does not reach the campus", async () => {
    // The repository owns this rule; the screen's job is to show what it said.
    createRoom.mockRejectedValue({
      kind: "invalid",
      message:
        "Lab 1 is in Science Block, which South does not have. Share Science Block with South first, or clear the room's building.",
    });
    await open();
    await startNewRoom();
    await userEvent.type(screen.getByLabelText(/Room name/), "Lab 1");
    await userEvent.type(screen.getByLabelText(/^Code/), "L1");
    await userEvent.type(sharedWith(), "south");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(submitButton("Add room"));

    expect(await screen.findByText(/which South does not have/)).toBeTruthy();
    expect(screen.getByText(/Share Science Block with South first/)).toBeTruthy();
  });
});

describe("a room with no building", () => {
  it("is offered and created without one", async () => {
    listBuildings.mockResolvedValue([]);
    await open();
    await startNewRoom();

    expect(screen.getByText(/This campus has no buildings/i)).toBeTruthy();

    await userEvent.type(screen.getByLabelText(/Room name/), "Field");
    await userEvent.type(screen.getByLabelText(/^Code/), "FLD");
    await userEvent.type(sharedWith(), "south");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(submitButton("Add room"));

    await waitFor(() => expect(createRoom).toHaveBeenCalled());
    expect(createRoom.mock.calls[0]?.[0]).toMatchObject({ buildingId: null });
    expect(createRoom.mock.calls[0]?.[1]).toEqual(["south"]);
  });

  it("shows no building rather than an empty cell", async () => {
    listRooms.mockResolvedValue([PLAIN_CLASSROOM]);
    await open();

    const row = (await within(roomsTable()).findByText("Classroom 1")).closest("tr");
    // The em dash the `Absent` component renders, so a blank cell is deliberate.
    expect(within(row as HTMLElement).getAllByText("—").length).toBeGreaterThan(0);
  });
});

describe("editing the share set", () => {
  it("opens with the campuses it is already shared with, home excluded", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    expect(screen.getByRole("button", { name: "Remove South (S)" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Remove North/ })).toBeNull();
  });

  it("adds a campus without touching the existing one", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareRoom).toHaveBeenCalled());
    expect(shareRoom).toHaveBeenCalledWith("r-hall", "east");
    expect(unshareRoom).not.toHaveBeenCalled();
  });

  it("renames without disturbing the shares", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    const name = screen.getByLabelText(/Room name/);
    await userEvent.clear(name);
    await userEvent.type(name, "Great Hall");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoom).toHaveBeenCalled());
    expect(updateRoom.mock.calls[0]?.[1]).toMatchObject({ name: "Great Hall" });
    expect(shareRoom).not.toHaveBeenCalled();
    expect(unshareRoom).not.toHaveBeenCalled();
  });

  it("never sends the share set inside the room input", async () => {
    // `RoomInput` is generated from the Rust record and has no such field; sending one
    // would be a second, hand-written input shape — the thing ADR 0012 forbids.
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoom).toHaveBeenCalled());
    expect(updateRoom.mock.calls[0]?.[1]).not.toHaveProperty("sharedCampusIds");
  });

  it("withdraws before changing the room, and shares after it", async () => {
    // The order is what makes "move it into a campus-local building and drop the campus
    // that cannot reach it" possible as one edit. Reversed, the update would be refused
    // for a share the user had already removed.
    listRooms.mockResolvedValue([
      { room: HALL, campusIds: ["north", "south"] },
    ]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareRoom).toHaveBeenCalled());
    expect(order).toEqual(["unshare:south", "update", "share:east"]);
  });
});

describe("withdrawing a share", () => {
  it("asks before sending it, naming the campus that loses the room", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(unshareRoom).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/no longer be available to/i)).toBeTruthy();
    expect(within(dialog).getByText("South")).toBeTruthy();
    expect(within(dialog).getByText(/not moved, copied or deleted/i)).toBeTruthy();
  });

  it("sends it once confirmed", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareRoom).toHaveBeenCalled());
    expect(unshareRoom).toHaveBeenCalledWith("r-hall", "south");
  });

  it("abandons it on cancel, with the edit still on screen", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: /Cancel/i }));

    expect(unshareRoom).not.toHaveBeenCalled();
    expect(updateRoom).not.toHaveBeenCalled();
    expect(screen.getByLabelText(/Room name/)).toBeTruthy();
  });

  it("does not ask when nothing is being removed", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.type(sharedWith(), "east");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareRoom).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the backend's refusal rather than swallowing it", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    unshareRoom.mockRejectedValue({
      kind: "invalid",
      message:
        "This room was created on North, so North cannot be removed from it. Delete the room instead, or remove the other campuses.",
    });
    await open();
    await openRoomForm("Sports Hall");

    await userEvent.click(screen.getByRole("button", { name: "Remove South (S)" }));
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText(/cannot be removed from it/)).toBeTruthy();
  });
});

describe("deleting a shared room", () => {
  it("warns that it goes from every campus, not just this one", async () => {
    listRooms.mockResolvedValue([SHARED_HALL]);
    await open();

    const row = within(roomsTable()).getByText("Sports Hall").closest("tr");
    await userEvent.click(
      within(row as HTMLElement).getByRole("button", { name: /Delete/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/removes it from all of them/i)).toBeTruthy();
  });

  it("does not warn about sharing for a room only one campus has", async () => {
    listRooms.mockResolvedValue([PLAIN_CLASSROOM]);
    await open();

    const row = within(roomsTable()).getByText("Classroom 1").closest("tr");
    await userEvent.click(
      within(row as HTMLElement).getByRole("button", { name: /Delete/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByText(/removes it from all of them/i)).toBeNull();
  });
});

describe("accessibility of the share field", () => {
  it("is a labelled combobox reachable and operable from the keyboard", async () => {
    await open();
    await startNewRoom();

    const field = sharedWith();
    expect(field.getAttribute("aria-expanded")).toBe("false");

    field.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(field.getAttribute("aria-expanded")).toBe("true");

    // Choose the highlighted option with the keyboard alone, then remove it again.
    await userEvent.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "Remove South (S)" })).toBeTruthy();

    await userEvent.keyboard("{Backspace}");
    expect(screen.queryByRole("button", { name: "Remove South (S)" })).toBeNull();
  });
});
