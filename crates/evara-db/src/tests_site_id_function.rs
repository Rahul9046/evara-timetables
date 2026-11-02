//! `evara_site_id()` and the `SQLITE_INNOCUOUS` flag it carries.
//!
//! # What is actually being asserted, and why it is not "nothing broke"
//!
//! Marking the function innocuous changes nothing observable **today**, because the bundled
//! build sets no `-DSQLITE_TRUSTED_SCHEMA` and so `trusted_schema` defaults on — and with
//! it on, SQLite calls application-defined functions from triggers regardless of the flag.
//! A test that only opened a project and wrote a row would therefore pass before and after
//! the change, proving nothing.
//!
//! So the test that matters turns `trusted_schema` **off** and asserts the change-log
//! triggers still fire. That is the state M10 opens a staged payload in — a file whose
//! schema nobody has vouched for — and a payload still has to be migrated, which fires
//! Evara's own triggers. Without the flag those triggers fail with *"unsafe use of
//! evara_site_id()"*, and the two requirements cannot both be met.
//!
//! The negative case is asserted too: a connection with no function registered must still
//! be unable to write a tracked row. That is a guarantee
//! [`Database::open_for_inspection`] depends on, and it must not have been weakened in the
//! process of making the function usable from a trigger.

use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

use crate::{PROJECT_EXTENSION, change_log, connection, meta, migrations};

/// A migrated project with its change-log triggers installed, attributed to `site_id`.
fn migrated_project(dir: &TempDir, name: &str, site_id: Uuid) -> (std::path::PathBuf, Connection) {
    let path = dir.path().join(format!("{name}.{PROJECT_EXTENSION}"));
    let mut conn = connection::open_configured(&path, site_id).expect("open configured");
    migrations::run(&mut conn).expect("migrate");
    let tx = conn.transaction().expect("begin");
    change_log::install_all(&tx).expect("install triggers");
    tx.commit().expect("commit");
    (path, conn)
}

/// Inserts one school directly, which is a write to a tracked table.
///
/// Raw SQL rather than the repository, because the subject is the trigger and the
/// repository would add revision and timestamp handling that is beside the point.
fn insert_a_school(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    let now = meta::now_iso8601();
    conn.execute(
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES (?1, ?2, 'UTC', 'en-GB', ?3, ?3, 1)",
        (Uuid::now_v7().to_string(), name, &now),
    )
    .map(|_| ())
}

/// Rows in the change feed for `school`.
fn school_change_rows(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM change_log WHERE entity_table = 'school'",
        [],
        |row| row.get(0),
    )
    .expect("count change_log")
}

#[test]
fn change_log_triggers_fire_with_trusted_schema_off() {
    let dir = TempDir::new().expect("temp dir");
    let site_id = Uuid::now_v7();
    let (_path, conn) = migrated_project(&dir, "payload", site_id);

    // The state a staged, untrusted payload is opened in. Set *after* migration here
    // because this test is about the trigger, not about the ordering — M10's boundary sets
    // it before it touches the file at all.
    conn.pragma_update(None, "trusted_schema", "OFF")
        .expect("turn trusted_schema off");
    let trusted: i64 = conn
        .query_row("PRAGMA trusted_schema", [], |row| row.get(0))
        .expect("read back");
    assert_eq!(
        trusted, 0,
        "the pragma must have taken, or this test proves nothing"
    );

    insert_a_school(&conn, "Northgate")
        .expect("a tracked write must succeed with trusted_schema off -- this is what SQLITE_INNOCUOUS buys");

    assert_eq!(
        school_change_rows(&conn),
        1,
        "the change-log trigger must have fired and attributed the write"
    );

    let (logged_site, op): (String, String) = conn
        .query_row(
            "SELECT site_id, op FROM change_log WHERE entity_table = 'school'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the row");
    assert_eq!(
        logged_site,
        site_id.to_string(),
        "and must have recorded the installation the connection carries, \
         not a placeholder"
    );
    assert_eq!(op, "INSERT");
}

#[test]
fn change_log_triggers_fire_with_trusted_schema_on() {
    // The default today. Kept so the innocuous flag cannot be shown to have *broken* the
    // ordinary path while fixing the untrusted one.
    let dir = TempDir::new().expect("temp dir");
    let site_id = Uuid::now_v7();
    let (_path, conn) = migrated_project(&dir, "ordinary", site_id);

    let trusted: i64 = conn
        .query_row("PRAGMA trusted_schema", [], |row| row.get(0))
        .expect("read");
    assert_eq!(
        trusted, 1,
        "the bundled build sets no -DSQLITE_TRUSTED_SCHEMA, so this defaults on"
    );

    insert_a_school(&conn, "Southmoor").expect("a tracked write must succeed");
    assert_eq!(school_change_rows(&conn), 1);
}

#[test]
fn update_and_delete_triggers_also_fire_with_trusted_schema_off() {
    // All three triggers per table, because `install` creates three and only the INSERT
    // one would be exercised by the test above.
    let dir = TempDir::new().expect("temp dir");
    let (_path, conn) = migrated_project(&dir, "all-three", Uuid::now_v7());
    conn.pragma_update(None, "trusted_schema", "OFF")
        .expect("off");

    insert_a_school(&conn, "Eastbrook").expect("insert");
    conn.execute(
        "UPDATE school SET name = 'Eastbrook Renamed', rev = rev + 1 WHERE name = 'Eastbrook'",
        [],
    )
    .expect("a tracked update must succeed with trusted_schema off");
    conn.execute("DELETE FROM school WHERE name = 'Eastbrook Renamed'", [])
        .expect("a tracked delete must succeed with trusted_schema off");

    let ops: Vec<String> = conn
        .prepare("SELECT op FROM change_log WHERE entity_table = 'school' ORDER BY seq")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("collect");

    assert_eq!(
        ops,
        vec![
            "INSERT".to_owned(),
            "UPDATE".to_owned(),
            "DELETE".to_owned()
        ],
        "all three triggers must fire with trusted_schema off"
    );
}

#[test]
fn a_connection_without_the_function_still_cannot_write_a_tracked_row() {
    // The guarantee `open_for_inspection` rests on, re-asserted after the flag change.
    // `SQLITE_INNOCUOUS` says the function is safe to *call* from a trigger; it must not
    // have made the trigger able to proceed without it.
    let dir = TempDir::new().expect("temp dir");
    let (path, conn) = migrated_project(&dir, "unattributed", Uuid::now_v7());
    drop(conn);

    // `open_plain` configures the pragmas but registers no site-id function.
    let plain = connection::open_plain(&path).expect("open plain");

    let error = insert_a_school(&plain, "Nowhere")
        .expect_err("a write with no installation identity must be refused");
    let message = error.to_string();
    assert!(
        message.contains("evara_site_id") || message.contains("no such function"),
        "the refusal must come from the missing attribution function, got: {message}"
    );
    assert_eq!(
        school_change_rows(&plain),
        0,
        "and nothing may have been logged"
    );

    let schools: i64 = plain
        .query_row("SELECT COUNT(*) FROM school", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        schools, 0,
        "nor may the row itself have landed -- the trigger's failure must roll the \
         statement back"
    );
}

#[test]
fn the_function_is_registered_as_deterministic_and_innocuous() {
    // Asserted through behaviour rather than by reading flags back, which SQLite does not
    // expose: an innocuous, deterministic, zero-argument function is usable inside an
    // index-eligible expression and inside a view, both of which are refused for a
    // function lacking the flag when trusted_schema is off.
    let dir = TempDir::new().expect("temp dir");
    let site_id = Uuid::now_v7();
    let (_path, conn) = migrated_project(&dir, "flags", site_id);
    conn.pragma_update(None, "trusted_schema", "OFF")
        .expect("off");

    conn.execute_batch("CREATE VIEW who AS SELECT evara_site_id() AS site")
        .expect("a view may name an innocuous function");

    let seen: String = conn
        .query_row("SELECT site FROM who", [], |row| row.get(0))
        .expect("reading through the view must call the function");

    assert_eq!(
        seen,
        site_id.to_string(),
        "and it must return this connection's installation"
    );
}
