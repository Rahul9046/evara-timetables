//! Standing a staged payload up as a project: V0, then V1, then V2, first failure stopping
//! everything.
//!
//! # The order is the decision
//!
//! Three phases, and which comes first is load-bearing rather than tidy:
//!
//! - **V0 — audit, before anything writes.** The payload is opened file-level read-only with
//!   every capability turned down ([`super::boundary`]), `integrity_check` and
//!   `foreign_key_check` are run, and its schema is compared against a reference **this
//!   binary generates** from its own migrations ([`super::conformance`]). Only then is
//!   `app_meta` read. Reading the identity first would mean trusting a row out of the file to
//!   decide whether to trust the file — which is the ordering revision 1 of the design got
//!   wrong, and ADR 0056 is the record of it.
//! - **V1 — migrate.** A writable connection on the staged copy, `migrations::run` in its
//!   grouped transaction, then `change_log::install_all`. Reached only once V0 passed.
//! - **V2 — verify what migration produced.** `foreign_key_check` empty, conformance at the
//!   *current* version, and the census the caller compares a round trip with.
//!
//! # What is deliberately not done here
//!
//! **No safety backup of the staged file.** `backup::before_schema_rebuild` fires on any open
//! below V12 and would litter staging with a `.pre-vN.evaraproj`. On this path it is
//! redundant: the archive is an untouched, checksummed copy of the source, which is a better
//! backup than a `VACUUM` of a throwaway. ADR 0057, and it is the one respect in which V1 is
//! not `Database::open_existing`.
//!
//! **Nothing derived is rebuilt.** Re-scoring a transferred timetable needs a compiled
//! `Problem`, which can legitimately fail for a school mid-setup — an ordinary state that
//! must still transfer — and it would write a `REVALIDATION` row, which is not the immutable
//! `GENERATION` row a transferred evaluation carries. Scoring is an action the interface
//! offers afterwards; a failure to score is a finding about the project, not a failed import.
//!
//! **Nothing is renamed.** This leaves a verified project sitting in staging. Putting it at a
//! destination is the replacement protocol's job, and keeping the two apart is what lets the
//! protocol refuse after verification without having written anything a user can see.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use evara_core::scheduling::Cancel;
use uuid::Uuid;

use super::boundary::{PayloadAudit, UntrustedPayload};
use super::facts::ProjectFacts;
use crate::error::{DbError, Result};
use crate::{change_log, connection, meta, migrations, supported_schema_version};

/// Who is doing the staging, for the one write V1 may make.
///
/// Two values rather than two more parameters, because they travel together everywhere else
/// in this crate and because a call site that passed them the wrong way round would compile.
#[derive(Debug, Clone, Copy)]
pub struct Stager<'a> {
    /// This build's version string, recorded only if a migration ran.
    pub app_version: &'a str,
    /// This installation's identity, which the change-log triggers read during migration.
    ///
    /// Never written to `app_meta.created_by_site_id`: [`meta::ensure`] sets that only when
    /// a project has no identity at all, so an imported project keeps the provenance of the
    /// installation that created it. T-03.
    pub site_id: Uuid,
}

/// A staged payload that has passed every phase.
///
/// Holds no connection: the file is closed before this is returned, so it is a single file
/// the caller may rename. That is [`super::sidecars`]'s precondition and it is asserted
/// rather than assumed — `a_staged_payload_is_a_single_file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPayload {
    /// Where the verified copy is. Inside the caller's staging directory, always.
    pub path: PathBuf,
    /// The schema version the payload arrived at.
    pub schema_version_found: u32,
    /// The schema version it now stands at — this build's supported version.
    pub schema_version_now: u32,
    /// Whether a migration actually ran.
    pub migrated: bool,
    /// What V0's audit reported, carried so a caller can report it rather than re-run it.
    pub audit: PayloadAudit,
    /// Every fact the project holds, read after migration.
    pub facts: ProjectFacts,
}

impl StagedPayload {
    /// Row counts per table, as V2 read them.
    #[must_use]
    pub const fn census(&self) -> &BTreeMap<String, i64> {
        &self.facts.census
    }
}

/// Verifies a staged payload and migrates it forward.
///
/// `payload` is a file **Evara wrote**, inside a staging directory **Evara created** — no
/// byte of archive content contributes to either, which is D6 and the reason the path form
/// is an assertion here rather than a negotiation. `scratch` is where the reference schema is
/// generated; passing the staging directory keeps it on one volume and keeps nothing of a
/// user's project in a shared temp folder.
///
/// # Cancellation
///
/// Asked between phases. Not *within* one: `integrity_check` is a single statement with no
/// interrupt point this build can reach — `progress_handler` needs a `Send + 'static`
/// closure, which a borrowed `&dyn Cancel` is not (E-32) — so the bound on that phase is the
/// payload's declared byte length, which the container caps.
///
/// # Errors
///
/// - [`DbError::DestinationPathUnsafe`] if the path is not a plain file path (F-2).
/// - [`DbError::PayloadCorrupt`] if `integrity_check` is not `ok`, or the file is not shaped
///   like a project at all.
/// - [`DbError::ReferentialDamage`] if `foreign_key_check` reports violations — before
///   migration, where it is the payload's own damage, and after, where a migration would
///   have had to introduce it.
/// - [`DbError::SchemaDoesNotConform`] naming each difference. Raised **before** the identity
///   is read and before anything writes.
/// - [`DbError::SchemaTooNew`] if the payload stands above this build's supported version.
/// - [`DbError::UnknownSchemaVersion`] if the claimed version is one this build cannot
///   generate a reference for — which is how an impossible version is refused.
/// - [`DbError::SnapshotCancelled`] if the caller stopped it between phases.
/// - [`DbError::Migration`] if the migration failed. Nothing was applied, and the staged copy
///   is the caller's to discard.
pub fn stage(
    payload: &Path,
    scratch: &Path,
    stager: Stager<'_>,
    cancel: &dyn Cancel,
) -> Result<StagedPayload> {
    super::paths::require_plain_database_path(payload)?;
    super::paths::require_plain_database_path(scratch)?;

    let audit = audit_v0(payload, scratch, cancel)?;
    let schema_version_found = audit.schema_version;

    if cancel.is_cancelled() {
        return Err(DbError::SnapshotCancelled);
    }

    let migrated = migrate_v1(payload, schema_version_found, stager)?;

    if cancel.is_cancelled() {
        return Err(DbError::SnapshotCancelled);
    }

    let facts = verify_v2(payload, scratch)?;

    Ok(StagedPayload {
        path: payload.to_path_buf(),
        schema_version_found,
        schema_version_now: facts.schema_version,
        migrated,
        audit,
        facts,
    })
}

/// Phase V0. Read-only, capabilities down, and in this exact order.
fn audit_v0(payload: &Path, scratch: &Path, cancel: &dyn Cancel) -> Result<PayloadAudit> {
    let untrusted = UntrustedPayload::open(payload)?;
    let audit = untrusted.audit()?;

    // 1. Corruption. A damaged b-tree is not a schema question and must not be reasoned
    //    about as one.
    if audit.integrity != "ok" {
        return Err(DbError::PayloadCorrupt {
            detail: audit.integrity.clone(),
        });
    }

    // 2. Is it even shaped like a project. Checked from `sqlite_master` rather than by
    //    reading `app_meta`, which is step 4's job and is not reached until conformance has
    //    passed.
    if !audit.looks_like_a_project {
        return Err(DbError::NotAnEvaraProject {
            name: untrusted.name().to_owned(),
        });
    }

    // 3. Referential damage, which `integrity_check` calls `ok` (T-80). Reported before the
    //    migration rather than after, so a payload that arrived broken is distinguished from
    //    one a migration broke.
    if audit.foreign_key_violations > 0 {
        return Err(DbError::ReferentialDamage {
            count: audit.foreign_key_violations,
        });
    }

    if cancel.is_cancelled() {
        return Err(DbError::SnapshotCancelled);
    }

    // 4. Conformance, against a reference generated for the version the payload *claims*.
    //    This is the step that catches the hostile trigger, the extra table, the view and
    //    the virtual table — none of which `integrity_check` notices.
    super::conformance::check(&untrusted, scratch, audit.schema_version)?;

    // 5. Only now may a row out of the file be trusted. `reject_if_newer_than_supported`
    //    reads `app_meta`, so it is deliberately last.
    migrations::reject_if_newer_than_supported(untrusted.connection())?;

    Ok(audit)
}

/// Phase V1. The only phase that writes, and it writes only inside staging.
///
/// Returns whether a migration ran. A payload already at the supported version is **still**
/// opened and has `change_log::install_all` applied, because a project exported by a build
/// that had lost a trigger would otherwise arrive missing it, and the change feed would then
/// have a hole nothing later notices.
///
/// `app_meta` is stamped **only when a migration ran**. An import is not an open — D9's
/// rule, and the reason `project_import` does not open what it imported — so recording
/// `last_opened_with_app_version` on a payload this build did not have to change would make
/// a transfer modify a project for no reason. When the schema did move, the stamp is the
/// truth and `app_meta.schema_version` has to follow it.
fn migrate_v1(payload: &Path, found: u32, stager: Stager<'_>) -> Result<bool> {
    let mut conn = connection::open_configured(payload, stager.site_id)?;

    // No `backup::before_schema_rebuild`. It fires on any open below V12 and would litter
    // staging with a `.pre-vN.evaraproj`; on this path it is redundant, because the archive
    // is an untouched checksummed copy of the source, which is a better backup than a
    // `VACUUM` of a throwaway. ADR 0057, and it is the one respect in which this is not
    // `Database::open_existing`.
    let now = migrations::run(&mut conn)?;

    let tx = conn.transaction()?;
    change_log::install_all(&tx)?;
    tx.commit()?;

    let migrated = found < now;
    if migrated {
        meta::ensure(&mut conn, stager.app_version, now, stager.site_id)?;
    }

    // Closed so the staged file is a single file again before V2 opens it read-only and
    // before the caller renames it. Done here rather than left to a `Drop`, because the
    // whole phase being one function is what makes the write set reviewable.
    conn.close()
        .map_err(|(_, source)| DbError::Internal(crate::error::InternalError::wrap(source)))?;
    Ok(migrated)
}

/// Phase V2. What migration produced, verified against this build's own reference.
fn verify_v2(payload: &Path, scratch: &Path) -> Result<ProjectFacts> {
    let supported = supported_schema_version();

    {
        let untrusted = UntrustedPayload::open(payload)?;
        let audit = untrusted.audit()?;

        if audit.integrity != "ok" {
            return Err(DbError::PayloadCorrupt {
                detail: audit.integrity,
            });
        }
        // `foreign_key_check` again, because a migration that rebuilds a table can break a
        // reference the pre-migration check could not have seen. ADR 0056's "verify after
        // migrating" half, and E-06 is why it is this pragma rather than `integrity_check`.
        if audit.foreign_key_violations > 0 {
            return Err(DbError::ReferentialDamage {
                count: audit.foreign_key_violations,
            });
        }
        if audit.schema_version != supported {
            return Err(DbError::PayloadCorrupt {
                detail: format!(
                    "migrating left it at schema version {} rather than {supported}",
                    audit.schema_version
                ),
            });
        }
        super::conformance::check(&untrusted, scratch, supported)?;
    }

    // Read through the ordinary fact reader, on its own connection, after the hardened one
    // is closed — so the facts a caller reports are read exactly as an export reads them and
    // there is one implementation of that question.
    super::facts::facts_of(payload)
}
