/**
 * The School Setup IPC wrappers.
 *
 * ## Why these tests exist at all, given the types are generated
 *
 * ADR 0009 chose `ts-rs` over `tauri-specta`, and named the cost: the command *name* and
 * the argument *shape* stay unchecked across the boundary. A generated payload type
 * cannot catch `setup_campus_lst`, and it cannot catch sending `{ campus_id }` where the
 * backend expects `{ campusId }` — both compile, and both fail only when a human clicks
 * the thing. These tests are the agreed price of that choice: one assertion per command,
 * on the name it sends and the keys it sends.
 *
 * There is no Tauri runtime under Vitest, so `invoke` is mocked.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const setup = await import("./setup");

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
});

const ID = "018f3a2c-0000-7000-8000-000000000001";
const OTHER = "018f3a2c-0000-7000-8000-000000000002";

describe("command names and payloads", () => {
  /**
   * Every list command: no arguments, or exactly the scoping key the Rust parameter is
   * named after. Tauri matches payload keys to parameters as camelCase, so `campus_id`
   * on the Rust side must be sent as `campusId` — the mistake this table catches.
   */
  it.each([
    ["setup_school_get", () => setup.getSchool(), undefined],
    ["setup_campus_list", () => setup.listCampuses(), undefined],
    ["setup_room_type_list", () => setup.listRoomTypes(), undefined],
    ["setup_resource_list", () => setup.listResources(), undefined],
    ["setup_academic_year_list", () => setup.listAcademicYears(), undefined],
    ["setup_cycle_list", () => setup.listCycles(), undefined],
    ["setup_period_structure_list", () => setup.listPeriodStructures(), undefined],
    ["setup_building_list", () => setup.listBuildings(ID), { campusId: ID }],
    ["setup_room_list", () => setup.listRooms(ID), { campusId: ID }],
    ["setup_term_list", () => setup.listTerms(ID), { academicYearId: ID }],
    ["setup_cycle_day_list", () => setup.listCycleDays(ID), { cycleId: ID }],
    ["setup_period_list", () => setup.listPeriods(ID), { periodStructureId: ID }],
    ["setup_cycle_coverage", () => setup.cycleCoverage(ID), { cycleId: ID }],
    [
      "setup_calendar_day_list",
      () => setup.listCalendarDays("2027-09-01", "2028-07-20"),
      { from: "2027-09-01", to: "2028-07-20" },
    ],
  ])("%s sends the right payload", async (command, call, payload) => {
    await call();
    if (payload === undefined) {
      expect(invoke).toHaveBeenCalledWith(command);
    } else {
      expect(invoke).toHaveBeenCalledWith(command, payload);
    }
  });

  /** Every delete: the identifier, and nothing else. */
  it.each([
    ["setup_campus_delete", () => setup.deleteCampus(ID)],
    ["setup_building_delete", () => setup.deleteBuilding(ID)],
    ["setup_room_type_delete", () => setup.deleteRoomType(ID)],
    ["setup_room_delete", () => setup.deleteRoom(ID)],
    ["setup_resource_delete", () => setup.deleteResource(ID)],
    ["setup_academic_year_delete", () => setup.deleteAcademicYear(ID)],
    ["setup_term_delete", () => setup.deleteTerm(ID)],
    ["setup_cycle_delete", () => setup.deleteCycle(ID)],
    ["setup_cycle_day_delete", () => setup.deleteCycleDay(ID)],
    ["setup_period_structure_delete", () => setup.deletePeriodStructure(ID)],
    ["setup_period_delete", () => setup.deletePeriod(ID)],
    ["setup_calendar_day_delete", () => setup.deleteCalendarDay(ID)],
  ])("%s sends only the identifier", async (command, call) => {
    await call();
    expect(invoke).toHaveBeenCalledWith(command, { id: ID });
  });

  /** Every single-record read. */
  it.each([
    ["setup_campus_get", () => setup.getCampus(ID)],
    ["setup_building_get", () => setup.getBuilding(ID)],
    ["setup_room_type_get", () => setup.getRoomType(ID)],
    ["setup_room_get", () => setup.getRoom(ID)],
    ["setup_resource_get", () => setup.getResource(ID)],
    ["setup_academic_year_get", () => setup.getAcademicYear(ID)],
    ["setup_term_get", () => setup.getTerm(ID)],
    ["setup_cycle_get", () => setup.getCycle(ID)],
    ["setup_cycle_day_get", () => setup.getCycleDay(ID)],
    ["setup_period_structure_get", () => setup.getPeriodStructure(ID)],
    ["setup_period_get", () => setup.getPeriod(ID)],
    ["setup_calendar_day_get", () => setup.getCalendarDay(ID)],
  ])("%s reads one record by identifier", async (command, call) => {
    await call();
    expect(invoke).toHaveBeenCalledWith(command, { id: ID });
  });

  it("wraps a create input under `input`, matching the Rust parameter", async () => {
    await setup.createCampus({
      schoolId: ID,
      name: "Main",
      code: "MAIN",
      address: null,
    });
    expect(invoke).toHaveBeenCalledWith("setup_campus_create", {
      input: { schoolId: ID, name: "Main", code: "MAIN", address: null },
    });
  });

  it("sends both the identifier and the input on an update", async () => {
    await setup.updateCampus(ID, {
      schoolId: OTHER,
      name: "Main",
      code: "MAIN",
      address: null,
    });
    expect(invoke).toHaveBeenCalledWith("setup_campus_update", {
      id: ID,
      input: { schoolId: OTHER, name: "Main", code: "MAIN", address: null },
    });
  });

  it("sends nulls for cleared optional references rather than omitting them", async () => {
    // Omitting a key would make serde fail on a non-optional field, and would be
    // indistinguishable from "leave it alone" — which these commands never mean.
    await setup.updateRoom(ID, {
      campusId: OTHER,
      buildingId: null,
      roomTypeId: null,
      name: "Lab 1",
      code: "L1",
      capacity: 24,
      isBookable: true,
      notes: null,
    });
    const payload = invoke.mock.calls[0]?.[1] as { input: Record<string, unknown> };
    expect(Object.keys(payload.input)).toContain("buildingId");
    expect(payload.input["buildingId"]).toBeNull();
    expect(payload.input["roomTypeId"]).toBeNull();
  });
});

describe("the grid commands", () => {
  const request = {
    cycleId: ID,
    periodStructureId: OTHER,
    termId: null,
    withoutCycleDays: [],
    withoutPeriods: [],
  };

  it("previews without sending a fingerprint", async () => {
    await setup.previewGrid(request);
    expect(invoke).toHaveBeenCalledWith("setup_grid_preview", { request });
  });

  it("sends the reviewed fingerprint back with a rebuild", async () => {
    await setup.rebuildGrid(request, "a1b2c3d4e5f60718");
    expect(invoke).toHaveBeenCalledWith("setup_grid_rebuild", {
      request,
      reviewed: "a1b2c3d4e5f60718",
    });
  });

  it("sends the reviewed fingerprint back with an orphan release", async () => {
    await setup.releaseOrphans(request, "a1b2c3d4e5f60718");
    expect(invoke).toHaveBeenCalledWith("setup_grid_release_orphans", {
      request,
      reviewed: "a1b2c3d4e5f60718",
    });
  });

  it("carries exclusions through unchanged, so a proposal is what it says", async () => {
    const proposal = {
      ...request,
      withoutCycleDays: ["day-1"],
      withoutPeriods: ["period-3", "period-4"],
    };
    await setup.previewGrid(proposal);
    expect(invoke).toHaveBeenCalledWith("setup_grid_preview", { request: proposal });
  });

  it("releasing and rebuilding are different commands", () => {
    // The guarantee the whole screen rests on: destruction cannot be reached by passing a
    // flag to the safe call. If these two names were ever the same, that would be gone.
    expect(setup.rebuildGrid).not.toBe(setup.releaseOrphans);
  });

  it("reads the stored slots of one grid, term included", async () => {
    await setup.listTimeslots(ID, OTHER);
    expect(invoke).toHaveBeenCalledWith("setup_grid_timeslots", {
      cycleId: ID,
      termId: OTHER,
    });
  });

  it("sends a null term for a year-wide grid rather than omitting it", async () => {
    await setup.listTimeslots(ID, null);
    expect(invoke).toHaveBeenCalledWith("setup_grid_timeslots", {
      cycleId: ID,
      termId: null,
    });
  });
});

describe("the error contract", () => {
  it("narrows a tagged backend failure", () => {
    expect(setup.isSetupError({ kind: "notFound", message: "gone" })).toBe(true);
    expect(setup.isSetupError({ kind: "notFound" })).toBe(false);
    expect(setup.isSetupError("notFound")).toBe(false);
    expect(setup.isSetupError(null)).toBe(false);
    expect(setup.isSetupError(new Error("boom"))).toBe(false);
  });

  it("shows the backend's own wording, which is the single source of it", () => {
    expect(
      setup.describeSetupError({
        kind: "stillReferenced",
        entity: "campus",
        message: "This campus cannot be deleted while other records depend on it.",
      }),
    ).toMatch(/cannot be deleted/);
  });

  it("does not leak an unexpected throw to the user", () => {
    // A non-SetupError rejection is a bug in the frontend, not something the user did.
    expect(setup.describeSetupError(new Error("ReferenceError: x is not defined"))).toBe(
      "Something went wrong. Please try again.",
    );
    expect(setup.describeSetupError(undefined)).toMatch(/Something went wrong/);
  });

  it("names the offending field on a duplicate, and only then", () => {
    expect(
      setup.offendingField({
        kind: "duplicate",
        entity: "campus",
        field: "code",
        message: "taken",
      }),
    ).toBe("code");
    expect(setup.offendingField({ kind: "invalid", message: "no" })).toBeNull();
    expect(setup.offendingField(new Error("boom"))).toBeNull();
  });

  it("recognises a stale plan, which the grid screen handles differently", () => {
    expect(setup.isReviewRequired({ kind: "reviewRequired", message: "re-read" })).toBe(true);
    expect(setup.isReviewRequired({ kind: "invalid", message: "no" })).toBe(false);
    expect(setup.isReviewRequired({ kind: "failed", message: "no" })).toBe(false);
  });

  it("recognises a RESTRICT refusal", () => {
    expect(
      setup.isStillReferenced({ kind: "stillReferenced", entity: "campus", message: "x" }),
    ).toBe(true);
    expect(setup.isStillReferenced({ kind: "notFound", entity: "campus", message: "x" })).toBe(
      false,
    );
  });
});

describe("rejections", () => {
  it("propagates a backend failure rather than swallowing it", async () => {
    invoke.mockRejectedValue({ kind: "duplicate", entity: "campus", field: "code", message: "taken" });
    await expect(
      setup.createCampus({ schoolId: ID, name: "Main", code: "MAIN", address: null }),
    ).rejects.toMatchObject({ kind: "duplicate", field: "code" });
  });

  it("propagates a review-required rejection from a rebuild", async () => {
    invoke.mockRejectedValue({ kind: "reviewRequired", message: "the grid changed" });
    const request = {
      cycleId: ID,
      periodStructureId: OTHER,
      termId: null,
      withoutCycleDays: [],
      withoutPeriods: [],
    };
    await expect(setup.rebuildGrid(request, "stale")).rejects.toMatchObject({
      kind: "reviewRequired",
    });

    // And the helper the grid screen branches on agrees about it.
    await setup.rebuildGrid(request, "stale").catch((e: unknown) => {
      expect(setup.isReviewRequired(e)).toBe(true);
    });
  });
});
