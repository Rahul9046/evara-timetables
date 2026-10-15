//! Stage B: what a school teaches, and what one person is qualified and organised to
//! teach.
//!
//! # What these tests are for
//!
//! `V14` puts a project-level identity and a school-owned entity on either end of the same
//! row, twice, with deliberately different rules. Four things therefore have to be proved
//! rather than argued:
//!
//! 1. **A school's curriculum is its own.** Two schools may each hold "Mathematics", with
//!    the same code, as two records with two identifiers; a duplicate *within* one school
//!    is refused; and no write reaches across the boundary.
//! 2. **A qualification is a property of the person.** One teacher, two schools, two
//!    subject records, two qualifications, and still **one** teacher row. It survives a
//!    deactivated membership, a transfer and every edit short of deleting the person.
//! 3. **A department association is a narrowing of membership.** It requires employment at
//!    the department's school, and goes when that employment is withdrawn — the same
//!    bargain `teacher_campus` struck in `V13`.
//! 4. **Nothing an existing project held moved.** `V14` is additive with no backfill: a
//!    project migrated by it describes exactly the same schools it did before, with the
//!    same identifiers and the same revisions.
//!
//! # The schema is tested as well as the repository
//!
//! Several rules are a trigger pair *and* a repository check, for the reason `V12` gives:
//! the repository can name the offending field and the remedy, the trigger is what makes
//! the invariant true for a future repository, the CLI, an import or a hand-repair. Both
//! halves are asserted — the repository through its typed error, the trigger by writing the
//! row underneath it.

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::Expected;
use crate::repo::curriculum::{DepartmentInput, SubjectInput};
use crate::repo::membership::TeacherSchoolInput;
use crate::repo::people::TeacherInput;
use crate::repo::structure::{CampusInput, SchoolInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// The SQL under test, read from the migration itself so the two cannot drift apart.
const V14_SQL: &str = include_str!("../migrations/V14__curriculum_and_qualifications.sql");

// ================================================================== the trust fixture

/// Two schools in one project. Named rather than numbered, because every assertion below
/// is about *which* school and `school_a`/`school_b` makes a wrong expectation read as
/// plausible.
struct Trust {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
    north_campus: Uuid,
}

impl Trust {
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
            north_campus: Uuid::nil(),
        };
        this.north = this.school("Northgate Secondary");
        this.south = this.school("Southmoor College");
        this.north_campus = this.campus(this.north, "Northgate Main", "NGM");
        this
    }

    fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    fn school(&mut self, name: &str) -> Uuid {
        self.db()
            .structure()
            .create_school(&SchoolInput {
                name: name.to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("create school")
            .id
    }

    fn campus(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .structure()
            .create_campus(&CampusInput {
                school_id: school,
                name: name.to_owned(),
                code: code.to_owned(),
                address: None,
            })
            .expect("create campus")
            .id
    }

    fn teacher(&mut self, school: Uuid, family: &str) -> Uuid {
        self.db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: school,
                code: None,
                given_name: "Ada".to_owned(),
                family_name: family.to_owned(),
                email: None,
                home_campus_id: None,
                fte_percent: 100,
                max_periods_per_day: None,
                max_periods_per_cycle: None,
                max_consecutive_periods: None,
                notes: None,
                is_active: true,
            })
            .expect("create teacher")
            .id
    }

    fn department(&mut self, school: Uuid, name: &str, code: Option<&str>) -> Uuid {
        self.db()
            .curriculum()
            .create_department(&department_input(school, name, code))
            .expect("create department")
            .id
    }

    fn subject(&mut self, school: Uuid, name: &str, department: Option<Uuid>) -> Uuid {
        self.db()
            .curriculum()
            .create_subject(&subject_input(school, name, department))
            .expect("create subject")
            .id
    }

    /// Employs an existing teacher at another school.
    fn employ(&mut self, school: Uuid, teacher: Uuid) {
        self.db()
            .membership()
            .add_teacher_to_school(&TeacherSchoolInput {
                teacher_id: teacher,
                school_id: school,
                is_active: true,
            })
            .expect("add membership");
    }
}

fn department_input(school: Uuid, name: &str, code: Option<&str>) -> DepartmentInput {
    DepartmentInput {
        school_id: school,
        name: name.to_owned(),
        code: code.map(ToOwned::to_owned),
        description: None,
    }
}

fn subject_input(school: Uuid, name: &str, department: Option<Uuid>) -> SubjectInput {
    SubjectInput {
        school_id: school,
        department_id: department,
        name: name.to_owned(),
        code: None,
        description: None,
    }
}

// ======================================================================= the migration

#[test]
fn the_four_tables_arrive_with_their_change_log_triggers() {
    // `every_entity_table_has_all_three_change_log_triggers` in `tests_structure`
    // discovers tables from the live schema and would catch an omission, but it would do
    // so without naming which table. This says which.
    let mut trust = Trust::new();
    let triggers: Vec<String> = trust
        .db()
        .read(|conn| {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .expect("list triggers");

    for table in [
        "department",
        "subject",
        "teacher_qualification",
        "teacher_department",
    ] {
        for operation in ["insert", "update", "delete"] {
            let expected = format!("trg_{table}_changelog_{operation}");
            assert!(
                triggers.contains(&expected),
                "{expected} is missing, so {table} is absent from the change feed"
            );
        }
    }
}

#[test]
fn the_new_tables_are_registered_for_change_tracking() {
    for table in [
        "department",
        "subject",
        "teacher_qualification",
        "teacher_department",
    ] {
        assert!(
            crate::change_log::TRACKED_TABLES.contains(&table),
            "`{table}` must be in TRACKED_TABLES or `install_all` will never create its triggers"
        );
    }
}

#[test]
fn every_write_appears_in_the_change_feed() {
    let mut trust = Trust::new();
    let before = trust.db().changes_since(0, 10_000).expect("feed").len();

    let north = trust.north;
    let department = trust.department(north, "Mathematics", Some("MAT"));
    let subject = trust.subject(north, "Mathematics", Some(department));
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, department)
        .expect("assign department");

    let feed = trust.db().changes_since(0, 10_000).expect("feed");
    let tables: Vec<&str> = feed[before..]
        .iter()
        .map(|record| record.entity_table.as_str())
        .collect();

    for table in [
        "department",
        "subject",
        "teacher_qualification",
        "teacher_department",
    ] {
        assert!(
            tables.contains(&table),
            "a write to {table} produced no change-log entry; tables seen: {tables:?}"
        );
    }
}

/// The upgrade path every existing project will actually take.
///
/// A project left at `V13` by the previous release, with schools, campuses, teachers and
/// memberships in it, must reach `V14` with every row and every revision exactly as it
/// was — `V14` adds tables and backfills nothing, so there is no excuse for a single
/// changed value.
#[test]
fn a_v13_project_upgrades_without_touching_a_single_row() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("legacy.{PROJECT_EXTENSION}"));
    let site = Uuid::now_v7();
    const STAMP: &str = "2026-01-01T00:00:00.000Z";

    let school = Uuid::now_v7();
    let teacher = Uuid::now_v7();
    let membership = Uuid::now_v7();

    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    let version = crate::migrations::run_up_to(&mut conn, 13).expect("migrate to V13");
    assert_eq!(version, 13, "the fixture must stop at the previous version");

    let tx = conn.transaction().expect("transaction");
    tx.execute(
        "INSERT INTO school (id, name, timezone, locale, working_days,
                             created_at, updated_at, rev)
         VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', 31, ?, ?, 3)",
        (school.to_string(), STAMP, STAMP),
    )
    .expect("school");
    tx.execute(
        "INSERT INTO teacher (id, school_id, code, given_name, family_name, email,
                              home_campus_id, fte_percent, is_active,
                              created_at, updated_at, rev)
         VALUES (?, ?, 'T1', 'Ada', 'Okafor', NULL, NULL, 80, 1, ?, ?, 5)",
        (teacher.to_string(), school.to_string(), STAMP, STAMP),
    )
    .expect("teacher");
    tx.execute(
        "INSERT INTO teacher_school (id, teacher_id, school_id, is_active,
                                     created_at, updated_at, rev)
         VALUES (?, ?, ?, 1, ?, ?, 2)",
        (
            membership.to_string(),
            teacher.to_string(),
            school.to_string(),
            STAMP,
            STAMP,
        ),
    )
    .expect("membership");
    tx.commit().expect("commit");
    drop(conn);

    // The upgrade.
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("open and migrate");
    assert_eq!(
        db.schema_version(),
        crate::migrations::supported_schema_version(),
        "the project must reach the current schema"
    );

    // Every row, byte for byte.
    let found = db
        .structure()
        .school(school)
        .expect("read school")
        .expect("school");
    assert_eq!(found.name, "Northgate Secondary");
    assert_eq!(found.rev, 3, "V14 must not consume a revision");
    assert_eq!(found.updated_at, STAMP, "nor rewrite a timestamp");

    let found = db
        .people()
        .teacher(teacher)
        .expect("read teacher")
        .expect("teacher");
    assert_eq!(found.code.as_deref(), Some("T1"));
    assert_eq!(found.fte_percent, 80);
    assert_eq!(found.rev, 5);
    assert_eq!(found.updated_at, STAMP);

    let memberships = db
        .membership()
        .teacher_memberships(teacher)
        .expect("read memberships");
    assert_eq!(memberships.len(), 1, "the V13 membership must survive");
    assert_eq!(memberships[0].rev, 2);

    // And nothing was invented.
    assert!(
        db.curriculum()
            .departments(school)
            .expect("departments")
            .is_empty(),
        "V14 has no backfill: a school that was never asked what it teaches has no departments"
    );
    assert!(
        db.curriculum()
            .subjects(school)
            .expect("subjects")
            .is_empty(),
        "nor any subjects"
    );
}

/// `V14` is additive, so a project already at or past the first rebuilding migration needs
/// no safety copy to reach it.
#[test]
fn upgrading_from_v13_needs_no_safety_copy() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("legacy.{PROJECT_EXTENSION}"));
    let site = Uuid::now_v7();

    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    crate::migrations::run_up_to(&mut conn, 13).expect("migrate to V13");
    drop(conn);

    let db = Database::open_existing(&path, APP_VERSION, site).expect("open and migrate");
    assert!(
        db.safety_backup().is_none(),
        "crossing 13 to 14 rebuilds nothing, so a copy would be ceremony rather than protection"
    );

    // A project from *before* the rebuild still gets one, which is the mechanism working
    // unchanged rather than being weakened by this migration.
    let path = dir.path().join(format!("older.{PROJECT_EXTENSION}"));
    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    crate::migrations::run_up_to(&mut conn, 11).expect("migrate to V11");
    drop(conn);
    let db = Database::open_existing(&path, APP_VERSION, site).expect("open and migrate");
    assert!(
        db.safety_backup().is_some(),
        "a project below V12 is about to cross it, so it must still be protected"
    );
}

/// Reopening after the upgrade must be ordinary: refinery records V14 once, and the
/// triggers are not duplicated by `install_all` running again.
#[test]
fn the_upgrade_survives_a_reopen() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("reopen.{PROJECT_EXTENSION}"));
    let site = Uuid::now_v7();

    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    crate::migrations::run_up_to(&mut conn, 13).expect("migrate to V13");
    drop(conn);

    let first = Database::open_existing(&path, APP_VERSION, site)
        .expect("upgrade")
        .schema_version();
    let second = Database::open_existing(&path, APP_VERSION, site).expect("reopen");

    assert_eq!(first, second.schema_version());
    let applied: i64 = second
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM refinery_schema_history WHERE version = 14",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("count");
    assert_eq!(applied, 1, "V14 must be recorded exactly once");
}

/// The migration text itself, applied twice, must fail — which is what proves the
/// `CREATE TABLE`s are not accidentally idempotent and therefore that refinery's checksum
/// is the only thing standing between a project and a double application.
#[test]
fn the_migration_is_not_silently_reapplicable() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("twice.{PROJECT_EXTENSION}"));
    let site = Uuid::now_v7();

    let mut conn = crate::connection::open_configured(&path, site).expect("open");
    crate::migrations::run_up_to(&mut conn, 14).expect("migrate to V14");

    let tx = conn.transaction().expect("transaction");
    let outcome = tx.execute_batch(V14_SQL);
    assert!(
        outcome.is_err(),
        "re-running the migration must fail rather than quietly produce a second schema"
    );
}

// ======================================================================== departments

#[test]
fn a_department_belongs_to_exactly_one_school() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.north, "Mathematics", Some("MAT"));

    let found = trust
        .db()
        .curriculum()
        .department(maths)
        .expect("read")
        .expect("department");
    assert_eq!(found.school_id, trust.north);
    assert_eq!(found.rev, 1, "a new row starts at revision 1");
    assert_eq!(found.created_at, found.updated_at);
    assert_eq!(found.id.get_version_num(), 7, "identifiers are UUIDv7");

    // And it appears only in its own school's list.
    let north = trust.north;
    let south = trust.south;
    assert_eq!(
        trust
            .db()
            .curriculum()
            .departments(north)
            .expect("list")
            .len(),
        1
    );
    assert!(
        trust
            .db()
            .curriculum()
            .departments(south)
            .expect("list")
            .is_empty(),
        "a department must never appear under another school"
    );
}

#[test]
fn two_schools_may_each_hold_a_mathematics_department() {
    let mut trust = Trust::new();
    let north_maths = trust.department(trust.north, "Mathematics", Some("MAT"));
    // Same name, same code. Different school, so neither rule is broken.
    let south_maths = trust.department(trust.south, "Mathematics", Some("MAT"));

    assert_ne!(
        north_maths, south_maths,
        "two schools' Mathematics departments are two records"
    );
}

#[test]
fn a_duplicate_department_name_within_one_school_is_refused() {
    let mut trust = Trust::new();
    let north = trust.north;
    trust.department(north, "Mathematics", None);

    let outcome =
        trust
            .db()
            .curriculum()
            .create_department(&department_input(north, "Mathematics", None));

    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { field: "name", .. })),
        "expected a name collision, got {outcome:?}"
    );
}

#[test]
fn a_duplicate_department_code_within_one_school_is_refused_and_a_blank_one_is_not() {
    let mut trust = Trust::new();
    let north = trust.north;
    trust.department(north, "Mathematics", Some("MAT"));

    let outcome = trust.db().curriculum().create_department(&department_input(
        north,
        "Further Mathematics",
        Some("MAT"),
    ));
    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { field: "code", .. })),
        "expected a code collision, got {outcome:?}"
    );

    // The partial index is the documented exception to the COALESCE rule: a school that
    // does not code its departments must be able to enter more than one.
    trust.department(north, "Humanities", None);
    trust.department(north, "Science", None);
    assert_eq!(
        trust
            .db()
            .curriculum()
            .departments(north)
            .expect("list")
            .len(),
        3
    );
}

#[test]
fn a_department_update_refuses_another_schools_identifier() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let maths = trust.department(north, "Mathematics", None);

    // Southmoor holds Northgate's identifier — ordinary, since Stage 0C lists project-wide
    // — and names itself as the acting school.
    let outcome = trust.db().curriculum().update_department(
        maths,
        Expected::Rev(1),
        &department_input(south, "Hijacked", None),
    );

    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "department"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );

    // And nothing was written.
    let found = trust
        .db()
        .curriculum()
        .department(maths)
        .expect("read")
        .expect("department");
    assert_eq!(found.name, "Mathematics");
    assert_eq!(found.rev, 1, "a refused update consumes no revision");
}

#[test]
fn a_department_delete_refuses_another_schools_identifier() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.north, "Mathematics", None);
    let south = trust.south;

    let outcome = trust
        .db()
        .curriculum()
        .delete_department(south, maths, Expected::Rev(1));

    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "department"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );
    assert!(
        trust
            .db()
            .curriculum()
            .department(maths)
            .expect("read")
            .is_some(),
        "the department must still be there"
    );
}

#[test]
fn a_department_with_subjects_in_it_cannot_be_deleted() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.north, "Mathematics", None);
    let subject = trust.subject(trust.north, "Mathematics", Some(maths));
    let north = trust.north;

    let report = trust
        .db()
        .curriculum()
        .department_dependents(maths)
        .expect("dependents");
    assert_eq!(report.subjects, 1);
    assert_eq!(report.teachers, 0);
    assert_eq!(report.total, 1, "the total must agree with its parts");

    let outcome = trust
        .db()
        .curriculum()
        .delete_department(north, maths, Expected::Rev(1));
    assert!(
        matches!(
            outcome,
            Err(DbError::StillReferenced {
                entity: "department"
            })
        ),
        "expected a refusal, got {outcome:?}"
    );

    // Unclassifying the subject is the user's own step, and then the delete is offered.
    trust
        .db()
        .curriculum()
        .update_subject(
            subject,
            Expected::Rev(1),
            &subject_input(north, "Mathematics", None),
        )
        .expect("unclassify");
    trust
        .db()
        .curriculum()
        .delete_department(north, maths, Expected::Rev(1))
        .expect("the department is now free");

    assert!(
        trust
            .db()
            .curriculum()
            .subject(subject)
            .expect("read")
            .is_some(),
        "the subject survives: deleting a department never deletes what it taught"
    );
}

#[test]
fn a_department_with_teachers_in_it_cannot_be_deleted() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.north, "Mathematics", None);
    let teacher = trust.teacher(trust.north, "Okafor");
    let north = trust.north;
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths)
        .expect("assign");

    let report = trust
        .db()
        .curriculum()
        .department_dependents(maths)
        .expect("dependents");
    assert_eq!(report.teachers, 1);

    let outcome = trust
        .db()
        .curriculum()
        .delete_department(north, maths, Expected::Rev(1));
    assert!(
        matches!(
            outcome,
            Err(DbError::StillReferenced {
                entity: "department"
            })
        ),
        "expected a refusal, got {outcome:?}"
    );
}

#[test]
fn a_school_that_still_describes_a_curriculum_cannot_be_deleted_and_is_told_why() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.south, "Mathematics", None);
    trust.subject(trust.south, "Mathematics", Some(maths));
    let south = trust.south;

    let dependents = trust
        .db()
        .structure()
        .school_dependents(south)
        .expect("dependents");
    assert_eq!(dependents.departments, 1);
    assert_eq!(dependents.subjects, 1);
    assert!(dependents.total >= 2);

    let outcome = trust.db().structure().delete_school(south, Expected::Any);
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { entity: "school" })),
        "expected a refusal naming the school, got {outcome:?}"
    );
}

// =========================================================================== subjects

#[test]
fn a_subject_may_exist_without_a_department() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Mathematics", None);

    let found = trust
        .db()
        .curriculum()
        .subject(subject)
        .expect("read")
        .expect("subject");
    assert_eq!(found.department_id, None);
    assert_eq!(found.school_id, trust.north);
}

#[test]
fn two_schools_mathematics_are_two_subjects_with_the_same_code() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;

    let north_maths = trust
        .db()
        .curriculum()
        .create_subject(&SubjectInput {
            school_id: north,
            department_id: None,
            name: "Mathematics".to_owned(),
            code: Some("MAT".to_owned()),
            description: None,
        })
        .expect("Northgate's Mathematics")
        .id;
    let south_maths = trust
        .db()
        .curriculum()
        .create_subject(&SubjectInput {
            school_id: south,
            department_id: None,
            name: "Mathematics".to_owned(),
            code: Some("MAT".to_owned()),
            description: None,
        })
        .expect("Southmoor's Mathematics")
        .id;

    assert_ne!(north_maths, south_maths);
    // And each school's list holds exactly its own.
    let listed: Vec<Uuid> = trust
        .db()
        .curriculum()
        .subjects(north)
        .expect("list")
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(listed, vec![north_maths]);
}

#[test]
fn a_duplicate_subject_name_within_one_school_is_refused() {
    let mut trust = Trust::new();
    let north = trust.north;
    trust.subject(north, "Mathematics", None);

    let outcome =
        trust
            .db()
            .curriculum()
            .create_subject(&subject_input(north, "Mathematics", None));

    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { field: "name", .. })),
        "expected a name collision, got {outcome:?}"
    );
}

#[test]
fn a_subject_cannot_be_taught_by_another_schools_department() {
    let mut trust = Trust::new();
    let south_maths = trust.department(trust.south, "Mathematics", None);
    let north = trust.north;

    // On create.
    let outcome = trust.db().curriculum().create_subject(&subject_input(
        north,
        "Mathematics",
        Some(south_maths),
    ));
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "expected a refusal naming the field rather than the school, got {outcome:?}"
    );

    // And on update, which is the case a stale department picker actually produces.
    let subject = trust.subject(north, "Mathematics", None);
    let outcome = trust.db().curriculum().update_subject(
        subject,
        Expected::Rev(1),
        &subject_input(north, "Mathematics", Some(south_maths)),
    );
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "expected a refusal, got {outcome:?}"
    );
    let found = trust
        .db()
        .curriculum()
        .subject(subject)
        .expect("read")
        .expect("subject");
    assert_eq!(found.department_id, None, "nothing was written");
    assert_eq!(found.rev, 1, "and no revision was consumed");
}

/// The repository refuses it; so does the schema, which is what makes the rule true for a
/// future repository, the CLI, an import or a hand-repair.
#[test]
fn the_schema_itself_refuses_a_cross_school_department_on_a_subject() {
    let mut trust = Trust::new();
    let south_maths = trust.department(trust.south, "Mathematics", None);
    let subject = trust.subject(trust.north, "Mathematics", None);

    let outcome = trust.db().write(|tx| {
        tx.execute(
            "UPDATE subject SET department_id = ? WHERE id = ?",
            [south_maths.to_string(), subject.to_string()],
        )
        .map_err(crate::error::DbError::from)
    });

    assert!(
        outcome.is_err(),
        "the trigger pair must refuse the row even written directly"
    );
}

#[test]
fn a_subjects_identifier_survives_every_edit() {
    let mut trust = Trust::new();
    let maths = trust.department(trust.north, "Mathematics", None);
    let subject = trust.subject(trust.north, "Mathematics", None);
    let north = trust.north;

    // Rename, code, department, description — Stage C references this identifier, so the
    // one thing that must not change is the one thing nothing here writes.
    let saved = trust
        .db()
        .curriculum()
        .update_subject(
            subject,
            Expected::Rev(1),
            &SubjectInput {
                school_id: north,
                department_id: Some(maths),
                name: "Further Mathematics".to_owned(),
                code: Some("FMAT".to_owned()),
                description: Some("A-level only".to_owned()),
            },
        )
        .expect("update");

    assert_eq!(
        saved.id, subject,
        "the identifier is stable across every edit"
    );
    assert_eq!(saved.rev, 2);
    assert_eq!(saved.department_id, Some(maths));
    assert_eq!(saved.name, "Further Mathematics");
}

#[test]
fn a_subject_nobody_is_qualified_for_can_be_deleted() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Mathematics", None);
    let north = trust.north;

    let report = trust
        .db()
        .curriculum()
        .subject_dependents(subject)
        .expect("dependents");
    assert_eq!(report.total, 0);

    trust
        .db()
        .curriculum()
        .delete_subject(north, subject, Expected::Rev(1))
        .expect("delete");
    assert!(
        trust
            .db()
            .curriculum()
            .subject(subject)
            .expect("read")
            .is_none()
    );
}

#[test]
fn a_subject_somebody_is_qualified_for_is_refused_rather_than_losing_the_record() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Mathematics", None);
    let teacher = trust.teacher(trust.north, "Okafor");
    let north = trust.north;
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    let report = trust
        .db()
        .curriculum()
        .subject_dependents(subject)
        .expect("dependents");
    assert_eq!(report.qualified_teachers, 1);

    let outcome = trust
        .db()
        .curriculum()
        .delete_subject(north, subject, Expected::Rev(1));
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { entity: "subject" })),
        "a qualification is a record about a person and must not be discarded silently: {outcome:?}"
    );

    // Removing it explicitly is the remedy, and then the delete is offered.
    trust
        .db()
        .curriculum()
        .unqualify_teacher(north, teacher, subject, Expected::Any)
        .expect("unqualify");
    trust
        .db()
        .curriculum()
        .delete_subject(north, subject, Expected::Rev(1))
        .expect("the subject is now free");
}

#[test]
fn a_subjects_qualified_teachers_are_listed_project_wide() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Chemistry", None);
    // One of Northgate's own, and one borrowed from Southmoor.
    let mine = trust.teacher(trust.north, "Okafor");
    let visiting = trust.teacher(trust.south, "Bellweather");
    let north = trust.north;
    trust.employ(north, visiting);

    for teacher in [mine, visiting] {
        trust
            .db()
            .curriculum()
            .qualify_teacher(north, teacher, subject)
            .expect("qualify");
    }

    let listed = trust
        .db()
        .curriculum()
        .subject_teachers(north, subject)
        .expect("list");
    let ids: Vec<Uuid> = listed.iter().map(|entry| entry.teacher.id).collect();
    assert!(
        ids.contains(&mine) && ids.contains(&visiting),
        "a visiting teacher qualified for this school's subject is qualified for it: {ids:?}"
    );
    assert_eq!(listed.len(), 2);

    // Ordered by name, so the screen does not re-sort.
    assert_eq!(listed[0].teacher.family_name, "Bellweather");
}

#[test]
fn another_school_may_not_read_or_write_this_schools_subject_qualifications() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Chemistry", None);
    let teacher = trust.teacher(trust.north, "Okafor");
    let south = trust.south;

    let outcome = trust.db().curriculum().subject_teachers(south, subject);
    assert!(
        matches!(outcome, Err(DbError::WrongSchool { entity: "subject" })),
        "expected a wrong-school refusal on the read, got {outcome:?}"
    );

    let outcome = trust
        .db()
        .curriculum()
        .qualify_teacher(south, teacher, subject);
    assert!(
        matches!(outcome, Err(DbError::WrongSchool { entity: "subject" })),
        "a school may not record a qualification against an institution it does not run: {outcome:?}"
    );
}

// ===================================================================== qualifications

#[test]
fn one_teacher_may_be_qualified_at_two_schools_and_stays_one_teacher() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;

    // The arrangement the stage exists for: the same name in two curricula.
    let north_maths = trust.subject(north, "Mathematics", None);
    let south_maths = trust.subject(south, "Mathematics", None);

    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);

    for (school, subject) in [(north, north_maths), (south, south_maths)] {
        trust
            .db()
            .curriculum()
            .qualify_teacher(school, teacher, subject)
            .expect("qualify");
    }

    // Two qualifications, naming two different subject records.
    let held = trust
        .db()
        .curriculum()
        .qualifications_of_teacher(teacher)
        .expect("list");
    assert_eq!(held.len(), 2);
    let subjects: Vec<Uuid> = held.iter().map(|entry| entry.subject.id).collect();
    assert!(subjects.contains(&north_maths) && subjects.contains(&south_maths));
    let schools: Vec<Uuid> = held.iter().map(|entry| entry.subject.school_id).collect();
    assert!(schools.contains(&north) && schools.contains(&south));

    // And exactly one teacher row in the whole project, which is the point.
    let everywhere: Vec<Uuid> = [north, south]
        .into_iter()
        .flat_map(|school| {
            trust
                .db()
                .people()
                .teachers(school)
                .expect("list")
                .into_iter()
                .map(|row| row.id)
        })
        .collect();
    assert_eq!(
        everywhere,
        vec![teacher, teacher],
        "one person, two lists, one row"
    );
}

#[test]
fn the_same_qualification_cannot_be_recorded_twice() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Mathematics", None);
    let teacher = trust.teacher(trust.north, "Okafor");
    let north = trust.north;
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    let outcome = trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject);
    assert!(
        matches!(
            outcome,
            Err(DbError::DuplicateValue {
                entity: "qualification",
                ..
            })
        ),
        "saying it twice says nothing more, and is a mistake rather than a no-op: {outcome:?}"
    );

    let held = trust
        .db()
        .curriculum()
        .qualifications_of_teacher(teacher)
        .expect("list");
    assert_eq!(held.len(), 1);
}

/// The schema's own `UNIQUE (teacher_id, subject_id)`, checked underneath the repository.
#[test]
fn the_schema_itself_refuses_a_duplicate_qualification() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.north, "Mathematics", None);
    let teacher = trust.teacher(trust.north, "Okafor");
    let north = trust.north;
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    let outcome = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO teacher_qualification (id, teacher_id, subject_id,
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
            [
                Uuid::now_v7().to_string(),
                teacher.to_string(),
                subject.to_string(),
            ],
        )
        .map_err(crate::error::DbError::from)
    });
    assert!(
        outcome.is_err(),
        "the unique index must hold underneath the repository"
    );
}

#[test]
fn a_qualification_needs_no_school_membership_and_creates_none() {
    let mut trust = Trust::new();
    let subject = trust.subject(trust.south, "Mathematics", None);
    // Created at Northgate, never employed by Southmoor.
    let teacher = trust.teacher(trust.north, "Okafor");
    let south = trust.south;

    trust
        .db()
        .curriculum()
        .qualify_teacher(south, teacher, subject)
        .expect("a qualification is a statement about competence, not about employment");

    let memberships = trust
        .db()
        .membership()
        .teacher_memberships(teacher)
        .expect("memberships");
    assert_eq!(
        memberships.len(),
        1,
        "qualifying somebody must not quietly employ them: {memberships:?}"
    );
    assert_eq!(memberships[0].school_id, trust.north);
}

#[test]
fn a_qualification_survives_a_deactivated_membership() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let subject = trust.subject(south, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .qualify_teacher(south, teacher, subject)
        .expect("qualify");

    // She leaves Southmoor. Her membership is deactivated, not removed.
    trust
        .db()
        .membership()
        .set_teacher_membership_active(south, teacher, Expected::Any, false)
        .expect("deactivate");

    let held = trust
        .db()
        .curriculum()
        .qualifications_of_teacher(teacher)
        .expect("list");
    assert_eq!(
        held.len(),
        1,
        "a teacher who has left does not become unqualified to teach what she could teach"
    );
    assert_eq!(held[0].subject.id, subject);
}

#[test]
fn a_qualification_survives_a_membership_being_removed_outright() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let subject = trust.subject(south, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .qualify_teacher(south, teacher, subject)
        .expect("qualify");

    trust
        .db()
        .membership()
        .remove_teacher_from_school(south, teacher)
        .expect("withdraw employment");

    let held = trust
        .db()
        .curriculum()
        .qualifications_of_teacher(teacher)
        .expect("list");
    assert_eq!(
        held.len(),
        1,
        "withdrawing employment is not a statement about what somebody can teach"
    );
}

/// Every other thing that could plausibly be thought to clear a qualification, and does
/// not. ADR 0025 §5 lists them; "we would never do that" is not a guarantee.
#[test]
fn nothing_but_an_explicit_removal_clears_a_qualification() {
    let mut trust = Trust::new();
    let north = trust.north;
    let campus = trust.north_campus;
    let maths = trust.department(north, "Mathematics", None);
    let subject = trust.subject(north, "Mathematics", Some(maths));
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    let still_held = |trust: &mut Trust| -> usize {
        trust
            .db()
            .curriculum()
            .qualifications_of_teacher(teacher)
            .expect("list")
            .len()
    };

    // Archiving the person in the project.
    let stored = trust
        .db()
        .people()
        .teacher(teacher)
        .expect("read")
        .expect("teacher");
    trust
        .db()
        .people()
        .update_teacher(
            teacher,
            Expected::Rev(stored.rev),
            &TeacherInput {
                school_id: north,
                code: None,
                given_name: "Ada".to_owned(),
                family_name: "Okafor-Hale".to_owned(),
                email: None,
                home_campus_id: None,
                fte_percent: 100,
                max_periods_per_day: None,
                max_periods_per_cycle: None,
                max_consecutive_periods: None,
                notes: None,
                is_active: false,
            },
        )
        .expect("archive and rename");
    assert_eq!(
        still_held(&mut trust),
        1,
        "editing identity changed a qualification"
    );

    // Campus eligibility, assigned and then withdrawn.
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, teacher, campus)
        .expect("assign campus");
    assert_eq!(
        still_held(&mut trust),
        1,
        "assigning a campus changed a qualification"
    );
    trust
        .db()
        .membership()
        .remove_teacher_campus(north, teacher, campus)
        .expect("withdraw campus");
    assert_eq!(
        still_held(&mut trust),
        1,
        "withdrawing a campus changed a qualification"
    );

    // A department association, assigned and then removed.
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths)
        .expect("assign department");
    assert_eq!(
        still_held(&mut trust),
        1,
        "joining a department changed a qualification"
    );
    trust
        .db()
        .curriculum()
        .remove_teacher_department(north, teacher, maths, Expected::Any)
        .expect("leave department");
    assert_eq!(
        still_held(&mut trust),
        1,
        "leaving a department changed a qualification"
    );

    // And the one thing that does clear it.
    trust
        .db()
        .curriculum()
        .unqualify_teacher(north, teacher, subject, Expected::Any)
        .expect("remove explicitly");
    assert_eq!(still_held(&mut trust), 0);
}

#[test]
fn deleting_the_person_takes_their_qualifications_with_them() {
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    // Destroying the person is the one operation that may: a qualification with nobody to
    // hold it is a dangling fact rather than history.
    trust
        .db()
        .people()
        .delete_teacher(north, teacher, Expected::Any)
        .expect("delete the teacher");

    let remaining = trust
        .db()
        .curriculum()
        .subject_teachers(north, subject)
        .expect("list");
    assert!(remaining.is_empty());
    // And the subject itself is untouched.
    assert!(
        trust
            .db()
            .curriculum()
            .subject(subject)
            .expect("read")
            .is_some()
    );
}

#[test]
fn removing_a_qualification_needs_the_school_that_owns_the_subject() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let subject = trust.subject(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    let outcome = trust
        .db()
        .curriculum()
        .unqualify_teacher(south, teacher, subject, Expected::Any);
    assert!(
        matches!(outcome, Err(DbError::WrongSchool { entity: "subject" })),
        "another school may see this qualification but not silently modify it: {outcome:?}"
    );
    assert_eq!(
        trust
            .db()
            .curriculum()
            .qualifications_of_teacher(teacher)
            .expect("list")
            .len(),
        1
    );
}

// =============================================================== department membership

#[test]
fn a_department_association_requires_employment_at_that_school() {
    let mut trust = Trust::new();
    let south_maths = trust.department(trust.south, "Mathematics", None);
    // Employed by Northgate only.
    let teacher = trust.teacher(trust.north, "Okafor");
    let south = trust.south;

    let outcome = trust
        .db()
        .curriculum()
        .assign_teacher_department(south, teacher, south_maths);
    match outcome {
        Err(DbError::Invalid { ref message }) => assert!(
            message.contains("Add this teacher to the school"),
            "the message must name the remedy, which the database's own refusal cannot: {message}"
        ),
        other => panic!("expected an Invalid naming the remedy, got {other:?}"),
    }

    // With the membership in place it is accepted.
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .assign_teacher_department(south, teacher, south_maths)
        .expect("assign");
}

/// The repository refuses it; so does the trigger pair, for the reason `V13` gives.
#[test]
fn the_schema_itself_refuses_a_department_of_a_school_the_teacher_does_not_belong_to() {
    let mut trust = Trust::new();
    let south_maths = trust.department(trust.south, "Mathematics", None);
    let teacher = trust.teacher(trust.north, "Okafor");

    let outcome = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO teacher_department (id, teacher_id, department_id,
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
            [
                Uuid::now_v7().to_string(),
                teacher.to_string(),
                south_maths.to_string(),
            ],
        )
        .map_err(crate::error::DbError::from)
    });
    assert!(
        outcome.is_err(),
        "the trigger must refuse the row even written directly"
    );
}

#[test]
fn a_teacher_may_belong_to_different_departments_at_two_schools() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let north_maths = trust.department(north, "Mathematics", None);
    let south_science = trust.department(south, "Science", None);

    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, north_maths)
        .expect("Mathematics at Northgate");
    trust
        .db()
        .curriculum()
        .assign_teacher_department(south, teacher, south_science)
        .expect("Science at Southmoor");

    // The arrangement a single `teacher.department_id` could not have expressed, which is
    // why V14 does not have one.
    let held = trust
        .db()
        .curriculum()
        .departments_of_teacher(teacher)
        .expect("list");
    assert_eq!(held.len(), 2);
    let names: Vec<&str> = held
        .iter()
        .map(|entry| entry.department.name.as_str())
        .collect();
    assert!(
        names.contains(&"Mathematics") && names.contains(&"Science"),
        "{names:?}"
    );
}

#[test]
fn a_teacher_may_belong_to_two_departments_at_one_school() {
    let mut trust = Trust::new();
    let north = trust.north;
    let maths = trust.department(north, "Mathematics", None);
    let physics = trust.department(north, "Physics", None);
    let teacher = trust.teacher(north, "Okafor");

    for department in [maths, physics] {
        trust
            .db()
            .curriculum()
            .assign_teacher_department(north, teacher, department)
            .expect("assign");
    }

    // The arrangement `teacher_school.department_id` could not have expressed either.
    assert_eq!(
        trust
            .db()
            .curriculum()
            .departments_of_teacher(teacher)
            .expect("list")
            .len(),
        2
    );
}

#[test]
fn the_same_department_association_cannot_be_recorded_twice() {
    let mut trust = Trust::new();
    let north = trust.north;
    let maths = trust.department(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths)
        .expect("assign");

    let outcome = trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths);
    assert!(
        matches!(
            outcome,
            Err(DbError::DuplicateValue {
                entity: "department association",
                ..
            })
        ),
        "expected a duplicate, got {outcome:?}"
    );
}

#[test]
fn only_the_school_that_owns_a_department_may_change_who_is_in_it() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let north_maths = trust.department(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, north_maths)
        .expect("assign");

    // Southmoor employs her and can see the association, and still may not touch it.
    let outcome = trust
        .db()
        .curriculum()
        .assign_teacher_department(south, teacher, north_maths);
    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "department"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );

    let outcome = trust.db().curriculum().remove_teacher_department(
        south,
        teacher,
        north_maths,
        Expected::Any,
    );
    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "department"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );
    assert_eq!(
        trust
            .db()
            .curriculum()
            .departments_of_teacher(teacher)
            .expect("list")
            .len(),
        1,
        "nothing was removed"
    );
}

#[test]
fn a_deactivated_membership_keeps_the_department_association() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let south_science = trust.department(south, "Science", None);
    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    trust
        .db()
        .curriculum()
        .assign_teacher_department(south, teacher, south_science)
        .expect("assign");

    trust
        .db()
        .membership()
        .set_teacher_membership_active(south, teacher, Expected::Any, false)
        .expect("deactivate");

    assert_eq!(
        trust
            .db()
            .curriculum()
            .departments_of_teacher(teacher)
            .expect("list")
            .len(),
        1,
        "a teacher who has left keeps the record of which departments she worked in, the \
         way she keeps which campuses she worked at"
    );
}

#[test]
fn removing_a_membership_clears_that_schools_department_associations_and_no_others() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let north_maths = trust.department(north, "Mathematics", None);
    let south_science = trust.department(south, "Science", None);
    let teacher = trust.teacher(north, "Okafor");
    trust.employ(south, teacher);
    for (school, department) in [(north, north_maths), (south, south_science)] {
        trust
            .db()
            .curriculum()
            .assign_teacher_department(school, teacher, department)
            .expect("assign");
    }

    trust
        .db()
        .membership()
        .remove_teacher_from_school(south, teacher)
        .expect("withdraw Southmoor's employment");

    let held = trust
        .db()
        .curriculum()
        .departments_of_teacher(teacher)
        .expect("list");
    assert_eq!(
        held.len(),
        1,
        "only Southmoor's association should have gone"
    );
    assert_eq!(held[0].department.id, north_maths);

    // And each removal is a truthful change-log entry rather than a silent orphan.
    let feed = trust.db().changes_since(0, 10_000).expect("feed");
    assert!(
        feed.iter()
            .any(|record| record.entity_table == "teacher_department" && record.op == "DELETE"),
        "the removal must be visible in the change feed"
    );
}

#[test]
fn deleting_the_person_takes_their_department_associations_with_them() {
    let mut trust = Trust::new();
    let north = trust.north;
    let maths = trust.department(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths)
        .expect("assign");

    trust
        .db()
        .people()
        .delete_teacher(north, teacher, Expected::Any)
        .expect("delete the teacher");

    assert!(
        trust
            .db()
            .curriculum()
            .department_teachers(north, maths)
            .expect("list")
            .is_empty()
    );
    // And the department survives, which is the whole asymmetry: a department is a
    // school's own record, a person is not.
    assert!(
        trust
            .db()
            .curriculum()
            .department(maths)
            .expect("read")
            .is_some()
    );
}

#[test]
fn a_departments_teachers_are_listed_for_its_own_school_only() {
    let mut trust = Trust::new();
    let north = trust.north;
    let south = trust.south;
    let maths = trust.department(north, "Mathematics", None);
    let teacher = trust.teacher(north, "Okafor");
    trust
        .db()
        .curriculum()
        .assign_teacher_department(north, teacher, maths)
        .expect("assign");

    assert_eq!(
        trust
            .db()
            .curriculum()
            .department_teachers(north, maths)
            .expect("list")
            .len(),
        1
    );
    let outcome = trust.db().curriculum().department_teachers(south, maths);
    assert!(
        matches!(
            outcome,
            Err(DbError::WrongSchool {
                entity: "department"
            })
        ),
        "expected a wrong-school refusal, got {outcome:?}"
    );
}
