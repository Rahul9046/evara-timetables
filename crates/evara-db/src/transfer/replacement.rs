//! The replacement journal, and the decision function recovery is driven by.
//!
//! # Safeguard S2: the journal records *intent*; the filesystem is the truth
//!
//! The journal row is committed **before** the rename it authorises. That is what makes the
//! protocol recoverable at all — but it also means a crash in that window leaves the phase
//! *ahead* of the disk, and a recovery that mapped a phase straight to an action would act
//! on something that never happened.
//!
//! The concrete data-loss case, which the first draft of the design got wrong: the journal
//! says `PLACED`, but the crash landed between that commit and the rename. On disk the
//! destination is **absent**, the staged file is still in staging, and the old project is in
//! `staging/aside/`. Read literally, `PLACED` means "finish step 3 — move the aside set to
//! the keep-name", which moves the user's only project to a `.replaced-` name and leaves
//! **no project at the destination at all**.
//!
//! So [`reconcile`] takes **observed facts** and the journal row, and the facts decide. One
//! invariant dominates every row of the table and is asserted in
//! [`RecoveryAction::releases_the_old_set`]:
//!
//! > **The old file set is released only once a destination that looks like a project has
//! > been observed.**
//!
//! Everything that does not satisfy it rolls back or reports. An ambiguous state is
//! **reported and acted on by nothing** — guessing is how the `PLACED` case above loses a
//! project.
//!
//! # Why this function is pure
//!
//! [`reconcile`] performs no I/O. Every row of the table can therefore be driven from
//! fabricated [`DiskFacts`] in a unit test, including the ambiguous ones that a
//! process-killing test would reach rarely or never. The kill tests still exist — they prove
//! the fabricated combinations are states that genuinely occur — but coverage comes from
//! here, which is what makes covering all of it affordable.
//!
//! # Safeguard S3, carried into recovery
//!
//! [`DiskFacts`] reads sidecar presence **per file**, and a main file with any sidecar beside
//! it does **not** count as a project. A new main file with a foreign `-wal` opens cleanly
//! and is wrong, so the only safe reading of an unexplained sidecar is that the destination
//! is not trustworthy yet. Recovery never deletes a `-wal` to make a destination look clean:
//! that discards committed transactions the main file does not hold.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::sidecars::SidecarState;

/// How far a replacement had got, as the journal recorded it.
///
/// An *intent*, not a description of the disk. See this module's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementPhase {
    /// Staging exists and has been written to. Nothing outside it has been touched, except
    /// possibly a zero-length claim at the destination.
    Staged,
    /// The destination's old file was moved into `staging/aside/`.
    Aside,
    /// The new file was renamed to the destination.
    Placed,
}

impl ReplacementPhase {
    /// The value stored in the `phase` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "STAGED",
            Self::Aside => "ASIDE",
            Self::Placed => "PLACED",
        }
    }

    /// Parses the stored value.
    ///
    /// Returns `None` for anything else. The column has a `CHECK` constraint, so an
    /// unrecognised value means the row was written by a build that knows a phase this one
    /// does not — which is a reason to leave it alone, not to guess at it.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "STAGED" => Some(Self::Staged),
            "ASIDE" => Some(Self::Aside),
            "PLACED" => Some(Self::Placed),
            _ => None,
        }
    }
}

/// One in-flight replacement, as recorded in the settings database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingReplacement {
    /// Row identity, UUIDv7.
    pub id: Uuid,
    /// How far the protocol intended to have got, or `None` for a stored phase this build
    /// does not recognise.
    ///
    /// An `Option` rather than a value with a default, because a row written by a newer
    /// build describes a protocol this one cannot see and the only safe reading of it is
    /// "unknown". It is passed straight to [`reconcile`], which reports
    /// [`Ambiguity::UnknownPhase`] and touches nothing — whereas substituting any of the
    /// three known phases would have recovery act on an intent nobody recorded.
    pub phase: Option<ReplacementPhase>,
    /// The project file being replaced.
    pub destination: PathBuf,
    /// The staging directory, a sibling of the destination.
    pub staging_dir: PathBuf,
    /// Where the old file will be kept, once there is one to keep.
    pub keep_name: Option<PathBuf>,
    /// When the replacement began, ISO-8601 UTC.
    pub started_at: String,
    /// When the row last advanced.
    pub updated_at: String,
}

impl PendingReplacement {
    /// Path of the staged project inside the staging directory.
    #[must_use]
    pub fn staged_file(&self) -> PathBuf {
        self.staging_dir.join("staged.evaraproj")
    }

    /// Directory the old file set is moved into.
    #[must_use]
    pub fn aside_dir(&self) -> PathBuf {
        self.staging_dir.join("aside")
    }

    /// Path of the old project inside the aside directory.
    #[must_use]
    pub fn aside_file(&self) -> PathBuf {
        self.aside_dir().join(
            self.destination
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("project.evaraproj")),
        )
    }
}

/// What is at the destination path.
///
/// An enum rather than three booleans. `present`, `is_project` and `zero_length` encode one
/// four-state fact, and as separate flags they admit combinations that cannot exist —
/// "absent but is a project", "a project that is zero length" — which every reader of
/// [`reconcile`] would then have to reason about. Clippy's `struct_excessive_bools` pointed
/// at the symptom; this is the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationState {
    /// Nothing is at the path.
    Absent,
    /// A readable project with **no sidecar beside it**.
    ///
    /// The conjunction is part of the state, not a separate flag, so no caller can observe
    /// one half and act on it. S3: a main file with a stray `-wal` opens cleanly and is
    /// wrong, so it is not a `Project` however well it reads.
    Project,
    /// Something is there that may not be blessed as a project.
    ///
    /// Covers a file that is not a database, a database that is not a project, a project
    /// with a sidecar beside it, and our own abandoned zero-length claim.
    Unknown {
        /// Whether it is zero length.
        ///
        /// The one case recovery may remove, and only with staging alongside — together
        /// those mean the file was **our** claim. A file with bytes in it is somebody's and
        /// is never removed. S1.
        zero_length: bool,
    },
}

impl DestinationState {
    /// Whether the old file set may be released on the strength of this destination.
    #[must_use]
    pub const fn is_blessable_project(self) -> bool {
        matches!(self, Self::Project)
    }

    /// Whether anything is at the path at all.
    #[must_use]
    pub const fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// What is actually on disk, read before any decision is made.
///
/// Every field is an observation. Nothing here is inferred from the journal, which is the
/// entire point: see this module's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskFacts {
    /// What is at the destination path.
    pub destination: DestinationState,
    /// `staging/staged.evaraproj` exists.
    pub staged_present: bool,
    /// The aside directory holds the old project.
    pub aside_present: bool,
    /// Something already exists at the keep-name.
    pub keep_name_present: bool,
    /// Which sidecars are beside the destination, read one at a time. S3.
    pub destination_sidecars: SidecarState,
    /// Which sidecars travelled with the aside set.
    pub aside_sidecars: SidecarState,
}

impl DiskFacts {
    /// Reads the facts for `row`.
    ///
    /// The one place in this module that touches the filesystem. `is_project` is supplied by
    /// the caller because recognising a project means opening it, which belongs to
    /// [`crate::Database`] rather than here — and injecting it keeps [`reconcile`]'s inputs
    /// constructible in a test without a real database.
    #[must_use]
    pub fn observe(row: &PendingReplacement, is_project: &dyn Fn(&Path) -> bool) -> Self {
        let path = &row.destination;
        let destination_sidecars = SidecarState::observe(path);

        // `symlink_metadata`, so a symlink at the destination counts as present rather than
        // being followed to wherever it points.
        let destination = match path.symlink_metadata() {
            Err(_) => DestinationState::Absent,
            Ok(metadata) => {
                // The S3 conjunction, applied once here so no caller can forget half of it.
                if !destination_sidecars.any() && is_project(path) {
                    DestinationState::Project
                } else {
                    DestinationState::Unknown {
                        zero_length: metadata.len() == 0,
                    }
                }
            }
        };

        Self {
            destination,
            staged_present: row.staged_file().symlink_metadata().is_ok(),
            aside_present: row.aside_file().symlink_metadata().is_ok(),
            keep_name_present: row
                .keep_name
                .as_ref()
                .is_some_and(|path| path.symlink_metadata().is_ok()),
            destination_sidecars,
            aside_sidecars: SidecarState::observe(&row.aside_file()),
        }
    }
}

/// Why a state could not be acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ambiguity {
    /// The destination holds a project *and* a staged file is still waiting *and* the old
    /// set is aside. Two of those three are expected at once; all three are not, and any
    /// choice between them could be the wrong one.
    DestinationAndStagedAndAside,
    /// Nothing is anywhere: no destination, nothing staged, nothing aside. There is nothing
    /// to restore and nothing to finish.
    NothingRemains,
    /// The destination holds something that is not a project, and no old set is aside to
    /// put back. Recovery has no material to work with and must not delete what is there.
    UnknownFileAndNothingAside,
    /// The destination has a sidecar beside it that recovery cannot account for.
    ///
    /// Never resolved by deleting the sidecar: a `-wal` may hold committed transactions the
    /// main file does not.
    UnaccountableSidecar,
    /// The journal row names a phase this build does not recognise.
    UnknownPhase,
}

/// What recovery should do, decided from observed facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Delete the staging directory, and the destination **only if** it is our own
    /// zero-length claim. Rolls back.
    RollBackDeletingStaging {
        /// Whether a zero-length claim at the destination is ours to remove.
        remove_claim: bool,
    },
    /// Move the aside set back to the destination, then delete staging. Rolls back.
    RestoreAside {
        /// Whether a staging directory is also present and should go afterwards.
        then_delete_staging: bool,
    },
    /// Set the unrecognised destination file aside, then restore the old project over it.
    ///
    /// Rolls back. The unknown file is **kept**, never deleted — the same rule that governs
    /// the project being replaced.
    DisplaceUnknownThenRestoreAside,
    /// Move the aside set to the keep-name, finishing the replacement.
    ///
    /// The **only** action that releases the old set, and reachable only with a destination
    /// that looks like a project.
    FinishMovingAsideToKeepName,
    /// Delete the staging directory and then the journal row. The work is done.
    FinishDeletingStaging,
    /// Delete the journal row. Nothing is left to do.
    AlreadyFinished,
    /// Report and change nothing.
    Report(Ambiguity),
}

impl RecoveryAction {
    /// Whether this action lets go of the project being replaced.
    ///
    /// The invariant S2 turns on, expressed as a property of the action so a test can assert
    /// it over every reachable combination rather than over the rows somebody remembered to
    /// write down.
    #[must_use]
    pub const fn releases_the_old_set(self) -> bool {
        matches!(self, Self::FinishMovingAsideToKeepName)
    }

    /// Whether this action changes anything on disk.
    #[must_use]
    pub const fn is_inert(self) -> bool {
        matches!(self, Self::Report(_) | Self::AlreadyFinished)
    }
}

/// Decides what recovery should do. **Pure** — no I/O, no clock, no filesystem.
///
/// `phase` is taken as an `Option` so an unrecognised stored value reaches here as `None`
/// and is reported rather than parsed into a guess.
///
/// # The table
///
/// | destination | staged | aside | action |
/// | --- | --- | --- | --- |
/// | absent | present | absent | roll back, delete staging |
/// | absent | present | present | restore aside, then delete staging |
/// | absent | absent | present | restore aside |
/// | absent | absent | absent | **report** — nothing remains |
/// | project | absent | present | finish: aside → keep-name |
/// | project | absent | absent | already finished |
/// | project | present | present | **report** — ambiguous |
/// | project | present | absent | finish: delete staging |
/// | not a project | any | present | displace the unknown file, restore aside |
/// | zero-length claim | present | absent | roll back, remove the claim |
/// | not a project | any | absent | **report** |
#[must_use]
pub fn reconcile(facts: DiskFacts, phase: Option<ReplacementPhase>) -> RecoveryAction {
    // An unrecognised phase means a newer build wrote the row. Its protocol may differ in
    // ways this one cannot see, so nothing is touched.
    if phase.is_none() {
        return RecoveryAction::Report(Ambiguity::UnknownPhase);
    }

    // A sidecar at a destination that is otherwise a project is the S3 state recovery must
    // not paper over. Such a destination is `Unknown`, so without this arm the combination
    // would fall through to the "not a project" rows and be *displaced* — moving a
    // possibly-sound project aside because of a file we could have reported instead.
    if facts.destination.is_present() && facts.destination_sidecars.any() && !facts.aside_present {
        return RecoveryAction::Report(Ambiguity::UnaccountableSidecar);
    }

    match (facts.destination, facts.staged_present, facts.aside_present) {
        // ---- The destination is gone. Every arm rolls backwards. ----
        (DestinationState::Absent, true, false) => RecoveryAction::RollBackDeletingStaging {
            remove_claim: false,
        },
        (DestinationState::Absent, true, true) => RecoveryAction::RestoreAside {
            then_delete_staging: true,
        },
        (DestinationState::Absent, false, true) => RecoveryAction::RestoreAside {
            then_delete_staging: false,
        },
        // Nothing anywhere. Not recoverable and not ignorable: the user is told, and
        // nothing is invented.
        (DestinationState::Absent, false, false) => {
            RecoveryAction::Report(Ambiguity::NothingRemains)
        }

        // ---- The destination is a project. Only these arms may release the old set. ----
        (DestinationState::Project, false, true) => RecoveryAction::FinishMovingAsideToKeepName,
        (DestinationState::Project, false, false) => RecoveryAction::AlreadyFinished,
        // All three present. Expected pairs exist; this triple does not, and either choice
        // could be the wrong one.
        (DestinationState::Project, true, true) => {
            RecoveryAction::Report(Ambiguity::DestinationAndStagedAndAside)
        }
        (DestinationState::Project, true, false) => RecoveryAction::FinishDeletingStaging,

        // ---- The destination holds something that may not be blessed. ----
        //
        // With the old set aside there is material to roll back to, and the unknown file is
        // kept rather than destroyed.
        (DestinationState::Unknown { .. }, _, true) => {
            RecoveryAction::DisplaceUnknownThenRestoreAside
        }
        // A zero-length file with staging alongside is our own abandoned claim: S1's
        // Create-mode path, interrupted. Ours to remove, and only because it is empty.
        (DestinationState::Unknown { zero_length: true }, true, false) => {
            RecoveryAction::RollBackDeletingStaging { remove_claim: true }
        }
        // Someone's file, and nothing aside to restore. Reported; never deleted.
        (DestinationState::Unknown { .. }, _, false) => {
            RecoveryAction::Report(Ambiguity::UnknownFileAndNothingAside)
        }
    }
}

/// Whether a destination may be **claimed** for a Create-mode import.
///
/// # Safeguard S1
///
/// The approved design claimed a destination that was *"absent, or present and zero
/// length"* with `OpenOptions::new().write(true).create_new(true)`. Those two halves cannot
/// both hold: `create_new` fails if the path exists **at all**, zero length included.
///
/// The resolution keeps `create_new` — it is the only atomic claim available without FFI,
/// and `unsafe_code` is forbidden workspace-wide — and narrows Create to a destination that
/// does **not exist**. An existing zero-length file is *occupied*: it needs an explicit
/// `Replace`, which keeps it as a `.replaced-` copy, so even an unknown empty file is never
/// destroyed.
///
/// This **deliberately diverges from [`crate::Database::create`]**, which treats a
/// zero-length file as creatable. The two are different acts: `Database::create` is making a
/// project where the user just said to, while an import adopting an unknown empty file would
/// be writing over something nobody has looked at. The divergence is stated rather than
/// reconciled.
///
/// Rejected alternatives, for the record: `renameat2(RENAME_NOREPLACE)`,
/// `renamex_np(RENAME_EXCL)` and `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` all need
/// FFI. `fs::hard_link` fails atomically when the target exists and would work on NTFS and
/// APFS but not on exFAT — a USB stick would break it, so it is not the primitive to depend
/// on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationVerdict {
    /// Nothing is there. `create_new(true)` will claim it and hold the name.
    Claimable,
    /// Something is there. Requires an explicit replace, whatever its length.
    Occupied {
        /// Whether it is zero length — reportable, but **not** a licence to adopt it.
        zero_length: bool,
    },
}

/// Decides whether a path may be claimed. Pure; the caller does the `create_new`.
#[must_use]
pub fn classify_destination(exists: bool, zero_length: bool) -> DestinationVerdict {
    if exists {
        // No zero-length exception. That exception was S1's defect.
        DestinationVerdict::Occupied { zero_length }
    } else {
        DestinationVerdict::Claimable
    }
}
