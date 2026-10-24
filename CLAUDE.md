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
crates/evara-report     projections and renderers   -> core, constraints
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
- **The scheduling vocabulary is `evara-core`'s, and its unknowns are variants.**
  `scheduling::Problem` and its facts, `Placement`, `Assignment`, the diagnostics and
  `SolveOutcome` live there because `evara-db`, `evara-constraints`, `evara-solver` and the
  command layer all have to agree on them. Three of its types carry a decision in their
  shape and must keep it: `ExpectedAttendance::Unknown` holds **no number**, so there is no
  `0` to leak into a comparison; `Overlap::Indeterminate` is reachable only by naming it, so
  no caller can write `!= Disjoint` and promote the unproven case into a clash; and
  `DisjointnessDeclaration` keeps `NotStated` and `MayOverlap` apart, because they give the
  same overlap answer and are different states. A duration is a **count of periods** and the
  wall-clock interval is derived from the periods covered — never the other way round.
  `RoomFact::is_candidate_for` is the **one** implementation of the room candidate domain,
  and the loader, the move validator and the fixtures all call it rather than restating the
  predicate. [SCHEDULING-FIXTURES.md](docs/SCHEDULING-FIXTURES.md).
- **The loader compiles one school and reuses every shared rule.** `evara_db::scheduling` is
  the one place UUIDs become dense indices, and it **calls** `resolve::MaterialisedGrid` for
  the bell schedule and `RoomFact::is_candidate_for` for the room domain rather than restating
  either — a second copy is the failure §2's most important rule exists to prevent. It compiles
  **no** overlap answers (`OverlapRelation::unknown`): ADR 0035 §3's seven rules are M5's, and
  `clash_mask` admitting only `Overlaps` is what makes that safe rather than permissive. A
  finding about a *stored row* is the loader's, not the validator's — a compiled `Problem` has
  already dropped the row ([adr/0037](docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md)).
  [SCHEDULING-PROBLEM-ASSEMBLY.md](docs/SCHEDULING-PROBLEM-ASSEMBLY.md).
- **Opening a project writes; inspecting one must not.** `Database::open_existing` runs
  migrations, recreates triggers and stamps `last_opened_with_app_version` on *every* open.
  Anything headless that reads somebody's live file goes through
  `Database::open_for_inspection`, which does none of the three and hands back a `query_only`
  connection with no `evara_site_id()` registered. Never reach for the mutating path to avoid
  a `SchemaUpgradeRequired`; that refusal is the user's decision to make.
- **A rule whose absence would make a timetable wrong is mandatory in the registry, not a
  row.** Evaluation runs every implemented `ConstraintType` whatever the database holds, and
  **no constraint rows are seeded**: a `constraint_instance` row *tunes* a rule — weight,
  term, parameters, and a level it may **raise** but never weaken — so no deletion can
  disable a safety check. None of the eight can be narrowed, and a `constraint_ref` for one
  is refused rather than ignored. The repository refuses what the registry would refuse, so a
  stored row is always compilable; the one thing it cannot judge is the **shape** of
  `params`, because the JSON Schemas live beside the evaluators and `evara-db` may not depend
  on `evara-constraints` — `Registry::compile` is the other half.
  [adr/0039](docs/adr/0039-a-mandatory-constraint-lives-in-the-registry.md),
  [CONSTRAINT-ENGINE.md](docs/CONSTRAINT-ENGINE.md).
- **A clash is overlapping minutes, and an occupancy is resolved once.** Every clash rule
  compares `[start, end)` intervals on a cycle day, never a `timeslot_id` — two campuses on
  two bell schedules hold different slots over the same minutes, so the obvious
  implementation is wrong exactly when sharing matters. `Occupancy` resolves what each
  occurrence occupies once and every evaluator reads it, so no two rules can disagree about a
  span. An evaluator never restates a predicate that exists: `RoomFact::is_candidate_for`,
  `runs_on`, `Overlap::clashes` and `SlotAvailability::permits_placement` are each the one
  implementation of their question.
- **Solving is deterministic, and every execution limit is a count.** One seeded RNG —
  `evara_core::scheduling::rng` — shared by the fixture generator and the solver, reading no clock
  and depending on no RNG crate, so neither a fixture's bytes nor a timetable can change under a
  dependency bump. No `Instant::now()` or unseeded randomness inside the solver's decisions, and
  **no time limit**: candidate evaluations, construction steps and retry passes are counts, so the
  same seed stops in the same place on every machine. A wall clock reaches the engine only as the
  caller's `Cancel`, which is why `Termination::Cancelled` is the one result this workspace
  documents as not reproducible. Determinism is asserted as whole-outcome equality, which is why
  `SolveStatistics` carries counters and no elapsed time.
- **The solver searches; the registry decides what is legal.** `evara-solver` chooses what to try,
  in what order and when to stop. Every candidate goes through `IncrementalScorer::probe` and the
  finished arrangement is re-checked by the reference `evaluate` — a disagreement between the two
  is a **defect in Evara**, reported as one, never a timetable returned as a success. Candidate
  generation may pre-filter a rule only by *calling* the function that owns it (`runs_on`,
  `RoomFact::is_candidate_for`, `SlotAvailability::permits_placement`); a second copy of a
  predicate is §2's most important rule broken where a solver usually breaks it. The incremental
  scorer therefore lives with the evaluators, not with the search, and has no hand-written
  per-rule delta — [adr/0041](docs/adr/0041-the-incremental-scorer-lives-with-the-evaluators.md),
  [TIMETABLE-CONSTRUCTION-SOLVER.md](docs/TIMETABLE-CONSTRUCTION-SOLVER.md).
- **Never report that a demand cannot be scheduled.** The mirror of `evara_core::lessons`' rule,
  and it binds the solver: a greedy construction that leaves a lesson unplaced has discovered that
  *its* arrangement had no room, not that no timetable exists. `UnplacedCause::is_impossible` is
  the one place the difference is decided, and only a property of the **problem** — no run long
  enough, no candidate room, no resolved bell schedule, availability emptying the range — may be
  phrased as impossibility. A generated timetable is described as having **no confirmed clash
  within the school solved**, with the unverified counts beside it; never as conflict-free.
- **A stored timetable is a version, and a save refuses inputs that moved.** Saving always
  **inserts**: several timetables coexist for one `(cycle, term)` and that is the versioning, so a
  regeneration never overwrites and replacement is a save followed by an explicit delete. One
  `timetable_entry` row is **one occurrence**, carrying the *start* slot — rows 2…`d` are derived
  by `evara_core::scheduling::span`, and storing them would make a corrupt timetable and a partial
  multi-period move representable. A save is refused when any scheduling input moved since the
  problem was compiled, compared **inside the writing transaction** against the change-log
  watermark: nothing referential breaks when an availability rule changes, so nothing else would
  catch it, and the result would be a plausible wrong timetable. Never compare a watermark in the
  interface — ADR 0030's rule holds here too. A `GENERATION` scoring is **immutable**; a
  revalidation replaces only the current one, so re-scoring can never misrepresent how a timetable
  was generated. `evara generate` writes nothing and `evara save` is the command that writes.
  [adr/0042](docs/adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md),
  [TIMETABLE-PERSISTENCE.md](docs/TIMETABLE-PERSISTENCE.md).
- **Scores are integers, and penalties are negative-going.** Three levels, compared
  lexicographically. A weight is a non-negative magnitude; an evaluator negates it **once**, when
  it builds a `Violation`, so `Score::PERFECT` is the top of the order and `Ord` is the comparison
  a search can use with no wrapper (`SOLVER.md` §3). Weights become integer penalties at compile
  time, not at scoring time.
- **A grid cell holds a list, and `rowSpan` is decided in Rust.** Two occurrences can occupy one
  cell — ordinary in a master grid, a clash in a teacher's — and a run can be non-contiguous when
  a slot inside it was never materialised. Either produces a broken table and a lost lesson if a
  renderer works out its own spans, so `evara-report` emits one `TimetableCell` per
  `(day, period)` and a lesson earns a span **only** when it is alone in every cell it occupies
  and contiguous in that axis. Everything else is `Pinned` at `rowSpan` 1, carrying **every**
  lesson involved: nothing is discarded to make a table render. `GridPanel::is_well_formed`
  asserts it. **One axis per bell schedule, never merged** — two campuses' Period 3 is not one
  row. The view **groups** already-resolved lessons and resolves nothing: no span, no clash, no
  end time, no weekday, and never a judgement about whether a cause is impossible.
  [TIMETABLE-VIEWS.md](docs/TIMETABLE-VIEWS.md).
- **A report renders the rule's own sentence, and a label is never a scheduling input.**
  `explain()` belongs to the evaluator that produced the violation, so `evara-report` **calls**
  it rather than composing wording that would drift from the check
  ([adr/0043](docs/adr/0043-a-report-renders-a-rules-own-explanation.md)) — and a stored
  `violation` row deliberately carries no sentence, so the stored tier renders the fact and not
  prose. The names a *grid* needs and a solve does not — subject, course, year level, building,
  student — live in `evara_core::scheduling::display`, are populated by `evara-db` on request and
  are **never** added to `Problem`: ADR 0037 §2's refusal to widen it for a diagnostic-only field
  binds a report-only one identically. A per-student view is derived from `class_enrolment` and
  from **nothing else** — never a group's membership, never `student_group.size` — because
  substituting one presents a guess as a child's confirmed schedule.
  [adr/0044](docs/adr/0044-the-display-vocabulary-sits-beside-the-problem.md).
- **A solve never holds the workspace lock, and a save is claimed once.** The command thread
  compiles the `Assembly` under the lock, releases it, and **moves** it to a worker —
  `ARCHITECTURE.md` §3.3 is what permits that, and `evara_db::scheduling`'s `send_assertions`
  test is what keeps it true. Cancellation is a **request**: it sets an `AtomicBool` the engine
  reads between steps and changes no state, so the worker is the only thing that leaves
  `Running` and a cancel racing a finish has one deterministic outcome. A save is claimed by a
  compare-and-swap into `Saving` *before* any database work, because two clicks otherwise both
  pass a "has it finished?" test and write twice; `Saved` is terminal. A session records the
  project epoch it began under, so a timetable generated against one campus file can never be
  saved into another.
- **A lock is a property of a placement, and an edit never writes a rule.**
  `timetable_entry.is_locked` — per version, **never `activity.is_locked`**: several timetables
  coexist for one `(cycle, term)`, so a lock on the lesson would pin it in all of them at once
  ([adr/0045](docs/adr/0045-a-lock-is-a-property-of-a-placement.md)). The **loader compiles no
  pins**; `Problem::pinned` is filled by the caller that opened a timetable, because a lock is an
  *output* of a solve. `activity-fixed` is the ninth type and compares the whole `Placement`, slot
  and room — and an **unplaced** pinned occurrence is *not* a violation of it, or the empty
  arrangement of a pinned problem would start infeasible and break construction. The editor adds
  no move validator: `IncrementalScorer::probe` was already one, and it withdraws the occurrence
  before placing it at the candidate, so a one-period move is not reported as clashing with
  itself. An edit **changes the version it is open on** and `timetable_duplicate` is the
  affordance that makes that safe ([adr/0046](docs/adr/0046-an-edit-changes-the-version-it-is-open-on.md));
  an undo is a new transaction, so it can be refused, and a refusal clears both stacks. Every
  editing write carries the **exact `(entry id, rev)` list** of the arrangement beside its
  revision, compared inside the writing transaction — a digest collides on
  unschedule-then-place and a scoped change-log watermark cannot see a deletion
  ([adr/0047](docs/adr/0047-a-cross-session-guard-is-an-exact-placement-list.md)).
  [TIMETABLE-EDITING.md](docs/TIMETABLE-EDITING.md).

## 4. Definition of done for a constraint

A constraint type is not "supported" until all six exist:

1. a stable `type_key` and a registry entry in `crates/evara-constraints/`
2. a JSON Schema for its parameters, next to the evaluator
3. documented semantics in [docs/CONSTRAINTS.md](docs/CONSTRAINTS.md)
4. an isolated unit test, plus a synthetic fixture exercising it
5. an `explain()` implementation producing a human-readable reason
6. an `evaluate_involving()` — the rule's own narrowed query, **sharing one body** with its
   full sweep rather than restating the check. Since M6 this is a trait member rather than a
   convention, so a type without one cannot be registered; it is what lets the incremental
   scorer ask about a single occurrence, and the union property test is what proves it complete.

The reference-vs-incremental scorer property test must stay green. It is the gate that
keeps incremental scoring honest, and a wrong delta silently produces a wrong timetable.
`crates/evara-constraints/src/differential.rs` is that gate, and one of its assertions is a
**precondition of the whole design**: every violation names at most two occurrences. A rule whose
violation named three would break the incremental identity silently, so the test is what turns
that into a red build rather than a wrong timetable.

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
| `cargo run -p evara-cli -- fixture --list` | List the synthetic scheduling fixtures |
| `cargo run -p evara-cli -- fixture tiny` | Write one as JSON — synthetic test data, **not** a timetable |
| `cargo run -p evara-cli -- validate tiny` | Pre-solve validation of a fixture's inputs; exits 1 on a blocking finding |
| `cargo run -p evara-cli -- evaluate tiny` | Score an arrangement against the constraint rules; exits 1 on a hard violation |
| `cargo run -p evara-cli -- generate tiny --seed 42` | **Construct** a timetable over a fixture; exits 1 if partial, 3 if refused |
| `cargo run -p evara-cli -- generate --project <path> --school <id> --cycle <id>` | Construct a timetable for a **real project**, read-only — migrates nothing, writes nothing, persists no timetable |
| `cargo run -p evara-cli -- save --project <path> --school <id> --cycle <id>` | Generate **and save** a timetable. The one command that writes one; never overwrites an existing timetable |
| `cargo run -p evara-cli -- save --project <path> --revalidate <id>` | Re-score a stored timetable and replace its current findings, leaving the generation record alone |
| `cargo run -p evara-cli -- evaluate --project <path> --timetable <id>` | Score a **stored** timetable against the inputs as they now stand; writes nothing |
| `cargo run -p evara-cli -- validate --project <path> --list` | What schools, cycles and terms a `.evaraproj` holds, with their identifiers |
| `cargo run -p evara-cli -- validate --project <path> --school <id> --cycle <id>` | Pre-solve validation of a **real project**, read-only — migrates nothing, writes nothing |
