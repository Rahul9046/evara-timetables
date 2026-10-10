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
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_close,
            commands::project::project_current,
            commands::project::project_recent,
            commands::project::project_forget_recent,
            commands::project::project_extension,
            commands::project::project_default_folder,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Evara Timetables window");
}
