-- The People group: the teachers, students and audiences a timetable is built for.
--
-- DATA-MODEL.md §4 is the source of this schema. Two deliberate departures from it are
-- recorded in docs/adr/0018-people-identity-and-deactivation.md:
--
--   * `teacher.is_active` and `student.is_active`. The documented model has no such
--     column. People leave and return, and a school that cannot say "not this year"
--     without deleting the record will delete the record — taking its history with it.
--   * `code` is optional here and NOT NULL in the documented model. A school that
--     numbers its staff should be able to say so; one that does not should not be made
--     to invent numbers. See the partial indexes below for what that costs.
--
-- `department`, `subject` and `teacher_qualification` are documented alongside these
-- tables but belong with Curriculum, and arrive with it. `teacher.department_id` is the
-- one column that implies, and adding it later is `ALTER TABLE ... ADD COLUMN` against a
-- nullable reference — the cheap direction. ADR 0006's "carry the columns early" argument
-- applies to structures a later milestone would have to migrate *through*, which this is
-- not: nothing reads a teacher's department until courses exist.
--
-- Deletion semantics, chosen rather than defaulted:
--
--   * Ownership edges (school, and a group's parent) are RESTRICT, as everywhere else.
--   * Optional classification — a teacher's home campus, a student's year level and
--     campus, a group's year level — is SET NULL. The person is the record; where they
--     are based and which year they sit in is metadata, and losing a campus should not
--     take a teacher with it.
--   * `student_group_member` is RESTRICT on both sides, and the repository clears the
--     rows it means to clear inside the same transaction. A membership is history, and
--     the one thing this group must never do is lose it silently. Deleting a *group*
--     removes its own membership rows after the caller has been told how many; deleting
--     a *student* who is still in a group is refused outright, because that is the case
--     where deactivation is what was actually meant.
--
-- Forward-only. V1-V9 are untouched.

-- Year levels: "Year 9", "Grade 8", "Sixth Form". Ordinal is the school's own progression
-- order, not an age and not a date — the same reason cycle_day.ordinal exists.
CREATE TABLE year_level (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    ordinal    INTEGER NOT NULL CHECK (ordinal >= 0),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, name),
    -- Two year levels sharing a position would make "the year below" meaningless.
    UNIQUE (school_id, ordinal)
) STRICT;

CREATE TABLE teacher (
    id                      TEXT    PRIMARY KEY NOT NULL,
    school_id               TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- Optional staff or payroll reference. Unique when given; see teacher_code_unique.
    code                    TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    given_name              TEXT    NOT NULL CHECK (length(trim(given_name)) > 0),
    family_name             TEXT    NOT NULL CHECK (length(trim(family_name)) > 0),
    -- Optional, and never a credential: Evara has no accounts and no login. ADR 0001.
    email                   TEXT             CHECK (email IS NULL OR length(trim(email)) > 0),
    home_campus_id          TEXT             REFERENCES campus (id) ON DELETE SET NULL,
    -- Full-time equivalent, as a percentage: 100 is full time, 50 is a half timetable.
    -- DATA-MODEL.md writes this as `fte REAL`. An integer is used instead because this is
    -- a number the Phase 4 load constraints will compare and sum, and CLAUDE.md already
    -- requires integer arithmetic on that path -- "scores are integers", weights become
    -- integer penalties. A REAL would also make the record non-`Eq`, which every other
    -- entity in this crate is. Carried now, read in Phase 4.
    fte_percent             INTEGER NOT NULL DEFAULT 100
                                     CHECK (fte_percent BETWEEN 0 AND 100),
    max_periods_per_day     INTEGER          CHECK (max_periods_per_day IS NULL OR max_periods_per_day >= 0),
    max_periods_per_cycle   INTEGER          CHECK (max_periods_per_cycle IS NULL OR max_periods_per_cycle >= 0),
    max_consecutive_periods INTEGER          CHECK (max_consecutive_periods IS NULL OR max_consecutive_periods >= 0),
    notes                   TEXT,
    -- Employment state, not a soft delete: an inactive teacher keeps every assignment
    -- they ever had, and simply stops being offered for new ones.
    is_active               INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at              TEXT    NOT NULL,
    updated_at              TEXT    NOT NULL,
    rev                     INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

-- `code` is nullable, so a plain UNIQUE would not hold: SQLite treats NULLs as distinct,
-- which is right here and wrong in general — CLAUDE.md's COALESCE rule exists for the
-- opposite case, where unset rows must still collide. Wrapping this one in
-- COALESCE(code, '') would be actively wrong: every teacher without a staff number would
-- collide with every other, and a school that does not number its staff could enter
-- exactly one teacher. A partial index says what is actually meant — unique when present,
-- unconstrained when absent.
CREATE UNIQUE INDEX teacher_code_unique
    ON teacher (school_id, code) WHERE code IS NOT NULL;

CREATE INDEX teacher_by_name ON teacher (school_id, family_name, given_name);

CREATE TABLE student (
    id            TEXT    PRIMARY KEY NOT NULL,
    school_id     TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- Admission or roll number. Optional, unique when given.
    code          TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    given_name    TEXT    NOT NULL CHECK (length(trim(given_name)) > 0),
    family_name   TEXT    NOT NULL CHECK (length(trim(family_name)) > 0),
    email         TEXT             CHECK (email IS NULL OR length(trim(email)) > 0),
    year_level_id TEXT             REFERENCES year_level (id) ON DELETE SET NULL,
    campus_id     TEXT             REFERENCES campus (id) ON DELETE SET NULL,
    -- Enrolment state. A student who has left keeps their group history.
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

CREATE UNIQUE INDEX student_code_unique
    ON student (school_id, code) WHERE code IS NOT NULL;

CREATE INDEX student_by_name ON student (school_id, family_name, given_name);

-- One table for every audience: a class, a cohort, a homeroom, an elective set.
-- DATA-MODEL.md §4 is explicit that this is deliberately not five tables.
CREATE TABLE student_group (
    id              TEXT    PRIMARY KEY NOT NULL,
    school_id       TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name            TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code            TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    kind            TEXT    NOT NULL CHECK (kind IN ('CLASS', 'COHORT', 'HOMEROOM',
                                                     'ELECTIVE_SET', 'TEACHING_SET')),
    year_level_id   TEXT             REFERENCES year_level (id) ON DELETE SET NULL,
    -- The hierarchy that drives student clash detection: a cohort split into three
    -- teaching sets means those sets clash with the cohort but not with each other.
    -- RESTRICT, because deleting a parent out from under its children would silently
    -- flatten exactly the structure clash detection reads.
    parent_group_id TEXT             REFERENCES student_group (id) ON DELETE RESTRICT,
    -- Declared size, for a school that timetables without entering students at all.
    -- NULL means "membership is the truth"; a number means "there are this many, and
    -- I am not listing them". ADR 0006 requires the second to keep working.
    size            INTEGER          CHECK (size IS NULL OR size >= 0),
    campus_id       TEXT             REFERENCES campus (id) ON DELETE SET NULL,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    rev             INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, name),
    -- Catches the one-step cycle. Longer cycles are walked by the repository before any
    -- write, because SQLite cannot express "no cycles" as a constraint.
    CHECK (parent_group_id IS NULL OR parent_group_id <> id)
) STRICT;

CREATE UNIQUE INDEX student_group_code_unique
    ON student_group (school_id, code) WHERE code IS NOT NULL;

CREATE INDEX student_group_by_parent ON student_group (parent_group_id);

-- Membership is many-to-many on purpose. A student is in a homeroom *and* a maths set
-- *and* an elective; two students in one homeroom take different electives. Anything that
-- assumed one group per student would have to be unpicked the moment electives arrive.
--
-- DATA-MODEL.md §4 writes this as PRIMARY KEY (student_group_id, student_id). It gets its
-- own `id` and audit columns instead, for the reason V5 gives for `building_campus`: the
-- change-log triggers read NEW.id and NEW.rev, and repo::update / repo::delete address
-- rows by id, so a composite-key table would sit outside both — and therefore outside the
-- change feed. Putting a student into a group is a real edit to a real school and belongs
-- in the feed like any other.
CREATE TABLE student_group_member (
    id               TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception. The repository
    -- removes these rows itself through repo::delete so each one is a truthful change-log
    -- entry; a CASCADE would remove them without firing the trigger, because SQLite does
    -- not fire triggers for foreign-key actions unless recursive_triggers is on.
    student_group_id TEXT    NOT NULL REFERENCES student_group (id) ON DELETE RESTRICT,
    student_id       TEXT    NOT NULL REFERENCES student (id) ON DELETE RESTRICT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Both columns are NOT NULL, so no COALESCE is needed for this one to actually hold.
    UNIQUE (student_group_id, student_id)
) STRICT;

-- Every RESTRICT delete scans the children to decide whether to refuse. student_group_id
-- is covered by the UNIQUE index above; student_id needs its own.
CREATE INDEX idx_student_group_member_student ON student_group_member (student_id);
