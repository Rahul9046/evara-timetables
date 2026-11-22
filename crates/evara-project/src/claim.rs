//! Holding a destination's name while something long is written to a temporary file.
//!
//! # Why a claim and not a check
//!
//! Both halves of transfer finish the same way: content is written somewhere else and then
//! **renamed** onto the name the user chose — `seal` renames its `.part`, and `import` renames
//! the staged payload. `rename` replaces whatever is at the name, so the decision "may I write
//! here?" has to be made somewhere, and the obvious place to make it is wrong:
//!
//! ```text
//! if destination.exists() { refuse }     <- the answer
//!     ... snapshot, hash, compress ...   <- seconds, sometimes minutes
//! rename(part, destination)              <- the action
//! ```
//!
//! A file created by another program inside that window is destroyed without anybody being
//! asked, and nothing in the result says so. The gap is not closable by checking again —
//! checking twice is the same race with a shorter window.
//!
//! [`DestinationClaim`] closes it instead: it **creates** the destination, with
//! `create_new(true)`, so the question and the answer are one filesystem operation. Two
//! processes racing for one name have exactly one winner, and the loser is told the name is
//! occupied, which is true. The placeholder it leaves is replaced by the final rename, so
//! the rename has nothing of anybody else's to destroy.
//!
//! It is also the only atomic claim available here: `unsafe_code` is forbidden
//! workspace-wide, so there is no `O_EXCL` by FFI and no platform lock to take.
//!
//! # What it does not claim
//!
//! A claim is not a lock, and this module does not pretend otherwise. It proves that *at the
//! instant of the call* nothing was at that name and that this process now holds it. An
//! ancestor directory swapped for a link, or a destination removed by a user between the claim
//! and the rename, are outside what any check or claim in userspace can prevent —
//! [`crate::paths::require_safe_destination`]'s `# TOCTOU` section states that bound rather
//! than claiming more, and this is the other half of the same honesty.
//!
//! So the ordering matters, and every caller follows it: **judge the path, then claim it.**
//! An extension refusal and [`crate::paths::require_safe_destination`] run first, so a path
//! Evara will not write to is refused without a file ever being made at it — and a destination
//! that is a *symlink* is reported as the link it is rather than as an occupied name.
//!
//! # One implementation, three callers
//!
//! `evara-project`'s import, the CLI's export and the desktop's export all need exactly this
//! device, and `CLAUDE.md` §2 makes a rule implemented twice an architecture bug. The policy
//! on top of it differs — an import offers `Replace`, an export refuses outright, and the
//! desktop phrases both for a screen — but the mechanism is here, once.

use std::path::{Path, PathBuf};

use crate::error::{Result, TransferError};
use crate::paths::file_name_of;

/// The zero-length placeholder that holds a destination's name.
///
/// Taken with [`DestinationClaim::take`], given up with [`DestinationClaim::released`] once a
/// rename has replaced it, and otherwise removed when it drops — so a refusal, a failure or a
/// cancellation after the claim leaves the folder exactly as it was found.
///
/// The file it removes is only ever a zero-length one this process created moments ago, so
/// the cleanup destroys nothing. See the [module docs](self) for why this is a claim rather
/// than a check.
#[derive(Debug)]
pub struct DestinationClaim {
    path: PathBuf,
    released: bool,
}

impl DestinationClaim {
    /// Claims `destination`, refusing if anything is already there.
    ///
    /// *Anything*: `create_new` fails if the path exists **at all**, so a zero-length file is
    /// occupied rather than adoptable, which is §0.2 S1's rule and deliberately unlike
    /// `Database::create`. A symlink is occupied too — `O_EXCL` fails on one whatever it
    /// points at — which is why callers judge the path before reaching here.
    ///
    /// # Errors
    ///
    /// - [`TransferError::DestinationExists`] — the name was taken, by a file, a directory or
    ///   a link, and this call is the proof rather than a prediction.
    /// - [`TransferError::Io`] — the name could not be created for some other reason: a
    ///   missing folder, a read-only volume, a permission.
    pub fn take(destination: &Path) -> Result<Self> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| match error.kind() {
                // Somebody else holds the name. Reported as the occupied destination it now
                // is, which is the same answer a check would have given a moment earlier —
                // and the reason this is a claim rather than a check.
                std::io::ErrorKind::AlreadyExists => TransferError::DestinationExists {
                    name: file_name_of(destination),
                },
                _ => TransferError::Io {
                    action: "claim the destination",
                    source: error,
                },
            })
            .map(|_| Self {
                path: destination.to_path_buf(),
                released: false,
            })
    }

    /// The path this claim holds.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A rename replaced it, so it is no longer ours to remove.
    ///
    /// Consumes the claim and sets the flag its `Drop` reads, which is the idiom `Snapshot`
    /// and the `.part` file use: the cleanup stays structural and the release is a single
    /// named call rather than a flag a caller might forget.
    pub fn released(mut self) {
        self.released = true;
    }
}

impl Drop for DestinationClaim {
    fn drop(&mut self) {
        if !self.released {
            // Only ever a zero-length file this process made moments ago, so removing it
            // destroys nothing. A placeholder left behind would read as an empty project or
            // an empty portable file in a folder listing, which is worse than nothing at all.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
