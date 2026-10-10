//! The single place a project database connection is created and configured.
//!
//! # Why this is centralised
//!
//! SQLite's `foreign_keys` pragma is **off by default and scoped to one connection**. A
//! connection opened anywhere else, by any future code path, would silently accept
//! referential corruption. The same is true of `busy_timeout` and `synchronous`.
//!
//! So there is exactly one constructor, it is `pub(crate)`, and nothing outside this
//! crate can produce a [`rusqlite::Connection`] for a project. Every connection is
//! configured and then **verified by reading the settings back** — applying a pragma can
//! fail silently, so we never assume it worked.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Name of the application-defined SQL function returning the installation doing the
/// writing. Called by the change-log triggers.
pub(crate) const SITE_ID_FUNCTION: &str = "evara_site_id";

/// How long a blocked writer waits for a lock before giving up.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Required journal mode. Write-ahead logging survives a crash mid-write and lets a
/// reader run while a write is in flight.
const JOURNAL_MODE: &str = "wal";

/// `PRAGMA synchronous = NORMAL` reports as `1`. With WAL this is durable against
/// application crashes; only a power loss can lose the most recent commits, which is the
/// right trade for a desktop document.
const SYNCHRONOUS_NORMAL: i64 = 1;

/// The configured state of a connection, read back from SQLite.
///
/// Exposed so the application can show it in diagnostics and so tests can assert the
/// guarantees hold without being handed a raw connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PragmaReport {
    /// Whether foreign key constraints are enforced.
    pub foreign_keys: bool,
    /// Journal mode, lowercase as SQLite reports it.
    pub journal_mode: String,
    /// Lock wait in milliseconds.
    pub busy_timeout_ms: i64,
    /// Raw `synchronous` level.
    pub synchronous: i64,
}

impl PragmaReport {
    /// True when every architectural requirement is met.
    #[must_use]
    pub fn is_compliant(&self) -> bool {
        self.foreign_keys
            && self.journal_mode == JOURNAL_MODE
            && self.busy_timeout_ms == i64::try_from(BUSY_TIMEOUT.as_millis()).unwrap_or(i64::MAX)
            && self.synchronous == SYNCHRONOUS_NORMAL
    }
}

/// Opens a project file and brings it to the configuration the architecture requires.
///
/// Creates the file if it does not exist. The returned connection is always verified, and
/// carries the identity of the installation that will be attributed for its writes.
pub(crate) fn open_configured(path: &Path, site_id: Uuid) -> Result<Connection> {
    let conn = open_plain(path)?;
    register_site_id(&conn, site_id)?;
    Ok(conn)
}

/// Opens and configures a database without installing the site-id function.
///
/// For databases that have no change log of their own — the settings database — and for
/// read-only inspection where nothing will be written.
pub(crate) fn open_plain(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(|source| DbError::opening(path, source))?;
    configure(&conn)?;
    verify(&conn)?;
    Ok(conn)
}

/// Registers `evara_site_id()` for this connection.
///
/// This is how the active installation is "supplied when writes occur". A SQL function is
/// used rather than a session table because SQLite refuses to let a trigger reference the
/// `temp` schema (*"trigger ... cannot reference objects in database temp"*), and a real
/// table would make per-connection state persistent and shared.
///
/// A connection without it cannot write to any tracked table: the trigger fails with
/// *"no such function"*. That is deliberate — an unattributable change is worse than a
/// refused one, and it means another tool cannot quietly write rows that the change feed
/// then misreports.
fn register_site_id(conn: &Connection, site_id: Uuid) -> Result<()> {
    let value = site_id.to_string();
    conn.create_scalar_function(
        SITE_ID_FUNCTION,
        0,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |_ctx| Ok(value.clone()),
    )?;
    Ok(())
}

/// Applies the required pragmas.
///
/// Ordering matters: `journal_mode` cannot be changed inside a transaction, so this runs
/// before anything else touches the database.
fn configure(conn: &Connection) -> Result<()> {
    // Returns the resulting mode as a row, so it must be queried rather than executed.
    let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

/// Reads every required setting back and fails if any did not take.
fn verify(conn: &Connection) -> Result<()> {
    let report = report(conn)?;

    if !report.foreign_keys {
        return Err(DbError::PragmaRejected {
            pragma: "foreign_keys",
            expected: "1".to_owned(),
            actual: "0".to_owned(),
        });
    }
    if report.journal_mode != JOURNAL_MODE {
        return Err(DbError::PragmaRejected {
            pragma: "journal_mode",
            expected: JOURNAL_MODE.to_owned(),
            actual: report.journal_mode,
        });
    }
    let expected_timeout = i64::try_from(BUSY_TIMEOUT.as_millis()).unwrap_or(i64::MAX);
    if report.busy_timeout_ms != expected_timeout {
        return Err(DbError::PragmaRejected {
            pragma: "busy_timeout",
            expected: expected_timeout.to_string(),
            actual: report.busy_timeout_ms.to_string(),
        });
    }
    if report.synchronous != SYNCHRONOUS_NORMAL {
        return Err(DbError::PragmaRejected {
            pragma: "synchronous",
            expected: SYNCHRONOUS_NORMAL.to_string(),
            actual: report.synchronous.to_string(),
        });
    }
    Ok(())
}

/// Reads the current connection settings back from SQLite.
pub(crate) fn report(conn: &Connection) -> Result<PragmaReport> {
    let foreign_keys: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    let busy_timeout_ms: i64 = conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
    let synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?;

    Ok(PragmaReport {
        foreign_keys: foreign_keys != 0,
        journal_mode: journal_mode.to_lowercase(),
        busy_timeout_ms,
        synchronous,
    })
}
