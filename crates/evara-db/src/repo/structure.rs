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

use std::collections::{HashMap, HashSet};

use rusqlite::types::ToSql;
use rusqlite::{OptionalExtension, Transaction};
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

/// The same columns, qualified, for the joins through `building_campus`.
const BUILDING_COLUMNS_QUALIFIED: &str =
    "b.id, b.campus_id, b.name, b.created_at, b.updated_at, b.rev";

/// A building together with every campus it serves.
///
/// Returned by [`Structure::buildings`] rather than a bare [`Building`] so a listing can
/// mark a shared building without a follow-up question per row.
///
/// `campus_ids` comes from `building_campus` and is complete: it includes
/// `building.campus_id`, the home campus. Nothing reads that column to answer "which
/// campuses?" — see [ADR 0013](../../../docs/adr/0013-buildings-shared-across-campuses.md).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BuildingWithCampuses {
    /// The building itself. Its identity is unaffected by sharing.
    pub building: Building,
    /// Every campus this building serves, by campus name. Never empty.
    pub campus_ids: Vec<Uuid>,
}

impl Structure<'_> {
    /// Creates a building on its home campus, optionally shared with others.
    ///
    /// One transaction: the building, its home association, and one association per entry
    /// in `shared_with`. A partial share is never persisted, so a user cannot end up with
    /// a building that exists but is missing half the campuses they asked for.
    ///
    /// Duplicates within `shared_with`, and the home campus appearing there, are ignored
    /// rather than rejected — the caller is a multi-select, and "you picked the campus you
    /// are already on" is not a mistake worth a round trip.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if the name is taken on the home campus or on any
    ///   campus it is being shared to.
    /// - [`DbError::NotFound`] if a named campus does not exist.
    pub fn create_building(
        &mut self,
        input: &BuildingInput,
        shared_with: &[Uuid],
    ) -> Result<Building> {
        let stamp = Stamp::new();
        let extra = others(shared_with, input.campus_id);

        self.db.write(|tx| {
            require_campus(tx, input.campus_id)?;
            for campus_id in &extra {
                require_campus(tx, *campus_id)?;
                require_name_free(tx, *campus_id, &input.name, None)?;
            }

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

            // The home campus is an association like any other. Writing it here rather
            // than leaving `building.campus_id` to imply it is what lets every later
            // query read one table and get the whole answer.
            associate(tx, stamp.id, input.campus_id)?;
            for campus_id in &extra {
                associate(tx, stamp.id, *campus_id)?;
            }
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

    /// Buildings available to one campus, by name, each with every campus it serves.
    ///
    /// "Available to" rather than "owned by": a building shared to this campus appears
    /// here, as the same row with the same identifier it has on its home campus. That is
    /// what makes one building show up in several campus listings without being
    /// duplicated.
    ///
    /// Two queries rather than one per row — the second reads the associations of every
    /// building in the result in a single pass.
    ///
    /// # Errors
    ///
    /// Propagates any database failure, or [`DbError::CorruptIdentifier`] for an
    /// unreadable stored identifier.
    pub fn buildings(&self, campus_id: Uuid) -> Result<Vec<BuildingWithCampuses>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {BUILDING_COLUMNS_QUALIFIED} FROM building b
                 JOIN building_campus bc ON bc.building_id = b.id
                 WHERE bc.campus_id = ?
                 ORDER BY b.name, b.id"
            ))?;
            let buildings = stmt
                .query_map([text(campus_id)], Building::from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            // Ordered by campus name so what the user reads never varies, and by building
            // first so the grouping below is a single pass.
            let mut stmt = conn.prepare(
                "SELECT bc.building_id, bc.campus_id
                 FROM building_campus bc
                 JOIN campus c ON c.id = bc.campus_id
                 WHERE bc.building_id IN (
                     SELECT building_id FROM building_campus WHERE campus_id = ?
                 )
                 ORDER BY bc.building_id, c.name, c.id",
            )?;
            let pairs = stmt
                .query_map([text(campus_id)], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut shares: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
            for (building_id, campus) in pairs {
                let building_id = parse_id_text("building_campus.building_id", &building_id)?;
                let campus = parse_id_text("building_campus.campus_id", &campus)?;
                shares.entry(building_id).or_default().push(campus);
            }

            Ok(buildings
                .into_iter()
                .map(|building| {
                    let campus_ids = shares.remove(&building.id).unwrap_or_default();
                    BuildingWithCampuses {
                        building,
                        campus_ids,
                    }
                })
                .collect())
        })
    }

    /// Every campus one building serves, by campus name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure, or [`DbError::CorruptIdentifier`].
    pub fn building_campuses(&self, building_id: Uuid) -> Result<Vec<Uuid>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT bc.campus_id FROM building_campus bc
                 JOIN campus c ON c.id = bc.campus_id
                 WHERE bc.building_id = ? ORDER BY c.name, c.id",
            )?;
            let raw = stmt
                .query_map([text(building_id)], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            raw.iter()
                .map(|value| parse_id_text("building_campus.campus_id", value))
                .collect()
        })
    }

    /// Renames a building. Its home campus never changes.
    ///
    /// The new name must be free on every campus the building serves, not only its home
    /// one: `UNIQUE (campus_id, name)` can see only the home campus, and a rename that
    /// collided on a shared campus would put two identically named buildings in one
    /// listing.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] or [`DbError::DuplicateValue`].
    pub fn update_building(&mut self, id: Uuid, input: &BuildingInput) -> Result<Building> {
        self.db.write(|tx| {
            for campus_id in associated_campuses(tx, id)? {
                require_name_free(tx, campus_id, &input.name, Some(id))?;
            }
            update(tx, "building", "building", id, "name = ?", &[&input.name])
        })?;
        self.building(id)?
            .ok_or(DbError::NotFound { entity: "building" })
    }

    /// Shares an existing building with another campus.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if the building or the campus does not exist.
    /// - [`DbError::Invalid`] if that campus already has it. Associating a campus twice is
    ///   refused rather than quietly ignored: reaching this point means the interface's
    ///   idea of the share set is stale, and it should re-read rather than believe a
    ///   success it did not cause.
    /// - [`DbError::DuplicateValue`] if that campus already has a different building of
    ///   the same name.
    pub fn share_building(&mut self, building_id: Uuid, campus_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            let name = building_name(tx, building_id)?;
            let campus = campus_name(tx, campus_id)?;

            if associated_campuses(tx, building_id)?.contains(&campus_id) {
                return Err(DbError::Invalid {
                    message: format!("{campus} already has this building."),
                });
            }

            require_name_free(tx, campus_id, &name, Some(building_id))?;
            associate(tx, building_id, campus_id)
        })
    }

    /// Stops sharing a building with a campus.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if that campus does not share this building.
    /// - [`DbError::Invalid`] if it is the building's home campus, or if rooms on that
    ///   campus are still in the building. Neither may proceed: the first would leave the
    ///   building with no campus and no screen it appears on, the second would leave rooms
    ///   pointing at a building their own campus no longer has.
    pub fn unshare_building(&mut self, building_id: Uuid, campus_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            let home = building_home_campus(tx, building_id)?;
            if home == campus_id {
                let campus = campus_name(tx, campus_id)?;
                return Err(DbError::Invalid {
                    message: format!(
                        "This building was created on {campus}, so {campus} cannot be \
                         removed from it. Delete the building instead, or remove the other \
                         campuses."
                    ),
                });
            }

            let Some(association_id) = association(tx, building_id, campus_id)? else {
                return Err(DbError::NotFound {
                    entity: "building share",
                });
            };

            // Rooms are not silently detached here. Detaching on a *delete* is right — the
            // building is going away and the room has to live somewhere — but a share
            // being withdrawn is a reversible administrative change, and quietly emptying
            // a campus's rooms out of their building is not what the user asked for.
            let rooms = rooms_in_building_on_campus(tx, building_id, campus_id)?;
            if rooms > 0 {
                let campus = campus_name(tx, campus_id)?;
                let name = building_name(tx, building_id)?;
                let plural = if rooms == 1 { "room is" } else { "rooms are" };
                return Err(DbError::Invalid {
                    message: format!(
                        "{rooms} {plural} still in {name} on {campus}. Move them to another \
                         building, clear their building, or stop sharing them with \
                         {campus}, before removing this campus."
                    ),
                });
            }

            delete(tx, "building", "building_campus", association_id)
        })
    }

    /// Deletes a building, detaching any rooms in it first.
    ///
    /// The rooms survive — a room is an asset, its building is metadata. They are detached
    /// by a proper update so each one's `rev` advances and the change feed stays truthful;
    /// the schema's `ON DELETE SET NULL` is only a backstop for writes that bypass this.
    ///
    /// Its campus associations go the same way: removed explicitly through
    /// [`super::delete`], one change-log entry each, rather than by a `CASCADE` SQLite
    /// would apply without firing the trigger.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such building.
    pub fn delete_building(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            detach_rooms(tx, "building_id", id)?;
            for association_id in associations_of(tx, id)? {
                delete(tx, "building", "building_campus", association_id)?;
            }
            delete(tx, "building", "building", id)
        })
    }
}

/// The entries of `requested` that are not `home`, deduplicated, in the order given.
///
/// A multi-select can report the same campus twice and can include the campus the user is
/// already on. Neither is worth an error.
fn others(requested: &[Uuid], home: Uuid) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    requested
        .iter()
        .copied()
        .filter(|campus_id| *campus_id != home && seen.insert(*campus_id))
        .collect()
}

/// Writes one association row.
fn associate(tx: &Transaction<'_>, building_id: Uuid, campus_id: Uuid) -> Result<()> {
    let stamp = Stamp::new();
    tx.execute(
        "INSERT INTO building_campus
             (id, building_id, campus_id, created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?)",
        (
            text(stamp.id),
            text(building_id),
            text(campus_id),
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ),
    )
    .map_err(|error| translate(error, "building"))?;
    Ok(())
}

/// Identifier of one association, or `None` when that campus does not share the building.
fn association(tx: &Transaction<'_>, building_id: Uuid, campus_id: Uuid) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM building_campus WHERE building_id = ? AND campus_id = ?",
            [text(building_id), text(campus_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text("building_campus.id", value))
        .transpose()
}

/// Identifiers of every association of one building.
fn associations_of(tx: &Transaction<'_>, building_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt =
        tx.prepare("SELECT id FROM building_campus WHERE building_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(building_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("building_campus.id", value))
        .collect()
}

/// Every campus one building serves.
fn associated_campuses(tx: &Transaction<'_>, building_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare(
        "SELECT campus_id FROM building_campus WHERE building_id = ? ORDER BY campus_id",
    )?;
    let raw = stmt
        .query_map([text(building_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("building_campus.campus_id", value))
        .collect()
}

/// How many rooms available to one campus are in one building.
///
/// "Available to", not "homed on": since V6 a room may serve a campus it was not created
/// on, and such a room depends on the building share just as much as a local one does.
/// Counting only `room.campus_id` here would let a building be withdrawn from a campus
/// that is still listing shared rooms inside it — the invalid state ADR 0014 calls out,
/// reached from the building side.
///
/// `room_campus` contains the home campus too, so this subsumes the V5 check rather than
/// sitting beside it.
fn rooms_in_building_on_campus(
    tx: &Transaction<'_>,
    building_id: Uuid,
    campus_id: Uuid,
) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM room r
         JOIN room_campus rc ON rc.room_id = r.id
         WHERE r.building_id = ? AND rc.campus_id = ?",
        [text(building_id), text(campus_id)],
        |row| row.get(0),
    )?)
}

/// The building's name, or [`DbError::NotFound`].
fn building_name(tx: &Transaction<'_>, building_id: Uuid) -> Result<String> {
    tx.query_row(
        "SELECT name FROM building WHERE id = ?",
        [text(building_id)],
        |row| row.get(0),
    )
    .optional()?
    .ok_or(DbError::NotFound { entity: "building" })
}

/// The building's home campus, or [`DbError::NotFound`].
fn building_home_campus(tx: &Transaction<'_>, building_id: Uuid) -> Result<Uuid> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT campus_id FROM building WHERE id = ?",
            [text(building_id)],
            |row| row.get(0),
        )
        .optional()?;
    let raw = raw.ok_or(DbError::NotFound { entity: "building" })?;
    parse_id_text("building.campus_id", &raw)
}

/// The campus's name, or [`DbError::NotFound`].
fn campus_name(tx: &Transaction<'_>, campus_id: Uuid) -> Result<String> {
    tx.query_row(
        "SELECT name FROM campus WHERE id = ?",
        [text(campus_id)],
        |row| row.get(0),
    )
    .optional()?
    .ok_or(DbError::NotFound { entity: "campus" })
}

/// Checks a campus exists, so a missing one reports itself rather than arriving as a
/// foreign-key failure that reads as "still in use".
fn require_campus(tx: &Transaction<'_>, campus_id: Uuid) -> Result<()> {
    campus_name(tx, campus_id).map(|_| ())
}

/// Refuses a name already used by a *different* building available to that campus.
///
/// `UNIQUE (campus_id, name)` scopes only a building's home campus, so it cannot see this
/// case. The rule needs the other rows, which puts it here in Rust rather than in the
/// frontend's validators.
fn require_name_free(
    tx: &Transaction<'_>,
    campus_id: Uuid,
    name: &str,
    except: Option<Uuid>,
) -> Result<()> {
    let taken: i64 = tx.query_row(
        "SELECT COUNT(*) FROM building b
         JOIN building_campus bc ON bc.building_id = b.id
         WHERE bc.campus_id = ? AND b.name = ? AND b.id <> ?",
        [
            text(campus_id),
            name.to_owned(),
            except.map(text).unwrap_or_default(),
        ],
        |row| row.get(0),
    )?;

    if taken > 0 {
        return Err(DbError::DuplicateValue {
            entity: "building",
            field: "name",
        });
    }
    Ok(())
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

/// The same columns, qualified, for the joins through `room_campus`.
const ROOM_COLUMNS_QUALIFIED: &str = "r.id, r.campus_id, r.building_id, r.room_type_id, r.name, r.code, r.capacity, \
     r.is_bookable, r.notes, r.created_at, r.updated_at, r.rev";

/// A room together with every campus it serves.
///
/// Returned by [`Structure::rooms`] rather than a bare [`Room`] so a listing can mark a
/// shared room without a follow-up question per row.
///
/// `campus_ids` comes from `room_campus` and is complete: it includes `room.campus_id`,
/// the home campus. Nothing reads that column to answer "which campuses?" — see
/// [ADR 0014](../../../docs/adr/0014-rooms-shared-across-campuses.md).
///
/// The room inside is **one** room. Two campuses listing it are listing the same
/// `room.id`, which is what makes its occupancy a single fact rather than one per campus.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RoomWithCampuses {
    /// The room itself. Its identity is unaffected by sharing.
    pub room: Room,
    /// Every campus this room serves, by campus name. Never empty.
    pub campus_ids: Vec<Uuid>,
}

impl Structure<'_> {
    /// Creates a room on its home campus, optionally shared with others.
    ///
    /// One transaction: the room, its home association, and one association per entry in
    /// `shared_with`. A partially shared room is never persisted, so a user cannot end up
    /// with a room that exists but is missing half the campuses they asked for.
    ///
    /// Duplicates within `shared_with`, and the home campus appearing there, are ignored
    /// rather than rejected — the caller is a multi-select, and "you picked the campus you
    /// are already on" is not a mistake worth a round trip.
    ///
    /// # Errors
    ///
    /// - [`DbError::DuplicateValue`] if the code is taken on the home campus or on any
    ///   campus it is being shared to.
    /// - [`DbError::NotFound`] if a named campus does not exist.
    /// - [`DbError::Invalid`] if the building named is not available to the home campus or
    ///   to one of the campuses it is being shared to.
    pub fn create_room(&mut self, input: &RoomInput, shared_with: &[Uuid]) -> Result<Room> {
        let stamp = Stamp::new();
        let extra = others(shared_with, input.campus_id);

        self.db.write(|tx| {
            require_campus(tx, input.campus_id)?;
            for campus_id in &extra {
                require_campus(tx, *campus_id)?;
                require_room_code_free(tx, *campus_id, &input.code, None)?;
                // Checked before the room exists, so the message can name the building
                // the caller asked for rather than reading it back off the row.
                if let Some(building_id) = input.building_id {
                    require_building_available(tx, building_id, *campus_id, &input.name)?;
                }
            }

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

            // The home campus is an association like any other. Writing it here rather
            // than leaving `room.campus_id` to imply it is what lets every later query
            // read one table and get the whole answer.
            associate_room(tx, stamp.id, input.campus_id)?;
            for campus_id in &extra {
                associate_room(tx, stamp.id, *campus_id)?;
            }
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

    /// Rooms available to one campus, by name, each with every campus it serves.
    ///
    /// "Available to" rather than "on": a room shared with this campus appears here, as
    /// the same row with the same identifier it has on its home campus. That is what
    /// makes one room show up in several campus listings without being duplicated — and
    /// what will make its occupancy one fact rather than one per campus when scheduling
    /// arrives.
    ///
    /// Two queries rather than one per row — the second reads the associations of every
    /// room in the result in a single pass.
    ///
    /// # Errors
    ///
    /// Propagates any database failure, or [`DbError::CorruptIdentifier`] for an
    /// unreadable stored identifier.
    pub fn rooms(&self, campus_id: Uuid) -> Result<Vec<RoomWithCampuses>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROOM_COLUMNS_QUALIFIED} FROM room r
                 JOIN room_campus rc ON rc.room_id = r.id
                 WHERE rc.campus_id = ?
                 ORDER BY r.name, r.id"
            ))?;
            let rooms = stmt
                .query_map([text(campus_id)], Room::from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            // Ordered by campus name so what the user reads never varies, and by room
            // first so the grouping below is a single pass.
            let mut stmt = conn.prepare(
                "SELECT rc.room_id, rc.campus_id
                 FROM room_campus rc
                 JOIN campus c ON c.id = rc.campus_id
                 WHERE rc.room_id IN (
                     SELECT room_id FROM room_campus WHERE campus_id = ?
                 )
                 ORDER BY rc.room_id, c.name, c.id",
            )?;
            let pairs = stmt
                .query_map([text(campus_id)], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut shares: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
            for (room_id, campus) in pairs {
                let room_id = parse_id_text("room_campus.room_id", &room_id)?;
                let campus = parse_id_text("room_campus.campus_id", &campus)?;
                shares.entry(room_id).or_default().push(campus);
            }

            Ok(rooms
                .into_iter()
                .map(|room| {
                    let campus_ids = shares.remove(&room.id).unwrap_or_default();
                    RoomWithCampuses { room, campus_ids }
                })
                .collect())
        })
    }

    /// Every campus one room serves, by campus name.
    ///
    /// # Errors
    ///
    /// Propagates any database failure, or [`DbError::CorruptIdentifier`].
    pub fn room_campuses(&self, room_id: Uuid) -> Result<Vec<Uuid>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT rc.campus_id FROM room_campus rc
                 JOIN campus c ON c.id = rc.campus_id
                 WHERE rc.room_id = ? ORDER BY c.name, c.id",
            )?;
            let raw = stmt
                .query_map([text(room_id)], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            raw.iter()
                .map(|value| parse_id_text("room_campus.campus_id", value))
                .collect()
        })
    }

    /// Replaces a room's details. Its home campus never changes.
    ///
    /// The code must be free on every campus the room serves, not only its home one:
    /// `UNIQUE (campus_id, code)` can see only the home campus, and a code that collided
    /// on a shared campus would put two identically coded rooms in one listing.
    ///
    /// A change of building must hold for every campus the room serves too. Both are
    /// checked here, before the write, so the message can name the campus and the
    /// building; the schema's triggers are the backstop for a write that bypasses this.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`], [`DbError::DuplicateValue`], or [`DbError::Invalid`] if the
    /// building is not available to a campus the room serves.
    pub fn update_room(&mut self, id: Uuid, input: &RoomInput) -> Result<Room> {
        self.db.write(|tx| {
            for campus_id in room_campuses_in(tx, id)? {
                require_room_code_free(tx, campus_id, &input.code, Some(id))?;
                if let Some(building_id) = input.building_id {
                    require_building_available(tx, building_id, campus_id, &input.name)?;
                }
            }

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

    /// Shares an existing room with another campus.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if the room or the campus does not exist.
    /// - [`DbError::Invalid`] if that campus already has it. Associating a campus twice is
    ///   refused rather than quietly ignored: reaching this point means the interface's
    ///   idea of the share set is stale, and it should re-read rather than believe a
    ///   success it did not cause.
    /// - [`DbError::Invalid`] if the room is in a building that campus does not have.
    /// - [`DbError::DuplicateValue`] if that campus already has a different room with the
    ///   same code.
    pub fn share_room(&mut self, room_id: Uuid, campus_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            let (name, code, building_id) = room_identity(tx, room_id)?;
            let campus = campus_name(tx, campus_id)?;

            if room_campuses_in(tx, room_id)?.contains(&campus_id) {
                return Err(DbError::Invalid {
                    message: format!("{campus} already has this room."),
                });
            }

            require_room_code_free(tx, campus_id, &code, Some(room_id))?;
            if let Some(building_id) = building_id {
                require_building_available(tx, building_id, campus_id, &name)?;
            }
            associate_room(tx, room_id, campus_id)
        })
    }

    /// Stops sharing a room with a campus.
    ///
    /// Nothing is moved, duplicated or deleted: the room keeps its identity and every
    /// other campus keeps it. Only this campus stops listing it.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if that campus does not share this room.
    /// - [`DbError::Invalid`] if it is the room's home campus. Withdrawing that would
    ///   leave the room with no campus and no screen it appears on.
    pub fn unshare_room(&mut self, room_id: Uuid, campus_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            let home = room_home_campus(tx, room_id)?;
            if home == campus_id {
                let campus = campus_name(tx, campus_id)?;
                return Err(DbError::Invalid {
                    message: format!(
                        "This room was created on {campus}, so {campus} cannot be removed \
                         from it. Delete the room instead, or remove the other campuses."
                    ),
                });
            }

            let Some(association_id) = room_association(tx, room_id, campus_id)? else {
                return Err(DbError::NotFound {
                    entity: "room share",
                });
            };

            // Nothing depends on a room-campus association yet. When activities arrive
            // they will: an activity scheduled into this room on behalf of a group on
            // this campus is exactly the dependency that has to block a withdrawal here,
            // the way rooms block `unshare_building`. Recorded in ADR 0014 rather than
            // guessed at now, because the shape of that reference is a later phase's.
            delete(tx, "room", "room_campus", association_id)
        })
    }

    /// Deletes a room.
    ///
    /// Its campus associations are removed explicitly through [`super::delete`], one
    /// change-log entry each, rather than by a `CASCADE` SQLite would apply without
    /// firing the trigger.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if there is no such room.
    pub fn delete_room(&mut self, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            for association_id in room_associations_of(tx, id)? {
                delete(tx, "room", "room_campus", association_id)?;
            }
            delete(tx, "room", "room", id)
        })
    }
}

/// Writes one room-campus association row.
fn associate_room(tx: &Transaction<'_>, room_id: Uuid, campus_id: Uuid) -> Result<()> {
    let stamp = Stamp::new();
    tx.execute(
        "INSERT INTO room_campus (id, room_id, campus_id, created_at, updated_at, rev)
         VALUES (?, ?, ?, ?, ?, ?)",
        (
            text(stamp.id),
            text(room_id),
            text(campus_id),
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ),
    )
    .map_err(room_error)?;
    Ok(())
}

/// Identifier of one association, or `None` when that campus does not share the room.
fn room_association(tx: &Transaction<'_>, room_id: Uuid, campus_id: Uuid) -> Result<Option<Uuid>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT id FROM room_campus WHERE room_id = ? AND campus_id = ?",
            [text(room_id), text(campus_id)],
            |row| row.get(0),
        )
        .optional()?;
    found
        .as_deref()
        .map(|value| parse_id_text("room_campus.id", value))
        .transpose()
}

/// Identifiers of every association of one room.
fn room_associations_of(tx: &Transaction<'_>, room_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = tx.prepare("SELECT id FROM room_campus WHERE room_id = ? ORDER BY id")?;
    let raw = stmt
        .query_map([text(room_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("room_campus.id", value))
        .collect()
}

/// Every campus one room serves, inside a transaction.
fn room_campuses_in(tx: &Transaction<'_>, room_id: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt =
        tx.prepare("SELECT campus_id FROM room_campus WHERE room_id = ? ORDER BY campus_id")?;
    let raw = stmt
        .query_map([text(room_id)], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    raw.iter()
        .map(|value| parse_id_text("room_campus.campus_id", value))
        .collect()
}

/// The room's name, code and building, or [`DbError::NotFound`].
fn room_identity(tx: &Transaction<'_>, room_id: Uuid) -> Result<(String, String, Option<Uuid>)> {
    let found: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT name, code, building_id FROM room WHERE id = ?",
            [text(room_id)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (name, code, building) = found.ok_or(DbError::NotFound { entity: "room" })?;
    let building = building
        .as_deref()
        .map(|value| parse_id_text("room.building_id", value))
        .transpose()?;
    Ok((name, code, building))
}

/// The room's home campus, or [`DbError::NotFound`].
fn room_home_campus(tx: &Transaction<'_>, room_id: Uuid) -> Result<Uuid> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT campus_id FROM room WHERE id = ?",
            [text(room_id)],
            |row| row.get(0),
        )
        .optional()?;
    let raw = raw.ok_or(DbError::NotFound { entity: "room" })?;
    parse_id_text("room.campus_id", &raw)
}

/// Refuses a code already used by a *different* room available to that campus.
///
/// `UNIQUE (campus_id, code)` scopes only a room's home campus, so it cannot see this
/// case. The rule needs the other rows, which puts it here in Rust rather than in the
/// frontend's validators.
fn require_room_code_free(
    tx: &Transaction<'_>,
    campus_id: Uuid,
    code: &str,
    except: Option<Uuid>,
) -> Result<()> {
    let taken: i64 = tx.query_row(
        "SELECT COUNT(*) FROM room r
         JOIN room_campus rc ON rc.room_id = r.id
         WHERE rc.campus_id = ? AND r.code = ? AND r.id <> ?",
        [
            text(campus_id),
            code.to_owned(),
            except.map(text).unwrap_or_default(),
        ],
        |row| row.get(0),
    )?;

    if taken > 0 {
        return Err(DbError::DuplicateValue {
            entity: "room",
            field: "code",
        });
    }
    Ok(())
}

/// Refuses a room-campus pairing whose building does not reach that campus.
///
/// A shared building does not imply shared rooms and a shared room does not imply a
/// shared building; this is the one point where the two settings have to agree. The
/// schema's triggers enforce the same rule, but with one fixed message for every route
/// into it. This runs first so the user is told which building and which campus, and what
/// to do about it.
fn require_building_available(
    tx: &Transaction<'_>,
    building_id: Uuid,
    campus_id: Uuid,
    room_name: &str,
) -> Result<()> {
    let available: i64 = tx.query_row(
        "SELECT COUNT(*) FROM building_campus WHERE building_id = ? AND campus_id = ?",
        [text(building_id), text(campus_id)],
        |row| row.get(0),
    )?;
    if available > 0 {
        return Ok(());
    }

    let building = building_name(tx, building_id)?;
    let campus = campus_name(tx, campus_id)?;
    Err(DbError::Invalid {
        message: format!(
            "{room_name} is in {building}, which {campus} does not have. Share {building} \
             with {campus} first, or clear the room's building."
        ),
    })
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
