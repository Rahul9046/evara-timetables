//! One end-to-end walk through School Setup, in the order a person would do it.
//!
//! The other test modules check each rule in isolation, which is what makes a failure
//! easy to read. This one does the opposite: it performs the nine sections in sequence,
//! on one project, through the same repository calls the commands make, and then closes
//! and reopens the file. Nothing here is a new rule — it is the *chain* that is under
//! test, because a project can be correct rule by rule and still not be completable.
//!
//! It is also the closest automated stand-in for clicking through the window. It is not a
//! substitute for doing that, and is not reported as one.
//!
//! The school is synthetic, deliberately not a five-day week, and deliberately not
//! Monday-to-Friday in its labels.

use crate::repo::structure::{
    AcademicYearInput, BuildingInput, CampusInput, ResourceInput, RoomInput, RoomTypeInput,
    SchoolInput, TermInput,
};
use crate::repo::time_model::{
    CalendarDayInput, CalendarDayKind, CycleDayInput, CycleInput, MaterialisationRequest,
    PeriodInput, PeriodKind, PeriodStructureInput,
};
use crate::workspace::Workspace;
use crate::{DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the point of this test is the whole sequence in one place"
)]
fn a_school_can_be_set_up_from_an_empty_project_and_survives_a_reopen() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let mut workspace =
        Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
            .expect("workspace");
    let path = dir.path().join(format!("Northgate.{PROJECT_EXTENSION}"));
    workspace.create_project(&path).expect("create project");

    // ---------------------------------------------------- 1. school details
    // Nothing exists yet, which is what drives the first-run empty state.
    assert_eq!(
        workspace.structure().expect("open").school().expect("read"),
        None,
        "a new project has no school, and the interface opens on that"
    );

    let school = workspace
        .structure()
        .expect("open")
        .create_school(&SchoolInput {
            name: "Northgate Secondary".to_owned(),
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
        })
        .expect("school")
        .id;

    // ------------------------------------------- 2. campuses and buildings
    let campus = workspace
        .structure()
        .expect("open")
        .create_campus(&CampusInput {
            school_id: school,
            name: "Riverside".to_owned(),
            code: "RIV".to_owned(),
            address: Some("1 Example Way".to_owned()),
        })
        .expect("campus")
        .id;
    let building = workspace
        .structure()
        .expect("open")
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Science Block".to_owned(),
        })
        .expect("building")
        .id;

    // ------------------------------------------ 3. rooms and room types
    let room_type = workspace
        .structure()
        .expect("open")
        .create_room_type(&RoomTypeInput {
            school_id: school,
            name: "Science Lab".to_owned(),
            code: "LAB".to_owned(),
        })
        .expect("room type")
        .id;
    let classified = workspace
        .structure()
        .expect("open")
        .create_room(&RoomInput {
            campus_id: campus,
            building_id: Some(building),
            room_type_id: Some(room_type),
            name: "Lab 1".to_owned(),
            code: "L1".to_owned(),
            capacity: 24,
            is_bookable: true,
            notes: None,
        })
        .expect("room")
        .id;
    // A room with neither optional reference is a normal state, not an incomplete one.
    workspace
        .structure()
        .expect("open")
        .create_room(&RoomInput {
            campus_id: campus,
            building_id: None,
            room_type_id: None,
            name: "Hall".to_owned(),
            code: "HALL".to_owned(),
            capacity: 200,
            is_bookable: false,
            notes: Some("Assembly only".to_owned()),
        })
        .expect("unclassified room");

    // ------------------------------------------------------- 4. resources
    workspace
        .structure()
        .expect("open")
        .create_resource(&ResourceInput {
            school_id: school,
            campus_id: None,
            name: "Projector trolley".to_owned(),
            code: "PROJ".to_owned(),
            quantity: 3,
        })
        .expect("resource");

    // ------------------------------------------- 5. academic year and terms
    let year = workspace
        .structure()
        .expect("open")
        .create_academic_year(&AcademicYearInput {
            school_id: school,
            name: "2027/28".to_owned(),
            starts_on: "2027-09-01".to_owned(),
            ends_on: "2028-07-20".to_owned(),
        })
        .expect("year")
        .id;
    let term = workspace
        .structure()
        .expect("open")
        .create_term(&TermInput {
            academic_year_id: year,
            name: "Autumn".to_owned(),
            ordinal: 1,
            starts_on: "2027-09-01".to_owned(),
            ends_on: "2027-12-17".to_owned(),
        })
        .expect("term")
        .id;

    // ------------------------------------------- 6. a six-day rotating cycle
    let cycle = workspace
        .time_model()
        .expect("open")
        .create_cycle(&CycleInput {
            school_id: school,
            name: "Six-day rotation".to_owned(),
            day_count: 6,
            week_count: 2,
            is_default: true,
        })
        .expect("cycle")
        .id;

    // Add four of the six days, as someone building it a bit at a time would.
    let mut days = Vec::new();
    for (index, label) in ["Day A", "Day B", "Day C", "Day D"].iter().enumerate() {
        let ordinal = i64::try_from(index).expect("small") + 1;
        days.push(
            workspace
                .time_model()
                .expect("open")
                .create_cycle_day(&CycleDayInput {
                    cycle_id: cycle,
                    ordinal,
                    label: (*label).to_owned(),
                    weekday_hint: None,
                })
                .expect("cycle day")
                .id,
        );
    }

    // The interface must be able to say the cycle is unfinished, and which days are missing.
    let coverage = workspace
        .time_model()
        .expect("open")
        .cycle_coverage(cycle)
        .expect("coverage");
    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_ordinals, vec![5, 6]);

    // Finish it — what the "add the missing positions" action does.
    for ordinal in coverage.missing_ordinals.clone() {
        days.push(
            workspace
                .time_model()
                .expect("open")
                .create_cycle_day(&CycleDayInput {
                    cycle_id: cycle,
                    ordinal,
                    label: format!("Day {ordinal}"),
                    weekday_hint: None,
                })
                .expect("cycle day")
                .id,
        );
    }
    assert!(
        workspace
            .time_model()
            .expect("open")
            .cycle_coverage(cycle)
            .expect("coverage")
            .is_complete(),
        "six of six positions now have a day"
    );

    // ------------------------------------- 7. a bell schedule with a break
    let bells = workspace
        .time_model()
        .expect("open")
        .create_period_structure(&PeriodStructureInput {
            school_id: school,
            campus_id: Some(campus),
            name: "Riverside bells".to_owned(),
            is_default: true,
        })
        .expect("bell schedule")
        .id;
    let periods = [
        (1, "P1", "09:00", "09:50", PeriodKind::Teaching),
        (2, "P2", "09:55", "10:45", PeriodKind::Teaching),
        (3, "Break", "10:45", "11:05", PeriodKind::Break),
        (4, "P3", "11:05", "11:55", PeriodKind::Teaching),
    ];
    for (ordinal, label, starts, ends, kind) in periods {
        workspace
            .time_model()
            .expect("open")
            .create_period(&PeriodInput {
                period_structure_id: bells,
                ordinal,
                label: label.to_owned(),
                starts_at: starts.to_owned(),
                ends_at: ends.to_owned(),
                kind,
                counts_as_load: kind == PeriodKind::Teaching,
            })
            .expect("period");
    }

    // ----------------------------------------------- 8. calendar mapping
    // One teaching date, and one closure with no cycle day at all.
    workspace
        .time_model()
        .expect("open")
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-09-06".to_owned(),
            term_id: Some(term),
            campus_id: Some(campus),
            cycle_day_id: Some(days[0]),
            kind: CalendarDayKind::School,
            note: None,
        })
        .expect("teaching date");
    workspace
        .time_model()
        .expect("open")
        .create_calendar_day(&CalendarDayInput {
            school_id: school,
            date: "2027-12-25".to_owned(),
            term_id: Some(term),
            campus_id: Some(campus),
            cycle_day_id: None,
            kind: CalendarDayKind::Holiday,
            note: Some("Winter closure".to_owned()),
        })
        .expect("closure");

    // ------------------------------------------------- 9. the timetable grid
    let request = MaterialisationRequest::new(cycle, bells, Some(term));

    let before = workspace
        .time_model()
        .expect("open")
        .grid_preview(&request)
        .expect("preview");
    assert_eq!(before.days.len(), 6);
    assert_eq!(before.periods.len(), 4);
    assert_eq!(before.counts.created, 24, "nothing is built yet");
    assert!(!before.is_complete());

    // Confirm with the fingerprint the preview issued, as the screen does.
    let plan = workspace
        .time_model()
        .expect("open")
        .apply_materialisation_reviewed(&request, &before.fingerprint)
        .expect("rebuild");
    assert_eq!(plan.created.len(), 24);
    assert!(!plan.would_orphan());

    let after = workspace
        .time_model()
        .expect("open")
        .grid_preview(&request)
        .expect("preview");
    assert!(after.is_complete(), "a finished grid over a finished cycle");
    let identifiers: Vec<_> = after.cells.iter().map(|cell| cell.timeslot_id).collect();
    assert!(identifiers.iter().all(Option::is_some));

    // ------------------------------------- the whole thing survives a reopen
    workspace.close_project();
    workspace.open_project(&path).expect("reopen");

    let reopened = workspace
        .time_model()
        .expect("open")
        .grid_preview(&request)
        .expect("preview");
    assert_eq!(
        reopened.cells, after.cells,
        "every slot identifier is persisted, not an artefact of one session"
    );
    assert_eq!(reopened.fingerprint, after.fingerprint);

    let structure = workspace.structure().expect("open");
    assert_eq!(
        structure.school().expect("read").map(|s| s.name).as_deref(),
        Some("Northgate Secondary")
    );
    assert_eq!(structure.campuses().expect("read").len(), 1);
    assert_eq!(structure.rooms(campus).expect("read").len(), 2);
    assert_eq!(structure.resources().expect("read").len(), 1);
    assert_eq!(structure.terms(year).expect("read").len(), 1);

    let time_model = workspace.time_model().expect("open");
    assert_eq!(time_model.cycle_days(cycle).expect("read").len(), 6);
    assert_eq!(time_model.periods(bells).expect("read").len(), 4);
    assert_eq!(
        time_model
            .calendar_days("2027-09-01", "2028-07-20")
            .expect("read")
            .len(),
        2
    );

    // And the nullable room reference is still exactly as it was set.
    let room = workspace
        .structure()
        .expect("open")
        .room(classified)
        .expect("read")
        .expect("the lab");
    assert_eq!(room.building_id, Some(building));
    assert_eq!(room.room_type_id, Some(room_type));
}

/// Removing a cycle day, the long way round — which is the only way there is.
///
/// This is the workflow §6 of the brief describes, performed in order: the delete is
/// refused, the preview names what is in the way, the stranded slots are released after a
/// confirmation, and only then does the day go. Each step exists in isolation elsewhere;
/// what this pins down is that they compose into something a user can actually finish.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the sequence is the subject; splitting it would hide the order"
)]
fn removing_a_cycle_day_takes_the_whole_guarded_route() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let mut workspace =
        Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
            .expect("workspace");
    let path = dir.path().join(format!("Northgate.{PROJECT_EXTENSION}"));
    workspace.create_project(&path).expect("create project");

    let school = workspace
        .structure()
        .expect("open")
        .create_school(&SchoolInput {
            name: "Northgate Secondary".to_owned(),
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
        })
        .expect("school")
        .id;
    let cycle = workspace
        .time_model()
        .expect("open")
        .create_cycle(&CycleInput {
            school_id: school,
            name: "Three-day rotation".to_owned(),
            day_count: 3,
            week_count: 1,
            is_default: true,
        })
        .expect("cycle")
        .id;
    let mut days = Vec::new();
    for ordinal in 1..=3 {
        days.push(
            workspace
                .time_model()
                .expect("open")
                .create_cycle_day(&CycleDayInput {
                    cycle_id: cycle,
                    ordinal,
                    label: format!("Day {ordinal}"),
                    weekday_hint: None,
                })
                .expect("cycle day")
                .id,
        );
    }
    let bells = workspace
        .time_model()
        .expect("open")
        .create_period_structure(&PeriodStructureInput {
            school_id: school,
            campus_id: None,
            name: "Bells".to_owned(),
            is_default: true,
        })
        .expect("bells")
        .id;
    workspace
        .time_model()
        .expect("open")
        .create_period(&PeriodInput {
            period_structure_id: bells,
            ordinal: 1,
            label: "P1".to_owned(),
            starts_at: "09:00".to_owned(),
            ends_at: "09:50".to_owned(),
            kind: PeriodKind::Teaching,
            counts_as_load: true,
        })
        .expect("period");

    let request = MaterialisationRequest::new(cycle, bells, None);
    workspace
        .time_model()
        .expect("open")
        .apply_materialisation(&request)
        .expect("build");

    // Step 1: the straightforward attempt is refused, and says why.
    let refused = workspace
        .time_model()
        .expect("open")
        .delete_cycle_day(days[2])
        .expect_err("the grid holds the day");
    assert!(
        matches!(refused, DbError::StillReferenced { .. }),
        "{refused:?}"
    );

    // Step 2: preview the grid without that day, to see what would be stranded.
    let proposal = MaterialisationRequest::new(cycle, bells, None).without_cycle_day(days[2]);
    let preview = workspace
        .time_model()
        .expect("open")
        .grid_preview(&proposal)
        .expect("preview");
    assert_eq!(preview.counts.orphaned, 1);
    assert_eq!(preview.counts.created, 0);

    // Step 3: a rebuild would still not remove them. Proven, not assumed.
    workspace
        .time_model()
        .expect("open")
        .apply_materialisation_reviewed(&proposal, &preview.fingerprint)
        .expect("rebuild");
    assert_eq!(
        workspace
            .time_model()
            .expect("open")
            .timeslots(cycle, None)
            .expect("read")
            .len(),
        3,
        "a rebuild never deletes"
    );

    // Step 4: release them explicitly, against a freshly reviewed plan.
    let fresh = workspace
        .time_model()
        .expect("open")
        .grid_preview(&proposal)
        .expect("preview");
    let removed = workspace
        .time_model()
        .expect("open")
        .release_orphans(&proposal, &fresh.fingerprint)
        .expect("release");
    assert_eq!(removed.len(), 1);

    // Step 5: now the day can go, and the rest of the grid is untouched.
    workspace
        .time_model()
        .expect("open")
        .delete_cycle_day(days[2])
        .expect("with its slot gone, the day can go");
    assert_eq!(
        workspace
            .time_model()
            .expect("open")
            .timeslots(cycle, None)
            .expect("read")
            .len(),
        2
    );
    assert_eq!(
        workspace
            .time_model()
            .expect("open")
            .cycle_days(cycle)
            .expect("read")
            .len(),
        2
    );
}
