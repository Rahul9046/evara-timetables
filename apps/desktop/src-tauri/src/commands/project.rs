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
    /// The one import in progress, if any.
    ///
    /// Its own lock rather than a field inside the workspace: an import session holds a
    /// parsed file and no connection, it outlives individual commands, and a reader of
    /// the mapper must not be blocked behind a write somewhere else in the application.
    /// Cleared when the project closes — see [`project_close`].
    pub imports: crate::commands::import::ImportSessions,
    /// Every generation this process has started.
    ///
    /// Its own lock, like `imports` and for the same reasons: a session holds a compiled
    /// problem and **no** connection, it outlives the command that started it, and a poll of
    /// its progress must not block behind a write somewhere else in the application.
    ///
    /// Cleared when the project closes — see [`project_close`].
    pub generations: crate::commands::session::GenerationSessions,
    /// Every timetable open for editing.
    ///
    /// Its own lock, like `generations` and for the same reasons: an edit session holds a
    /// compiled problem and **no** connection, it outlives the command that opened it, and
    /// reading its history must not block behind a write somewhere else in the application.
    ///
    /// Cleared when the project closes — see [`project_close`]. An undo stack whose rows are in
    /// a file that is no longer open is worse than no undo stack.
    pub edits: crate::commands::edit_session::EditSessions,
    /// Every repair this process has started.
    ///
    /// Its own lock, for the reasons `generations` and `edits` have one: a repair session holds
    /// a compiled problem, a baseline arrangement and **no** connection, it outlives the command
    /// that started it, and polling its progress must not block behind a write.
    ///
    /// Cleared when the project closes — see [`project_close`]. A baseline and an arrangement
    /// stamp describing a file that is no longer open are worse than nothing, because the stamp
    /// would appear to guard something.
    pub repairs: crate::commands::repair_session::RepairSessions,
    /// Every project transfer this process has started — M10 Stage 4, D9.
    ///
    /// Its own lock, for the reasons `generations`, `edits` and `repairs` have one: a
    /// transfer session holds a path, a report and **no** connection, it outlives the command
    /// that started it, and polling its progress must not block behind a write.
    ///
    /// Cleared when the project is created, opened or closed — see [`project_close`]. An
    /// export's snapshot describes a campus the user has moved away from, and a session that
    /// outlived its epoch could be neither read nor cancelled honestly.
    pub transfers: crate::commands::transfer_session::TransferSessions,
    /// What startup recovery did to interrupted replacements — D11.
    ///
    /// Computed once, before any project is open, and never again: the project that needs
    /// recovering may be the one that will not open, so it cannot wait for a project to be
    /// chosen. Read back by `project_recovery_report`.
    pub recovery: Vec<crate::commands::transfer::RecoveredView>,
    /// Which project is open, as a monotonic counter.
    ///
    /// Bumped on every create, open and close. A generation session records the value it
    /// started under and every later command compares, so a solve begun against one campus
    /// file can never be saved into another — see `commands::solve`.
    pub epoch: crate::commands::session::ProjectEpoch,
}

/// A lifecycle failure, in a form the interface can act on.
///
/// Tagged so the frontend matches on `kind` instead of reading English. Every variant
/// carries `message` so there is one source of wording, and it lives next to the rule
/// that produced it.
#[derive(Debug, Clone, Serialize)]
// `rename_all_fields` as well as `rename_all`, for the reason `SetupError` carries it: a
// variant's field names cross as JSON keys, and `CLAUDE.md` §3 requires camelCase on the
// wire. Without it `DestinationIsOpen`'s `file_name` would arrive in snake_case — the one
// shape a generated type cannot warn about, because it would be generated wrong too.
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    // ------------------------------------------------- M10 project transfer (Stage 4)
    //
    // Transfer *is* project lifecycle, so these extend this enum rather than opening a third
    // one. Each has its own kind because each has its own next step: "ask for a fresh export"
    // and "this is not an Evara archive" are different sentences with different remedies, and
    // an interface that could not tell them apart would offer the wrong one.
    /// The file is not a readable `.evara` archive at all.
    ///
    /// Not an archive, no manifest, a manifest that will not parse, or a `format` this build
    /// has never heard of. The remedy is *check you were sent the right file* — not *ask for
    /// another copy*, which is [`ProjectError::ArchiveDamaged`]'s.
    ArchiveUnreadable {
        /// Message to show.
        message: String,
    },

    /// The archive is an Evara archive and its contents do not match its own digests.
    ///
    /// Truncation, bit rot, a bad copy, a partially written archive, a substituted payload —
    /// or a payload that is damaged, non-conforming or referentially broken once opened. The
    /// remedy is always the same and is always available: ask for a fresh export.
    ///
    /// **This is corruption, not authenticity.** Anyone can rewrite a manifest and recompute
    /// every digest below it, so a clean verification says the bytes arrived intact and says
    /// nothing at all about who produced them — §6.2, and the import screen says so too.
    ArchiveDamaged {
        /// Message to show.
        message: String,
    },

    /// The archive's **container** is newer than this build reads.
    ///
    /// The first of D2's two version axes, and distinct from [`ProjectError::SchemaTooNew`],
    /// which is the second. One refusal each, and they are not interchangeable: a container
    /// this build cannot open is a different problem from data it cannot interpret, and
    /// collapsing them would tell the user to update for the wrong reason.
    FormatTooNew {
        /// Container version found in the archive.
        found: u32,
        /// Highest container version this build reads.
        supported: u32,
        /// Message to show.
        message: String,
    },

    /// The archive needs something this build does not implement.
    ///
    /// D3: `capabilities` is derived from the data, so this names a rule the reader could not
    /// evaluate. A refusal rather than a row quietly dropped — importing the rest would
    /// produce a campus that looks complete and is not.
    UnsupportedCapability {
        /// The first capability not recognised.
        capability: String,
        /// Message to show.
        message: String,
    },

    /// The destination is the campus this window currently has open.
    ///
    /// D13 case 3. Replacing a file SQLite still holds open is how a destination ends up
    /// paired with a live `-wal` belonging to a database that no longer exists — which opens
    /// cleanly and is wrong. Refused with the remedy named rather than attempted.
    DestinationIsOpen {
        /// The file, for the message and for a screen to highlight.
        file_name: String,
        /// Message to show.
        message: String,
    },

    /// Evara will not write to that path.
    ///
    /// The destination itself is a link, or it is a URI rather than a path. A link in an
    /// *ancestor* is fine and is deliberately allowed — that is how macOS reaches its own
    /// temporary directory.
    DestinationUnsafe {
        /// The file, for the message.
        file_name: String,
        /// Message to show.
        message: String,
    },

    /// Something is already using the destination, so the transfer cannot finish.
    ///
    /// A rename that could not complete after the bounded retry, a sidecar that could not be
    /// released, or a replacement of the same destination already in flight. Every one of
    /// them is answered by *close whatever has it open and try again*, and in each of them
    /// **nothing has moved**.
    TransferBlocked {
        /// The file, for the message.
        file_name: String,
        /// Message to show.
        message: String,
    },

    /// The transfer stopped because it was asked to.
    ///
    /// Its own kind and not [`ProjectError::Failed`]: the user pressed Cancel, nothing was
    /// written, and presenting a working control as a fault is how a safe operation acquires
    /// a reputation for being unreliable.
    Cancelled {
        /// Message to show.
        message: String,
    },

    /// Anything else: migration failure, a broken file, a disk problem.
    Failed {
        /// Message to show.
        message: String,
    },
}

/// What a user creates and opens, in the interface's own words.
///
/// `evara-db` calls it a project, and so does every identifier in this crate. The
/// interface calls it a **campus** — see `App.tsx` for why, and for why the longer
/// "campus file" is used wherever a school's own campuses could be on screen at the same
/// time.
///
/// The three messages below are written here rather than forwarded from `DbError`'s
/// `Display` for that reason. `evara-db` is a library the CLI and the tests also use, its
/// wording is not the desktop interface's to change, and this module already replaces
/// several messages for a comparable reason — see the `Failed` arm just below.
const NOT_A_CAMPUS: &str = "That file is not an Evara campus.";
const WRONG_EXTENSION_PREFIX: &str = "An Evara campus must use the";
const NONE_OPEN: &str = "No campus is open.";
/// What a cancelled transfer is told, once, here.
///
/// Said as a completed fact rather than as a failure: the user pressed Cancel, nothing was
/// written, and the protocol's guarantees are what make that sentence true rather than a
/// hope — a cancelled export leaves no `.evara` and no `.part`, and a cancelled import
/// leaves no campus file and no staging.
const CANCELLED: &str = "That was stopped before it finished. Nothing was written.";

impl From<DbError> for ProjectError {
    fn from(error: DbError) -> Self {
        let message = error.to_string();
        match error {
            DbError::SchemaTooNew { found, supported } => Self::SchemaTooNew {
                found,
                supported,
                message: format!(
                    "This campus was created by a newer version of Evara (file format \
                     v{found}; this version understands up to v{supported}). Update Evara \
                     to open it."
                ),
            },
            DbError::ProjectNotFound { .. } => Self::NotFound { message },
            DbError::NotAnEvaraProject { .. } => Self::NotAnEvaraProject {
                message: NOT_A_CAMPUS.to_owned(),
            },
            DbError::WrongExtension { expected } => Self::WrongExtension {
                expected: expected.to_owned(),
                message: format!("{WRONG_EXTENSION_PREFIX} {expected} extension."),
            },
            DbError::ProjectAlreadyExists { .. } => Self::AlreadyExists { message },
            DbError::NoProjectOpen => Self::NoProjectOpen {
                message: NONE_OPEN.to_owned(),
            },

            // --------------------------------------- M10 project transfer (Stage 4)
            //
            // Mapped here rather than in `commands::transfer`, so there is **one** place a
            // `DbError` becomes a `ProjectError`. An export reaches these directly from
            // `snapshot_open_project`; an import reaches them wrapped in
            // `TransferError::Database`, and that wrapper is unwrapped onto this impl rather
            // than classified a second time.
            DbError::SnapshotCancelled => Self::Cancelled {
                message: CANCELLED.to_owned(),
            },
            // The archive is a database and a project and is still unusable. One sentence,
            // one remedy, and the remedy is always available: ask for a fresh export. The
            // `detail` and the version are forwarded because they are the library's own
            // words about the data, not engine wording about SQLite.
            //
            // `UnknownSchemaVersion` joins them: the payload claims a version this build has
            // no migration for, which means the archive was not assembled by Evara — damage
            // in the only sense a user can act on.
            DbError::PayloadCorrupt { .. }
            | DbError::ReferentialDamage { .. }
            | DbError::SchemaDoesNotConform { .. }
            | DbError::UnknownSchemaVersion { .. } => Self::ArchiveDamaged { message },
            DbError::RenameBlocked { name, .. } => Self::TransferBlocked {
                file_name: name.clone(),
                message,
            },
            DbError::DestinationPathUnsafe { name, .. } => Self::DestinationUnsafe {
                file_name: name.clone(),
                message,
            },
            // `BackupRefused` is deliberately **not** listed: the safety copy a schema
            // rebuild needs could not be made, so nothing was changed, and its own wording
            // already says exactly that while naming no path and no engine detail. The
            // wildcard below forwards it, which is the behaviour wanted.
            // Everything else is an internal failure. These carry engine detail in their
            // source chain, so their own wording is replaced rather than forwarded.
            DbError::Open { .. } | DbError::Internal(_) | DbError::Migration(_) => Self::Failed {
                message: "That campus could not be opened. The file may be damaged or in \
                          use by another program."
                    .to_owned(),
            },
            _ => Self::Failed { message },
        }
    }
}

/// A project as the interface sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// The project's identity, where this installation recorded one.
    ///
    /// `null` for an entry remembered before identities were recorded. Carried so a screen
    /// can say *which* campus two entries are, rather than only that they are the same one.
    pub project_id: Option<String>,
    /// Whether another remembered entry carries the same identity — D13.
    ///
    /// Derived in `evara-db` by grouping the whole list, never stored. Evara cannot merge two
    /// copies of one campus, so the remedy for a duplicate is disclosure, and the disclosure
    /// belongs in the recent list itself rather than only at the moment of import.
    pub duplicate_identity: bool,
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
            project_id: recent.project_id.map(|id| id.to_string()),
            duplicate_identity: recent.duplicate_identity,
        }
    }
}

pub(crate) type CommandResult<T> = Result<T, ProjectError>;

/// Locks the workspace, failing loudly if a previous command panicked while holding it.
pub(crate) fn lock<'a>(
    state: &'a State<'_, AppState>,
) -> CommandResult<std::sync::MutexGuard<'a, Workspace>> {
    lock_state(state)
}

/// Locks the workspace of a plain [`AppState`].
///
/// The body of [`lock`], reached without a Tauri `State` guard so the transfer commands'
/// logic can be driven in a test. `State` derefs to its inner value, so the two are the same
/// lock and there is no second poisoning policy.
pub(crate) fn lock_state(state: &AppState) -> CommandResult<std::sync::MutexGuard<'_, Workspace>> {
    state.workspace.lock().map_err(|_| ProjectError::Failed {
        message: "Evara is in an inconsistent state. Please restart the application.".to_owned(),
    })
}

/// Creates a project at `path` and opens it.
#[tauri::command]
pub fn project_create(state: State<'_, AppState>, path: PathBuf) -> CommandResult<ProjectView> {
    // A generation in flight belongs to whatever was open before. Discarded first, for the
    // reason `project_close` clears the import session first: a failure to take the lock must
    // not leave a session pointing at a project that is no longer the open one.
    state.generations.clear();
    state.edits.clear();
    state.repairs.clear();
    state.transfers.clear();
    state.epoch.bump();
    let mut workspace = lock(&state)?;
    Ok(workspace.create_project(&path)?.into())
}

/// Opens an existing project.
#[tauri::command]
pub fn project_open(state: State<'_, AppState>, path: PathBuf) -> CommandResult<ProjectView> {
    state.generations.clear();
    state.edits.clear();
    state.repairs.clear();
    state.transfers.clear();
    state.epoch.bump();
    let mut workspace = lock(&state)?;
    Ok(workspace.open_project(&path)?.into())
}

/// Closes the open project, releasing its database connection.
///
/// Returns whether anything was open, so closing twice is not an error.
#[tauri::command]
pub fn project_close(state: State<'_, AppState>) -> CommandResult<bool> {
    // The import session names a school of the project being closed, so it cannot survive
    // it. Cleared first: a failure to take the lock must not leave a session pointing at a
    // project that is already gone.
    state.imports.clear();
    // A session holds an `Assembly` describing the database about to be shut, so it cannot
    // survive the close. Discarding stops a worker mid-solve rather than leaving it computing
    // an answer nothing can save.
    state.generations.clear();
    state.edits.clear();
    state.repairs.clear();
    state.transfers.clear();
    state.epoch.bump();
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
