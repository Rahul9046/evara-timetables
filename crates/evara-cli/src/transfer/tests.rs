//! In-crate tests for the transfer commands.
//!
//! # Why `import` is tested here and not against the binary
//!
//! `crates/evara-cli/tests/transfer_cli.rs` drives the **real executable** for `export` and
//! `inspect`, which is the better test of a command: it covers argument parsing, the exit
//! code and the output a person actually sees.
//!
//! `import` cannot be tested that way, and the reason is worth writing down rather than
//! leaving as an apparent gap. A spawned binary is compiled **without** `cfg(test)`, so its
//! `open_settings` is the production one — `Settings::open_default()` — which resolves
//! `directories::ProjectDirs` and opens **the developer's own** settings database. That file
//! has a forward-only refinery chain, so a test that reached it would migrate the
//! installation as a side effect of `cargo test` and leave every older build unable to open
//! its settings. PR #39 fixed exactly that defect for `save`.
//!
//! Redirecting it from outside does not work portably either: `directories` reads `$HOME` on
//! macOS but asks the Windows known-folder API on Windows, so there is no environment
//! variable to set for the platform Evara also ships on. And adding a `--settings` flag to
//! the shipped command would be a production argument whose only purpose is testing, which
//! is the thing the brief forbids.
//!
//! So `import` is driven through [`super::import_with`] — the real body, the real
//! `evara_project::import`, the real staging and replacement protocol — with the settings
//! database pointed at a `TempDir` through the same seam `save.rs` uses. Nothing about the
//! protocol is mocked.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{Cancel, NeverCancel, ProblemIdentity, SolveStatus};
use evara_db::fixtures::transfer::Shape;
use evara_db::transfer::{facts_of, table_census};
use evara_db::{RUN_GENERATION, RunProvenance};
use evara_solver::{GenerateRequest, SolveConfig, generate};

use super::{ExportRequest, ImportRequest, Outcome, export_with, import_with, is_refusal};

// ---------------------------------------------------------------------------------------
// the settings seam
// ---------------------------------------------------------------------------------------

thread_local! {
    /// The settings database this thread's test named.
    ///
    /// Thread-local rather than a global, because `cargo test` runs tests in parallel and a
    /// shared cell would hand one test another's path.
    static SETTINGS: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Points this thread's `import` at `path` instead of the installation's settings database.
pub(super) fn use_settings_at(path: &Path) {
    SETTINGS.with(|cell| *cell.borrow_mut() = Some(path.to_path_buf()));
}

/// What this thread's test named, if it named one.
pub(super) fn settings_override() -> Option<PathBuf> {
    SETTINGS.with(|cell| cell.borrow().clone())
}

/// A `Cancel` that is already cancelled.
struct Cancelled;

impl Cancel for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}

/// A `Cancel` that turns on after `after` questions.
struct CancelAfter {
    seen: std::sync::atomic::AtomicUsize,
    after: usize,
}

impl CancelAfter {
    const fn new(after: usize) -> Self {
        Self {
            seen: std::sync::atomic::AtomicUsize::new(0),
            after,
        }
    }
}

impl Cancel for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.seen.fetch_add(1, Ordering::Relaxed) >= self.after
    }
}

/// A built project, an archive made from it, and a place to put things.
struct Scene {
    dir: tempfile::TempDir,
    project: PathBuf,
    archive: PathBuf,
    /// The source project's facts, read before the export.
    source: evara_db::transfer::ProjectFacts,
}

impl Scene {
    /// Builds a realistic project and exports it through the CLI's own `export`.
    fn new() -> Self {
        Self::of_shape(Shape::Varied)
    }

    fn of_shape(shape: Shape) -> Self {
        Self::built(shape, false)
    }

    /// A scene whose project carries a real generated timetable and a lock.
    ///
    /// `evara-db`'s fixtures build the *school*; the timetable layer is
    /// `evara-project`'s own test fixture, which solves and saves through the real
    /// repository. That fixture cannot be shared: making it a feature would give
    /// `evara-project` an optional `evara-solver` dependency, and a crate that owns a file
    /// format must not depend on search — its own manifest says so. So the one timetable this
    /// test needs is built here, minimally.
    ///
    /// **Repair lineage is not built here.** A repair needs the repair engine, a movable set
    /// and a second save, and duplicating that would be a third copy of a fixture for no
    /// additional *CLI* coverage: `evara-project`'s `lineage_and_locks_survive_and_are_declared`
    /// already proves lineage travels, through the identical `seal` and `import` calls these
    /// commands make. What is asserted here is that the CLI path carries the timetable tables
    /// and their flags at all.
    fn with_timetable(shape: Shape) -> Self {
        Self::built(shape, true)
    }

    fn built(shape: Shape, timetable: bool) -> Self {
        let (mut fixture, built) = evara_db::fixtures::transfer::build(shape);

        if timetable {
            let selection = built.main.selection();
            let assembly = fixture
                .load(&selection)
                .expect("the fixture school assembles");
            let outcome = generate(&GenerateRequest {
                problem: &assembly.problem,
                identity: ProblemIdentity {
                    school: assembly.problem.context.school.id,
                    cycle: selection.cycle_id,
                    term: selection.term_id,
                },
                constraints: &Registry::mandatory(),
                preflight: evara_db::AssemblyReport::of(&assembly).findings.clone(),
                may_solve: evara_db::AssemblyReport::of(&assembly).may_solve(),
                unenforced_restrictions: 0,
                config: SolveConfig::seeded(2026),
            });
            assert_ne!(
                outcome.status,
                SolveStatus::Failed,
                "the fixture school must be solvable for a timetable to exist to transfer"
            );
            let violations = evaluate(
                &assembly.problem,
                &outcome.assignment,
                &Registry::mandatory(),
            )
            .violations;
            let saved = fixture
                .db()
                .timetables()
                .save_generated(
                    &assembly,
                    &outcome,
                    &violations,
                    &RunProvenance {
                        engine: String::from("construction"),
                        settings: String::from("{}"),
                        started_at: String::from("2026-11-12T09:00:00.000Z"),
                        finished_at: String::from("2026-11-12T09:00:01.000Z"),
                        kind: String::from(RUN_GENERATION),
                        scope: String::from("{}"),
                    },
                    "Draft",
                )
                .expect("save the generated timetable");
            assert!(
                saved.entries > 0,
                "a saved timetable with no entries would make the assertion vacuous"
            );
        }

        let source_path = fixture.path().to_path_buf();
        fixture.close();

        // Copied out of the fixture's own directory, which goes away with `fixture`.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let project = dir.path().join("northgate.evaraproj");
        std::fs::copy(&source_path, &project).expect("copy the project somewhere we own");
        drop(fixture);

        let source = facts_of(&project).expect("read the source project's facts");
        let archive = dir.path().join("northgate.evara");

        let request = ExportRequest {
            project: project.clone(),
            to: archive.clone(),
            quiet: true,
        };
        assert_eq!(
            export_with(&request, &NeverCancel).expect("export"),
            Outcome::Done
        );

        Self {
            dir,
            project,
            archive,
            source,
        }
    }

    /// A path inside the scene's directory.
    fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// An isolated settings database for this scene, registered for this thread.
    fn use_own_settings(&self) {
        use_settings_at(&self.dir.path().join("settings.sqlite"));
    }

    /// Imports into `destination`.
    fn import_into(&self, destination: &Path, replace: bool) -> Result<Outcome, String> {
        self.import_cancelling(destination, replace, &NeverCancel)
    }

    fn import_cancelling(
        &self,
        destination: &Path,
        replace: bool,
        cancel: &dyn Cancel,
    ) -> Result<Outcome, String> {
        self.use_own_settings();
        import_with(
            &ImportRequest {
                archive: self.archive.clone(),
                to: destination.to_path_buf(),
                replace,
                quiet: true,
            },
            cancel,
        )
    }
}

// ---------------------------------------------------------------------------------------
// 1-5: export, inspect, import, and that everything arrived
// ---------------------------------------------------------------------------------------

#[test]
fn a_realistic_project_exports_inspects_and_imports_with_every_row_intact() {
    let scene = Scene::new();

    // The archive the CLI wrote is readable by the Stage 2 reader, by name.
    let summary = evara_project::inspect(&scene.archive).expect("inspect the archive");
    assert!(summary.readable.is_readable());
    assert_eq!(
        summary.manifest.project_id,
        scene.source.identity.project_id
    );

    let destination = scene.at("imported.evaraproj");
    assert_eq!(
        scene.import_into(&destination, false).expect("import"),
        Outcome::Done
    );

    // The census is discovered from `sqlite_master`, so a table a future migration adds is
    // covered by discovery rather than by somebody remembering to extend this list.
    let after = table_census(&destination).expect("census the imported project");
    assert_eq!(
        after, scene.source.census,
        "a transfer must preserve every row of every table the schema declares"
    );
    assert!(
        after.values().sum::<i64>() > 100,
        "the fixture must actually hold something: {after:?}"
    );

    // Identity, and the provenance that must not be rewritten by whoever sent it on.
    let imported = facts_of(&destination).expect("read the imported facts");
    assert_eq!(
        imported.identity.project_id, scene.source.identity.project_id,
        "a transferred project is the same project"
    );
    assert_eq!(
        imported.identity.created_by_site_id, scene.source.identity.created_by_site_id,
        "the importing installation must not become the creator"
    );

    // The feed travels complete, and its watermark with it: every stored `inputs_watermark`
    // points into this sequence.
    assert_eq!(imported.change_log.entries, scene.source.change_log.entries);
    assert_eq!(
        imported.change_log.watermark,
        scene.source.change_log.watermark
    );
}

#[test]
fn timetable_history_locks_and_repair_lineage_travel() {
    // Neither `Varied` nor `Realistic` carries a timetable: `evara-db`'s fixtures build the
    // school, and the timetable layer belongs to `evara-project`'s test fixture, which runs
    // the real solver. Both shapes come back with `timetable: 0`, so a test asserting the
    // history travels has to build one — `Scene::with_timetable` says why it is built here
    // rather than shared.
    let scene = Scene::with_timetable(Shape::Varied);
    let destination = scene.at("imported.evaraproj");
    scene.import_into(&destination, false).expect("import");

    let after = table_census(&destination).expect("census");
    for table in [
        "timetable",
        "timetable_entry",
        "timetable_evaluation",
        "solve_run",
    ] {
        let carried = after.get(table).copied().unwrap_or_default();
        assert!(
            carried > 0,
            "{table} must arrive populated, or the history did not travel: {after:?}"
        );
        assert_eq!(
            carried,
            scene.source.census.get(table).copied().unwrap_or_default(),
            "{table} lost or gained rows in transfer"
        );
    }

    // And the payload declared what it carries, derived from the data rather than
    // hand-maintained (D3) — so a reader that could not evaluate a rule would refuse rather
    // than drop rows quietly.
    assert!(
        !scene.source.features.is_empty(),
        "a project with a timetable must declare the features that describe it"
    );
}

// ---------------------------------------------------------------------------------------
// 6: malformed and tampered archives
// ---------------------------------------------------------------------------------------

#[test]
fn a_truncated_archive_is_refused_and_writes_no_project() {
    let scene = Scene::new();

    // Half an archive: the central directory is gone, so the reader cannot even enumerate
    // entries. A deterministic refusal, and the shape a part-downloaded file actually has.
    let bytes = std::fs::read(&scene.archive).expect("read the archive");
    let truncated = scene.at("truncated.evara");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).expect("write a half archive");

    scene.use_own_settings();
    let destination = scene.at("from-truncated.evaraproj");
    let outcome = import_with(
        &ImportRequest {
            archive: truncated,
            to: destination.clone(),
            replace: false,
            quiet: true,
        },
        &NeverCancel,
    )
    .expect("a damaged archive is a refusal, not a command failure");

    assert_eq!(outcome, Outcome::Refused);
    assert!(
        !destination.exists(),
        "a refused import must leave no project behind"
    );
}

#[test]
fn a_tampered_payload_never_produces_a_project() {
    let scene = Scene::new();

    // One byte flipped in the middle of the container, which lands in the payload's deflate
    // stream. **Either answer is acceptable here and the test says why rather than pinning
    // one:** if the digest is reached first it is a `ChecksumMismatch` and so a refusal, and
    // if the decoder fails first `evara-project` reports `Io`, which this CLI classifies as a
    // failure because it cannot tell a corrupt stream from a failing disk — they arrive as the
    // same variant. Distinguishing them needs a new error in `evara-project`, which is Stage
    // 2's library and not Stage 3's to widen.
    //
    // What must hold either way, and is the thing worth asserting: no project appears.
    let mut bytes = std::fs::read(&scene.archive).expect("read the archive");
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 0xFF;
    let tampered = scene.at("tampered.evara");
    std::fs::write(&tampered, &bytes).expect("write the tampered archive");

    scene.use_own_settings();
    let destination = scene.at("from-tampered.evaraproj");
    let result = import_with(
        &ImportRequest {
            archive: tampered,
            to: destination.clone(),
            replace: false,
            quiet: true,
        },
        &NeverCancel,
    );

    if let Ok(outcome) = result {
        assert_eq!(
            outcome,
            Outcome::Refused,
            "a tampered archive must never import"
        );
    }
    assert!(
        !destination.exists(),
        "a tampered archive must leave no project behind"
    );
}

#[test]
fn bytes_that_are_not_an_archive_are_refused() {
    let scene = Scene::new();
    let nonsense = scene.at("nonsense.evara");
    std::fs::write(&nonsense, b"this is not a zip file").expect("write");

    scene.use_own_settings();
    let destination = scene.at("from-nonsense.evaraproj");
    let outcome = import_with(
        &ImportRequest {
            archive: nonsense,
            to: destination.clone(),
            replace: false,
            quiet: true,
        },
        &NeverCancel,
    )
    .expect("not-an-archive is a refusal");
    assert_eq!(outcome, Outcome::Refused);
    assert!(!destination.exists());
}

// ---------------------------------------------------------------------------------------
// 7: unsafe destinations
// ---------------------------------------------------------------------------------------

#[test]
fn an_unsafe_destination_is_refused_by_the_library_and_reported_as_a_refusal() {
    let scene = Scene::new();

    // `file:` is the form the bundled engine would read as a URI whatever open flags are
    // passed — finding F-2 — so it is refused on the way in rather than opened later with
    // parameters nobody chose.
    for unsafe_destination in ["file:somewhere.evaraproj", "../escaped.evaraproj"] {
        scene.use_own_settings();
        let outcome = import_with(
            &ImportRequest {
                archive: scene.archive.clone(),
                to: PathBuf::from(unsafe_destination),
                replace: false,
                quiet: true,
            },
            &NeverCancel,
        )
        .expect("an unsafe path is a refusal the command correctly produced");
        assert_eq!(
            outcome,
            Outcome::Refused,
            "{unsafe_destination} must be refused"
        );
    }
}

#[test]
fn a_destination_without_the_project_extension_is_refused() {
    let scene = Scene::new();
    let outcome = scene
        .import_into(&scene.at("wrong.sqlite"), false)
        .expect("a wrong extension is a refusal");
    assert_eq!(outcome, Outcome::Refused);
}

// ---------------------------------------------------------------------------------------
// 8-9: existing destinations, explicit replacement, kept backups
// ---------------------------------------------------------------------------------------

#[test]
fn an_existing_destination_is_refused_without_replace() {
    let scene = Scene::new();
    let destination = scene.at("occupied.evaraproj");
    scene
        .import_into(&destination, false)
        .expect("first import");

    let outcome = scene
        .import_into(&destination, false)
        .expect("an occupied destination is a refusal, not a failure");
    assert_eq!(
        outcome,
        Outcome::Refused,
        "replacement must need explicit intent"
    );
}

#[test]
fn replace_keeps_the_project_it_replaced() {
    let scene = Scene::new();
    let destination = scene.at("occupied.evaraproj");
    scene.import_into(&destination, false).expect("first");

    // Something recognisable in the file being replaced, so the kept copy can be proven to be
    // that file rather than a fresh one.
    let before = std::fs::read(&destination).expect("read the first import");

    assert_eq!(
        scene.import_into(&destination, true).expect("replace"),
        Outcome::Done
    );

    // `<stem>.replaced-<stamp>.evaraproj`, beside the destination, and never deleted by any
    // code path — D11 as amended by the second review's safeguards.
    let kept: Vec<PathBuf> = std::fs::read_dir(scene.dir.path())
        .expect("list the directory")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".replaced-"))
        })
        .collect();

    assert_eq!(
        kept.len(),
        1,
        "exactly one replaced copy must be kept, found {kept:?}"
    );
    assert_eq!(
        std::fs::read(&kept[0]).expect("read the kept copy"),
        before,
        "the kept copy must be the bytes that were replaced, not a re-export"
    );
    assert!(destination.exists(), "and the new project is in place");
}

// ---------------------------------------------------------------------------------------
// 10: cancellation, and what an interrupted operation leaves
// ---------------------------------------------------------------------------------------

#[test]
fn a_cancelled_import_leaves_no_project() {
    let scene = Scene::new();
    let destination = scene.at("cancelled.evaraproj");

    let outcome = scene.import_cancelling(&destination, false, &Cancelled);
    assert!(
        outcome.is_err(),
        "a cancellation is the command not completing, so it is exit 2 rather than a refusal"
    );
    assert!(
        !destination.exists(),
        "a cancelled import must leave nothing at the destination"
    );
}

#[test]
fn a_cancellation_partway_through_still_leaves_no_project() {
    let scene = Scene::new();
    let destination = scene.at("cancelled-late.evaraproj");

    // Cancellation observed after the archive has been opened and some work done, rather than
    // at the first question — the phase an "it left half a project" bug would live in.
    let outcome = scene.import_cancelling(&destination, false, &CancelAfter::new(3));
    assert!(outcome.is_err());
    assert!(
        !destination.exists(),
        "no partial project at the destination"
    );
    // And no staging litter beside it.
    let strays: Vec<_> = std::fs::read_dir(scene.dir.path())
        .expect("list")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("cancelled-late"))
        .collect();
    assert!(
        strays.is_empty(),
        "a cancelled import left files behind: {strays:?}"
    );
}

#[test]
fn a_cancelled_export_writes_no_archive() {
    let scene = Scene::new();
    let destination = scene.at("cancelled.evara");

    let request = ExportRequest {
        project: scene.project.clone(),
        to: destination.clone(),
        quiet: true,
    };
    let outcome = export_with(&request, &Cancelled);
    assert!(outcome.is_err(), "a cancelled export does not complete");
    assert!(
        !destination.exists(),
        "and leaves no archive — the writer stages to a `.part` and renames"
    );
    assert!(
        !scene.at("cancelled.evara.part").exists(),
        "nor a partial one"
    );
}

// ---------------------------------------------------------------------------------------
// 11: exit codes and the refusal/failure split
// ---------------------------------------------------------------------------------------

#[test]
fn every_archive_and_destination_judgement_is_a_refusal_and_every_machine_failure_is_not() {
    use evara_project::TransferError;

    // The two ends of the classification, named individually so a new variant added without a
    // decision shows up as a compile error in `is_refusal` rather than as a silent exit 2.
    assert!(is_refusal(&TransferError::Truncated));
    assert!(is_refusal(&TransferError::UnsupportedCapability {
        capability: "cycle-kind:lunar".to_owned()
    }));
    assert!(is_refusal(&TransferError::FormatTooNew {
        found: 9,
        supported: 1
    }));
    assert!(is_refusal(&TransferError::DestinationExists {
        name: "x.evaraproj".to_owned()
    }));
    assert!(is_refusal(&TransferError::Database(
        evara_db::DbError::SchemaTooNew {
            found: 99,
            supported: 18
        }
    )));

    assert!(
        !is_refusal(&TransferError::Cancelled),
        "a cancellation is not a reasoned no"
    );
    assert!(!is_refusal(&TransferError::RenameBlocked {
        name: "x".to_owned()
    }));
    assert!(!is_refusal(&TransferError::Database(
        evara_db::DbError::internal("something broke")
    )));
}

#[test]
fn an_export_refuses_an_occupied_archive_rather_than_overwriting_it() {
    let scene = Scene::new();

    // The archive from `Scene::new` is already there. Exporting over it must refuse: an
    // archive is a document the user may still need, and `seal` itself renames over whatever
    // is at the destination, so this guard is the CLI's.
    let before = std::fs::read(&scene.archive).expect("read");
    let request = ExportRequest {
        project: scene.project.clone(),
        to: scene.archive.clone(),
        quiet: true,
    };
    assert_eq!(
        export_with(&request, &NeverCancel).expect("occupied is a refusal"),
        Outcome::Refused
    );
    assert_eq!(
        std::fs::read(&scene.archive).expect("read again"),
        before,
        "the existing archive must be untouched"
    );
}

#[test]
fn an_export_of_a_missing_project_is_a_failure_rather_than_a_refusal() {
    let scene = Scene::new();
    let request = ExportRequest {
        project: scene.at("not-there.evaraproj"),
        to: scene.at("out.evara"),
        quiet: true,
    };
    assert!(
        export_with(&request, &NeverCancel).is_err(),
        "nothing to export is the command unable to do its job"
    );
}

#[test]
fn an_export_leaves_the_source_project_byte_for_byte_unchanged() {
    let scene = Scene::new();
    let before = std::fs::read(&scene.project).expect("read the source");

    let request = ExportRequest {
        project: scene.project.clone(),
        to: scene.at("second.evara"),
        quiet: true,
    };
    export_with(&request, &NeverCancel).expect("export again");

    assert_eq!(
        std::fs::read(&scene.project).expect("read the source again"),
        before,
        "an export opens the project read-only and must not migrate, stamp or checkpoint it"
    );
    // And no sidecar was left beside it, which an `open_existing` would have created.
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", scene.project.display()));
        assert!(!sidecar.exists(), "an export created {sidecar:?}");
    }
}

#[test]
fn an_export_leaves_no_staging_directory_behind() {
    let scene = Scene::new();
    let strays: Vec<_> = std::fs::read_dir(scene.dir.path())
        .expect("list")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".evara-export-"))
        .collect();
    assert!(
        strays.is_empty(),
        "the snapshot staging directory must go away with the command: {strays:?}"
    );
}

// ---------------------------------------------------------------------------------------
// 12: settings isolation, and recovery
// ---------------------------------------------------------------------------------------

#[test]
fn an_import_writes_only_the_settings_database_the_test_named() {
    let scene = Scene::new();
    let settings = scene.at("settings.sqlite");
    assert!(
        !settings.exists(),
        "the scene has not opened its settings database yet"
    );

    scene
        .import_into(&scene.at("isolated.evaraproj"), false)
        .expect("import");

    assert!(
        settings.exists(),
        "the import must have used the settings database the test named"
    );
}

#[test]
fn a_pending_replacement_is_recovered_before_the_next_import() {
    let scene = Scene::new();
    let destination = scene.at("recovered.evaraproj");
    let settings_path = scene.at("settings.sqlite");

    // A journal row with a **staged payload** and no destination: the shape a crash after
    // staging and before the rename leaves. `reconcile` answers that one
    // `RollBackDeletingStaging` — nothing was changed, so nothing is restored and the row is
    // cleared.
    //
    // Deliberately *not* the staging-directory-only state: `(Absent, false, false)` is
    // `Report(NothingRemains)`, which recovery leaves on the journal on purpose — "not
    // recoverable and not ignorable: the user is told, and nothing is invented". Asserting
    // that one gets cleared would be asserting against the design.
    {
        let mut settings = evara_db::Settings::open_at(&settings_path).expect("settings");
        let staging = scene.at(".evara-import-abandoned");
        std::fs::create_dir_all(&staging).expect("create the staging directory");
        std::fs::write(
            staging.join(evara_project::staging::STAGED_FILE),
            b"a staged payload that never arrived",
        )
        .expect("write a staged payload");
        evara_project::ReplacementJournal::begin(&mut settings, &destination, &staging)
            .expect("journal a replacement that never finished");
        assert_eq!(
            settings.pending_replacements().expect("read pending").len(),
            1
        );
    }

    // The import runs recovery first, so the row is dealt with on the way in.
    scene.import_into(&destination, false).expect("import");

    let settings = evara_db::Settings::open_at(&settings_path).expect("reopen settings");
    assert!(
        settings
            .pending_replacements()
            .expect("read pending")
            .is_empty(),
        "the interrupted replacement must be resolved rather than left for ever"
    );
    assert!(destination.exists(), "and the import still happened");
}

// ---------------------------------------------------------------------------------------
// 13: argument parsing, including the platform path shapes
// ---------------------------------------------------------------------------------------

#[test]
fn export_parses_its_arguments_and_rejects_the_rest() {
    let parse = |args: &[&str]| {
        ExportRequest::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    };

    let request = parse(&["--project", "a.evaraproj", "--to", "b.evara"])
        .expect("parses")
        .expect("not help");
    assert_eq!(request.project, PathBuf::from("a.evaraproj"));
    assert_eq!(request.to, PathBuf::from("b.evara"));
    assert!(!request.quiet);

    assert!(
        parse(&["--help"]).expect("help parses").is_none(),
        "--help asks for the usage text"
    );
    assert!(parse(&["--project", "a.evaraproj"]).is_err(), "needs --to");
    assert!(parse(&["--to", "b.evara"]).is_err(), "needs --project");
    assert!(parse(&["--project"]).is_err(), "a flag needs its value");
    assert!(parse(&["--nonsense"]).is_err());
}

#[test]
fn import_parses_its_arguments_and_rejects_the_rest() {
    let parse = |args: &[&str]| {
        ImportRequest::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    };

    let request = parse(&["a.evara", "--to", "b.evaraproj"])
        .expect("parses")
        .expect("not help");
    assert_eq!(request.archive, PathBuf::from("a.evara"));
    assert_eq!(request.to, PathBuf::from("b.evaraproj"));
    assert!(!request.replace, "replacement is never the default");

    let replacing = parse(&["a.evara", "--to", "b.evaraproj", "--replace"])
        .expect("parses")
        .expect("not help");
    assert!(replacing.replace);

    assert!(parse(&["a.evara"]).is_err(), "needs --to");
    assert!(parse(&["--to", "b.evaraproj"]).is_err(), "needs an archive");
    assert!(
        parse(&["a.evara", "b.evara", "--to", "c.evaraproj"]).is_err(),
        "one archive at a time"
    );
    assert!(parse(&["--nonsense", "a.evara"]).is_err());
}

#[test]
fn a_windows_style_absolute_destination_is_accepted_where_it_is_meaningful() {
    // The point is that a drive-qualified path parses as one path rather than being split, on
    // both platforms. Whether it is *usable* is the filesystem's business, so this asserts
    // parsing only — the destination rules themselves are `evara-project`'s and are tested
    // there against real directories.
    let args = [
        "c:\\schools\\north.evara",
        "--to",
        "c:\\schools\\north.evaraproj",
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect::<Vec<_>>();
    let request = ImportRequest::parse(&args)
        .expect("parses")
        .expect("not help");
    assert_eq!(request.archive, PathBuf::from("c:\\schools\\north.evara"));
    assert_eq!(request.to, PathBuf::from("c:\\schools\\north.evaraproj"));
}

#[test]
fn the_usage_text_says_what_a_digest_does_and_does_not_prove() {
    // The one claim a user cannot check for themselves, so it is asserted rather than left to
    // whoever edits the help text next.
    assert!(super::INSPECT_USAGE.contains("CORRUPTION"));
    assert!(super::INSPECT_USAGE.contains("AUTHENTICITY"));
    assert!(
        super::IMPORT_USAGE.contains("never deleted"),
        "the import text must say a replaced project is kept"
    );
    assert!(
        super::EXPORT_USAGE.contains("refused rather than overwritten"),
        "the export text must say it will not overwrite"
    );
}
