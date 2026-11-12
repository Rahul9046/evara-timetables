//! Reading an archive from somewhere else, in an order where each step gates the next.
//!
//! # The order is the design
//!
//! | # | Step | Refusal |
//! | --- | --- | --- |
//! | 1 | Open the ZIP; count entries; judge every name | `NotAnArchive`, `TooManyEntries`, `UnsafeEntryName`, `DuplicateEntryName` |
//! | 2 | Locate `manifest.json` by **exact name** | `ManifestMissing` |
//! | 3 | Read ≤ 64 KiB through a limited reader | `EntryTooLarge` — refused *before* being parsed |
//! | 4 | Parse; check `format` and `formatVersion` | `ManifestUnreadable`, `UnknownFormat`, `FormatTooNew` |
//! | 5 | Locate `checksums.json`; read ≤ 1 MiB; digest it; compare with `manifest.integrity.sha256` | `EntryMissing`, `EntryTooLarge`, `ChecksumMismatch` |
//! | 6 | Parse it; require `algorithm == "sha256"` exactly | `UnsupportedDigest` |
//! | 7 | Check `payload.schemaVersion` against `supported_schema_version()` | `SchemaTooNew` — **before a payload byte is written** |
//! | 8 | Check `capabilities` | `UnsupportedCapability` — also before any payload write |
//! | 9 | Stream the payload into staging through a reader limited to `payload.byteLength`, hashing | `EntryTooLarge`, `PayloadLengthMismatch`, `ChecksumMismatch` |
//! | 10 | Only now is the payload a file worth opening | — |
//!
//! Steps 7 and 8 sit before step 9 deliberately: **an archive this build cannot use is
//! refused without writing anything at all.** That is the difference between an import that
//! fails and an import that fails having left a half-staged project behind.
//!
//! # Nothing is ever extracted to a path from the archive
//!
//! Three entries, opened by exact name. `zip::ZipArchive::extract` — where all three of the
//! RUSTSEC advisories Stage P closed live — is never called, no entry's attributes are
//! honoured, and the payload is streamed to a path **Evara chose**. See [`crate::entry`] and
//! ADR 0055.
//!
//! # What a digest proves, and what it does not
//!
//! Corruption, not authenticity. The manifest is the deliberately unverified root: anyone can
//! rewrite it and recompute every digest below it. [`crate::manifest`] says so where the type
//! is defined, and the interface says so at the point of import.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use evara_core::scheduling::Cancel;
use zip::{CompressionMethod, ZipArchive};

use crate::checksums::{Checksums, HashingReader, classify_copy_failure, digest_of_bytes};
use crate::entry::{self, CHECKSUMS_ENTRY, MANIFEST_ENTRY, PAYLOAD_ENTRY};
use crate::error::{Result, TransferError};
use crate::limits::{
    COPY_BUFFER_BYTES, MAX_CHECKSUMS_BYTES, MAX_COMPRESSION_RATIO, MAX_ENTRIES, MAX_MANIFEST_BYTES,
    MAX_PAYLOAD_BYTES,
};
use crate::manifest::{DIGEST_ALGORITHM, FORMAT, FORMAT_VERSION, Manifest};
use crate::paths::file_name_of;
use crate::progress::{Phase, TransferProgress};

/// Why an archive cannot be imported by this build, as a value rather than an error.
///
/// Returned by [`inspect`] rather than raised, because inspecting is how an interface finds
/// out *whether* it can offer the import — and a question whose "no" is an error forces the
/// caller to catch one in order to render a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readability {
    /// This build can import it.
    Readable,
    /// The container is newer than this build reads.
    FormatTooNew {
        /// Found in the manifest.
        found: u32,
        /// The highest this build reads.
        supported: u32,
    },
    /// The payload's schema is newer than this build supports.
    SchemaTooNew {
        /// Found in the manifest.
        found: u32,
        /// The highest this build supports.
        supported: u32,
    },
    /// The archive needs a capability this build does not implement.
    UnsupportedCapability {
        /// The first one not recognised.
        capability: String,
    },
}

impl Readability {
    /// Whether an import may be offered.
    #[must_use]
    pub const fn is_readable(&self) -> bool {
        matches!(self, Self::Readable)
    }

    /// The refusal this readability would produce, or `None` when it is readable.
    ///
    /// One place turns a readability into a refusal, so [`inspect`]'s answer and
    /// [`crate::import`]'s refusal cannot drift into saying different things about one
    /// archive.
    #[must_use]
    pub fn as_refusal(&self) -> Option<TransferError> {
        match self {
            Self::Readable => None,
            Self::FormatTooNew { found, supported } => Some(TransferError::FormatTooNew {
                found: *found,
                supported: *supported,
            }),
            Self::SchemaTooNew { found, supported } => {
                Some(TransferError::Database(evara_db::DbError::SchemaTooNew {
                    found: *found,
                    supported: *supported,
                }))
            }
            Self::UnsupportedCapability { capability } => {
                Some(TransferError::UnsupportedCapability {
                    capability: capability.clone(),
                })
            }
        }
    }
}

/// What an archive says about itself. Reading this writes nothing and stages nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSummary {
    /// The manifest, as parsed.
    pub manifest: Manifest,
    /// How big the archive file is.
    pub archive_bytes: u64,
    /// Safe entry names this build does not know, ignored and reported — ADR 0004's forward
    /// tolerance.
    pub unknown_entries: Vec<String>,
    /// Whether this build can import it.
    pub readable: Readability,
}

/// Reads an archive's manifest and checksums. No staging, no writes, no database.
///
/// Steps 1–8 of the order above, and not step 9: nothing is streamed and no file is created.
/// T-55 asserts that by comparing the filesystem before and after.
///
/// # Errors
///
/// Every refusal in steps 1–6. A **version or capability** problem is not an error here — it
/// comes back as [`ArchiveSummary::readable`], so an interface can say what is wrong without
/// catching anything.
pub fn inspect(archive: &Path) -> Result<ArchiveSummary> {
    let archive_bytes = std::fs::metadata(archive)
        .map_err(TransferError::io("read the archive"))?
        .len();

    let mut zip = open_archive(archive)?;
    let unknown_entries = audit_entry_names(&mut zip)?;
    let manifest = read_manifest(&mut zip)?;
    let _ = read_checksums(&mut zip, &manifest)?;

    Ok(ArchiveSummary {
        readable: judge(&manifest),
        manifest,
        archive_bytes,
        unknown_entries,
    })
}

/// What a verified payload stream produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamedPayload {
    /// How many bytes were written.
    pub byte_length: u64,
    /// Their digest, lowercase hex.
    pub sha256: String,
}

/// Streams the payload into a path **the caller chose**, verifying length and digest.
///
/// Step 9. The destination is created by this function and is removed by the caller on any
/// failure — the staging lifecycle owns that, because it owns the directory.
///
/// # Errors
///
/// - [`TransferError::EntryTooLarge`] if more bytes arrive than the manifest declared.
/// - [`TransferError::PayloadLengthMismatch`] if fewer do.
/// - [`TransferError::ChecksumMismatch`] if the digest disagrees with `checksums.json`.
/// - [`TransferError::Cancelled`] if the caller stopped it.
pub fn stream_payload(
    archive: &Path,
    manifest: &Manifest,
    checksums: &Checksums,
    into: &Path,
    progress: &dyn TransferProgress,
    cancel: &dyn Cancel,
) -> Result<StreamedPayload> {
    let declared = manifest.payload.byte_length;
    if declared > MAX_PAYLOAD_BYTES {
        return Err(TransferError::EntryTooLarge {
            entry: String::from(PAYLOAD_ENTRY),
            limit: MAX_PAYLOAD_BYTES,
        });
    }
    let expected = checksums.digest_of(PAYLOAD_ENTRY)?.to_owned();

    let mut zip = open_archive(archive)?;
    require_readable_entry(&mut zip, PAYLOAD_ENTRY)?;

    let mut source = zip
        .by_name(PAYLOAD_ENTRY)
        .map_err(|_| TransferError::EntryMissing {
            entry: PAYLOAD_ENTRY,
        })?;

    // Limited to the declared length, so more bytes than declared stop at the bound rather
    // than filling the disk. Hashing as they pass, so the digest is of exactly what was
    // written and not of a second read of the file.
    let mut reader = HashingReader::new(&mut source, declared, PAYLOAD_ENTRY);
    let mut target = File::create(into).map_err(TransferError::io("stage the campus"))?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut written = 0_u64;

    loop {
        if cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| classify_copy_failure("stage the campus", error))?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut target, &buffer[..read])
            .map_err(TransferError::io("stage the campus"))?;
        written += read as u64;
        progress.progress(Phase::Staging, written, declared);
    }

    target
        .sync_all()
        .map_err(TransferError::io("stage the campus"))?;
    drop(target);

    // Length before digest. A short payload and a corrupted one are two different things to
    // tell a user, and a digest comparison would report both as corruption.
    if written != declared {
        return Err(TransferError::PayloadLengthMismatch {
            declared,
            actual: written,
        });
    }

    let found = reader.finish();
    if found != expected {
        return Err(TransferError::ChecksumMismatch {
            entry: String::from(PAYLOAD_ENTRY),
        });
    }

    Ok(StreamedPayload {
        byte_length: written,
        sha256: found,
    })
}

/// Reads the manifest and the checksums together, for a caller about to stage.
///
/// # Errors
///
/// As [`inspect`], plus the readability refusal: a caller that is **importing** wants a
/// version or capability problem as an error, because it is about to write.
pub fn read_verified_header(archive: &Path) -> Result<(Manifest, Checksums, Vec<String>)> {
    let mut zip = open_archive(archive)?;
    let unknown = audit_entry_names(&mut zip)?;
    let manifest = read_manifest(&mut zip)?;
    let checksums = read_checksums(&mut zip, &manifest)?;

    // Steps 7 and 8, before a byte is staged.
    if let Some(refusal) = judge(&manifest).as_refusal() {
        return Err(refusal);
    }
    Ok((manifest, checksums, unknown))
}

/// Opens the ZIP, turning every `zip` failure into one of this crate's.
fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>> {
    let file = File::open(path).map_err(TransferError::io("open the archive"))?;
    ZipArchive::new(BufReader::new(file)).map_err(|_| TransferError::NotAnArchive {
        name: file_name_of(path),
    })
}

/// Steps 1: count, judge every name, and report the safe unknowns.
fn audit_entry_names<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<Vec<String>> {
    if zip.len() > MAX_ENTRIES {
        return Err(TransferError::TooManyEntries {
            found: zip.len(),
            limit: MAX_ENTRIES,
        });
    }

    // `zip`'s own answer rather than a set this code builds. Which of two entries with one
    // name a reader returns is a property of the reader, so an archive carrying a duplicate
    // is one where two readers legitimately disagree — and the disagreement would be
    // invisible. Refused rather than resolved.
    if zip.has_duplicate_names() {
        let first = zip
            .file_names_raw()
            .next()
            .map_or_else(String::new, |name| {
                String::from_utf8_lossy(name).into_owned()
            });
        return Err(TransferError::DuplicateEntryName { entry: first });
    }

    // The **raw** stored bytes, not an interpreted name: the point is to judge what the
    // archive says, and `mangled_name` has already decided what to do about a hostile one.
    let raw: Vec<Vec<u8>> = zip.file_names_raw().map(<[u8]>::to_vec).collect();
    let mut unknown = Vec::new();
    for name in &raw {
        entry::require_safe(name)?;
        // Safe, so it is valid UTF-8 by construction — `classify` refuses anything else.
        let text = String::from_utf8_lossy(name).into_owned();
        if !entry::is_known(&text) {
            unknown.push(text);
        }
    }
    unknown.sort();
    Ok(unknown)
}

/// Steps 2–4: locate, bound, parse, and check the container version's two fields.
fn read_manifest<R: Read + Seek>(zip: &mut ZipArchive<R>) -> Result<Manifest> {
    let bytes = read_small_entry(zip, MANIFEST_ENTRY, MAX_MANIFEST_BYTES).map_err(|error| {
        // A missing manifest is its own sentence: it is the difference between "this is not
        // an Evara archive" and "this archive is damaged".
        match error {
            TransferError::EntryMissing { .. } => TransferError::ManifestMissing {
                name: String::from("this file"),
            },
            other => other,
        }
    })?;

    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|error| TransferError::ManifestUnreadable {
            detail: error.to_string(),
        })?;

    if manifest.format != FORMAT {
        return Err(TransferError::UnknownFormat {
            found: manifest.format,
        });
    }
    if !manifest.format_is_readable() {
        return Err(TransferError::FormatTooNew {
            found: manifest.format_version,
            supported: FORMAT_VERSION,
        });
    }
    if manifest.payload.entry != PAYLOAD_ENTRY {
        return Err(TransferError::ManifestDisagreesWithPayload {
            field: "payload.entry",
        });
    }
    if manifest.payload.kind != crate::manifest::PAYLOAD_KIND {
        return Err(TransferError::ManifestDisagreesWithPayload {
            field: "payload.kind",
        });
    }
    if manifest.integrity.entry != CHECKSUMS_ENTRY {
        return Err(TransferError::ManifestDisagreesWithPayload {
            field: "integrity.entry",
        });
    }
    Ok(manifest)
}

/// Steps 5–6: bound, digest, compare against the manifest, then parse and check the
/// algorithm.
///
/// The digest comparison comes **before** the parse, which is the point of the two-level
/// tree: a `checksums.json` somebody edited is refused as damaged rather than parsed and
/// believed.
fn read_checksums<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    manifest: &Manifest,
) -> Result<Checksums> {
    let bytes = read_small_entry(zip, CHECKSUMS_ENTRY, MAX_CHECKSUMS_BYTES)?;

    if digest_of_bytes(&bytes) != manifest.integrity.sha256 {
        return Err(TransferError::ChecksumMismatch {
            entry: String::from(CHECKSUMS_ENTRY),
        });
    }
    if manifest.integrity.algorithm != DIGEST_ALGORITHM {
        return Err(TransferError::UnsupportedDigest {
            algorithm: manifest.integrity.algorithm.clone(),
        });
    }

    let checksums: Checksums =
        serde_json::from_slice(&bytes).map_err(|error| TransferError::ManifestUnreadable {
            detail: error.to_string(),
        })?;
    if checksums.algorithm != DIGEST_ALGORITHM {
        return Err(TransferError::UnsupportedDigest {
            algorithm: checksums.algorithm,
        });
    }
    Ok(checksums)
}

/// Reads one small entry into memory, bounded, after judging its metadata.
fn read_small_entry<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &'static str,
    limit: u64,
) -> Result<Vec<u8>> {
    require_readable_entry(zip, name)?;

    let mut file = zip
        .by_name(name)
        .map_err(|_| TransferError::EntryMissing { entry: name })?;
    let mut reader = HashingReader::new(&mut file, limit, name);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| classify_copy_failure("read the archive", error))?;
    Ok(bytes)
}

/// Judges one entry's metadata before a byte of it is decompressed.
///
/// Every check here reads the central directory only, so a refusal costs nothing and an
/// archive built to exhaust a decompressor never reaches one.
fn require_readable_entry<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &'static str,
) -> Result<()> {
    let index = zip
        .index_for_name(name)
        .ok_or(TransferError::EntryMissing { entry: name })?;
    let metadata = zip
        .by_index_data(index)
        .map_err(|_| TransferError::EntryMissing { entry: name })?;

    // `Stored` or `Deflate` and nothing else. This closes off zstd, bzip2, lzma and ppmd
    // decoders and their bombs without having to reason about any of them — and the `zip`
    // features for all four are off in this crate's manifest, so an archive using one would
    // fail to decompress anyway. Refusing by method is the readable half of that.
    if !matches!(
        metadata.compression(),
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return Err(TransferError::UnsupportedCompression {
            entry: name.to_owned(),
        });
    }
    if metadata.encrypted() {
        return Err(TransferError::EntryEncrypted {
            entry: name.to_owned(),
        });
    }
    if metadata.is_dir() || metadata.is_symlink() {
        return Err(TransferError::EntryNotAFile {
            entry: name.to_owned(),
        });
    }

    // The declared ratio, from the directory. A bomb is refused here rather than discovered
    // at the byte cap: both stop it, and this one stops it without decompressing anything.
    // `checked_div` rather than a guard, so a zero-length compressed entry — which is a
    // zero-length entry, and legitimate — is `None` rather than a division by zero.
    if let Some(ratio) = metadata.size().checked_div(metadata.compressed_size())
        && ratio > MAX_COMPRESSION_RATIO
    {
        return Err(TransferError::EntryExpandsTooFar {
            entry: name.to_owned(),
            ratio,
        });
    }
    Ok(())
}

/// Steps 7 and 8, as a value.
fn judge(manifest: &Manifest) -> Readability {
    if !manifest.format_is_readable() {
        return Readability::FormatTooNew {
            found: manifest.format_version,
            supported: FORMAT_VERSION,
        };
    }

    let supported = evara_db::supported_schema_version();
    if manifest.payload.schema_version > supported {
        return Readability::SchemaTooNew {
            found: manifest.payload.schema_version,
            supported,
        };
    }

    if let Some(capability) = manifest
        .capabilities
        .iter()
        .find(|capability| !is_supported_capability(capability))
    {
        return Readability::UnsupportedCapability {
            capability: capability.clone(),
        };
    }

    Readability::Readable
}

/// Whether this build understands a capability.
///
/// Namespaced, so an unrecognised **prefix** is a known-unknown rather than a parse
/// ambiguity — and it is still a refusal, because a prefix this build has never heard of
/// describes something it cannot render honestly.
///
/// `constraint:` is checked against `evara_core::scheduling::ConstraintType::from_key`, which
/// is why this crate needs no `evara-constraints` edge: D3, and the registry is where a
/// constraint type lives.
fn is_supported_capability(capability: &str) -> bool {
    let Some((prefix, value)) = capability.split_once(':') else {
        return false;
    };
    match prefix {
        "constraint" => evara_core::scheduling::ConstraintType::from_key(value).is_some(),
        "cycle-kind" => matches!(value, "WEEKLY" | "CUSTOM"),
        "lineage" => value == "repair",
        "lock" => value == "placement",
        "placement" => value == "manual",
        "logo" => value == "embedded",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_supported_capability;

    #[test]
    fn every_capability_this_build_writes_is_one_it_reads() {
        // The property that matters: an archive this build produced must import into it.
        // Written as a sweep over the registry rather than a list, so a tenth constraint
        // type is covered by the test the moment it is registered.
        for constraint_type in evara_core::scheduling::ConstraintType::all() {
            assert!(
                is_supported_capability(&format!("constraint:{}", constraint_type.key())),
                "{} is written by an export and must be readable",
                constraint_type.key()
            );
        }
        for capability in [
            "cycle-kind:WEEKLY",
            "cycle-kind:CUSTOM",
            "lineage:repair",
            "lock:placement",
            "placement:manual",
            "logo:embedded",
        ] {
            assert!(is_supported_capability(capability));
        }
    }

    #[test]
    fn an_unknown_capability_or_prefix_is_refused() {
        for capability in [
            "constraint:teacher-must-like-mondays",
            "cycle-kind:FORTNIGHTLY",
            "lineage:merge",
            "something-new:value",
            "no-colon-at-all",
            "",
        ] {
            assert!(
                !is_supported_capability(capability),
                "{capability} must not be treated as supported"
            );
        }
    }
}
