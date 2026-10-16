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
crates/evara-import     spreadsheet parsing, templates -> core ONLY
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
- **Every update and delete the interface can reach takes the caller's revision**, and
  `repo::update` / `repo::delete` put it in the statement's `WHERE` clause. *Read the
  revision, compare it in Rust, then write* is not a concurrency control — it is the same
  race with a longer window — so it is not available: the comparison **is** the predicate.
  A stale write matches nothing, so no column changes, no revision is consumed and no
  change-log entry is appended; it is reported as `DbError::Conflict`, which is deliberately
  not `NotFound`. `Expected::Any` means *no predicate* and is for three documented cases
  only — a cascade inside another operation's transaction, a mutation whose contract is not
  a revision, or a test. **No command handler constructs one.** See
  [docs/adr/0027](docs/adr/0027-revisions-are-checked-on-write.md), whose §3 is the
  inventory of the 84 mutations that existed when it was written;
  [adr/0029](docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §8 extends
  it to Stage C's 16, for 100 mutations of which 53 carry a revision. A new mutation is
  added to one of those inventories, never left out of both.
- **A lesson is a course, a class section, and one row per occurrence.** `subject → course →
  class_section → activity`, and **no table in that chain carries a `school_id`** — every
  school-owned read and write resolves one through the chain with `require_school_owner`.
  One `activity` row is **one placeable occurrence**: four single periods a cycle is four
  rows, and there is no `occurrences_per_cycle` column, because `timetable_entry` is unique
  on `(timetable_id, activity_id, timeslot_id)` and Phase 4's "spread these across four
  days" has to address the occurrences individually. `split_index` is assigned once and
  **never renumbered**. The interface asks *periods per lesson* and *times per cycle* and
  writes `count` rows, which is a creation and so carries no revision precondition.
  [adr/0029](docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md).
- **Giving somebody a lesson needs employment and eligibility, and only *warns* about a
  qualification.** An **active** `teacher_school` membership at the section's school and
  campus eligibility where the section names a campus are refusals — a trigger pair each,
  plus a repository check that names the remedy. A missing `teacher_qualification` is
  reported and refuses nothing: a database that refused would be answered by entering a
  qualification somebody does not have, which destroys the record it was protecting. This
  amends [adr/0025](docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md)
  §4's conjunction and leaves its ownership model alone — **nothing in the lessons code
  writes or removes a qualification.**
  [adr/0028](docs/adr/0028-assigning-a-lesson-warns-about-qualifications.md).
- **A class section's audience is its student groups. The individual roster is never added to
  it.** A section taught to a group of 28 that also names four children has an audience of
  28, because those four are among the 28. ADR 0006 requires a full timetable to be buildable
  from `student_group.size` with no student records at all, so `class_enrolment` is optional
  everywhere and has no scheduling meaning in Milestone 1.
- **Never report that a demand *can* be scheduled.** `evara_core::lessons` reports
  impossibility only — an occurrence longer than any unbroken run of teaching periods, a
  total beyond every slot in the cycle. A demand that is merely difficult is not reported,
  and before the school has a cycle and a bell schedule nothing is judged at all. Only a
  solver can say a timetable exists, and claiming otherwise spends the user's trust on a
  guess.
- **A qualification is a property of the person; a department association is a narrowing of
  school membership.** The two look alike and are governed oppositely:
  `teacher_qualification` carries no `school_id`, requires no membership and survives one
  being withdrawn, while `teacher_department` requires membership, is held to it by a
  trigger pair, and is cleared when it goes. Never add `teacher.department_id` — a single
  column on a project-level identity cannot say "Mathematics at Northgate, Science at
  Southmoor". [adr/0025](docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md)
  and [adr/0026](docs/adr/0026-teacher-departments-are-a-narrowing-of-membership.md).
- **New entity table?** Add it to `change_log::TRACKED_TABLES`. A schema-level test
  discovers entity tables and fails if any lacks its three triggers, so forgetting is a
  red test rather than a silent hole in the change feed.
- **Choose `ON DELETE` deliberately.** Ownership edges are `RESTRICT`; optional
  classification references are `SET NULL`, and the repository clears them with a proper
  update first so `rev` stays truthful. Never default to `CASCADE`. One documented
  departure: `subject.department_id` is `RESTRICT` despite looking like a classification,
  because a database-level `SET NULL` would rewrite rows without bumping `rev` and so
  bypass the precondition above (R10, and ADR 0025 §6).
- **No driver type crosses `evara-db`'s public API.** `rusqlite` and `refinery` errors are
  wrapped in `InternalError`; no other crate may depend on the SQLite driver, even for
  tests.
- **A project holds several schools, and every school-owned read says which.** Since V12
  there is no "the school": a repository list of a school-owned entity takes an explicit
  `school_id`, and **every `update_*` and `delete_*` refuses an identifier belonging to
  another school** (`DbError::WrongSchool`), checked in the same transaction as the write
  and before anything is cleared. A `LIMIT 1` read of `school` would hand the caller an
  arbitrary institution and silently scope everything after it. A child's school is resolved
  through its parent chain, never by adding it a second `school_id` column. Facilities are
  the exception by design — `building_campus` and `room_campus` reach across schools, so a
  shared building or room can be *destroyed* only by its home school and only while nobody
  else shares it, and `room-clash` stays a per-`room_id` rule for the whole project. See
  [docs/adr/0019](docs/adr/0019-multiple-schools-in-one-project.md).
- **Identity is split three ways and must stay that way:** `project_id` and
  `created_by_site_id` live in the project; the installation's `site_id` lives in the
  settings database and reaches the change log through the per-connection
  `evara_site_id()` function. Never store the active installation in a project.
- **`.evaraproj` is the live project; `.evara` is the portable export.** Never use one
  where the other belongs, and never introduce a third extension. A spreadsheet is neither:
  `.xlsx` and `.csv` are read by `evara-import` and are never written as a project.
- **An import is a reviewed plan, and a preview writes nothing.** `Import::plan` runs under
  `db.read`; `Import::apply` runs under `db.write`, **rebuilds the plan inside the
  transaction** and refuses `DbError::ReviewRequired` if the fingerprint has moved. Never
  compare a fingerprint in the interface — that is check-then-act. Never import row by row
  or transaction by row: a failure on the last row must leave none of the others, which is
  the whole reason the engine is in `evara-db` rather than composed from public repository
  methods. [adr/0030](docs/adr/0030-an-import-is-a-reviewed-plan.md).
- **An import never invents a record and never merges an identity.** A row naming something
  the school does not hold is an error on that row — never a placeholder entity. A code
  matches within its school; a name is evidence and produces a `Conflict` the user answers.
  An existing record that disagrees with the file is never overwritten without an answer,
  and an import never deletes an activity. [adr/0031](docs/adr/0031-import-identity-matching.md).
- **A spreadsheet identifier is text, and a formula is never evaluated.** `SourceCell::code`
  refuses a numeric cell: `00124` arriving as `124` has already lost what made it an
  identifier, so it is reported with the remedy rather than guessed at. Dates are
  `YYYY-MM-DD` only — an ambiguous one is an error, never an interpretation. A formula cell
  with no stored result is `Unsupported`; macro workbooks are refused at the extension.
- **An unmapped column is silence; a mapped blank column is an answer.** Unmapped means the
  import has no opinion and the stored value survives; mapped-and-blank means there is no
  value and the field is cleared. Collapsing the two makes "importing twice is safe"
  untrue.
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
