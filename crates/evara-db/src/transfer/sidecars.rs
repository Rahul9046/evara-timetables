//! Reducing a WAL database to a single file, so there is no set to move.
//!
//! # The failure this exists to make unrepresentable — safeguard S3
//!
//! A project is up to four files: `X.evaraproj`, `X.evaraproj-wal`, `X.evaraproj-shm`, and
//! `X.evaraproj-journal` if an older build left one. **There is no atomic multi-file move on
//! either platform.** So a replacement that moved "the whole set, together" cannot deliver
//! what that phrase promises: a crash between moving the main file and moving `-wal` leaves
//! the old `-wal` sitting at the destination's name, and the new main file is then renamed in
//! beside it.
//!
//! A new main file paired with a foreign `-wal` is **a database that opens cleanly and is
//! wrong**. SQLite will replay the stale log over the new file. That is the one corruption
//! shape with no symptom, which is why the design does not try to make the multi-file move
//! reliable — it removes the multi-file move instead:
//!
//! 1. Open the database, `PRAGMA wal_checkpoint(TRUNCATE)`, close it. A clean close removes
//!    `-wal` and `-shm`.
//! 2. **Verify they are gone.** If one cannot be removed, refuse with
//!    [`DbError::RenameBlocked`] — *before anything has moved*.
//! 3. Each side is now one file, so each move is one rename.
//! 4. Before renaming the new main file into place, assert no sidecar exists at the
//!    destination's name. Refuse rather than rename beside it.
//!
//! Step 4 is the safeguard proper: it makes the bad pairing unrepresentable rather than
//! merely unlikely, and it holds even if a future change reintroduces a multi-file move.
//!
//! # Nothing here ever deletes a `-wal`
//!
//! Checkpointing *moves* a log's contents into the main file and SQLite then removes the
//! log. Deleting a `-wal` outright is a different act: it discards committed transactions
//! the main file does not yet contain. This module never does it, and neither does recovery
//! — see [`super::replacement`].

use std::path::{Path, PathBuf};

use crate::error::{DbError, InternalError, Result};
use crate::{connection, file_name_of};

/// The sidecar suffixes a project file can have beside it.
///
/// `-journal` is included because a database written by a build before WAL, or one whose
/// journal mode was changed, can leave one. It is never produced by Evara today and is
/// checked for anyway: the point of the assertion is to catch a file we did *not* write.
pub const SIDECAR_SUFFIXES: &[&str] = &["-wal", "-shm", "-journal"];

/// Which sidecars exist beside a database, read one at a time.
///
/// Read **per file** rather than as a set, because each one moves on its own and an
/// interrupted replacement can therefore leave any subset of them behind. A single
/// `has_sidecars: bool` would collapse exactly the distinction recovery needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SidecarState {
    /// `X-wal` exists — a write-ahead log, possibly holding committed data.
    pub wal: bool,
    /// `X-shm` exists — shared memory for WAL coordination, holding no data of its own.
    pub shm: bool,
    /// `X-journal` exists — a rollback journal from a non-WAL build.
    pub journal: bool,
}

impl SidecarState {
    /// Reads which sidecars are beside `database`.
    ///
    /// Uses `symlink_metadata`, so a symlink named `X-wal` counts as present rather than
    /// being followed to wherever it points.
    #[must_use]
    pub fn observe(database: &Path) -> Self {
        Self {
            wal: sidecar_path(database, "-wal").symlink_metadata().is_ok(),
            shm: sidecar_path(database, "-shm").symlink_metadata().is_ok(),
            journal: sidecar_path(database, "-journal")
                .symlink_metadata()
                .is_ok(),
        }
    }

    /// Whether any sidecar is present.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.wal || self.shm || self.journal
    }

    /// The first sidecar present, by the order a reader would care about.
    ///
    /// `-wal` first because it is the one that can carry data; a message naming it is more
    /// actionable than one naming `-shm`.
    #[must_use]
    pub const fn first_present(&self) -> Option<&'static str> {
        if self.wal {
            Some("-wal")
        } else if self.journal {
            Some("-journal")
        } else if self.shm {
            Some("-shm")
        } else {
            None
        }
    }
}

/// Path of a sidecar beside `database`.
///
/// Built by appending to the **whole file name**, not by replacing an extension: SQLite's
/// sidecars are `school.evaraproj-wal`, not `school-wal.evaraproj`, and a `set_extension`
/// here would silently produce a path nothing uses.
#[must_use]
pub fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Checkpoints `database` and closes it, so it is one file.
///
/// Opens the database, truncates its write-ahead log into the main file, closes it, and then
/// **verifies** no sidecar remains. Refuses rather than reporting success if one does.
///
/// # Errors
///
/// - [`DbError::RenameBlocked`] if a sidecar survives the close — most often because another
///   process still holds the database open. The remedy is in the message, and **nothing has
///   moved**, which is the whole reason this is checked here rather than mid-protocol.
/// - [`DbError::Open`] or [`DbError::Internal`] if the database cannot be opened or
///   checkpointed at all.
pub fn checkpoint_to_single_file(database: &Path) -> Result<()> {
    if !database.exists() {
        // Not an error. A destination that is absent is already "one file or fewer", and
        // the caller's next step is a rename that will create it.
        return Ok(());
    }

    {
        // `open_plain` registers no site-id function, so this connection could not write a
        // tracked row even if something asked it to. A checkpoint is not a tracked write.
        let conn = connection::open_plain(database)?;

        // TRUNCATE rather than PASSIVE or FULL: PASSIVE may checkpoint nothing if a reader
        // is active, and FULL leaves the `-wal` file in place at zero length. Only TRUNCATE
        // asks for the log to end up empty, which is the precondition for SQLite removing
        // it on close.
        let _: (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;

        conn.close()
            .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    }

    // The verification S3 turns on. A clean close *should* have removed both; this asserts
    // it rather than assuming, because the whole protocol downstream is built on each side
    // being a single file.
    let remaining = SidecarState::observe(database);
    if let Some(sidecar) = remaining.first_present() {
        return Err(DbError::RenameBlocked {
            name: file_name_of(database),
            sidecar,
        });
    }

    Ok(())
}

/// Refuses if any sidecar exists at `destination`.
///
/// Called **immediately before** the rename that puts a new main file at that name. This is
/// S3 step 4, and it is what makes a main-file-plus-foreign-sidecar pairing unrepresentable
/// instead of unlikely: whatever sequence of interruptions produced the stray file, the
/// rename does not happen on top of it.
///
/// # Errors
///
/// [`DbError::RenameBlocked`], naming which sidecar is in the way.
pub fn refuse_if_sidecars_present(destination: &Path) -> Result<()> {
    let found = SidecarState::observe(destination);
    if let Some(sidecar) = found.first_present() {
        return Err(DbError::RenameBlocked {
            name: file_name_of(destination),
            sidecar,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{
        SIDECAR_SUFFIXES, SidecarState, checkpoint_to_single_file, refuse_if_sidecars_present,
        sidecar_path,
    };
    use crate::error::DbError;

    #[test]
    fn a_sidecar_path_appends_to_the_whole_name() {
        let path = std::path::Path::new("/schools/north.evaraproj");
        assert_eq!(
            sidecar_path(path, "-wal"),
            std::path::PathBuf::from("/schools/north.evaraproj-wal"),
            "a sidecar sits beside the file with its full name, not with the stem"
        );
    }

    #[test]
    fn every_suffix_is_observed_independently() {
        let dir = TempDir::new().expect("temp dir");
        let database = dir.path().join("school.evaraproj");
        std::fs::write(&database, b"").expect("create");

        assert_eq!(SidecarState::observe(&database), SidecarState::default());
        assert!(!SidecarState::observe(&database).any());

        // One at a time, because the point of the struct is that a subset is representable.
        for suffix in SIDECAR_SUFFIXES {
            std::fs::write(sidecar_path(&database, suffix), b"x").expect("create sidecar");
            let state = SidecarState::observe(&database);
            assert!(state.any(), "{suffix} must be observed");
            std::fs::remove_file(sidecar_path(&database, suffix)).expect("remove");
            assert!(
                !SidecarState::observe(&database).any(),
                "and must stop being observed once removed"
            );
        }
    }

    #[test]
    fn first_present_prefers_the_sidecar_that_can_hold_data() {
        let all = SidecarState {
            wal: true,
            shm: true,
            journal: true,
        };
        assert_eq!(
            all.first_present(),
            Some("-wal"),
            "-wal is the one whose loss would discard committed work, so it is named first"
        );
        let shm_only = SidecarState {
            shm: true,
            ..SidecarState::default()
        };
        assert_eq!(shm_only.first_present(), Some("-shm"));
    }

    #[test]
    fn refuse_if_sidecars_present_names_the_file_in_the_way() {
        let dir = TempDir::new().expect("temp dir");
        let destination = dir.path().join("school.evaraproj");

        refuse_if_sidecars_present(&destination).expect("a clean destination is fine");

        std::fs::write(sidecar_path(&destination, "-wal"), b"stale").expect("create");
        let error = refuse_if_sidecars_present(&destination)
            .expect_err("a stray -wal must refuse the rename");

        match error {
            DbError::RenameBlocked { sidecar, name } => {
                assert_eq!(sidecar, "-wal");
                assert_eq!(name, "school.evaraproj");
            }
            other => panic!("expected RenameBlocked, got {other:?}"),
        }
    }

    #[test]
    fn an_absent_database_needs_no_checkpoint() {
        let dir = TempDir::new().expect("temp dir");
        checkpoint_to_single_file(&dir.path().join("nothing.evaraproj"))
            .expect("an absent destination is already one file or fewer");
    }
}
