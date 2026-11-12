//! Projects as earlier releases left them, for the T-41 compatibility matrix.
//!
//! # What this is for
//!
//! §1.4.6 of the M10 proposal records a gap: *nothing proves a historical project opens in
//! today's build*. `find . -name '*.evaraproj'` outside `target/` returns nothing, and the
//! only historical coverage was one hand-written pre-V12 fixture. This module is what T-41's
//! matrices are built from: a project at **any** schema version, seeded with what that schema
//! can hold.
//!
//! No binary fixture enters git. Everything here is generated deterministically, which is
//! this repository's practice and also what makes the matrices extend themselves when V19
//! lands.
//!
//! # What the consumer found, so a reader of this module is not misled
//!
//! `evara-project`'s `tests_historical` takes these projects down two paths, and only one of
//! them spans the whole chain:
//!
//! - **open and migrate** — `Database::open_existing`, V1…V18, the path every existing project
//!   actually takes. That is §1.4.6's question, and it found a real defect: `backup.rs` counted
//!   `FROM school` for every project below V12, and `school` arrives at V3, so a V1 or V2 file
//!   could not be opened at all.
//! - **export and import** — only at the **current** version, structurally. A manifest is built
//!   from `facts_of`, whose `RepairLineage` probe reads `based_on_timetable_id`, a V18 column,
//!   so no archive can be assembled for an earlier payload. Which matches reality: transfer
//!   shipped at V18 and no real archive carries an older payload.
//!
//! So `write_project_at` takes a version and makes no promise about what can be done with the
//! result — that is the caller's question, and the two answers differ.
//!
//! # Why it is a generator and not eighteen fixtures
//!
//! Eighteen hand-maintained fixtures would be eighteen things to update whenever a migration
//! adds a column, and the one most likely to rot is the one nobody runs. So the seed plan is
//! **declared once** ([`PLAN`]) and applied **version-awarely**:
//!
//! 1. a row whose table does not exist at this version is skipped, with a reason;
//! 2. a declared column that does not exist at this version is **dropped from the insert** —
//!    which is how one declaration covers `school` both before and after V12 removed
//!    `only_one` and `working_days`;
//! 3. a column that is `NOT NULL` with no default and is **not** covered by the declaration
//!    makes the row skipped, with a reason naming the column.
//!
//! Rule 3 is the one that matters. Without it a migration adding a required column would
//! silently stop the row being seeded, and the matrix would keep passing while testing less
//! and less. `Historical::skipped` carries every reason out to the caller, and the matrix
//! asserts against it rather than trusting it.
//!
//! # What is deliberately not here
//!
//! **No production API is widened.** `migrations::run_up_to` stays `pub(crate)`; this module
//! is inside `evara-db`, so it can call it, and only this module is public — behind the
//! off-by-default `fixtures` feature, like everything else in [`super`]. Nothing shipped
//! carries it, and no crate outside gains a way to build a half-migrated database.
//!
//! **No timetable layer.** A stored timetable needs a real `SolveOutcome`, which needs the
//! solver, which `evara-db` must not depend on. The rich-data round trip is covered at V18 by
//! `evara-project`'s own fixture; what the matrix is for is the **schema chain**.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use uuid::Uuid;

use crate::error::Result;

/// The timestamp every seeded row carries. Fixed, so a fixture's bytes do not move.
const STAMP: &str = "2026-01-05T09:00:00.000Z";

/// The app version a historical project claims to have been created with.
const CREATED_WITH: &str = "0.0.1-historical";

/// One value in a declared row.
#[derive(Debug, Clone, Copy)]
enum Val {
    /// A literal string.
    Text(&'static str),
    /// A literal integer.
    Int(i64),
    /// This row's own minted identifier.
    Id,
    /// The identifier minted for another table's row, by table name.
    ///
    /// What gives the seeded data referential integrity without anybody writing a join: the
    /// generator resolves it from what it has already inserted, and a reference to a table
    /// that was skipped skips this row too rather than inserting a dangling one.
    Ref(&'static str),
    /// [`STAMP`].
    Stamp,
}

/// One row to insert, if the schema at this version admits it.
struct Row {
    /// Which table.
    table: &'static str,
    /// Declared columns, in no particular order. Filtered to those that exist.
    columns: &'static [(&'static str, Val)],
}

/// The seed plan, in foreign-key order.
///
/// Ordered by dependency rather than by version, because that is what the inserts need; each
/// row's availability is decided from the schema at runtime, not from a version number
/// written here. A version annotation would be a second source of truth about when a table
/// appeared, and the database is the first one.
///
/// Values are chosen to satisfy the `CHECK` constraints the schema actually carries —
/// non-empty trimmed names, `rev >= 1`, non-negative capacities, the `IN (…)` sets — because
/// a `STRICT` table with checks is not something a generic value synthesiser can satisfy by
/// guessing.
const PLAN: &[Row] = &[
    Row {
        table: "school",
        columns: &[
            ("id", Val::Id),
            ("name", Val::Text("Northgate Secondary (synthetic)")),
            ("timezone", Val::Text("Europe/London")),
            ("locale", Val::Text("en-GB")),
            // V3…V11 only; V12 removed both. Dropped automatically afterwards.
            ("only_one", Val::Int(1)),
            ("working_days", Val::Int(31)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "campus",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("Main")),
            ("code", Val::Text("MAIN")),
            ("address", Val::Text("1 Example Way")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "building",
        columns: &[
            ("id", Val::Id),
            ("campus_id", Val::Ref("campus")),
            ("name", Val::Text("Science Block")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "room_type",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("Laboratory")),
            ("code", Val::Text("LAB")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    // V5 onwards. A building reaches its campuses through this table, and V5 added a trigger
    // refusing a room whose building is not on the room's campus — so this row has to exist
    // before `room` is inserted, which is why the plan is ordered by dependency.
    Row {
        table: "building_campus",
        columns: &[
            ("id", Val::Id),
            ("building_id", Val::Ref("building")),
            ("campus_id", Val::Ref("campus")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "room",
        columns: &[
            ("id", Val::Id),
            ("campus_id", Val::Ref("campus")),
            ("building_id", Val::Ref("building")),
            ("room_type_id", Val::Ref("room_type")),
            ("name", Val::Text("Lab 1")),
            ("code", Val::Text("L1")),
            ("capacity", Val::Int(30)),
            ("is_bookable", Val::Int(1)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    // V6 onwards, the same shape for rooms.
    Row {
        table: "room_campus",
        columns: &[
            ("id", Val::Id),
            ("room_id", Val::Ref("room")),
            ("campus_id", Val::Ref("campus")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "year_level",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("Year 7")),
            ("ordinal", Val::Int(7)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "academic_year",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("2026")),
            ("starts_on", Val::Text("2026-09-01")),
            ("ends_on", Val::Text("2027-07-20")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "term",
        columns: &[
            ("id", Val::Id),
            ("academic_year_id", Val::Ref("academic_year")),
            ("name", Val::Text("Autumn")),
            ("ordinal", Val::Int(1)),
            ("starts_on", Val::Text("2026-09-01")),
            ("ends_on", Val::Text("2026-12-18")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "cycle",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("Week A")),
            ("day_count", Val::Int(5)),
            ("week_count", Val::Int(1)),
            // V9 added it; before that a cycle had no kind. `CUSTOM` rather than `WEEKLY`
            // because V9 also added the rule that a weekly cycle is seven days long, and a
            // five-position custom cycle is the one shape valid at every version — which is
            // the whole point of declaring values once.
            ("kind", Val::Text("CUSTOM")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "cycle_day",
        columns: &[
            ("id", Val::Id),
            ("cycle_id", Val::Ref("cycle")),
            ("ordinal", Val::Int(1)),
            ("label", Val::Text("Monday")),
            ("weekday_hint", Val::Int(1)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "period_structure",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("campus_id", Val::Ref("campus")),
            ("name", Val::Text("Main bell schedule")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "period",
        columns: &[
            ("id", Val::Id),
            ("period_structure_id", Val::Ref("period_structure")),
            ("ordinal", Val::Int(1)),
            ("label", Val::Text("P1")),
            ("starts_at", Val::Text("09:00")),
            ("ends_at", Val::Text("09:50")),
            ("kind", Val::Text("TEACHING")),
            ("counts_as_load", Val::Int(1)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "subject",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("name", Val::Text("Mathematics")),
            ("code", Val::Text("MATH")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "teacher",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("given_name", Val::Text("Ada")),
            ("family_name", Val::Text("Synthetic")),
            ("code", Val::Text("ASY")),
            ("email", Val::Text("ada@example.invalid")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "student",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("given_name", Val::Text("Blake")),
            ("family_name", Val::Text("Synthetic")),
            ("code", Val::Text("S0001")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    Row {
        table: "student_group",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("year_level_id", Val::Ref("year_level")),
            ("name", Val::Text("7A")),
            ("code", Val::Text("7A")),
            ("kind", Val::Text("CLASS")),
            ("size", Val::Int(28)),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    // V11 onwards. Polymorphic by design — `entity_kind` names the table and there is no
    // foreign key — so it points at the seeded teacher, and a cycle day and period rather than
    // a `timeslot`, because nothing here materialises a grid.
    Row {
        table: "availability",
        columns: &[
            ("id", Val::Id),
            ("school_id", Val::Ref("school")),
            ("entity_kind", Val::Text("TEACHER")),
            ("entity_id", Val::Ref("teacher")),
            ("cycle_day_id", Val::Ref("cycle_day")),
            ("period_id", Val::Ref("period")),
            ("status", Val::Text("UNAVAILABLE")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
    // V13 onwards: a teacher is a project-level identity and their employment at a school is
    // this row. Later tables refuse a lesson without it, so it is part of the spine rather
    // than an extra.
    Row {
        table: "teacher_school",
        columns: &[
            ("id", Val::Id),
            ("teacher_id", Val::Ref("teacher")),
            ("school_id", Val::Ref("school")),
            ("status", Val::Text("ACTIVE")),
            ("created_at", Val::Stamp),
            ("updated_at", Val::Stamp),
            ("rev", Val::Int(1)),
        ],
    },
];

/// A project built as an earlier release left it.
#[derive(Debug, Clone)]
pub struct Historical {
    /// The schema version it stands at.
    pub version: u32,
    /// Its identity, which transfer must preserve.
    pub project_id: Uuid,
    /// The installation recorded as having created it.
    pub created_by_site_id: Uuid,
    /// Identifier minted per seeded table, so a caller can assert the row survived.
    pub ids: BTreeMap<&'static str, Uuid>,
    /// Tables a row was inserted into, in order.
    pub seeded: Vec<&'static str>,
    /// Tables skipped, and why — asserted by the matrix rather than trusted.
    pub skipped: Vec<(&'static str, String)>,
}

impl Historical {
    /// The identifier seeded for `table`, if it was seeded.
    #[must_use]
    pub fn id_of(&self, table: &str) -> Option<Uuid> {
        self.ids.get(table).copied()
    }
}

/// Writes a project at schema `version`, seeded with what that schema can hold.
///
/// # Errors
///
/// Propagates the open, the migration and any insert failure. An insert failing is a fixture
/// defect rather than a finding — the plan declares values the schema's checks accept — so it
/// is surfaced rather than skipped.
///
/// # Panics
///
/// If `run_up_to` stops somewhere other than `version`, which would mean the migration chain
/// and `supported_schema_version` disagree.
pub fn write_project_at(path: &Path, version: u32, site: Uuid) -> Result<Historical> {
    let mut conn = crate::connection::open_configured(path, site)?;
    let reached = crate::migrations::run_up_to(&mut conn, version)?;
    assert_eq!(
        reached, version,
        "run_up_to must stop exactly where it is told"
    );

    // The triggers every real open installs. `run_up_to` applies the migrations and nothing
    // else, so without this the fixture is a project no release would ever have written: the
    // change-log triggers are created by `Database::open_or_create` on **every** open, which
    // is why a migration adding a table only has to add it to `TRACKED_TABLES`.
    //
    // Omitting it made the V18 archive row fail conformance with 129 missing triggers — the
    // import boundary correctly refusing a file that was not shaped like an Evara project.
    {
        let tx = conn.transaction()?;
        crate::change_log::install_all(&tx)?;
        tx.commit()?;
    }

    let project_id = Uuid::now_v7();
    let created_by_site_id = Uuid::now_v7();
    seed_identity(&conn, version, project_id, created_by_site_id)?;

    let mut built = Historical {
        version,
        project_id,
        created_by_site_id,
        ids: BTreeMap::new(),
        seeded: Vec::new(),
        skipped: Vec::new(),
    };

    for row in PLAN {
        match plan_insert(&conn, row, &built.ids)? {
            Planned::Insert { sql, values, id } => {
                // Named rather than propagated raw: `DbError::Internal` deliberately hides the
                // driver's message, which is right for a product error and useless in a
                // fixture. A failing insert here is a defect in `PLAN`, and the statement is
                // what a reader needs to fix it.
                conn.execute(&sql, rusqlite::params_from_iter(values.iter()))
                    .map_err(|error| {
                        crate::DbError::internal(format!(
                            "seeding `{}` at V{version} failed: {error}; statement was: {sql}",
                            row.table
                        ))
                    })?;
                built.ids.insert(row.table, id);
                built.seeded.push(row.table);
            }
            Planned::Skip(reason) => built.skipped.push((row.table, reason)),
        }
    }

    drop(conn);
    Ok(built)
}

/// Writes the `app_meta` rows that make the file a project rather than a database.
///
/// `meta::ensure` is not used: it writes the *current* schema version and this file is
/// deliberately at an earlier one. The keys are `meta`'s own constants, so a rename there is a
/// compile error here rather than a fixture that silently stops being a project.
fn seed_identity(conn: &Connection, version: u32, project: Uuid, site: Uuid) -> Result<()> {
    for (key, value) in [
        (crate::meta::KEY_PROJECT_ID, project.to_string()),
        (crate::meta::KEY_CREATED_BY_SITE_ID, site.to_string()),
        (crate::meta::KEY_SCHEMA_VERSION, version.to_string()),
        (crate::meta::KEY_CREATED_WITH, CREATED_WITH.to_owned()),
        (crate::meta::KEY_CREATED_AT, STAMP.to_owned()),
    ] {
        conn.execute(
            "INSERT INTO app_meta (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                           updated_at = excluded.updated_at",
            (key, value, STAMP),
        )?;
    }
    Ok(())
}

/// What the generator decided about one declared row.
enum Planned {
    /// Insert it, with the columns that exist at this version.
    Insert {
        sql: String,
        values: Vec<String>,
        id: Uuid,
    },
    /// Leave it out, for a reason worth reporting.
    Skip(String),
}

/// Decides whether `row` can be inserted at this version, and how.
fn plan_insert(
    conn: &Connection,
    row: &Row,
    known: &BTreeMap<&'static str, Uuid>,
) -> Result<Planned> {
    let Some(schema) = columns_of(conn, row.table)? else {
        return Ok(Planned::Skip(String::from("the table does not exist yet")));
    };

    let id = Uuid::now_v7();
    let mut names = Vec::new();
    let mut values = Vec::new();

    for (column, value) in row.columns {
        // Rule 2: a declared column the schema does not have is dropped, which is how one
        // declaration spans a migration that removed a column.
        if !schema.iter().any(|known| &known.name == column) {
            continue;
        }
        let rendered = match value {
            Val::Text(text) => (*text).to_owned(),
            Val::Int(number) => number.to_string(),
            Val::Stamp => STAMP.to_owned(),
            Val::Id => id.to_string(),
            Val::Ref(table) => match known.get(table) {
                Some(referenced) => referenced.to_string(),
                // A reference to a row that was skipped would be a dangling key, and
                // `foreign_keys` is on. Skipping is the only answer that keeps the fixture
                // sound, and the reason names the dependency rather than the symptom.
                None => {
                    return Ok(Planned::Skip(format!(
                        "it references `{table}`, which was not seeded"
                    )));
                }
            },
        };
        names.push(*column);
        values.push(rendered);
    }

    // Rule 3: every required column must be covered, or the row is skipped and says which one
    // is missing. This is what stops a migration adding a NOT NULL column from quietly
    // shrinking what the matrix tests.
    let required: Vec<&str> = schema
        .iter()
        .filter(|column| column.required && !names.contains(&column.name.as_str()))
        .map(|column| column.name.as_str())
        .collect();
    if !required.is_empty() {
        return Ok(Planned::Skip(format!(
            "the plan does not cover its required column(s): {}",
            required.join(", ")
        )));
    }

    let placeholders = (1..=names.len())
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({placeholders})",
        row.table,
        names.join(", ")
    );
    Ok(Planned::Insert { sql, values, id })
}

/// One column of a table, as the schema has it now.
struct Column {
    name: String,
    /// `NOT NULL` with no default, so an insert must supply it.
    required: bool,
}

/// The columns of `table`, or `None` if it does not exist at this version.
fn columns_of(conn: &Connection, table: &str) -> Result<Option<Vec<Column>>> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(None);
    }

    // `PRAGMA table_info` rather than parsing the migration SQL: the database is the authority
    // on its own shape, and a generator that read the `.sql` files would be a second parser to
    // keep in step with SQLite's.
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let not_null: i64 = row.get(3)?;
            let default: Option<String> = row.get(4)?;
            Ok(Column {
                name,
                required: not_null == 1 && default.is_none(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Some(columns))
}

/// Whether `table` still holds a row with primary key `id`.
///
/// How the matrix proves a *seeded row* survived migration, rather than proving only that the
/// table still has the right number of rows in it. Opened `query_only`, so asking cannot
/// change the answer.
///
/// # Errors
///
/// Propagates the open and the query. A missing table is `Ok(false)` rather than an error:
/// after a migration chain a table can legitimately be gone, and that is an answer the caller
/// should see as "the row is not there".
pub fn id_present(path: &Path, table: &str, id: Uuid) -> Result<bool> {
    let conn = crate::connection::open_query_only(path)?;
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(false);
    }
    let found: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE id = ?1"),
        [id.to_string()],
        |row| row.get(0),
    )?;
    Ok(found > 0)
}

/// Every version this build can construct, for a matrix to iterate.
#[must_use]
pub fn every_version() -> std::ops::RangeInclusive<u32> {
    1..=crate::supported_schema_version()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{every_version, write_project_at};

    /// Reports what the plan reaches at every version.
    ///
    /// Printed rather than asserted per table, because the point is coverage a reader can
    /// see: `cargo test -p evara-db --features fixtures historical -- --nocapture`.
    #[test]
    fn the_plan_reaches_every_table_it_can_at_every_version() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        for version in every_version() {
            let path = dir.path().join(format!("v{version}.evaraproj"));
            let built =
                write_project_at(&path, version, uuid::Uuid::now_v7()).unwrap_or_else(|error| {
                    // The whole chain: `DbError::Internal` hides the driver's message behind a
                    // deliberately opaque Display, which is right for the product and useless
                    // here, so the fixture's own detail is read out of the source.
                    let mut detail = error.to_string();
                    let mut source = std::error::Error::source(&error);
                    while let Some(inner) = source {
                        let _ = writeln!(detail);
                        detail.push_str("  caused by: ");
                        detail.push_str(&inner.to_string());
                        source = std::error::Error::source(inner);
                    }
                    panic!("build a project at V{version}: {detail}");
                });

            println!(
                "V{version:<2} seeded {:>2}: {}",
                built.seeded.len(),
                built.seeded.join(", ")
            );
            for (table, reason) in &built.skipped {
                // "does not exist yet" is the ordinary case and not worth a line each.
                if !reason.contains("does not exist yet") {
                    println!("       skipped {table}: {reason}");
                }
            }
        }
    }
}
