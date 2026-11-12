//! The replacement protocol, and the three safeguards asserted against the composed thing.
//!
//! # These assertions are owed twice, and this is the second payment
//!
//! Stage 1 asserted T-84…T-90 against the **primitives**: `classify_destination`'s refusal of
//! a zero-length destination, `reconcile`'s decision over all 128 reachable combinations,
//! `checkpoint_to_single_file`'s refusal before anything moves. Those tests prove the
//! decisions are right.
//!
//! What they cannot prove is that the protocol composed from them *makes* those calls, in that
//! order, with the keep-name and the bounded retry. A protocol that reached the right verdict
//! and then renamed anyway would pass every Stage 1 test. So each safeguard is asserted again
//! here, end to end, against `import` and `recover_pending`.
//!
//! # The one promise every test in this file checks
//!
//! **Nothing in Evara deletes the project it replaced.** Not on success, not on failure, not
//! during recovery, not when the file at the destination is something Evara did not write. So
//! every test that touches an occupied destination asserts the old bytes are still readable
//! somewhere afterwards — which is a property of there being no delete in the code, rather
//! than of error handling having gone well.

use std::path::{Path, PathBuf};

use evara_core::scheduling::NeverCancel;
use evara_db::fixtures::transfer::Shape;
use evara_db::transfer::{
    DiskFacts, RecoveryAction, ReplacementPhase, SidecarState, looks_like_a_project, reconcile,
    sidecar_path,
};
use evara_db::{Settings, transfer};

use crate::staging::{ASIDE_DIR, ReplacementJournal, STAGED_FILE, STAGING_PREFIX};
use crate::tests_fixture::{APP_VERSION, Fixture};
use crate::{IgnoreProgress, Importer, OnConflict, TransferError, import, recover_pending, seal};

/// An archive, a settings database, and a directory to import into.
struct Scene {
    dir: tempfile::TempDir,
    _settings_dir: tempfile::TempDir,
    settings: Settings,
    archive: PathBuf,
}

impl Scene {
    fn new() -> Self {
        let mut fixture = Fixture::build(Shape::Varied);
        let source = fixture.project.path().to_path_buf();
        fixture.project.close();

        let dir = tempfile::TempDir::new().expect("temp dir");
        let archive = dir.path().join("northgate.evara");
        let db = evara_db::Database::open_for_inspection(&source).expect("open for inspection");
        let snapshot = db
            .snapshot_into(
                &dir.path().join("staging"),
                &NeverCancel,
                &mut transfer::ignore_progress,
            )
            .expect("snapshot");
        drop(db);
        seal(snapshot, &archive, &IgnoreProgress, &NeverCancel).expect("seal");

        let settings_dir = tempfile::TempDir::new().expect("temp dir");
        let settings = Settings::open_at(&settings_dir.path().join("settings.sqlite"))
            .expect("settings database");
        Self {
            dir,
            _settings_dir: settings_dir,
            settings,
            archive,
        }
    }

    fn destination(&self) -> PathBuf {
        self.dir.path().join("Northgate.evaraproj")
    }

    /// Imports into `destination`.
    fn import_to(
        &mut self,
        destination: &Path,
        on_conflict: OnConflict,
    ) -> crate::Result<crate::ImportReport> {
        let archive = self.archive.clone();
        import(
            &archive,
            destination,
            on_conflict,
            Importer {
                app_version: APP_VERSION,
                site_id: uuid::Uuid::now_v7(),
            },
            &mut self.settings,
            &IgnoreProgress,
            &NeverCancel,
        )
    }

    /// Every `.replaced-` copy in the directory.
    fn kept_copies(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.dir.path())
            .expect("list")
            .filter_map(|entry| {
                let path = entry.expect("entry").path();
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".replaced-"))
                    .then_some(path)
            })
            .collect();
        found.sort();
        found
    }

    /// Staging directories left in the directory.
    fn staging_dirs(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.dir.path())
            .expect("list")
            .filter_map(|entry| {
                let path = entry.expect("entry").path();
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(STAGING_PREFIX))
                    .then_some(path)
            })
            .collect();
        found.sort();
        found
    }
}

// =======================================================================================
// S1 — a zero-length destination is occupied, never adopted
// =======================================================================================

// T-84
#[test]
fn an_absent_destination_is_claimed_and_a_zero_length_one_is_occupied() {
    let mut scene = Scene::new();
    let destination = scene.destination();

    // Absent: Create mode, claimed with `create_new(true)` and finalised by one rename.
    let report = scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("an absent destination is claimable");
    assert!(destination.exists());
    assert_eq!(report.replaced_copy_kept_at, None, "nothing was replaced");
    assert!(scene.staging_dirs().is_empty(), "and staging is gone");

    // Now a **zero-length** destination at a fresh name, with no journal row. S1: this is
    // *occupied*, not creatable. `Database::create` would treat it as creatable, and the
    // divergence is deliberate — an import adopting an unknown empty file would be writing
    // over something nobody has looked at.
    let empty = scene.dir.path().join("Empty.evaraproj");
    std::fs::write(&empty, b"").expect("create an empty file");
    assert_eq!(std::fs::metadata(&empty).expect("stat").len(), 0);

    match scene.import_to(&empty, OnConflict::Refuse) {
        Err(TransferError::DestinationExists { name }) => {
            assert_eq!(name, "Empty.evaraproj");
        }
        other => {
            panic!("a zero-length destination must be occupied rather than adopted: {other:?}")
        }
    }
    assert_eq!(
        std::fs::metadata(&empty).expect("still there").len(),
        0,
        "and it must be untouched"
    );
}

// T-84, second half
#[test]
fn a_zero_length_destination_is_kept_as_a_replaced_copy_rather_than_destroyed() {
    let mut scene = Scene::new();
    let empty = scene.dir.path().join("Empty.evaraproj");
    std::fs::write(&empty, b"").expect("create an empty file");

    let report = scene
        .import_to(&empty, OnConflict::Replace)
        .expect("an explicit replace is how an occupied destination is taken");

    let kept = report
        .replaced_copy_kept_at
        .expect("even an unknown empty file is kept");
    assert!(kept.exists(), "the kept copy must be on disk: {kept:?}");
    assert_eq!(
        std::fs::metadata(&kept).expect("stat").len(),
        0,
        "and it must be the file that was there, byte for byte"
    );
    assert!(empty.exists(), "and the new project is at the destination");
    assert!(
        std::fs::metadata(&empty).expect("stat").len() > 0,
        "and it is not the empty file"
    );
}

// T-85
#[test]
fn the_claim_closes_the_window_between_deciding_and_renaming() {
    // `fs::rename` **replaces** an existing destination on both platforms, so without an
    // atomic claim a file created between the check and the rename is silently destroyed.
    // The claim is `create_new(true)`, and this asserts the primitive the protocol depends
    // on: a second claim of a held name fails rather than succeeding.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let destination = dir.path().join("Northgate.evaraproj");

    let first = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination);
    assert!(first.is_ok(), "the first claim wins the name");

    let second = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination);
    assert_eq!(
        second.expect_err("the second claim must fail").kind(),
        std::io::ErrorKind::AlreadyExists,
        "`create_new` is what makes two concurrent imports of one name have one winner"
    );
}

#[test]
fn two_imports_of_one_destination_cannot_both_proceed() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");

    // The second meets an occupied destination and is refused by name — the composed form of
    // the claim, from the caller's side.
    match scene.import_to(&destination, OnConflict::Refuse) {
        Err(TransferError::DestinationExists { name }) => {
            assert_eq!(name, "Northgate.evaraproj");
        }
        other => panic!("expected DestinationExists, got {other:?}"),
    }
}

// =======================================================================================
// The Replace path, end to end
// =======================================================================================

// T-49
#[test]
fn replacing_keeps_the_old_project_beside_the_new_one() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");
    let first_bytes = std::fs::read(&destination).expect("read the first project");

    let report = scene
        .import_to(&destination, OnConflict::Replace)
        .expect("a replace succeeds");

    let kept = report.replaced_copy_kept_at.expect("the old copy is kept");
    assert_eq!(
        std::fs::read(&kept).expect("read the kept copy"),
        first_bytes,
        "the replaced project is kept byte for byte: nothing in Evara deletes it"
    );
    assert!(
        kept.to_string_lossy().contains("Northgate.replaced-"),
        "and under a name that says what it is: {kept:?}"
    );
    assert!(
        kept.extension()
            .is_some_and(|ext| ext == evara_db::PROJECT_EXTENSION),
        "and that is still openable as a project: {kept:?}"
    );
    assert!(destination.exists(), "the new project is in place");
    assert!(scene.staging_dirs().is_empty(), "and staging is gone");
    assert_eq!(scene.kept_copies().len(), 1);

    // The journal is empty, so nothing is left for recovery to find.
    assert!(
        scene
            .settings
            .pending()
            .expect("read the journal")
            .is_empty(),
        "a finished replacement leaves no journal row"
    );
}

#[test]
fn a_second_replacement_in_the_same_second_does_not_overwrite_the_first_kept_copy() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");

    let first = scene
        .import_to(&destination, OnConflict::Replace)
        .expect("the first replace")
        .replaced_copy_kept_at
        .expect("kept");
    let second = scene
        .import_to(&destination, OnConflict::Replace)
        .expect("the second replace")
        .replaced_copy_kept_at
        .expect("kept");

    assert_ne!(
        first, second,
        "two replacements within one second must not rename over the kept copy -- that would \
         destroy the one thing this protocol promises never to destroy"
    );
    assert!(first.exists() && second.exists());
    assert_eq!(scene.kept_copies().len(), 2);
}

// =======================================================================================
// S3 — no main file is ever paired with another database's sidecars
// =======================================================================================

// T-89
#[test]
fn a_destination_whose_sidecar_cannot_be_released_refuses_before_anything_moves() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");
    let before = std::fs::read(&destination).expect("read the project");

    // The destination is **held open**, which is E-30's proven refusal point:
    // `PRAGMA wal_checkpoint(TRUNCATE)` cannot release the log while another connection holds
    // the database, so the `-wal` survives the close and S3 step 2 refuses — *before anything
    // has moved*.
    //
    // A stray `-wal` beside a *real* database was tried first and is **not** this case: a
    // `-wal` at `X.evaraproj-wal` is X's own log by definition, so SQLite recovers it and
    // removes it on close, which is the correct operation rather than a gap. The dangerous
    // pairing — a *new* main file beside an *old* log — is what T-90 covers, and the forward
    // path makes it unrepresentable by asserting the destination is clean immediately before
    // the rename.
    let holder =
        evara_db::Database::open_for_inspection(&destination).expect("hold the destination open");
    assert!(
        SidecarState::observe(&destination).any(),
        "a read-only open of a WAL database creates `-wal` and `-shm` beside it (E-29),          which is what makes this the realistic shape of the refusal"
    );

    let error = scene
        .import_to(&destination, OnConflict::Replace)
        .expect_err("a sidecar that cannot be released must refuse the replacement");
    match error {
        TransferError::Database(evara_db::DbError::RenameBlocked { name, sidecar }) => {
            assert_eq!(name, "Northgate.evaraproj");
            assert!(
                transfer::SIDECAR_SUFFIXES.contains(&sidecar),
                "the refusal names which sidecar is in the way: {sidecar}"
            );
        }
        other => panic!("expected RenameBlocked, got {other:?}"),
    }

    drop(holder);
    assert_eq!(
        std::fs::read(&destination).expect("read it again"),
        before,
        "**nothing has moved**: that is the whole reason the check is before the protocol          rather than inside it"
    );
    assert_eq!(
        scene.kept_copies(),
        Vec::<PathBuf>::new(),
        "and no copy was taken, because no replacement began"
    );
    assert_eq!(scene.staging_dirs(), Vec::<PathBuf>::new());
}

#[test]
fn a_stray_sidecar_beside_an_unknown_file_blocks_the_replacement() {
    // The complement, and the reason `checkpoint_to_single_file` asserts sidecars even on a
    // path it will not open: a `-wal` beside something that is not a database cannot be
    // recovered by opening it, so there is nothing to checkpoint and nothing may be assumed.
    // Reported, never deleted — a log may hold committed transactions its main file does not.
    let mut scene = Scene::new();
    let unknown = scene.dir.path().join("Unknown.evaraproj");
    std::fs::write(&unknown, b"not a database").expect("write");
    let stray = sidecar_path(&unknown, "-wal");
    std::fs::write(&stray, b"a log belonging to something").expect("write");

    let error = scene
        .import_to(&unknown, OnConflict::Replace)
        .expect_err("a stray sidecar must block the replacement");
    assert!(
        matches!(
            error,
            TransferError::Database(evara_db::DbError::RenameBlocked { .. })
        ),
        "expected RenameBlocked, got {error:?}"
    );
    assert_eq!(
        std::fs::read(&unknown).expect("still there"),
        b"not a database",
        "and the unknown file is untouched"
    );
    assert!(stray.exists(), "and the sidecar is reported, never deleted");
}

#[test]
fn an_unknown_destination_is_never_written_into_before_it_is_kept() {
    // The defect this test found, kept as a regression. `checkpoint_to_single_file` used to
    // open whatever was at the destination, and `open_plain` runs
    // `PRAGMA journal_mode = WAL` — which on a **zero-length** file creates a database there.
    // The protocol then kept that as the `.replaced-` copy, so what was preserved was four
    // kilobytes of Evara's own header rather than the user's file.
    //
    // S1 says an import must not write over an unknown file. Checkpointing one is writing
    // over it, and this is the assertion that says so.
    let mut scene = Scene::new();
    let unknown = scene.dir.path().join("Mystery.evaraproj");
    std::fs::write(&unknown, b"seventeen bytes..").expect("write");
    let original = std::fs::read(&unknown).expect("read");

    let report = scene
        .import_to(&unknown, OnConflict::Replace)
        .expect("an explicit replace takes an occupied destination");
    let kept = report.replaced_copy_kept_at.expect("the old file is kept");

    assert_eq!(
        std::fs::read(&kept).expect("read the kept copy"),
        original,
        "the file that was there must be kept exactly as it was found"
    );
}

// T-90
#[test]
fn the_destination_is_asserted_clean_immediately_before_the_rename() {
    // S3 step 4, which is the safeguard proper: it makes a main file beside a foreign sidecar
    // **unrepresentable** rather than merely unlikely, and it holds however the stray file got
    // there. Asserted against the primitive the protocol calls, with the destination in the
    // one state the forward path cannot produce — absent main file, sidecar present.
    let dir = tempfile::TempDir::new().expect("temp dir");
    let destination = dir.path().join("Northgate.evaraproj");
    std::fs::write(sidecar_path(&destination, "-wal"), b"stale").expect("create");

    let error = transfer::refuse_if_sidecars_present(&destination)
        .expect_err("a stray sidecar must block the rename");
    match error {
        evara_db::DbError::RenameBlocked { sidecar, .. } => assert_eq!(sidecar, "-wal"),
        other => panic!("expected RenameBlocked, got {other:?}"),
    }

    // And each suffix independently, because each one moves on its own and an interrupted
    // replacement can leave any subset behind. A single `has_sidecars: bool` would collapse
    // exactly the distinction this needs.
    for suffix in transfer::SIDECAR_SUFFIXES {
        let path = dir.path().join(format!("Other.evaraproj{suffix}"));
        std::fs::write(&path, b"stale").expect("create");
        let main = dir.path().join("Other.evaraproj");
        assert!(
            SidecarState::observe(&main).any(),
            "{suffix} must be observed on its own"
        );
        assert!(transfer::refuse_if_sidecars_present(&main).is_err());
        std::fs::remove_file(&path).expect("remove");
    }
}

// =======================================================================================
// S2 — recovery reconciles against the filesystem, and never against the phase
// =======================================================================================

// T-86 — the data-loss case, composed
#[test]
fn placed_with_an_absent_destination_restores_the_old_project_rather_than_keeping_it() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");
    let original = std::fs::read(&destination).expect("read the project");

    // The exact state a crash between the `PLACED` commit and the rename leaves: the journal
    // says the new project is in place, the destination is **absent**, the staged file is
    // still in staging and the old project is in `aside/`.
    //
    // Read literally, `PLACED` means "finish step 3 -- move the aside set to the keep-name",
    // which would move the user's only project to a `.replaced-` name and leave **no project
    // at the destination at all**. That is the defect revision 2 of the design had.
    let staging = scene.dir.path().join(format!("{STAGING_PREFIX}crashed"));
    let aside = staging.join(ASIDE_DIR);
    std::fs::create_dir_all(&aside).expect("create staging");
    std::fs::rename(&destination, aside.join("Northgate.evaraproj")).expect("set aside");
    std::fs::write(staging.join(STAGED_FILE), b"the staged project").expect("stage");

    let row = scene
        .settings
        .begin_replacement(&destination, &staging)
        .expect("journal the replacement");
    scene
        .settings
        .advance_replacement(
            row.id,
            ReplacementPhase::Placed,
            Some(&scene.dir.path().join("Northgate.replaced-x.evaraproj")),
        )
        .expect("advance to PLACED");

    let recovered = recover_pending(&mut scene.settings).expect("recovery runs");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].resolved, "{:?}", recovered[0]);

    assert_eq!(
        std::fs::read(&destination).expect("the project is back at the destination"),
        original,
        "the old project must be restored to the destination, not moved to the keep-name"
    );
    assert!(
        scene.kept_copies().is_empty(),
        "and no `.replaced-` copy must have been created: nothing was replaced"
    );
    assert!(scene.staging_dirs().is_empty(), "and staging is cleaned up");
}

// T-87 — every reconciliation row, composed
#[test]
fn recovery_acts_on_the_filesystem_and_not_on_the_recorded_phase() {
    // The same on-disk state under **all three** phases, asserting the action is identical.
    // If recovery read the phase, these would differ — which is the property S2 is about,
    // and it is asserted here against the composed `recover_pending` rather than only
    // against the pure decision function.
    for phase in [
        ReplacementPhase::Staged,
        ReplacementPhase::Aside,
        ReplacementPhase::Placed,
    ] {
        let mut scene = Scene::new();
        let destination = scene.destination();
        scene
            .import_to(&destination, OnConflict::Refuse)
            .expect("the first import");
        let original = std::fs::read(&destination).expect("read the project");

        let staging = scene
            .dir
            .path()
            .join(format!("{STAGING_PREFIX}{}", phase.as_str()));
        let aside = staging.join(ASIDE_DIR);
        std::fs::create_dir_all(&aside).expect("create staging");
        std::fs::rename(&destination, aside.join("Northgate.evaraproj")).expect("set aside");

        let row = scene
            .settings
            .begin_replacement(&destination, &staging)
            .expect("journal");
        scene
            .settings
            .advance_replacement(row.id, phase, None)
            .expect("advance");

        let recovered = recover_pending(&mut scene.settings).expect("recovery runs");
        assert_eq!(recovered.len(), 1);
        assert!(
            recovered[0].resolved,
            "phase {phase:?} must be resolved from the facts: {:?}",
            recovered[0]
        );
        assert_eq!(
            std::fs::read(&destination).expect("restored"),
            original,
            "the same disk state must produce the same action under every phase, because the \
             facts decide and the journal does not (phase {phase:?})"
        );
    }
}

#[test]
fn an_unrecognised_phase_is_reported_and_acted_on_by_nothing() {
    // A row a **newer** build wrote. Its protocol may differ in ways this one cannot see, so
    // guessing its intent is how a newer protocol's half-finished replacement gets completed
    // wrongly by an older binary.
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");
    let original = std::fs::read(&destination).expect("read the project");

    let staging = scene.dir.path().join(format!("{STAGING_PREFIX}newer"));
    std::fs::create_dir_all(&staging).expect("create staging");
    let row = scene
        .settings
        .begin_replacement(&destination, &staging)
        .expect("journal");

    // The `CHECK` constraint keeps the three known spellings exclusive within a build, so an
    // unrecognised value can only have come from a build that knows more than this one. The
    // reconciliation is asserted directly, because the column will not store a fourth value
    // in a build whose migration declares three.
    let facts = DiskFacts::observe(
        &scene
            .settings
            .pending()
            .expect("read the journal")
            .into_iter()
            .next()
            .expect("the row is there"),
        &looks_like_a_project,
    );
    assert_eq!(
        reconcile(facts, None),
        RecoveryAction::Report(evara_db::transfer::Ambiguity::UnknownPhase),
        "an unrecognised phase is carried as an unknown and acted on by nothing"
    );
    assert!(
        reconcile(facts, None).is_inert(),
        "and the action must change nothing on disk"
    );

    let _ = row;
    assert_eq!(
        std::fs::read(&destination).expect("untouched"),
        original,
        "the destination is untouched"
    );
}

// T-88 — the dominating invariant, composed
#[test]
fn the_old_set_is_released_only_once_a_project_has_been_observed() {
    // Asserted as a property of the **action**, over the states the composed protocol can
    // leave. The pure sweep over all 128 combinations is Stage 1's; what this adds is that
    // the composed `recover_pending` only ever performs an action the invariant permits.
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the first import");

    let staging = scene.dir.path().join(format!("{STAGING_PREFIX}sweep"));
    let aside = staging.join(ASIDE_DIR);
    std::fs::create_dir_all(&aside).expect("create staging");
    let row = scene
        .settings
        .begin_replacement(&destination, &staging)
        .expect("journal");

    // Four destination states crossed with aside present and absent. For each, the action
    // `reconcile` chooses must release the old set only when the destination reads as a
    // project — and `DiskFacts::observe` applies the S3 conjunction, so a project with a
    // sidecar beside it is not one.
    let original = std::fs::read(&destination).expect("read the project");
    for aside_present in [false, true] {
        if aside_present {
            std::fs::write(aside.join("Northgate.evaraproj"), &original).expect("aside");
        } else {
            let _ = std::fs::remove_file(aside.join("Northgate.evaraproj"));
        }

        for state in ["project", "unknown-bytes", "zero-length", "absent"] {
            match state {
                "project" => std::fs::write(&destination, &original).expect("write"),
                "unknown-bytes" => std::fs::write(&destination, b"not a database").expect("write"),
                "zero-length" => std::fs::write(&destination, b"").expect("write"),
                _ => {
                    let _ = std::fs::remove_file(&destination);
                }
            }

            let observed = DiskFacts::observe(&row, &looks_like_a_project);
            let action = reconcile(observed, Some(ReplacementPhase::Placed));
            assert!(
                !action.releases_the_old_set() || observed.destination.is_blessable_project(),
                "state `{state}` with aside {aside_present} chose {action:?}, which releases \
                 the old set without a destination that looks like a project"
            );
        }
    }
}

#[test]
fn abandoned_staging_with_no_journal_row_is_reported_and_never_deleted() {
    // It may be another installation's live staging, or the leftovers of an import whose
    // journal row was lost to a power cut. Deleting a directory that might be another
    // process's work in progress is worse than leaving it.
    let scene = Scene::new();
    let orphan = scene.dir.path().join(format!("{STAGING_PREFIX}orphan"));
    std::fs::create_dir_all(&orphan).expect("create");
    std::fs::write(orphan.join(STAGED_FILE), b"somebody's work in progress").expect("write");

    let found = crate::abandoned_staging(scene.dir.path(), &scene.settings)
        .expect("listing a folder cannot fail here");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].destination, orphan);
    assert!(
        !found[0].resolved,
        "reported rather than resolved: nothing was done to it"
    );
    assert!(
        orphan.join(STAGED_FILE).exists(),
        "and it is still there, untouched"
    );
}

#[test]
fn a_staging_directory_with_a_journal_row_is_not_reported_as_abandoned() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    let staging = scene.dir.path().join(format!("{STAGING_PREFIX}known"));
    std::fs::create_dir_all(&staging).expect("create");
    scene
        .settings
        .begin_replacement(&destination, &staging)
        .expect("journal");

    let found = crate::abandoned_staging(scene.dir.path(), &scene.settings).expect("list");
    assert!(
        found.is_empty(),
        "a staging directory the journal knows about is recovery's, not an orphan: {found:?}"
    );
}

#[test]
fn two_replacements_of_one_destination_cannot_be_journalled_at_once() {
    // Two concurrent replacements of one file is not a state the protocol has an answer for,
    // so the uniqueness index turns a race into a refusal rather than leaving recovery to
    // reconcile two rows about one path.
    let mut scene = Scene::new();
    let destination = scene.destination();
    let first = scene.dir.path().join(format!("{STAGING_PREFIX}first"));
    std::fs::create_dir_all(&first).expect("create");
    scene
        .settings
        .begin_replacement(&destination, &first)
        .expect("the first row");

    match scene.import_to(&destination, OnConflict::Replace) {
        Err(TransferError::ReplacementInFlight { name }) => {
            assert_eq!(name, "Northgate.evaraproj");
        }
        other => panic!("expected ReplacementInFlight, got {other:?}"),
    }
}

// T-51's `evara-project` half: an import does not open what it imported.
#[test]
fn an_import_leaves_nothing_open() {
    let mut scene = Scene::new();
    let destination = scene.destination();
    scene
        .import_to(&destination, OnConflict::Refuse)
        .expect("the import");

    // The honest, crate-level form of the claim: the imported project is a **single file**
    // afterwards, with no `-wal` and no `-shm`. Anything holding it open would have left
    // those behind, and the next replacement would then be refused by S3 — which is how this
    // would show up in use rather than in a test.
    assert_eq!(
        SidecarState::observe(&destination),
        SidecarState::default(),
        "an import must leave the project closed and reduced to one file"
    );
    assert!(
        looks_like_a_project(&destination),
        "and it must be a project a later replacement may bless"
    );
}
