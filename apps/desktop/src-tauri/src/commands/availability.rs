//! School Setup commands for availability.
//!
//! Same shape as the rest: lock the workspace, call one repository method, convert the
//! error. Nothing here decides anything.
//!
//! # Why there is no `availability_create` and `availability_update`
//!
//! Because the grid does not have those two actions. A cell is set to a status or cleared,
//! and whether that turns out to be an insert or an edit is the repository's business —
//! it keys on the scope tuple the unique index already enforces. Exposing create and
//! update separately would push "does a rule already exist here?" into the interface,
//! which would then have to ask before every click.
//!
//! `availability_clear` reports whether anything went rather than failing when nothing
//! did. The grid sends the same request whichever way a cell was toggled.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by
// value, and every other parameter is deserialised from the IPC payload rather than
// borrowed. See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{AvailabilityEntity, AvailabilityRule, AvailabilityRuleInput};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

/// Writes a rule, replacing whatever already covers the same scope.
#[tauri::command]
pub fn setup_availability_set(
    state: State<'_, AppState>,
    input: AvailabilityRuleInput,
) -> SetupResult<AvailabilityRule> {
    let mut ws = workspace(&state)?;
    Ok(ws.availability()?.set(&input)?)
}

/// Removes the rule covering one scope. `false` means there was none, which is not an error.
#[tauri::command]
pub fn setup_availability_clear(
    state: State<'_, AppState>,
    scope: AvailabilityRuleInput,
) -> SetupResult<bool> {
    let mut ws = workspace(&state)?;
    Ok(ws.availability()?.clear(&scope)?)
}

/// Every rule for one entity.
#[tauri::command]
pub fn setup_availability_list(
    state: State<'_, AppState>,
    school_id: Uuid,
    entity_kind: AvailabilityEntity,
    entity_id: Uuid,
) -> SetupResult<Vec<AvailabilityRule>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .availability()?
        .rules_for(school_id, entity_kind, entity_id)?)
}

/// How many rules one entity has, for a listing that marks who has been configured.
#[tauri::command]
pub fn setup_availability_count(
    state: State<'_, AppState>,
    school_id: Uuid,
    entity_kind: AvailabilityEntity,
    entity_id: Uuid,
) -> SetupResult<i64> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .availability()?
        .count_for(school_id, entity_kind, entity_id)?)
}

/// Rules whose entity no longer exists.
///
/// The integrity sweep for the one reference the database cannot enforce. Exposed so a
/// diagnostic screen or the CLI can run it; nothing in normal operation should ever see a
/// non-empty result.
#[tauri::command]
pub fn setup_availability_orphans(
    state: State<'_, AppState>,
) -> SetupResult<Vec<AvailabilityRule>> {
    let mut ws = workspace(&state)?;
    Ok(ws.availability()?.orphans()?)
}
