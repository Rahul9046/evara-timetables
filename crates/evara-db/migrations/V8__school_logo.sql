-- A school's own logo, stored inside the project file.
--
-- Forward-only. V1-V7 are untouched.
--
-- See ADR 0016 for the reasoning. The short version of each decision:
--
--   * A table, not a column on `school`. The bytes are up to 2 MiB and `school` is read on
--     almost every screen; putting a BLOB in that row would carry the image across the IPC
--     boundary every time anything asked for the school's name. A separate table means the
--     logo is fetched only when something actually wants to show it.
--
--     It is a first-class entity in the V5 sense -- its own UUIDv7, its own audit columns,
--     its own change-log triggers, its own entry in change_log::TRACKED_TABLES -- because
--     repo::update and repo::delete address rows by id and the triggers read NEW.id and
--     NEW.rev. A composite-keyed side table would sit outside both, and replacing a school
--     logo is a real edit that belongs in the change feed like any other.
--
--   * A BLOB in the project, not a file beside it. The project file IS the unit a user
--     copies, backs up and sends to a colleague, and a logo that lived next to it would be
--     lost by every one of those actions. It also means no absolute path to an external
--     file is ever stored, so a project opened on another machine, or from a different
--     folder, still has its logo. A future `.evara` export reads this table like any
--     other; nothing extra is needed to carry the image.
--
--   * UNIQUE (school_id) makes this strictly one logo per school. A school with no row
--     has no custom logo and falls back to Evara's own -- absence is the fallback
--     condition, so there is no "is_active" flag and no way to have two.
--
--   * ON DELETE RESTRICT, continuing ADR 0010 without exception. There is no
--     delete_school, so this edge is never exercised today; it is RESTRICT so that if one
--     is ever written it has to think about the logo rather than silently dropping it.
--
-- The CHECKs restate evara_core::branding's limits so they hold even against a write that
-- bypasses the repository -- a hand-repaired file, or a future build with a bug. The
-- module is the authority for the message a user sees; these are the backstop.
-- `evara_core::branding::tests` pins MAX_BYTES to the literal below.
--
-- Existing projects need no data migration. A project upgraded from V7 simply has no row
-- here, which is exactly "this school uses the Evara logo" -- the same state a brand new
-- project starts in. Nothing is written, no revision is consumed, and no existing school
-- row is touched.

CREATE TABLE school_logo (
    id         TEXT    PRIMARY KEY NOT NULL,
    school_id  TEXT    NOT NULL REFERENCES school (id) ON DELETE RESTRICT,
    -- The format as read from the file's own header, never from a filename. The set is
    -- closed deliberately: these three are what a WebView renders reliably and what
    -- evara_core::branding can measure without a decoder.
    media_type TEXT    NOT NULL CHECK (media_type IN ('image/png', 'image/jpeg', 'image/webp')),
    -- 2 MiB, matching evara_core::branding::MAX_BYTES.
    bytes      BLOB    NOT NULL CHECK (length(bytes) > 0 AND length(bytes) <= 2097152),
    -- Measured from the header at upload, so a preview can be laid out and a report can
    -- scale the image without decoding it again.
    width      INTEGER NOT NULL CHECK (width BETWEEN 16 AND 2048),
    height     INTEGER NOT NULL CHECK (height BETWEEN 16 AND 2048),
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    -- One logo per school. Both columns are NOT NULL, so no COALESCE is needed for this
    -- to actually hold.
    UNIQUE (school_id)
) STRICT;
