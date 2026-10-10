//! Repositories for the static Structure entities.
//!
//! Places (school, campus, building, room type, room, resource) and the coarse academic
//! calendar (academic year, term). The *time* half of the Structure group — cycle,
//! cycle day, period structure, period, timeslot and calendar day — is [`super::time_model`].
//!
//! # Shape
//!
//! Domain-oriented, not SQL-oriented: callers pass an input struct and receive a record.
//! No `rusqlite` type appears in any signature, and no caller writes SQL.
//!
//! Every write goes through [`super::update`] or [`super::delete`], which own `rev` and
//! `updated_at`. A repository here never writes those columns itself — that is what makes
//! D17 hard to violate rather than merely documented.
//!
//! `campus` was written first and the rest follow its shape deliberately, so the pattern is
//! obvious and a reviewer can check one and skim the others.
//!
//! Row decoding and the `entity!` declaration macro live in [`super::row`], shared with the
//! time model.

use rusqlite::types::ToSql;
use uuid::Uuid;

use super::row::{entity, opt_text, parse_id_text, text};
use super::{Stamp, delete, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing Structure entities.
///
/// Obtained from [`Database::structure`]. Holds the database mutably because most
/// operations write.
#[derive(Debug)]
pub struct Structure<'a> {
    db: &'a mut Database,
}

impl<'a> Structure<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

entity! {
    /// The school this project describes. Exactly one per project.
    School,
    /// Values for creating or replacing the school record.
    SchoolInput,
    {
        /// Display name.
        name: String,
        /// IANA timezone, e.g. `Europe/London`.
        timezone: String,
        /// BCP-47 locale, e.g. `en-GB`.
        locale: String,
    }
}

entity! {
    /// A physical site belonging to the school.
    Campus,
    /// Values for creating or replacing a campus.
    CampusInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name.
        name: String,
        /// Short code, unique within the school.
        code: String,
        /// Optional postal address.
        address: Option<String>,
    }
}

entity! {
    /// A building on a campus.
    Building,
    /// Values for creating or replacing a building.
    BuildingInput,
    {
        /// Owning campus.
        campus_id: Uuid,
        /// Display name, unique within the campus.
        name: String,
    }
}

entity! {
    /// A kind of room, such as "Science Lab" or "Gym".
    RoomType,
    /// Values for creating or replacing a room type.
    RoomTypeInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name.
        name: String,
        /// Short code, unique within the school.
        code: String,
    }
}

entity! {
    /// A teaching space.
    Room,
    /// Values for creating or replacing a room.
    RoomInput,
    {
        /// Campus the room is on.
        campus_id: Uuid,
        /// Building, when the school models them. Must be on the same campus.
        building_id: Option<Uuid>,
        /// Kind of room, when classified.
        room_type_id: Option<Uuid>,
        /// Display name.
        name: String,
        /// Short code, unique within the campus.
        code: String,
        /// How many students it seats.
        capacity: i64,
        /// Whether the room may be scheduled into.
        is_bookable: bool,
        /// Free text.
        notes: Option<String>,
    }
}

entity! {
    /// A shared, countable thing a lesson may need — a projector trolley, a minibus.
    Resource,
    /// Values for creating or replacing a resource.
    ResourceInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Campus it is tied to, or `None` for school-wide.
        campus_id: Option<Uuid>,
        /// Display name.
        name: String,
        /// Short code, unique within the school.
        code: String,
        /// How many exist.
        quantity: i64,
    }
}

entity! {
    /// An academic year.
    AcademicYear,
    /// Values for creating or replacing an academic year.
    AcademicYearInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// Display name, unique within the school.
        name: String,
        /// First day, `YYYY-MM-DD`.
        starts_on: String,
        /// Last day, `YYYY-MM-DD`.
        ends_on: String,
    }
}

entity! {
    /// A division of an academic year.
    Term,
    /// Values for creating or replacing a term.
    TermInput,
    {
        /// Owning academic year.
        academic_year_id: Uuid,
        /// Display name, unique within the year.
        name: String,
        /// Position within the year, starting at 1.
        ordinal: i64,
        /// First day, `YYYY-MM-DD`.
        starts_on: String,
        /// Last day, `YYYY-MM-DD`.
        ends_on: String,
    }
}

// ---------------------------------------------------------------------------- school

const SCHOOL_COLUMNS: &str = "id, name, timezone, locale, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates the school record.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if a school already exists — a project describes exactly
    /// one school ([ADR 0003](../../../docs/adr/0003-sqlite-document-model.md)).
    pub fn create_school(&mut self, input: &SchoolInput) -> Result<School> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO school (id, name, timezone, locale, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    &input.name,
                    &input.timezone,
                    &input.locale,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "school"))?;
            Ok(())
        })?;

        Ok(School {
            id: stamp.id,
            name: input.name.clone(),
            timezone: input.timezone.clone(),
            locale: input.locale.clone(),
            created_at: stamp.created_at,
            updated_at: stamp.updated_at,
            rev: stamp.rev,
        })
    }

    /// The school, or `None` for a project where it has not been set up yet.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn school(&self) -> Result<Option<School>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!("SELECT {SCHOOL_COLUMNS} FROM school LIMIT 1"))?;
            let mut rows = stmt.query_map([], School::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Replaces the school's details.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if no school exists yet.
    pub fn update_school(&mut self, id: Uuid, input: &SchoolInput) -> Result<School> {
        self.db.write(|tx| {
            update(
                tx,
                "school",
                "school",
                id,
                "name = ?, timezone = ?, locale = ?",
                &[&input.name, &input.timezone, &input.locale],
            )
        })?;
        self.require_school(id)
    }

    fn require_school(&self, id: Uuid) -> Result<School> {
        self.db.read(|conn| {
            conn.query_row(
                &format!("SELECT {SCHOOL_COLUMNS} FROM school WHERE id = ?"),
                [text(id)],
                School::from_row,
            )
            .map_err(|_| DbError::NotFound { entity: "school" })
        })
    }
}

// ---------------------------------------------------------------------------- campus

const CAMPUS_COLUMNS: &str = "id, school_id, name, code, address, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a campus.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken within the school;
    /// [`DbError::StillReferenced`] shape errors if the school does not exist.
    pub fn create_campus(&mut self, input: &CampusInput) -> Result<Campus> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO campus (id, school_id, name, code, address, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    &input.code,
                    &input.address,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "campus"))?;
            Ok(())
        })?;
        self.campus(stamp.id)?
            .ok_or(DbError::NotFound { entity: "campus" })
    }

    /// One campus by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn campus(&self, id: Uuid) -> Result<Option<Campus>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {CAMPUS_COLUMNS} FROM campus WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Campus::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every campus, ordered by name then identifier so the order never varies.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn campuses(&self) -> Result<Vec<Campus>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CAMPUS_COLUMNS} FROM campus ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], Campus::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a campus's details. Its school never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_campus(&mut self, id: Uuid, input: &CampusInput) -> Result<Campus> {
        self.db.write(|tx| {
            update(
                tx,
                "campus",
                "campus",
                id,
                "name = ?, code = ?, address = ?",
                &[&input.name, &input.code, &input.address],
            )
        })?;
        self.campus(id)?
            .ok_or(DbError::NotFound { entity: "campus" })
    }

    /// Deletes a campus.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if buildings, rooms or resources remain on it. Campus
    /// ownership is RESTRICT: the user removes or moves those deliberately.
    pub fn delete_campus(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "campus", "campus", id))
    }
}

// -------------------------------------------------------------------------- building

const BUILDING_COLUMNS: &str = "id, campus_id, name, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a building.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name is taken on that campus.
    pub fn create_building(&mut self, input: &BuildingInput) -> Result<Building> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO building (id, campus_id, name, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.campus_id),
                    &input.name,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "building"))?;
            Ok(())
        })?;
        self.building(stamp.id)?
            .ok_or(DbError::NotFound { entity: "building" })
    }

    /// One building by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn building(&self, id: Uuid) -> Result<Option<Building>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {BUILDING_COLUMNS} FROM building WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Building::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Buildings on one campus, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn buildings(&self, campus_id: Uuid) -> Result<Vec<Building>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {BUILDING_COLUMNS} FROM building WHERE campus_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(campus_id)], Building::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Renames a building.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_building(&mut self, id: Uuid, input: &BuildingInput) -> Result<Building> {
        self.db
            .write(|tx| update(tx, "building", "building", id, "name = ?", &[&input.name]))?;
        self.building(id)?
            .ok_or(DbError::NotFound { entity: "building" })
    }

    /// Deletes a building, detaching any rooms in it first.
    ///
    /// The rooms survive — a room is an asset, its building is metadata. They are detached
    /// by a proper update so each one's `rev` advances and the change feed stays truthful;
    /// the schema's `ON DELETE SET NULL` is only a backstop for writes that bypass this.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such building.
    pub fn delete_building(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            detach_rooms(tx, "building_id", id)?;
            delete(tx, "building", "building", id)
        })
    }
}

// ------------------------------------------------------------------------- room_type

const ROOM_TYPE_COLUMNS: &str = "id, school_id, name, code, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a room type.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken within the school.
    pub fn create_room_type(&mut self, input: &RoomTypeInput) -> Result<RoomType> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO room_type (id, school_id, name, code, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    &input.code,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "room type"))?;
            Ok(())
        })?;
        self.room_type(stamp.id)?.ok_or(DbError::NotFound {
            entity: "room type",
        })
    }

    /// One room type by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn room_type(&self, id: Uuid) -> Result<Option<RoomType>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROOM_TYPE_COLUMNS} FROM room_type WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], RoomType::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every room type, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn room_types(&self) -> Result<Vec<RoomType>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROOM_TYPE_COLUMNS} FROM room_type ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], RoomType::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a room type's details.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_room_type(&mut self, id: Uuid, input: &RoomTypeInput) -> Result<RoomType> {
        self.db.write(|tx| {
            update(
                tx,
                "room type",
                "room_type",
                id,
                "name = ?, code = ?",
                &[&input.name, &input.code],
            )
        })?;
        self.room_type(id)?.ok_or(DbError::NotFound {
            entity: "room type",
        })
    }

    /// Deletes a room type, unclassifying any rooms that used it.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such room type.
    pub fn delete_room_type(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            detach_rooms(tx, "room_type_id", id)?;
            delete(tx, "room type", "room_type", id)
        })
    }
}

/// Clears an optional room reference, advancing each affected room's revision.
///
/// `column` is a fixed string from this module, never caller input, and still passes
/// through the identifier check before interpolation.
fn detach_rooms(tx: &rusqlite::Transaction<'_>, column: &str, id: Uuid) -> Result<()> {
    let column = super::identifier(column)?;
    let mut stmt = tx.prepare(&format!("SELECT id FROM room WHERE {column} = ?"))?;
    let affected: Vec<String> = stmt
        .query_map([text(id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for room_id in affected {
        let room_id = parse_id_text("room.id", &room_id)?;
        let null: Option<String> = None;
        update(
            tx,
            "room",
            "room",
            room_id,
            &format!("{column} = ?"),
            &[&null],
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------------------ room

const ROOM_COLUMNS: &str = "id, campus_id, building_id, room_type_id, name, code, capacity, \
                            is_bookable, notes, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a room.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken on that campus, or
    /// [`DbError::Invalid`] if the building named is on a different campus.
    pub fn create_room(&mut self, input: &RoomInput) -> Result<Room> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO room (id, campus_id, building_id, room_type_id, name, code,
                                   capacity, is_bookable, notes, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.campus_id),
                    opt_text(input.building_id),
                    opt_text(input.room_type_id),
                    &input.name,
                    &input.code,
                    input.capacity,
                    input.is_bookable,
                    &input.notes,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(room_error)?;
            Ok(())
        })?;
        self.room(stamp.id)?
            .ok_or(DbError::NotFound { entity: "room" })
    }

    /// One room by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn room(&self, id: Uuid) -> Result<Option<Room>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {ROOM_COLUMNS} FROM room WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Room::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Rooms on one campus, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn rooms(&self, campus_id: Uuid) -> Result<Vec<Room>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROOM_COLUMNS} FROM room WHERE campus_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(campus_id)], Room::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a room's details. Its campus never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`], or [`DbError::Invalid`] if the
    /// building is on another campus.
    pub fn update_room(&mut self, id: Uuid, input: &RoomInput) -> Result<Room> {
        self.db.write(|tx| {
            let building = opt_text(input.building_id);
            let room_type = opt_text(input.room_type_id);
            update(
                tx,
                "room",
                "room",
                id,
                "building_id = ?, room_type_id = ?, name = ?, code = ?, capacity = ?, \
                 is_bookable = ?, notes = ?",
                &[
                    &building as &dyn ToSql,
                    &room_type,
                    &input.name,
                    &input.code,
                    &input.capacity,
                    &input.is_bookable,
                    &input.notes,
                ],
            )
        })?;
        self.room(id)?.ok_or(DbError::NotFound { entity: "room" })
    }

    /// Deletes a room.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such room.
    pub fn delete_room(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "room", "room", id))
    }
}

/// The cross-campus rule is enforced by a trigger, which surfaces as a generic SQLite
/// error rather than a constraint code, so it is recognised by its message.
fn room_error(error: rusqlite::Error) -> DbError {
    if error.to_string().contains("building must be on the room") {
        return DbError::Invalid {
            message: "that building is on a different campus from the room".to_owned(),
        };
    }
    translate(error, "room")
}

// -------------------------------------------------------------------------- resource

const RESOURCE_COLUMNS: &str =
    "id, school_id, campus_id, name, code, quantity, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a resource.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the code is taken within the school.
    pub fn create_resource(&mut self, input: &ResourceInput) -> Result<Resource> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO resource (id, school_id, campus_id, name, code, quantity,
                                       created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    opt_text(input.campus_id),
                    &input.name,
                    &input.code,
                    input.quantity,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "resource"))?;
            Ok(())
        })?;
        self.resource(stamp.id)?
            .ok_or(DbError::NotFound { entity: "resource" })
    }

    /// One resource by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn resource(&self, id: Uuid) -> Result<Option<Resource>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {RESOURCE_COLUMNS} FROM resource WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], Resource::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every resource, by name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn resources(&self) -> Result<Vec<Resource>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {RESOURCE_COLUMNS} FROM resource ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], Resource::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a resource's details.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_resource(&mut self, id: Uuid, input: &ResourceInput) -> Result<Resource> {
        self.db.write(|tx| {
            let campus = opt_text(input.campus_id);
            update(
                tx,
                "resource",
                "resource",
                id,
                "campus_id = ?, name = ?, code = ?, quantity = ?",
                &[
                    &campus as &dyn ToSql,
                    &input.name,
                    &input.code,
                    &input.quantity,
                ],
            )
        })?;
        self.resource(id)?
            .ok_or(DbError::NotFound { entity: "resource" })
    }

    /// Deletes a resource.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such resource.
    pub fn delete_resource(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "resource", "resource", id))
    }
}

// --------------------------------------------------------------------- academic_year

const ACADEMIC_YEAR_COLUMNS: &str =
    "id, school_id, name, starts_on, ends_on, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates an academic year.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name is taken, or [`DbError::Invalid`] if the
    /// dates are unparseable or end before they start.
    pub fn create_academic_year(&mut self, input: &AcademicYearInput) -> Result<AcademicYear> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO academic_year (id, school_id, name, starts_on, ends_on,
                                            created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    text(input.school_id),
                    &input.name,
                    &input.starts_on,
                    &input.ends_on,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ),
            )
            .map_err(|error| translate(error, "academic year"))?;
            Ok(())
        })?;
        self.academic_year(stamp.id)?.ok_or(DbError::NotFound {
            entity: "academic year",
        })
    }

    /// One academic year by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn academic_year(&self, id: Uuid) -> Result<Option<AcademicYear>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ACADEMIC_YEAR_COLUMNS} FROM academic_year WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], AcademicYear::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every academic year, earliest first.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn academic_years(&self) -> Result<Vec<AcademicYear>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ACADEMIC_YEAR_COLUMNS} FROM academic_year ORDER BY starts_on, name, id"
            ))?;
            let rows = stmt.query_map([], AcademicYear::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces an academic year's details.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_academic_year(
        &mut self,
        id: Uuid,
        input: &AcademicYearInput,
    ) -> Result<AcademicYear> {
        self.db.write(|tx| {
            update(
                tx,
                "academic year",
                "academic_year",
                id,
                "name = ?, starts_on = ?, ends_on = ?",
                &[&input.name, &input.starts_on, &input.ends_on],
            )
        })?;
        self.academic_year(id)?.ok_or(DbError::NotFound {
            entity: "academic year",
        })
    }

    /// Deletes an academic year.
    ///
    /// # Errors
    ///
    /// [`DbError::StillReferenced`] if it still has terms. Terms are removed deliberately,
    /// never as a side effect.
    pub fn delete_academic_year(&mut self, id: Uuid) -> Result<()> {
        self.db
            .write(|tx| delete(tx, "academic year", "academic_year", id))
    }
}

// ------------------------------------------------------------------------------ term

const TERM_COLUMNS: &str =
    "id, academic_year_id, name, ordinal, starts_on, ends_on, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a term.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name or ordinal is taken within the year.
    pub fn create_term(&mut self, input: &TermInput) -> Result<Term> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO term (id, academic_year_id, name, ordinal, starts_on, ends_on,
                                   created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.academic_year_id),
                    &input.name,
                    input.ordinal,
                    &input.starts_on,
                    &input.ends_on,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "term"))?;
            Ok(())
        })?;
        self.term(stamp.id)?
            .ok_or(DbError::NotFound { entity: "term" })
    }

    /// One term by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn term(&self, id: Uuid) -> Result<Option<Term>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {TERM_COLUMNS} FROM term WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], Term::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Terms of one academic year, in teaching order.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn terms(&self, academic_year_id: Uuid) -> Result<Vec<Term>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TERM_COLUMNS} FROM term WHERE academic_year_id = ? ORDER BY ordinal, id"
            ))?;
            let rows = stmt.query_map([text(academic_year_id)], Term::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// Replaces a term's details. Its academic year never changes.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`] or [`DbError::Invalid`].
    pub fn update_term(&mut self, id: Uuid, input: &TermInput) -> Result<Term> {
        self.db.write(|tx| {
            update(
                tx,
                "term",
                "term",
                id,
                "name = ?, ordinal = ?, starts_on = ?, ends_on = ?",
                &[
                    &input.name as &dyn ToSql,
                    &input.ordinal,
                    &input.starts_on,
                    &input.ends_on,
                ],
            )
        })?;
        self.term(id)?.ok_or(DbError::NotFound { entity: "term" })
    }

    /// Deletes a term.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such term.
    pub fn delete_term(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| delete(tx, "term", "term", id))
    }
}
