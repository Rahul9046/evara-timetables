//! Stage C: what a school teaches to whom, how often, and for how long at a time.
//!
//! # What these tests are for
//!
//! `V15` puts four tables in a chain with no `school_id` anywhere in it, makes a teacher
//! assignment turn on three facts of two different kinds, and represents a cycle's worth of
//! demand as rows rather than a number. Five things therefore have to be proved rather than
//! argued:
//!
//! 1. **The chain carries the school.** A course belongs to a school only through its
//!    subject, a section only through its course, an activity only through its section —
//!    and every write refuses an identifier from another school at every depth. The whole
//!    reason there is no second `school_id` is that the chain is enough, so "the chain is
//!    enough" is the claim under test.
//! 2. **Two of the three assignment facts refuse and one warns.** ADR 0028 amends
//!    ADR 0025 §4, and the amendment is only safe if the two halves stay separate: an
//!    inactive membership is refused, a missing qualification is reported and accepted.
//! 3. **Demand is rows.** Four single periods is four rows; one double is one row of
//!    duration 2; the totals are a sum and not a product; and nothing claims a demand is
//!    schedulable.
//! 4. **A group alone is a complete audience.** ADR 0006 requires a timetable to be
//!    buildable from group sizes with no student records at all, and a roster that *is*
//!    entered must not be added to the group size on top.
//! 5. **Nothing an existing project held moved.** `V15` is additive with no backfill: a
//!    project migrated by it describes exactly the same schools it did before, with the
//!    same identifiers and the same revisions.
//!
//! # The schema is tested as well as the repository
//!
//! Every cross-school and eligibility rule here is a trigger pair *and* a repository check,
//! for the reason `V12` gives: the repository can name the offending field and the remedy,
//! the trigger is what makes the invariant true for a future repository, the CLI, an import
//! or a hand-repair. Both halves are asserted — the repository through its typed error, the
//! trigger by writing the row underneath it.

use evara_core::calendar::WorkingDays;
use evara_core::lessons::{DeclaredComparison, DemandFinding};
use tempfile::TempDir;
use uuid::Uuid;

use crate::repo::Expected;
use crate::repo::curriculum::SubjectInput;
use crate::repo::lessons::{ClassSectionInput, CourseInput};
use crate::repo::membership::{EnrolmentStatus, StudentEnrolmentInput, TeacherSchoolInput};
use crate::repo::people::{
    GroupKind, StudentGroupInput, StudentGroupMemberInput, StudentInput, TeacherInput,
    YearLevelInput,
};
use crate::repo::structure::{CampusInput, RoomTypeInput, SchoolInput};
use crate::repo::time_model::{
    CycleInput, CycleKind, PeriodInput, PeriodKind, PeriodStructureInput,
};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// The SQL under test, read from the migration itself so the two cannot drift apart.
const V15_SQL: &str = include_str!("../migrations/V15__lessons_and_activities.sql");

/// A fixed timestamp for rows written directly, so a migration test can assert that the
/// upgrade left them byte for byte as they were.
const STAMP: &str = "2026-01-01T00:00:00.000Z";

// ================================================================== the trust fixture

/// Two schools in one project, with enough of each to build a lesson.
///
/// Named rather than numbered, for the reason `tests_curriculum` gives: every assertion
/// below is about *which* school, and `school_a`/`school_b` makes a wrong expectation read
/// as plausible.
struct Trust {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
    north_main: Uuid,
    north_junior: Uuid,
    south_main: Uuid,
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
            north_main: Uuid::nil(),
            north_junior: Uuid::nil(),
            south_main: Uuid::nil(),
        };
        this.north = this.school("Northgate Secondary");
        this.south = this.school("Southmoor College");
        this.north_main = this.campus(this.north, "Northgate Main", "NGM");
        this.north_junior = this.campus(this.north, "Northgate Junior", "NGJ");
        this.south_main = this.campus(this.south, "Southmoor Main", "SMM");
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

    fn year_level(&mut self, school: Uuid, name: &str, ordinal: i64) -> Uuid {
        self.db()
            .people()
            .create_year_level(&YearLevelInput {
                school_id: school,
                name: name.to_owned(),
                ordinal,
            })
            .expect("create year level")
            .id
    }

    fn room_type(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .structure()
            .create_room_type(&RoomTypeInput {
                school_id: school,
                name: name.to_owned(),
                code: code.to_owned(),
            })
            .expect("create room type")
            .id
    }

    fn subject(&mut self, school: Uuid, name: &str) -> Uuid {
        self.db()
            .curriculum()
            .create_subject(&SubjectInput {
                school_id: school,
                department_id: None,
                name: name.to_owned(),
                code: None,
                description: None,
            })
            .expect("create subject")
            .id
    }

    /// A teacher created at `school`, so that school is their home and their membership
    /// exists and is active.
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

    fn group(&mut self, school: Uuid, name: &str, size: Option<i64>) -> Uuid {
        self.db()
            .people()
            .create_student_group(&StudentGroupInput {
                school_id: school,
                name: name.to_owned(),
                code: None,
                kind: GroupKind::Class,
                year_level_id: None,
                parent_group_id: None,
                size,
                campus_id: None,
                children_are_disjoint: None,
            })
            .expect("create student group")
            .id
    }

    /// A student created at `school`, so that school holds their one current spell.
    fn student(&mut self, school: Uuid, family: &str) -> Uuid {
        self.db()
            .people()
            .create_student(&StudentInput {
                school_id: school,
                code: None,
                given_name: "Bea".to_owned(),
                family_name: family.to_owned(),
                email: None,
                year_level_id: None,
                campus_id: None,
                is_active: true,
            })
            .expect("create student")
            .id
    }

    /// A course of one of `school`'s subjects, with a declared demand.
    fn course(&mut self, school: Uuid, subject: Uuid, name: &str, periods: Option<i64>) -> Uuid {
        self.db()
            .lessons()
            .create_course(school, &course_input(subject, name, periods))
            .expect("create course")
            .id
    }

    fn section(&mut self, school: Uuid, course: Uuid, name: &str, campus: Option<Uuid>) -> Uuid {
        self.db()
            .lessons()
            .create_class_section(
                school,
                &ClassSectionInput {
                    course_id: course,
                    name: name.to_owned(),
                    code: None,
                    campus_id: campus,
                },
            )
            .expect("create class section")
            .id
    }

    /// A five-day week of six teaching periods with a break after the third.
    ///
    /// The shape every capacity assertion below is measured against: six teaching periods
    /// in a day, but never more than three in a row.
    fn ordinary_time_model(&mut self, school: Uuid) {
        let cycle = self
            .db()
            .time_model()
            .create_cycle(&CycleInput {
                school_id: school,
                name: "Week".to_owned(),
                kind: CycleKind::Weekly,
                day_count: 7,
                week_count: 1,
                is_default: true,
            })
            .expect("create cycle")
            .id;
        // A weekly cycle creates all seven positions itself (ADR 0017), so only a custom
        // one needs its days adding. Assert the shape rather than assume it.
        let days = self
            .db()
            .time_model()
            .cycle_days(cycle)
            .expect("days")
            .len();
        assert_eq!(days, 7, "a weekly cycle materialises all seven positions");

        let structure = self
            .db()
            .time_model()
            .create_period_structure(&PeriodStructureInput {
                school_id: school,
                campus_id: None,
                name: "Standard day".to_owned(),
                is_default: true,
            })
            .expect("create bell schedule")
            .id;

        // Three teaching, a break, three teaching: six in the day, three in a row.
        for (ordinal, kind, start, end) in [
            (1, PeriodKind::Teaching, "09:00", "09:50"),
            (2, PeriodKind::Teaching, "09:55", "10:45"),
            (3, PeriodKind::Teaching, "10:50", "11:40"),
            (4, PeriodKind::Break, "11:40", "12:20"),
            (5, PeriodKind::Teaching, "12:20", "13:10"),
            (6, PeriodKind::Teaching, "13:15", "14:05"),
            (7, PeriodKind::Teaching, "14:10", "15:00"),
        ] {
            self.db()
                .time_model()
                .create_period(&PeriodInput {
                    period_structure_id: structure,
                    ordinal,
                    label: format!("P{ordinal}"),
                    starts_at: start.to_owned(),
                    ends_at: end.to_owned(),
                    kind,
                    counts_as_load: kind == PeriodKind::Teaching,
                })
                .expect("create period");
        }
    }

    /// The simplest complete lesson: a subject, a course, a section, at `school`.
    fn simple_lesson(&mut self, school: Uuid) -> (Uuid, Uuid, Uuid) {
        let subject = self.subject(school, "Mathematics");
        let course = self.course(school, subject, "Year 9 Mathematics", Some(4));
        let section = self.section(school, course, "A", None);
        (subject, course, section)
    }

    /// A lesson with one of everything on it: staff, audience, roster, occurrence.
    ///
    /// The fixture the two revision-coverage tests share, so each of them is a list of
    /// assertions rather than a page of setup followed by one.
    fn staffed_lesson(&mut self, school: Uuid) -> Staffed {
        let (subject, course, section) = self.simple_lesson(school);
        let teacher = self.teacher(school, "Teaching");
        let group = self.group(school, "9A", Some(28));
        let student = self.student(school, "Named");
        self.db()
            .lessons()
            .assign_class_teacher(school, section, teacher)
            .expect("staff it");
        self.db()
            .lessons()
            .assign_class_group(school, section, group)
            .expect("fill it");
        self.db()
            .lessons()
            .enrol_in_class(school, section, student)
            .expect("roster it");
        let activity = self
            .db()
            .lessons()
            .add_activities(school, section, None, 1, 1)
            .expect("one occurrence")
            .remove(0);
        Staffed {
            subject,
            course,
            section,
            teacher,
            group,
            student,
            activity: activity.id,
        }
    }
}

/// One lesson with one of everything on it. See [`Trust::staffed_lesson`].
struct Staffed {
    subject: Uuid,
    course: Uuid,
    section: Uuid,
    teacher: Uuid,
    group: Uuid,
    student: Uuid,
    activity: Uuid,
}

/// Asserts that a mutation refused a stale revision, and refused it as a *conflict*.
///
/// Named rather than inlined because the distinction is the assertion: `NotFound` would
/// send the user looking for a record that is in fact there, so a mutation that reported
/// the wrong one of the two would pass a weaker test.
fn conflicts(error: &DbError, what: &str) {
    assert!(
        matches!(error, DbError::Conflict { .. }),
        "{what} must refuse a stale revision with Conflict, got {error:?}"
    );
}

/// A revision nobody is at.
const STALE: Expected = Expected::Rev(999);

fn course_input(subject: Uuid, name: &str, periods: Option<i64>) -> CourseInput {
    CourseInput {
        subject_id: subject,
        year_level_id: None,
        term_id: None,
        name: name.to_owned(),
        code: None,
        description: None,
        periods_per_cycle: periods,
        default_duration_periods: 1,
        required_room_type_id: None,
    }
}

// ======================================================================= the migration

#[test]
fn the_six_tables_arrive_with_their_change_log_triggers() {
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

    for table in LESSON_TABLES {
        for operation in ["insert", "update", "delete"] {
            let expected = format!("trg_{table}_changelog_{operation}");
            assert!(
                triggers.contains(&expected),
                "{expected} is missing, so {table} is absent from the change feed"
            );
        }
    }
}

/// The six tables `V15` adds, in the order the migration creates them.
const LESSON_TABLES: &[&str] = &[
    "course",
    "class_section",
    "class_teacher",
    "class_group",
    "class_enrolment",
    "activity",
];

#[test]
fn the_new_tables_are_registered_for_change_tracking() {
    for table in LESSON_TABLES {
        assert!(
            crate::change_log::TRACKED_TABLES.contains(table),
            "`{table}` must be in TRACKED_TABLES or `install_all` will never create its triggers"
        );
    }
}

#[test]
fn the_migration_invents_no_lessons() {
    // The claim in `V15`'s header: additive, with no backfill. A project that has never
    // been asked what it teaches has no courses, and one course per subject would be
    // putting words in a school's mouth.
    let mut trust = Trust::new();
    let north = trust.north;
    let _subject = trust.subject(north, "Mathematics");
    let _second = trust.subject(north, "Physics");

    let counts: Vec<i64> = trust
        .db()
        .read(|conn| {
            let mut found = Vec::new();
            for table in LESSON_TABLES {
                found.push(conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table}"),
                    [],
                    |row| row.get::<_, i64>(0),
                )?);
            }
            Ok(found)
        })
        .expect("count lesson rows");

    assert_eq!(
        counts,
        vec![0, 0, 0, 0, 0, 0],
        "two subjects and no courses is the ordinary state of a school part-way through \
         setup, and V15 must leave it that way"
    );
}

#[test]
fn a_project_at_the_previous_schema_keeps_every_row_and_revision() {
    // `V15` rebuilds nothing. A project migrated by it describes exactly the same schools
    // it did before, with the same identifiers and — the part a backfill would break — the
    // same revisions.
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let path = dir.path().join(format!("before.{PROJECT_EXTENSION}"));

    let (school_id, subject_id, before) = {
        let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
        workspace.create_project(&path).expect("create project");
        let db = workspace.open_database_for_test();
        let school = db
            .structure()
            .create_school(&SchoolInput {
                name: "Northgate Secondary".to_owned(),
                code: Some("NGS".to_owned()),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("create school");
        let subject = db
            .curriculum()
            .create_subject(&SubjectInput {
                school_id: school.id,
                department_id: None,
                name: "Mathematics".to_owned(),
                code: Some("MAT".to_owned()),
                description: None,
            })
            .expect("create subject");
        // Edited once, so its revision is 2 and a backfill that touched it would show.
        let subject = db
            .curriculum()
            .update_subject(
                subject.id,
                Expected::Rev(subject.rev),
                &SubjectInput {
                    school_id: school.id,
                    department_id: None,
                    name: "Mathematics".to_owned(),
                    code: Some("MATH".to_owned()),
                    description: None,
                },
            )
            .expect("update subject");
        assert_eq!(subject.rev, 2);
        (school.id, subject.id, subject)
    };

    // Reopening runs every pending migration. The project above was created by this same
    // binary, so to prove the *upgrade* path specifically the test drops V15's record and
    // its tables, then reopens — which is how `tests_curriculum` proves the same thing.
    {
        let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
        workspace.open_project(&path).expect("open project");
        let db = workspace.open_database_for_test();
        db.write(|tx| {
            // V15 also puts a trigger on `teacher_school`, which is V13's table and so
            // survives the rewind. Dropping it is part of getting back to V14.
            tx.execute_batch(
                "DROP TRIGGER IF EXISTS teacher_school_delete_requires_no_class_assignment",
            )?;
            for table in LESSON_TABLES.iter().rev() {
                tx.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))?;
            }
            // V16 goes too. refinery reports a *gap* rather than a rewind if a later
            // migration stays applied while an earlier one's record is removed — "migration
            // V15 is missing from the filesystem" — so the history has to come back to V14
            // contiguously. Reopening then re-runs V15 and V16 together, which is the real
            // upgrade path a V14 project takes today.
            tx.execute_batch(
                "DROP TABLE IF EXISTS constraint_ref;
                 DROP TABLE IF EXISTS constraint_instance;
                 ALTER TABLE student_group DROP COLUMN children_are_disjoint",
            )?;
            tx.execute(
                "DELETE FROM refinery_schema_history WHERE version IN (15, 16)",
                [],
            )?;
            Ok(())
        })
        .expect("rewind to V14");
    }

    let mut workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");
    workspace.open_project(&path).expect("reopen and migrate");
    let db = workspace.open_database_for_test();

    let after = db
        .curriculum()
        .subject(subject_id)
        .expect("read subject")
        .expect("the subject is still there");
    assert_eq!(
        after, before,
        "V15 rewrote a row it had no business touching"
    );

    let schools = db.structure().schools().expect("list schools");
    assert_eq!(schools.len(), 1);
    assert_eq!(schools[0].id, school_id);

    // And the new tables are back, with their triggers, because `install_all` runs on
    // every open rather than inside the migration.
    let courses = db.lessons().courses(school_id).expect("list courses");
    assert!(courses.is_empty(), "no backfill invented a course");
}

#[test]
fn the_migration_is_forward_only_and_names_no_down_path() {
    // There are no down migrations, ever (CLAUDE.md). A file that contained one would make
    // the "forward-only" claim in its own header false.
    // Comment lines are stripped first. The header *discusses* ALTER TABLE at length --
    // naming each deferred column as "one nullable ALTER TABLE away, the cheap direction"
    // -- and a test that read the prose would be asserting that the file says nothing
    // rather than that it does nothing.
    let statements: String = V15_SQL
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(
        !statements.contains("drop table"),
        "V15 must not drop a table: a released migration is immutable and there is no down path"
    );
    assert!(
        !statements.contains("alter table"),
        "V15 is purely additive; an ALTER here would mean it rebuilds something"
    );
    assert!(
        !statements.contains("insert into"),
        "V15 has no backfill: inventing a course would be putting words in a school mouth"
    );
    assert!(
        V15_SQL.to_lowercase().contains("forward-only"),
        "the header must say so, because that is what a reader checks first"
    );
}

// ============================================================ the chain carries the school

#[test]
fn a_course_has_no_school_column_and_resolves_one_through_its_subject() {
    // The claim the whole of V15's ownership design rests on, and ROADMAP.md's Phase 3
    // note stated it in advance: resolve the school through `subject.school_id` rather
    // than adding a second `school_id` to `course`.
    let mut trust = Trust::new();
    let columns: Vec<String> = trust
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('course')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .expect("read course columns");

    assert!(
        !columns.contains(&"school_id".to_owned()),
        "a second place to record ownership is a second place for it to be wrong"
    );

    for table in [
        "class_section",
        "class_teacher",
        "class_group",
        "class_enrolment",
        "activity",
    ] {
        let columns: Vec<String> = trust
            .db()
            .read(|conn| {
                let mut stmt =
                    conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .expect("read columns");
        assert!(
            !columns.contains(&"school_id".to_owned()),
            "{table} must resolve its school through the chain, not hold a copy"
        );
    }
}

#[test]
fn a_course_cannot_be_built_on_another_schools_subject() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let southern = trust.subject(south, "Mathematics");

    let error = trust
        .db()
        .lessons()
        .create_course(
            north,
            &course_input(southern, "Year 9 Mathematics", Some(4)),
        )
        .expect_err("Northgate may not build a curriculum on Southmoor's subject");
    assert!(
        matches!(error, DbError::WrongSchool { .. }),
        "got {error:?}"
    );
}

#[test]
fn two_schools_may_each_offer_a_course_of_the_same_name_and_code() {
    // The multi-school point of the screen, one level down from Stage B's. Two schools'
    // Mathematics are two subjects, so two offerings of them are two courses — and the
    // code uniqueness is scoped to the school, so both may use "9MAT".
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let n_subject = trust.subject(north, "Mathematics");
    let s_subject = trust.subject(south, "Mathematics");

    let mut northern = course_input(n_subject, "Year 9 Mathematics", Some(4));
    northern.code = Some("9MAT".to_owned());
    let mut southern = course_input(s_subject, "Year 9 Mathematics", Some(4));
    southern.code = Some("9MAT".to_owned());

    let a = trust
        .db()
        .lessons()
        .create_course(north, &northern)
        .expect("Northgate's own 9MAT");
    let b = trust
        .db()
        .lessons()
        .create_course(south, &southern)
        .expect("Southmoor's own 9MAT, which is a different course");
    assert_ne!(a.id, b.id, "nothing merges them on a matching code");

    assert_eq!(trust.db().lessons().courses(north).expect("list").len(), 1);
    assert_eq!(trust.db().lessons().courses(south).expect("list").len(), 1);
}

#[test]
fn a_course_code_is_unique_within_the_school_across_subjects() {
    // Scoped to the school and not to the subject, which is why `V15` holds it as a trigger
    // pair rather than a partial unique index. Both halves are asserted: the repository's
    // typed error here, and the trigger underneath it below.
    let mut trust = Trust::new();
    let north = trust.north;
    let maths = trust.subject(north, "Mathematics");
    let physics = trust.subject(north, "Physics");

    let mut first = course_input(maths, "Year 9 Mathematics", Some(4));
    first.code = Some("9CORE".to_owned());
    trust
        .db()
        .lessons()
        .create_course(north, &first)
        .expect("the first 9CORE");

    let mut second = course_input(physics, "Year 9 Physics", Some(3));
    second.code = Some("9CORE".to_owned());
    let error = trust
        .db()
        .lessons()
        .create_course(north, &second)
        .expect_err("a course code identifies one course within one school");
    assert!(
        matches!(
            error,
            DbError::DuplicateValue {
                field: "code",
                entity: "course"
            }
        ),
        "the message must name the Code field, not the Name field: got {error:?}"
    );
}

#[test]
fn the_schema_refuses_a_duplicate_course_code_underneath_the_repository() {
    // The trigger half. `V14` established the division of labour: the repository names the
    // field and the remedy, the trigger is what makes the invariant true for a future
    // repository, the CLI, an import or a hand-repair.
    let mut trust = Trust::new();
    let north = trust.north;
    let maths = trust.subject(north, "Mathematics");
    let physics = trust.subject(north, "Physics");
    let mut first = course_input(maths, "Year 9 Mathematics", Some(4));
    first.code = Some("9CORE".to_owned());
    trust
        .db()
        .lessons()
        .create_course(north, &first)
        .expect("the first 9CORE");

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO course (id, subject_id, year_level_id, term_id, name, code,
                 description, periods_per_cycle, default_duration_periods,
                 required_room_type_id, created_at, updated_at, rev)
             VALUES (?, ?, NULL, NULL, 'Smuggled', '9CORE', NULL, NULL, 1, NULL, ?, ?, 1)",
            rusqlite::params![
                Uuid::now_v7().to_string(),
                physics.to_string(),
                STAMP,
                STAMP
            ],
        )?;
        Ok(())
    });
    assert!(
        refused.is_err(),
        "a hand-written duplicate must be refused by the schema, not only by the repository"
    );
}

#[test]
fn a_courses_year_level_term_and_room_type_must_be_its_own_schools() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let subject = trust.subject(north, "Mathematics");
    let foreign_year = trust.year_level(south, "Year 9", 9);
    let foreign_room = trust.room_type(south, "Laboratory", "LAB");

    let mut with_year = course_input(subject, "Year 9 Mathematics", Some(4));
    with_year.year_level_id = Some(foreign_year);
    let error = trust
        .db()
        .lessons()
        .create_course(north, &with_year)
        .expect_err("another school's year level is not a choice here");
    // `Invalid` and not `WrongSchool`: the write is for the right school and one field in
    // it names somebody else's row, so the remedy is to pick a different year level rather
    // than to switch schools.
    assert!(
        matches!(error, DbError::Invalid { .. }),
        "got {error:?} — the remedy is to pick a different year level, not to switch school"
    );

    let mut with_room = course_input(subject, "Year 9 Mathematics", Some(4));
    with_room.required_room_type_id = Some(foreign_room);
    let error = trust
        .db()
        .lessons()
        .create_course(north, &with_room)
        .expect_err("another school's room type is not a requirement this school can set");
    assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
}

#[test]
fn the_schema_refuses_a_cross_school_year_level_underneath_the_repository() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let subject = trust.subject(north, "Mathematics");
    let foreign_year = trust.year_level(south, "Year 9", 9);

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO course (id, subject_id, year_level_id, term_id, name, code,
                 description, periods_per_cycle, default_duration_periods,
                 required_room_type_id, created_at, updated_at, rev)
             VALUES (?, ?, ?, NULL, 'Smuggled', NULL, NULL, NULL, 1, NULL, ?, ?, 1)",
            rusqlite::params![
                Uuid::now_v7().to_string(),
                subject.to_string(),
                foreign_year.to_string(),
                STAMP,
                STAMP
            ],
        )?;
        Ok(())
    });
    assert!(
        refused.is_err(),
        "the trigger pair must hold without the repository"
    );
}

#[test]
fn a_section_cannot_be_created_or_edited_from_another_school() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, course, section) = trust.simple_lesson(north);

    let error = trust
        .db()
        .lessons()
        .create_class_section(
            south,
            &ClassSectionInput {
                course_id: course,
                name: "B".to_owned(),
                code: None,
                campus_id: None,
            },
        )
        .expect_err("Southmoor may not add a section to Northgate's course");
    assert!(
        matches!(error, DbError::WrongSchool { .. }),
        "got {error:?}"
    );

    let held = trust
        .db()
        .lessons()
        .class_section(section)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .lessons()
        .update_class_section(
            south,
            section,
            Expected::Rev(held.rev),
            &ClassSectionInput {
                course_id: course,
                name: "Renamed".to_owned(),
                code: None,
                campus_id: None,
            },
        )
        .expect_err("nor rename it");
    assert!(
        matches!(error, DbError::WrongSchool { .. }),
        "got {error:?}"
    );
}

#[test]
fn an_activity_resolves_its_school_three_tables_up() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let made = trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 1)
        .expect("one occurrence");

    let error = trust
        .db()
        .lessons()
        .delete_activity(south, made[0].id, Expected::Rev(made[0].rev))
        .expect_err("Southmoor may not delete Northgate's occurrence");
    assert!(
        matches!(error, DbError::WrongSchool { .. }),
        "the chain subject -> course -> class_section -> activity must carry the refusal: \
         got {error:?}"
    );
}

#[test]
fn a_sections_campus_must_belong_to_its_own_school() {
    let mut trust = Trust::new();
    let (north, south_main) = (trust.north, trust.south_main);
    let (_subject, course, _section) = trust.simple_lesson(north);

    let error = trust
        .db()
        .lessons()
        .create_class_section(
            north,
            &ClassSectionInput {
                course_id: course,
                name: "B".to_owned(),
                code: None,
                campus_id: Some(south_main),
            },
        )
        .expect_err("a Northgate section is not taught at a Southmoor campus");
    assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
}

// ================================================ assignment: two refusals and a warning

#[test]
fn an_unqualified_teacher_can_be_assigned_and_is_reported_as_unqualified() {
    // ADR 0028's whole substance. ADR 0025 §4 asked Stage C for a conjunction of three
    // things; this is the one that became advisory, and the test has to prove both halves:
    // the assignment succeeds, AND the absence is still visible.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let cover = trust.teacher(north, "Cover");

    let assignment = trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, cover)
        .expect("a school covering an absent specialist must be able to say so");

    let assigned = trust
        .db()
        .lessons()
        .class_teachers(north, section)
        .expect("read staffing");
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].assignment.id, assignment.id);
    assert!(
        !assigned[0].qualified,
        "the warning must survive the save, or it is not a warning but a lie"
    );
}

#[test]
fn a_qualified_teacher_is_reported_as_qualified() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (subject, _course, section) = trust.simple_lesson(north);
    let specialist = trust.teacher(north, "Specialist");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, specialist, subject)
        .expect("qualify");

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, specialist)
        .expect("assign");

    let assigned = trust
        .db()
        .lessons()
        .class_teachers(north, section)
        .expect("read staffing");
    assert!(assigned[0].qualified);
}

#[test]
fn a_teacher_the_school_does_not_employ_is_refused() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let visitor = trust.teacher(south, "Visitor");

    let error = trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, visitor)
        .expect_err("existing in the project is not being employed here");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(
        message.contains("Add this teacher to the school"),
        "the message must name the remedy: {message}"
    );
}

#[test]
fn a_teacher_whose_membership_is_inactive_keeps_their_classes_and_gets_no_new_ones() {
    // ADR 0018's sentence, enforced as a pair: "keeps every assignment they ever had, and
    // simply stops being offered for new ones". Both halves, in one test, because either
    // on its own would be the wrong behaviour.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, course, first) = trust.simple_lesson(north);
    let leaver = trust.teacher(north, "Leaver");
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, first, leaver)
        .expect("assigned while active");

    let membership = trust
        .db()
        .membership()
        .teacher_memberships(leaver)
        .expect("memberships")
        .into_iter()
        .find(|m| m.school_id == north)
        .expect("a home membership");
    trust
        .db()
        .membership()
        .set_teacher_membership_active(north, leaver, Expected::Rev(membership.rev), false)
        .expect("record that they have left");

    // Half one: the existing assignment is untouched.
    let still = trust
        .db()
        .lessons()
        .class_teachers(north, first)
        .expect("read staffing");
    assert_eq!(
        still.len(),
        1,
        "deactivating a membership must not silently unstaff a class"
    );
    assert!(!still[0].membership.is_active, "and the screen can see why");

    // Half two: no new ones.
    let second = trust.section(north, course, "B", None);
    let error = trust
        .db()
        .lessons()
        .assign_class_teacher(north, second, leaver)
        .expect_err("somebody recorded as having left gets no new classes");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(
        message.contains("no longer teaching"),
        "the message must say which of the four reasons it is: {message}"
    );
}

#[test]
fn the_schema_refuses_an_inactive_members_assignment_underneath_the_repository() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let visitor = trust.teacher(trust.south, "Visitor");

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO class_teacher (id, class_section_id, teacher_id,
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, 1)",
            rusqlite::params![
                Uuid::now_v7().to_string(),
                section.to_string(),
                visitor.to_string(),
                STAMP,
                STAMP
            ],
        )?;
        Ok(())
    });
    assert!(
        refused.is_err(),
        "a non-member assignment must be refused by the schema, not only by the repository"
    );
}

#[test]
fn a_teacher_narrowed_to_one_campus_cannot_be_given_a_section_at_another() {
    let mut trust = Trust::new();
    let (north, main, junior) = (trust.north, trust.north_main, trust.north_junior);
    let (_subject, course, _first) = trust.simple_lesson(north);
    let senior_only = trust.section(north, course, "Senior", Some(main));
    let junior_section = trust.section(north, course, "Junior", Some(junior));

    let teacher = trust.teacher(north, "JuniorOnly");
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, teacher, junior)
        .expect("narrow them to the junior campus");

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, junior_section, teacher)
        .expect("the campus they are eligible at");

    let error = trust
        .db()
        .lessons()
        .assign_class_teacher(north, senior_only, teacher)
        .expect_err("and not the one they are not");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(
        message.contains("not eligible at the campus"),
        "the message must name the campus rule: {message}"
    );
}

#[test]
fn a_teacher_with_no_campus_eligibility_recorded_is_not_narrowed() {
    // `teacher_campus` is a NARROWING (ADR 0021 §3), so an empty set narrows nothing.
    // Reading it the other way would make every teacher in a multi-campus school
    // unassignable until somebody filled the table in — and would have broken every
    // project V13 migrated.
    let mut trust = Trust::new();
    let (north, main) = (trust.north, trust.north_main);
    let (_subject, course, _first) = trust.simple_lesson(north);
    let section = trust.section(north, course, "Senior", Some(main));
    let unnarrowed = trust.teacher(north, "Anywhere");

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, unnarrowed)
        .expect("no eligibility recorded means no narrowing, not no eligibility");
}

#[test]
fn a_section_with_no_campus_constrains_nobody() {
    let mut trust = Trust::new();
    let (north, junior) = (trust.north, trust.north_junior);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let narrowed = trust.teacher(north, "JuniorOnly");
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, narrowed, junior)
        .expect("narrow them");

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, narrowed)
        .expect("a section with no campus is not campus-constrained");
}

#[test]
fn a_section_cannot_be_moved_to_a_campus_that_would_strand_its_staff() {
    // The other direction of the campus rule. V15's trigger pair fires on a write to
    // `class_teacher`, not to `class_section`, so without the repository's check a school
    // could move a section to a campus its own staff are not eligible at.
    let mut trust = Trust::new();
    let (north, main, junior) = (trust.north, trust.north_main, trust.north_junior);
    let (_subject, course, _first) = trust.simple_lesson(north);
    let section = trust.section(north, course, "Floating", None);
    let narrowed = trust.teacher(north, "JuniorOnly");
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, narrowed, junior)
        .expect("narrow them");
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, narrowed)
        .expect("assignable while the section has no campus");

    let held = trust
        .db()
        .lessons()
        .class_section(section)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .lessons()
        .update_class_section(
            north,
            section,
            Expected::Rev(held.rev),
            &ClassSectionInput {
                course_id: course,
                name: "Floating".to_owned(),
                code: None,
                campus_id: Some(main),
            },
        )
        .expect_err("moving the section must not strand the teacher on it");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(message.contains("not eligible at that campus"), "{message}");

    // And it may still be moved to the campus they *are* eligible at.
    trust
        .db()
        .lessons()
        .update_class_section(
            north,
            section,
            Expected::Rev(held.rev),
            &ClassSectionInput {
                course_id: course,
                name: "Floating".to_owned(),
                code: None,
                campus_id: Some(junior),
            },
        )
        .expect("the junior campus is fine");
}

#[test]
fn a_shared_teacher_may_teach_at_both_schools_and_stays_one_teacher() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_n_subject, _n_course, n_section) = trust.simple_lesson(north);
    let s_subject = trust.subject(south, "Mathematics");
    let s_course = trust.course(south, s_subject, "Year 9 Mathematics", Some(4));
    let s_section = trust.section(south, s_course, "A", None);

    let shared = trust.teacher(north, "Shared");
    trust.employ(south, shared);

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, n_section, shared)
        .expect("Northgate's class");
    trust
        .db()
        .lessons()
        .assign_class_teacher(south, s_section, shared)
        .expect("and Southmoor's");

    let taught = trust
        .db()
        .lessons()
        .sections_of_teacher(shared)
        .expect("project-wide listing");
    assert_eq!(taught.len(), 2, "both are true and a screen must see both");
    assert_eq!(
        trust.db().people().teachers(north).expect("teachers").len(),
        1,
        "one real person is one teacher row, however many schools"
    );
}

#[test]
fn the_same_teacher_cannot_be_assigned_twice() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let teacher = trust.teacher(north, "Once");
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect("first");
    let error = trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect_err("saying it twice says nothing more");
    assert!(
        matches!(error, DbError::DuplicateValue { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_section_may_have_several_teachers() {
    // Co-teaching is ordinary, and DATA-MODEL.md names the prototype's single `teacherId`
    // as the shortcut to avoid.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    for family in ["First", "Second", "Third"] {
        let teacher = trust.teacher(north, family);
        trust
            .db()
            .lessons()
            .assign_class_teacher(north, section, teacher)
            .expect("assign");
    }
    assert_eq!(
        trust
            .db()
            .lessons()
            .class_teachers(north, section)
            .expect("read")
            .len(),
        3
    );
}

#[test]
fn the_picker_reports_every_member_including_the_ones_it_would_refuse() {
    // ADR 0023 §5's rule: visible, explained, not silently editable. Somebody expected and
    // missing is worse than somebody shown and disabled, because the second explains itself.
    let mut trust = Trust::new();
    let (north, main, junior) = (trust.north, trust.north_main, trust.north_junior);
    let (subject, course, _first) = trust.simple_lesson(north);
    let section = trust.section(north, course, "Senior", Some(main));

    let qualified = trust.teacher(north, "Qualified");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, qualified, subject)
        .expect("qualify");

    let unqualified = trust.teacher(north, "Unqualified");

    let elsewhere = trust.teacher(north, "JuniorOnly");
    trust
        .db()
        .membership()
        .assign_teacher_campus(north, elsewhere, junior)
        .expect("narrow them away from this section's campus");

    let archived = trust.teacher(north, "Archived");
    let held = trust
        .db()
        .people()
        .teacher(archived)
        .expect("read")
        .expect("there");
    trust
        .db()
        .people()
        .update_teacher(
            archived,
            Expected::Rev(held.rev),
            &TeacherInput {
                is_active: false,
                ..input_of(&held)
            },
        )
        .expect("archive them");

    let candidates = trust
        .db()
        .lessons()
        .assignable_teachers(north, section)
        .expect("read candidates");
    assert_eq!(candidates.len(), 4, "every member is offered or explained");

    let find = |id: Uuid| {
        candidates
            .iter()
            .find(|c| c.teacher.id == id)
            .expect("in the listing")
    };

    assert!(find(qualified).eligibility.is_allowed());
    assert!(find(qualified).eligibility.qualified);

    // The warning case: allowed, and reported as unqualified.
    assert!(find(unqualified).eligibility.is_allowed());
    assert!(!find(unqualified).eligibility.qualified);

    // The two refusals.
    assert!(!find(elsewhere).eligibility.campus_eligible);
    assert!(!find(elsewhere).eligibility.is_allowed());
    assert!(!find(archived).eligibility.employed_and_active);
    assert!(!find(archived).eligibility.is_allowed());
}

/// Rebuilds a teacher's input from the stored record, for an edit that changes one field.
fn input_of(teacher: &crate::repo::people::Teacher) -> TeacherInput {
    TeacherInput {
        school_id: teacher.school_id,
        code: teacher.code.clone(),
        given_name: teacher.given_name.clone(),
        family_name: teacher.family_name.clone(),
        email: teacher.email.clone(),
        home_campus_id: teacher.home_campus_id,
        fte_percent: teacher.fte_percent,
        max_periods_per_day: teacher.max_periods_per_day,
        max_periods_per_cycle: teacher.max_periods_per_cycle,
        max_consecutive_periods: teacher.max_consecutive_periods,
        notes: teacher.notes.clone(),
        is_active: teacher.is_active,
    }
}

#[test]
fn a_qualification_is_untouched_by_everything_this_stage_does() {
    // ADR 0025's ownership model must not be weakened by ADR 0028's amendment. Nothing in
    // Stage C writes or removes a qualification.
    let mut trust = Trust::new();
    let north = trust.north;
    let (subject, _course, section) = trust.simple_lesson(north);
    let teacher = trust.teacher(north, "Specialist");
    trust
        .db()
        .curriculum()
        .qualify_teacher(north, teacher, subject)
        .expect("qualify");

    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect("assign");
    let held = trust
        .db()
        .lessons()
        .class_teachers(north, section)
        .expect("read")
        .remove(0);
    trust
        .db()
        .lessons()
        .remove_class_teacher(north, section, teacher, Expected::Rev(held.assignment.rev))
        .expect("unassign");

    assert_eq!(
        trust
            .db()
            .curriculum()
            .qualifications_of_teacher(teacher)
            .expect("read")
            .len(),
        1,
        "assigning and unassigning a class says nothing about what somebody can teach"
    );
}

#[test]
fn a_membership_cannot_be_withdrawn_while_its_classes_remain() {
    // The asymmetry ADR 0029 §7 argues: campus eligibility and department associations are
    // cleared by withdrawing a membership, and class assignments refuse it.
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let shared = trust.teacher(south, "Shared");
    trust.employ(north, shared);
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, shared)
        .expect("assign");

    let error = trust
        .db()
        .membership()
        .remove_teacher_from_school(north, shared)
        .expect_err("the useful answer names what is in the way");
    let DbError::Invalid { message } = error else {
        panic!("expected a counted remedy, got {error:?}");
    };
    assert!(
        message.contains("still teaches 1"),
        "the count is the point of the message: {message}"
    );

    // Removed from the section, the membership goes.
    let held = trust
        .db()
        .lessons()
        .class_teachers(north, section)
        .expect("read")
        .remove(0);
    trust
        .db()
        .lessons()
        .remove_class_teacher(north, section, shared, Expected::Rev(held.assignment.rev))
        .expect("unassign");
    trust
        .db()
        .membership()
        .remove_teacher_from_school(north, shared)
        .expect("and now it may be withdrawn");
}

#[test]
fn the_schema_refuses_a_membership_withdrawal_that_would_orphan_an_assignment() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let shared = trust.teacher(south, "Shared");
    trust.employ(north, shared);
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, shared)
        .expect("assign");

    let refused = trust.db().write(|tx| {
        tx.execute(
            "DELETE FROM teacher_school WHERE teacher_id = ? AND school_id = ?",
            [shared.to_string(), north.to_string()],
        )?;
        Ok(())
    });
    assert!(
        refused.is_err(),
        "V15's trigger is what makes the refusal a property of the schema"
    );
}

#[test]
fn deleting_the_person_does_take_their_assignments() {
    // The other side of the same line: withdrawing one school's employment is an edit, and
    // destroying the person is not. ADR 0025 §5's judgement about qualifications.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let teacher = trust.teacher(north, "Departing");
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect("assign");

    let held = trust
        .db()
        .people()
        .teacher(teacher)
        .expect("read")
        .expect("there");
    trust
        .db()
        .people()
        .delete_teacher(north, teacher, Expected::Rev(held.rev))
        .expect("an assignment with nobody to hold it is a dangling fact");

    assert!(
        trust
            .db()
            .lessons()
            .class_teachers(north, section)
            .expect("read")
            .is_empty(),
        "the section is unstaffed, and that is visible rather than broken"
    );
}

// ================================================= groups, rosters and double counting

#[test]
fn a_lesson_needs_no_students_at_all() {
    // ADR 0006's requirement, which this stage must not quietly cost: "A school must be
    // able to build a full timetable using only `student_group.size`, without entering a
    // single student."
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", Some(28));

    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("a declared size is a complete audience");
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 4)
        .expect("and the demand can be defined against it");

    let view = trust
        .db()
        .lessons()
        .class_sections(north, course)
        .expect("read")
        .remove(0);
    assert_eq!(view.groups, 1);
    assert_eq!(view.group_population, 28);
    assert_eq!(
        view.enrolled_students, 0,
        "and no student record was needed"
    );
    assert_eq!(view.demand.periods_per_cycle, 4);

    assert_eq!(
        trust.db().people().students(north).expect("students").len(),
        0,
        "nothing created a student to make this work"
    );
}

#[test]
fn a_group_without_a_declared_size_contributes_its_counted_membership() {
    // "NULL means membership is the truth; a number means there are this many and I am not
    // listing them" — V10's own words for the column.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", None);
    for family in ["One", "Two", "Three"] {
        let student = trust.student(north, family);
        trust
            .db()
            .people()
            .add_student_group_member(&StudentGroupMemberInput {
                student_group_id: group,
                student_id: student,
            })
            .expect("add to group");
    }
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("attend");

    let attending = trust
        .db()
        .lessons()
        .class_groups(north, section)
        .expect("read")
        .remove(0);
    assert_eq!(attending.named_members, 3);
    assert_eq!(attending.population, 3, "the membership is the truth here");
}

#[test]
fn a_group_size_and_a_roster_are_never_added_together() {
    // ADR 0029 §4. A section taught to a group of 28 that also names four children has an
    // audience of 28, because those four are among the 28. This is the test that pins the
    // two figures apart.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", Some(28));
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("attend");

    let mut named = Vec::new();
    for family in ["One", "Two", "Three", "Four"] {
        let student = trust.student(north, family);
        trust
            .db()
            .people()
            .add_student_group_member(&StudentGroupMemberInput {
                student_group_id: group,
                student_id: student,
            })
            .expect("they are in the group");
        trust
            .db()
            .lessons()
            .enrol_in_class(north, section, student)
            .expect("and named on the roster");
        named.push(student);
    }

    let view = trust
        .db()
        .lessons()
        .class_sections(north, course)
        .expect("read")
        .remove(0);
    assert_eq!(
        view.group_population, 28,
        "the scheduling population is the group's, and naming four of them changes nothing"
    );
    assert_eq!(view.enrolled_students, 4);
    assert_ne!(
        view.group_population, 32,
        "28 + 4 is the wrong answer and no field may ever hold it: those four ARE four of          the twenty-eight"
    );

    let roster = trust
        .db()
        .lessons()
        .class_enrolments(north, section)
        .expect("read roster");
    assert!(
        roster.iter().all(|r| r.covered_by_group),
        "and each row says it is covered, rather than the overlap being silent"
    );
}

#[test]
fn a_rostered_student_no_attending_group_covers_is_reported_as_such() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", Some(28));
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("attend");

    let outsider = trust.student(north, "Outsider");
    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, outsider)
        .expect("named without being in the group");

    let roster = trust
        .db()
        .lessons()
        .class_enrolments(north, section)
        .expect("read");
    assert_eq!(roster.len(), 1);
    assert!(
        !roster[0].covered_by_group,
        "somebody on the roster whom no attending group covers is worth seeing"
    );
}

#[test]
fn a_group_from_another_school_cannot_attend() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let foreign = trust.group(south, "9A", Some(28));

    let error = trust
        .db()
        .lessons()
        .assign_class_group(north, section, foreign)
        .expect_err("a Northgate section is not taught to Southmoor's children");
    assert!(
        matches!(error, DbError::WrongSchool { .. }),
        "got {error:?}"
    );
}

#[test]
fn the_schema_refuses_a_cross_school_group_underneath_the_repository() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let foreign = trust.group(south, "9A", Some(28));

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO class_group (id, class_section_id, student_group_id,
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, 1)",
            rusqlite::params![
                Uuid::now_v7().to_string(),
                section.to_string(),
                foreign.to_string(),
                STAMP,
                STAMP
            ],
        )?;
        Ok(())
    });
    assert!(refused.is_err(), "the trigger pair must hold on its own");
}

#[test]
fn a_section_may_be_taught_to_several_groups() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, course, section) = trust.simple_lesson(north);
    for (name, size) in [("9A", 28), ("9B", 26)] {
        let group = trust.group(north, name, Some(size));
        trust
            .db()
            .lessons()
            .assign_class_group(north, section, group)
            .expect("a combined cohort is ordinary");
    }

    let view = trust
        .db()
        .lessons()
        .class_sections(north, course)
        .expect("read")
        .remove(0);
    assert_eq!(view.groups, 2);
    assert_eq!(view.group_population, 54);
}

#[test]
fn the_same_group_cannot_attend_twice() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", Some(28));
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("first");
    let error = trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect_err("second");
    assert!(
        matches!(error, DbError::DuplicateValue { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_student_another_school_enrolled_is_not_eligible_here() {
    // The multi-school safety ADR 0029 §4 is most careful about: a project-wide identity
    // exists the moment *any* school enrolled them, and that is not eligibility here.
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let southern = trust.student(south, "Southern");

    let error = trust
        .db()
        .lessons()
        .enrol_in_class(north, section, southern)
        .expect_err("existing in the project is not being enrolled here");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(
        message.contains("not currently enrolled at this school"),
        "the message must name the remedy: {message}"
    );
}

#[test]
fn a_concurrently_enrolled_student_may_be_put_in_either_schools_classes() {
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_n_subject, _n_course, n_section) = trust.simple_lesson(north);
    let s_subject = trust.subject(south, "Mathematics");
    let s_course = trust.course(south, s_subject, "Year 9 Mathematics", Some(4));
    let s_section = trust.section(south, s_course, "A", None);

    let shared = trust.student(north, "Shared");
    trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            student_id: shared,
            school_id: south,
            admission_no: None,
            year_level_id: None,
            campus_id: None,
            status: EnrolmentStatus::Enrolled,
            enrolled_on: None,
            left_on: None,
        })
        .expect("a second, concurrent spell");

    trust
        .db()
        .lessons()
        .enrol_in_class(north, n_section, shared)
        .expect("Northgate's class");
    trust
        .db()
        .lessons()
        .enrol_in_class(south, s_section, shared)
        .expect("and Southmoor's");

    assert_eq!(
        trust.db().people().students(north).expect("students").len(),
        1,
        "one real child is one student row"
    );
}

#[test]
fn a_prospective_student_may_be_put_on_a_roster() {
    // V13 describes PROSPECTIVE as "an applicant: recorded, placed, not yet attending",
    // and a school sectioning next September's intake is doing precisely that. A rule
    // admitting only ENROLLED would refuse the main reason to build a roster in advance.
    let mut trust = Trust::new();
    let (north, south) = (trust.north, trust.south);
    let (_subject, _course, section) = trust.simple_lesson(north);
    let applicant = trust.student(south, "Applicant");
    trust
        .db()
        .membership()
        .enrol_student(&StudentEnrolmentInput {
            student_id: applicant,
            school_id: north,
            admission_no: None,
            year_level_id: None,
            campus_id: None,
            status: EnrolmentStatus::Prospective,
            enrolled_on: None,
            left_on: None,
        })
        .expect("record the application");

    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, applicant)
        .expect("an applicant can be placed in a class before term starts");
}

#[test]
fn leaving_a_school_keeps_the_record_of_which_classes_they_were_in() {
    // ADR 0018's rule: where the dependent row is the history, it is kept and the remedy is
    // made visible rather than performed. "She was in 9MAT-A" is history of that kind.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let student = trust.student(north, "Leaver");
    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, student)
        .expect("on the roster");

    let spell = trust
        .db()
        .membership()
        .current_enrolment_at(north, student)
        .expect("read spell")
        .expect("there is one");
    trust
        .db()
        .membership()
        .leave_school(north, student, Expected::Rev(spell.rev), None)
        .expect("record that they have left");

    assert_eq!(
        trust
            .db()
            .lessons()
            .class_enrolments(north, section)
            .expect("read")
            .len(),
        1,
        "a departure is not a reason to erase what they attended"
    );
}

#[test]
fn the_same_student_cannot_be_rostered_twice() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let student = trust.student(north, "Once");
    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, student)
        .expect("first");
    let error = trust
        .db()
        .lessons()
        .enrol_in_class(north, section, student)
        .expect_err("second");
    assert!(
        matches!(error, DbError::DuplicateValue { .. }),
        "got {error:?}"
    );
}

// ======================================================================= demand is rows

#[test]
fn four_single_periods_are_four_rows_and_one_double_is_one() {
    // ADR 0029 §2, and the arithmetic that falls out of it: the demand is a SUM over rows,
    // which no single multiplication produces for a mixed section.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);

    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 4)
        .expect("four singles");
    trust
        .db()
        .lessons()
        .add_activities(north, section, Some("Laboratory"), 2, 1)
        .expect("and one double");

    let rows = trust
        .db()
        .lessons()
        .activities(north, section)
        .expect("read occurrences");
    assert_eq!(rows.len(), 5, "five occurrences, not two requirements");
    assert_eq!(
        rows.iter().map(|a| a.split_index).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5],
        "numbered once, in order, within the section"
    );

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read demand");
    assert_eq!(demand.demand.occurrences, 5);
    assert_eq!(demand.demand.periods_per_cycle, 6, "4 x 1 + 1 x 2");
    assert_eq!(demand.demand.longest_occurrence, 2);
}

#[test]
fn the_generator_expands_a_courses_declaration_into_occurrences() {
    // ROADMAP.md's Phase 3 "activity generator": expand `periods_per_cycle` into activities
    // respecting `default_duration_periods`.
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Physics");
    let mut input = course_input(subject, "Year 10 Physics", Some(4));
    input.default_duration_periods = 2;
    let course = trust
        .db()
        .lessons()
        .create_course(north, &input)
        .expect("create course")
        .id;
    let section = trust.section(north, course, "Laboratory 1", None);

    let made = trust
        .db()
        .lessons()
        .generate_activities(north, section)
        .expect("generate");
    assert_eq!(made.len(), 2, "four periods at duration 2 is two doubles");
    assert!(made.iter().all(|a| a.duration_periods == 2));

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read demand");
    assert_eq!(demand.demand.periods_per_cycle, 4);
    assert_eq!(demand.declared, DeclaredComparison::Matches);
}

#[test]
fn the_generator_refuses_to_double_a_sections_demand() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    trust
        .db()
        .lessons()
        .generate_activities(north, section)
        .expect("first run");
    let error = trust
        .db()
        .lessons()
        .generate_activities(north, section)
        .expect_err("pressed twice must not silently double the demand");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(message.contains("already has occurrences"), "{message}");
}

#[test]
fn the_generator_invents_no_number_when_the_course_declares_none() {
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Mathematics");
    let course = trust.course(north, subject, "Year 9 Mathematics", None);
    let section = trust.section(north, course, "A", None);

    let error = trust
        .db()
        .lessons()
        .generate_activities(north, section)
        .expect_err("nothing here guesses how often a class should meet");
    let DbError::Invalid { message } = error else {
        panic!("expected a named remedy, got {error:?}");
    };
    assert!(
        message.contains("does not say how many periods"),
        "{message}"
    );
}

#[test]
fn a_declaration_that_does_not_divide_is_refused_rather_than_rounded() {
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Physics");
    let mut input = course_input(subject, "Year 10 Physics", Some(5));
    input.default_duration_periods = 2;
    let course = trust
        .db()
        .lessons()
        .create_course(north, &input)
        .expect("create")
        .id;
    let section = trust.section(north, course, "A", None);

    let error = trust
        .db()
        .lessons()
        .generate_activities(north, section)
        .expect_err("five periods does not divide into doubles");
    assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
}

#[test]
fn a_sections_demand_may_differ_from_its_courses_declaration_and_is_only_reported() {
    // ADR 0029 §3: the declaration is advice, never a constraint. A section may
    // legitimately need more or less than its course's default.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north); // declares 4
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 3)
        .expect("three, not four — and this must succeed");

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read");
    assert_eq!(
        demand.declared,
        DeclaredComparison::Short {
            declared: 4,
            shortfall: 1
        }
    );

    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 2)
        .expect("and five is equally permitted");
    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read");
    assert_eq!(
        demand.declared,
        DeclaredComparison::Over {
            declared: 4,
            excess: 1
        }
    );
}

#[test]
fn an_occurrence_needs_at_least_one_period() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);

    for bad in [0, -1] {
        let error = trust
            .db()
            .lessons()
            .add_activities(north, section, None, bad, 1)
            .expect_err("a zero-period lesson is not a lesson");
        assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
    }
    let error = trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 0)
        .expect_err("adding none of something is not an operation");
    assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
}

#[test]
fn the_schema_refuses_a_zero_period_occurrence_underneath_the_repository() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);

    let refused = trust.db().write(|tx| {
        tx.execute(
            "INSERT INTO activity (id, class_section_id, name, duration_periods, split_index,
                 created_at, updated_at, rev)
             VALUES (?, ?, NULL, 0, 1, ?, ?, 1)",
            rusqlite::params![
                Uuid::now_v7().to_string(),
                section.to_string(),
                STAMP,
                STAMP
            ],
        )?;
        Ok(())
    });
    assert!(refused.is_err(), "the CHECK must hold on its own");
}

#[test]
fn deleting_one_occurrence_does_not_renumber_the_others() {
    // Renumbering would rewrite rows the user did not touch and consume their revisions,
    // which would make a concurrent edit to one of them fail for no visible reason.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let made = trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 4)
        .expect("four");

    trust
        .db()
        .lessons()
        .delete_activity(north, made[1].id, Expected::Rev(made[1].rev))
        .expect("delete the second");

    let left = trust
        .db()
        .lessons()
        .activities(north, section)
        .expect("read");
    assert_eq!(
        left.iter().map(|a| a.split_index).collect::<Vec<_>>(),
        vec![1, 3, 4],
        "the gap is deliberate; the interface numbers what it displays"
    );
    assert!(
        left.iter().all(|a| a.rev == 1),
        "no surviving occurrence had its revision consumed by somebody else's delete"
    );

    // And a new one continues past the high-water mark rather than filling the gap.
    let next = trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 1)
        .expect("one more");
    assert_eq!(next[0].split_index, 5);
}

#[test]
fn a_mistyped_occurrence_count_is_refused_rather_than_written() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let error = trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 400)
        .expect_err("a typo guard, not a scheduling rule");
    assert!(matches!(error, DbError::Invalid { .. }), "got {error:?}");
    assert!(
        trust
            .db()
            .lessons()
            .activities(north, section)
            .expect("read")
            .is_empty(),
        "and nothing was written"
    );
}

// ========================================================= what can and cannot be said

#[test]
fn an_occurrence_longer_than_any_unbroken_run_is_reported_as_impossible() {
    // Structurally impossible, and therefore honest to say: six teaching periods a day but
    // never more than three in a row, so a four-period occurrence has nowhere to go.
    let mut trust = Trust::new();
    let north = trust.north;
    trust.ordinary_time_model(north);
    let (_subject, _course, section) = trust.simple_lesson(north);
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 4, 1)
        .expect("the demand is accepted; it is the placement that cannot exist");

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read");
    assert_eq!(demand.capacity.cycle_days, 7);
    assert_eq!(demand.capacity.teaching_periods_per_day, 6);
    assert_eq!(demand.capacity.longest_teaching_run, 3);
    assert_eq!(
        demand.findings,
        vec![DemandFinding::OccurrenceLongerThanAnyRun {
            needed: 4,
            longest_run: 3
        }]
    );
}

#[test]
fn a_demand_that_is_merely_difficult_is_not_reported_at_all() {
    // Nothing here claims an activity IS schedulable, and nothing here calls a hard demand
    // impossible. Only a solver can tell the difference, and there is not one yet.
    let mut trust = Trust::new();
    let north = trust.north;
    trust.ordinary_time_model(north);
    let (_subject, _course, section) = trust.simple_lesson(north);
    // 40 of a possible 42 slots: almost certainly infeasible once clashes, rooms and staff
    // availability are accounted for, and nothing here pretends to know.
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 40)
        .expect("accepted");

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read");
    assert!(
        demand.findings.is_empty(),
        "difficulty is not impossibility, and claiming otherwise would be a guess: {:?}",
        demand.findings
    );
}

#[test]
fn nothing_is_judged_before_the_school_has_a_time_model() {
    // Lessons is section 10 of School Setup and the cycle is section 14 (ADR 0024), so
    // entering lessons first is the expected path and must not be told it is wrong.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 4, 10)
        .expect("accepted");

    let demand = trust
        .db()
        .lessons()
        .section_demand(north, section)
        .expect("read");
    assert!(!demand.capacity.is_known());
    assert!(
        demand.findings.is_empty(),
        "zeroes are an unanswered question, not a cycle with no room in it"
    );
}

// ========================================================================== deletion

#[test]
fn a_subject_with_courses_cannot_be_deleted_and_the_count_says_so() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (subject, _course, _section) = trust.simple_lesson(north);

    let dependents = trust
        .db()
        .curriculum()
        .subject_dependents(subject)
        .expect("read dependents");
    assert_eq!(dependents.courses, 1);
    assert_eq!(dependents.total, 1);

    let held = trust
        .db()
        .curriculum()
        .subject(subject)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .curriculum()
        .delete_subject(north, subject, Expected::Rev(held.rev))
        .expect_err("a course is an offering OF this subject");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_course_with_sections_cannot_be_deleted_and_the_count_says_so() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, course, _section) = trust.simple_lesson(north);

    let dependents = trust
        .db()
        .lessons()
        .course_dependents(course)
        .expect("read dependents");
    assert_eq!(dependents.sections, 1);

    let held = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .lessons()
        .delete_course(north, course, Expected::Rev(held.rev))
        .expect_err("a section has staff and a roster on it");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "got {error:?}"
    );
}

#[test]
fn deleting_a_section_takes_its_staffing_audience_roster_and_occurrences() {
    // The opposite treatment, and ADR 0029 §7 argues why: every one of those four things is
    // *about* this section and means nothing without it.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let teacher = trust.teacher(north, "Teaching");
    let group = trust.group(north, "9A", Some(28));
    let student = trust.student(north, "Named");
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect("staff it");
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("fill it");
    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, student)
        .expect("roster it");
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 4)
        .expect("and define its demand");

    let dependents = trust
        .db()
        .lessons()
        .class_section_dependents(section)
        .expect("read");
    assert_eq!(dependents.teachers, 1);
    assert_eq!(dependents.groups, 1);
    assert_eq!(dependents.enrolled_students, 1);
    assert_eq!(dependents.activities, 4);
    assert_eq!(dependents.total, 7, "and the confirmation can say so first");

    let held = trust
        .db()
        .lessons()
        .class_section(section)
        .expect("read")
        .expect("there");
    trust
        .db()
        .lessons()
        .delete_class_section(north, section, Expected::Rev(held.rev))
        .expect("the counts explain, they do not block");

    // The dependents went; the things they pointed at did not.
    let counts: Vec<i64> = trust
        .db()
        .read(|conn| {
            let mut found = Vec::new();
            for table in [
                "class_teacher",
                "class_group",
                "class_enrolment",
                "activity",
            ] {
                found.push(conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table}"),
                    [],
                    |row| row.get::<_, i64>(0),
                )?);
            }
            Ok(found)
        })
        .expect("count");
    assert_eq!(counts, vec![0, 0, 0, 0]);

    assert!(
        trust
            .db()
            .people()
            .teacher(teacher)
            .expect("read")
            .is_some(),
        "the teacher is a person, not a dependent of a class"
    );
    assert!(
        trust
            .db()
            .people()
            .student_group(group)
            .expect("read")
            .is_some(),
        "and the group is a group"
    );
}

#[test]
fn every_cascaded_removal_appears_in_the_change_feed() {
    // Not a schema-level CASCADE, which would not fire the change-log trigger at all. Each
    // removal goes through repo::delete, so each one is attributable.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 3)
        .expect("three occurrences");

    let before = trust.db().changes_since(0, 10_000).expect("feed").len();
    let held = trust
        .db()
        .lessons()
        .class_section(section)
        .expect("read")
        .expect("there");
    trust
        .db()
        .lessons()
        .delete_class_section(north, section, Expected::Rev(held.rev))
        .expect("delete");

    let after = trust.db().changes_since(0, 10_000).expect("feed");
    let added = &after[before..];
    let deletes = added
        .iter()
        .filter(|entry| entry.entity_table == "activity")
        .count();
    assert_eq!(
        deletes, 3,
        "three occurrences went, so the feed must record three removals"
    );
}

#[test]
fn a_student_group_a_class_is_taught_to_cannot_be_deleted() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let group = trust.group(north, "9A", Some(28));
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("attend");

    assert_eq!(
        trust
            .db()
            .lessons()
            .class_count_of_group(group)
            .expect("read count"),
        1,
        "and the confirmation can explain before the button"
    );

    let held = trust
        .db()
        .people()
        .student_group(group)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .people()
        .delete_student_group(north, group, Expected::Rev(held.rev))
        .expect_err("deleting the audience out from under a lesson");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_student_on_a_class_roster_cannot_be_deleted() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let student = trust.student(north, "Named");
    trust
        .db()
        .lessons()
        .enrol_in_class(north, section, student)
        .expect("roster");

    assert_eq!(
        trust
            .db()
            .lessons()
            .class_count_of_student(student)
            .expect("read count"),
        1
    );

    let held = trust
        .db()
        .people()
        .student(student)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .people()
        .delete_student(north, student, Expected::Rev(held.rev))
        .expect_err("where the dependent row IS the history, the delete is refused");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "got {error:?}"
    );
}

#[test]
fn deleting_a_year_level_leaves_the_course_and_bumps_its_revision() {
    // ADR 0010's "optional classification, therefore SET NULL" case, cleared by the
    // repository first so the affected course's `rev` stays truthful. R10 in OPEN-DECISIONS
    // is why it is not left to SQLite.
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Mathematics");
    let year = trust.year_level(north, "Year 9", 9);
    let mut input = course_input(subject, "Year 9 Mathematics", Some(4));
    input.year_level_id = Some(year);
    let course = trust
        .db()
        .lessons()
        .create_course(north, &input)
        .expect("create")
        .id;

    let held = trust
        .db()
        .people()
        .year_level(year)
        .expect("read")
        .expect("there");
    trust
        .db()
        .people()
        .delete_year_level(north, year, Expected::Rev(held.rev))
        .expect("a course outlives losing its year level");

    let after = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("still there");
    assert_eq!(after.year_level_id, None);
    assert_eq!(
        after.rev, 2,
        "cleared through repo::update, so the change-log entry is truthful"
    );
}

#[test]
fn deleting_a_room_type_leaves_the_course_and_bumps_its_revision() {
    let mut trust = Trust::new();
    let north = trust.north;
    let subject = trust.subject(north, "Physics");
    let lab = trust.room_type(north, "Laboratory", "LAB");
    let mut input = course_input(subject, "Year 10 Physics", Some(4));
    input.required_room_type_id = Some(lab);
    let course = trust
        .db()
        .lessons()
        .create_course(north, &input)
        .expect("create")
        .id;

    let held = trust
        .db()
        .structure()
        .room_type(lab)
        .expect("read")
        .expect("there");
    trust
        .db()
        .structure()
        .delete_room_type(north, lab, Expected::Rev(held.rev))
        .expect("delete");

    let after = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("still there");
    assert_eq!(after.required_room_type_id, None);
    assert_eq!(after.rev, 2);
}

#[test]
fn a_campus_a_class_section_is_taught_at_cannot_be_deleted() {
    let mut trust = Trust::new();
    let (north, junior) = (trust.north, trust.north_junior);
    let (_subject, course, _first) = trust.simple_lesson(north);
    let _section = trust.section(north, course, "Junior", Some(junior));

    let held = trust
        .db()
        .structure()
        .campus(junior)
        .expect("read")
        .expect("there");
    let error = trust
        .db()
        .structure()
        .delete_campus(north, junior, Expected::Rev(held.rev))
        .expect_err("a campus with lessons at it is in use");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "got {error:?}"
    );
}

// ====================================================================== revisions

#[test]
fn a_stale_course_save_is_refused_rather_than_winning() {
    // ADR 0027's contract, extended to every one of Stage C's mutable rows. The second
    // window to save must be refused, nothing must change, and no revision must be
    // consumed — all three, because the third is what a plausible-but-wrong implementation
    // gets wrong.
    let mut trust = Trust::new();
    let north = trust.north;
    let (subject, course, _section) = trust.simple_lesson(north);

    let read_by_both = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("there");

    let mut renamed = course_input(subject, "Year 9 Core Mathematics", Some(4));
    renamed.name = "Year 9 Core Mathematics".to_owned();
    trust
        .db()
        .lessons()
        .update_course(north, course, Expected::Rev(read_by_both.rev), &renamed)
        .expect("the first window saves");

    let mut other = course_input(subject, "Year 9 Mathematics", Some(5));
    other.name = "Year 9 Mathematics".to_owned();
    let error = trust
        .db()
        .lessons()
        .update_course(north, course, Expected::Rev(read_by_both.rev), &other)
        .expect_err("the second window must not overwrite the first");

    let DbError::Conflict {
        expected, actual, ..
    } = error
    else {
        panic!("a stale save must be a Conflict, not a NotFound: got {error:?}");
    };
    assert_eq!(expected, 1);
    assert_eq!(actual, 2);

    let after = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("there");
    assert_eq!(
        after.name, "Year 9 Core Mathematics",
        "the first edit survived"
    );
    assert_eq!(
        after.periods_per_cycle,
        Some(4),
        "and the refused one changed nothing"
    );
    assert_eq!(after.rev, 2, "a refused write consumes no revision");
}

#[test]
fn a_refused_write_appends_nothing_to_the_change_feed() {
    let mut trust = Trust::new();
    let north = trust.north;
    let (subject, course, _section) = trust.simple_lesson(north);
    let held = trust
        .db()
        .lessons()
        .course(course)
        .expect("read")
        .expect("there");

    let before = trust.db().changes_since(0, 10_000).expect("feed").len();
    let _ = trust
        .db()
        .lessons()
        .update_course(
            north,
            course,
            Expected::Rev(held.rev + 99),
            &course_input(subject, "Whatever", Some(4)),
        )
        .expect_err("refused");
    let after = trust.db().changes_since(0, 10_000).expect("feed").len();
    assert_eq!(
        before, after,
        "the statement matched nothing, so no trigger fired and the feed gained no entry"
    );
}

#[test]
fn every_stage_c_update_takes_a_revision() {
    // The claim is about coverage: a Stage C mutation that omitted the precondition would
    // be the only unguarded single-record write in the project. ADR 0027 §3's inventory is
    // the shape being extended, so the test enumerates rather than sampling.
    let mut trust = Trust::new();
    let north = trust.north;
    let lesson = trust.staffed_lesson(north);

    conflicts(
        &trust
            .db()
            .lessons()
            .update_course(
                north,
                lesson.course,
                STALE,
                &course_input(lesson.subject, "Renamed", Some(4)),
            )
            .expect_err("course update"),
        "update_course",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .update_class_section(
                north,
                lesson.section,
                STALE,
                &ClassSectionInput {
                    course_id: lesson.course,
                    name: "Renamed".to_owned(),
                    code: None,
                    campus_id: None,
                },
            )
            .expect_err("section update"),
        "update_class_section",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .update_activity(north, lesson.activity, STALE, None, 2)
            .expect_err("activity update"),
        "update_activity",
    );
}

#[test]
fn every_stage_c_removal_takes_a_revision() {
    // The other half of the inventory, including the three relationship removals. ADR 0027
    // §4's rule for those is about the interface rather than the schema — *pass the
    // revision wherever the screen already holds the row* — and every one of these rows is
    // displayed in a list the user is acting on.
    let mut trust = Trust::new();
    let north = trust.north;
    let lesson = trust.staffed_lesson(north);

    conflicts(
        &trust
            .db()
            .lessons()
            .delete_activity(north, lesson.activity, STALE)
            .expect_err("activity delete"),
        "delete_activity",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .remove_class_teacher(north, lesson.section, lesson.teacher, STALE)
            .expect_err("teacher removal"),
        "remove_class_teacher",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .remove_class_group(north, lesson.section, lesson.group, STALE)
            .expect_err("group removal"),
        "remove_class_group",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .remove_from_class(north, lesson.section, lesson.student, STALE)
            .expect_err("roster removal"),
        "remove_from_class",
    );
    conflicts(
        &trust
            .db()
            .lessons()
            .delete_class_section(north, lesson.section, STALE)
            .expect_err("section delete"),
        "delete_class_section",
    );

    // The course's own precondition is reached only once the section is gone, because
    // `delete_course` refuses a course that still has one and that refusal comes first.
    // Clearing the way is therefore part of the test rather than a detour: a
    // `StillReferenced` here would otherwise hide whether the revision is checked at all.
    let held = trust
        .db()
        .lessons()
        .class_section(lesson.section)
        .expect("read")
        .expect("there");
    trust
        .db()
        .lessons()
        .delete_class_section(north, lesson.section, Expected::Rev(held.rev))
        .expect("the section goes at its own revision");

    conflicts(
        &trust
            .db()
            .lessons()
            .delete_course(north, lesson.course, STALE)
            .expect_err("course delete"),
        "delete_course",
    );
}

#[test]
fn a_removal_of_something_that_was_never_there_is_not_found_rather_than_a_conflict() {
    // The two names need completely different responses from the interface. "That record no
    // longer exists" sends somebody looking for something that is there.
    let mut trust = Trust::new();
    let north = trust.north;
    let (_subject, _course, section) = trust.simple_lesson(north);
    let unassigned = trust.teacher(north, "Unassigned");

    let error = trust
        .db()
        .lessons()
        .remove_class_teacher(north, section, unassigned, Expected::Rev(1))
        .expect_err("they never taught it");
    assert!(matches!(error, DbError::NotFound { .. }), "got {error:?}");
}

#[test]
fn every_write_appears_in_the_change_feed() {
    let mut trust = Trust::new();
    let north = trust.north;
    let before = trust.db().changes_since(0, 10_000).expect("feed").len();

    let (_subject, _course, section) = trust.simple_lesson(north);
    let teacher = trust.teacher(north, "Teaching");
    let group = trust.group(north, "9A", Some(28));
    trust
        .db()
        .lessons()
        .assign_class_teacher(north, section, teacher)
        .expect("staff");
    trust
        .db()
        .lessons()
        .assign_class_group(north, section, group)
        .expect("fill");
    trust
        .db()
        .lessons()
        .add_activities(north, section, None, 1, 2)
        .expect("occurrences");

    let after = trust.db().changes_since(0, 10_000).expect("feed");
    let tables: Vec<&str> = after[before..]
        .iter()
        .map(|entry| entry.entity_table.as_str())
        .collect();
    for expected in [
        "course",
        "class_section",
        "class_teacher",
        "class_group",
        "activity",
    ] {
        assert!(
            tables.contains(&expected),
            "a write to {expected} is a real edit to a real project and belongs in the feed: \
             got {tables:?}"
        );
    }
}
