//! A room that serves more than one campus.
//!
//! The behaviour under test is one physical room with **one identity**, reachable from
//! every campus it serves. The happy path is the small part. The risk is in the ways it
//! could quietly become two rooms, appear on a campus whose building does not contain it,
//! or let a building share be withdrawn from under it.
//!
//! Real temporary project files throughout, never `:memory:` — the association table's
//! foreign keys, the replaced room triggers and the change-log triggers are all
//! properties of a real database file.
//!
//! See [ADR 0014](../../../docs/adr/0014-rooms-shared-across-campuses.md) for why
//! `room.campus_id` survived V6 as the home campus, and for the scheduling-identity
//! invariant the last section of this file pins down as far as it can be pinned down
//! before there is a solver.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use uuid::Uuid;

use evara_core::calendar::WorkingDays;

use crate::repo::structure::{BuildingInput, CampusInput, RoomInput, SchoolInput};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// An open project with a school and three campuses.
struct Project {
    _dir: TempDir,
    workspace: Workspace,
    path: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let mut workspace =
            Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
                .expect("workspace");
        let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");
        Self {
            _dir: dir,
            workspace,
            path,
        }
    }

    fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    /// A school with three campuses: north, south and east.
    fn with_campuses(&mut self) -> [Uuid; 3] {
        let school = self
            .db()
            .structure()
            .create_school(&SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("create school");

        ["North", "South", "East"].map(|name| {
            self.db()
                .structure()
                .create_campus(&CampusInput {
                    school_id: school.id,
                    name: name.to_owned(),
                    code: name.to_uppercase(),
                    address: None,
                })
                .expect("create campus")
                .id
        })
    }

    fn reopen(&mut self) {
        self.workspace.close_project();
        self.workspace
            .open_project(&self.path.clone())
            .expect("reopen");
    }
}

fn building(campus_id: Uuid, name: &str) -> BuildingInput {
    BuildingInput {
        campus_id,
        name: name.to_owned(),
    }
}

fn room(campus_id: Uuid, code: &str, building_id: Option<Uuid>) -> RoomInput {
    RoomInput {
        campus_id,
        building_id,
        room_type_id: None,
        name: format!("Room {code}"),
        code: code.to_owned(),
        capacity: 30,
        is_bookable: true,
        notes: None,
    }
}

/// Identifiers of the rooms a campus can see.
fn visible(project: &mut Project, campus: Uuid) -> BTreeSet<Uuid> {
    project
        .db()
        .structure()
        .rooms(campus)
        .expect("list rooms")
        .into_iter()
        .map(|entry| entry.room.id)
        .collect()
}

// ====================================================================== creating

#[test]
fn a_room_created_for_one_campus_is_visible_only_there() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let local = project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[])
        .expect("create room");

    assert_eq!(visible(&mut project, north), BTreeSet::from([local.id]));
    assert!(visible(&mut project, south).is_empty());
}

#[test]
fn a_new_room_reports_the_campus_it_was_created_on() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let local = project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[])
        .expect("create room");

    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(local.id)
            .expect("campuses"),
        vec![north],
        "the home campus is an association like any other, not an implication"
    );
}

#[test]
fn an_unshared_room_reports_exactly_one_campus() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[])
        .expect("create room");

    let listing = project.db().structure().rooms(north).expect("list");
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].campus_ids, vec![north]);
}

#[test]
fn a_room_can_be_created_already_shared() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    for campus in [north, south, east] {
        assert!(
            visible(&mut project, campus).contains(&shared.id),
            "every campus named at creation can see the room"
        );
    }
}

#[test]
fn the_home_campus_appearing_in_the_share_list_is_not_an_error() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        // A multi-select can report the campus the user is already on, twice over.
        .create_room(&room(north, "HALL", None), &[north, south, south])
        .expect("the duplicates are ignored, not rejected");

    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(shared.id)
            .expect("campuses")
            .len(),
        2,
        "north once, south once"
    );
}

#[test]
fn creating_a_room_shared_with_a_campus_that_does_not_exist_writes_nothing() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();
    let ghost = Uuid::now_v7();

    let outcome = project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[ghost]);

    assert!(matches!(
        outcome,
        Err(DbError::NotFound { entity: "campus" })
    ));
    assert!(
        visible(&mut project, north).is_empty(),
        "the room must not survive a share it could not complete"
    );
}

// ====================================================================== sharing

#[test]
fn sharing_creates_no_second_room_record() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[])
        .expect("create room");

    project
        .db()
        .structure()
        .share_room(shared.id, south)
        .expect("share");

    let north_listing = project.db().structure().rooms(north).expect("list");
    let south_listing = project.db().structure().rooms(south).expect("list");

    assert_eq!(north_listing.len(), 1);
    assert_eq!(south_listing.len(), 1);
    assert_eq!(
        north_listing[0].room.id, south_listing[0].room.id,
        "one physical room is one row with one identifier"
    );
    assert_eq!(north_listing[0].room, south_listing[0].room);
}

#[test]
fn the_listing_names_every_campus_a_room_serves() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    let listing = project.db().structure().rooms(south).expect("list");
    assert_eq!(listing.len(), 1);
    assert_eq!(
        listing[0]
            .campus_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([north, south, east]),
        "the set is complete, so a screen never asks a follow-up question per row"
    );
}

#[test]
fn the_same_campus_cannot_be_associated_twice() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let repeat = project.db().structure().share_room(shared.id, south);
    assert!(
        matches!(repeat, Err(DbError::Invalid { .. })),
        "a repeat is reported rather than ignored, so a stale screen finds out"
    );

    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(shared.id)
            .expect("campuses")
            .len(),
        2
    );
}

#[test]
fn the_home_campus_cannot_be_shared_to_itself() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let local = project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[])
        .expect("create room");

    assert!(matches!(
        project.db().structure().share_room(local.id, north),
        Err(DbError::Invalid { .. })
    ));
}

#[test]
fn a_duplicate_association_is_impossible_even_behind_the_repository() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    // The repository refuses a repeat with a message; `UNIQUE (room_id, campus_id)` is
    // what makes that a guarantee rather than a convention. Both columns are NOT NULL so
    // the index actually holds — no COALESCE needed.
    let outcome = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO room_campus (id, room_id, campus_id, created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-10-11T00:00:00.000Z', '2026-10-11T00:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                shared.id.to_string(),
                south.to_string(),
            ),
        )?;
        Ok(())
    });

    assert!(outcome.is_err(), "the index refuses the second row");
}

#[test]
fn a_code_already_used_on_the_target_campus_blocks_the_share() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let travelling = project
        .db()
        .structure()
        .create_room(&room(north, "H1", None), &[])
        .expect("create north room");
    project
        .db()
        .structure()
        .create_room(&room(south, "H1", None), &[])
        .expect("create south room with the same code");

    let outcome = project.db().structure().share_room(travelling.id, south);
    assert!(
        matches!(
            outcome,
            Err(DbError::DuplicateValue {
                entity: "room",
                field: "code"
            })
        ),
        "UNIQUE (campus_id, code) scopes the home campus only, so this rule lives in Rust"
    );
}

#[test]
fn a_code_collision_on_any_campus_it_serves_blocks_a_rename() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");
    project
        .db()
        .structure()
        .create_room(&room(south, "GYM", None), &[])
        .expect("create a south room");

    let clash = project
        .db()
        .structure()
        .update_room(shared.id, &room(north, "GYM", None));

    assert!(
        matches!(
            clash,
            Err(DbError::DuplicateValue {
                entity: "room",
                field: "code"
            })
        ),
        "the new code has to be free on every campus the room serves, not just its home"
    );
}

#[test]
fn renaming_a_shared_room_renames_it_everywhere() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let mut renamed = room(north, "HALL", None);
    renamed.name = "Great Hall".to_owned();
    project
        .db()
        .structure()
        .update_room(shared.id, &renamed)
        .expect("rename");

    let south_listing = project.db().structure().rooms(south).expect("list");
    assert_eq!(south_listing[0].room.name, "Great Hall");
    assert_eq!(
        south_listing[0].room.id, shared.id,
        "the same row, so one edit is enough"
    );
}

// ================================================= withdrawing a share

#[test]
fn the_home_campus_cannot_be_withdrawn() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let outcome = project.db().structure().unshare_room(shared.id, north);
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "a room must never reach zero campuses and vanish from every screen"
    );
    assert!(visible(&mut project, north).contains(&shared.id));
}

#[test]
fn withdrawing_a_share_that_does_not_exist_is_reported_not_ignored() {
    let mut project = Project::new();
    let [north, _south, east] = project.with_campuses();

    let local = project
        .db()
        .structure()
        .create_room(&room(north, "N1", None), &[])
        .expect("create room");

    assert!(matches!(
        project.db().structure().unshare_room(local.id, east),
        Err(DbError::NotFound {
            entity: "room share"
        })
    ));
}

#[test]
fn withdrawing_a_share_takes_the_room_from_that_campus_only() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    project
        .db()
        .structure()
        .unshare_room(shared.id, south)
        .expect("withdraw");

    assert!(!visible(&mut project, south).contains(&shared.id));
    assert!(visible(&mut project, north).contains(&shared.id));
    assert!(visible(&mut project, east).contains(&shared.id));

    let still_there = project
        .db()
        .structure()
        .room(shared.id)
        .expect("read")
        .expect("the room itself is untouched");
    assert_eq!(still_there.id, shared.id);
    assert_eq!(still_there.campus_id, north);
}

// ============================================== buildings and rooms

#[test]
fn a_room_with_no_building_shares_freely() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    // No building anywhere in this school. Sharing must not require one.
    let hall = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create and share a room with no building");

    project
        .db()
        .structure()
        .share_room(hall.id, east)
        .expect("share again");

    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(hall.id)
            .expect("campuses")
            .len(),
        3
    );
    assert_eq!(
        project
            .db()
            .structure()
            .room(hall.id)
            .expect("read")
            .expect("room")
            .building_id,
        None,
        "and it still has no building"
    );
}

#[test]
fn a_shared_building_does_not_share_the_rooms_inside_it() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");

    let local_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[])
        .expect("create room in the shared building");

    assert!(
        !visible(&mut project, south).contains(&local_room.id),
        "sharing the building is not sharing its rooms — the two settings are independent"
    );
    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(local_room.id)
            .expect("campuses"),
        vec![north]
    );
}

#[test]
fn a_room_in_a_shared_building_can_be_shared_to_a_campus_that_building_serves() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");

    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[south])
        .expect("the building reaches south, so the room may too");

    assert!(visible(&mut project, south).contains(&shared_room.id));
}

#[test]
fn a_room_cannot_be_shared_to_a_campus_its_building_does_not_reach() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    // A building local to north only.
    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");
    let lab = project
        .db()
        .structure()
        .create_room(&room(north, "LAB1", Some(block.id)), &[])
        .expect("create room");

    let outcome = project.db().structure().share_room(lab.id, south);
    let Err(DbError::Invalid { message }) = outcome else {
        panic!("sharing into a campus the building does not reach must be refused");
    };
    assert!(
        message.contains("Science Block") && message.contains("South"),
        "the message has to name the building and the campus to be actionable: {message}"
    );

    assert!(!visible(&mut project, south).contains(&lab.id));
}

#[test]
fn creating_a_room_shared_past_its_buildings_reach_writes_nothing() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    let outcome = project
        .db()
        .structure()
        .create_room(&room(north, "LAB1", Some(block.id)), &[south]);

    assert!(matches!(outcome, Err(DbError::Invalid { .. })));
    assert!(
        visible(&mut project, north).is_empty(),
        "the room must not survive a share it could not complete"
    );
}

#[test]
fn a_shared_room_cannot_be_moved_into_a_building_that_does_not_reach_its_campuses() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let local_block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create local building");

    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room with no building");

    let outcome = project
        .db()
        .structure()
        .update_room(shared_room.id, &room(north, "HALL", Some(local_block.id)));

    let Err(DbError::Invalid { message }) = outcome else {
        panic!("moving a shared room into a local building must be refused");
    };
    assert!(
        message.contains("Science Block") && message.contains("South"),
        "the message names the building and the campus that cannot reach it: {message}"
    );

    assert_eq!(
        project
            .db()
            .structure()
            .room(shared_room.id)
            .expect("read")
            .expect("room")
            .building_id,
        None,
        "and the room is left exactly as it was"
    );
}

#[test]
fn clearing_a_shared_rooms_building_is_always_allowed() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");
    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[south])
        .expect("create shared room");

    project
        .db()
        .structure()
        .update_room(shared_room.id, &room(north, "SC1", None))
        .expect("a room with no building is a normal state, on any number of campuses");

    assert!(visible(&mut project, south).contains(&shared_room.id));
}

#[test]
fn the_trigger_refuses_an_incompatible_association_even_behind_the_repository() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");
    let lab = project
        .db()
        .structure()
        .create_room(&room(north, "LAB1", Some(block.id)), &[])
        .expect("create room");

    // The repository checks this first and raises a message naming the building and the
    // campus. The trigger is what makes the rule a property of the schema rather than a
    // convention of one code path — ADR 0010's treatment of SET NULL, applied here.
    let outcome = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO room_campus (id, room_id, campus_id, created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-10-11T00:00:00.000Z', '2026-10-11T00:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                lab.id.to_string(),
                south.to_string(),
            ),
        )?;
        Ok(())
    });

    assert!(outcome.is_err(), "the trigger refuses it");
}

// ========================= withdrawing a building share, from the room side

#[test]
fn a_shared_room_blocks_withdrawing_the_building_share_it_depends_on() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");

    // Homed on north, shared to south. Nothing in `room.campus_id` says south.
    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[south])
        .expect("create shared room");

    let outcome = project
        .db()
        .structure()
        .unshare_building(shared_building.id, south);

    let Err(DbError::Invalid { message }) = outcome else {
        panic!("a room shared to south still depends on the building reaching south");
    };
    assert!(
        message.contains("South"),
        "the message names the campus: {message}"
    );

    assert!(
        visible(&mut project, south).contains(&shared_room.id),
        "and nothing was moved, duplicated or deleted"
    );
}

#[test]
fn withdrawing_a_building_share_is_allowed_once_the_room_share_is_withdrawn() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");
    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[south])
        .expect("create shared room");

    project
        .db()
        .structure()
        .unshare_room(shared_room.id, south)
        .expect("withdraw the room first");
    project
        .db()
        .structure()
        .unshare_building(shared_building.id, south)
        .expect("now the building has nothing depending on it");

    assert!(visible(&mut project, north).contains(&shared_room.id));
    assert!(!visible(&mut project, south).contains(&shared_room.id));
}

#[test]
fn deleting_a_building_detaches_the_rooms_shared_through_it_rather_than_deleting_them() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared_building = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared building");
    let shared_room = project
        .db()
        .structure()
        .create_room(&room(north, "SC1", Some(shared_building.id)), &[south])
        .expect("create shared room");

    project
        .db()
        .structure()
        .delete_building(shared_building.id)
        .expect("delete the building");

    let survivor = project
        .db()
        .structure()
        .room(shared_room.id)
        .expect("read")
        .expect("the room survives — its building was metadata");
    assert_eq!(survivor.building_id, None);
    assert_eq!(
        survivor.rev, 2,
        "detached by a real update, so the revision advanced"
    );

    assert!(
        visible(&mut project, south).contains(&shared_room.id),
        "and it is still shared with south — losing a building is not losing a campus"
    );
}

// ====================================================== deleting a room

#[test]
fn deleting_a_shared_room_removes_it_from_every_campus() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    project
        .db()
        .structure()
        .delete_room(shared.id)
        .expect("delete");

    for campus in [north, south, east] {
        assert!(visible(&mut project, campus).is_empty());
    }
    assert!(
        project
            .db()
            .structure()
            .room(shared.id)
            .expect("read")
            .is_none(),
        "one room, one delete"
    );
}

#[test]
fn a_campus_sharing_a_room_cannot_be_deleted_until_the_share_is_withdrawn() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    assert!(
        project.db().structure().delete_campus(south).is_err(),
        "RESTRICT on room_campus.campus_id, so 'delete a campus' means the same on every edge"
    );

    project
        .db()
        .structure()
        .unshare_room(shared.id, south)
        .expect("withdraw");
    project
        .db()
        .structure()
        .delete_campus(south)
        .expect("now south has nothing of its own");
}

// ============================================== persistence and migration

#[test]
fn shares_survive_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    project.reopen();

    assert_eq!(
        project
            .db()
            .structure()
            .room_campuses(shared.id)
            .expect("campuses")
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([north, south, east])
    );
    assert!(visible(&mut project, south).contains(&shared.id));
}

/// The fixture's audit stamp. Fixed so the assertions can prove nothing was rewritten.
const FIXTURE_STAMP: &str = "2026-10-07T09:30:00.000Z";

struct FixtureIds {
    north: Uuid,
    south: Uuid,
    block: Uuid,
    classified: Uuid,
    bare: Uuid,
}

/// Writes a project as V5 left it: two campuses, one building, two rooms.
///
/// Raw SQL at the V5 schema, deliberately — the repository of *this* build writes
/// `room_campus` rows, which is exactly what a project from the previous release does not
/// have. Going through it would test nothing.
fn write_v5_fixture(path: &Path, site: Uuid) -> FixtureIds {
    let ids = FixtureIds {
        north: Uuid::now_v7(),
        south: Uuid::now_v7(),
        block: Uuid::now_v7(),
        classified: Uuid::now_v7(),
        bare: Uuid::now_v7(),
    };
    let school = Uuid::now_v7();

    let mut conn = crate::connection::open_configured(path, site).expect("open");
    crate::migrations::run_up_to(&mut conn, 5).expect("migrate to V5");

    conn.execute(
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', ?, ?, 1)",
        (school.to_string(), FIXTURE_STAMP, FIXTURE_STAMP),
    )
    .expect("school");

    for (id, name, code) in [(ids.north, "North", "NORTH"), (ids.south, "South", "SOUTH")] {
        conn.execute(
            "INSERT INTO campus (id, school_id, name, code, address, created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, NULL, ?, ?, 1)",
            (
                id.to_string(),
                school.to_string(),
                name,
                code,
                FIXTURE_STAMP,
                FIXTURE_STAMP,
            ),
        )
        .expect("campus");
    }

    conn.execute(
        "INSERT INTO building (id, campus_id, name, created_at, updated_at, rev)
         VALUES (?, ?, 'Science Block', ?, ?, 1)",
        (
            ids.block.to_string(),
            ids.north.to_string(),
            FIXTURE_STAMP,
            FIXTURE_STAMP,
        ),
    )
    .expect("building");
    // V5's own backfill shape: the home campus is an association row.
    conn.execute(
        "INSERT INTO building_campus (id, building_id, campus_id, created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, 1)",
        (
            Uuid::now_v7().to_string(),
            ids.block.to_string(),
            ids.north.to_string(),
            FIXTURE_STAMP,
            FIXTURE_STAMP,
        ),
    )
    .expect("building_campus");

    // One room in the building, one with none — both paths through V6's triggers.
    conn.execute(
        "INSERT INTO room (id, campus_id, building_id, room_type_id, name, code, capacity,
                           is_bookable, notes, created_at, updated_at, rev)
         VALUES (?, ?, ?, NULL, 'Lab 1', 'L1', 24, 1, NULL, ?, ?, 1)",
        (
            ids.classified.to_string(),
            ids.north.to_string(),
            ids.block.to_string(),
            FIXTURE_STAMP,
            FIXTURE_STAMP,
        ),
    )
    .expect("classified room");
    conn.execute(
        "INSERT INTO room (id, campus_id, building_id, room_type_id, name, code, capacity,
                           is_bookable, notes, created_at, updated_at, rev)
         VALUES (?, ?, NULL, NULL, 'Hall', 'HALL', 200, 0, NULL, ?, ?, 1)",
        (
            ids.bare.to_string(),
            ids.south.to_string(),
            FIXTURE_STAMP,
            FIXTURE_STAMP,
        ),
    )
    .expect("bare room");

    ids
}

#[test]
fn migrating_a_v5_project_preserves_every_identifier_and_relationship() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000003").expect("valid uuid");

    let ids = write_v5_fixture(&path, site);

    // Open it with this build, which migrates it forward.
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("migrate and open");
    assert_eq!(
        db.schema_version(),
        crate::migrations::supported_schema_version(),
        "the project migrated all the way forward"
    );

    // Identity is untouched.
    let lab = db
        .structure()
        .room(ids.classified)
        .expect("read")
        .expect("the room kept its identifier");
    assert_eq!(lab.id, ids.classified);
    assert_eq!(lab.name, "Lab 1");
    assert_eq!(lab.building_id, Some(ids.block), "and the same building");
    assert_eq!(lab.rev, 1, "migrating must not consume a revision");
    assert_eq!(
        lab.updated_at, FIXTURE_STAMP,
        "nor rewrite the audit columns"
    );

    let hall = db
        .structure()
        .room(ids.bare)
        .expect("read")
        .expect("the room with no building kept its identifier too");
    assert_eq!(hall.building_id, None);
    assert_eq!(hall.capacity, 200);
    assert!(!hall.is_bookable, "and the same bookability");
    assert_eq!(hall.rev, 1);

    // Every pre-existing relationship became an association.
    assert_eq!(
        db.structure()
            .room_campuses(ids.classified)
            .expect("campuses"),
        vec![ids.north]
    );
    assert_eq!(
        db.structure().room_campuses(ids.bare).expect("campuses"),
        vec![ids.south]
    );

    let north_listing = db.structure().rooms(ids.north).expect("list");
    assert_eq!(north_listing.len(), 1);
    assert_eq!(north_listing[0].room.id, ids.classified);
    assert_eq!(north_listing[0].campus_ids, vec![ids.north]);

    let south_listing = db.structure().rooms(ids.south).expect("list");
    assert_eq!(south_listing.len(), 1);
    assert_eq!(south_listing[0].room.id, ids.bare);
}

#[test]
fn the_backfilled_room_associations_carry_real_uuid_v7_identifiers() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000004").expect("valid uuid");

    write_v5_fixture(&path, site);
    let db = Database::open_existing(&path, APP_VERSION, site).expect("migrate and open");

    // The backfill lays a v7 identifier out field by field in SQL, because refinery
    // migrations are .sql files and SQLite has no uuid(). Assert the result rather than
    // trusting the printf.
    let ids: Vec<String> = db
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT id FROM room_campus")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .expect("read associations");

    assert_eq!(ids.len(), 2, "one per room in the fixture");
    for raw in ids {
        let parsed = Uuid::parse_str(&raw).expect("a parseable identifier");
        assert_eq!(
            parsed.get_version_num(),
            7,
            "{raw} must be a UUIDv7 like every other identifier in the project"
        );
    }
}

#[test]
fn a_migrated_project_can_then_share_its_rooms() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000005").expect("valid uuid");

    let ids = write_v5_fixture(&path, site);
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("migrate and open");

    // The hall has no building, so nothing stands in the way of sharing it north.
    db.structure()
        .share_room(ids.bare, ids.north)
        .expect("share a migrated room");

    assert_eq!(
        db.structure()
            .room_campuses(ids.bare)
            .expect("campuses")
            .len(),
        2
    );

    // The lab is in a north-only building, so south is refused — the rule applies to
    // migrated rows exactly as it does to new ones.
    assert!(matches!(
        db.structure().share_room(ids.classified, ids.south),
        Err(DbError::Invalid { .. })
    ));
}

// ============================================== change feed and revisions

#[test]
fn creating_a_shared_room_logs_the_room_and_every_association() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room" && r.op == "INSERT")
            .count(),
        1
    );
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room_campus" && r.op == "INSERT")
            .count(),
        3,
        "the home campus and both shares each get a row, so the feed is complete"
    );
}

#[test]
fn sharing_and_withdrawing_both_reach_the_change_feed() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[])
        .expect("create room");

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    project
        .db()
        .structure()
        .share_room(shared.id, south)
        .expect("share");
    project
        .db()
        .structure()
        .unshare_room(shared.id, south)
        .expect("withdraw");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room_campus" && r.op == "INSERT")
            .count(),
        1
    );
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room_campus" && r.op == "DELETE")
            .count(),
        1,
        "a withdrawal is a real edit and belongs in the feed like any other"
    );
}

#[test]
fn deleting_a_shared_room_logs_the_removal_of_each_association() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    project
        .db()
        .structure()
        .delete_room(shared.id)
        .expect("delete");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room_campus" && r.op == "DELETE")
            .count(),
        2,
        "removed through repo::delete, not by a CASCADE that skips the trigger"
    );
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "room" && r.op == "DELETE")
            .count(),
        1
    );
}

#[test]
fn a_refused_share_writes_nothing_to_the_feed() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    assert!(
        project
            .db()
            .structure()
            .share_room(shared.id, south)
            .is_err()
    );

    assert_eq!(
        project.db().changes_since(0, 1000).expect("feed").len(),
        before,
        "a refusal is not a change"
    );
}

#[test]
fn sharing_a_room_does_not_touch_the_rooms_own_revision() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[])
        .expect("create room");
    assert_eq!(shared.rev, 1);

    project
        .db()
        .structure()
        .share_room(shared.id, south)
        .expect("share");

    let after = project
        .db()
        .structure()
        .room(shared.id)
        .expect("read")
        .expect("room");
    assert_eq!(
        after.rev, 1,
        "the association changed, not the room — its revision must not drift"
    );
    assert_eq!(after.updated_at, shared.updated_at);
}

// ====================================================== scheduling identity
//
// Nothing here schedules anything: the solver, activities and `room-clash` are later
// phases, and ADR 0014 forbids a second implementation of any scheduling rule in the
// meantime. What these tests can do — and what makes the invariant testable at all today
// — is pin down the fact the future rule will depend on: a shared room is ONE row with
// ONE identifier, so occupancy can only ever be a property of `room.id`.

#[test]
fn a_shared_room_has_exactly_one_identity_across_every_campus() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south, east])
        .expect("create shared room");

    let seen: BTreeSet<Uuid> = [north, south, east]
        .into_iter()
        .flat_map(|campus| {
            project
                .db()
                .structure()
                .rooms(campus)
                .expect("list")
                .into_iter()
                .map(|entry| entry.room.id)
        })
        .collect();

    assert_eq!(
        seen,
        BTreeSet::from([shared.id]),
        "three campuses, one room identifier — occupancy cannot be per-campus because \
         there is no per-campus room to hang it on"
    );
}

#[test]
fn every_campus_reads_the_identical_room_record() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[south])
        .expect("create shared room");

    let from_north = project.db().structure().rooms(north).expect("list");
    let from_south = project.db().structure().rooms(south).expect("list");

    assert_eq!(
        from_north[0].room, from_south[0].room,
        "same capacity, same bookability, same revision — one record, not a copy per campus"
    );
    assert_eq!(from_north[0].campus_ids, from_south[0].campus_ids);
    assert_eq!(from_north[0].room.id, shared.id);
}

#[test]
fn the_number_of_distinct_rooms_does_not_grow_when_a_room_is_shared() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_room(&room(north, "HALL", None), &[])
        .expect("create room");

    let count_rows = |project: &mut Project| -> i64 {
        project
            .db()
            .read(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM room", [], |row| row.get::<_, i64>(0))?)
            })
            .expect("count rooms")
    };

    assert_eq!(count_rows(&mut project), 1);

    project
        .db()
        .structure()
        .share_room(shared.id, south)
        .expect("share");

    assert_eq!(
        count_rows(&mut project),
        1,
        "sharing adds an association, never a room — the invariant the solver will rely on"
    );
}

#[test]
fn a_bookable_shared_room_is_bookable_from_every_campus() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let mut unbookable = room(north, "STORE", None);
    unbookable.is_bookable = false;

    let store = project
        .db()
        .structure()
        .create_room(&unbookable, &[south])
        .expect("create shared room");

    // `is_bookable` is a property of the room, so it cannot differ by campus. A campus
    // that wanted it bookable and another that did not would need two rooms, which is the
    // thing V6 exists to avoid.
    for campus in [north, south] {
        let listing = project.db().structure().rooms(campus).expect("list");
        assert_eq!(listing[0].room.id, store.id);
        assert!(!listing[0].room.is_bookable);
    }
}
