-- What kind of repeating pattern a cycle is: a week, or a named rotation.
--
-- Forward-only. V1-V8 are untouched.
--
-- See ADR 0017 for the reasoning. The short version of each decision:
--
--   * A `kind` column on `cycle`, not two tables and not a flag per behaviour. The two
--     kinds are the same entity with the same days, the same timeslots and the same
--     calendar mapping; what differs is *who decides what the positions are*. For a
--     WEEKLY cycle the calendar decides and they are fixed; for a CUSTOM cycle the user
--     decides how many there are and what each is called. That is one discriminator, so
--     it is one column.
--
--   * DEFAULT 'CUSTOM' for every cycle that already exists. This is the whole of the
--     upgrade story and it is deliberately the kind that *asserts nothing*. A CUSTOM
--     cycle claims only what every pre-V9 cycle already was: a declared number of
--     positions, each with a label the user chose. WEEKLY carries a real invariant --
--     seven positions, position N is ISO weekday N -- and an existing six-day rotation,
--     or a five-day cycle whose days are called "Day A".."Day E", satisfies none of it.
--     Classifying those as WEEKLY would make the column a lie about rows nobody looked
--     at.
--
--     As in V7, `ALTER TABLE ADD COLUMN` with a constant default does not rewrite the
--     stored rows -- SQLite returns the default for rows written before the column
--     existed -- so every existing cycle keeps its `id`, its `rev`, its `updated_at` and
--     its change-log history exactly as they were. Every existing `cycle_day` keeps its
--     `id` too: this migration creates, deletes and renumbers nothing. Nothing here
--     consumes a revision or appends to the change feed, which is correct -- the cycles
--     did not change, the schema did.
--
--   * No `ON DELETE` work, no new table, no index. `kind` is an attribute, not a
--     relationship, so there is no edge to choose semantics for.
--
-- No change_log work is needed. `cycle` has been in change_log::TRACKED_TABLES since V4,
-- `install_all` reinstalls triggers on every open, and those triggers record only id, op
-- and rev -- they name no columns, so a new column is already covered.
--
-- What this column must never become: a place to put a school's calendar. A cycle is a
-- *reusable definition*. It carries no start date, no starting rotation position, no
-- working-week selection and no holiday policy, and adding one here would bind the shared
-- definition to one institution's calendar. Which real date is which cycle day is settled
-- by an explicit `calendar_day` row and nothing else (ADR 0011), and which of a weekly
-- cycle's seven positions a particular school teaches on is read from
-- `school.working_days` at the moment of display -- never stored against the cycle. That
-- is what lets two schools, or two campuses on different weeks, share one cycle.

ALTER TABLE cycle
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'CUSTOM'
        CHECK (kind IN ('WEEKLY', 'CUSTOM'));

-- A weekly cycle is the week. Seven positions, no more and no fewer.
--
-- The repository derives the length from the kind rather than trusting the caller, so
-- these are backstops in the sense V4's timeslot triggers are: the rule is stated where
-- the storage is, so a future repository, the CLI, an import or a hand-repair cannot
-- produce a "weekly" cycle that is five days long. A table-level CHECK would be the
-- natural home, but `ALTER TABLE` cannot add one, and rebuilding `cycle` is unavailable
-- here -- it is a STRICT table with RESTRICT children, and the rebuild procedure needs
-- `PRAGMA foreign_keys`, which is a no-op inside refinery's transaction (ADR 0013, and
-- ADR 0015 section 6 for the same hazard).
CREATE TRIGGER cycle_weekly_is_seven_days_insert
BEFORE INSERT ON cycle FOR EACH ROW
WHEN NEW.kind = 'WEEKLY' AND NEW.day_count <> 7
BEGIN
    SELECT RAISE(ABORT, 'a weekly cycle is seven days long');
END;

CREATE TRIGGER cycle_weekly_is_seven_days_update
BEFORE UPDATE ON cycle FOR EACH ROW
WHEN NEW.kind = 'WEEKLY' AND NEW.day_count <> 7
BEGIN
    SELECT RAISE(ABORT, 'a weekly cycle is seven days long');
END;

-- A cycle's kind never changes after it is created.
--
-- Converting between the two is not an edit, it is a different cycle. Becoming WEEKLY
-- would require the days to be the seven weekdays, and the only way to get there from a
-- six-day rotation is to delete days -- which `timeslot` and `calendar_day` hold with
-- RESTRICT precisely so that removing a day is a deliberate, confirmed act. Becoming
-- CUSTOM would quietly drop the invariant that the surviving rows were created under.
-- Either way the honest answer is a new cycle, so the column is immutable here as well as
-- in the repository.
CREATE TRIGGER cycle_kind_is_immutable
BEFORE UPDATE OF kind ON cycle FOR EACH ROW
WHEN NEW.kind <> OLD.kind
BEGIN
    SELECT RAISE(ABORT, 'a cycle''s kind cannot be changed after it is created');
END;

-- A weekly cycle's position N is ISO weekday N. Monday is 1, Sunday is 7.
--
-- `ordinal` remains the day's only scheduling identity, exactly as before: nothing in
-- materialisation, ordering or identity reads `weekday_hint`, and this trigger does not
-- make it do so. What it does is forbid the two from *disagreeing* on a weekly cycle,
-- which is what makes "the weekday identity is stable" a property of the file rather than
-- a convention the repository happens to follow. On a CUSTOM cycle `weekday_hint` stays
-- what it has always been -- optional, free, and display-only.
CREATE TRIGGER cycle_day_weekly_identity_insert
BEFORE INSERT ON cycle_day FOR EACH ROW
WHEN EXISTS (SELECT 1 FROM cycle WHERE cycle.id = NEW.cycle_id AND cycle.kind = 'WEEKLY')
     AND (NEW.weekday_hint IS NULL OR NEW.weekday_hint <> NEW.ordinal)
BEGIN
    SELECT RAISE(ABORT, 'a weekly cycle''s day carries the weekday of its position');
END;

CREATE TRIGGER cycle_day_weekly_identity_update
BEFORE UPDATE ON cycle_day FOR EACH ROW
WHEN EXISTS (SELECT 1 FROM cycle WHERE cycle.id = NEW.cycle_id AND cycle.kind = 'WEEKLY')
     AND (NEW.weekday_hint IS NULL OR NEW.weekday_hint <> NEW.ordinal)
BEGIN
    SELECT RAISE(ABORT, 'a weekly cycle''s day carries the weekday of its position');
END;
