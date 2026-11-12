//! The digest record, and the hashing reader that bounds what it covers.
//!
//! # One digest per fact, and the manifest is not in here
//!
//! `checksums.json` carries a digest for the payload and for nothing else in
//! `formatVersion 1`. It carries **no digest of the manifest**, and the manifest is **not**
//! listed among its entries — that is the acyclicity property, and it is asserted directly by
//! T-06 rather than left as a reading of the schema.
//!
//! Revision 1 of the design had the two files cover each other. That is a cycle: neither can
//! be verified before the other, so neither was verified at all. The fix is a tree with one
//! root, and the root is the manifest, which is covered by nothing. §C.3.
//!
//! # Why a hashing reader rather than hash-then-read
//!
//! Hashing a file and then reading it is two passes over the bytes and, worse, two chances to
//! disagree: the thing hashed and the thing written are related only by the path they came
//! from. [`HashingReader`] hashes **as the bytes pass through**, so the digest is of exactly
//! what was staged. It also carries the byte cap, so the refusal for "more bytes than
//! declared" arrives at the cap rather than at the end of the stream.

use std::collections::BTreeMap;
use std::io::Read;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, TransferError};
use crate::manifest::DIGEST_ALGORITHM;

/// One digest per covered entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checksums {
    /// Exactly [`DIGEST_ALGORITHM`]. Any other value is a refusal, never a fallback.
    pub algorithm: String,
    /// Entry name to lowercase-hex digest.
    ///
    /// A map rather than a list so a future entry slots in with no `formatVersion` bump —
    /// adding a covered entry is additive, changing the **algorithm** is not.
    pub entries: BTreeMap<String, String>,
}

impl Checksums {
    /// Builds a record covering one entry.
    #[must_use]
    pub fn of(entry: &str, digest: String) -> Self {
        let mut entries = BTreeMap::new();
        entries.insert(entry.to_owned(), digest);
        Self {
            algorithm: String::from(DIGEST_ALGORITHM),
            entries,
        }
    }

    /// Serialises to the exact bytes an archive carries: LF only, trailing newline.
    ///
    /// # Errors
    ///
    /// Propagates a serialisation failure.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text.into_bytes())
    }

    /// The digest recorded for `entry`.
    ///
    /// # Errors
    ///
    /// [`TransferError::ChecksumMismatch`] if the record does not cover the entry at all. A
    /// missing digest is a mismatch rather than a pass: an archive that declines to say what
    /// its payload should hash to has not had its payload verified, and treating that as
    /// success is the one reading that makes the whole chain decorative.
    pub fn digest_of(&self, entry: &str) -> Result<&str> {
        self.entries
            .get(entry)
            .map(String::as_str)
            .ok_or_else(|| TransferError::ChecksumMismatch {
                entry: entry.to_owned(),
            })
    }
}

/// The SHA-256 of a byte slice, lowercase hex.
#[must_use]
pub fn digest_of_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Lowercase hex of a digest.
///
/// Written out rather than taken from a dependency: it is four lines, and `hex` would be a
/// crate in the shipped tree for them.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        // Infallible: writing to a `String` cannot fail, and `expect` here would be noise.
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// A reader that hashes what passes through it and refuses to pass more than a cap.
///
/// Wrapping rather than hashing afterwards, so the digest is of exactly the bytes that were
/// written. See this module's header.
pub struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    read: u64,
    limit: u64,
    entry: String,
}

impl<R: Read> HashingReader<R> {
    /// Wraps `inner`, refusing after `limit` bytes.
    #[must_use]
    pub fn new(inner: R, limit: u64, entry: &str) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            read: 0,
            limit,
            entry: entry.to_owned(),
        }
    }

    /// How many bytes have passed through.
    #[must_use]
    pub const fn bytes_read(&self) -> u64 {
        self.read
    }

    /// The digest of everything that passed through, lowercase hex.
    #[must_use]
    pub fn finish(self) -> String {
        hex(&self.hasher.finalize())
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Clamped so the *last* allowed read cannot overshoot the cap: without this the
        // refusal would be raised only after the excess bytes had already been hashed and
        // handed to the caller, which on the payload path means written to disk.
        let remaining = self.limit.saturating_sub(self.read);
        if remaining == 0 {
            // A zero-length read would be end-of-stream, so a stream with more to give has
            // to be refused rather than silently truncated.
            let mut probe = [0_u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(std::io::Error::other(TooLarge {
                    entry: self.entry.clone(),
                    limit: self.limit,
                })),
            };
        }

        let allowed = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let read = self.inner.read(&mut buf[..allowed])?;
        self.hasher.update(&buf[..read]);
        self.read += read as u64;
        Ok(read)
    }
}

/// The cap refusal, carried through `io::Error` so it survives a `std::io::copy`.
///
/// `io::copy` only ever gives back an `io::Error`, so a cap reached mid-copy has to travel as
/// one and be recognised on the other side. Recognising it by *type* rather than by matching
/// on a message is what makes [`TransferError::EntryTooLarge`] reachable without a string
/// comparison.
#[derive(Debug)]
struct TooLarge {
    entry: String,
    limit: u64,
}

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is larger than {} bytes", self.entry, self.limit)
    }
}

impl std::error::Error for TooLarge {}

/// Turns an I/O failure from a bounded copy into the right refusal.
///
/// A cap reached mid-copy is [`TransferError::EntryTooLarge`]; anything else is the I/O
/// failure it was, named by what was being attempted.
pub(crate) fn classify_copy_failure(action: &'static str, error: std::io::Error) -> TransferError {
    if let Some(too_large) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<TooLarge>())
    {
        return TransferError::EntryTooLarge {
            entry: too_large.entry.clone(),
            limit: too_large.limit,
        };
    }
    TransferError::Io {
        action,
        source: error,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{Checksums, HashingReader, classify_copy_failure, digest_of_bytes};
    use crate::error::TransferError;

    #[test]
    fn the_digest_is_the_published_sha256_of_the_empty_string() {
        // A known-answer test rather than a round trip, so a swapped algorithm is caught
        // rather than being self-consistently wrong.
        assert_eq!(
            digest_of_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest_of_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_hashing_reader_agrees_with_hashing_the_bytes() {
        let bytes = b"a snapshot's worth of bytes, more or less";
        let mut reader = HashingReader::new(&bytes[..], 1024, "payload");
        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read");
        assert_eq!(out, bytes);
        assert_eq!(reader.bytes_read(), bytes.len() as u64);
        assert_eq!(reader.finish(), digest_of_bytes(bytes));
    }

    #[test]
    fn the_cap_refuses_before_the_excess_byte_is_handed_over() {
        let bytes = [0_u8; 100];
        let mut reader = HashingReader::new(&bytes[..], 10, "payload/project.evaraproj");
        let mut out = Vec::new();
        let error = reader
            .read_to_end(&mut out)
            .expect_err("the cap must refuse");

        assert_eq!(out.len(), 10, "exactly the cap was passed through, no more");
        match classify_copy_failure("stage the campus", error) {
            TransferError::EntryTooLarge { entry, limit } => {
                assert_eq!(entry, "payload/project.evaraproj");
                assert_eq!(limit, 10);
            }
            other => panic!("expected EntryTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn a_stream_exactly_at_the_cap_is_not_refused() {
        let bytes = [7_u8; 10];
        let mut reader = HashingReader::new(&bytes[..], 10, "payload");
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .expect("ten bytes under a ten-byte cap is fine");
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn an_unlisted_entry_is_a_mismatch_and_not_a_pass() {
        let checksums = Checksums::of("payload/project.evaraproj", String::from("ab"));
        checksums
            .digest_of("payload/project.evaraproj")
            .expect("the covered entry");
        match checksums.digest_of("manifest.json") {
            Err(TransferError::ChecksumMismatch { entry }) => {
                assert_eq!(entry, "manifest.json");
            }
            other => panic!("an unlisted entry must not read as verified: {other:?}"),
        }
    }

    #[test]
    fn the_manifest_is_never_listed() {
        // The acyclicity assertion, checksums half.
        let checksums = Checksums::of("payload/project.evaraproj", String::from("ab"));
        assert!(!checksums.entries.contains_key(crate::entry::MANIFEST_ENTRY));
    }

    #[test]
    fn an_ordinary_io_failure_stays_an_io_failure() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        match classify_copy_failure("stage the campus", error) {
            TransferError::Io { action, .. } => assert_eq!(action, "stage the campus"),
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
