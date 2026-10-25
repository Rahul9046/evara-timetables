//! Tauri command handlers.
//!
//! One module per surface, each kept thin. Planned, with the roadmap phase that adds it:
//!
//! | Module | Phase | Surface |
//! |---|---|---|
//! | `project` | 1 | create, open, close, recent projects |
//! | `crud` | 2–3 | entity read/write for every domain group |
//! | `constraints` | 4 | constraint instances and parameter schemas |
//! | `validate` | 5 | score a timetable, validate a move, explain a violation |
//! | `solve` | 6 | start, monitor and cancel a solve |
//! | `report` | 7, 10 | timetable projections and rendered output |
//! | `io` | 9 | `*.evara` export and import |
//!
//! Stage D adds `import`, which is the spreadsheet roster import rather than the `io` row
//! above: that one is `.evara`, Evara's own portable format, and is a different feature.
//!
//! Phase 8 splits the `validate` row above in two: `edit` is the manual-editing surface — place,
//! move, unschedule, lock, undo — and `edit_session` is the state it needs between commands.
//!
//! Phase 1B added the project lifecycle. Phase 1E adds School Setup: `structure`,
//! `time_model` and `grid`, which together are the `crud` row above for the Structure and
//! Time Model groups.

pub mod availability;
pub mod curriculum;
pub mod edit;
pub mod edit_session;
pub mod error;
pub mod grid;
pub mod import;
pub mod lessons;
pub mod membership;
pub mod people;
pub mod project;
pub mod repair;
pub mod repair_session;
pub mod session;
pub mod solve;
pub mod structure;
pub mod time_model;
pub mod views;

use std::sync::MutexGuard;

use evara_db::Workspace;
use serde::Serialize;
use tauri::State;

use crate::commands::error::{SetupError, SetupResult};
use crate::commands::project::AppState;

/// Locks the workspace for a setup command.
///
/// Shared by every School Setup handler so the poisoned-mutex case is handled once. A
/// poisoned lock means a previous command panicked mid-write; the transaction was rolled
/// back by SQLite, but the process is no longer trustworthy, so the user is told to
/// restart rather than allowed to continue against unknown state.
fn workspace<'a>(state: &'a State<'_, AppState>) -> SetupResult<MutexGuard<'a, Workspace>> {
    state.workspace.lock().map_err(|_| SetupError::Failed {
        message: "Evara is in an inconsistent state. Please restart the application.".to_owned(),
    })
}

/// Build and version facts about the running application.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppInfo {
    /// Application version, from the crate manifest.
    pub version: &'static str,
    /// Whether this is a debug build.
    pub debug: bool,
    /// Roadmap phase the codebase has reached.
    pub phase: &'static str,
}

/// Returns build and version facts.
///
/// Exists to prove the frontend/backend IPC boundary works before any feature depends
/// on it. Safe to keep: an About window will want it.
#[tauri::command]
#[allow(clippy::unnecessary_wraps)] // Signature stays Result for forward compatibility.
pub fn app_info() -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        debug: cfg!(debug_assertions),
        phase: "8B — automated timetable repair and partial regeneration",
    })
}

#[cfg(test)]
mod tests {
    use super::app_info;

    #[test]
    fn app_info_reports_the_crate_version() {
        let info = app_info().expect("app_info is infallible");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.debug, cfg!(debug_assertions));
    }

    #[test]
    fn app_info_serialises_to_camel_case() {
        let json = serde_json::to_string(&app_info().unwrap()).unwrap();
        assert!(json.contains("\"version\""), "got {json}");
        assert!(json.contains("\"debug\""), "got {json}");
    }
}
