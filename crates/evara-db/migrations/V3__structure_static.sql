-- Structure, static part: places and the academic calendar's coarse divisions.
--
-- Scope is deliberately narrow. The *time* half of the Structure group — cycle, cycle_day,
-- period_structure, period and the materialised timeslot grid — is Phase 1D, because
-- timeslot materialisation has to preserve surviving slot identifiers and that deserves
-- its own migration and its own tests.
--
-- Deletion semantics, chosen deliberately rather than defaulted (DATA-MODEL.md §3 does not
-- state them):
--
--   * Ownership edges are ON DELETE RESTRICT. Deleting a campus that still holds buildings,
--     rooms or resources fails and says so. School data is expensive to recreate, and a
--     cascade turns one mis-click into silent loss of structure that timetables depend on.
--   * Optional classification references within a campus — room.building_id and
--     room.room_type_id — are ON DELETE SET NULL. The room is the asset; where it sits and
--     what kind it is are metadata.
--
-- Repositories clear those optional references explicitly before deleting a parent, so the
-- SET NULL path is a backstop rather than the normal route; see crates/evara-db/src/repo.
-- That matters because a database-level SET NULL changes a row without incrementing `rev`.
--
-- Forward-only. V1 and V2 are untouched.

-- One school per project. ADR 0003: "Multiple schools means multiple project files."
-- The constant column with a UNIQUE index is the standard SQLite way to say "at most one
-- row" without a trigger.
CREATE TABLE school (
    id         TEXT    PRIMARY KEY NOT NULL,
    only_one   INTEGER NOT NULL DEFAULT 1 CHECK (only_one = 1) UNIQUE,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    timezone   TEXT    NOT NULL CHECK (length(trim(timezone)) > 0),
    locale     TEXT    NOT NULL CHECK (length(trim(locale)) > 0),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

CREATE TABLE campus (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code       TEXT    NOT NULL CHECK (length(trim(code)) > 0),
    address    TEXT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Codes are unique within their parent, never globally: schools rename and reuse them.
    UNIQUE (school_id, code)
) STRICT;

-- A building has no code in the documented model, so its name is what distinguishes it
-- within a campus.
CREATE TABLE building (
    id         TEXT    PRIMARY KEY NOT NULL,
    campus_id  TEXT    NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (campus_id, name)
) STRICT;

CREATE TABLE room_type (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code       TEXT    NOT NULL CHECK (length(trim(code)) > 0),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, code)
) STRICT;

CREATE TABLE room (
    id           TEXT    PRIMARY KEY NOT NULL,
    campus_id    TEXT    NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
    building_id  TEXT             REFERENCES building (id) ON DELETE SET NULL,
    room_type_id TEXT             REFERENCES room_type (id) ON DELETE SET NULL,
    name         TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code         TEXT    NOT NULL CHECK (length(trim(code)) > 0),
    capacity     INTEGER NOT NULL CHECK (capacity >= 0),
    is_bookable  INTEGER NOT NULL DEFAULT 1 CHECK (is_bookable IN (0, 1)),
    notes        TEXT,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (campus_id, code)
) STRICT;

-- A room names both its campus and, optionally, a building. Nothing in a plain foreign key
-- stops those disagreeing, and a composite key cannot be used because ON DELETE SET NULL
-- would try to null the NOT NULL campus_id too. Triggers enforce it instead.
CREATE TRIGGER room_building_must_share_campus_insert
BEFORE INSERT ON room FOR EACH ROW
WHEN NEW.building_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE NOT EXISTS (
        SELECT 1 FROM building
        WHERE building.id = NEW.building_id AND building.campus_id = NEW.campus_id
    );
END;

CREATE TRIGGER room_building_must_share_campus_update
BEFORE UPDATE ON room FOR EACH ROW
WHEN NEW.building_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s building must be on the room''s campus')
    WHERE NOT EXISTS (
        SELECT 1 FROM building
        WHERE building.id = NEW.building_id AND building.campus_id = NEW.campus_id
    );
END;

CREATE TABLE resource (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- Optional: a resource may be school-wide or tied to one campus. RESTRICT, not
    -- SET NULL, so that "delete a campus" means the same thing everywhere.
    campus_id  TEXT             REFERENCES campus (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code       TEXT    NOT NULL CHECK (length(trim(code)) > 0),
    quantity   INTEGER NOT NULL CHECK (quantity >= 0),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, code)
) STRICT;

CREATE TABLE academic_year (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- ISO-8601 calendar dates. `date()` returns NULL for anything SQLite cannot parse,
    -- which is how the format is enforced without a regular expression.
    starts_on  TEXT    NOT NULL CHECK (date(starts_on) IS NOT NULL),
    ends_on    TEXT    NOT NULL CHECK (date(ends_on) IS NOT NULL),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    CHECK (ends_on >= starts_on),
    UNIQUE (school_id, name)
) STRICT;

CREATE TABLE term (
    id               TEXT    PRIMARY KEY NOT NULL,
    academic_year_id TEXT    NOT NULL REFERENCES academic_year (id) ON DELETE RESTRICT,
    name             TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    ordinal          INTEGER NOT NULL CHECK (ordinal >= 1),
    starts_on        TEXT    NOT NULL CHECK (date(starts_on) IS NOT NULL),
    ends_on          TEXT    NOT NULL CHECK (date(ends_on) IS NOT NULL),
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    CHECK (ends_on >= starts_on),
    UNIQUE (academic_year_id, ordinal),
    UNIQUE (academic_year_id, name)
) STRICT;

-- Foreign key columns are not indexed automatically, and every RESTRICT delete has to scan
-- the children to decide whether to refuse.
CREATE INDEX idx_campus_school ON campus (school_id);
CREATE INDEX idx_building_campus ON building (campus_id);
CREATE INDEX idx_room_type_school ON room_type (school_id);
CREATE INDEX idx_room_campus ON room (campus_id);
CREATE INDEX idx_room_building ON room (building_id);
CREATE INDEX idx_room_type_of_room ON room (room_type_id);
CREATE INDEX idx_resource_school ON resource (school_id);
CREATE INDEX idx_resource_campus ON resource (campus_id);
CREATE INDEX idx_academic_year_school ON academic_year (school_id);
CREATE INDEX idx_term_year ON term (academic_year_id);
