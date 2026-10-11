//! A building that serves more than one campus.
//!
//! The behaviour under test is one physical building with **one identity**, reachable from
//! every campus it serves. Most of the risk in that is not in the happy path — it is in the
//! four ways it could quietly become two buildings, lose a room, or let a share be
//! withdrawn out from under rooms that depend on it.
//!
//! Real temporary project files throughout, never `:memory:` — the association table's
//! foreign keys, the replaced room trigger and the change-log triggers are all properties
//! of a real database file.
//!
//! See [ADR 0013](../../../docs/adr/0013-buildings-shared-across-campuses.md) for why
//! `building.campus_id` survived V5 as the home campus rather than being dropped.

use std::collections::BTreeSet;
use std::path::PathBuf;

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

/// Identifiers of the buildings a campus can see.
fn visible(project: &mut Project, campus: Uuid) -> BTreeSet<Uuid> {
    project
        .db()
        .structure()
        .buildings(campus)
        .expect("list buildings")
        .into_iter()
        .map(|entry| entry.building.id)
        .collect()
}

// ====================================================================== creating

#[test]
fn a_building_created_for_one_campus_is_visible_only_there() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    assert_eq!(visible(&mut project, north), BTreeSet::from([block.id]));
    assert!(
        visible(&mut project, south).is_empty(),
        "an unshared building belongs to its own campus only"
    );
}

#[test]
fn a_new_building_reports_the_campus_it_was_created_on() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    // The home campus is an association like any other, so a building that was never
    // shared still has exactly one row to find.
    assert_eq!(
        project
            .db()
            .structure()
            .building_campuses(block.id)
            .expect("campuses"),
        vec![north]
    );
}

#[test]
fn a_building_can_be_created_already_shared() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south, east])
        .expect("create shared building");

    for campus in [north, south, east] {
        assert!(
            visible(&mut project, campus).contains(&block.id),
            "the building should be visible from every campus it was shared with"
        );
    }
}

#[test]
fn the_home_campus_appearing_in_the_share_list_is_not_an_error() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    // A multi-select can report the campus the user is already on. Ignoring it is kinder
    // than a round trip that says "you picked the campus you are on".
    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Hall"), &[north, south, south])
        .expect("create building");

    assert_eq!(
        project
            .db()
            .structure()
            .building_campuses(block.id)
            .expect("campuses")
            .len(),
        2,
        "the home campus and the duplicate must each be counted once"
    );
}

#[test]
fn creating_a_building_shared_with_a_campus_that_does_not_exist_writes_nothing() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let outcome = project
        .db()
        .structure()
        .create_building(&building(north, "Ghost Block"), &[Uuid::now_v7()]);

    assert!(
        matches!(outcome, Err(DbError::NotFound { entity: "campus" })),
        "expected NotFound for the campus, got {outcome:?}"
    );
    assert!(
        visible(&mut project, north).is_empty(),
        "a refused create must leave no half-made building behind"
    );
}

// ======================================================================= sharing

#[test]
fn a_building_can_be_shared_after_the_fact() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    project
        .db()
        .structure()
        .share_building(block.id, south)
        .expect("share");

    assert!(visible(&mut project, south).contains(&block.id));
}

#[test]
fn sharing_creates_no_second_building_record() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[south, east])
        .expect("create building");

    // The whole point: three campuses, one row, one identity.
    let rows: i64 = project
        .db()
        .read(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM building", [], |row| row.get(0))?))
        .expect("count buildings");
    assert_eq!(rows, 1, "sharing must not duplicate the building");

    for campus in [north, south, east] {
        let listed = project
            .db()
            .structure()
            .buildings(campus)
            .expect("list")
            .into_iter()
            .map(|entry| entry.building)
            .collect::<Vec<_>>();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0], block,
            "every campus must see the identical record, revision included"
        );
    }
}

#[test]
fn the_listing_names_every_campus_a_building_serves() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south, east])
        .expect("create building");

    let listed = project.db().structure().buildings(east).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0]
            .campus_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([north, south, east]),
        "the listing must carry the whole share set, not just the campus asked about"
    );
}

#[test]
fn an_unshared_building_reports_exactly_one_campus() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    project
        .db()
        .structure()
        .create_building(&building(north, "Hall"), &[])
        .expect("create building");

    let listed = project.db().structure().buildings(north).expect("list");
    assert_eq!(
        listed[0].campus_ids,
        vec![north],
        "so the interface can tell shared from unshared by counting"
    );
}

#[test]
fn the_same_campus_cannot_be_associated_twice() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[south])
        .expect("create building");

    let outcome = project.db().structure().share_building(block.id, south);
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "a repeated share must be refused, not silently accepted: {outcome:?}"
    );

    assert_eq!(
        project
            .db()
            .structure()
            .building_campuses(block.id)
            .expect("campuses")
            .len(),
        2,
        "and must not have added a row"
    );
}

#[test]
fn the_home_campus_cannot_be_shared_to_itself() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Hall"), &[])
        .expect("create building");

    let outcome = project.db().structure().share_building(block.id, north);
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "the home campus already has it: {outcome:?}"
    );
}

#[test]
fn a_duplicate_association_is_impossible_even_behind_the_repository() {
    let mut project = Project::new();
    let [north, _south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Hall"), &[])
        .expect("create building");

    // The repository refuses first, but the rule is in the schema too, so a future caller
    // that forgets the check still cannot write a duplicate.
    let outcome = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO building_campus (id, building_id, campus_id, created_at, updated_at, rev)
             VALUES (?, ?, ?, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
            (
                Uuid::now_v7().to_string(),
                block.id.to_string(),
                north.to_string(),
            ),
        )?;
        Ok(())
    });

    assert!(
        outcome.is_err(),
        "UNIQUE (building_id, campus_id) must hold at the schema level"
    );
}

#[test]
fn a_name_already_used_on_the_target_campus_blocks_the_share() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create north block");
    project
        .db()
        .structure()
        .create_building(&building(south, "Science Block"), &[])
        .expect("create south block");

    // UNIQUE (campus_id, name) cannot see this: the two buildings have different home
    // campuses. Without the repository's check, South would end up with two buildings
    // called "Science Block" in one listing.
    let outcome = project.db().structure().share_building(block.id, south);
    assert!(
        matches!(
            outcome,
            Err(DbError::DuplicateValue {
                entity: "building",
                field: "name"
            })
        ),
        "expected a duplicate-name refusal, got {outcome:?}"
    );
}

#[test]
fn renaming_is_blocked_by_a_collision_on_any_campus_it_serves() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    project
        .db()
        .structure()
        .create_building(&building(south, "Library"), &[])
        .expect("create south library");

    let outcome = project
        .db()
        .structure()
        .update_building(shared.id, &building(north, "Library"));

    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { .. })),
        "a rename must not create two identically named buildings on South: {outcome:?}"
    );
}

#[test]
fn renaming_a_shared_building_renames_it_everywhere() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");

    project
        .db()
        .structure()
        .update_building(shared.id, &building(north, "Sports Hall"))
        .expect("rename");

    for campus in [north, south] {
        let listed = project.db().structure().buildings(campus).expect("list");
        assert_eq!(
            listed[0].building.name, "Sports Hall",
            "one physical building has one name"
        );
    }
}

// ========================================================================= rooms

#[test]
fn a_shared_building_holds_rooms_from_several_campuses() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");

    let north_room = project
        .db()
        .structure()
        .create_room(&room(north, "N1", Some(shared.id)), &[])
        .expect("a room on the home campus");
    let south_room = project
        .db()
        .structure()
        .create_room(&room(south, "S1", Some(shared.id)), &[])
        .expect("a room on the shared campus");

    assert_eq!(north_room.building_id, Some(shared.id));
    assert_eq!(south_room.building_id, Some(shared.id));
    assert_ne!(north_room.campus_id, south_room.campus_id);
}

#[test]
fn a_room_cannot_name_a_building_its_campus_does_not_have() {
    let mut project = Project::new();
    let [north, _south, east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    // The V5 trigger reads building_campus rather than building.campus_id, but the rule it
    // enforces is the one V3 enforced.
    let outcome = project
        .db()
        .structure()
        .create_room(&room(east, "E1", Some(block.id)), &[]);

    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "expected the room/campus rule to refuse, got {outcome:?}"
    );
}

#[test]
fn withdrawing_a_share_is_blocked_by_the_rooms_that_depend_on_it() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    let south_room = project
        .db()
        .structure()
        .create_room(&room(south, "S1", Some(shared.id)), &[])
        .expect("create room");

    let outcome = project.db().structure().unshare_building(shared.id, south);

    match outcome {
        Err(DbError::Invalid { message }) => {
            assert!(
                message.contains("South") && message.contains("Sports Centre"),
                "the message must name the campus and the building: {message}"
            );
            assert!(
                message.contains("Move them") || message.contains("clear their building"),
                "and must say what to do about it: {message}"
            );
        }
        other => panic!("expected a blocked unshare, got {other:?}"),
    }

    // Nothing was detached and nothing was deleted on the way to refusing.
    let reloaded = project
        .db()
        .structure()
        .room(south_room.id)
        .expect("read")
        .expect("the room still exists");
    assert_eq!(
        reloaded, south_room,
        "a refused unshare must not touch the room at all, revision included"
    );
    assert!(visible(&mut project, south).contains(&shared.id));
}

#[test]
fn withdrawing_a_share_succeeds_once_the_rooms_are_reassigned() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    let south_room = project
        .db()
        .structure()
        .create_room(&room(south, "S1", Some(shared.id)), &[])
        .expect("create room");

    project
        .db()
        .structure()
        .update_room(south_room.id, &room(south, "S1", None))
        .expect("clear the room's building");

    project
        .db()
        .structure()
        .unshare_building(shared.id, south)
        .expect("now it may be withdrawn");

    assert!(!visible(&mut project, south).contains(&shared.id));
    assert!(
        visible(&mut project, north).contains(&shared.id),
        "and it is untouched on its home campus"
    );
}

#[test]
fn rooms_on_other_campuses_do_not_block_a_withdrawal() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    project
        .db()
        .structure()
        .create_room(&room(north, "N1", Some(shared.id)), &[])
        .expect("a room on the home campus");

    // Only South's own rooms are South's business.
    project
        .db()
        .structure()
        .unshare_building(shared.id, south)
        .expect("North's rooms are irrelevant to withdrawing South");
}

// ==================================================================== withdrawal

#[test]
fn the_home_campus_cannot_be_withdrawn() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");

    let outcome = project.db().structure().unshare_building(shared.id, north);

    match outcome {
        Err(DbError::Invalid { message }) => assert!(
            message.contains("North"),
            "the message must name the campus: {message}"
        ),
        other => panic!("a building may never reach zero campuses, got {other:?}"),
    }
}

#[test]
fn withdrawing_a_share_that_does_not_exist_is_reported_not_ignored() {
    let mut project = Project::new();
    let [north, _south, east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Hall"), &[])
        .expect("create building");

    let outcome = project.db().structure().unshare_building(block.id, east);
    assert!(
        matches!(
            outcome,
            Err(DbError::NotFound {
                entity: "building share"
            })
        ),
        "expected NotFound for the share, got {outcome:?}"
    );
}

// ====================================================================== deletion

#[test]
fn deleting_a_shared_building_removes_it_from_every_campus() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south, east])
        .expect("create shared");

    project
        .db()
        .structure()
        .delete_building(shared.id)
        .expect("delete");

    for campus in [north, south, east] {
        assert!(
            visible(&mut project, campus).is_empty(),
            "the building is gone, so no campus may still list it"
        );
    }

    let associations: i64 = project
        .db()
        .read(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM building_campus", [], |row| row.get(0))?)
        })
        .expect("count associations");
    assert_eq!(associations, 0, "its associations go with it");
}

#[test]
fn deleting_a_shared_building_detaches_rooms_on_every_campus_rather_than_deleting_them() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    let north_room = project
        .db()
        .structure()
        .create_room(&room(north, "N1", Some(shared.id)), &[])
        .expect("north room");
    let south_room = project
        .db()
        .structure()
        .create_room(&room(south, "S1", Some(shared.id)), &[])
        .expect("south room");

    project
        .db()
        .structure()
        .delete_building(shared.id)
        .expect("delete");

    for (id, campus) in [(north_room.id, north), (south_room.id, south)] {
        let reloaded = project
            .db()
            .structure()
            .room(id)
            .expect("read")
            .expect("a room is an asset and survives its building");
        assert_eq!(reloaded.building_id, None, "detached, not deleted");
        assert_eq!(reloaded.campus_id, campus, "and still on its own campus");
        assert_eq!(
            reloaded.rev, 2,
            "detaching is a real update, so rev advances"
        );
    }
}

#[test]
fn a_campus_sharing_a_building_cannot_be_deleted_until_the_share_is_withdrawn() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");

    // ADR 0010's rule, applied to the new edge: ownership and association both refuse.
    let outcome = project.db().structure().delete_campus(south);
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "expected the campus delete to be refused, got {outcome:?}"
    );

    project
        .db()
        .structure()
        .unshare_building(shared.id, south)
        .expect("withdraw");
    project
        .db()
        .structure()
        .delete_campus(south)
        .expect("now the campus may go");
}

// ===================================================================== reopening

#[test]
fn shares_survive_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");
    let plain = project
        .db()
        .structure()
        .create_building(&building(east, "Library"), &[])
        .expect("create plain");

    project.reopen();

    assert_eq!(visible(&mut project, north), BTreeSet::from([shared.id]));
    assert_eq!(visible(&mut project, south), BTreeSet::from([shared.id]));
    assert_eq!(visible(&mut project, east), BTreeSet::from([plain.id]));

    let listed = project.db().structure().buildings(south).expect("list");
    assert_eq!(
        listed[0]
            .campus_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([north, south]),
        "the share set is persisted, not reconstructed from the home campus"
    );
}

// ==================================================================== change log

#[test]
fn sharing_and_withdrawing_both_reach_the_change_feed() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[])
        .expect("create building");

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    project
        .db()
        .structure()
        .share_building(block.id, south)
        .expect("share");
    project
        .db()
        .structure()
        .unshare_building(block.id, south)
        .expect("withdraw");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    let ops: Vec<_> = new
        .iter()
        .filter(|record| record.entity_table == "building_campus")
        .map(|record| record.op.as_str())
        .collect();
    assert_eq!(
        ops,
        vec!["INSERT", "DELETE"],
        "an association is school data and both edits belong in the feed"
    );
}

#[test]
fn creating_a_shared_building_logs_the_building_and_every_association() {
    let mut project = Project::new();
    let [north, south, east] = project.with_campuses();

    let before = project.db().changes_since(0, 1000).expect("feed").len();

    project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south, east])
        .expect("create shared");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "building" && r.op == "INSERT")
            .count(),
        1
    );
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "building_campus" && r.op == "INSERT")
            .count(),
        3,
        "the home campus and both shares each get a row, so the feed is complete"
    );
}

#[test]
fn deleting_a_shared_building_logs_the_removal_of_each_association() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let shared = project
        .db()
        .structure()
        .create_building(&building(north, "Sports Centre"), &[south])
        .expect("create shared");

    let before = project.db().changes_since(0, 1000).expect("feed").len();
    project
        .db()
        .structure()
        .delete_building(shared.id)
        .expect("delete");

    let feed = project.db().changes_since(0, 1000).expect("feed");
    let new = &feed[before..];

    // The reason delete_building removes them itself rather than leaving it to a CASCADE:
    // SQLite would not fire the trigger, and the feed would still claim they exist.
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "building_campus" && r.op == "DELETE")
            .count(),
        2
    );
    assert_eq!(
        new.iter()
            .filter(|r| r.entity_table == "building" && r.op == "DELETE")
            .count(),
        1
    );
}

#[test]
fn a_refused_share_writes_nothing_to_the_feed() {
    let mut project = Project::new();
    let [north, south, _east] = project.with_campuses();

    let block = project
        .db()
        .structure()
        .create_building(&building(north, "Science Block"), &[south])
        .expect("create building");

    let before = project.db().changes_since(0, 1000).expect("feed").len();
    let _ = project.db().structure().share_building(block.id, south);

    assert_eq!(
        project.db().changes_since(0, 1000).expect("feed").len(),
        before,
        "a rejected write leaves no trace"
    );
}

// ===================================================================== migration

/// The migration path every existing project will actually take.
///
/// A V4 file with buildings and rooms, opened by this build. The requirement is not merely
/// that it opens: every identifier has to be the same afterwards, and every
/// building-to-campus relationship has to have survived as an association row.
/// The identifiers written into a V4 fixture, so the assertions can look for them again.
struct V4Fixture {
    north: Uuid,
    south: Uuid,
    block: Uuid,
    library: Uuid,
    room: Uuid,
}

/// The timestamp every fixture row is written with, so "unchanged" is checkable.
const FIXTURE_STAMP: &str = "2026-10-06T00:00:00.000Z";

/// Writes a project exactly as the previous schema version would have left it.
///
/// Raw SQL rather than the repositories, because the repositories are this build's and
/// would write this build's schema. The point of the fixture is to be a V4 file.
fn write_v4_fixture(path: &std::path::Path, site: Uuid) -> V4Fixture {
    let ids = V4Fixture {
        north: Uuid::now_v7(),
        south: Uuid::now_v7(),
        block: Uuid::now_v7(),
        library: Uuid::now_v7(),
        room: Uuid::now_v7(),
    };
    let school_id = Uuid::now_v7();
    let stamp = FIXTURE_STAMP;

    let mut conn = crate::connection::open_configured(path, site).expect("open");
    let version = crate::migrations::run_up_to(&mut conn, 4).expect("migrate to V4");
    assert_eq!(version, 4, "the fixture must stop at the previous version");

    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .expect("prepare");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query");
        rows.collect::<rusqlite::Result<Vec<_>>>().expect("collect")
    };
    assert!(
        !tables.contains(&"building_campus".to_owned()),
        "the fixture must not already have the new schema"
    );

    conn.execute(
        "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
         VALUES (?, 'Northgate Secondary', 'Europe/London', 'en-GB', ?, ?, 1)",
        (school_id.to_string(), stamp, stamp),
    )
    .expect("school");

    for (id, name, code) in [(ids.north, "North", "N"), (ids.south, "South", "S")] {
        conn.execute(
            "INSERT INTO campus (id, school_id, name, code, created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, ?, 1)",
            (
                id.to_string(),
                school_id.to_string(),
                name,
                code,
                stamp,
                stamp,
            ),
        )
        .expect("campus");
    }

    for (id, campus, name) in [
        (ids.block, ids.north, "Science Block"),
        (ids.library, ids.south, "Library"),
    ] {
        conn.execute(
            "INSERT INTO building (id, campus_id, name, created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, 1)",
            (id.to_string(), campus.to_string(), name, stamp, stamp),
        )
        .expect("building");
    }

    conn.execute(
        "INSERT INTO room (id, campus_id, building_id, room_type_id, name, code, capacity,
                           is_bookable, notes, created_at, updated_at, rev)
         VALUES (?, ?, ?, NULL, 'Lab 1', 'L1', 30, 1, NULL, ?, ?, 1)",
        (
            ids.room.to_string(),
            ids.north.to_string(),
            ids.block.to_string(),
            stamp,
            stamp,
        ),
    )
    .expect("room");

    ids
}

#[test]
fn migrating_a_v4_project_preserves_every_identifier_and_relationship() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000001").expect("valid uuid");

    let ids = write_v4_fixture(&path, site);
    let stamp = FIXTURE_STAMP;

    // Open it with this build, which migrates it forward.
    let mut db = Database::open_existing(&path, APP_VERSION, site).expect("migrate and open");
    // Against the supported version rather than a literal: a V4 project must migrate all
    // the way forward, and hardcoding "5" here made this test fail the moment V6 shipped
    // for a reason that had nothing to do with what it checks.
    assert_eq!(
        db.schema_version(),
        crate::migrations::supported_schema_version(),
        "the project migrated all the way forward"
    );

    // Identity is untouched.
    let block = db
        .structure()
        .building(ids.block)
        .expect("read")
        .expect("the building kept its identifier");
    assert_eq!(block.id, ids.block);
    assert_eq!(block.name, "Science Block");
    assert_eq!(block.rev, 1, "migrating must not consume a revision");
    assert_eq!(block.updated_at, stamp, "nor rewrite the audit columns");

    let migrated_room = db
        .structure()
        .room(ids.room)
        .expect("read")
        .expect("the room kept its identifier");
    assert_eq!(migrated_room.id, ids.room);
    assert_eq!(
        migrated_room.building_id,
        Some(ids.block),
        "and still names the same building"
    );
    assert_eq!(migrated_room.rev, 1);

    // Every pre-existing relationship became an association.
    assert_eq!(
        db.structure()
            .building_campuses(ids.block)
            .expect("campuses"),
        vec![ids.north]
    );
    assert_eq!(
        db.structure()
            .building_campuses(ids.library)
            .expect("campuses"),
        vec![ids.south]
    );

    let north_listing = db.structure().buildings(ids.north).expect("list");
    assert_eq!(north_listing.len(), 1);
    assert_eq!(north_listing[0].building.id, ids.block);
    assert_eq!(north_listing[0].campus_ids, vec![ids.north]);

    let south_listing = db.structure().buildings(ids.south).expect("list");
    assert_eq!(south_listing.len(), 1);
    assert_eq!(south_listing[0].building.id, ids.library);
}

#[test]
fn the_backfilled_associations_carry_real_uuid_v7_identifiers() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let site = Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-000000000002").expect("valid uuid");

    let school_id = Uuid::now_v7();
    let campus_id = Uuid::now_v7();
    let stamp = "2026-10-06T00:00:00.000Z";

    {
        let mut conn = crate::connection::open_configured(&path, site).expect("open");
        crate::migrations::run_up_to(&mut conn, 4).expect("migrate to V4");
        conn.execute(
            "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
             VALUES (?, 'S', 'UTC', 'en-GB', ?, ?, 1)",
            (school_id.to_string(), stamp, stamp),
        )
        .expect("school");
        conn.execute(
            "INSERT INTO campus (id, school_id, name, code, created_at, updated_at, rev)
             VALUES (?, ?, 'Main', 'MAIN', ?, ?, 1)",
            (campus_id.to_string(), school_id.to_string(), stamp, stamp),
        )
        .expect("campus");
        // Several buildings, so the identifiers also have to differ from each other.
        for name in ["A", "B", "C", "D", "E"] {
            conn.execute(
                "INSERT INTO building (id, campus_id, name, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, 1)",
                (
                    Uuid::now_v7().to_string(),
                    campus_id.to_string(),
                    name,
                    stamp,
                    stamp,
                ),
            )
            .expect("building");
        }
    }

    let db = Database::open_existing(&path, APP_VERSION, site).expect("migrate and open");

    // The migration mints these in SQL, because refinery migrations are .sql files and
    // SQLite has no uuid(). The printf that lays out the v7 fields is asserted here rather
    // than trusted.
    let (ids, created): (Vec<String>, Vec<String>) = db
        .read(|conn| {
            let mut stmt = conn.prepare("SELECT id, created_at FROM building_campus")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            Ok(rows
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .unzip())
        })
        .expect("read associations");

    assert_eq!(ids.len(), 5, "one association per existing building");
    for id in &ids {
        let parsed = Uuid::parse_str(id).unwrap_or_else(|e| panic!("{id} is not a UUID: {e}"));
        assert_eq!(parsed.get_version_num(), 7, "{id} must be UUIDv7");
        assert_eq!(parsed.get_variant(), uuid::Variant::RFC4122, "{id}");
    }
    assert_eq!(
        ids.iter().collect::<BTreeSet<_>>().len(),
        5,
        "and they must be distinct"
    );
    for timestamp in &created {
        assert_eq!(
            timestamp, stamp,
            "the association is as old as the relationship it records"
        );
    }
}
