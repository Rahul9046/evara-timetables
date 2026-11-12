//! The directory an import works in, and the journal that makes an interruption recoverable.
//!
//! # Staging is a sibling of the destination
//!
//! `…/Documents/.evara-import-<uuid7>/` beside `…/Documents/Northgate.evaraproj`. Every later
//! rename is therefore intra-directory on one volume, which is the only arrangement the
//! atomicity claim in [`crate::paths`] covers — a cross-volume rename is a copy, and it fails
//! at the worst possible moment.
//!
//! # The journal records intent; the filesystem is the truth
//!
//! Safeguard **S2**. The journal row is committed **before** the rename it authorises, which
//! is what makes recovery possible at all — and also means a crash in that window leaves the
//! phase *ahead* of the disk. So recovery never maps a phase to an action: it observes the
//! disk and `evara_db::transfer::reconcile` decides, with the journal as one input among
//! five. [`crate::recovery`] is that half.
//!
//! The journal lives in the **settings** database, not in a project: it is the only store
//! reachable at startup without knowing which project to look at, and the project that needs
//! recovering may be the one that will not open.
//!
//! # Staging removes itself unless it is adopted
//!
//! [`Staging`]'s `Drop` deletes the directory and clears the journal row. A cancelled import,
//! a failed verification or an early `?` therefore leaves nothing — the same structural
//! arrangement `Snapshot` and the `.part` file have, and for the same reason: remembering
//! cleanup at each `?` is how litter gets left behind.
//!
//! **Abandoned staging with no journal row** is the one thing never auto-deleted: it may be
//! another installation's work in progress, and deleting that is worse than leaving it. It is
//! reported.

use std::path::{Path, PathBuf};

use evara_db::transfer::ReplacementPhase;
use uuid::Uuid;

use crate::error::{Result, TransferError};

/// The file name a staged project is given inside staging.
///
/// Fixed and Evara's own, which is the point: no byte of archive content contributes to it.
pub const STAGED_FILE: &str = "staged.evaraproj";

/// The directory the old project is moved into during a replacement.
pub const ASIDE_DIR: &str = "aside";

/// The prefix a staging directory's name carries.
///
/// Forty-five characters with the UUID, which is why Windows' `MAX_PATH` is named as a risk
/// in §7.4 rather than ignored.
pub const STAGING_PREFIX: &str = ".evara-import-";

/// Somewhere to record an in-flight replacement.
///
/// A trait rather than `&mut evara_db::Settings` directly, for two reasons. A test can drive
/// every recovery state from a journal it controls, including the ambiguous ones a real crash
/// reaches rarely. And the protocol's dependency is on *the record*, not on the settings
/// database — which is what lets the ownership of that database stay with the `Workspace`
/// that already holds it.
pub trait ReplacementJournal {
    /// Opens a row at [`ReplacementPhase::Staged`], before anything outside staging is
    /// touched.
    ///
    /// # Errors
    ///
    /// A duplicate destination is a refusal: two concurrent replacements of one file is not a
    /// state the protocol has an answer for, so the uniqueness index turns a race into a
    /// refusal.
    fn begin(&mut self, destination: &Path, staging_dir: &Path) -> Result<Uuid>;

    /// Advances a row, optionally recording the keep-name.
    ///
    /// # Errors
    ///
    /// Propagates the store's failure. A row that has gone means another process finished or
    /// abandoned this replacement, and the caller must not continue acting on it.
    fn advance(
        &mut self,
        id: Uuid,
        phase: ReplacementPhase,
        keep_name: Option<&Path>,
    ) -> Result<()>;

    /// Removes a row. Idempotent: clearing one another process already cleared is success.
    ///
    /// # Errors
    ///
    /// Propagates the store's failure.
    fn clear(&mut self, id: Uuid) -> Result<()>;

    /// Every in-flight replacement, oldest first.
    ///
    /// # Errors
    ///
    /// Propagates the store's failure, including a damaged identifier — which is corruption
    /// and never a substituted nil UUID.
    fn pending(&self) -> Result<Vec<evara_db::transfer::PendingReplacement>>;
}

impl ReplacementJournal for evara_db::Settings {
    fn begin(&mut self, destination: &Path, staging_dir: &Path) -> Result<Uuid> {
        match self.begin_replacement(destination, staging_dir) {
            Ok(row) => Ok(row.id),
            Err(evara_db::DbError::DuplicateValue { .. }) => {
                Err(TransferError::ReplacementInFlight {
                    name: crate::paths::file_name_of(destination),
                })
            }
            Err(other) => Err(TransferError::Database(other)),
        }
    }

    fn advance(
        &mut self,
        id: Uuid,
        phase: ReplacementPhase,
        keep_name: Option<&Path>,
    ) -> Result<()> {
        self.advance_replacement(id, phase, keep_name)
            .map_err(TransferError::Database)
    }

    fn clear(&mut self, id: Uuid) -> Result<()> {
        self.clear_replacement(id)
            .map(|_| ())
            .map_err(TransferError::Database)
    }

    fn pending(&self) -> Result<Vec<evara_db::transfer::PendingReplacement>> {
        self.pending_replacements().map_err(TransferError::Database)
    }
}

/// A staging directory, and the journal row that explains it.
///
/// Removes both on drop unless [`Staging::adopt`] was called.
pub struct Staging<'a> {
    dir: PathBuf,
    destination: PathBuf,
    id: Uuid,
    journal: &'a mut dyn ReplacementJournal,
    adopted: bool,
}

impl<'a> Staging<'a> {
    /// Creates a staging directory beside `destination` and journals the intent.
    ///
    /// # Errors
    ///
    /// - [`TransferError::DestinationUnsafe`] if the destination has no parent directory.
    /// - [`TransferError::ReplacementInFlight`] if one is already recorded for it.
    /// - [`TransferError::Io`] if the directory cannot be made.
    pub fn open(destination: &Path, journal: &'a mut dyn ReplacementJournal) -> Result<Self> {
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| TransferError::DestinationUnsafe {
                name: crate::paths::file_name_of(destination),
                reason: "it has no folder to stage beside",
            })?;

        let dir = parent.join(format!("{STAGING_PREFIX}{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).map_err(TransferError::io("prepare a staging folder"))?;

        // Journalled **after** the directory exists and **before** anything outside it is
        // touched. The other order would record a directory that is not there, which is a
        // row recovery has to treat as ambiguous for no reason.
        let id = match journal.begin(destination, &dir) {
            Ok(id) => id,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(error);
            }
        };

        Ok(Self {
            dir,
            destination: destination.to_path_buf(),
            id,
            journal,
            adopted: false,
        })
    }

    /// The staging directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the staged project goes.
    #[must_use]
    pub fn staged_file(&self) -> PathBuf {
        self.dir.join(STAGED_FILE)
    }

    /// The directory the old project is moved into.
    #[must_use]
    pub fn aside_dir(&self) -> PathBuf {
        self.dir.join(ASIDE_DIR)
    }

    /// Where the old project goes inside [`Staging::aside_dir`].
    ///
    /// Keeps the destination's own file name, so the kept copy is recognisable and so
    /// recovery's observation of "is the aside set there" is a single `symlink_metadata` on a
    /// path both halves compute the same way.
    #[must_use]
    pub fn aside_file(&self) -> PathBuf {
        self.aside_dir().join(
            self.destination
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("project.evaraproj")),
        )
    }

    /// The journal row's identity.
    #[must_use]
    pub const fn journal_id(&self) -> Uuid {
        self.id
    }

    /// Advances the journal row.
    ///
    /// # Errors
    ///
    /// Propagates the store's failure.
    pub fn advance(&mut self, phase: ReplacementPhase, keep_name: Option<&Path>) -> Result<()> {
        self.journal.advance(self.id, phase, keep_name)
    }

    /// Finishes: clears the journal row and removes the directory.
    ///
    /// Called once the destination holds the new project and the old one has been kept. The
    /// order is deliberate — **the directory goes first, then the row** — because a row with
    /// no directory is an ambiguous state recovery has to report, while a directory with no
    /// row is reported and left alone, which is the safer of the two to be interrupted in.
    ///
    /// # Errors
    ///
    /// Propagates the store's failure. A directory that cannot be removed is **not** an
    /// error: the import succeeded, and failing it afterwards for a leftover folder would
    /// report a success as a failure.
    pub fn adopt(mut self) -> Result<()> {
        self.adopted = true;
        let _ = std::fs::remove_dir_all(&self.dir);
        self.journal.clear(self.id)
    }
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        if !self.adopted {
            // Both ignored deliberately: this runs on the failure path, where replacing a
            // useful message with a cleanup error would be the wrong trade. What the user is
            // told is why the import was refused.
            let _ = std::fs::remove_dir_all(&self.dir);
            let _ = self.journal.clear(self.id);
        }
    }
}

/// The name the replaced project is kept under.
///
/// `<stem>.replaced-<ISO stamp>.evaraproj`. **Nothing in Evara ever deletes it** — the rule
/// `backup.rs` established for a schema-rebuild safety copy, applied here, which is why "a
/// failed import must not destroy the previous project" is not a promise about error handling:
/// there is no code path that deletes it at all.
///
/// The stamp is filesystem-safe: ISO-8601's colons are not legal in a Windows file name, so
/// they become hyphens and the milliseconds are dropped. Two replacements inside one second
/// would collide, and the caller resolves that by looking — [`unused_keep_name`].
#[must_use]
pub fn keep_name_for(destination: &Path, at: &str) -> PathBuf {
    let stem = destination.file_stem().map_or_else(
        || String::from("project"),
        |s| s.to_string_lossy().into_owned(),
    );
    let stamp = at.split('.').next().unwrap_or(at).replace(':', "-");
    destination.with_file_name(format!(
        "{stem}.replaced-{stamp}Z.{}",
        evara_db::PROJECT_EXTENSION
    ))
}

/// A keep-name nothing is using.
///
/// Two replacements within one second would otherwise collide, and a collision here would
/// mean a rename **over a kept copy** — destroying the one thing this protocol promises never
/// to destroy. So the name is probed and suffixed rather than assumed unique.
///
/// # Errors
///
/// [`TransferError::Io`] if sixteen names are all taken, which means something is wrong that
/// a seventeenth would not fix.
pub fn unused_keep_name(destination: &Path, at: &str) -> Result<PathBuf> {
    let base = keep_name_for(destination, at);
    if !base.exists() {
        return Ok(base);
    }
    let stem = base.file_stem().map_or_else(
        || String::from("project"),
        |s| s.to_string_lossy().into_owned(),
    );
    for n in 2..=16_u32 {
        let candidate = base.with_file_name(format!("{stem}-{n}.{}", evara_db::PROJECT_EXTENSION));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(TransferError::Io {
        action: "find an unused name for the copy being kept",
        source: std::io::Error::other("sixteen kept copies already exist for this second"),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{keep_name_for, unused_keep_name};

    #[test]
    fn a_keep_name_has_no_colons_in_it() {
        let name = keep_name_for(
            Path::new("/schools/Northgate.evaraproj"),
            "2026-11-12T09:14:22.113Z",
        );
        let text = name.to_string_lossy();
        assert!(
            !text.contains(':'),
            "a colon is not legal in a Windows file name: {text}"
        );
        assert!(text.ends_with(".evaraproj"), "{text}");
        assert!(
            text.contains("Northgate.replaced-2026-11-12T09-14-22Z"),
            "{text}"
        );
    }

    #[test]
    fn a_taken_keep_name_is_stepped_past_rather_than_written_over() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let destination = dir.path().join("Northgate.evaraproj");
        let first = unused_keep_name(&destination, "2026-11-12T09:14:22.113Z").expect("first");
        std::fs::write(&first, b"the copy being kept").expect("write");

        let second = unused_keep_name(&destination, "2026-11-12T09:14:22.113Z").expect("second");
        assert_ne!(
            first, second,
            "a second replacement in the same second must not rename over the kept copy"
        );
        assert_eq!(
            std::fs::read(&first).expect("still there"),
            b"the copy being kept",
            "and the first kept copy is untouched"
        );
    }
}
