//! The Evara Timetables desktop application.
//!
//! # Layering contract
//!
//! This crate **orchestrates**; it does not decide. Command handlers translate an IPC
//! request into a call on one of the `evara-*` crates and translate the result back.
//! Business logic in a command handler is an architecture bug — it would be unreachable
//! from the headless CLI and from tests.
//!
//! Specifically, this crate must not:
//!
//! - contain a scheduling rule (those live in `evara-constraints`)
//! - contain SQL (that lives in `evara-db`)
//! - contain a solver heuristic (that lives in `evara-solver`)

pub mod commands;

use std::sync::Mutex;

use commands::project::AppState;

/// Builds and runs the application.
///
/// # Panics
///
/// Panics if the Tauri context cannot be initialised, which indicates a broken build
/// rather than a runtime condition worth recovering from.
pub fn run() {
    // The workspace owns the installation settings and the one open project. Built once,
    // here, because a failure to reach the settings database means Evara cannot function
    // and the user needs to know immediately rather than on first use.
    let workspace = evara_db::Workspace::new(env!("CARGO_PKG_VERSION"))
        .expect("failed to open the Evara settings database");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            workspace: Mutex::new(workspace),
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            // Project lifecycle — Phase 1B.
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_close,
            commands::project::project_current,
            commands::project::project_recent,
            commands::project::project_forget_recent,
            commands::project::project_extension,
            commands::project::project_default_folder,
            // School Setup, static structure — Phase 1E.
            commands::structure::setup_school_create,
            commands::structure::setup_school_get,
            commands::structure::setup_school_update,
            commands::structure::setup_campus_create,
            commands::structure::setup_campus_get,
            commands::structure::setup_campus_list,
            commands::structure::setup_campus_update,
            commands::structure::setup_campus_delete,
            commands::structure::setup_building_create,
            commands::structure::setup_building_get,
            commands::structure::setup_building_list,
            commands::structure::setup_building_update,
            commands::structure::setup_building_delete,
            commands::structure::setup_room_type_create,
            commands::structure::setup_room_type_get,
            commands::structure::setup_room_type_list,
            commands::structure::setup_room_type_update,
            commands::structure::setup_room_type_delete,
            commands::structure::setup_room_create,
            commands::structure::setup_room_get,
            commands::structure::setup_room_list,
            commands::structure::setup_room_update,
            commands::structure::setup_room_delete,
            commands::structure::setup_resource_create,
            commands::structure::setup_resource_get,
            commands::structure::setup_resource_list,
            commands::structure::setup_resource_update,
            commands::structure::setup_resource_delete,
            commands::structure::setup_academic_year_create,
            commands::structure::setup_academic_year_get,
            commands::structure::setup_academic_year_list,
            commands::structure::setup_academic_year_update,
            commands::structure::setup_academic_year_delete,
            commands::structure::setup_term_create,
            commands::structure::setup_term_get,
            commands::structure::setup_term_list,
            commands::structure::setup_term_update,
            commands::structure::setup_term_delete,
            // School Setup, time model — Phase 1E.
            commands::time_model::setup_cycle_create,
            commands::time_model::setup_cycle_get,
            commands::time_model::setup_cycle_list,
            commands::time_model::setup_cycle_update,
            commands::time_model::setup_cycle_delete,
            commands::time_model::setup_cycle_coverage,
            commands::time_model::setup_cycle_day_create,
            commands::time_model::setup_cycle_day_get,
            commands::time_model::setup_cycle_day_list,
            commands::time_model::setup_cycle_day_update,
            commands::time_model::setup_cycle_day_delete,
            commands::time_model::setup_period_structure_create,
            commands::time_model::setup_period_structure_get,
            commands::time_model::setup_period_structure_list,
            commands::time_model::setup_period_structure_update,
            commands::time_model::setup_period_structure_delete,
            commands::time_model::setup_period_create,
            commands::time_model::setup_period_get,
            commands::time_model::setup_period_list,
            commands::time_model::setup_period_update,
            commands::time_model::setup_period_delete,
            commands::time_model::setup_calendar_day_create,
            commands::time_model::setup_calendar_day_get,
            commands::time_model::setup_calendar_day_list,
            commands::time_model::setup_calendar_day_update,
            commands::time_model::setup_calendar_day_delete,
            // Timetable grid — Phase 1E. `release_orphans` is the only destructive one.
            commands::grid::setup_grid_preview,
            commands::grid::setup_grid_rebuild,
            commands::grid::setup_grid_release_orphans,
            commands::grid::setup_grid_timeslots,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Evara Timetables window");
}
