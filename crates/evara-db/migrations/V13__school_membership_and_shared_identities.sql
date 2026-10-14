-- Shared identities: school membership, campus eligibility, enrolment history, shared
-- resources.
--
-- V12 (ADR 0019) made a project hold several schools, and said explicitly what it did not
-- do: "a teacher or student belonging to several schools. Both still have exactly one
-- school_id. Project-level identities with school membership and enrolment history --
-- teacher_school, teacher_campus, student_school_enrolment -- are the next stage." This is
-- that stage. ADR 0021 and ADR 0022 record the decisions.
--
-- The arrangement this exists for is a trust, a group, or a school with a separate junior
-- programme: several institutions that SHARE STAFF, ROOMS AND EQUIPMENT. V12 made the
-- rooms work, because ADR 0013/0014's campus associations never mentioned a school. People
-- and equipment did not, and modelling a shared teacher as two teacher rows gives one
-- person two identities, two availability records and two independent timetables -- the
-- same correctness failure ADR 0014 rejected for rooms, where two room rows have two
-- occupancy records and neither notices the clash.
--
-- ---------------------------------------------------------------------------------------
-- Four relationships, all of them first-class entities
-- ---------------------------------------------------------------------------------------
--
--   teacher_school             which schools employ this person          (the authority)
--   teacher_campus             which campuses they may be based at       (a narrowing)
--   student_school_enrolment   one row per spell at one school           (history)
--   resource_campus            which campuses a resource serves          (mirrors V6)
--
-- Every one of them gets its own UUIDv7 `id` and audit columns rather than a composite
-- primary key, for the reason V5, V6 and V10 all give: the change-log triggers read
-- NEW.id and NEW.rev, and repo::update / repo::delete address rows by id, so a
-- composite-key table would sit outside both -- and therefore outside the change feed.
-- Putting a teacher into a school is a real edit to a real project and belongs in the feed
-- like any other. All four are added to change_log::TRACKED_TABLES, and
-- change_log::install_all creates their triggers on the next open.
--
-- ---------------------------------------------------------------------------------------
-- Additive, so the V12 safety copy is untouched by this
-- ---------------------------------------------------------------------------------------
--
-- Nothing here rebuilds a table. Four CREATE TABLEs, their indexes, four backfills, and
-- trigger work. `backup::FIRST_REBUILDING_MIGRATION` stays at 12: a project already at V12
-- needs no copy to reach V13, and a project at V11 still gets one, because it is about to
-- cross 12 on the way here. That is the Stage 0A mechanism working unchanged rather than
-- being extended.
--
-- Forward-only. V1-V12 are untouched.
--
-- ---------------------------------------------------------------------------------------
-- What a backfill must not do
-- ---------------------------------------------------------------------------------------
--
-- No existing row is rewritten and no revision is consumed. Every backfilled row reuses
-- its subject's own `created_at`, because the relationship is as old as the fact it
-- records, and `updated_at` matches it because nothing has edited it yet -- the Stamp
-- convention for a row at rev 1. A project migrated by this file describes exactly the
-- same schools it did before.
--
-- The identifiers are laid out field by field for the reason V5's and V6's backfills were:
-- SQLite has no uuid() and refinery migrations are .sql files. 48-bit millisecond
-- timestamp, version nibble 7, then the two-bit variant as one of 8/9/a/b, then random.
-- `random() & 3` keeps the low two bits and is therefore always 0-3 even though random()
-- is signed. A test asserts get_version_num() == 7 on the results rather than trusting
-- this printf.

-- =======================================================================================
-- teacher_school -- which schools employ this person
-- =======================================================================================
--
-- `teacher.school_id` survives, with a narrowed meaning, exactly as `room.campus_id`
-- survived V6:
--
--   * `teacher_school` is the COMPLETE set of schools a teacher belongs to. Every school
--     has a row here, the home school included. Nothing reads `teacher.school_id` to
--     answer "which schools?".
--   * `teacher.school_id` is the HOME school: the one they were created under. Immutable,
--     always has a matching `teacher_school` row, and its membership cannot be removed.
--     It still scopes `teacher_code_unique (school_id, code)` -- see ADR 0021 §4 for why
--     that index is deliberately NOT widened in this stage -- and it guarantees a teacher
--     can never reach zero schools and vanish from every screen.
--
-- ONE ROW PER (teacher, school), NOT A HISTORY. A membership is deactivated rather than
-- duplicated: `is_active = 0` is "no longer employed here, and every assignment they ever
-- had still resolves". That is the same bargain `teacher.is_active` struck in ADR 0018 and
-- the same reason -- a school that cannot say "not this year" without deleting the record
-- will delete the record. Enrolment is the opposite case and is modelled the opposite way
-- below, because a student's spells at a school are the thing a school is asked about.
--
-- TWO LEVELS OF "ACTIVE", WITH DISTINCT MEANINGS (audit decision 3):
--
--   teacher.is_active         in the project at all      -- an archive flag
--   teacher_school.is_active  employed by THIS school    -- per-school employment
--
-- Effective eligibility is the conjunction. A teacher active in the project but inactive
-- at Southmoor is not available to Southmoor, and nothing may read the project-level flag
-- alone and conclude otherwise.
CREATE TABLE teacher_school (
    id         TEXT    PRIMARY KEY NOT NULL,
    -- RESTRICT on both edges, continuing ADR 0010 without exception. The repository
    -- removes these rows itself through repo::delete so each one is a truthful change-log
    -- entry; a CASCADE would remove them without firing the trigger, because SQLite does
    -- not fire triggers for foreign-key actions unless recursive_triggers is on.
    --
    -- The school edge is also what makes `delete_school` keep working the way ADR 0019
    -- promised: a school that still employs anybody is refused, with
    -- `Structure::school_dependents` reporting it, rather than quietly unemploying them.
    teacher_id TEXT    NOT NULL REFERENCES teacher (id) ON DELETE RESTRICT,
    school_id  TEXT    NOT NULL REFERENCES school  (id) ON DELETE RESTRICT,
    -- Per-school employment state. See the note above: this is not a soft delete, and a
    -- deactivated membership keeps the person, their availability and their history.
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- Both columns are NOT NULL, so no COALESCE is needed for this one to actually hold.
    UNIQUE (teacher_id, school_id)
) STRICT;

-- Every RESTRICT delete scans the children to decide whether to refuse. teacher_id is
-- covered by the UNIQUE index above; school_id needs its own, and it is also the index
-- every school-scoped teacher listing now reads.
CREATE INDEX idx_teacher_school_school ON teacher_school (school_id);

-- One membership per existing teacher, naming the school they already belonged to.
INSERT INTO teacher_school (id, teacher_id, school_id, is_active, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    teacher.id,
    teacher.school_id,
    -- The membership inherits the teacher's own state, because until this migration that
    -- flag WAS the per-school answer -- there was only one school to be employed by.
    -- Reading it as 1 regardless would silently re-employ every teacher a school had
    -- marked as having left.
    teacher.is_active,
    teacher.created_at,
    teacher.created_at,
    1
FROM teacher;

-- =======================================================================================
-- teacher_campus -- which campuses they may be based at
-- =======================================================================================
--
-- Eligibility is a NARROWING of membership, never an independent claim. The common
-- part-time arrangement is "School A, junior campus only", which membership alone cannot
-- express; and membership alone is also needed, because a school can be joined before its
-- campuses are configured and there has to be somewhere to hang per-school employment
-- state. So both exist, and the drift between them -- a teacher eligible at a campus of a
-- school they do not belong to -- is rejected by the database rather than by convention.
CREATE TABLE teacher_campus (
    id         TEXT    PRIMARY KEY NOT NULL,
    teacher_id TEXT    NOT NULL REFERENCES teacher (id) ON DELETE RESTRICT,
    campus_id  TEXT    NOT NULL REFERENCES campus  (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (teacher_id, campus_id)
) STRICT;

CREATE INDEX idx_teacher_campus_campus ON teacher_campus (campus_id);

-- One eligibility row per teacher who already had a home campus. A teacher with none gets
-- none, which is correct and is the common case for a single-campus school: eligibility is
-- a restriction, and "no restriction recorded" is not the same claim as "restricted to
-- nowhere".
--
-- `home_campus_id` is a `SET NULL` reference (V10), so it can only ever name a campus that
-- exists; and V12's trigger held it to the teacher's own school. Both make this backfill
-- unable to produce a row the membership trigger below would refuse.
INSERT INTO teacher_campus (id, teacher_id, campus_id, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    teacher.id,
    teacher.home_campus_id,
    teacher.created_at,
    teacher.created_at,
    1
FROM teacher
WHERE teacher.home_campus_id IS NOT NULL;

-- Eligibility may not contradict membership.
--
-- Same shape as V6's "a room's building must be available on every campus the room
-- serves": the rule is stated where the storage is, so a future repository, the CLI, an
-- import or a hand-repair cannot write a row that breaks it. The repository checks it
-- first too, because a trigger cannot say which field is wrong -- and, more usefully here,
-- cannot say "add the school membership first", which is the one ordering mistake this
-- rule will actually produce.
--
-- Note what is NOT required: that the membership be *active*. A teacher who has left
-- Southmoor keeps the record of which Southmoor campuses they worked at, the way an
-- inactive teacher keeps every assignment they ever had. Removing a membership outright is
-- what clears eligibility, and the repository does that in one transaction.
CREATE TRIGGER teacher_campus_requires_school_membership_insert
BEFORE INSERT ON teacher_campus FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s campus must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN teacher_school ts ON ts.school_id = campus.school_id
        WHERE campus.id = NEW.campus_id
          AND ts.teacher_id = NEW.teacher_id
    );
END;

-- Nothing in the repository updates an eligibility row -- assign and remove are an insert
-- and a delete -- so this should never fire. It exists so the invariant does not depend on
-- that staying true: a future method that repointed a row would otherwise break the rule
-- silently. Same reasoning as V6's `room_campus_..._update`.
CREATE TRIGGER teacher_campus_requires_school_membership_update
BEFORE UPDATE ON teacher_campus FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s campus must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1
        FROM campus
        JOIN teacher_school ts ON ts.school_id = campus.school_id
        WHERE campus.id = NEW.campus_id
          AND ts.teacher_id = NEW.teacher_id
    );
END;

-- The other direction: removing a membership must not leave eligibility asserting it.
--
-- The repository clears the dependent `teacher_campus` rows in the same transaction,
-- through repo::delete, so each removal is a truthful change-log entry -- the pattern
-- `delete_student_group` established for memberships and `delete_room` for associations.
-- This trigger is the backstop for a delete that bypasses it, and it refuses rather than
-- cascading: a membership removal that silently took eligibility rows with it would leave
-- no change-log evidence that they had ever existed.
CREATE TRIGGER teacher_school_delete_requires_no_campus_eligibility
BEFORE DELETE ON teacher_school FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'remove the teacher''s campuses in this school before the membership')
    WHERE EXISTS (
        SELECT 1
        FROM teacher_campus tc
        JOIN campus ON campus.id = tc.campus_id
        WHERE tc.teacher_id = OLD.teacher_id
          AND campus.school_id = OLD.school_id
    );
END;

-- ---------------------------------------------------------------------------------------
-- V12's teacher trigger pair, revised for membership
-- ---------------------------------------------------------------------------------------
--
-- V12 said of these: "Stage 0A keeps a teacher owned by one school; ADR 0021 will replace
-- this with school membership, and this trigger goes when that arrives." It does not go --
-- it widens. The guarantee it exists for is unchanged: a teacher's home campus must not be
-- some other institution's. What changes is which institutions count as theirs.
--
-- The home-school disjunct is kept and is not redundant. `create_teacher` writes the
-- teacher row and its membership row in one transaction, and the teacher row is first --
-- it has to be, because the membership references it. So at the moment this trigger runs
-- on INSERT there is no membership yet, and only `NEW.school_id` can answer. Dropping the
-- disjunct would make it impossible to create a teacher with a home campus at all.
DROP TRIGGER teacher_references_own_school_insert;
DROP TRIGGER teacher_references_own_school_update;

CREATE TRIGGER teacher_references_a_member_school_insert
BEFORE INSERT ON teacher FOR EACH ROW
WHEN NEW.home_campus_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s home campus must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1 FROM campus
        WHERE campus.id = NEW.home_campus_id
          AND (campus.school_id = NEW.school_id
               OR EXISTS (SELECT 1 FROM teacher_school ts
                          WHERE ts.teacher_id = NEW.id
                            AND ts.school_id = campus.school_id))
    );
END;

CREATE TRIGGER teacher_references_a_member_school_update
BEFORE UPDATE ON teacher FOR EACH ROW
WHEN NEW.home_campus_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a teacher''s home campus must belong to a school they are a member of')
    WHERE NOT EXISTS (
        SELECT 1 FROM campus
        WHERE campus.id = NEW.home_campus_id
          AND (campus.school_id = NEW.school_id
               OR EXISTS (SELECT 1 FROM teacher_school ts
                          WHERE ts.teacher_id = NEW.id
                            AND ts.school_id = campus.school_id))
    );
END;

-- =======================================================================================
-- student_school_enrolment -- one row per spell at one school
-- =======================================================================================
--
-- Modelled as HISTORY, unlike teacher membership, because a spell at a school is the thing
-- a school is actually asked about: when did they join, when did they leave, did they come
-- back. A transfer therefore writes a LEFT row and an ENROLLED row; it never edits one in
-- place and never produces a second student record.
--
-- `student.id` is untouched by every one of those operations. That is the whole point: one
-- real person, one identity, however many schools and however many spells.
CREATE TABLE student_school_enrolment (
    id            TEXT    PRIMARY KEY NOT NULL,
    student_id    TEXT    NOT NULL REFERENCES student (id) ON DELETE RESTRICT,
    school_id     TEXT    NOT NULL REFERENCES school  (id) ON DELETE RESTRICT,
    -- An admission number is a SCHOOL's identifier for a student, which is why it lives
    -- here and not on `student`. Two schools may legitimately number the same person
    -- differently, and neither number is "the" one.
    admission_no  TEXT             CHECK (admission_no IS NULL OR length(trim(admission_no)) > 0),
    -- School-specific placement. SET NULL, matching V10's treatment of the same two
    -- references on `student`: the enrolment is the record, and which year they sat in is
    -- metadata that losing a year level should not take an enrolment with it.
    year_level_id TEXT             REFERENCES year_level (id) ON DELETE SET NULL,
    campus_id     TEXT             REFERENCES campus (id)     ON DELETE SET NULL,
    -- PROSPECTIVE is an applicant: recorded, placed, not yet attending. ENROLLED is
    -- attending. LEFT is history. Checked here as well as in Rust so a project opened by
    -- an older build fails loudly rather than reading an unknown status as a plausible one
    -- -- the same reason `student_group.kind` and `cycle.kind` carry CHECKs.
    status        TEXT    NOT NULL CHECK (status IN ('ENROLLED', 'LEFT', 'PROSPECTIVE')),
    -- ISO-8601 calendar dates, both optional. `date()` returns NULL for anything SQLite
    -- cannot parse, which is how the format is enforced without a regular expression --
    -- the V3 pattern. Optional because plenty of schools do not record the day somebody
    -- joined, and a form that insisted would be answered with invented dates.
    enrolled_on   TEXT             CHECK (enrolled_on IS NULL OR date(enrolled_on) IS NOT NULL),
    left_on       TEXT             CHECK (left_on IS NULL OR date(left_on) IS NOT NULL),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- A spell cannot end before it began. Both nullable, so the comparison is guarded.
    CHECK (left_on IS NULL OR enrolled_on IS NULL OR left_on >= enrolled_on),
    -- A LEFT spell is the one that has an end date to have; the others must not carry one,
    -- or "has this student left?" would have two answers that could disagree.
    CHECK (status = 'LEFT' OR left_on IS NULL)
) STRICT;

CREATE INDEX idx_student_school_enrolment_school ON student_school_enrolment (school_id);
CREATE INDEX idx_student_school_enrolment_student ON student_school_enrolment (student_id);

-- AT MOST ONE CURRENT ENROLMENT PER (student, school); UNLIMITED HISTORY.
--
-- This is the constraint that makes every requirement hold at once:
--
--   * a second ENROLLED row for the same school is refused -- two current spells at one
--     school is not a thing that can be true;
--   * a PROSPECTIVE row alongside an ENROLLED one at the same school is likewise refused,
--     because an applicant who is already attending is a contradiction, not a record;
--   * LEFT rows are unconstrained, so a student may leave and rejoin the same school any
--     number of times and every spell is kept;
--   * two schools are two different keys, so concurrent enrolment falls out for free.
--
-- The audit proposed `UNIQUE (student_id, school_id, COALESCE(enrolled_on, ''))` instead.
-- That is rejected, and for exactly the reason CLAUDE.md's COALESCE rule exists in the
-- other direction: `enrolled_on` is optional, so COALESCE would fold every undated spell
-- onto one key and permit exactly one of them -- which is to say a school that does not
-- record joining dates could not re-enrol anybody, the common case made impossible by a
-- constraint meant to protect it. See ADR 0021 §5.
CREATE UNIQUE INDEX student_enrolment_one_current
    ON student_school_enrolment (student_id, school_id) WHERE status <> 'LEFT';

-- An admission number identifies one CURRENT student within one school.
--
-- Partial, following ADR 0018 §4: unique when given, unconstrained when absent, never
-- COALESCE -- a school that does not issue admission numbers must be able to enrol more
-- than one student. Scoped to current spells for a second reason: a student who leaves and
-- rejoins keeps their old number on the historical row, and a school that reissues a
-- retired number to somebody else has not done anything wrong. Constraining history would
-- refuse both.
CREATE UNIQUE INDEX student_admission_no_unique
    ON student_school_enrolment (school_id, admission_no)
    WHERE admission_no IS NOT NULL AND status <> 'LEFT';

-- One enrolment per existing student, naming the school they already belonged to, carrying
-- the placement and the admission number they already had.
--
-- `student.code` is V10's "admission or roll number", so it maps to `admission_no`. The
-- column is left in place and its value is not rewritten -- see ADR 0021 §6 for the
-- compatibility policy, which keeps the two in step through the repository rather than by
-- dropping one of them here.
--
-- Status comes from `student.is_active`, for the reason the teacher backfill reads
-- `teacher.is_active`: until this migration that flag WAS the per-school answer. A student
-- a school had marked as having left becomes a LEFT spell, not a re-enrolment.
--
-- `left_on` stays NULL even for a LEFT spell. The date is not recorded anywhere in a V12
-- project, and inventing one would be fabricating school data -- the `CHECK` above permits
-- a LEFT row without an end date precisely so this backfill does not have to.
INSERT INTO student_school_enrolment
    (id, student_id, school_id, admission_no, year_level_id, campus_id,
     status, enrolled_on, left_on, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    student.id,
    student.school_id,
    student.code,
    student.year_level_id,
    student.campus_id,
    CASE WHEN student.is_active = 1 THEN 'ENROLLED' ELSE 'LEFT' END,
    NULL,
    NULL,
    student.created_at,
    student.created_at,
    1
FROM student;

-- An enrolment's placement must belong to the enrolment's school.
--
-- The same rule V12 wrote for `student`, restated where the authoritative placement now
-- lives. Without it a student could be enrolled at Southmoor and placed in Northgate's
-- Year 9, which is corruption no screen would reveal -- the student simply appears in the
-- wrong year, or in no year, depending on which query is asked.
CREATE TRIGGER student_enrolment_references_own_school_insert
BEFORE INSERT ON student_school_enrolment FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an enrolment''s year level must belong to the enrolment''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an enrolment''s campus must belong to the enrolment''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

CREATE TRIGGER student_enrolment_references_own_school_update
BEFORE UPDATE ON student_school_enrolment FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'an enrolment''s year level must belong to the enrolment''s school')
    WHERE NEW.year_level_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM year_level
                      WHERE year_level.id = NEW.year_level_id
                        AND year_level.school_id = NEW.school_id);
    SELECT RAISE(ABORT, 'an enrolment''s campus must belong to the enrolment''s school')
    WHERE NEW.campus_id IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM campus
                      WHERE campus.id = NEW.campus_id
                        AND campus.school_id = NEW.school_id);
END;

-- ---------------------------------------------------------------------------------------
-- A group's members must be enrolled in the group's school
-- ---------------------------------------------------------------------------------------
--
-- This is a hole V12 left and could not close. `student_group_member` carries no
-- `school_id` of its own -- the group owns it -- and V12's audit (§2.5) listed the tables
-- whose own columns could disagree, which this one has none of. The disagreement is
-- between its two ends: nothing stopped Northgate's student being put into Southmoor's
-- cohort, and a cohort containing another institution's students is not an audience
-- anything can be timetabled for.
--
-- Stated against ENROLMENT rather than against `student.school_id`, which is what makes it
-- correct under sharing: a student enrolled in both schools may legitimately be in a group
-- in either. Any spell counts, LEFT included, for the reason ADR 0018 gives throughout --
-- membership of 9-MAT-A last year is the only evidence it happened, and a student who has
-- left must not have their history refused.
--
-- ADR 0006 is untouched. A school that timetables with `student_group.size` and no student
-- records at all writes no rows in this table, so this rule has nothing to say to it.
CREATE TRIGGER student_group_member_requires_enrolment_insert
BEFORE INSERT ON student_group_member FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a group member must be enrolled in the group''s school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM student_group g
        JOIN student_school_enrolment e ON e.school_id = g.school_id
        WHERE g.id = NEW.student_group_id
          AND e.student_id = NEW.student_id
    );
END;

CREATE TRIGGER student_group_member_requires_enrolment_update
BEFORE UPDATE ON student_group_member FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'a group member must be enrolled in the group''s school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM student_group g
        JOIN student_school_enrolment e ON e.school_id = g.school_id
        WHERE g.id = NEW.student_group_id
          AND e.student_id = NEW.student_id
    );
END;

-- =======================================================================================
-- resource_campus -- which campuses a resource serves
-- =======================================================================================
--
-- The last of ADR 0019's "one school, for now" list. `building_campus` and `room_campus`
-- already reach across schools, because they associate a facility with CAMPUSES and never
-- mention a school; a projector trolley or a minibus shared between two institutions on
-- one site could not be expressed at all, and modelling it as two resources gives one
-- object two identities and two independent availability records.
--
-- Mirrors V6 exactly, with one deliberate difference recorded in ADR 0022: `room.campus_id`
-- is NOT NULL and `resource.campus_id` is not, so a resource may serve NO campus. That is
-- not an error state -- it is V3's "a resource may be school-wide or tied to one campus",
-- preserved. So, unlike `room_campus`, this table may hold nothing for a resource, and the
-- "a facility must never reach zero campuses" rule applies only to a resource that has a
-- home campus to lose.
CREATE TABLE resource_campus (
    id          TEXT    PRIMARY KEY NOT NULL,
    resource_id TEXT    NOT NULL REFERENCES resource (id) ON DELETE RESTRICT,
    campus_id   TEXT    NOT NULL REFERENCES campus (id)   ON DELETE RESTRICT,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    rev         INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (resource_id, campus_id)
) STRICT;

CREATE INDEX idx_resource_campus_campus ON resource_campus (campus_id);

-- Carry every existing resource-to-campus relationship across. A school-wide resource has
-- no `campus_id` and therefore gets no row, which is the same statement it was already
-- making.
INSERT INTO resource_campus (id, resource_id, campus_id, created_at, updated_at, rev)
SELECT
    substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 1, 8)
        || '-'
        || substr(printf('%012x', CAST(ROUND((julianday('now') - 2440587.5) * 86400000.0) AS INTEGER)), 9, 4)
        || '-7' || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || substr('89ab', 1 + (random() & 3), 1) || substr(lower(hex(randomblob(2))), 1, 3)
        || '-' || lower(hex(randomblob(6))),
    resource.id,
    resource.campus_id,
    resource.created_at,
    resource.created_at,
    1
FROM resource
WHERE resource.campus_id IS NOT NULL;

-- DELIBERATELY NOT CONSTRAINED BY SCHOOL, exactly as V12 left `building_campus` and
-- `room_campus`. A resource shared between campuses of different schools is the entire
-- point, and the matching invariant carries over from ADR 0014 unchanged: a shared
-- resource is ONE resource, `resource.id` is its scheduling identity, and any future
-- resource-clash rule must be evaluated per `resource_id` across the whole project, never
-- per campus. Nothing enforces that here because nothing is scheduled yet.
--
-- `resource.school_id` stays, narrowed to the HOME school, and V12's
-- `resource_references_own_school_*` pair stays with it: the home campus still belongs to
-- the home school, so that rule is still true and still worth asserting. What it no longer
-- means is "the only school that may use this".
