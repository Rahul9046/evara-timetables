//! A consistent, owned copy of a project, taken with the lock held for as little as
//! possible.
//!
//! # One mechanism, and why it is not the two the proposal specified
//!
//! A project runs in WAL, so its committed state is split between the `.evaraproj` file and
//! its `-wal` sidecar. Copying the main file is therefore wrong in exactly the case that
//! matters: it yields a database that opens cleanly and silently lacks the most recent work.
//! Whatever mechanism is used must produce a **fully checkpointed single file**.
//!
//! The approved design (D8) specified two: `VACUUM INTO` from a read-write handle, and the
//! online backup API from a `query_only` one — the latter because `VACUUM INTO` is
//! **refused** under `query_only` (T-75), which left no choice on that path.
//!
//! **This implementation uses the backup API for both,** and the reason is cancellation
//! rather than preference. `VACUUM INTO` is a single opaque statement, so the only way to
//! interrupt it is SQLite's progress handler — whose closure rusqlite requires to be
//! `Send + 'static`. A borrowed `&dyn Cancel` is neither. Honouring D8 literally would
//! therefore have produced a desktop export that **cannot be cancelled**, which is the one
//! path where a user is most likely to want to, and the proposal asks for consistency *and*
//! cancellation together.
//!
//! The alternatives were weighed and rejected: widening the public signature to
//! `&(dyn Cancel + Sync)` and spawning a watcher thread to call `interrupt()` would add a
//! thread and a poll interval to every export to preserve a mechanism split that buys
//! nothing else; and shipping an uncancellable desktop path would be a silent downgrade of
//! what was asked for.
//!
//! **What is given up, stated plainly:** `VACUUM INTO` compacts as it copies, and the backup
//! API does not — it reproduces the source page for page, free pages included. So a snapshot
//! of a heavily edited project is larger than a vacuumed one would be. It is still a valid,
//! complete, single-file database; the cost is bytes, and `.evara` compresses them. Stage 2
//! measures §7's disk budget and can add a compaction step if the measurement justifies one.
//! This is recorded as a deviation rather than folded in silently.
//!
//! # Consistency
//!
//! The backup path runs inside an explicit `BEGIN DEFERRED` whose first read pins a WAL
//! snapshot, so the copy is of one coherent instant even though it is assembled over many
//! steps. Without that, SQLite restarts the backup whenever it notices the source changing
//! and the copy would follow a concurrent writer instead of holding still. **T-77 is the
//! test that proves it**, and it is the single most important test in this module: the
//! failure mode is not a crash but a plausible, internally consistent, *wrong* export.
//!
//! `VACUUM INTO` takes its own read transaction for the duration and needs no help.
//!
//! # Every fact comes from the copy, never from the source
//!
//! [`Snapshot`] carries every fact it will be described by — identity, schema version,
//! census, change feed, schools, capability evidence — and reads all of them **out of the
//! file it just wrote**. Reading them from
//! the live project instead would describe something that was true a moment earlier, and the
//! whole point of a snapshot is that one instant is being captured. This is finding 4 of the
//! architecture review and it is why the facts live on this type rather than being read
//! later by whoever seals the archive.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use evara_core::scheduling::Cancel;
use rusqlite::Connection;

use crate::error::{DbError, InternalError, Result};
use crate::{Database, connection, meta};

/// How far a snapshot has got.
///
/// A plain value passed to a plain callback. Deliberately **not** `evara-solver`'s
/// `ProgressSink`: that trait's contract is a solve's — placements of a total, after so many
/// steps — and a page copy is not that shape. Reusing it would have meant moving a second
/// trait into `evara-core` for one caller, which is the opposite of the reason [`Cancel`]
/// moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotProgress {
    /// Pages written so far.
    pub copied_pages: u32,
    /// Pages in total, as the engine reports them.
    pub total_pages: u32,
}

impl SnapshotProgress {
    /// Progress as a fraction.
    ///
    /// `None` only before the first step, when the engine has not yet reported a page
    /// count. A caller must not divide by `total_pages` itself.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        (self.total_pages > 0).then(|| f64::from(self.copied_pages) / f64::from(self.total_pages))
    }
}

/// A complete, verified copy of a project, owned by whoever holds this value.
///
/// # Ownership is the point
///
/// `Database::snapshot_into` returns this with the workspace lock **released**. It is
/// [`Send`], so the command thread can hand it to a worker that compresses and seals it
/// while the interface stays live — the same arrangement `evara_db::scheduling`'s `Assembly`
/// has, and asserted the same way.
///
/// # It removes its own file
///
/// Dropping a `Snapshot` deletes the file unless [`Snapshot::keep`] was called. A cancelled
/// export, a failed verification, or an early `?` therefore leaves nothing behind — which is
/// the same defect `backup.rs` had and the reason this is structural rather than remembered
/// at each call site.
#[derive(Debug)]
pub struct Snapshot {
    path: PathBuf,
    pages: u32,
    facts: super::facts::ProjectFacts,
    byte_len: u64,
    keep: bool,
}

impl Snapshot {
    /// Where the copy is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many pages were copied.
    #[must_use]
    pub const fn pages(&self) -> u32 {
        self.pages
    }

    /// Schema version of the copy, read from the copy.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.facts.schema_version
    }

    /// Project identity of the copy, read from the copy.
    ///
    /// Never regenerated. ADR 0004's rule: a transferred project is the same project, and a
    /// new identifier would destroy round-trip comparison and any future synchronisation.
    #[must_use]
    pub const fn identity(&self) -> meta::ProjectIdentity {
        self.facts.identity
    }

    /// Row counts per table, read from the copy.
    ///
    /// Discovered from `sqlite_master` rather than from a hand-written list, so a table
    /// added by a future migration is counted without anyone remembering to add it here.
    #[must_use]
    pub const fn census(&self) -> &BTreeMap<String, i64> {
        &self.facts.census
    }

    /// Every fact the manifest describing this snapshot carries.
    ///
    /// Read **from the copy**, on the worker thread, after the workspace lock was released.
    /// Finding 4 of the architecture review: reading them from the live project instead would
    /// let the manifest describe an instant the payload does not hold.
    #[must_use]
    pub const fn facts(&self) -> &super::facts::ProjectFacts {
        &self.facts
    }

    /// Highest `change_log.seq` in the copy.
    ///
    /// The instant the snapshot represents, expressed in the project's own terms.
    #[must_use]
    pub const fn watermark(&self) -> i64 {
        self.facts.change_log.watermark
    }

    /// Size of the copy on disk.
    #[must_use]
    pub const fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Hands the file to the caller, so dropping this no longer removes it.
    ///
    /// Returns the path. Call this once the bytes have been sealed into something durable;
    /// until then the automatic cleanup is what keeps a failure from leaving litter.
    #[must_use]
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        if !self.keep {
            // Ignored deliberately: this runs on the failure path, and a cleanup error
            // would replace a useful message with a confusing one.
            remove_with_sidecars(&self.path);
        }
    }
}

/// Discards every progress report. For callers that have nowhere to put one.
pub fn ignore_progress(_: SnapshotProgress) {}

/// Removes a snapshot file and any sidecar beside it.
///
/// The destination connection is closed before anything reads the copy, so a completed
/// snapshot is a single file — asserted by `a_snapshot_is_a_single_file`. A copy abandoned
/// *mid-step* has not been closed, so its `-wal` and `-shm` may still exist, and leaving
/// them behind would be litter that S3's sidecar assertions later trip over.
fn remove_with_sidecars(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(super::sidecars::sidecar_path(path, suffix));
    }
}

impl Database {
    /// Writes a consistent copy of this project into `directory` and describes it.
    ///
    /// The returned [`Snapshot`] owns the file and removes it when dropped, so a caller that
    /// fails partway leaves nothing behind. Call [`Snapshot::keep`] to adopt it.
    ///
    /// One mechanism serves both a read-write and a `query_only` handle — see this module's
    /// header for why, and for what that costs.
    ///
    /// # Cancellation
    ///
    /// `cancel` is asked before the copy begins and between every page batch. A cancelled
    /// snapshot returns [`DbError::SnapshotCancelled`] and has removed its partial file.
    ///
    /// # Errors
    ///
    /// - [`DbError::SnapshotCancelled`] if the caller stopped it.
    /// - [`DbError::BackupRefused`] if the copy could not be written or did not verify.
    /// - [`DbError::Internal`] for a SQLite failure with no better name.
    pub fn snapshot_into(
        &self,
        directory: &Path,
        cancel: &dyn Cancel,
        progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<Snapshot> {
        let target = unique_target(directory)?;

        // Refused before a single page is copied. A caller that cancelled while queuing has
        // not asked for a file to appear and then vanish.
        if cancel.is_cancelled() {
            return Err(DbError::SnapshotCancelled);
        }

        let pages = match self.backup_into(&target, cancel, progress) {
            Ok(pages) => pages,
            Err(error) => {
                // Every failure path converges here, so cleanup is in one place rather
                // than at each `?`. The `Snapshot` has not been constructed yet, so its
                // `Drop` cannot do this for us.
                remove_with_sidecars(&target);
                return Err(error);
            }
        };

        match describe(&target, pages) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                remove_with_sidecars(&target);
                Err(error)
            }
        }
    }

    /// The read-only mechanism: stepped, inside a pinned read transaction.
    fn backup_into(
        &self,
        target: &Path,
        cancel: &dyn Cancel,
        progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<u32> {
        /// Pages per step. Small enough that cancellation is observed promptly, large
        /// enough that the per-step overhead stays negligible on a big project.
        const PAGES_PER_STEP: i32 = 256;

        let mut destination =
            Connection::open(target).map_err(|source| DbError::opening(target, source))?;

        let pages = self.read(|conn| {
            // The whole consistency guarantee. DEFERRED acquires nothing until the first
            // read, so the read below is part of taking the snapshot and not a formality:
            // it pins the WAL snapshot the copy is assembled from. T-77.
            conn.execute_batch("BEGIN DEFERRED")
                .map_err(|source| backup_refused(target, InternalError::wrap(source)))?;
            let _: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
                .map_err(|source| backup_refused(target, InternalError::wrap(source)))?;

            let result =
                Self::step_backup(conn, &mut destination, PAGES_PER_STEP, cancel, progress);

            // Released whatever happened. A read transaction left open would hold the WAL
            // snapshot for the life of the handle and block a later checkpoint.
            let _ = conn.execute_batch("COMMIT");
            result
        })?;

        // Closed before anything reads the file, so its own `-wal` is checkpointed away and
        // the copy is a single file like the other mechanism's.
        destination
            .close()
            .map_err(|(_, source)| backup_refused(target, InternalError::wrap(source)))?;
        Ok(pages)
    }

    /// Drives the backup to completion, reporting and checking between batches.
    fn step_backup(
        source: &Connection,
        destination: &mut Connection,
        pages_per_step: i32,
        cancel: &dyn Cancel,
        progress: &mut dyn FnMut(SnapshotProgress),
    ) -> Result<u32> {
        use rusqlite::backup::{Backup, StepResult};

        let backup = Backup::new(source, destination)
            .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;

        loop {
            if cancel.is_cancelled() {
                return Err(DbError::SnapshotCancelled);
            }

            let step = backup
                .step(pages_per_step)
                .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;

            let at = backup.progress();
            let total = u32::try_from(at.pagecount).unwrap_or(u32::MAX);
            let remaining = u32::try_from(at.remaining).unwrap_or(0);
            progress(SnapshotProgress {
                copied_pages: total.saturating_sub(remaining),
                total_pages: total,
            });

            match step {
                StepResult::Done => return Ok(total),
                StepResult::More => (),
                // Neither should occur: the source is a connection we hold and the
                // destination is a file nobody else knows about. Reported rather than
                // retried, because a silent retry loop here would be an unbounded wait.
                //
                // `StepResult` is `#[non_exhaustive]`, so the catch-all is required rather
                // than defensive — and a variant a future rusqlite adds is reported the
                // same way instead of being matched as success.
                _ => {
                    return Err(DbError::Internal(InternalError::from_message(
                        "the snapshot was blocked by another connection",
                    )));
                }
            }
        }
    }
}

/// Reads back everything the snapshot will be described by, from the snapshot.
fn describe(target: &Path, pages: u32) -> Result<Snapshot> {
    // Opened plainly, with no site-id function: nothing is written to a snapshot, and a
    // connection that *could* write to one is a capability with no use.
    let conn = connection::open_plain(target)
        .map_err(|error| backup_refused(target, InternalError::from_message(error.to_string())))?;

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|source| backup_refused(target, InternalError::wrap(source)))?;
    if integrity != "ok" {
        return Err(backup_refused(
            target,
            InternalError::from_message(format!("the copy reports `{integrity}`")),
        ));
    }

    let facts = super::facts::read_facts(&conn)?;

    // Closed before the size is read, so the figure is the single file the archive will
    // carry rather than a main file with an unflushed `-wal` beside it.
    conn.close()
        .map_err(|(_, source)| backup_refused(target, InternalError::wrap(source)))?;
    let byte_len = std::fs::metadata(target).map_or(0, |metadata| metadata.len());

    Ok(Snapshot {
        path: target.to_path_buf(),
        pages,
        facts,
        byte_len,
        keep: false,
    })
}

/// A name in `directory` that is not taken.
///
/// A UUIDv7 rather than a counter, because two exports may run from two installations into
/// one shared folder and a counter would have them race for the same name. The existence
/// check is still made: T-78 proves `VACUUM INTO` adopts a *zero-length* target rather than
/// refusing it, so the engine is not the guard here.
fn unique_target(directory: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(directory).map_err(|source| DbError::SettingsIo {
        action: "create the snapshot directory",
        source,
    })?;

    for _ in 0..16 {
        let candidate = directory.join(format!(
            "snapshot-{}.{}",
            uuid::Uuid::now_v7(),
            crate::workspace::PROJECT_EXTENSION
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(DbError::Internal(InternalError::from_message(
        "could not find an unused name for the snapshot",
    )))
}

/// Builds the refusal, naming the file and never its directory.
fn backup_refused(path: &Path, source: InternalError) -> DbError {
    DbError::BackupRefused {
        name: path.file_name().map_or_else(
            || String::from("the snapshot"),
            |name| name.to_string_lossy().into_owned(),
        ),
        source,
    }
}

#[cfg(test)]
mod send_assertions {
    //! That a snapshot can cross a thread.
    //!
    //! Mirrors `evara_db::scheduling`'s assertion for `Assembly`, and for the same reason:
    //! `ARCHITECTURE.md` §3.3 permits the command thread to compile under the lock, release
    //! it, and **move** the result to a worker. If `Snapshot` stopped being `Send` the
    //! export would have to hold the workspace lock for the whole archive write, which is
    //! exactly what D8 forbids — and the compiler would report it at the Tauri command,
    //! two stages away from the cause.

    use super::Snapshot;

    const fn assert_send<T: Send>() {}

    #[test]
    fn a_snapshot_can_be_moved_to_a_worker() {
        assert_send::<Snapshot>();
    }
}
