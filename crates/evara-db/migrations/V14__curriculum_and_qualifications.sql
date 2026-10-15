-- Curriculum prerequisites: departments, subjects, and what a teacher is qualified and
-- organised to teach.
--
-- This is the migration DATA-MODEL.md §4 has been promising since V10, which said of it:
-- "`department`, `subject` and `teacher_qualification` are documented alongside these
-- tables but belong with Curriculum, and arrive with it." They arrive here, one stage
-- ahead of the lessons that read them, because Stage C's `course` references a subject and
-- cannot be designed around a table that does not exist.
--
-- ADR 0025 and ADR 0026 record the two decisions that are not obvious. Read them before
-- changing anything below.
--
-- ---------------------------------------------------------------------------------------
-- Four tables, and what each one is ABOUT
-- ---------------------------------------------------------------------------------------
--
--   department             an organisational unit INSIDE one school      (school-owned)
--   subject                something this school teaches                 (school-owned)
--   teacher_qualification  a property of a PERSON, naming one subject     (project-level)
--   teacher_department     a NARROWING of school membership              (per school)
--
-- The split in the right-hand column is the whole of ADR 0025, and getting it wrong is the
-- one mistake this migration exists to make impossible:
--
--   * A QUALIFICATION is a fact about a human being. "She can teach A-level Chemistry" is
--     true whether or not she is currently employed anywhere, and it survives her leaving.
--     So `teacher_qualification` has NO school_id and NO membership requirement, and
--     removing a school membership does not touch it.
--
--   * A DEPARTMENT ASSOCIATION is a fact about an institution's internal organisation.
--     "He is in the Science department" is a statement about one school and is meaningless
--     away from it. So `teacher_department` is constrained to the departments of schools
--     the teacher belongs to, exactly as V13 constrained `teacher_campus` -- and, like
--     eligibility, it survives a DEACTIVATED membership and goes with a REMOVED one.
--
-- V13's header note applies unchanged to both new relationship tables: each is a full
-- entity with its own UUIDv7 `id`, its own audit columns and its own change-log triggers,
-- rather than a bare composite-key junction. The triggers read NEW.id and NEW.rev, and
-- repo::update / repo::delete address rows by id, so a composite-key table would sit
-- outside the change feed. Qualifying a teacher is a real edit to a real project and
-- belongs in the feed like any other. All four tables are added to
-- change_log::TRACKED_TABLES, and change_log::install_all creates their triggers on the
-- next open.
--
-- ---------------------------------------------------------------------------------------
-- Why there is no `teacher.department_id`
-- ---------------------------------------------------------------------------------------
--
-- V10's header reserved it: "`teacher.department_id` is the one column that implies, and
-- adding it later is ALTER TABLE ... ADD COLUMN against a nullable reference -- the cheap
-- direction." The column is cheap. It is also WRONG, and V13 is what made it wrong.
--
-- A teacher is a project-level identity that may be employed by several schools. One
-- column on that identity cannot say "Mathematics at Northgate, Science at Southmoor",
-- which is an ordinary arrangement in exactly the trust V13 exists to serve. It would
-- force either a second teacher record -- the duplication V13 removed -- or an arbitrary
-- choice of which school's answer to keep.
--
-- The multi-school audit's own proposal, `teacher_school.department_id`, fixes the scope
-- and not the cardinality: a teacher in both Mathematics and Physics at one school is just
-- as ordinary, and a single column cannot say that either. So the association is its own
-- table, and a teacher may hold as many as the school's organisation actually has. One
-- association is the degenerate case of that, never the other way round.
--
-- ---------------------------------------------------------------------------------------
-- Deletion, chosen rather than defaulted
-- ---------------------------------------------------------------------------------------
--
-- ADR 0010's rule is that ownership edges are RESTRICT and optional classification
-- references are SET NULL. `subject.department_id` is shaped like the second -- it is
-- optional, and a subject exists perfectly well without a department -- and is
-- nevertheless RESTRICT. ADR 0025 §6 argues it; the short version is R10 in
-- OPEN-DECISIONS: a database-level SET NULL rewrites a row without incrementing its `rev`,
-- which silently bypasses the revision contract this very migration's stage exists to
-- enforce. Refusing is also what a user wants from a department that is still in use, and
-- the repository asks the question first so the refusal arrives as an explanation with
-- counts in it rather than as a failed button press.
--
-- Everything else is RESTRICT for the reasons ADR 0010 already gives, and the repository
-- clears what it means to clear inside the same transaction through repo::delete, so each
-- removal is a truthful change-log entry rather than a silent orphan.
--
-- ---------------------------------------------------------------------------------------
-- Additive, with nothing invented
-- ---------------------------------------------------------------------------------------
--
-- Four CREATE TABLEs, their indexes, and trigger work. No table is rebuilt, no existing
-- row is rewritten, no revision is consumed, and there is NO BACKFILL -- a project that
-- has never been asked what it teaches has no departments and no subjects, and inventing a
-- "General" department would be putting words in a school's mouth.
--
-- `backup::FIRST_REBUILDING_MIGRATION` stays at 12 for the reason V13 left it there: a
-- project already at or past 12 needs no safety copy to reach 14, and a project below it
-- still gets one because it is about to cross 12 on the way here.
--
-- Forward-only. V1-V13 are untouched.

-- =======================================================================================
-- department -- an organisational unit inside one school
-- =======================================================================================
--
-- Never shared. Two schools in one campus file may each hold a "Mathematics Department",
-- as two rows with two identifiers, and neither sees the other's -- the same treatment
-- `year_level` gets, and for the same reason: this is a statement about how one
-- institution organises itself.
CREATE TABLE department (
    id          TEXT    PRIMARY KEY NOT NULL,
    -- Ownership. RESTRICT, so `delete_school` keeps the promise ADR 0019 made: a school
    -- that still describes a curriculum is refused and explained, never quietly emptied.
    school_id   TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- Optional short reference. Unique when given; see department_code_unique.
    code        TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    description TEXT             CHECK (description IS NULL OR length(trim(description)) > 0),
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    rev         INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Within the school, and only within it.
    UNIQUE (school_id, name)
) STRICT;

-- A partial index rather than a COALESCE one, which is the documented exception CLAUDE.md
-- points at: the COALESCE rule exists for unset rows that must still collide, and here a
-- school that does not code its departments must be able to enter more than one. Same
-- shape as teacher_code_unique (V10) and school_code_unique (V12).
CREATE UNIQUE INDEX department_code_unique
    ON department (school_id, code) WHERE code IS NOT NULL;

CREATE INDEX department_by_name ON department (school_id, name);

-- =======================================================================================
-- subject -- something this school teaches
-- =======================================================================================
--
-- School-owned, like the department. Northgate's Mathematics and Southmoor's Mathematics
-- are two subjects with two identifiers, and they may share a name and a code without
-- colliding, because the uniqueness rules are scoped to the school. Nothing merges them,
-- and nothing should: they may have different departments, different room requirements and
-- different courses, and a project-wide subject catalogue would have to choose one
-- school's answer for all of them.
--
-- DATA-MODEL.md §5 sketches `colour` and `default_room_type_id` on this table. Both are
-- deliberately absent: a colour is a timetable-rendering decision that belongs with the
-- renderer in Phase 7, and a default room type is a scheduling default that belongs with
-- `course.required_room_type_id` in Stage C, where there is something to default. Adding
-- either is one nullable ALTER TABLE, which is the cheap direction, and guessing now would
-- ship two columns no screen can explain.
CREATE TABLE subject (
    id            TEXT    PRIMARY KEY NOT NULL,
    school_id     TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- Optional, and restricted to a department of THIS school by the trigger pair below.
    --
    -- RESTRICT rather than ADR 0010's SET NULL for classification. See this file's header
    -- and ADR 0025 §6: a SET NULL here would unclassify every subject of a deleted
    -- department without bumping a single `rev`, which is R10's failure exactly, and
    -- refusing is also the behaviour a school wants from a unit that is still in use.
    department_id TEXT             REFERENCES department (id) ON DELETE RESTRICT,
    name          TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code          TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    description   TEXT             CHECK (description IS NULL OR length(trim(description)) > 0),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (school_id, name)
) STRICT;

CREATE UNIQUE INDEX subject_code_unique
    ON subject (school_id, code) WHERE code IS NOT NULL;

CREATE INDEX subject_by_name ON subject (school_id, name);

-- Every RESTRICT delete scans the children to decide whether to refuse, and this is also
-- the index the "which subjects are in this department?" read uses.
CREATE INDEX idx_subject_department ON subject (department_id);

-- A subject's department must belong to the subject's own school.
--
-- The same trigger-pair shape V12 used for every cross-school reference it could not
-- otherwise have prevented, and V13 used for campus eligibility. The rule is stated where
-- the storage is, so a future repository, the CLI, an import or a hand-repair cannot write
-- a row that breaks it. The repository checks it first as well, because a trigger cannot
-- say which field is wrong -- and cannot say "that department belongs to another school",
-- which is the one mistake a department picker left open on a stale screen would produce.
CREATE TRIGGER subject_department_same_school_insert
BEFORE INSERT ON subject FOR EACH ROW
WHEN NEW.department_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a subject''s department must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1 FROM department
        WHERE department.id = NEW.department_id
          AND department.school_id = NEW.school_id
    );
END;

CREATE TRIGGER subject_department_same_school_update
BEFORE UPDATE ON subject FOR EACH ROW
WHEN NEW.department_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a subject''s department must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1 FROM department
        WHERE department.id = NEW.department_id
          AND department.school_id = NEW.school_id
    );
END;

-- =======================================================================================
-- teacher_qualification -- what a person can teach
-- =======================================================================================
--
-- PROJECT-LEVEL, and the one place where a project-wide identity and a school-owned entity
-- genuinely collide. The multi-school audit flagged it as open question 6 and recommended
-- exactly this resolution: the qualification is a property of the person, and the SUBJECT
-- it names is what supplies the school.
--
-- So a teacher shared between two schools holds two rows to teach "Mathematics" at both --
-- one per subject record -- and that is correct rather than redundant. The two subjects are
-- different curricula owned by different institutions, and a school may perfectly well
-- qualify her for its own Mathematics and not for its Further Mathematics.
--
-- NO school_id COLUMN, and NO MEMBERSHIP REQUIREMENT. Both are deliberate and are the
-- substance of ADR 0025:
--
--   * A second school_id would be a second place to record what `subject.school_id`
--     already says -- CLAUDE.md's rule against resolving a child's school any way but
--     through its parent chain.
--   * Requiring membership would make a qualification expire when employment did, which is
--     the opposite of what a qualification is. A teacher who leaves Northgate does not
--     become unqualified to teach Northgate's Chemistry, and a school re-employing her
--     later should not have to re-enter what it already knew. Whether she may be GIVEN a
--     lesson is a different question, answered in Stage C by qualification AND current
--     membership AND campus eligibility together.
CREATE TABLE teacher_qualification (
    id         TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception.
    --
    -- The teacher edge: `delete_teacher` removes these rows itself, in its own
    -- transaction and through repo::delete, so each removal is a truthful change-log
    -- entry. Destroying the person is the one operation that may take them, because a
    -- qualification with no-one to hold it is not history, it is a dangling fact.
    --
    -- The subject edge: `delete_subject` is REFUSED while anybody is qualified for it,
    -- rather than silently discarding what a school recorded about its staff. The remedy
    -- is visible and belongs to the user -- remove the qualifications, then the subject.
    teacher_id TEXT    NOT NULL REFERENCES teacher (id) ON DELETE RESTRICT,
    subject_id TEXT    NOT NULL REFERENCES subject (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Both columns are NOT NULL, so no COALESCE is needed for this to actually hold.
    -- One qualification per (person, subject): saying it twice says nothing more.
    UNIQUE (teacher_id, subject_id)
) STRICT;

-- teacher_id is covered by the UNIQUE index above; subject_id needs its own, both for the
-- RESTRICT scan and for "who can teach this?", which is a question the Subjects screen
-- asks per row.
CREATE INDEX idx_teacher_qualification_subject ON teacher_qualification (subject_id);

-- =======================================================================================
-- teacher_department -- which of a school's units a teacher belongs to
-- =======================================================================================
--
-- A NARROWING OF MEMBERSHIP, modelled on V13's `teacher_campus` down to the trigger names.
-- Everything V13 said about eligibility applies here: the drift it forbids -- a teacher in
-- a department of a school they do not belong to -- is rejected by the database rather than
-- by convention, and the repository checks it first so the message can name the remedy.
--
-- The school is reached through `department.school_id`. There is no second school_id, for
-- the same reason the qualification has none.
CREATE TABLE teacher_department (
    id            TEXT    PRIMARY KEY NOT NULL,
    teacher_id    TEXT    NOT NULL REFERENCES teacher (id)    ON DELETE RESTRICT,
    department_id TEXT    NOT NULL REFERENCES department (id) ON DELETE RESTRICT,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- A teacher may be in several departments of one school -- Mathematics and Physics is
    -- ordinary -- so the uniqueness is per (person, department), not per (person, school).
    UNIQUE (teacher_id, department_id)
) STRICT;

CREATE INDEX idx_teacher_department_department ON teacher_department (department_id);

-- A teacher's department must belong to a school they are a member of.
--
-- Note what is NOT required, exactly as in V13: that the membership be *active*. A teacher
-- who has left Southmoor keeps the record of which Southmoor departments they worked in,
-- the way they keep which campuses they worked at. Removing the membership outright is what
-- clears it, and the repository does that in one transaction.
CREATE TRIGGER teacher_department_requires_school_membership_insert
BEFORE INSERT ON teacher_department FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s department must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1
        FROM department
        JOIN teacher_school ts ON ts.school_id = department.school_id
        WHERE department.id = NEW.department_id
          AND ts.teacher_id = NEW.teacher_id
    );
END;

-- Nothing in the repository updates an association row -- assign and remove are an insert
-- and a delete -- so this should never fire. It exists so the invariant does not depend on
-- that staying true. Same reasoning as V13's `teacher_campus_..._update` and V6's
-- `room_campus_..._update`.
CREATE TRIGGER teacher_department_requires_school_membership_update
BEFORE UPDATE ON teacher_department FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s department must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1
        FROM department
        JOIN teacher_school ts ON ts.school_id = department.school_id
        WHERE department.id = NEW.department_id
          AND ts.teacher_id = NEW.teacher_id
    );
END;

-- The other direction: removing a membership must not leave an association asserting it.
--
-- The repository clears the dependent `teacher_department` rows in the same transaction,
-- through repo::delete, so each removal is a truthful change-log entry. This trigger is the
-- backstop for a delete that bypasses it, and it refuses rather than cascading: a
-- membership removal that silently took department associations with it would leave no
-- change-log evidence that they had ever existed.
--
-- A teacher's QUALIFICATIONS are deliberately absent from this rule. They are not a
-- narrowing of membership and they do not go when it does; see this file's header.
CREATE TRIGGER teacher_school_delete_requires_no_department_association
BEFORE DELETE ON teacher_school FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'remove the teacher''s departments in this school before the membership')
    WHERE EXISTS (
        SELECT 1
        FROM teacher_department td
        JOIN department ON department.id = td.department_id
        WHERE td.teacher_id = OLD.teacher_id
          AND department.school_id = OLD.school_id
    );
END;
