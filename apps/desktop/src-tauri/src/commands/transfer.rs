//! The eleven project-transfer commands: inspect, export, import, recover.
//!
//! # The shape, and what each command is for
//!
//! | # | Command | Answers | Writes |
//! | --- | --- | --- | --- |
//! | 1 | [`project_archive_extension`] | *"what does a portable file end in?"* — one source for a dialog filter | no |
//! | 2 | [`project_export_start`] | snapshots the open campus and seals it on a worker thread | the archive only |
//! | 3 | [`project_export_status`] | phase, progress, and the report once there is one | no |
//! | 4 | [`project_export_cancel`] | sets the flag. Changes no state | no |
//! | 5 | [`project_export_discard`] | forgets the session | no |
//! | 6 | [`project_import_inspect`] | *"what does this archive say about itself, and can we read it?"* | **no** |
//! | 7 | [`project_import_start`] | stages, verifies, migrates and renames, on a worker thread | the new campus only |
//! | 8 | [`project_import_status`] | phase, progress, and the report once there is one | no |
//! | 9 | [`project_import_cancel`] | sets the flag | no |
//! | 10 | [`project_import_discard`] | forgets the session | no |
//! | 11 | [`project_recovery_report`] | what startup recovery did to interrupted replacements — D11 | no |
//!
//! # This module is a command surface and nothing else
//!
//! Every byte of the protocol lives in [`evara_project`] and [`evara_db::transfer`]: the
//! archive writer, the reader and its verification order, the staging lifecycle, the
//! replacement protocol and its recovery. What is here is a session, a progress stream, a
//! translated error and a view a screen can render. **There is no second archive writer and
//! no second import path** — `evara-cli` is the first caller of that API and this is the
//! second, with no shared code between them to keep in step.
//!
//! Three consequences worth stating, because each is a rule somebody could break here:
//!
//! - **Nothing in this module compares a digest, opens a payload or judges a schema.** The
//!   reader does, in the order its own documentation fixes, and an opinion formed here could
//!   tell a user an archive was fine that `import` then refused.
//! - **Nothing here extracts anything.** Three entries, by exact name, streamed to a path
//!   Evara chose — ADR 0055, and this layer never sees an entry name at all.
//! - **An import does not open what it imported.** D9. Auto-opening would close whatever the
//!   user had open, discarding an edit session, a generation and a repair as a side effect of
//!   an import. The imported campus is **remembered** instead, so the interface can offer
//!   "Open now" as the user's own decision.
//!
//! # Where the workspace lock is held, and where it is not
//!
//! An export holds it for the **snapshot only** — D8, and the arrangement
//! `ARCHITECTURE.md` §3.3 permits: `evara_project::seal` takes an owned `Snapshot` and no
//! `&Database`, so the command thread snapshots, releases the lock, and moves the snapshot to
//! a worker. Compressing a multi-hundred-megabyte payload under the lock would block every
//! other command for the whole of it.
//!
//! An import never holds it at all while it works. Its journal is the installation's settings
//! database, reached through a handle the worker opens at
//! [`evara_db::Workspace::settings_path`] rather than by borrowing this workspace's — the same
//! arrangement the headless CLI has. The lock is taken twice, briefly: once before the worker
//! starts, to refuse a destination that is the open campus, and once at the end, to remember
//! the imported project in the recent list.
//!
//! # Recovery runs at startup, not here
//!
//! D11: `recover_pending` runs before any project is open, because *the project that needs
//! recovering may be the one that will not open*. [`project_recovery_report`] reads back what
//! it did; it does not run it. See `crate::run`.

// See `project.rs` for why this lint is inapplicable to a module of command handlers: Tauri
// deserialises every parameter, so each one must be owned.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use evara_db::RecentProject;
use evara_db::transfer::Snapshot;
use evara_project::{
    ARCHIVE_EXTENSION, ArchiveSummary, ExportReport, ImportReport, Importer, OnConflict, Phase,
    Readability, TransferError,
};
use serde::Serialize;
use tauri::{Emitter, Manager as _, State};
use uuid::Uuid;

use super::project::{AppState, CommandResult, ProjectError, RecentProjectView, lock, lock_state};
use super::transfer_session::{
    CoalescingTransferSink, TransferKind, TransferOutcome, TransferPhase, TransferSession, Work,
};

/// This build's version, recorded in a manifest's `generator` and as the importing
/// application version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Progress events, so a screen can show a phase and a bar rather than a spinner.
pub const PROGRESS_EVENT: &str = "transfer://progress";
/// Emitted once, when the worker returns, whatever it found.
pub const FINISHED_EVENT: &str = "transfer://finished";

/// What an archive's digests prove, said at the point somebody decides whether to trust one.
///
/// Written here rather than in the interface for the reason every other message in this crate
/// is: one source of wording, beside the rule it describes. §6.2 and ADR 0059 — the integrity
/// model detects **corruption, not authenticity**, because the manifest is the deliberately
/// unverified root of the digest tree and anyone can rewrite it and recompute every digest
/// below it. There is no signature, because a signature needs key distribution and ADR 0001
/// forbids the service that would provide it.
pub const INTEGRITY_NOTICE: &str = "The digests in this archive detect corruption, not \
                                    authenticity. Anyone can rewrite an archive and recompute \
                                    every digest inside it, so a campus file carries exactly \
                                    the authority of the person who sent it to you.";

// =======================================================================================
// What crosses the boundary
// =======================================================================================

/// One school an archive or a snapshot names.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SchoolSummaryView {
    /// Stable identifier.
    pub id: Uuid,
    /// What the school is called.
    pub name: String,
    /// How many campuses it holds.
    pub campuses: i64,
}

impl From<&evara_project::manifest::SchoolSummary> for SchoolSummaryView {
    fn from(summary: &evara_project::manifest::SchoolSummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name.clone(),
            campuses: summary.campuses,
        }
    }
}

/// A table whose declared count and actual count disagree.
///
/// **Advisory, never a refusal.** The payload is the authority and a manifest count is a
/// convenience — but a disagreement means the archive was assembled by something that got one
/// of the two wrong, which a user is better off knowing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CountDisagreementView {
    /// Which table.
    pub table: String,
    /// What the manifest said.
    pub declared: i64,
    /// What the campus holds.
    pub found: i64,
}

/// What an export produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExportReportView {
    /// Where the archive is. The user chose it, so showing it back is expected.
    pub path: String,
    /// Its file name, for a sentence.
    pub file_name: String,
    /// How big the archive is.
    pub archive_bytes: u64,
    /// How big the campus was, uncompressed.
    pub payload_bytes: u64,
    /// The payload's digest, lowercase hex.
    pub payload_sha256: String,
    /// How much smaller the archive is than the campus inside it.
    pub compression_saving_percent: f64,
    /// The identity that travelled. **Never regenerated** — ADR 0004 and D13.
    pub project_id: Uuid,
    /// Which schema version the archive carries.
    pub schema_version: u32,
    /// The schools inside it.
    pub schools: Vec<SchoolSummaryView>,
    /// What a reader must understand to import it, derived from the data — D3.
    pub capabilities: Vec<String>,
    /// How many change-feed entries travelled.
    pub change_log_entries: i64,
    /// Where the change feed stood.
    pub change_log_watermark: i64,
    /// When it was written, ISO-8601 UTC.
    pub exported_at: String,
}

impl ExportReportView {
    fn of(report: &ExportReport) -> Self {
        let manifest = &report.manifest;
        Self {
            file_name: file_name_of(&report.path),
            path: report.path.to_string_lossy().into_owned(),
            archive_bytes: report.archive_bytes,
            payload_bytes: report.payload_bytes,
            payload_sha256: report.payload_sha256.clone(),
            compression_saving_percent: report.compression_saving_percent(),
            project_id: manifest.project_id,
            schema_version: manifest.payload.schema_version,
            schools: manifest.schools.iter().map(Into::into).collect(),
            capabilities: manifest.capabilities.clone(),
            change_log_entries: manifest.change_log.entries,
            change_log_watermark: manifest.change_log.watermark,
            exported_at: manifest.exported_at.clone(),
        }
    }
}

/// What an import produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportReportView {
    /// Where the campus now is.
    pub path: String,
    /// Its file name, for a sentence.
    pub file_name: String,
    /// Its identity, which is the archive's identity. **Never regenerated.**
    pub project_id: Uuid,
    /// The schema version the payload arrived at.
    pub schema_version_found: u32,
    /// The schema version it now stands at.
    pub schema_version_now: u32,
    /// Whether a migration ran.
    pub migrated: bool,
    /// Row counts per table, read from the imported campus.
    pub counts: BTreeMap<String, i64>,
    /// How many tables carry at least one row.
    pub tables_with_rows: usize,
    /// How many rows arrived in total.
    pub total_rows: i64,
    /// Where the replaced campus was kept.
    ///
    /// `null` unless a campus was replaced. **No code path deletes it**, on success or on
    /// failure — which is what makes "a failed import must not destroy the previous campus" a
    /// structural property rather than a promise about error handling.
    pub replaced_copy_kept_at: Option<String>,
    /// The kept copy's file name, for the sentence that names it.
    pub replaced_copy_name: Option<String>,
    /// Safe entry names this build ignored — ADR 0004's forward tolerance.
    pub unknown_entries: Vec<String>,
    /// Tables where the manifest's count and the campus's disagree. Advisory.
    pub count_disagreements: Vec<CountDisagreementView>,
    /// Whether the imported campus reached the recent list.
    ///
    /// `false` means the campus is on disk and correct and only the convenience entry was
    /// not written. Reported rather than raised: failing an import that succeeded, because a
    /// list of recent files could not be updated, would be the wrong answer by a long way.
    pub remembered: bool,
}

impl ImportReportView {
    fn of(report: &ImportReport, remembered: bool) -> Self {
        Self {
            file_name: file_name_of(&report.path),
            path: report.path.to_string_lossy().into_owned(),
            project_id: report.project_id,
            schema_version_found: report.schema_version_found,
            schema_version_now: report.schema_version_now,
            migrated: report.migrated,
            tables_with_rows: report.counts.values().filter(|rows| **rows > 0).count(),
            total_rows: report.counts.values().sum(),
            counts: report.counts.clone(),
            replaced_copy_name: report.replaced_copy_kept_at.as_deref().map(file_name_of),
            replaced_copy_kept_at: report
                .replaced_copy_kept_at
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            unknown_entries: report.unknown_entries.clone(),
            count_disagreements: report
                .count_disagreements
                .iter()
                .map(|disagreement| CountDisagreementView {
                    table: disagreement.table.clone(),
                    declared: disagreement.declared,
                    found: disagreement.found,
                })
                .collect(),
            remembered,
        }
    }
}

/// What an archive says about itself, and whether this build can read it.
///
/// Produced by [`project_import_inspect`], which **writes nothing and stages nothing**. In
/// particular it does not read the payload: streaming and hashing that is step 9 of the
/// reader's verification order and belongs to the import, which is why a damaged payload is
/// caught then — before anything is written — and not here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArchiveSummaryView {
    /// Where the archive is.
    pub path: String,
    /// Its file name.
    pub file_name: String,
    /// How big it is.
    pub archive_bytes: u64,
    /// The container's format name.
    pub format: String,
    /// The container's version — D2's first axis.
    pub format_version: u32,
    /// Which application wrote it.
    pub generator_app: String,
    /// Which version of it.
    pub generator_version: String,
    /// When, ISO-8601 UTC.
    pub exported_at: String,
    /// The identity inside.
    pub project_id: Uuid,
    /// The installation that created the campus, where recorded.
    pub created_by_site_id: Option<Uuid>,
    /// The installation that exported it, where recorded. Never becomes the created-by.
    pub exported_by_site_id: Option<Uuid>,
    /// The payload's schema version — D2's second axis.
    pub schema_version: u32,
    /// How big the campus inside is, uncompressed.
    pub payload_bytes: u64,
    /// The schools it names.
    pub schools: Vec<SchoolSummaryView>,
    /// How many change-feed entries it carries.
    pub change_log_entries: i64,
    /// Where the change feed stood.
    pub change_log_watermark: i64,
    /// Declared row counts, per table.
    pub counts: BTreeMap<String, i64>,
    /// How many tables are declared to carry rows.
    pub tables_with_rows: usize,
    /// How many rows are declared in total.
    pub total_rows: i64,
    /// What a reader must understand — D3.
    pub capabilities: Vec<String>,
    /// Safe entry names this build does not know, which it would ignore.
    pub unknown_entries: Vec<String>,
    /// Whether this build can import it.
    pub readable: bool,
    /// Why not, when it cannot.
    ///
    /// A value rather than a thrown error, because inspecting is how an interface finds out
    /// *whether* it can offer the import — and a question whose "no" is an exception forces
    /// a caller to catch one in order to render a screen.
    pub refusal: Option<ProjectError>,
    /// A name a campus file could sensibly be given, derived from the archive's own.
    pub suggested_file_name: String,
    /// Recent entries already carrying this identity — D13.
    pub identity_conflicts: Vec<RecentProjectView>,
    /// D13's first sentence, when there is a twin on this machine.
    pub identity_notice: Option<String>,
    /// What the digests prove, and what they do not. Always present.
    pub integrity_notice: &'static str,
}

/// What startup recovery did to one interrupted replacement — D11.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecoveredView {
    /// The campus file the interrupted import was aimed at.
    pub destination: String,
    /// Its file name.
    pub file_name: String,
    /// What recovery did, in words a user can relay.
    pub outcome: String,
    /// Whether it was resolved.
    ///
    /// `false` means Evara **changed nothing** and left the record in place so a person or a
    /// newer build can look at it. An ambiguous state is reported and acted on by nothing;
    /// guessing is how the protocol would lose somebody's campus.
    pub resolved: bool,
}

impl From<&evara_project::Recovered> for RecoveredView {
    fn from(recovered: &evara_project::Recovered) -> Self {
        Self {
            file_name: file_name_of(&recovered.destination),
            destination: recovered.destination.to_string_lossy().into_owned(),
            outcome: recovered.outcome.clone(),
            resolved: recovered.resolved,
        }
    }
}

/// What [`project_export_start`] returns.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExportStarted {
    /// The session every later command names.
    pub run_id: Uuid,
    /// How many bytes the campus inside the archive will hold.
    ///
    /// Exact, because the snapshot is already taken by the time this returns — which is also
    /// why the workspace lock is released before the archive is written.
    pub total_bytes: u64,
}

/// What [`project_import_start`] returns.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportStarted {
    /// The session every later command names.
    pub run_id: Uuid,
    /// How many bytes the campus inside the archive holds, as the manifest declares it.
    pub total_bytes: u64,
}

/// How an export is doing.
///
/// `phase` is the **session's** lifecycle, as `GenerateStatus` and `RepairStatus` use the
/// word in this crate; `step` is where the *work* has got to. Two different questions, two
/// different words.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExportStatus {
    /// Which session.
    pub run_id: Uuid,
    /// Where it is in its lifecycle.
    pub phase: TransferPhase,
    /// Which part of the work is running.
    pub step: Phase,
    /// How far that part has got.
    pub done: u64,
    /// How much there is, or `0` when it is not yet known. Never divide by it.
    pub total: u64,
    /// Whether a cancel has been asked for.
    pub cancel_requested: bool,
    /// Where the archive is going.
    pub destination: String,
    /// What it produced, once it has.
    pub report: Option<ExportReportView>,
    /// Why it did not, when it did not.
    pub failure: Option<ProjectError>,
}

/// How an import is doing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportStatus {
    /// Which session.
    pub run_id: Uuid,
    /// Where it is in its lifecycle.
    pub phase: TransferPhase,
    /// Which part of the work is running.
    pub step: Phase,
    /// How far that part has got.
    pub done: u64,
    /// How much there is, or `0` when it is not yet known.
    pub total: u64,
    /// Whether a cancel has been asked for.
    pub cancel_requested: bool,
    /// Which archive is being read.
    pub archive: String,
    /// Which campus file is being written.
    pub destination: String,
    /// What it produced, once it has.
    pub report: Option<ImportReportView>,
    /// Why it did not, when it did not.
    pub failure: Option<ProjectError>,
}

/// A progress event.
///
/// Carries `run_id` **and** `epoch`, so a screen can filter on the session it started and a
/// session belonging to a campus the user has moved away from cannot drive the screen in
/// front of them.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TransferProgressEvent {
    /// Which session.
    pub run_id: Uuid,
    /// Which project.
    pub epoch: u64,
    /// Which direction, so one listener can route both.
    pub kind: TransferKind,
    /// Which part of the work is running.
    pub step: Phase,
    /// How far that part has got.
    pub done: u64,
    /// How much there is, or `0` when it is not yet known.
    pub total: u64,
}

/// Emitted once, when the worker returns.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TransferFinishedEvent {
    /// Which session.
    pub run_id: Uuid,
    /// Which project.
    pub epoch: u64,
    /// Which direction.
    pub kind: TransferKind,
    /// Where the session ended up.
    ///
    /// Beside `ok` rather than instead of it: a cancellation and a failure are both *not* a
    /// success and need completely different words said about them, and a screen that could
    /// only see `ok` would report the user's own Cancel as a fault.
    pub phase: TransferPhase,
    /// Whether there is a report to read.
    pub ok: bool,
}

// =======================================================================================
// The host seam
// =======================================================================================

/// What a transfer worker needs from the application around it.
///
/// A trait rather than a `tauri::AppHandle`, so the whole of an export and an import can be
/// driven in a test with no window, no event loop and no managed state — which is what lets
/// cancellation at each phase, progress ordering, destination conflicts and the recent-list
/// entry be asserted rather than reasoned about.
pub trait TransferHost: Send + Sync + 'static {
    /// Emits a progress event.
    fn emit_progress(&self, event: TransferProgressEvent);
    /// Emits the one terminal event.
    fn emit_finished(&self, event: TransferFinishedEvent);
    /// Records an imported campus in the recent list **without opening it** — D9.
    ///
    /// Returns whether it was recorded. A `false` is reported in the import's own report and
    /// never raised: the campus is on disk and correct, and failing the import because a list
    /// of recent files could not be updated would be the wrong answer by a long way.
    fn remember_imported(&self, path: &Path, project_id: Uuid) -> bool;
}

/// The shipped host: a Tauri handle, and the managed workspace behind it.
pub struct DesktopHost(pub tauri::AppHandle);

impl TransferHost for DesktopHost {
    fn emit_progress(&self, event: TransferProgressEvent) {
        // Ignored deliberately: an event that cannot be delivered means the window has gone,
        // and a transfer must not fail because nobody is listening to it.
        let _ = self.0.emit(PROGRESS_EVENT, event);
    }

    fn emit_finished(&self, event: TransferFinishedEvent) {
        let _ = self.0.emit(FINISHED_EVENT, event);
    }

    fn remember_imported(&self, path: &Path, project_id: Uuid) -> bool {
        // `try_state` rather than `state`: the latter panics when nothing is managed, and a
        // worker finishing during shutdown is not a reason to abort the process.
        let Some(state) = self.0.try_state::<AppState>() else {
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

// =======================================================================================
// 1 — the extension
// =======================================================================================

/// The extension a portable archive must use.
///
/// So the interface builds its file-dialog filter from one source rather than hardcoding the
/// string — and so `.evara` and `.evaraproj` cannot drift apart in a dialog, which is exactly
/// the confusion `CLAUDE.md` §3 forbids.
#[tauri::command]
pub const fn project_archive_extension() -> &'static str {
    ARCHIVE_EXTENSION
}

// =======================================================================================
// 2–5 — export
// =======================================================================================

/// Snapshots the open campus and seals it into an archive on a worker thread.
///
/// Writes the archive and **nothing else**: the campus is read inside a read transaction and
/// its change-log watermark, every row's revision and the file's modification time are
/// unchanged.
///
/// # Errors
///
/// - [`ProjectError::WrongExtension`] unless `path` ends in `.evara`.
/// - [`ProjectError::AlreadyExists`] if something is already there. Evara never overwrites an
///   archive you may still need, and there is deliberately no "overwrite" answer: the remedy
///   is another name, which costs the user nothing and cannot destroy anything.
/// - [`ProjectError::NoProjectOpen`] if no campus is open.
/// - [`ProjectError::DestinationUnsafe`] if the path is a link or a URI.
/// - Anything the snapshot refuses, in which case nothing was written anywhere.
#[tauri::command]
pub fn project_export_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
) -> CommandResult<ExportStarted> {
    export_start(
        &(Arc::new(DesktopHost(app)) as Arc<dyn TransferHost>),
        &state,
        &path,
    )
}

/// The body of [`project_export_start`], against the host seam.
pub(crate) fn export_start(
    host: &Arc<dyn TransferHost>,
    state: &AppState,
    path: &Path,
) -> CommandResult<ExportStarted> {
    evara_project::writer::require_archive_extension(path).map_err(classify)?;

    // Asked first, because it is the more useful of the two answers: "Evara is already
    // writing this" and "something is already here" send the user to different places.
    if state.transfers.is_writing_to(path) {
        return Err(ProjectError::TransferBlocked {
            file_name: file_name_of(path),
            message: format!(
                "Evara is already writing {}. Wait for it to finish, or stop it first.",
                file_name_of(path)
            ),
        });
    }

    // **Claimed, not checked.** Nothing in the protocol refuses an occupied archive path — a
    // destination the user typed is theirs, and the library leaves the policy to its caller —
    // so the refusal is this layer's, and `seal` finishes by renaming its `.part` over
    // whatever is at the name. An `exists()` test would leave seconds between the answer and
    // that rename, and a file created inside that window would be destroyed without anybody
    // being asked. `create_new` is atomic and is the same claim the import's Create path
    // takes, for the same reason.
    let claim = ArchiveClaim::take(path)?;

    let epoch = state.epoch.get();
    let run_id = Uuid::now_v7();
    let session = Arc::new(TransferSession::new(
        run_id,
        TransferKind::Export,
        epoch,
        None,
        path.to_path_buf(),
        0,
        evara_db::now_iso8601(),
    ));
    if !state.transfers.insert(Arc::clone(&session)) || !session.claim_start() {
        return Err(ProjectError::Failed {
            message: "Evara could not start that export. Please try again.".to_owned(),
        });
    }

    // Staging beside the archive: the same volume the archive is going to, so the copy is
    // written where the user has already decided there is room. A `TempDir` so the directory
    // and the copy inside it go away on **every** path out of here, including a `?`.
    let staging = match staging_beside(path) {
        Ok(staging) => staging,
        Err(error) => {
            state.transfers.discard(run_id, TransferKind::Export);
            return Err(error);
        }
    };

    // ------------------------------------------------------------- under the lock, briefly
    let snapshot = {
        let workspace = match lock_state(state) {
            Ok(workspace) => workspace,
            Err(error) => {
                state.transfers.discard(run_id, TransferKind::Export);
                return Err(error);
            }
        };
        let cancel = session.cancellation();
        let life = session.lifecycle();
        let emitter = Arc::clone(host);
        let taken = workspace.snapshot_open_project(staging.path(), &cancel, &mut |page| {
            let work = Work {
                step: Phase::Snapshotting,
                done: u64::from(page.copied_pages),
                total: u64::from(page.total_pages),
            };
            life.report(work);
            emitter.emit_progress(TransferProgressEvent {
                run_id,
                epoch,
                kind: TransferKind::Export,
                step: work.step,
                done: work.done,
                total: work.total,
            });
        });
        match taken {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // The session is forgotten rather than left in `Failed`: the caller never
                // received a `run_id`, so a session it cannot name is unreachable state.
                drop(workspace);
                state.transfers.discard(run_id, TransferKind::Export);
                return Err(error.into());
            }
        }
    };
    // ------------------------------------------- lock released, before a byte is compressed

    let total_bytes = snapshot.byte_len();
    spawn_export(Arc::clone(host), &session, snapshot, staging, claim).inspect_err(|_| {
        state.transfers.discard(run_id, TransferKind::Export);
    })?;

    Ok(ExportStarted {
        run_id,
        total_bytes,
    })
}

/// Moves the owned snapshot to a worker thread and seals it there.
///
/// The workspace lock is already released by the time this is called — D8, and
/// `ARCHITECTURE.md` §3.3's arrangement.
fn spawn_export(
    host: Arc<dyn TransferHost>,
    session: &Arc<TransferSession>,
    snapshot: Snapshot,
    staging: tempfile::TempDir,
    claim: ArchiveClaim,
) -> CommandResult<()> {
    let worker = Arc::clone(session);
    let run_id = session.run_id;
    let epoch = session.epoch;
    let destination = session.destination.clone();

    std::thread::Builder::new()
        .name(format!("evara-export-{run_id}"))
        .spawn(move || {
            let life = worker.lifecycle();
            let cancel = worker.cancellation();
            let emitter = Arc::clone(&host);
            let sink = CoalescingTransferSink::new(Arc::clone(&life), move |work: Work| {
                emitter.emit_progress(TransferProgressEvent {
                    run_id,
                    epoch,
                    kind: TransferKind::Export,
                    step: work.step,
                    done: work.done,
                    total: work.total,
                });
            });

            let result = evara_project::seal(snapshot, &destination, &sink, &cancel);
            // The staging directory and the snapshot inside it go away here, after `seal`
            // has consumed the copy and not before.
            drop(staging);
            // The placeholder has been replaced by `seal`'s own rename, so it is no longer
            // ours to remove. On every other path it is dropped, which removes it — a
            // cancelled or failed export leaves the folder exactly as it found it.
            if result.is_ok() {
                claim.released();
            } else {
                drop(claim);
            }
            settle(
                &worker,
                &*host,
                result.map(|report| {
                    TransferOutcome::Exported(Box::new(ExportReportView::of(&report)))
                }),
            );
        })
        .map_err(|error| ProjectError::Failed {
            message: format!("Evara could not start the export: {error}"),
        })?;
    Ok(())
}

/// How an export is doing.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process no longer holds that export session — including
/// when the identifier names an *import*, which this command may not read.
#[tauri::command]
pub fn project_export_status(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> CommandResult<ExportStatus> {
    export_status(&state, run_id)
}

/// The body of [`project_export_status`].
pub(crate) fn export_status(state: &AppState, run_id: Uuid) -> CommandResult<ExportStatus> {
    let session = require(state, run_id, TransferKind::Export)?;
    let work = session.work();
    let report = match session.outcome() {
        Some(TransferOutcome::Exported(report)) => Some(*report),
        _ => None,
    };
    Ok(ExportStatus {
        run_id,
        phase: session.phase(),
        step: work.step,
        done: work.done,
        total: if work.total == 0 {
            session.total_bytes
        } else {
            work.total
        },
        cancel_requested: session.cancel_requested(),
        destination: session.destination.to_string_lossy().into_owned(),
        report,
        failure: session.failure(),
    })
}

/// Asks an export to stop.
///
/// Returns whether something was still running. `false` is not an error: there was nothing to
/// stop, and reporting success would tell the user their cancel did something it did not.
///
/// Sets a flag and **no state**. A cancelled export leaves no archive and no partial file —
/// that is the protocol's guarantee, not this command's.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process holds no export with that identifier.
#[tauri::command]
pub fn project_export_cancel(state: State<'_, AppState>, run_id: Uuid) -> CommandResult<bool> {
    export_cancel(&state, run_id)
}

/// The body of [`project_export_cancel`].
pub(crate) fn export_cancel(state: &AppState, run_id: Uuid) -> CommandResult<bool> {
    Ok(require(state, run_id, TransferKind::Export)?.request_cancel())
}

/// Forgets an export session, stopping it if it is still going.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process holds no export with that identifier.
#[tauri::command]
pub fn project_export_discard(state: State<'_, AppState>, run_id: Uuid) -> CommandResult<()> {
    export_discard(&state, run_id)
}

/// The body of [`project_export_discard`].
pub(crate) fn export_discard(state: &AppState, run_id: Uuid) -> CommandResult<()> {
    if state.transfers.discard(run_id, TransferKind::Export) {
        Ok(())
    } else {
        Err(unknown_run(TransferKind::Export))
    }
}

// =======================================================================================
// 6–10 — import
// =======================================================================================

/// What an archive says about itself. **Writes nothing, stages nothing, imports nothing.**
///
/// Steps 1–8 of the reader's verification order, and not step 9: no byte of the payload is
/// streamed and no file is created. A version or capability problem is **not** an error here
/// — it comes back as [`ArchiveSummaryView::refusal`], so a screen can say what is wrong
/// without catching anything.
///
/// A clean inspection is **not** proof the payload is intact. The payload is verified against
/// the archive's own digest record during the import, which is where a damaged one is caught
/// and refused before anything is written.
///
/// # Errors
///
/// Everything in steps 1–6: not an archive, no manifest, a manifest that will not parse, a
/// digest record that disagrees with the manifest.
#[tauri::command]
pub fn project_import_inspect(
    state: State<'_, AppState>,
    path: PathBuf,
) -> CommandResult<ArchiveSummaryView> {
    let summary = evara_project::inspect(&path).map_err(classify)?;
    // The recent list is read **after** the archive, so a failure to read an unreadable
    // archive costs nothing and the lock is taken only when there is an identity to ask
    // about.
    let conflicts = lock(&state)?
        .projects_with_identity(summary.manifest.project_id)
        .map_err(ProjectError::from)?;
    Ok(summarise(&path, &summary, &conflicts))
}

/// Assembles the view. Separated so the D13 wording is reachable without an archive on disk.
pub(crate) fn summarise(
    path: &Path,
    summary: &ArchiveSummary,
    conflicts: &[RecentProject],
) -> ArchiveSummaryView {
    let manifest = &summary.manifest;
    ArchiveSummaryView {
        file_name: file_name_of(path),
        path: path.to_string_lossy().into_owned(),
        archive_bytes: summary.archive_bytes,
        format: manifest.format.clone(),
        format_version: manifest.format_version,
        generator_app: manifest.generator.app.clone(),
        generator_version: manifest.generator.version.clone(),
        exported_at: manifest.exported_at.clone(),
        project_id: manifest.project_id,
        created_by_site_id: manifest.created_by_site_id,
        exported_by_site_id: manifest.exported_by_site_id,
        schema_version: manifest.payload.schema_version,
        payload_bytes: manifest.payload.byte_length,
        schools: manifest.schools.iter().map(Into::into).collect(),
        change_log_entries: manifest.change_log.entries,
        change_log_watermark: manifest.change_log.watermark,
        tables_with_rows: manifest.counts.values().filter(|rows| **rows > 0).count(),
        total_rows: manifest.counts.values().sum(),
        counts: manifest.counts.clone(),
        capabilities: manifest.capabilities.clone(),
        unknown_entries: summary.unknown_entries.clone(),
        readable: summary.readable.is_readable(),
        refusal: readability_refusal(&summary.readable),
        suggested_file_name: suggested_project_name(path),
        identity_notice: identity_notice(conflicts),
        identity_conflicts: conflicts.iter().cloned().map(Into::into).collect(),
        integrity_notice: INTEGRITY_NOTICE,
    }
}

/// Reads an archive into a campus file, on a worker thread.
///
/// Writes the new campus and **nothing else**. It does not open what it imported — D9 — and
/// it does not disturb whatever campus is open.
///
/// # Errors
///
/// - [`ProjectError::WrongExtension`] unless `destination_path` ends in `.evaraproj`.
///   `.evara` as a destination is refused by the same check, which is the point.
/// - [`ProjectError::DestinationIsOpen`] if it is the campus this window has open — D13.
/// - [`ProjectError::AlreadyExists`] with `"refuse"`, naming the file. An existing
///   **zero-length** file counts as occupied, deliberately unlike creating a project: adopting
///   an unknown empty file would be writing over something nobody has looked at.
/// - [`ProjectError::FormatTooNew`], [`ProjectError::SchemaTooNew`],
///   [`ProjectError::UnsupportedCapability`] — before a byte is staged.
/// - [`ProjectError::ArchiveDamaged`] for a bad digest, a short payload, or a campus inside
///   that is corrupt, non-conforming or referentially broken.
#[tauri::command]
pub fn project_import_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    archive_path: PathBuf,
    destination_path: PathBuf,
    on_conflict: OnConflict,
) -> CommandResult<ImportStarted> {
    import_start(
        &(Arc::new(DesktopHost(app)) as Arc<dyn TransferHost>),
        &state,
        &archive_path,
        &destination_path,
        on_conflict,
    )
}

/// The body of [`project_import_start`], against the host seam.
pub(crate) fn import_start(
    host: &Arc<dyn TransferHost>,
    state: &AppState,
    archive_path: &Path,
    destination_path: &Path,
    on_conflict: OnConflict,
) -> CommandResult<ImportStarted> {
    evara_project::import::require_project_extension(destination_path).map_err(classify)?;

    // Read the header first. It writes nothing, and it is what turns "this build cannot use
    // this archive" into a refusal before any destination has been touched.
    let summary = evara_project::inspect(archive_path).map_err(classify)?;
    if let Some(refusal) = readability_refusal(&summary.readable) {
        return Err(refusal);
    }

    let (settings_path, site_id, conflicts) = {
        let workspace = lock_state(state)?;
        // D13 case 3. Replacing a file SQLite still holds open is how a campus ends up
        // paired with a live `-wal` belonging to a database that no longer exists.
        if workspace.is_open_project(destination_path) {
            return Err(ProjectError::DestinationIsOpen {
                file_name: file_name_of(destination_path),
                message: format!(
                    "{} is open. Close it before replacing it.",
                    file_name_of(destination_path)
                ),
            });
        }
        (
            workspace.settings_path().to_path_buf(),
            workspace.site_id(),
            workspace
                .projects_with_identity(summary.manifest.project_id)
                .map_err(ProjectError::from)?,
        )
    };

    // Refused here as well as inside the protocol, so the sentence a user reads names the
    // campus they are about to replace rather than only the file — D13's second sentence.
    // This is **wording, not the guard**: the atomic one is the protocol's `create_new`
    // claim, so a file that appears between this answer and that claim is still refused
    // rather than destroyed, with the plainer sentence.
    if on_conflict == OnConflict::Refuse && destination_path.symlink_metadata().is_ok() {
        return Err(ProjectError::AlreadyExists {
            message: replacement_notice(destination_path, &conflicts),
        });
    }
    if state.transfers.is_writing_to(destination_path) {
        return Err(ProjectError::TransferBlocked {
            file_name: file_name_of(destination_path),
            message: format!(
                "Evara is already writing {}. Wait for it to finish, or stop it first.",
                file_name_of(destination_path)
            ),
        });
    }

    let epoch = state.epoch.get();
    let run_id = Uuid::now_v7();
    let session = Arc::new(TransferSession::new(
        run_id,
        TransferKind::Import,
        epoch,
        Some(archive_path.to_path_buf()),
        destination_path.to_path_buf(),
        summary.manifest.payload.byte_length,
        evara_db::now_iso8601(),
    ));
    if !state.transfers.insert(Arc::clone(&session)) || !session.claim_start() {
        return Err(ProjectError::Failed {
            message: "Evara could not start that import. Please try again.".to_owned(),
        });
    }

    let total_bytes = session.total_bytes;
    spawn_import(
        Arc::clone(host),
        &session,
        ImportWork {
            archive: archive_path.to_path_buf(),
            destination: destination_path.to_path_buf(),
            on_conflict,
            settings_path,
            site_id,
        },
    )
    .inspect_err(|_| {
        state.transfers.discard(run_id, TransferKind::Import);
    })?;

    Ok(ImportStarted {
        run_id,
        total_bytes,
    })
}

/// What the import worker needs that is not the session.
struct ImportWork {
    archive: PathBuf,
    destination: PathBuf,
    on_conflict: OnConflict,
    settings_path: PathBuf,
    site_id: Uuid,
}

/// Runs the import protocol on a worker thread.
///
/// The worker opens **its own** handle on the installation's settings database, because the
/// replacement journal is needed for the whole of the import and the workspace lock may not
/// be held for that long. One file, two short-lived handles, serialised by SQLite's own busy
/// timeout — the arrangement `evara-cli` already has.
fn spawn_import(
    host: Arc<dyn TransferHost>,
    session: &Arc<TransferSession>,
    work: ImportWork,
) -> CommandResult<()> {
    let worker = Arc::clone(session);
    let run_id = session.run_id;
    let epoch = session.epoch;

    std::thread::Builder::new()
        .name(format!("evara-import-{run_id}"))
        .spawn(move || {
            let life = worker.lifecycle();
            let cancel = worker.cancellation();
            let emitter = Arc::clone(&host);
            let sink = CoalescingTransferSink::new(Arc::clone(&life), move |progress: Work| {
                emitter.emit_progress(TransferProgressEvent {
                    run_id,
                    epoch,
                    kind: TransferKind::Import,
                    step: progress.step,
                    done: progress.done,
                    total: progress.total,
                });
            });

            let result = evara_db::Settings::open_at(&work.settings_path)
                .map_err(TransferError::Database)
                .and_then(|mut journal| {
                    evara_project::import(
                        &work.archive,
                        &work.destination,
                        work.on_conflict,
                        Importer {
                            app_version: VERSION,
                            site_id: work.site_id,
                        },
                        &mut journal,
                        &sink,
                        &cancel,
                    )
                });

            let outcome = result.map(|report| {
                // D9: remembered, never opened. The handle above is dropped by now, so the
                // workspace's own settings connection is the only one writing this row.
                let remembered = host.remember_imported(&report.path, report.project_id);
                TransferOutcome::Imported(Box::new(ImportReportView::of(&report, remembered)))
            });
            settle(&worker, &*host, outcome);
        })
        .map_err(|error| ProjectError::Failed {
            message: format!("Evara could not start the import: {error}"),
        })?;
    Ok(())
}

/// How an import is doing.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process no longer holds that import session — including
/// when the identifier names an *export*.
#[tauri::command]
pub fn project_import_status(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> CommandResult<ImportStatus> {
    import_status(&state, run_id)
}

/// The body of [`project_import_status`].
pub(crate) fn import_status(state: &AppState, run_id: Uuid) -> CommandResult<ImportStatus> {
    let session = require(state, run_id, TransferKind::Import)?;
    let work = session.work();
    let report = match session.outcome() {
        Some(TransferOutcome::Imported(report)) => Some(*report),
        _ => None,
    };
    Ok(ImportStatus {
        run_id,
        phase: session.phase(),
        step: work.step,
        done: work.done,
        total: if work.total == 0 {
            session.total_bytes
        } else {
            work.total
        },
        cancel_requested: session.cancel_requested(),
        archive: session
            .source
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        destination: session.destination.to_string_lossy().into_owned(),
        report,
        failure: session.failure(),
    })
}

/// Asks an import to stop.
///
/// Returns whether something was still running. A cancelled import leaves no campus file and
/// no staging — the protocol's guarantee, not this command's.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process holds no import with that identifier.
#[tauri::command]
pub fn project_import_cancel(state: State<'_, AppState>, run_id: Uuid) -> CommandResult<bool> {
    import_cancel(&state, run_id)
}

/// The body of [`project_import_cancel`].
pub(crate) fn import_cancel(state: &AppState, run_id: Uuid) -> CommandResult<bool> {
    Ok(require(state, run_id, TransferKind::Import)?.request_cancel())
}

/// Forgets an import session, stopping it if it is still going.
///
/// # Errors
///
/// [`ProjectError::NotFound`] if this process holds no import with that identifier.
#[tauri::command]
pub fn project_import_discard(state: State<'_, AppState>, run_id: Uuid) -> CommandResult<()> {
    import_discard(&state, run_id)
}

/// The body of [`project_import_discard`].
pub(crate) fn import_discard(state: &AppState, run_id: Uuid) -> CommandResult<()> {
    if state.transfers.discard(run_id, TransferKind::Import) {
        Ok(())
    } else {
        Err(unknown_run(TransferKind::Import))
    }
}

// =======================================================================================
// 11 — recovery
// =======================================================================================

/// What startup recovery did to interrupted replacements — D11.
///
/// Reads back a decision taken **before any project was open**, because the campus that needs
/// recovering may be the one that will not open. Running it here instead would be too late
/// and would run it again on every call.
///
/// An entry with `resolved: false` means Evara **changed nothing**: the state was ambiguous,
/// and guessing is how this protocol would lose somebody's campus. The record is left in
/// place so a person or a newer build can look at it.
///
/// # Errors
///
/// None in practice; the signature stays `Result` so the frontend's error handling is uniform
/// across the eleven.
#[tauri::command]
#[allow(clippy::unnecessary_wraps)]
pub fn project_recovery_report(state: State<'_, AppState>) -> CommandResult<Vec<RecoveredView>> {
    Ok(state.recovery.clone())
}

/// Completes or rolls back every interrupted replacement, at startup.
///
/// Called once by `crate::run` before the window exists. Reports abandoned staging alongside
/// it, which is **never deleted**: one may be another installation's work in progress.
///
/// Failure is reported as a row rather than raised, because this runs before anything the
/// user asked for and a settings database that cannot be read must not stop the application
/// from starting — the user still needs to be able to open a campus and be told why.
#[must_use]
pub fn recover_at_startup(settings_path: &Path) -> Vec<RecoveredView> {
    let mut settings = match evara_db::Settings::open_at(settings_path) {
        Ok(settings) => settings,
        Err(error) => return vec![startup_problem(&error.to_string())],
    };
    match evara_project::recover_pending(&mut settings) {
        Ok(recovered) => recovered.iter().map(Into::into).collect(),
        Err(error) => vec![startup_problem(&error.to_string())],
    }
}

/// A recovery row standing for "recovery itself could not run".
fn startup_problem(detail: &str) -> RecoveredView {
    RecoveredView {
        destination: String::new(),
        file_name: String::new(),
        outcome: format!(
            "Evara could not check for interrupted imports when it started ({detail}). \
             Nothing was changed."
        ),
        resolved: false,
    }
}

// =======================================================================================
// Shared
// =======================================================================================

/// Records what a worker produced and emits the one terminal event.
///
/// One place, so a cancellation is never reported as a failure and a failure is never
/// reported as a cancellation. [`TransferError::Cancelled`] is the protocol saying *you asked
/// me to stop and I wrote nothing*; everything else is a reason the user may need to act on.
fn settle(
    session: &Arc<TransferSession>,
    host: &dyn TransferHost,
    result: Result<TransferOutcome, TransferError>,
) {
    let life = session.lifecycle();
    match result {
        Ok(outcome) => life.finish(outcome),
        Err(TransferError::Cancelled) => life.cancelled(),
        Err(error) => life.fail(classify(error)),
    }

    let phase = session.phase();
    host.emit_finished(TransferFinishedEvent {
        run_id: session.run_id,
        epoch: session.epoch,
        kind: session.kind,
        phase,
        ok: phase == TransferPhase::Finished,
    });
}

/// One session of the kind the caller named, or a refusal.
fn require(
    state: &AppState,
    run_id: Uuid,
    kind: TransferKind,
) -> CommandResult<Arc<TransferSession>> {
    state
        .transfers
        .get_of_kind(run_id, kind)
        .ok_or_else(|| unknown_run(kind))
}

/// The refusal for an identifier this process does not hold **as that kind**.
///
/// One sentence for both cases on purpose. "That export has been forgotten" and "that
/// identifier is an import" lead to the same place — the screen has lost track of a session —
/// and distinguishing them would hand a caller a way to probe which identifiers exist.
fn unknown_run(kind: TransferKind) -> ProjectError {
    ProjectError::NotFound {
        message: format!(
            "Evara is no longer tracking that {}. It may have finished, been stopped, or \
             belonged to a campus that is no longer open.",
            kind.as_str()
        ),
    }
}

/// The zero-length placeholder that holds an archive destination's name.
///
/// `create_new(true)` is the claim, and it is atomic: two callers racing for one name have
/// exactly one winner, and a file somebody else creates during the export is never silently
/// replaced by it. The same device `evara_project::import`'s Create path uses, for the same
/// reason — and the only atomic claim available without FFI, since `unsafe_code` is forbidden
/// workspace-wide.
///
/// The file is removed on drop unless it was released, so a refusal, a failure or a
/// cancellation after the claim leaves nothing behind. It is only ever a zero-length file
/// Evara made moments ago, so removing it destroys nothing.
#[derive(Debug)]
struct ArchiveClaim {
    path: PathBuf,
    released: bool,
}

impl ArchiveClaim {
    /// Claims `destination`, refusing if anything is already there.
    fn take(destination: &Path) -> CommandResult<Self> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => ProjectError::AlreadyExists {
                    message: format!(
                        "{} already exists. Choose another name — Evara will not overwrite \
                         a portable file you may still need.",
                        file_name_of(destination)
                    ),
                },
                _ => ProjectError::Failed {
                    message: format!(
                        "Evara could not write to {}: {error}",
                        file_name_of(destination)
                    ),
                },
            })?;
        Ok(Self {
            path: destination.to_path_buf(),
            released: false,
        })
    }

    /// The archive replaced it, so it is no longer ours to remove.
    ///
    /// Consumes the claim and sets the flag its `Drop` reads — the idiom `Snapshot` and the
    /// `.part` file use, so the cleanup stays structural and the release is a single named
    /// call rather than a flag a caller might forget.
    fn released(mut self) {
        self.released = true;
    }
}

impl Drop for ArchiveClaim {
    fn drop(&mut self) {
        if !self.released {
            // A placeholder left behind would read as an empty portable file in a folder
            // listing, which is worse than nothing at all.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A staging directory beside `destination`, on the same volume.
fn staging_beside(destination: &Path) -> CommandResult<tempfile::TempDir> {
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty());
    let made = match parent {
        Some(parent) => tempfile::Builder::new()
            .prefix(".evara-export-")
            .tempdir_in(parent),
        None => tempfile::Builder::new().prefix(".evara-export-").tempdir(),
    };
    made.map_err(|error| ProjectError::Failed {
        message: format!("Evara could not prepare a working folder for the export: {error}"),
    })
}

/// Turns a protocol error into the tagged refusal the interface branches on.
///
/// `Database` is forwarded to `From<DbError>` rather than classified a second time, so there
/// is exactly **one** place a database failure becomes a `ProjectError` and the two cannot
/// drift into saying different things about one archive.
pub(crate) fn classify(error: TransferError) -> ProjectError {
    match error {
        // Not an Evara archive at all, or one this build has never heard of.
        TransferError::NotAnArchive { .. }
        | TransferError::ManifestMissing { .. }
        | TransferError::ManifestUnreadable { .. }
        | TransferError::UnknownFormat { .. } => ProjectError::ArchiveUnreadable {
            message: format!("{error}"),
        },

        // D2's first axis.
        TransferError::FormatTooNew { found, supported } => ProjectError::FormatTooNew {
            found,
            supported,
            message: format!(
                "This portable campus file was written by a newer version of Evara \
                 (file format v{found}; this version reads up to v{supported}). Update \
                 Evara to import it."
            ),
        },

        // D3: a rule this reader cannot evaluate. Named, and never a row quietly dropped.
        TransferError::UnsupportedCapability { capability } => {
            ProjectError::UnsupportedCapability {
                message: format!(
                    "This campus uses something this version of Evara does not understand \
                     ({capability}), so importing it would leave out part of the timetable. \
                     Update Evara to import it."
                ),
                capability,
            }
        }

        // Everything the integrity model is for. One remedy, and it is always available.
        TransferError::ManifestDisagreesWithPayload { .. }
        | TransferError::UnsupportedDigest { .. }
        | TransferError::EntryMissing { .. }
        | TransferError::UnsafeEntryName { .. }
        | TransferError::UnsupportedCompression { .. }
        | TransferError::EntryEncrypted { .. }
        | TransferError::EntryTooLarge { .. }
        | TransferError::EntryExpandsTooFar { .. }
        | TransferError::TooManyEntries { .. }
        | TransferError::DuplicateEntryName { .. }
        | TransferError::EntryNotAFile { .. }
        | TransferError::PayloadLengthMismatch { .. }
        | TransferError::ChecksumMismatch { .. }
        | TransferError::Truncated => ProjectError::ArchiveDamaged {
            message: format!("{error} Ask whoever sent it for a fresh copy."),
        },

        TransferError::DestinationExists { ref name } => ProjectError::AlreadyExists {
            message: format!(
                "{name} already exists. Choose another name, or replace it — Evara keeps \
                 your current copy either way."
            ),
        },
        TransferError::WrongExtension { expected } => ProjectError::WrongExtension {
            expected: expected.to_owned(),
            message: format!("An Evara campus must use the {expected} extension."),
        },
        TransferError::DestinationUnsafe { ref name, .. } => ProjectError::DestinationUnsafe {
            file_name: name.clone(),
            message: format!("{error}"),
        },
        TransferError::RenameBlocked { ref name }
        | TransferError::ReplacementInFlight { ref name } => ProjectError::TransferBlocked {
            file_name: name.clone(),
            message: format!("{error}"),
        },
        TransferError::Cancelled => ProjectError::Cancelled {
            message: "That was stopped before it finished. Nothing was written.".to_owned(),
        },

        // One mapping for a database failure, and it is the existing one.
        TransferError::Database(error) => ProjectError::from(error),

        // `TransferError` is `#[non_exhaustive]`, so this arm is required whatever is listed
        // above it. What it means for a **new** variant is deliberate: it defaults to a
        // generic failure rather than to a judgement about the archive, which is the safer
        // mistake. Claiming Evara looked at an archive and decided against it, for an error
        // nobody has classified yet, would put words in the product's mouth.
        _ => ProjectError::Failed {
            message: format!("{error}"),
        },
    }
}

/// The refusal a readability would produce, translated.
fn readability_refusal(readable: &Readability) -> Option<ProjectError> {
    readable.as_refusal().map(classify)
}

/// D13's first sentence: a twin of this campus is already on this machine.
///
/// `None` when there is no twin, which is the ordinary case. Named here rather than assembled
/// in React for the reason every other message in this crate is: one source of wording, and
/// `evara-db` calls it a project while the interface calls it a **campus**.
pub(crate) fn identity_notice(conflicts: &[RecentProject]) -> Option<String> {
    let first = conflicts.first()?;
    let folder = first
        .path
        .parent()
        .and_then(|parent| parent.file_name())
        .map_or_else(
            || String::from("another folder"),
            |name| name.to_string_lossy().into_owned(),
        );
    Some(format!(
        "A campus with this identity is already on this machine: {} in {folder}. Importing \
         creates a second copy. Evara cannot merge two copies, so changes you make in one \
         will not appear in the other.",
        file_name_of(&first.path)
    ))
}

/// D13's second sentence, or the plain one when the destination is a different campus.
///
/// The sharper wording is used only when the recent list says the destination **is** this same
/// campus. That is read from the recent list and from nothing else — deciding what is at a
/// path must not change it, so no file is opened to find out.
pub(crate) fn replacement_notice(destination: &Path, conflicts: &[RecentProject]) -> String {
    let name = file_name_of(destination);
    let same_campus = conflicts
        .iter()
        .any(|entry| entry.path.as_path() == destination);

    if same_campus {
        format!(
            "This replaces {name}, which is the same campus. Your current copy is kept \
             beside it as {}.",
            keep_name_pattern(destination)
        )
    } else {
        format!(
            "{name} already exists. Choose another name, or replace it — your current copy \
             is kept beside it as {}.",
            keep_name_pattern(destination)
        )
    }
}

/// The shape of the name a replaced campus is kept under.
///
/// The stamp is chosen by the protocol at the moment it moves the file, so a literal one here
/// would be a guess. The pattern is shown instead, with the real stem, because the part that
/// matters to the user is *where their current copy goes* rather than which second it was.
fn keep_name_pattern(destination: &Path) -> String {
    let stem = destination.file_stem().map_or_else(
        || String::from("campus"),
        |s| s.to_string_lossy().into_owned(),
    );
    format!(
        "{stem}.replaced-<date and time>.{}",
        evara_db::PROJECT_EXTENSION
    )
}

/// A campus file name an archive could sensibly be imported as.
fn suggested_project_name(archive: &Path) -> String {
    let stem = archive.file_stem().map_or_else(
        || String::from("Campus"),
        |s| s.to_string_lossy().into_owned(),
    );
    format!("{stem}.{}", evara_db::PROJECT_EXTENSION)
}

/// A path's file name, or a phrase that reads correctly in a sentence when it has none.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("that file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Builds the workspace accessor the import worker's recent-list write needs.
///
/// Exists so [`DesktopHost`] is not the only implementation and the trait object in
/// [`export_start`] has one construction site in the shipped application.
impl From<tauri::AppHandle> for DesktopHost {
    fn from(app: tauri::AppHandle) -> Self {
        Self(app)
    }
}

#[cfg(test)]
mod tests;
