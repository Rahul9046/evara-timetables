-- Structure, time part: the repeating cycle, the bell schedule, the materialised timeslot
-- grid, and the projection of real dates onto cycle days.
--
-- Forward-only. V1, V2 and V3 are untouched.
--
-- The model deliberately avoids assuming Monday–Friday. `cycle_day.ordinal` is the only
-- scheduling identity a day has; `label` ("Monday", "Day A", "Day 1") and `weekday_hint`
-- are display, and nothing in materialisation, ordering or identity reads them. A one-day
-- cycle, a six-day rotation and a ten-day fortnight differ only in how many rows exist.
--
-- `timeslot` is materialised rather than computed (DATA-MODEL.md §3) because the solver
-- needs a stable, indexable value range that a locked assignment can point at. Its logical
-- identity is (term_id, cycle_day_id, period_id) and its `id` is assigned once and never
-- reassigned while that tuple exists — see ADR 0011, which also records why four schema
-- questions were asked before this file was written rather than defaulted.
--
-- Deletion semantics, continuing D19/ADR 0010 and resolved for the new edges in ADR 0011:
--
--   * Ownership edges are ON DELETE RESTRICT, as everywhere else.
--   * Optional references are RESTRICT too, not SET NULL, so that "delete a campus" or
--     "delete a term" means the same thing on every edge. For `timeslot.term_id` this is
--     not merely consistency: term_id is part of the logical identity, so nulling it would
--     silently rewrite which slot a row *is* and could collide with an existing slot.
--   * `timeslot` is never cascade-deleted. A materialised grid blocks the deletion of the
--     cycle day or period it was built from, which is the point — removing either is a
--     deliberate, confirmed operation, not a side effect.
--   * `calendar_day.cycle_day_id` is RESTRICT although NULL is meaningful there ("no
--     lessons that day"), because SET NULL would silently reinterpret mapped teaching
--     dates as non-teaching ones.
--
-- Several unique indexes use COALESCE over a nullable column. SQLite treats NULLs as
-- distinct in a UNIQUE constraint, so the uniqueness DATA-MODEL.md documents for
-- `timeslot` and `calendar_day` would not have held for the term-less and campus-less
-- rows — exactly the common cases. The expression index makes it hold.

-- The repeating pattern. day_count = 5 for a weekly cycle, 10 for a fortnight, 6 for a
-- rotation that drifts against the calendar week. Deliberately no upper bound.
--
-- day_count is the *declared* length (ADR 0011 Q4). The materialiser uses the cycle_day
-- rows that actually exist, and the repository rejects an ordinal beyond day_count without
-- demanding the set be complete, so building a cycle one day at a time is legal.
CREATE TABLE cycle (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    day_count  INTEGER NOT NULL CHECK (day_count >= 1),
    week_count INTEGER NOT NULL DEFAULT 1 CHECK (week_count >= 1),
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, name)
) STRICT;

-- At most one default cycle per school. A default with two candidates surfaces much later
-- as "the app picked the wrong cycle".
CREATE UNIQUE INDEX idx_cycle_single_default ON cycle (school_id) WHERE is_default = 1;

-- One day of the pattern. `ordinal` is the identity; `label` is what a human reads.
CREATE TABLE cycle_day (
    id           TEXT    PRIMARY KEY NOT NULL,
    cycle_id     TEXT    NOT NULL REFERENCES cycle (id) ON DELETE RESTRICT,
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 1),
    label        TEXT    NOT NULL CHECK (length(trim(label)) > 0),
    -- 1..7, display only, never logic. Lets a weekly school show "Monday" without the
    -- scheduling model knowing what a Monday is.
    weekday_hint INTEGER          CHECK (weekday_hint BETWEEN 1 AND 7),
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (cycle_id, ordinal),
    -- Labels are how a user refers to a day, so two "Day A"s in one cycle are a mistake.
    -- Follows D20's rule that a name is unique within its parent.
    UNIQUE (cycle_id, label)
) STRICT;

-- A bell schedule. Campus-scoped, so campuses may run different times; NULL campus_id
-- means school-wide.
CREATE TABLE period_structure (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    campus_id  TEXT             REFERENCES campus (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, name)
) STRICT;

-- At most one default per campus, with the school-wide rows forming their own bucket.
CREATE UNIQUE INDEX idx_period_structure_single_default
    ON period_structure (school_id, COALESCE(campus_id, '')) WHERE is_default = 1;

-- One period of the bell schedule. Times are 'HH:MM' local wall clock: the school's
-- timezone lives on `school`, and a bell schedule is a wall-clock thing, not an instant.
CREATE TABLE period (
    id                  TEXT    PRIMARY KEY NOT NULL,
    period_structure_id TEXT    NOT NULL REFERENCES period_structure (id) ON DELETE RESTRICT,
    ordinal             INTEGER NOT NULL CHECK (ordinal >= 1),
    label               TEXT    NOT NULL CHECK (length(trim(label)) > 0),
    -- LIKE pins the shape and time() pins validity; together they reject '9:30' as well
    -- as '25:00', so stored times sort as text.
    starts_at           TEXT    NOT NULL CHECK (starts_at LIKE '__:__' AND time(starts_at) IS NOT NULL),
    ends_at             TEXT    NOT NULL CHECK (ends_at LIKE '__:__' AND time(ends_at) IS NOT NULL),
    kind                TEXT    NOT NULL CHECK (kind IN ('TEACHING', 'BREAK', 'REGISTRATION', 'OTHER')),
    counts_as_load      INTEGER NOT NULL DEFAULT 1 CHECK (counts_as_load IN (0, 1)),
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    rev                 INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    CHECK (ends_at > starts_at),
    UNIQUE (period_structure_id, ordinal),
    UNIQUE (period_structure_id, label)
) STRICT;

-- The materialised (cycle_day × period) grid: the solver's value range.
--
-- school_id, cycle_id, campus_id, ordinal and is_teaching are *derived* projections,
-- refreshed on every rematerialisation. Only `id` is stable, and only the identity tuple
-- decides which row is which. `ordinal` in particular is expected to change — inserting a
-- period renumbers every later slot in the cycle while every one of them keeps its id —
-- which is why it carries an index but no uniqueness constraint: renumbering inside one
-- transaction would otherwise collide with itself halfway through.
CREATE TABLE timeslot (
    id           TEXT    PRIMARY KEY NOT NULL,
    school_id    TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    cycle_id     TEXT    NOT NULL REFERENCES cycle (id) ON DELETE RESTRICT,
    cycle_day_id TEXT    NOT NULL REFERENCES cycle_day (id) ON DELETE RESTRICT,
    period_id    TEXT    NOT NULL REFERENCES period (id) ON DELETE RESTRICT,
    campus_id    TEXT             REFERENCES campus (id) ON DELETE RESTRICT,
    -- NULL means the grid applies to the whole year. Part of the logical identity, which
    -- is why it is RESTRICT rather than SET NULL.
    term_id      TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 1),
    is_teaching  INTEGER NOT NULL CHECK (is_teaching IN (0, 1)),
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

-- The identity invariant, enforced. COALESCE because a plain UNIQUE would not constrain
-- the term-less rows at all.
CREATE UNIQUE INDEX idx_timeslot_identity
    ON timeslot (cycle_day_id, period_id, COALESCE(term_id, ''));

-- The denormalised columns must agree with what the slot was built from, or the grid can
-- drift from the cycle it claims to belong to. Same approach as room/building in V3; `IS`
-- rather than `=` so the nullable campus compares null-safely.
CREATE TRIGGER timeslot_derived_columns_must_agree_insert
BEFORE INSERT ON timeslot FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a timeslot''s school, cycle and campus must match the cycle day and period it is built from')
    WHERE NOT EXISTS (
        SELECT 1
        FROM cycle_day
        JOIN cycle ON cycle.id = cycle_day.cycle_id
        JOIN period ON period.id = NEW.period_id
        JOIN period_structure ON period_structure.id = period.period_structure_id
        WHERE cycle_day.id = NEW.cycle_day_id
          AND NEW.cycle_id = cycle_day.cycle_id
          AND NEW.school_id = cycle.school_id
          AND period_structure.school_id = cycle.school_id
          AND NEW.campus_id IS period_structure.campus_id
    );
END;

CREATE TRIGGER timeslot_derived_columns_must_agree_update
BEFORE UPDATE ON timeslot FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a timeslot''s school, cycle and campus must match the cycle day and period it is built from')
    WHERE NOT EXISTS (
        SELECT 1
        FROM cycle_day
        JOIN cycle ON cycle.id = cycle_day.cycle_id
        JOIN period ON period.id = NEW.period_id
        JOIN period_structure ON period_structure.id = period.period_structure_id
        WHERE cycle_day.id = NEW.cycle_day_id
          AND NEW.cycle_id = cycle_day.cycle_id
          AND NEW.school_id = cycle.school_id
          AND period_structure.school_id = cycle.school_id
          AND NEW.campus_id IS period_structure.campus_id
    );
END;

-- Maps real dates onto cycle days, which is what makes rotating and multi-week cycles
-- work: the timetable is defined over the cycle, and the calendar projects it onto dates.
--
-- Phase 1D stores and reads these rows; it does not generate them. What Day follows a
-- holiday, and whether a closure consumes a rotation slot, is policy DATA-MODEL.md does
-- not state — so the mapping is explicit per date and automatic progression waits.
CREATE TABLE calendar_day (
    id           TEXT    PRIMARY KEY NOT NULL,
    school_id    TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- Canonical YYYY-MM-DD: date() rejects nonsense, and the equality rejects '2027-9-1'.
    date         TEXT    NOT NULL CHECK (date(date) IS NOT NULL AND date = date(date)),
    term_id      TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    campus_id    TEXT             REFERENCES campus (id) ON DELETE RESTRICT,
    -- NULL means no lessons that day. RESTRICT, not SET NULL: see the header.
    cycle_day_id TEXT             REFERENCES cycle_day (id) ON DELETE RESTRICT,
    kind         TEXT    NOT NULL CHECK (kind IN ('SCHOOL', 'HOLIDAY', 'EXAM', 'EVENT', 'PD')),
    note         TEXT,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

-- One entry per date per campus, with the school-wide rows forming their own bucket.
CREATE UNIQUE INDEX idx_calendar_day_identity
    ON calendar_day (school_id, date, COALESCE(campus_id, ''));

-- Foreign key columns are not indexed automatically, and every RESTRICT delete has to scan
-- the children to decide whether to refuse.
CREATE INDEX idx_cycle_school ON cycle (school_id);
CREATE INDEX idx_cycle_day_cycle ON cycle_day (cycle_id);
CREATE INDEX idx_period_structure_school ON period_structure (school_id);
CREATE INDEX idx_period_structure_campus ON period_structure (campus_id);
CREATE INDEX idx_period_structure_of_period ON period (period_structure_id);

-- The solver's read pattern: the whole value range of one cycle, in order.
CREATE INDEX idx_timeslot_cycle ON timeslot (cycle_id, ordinal);
CREATE INDEX idx_timeslot_school ON timeslot (school_id);
CREATE INDEX idx_timeslot_day ON timeslot (cycle_day_id);
CREATE INDEX idx_timeslot_period ON timeslot (period_id);
CREATE INDEX idx_timeslot_campus ON timeslot (campus_id);
CREATE INDEX idx_timeslot_term ON timeslot (term_id);

CREATE INDEX idx_calendar_day_school ON calendar_day (school_id);
CREATE INDEX idx_calendar_day_term ON calendar_day (term_id);
CREATE INDEX idx_calendar_day_campus ON calendar_day (campus_id);
CREATE INDEX idx_calendar_day_cycle_day ON calendar_day (cycle_day_id);
