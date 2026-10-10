//! Project lifecycle: create, open, close, and the recent list.
//!
//! A [`Workspace`] is the application's single piece of persistent state. It owns the
//! installation [`Settings`] and at most one open [`Database`], and coordinates the two:
//! opening a project stamps the recent list, and every write to the project is attributed
//! to the installation's `site_id`.
//!
//! # Why this lives in `evara-db`
//!
//! It coordinates two SQLite databases, and SQL belongs in exactly one crate. Keeping it
//! here means the Tauri command layer stays what it should be — a translation of requests
//! into calls — and the whole lifecycle stays testable without a window.
//!
//! # One project at a time
//!
//! Deliberate for now. Nothing in Milestone 1 needs two open at once, and a single slot
//! makes "which project does this command act on?" unambiguous.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::Database;
use crate::cloud_sync::{self, SyncedFolder};
use crate::error::{DbError, Result};
use crate::settings::{RecentProject, Settings};

/// Extension of a live, editable Evara project.
///
/// Distinct from `.evara`, which is reserved for the portable export package. The two are
/// different things and must never be confused: one is a working SQLite database with
/// WAL sidecar files, the other is a self-contained archive for exchange.
pub const PROJECT_EXTENSION: &str = "evaraproj";

/// What the interface needs to know about the open project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Stable project identifier.
    pub project_id: Uuid,
    /// Name derived from the file name.
    pub display_name: String,
    /// Full path, needed to reopen and to show the user where their data is.
    pub path: PathBuf,
    /// Schema version currently applied.
    pub schema_version: u32,
    /// Installation that created the project, when recorded.
    pub created_by_site_id: Option<Uuid>,
    /// Whether this installation created it.
    pub created_here: bool,
    /// Cloud provider the file appears to sit under, if any. Advisory only.
    pub synced_folder: Option<SyncedFolder>,
}

/// The application's open project and installation settings.
#[derive(Debug)]
pub struct Workspace {
    settings: Settings,
    open: Option<Database>,
    app_version: String,
}

impl Workspace {
    /// Builds a workspace over the installation's real settings database.
    ///
    /// # Errors
    ///
    /// Propagates settings-database failures.
    pub fn new(app_version: impl Into<String>) -> Result<Self> {
        Ok(Self {
            settings: Settings::open_default()?,
            open: None,
            app_version: app_version.into(),
        })
    }

    /// Builds a workspace over a settings database at an explicit path.
    ///
    /// Tests use this so they never touch the real installation's settings.
    ///
    /// # Errors
    ///
    /// Propagates settings-database failures.
    pub fn with_settings_at(settings_path: &Path, app_version: impl Into<String>) -> Result<Self> {
        Ok(Self {
            settings: Settings::open_at(settings_path)?,
            open: None,
            app_version: app_version.into(),
        })
    }

    /// This installation's identity. Stable across restarts and across projects.
    #[must_use]
    pub const fn site_id(&self) -> Uuid {
        self.settings.site_id()
    }

    /// Creates a project and opens it.
    ///
    /// Any already-open project is closed first.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongExtension`] unless the path ends in `.evaraproj`.
    /// - [`DbError::ProjectAlreadyExists`] if a project is already there.
    pub fn create_project(&mut self, path: &Path) -> Result<ProjectSummary> {
        require_project_extension(path)?;
        self.close_project();

        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| DbError::SettingsIo {
                action: "create the folder for the new project",
                source,
            })?;
        }

        let db = Database::create(path, &self.app_version, self.site_id())?;
        self.adopt(db)
    }

    /// Opens an existing project.
    ///
    /// Any already-open project is closed first. A project that cannot be opened leaves
    /// the workspace with nothing open rather than with the previous project still
    /// loaded, so the interface never shows stale state after a failed open.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongExtension`], [`DbError::ProjectNotFound`],
    ///   [`DbError::NotAnEvaraProject`], [`DbError::SchemaTooNew`].
    pub fn open_project(&mut self, path: &Path) -> Result<ProjectSummary> {
        require_project_extension(path)?;
        self.close_project();

        let db = Database::open_existing(path, &self.app_version, self.site_id())?;
        self.adopt(db)
    }

    /// Takes ownership of a freshly opened database and records it as recent.
    fn adopt(&mut self, db: Database) -> Result<ProjectSummary> {
        let summary = self.summarise(&db);
        self.settings
            .remember_project(db.path(), Some(db.project_id()))?;
        self.open = Some(db);
        Ok(summary)
    }

    fn summarise(&self, db: &Database) -> ProjectSummary {
        let path = db.path().to_path_buf();
        ProjectSummary {
            project_id: db.project_id(),
            display_name: path.file_stem().map_or_else(
                || String::from("Untitled project"),
                |stem| stem.to_string_lossy().into_owned(),
            ),
            schema_version: db.schema_version(),
            created_by_site_id: db.created_by_site_id(),
            created_here: db.created_by_site_id() == Some(self.site_id()),
            synced_folder: cloud_sync::detect(&path),
            path,
        }
    }

    /// Closes the open project, releasing the SQLite connection.
    ///
    /// Returns whether anything was open. Idempotent.
    pub fn close_project(&mut self) -> bool {
        // Dropping the Database drops its Connection, which closes the file and
        // checkpoints the WAL.
        self.open.take().is_some()
    }

    /// The open project, if any.
    #[must_use]
    pub fn current_project(&self) -> Option<ProjectSummary> {
        self.open.as_ref().map(|db| self.summarise(db))
    }

    /// Whether a project is open.
    #[must_use]
    pub const fn has_open_project(&self) -> bool {
        self.open.is_some()
    }

    /// Recently opened projects, newest first.
    ///
    /// # Errors
    ///
    /// Propagates settings-database failures.
    pub fn recent_projects(&self) -> Result<Vec<RecentProject>> {
        self.settings.recent_projects()
    }

    /// Mutable access to the open project, for tests that exercise tracked writes.
    ///
    /// Not part of the public surface: repositories reach the database through the
    /// workspace, and SQL stays inside this crate.
    #[cfg(test)]
    pub(crate) fn open_database_for_test(&mut self) -> &mut Database {
        self.open.as_mut().expect("a project must be open")
    }

    /// Removes a project from the recent list. The file itself is untouched.
    ///
    /// # Errors
    ///
    /// Propagates settings-database failures.
    pub fn forget_recent_project(&mut self, path: &Path) -> Result<bool> {
        self.settings.forget_project(path)
    }
}

/// Rejects a path that is not a `.evaraproj` file.
///
/// Case-insensitive, because a user typing `School.EvaraProj` on Windows means the same
/// thing.
fn require_project_extension(path: &Path) -> Result<()> {
    let matches = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(PROJECT_EXTENSION));

    if matches {
        Ok(())
    } else {
        Err(DbError::WrongExtension {
            expected: PROJECT_EXTENSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PROJECT_EXTENSION, require_project_extension};
    use std::path::Path;

    #[test]
    fn accepts_the_project_extension() {
        assert!(require_project_extension(Path::new("/a/school.evaraproj")).is_ok());
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert!(require_project_extension(Path::new("/a/School.EvaraProj")).is_ok());
    }

    #[test]
    fn rejects_other_extensions() {
        // `.evara` especially: that is the portable export package, not a live project.
        for bad in [
            "/a/school.evara",
            "/a/school.sqlite",
            "/a/school",
            "/a/school.evdb",
        ] {
            assert!(
                require_project_extension(Path::new(bad)).is_err(),
                "{bad} should not be accepted as a live project"
            );
        }
    }

    #[test]
    fn the_extension_is_not_the_export_extension() {
        assert_ne!(
            PROJECT_EXTENSION, "evara",
            "live project and export must differ"
        );
    }
}
