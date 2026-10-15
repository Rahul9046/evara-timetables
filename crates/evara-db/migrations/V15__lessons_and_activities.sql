-- Lessons: what a school teaches to whom, how often, and for how long at a time.
--
-- V14 stopped at the prerequisites and said why: "Stage C's `course` references a subject
-- and cannot be designed around a table that does not exist." This is Stage C. The subject
-- exists, and six tables turn it into scheduling demand.
--
-- ADR 0028 and ADR 0029 record the decisions that are not obvious -- who may be given a
-- lesson, and what shape a lesson's demand has. Read both before changing anything below.
--
-- ---------------------------------------------------------------------------------------
-- Six tables, and the one sentence each is about
-- ---------------------------------------------------------------------------------------
--
--   course          a school's offering of one subject           "Year 9 Mathematics"
--   class_section   one deliverable instance of that offering    "Year 9 Mathematics A"
--   class_teacher   who teaches that instance                    many per section
--   class_group     who attends it                               many per section
--   class_enrolment which named individuals attend it            optional, a roster
--   activity        ONE placeable unit of work                   "a single period of it"
--
-- The chain is `subject -> course -> class_section -> activity`, and every link is an
-- ownership edge. Nothing in it carries a second `school_id`.
--
-- ---------------------------------------------------------------------------------------
-- Why there is no `course.school_id`
-- ---------------------------------------------------------------------------------------
--
-- DATA-MODEL.md's sketch of this group wrote `course (id, school_id, subject_id, ...)`.
-- That is corrected rather than implemented, and ROADMAP.md's Phase 3 notes are where the
-- correction was already written down:
--
--   "A subject is school-owned, and `course` must be too. Two schools' 'Mathematics' are
--    two records; a course referencing one is a course of that school. Resolve the school
--    through `subject.school_id` rather than adding a second `school_id` to `course` --
--    the same rule V14's own tables follow."
--
-- It is also CLAUDE.md's rule in general terms: a child's school is resolved through its
-- parent chain, never by adding it a second `school_id` column. A second place to record
-- the same ownership is a second place for it to be wrong, and the chain is already
-- enforced by foreign keys. `repo::require_school_owner` exists for exactly this, and
-- `lessons.rs` writes one owner query per table -- the chain IS the domain statement.
--
-- The consequence is that every rule below reaching for "this section's school" joins
-- class_section -> course -> subject. That join appears in five triggers. It is written out
-- each time rather than hidden behind a view, because a trigger reading a view would make
-- the invariant depend on the view surviving, and because the join is three tables long and
-- says what it means.
--
-- ---------------------------------------------------------------------------------------
-- An activity is ONE occurrence, and a cycle's worth of them is several rows
-- ---------------------------------------------------------------------------------------
--
-- This is the decision ADR 0029 exists for, and getting it the other way round is the one
-- mistake this file is shaped to prevent.
--
-- A class section needing four single periods a cycle holds FOUR activity rows. One needing
-- a double period once a cycle holds ONE row with `duration_periods = 2`. There is no
-- `occurrences_per_cycle` column: the occurrence count is how many rows there are, and the
-- total demand is SUM(duration_periods).
--
-- ROADMAP.md's Phase 3 work list asks for exactly this -- "expand a course's
-- `periods_per_cycle` into activities with `split_index`, respecting
-- `default_duration_periods`" -- and three things depend on it:
--
--   * One activity is one placeable unit, so the solver's variable set IS the activity
--     rows. A column saying "and three more like me" would make a row mean a variable
--     sometimes and a template other times.
--   * DATA-MODEL.md §7's `timetable_entry` is unique on
--     (timetable_id, activity_id, timeslot_id) because a MULTI-PERIOD activity occupies
--     several slots. Four occurrences of one row would need a fifth column in that key to
--     tell them apart, and every constraint reading an audience would have to carry it.
--   * The most common real constraint in this product category is "spread these four
--     Mathematics periods across four different days". That rule addresses the occurrences
--     individually. They have to BE individual rows for it to have anything to point at.
--
-- `duration_periods` is consecutive periods within one occurrence -- D6 in OPEN-DECISIONS,
-- one activity rather than N linked ones -- and is not to be confused with a teaching load.
--
-- ---------------------------------------------------------------------------------------
-- What this file deliberately does NOT add
-- ---------------------------------------------------------------------------------------
--
-- DATA-MODEL.md §5 and §7 sketch more than this. Each omission is a decision, argued in
-- ADR 0029 §5, and each is one nullable ALTER TABLE or one new table away -- the cheap
-- direction, and the same judgement V14 made about `subject.colour`:
--
--   activity.school_id, activity.kind    an activity with NO class section (an assembly, a
--                                        meeting, a yard duty) is the only thing that needs
--                                        either, and Stage C does not model one. Widening
--                                        this table for them is an ADR 0020 rebuild, which
--                                        V12 has already shown how to do.
--   activity_teacher, activity_group     Stage C's activities inherit both from their class
--                                        section. Two places to say who teaches a lesson is
--                                        two places for it to disagree.
--   activity_room_candidate,             room ALLOCATION is Phase 6. A requirement is not an
--   activity.preferred_room_id,          assignment, and `course.required_room_type_id`
--   activity_resource                    below is the requirement.
--   class_teacher.role                   PRIMARY/CO/ASSISTANT with no reader, and the
--                                        sketch's (section, teacher, role) key would let
--                                        one person be a section's PRIMARY and its
--                                        ASSISTANT at once.
--   class_section.max_size               ADR 0006 asks for the SHAPE of class_section and
--   course_request, line, line_member,   class_enrolment to exist before populated
--   activity_link                        projects need migrating, and that is what this
--                                        file delivers. The columns and tables above them
--                                        have no reader in Milestone 1.
--   class_enrolment.status               a roster row either exists or it does not; see
--                                        that table's header.
--
-- ---------------------------------------------------------------------------------------
-- Deletion, chosen rather than defaulted
-- ---------------------------------------------------------------------------------------
--
-- ADR 0010's rule is that ownership edges are RESTRICT and optional classification
-- references are SET NULL, cleared by the repository FIRST through a proper rev-bumping
-- update so the schema's rule is a backstop that never fires. Both halves are used here,
-- and which an edge is was decided one at a time:
--
--   RESTRICT, ownership        course.subject_id, class_section.course_id,
--                              class_section.campus_id, every class_* and activity edge to
--                              its class_section, class_teacher.teacher_id,
--                              class_group.student_group_id, class_enrolment.student_id
--   SET NULL, classification   course.year_level_id, course.required_room_type_id
--
-- The two SET NULL edges follow `room.room_type_id` and `student_group.year_level_id`
-- exactly: `delete_year_level` and `delete_room_type` already clear their referring rows
-- one at a time through repo::update, and both gain `course` in this stage. They are NOT
-- treated the way V14 treated `subject.department_id`. ADR 0025 §6 departed from ADR 0010
-- there on two grounds, and only the first applies here -- R10's "a database-level SET NULL
-- rewrites rows without incrementing rev" is answered by the repository clearing them
-- first, which is the existing mechanism for these two exact columns. The second ground was
-- that refusing is what a school WANTS from a department still in use; it does not carry
-- over, because a course outlives losing its year level perfectly well. The year level is
-- optional on a course, and the remaining course is still a complete record.
--
-- Where RESTRICT would merely be hostile, the repository clears the dependents itself
-- inside the delete's own transaction, through repo::delete, so each removal is a truthful
-- change-log entry rather than a silent orphan. `delete_class_section` does that for its
-- teachers, groups, enrolments and activities, which is `delete_teacher`'s treatment of
-- qualifications and ADR 0025 §5's reasoning: a class_teacher row with no class section is
-- not history, it is a dangling fact. `delete_course` does NOT -- a class section is a
-- substantial thing a school built, with staff and a roster on it, so that one is refused
-- with counts, which is `delete_department`'s treatment of subjects.
--
-- ---------------------------------------------------------------------------------------
-- Additive, with nothing invented
-- ---------------------------------------------------------------------------------------
--
-- Six CREATE TABLEs, their indexes, and trigger work. No table is rebuilt, no existing row
-- is rewritten, no revision is consumed, and there is NO BACKFILL. A project that has never
-- been asked what it teaches has no courses, and inventing one course per subject would be
-- putting words in a school's mouth -- it would also be wrong, because a subject with no
-- course is the ordinary state of a school part-way through setup.
--
-- `backup::FIRST_REBUILDING_MIGRATION` stays at 12, for the reason V13 and V14 left it
-- there: a project already at or past 12 needs no safety copy to reach 15, and a project
-- below it still gets one because it is about to cross 12 on the way here.
--
-- All six tables are added to change_log::TRACKED_TABLES, and change_log::install_all
-- creates their triggers on the next open.
--
-- Forward-only. V1-V14 are untouched.

-- =======================================================================================
-- course -- one school's offering of one subject
-- =======================================================================================
--
-- "Year 9 Mathematics" is a course; "Mathematics" is the subject it is an offering of. The
-- distinction earns its keep the moment a school teaches the same subject at three year
-- levels with different period allocations, which is every school.
CREATE TABLE course (
    id                      TEXT    PRIMARY KEY NOT NULL,
    -- What is taught, and the ONLY source of this course's school. RESTRICT, continuing
    -- ADR 0010 without exception: `delete_subject` is refused while any course is an
    -- offering of it, and `Curriculum::subject_dependents` gains the count so the
    -- confirmation explains before the user presses anything.
    subject_id              TEXT    NOT NULL REFERENCES subject (id) ON DELETE RESTRICT,
    -- Who it is for. Optional, because a course may be open to several year levels or to
    -- none in particular, and restricted to a year level of THIS course's school by the
    -- trigger pair below.
    --
    -- SET NULL, matching `student_group.year_level_id` and `student.year_level_id` from
    -- V10 and `student_school_enrolment.year_level_id` from V13. `delete_year_level`
    -- clears every one of them through repo::update first, so each affected row's revision
    -- stays truthful and this rule never actually fires. See this file's header.
    year_level_id           TEXT             REFERENCES year_level (id) ON DELETE SET NULL,
    -- When it runs. NULL means all year, which is the common case and the reason this is
    -- nullable rather than defaulted to the first term.
    --
    -- RESTRICT rather than SET NULL, unlike the two columns either side of it: a term is
    -- not a classification of a course, it is the stretch of the year the course exists in,
    -- and silently widening a one-term course to the whole year is a different course.
    -- `delete_term` refuses while anything refers to a term already.
    term_id                 TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    name                    TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    -- Optional short reference, unique within the SCHOOL rather than within the subject --
    -- see course_code_unique_in_school_insert for why that needs a trigger here.
    code                    TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    description             TEXT             CHECK (description IS NULL OR length(trim(description)) > 0),
    -- How many period slots ONE section of this course needs per cycle: the declared
    -- demand, which the activity generator expands into rows.
    --
    -- Nullable, because "we have not decided yet" is a real state for a school part-way
    -- through setup and a form that insisted would be answered with a guess. The generator
    -- refuses while it is NULL and says so, rather than inventing a number.
    --
    -- This is a DECLARATION, and `SUM(activity.duration_periods)` over a section is the
    -- ACTUAL demand. The two are allowed to disagree -- a section may legitimately need
    -- more or less than its course's default -- and the interface reports the difference as
    -- advice rather than refusing it. ADR 0029 §3.
    periods_per_cycle       INTEGER          CHECK (periods_per_cycle IS NULL OR periods_per_cycle >= 1),
    -- How long one of those occurrences is, by default, when the generator expands them.
    -- A course of 4 periods per cycle at duration 1 is four single periods; at duration 2
    -- it is two doubles. NOT NULL with a default of 1, because "one period" is the answer
    -- for almost every course and a nullable version of it would mean the same thing.
    default_duration_periods INTEGER NOT NULL DEFAULT 1 CHECK (default_duration_periods >= 1),
    -- What kind of room a lesson of this course needs. A REQUIREMENT, not an assignment:
    -- nothing here books a room, and room allocation is Phase 6. Restricted to a room type
    -- of this course's school by the trigger pair below.
    --
    -- SET NULL for the same reason as `room.room_type_id`, which this mirrors exactly, and
    -- `delete_room_type` clears it first through repo::update.
    required_room_type_id   TEXT             REFERENCES room_type (id) ON DELETE SET NULL,
    created_at              TEXT    NOT NULL,
    updated_at              TEXT    NOT NULL,
    rev                     INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Within the subject, not within the school. Two offerings of Mathematics may not both
    -- be called "Year 9"; a Mathematics course and a Physics course may both be called
    -- "Core", and refusing that would be refusing a true statement. The subject is always
    -- present wherever a course name is shown, so the pair is what a reader actually sees.
    UNIQUE (subject_id, name)
) STRICT;

-- Every RESTRICT delete scans the children to decide whether to refuse, and these are also
-- the "which courses are in this school / of this subject?" reads.
CREATE INDEX idx_course_subject    ON course (subject_id);
CREATE INDEX idx_course_year_level ON course (year_level_id);
CREATE INDEX idx_course_term       ON course (term_id);
CREATE INDEX idx_course_room_type  ON course (required_room_type_id);

-- A course code identifies one course within one SCHOOL.
--
-- Not a partial UNIQUE index, which is what V10's `teacher_code_unique`, V12's
-- `school_code_unique` and V14's `subject_code_unique` all are, because those tables name
-- their school and this one deliberately does not. The scope a code needs is the school; the
-- column that would express it is the one this file argues against adding. So the rule is a
-- trigger pair, which can reach the school through the subject.
--
-- Unique when given, unconstrained when absent -- CLAUDE.md's documented exception to the
-- COALESCE rule, for the reason V10 states at length: COALESCE(code, '') would make every
-- uncoded course collide with every other, so a school that does not code its courses could
-- enter exactly one.
--
-- The repository checks this first as well, because a trigger cannot say WHICH field is
-- wrong and `repo::translate` reads SQLite's own constraint text rather than a RAISE
-- message. V14 established that division: the repository names the field and the remedy,
-- the trigger is what makes the invariant true for a future repository, the CLI, an import
-- or a hand-repair.
CREATE TRIGGER course_code_unique_in_school_insert
BEFORE INSERT ON course FOR EACH ROW
WHEN NEW.code IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'another course of this school already uses that code')
    WHERE EXISTS (
        SELECT 1
        FROM course other
        JOIN subject other_subject ON other_subject.id = other.subject_id
        WHERE other.code = NEW.code
          AND other.id <> NEW.id
          AND other_subject.school_id = (
              SELECT school_id FROM subject WHERE subject.id = NEW.subject_id
          )
    );
END;

CREATE TRIGGER course_code_unique_in_school_update
BEFORE UPDATE ON course FOR EACH ROW
WHEN NEW.code IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'another course of this school already uses that code')
    WHERE EXISTS (
        SELECT 1
        FROM course other
        JOIN subject other_subject ON other_subject.id = other.subject_id
        WHERE other.code = NEW.code
          AND other.id <> NEW.id
          AND other_subject.school_id = (
              SELECT school_id FROM subject WHERE subject.id = NEW.subject_id
          )
    );
END;

-- A course's year level must belong to the course's own school.
--
-- The same trigger-pair shape V12 used for every cross-school reference it could not
-- otherwise prevent, V13 used for campus eligibility and V14 used for a subject's
-- department. The school is reached through the subject, which is the whole of this file's
-- ownership argument made concrete.
CREATE TRIGGER course_year_level_same_school_insert
BEFORE INSERT ON course FOR EACH ROW
WHEN NEW.year_level_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s year level must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM year_level
        JOIN subject ON subject.id = NEW.subject_id
        WHERE year_level.id = NEW.year_level_id
          AND year_level.school_id = subject.school_id
    );
END;

CREATE TRIGGER course_year_level_same_school_update
BEFORE UPDATE ON course FOR EACH ROW
WHEN NEW.year_level_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s year level must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM year_level
        JOIN subject ON subject.id = NEW.subject_id
        WHERE year_level.id = NEW.year_level_id
          AND year_level.school_id = subject.school_id
    );
END;

-- A course's required room type must belong to the course's own school.
--
-- Room types are school-owned (`room_type.school_id`), unlike the ROOMS they classify,
-- which ADR 0014 lets several schools share through `room_campus`. So this is a plain
-- same-school rule and not a sharing-eligibility one: a school may require "Laboratory" as
-- its own curriculum decision, and which physical rooms satisfy it is Phase 6's question.
CREATE TRIGGER course_room_type_same_school_insert
BEFORE INSERT ON course FOR EACH ROW
WHEN NEW.required_room_type_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s required room type must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM room_type
        JOIN subject ON subject.id = NEW.subject_id
        WHERE room_type.id = NEW.required_room_type_id
          AND room_type.school_id = subject.school_id
    );
END;

CREATE TRIGGER course_room_type_same_school_update
BEFORE UPDATE ON course FOR EACH ROW
WHEN NEW.required_room_type_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s required room type must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM room_type
        JOIN subject ON subject.id = NEW.subject_id
        WHERE room_type.id = NEW.required_room_type_id
          AND room_type.school_id = subject.school_id
    );
END;

-- A course's term must belong to the course's own school.
--
-- Reached through `term -> academic_year -> school`, which is the chain
-- `TimeModel::update_term` already walks for the same reason.
CREATE TRIGGER course_term_same_school_insert
BEFORE INSERT ON course FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s term must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        JOIN subject ON subject.id = NEW.subject_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = subject.school_id
    );
END;

CREATE TRIGGER course_term_same_school_update
BEFORE UPDATE ON course FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a course''s term must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        JOIN subject ON subject.id = NEW.subject_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = subject.school_id
    );
END;

-- =======================================================================================
-- class_section -- one deliverable instance of a course
-- =======================================================================================
--
-- "Year 9 Mathematics A" and "Year 9 Mathematics B" are two sections of one course: same
-- curriculum, different staff and different children. Nothing assumes a course has exactly
-- one, and nothing assumes a section has exactly one teacher -- co-teaching and combined
-- cohorts are ordinary, and DATA-MODEL.md §7 names the Java prototype's single `teacherId`
-- as the shortcut to avoid.
--
-- `class_section.id` is the durable identity, and it is what Phase 6's timetable and
-- Stage D's imports will both address. Names and codes may change without breaking a
-- reference, which is the whole point of UUIDv7 keys that are never renumbered.
CREATE TABLE class_section (
    id         TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT. `delete_course` is refused while any section exists, with counts, rather
    -- than destroying the staffing and roster a school entered -- see this file's header.
    course_id  TEXT    NOT NULL REFERENCES course (id) ON DELETE RESTRICT,
    name       TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    code       TEXT             CHECK (code IS NULL OR length(trim(code)) > 0),
    -- Where it is taught, when that is decided. Optional, and it is what makes the campus
    -- half of ADR 0028's assignment rule mean anything: a teacher narrowed to the junior
    -- campus may not be given a section AT the senior one. A section with no campus is not
    -- campus-constrained, and the narrowing has nothing to bite on.
    --
    -- RESTRICT, so "delete a campus" means the same thing on every edge rather than
    -- mostly-refuse-but-sometimes-detach. ADR 0010 made that call for `resource.campus_id`,
    -- which is optional in exactly the same way.
    campus_id  TEXT             REFERENCES campus (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (course_id, name)
) STRICT;

-- Within the course, deliberately, and NOT within the school as a course code is.
--
-- The two codes are read differently. A course code is quoted on its own -- "9MAT" appears
-- in a report with nothing around it -- so it has to identify a course across the school. A
-- section code is read in the context of its course: "9MAT / A". Making "A" unique across
-- the school would refuse "A" for Mathematics and "A" for Physics, which is the normal way
-- schools name sections.
CREATE UNIQUE INDEX class_section_code_unique
    ON class_section (course_id, code) WHERE code IS NOT NULL;

CREATE INDEX idx_class_section_course ON class_section (course_id);
CREATE INDEX idx_class_section_campus ON class_section (campus_id);

-- A section's campus must belong to the section's own school.
CREATE TRIGGER class_section_campus_same_school_insert
BEFORE INSERT ON class_section FOR EACH ROW
WHEN NEW.campus_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a class section''s campus must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN course ON course.id = NEW.course_id
        JOIN subject ON subject.id = course.subject_id
        WHERE campus.id = NEW.campus_id
          AND campus.school_id = subject.school_id
    );
END;

CREATE TRIGGER class_section_campus_same_school_update
BEFORE UPDATE ON class_section FOR EACH ROW
WHEN NEW.campus_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a class section''s campus must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN course ON course.id = NEW.course_id
        JOIN subject ON subject.id = course.subject_id
        WHERE campus.id = NEW.campus_id
          AND campus.school_id = subject.school_id
    );
END;

-- =======================================================================================
-- class_teacher -- who teaches one section
-- =======================================================================================
--
-- Many-to-many from day one. DATA-MODEL.md §7's reasoning applies to the section as much as
-- to the activity: co-teaching is normal and retro-fitting it is expensive.
--
-- The sketch's primary key is (class_section_id, teacher_id, role) with role in
-- PRIMARY/CO/ASSISTANT. Both halves are departed from. `role` has no reader in Milestone 1
-- and that key would let one person be a section's PRIMARY and its ASSISTANT at once, which
-- is not a staffing arrangement, it is two contradictory rows. One row per (section,
-- person); adding a nullable `role` later is the cheap direction.
--
-- ---------------------------------------------------------------------------------------
-- The rule this table is really about
-- ---------------------------------------------------------------------------------------
--
-- ADR 0025 §4 wrote down what Stage C had to implement: "Assigning a lesson requires the
-- conjunction: a qualification for the lesson's subject, an ACTIVE teacher_school
-- membership at the lesson's school, and campus eligibility where the lesson's campus is
-- constrained."
--
-- ADR 0028 amends the first of those three and keeps the other two. The amendment is
-- argued there; the short version is that a qualification record and an authorisation are
-- different things, and the gap between them is almost always an incomplete record rather
-- than an unqualified teacher. A school covering an absent specialist on Tuesday must be
-- able to say so, and a database that refuses will be answered by inventing a
-- qualification -- which destroys the one record this all rests on.
--
-- So the schema enforces TWO of the three, and the missing qualification is a warning the
-- interface shows, computed by `Lessons::assignable_teachers` from the same data. What is
-- NOT weakened: ADR 0025's ownership model. A qualification is still a property of the
-- person, still carries no school_id, and is still removed only explicitly.
CREATE TABLE class_teacher (
    id               TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception.
    --
    -- The section edge: `delete_class_section` removes these rows itself, in its own
    -- transaction and through repo::delete, so each removal is a truthful change-log entry.
    -- A class_teacher row with no section is not history, it is a dangling fact.
    --
    -- The teacher edge: `delete_teacher` does the same, for the reason ADR 0025 §5 gives
    -- about qualifications -- destroying the person is the one operation that may take
    -- them. `remove_teacher_from_school` is REFUSED instead, and the trigger at the foot of
    -- this file is what makes that refusal a property of the schema; see its comment.
    class_section_id TEXT    NOT NULL REFERENCES class_section (id) ON DELETE RESTRICT,
    teacher_id       TEXT    NOT NULL REFERENCES teacher (id)       ON DELETE RESTRICT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Both columns are NOT NULL, so no COALESCE is needed for this to actually hold.
    -- Saying twice that she teaches 9MAT-A says nothing more.
    UNIQUE (class_section_id, teacher_id)
) STRICT;

-- class_section_id is covered by the UNIQUE index above; teacher_id needs its own, both for
-- the RESTRICT scan and for "what does this teacher teach?", which is a question the
-- Teachers screen and every Phase 4 load constraint will ask.
CREATE INDEX idx_class_teacher_teacher ON class_teacher (teacher_id);

-- A teacher given a section must be ACTIVELY employed by that section's school.
--
-- `ts.is_active = 1`, and this is where the rule deliberately differs from V13's
-- `teacher_campus_requires_school_membership` and V14's
-- `teacher_department_requires_school_membership`, both of which require membership without
-- requiring it to be active. Those two are RECORDS -- which campuses she worked at, which
-- departments she was in -- and ADR 0018 is explicit that they survive a deactivation.
--
-- A class assignment is not a record of the past, it is NEW WORK. ADR 0018's own sentence
-- for the inactive case is that such a person "keeps every assignment they ever had, and
-- simply stops being offered for new ones", and this trigger is the second half of that:
-- the assignments she already has are untouched by deactivating her, and no new one can be
-- created. The pair is enforced by the trigger firing on INSERT and UPDATE only, never on a
-- change to `teacher_school`.
CREATE TRIGGER class_teacher_requires_active_membership_insert
BEFORE INSERT ON class_teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a class section''s teacher must be actively employed by its school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        JOIN teacher_school ts
             ON ts.school_id  = subject.school_id
            AND ts.teacher_id = NEW.teacher_id
        WHERE class_section.id = NEW.class_section_id
          AND ts.is_active = 1
    );
END;

-- Nothing in the repository updates an assignment row -- assign and remove are an insert
-- and a delete -- so this should never fire. It exists so the invariant does not depend on
-- that staying true. Same reasoning as V14's `teacher_department_..._update` and V6's
-- `room_campus_..._update`.
CREATE TRIGGER class_teacher_requires_active_membership_update
BEFORE UPDATE ON class_teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a class section''s teacher must be actively employed by its school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        JOIN teacher_school ts
             ON ts.school_id  = subject.school_id
            AND ts.teacher_id = NEW.teacher_id
        WHERE class_section.id = NEW.class_section_id
          AND ts.is_active = 1
    );
END;

-- A teacher narrowed to particular campuses may only be given a section at one of them.
--
-- Read the three conditions in order, because the middle one is the whole subtlety:
--
--   1. the section names a campus -- otherwise it is not campus-constrained and there is
--      nothing to check;
--   2. this teacher has SOME campus eligibility recorded at this school -- `teacher_campus`
--      is a NARROWING (ADR 0021 §3), so no rows means "not narrowed", which is the state
--      every teacher is in until somebody says otherwise. Reading an empty set as "eligible
--      nowhere" would make every teacher in a multi-campus school unassignable until the
--      eligibility table was filled in, and would have broken every project V13 migrated;
--   3. and the section's campus is not among them.
--
-- Only then is it refused. The eligibility is scoped to campuses of the SECTION's school,
-- so a teacher narrowed at Southmoor is not thereby narrowed at Northgate -- two schools of
-- one trust make their own arrangements, which is the point of V13.
CREATE TRIGGER class_teacher_respects_campus_eligibility_insert
BEFORE INSERT ON class_teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'that teacher is not eligible at this class section''s campus')
    WHERE EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        WHERE class_section.id = NEW.class_section_id
          AND class_section.campus_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM teacher_campus tc
              JOIN campus ON campus.id = tc.campus_id
              WHERE tc.teacher_id = NEW.teacher_id
                AND campus.school_id = subject.school_id
          )
          AND NOT EXISTS (
              SELECT 1 FROM teacher_campus tc
              WHERE tc.teacher_id = NEW.teacher_id
                AND tc.campus_id = class_section.campus_id
          )
    );
END;

CREATE TRIGGER class_teacher_respects_campus_eligibility_update
BEFORE UPDATE ON class_teacher FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'that teacher is not eligible at this class section''s campus')
    WHERE EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        WHERE class_section.id = NEW.class_section_id
          AND class_section.campus_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM teacher_campus tc
              JOIN campus ON campus.id = tc.campus_id
              WHERE tc.teacher_id = NEW.teacher_id
                AND campus.school_id = subject.school_id
          )
          AND NOT EXISTS (
              SELECT 1 FROM teacher_campus tc
              WHERE tc.teacher_id = NEW.teacher_id
                AND tc.campus_id = class_section.campus_id
          )
    );
END;

-- =======================================================================================
-- class_group -- who attends one section
-- =======================================================================================
--
-- The PRIMARY scheduling population. ADR 0006 decided group-based scheduling for
-- Milestone 1 and said what that must keep costing: "A school must be able to build a full
-- timetable using only `student_group.size`, without entering a single student."
--
-- So a section with groups and no individual enrolments is complete, not half-finished, and
-- nothing in this file makes a student roster a prerequisite for anything.
CREATE TABLE class_group (
    id               TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges. The section edge is cleared by `delete_class_section` in its
    -- own transaction; the group edge REFUSES, and `delete_student_group` reports the count
    -- first. A group is substantial -- it has members, a parent, a size and availability --
    -- and deleting one that three class sections are taught to is almost always the wrong
    -- row. That is `delete_subject`'s judgement about qualifications, for the same reason.
    class_section_id TEXT    NOT NULL REFERENCES class_section (id) ON DELETE RESTRICT,
    student_group_id TEXT    NOT NULL REFERENCES student_group (id) ON DELETE RESTRICT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (class_section_id, student_group_id)
) STRICT;

CREATE INDEX idx_class_group_group ON class_group (student_group_id);

-- An attending group must belong to the section's own school.
--
-- Student groups are school-owned (`student_group.school_id`) and are NOT shared the way
-- ADR 0013/0014 share buildings and rooms. Two schools of one trust teaching "9A" hold two
-- groups, and a section of one school may not be taught to the other's children.
CREATE TRIGGER class_group_same_school_insert
BEFORE INSERT ON class_group FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a class section''s student group must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course        ON course.id  = class_section.course_id
        JOIN subject       ON subject.id = course.subject_id
        JOIN student_group ON student_group.id = NEW.student_group_id
        WHERE class_section.id = NEW.class_section_id
          AND student_group.school_id = subject.school_id
    );
END;

CREATE TRIGGER class_group_same_school_update
BEFORE UPDATE ON class_group FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a class section''s student group must belong to the same school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course        ON course.id  = class_section.course_id
        JOIN subject       ON subject.id = course.subject_id
        JOIN student_group ON student_group.id = NEW.student_group_id
        WHERE class_section.id = NEW.class_section_id
          AND student_group.school_id = subject.school_id
    );
END;

-- =======================================================================================
-- class_enrolment -- which named individuals attend one section
-- =======================================================================================
--
-- OPTIONAL, and a ROSTER OVERLAY rather than a second scheduling population. ADR 0029 §4
-- settles what that means and why the alternative is a correctness bug: a section taught to
-- group "9A" (size 28) that also names four children individually has an audience of 28,
-- not 32. Those four ARE some of the 28. Adding the two numbers would double-count every
-- student a school had bothered to name, so nothing adds them -- the group side is the
-- scheduling population in Milestone 1, exactly as ADR 0006 requires, and these rows are a
-- register, a report and the input Milestone 2's sectioning will derive.
--
-- The sketch calls this table `enrolment`. It is renamed, because V13 already has
-- `student_school_enrolment` and "enrolment" now unambiguously means a SPELL AT A SCHOOL
-- throughout this crate -- `Membership::enrol_student`, `StudentEnrolment`,
-- `EnrolmentStatus`. A second table called `enrolment` meaning something else entirely is
-- the kind of name that produces a wrong join years later.
--
-- The sketch's `status` column is omitted. A roster row either exists or it does not, and
-- the statuses a sketch of this table would carry -- enrolled, withdrawn, waitlisted -- are
-- Milestone 2's sectioning vocabulary, which has no reader here. One nullable ALTER away.
CREATE TABLE class_enrolment (
    id               TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, and they are treated oppositely for the reason the class_group
    -- edges are: `delete_class_section` clears its own roster, and `delete_student` is
    -- REFUSED while the person is on one. ADR 0018 already refuses `delete_student` while
    -- any group membership exists, on the ground that the dependent row IS the history, and
    -- "she was in 9MAT-A" is history of exactly that kind.
    class_section_id TEXT    NOT NULL REFERENCES class_section (id) ON DELETE RESTRICT,
    -- The PROJECT-LEVEL identity, not a school enrolment spell.
    --
    -- This was a real choice and the alternative is tempting: pointing at a
    -- `student_school_enrolment` row would make the school context structural rather than
    -- checked. It is rejected on two grounds. A student may hold several spells at one
    -- school -- V13 keeps unlimited LEFT history so somebody may leave and return -- so a
    -- spell-keyed roster could list the same child twice in one class, once per spell, and
    -- "who is in 9MAT-A?" would have a wrong answer that looks right. And the question a
    -- roster answers is about a person, not about an episode of their attendance.
    --
    -- The school context is therefore enforced instead of inferred, by the trigger pair
    -- below, which is the same technique V13 used for campus eligibility.
    student_id       TEXT    NOT NULL REFERENCES student (id) ON DELETE RESTRICT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (class_section_id, student_id)
) STRICT;

CREATE INDEX idx_class_enrolment_student ON class_enrolment (student_id);

-- An enrolled individual must have a CURRENT spell at the section's own school.
--
-- `status <> 'LEFT'`, which is ENROLLED or PROSPECTIVE, and the second of those is
-- deliberate: V13 describes PROSPECTIVE as "an applicant: recorded, placed, not yet
-- attending", and a school sectioning next September's intake is doing precisely that. A
-- rule admitting only ENROLLED would refuse the main reason to build a roster in advance.
--
-- `student_enrolment_one_current` guarantees at most one such spell per (student, school),
-- so this cannot match two rows and quietly mean something weaker than it says.
--
-- Note what this does NOT do, exactly as V13 and V14 did not: it does not fire when a spell
-- changes. A student who LEAVES the school keeps every class roster row they were on, and
-- `Membership::leave_school` is untouched by this file. The rows are the record that they
-- were in that class, and ADR 0018's rule is that where the dependent row is the history,
-- it is kept and the remedy is made visible rather than performed. Removing somebody from a
-- class is a separate, explicit act.
CREATE TRIGGER class_enrolment_requires_current_spell_insert
BEFORE INSERT ON class_enrolment FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an enrolled student must be currently enrolled at this section''s school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        JOIN student_school_enrolment sse
             ON sse.school_id  = subject.school_id
            AND sse.student_id = NEW.student_id
        WHERE class_section.id = NEW.class_section_id
          AND sse.status <> 'LEFT'
    );
END;

CREATE TRIGGER class_enrolment_requires_current_spell_update
BEFORE UPDATE ON class_enrolment FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an enrolled student must be currently enrolled at this section''s school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM class_section
        JOIN course  ON course.id  = class_section.course_id
        JOIN subject ON subject.id = course.subject_id
        JOIN student_school_enrolment sse
             ON sse.school_id  = subject.school_id
            AND sse.student_id = NEW.student_id
        WHERE class_section.id = NEW.class_section_id
          AND sse.status <> 'LEFT'
    );
END;

-- =======================================================================================
-- activity -- ONE placeable unit of work
-- =======================================================================================
--
-- The thing a timetable places. One row is one occurrence: a class section needing four
-- single periods a cycle holds four of these, and a section needing one double holds one
-- with `duration_periods = 2`. See this file's header for the three reasons it is rows
-- rather than a count, and ADR 0029 for the argument in full.
--
-- An activity has no timeslot, no room and no cycle. It is DEMAND, and nothing in Stage C
-- claims it is schedulable -- only a solver can say that, and there is not one yet.
CREATE TABLE activity (
    id               TEXT    PRIMARY KEY NOT NULL,
    -- NOT NULL, which is a departure from DATA-MODEL.md §7's nullable version, and the
    -- reason the table needs no `school_id`: every activity Stage C models belongs to a
    -- class section, and the section's chain supplies the school. The sketch's nullable
    -- column exists for an assembly or a yard duty, which Stage C does not model; adding
    -- them means widening this column and adding `school_id` and `kind`, which is an
    -- ADR 0020 in-place rebuild of the kind V12 has already done once. Carrying three
    -- columns now that no screen can explain is the mistake V14 named about
    -- `subject.colour`.
    class_section_id TEXT    NOT NULL REFERENCES class_section (id) ON DELETE RESTRICT,
    -- What to call this occurrence, when it needs its own name: "Laboratory", "Double".
    -- Optional, because most occurrences are simply "a period of this class" and a form
    -- that insisted would be answered by repeating the section's name.
    name             TEXT             CHECK (name IS NULL OR length(trim(name)) > 0),
    -- CONSECUTIVE periods within this one occurrence. A double period is one activity with
    -- duration_periods = 2, not two activities and a link -- D6 in OPEN-DECISIONS, decided
    -- so that "this must not be split" is structural rather than a rule that can be
    -- switched off.
    --
    -- Not to be confused with a teaching load: a section with four of these at duration 1
    -- demands four periods a cycle, and one of these at duration 2 demands two.
    duration_periods INTEGER NOT NULL DEFAULT 1 CHECK (duration_periods >= 1),
    -- Which occurrence this is, within its section. Assigned once, from
    -- MAX(split_index) + 1, and NEVER renumbered -- deleting the second of four leaves
    -- 1, 3, 4 rather than rewriting two rows the user did not touch and consuming their
    -- revisions. The interface numbers the rows it displays; this is for stable ordering
    -- and for Phase 4's "spread these four across four days", which needs the occurrences
    -- to be individually addressable.
    split_index      INTEGER NOT NULL CHECK (split_index >= 1),
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    rev              INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (class_section_id, split_index)
) STRICT;

-- =======================================================================================
-- Withdrawing a membership must not leave an assignment asserting it
-- =======================================================================================
--
-- The other direction of `class_teacher_requires_active_membership`, and the one place this
-- file reaches back into a V13 table.
--
-- It REFUSES rather than cascading, and that is the opposite of what V13 and V14 chose for
-- the two other narrowings of membership. `remove_teacher_from_school` clears a teacher's
-- campus eligibility and department associations itself, in the same transaction, because
-- those are organisational labels that mean nothing once the employment has gone.
--
-- A class assignment is not a label. It is scheduling input that a timetable will be built
-- on, and removing somebody from a school while they still teach six classes is almost
-- certainly the wrong row -- the useful answer names what is in the way. ADR 0018's rule
-- applies: where the dependent row is real, the delete is refused and the remedy is made
-- visible rather than performed.
--
-- The remedy is also the one a school actually wants. Recording that somebody has left is
-- `set_teacher_membership_active(false)`, which keeps every assignment they ever had and
-- stops them being offered for new ones; withdrawing the membership outright is for
-- somebody who should never have been given this school at all.
--
-- `Membership::remove_teacher_from_school` checks this first, with counts, because a trigger
-- cannot name the remedy.
CREATE TRIGGER teacher_school_delete_requires_no_class_assignment
BEFORE DELETE ON teacher_school FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'remove the teacher from this school''s class sections before the membership')
    WHERE EXISTS (
        SELECT 1
        FROM class_teacher ct
        JOIN class_section ON class_section.id = ct.class_section_id
        JOIN course        ON course.id        = class_section.course_id
        JOIN subject       ON subject.id       = course.subject_id
        WHERE ct.teacher_id = OLD.teacher_id
          AND subject.school_id = OLD.school_id
    );
END;
