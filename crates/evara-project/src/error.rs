//! What a transfer refuses, and the one rule every variant here obeys.
//!
//! # No driver type crosses this boundary
//!
//! `CLAUDE.md`'s rule for `evara-db` and `rusqlite`, applied to this crate and `zip`: no
//! `zip::result::ZipError` and no `rusqlite::Error` appears in any variant below, and the
//! crate asserts it the way `evara-db` does — **by not having the dev-dependency**, so a
//! test could not name one even if somebody tried. T-62.
//!
//! A `zip` failure therefore arrives as [`TransferError::NotAnArchive`], or as the specific
//! refusal the reader turned it into. That is a deliberate loss of detail: `zip`'s error
//! enumeration is its own API surface, and a `TransferError` that re-exported it would make
//! a `zip` major bump a breaking change to this crate's public type.
//!
//! # Every variant names a file or a field
//!
//! A refusal a user cannot act on is a refusal that becomes a bug report. So each variant
//! carries the entry, the field or the figure involved — and never the full path, because a
//! message is shown in an interface and a path is not what is wrong.

use std::path::PathBuf;

use evara_db::DbError;

/// A transfer that was refused, and why.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransferError {
    /// Not a ZIP archive at all, or one whose central directory will not read.
    #[error("{name} is not an Evara archive")]
    NotAnArchive {
        /// File name, never the path.
        name: String,
    },

    /// No `manifest.json` entry.
    #[error("{name} has no manifest, so Evara cannot tell what is in it")]
    ManifestMissing {
        /// File name.
        name: String,
    },

    /// The manifest is not JSON, or is missing a field a reader needs first.
    #[error("this archive's manifest cannot be read ({detail})")]
    ManifestUnreadable {
        /// What was wrong with it.
        detail: String,
    },

    /// The manifest's `format` is not this format.
    #[error("this file says it is `{found}`, which is not an Evara campus archive")]
    UnknownFormat {
        /// What it claimed to be.
        found: String,
    },

    /// The container version is newer than this build reads.
    #[error(
        "this archive was written in a newer format (version {found}; this Evara reads {supported}). \
         Update Evara to open it."
    )]
    FormatTooNew {
        /// The version found.
        found: u32,
        /// The highest this build reads.
        supported: u32,
    },

    /// The manifest and the payload disagree about a fact the manifest is not the authority
    /// on.
    ///
    /// A refusal rather than a correction: the payload *is* the authority, so a disagreement
    /// means the archive was not assembled by Evara, and reading on would be reading
    /// something somebody edited by hand.
    #[error("this archive's manifest disagrees with the campus inside it about `{field}`")]
    ManifestDisagreesWithPayload {
        /// Which field disagreed.
        field: &'static str,
    },

    /// The archive needs something this build does not implement.
    ///
    /// Never partially imported and never offered as "drop the rule": dropping a constraint
    /// silently is how a timetable comes to be wrong.
    #[error(
        "this campus uses `{capability}`, which this version of Evara does not support. \
         Update Evara to import it."
    )]
    UnsupportedCapability {
        /// The capability named in the manifest.
        capability: String,
    },

    /// A digest algorithm other than SHA-256.
    ///
    /// One value is admitted. An algorithm field that accepted alternatives would be a
    /// downgrade surface, and with no signature there is nothing to negotiate.
    #[error("this archive's checksums use `{algorithm}`, which Evara does not accept")]
    UnsupportedDigest {
        /// What was declared.
        algorithm: String,
    },

    /// An entry the format requires is not there.
    #[error("this archive is incomplete: `{entry}` is missing")]
    EntryMissing {
        /// Which entry.
        entry: &'static str,
    },

    /// An entry name that is not safe to have in an archive at all.
    ///
    /// Not merely ignored. An archive carrying a traversal attempt is not one to read the
    /// rest of, whatever else it contains — which is why this refuses the archive rather
    /// than skipping the entry.
    #[error("this archive contains an unsafe entry name, so Evara will not read it")]
    UnsafeEntryName {
        /// The name, escaped for display.
        entry: String,
        /// Which rule it broke.
        reason: &'static str,
    },

    /// An entry compressed with a method this reader does not accept.
    ///
    /// `Stored` and `Deflate` only, which closes off zstd, bzip2, lzma and ppmd decoders and
    /// their bombs without reasoning about any of them.
    #[error("`{entry}` in this archive uses a compression Evara does not read")]
    UnsupportedCompression {
        /// Which entry.
        entry: String,
    },

    /// An entry is encrypted.
    #[error("`{entry}` in this archive is encrypted, and Evara archives are never encrypted")]
    EntryEncrypted {
        /// Which entry.
        entry: String,
    },

    /// An entry declares, or produces, more bytes than its cap.
    #[error("`{entry}` in this archive is larger than Evara will read ({limit} bytes)")]
    EntryTooLarge {
        /// Which entry.
        entry: String,
        /// The cap it exceeded.
        limit: u64,
    },

    /// An entry declares a decompression ratio only a bomb reaches.
    #[error(
        "`{entry}` in this archive expands {ratio} times, which Evara refuses as a \
         decompression bomb"
    )]
    EntryExpandsTooFar {
        /// Which entry.
        entry: String,
        /// The declared ratio.
        ratio: u64,
    },

    /// More entries than the cap.
    #[error("this archive has {found} entries, and Evara reads at most {limit}")]
    TooManyEntries {
        /// How many were found.
        found: usize,
        /// The cap.
        limit: usize,
    },

    /// Two entries with one name.
    ///
    /// Refused rather than resolved. Which of the two a reader gets is a property of the
    /// reader, so an archive with a duplicated name is one where two readers legitimately
    /// disagree about the contents — and the disagreement would be invisible.
    #[error("this archive names `{entry}` twice, so what it contains is ambiguous")]
    DuplicateEntryName {
        /// The repeated name.
        entry: String,
    },

    /// A directory or symlink entry.
    ///
    /// Refused because no entry's attributes are honoured and nothing is extracted by name:
    /// an entry that is not a file has nothing to contribute and its presence means the
    /// archive was built by something other than Evara.
    #[error("this archive contains `{entry}`, which is not a file Evara can read")]
    EntryNotAFile {
        /// Which entry.
        entry: String,
    },

    /// The payload was shorter than the manifest declared.
    #[error("this archive's campus is {actual} bytes, and its manifest says {declared}")]
    PayloadLengthMismatch {
        /// What the manifest declared.
        declared: u64,
        /// What the stream produced.
        actual: u64,
    },

    /// A digest did not match.
    #[error("`{entry}` in this archive is damaged; ask for a fresh export")]
    ChecksumMismatch {
        /// Which entry failed.
        entry: String,
    },

    /// The archive ended mid-entry.
    #[error("this archive is incomplete; the copy may have been interrupted")]
    Truncated,

    /// A destination that is already occupied, and the caller did not ask to replace it.
    #[error("{name} already exists")]
    DestinationExists {
        /// File name.
        name: String,
    },

    /// A destination that is not a `.evaraproj`.
    #[error("an imported campus must be saved with the {expected} extension")]
    WrongExtension {
        /// The extension required.
        expected: &'static str,
    },

    /// A destination whose path Evara will not write to.
    #[error("Evara will not write to {name} because {reason}")]
    DestinationUnsafe {
        /// File name.
        name: String,
        /// Why not.
        reason: &'static str,
    },

    /// A replacement of this destination is already in flight.
    #[error("{name} is already being replaced; wait for that to finish or restart Evara")]
    ReplacementInFlight {
        /// File name.
        name: String,
    },

    /// The caller stopped it.
    ///
    /// Nothing was created: a cancelled export leaves no archive, no `.part` and no
    /// snapshot, and a cancelled import leaves no destination and no staging.
    #[error("the transfer was stopped before it finished")]
    Cancelled,

    /// A filesystem operation failed, named by what was being attempted.
    #[error("could not {action}")]
    Io {
        /// What was being attempted, in words.
        action: &'static str,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// A rename could not be completed, after the bounded retry.
    #[error("another program is using {name}; close it and try again")]
    RenameBlocked {
        /// File name.
        name: String,
    },

    /// A state recovery found but will not act on.
    ///
    /// Reported and never guessed at. Guessing is how the `PLACED`-with-an-absent-destination
    /// case loses a project, which is the data-loss case safeguard S2 exists for.
    #[error("an interrupted import of {name} is in a state Evara will not resolve on its own")]
    RecoveryAmbiguous {
        /// File name.
        name: String,
        /// Which ambiguity, in words.
        detail: String,
    },

    /// The database layer refused.
    ///
    /// Wrapped rather than flattened: `DbError` already carries the remedies for a corrupt
    /// payload, a non-conforming schema and a schema from a newer build, and restating them
    /// here would be two places for one sentence.
    #[error(transparent)]
    Database(#[from] DbError),
}

/// What a transfer produces, or why it was refused.
pub type Result<T> = std::result::Result<T, TransferError>;

impl TransferError {
    /// Builds an [`TransferError::Io`] for a named action.
    pub(crate) fn io(action: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { action, source }
    }

    /// Whether this refusal means the user's own files are untouched.
    ///
    /// True for every refusal raised before anything is renamed into place, which is all of
    /// them except [`Self::RenameBlocked`] and [`Self::RecoveryAmbiguous`] — and even those
    /// two leave the previous project where it was, because nothing in Evara deletes it.
    /// Exposed so an interface can say *"nothing was changed"* truthfully rather than
    /// hopefully.
    #[must_use]
    pub const fn left_everything_alone(&self) -> bool {
        !matches!(
            self,
            Self::RenameBlocked { .. } | Self::RecoveryAmbiguous { .. }
        )
    }
}

/// Where a recovered replacement ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// The destination the interrupted replacement was aimed at.
    pub destination: PathBuf,
    /// What recovery did, in words a user can be shown.
    pub outcome: String,
    /// Whether the journal row was cleared, so it will not be looked at again.
    pub resolved: bool,
}
