//! Phase 1D tests: the cycle/period time model, and the timeslot materialiser.
//!
//! The materialiser tests are the point of the phase. Every one of them is ultimately
//! about the same invariant — a logical timeslot that survives a change keeps its
//! identifier — because once `timetable_entry` exists, breaking that is silent corruption
//! (risk R6).
//!
//! Real temporary project files throughout, never `:memory:`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use tempfile::TempDir;
use uuid::Uuid;

use crate::repo::structure::{AcademicYearInput, CampusInput, SchoolInput, TermInput};
use crate::repo::time_model::{
    CalendarDayInput, CalendarDayKind, CycleDayInput, CycleInput, MaterialisationRequest,
    PeriodInput, PeriodKind, PeriodStructureInput, Timeslot,
};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// An open project with a school, a campus and a term already in it.
///
/// `pub(crate)` because Phase 1E's tests ([`crate::tests_setup`]) build on the same
/// fixture. A second copy of "a school with a campus and an autumn term" would be the
/// kind of duplication that drifts.
pub(crate) struct Project {
    _dir: TempDir,
    pub(crate) workspace: Workspace,
    path: PathBuf,
    school: Uuid,
    pub(crate) campus: Uuid,
    pub(crate) term: Uuid,
}

impl Project {
    pub(crate) fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let mut workspace =
            Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
                .expect("workspace");
        let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");

        let db = workspace.open_database_for_test();
        let school = db
            .structure()
            .create_school(&SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
            })
            .expect("school")
            .id;
        let campus = db
            .structure()
            .create_campus(&CampusInput {
                school_id: school,
                name: "Main".to_owned(),
                code: "MAIN".to_owned(),
                address: None,
            })
            .expect("campus")
            .id;
        let year = db
            .structure()
            .create_academic_year(&AcademicYearInput {
                school_id: school,
                name: "2027–2028".to_owned(),
                starts_on: "2027-09-01".to_owned(),
                ends_on: "2028-07-20".to_owned(),
            })
            .expect("year")
            .id;
        let term = db
            .structure()
            .create_term(&TermInput {
                academic_year_id: year,
                name: "Autumn".to_owned(),
                ordinal: 1,
                starts_on: "2027-09-01".to_owned(),
                ends_on: "2027-12-17".to_owned(),
            })
            .expect("term")
            .id;

        Self {
            _dir: dir,
            workspace,
            path,
            school,
            campus,
            term,
        }
    }

    pub(crate) fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    pub(crate) fn cycle(&mut self, name: &str, day_count: i64) -> Uuid {
        let school = self.school;
        self.db()
            .time_model()
            .create_cycle(&CycleInput {
                school_id: school,
                name: name.to_owned(),
                day_count,
                week_count: 1,
                is_default: false,
            })
            .expect("cycle")
            .id
    }

    pub(crate) fn day(&mut self, cycle: Uuid, ordinal: i64, label: &str) -> Uuid {
        self.db()
            .time_model()
            .create_cycle_day(&CycleDayInput {
                cycle_id: cycle,
                ordinal,
                label: label.to_owned(),
                weekday_hint: None,
            })
            .expect("cycle day")
            .id
    }

    pub(crate) fn structure(&mut self, name: &str, campus: Option<Uuid>) -> Uuid {
        let school = self.school;
        self.db()
            .time_model()
            .create_period_structure(&PeriodStructureInput {
                school_id: school,
                campus_id: campus,
                name: name.to_owned(),
                is_default: false,
            })
            .expect("period structure")
            .id
    }

    pub(crate) fn period(
        &mut self,
        structure: Uuid,
        ordinal: i64,
        label: &str,
        kind: PeriodKind,
    ) -> Uuid {
        let starts = format!("{:02}:00", 8 + ordinal);
        let ends = format!("{:02}:50", 8 + ordinal);
        self.db()
            .time_model()
            .create_period(&PeriodInput {
                period_structure_id: structure,
                ordinal,
                label: label.to_owned(),
                starts_at: starts,
                ends_at: ends,
                kind,
                counts_as_load: kind == PeriodKind::Teaching,
            })
            .expect("period")
            .id
    }

    /// A six-day rotation with four periods: deliberately not a Monday-to-Friday week.
    pub(crate) fn six_day_grid(&mut self) -> (Uuid, Uuid, Vec<Uuid>, Vec<Uuid>) {
        let cycle = self.cycle("Six-day rotation", 6);
        let days: Vec<Uuid> = ["Day A", "Day B", "Day C", "Day D", "Day E", "Day F"]
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let ordinal = i64::try_from(index).expect("small") + 1;
                self.day(cycle, ordinal, label)
            })
            .collect();

        let structure = self.structure("Standard bells", None);
        let periods = vec![
            self.period(structure, 1, "1", PeriodKind::Teaching),
            self.period(structure, 2, "2", PeriodKind::Teaching),
            self.period(structure, 3, "Break", PeriodKind::Break),
            self.period(structure, 4, "3", PeriodKind::Teaching),
        ];

        (cycle, structure, days, periods)
    }

    pub(crate) fn slots_by_tuple(
        &mut self,
        cycle: Uuid,
        term: Option<Uuid>,
    ) -> HashMap<(Uuid, Uuid), Uuid> {
        self.db()
            .time_model()
            .timeslots(cycle, term)
            .expect("read slots")
            .into_iter()
            .map(|slot| ((slot.cycle_day_id, slot.period_id), slot.id))
            .collect()
    }

    pub(crate) fn slots(&mut self, cycle: Uuid, term: Option<Uuid>) -> Vec<Timeslot> {
        self.db()
            .time_model()
            .timeslots(cycle, term)
            .expect("read slots")
    }

    /// Closes and reopens the project, so a test can tell persisted state from in-memory
    /// state.
    pub(crate) fn reopen(&mut self) {
        self.workspace.close_project();
        self.workspace
            .open_project(&self.path)
            .expect("reopen the project");
    }

    /// Renames a cycle day, changing nothing a timeslot's identity depends on.
    pub(crate) fn relabel_day(&mut self, day: Uuid, label: &str) {
        let existing = self
            .db()
            .time_model()
            .cycle_day(day)
            .expect("read")
            .expect("the day exists");
        self.db()
            .time_model()
            .update_cycle_day(
                day,
                &CycleDayInput {
                    cycle_id: existing.cycle_id,
                    ordinal: existing.ordinal,
                    label: label.to_owned(),
                    weekday_hint: existing.weekday_hint,
                },
            )
            .expect("relabel");
    }
}

// ================================================================ the time model itself

#[test]
fn a_cycle_may_be_longer_than_a_week() {
    let mut project = Project::new();
    let cycle = project.cycle("Ten-day fortnight", 10);
    for ordinal in 1..=10 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }

    let days = project.db().time_model().cycle_days(cycle).expect("days");
    assert_eq!(days.len(), 10, "a ten-day cycle is not a special case");
    assert_eq!(
        days.iter().map(|day| day.ordinal).collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>(),
        "and its days come back in cycle order"
    );
}

#[test]
fn a_one_day_cycle_works() {
    let mut project = Project::new();
    let cycle = project.cycle("Every day the same", 1);
    project.day(cycle, 1, "Day");
    assert_eq!(
        project.db().time_model().cycle_days(cycle).unwrap().len(),
        1
    );
}

/// `ordinal` is the identity, so insertion order and labels must not affect it.
#[test]
fn cycle_day_order_is_deterministic_and_independent_of_insertion_order() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 4);
    project.day(cycle, 3, "Zebra");
    project.day(cycle, 1, "Yak");
    project.day(cycle, 4, "Xerus");
    project.day(cycle, 2, "Wombat");

    let labels: Vec<String> = project
        .db()
        .time_model()
        .cycle_days(cycle)
        .expect("days")
        .into_iter()
        .map(|day| day.label)
        .collect();

    assert_eq!(
        labels,
        vec!["Yak", "Wombat", "Zebra", "Xerus"],
        "ordinal decides the order, not the label and not the insert order"
    );
}

#[test]
fn period_order_is_deterministic() {
    let mut project = Project::new();
    let structure = project.structure("Bells", None);
    project.period(structure, 2, "Second", PeriodKind::Teaching);
    project.period(structure, 1, "First", PeriodKind::Teaching);

    let labels: Vec<String> = project
        .db()
        .time_model()
        .periods(structure)
        .expect("periods")
        .into_iter()
        .map(|period| period.label)
        .collect();
    assert_eq!(labels, vec!["First", "Second"]);
}

#[test]
fn a_cycle_day_beyond_the_declared_length_is_refused() {
    let mut project = Project::new();
    let cycle = project.cycle("Five-day week", 5);

    let error = project
        .db()
        .time_model()
        .create_cycle_day(&CycleDayInput {
            cycle_id: cycle,
            ordinal: 6,
            label: "Day F".to_owned(),
            weekday_hint: None,
        })
        .expect_err("day 6 of a five-day cycle is a contradiction");

    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
}

/// ADR 0011 Q4: the rows are the reality, so the declared length cannot shrink past them.
#[test]
fn a_cycle_cannot_be_shortened_below_a_day_it_already_has() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 6);
    project.day(cycle, 6, "Day F");

    let school = project.school;
    let error = project
        .db()
        .time_model()
        .update_cycle(
            cycle,
            &CycleInput {
                school_id: school,
                name: "Rotation".to_owned(),
                day_count: 5,
                week_count: 1,
                is_default: false,
            },
        )
        .expect_err("shortening past an existing day must be refused");

    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
}

#[test]
fn uniqueness_is_scoped_to_the_parent_not_the_table() {
    let mut project = Project::new();
    let first = project.cycle("First", 5);
    let second = project.cycle("Second", 5);

    project.day(first, 1, "Day A");
    // Same ordinal and same label, different cycle: fine.
    project.day(second, 1, "Day A");

    let duplicate = project
        .db()
        .time_model()
        .create_cycle_day(&CycleDayInput {
            cycle_id: first,
            ordinal: 1,
            label: "Something else".to_owned(),
            weekday_hint: None,
        })
        .expect_err("two day-1s in one cycle");
    assert!(
        matches!(duplicate, DbError::DuplicateValue { .. }),
        "{duplicate:?}"
    );

    let relabel = project
        .db()
        .time_model()
        .create_cycle_day(&CycleDayInput {
            cycle_id: first,
            ordinal: 2,
            label: "Day A".to_owned(),
            weekday_hint: None,
        })
        .expect_err("two Day As in one cycle");
    assert!(
        matches!(relabel, DbError::DuplicateValue { .. }),
        "{relabel:?}"
    );
}

#[test]
fn a_cycle_day_in_a_cycle_that_does_not_exist_is_refused() {
    let mut project = Project::new();
    let error = project
        .db()
        .time_model()
        .create_cycle_day(&CycleDayInput {
            cycle_id: Uuid::now_v7(),
            ordinal: 1,
            label: "Day A".to_owned(),
            weekday_hint: None,
        })
        .expect_err("a dangling cycle reference must fail");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "{error:?}"
    );
}

#[test]
fn a_period_must_end_after_it_starts() {
    let mut project = Project::new();
    let structure = project.structure("Bells", None);

    let error = project
        .db()
        .time_model()
        .create_period(&PeriodInput {
            period_structure_id: structure,
            ordinal: 1,
            label: "Backwards".to_owned(),
            starts_at: "10:00".to_owned(),
            ends_at: "09:00".to_owned(),
            kind: PeriodKind::Teaching,
            counts_as_load: true,
        })
        .expect_err("a period cannot end before it begins");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
}

#[test]
fn a_period_time_must_be_a_zero_padded_wall_clock() {
    let mut project = Project::new();
    let structure = project.structure("Bells", None);

    for bad in ["9:00", "0900", "25:00", "09:61"] {
        let error = project
            .db()
            .time_model()
            .create_period(&PeriodInput {
                period_structure_id: structure,
                ordinal: 1,
                label: "P".to_owned(),
                starts_at: bad.to_owned(),
                ends_at: "23:59".to_owned(),
                kind: PeriodKind::Teaching,
                counts_as_load: true,
            })
            .expect_err("malformed times must be refused");
        assert!(matches!(error, DbError::Invalid { .. }), "{bad}: {error:?}");
    }
}

#[test]
fn only_one_cycle_may_be_the_default() {
    let mut project = Project::new();
    let school = project.school;

    let first = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: school,
            name: "First".to_owned(),
            day_count: 5,
            week_count: 1,
            is_default: true,
        })
        .expect("first");

    let second = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: school,
            name: "Second".to_owned(),
            day_count: 6,
            week_count: 1,
            is_default: true,
        })
        .expect("second");

    let first = project
        .db()
        .time_model()
        .cycle(first.id)
        .expect("read")
        .expect("still there");

    assert!(!first.is_default, "the previous default was stood down");
    assert!(second.is_default);
    assert_eq!(
        first.rev, 2,
        "and stood down through the revision contract, not behind its back"
    );
}

/// The school-wide bucket and each campus are separate, matching the unique index.
#[test]
fn a_default_period_structure_is_per_campus() {
    let mut project = Project::new();
    let campus = project.campus;
    let school = project.school;

    let school_wide = project
        .db()
        .time_model()
        .create_period_structure(&PeriodStructureInput {
            school_id: school,
            campus_id: None,
            name: "School-wide".to_owned(),
            is_default: true,
        })
        .expect("school-wide");

    project
        .db()
        .time_model()
        .create_period_structure(&PeriodStructureInput {
            school_id: school,
            campus_id: Some(campus),
            name: "Main campus".to_owned(),
            is_default: true,
        })
        .expect("campus default");

    let school_wide = project
        .db()
        .time_model()
        .period_structure(school_wide.id)
        .expect("read")
        .expect("still there");
    assert!(
        school_wide.is_default,
        "a campus default must not stand down the school-wide one"
    );
}

#[test]
fn a_created_row_starts_at_revision_one() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 6);
    let day = project.day(cycle, 1, "Day A");

    assert_eq!(
        project.db().time_model().cycle(cycle).unwrap().unwrap().rev,
        1
    );
    assert_eq!(
        project
            .db()
            .time_model()
            .cycle_day(day)
            .unwrap()
            .unwrap()
            .rev,
        1
    );
}

#[test]
fn an_update_consumes_exactly_one_revision() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 6);
    let day = project.day(cycle, 1, "Day A");
    let before = project.db().time_model().cycle_day(day).unwrap().unwrap();

    // Timestamps have millisecond precision, so the two writes need to be distinguishable.
    std::thread::sleep(std::time::Duration::from_millis(5));
    let after = project
        .db()
        .time_model()
        .update_cycle_day(
            day,
            &CycleDayInput {
                cycle_id: cycle,
                ordinal: 1,
                label: "Monday".to_owned(),
                weekday_hint: Some(1),
            },
        )
        .expect("update");

    assert_eq!(after.rev, before.rev + 1, "exactly one revision");
    assert!(
        after.updated_at > before.updated_at,
        "updated_at must advance: {} -> {}",
        before.updated_at,
        after.updated_at
    );
    assert_eq!(after.created_at, before.created_at, "creation is history");
}

// ============================================================ materialisation: identity

#[test]
fn initial_materialisation_creates_the_whole_grid() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");

    assert_eq!(plan.counts().created, days.len() * periods.len());
    assert_eq!(plan.counts().preserved, 0);
    assert_eq!(plan.counts().orphaned, 0);
    assert!(!plan.would_orphan());

    let slots = project.slots(cycle, Some(term));
    assert_eq!(slots.len(), 24, "six days by four periods");
    assert!(
        slots.iter().all(|slot| slot.rev == 1),
        "a created slot starts at revision 1"
    );

    // ordinal runs 1..n in (day, period) order, with no gaps and no repeats.
    assert_eq!(
        slots.iter().map(|slot| slot.ordinal).collect::<Vec<_>>(),
        (1..=24).collect::<Vec<_>>()
    );

    // is_teaching is derived from the period's kind: the break column is not teachable.
    let break_period = periods[2];
    assert_eq!(
        slots
            .iter()
            .filter(|slot| slot.period_id == break_period)
            .filter(|slot| !slot.is_teaching)
            .count(),
        6,
        "every break slot is non-teaching, one per day"
    );
}

/// The invariant, stated as plainly as a test can state it.
#[test]
fn rematerialising_preserves_every_single_identifier() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let term = project.term;
    let request = MaterialisationRequest::new(cycle, structure, Some(term));

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("first");
    let before = project.slots_by_tuple(cycle, Some(term));

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("second");
    let after = project.slots_by_tuple(cycle, Some(term));

    assert_eq!(before, after, "every tuple kept the identifier it had");
    assert_eq!(plan.counts().created, 0);
    assert_eq!(plan.counts().preserved, before.len());
}

/// Idempotent means "wrote nothing", not "ended up with the same number of rows".
#[test]
fn a_second_materialisation_consumes_no_revisions() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let term = project.term;
    let request = MaterialisationRequest::new(cycle, structure, Some(term));

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("first");
    let before = project.slots(cycle, Some(term));

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("second");
    let after = project.slots(cycle, Some(term));

    assert_eq!(plan.counts().refreshed, 0, "nothing needed refreshing");
    assert_eq!(before, after, "not one column of one row changed");
    assert!(
        after.iter().all(|slot| slot.rev == 1),
        "and no revision was consumed"
    );
}

#[test]
fn adding_a_period_preserves_old_identifiers_and_creates_only_the_new_tuples() {
    let mut project = Project::new();
    let (cycle, structure, days, _) = project.six_day_grid();
    let term = project.term;
    let request = MaterialisationRequest::new(cycle, structure, Some(term));

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("first");
    let before = project.slots_by_tuple(cycle, Some(term));

    // Deliberately in the middle of the day, so every later slot's ordinal shifts.
    let inserted = project.period(structure, 5, "4", PeriodKind::Teaching);

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("rematerialise");

    assert_eq!(plan.counts().created, days.len(), "one new slot per day");
    assert_eq!(plan.counts().preserved, before.len());
    assert_eq!(plan.counts().orphaned, 0);

    let after = project.slots_by_tuple(cycle, Some(term));
    for (tuple, id) in &before {
        assert_eq!(
            after.get(tuple),
            Some(id),
            "an existing tuple must keep its identifier"
        );
    }
    assert_eq!(
        after.len(),
        before.len() + days.len(),
        "and only the new tuples are added"
    );
    assert!(
        after.keys().any(|(_, period)| *period == inserted),
        "the new period is in the grid"
    );

    // The renumbering really did happen — ordinal is derived, not identity.
    let slots = project.slots(cycle, Some(term));
    assert_eq!(
        slots.iter().map(|slot| slot.ordinal).collect::<Vec<_>>(),
        (1..=30).collect::<Vec<_>>()
    );
}

#[test]
fn adding_a_cycle_day_preserves_old_identifiers_and_creates_only_the_new_tuples() {
    let mut project = Project::new();
    let cycle = project.cycle("Growing rotation", 7);
    for ordinal in 1..=6 {
        project.day(cycle, ordinal, &format!("Day {ordinal}"));
    }
    let structure = project.structure("Bells", None);
    let periods = [
        project.period(structure, 1, "1", PeriodKind::Teaching),
        project.period(structure, 2, "2", PeriodKind::Teaching),
    ];
    let term = project.term;
    let request = MaterialisationRequest::new(cycle, structure, Some(term));

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("first");
    let before = project.slots_by_tuple(cycle, Some(term));

    project.day(cycle, 7, "Day 7");

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("rematerialise");

    assert_eq!(plan.counts().created, periods.len());
    assert_eq!(plan.counts().preserved, before.len());
    assert_eq!(plan.counts().orphaned, 0);

    let after = project.slots_by_tuple(cycle, Some(term));
    for (tuple, id) in &before {
        assert_eq!(after.get(tuple), Some(id));
    }
    assert_eq!(after.len(), 14, "seven days by two periods");
}

#[test]
fn reopening_the_project_preserves_every_identifier() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");
    let before = project.slots_by_tuple(cycle, Some(term));

    project.workspace.close_project();
    project
        .workspace
        .open_project(&project.path)
        .expect("reopen");

    assert_eq!(
        project.slots_by_tuple(cycle, Some(term)),
        before,
        "identity is persisted, not an artefact of one session"
    );
}

// ============================================================== materialisation: orphans

#[test]
fn removing_a_period_is_reported_as_orphaned_without_deleting_anything() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");
    let before = project.slots_by_tuple(cycle, Some(term));

    let doomed = periods[1];
    let proposal = MaterialisationRequest::new(cycle, structure, Some(term)).without_period(doomed);

    let plan = project
        .db()
        .time_model()
        .plan_materialisation(&proposal)
        .expect("plan");

    assert!(plan.would_orphan(), "the caller must be able to see this");
    assert_eq!(plan.counts().orphaned, days.len(), "one slot per day");
    assert_eq!(plan.counts().created, 0);
    assert!(
        plan.orphaned.iter().all(|slot| slot.period_id == doomed),
        "and exactly which slots they are"
    );

    assert_eq!(
        project.slots_by_tuple(cycle, Some(term)),
        before,
        "planning changed nothing"
    );

    // Applying the same proposal is equally non-destructive.
    let applied = project
        .db()
        .time_model()
        .apply_materialisation(&proposal)
        .expect("apply");
    assert_eq!(applied.counts().orphaned, days.len());
    assert_eq!(
        project.slots_by_tuple(cycle, Some(term)),
        before,
        "apply creates and refreshes; it never deletes"
    );
}

#[test]
fn removing_a_cycle_day_is_reported_as_orphaned_without_deleting_anything() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");
    let before = project.slots_by_tuple(cycle, Some(term));

    let doomed = days[5];
    let plan = project
        .db()
        .time_model()
        .plan_materialisation(
            &MaterialisationRequest::new(cycle, structure, Some(term)).without_cycle_day(doomed),
        )
        .expect("plan");

    assert_eq!(plan.counts().orphaned, periods.len());
    assert!(plan.orphaned.iter().all(|slot| slot.cycle_day_id == doomed));
    assert_eq!(project.slots_by_tuple(cycle, Some(term)), before);
}

/// The RESTRICT that makes the exclusion-based API necessary in the first place.
#[test]
fn a_materialised_grid_blocks_deleting_the_day_or_period_it_came_from() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");

    let day = project
        .db()
        .time_model()
        .delete_cycle_day(days[0])
        .expect_err("a materialised day cannot just vanish");
    assert!(matches!(day, DbError::StillReferenced { .. }), "{day:?}");

    let period = project
        .db()
        .time_model()
        .delete_period(periods[0])
        .expect_err("nor a materialised period");
    assert!(
        matches!(period, DbError::StillReferenced { .. }),
        "{period:?}"
    );
}

/// The full confirmed-removal workflow, which is the reason the API is shaped this way.
#[test]
fn releasing_orphans_explicitly_is_what_finally_allows_a_removal() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");

    let doomed = days[5];
    let plan = project
        .db()
        .time_model()
        .plan_materialisation(
            &MaterialisationRequest::new(cycle, structure, Some(term)).without_cycle_day(doomed),
        )
        .expect("plan");

    // Step 2: a human has now seen the counts and agreed.
    project
        .db()
        .time_model()
        .delete_timeslots(&plan.orphaned_ids())
        .expect("release the slots the plan named");

    // Step 3: only now will the day go.
    project
        .db()
        .time_model()
        .delete_cycle_day(doomed)
        .expect("the obstruction is gone");

    let remaining = project.slots(cycle, Some(term));
    assert_eq!(
        remaining.len(),
        (days.len() - 1) * periods.len(),
        "exactly the named slots went, and nothing else"
    );
    assert!(
        remaining.iter().all(|slot| slot.cycle_day_id != doomed),
        "none of the deleted day's slots survive"
    );

    // And the survivors still have the identifiers they started with.
    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("rematerialise");
    assert_eq!(plan.counts().created, 0, "nothing was lost and regenerated");
    assert_eq!(plan.counts().orphaned, 0, "the grid is settled again");
}

#[test]
fn deleting_a_timeslot_that_does_not_exist_is_an_error_not_a_silent_no_op() {
    let mut project = Project::new();
    let error = project
        .db()
        .time_model()
        .delete_timeslots(&[Uuid::now_v7()])
        .expect_err("a phantom identifier must not pass quietly");
    assert!(matches!(error, DbError::NotFound { .. }), "{error:?}");
}

// =========================================================== materialisation: integrity

#[test]
fn a_failed_materialisation_rolls_back_completely() {
    let mut project = Project::new();
    let (cycle, structure, _, periods) = project.six_day_grid();
    let term = project.term;

    // A duplicate identity tuple cannot be used to force this: any row matching the tuple
    // is, by construction, inside the grid's scope, so the materialiser correctly treats it
    // as an existing slot and preserves it. The failure is injected instead, with a trigger
    // that refuses one particular period — so the run dies after three successful inserts
    // rather than at the first, which is the case a rollback actually has to survive.
    let doomed = periods[3];
    project
        .db()
        .write(|tx| {
            tx.execute_batch(&format!(
                "CREATE TRIGGER test_refuse_one_period
                 BEFORE INSERT ON timeslot FOR EACH ROW
                 WHEN NEW.period_id = '{doomed}'
                 BEGIN
                     SELECT RAISE(ABORT, 'simulated mid-run failure');
                 END;"
            ))?;
            Ok(())
        })
        .expect("install the failure");

    assert!(
        project.slots(cycle, Some(term)).is_empty(),
        "the grid starts empty"
    );

    let error = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect_err("the injected failure must abort the run");
    // The specific variant is not the point; that nothing survived is.
    let _ = error;

    assert!(
        project.slots(cycle, Some(term)).is_empty(),
        "a failed materialisation leaves no slots at all — not the three it managed to          insert before the failure"
    );

    // With the obstruction gone the same request succeeds completely, which proves the
    // rollback left no half-written state behind either.
    project
        .db()
        .write(|tx| {
            tx.execute_batch("DROP TRIGGER test_refuse_one_period")?;
            Ok(())
        })
        .expect("remove the failure");

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");
    assert_eq!(plan.counts().created, 24);
    assert_eq!(
        plan.counts().preserved,
        0,
        "nothing was left over to preserve"
    );
}

/// Two campuses on different bells share a cycle. Materialising one must ignore the other.
#[test]
fn sibling_grids_over_the_same_cycle_do_not_orphan_each_other() {
    let mut project = Project::new();
    let (cycle, main_structure, days, _) = project.six_day_grid();
    let term = project.term;
    let campus = project.campus;

    let annexe = project.structure("Annexe bells", Some(campus));
    project.period(annexe, 1, "A1", PeriodKind::Teaching);

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(
            cycle,
            main_structure,
            Some(term),
        ))
        .expect("main grid");
    let main_slots = project.slots_by_tuple(cycle, Some(term));

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, annexe, Some(term)))
        .expect("annexe grid");

    assert_eq!(plan.counts().created, days.len());
    assert_eq!(
        plan.counts().orphaned,
        0,
        "the main campus's slots are a different grid, not orphans"
    );

    let after = project.slots_by_tuple(cycle, Some(term));
    for (tuple, id) in &main_slots {
        assert_eq!(after.get(tuple), Some(id), "and they were left alone");
    }
}

/// `term_id IS NULL` means "the whole year" (ADR 0011 Q1) and is its own grid.
#[test]
fn a_year_wide_grid_is_distinct_from_a_terms_grid() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;

    let year_wide = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, None))
        .expect("year-wide");
    assert_eq!(year_wide.counts().created, days.len() * periods.len());

    let per_term = project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("per-term");
    assert_eq!(
        per_term.counts().created,
        days.len() * periods.len(),
        "a different term scope is a different grid, not the same one"
    );
    assert_eq!(per_term.counts().orphaned, 0);
    assert_eq!(per_term.counts().preserved, 0);

    assert_eq!(project.slots(cycle, None).len(), 24);
    assert_eq!(project.slots(cycle, Some(term)).len(), 24);
}

/// The hole the COALESCE unique index exists to close.
#[test]
fn the_identity_constraint_also_holds_for_the_term_less_grid() {
    let mut project = Project::new();
    let (cycle, structure, _, periods) = project.six_day_grid();

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, None))
        .expect("materialise");

    let days = project.db().time_model().cycle_days(cycle).expect("days");
    let school = project.school;
    let duplicate = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO timeslot
                 (id, school_id, cycle_id, cycle_day_id, period_id, campus_id, term_id,
                  ordinal, is_teaching, created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, NULL, NULL, 99, 1, ?, ?, 1)",
            (
                Uuid::now_v7().to_string(),
                school.to_string(),
                cycle.to_string(),
                days[0].id.to_string(),
                periods[0].to_string(),
                "2026-10-09T00:00:00.000Z",
                "2026-10-09T00:00:00.000Z",
            ),
        )?;
        Ok(())
    });

    assert!(
        duplicate.is_err(),
        "a plain UNIQUE would have allowed a second (day, period, NULL) slot"
    );
}

#[test]
fn a_grid_from_an_unknown_cycle_or_schedule_is_refused() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();

    let no_cycle = project
        .db()
        .time_model()
        .plan_materialisation(&MaterialisationRequest::new(
            Uuid::now_v7(),
            structure,
            None,
        ))
        .expect_err("an unknown cycle is not an empty grid");
    assert!(matches!(no_cycle, DbError::NotFound { .. }), "{no_cycle:?}");

    let no_term = project
        .db()
        .time_model()
        .plan_materialisation(&MaterialisationRequest::new(
            cycle,
            structure,
            Some(Uuid::now_v7()),
        ))
        .expect_err("an unknown term is not the whole year");
    assert!(matches!(no_term, DbError::NotFound { .. }), "{no_term:?}");
}

/// Derived columns follow their source, without disturbing identity.
#[test]
fn changing_a_periods_kind_refreshes_the_grid_without_renaming_anything() {
    let mut project = Project::new();
    let (cycle, structure, days, periods) = project.six_day_grid();
    let term = project.term;
    let request = MaterialisationRequest::new(cycle, structure, Some(term));

    project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("materialise");
    let before = project.slots_by_tuple(cycle, Some(term));

    // The break becomes a teaching period.
    let was_break = periods[2];
    project
        .db()
        .time_model()
        .update_period(
            was_break,
            &PeriodInput {
                period_structure_id: structure,
                ordinal: 3,
                label: "Break".to_owned(),
                starts_at: "11:00".to_owned(),
                ends_at: "11:50".to_owned(),
                kind: PeriodKind::Teaching,
                counts_as_load: true,
            },
        )
        .expect("update period");

    let plan = project
        .db()
        .time_model()
        .apply_materialisation(&request)
        .expect("rematerialise");

    assert_eq!(plan.counts().created, 0);
    assert_eq!(
        plan.counts().refreshed,
        days.len(),
        "one refresh per affected slot, and no more"
    );
    assert_eq!(
        project.slots_by_tuple(cycle, Some(term)),
        before,
        "refreshing derived data must not touch identity"
    );

    let slots = project.slots(cycle, Some(term));
    assert!(
        slots
            .iter()
            .filter(|slot| slot.period_id == was_break)
            .all(|slot| slot.is_teaching && slot.rev == 2),
        "the derived flag followed its source, through the revision contract"
    );
}

// ======================================================================== calendar days

#[test]
fn explicit_dates_map_onto_arbitrarily_named_cycle_days() {
    let mut project = Project::new();
    let cycle = project.cycle("Three-day rotation", 3);
    let a = project.day(cycle, 1, "Day A");
    let b = project.day(cycle, 2, "Day B");
    let c = project.day(cycle, 3, "Day C");
    let school = project.school;
    let term = project.term;

    for (date, day) in [
        ("2027-09-01", a),
        ("2027-09-02", b),
        ("2027-09-03", c),
        // The rotation wraps, and it does not care that the 4th is a Saturday.
        ("2027-09-06", a),
    ] {
        project
            .db()
            .time_model()
            .create_calendar_day(&CalendarDayInput {
                school_id: school,
                date: date.to_owned(),
                term_id: Some(term),
                campus_id: None,
                cycle_day_id: Some(day),
                kind: CalendarDayKind::School,
                note: None,
            })
            .expect("calendar day");
    }

    let mapped = project
        .db()
        .time_model()
        .calendar_days("2027-09-01", "2027-09-30")
        .expect("read calendar");

    assert_eq!(
        mapped
            .iter()
            .map(|entry| (entry.date.clone(), entry.cycle_day_id))
            .collect::<Vec<_>>(),
        vec![
            ("2027-09-01".to_owned(), Some(a)),
            ("2027-09-02".to_owned(), Some(b)),
            ("2027-09-03".to_owned(), Some(c)),
            ("2027-09-06".to_owned(), Some(a)),
        ],
        "a three-day cycle projected onto dates, with no weekday arithmetic anywhere"
    );
}

#[test]
fn a_date_with_no_cycle_day_is_a_day_without_lessons() {
    let mut project = Project::new();
    let school = project.school;

    let holiday = project
        .db()
        .time_model()
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-12-25".to_owned(),
            term_id: None,
            campus_id: None,
            cycle_day_id: None,
            kind: CalendarDayKind::Holiday,
            note: Some("Closed".to_owned()),
        })
        .expect("holiday");

    assert_eq!(holiday.cycle_day_id, None);
    assert_eq!(holiday.kind, CalendarDayKind::Holiday);
}

#[test]
fn one_date_cannot_be_recorded_twice_for_the_same_campus() {
    let mut project = Project::new();
    let school = project.school;
    let campus = project.campus;

    let entry = |campus_id| CalendarDayInput {
        school_id: school,
        date: "2027-09-01".to_owned(),
        term_id: None,
        campus_id,
        cycle_day_id: None,
        kind: CalendarDayKind::School,
        note: None,
    };

    project
        .db()
        .time_model()
        .create_calendar_day(&entry(None))
        .expect("school-wide");
    // A campus entry for the same date is a different row: campuses may differ.
    project
        .db()
        .time_model()
        .create_calendar_day(&entry(Some(campus)))
        .expect("campus-specific");

    // But the school-wide bucket must not take a second entry — the case a plain
    // UNIQUE(school_id, date, campus_id) would have let through.
    let error = project
        .db()
        .time_model()
        .create_calendar_day(&entry(None))
        .expect_err("two school-wide entries for one date");
    assert!(matches!(error, DbError::DuplicateValue { .. }), "{error:?}");
}

#[test]
fn a_mapped_date_blocks_deleting_the_cycle_day_it_names() {
    let mut project = Project::new();
    let cycle = project.cycle("Rotation", 3);
    let day = project.day(cycle, 1, "Day A");
    let school = project.school;

    project
        .db()
        .time_model()
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-09-01".to_owned(),
            term_id: None,
            campus_id: None,
            cycle_day_id: Some(day),
            kind: CalendarDayKind::School,
            note: None,
        })
        .expect("calendar day");

    let error = project
        .db()
        .time_model()
        .delete_cycle_day(day)
        .expect_err("ADR 0011 Q2: RESTRICT, not a silent reinterpretation");
    assert!(
        matches!(error, DbError::StillReferenced { .. }),
        "{error:?}"
    );

    let still_mapped = project
        .db()
        .time_model()
        .calendar_days("2027-09-01", "2027-09-01")
        .expect("read");
    assert_eq!(
        still_mapped[0].cycle_day_id,
        Some(day),
        "and the date was not quietly turned into a non-teaching day"
    );
}

#[test]
fn a_malformed_date_is_refused() {
    let mut project = Project::new();
    let school = project.school;

    for bad in ["2027-9-1", "01/09/2027", "2027-02-30", "not a date"] {
        let error = project
            .db()
            .time_model()
            .create_calendar_day(&CalendarDayInput {
                school_id: school,
                date: bad.to_owned(),
                term_id: None,
                campus_id: None,
                cycle_day_id: None,
                kind: CalendarDayKind::School,
                note: None,
            })
            .expect_err("only canonical YYYY-MM-DD is stored");
        assert!(matches!(error, DbError::Invalid { .. }), "{bad}: {error:?}");
    }
}

// ================================================================ change-log attribution

/// The feed must carry the new tables, attributed to the installation that wrote them.
#[test]
fn materialisation_is_attributed_to_the_active_installation() {
    let mut project = Project::new();
    let (cycle, structure, _, _) = project.six_day_grid();
    let term = project.term;

    project
        .db()
        .time_model()
        .apply_materialisation(&MaterialisationRequest::new(cycle, structure, Some(term)))
        .expect("materialise");

    let site = project.workspace.site_id().to_string();
    let changes = project.db().changes_since(0, 1000).expect("changes");

    let tables: BTreeSet<&str> = changes
        .iter()
        .map(|record| record.entity_table.as_str())
        .collect();
    for expected in [
        "cycle",
        "cycle_day",
        "period_structure",
        "period",
        "timeslot",
    ] {
        assert!(
            tables.contains(expected),
            "`{expected}` must appear in the change feed"
        );
    }

    assert!(
        changes.iter().all(|record| record.site_id == site),
        "every change is attributed to the installation that made it"
    );
    assert!(
        changes
            .iter()
            .filter(|record| record.entity_table == "timeslot")
            .all(|record| record.op == "INSERT" && record.rev == 1),
        "created slots are logged as inserts at revision 1"
    );
}
