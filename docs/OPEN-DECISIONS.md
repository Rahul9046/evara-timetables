# Open Decisions and Risks

Decisions that are **not** settled, and risks worth naming before code depends on them.
Resolved items move into [adr/](adr/).

---

## Decisions needed before coding

### D1 — Solver engine: Rust in-process vs. JVM sidecar ✅ RESOLVED 2026-10-06

**Decided: Rust, in-process.** See [adr/0002](adr/0002-rust-in-process-solver.md).
Approved with two conditions, both now recorded in the ADR: keep the `SolverEngine`
abstraction so CP-SAT or another engine can be introduced later without redesigning the
application, and share the constraint evaluators across generation, scoring, manual move
validation and violation explanation.

Original analysis, retained: **Rust, in-process.** Full analysis in [SOLVER.md](SOLVER.md) §1. The
deciding arguments are that the manual-move validator must share the constraint code with
the generator, and that Tauri already requires Rust while a JVM sidecar adds a bundled
runtime, a second notarisation target and 1–3 s of startup latency. Timefold's
multi-threaded solving is also Enterprise-only and commercial, which does not fit an
open-source project.

### D2 — Rust-owned data layer vs. TypeScript-owned

**Recommendation: Rust owns the database and the domain; TypeScript is the view layer**,
with DTO types generated from Rust into `packages/domain`.

Cost: a meaningful amount of Rust CRUD. (The toolchain itself is no longer a cost —
Rust 1.99 was installed in Phase 0.) The alternative — `tauri-plugin-sql` with queries in
TypeScript — would be faster to start and worse within weeks: the solver would then need
its own loader anyway, and transactions would be spread across UI handlers.

### D3 — Type generation tool ✅ RESOLVED 2026-10-06

**Decided: `ts-rs`**, with the per-command wrappers staying hand-written. Reviewed in
Phase 1C once real entities existed; see
[adr/0009](adr/0009-typescript-type-generation.md). The deciding facts: `tauri-specta`
is still `2.0.0-rc.25` with no stable release, and its main benefit — generating the
call layer — would replace wrappers that already do more, because they validate the
payload at runtime rather than asserting a type over it.

Not wired up in Phase 1C by design; generation arrives in Phase 2 with the first
screens that consume entities.

### D4 — SQLite driver and migration runner ✅ RESOLVED 2026-10-06

**Decided: `rusqlite` 0.37 (`bundled`) + `refinery` 0.10.** Implemented in Phase 1A; see
[adr/0007](adr/0007-rusqlite-refinery.md) for the reasoning and its consequences —
notably that released migrations are immutable, and that `refinery` pins `rusqlite` 0.37
so we do not take 0.40 until it moves.

### D5 — Group-based vs. request-based scheduling first ✅ RESOLVED 2026-10-06

**Decided: group-based first, request-based schema preserved from day one.** See
[adr/0006](adr/0006-group-based-scheduling-first.md). Both traditions must ultimately be
supported. Original analysis retained below.

These are different optimisation problems, not variations:

- **Group-based** (aSc, TimeTabler, much of Europe/Australia): classes are pre-formed;
  the solver places lessons. This is what [SOLVER.md](SOLVER.md) describes.
- **Request-based** (PowerScheduler, Veracross, Edval electives, much of the US): students
  submit course requests; the system must *build* sections, assign students to them
  (**sectioning**), construct elective lines, and schedule — typically as separate
  interacting phases.

**Recommendation:** model both in the database from day one (the schema in
[DATA-MODEL.md](DATA-MODEL.md) does), but solve group-based in Milestone 1 and add a
sectioning phase in Milestone 2. Sectioning on top of a working group-based engine is an
addition; retro-fitting the data model later is not.

~~This needs your answer on market priority.~~ Answered: build group-based first, and
preserve the schema for individual requests, weighted requests, alternate requests,
elective line generation, sectioning, class balancing and student allocation. No
request-based algorithm is implemented in Milestone 1.

### D6 — Multi-period activities: one activity or several linked

**Recommendation: one activity with `duration_periods = N`** occupying consecutive
periods. Alternative (N single-period activities + a `CONSECUTIVE` link) makes "double
period" a rule that can be switched off, which it is not. Consequence: every evaluator
must reason about slot *ranges*, and room/teacher occupancy indexes are built over spans.
Cheap to get right at the start, expensive to add later.

### D7 — Is the teacher a planning variable?

Milestone 1 fixes teachers on activities (`activity_teacher`) and plans timeslot + room.
Real staffing ("find any qualified Maths teacher who is free") needs teacher as a third
planning variable, which changes move generation and adds `teacher-qualified` and
`teacher-balanced-load` to the hot path. **Recommendation:** keep teachers fixed in M1,
but shape `Problem`/`Placement` so a third variable is additive rather than a rewrite.

### D8 — UUID version and storage

**Recommendation: UUIDv7 stored as `TEXT`.** v7 is time-ordered, so primary-key index
locality is good and insertion order is recoverable — both useful for a future change
feed. `TEXT` over `BLOB(16)` costs ~20 bytes per key but keeps the database readable in
any SQLite browser, which matters for debugging and for user-submitted project files.
Reconsider only if a measured index-size problem appears.

### D9 — Undo/redo

Options: in-memory inverse-command stack (simple, lost on close), a persisted command
log (survives restart, enables an audit trail, more work), or SQLite savepoints (natural
for transactions, awkward across UI sessions). **Recommendation:** in-memory inverse
commands for M1, designed so a persisted log can back it later. `change_log` already
captures the data-level history.

### D10 — PDF generation

Webview print-to-PDF (free, zero dependencies, uses the print stylesheet we need anyway,
but gives limited control over pagination and page headers) vs. a Rust renderer
(`printpdf`/`typst` — real control, real work). **Recommendation:** webview printing for
M1, a renderer later behind `evara-report`'s renderer trait. Flagged because
multi-page timetable printing with repeated headers is a common hard requirement in this
product category and webview printing may not survive contact with it.

### D11 — macOS distribution

Notarisation requires an Apple Developer account (~US$99/year) and CI secrets. Without
it, macOS users must bypass Gatekeeper manually, which for a school-admin audience is
effectively a blocker. Windows code signing has a similar cost. Not a dependency problem
— a distribution-cost decision that needs an owner before Phase 11.

### D12 — Student clash semantics when membership is not modelled

Many schools will never enter individual students. Clash detection then relies on the
`student_group` hierarchy (`parent_group_id`) to decide which groups overlap. This is a
genuinely subtle area — combined classes, split sets and composite groups are where
competitor products differentiate — and getting it wrong produces either phantom clashes
or missed ones. Needs its own design note and a dedicated fixture before Phase 5.

### D13 — Minimum supported Rust version ✅ RESOLVED 2026-10-06 (Phase 0)

Set to **1.90**, matching Tauri 2.12. Started at 1.85 (the edition-2024 floor), which the
MSRV-aware resolver then used to hold the whole tree back — including three security
advisories it refused to resolve past. Nobody compiles Evara with an old toolchain
because we ship the binary, so a conservative MSRV costs patches and buys nothing.

### D14 — cargo-deny policy for unmaintained crates ✅ RESOLVED 2026-10-06 (Phase 0)

Vulnerabilities always fail the build. "Unmaintained" is scoped to workspace
dependencies (`unmaintained = "workspace"`), because `proc-macro-error` is reachable only
through Tauri's Linux GTK backend — not compiled on Windows or macOS, build-time only,
and with no upgrade available. Failing CI on something unfixable trains people to ignore
the check.

---

### D15 — Project file extension ✅ RESOLVED 2026-10-06

The portable export is `*.evara` ([ADR 0004](adr/0004-evara-project-format.md)). The
*live* project database needs its own extension so the two are never confused — opening a
raw SQLite file expecting an archive, or mailing a WAL-mode database to a colleague, are
both bad outcomes.

**Decided: `.evaraproj` for the live editable project; `.evara` reserved for the portable
exchange package.** The distinction is intentional and enforced in code: a live project is
a working SQLite database with WAL sidecar files, an export is a self-contained archive.
`Workspace` rejects any other extension, including `.evara`, with
`DbError::WrongExtension`. The Phase 1A placeholder `.evdb` is gone from the codebase.

### D16 — Where `site_id` really belongs ✅ RESOLVED 2026-10-06

`site_id` identifies the *installation*, but Phase 1A stores it inside the project file,
because no application-level settings database exists yet. The consequence is visible:
open a project created on another machine and it keeps that machine's `site_id`, so the
change feed will attribute your edits to them.

**Decided and implemented in Phase 1B.** `site_id` identifies an installation and now
lives in the application settings database. A project stores `project_id` and
`created_by_site_id` (historical, never updated). The installation doing the writing is
supplied per connection through the application-defined `evara_site_id()` SQL function,
which the change-log triggers call — see
[adr/0008](adr/0008-installation-identity-and-project-lifecycle.md). Migration V2 performs
the correction; V1 was left untouched.

### D17 — Who maintains `updated_at` and `rev` ✅ RESOLVED 2026-10-06

[DATA-MODEL.md](DATA-MODEL.md) originally said `updated_at` is "maintained by trigger".
Phase 1A deliberately did **not** do that: a trigger that updates its own table interacts
with SQLite's `recursive_triggers` setting in ways that are easy to get subtly wrong, and
the failure mode — a change logged twice, or not at all — is exactly what the change feed
exists to prevent.

**Decided: repositories maintain `rev` and `updated_at` explicitly.** No self-updating
SQLite triggers for those fields. The change-log triggers only *observe* the values the
repository wrote. This is predictable and testable, and it avoids the `recursive_triggers`
interaction whose failure mode — a change logged twice, or not at all — is exactly what
the change feed exists to prevent.

Enforcement lands with the first repository in Phase 1C: a shared update helper, plus a
test asserting no entity table is written without bumping `rev`.

### D18 — Where the project lifecycle lives ✅ RESOLVED 2026-10-06 (Phase 1B)

`Workspace` is in `evara-db` rather than in a new orchestration crate or in the Tauri
command layer. It coordinates two SQLite databases, and SQL belongs in exactly one crate;
putting it in `src-tauri` would have made the lifecycle untestable without a window and
would have broken the "no business logic in command handlers" rule.

Revisit if a second non-SQL concern attaches itself to the lifecycle, at which point an
`evara-app` crate becomes the right home.

### D19 — Structure deletion semantics ✅ RESOLVED 2026-10-06 (Phase 1C)

DATA-MODEL.md §3 documented the Structure parent links but never stated `ON DELETE`
behaviour, so it was decided explicitly rather than defaulted:

- **Ownership edges are RESTRICT** — campus→school, building→campus, room→campus,
  resource→school, resource→campus, academic_year→school, term→academic_year. Deleting a
  parent that still has children fails and says what is in the way.
- **Optional classification references are SET NULL** — `room.building_id` and
  `room.room_type_id`. The room is the asset; where it sits and what kind it is are
  metadata.

`resource.campus_id` is RESTRICT despite being optional, so that "delete a campus" means
the same thing on every edge. See
[adr/0010](adr/0010-structure-deletion-semantics.md).

### D20 — Ambiguities found in DATA-MODEL.md §3 ✅ RESOLVED 2026-10-06 (Phase 1C)

Resolved while implementing, and now reflected in the document:

| Gap | Resolution |
| --- | --- |
| No uniqueness scopes except `campus(school_id, code)` | Applied the documented rule — codes are unique within their parent: `room_type(school_id, code)`, `room(campus_id, code)`, `resource(school_id, code)`, `academic_year(school_id, name)`, `term(academic_year_id, ordinal)` and `(academic_year_id, name)` |
| `building` has no `code` | Its `name` is unique within the campus instead |
| Is `school` a singleton? | Yes — ADR 0003 says multiple schools means multiple project files. Enforced with a constant column and a unique index |
| `room` names both a campus and an optional building, which could disagree | Enforced by a trigger. A composite foreign key cannot be used because `ON DELETE SET NULL` would try to null the NOT NULL `campus_id` |
| `deleted_at` is "only on tables a future sync must tombstone" — which ones? | None in Phase 1C. Deletes are hard, guarded by RESTRICT, and remain attributable through `change_log`. Revisit when synchronisation is designed |

### D21 — Timeslot identity, and ambiguities in DATA-MODEL.md's time model ✅ RESOLVED 2026-10-09 (Phase 1D)

The identity rule had to be settled before `V4` was written, because a released migration
is immutable and reassigning slot identifiers is silent corruption (R6). Decided:
**logical identity is `(term_id, cycle_day_id, period_id)`, and `timeslot.id` is assigned
once and never reassigned while that tuple exists.** See
[adr/0011](adr/0011-stable-timeslot-identity.md).

Four further ambiguities were put to the maintainer rather than defaulted:

| Gap | Resolution |
| --- | --- |
| `timeslot.term_id` is nullable, so the documented `UNIQUE(cycle_day_id, period_id, term_id)` does not constrain the term-less rows at all — SQLite treats `NULL`s as distinct | `NULL` keeps its meaning, "the whole year", and identity is enforced by a unique index over `COALESCE(term_id, '')`. The plain three-column constraint is not also created: redundant where the term is set, useless where it is not |
| `ON DELETE` for `calendar_day.cycle_day_id` | `RESTRICT`. `SET NULL` was available, since `NULL` means "no lessons that day", but it would silently reinterpret mapped teaching dates as non-teaching ones |
| Whether `is_default` is constrained | At most one, enforced in the schema: one default cycle per school, one default period structure per campus — with the school-wide `NULL` campus as its own bucket, via the same `COALESCE` technique |
| What `cycle.day_count` means next to the actual `cycle_day` rows | The *declared* length. The materialiser uses the rows that exist and the repository refuses an ordinal beyond `day_count`, but does not demand the set be complete or contiguous — so building a cycle one day at a time stays legal. No cross-table trigger |

Two things were decided while implementing, by applying rules this document already
states rather than by inventing policy, and are recorded here for visibility:

- `cycle_day(cycle_id, label)` and `period(period_structure_id, label)` are unique,
  applying D20's rule that a name is unique within its parent (as `building` already
  does). Two "Day A"s in one cycle is a mistake, not a configuration.
- `calendar_day(school_id, date, COALESCE(campus_id, ''))` gets the same `COALESCE`
  treatment as `timeslot`, for the same reason: the campus-less rows are the common case.

## Technical risks

### R1 — Incremental scoring correctness (highest)

A wrong delta score silently produces a wrong timetable, and it is the hardest part of
any solver to debug. **Mitigation:** the two-scorer design plus the property test in
[SOLVER.md](SOLVER.md) §4, in CI, as a gate on every new constraint. Non-negotiable.

### R2 — Solver quality is open-ended work

"Generate a timetable" is binary; "generate a timetable a timetabler accepts" is years of
refinement, and incumbents have had decades. **Mitigation:** benchmark fixtures with
committed score baselines from Phase 6, so progress and regressions are measurable rather
than argued about. Set expectations that M1 produces a *feasible* timetable, not a
competitive one.

### R3 — Constraint count is the real scope

~22 constraint types in M1, 60+ eventually, each needing an evaluator, an incremental
path, a param schema, an explanation, a test and a fixture. This dominates the effort —
more than the solver core. **Mitigation:** the registry pattern, and a strict
definition-of-done per constraint so coverage claims stay honest.

### R4 — Rust/TypeScript boundary churn

Every schema change touches a migration, a repository, a DTO, generated types, an IPC
wrapper and a form. **Mitigation:** generated types, generic CRUD conventions established
in Phase 2, and batching schema changes per phase rather than per field.

### R5 — Performance at scale, unknown until measured

A 2,000-activity secondary school with electives is the realistic upper end. Nothing in
the design should break, but load time, solve time and grid rendering are all unmeasured.
**Mitigation:** generate a large synthetic fixture in Phase 3 — *before* the solver — and
treat its numbers as a budget.

### R6 — Timeslot rematerialisation — *mitigation shipped in Phase 1D*

Changing the cycle or period structure after a timetable exists invalidates placements.
Silent data loss is the worst outcome. **Mitigation:** the materialiser preserves surviving
slot IDs, and any destructive change requires explicit confirmation showing exactly how
many entries would be orphaned.

Implemented in Phase 1D ([adr/0011](adr/0011-stable-timeslot-identity.md)): identity is
the `(term_id, cycle_day_id, period_id)` tuple, `apply_materialisation` never deletes,
orphan candidates come back in the returned plan with their identifiers, and removing them
is a separate call. A mutation check confirms the invariant has teeth — breaking
preservation in the materialiser fails eight tests.

**What is still open:** the confirmation *interface* does not exist yet, because there is
no UI for the time model and no `timetable_entry` to count. The repository exposes
everything that interface will need; nobody is obliged to call it until Phase 7.

### R7 — Licence hygiene

Apache-2.0 obliges us to keep shipped dependencies permissive. Tauri (MIT/Apache-2.0),
rusqlite (MIT), React (MIT) are fine. The risk is an incidental GPL/AGPL transitive
dependency — plausible in PDF and font libraries specifically. **Mitigation:**
`cargo-deny` licence allow-list in CI from Phase 0, and a generated third-party notice
file in Phase 11.

### R10 — A database-level SET NULL bypasses the revision contract

`ON DELETE SET NULL` modifies a row without incrementing `rev`, so the change-log entry
would record a stale revision. Repositories therefore clear `room.building_id` and
`room.room_type_id` with proper rev-bumping updates *before* deleting the parent, making
the schema rule a backstop that never fires in normal operation.

It would still fire for a write that bypassed the repository — which cannot happen today,
since a connection without `evara_site_id()` cannot write to a tracked table at all. Worth
revisiting if any future edge genuinely needs SET NULL on a hot path.

### R8 — Single-file SQLite on synced folders — *detection and warning shipped*

Users will put project files in OneDrive, iCloud Drive or Dropbox. WAL-mode SQLite on a
cloud-synced folder is a known corruption source. **Mitigation:** detect common synced
paths and warn on open; make `.evara` export the blessed way to move a project; document
it. Worth deciding in Phase 1, not Phase 11.

### R9 — Fixture realism without real data

Synthetic fixtures are mandatory, and naive ones are too easy — they hide the pathologies
(uneven subject loads, specialist-room bottlenecks, part-time staff) that break real
timetables. **Mitigation:** a parameterised generator in `evara-cli` with knobs for
tightness, specialist-room scarcity and part-time fraction, plus hand-built adversarial
cases.
