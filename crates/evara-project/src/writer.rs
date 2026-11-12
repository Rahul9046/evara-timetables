//! Sealing an owned snapshot into a `.evara`, and finalising it by rename.
//!
//! # Three entries, in one order, with fixed timestamps
//!
//! ```text
//! Northgate.evara                        ZIP
//! ├── manifest.json                      STORED (uncompressed), first local header
//! ├── checksums.json                     DEFLATE
//! └── payload/project.evaraproj          DEFLATE   — the verified snapshot
//! ```
//!
//! `manifest.json` is **stored** rather than deflated, so it is readable with no decompressor
//! and cheaply — it is parsed before anything else is trusted, and a reader should not have to
//! inflate anything to find out whether it is looking at an Evara archive at all.
//!
//! Every entry timestamp is the **ZIP epoch**, not the clock. Two exports of an unchanged
//! project therefore differ only in `manifest.exportedAt` and `exportedBySiteId`, which is
//! T-05, and it is what makes an archive's bytes a function of the project rather than of
//! when somebody pressed the button.
//!
//! # The snapshot is taken by value, and that is the whole point
//!
//! [`seal`] takes a `Snapshot` and no `&Database`. The command thread takes the snapshot
//! under the workspace lock, **releases the lock**, and moves the snapshot to a worker that
//! runs this — `ARCHITECTURE.md` §3.3's arrangement, the same one `Assembly` has, and the fix
//! for revision 1 of the design, which held a borrow of the database across the whole archive
//! write and so would have held the lock for it too.
//!
//! Every manifest fact is read off the `Snapshot`, which read them out of the copy. Nothing
//! here reopens the project.
//!
//! # Finalising, and why the destination is only ever written by a rename
//!
//! The archive is written to `<dest>.part` beside the destination and renamed. Evara never
//! writes over an existing `.evara` directly, so a failure mid-write leaves the previous
//! archive intact and readable (T-47). The `.part` is removed on every failure path and on
//! cancellation, by [`PartFile`]'s `Drop` rather than by each `?` remembering — the same
//! defect `backup.rs` had, fixed structurally.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use evara_core::scheduling::Cancel;
use evara_db::transfer::{ProjectFacts, Snapshot};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::checksums::{Checksums, HashingReader, classify_copy_failure, digest_of_bytes};
use crate::entry::{CHECKSUMS_ENTRY, MANIFEST_ENTRY, PAYLOAD_ENTRY};
use crate::error::{Result, TransferError};
use crate::limits::{COPY_BUFFER_BYTES, MAX_PAYLOAD_BYTES};
use crate::manifest::{
    ChangeLogSummary, DIGEST_ALGORITHM, FORMAT, FORMAT_VERSION, Generator, Integrity, Manifest,
    PAYLOAD_KIND, PayloadDescriptor, SchoolSummary,
};
use crate::progress::{Phase, TransferProgress};

/// The extension a portable archive carries.
///
/// Distinct from `evara_db::PROJECT_EXTENSION`, and they must stay distinct: `.evaraproj` is
/// the **live** project and `.evara` is the **portable export**. `CLAUDE.md` §3 forbids using
/// one where the other belongs and forbids a third, and `require_project_extension` already
/// rejects `.evara` as a live project.
pub const ARCHIVE_EXTENSION: &str = "evara";

/// What an export produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Where the archive is.
    pub path: PathBuf,
    /// How big it is.
    pub archive_bytes: u64,
    /// How big the payload was, uncompressed.
    pub payload_bytes: u64,
    /// The payload's digest, lowercase hex.
    pub payload_sha256: String,
    /// The manifest, as written.
    pub manifest: Manifest,
}

impl ExportReport {
    /// How much smaller the archive is than its payload, as a percentage of the payload.
    ///
    /// Reported rather than asserted. A snapshot taken by the backup API reproduces the
    /// source page for page, free pages included (§C.6), so a heavily edited project carries
    /// its free list into the payload — and free pages are mostly zeroes, which deflate
    /// almost completely. This figure is how §7.2's compaction question gets answered from a
    /// measurement rather than from a guess.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a percentage of a file size: the mantissa runs out at eight petabytes, and                   the byte cap refuses anything past two gibibytes"
    )]
    pub fn compression_saving_percent(&self) -> f64 {
        if self.payload_bytes == 0 {
            return 0.0;
        }
        let payload = self.payload_bytes as f64;
        let archive = self.archive_bytes as f64;
        ((payload - archive) / payload) * 100.0
    }
}

/// Seals an owned snapshot into an archive at `destination`.
///
/// Consumes the snapshot: the file is removed once it has been sealed, by [`Snapshot`]'s own
/// `Drop`, so there is nothing for a caller to remember.
///
/// # Errors
///
/// - [`TransferError::WrongExtension`] if `destination` is not a `.evara`.
/// - [`TransferError::EntryTooLarge`] if the snapshot is beyond [`MAX_PAYLOAD_BYTES`].
/// - [`TransferError::Cancelled`] if the caller stopped it. No archive and no `.part`.
/// - [`TransferError::Io`] naming what was being attempted.
#[expect(
    clippy::needless_pass_by_value,
    reason = "taking the snapshot by value is the point: it is what lets the caller release \
              the workspace lock before this runs (D8), and it *is* consumed -- by its own \
              `Drop`, which removes the file"
)]
pub fn seal(
    snapshot: Snapshot,
    destination: &Path,
    progress: &dyn TransferProgress,
    cancel: &dyn Cancel,
) -> Result<ExportReport> {
    require_archive_extension(destination)?;
    crate::paths::require_safe_destination(destination)?;

    if snapshot.byte_len() > MAX_PAYLOAD_BYTES {
        return Err(TransferError::EntryTooLarge {
            entry: String::from(PAYLOAD_ENTRY),
            limit: MAX_PAYLOAD_BYTES,
        });
    }
    if cancel.is_cancelled() {
        return Err(TransferError::Cancelled);
    }

    // Hashed before the archive is opened, so a snapshot that cannot be read produces no
    // `.part` at all. One pass, and the figure the manifest's bound is checked against.
    progress.progress(Phase::Hashing, 0, snapshot.byte_len());
    let payload_sha256 = digest_of_file(snapshot.path(), cancel)?;
    progress.progress(Phase::Hashing, snapshot.byte_len(), snapshot.byte_len());

    let checksums = Checksums::of(PAYLOAD_ENTRY, payload_sha256.clone());
    let checksums_bytes =
        checksums
            .to_bytes()
            .map_err(|error| TransferError::ManifestUnreadable {
                detail: error.to_string(),
            })?;

    let manifest = build_manifest(&snapshot, &digest_of_bytes(&checksums_bytes));
    let manifest_bytes =
        manifest
            .to_bytes()
            .map_err(|error| TransferError::ManifestUnreadable {
                detail: error.to_string(),
            })?;

    let part = PartFile::claim(destination)?;
    write_archive(
        part.path(),
        &manifest_bytes,
        &checksums_bytes,
        snapshot.path(),
        snapshot.byte_len(),
        progress,
        cancel,
    )?;

    progress.progress(Phase::Finalising, 0, 1);
    let archive_bytes = std::fs::metadata(part.path())
        .map_err(TransferError::io("read the archive back"))?
        .len();
    part.finalise(destination)?;
    progress.progress(Phase::Finalising, 1, 1);

    Ok(ExportReport {
        path: destination.to_path_buf(),
        archive_bytes,
        payload_bytes: snapshot.byte_len(),
        payload_sha256,
        manifest,
    })
}

/// Refuses a destination that is not a `.evara`.
///
/// # Errors
///
/// [`TransferError::WrongExtension`].
pub fn require_archive_extension(path: &Path) -> Result<()> {
    let matches = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(ARCHIVE_EXTENSION));
    if matches {
        Ok(())
    } else {
        Err(TransferError::WrongExtension {
            expected: ARCHIVE_EXTENSION,
        })
    }
}

/// Every manifest fact, taken off the snapshot.
fn build_manifest(snapshot: &Snapshot, checksums_digest: &str) -> Manifest {
    let facts: &ProjectFacts = snapshot.facts();
    Manifest {
        format: String::from(FORMAT),
        format_version: FORMAT_VERSION,
        payload: PayloadDescriptor {
            kind: String::from(PAYLOAD_KIND),
            entry: String::from(PAYLOAD_ENTRY),
            schema_version: facts.schema_version,
            byte_length: snapshot.byte_len(),
        },
        generator: Generator {
            app: String::from("Evara Timetables"),
            version: String::from(env!("CARGO_PKG_VERSION")),
        },
        project_id: facts.identity.project_id,
        created_by_site_id: facts.identity.created_by_site_id,
        // Filled in by the caller that knows which installation it is. Left `None` here
        // rather than guessed at: `evara-project` has no installation identity of its own,
        // and writing `created_by_site_id` into it would be exactly the confusion T-03 is
        // written to catch.
        exported_by_site_id: None,
        exported_at: evara_db::now_iso8601(),
        change_log: ChangeLogSummary {
            entries: facts.change_log.entries,
            watermark: facts.change_log.watermark,
        },
        schools: facts
            .schools
            .iter()
            .map(|school| SchoolSummary {
                id: school.id,
                name: school.name.clone(),
                campuses: school.campuses,
            })
            .collect(),
        counts: facts.census.clone(),
        capabilities: facts.capabilities(),
        integrity: Integrity {
            algorithm: String::from(DIGEST_ALGORITHM),
            entry: String::from(CHECKSUMS_ENTRY),
            sha256: checksums_digest.to_owned(),
        },
    }
}

/// Writes the three entries, in order, checking cancellation between buffers.
fn write_archive(
    part: &Path,
    manifest_bytes: &[u8],
    checksums_bytes: &[u8],
    payload: &Path,
    payload_bytes: u64,
    progress: &dyn TransferProgress,
    cancel: &dyn Cancel,
) -> Result<()> {
    let file = File::create(part).map_err(TransferError::io("create the archive"))?;
    let mut archive = ZipWriter::new(BufWriter::new(file));

    // The ZIP epoch, not the clock. `SimpleFileOptions::default()` is 1980-01-01 in this
    // build — the `time` feature is off — but it is set explicitly so enabling that feature
    // somewhere else cannot silently make every export's bytes depend on the clock.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        // ZIP64 for a payload at or beyond 4 GiB. The byte cap refuses one that large first,
        // so this is the belt to that braces: a future cap increase does not also need
        // somebody to remember this line.
        .large_file(payload_bytes >= u64::from(u32::MAX));

    archive
        .start_file(MANIFEST_ENTRY, stored)
        .map_err(|_| TransferError::io("write the manifest")(zip_io()))?;
    archive
        .write_all(manifest_bytes)
        .map_err(TransferError::io("write the manifest"))?;

    archive
        .start_file(CHECKSUMS_ENTRY, deflated)
        .map_err(|_| TransferError::io("write the checksums")(zip_io()))?;
    archive
        .write_all(checksums_bytes)
        .map_err(TransferError::io("write the checksums"))?;

    archive
        .start_file(PAYLOAD_ENTRY, deflated)
        .map_err(|_| TransferError::io("write the campus")(zip_io()))?;

    let mut source = File::open(payload).map_err(TransferError::io("read the copy"))?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut written = 0_u64;
    loop {
        if cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let read = source
            .read(&mut buffer)
            .map_err(TransferError::io("read the copy"))?;
        if read == 0 {
            break;
        }
        archive
            .write_all(&buffer[..read])
            .map_err(TransferError::io("write the campus"))?;
        written += read as u64;
        progress.progress(Phase::Compressing, written, payload_bytes);
    }

    let inner = archive
        .finish()
        .map_err(|_| TransferError::io("finish the archive")(zip_io()))?;
    inner
        .into_inner()
        .map_err(|error| TransferError::io("flush the archive")(error.into_error()))?
        // `sync_all` rather than a bare drop: the rename that follows is what makes the
        // destination appear, and a destination that appears before its contents are on disk
        // is the one state the atomic-finalise claim does not cover.
        .sync_all()
        .map_err(TransferError::io("flush the archive"))?;
    Ok(())
}

/// The SHA-256 of a file, read through the byte cap.
fn digest_of_file(path: &Path, cancel: &dyn Cancel) -> Result<String> {
    let file = File::open(path).map_err(TransferError::io("read the copy"))?;
    let mut reader = HashingReader::new(file, MAX_PAYLOAD_BYTES, PAYLOAD_ENTRY);
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        if cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| classify_copy_failure("read the copy", error))?;
        if read == 0 {
            break;
        }
    }
    Ok(reader.finish())
}

/// A stand-in `io::Error` for a `zip` failure.
///
/// `zip`'s own error type may not cross this crate's boundary — `CLAUDE.md`'s driver-type
/// rule, applied to a second crate — and every `zip` failure reachable from the three
/// `start_file` calls above is a write failure on the underlying file, so it is reported as
/// one. The detail is lost deliberately: a `TransferError` that re-exported `ZipError` would
/// make a `zip` major bump a breaking change to this crate's public type.
fn zip_io() -> std::io::Error {
    std::io::Error::other("the archive writer refused")
}

/// The `.part` file an export writes into, which removes itself unless finalised.
///
/// The claim is `create_new`, so two exports to one destination cannot both believe they own
/// the `.part`. Structural cleanup rather than per-call-site, for the same reason
/// `Snapshot` has it: a cancelled or failed export must leave nothing behind, and remembering
/// that at each `?` is how `backup.rs` came to leave a zero-byte file named like a backup.
struct PartFile {
    path: PathBuf,
    keep: bool,
}

impl PartFile {
    /// Claims `<destination>.part`.
    fn claim(destination: &Path) -> Result<Self> {
        let mut name = destination.as_os_str().to_os_string();
        name.push(".part");
        let path = PathBuf::from(name);

        // Removed first if it is there: a `.part` left by an abandoned export belongs to
        // nobody, is not a user document, and is the one file in this protocol that may be
        // cleaned up without asking. Everything the user can see is kept.
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(TransferError::io("clear an abandoned partial archive"))?;
        }
        Ok(Self { path, keep: false })
    }

    /// Where it is.
    fn path(&self) -> &Path {
        &self.path
    }

    /// Renames it to `destination`, after which it is no longer ours to remove.
    fn finalise(mut self, destination: &Path) -> Result<()> {
        crate::paths::rename_with_retry(&self.path, destination)?;
        self.keep = true;
        Ok(())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.keep {
            // Ignored deliberately: this runs on the failure path, and a cleanup error would
            // replace a useful message with a confusing one.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
