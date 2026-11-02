//! T-75…T-82 — the eight SQLite experiments, re-proven against the **bundled** engine.
//!
//! # Why these exist as tests rather than as notes
//!
//! Every one of these facts was first established with the `sqlite3` command-line shell at
//! **3.50.6**. The engine Evara actually ships is **3.50.2**, statically linked through
//! `libsqlite3-sys`, reached through `rusqlite`'s bindings rather than through the shell.
//! Same minor series, different binding, and a different set of compile-time options — the
//! shell turns `trusted_schema` off for itself, for instance, which is exactly the kind of
//! difference that makes a shell reading a bad guide to library behaviour.
//!
//! M10's design *branches* on T-75: if `VACUUM INTO` were permitted on a `query_only`
//! connection there would be one snapshot mechanism instead of two, and
//! [`super::snapshot`] would be half its size. A design that branches on a fact asserts
//! that fact in CI.
//!
//! These tests use raw SQLite databases rather than Evara projects wherever the question is
//! about the engine. A project would add migrations, triggers and fifty tables to a test
//! whose subject is one pragma.

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};
use tempfile::TempDir;

/// A small WAL-mode database with one table and a few rows.
///
/// WAL because every Evara project is in WAL, and the committed state of a WAL database is
/// split between the main file and its `-wal` sidecar — which is the whole reason a
/// file-copy backup is wrong and these two mechanisms exist.
fn source_database(dir: &TempDir, name: &str, rows: i64) -> std::path::PathBuf {
    let path = dir.path().join(name);
    let conn = Connection::open(&path).expect("open source");
    let _: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .expect("wal");
    conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, payload TEXT NOT NULL)")
        .expect("create");
    for id in 1..=rows {
        conn.execute("INSERT INTO t (id, payload) VALUES (?1, ?2)", (id, "x"))
            .expect("insert");
    }
    conn.close().expect("close source");
    path
}

/// Row count in `t`, read from a database on its own.
fn count_rows(path: &std::path::Path) -> i64 {
    let conn = Connection::open(path).expect("open for counting");
    conn.query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
        .expect("count")
}

// --------------------------------------------------------------------- T-75 / T-82

#[test]
fn t75_vacuum_into_is_refused_under_query_only_and_leaves_a_zero_byte_target() {
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 10);
    let target = dir.path().join("snapshot.db");

    let conn = Connection::open(&source).expect("open");
    conn.pragma_update(None, "query_only", "ON")
        .expect("pragma");

    let error = conn
        .execute("VACUUM INTO ?", [target.to_str().expect("utf-8 path")])
        .expect_err("`VACUUM INTO` must be refused on a query_only connection");

    // The *message* is what the design quotes, so assert on it rather than on a code: a
    // future engine that refused for a different reason would still be a refusal, and we
    // would want to know the reason changed.
    let message = error.to_string();
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "expected a readonly-database refusal, got: {message}"
    );

    // The second half, and the one that matters operationally: the refusal still created
    // the file. This is the defect `backup::before_schema_rebuild` used to leave behind,
    // and the reason `Snapshot` removes its own file on drop.
    assert!(
        target.exists(),
        "the refused `VACUUM INTO` is still expected to have created its target"
    );
    assert_eq!(
        std::fs::metadata(&target).expect("metadata").len(),
        0,
        "the target left by a refused `VACUUM INTO` is zero length"
    );
}

#[test]
fn t82_a_vacuum_into_to_a_missing_directory_leaves_nothing_behind() {
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 10);
    let missing = dir.path().join("no-such-directory");
    let target = missing.join("snapshot.db");

    let conn = Connection::open(&source).expect("open");
    conn.execute("VACUUM INTO ?", [target.to_str().expect("utf-8 path")])
        .expect_err("a target in a missing directory must fail");

    assert!(
        !missing.exists(),
        "`VACUUM INTO` must not create the directory"
    );
    assert!(!target.exists(), "and must leave no target file");
}

#[test]
fn t78_vacuum_into_refuses_a_non_empty_target_but_adopts_a_zero_length_one() {
    // ---- A correction to the accepted record. ----
    //
    // The evidence log carries this as E-04, "`VACUUM INTO` refuses an existing target --
    // *output file already exists*", and `backup.rs`'s module header leans on it: *"SQLite
    // refuses to write it if the target exists, which is also exactly the 'do not
    // overwrite an existing backup' rule, enforced by the engine rather than by a
    // `try_exists` race."*
    //
    // That is true of a target with bytes in it and **false of a zero-length one**, which
    // SQLite treats as an empty database it may initialise. The distinction is not
    // academic: T-75 proves a *refused* `VACUUM INTO` leaves exactly such a zero-length
    // file, so the two facts together describe a real sequence -- a failed backup leaves a
    // 0-byte file, and the next `VACUUM INTO` aimed at that name writes into it rather
    // than refusing.
    //
    // So the engine is not the guard the header claims. `choose_path`'s existence check is
    // load-bearing, and the cleanup this stage adds to `before_schema_rebuild` is what
    // keeps the 0-byte case from arising at all.
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 10);
    let conn = Connection::open(&source).expect("open");

    // Half one: an existing *database* is refused with the message E-04 quotes.
    let occupied = dir.path().join("occupied.db");
    {
        let other = Connection::open(&occupied).expect("create a real database there");
        other
            .execute_batch("CREATE TABLE anything (id INTEGER PRIMARY KEY)")
            .expect("give it a page");
        other.close().expect("close");
    }
    let error = conn
        .execute("VACUUM INTO ?", [occupied.to_str().expect("utf-8 path")])
        .expect_err("`VACUUM INTO` must refuse an existing database");
    assert!(
        error.to_string().contains("already exists"),
        "expected an output-file-already-exists refusal, got: {error}"
    );

    // Half two: non-empty bytes that are not a database are refused differently -- worth
    // pinning down, because the two refusals are not interchangeable and a caller matching
    // on the message would be surprised by this one.
    let garbage = dir.path().join("garbage.db");
    std::fs::write(&garbage, b"not a database at all").expect("create garbage");
    let error = conn
        .execute("VACUUM INTO ?", [garbage.to_str().expect("utf-8 path")])
        .expect_err("`VACUUM INTO` must refuse a non-database file");
    assert!(
        error.to_string().contains("not a database"),
        "expected a not-a-database refusal, got: {error}"
    );

    // Half three, and the operationally important one: a zero-length target is adopted
    // and written into, with no complaint at all.
    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, b"").expect("create a zero-length target");
    conn.execute("VACUUM INTO ?", [empty.to_str().expect("utf-8 path")])
        .expect("a zero-length target is adopted rather than refused");

    assert_eq!(
        count_rows(&empty),
        10,
        "and receives the whole database, so the engine is not protecting \
         a 0-byte leftover from being written over"
    );
}

// --------------------------------------------------------------------- T-76 / T-77

#[test]
fn t76_the_backup_api_works_from_a_query_only_connection() {
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 200);
    let target = dir.path().join("snapshot.db");

    let from = Connection::open(&source).expect("open source");
    from.pragma_update(None, "query_only", "ON")
        .expect("pragma");
    let mut to = Connection::open(&target).expect("open target");

    {
        let backup = rusqlite::backup::Backup::new(&from, &mut to).expect("begin backup");
        backup
            .step(-1)
            .expect("a whole-database step must succeed from a query_only source");
    }
    to.close().expect("close target");

    assert_eq!(
        count_rows(&target),
        200,
        "the copy taken from a query_only connection must be complete"
    );
}

#[test]
fn t77_a_stepped_backup_in_a_read_transaction_does_not_see_a_concurrent_write() {
    // ---- The test M10's headless snapshot mechanism rests on. ----
    //
    // SQLite's online backup restarts itself when it notices the source database changing
    // under it, which on its own would make a stepped copy follow a concurrent writer
    // instead of holding still. The claim being proven is that an explicit read
    // transaction on the *source connection* pins a WAL snapshot, so the copy is of one
    // coherent instant even though it is assembled over many steps.
    //
    // If this fails, the backup API cannot be used for a consistent snapshot and the
    // read-only `VACUUM INTO` fallback is the mechanism. That is Q4, and it is answered
    // here rather than assumed.
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 500);
    let target = dir.path().join("snapshot.db");

    let from = Connection::open(&source).expect("open source");
    let mut to = Connection::open(&target).expect("open target");

    // DEFERRED acquires nothing until the first read, so the read is part of taking the
    // snapshot, not a formality.
    from.execute_batch("BEGIN DEFERRED").expect("begin");
    let seen_at_start: i64 = from
        .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
        .expect("read inside the transaction");
    assert_eq!(seen_at_start, 500);

    let writer = Connection::open(&source).expect("open writer");
    writer
        .busy_timeout(std::time::Duration::from_secs(5))
        .expect("timeout");

    {
        let backup = rusqlite::backup::Backup::new(&from, &mut to).expect("begin backup");

        // Copy a little, so the backup is genuinely in progress and not an atomic step(-1).
        let first = backup.step(1).expect("first step");
        assert_eq!(
            first,
            rusqlite::backup::StepResult::More,
            "500 rows must need more than one page, or this test proves nothing"
        );

        // A different connection commits while the copy is open.
        for id in 501..=600 {
            writer
                .execute("INSERT INTO t (id, payload) VALUES (?1, ?2)", (id, "late"))
                .expect("concurrent insert");
        }

        // Finish the copy.
        loop {
            match backup.step(1).expect("step") {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More => (),
                other => panic!("unexpected step result: {other:?}"),
            }
        }
    }

    from.execute_batch("COMMIT").expect("commit the read");
    to.close().expect("close target");

    // The source really did change, so the assertion below is about isolation and not
    // about the writer having failed.
    assert_eq!(
        count_rows(&source),
        600,
        "the concurrent writer must actually have committed"
    );
    assert_eq!(
        count_rows(&target),
        500,
        "the snapshot must hold the instant the read transaction began, \
         not the rows a concurrent writer added during the copy"
    );
}

// --------------------------------------------------------------------- T-79 / T-80

#[test]
fn t79_integrity_check_is_ok_on_a_database_carrying_a_hostile_trigger() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("hostile.db");
    let conn = Connection::open(&path).expect("open");

    // Nothing here is malformed: a trigger and an extra table are ordinary SQL. That is
    // precisely the point — `integrity_check` answers "is this file structurally sound",
    // never "is this the schema I expected", which is why `conformance` exists.
    conn.execute_batch(
        "CREATE TABLE t (id INTEGER PRIMARY KEY);
         CREATE TABLE stowaway (secret TEXT);
         CREATE TRIGGER hostile AFTER INSERT ON t
         BEGIN
             INSERT INTO stowaway (secret) VALUES ('fired');
         END;",
    )
    .expect("create a hostile but well-formed schema");

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity_check");

    assert_eq!(
        integrity, "ok",
        "`integrity_check` must report `ok` here -- it is not a schema check, \
         and treating it as one is the mistake this test exists to prevent"
    );
}

#[test]
fn t80_foreign_key_check_reports_a_violation_integrity_check_calls_ok() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("dangling.db");
    let conn = Connection::open(&path).expect("open");

    // Built with enforcement *off*, which is SQLite's default and therefore the state any
    // file from elsewhere may have been written in.
    conn.pragma_update(None, "foreign_keys", "OFF")
        .expect("off");
    conn.execute_batch(
        "CREATE TABLE parent (id INTEGER PRIMARY KEY);
         CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id));
         INSERT INTO child (id, parent_id) VALUES (1, 999);",
    )
    .expect("insert a dangling reference");

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity_check");
    assert_eq!(
        integrity, "ok",
        "a dangling reference is not structural damage, so `integrity_check` says `ok`"
    );

    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign_key_check");
    assert!(
        violations > 0,
        "`foreign_key_check` must report what `integrity_check` cannot see -- \
         both are needed because they answer different questions"
    );
}

// --------------------------------------------------------------------- T-81

#[test]
fn t81_writable_schema_succeeds_by_default_and_defensive_blocks_it() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("schema.db");

    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .expect("create");

        // Undefended: `writable_schema` is settable, which means a caller could rewrite
        // `sqlite_master` directly. This is the capability `DEFENSIVE` exists to remove.
        conn.pragma_update(None, "writable_schema", "ON")
            .expect("writable_schema must be settable by default");
        let writable: i64 = conn
            .query_row("PRAGMA writable_schema", [], |row| row.get(0))
            .expect("read back");
        assert_eq!(writable, 1, "and must read back as on");
    }

    {
        let conn = Connection::open(&path).expect("reopen");
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
            .expect("set DEFENSIVE");

        // `DEFENSIVE` makes the pragma a no-op rather than an error, so the proof is in
        // the read-back, not in an `expect_err`. A test asserting a failure here would
        // pass for the wrong reason on an engine that errored instead.
        let _ = conn.pragma_update(None, "writable_schema", "ON");
        let writable: i64 = conn
            .query_row("PRAGMA writable_schema", [], |row| row.get(0))
            .expect("read back");
        assert_eq!(
            writable, 0,
            "with DEFENSIVE set, `writable_schema = ON` must not take effect"
        );
    }
}

// --------------------------------------------------------------------- the open flags

#[test]
fn a_read_only_open_flag_is_not_the_same_as_the_query_only_pragma() {
    // The distinction T-75 turns on, asserted so nobody "simplifies" the two snapshot
    // mechanisms into one. `SQLITE_OPEN_READ_ONLY` is a *file* mode and permits
    // `VACUUM INTO` (whose writes go to a different file); `query_only` is a *connection*
    // mode and refuses it.
    let dir = TempDir::new().expect("temp dir");
    let source = source_database(&dir, "source.db", 50);
    let target = dir.path().join("snapshot.db");

    let conn = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open read-only");
    conn.execute("VACUUM INTO ?", [target.to_str().expect("utf-8 path")])
        .expect("`VACUUM INTO` is permitted from a file-level read-only connection");

    assert_eq!(
        count_rows(&target),
        50,
        "and produces a complete copy -- this is the documented fallback if the \
         backup API ever stops holding a snapshot (T-77)"
    );
}
