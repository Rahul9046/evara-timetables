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

Two further crates sit at the edges: `evara-report` (timetable projections → HTML/CSV/PDF)
and `evara-integrations` (future external adapters, empty and feature-gated).
`evara-cli` is a headless binary used for tests, fixtures and solver benchmarking.

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
   and the validator both consume it.
3. `evara-solver` depends on `evara-core` + `evara-constraints`. It receives a fully
   materialised problem **in memory** and returns a solution. It never opens a database.
4. `evara-db` is the only crate that knows SQL. It loads a `Problem` for the solver and
   persists a `Solution` back.
5. `evara-integrations` may depend on `evara-core` and nothing else. Import adapters
   produce core types; the core never references a vendor.
6. `src-tauri` orchestrates. Business logic does not live in command handlers.

A rule in the UI, SQL in the solver, or a vendor name in the core are all architecture
bugs.

---

## 4. Key documents

| Document | Contents |
| --- | --- |
| [DATA-MODEL.md](DATA-MODEL.md) | SQLite schema, domain boundaries, sync-readiness columns |
| [SOLVER.md](SOLVER.md) | Solver architecture, algorithms, explanation and repair |
| [CONSTRAINTS.md](CONSTRAINTS.md) | Constraint registry, levels, parameters |
| [PROJECT-FORMAT.md](PROJECT-FORMAT.md) | The `*.evara` portable project format |
| [ROADMAP.md](ROADMAP.md) | Milestone 1 broken into implementation phases |
| [OPEN-DECISIONS.md](OPEN-DECISIONS.md) | Risks and decisions still to be made |
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
