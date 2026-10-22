//! Rooms and the kinds of room a school classifies by.
//!
//! # One physical room, one identity
//!
//! [ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md): a room shared by
//! two campuses — or by two schools of one trust — is **one** `room.id`, because occupancy
//! is a property of that identifier and not of `(room, campus)`. Modelling a shared hall as
//! two rooms gives one physical space two independent occupancy records, so each campus
//! could book it at the same time and neither would notice. That record calls it "a
//! correctness failure", and this loader is the first code that could commit it.
//!
//! So the rooms in scope are read through `room_campus`, which `V6`'s header establishes as
//! *the complete set of campuses a room serves, the home campus included* — nothing here
//! reads `room.campus_id`, which since `V6` means only "the campus it was created under".
//!
//! # Room types are not all the school's own
//!
//! `Problem::room_types` holds the school's own types **and** any type an in-scope room is
//! classified under, because `create_room` does not require a room's type to belong to its
//! campus's school and a shared room's type belongs to whichever school created it.
//!
//! Dropping such a classification was the tempting alternative and is wrong in the dangerous
//! direction: an unclassified room is a candidate for *every* requirement, so a shared
//! laboratory would silently satisfy "this section needs a gymnasium". Keeping it means a
//! shared laboratory classified under Northgate's "Science Laboratory" does **not** match
//! Southmoor's "Science Laboratory" — two different records — which is correct, because
//! Southmoor has not said the shared room is one of its laboratories.

use evara_core::scheduling::{OtherSchool, RoomFact, RoomTypeFact};
use rusqlite::Connection;

use super::index::ProblemIndex;
use super::{SchedulingSelection, require_index};
use crate::error::Result;
use crate::repo::row::{parse_id_text, text};

/// Reads the room classifications in scope.
///
/// The school's own first, in the order the interface lists them, then any type a shared
/// room is classified under. The two groups are kept in that order so an index is stable
/// for a school whose own list does not change.
pub(super) fn room_types(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<Vec<RoomTypeFact>> {
    let mut facts = Vec::new();

    let mut own =
        conn.prepare("SELECT id, name FROM room_type WHERE school_id = ? ORDER BY name, id")?;
    let rows = own.query_map([text(selection.school_id)], read_room_type)?;
    for row in rows {
        let (raw, name) = row?;
        let id = parse_id_text("room_type.id", &raw)?;
        index.room_types.intern(id);
        facts.push(RoomTypeFact { id, name });
    }
    drop(own);

    let mut borrowed = conn.prepare(
        "SELECT DISTINCT rt.id, rt.name
         FROM room_type rt
         JOIN room        r  ON r.room_type_id = rt.id
         JOIN room_campus rc ON rc.room_id     = r.id
         JOIN campus      c  ON c.id           = rc.campus_id
         WHERE c.school_id = ?1 AND rt.school_id <> ?1
         ORDER BY rt.name, rt.id",
    )?;
    let rows = borrowed.query_map([text(selection.school_id)], read_room_type)?;
    for row in rows {
        let (raw, name) = row?;
        let id = parse_id_text("room_type.id", &raw)?;
        index.room_types.intern(id);
        facts.push(RoomTypeFact { id, name });
    }

    Ok(facts)
}

/// Reads every room reachable from a campus of this school.
///
/// `campuses` lists only **this** school's campuses that reach the room; `also_at` names the
/// other schools that reach it, which is the exact condition
/// [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
/// rule 3 warns about. No row of the other school is read beyond its name.
pub(super) fn rooms(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &mut ProblemIndex,
) -> Result<Vec<RoomFact>> {
    let school = text(selection.school_id);

    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.id, r.name, r.room_type_id, r.capacity, r.is_bookable
         FROM room        r
         JOIN room_campus rc ON rc.room_id = r.id
         JOIN campus      c  ON c.id       = rc.campus_id
         WHERE c.school_id = ?
         ORDER BY r.name, r.id",
    )?;
    let rows = stmt.query_map([&school], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, bool>(4)?,
        ))
    })?;

    let mut facts = Vec::new();
    for row in rows {
        let (raw, name, raw_type, capacity, is_bookable) = row?;
        let id = parse_id_text("room.id", &raw)?;
        let room_type = raw_type
            .map(|raw| parse_id_text("room.room_type_id", &raw))
            .transpose()?
            .map(|room_type| require_index(&index.room_types, room_type, "room.room_type_id"))
            .transpose()?;
        index.rooms.intern(id);
        facts.push(RoomFact {
            id,
            name,
            campuses: Vec::new(),
            room_type,
            capacity,
            is_bookable,
            also_at: Vec::new(),
        });
    }
    drop(stmt);

    campuses_of_rooms(conn, selection, index, &mut facts)?;
    sharing_of_rooms(conn, selection, index, &mut facts)?;
    Ok(facts)
}

/// Fills in which of this school's campuses reach each room, in campus-index order.
///
/// One query for every room rather than one per room: a school with 200 rooms over three
/// campuses is 600 associations, and asking per room would be the N+1 the loader must avoid.
fn campuses_of_rooms(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
    facts: &mut [RoomFact],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT rc.room_id, rc.campus_id
         FROM room_campus rc
         JOIN campus      c ON c.id = rc.campus_id
         WHERE c.school_id = ?
         ORDER BY c.name, c.id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (raw_room, raw_campus) = row?;
        let room = parse_id_text("room_campus.room_id", &raw_room)?;
        let campus = parse_id_text("room_campus.campus_id", &raw_campus)?;
        // A room this school reaches is already in the table; one it does not is another
        // school's entirely and is not in scope.
        let Some(at) = index.rooms.index_of(room) else {
            continue;
        };
        let campus = require_index(&index.campuses, campus, "room_campus.campus_id")?;
        facts[at.get()].campuses.push(campus);
    }
    Ok(())
}

/// Names the other schools that also reach each in-scope room.
fn sharing_of_rooms(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
    facts: &mut [RoomFact],
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT rc.room_id, s.id, s.name
         FROM room_campus rc
         JOIN campus      c ON c.id = rc.campus_id
         JOIN school      s ON s.id = c.school_id
         WHERE c.school_id <> ?
         ORDER BY s.name, s.id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (raw_room, raw_school, name) = row?;
        let room = parse_id_text("room_campus.room_id", &raw_room)?;
        let Some(at) = index.rooms.index_of(room) else {
            continue;
        };
        let id = parse_id_text("school.id", &raw_school)?;
        let other = OtherSchool { id, name };
        if !facts[at.get()].also_at.contains(&other) {
            facts[at.get()].also_at.push(other);
        }
    }
    Ok(())
}

/// Reads one room-type row.
fn read_room_type(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String)> {
    Ok((row.get(0)?, row.get(1)?))
}

/// The rooms a section with this campus and this requirement may be placed in.
///
/// Delegates to
/// [`RoomFact::is_candidate_for`](evara_core::scheduling::RoomFact::is_candidate_for), which
/// is the **one** implementation of the candidate domain. The loader does not restate the
/// three conditions, and neither will M11's move validator — which is the guarantee
/// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.1 asks for, since *"a domain the manual editor
/// could bypass is a rule living in two places."*
pub(super) fn candidates(
    rooms: &[RoomFact],
    campus: Option<evara_core::scheduling::CampusIx>,
    required_type: Option<evara_core::scheduling::RoomTypeIx>,
) -> Vec<evara_core::scheduling::RoomIx> {
    rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| room.is_candidate_for(campus, required_type))
        .map(|(at, _)| evara_core::scheduling::RoomIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
        .collect()
}
