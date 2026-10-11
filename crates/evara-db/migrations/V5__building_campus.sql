-- A building may serve more than one campus.
--
-- Forward-only. V1-V4 are untouched.
--
-- V3 modelled a building as owned by exactly one campus. Real schools share one: a sports
-- centre between two campuses of a trust, a science block two campuses timetable into.
-- Modelling that as two buildings gives one physical thing two identities, two sets of
-- rooms to keep in step, and a solver that cannot tell that crossing between them is not
-- actually a journey.
--
-- See ADR 0013 for why `building.campus_id` survives this migration rather than being
-- dropped. The short version: refinery runs migrations inside one transaction,
-- `PRAGMA foreign_keys` is a no-op inside a transaction, and so the documented
-- table-rebuild procedure is unavailable — while `DROP TABLE building` with foreign keys
-- on would fire `room.building_id`'s SET NULL and silently detach every room in the
-- school on the way past. The column stays, with a narrowed meaning:
--
--   * `building_campus` is the COMPLETE set of campuses a building serves. Every campus
--     has a row here, the home campus included. Nothing reads `building.campus_id` to
--     answer "which campuses?".
--   * `building.campus_id` is the HOME campus: the one it was created under. Immutable,
--     always has a matching `building_campus` row, and cannot be unshared. It still scopes
--     UNIQUE (campus_id, name), and it guarantees a building can never reach zero campuses
--     and vanish from every screen.

-- An association is a first-class entity, not a bare junction row: its own UUIDv7, its own
-- audit columns, its own change-log triggers, its own entry in change_log::TRACKED_TABLES.
-- That is required rather than decorative — the change-log triggers read NEW.id and
-- NEW.rev, and repo::update/repo::delete address rows by id, so a composite-key table
-- would sit outside both, which means outside the change feed. Sharing a building is a
-- real edit to a real school and belongs in the feed like any other.
CREATE TABLE building_campus (
    id          TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception. delete_building
    -- removes these rows itself through repo::delete so each one is a truthful change-log
    -- entry; a CASCADE would remove them without firing the trigger, because SQLite does
    -- not fire triggers for foreign-key actions unless recursive_triggers is on.
    building_id TEXT    NOT NULL REFERENCES building (id) ON DELETE RESTRICT,
    campus_id   TEXT    NOT NULL REFERENCES campus (id)   ON DELETE RESTRICT,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    rev         INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- The same campus cannot be associated twice. Both columns are NOT NULL, so no
    -- COALESCE is needed for this one to actually hold.
    UNIQUE (building_id, campus_id)
) STRICT;

-- Every RESTRICT delete scans the children to decide whether to refuse. building_id is
-- covered by the UNIQUE index above; campus_id needs its own.
CREATE INDEX idx_building_campus_campus ON building_campus (campus_id);

-- Carry every existing building-to-campus relationship across. One row per building,
-- naming its existing campus, so a project migrated by this statement describes exactly
-- the same school it did before — no building changes identity, no row in `building` or
-- `room` is rewritten, and no revision is consumed.
--
-- `created_at` is the building's own, because the association is as old as the
-- relationship it records, and `updated_at` matches it because nothing has edited it yet —
-- the Stamp convention for a row at rev 1.
--
-- SQLite has no uuid(), and refinery migrations are .sql files, so the identifier is laid
-- out field by field: 48-bit millisecond timestamp, version nibble 7, then the two-bit
-- variant as one of 8/9/a/b, then random. `random() & 3` keeps the low two bits and is
-- therefore always 0-3 even though random() is signed. A test asserts
-- get_version_num() == 7 on the result rather than trusting this printf.
INSERT INTO building_campus (id, building_id, campus_id, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    building.id,
    building.campus_id,
    building.created_at,
    building.created_at,
    1
FROM building;

-- V3 enforced "a room's building must be on the room's campus" by comparing
-- building.campus_id to room.campus_id. Under sharing that is the wrong question: it would
-- reject a room on a campus the building genuinely serves. The rule itself is still right
-- and still belongs in the schema — it is what makes "one shared building, rooms belonging
-- to different campuses" safe rather than merely intended — so the triggers are replaced,
-- not removed.
--
-- The abort message is unchanged on purpose: structure::translate_room matches on
-- "building must be on the room" to turn this into a typed error the form can show.
DROP TRIGGER room_building_must_share_campus_insert;
DROP TRIGGER room_building_must_share_campus_update;

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

CREATE TRIGGER room_building_must_share_campus_update
BEFORE UPDATE ON room FOR EACH ROW
WHEN NEW.building_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE NOT EXISTS (
        SELECT 1 FROM building_campus
        WHERE building_campus.building_id = NEW.building_id
          AND building_campus.campus_id = NEW.campus_id
    );
END;
