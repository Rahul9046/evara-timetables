# Evara Timetables — Architecture

Evara Timetables is an **open-source, local-first desktop application** for building
constraint-based school timetables. It is vendor-neutral: features found in
PowerSchool PowerScheduler, Veracross, aSc TimeTables, Timetabling Solutions, Edval/Tes
and TimeTabler are modelled as **generic scheduling concepts and constraints**, never as
vendor-specific behaviour.

Status: **design phase.** This document is the target architecture. See
[ROADMAP.md](ROADMAP.md) for what is actually built.

---

## 1. Non-negotiable properties

| Property | Consequence |
| --- | --- |
| Local-first | All data lives in a file on the user's machine. No server is required to create, edit, generate, view, import or export a timetable. |
| Offline | No network call is on any normal code path. The app must function with networking disabled. |
| No authentication | No login, accounts, sessions, roles or tokens anywhere in the codebase. |
| Desktop | Windows and macOS. Tauri 2 shell, native installers. |
| Open source | Apache-2.0. Every dependency must be permissively licensed (MIT/Apache-2.0/BSD/ISC). No paid SaaS, no mandatory external service, no copyleft in shipped binaries. |
| Sync-ready, not synced | The schema carries stable UUIDs, row revisions and a change log so synchronisation can be added later. Nothing cloud is implemented now. |

---

## 2. Shape of the system

```
┌──────────────────────────────────────────────────────────────┐
│  apps/desktop/src          React + TypeScript (view layer)   │
│  ─ project browser, editors, timetable grids, reports        │
│  ─ holds NO scheduling rules and NO persistence logic        │
└───────────────────────────┬──────────────────────────────────┘
                            │ Tauri IPC — typed commands + events
┌───────────────────────────┴──────────────────────────────────┐
│  apps/desktop/src-tauri    thin command layer                │
│  ─ maps commands to crate calls, owns app state & windows    │
└───────────────────────────┬──────────────────────────────────┘
          ┌─────────────────┼─────────────────┬────────────────┐
          │                 │                 │                │
┌─────────┴──────┐ ┌────────┴───────┐ ┌───────┴──────┐ ┌───────┴───────┐
│ evara-db       │ │ evara-constr.  │ │ evara-solver │ │ evara-project │
│ SQLite,        │ │ registry of    │ │ construction │ │ .evara import │
│ migrations,    │ │ evaluators +   │ │ + local      │ │ /export with  │
│ repositories   │ │ explainers     │ │ search       │ │ format migra. │
└─────────┬──────┘ └────────┬───────┘ └───────┬──────┘ └───────┬───────┘
          └─────────────────┴────────┬────────┴────────────────┘
                            ┌────────┴────────┐
                            │ evara-core      │  domain types, IDs,
                            │ (no I/O)        │  cycle/time model, score
                            └─────────────────┘
```

Three further crates sit at the edges: `evara-report` (timetable projections →
HTML/CSV/PDF), `evara-import` (spreadsheet bytes → a parsed table, and the downloadable
templates) and `evara-integrations` (future external adapters, empty and feature-gated).
`evara-cli` is a headless binary used for tests, fixtures and solver benchmarking.

`evara-import` is the one with a dependency set nothing else should inherit — `calamine`,
`csv`, `rust_xlsxwriter` — which is why it is its own crate rather than a module of
`evara-db`: the crate whose charter is SQL has no business also linking three file-format
libraries. It depends on `evara-core` alone and knows nothing about schools. The import
*engine* — resolving a row to a record, planning, and applying the plan in one transaction —
is in `evara-db`, because `Database::write` is `pub(crate)` and an atomic multi-entity write
is only expressible from inside it. See [adr/0030](adr/0030-an-import-is-a-reviewed-plan.md)
§1–2.

### Why the backend is Rust, not TypeScript

Tauri already requires a Rust toolchain, so Rust adds no new dependency. Putting the
domain, database, constraints and solver in Rust buys:

- **One implementation of every scheduling rule.** The solver and the "is this manual
  move legal?" validator call the *same* evaluator code. A TypeScript copy of the rules
  would drift (the existing repo already demonstrates this: `packages/domain` models
  `teacherIds: ID[]` while the Java `Lesson` has a single `teacherId`).
- **Transactional integrity.** Migrations, foreign keys and multi-table writes stay in
  one place instead of being spread across UI handlers.
- **Solver speed** without a second runtime, and `cargo test` for the parts that matter.
- **A headless path.** `evara-cli` can solve a fixture in CI with no window.

TypeScript keeps everything it is good at: the entire interface, grid interaction,
drag-and-drop, local view state.

### The domain is defined once, in Rust

DTO types crossing the IPC boundary are declared in `evara-core` and **generated** into
`packages/domain` as TypeScript (`ts-rs`, or `tauri-specta` if we also want generated
command wrappers). `packages/domain` becomes build output plus hand-written UI helpers —
never a second hand-maintained model.

---

## 3. Layering rules

1. `evara-core` has no I/O and no dependency on the other crates. It owns entity types,
   ID types, the cycle/calendar/timeslot model, `Score`, `Violation` and `Move`.
2. `evara-constraints` depends on `evara-core` only. It never touches SQLite. The solver
   and the validator both consume it. **It holds one kind of thing**: `type_key`-addressed
   evaluators with a JSON Schema and an `explain()`, each scored against a candidate
   timetable. A rule that has an answer *before* any placement exists is not one of those
   and belongs in `evara-core` — `lessons::DemandFinding` is the example, and its module
   header says why putting it here would have been the mistake.
3. `evara-solver` depends on `evara-core` + `evara-constraints`. It receives a fully
   materialised problem **in memory** and returns a solution. It never opens a database.
4. `evara-db` is the only crate that knows SQL. It *will* load a `Problem` for the solver and
   persist a `Solution` back — the loader is M4's and the writer M7's, and neither exists at
   `V15`. The rule is stated now because it decides where that code goes when it arrives:
   the translation from UUIDs to the solver's dense indexes happens here, once, and no other
   crate acquires a database handle to avoid it.
5. `evara-import` may depend on `evara-core` and nothing else. It turns bytes into an
   `evara_core::import::SourceTable` and writes templates; it never opens a file, never
   reaches the network, never evaluates a formula and never runs a macro. It is given bytes
   and a name — the name only to choose a parser.
6. `evara-integrations` may depend on `evara-core` and nothing else. Import adapters
   produce core types; the core never references a vendor.
7. `src-tauri` orchestrates. Business logic does not live in command handlers.

A rule in the UI, SQL in the solver, or a vendor name in the core are all architecture
bugs.

### The write contract is one of these rules

`evara-db`'s `repo` module owns two things no repository may reimplement, because a second
copy is a second place for them to go wrong:

- **`Stamp` and `rev`/`updated_at` maintenance.** A repository never names those columns in
  its own SQL; `repo::update` appends them. See
  [OPEN-DECISIONS](OPEN-DECISIONS.md) D17.
- **The revision precondition.** Since Stage B, `repo::update` and `repo::delete` take an
  `Expected` and put it in the statement's `WHERE` clause, so a save built on a version
  somebody else has since changed is refused rather than applied. A command handler never
  constructs `Expected::Any`; it wraps the `expectedRev` the interface sent. See
  [adr/0027](adr/0027-revisions-are-checked-on-write.md), and
  [adr/0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §8 for the
  inventory Stage C extended.
- **A child's school comes from its parent chain.** Since `V15` the longest one is four
  tables — `subject → course → class_section → activity` — and not one of the six Lessons
  tables holds a `school_id`. `require_school_owner` takes an owner query per table, written
  next to the entity it is about, because the chain *is* the domain statement. A second
  column recording the same ownership would be a second place for it to be wrong. See
  [adr/0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §1.

A read-compare-write revision check would be an architecture bug of the same kind as a rule
in the UI: it looks like the thing and is not.

---

## 4. Key documents

| Document | Contents |
| --- | --- |
| [DATA-MODEL.md](DATA-MODEL.md) | SQLite schema, domain boundaries, sync-readiness columns |
| [SOLVER.md](SOLVER.md) | Solver architecture, algorithms, explanation and repair |
| [CONSTRAINTS.md](CONSTRAINTS.md) | Constraint registry, levels, parameters |
| [PROJECT-FORMAT.md](PROJECT-FORMAT.md) | The `*.evara` portable project format |
| [IMPORT.md](IMPORT.md) | Spreadsheet roster imports: formats, mapping, matching |
| [ROADMAP.md](ROADMAP.md) | Milestone 1 broken into implementation phases |
| [OPEN-DECISIONS.md](OPEN-DECISIONS.md) | Risks and decisions still to be made |
| [SCHEDULING-READINESS-AUDIT.md](SCHEDULING-READINESS-AUDIT.md) | What is built towards timetable generation, what is not, and the milestone sequence to get there |
| [SCHEDULING-ARCHITECTURE-DECISIONS.md](SCHEDULING-ARCHITECTURE-DECISIONS.md) | The four decisions that precede solver code, the scheduling context and conflict contracts they fix, and which future schema changes they require |
| [adr/](adr/) | Accepted architecture decision records |

---

## 5. What the existing repository gets wrong

Recorded here so the migration is deliberate rather than accidental. Details and the
removal plan are in [adr/0002-rust-in-process-solver.md](adr/0002-rust-in-process-solver.md)
and [ROADMAP.md](ROADMAP.md) Phase 0.

| Artefact | Problem |
| --- | --- |
| `services/solver/` + branch `feat/v0.1-solver-service` | A Spring Boot REST service with `@CrossOrigin(origins="*")`. Assumes a server, a JVM at runtime and a network hop. Incompatible with a self-contained offline desktop app. |
| `docs/ARCHITECTURE.md` (previous version) | Mandated "Keep optimization separate from the web application" and a Timefold service. Superseded by this document. |
| `apps/web/src/main.tsx` | A static mock with a hardcoded `lessons` array. Useful as a visual reference; it is not an application. |
| `packages/domain/src/index.ts` | Flat and timeslot-centric. No school, campus, term, calendar, cycle, course or enrolment. No UUID/timestamp/revision discipline. Constraints have no scope. Already diverged from the Java model. |
| `fixtures/basic-school/problem.json` | Shaped for the old flat model. Must be regenerated. |
| `.gitignore` | No Rust/Tauri entries. |

Nothing here is working code that we would be rewriting without reason — the only
executable artefacts are a static page and an unmerged solver prototype. The constraint
*semantics* in `TimetableConstraintProvider.java` are worth keeping as a specification
for the Rust port, and are carried forward in `CONSTRAINTS.md` as `CORE-001..CORE-012`.
