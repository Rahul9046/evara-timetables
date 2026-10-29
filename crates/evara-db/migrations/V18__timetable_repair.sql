-- A repaired timetable is a new version that knows which one it came from.
--
-- Forward-only. V1-V17 are untouched, nothing is rebuilt, and no existing row is rewritten:
-- three added columns and one index, with defaults that make every V17 row already correct.
-- A released migration is immutable, so additive is not a style preference here -- it is what
-- lets an existing project open without a data migration.
--
-- WHY NO TABLE IS REBUILT, and what that forbids
--
-- `timetable` and `solve_run` are STRICT (V17 lines 143 and 302). SQLite cannot ALTER a CHECK
-- constraint, so widening one means the 12-step in-place rebuild of ADR 0020. Two consequences
-- shaped this file:
--
--   * `timetable_evaluation.kind` keeps its CHECK of ('GENERATION', 'REVALIDATION'). A repair
--     writes GENERATION -- it IS this version's original, immutable scoring, which is ADR 0042
--     Sect. 5's meaning exactly. `solve_run.kind` below is the discriminator instead, and it
--     lives with the seed, the engine and the counters, which is where "how did this come to
--     be" already belongs.
--   * `solve_run_unscheduled.cause` keeps its seven values. A repair reports only what it
--     ATTEMPTED, so an out-of-scope occurrence that was already unplaced is absent from that
--     table rather than carrying an eighth "out of repair scope" cause. The preview reports it
--     as still-unplaced, which is a projection and needs no column.
--
-- ADR 0020 is therefore not engaged, and this migration stays additive.

-- ---------------------------------------------------------------- timetable lineage

-- Which timetable this one was repaired from, where it was.
--
-- NULL for every generated timetable, which is every row V17 can hold -- so the default is
-- correct for existing data without touching it.
--
-- RESTRICT because lineage is an ownership edge: a source with repaired descendants may not
-- vanish and leave them claiming a parent that is gone. SET NULL was rejected because it
-- rewrites rows without bumping `rev`, which is the precise reason `subject.department_id` is
-- RESTRICT despite looking like a classification (R10, ADR 0025 Sect. 6); CASCADE was rejected
-- because deleting one timetable would silently delete every repair derived from it.
--
-- `Timetables::delete` counts descendants first and refuses with a typed error, so the user
-- reads "three repairs descend from this one" rather than an opaque foreign-key failure. This
-- column is the backstop that keeps that true even for a future writer that forgets the check.
--
-- Added nullable with a NULL default, which is what SQLite requires of a column carrying a
-- REFERENCES clause while `foreign_keys = ON`.
ALTER TABLE timetable ADD COLUMN based_on_timetable_id TEXT
    REFERENCES timetable (id) ON DELETE RESTRICT;

-- Following a lineage downwards -- "what descends from this?" -- is the question `delete` asks
-- on every timetable deletion, so it is an index rather than a scan.
CREATE INDEX idx_timetable_based_on ON timetable (based_on_timetable_id);

-- ---------------------------------------------------------------- run provenance

-- Whether this run generated a timetable from nothing or repaired an existing one.
--
-- DEFAULT 'GENERATION' makes every existing row correct: V17 could only record a generation.
--
-- Deliberately NOT the same column as `timetable_evaluation.kind`, which answers a different
-- question (is this scoring the immutable original or the replaceable current one?) over a
-- different value set. Two columns named `kind` on two tables is a readability cost accepted
-- here because the alternative is a rebuild.
ALTER TABLE solve_run ADD COLUMN kind TEXT NOT NULL DEFAULT 'GENERATION'
    CHECK (kind IN ('GENERATION', 'REPAIR'));

-- What the repair was allowed to move, as JSON.
--
-- The scope kind and its selection, so a stored repair can say "one teacher's load, this
-- teacher" rather than only "a repair happened". `'{}'` for a generation, which has no scope.
--
-- `json_valid` is the floor and the shape is the caller's business -- the
-- `constraint_instance.params` precedent (V16 line 100), followed again by
-- `solve_run.settings` (V17 line 279). A schema that tried to constrain the shape here would
-- be a second statement of a Rust type.
ALTER TABLE solve_run ADD COLUMN scope TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(scope) AND json_type(scope) = 'object');

-- No `change_log::install` call is needed. `timetable` and `solve_run` are already in
-- `TRACKED_TABLES`, this migration adds no table, and the triggers interpolate only the table
-- name, `id` and `rev` -- the same reason V16's `student_group.children_are_disjoint` needed
-- no entry. `install_all` runs after migrations on every open.
