//! Comparing a payload's schema against one this binary generates.
//!
//! # Why generated and not written down
//!
//! The alternative is a hand-maintained allow-list of tables, columns, indexes and
//! triggers. It would be wrong within one migration: the project schema is at V18 with
//! roughly 74 migration-created triggers and over a hundred change-log triggers, and
//! nothing would fail if somebody added a table and forgot the list. A stale allow-list does
//! not report a gap — it silently accepts whatever it never heard of.
//!
//! So the reference is **built by running this binary's own migrations** into a scratch
//! database, up to the version the payload claims. The comparison therefore cannot drift
//! from the migrations, because it *is* the migrations.
//!
//! `migrations::run_up_to` is what makes it possible, and it is why that function stopped
//! being `#[cfg(test)]`: the payload's claimed version is input from a file written
//! elsewhere, so an impossible version is now a refusal rather than a test-time panic.
//!
//! # Order, which is a decision and not an implementation detail
//!
//! Conformance runs **before** the payload is migrated **and before its identity is read**
//! (ADR 0056). An extra table or an unexpected trigger must be refused before any code
//! trusts a row out of the file. Reading the identity first would mean trusting
//! `app_meta` — a table the payload itself supplied — to decide whether to trust the
//! payload.
//!
//! # What this does and does not prove
//!
//! It proves the payload's visible schema is what an Evara project at that version looks
//! like. It is **not a sandbox**: T-79 proves `PRAGMA integrity_check` returns `ok` on a
//! database carrying a hostile trigger, and this comparison is what catches that one — but
//! neither makes SQLite's parser infallible. §6.2 is the statement of what is not
//! guaranteed, and nothing here should be read as widening it.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::error::{DbError, Result};
use crate::{connection, migrations};

/// One way a payload's schema differs from the reference.
///
/// Carried rather than collapsed into a boolean so a refusal can name what was wrong. A
/// message saying only *"the schema does not match"* gives a user nothing to act on and
/// gives a bug report nothing to go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDifference {
    /// The payload has an object the reference does not. The dangerous direction: an extra
    /// trigger or view is how a payload gets to execute something.
    Unexpected {
        /// `table`, `index`, `trigger` or `view`.
        kind: String,
        /// Object name.
        name: String,
    },
    /// The reference has an object the payload does not.
    Missing {
        /// `table`, `index`, `trigger` or `view`.
        kind: String,
        /// Object name.
        name: String,
    },
    /// Both have it, with different definitions.
    Altered {
        /// `table`, `index`, `trigger` or `view`.
        kind: String,
        /// Object name.
        name: String,
    },
}

impl std::fmt::Display for SchemaDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unexpected { kind, name } => write!(f, "unexpected {kind} `{name}`"),
            Self::Missing { kind, name } => write!(f, "missing {kind} `{name}`"),
            Self::Altered { kind, name } => write!(f, "{kind} `{name}` differs"),
        }
    }
}

/// A schema this binary produced, to compare a payload against.
#[derive(Debug, Clone)]
pub struct SchemaReference {
    version: u32,
    objects: BTreeMap<(String, String), String>,
}

impl SchemaReference {
    /// Builds the reference for `version` by migrating a scratch database to it.
    ///
    /// `scratch` is supplied by the caller rather than taken from the system temporary
    /// directory, for two reasons. It avoids making `tempfile` a production dependency of
    /// this crate for one function — it is a dev-dependency today. And the caller already
    /// owns a staging directory beside the payload, which is the right place: same volume,
    /// same cleanup, and nothing of a user's project passes through a shared temp folder,
    /// which a local-first application should not do casually.
    ///
    /// The reference is a real file rather than `:memory:`: migrations run against a real
    /// file in every other path, and an in-memory database differs in ways that are small
    /// but real — page size, `journal_mode` acceptance — so the reference is generated the
    /// way the thing it describes was.
    ///
    /// The scratch file is removed before returning. The reference is the `objects` map, not
    /// the database.
    ///
    /// # Errors
    ///
    /// - [`DbError::UnknownSchemaVersion`] if this build has no migration for `version`,
    ///   which is how a payload claiming an impossible version is refused.
    /// - [`DbError::Migration`] if the scratch migration fails.
    pub fn generate_in(scratch: &std::path::Path, version: u32) -> Result<Self> {
        std::fs::create_dir_all(scratch).map_err(|source| DbError::SettingsIo {
            action: "create a scratch directory for the reference schema",
            source,
        })?;
        let path = scratch.join(format!(
            "reference-v{version}-{}.sqlite",
            uuid::Uuid::now_v7()
        ));

        let objects = Self::build(&path, version);

        // Removed whatever happened. The sidecars go too: `open_plain` puts the reference
        // in WAL, so it is up to three files, and leaving them in the caller's staging
        // directory would be litter that S3's sidecar assertions would later trip over.
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(super::sidecars::sidecar_path(&path, suffix));
        }

        Ok(Self {
            version,
            objects: objects?,
        })
    }

    /// Migrates a scratch database and reads its schema back.
    fn build(path: &std::path::Path, version: u32) -> Result<BTreeMap<(String, String), String>> {
        let mut conn = connection::open_plain(path)?;
        migrations::run_up_to(&mut conn, version)?;
        // Change-log triggers are installed on every ordinary open, not by the migrations
        // alone, so a reference without them would report every project's triggers as
        // unexpected.
        let tx = conn.transaction()?;
        crate::change_log::install_all(&tx)?;
        tx.commit()?;

        let objects = read_objects(&conn)?;
        // Closed before the caller removes the file, so its WAL is checkpointed away.
        conn.close()
            .map_err(|(_, source)| DbError::Internal(crate::error::InternalError::wrap(source)))?;
        Ok(objects)
    }

    /// The version this reference describes.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// How many schema objects it holds.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Every way `payload` differs from this reference, in a stable order.
    ///
    /// Empty means conforming.
    #[must_use]
    pub fn differences(
        &self,
        payload: &BTreeMap<(String, String), String>,
    ) -> Vec<SchemaDifference> {
        let mut differences = Vec::new();

        for ((kind, name), definition) in payload {
            match self.objects.get(&(kind.clone(), name.clone())) {
                None => differences.push(SchemaDifference::Unexpected {
                    kind: kind.clone(),
                    name: name.clone(),
                }),
                Some(expected) if expected != definition => {
                    differences.push(SchemaDifference::Altered {
                        kind: kind.clone(),
                        name: name.clone(),
                    });
                }
                Some(_) => (),
            }
        }

        for (kind, name) in self.objects.keys() {
            if !payload.contains_key(&(kind.clone(), name.clone())) {
                differences.push(SchemaDifference::Missing {
                    kind: kind.clone(),
                    name: name.clone(),
                });
            }
        }

        // `Unexpected` first: an extra trigger is the finding that matters most, and a
        // refusal naming it is more useful than one that happens to list a missing index
        // first. Within a kind, by name, so the order is deterministic.
        differences.sort_by_key(|difference| match difference {
            SchemaDifference::Unexpected { kind, name } => (0, kind.clone(), name.clone()),
            SchemaDifference::Altered { kind, name } => (1, kind.clone(), name.clone()),
            SchemaDifference::Missing { kind, name } => (2, kind.clone(), name.clone()),
        });
        differences
    }
}

/// Reads every schema object from a database, normalised for comparison.
///
/// `sqlite_%` is SQLite's own bookkeeping and `refinery_schema_history` is the migration
/// runner's. Both are excluded: the first is not ours to compare, and the second legitimately
/// differs — a payload migrated through a different sequence of releases has different rows
/// in it and the same resulting schema, which is the whole reason the comparison is of
/// *schema* and not of bytes.
///
/// Auto-indexes (`sqlite_autoindex_*`) are excluded for a subtler reason: SQLite creates them
/// implicitly for `UNIQUE` and `PRIMARY KEY` constraints and their numbering depends on the
/// order constraints were declared in, which a table rebuild can change without changing
/// what the table means.
pub(crate) fn read_objects(conn: &Connection) -> Result<BTreeMap<(String, String), String>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, COALESCE(sql, '')
           FROM sqlite_master
          WHERE name NOT LIKE 'sqlite_%'
            AND name <> 'refinery_schema_history'
          ORDER BY type, name",
    )?;
    let rows = stmt.query_map([], |row| {
        let kind: String = row.get(0)?;
        let name: String = row.get(1)?;
        let sql: String = row.get(2)?;
        Ok(((kind, name), normalise(&sql)))
    })?;
    Ok(rows.collect::<std::result::Result<BTreeMap<_, _>, _>>()?)
}

/// Collapses whitespace in a schema definition.
///
/// SQLite stores the `CREATE` statement verbatim, comments and line breaks included, so two
/// identical schemas written by two releases of the same migration would compare unequal on
/// formatting alone. Only runs of whitespace are collapsed — nothing is lowercased and no
/// token is removed, because a comparison that normalised away case would stop noticing a
/// column renamed from `Name` to `name`.
fn normalise(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Compares `payload`'s schema against a reference generated for its claimed version.
///
/// # Errors
///
/// - [`DbError::SchemaDoesNotConform`] listing every difference found.
/// - [`DbError::UnknownSchemaVersion`] if the claimed version cannot be generated.
pub fn check(
    payload: &super::boundary::UntrustedPayload,
    scratch: &std::path::Path,
    version: u32,
) -> Result<()> {
    let reference = SchemaReference::generate_in(scratch, version)?;
    let found = read_objects(payload.connection())?;
    let differences = reference.differences(&found);

    if differences.is_empty() {
        return Ok(());
    }

    // Capped at eight in the message. A payload that is not a project at all differs in
    // hundreds of ways, and a sentence listing every one is unreadable; the count is what
    // tells the reader it is wholesale rather than a near miss.
    let shown = differences
        .iter()
        .take(8)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    let summary = if differences.len() > 8 {
        format!("{shown}; and {} more", differences.len() - 8)
    } else {
        shown
    };

    Err(DbError::SchemaDoesNotConform { version, summary })
}
