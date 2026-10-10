/**
 * School Setup, across the IPC boundary.
 *
 * Every setup command is wrapped exactly once, here. Components call these functions and
 * never `invoke` directly, so the surface stays enumerable and mockable — and so the
 * command *name* and argument shape, the one part of the boundary that generated types
 * cannot check, is asserted in exactly one place and covered by tests.
 *
 * ## The types are generated
 *
 * Every entity, input and projection comes from `@evara/domain`, which is written by
 * `npm run types:generate` from the Rust definitions. Nothing in this file declares an
 * entity shape; a type here would be the second definition of something, which is what
 * ADR 0009 exists to prevent.
 *
 * ## Arguments are camelCase
 *
 * Tauri matches payload keys to Rust parameters with `rename_all = "camelCase"` by
 * default, so `campus_id` on the Rust side is `campusId` here.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  AcademicYear,
  AcademicYearInput,
  Building,
  BuildingInput,
  CalendarDay,
  CalendarDayInput,
  Campus,
  CampusInput,
  Cycle,
  CycleCoverage,
  CycleDay,
  CycleDayInput,
  CycleInput,
  GridPreview,
  MaterialisationPlan,
  MaterialisationRequest,
  Period,
  PeriodInput,
  PeriodStructure,
  PeriodStructureInput,
  PlanFingerprint,
  Resource,
  ResourceInput,
  Room,
  RoomInput,
  RoomType,
  RoomTypeInput,
  School,
  SchoolInput,
  SetupError,
  Term,
  TermInput,
  Timeslot,
} from "@evara/domain";

// ---------------------------------------------------------------------- error helpers

/** Narrows an unknown rejection to a {@link SetupError}. */
export function isSetupError(value: unknown): value is SetupError {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v["kind"] === "string" && typeof v["message"] === "string";
}

/**
 * Turns any rejection into something showable.
 *
 * A thrown value that is not a {@link SetupError} means a bug rather than a condition the
 * user caused, so it gets a deliberately generic message instead of leaking a stack.
 */
export function describeSetupError(error: unknown): string {
  if (isSetupError(error)) return error.message;
  return "Something went wrong. Please try again.";
}

/**
 * The form field a failure belongs to, when the backend named one.
 *
 * Lets a form mark the offending input rather than showing a banner and leaving the user
 * to work out which box is wrong. The backend's vocabulary is `code`, `name`, `label`,
 * `position` and `date`; a form maps those onto its own inputs.
 */
export function offendingField(error: unknown): string | null {
  return isSetupError(error) && error.kind === "duplicate" ? error.field : null;
}

/**
 * Whether the grid moved between a preview and a confirmation.
 *
 * The one failure the grid screen must handle specially: nothing was written, so the fix
 * is to re-read the plan and ask again rather than to report a problem.
 */
export function isReviewRequired(error: unknown): boolean {
  return isSetupError(error) && error.kind === "reviewRequired";
}

/** Whether the failure is a refusal to delete something other records depend on. */
export function isStillReferenced(error: unknown): boolean {
  return isSetupError(error) && error.kind === "stillReferenced";
}

// --------------------------------------------------------------------------- school

/** Creates the school record. Exactly one per project. */
export function createSchool(input: SchoolInput): Promise<School> {
  return invoke<School>("setup_school_create", { input });
}

/** The school record, or null when setup has not started. */
export function getSchool(): Promise<School | null> {
  return invoke<School | null>("setup_school_get");
}

/** Replaces the school's details. */
export function updateSchool(id: string, input: SchoolInput): Promise<School> {
  return invoke<School>("setup_school_update", { id, input });
}

// --------------------------------------------------------------------------- campus

/** Creates a campus. */
export function createCampus(input: CampusInput): Promise<Campus> {
  return invoke<Campus>("setup_campus_create", { input });
}

/** One campus by identifier. */
export function getCampus(id: string): Promise<Campus | null> {
  return invoke<Campus | null>("setup_campus_get", { id });
}

/** Every campus. */
export function listCampuses(): Promise<Campus[]> {
  return invoke<Campus[]>("setup_campus_list");
}

/** Replaces a campus's details. */
export function updateCampus(id: string, input: CampusInput): Promise<Campus> {
  return invoke<Campus>("setup_campus_update", { id, input });
}

/** Deletes a campus. Refused while it still owns buildings, rooms or resources. */
export function deleteCampus(id: string): Promise<void> {
  return invoke<void>("setup_campus_delete", { id });
}

// ------------------------------------------------------------------------- building

/** Creates a building on a campus. */
export function createBuilding(input: BuildingInput): Promise<Building> {
  return invoke<Building>("setup_building_create", { input });
}

/** One building by identifier. */
export function getBuilding(id: string): Promise<Building | null> {
  return invoke<Building | null>("setup_building_get", { id });
}

/** Every building on one campus. */
export function listBuildings(campusId: string): Promise<Building[]> {
  return invoke<Building[]>("setup_building_list", { campusId });
}

/** Replaces a building's details. */
export function updateBuilding(id: string, input: BuildingInput): Promise<Building> {
  return invoke<Building>("setup_building_update", { id, input });
}

/** Deletes a building. Its rooms survive, detached rather than deleted. */
export function deleteBuilding(id: string): Promise<void> {
  return invoke<void>("setup_building_delete", { id });
}

// ------------------------------------------------------------------------ room type

/** Creates a room type. */
export function createRoomType(input: RoomTypeInput): Promise<RoomType> {
  return invoke<RoomType>("setup_room_type_create", { input });
}

/** One room type by identifier. */
export function getRoomType(id: string): Promise<RoomType | null> {
  return invoke<RoomType | null>("setup_room_type_get", { id });
}

/** Every room type. */
export function listRoomTypes(): Promise<RoomType[]> {
  return invoke<RoomType[]>("setup_room_type_list");
}

/** Replaces a room type's details. */
export function updateRoomType(id: string, input: RoomTypeInput): Promise<RoomType> {
  return invoke<RoomType>("setup_room_type_update", { id, input });
}

/** Deletes a room type. Rooms that used it survive, unclassified. */
export function deleteRoomType(id: string): Promise<void> {
  return invoke<void>("setup_room_type_delete", { id });
}

// ----------------------------------------------------------------------------- room

/** Creates a room. */
export function createRoom(input: RoomInput): Promise<Room> {
  return invoke<Room>("setup_room_create", { input });
}

/** One room by identifier. */
export function getRoom(id: string): Promise<Room | null> {
  return invoke<Room | null>("setup_room_get", { id });
}

/** Every room on one campus. */
export function listRooms(campusId: string): Promise<Room[]> {
  return invoke<Room[]>("setup_room_list", { campusId });
}

/** Replaces a room's details. `buildingId` and `roomTypeId` may be null. */
export function updateRoom(id: string, input: RoomInput): Promise<Room> {
  return invoke<Room>("setup_room_update", { id, input });
}

/** Deletes a room. */
export function deleteRoom(id: string): Promise<void> {
  return invoke<void>("setup_room_delete", { id });
}

// ------------------------------------------------------------------------- resource

/** Creates a resource. */
export function createResource(input: ResourceInput): Promise<Resource> {
  return invoke<Resource>("setup_resource_create", { input });
}

/** One resource by identifier. */
export function getResource(id: string): Promise<Resource | null> {
  return invoke<Resource | null>("setup_resource_get", { id });
}

/** Every resource. */
export function listResources(): Promise<Resource[]> {
  return invoke<Resource[]>("setup_resource_list");
}

/** Replaces a resource's details. */
export function updateResource(id: string, input: ResourceInput): Promise<Resource> {
  return invoke<Resource>("setup_resource_update", { id, input });
}

/** Deletes a resource. */
export function deleteResource(id: string): Promise<void> {
  return invoke<void>("setup_resource_delete", { id });
}

// -------------------------------------------------------------------- academic year

/** Creates an academic year. */
export function createAcademicYear(input: AcademicYearInput): Promise<AcademicYear> {
  return invoke<AcademicYear>("setup_academic_year_create", { input });
}

/** One academic year by identifier. */
export function getAcademicYear(id: string): Promise<AcademicYear | null> {
  return invoke<AcademicYear | null>("setup_academic_year_get", { id });
}

/** Every academic year, earliest first. */
export function listAcademicYears(): Promise<AcademicYear[]> {
  return invoke<AcademicYear[]>("setup_academic_year_list");
}

/** Replaces an academic year's details. */
export function updateAcademicYear(
  id: string,
  input: AcademicYearInput,
): Promise<AcademicYear> {
  return invoke<AcademicYear>("setup_academic_year_update", { id, input });
}

/** Deletes an academic year. Refused while it still has terms. */
export function deleteAcademicYear(id: string): Promise<void> {
  return invoke<void>("setup_academic_year_delete", { id });
}

// ----------------------------------------------------------------------------- term

/** Creates a term within an academic year. */
export function createTerm(input: TermInput): Promise<Term> {
  return invoke<Term>("setup_term_create", { input });
}

/** One term by identifier. */
export function getTerm(id: string): Promise<Term | null> {
  return invoke<Term | null>("setup_term_get", { id });
}

/** Every term of one academic year, in teaching order. */
export function listTerms(academicYearId: string): Promise<Term[]> {
  return invoke<Term[]>("setup_term_list", { academicYearId });
}

/** Replaces a term's details. */
export function updateTerm(id: string, input: TermInput): Promise<Term> {
  return invoke<Term>("setup_term_update", { id, input });
}

/** Deletes a term. Refused while a timeslot or calendar day still names it. */
export function deleteTerm(id: string): Promise<void> {
  return invoke<void>("setup_term_delete", { id });
}

// ---------------------------------------------------------------------------- cycle

/** Creates a cycle. `dayCount` is the declared length, not a requirement to fill it. */
export function createCycle(input: CycleInput): Promise<Cycle> {
  return invoke<Cycle>("setup_cycle_create", { input });
}

/** One cycle by identifier. */
export function getCycle(id: string): Promise<Cycle | null> {
  return invoke<Cycle | null>("setup_cycle_get", { id });
}

/** Every cycle, the default first. */
export function listCycles(): Promise<Cycle[]> {
  return invoke<Cycle[]>("setup_cycle_list");
}

/** Replaces a cycle's details. */
export function updateCycle(id: string, input: CycleInput): Promise<Cycle> {
  return invoke<Cycle>("setup_cycle_update", { id, input });
}

/** Deletes a cycle. Refused while it still has days or timeslots. */
export function deleteCycle(id: string): Promise<void> {
  return invoke<void>("setup_cycle_delete", { id });
}

/**
 * Which of a cycle's declared positions have a day, and which are missing.
 *
 * The rule lives in Rust (`evara_core::time`). The interface asks; it does not decide.
 */
export function cycleCoverage(cycleId: string): Promise<CycleCoverage> {
  return invoke<CycleCoverage>("setup_cycle_coverage", { cycleId });
}

// ------------------------------------------------------------------------ cycle day

/** Adds a day to a cycle. */
export function createCycleDay(input: CycleDayInput): Promise<CycleDay> {
  return invoke<CycleDay>("setup_cycle_day_create", { input });
}

/** One cycle day by identifier. */
export function getCycleDay(id: string): Promise<CycleDay | null> {
  return invoke<CycleDay | null>("setup_cycle_day_get", { id });
}

/** Every day of one cycle, in position order. */
export function listCycleDays(cycleId: string): Promise<CycleDay[]> {
  return invoke<CycleDay[]>("setup_cycle_day_list", { cycleId });
}

/** Replaces a cycle day's details. */
export function updateCycleDay(id: string, input: CycleDayInput): Promise<CycleDay> {
  return invoke<CycleDay>("setup_cycle_day_update", { id, input });
}

/** Deletes a cycle day. Refused while a timeslot or mapped date still names it. */
export function deleteCycleDay(id: string): Promise<void> {
  return invoke<void>("setup_cycle_day_delete", { id });
}

// ----------------------------------------------------------------- period structure

/** Creates a bell schedule. `campusId` is null for a school-wide schedule. */
export function createPeriodStructure(
  input: PeriodStructureInput,
): Promise<PeriodStructure> {
  return invoke<PeriodStructure>("setup_period_structure_create", { input });
}

/** One bell schedule by identifier. */
export function getPeriodStructure(id: string): Promise<PeriodStructure | null> {
  return invoke<PeriodStructure | null>("setup_period_structure_get", { id });
}

/** Every bell schedule. */
export function listPeriodStructures(): Promise<PeriodStructure[]> {
  return invoke<PeriodStructure[]>("setup_period_structure_list");
}

/** Replaces a bell schedule's details. */
export function updatePeriodStructure(
  id: string,
  input: PeriodStructureInput,
): Promise<PeriodStructure> {
  return invoke<PeriodStructure>("setup_period_structure_update", { id, input });
}

/** Deletes a bell schedule. Refused while it still has periods. */
export function deletePeriodStructure(id: string): Promise<void> {
  return invoke<void>("setup_period_structure_delete", { id });
}

// --------------------------------------------------------------------------- period

/** Adds a period to a bell schedule. */
export function createPeriod(input: PeriodInput): Promise<Period> {
  return invoke<Period>("setup_period_create", { input });
}

/** One period by identifier. */
export function getPeriod(id: string): Promise<Period | null> {
  return invoke<Period | null>("setup_period_get", { id });
}

/** Every period of one bell schedule, in position order. */
export function listPeriods(periodStructureId: string): Promise<Period[]> {
  return invoke<Period[]>("setup_period_list", { periodStructureId });
}

/** Replaces a period's details. */
export function updatePeriod(id: string, input: PeriodInput): Promise<Period> {
  return invoke<Period>("setup_period_update", { id, input });
}

/** Deletes a period. Refused while a timeslot was materialised from it. */
export function deletePeriod(id: string): Promise<void> {
  return invoke<void>("setup_period_delete", { id });
}

// --------------------------------------------------------------------- calendar day

/**
 * Records what one real date is.
 *
 * `cycleDayId: null` means a day with no lessons. Evara does not infer weekends,
 * holidays or cycle progression — every mapping is one a human made.
 */
export function createCalendarDay(input: CalendarDayInput): Promise<CalendarDay> {
  return invoke<CalendarDay>("setup_calendar_day_create", { input });
}

/** One calendar day by identifier. */
export function getCalendarDay(id: string): Promise<CalendarDay | null> {
  return invoke<CalendarDay | null>("setup_calendar_day_get", { id });
}

/** Calendar days in an inclusive `YYYY-MM-DD` range, in date order. */
export function listCalendarDays(from: string, to: string): Promise<CalendarDay[]> {
  return invoke<CalendarDay[]>("setup_calendar_day_list", { from, to });
}

/** Replaces a calendar day's details. */
export function updateCalendarDay(
  id: string,
  input: CalendarDayInput,
): Promise<CalendarDay> {
  return invoke<CalendarDay>("setup_calendar_day_update", { id, input });
}

/** Deletes a calendar day. The dates around it are untouched. */
export function deleteCalendarDay(id: string): Promise<void> {
  return invoke<void>("setup_calendar_day_delete", { id });
}

// ----------------------------------------------------------------------------- grid

/**
 * What a rebuild would do to one grid, and the state the grid is in now.
 *
 * Writes nothing, so it is safe to call whenever the screen needs refreshing. The
 * `fingerprint` it returns is what {@link rebuildGrid} and {@link releaseOrphans} send
 * back to prove the confirmation belongs to these figures.
 */
export function previewGrid(request: MaterialisationRequest): Promise<GridPreview> {
  return invoke<GridPreview>("setup_grid_preview", { request });
}

/**
 * Creates the slots the grid is missing and refreshes the ones it has.
 *
 * Never deletes. Rejects with `reviewRequired` — nothing written — if the grid moved
 * since `reviewed` was issued.
 */
export function rebuildGrid(
  request: MaterialisationRequest,
  reviewed: PlanFingerprint,
): Promise<MaterialisationPlan> {
  return invoke<MaterialisationPlan>("setup_grid_rebuild", { request, reviewed });
}

/**
 * Deletes the slots this proposal strands. **The only destructive call here.**
 *
 * Separate from {@link rebuildGrid} on purpose: cleaning up after a removed day or period
 * is always a second, explicit decision, and the interface must show the list and take a
 * confirmation first.
 */
export function releaseOrphans(
  request: MaterialisationRequest,
  reviewed: PlanFingerprint,
): Promise<string[]> {
  return invoke<string[]>("setup_grid_release_orphans", { request, reviewed });
}

/** Every stored slot of one grid, in cycle order. */
export function listTimeslots(
  cycleId: string,
  termId: string | null,
): Promise<Timeslot[]> {
  return invoke<Timeslot[]>("setup_grid_timeslots", { cycleId, termId });
}
