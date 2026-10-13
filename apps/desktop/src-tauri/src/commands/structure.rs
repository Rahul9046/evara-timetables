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

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use evara_core::branding::{LogoFormat, MAX_BYTES, MAX_EDGE, MIN_EDGE};
use evara_db::{
    AcademicYear, AcademicYearInput, Building, BuildingInput, BuildingWithCampuses, Campus,
    CampusInput, Resource, ResourceInput, Room, RoomInput, RoomType, RoomTypeInput,
    RoomWithCampuses, School, SchoolDependents, SchoolInput, SchoolLogo, Term, TermInput,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ---------------------------------------------------------------------------- school

/// Creates a school. A project may hold several since V12.
#[tauri::command]
pub fn setup_school_create(state: State<'_, AppState>, input: SchoolInput) -> SetupResult<School> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_school(&input)?)
}

/// Every school in the project, ordered by name.
///
/// An empty list is what drives the first-run empty state: no school has been described
/// yet, so the only sensible next action is to describe one.
///
/// This replaces the old `setup_school_get`, which returned "the" school from a `LIMIT 1`
/// read. With more than one school that would hand the interface an arbitrary institution
/// and silently scope every later request to it.
#[tauri::command]
pub fn setup_school_list(state: State<'_, AppState>) -> SetupResult<Vec<School>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.schools()?)
}

/// One school by identifier, or `null` if it is not there.
#[tauri::command]
pub fn setup_school_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<School>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.school(id)?)
}

/// What a school still owns, so a confirmation can say why it cannot be deleted.
#[tauri::command]
pub fn setup_school_dependents(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<SchoolDependents> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.school_dependents(id)?)
}

/// Deletes a school that owns nothing.
///
/// Refused while it still owns anything at all. There is no "delete everything in it":
/// see `Structure::delete_school`.
#[tauri::command]
pub fn setup_school_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_school(id)?)
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

// ------------------------------------------------------------------------ school logo

/// A school logo as the interface needs it: ready to put in an `<img>`.
///
/// # The one place a DTO is not the repository record
///
/// [ADR 0012](../../../../../docs/adr/0012-records-are-the-ipc-contract.md) says repository
/// records *are* the IPC contract, and nothing here contradicts that lightly. The exception
/// is binary: `SchoolLogo.bytes` is a `Vec<u8>`, and JSON has no byte array, so serde would
/// write a 2 MiB image as a list of decimal numbers roughly six times its size. Sending the
/// record as it stands would make the largest allowed logo about 12 MB of text to parse.
///
/// So the bytes cross as base64 inside a `data:` URL, which is both compact and exactly
/// what `<img src>` wants — the frontend never decodes anything. Everything else about the
/// row travels unchanged.
///
/// A future report or export in Rust uses [`SchoolLogo`] directly and never sees this type;
/// raw bytes are what those want anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SchoolLogoView {
    /// `data:image/png;base64,...`, ready to use as an image source.
    pub data_url: String,
    /// The stored media type, for a download or an export that needs to name the format.
    pub media_type: String,
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
    /// Size of the stored image in bytes, before base64.
    pub byte_size: i64,
    /// When the logo last changed, ISO-8601 UTC.
    pub updated_at: String,
}

impl SchoolLogoView {
    /// Renders a stored logo for the interface.
    ///
    /// Returns `None` when the stored media type is one this build cannot name — a row
    /// written by a future version, or a damaged one. The caller then shows the Evara logo,
    /// which is the same thing it does for a school that never uploaded one. Handing a
    /// WebView a `data:` URL with a type it will not render would produce a broken image
    /// icon and no explanation.
    fn render(logo: &SchoolLogo) -> Option<Self> {
        let format = LogoFormat::from_media_type(&logo.media_type)?;
        let encoded = BASE64.encode(&logo.bytes);
        Some(Self {
            data_url: format!("data:{};base64,{encoded}", format.media_type()),
            media_type: logo.media_type.clone(),
            width: logo.width,
            height: logo.height,
            byte_size: i64::try_from(logo.bytes.len()).unwrap_or(i64::MAX),
            updated_at: logo.updated_at.clone(),
        })
    }
}

/// What the interface may accept, so it does not have to restate the rules.
///
/// Sent rather than hardcoded in TypeScript because `evara_core::branding` is the authority
/// on all four numbers, and a duplicated limit is a limit that drifts. The frontend uses
/// these for its file picker's `accept` list, its hint text, and the instant check it does
/// before sending anything over IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogoLimits {
    /// Largest accepted file, in bytes.
    pub max_bytes: i64,
    /// Largest accepted edge, in pixels.
    pub max_edge: i64,
    /// Smallest accepted edge, in pixels.
    pub min_edge: i64,
    /// Accepted media types, in the order the interface should list them.
    pub media_types: Vec<String>,
}

/// The school's logo, or `null` when it uses Evara's.
///
/// `null` is the ordinary case: a new project, a project upgraded from a schema older than
/// V8, and a school that removed its logo all return it.
#[tauri::command]
pub fn setup_school_logo_get(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Option<SchoolLogoView>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .school_logo(school_id)?
        .as_ref()
        .and_then(SchoolLogoView::render))
}

/// Sets or replaces the school's logo from base64-encoded image bytes.
///
/// No filename is taken and none is wanted: the format is decided from the content by
/// `evara_core::branding`, which also applies the size and dimension limits. A `.png` that
/// holds a JPEG is stored as a JPEG, and a `.png` that holds an executable is refused.
#[tauri::command]
pub fn setup_school_logo_set(
    state: State<'_, AppState>,
    school_id: Uuid,
    data_base64: String,
) -> SetupResult<SchoolLogoView> {
    // Refused before decoding. Base64 inflates by about a third, so this bounds the
    // allocation below rather than letting a caller hand over an arbitrary amount of
    // memory to expand. The real limit is applied to the decoded bytes by `inspect`.
    if data_base64.len() > MAX_BYTES * 2 {
        return Err(evara_db::DbError::Invalid {
            message: format!("that image is larger than the {MAX_BYTES}-byte limit"),
        }
        .into());
    }

    let bytes = BASE64
        .decode(data_base64.as_bytes())
        .map_err(|_| evara_db::DbError::Invalid {
            message: "that image could not be read".to_owned(),
        })?;

    let mut ws = workspace(&state)?;
    let logo = ws.structure()?.set_school_logo(school_id, &bytes)?;
    SchoolLogoView::render(&logo).ok_or_else(|| {
        evara_db::DbError::Invalid {
            message: "that image could not be read back".to_owned(),
        }
        .into()
    })
}

/// Removes the school's logo, restoring the Evara one. Removing nothing succeeds.
#[tauri::command]
pub fn setup_school_logo_clear(state: State<'_, AppState>, school_id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.clear_school_logo(school_id)?)
}

/// The limits the interface should enforce before sending, and state in its hint text.
#[tauri::command]
#[allow(clippy::unnecessary_wraps)] // Signature stays Result to match its neighbours.
pub fn setup_school_logo_limits() -> SetupResult<LogoLimits> {
    Ok(LogoLimits {
        max_bytes: i64::try_from(MAX_BYTES).unwrap_or(i64::MAX),
        max_edge: i64::from(MAX_EDGE),
        min_edge: i64::from(MIN_EDGE),
        media_types: LogoFormat::all()
            .iter()
            .map(|format| format.media_type().to_owned())
            .collect(),
    })
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

/// Every campus of one school.
#[tauri::command]
pub fn setup_campus_list(state: State<'_, AppState>, school_id: Uuid) -> SetupResult<Vec<Campus>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.campuses(school_id)?)
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
pub fn setup_campus_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_campus(school_id, id)?)
}

// -------------------------------------------------------------------------- building

/// Creates a building on a campus, optionally shared with others.
///
/// `shared_campus_ids` names the *additional* campuses. The building and every association
/// are written in one transaction, so a half-shared building is never persisted — see
/// [ADR 0013](../../../../docs/adr/0013-buildings-shared-across-campuses.md).
#[tauri::command]
pub fn setup_building_create(
    state: State<'_, AppState>,
    input: BuildingInput,
    shared_campus_ids: Vec<Uuid>,
) -> SetupResult<Building> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .create_building(&input, &shared_campus_ids)?)
}

/// One building by identifier.
#[tauri::command]
pub fn setup_building_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Building>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.building(id)?)
}

/// Every building available to one campus, shared ones included.
///
/// Each entry carries the full set of campuses that building serves, so the listing can
/// mark a shared building without a call per row.
#[tauri::command]
pub fn setup_building_list(
    state: State<'_, AppState>,
    campus_id: Uuid,
) -> SetupResult<Vec<BuildingWithCampuses>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.buildings(campus_id)?)
}

/// Every campus one building serves.
#[tauri::command]
pub fn setup_building_campuses(
    state: State<'_, AppState>,
    building_id: Uuid,
) -> SetupResult<Vec<Uuid>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.building_campuses(building_id)?)
}

/// Replaces a building's details. Its home campus never changes.
#[tauri::command]
pub fn setup_building_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: BuildingInput,
) -> SetupResult<Building> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_building(id, &input)?)
}

/// Shares an existing building with another campus.
///
/// The same campus cannot be associated twice; a repeat is refused rather than ignored, so
/// a screen working from a stale share set finds out instead of believing a success.
#[tauri::command]
pub fn setup_building_share(
    state: State<'_, AppState>,
    building_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.share_building(building_id, campus_id)?)
}

/// Stops sharing a building with a campus.
///
/// Refused while rooms on that campus are still in the building, and refused outright for
/// the campus the building was created on. Nothing is detached or deleted either way — a
/// withdrawn share is reversible, so it must not take rooms with it.
#[tauri::command]
pub fn setup_building_unshare(
    state: State<'_, AppState>,
    school_id: Uuid,
    building_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .unshare_building(school_id, building_id, campus_id)?)
}

/// Deletes a building, detaching its rooms rather than deleting them.
///
/// A room's `building_id` is an optional classification, so ADR 0010 makes it `SET NULL`
/// — through a real repository update, so each affected room's revision stays truthful.
/// The rooms themselves survive; they simply stop naming a building. Its campus
/// associations are removed the same way, one change-log entry each.
#[tauri::command]
pub fn setup_building_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_building(school_id, id)?)
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

/// Every room type of one school.
#[tauri::command]
pub fn setup_room_type_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<RoomType>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room_types(school_id)?)
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
pub fn setup_room_type_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_room_type(school_id, id)?)
}

// ------------------------------------------------------------------------------ room

/// Creates a room on a campus, optionally shared with others.
///
/// `shared_campus_ids` names the *additional* campuses. The room and every association are
/// written in one transaction, so a half-shared room is never persisted — see
/// [ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md).
#[tauri::command]
pub fn setup_room_create(
    state: State<'_, AppState>,
    input: RoomInput,
    shared_campus_ids: Vec<Uuid>,
) -> SetupResult<Room> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.create_room(&input, &shared_campus_ids)?)
}

/// One room by identifier.
#[tauri::command]
pub fn setup_room_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Room>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room(id)?)
}

/// Every room available to one campus, shared ones included.
///
/// Each entry carries the full set of campuses that room serves, so the listing can mark a
/// shared room without a call per row.
#[tauri::command]
pub fn setup_room_list(
    state: State<'_, AppState>,
    campus_id: Uuid,
) -> SetupResult<Vec<RoomWithCampuses>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.rooms(campus_id)?)
}

/// Every campus one room serves.
#[tauri::command]
pub fn setup_room_campuses(state: State<'_, AppState>, room_id: Uuid) -> SetupResult<Vec<Uuid>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.room_campuses(room_id)?)
}

/// Replaces a room's details. Its home campus never changes.
///
/// `buildingId` and `roomTypeId` stay nullable here: clearing either is a normal edit, not
/// a deletion, and the repository accepts `null` for both. A change of building is refused
/// when the new building is not available to a campus the room serves.
#[tauri::command]
pub fn setup_room_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: RoomInput,
) -> SetupResult<Room> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.update_room(id, &input)?)
}

/// Shares an existing room with another campus.
///
/// Refused when the room's building is not available to that campus, and refused for a
/// campus that already has it — a repeat is reported rather than ignored, so a screen
/// working from a stale share set finds out instead of believing a success.
#[tauri::command]
pub fn setup_room_share(
    state: State<'_, AppState>,
    room_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.share_room(room_id, campus_id)?)
}

/// Stops sharing a room with a campus.
///
/// Refused outright for the campus the room was created on. Nothing is moved, duplicated
/// or deleted: the room keeps its identity and every other campus keeps it.
#[tauri::command]
pub fn setup_room_unshare(
    state: State<'_, AppState>,
    school_id: Uuid,
    room_id: Uuid,
    campus_id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .structure()?
        .unshare_room(school_id, room_id, campus_id)?)
}

/// Deletes a room. Its campus associations go with it, one change-log entry each.
#[tauri::command]
pub fn setup_room_delete(state: State<'_, AppState>, school_id: Uuid, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_room(school_id, id)?)
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

/// Every resource of one school.
#[tauri::command]
pub fn setup_resource_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<Resource>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.resources(school_id)?)
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
pub fn setup_resource_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_resource(school_id, id)?)
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

/// Every academic year of one school, earliest first.
#[tauri::command]
pub fn setup_academic_year_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<AcademicYear>> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.academic_years(school_id)?)
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
pub fn setup_academic_year_delete(
    state: State<'_, AppState>,
    school_id: Uuid,
    id: Uuid,
) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_academic_year(school_id, id)?)
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
pub fn setup_term_delete(state: State<'_, AppState>, school_id: Uuid, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.structure()?.delete_term(school_id, id)?)
}
