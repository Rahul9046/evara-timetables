//! Project metadata: identity and version bookkeeping.
//!
//! A project records two things about identity, and deliberately **not** a third:
//!
//! - **`project_id`** identifies the school project. Generated once, travels with the
//!   file, survives export and re-import. Future synchronisation uses it to know that two
//!   files are the same project.
//! - **`created_by_site_id`** records which installation created the file. A historical
//!   fact, never updated.
//!
//! The installation *currently writing* is *not* stored here. It belongs to the
//! installation ([`crate::settings`]) and reaches the change log through the
//! per-connection `evara_site_id()` function. Storing it in the project — as Phase 1A
//! did — meant opening a colleague's project adopted their identity and attributed your
//! edits to them.
//!
//! Identifiers are UUIDv7: time-ordered, so index locality is good and creation order is
//! recoverable from the identifier itself.

use rusqlite::{Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Stable identity of the school project.
pub const KEY_PROJECT_ID: &str = "project_id";
/// Installation that created this project. Historical; never updated.
pub const KEY_CREATED_BY_SITE_ID: &str = "created_by_site_id";
/// Schema version, mirrored from the migration history for diagnostics.
pub const KEY_SCHEMA_VERSION: &str = "schema_version";
/// Evara version that created the project.
pub const KEY_CREATED_WITH: &str = "created_with_app_version";
/// Evara version that opened it most recently.
pub const KEY_LAST_OPENED_WITH: &str = "last_opened_with_app_version";
/// When the project was created, ISO-8601 UTC.
pub const KEY_CREATED_AT: &str = "created_at";

/// Identity recorded inside a project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectIdentity {
    /// Stable project identifier, preserved across export and import.
    pub project_id: Uuid,
    /// Installation that created the project. `None` for a project migrated from a
    /// Phase 1A file whose provenance was never recorded.
    pub created_by_site_id: Option<Uuid>,
}

/// Current UTC timestamp, ISO-8601 with millisecond precision.
///
/// The same format SQLite's `strftime('%Y-%m-%dT%H:%M:%fZ')` produces in the change-log
/// triggers, so timestamps written by Rust and by SQL sort together.
pub(crate) fn now_iso8601() -> String {
    use time::format_description::well_known::Iso8601;
    use time::format_description::well_known::iso8601::{Config, EncodedConfig, TimePrecision};

    const CONFIG: EncodedConfig = Config::DEFAULT
        .set_time_precision(TimePrecision::Second {
            decimal_digits: std::num::NonZeroU8::new(3),
        })
        .encode();

    time::OffsetDateTime::now_utc()
        .format(&Iso8601::<CONFIG>)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00.000Z"))
}

/// Reads one metadata value.
pub(crate) fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM app_meta WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?)
}

/// Writes one metadata value, replacing any existing entry.
pub(crate) fn set(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO app_meta (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        (key, value, now_iso8601()),
    )?;
    Ok(())
}

/// Ensures a freshly migrated project has identity and version metadata.
///
/// Generates `project_id` and `site_id` only when absent, so reopening a project never
/// changes its identity. Runs in one transaction.
pub(crate) fn ensure(
    conn: &mut Connection,
    app_version: &str,
    schema_version: u32,
    site_id: Uuid,
) -> Result<ProjectIdentity> {
    let tx = conn.transaction()?;

    if get(&tx, KEY_PROJECT_ID)?.is_none() {
        set(&tx, KEY_PROJECT_ID, &Uuid::now_v7().to_string())?;
        set(&tx, KEY_CREATED_WITH, app_version)?;
        set(&tx, KEY_CREATED_AT, &now_iso8601())?;
        // Only ever written at creation. Opening someone else's project must not
        // overwrite their provenance with ours.
        set(&tx, KEY_CREATED_BY_SITE_ID, &site_id.to_string())?;
    }
    set(&tx, KEY_SCHEMA_VERSION, &schema_version.to_string())?;
    set(&tx, KEY_LAST_OPENED_WITH, app_version)?;

    tx.commit()?;

    read_identity(conn)
}

/// Reads the project identity, failing if `project_id` is missing or unreadable.
pub(crate) fn read_identity(conn: &Connection) -> Result<ProjectIdentity> {
    Ok(ProjectIdentity {
        project_id: read_uuid(conn, KEY_PROJECT_ID)?,
        // V2 writes 'unknown' for a project whose provenance predates the field, so this
        // parses leniently rather than refusing to open an older file.
        created_by_site_id: get(conn, KEY_CREATED_BY_SITE_ID)?
            .and_then(|raw| Uuid::parse_str(&raw).ok()),
    })
}

fn read_uuid(conn: &Connection, key: &'static str) -> Result<Uuid> {
    let raw = get(conn, key)?.ok_or(DbError::MissingMetadata { key })?;
    Uuid::parse_str(&raw).map_err(|_| DbError::InvalidMetadata { key, value: raw })
}

#[cfg(test)]
mod tests {
    use super::now_iso8601;

    #[test]
    fn timestamp_is_iso8601_utc_with_milliseconds() {
        let stamp = now_iso8601();
        assert_eq!(
            stamp.len(),
            24,
            "expected YYYY-MM-DDTHH:MM:SS.sssZ, got {stamp}"
        );
        assert!(stamp.ends_with('Z'), "timestamps must be UTC: {stamp}");
        assert_eq!(stamp.as_bytes()[10], b'T', "{stamp}");
        assert_eq!(stamp.as_bytes()[19], b'.', "{stamp}");
    }

    #[test]
    fn timestamps_sort_lexicographically() {
        let a = now_iso8601();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = now_iso8601();
        assert!(a <= b, "{a} should sort before {b}");
    }
}
