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
  AvailabilityEntity,
  AvailabilityRule,
  AvailabilityRuleInput,
  Building,
  BuildingInput,
  BuildingWithCampuses,
  CalendarDay,
  CalendarDayInput,
  Campus,
  CampusInput,
  Cycle,
  CycleCoverage,
  CycleDay,
  CycleDayInput,
  CycleInput,
  CycleShape,
  GridPreview,
  LogoLimits,
  MaterialisationPlan,
  MaterialisationRequest,
  NonWorkingInstructionalDay,
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
  RoomWithCampuses,
  School,
  SchoolDependents,
  SchoolInput,
  SchoolLogoView,
  SetupError,
  Student,
  StudentEnrolment,
  StudentEnrolmentInput,
  StudentGroup,
  StudentGroupInput,
  StudentGroupMember,
  StudentGroupMemberInput,
  StudentInSchool,
  StudentInput,
  Teacher,
  TeacherCampus,
  TeacherInSchool,
  TeacherInput,
  TeacherSchool,
  TeacherSchoolInput,
  Term,
  TermInput,
  Timeslot,
  YearLevel,
  YearLevelInput,
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

/** Creates a school. A project may hold several since V12. */
export function createSchool(input: SchoolInput): Promise<School> {
  return invoke<School>("setup_school_create", { input });
}

/**
 * Every school in the project, ordered by name.
 *
 * An empty array drives the first-run empty state. There is deliberately no "get the
 * school": with more than one, a call like that would return an arbitrary institution and
 * silently scope everything downstream to it.
 */
export function listSchools(): Promise<School[]> {
  return invoke<School[]>("setup_school_list");
}

/** One school by identifier, or null if it is not there. */
export function getSchool(id: string): Promise<School | null> {
  return invoke<School | null>("setup_school_get", { id });
}

/** What a school still owns, so a confirmation can say why it cannot be deleted. */
export function schoolDependents(id: string): Promise<SchoolDependents> {
  return invoke<SchoolDependents>("setup_school_dependents", { id });
}

/** Deletes a school that owns nothing. Refused while it still owns anything. */
export function deleteSchool(id: string): Promise<void> {
  return invoke<void>("setup_school_delete", { id });
}

/** Replaces the school's details. */
export function updateSchool(id: string, input: SchoolInput): Promise<School> {
  return invoke<School>("setup_school_update", { id, input });
}

// ---------------------------------------------------------------------- school logo

/**
 * The school's logo, or `null` when it uses Evara's.
 *
 * Separate from {@link getSchool} on purpose: the image is up to 2 MiB, and `getSchool` is
 * called by almost every screen. Nothing pays for the bytes unless it is going to show
 * them.
 */
export function getSchoolLogo(schoolId: string): Promise<SchoolLogoView | null> {
  return invoke<SchoolLogoView | null>("setup_school_logo_get", { schoolId });
}

/**
 * Sets or replaces the school's logo from base64-encoded image bytes.
 *
 * No filename is sent and none is wanted. The backend decides the format from the content
 * — `evara_core::branding` — so a `.png` holding a JPEG is stored as a JPEG and a `.png`
 * holding something else is refused.
 */
export function setSchoolLogo(
  schoolId: string,
  dataBase64: string,
): Promise<SchoolLogoView> {
  return invoke<SchoolLogoView>("setup_school_logo_set", { schoolId, dataBase64 });
}

/** Removes the school's logo, restoring the Evara one. Removing nothing succeeds. */
export function clearSchoolLogo(schoolId: string): Promise<void> {
  return invoke<void>("setup_school_logo_clear", { schoolId });
}

/**
 * The limits a logo must satisfy.
 *
 * Fetched rather than hardcoded because `evara_core::branding` is the authority on every
 * one of these numbers, and a duplicated limit is a limit that drifts.
 */
export function getSchoolLogoLimits(): Promise<LogoLimits> {
  return invoke<LogoLimits>("setup_school_logo_limits");
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
export function listCampuses(schoolId: string): Promise<Campus[]> {
  return invoke<Campus[]>("setup_campus_list", { schoolId });
}

/** Replaces a campus's details. */
export function updateCampus(id: string, input: CampusInput): Promise<Campus> {
  return invoke<Campus>("setup_campus_update", { id, input });
}

/** Deletes a campus. Refused while it still owns buildings, rooms or resources. */
export function deleteCampus(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_campus_delete", { schoolId, id });
}

// ------------------------------------------------------------------------- building

/**
 * Creates a building on a campus, optionally shared with others.
 *
 * `sharedCampusIds` names the *additional* campuses. The building and every association
 * are written in one transaction, so a half-shared building is never persisted.
 */
export function createBuilding(
  input: BuildingInput,
  sharedCampusIds: string[] = [],
): Promise<Building> {
  return invoke<Building>("setup_building_create", { input, sharedCampusIds });
}

/** One building by identifier. */
export function getBuilding(id: string): Promise<Building | null> {
  return invoke<Building | null>("setup_building_get", { id });
}

/**
 * Every building available to one campus, shared ones included.
 *
 * Each entry carries the full set of campuses that building serves, so a listing can mark
 * a shared building without a call per row.
 */
export function listBuildings(campusId: string): Promise<BuildingWithCampuses[]> {
  return invoke<BuildingWithCampuses[]>("setup_building_list", { campusId });
}

/** Every campus one building serves. */
export function listBuildingCampuses(buildingId: string): Promise<string[]> {
  return invoke<string[]>("setup_building_campuses", { buildingId });
}

/** Replaces a building's details. Its home campus never changes. */
export function updateBuilding(id: string, input: BuildingInput): Promise<Building> {
  return invoke<Building>("setup_building_update", { id, input });
}

/** Shares an existing building with another campus. */
export function shareBuilding(buildingId: string, campusId: string): Promise<void> {
  return invoke<void>("setup_building_share", { buildingId, campusId });
}

/**
 * Stops sharing a building with a campus.
 *
 * Refused while rooms on that campus are still in the building, and refused outright for
 * the campus the building was created on. Nothing is detached or deleted either way.
 */
export function unshareBuilding(
  schoolId: string,
  buildingId: string,
  campusId: string,
): Promise<void> {
  return invoke<void>("setup_building_unshare", { schoolId, buildingId, campusId });
}

/** Deletes a building. Its rooms survive, detached rather than deleted. */
export function deleteBuilding(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_building_delete", { schoolId, id });
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
export function listRoomTypes(schoolId: string): Promise<RoomType[]> {
  return invoke<RoomType[]>("setup_room_type_list", { schoolId });
}

/** Replaces a room type's details. */
export function updateRoomType(id: string, input: RoomTypeInput): Promise<RoomType> {
  return invoke<RoomType>("setup_room_type_update", { id, input });
}

/** Deletes a room type. Rooms that used it survive, unclassified. */
export function deleteRoomType(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_room_type_delete", { schoolId, id });
}

// ----------------------------------------------------------------------------- room

/**
 * Creates a room on a campus, optionally shared with others.
 *
 * `sharedCampusIds` names the *additional* campuses. The room and every association are
 * written in one transaction, so a half-shared room is never persisted.
 */
export function createRoom(
  input: RoomInput,
  sharedCampusIds: string[] = [],
): Promise<Room> {
  return invoke<Room>("setup_room_create", { input, sharedCampusIds });
}

/** One room by identifier. */
export function getRoom(id: string): Promise<Room | null> {
  return invoke<Room | null>("setup_room_get", { id });
}

/**
 * Every room available to one campus, shared ones included.
 *
 * Each entry carries the full set of campuses that room serves, so a listing can mark a
 * shared room without a call per row.
 */
export function listRooms(campusId: string): Promise<RoomWithCampuses[]> {
  return invoke<RoomWithCampuses[]>("setup_room_list", { campusId });
}

/** Every campus one room serves. */
export function listRoomCampuses(roomId: string): Promise<string[]> {
  return invoke<string[]>("setup_room_campuses", { roomId });
}

/**
 * Replaces a room's details. `buildingId` and `roomTypeId` may be null.
 *
 * Refused when the new building is not available to a campus the room serves.
 */
export function updateRoom(id: string, input: RoomInput): Promise<Room> {
  return invoke<Room>("setup_room_update", { id, input });
}

/** Shares an existing room with another campus. */
export function shareRoom(roomId: string, campusId: string): Promise<void> {
  return invoke<void>("setup_room_share", { roomId, campusId });
}

/**
 * Stops sharing a room with a campus.
 *
 * Refused outright for the campus the room was created on. Nothing is moved, duplicated
 * or deleted — the room keeps its identity and every other campus keeps it.
 */
export function unshareRoom(
  schoolId: string,
  roomId: string,
  campusId: string,
): Promise<void> {
  return invoke<void>("setup_room_unshare", { schoolId, roomId, campusId });
}

/** Deletes a room. Its campus associations go with it. */
export function deleteRoom(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_room_delete", { schoolId, id });
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
export function listResources(schoolId: string): Promise<Resource[]> {
  return invoke<Resource[]>("setup_resource_list", { schoolId });
}

/** Replaces a resource's details. */
export function updateResource(id: string, input: ResourceInput): Promise<Resource> {
  return invoke<Resource>("setup_resource_update", { id, input });
}

/** Deletes a resource. */
export function deleteResource(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_resource_delete", { schoolId, id });
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
export function listAcademicYears(schoolId: string): Promise<AcademicYear[]> {
  return invoke<AcademicYear[]>("setup_academic_year_list", { schoolId });
}

/** Replaces an academic year's details. */
export function updateAcademicYear(
  id: string,
  input: AcademicYearInput,
): Promise<AcademicYear> {
  return invoke<AcademicYear>("setup_academic_year_update", { id, input });
}

/** Deletes an academic year. Refused while it still has terms. */
export function deleteAcademicYear(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_academic_year_delete", { schoolId, id });
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
export function deleteTerm(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_term_delete", { schoolId, id });
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
export function listCycles(schoolId: string): Promise<Cycle[]> {
  return invoke<Cycle[]>("setup_cycle_list", { schoolId });
}

/** Replaces a cycle's details. */
export function updateCycle(id: string, input: CycleInput): Promise<Cycle> {
  return invoke<Cycle>("setup_cycle_update", { id, input });
}

/** Deletes a cycle. Refused while it still has days or timeslots. */
export function deleteCycle(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_cycle_delete", { schoolId, id });
}

/**
 * Which of a cycle's declared positions have a day, and which are missing.
 *
 * The rule lives in Rust (`evara_core::time`). The interface asks; it does not decide.
 */
export function cycleCoverage(cycleId: string): Promise<CycleCoverage> {
  return invoke<CycleCoverage>("setup_cycle_coverage", { cycleId });
}

/**
 * What kind of cycle this is, how its rotation runs, and which of a weekly cycle's seven
 * positions a school keeping `workingDays` teaches on.
 *
 * One call rather than several, because the Cycles screen's questions are all about the
 * same cycle at the same moment.
 *
 * `workingDays` is sent rather than looked up, which is what keeps a shared cycle shared:
 * the backend compares it and writes nothing, so two schools asking about the same cycle
 * get two answers and the cycle holds neither.
 */
export function cycleShape(
  cycleId: string,
  workingDays: readonly number[],
): Promise<CycleShape> {
  return invoke<CycleShape>("setup_cycle_shape", { cycleId, workingDays });
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
export function deleteCycleDay(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_cycle_day_delete", { schoolId, id });
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
export function listPeriodStructures(schoolId: string): Promise<PeriodStructure[]> {
  return invoke<PeriodStructure[]>("setup_period_structure_list", { schoolId });
}

/** Replaces a bell schedule's details. */
export function updatePeriodStructure(
  id: string,
  input: PeriodStructureInput,
): Promise<PeriodStructure> {
  return invoke<PeriodStructure>("setup_period_structure_update", { id, input });
}

/** Deletes a bell schedule. Refused while it still has periods. */
export function deletePeriodStructure(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_period_structure_delete", { schoolId, id });
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
export function deletePeriod(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_period_delete", { schoolId, id });
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
export function listCalendarDays(
  schoolId: string,
  from: string,
  to: string,
): Promise<CalendarDay[]> {
  return invoke<CalendarDay[]>("setup_calendar_day_list", { schoolId, from, to });
}

/** Replaces a calendar day's details. */
export function updateCalendarDay(
  id: string,
  input: CalendarDayInput,
): Promise<CalendarDay> {
  return invoke<CalendarDay>("setup_calendar_day_update", { id, input });
}

/** Deletes a calendar day. The dates around it are untouched. */
export function deleteCalendarDay(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_calendar_day_delete", { schoolId, id });
}

/**
 * Mapped teaching dates that land on a weekday the school does not work.
 *
 * Advisory and read-only. Asking this never creates, deletes or rewrites a mapping —
 * `TimeModel::non_working_instructional_days` records why that has to stay true — so the
 * interface may call it as freely as it likes, including to preview a week before saving
 * one.
 *
 * `workingDays` is a list of ISO weekday numbers, 1 Monday through 7 Sunday. An empty list
 * is rejected by the backend rather than treated as "every day".
 */
export function nonWorkingInstructionalDays(
  schoolId: string,
  workingDays: readonly number[],
): Promise<NonWorkingInstructionalDay[]> {
  return invoke<NonWorkingInstructionalDay[]>("setup_calendar_non_working_days", {
    schoolId,
    workingDays,
  });
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
  schoolId: string,
  request: MaterialisationRequest,
  reviewed: PlanFingerprint,
): Promise<string[]> {
  return invoke<string[]>("setup_grid_release_orphans", {
    schoolId,
    request,
    reviewed,
  });
}

/** Every stored slot of one grid, in cycle order. */
export function listTimeslots(
  cycleId: string,
  termId: string | null,
): Promise<Timeslot[]> {
  return invoke<Timeslot[]>("setup_grid_timeslots", { cycleId, termId });
}

// ----------------------------------------------------------------------- year level

/** Creates a year level. */
export function createYearLevel(input: YearLevelInput): Promise<YearLevel> {
  return invoke<YearLevel>("setup_year_level_create", { input });
}

/** One year level by identifier. */
export function getYearLevel(id: string): Promise<YearLevel | null> {
  return invoke<YearLevel | null>("setup_year_level_get", { id });
}

/** Every year level, in the school's own progression order. */
export function listYearLevels(schoolId: string): Promise<YearLevel[]> {
  return invoke<YearLevel[]>("setup_year_level_list", { schoolId });
}

/** Replaces a year level's details. */
export function updateYearLevel(
  id: string,
  input: YearLevelInput,
): Promise<YearLevel> {
  return invoke<YearLevel>("setup_year_level_update", { id, input });
}

/** Deletes a year level, detaching anything classified by it. */
export function deleteYearLevel(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_year_level_delete", { schoolId, id });
}

// -------------------------------------------------------------------------- teacher

/** Creates a teacher. */
export function createTeacher(input: TeacherInput): Promise<Teacher> {
  return invoke<Teacher>("setup_teacher_create", { input });
}

/** One teacher by identifier. */
export function getTeacher(id: string): Promise<Teacher | null> {
  return invoke<Teacher | null>("setup_teacher_get", { id });
}

/**
 * Every teacher, active and inactive.
 *
 * Inactive teachers are included deliberately. Deciding who to *offer* is a screen's
 * decision; a list that hid them would also hide a teacher a timetable already names.
 */
export function listTeachers(schoolId: string): Promise<Teacher[]> {
  return invoke<Teacher[]>("setup_teacher_list", { schoolId });
}

/** Replaces a teacher's details. Their identity is unchanged. */
export function updateTeacher(id: string, input: TeacherInput): Promise<Teacher> {
  return invoke<Teacher>("setup_teacher_update", { id, input });
}

/** Deletes a teacher. */
export function deleteTeacher(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_teacher_delete", { schoolId, id });
}

// -------------------------------------------------------------------------- student

/** Creates a student. */
export function createStudent(input: StudentInput): Promise<Student> {
  return invoke<Student>("setup_student_create", { input });
}

/** One student by identifier. */
export function getStudent(id: string): Promise<Student | null> {
  return invoke<Student | null>("setup_student_get", { id });
}

/** Every student, enrolled and left. */
export function listStudents(schoolId: string): Promise<Student[]> {
  return invoke<Student[]>("setup_student_list", { schoolId });
}

/** Replaces a student's details. Their identity is unchanged. */
export function updateStudent(id: string, input: StudentInput): Promise<Student> {
  return invoke<Student>("setup_student_update", { id, input });
}

/**
 * Deletes a student who belongs to no group.
 *
 * Rejects when they are still in one; {@link listGroupsForStudent} is how a screen finds
 * that out before offering the action rather than after it.
 */
export function deleteStudent(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_student_delete", { schoolId, id });
}

// -------------------------------------------------------------------- student group

/** Creates a student group. */
export function createStudentGroup(
  input: StudentGroupInput,
): Promise<StudentGroup> {
  return invoke<StudentGroup>("setup_student_group_create", { input });
}

/** One student group by identifier. */
export function getStudentGroup(id: string): Promise<StudentGroup | null> {
  return invoke<StudentGroup | null>("setup_student_group_get", { id });
}

/** Every student group, ordered by name. */
export function listStudentGroups(schoolId: string): Promise<StudentGroup[]> {
  return invoke<StudentGroup[]>("setup_student_group_list", { schoolId });
}

/** Replaces a group's details. */
export function updateStudentGroup(
  id: string,
  input: StudentGroupInput,
): Promise<StudentGroup> {
  return invoke<StudentGroup>("setup_student_group_update", { id, input });
}

/** Deletes a group and the memberships that only existed because of it. */
export function deleteStudentGroup(schoolId: string, id: string): Promise<void> {
  return invoke<void>("setup_student_group_delete", { schoolId, id });
}

// ----------------------------------------------------------------------- membership

/** Adds a student to a group. */
export function addGroupMember(
  input: StudentGroupMemberInput,
): Promise<StudentGroupMember> {
  return invoke<StudentGroupMember>("setup_student_group_member_add", { input });
}

/** Removes one student from one group. The student is untouched. */
export function removeGroupMember(
  schoolId: string,
  studentGroupId: string,
  studentId: string,
): Promise<void> {
  return invoke<void>("setup_student_group_member_remove", {
    schoolId,
    studentGroupId,
    studentId,
  });
}

/** Every student in one group, ordered by name. */
export function listGroupMembers(studentGroupId: string): Promise<Student[]> {
  return invoke<Student[]>("setup_student_group_member_list", { studentGroupId });
}

/** How many students are in one group, for a confirmation that says so before it asks. */
export function countGroupMembers(studentGroupId: string): Promise<number> {
  return invoke<number>("setup_student_group_member_count", { studentGroupId });
}

/** Every group one student belongs to. */
export function listGroupsForStudent(studentId: string): Promise<StudentGroup[]> {
  return invoke<StudentGroup[]>("setup_student_groups_for_student", { studentId });
}

// --------------------------------------------------------------------- availability

/**
 * Writes an availability rule, replacing whatever already covers the same scope.
 *
 * There is no separate create and update, because the grid does not have those two
 * actions: a cell is set or cleared, and whether that is an insert or an edit is the
 * repository's business. It keys on the scope tuple, so the existing rule keeps its
 * identity and gains a revision.
 */
export function setAvailability(
  input: AvailabilityRuleInput,
): Promise<AvailabilityRule> {
  return invoke<AvailabilityRule>("setup_availability_set", { input });
}

/**
 * Removes the rule covering one scope.
 *
 * Resolves `false` when there was none. That is not an error: the grid sends the same
 * request whichever way a cell was toggled, and "nothing to remove" is the right outcome.
 */
export function clearAvailability(
  scope: AvailabilityRuleInput,
): Promise<boolean> {
  return invoke<boolean>("setup_availability_clear", { scope });
}

/** Every availability rule for one entity. */
export function listAvailability(
  schoolId: string,
  entityKind: AvailabilityEntity,
  entityId: string,
): Promise<AvailabilityRule[]> {
  return invoke<AvailabilityRule[]>("setup_availability_list", {
    schoolId,
    entityKind,
    entityId,
  });
}

/** How many rules one entity has, for a listing that marks who has been configured. */
export function countAvailability(
  schoolId: string,
  entityKind: AvailabilityEntity,
  entityId: string,
): Promise<number> {
  return invoke<number>("setup_availability_count", {
    schoolId,
    entityKind,
    entityId,
  });
}

/**
 * Rules whose entity no longer exists.
 *
 * The integrity sweep for the one reference the database cannot enforce. Nothing in
 * normal operation should ever see a non-empty result.
 */
export function availabilityOrphans(): Promise<AvailabilityRule[]> {
  return invoke<AvailabilityRule[]>("setup_availability_orphans");
}

// =================================================== shared identities — Stage 0B
//
// A teacher and a student are one record each, for one real person. Which schools employ
// or enrol them is a relationship, and a project may hold several per person — so these
// functions are about relationships, never about creating a second copy of somebody.
//
// Every mutation names the school it acts for. Holding a shared person's identifier is
// ordinary now, and is not authority to touch another school's employment of them; the
// backend refuses it with `WrongSchool`. `transferStudent` is the one exception, and names
// both schools, because a transfer is a single fact about two institutions.

// --------------------------------------------------------------------- school membership

/** Adds an existing teacher to another school. Creates no teacher record. */
export function addTeacherToSchool(input: TeacherSchoolInput): Promise<TeacherSchool> {
  return invoke<TeacherSchool>("setup_teacher_membership_add", { input });
}

/**
 * Every school one teacher belongs to.
 *
 * The complete answer. `teacher.schoolId` names only the school that created them and is
 * never the whole membership set.
 */
export function listTeacherMemberships(teacherId: string): Promise<TeacherSchool[]> {
  return invoke<TeacherSchool[]>("setup_teacher_membership_list", { teacherId });
}

/**
 * Every teacher one school employs, with that school's own view of their employment.
 *
 * A shared teacher appears here as the same record, with the same identifier, that the
 * other school sees. Use this rather than `listTeachers` wherever a screen shows
 * per-school status: the plain list carries only the project-level flag.
 */
export function listSchoolTeachers(schoolId: string): Promise<TeacherInSchool[]> {
  return invoke<TeacherInSchool[]>("setup_school_teacher_list", { schoolId });
}

/** Activates or deactivates one school's employment of a teacher. Not a delete. */
export function setTeacherMembershipActive(
  schoolId: string,
  teacherId: string,
  isActive: boolean,
): Promise<TeacherSchool> {
  return invoke<TeacherSchool>("setup_teacher_membership_set_active", {
    schoolId,
    teacherId,
    isActive,
  });
}

/** Withdraws one school's employment of a teacher. The person is untouched. */
export function removeTeacherFromSchool(schoolId: string, teacherId: string): Promise<void> {
  return invoke<void>("setup_teacher_membership_remove", { schoolId, teacherId });
}

// -------------------------------------------------------------------- campus eligibility

/** Every campus one teacher may be based at, across every school they belong to. */
export function listTeacherCampuses(teacherId: string): Promise<string[]> {
  return invoke<string[]>("setup_teacher_campus_list", { teacherId });
}

/** Makes a teacher eligible at one of the acting school's campuses. */
export function assignTeacherCampus(
  schoolId: string,
  teacherId: string,
  campusId: string,
): Promise<TeacherCampus> {
  return invoke<TeacherCampus>("setup_teacher_campus_assign", {
    schoolId,
    teacherId,
    campusId,
  });
}

/** Withdraws a teacher's eligibility for one campus. Their membership is untouched. */
export function removeTeacherCampus(
  schoolId: string,
  teacherId: string,
  campusId: string,
): Promise<void> {
  return invoke<void>("setup_teacher_campus_remove", { schoolId, teacherId, campusId });
}

// ---------------------------------------------------------------------- student enrolment

/** Enrols an existing student at a school. Creates no student record. */
export function enrolStudent(input: StudentEnrolmentInput): Promise<StudentEnrolment> {
  return invoke<StudentEnrolment>("setup_enrolment_create", { input });
}

/** Every student currently at one school, with that school's placement for them. */
export function listSchoolStudents(schoolId: string): Promise<StudentInSchool[]> {
  return invoke<StudentInSchool[]>("setup_school_student_list", { schoolId });
}

/**
 * Every spell one student has ever had, at every school, oldest first.
 *
 * A transfer keeps the previous spell rather than rewriting it, so this is where "was she
 * ever at Northgate?" is answered.
 */
export function listEnrolmentHistory(studentId: string): Promise<StudentEnrolment[]> {
  return invoke<StudentEnrolment[]>("setup_enrolment_history", { studentId });
}

/** One student's current spell at one school, or `null`. */
export function currentEnrolment(
  schoolId: string,
  studentId: string,
): Promise<StudentEnrolment | null> {
  return invoke<StudentEnrolment | null>("setup_enrolment_current", { schoolId, studentId });
}

/** Replaces one spell's placement, status and dates. */
export function updateEnrolment(
  schoolId: string,
  id: string,
  input: StudentEnrolmentInput,
): Promise<StudentEnrolment> {
  return invoke<StudentEnrolment>("setup_enrolment_update", { schoolId, id, input });
}

/** Records that a student has left one school. The spell is kept, not deleted. */
export function leaveSchool(
  schoolId: string,
  studentId: string,
  leftOn: string | null,
): Promise<StudentEnrolment> {
  return invoke<StudentEnrolment>("setup_enrolment_leave", { schoolId, studentId, leftOn });
}

/**
 * Moves a student between two schools, preserving the identity and both spells.
 *
 * Names both schools deliberately: a transfer is one fact about two institutions, and
 * splitting it would let a student end up enrolled nowhere.
 */
export function transferStudent(
  fromSchoolId: string,
  toSchoolId: string,
  studentId: string,
  arriving: StudentEnrolmentInput,
): Promise<StudentEnrolment> {
  return invoke<StudentEnrolment>("setup_student_transfer", {
    fromSchoolId,
    toSchoolId,
    studentId,
    arriving,
  });
}

// ----------------------------------------------------------------------- resource sharing

/** Every campus one resource serves. Possibly empty: a resource may be school-wide. */
export function listResourceCampuses(resourceId: string): Promise<string[]> {
  return invoke<string[]>("setup_resource_campus_list", { resourceId });
}

/** Shares a resource with a campus, which may belong to another school. */
export function shareResource(
  schoolId: string,
  resourceId: string,
  campusId: string,
): Promise<void> {
  return invoke<void>("setup_resource_share", { schoolId, resourceId, campusId });
}

/** Stops sharing a resource with a campus. Nothing is moved or duplicated. */
export function unshareResource(
  schoolId: string,
  resourceId: string,
  campusId: string,
): Promise<void> {
  return invoke<void>("setup_resource_unshare", { schoolId, resourceId, campusId });
}
