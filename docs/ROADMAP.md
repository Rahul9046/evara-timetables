# Roadmap — Milestone 1

Milestone 1 is a **complete vertical slice**: create a project, define a school, generate
a timetable, inspect it, edit it by hand, understand why an edit is rejected, and move the
whole project to another machine.

The 19 acceptance items from the product brief are tracked as `M1-1` … `M1-19` and each
phase below states which it closes. Phases are ordered by dependency and each one ends
with something demonstrable — none of them is "build everything in one pass".

---

## Target repository structure

```
evara-timetables/
├── apps/
│   └── desktop/                    # Tauri 2 application (replaces apps/web)
│       ├── index.html
│       ├── vite.config.ts
│       ├── src/                    # React + TypeScript
│       │   ├── app/                # shell, routing, project lifecycle
│       │   ├── features/           # structure, people, curriculum, activities,
│       │   │                       # constraints, generate, timetable, reports
│       │   ├── ipc/                # typed Tauri command wrappers
│       │   └── ui/                 # primitives, grid, drag-and-drop
│       └── src-tauri/
│           ├── Cargo.toml
│           ├── tauri.conf.json
│           └── src/commands/       # thin: project, crud, solve, validate, report, io
├── crates/
│   ├── evara-core/                 # domain types, IDs, cycle/time model, Score (no I/O)
│   ├── evara-db/                   # SQLite: migrations, repositories, loader, writer
│   ├── evara-constraints/          # registry, evaluators, explainers, param schemas
│   ├── evara-solver/               # construction, local search, moves, scorers
│   ├── evara-project/              # .evara export/import + format migrations
│   ├── evara-report/               # view projections, HTML/CSV/PDF renderers
│   ├── evara-integrations/         # external adapters — empty, feature-gated
│   └── evara-cli/                  # headless solve / benchmark / migrate / fixture gen
├── packages/
│   └── domain/                     # GENERATED TypeScript types + UI-side guards
├── fixtures/                       # synthetic schools (JSON + generator + .evara)
├── docs/                           # this directory, plus docs/adr/
└── .github/workflows/              # CI: Windows + macOS, cargo + vitest + fixtures
```

`services/` disappears. `crates/` holds libraries, not services, because there is no
service.

---

## Phase 0 — Clear the ground ✅ complete

Removes what contradicts the architecture before anything is built on top of it.

- Scaffold the Cargo workspace and the Tauri 2 app at `apps/desktop`. Migrate the useful
  parts of `apps/web` (`index.html`, `styles.css` as a visual starting point) and delete
  `apps/web/src/main.tsx`'s hardcoded `lessons` mock.
- Delete `services/solver/`. Do **not** merge `feat/v0.1-solver-service`; tag it
  `archive/timefold-prototype` first so the prototype stays recoverable, then close it.
- Reduce `packages/domain/src/index.ts` to generated output. Nothing hand-written there.
- Extend `.gitignore` for Rust/Tauri (`target/`, `src-tauri/target/`, `*.evara`,
  `*.sqlite*`, `dist/`). Commit the root `package-lock.json` (currently untracked).
- CI on `windows-latest` and `macos-latest`: `cargo fmt --check`, `cargo clippy -D
  warnings`, `cargo test`, `tsc --noEmit`, frontend tests.
- `CLAUDE.md` / contributor notes recording the layering rules from
  [ARCHITECTURE.md](ARCHITECTURE.md) §3.

**Done when:** an empty Tauri window opens on both platforms from a clean clone, and CI is
green. No feature code yet.

**Outcome.** Workspace of 8 crates plus the Tauri app compiles; `npm run dev` opens the
window and a Phase 0 `app_info` command proves the IPC round trip end to end.
`cargo fmt`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `vitest` and the
Vite production build all pass on Windows. macOS is covered by CI but has not been run on
hardware yet. `services/` and `apps/web` are gone; the Java prototype survives only as the
constraint semantics recorded in `docs/CONSTRAINTS.md`.

---

## Phase 1 — Project lifecycle and persistence

Split in three.

**Phase 1A — persistence foundations ✅ complete.** Connection factory with verified
pragmas, forward-only embedded migrations with grouped transactions and a typed
newer-schema refusal, project identity in `app_meta`, and the `change_log` table with its
reusable trigger generator. No entity tables.

**Phase 1B — settings and project lifecycle ✅ complete.** Installation settings database
(`site_id`, recent projects); migration V2 correcting project identity to `project_id` +
`created_by_site_id`; per-connection `evara_site_id()` attribution; `Workspace` with
create/open/close/current/recent over `.evaraproj`; typed Tauri commands; minimal
lifecycle UI; advisory cloud-folder warning. 74 Rust + 15 frontend tests. Still no entity
tables. See [adr/0008](adr/0008-installation-identity-and-project-lifecycle.md).

**Phase 1C — repository foundation and static Structure ✅ complete.** Driver types
removed from the public `DbError` API; the repository write contract (`Stamp::new`,
`update`, `delete`) that owns `rev` and `updated_at`; migration `V3` adding school,
campus, building, room_type, room, resource, academic_year and term with deliberate
RESTRICT/SET NULL semantics; repositories for all eight; and a schema-level test that
discovers entity tables and fails if any lacks change-log triggers. 108 Rust tests. See
[adr/0009](adr/0009-typescript-type-generation.md) and
[adr/0010](adr/0010-structure-deletion-semantics.md).

**Phase 1D — the time model ✅ complete.** Migration `V4` adding cycle, cycle_day,
period_structure, period, timeslot and calendar_day; repositories for all six; and the
timeslot materialiser, which preserves the identifier of every logical slot that survives
a change to the cycle or the bell schedule. Nothing in it deletes a timeslot: a
rematerialisation reports orphan candidates and leaves them alone, and removing them is a
separate call taking explicit identifiers. The model makes no Monday–Friday assumption —
`cycle_day.ordinal` is a day's only scheduling identity, and real dates reach it solely
through explicit `calendar_day` rows. 151 Rust tests. See
[adr/0011](adr/0011-stable-timeslot-identity.md), which also records the four schema
questions settled before the migration was written.

Also in 1D, fixing a Phase 1C defect: `parse_id` substituted `Uuid::nil()` for a malformed
stored identifier, so a damaged project read as a valid one in which unrelated rows shared
the all-zero identifier. It now returns a typed `DbError::CorruptIdentifier`.

**Phase 1E — School Setup ✅ complete.** The interface half of the original 1D scope: a
nine-section School Setup workflow, and 68 typed Tauri commands behind it. School details,
campuses and buildings, rooms and room types, resources, academic years and terms, cycles
and cycle days, bell schedules and periods, calendar mapping, and the timetable grid
preview.

Three things in it are more than CRUD:

- **Type generation is wired up**, pulled forward from Phase 2 because forty types had to
  cross the boundary at once. `ts-rs` generates `packages/domain/src/generated/` from the
  repository records themselves rather than from a parallel DTO layer — see
  [adr/0012](adr/0012-records-are-the-ipc-contract.md) — and `npm run types:check` fails
  the build when the committed output and the Rust disagree.
- **The grid preview implements a preview/confirm/apply contract.** A preview carries a
  fingerprint of the plan it reports; a rebuild sends it back and the repository recomputes
  the plan inside its write transaction, refusing with `ReviewRequired` if it has moved.
  Releasing stranded slots is a separate guarded call that deletes exactly what the
  recomputed plan names, which closes the hole a stale orphan list would otherwise open.
- **Cycle completeness is a domain rule, not a screen's arithmetic.**
  `evara_core::time::CycleCoverage` is the first thing to live in `evara-core`; the
  interface asks it over IPC rather than counting rows itself.

190 Rust tests, 147 frontend tests.

**Closes M1-1, M1-2, M1-15, M1-16.**

---

## Phase 2 — Resources

- Migration `0002`: the People group and `subject`.
- ~~Type generation (`ts-rs`) wired into the build, plus the typed `src/ipc/` wrappers.~~
  Done in Phase 1E.
- ~~One reusable table + form shell in the UI.~~ Done in Phase 1E —
  `app/ui/CrudPanel.tsx` takes a declarative spec and the People screens should use it.
- Remaining from this item: `rev` checking on update, for optimistic concurrency. Phase 1E
  exposes `rev` to the interface but no command takes it as a precondition yet, so a
  second window can still overwrite the first's edit.
- Screens: Teachers, Subjects, Rooms, Student Groups, Year Levels.
- The `availability` table and an availability grid editor shared by teachers, rooms and
  groups.

**Closes M1-3, M1-4, M1-5, M1-6.**

---

## Phase 3 — Curriculum and activities

- Migration `0003`: Curriculum group, then Activities group (activity, activity_teacher,
  activity_group, activity_room_candidate, activity_resource, line, line_member,
  activity_link).
- Activity generator: expand a course's `periods_per_cycle` into activities with
  `split_index`, respecting `default_duration_periods`.
- Screens: Courses → Class sections → Lessons, with teacher and group assignment,
  duration, room requirements, and lock toggles. Lines editor (basic).
- `fixtures/`: a synthetic school generator in `evara-cli` plus 3 checked-in fixtures —
  tiny (feasible by hand), medium (~400 activities), and one deliberately infeasible.

**Closes M1-7.**

---

## Phase 4 — Constraints as data

- Migration `0004`: `constraint_instance`, `constraint_ref`.
- `evara-constraints`: the registry skeleton, `ConstraintEvaluator` trait, JSON Schema per
  type, param validation on write, and the default constraint set seeded into new projects.
- Screens: constraint list grouped by family; per-instance level, weight and enable
  toggle; scope picker writing `constraint_ref`; forms generated from the param schema.

**Closes M1-8** (data and configuration; evaluation is Phase 5).

---

## Phase 5 — Scorer, validator, diagnostics

- Implement the `M1` evaluators from [CONSTRAINTS.md](CONSTRAINTS.md) §3 — roughly 22
  types. One module and one test file each.
- `ReferenceScorer` (full evaluation) and `IncrementalScorer` (indexed), plus the
  property test asserting they agree after random move sequences. This test gates every
  later constraint.
- `Explanation` plumbing and the `validate_move` command.
- Pre-solve feasibility diagnostics ([SOLVER.md](SOLVER.md) §8 Layer 1).

**Done when:** the infeasible fixture produces specific, actionable diagnostics with no
solver involved.

---

## Phase 6 — Solver v1

- Problem loader: database → dense-index `Problem`. Solution writer: `Assignment` →
  `timetable_entry`, transactionally.
- Construction heuristic (most-constrained-first, locks placed first).
- Late Acceptance Hill Climbing with the move set from [SOLVER.md](SOLVER.md) §5, seeded
  RNG, termination config, cancel token.
- Background thread, `solver://progress` events, `solve_run` persistence.
- `evara-cli solve` + `bench`, with score baselines per fixture checked into CI so solver
  regressions are caught.
- UI: Generate screen with settings, live progress, score, cancel, and the hard-violation
  report on infeasibility.

**Closes M1-9.**

---

## Phase 7 — Timetable views

- `evara-report` projections: master, by teacher, by class/group, by room, by student.
- Grid component driven by cycle days × periods, not weekdays. Multi-period activities
  render as spans.
- Score health panel: hard / medium / soft totals with a per-constraint breakdown,
  clicking through to the offending entries.

**Closes M1-10, M1-11, M1-12.**

---

## Phase 8 — Manual editing

- Drag-and-drop in the grid, validating on hover: legal targets tinted, illegal targets
  showing the blocking reason.
- `apply_move` command writing through a transaction; undo/redo stack.
- Violation inspector listing every current violation with entity links.
- Lock/unlock an entry. Repair run: scope filter + `minimise-disruption`, offering a diff
  of what would change before committing.

**Closes M1-13, M1-14.**

---

## Phase 9 — Project transfer

- `evara-project`: writer, reader, manifest, checksums, format migration chain scaffold
  (currently `formatVersion: 1` only).
- Commands `project_export` / `project_import`, native file dialogs, progress for large
  schools.
- The full test set from [PROJECT-FORMAT.md](PROJECT-FORMAT.md) §7, round-trip included,
  in CI.

**Closes M1-17, M1-18.**

---

## Phase 10 — Output

- Print-oriented HTML + a print stylesheet per view; print and "Save as PDF" through the
  webview's print pipeline. A Rust vector-PDF renderer is a later refinement, not an M1
  requirement — see [OPEN-DECISIONS.md](OPEN-DECISIONS.md) D10.
- CSV export per view.
- A renderer trait so additional formats drop in without touching the projections.

**Closes M1-19.**

---

## Phase 11 — Packaging

- MSI/NSIS for Windows, DMG for macOS (universal binary), auto-updater **off**.
- Icons, bundle metadata, third-party licence manifest generated from `cargo-about` and
  `license-checker`.
- Reproducible release build in CI, artefacts attached to a tag.
- Smoke test: install from the artefact, create a project, generate, export, import.

**Closes the milestone.**

---

## After Milestone 1

Rough order, not a commitment: individual student sectioning and course requests
(`request-*`, `section-*`); the remaining relationship and multi-campus constraints;
relaxation-based infeasibility suggestions ([SOLVER.md](SOLVER.md) §8 Layer 3); the
daily-operations layer (absence, substitutions, room changes, merges, yard duties);
CSV import; then vendor adapters in `evara-integrations`, one at a time, each behind a
feature flag and its own fixtures.
