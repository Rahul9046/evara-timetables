//! `Database::snapshot_into`: completeness, consistency, cleanup and cancellation.
//!
//! The engine-level proof that a stepped copy inside a read transaction does not follow a
//! concurrent writer is T-77, in [`super::tests_engine`]. What is here is the same property
//! asserted through the real API, plus the ownership and cleanup behaviour that makes a
//! failed export leave nothing behind.

use std::cell::Cell;
use std::path::Path;

use evara_core::calendar::WorkingDays;
use evara_core::scheduling::{Cancel, NeverCancel};
use tempfile::TempDir;
use uuid::Uuid;

use crate::transfer::{SnapshotProgress, ignore_progress};
use crate::{Database, PROJECT_EXTENSION, SchoolInput};

const APP_VERSION: &str = "0.1.0-test";

fn school_input(name: &str) -> SchoolInput {
    SchoolInput {
        name: name.to_owned(),
        code: None,
        timezone: "Europe/London".to_owned(),
        locale: "en-GB".to_owned(),
        working_days: WorkingDays::MONDAY_TO_FRIDAY,
    }
}

/// A project holding the named schools, closed and ready to be snapshotted.
fn project_with_schools(dir: &TempDir, names: &[&str]) -> std::path::PathBuf {
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let mut db = Database::create(&path, APP_VERSION, Uuid::now_v7()).expect("create project");
    for name in names {
        db.structure()
            .create_school(&school_input(name))
            .expect("create school");
    }
    drop(db);
    path
}

/// Adds one more school to an existing project, through its own handle.
fn add_school(path: &Path, name: &str) {
    let mut db =
        Database::open_existing(path, APP_VERSION, Uuid::now_v7()).expect("reopen read-write");
    db.structure()
        .create_school(&school_input(name))
        .expect("create school");
}

/// Cancels after being asked `after` times.
struct CancelAfter {
    asked: Cell<u32>,
    after: u32,
}

impl CancelAfter {
    const fn new(after: u32) -> Self {
        Self {
            asked: Cell::new(0),
            after,
        }
    }
}

impl Cancel for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let asked = self.asked.get() + 1;
        self.asked.set(asked);
        asked > self.after
    }
}

/// Files in a directory, sorted, for asserting that cleanup left nothing.
fn entries(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_snapshot_is_a_complete_readable_project() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate", "Southmoor"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open for inspection");
    let snapshot = db
        .snapshot_into(&into, &NeverCancel, &mut ignore_progress)
        .expect("snapshot");

    assert!(snapshot.path().is_file(), "the copy must exist");
    assert!(snapshot.byte_len() > 0, "and have content");
    assert_eq!(
        snapshot.schema_version(),
        crate::supported_schema_version(),
        "a copy of a current project is at the current version"
    );

    // The copy opens on its own, with no sidecar and no help.
    let copy = Database::open_for_inspection(snapshot.path()).expect("the copy must open");
    assert_eq!(
        copy.project_id(),
        snapshot.identity().project_id,
        "and is the same project -- the identity is never regenerated"
    );

    let schools = snapshot.census().get("school").copied();
    assert_eq!(
        schools,
        Some(2),
        "the census must count what is actually in the copy"
    );
}

#[test]
fn a_snapshot_is_a_single_file() {
    // The property S3's whole design depends on: a snapshot has no `-wal` or `-shm` beside
    // it, so it is one file and one rename moves it.
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open");
    let snapshot = db
        .snapshot_into(&into, &NeverCancel, &mut ignore_progress)
        .expect("snapshot");

    for suffix in crate::transfer::SIDECAR_SUFFIXES {
        let sidecar = crate::transfer::sidecar_path(snapshot.path(), suffix);
        assert!(
            !sidecar.exists(),
            "a finished snapshot must have no {suffix} beside it"
        );
    }
    assert_eq!(
        entries(&into).len(),
        1,
        "exactly one file: {:?}",
        entries(&into)
    );
}

#[test]
fn the_facts_come_from_the_copy_and_not_from_the_source() {
    // Finding 4 of the architecture review. The snapshot is taken, then the *source* gains
    // another school; the snapshot's census must still describe the instant it captured.
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let snapshot = {
        let db = Database::open_for_inspection(&path).expect("open");
        db.snapshot_into(&into, &NeverCancel, &mut ignore_progress)
            .expect("snapshot")
    };
    assert_eq!(snapshot.census().get("school").copied(), Some(1));
    let captured = snapshot.watermark();

    // Now change the source.
    add_school(&path, "Eastbrook");

    assert_eq!(
        snapshot.census().get("school").copied(),
        Some(1),
        "the snapshot describes the instant it captured, not the project as it now is"
    );
    assert_eq!(
        snapshot.watermark(),
        captured,
        "and its watermark does not move either"
    );

    let copy = Database::open_for_inspection(snapshot.path()).expect("open the copy");
    let count = copy
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM school", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(Into::into)
        })
        .expect("count in the copy");
    assert_eq!(count, 1, "and the bytes agree with the census");
}

#[test]
fn a_concurrent_write_does_not_reach_the_snapshot() {
    // T-77 through the real API rather than through raw rusqlite. The writer commits while
    // the copy is in flight, driven from the progress callback so the interleaving is
    // deterministic rather than a sleep.
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open for inspection");

    let wrote = Cell::new(false);
    let mut on_progress = |_: SnapshotProgress| {
        // Fires after the first batch, so the copy has genuinely begun. A separate
        // read-write handle commits here, which is the interleaving T-77 is about —
        // driven from the callback rather than from a sleep, so it is deterministic.
        if wrote.get() {
            return;
        }
        wrote.set(true);
        add_school(&path, "Latecomer");
    };

    let snapshot = db
        .snapshot_into(&into, &NeverCancel, &mut on_progress)
        .expect("snapshot");

    assert!(wrote.get(), "the writer must actually have run");
    assert_eq!(
        snapshot.census().get("school").copied(),
        Some(1),
        "the snapshot must hold the instant the read transaction began -- a copy that \
         followed the concurrent writer would be a plausible, internally consistent, \
         wrong export"
    );
}

#[test]
fn progress_is_reported_in_pages() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let mut reports: Vec<SnapshotProgress> = Vec::new();
    let db = Database::open_for_inspection(&path).expect("open");
    let snapshot = db
        .snapshot_into(&into, &NeverCancel, &mut |at| reports.push(at))
        .expect("snapshot");

    assert!(
        !reports.is_empty(),
        "progress must be reported at least once"
    );
    let last = reports.last().copied().expect("a report");
    assert!(last.total_pages > 0, "the page count must be known");
    assert_eq!(
        last.copied_pages, last.total_pages,
        "the final report must show the copy complete"
    );
    assert_eq!(
        last.fraction(),
        Some(1.0),
        "and its fraction must be exactly one"
    );
    assert_eq!(
        snapshot.pages(),
        last.total_pages,
        "the snapshot must agree with the last report"
    );
}

#[test]
fn a_cancellation_before_the_first_page_leaves_nothing() {
    struct AlwaysCancel;
    impl Cancel for AlwaysCancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }

    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open");
    let error = db
        .snapshot_into(&into, &AlwaysCancel, &mut ignore_progress)
        .expect_err("a cancelled snapshot must not succeed");

    assert!(
        matches!(error, crate::DbError::SnapshotCancelled),
        "expected SnapshotCancelled, got {error:?}"
    );
    assert_eq!(
        entries(&into),
        Vec::<String>::new(),
        "a cancellation before the first page must leave no file at all"
    );
}

#[test]
fn a_cancellation_during_the_copy_leaves_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate", "Southmoor", "Eastbrook"]);
    let into = dir.path().join("snapshots");

    // Allows the first check through, so the copy starts, then stops it.
    let cancel = CancelAfter::new(1);
    let db = Database::open_for_inspection(&path).expect("open");
    let error = db
        .snapshot_into(&into, &cancel, &mut ignore_progress)
        .expect_err("a cancelled snapshot must not succeed");

    assert!(
        matches!(error, crate::DbError::SnapshotCancelled),
        "expected SnapshotCancelled, got {error:?}"
    );
    assert_eq!(
        entries(&into),
        Vec::<String>::new(),
        "the partial copy and its sidecars must both be gone"
    );
}

#[test]
fn dropping_a_snapshot_removes_its_file() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open");
    let copied = {
        let snapshot = db
            .snapshot_into(&into, &NeverCancel, &mut ignore_progress)
            .expect("snapshot");
        snapshot.path().to_path_buf()
    };

    assert!(
        !copied.exists(),
        "a dropped snapshot must take its file with it -- this is what keeps a failed \
         export from leaving litter"
    );
    assert_eq!(entries(&into), Vec::<String>::new());
}

#[test]
fn keeping_a_snapshot_hands_the_file_over() {
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_for_inspection(&path).expect("open");
    let kept = db
        .snapshot_into(&into, &NeverCancel, &mut ignore_progress)
        .expect("snapshot")
        .keep();

    assert!(
        kept.is_file(),
        "a kept snapshot survives the value that produced it"
    );
}

#[test]
fn a_read_write_handle_snapshots_the_same_way() {
    // The approved design used `VACUUM INTO` here. One mechanism serves both now (see the
    // module header), so this asserts the read-write path produces an equally complete copy
    // rather than that a different mechanism ran.
    let dir = TempDir::new().expect("temp dir");
    let path = project_with_schools(&dir, &["Northgate", "Southmoor"]);
    let into = dir.path().join("snapshots");

    let db = Database::open_existing(&path, APP_VERSION, Uuid::now_v7()).expect("open read-write");
    assert!(!db.is_read_only(), "this is the read-write path");

    let snapshot = db
        .snapshot_into(&into, &NeverCancel, &mut ignore_progress)
        .expect("snapshot from a read-write handle");

    assert_eq!(snapshot.census().get("school").copied(), Some(2));
    assert_eq!(snapshot.schema_version(), crate::supported_schema_version());
    for suffix in crate::transfer::SIDECAR_SUFFIXES {
        assert!(
            !crate::transfer::sidecar_path(snapshot.path(), suffix).exists(),
            "still a single file"
        );
    }
}
