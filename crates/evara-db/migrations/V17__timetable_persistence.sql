-- A generated timetable becomes a row, and a regeneration never overwrites one.
--
-- Forward-only. V1-V16 are untouched.
--
-- Seven tables, which is the whole of M7's schema need:
--
--   * `timetable`              -- one timetable for one (cycle, term). Several may coexist,
--                                 and that coexistence IS the versioning.
--   * `timetable_entry`        -- one row per placeable occurrence: where it starts, and in
--                                 which room. The run it occupies is derived.
--   * `solve_run`              -- how a timetable came to be: engine, seed, limits, the six
--                                 counters, the termination, and the input watermark.
--   * `solve_run_unscheduled`  -- what that run could not place, and which of two kinds of
--                                 answer each lesson got.
--   * `timetable_evaluation`   -- one scoring of a timetable. GENERATION is immutable
--                                 provenance; REVALIDATION is the replaceable current one.
--   * `violation`              -- what an evaluation found, structured.
--   * `violation_ref`          -- the entities one violation names.
--
-- Nothing is rebuilt, no existing row is rewritten, and every existing project reads these
-- tables as empty -- which is the correct state for a project that has never generated.
--
-- ============================================================================ deviations
--
-- `DATA-MODEL.md` Sect. 7 sketches `timetable`, `timetable_entry`, `solve_run`, `violation`
-- and `violation_ref`. Six details depart from that sketch, each because a record or a
-- convention established after it was written says otherwise. They are recorded here rather
-- than in a commit message, because a migration is immutable and the next reader of this
-- file is the person who needs to know.
--
-- 1. ONE ROW PER OCCURRENCE, NOT ONE PER OCCUPIED SLOT.
--
--    The sketch explains its key as being "because a multi-period activity occupies several
--    slots", and ADR 0029 Sect. 2 repeats that as supporting evidence. Both are followed in
--    spirit and not in letter: the key is kept verbatim, and there is one row per occurrence
--    carrying the START slot.
--
--    Two reasons. Rows 2..d are DERIVABLE from the start slot and `activity.duration_periods`,
--    and `CLAUDE.md` fixes the direction of that derivation -- "a duration is a count of
--    periods and the wall-clock interval is derived from the periods covered, never the other
--    way round". Storing the covered slots makes a corrupt timetable representable (three rows
--    for a two-period lesson) and makes a partial multi-period move representable in Phase 8,
--    which is the state the sketch itself calls impossible: "written and moved as a unit".
--
--    `UNIQUE (timetable_id, activity_id)` is added to say so in the schema rather than in a
--    comment: an `Assignment` is one `Option<Placement>` per activity, and this is that type
--    written down.
--
-- 2. NO `school_id`.
--
--    `CLAUDE.md` Sect. 3: "A child's school is resolved through its parent chain, never by
--    adding it a second `school_id` column." `cycle.school_id` is NOT NULL, so a timetable's
--    school is determined. The sketch's column would be the second place for it to be wrong --
--    the argument ADR 0029 Sect. 1 used to keep it off six tables.
--
--    The cost is a join on every school-scoped read, which V15's lesson chain already pays.
--    A trigger pair enforces that `cycle` and `term` agree about the school, because ADR 0032
--    rule 1 scopes a timetable to ONE school and nothing else would stop a row naming
--    Northgate's cycle and Southmoor's term.
--
-- 3. EVERY CHILD EDGE IS `RESTRICT`, AND THE REPOSITORY CLEARS THE ROWS.
--
--    The sketch has `timetable_entry ... ON DELETE CASCADE`. ADR 0040 Sect. 2 settles that
--    this is now wrong, for a reason that applies identically here: SQLite fires a DELETE
--    trigger for a row removed by a foreign-key action only when `recursive_triggers` is on,
--    and this project never enables it. A CASCADE would delete entries with no change-log
--    entry -- silently, and only for the rows reached by the cascade.
--
--    `timetable_entry.timeslot_id` is RESTRICT for a second, independent reason: ADR 0011
--    Sect. 5 says so by name -- "It relies on RESTRICT to refuse any slot a future
--    `timetable_entry` references, which is why timetable entries will not need a cascade."
--
-- 4. `penalty` IS AN INTEGER.
--
--    The sketch says REAL. `CLAUDE.md` Sect. 3: "Scores are integers." ADR 0040 Sect. 3
--    corrected `constraint_instance.weight` the same way and for the same reason, and M6
--    corrected the sign: a penalty is negative-going, so a violation makes a score smaller
--    and `Score::PERFECT` is the top of the order.
--
-- 5. NO `status`, NO `based_on_timetable_id`, AND NO SCORE ON `timetable`.
--
--    `status` with DRAFT/PUBLISHED/ARCHIVED is the published-archived lifecycle, which
--    `SCHEDULING-ARCHITECTURE-DECISIONS.md` Sect. 6 lists among what is NOT in the MVP. A
--    CHECK with three values of which only one is ever written is a column that lies about
--    being used. `based_on_timetable_id` is scenario lineage, which is what REPAIR needs
--    (`SOLVER.md` Sect. 7) and repair is not built. Both are one additive column later.
--
--    The sketch's three score columns are omitted because a score on `timetable` goes stale
--    the moment Phase 8 edits an entry, and a stale score is worse than no score. A score
--    belongs to the EVALUATION that produced it, which is what `timetable_evaluation` is.
--
-- 6. `timetable_evaluation` IS NEW, AND IS WHAT KEEPS HISTORY FROM BEING OVERWRITTEN.
--
--    The sketch hangs `violation` off `timetable` and `solve_run` directly. That cannot
--    express the difference between "what the generator found" and "what is wrong with this
--    timetable now", so a revalidation would have to delete the generation's findings to
--    replace the current ones -- destroying the provenance `solve_run` exists to record.
--
--    So violations belong to an evaluation, an evaluation has a `kind`, and a revalidation
--    replaces only the REVALIDATION row. The GENERATION row is immutable for the life of the
--    timetable. Two partial unique indexes enforce at most one of each.
--
-- ===================================================================== what is NOT here
--
-- No export or import. `violation` and `violation_ref` are derived and are rebuilt by an
-- evaluation (`PROJECT-FORMAT.md` Sect. 3); `solve_run_unscheduled` is NOT derived -- it is
-- historical generation information that cannot be reconstructed once the inputs move -- so
-- Phase 9 exports it. That contract is documented and not implemented here.
--
-- No manual editing. `is_locked` and `is_manual` are written as 0 by the generator and are
-- Phase 8's to set; they are columns rather than a later ALTER because the sketch declares
-- them and because an entry's provenance is not something to add after rows exist.

-- --------------------------------------------------------------------------- timetable

CREATE TABLE timetable (
    id              TEXT    PRIMARY KEY NOT NULL,
    -- The school is `cycle.school_id`. See deviation 2.
    cycle_id        TEXT    NOT NULL REFERENCES cycle (id) ON DELETE RESTRICT,
    -- NULL means a grid materialised for the whole year, which is an ordinary selection and
    -- not a missing value.
    term_id         TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    -- How a human tells two timetables for one scope apart. Defaulted by the caller, never
    -- by the database: only the caller knows what to call it.
    name            TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    generated_at    TEXT    NOT NULL,
    -- ADR 0032 rule 4: "A timetable is never described as conflict-free. It is described as
    -- having no CONFIRMED clash within the school solved, with the unverified counts beside
    -- it." Never nullable and never omitted, because all zeroes is a real state that still
    -- means "no confirmed clash" rather than "clash-free". Named exactly as
    -- `UnverifiedCounts`' four fields, so the serde mapping is one to one.
    indeterminate_student_pairs INTEGER NOT NULL DEFAULT 0
                                CHECK (indeterminate_student_pairs >= 0),
    unchecked_capacities        INTEGER NOT NULL DEFAULT 0
                                CHECK (unchecked_capacities >= 0),
    shared_entities             INTEGER NOT NULL DEFAULT 0
                                CHECK (shared_entities >= 0),
    unenforced_restrictions     INTEGER NOT NULL DEFAULT 0
                                CHECK (unenforced_restrictions >= 0),
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    rev             INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1)
) STRICT;

-- Two timetables for one scope may not share a name.
--
-- A UNIQUE INDEX rather than a table-level UNIQUE, because SQLite allows an expression only
-- in an index. `COALESCE` is what `CLAUDE.md` requires of a nullable column in a uniqueness
-- rule: SQLite treats NULLs as distinct, so without it the constraint would silently fail to
-- hold for exactly the whole-year timetables, which are the common case.
CREATE UNIQUE INDEX idx_timetable_scope_name
    ON timetable (cycle_id, COALESCE(term_id, ''), name);

CREATE INDEX idx_timetable_cycle ON timetable (cycle_id, term_id);

-- A term must belong to the cycle's school.
--
-- `term` reaches its school through `academic_year` and `cycle` carries one directly, so this
-- cannot be a foreign key. V15's and V16's same-school trigger pairs have the same shape, and
-- it is a pair rather than one trigger for the same reason they are: nothing in the repository
-- updates `cycle_id`, and the invariant should not depend on that staying true.
CREATE TRIGGER timetable_term_same_school_insert
BEFORE INSERT ON timetable FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'that term belongs to a different school than the cycle')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        JOIN cycle         ON cycle.id = NEW.cycle_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = cycle.school_id
    );
END;

CREATE TRIGGER timetable_term_same_school_update
BEFORE UPDATE ON timetable FOR EACH ROW
WHEN NEW.term_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'that term belongs to a different school than the cycle')
    WHERE NOT EXISTS (
        SELECT 1
        FROM term
        JOIN academic_year ON academic_year.id = term.academic_year_id
        JOIN cycle         ON cycle.id = NEW.cycle_id
        WHERE term.id = NEW.term_id
          AND academic_year.school_id = cycle.school_id
    );
END;

-- --------------------------------------------------------------------- timetable_entry

CREATE TABLE timetable_entry (
    id           TEXT    PRIMARY KEY NOT NULL,
    timetable_id TEXT    NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    activity_id  TEXT    NOT NULL REFERENCES activity  (id) ON DELETE RESTRICT,
    -- The slot the occurrence STARTS in. A `duration_periods = d` occurrence continues into
    -- later slots of the same schedule and cycle day, and `evara_core::scheduling::span`
    -- resolves which -- one implementation, so the writer, the reader, the evaluators and
    -- Phase 7's grid cannot disagree about what a lesson occupies.
    timeslot_id  TEXT    NOT NULL REFERENCES timeslot  (id) ON DELETE RESTRICT,
    -- NULL is legal and means no room is booked: the state for a section whose course
    -- requires no room type and which was given none. ADR 0034 Sect. 4 prefers an unroomed
    -- lesson to no lesson.
    room_id      TEXT             REFERENCES room      (id) ON DELETE RESTRICT,
    -- Phase 8's, both of them. The generator writes 0 and 0.
    is_locked    INTEGER NOT NULL DEFAULT 0 CHECK (is_locked IN (0, 1)),
    is_manual    INTEGER NOT NULL DEFAULT 0 CHECK (is_manual IN (0, 1)),
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- `DATA-MODEL.md` Sect. 7's key, kept verbatim.
    UNIQUE (timetable_id, activity_id, timeslot_id),
    -- One placement per occurrence -- deviation 1. The type `Assignment` already enforces
    -- this; the schema says it too, so a row cannot describe a state the type cannot hold.
    UNIQUE (timetable_id, activity_id)
) STRICT;

-- `DATA-MODEL.md` Sect. 9's PLANNED block, written as specified.
CREATE INDEX idx_entry_timetable_slot ON timetable_entry (timetable_id, timeslot_id);
CREATE INDEX idx_entry_timetable_act  ON timetable_entry (timetable_id, activity_id);
CREATE INDEX idx_entry_room           ON timetable_entry (timetable_id, room_id, timeslot_id);

-- An entry's timeslot must belong to the timetable's own cycle.
--
-- Not a foreign key: `timeslot` carries `cycle_id`, and the pairing is between two of this
-- row's own references. Without it a timetable for one cycle could hold a placement in
-- another cycle's grid, which no loader would ever produce and which nothing else would
-- refuse -- and ADR 0032 rule 1 scopes a timetable to one cycle.
CREATE TRIGGER timetable_entry_slot_in_cycle_insert
BEFORE INSERT ON timetable_entry FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'that timeslot belongs to a different cycle than the timetable')
    WHERE NOT EXISTS (
        SELECT 1
        FROM timeslot
        JOIN timetable ON timetable.id = NEW.timetable_id
        WHERE timeslot.id = NEW.timeslot_id
          AND timeslot.cycle_id = timetable.cycle_id
    );
END;

CREATE TRIGGER timetable_entry_slot_in_cycle_update
BEFORE UPDATE ON timetable_entry FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'that timeslot belongs to a different cycle than the timetable')
    WHERE NOT EXISTS (
        SELECT 1
        FROM timeslot
        JOIN timetable ON timetable.id = NEW.timetable_id
        WHERE timeslot.id = NEW.timeslot_id
          AND timeslot.cycle_id = timetable.cycle_id
    );
END;

-- --------------------------------------------------------------------------- solve_run

CREATE TABLE solve_run (
    id             TEXT    PRIMARY KEY NOT NULL,
    timetable_id   TEXT    NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    -- Which engine produced it. `SolverEngine` is a reversibility seam (`SOLVER.md` Sect. 9),
    -- so a run records which implementation ran rather than assuming there is one.
    engine         TEXT    NOT NULL CHECK (length(trim(engine)) > 0),
    -- The whole point of the row: "a run whose seed was not recorded cannot be reproduced,
    -- which makes a regression undebuggable and a benchmark baseline meaningless".
    seed           INTEGER NOT NULL,
    status         TEXT    NOT NULL CHECK (status IN
                       ('COMPLETE', 'PARTIAL', 'INFEASIBLE', 'CANCELLED', 'FAILED')),
    -- Why the engine stopped looking, which is a different question from what came back.
    -- `Termination::key()`'s eight values.
    termination    TEXT    NOT NULL CHECK (termination IN
                       ('complete', 'exhausted', 'candidate-limit-reached',
                        'step-limit-reached', 'pass-limit-reached', 'cancelled',
                        'input-rejected', 'internal-check-failed')),
    -- `SolveLimits`, as JSON. The `constraint_instance.params` precedent: `json_valid` is the
    -- floor and the shape is the caller's business.
    settings       TEXT    NOT NULL DEFAULT '{}'
                       CHECK (json_valid(settings) AND json_type(settings) = 'object'),
    -- The change-log sequence number the inputs stood at when the problem was loaded. The
    -- freshness check compares against it INSIDE the writing transaction; stored afterwards
    -- it is provenance only, and nothing decides anything from the stored copy.
    inputs_watermark INTEGER NOT NULL CHECK (inputs_watermark >= 0),
    -- `SolveStatistics`' six counters, named as its fields are. Deterministic, so two runs of
    -- one problem at one seed store identical figures and a baseline can be compared.
    candidate_evaluations INTEGER NOT NULL CHECK (candidate_evaluations >= 0),
    candidates_rejected   INTEGER NOT NULL CHECK (candidates_rejected   >= 0),
    placements_committed  INTEGER NOT NULL CHECK (placements_committed  >= 0),
    placements_withdrawn  INTEGER NOT NULL CHECK (placements_withdrawn  >= 0),
    construction_steps    INTEGER NOT NULL CHECK (construction_steps    >= 0),
    passes                INTEGER NOT NULL CHECK (passes                >= 0),
    -- The CALLER's clock. The solver reads none, which is why `SolveStatistics` carries no
    -- duration: a duration on the outcome would make whole-outcome equality -- how determinism
    -- is tested -- machine-dependent. Elapsed time is the difference of these two.
    started_at     TEXT    NOT NULL,
    finished_at    TEXT    NOT NULL,
    app_version    TEXT    NOT NULL,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL,
    rev            INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    CHECK (finished_at >= started_at)
) STRICT;

CREATE INDEX idx_solve_run_timetable ON solve_run (timetable_id);

-- --------------------------------------------------------------- solve_run_unscheduled

-- What a run could not place. Historical, and NOT derived: once the inputs move, nothing can
-- reconstruct which lessons a past run failed on or why. That is why Phase 9 exports it.
CREATE TABLE solve_run_unscheduled (
    id              TEXT PRIMARY KEY NOT NULL,
    solve_run_id    TEXT NOT NULL REFERENCES solve_run (id) ON DELETE RESTRICT,
    activity_id     TEXT NOT NULL REFERENCES activity  (id) ON DELETE RESTRICT,
    -- `UnplacedCause::key()`'s seven values. The first four are properties of the PROBLEM and
    -- may be read as "no timetable can place this"; the last three are properties of this RUN
    -- and may not. `UnplacedCause::is_impossible()` is the one place that line is drawn, and
    -- it is a pure function of this column rather than a second stored flag.
    cause           TEXT NOT NULL CHECK (cause IN
                      ('no-resolved-bell-schedule', 'no-contiguous-run', 'no-candidate-room',
                       'all-candidates-unavailable', 'blocked-by-placements', 'not-reached',
                       'input-rejected')),
    -- The pre-solve finding that already proved it, where there was one.
    diagnostic_code TEXT,
    -- The engine-authored sentence is NOT stored. A wording improvement would leave old rows
    -- reading the old way; a cause survives a wording change and a string does not. The
    -- section name a sentence needs is joined through `activity -> class_section`.
    UNIQUE (solve_run_id, activity_id)
) STRICT;

CREATE INDEX idx_unscheduled_run ON solve_run_unscheduled (solve_run_id);

-- ---------------------------------------------------------------- timetable_evaluation

-- One scoring of one timetable. See deviation 6.
CREATE TABLE timetable_evaluation (
    id            TEXT    PRIMARY KEY NOT NULL,
    timetable_id  TEXT    NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    -- GENERATION is immutable provenance and belongs to a run. REVALIDATION is the current
    -- snapshot and is replaced wholesale, because two scorings of a changing world are not
    -- additive.
    kind          TEXT    NOT NULL CHECK (kind IN ('GENERATION', 'REVALIDATION')),
    solve_run_id  TEXT             REFERENCES solve_run (id) ON DELETE RESTRICT,
    evaluated_at  TEXT    NOT NULL,
    -- Where the inputs stood when this scoring ran, so a reader can say "N changes have
    -- happened since" without the figure ever deciding anything.
    inputs_watermark INTEGER NOT NULL CHECK (inputs_watermark >= 0),
    -- Negative-going. A violation makes a score smaller and all zeroes is the best reachable.
    score_hard    INTEGER NOT NULL CHECK (score_hard   <= 0),
    score_medium  INTEGER NOT NULL CHECK (score_medium <= 0),
    score_soft    INTEGER NOT NULL CHECK (score_soft   <= 0),
    -- How many occurrences this scoring found placed, against how many the problem held. A
    -- partial timetable is an ordinary state, so both numbers are kept rather than a ratio.
    placed_count   INTEGER NOT NULL CHECK (placed_count   >= 0),
    required_count INTEGER NOT NULL CHECK (required_count >= 0),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    CHECK (placed_count <= required_count),
    -- A generation scoring is a run's; a revalidation has no run.
    CHECK ((kind = 'GENERATION') = (solve_run_id IS NOT NULL))
) STRICT;

-- At most one of each kind per timetable. Two partial indexes rather than one on
-- `(timetable_id, kind)`, because that is the same constraint and these two say which is
-- which at the point a reader meets them.
CREATE UNIQUE INDEX idx_evaluation_one_generation
    ON timetable_evaluation (timetable_id) WHERE kind = 'GENERATION';
CREATE UNIQUE INDEX idx_evaluation_one_revalidation
    ON timetable_evaluation (timetable_id) WHERE kind = 'REVALIDATION';

CREATE INDEX idx_evaluation_timetable ON timetable_evaluation (timetable_id);

-- --------------------------------------------------------------------------- violation

-- Derived: what an evaluation found. Rebuilt by re-evaluating, and not exported
-- (`PROJECT-FORMAT.md` Sect. 3).
CREATE TABLE violation (
    id              TEXT    PRIMARY KEY NOT NULL,
    evaluation_id   TEXT    NOT NULL REFERENCES timetable_evaluation (id) ON DELETE RESTRICT,
    -- The registry's `type_key`. Stored rather than an enum, because the registry is open and
    -- a project written by a newer build keeps findings this one cannot name.
    constraint_type TEXT    NOT NULL CHECK (length(trim(constraint_type)) > 0),
    -- The instance's level, which is not necessarily the shipped default: ADR 0039 lets a
    -- school raise one.
    level           TEXT    NOT NULL CHECK (level IN ('HARD', 'MEDIUM', 'SOFT')),
    -- The stored instance that produced it, or NULL for a mandatory built-in -- which is the
    -- ordinary case, because no constraint rows are seeded (ADR 0039).
    constraint_id   TEXT             REFERENCES constraint_instance (id) ON DELETE RESTRICT,
    -- Negative-going, already multiplied by the instance's weight. INTEGER: deviation 4.
    penalty         INTEGER NOT NULL CHECK (penalty <= 0),
    -- Where the violation happens, for a violation that has a when. `cycle_day.ordinal` is a
    -- day's only scheduling identity; nothing here reads a weekday.
    cycle_day_id    TEXT             REFERENCES cycle_day (id) ON DELETE RESTRICT,
    -- The wall-clock span the clash was decided on. ADR 0032 rule 2: the clash key is an
    -- interval and never a timeslot identifier, so a clash violation carries the interval.
    -- 'HH:MM', zero-padded, so text order is chronological -- the `period` convention.
    starts_at       TEXT             CHECK (starts_at IS NULL OR
                                            starts_at GLOB '[0-2][0-9]:[0-5][0-9]'),
    ends_at         TEXT             CHECK (ends_at IS NULL OR
                                            ends_at GLOB '[0-2][0-9]:[0-5][0-9]'),
    -- The specific fact, for a message that has to be more than the rule's name. A
    -- `Violation` is structured and never a pre-rendered sentence; `explain()` writes the
    -- prose, and it lives with the rule so it cannot drift from the check.
    detail          TEXT    NOT NULL,
    CHECK ((starts_at IS NULL) = (ends_at IS NULL)),
    CHECK (ends_at IS NULL OR ends_at > starts_at)
) STRICT;

CREATE INDEX idx_violation_evaluation ON violation (evaluation_id, level);

-- The entities one violation names. Derived, with `violation`.
CREATE TABLE violation_ref (
    id           TEXT PRIMARY KEY NOT NULL,
    violation_id TEXT NOT NULL REFERENCES violation (id) ON DELETE RESTRICT,
    -- `activity` is one of the occurrences at fault; `subject` is what the rule is about;
    -- `related` is everything else a reader needs to see.
    role         TEXT NOT NULL CHECK (role IN ('activity', 'subject', 'related')),
    -- Polymorphic, so no foreign key -- the `constraint_ref` precedent, and `DATA-MODEL.md`
    -- Sect. 7 accepts the cost there for the same reason: normalising the reference into a
    -- real row is what pays back the absence of a typed edge.
    entity_kind  TEXT NOT NULL CHECK (length(trim(entity_kind)) > 0),
    entity_id    TEXT NOT NULL,
    -- The order the engine produced them in, so a report reads the same way twice.
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 0),
    UNIQUE (violation_id, role, ordinal)
) STRICT;

CREATE INDEX idx_violation_ref_violation ON violation_ref (violation_id);
