//! The change feed and the reusable trigger generator that fills it.
//!
//! # Why triggers rather than repository code
//!
//! Every mutable entity table must append to `change_log`. If repositories did that, a
//! single forgotten call site would leave a silent hole in the feed that nobody notices
//! until a future synchronisation produces a wrong merge. Triggers make coverage a
//! property of the schema instead of a property of developer discipline.
//!
//! # Using this from a migration
//!
//! A migration that adds an entity table calls [`install`] for it, so the table is
//! tracked from the moment it exists:
//!
//! ```ignore
//! change_log::install(&tx, &TrackedTable::new("teacher")?)?;
//! ```
//!
//! [`triggers_sql`] is the pure half, so the generated SQL can be asserted in a test
//! without a database.
//!
//! # Scope
//!
//! These triggers record *what changed*. They do **not** maintain `updated_at` or bump
//! `rev` — writing those is the repository's job, decided in Phase 1B. A trigger that
//! updates its own table interacts with SQLite's `recursive_triggers` setting in ways
//! that are easy to get subtly wrong, and double-logging a change would be worse than
//! writing two extra columns explicitly.

use rusqlite::{Connection, Transaction};

use crate::error::{DbError, Result};

/// A table whose changes are recorded in `change_log`.
///
/// Construction validates every identifier, because table and column names are
/// interpolated into SQL and cannot be parameter-bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedTable {
    table: String,
    id_column: String,
    rev_column: String,
}

impl TrackedTable {
    /// Tracks `table` using the conventional `id` and `rev` columns.
    pub fn new(table: &str) -> Result<Self> {
        Self::with_columns(table, "id", "rev")
    }

    /// Tracks `table` with explicitly named identity and revision columns.
    pub fn with_columns(table: &str, id_column: &str, rev_column: &str) -> Result<Self> {
        Ok(Self {
            table: validate_identifier(table)?,
            id_column: validate_identifier(id_column)?,
            rev_column: validate_identifier(rev_column)?,
        })
    }

    /// The table name.
    #[must_use]
    pub fn table(&self) -> &str {
        &self.table
    }
}

/// Accepts only a plain lowercase SQL identifier.
///
/// Deliberately strict. Anything rejected here would otherwise be interpolated into a
/// `CREATE TRIGGER` statement, and the project's own naming convention is lowercase
/// snake_case anyway.
fn validate_identifier(value: &str) -> Result<String> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if valid {
        Ok(value.to_owned())
    } else {
        Err(DbError::InvalidIdentifier {
            value: value.to_owned(),
        })
    }
}

/// SQLite expression producing an ISO-8601 UTC timestamp with millisecond precision.
///
/// Matches `meta::now_iso8601`, so timestamps written by SQL and by Rust sort together.
const NOW_EXPR: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// SQLite expression resolving the installation doing the writing.
///
/// An application-defined function registered per connection (see
/// `connection::register_site_id`), *not* a lookup in the project. The project records
/// only who created it; the installation making a given change is whoever has the file
/// open, and that is connection state.
const SITE_EXPR: &str = "evara_site_id()";

/// Builds the three trigger statements that keep `change_log` fed for one table.
///
/// Pure, so the output can be asserted without a database.
#[must_use]
pub fn triggers_sql(spec: &TrackedTable) -> String {
    use std::fmt::Write as _;

    let TrackedTable {
        table,
        id_column,
        rev_column,
    } = spec;

    let mut sql = String::new();
    for (op, when, row) in [
        ("INSERT", "AFTER INSERT", "NEW"),
        ("UPDATE", "AFTER UPDATE", "NEW"),
        ("DELETE", "AFTER DELETE", "OLD"),
    ] {
        let suffix = op.to_lowercase();
        let _ = write!(
            sql,
            "CREATE TRIGGER IF NOT EXISTS trg_{table}_changelog_{suffix}
             {when} ON {table} FOR EACH ROW
             BEGIN
                 INSERT INTO change_log (site_id, entity_table, entity_id, op, rev, at)
                 VALUES ({SITE_EXPR}, '{table}', {row}.{id_column}, '{op}', {row}.{rev_column}, {NOW_EXPR});
             END;
"
        );
    }
    sql
}

/// Every entity table whose changes are recorded.
///
/// This is the registry a migration adds to. It is checked from two directions, so a
/// table cannot quietly escape tracking:
///
/// 1. [`install_all`] runs on every open, creating any missing trigger.
/// 2. A schema-level test discovers entity tables from `sqlite_master` and fails if one is
///    absent from this list or missing a trigger. Forgetting to add a table here is
///    therefore a test failure, not a silent hole in the change feed.
pub const TRACKED_TABLES: &[&str] = &[
    // V3 — Structure, static part.
    "school",
    "campus",
    "building",
    "room_type",
    "room",
    "resource",
    "academic_year",
    "term",
    // V4 — Structure, time part.
    "cycle",
    "cycle_day",
    "period_structure",
    "period",
    "timeslot",
    "calendar_day",
];

/// Installs the change-log triggers for one table.
///
/// Idempotent — every statement is `CREATE TRIGGER IF NOT EXISTS`.
pub fn install(tx: &Transaction<'_>, spec: &TrackedTable) -> Result<()> {
    tx.execute_batch(&triggers_sql(spec))?;
    Ok(())
}

/// Ensures every table in [`TRACKED_TABLES`] has its triggers.
///
/// Runs after migrations on every open rather than inside each migration. Triggers are not
/// data: recreating a missing one is always safe, and doing it centrally means a migration
/// author cannot forget. Tables listed but not yet created by a migration are skipped, so
/// the registry may name a table ahead of the migration that introduces it.
pub(crate) fn install_all(tx: &Transaction<'_>) -> Result<()> {
    for table in TRACKED_TABLES {
        if !table_exists(tx, table)? {
            continue;
        }
        install(tx, &TrackedTable::new(table)?)?;
    }
    Ok(())
}

fn table_exists(tx: &Transaction<'_>, table: &str) -> Result<bool> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// One recorded change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRecord {
    /// Monotonic sequence number.
    pub seq: i64,
    /// Installation that made the change.
    pub site_id: String,
    /// Table the row belongs to.
    pub entity_table: String,
    /// Identifier of the changed row.
    pub entity_id: String,
    /// `INSERT`, `UPDATE` or `DELETE`.
    pub op: String,
    /// Row revision at the time of the change.
    pub rev: i64,
    /// ISO-8601 UTC timestamp.
    pub at: String,
}

/// Reads the change feed in order, from an exclusive sequence number.
///
/// `after = 0` returns the feed from the beginning.
pub(crate) fn read_since(conn: &Connection, after: i64, limit: i64) -> Result<Vec<ChangeRecord>> {
    let mut stmt = conn.prepare(
        "SELECT seq, site_id, entity_table, entity_id, op, rev, at
         FROM change_log WHERE seq > ?1 ORDER BY seq LIMIT ?2",
    )?;
    let rows = stmt.query_map((after, limit), |row| {
        Ok(ChangeRecord {
            seq: row.get(0)?,
            site_id: row.get(1)?,
            entity_table: row.get(2)?,
            entity_id: row.get(3)?,
            op: row.get(4)?,
            rev: row.get(5)?,
            at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::{TrackedTable, triggers_sql};

    #[test]
    fn rejects_identifiers_that_could_carry_sql() {
        for bad in [
            "teacher; DROP TABLE app_meta",
            "teacher--",
            "\"teacher\"",
            "Teacher",
            "1teacher",
            "",
            "teacher table",
            "teacher'",
        ] {
            assert!(
                TrackedTable::new(bad).is_err(),
                "{bad:?} should have been rejected as an identifier"
            );
        }
    }

    #[test]
    fn accepts_conventional_names() {
        for good in [
            "teacher",
            "student_group",
            "activity_teacher",
            "_staging",
            "room2",
        ] {
            assert!(
                TrackedTable::new(good).is_ok(),
                "{good:?} should be accepted"
            );
        }
    }

    #[test]
    fn generates_one_trigger_per_operation() {
        let sql = triggers_sql(&TrackedTable::new("teacher").unwrap());
        assert_eq!(sql.matches("CREATE TRIGGER").count(), 3);
        assert!(sql.contains("trg_teacher_changelog_insert"));
        assert!(sql.contains("trg_teacher_changelog_update"));
        assert!(sql.contains("trg_teacher_changelog_delete"));
        // Deletes must read the disappearing row, not the absent new one.
        assert!(sql.contains("OLD.id"), "{sql}");
        assert!(sql.contains("IF NOT EXISTS"), "triggers must be idempotent");
    }

    #[test]
    fn site_id_comes_from_the_connection_not_the_project() {
        let sql = triggers_sql(&TrackedTable::new("room").unwrap());
        assert!(
            sql.contains("evara_site_id()"),
            "attribution must call the per-connection function: {sql}"
        );
        assert!(
            !sql.contains("app_meta"),
            "the project must not supply the writing installation; that was the D16 bug"
        );
    }

    #[test]
    fn honours_custom_column_names() {
        let spec = TrackedTable::with_columns("legacy", "uuid", "version").unwrap();
        let sql = triggers_sql(&spec);
        assert!(sql.contains("NEW.uuid"), "{sql}");
        assert!(sql.contains("NEW.version"), "{sql}");
    }
}
