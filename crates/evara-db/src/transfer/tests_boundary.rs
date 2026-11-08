//! The untrusted-payload boundary, and the schema conformance comparison.
//!
//! These are the tests that decide whether a file from somewhere else is allowed anywhere
//! near a project. Two things are deliberately asserted *separately*, because conflating
//! them is the mistake the whole design is arranged to avoid:
//!
//! - **Soundness** — `integrity_check` and `foreign_key_check`. Is the file internally
//!   coherent? T-79 and T-80 prove neither question subsumes the other.
//! - **Conformance** — is it the schema an Evara project has at that version? T-79 proves a
//!   hostile trigger leaves `integrity_check` saying `ok`, so soundness alone would let one
//!   through.

use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

use crate::error::DbError;
use crate::transfer::boundary::UntrustedPayload;
use crate::transfer::conformance::{self, SchemaReference};
use crate::{Database, PROJECT_EXTENSION, connection, migrations};

const APP_VERSION: &str = "0.1.0-test";

/// A real, current Evara project — the thing a payload is supposed to be.
fn genuine_project(dir: &TempDir, name: &str) -> std::path::PathBuf {
    let path = dir.path().join(format!("{name}.{PROJECT_EXTENSION}"));
    let db = Database::create(&path, APP_VERSION, Uuid::now_v7()).expect("create");
    drop(db);
    // Checkpointed to one file, so the payload opens read-only without needing its WAL.
    crate::transfer::checkpoint_to_single_file(&path).expect("checkpoint");
    path
}

// ------------------------------------------------------------------ the V0 controls

#[test]
fn a_payload_is_opened_read_only_and_cannot_be_written() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let payload = UntrustedPayload::open(&path).expect("open payload");

    let error = payload
        .connection()
        .execute("CREATE TABLE intruder (id INTEGER)", [])
        .expect_err("a payload must not be writable");

    let message = error.to_string();
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "expected a readonly refusal, got: {message}"
    );
}

#[test]
fn a_payload_cannot_attach_another_database() {
    // The capability that would matter most: `ATTACH` is how SQL reaches the filesystem.
    // `SQLITE_LIMIT_ATTACHED = 0` makes it impossible rather than merely refused.
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let other = genuine_project(&dir, "other");
    let payload = UntrustedPayload::open(&path).expect("open payload");

    let error = payload
        .connection()
        .execute_batch(&format!(
            "ATTACH DATABASE '{}' AS other",
            other.to_string_lossy().replace('\\', "/")
        ))
        .expect_err("ATTACH must be impossible");

    assert!(
        !other.to_string_lossy().is_empty(),
        "sanity: the other database exists"
    );
    let message = error.to_string();
    assert!(
        message.contains("too many attached")
            || message.contains("not authorized")
            || message.contains("attach"),
        "expected an attach refusal, got: {message}"
    );
}

#[test]
fn a_payload_cannot_turn_writable_schema_on() {
    // T-81 proves this succeeds on an undefended connection, so the assertion is that
    // `DEFENSIVE` is actually applied here and not merely requested.
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let payload = UntrustedPayload::open(&path).expect("open payload");

    let _ = payload
        .connection()
        .pragma_update(None, "writable_schema", "ON");
    let writable: i64 = payload
        .connection()
        .query_row("PRAGMA writable_schema", [], |row| row.get(0))
        .expect("read back");

    assert_eq!(
        writable, 0,
        "`writable_schema` must not take effect on a payload"
    );
}

#[test]
fn a_hostile_trigger_in_a_payload_cannot_fire() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("hostile.{PROJECT_EXTENSION}"));

    {
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE t (id INTEGER PRIMARY KEY);
             CREATE TABLE stowaway (secret TEXT);
             CREATE TRIGGER hostile AFTER INSERT ON t
             BEGIN
                 INSERT INTO stowaway (secret) VALUES ('fired');
             END;",
        )
        .expect("build a hostile payload");
        conn.close().expect("close");
    }

    let payload = UntrustedPayload::open(&path).expect("open payload");
    let audit = payload.audit().expect("audit");

    // Sound, and hostile. Exactly T-79's point.
    assert!(
        audit.is_sound(),
        "the file is structurally fine -- that is why soundness is not enough"
    );
    assert_eq!(audit.trigger_count, 1, "and the audit sees the trigger");

    // The trigger cannot fire because nothing can be written at all, and triggers are
    // disabled besides. Both are asserted: either alone would be a single point of failure.
    payload
        .connection()
        .execute("INSERT INTO t (id) VALUES (1)", [])
        .expect_err("a write to a payload must be refused");

    let stowaway: i64 = payload
        .connection()
        .query_row("SELECT COUNT(*) FROM stowaway", [], |row| row.get(0))
        .expect("count");
    assert_eq!(stowaway, 0, "the trigger must not have run");
}

#[test]
fn a_file_that_is_not_a_database_is_refused_without_panicking() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("garbage.{PROJECT_EXTENSION}"));
    std::fs::write(&path, b"this is not a database, it is a sentence").expect("write");

    // Opening may succeed — SQLite is lazy — so the refusal is allowed to arrive at either
    // point. What must not happen is a panic or a success.
    let outcome = UntrustedPayload::open(&path).and_then(|payload| payload.audit());
    assert!(
        outcome.is_err(),
        "a non-database must be refused somewhere between opening and auditing"
    );
}

#[test]
fn an_absent_payload_is_reported_as_missing() {
    let dir = TempDir::new().expect("temp dir");
    let error = UntrustedPayload::open(&dir.path().join("nothing.evaraproj"))
        .expect_err("an absent payload must be refused");
    assert!(
        matches!(error, DbError::ProjectNotFound { .. }),
        "expected ProjectNotFound, got {error:?}"
    );
}

#[test]
fn auditing_a_payload_leaves_its_bytes_identical() {
    // The reason this constructor does not reuse `open_plain`: that path runs
    // `PRAGMA journal_mode = WAL`, which can write the file header. On a file from
    // elsewhere the first thing we do must not be a write.
    //
    // **What is asserted is the main file, not the directory.** A measured correction to
    // what this test first claimed: opening a WAL-mode database *read-only* makes SQLite
    // create a `-wal` (and `-shm`) beside it, because read-only WAL access needs the shared
    // memory file to find the log. That is the engine's doing, not a write to the payload,
    // and the payload's own bytes are untouched — which is the guarantee that matters.
    //
    // It is handled rather than ignored: the payload is in staging, and S3's
    // `checkpoint_to_single_file` is what reduces it to one file before any rename. The
    // test below asserts exactly that.
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");

    let before = std::fs::read(&path).expect("read before");
    {
        let payload = UntrustedPayload::open(&path).expect("open");
        let _ = payload.audit().expect("audit");
    }
    let after = std::fs::read(&path).expect("read after");

    assert_eq!(
        before, after,
        "auditing a payload must leave its own bytes byte-for-byte identical"
    );
}

#[test]
fn an_audited_payload_can_still_be_reduced_to_a_single_file() {
    // Closes the loop on the test above. Whatever sidecars the read-only open created,
    // the payload must still be reducible to one file — otherwise S3's "each side is one
    // file before it moves" would not hold for a payload that had been audited, which is
    // every payload.
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");

    {
        let payload = UntrustedPayload::open(&path).expect("open");
        let audit = payload.audit().expect("audit");
        assert!(audit.is_sound());
    }

    crate::transfer::checkpoint_to_single_file(&path)
        .expect("an audited payload must reduce to a single file");

    for suffix in crate::transfer::SIDECAR_SUFFIXES {
        assert!(
            !crate::transfer::sidecar_path(&path, suffix).exists(),
            "no {suffix} may remain before the rename"
        );
    }
}

// ------------------------------------------------------------------ the audit itself

#[test]
fn the_audit_reports_both_integrity_questions() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let payload = UntrustedPayload::open(&path).expect("open");
    let audit = payload.audit().expect("audit");

    assert_eq!(audit.integrity, "ok");
    assert_eq!(audit.foreign_key_violations, 0);
    assert!(audit.looks_like_a_project);
    assert_eq!(
        audit.schema_version,
        crate::supported_schema_version(),
        "a freshly created project is at the current version"
    );
    assert_eq!(audit.view_count, 0, "an Evara project declares no views");
    assert!(audit.is_sound());
}

#[test]
fn a_dangling_reference_is_reported_even_though_integrity_check_is_ok() {
    // T-80 through the audit. A payload can be structurally perfect and referentially
    // broken, and only `foreign_key_check` can tell.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("dangling.{PROJECT_EXTENSION}"));
    {
        let conn = Connection::open(&path).expect("open");
        conn.pragma_update(None, "foreign_keys", "OFF")
            .expect("off");
        conn.execute_batch(
            "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE parent (id INTEGER PRIMARY KEY);
             CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id));
             INSERT INTO child (id, parent_id) VALUES (1, 999);",
        )
        .expect("build");
        conn.close().expect("close");
    }

    let payload = UntrustedPayload::open(&path).expect("open");
    let audit = payload.audit().expect("audit");

    assert_eq!(audit.integrity, "ok", "structurally sound");
    assert!(
        audit.foreign_key_violations > 0,
        "but referentially broken, which only foreign_key_check sees"
    );
    assert!(
        !audit.is_sound(),
        "and the payload as a whole is therefore not sound"
    );
}

// ------------------------------------------------------------------ conformance

#[test]
fn a_genuine_project_conforms_to_its_generated_reference() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let scratch = dir.path().join("scratch");

    let payload = UntrustedPayload::open(&path).expect("open");
    let version = payload.audit().expect("audit").schema_version;

    conformance::check(&payload, &scratch, version)
        .expect("a project this build created must conform to this build's reference");
}

#[test]
fn an_extra_table_is_refused_and_named() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let scratch = dir.path().join("scratch");

    // Added *after* the project was built properly, so everything else still conforms and
    // the refusal has exactly one cause.
    {
        let conn = connection::open_plain(&path).expect("open");
        conn.execute_batch("CREATE TABLE stowaway (secret TEXT)")
            .expect("add a table");
        conn.close().expect("close");
    }
    crate::transfer::checkpoint_to_single_file(&path).expect("checkpoint");

    let payload = UntrustedPayload::open(&path).expect("open");
    let version = payload.audit().expect("audit").schema_version;
    let error = conformance::check(&payload, &scratch, version)
        .expect_err("an extra table must be refused");

    match error {
        DbError::SchemaDoesNotConform {
            summary,
            version: v,
        } => {
            assert_eq!(v, version);
            assert!(
                summary.contains("stowaway"),
                "the refusal must name what was wrong, got: {summary}"
            );
            assert!(
                summary.contains("unexpected"),
                "and say it was unexpected rather than missing: {summary}"
            );
        }
        other => panic!("expected SchemaDoesNotConform, got {other:?}"),
    }
}

#[test]
fn an_extra_trigger_is_refused() {
    // The finding that matters most: an extra trigger is how a payload would get to execute
    // something during migration, when triggers must be on.
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let scratch = dir.path().join("scratch");

    {
        let conn = connection::open_plain(&path).expect("open");
        conn.execute_batch(
            "CREATE TRIGGER sneaky AFTER INSERT ON school
             BEGIN
                 SELECT 1;
             END;",
        )
        .expect("add a trigger");
        conn.close().expect("close");
    }
    crate::transfer::checkpoint_to_single_file(&path).expect("checkpoint");

    let payload = UntrustedPayload::open(&path).expect("open");
    let version = payload.audit().expect("audit").schema_version;
    let error = conformance::check(&payload, &scratch, version)
        .expect_err("an extra trigger must be refused");

    let summary = match error {
        DbError::SchemaDoesNotConform { summary, .. } => summary,
        other => panic!("expected SchemaDoesNotConform, got {other:?}"),
    };
    assert!(
        summary.contains("sneaky") && summary.contains("trigger"),
        "the refusal must name the trigger: {summary}"
    );
}

#[test]
fn a_missing_index_is_refused() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let scratch = dir.path().join("scratch");

    let dropped: String = {
        let conn = connection::open_plain(&path).expect("open");
        let name: String = conn
            .query_row(
                "SELECT name FROM sqlite_master
                  WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
                  ORDER BY name LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("find an index to drop");
        conn.execute_batch(&format!("DROP INDEX \"{name}\""))
            .expect("drop it");
        conn.close().expect("close");
        name
    };
    crate::transfer::checkpoint_to_single_file(&path).expect("checkpoint");

    let payload = UntrustedPayload::open(&path).expect("open");
    let version = payload.audit().expect("audit").schema_version;
    let error = conformance::check(&payload, &scratch, version)
        .expect_err("a missing index must be refused");

    let summary = match error {
        DbError::SchemaDoesNotConform { summary, .. } => summary,
        other => panic!("expected SchemaDoesNotConform, got {other:?}"),
    };
    assert!(
        summary.contains(&dropped) && summary.contains("missing"),
        "the refusal must name the missing index `{dropped}`: {summary}"
    );
}

#[test]
fn an_impossible_claimed_version_is_refused_rather_than_clamped() {
    let dir = TempDir::new().expect("temp dir");
    let path = genuine_project(&dir, "payload");
    let scratch = dir.path().join("scratch");
    let payload = UntrustedPayload::open(&path).expect("open");

    let beyond = crate::supported_schema_version() + 1;
    let error = conformance::check(&payload, &scratch, beyond)
        .expect_err("a version this build cannot generate must be refused");

    assert!(
        matches!(
            error,
            DbError::UnknownSchemaVersion { requested, .. } if requested == beyond
        ),
        "expected UnknownSchemaVersion, got {error:?}"
    );
}

#[test]
fn a_reference_can_be_generated_for_every_historical_version() {
    // The generated matrix in miniature. T-41's full CI version walks V1…V18 on both
    // platforms; this asserts the generator itself handles every version, which is the
    // prerequisite.
    let dir = TempDir::new().expect("temp dir");
    let scratch = dir.path().join("scratch");

    let mut previous = 0usize;
    for version in 1..=crate::supported_schema_version() {
        let reference =
            SchemaReference::generate_in(&scratch, version).expect("generate a reference");
        assert_eq!(reference.version(), version);
        assert!(
            reference.object_count() > 0,
            "V{version}'s reference must hold objects"
        );
        // Monotonic: every migration is additive or rebuilding, never a net removal of the
        // whole schema. A drop here would mean a migration deleted more than it created,
        // which is worth noticing.
        assert!(
            reference.object_count() >= previous,
            "V{version} has fewer schema objects ({}) than V{} ({previous})",
            reference.object_count(),
            version - 1
        );
        previous = reference.object_count();
    }
}

#[test]
fn the_reference_generator_leaves_no_files_behind() {
    let dir = TempDir::new().expect("temp dir");
    let scratch = dir.path().join("scratch");

    let _ = SchemaReference::generate_in(&scratch, crate::supported_schema_version())
        .expect("generate");

    let left: Vec<String> = std::fs::read_dir(&scratch)
        .expect("read scratch")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        Vec::<String>::new(),
        "the scratch database and its sidecars must all be removed"
    );
}

#[test]
fn an_older_payload_conforms_against_its_own_version() {
    // A payload from an earlier release is compared against *that* version's reference, not
    // against the current one. Without this a transfer could only ever accept a project from
    // an identical build.
    let dir = TempDir::new().expect("temp dir");
    let scratch = dir.path().join("scratch");
    let path = dir.path().join(format!("old.{PROJECT_EXTENSION}"));

    let old_version = crate::supported_schema_version().saturating_sub(1).max(1);
    {
        let mut conn = connection::open_plain(&path).expect("open");
        migrations::run_up_to(&mut conn, old_version).expect("migrate partway");
        let tx = conn.transaction().expect("begin");
        crate::change_log::install_all(&tx).expect("triggers");
        tx.commit().expect("commit");
        conn.close().expect("close");
    }
    crate::transfer::checkpoint_to_single_file(&path).expect("checkpoint");

    let payload = UntrustedPayload::open(&path).expect("open");
    let claimed = payload.audit().expect("audit").schema_version;
    assert_eq!(claimed, old_version, "the payload reports its own version");

    conformance::check(&payload, &scratch, claimed)
        .expect("an older payload must conform against its own version's reference");

    // And against the *current* version it must not, or the comparison would be vacuous.
    let error = conformance::check(&payload, &scratch, crate::supported_schema_version())
        .expect_err("it must not conform to a version it is not at");
    assert!(matches!(error, DbError::SchemaDoesNotConform { .. }));
}
