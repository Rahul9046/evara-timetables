//! School Setup commands for the time model.
//!
//! Cycles, cycle days, bell schedules, periods and the calendar mapping. The timeslot grid
//! itself is [`super::grid`], because its operations are not CRUD — they are a preview and
//! two guarded applications.
//!
//! Thin for the same reason as [`super::structure`]: the rules are in `evara-db`, where
//! the CLI and the tests reach them too.
//!
//! # Nothing here knows what a week is
//!
//! A cycle day's `ordinal` is its whole scheduling identity; `label` and `weekdayHint` are
//! display. These commands pass all three through untouched and never derive one from
//! another, so a one-day cycle, a six-day rotation and a ten-day fortnight are the same
//! code path. Real dates reach the model only through `calendar_day`, below, and only as
//! an explicit mapping a human made.

// See `project.rs` for why this lint is inapplicable to a module of command handlers.
#![allow(clippy::needless_pass_by_value)]

use evara_core::time::CycleCoverage;
use evara_db::{
    CalendarDay, CalendarDayInput, Cycle, CycleDay, CycleDayInput, CycleInput, Period, PeriodInput,
    PeriodStructure, PeriodStructureInput,
};
use tauri::State;
use uuid::Uuid;

use super::error::SetupResult;
use super::project::AppState;
use super::workspace;

// ----------------------------------------------------------------------------- cycle

/// Creates a cycle.
///
/// `dayCount` is the *declared* length. The cycle is legal before it has that many days —
/// adding them one at a time has to work — and
/// [`setup_cycle_coverage`] is how the interface says so.
#[tauri::command]
pub fn setup_cycle_create(state: State<'_, AppState>, input: CycleInput) -> SetupResult<Cycle> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.create_cycle(&input)?)
}

/// One cycle by identifier.
#[tauri::command]
pub fn setup_cycle_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Cycle>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.cycle(id)?)
}

/// Every cycle, the default first.
#[tauri::command]
pub fn setup_cycle_list(state: State<'_, AppState>) -> SetupResult<Vec<Cycle>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.cycles()?)
}

/// Replaces a cycle's details.
///
/// Marking it default clears the previous default in the same transaction. Shortening it
/// below a day it already has is refused rather than silently dropping the day.
#[tauri::command]
pub fn setup_cycle_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: CycleInput,
) -> SetupResult<Cycle> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.update_cycle(id, &input)?)
}

/// Deletes a cycle. Refused while it still has days or timeslots.
#[tauri::command]
pub fn setup_cycle_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.delete_cycle(id)?)
}

/// Which of a cycle's declared positions have a day, and which are missing.
///
/// The rule lives in `evara_core::time`, not here and not in the interface: whether a
/// cycle is finished is a question about the scheduling model, and a screen that decided
/// it for itself would be the second implementation of it.
#[tauri::command]
pub fn setup_cycle_coverage(
    state: State<'_, AppState>,
    cycle_id: Uuid,
) -> SetupResult<CycleCoverage> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.cycle_coverage(cycle_id)?)
}

// ------------------------------------------------------------------------- cycle day

/// Adds a day to a cycle.
///
/// `ordinal` is the day's identity and must be within the cycle's declared length.
/// `label` is free text — "Monday", "Day A", "Week 2 Thu" — and nothing reads it.
#[tauri::command]
pub fn setup_cycle_day_create(
    state: State<'_, AppState>,
    input: CycleDayInput,
) -> SetupResult<CycleDay> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.create_cycle_day(&input)?)
}

/// One cycle day by identifier.
#[tauri::command]
pub fn setup_cycle_day_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<CycleDay>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.cycle_day(id)?)
}

/// Every day of one cycle, in `ordinal` order.
#[tauri::command]
pub fn setup_cycle_day_list(
    state: State<'_, AppState>,
    cycle_id: Uuid,
) -> SetupResult<Vec<CycleDay>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.cycle_days(cycle_id)?)
}

/// Replaces a cycle day's details.
#[tauri::command]
pub fn setup_cycle_day_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: CycleDayInput,
) -> SetupResult<CycleDay> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.update_cycle_day(id, &input)?)
}

/// Deletes a cycle day.
///
/// Refused while a timeslot or a mapped calendar date still names it. That refusal is the
/// point: see [`super::grid::setup_grid_preview`] for how to find out what is in the way
/// *before* trying, by previewing the grid without this day.
#[tauri::command]
pub fn setup_cycle_day_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.delete_cycle_day(id)?)
}

// -------------------------------------------------------------------- period structure

/// Creates a bell schedule.
///
/// `campusId` is `null` for a school-wide schedule. A school with two campuses on
/// different bells has two structures, and therefore two grids over the same cycle.
#[tauri::command]
pub fn setup_period_structure_create(
    state: State<'_, AppState>,
    input: PeriodStructureInput,
) -> SetupResult<PeriodStructure> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.create_period_structure(&input)?)
}

/// One bell schedule by identifier.
#[tauri::command]
pub fn setup_period_structure_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<PeriodStructure>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.period_structure(id)?)
}

/// Every bell schedule.
#[tauri::command]
pub fn setup_period_structure_list(
    state: State<'_, AppState>,
) -> SetupResult<Vec<PeriodStructure>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.period_structures()?)
}

/// Replaces a bell schedule's details.
///
/// At most one default per campus, and the school-wide bucket counts as its own campus for
/// that rule. Changing the default clears the previous one in the same transaction.
#[tauri::command]
pub fn setup_period_structure_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: PeriodStructureInput,
) -> SetupResult<PeriodStructure> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.update_period_structure(id, &input)?)
}

/// Deletes a bell schedule. Refused while it still has periods.
#[tauri::command]
pub fn setup_period_structure_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.delete_period_structure(id)?)
}

// ---------------------------------------------------------------------------- period

/// Adds a period to a bell schedule.
///
/// `ordinal` is the period's identity within the day; `startsAt` and `endsAt` are
/// zero-padded `HH:MM` wall clock and the end must be after the start. `kind` decides
/// whether lessons can be scheduled into the slots built from it.
#[tauri::command]
pub fn setup_period_create(state: State<'_, AppState>, input: PeriodInput) -> SetupResult<Period> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.create_period(&input)?)
}

/// One period by identifier.
#[tauri::command]
pub fn setup_period_get(state: State<'_, AppState>, id: Uuid) -> SetupResult<Option<Period>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.period(id)?)
}

/// Every period of one bell schedule, in `ordinal` order.
#[tauri::command]
pub fn setup_period_list(
    state: State<'_, AppState>,
    period_structure_id: Uuid,
) -> SetupResult<Vec<Period>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.periods(period_structure_id)?)
}

/// Replaces a period's details.
///
/// Changing `kind` or the times does not change any slot's identity: the grid keeps every
/// identifier and only its derived columns are refreshed on the next rebuild.
#[tauri::command]
pub fn setup_period_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: PeriodInput,
) -> SetupResult<Period> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.update_period(id, &input)?)
}

/// Deletes a period. Refused while a timeslot was materialised from it.
#[tauri::command]
pub fn setup_period_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.delete_period(id)?)
}

// ----------------------------------------------------------------------- calendar day

/// Records what one real date is.
///
/// The only bridge between dates and the scheduling model, and an explicit one. Evara does
/// not infer weekends, holidays or what day follows a closure: that is policy the data
/// model does not state, and a guess would be indistinguishable from a decision. A date
/// with `cycleDayId` of `null` is a day with no lessons.
#[tauri::command]
pub fn setup_calendar_day_create(
    state: State<'_, AppState>,
    input: CalendarDayInput,
) -> SetupResult<CalendarDay> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.create_calendar_day(&input)?)
}

/// One calendar day by identifier.
#[tauri::command]
pub fn setup_calendar_day_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> SetupResult<Option<CalendarDay>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.calendar_day(id)?)
}

/// Calendar days in an inclusive `YYYY-MM-DD` range, in date order.
#[tauri::command]
pub fn setup_calendar_day_list(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> SetupResult<Vec<CalendarDay>> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.calendar_days(&from, &to)?)
}

/// Replaces a calendar day's details.
#[tauri::command]
pub fn setup_calendar_day_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: CalendarDayInput,
) -> SetupResult<CalendarDay> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.update_calendar_day(id, &input)?)
}

/// Deletes a calendar day. The dates around it are untouched.
#[tauri::command]
pub fn setup_calendar_day_delete(state: State<'_, AppState>, id: Uuid) -> SetupResult<()> {
    let mut ws = workspace(&state)?;
    Ok(ws.time_model()?.delete_calendar_day(id)?)
}
