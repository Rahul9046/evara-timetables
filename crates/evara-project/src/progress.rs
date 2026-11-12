//! Where a transfer has got to, as a phase and two numbers.
//!
//! # Why a plain callback and not the solver's sink
//!
//! `evara-solver`'s `ProgressSink` reports placements of a total after so many steps, which
//! is a solve's shape. A transfer's is a phase and a byte count, and widening `evara-core`
//! for a second caller is the opposite of the reason `Cancel` moved there — a type belongs in
//! `evara-core` when several crates must **agree** on it, and nothing in the solver has an
//! opinion about compressing a file.
//!
//! So this is a small trait of this crate's own, and `evara-db`'s snapshot reports pages
//! through a plain `FnMut` of its own, and the Tauri layer is where the two become one
//! coalesced event stream — which is where a caller actually needs them to agree.

/// Which part of a transfer is running.
///
/// Named rather than a percentage, because the phases take wildly different times and a
/// single bar that spends ninety per cent of its life in one phase tells a user less than a
/// label does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Taking the consistent copy, under the workspace lock.
    Snapshotting,
    /// Digesting the copy.
    Hashing,
    /// Writing the archive.
    Compressing,
    /// Streaming a payload out of an archive into staging.
    Staging,
    /// Auditing a staged payload.
    Verifying,
    /// Migrating a staged payload forward.
    Migrating,
    /// Renaming into place.
    Finalising,
}

impl Phase {
    /// A short label, for a progress line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Snapshotting => "snapshotting",
            Self::Hashing => "hashing",
            Self::Compressing => "compressing",
            Self::Staging => "staging",
            Self::Verifying => "verifying",
            Self::Migrating => "migrating",
            Self::Finalising => "finalising",
        }
    }
}

/// Somewhere to report progress to.
///
/// `&self` rather than `&mut self`, so one sink can be shared with a worker thread without a
/// lock at the call site — the shape `CoalescingSink` already has in the command layer.
pub trait TransferProgress: Send + Sync {
    /// How far `phase` has got, out of `total`.
    ///
    /// `total` may be `0` when it is not yet known. An implementation must not divide by it.
    fn progress(&self, phase: Phase, done: u64, total: u64);
}

/// A sink that discards everything.
///
/// Named rather than an `Option<&dyn TransferProgress>`, so a caller that genuinely wants no
/// progress says so instead of leaving something that reads as an oversight — the same
/// argument `evara_core::scheduling::NeverCancel` is written down with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IgnoreProgress;

impl TransferProgress for IgnoreProgress {
    fn progress(&self, _phase: Phase, _done: u64, _total: u64) {}
}
