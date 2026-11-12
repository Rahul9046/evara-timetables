//! The caps every read is bounded by, and the number each one is.
//!
//! # Why a bound exists before a digest does
//!
//! A `.evara` arrives from somewhere else. Its central directory says how big each entry is,
//! and **the central directory is part of the archive** — so a declared size is a claim by
//! the sender and not a fact. Every read here is therefore bounded by a constant *this build*
//! chose, applied through a limited reader, so a 1 KiB entry that decompresses to a gigabyte
//! stops at the cap instead of at the end of memory.
//!
//! The digests cannot help: a digest is known only once the bytes have been read, which is
//! after the damage would have been done. That ordering is the whole reason §3.3's
//! verification order exists and the reason these are constants rather than parameters.
//!
//! # The numbers, and why each is what it is
//!
//! | Cap | Value | Reasoning |
//! | --- | --- | --- |
//! | [`MAX_MANIFEST_BYTES`] | 64 KiB | the manifest is counts, a school list and digests. A real one at ten schools is a few kilobytes; 64 KiB is two orders of margin and still small enough to hold in memory before parsing |
//! | [`MAX_CHECKSUMS_BYTES`] | 1 MiB | one entry per archive entry, and [`MAX_ENTRIES`] bounds that at 64. A megabyte is absurdly generous, which is the point: the cap is a backstop, not a budget |
//! | [`MAX_PAYLOAD_BYTES`] | 2 GiB | §7.3's ten-times project is 60–150 MiB. 2 GiB is an order beyond the largest project anyone has described and below the point where a 32-bit size would be a question |
//! | [`MAX_ENTRIES`] | 64 | v1 writes three. A reader tolerates unknown-but-safe entries for forward compatibility, and 64 is generous for that while refusing the 10 000-entry archive whose cost is in the directory itself |
//! | [`MAX_ENTRY_NAME_BYTES`] | 128 | the three names are 13, 14 and 25 bytes. A 4 KiB name is not a name |
//! | [`MAX_COMPRESSION_RATIO`] | 2 000 | a SQLite page of zeroes deflates to almost nothing, so a *legitimate* payload ratio can be high. 2 000 is above anything a real project reaches and far below a zip bomb's millions |
//! | [`COPY_BUFFER_BYTES`] | 1 MiB | the one buffer a stream uses, so memory is O(1) in project size |
//!
//! # These are refusals, not negotiations
//!
//! An archive over a cap is refused and named. It is never read "as far as the cap and then
//! treated as truncated", because a partially read payload whose digest therefore fails
//! would report corruption for what is actually a resource refusal — two different things to
//! tell a user, and the remedy differs.

/// Largest `manifest.json` this build will read.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Largest `checksums.json` this build will read.
pub const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;

/// Largest payload this build will stage.
pub const MAX_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Most entries an archive may hold.
pub const MAX_ENTRIES: usize = 64;

/// Longest entry name, in bytes of its stored form.
pub const MAX_ENTRY_NAME_BYTES: usize = 128;

/// Highest uncompressed-to-compressed ratio an entry may declare.
///
/// Applied to the **declared** sizes in the central directory, before a byte is decompressed,
/// so a bomb of that shape is refused rather than discovered.
///
/// # This cap cannot fire on a Deflate entry, and that is worth knowing
///
/// Deflate's own maximum expansion is **1032:1** — its longest match is 258 bytes from a
/// minimum-cost reference, and that bound is a property of the format rather than of an
/// encoder. So no `Deflate` entry can declare a ratio above ~1032, and this cap at 2 000 is
/// unreachable for the two methods `formatVersion 1` admits. `a_deflate_entry_cannot_declare_a_bomb_ratio`
/// is the test that records it.
///
/// It is kept anyway, for two reasons, and lowering it was considered and rejected:
///
/// - **A future `formatVersion` admitting another method** — zstd reaches millions to one —
///   would need exactly this check, and a cap added at that point is a cap nobody tested.
/// - **Lowering it below 1032 would refuse legitimate payloads.** A snapshot taken by the
///   backup API carries the source's free pages, which are mostly zeroes and deflate almost
///   completely; a heavily edited project's payload legitimately approaches the ceiling.
///
/// **What actually bounds a Deflate bomb** is therefore not this: it is
/// [`MAX_PAYLOAD_BYTES`] and the manifest's declared `byteLength`, applied through a limited
/// reader so a stream that produces more than it declared stops at the bound rather than at
/// the end of the disk.
pub const MAX_COMPRESSION_RATIO: u64 = 2_000;

/// The one buffer a stream copies through.
pub const COPY_BUFFER_BYTES: usize = 1024 * 1024;
