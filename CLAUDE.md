# Working rules for Claude in this repository

Evara Timetables is an **open-source, local-first desktop** school timetabling
application. React + TypeScript · Tauri 2 · Rust · SQLite, shipped as one signed binary
per platform.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before making structural changes, and
[docs/ROADMAP.md](docs/ROADMAP.md) to see which phase the work belongs to.

---

## 1. Hard product rules

These are not preferences. A change that breaks one of them is wrong regardless of how
well it works.

1. **No cloud, no server, no network.** No code path in normal operation may make a
   network request. No localhost server, no port, no sidecar process. Offline is a
   correctness property, not a feature.
2. **No authentication.** No login, logout, accounts, users, sessions, roles, tokens or
   permissions. Do not add a `user` table "for later".
3. **No real school data, ever.** Fixtures, tests, examples, screenshots and bug-report
   attachments use synthetic data only. No real student, teacher, parent or school
   names.
4. **Permissive licences only.** MIT / Apache-2.0 / BSD / ISC. No GPL or AGPL in shipped
   binaries, no paid SaaS, no mandatory external service. `cargo deny check licenses`
   must pass.
5. **No vendor concepts in the core.** PowerSchool, Veracross, aSc, Timetabling
   Solutions, Edval and TimeTabler features are modelled as *generic* scheduling
   concepts and constraints. When a vendor concept does not fit, add a new generic
   constraint type with a vendor-neutral name — never a vendor branch, field or
   `if` in the core.

## 2. Layering

```
apps/desktop/src        React — view layer, no rules, no SQL
apps/desktop/src-tauri  command handlers — orchestration only
crates/evara-project    .evara import/export        -> core, db
crates/evara-report     projections and renderers   -> core
crates/evara-integrations external adapters         -> core ONLY
crates/evara-db         the only crate with SQL     -> core
crates/evara-solver     generation                  -> core, constraints
crates/evara-constraints scheduling rules           -> core
crates/evara-core       domain types, no I/O        -> nothing
```

Each of the following is an architecture bug, not a style question:

- a scheduling rule in TypeScript, or any rule implemented twice
- SQL outside `evara-db`
- `evara-solver` or `evara-constraints` opening a database or a file
- business logic inside a Tauri command handler
- `evara-core` depending on another `evara-*` crate, or doing I/O
- a vendor name anywhere outside `crates/evara-integrations` and the vendor-mapping
  table in `docs/CONSTRAINTS.md`
- a hand-written TypeScript entity type that mirrors a Rust one (generate it instead)

**The single most important rule:** the constraint evaluators in `evara-constraints` are
shared by automatic generation, timetable scoring, manual move validation and violation
explanation. One implementation, four callers. If generation and the editor can ever
disagree about whether a placement is legal, the product is broken.

## 3. Conventions

- **Rust**: edition 2024. `cargo fmt` is authoritative. `cargo clippy` must be clean —
  lints are declared in the workspace `Cargo.toml` and inherited, so do not add crate
  level `allow`s to silence them; fix the code or argue the exception in review.
  `unsafe_code` is forbidden workspace-wide.
- **IDs**: UUIDv7, stored as `TEXT`, stable across export and import. Never renumber.
- **Timestamps**: ISO-8601 UTC with millisecond precision, as `TEXT`.
- **Every mutable table** carries `created_at`, `updated_at`, `rev`, and a `change_log`
  trigger. This is what keeps future synchronisation possible; do not skip it.
- **SQLite**: `foreign_keys = ON` is per-connection and off by default — it belongs in
  the connection factory, never at a call site. Never construct a `rusqlite::Connection`
  outside `evara_db::connection`; go through `Database`, and write through
  `Database::write` so every mutation is in a transaction.
- **A released migration is immutable.** refinery checksums applied migrations, so editing
  a shipped `migrations/*.sql` file makes every existing project refuse to open. Fix
  forward with a new numbered migration. There are no down migrations, ever.
- **A migration that adds an entity table must call `change_log::install` for it**, or
  that table is silently absent from the change feed.
- **Repositories write `rev` and `updated_at` on every update.** Never a self-updating
  trigger. Change-log triggers only observe what the repository wrote. In practice:
  go through `repo::update` and `repo::delete`, which own those columns — a repository
  must never name `rev` or `updated_at` in its own SQL.
- **New entity table?** Add it to `change_log::TRACKED_TABLES`. A schema-level test
  discovers entity tables and fails if any lacks its three triggers, so forgetting is a
  red test rather than a silent hole in the change feed.
- **Choose `ON DELETE` deliberately.** Ownership edges are `RESTRICT`; optional
  classification references are `SET NULL`, and the repository clears them with a proper
  update first so `rev` stays truthful. Never default to `CASCADE`.
- **No driver type crosses `evara-db`'s public API.** `rusqlite` and `refinery` errors are
  wrapped in `InternalError`; no other crate may depend on the SQLite driver, even for
  tests.
- **Identity is split three ways and must stay that way:** `project_id` and
  `created_by_site_id` live in the project; the installation's `site_id` lives in the
  settings database and reaches the change log through the per-connection
  `evara_site_id()` function. Never store the active installation in a project.
- **`.evaraproj` is the live project; `.evara` is the portable export.** Never use one
  where the other belongs, and never introduce a third extension.
- **A persisted identifier that will not parse is corruption, not input.** Never
  substitute a default, never panic — return `DbError::CorruptIdentifier`. A nil-UUID
  fallback makes every damaged row compare equal to every other, which is worse than the
  failure it hides.
- **A timeslot's identity is the tuple `(term_id, cycle_day_id, period_id)`, and its `id`
  is assigned once.** Rematerialising the grid must preserve the `id` of every logical slot
  that still exists; `ordinal` and the other derived columns are expected to change.
  Materialisation **never deletes** — it reports orphan candidates and leaves them alone.
  See [docs/adr/0011](docs/adr/0011-stable-timeslot-identity.md).
- **Nothing in scheduling may read a weekday.** `cycle_day.ordinal` is a day's only
  scheduling identity; `label` and `weekday_hint` are display. Real dates reach the model
  only through explicit `calendar_day` rows.
- **A nullable column in a uniqueness rule needs `COALESCE`.** SQLite treats `NULL`s as
  distinct in a `UNIQUE` index, so the constraint silently fails to hold for exactly the
  rows where the column is unset — often the common case.
- **Naming**: `snake_case` in SQL and Rust, `camelCase` in JSON and TypeScript, mapped in
  exactly one place with serde attributes.
- **Solving is deterministic.** Seeded RNG, recorded on the run. No `Instant::now()` or
  unseeded randomness inside the solver's decisions.
- **Scores are integers.** Three levels, compared lexicographically. Weights become
  integer penalties at compile time, not at scoring time.

## 4. Definition of done for a constraint

A constraint type is not "supported" until all five exist:

1. a stable `type_key` and a registry entry in `crates/evara-constraints/`
2. a JSON Schema for its parameters, next to the evaluator
3. documented semantics in [docs/CONSTRAINTS.md](docs/CONSTRAINTS.md)
4. an isolated unit test, plus a synthetic fixture exercising it
5. an `explain()` implementation producing a human-readable reason

The reference-vs-incremental scorer property test must stay green. It is the gate that
keeps incremental scoring honest, and a wrong delta silently produces a wrong timetable.

## 5. Process

- **Work the current phase only.** Phases exist so each one ships something
  demonstrable. Do not implement SQLite entities, CRUD screens, constraints or solver
  heuristics ahead of their phase.
- **Do not commit, push, merge or open a PR** unless explicitly asked.
- **Do not merge `feat/v0.1-solver-service`.** It is the archived Java/Timefold
  prototype, superseded by [docs/adr/0002-rust-in-process-solver.md](docs/adr/0002-rust-in-process-solver.md).
- Record a decision that rejects a reasonable alternative as an ADR in `docs/adr/`.
  Decisions still open belong in [docs/OPEN-DECISIONS.md](docs/OPEN-DECISIONS.md).
- Run `npm run check` before reporting work complete, and report failures with their
  output rather than summarising them away.

## 6. Commands

| Command | Does |
| --- | --- |
| `npm run dev` | Launch the desktop app (Vite + Tauri, hot reload) |
| `npm run build` | Build installers for the current platform |
| `npm run check` | Everything CI runs: fmt, clippy, cargo test, tsc, vitest |
| `npm run rust:test` | Rust tests only |
| `npm run typecheck` | TypeScript only |
| `cargo run -p evara-cli -- --help` | Headless CLI |
