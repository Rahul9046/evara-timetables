//! Completing or rolling back an interrupted replacement, from what is on disk.
//!
//! # Safeguard S2: the journal is intent, the filesystem is truth
//!
//! The row is committed **before** the rename it authorises. That is what makes recovery
//! possible — and it means a crash in that window leaves the phase *ahead* of the disk.
//!
//! The concrete data-loss case, which the first draft of the design got wrong: the journal
//! says `PLACED`, but the crash landed between that commit and the rename. On disk the
//! destination is **absent**, the staged file is still in staging, and the old project is in
//! `staging/aside/`. Read literally, `PLACED` means *"finish step 3 — move the aside set to
//! the keep-name"*, which moves the user's only project to a `.replaced-` name and leaves
//! **no project at the destination at all**.
//!
//! So nothing here maps a phase to an action. `DiskFacts::observe` reads five facts,
//! `reconcile` decides from them, and this module only *performs* what it decided. The
//! decision function is pure and lives in `evara-db` beside the journal, which is what lets
//! all 128 reachable combinations be driven in a unit test rather than only the twelve rows
//! somebody wrote down.
//!
//! One invariant dominates the table, and it is a property of the **action** rather than a
//! rule this code remembers: *the old file set is released only once a destination that looks
//! like a project has been observed.* `RecoveryAction::releases_the_old_set` is that
//! property, and [`apply`] below is where it is spent.
//!
//! # Nothing is deleted to make a state tidy
//!
//! - **A `-wal` is never removed.** Checkpointing *moves* a log's contents into the main file
//!   and SQLite then removes the log; deleting one outright discards committed transactions
//!   the main file does not yet hold. That is the data-loss move dressed up as cleanup.
//! - **An unknown file at the destination is kept**, not overwritten. It is set aside under
//!   its own `.replaced-` name first, exactly as a project would be.
//! - **Abandoned staging with no journal row is reported and never deleted.** It may be
//!   another installation's work in progress, and deleting that is worse than leaving it.
//! - **An ambiguous state is reported and acted on by nothing.** Guessing is how the `PLACED`
//!   case above loses a project.

use std::path::Path;

use evara_db::transfer::{
    Ambiguity, DiskFacts, PendingReplacement, RecoveryAction, looks_like_a_project,
};

use crate::error::{Recovered, Result, TransferError};
use crate::staging::{ReplacementJournal, unused_keep_name};

/// Completes or rolls back every interrupted replacement.
///
/// Run at startup, before any project is open — which is the point, because the project that
/// needs recovering may be the one that will not open.
///
/// Returns one [`Recovered`] per journal row, in the order the attempts happened. A row this
/// build will not act on is reported with `resolved: false` and its row **left in place**, so
/// it is still there to be looked at by a person or by a newer build.
///
/// # Errors
///
/// [`TransferError::Database`] if the journal cannot be read — including a damaged identifier,
/// which is corruption and never a substituted nil UUID. The whole listing fails, which is
/// the right answer: a settings database somebody has edited by hand is not a file recovery
/// should act on.
pub fn recover_pending(journal: &mut dyn ReplacementJournal) -> Result<Vec<Recovered>> {
    let rows = journal.pending()?;
    let mut recovered = Vec::with_capacity(rows.len());

    for row in rows {
        let facts = DiskFacts::observe(&row, &looks_like_a_project);
        let action = evara_db::transfer::reconcile(facts, row.phase);
        recovered.push(apply(journal, &row, action)?);
    }
    Ok(recovered)
}

/// Performs one decided action.
///
/// Every branch either rolls backwards, completes, or changes nothing. There is no branch
/// that deletes a project, an unknown file or a sidecar.
fn apply(
    journal: &mut dyn ReplacementJournal,
    row: &PendingReplacement,
    action: RecoveryAction,
) -> Result<Recovered> {
    // Asserted here rather than trusted: the invariant S2 turns on is a property of the
    // action, and this is the one place an action becomes a filesystem operation. A future
    // branch that released the old set without having observed a project would trip this
    // instead of losing somebody's school.
    debug_assert!(
        !action.releases_the_old_set() || facts_showed_a_project(row),
        "the old file set may be released only once a destination that looks like a \
         project has been observed"
    );

    let outcome = match action {
        RecoveryAction::RollBackDeletingStaging { remove_claim } => {
            if remove_claim {
                // Only ever a zero-length file with staging alongside, which together mean
                // it was **our** Create-mode claim. A file with bytes in it is somebody's
                // and is never removed — `reconcile` is what decides that, and this branch
                // is only reachable when it did.
                let _ = std::fs::remove_file(&row.destination);
            }
            remove_staging(row);
            String::from("the interrupted import was rolled back; nothing was changed")
        }
        RecoveryAction::RestoreAside {
            then_delete_staging,
        } => {
            crate::paths::rename_with_retry(&row.aside_file(), &row.destination)?;
            if then_delete_staging {
                remove_staging(row);
            }
            String::from("the previous campus file was put back; nothing was changed")
        }
        RecoveryAction::DisplaceUnknownThenRestoreAside => {
            // The unknown file is **kept**, under its own `.replaced-` name. The same rule
            // that governs the project being replaced: Evara does not delete a file it did
            // not make, however little sense it makes at this path.
            let displaced = unused_keep_name(&row.destination, &evara_db::now_iso8601())?;
            crate::paths::rename_with_retry(&row.destination, &displaced)?;
            crate::paths::rename_with_retry(&row.aside_file(), &row.destination)?;
            remove_staging(row);
            format!(
                "the previous campus file was put back; the file that was in its place was \
                 kept as {}",
                crate::paths::file_name_of(&displaced)
            )
        }
        RecoveryAction::FinishMovingAsideToKeepName => {
            let keep_name = match row.keep_name.clone() {
                Some(name) if !name.exists() => name,
                // A journal row with no keep-name, or one already taken, is answered by
                // choosing a fresh name rather than by renaming over a kept copy — which
                // would destroy the one thing this protocol promises never to destroy.
                _ => unused_keep_name(&row.destination, &evara_db::now_iso8601())?,
            };
            crate::paths::rename_with_retry(&row.aside_file(), &keep_name)?;
            remove_staging(row);
            format!(
                "the import finished; your previous campus file is kept as {}",
                crate::paths::file_name_of(&keep_name)
            )
        }
        RecoveryAction::FinishDeletingStaging => {
            remove_staging(row);
            String::from("the import had already finished")
        }
        RecoveryAction::AlreadyFinished => String::from("the import had already finished"),
        RecoveryAction::Report(ambiguity) => {
            // Nothing is touched, and the row is left so the state is still inspectable.
            return Ok(Recovered {
                destination: row.destination.clone(),
                outcome: describe(ambiguity).to_owned(),
                resolved: false,
            });
        }
    };

    journal.clear(row.id)?;
    Ok(Recovered {
        destination: row.destination.clone(),
        outcome,
        resolved: true,
    })
}

/// Removes a staging directory recovery has finished with.
///
/// Ignored on failure, and deliberately: the destination is already correct by the time this
/// runs, and reporting a leftover folder as a failed recovery would tell a user their project
/// is at risk when it is not. A folder that could not be removed is reported by the *next*
/// startup as abandoned staging, which is where a person can act on it.
fn remove_staging(row: &PendingReplacement) {
    let _ = std::fs::remove_dir_all(&row.staging_dir);
}

/// Re-reads the destination, for the debug assertion only.
fn facts_showed_a_project(row: &PendingReplacement) -> bool {
    DiskFacts::observe(row, &looks_like_a_project)
        .destination
        .is_blessable_project()
}

/// What an ambiguity means, in words a user can relay.
const fn describe(ambiguity: Ambiguity) -> &'static str {
    match ambiguity {
        Ambiguity::DestinationAndStagedAndAside => {
            "a campus file, a half-finished import and a previous copy are all present, so \
             Evara cannot tell which is current"
        }
        Ambiguity::NothingRemains => {
            "nothing remains of the interrupted import, so there is nothing to put back"
        }
        Ambiguity::UnknownFileAndNothingAside => {
            "something Evara did not write is at that path, and there is no previous copy to \
             put back"
        }
        Ambiguity::UnaccountableSidecar => {
            "a database log file is beside that campus file and Evara cannot account for it; \
             removing it would discard saved work"
        }
        Ambiguity::UnknownPhase => {
            "this interrupted import was recorded by a newer version of Evara, so this one \
             will not act on it"
        }
    }
}

/// Staging directories with no journal row, found beside a project.
///
/// **Reported, never deleted.** One may be another installation's live staging, or the
/// leftovers of an import whose journal row was lost to a power cut. Deleting a directory
/// that might be another process's work in progress is worse than leaving it, and a user told
/// where it is can decide.
///
/// # Errors
///
/// [`TransferError::Io`] if the directory cannot be listed.
pub fn abandoned_staging(
    beside: &Path,
    journal: &dyn ReplacementJournal,
) -> Result<Vec<Recovered>> {
    let known: Vec<_> = journal
        .pending()?
        .into_iter()
        .map(|row| row.staging_dir)
        .collect();

    let mut found = Vec::new();
    let entries = std::fs::read_dir(beside).map_err(TransferError::io("list that folder"))?;
    for entry in entries {
        let entry = entry.map_err(TransferError::io("list that folder"))?;
        let path = entry.path();
        let is_staging = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(crate::staging::STAGING_PREFIX));

        if is_staging && path.is_dir() && !known.contains(&path) {
            found.push(Recovered {
                destination: path.clone(),
                outcome: String::from(
                    "a half-finished import is in this folder and Evara has no record of it; \
                     it has been left alone",
                ),
                resolved: false,
            });
        }
    }
    found.sort_by(|a, b| a.destination.cmp(&b.destination));
    Ok(found)
}
