//! Tests for the persistence foundations.
//!
//! These use real files rather than `:memory:` databases on purpose: WAL is a property
//! of the file, and an in-memory database silently reports `journal_mode = memory`.
//! Testing against memory would quietly exempt the most important guarantee from its own
//! test.

use tempfile::TempDir;

use uuid::Uuid;

use crate::change_log::{self, TrackedTable};
use crate::migrations;
use crate::{Database, DbError, meta};

const APP_VERSION: &str = "0.1.0-test";

/// A fixed stand-in for an installation identity.
fn test_site() -> Uuid {
    Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000001").expect("valid uuid")
}

/// A temporary project path. The directory lives as long as the returned handle.
fn project() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("school.evaraproj");
    (dir, path)
}

/// Creates the project if absent, otherwise reopens it.
fn open(path: &std::path::Path) -> Database {
    open_as(path, test_site())
}

fn open_as(path: &std::path::Path, site_id: Uuid) -> Database {
    if path.is_file() {
        Database::open_existing(path, APP_VERSION, site_id).expect("open project")
    } else {
        Database::create(path, APP_VERSION, site_id).expect("create project")
    }
}

// ---------------------------------------------------------------- connection policy

#[test]
fn every_connection_enforces_the_required_pragmas() {
    let (_dir, path) = project();
    let db = open(&path);

    let report = db.pragma_report().expect("read pragmas");
    assert!(report.foreign_keys, "foreign_keys must be ON: {report:?}");
    assert_eq!(report.journal_mode, "wal", "{report:?}");
    assert_eq!(report.busy_timeout_ms, 5_000, "{report:?}");
    assert_eq!(
        report.synchronous, 1,
        "synchronous must be NORMAL: {report:?}"
    );
    assert!(report.is_compliant(), "{report:?}");
}

/// The single most important test in this crate.
///
/// `foreign_keys` is off by default and scoped to one connection, so a regression here
/// would not fail loudly — it would silently let referential corruption into every
/// project from then on. This asserts enforcement by provoking an actual violation
/// rather than by reading the pragma back.
#[test]
fn foreign_keys_are_actually_enforced_not_just_reported() {
    let (_dir, path) = project();
    let mut db = open(&path);

    db.write(|tx| {
        tx.execute_batch(
            "CREATE TABLE fk_parent (id TEXT PRIMARY KEY NOT NULL) STRICT;
             CREATE TABLE fk_child (
                 id        TEXT PRIMARY KEY NOT NULL,
                 parent_id TEXT NOT NULL REFERENCES fk_parent(id)
             ) STRICT;",
        )?;
        Ok(())
    })
    .expect("create test tables");

    let violation = db.write(|tx| {
        tx.execute(
            "INSERT INTO fk_child (id, parent_id) VALUES ('c1', 'does-not-exist')",
            [],
        )?;
        Ok(())
    });

    let err = violation.expect_err("inserting a dangling foreign key must be rejected");
    assert!(
        matches!(&err, DbError::Internal(e) if e.to_string().to_lowercase().contains("foreign key")),
        "expected a foreign key violation, got: {err}"
    );
}

#[test]
fn pragmas_still_hold_after_reopening() {
    let (_dir, path) = project();
    drop(open(&path));

    let db = open(&path);
    assert!(
        db.pragma_report().expect("read pragmas").is_compliant(),
        "a reopened project must be configured identically to a new one"
    );
}

// ---------------------------------------------------------------------- migrations

#[test]
fn creating_a_new_database_applies_every_migration() {
    let (_dir, path) = project();
    let db = open(&path);

    assert_eq!(db.schema_version(), migrations::supported_schema_version());
    assert!(
        db.schema_version() >= 1,
        "at least the bootstrap migration must apply"
    );
    assert!(path.exists(), "the project file should have been created");
}

#[test]
fn migration_is_idempotent_across_reopens() {
    let (_dir, path) = project();

    let first = open(&path).schema_version();
    let second = open(&path).schema_version();
    let third = open(&path).schema_version();

    assert_eq!(first, second);
    assert_eq!(second, third);

    // Re-running must not re-apply history either.
    let db = open(&path);
    let applied: i64 = db
        .read(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .expect("count applied migrations");
    assert_eq!(
        applied,
        i64::from(migrations::supported_schema_version()),
        "each migration must be recorded exactly once"
    );
}

/// Guards the phase boundary.
///
/// Phase 1C added the static half of the Structure group and Phase 1D the time half, so
/// the whole group now exists. Everything past it — people, curriculum, activities,
/// constraints, timetables, operations — belongs to a later phase and must not arrive by
/// accident alongside something else.
#[test]
fn the_schema_contains_exactly_the_phases_shipped_so_far() {
    let (_dir, path) = project();
    let db = open(&path);

    let tables: Vec<String> = db
        .read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .expect("list tables");

    assert_eq!(
        tables,
        vec![
            // V3 — Structure, static part.
            "academic_year".to_owned(),
            // V1 — bootstrap.
            "app_meta".to_owned(),
            "building".to_owned(),
            // V4 — Structure, time part.
            "calendar_day".to_owned(),
            "campus".to_owned(),
            "change_log".to_owned(),
            "cycle".to_owned(),
            "cycle_day".to_owned(),
            "period".to_owned(),
            "period_structure".to_owned(),
            "refinery_schema_history".to_owned(),
            "resource".to_owned(),
            "room".to_owned(),
            "room_type".to_owned(),
            "school".to_owned(),
            "term".to_owned(),
            "timeslot".to_owned(),
        ],
        "the schema has drifted from the phases actually shipped"
    );

    // Named individually rather than inferred from the list above, so that a later
    // domain arriving early fails with the name of the table that jumped the queue.
    for not_yet in [
        // Phase 2 — people, and the first curriculum table.
        "teacher",
        "department",
        "teacher_qualification",
        "year_level",
        "student",
        "student_group",
        "student_group_member",
        "subject",
        // Phase 3 — curriculum.
        "course",
        "class_section",
        "class_teacher",
        "class_group",
        "enrolment",
        "course_request",
        // Phase 4 — activities.
        "activity",
        "activity_teacher",
        "activity_group",
        "activity_room_candidate",
        "activity_resource",
        "line",
        "line_member",
        "activity_link",
        "activity_link_member",
        "availability",
        // Phase 5 onwards — constraints, scheduling, operations.
        "constraint_instance",
        "constraint_ref",
        "timetable",
        "timetable_entry",
        "solve_run",
        "violation",
        "absence",
        "coverage",
    ] {
        assert!(
            !tables.contains(&not_yet.to_owned()),
            "`{not_yet}` belongs to a later phase and must not appear yet"
        );
    }
}

#[test]
fn a_project_from_a_newer_evara_is_refused_not_downgraded() {
    let (_dir, path) = project();
    {
        let mut db = open(&path);
        // Forge a future migration record, as a newer Evara would have left behind.
        db.write(|tx| {
            tx.execute(
                "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
                 VALUES (?1, 'future_schema', '2099-01-01T00:00:00.000Z', '0')",
                [i64::from(migrations::supported_schema_version()) + 5],
            )?;
            Ok(())
        })
        .expect("insert future migration row");
    }

    let err = Database::open_existing(&path, APP_VERSION, test_site())
        .expect_err("a newer project must be refused");
    assert!(err.is_schema_too_new(), "got: {err}");

    match err {
        DbError::SchemaTooNew { found, supported } => {
            assert!(found > supported, "found {found}, supported {supported}");
        }
        other => panic!("expected SchemaTooNew, got {other}"),
    }

    // And the message must tell the user what to do about it.
    let message = Database::open_existing(&path, APP_VERSION, test_site())
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("Update Evara"),
        "unhelpful message: {message}"
    );
}

/// The migration path every project on disk will actually take.
///
/// Phase 1C shipped `V3`; Phase 1D adds `V4`. Creating a fresh project exercises the whole
/// chain at once and so proves nothing about *upgrading* one — which is the case that can
/// only be got wrong once, since a released migration is immutable and a half-migrated
/// project is the worst outcome available.
#[test]
fn a_project_left_at_the_previous_schema_version_migrates_forward() {
    let (_dir, path) = project();

    // Build the project as the previous phase would have left it, and put a row in it.
    {
        let mut conn = crate::connection::open_configured(&path, test_site()).expect("open");
        let version = migrations::run_up_to(&mut conn, 3).expect("migrate to V3");
        assert_eq!(version, 3, "the fixture must stop at the previous version");

        conn.execute(
            "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
             VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', ?, ?, 1)",
            (
                uuid::Uuid::now_v7().to_string(),
                "2026-10-06T00:00:00.000Z",
                "2026-10-06T00:00:00.000Z",
            ),
        )
        .expect("write a row the old build could have written");

        let tables = table_names(&conn);
        assert!(tables.contains(&"school".to_owned()));
        assert!(
            !tables.contains(&"timeslot".to_owned()),
            "the fixture must not already have the new schema"
        );
    }

    // Open it with this build, which migrates it.
    let db = open(&path);
    assert_eq!(
        db.schema_version(),
        migrations::supported_schema_version(),
        "opening an older project brings it up to date"
    );

    let (tables, name) = db
        .read(|conn| {
            let name: String = conn.query_row("SELECT name FROM school", [], |row| row.get(0))?;
            Ok((table_names(conn), name))
        })
        .expect("read the migrated project");

    assert_eq!(name, "Northgate Secondary", "existing data survived");
    for added in [
        "cycle",
        "cycle_day",
        "period_structure",
        "period",
        "timeslot",
        "calendar_day",
    ] {
        assert!(
            tables.contains(&added.to_owned()),
            "`{added}` must exist after the upgrade"
        );
    }

    // And the change-log triggers for the new tables were installed on open, not only on
    // create — the case a create-only test would have missed entirely.
    let triggers = db
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .expect("list triggers");
    for operation in ["insert", "update", "delete"] {
        let expected = format!("trg_timeslot_changelog_{operation}");
        assert!(triggers.contains(&expected), "missing {expected}");
    }
}

/// Table names in a database, for the migration tests.
fn table_names(conn: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query");
    rows.collect::<rusqlite::Result<Vec<_>>>().expect("collect")
}

#[test]
fn inspect_reports_version_without_modifying_the_project() {
    let (_dir, path) = project();
    drop(open(&path));

    let before = std::fs::metadata(&path).expect("stat").len();
    let version = Database::inspect(&path).expect("inspect");
    let after = std::fs::metadata(&path).expect("stat").len();

    assert_eq!(version, migrations::supported_schema_version());
    assert_eq!(before, after, "inspect must not write to the project");
}

#[test]
fn a_failing_migration_rolls_back_completely() {
    let (_dir, path) = project();
    let mut db = open(&path);

    // Two statements where the second fails: the first must not survive.
    let outcome = db.write(|tx| {
        tx.execute_batch("CREATE TABLE rollback_probe (id TEXT PRIMARY KEY NOT NULL) STRICT;")?;
        tx.execute_batch("CREATE TABLE rollback_probe (id TEXT PRIMARY KEY NOT NULL) STRICT;")?;
        Ok(())
    });
    assert!(outcome.is_err(), "the duplicate table must fail");

    let exists: i64 = db
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='rollback_probe'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("check table");
    assert_eq!(exists, 0, "a failed transaction must leave no trace");
}

/// Proves the *grouped migration* guarantee, not merely that transactions work.
///
/// Two pending migrations where the second fails must leave the project at its original
/// version with neither applied. A half-migrated project file is the worst outcome
/// available, so this is the behaviour `set_grouped(true)` is there to buy.
#[test]
fn a_batch_of_migrations_is_all_or_nothing() {
    let (_dir, path) = project();
    let baseline = open(&path).schema_version();

    let mut conn = crate::connection::open_configured(&path, test_site()).expect("open");

    let good = refinery::Migration::unapplied(
        "V90__adds_a_table",
        "CREATE TABLE grouped_probe_ok (id TEXT PRIMARY KEY NOT NULL) STRICT;",
    )
    .expect("build migration");
    let bad = refinery::Migration::unapplied(
        "V91__is_not_valid_sql",
        "CREATE TABLE grouped_probe_bad (this is not valid sql);",
    )
    .expect("build migration");

    let outcome = refinery::Runner::new(&[good, bad])
        .set_grouped(true)
        .run(&mut conn);
    assert!(
        outcome.is_err(),
        "the invalid migration must fail the batch"
    );

    let survived: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='grouped_probe_ok'",
            [],
            |row| row.get(0),
        )
        .expect("check table");
    assert_eq!(
        survived, 0,
        "the migration that succeeded must be rolled back with the one that failed"
    );

    drop(conn);
    assert_eq!(
        open(&path).schema_version(),
        baseline,
        "a failed batch must leave the schema version untouched"
    );
}

// ------------------------------------------------------------------ project metadata

#[test]
fn project_and_site_identity_persist_across_reopens() {
    let (_dir, path) = project();

    let (project_id, site_id) = {
        let db = open(&path);
        (db.project_id(), db.site_id())
    };

    let reopened = open(&path);
    assert_eq!(
        reopened.project_id(),
        project_id,
        "project_id must be stable"
    );
    assert_eq!(reopened.site_id(), site_id, "site_id must be stable");
}

#[test]
fn identities_are_uuid_v7_and_distinct() {
    let (_dir, path) = project();
    let db = open(&path);

    assert_eq!(
        db.project_id().get_version_num(),
        7,
        "project_id must be UUIDv7"
    );
    assert_eq!(db.site_id().get_version_num(), 7, "site_id must be UUIDv7");
    assert_ne!(db.project_id(), db.site_id());
}

#[test]
fn separate_projects_get_separate_identities() {
    let (_a, path_a) = project();
    let (_b, path_b) = project();

    assert_ne!(open(&path_a).project_id(), open(&path_b).project_id());
}

#[test]
fn version_metadata_is_recorded() {
    let (_dir, path) = project();
    let db = open(&path);

    assert_eq!(
        db.metadata(meta::KEY_SCHEMA_VERSION).unwrap().as_deref(),
        Some(migrations::supported_schema_version().to_string().as_str())
    );
    assert_eq!(
        db.metadata(meta::KEY_CREATED_WITH).unwrap().as_deref(),
        Some(APP_VERSION)
    );
    assert!(db.metadata(meta::KEY_CREATED_AT).unwrap().is_some());
    assert!(db.metadata("no_such_key").unwrap().is_none());
}

#[test]
fn reopening_updates_last_opened_but_not_created_with() {
    let (_dir, path) = project();
    drop(Database::create(&path, "0.1.0", test_site()).expect("create"));

    let db = Database::open_existing(&path, "0.2.0", test_site())
        .expect("reopen with a newer app version");
    assert_eq!(
        db.metadata(meta::KEY_CREATED_WITH).unwrap().as_deref(),
        Some("0.1.0"),
        "the creating version must not be overwritten"
    );
    assert_eq!(
        db.metadata(meta::KEY_LAST_OPENED_WITH).unwrap().as_deref(),
        Some("0.2.0")
    );
}

// --------------------------------------------------------------------- change feed

/// Creates a minimal entity-shaped table and tracks it.
///
/// A test-only stand-in for the real tables that arrive in Phase 2, so the trigger
/// mechanism is proven before anything depends on it.
fn install_probe_table(db: &mut Database) {
    db.write(|tx| {
        tx.execute_batch(
            "CREATE TABLE probe (
                 id         TEXT PRIMARY KEY NOT NULL,
                 name       TEXT NOT NULL,
                 rev        INTEGER NOT NULL DEFAULT 1,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             ) STRICT;",
        )?;
        change_log::install(tx, &TrackedTable::new("probe").expect("valid identifier"))?;
        Ok(())
    })
    .expect("install probe table");
}

#[test]
fn change_log_records_insert_update_and_delete() {
    let (_dir, path) = project();
    let mut db = open(&path);
    install_probe_table(&mut db);

    db.write(|tx| {
        tx.execute(
            "INSERT INTO probe (id, name, rev, created_at, updated_at)
             VALUES ('p1', 'first', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )?;
        tx.execute(
            "UPDATE probe SET name = 'second', rev = 2 WHERE id = 'p1'",
            [],
        )?;
        tx.execute("DELETE FROM probe WHERE id = 'p1'", [])?;
        Ok(())
    })
    .expect("mutate probe");

    let changes = db.changes_since(0, 100).expect("read change feed");
    let ops: Vec<&str> = changes.iter().map(|c| c.op.as_str()).collect();
    assert_eq!(ops, vec!["INSERT", "UPDATE", "DELETE"]);

    for change in &changes {
        assert_eq!(change.entity_table, "probe");
        assert_eq!(change.entity_id, "p1");
        assert_eq!(
            change.site_id,
            db.site_id().to_string(),
            "site_id must be stamped"
        );
        assert!(
            change.at.ends_with('Z'),
            "timestamp must be UTC: {}",
            change.at
        );
        assert_eq!(change.at.len(), 24, "timestamp format: {}", change.at);
    }
    assert_eq!(changes[1].rev, 2, "the update must record the new revision");
    assert_eq!(
        changes[2].rev, 2,
        "the delete must record the vanishing row's revision"
    );
}

#[test]
fn change_log_sequence_is_monotonic() {
    let (_dir, path) = project();
    let mut db = open(&path);
    install_probe_table(&mut db);

    db.write(|tx| {
        for i in 0..5 {
            tx.execute(
                "INSERT INTO probe (id, name, rev, created_at, updated_at)
                 VALUES (?1, 'n', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [format!("p{i}")],
            )?;
        }
        Ok(())
    })
    .expect("insert rows");

    let changes = db.changes_since(0, 100).expect("read feed");
    assert_eq!(changes.len(), 5);
    assert!(
        changes.windows(2).all(|w| w[0].seq < w[1].seq),
        "sequence numbers must increase"
    );

    let tail = db.changes_since(changes[2].seq, 100).expect("read tail");
    assert_eq!(
        tail.len(),
        2,
        "resuming from a sequence number must skip earlier rows"
    );
}

#[test]
fn rolled_back_changes_are_not_logged() {
    let (_dir, path) = project();
    let mut db = open(&path);
    install_probe_table(&mut db);

    let _ = db.write(|tx| {
        tx.execute(
            "INSERT INTO probe (id, name, rev, created_at, updated_at)
             VALUES ('ghost', 'n', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )?;
        // Abort after a logged write.
        Err::<(), _>(DbError::MissingMetadata { key: "deliberate" })
    });

    assert!(
        db.changes_since(0, 100).expect("read feed").is_empty(),
        "a change rolled back with its transaction must not appear in the feed"
    );
}

#[test]
fn installing_triggers_twice_is_safe() {
    let (_dir, path) = project();
    let mut db = open(&path);
    install_probe_table(&mut db);

    db.write(|tx| {
        change_log::install(tx, &TrackedTable::new("probe").unwrap())?;
        change_log::install(tx, &TrackedTable::new("probe").unwrap())?;
        Ok(())
    })
    .expect("re-installing triggers must be idempotent");

    db.write(|tx| {
        tx.execute(
            "INSERT INTO probe (id, name, rev, created_at, updated_at)
             VALUES ('p1', 'n', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )?;
        Ok(())
    })
    .expect("insert");

    assert_eq!(
        db.changes_since(0, 100).expect("read feed").len(),
        1,
        "a row must be logged exactly once, not once per trigger installation"
    );
}

#[test]
fn change_log_survives_reopening() {
    let (_dir, path) = project();
    {
        let mut db = open(&path);
        install_probe_table(&mut db);
        db.write(|tx| {
            tx.execute(
                "INSERT INTO probe (id, name, rev, created_at, updated_at)
                 VALUES ('p1', 'n', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )?;
            Ok(())
        })
        .expect("insert");
    }

    let db = open(&path);
    assert_eq!(db.changes_since(0, 100).expect("read feed").len(), 1);
}

// --------------------------------------------------------------------- cloud sync

#[test]
fn a_project_in_an_ordinary_folder_is_not_flagged() {
    let (_dir, path) = project();
    assert_eq!(
        open(&path).synced_folder(),
        None,
        "a temp directory is not a synchronised folder"
    );
}
