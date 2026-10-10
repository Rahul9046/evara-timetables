-- Application settings: what belongs to this *installation*, not to any project.
--
-- Deliberately minimal. Only what Phase 1B needs: the installation identity and the
-- recent-project list. No speculative settings — a setting nothing reads is a schema
-- change nobody can safely remove later.
--
-- This database never contains school data. It lives in the platform application data
-- directory, while school data lives in `.evaraproj` files wherever the user keeps them.

CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- Recently opened projects, newest first.
--
-- Keyed by `path_key` rather than `path`: Windows paths are case-insensitive, so
-- `C:\Work\School.evaraproj` and `c:\work\school.evaraproj` are the same file and must not
-- produce two entries. `path` keeps the original spelling for display.
CREATE TABLE recent_project (
    path_key        TEXT PRIMARY KEY NOT NULL,
    path            TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    project_id      TEXT,
    last_opened_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_recent_project_opened ON recent_project (last_opened_at DESC);
