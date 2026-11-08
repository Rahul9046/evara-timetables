-- The replacement recovery journal.
--
-- Replacing a project is several renames, and a crash between any two of them leaves a
-- state somebody has to reason about on the next launch. This table is what makes that
-- reasoning possible rather than a guess: each phase is committed *before* the rename it
-- authorises, so whatever the filesystem looks like afterwards, there is a record of what
-- was being attempted and which paths were involved.
--
-- # Why this lives in the settings database and not in a project
--
-- Three reasons, and the third is decisive:
--
--   1. Evara owns this database outright. It is not a user document.
--   2. It is transactional and in WAL, so a phase commit is durable.
--   3. It is the only store reachable **at startup without knowing which project to look
--      at** — and recovery runs before any project is open, precisely because the project
--      that needs recovering may be the one that will not open.
--
-- Putting it in a `.evaraproj` would mean the record of how to repair a file lives inside
-- the file being repaired.
--
-- # This is not a project migration
--
-- The settings database has its own refinery chain. No `.evaraproj` is touched by this
-- file, and M10 adds no project migration: V18 is the schema a transfer reads and copies.
--
-- # Backward compatibility, stated rather than discovered
--
-- `Settings::open_at` runs this chain with `set_abort_missing(true)`, so a settings
-- database advanced to V2 is **refused by any build that does not embed this migration**.
-- The practical effect is one-way: after running a build with M10, downgrading to an
-- earlier Evara fails to open its own settings until the file is removed.
--
-- Accepted deliberately. The settings database is per-installation and holds only the
-- installation identity and the recent-project list, so the cost of recreating it is a
-- forgotten recent list and a new `site_id` -- not school data, which lives in project
-- files this migration cannot reach. Project downgrade is already unsupported for the same
-- forward-only reason (`migrations.rs`), so this introduces no new class of restriction.

CREATE TABLE pending_replacement (
    -- UUIDv7, so rows sort by when the replacement began.
    id           TEXT    PRIMARY KEY NOT NULL,
    -- How far the protocol *intended* to have got. Deliberately not a description of the
    -- disk: the row is committed before the rename it authorises, so recovery reads this as
    -- intent and the filesystem as truth.
    phase        TEXT    NOT NULL CHECK (phase IN ('STAGED', 'ASIDE', 'PLACED')),
    -- The project file being replaced.
    destination  TEXT    NOT NULL CHECK (length(trim(destination)) > 0),
    -- The staging directory, a sibling of the destination so no rename crosses a volume.
    staging_dir  TEXT    NOT NULL CHECK (length(trim(staging_dir)) > 0),
    -- Where the replaced project will be kept. NULL until the protocol knows the name,
    -- which is only once a replacement -- rather than a plain create -- is under way.
    keep_name    TEXT             CHECK (keep_name IS NULL OR length(trim(keep_name)) > 0),
    started_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL
) STRICT;

-- At most one in-flight replacement per destination.
--
-- Two concurrent replacements of one file is not a state the protocol has an answer for,
-- and the uniqueness is what turns it from a race into a refusal. `destination` is NOT NULL
-- so this needs no COALESCE -- CLAUDE.md's rule applies to nullable columns in a uniqueness
-- rule, and this column cannot be null.
CREATE UNIQUE INDEX pending_replacement_destination_unique
    ON pending_replacement (destination);

-- Recovery reads every row at startup, oldest first, so an interrupted run is explained in
-- the order the attempts happened.
CREATE INDEX idx_pending_replacement_started ON pending_replacement (started_at);
