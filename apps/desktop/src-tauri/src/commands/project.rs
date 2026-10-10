//! Project lifecycle commands.
//!
//! Thin by design: each one locks the [`Workspace`], calls a single method and translates
//! the result. The lifecycle itself lives in `evara-db`, so it stays testable without a
//! window and the rules are not duplicated here.
//!
//! # What crosses the boundary
//!
//! Failures become [`ProjectError`], a tagged union the interface can branch on, rather
//! than a formatted string it would have to parse. Raw SQLite errors and full filesystem
//! paths are deliberately **not** forwarded: the interface gets the file name and a
//! message fit to show someone, and the detail stays in the backend.

// Tauri's command macro dictates these signatures: `State` is a guard type that must be
// taken by value, and every other parameter is deserialised from the IPC payload, so it
// cannot be borrowed. The lint is right in general and inapplicable to this module, which
// contains nothing but command handlers.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use evara_db::{DbError, ProjectSummary, RecentProject, Workspace};
use serde::Serialize;
use tauri::State;

/// The application's single open project, shared across commands.
pub struct AppState {
    /// `Mutex` because commands may run concurrently and a SQLite connection is not
    /// shareable between threads.
    pub workspace: Mutex<Workspace>,
}

/// A lifecycle failure, in a form the interface can act on.
///
/// Tagged so the frontend matches on `kind` instead of reading English. Every variant
/// carries `message` so there is one source of wording, and it lives next to the rule
/// that produced it.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProjectError {
    /// The project was written by a newer Evara.
    SchemaTooNew {
        /// Schema version found in the file.
        found: u32,
        /// Highest version this build understands.
        supported: u32,
        /// Message to show.
        message: String,
    },
    /// No file at that path.
    NotFound {
        /// Message to show.
        message: String,
    },
    /// The file exists but is not an Evara project.
    NotAnEvaraProject {
        /// Message to show.
        message: String,
    },
    /// The path does not end in `.evaraproj`.
    WrongExtension {
        /// The extension Evara requires.
        expected: String,
        /// Message to show.
        message: String,
    },
    /// A project already exists where a new one was requested.
    AlreadyExists {
        /// Message to show.
        message: String,
    },
    /// An operation needing an open project ran without one.
    NoProjectOpen {
        /// Message to show.
        message: String,
    },
    /// Anything else: migration failure, a broken file, a disk problem.
    Failed {
        /// Message to show.
        message: String,
    },
}

impl From<DbError> for ProjectError {
    fn from(error: DbError) -> Self {
        let message = error.to_string();
        match error {
            DbError::SchemaTooNew { found, supported } => Self::SchemaTooNew {
                found,
                supported,
                message,
            },
            DbError::ProjectNotFound { .. } => Self::NotFound { message },
            DbError::NotAnEvaraProject { .. } => Self::NotAnEvaraProject { message },
            DbError::WrongExtension { expected } => Self::WrongExtension {
                expected: expected.to_owned(),
                message,
            },
            DbError::ProjectAlreadyExists { .. } => Self::AlreadyExists { message },
            DbError::NoProjectOpen => Self::NoProjectOpen { message },
            // Everything else is an internal failure. These carry engine detail in their
            // source chain, so their own wording is replaced rather than forwarded.
            DbError::Open { .. } | DbError::Internal(_) | DbError::Migration(_) => Self::Failed {
                message: "The project could not be opened. The file may be damaged or in use \
                          by another program."
                    .to_owned(),
            },
            _ => Self::Failed { message },
        }
    }
}

/// A project as the interface sees it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// Stable project identifier.
    pub project_id: String,
    /// Name derived from the file name.
    pub display_name: String,
    /// Full path. The user chose it, so showing it back is expected.
    pub path: String,
    /// Folder the project sits in, for display.
    pub folder: String,
    /// Schema version applied.
    pub schema_version: u32,
    /// Whether this installation created the project.
    pub created_here: bool,
    /// Cloud provider the project appears to sit under — advisory only.
    pub synced_folder: Option<String>,
}

impl From<ProjectSummary> for ProjectView {
    fn from(summary: ProjectSummary) -> Self {
        Self {
            project_id: summary.project_id.to_string(),
            display_name: summary.display_name,
            path: summary.path.to_string_lossy().into_owned(),
            folder: summary
                .path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            schema_version: summary.schema_version,
            created_here: summary.created_here,
            synced_folder: summary.synced_folder.map(|f| f.provider().to_owned()),
        }
    }
}

/// An entry in the recent-project list.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectView {
    /// Full path, needed to reopen it.
    pub path: String,
    /// Folder, for display.
    pub folder: String,
    /// Name derived from the file name.
    pub display_name: String,
    /// When this installation last opened it.
    pub last_opened_at: String,
    /// Whether the file is still there. A missing entry stays listed so the user can
    /// decide to forget it.
    pub exists: bool,
}

impl From<RecentProject> for RecentProjectView {
    fn from(recent: RecentProject) -> Self {
        Self {
            folder: recent
                .path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: recent.path.to_string_lossy().into_owned(),
            display_name: recent.display_name,
            last_opened_at: recent.last_opened_at,
            exists: recent.exists,
        }
    }
}

type CommandResult<T> = Result<T, ProjectError>;

/// Locks the workspace, failing loudly if a previous command panicked while holding it.
fn lock<'a>(state: &'a State<'_, AppState>) -> CommandResult<std::sync::MutexGuard<'a, Workspace>> {
    state.workspace.lock().map_err(|_| ProjectError::Failed {
        message: "Evara is in an inconsistent state. Please restart the application.".to_owned(),
    })
}

/// Creates a project at `path` and opens it.
#[tauri::command]
pub fn project_create(state: State<'_, AppState>, path: PathBuf) -> CommandResult<ProjectView> {
    let mut workspace = lock(&state)?;
    Ok(workspace.create_project(&path)?.into())
}

/// Opens an existing project.
#[tauri::command]
pub fn project_open(state: State<'_, AppState>, path: PathBuf) -> CommandResult<ProjectView> {
    let mut workspace = lock(&state)?;
    Ok(workspace.open_project(&path)?.into())
}

/// Closes the open project, releasing its database connection.
///
/// Returns whether anything was open, so closing twice is not an error.
#[tauri::command]
pub fn project_close(state: State<'_, AppState>) -> CommandResult<bool> {
    let mut workspace = lock(&state)?;
    Ok(workspace.close_project())
}

/// The open project, or `null`.
#[tauri::command]
pub fn project_current(state: State<'_, AppState>) -> CommandResult<Option<ProjectView>> {
    let workspace = lock(&state)?;
    Ok(workspace.current_project().map(Into::into))
}

/// Recently opened projects, newest first.
#[tauri::command]
pub fn project_recent(state: State<'_, AppState>) -> CommandResult<Vec<RecentProjectView>> {
    let workspace = lock(&state)?;
    Ok(workspace
        .recent_projects()?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// Removes a project from the recent list. The file itself is never touched.
#[tauri::command]
pub fn project_forget_recent(state: State<'_, AppState>, path: PathBuf) -> CommandResult<bool> {
    let mut workspace = lock(&state)?;
    Ok(workspace.forget_recent_project(&path)?)
}

/// The extension a live project must use, so the interface can build file-dialog filters
/// from one source rather than hardcoding the string.
#[tauri::command]
pub const fn project_extension() -> &'static str {
    evara_db::PROJECT_EXTENSION
}

/// Suggests a default folder for a new project.
///
/// Never scans for projects — Evara does not search the filesystem.
#[tauri::command]
pub fn project_default_folder() -> Option<String> {
    directories_default().map(|p| p.to_string_lossy().into_owned())
}

fn directories_default() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| Path::new(&home).join("Documents"))
        .filter(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::{ProjectError, ProjectView};
    use evara_db::DbError;

    fn kind_of(error: DbError) -> String {
        let value = serde_json::to_value(ProjectError::from(error)).expect("serialise");
        value["kind"].as_str().expect("tagged").to_owned()
    }

    #[test]
    fn important_states_are_distinguishable_by_the_interface() {
        assert_eq!(
            kind_of(DbError::SchemaTooNew {
                found: 9,
                supported: 2
            }),
            "schemaTooNew"
        );
        assert_eq!(
            kind_of(DbError::ProjectNotFound {
                name: "school.evaraproj".to_owned()
            }),
            "notFound"
        );
        assert_eq!(
            kind_of(DbError::NotAnEvaraProject {
                name: "notes.txt".to_owned()
            }),
            "notAnEvaraProject"
        );
        assert_eq!(
            kind_of(DbError::WrongExtension {
                expected: "evaraproj"
            }),
            "wrongExtension"
        );
        assert_eq!(
            kind_of(DbError::ProjectAlreadyExists {
                name: "school.evaraproj".to_owned()
            }),
            "alreadyExists"
        );
        assert_eq!(kind_of(DbError::NoProjectOpen), "noProjectOpen");
    }

    #[test]
    fn schema_too_new_carries_both_versions() {
        let value = serde_json::to_value(ProjectError::from(DbError::SchemaTooNew {
            found: 9,
            supported: 2,
        }))
        .expect("serialise");
        assert_eq!(value["found"], 9);
        assert_eq!(value["supported"], 2);
        assert!(
            value["message"].as_str().unwrap().contains("Update Evara"),
            "the user must be told what to do"
        );
    }

    /// Constructed without `rusqlite` on purpose: this crate must not need the SQLite
    /// driver even to build an error in a test. `InternalError::from_message` is what
    /// makes that possible, and the absence of a `rusqlite` dev-dependency is what proves
    /// the boundary holds.
    #[test]
    fn internal_failures_do_not_leak_paths_or_engine_detail() {
        let error = DbError::Open {
            name: "secret school.evaraproj".to_owned(),
            source: evara_db::InternalError::from_message(
                "unable to open database file: /home/someone/Private/secret school.evaraproj",
            ),
        };
        let value = serde_json::to_value(ProjectError::from(error)).expect("serialise");
        let message = value["message"].as_str().expect("message");

        assert_eq!(value["kind"], "failed");
        assert!(
            !message.contains("secret school"),
            "leaked a file name: {message}"
        );
        assert!(!message.contains("/home/"), "leaked a path: {message}");
        assert!(
            !message.contains("database file"),
            "leaked engine detail: {message}"
        );
    }

    #[test]
    fn an_opaque_internal_error_maps_to_a_generic_failure() {
        let value = serde_json::to_value(ProjectError::from(DbError::internal(
            "near \"SELCT\": syntax",
        )))
        .expect("serialise");
        assert_eq!(value["kind"], "failed");
        assert!(
            !value["message"].as_str().unwrap().contains("SELCT"),
            "SQL must never reach the interface"
        );
    }

    #[test]
    fn a_project_view_renders_camel_case_for_the_frontend() {
        let view = ProjectView {
            project_id: "018f3a2c-0000-7000-8000-000000000001".to_owned(),
            display_name: "Northgate".to_owned(),
            path: "/tmp/Northgate.evaraproj".to_owned(),
            folder: "/tmp".to_owned(),
            schema_version: 2,
            created_here: true,
            synced_folder: Some("Dropbox".to_owned()),
        };
        let json = serde_json::to_string(&view).expect("serialise");
        assert!(json.contains("\"projectId\""), "{json}");
        assert!(json.contains("\"schemaVersion\""), "{json}");
        assert!(json.contains("\"syncedFolder\":\"Dropbox\""), "{json}");
    }
}
