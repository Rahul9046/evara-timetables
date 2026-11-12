//! Staging, verifying, migrating and renaming — the protocol, with its three safeguards.
//!
//! # The phases, and what is true after each
//!
//! | Phase | What it does | What is true if it is interrupted |
//! | --- | --- | --- |
//! | **P0** | validate the destination path and the archive's header | nothing exists that did not before |
//! | **P1** | decide: claim an absent destination, or require an explicit replace | at most a zero-length claim Evara made |
//! | **P2** | stage: journal, stream, verify, migrate, verify | the destination is untouched except that claim |
//! | **P3** | Create finalise: one rename | either the placeholder or the new project is at the name |
//! | **P4** | Replace finalise: four steps, each after its journal commit | one of `{old intact}` or `{new in place, old kept beside it}` |
//!
//! # Safeguard S1 — `create_new` is the only claim, so Create needs an absent destination
//!
//! The approved design claimed a destination that was *"absent, or present and zero length"*
//! with `create_new(true)` — and `create_new` **fails if the path exists at all**, zero length
//! included. The two halves of that sentence cannot both hold.
//!
//! Resolved by narrowing Create to a destination that does not exist, and keeping
//! `create_new` as the claim. It is the only atomic claim available without FFI, and
//! `unsafe_code` is forbidden workspace-wide. An existing zero-length file is therefore
//! **occupied**: it needs an explicit [`OnConflict::Replace`], and the Replace path keeps it
//! as a `.replaced-` copy — so even an unknown empty file is never destroyed.
//!
//! This **deliberately diverges from `Database::create`**, which treats a zero-length file as
//! creatable. The two are different acts: `Database::create` is making a project where the
//! user just said to, and an import adopting an unknown empty file would be writing over
//! something nobody has looked at.
//!
//! The claim is what closes the TOCTOU window. `fs::rename` **replaces** an existing
//! destination on both platforms, so without the claim a file created between the check and
//! the rename is silently destroyed.
//!
//! # Safeguard S3 — each side is reduced to one file before it moves
//!
//! A project is up to four files, and **there is no atomic multi-file move on either
//! platform**. A crash between moving `X.evaraproj` and moving `X.evaraproj-wal` leaves the
//! old `-wal` at the destination's name, and the new main file is then renamed in beside it —
//! which is a database that **opens cleanly and is wrong**, because SQLite replays the stale
//! log over the new file.
//!
//! So the set is removed rather than moved carefully: `checkpoint_to_single_file` reduces each
//! side to one file first and **refuses before anything has moved** if a sidecar cannot be
//! released; and `refuse_if_sidecars_present` asserts the destination is clean immediately
//! before the rename that puts the new main file there. The second is the safeguard proper —
//! it makes the bad pairing unrepresentable rather than merely unlikely, and it holds even if
//! a future change reintroduces a multi-file move.
//!
//! # Nothing is ever deleted
//!
//! The replaced project is kept as `<stem>.replaced-<stamp>.evaraproj` and **no code path
//! deletes it**, on success or on failure. So "a failed import must not destroy the previous
//! project" is not a promise about error handling. The two things this protocol does remove
//! are its own staging directory and its own `.part` file, neither of which is a user
//! document.

use std::path::{Path, PathBuf};

use evara_core::scheduling::Cancel;
use evara_db::transfer::{
    DestinationVerdict, ReplacementPhase, StagedPayload, Stager, checkpoint_to_single_file,
    classify_destination, refuse_if_sidecars_present,
};
use uuid::Uuid;

use crate::error::{Result, TransferError};
use crate::manifest::Manifest;
use crate::progress::{Phase, TransferProgress};
use crate::reader;
use crate::staging::{ReplacementJournal, Staging, unused_keep_name};

/// What to do about an occupied destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnConflict {
    /// Refuse, naming the file. The interface then offers another name, or Replace.
    Refuse,
    /// Replace it, keeping the old project beside the new one.
    Replace,
}

/// Which installation is importing, and what it is called.
#[derive(Debug, Clone, Copy)]
pub struct Importer<'a> {
    /// This build's version string.
    pub app_version: &'a str,
    /// This installation's identity, which the change-log triggers read during migration.
    pub site_id: Uuid,
}

/// How many rows the archive claimed, against how many arrived.
///
/// Reported and never refused: the payload is the authority and a count is a convenience.
/// A disagreement nevertheless means the archive was assembled by something that got one of
/// the two wrong, which a user is better off knowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountDisagreement {
    /// Which table.
    pub table: String,
    /// What the manifest said.
    pub declared: i64,
    /// What the payload holds.
    pub found: i64,
}

/// What an import produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Where the project now is.
    pub path: PathBuf,
    /// Its identity, which is the archive's identity. Never regenerated.
    pub project_id: Uuid,
    /// The schema version the payload arrived at.
    pub schema_version_found: u32,
    /// The schema version it now stands at.
    pub schema_version_now: u32,
    /// Whether a migration ran.
    pub migrated: bool,
    /// Row counts per table, read from the imported project.
    pub counts: std::collections::BTreeMap<String, i64>,
    /// Where the replaced project was kept, if one was replaced.
    pub replaced_copy_kept_at: Option<PathBuf>,
    /// Safe entry names this build ignored.
    pub unknown_entries: Vec<String>,
    /// Tables where the manifest's count and the payload's disagree.
    pub count_disagreements: Vec<CountDisagreement>,
}

/// Imports an archive to `destination`.
///
/// Does **not** open what it imported. Auto-opening would close whatever the user had open,
/// discarding an edit session, a generation and a repair as a side effect of an import — D9.
/// The interface offers "Open now" instead, which is the user's decision.
///
/// # Errors
///
/// Every refusal in [`reader`]'s order, plus:
///
/// - [`TransferError::WrongExtension`] if `destination` is not a `.evaraproj`.
/// - [`TransferError::DestinationUnsafe`] if it *is* a link, or it is a URI. A link in an
///   ancestor is fine — `crate::paths::require_safe_destination` carries why.
/// - [`TransferError::DestinationExists`] with [`OnConflict::Refuse`].
/// - [`TransferError::ReplacementInFlight`] if one is already journalled for it.
/// - [`TransferError::RenameBlocked`] if a sidecar cannot be released, or a rename cannot be
///   completed after the bounded retry. **Nothing has moved** in the first case.
/// - [`TransferError::Cancelled`]. No destination, no staging.
/// - [`TransferError::Database`] for a corrupt, non-conforming or too-new payload.
pub fn import(
    archive: &Path,
    destination: &Path,
    on_conflict: OnConflict,
    importer: Importer<'_>,
    journal: &mut dyn ReplacementJournal,
    progress: &dyn TransferProgress,
    cancel: &dyn Cancel,
) -> Result<ImportReport> {
    // ---------------------------------------------------------------- P0: validate
    require_project_extension(destination)?;
    crate::paths::require_safe_destination(destination)?;

    let (manifest, checksums, unknown_entries) = reader::read_verified_header(archive)?;

    // ---------------------------------------------------------------- P1: claim or decide
    let mode = decide(destination, on_conflict)?;
    let claim = match mode {
        Mode::Create => Some(Claim::take(destination)?),
        Mode::Replace => None,
    };

    // ---------------------------------------------------------------- P2: stage
    let mut staging = Staging::open(destination, journal)?;
    let staged_file = staging.staged_file();

    progress.progress(Phase::Staging, 0, manifest.payload.byte_length);
    reader::stream_payload(
        archive,
        &manifest,
        &checksums,
        &staged_file,
        progress,
        cancel,
    )?;

    if cancel.is_cancelled() {
        return Err(TransferError::Cancelled);
    }

    progress.progress(Phase::Verifying, 0, 1);
    let staged = evara_db::transfer::stage(
        &staged_file,
        staging.dir(),
        Stager {
            app_version: importer.app_version,
            site_id: importer.site_id,
        },
        cancel,
    )?;
    progress.progress(Phase::Migrating, 1, 1);

    // The one refusal that compares the manifest with the payload. Checked after staging
    // rather than before, because the **payload's** `refinery_schema_history` is the
    // authority and only a staged file can be asked. A disagreement means the archive was not
    // assembled by Evara, so reading on would be reading something edited by hand.
    if manifest.payload.schema_version != staged.schema_version_found {
        return Err(TransferError::ManifestDisagreesWithPayload {
            field: "payload.schemaVersion",
        });
    }

    let count_disagreements = compare_counts(&manifest, &staged);

    if cancel.is_cancelled() {
        return Err(TransferError::Cancelled);
    }

    // ---------------------------------------------------------------- P3/P4: finalise
    progress.progress(Phase::Finalising, 0, 1);
    let replaced_copy_kept_at = match mode {
        Mode::Create => {
            finalise_create(&mut staging, &staged_file, destination, claim)?;
            None
        }
        Mode::Replace => Some(finalise_replace(&mut staging, &staged_file, destination)?),
    };
    progress.progress(Phase::Finalising, 1, 1);

    // The journal row goes last. Until it does, an interruption is recoverable; after it, the
    // import is finished and there is nothing to recover.
    staging.adopt()?;

    Ok(ImportReport {
        path: destination.to_path_buf(),
        project_id: staged.facts.identity.project_id,
        schema_version_found: staged.schema_version_found,
        schema_version_now: staged.schema_version_now,
        migrated: staged.migrated,
        counts: staged.facts.census.clone(),
        replaced_copy_kept_at,
        unknown_entries,
        count_disagreements,
    })
}

/// Refuses a destination that is not a `.evaraproj`.
///
/// `.evara` as a destination is refused by this too, which is the point: the two extensions
/// are different kinds of object and `CLAUDE.md` §3 forbids using one where the other belongs.
///
/// # Errors
///
/// [`TransferError::WrongExtension`].
pub fn require_project_extension(path: &Path) -> Result<()> {
    let matches = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(evara_db::PROJECT_EXTENSION));
    if matches {
        Ok(())
    } else {
        Err(TransferError::WrongExtension {
            expected: evara_db::PROJECT_EXTENSION,
        })
    }
}

/// Which finalise applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The destination did not exist, and is claimed.
    Create,
    /// The destination existed, and the caller asked for it to be replaced.
    Replace,
}

/// P1. Decides from the destination's state and the caller's answer.
///
/// S1 lives here: [`classify_destination`] has **no zero-length exception**, so an existing
/// empty file is `Occupied` and needs an explicit Replace.
fn decide(destination: &Path, on_conflict: OnConflict) -> Result<Mode> {
    // `symlink_metadata`, so a symlink at the destination counts as present rather than
    // being followed to wherever it points.
    let metadata = destination.symlink_metadata();
    let exists = metadata.is_ok();
    let zero_length = metadata.is_ok_and(|metadata| metadata.len() == 0);

    match classify_destination(exists, zero_length) {
        DestinationVerdict::Claimable => Ok(Mode::Create),
        DestinationVerdict::Occupied { .. } => match on_conflict {
            OnConflict::Replace => Ok(Mode::Replace),
            OnConflict::Refuse => Err(TransferError::DestinationExists {
                name: crate::paths::file_name_of(destination),
            }),
        },
    }
}

/// P3. One rename, replacing our own placeholder.
fn finalise_create(
    staging: &mut Staging<'_>,
    staged_file: &Path,
    destination: &Path,
    claim: Option<Claim>,
) -> Result<()> {
    // S3 step 4, on the Create path too. The destination is a zero-length placeholder Evara
    // just made, so a sidecar beside it cannot be ours — and a `-wal` whose main file is
    // about to be replaced is the one corruption that opens cleanly.
    refuse_if_sidecars_present(destination).map_err(TransferError::Database)?;
    crate::paths::rename_with_retry(staged_file, destination)?;

    // The placeholder is gone, replaced by the rename. Releasing the claim afterwards rather
    // than before is what keeps the TOCTOU window closed for the whole phase.
    if let Some(claim) = claim {
        claim.released();
    }
    staging.advance(ReplacementPhase::Placed, None)?;
    Ok(())
}

/// P4. Four steps, each after its journal commit.
///
/// Returns where the replaced project was kept.
fn finalise_replace(
    staging: &mut Staging<'_>,
    staged_file: &Path,
    destination: &Path,
) -> Result<PathBuf> {
    // S3 steps 1 and 2: reduce the destination to one file, and refuse **before anything has
    // moved** if it cannot be. `checkpoint_to_single_file` reports `RenameBlocked`, which
    // most often means another process still holds the database open — and the remedy is in
    // the message.
    checkpoint_to_single_file(destination).map_err(TransferError::Database)?;

    let keep_name = unused_keep_name(destination, &evara_db::now_iso8601())?;

    // Step 1: the old project goes aside. Journalled first, because the row is what explains
    // a destination that is briefly absent.
    let aside_dir = staging.aside_dir();
    std::fs::create_dir_all(&aside_dir).map_err(TransferError::io("prepare the aside folder"))?;
    staging.advance(ReplacementPhase::Aside, Some(&keep_name))?;
    let aside_file = staging.aside_file();
    crate::paths::rename_with_retry(destination, &aside_file)?;

    // Step 2: the new project goes in. S3 step 4 immediately before the rename — the
    // assertion that makes a main file beside a foreign sidecar unrepresentable rather than
    // unlikely, and it holds whatever sequence of interruptions produced the stray file.
    refuse_if_sidecars_present(destination).map_err(TransferError::Database)?;
    crate::paths::rename_with_retry(staged_file, destination)?;
    staging.advance(ReplacementPhase::Placed, Some(&keep_name))?;

    // Step 3: the old project is kept. **Never deleted** — `backup.rs`'s rule, and the reason
    // this protocol's failure guarantee is structural rather than a promise about error
    // handling.
    crate::paths::rename_with_retry(&aside_file, &keep_name)?;

    Ok(keep_name)
}

/// What the manifest claimed against what arrived.
fn compare_counts(manifest: &Manifest, staged: &StagedPayload) -> Vec<CountDisagreement> {
    // Only tables the manifest named. A table the payload has and the manifest does not is
    // ordinary — the archive was written at an older schema and a migration added it — so
    // reporting that as a disagreement would make every version-crossing import noisy.
    let mut disagreements = Vec::new();
    for (table, declared) in &manifest.counts {
        let found = staged.facts.count_of(table);
        if found != *declared {
            disagreements.push(CountDisagreement {
                table: table.clone(),
                declared: *declared,
                found,
            });
        }
    }
    disagreements
}

/// The zero-length placeholder that holds a destination's name.
///
/// `create_new(true)` is the claim, and it is atomic: two processes racing for one name have
/// exactly one winner. The file is removed on drop unless it was released, so a refusal after
/// the claim leaves nothing behind.
struct Claim {
    path: PathBuf,
    released: bool,
}

impl Claim {
    /// Claims `destination`, failing if anything is already there.
    fn take(destination: &Path) -> Result<Self> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| match error.kind() {
                // Somebody won the race between `decide` and here. Reported as the occupied
                // destination it now is, which is the same answer `decide` would have given
                // a moment later — and the reason the claim exists rather than a check.
                std::io::ErrorKind::AlreadyExists => TransferError::DestinationExists {
                    name: crate::paths::file_name_of(destination),
                },
                _ => TransferError::Io {
                    action: "claim the destination",
                    source: error,
                },
            })?;
        Ok(Self {
            path: destination.to_path_buf(),
            released: false,
        })
    }

    /// The rename replaced it, so it is no longer ours to remove.
    ///
    /// Consumes the claim and sets the flag its `Drop` reads, which is the idiom `Snapshot`
    /// and the `.part` file use: the cleanup stays structural and the release is a single
    /// named call rather than a flag a caller might forget.
    fn released(mut self) {
        self.released = true;
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if !self.released {
            // Only ever a zero-length file Evara made moments ago, so removing it destroys
            // nothing. A placeholder left behind would read as an empty project in a folder
            // listing, which is worse than nothing at all.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
