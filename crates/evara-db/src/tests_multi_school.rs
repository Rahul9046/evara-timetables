//! Stage 0A: a project holds several schools, and nothing it already held moved.
//!
//! # What these tests are for
//!
//! `V12` is the first migration in Evara's life to **rebuild an existing table**, and the
//! table it rebuilds is the one every school-owned row hangs off. Three things therefore
//! have to be proved rather than argued:
//!
//! 1. **Nothing moves.** Every identifier, revision, timestamp and byte that an existing
//!    project held is still there afterwards, and the rebuild consumed no revision.
//! 2. **A failure changes nothing.** refinery runs the batch in one transaction; if
//!    anything in it fails, the original table comes back with its data and its triggers.
//! 3. **The point of the exercise works.** Two schools fit in one file, each owns its own
//!    year levels, and neither can reach into the other's.
//!
//! These run against the **bundled** SQLite through `evara_db`'s own layer, not against
//! whatever engine happens to be on the machine. That matters because the whole migration
//! rests on `PRAGMA defer_foreign_keys` being honoured inside a transaction where
//! `PRAGMA foreign_keys` is not, and that is an engine behaviour â€” so it is asserted here
//! rather than assumed. See
//! [ADR 0020](../../../docs/adr/0020-in-place-table-rebuild.md).

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::availability::{AvailabilityEntity, AvailabilityRuleInput, AvailabilityStatus};
use crate::repo::people::{
    GroupKind, StudentGroupInput, StudentInput, TeacherInput, YearLevelInput,
};
use crate::repo::structure::{CampusInput, SchoolInput};
use crate::repo::time_model::{CycleInput, CycleKind, MaterialisationRequest};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// The SQL under test, read from the migration itself so the two cannot drift apart.
const V12_SQL: &str = include_str!("../migrations/V12__multiple_schools.sql");

fn school_input(name: &str) -> SchoolInput {
    SchoolInput {
        name: name.to_owned(),
        code: None,
        timezone: "Europe/London".to_owned(),
        locale: "en-GB".to_owned(),
        working_days: WorkingDays::MONDAY_TO_FRIDAY,
    }
}

fn year_level(school: Uuid, name: &str, ordinal: i64) -> YearLevelInput {
    YearLevelInput {
        school_id: school,
        name: name.to_owned(),
        ordinal,
    }
}

fn teacher(school: Uuid, given: &str, family: &str) -> TeacherInput {
    TeacherInput {
        school_id: school,
        code: None,
        given_name: given.to_owned(),
        family_name: family.to_owned(),
        email: None,
        home_campus_id: None,
        fte_percent: 100,
        max_periods_per_day: None,
        max_periods_per_cycle: None,
        max_consecutive_periods: None,
        notes: None,
        is_active: true,
    }
}

// ===================================================== a project written before V12

/// Everything a V11 project could hold, written exactly as the previous build would have.
///
/// Values are fixed rather than generated so the assertions can name them: a revision of
/// 3 on the school and 2 on the campus is how "the migration consumed no revision" becomes
/// checkable rather than vacuous.
struct PreV12 {
    school: Uuid,
    campus: Uuid,
    year_level: Uuid,
    teacher: Uuid,
    student: Uuid,
    group: Uuid,
    cycle: Uuid,
    cycle_day: Uuid,
    availability: Uuid,
    logo: Uuid,
}

const STAMP: &str = "2026-10-01T09:00:00.000Z";

#[expect(
    clippy::too_many_lines,
    reason = "one fixture holding every kind of row a V11 project could have, in one place"
)]
fn write_a_pre_v12_project(path: &std::path::Path, site: Uuid) -> PreV12 {
    let ids = PreV12 {
        school: Uuid::now_v7(),
        campus: Uuid::now_v7(),
        year_level: Uuid::now_v7(),
        teacher: Uuid::now_v7(),
        student: Uuid::now_v7(),
        group: Uuid::now_v7(),
        cycle: Uuid::now_v7(),
        cycle_day: Uuid::now_v7(),
        availability: Uuid::now_v7(),
        logo: Uuid::now_v7(),
    };

    let mut conn = crate::connection::open_configured(path, site).expect("open");
    let version = crate::migrations::run_up_to(&mut conn, 11).expect("migrate to V11");
    assert_eq!(version, 11, "the fixture must stop at the previous version");

    let tx = conn.transaction().expect("transaction");
    tx.execute(
        "INSERT INTO school (id, only_one, name, timezone, locale, working_days,
                             created_at, updated_at, rev)
         VALUES (?, 1, 'Northgate Secondary', 'Europe/London', 'en-GB', 31, ?, ?, 3)",
        (ids.school.to_string(), STAMP, STAMP),
    )
    .expect("school");
    tx.execute(
        "INSERT INTO campus (id, school_id, name, code, address, created_at, updated_at, rev)
         VALUES (?, ?, 'Main', 'MAIN', '1 Example Way', ?, ?, 2)",
        (ids.campus.to_string(), ids.school.to_string(), STAMP, STAMP),
    )
    .expect("campus");
    tx.execute(
        "INSERT INTO year_level (id, school_id, name, ordinal, created_at, updated_at, rev)
         VALUES (?, ?, 'Grade 1', 1, ?, ?, 1)",
        (
            ids.year_level.to_string(),
            ids.school.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("year level");
    // Load ceilings and an FTE, because those are the columns Stage A will stop showing
    // and must therefore survive untouched.
    tx.execute(
        "INSERT INTO teacher (id, school_id, code, given_name, family_name, email,
                              home_campus_id, fte_percent, max_periods_per_day,
                              max_periods_per_cycle, max_consecutive_periods, notes,
                              is_active, created_at, updated_at, rev)
         VALUES (?, ?, 'T001', 'Ada', 'Okafor', NULL, ?, 80, 5, 40, 3, 'part time', 1, ?, ?, 4)",
        (
            ids.teacher.to_string(),
            ids.school.to_string(),
            ids.campus.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("teacher");
    tx.execute(
        "INSERT INTO student (id, school_id, code, given_name, family_name, email,
                              year_level_id, campus_id, is_active, created_at, updated_at, rev)
         VALUES (?, ?, 'S001', 'Cass', 'Pupilsen', NULL, ?, ?, 1, ?, ?, 1)",
        (
            ids.student.to_string(),
            ids.school.to_string(),
            ids.year_level.to_string(),
            ids.campus.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("student");
    tx.execute(
        "INSERT INTO student_group (id, school_id, name, code, kind, year_level_id,
                                    parent_group_id, size, campus_id,
                                    created_at, updated_at, rev)
         VALUES (?, ?, 'Grade 1 Cohort', 'G1', 'COHORT', ?, NULL, 28, ?, ?, ?, 1)",
        (
            ids.group.to_string(),
            ids.school.to_string(),
            ids.year_level.to_string(),
            ids.campus.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("group");
    tx.execute(
        "INSERT INTO student_group_member (id, student_group_id, student_id,
                                           created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, 1)",
        (
            Uuid::now_v7().to_string(),
            ids.group.to_string(),
            ids.student.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("membership");
    tx.execute(
        "INSERT INTO cycle (id, school_id, name, day_count, week_count, is_default,
                            created_at, updated_at, rev, kind)
         VALUES (?, ?, 'Ten Day', 10, 2, 1, ?, ?, 1, 'CUSTOM')",
        (ids.cycle.to_string(), ids.school.to_string(), STAMP, STAMP),
    )
    .expect("cycle");
    tx.execute(
        "INSERT INTO cycle_day (id, cycle_id, ordinal, label, weekday_hint,
                                created_at, updated_at, rev)
         VALUES (?, ?, 1, 'Day 1', NULL, ?, ?, 1)",
        (
            ids.cycle_day.to_string(),
            ids.cycle.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("cycle day");
    tx.execute(
        "INSERT INTO availability (id, school_id, entity_kind, entity_id, timeslot_id,
                                   cycle_day_id, period_id, term_id, status, weight, reason,
                                   effective_from, effective_to, created_at, updated_at, rev)
         VALUES (?, ?, 'TEACHER', ?, NULL, ?, NULL, NULL, 'UNAVAILABLE', NULL,
                 'school governor duties', NULL, NULL, ?, ?, 1)",
        (
            ids.availability.to_string(),
            ids.school.to_string(),
            ids.teacher.to_string(),
            ids.cycle_day.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("availability");
    // A logo, because binary data through a table rebuild is its own risk: an eight-byte
    // PNG header is enough to prove the BLOB arrived byte for byte.
    tx.execute(
        "INSERT INTO school_logo (id, school_id, media_type, bytes, width, height,
                                   created_at, updated_at, rev)
         VALUES (?, ?, 'image/png', x'89504e470d0a1a0a', 16, 16, ?, ?, 1)",
        (ids.logo.to_string(), ids.school.to_string(), STAMP, STAMP),
    )
    .expect("logo");
    tx.commit().expect("commit fixture");

    ids
}

/// The migration path every existing project will actually take.
///
/// A released migration is immutable, so a `V12` that renumbered an identifier, consumed a
/// revision, rewrote a timestamp or dropped a byte would be unfixable in place. This is
/// the test that makes that claim checkable.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the point of this test is that EVERYTHING survived, asserted in one place"
)]
fn a_project_written_before_v12_migrates_without_touching_what_it_had() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000012").expect("valid uuid");

    let ids = write_a_pre_v12_project(&path, site);

    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("open project");
    assert_eq!(db.schema_version(), crate::supported_schema_version());

    // --- the school itself -----------------------------------------------------------
    let schools = db.structure().schools().expect("read schools");
    assert_eq!(schools.len(), 1, "one school in, one school out");
    let kept = &schools[0];
    assert_eq!(kept.id, ids.school, "its identifier is the one it had");
    assert_eq!(kept.name, "Northgate Secondary");
    assert_eq!(kept.code, None, "a school that never had a code has none");
    assert_eq!(kept.timezone, "Europe/London");
    assert_eq!(kept.locale, "en-GB");
    assert_eq!(
        kept.working_days,
        WorkingDays::MONDAY_TO_FRIDAY,
        "V7's column survived the rebuild"
    );
    assert_eq!(kept.rev, 3, "the rebuild consumed no revision");
    assert_eq!(kept.created_at, STAMP);
    assert_eq!(kept.updated_at, STAMP, "and rewrote no timestamp");

    // --- everything that references it ------------------------------------------------
    let campuses = db.structure().campuses(ids.school).expect("campuses");
    assert_eq!(campuses.len(), 1);
    assert_eq!(campuses[0].id, ids.campus);
    assert_eq!(campuses[0].rev, 2, "a child's revision is untouched too");
    assert_eq!(campuses[0].address.as_deref(), Some("1 Example Way"));

    let year_levels = db.people().year_levels(ids.school).expect("year levels");
    assert_eq!(year_levels.len(), 1);
    assert_eq!(year_levels[0].id, ids.year_level);
    assert_eq!(year_levels[0].name, "Grade 1");

    let teachers = db.people().teachers(ids.school).expect("teachers");
    assert_eq!(teachers.len(), 1);
    let kept_teacher = &teachers[0];
    assert_eq!(kept_teacher.id, ids.teacher);
    assert_eq!(kept_teacher.rev, 4);
    // The four fields Stage A will remove from the form, still exactly as stored.
    assert_eq!(kept_teacher.fte_percent, 80);
    assert_eq!(kept_teacher.max_periods_per_day, Some(5));
    assert_eq!(kept_teacher.max_periods_per_cycle, Some(40));
    assert_eq!(kept_teacher.max_consecutive_periods, Some(3));
    assert_eq!(kept_teacher.home_campus_id, Some(ids.campus));

    let students = db.people().students(ids.school).expect("students");
    assert_eq!(students.len(), 1);
    assert_eq!(students[0].id, ids.student);
    assert_eq!(students[0].year_level_id, Some(ids.year_level));

    let groups = db.people().student_groups(ids.school).expect("groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, ids.group);
    assert_eq!(groups[0].size, Some(28));
    assert_eq!(
        db.people()
            .student_group_member_count(ids.group)
            .expect("members"),
        1,
        "membership is history and must not be touched by a schema change"
    );

    let cycles = db.time_model().cycles(ids.school).expect("cycles");
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].id, ids.cycle);
    assert_eq!(cycles[0].kind, CycleKind::Custom);

    // --- availability keeps its meaning, not merely its row --------------------------
    let rules = db
        .availability()
        .rules_for(ids.school, AvailabilityEntity::Teacher, ids.teacher)
        .expect("rules");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, ids.availability);
    assert_eq!(
        rules[0].cycle_day_id,
        Some(ids.cycle_day),
        "a cycle-based rule still names the same cycle day; nothing was reinterpreted"
    );
    assert_eq!(rules[0].status, AvailabilityStatus::Unavailable);
    assert_eq!(rules[0].reason.as_deref(), Some("school governor duties"));

    // --- binary data ------------------------------------------------------------------
    let logo = db
        .structure()
        .school_logo(ids.school)
        .expect("read logo")
        .expect("the logo survived");
    assert_eq!(logo.id, ids.logo);
    assert_eq!(
        logo.bytes,
        vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]
    );
    assert_eq!((logo.width, logo.height), (16, 16));

    // --- the database's own opinion of itself -----------------------------------------
    assert_eq!(orphan_references(&db), 0, "PRAGMA foreign_key_check");
    assert_eq!(integrity(&db), "ok", "PRAGMA integrity_check");
    assert_eq!(
        school_changelog_triggers(&db),
        3,
        "the rebuild dropped the school's change-log triggers and install_all put them back"
    );

    // --- and it all survives a close and reopen ---------------------------------------
    drop(db);
    let mut reopened = Database::open_existing(&path, APP_VERSION, site).expect("reopen");
    assert_eq!(
        reopened.structure().schools().expect("read")[0].id,
        ids.school
    );
    assert_eq!(
        reopened.safety_backup(),
        None,
        "reopening an already-migrated project takes no further copy"
    );
}

/// The rebuild emits no entity-change events of its own.
///
/// A schema change is not an edit to a school, so the change feed must not report one.
/// `DROP TABLE`'s implicit delete does not fire triggers and the replacement table has
/// none yet when its rows are restored, which together produce exactly this.
#[test]
fn the_rebuild_adds_nothing_to_the_change_feed() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000013").expect("valid uuid");

    let ids = write_a_pre_v12_project(&path, site);
    let db = Database::open_existing(&path, APP_VERSION, site).expect("open project");

    let entries: i64 = db
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM change_log WHERE entity_table = 'school'",
                [],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("read feed");
    assert_eq!(
        entries, 0,
        "the fixture wrote its school with raw SQL before any trigger existed, and the \
         rebuild must not invent an INSERT or a DELETE for it"
    );

    // But ordinary writes after the migration do reach the feed, which is what proves the
    // triggers are not merely present but working.
    let mut db = db;
    db.structure()
        .update_school(
            ids.school,
            Expected::Any,
            &school_input("Northgate Academy"),
        )
        .expect("rename the school");
    let after: i64 = db
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM change_log WHERE entity_table = 'school' AND op = 'UPDATE'",
                [],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("read feed");
    assert_eq!(after, 1, "a real edit is recorded");
}

// ========================================================================= rollback

/// A failure anywhere in the batch leaves the project exactly as it was.
///
/// The migration's own SQL is replayed here, read from the file with `include_str!` so the
/// two cannot drift, and then deliberately followed by a statement that fails. What has to
/// survive is not just the data but the *old table*: its `only_one` column, its rows, and
/// its change-log triggers.
#[test]
fn a_failure_after_the_rebuild_rolls_the_whole_batch_back() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000014").expect("valid uuid");

    // A project at V11, opened without migrating past it, so the rebuild has not run yet.
    let ids = write_a_pre_v12_project(&path, site);
    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    crate::change_log::install_all(&conn.transaction().expect("tx")).ok();

    let before_triggers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
             AND name LIKE 'trg_school_changelog%'",
            [],
            |row| row.get(0),
        )
        .expect("count triggers");

    // The batch: the real V12, then something that cannot succeed.
    let tx = conn.transaction().expect("transaction");
    tx.execute_batch(V12_SQL)
        .expect("the rebuild itself applies");
    let failure = tx.execute_batch(
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES (NULL, NULL, NULL, NULL, NULL, NULL, NULL);",
    );
    assert!(
        failure.is_err(),
        "the deliberate failure must actually fail"
    );
    drop(tx); // No commit: this is what refinery does with a failed batch.

    // The old table is back, with everything in it.
    let only_one: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('school') WHERE name = 'only_one'",
            [],
            |row| row.get(0),
        )
        .expect("inspect school");
    assert_eq!(only_one, 1, "the rolled-back table is the original one");

    let schools: i64 = conn
        .query_row("SELECT COUNT(*) FROM school", [], |row| row.get(0))
        .expect("count schools");
    assert_eq!(schools, 1);
    let name: String = conn
        .query_row(
            "SELECT name FROM school WHERE id = ?",
            [ids.school.to_string()],
            |row| row.get(0),
        )
        .expect("the school is still there");
    assert_eq!(name, "Northgate Secondary");

    let leftovers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'school_rebuild'",
            [],
            |row| row.get(0),
        )
        .expect("look for the holding table");
    assert_eq!(leftovers, 0, "the holding table went with the rollback");

    let after_triggers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
             AND name LIKE 'trg_school_changelog%'",
            [],
            |row| row.get(0),
        )
        .expect("count triggers");
    assert_eq!(
        after_triggers, before_triggers,
        "the rollback restored the triggers that went with the dropped table"
    );

    let orphans: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign_key_check");
    assert_eq!(orphans, 0);

    // And the project still opens and migrates properly afterwards.
    drop(conn);
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("reopen and migrate");
    assert_eq!(db.schema_version(), crate::supported_schema_version());
    assert_eq!(db.structure().schools().expect("read").len(), 1);
}

/// Deferred foreign keys are deferred, not disabled.
///
/// The safety of the whole migration rests on this. If `defer_foreign_keys` merely
/// suppressed enforcement, a rebuild that failed to restore its rows would commit and
/// leave every campus, teacher and timeslot in the project pointing at a school that no
/// longer exists. Asserted on the engine Evara actually ships.
#[test]
fn a_rebuild_that_loses_its_rows_cannot_commit() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000015").expect("valid uuid");

    write_a_pre_v12_project(&path, site);
    let mut conn = crate::connection::open_configured(&path, site).expect("open");

    let tx = conn.transaction().expect("transaction");
    // `foreign_keys` cannot be turned off in here -- the premise ADR 0013 recorded.
    tx.execute_batch("PRAGMA foreign_keys = OFF")
        .expect("pragma");
    let still_on: i64 = tx
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read pragma");
    assert_eq!(
        still_on, 1,
        "foreign_keys is a no-op inside a transaction, which is why the twelve-step \
         rebuild is unavailable to a refinery migration"
    );

    // ...but `defer_foreign_keys` is honoured.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")
        .expect("pragma");
    let deferred: i64 = tx
        .query_row("PRAGMA defer_foreign_keys", [], |row| row.get(0))
        .expect("read pragma");
    assert_eq!(deferred, 1);

    // Drop and recreate, and deliberately do not restore the rows.
    tx.execute_batch(
        "DROP TABLE school;
         CREATE TABLE school (
             id TEXT PRIMARY KEY NOT NULL,
             name TEXT NOT NULL,
             timezone TEXT NOT NULL,
             locale TEXT NOT NULL,
             working_days INTEGER NOT NULL DEFAULT 31,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             rev INTEGER NOT NULL DEFAULT 1
         ) STRICT;",
    )
    .expect("the statements themselves succeed; enforcement is deferred");

    assert!(
        tx.commit().is_err(),
        "the commit must be refused: every school-owned row would otherwise be orphaned"
    );

    // And the project is untouched.
    drop(conn);
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("open project");
    assert_eq!(db.structure().schools().expect("read").len(), 1);
    assert_eq!(orphan_references(&db), 0);
}

// =========================================================================== backup

/// Upgrading a project leaves a verified copy of it beside the original.
#[test]
fn upgrading_an_existing_project_leaves_a_usable_copy_of_it() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000016").expect("valid uuid");

    let ids = write_a_pre_v12_project(&path, site);
    let db = Database::open_existing(&path, APP_VERSION, site).expect("open project");

    let backup = db
        .safety_backup()
        .expect("upgrading past the rebuild takes a copy")
        .to_path_buf();
    assert_eq!(
        backup.file_name().map(std::ffi::OsStr::to_string_lossy),
        Some(format!("school.pre-v11.{PROJECT_EXTENSION}").into()),
        "the name says which version it was taken from"
    );
    assert!(backup.is_file(), "and the file is actually there");
    drop(db);

    // The copy is a working project at the *old* schema, holding the old data.
    let copy = crate::connection::open_plain(&backup).expect("the copy opens");
    let integrity: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity");
    assert_eq!(integrity, "ok");
    let only_one: i64 = copy
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('school') WHERE name = 'only_one'",
            [],
            |row| row.get(0),
        )
        .expect("inspect");
    assert_eq!(
        only_one, 1,
        "it is a snapshot of the project before the rebuild"
    );
    let kept: String = copy
        .query_row(
            "SELECT name FROM school WHERE id = ?",
            [ids.school.to_string()],
            |row| row.get(0),
        )
        .expect("the school is in the copy");
    assert_eq!(kept, "Northgate Secondary");
    let teachers: i64 = copy
        .query_row("SELECT COUNT(*) FROM teacher", [], |row| row.get(0))
        .expect("count");
    assert_eq!(teachers, 1, "and so is everything it owned");
    let orphans: i64 = copy
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign_key_check");
    assert_eq!(orphans, 0);
}

/// A new project needs no copy, because there is nothing yet to protect.
#[test]
fn creating_a_project_takes_no_backup() {
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    let path = dir.path().join(format!("fresh.{PROJECT_EXTENSION}"));

    let summary = workspace.create_project(&path).expect("create");
    assert_eq!(summary.safety_backup, None);

    let siblings = std::fs::read_dir(dir.path())
        .expect("read dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v"))
        .count();
    assert_eq!(siblings, 0, "no copy is written beside a brand-new project");
}

/// An existing copy is never overwritten; the next name is used instead.
#[test]
fn an_existing_backup_is_left_alone() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000017").expect("valid uuid");

    write_a_pre_v12_project(&path, site);

    // Something is already sitting at the name the backup would take -- the evidence of an
    // earlier attempt, which must survive.
    let taken = dir
        .path()
        .join(format!("school.pre-v11.{PROJECT_EXTENSION}"));
    std::fs::write(&taken, b"an earlier attempt's copy").expect("write placeholder");

    let db = Database::open_existing(&path, APP_VERSION, site).expect("open project");
    let backup = db.safety_backup().expect("a copy was still taken");
    assert_eq!(
        backup.file_name().map(std::ffi::OsStr::to_string_lossy),
        Some(format!("school.pre-v11-2.{PROJECT_EXTENSION}").into()),
        "the second attempt gets the next name"
    );
    assert_eq!(
        std::fs::read(&taken).expect("read placeholder"),
        b"an earlier attempt's copy",
        "and the first copy is untouched"
    );
}

// ================================================================= two schools, apart

/// A fixture with two schools whose names collide everywhere they are allowed to.
struct TwoSchools {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
}

impl TwoSchools {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let settings = dir.path().join("settings.sqlite");
        let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
        let path = dir.path().join(format!("trust.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");

        let mut this = Self {
            _dir: dir,
            workspace,
            north: Uuid::nil(),
            south: Uuid::nil(),
        };
        this.north = this
            .db()
            .structure()
            .create_school(&school_input("Northgate Secondary"))
            .expect("first school")
            .id;
        this.south = this
            .db()
            .structure()
            .create_school(&school_input("Southmoor College"))
            .expect("second school")
            .id;
        this
    }

    fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }
}

/// The requirement the whole stage exists for: the same year level in two schools.
#[test]
fn each_school_owns_its_own_year_levels_under_the_same_names() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let north_grade_1 = project
        .db()
        .people()
        .create_year_level(&year_level(north, "Grade 1", 1))
        .expect("Grade 1 at Northgate");
    let south_grade_1 = project
        .db()
        .people()
        .create_year_level(&year_level(south, "Grade 1", 1))
        .expect("Grade 1 at Southmoor: same name, same ordinal, different school");

    assert_ne!(
        north_grade_1.id, south_grade_1.id,
        "they are two records, not one shared one"
    );

    // ...and a duplicate *within* one school is still refused, which is the half of
    // `UNIQUE (school_id, name)` that must keep working.
    let duplicate = project
        .db()
        .people()
        .create_year_level(&year_level(north, "Grade 1", 9));
    assert!(
        matches!(duplicate, Err(DbError::DuplicateValue { .. })),
        "one school cannot hold two Grade 1s: {duplicate:?}"
    );
    let duplicate_ordinal =
        project
            .db()
            .people()
            .create_year_level(&year_level(north, "Reception", 1));
    assert!(
        matches!(duplicate_ordinal, Err(DbError::DuplicateValue { .. })),
        "nor two year levels at position 1: {duplicate_ordinal:?}"
    );

    let listed_north = project.db().people().year_levels(north).expect("north");
    assert_eq!(listed_north.len(), 1);
    assert_eq!(listed_north[0].id, north_grade_1.id);
    let listed_south = project.db().people().year_levels(south).expect("south");
    assert_eq!(listed_south.len(), 1);
    assert_eq!(listed_south[0].id, south_grade_1.id);
}

/// Every scoped list returns one school's rows and no more.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "every scoped listing in one test, because a missed one is the whole risk"
)]
fn no_listing_leaks_one_schools_records_into_the_others() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    for school in [north, south] {
        // Deliberately identical names and codes in both schools, because that is the case
        // a missing `WHERE school_id` would quietly double.
        project
            .db()
            .structure()
            .create_campus(&CampusInput {
                school_id: school,
                name: "Main Campus".to_owned(),
                code: "MAIN".to_owned(),
                address: None,
            })
            .expect("a campus called Main in each school");
        project
            .db()
            .people()
            .create_year_level(&year_level(school, "Grade 8", 8))
            .expect("Grade 8 in each school");
        project
            .db()
            .people()
            .create_teacher(&teacher(school, "Ada", "Okafor"))
            .expect("a teacher with the same name in each school");
        project
            .db()
            .people()
            .create_student(&StudentInput {
                school_id: school,
                code: None,
                given_name: "Cass".to_owned(),
                family_name: "Pupilsen".to_owned(),
                email: None,
                year_level_id: None,
                campus_id: None,
                is_active: true,
            })
            .expect("a student with the same name in each school");
        project
            .db()
            .people()
            .create_student_group(&StudentGroupInput {
                school_id: school,
                name: "Grade 8 Cohort".to_owned(),
                code: None,
                kind: GroupKind::Cohort,
                year_level_id: None,
                parent_group_id: None,
                size: Some(30),
                campus_id: None,
            })
            .expect("a group with the same name in each school");
        project
            .db()
            .time_model()
            .create_cycle(&CycleInput {
                school_id: school,
                name: "Two Week".to_owned(),
                day_count: 10,
                week_count: 2,
                is_default: true,
                kind: CycleKind::Custom,
            })
            .expect("a cycle with the same name in each school");
    }

    for school in [north, south] {
        assert_eq!(
            project
                .db()
                .structure()
                .campuses(school)
                .expect("campuses")
                .len(),
            1,
            "each school sees one Main Campus, not two"
        );
        assert_eq!(
            project
                .db()
                .people()
                .year_levels(school)
                .expect("years")
                .len(),
            1
        );
        assert_eq!(
            project
                .db()
                .people()
                .teachers(school)
                .expect("teachers")
                .len(),
            1
        );
        assert_eq!(
            project
                .db()
                .people()
                .students(school)
                .expect("students")
                .len(),
            1
        );
        assert_eq!(
            project
                .db()
                .people()
                .student_groups(school)
                .expect("groups")
                .len(),
            1
        );
        assert_eq!(
            project
                .db()
                .time_model()
                .cycles(school)
                .expect("cycles")
                .len(),
            1
        );
    }

    // Every row really is in the project; the lists are scoped, not short.
    let total: i64 = project
        .db()
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM campus", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("count");
    assert_eq!(
        total, 2,
        "both campuses exist; each school sees only its own"
    );
}

/// An identifier from another school is not authority to edit it.
#[test]
fn an_update_naming_another_schools_record_is_refused() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let northern = project
        .db()
        .people()
        .create_year_level(&year_level(north, "Grade 8", 8))
        .expect("create");

    // A screen scoped to Southmoor, handed Northgate's identifier.
    let attempt = project.db().people().update_year_level(
        northern.id,
        Expected::Any,
        &year_level(south, "Renamed", 8),
    );
    assert!(
        matches!(attempt, Err(DbError::WrongSchool { .. })),
        "expected WrongSchool, got {attempt:?}"
    );

    let unchanged = project
        .db()
        .people()
        .year_level(northern.id)
        .expect("read")
        .expect("still there");
    assert_eq!(unchanged.name, "Grade 8", "and nothing was written");
    assert_eq!(unchanged.rev, 1, "no revision was consumed");

    // The same guard on a campus, a teacher and a cycle.
    let campus = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: north,
            name: "Main".to_owned(),
            code: "MAIN".to_owned(),
            address: None,
        })
        .expect("create");
    let moved = project.db().structure().update_campus(
        campus.id,
        Expected::Any,
        &CampusInput {
            school_id: south,
            name: "Main".to_owned(),
            code: "MAIN".to_owned(),
            address: None,
        },
    );
    assert!(
        matches!(moved, Err(DbError::WrongSchool { .. })),
        "{moved:?}"
    );

    let staff = project
        .db()
        .people()
        .create_teacher(&teacher(north, "Ada", "Okafor"))
        .expect("create");
    let poached = project.db().people().update_teacher(
        staff.id,
        Expected::Any,
        &teacher(south, "Ada", "Okafor"),
    );
    assert!(
        matches!(poached, Err(DbError::WrongSchool { .. })),
        "a teacher belongs to one school in Stage 0A, and an update cannot move them: \
         {poached:?}"
    );
}

/// Cross-school references are refused by the schema, not merely by the interface.
#[test]
fn a_record_cannot_reference_another_schools_structures() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let southern_year = project
        .db()
        .people()
        .create_year_level(&year_level(south, "Grade 8", 8))
        .expect("create");
    let southern_campus = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: south,
            name: "North Campus".to_owned(),
            code: "NTH".to_owned(),
            address: None,
        })
        .expect("create");

    let wrong_year = project.db().people().create_student(&StudentInput {
        school_id: north,
        code: None,
        given_name: "Cass".to_owned(),
        family_name: "Pupilsen".to_owned(),
        email: None,
        year_level_id: Some(southern_year.id),
        campus_id: None,
        is_active: true,
    });
    assert!(
        wrong_year.is_err(),
        "a Northgate student cannot sit in Southmoor's Grade 8"
    );

    let wrong_campus = project.db().people().create_teacher(&TeacherInput {
        home_campus_id: Some(southern_campus.id),
        ..teacher(north, "Ada", "Okafor")
    });
    assert!(
        wrong_campus.is_err(),
        "a Northgate teacher cannot be based on a Southmoor campus"
    );

    let wrong_group_year = project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            school_id: north,
            name: "Grade 8 Cohort".to_owned(),
            code: None,
            kind: GroupKind::Cohort,
            year_level_id: Some(southern_year.id),
            parent_group_id: None,
            size: None,
            campus_id: None,
        });
    assert!(
        wrong_group_year.is_err(),
        "nor can a Northgate group be a Southmoor year's cohort"
    );
}

/// An availability rule cannot be recorded against another school's cycle.
#[test]
fn availability_cannot_name_another_schools_time() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let southern_cycle = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: south,
            name: "Two Week".to_owned(),
            day_count: 10,
            week_count: 2,
            is_default: true,
            kind: CycleKind::Custom,
        })
        .expect("cycle");
    // Creating a cycle materialises its days, so this reads one rather than adding a
    // second Day 1.
    let southern_day = project
        .db()
        .time_model()
        .cycle_days(southern_cycle.id)
        .expect("read cycle days")
        .into_iter()
        .next()
        .expect("a cycle has days");
    let northern_teacher = project
        .db()
        .people()
        .create_teacher(&teacher(north, "Ada", "Okafor"))
        .expect("teacher");

    let attempt = project.db().availability().set(&AvailabilityRuleInput {
        school_id: north,
        entity_kind: AvailabilityEntity::Teacher,
        entity_id: northern_teacher.id,
        timeslot_id: None,
        cycle_day_id: Some(southern_day.id),
        period_id: None,
        term_id: None,
        status: AvailabilityStatus::Unavailable,
        weight: None,
        reason: None,
        effective_from: None,
        effective_to: None,
    });
    assert!(
        attempt.is_err(),
        "a rule about Southmoor's Day 1 is not a rule about anything at Northgate"
    );
}

// ======================================================================== deletion

/// A school that still owns anything cannot be deleted, and says what is in the way.
#[test]
fn a_school_with_dependents_cannot_be_deleted_and_reports_why() {
    let mut project = TwoSchools::new();
    let north = project.north;

    project
        .db()
        .people()
        .create_year_level(&year_level(north, "Grade 8", 8))
        .expect("year level");
    project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: north,
            name: "Main".to_owned(),
            code: "MAIN".to_owned(),
            address: None,
        })
        .expect("campus");

    let dependents = project
        .db()
        .structure()
        .school_dependents(north)
        .expect("count dependents");
    assert_eq!(dependents.campuses, 1);
    assert_eq!(dependents.year_levels, 1);
    assert_eq!(dependents.total, 2, "and the total is the sum");

    let refused = project.db().structure().delete_school(north, Expected::Any);
    assert!(
        matches!(refused, Err(DbError::StillReferenced { .. })),
        "expected StillReferenced, got {refused:?}"
    );
    assert_eq!(
        project.db().structure().schools().expect("read").len(),
        2,
        "nothing was deleted"
    );
}

/// An empty school can be deleted, and takes only its own logo with it.
#[test]
fn an_empty_school_can_be_deleted() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    // The one thing a delete is allowed to remove: the school's own picture of itself.
    // Written as a row rather than through `set_school_logo`, so the assertion below
    // cannot pass merely because no logo was ever created.
    give_a_logo(&mut project, south);

    project
        .db()
        .structure()
        .delete_school(south, Expected::Any)
        .expect("an empty school can go");

    let left = project.db().structure().schools().expect("read");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, north, "the other school is untouched");

    let logos: i64 = project
        .db()
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM school_logo", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("count logos");
    assert_eq!(logos, 0, "its logo went with it rather than blocking it");

    assert!(
        matches!(
            project.db().structure().delete_school(south, Expected::Any),
            Err(DbError::NotFound { .. })
        ),
        "deleting it twice is a NotFound, not a silent success"
    );
}

// =============================================================== shared facilities

/// A room shared between campuses of *different* schools keeps working.
///
/// This is the regression test for the obvious way to get multi-school wrong: scoping
/// facilities by `home campus -> school` and thereby breaking ADR 0013 and ADR 0014. One
/// physical room, one occupancy record, two institutions timetabling into it.
#[test]
fn a_room_can_still_be_shared_across_campuses_of_different_schools() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let northern = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: north,
            name: "Riverside".to_owned(),
            code: "RIV".to_owned(),
            address: None,
        })
        .expect("campus");
    let southern = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: south,
            name: "Riverside".to_owned(),
            code: "RIV".to_owned(),
            address: None,
        })
        .expect("campus");

    let hall = project
        .db()
        .structure()
        .create_room(
            &crate::repo::structure::RoomInput {
                campus_id: northern.id,
                building_id: None,
                room_type_id: None,
                name: "Sports Hall".to_owned(),
                code: "HALL".to_owned(),
                capacity: 200,
                is_bookable: true,
                notes: None,
            },
            &[southern.id],
        )
        .expect("one hall, shared with the other school's campus at creation");

    let campuses = project
        .db()
        .structure()
        .room_campuses(hall.id)
        .expect("read associations");
    assert_eq!(
        campuses.len(),
        2,
        "the association set spans both schools, as ADR 0014 intends"
    );
    assert!(campuses.contains(&northern.id));
    assert!(campuses.contains(&southern.id));

    // Both schools can see it through their own campus, and it is the SAME room: one
    // identifier, therefore one occupancy record for the future solver to reason about.
    let seen_by_north = project.db().structure().rooms(northern.id).expect("rooms");
    let seen_by_south = project.db().structure().rooms(southern.id).expect("rooms");
    assert_eq!(seen_by_north.len(), 1);
    assert_eq!(seen_by_south.len(), 1);
    assert_eq!(
        seen_by_north[0].room.id, seen_by_south[0].room.id,
        "a shared room is one room, not a copy per school"
    );
}

// ========================================================================== helpers

fn orphan_references(db: &Database) -> i64 {
    db.read(|conn| {
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(DbError::from)
    })
    .expect("foreign_key_check")
}

fn integrity(db: &Database) -> String {
    db.read(|conn| {
        conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(DbError::from)
    })
    .expect("integrity_check")
}

fn school_changelog_triggers(db: &Database) -> i64 {
    db.read(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
             AND name LIKE 'trg_school_changelog%'",
            [],
            |row| row.get(0),
        )
        .map_err(DbError::from)
    })
    .expect("count triggers")
}

// ================================================== deletion cannot cross a school

/// One school, filled in enough to delete things out of.
///
/// Every kind of destructive path is represented: a row with its own `school_id`, a child
/// whose school is only reachable through a parent, a membership, an availability rule and
/// a logo.
struct Filled {
    campus: Uuid,
    room_type: Uuid,
    year_level: Uuid,
    teacher: Uuid,
    student: Uuid,
    group: Uuid,
    academic_year: Uuid,
    term: Uuid,
    cycle: Uuid,
    cycle_day: Uuid,
    bells: Uuid,
    period: Uuid,
}

#[expect(
    clippy::too_many_lines,
    reason = "one school filled with every kind of row a deletion check has to cover"
)]
fn fill(project: &mut TwoSchools, school: Uuid, tag: &str) -> Filled {
    let campus = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: school,
            name: format!("{tag} Campus"),
            code: format!("{tag}C"),
            address: None,
        })
        .expect("campus")
        .id;
    let room_type = project
        .db()
        .structure()
        .create_room_type(&crate::repo::structure::RoomTypeInput {
            school_id: school,
            name: format!("{tag} Lab"),
            code: format!("{tag}LAB"),
        })
        .expect("room type")
        .id;
    let year_level = project
        .db()
        .people()
        .create_year_level(&year_level(school, "Grade 8", 8))
        .expect("year level")
        .id;
    let teacher_id = project
        .db()
        .people()
        .create_teacher(&teacher(school, "Ada", "Okafor"))
        .expect("teacher")
        .id;
    let student = project
        .db()
        .people()
        .create_student(&StudentInput {
            school_id: school,
            code: None,
            given_name: "Cass".to_owned(),
            family_name: "Pupilsen".to_owned(),
            email: None,
            year_level_id: None,
            campus_id: None,
            is_active: true,
        })
        .expect("student")
        .id;
    let group = project
        .db()
        .people()
        .create_student_group(&StudentGroupInput {
            school_id: school,
            name: format!("{tag} Cohort"),
            code: None,
            kind: GroupKind::Cohort,
            year_level_id: None,
            parent_group_id: None,
            size: None,
            campus_id: None,
        })
        .expect("group")
        .id;
    project
        .db()
        .people()
        .add_student_group_member(&crate::repo::people::StudentGroupMemberInput {
            student_group_id: group,
            student_id: student,
        })
        .expect("membership");
    let academic_year = project
        .db()
        .structure()
        .create_academic_year(&crate::repo::structure::AcademicYearInput {
            school_id: school,
            name: format!("{tag} 2027"),
            starts_on: "2027-01-28".to_owned(),
            ends_on: "2027-12-10".to_owned(),
        })
        .expect("academic year")
        .id;
    let term = project
        .db()
        .structure()
        .create_term(&crate::repo::structure::TermInput {
            academic_year_id: academic_year,
            name: "Term 1".to_owned(),
            ordinal: 1,
            starts_on: "2027-01-28".to_owned(),
            ends_on: "2027-04-02".to_owned(),
        })
        .expect("term")
        .id;
    let cycle = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: school,
            name: format!("{tag} Cycle"),
            day_count: 4,
            week_count: 1,
            is_default: true,
            kind: CycleKind::Custom,
        })
        .expect("cycle")
        .id;
    let cycle_day = project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("days")
        .into_iter()
        .next()
        .expect("a cycle has days")
        .id;
    let bells = project
        .db()
        .time_model()
        .create_period_structure(&crate::repo::time_model::PeriodStructureInput {
            school_id: school,
            campus_id: None,
            name: format!("{tag} Day"),
            is_default: true,
        })
        .expect("bell schedule")
        .id;
    let period = project
        .db()
        .time_model()
        .create_period(&crate::repo::time_model::PeriodInput {
            period_structure_id: bells,
            ordinal: 1,
            label: "Period 1".to_owned(),
            starts_at: "08:45".to_owned(),
            ends_at: "09:30".to_owned(),
            kind: crate::repo::time_model::PeriodKind::Teaching,
            counts_as_load: true,
        })
        .expect("period")
        .id;

    Filled {
        campus,
        room_type,
        year_level,
        teacher: teacher_id,
        student,
        group,
        academic_year,
        term,
        cycle,
        cycle_day,
        bells,
        period,
    }
}

/// Every destructive path, asked to delete the other school's row.
///
/// This is the whole of the follow-up to Stage 0A in one test. An identifier was never
/// authority to delete anything once a project held two schools: a screen scoped to
/// Northgate, handed a Southmoor identifier, has to be refused — and refused *before*
/// anything goes, which the row counts at the end are what actually prove.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table of every destructive path in one place; a missed row is the risk"
)]
fn no_deletion_reaches_into_another_school() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);
    let southern = fill(&mut project, south, "South");

    let before = row_counts(&mut project);

    // Each of these is Northgate asking to delete one of Southmoor's rows.
    let refusals: Vec<(&str, DbError)> = vec![
        (
            "campus",
            project
                .db()
                .structure()
                .delete_campus(north, southern.campus, Expected::Any)
                .expect_err("campus"),
        ),
        (
            "room type",
            project
                .db()
                .structure()
                .delete_room_type(north, southern.room_type, Expected::Any)
                .expect_err("room type"),
        ),
        (
            "academic year",
            project
                .db()
                .structure()
                .delete_academic_year(north, southern.academic_year, Expected::Any)
                .expect_err("academic year"),
        ),
        (
            // Derived: a term's school is reachable only through its academic year.
            "term",
            project
                .db()
                .structure()
                .delete_term(north, southern.term, Expected::Any)
                .expect_err("term"),
        ),
        (
            "cycle",
            project
                .db()
                .time_model()
                .delete_cycle(north, southern.cycle, Expected::Any)
                .expect_err("cycle"),
        ),
        (
            // Derived: through its cycle.
            "cycle day",
            project
                .db()
                .time_model()
                .delete_cycle_day(north, southern.cycle_day, Expected::Any)
                .expect_err("cycle day"),
        ),
        (
            "bell schedule",
            project
                .db()
                .time_model()
                .delete_period_structure(north, southern.bells, Expected::Any)
                .expect_err("bell schedule"),
        ),
        (
            // Derived: through its bell schedule.
            "period",
            project
                .db()
                .time_model()
                .delete_period(north, southern.period, Expected::Any)
                .expect_err("period"),
        ),
        (
            "year level",
            project
                .db()
                .people()
                .delete_year_level(north, southern.year_level, Expected::Any)
                .expect_err("year level"),
        ),
        (
            "teacher",
            project
                .db()
                .people()
                .delete_teacher(north, southern.teacher, Expected::Any)
                .expect_err("teacher"),
        ),
        (
            "student",
            project
                .db()
                .people()
                .delete_student(north, southern.student, Expected::Any)
                .expect_err("student"),
        ),
        (
            "student group",
            project
                .db()
                .people()
                .delete_student_group(north, southern.group, Expected::Any)
                .expect_err("student group"),
        ),
        (
            // Derived: a membership has no school of its own; its group has.
            "group membership",
            project
                .db()
                .people()
                .remove_student_group_member(north, southern.group, southern.student)
                .expect_err("membership"),
        ),
    ];

    for (what, error) in &refusals {
        assert!(
            matches!(error, DbError::WrongSchool { .. }),
            "deleting another school's {what} must be WrongSchool, got {error:?}"
        );
    }
    assert_eq!(refusals.len(), 13, "every destructive path is covered");

    // Nothing went. This is the assertion that matters: a check that refused *after*
    // clearing an availability rule or a membership would still read as a refusal.
    assert_eq!(
        row_counts(&mut project),
        before,
        "a refused deletion must change no row anywhere"
    );
    assert_eq!(orphan_references_of(&mut project), 0);

    // And the owning school can still delete its own, which is what makes the refusals
    // above a boundary rather than a wall.
    project
        .db()
        .people()
        .remove_student_group_member(south, southern.group, southern.student)
        .expect("Southmoor may remove its own membership");
    project
        .db()
        .people()
        .delete_student(south, southern.student, Expected::Any)
        .expect("Southmoor may delete its own student");
    project
        .db()
        .time_model()
        .delete_period(south, southern.period, Expected::Any)
        .expect("Southmoor may delete its own period");
    project
        .db()
        .structure()
        .delete_term(south, southern.term, Expected::Any)
        .expect("Southmoor may delete its own term");
}

/// An availability rule cannot be cleared from another school's context.
#[test]
fn clearing_availability_cannot_cross_a_school() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);
    let southern = fill(&mut project, south, "South");

    let scope = AvailabilityRuleInput {
        school_id: south,
        entity_kind: AvailabilityEntity::Teacher,
        entity_id: southern.teacher,
        timeslot_id: None,
        cycle_day_id: Some(southern.cycle_day),
        period_id: Some(southern.period),
        term_id: None,
        status: AvailabilityStatus::Unavailable,
        weight: None,
        reason: None,
        effective_from: None,
        effective_to: None,
    };
    project.db().availability().set(&scope).expect("rule");

    // The same cell, claimed by Northgate. `find_scope` is scoped by school, so there is
    // nothing for it to find and nothing for it to delete.
    let cleared = project
        .db()
        .availability()
        .clear(&AvailabilityRuleInput {
            school_id: north,
            ..scope.clone()
        })
        .expect("clearing a cell that is not yours is not an error, it is a no-op");
    assert!(!cleared, "it must report that nothing was removed");
    assert_eq!(
        project
            .db()
            .availability()
            .count_for(south, AvailabilityEntity::Teacher, southern.teacher)
            .expect("count"),
        1,
        "and Southmoor's rule is still there"
    );

    // Southmoor clears its own.
    assert!(
        project.db().availability().clear(&scope).expect("clear"),
        "the owning school removes it"
    );
}

/// Removing a logo cannot reach another school's.
#[test]
fn clearing_a_logo_cannot_cross_a_school() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    give_a_logo(&mut project, south);

    // Northgate clears "its" logo. It has none, which succeeds and does nothing -- the
    // operation is addressed by school, so there is no identifier to point at the wrong row.
    project
        .db()
        .structure()
        .clear_school_logo(north)
        .expect("clearing a logo that is not there is not an error");

    assert!(
        project
            .db()
            .structure()
            .school_logo(south)
            .expect("read")
            .is_some(),
        "Southmoor's logo must survive Northgate clearing its own"
    );
}

/// A facility another school still uses cannot be destroyed, and sharing keeps working.
#[test]
fn a_shared_facility_cannot_be_destroyed_from_one_schools_context() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);

    let northern_campus = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: north,
            name: "Riverside".to_owned(),
            code: "RIV".to_owned(),
            address: None,
        })
        .expect("campus");
    let southern_campus = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: south,
            name: "Riverside".to_owned(),
            code: "RIV".to_owned(),
            address: None,
        })
        .expect("campus");

    let hall = project
        .db()
        .structure()
        .create_room(
            &crate::repo::structure::RoomInput {
                campus_id: northern_campus.id,
                building_id: None,
                room_type_id: None,
                name: "Sports Hall".to_owned(),
                code: "HALL".to_owned(),
                capacity: 200,
                is_bookable: true,
                notes: None,
            },
            &[southern_campus.id],
        )
        .expect("one hall, shared with the other school");

    // Southmoor does not own it and cannot destroy it.
    let not_yours = project
        .db()
        .structure()
        .delete_room(south, hall.id, Expected::Any)
        .expect_err("Southmoor does not own the hall");
    assert!(
        matches!(not_yours, DbError::WrongSchool { .. }),
        "{not_yours:?}"
    );

    // Nor can Northgate, while Southmoor is still timetabling into it. This is the case
    // the ownership check alone would have let through.
    let still_used = project
        .db()
        .structure()
        .delete_room(north, hall.id, Expected::Any)
        .expect_err("Southmoor still shares it");
    assert!(
        matches!(still_used, DbError::StillReferenced { .. }),
        "expected StillReferenced so the interface says something still uses it, got \
         {still_used:?}"
    );

    // The association is intact throughout: nothing was half-removed on the way to the
    // refusal.
    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(hall.id)
            .expect("read")
            .len(),
        2,
        "the cross-school share of ADR 0014 survives a refused delete"
    );

    // A third school has no say in withdrawing the share...
    let third = project
        .db()
        .structure()
        .create_school(&school_input("Eastbrook Free School"))
        .expect("third school");
    let meddling = project
        .db()
        .structure()
        .unshare_room(third.id, hall.id, southern_campus.id)
        .expect_err("a third school has no say");
    assert!(
        matches!(meddling, DbError::WrongSchool { .. }),
        "{meddling:?}"
    );

    // ...but the school losing access does, and so would the owner.
    project
        .db()
        .structure()
        .unshare_room(south, hall.id, southern_campus.id)
        .expect("Southmoor may give up its own access");

    // And now the owner can destroy it, because nobody else uses it.
    project
        .db()
        .structure()
        .delete_room(north, hall.id, Expected::Any)
        .expect("with the share withdrawn, the owner may delete it");
    assert_eq!(orphan_references_of(&mut project), 0);
}

/// Releasing a grid's orphans cannot reach another school's grid.
#[test]
fn releasing_orphans_cannot_cross_a_school() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);
    let southern = fill(&mut project, south, "South");

    let request = MaterialisationRequest::new(southern.cycle, southern.bells, None);
    let preview = project
        .db()
        .time_model()
        .grid_preview(&request)
        .expect("preview");

    let refused = project
        .db()
        .time_model()
        .release_orphans(north, &request, &preview.fingerprint)
        .expect_err("the grid belongs to Southmoor");
    assert!(
        matches!(refused, DbError::WrongSchool { .. }),
        "{refused:?}"
    );

    let timeslot_refused = project
        .db()
        .time_model()
        .delete_timeslots(north, &[Uuid::now_v7()])
        .expect_err("a timeslot identifier is not authority either");
    assert!(
        matches!(
            timeslot_refused,
            DbError::WrongSchool { .. } | DbError::NotFound { .. }
        ),
        "{timeslot_refused:?}"
    );
}

/// A refused deletion writes nothing to the change feed.
///
/// The feed is what a future synchronisation will merge from, so a refusal that logged a
/// removal would be worse than one that silently deleted the row: the row would still be
/// there and every other installation would be told it had gone.
#[test]
fn a_refused_deletion_is_not_recorded_as_a_deletion() {
    let mut project = TwoSchools::new();
    let (north, south) = (project.north, project.south);
    let southern = fill(&mut project, south, "South");

    let before = deletions_logged(&mut project);

    project
        .db()
        .people()
        .delete_teacher(north, southern.teacher, Expected::Any)
        .expect_err("not Northgate's teacher");
    project
        .db()
        .structure()
        .delete_campus(north, southern.campus, Expected::Any)
        .expect_err("not Northgate's campus");

    assert_eq!(
        deletions_logged(&mut project),
        before,
        "a refusal is not a deletion and must not be logged as one"
    );

    // A real deletion still is -- twice over since V13, because a teacher now owns a
    // school membership and that row is removed through `repo::delete` rather than by a
    // CASCADE. Two rows went, so two removals are logged; a feed that reported one would
    // leave every other installation believing the membership was still there.
    project
        .db()
        .people()
        .delete_teacher(south, southern.teacher, Expected::Any)
        .expect("Southmoor's own teacher");
    assert_eq!(
        deletions_logged(&mut project),
        before + 2,
        "the teacher and their home-school membership"
    );
}

/// Writes a logo row directly.
///
/// `set_school_logo` measures real image bytes and refuses anything it cannot decode, and
/// these tests are about which school a logo belongs to rather than about decoding. The
/// row is what the scoping acts on, so the row is what they create.
fn give_a_logo(project: &mut TwoSchools, school: Uuid) {
    project
        .db()
        .write(|tx| {
            tx.execute(
                "INSERT INTO school_logo (id, school_id, media_type, bytes, width, height,
                                           created_at, updated_at, rev)
                 VALUES (?, ?, 'image/png', x'89504e470d0a1a0a', 16, 16, ?, ?, 1)",
                (Uuid::now_v7().to_string(), school.to_string(), STAMP, STAMP),
            )?;
            Ok(())
        })
        .expect("write a logo row");
}

fn row_counts(project: &mut TwoSchools) -> Vec<(String, i64)> {
    let tables = [
        "campus",
        "room_type",
        "academic_year",
        "term",
        "cycle",
        "cycle_day",
        "period_structure",
        "period",
        "year_level",
        "teacher",
        "student",
        "student_group",
        "student_group_member",
        "availability",
        "school_logo",
    ];
    tables
        .iter()
        .map(|table| {
            let count: i64 = project
                .db()
                .read(|conn| {
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .map_err(DbError::from)
                })
                .expect("count");
            ((*table).to_owned(), count)
        })
        .collect()
}

fn deletions_logged(project: &mut TwoSchools) -> i64 {
    project
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM change_log WHERE op = 'DELETE'",
                [],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("count deletions")
}

fn orphan_references_of(project: &mut TwoSchools) -> i64 {
    let db: &Database = project.db();
    orphan_references(db)
}
