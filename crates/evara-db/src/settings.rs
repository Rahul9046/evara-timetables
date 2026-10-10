//! Application settings: installation identity and the recent-project list.
//!
//! A second, separate SQLite database living in the platform application data directory.
//! It holds what belongs to **this installation of Evara**, never school data.
//!
//! # Why `site_id` lives here
//!
//! `site_id` identifies an installation, not a project. Phase 1A stored it inside the
//! project file, which meant opening a colleague's project adopted *their* identity and
//! attributed your edits to them. Keeping it here fixes that: the project records only
//! `created_by_site_id` as a historical fact, and the installation doing the writing is
//! supplied per connection.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::connection;
use crate::error::{DbError, Result};
use crate::meta::now_iso8601;

mod embedded {
    refinery::embed_migrations!("settings-migrations");
}

/// Stable identifier of this Evara installation.
const KEY_SITE_ID: &str = "site_id";

/// How many projects the recent list keeps.
///
/// Fixed rather than configurable: a recent list is a convenience, and an unbounded one
/// becomes a list of every project the user has ever touched.
pub const MAX_RECENT_PROJECTS: usize = 12;

/// A previously opened project, as remembered by this installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentProject {
    /// Full path to the project file.
    pub path: PathBuf,
    /// Name shown in the interface, derived from the file name.
    pub display_name: String,
    /// Project identity, when it was recorded.
    pub project_id: Option<Uuid>,
    /// When this installation last opened it, ISO-8601 UTC.
    pub last_opened_at: String,
    /// Whether the file is still where we left it.
    ///
    /// Checked when the list is read, so a project moved or deleted outside Evara shows
    /// as missing rather than failing when clicked. The list is never used to *discover*
    /// projects — Evara does not scan the filesystem.
    pub exists: bool,
}

/// This installation's settings database.
#[derive(Debug)]
pub struct Settings {
    conn: Connection,
    site_id: Uuid,
    path: PathBuf,
}

impl Settings {
    /// Opens (or creates) the settings database in the platform application data
    /// directory, assigning this installation an identity on first run.
    ///
    /// # Errors
    ///
    /// [`DbError::SettingsDirectoryUnavailable`] if the platform has no application data
    /// directory, or any SQLite failure.
    pub fn open_default() -> Result<Self> {
        Self::open_at(&default_settings_path()?)
    }

    /// Opens (or creates) a settings database at an explicit path.
    ///
    /// Tests use this so they never touch the real installation.
    ///
    /// # Errors
    ///
    /// Propagates directory creation and SQLite failures.
    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DbError::SettingsIo {
                action: "create the settings directory",
                source,
            })?;
        }

        // The settings database has no change log and no project identity, so it needs no
        // site_id of its own — it is where site_id comes from.
        let mut conn = connection::open_plain(path)?;
        embedded::migrations::runner()
            .set_grouped(true)
            .set_abort_divergent(true)
            .set_abort_missing(true)
            .run(&mut conn)?;

        let site_id = ensure_site_id(&mut conn)?;
        Ok(Self {
            conn,
            site_id,
            path: path.to_path_buf(),
        })
    }

    /// This installation's stable identifier.
    ///
    /// Created once, on first run, and unchanged thereafter — including when opening a
    /// project created by someone else.
    #[must_use]
    pub const fn site_id(&self) -> Uuid {
        self.site_id
    }

    /// Where the settings database lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records a project as most recently opened.
    ///
    /// Upserts by normalised path, so reopening the same file moves it to the top rather
    /// than adding a duplicate. Trims the list to [`MAX_RECENT_PROJECTS`].
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn remember_project(&mut self, path: &Path, project_id: Option<Uuid>) -> Result<()> {
        let key = path_key(path);
        let display_name = display_name_for(path);

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO recent_project (path_key, path, display_name, project_id, last_opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path_key) DO UPDATE SET
                 path = excluded.path,
                 display_name = excluded.display_name,
                 project_id = COALESCE(excluded.project_id, recent_project.project_id),
                 last_opened_at = excluded.last_opened_at",
            (
                &key,
                path.to_string_lossy().as_ref(),
                &display_name,
                project_id.map(|id| id.to_string()),
                now_iso8601(),
            ),
        )?;

        // Keep only the newest entries. Ordered by time then key so the result is
        // deterministic when two projects share a timestamp.
        tx.execute(
            "DELETE FROM recent_project WHERE path_key NOT IN (
                 SELECT path_key FROM recent_project
                 ORDER BY last_opened_at DESC, path_key ASC LIMIT ?1
             )",
            [MAX_RECENT_PROJECTS],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Reads the recent-project list, newest first.
    ///
    /// Each entry reports whether its file still exists. Nothing is removed
    /// automatically: a project on a disconnected network drive or an unmounted volume is
    /// missing today and back tomorrow, so forgetting it is the user's decision.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn recent_projects(&self) -> Result<Vec<RecentProject>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, display_name, project_id, last_opened_at
             FROM recent_project ORDER BY last_opened_at DESC, path_key ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let project_id: Option<String> = row.get(2)?;
            let path = PathBuf::from(path);
            Ok(RecentProject {
                exists: path.is_file(),
                path,
                display_name: row.get(1)?,
                project_id: project_id.and_then(|id| Uuid::parse_str(&id).ok()),
                last_opened_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Removes one project from the recent list. The file itself is never touched.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn forget_project(&mut self, path: &Path) -> Result<bool> {
        let removed = self.conn.execute(
            "DELETE FROM recent_project WHERE path_key = ?1",
            [path_key(path)],
        )?;
        Ok(removed > 0)
    }
}

/// Assigns this installation an identity on first run, and reads it thereafter.
fn ensure_site_id(conn: &mut Connection) -> Result<Uuid> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [KEY_SITE_ID],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(raw) = existing {
        return Uuid::parse_str(&raw).map_err(|_| DbError::InvalidMetadata {
            key: "site_id",
            value: raw,
        });
    }

    let site_id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
        (KEY_SITE_ID, site_id.to_string(), now_iso8601()),
    )?;
    Ok(site_id)
}

/// Where the settings database lives on this platform.
///
/// Windows: `%APPDATA%\Evara\Evara Timetables\data`.
/// macOS: `~/Library/Application Support/app.evara.timetables`.
fn default_settings_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("app", "Evara", "Evara Timetables")
        .ok_or(DbError::SettingsDirectoryUnavailable)?;
    Ok(dirs.data_dir().join("settings.sqlite"))
}

/// Normalises a path into a deduplication key.
///
/// Absolute where possible, and lowercased on Windows because its filesystem is
/// case-insensitive. Deliberately avoids `canonicalize`, which fails for a file that no
/// longer exists — and a deleted project is exactly the case the recent list must survive.
fn path_key(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = absolute
        .to_string_lossy()
        .replace('/', std::path::MAIN_SEPARATOR_STR);
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Human-readable name for a project path: the file name without its extension.
fn display_name_for(path: &Path) -> String {
    path.file_stem().map_or_else(
        || String::from("Untitled project"),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::{display_name_for, path_key};
    use std::path::Path;

    #[test]
    fn display_name_drops_the_extension() {
        assert_eq!(
            display_name_for(Path::new("/a/b/Northgate Secondary.evaraproj")),
            "Northgate Secondary"
        );
    }

    #[test]
    fn path_key_is_stable_for_the_same_path() {
        let a = path_key(Path::new("/tmp/school.evaraproj"));
        let b = path_key(Path::new("/tmp/school.evaraproj"));
        assert_eq!(a, b);
    }

    #[test]
    fn path_key_distinguishes_different_projects() {
        assert_ne!(
            path_key(Path::new("/tmp/a.evaraproj")),
            path_key(Path::new("/tmp/b.evaraproj"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn path_key_is_case_insensitive_on_windows() {
        assert_eq!(
            path_key(Path::new(r"C:\Work\School.evaraproj")),
            path_key(Path::new(r"c:\work\school.evaraproj")),
            "Windows paths are case-insensitive, so these are one project"
        );
    }

    #[cfg(windows)]
    #[test]
    fn path_key_normalises_separators_on_windows() {
        assert_eq!(
            path_key(Path::new(r"C:\Work\School.evaraproj")),
            path_key(Path::new("C:/Work/School.evaraproj"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn path_key_is_case_sensitive_off_windows() {
        assert_ne!(
            path_key(Path::new("/work/School.evaraproj")),
            path_key(Path::new("/work/school.evaraproj")),
            "these are genuinely different files on a case-sensitive filesystem"
        );
    }
}
