//! Phase 1C tests: the repository contract, the static Structure entities, and
//! schema-level change-log coverage.
//!
//! Real temporary project files throughout, never `:memory:` — foreign keys, WAL and
//! trigger behaviour are all properties of a real database file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use tempfile::TempDir;
use uuid::Uuid;

use crate::repo::structure::{
    AcademicYearInput, BuildingInput, CampusInput, ResourceInput, RoomInput, RoomTypeInput,
    SchoolInput, TermInput,
};
use crate::workspace::Workspace;
use crate::{Database, DbError, PROJECT_EXTENSION, change_log};

const APP_VERSION: &str = "0.1.0-test";

/// Tables that are infrastructure rather than school data, and so carry no change log.
const NON_ENTITY_TABLES: &[&str] = &["app_meta", "change_log", "refinery_schema_history"];

/// An open project in its own temporary directory.
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

    /// A school with one campus, the usual starting point.
    fn with_school(&mut self) -> (Uuid, Uuid) {
        let school = self
            .db()
            .structure()
            .create_school(&SchoolInput {
                name: "Northgate Secondary".to_owned(),
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
            })
            .expect("create school");

        let campus = self
            .db()
            .structure()
            .create_campus(&CampusInput {
                school_id: school.id,
                name: "Main".to_owned(),
                code: "MAIN".to_owned(),
                address: None,
            })
            .expect("create campus");

        (school.id, campus.id)
    }
}

fn room(campus_id: Uuid, code: &str) -> RoomInput {
    RoomInput {
        campus_id,
        building_id: None,
        room_type_id: None,
        name: format!("Room {code}"),
        code: code.to_owned(),
        capacity: 30,
        is_bookable: true,
        notes: None,
    }
}

// ============================================================ change-log coverage

/// The guard that makes omitting change tracking impossible to do quietly.
///
/// Discovers entity tables from the live schema rather than consulting a list, so a table
/// added by a future migration is covered the moment it exists. A developer who adds a
/// table and forgets [`change_log::TRACKED_TABLES`] fails here, not in production.
#[test]
fn every_entity_table_has_all_three_change_log_triggers() {
    let mut project = Project::new();
    let tables = entity_tables(project.db());
    assert!(!tables.is_empty(), "the discovery query itself must work");

    let triggers = triggers(project.db());
    let mut missing = Vec::new();

    for table in &tables {
        for operation in ["insert", "update", "delete"] {
            let expected = format!("trg_{table}_changelog_{operation}");
            if !triggers.contains(&expected) {
                missing.push(expected);
            }
        }
    }

    assert!(
        missing.is_empty(),
        "entity tables are missing change-log triggers: {missing:?}\n\
         Add the table to `change_log::TRACKED_TABLES`, or add it to NON_ENTITY_TABLES if \
         it genuinely holds no school data."
    );
}

/// Catches the opposite mistake: a table listed as tracked that no longer exists.
#[test]
fn every_tracked_table_exists_in_the_schema() {
    let mut project = Project::new();
    let tables = entity_tables(project.db());

    for tracked in change_log::TRACKED_TABLES {
        assert!(
            tables.contains(*tracked),
            "`{tracked}` is registered for change tracking but no such table exists"
        );
    }
}

#[test]
fn every_entity_table_carries_the_common_metadata_columns() {
    let mut project = Project::new();
    let tables = entity_tables(project.db());

    for table in tables {
        let columns = columns_of(project.db(), &table);
        for required in ["id", "created_at", "updated_at", "rev"] {
            assert!(
                columns.contains(required),
                "`{table}` is missing the `{required}` column every entity table must carry"
            );
        }
    }
}

fn entity_tables(db: &Database) -> BTreeSet<String> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows
            .collect::<rusqlite::Result<BTreeSet<_>>>()?
            .into_iter()
            .filter(|name| !NON_ENTITY_TABLES.contains(&name.as_str()))
            .collect())
    })
    .expect("list tables")
}

fn triggers(db: &Database) -> BTreeSet<String> {
    db.read(|conn| {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<BTreeSet<_>>>()?)
    })
    .expect("list triggers")
}

fn columns_of(db: &Database, table: &str) -> BTreeSet<String> {
    db.read(|conn| {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        Ok(rows.collect::<rusqlite::Result<BTreeSet<_>>>()?)
    })
    .expect("read columns")
}

// ============================================================== repository contract

#[test]
fn a_created_row_starts_at_revision_one() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    let campus = project
        .db()
        .structure()
        .campus(campus)
        .expect("read")
        .expect("exists");
    assert_eq!(campus.rev, 1, "a new row is revision 1");
    assert_eq!(
        campus.created_at, campus.updated_at,
        "an untouched row was created and updated at the same moment"
    );
}

#[test]
fn an_update_increments_revision_exactly_once() {
    let mut project = Project::new();
    let (school, campus_id) = project.with_school();

    let input = CampusInput {
        school_id: school,
        name: "Main Site".to_owned(),
        code: "MAIN".to_owned(),
        address: Some("1 Example Street".to_owned()),
    };
    let updated = project
        .db()
        .structure()
        .update_campus(campus_id, &input)
        .expect("update");

    assert_eq!(updated.rev, 2, "one update, one revision");
    assert_eq!(updated.name, "Main Site");

    let again = project
        .db()
        .structure()
        .update_campus(campus_id, &input)
        .expect("update again");
    assert_eq!(again.rev, 3, "a second update advances by exactly one");
}

#[test]
fn an_update_replaces_updated_at_but_not_created_at() {
    let mut project = Project::new();
    let (school, campus_id) = project.with_school();
    let before = project.db().structure().campus(campus_id).unwrap().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(5));
    let after = project
        .db()
        .structure()
        .update_campus(
            campus_id,
            &CampusInput {
                school_id: school,
                name: "Renamed".to_owned(),
                code: "MAIN".to_owned(),
                address: None,
            },
        )
        .expect("update");

    assert_eq!(
        after.created_at, before.created_at,
        "created_at is immutable"
    );
    assert!(
        after.updated_at > before.updated_at,
        "updated_at must advance: {} -> {}",
        before.updated_at,
        after.updated_at
    );
}

#[test]
fn a_failed_update_does_not_consume_a_revision() {
    let mut project = Project::new();
    let (school, campus_id) = project.with_school();

    project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: school,
            name: "Second".to_owned(),
            code: "SECOND".to_owned(),
            address: None,
        })
        .expect("create second campus");

    // Collide with the other campus's code.
    let clash = project.db().structure().update_campus(
        campus_id,
        &CampusInput {
            school_id: school,
            name: "Main".to_owned(),
            code: "SECOND".to_owned(),
            address: None,
        },
    );
    assert!(
        matches!(clash, Err(DbError::DuplicateValue { .. })),
        "{clash:?}"
    );

    let unchanged = project.db().structure().campus(campus_id).unwrap().unwrap();
    assert_eq!(
        unchanged.rev, 1,
        "a rejected update must not advance the revision"
    );
    assert_eq!(unchanged.code, "MAIN", "nor change anything else");
}

#[test]
fn updating_something_that_does_not_exist_reports_not_found() {
    let mut project = Project::new();
    let (school, _) = project.with_school();

    let outcome = project.db().structure().update_campus(
        Uuid::now_v7(),
        &CampusInput {
            school_id: school,
            name: "Ghost".to_owned(),
            code: "GHOST".to_owned(),
            address: None,
        },
    );
    assert!(
        matches!(outcome, Err(DbError::NotFound { entity: "campus" })),
        "{outcome:?}"
    );
}

// ================================================================= CRUD round trips

#[test]
fn campus_create_read_list_update_delete() {
    let mut project = Project::new();
    let (school, first) = project.with_school();

    let second = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: school,
            name: "Annexe".to_owned(),
            code: "ANX".to_owned(),
            address: None,
        })
        .expect("create");

    let listed = project.db().structure().campuses().expect("list");
    assert_eq!(
        listed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Annexe", "Main"],
        "listing is ordered by name, deterministically"
    );

    project
        .db()
        .structure()
        .delete_campus(second.id)
        .expect("delete");
    assert!(
        project
            .db()
            .structure()
            .campus(second.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(project.db().structure().campuses().unwrap().len(), 1);
    assert!(project.db().structure().campus(first).unwrap().is_some());
}

#[test]
fn terms_list_in_teaching_order() {
    let mut project = Project::new();
    let (school, _) = project.with_school();

    let year = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: school,
            name: "2026/27".to_owned(),
            starts_on: "2026-09-01".to_owned(),
            ends_on: "2027-07-20".to_owned(),
        })
        .expect("create year");

    for (ordinal, name) in [(3, "Summer"), (1, "Autumn"), (2, "Spring")] {
        project
            .db()
            .structure()
            .create_term(&TermInput {
                academic_year_id: year.id,
                name: name.to_owned(),
                ordinal,
                starts_on: "2026-09-01".to_owned(),
                ends_on: "2026-12-18".to_owned(),
            })
            .expect("create term");
    }

    let terms = project.db().structure().terms(year.id).expect("list");
    assert_eq!(
        terms.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Autumn", "Spring", "Summer"],
        "terms list by ordinal, not by name or insertion order"
    );
}

// ============================================================== deletion semantics

#[test]
fn a_campus_that_still_owns_buildings_cannot_be_deleted() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Science Block".to_owned(),
        })
        .expect("create building");

    let outcome = project.db().structure().delete_campus(campus);
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { entity: "campus" })),
        "deleting a campus with buildings must be refused, not cascaded: {outcome:?}"
    );
    assert!(project.db().structure().campus(campus).unwrap().is_some());
}

#[test]
fn a_campus_that_still_owns_rooms_cannot_be_deleted() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();
    project
        .db()
        .structure()
        .create_room(&room(campus, "R1"))
        .expect("create room");

    let outcome = project.db().structure().delete_campus(campus);
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "{outcome:?}"
    );
}

#[test]
fn a_campus_that_still_owns_resources_cannot_be_deleted() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();
    project
        .db()
        .structure()
        .create_resource(&ResourceInput {
            school_id: school,
            campus_id: Some(campus),
            name: "Projector trolley".to_owned(),
            code: "PROJ".to_owned(),
            quantity: 3,
        })
        .expect("create resource");

    let outcome = project.db().structure().delete_campus(campus);
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "campus deletion means the same thing everywhere: {outcome:?}"
    );
}

#[test]
fn deleting_a_room_type_unclassifies_its_rooms_and_advances_their_revision() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();

    let lab = project
        .db()
        .structure()
        .create_room_type(&RoomTypeInput {
            school_id: school,
            name: "Science Lab".to_owned(),
            code: "LAB".to_owned(),
        })
        .expect("create room type");

    let mut input = room(campus, "S1");
    input.room_type_id = Some(lab.id);
    let created = project
        .db()
        .structure()
        .create_room(&input)
        .expect("create room");
    assert_eq!(created.rev, 1);

    project
        .db()
        .structure()
        .delete_room_type(lab.id)
        .expect("delete room type");

    let after = project
        .db()
        .structure()
        .room(created.id)
        .unwrap()
        .expect("room survives");
    assert_eq!(
        after.room_type_id, None,
        "the room is unclassified, not deleted"
    );
    assert_eq!(
        after.rev, 2,
        "detaching is a real update, so the revision advances and the change feed stays true"
    );
}

#[test]
fn deleting_a_building_detaches_its_rooms() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    let block = project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Science Block".to_owned(),
        })
        .expect("create building");

    let mut input = room(campus, "S1");
    input.building_id = Some(block.id);
    let created = project
        .db()
        .structure()
        .create_room(&input)
        .expect("create room");

    project
        .db()
        .structure()
        .delete_building(block.id)
        .expect("delete building");

    let after = project
        .db()
        .structure()
        .room(created.id)
        .unwrap()
        .expect("room survives");
    assert_eq!(after.building_id, None);
    assert_eq!(after.rev, 2);
}

#[test]
fn an_academic_year_with_terms_cannot_be_deleted() {
    let mut project = Project::new();
    let (school, _) = project.with_school();

    let year = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: school,
            name: "2026/27".to_owned(),
            starts_on: "2026-09-01".to_owned(),
            ends_on: "2027-07-20".to_owned(),
        })
        .expect("create year");
    project
        .db()
        .structure()
        .create_term(&TermInput {
            academic_year_id: year.id,
            name: "Autumn".to_owned(),
            ordinal: 1,
            starts_on: "2026-09-01".to_owned(),
            ends_on: "2026-12-18".to_owned(),
        })
        .expect("create term");

    let outcome = project.db().structure().delete_academic_year(year.id);
    assert!(
        matches!(
            outcome,
            Err(DbError::StillReferenced {
                entity: "academic year"
            })
        ),
        "terms are removed deliberately, never as a side effect: {outcome:?}"
    );
}

#[test]
fn a_school_with_dependent_structure_cannot_be_deleted() {
    let mut project = Project::new();
    let (school, _campus) = project.with_school();

    // There is no delete_school on the repository by design, so this exercises the
    // database rule directly: the school is the root and nothing may orphan it.
    let outcome = project
        .db()
        .write(|tx| crate::repo::delete(tx, "school", "school", school));

    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "a school owning a campus must not be removable: {outcome:?}"
    );
}

// ============================================================ relational integrity

#[test]
fn a_campus_cannot_reference_a_school_that_does_not_exist() {
    let mut project = Project::new();
    let outcome = project.db().structure().create_campus(&CampusInput {
        school_id: Uuid::now_v7(),
        name: "Orphan".to_owned(),
        code: "ORP".to_owned(),
        address: None,
    });
    assert!(
        matches!(outcome, Err(DbError::StillReferenced { .. })),
        "{outcome:?}"
    );
}

#[test]
fn a_room_cannot_sit_in_a_building_on_another_campus() {
    let mut project = Project::new();
    let (school, main) = project.with_school();

    let annexe = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: school,
            name: "Annexe".to_owned(),
            code: "ANX".to_owned(),
            address: None,
        })
        .expect("create campus");
    let block = project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: annexe.id,
            name: "Far Block".to_owned(),
        })
        .expect("create building");

    let mut input = room(main, "X1");
    input.building_id = Some(block.id);

    let outcome = project.db().structure().create_room(&input);
    assert!(
        matches!(outcome, Err(DbError::Invalid { .. })),
        "a room's building must be on the room's own campus: {outcome:?}"
    );
}

#[test]
fn campus_codes_are_unique_within_a_school() {
    let mut project = Project::new();
    let (school, _) = project.with_school();

    let outcome = project.db().structure().create_campus(&CampusInput {
        school_id: school,
        name: "Different name".to_owned(),
        code: "MAIN".to_owned(),
        address: None,
    });
    assert!(
        matches!(outcome, Err(DbError::DuplicateValue { .. })),
        "{outcome:?}"
    );
}

#[test]
fn room_codes_are_unique_within_a_campus_but_not_across_them() {
    let mut project = Project::new();
    let (school, main) = project.with_school();
    project
        .db()
        .structure()
        .create_room(&room(main, "R1"))
        .expect("first");

    let clash = project.db().structure().create_room(&room(main, "R1"));
    assert!(
        matches!(clash, Err(DbError::DuplicateValue { .. })),
        "{clash:?}"
    );

    let annexe = project
        .db()
        .structure()
        .create_campus(&CampusInput {
            school_id: school,
            name: "Annexe".to_owned(),
            code: "ANX".to_owned(),
            address: None,
        })
        .expect("create campus");
    assert!(
        project
            .db()
            .structure()
            .create_room(&room(annexe.id, "R1"))
            .is_ok(),
        "the same code on a different campus is a different room"
    );
}

#[test]
fn only_one_school_can_exist_per_project() {
    let mut project = Project::new();
    project.with_school();

    let second = project.db().structure().create_school(&SchoolInput {
        name: "Another School".to_owned(),
        timezone: "Europe/London".to_owned(),
        locale: "en-GB".to_owned(),
    });
    assert!(
        second.is_err(),
        "a project describes one school; multiple schools means multiple project files"
    );
}

#[test]
fn schema_rules_reject_nonsense_values() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();

    let mut negative = room(campus, "NEG");
    negative.capacity = -5;
    assert!(
        project.db().structure().create_room(&negative).is_err(),
        "capacity cannot be negative"
    );

    let blank = project.db().structure().create_campus(&CampusInput {
        school_id: school,
        name: "   ".to_owned(),
        code: "BLANK".to_owned(),
        address: None,
    });
    assert!(blank.is_err(), "a blank name is not a name");

    let backwards = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: school,
            name: "Backwards".to_owned(),
            starts_on: "2027-07-20".to_owned(),
            ends_on: "2026-09-01".to_owned(),
        });
    assert!(backwards.is_err(), "a year cannot end before it starts");

    let nonsense_date = project
        .db()
        .structure()
        .create_academic_year(&AcademicYearInput {
            school_id: school,
            name: "Nonsense".to_owned(),
            starts_on: "not-a-date".to_owned(),
            ends_on: "2027-07-20".to_owned(),
        });
    assert!(nonsense_date.is_err(), "dates must actually be dates");
}

// ==================================================================== change feed

#[test]
fn change_log_records_entity_writes_with_the_active_installation() {
    let mut project = Project::new();
    let site = project.workspace.site_id().to_string();
    let (_school, campus_id) = project.with_school();

    let changes = project.db().changes_since(0, 100).expect("read feed");
    let campus_changes: Vec<_> = changes
        .iter()
        .filter(|c| c.entity_table == "campus")
        .collect();

    assert_eq!(campus_changes.len(), 1);
    assert_eq!(campus_changes[0].op, "INSERT");
    assert_eq!(campus_changes[0].rev, 1);
    assert_eq!(
        campus_changes[0].site_id, site,
        "attributed to this installation"
    );
    assert_eq!(campus_changes[0].entity_id, campus_id.to_string());
}

#[test]
fn change_log_follows_a_row_through_update_and_delete() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    let block = project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Block A".to_owned(),
        })
        .expect("create");
    project
        .db()
        .structure()
        .update_building(
            block.id,
            &BuildingInput {
                campus_id: campus,
                name: "Block B".to_owned(),
            },
        )
        .expect("update");
    project
        .db()
        .structure()
        .delete_building(block.id)
        .expect("delete");

    let changes = project.db().changes_since(0, 100).expect("read feed");
    let ours: Vec<_> = changes
        .iter()
        .filter(|c| c.entity_id == block.id.to_string())
        .collect();

    assert_eq!(
        ours.iter().map(|c| c.op.as_str()).collect::<Vec<_>>(),
        vec!["INSERT", "UPDATE", "DELETE"]
    );
    assert_eq!(ours[0].rev, 1);
    assert_eq!(
        ours[1].rev, 2,
        "the update records the revision the repository wrote"
    );
    assert_eq!(
        ours[2].rev, 2,
        "the delete records the vanishing row's revision"
    );
}

#[test]
fn a_rejected_write_leaves_neither_a_row_nor_a_change_log_entry() {
    let mut project = Project::new();
    let (school, _) = project.with_school();
    let before = project
        .db()
        .changes_since(0, 1000)
        .expect("read feed")
        .len();

    let outcome = project.db().structure().create_campus(&CampusInput {
        school_id: school,
        name: "Clash".to_owned(),
        code: "MAIN".to_owned(),
        address: None,
    });
    assert!(outcome.is_err());

    assert_eq!(
        project
            .db()
            .changes_since(0, 1000)
            .expect("read feed")
            .len(),
        before,
        "a rolled-back write must leave the change feed untouched"
    );
    assert_eq!(
        project.db().structure().campuses().expect("list").len(),
        1,
        "and must leave no row behind"
    );
}

#[test]
fn a_multi_step_delete_is_one_transaction() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();

    let lab = project
        .db()
        .structure()
        .create_room_type(&RoomTypeInput {
            school_id: school,
            name: "Lab".to_owned(),
            code: "LAB".to_owned(),
        })
        .expect("create type");

    for code in ["S1", "S2", "S3"] {
        let mut input = room(campus, code);
        input.room_type_id = Some(lab.id);
        project
            .db()
            .structure()
            .create_room(&input)
            .expect("create room");
    }

    let before = project.db().changes_since(0, 1000).expect("feed").len();
    project
        .db()
        .structure()
        .delete_room_type(lab.id)
        .expect("delete");
    let after = project.db().changes_since(0, 1000).expect("feed");

    // Three room updates plus one room_type delete, all from one logical operation.
    assert_eq!(after.len(), before + 4);
    let tail = &after[after.len() - 4..];
    assert_eq!(tail.iter().filter(|c| c.op == "UPDATE").count(), 3);
    assert_eq!(tail.iter().filter(|c| c.op == "DELETE").count(), 1);
}

// ======================================================================= durability

#[test]
fn structure_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();

    let block = project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Science Block".to_owned(),
        })
        .expect("create building");
    let lab = project
        .db()
        .structure()
        .create_room_type(&RoomTypeInput {
            school_id: school,
            name: "Science Lab".to_owned(),
            code: "LAB".to_owned(),
        })
        .expect("create type");

    let mut input = room(campus, "S1");
    input.building_id = Some(block.id);
    input.room_type_id = Some(lab.id);
    input.capacity = 24;
    let created = project
        .db()
        .structure()
        .create_room(&input)
        .expect("create room");

    project.workspace.close_project();
    project
        .workspace
        .open_project(&project.path)
        .expect("reopen");

    let reloaded = project
        .db()
        .structure()
        .room(created.id)
        .expect("read")
        .expect("the room is still there");

    assert_eq!(reloaded, created, "every field survives a close and reopen");
    assert_eq!(
        project.db().structure().school().unwrap().unwrap().id,
        school
    );
    assert_eq!(project.db().structure().buildings(campus).unwrap().len(), 1);
    assert_eq!(project.db().structure().room_types().unwrap().len(), 1);
}

// =================================================== stored identifier corruption

/// Writes raw SQL straight past the repositories, to manufacture a damaged file.
///
/// The only way to test the corruption path: every repository writes well-formed
/// identifiers, so the damage has to be done behind their back — which is exactly what an
/// external editor, a truncated write or a failing disk does.
fn corrupt(db: &mut Database, sql: &str) {
    db.write(|tx| {
        tx.execute_batch(sql)?;
        Ok(())
    })
    .expect("corrupting SQL must itself succeed");
}

/// The Phase 1C defect this phase opened by fixing.
///
/// `parse_id` substituted `Uuid::nil()` for anything it could not parse, so a damaged file
/// read as a *valid* one in which several rows shared the all-zero identifier. Nothing
/// downstream could tell the difference.
#[test]
fn a_malformed_stored_identifier_is_an_error_never_the_nil_uuid() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    corrupt(
        project.db(),
        "UPDATE campus SET id = 'not-a-uuid' WHERE id IS NOT NULL",
    );

    let error = project
        .db()
        .structure()
        .campuses()
        .expect_err("reading a damaged identifier must fail");

    match error {
        DbError::CorruptIdentifier { column, value } => {
            assert_eq!(column, "id", "the error should name the column");
            assert_eq!(value, "not-a-uuid", "and carry the value as stored");
        }
        other => panic!("expected CorruptIdentifier, got {other:?}"),
    }

    // The specific regression: no read may yield the nil identifier.
    assert_ne!(campus, Uuid::nil(), "the real identifier was never nil");
    assert!(
        project.db().structure().campus(Uuid::nil()).is_ok(),
        "looking up the nil identifier is a miss, not an error"
    );
}

/// A damaged identifier must not be reported as a generic internal failure.
///
/// It travels out through the driver as a conversion failure, so without the recovery in
/// `From<rusqlite::Error>` it would arrive as `Internal` and read as "the database
/// reported an error" — indistinguishable from a locked file.
#[test]
fn a_malformed_stored_identifier_is_not_a_generic_internal_error() {
    let mut project = Project::new();
    project.with_school();

    // Empty text rather than rubbish text: the other end of the same hole.
    corrupt(project.db(), "UPDATE campus SET id = ''");

    let error = project
        .db()
        .structure()
        .campuses()
        .expect_err("reading a damaged identifier must fail");

    assert!(
        matches!(error, DbError::CorruptIdentifier { .. }),
        "expected CorruptIdentifier, got {error:?}"
    );
    assert!(
        !matches!(error, DbError::Internal(_)),
        "corruption must not degrade to an internal failure"
    );
}

/// The nullable-reference decoder has its own path and its own former bug.
///
/// `Option<Uuid>` mapped `parse_id` over the value, so a damaged optional reference became
/// `Some(nil)` — a room that claimed to belong to a building that cannot exist.
#[test]
fn a_malformed_optional_reference_is_an_error_not_some_nil() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    let block = project
        .db()
        .structure()
        .create_building(&BuildingInput {
            campus_id: campus,
            name: "Block A".to_owned(),
        })
        .expect("create building");

    let mut input = room(campus, "A1");
    input.building_id = Some(block.id);
    let created = project
        .db()
        .structure()
        .create_room(&input)
        .expect("create room");

    // Both ends are damaged to the *same* text, so the file stays referentially consistent
    // and the corruption is purely in the format. `foreign_keys` cannot be turned off
    // inside a transaction — SQLite ignores it there — so enforcement is deferred to commit
    // instead, which lets the two statements disagree in between.
    corrupt(
        project.db(),
        "PRAGMA defer_foreign_keys = ON;\n\
         UPDATE building SET id = 'xxxx';\n\
         UPDATE room SET building_id = 'xxxx';",
    );

    let error = project
        .db()
        .structure()
        .room(created.id)
        .expect_err("reading a damaged optional reference must fail");

    match error {
        DbError::CorruptIdentifier { column, value } => {
            assert_eq!(column, "building_id");
            assert_eq!(value, "xxxx");
        }
        other => panic!("expected CorruptIdentifier, got {other:?}"),
    }
}

/// A `NULL` optional reference is absence, not corruption.
///
/// Guards the obvious over-correction: making the decoder strict must not turn every
/// unset reference into a failure.
#[test]
fn an_absent_optional_reference_is_still_simply_absent() {
    let mut project = Project::new();
    let (_school, campus) = project.with_school();

    let created = project
        .db()
        .structure()
        .create_room(&room(campus, "B1"))
        .expect("create room");

    let read = project
        .db()
        .structure()
        .room(created.id)
        .expect("read")
        .expect("present");

    assert_eq!(read.building_id, None);
    assert_eq!(read.room_type_id, None);
}

/// Detaching optional references reads identifiers on its own, outside row decoding.
///
/// That path reported `NotFound` for a damaged identifier until Phase 1D, which claimed
/// the room was absent when it is present and broken.
#[test]
fn detaching_a_reference_reports_corruption_rather_than_a_missing_row() {
    let mut project = Project::new();
    let (school, campus) = project.with_school();

    let lab = project
        .db()
        .structure()
        .create_room_type(&RoomTypeInput {
            school_id: school,
            name: "Science Lab".to_owned(),
            code: "LAB".to_owned(),
        })
        .expect("create type");

    let mut input = room(campus, "S2");
    input.room_type_id = Some(lab.id);
    project
        .db()
        .structure()
        .create_room(&input)
        .expect("create room");

    // Nothing references `room`, so its own identifier can be damaged outright.
    corrupt(project.db(), "UPDATE room SET id = 'broken'");

    let error = project
        .db()
        .structure()
        .delete_room_type(lab.id)
        .expect_err("detaching a damaged room must fail");

    assert!(
        matches!(error, DbError::CorruptIdentifier { .. }),
        "expected CorruptIdentifier, got {error:?}"
    );
    assert!(
        !matches!(error, DbError::NotFound { .. }),
        "a present, damaged row must not be reported as missing"
    );
}

/// A failed read changes nothing, and the damaged row is still there to be repaired.
#[test]
fn a_failed_read_leaves_the_project_untouched() {
    let mut project = Project::new();
    project.with_school();

    corrupt(project.db(), "UPDATE campus SET id = 'not-a-uuid'");

    assert!(project.db().structure().campuses().is_err());

    let still_there: i64 = project
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT count(*) FROM campus WHERE id = 'not-a-uuid'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("count");

    assert_eq!(
        still_there, 1,
        "refusing to read must not delete or rewrite the damaged row"
    );
}
