//! Forward-only schema migration.
//!
//! # Rules
//!
//! - **Forward only.** There are no down migrations. Downgrading a project is never
//!   attempted, because a newer Evara may have written data this build cannot represent;
//!   discarding it automatically would be data loss disguised as compatibility.
//! - **Embedded.** Migration SQL is compiled into the binary, so a project can be opened
//!   on a machine that has nothing but the application.
//! - **Transactional.** All pending migrations run as one group inside a single
//!   transaction. A failure anywhere leaves the project exactly as it was — a
//!   half-migrated project file is the worst outcome available.
//! - **Immutable once released.** refinery checksums applied migrations; editing a
//!   released file makes existing projects refuse to open. Corrections ship as a new
//!   migration.

use rusqlite::Connection;

use crate::error::{DbError, Result};

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// refinery's bookkeeping table.
const HISTORY_TABLE: &str = "refinery_schema_history";

/// The highest schema version this binary can produce.
///
/// Computed from the embedded set rather than hand-maintained, so it cannot drift out of
/// step with the migration files.
#[must_use]
pub fn supported_schema_version() -> u32 {
    embedded_versions().into_iter().max().unwrap_or(0)
}

/// Versions of every embedded migration.
///
/// refinery reports versions as `i32`; Evara treats a schema version as unsigned, so the
/// conversion happens once, here, rather than at every comparison.
fn embedded_versions() -> Vec<u32> {
    embedded::migrations::runner()
        .get_migrations()
        .iter()
        .map(|migration| u32::try_from(migration.version()).unwrap_or(0))
        .collect()
}

/// Reads the schema version recorded in a project, or `0` for a database that has never
/// been migrated.
pub(crate) fn applied_schema_version(conn: &Connection) -> Result<u32> {
    if !history_table_exists(conn)? {
        return Ok(0);
    }
    let version: Option<i64> = conn.query_row(
        &format!("SELECT MAX(version) FROM {HISTORY_TABLE}"),
        [],
        |row| row.get(0),
    )?;
    Ok(version.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)))
}

fn history_table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [HISTORY_TABLE],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Refuses a project written by a newer Evara.
///
/// Called **before** any migration runs, so an unsupported project is never partially
/// touched.
pub(crate) fn reject_if_newer_than_supported(conn: &Connection) -> Result<()> {
    let found = applied_schema_version(conn)?;
    let supported = supported_schema_version();
    if found > supported {
        return Err(DbError::SchemaTooNew { found, supported });
    }
    Ok(())
}

/// Applies every pending migration, as one transaction.
///
/// Idempotent: running it against an already-current project applies nothing and
/// succeeds.
pub(crate) fn run(conn: &mut Connection) -> Result<u32> {
    reject_if_newer_than_supported(conn)?;

    embedded::migrations::runner()
        // One transaction for the whole batch: either the project reaches the new schema
        // version or it stays untouched.
        .set_grouped(true)
        // A migration whose checksum no longer matches, or one that vanished from the
        // binary, means the project and this build disagree about history. Stop rather
        // than guess.
        .set_abort_divergent(true)
        .set_abort_missing(true)
        .run(conn)?;

    applied_schema_version(conn)
}

/// Applies migrations only as far as `version`.
///
/// The only way to build a database as an *earlier* release left it. Two callers need that,
/// for different reasons:
///
/// - **Tests**, to prove a file created by the previous release still opens — the one
///   migration path every existing project will actually take.
/// - **Transfer** (M10), to generate the reference schema a staged payload is compared
///   against. A payload claiming schema version *n* is checked against what *this* binary's
///   migrations produce at *n*, so the comparison cannot drift from the migrations the way a
///   hand-written allow-list would.
///
/// That second caller is why this is no longer `#[cfg(test)]`, and why `version` is now
/// validated instead of asserted: it arrives from a file written elsewhere.
///
/// # Errors
///
/// [`DbError::UnknownSchemaVersion`] if `version` is `0` or above
/// [`supported_schema_version`]. Migration failures propagate as [`DbError::Migration`],
/// having applied nothing — refinery's grouped transaction covers the whole batch.
pub(crate) fn run_up_to(conn: &mut Connection, version: u32) -> Result<u32> {
    let supported = supported_schema_version();
    // Rejected rather than clamped. A caller asking for a version this build cannot
    // construct has a wrong idea about something, and quietly giving them the nearest
    // available schema would make the conformance comparison pass against the wrong
    // reference — which is worse than refusing, because it looks like success.
    if version == 0 || version > supported {
        return Err(DbError::UnknownSchemaVersion {
            requested: version,
            supported,
        });
    }

    // refinery's own version type is signed; the conversion happens here for the same
    // reason `embedded_versions` does it — once, at the boundary. Infallible after the
    // bound check above, and written without a panic so it stays that way if the bound
    // ever moves.
    let target = i32::try_from(version).map_err(|_| DbError::UnknownSchemaVersion {
        requested: version,
        supported,
    })?;
    embedded::migrations::runner()
        .set_grouped(true)
        .set_target(refinery::Target::Version(target))
        .run(conn)?;
    applied_schema_version(conn)
}

#[cfg(test)]
mod tests {
    use super::{
        applied_schema_version, embedded, embedded_versions, run_up_to, supported_schema_version,
    };
    use crate::connection;
    use crate::error::DbError;

    /// A fresh, empty database to migrate.
    fn blank(dir: &tempfile::TempDir, name: &str) -> rusqlite::Connection {
        connection::open_plain(&dir.path().join(name)).expect("open")
    }

    #[test]
    fn run_up_to_stops_exactly_where_it_is_told() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        // Every embedded version, not a sample: the reference generator asks for whichever
        // one a payload claims, so each must be constructible.
        for version in 1..=supported_schema_version() {
            let mut conn = blank(&dir, &format!("at-{version}.sqlite"));
            let reached = run_up_to(&mut conn, version).expect("migrate to a known version");
            assert_eq!(
                reached, version,
                "run_up_to({version}) must stop at {version}"
            );
            assert_eq!(
                applied_schema_version(&conn).expect("read back"),
                version,
                "and the database must report it"
            );
        }
    }

    #[test]
    fn run_up_to_refuses_a_version_it_cannot_build() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let supported = supported_schema_version();

        for requested in [0, supported + 1, u32::MAX] {
            let mut conn = blank(&dir, &format!("bad-{requested}.sqlite"));
            let error = run_up_to(&mut conn, requested)
                .expect_err("a version this build cannot construct must be refused");
            assert!(
                matches!(
                    error,
                    DbError::UnknownSchemaVersion { requested: r, .. } if r == requested
                ),
                "expected UnknownSchemaVersion for {requested}, got {error:?}"
            );
            assert_eq!(
                applied_schema_version(&conn).expect("read back"),
                0,
                "a refused request must have applied nothing"
            );
        }
    }

    #[test]
    fn embedded_migrations_are_present() {
        let migrations = embedded::migrations::runner().get_migrations().clone();
        assert!(
            !migrations.is_empty(),
            "no migrations were embedded; check the `migrations/` directory path"
        );
        // Not pinned to a literal: every phase adds migrations, and a test that must be
        // edited on each one stops being a test.
        assert_eq!(
            supported_schema_version(),
            embedded_versions().into_iter().max().unwrap_or(0),
            "the supported version must be the highest embedded one"
        );
        assert!(
            supported_schema_version() >= 2,
            "bootstrap (V1) and the installation-identity correction (V2) must both exist"
        );
    }

    #[test]
    fn migration_versions_are_unique_and_contiguous() {
        let mut versions = embedded_versions();
        versions.sort_unstable();

        let mut deduped = versions.clone();
        deduped.dedup();
        assert_eq!(versions, deduped, "two migrations share a version number");

        for (index, version) in versions.iter().enumerate() {
            let expected = u32::try_from(index + 1).expect("migration count fits in u32");
            assert_eq!(
                *version, expected,
                "migration versions must start at 1 and not skip; found {versions:?}"
            );
        }
    }
}
