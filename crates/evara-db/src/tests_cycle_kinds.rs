//! Weekly and custom cycles: what each kind guarantees, and what neither may hold.
//!
//! `evara_core::calendar` owns the seven weekday positions and `evara_core::time` owns the
//! rotation arithmetic, and both are tested there. This file owns the things only a real
//! project file can show:
//!
//! - creating a cycle lays out its positions, in the same transaction, per its kind
//! - a weekly cycle is seven days whatever length the caller asked for
//! - one weekly cycle, two working weeks, two answers and **one unchanged cycle** — the
//!   requirement the whole design exists for
//! - a kind never changes, and the schema refuses it even if the repository were bypassed
//! - every identifier is stable across relabelling, lengthening and reopening
//! - timeslots and calendar mappings survive all of it
//! - a cycle written before the kind existed reads as `CUSTOM`, with its days, labels,
//!   identifiers and revisions untouched
//! - the `cycle` table holds no date, no rotation offset, no working week and no holiday
//!   policy, so a cycle cannot bind itself to one school's calendar
//!
//! Real temporary project files throughout, never `:memory:`.

use evara_core::calendar::{WeeklyPosition, WorkingDays};
use tempfile::TempDir;
use uuid::Uuid;

use crate::repo::time_model::{
    CalendarDayInput, CalendarDayKind, CycleDay, CycleDayInput, CycleInput, CycleKind,
    MaterialisationRequest,
};
use crate::tests_support;
use crate::tests_time_model::Project;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// The project's one school.
fn school_of(project: &mut Project) -> Uuid {
    tests_support::only_school_id(project.db())
}

/// A cycle's days in cycle order.
fn days_of(project: &mut Project, cycle: Uuid) -> Vec<CycleDay> {
    project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("read days")
}

/// A cycle's days as `(ordinal, id, label, weekday_hint, rev)`.
///
/// Carries `id` and `rev` deliberately: "unchanged" has to mean the rows were not
/// rewritten, not merely that rows with those labels still exist.
fn snapshot(project: &mut Project, cycle: Uuid) -> Vec<(i64, Uuid, String, Option<i64>, i64)> {
    days_of(project, cycle)
        .into_iter()
        .map(|day| (day.ordinal, day.id, day.label, day.weekday_hint, day.rev))
        .collect()
}

/// Replaces a cycle's details, carrying its stored kind through as the interface does.
fn edit(project: &mut Project, cycle: Uuid, name: &str, day_count: i64) -> Result<(), DbError> {
    let school = school_of(project);
    let kind = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("the cycle exists")
        .kind;
    project
        .db()
        .time_model()
        .update_cycle(
            cycle,
            &CycleInput {
                school_id: school,
                name: name.to_owned(),
                kind,
                day_count,
                week_count: 1,
                is_default: false,
            },
        )
        .map(|_| ())
}

// ======================================================================== weekly cycles

#[test]
fn a_weekly_cycle_is_created_with_the_seven_weekdays() {
    let mut project = Project::new();
    // The helper asks for one day on purpose: the kind decides the length, not the caller.
    let cycle = project.weekly_cycle("School week");

    let row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("exists");
    assert_eq!(row.kind, CycleKind::Weekly);
    assert_eq!(row.day_count, 7, "a week is seven days, whatever was asked");

    let days = days_of(&mut project, cycle);
    assert_eq!(days.len(), 7, "no manual day creation was needed");
    assert_eq!(
        days.iter()
            .map(|day| day.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday"
        ]
    );
    for (index, day) in days.iter().enumerate() {
        let position = i64::try_from(index).expect("small") + 1;
        assert_eq!(day.ordinal, position, "the position is the identity");
        assert_eq!(
            day.weekday_hint,
            Some(position),
            "and it carries the weekday of that position"
        );
        assert_eq!(day.rev, 1, "written once, by creation");
    }
}

#[test]
fn a_weekly_cycle_is_complete_the_moment_it_exists() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");
    assert!(coverage.is_complete());
    assert_eq!(coverage.present, 7);
    assert_eq!(coverage.missing_ordinals, Vec::<i64>::new());
}

/// The requirement the whole design exists for.
///
/// One cycle definition, asked about three different working weeks. Each answer differs,
/// and the cycle itself is byte-for-byte unchanged by the asking — no row records which
/// week was asked about, so a second school using this cycle cannot be affected by the
/// first.
#[test]
fn one_weekly_cycle_serves_schools_with_different_working_weeks() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let before = snapshot(&mut project, cycle);

    let applicable = |project: &mut Project, week: WorkingDays| -> Vec<i64> {
        project
            .db()
            .time_model()
            .cycle_shape(cycle, week)
            .expect("shape")
            .weekdays
            .into_iter()
            .filter(|day| day.applies)
            .map(|day| day.ordinal)
            .collect()
    };

    let monday_to_friday = WorkingDays::MONDAY_TO_FRIDAY;
    let sunday_to_thursday = WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week");
    let six_days = WorkingDays::from_days(&[1, 2, 3, 4, 5, 6]).expect("a real week");

    assert_eq!(
        applicable(&mut project, monday_to_friday),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(
        applicable(&mut project, sunday_to_thursday),
        vec![1, 2, 3, 4, 7]
    );
    assert_eq!(applicable(&mut project, six_days), vec![1, 2, 3, 4, 5, 6]);

    // Asked three times, and the definition is exactly what it was.
    assert_eq!(snapshot(&mut project, cycle), before);
    let row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("exists");
    assert_eq!(row.rev, 1, "no revision was consumed by being asked");
    assert_eq!(row.day_count, 7, "and all seven positions are still there");
}

/// A five-day school sees its five days with nothing further to configure.
#[test]
fn a_monday_to_friday_school_sees_five_applicable_days_unconfigured() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let week = school_working_days(&mut project);
    assert_eq!(
        week,
        WorkingDays::MONDAY_TO_FRIDAY,
        "the fixture's own week"
    );

    let shape = project
        .db()
        .time_model()
        .cycle_shape(cycle, week)
        .expect("shape");

    assert_eq!(shape.weekdays.len(), 7, "the definition holds the week");
    let applying: Vec<&str> = shape
        .weekdays
        .iter()
        .filter(|day| day.applies)
        .map(|day| day.canonical_label.as_str())
        .collect();
    assert_eq!(
        applying,
        vec!["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]
    );
    for day in &shape.weekdays {
        assert!(day.cycle_day_id.is_some(), "every position has a day");
        assert_eq!(day.label.as_deref(), Some(day.canonical_label.as_str()));
    }
}

/// The school's own week, read through the repository.
fn school_working_days(project: &mut Project) -> WorkingDays {
    tests_support::only_school(project.db())
        .expect("a school exists")
        .working_days
}

#[test]
fn a_weekly_days_label_may_be_changed_and_its_position_may_not() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let monday = days_of(&mut project, cycle)[0].id;

    project.relabel_day(monday, "Lundi");

    let days = days_of(&mut project, cycle);
    assert_eq!(days[0].id, monday, "the identifier is stable");
    assert_eq!(days[0].label, "Lundi");
    assert_eq!(days[0].ordinal, 1, "and the position did not move");
    assert_eq!(days[0].weekday_hint, Some(1));

    // The projection still knows what the position *is*, whatever it is now called.
    let shape = project
        .db()
        .time_model()
        .cycle_shape(cycle, WorkingDays::MONDAY_TO_FRIDAY)
        .expect("shape");
    let first = &shape.weekdays[0];
    assert_eq!(first.canonical_label, "Monday");
    assert_eq!(first.label.as_deref(), Some("Lundi"));
    assert!(first.applies);
}

/// The schema's backstop, provoked past the repository.
///
/// The repository never writes a mismatched hint, so this writes one directly to prove the
/// file itself holds the rule — a future repository, the CLI or a hand repair cannot make
/// a weekly cycle whose position 6 claims to be a Tuesday.
#[test]
fn the_schema_refuses_a_weekly_day_whose_weekday_disagrees_with_its_position() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let saturday = days_of(&mut project, cycle)[5].id;

    let outcome = project.db().time_model().update_cycle_day(
        saturday,
        &CycleDayInput {
            cycle_id: cycle,
            ordinal: 6,
            label: "Saturday".to_owned(),
            weekday_hint: Some(2),
        },
    );
    assert!(outcome.is_err(), "a Saturday cannot claim to be a Tuesday");

    let outcome = project.db().time_model().update_cycle_day(
        saturday,
        &CycleDayInput {
            cycle_id: cycle,
            ordinal: 6,
            label: "Saturday".to_owned(),
            weekday_hint: None,
        },
    );
    assert!(outcome.is_err(), "nor may it have no weekday at all");

    // And the stored row is what it was.
    let days = days_of(&mut project, cycle);
    assert_eq!(days[5].weekday_hint, Some(6));
    assert_eq!(days[5].rev, 1);
}

/// A weekly cycle's length is not the caller's to change, and a lost day comes back.
#[test]
fn editing_a_weekly_cycle_restores_a_day_that_went_missing() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let before = snapshot(&mut project, cycle);
    let sunday = before[6].1;

    tests_support::delete_cycle_day_here(project.db(), sunday)
        .expect("nothing references a brand-new day");
    assert_eq!(days_of(&mut project, cycle).len(), 6);

    // Asking for five days, which a weekly cycle ignores.
    edit(&mut project, cycle, "Term-time week", 5).expect("renaming a weekly cycle");

    let after = snapshot(&mut project, cycle);
    assert_eq!(after.len(), 7, "the missing weekday was laid out again");
    assert_eq!(after[6].0, 7);
    assert_eq!(after[6].2, "Sunday");
    assert_ne!(
        after[6].1, sunday,
        "a replacement row, honestly a new one: the old identifier is gone for good"
    );
    // The six that survived are untouched.
    for existing in before.iter().take(6) {
        assert!(after.contains(existing), "day {existing:?} was altered");
    }

    let row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("exists");
    assert_eq!(row.day_count, 7, "and the length was never five");
    assert_eq!(row.name, "Term-time week");
}

// ======================================================================== custom cycles

#[test]
fn a_custom_five_day_cycle_is_created_with_day_one_to_day_five() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);

    let row = project
        .db()
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("exists");
    assert_eq!(row.kind, CycleKind::Custom);
    assert_eq!(row.day_count, 5);

    let days = days_of(&mut project, cycle);
    assert_eq!(days.len(), 5, "no manual day creation was needed");
    assert_eq!(
        days.iter()
            .map(|day| day.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Day 1", "Day 2", "Day 3", "Day 4", "Day 5"]
    );
    for (index, day) in days.iter().enumerate() {
        assert_eq!(day.ordinal, i64::try_from(index).expect("small") + 1);
        assert_eq!(
            day.weekday_hint, None,
            "a rotation position names no weekday"
        );
        assert_eq!(day.rev, 1);
    }

    let coverage = project
        .db()
        .time_model()
        .cycle_coverage(cycle)
        .expect("coverage");
    assert!(coverage.is_complete());
}

/// Day 5 hands over to Day 1, and the projection says so rather than the screen guessing.
#[test]
fn a_custom_cycle_reports_where_its_rotation_wraps() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);

    let shape = project
        .db()
        .time_model()
        .cycle_shape(cycle, WorkingDays::MONDAY_TO_FRIDAY)
        .expect("shape");

    assert_eq!(shape.kind, CycleKind::Custom);
    assert_eq!(shape.rotation.sequence, vec![1, 2, 3, 4, 5]);
    assert_eq!(shape.rotation.wraps_to, Some(1));
    assert_eq!(shape.rotation.next_after_last(), Some(1));
    assert!(
        shape.weekdays.is_empty(),
        "a custom cycle has positions, not weekdays"
    );
}

#[test]
fn custom_day_labels_may_be_edited_and_the_identifiers_do_not_move() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    let before = snapshot(&mut project, cycle);

    for (index, label) in ["Red", "Amber", "Green", "Blue", "Violet"]
        .iter()
        .enumerate()
    {
        project.relabel_day(before[index].1, label);
    }

    let after = snapshot(&mut project, cycle);
    assert_eq!(
        after.iter().map(|day| day.1).collect::<Vec<_>>(),
        before.iter().map(|day| day.1).collect::<Vec<_>>(),
        "every identifier survived the renaming"
    );
    assert_eq!(
        after.iter().map(|day| day.2.as_str()).collect::<Vec<_>>(),
        vec!["Red", "Amber", "Green", "Blue", "Violet"]
    );
    assert_eq!(
        after.iter().map(|day| day.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5],
        "and so did every position"
    );
    for day in &after {
        assert_eq!(day.4, 2, "one update, one revision");
    }
}

#[test]
fn lengthening_a_custom_cycle_leaves_a_renamed_day_alone() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 3);
    let before = snapshot(&mut project, cycle);
    project.relabel_day(before[1].1, "Red Day");
    let renamed = snapshot(&mut project, cycle);

    edit(&mut project, cycle, "Rotation", 5).expect("lengthening is safe");

    let after = snapshot(&mut project, cycle);
    assert_eq!(after.len(), 5);
    for existing in &renamed {
        assert!(after.contains(existing), "day {existing:?} was altered");
    }
    assert_eq!(after[1].2, "Red Day", "the rename stands");
    assert_eq!(after[3].2, "Day 4");
    assert_eq!(after[4].2, "Day 5");
}

/// A generated label must not collide with one the user chose.
///
/// Day 3 renamed to "Day 4" would otherwise make "lengthen to four days" fail on
/// `UNIQUE (cycle_id, label)`, reporting a duplicate against a field the form never
/// showed. The rule is kept; the generated label steps aside.
#[test]
fn a_laid_out_label_steps_aside_rather_than_failing_the_edit() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 3);
    let third = snapshot(&mut project, cycle)[2].1;
    project.relabel_day(third, "Day 4");

    edit(&mut project, cycle, "Rotation", 4).expect("the edit must not be blocked");

    let labels: Vec<String> = days_of(&mut project, cycle)
        .into_iter()
        .map(|day| day.label)
        .collect();
    assert_eq!(labels, vec!["Day 1", "Day 2", "Day 4", "Day 4 (2)"]);
}

/// Shortening is still refused rather than deleting the days in the way.
#[test]
fn shortening_a_custom_cycle_is_refused_not_resolved_by_deleting() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);
    let before = snapshot(&mut project, cycle);

    let error = edit(&mut project, cycle, "Rotation", 3).expect_err("must be refused");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");

    assert_eq!(snapshot(&mut project, cycle), before, "nothing was removed");
}

// ============================================================== the kind never changes

#[test]
fn a_cycles_kind_cannot_be_changed_after_it_is_created() {
    let mut project = Project::new();
    let custom = project.cycle("Rotation", 5);
    let school = school_of(&mut project);

    let error = project
        .db()
        .time_model()
        .update_cycle(
            custom,
            &CycleInput {
                school_id: school,
                name: "Rotation".to_owned(),
                kind: CycleKind::Weekly,
                day_count: 7,
                week_count: 1,
                is_default: false,
            },
        )
        .expect_err("a custom cycle cannot become weekly");

    match error {
        DbError::Invalid { ref message } => assert!(
            message.contains("cannot be changed") && message.contains("custom"),
            "the refusal must say what it is and that it is fixed: {message}"
        ),
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }

    let row = project
        .db()
        .time_model()
        .cycle(custom)
        .expect("read")
        .expect("exists");
    assert_eq!(row.kind, CycleKind::Custom);
    assert_eq!(row.day_count, 5, "and nothing else was written either");
    assert_eq!(row.rev, 1);
}

/// The schema's backstop for the same rule, provoked past the repository.
#[test]
fn the_schema_refuses_a_kind_change_even_written_directly() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 5);

    let outcome = project.db().write(|tx| {
        tx.execute(
            "UPDATE cycle SET kind = 'WEEKLY' WHERE id = ?",
            [cycle.to_string()],
        )?;
        Ok(())
    });
    assert!(outcome.is_err(), "the file itself must refuse this");

    assert_eq!(
        project
            .db()
            .time_model()
            .cycle(cycle)
            .expect("read")
            .expect("exists")
            .kind,
        CycleKind::Custom
    );
}

/// A weekly cycle of any other length is unrepresentable, not merely unproduced.
#[test]
fn the_schema_refuses_a_weekly_cycle_that_is_not_seven_days() {
    let mut project = Project::new();
    let school = school_of(&mut project);

    let outcome = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO cycle
                 (id, school_id, name, kind, day_count, week_count, is_default,
                  created_at, updated_at, rev)
             VALUES (?, ?, 'Five-day week', 'WEEKLY', 5, 1, 0, ?, ?, 1)",
            (
                Uuid::now_v7().to_string(),
                school.to_string(),
                "2026-10-12T00:00:00.000Z",
                "2026-10-12T00:00:00.000Z",
            ),
        )?;
        Ok(())
    });
    assert!(outcome.is_err(), "a weekly cycle is seven days long");
}

// ============================================ what the cycle table may never hold

/// A cycle is a reusable definition, and the schema is what keeps it one.
///
/// Named columns rather than a count, so a future column arrives as a failure that says
/// which one jumped in. The forbidden list is the brief's section 3: a start date, a
/// starting rotation position, a working week and a holiday policy all belong to a
/// school's calendar mapping, scoped per school, and none of them may live here.
#[test]
fn the_cycle_table_holds_nothing_about_a_schools_calendar() {
    let mut project = Project::new();
    let columns: Vec<String> = project
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('cycle')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .expect("read the cycle columns");

    assert_eq!(
        columns,
        vec![
            "id".to_owned(),
            "school_id".to_owned(),
            "name".to_owned(),
            "day_count".to_owned(),
            "week_count".to_owned(),
            "is_default".to_owned(),
            "created_at".to_owned(),
            "updated_at".to_owned(),
            "rev".to_owned(),
            // V9, appended by ALTER TABLE.
            "kind".to_owned(),
        ],
        "the cycle table has grown a column that was not reviewed"
    );

    for forbidden in [
        "starts_on",
        "start_date",
        "start_ordinal",
        "starting_cycle_day_id",
        "rotation_offset",
        "working_days",
        "holiday_policy",
        "holidays_consume_rotation",
        "campus_id",
    ] {
        assert!(
            !columns.iter().any(|column| column == forbidden),
            "`{forbidden}` is a school's calendar, not a shared cycle's definition"
        );
    }

    // Same rule for the days: a position carries no date either.
    let day_columns: Vec<String> = project
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('cycle_day')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .expect("read the cycle_day columns");
    for forbidden in ["date", "starts_on", "is_working_day", "holiday"] {
        assert!(
            !day_columns.iter().any(|column| column == forbidden),
            "`{forbidden}` would bind a cycle day to one school's calendar"
        );
    }
}

// =================================================== timeslots, mappings, persistence

/// Relabelling and lengthening touch no timeslot identifier and no calendar mapping.
///
/// This is risk R6: a timeslot identifier is what a future lesson points at, so a rebuild
/// that reassigned one would be silent corruption. Both kinds are exercised, because the
/// weekly path creates its days through new code.
#[test]
fn timeslots_and_calendar_mappings_survive_cycle_editing() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let structure = project.structure("Standard bells", None);
    let _ = project.period(structure, 1, "1", crate::PeriodKind::Teaching);
    let _ = project.period(structure, 2, "2", crate::PeriodKind::Teaching);
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");
    let slots_before = project.slots_by_tuple(cycle, Some(term));
    assert_eq!(slots_before.len(), 14, "seven days by two periods");

    let monday = days_of(&mut project, cycle)[0].id;
    let school = school_of(&mut project);
    let mapping = project
        .db()
        .time_model()
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-09-06".to_owned(),
            term_id: Some(term),
            campus_id: None,
            cycle_day_id: Some(monday),
            kind: CalendarDayKind::School,
            note: None,
        })
        .expect("map a date onto Monday")
        .id;

    // Rename the cycle, rename a day, and ask about a different working week.
    edit(&mut project, cycle, "Teaching week", 7).expect("rename");
    project.relabel_day(monday, "Lundi");
    let _ = project
        .db()
        .time_model()
        .cycle_shape(
            cycle,
            WorkingDays::from_days(&[6, 7]).expect("a weekend school"),
        )
        .expect("shape");

    assert_eq!(
        project.slots_by_tuple(cycle, Some(term)),
        slots_before,
        "every timeslot identifier is where it was"
    );
    let stored = project
        .db()
        .time_model()
        .calendar_day(mapping)
        .expect("read")
        .expect("still there");
    assert_eq!(stored.cycle_day_id, Some(monday));
    assert_eq!(stored.date, "2027-09-06");
    assert_eq!(stored.rev, 1, "and it was never rewritten");
}

#[test]
fn both_kinds_survive_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let weekly = project.weekly_cycle("School week");
    let custom = project.cycle("Six-day rotation", 6);
    let first = days_of(&mut project, custom)[0].id;
    project.relabel_day(first, "Red Day");

    let weekly_before = snapshot(&mut project, weekly);
    let custom_before = snapshot(&mut project, custom);

    project.reopen();

    assert_eq!(snapshot(&mut project, weekly), weekly_before);
    assert_eq!(snapshot(&mut project, custom), custom_before);

    let kinds: Vec<CycleKind> = tests_support::cycles_here(project.db())
        .into_iter()
        .map(|cycle| cycle.kind)
        .collect();
    assert_eq!(kinds.len(), 2);
    assert!(kinds.contains(&CycleKind::Weekly));
    assert!(kinds.contains(&CycleKind::Custom));

    // And the projection still answers from the reopened file.
    let shape = project
        .db()
        .time_model()
        .cycle_shape(weekly, WorkingDays::MONDAY_TO_FRIDAY)
        .expect("shape");
    assert_eq!(shape.weekdays.iter().filter(|day| day.applies).count(), 5);
}

// ============================================================== an existing project

/// Writes a project as a V8 build would have left it: a six-day rotation with its own
/// labels and one position deliberately empty.
///
/// Raw SQL straight past the repositories, which is the only way to produce a row that a
/// build without the `kind` column would have written. Same technique and the same
/// justification as `tests_working_days::raw`.
fn write_a_pre_v9_project(
    path: &std::path::Path,
    site: Uuid,
    school: Uuid,
    cycle: Uuid,
    day_ids: &[Uuid],
    stamp: &str,
) {
    let mut conn = crate::connection::open_configured(path, site).expect("open");

    let version = crate::migrations::run_up_to(&mut conn, 8).expect("migrate to V8");
    assert_eq!(version, 8, "the fixture must stop at the previous version");

    conn.execute(
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', ?, ?, 1)",
        (school.to_string(), stamp, stamp),
    )
    .expect("write a school the old build could have written");
    conn.execute(
        "INSERT INTO cycle
             (id, school_id, name, day_count, week_count, is_default,
              created_at, updated_at, rev)
         VALUES (?, ?, 'Six-day rotation', 6, 2, 1, ?, ?, 4)",
        (cycle.to_string(), school.to_string(), stamp, stamp),
    )
    .expect("write a cycle the old build could have written");
    for (index, id) in day_ids.iter().enumerate() {
        // Positions 1, 2, 3, 4, 5 and 7 would be wrong; 6 is the declared length, so
        // this leaves position 3 empty instead -- a legal half-built cycle.
        let ordinal = match index {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 6,
            _ => continue,
        };
        conn.execute(
            "INSERT INTO cycle_day
                 (id, cycle_id, ordinal, label, weekday_hint, created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, NULL, ?, ?, 3)",
            (
                id.to_string(),
                cycle.to_string(),
                ordinal,
                format!(
                    "Day {}",
                    char::from(b'A' + u8::try_from(index).expect("small"))
                ),
                stamp,
                stamp,
            ),
        )
        .expect("write a cycle day the old build could have written");
    }
}

/// A cycle written before V9 reads as `CUSTOM`, with nothing about it rewritten.
///
/// This is the whole upgrade story, and it is the case that can only be got wrong once: a
/// released migration is immutable, so a build that classified these as `WEEKLY` — or that
/// deleted, renumbered or recreated their days to fit — could not be corrected in place.
#[test]
fn a_cycle_from_before_the_kind_existed_is_custom_and_untouched() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000009").expect("valid uuid");

    let school = Uuid::now_v7();
    let cycle = Uuid::now_v7();
    let day_ids: Vec<Uuid> = (0..6).map(|_| Uuid::now_v7()).collect();
    let stamp = "2026-10-01T09:00:00.000Z";

    // Build the project as a V8 build would have left it: a six-day rotation with its own
    // labels, one position deliberately empty, and no `kind` column in sight.
    write_a_pre_v9_project(&path, site, school, cycle, &day_ids, stamp);

    // Open it with this build, which migrates it.
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("open project");
    assert_eq!(db.schema_version(), crate::supported_schema_version());

    let stored = db
        .time_model()
        .cycle(cycle)
        .expect("read")
        .expect("the cycle survived");
    assert_eq!(
        stored.kind,
        CycleKind::Custom,
        "an existing cycle is a custom rotation: it was never claimed to be a week"
    );
    assert_eq!(stored.id, cycle, "its identifier is the one it had");
    assert_eq!(stored.name, "Six-day rotation");
    assert_eq!(stored.day_count, 6);
    assert_eq!(
        stored.week_count, 2,
        "the retired column is carried, not read"
    );
    assert!(stored.is_default);
    assert_eq!(stored.rev, 4, "the migration consumed no revision");
    assert_eq!(stored.created_at, stamp);
    assert_eq!(stored.updated_at, stamp, "and rewrote no timestamp");

    let days = db.time_model().cycle_days(cycle).expect("read days");
    assert_eq!(days.len(), 5, "nothing was created to fill the gap");
    assert_eq!(
        days.iter().map(|day| day.ordinal).collect::<Vec<_>>(),
        vec![1, 2, 4, 5, 6],
        "and nothing was renumbered to close it"
    );
    assert_eq!(
        days.iter()
            .map(|day| day.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Day A", "Day B", "Day C", "Day D", "Day E"]
    );
    for (index, day) in days.iter().enumerate() {
        assert_eq!(
            day.id, day_ids[index],
            "every cycle-day identifier is the one it had"
        );
        assert_eq!(day.rev, 3, "at the revision it was written with");
        assert_eq!(day.weekday_hint, None);
    }

    // It is still editable as a custom cycle, and still reports its gap.
    let coverage = db.time_model().cycle_coverage(cycle).expect("coverage");
    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_ordinals, vec![3]);

    let shape = db
        .time_model()
        .cycle_shape(cycle, WorkingDays::MONDAY_TO_FRIDAY)
        .expect("shape");
    assert_eq!(shape.kind, CycleKind::Custom);
    assert_eq!(shape.rotation.sequence, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(shape.rotation.wraps_to, Some(1));
    assert!(
        shape.weekdays.is_empty(),
        "a custom cycle has positions, not weekdays"
    );
}

/// The positions of a weekly cycle are the ones `evara-core` publishes, not a second list.
#[test]
fn the_weekly_positions_written_are_the_ones_the_domain_declares() {
    let mut project = Project::new();
    let cycle = project.weekly_cycle("School week");
    let days = days_of(&mut project, cycle);

    for (position, day) in WeeklyPosition::ALL.iter().zip(days.iter()) {
        assert_eq!(day.ordinal, position.ordinal);
        assert_eq!(day.weekday_hint, Some(position.weekday));
        assert_eq!(day.label, position.label);
    }
}
