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
//! Phase 1B adds the project lifecycle; the rest arrive with their phases.

pub mod project;

use serde::Serialize;

/// Build and version facts about the running application.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
        phase: "1B — project lifecycle",
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
