//! The safety copy taken before a migration rebuilds a table.
//!
//! # Why this exists at all
//!
//! Every migration before `V12` was additive: a new table, a new column, a new index. `V12`
//! is the first that **drops and recreates** one, and it does so to the table every other
//! school-owned row hangs off. refinery's grouped transaction means a failure rolls the
//! whole batch back and the project is untouched — that is tested, in
//! [`crate::tests_multi_school`] — so in theory this module is unnecessary.
//!
//! It exists because the first table rebuild in a product's life is the wrong place to rely
//! only on theory. A power loss mid-COMMIT, a full disk, a cloud-sync client writing the
//! file underneath us, a bug in this very migration: none of those are the transaction's
//! problem to solve, and all of them land on a file a school cannot recreate.
//!
//! # Why `VACUUM INTO` and not a file copy
//!
//! A project runs in WAL mode, so the committed state of the database is split between the
//! `.evaraproj` file and its `-wal` sidecar. Copying the main file alone yields a database
//! missing every transaction still in the log — a backup that opens cleanly and silently
//! lacks the most recent work, which is worse than no backup at all.
//!
//! `VACUUM INTO` runs inside a read transaction and writes a **fully checkpointed, complete
//! copy** of the database to a new file: WAL content included, no sidecars needed, readable
//! on its own.
//!
//! # The engine is *not* the "do not overwrite a backup" guard
//!
//! This header used to claim that SQLite refuses to write the target if it exists, "enforced
//! by the engine rather than by a `try_exists` race". That is true of a target holding a
//! database — *"output file already exists"* — and **false of a zero-length one**, which
//! SQLite adopts and writes into without complaint. `transfer::tests_engine`'s T-78 proves
//! all three outcomes.
//!
//! The distinction matters here because a *refused* `VACUUM INTO` leaves exactly such a
//! zero-length file (T-75). So [`choose_path`]'s existence check is load-bearing rather
//! than belt-and-braces, and [`before_schema_rebuild`] removes its target on **every**
//! failure path — otherwise a failed upgrade leaves a dead 0-byte file that permanently
//! consumes a backup name, and sixteen of those turn the next upgrade into a
//! [`DbError::BackupRefused`] about nothing.
//!
//! The copy is not byte-identical to the original — `VACUUM` defragments as it goes — but it
//! is *logically* identical, which is what a recovery needs. The original file is never
//! written to by any of this.
//!
//! # No network, ever
//!
//! The backup is a sibling file in the project's own directory. There is no provider, no
//! upload and no configuration: ADR 0001 forbids all three, and a backup that depended on a
//! service would be useless in the case it exists for.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::connection;
use crate::error::{DbError, InternalError, Result};

/// The first schema version whose migration rebuilds an existing table.
///
/// A project already at or past this version has been through the rebuild and needs no
/// further protection; a project below it is about to be.
pub(crate) const FIRST_REBUILDING_MIGRATION: u32 = 12;

/// How many suffixed names to try before giving up.
///
/// A deterministic first choice keeps the common case predictable — one upgrade, one
/// obvious file. The suffixes exist so that a second attempt after a failed upgrade does
/// not overwrite the evidence of the first.
const MAX_ATTEMPTS: u32 = 16;

/// Takes a verified safety copy if `applied` is about to cross [`FIRST_REBUILDING_MIGRATION`].
///
/// Returns the path written, or `None` when no backup was needed — a brand-new project
/// (`applied == 0`, nothing to protect) or one already past the rebuild.
///
/// # Errors
///
/// [`DbError::BackupRefused`] if a copy could not be written **or could not be verified**,
/// in which case nothing has been migrated and the project is exactly as it was. Refusing
/// is the point: proceeding without protection would silently drop the one guarantee this
/// module is here to provide.
pub(crate) fn before_schema_rebuild(
    conn: &Connection,
    path: &Path,
    applied: u32,
) -> Result<Option<PathBuf>> {
    if applied == 0 || applied >= FIRST_REBUILDING_MIGRATION {
        return Ok(None);
    }

    let target = choose_path(path, applied)?;

    // `VACUUM INTO` takes a read lock for the duration, so the snapshot is consistent even
    // if something else is mid-write.
    //
    // A refusal still creates the target, zero length (T-75), so the file is taken away on
    // this path too. Without it the dead file survives, `choose_path` steps past the name
    // on every later attempt, and after `MAX_ATTEMPTS` the upgrade is refused for a reason
    // that has nothing to do with the project.
    if let Err(source) = conn.execute("VACUUM INTO ?", [path_arg(&target)?]) {
        remove_quietly(&target);
        return Err(refused(path, InternalError::wrap(source)));
    }

    // A backup nobody has opened is a guess. Verify before letting the migration proceed,
    // and take the bad copy away if it is not sound — leaving a broken file named "backup"
    // is how someone comes to trust one.
    if let Err(error) = verify(&target, conn) {
        remove_quietly(&target);
        return Err(error);
    }

    Ok(Some(target))
}

/// Picks `<name>.pre-vN.evaraproj`, or the first free `-2`, `-3` … variant.
///
/// The backup keeps the project extension because it **is** a project: if the worst
/// happens, the recovery is to open it, not to rename it first. `PROJECT_EXTENSION` is
/// deliberately reused rather than inventing a third extension, which `CLAUDE.md` forbids.
fn choose_path(path: &Path, applied: u32) -> Result<PathBuf> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().map_or_else(
        || String::from("project"),
        |s| s.to_string_lossy().into_owned(),
    );

    for attempt in 1..=MAX_ATTEMPTS {
        let suffix = if attempt == 1 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let candidate = directory.join(format!(
            "{stem}.pre-v{applied}{suffix}.{}",
            crate::workspace::PROJECT_EXTENSION
        ));
        // Checked rather than trusted to `VACUUM INTO`'s own refusal, so that an existing
        // backup produces the next name instead of an error.
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(refused(
        path,
        InternalError::from_message(format!(
            "{MAX_ATTEMPTS} backup names beside the project are already taken"
        )),
    ))
}

/// Opens the copy and satisfies itself that it is a usable project.
///
/// Three questions, because each catches a different kind of bad copy: a structurally
/// damaged file, a file that is not an Evara project, and a file whose school rows did not
/// all arrive.
fn verify(target: &Path, source: &Connection) -> Result<()> {
    let copy = connection::open_plain(target)
        .map_err(|error| refused(target, InternalError::from_message(error.to_string())))?;

    let integrity: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|source| refused(target, InternalError::wrap(source)))?;
    if integrity != "ok" {
        return Err(refused(
            target,
            InternalError::from_message(format!("the copy reports `{integrity}`")),
        ));
    }

    let has_meta: i64 = copy
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_meta'",
            [],
            |row| row.get(0),
        )
        .map_err(|source| refused(target, InternalError::wrap(source)))?;
    if has_meta == 0 {
        return Err(refused(
            target,
            InternalError::from_message("the copy does not look like an Evara project"),
        ));
    }

    let expected: i64 = source.query_row("SELECT COUNT(*) FROM school", [], |row| row.get(0))?;
    let found: i64 = copy
        .query_row("SELECT COUNT(*) FROM school", [], |row| row.get(0))
        .map_err(|source| refused(target, InternalError::wrap(source)))?;
    if found != expected {
        return Err(refused(
            target,
            InternalError::from_message(format!(
                "the copy holds {found} school rows where the project holds {expected}"
            )),
        ));
    }

    Ok(())
}

/// Removes a failed copy, ignoring whether it was there.
///
/// Deliberately silent. This runs on a path that is already returning an error, and the
/// caller's refusal names the real problem — reporting that the cleanup also failed would
/// replace a useful message with a confusing one. A sidecar is never removed here because
/// `VACUUM INTO` writes a single file and creates none.
fn remove_quietly(target: &Path) {
    let _ = std::fs::remove_file(target);
}

/// Binds a path for SQLite, which takes it as text.
fn path_arg(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        refused(
            path,
            InternalError::from_message("the folder name cannot be expressed as text"),
        )
    })
}

/// Builds the refusal, naming the file and never its directory.
fn refused(path: &Path, source: InternalError) -> DbError {
    DbError::BackupRefused {
        name: path.file_name().map_or_else(
            || String::from("the project"),
            |name| name.to_string_lossy().into_owned(),
        ),
        source,
    }
}
