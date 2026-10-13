-- Multiple schools in one project.
--
-- V3 modelled a project as describing exactly one school, enforced by a constant column
-- with a unique index:
--
--     only_one INTEGER NOT NULL DEFAULT 1 CHECK (only_one = 1) UNIQUE
--
-- ADR 0003 said multiple schools meant multiple project files. ADR 0019 supersedes that
-- one claim: a trust, a group, or a school with a separate junior programme runs several
-- institutions that share staff, rooms and equipment, and splitting them across files
-- makes the shared half unrepresentable -- two files cannot tell each other that the same
-- teacher is already teaching at ten o'clock.
--
-- ---------------------------------------------------------------------------------------
-- Why this rebuilds the table, and why that is safe
-- ---------------------------------------------------------------------------------------
--
-- `only_one` carries a CHECK and a UNIQUE, both *table-level* constraints. SQLite cannot
-- drop either with ALTER TABLE, so the table has to be rebuilt.
--
-- ADR 0013, ADR 0014 and V9's own comments all record that a rebuild was believed
-- impossible here: the documented twelve-step procedure needs `PRAGMA foreign_keys = OFF`,
-- and that pragma is a no-op inside a transaction while refinery runs every pending
-- migration inside one. That is true, and it is incomplete. The rebuild does not need
-- enforcement *disabled*; it needs it *postponed*, and `PRAGMA defer_foreign_keys` is
-- honoured inside a transaction where `foreign_keys` is not.
--
-- So this migration drops and recreates `school` **in place, under its own name**. Keeping
-- the name is what makes it work: not one of the fourteen tables that reference
-- `school (id)` has its foreign-key clause rewritten, because nothing is renamed. The
-- window in which those tables point at a parent row that does not exist is tolerated
-- until COMMIT, by which time the rows are back with identical identifiers.
--
-- Two things this is NOT, both tested and rejected -- see ADR 0020:
--
--   * `PRAGMA legacy_alter_table` plus a rename-swap. The rename does leave referencing
--     clauses intact, but `DROP TABLE school_old` then fails with a foreign-key error,
--     with or without deferred enforcement.
--   * `PRAGMA writable_schema` surgery on the stored DDL. Refused by a defensive build,
--     and unsound anyway: the UNIQUE constraint's implicit index is its own sqlite_master
--     row that no amount of editing the CREATE TABLE text removes, so the rebuilt table
--     would still have an index enforcing one row.
--
-- Deferral is enforcement, not a bypass. A rebuild that failed to restore its rows would
-- be refused at COMMIT and the project left untouched -- which is a stronger guarantee
-- than the twelve-step procedure, where enforcement is genuinely off and only a later
-- `foreign_key_check` would notice.
--
-- The change-log triggers on `school` go with the dropped table. `change_log::install_all`
-- runs on every open, immediately after migrations, with CREATE TRIGGER IF NOT EXISTS, so
-- they are back before the application can write a single row. The rebuild itself emits no
-- change-log entries: DROP TABLE's implicit delete does not fire triggers, and the
-- replacement table has none yet when its rows are restored. That is correct -- a schema
-- change is not an entity change -- but it means a future synchronisation must read the
-- schema version, never the change feed, to notice that this happened.
--
-- Forward-only. V1-V11 are untouched.

PRAGMA defer_foreign_keys = ON;

-- Every column, including the one V7 added by ALTER TABLE. A holding table created with
-- CREATE TABLE AS carries no constraints, which is exactly what is wanted for something
-- that exists for four statements.
CREATE TABLE school_rebuild AS SELECT * FROM school;

DROP TABLE school;

CREATE TABLE school (
    id           TEXT    PRIMARY KEY NOT NULL,
    name         TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- Optional, and unique when given. The partial index below is the same reading of
    -- CLAUDE.md's COALESCE rule that ADR 0018 section 4 applied to `teacher.code`: a
    -- COALESCE index would fold every un-coded school onto one key and permit exactly one
    -- of them, which is the opposite of what "optional" means.
    code         TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    timezone     TEXT    NOT NULL CHECK (length(trim(timezone)) > 0),
    locale       TEXT    NOT NULL CHECK (length(trim(locale)) > 0),
    -- V7's column, preserved exactly, CHECK included.
    working_days INTEGER NOT NULL DEFAULT 31
                          CHECK (working_days BETWEEN 1 AND 127),
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Two schools with one name in one project would make every selector ambiguous. Names
    -- are the thing a user picks from, so this is the constraint that has to hold; the
    -- code is the optional convenience.
    UNIQUE (name)
) STRICT;

CREATE UNIQUE INDEX school_code_unique ON school (code) WHERE code IS NOT NULL;

-- Identifiers, revisions and timestamps are carried across verbatim. The existing school
-- is the same school afterwards: nothing it owns changes, no revision is consumed, and no
-- row anywhere else in the project is rewritten. `code` is NULL for it, because a project
-- that has only ever had one school was never asked for one.
INSERT INTO school (id, name, code, timezone, locale, working_days, created_at, updated_at, rev)
SELECT id, name, NULL, timezone, locale, working_days, created_at, updated_at, rev
FROM school_rebuild;

DROP TABLE school_rebuild;

-- ---------------------------------------------------------------------------------------
-- Cross-school referential integrity
-- ---------------------------------------------------------------------------------------
--
-- Every one of these rules was unenforceable-but-true until a moment ago: with one school
-- in the project, a reference could not point at another school's structures because there
-- was no other school. Now there can be, and a `student` whose `school_id` says one thing
-- and whose `year_level_id` belongs to another school is corruption that no screen would
-- reveal -- the student simply appears in the wrong year, or in no year, depending on which
-- query is asked.
--
-- Triggers rather than repository checks, for the reason V3, V6 and V9 all give: the rule
-- is stated where the storage is, so a future repository, the CLI, an import or a
-- hand-repair cannot produce a row that breaks it. The repositories validate too, because
-- a trigger produces "a student's year level must belong to the student's school" and the
-- interface needs to say which field is wrong -- but the trigger is what makes the
-- invariant true rather than merely intended.
--
-- Each table gets one trigger per operation holding every check for that table, rather
-- than one trigger per column. Fewer objects, and the whole rule for a table reads in one
-- place.
--
-- WHAT IS DELIBERATELY NOT CONSTRAINED: `building_campus` and `room_campus`. A facility
-- shared between campuses that belong to *different* schools is the whole point of
-- ADR 0013 and ADR 0014 -- one physical sports hall, one occupancy record, two
-- institutions timetabling into it. Those tables stay unconstrained by school on purpose,
-- and `room-clash` remains a per-`room_id` rule across the whole project.

-- A student's year level and campus must both belong to the student's school.
CREATE TRIGGER student_references_own_school_insert
BEFORE INSERT ON student FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a student''s year level must belong to the student''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a student''s campus must belong to the student''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER student_references_own_school_update
BEFORE UPDATE ON student FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a student''s year level must belong to the student''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a student''s campus must belong to the student''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- A group's year level, parent group and campus must all belong to the group's school.
-- The parent edge matters most: the hierarchy is what clash detection reads, and a cohort
-- in another school is not a parent, it is a different institution's audience.
CREATE TRIGGER student_group_references_own_school_insert
BEFORE INSERT ON student_group FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a group''s year level must belong to the group''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a group''s parent group must belong to the same school')
    WHERE NEW.parent_group_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM student_group parent
                      WHERE parent.id = NEW.parent_group_id
                        AND parent.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a group''s campus must belong to the group''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER student_group_references_own_school_update
BEFORE UPDATE ON student_group FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a group''s year level must belong to the group''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a group''s parent group must belong to the same school')
    WHERE NEW.parent_group_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM student_group parent
                      WHERE parent.id = NEW.parent_group_id
                        AND parent.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a group''s campus must belong to the group''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- A teacher's home campus must belong to the teacher's school. Stage 0A keeps a teacher
-- owned by one school; ADR 0021 will replace this with school membership, and this trigger
-- goes when that arrives.
CREATE TRIGGER teacher_references_own_school_insert
BEFORE INSERT ON teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s home campus must belong to the teacher''s school')
    WHERE NEW.home_campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.home_campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER teacher_references_own_school_update
BEFORE UPDATE ON teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s home campus must belong to the teacher''s school')
    WHERE NEW.home_campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.home_campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- A resource's campus must belong to the resource's school.
CREATE TRIGGER resource_references_own_school_insert
BEFORE INSERT ON resource FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a resource''s campus must belong to the resource''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER resource_references_own_school_update
BEFORE UPDATE ON resource FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a resource''s campus must belong to the resource''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- A bell schedule restricted to a campus must be restricted to one of its own school's.
CREATE TRIGGER period_structure_references_own_school_insert
BEFORE INSERT ON period_structure FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a bell schedule''s campus must belong to its school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER period_structure_references_own_school_update
BEFORE UPDATE ON period_structure FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a bell schedule''s campus must belong to its school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- A room's type is owned by a school; the room itself is owned by a campus, which is owned
-- by a school. Nothing previously required those two schools to be the same one.
CREATE TRIGGER room_type_must_share_school_insert
BEFORE INSERT ON room FOR EACH ROW
WHEN NEW.room_type_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s type must belong to the school that owns its campus')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN room_type ON room_type.id = NEW.room_type_id
        WHERE campus.id = NEW.campus_id
          AND room_type.school_id = campus.school_id
    );
END;

CREATE TRIGGER room_type_must_share_school_update
BEFORE UPDATE ON room FOR EACH ROW
WHEN NEW.room_type_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a room''s type must belong to the school that owns its campus')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN room_type ON room_type.id = NEW.room_type_id
        WHERE campus.id = NEW.campus_id
          AND room_type.school_id = campus.school_id
    );
END;

-- A calendar day names a real date in one school's calendar. Its term, campus and cycle
-- day must all be that school's, or the mapping ADR 0011 makes authoritative would be
-- reading another institution's cycle.
CREATE TRIGGER calendar_day_references_own_school_insert
BEFORE INSERT ON calendar_day FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a calendar day''s term must belong to the same school')
    WHERE NEW.term_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a calendar day''s campus must belong to the same school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a calendar day''s cycle day must belong to the same school')
    WHERE NEW.cycle_day_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM cycle_day
                      JOIN cycle ON cycle.id = cycle_day.cycle_id
                      WHERE cycle_day.id = NEW.cycle_day_id
                        AND cycle.school_id = NEW.school_id);
END;

CREATE TRIGGER calendar_day_references_own_school_update
BEFORE UPDATE ON calendar_day FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a calendar day''s term must belong to the same school')
    WHERE NEW.term_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a calendar day''s campus must belong to the same school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'a calendar day''s cycle day must belong to the same school')
    WHERE NEW.cycle_day_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM cycle_day
                      JOIN cycle ON cycle.id = cycle_day.cycle_id
                      WHERE cycle_day.id = NEW.cycle_day_id
                        AND cycle.school_id = NEW.school_id);
END;

-- A timeslot's school, cycle and campus are already held to agree with the cycle day and
-- period it was built from (V4). Its term was not checked, and now must be: a grid is one
-- school's, and so is the term that scopes it.
CREATE TRIGGER timeslot_term_must_share_school_insert
BEFORE INSERT ON timeslot FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a timeslot''s term must belong to the timeslot''s school')
    WHERE NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
END;

CREATE TRIGGER timeslot_term_must_share_school_update
BEFORE UPDATE ON timeslot FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a timeslot''s term must belong to the timeslot''s school')
    WHERE NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
END;

-- An availability rule names a school and a time. Every time reference it carries has to
-- be that school's. This is the rule that stops a teacher's "not Monday period 1" being
-- recorded against another school's Monday, which would read as a rule about nothing.
--
-- The rule's *subject* -- `entity_id` -- is NOT checked here. It is polymorphic and cannot
-- carry a foreign key (V11), the repository validates it on write, and `Availability::orphans`
-- sweeps for any that got through. Stage 0B's school membership is what will make a
-- shared teacher's subject checkable at all.
CREATE TRIGGER availability_references_own_school_insert
BEFORE INSERT ON availability FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an availability rule''s cycle day must belong to its school')
    WHERE NEW.cycle_day_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM cycle_day
                      JOIN cycle ON cycle.id = cycle_day.cycle_id
                      WHERE cycle_day.id = NEW.cycle_day_id
                        AND cycle.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s period must belong to its school')
    WHERE NEW.period_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM period
                      JOIN period_structure ON period_structure.id = period.period_structure_id
                      WHERE period.id = NEW.period_id
                        AND period_structure.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s timeslot must belong to its school')
    WHERE NEW.timeslot_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM timeslot
                      WHERE timeslot.id = NEW.timeslot_id
                        AND timeslot.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s term must belong to its school')
    WHERE NEW.term_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
END;

CREATE TRIGGER availability_references_own_school_update
BEFORE UPDATE ON availability FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an availability rule''s cycle day must belong to its school')
    WHERE NEW.cycle_day_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM cycle_day
                      JOIN cycle ON cycle.id = cycle_day.cycle_id
                      WHERE cycle_day.id = NEW.cycle_day_id
                        AND cycle.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s period must belong to its school')
    WHERE NEW.period_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM period
                      JOIN period_structure ON period_structure.id = period.period_structure_id
                      WHERE period.id = NEW.period_id
                        AND period_structure.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s timeslot must belong to its school')
    WHERE NEW.timeslot_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM timeslot
                      WHERE timeslot.id = NEW.timeslot_id
                        AND timeslot.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an availability rule''s term must belong to its school')
    WHERE NEW.term_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM term
                      JOIN academic_year ON academic_year.id = term.academic_year_id
                      WHERE term.id = NEW.term_id
                        AND academic_year.school_id = NEW.school_id);
END;
