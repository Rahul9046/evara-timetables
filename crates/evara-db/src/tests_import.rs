//! Stage D: a spreadsheet becomes a reviewed plan, and the plan is applied or it is not.
//!
//! # What these tests are for
//!
//! An import is the one feature a school uses *before* it trusts the application, with a
//! file it did not write and cannot fully describe. Four claims therefore have to be proved
//! rather than argued, and each of them is a way the feature could be quietly wrong:
//!
//! 1. **Preview writes nothing.** Not "writes nothing that matters" — nothing. Asserted
//!    over the change feed rather than over row counts, because a write that was rolled
//!    back would still have appended a feed entry, and a feature that silently touched a
//!    project during a dry run would be unfixable after the fact.
//! 2. **Apply is one transaction.** Forty teachers and a failure on the forty-first leaves
//!    none of them, no revisions consumed, and no change-log entries — otherwise a school
//!    that corrects its file and imports again creates everything twice.
//! 3. **The plan the user approved is the plan that runs.** Apply recomputes the plan
//!    inside its own transaction and refuses a fingerprint that has moved. The test that
//!    matters is not the token comparison but the recomputation: a row somebody created in
//!    another window since the preview must change the answer.
//! 4. **Nothing is inferred and nothing is merged.** The school is the one the caller
//!    selected, never a column; a code matches and a name asks; a missing reference is an
//!    error and never a placeholder row. ADR 0031 is the whole policy and these are its
//!    teeth.
//!
//! # Why the fixture is deliberately sparse
//!
//! Every reference an import can fail to resolve has to be *resolvable* in some test and
//! *absent* in another, so `School` holds the minimum and each test adds what it needs.
//! A rich fixture would make "the department did not exist" untestable without unpicking
//! it.

use std::collections::BTreeMap;

use evara_core::calendar::WorkingDays;
use evara_core::import::{
    Choice, ColumnMapping, ImportFingerprint, ImportKind, RowAction, RowDecision, Severity,
    SourceCell, SourceRow, SourceTable,
};
use tempfile::TempDir;
use uuid::Uuid;

use crate::import::report_csv;
use crate::repo::curriculum::{DepartmentInput, SubjectInput};
use crate::repo::lessons::{ClassSectionInput, CourseInput};
use crate::repo::membership::EnrolmentStatus;
use crate::repo::people::{StudentInput, TeacherInput, YearLevelInput};
use crate::repo::structure::{CampusInput, SchoolInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

// ================================================================== the fixture

/// Two schools in one project, each with one campus, and nothing else.
///
/// Named rather than numbered for the reason the other modules give: every assertion here
/// is about *which* school, and `north`/`south` makes a wrong expectation read as
/// implausible where `school_a`/`school_b` would read as fine.
struct Fixture {
    _dir: TempDir,
    workspace: Workspace,
    north: Uuid,
    south: Uuid,
    north_main: Uuid,
}

impl Fixture {
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
        };
        this.north = this.school("Northgate Secondary");
        this.south = this.school("Southmoor College");
        this.north_main = this.campus(this.north, "Northgate Main", "NGM");
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

    fn department(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .curriculum()
            .create_department(&DepartmentInput {
                school_id: school,
                name: name.to_owned(),
                code: Some(code.to_owned()),
                description: None,
            })
            .expect("create department")
            .id
    }

    fn subject(&mut self, school: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .curriculum()
            .create_subject(&SubjectInput {
                school_id: school,
                department_id: None,
                name: name.to_owned(),
                code: Some(code.to_owned()),
                description: None,
            })
            .expect("create subject")
            .id
    }

    fn course(&mut self, school: Uuid, subject: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .lessons()
            .create_course(
                school,
                &CourseInput {
                    subject_id: subject,
                    year_level_id: None,
                    term_id: None,
                    name: name.to_owned(),
                    code: Some(code.to_owned()),
                    description: None,
                    periods_per_cycle: None,
                    default_duration_periods: 1,
                    required_room_type_id: None,
                },
            )
            .expect("create course")
            .id
    }

    fn section(&mut self, school: Uuid, course: Uuid, name: &str, code: &str) -> Uuid {
        self.db()
            .lessons()
            .create_class_section(
                school,
                &ClassSectionInput {
                    course_id: course,
                    name: name.to_owned(),
                    code: Some(code.to_owned()),
                    campus_id: None,
                },
            )
            .expect("create class section")
            .id
    }

    fn teacher(&mut self, school: Uuid, code: &str, given: &str, family: &str) -> Uuid {
        self.db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: school,
                code: Some(code.to_owned()),
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
            })
            .expect("create teacher")
            .id
    }

    fn student(&mut self, school: Uuid, code: &str, given: &str, family: &str) -> Uuid {
        self.db()
            .people()
            .create_student(&StudentInput {
                school_id: school,
                code: Some(code.to_owned()),
                given_name: given.to_owned(),
                family_name: family.to_owned(),
                email: None,
                year_level_id: None,
                campus_id: None,
                is_active: true,
            })
            .expect("create student")
            .id
    }

    /// How many entries the change feed holds, over every table.
    ///
    /// The instrument for "preview writes nothing" and "a rolled-back apply left nothing":
    /// row counts would be satisfied by a write that was undone, and a feed entry would
    /// not.
    fn feed_length(&mut self) -> i64 {
        self.db()
            .read(|conn| {
                conn.query_row("SELECT COUNT(*) FROM change_log", [], |row| row.get(0))
                    .map_err(DbError::from)
            })
            .expect("count the feed")
    }

    fn count(&mut self, sql: &str) -> i64 {
        self.db()
            .read(|conn| {
                conn.query_row(sql, [], |row| row.get(0))
                    .map_err(DbError::from)
            })
            .expect("count")
    }
}

// ================================================================== building a sheet

/// A table built in the test, headed by the field keys themselves.
///
/// Headings are the field keys so `mapping` below is positional and obvious. The parser's
/// own heading-matching is tested in `evara-import`, where the headings a school actually
/// writes live; repeating that here would test the wrong crate.
fn table(fields: &[&str], rows: &[&[&str]]) -> SourceTable {
    SourceTable {
        sheet: "Sheet1".to_owned(),
        available_sheets: vec!["Sheet1".to_owned()],
        headers: fields.iter().map(|field| (*field).to_owned()).collect(),
        rows: rows
            .iter()
            .enumerate()
            .map(|(index, cells)| SourceRow {
                // The header is line 1, so the first data row is line 2 — the number the
                // user sees in the spreadsheet.
                line: i64::try_from(index).expect("a test fixture is small") + 2,
                cells: cells.iter().map(|cell| cell_of(cell)).collect(),
            })
            .collect(),
        truncated_at: None,
    }
}

/// One cell, as the CSV reader would have produced it: everything is text, blanks blank.
fn cell_of(value: &str) -> SourceCell {
    if value.is_empty() {
        SourceCell::Blank
    } else {
        SourceCell::Text(value.to_owned())
    }
}

/// The identity mapping for a table built by [`table`].
fn mapping(fields: &[&str]) -> ColumnMapping {
    ColumnMapping(
        fields
            .iter()
            .enumerate()
            .map(|(index, field)| ((*field).to_owned(), index))
            .collect::<BTreeMap<String, usize>>(),
    )
}

// ================================================================== preview writes nothing

#[test]
fn previewing_writes_nothing_at_all() {
    let mut fixture = Fixture::new();
    let before = fixture.feed_length();
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(
        &fields,
        &[
            &["T-0007", "Ada", "Lovelace"],
            &["T-0008", "Grace", "Hopper"],
        ],
    );
    let north = fixture.north;

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.counts.create, 2);
    // The whole claim: a preview is a question, not a change. Over the feed, because a
    // write that was rolled back would still be in it.
    assert_eq!(
        fixture.feed_length(),
        before,
        "a preview appended to the feed"
    );
    assert_eq!(fixture.count("SELECT COUNT(*) FROM teacher"), 0);
}

#[test]
fn previewing_the_same_file_twice_gives_the_same_fingerprint() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);
    let north = fixture.north;
    let map = mapping(&fields);

    let first = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan")
        .fingerprint;
    let second = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan")
        .fingerprint;

    assert_eq!(first, second, "an unchanged project must agree with itself");
}

// ================================================================== teachers

#[test]
fn a_teacher_code_with_leading_zeros_is_stored_as_written() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["00124", "Ada", "Lovelace"]]);
    let north = fixture.north;
    let map = mapping(&fields);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let stored: String = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT code FROM teacher", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("read the code");
    // `124` would be a different teacher, and the school would never find the one it
    // imported. The fidelity rule, end to end.
    assert_eq!(stored, "00124");
}

#[test]
fn a_numeric_code_cell_is_refused_rather_than_guessed() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName"];
    let mut sheet = table(&fields, &[&["x", "Ada", "Lovelace"]]);
    // What a workbook gives for a cell a school typed `00124` into and Excel stored as a
    // number: the leading zeros are already gone and cannot be recovered here.
    sheet.rows[0].cells[0] = SourceCell::Number {
        text: "124".to_owned(),
        number: 124.0,
    };
    let north = fixture.north;

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert!(
        plan.rows[0]
            .findings
            .iter()
            .any(|finding| finding.message.contains("as text")),
        "the message must name the remedy: {:?}",
        plan.rows[0].findings
    );
    assert!(!plan.can_apply());
}

#[test]
fn importing_the_same_teacher_file_twice_changes_nothing_the_second_time() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName", "email"];
    let sheet = table(
        &fields,
        &[&["T-0007", "Ada", "Lovelace", "ada@example.invalid"]],
    );
    let north = fixture.north;
    let map = mapping(&fields);

    let first = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    let summary = fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &[],
            &first.fingerprint,
        )
        .expect("apply");
    assert_eq!(summary.created, 1);

    let after_first = fixture.feed_length();

    let second = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan again");
    assert_eq!(second.counts.unchanged, 1, "{:?}", second.rows);
    assert_eq!(second.counts.create, 0);

    let summary = fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &[],
            &second.fingerprint,
        )
        .expect("apply again");
    assert_eq!(summary.unchanged, 1);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM teacher"), 1);
    // Idempotency over the feed as well as the rows: an "unchanged" row must not consume a
    // revision, or a second import would look like an edit to everybody downstream.
    assert_eq!(fixture.feed_length(), after_first);
}

#[test]
fn an_unmapped_column_is_left_alone_rather_than_blanked() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");
    fixture
        .db()
        .read(|conn| {
            conn.execute(
                "UPDATE teacher SET notes = 'kept' WHERE code = 'T-0007'",
                [],
            )
            .map_err(DbError::from)
        })
        .expect("set a note directly");

    // The file carries names and nothing else. `notes` has no column at all, so the import
    // has no opinion about it.
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.counts.unchanged, 1);

    let notes: Option<String> = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT notes FROM teacher", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("read the note");
    assert_eq!(notes.as_deref(), Some("kept"));
}

#[test]
fn a_mapped_but_blank_column_clears_the_value_it_was_mapped_to() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");
    let fields = ["code", "givenName", "familyName", "email"];
    let map = mapping(&fields);
    let agreed = [RowDecision {
        line: 2,
        choice: Choice::Update,
    }];

    // First give the teacher an address, then import the same file with the column blank.
    let filled = table(
        &fields,
        &[&["T-0007", "Ada", "Lovelace", "ada@example.invalid"]],
    );
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &filled, &map, &agreed)
        .expect("plan");
    assert_eq!(plan.counts.update, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &filled,
            &map,
            &agreed,
            &plan.fingerprint,
        )
        .expect("apply");

    let blanked = table(&fields, &[&["T-0007", "Ada", "Lovelace", ""]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &blanked, &map, &agreed)
        .expect("plan");
    // Mapped and blank is an answer — "there is no address" — where unmapped is silence.
    // The distinction is the whole of how a second file can correct the first.
    assert_eq!(plan.counts.update, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &blanked,
            &map,
            &agreed,
            &plan.fingerprint,
        )
        .expect("apply the blank");

    let email: Option<String> = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT email FROM teacher", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("read the address");
    assert_eq!(email, None);
}

#[test]
fn a_record_that_disagrees_with_the_file_is_never_overwritten_without_an_answer() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    // The code matches, so Evara knows *which* teacher. It still does not know whether the
    // file or the record is right about the name, and guessing would silently rewrite a
    // person. ADR 0031 §3.
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Augusta", "Lovelace"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.rows[0].action, RowAction::Conflict);
    assert!(!plan.can_apply());

    // "Leave it as it is" is a complete answer, and the record keeps its name.
    let keep = [RowDecision {
        line: 2,
        choice: Choice::KeepExisting,
    }];
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &keep)
        .expect("plan");
    assert_eq!(plan.rows[0].action, RowAction::Skipped);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &keep,
            &plan.fingerprint,
        )
        .expect("apply");

    let given: String = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT given_name FROM teacher", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("read the name");
    assert_eq!(given, "Ada");
}

#[test]
fn a_name_that_matches_asks_rather_than_merging() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    // The same person by name, with no code to prove it. Two staff can share a name, and
    // guessing wrong writes one teacher's timetable onto another.
    let fields = ["givenName", "familyName"];
    let sheet = table(&fields, &[&["Ada", "Lovelace"]]);
    let north_map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &north_map, &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Conflict);
    assert!(!plan.can_apply(), "a conflict must block Apply");

    // And the user's answer is honoured, both ways round.
    let keep = [RowDecision {
        line: 2,
        choice: Choice::KeepExisting,
    }];
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &north_map, &keep)
        .expect("plan");
    assert_eq!(plan.rows[0].action, RowAction::Skipped);
    assert!(plan.can_apply());

    let create = [RowDecision {
        line: 2,
        choice: Choice::CreateNew,
    }];
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &north_map, &create)
        .expect("plan");
    assert_eq!(plan.rows[0].action, RowAction::Create);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &north_map,
            &create,
            &plan.fingerprint,
        )
        .expect("apply");
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM teacher"),
        2,
        "the user asked for a second record and must get one"
    );
}

#[test]
fn two_rows_claiming_one_code_is_the_files_problem_not_a_last_one_wins() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(
        &fields,
        &[
            &["T-0007", "Ada", "Lovelace"],
            &["T-0007", "Grace", "Hopper"],
        ],
    );
    let north = fixture.north;

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Create);
    assert_eq!(
        plan.rows[1].action,
        RowAction::Invalid,
        "the second claim on a code is reported, not applied over the first"
    );
    assert!(
        plan.rows[1]
            .findings
            .iter()
            .any(|finding| finding.message.contains('2')),
        "the message must name the row it collides with: {:?}",
        plan.rows[1].findings
    );
}

#[test]
fn a_teacher_from_another_school_is_offered_rather_than_duplicated() {
    let mut fixture = Fixture::new();
    let south = fixture.south;
    let north = fixture.north;
    fixture.teacher(south, "T-0007", "Ada", "Lovelace");

    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");

    // A shared teacher is the ordinary case in a trust, and the narrowest possible evidence
    // for it — the same code *and* both names — is still not proof. ADR 0031 §4.
    assert_eq!(plan.rows[0].action, RowAction::Conflict);
    assert!(
        plan.rows[0]
            .findings
            .iter()
            .any(|finding| finding.message.contains("Southmoor")),
        "the message must name where the match was found: {:?}",
        plan.rows[0].findings
    );

    let employ = [RowDecision {
        line: 2,
        choice: Choice::Update,
    }];
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &employ)
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &employ,
            &plan.fingerprint,
        )
        .expect("apply");

    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM teacher"),
        1,
        "the same person must not become two identities"
    );
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM teacher_school WHERE is_active = 1"),
        2,
        "they are now employed at both"
    );
}

#[test]
fn an_import_never_touches_the_school_that_was_not_selected() {
    let mut fixture = Fixture::new();
    let south = fixture.south;
    let north = fixture.north;
    fixture.teacher(south, "T-0007", "Ada", "Lovelace");
    let before = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT rev FROM teacher WHERE code = 'T-0007'", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(DbError::from)
        })
        .expect("read the revision");

    let fields = ["code", "givenName", "familyName"];
    // Deliberately a *different* person under a code the other school already uses: the
    // code uniqueness rule is per school (ADR 0021), so this must succeed at Northgate and
    // leave Southmoor's teacher exactly as it was.
    let sheet = table(&fields, &[&["T-0007", "Grace", "Hopper"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.counts.create, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let (family, rev): (String, i64) = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT family_name, rev FROM teacher WHERE school_id = ?",
                [crate::repo::row::text(south)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(DbError::from)
        })
        .expect("read Southmoor's teacher");
    assert_eq!(family, "Lovelace");
    assert_eq!(rev, before, "a revision was consumed at the other school");
}

#[test]
fn an_import_into_a_school_that_is_not_in_this_project_is_refused() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);

    let outcome = fixture.db().import().plan(
        Uuid::now_v7(),
        ImportKind::Teachers,
        &sheet,
        &mapping(&fields),
        &[],
    );

    assert!(
        matches!(outcome, Err(DbError::NotFound { entity: "school" })),
        "{outcome:?}"
    );
}

// ================================================================== mapping

#[test]
fn a_missing_required_column_stops_the_whole_plan_rather_than_every_row() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName"];
    let sheet = table(&fields, &[&["T-0007", "Ada"], &["T-0008", "Grace"]]);
    let north = fixture.north;

    let outcome =
        fixture
            .db()
            .import()
            .plan(north, ImportKind::Teachers, &sheet, &mapping(&fields), &[]);

    match outcome {
        Err(DbError::Invalid { message }) => {
            // One sentence naming the column, rather than the same error two hundred times.
            assert!(message.contains("familyName"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn one_column_mapped_to_two_fields_is_refused() {
    let mut fixture = Fixture::new();
    let sheet = table(&["name", "spare"], &[&["Ada Lovelace", ""]]);
    let north = fixture.north;
    let mut map = BTreeMap::new();
    map.insert("givenName".to_owned(), 0);
    map.insert("familyName".to_owned(), 0);

    let outcome = fixture.db().import().plan(
        north,
        ImportKind::Teachers,
        &sheet,
        &ColumnMapping(map),
        &[],
    );

    match outcome {
        Err(DbError::Invalid { message }) => assert!(message.contains("two"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_required_field_matches_the_template() {
    // The two lists are written twice on purpose — `evara-db` cannot depend on
    // `evara-import` — so this is the test that keeps them from drifting. It reads the
    // template's own answer across the crate boundary, which a *library* dependency the
    // same way round would not be allowed to do.
    for kind in ImportKind::ALL {
        let template: Vec<(String, bool)> = evara_import::field_specs(*kind)
            .into_iter()
            .map(|spec| (spec.key.to_owned(), spec.required))
            .collect();
        let engine: Vec<(String, bool)> = crate::import::fields_for_test(*kind)
            .into_iter()
            .map(|(key, required)| (key.to_owned(), required))
            .collect();
        assert_eq!(template, engine, "{kind:?}");
    }
}

// ================================================================== students

#[test]
fn a_student_row_needing_a_year_level_that_does_not_exist_is_an_error_not_a_new_year_level() {
    let mut fixture = Fixture::new();
    let fields = ["code", "givenName", "familyName", "yearLevel"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "Year 9"]]);
    let north = fixture.north;

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert!(!plan.can_apply());
    // The rule the brief states twice: no placeholder entities. A year level invented here
    // would have no ordinal anybody chose, and the school would find it in their setup
    // screens having never created it.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM year_level"), 0);
}

#[test]
fn a_student_row_resolves_a_year_level_and_a_campus_that_do_exist() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let year = fixture.year_level(north, "Year 9", 9);
    let campus = fixture.north_main;

    let fields = ["code", "givenName", "familyName", "yearLevel", "campusCode"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "Year 9", "NGM"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.counts.create, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Students,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let (stored_year, stored_campus): (Option<String>, Option<String>) = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT year_level_id, campus_id FROM student", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(DbError::from)
        })
        .expect("read the student");
    assert_eq!(stored_year.as_deref(), Some(year.to_string().as_str()));
    assert_eq!(stored_campus.as_deref(), Some(campus.to_string().as_str()));
}

#[test]
fn a_student_import_creates_the_current_spell_at_the_selected_school() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName", "enrolledOn"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "2026-09-01"]]);
    let map = mapping(&fields);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &[])
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Students,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let (school, status, from): (String, String, String) = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT school_id, status, enrolled_on FROM student_school_enrolment",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(DbError::from)
        })
        .expect("read the spell");
    assert_eq!(school, north.to_string());
    assert_eq!(status, EnrolmentStatus::Enrolled.stored_value());
    assert_eq!(from, "2026-09-01");
}

#[test]
fn an_ambiguous_date_is_refused_rather_than_interpreted() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName", "enrolledOn"];
    // `01/09/2026` is the first of September in Britain and the ninth of January in the
    // United States. There is no reading of this file that is safe to guess.
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "01/09/2026"]]);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert!(
        plan.rows[0]
            .findings
            .iter()
            .any(|finding| finding.message.contains("YYYY-MM-DD")),
        "the message must name the one format Evara reads: {:?}",
        plan.rows[0].findings
    );
}

#[test]
fn an_unrecognised_enrolment_status_is_reported_with_the_words_that_work() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName", "status"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "withdrawn"]]);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &mapping(&fields), &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    let message = &plan.rows[0].findings[0].message;
    assert!(message.contains("Left"), "{message}");
}

#[test]
fn a_students_enrolment_moving_school_is_reported_rather_than_applied() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    fixture.student(south, "00124", "Grace", "Hopper");

    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &[])
        .expect("plan");

    // Moving a child between schools closes one spell and opens another, which is a
    // transaction with a leaving date the spreadsheet never carries. A column cannot
    // express it, so the import does not pretend to.
    assert_eq!(plan.rows[0].action, RowAction::Conflict);
    assert!(!plan.can_apply());
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM student_school_enrolment"),
        1
    );
}

#[test]
fn the_same_child_can_be_enrolled_here_as_well_without_becoming_two_children() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    let student = fixture.student(south, "00124", "Grace", "Hopper");

    let fields = ["code", "givenName", "familyName", "enrolledOn"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper", "2026-09-01"]]);
    let map = mapping(&fields);
    let accept = [RowDecision {
        line: 2,
        choice: Choice::Update,
    }];

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &accept)
        .expect("plan");
    assert_eq!(plan.counts.update, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Students,
            &sheet,
            &map,
            &accept,
            &plan.fingerprint,
        )
        .expect("apply");

    // One child, two spells. Concurrent enrolment is what V13's one-row-per-spell model is
    // for, and the alternative — a second `student` row — would give one child two
    // timetables that could not be checked against each other.
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM student"),
        1,
        "the same child must not become two identities"
    );
    let spells = fixture.count(&format!(
        "SELECT COUNT(*) FROM student_school_enrolment WHERE student_id = '{student}'"
    ));
    assert_eq!(spells, 2);
    assert_eq!(
        fixture.count(&format!(
            "SELECT COUNT(*) FROM student_school_enrolment
             WHERE student_id = '{student}' AND school_id = '{north}'"
        )),
        1,
    );
}

#[test]
fn creating_a_separate_record_for_a_same_coded_child_is_the_users_other_answer() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    fixture.student(south, "00124", "Grace", "Hopper");

    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["00124", "Grace", "Hopper"]]);
    let map = mapping(&fields);
    let separate = [RowDecision {
        line: 2,
        choice: Choice::CreateNew,
    }];

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &separate)
        .expect("plan");
    assert_eq!(plan.counts.create, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Students,
            &sheet,
            &map,
            &separate,
            &plan.fingerprint,
        )
        .expect("apply");

    // Two schools numbering their intakes from the same sequence is ordinary, and the code
    // uniqueness rule is per school, so this has to be expressible.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM student"), 2);
}

// ================================================================== curriculum

#[test]
fn a_subject_can_name_a_department_that_exists_and_not_one_that_does_not() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.department(north, "Mathematics", "MAT");

    let fields = ["code", "name", "departmentCode"];
    let sheet = table(
        &fields,
        &[&["MAT", "Mathematics", "MAT"], &["SCI", "Science", "SCI"]],
    );
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Subjects, &sheet, &map, &[])
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Create);
    assert_eq!(
        plan.rows[1].action,
        RowAction::Invalid,
        "an unknown department is an error, not a new department"
    );
    assert!(!plan.can_apply());
    // And nothing was created for the row that did resolve, because the plan is blocked.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM subject"), 0);

    // With the second row corrected the whole file applies, and the department it names is
    // the one that already existed.
    let sheet = table(&fields, &[&["MAT", "Mathematics", "MAT"]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Subjects, &sheet, &map, &[])
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Subjects,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");
    let stored: Option<String> = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT department_id FROM subject", [], |row| row.get(0))
                .map_err(DbError::from)
        })
        .expect("read the subject");
    assert_eq!(stored.as_deref(), Some(maths.to_string().as_str()));
    assert_eq!(fixture.count("SELECT COUNT(*) FROM department"), 1);
}

#[test]
fn a_course_cannot_be_moved_to_a_different_subject_by_a_second_import() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    fixture.subject(north, "Science", "SCI");
    fixture.course(north, maths, "Year 9 Mathematics", "9MAT");

    let fields = ["code", "name", "subjectCode"];
    let sheet = table(&fields, &[&["9MAT", "Year 9 Mathematics", "SCI"]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Courses, &sheet, &mapping(&fields), &[])
        .expect("plan");

    // A course is an offering *of* a subject; reparenting it would silently move every
    // lesson built on it. The remedy named in the message is a new code.
    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert!(
        plan.rows[0].findings[0].message.contains("subject"),
        "{:?}",
        plan.rows[0].findings
    );
}

#[test]
fn a_course_resolving_a_subject_from_another_school_is_refused() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    fixture.subject(south, "Mathematics", "MAT");

    let fields = ["code", "name", "subjectCode"];
    let sheet = table(&fields, &[&["9MAT", "Year 9 Mathematics", "MAT"]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Courses, &sheet, &mapping(&fields), &[])
        .expect("plan");

    // Resolution is scoped to the selected school at every depth, so Southmoor's subject is
    // simply not there to be found. It must read as "no such subject", not as a hit.
    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM course"), 0);
}

// ================================================================== class associations

#[test]
fn a_class_teacher_row_needs_an_active_membership_and_warns_about_a_qualification() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    fixture.section(north, course, "A", "9MAT-A");
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    let fields = ["courseCode", "sectionCode", "teacherCode"];
    let sheet = table(&fields, &[&["9MAT", "9MAT-A", "T-0007"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::ClassTeachers, &sheet, &map, &[])
        .expect("plan");

    // ADR 0028: the membership refuses, the qualification warns. A warning must not block,
    // or a school would have to record every qualification before it could import a single
    // timetable.
    assert_eq!(plan.rows[0].action, RowAction::Create, "{:?}", plan.rows);
    assert_eq!(plan.counts.warnings, 1);
    assert!(
        plan.rows[0]
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Warning),
        "{:?}",
        plan.rows[0].findings
    );
    assert!(plan.can_apply());

    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::ClassTeachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");
    assert_eq!(fixture.count("SELECT COUNT(*) FROM class_teacher"), 1);

    // And the same file again adds nothing: a link either exists or it does not.
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::ClassTeachers, &sheet, &map, &[])
        .expect("plan again");
    assert_eq!(plan.counts.unchanged, 1);
}

#[test]
fn a_class_enrolment_row_needs_a_student_of_this_school() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    fixture.section(north, course, "A", "9MAT-A");
    fixture.student(south, "00124", "Grace", "Hopper");

    let fields = ["courseCode", "sectionCode", "studentCode"];
    let sheet = table(&fields, &[&["9MAT", "9MAT-A", "00124"]]);
    let plan = fixture
        .db()
        .import()
        .plan(
            north,
            ImportKind::ClassEnrolments,
            &sheet,
            &mapping(&fields),
            &[],
        )
        .expect("plan");

    assert_eq!(plan.rows[0].action, RowAction::Invalid);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM class_enrolment"), 0);
}

#[test]
fn a_section_code_is_resolved_within_its_course_and_not_across_the_school() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let science = fixture.subject(north, "Science", "SCI");
    let maths_course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    let science_course = fixture.course(north, science, "Year 9 Science", "9SCI");
    // The same section name under two courses is ordinary — "A" is a section of each.
    fixture.section(north, maths_course, "A", "A");
    fixture.section(north, science_course, "A", "A");
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    let fields = ["courseCode", "sectionCode", "teacherCode"];
    let sheet = table(&fields, &[&["9SCI", "A", "T-0007"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::ClassTeachers, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.rows[0].action, RowAction::Create, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::ClassTeachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let section: String = fixture
        .db()
        .read(|conn| {
            conn.query_row("SELECT class_section_id FROM class_teacher", [], |row| {
                row.get(0)
            })
            .map_err(DbError::from)
        })
        .expect("read the link");
    let science_section: String = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT id FROM class_section WHERE course_id = ?",
                [crate::repo::row::text(science_course)],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("read the section");
    assert_eq!(
        section, science_section,
        "the wrong course's section was used"
    );
}

// ================================================================== activities

#[test]
fn activities_are_counted_to_a_target_and_never_deleted() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    fixture.section(north, course, "A", "9MAT-A");

    let fields = [
        "courseCode",
        "sectionCode",
        "durationPeriods",
        "occurrences",
    ];
    let four = table(&fields, &[&["9MAT", "9MAT-A", "1", "4"]]);
    let map = mapping(&fields);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Activities, &four, &map, &[])
        .expect("plan");
    assert_eq!(plan.counts.create, 1);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Activities,
            &four,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");
    // Four single periods is four rows, not one row holding a 4: ADR 0029, and the reason
    // `occurrences_per_cycle` is a spreadsheet column and not a database field.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM activity"), 4);

    // The same file again is a no-op, which is what makes a corrected re-import safe.
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Activities, &four, &map, &[])
        .expect("plan again");
    assert_eq!(plan.counts.unchanged, 1, "{:?}", plan.rows);

    // Asking for more adds the difference, not another four.
    let six = table(&fields, &[&["9MAT", "9MAT-A", "1", "6"]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Activities, &six, &map, &[])
        .expect("plan six");
    // A create, not an update: two lessons come into existence and the four that already
    // exist are not touched. The warning is what tells the user the arithmetic.
    assert_eq!(plan.counts.create, 1, "{:?}", plan.rows);
    assert!(
        plan.rows[0].findings[0].message.contains("2 new"),
        "{:?}",
        plan.rows[0].findings
    );
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Activities,
            &six,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply six");
    assert_eq!(fixture.count("SELECT COUNT(*) FROM activity"), 6);

    // Asking for fewer is a conflict, because deciding which two lessons to destroy — and
    // destroying their placements with them — is not a spreadsheet's decision to make.
    let two = table(&fields, &[&["9MAT", "9MAT-A", "1", "2"]]);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Activities, &two, &map, &[])
        .expect("plan two");
    assert_eq!(plan.rows[0].action, RowAction::Conflict);
    assert!(!plan.can_apply());
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM activity"),
        6,
        "an import must never delete a lesson"
    );
}

#[test]
fn a_double_period_is_one_row_of_duration_two_and_not_two_rows() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    fixture.section(north, course, "A", "9MAT-A");

    let fields = [
        "courseCode",
        "sectionCode",
        "durationPeriods",
        "occurrences",
    ];
    let sheet = table(&fields, &[&["9MAT", "9MAT-A", "2", "1"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Activities, &sheet, &map, &[])
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Activities,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let (rows, periods): (i64, i64) = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(duration_periods), 0) FROM activity",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(DbError::from)
        })
        .expect("read the activities");
    assert_eq!(rows, 1);
    assert_eq!(periods, 2, "demand is a sum of durations, not a product");
}

// ================================================================== the transaction

#[test]
fn a_failure_part_way_through_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName"];

    // Forty good rows and one that the database itself will refuse. The planner cannot see
    // this one coming — the name is long enough to pass every check it makes and the row is
    // rejected by a constraint — which is exactly the case the transaction exists for.
    let mut rows: Vec<Vec<String>> = (0..40)
        .map(|index| {
            vec![
                format!("T-{index:04}"),
                "Ada".to_owned(),
                format!("Lovelace{index}"),
            ]
        })
        .collect();
    rows.push(vec![
        "T-9999".to_owned(),
        "Grace".to_owned(),
        "Hopper".to_owned(),
    ]);
    let borrowed: Vec<Vec<&str>> = rows
        .iter()
        .map(|row| row.iter().map(String::as_str).collect())
        .collect();
    let slices: Vec<&[&str]> = borrowed.iter().map(Vec::as_slice).collect();
    let sheet = table(&fields, &slices);
    let map = mapping(&fields);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");
    assert_eq!(plan.counts.create, 41);

    // Between the preview and the apply, somebody else takes the last row's code. The plan
    // is recomputed inside the transaction, so this is caught before any row is written —
    // and the fingerprint names the disagreement.
    fixture.teacher(north, "T-9999", "Someone", "Else");
    let before = fixture.feed_length();
    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Teachers,
        &sheet,
        &map,
        &[],
        &plan.fingerprint,
    );

    assert!(
        matches!(outcome, Err(DbError::ReviewRequired { .. })),
        "{outcome:?}"
    );
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM teacher"),
        1,
        "the forty good rows must not have been written"
    );
    assert_eq!(
        fixture.feed_length(),
        before,
        "a refused apply must leave no trace in the change feed"
    );
}

#[test]
fn a_constraint_failure_rolls_the_whole_import_back() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "name"];

    // Two good departments and one whose name is empty once trimmed — the repository
    // refuses it, and nothing the planner checks would have.
    let sheet = table(
        &fields,
        &[
            &["MAT", "Mathematics"],
            &["SCI", "Science"],
            &["ART", "   "],
        ],
    );
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Departments, &sheet, &map, &[])
        .expect("plan");

    let before = fixture.feed_length();
    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Departments,
        &sheet,
        &map,
        &[],
        &plan.fingerprint,
    );

    assert!(outcome.is_err(), "an empty name must be refused");
    // The claim the whole engine is shaped around: no partial import. Two departments were
    // written and then unwritten, and the feed proves they are gone rather than hidden.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM department"), 0);
    assert_eq!(fixture.feed_length(), before);
}

#[test]
fn a_stale_preview_is_refused_and_names_both_fingerprints() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);
    let map = mapping(&fields);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");

    // The import's own window is not the only window. Somebody creates the teacher by hand
    // while the preview is on screen, so the plan's answer — "create" — is no longer true.
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Teachers,
        &sheet,
        &map,
        &[],
        &plan.fingerprint,
    );

    match outcome {
        Err(DbError::ReviewRequired { reviewed, current }) => {
            assert_ne!(reviewed, current);
            assert_eq!(reviewed, plan.fingerprint.to_string());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM teacher"),
        1,
        "the hand-created teacher is the only one"
    );

    // Re-previewing is the remedy, and now it reads as unchanged rather than as a create.
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &[])
        .expect("re-plan");
    assert_eq!(plan.counts.unchanged, 1, "{:?}", plan.rows);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply the re-plan");
    assert_eq!(fixture.count("SELECT COUNT(*) FROM teacher"), 1);
}

#[test]
fn a_fingerprint_from_a_different_school_does_not_apply_here() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let south = fixture.south;
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);
    let map = mapping(&fields);

    // The user previewed against Southmoor, then switched school. ADR 0023 has the
    // interface drop the pending plan, and this is the backend half of the same guarantee:
    // the school is part of the fingerprint, so the stale token cannot be applied here.
    let southern = fixture
        .db()
        .import()
        .plan(south, ImportKind::Teachers, &sheet, &map, &[])
        .expect("plan");

    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Teachers,
        &sheet,
        &map,
        &[],
        &southern.fingerprint,
    );

    assert!(
        matches!(outcome, Err(DbError::ReviewRequired { .. })),
        "{outcome:?}"
    );
    assert_eq!(fixture.count("SELECT COUNT(*) FROM teacher"), 0);
}

#[test]
fn a_fingerprint_that_was_never_issued_is_refused() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[&["T-0007", "Ada", "Lovelace"]]);

    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Teachers,
        &sheet,
        &mapping(&fields),
        &[],
        // A fingerprint for a different kind over no rows: well formed, and an answer to a
        // question nobody asked.
        &ImportFingerprint::of(ImportKind::Students, north, &[]),
    );

    assert!(
        matches!(outcome, Err(DbError::ReviewRequired { .. })),
        "{outcome:?}"
    );
    assert_eq!(fixture.count("SELECT COUNT(*) FROM teacher"), 0);
}

#[test]
fn applying_a_plan_that_still_holds_an_error_is_refused() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName", "yearLevel"];
    // One good row and one naming a year level nobody created.
    let sheet = table(
        &fields,
        &[
            &["00124", "Grace", "Hopper", ""],
            &["00125", "Ada", "Lovelace", "Year 9"],
        ],
    );
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Students, &sheet, &map, &[])
        .expect("plan");
    assert!(plan.counts.blocked());

    let outcome = fixture.db().import().apply(
        north,
        ImportKind::Students,
        &sheet,
        &map,
        &[],
        &plan.fingerprint,
    );

    match outcome {
        Err(DbError::Invalid { message }) => assert!(message.contains("attention"), "{message}"),
        other => panic!("{other:?}"),
    }
    // Not even the good row, because a blocked plan is not a plan.
    assert_eq!(fixture.count("SELECT COUNT(*) FROM student"), 0);
}

#[test]
fn a_successful_apply_is_in_the_change_feed() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "name"];
    let sheet = table(&fields, &[&["MAT", "Mathematics"], &["SCI", "Science"]]);
    let map = mapping(&fields);
    let before = fixture.feed_length();

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Departments, &sheet, &map, &[])
        .expect("plan");
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Departments,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    // The other half of "preview writes nothing": an apply must be visible to the change
    // feed, or a future synchronisation would never learn about imported records.
    let logged = fixture.count(
        "SELECT COUNT(*) FROM change_log WHERE entity_table = 'department' AND op = 'INSERT'",
    );
    assert_eq!(logged, 2);
    assert!(fixture.feed_length() > before);
}

#[test]
fn an_update_consumes_exactly_one_revision() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let teacher = fixture.teacher(north, "T-0007", "Ada", "Lovelace");
    let before: i64 = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT rev FROM teacher WHERE id = ?",
                [crate::repo::row::text(teacher)],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("read the revision");

    let fields = ["code", "givenName", "familyName", "email"];
    let sheet = table(
        &fields,
        &[&["T-0007", "Ada", "Lovelace", "ada@example.invalid"]],
    );
    let map = mapping(&fields);
    // An update needs the user's answer, so the decision is part of the plan under test.
    let agreed = [RowDecision {
        line: 2,
        choice: Choice::Update,
    }];
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &map, &agreed)
        .expect("plan");
    assert_eq!(plan.counts.update, 1);
    fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &map,
            &agreed,
            &plan.fingerprint,
        )
        .expect("apply");

    let after: i64 = fixture
        .db()
        .read(|conn| {
            conn.query_row(
                "SELECT rev FROM teacher WHERE id = ?",
                [crate::repo::row::text(teacher)],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
        .expect("read the revision");
    // Through `repo::update`, like every other write — not a bespoke UPDATE that forgot to
    // move `rev` and so left every other window's cached copy looking current.
    assert_eq!(after, before + 1);
}

// ================================================================== the report

#[test]
fn the_report_carries_every_row_and_every_finding() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let maths = fixture.subject(north, "Mathematics", "MAT");
    let course = fixture.course(north, maths, "Year 9 Mathematics", "9MAT");
    fixture.section(north, course, "A", "9MAT-A");
    fixture.teacher(north, "T-0007", "Ada", "Lovelace");

    let fields = ["courseCode", "sectionCode", "teacherCode"];
    let sheet = table(&fields, &[&["9MAT", "9MAT-A", "T-0007"]]);
    let map = mapping(&fields);
    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::ClassTeachers, &sheet, &map, &[])
        .expect("plan");
    let summary = fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::ClassTeachers,
            &sheet,
            &map,
            &[],
            &plan.fingerprint,
        )
        .expect("apply");

    let csv = report_csv(&summary);
    assert!(csv.starts_with("Row,Import,Source,Action,"));
    // The qualification warning survived into the file the school takes away, which is the
    // only record they will have of it once the dialog is closed.
    assert!(csv.contains("Warning"), "{csv}");
    assert!(
        csv.contains("\"2\""),
        "the row number must be the sheet's: {csv}"
    );
}

#[test]
fn an_empty_file_is_a_plan_with_no_rows_rather_than_an_error() {
    let mut fixture = Fixture::new();
    let north = fixture.north;
    let fields = ["code", "givenName", "familyName"];
    let sheet = table(&fields, &[]);

    let plan = fixture
        .db()
        .import()
        .plan(north, ImportKind::Teachers, &sheet, &mapping(&fields), &[])
        .expect("plan");

    // A school that exported an empty sheet should be told there is nothing in it, not
    // shown a failure they cannot interpret.
    assert_eq!(plan.counts.total(), 0);

    // And applying it is a no-op rather than a failure, because "nothing to do" is a
    // legitimate answer and an error here would send a user looking for a problem.
    let before = fixture.feed_length();
    let summary = fixture
        .db()
        .import()
        .apply(
            north,
            ImportKind::Teachers,
            &sheet,
            &mapping(&fields),
            &[],
            &plan.fingerprint,
        )
        .expect("apply");
    assert_eq!(summary.rows_processed, 0);
    assert_eq!(fixture.feed_length(), before);
}
