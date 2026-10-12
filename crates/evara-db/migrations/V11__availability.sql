-- Availability: when an entity cannot, or would rather not, be timetabled.
--
-- DATA-MODEL.md §6 "Availability — one table for all entity kinds" is the source of this
-- schema. One table, six entity kinds, because the question "when is this thing free?" has
-- the same shape for a teacher, a room and a cohort, and the solver asks it the same way.
--
-- **Available is the default, so only exceptions are stored.** A teacher with no rows here
-- is available in every slot. That is what keeps the table small and what makes "I have
-- not configured this yet" and "this teacher is never free" different states rather than
-- the same empty set.
--
-- Three granularities, all expressible, all in one row:
--
--   * `cycle_day_id` alone  -> "all of Monday"
--   * `period_id` alone     -> "period 1, every day"
--   * both                  -> one recurring cell, which is what the grid editor writes
--   * `timeslot_id` alone   -> one exact materialised slot, term included
--
-- At least one must be present; a row naming none of them would be a rule about nothing.
--
-- Deliberate departure from the documented model, extending
-- docs/adr/0018-people-identity-and-deactivation.md §5 to this table: `weight` is INTEGER
-- rather than REAL. Phase 4 sums and compares it, CLAUDE.md requires that arithmetic to be
-- integer ("scores are integers; weights become integer penalties at compile time"), and a
-- REAL would make the record non-`Eq` while every other entity in this crate is. NULL means
-- "unset — let the constraint decide", which is the honest value for a phase that does not
-- collect it.
--
-- Deletion semantics:
--
--   * `cycle_day_id`, `period_id`, `timeslot_id` and `term_id` are RESTRICT, like every
--     other reference to them. This is what stops a cycle-day deletion from silently
--     discarding the availability configured against it: the delete is refused and the
--     user is told, rather than the rule disappearing with the day.
--   * `entity_id` is polymorphic and therefore carries no foreign key — the one place in
--     this schema where a reference cannot be enforced by the database. The repository
--     checks the entity exists on write, clears an entity's rules when that entity is
--     deleted, and `Availability::orphans` sweeps for any that got through.
--
-- Forward-only. V1-V10 are untouched.

CREATE TABLE availability (
    id             TEXT    PRIMARY KEY NOT NULL,
    school_id      TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    entity_kind    TEXT    NOT NULL CHECK (entity_kind IN ('TEACHER', 'ROOM', 'STUDENT_GROUP',
                                                           'RESOURCE', 'CAMPUS', 'SUBJECT')),
    -- Polymorphic: the row this rule is about, in the table `entity_kind` names. No foreign
    -- key is possible; see the header.
    entity_id      TEXT    NOT NULL,
    -- One exact materialised slot. Carries its own term, so `term_id` is redundant with it
    -- and the two are not combined by anything that writes here.
    timeslot_id    TEXT             REFERENCES timeslot (id) ON DELETE RESTRICT,
    cycle_day_id   TEXT             REFERENCES cycle_day (id) ON DELETE RESTRICT,
    period_id      TEXT             REFERENCES period (id) ON DELETE RESTRICT,
    -- NULL means the rule applies all year, which is the common case and not the exotic one.
    term_id        TEXT             REFERENCES term (id) ON DELETE RESTRICT,
    status         TEXT    NOT NULL CHECK (status IN ('UNAVAILABLE', 'PREFERRED', 'DISCOURAGED')),
    -- Soft weight for PREFERRED and DISCOURAGED. Meaningless for UNAVAILABLE, which is a
    -- hard rule. NULL means unset; Phase 4 supplies the constraint's own default.
    weight         INTEGER          CHECK (weight IS NULL OR weight >= 0),
    reason         TEXT,
    -- Date-bounded exceptions -- a term's maternity cover, a room closed for refurbishment.
    -- Carried now per ADR 0006 and not collected by any screen in this phase; the editor
    -- writes recurring rules only.
    effective_from TEXT             CHECK (effective_from IS NULL
                                           OR (date(effective_from) IS NOT NULL
                                               AND effective_from = date(effective_from))),
    effective_to   TEXT             CHECK (effective_to IS NULL
                                           OR (date(effective_to) IS NOT NULL
                                               AND effective_to = date(effective_to))),
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL,
    rev            INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- A rule has to be about some time, or it is about nothing.
    CHECK (timeslot_id IS NOT NULL OR cycle_day_id IS NOT NULL OR period_id IS NOT NULL),
    CHECK (effective_from IS NULL OR effective_to IS NULL OR effective_to >= effective_from)
) STRICT;

-- One rule per entity per scope. Every scope column is nullable, and here COALESCE is
-- exactly right -- the opposite of `teacher_code_unique`, which is a *partial* index for
-- the opposite reason.
--
-- The difference is worth stating once. A teacher's staff code is unique *when given*, and
-- rows without one must not collide, so the index excludes them. Two availability rules
-- that both say "all of Monday, all year" are the *same rule written twice*, and must
-- collide — so the unset columns are folded to a value that compares equal. SQLite treats
-- NULLs as distinct, so without COALESCE this constraint would silently fail to hold for
-- precisely the common case.
CREATE UNIQUE INDEX idx_availability_identity ON availability (
    entity_kind,
    entity_id,
    COALESCE(timeslot_id, ''),
    COALESCE(cycle_day_id, ''),
    COALESCE(period_id, ''),
    COALESCE(term_id, '')
);

-- Every RESTRICT delete scans the children to decide whether to refuse. The unique index
-- above leads on entity_kind, so none of these are covered by it.
CREATE INDEX idx_availability_cycle_day ON availability (cycle_day_id);
CREATE INDEX idx_availability_period ON availability (period_id);
CREATE INDEX idx_availability_timeslot ON availability (timeslot_id);
CREATE INDEX idx_availability_term ON availability (term_id);
