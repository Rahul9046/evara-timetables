//! Everything a manifest says about a project, read out of the file it describes.
//!
//! # Why these are read here and not composed by the caller
//!
//! `evara-project` owns the `.evara` format and may not write SQL — that is the layering
//! rule, and it is why the facts a manifest carries are read in this crate and handed over
//! as plain values. The alternative, exposing a connection, is the one thing
//! [`crate::Database`] has never done.
//!
//! # Why from the copy and never from the source
//!
//! Finding 4 of the architecture review. Revision 1 read the manifest's facts from the live
//! project while the snapshot was being taken, which means the manifest could describe a
//! state the payload did not hold — a count off by whatever a concurrent write changed. So
//! [`ProjectFacts`] is read from the snapshot file itself, after the workspace lock has been
//! released, and the only instant it can describe is the one the snapshot captured.
//!
//! # `capabilities` is derived, never hand-maintained
//!
//! D3. A constraint type ships in the **registry** rather than in a migration, so a build can
//! add a tenth `type_key` with no schema change, and a project configuring it would import
//! perfectly into an older build and then fail the moment anybody evaluated. The
//! `type_key`s actually stored are therefore read from the data
//! ([`ProjectFacts::constraint_type_keys`]) and the archive declares them, so a build that
//! cannot read one refuses rather than dropping a rule. A hand-kept list would go stale
//! exactly when it mattered.
//!
//! The feature markers are the same shape: `lineage:repair` is true because a
//! `based_on_timetable_id` edge exists, `lock:placement` because a `timetable_entry` is
//! locked. Nothing here is a flag somebody sets.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::Connection;
use uuid::Uuid;

use crate::error::{DbError, InternalError, Result};
use crate::{change_log, connection, meta, migrations, repo};

/// One school, as the manifest lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchoolSummary {
    /// Stable school identifier, preserved across transfer.
    pub id: Uuid,
    /// The school's name. Synthetic in every fixture, per `CLAUDE.md` rule 3.
    pub name: String,
    /// How many campuses it holds.
    pub campuses: i64,
}

/// A feature of the data a reader has to understand to render the project honestly.
///
/// A set of named variants rather than four booleans, and not only because clippy counts
/// bools: `ProjectFacts::capabilities` is a set operation, every one of these is evidence
/// read from the data rather than a flag anybody sets, and a future marker is one variant
/// rather than one more field every constructor has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PayloadFeature {
    /// A timetable names a parent — a repair or a duplication edge. V18's lineage.
    RepairLineage,
    /// A placement is locked. Per version, never per lesson (ADR 0045).
    PlacementLocks,
    /// A placement was made by hand rather than by a solve.
    ManualPlacements,
    /// A school logo is stored in the project (ADR 0016's `school_logo` row).
    EmbeddedLogo,
}

impl PayloadFeature {
    /// Every feature, so a caller can probe for all of them without listing them again.
    pub const ALL: &'static [Self] = &[
        Self::RepairLineage,
        Self::PlacementLocks,
        Self::ManualPlacements,
        Self::EmbeddedLogo,
    ];

    /// The namespaced capability string an archive declares.
    #[must_use]
    pub const fn as_capability(self) -> &'static str {
        match self {
            Self::RepairLineage => "lineage:repair",
            Self::PlacementLocks => "lock:placement",
            Self::ManualPlacements => "placement:manual",
            Self::EmbeddedLogo => "logo:embedded",
        }
    }

    /// The one-row probe that decides whether this project has it.
    const fn probe(self) -> &'static str {
        match self {
            Self::RepairLineage => {
                "SELECT 1 FROM timetable WHERE based_on_timetable_id IS NOT NULL LIMIT 1"
            }
            Self::PlacementLocks => "SELECT 1 FROM timetable_entry WHERE is_locked = 1 LIMIT 1",
            Self::ManualPlacements => "SELECT 1 FROM timetable_entry WHERE is_manual = 1 LIMIT 1",
            Self::EmbeddedLogo => "SELECT 1 FROM school_logo LIMIT 1",
        }
    }
}

/// What the change feed looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeLogSummary {
    /// How many rows the feed holds.
    pub entries: i64,
    /// Highest `seq`, which is what a stored `inputs_watermark` points at.
    ///
    /// Carried beside `entries` rather than derived from it: a feed whose oldest rows were
    /// never written by this installation still has a monotonic `seq`, so the two numbers
    /// are not the same fact and a reader comparing them would be comparing nothing.
    pub watermark: i64,
}

/// Every fact a manifest carries, read from one database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFacts {
    /// `project_id` and `created_by_site_id`, never regenerated.
    pub identity: meta::ProjectIdentity,
    /// Which migration the database stands at.
    pub schema_version: u32,
    /// Row counts for every table discovered from `sqlite_master`.
    pub census: BTreeMap<String, i64>,
    /// The change feed.
    pub change_log: ChangeLogSummary,
    /// Every school, by name.
    pub schools: Vec<SchoolSummary>,
    /// Distinct `constraint_instance.type_key` values, sorted.
    pub constraint_type_keys: Vec<String>,
    /// Distinct `cycle.kind` values, sorted.
    pub cycle_kinds: Vec<String>,
    /// Which features of the data this project exercises, read from the data.
    pub features: BTreeSet<PayloadFeature>,
}

impl ProjectFacts {
    /// The capability vocabulary this project requires a reader to understand.
    ///
    /// Namespaced, so a prefix a future build adds is a known-unknown rather than a parse
    /// ambiguity. Sorted and deduplicated, so two exports of one project produce identical
    /// bytes — which is what T-05 asserts.
    #[must_use]
    pub fn capabilities(&self) -> Vec<String> {
        let mut capabilities: Vec<String> = Vec::new();
        for key in &self.constraint_type_keys {
            capabilities.push(format!("constraint:{key}"));
        }
        for kind in &self.cycle_kinds {
            capabilities.push(format!("cycle-kind:{kind}"));
        }
        for feature in &self.features {
            capabilities.push(feature.as_capability().to_owned());
        }
        capabilities.sort();
        capabilities.dedup();
        capabilities
    }

    /// Row count for one table, or `0` if the table is not in the census.
    ///
    /// A missing table reads as zero deliberately: the census is discovered, so a table
    /// absent from it did not exist at that schema version, and a caller asking about it
    /// wants "none" rather than a panic.
    #[must_use]
    pub fn count_of(&self, table: &str) -> i64 {
        self.census.get(table).copied().unwrap_or_default()
    }
}

/// Reads every fact from an open connection.
pub(crate) fn read_facts(conn: &Connection) -> Result<ProjectFacts> {
    let schema_version = migrations::applied_schema_version(conn)?;
    let identity = meta::read_identity(conn)?;
    let census = table_census_from(conn)?;

    let entries: i64 = conn.query_row("SELECT COUNT(*) FROM change_log", [], |row| row.get(0))?;
    let watermark = change_log::watermark(conn)?.get();

    let mut schools = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT s.id,
                    s.name,
                    (SELECT COUNT(*) FROM campus c WHERE c.school_id = s.id)
               FROM school s
              ORDER BY s.name, s.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (id, name, campuses) = row?;
            schools.push(SchoolSummary {
                id: repo::row::parse_id_text("school.id", &id)?,
                name,
                campuses,
            });
        }
    }

    Ok(ProjectFacts {
        identity,
        schema_version,
        census,
        change_log: ChangeLogSummary { entries, watermark },
        schools,
        constraint_type_keys: distinct_text(
            conn,
            "SELECT DISTINCT type_key FROM constraint_instance ORDER BY type_key",
        )?,
        cycle_kinds: distinct_text(conn, "SELECT DISTINCT kind FROM cycle ORDER BY kind")?,
        features: features_of(conn)?,
    })
}

/// Reads every fact from a database file, which nothing else holds open.
///
/// Opened with no site-id function registered and nothing writable: a fact read is a read,
/// and a connection that *could* write here would be a capability with no use.
///
/// # Errors
///
/// - [`DbError::DestinationPathUnsafe`] if the path is not a plain file path — F-2.
/// - [`DbError::Open`] if SQLite will not open the file.
/// - [`DbError::MissingMetadata`] if it is not an Evara project after all.
pub fn facts_of(path: &Path) -> Result<ProjectFacts> {
    super::paths::require_plain_database_path(path)?;
    let conn = connection::open_plain(path)?;
    let facts = read_facts(&conn);
    // Closed explicitly so the file is a single file again before the caller moves or
    // hashes it: `open_plain` puts the database in WAL and so creates sidecars beside it.
    conn.close()
        .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    facts
}

/// Row counts for every ordinary table in a database file.
///
/// The round-trip assertion's primitive: a census of the **destination** after an import,
/// compared against the census the snapshot carried. Discovered from `sqlite_master` rather
/// than listed, so a table a future migration adds is compared without anybody remembering
/// to add it here — which is what T-01 turns on.
///
/// # Errors
///
/// As [`facts_of`].
pub fn table_census(path: &Path) -> Result<BTreeMap<String, i64>> {
    super::paths::require_plain_database_path(path)?;
    let conn = connection::open_plain(path)?;
    let census = table_census_from(&conn);
    conn.close()
        .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    census
}

/// Row counts for every ordinary table on an open connection.
///
/// `sqlite_%` is SQLite's own bookkeeping and `refinery_schema_history` is the migration
/// runner's. Neither is project data, and the second legitimately differs between two
/// projects with the same schema — a project migrated through a different sequence of
/// releases has different rows in it — so comparing it would report a difference that means
/// nothing.
pub(crate) fn table_census_from(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table'
               AND name NOT LIKE 'sqlite_%'
               AND name <> 'refinery_schema_history'
             ORDER BY name",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;

    let mut census = BTreeMap::new();
    for table in tables {
        // The name comes from `sqlite_master`, so it is an existing identifier and cannot be
        // bound as a parameter. Double-quoted so a table named like a keyword still reads.
        let count: i64 =
            conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })?;
        census.insert(table, count);
    }
    Ok(census)
}

/// Which features the data exercises.
///
/// Every variant is probed, from `PayloadFeature::ALL`, so a feature added to the enum is
/// read without a second place to remember — the same reason the census is discovered rather
/// than listed.
fn features_of(conn: &Connection) -> Result<BTreeSet<PayloadFeature>> {
    let mut found = BTreeSet::new();
    for feature in PayloadFeature::ALL {
        if exists(conn, feature.probe())? {
            found.insert(*feature);
        }
    }
    Ok(found)
}

/// How many bytes each schema object occupies on disk.
///
/// # Why this exists, and why it is not a sum of column widths
///
/// `T-60` asks what share of a payload the change feed is, and the honest answer is a page
/// count. A row's cost on disk is its **page occupancy** — overflow pages, free space inside a
/// partly filled page, the index entries beside it — not the length of its text, so an estimate
/// from `length(...)` would be wrong in whichever direction the page fill happened to go.
///
/// So this reads SQLite's own `dbstat` virtual table, which the bundled build compiles in
/// (`-DSQLITE_ENABLE_DBSTAT_VTAB`, `libsqlite3-sys-0.35.0/build.rs:129`). It is exact, it is a
/// read, and it needs no copy of the file — which matters, because the alternative considered
/// first was *deleting the feed from a copy and comparing sizes*, and that would have meant a
/// public function whose only purpose was to break a project.
///
/// Keyed by `dbstat.name`, which is a **table or index** name: `change_log` and every index on
/// it appear separately, so a caller asking about a table's total cost sums the entries whose
/// name begins with it. Returned rather than aggregated here, because which indexes belong to
/// a table is the caller's question and guessing it from a name prefix inside this function
/// would make a table called `activity` include one called `activity_log`.
///
/// # Errors
///
/// As [`facts_of`].
pub fn page_bytes_by_object(path: &Path) -> Result<BTreeMap<String, i64>> {
    super::paths::require_plain_database_path(path)?;
    let conn = connection::open_plain(path)?;
    let measured = (|| {
        let mut stmt =
            conn.prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
    })();
    conn.close()
        .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    measured
}

/// How many pages a database holds on its free list.
///
/// The figure §7.2's inflation question is actually about. A snapshot taken by the backup API
/// reproduces the source **page for page, free pages included**, so a project that has had
/// data deleted carries its free list into the payload — and a project that has never deleted
/// anything has no free list and shows no inflation at all, which is why a measurement taken
/// against one would say nothing.
///
/// `PRAGMA freelist_count` rather than arithmetic over `page_count` and `dbstat`: it is the
/// engine's own answer, and the two would disagree by whatever the page cache had not yet
/// written.
///
/// # Errors
///
/// As [`facts_of`].
pub fn free_page_count(path: &Path) -> Result<i64> {
    super::paths::require_plain_database_path(path)?;
    let conn = connection::open_plain(path)?;
    let count = conn.query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0));
    conn.close()
        .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    Ok(count?)
}

/// The page size a database uses, in bytes.
///
/// Read rather than assumed. Evara never sets `page_size`, so every project uses the engine's
/// compiled-in default — but "the default is four kilobytes" is a fact about a build and not
/// about a file, and a measurement that multiplied a page count by a constant would be quietly
/// wrong on a database some other tool had rewritten.
///
/// # Errors
///
/// As [`facts_of`].
pub fn page_size_of(path: &Path) -> Result<i64> {
    super::paths::require_plain_database_path(path)?;
    let conn = connection::open_plain(path)?;
    let size = conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0));
    conn.close()
        .map_err(|(_, source)| DbError::Internal(InternalError::wrap(source)))?;
    Ok(size?)
}

/// Whether a one-row probe returns anything.
fn exists(conn: &Connection, sql: &str) -> Result<bool> {
    let found: Option<i64> = conn.query_row(sql, [], |row| row.get(0)).optional_row()?;
    Ok(found.is_some())
}

/// Distinct text values from a query, in the order the query gives them.
fn distinct_text(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(sql)?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?)
}

/// `Option`-returning row reads, so "no row" is not an error.
trait OptionalRow<T> {
    /// Maps SQLite's no-rows failure to `None`.
    fn optional_row(self) -> Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional_row(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(other) => Err(DbError::from(other)),
        }
    }
}
