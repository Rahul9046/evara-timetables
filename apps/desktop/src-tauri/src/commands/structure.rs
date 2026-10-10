//! School Setup commands for the static Structure entities.
//!
//! School details, campuses, buildings, room types, rooms, resources, academic years and
//! terms. The time half — cycles, bell schedules, the calendar and the grid — is
//! [`super::time_model`] and [`super::grid`].
//!
//! # Every handler is three lines, and that is the design
//!
//! Lock the workspace, call one repository method, convert the error. No validation, no
//! defaulting, no ordering and no uniqueness checking happens here: all of it is in
//! `evara-db`, where the CLI and the tests can reach it too. A rule implemented in a
//! command handler would be unreachable from `cargo run -p evara-cli`, which is exactly
//! the architecture bug `CLAUDE.md` names.
//!
//! The repetition is deliberate and mirrors the repository's own shape — `campus` was
//! written first and the rest follow it, so a reviewer checks one and skims the others. A
//! macro would save lines at the cost of making `tauri::generate_handler!` and the error
//! messages harder to follow.
//!
//! # No `delete_school`
//!
//! There is exactly one school per project, and deleting it would mean deleting the
//! project's entire contents through a cascade the schema deliberately does not have. The
//! repository offers no such call and neither does this module; the way to discard a
//! school is to discard the project file.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    AcademicYear, AcademicYearInput, Building, BuildingInput, Campus, CampusInput, Resource,
    ResourceInput, Room, RoomInput, RoomType, RoomTypeInput, School, SchoolInput, Term, TermInput,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ---------------------------------------------------------------------------- school

/// Creates the school record. Exactly one per project.
#[tauri::command]
pub fn setup_school_create(state: State<'_, AppState>, input: SchoolInput) -> SetupResult<School> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_school(&input)?)
}

/// The school record, or `null` when setup has not started.
///
/// `null` is what drives the first-run empty state: there is no school yet, so the only
/// sensible next action is to describe one.
#[tauri::command]
pub fn setup_school_get(state: State<'_, AppState>) -> SetupResult<Option<School>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.school()?)
}

/// Replaces the school's details.
#[tauri::command]
pub fn setup_school_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: SchoolInput,
) -> SetupResult<School> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_school(id, &input)?)
}

// ---------------------------------------------------------------------------- campus

/// Creates a campus.
#[tauri::command]
pub fn setup_campus_create(state: State<'_, AppState>, input: CampusInput) -> SetupResult<Campus> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_campus(&input)?)
}

/// One campus by identifier.
#[tauri::command]
pub fn setup_campus_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Campus>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.campus(id)?)
}

/// Every campus.
#[tauri::command]
pub fn setup_campus_list(state: State<'_, AppState>) -> SetupResult<Vec<Campus>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.campuses()?)
}

/// Replaces a campus's details.
#[tauri::command]
pub fn setup_campus_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: CampusInput,
) -> SetupResult<Campus> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_campus(id, &input)?)
}

/// Deletes a campus.
///
/// Refused while it still owns buildings, rooms or resources — ownership edges are
/// `RESTRICT` by [ADR 0010](../../../../docs/adr/0010-structure-deletion-semantics.md), so
/// the user removes or reassigns those first.
#[tauri::command]
pub fn setup_campus_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_campus(id)?)
}

// -------------------------------------------------------------------------- building

/// Creates a building on a campus.
#[tauri::command]
pub fn setup_building_create(
    state: State<'_, AppState>,
    input: BuildingInput,
) -> SetupResult<Building> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_building(&input)?)
}

/// One building by identifier.
#[tauri::command]
pub fn setup_building_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Building>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.building(id)?)
}

/// Every building on one campus.
#[tauri::command]
pub fn setup_building_list(
    state: State<'_, AppState>,
    campus_id: Uuid,
) -> SetupResult<Vec<Building>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.buildings(campus_id)?)
}

/// Replaces a building's details.
#[tauri::command]
pub fn setup_building_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: BuildingInput,
) -> SetupResult<Building> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_building(id, &input)?)
}

/// Deletes a building, detaching its rooms rather than deleting them.
///
/// A room's `building_id` is an optional classification, so ADR 0010 makes it `SET NULL`
/// — through a real repository update, so each affected room's revision stays truthful.
/// The rooms themselves survive; they simply stop naming a building.
#[tauri::command]
pub fn setup_building_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_building(id)?)
}

// ------------------------------------------------------------------------- room type

/// Creates a room type.
#[tauri::command]
pub fn setup_room_type_create(
    state: State<'_, AppState>,
    input: RoomTypeInput,
) -> SetupResult<RoomType> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_room_type(&input)?)
}

/// One room type by identifier.
#[tauri::command]
pub fn setup_room_type_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<RoomType>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room_type(id)?)
}

/// Every room type.
#[tauri::command]
pub fn setup_room_type_list(state: State<'_, AppState>) -> SetupResult<Vec<RoomType>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room_types()?)
}

/// Replaces a room type's details.
#[tauri::command]
pub fn setup_room_type_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: RoomTypeInput,
) -> SetupResult<RoomType> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_room_type(id, &input)?)
}

/// Deletes a room type, unclassifying the rooms that used it.
///
/// `SET NULL` for the same reason as a building: a classification is not ownership, so
/// losing the category must not lose the room.
#[tauri::command]
pub fn setup_room_type_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_room_type(id)?)
}

// ------------------------------------------------------------------------------ room

/// Creates a room.
#[tauri::command]
pub fn setup_room_create(state: State<'_, AppState>, input: RoomInput) -> SetupResult<Room> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_room(&input)?)
}

/// One room by identifier.
#[tauri::command]
pub fn setup_room_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Room>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room(id)?)
}

/// Every room on one campus.
#[tauri::command]
pub fn setup_room_list(state: State<'_, AppState>, campus_id: Uuid) -> SetupResult<Vec<Room>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.rooms(campus_id)?)
}

/// Replaces a room's details.
///
/// `buildingId` and `roomTypeId` stay nullable here: clearing either is a normal edit, not
/// a deletion, and the repository accepts `null` for both.
#[tauri::command]
pub fn setup_room_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: RoomInput,
) -> SetupResult<Room> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_room(id, &input)?)
}

/// Deletes a room.
#[tauri::command]
pub fn setup_room_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_room(id)?)
}

// -------------------------------------------------------------------------- resource

/// Creates a resource.
#[tauri::command]
pub fn setup_resource_create(
    state: State<'_, AppState>,
    input: ResourceInput,
) -> SetupResult<Resource> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_resource(&input)?)
}

/// One resource by identifier.
#[tauri::command]
pub fn setup_resource_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Resource>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.resource(id)?)
}

/// Every resource.
#[tauri::command]
pub fn setup_resource_list(state: State<'_, AppState>) -> SetupResult<Vec<Resource>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.resources()?)
}

/// Replaces a resource's details.
#[tauri::command]
pub fn setup_resource_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: ResourceInput,
) -> SetupResult<Resource> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_resource(id, &input)?)
}

/// Deletes a resource.
#[tauri::command]
pub fn setup_resource_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_resource(id)?)
}

// --------------------------------------------------------------------- academic year

/// Creates an academic year.
#[tauri::command]
pub fn setup_academic_year_create(
    state: State<'_, AppState>,
    input: AcademicYearInput,
) -> SetupResult<AcademicYear> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_academic_year(&input)?)
}

/// One academic year by identifier.
#[tauri::command]
pub fn setup_academic_year_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<AcademicYear>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.academic_year(id)?)
}

/// Every academic year, earliest first.
#[tauri::command]
pub fn setup_academic_year_list(state: State<'_, AppState>) -> SetupResult<Vec<AcademicYear>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.academic_years()?)
}

/// Replaces an academic year's details.
#[tauri::command]
pub fn setup_academic_year_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: AcademicYearInput,
) -> SetupResult<AcademicYear> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_academic_year(id, &input)?)
}

/// Deletes an academic year. Refused while it still has terms.
#[tauri::command]
pub fn setup_academic_year_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_academic_year(id)?)
}

// ------------------------------------------------------------------------------ term

/// Creates a term within an academic year.
#[tauri::command]
pub fn setup_term_create(state: State<'_, AppState>, input: TermInput) -> SetupResult<Term> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_term(&input)?)
}

/// One term by identifier.
#[tauri::command]
pub fn setup_term_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Term>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.term(id)?)
}

/// Every term of one academic year, in teaching order.
#[tauri::command]
pub fn setup_term_list(
    state: State<'_, AppState>,
    academic_year_id: Uuid,
) -> SetupResult<Vec<Term>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.terms(academic_year_id)?)
}

/// Replaces a term's details.
#[tauri::command]
pub fn setup_term_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: TermInput,
) -> SetupResult<Term> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_term(id, &input)?)
}

/// Deletes a term. Refused while a timeslot or calendar day still names it.
#[tauri::command]
pub fn setup_term_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_term(id)?)
}
