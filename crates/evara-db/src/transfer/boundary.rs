//! Opening a file from somewhere else, with SQLite's own capabilities turned down.
//!
//! # What this is and is not
//!
//! A staged payload is a SQLite database that arrived from outside this installation.
//! Opening it means handing an untrusted file to a C parser, and **no arrangement of
//! pragmas makes that safe in the sense of "cannot be exploited"**. What the controls here
//! do is remove the capabilities an attacker would otherwise simply *ask* for: attaching
//! another database, rewriting `sqlite_master`, running a trigger or a view on read,
//! exhausting memory through a declared-huge structure.
//!
//! §6.2 of the proposal says so explicitly and this module repeats it rather than letting a
//! reader infer a guarantee from a long list of flags. The honest summary: **corruption and
//! hostility are detected; invulnerability is not claimed.**
//!
//! # Why this is not `open_query_only`
//!
//! [`connection::open_query_only`](crate::connection) exists for *our own* project, opened
//! for inspection. It calls `open_plain`, which runs `PRAGMA journal_mode = WAL` — and that
//! statement **can write the file header**. Acceptable on a file Evara created; not
//! acceptable on a file from elsewhere, where the first thing we do must not be a write.
//!
//! So this constructor is separate, and deliberately does not reuse that one. It is the
//! subtlest point in the module: a later refactor that "shares the factory" would
//! reintroduce a header write on an untrusted file.
//!
//! # The path a payload is opened by — finding F-2
//!
//! The record said URI filenames were disabled by leaving `SQLITE_OPEN_URI` out of the open
//! flags. They are not: the bundled engine carries `-DSQLITE_USE_URI`, which makes URI
//! parsing the **global default**, so the flag can only add the capability and never remove
//! it. The real control is that every path a payload is opened by is one Evara chose, and
//! [`super::paths::require_plain_database_path`] asserts the form regardless. See that
//! module for the forms and why each one matters.
//!
//! # The phases
//!
//! - **V0 — audit.** File-level read-only, every capability off, limits clamped. Nothing is
//!   trusted and nothing is written. `integrity_check`, `foreign_key_check`, and the schema
//!   conformance comparison all happen here.
//! - **V1 — migrate.** Only once V0 passed. `trusted_schema` stays **off**, which is what
//!   makes `evara_site_id`'s `SQLITE_INNOCUOUS` flag necessary rather than decorative: the
//!   change-log triggers fire during migration and would otherwise be refused.
//!
//! V0 and the audit are here; V1 and V2 are [`super::stage`], which owns the order the three
//! run in and is the only caller that should compose them.

use std::path::Path;

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};

use crate::error::{DbError, InternalError, Result};
use crate::{file_name_of, migrations};

/// Capabilities turned **off** for a payload under audit, with why each one matters.
///
/// Expressed as a table rather than a sequence of calls so the set is reviewable, and so the
/// test that asserts each one can iterate the same list the constructor applies. A flag
/// added here without a test is therefore visible as a gap.
const AUDIT_DISABLED: &[(DbConfig, &str)] = &[
    // A view or trigger running during an ordinary `SELECT` is the main way a hostile schema
    // gets to execute anything at all.
    (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, "trusted_schema"),
    // Direct `sqlite_master` rewriting. `DEFENSIVE` below also covers it; both are set
    // because they are independent switches and a future engine may decouple them further.
    (DbConfig::SQLITE_DBCONFIG_WRITABLE_SCHEMA, "writable_schema"),
    // No views at all during the audit: the census and the conformance comparison read
    // tables, and a view is only a way for the file to interpose on that.
    (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, "views"),
    // No triggers during the audit. They are needed in V1 and not before.
    (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, "triggers"),
    // `ATTACH` creating or writing a file is a filesystem capability a payload must not
    // have. `SQLITE_LIMIT_ATTACHED = 0` below makes it unreachable anyway; these make it
    // unreachable *and* unprivileged.
    (
        DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE,
        "attach-create",
    ),
    (
        DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE,
        "attach-write",
    ),
];

/// What the audit found. Every field is read from the payload, nothing is assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadAudit {
    /// What `PRAGMA integrity_check` said. `"ok"` when sound.
    pub integrity: String,
    /// How many referential violations `PRAGMA foreign_key_check` found.
    ///
    /// Counted separately from `integrity` because the two answer different questions:
    /// T-80 proves a dangling reference leaves `integrity_check` reporting `ok`.
    pub foreign_key_violations: i64,
    /// Schema version recorded in the payload, or `0` if it has never been migrated.
    pub schema_version: u32,
    /// Whether the payload carries an `app_meta` table, so it is plausibly an Evara project.
    pub looks_like_a_project: bool,
    /// How many views the payload declares.
    ///
    /// Reported rather than refused here: an Evara project declares none, and the
    /// conformance comparison is what turns that into a refusal. Carried so a refusal can
    /// say what was unexpected.
    pub view_count: i64,
    /// How many triggers the payload declares.
    pub trigger_count: i64,
}

impl PayloadAudit {
    /// Whether the payload is structurally and referentially sound.
    ///
    /// Says nothing about whether it is the *right* schema — that is
    /// [`super::conformance`]'s question, and conflating the two is the mistake T-79 exists
    /// to prevent.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        self.integrity == "ok" && self.foreign_key_violations == 0
    }
}

/// A staged payload, open read-only with its capabilities turned down.
///
/// Holds the connection so nothing else can obtain one on the same file without going
/// through this constructor — the same reason [`crate::Database`] never hands out a
/// `Connection`.
#[derive(Debug)]
pub struct UntrustedPayload {
    conn: Connection,
    name: String,
}

impl UntrustedPayload {
    /// Opens `path` as an untrusted payload under phase V0.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectNotFound`] if nothing is there.
    /// - [`DbError::NotAnEvaraProject`] if the bytes are not a database. Probed with one read
    ///   of `sqlite_master`, because a read-only open of arbitrary bytes **succeeds** —
    ///   SQLite does not read the file header until the first statement.
    /// - [`DbError::Open`] if SQLite will not open it at all.
    /// - [`DbError::PragmaRejected`] if a control did not take effect. Read back rather than
    ///   assumed: a pragma that silently failed would leave the payload opened with full
    ///   capabilities while the code believed otherwise.
    pub fn open(path: &Path) -> Result<Self> {
        // The URI boundary, which is a property of the **path** and not of a flag — F-2.
        // The bundled engine is compiled with `-DSQLITE_USE_URI`, so URI interpretation is
        // the global default and omitting `SQLITE_OPEN_URI` below does not disable it
        // (`t91_…` proves that against the engine). What makes the question closed is that
        // every path reaching here is one Evara minted, and that its form is asserted.
        //
        // Asked **before** the existence check, so `file:x?mode=rwc` is reported as the
        // path form it is rather than as a file that happens not to exist.
        super::paths::require_plain_database_path(path)?;

        if !path.is_file() {
            return Err(DbError::ProjectNotFound {
                name: file_name_of(path),
            });
        }

        // File-level read-only, which is a stronger statement than the `query_only` pragma:
        // SQLite will not open a write journal against it at all. Deliberately **not**
        // `open_plain`, which would run `PRAGMA journal_mode = WAL` and write the header of
        // a file we have not vetted.
        //
        // `SQLITE_OPEN_URI` is **not** requested. It cannot subtract URI handling from this
        // build, so this is a statement of intent rather than a control: nothing here asks
        // for a capability the path guard above exists to deny.
        //
        // `SQLITE_OPEN_NO_MUTEX` is not requested: the connection never leaves this type.
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|source| DbError::opening(path, source))?;

        let payload = Self {
            conn,
            name: file_name_of(path),
        };
        payload.harden()?;

        // The probe that turns "not a database" into a sentence. `Connection::open_with_flags`
        // against a read-only file **succeeds** on bytes that are not a database at all:
        // SQLite does not read the header until the first statement, so without this the
        // failure arrives later as an opaque internal error from whichever query happened to
        // run first. One cheap read of `sqlite_master` is where it belongs.
        if payload
            .conn
            .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .is_err()
        {
            return Err(DbError::NotAnEvaraProject {
                name: payload.name.clone(),
            });
        }

        Ok(payload)
    }

    /// Applies every V0 control and verifies each one took effect.
    fn harden(&self) -> Result<()> {
        // DEFENSIVE first: it is the switch that makes several of the others stick.
        self.conn
            .set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
            .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;

        for (config, label) in AUDIT_DISABLED {
            self.conn
                .set_db_config(*config, false)
                .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;

            // Read back. `set_db_config` returns the resulting value, but reading it
            // separately is what catches an engine that accepted the call and ignored it.
            let still_on = self
                .conn
                .db_config(*config)
                .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;
            if still_on {
                return Err(DbError::PragmaRejected {
                    pragma: "db_config",
                    expected: format!("{label} = off"),
                    actual: format!("{label} = on"),
                });
            }
        }

        // Limits. Each is a ceiling on what the *payload* can make the engine allocate or
        // attempt, so a declared-enormous structure fails rather than exhausting memory.
        //
        // `ATTACHED = 0` is the important one: with no attach slots, `ATTACH` cannot
        // succeed whatever the capability flags say. A payload reaching the filesystem
        // through SQL is the capability that would matter most.
        for (limit, value) in [
            (rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0),
            (
                rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
                64 * 1024 * 1024,
            ),
            (
                rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,
                1024 * 1024,
            ),
            (rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 2_000),
            (rusqlite::limits::Limit::SQLITE_LIMIT_EXPR_DEPTH, 1_000),
            (rusqlite::limits::Limit::SQLITE_LIMIT_COMPOUND_SELECT, 50),
            (
                rusqlite::limits::Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH,
                1_000,
            ),
        ] {
            self.conn
                .set_limit(limit, value)
                .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;
            let applied = self
                .conn
                .limit(limit)
                .map_err(|source| DbError::Internal(InternalError::wrap(source)))?;
            if applied != value {
                return Err(DbError::PragmaRejected {
                    pragma: "limit",
                    expected: value.to_string(),
                    actual: applied.to_string(),
                });
            }
        }

        // `query_only` as well as the read-only open flag. Belt and braces, and cheap: the
        // flag stops the file being written, this stops a statement even trying.
        self.conn.pragma_update(None, "query_only", "ON")?;
        let query_only: i64 = self
            .conn
            .query_row("PRAGMA query_only", [], |row| row.get(0))?;
        if query_only == 0 {
            return Err(DbError::PragmaRejected {
                pragma: "query_only",
                expected: "1".to_owned(),
                actual: "0".to_owned(),
            });
        }

        Ok(())
    }

    /// File name of the payload, for messages. Never the full path.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Reads the payload's connection, for the conformance comparison.
    pub(crate) const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Runs the V0 audit.
    ///
    /// Both integrity questions, because T-79 and T-80 prove neither subsumes the other: a
    /// hostile trigger leaves `integrity_check` saying `ok`, and a dangling reference does
    /// too while `foreign_key_check` reports it.
    ///
    /// # Errors
    ///
    /// Propagates SQLite failures. A payload so damaged that the audit itself cannot run
    /// reaches the caller as [`DbError::Internal`], which is the right answer — it is not a
    /// project.
    pub fn audit(&self) -> Result<PayloadAudit> {
        let integrity: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;

        let foreign_key_violations: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })?;

        let looks_like_a_project: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_meta'",
            [],
            |row| row.get(0),
        )?;

        let view_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view'",
            [],
            |row| row.get(0),
        )?;
        let trigger_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'",
            [],
            |row| row.get(0),
        )?;

        // Read through the migration runner's own reader, so a payload's claimed version is
        // interpreted exactly as an ordinary open would interpret it.
        let schema_version = migrations::applied_schema_version(&self.conn)?;

        Ok(PayloadAudit {
            integrity,
            foreign_key_violations,
            schema_version,
            looks_like_a_project: looks_like_a_project > 0,
            view_count,
            trigger_count,
        })
    }
}
