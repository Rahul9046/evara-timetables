//! Timetable grid commands: preview, rebuild, and the separate orphan release.
//!
//! Not CRUD, which is why these are not in [`super::time_model`]. A timeslot is never
//! created or edited directly — it exists because materialisation produced it, and its
//! identifier is the thing a future timetable entry will point at. The operations are
//! therefore a read-only preview and two applications of it, each guarded.
//!
//! # The contract these three commands implement
//!
//! 1. [`setup_grid_preview`] reads the grid and returns what a rebuild *would* do, plus a
//!    [`PlanFingerprint`] of that answer. It writes nothing.
//! 2. [`setup_grid_rebuild`] takes the fingerprint back. The repository recomputes the
//!    plan inside its write transaction and refuses — `reviewRequired`, nothing written —
//!    if it no longer matches. It creates missing slots and refreshes surviving ones.
//!    **It never deletes.**
//! 3. [`setup_grid_release_orphans`] is a different command, taking the same fingerprint,
//!    and is the only one that destroys anything. It deletes exactly the orphans the
//!    recomputed plan names.
//!
//! Three properties follow, and all three were requirements rather than conveniences:
//!
//! - **Rebuilding cannot lose a placement.** Creation and destruction are different
//!   commands, not a flag on one command, so destruction cannot be reached by accident.
//! - **A confirmation belongs to the figures that were shown.** The staleness check happens
//!   inside the same transaction as the write, so there is no window between checking and
//!   acting. Doing that comparison here, in the handler, would be a check-then-act race.
//! - **An orphan release cannot delete a slot that has come back into use.** The
//!   identifiers come from the plan recomputed at deletion time, not from a list captured
//!   when the screen was drawn.
//!
//! A slot a future `timetable_entry` references cannot be deleted at all: the foreign key
//! is `RESTRICT`, so the release fails with `stillReferenced` and the whole call rolls
//! back. See [ADR
//! 0011](../../../../docs/adr/0011-stable-timeslot-identity.md).

// See `project.rs` for why this lint is inapplicable to a module of command handlers.
#![allow(clippy::needless_pass_by_value)]

use evara_db::{
    GridPreview, MaterialisationPlan, MaterialisationRequest, PlanFingerprint, Timeslot,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

/// What a rebuild would do to one grid, and the state the grid is in now.
///
/// `request` names the grid — cycle, bell schedule, optional term — and may carry
/// exclusions. The exclusions are how the interface answers "what would I lose if I
/// removed Day F?" *before* removing anything, which it has to be able to do because the
/// grid blocks that deletion until the slots are gone.
///
/// Writes nothing, so it is safe to call on every render.
#[tauri::command]
pub fn setup_grid_preview(
    state: State<'_, AppState>,
    request: MaterialisationRequest,
) -> SetupResult<GridPreview> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.grid_preview(&request)?)
}

/// Creates the slots the grid is missing and refreshes the ones it already has.
///
/// `reviewed` is the fingerprint from the [`setup_grid_preview`] the user was looking at.
/// If the grid has moved since — a period added in another screen, a day renamed — the
/// call fails with `reviewRequired` and **nothing is written**; the interface re-previews
/// and asks again rather than applying a materially different result.
///
/// Every surviving slot keeps its identifier. Orphans are reported in the returned plan
/// and left exactly as they were.
///
/// Idempotent: a rebuild of an already-correct grid writes nothing and consumes no
/// revisions.
#[tauri::command]
pub fn setup_grid_rebuild(
    state: State<'_, AppState>,
    request: MaterialisationRequest,
    reviewed: PlanFingerprint,
) -> SetupResult<MaterialisationPlan> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .time_model()?
        .apply_materialisation_reviewed(&request, &reviewed)?)
}

/// Deletes the slots this proposal strands. **The only destructive command here.**
///
/// Deliberately separate from [`setup_grid_rebuild`] so that cleaning up after a removed
/// day or period is always a second, explicit decision. The interface must show the
/// orphan list and take a confirmation before calling it.
///
/// Returns the identifiers removed. An empty list means the proposal stranded nothing,
/// which is a success, not a failure.
///
/// # Errors
///
/// - `reviewRequired` if the grid moved since the preview. Nothing is deleted.
/// - `stillReferenced` if anything refers to one of the slots. Nothing is deleted — the
///   call rolls back whole, so no subset disappears.
#[tauri::command]
pub fn setup_grid_release_orphans(
    state: State<'_, AppState>,
    request: MaterialisationRequest,
    reviewed: PlanFingerprint,
) -> SetupResult<Vec<Uuid>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.release_orphans(&request, &reviewed)?)
}

/// Every stored slot of one grid, in cycle order.
///
/// The preview is what the screen renders; this is the raw rows, for the detail panel and
/// for confirming that identifiers survived a rebuild.
#[tauri::command]
pub fn setup_grid_timeslots(
    state: State<'_, AppState>,
    cycle_id: Uuid,
    term_id: Option<Uuid>,
) -> SetupResult<Vec<Timeslot>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.timeslots(cycle_id, term_id)?)
}
