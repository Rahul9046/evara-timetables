# ADR 0007 — rusqlite + refinery, with a bundled SQLite and a single connection factory

- Status: **Accepted** (approved 2026-10-06, resolves OPEN-DECISIONS D4)
- Date: 2026-10-06

## Context

[ADR 0003](0003-sqlite-document-model.md) settled that SQLite is the canonical store and
that one file is one project. It left the driver and migration runner open as D4:
`rusqlite` + `refinery` versus `sqlx` with its built-in migrator.

## Decision

**`rusqlite` 0.37 with the `bundled` feature, and `refinery` 0.10 for migrations.**

- **`rusqlite`, not `sqlx`.** A single-user desktop document has no concurrent-request
  workload, so `sqlx`'s async machinery buys nothing and costs a runtime. The solver and
  the loader are synchronous, and `rusqlite` is a thin, well-understood wrapper.
- **`bundled`.** SQLite is compiled from source into the binary rather than taken from
  the host. Every installation then runs an identical, known engine with a known feature
  set — a desktop app cannot rely on whatever SQLite happens to sit on a user's machine,
  and macOS in particular ships an old build.
- **`refinery`.** Embedded, numbered, forward-only migrations with checksum verification,
  which is exactly the model [ADR 0003](0003-sqlite-document-model.md) describes. It also
  shares our `rusqlite` version through its `rusqlite-bundled` feature, so there is one
  SQLite in the build.

Two implementation decisions follow, and matter more than the crate choice:

**One connection factory, and nothing else can make a connection.** `foreign_keys` is off
by default in SQLite *and scoped to a single connection*. A connection opened anywhere
else would silently accept referential corruption. So `Database` owns its
`rusqlite::Connection` and never hands it out; the constructor is `pub(crate)`; writing
goes through `Database::write`, which is the only route to a transaction.

**Pragmas are verified, not assumed.** Applying a pragma can fail quietly. Every
connection reads `foreign_keys`, `journal_mode`, `busy_timeout` and `synchronous` back and
refuses to open if any disagrees ([`DbError::PragmaRejected`]). The test for this provokes
a real foreign-key violation rather than reading the pragma, because the pragma reporting
`1` is not the same claim as the constraint being enforced.

**Migrations run as one group in one transaction** (`set_grouped(true)`), with
`abort_divergent` and `abort_missing` on. A batch that fails leaves the project exactly as
it was. A half-migrated project file is the worst outcome available, and SQLite's
transactional DDL means we can simply refuse to produce one.

## Consequences

- Synchronous persistence throughout. If a long write ever blocks the UI, it moves to a
  background thread — the same place the solver already runs — rather than infecting the
  data layer with async.
- Build time and binary size grow: `bundled` compiles SQLite, which is the main cost in a
  clean build of this crate. Worth it for a predictable engine.
- `refinery` 0.10 requires Rust 1.92, which raised the workspace MSRV. Consistent with
  D13: we ship the binary, so a conservative MSRV only blocks updates.
- Released migrations become immutable. refinery checksums applied migrations, so editing
  a shipped file makes every existing project refuse to open. Corrections ship as a new
  numbered migration, and this needs to be stated wherever contributors are told how to
  add a table.
- We read refinery's `refinery_schema_history` table directly to answer "what schema
  version is this project?" before deciding whether to migrate. That is a mild coupling to
  a table name refinery owns; it is stable and documented, and the alternative — asking
  refinery to tell us, which requires it to create its table first — would mean touching a
  project we may be about to refuse.
- `rusqlite` 0.40 exists but `refinery` 0.10 pins 0.37. Taking the newer driver would fork
  SQLite into two versions in one binary, so we stay on 0.37 until refinery moves.

## Alternatives considered

**`sqlx` with `sqlx::migrate!`.** Compile-time-checked queries are genuinely attractive,
but they require a database at build time, and the async runtime is pure overhead for a
single-user document. Its migrator is also less strict about checksums.

**Hand-rolled migrations over `PRAGMA user_version`.** No dependency and complete control,
but it means reimplementing checksum verification, history recording and grouped
transactions — all of which are exactly the parts that must not have bugs.

**System SQLite (`rusqlite` without `bundled`).** Smaller binary, faster builds, and an
engine that varies by machine — including macOS builds old enough to lack `STRICT` tables,
which the schema uses.
