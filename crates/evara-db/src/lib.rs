//! SQLite persistence: migrations, repositories, problem loading and solution writing.
//!
//! # Layering contract
//!
//! The **only** crate that knows SQL. Depends on `evara-core`.
//!
//! Every connection must set `foreign_keys = ON` (SQLite defaults it off, per
//! connection), `journal_mode = WAL` and a `busy_timeout`. This is enforced in one
//! connection factory and covered by a test, never left to call sites.
//!
//! # What lives here (added from Phase 1)
//!
//! - The connection factory and pragma enforcement
//! - Forward-only numbered migrations embedded in the binary
//! - Repositories, one per domain boundary group
//! - The problem loader (database to dense-index `Problem`) and the solution writer
//!
//! # Status
//!
//! Phases 1A and 1B implement the foundations only: connection policy, migrations,
//! project identity, the change feed, installation settings and the project lifecycle.
//! **There are deliberately no entity tables yet.** The point of building this first is
//! that every table added later inherits the same guarantees automatically rather than
//! having them retro-fitted.
//!
//! A [`Workspace`] is the normal entry point â€” it owns the installation settings and the
//! one open project, and attributes every write to this installation:
//!
//! ```no_run
//! # fn main() -> Result<(), evara_db::DbError> {
//! let mut workspace = evara_db::Workspace::new("0.1.0")?;
//! let project = workspace.create_project(std::path::Path::new("school.evaraproj"))?;
//! println!("project {} at schema v{}", project.project_id, project.schema_version);
//!
//! if let Some(provider) = project.synced_folder {
//!     eprintln!("warning: this project is stored in {}", provider.provider());
//! }
//! # Ok(())
//! # }
//! ```

pub mod change_log;
pub mod cloud_sync;
mod connection;
mod error;
pub mod meta;
mod migrations;
pub mod repo;
pub mod settings;
pub mod workspace;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

pub use crate::change_log::{ChangeRecord, TrackedTable};
pub use crate::cloud_sync::SyncedFolder;
pub use crate::connection::{BUSY_TIMEOUT, PragmaReport};
pub use crate::error::{DbError, InternalError, Result};
pub use crate::meta::ProjectIdentity;
pub use crate::migrations::supported_schema_version;
pub use crate::repo::Stamp;
pub use crate::repo::structure::Structure;
pub use crate::repo::time_model::TimeModel;
pub use crate::settings::{MAX_RECENT_PROJECTS, RecentProject, Settings};
pub use crate::workspace::{PROJECT_EXTENSION, ProjectSummary, Workspace};

/// An open project database.
///
/// Owns its [`rusqlite::Connection`] and never hands it out. That is what makes the
/// connection policy in [`connection`] enforceable: there is no way for another crate,
/// or a future call site in this one, to obtain a connection that skipped it.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
    path: PathBuf,
    identity: ProjectIdentity,
    schema_version: u32,
    site_id: Uuid,
}

/// File name of a path, for messages that must not carry the full path.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("the selected file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl Database {
    /// Creates a new project file and migrates it to the current schema.
    ///
    /// Separate from [`Database::open_existing`] on purpose. A single create-or-open
    /// entry point would happily migrate any SQLite file the user picked by mistake into
    /// something that looks like an Evara project.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectAlreadyExists`] if a non-empty file is already there.
    /// - [`DbError::Migration`] if migration failed, in which case nothing was applied.
    pub fn create(path: impl AsRef<Path>, app_version: &str, site_id: Uuid) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() && std::fs::metadata(path).map_or(0, |m| m.len()) > 0 {
            return Err(DbError::ProjectAlreadyExists {
                name: file_name_of(path),
            });
        }
        Self::open_or_create(path, app_version, site_id)
    }

    /// Opens an existing project and migrates it to the current schema.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectNotFound`] if nothing is there.
    /// - [`DbError::NotAnEvaraProject`] if the file is not an Evara project â€” a different
    ///   SQLite database, or not a database at all.
    /// - [`DbError::SchemaTooNew`] if written by a newer Evara. Checked **before** any
    ///   migration runs, so an unsupported project is never modified.
    /// - [`DbError::PragmaRejected`] if a required connection setting did not take effect.
    pub fn open_existing(path: impl AsRef<Path>, app_version: &str, site_id: Uuid) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(DbError::ProjectNotFound {
                name: file_name_of(path),
            });
        }
        if !Self::looks_like_a_project(path)? {
            return Err(DbError::NotAnEvaraProject {
                name: file_name_of(path),
            });
        }
        Self::open_or_create(path, app_version, site_id)
    }

    /// Shared body of [`Database::create`] and [`Database::open_existing`].
    fn open_or_create(path: &Path, app_version: &str, site_id: Uuid) -> Result<Self> {
        let mut conn = connection::open_configured(path, site_id)?;
        let schema_version = migrations::run(&mut conn)?;

        // Triggers are not data: recreating a missing one is always safe, so coverage is
        // ensured centrally on every open rather than trusted to each migration author.
        let tx = conn.transaction()?;
        change_log::install_all(&tx)?;
        tx.commit()?;

        let identity = meta::ensure(&mut conn, app_version, schema_version, site_id)?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
            identity,
            schema_version,
            site_id,
        })
    }

    /// Whether a file is an Evara project, without modifying it.
    ///
    /// A valid but *empty* SQLite file is not a project â€” that is the case `create`
    /// produces and `open_existing` must reject.
    fn looks_like_a_project(path: &Path) -> Result<bool> {
        let conn = match connection::open_plain(path) {
            Ok(conn) => conn,
            // "file is not a database" and friends mean the user picked the wrong file.
            Err(DbError::Open { .. } | DbError::Internal(_)) => return Ok(false),
            Err(other) => return Err(other),
        };
        let found: std::result::Result<i64, _> = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_meta'",
            [],
            |row| row.get(0),
        );
        Ok(matches!(found, Ok(count) if count > 0))
    }

    /// Reads a project's schema version without migrating it.
    ///
    /// Used by project opening to explain *why* a file cannot be opened before attempting
    /// to change it.
    ///
    /// # Errors
    ///
    /// [`DbError::SchemaTooNew`] if the project is from a newer Evara.
    pub fn inspect(path: impl AsRef<Path>) -> Result<u32> {
        let conn = connection::open_plain(path.as_ref())?;
        migrations::reject_if_newer_than_supported(&conn)?;
        migrations::applied_schema_version(&conn)
    }

    /// Path this project was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stable identifier of the school project, preserved across export and import.
    #[must_use]
    pub const fn project_id(&self) -> Uuid {
        self.identity.project_id
    }

    /// Installation that created this project, when recorded.
    ///
    /// Historical provenance, not the installation currently writing â€” see
    /// [`Database::site_id`].
    #[must_use]
    pub const fn created_by_site_id(&self) -> Option<Uuid> {
        self.identity.created_by_site_id
    }

    /// Installation attributed for writes through this connection.
    #[must_use]
    pub const fn site_id(&self) -> Uuid {
        self.site_id
    }

    /// Schema version currently applied.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The cloud provider this project appears to be stored under, if any.
    ///
    /// Advisory only. See [`cloud_sync`] for why this never blocks opening a project.
    #[must_use]
    pub fn synced_folder(&self) -> Option<SyncedFolder> {
        cloud_sync::detect(&self.path)
    }

    /// Reads and writes the Structure entities â€” campuses, rooms, terms and the rest.
    ///
    /// The only route to that data. Callers never see SQL or a connection.
    pub fn structure(&mut self) -> Structure<'_> {
        Structure::new(self)
    }

    /// Reads and writes the time model — cycles, bell schedules, the timeslot grid.
    ///
    /// The only route to that data, and the only way to materialise the grid.
    pub fn time_model(&mut self) -> TimeModel<'_> {
        TimeModel::new(self)
    }

    /// Reads the connection settings back from SQLite, for diagnostics and tests.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn pragma_report(&self) -> Result<PragmaReport> {
        connection::report(&self.conn)
    }

    /// Reads one project metadata value.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn metadata(&self, key: &str) -> Result<Option<String>> {
        meta::get(&self.conn, key)
    }

    /// Reads the change feed in order, from an exclusive sequence number.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn changes_since(&self, after: i64, limit: i64) -> Result<Vec<ChangeRecord>> {
        change_log::read_since(&self.conn, after, limit)
    }

    /// Runs `work` inside a transaction, committing if it succeeds.
    ///
    /// The only route to writing, so no caller can accidentally write outside a
    /// transaction. Repositories in later phases are built on this.
    ///
    /// Deliberately `pub(crate)`: it hands out a [`rusqlite::Transaction`], and SQL
    /// outside this crate is an architecture bug. Other crates get repository methods,
    /// not a cursor into the database.
    ///
    /// # Errors
    ///
    /// Propagates whatever `work` returns, having rolled back.
    pub(crate) fn write<T>(
        &mut self,
        work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let tx = self.conn.transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Reads with a prepared statement, for repositories.
    ///
    /// `pub(crate)` for the same reason as [`Database::write`].
    ///
    /// # Errors
    ///
    /// Propagates whatever `work` returns.
    pub(crate) fn read<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        work(&self.conn)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_lifecycle;

#[cfg(test)]
mod tests_structure;

#[cfg(test)]
mod tests_time_model;
