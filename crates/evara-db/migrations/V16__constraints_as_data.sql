-- Constraints become data, and a parent can say whether its children share students.
--
-- Forward-only. V1-V15 are untouched.
--
-- Two tables and one column, which is the whole of M5's schema need:
--
--   * `constraint_instance` -- a configured rule: a `type_key`, a level, a weight, an
--     enable flag and JSON parameters. ADR 0005 commits to constraints-as-data, and this
--     is the row it names.
--   * `constraint_ref`      -- an instance's scope, as real rows rather than identifiers
--     buried in the JSON.
--   * `student_group.children_are_disjoint` -- ADR 0035 Sect. 4's one nullable column,
--     without which `student-clash` degenerates.
--
-- Nothing is rebuilt, no existing row is rewritten, and every existing project reads
-- `NULL` for the new column, which is exactly the behaviour ADR 0035 defines for them.
--
-- ============================================================================ deviations
--
-- `DATA-MODEL.md` Sect. 7 sketches these two tables. Three details depart from that
-- sketch, each because a convention established after it was written says otherwise. They
-- are recorded here rather than in a commit message, because a migration is immutable and
-- the next reader of this file is the person who needs to know.
--
-- 1. `constraint_ref` IS A FIRST-CLASS ENTITY, NOT A JUNCTION ROW.
--
--    The sketch gives it `PRIMARY KEY(constraint_id, role, entity_id)` and no audit
--    columns. Two tests written since -- `every_entity_table_carries_the_common_metadata_columns`
--    and `every_entity_table_has_all_three_change_log_triggers` -- require `id`,
--    `created_at`, `updated_at`, `rev` and three triggers on every entity table, and the
--    change-log triggers read NEW.id and NEW.rev, so a composite-key table sits outside
--    the change feed. V5's `building_campus` and V6's `room_campus` made the same choice
--    for the same reason, and their headers argue it: "An association is a first-class
--    entity, not a bare junction row". The uniqueness the sketch wanted survives as a
--    UNIQUE index.
--
-- 2. THE SCOPE EDGE IS `RESTRICT`, NOT `CASCADE`, AND THE REPOSITORY CLEARS IT.
--
--    ADR 0005 says scope references get "a cascading delete", and that was right when
--    `constraint_ref` had no triggers. It cannot be right now: SQLite fires a DELETE
--    trigger for a row removed by a foreign-key action only when `recursive_triggers` is
--    on, and this project never enables it (see `connection::configure`). A CASCADE would
--    therefore delete scope rows *without* a change-log entry -- silently, and only for
--    the rows reached by cascade.
--
--    So the edge is RESTRICT and `Constraints::delete` removes an instance's scope rows
--    itself, in the same transaction, through `repo::delete`. That is `delete_class_section`'s
--    pattern exactly, and V15's comment on it states the principle: a row removed this way
--    is "a truthful change-log entry", and a scope row with no instance "is not history, it
--    is a dangling fact". The user-visible behaviour ADR 0005 wanted is unchanged -- deleting
--    an instance takes its scope with it -- and the change feed now says so.
--
-- 3. `weight` IS AN INTEGER, AND THERE IS NO `timetable_id`.
--
--    `CLAUDE.md` Sect. 3: "Scores are integers... Weights become integer penalties at
--    compile time, not at scoring time." `AvailabilityRule.weight` already chose `i64` over
--    a float with that reasoning written down, and the `entity!` macro derives `Eq`, which
--    `f64` does not implement. `SOLVER.md` Sect. 3's "a float in the database" parenthetical
--    is corrected rather than followed.
--
--    `timetable_id` is omitted because there is no `timetable` table until M7, and a
--    nullable column with no foreign key would be the referential hole `constraint_ref`
--    exists to avoid. M7 adds it as one nullable ADD COLUMN, which is the cheap direction.
--
-- ===================================================================== what is NOT here
--
-- No default rows are seeded. The eight MVP types are mandatory in the *registry*, not in
-- the data: evaluation always runs them, so deleting a row cannot disable a safety rule and
-- a project that has never been configured is still fully checked. Seeding would also need
-- change-log entries attributed to an installation, and `evara_site_id()` is not registered
-- during a migration -- so the rows would either be unattributable or refused. See
-- `evara-constraints`' registry and ADR 0039.

-- --------------------------------------------------------------------- constraint_instance

-- A configured rule. One row per (type, scope) a school has set up.
--
-- `school_id` and not a project-level scope: the multi-school audit's OPEN 9 asked whether an
-- instance should be project-level for a shared teacher, and answered "yes, and it must be" --
-- but that answer was never recorded in an ADR, and generation is scoped to one school
-- (ADR 0032 rule 1), so an instance that spanned schools would have no solve to apply to.
-- School scope is what the MVP needs and what `DATA-MODEL.md` specifies; the project-level
-- case stays open and is one nullable column away.
CREATE TABLE constraint_instance (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- The registry's identity, not a Rust type name. Stored, so a project written by a
    -- newer build keeps rules this one does not recognise rather than losing them; the
    -- repository refuses to *create* an unknown key, and evaluation skips one it meets.
    type_key   TEXT    NOT NULL CHECK (length(trim(type_key)) > 0),
    -- Configurable per instance, which is ADR 0005's point: one school runs a rule SOFT and
    -- another runs it HARD. The registry ships a default; this column is the override.
    level      TEXT    NOT NULL CHECK (level IN ('HARD', 'MEDIUM', 'SOFT')),
    weight     INTEGER NOT NULL DEFAULT 1 CHECK (weight >= 0),
    enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- JSON, validated against the registry's schema for `type_key` on write. `json_valid`
    -- is the floor: it stops a non-JSON string reaching a parser, and the shape is the
    -- repository's business because only the registry knows the schema.
    params     TEXT    NOT NULL DEFAULT '{}'
                        CHECK (json_valid(params) AND json_type(params) = 'object'),
    -- Which term it applies to, or NULL for the whole year.
    term_id    TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

CREATE INDEX idx_constraint_instance_school ON constraint_instance (school_id);
CREATE INDEX idx_constraint_instance_type   ON constraint_instance (school_id, type_key);
CREATE INDEX idx_constraint_instance_term   ON constraint_instance (term_id);

-- A term must belong to the instance's own school.
--
-- `term` reaches its school through `academic_year`, so this cannot be a foreign key. Same
-- shape as V15's same-school trigger pairs, and a pair rather than one trigger for the same
-- reason: nothing in the repository updates `school_id`, and the invariant should not depend
-- on that staying true.
CREATE TRIGGER constraint_instance_term_same_school_insert
BEFORE INSERT ON constraint_instance FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'that term belongs to a different school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = NEW.school_id
    );
END;

CREATE TRIGGER constraint_instance_term_same_school_update
BEFORE UPDATE ON constraint_instance FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'that term belongs to a different school')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = NEW.school_id
    );
END;

-- Deleting an instance that still has scope rows is refused, so the repository has to clear
-- them itself and each removal becomes a real change-log entry. See deviation 2 above.
CREATE TRIGGER constraint_instance_delete_requires_no_scope
BEFORE DELETE ON constraint_instance FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'remove this constraint''s scope rows before the constraint')
    WHERE EXISTS (SELECT 1 FROM constraint_ref WHERE constraint_id = OLD.id);
END;

-- -------------------------------------------------------------------------- constraint_ref

-- One entity an instance is scoped to.
--
-- Scope lives here rather than inside `params` so that a reference is a real row with real
-- integrity, and `params` holds only scalars -- ADR 0005. `entity_id` is polymorphic and so
-- carries no foreign key, which is the same hole `availability.entity_id` has and is closed
-- the same three ways: the repository checks existence and school ownership on write, the
-- repositories that delete a referable entity clear its scope rows, and an integrity sweep
-- finds anything that got through.
CREATE TABLE constraint_ref (
    id            TEXT    PRIMARY KEY NOT NULL,
    constraint_id TEXT    NOT NULL REFERENCES constraint_instance (id) ON DELETE RESTRICT,
    -- What the reference is *for*. `CONSTRAINTS.md` Sect. 2's roles; a rule with no
    -- `appliesTo` row is school-wide.
    role          TEXT    NOT NULL CHECK (role IN ('appliesTo', 'subject', 'yearLevel',
                                                   'campus', 'timeslot', 'cycleDay',
                                                   'period')),
    -- Which kind of row `entity_id` names. Checked rather than free text so a project
    -- written by a newer build fails loudly here instead of decoding an unknown kind as a
    -- plausible one -- the rule `availability.entity_kind` follows.
    entity_kind   TEXT    NOT NULL CHECK (entity_kind IN ('TEACHER', 'ROOM', 'STUDENT_GROUP',
                                                          'CLASS_SECTION', 'ACTIVITY',
                                                          'SUBJECT', 'YEAR_LEVEL', 'CAMPUS',
                                                          'TIMESLOT', 'CYCLE_DAY', 'PERIOD')),
    entity_id     TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- The uniqueness `DATA-MODEL.md` expressed as a composite primary key. Naming the same
    -- entity twice in one role says nothing more than naming it once.
    UNIQUE (constraint_id, role, entity_id)
) STRICT;

CREATE INDEX idx_constraint_ref_constraint ON constraint_ref (constraint_id);
CREATE INDEX idx_constraint_ref_entity     ON constraint_ref (entity_kind, entity_id);

-- ------------------------------------------------------- student_group.children_are_disjoint

-- Does this group's children share students?
--
-- ADR 0035 Sect. 4. Asked of the PARENT, once, and read by rule 6 of Sect. 3's seven. It
-- means: no two of my child groups share a student.
--
--   NULL -- not stated. Every pair of children is Indeterminate.
--   1    -- declared disjoint. Rule 6 fires, and disjointness propagates to descendants.
--   0    -- "they may overlap", which is worth distinguishing from silence because it tells
--           pre-flight to stop asking.
--
-- NULL IS NOT 0. They give the same overlap answer today and they are different states, and
-- `DisjointnessDeclaration` keeps them apart in the domain for exactly that reason.
--
-- One nullable ADD COLUMN, with no trigger work and no ADR 0020 rebuild: `student_group` is
-- already in `change_log::TRACKED_TABLES` and the change-log triggers interpolate only the
-- table name, `id` and `rev`. Existing projects read NULL, which is the behaviour ADR 0035
-- defines for them -- no re-entry and no re-materialisation.
--
-- There is deliberately no DEFAULT. A default of 0 would be a statement the school has not
-- made, and a default of 1 would be the silent-clash failure the whole record exists to
-- prevent.
ALTER TABLE student_group
    ADD COLUMN children_are_disjoint INTEGER
        CHECK (children_are_disjoint IS NULL OR children_are_disjoint IN (0, 1));
