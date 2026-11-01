//! The safety copy's failure paths — that a refused copy leaves nothing behind.
//!
//! The successful path, and the `-2` suffix when a name is taken, are covered in
//! [`crate::tests_multi_school`] alongside the `V12` rebuild they protect. What is here is
//! the half that had a defect: a `VACUUM INTO` that *fails* still creates its target, zero
//! length (`transfer::tests_engine`'s T-75), and nothing used to take it away.
//!
//! Why that mattered rather than being untidy: `choose_path` steps past any name that
//! exists, so each dead 0-byte file permanently consumes one of the sixteen available
//! backup names. Sixteen failed upgrades and the seventeenth is refused with
//! `BackupRefused` — a message about backup names, on a project whose actual problem is
//! whatever was failing the copy.

use tempfile::TempDir;

use crate::connection;
use crate::error::DbError;
use crate::{PROJECT_EXTENSION, backup, migrations};

/// A project left at `V11` — one below the first rebuilding migration, so a backup is due.
fn project_awaiting_the_rebuild(dir: &TempDir, name: &str) -> std::path::PathBuf {
    let path = dir.path().join(format!("{name}.{PROJECT_EXTENSION}"));
    let mut conn = connection::open_plain(&path).expect("open");
    let applied = migrations::run_up_to(&mut conn, 11).expect("migrate to V11");
    assert_eq!(applied, 11, "the fixture must sit exactly one below V12");
    conn.close().expect("close");
    path
}

/// How many `pre-v*` files are beside the project.
fn leftovers(dir: &TempDir) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .filter_map(|entry| {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            name.contains("pre-v").then_some(name)
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_refused_vacuum_into_leaves_no_file_beside_the_project() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_awaiting_the_rebuild(&dir, "school");

    // `query_only` is the mechanism T-75 proves: `VACUUM INTO` is refused *and* creates a
    // zero-length target. Used here because it reproduces the real failure shape exactly,
    // rather than simulating it with a permissions trick that behaves differently on each
    // platform.
    let conn = connection::open_query_only(&path).expect("open query_only");

    let error = backup::before_schema_rebuild(&conn, &path, 11)
        .expect_err("the copy cannot be written, so the upgrade must be refused");

    assert!(
        matches!(error, DbError::BackupRefused { .. }),
        "a copy that cannot be written is a refusal, not a silent skip: {error:?}"
    );
    assert_eq!(
        leftovers(&dir),
        Vec::<String>::new(),
        "the zero-length target the refusal created must have been removed"
    );
}

#[test]
fn repeated_refusals_do_not_exhaust_the_backup_names() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_awaiting_the_rebuild(&dir, "school");
    let conn = connection::open_query_only(&path).expect("open query_only");

    // One more than `MAX_ATTEMPTS`. Before the fix the sixteen leftovers filled the name
    // space and this loop's last iteration failed for the wrong reason — a refusal about
    // backup names rather than about the copy.
    for attempt in 1..=17 {
        let error = backup::before_schema_rebuild(&conn, &path, 11)
            .expect_err("every attempt must be refused");
        let message = error.to_string();
        assert!(
            !message.contains("already taken"),
            "attempt {attempt} was refused for running out of names, \
             which means a previous attempt left its target behind: {message}"
        );
    }

    assert_eq!(
        leftovers(&dir),
        Vec::<String>::new(),
        "seventeen refusals must leave seventeen fewer files than they used to"
    );
}

#[test]
fn a_project_already_past_the_rebuild_is_not_copied() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("current.{PROJECT_EXTENSION}"));
    let mut conn = connection::open_plain(&path).expect("open");
    let applied = migrations::run(&mut conn).expect("migrate fully");
    assert!(applied >= backup::FIRST_REBUILDING_MIGRATION);

    let taken = backup::before_schema_rebuild(&conn, &path, applied).expect("no copy needed");

    assert!(
        taken.is_none(),
        "a project at or past the rebuild has nothing to protect"
    );
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}

#[test]
fn a_brand_new_project_is_not_copied() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("new.{PROJECT_EXTENSION}"));
    let conn = connection::open_plain(&path).expect("open");

    // `applied == 0` is a database with no schema at all. There is nothing to lose, and a
    // copy of an empty file is a file nobody should be invited to trust.
    let taken = backup::before_schema_rebuild(&conn, &path, 0).expect("no copy needed");

    assert!(taken.is_none(), "a new project has nothing to protect");
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}
