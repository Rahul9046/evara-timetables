-- Bootstrap: project identity and the change feed.
--
-- Deliberately contains NO domain tables. Its purpose is to establish the persistence
-- guarantees that every entity table added in later phases will inherit:
--   * stable project and installation identity
--   * a schema version that can be compared against the running binary
--   * a change feed that future synchronisation can replay
--
-- Forward-only. This file is immutable once released: refinery checksums applied
-- migrations, so editing it would make every existing project fail to open. Corrections
-- ship as a new numbered migration.

-- Key/value project metadata. A table rather than `PRAGMA user_version` because we need
-- several values (project id, installation id, app versions), not one integer.
CREATE TABLE app_meta (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- The change feed. Every mutable entity table gets triggers writing here, so coverage is
-- guaranteed by the schema rather than by developers remembering.
--
-- This is the ONLY concession made to future synchronisation. There is no sync protocol,
-- no vector clock and no network code anywhere in Evara.
CREATE TABLE change_log (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    site_id      TEXT NOT NULL,
    entity_table TEXT NOT NULL,
    entity_id    TEXT NOT NULL,
    op           TEXT NOT NULL CHECK (op IN ('INSERT', 'UPDATE', 'DELETE')),
    rev          INTEGER NOT NULL,
    at           TEXT NOT NULL
) STRICT;

-- Replaying one entity's history, and resuming a feed from a sequence number.
CREATE INDEX idx_change_log_entity ON change_log (entity_table, entity_id, seq);
CREATE INDEX idx_change_log_at ON change_log (at);
