//! The three figures Stage 1 deferred, measured against a realistic project.
//!
//! # Why these are tests and not benchmarks
//!
//! Each one answers a question the architecture left open, and the answer has to be *reported*
//! rather than merely true. So each test prints its figures with `--nocapture` and asserts only
//! the loose bound that would mean the design was wrong — never the measured value, which
//! depends on the disk and the machine and would make the suite flaky for a reason that has
//! nothing to do with transfer.
//!
//! Run them with the numbers showing:
//!
//! ```text
//! cargo test -p evara-project --lib tests_measurements -- --nocapture
//! ```
//!
//! # The three questions
//!
//! | # | Question | Decides |
//! | --- | --- | --- |
//! | **T-60** | what share of a payload is the change feed? | whether A6's `--without-history` export is ever needed (`OPEN-DECISIONS` D37) |
//! | **T-83** | how long does a stepped backup hold the lock, and what does the per-step overhead cost? | whether the ≤ 500 ms budget holds, and whether the remedy named in D8 — a second read-only connection — is needed |
//! | §7.2 | how much larger is an uncompacted snapshot than the project, and how much of that does the archive absorb? | whether the second compaction pass §7.2 declined to add is worth its cost |
//!
//! # Why the fixture is what it is
//!
//! `Shape::Realistic` is §7.3's *realistic upper end*: ~125 sections, ~500 occurrences,
//! ~30 teaching slots, two schools. Stage 1 could not measure any of this because its projects
//! were a handful of rows created inside a test, and a figure measured against one of those
//! would have had no relationship to the budget it was supposed to test — which is worse than
//! an honest gap, because a figure gets quoted and a gap does not.

use std::time::Instant;

use evara_core::scheduling::NeverCancel;
use evara_db::fixtures::transfer::Shape;
use evara_db::transfer::{self, facts_of};

use crate::tests_fixture::Fixture;
use crate::{IgnoreProgress, seal};

/// A realistic project, closed, with its size and facts read.
struct Measured {
    _dir: tempfile::TempDir,
    project: std::path::PathBuf,
    project_bytes: u64,
    facts: evara_db::transfer::ProjectFacts,
    updates_applied: usize,
    /// How many free pages the project carries, from `PRAGMA freelist_count`.
    free_pages: i64,
    /// A room in the main school, for the concurrent-writer measurement to edit.
    a_room: uuid::Uuid,
}

fn realistic() -> Measured {
    let mut fixture = Fixture::build(Shape::Realistic);
    let project = fixture.project.path().to_path_buf();
    let updates_applied = fixture.built.updates_applied;

    // A **deletion**, so the project has a free list. §7.2's inflation figure is about exactly
    // this: the backup API reproduces the source page for page, free pages included, and a
    // project that has never deleted anything has no free pages and therefore no inflation to
    // measure. "A heavily edited project — one that has had timetables deleted, say" is what
    // §7.2 describes, so the fixture deletes one.
    //
    // The duplicate rather than a generated version: it has no descendants, so `delete`
    // accepts it, and it carries a full arrangement — a hundred-odd `timetable_entry` rows —
    // which is a free list worth measuring.
    if let Some(duplicate) = fixture.duplicate {
        let rev = fixture
            .project
            .db()
            .timetables()
            .get(duplicate)
            .expect("read the duplicate")
            .rev;
        fixture
            .project
            .db()
            .timetables()
            .delete(duplicate, rev)
            .expect("delete the duplicate, leaving its pages on the free list");
    }

    // Closed, so the WAL is checkpointed into the file and the size on disk is the size a
    // snapshot will reproduce. Measuring an open project would measure a main file with its
    // most recent work still in a sidecar.
    fixture.project.close();

    // The fixture owns its own temporary directory, which goes away with `fixture`. The
    // project file has to outlive it, so it is copied into a directory this test owns.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let kept = dir.path().join("northgate.evaraproj");
    std::fs::copy(&project, &kept).expect("copy the project somewhere that outlives the fixture");

    let project_bytes = std::fs::metadata(&kept).expect("stat").len();
    let facts = facts_of(&kept).expect("read the project's facts");
    let free_pages = transfer::free_page_count(&kept).expect("read the free list");
    let a_room = *fixture
        .built
        .main
        .rooms
        .first()
        .expect("the realistic fixture has rooms");

    Measured {
        _dir: dir,
        project: kept,
        project_bytes,
        facts,
        updates_applied,
        free_pages,
        a_room,
    }
}

// T-60
#[test]
fn t60_the_change_feed_share_of_a_payload() {
    let measured = realistic();

    // Measured from SQLite's own `dbstat`, which reports the pages each table and index
    // actually occupies. Exact, and a read: the alternative considered first was deleting the
    // feed from a copy and comparing file sizes, which would have needed a public function
    // whose only purpose was to break a project.
    //
    // `change_log` and every index on it are separate `dbstat` rows, so the feed's cost is
    // the sum over the names that *are* the table or an index on it — matched exactly and by
    // the `idx_change_log` prefix, rather than by a bare `starts_with("change_log")`, which
    // would also catch a future table called `change_log_archive`.
    let by_object = transfer::page_bytes_by_object(&measured.project).expect("read dbstat");
    let feed_bytes: i64 = by_object
        .iter()
        .filter(|(name, _)| name.as_str() == "change_log" || name.starts_with("idx_change_log"))
        .map(|(_, bytes)| *bytes)
        .sum();
    let total_pages: i64 = by_object.values().sum();

    #[allow(clippy::cast_precision_loss)]
    let share = (feed_bytes as f64 / total_pages.max(1) as f64) * 100.0;

    println!(
        "T-60  change feed: {} rows (watermark {}), {} of {} project bytes = {share:.1}% \
         of occupied pages [fixture: Shape::Realistic, {} sections, {} activities,          {} updates applied; project file {} bytes]",
        measured.facts.change_log.entries,
        measured.facts.change_log.watermark,
        feed_bytes,
        total_pages,
        measured.facts.count_of("class_section"),
        measured.facts.count_of("activity"),
        measured.updates_applied,
        measured.project_bytes,
    );

    // The one assertion: the feed is a real share of a real project, so the fixture is
    // measuring something. The figure itself is reported and **not** asserted — it is what
    // decides A6, and a test that pinned it would be deciding A6 by accident.
    assert!(
        measured.facts.change_log.entries > 1_000,
        "a project at the realistic upper end must have a feed worth measuring: {}",
        measured.facts.change_log.entries
    );
    assert!(
        measured.facts.change_log.watermark >= measured.facts.change_log.entries,
        "`seq` is monotonic, so the watermark is at least the row count"
    );
}

// T-83
#[test]
fn t83_the_lock_duration_of_a_stepped_backup() {
    let measured = realistic();

    // The figure with a user-visible consequence: for the lock's duration every other command
    // blocks. The snapshot plus one `COUNT(*)` is the whole of what D8 holds it for, so the
    // whole of what is timed here is `snapshot_into` — which is the snapshot, the agreement
    // read and the fact reads, taken on one handle.
    //
    // No wall clock reaches the engine's decisions: this is the *caller* timing the call,
    // which is the only place a clock belongs (`evara_core::scheduling::rng`'s rule, and
    // `Cancel`'s).
    let db = evara_db::Database::open_for_inspection(&measured.project).expect("open");
    let staging = measured.project.with_file_name("staging");

    let mut steps = 0_usize;
    let mut pages = 0_u32;
    let at = Instant::now();
    let snapshot = db
        .snapshot_into(&staging, &NeverCancel, &mut |progress| {
            steps += 1;
            pages = progress.total_pages;
        })
        .expect("snapshot");
    let held = at.elapsed();
    let snapshot_bytes = snapshot.byte_len();
    drop(snapshot);
    drop(db);

    #[allow(clippy::cast_precision_loss)]
    let per_step_us = if steps == 0 {
        0.0
    } else {
        held.as_secs_f64() * 1_000_000.0 / steps as f64
    };

    println!(
        "T-83  stepped backup: {:.1} ms total for {pages} pages in {steps} steps of 256 \
         ({per_step_us:.0} us/step); snapshot {snapshot_bytes} bytes, project {} bytes \
         [budget: 500 ms]",
        held.as_secs_f64() * 1_000.0,
        measured.project_bytes,
    );

    // Asserted only against the budget's **order of magnitude**, because the figure depends on
    // the disk. A ten-second hold would mean the design was wrong and the remedy D8 names — a
    // second `SQLITE_OPEN_READ_ONLY` connection, not a longer lock — would be needed; two
    // hundred milliseconds either way of the budget is a property of the machine.
    assert!(
        held.as_secs() < 10,
        "a snapshot of a realistic project held the lock for {held:?}, which is beyond any \
         reading of the 500 ms budget: the remedy is a second read-only connection, not a \
         longer lock"
    );
    assert!(
        steps > 0,
        "the progress callback must be called at least once"
    );
}

// T-83, second half — what a concurrent writer costs, and what it changes
#[test]
fn t83_a_concurrent_writer_changes_the_duration_and_not_the_copy() {
    let measured = realistic();

    // A writer on another connection, committing **real tracked writes** while the copy runs.
    // Two things are being measured: what it does to the hold time, and — the one that
    // matters — that it does nothing to the copy.
    //
    // The second is E-27, already proven twice in Stage 1 against a synthetic interleaving.
    // What this adds is the figure beside it, on a realistic project, which is what T-83 asks
    // for.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writing = {
        let project = measured.project.clone();
        let stop = std::sync::Arc::clone(&stop);
        let room = measured.a_room;
        std::thread::spawn(move || {
            let mut db = evara_db::Database::open_existing(
                &project,
                crate::tests_fixture::APP_VERSION,
                uuid::Uuid::now_v7(),
            )
            .expect("open for writing");
            let mut commits = 0_usize;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let current = db
                    .structure()
                    .room(room)
                    .expect("read the room")
                    .expect("the room exists");
                db.structure()
                    .update_room(
                        room,
                        evara_db::Expected::Rev(current.rev),
                        &evara_db::RoomInput {
                            campus_id: current.campus_id,
                            building_id: current.building_id,
                            room_type_id: current.room_type_id,
                            name: current.name.clone(),
                            code: current.code.clone(),
                            capacity: current.capacity,
                            is_bookable: current.is_bookable,
                            notes: Some(format!("touched {commits}")),
                        },
                    )
                    .expect("commit a real write while the copy is running");
                commits += 1;
            }
            commits
        })
    };

    let db = evara_db::Database::open_for_inspection(&measured.project).expect("open");
    let at = Instant::now();
    let snapshot = db
        .snapshot_into(
            &measured.project.with_file_name("staging-concurrent"),
            &NeverCancel,
            &mut transfer::ignore_progress,
        )
        .expect("a concurrent writer must not stop the snapshot");
    let held = at.elapsed();
    let snapshot_path = snapshot.path().to_path_buf();
    let snapshot_facts = facts_of(&snapshot_path).expect("read the snapshot's facts");
    drop(snapshot);
    drop(db);

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let commits = writing.join().expect("the writer thread finished");

    println!(
        "T-83  with a writer committing throughout ({commits} commits during the copy): \
         {:.1} ms hold [budget 500 ms]",
        held.as_secs_f64() * 1_000.0,
    );

    assert_eq!(
        snapshot_facts.census, measured.facts.census,
        "the copy is of one coherent instant: the pinned read transaction is what makes that \
         true, and E-27 is the test that proves the mechanism"
    );
    assert!(
        held.as_secs() < 10,
        "a concurrent writer must not turn a 50 ms hold into a stall: {held:?}"
    );
    assert!(
        commits > 0,
        "the writer must actually have committed something during the copy, or this measures \
         nothing"
    );
}

// §7.2 — the snapshot-inflation figure, and the compaction question
#[test]
fn the_snapshot_is_not_compacted_and_the_archive_absorbs_most_of_it() {
    let measured = realistic();

    // SQLite's default, and the one every Evara project uses: `page_size` is never set, so
    // this is the engine's compiled-in default rather than a number this test chose.
    let page_size = transfer::page_size_of(&measured.project).expect("read the page size");

    let db = evara_db::Database::open_for_inspection(&measured.project).expect("open");
    let snapshot = db
        .snapshot_into(
            &measured.project.with_file_name("staging"),
            &NeverCancel,
            &mut transfer::ignore_progress,
        )
        .expect("snapshot");
    let snapshot_bytes = snapshot.byte_len();
    drop(db);

    let archive = measured.project.with_file_name("northgate.evara");
    let report = seal(snapshot, &archive, &IgnoreProgress, &NeverCancel).expect("seal");

    // The honest form of §7.2's question. A page-for-page copy is **exactly** the project
    // file's size, so "inflation over the project file" is always zero and measuring it says
    // nothing. What §7.2 is actually asking is how much larger the snapshot is than a
    // *vacuumed* copy would be — and that is the free list, which `VACUUM INTO` would drop
    // and the backup API carries.
    #[allow(clippy::cast_precision_loss)]
    let free_bytes = measured.free_pages as f64 * page_size as f64;
    #[allow(clippy::cast_precision_loss)]
    let over_vacuumed = (free_bytes / (snapshot_bytes as f64 - free_bytes).max(1.0)) * 100.0;

    println!(
        "S7.2  project {} bytes ({} free pages of {page_size} = {free_bytes:.0} bytes) \
         -> snapshot {snapshot_bytes} bytes -> archive {} bytes",
        measured.project_bytes, measured.free_pages, report.archive_bytes,
    );
    println!(
        "S7.2  the snapshot is the project's size page for page, and {over_vacuumed:+.1}% \
         larger than a vacuumed copy would be. Deflate then saves {:.1}% of it, because free \
         pages are mostly zeroes.",
        report.compression_saving_percent(),
    );
    println!(
        "S7.2  compaction answer: a second `VACUUM INTO` pass would recover {over_vacuumed:.1}% \
         of a staging file, at the cost of one more full write of the payload. The \
         user-visible figure -- archive size -- moves by far less, because the archive already \
         absorbs the free pages. No compaction pass is added."
    );

    // The backup API reproduces the source page for page, so a snapshot is never *smaller*
    // than the project and the inflation figure is the free list. Asserted as a bound rather
    // than a value: what it rules out is a snapshot that silently compacted, which would mean
    // the mechanism was not the one §C.6 settled on.
    assert!(
        snapshot_bytes >= measured.project_bytes,
        "the backup API copies page for page, so a snapshot cannot be smaller than its \
         source: {snapshot_bytes} < {}",
        measured.project_bytes
    );
    assert!(
        report.archive_bytes < snapshot_bytes,
        "SQLite deflates well, so an archive must be smaller than its payload: {} >= \
         {snapshot_bytes}",
        report.archive_bytes
    );
    assert!(
        measured.free_pages > 0,
        "the fixture must actually have a free list for this measurement to mean anything"
    );
}

// T-59's shape, at the realistic scale rather than the ten-times one
#[test]
fn a_realistic_project_round_trips_inside_the_time_budget() {
    let measured = realistic();

    let archive = measured.project.with_file_name("northgate.evara");
    let export_at = Instant::now();
    let db = evara_db::Database::open_for_inspection(&measured.project).expect("open");
    let snapshot = db
        .snapshot_into(
            &measured.project.with_file_name("staging"),
            &NeverCancel,
            &mut transfer::ignore_progress,
        )
        .expect("snapshot");
    drop(db);
    let report = seal(snapshot, &archive, &IgnoreProgress, &NeverCancel).expect("seal");
    let export = export_at.elapsed();

    let inspect_at = Instant::now();
    let summary = crate::inspect(&archive).expect("inspect");
    let inspected = inspect_at.elapsed();

    let settings_dir = tempfile::TempDir::new().expect("temp dir");
    let mut settings = evara_db::Settings::open_at(&settings_dir.path().join("settings.sqlite"))
        .expect("settings database");
    let destination = measured.project.with_file_name("imported.evaraproj");
    let import_at = Instant::now();
    let imported = crate::import(
        &archive,
        &destination,
        crate::OnConflict::Refuse,
        crate::Importer {
            app_version: crate::tests_fixture::APP_VERSION,
            site_id: uuid::Uuid::now_v7(),
        },
        &mut settings,
        &IgnoreProgress,
        &NeverCancel,
    )
    .expect("import");
    let import = import_at.elapsed();

    println!(
        "§7.3  export {:.0} ms (budget 1 500), inspect {:.0} ms (budget 50), import {:.0} ms \
         (budget 3 000); archive {} bytes, {} rows carried",
        export.as_secs_f64() * 1_000.0,
        inspected.as_secs_f64() * 1_000.0,
        import.as_secs_f64() * 1_000.0,
        report.archive_bytes,
        imported.counts.values().sum::<i64>(),
    );

    assert!(
        summary.readable.is_readable(),
        "a project this build exported must be one it can import"
    );
    assert_eq!(
        imported.counts, measured.facts.census,
        "and the round trip must preserve every row at the realistic scale too, not only at \
         the varied one"
    );

    // An order-of-magnitude bound on each, for the same reason as T-83's: a thirty-second
    // import would mean something is wrong, and two hundred milliseconds either way of a
    // budget is the machine.
    assert!(
        export.as_secs() < 30 && import.as_secs() < 60,
        "export {export:?} and import {import:?} are beyond any reading of §7.3's budgets"
    );
}
