# ADR 0003 — SQLite as canonical store, one file per project, sync-ready schema

- Status: **Accepted**
- Date: 2026-10-06

## Context

Data must stay on the machine, with migrations, foreign keys, transactions, stable UUIDs,
timestamps and change metadata. Synchronisation and realtime viewing are expected later,
so the schema must not make them impossible — while no cloud code is written now.

## Decision

**SQLite is the canonical local database. One SQLite file is one school project**, opened
like a document.

- Create / Open / Recent Projects. A separate small settings database holds window state
  and the recent-files list, never school data.
- A project contains one `school` row and may contain many `campus` rows, so multi-campus
  is supported inside a project. Multiple schools means multiple files.
- Connection factory sets `foreign_keys = ON` (off by default in SQLite, and
  per-connection), `journal_mode = WAL`, `busy_timeout`, `synchronous = NORMAL`. Covered
  by a test.
- Forward-only numbered migrations, embedded in the binary, applied in one transaction,
  recorded in `schema_migrations`. Opening a project newer than the binary fails with a
  clear message rather than reading it partially.
- Every entity table carries `id TEXT` (UUIDv7), `created_at`, `updated_at` (ISO-8601
  UTC), `rev INTEGER`, and `deleted_at` where tombstones will be needed.
- Triggers on every mutable entity table append to a `change_log` table recording
  `site_id`, table, entity id, operation and revision.

The `change_log` and the `rev`/`site_id` columns are the **only** concession to future
synchronisation. No sync protocol, no vector clocks, no network code.

## Consequences

- Export becomes a transformation of one file rather than a query across tenants, which
  is what makes [ADR 0004](0004-evara-project-format.md) simple.
- `rev` gives optimistic concurrency now (rejecting stale writes from a long-open form)
  and merge input later.
- Triggers rather than repository code means change-log coverage is guaranteed by the
  schema instead of by developer discipline, at the cost of migration boilerplate.
- `TEXT` UUIDs cost ~20 bytes per key versus `BLOB(16)`, accepted so project files stay
  readable in any SQLite browser — valuable when debugging a user-submitted file.
- WAL-mode SQLite inside OneDrive/iCloud/Dropbox is a known corruption source. The app
  must detect common synced paths and warn on open, and `.evara` export must be the
  blessed way to move a project (OPEN-DECISIONS R8).
- Changing the cycle or period structure rematerialises `timeslot` and can orphan
  placements; destructive changes require explicit confirmation showing the count
  (OPEN-DECISIONS R6).

## Alternatives considered

**A single application database holding many schools.** Worse for portability and export,
and it invents a tenancy concept the product does not have.

**JSON or YAML files on disk.** No transactions, no foreign keys, no indexed queries, and
whole-file rewrites that risk loss on crash.

**An embedded document store (sled, redb, LMDB).** Fast key-value storage, but we need
relational queries, referential integrity and ad-hoc reporting — all of which would have
to be rebuilt.

**Postgres embedded.** Far heavier than a desktop document needs, and awkward to ship.
