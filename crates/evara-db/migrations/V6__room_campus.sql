-- A room may serve more than one campus.
--
-- Forward-only. V1-V5 are untouched.
--
-- V3 modelled a room as belonging to exactly one campus. V5 made a *building* shareable;
-- this does the same for the rooms inside one, and for rooms with no building at all. The
-- motivating case is the same physical space used by two campuses on one site: a sports
-- hall, a shared science lab, a lecture theatre a sixth-form college and a school both
-- timetable into. Modelling that as two rooms gives one physical space two identities,
-- and — the part that matters — two independent occupancy records, so each campus could
-- book it at the same time and neither would notice.
--
-- See ADR 0014 for the whole decision, including why `room.campus_id` survives this
-- migration rather than being dropped. The reasoning is ADR 0013's, unchanged: refinery
-- runs migrations inside one transaction, `PRAGMA foreign_keys` is a no-op inside a
-- transaction, and so the documented table-rebuild procedure is unavailable. The column
-- stays, with a narrowed meaning:
--
--   * `room_campus` is the COMPLETE set of campuses a room serves. Every campus has a row
--     here, the home campus included. Nothing reads `room.campus_id` to answer "which
--     campuses?".
--   * `room.campus_id` is the HOME campus: the one it was created under. Immutable,
--     always has a matching `room_campus` row, and cannot be unshared. It still scopes
--     UNIQUE (campus_id, code), and it guarantees a room can never reach zero campuses
--     and vanish from every screen.
--
-- A SHARED ROOM IS ONE ROOM. `room.id` is its scheduling identity, and occupancy is a
-- property of that identifier, not of (room, campus). ADR 0014 records this as a binding
-- invariant on the constraints phase: `room-clash` (CORE-003) must be evaluated per
-- `room_id` across the whole project, never per campus. Nothing in this migration
-- enforces it, because nothing is scheduled yet.

-- An association is a first-class entity, not a bare junction row: its own UUIDv7, its own
-- audit columns, its own change-log triggers, its own entry in change_log::TRACKED_TABLES.
-- Same reasoning as `building_campus` in V5 — the change-log triggers read NEW.id and
-- NEW.rev, and repo::update/repo::delete address rows by id, so a composite-key table
-- would sit outside both, which means outside the change feed.
CREATE TABLE room_campus (
    id         TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception. delete_room removes
    -- these rows itself through repo::delete so each one is a truthful change-log entry;
    -- a CASCADE would remove them without firing the trigger, because SQLite does not
    -- fire triggers for foreign-key actions unless recursive_triggers is on.
    room_id    TEXT    NOT NULL REFERENCES room (id)   ON DELETE RESTRICT,
    campus_id  TEXT    NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- The same campus cannot be associated twice. Both columns are NOT NULL, so no
    -- COALESCE is needed for this one to actually hold.
    UNIQUE (room_id, campus_id)
) STRICT;

-- Every RESTRICT delete scans the children to decide whether to refuse. room_id is
-- covered by the UNIQUE index above; campus_id needs its own.
CREATE INDEX idx_room_campus_campus ON room_campus (campus_id);

-- Carry every existing room-to-campus relationship across. One row per room, naming its
-- existing campus, so a project migrated by this statement describes exactly the same
-- school it did before — no room changes identity, no row in `room` is rewritten, and no
-- revision is consumed.
--
-- `created_at` is the room's own, because the association is as old as the relationship it
-- records, and `updated_at` matches it because nothing has edited it yet — the Stamp
-- convention for a row at rev 1.
--
-- The identifier is laid out field by field for the reason V5's backfill was: SQLite has
-- no uuid() and refinery migrations are .sql files. 48-bit millisecond timestamp, version
-- nibble 7, then the two-bit variant as one of 8/9/a/b, then random. `random() & 3` keeps
-- the low two bits and is therefore always 0-3 even though random() is signed. A test
-- asserts get_version_num() == 7 on the result rather than trusting this printf.
INSERT INTO room_campus (id, room_id, campus_id, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    room.id,
    room.campus_id,
    room.created_at,
    room.created_at,
    1
FROM room;

-- ---------------------------------------------------------------------------------------
-- A room's building must be available on every campus the room serves
-- ---------------------------------------------------------------------------------------
--
-- A shared building does not imply shared rooms, and a shared room does not imply a shared
-- building. The two settings are independent, and this is the one place they have to agree:
-- a room may serve a campus only if its building — when it has one — is available to that
-- campus too. Otherwise a campus would list a room sitting in a building that campus does
-- not have, which is not a view of a school anyone can act on.
--
-- Rooms with no building are unaffected throughout: every check below is guarded on the
-- room's `building_id` being non-null.
--
-- V5's pair of triggers compared the building against `room.campus_id` alone. Under room
-- sharing the home campus is no longer the whole question, so they are replaced — the rule
-- is unchanged, its scope widens from one campus to every campus the room serves.
--
-- The abort message is unchanged on purpose: structure::room_error matches on
-- "building must be on the room" to turn this into a typed error the form can show. The
-- repository checks the same rule first and raises a message naming the campus and the
-- building; these triggers are the backstop for a write that bypasses it, exactly as
-- ADR 0010 treats `ON DELETE SET NULL`.
DROP TRIGGER room_building_must_share_campus_insert;
DROP TRIGGER room_building_must_share_campus_update;

-- On INSERT only the home campus exists to check: `room_campus` cannot yet hold rows for a
-- room that is not in the table. create_room writes the room, then its associations, and
-- each association is checked by its own trigger below.
CREATE TRIGGER room_building_must_share_campus_insert
BEFORE INSERT ON room FOR EACH ROW
WHEN NEW.building_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE NOT EXISTS (
        SELECT 1 FROM building_campus
        WHERE building_campus.building_id = NEW.building_id
          AND building_campus.campus_id = NEW.campus_id
    );
END;

-- On UPDATE the room may already serve several campuses, and moving it into a building
-- has to hold for all of them. The home campus is one of those rows, so it is covered by
-- the second clause too; it is checked explicitly as well because the invariant "home
-- always has an association row" is maintained by the repository rather than the schema,
-- and this trigger should not be the thing that quietly stops enforcing the original rule
-- if that ever broke.
CREATE TRIGGER room_building_must_share_campus_update
BEFORE UPDATE ON room FOR EACH ROW
WHEN NEW.building_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE NOT EXISTS (
        SELECT 1 FROM building_campus
        WHERE building_campus.building_id = NEW.building_id
          AND building_campus.campus_id = NEW.campus_id
    )
    OR EXISTS (
        SELECT 1 FROM room_campus rc
        WHERE rc.room_id = NEW.id
          AND NOT EXISTS (
              SELECT 1 FROM building_campus bc
              WHERE bc.building_id = NEW.building_id
                AND bc.campus_id = rc.campus_id
          )
    );
END;

-- The other direction: associating a room with a campus its building does not reach.
--
-- The message is the same string for the same reason — one rule, one typed error, whether
-- it is reached by moving a room into a building or by sharing a room to a campus.
CREATE TRIGGER room_campus_building_must_be_available_insert
BEFORE INSERT ON room_campus FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE EXISTS (
        SELECT 1 FROM room
        WHERE room.id = NEW.room_id
          AND room.building_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM building_campus bc
              WHERE bc.building_id = room.building_id
                AND bc.campus_id = NEW.campus_id
          )
    );
END;

-- Nothing in the repository updates an association row — share and unshare are an insert
-- and a delete — so this trigger should never fire. It exists so that the invariant does
-- not depend on that staying true: a future repository method that repointed an
-- association would otherwise break the rule silently.
CREATE TRIGGER room_campus_building_must_be_available_update
BEFORE UPDATE ON room_campus FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE EXISTS (
        SELECT 1 FROM room
        WHERE room.id = NEW.room_id
          AND room.building_id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM building_campus bc
              WHERE bc.building_id = room.building_id
                AND bc.campus_id = NEW.campus_id
          )
    );
END;
