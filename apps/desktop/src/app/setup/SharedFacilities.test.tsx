/**
 * Stage 0C: buildings and rooms shared across schools.
 *
 * `building_campus` and `room_campus` have associated a facility with **campuses** since
 * `V6`, and ADR 0019 deliberately left them unconstrained by school — a sports centre
 * between two institutions on one site is the arrangement multi-school projects exist for,
 * and `room-clash` stays a per-`room_id` rule for the whole campus file.
 *
 * `evara-db`'s `tests_shared_buildings.rs` and `tests_shared_rooms.rs` own the rules. What
 * only the interface could get wrong — and did, until this stage — is **offering the
 * campuses**: the share picker read `listCampuses(school.id)`, so a capability the model
 * has had since `V12` was unreachable from the screen built to expose it. This file owns
 * that, plus the three things that follow from it:
 *
 * - a cross-school campus is **offered**, and labelled with the school that owns it;
 * - ownership and access are **shown as different things**, because the two schools can do
 *   different things with the facility;
 * - **deleting a shared facility stays blocked**, explained before it is attempted;
 * - the **home campus** cannot be withdrawn through ordinary unsharing.
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

const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listBuildings = vi.fn<(campusId: string) => Promise<BuildingWithCampuses[]>>();
const listRooms = vi.fn<(campusId: string) => Promise<RoomWithCampuses[]>>();
const listRoomTypes = vi.fn<(schoolId: string) => Promise<RoomType[]>>();
const shareBuilding = vi.fn<(buildingId: string, campusId: string) => Promise<void>>();
const unshareBuilding =
  vi.fn<(schoolId: string, buildingId: string, campusId: string) => Promise<void>>();
const deleteBuilding = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const updateBuilding =
  vi.fn<(id: string, rev: number, input: unknown) => Promise<Building>>();
const shareRoom = vi.fn<(roomId: string, campusId: string) => Promise<void>>();
const unshareRoom =
  vi.fn<(schoolId: string, roomId: string, campusId: string) => Promise<void>>();
const deleteRoom = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const updateRoom = vi.fn<(id: string, rev: number, input: unknown) => Promise<Room>>();

vi.mock("../../ipc/setup", () => ({
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  createCampus: vi.fn(() => Promise.resolve({})),
  updateCampus: vi.fn(() => Promise.resolve({})),
  deleteCampus: vi.fn(() => Promise.resolve()),
  listBuildings: (campusId: string) => listBuildings(campusId),
  createBuilding: vi.fn(() => Promise.resolve({})),
  updateBuilding: (id: string, rev: number, input: unknown) =>
    updateBuilding(id, rev, input),
  deleteBuilding: (schoolId: string, id: string, rev: number) =>
    deleteBuilding(schoolId, id, rev),
  shareBuilding: (buildingId: string, campusId: string) => shareBuilding(buildingId, campusId),
  unshareBuilding: (schoolId: string, buildingId: string, campusId: string) =>
    unshareBuilding(schoolId, buildingId, campusId),
  listRooms: (campusId: string) => listRooms(campusId),
  createRoom: vi.fn(() => Promise.resolve({})),
  updateRoom: (id: string, rev: number, input: unknown) => updateRoom(id, rev, input),
  deleteRoom: (schoolId: string, id: string, rev: number) =>
    deleteRoom(schoolId, id, rev),
  shareRoom: (roomId: string, campusId: string) => shareRoom(roomId, campusId),
  unshareRoom: (schoolId: string, roomId: string, campusId: string) =>
    unshareRoom(schoolId, roomId, campusId),
  listRoomTypes: (schoolId: string) => listRoomTypes(schoolId),
  createRoomType: vi.fn(() => Promise.resolve({})),
  updateRoomType: vi.fn(() => Promise.resolve({})),
  deleteRoomType: vi.fn(() => Promise.resolve()),
  listAvailability: vi.fn(() => Promise.resolve([])),
  countAvailability: vi.fn(() => Promise.resolve(0)),
  setAvailability: vi.fn(() => Promise.resolve({})),
  clearAvailability: vi.fn(() => Promise.resolve(true)),
  listCycles: vi.fn(() => Promise.resolve([])),
  listPeriodStructures: vi.fn(() => Promise.resolve([])),
  gridPreview: vi.fn(() => Promise.resolve(null)),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: () => null,
}));

const { CampusesAndBuildings } = await import("./CampusesAndBuildings");
const { RoomsAndRoomTypes } = await import("./RoomsAndRoomTypes");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function campus(id: string, schoolId: string, name: string, code: string): Campus {
  return { id, schoolId, name, code, address: null, ...STAMP } as Campus;
}

const NORTH_MAIN = campus("c-north-main", NORTH.id, "Main Campus", "NM");
const SOUTH_MAIN = campus("c-south-main", SOUTH.id, "Riverside", "SR");

/** A sports centre on Northgate's campus. */
function sportsCentre(campusIds: readonly string[]): BuildingWithCampuses {
  return {
    building: {
      id: "b-sports",
      campusId: NORTH_MAIN.id,
      name: "Sports Centre",
      ...STAMP,
    } as Building,
    campusIds: [...campusIds],
  } as BuildingWithCampuses;
}

/** A hall on Northgate's campus. */
function hall(campusIds: readonly string[]): RoomWithCampuses {
  return {
    room: {
      id: "r-hall",
      campusId: NORTH_MAIN.id,
      buildingId: null,
      roomTypeId: null,
      name: "Great Hall",
      code: "GH",
      capacity: 200,
      isBookable: true,
      notes: null,
      ...STAMP,
    } as Room,
    campusIds: [...campusIds],
  } as RoomWithCampuses;
}

function context(overrides: Partial<SetupContext> = {}): SetupContext {
  return {
    schools: [NORTH, SOUTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listCampuses.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_MAIN] : [SOUTH_MAIN]),
  );
  listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id])]);
  listRooms.mockResolvedValue([hall([NORTH_MAIN.id])]);
  listRoomTypes.mockResolvedValue([]);
  shareBuilding.mockResolvedValue(undefined);
  unshareBuilding.mockResolvedValue(undefined);
  deleteBuilding.mockResolvedValue(undefined);
  updateBuilding.mockImplementation((id, _rev, input) =>
    Promise.resolve({
      id,
      campusId: NORTH_MAIN.id,
      ...(input as Record<string, unknown>),
      ...STAMP,
    } as Building),
  );
  shareRoom.mockResolvedValue(undefined);
  unshareRoom.mockResolvedValue(undefined);
  deleteRoom.mockResolvedValue(undefined);
  updateRoom.mockImplementation((id, _rev, input) =>
    Promise.resolve({ id, ...(input as Record<string, unknown>), ...STAMP } as Room),
  );
});

/**
 * The Ownership cell of the single row in one table.
 *
 * Read by column header rather than by index, so adding a column cannot silently move
 * what this is asserting about.
 */
function ownershipCell(caption: string): HTMLElement {
  const table = screen.getByRole("table", { name: caption });
  const headers = within(table)
    .getAllByRole("columnheader")
    .map((header) => header.textContent);
  const column = headers.indexOf("Ownership");
  expect(column).toBeGreaterThan(-1);
  const row = within(table).getAllByRole("row")[1] as HTMLElement;
  return within(row).getAllByRole("cell")[column] as HTMLElement;
}

/** The share picker's own option list, never the "Based at" or "Building" selects. */
async function shareOptions(): Promise<string[]> {
  const listbox = await screen.findByRole("listbox", { name: "Options" });
  return within(listbox)
    .getAllByRole("option")
    .map((option) => option.textContent ?? "");
}

describe("sharing a building across schools", () => {
  it("offers another school's campus, labelled with the school that owns it", async () => {
    const user = userEvent.setup();
    render(<CampusesAndBuildings school={NORTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));

    // The capability the picker used to hide: `building_campus` never mentioned a school.
    expect(await shareOptions()).toEqual(["Riverside (SR) — Southmoor College"]);
  });

  it("shares with it, keeping one building with one identifier", async () => {
    const user = userEvent.setup();
    render(<CampusesAndBuildings school={NORTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));
    await user.click(
      within(await screen.findByRole("listbox", { name: "Options" })).getByRole("option", {
        name: /Riverside/,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareBuilding).toHaveBeenCalled());
    expect(shareBuilding).toHaveBeenCalledWith("b-sports", SOUTH_MAIN.id);
  });

  it("appears under the borrowing school's campus, as the same building", async () => {
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);

    expect(await screen.findByText("Sports Centre")).toBeTruthy();
    expect(screen.getAllByText("Sports Centre")).toHaveLength(1);
  });

  it("distinguishes owning it from having access to it", async () => {
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    const view = render(<CampusesAndBuildings school={NORTH} context={context()} />);
    await screen.findByText("Sports Centre");

    // Northgate owns it and lends it.
    await waitFor(() => expect(screen.getByText("lent")).toBeTruthy());
    expect(screen.getByText("This school")).toBeTruthy();

    view.unmount();
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    // Southmoor has access to it, and the row says whose it is. Read from the Ownership
    // cell: "shared" also appears in the Campuses cell, where it is a different
    // statement — that the building serves more campuses than this one.
    await waitFor(() =>
      expect(ownershipCell("Buildings on this campus").textContent).toContain(
        "Northgate Secondary",
      ),
    );
    expect(ownershipCell("Buildings on this campus").textContent).toContain("shared");
  });

  it("says nothing about ownership in a single-school campus file", async () => {
    // One institution, so there is nothing for an ownership column to tell apart.
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id])]);
    render(<CampusesAndBuildings school={NORTH} context={context({ schools: [NORTH] })} />);

    await screen.findByText("Sports Centre");
    const table = screen.getByRole("table", { name: "Buildings on this campus" });
    const headers = within(table)
      .getAllByRole("columnheader")
      .map((header) => header.textContent);
    expect(headers).not.toContain("Ownership");
  });

  it("does not offer the home campus as something to withdraw", async () => {
    const user = userEvent.setup();
    // Viewed from Southmoor, whose campus is a *share*: Northgate's is the home one and
    // withdrawing it would leave the building on no campus at all.
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));

    const options = await shareOptions();
    expect(options.some((label) => label.includes("Main Campus"))).toBe(false);
  });

  it("withdraws the borrowing school's own campus, and nothing else", async () => {
    const user = userEvent.setup();
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/No rooms are deleted or detached/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareBuilding).toHaveBeenCalled());
    // Either end's school may withdraw, and the acting school is named.
    expect(unshareBuilding).toHaveBeenCalledWith(SOUTH.id, "b-sports", SOUTH_MAIN.id);
    expect(deleteBuilding).not.toHaveBeenCalled();
  });

  it("never withdraws the home campus when a share is withdrawn from elsewhere", async () => {
    // The defect this stage fixed: the edit path took the *viewed* campus as home, so a
    // building shared into this campus had its own access silently left alone.
    const user = userEvent.setup();
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareBuilding).toHaveBeenCalledTimes(1));
    expect(unshareBuilding.mock.calls[0]?.[2]).not.toBe(NORTH_MAIN.id);
    // And the rename that went with it named the real home campus, not the viewed one.
    expect(updateBuilding.mock.calls[0]?.[2]).toMatchObject({ campusId: NORTH_MAIN.id });
  });

  it("explains instead of offering a delete while another school shares it", async () => {
    const user = userEvent.setup();
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={NORTH} context={context()} />);
    await screen.findByText("Sports Centre");
    await waitFor(() => expect(screen.getByText("lent")).toBeTruthy());

    await user.click(screen.getByRole("button", { name: /^Delete Sports Centre/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Southmoor College/)).toBeTruthy();
    expect(
      within(dialog).getByText(/not authority to destroy something another school/),
    ).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete building" })).toBeNull();
    expect(deleteBuilding).not.toHaveBeenCalled();
  });

  it("explains instead of offering a delete to the school that merely borrows it", async () => {
    const user = userEvent.setup();
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Delete Sports Centre/ }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/Only the school a building was created on may delete it/),
    ).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete building" })).toBeNull();
  });

  it("still offers a delete for a building nobody else shares", async () => {
    const user = userEvent.setup();
    render(<CampusesAndBuildings school={NORTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Delete Sports Centre/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Delete building" })).toBeTruthy();
  });

  it("shows a refusal from the backend rather than swallowing it", async () => {
    const user = userEvent.setup();
    listBuildings.mockResolvedValue([sportsCentre([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    unshareBuilding.mockRejectedValue({
      kind: "invalid",
      message: "3 rooms are still in Sports Centre on Riverside.",
    });
    render(<CampusesAndBuildings school={SOUTH} context={context()} />);
    await screen.findByText("Sports Centre");

    await user.click(screen.getByRole("button", { name: /^Edit Sports Centre/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText(/3 rooms are still in Sports Centre/)).toBeTruthy();
  });
});

describe("sharing a room across schools", () => {
  it("offers another school's campus, labelled with the school that owns it", async () => {
    const user = userEvent.setup();
    render(<RoomsAndRoomTypes school={NORTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Edit Great Hall/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));

    expect(await shareOptions()).toEqual(["Riverside (SR) — Southmoor College"]);
  });

  it("shares with it, keeping one room with one identifier", async () => {
    const user = userEvent.setup();
    render(<RoomsAndRoomTypes school={NORTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Edit Great Hall/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));
    await user.click(
      within(await screen.findByRole("listbox", { name: "Options" })).getByRole("option", {
        name: /Riverside/,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareRoom).toHaveBeenCalled());
    expect(shareRoom).toHaveBeenCalledWith("r-hall", SOUTH_MAIN.id);
  });

  it("appears under the borrowing school's campus, as the same room", async () => {
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={SOUTH} context={context()} />);

    expect(await screen.findByText("Great Hall")).toBeTruthy();
    // One room: room clashes are a property of `room_id`, so two rows would be two
    // occupancy records and neither would notice the clash.
    expect(screen.getAllByText("Great Hall")).toHaveLength(1);
  });

  it("distinguishes owning it from having access to it", async () => {
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={SOUTH} context={context()} />);
    await screen.findByText("Great Hall");

    await waitFor(() =>
      expect(ownershipCell("Rooms on this campus").textContent).toContain(
        "Northgate Secondary",
      ),
    );
    expect(ownershipCell("Rooms on this campus").textContent).toContain("shared");
  });

  it("does not offer the home campus as something to withdraw", async () => {
    const user = userEvent.setup();
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={SOUTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Edit Great Hall/ }));
    await user.click(screen.getByLabelText(/Shared with campuses/));

    const options = await shareOptions();
    expect(options.some((label) => label.includes("Main Campus"))).toBe(false);
  });

  it("withdraws the borrowing school's own campus, naming the acting school", async () => {
    const user = userEvent.setup();
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={SOUTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Edit Great Hall/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/keeps its identity/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareRoom).toHaveBeenCalled());
    expect(unshareRoom).toHaveBeenCalledWith(SOUTH.id, "r-hall", SOUTH_MAIN.id);
    expect(unshareRoom).toHaveBeenCalledTimes(1);
    expect(deleteRoom).not.toHaveBeenCalled();
  });

  it("explains instead of offering a delete while another school shares it", async () => {
    const user = userEvent.setup();
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={NORTH} context={context()} />);
    await screen.findByText("Great Hall");
    await waitFor(() => expect(screen.getByText("lent")).toBeTruthy());

    await user.click(screen.getByRole("button", { name: /^Delete Great Hall/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Southmoor College/)).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete room" })).toBeNull();
    expect(deleteRoom).not.toHaveBeenCalled();
  });

  it("explains instead of offering a delete to the school that merely borrows it", async () => {
    const user = userEvent.setup();
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={SOUTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Delete Great Hall/ }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/Only the school a room was created on may delete it/),
    ).toBeTruthy();
  });

  it("still offers a delete for a room nobody else shares", async () => {
    const user = userEvent.setup();
    render(<RoomsAndRoomTypes school={NORTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Delete Great Hall/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Delete room" })).toBeTruthy();
  });
});

describe("existing cross-school sharing", () => {
  it("is read and shown rather than needing to be re-made", async () => {
    // A campus file written before this stage can already hold cross-school associations,
    // because the repository has always allowed them. The share picker now shows them as
    // chosen rather than as an unmatched value.
    const user = userEvent.setup();
    listRooms.mockResolvedValue([hall([NORTH_MAIN.id, SOUTH_MAIN.id])]);
    render(<RoomsAndRoomTypes school={NORTH} context={context()} />);
    await screen.findByText("Great Hall");

    await user.click(screen.getByRole("button", { name: /^Edit Great Hall/ }));

    // The chip is the existing association, named with its school.
    expect(
      await screen.findByRole("button", { name: /^Remove Riverside \(SR\) — Southmoor/ }),
    ).toBeTruthy();
    expect(shareRoom).not.toHaveBeenCalled();
    expect(unshareRoom).not.toHaveBeenCalled();
  });
});
