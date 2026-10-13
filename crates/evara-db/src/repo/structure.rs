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

use evara_core::branding::{LogoFormat, LogoImage};
use evara_core::calendar::WorkingDays;
use rusqlite::types::ToSql;
use rusqlite::{OptionalExtension, Transaction};
use uuid::Uuid;

use super::availability::{AvailabilityEntity, clear_for_entity};
use super::row::{entity, opt_text, parse_id_text, text};
use super::{Stamp, delete, require_same_school, require_school_owner, translate, update};
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
    /// One of the schools this project describes.
    ///
    /// A project held exactly one of these until V12. It can now hold several, each owning
    /// its own campuses, academic years, year levels, cycles and bell schedules, while
    /// facilities are shared between them through the campus associations of ADR 0013 and
    /// ADR 0014. See [ADR 0019](../../../docs/adr/0019-multiple-schools-in-one-project.md).
    School,
    /// Values for creating or replacing a school record.
    SchoolInput,
    {
        /// Display name, unique within the project.
        name: String,
        /// Optional short code. Unique within the project when given.
        code: Option<String>,
        /// IANA timezone, e.g. `Europe/London`.
        timezone: String,
        /// BCP-47 locale, e.g. `en-GB`.
        locale: String,
        /// Which weekdays the school normally teaches on. Never empty.
        ///
        /// Stored as a seven-bit set and carried over IPC as ascending ISO weekday
        /// numbers, `[1, 2, 3, 4, 5]` for Monday to Friday. A *calendar* default and an
        /// advisory, never a scheduling identity: nothing derives a cycle day from it and
        /// nothing infers it from `cycle.day_count`. See
        /// [`evara_core::calendar::WorkingDays`].
        working_days: WorkingDays,
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

const SCHOOL_COLUMNS: &str =
    "id, name, code, timezone, locale, working_days, created_at, updated_at, rev";

impl Structure<'_> {
    /// Creates a school.
    ///
    /// A project may hold several since V12. Each is created empty and configured
    /// independently; nothing is copied from a school that already exists.
    ///
    /// # Errors
    ///
    /// [`DbError::DuplicateValue`] if the name, or a given code, is already used by another
    /// school in this project.
    pub fn create_school(&mut self, input: &SchoolInput) -> Result<School> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO school
                     (id, name, code, timezone, locale, working_days, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    text(stamp.id),
                    &input.name,
                    &input.code,
                    &input.timezone,
                    &input.locale,
                    // Bits on the way in, the newtype everywhere else. The column's
                    // CHECK refuses an empty week, so this cannot write one even if a
                    // caller somehow built one.
                    input.working_days.bits(),
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
            code: input.code.clone(),
            timezone: input.timezone.clone(),
            locale: input.locale.clone(),
            working_days: input.working_days,
            created_at: stamp.created_at,
            updated_at: stamp.updated_at,
            rev: stamp.rev,
        })
    }

    /// Every school in the project, ordered by name then identifier.
    ///
    /// Empty for a project where nothing has been set up yet. There is no "the school"
    /// accessor: a `LIMIT 1` read would quietly pick one of several and every row it then
    /// produced would belong to an arbitrary institution, which is the single most
    /// damaging thing this change could leave behind.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn schools(&self) -> Result<Vec<School>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SCHOOL_COLUMNS} FROM school ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([], School::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// One school by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn school(&self, id: Uuid) -> Result<Option<School>> {
        self.db.read(|conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {SCHOOL_COLUMNS} FROM school WHERE id = ?"))?;
            let mut rows = stmt.query_map([text(id)], School::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Replaces a school's details.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if no school has that identifier;
    /// [`DbError::DuplicateValue`] if the name or code is taken by another school.
    pub fn update_school(&mut self, id: Uuid, input: &SchoolInput) -> Result<School> {
        self.db.write(|tx| {
            update(
                tx,
                "school",
                "school",
                id,
                "name = ?, code = ?, timezone = ?, locale = ?, working_days = ?",
                &[
                    &input.name,
                    &input.code,
                    &input.timezone,
                    &input.locale,
                    &input.working_days.bits(),
                ],
            )
        })?;
        self.require_school(id)
    }

    /// What a school still owns, so the interface can say why it cannot be deleted.
    ///
    /// Read *before* offering the action, for the reason `student_group_member_count`
    /// exists: a refusal that arrives after the user has pressed Delete explains nothing,
    /// and the user has already decided by then.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn school_dependents(&self, school_id: Uuid) -> Result<SchoolDependents> {
        self.db.read(|conn| {
            let id = text(school_id);
            let count = |sql: &str| -> Result<i64> {
                conn.query_row(sql, [&id], |row| row.get(0))
                    .map_err(DbError::from)
            };
            let campuses = count("SELECT COUNT(*) FROM campus WHERE school_id = ?")?;
            let room_types = count("SELECT COUNT(*) FROM room_type WHERE school_id = ?")?;
            let resources = count("SELECT COUNT(*) FROM resource WHERE school_id = ?")?;
            let academic_years = count("SELECT COUNT(*) FROM academic_year WHERE school_id = ?")?;
            let cycles = count("SELECT COUNT(*) FROM cycle WHERE school_id = ?")?;
            let period_structures =
                count("SELECT COUNT(*) FROM period_structure WHERE school_id = ?")?;
            let timeslots = count("SELECT COUNT(*) FROM timeslot WHERE school_id = ?")?;
            let calendar_days = count("SELECT COUNT(*) FROM calendar_day WHERE school_id = ?")?;
            let year_levels = count("SELECT COUNT(*) FROM year_level WHERE school_id = ?")?;
            let teachers = count("SELECT COUNT(*) FROM teacher WHERE school_id = ?")?;
            let students = count("SELECT COUNT(*) FROM student WHERE school_id = ?")?;
            let student_groups = count("SELECT COUNT(*) FROM student_group WHERE school_id = ?")?;
            let availability_rules =
                count("SELECT COUNT(*) FROM availability WHERE school_id = ?")?;

            Ok(SchoolDependents {
                total: campuses
                    + room_types
                    + resources
                    + academic_years
                    + cycles
                    + period_structures
                    + timeslots
                    + calendar_days
                    + year_levels
                    + teachers
                    + students
                    + student_groups
                    + availability_rules,
                campuses,
                room_types,
                resources,
                academic_years,
                cycles,
                period_structures,
                timeslots,
                calendar_days,
                year_levels,
                teachers,
                students,
                student_groups,
                availability_rules,
            })
        })
    }

    /// Deletes a school that owns nothing.
    ///
    /// # What this deliberately does not do
    ///
    /// There is **no** "delete this school and everything in it". Every school-owned table
    /// references `school (id)` with `ON DELETE RESTRICT`, and that is left exactly as
    /// ADR 0010 set it: a school is years of configuration, and one mis-click must not be
    /// able to take a campus, a timetable grid and a staff list with it. Emptying a school
    /// is the user's work, step by step, with a confirmation at each one.
    ///
    /// The single exception is the school's own **logo**, which is removed in the same
    /// transaction. A logo is part of the school record rather than something the school
    /// owns -- it is a picture of it -- and leaving it to block the delete would mean a
    /// school that had ever been given one could not be removed at all without the user
    /// first guessing that a decoration was in the way.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if no school has that identifier.
    /// - [`DbError::StillReferenced`] if it still owns anything. Call
    ///   [`Structure::school_dependents`] first to say what.
    pub fn delete_school(&mut self, id: Uuid) -> Result<()> {
        if self.school(id)?.is_none() {
            return Err(DbError::NotFound { entity: "school" });
        }
        let dependents = self.school_dependents(id)?;
        if dependents.total > 0 {
            // Reported before the attempt, so the message is about the school rather than
            // about whichever foreign key SQLite happened to check first.
            return Err(DbError::StillReferenced { entity: "school" });
        }

        self.db.write(|tx| {
            let logo: Option<String> = tx
                .query_row(
                    "SELECT id FROM school_logo WHERE school_id = ?",
                    [text(id)],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(logo_id) = logo {
                let logo_id = parse_id_text("school_logo.id", &logo_id)?;
                delete(tx, "school logo", "school_logo", logo_id)?;
            }
            delete(tx, "school", "school", id)
        })
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

/// How much a school still owns, counted per kind.
///
/// Every field counts rows that reference the school and would block its deletion. `total`
/// is their sum, so a caller can ask "can this be deleted?" without adding up thirteen
/// numbers, and still show the user which ones are in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SchoolDependents {
    /// Sum of every other field. Zero means the school can be deleted.
    pub total: i64,
    /// Campuses owned by the school.
    pub campuses: i64,
    /// Room types owned by the school.
    pub room_types: i64,
    /// Resources owned by the school.
    pub resources: i64,
    /// Academic years owned by the school.
    pub academic_years: i64,
    /// Cycles owned by the school.
    pub cycles: i64,
    /// Bell schedules owned by the school.
    pub period_structures: i64,
    /// Materialised timeslots owned by the school.
    pub timeslots: i64,
    /// Calendar mappings owned by the school.
    pub calendar_days: i64,
    /// Year levels owned by the school.
    pub year_levels: i64,
    /// Teachers owned by the school.
    pub teachers: i64,
    /// Students owned by the school.
    pub students: i64,
    /// Student groups owned by the school.
    pub student_groups: i64,
    /// Availability rules recorded against the school.
    pub availability_rules: i64,
}

// ------------------------------------------------------------------------ school logo

const SCHOOL_LOGO_COLUMNS: &str =
    "id, school_id, media_type, bytes, width, height, created_at, updated_at, rev";

entity! {
    /// A school's own logo, as stored in the project.
    ///
    /// Absent means the school has no custom logo and the interface shows Evara's own --
    /// see [`Structure::school_logo`]. There is no "disabled" state and no second row to
    /// choose between: `UNIQUE (school_id)` makes this strictly one per school.
    SchoolLogo,
    /// Values for setting a school's logo. Validated before anything is written.
    SchoolLogoInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// The format, read from the file's own header rather than any filename.
        media_type: String,
        /// The image, byte for byte as uploaded.
        ///
        /// Stored unaltered on purpose. Re-encoding would need a decoder, would lose
        /// transparency or sharpness depending on what it chose, and would mean the file a
        /// user gets back out of an export is not the file they put in.
        bytes: Vec<u8>,
        /// Width in pixels, measured from the header at upload.
        width: i64,
        /// Height in pixels, measured from the header at upload.
        height: i64,
    }
}

impl Structure<'_> {
    /// The school's logo, or `None` when it uses Evara's.
    ///
    /// **`None` is the ordinary case, not a failure.** A new project has no row here, and
    /// neither does a project upgraded from a schema older than V8. Both mean the same
    /// thing, and the caller falls back to the Evara logo.
    ///
    /// # Errors
    ///
    /// Propagates a read failure.
    pub fn school_logo(&self, school_id: Uuid) -> Result<Option<SchoolLogo>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SCHOOL_LOGO_COLUMNS} FROM school_logo WHERE school_id = ?"
            ))?;
            let mut rows = stmt.query_map([text(school_id)], SchoolLogo::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Sets or replaces the school's logo.
    ///
    /// The bytes are inspected first by [`LogoImage::inspect`], which decides the format
    /// from the content and measures it against the product's limits. Nothing a caller
    /// says about the file is believed: there is no filename parameter, and a declared
    /// media type would be another thing to disbelieve.
    ///
    /// Replacing is an update of the existing row rather than a delete and re-insert, so
    /// the logo keeps its identity and its `rev` tells the truth about how many times it
    /// has changed.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] if the bytes are not an acceptable image, carrying the
    /// specific reason. [`DbError::NotFound`] if no school exists yet.
    pub fn set_school_logo(&mut self, school_id: Uuid, bytes: &[u8]) -> Result<SchoolLogo> {
        let image = LogoImage::inspect(bytes).map_err(|error| DbError::Invalid {
            message: error.to_string(),
        })?;
        let media_type = image.format.media_type().to_owned();
        let width = i64::from(image.width);
        let height = i64::from(image.height);

        let existing = self.db.read(|conn| {
            conn.query_row(
                "SELECT id FROM school_logo WHERE school_id = ?",
                [text(school_id)],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(DbError::from)
        })?;

        // Replacing is an update so the row keeps its identity and its `rev` stays
        // truthful; a first upload is an insert.
        if let Some(raw) = existing {
            let id = parse_id_text("school_logo.id", &raw)?;
            self.db.write(|tx| {
                update(
                    tx,
                    "school logo",
                    "school_logo",
                    id,
                    "media_type = ?, bytes = ?, width = ?, height = ?",
                    &[&media_type, &bytes, &width, &height],
                )?;
                Ok(())
            })?;
        } else {
            let stamp = Stamp::new();
            self.db.write(|tx| {
                tx.execute(
                    "INSERT INTO school_logo
                         (id, school_id, media_type, bytes, width, height,
                          created_at, updated_at, rev)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        text(stamp.id),
                        text(school_id),
                        &media_type,
                        bytes,
                        width,
                        height,
                        &stamp.created_at,
                        &stamp.updated_at,
                        stamp.rev,
                    ),
                )
                .map_err(|error| translate(error, "school logo"))?;
                Ok(())
            })?;
        }

        self.school_logo(school_id)?.ok_or(DbError::NotFound {
            entity: "school logo",
        })
    }

    /// Removes the school's logo, restoring the Evara fallback.
    ///
    /// Removing one that is not there succeeds. "There is no custom logo" is the state the
    /// caller asked for, and it is already true -- reporting `NotFound` would make the
    /// interface guard a button for no reason.
    ///
    /// # Errors
    ///
    /// Propagates a write failure.
    pub fn clear_school_logo(&mut self, school_id: Uuid) -> Result<()> {
        let existing = self.db.read(|conn| {
            conn.query_row(
                "SELECT id FROM school_logo WHERE school_id = ?",
                [text(school_id)],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(DbError::from)
        })?;

        let Some(raw) = existing else {
            return Ok(());
        };
        let id = parse_id_text("school_logo.id", &raw)?;
        self.db
            .write(|tx| delete(tx, "school logo", "school_logo", id))
    }

    /// The format of a stored logo, or `None` if the project names one this build cannot read.
    ///
    /// A row written by a future build that supports more formats is reported as
    /// unreadable rather than guessed at or mislabelled, so the interface can fall back to
    /// the Evara logo instead of handing a WebView bytes it will not render.
    #[must_use]
    pub fn logo_format(logo: &SchoolLogo) -> Option<LogoFormat> {
        LogoFormat::from_media_type(&logo.media_type)
    }
}

/// Refuses destroying a facility that another school still uses.
///
/// # Why a shared facility needs more than an ownership check
///
/// ADR 0013 and ADR 0014 make a building and a room shareable **by campus**, and since
/// `V12` those campuses may belong to different schools. That is the point: one physical
/// sports hall, one occupancy record, two institutions timetabling into it.
///
/// It also means "the school that owns the home campus" is not sufficient authority to
/// *destroy* it. A hall created on Northgate's campus and shared with Southmoor's is a hall
/// Southmoor is timetabling into, and Northgate deleting it would take Southmoor's room
/// away without Southmoor being asked.
///
/// So three operations stay distinct:
///
/// | Operation | Who may | Effect |
/// | --- | --- | --- |
/// | Withdraw one school's access | that school, or the home school | one association row |
/// | Remove the home campus | nobody | refused; a facility must never reach zero campuses |
/// | Destroy the facility | the home school, **and only while nobody else shares it** | the row and all its associations |
///
/// The refusal is [`DbError::StillReferenced`] — the error a blocked delete already
/// produces, so the interface's existing handling applies. The way forward is to withdraw
/// the other school's access first, which is an operation that already exists and that the
/// other school can see.
fn require_unshared(
    tx: &Transaction<'_>,
    entity: &'static str,
    sharers_query: &str,
    id: Uuid,
    school_id: Uuid,
) -> Result<()> {
    let others: i64 = tx.query_row(sharers_query, [text(id), text(school_id)], |row| row.get(0))?;
    if others > 0 {
        return Err(DbError::StillReferenced { entity });
    }
    Ok(())
}

/// Refuses withdrawing an association that is none of the acting school's business.
///
/// Either authority will do, and both are legitimate:
///
/// - the school that owns **the campus being removed** — "we no longer use this room";
/// - the school that owns **the facility's home campus** — "we are no longer lending it".
///
/// A third school has no say in either. The home campus itself is refused separately and
/// for a different reason — a facility must never reach zero campuses and vanish from every
/// screen (ADR 0014) — so that check stays where it was, after this one.
fn require_may_unshare(
    tx: &Transaction<'_>,
    entity: &'static str,
    owner_query: &str,
    id: Uuid,
    campus_id: Uuid,
    school_id: Uuid,
) -> Result<()> {
    // Resolving the facility first also produces the NotFound for an identifier that is
    // not there, before anything else is considered.
    let owner: Option<String> = tx
        .query_row(owner_query, [text(id)], |row| row.get(0))
        .optional()?;
    let Some(owner) = owner else {
        return Err(DbError::NotFound { entity });
    };

    if owner == text(school_id) {
        return Ok(());
    }

    let campus_owner: Option<String> = tx
        .query_row(
            "SELECT school_id FROM campus WHERE id = ?",
            [text(campus_id)],
            |row| row.get(0),
        )
        .optional()?;
    match campus_owner {
        Some(raw) if raw == text(school_id) => Ok(()),
        _ => Err(DbError::WrongSchool { entity }),
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

    /// Every campus of one school, ordered by name then identifier so the order never
    /// varies.
    ///
    /// Scoped to a school since V12. Two schools may each have a "Main Campus", and a list
    /// that returned both would put another institution's site in this one's pickers.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn campuses(&self, school_id: Uuid) -> Result<Vec<Campus>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CAMPUS_COLUMNS} FROM campus WHERE school_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], Campus::from_row)?;
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
            require_same_school(tx, "campus", "campus", id, input.school_id)?;
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
    pub fn delete_campus(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "campus", "campus", id, school_id)?;
            // Availability is polymorphic and so has no foreign key to catch this. A rule
            // about a campus means nothing once the campus is gone.
            clear_for_entity(tx, AvailabilityEntity::Campus, id)?;
            delete(tx, "campus", "campus", id)
        })
    }
}

// -------------------------------------------------------------------------- building

/// A building belongs to the school that owns its home campus.
const BUILDING_OWNER: &str = "SELECT campus.school_id FROM building \
     JOIN campus ON campus.id = building.campus_id WHERE building.id = ?";

/// How many campuses of *other* schools (`?2` is the acting school) share `?1`.
const BUILDING_SHARERS: &str = "SELECT COUNT(*) FROM building_campus \
     JOIN campus ON campus.id = building_campus.campus_id \
     WHERE building_campus.building_id = ?1 AND campus.school_id <> ?2";

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
    pub fn unshare_building(
        &mut self,
        school_id: Uuid,
        building_id: Uuid,
        campus_id: Uuid,
    ) -> Result<()> {
        self.db.write(|tx| {
            require_may_unshare(
                tx,
                "building",
                BUILDING_OWNER,
                building_id,
                campus_id,
                school_id,
            )?;
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
    pub fn delete_building(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(tx, "building", BUILDING_OWNER, id, school_id)?;
            require_unshared(tx, "building", BUILDING_SHARERS, id, school_id)?;
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
    pub fn room_types(&self, school_id: Uuid) -> Result<Vec<RoomType>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ROOM_TYPE_COLUMNS} FROM room_type WHERE school_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], RoomType::from_row)?;
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
            require_same_school(tx, "room type", "room_type", id, input.school_id)?;
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
    pub fn delete_room_type(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "room type", "room_type", id, school_id)?;
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

/// A room belongs to the school that owns its home campus.
const ROOM_OWNER: &str = "SELECT campus.school_id FROM room \
     JOIN campus ON campus.id = room.campus_id WHERE room.id = ?";

/// How many campuses of *other* schools (`?2` is the acting school) share `?1`.
const ROOM_SHARERS: &str = "SELECT COUNT(*) FROM room_campus \
     JOIN campus ON campus.id = room_campus.campus_id \
     WHERE room_campus.room_id = ?1 AND campus.school_id <> ?2";

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
    pub fn unshare_room(&mut self, school_id: Uuid, room_id: Uuid, campus_id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_may_unshare(tx, "room", ROOM_OWNER, room_id, campus_id, school_id)?;
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
    pub fn delete_room(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(tx, "room", ROOM_OWNER, id, school_id)?;
            require_unshared(tx, "room", ROOM_SHARERS, id, school_id)?;
            clear_for_entity(tx, AvailabilityEntity::Room, id)?;
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
    pub fn resources(&self, school_id: Uuid) -> Result<Vec<Resource>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {RESOURCE_COLUMNS} FROM resource WHERE school_id = ? ORDER BY name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], Resource::from_row)?;
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
            require_same_school(tx, "resource", "resource", id, input.school_id)?;
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
    pub fn delete_resource(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "resource", "resource", id, school_id)?;
            clear_for_entity(tx, AvailabilityEntity::Resource, id)?;
            delete(tx, "resource", "resource", id)
        })
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
    pub fn academic_years(&self, school_id: Uuid) -> Result<Vec<AcademicYear>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ACADEMIC_YEAR_COLUMNS} FROM academic_year WHERE school_id = ? \
                 ORDER BY starts_on, name, id"
            ))?;
            let rows = stmt.query_map([text(school_id)], AcademicYear::from_row)?;
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
            require_same_school(tx, "academic year", "academic_year", id, input.school_id)?;
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
    pub fn delete_academic_year(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, "academic year", "academic_year", id, school_id)?;
            delete(tx, "academic year", "academic_year", id)
        })
    }
}

// ------------------------------------------------------------------------------ term

/// A term belongs to a school only by way of its academic year.
const TERM_OWNER: &str = "SELECT academic_year.school_id FROM term \
     JOIN academic_year ON academic_year.id = term.academic_year_id WHERE term.id = ?";

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
    pub fn delete_term(&mut self, school_id: Uuid, id: Uuid) -> Result<()> {
        self.db.write(|tx| {
            require_school_owner(tx, "term", TERM_OWNER, id, school_id)?;
            delete(tx, "term", "term", id)
        })
    }
}
