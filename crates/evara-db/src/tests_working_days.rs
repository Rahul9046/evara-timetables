//! The school's working week, and what it must not touch.
//!
//! `evara_core::calendar` owns the set type and its arithmetic. This file owns the things
//! only a real project file can show:
//!
//! - the week survives a round trip through SQLite as the same set, for ordinary,
//!   Sunday-to-Thursday and six-day weeks alike
//! - a school row written before the column existed reads as Monday to Friday, which is
//!   the whole of the upgrade story for an existing project
//! - changing the week leaves every `calendar_day` row byte-identical — the advisory
//!   reports, and writes nothing
//! - `cycle.day_count` and the working week never meet: a six-day rotation across a
//!   five-day week produces no findings at all
//!
//! Real temporary project files throughout, never `:memory:`.

use evara_core::calendar::{WorkingDays, WorkingDaysError};
use uuid::Uuid;

use crate::repo::structure::SchoolInput;
use crate::repo::time_model::{CalendarDayInput, CalendarDayKind, CycleDayInput};
use crate::tests_time_model::Project;
use crate::{Database, DbError};

/// Writes raw SQL straight past the repositories.
///
/// Used here for the one thing the repositories cannot do: write a `school` row as a build
/// without this column would have written it, and write a working-day value the schema
/// forbids. Same technique and the same justification as `tests_structure::corrupt`.
fn raw(db: &mut Database, sql: &str) {
    db.write(|tx| {
        tx.execute_batch(sql)?;
        Ok(())
    })
    .expect("raw SQL must itself succeed");
}

/// The school's week, read back through the repository.
fn week_of(project: &mut Project) -> WorkingDays {
    project
        .db()
        .structure()
        .school()
        .expect("read school")
        .expect("a school exists")
        .working_days
}

// ===================================================================== round tripping

#[test]
fn the_fixture_school_works_monday_to_friday() {
    let mut project = Project::new();
    assert_eq!(week_of(&mut project), WorkingDays::MONDAY_TO_FRIDAY);
    assert_eq!(week_of(&mut project).days(), vec![1, 2, 3, 4, 5]);
}

#[test]
fn a_sunday_to_thursday_week_survives_a_round_trip() {
    let mut project = Project::new();
    let school = week_of(&mut project);
    assert_ne!(school.days(), vec![1, 2, 3, 4, 7]);

    let id = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;
    let week = WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week");
    project
        .db()
        .structure()
        .update_school(
            id,
            &SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: week,
            },
        )
        .expect("update");

    // Through a fresh read, so this is the stored value and not the one just handed over.
    assert_eq!(week_of(&mut project), week);
    assert_eq!(week_of(&mut project).days(), vec![1, 2, 3, 4, 7]);
}

#[test]
fn every_possible_week_survives_a_round_trip() {
    let mut project = Project::new();
    let id = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;

    for bits in 1..=127i64 {
        let week = WorkingDays::from_bits(bits).expect("in range");
        project
            .db()
            .structure()
            .update_school(
                id,
                &SchoolInput {
                    name: "Northgate Secondary".to_owned(),
                    timezone: "Europe/London".to_owned(),
                    locale: "en-GB".to_owned(),
                    working_days: week,
                },
            )
            .expect("update");
        assert_eq!(
            week_of(&mut project),
            week,
            "{bits} did not survive storage"
        );
    }
}

#[test]
fn a_six_day_week_is_storable() {
    let mut project = Project::new();
    let id = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;
    let week = WorkingDays::from_days(&[1, 2, 3, 4, 5, 6]).expect("a real week");
    project
        .db()
        .structure()
        .update_school(
            id,
            &SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: week,
            },
        )
        .expect("update");
    assert_eq!(week_of(&mut project).count(), 6);
}

#[test]
fn changing_the_week_consumes_one_revision_and_leaves_the_rest_alone() {
    let mut project = Project::new();
    let before = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");

    let after = project
        .db()
        .structure()
        .update_school(
            before.id,
            &SchoolInput {
                name: before.name.clone(),
                timezone: before.timezone.clone(),
                locale: before.locale.clone(),
                working_days: WorkingDays::EVERY_DAY,
            },
        )
        .expect("update");

    assert_eq!(after.rev, before.rev + 1);
    assert_eq!(after.id, before.id);
    assert_eq!(after.name, before.name);
    assert_eq!(after.timezone, before.timezone);
    assert_eq!(after.locale, before.locale);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.working_days, WorkingDays::EVERY_DAY);
}

// ================================================================= existing projects

/// What the migration actually declared.
///
/// Direct evidence about the upgrade, independent of any row: SQLite returns a column's
/// declared default for every row written before that column existed. Together with
/// `evara_core::calendar::tests::the_stored_default_is_thirty_one` — which pins 31 to
/// `WorkingDays::MONDAY_TO_FRIDAY` — this is the whole guarantee for an existing project.
#[test]
fn the_column_declares_a_monday_to_friday_default_that_cannot_be_null() {
    let mut project = Project::new();
    let declared = project
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare("PRAGMA table_info(school)")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            let mut found = None;
            for row in rows {
                let (name, notnull, default) = row?;
                if name == "working_days" {
                    found = Some((notnull, default));
                }
            }
            Ok(found)
        })
        .expect("read schema");

    let (notnull, default) = declared.expect("school.working_days exists");
    assert_eq!(notnull, 1, "the column must be NOT NULL");
    assert_eq!(
        default.as_deref(),
        Some("31"),
        "the default must stay Monday to Friday"
    );
    assert_eq!(WorkingDays::MONDAY_TO_FRIDAY.bits(), 31);
}

/// The upgrade path, exercised the only way a single-schema test can.
///
/// A project written by a V6 build has a `school` row that predates `working_days`.
/// SQLite's `ALTER TABLE ADD COLUMN` with a constant default does not rewrite such a row —
/// it returns the default when the row is read — and a row *inserted* without naming the
/// column takes the identical path through the identical default. So this exercises the
/// same mechanism an upgraded project does, and asserts the thing that actually matters:
/// an existing school comes back as an ordinary Monday-to-Friday school rather than as a
/// failure, or as an empty week.
#[test]
fn a_school_row_written_without_the_column_reads_as_monday_to_friday() {
    let mut project = Project::new();
    // The fixture's campus, year and term reference the school with ON DELETE RESTRICT,
    // so they go first. Deleting through SQL rather than the repositories on purpose:
    // this is manufacturing a file state, not exercising a deletion rule.
    raw(
        project.db(),
        "DELETE FROM term;
         DELETE FROM academic_year;
         DELETE FROM campus;
         DELETE FROM school;",
    );

    raw(
        project.db(),
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES ('0192f0a0-0000-7000-8000-000000000001', 'Legacy School',
                 'Europe/London', 'en-GB',
                 '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
    );

    let school = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");

    assert_eq!(school.working_days, WorkingDays::MONDAY_TO_FRIDAY);
    assert_eq!(school.working_days.days(), vec![1, 2, 3, 4, 5]);
    // Untouched by the upgrade: the schema changed, the school did not.
    assert_eq!(school.rev, 1);
    assert_eq!(school.updated_at, "2026-01-01T00:00:00.000Z");
    assert_eq!(school.name, "Legacy School");
}

#[test]
fn the_schema_refuses_a_week_with_no_days() {
    let mut project = Project::new();
    // The repository cannot be asked for this — `WorkingDays` has no empty value — so the
    // CHECK is tested the only way it can be: by going behind the repository. The point
    // of encoding the week as bits is that this is impossible rather than merely
    // discouraged.
    let outcome = project.db().write(|tx| {
        tx.execute_batch("UPDATE school SET working_days = 0")?;
        Ok(())
    });
    assert!(
        outcome.is_err(),
        "a school with no working days must be refused by the schema"
    );

    for bad in ["-1", "128", "255"] {
        let outcome = project.db().write(|tx| {
            tx.execute_batch(&format!("UPDATE school SET working_days = {bad}"))?;
            Ok(())
        });
        assert!(outcome.is_err(), "{bad} must be refused by the schema");
    }
}

#[test]
fn a_stored_week_outside_the_range_is_corruption_not_a_silent_default() {
    let mut project = Project::new();
    assert_eq!(
        WorkingDays::from_bits(0),
        Err(WorkingDaysError::CorruptBits { bits: 0 })
    );

    // The CHECK makes this unreachable through SQL, so the decoder is exercised directly.
    // What matters is that it does not invent Monday-to-Friday: a damaged file must look
    // damaged. See `FromColumn for WorkingDays`.
    let school = project.db().structure().school().expect("read");
    assert!(school.is_some(), "the fixture school still reads normally");
}

// ============================================== the advisory, and what it must not do

/// A school with a cycle, its days, and calendar mappings for a given run of dates.
struct Mapped {
    project: Project,
    cycle_days: Vec<Uuid>,
}

impl Mapped {
    /// `day_count` cycle days, and one mapping per entry of `dates`.
    ///
    /// The cycle's length and the dates are chosen independently by each test, which is
    /// the point: nothing in the model relates them.
    fn new(day_count: i64, dates: &[&str]) -> Self {
        let mut project = Project::new();
        let cycle = project.cycle("Rotation", day_count);

        let mut cycle_days = Vec::new();
        for ordinal in 1..=day_count {
            let day = project
                .db()
                .time_model()
                .create_cycle_day(&CycleDayInput {
                    cycle_id: cycle,
                    ordinal,
                    label: format!("Day {ordinal}"),
                    // Deliberately absent. A rotation day has no weekday, and setting a
                    // hint here would make the test lie about what is being compared.
                    weekday_hint: None,
                })
                .expect("cycle day")
                .id;
            cycle_days.push(day);
        }

        let school = project
            .db()
            .structure()
            .school()
            .expect("read")
            .expect("exists")
            .id;

        for (index, date) in dates.iter().enumerate() {
            let cycle_day = cycle_days[index % cycle_days.len()];
            project
                .db()
                .time_model()
                .create_calendar_day(&CalendarDayInput {
                    school_id: school,
                    date: (*date).to_owned(),
                    term_id: None,
                    campus_id: None,
                    cycle_day_id: Some(cycle_day),
                    kind: CalendarDayKind::School,
                    note: None,
                })
                .expect("calendar day");
        }

        Self {
            project,
            cycle_days,
        }
    }

    fn findings(&mut self, week: WorkingDays) -> Vec<String> {
        self.project
            .db()
            .time_model()
            .non_working_instructional_days(week)
            .expect("advisory")
            .into_iter()
            .map(|day| day.date)
            .collect()
    }
}

#[test]
fn a_teaching_date_on_a_non_working_weekday_is_reported() {
    // 2027-09-01 is a Wednesday; 2027-09-04 is a Saturday.
    let mut mapped = Mapped::new(5, &["2027-09-01", "2027-09-02", "2027-09-04"]);

    let findings = mapped.findings(WorkingDays::MONDAY_TO_FRIDAY);

    assert_eq!(findings, vec!["2027-09-04".to_owned()]);
}

#[test]
fn the_weekday_reported_is_the_one_the_date_actually_falls_on() {
    let mut mapped = Mapped::new(5, &["2027-09-04", "2027-09-05"]);
    let found = mapped
        .project
        .db()
        .time_model()
        .non_working_instructional_days(WorkingDays::MONDAY_TO_FRIDAY)
        .expect("advisory");

    // Saturday is 6 and Sunday is 7 in ISO numbering. SQLite counts Sunday as 0, so this
    // is what keeps `iso_weekday` honest.
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].date, "2027-09-04");
    assert_eq!(found[0].weekday, 6);
    assert_eq!(found[1].date, "2027-09-05");
    assert_eq!(found[1].weekday, 7);
}

#[test]
fn a_sunday_to_thursday_school_is_warned_about_fridays_not_sundays() {
    // 2027-09-03 is a Friday, 2027-09-05 a Sunday.
    let mut mapped = Mapped::new(5, &["2027-09-03", "2027-09-05"]);
    let week = WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week");

    let findings = mapped.findings(week);

    assert_eq!(findings, vec!["2027-09-03".to_owned()]);
}

#[test]
fn a_date_with_no_lessons_is_never_a_finding() {
    let mut project = Project::new();
    let school = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;

    // A Saturday, explicitly marked as having no lessons. That is what a non-working day
    // is supposed to look like, so warning about it would be noise.
    project
        .db()
        .time_model()
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-09-04".to_owned(),
            term_id: None,
            campus_id: None,
            cycle_day_id: None,
            kind: CalendarDayKind::Holiday,
            note: None,
        })
        .expect("calendar day");

    let found = project
        .db()
        .time_model()
        .non_working_instructional_days(WorkingDays::MONDAY_TO_FRIDAY)
        .expect("advisory");
    assert!(found.is_empty(), "found {found:?}");
}

#[test]
fn a_six_day_cycle_across_a_five_day_week_produces_no_findings() {
    // The brief's case, and the one that would break first if anything started inferring
    // a working week from `day_count`. Six cycle days, mapped onto six consecutive
    // *weekdays* that happen to span two calendar weeks.
    let mut mapped = Mapped::new(
        6,
        &[
            "2027-09-01", // Wednesday
            "2027-09-02", // Thursday
            "2027-09-03", // Friday
            "2027-09-06", // Monday
            "2027-09-07", // Tuesday
            "2027-09-08", // Wednesday
        ],
    );

    assert_eq!(mapped.cycle_days.len(), 6);
    let findings = mapped.findings(WorkingDays::MONDAY_TO_FRIDAY);
    assert!(
        findings.is_empty(),
        "a six-day rotation on a five-day week is an ordinary school, found {findings:?}"
    );
}

#[test]
fn a_five_day_week_does_not_mean_monday_to_friday() {
    // Five working days, but not the usual five: 2027-09-03 is a Friday and 2027-09-06 a
    // Monday. A build that read "five working days" as Monday-to-Friday would report
    // neither; a Sunday-to-Thursday school is working on the Monday and not on the Friday.
    let mut mapped = Mapped::new(5, &["2027-09-03", "2027-09-06"]);
    let week = WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week");
    assert_eq!(week.count(), WorkingDays::MONDAY_TO_FRIDAY.count());

    let findings = mapped.findings(week);
    assert_eq!(findings, vec!["2027-09-03".to_owned()]);
}

/// Every `calendar_day` row, as a comparable snapshot.
///
/// Includes `rev` and `updated_at` on purpose: "the mapping is preserved" has to mean
/// the row was not rewritten, not merely that it still says the same thing.
fn calendar_snapshot(project: &mut Project) -> Vec<String> {
    project
        .db()
        .time_model()
        .calendar_days("0000-01-01", "9999-12-31")
        .expect("read calendar")
        .into_iter()
        .map(|day| {
            format!(
                "{}|{}|{:?}|{:?}|{}|{}|{}",
                day.id,
                day.date,
                day.cycle_day_id,
                day.term_id,
                day.rev,
                day.created_at,
                day.updated_at
            )
        })
        .collect()
}

#[test]
fn changing_the_working_week_does_not_touch_a_single_calendar_mapping() {
    // Mappings that include two Saturdays, so the new week genuinely disagrees with them.
    let mut mapped = Mapped::new(
        5,
        &[
            "2027-09-01",
            "2027-09-02",
            "2027-09-03",
            "2027-09-04",
            "2027-09-11",
        ],
    );

    let before = calendar_snapshot(&mut mapped.project);
    assert_eq!(before.len(), 5);

    let id = mapped
        .project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;
    for week in [
        WorkingDays::from_days(&[1]).expect("a real week"),
        WorkingDays::from_days(&[6, 7]).expect("a real week"),
        WorkingDays::EVERY_DAY,
        WorkingDays::MONDAY_TO_FRIDAY,
    ] {
        mapped
            .project
            .db()
            .structure()
            .update_school(
                id,
                &SchoolInput {
                    name: "Northgate Secondary".to_owned(),
                    timezone: "Europe/London".to_owned(),
                    locale: "en-GB".to_owned(),
                    working_days: week,
                },
            )
            .expect("update");

        // Asking the advisory as well, because a projection that mutated on read would be
        // the subtler version of the same bug.
        let _ = mapped.findings(week);

        assert_eq!(
            calendar_snapshot(&mut mapped.project),
            before,
            "the working week must not create, delete or rewrite a calendar mapping"
        );
    }
}

#[test]
fn the_advisory_never_changes_what_it_reports_on() {
    let mut mapped = Mapped::new(5, &["2027-09-04", "2027-09-05"]);
    let before = calendar_snapshot(&mut mapped.project);

    for _ in 0..3 {
        let findings = mapped.findings(WorkingDays::MONDAY_TO_FRIDAY);
        assert_eq!(findings.len(), 2);
    }

    assert_eq!(calendar_snapshot(&mut mapped.project), before);
}

#[test]
fn cycle_day_identities_are_untouched_by_a_change_of_week() {
    let mut mapped = Mapped::new(6, &["2027-09-01", "2027-09-02"]);
    let before = mapped.cycle_days.clone();

    let id = mapped
        .project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists")
        .id;
    mapped
        .project
        .db()
        .structure()
        .update_school(
            id,
            &SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week"),
            },
        )
        .expect("update");

    let cycle = mapped
        .project
        .db()
        .time_model()
        .cycles()
        .expect("cycles")
        .first()
        .expect("one cycle")
        .id;
    let after: Vec<Uuid> = mapped
        .project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("cycle days")
        .into_iter()
        .map(|day| day.id)
        .collect();

    assert_eq!(after, before, "a cycle day's identity is not a weekday's");
}

#[test]
fn the_advisory_answers_for_a_project_with_no_calendar_at_all() {
    // The common case on a fresh project: nothing mapped yet, so nothing to warn about.
    // It must answer rather than fail, because the screen asks on every load.
    let mut project = Project::new();
    raw(project.db(), "DELETE FROM calendar_day");

    for week in [
        WorkingDays::MONDAY_TO_FRIDAY,
        WorkingDays::EVERY_DAY,
        WorkingDays::from_days(&[6]).expect("a real week"),
    ] {
        let found = project
            .db()
            .time_model()
            .non_working_instructional_days(week)
            .expect("advisory");
        assert!(found.is_empty(), "found {found:?}");
    }
}

#[test]
fn a_missing_school_update_is_still_not_found() {
    let mut project = Project::new();
    let outcome = project.db().structure().update_school(
        Uuid::nil(),
        &SchoolInput {
            name: "Nobody".to_owned(),
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
            working_days: WorkingDays::MONDAY_TO_FRIDAY,
        },
    );
    assert!(matches!(
        outcome,
        Err(DbError::NotFound { entity: "school" })
    ));
}
