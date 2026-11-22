//! The eleven commands, driven end to end against real archives and real campus files.
//!
//! # Why these run the protocol rather than mocking it
//!
//! `evara-project` already proves the protocol: round trips, corruption, replacement,
//! recovery, the §0.2 safeguards. Repeating any of that here would be a second opinion about
//! somebody else's rule. What is **only** testable here is the wiring — the session, the
//! progress stream, the translated error, the lock discipline, the recent-list entry and the
//! refusals this layer owns because only it knows what is open — so these tests run the real
//! engine over a real fixture and assert about that wiring.
//!
//! # Nothing here touches the installation's own settings database
//!
//! Every harness builds its workspace with `Workspace::with_settings_at` inside a `TempDir`.
//! `Settings::open_at` runs the settings migration chain with `set_abort_missing(true)`, so a
//! test that reached the real file would migrate the developer's installation as a side
//! effect of `cargo test` and leave every older build unable to open it — the defect
//! `evara-cli`'s `open_settings` seam exists to prevent. [`settings_isolation`] asserts it
//! rather than leaving it to care.

use std::path::Path;
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant};

use evara_db::fixtures::seed::Project;
use evara_db::fixtures::transfer::Shape;
use evara_db::{RecentProject, Workspace};
use evara_project::{OnConflict, Phase, TransferError};

use super::*;

/// The version a harness workspace records.
const APP_VERSION: &str = "0.1.0-test";

/// How long a test waits for a worker before calling it stuck.
///
/// Generous: CI runners are slow and a flaky timeout is worse than a slow test. A transfer
/// of the `Varied` fixture is tens of milliseconds on a developer machine.
const PATIENCE: Duration = Duration::from_secs(90);

// =======================================================================================
// The harness
// =======================================================================================

/// A host that records every event and writes the recent-list entry the real one writes.
struct TestHost {
    progress: Mutex<Vec<TransferProgressEvent>>,
    finished: Mutex<Vec<TransferFinishedEvent>>,
    /// Set after the state exists, because the state owns the sessions the host emits about.
    state: Mutex<Weak<AppState>>,
}

impl TestHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            progress: Mutex::new(Vec::new()),
            finished: Mutex::new(Vec::new()),
            state: Mutex::new(Weak::new()),
        })
    }

    fn progress_of(&self, run_id: Uuid) -> Vec<TransferProgressEvent> {
        self.progress
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.run_id == run_id)
            .copied()
            .collect()
    }

    fn finished_of(&self, run_id: Uuid) -> Vec<TransferFinishedEvent> {
        self.finished
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.run_id == run_id)
            .copied()
            .collect()
    }
}

impl TransferHost for TestHost {
    fn emit_progress(&self, event: TransferProgressEvent) {
        self.progress.lock().expect("events").push(event);
    }

    fn emit_finished(&self, event: TransferFinishedEvent) {
        self.finished.lock().expect("events").push(event);
    }

    fn remember_imported(&self, path: &Path, project_id: Uuid) -> bool {
        // Deliberately the same body as `DesktopHost`'s: the point of the seam is that the
        // worker's behaviour is the shipped one and only the delivery differs.
        let Some(state) = self.state.lock().expect("state").upgrade() else {
            return false;
        };
        let Ok(mut workspace) = state.workspace.lock() else {
            return false;
        };
        workspace
            .remember_imported_project(path, project_id)
            .is_ok()
    }
}

/// A desktop application over a synthetic campus, with nothing of the installation's own.
struct Harness {
    /// Held so the temporary directory outlives everything in it.
    _fixture: Project,
    directory: std::path::PathBuf,
    project_path: std::path::PathBuf,
    state: Arc<AppState>,
    host: Arc<TestHost>,
}

impl Harness {
    /// Builds a campus, then opens it in a workspace of this harness's own.
    fn new() -> Self {
        Self::with_shape(Shape::Varied)
    }

    fn with_shape(shape: Shape) -> Self {
        let (mut fixture, _built) = evara_db::fixtures::transfer::build(shape);
        let directory = fixture.directory().to_path_buf();
        let project_path = fixture.path().to_path_buf();
        // The fixture's own workspace releases the file; this harness opens it with a
        // workspace whose settings database is its own.
        fixture.close();

        let mut workspace =
            Workspace::with_settings_at(&directory.join("desktop-settings.sqlite"), APP_VERSION)
                .expect("harness settings database");
        workspace
            .open_project(&project_path)
            .expect("open the fixture campus");

        let state = Arc::new(AppState {
            workspace: std::sync::Mutex::new(workspace),
            imports: crate::commands::import::ImportSessions::default(),
            generations: crate::commands::session::GenerationSessions::default(),
            edits: crate::commands::edit_session::EditSessions::default(),
            repairs: crate::commands::repair_session::RepairSessions::default(),
            transfers: crate::commands::transfer_session::TransferSessions::default(),
            recovery: Vec::new(),
            epoch: crate::commands::session::ProjectEpoch::default(),
        });

        let host = TestHost::new();
        *host.state.lock().expect("state") = Arc::downgrade(&state);

        Self {
            _fixture: fixture,
            directory,
            project_path,
            state,
            host,
        }
    }

    /// A path inside the harness's temporary directory.
    fn at(&self, name: &str) -> std::path::PathBuf {
        self.directory.join(name)
    }

    fn host(&self) -> Arc<dyn TransferHost> {
        Arc::clone(&self.host) as Arc<dyn TransferHost>
    }

    /// Exports to `name` and waits for the worker.
    fn export_to(&self, name: &str) -> (Uuid, ExportStatus) {
        let started = export_start(&self.host(), &self.state, &self.at(name))
            .unwrap_or_else(|error| panic!("export to {name} refused: {error:?}"));
        let status = self.await_export(started.run_id);
        (started.run_id, status)
    }

    /// Exports to `name`, waits, and returns the archive path once it is there.
    fn an_archive(&self, name: &str) -> std::path::PathBuf {
        let (_, status) = self.export_to(name);
        assert_eq!(
            status.phase,
            TransferPhase::Finished,
            "the fixture export must succeed: {:?}",
            status.failure
        );
        self.at(name)
    }

    fn await_export(&self, run_id: Uuid) -> ExportStatus {
        self.await_terminal(run_id, TransferKind::Export);
        export_status(&self.state, run_id).expect("the session is still held")
    }

    fn await_import(&self, run_id: Uuid) -> ImportStatus {
        self.await_terminal(run_id, TransferKind::Import);
        import_status(&self.state, run_id).expect("the session is still held")
    }

    /// Polls until the session leaves `Running`, or gives up loudly.
    fn await_terminal(&self, run_id: Uuid, kind: TransferKind) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let session = self
                .state
                .transfers
                .get_of_kind(run_id, kind)
                .unwrap_or_else(|| panic!("the {} session was forgotten", kind.as_str()));
            if session.is_terminal() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the {} worker did not finish within {PATIENCE:?}",
                kind.as_str()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Which campus is open, by path.
    fn open_path(&self) -> Option<std::path::PathBuf> {
        self.state
            .workspace
            .lock()
            .expect("workspace")
            .current_project()
            .map(|project| project.path)
    }

    fn recent(&self) -> Vec<RecentProject> {
        self.state
            .workspace
            .lock()
            .expect("workspace")
            .recent_projects()
            .expect("recent list")
    }
}

/// Every file in a directory, for a before-and-after comparison.
fn listing(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("list the directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// The `kind` tag a refusal carries on the wire.
fn kind_of(error: &ProjectError) -> String {
    serde_json::to_value(error).expect("serialise")["kind"]
        .as_str()
        .expect("tagged")
        .to_owned()
}

// =======================================================================================
// 1 — the eleven are registered and reachable
// =======================================================================================

#[test]
fn all_eleven_commands_exist_with_the_approved_names() {
    // Named as function items, so a rename or a deletion is a compile error here rather than
    // a runtime "command not found" the user discovers by clicking. The *registration* half
    // — that each is in `generate_handler!` and wrapped exactly once in the frontend — is
    // asserted by `ipc/registration.test.ts`, which reads both sides as text.
    let _: fn() -> &'static str = project_archive_extension;
    let _ = project_export_start;
    let _ = project_export_status;
    let _ = project_export_cancel;
    let _ = project_export_discard;
    let _ = project_import_inspect;
    let _ = project_import_start;
    let _ = project_import_status;
    let _ = project_import_cancel;
    let _ = project_import_discard;
    let _ = project_recovery_report;
}

#[test]
fn the_archive_extension_is_the_librarys_and_is_not_the_project_one() {
    assert_eq!(
        project_archive_extension(),
        evara_project::ARCHIVE_EXTENSION
    );
    assert_ne!(
        project_archive_extension(),
        evara_db::PROJECT_EXTENSION,
        "a live campus and a portable file are different objects and must never share an \
         extension"
    );
}

// =======================================================================================
// 2 — export and import through the handlers
// =======================================================================================

#[test]
fn an_export_writes_an_archive_and_changes_nothing_else() {
    let harness = Harness::new();
    let before = listing(&harness.directory);

    let (run_id, status) = harness.export_to("Northgate.evara");

    assert_eq!(
        status.phase,
        TransferPhase::Finished,
        "{:?}",
        status.failure
    );
    let report = status.report.expect("a finished export has a report");
    assert!(harness.at("Northgate.evara").is_file());
    assert_eq!(report.file_name, "Northgate.evara");
    assert!(report.archive_bytes > 0);
    assert!(report.payload_bytes > 0);
    assert_eq!(report.payload_sha256.len(), 64, "a sha256 in lowercase hex");
    assert!(!report.schools.is_empty(), "the fixture has two schools");

    // The campus is still open and still the one it was.
    assert_eq!(
        harness.open_path().as_deref(),
        Some(harness.project_path.as_path())
    );

    // Nothing but the archive appeared: no staging directory, no `.part`, no sidecar.
    let after = listing(&harness.directory);
    let added: Vec<&String> = after.iter().filter(|name| !before.contains(name)).collect();
    assert_eq!(
        added,
        vec![&String::from("Northgate.evara")],
        "an export writes the archive and nothing else; got {after:?}"
    );

    // And exactly one terminal event, saying it succeeded.
    let finished = harness.host.finished_of(run_id);
    assert_eq!(finished.len(), 1, "exactly one terminal event");
    assert!(finished[0].ok);
    assert_eq!(finished[0].phase, TransferPhase::Finished);
    assert_eq!(finished[0].kind, TransferKind::Export);
}

#[test]
fn an_import_writes_a_campus_carries_the_identity_and_does_not_open_it() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Imported.evaraproj");

    let started = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("import accepted");
    let status = harness.await_import(started.run_id);

    assert_eq!(
        status.phase,
        TransferPhase::Finished,
        "{:?}",
        status.failure
    );
    let report = status.report.expect("a finished import has a report");
    assert!(
        destination.is_file(),
        "the campus must be where it was asked for"
    );
    assert!(report.total_rows > 0, "the fixture carries rows");
    assert!(
        !report.migrated,
        "an archive this build wrote is already current"
    );
    assert_eq!(report.replaced_copy_kept_at, None, "nothing was replaced");

    // D13 / ADR 0004: the identity travelled and was not regenerated.
    let source_identity = evara_project::inspect(&archive)
        .expect("inspect")
        .manifest
        .project_id;
    assert_eq!(report.project_id, source_identity);

    // D9: remembered, never opened.
    assert_eq!(
        harness.open_path().as_deref(),
        Some(harness.project_path.as_path()),
        "an import must not close whatever the user had open"
    );
    assert!(report.remembered);
    assert!(
        harness
            .recent()
            .iter()
            .any(|entry| entry.path == destination && entry.project_id == Some(source_identity)),
        "the imported campus must be findable, so the interface can offer 'Open now'"
    );
}

#[test]
fn a_round_trip_through_the_handlers_preserves_every_table_count() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Imported.evaraproj");

    let declared = evara_project::inspect(&archive)
        .expect("inspect")
        .manifest
        .counts;
    let started = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("import accepted");
    let report = harness
        .await_import(started.run_id)
        .report
        .expect("a report");

    assert!(
        report.count_disagreements.is_empty(),
        "a manifest this build wrote must agree with the campus it describes: {:?}",
        report.count_disagreements
    );
    for (table, rows) in &declared {
        assert_eq!(
            report.counts.get(table),
            Some(rows),
            "{table} did not arrive intact"
        );
    }
}

// =======================================================================================
// 3 — inspection writes nothing  (T-55)
// =======================================================================================

#[test]
fn inspecting_an_archive_changes_nothing_on_disk() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");

    let before = listing(&harness.directory);
    let before_bytes = std::fs::read(&archive).expect("read the archive");

    let summary = evara_project::inspect(&archive).expect("inspect");
    let view = summarise(&archive, &summary, &[]);

    assert_eq!(
        listing(&harness.directory),
        before,
        "no file appeared or left"
    );
    assert_eq!(
        std::fs::read(&archive).expect("read the archive"),
        before_bytes,
        "the archive itself must be byte-identical afterwards"
    );
    assert!(view.readable);
    assert!(view.refusal.is_none());
    assert_eq!(view.suggested_file_name, "Northgate.evaraproj");
    assert!(
        view.integrity_notice.contains("not") && view.integrity_notice.contains("authenticity"),
        "the corruption-not-authenticity sentence is said at the point of import"
    );
}

#[test]
fn a_clean_inspection_is_not_a_claim_about_the_payload() {
    // The archive's header is intact and its *payload* is not. Inspect does not read the
    // payload — that is step 9 of the verification order and belongs to the import — so this
    // reads cleanly and the import then refuses it before anything is written.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");

    let mut bytes = std::fs::read(&archive).expect("read");
    // Corrupt deep inside the compressed payload, well past both JSON entries.
    let at = bytes.len() * 3 / 4;
    bytes[at] ^= 0xFF;
    std::fs::write(&archive, &bytes).expect("write back");

    let inspected = evara_project::inspect(&archive);
    let destination = harness.at("Damaged.evaraproj");
    let started = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    );

    // Either the header read cleanly and the import refused, or the corruption happened to
    // land in the central directory and the inspection refused first. Both are correct; what
    // must never happen is an import that *succeeds*.
    match started {
        Ok(started) => {
            let status = harness.await_import(started.run_id);
            assert_ne!(
                status.phase,
                TransferPhase::Finished,
                "a damaged payload must never become a campus"
            );
            assert!(
                inspected.is_ok(),
                "the header was intact, so inspect read it"
            );
        }
        Err(error) => {
            assert!(
                matches!(
                    error,
                    ProjectError::ArchiveDamaged { .. } | ProjectError::ArchiveUnreadable { .. }
                ),
                "got {error:?}"
            );
        }
    }
    assert!(
        !destination.exists(),
        "nothing is written when verification fails"
    );
}

// =======================================================================================
// 4 — session lifecycle and cleanup
// =======================================================================================

#[test]
fn closing_the_project_forgets_every_transfer_session() {
    let harness = Harness::new();
    let (run_id, _) = harness.export_to("Northgate.evara");
    assert!(export_status(&harness.state, run_id).is_ok());

    // What `project_close` does, in the order it does it.
    harness.state.transfers.clear();
    harness.state.epoch.bump();

    assert!(harness.state.transfers.is_empty());
    let refusal = export_status(&harness.state, run_id).expect_err("the session is gone");
    assert_eq!(kind_of(&refusal), "notFound");
}

#[test]
fn a_discarded_session_is_forgotten_and_a_second_discard_is_refused() {
    let harness = Harness::new();
    let (run_id, _) = harness.export_to("Northgate.evara");

    export_discard(&harness.state, run_id).expect("the first discard");
    let refusal = export_discard(&harness.state, run_id).expect_err("nothing left to discard");
    assert_eq!(kind_of(&refusal), "notFound");
}

#[test]
fn an_export_identifier_is_not_an_import_identifier() {
    let harness = Harness::new();
    let (run_id, _) = harness.export_to("Northgate.evara");

    for refusal in [
        import_status(&harness.state, run_id).err(),
        import_cancel(&harness.state, run_id).err(),
        import_discard(&harness.state, run_id).err(),
    ] {
        let refusal = refusal.expect("an import command must not reach an export session");
        assert_eq!(kind_of(&refusal), "notFound");
    }
    // And the export is untouched by every one of those attempts.
    assert_eq!(
        export_status(&harness.state, run_id)
            .expect("still held")
            .phase,
        TransferPhase::Finished
    );
}

// =======================================================================================
// 5 — concurrency and duplicate execution
// =======================================================================================

#[test]
fn two_exports_to_one_archive_cannot_both_be_started() {
    let harness = Harness::new();
    let destination = harness.at("Northgate.evara");

    let first = export_start(&harness.host(), &harness.state, &destination).expect("the first");
    // The second is refused whichever way the race goes: either the first is still writing,
    // or it has finished and the archive is now occupied. Both refusals name the file.
    let second = export_start(&harness.host(), &harness.state, &destination).expect_err("refused");
    assert!(
        matches!(
            second,
            ProjectError::TransferBlocked { .. } | ProjectError::AlreadyExists { .. }
        ),
        "got {second:?}"
    );

    let status = harness.await_export(first.run_id);
    assert_eq!(
        status.phase,
        TransferPhase::Finished,
        "{:?}",
        status.failure
    );
    assert_eq!(
        listing(&harness.directory)
            .iter()
            .filter(|name| {
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case(evara_project::ARCHIVE_EXTENSION)
                    })
            })
            .count(),
        1,
        "exactly one archive exists"
    );
}

#[test]
fn two_imports_to_one_destination_cannot_both_be_started() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Imported.evaraproj");

    let first = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("the first");
    let second = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect_err("refused");
    assert!(
        matches!(
            second,
            ProjectError::TransferBlocked { .. } | ProjectError::AlreadyExists { .. }
        ),
        "got {second:?}"
    );

    let status = harness.await_import(first.run_id);
    assert_eq!(
        status.phase,
        TransferPhase::Finished,
        "{:?}",
        status.failure
    );
    assert!(destination.is_file());
}

#[test]
fn two_exports_to_two_archives_both_run() {
    let harness = Harness::new();
    let first =
        export_start(&harness.host(), &harness.state, &harness.at("One.evara")).expect("the first");
    let second = export_start(&harness.host(), &harness.state, &harness.at("Two.evara"))
        .expect("concurrent transfers to different files are ordinary");

    for run_id in [first.run_id, second.run_id] {
        let status = harness.await_export(run_id);
        assert_eq!(
            status.phase,
            TransferPhase::Finished,
            "{:?}",
            status.failure
        );
    }
    assert!(harness.at("One.evara").is_file());
    assert!(harness.at("Two.evara").is_file());
}

// =======================================================================================
// 6 — progress ordering and coalescing
// =======================================================================================

#[test]
fn an_export_reports_its_phases_in_order_and_ends_with_exactly_one_terminal_event() {
    let harness = Harness::new();
    let (run_id, _) = harness.export_to("Northgate.evara");

    let events = harness.host.progress_of(run_id);
    assert!(!events.is_empty(), "a transfer must report progress");
    assert!(
        events
            .iter()
            .all(|event| event.kind == TransferKind::Export),
        "every event must name the direction it belongs to"
    );

    // The distinct phases, in first-seen order. `Phase` derives `Ord` in the library's own
    // declaration order, which is the order the protocol runs them.
    let mut seen: Vec<Phase> = Vec::new();
    for event in &events {
        if seen.last() != Some(&event.step) {
            seen.push(event.step);
        }
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] < pair[1]),
        "phases must not be reordered or revisited: {seen:?}"
    );
    assert_eq!(
        seen.first(),
        Some(&Phase::Snapshotting),
        "an export begins by taking the copy"
    );
    assert!(
        seen.contains(&Phase::Finalising),
        "and ends by renaming into place: {seen:?}"
    );

    assert_eq!(harness.host.finished_of(run_id).len(), 1);
}

#[test]
fn every_event_names_the_session_and_the_project_it_belongs_to() {
    let harness = Harness::new();
    let epoch = harness.state.epoch.get();
    let (run_id, _) = harness.export_to("Northgate.evara");

    for event in harness.host.progress_of(run_id) {
        assert_eq!(event.run_id, run_id);
        assert_eq!(
            event.epoch, epoch,
            "a session belonging to a campus the user has moved away from must be \
             distinguishable, or a stale event would drive the screen in front of them"
        );
    }
}

// =======================================================================================
// 7 — cancellation
// =======================================================================================

#[test]
fn cancelling_an_export_leaves_no_archive_and_no_partial_file() {
    let harness = Harness::new();
    let before = listing(&harness.directory);

    let started = export_start(
        &harness.host(),
        &harness.state,
        &harness.at("Northgate.evara"),
    )
    .expect("started");
    let was_running = export_cancel(&harness.state, started.run_id).expect("the session is held");
    let status = harness.await_export(started.run_id);

    if status.phase == TransferPhase::Cancelled {
        assert!(
            was_running,
            "a cancel that took effect saw something running"
        );
        assert!(
            status.failure.is_none(),
            "the user pressed Cancel; that is not a failure to report"
        );
        assert_eq!(
            listing(&harness.directory),
            before,
            "a cancelled export leaves no archive, no partial file and no staging"
        );
    } else {
        // The worker won the race and finished. That is the honest outcome and the archive
        // is real; what must never happen is a half-written one.
        assert_eq!(
            status.phase,
            TransferPhase::Finished,
            "{:?}",
            status.failure
        );
        assert!(harness.at("Northgate.evara").is_file());
        assert!(
            !harness.at("Northgate.evara.part").exists(),
            "a partial archive must never survive"
        );
    }
}

#[test]
fn cancelling_an_import_leaves_no_campus_and_no_staging() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let before = listing(&harness.directory);
    let destination = harness.at("Imported.evaraproj");

    let started = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("started");
    import_cancel(&harness.state, started.run_id).expect("the session is held");
    let status = harness.await_import(started.run_id);

    if status.phase == TransferPhase::Cancelled {
        assert_eq!(
            listing(&harness.directory),
            before,
            "a cancelled import leaves no campus file and no staging directory"
        );
    } else {
        assert_eq!(
            status.phase,
            TransferPhase::Finished,
            "{:?}",
            status.failure
        );
        assert!(destination.is_file());
    }
}

#[test]
fn a_cancelled_protocol_error_is_a_cancellation_and_never_a_failure() {
    // The mapping `settle` turns on, asserted directly: every other `TransferError` becomes a
    // failure the user may need to act on, and this one becomes the phase that says nothing
    // was written.
    let session = Arc::new(TransferSession::new(
        Uuid::now_v7(),
        TransferKind::Export,
        0,
        None,
        std::path::PathBuf::from("/tmp/a.evara"),
        0,
        String::from("2026-11-13T00:00:00.000Z"),
    ));
    let host = TestHost::new();
    settle(&session, &*host, Err(TransferError::Cancelled));

    assert_eq!(session.phase(), TransferPhase::Cancelled);
    assert!(session.failure().is_none());
    let events = host.finished_of(session.run_id);
    assert_eq!(events.len(), 1);
    assert!(!events[0].ok);
    assert_eq!(
        events[0].phase,
        TransferPhase::Cancelled,
        "`ok: false` alone cannot tell a screen whether to apologise"
    );
}

#[test]
fn cancelling_a_run_this_process_does_not_hold_is_refused() {
    let harness = Harness::new();
    let refusal = export_cancel(&harness.state, Uuid::now_v7()).expect_err("unknown run");
    assert_eq!(kind_of(&refusal), "notFound");
}

// =======================================================================================
// 8 — destination conflicts and explicit replacement
// =======================================================================================

#[test]
fn an_occupied_export_destination_is_refused_and_the_existing_archive_is_untouched() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let original = std::fs::read(&archive).expect("read");

    let refusal = export_start(&harness.host(), &harness.state, &archive).expect_err("refused");
    assert_eq!(kind_of(&refusal), "alreadyExists");
    assert_eq!(
        std::fs::read(&archive).expect("read"),
        original,
        "Evara never overwrites a portable file you may still need"
    );
}

#[test]
fn the_export_destination_is_claimed_atomically_rather_than_checked() {
    // An `exists()` test leaves seconds between the answer and the rename that finishes an
    // export, and a file created inside that window would be destroyed without anybody being
    // asked. The claim closes it, and it cleans up after itself on every path out.
    let harness = Harness::new();
    let destination = harness.at("Claimed.evara");

    let claim = ArchiveClaim::take(&destination).expect("the name is free");
    assert!(destination.exists(), "the placeholder holds the name");
    assert_eq!(
        std::fs::metadata(&destination).expect("metadata").len(),
        0,
        "and holds nothing else"
    );

    // A second claim of the same name loses, whoever is asking.
    let refused = ArchiveClaim::take(&destination).expect_err("the name is taken");
    assert_eq!(kind_of(&refused), "alreadyExists");

    drop(claim);
    assert!(
        !destination.exists(),
        "an unreleased claim removes its own placeholder, so a refusal leaves nothing behind"
    );
}

#[test]
fn an_occupied_campus_destination_is_refused_without_an_explicit_replace() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Occupied.evaraproj");
    std::fs::write(&destination, b"not a campus, and not ours to destroy").expect("write");
    let original = std::fs::read(&destination).expect("read");

    let refusal = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect_err("refused");

    assert_eq!(kind_of(&refusal), "alreadyExists");
    assert_eq!(std::fs::read(&destination).expect("read"), original);
}

#[test]
fn a_zero_length_destination_is_occupied_rather_than_adopted() {
    // Deliberately unlike creating a campus, which treats a zero-length file as creatable.
    // An import adopting an unknown empty file would be writing over something nobody has
    // looked at — safeguard S1.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Empty.evaraproj");
    std::fs::write(&destination, b"").expect("write an empty file");

    let refusal = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect_err("refused");
    assert_eq!(kind_of(&refusal), "alreadyExists");
}

#[test]
fn replacing_keeps_the_previous_campus_beside_the_new_one() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Target.evaraproj");

    // A first import makes a campus to replace.
    let first = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("the first import");
    assert_eq!(
        harness.await_import(first.run_id).phase,
        TransferPhase::Finished
    );
    let original = std::fs::read(&destination).expect("read");

    let second = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Replace,
    )
    .expect("replace accepted");
    let status = harness.await_import(second.run_id);
    assert_eq!(
        status.phase,
        TransferPhase::Finished,
        "{:?}",
        status.failure
    );

    let report = status.report.expect("a report");
    let kept = report
        .replaced_copy_kept_at
        .expect("the previous campus must be kept, not deleted");
    let kept = std::path::PathBuf::from(kept);
    assert!(kept.is_file(), "the kept copy must be on disk: {kept:?}");
    assert_eq!(
        std::fs::read(&kept).expect("read the kept copy"),
        original,
        "the kept copy must be the campus that was there, byte for byte"
    );
    assert!(
        report
            .replaced_copy_name
            .as_deref()
            .is_some_and(|name| name.contains(".replaced-")),
        "the kept copy's name is what the message names"
    );
    assert!(destination.is_file(), "and the new campus is in place");
}

#[test]
fn importing_over_the_open_campus_is_refused_with_the_remedy_named() {
    // D13 case 3, and T-66. Replacing a file SQLite still holds open is how a campus ends up
    // paired with a live `-wal` belonging to a database that no longer exists.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let open = harness.project_path.clone();
    let before = std::fs::metadata(&open).expect("metadata").len();

    let refusal = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &open,
        OnConflict::Replace,
    )
    .expect_err("refused");

    assert_eq!(kind_of(&refusal), "destinationIsOpen");
    let ProjectError::DestinationIsOpen { message, file_name } = &refusal else {
        panic!("got {refusal:?}");
    };
    assert!(message.contains(file_name), "the message names the file");
    assert!(
        message.contains("Close it"),
        "a refusal without a remedy strands the user: {message}"
    );
    assert_eq!(
        std::fs::metadata(&open).expect("metadata").len(),
        before,
        "the open campus must be untouched"
    );
}

#[test]
fn an_archive_is_refused_as_a_campus_destination_and_the_other_way_round() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");

    // `.evara` as a live campus.
    let refusal = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &harness.at("Wrong.evara"),
        OnConflict::Refuse,
    )
    .expect_err("refused");
    assert_eq!(kind_of(&refusal), "wrongExtension");

    // `.evaraproj` as a portable file.
    let refusal = export_start(
        &harness.host(),
        &harness.state,
        &harness.at("Wrong.evaraproj"),
    )
    .expect_err("refused");
    assert_eq!(kind_of(&refusal), "wrongExtension");
}

// =======================================================================================
// 9 — recovery, and the backup that is never deleted
// =======================================================================================

#[test]
fn startup_recovery_reports_what_it_did_and_the_report_is_read_back_not_recomputed() {
    let harness = Harness::new();
    // Nothing was interrupted, so recovery has nothing to say — and says so by returning an
    // empty list rather than by failing.
    let settings = harness.directory.join("desktop-settings.sqlite");
    assert!(recover_at_startup(&settings).is_empty());

    assert!(
        harness.state.recovery.is_empty(),
        "the command reads the startup decision back; it does not run recovery again"
    );
}

#[test]
fn recovery_that_cannot_run_is_reported_rather_than_stopping_the_application() {
    // A settings path that cannot be a database. The user still needs to be able to open a
    // campus and be told why, so this is a row rather than a panic at startup.
    let harness = Harness::new();
    // A path whose *parent* is an ordinary file, so the directory it would need cannot be
    // created. `Settings::open_at` makes missing directories, so an absent path is not
    // enough to reach this branch.
    let blocker = harness.at("blocker");
    std::fs::write(&blocker, b"a file where a folder would have to be").expect("write");
    let impossible = blocker.join("settings.sqlite");

    let reported = recover_at_startup(&impossible);
    assert_eq!(reported.len(), 1);
    assert!(!reported[0].resolved);
    assert!(
        reported[0].outcome.contains("Nothing was changed"),
        "got {}",
        reported[0].outcome
    );
}

#[test]
fn an_interrupted_replacement_is_reconciled_against_the_disk_and_never_the_journal() {
    // The data-loss case the first draft of the design got wrong: the journal says the new
    // campus is `PLACED`, and the crash landed between that commit and the rename. Read
    // literally, `PLACED` means "finish by moving the aside copy to the keep-name" — which
    // would leave no campus at the destination at all.
    let harness = Harness::new();
    let settings_path = harness.directory.join("desktop-settings.sqlite");
    let destination = harness.at("Interrupted.evaraproj");
    let staging = harness.at("staging-interrupted");
    let aside = staging.join("aside");
    std::fs::create_dir_all(&aside).expect("aside folder");

    // The old campus is in `aside/`, the destination is absent, and the journal says PLACED.
    let old = aside.join("Interrupted.evaraproj");
    std::fs::copy(&harness.project_path, &old).expect("put the old campus aside");
    let original = std::fs::read(&old).expect("read");

    {
        let mut settings = evara_db::Settings::open_at(&settings_path).expect("settings");
        let row = settings
            .begin_replacement(&destination, &staging)
            .expect("journal the replacement");
        settings
            .advance_replacement(row.id, evara_db::transfer::ReplacementPhase::Placed, None)
            .expect("advance");
    }

    let recovered = recover_at_startup(&settings_path);

    assert_eq!(
        recovered.len(),
        1,
        "one interrupted replacement: {recovered:?}"
    );
    assert!(
        destination.is_file(),
        "the user's only campus must be put back at its own name, not moved to a \
         `.replaced-` one"
    );
    assert_eq!(
        std::fs::read(&destination).expect("read"),
        original,
        "and it must be the campus that was there"
    );
}

#[test]
fn nothing_in_the_transfer_path_deletes_a_campus_it_replaced() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Target.evaraproj");

    let first = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("the first");
    harness.await_import(first.run_id);

    // Replace it twice. Both previous copies must still be there afterwards.
    let mut kept = Vec::new();
    for _ in 0..2 {
        let run = import_start(
            &harness.host(),
            &harness.state,
            &archive,
            &destination,
            OnConflict::Replace,
        )
        .expect("replace accepted");
        let report = harness.await_import(run.run_id).report.expect("a report");
        kept.push(report.replaced_copy_kept_at.expect("a kept copy"));
    }

    assert_eq!(kept.len(), 2);
    for path in &kept {
        assert!(
            Path::new(path).is_file(),
            "{path} was deleted; no code path may delete a campus it replaced"
        );
    }
    assert_ne!(kept[0], kept[1], "a second replacement needs a fresh name");
}

// =======================================================================================
// 10 — duplicate identity disclosure (D13)
// =======================================================================================

/// A recent entry naming `path` with `identity`.
fn a_recent(path: &Path, identity: Uuid) -> RecentProject {
    RecentProject {
        display_name: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.to_path_buf(),
        project_id: Some(identity),
        last_opened_at: String::from("2026-11-13T00:00:00.000Z"),
        exists: true,
        duplicate_identity: true,
    }
}

#[test]
fn a_twin_on_this_machine_is_disclosed_before_anything_is_staged() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let summary = evara_project::inspect(&archive).expect("inspect");
    let identity = summary.manifest.project_id;

    // The campus the archive came from is already in this installation's recent list, which
    // is the ordinary way a twin appears.
    let view = {
        let workspace = harness.state.workspace.lock().expect("workspace");
        let conflicts = workspace
            .projects_with_identity(identity)
            .expect("read by identity");
        assert!(
            !conflicts.is_empty(),
            "the open campus carries this identity and is remembered"
        );
        summarise(&archive, &summary, &conflicts)
    };

    assert!(!view.identity_conflicts.is_empty());
    assert!(
        view.identity_conflicts
            .iter()
            .all(|entry| entry.project_id.as_deref() == Some(identity.to_string().as_str())),
        "every conflict names the identity it conflicts on"
    );

    // D13's first sentence, verbatim in its shape.
    let notice = view.identity_notice.expect("a twin must be disclosed");
    assert!(notice.contains("already on this machine"), "{notice}");
    assert!(
        notice.contains("Importing creates a second copy"),
        "{notice}"
    );
    assert!(
        notice.contains("cannot merge two copies"),
        "the user must be told Evara cannot reconcile them: {notice}"
    );
}

#[test]
fn no_twin_means_no_notice_at_all() {
    assert_eq!(identity_notice(&[]), None);
}

#[test]
fn replacing_the_same_campus_is_worded_more_sharply_than_replacing_a_different_one() {
    let destination = Path::new("/campuses/Northgate.evaraproj");
    let identity = Uuid::now_v7();

    // D13's second sentence: the destination *is* this campus.
    let same = replacement_notice(destination, &[a_recent(destination, identity)]);
    assert!(same.contains("which is the same campus"), "{same}");
    assert!(same.contains("Northgate.evaraproj"), "{same}");
    assert!(
        same.contains("kept beside it as Northgate.replaced-"),
        "the user must be told where their current copy goes: {same}"
    );

    // A different campus at that path: still a replacement, still non-destructive, but it is
    // not the same campus and saying so would be false.
    let other = replacement_notice(
        destination,
        &[a_recent(
            Path::new("/campuses/Elsewhere.evaraproj"),
            identity,
        )],
    );
    assert!(!other.contains("the same campus"), "{other}");
    assert!(other.contains("kept beside it"), "{other}");
}

#[test]
fn an_identity_is_never_rewritten_by_an_import() {
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let identity = evara_project::inspect(&archive)
        .expect("inspect")
        .manifest
        .project_id;

    // Two copies, both carrying the one identity. Evara discloses; it does not deduplicate,
    // delete or regenerate.
    for name in ["CopyA.evaraproj", "CopyB.evaraproj"] {
        let run = import_start(
            &harness.host(),
            &harness.state,
            &archive,
            &harness.at(name),
            OnConflict::Refuse,
        )
        .expect("import accepted");
        let report = harness.await_import(run.run_id).report.expect("a report");
        assert_eq!(report.project_id, identity, "{name} was renumbered");
    }

    let recent = harness.recent();
    let copies: Vec<&RecentProject> = recent
        .iter()
        .filter(|entry| entry.project_id == Some(identity))
        .collect();
    assert!(
        copies.len() >= 2,
        "both copies must be listable: {copies:?}"
    );
    assert!(
        copies.iter().all(|entry| entry.duplicate_identity),
        "the recent list itself must show that these are the same campus"
    );
    assert!(
        harness.at("CopyA.evaraproj").is_file() && harness.at("CopyB.evaraproj").is_file(),
        "nothing deduplicates or deletes a copy"
    );
}

// =======================================================================================
// 11 — settings isolation
// =======================================================================================

#[test]
fn settings_isolation() {
    // The harness's workspace, the import worker's journal handle and the recent list must
    // all be the temporary file, never the installation's own. Asserted by path, because the
    // damage a slip causes shows up later, on another branch, as an unrelated failure.
    let harness = Harness::new();
    let settings = harness
        .state
        .workspace
        .lock()
        .expect("workspace")
        .settings_path()
        .to_path_buf();

    assert_eq!(settings, harness.directory.join("desktop-settings.sqlite"));
    assert!(
        settings.starts_with(&harness.directory),
        "a test must never migrate the developer's installation: {settings:?}"
    );
}

// =======================================================================================
// 12 — error classification and serialisation
// =======================================================================================

/// Every refusal this layer produces, with the tag it must carry on the wire.
///
/// A table rather than a test per case: the property is that these are *distinguishable*,
/// which is a statement about the set and not about any one of them, and a screen that could
/// not tell two of them apart would offer the wrong remedy.
fn classification_cases() -> Vec<(TransferError, &'static str)> {
    vec![
        (
            TransferError::NotAnArchive {
                name: String::from("notes.txt"),
            },
            "archiveUnreadable",
        ),
        (
            TransferError::ManifestMissing {
                name: String::from("x.evara"),
            },
            "archiveUnreadable",
        ),
        (
            TransferError::UnknownFormat {
                found: String::from("somebody-elses-format"),
            },
            "archiveUnreadable",
        ),
        (
            TransferError::FormatTooNew {
                found: 9,
                supported: 1,
            },
            "formatTooNew",
        ),
        (
            TransferError::UnsupportedCapability {
                capability: String::from("constraint:teacher-must-like-mondays"),
            },
            "unsupportedCapability",
        ),
        (
            TransferError::ChecksumMismatch {
                entry: String::from("payload/project.evaraproj"),
            },
            "archiveDamaged",
        ),
        (
            TransferError::PayloadLengthMismatch {
                declared: 10,
                actual: 4,
            },
            "archiveDamaged",
        ),
        (TransferError::Truncated, "archiveDamaged"),
        (
            TransferError::DestinationExists {
                name: String::from("Northgate.evaraproj"),
            },
            "alreadyExists",
        ),
        (
            TransferError::WrongExtension {
                expected: "evaraproj",
            },
            "wrongExtension",
        ),
        (
            TransferError::RenameBlocked {
                name: String::from("Northgate.evaraproj"),
            },
            "transferBlocked",
        ),
        (
            TransferError::ReplacementInFlight {
                name: String::from("Northgate.evaraproj"),
            },
            "transferBlocked",
        ),
        (TransferError::Cancelled, "cancelled"),
        (
            TransferError::Database(evara_db::DbError::SchemaTooNew {
                found: 99,
                supported: 18,
            }),
            "schemaTooNew",
        ),
        (
            TransferError::Database(evara_db::DbError::PayloadCorrupt {
                detail: String::from("page 4 is not a page"),
            }),
            "archiveDamaged",
        ),
        (
            TransferError::Database(evara_db::DbError::ReferentialDamage { count: 3 }),
            "archiveDamaged",
        ),
    ]
}

#[test]
fn every_refusal_this_layer_produces_is_distinguishable_on_the_wire() {
    for (error, expected) in classification_cases() {
        let rendered = classify(error);
        assert_eq!(
            kind_of(&rendered),
            expected,
            "wrong classification for {rendered:?}"
        );
        let value = serde_json::to_value(&rendered).expect("serialise");
        let message = value["message"]
            .as_str()
            .expect("every refusal has a message");
        assert!(
            !message.is_empty(),
            "{expected} needs words a user can read"
        );
    }
}

#[test]
fn the_two_version_axes_are_two_different_refusals() {
    // D2. A container this build cannot open is a different problem from data it cannot
    // interpret, and collapsing them would tell the user to update for the wrong reason.
    let container = classify(TransferError::FormatTooNew {
        found: 9,
        supported: 1,
    });
    let data = classify(TransferError::Database(evara_db::DbError::SchemaTooNew {
        found: 99,
        supported: 18,
    }));

    assert_ne!(kind_of(&container), kind_of(&data));
    let container = serde_json::to_value(&container).expect("serialise");
    assert_eq!(container["found"], 9);
    assert_eq!(container["supported"], 1);
}

#[test]
fn an_unsupported_capability_is_named_rather_than_summarised() {
    // D3: a rule this reader cannot evaluate is a refusal naming it, never a row quietly
    // dropped, because the alternative is a campus that looks complete and is not.
    let rendered = classify(TransferError::UnsupportedCapability {
        capability: String::from("constraint:teacher-must-like-mondays"),
    });
    let value = serde_json::to_value(&rendered).expect("serialise");
    assert_eq!(value["capability"], "constraint:teacher-must-like-mondays");
    assert!(
        value["message"]
            .as_str()
            .expect("message")
            .contains("teacher-must-like-mondays"),
        "the message must name it too"
    );
}

#[test]
fn a_report_crosses_the_boundary_in_camel_case() {
    let harness = Harness::new();
    let (run_id, _) = harness.export_to("Northgate.evara");
    let status = export_status(&harness.state, run_id).expect("held");
    let json = serde_json::to_string(&status).expect("serialise");

    for key in [
        "\"runId\"",
        "\"cancelRequested\"",
        "\"archiveBytes\"",
        "\"payloadSha256\"",
        "\"changeLogWatermark\"",
        "\"projectId\"",
        "\"schemaVersion\"",
    ] {
        assert!(json.contains(key), "{key} missing from {json}");
    }
    assert!(
        !json.contains("run_id") && !json.contains("archive_bytes"),
        "snake_case must never reach the interface: {json}"
    );
}

#[test]
fn an_unclassified_protocol_error_defaults_to_a_failure_rather_than_a_judgement() {
    // `TransferError` is `#[non_exhaustive]`. A new variant nobody has classified must not
    // claim Evara looked at an archive and decided against it.
    let rendered = classify(TransferError::RecoveryAmbiguous {
        name: String::from("Northgate.evaraproj"),
        detail: String::from("something nobody has written a sentence for"),
    });
    assert_eq!(kind_of(&rendered), "failed");
}

// =======================================================================================
// 13 — filesystem behaviour on both platforms
// =======================================================================================

#[test]
fn the_destination_is_recognised_however_its_path_is_spelled() {
    // Windows paths are case-insensitive and accept either separator; a case-sensitive
    // filesystem genuinely distinguishes two spellings. Both answers come from `evara-db`'s
    // one normalisation rather than from a comparison written here.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let open = harness.project_path.to_string_lossy().into_owned();
    let forward_slashed = std::path::PathBuf::from(open.replace(std::path::MAIN_SEPARATOR, "/"));

    let refusal = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &forward_slashed,
        OnConflict::Replace,
    )
    .expect_err("refused");
    assert_eq!(
        kind_of(&refusal),
        "destinationIsOpen",
        "a differently spelled path is the same campus"
    );
}

#[test]
fn an_import_leaves_no_sidecar_beside_the_campus_it_wrote() {
    // S3: a main file paired with a sidecar belonging to a different database is a campus
    // that opens cleanly and is wrong. The protocol reduces each side to one file; this
    // asserts the result from the outside.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Imported.evaraproj");

    let run = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("import accepted");
    assert_eq!(
        harness.await_import(run.run_id).phase,
        TransferPhase::Finished
    );

    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", destination.display()));
        assert!(
            !sidecar.exists(),
            "{sidecar:?} must not be beside a freshly imported campus"
        );
    }
    // And no staging directory survived.
    assert!(
        !listing(&harness.directory)
            .iter()
            .any(|name| name.starts_with(".evara-") || name.contains("staging")),
        "staging must be cleaned up: {:?}",
        listing(&harness.directory)
    );
}

#[test]
fn an_imported_campus_opens() {
    // The end of the story, and the only assertion that proves the bytes are a working
    // campus rather than a file of the right size: the application opens it.
    let harness = Harness::new();
    let archive = harness.an_archive("Northgate.evara");
    let destination = harness.at("Imported.evaraproj");
    let run = import_start(
        &harness.host(),
        &harness.state,
        &archive,
        &destination,
        OnConflict::Refuse,
    )
    .expect("import accepted");
    let report = harness.await_import(run.run_id).report.expect("a report");

    let mut workspace = harness.state.workspace.lock().expect("workspace");
    let opened = workspace
        .open_project(&destination)
        .expect("the imported campus must open");
    assert_eq!(opened.project_id, report.project_id);
    assert_eq!(opened.schema_version, report.schema_version_now);
}
