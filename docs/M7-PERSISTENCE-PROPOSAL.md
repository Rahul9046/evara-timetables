# M7 — Timetable Persistence and Versioning · Architecture Proposal

- Status: **Proposed — awaiting approval. Nothing is implemented.**
- Date: 2026-10-23
- Branch: `feat/m7-timetable-persistence`
- Base: `c021e79` (`origin/develop`, "Merge pull request #30 … feat/m6-timetable-construction-solver")
- Schema version at base: **`V16`** · proposed: **`V17`**

This answers the seven questions put to the review, proposes a schema, repository APIs, a
migration strategy, edge cases and a test plan, and ends with the decisions that need a yes
before any code is written.

**Three conflicts with accepted records are identified and none is resolved unilaterally** —
§0 lists them, and each is a numbered decision in §10.

---

## 0. Conflicts found, up front

| # | The conflict | Where | §10 decision |
| --- | --- | --- | --- |
| **C1** | `DATA-MODEL.md` §7 and ADR 0029 §2 both state `timetable_entry`'s key exists *"because a multi-period activity occupies several slots"* — i.e. **one row per occupied slot**. That representation stores derived data, and the only implementation of span resolution lives in a crate `evara-db` is forbidden to depend on. | `DATA-MODEL.md` §7, ADR 0029 §2, `CLAUDE.md` §3 | **D1** |
| **C2** | `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 lists *"the published/archived lifecycle"* among what is **not in the MVP**, while `DATA-MODEL.md` §7 gives `timetable` a `status` column with `DRAFT`/`PUBLISHED`/`ARCHIVED`, and this brief asks for versioning. | §6 vs §7 of two records, vs the brief | **D3** |
| **C3** | My own `TIMETABLE-CONSTRUCTION-SOLVER.md` §17 wrote that the per-occurrence-vs-per-slot choice was *"a decision this milestone deliberately does not make"*. That was **wrong**: two records had already asserted an answer. The sentence needs correcting whichever way D1 goes. | `TIMETABLE-CONSTRUCTION-SOLVER.md` §17 item 1 | **D1** |

Two smaller departures from the `DATA-MODEL.md` §7 sketch are proposed on the strength of
conventions established *after* it was written, exactly as `V16` did for `constraint_instance`
(its header records three such deviations): **no `school_id` on `timetable`** (§5.1) and
**`ON DELETE RESTRICT` rather than `CASCADE`** (§5.4). Both are **D6** and **D7**.

---

## 1. Q1 — One row per occurrence, or one row per occupied timeslot?

### What the records say

> `UNIQUE(timetable_id, activity_id, timeslot_id)` rather than `(timetable_id, activity_id)`
> because a multi-period activity occupies several slots; the entry rows for one activity are
> written and moved as a unit. — `DATA-MODEL.md` §7

> **`timetable_entry` is already unique on `(timetable_id, activity_id, timeslot_id)`**
> (`DATA-MODEL.md` §7) *because a multi-period activity occupies several slots.* — ADR 0029 §2

So the records assert **one row per occupied slot**. Note what kind of assertion each is:
`DATA-MODEL.md` §7 is an explicit *sketch of tables no migration defines*, and ADR 0029 §2 cites
the key as **supporting evidence** for its own decision (that an `activity` row is one
occurrence) rather than deciding `timetable_entry`'s shape. Neither is an ADR *about*
`timetable_entry`.

### The decisive new fact: the layering forbids the obvious implementation

A row per occupied slot requires the writer to resolve which `d` slots an occurrence covers.
There is **exactly one** implementation of that — `Occupancy::resolve` — and it is `pub(crate)`
in `evara-constraints`. `ARCHITECTURE.md` §3 rule 4 and `evara-db/Cargo.toml` are unambiguous:
**`evara-db` depends on `evara-core` only.** So the writer cannot call it.

That leaves three ways to store per-slot, and two of them are forbidden or ugly:

| Route | Verdict |
| --- | --- |
| Re-derive the span inside `evara-db` | **Forbidden.** A second implementation of a scheduling measure is `CLAUDE.md` §2's most important rule broken. |
| Have the caller (`src-tauri` / `evara-cli`, which may depend on both) flatten the spans and hand `evara-db` a list of triples | Possible, but it moves a scheduling decision into the orchestration layer, which `ARCHITECTURE.md` §3 rule 7 reserves for orchestration. |
| **Promote span resolution into `evara-core`** beside `runs_on` and `PlacementSpan`, and have `evara-constraints` *and* the writer both call it | Clean, and the established pattern (ADR 0036 for the bell-schedule resolver, M6 for `Rng`, ADR 0041 for the scorer). |

**The third route is worth doing regardless of D1**, because Phase 7's reporting needs the same
answer: "what occupies Monday P2?" is a span question, and `evara-report` also depends on
`evara-core` only. §9 proposes it as **D2**.

### The trade-off, once span resolution is reachable from both

| | One row per **occurrence** | One row per occupied **slot** |
| --- | --- | --- |
| Rows for 2 000 occurrences (≈15% multi-period) | ≈2 000 | ≈2 350 |
| Writer | 1:1 projection of `Assignment`. No derivation | derives the span, writes `d` rows |
| Reader: "where does this lesson start?" | the row | min period ordinal of its rows |
| Reader: "what occupies this slot?" | derive the span (one `evara-core` call) | the row |
| `Assignment` round-trip | **exact** | reconstruct start, discard the rest, *and* check they agree with `duration_periods` |
| Can a stored timetable be internally inconsistent? | **No.** There is nothing to disagree | **Yes.** 3 rows for a 2-period activity, and something must decide which is true |
| Phase 8 "moved as a unit" | one row moves | `d` rows move together; a partial move is representable and wrong |
| `coverage` (§8, post-MVP) pointing at one period of a double | needs a slot, so needs the span | points at the row |

### Recommendation

**One row per occurrence**, keeping the key `UNIQUE(timetable_id, activity_id, timeslot_id)`
verbatim — it still holds, and leaves the per-slot option open later without a key change.

The argument that decides it is the repository's own, stated in many forms: *a second place to
record a fact is a second place for it to be wrong.* Rows 2…`d` are **derivable** from the start
slot and `activity.duration_periods`, and `CLAUDE.md` already fixes the direction of that
derivation — *"a duration is a count of periods and the wall-clock interval is derived from the
periods covered — never the other way round."* Storing the covered slots makes a corrupt
timetable representable, and makes a partial multi-period move representable in Phase 8, which
is precisely the state the data model says is impossible ("written and moved as a unit").

**This contradicts a stated reason in two records, so it is D1 and needs approval.** If the
answer is no, §5.2 gives the per-slot DDL as the alternative; it is a small change to the same
table plus a `span_ordinal` column to make "which slot of the run is this?" explicit rather
than inferred from the period's ordinal.

---

## 2. Q2 — How timetable versions work

### The scope conflict first

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6: *"Not in the MVP, unchanged from the audit: … export,
and the published/archived lifecycle."* A `status` column with `DRAFT`/`PUBLISHED`/`ARCHIVED` is
that lifecycle. Building it in M7 contradicts an accepted record; the brief asks for versioning,
which is not the same thing.

### Proposal: versioning by rows, with no lifecycle

**A `timetable` row is one timetable for one `(cycle, term)`. Several may coexist, and that is
the versioning.** No version number, no status, no "current" flag.

| Operation | Behaviour |
| --- | --- |
| **Generate** | Always **inserts a new** `timetable`. Never overwrites, never asks. |
| **Regenerate** | The same thing. The previous timetable is untouched and still openable. |
| **Replace** | Not an operation. It is *generate* followed by an explicit *delete*, which is two decisions by the user rather than one silent one. |
| **Delete** | Explicit, carries the timetable's `expectedRev`, removes its entries, violations, unscheduled rows and solve runs in one transaction (§5.4). |
| **Preserve** | The default. Nothing in M7 deletes a timetable except a user asking. |

Why preservation is the default and not a setting: ADR 0010 makes deletion *"follow ownership,
and default to refusing"*, and M6's whole posture is that partial and superseded work is still
worth having. A regeneration that silently replaced the previous timetable would destroy the one
thing a timetabler compares against.

**`name` is how a human tells two apart**, defaulted by the caller (not the database) to
something like *"Generated 2026-10-23 14:02"*. `UNIQUE (cycle_id, COALESCE(term_id, ''), name)`
so two in the same scope cannot share a name — note the `COALESCE`, which `CLAUDE.md` requires
of any nullable column in a uniqueness rule.

**Deferred, named, and not stubbed:**

- `status` / the published-archived lifecycle — with the lifecycle (§6 of the M1 record).
- `based_on_timetable_id` — with **repair** (`SOLVER.md` §7), which is what needs lineage.
- A "current timetable" pointer — with the Generate screen, which is the thing that would need
  to remember a selection.

Each is one additive column in a later migration. Adding a column nothing writes is the
`Problem::pinned` shape, and for a *schema* there is no reason to pay for it early.

---

## 3. Q3 — `solve_run`, seeds, statistics, termination, unscheduled causes

One `solve_run` row per generation, holding everything needed to **reproduce and compare** the
run. A hand-built timetable (Phase 8) has no `solve_run`, which is an ordinary state.

| Stored | From | Note |
| --- | --- | --- |
| `seed` | `SolveOutcome::seed` | the whole point: *"a run whose seed was not recorded cannot be reproduced"* |
| `status` | `SolveStatus` | the five accepted variants, as the `CHECK` strings |
| `termination` | `Termination` | the eight kebab-case keys from `Termination::key()` |
| `engine` | the caller | `"construction"` — `SolverEngine` is a seam, so the row says which engine |
| `settings` | `SolveLimits` | JSON, `CHECK (json_valid(...))`, the `constraint_instance.params` precedent |
| six counters | `SolveStatistics` | `candidate_evaluations`, `candidates_rejected`, `placements_committed`, `placements_withdrawn`, `construction_steps`, `passes` |
| `score_hard/medium/soft` | `SolveOutcome::score` | **INTEGER**, negative-going (M6 corrected the sign) |
| `started_at` / `finished_at` | **the caller's clock** | the engine reads none; `SolveStatistics` carries no duration by design |
| `app_version` | `app_meta` | `solver_version` in the sketch; the app version is the honest value, and `meta.rs` already owns the key |

Elapsed time is `finished_at − started_at` and is **not** a stored duration: that keeps the one
place a clock is read at the boundary, where M6 put it.

### Unscheduled causes

`solve_run_unscheduled (id, solve_run_id, activity_id, cause, diagnostic_code NULL)`.

- `cause` is `UnplacedCause::key()` — a stable kebab-case string, `CHECK`ed against the seven.
- `diagnostic_code` is the pre-solve `DiagnosticCode::key()` where M6 attached one.
- **The sentence is not stored.** `UnscheduledActivity::reason` is engine-authored prose; storing
  it would let a wording improvement leave old rows reading the old way. The cause survives a
  wording change and a string does not — which is `TIMETABLE-CONSTRUCTION-SOLVER.md` §17 item 5's
  own recommendation, now acted on.
- The section name a sentence needs is **joined** through `activity → class_section`, not copied.

`is_impossible()` is **not** stored either: it is a pure function of `cause`, and a stored copy is
a second place for it to disagree.

---

## 4. Q4 — Violations and unverified counts across a restart

### Violations

```
violation       (id, timetable_id, solve_run_id NULL, constraint_id NULL,
                 constraint_type, level, penalty INTEGER, cycle_day_id NULL,
                 starts_at NULL, ends_at NULL, detail)
violation_ref   (id, violation_id, role, entity_kind, entity_id)
```

`Violation`'s shape maps with nothing invented:

| `Violation` field | Stored as |
| --- | --- |
| `constraint_type` | `constraint_type` TEXT — the registry's `type_key` |
| `level` | `level` TEXT `CHECK (level IN ('HARD','MEDIUM','SOFT'))` |
| `constraint_id` | `constraint_id` → `constraint_instance`, nullable (a mandatory built-in has none) |
| `penalty` | `penalty` **INTEGER**. The sketch's `REAL` is corrected, as ADR 0040 §3 corrected `weight` |
| `activities[]` | `violation_ref` rows with `role = 'activity'` |
| `subject` | one `violation_ref` with `role = 'subject'` |
| `related[]` | `violation_ref` rows with `role = 'related'` |
| `cycle_day` | `cycle_day_id` → `cycle_day`, nullable |
| `span` | `starts_at` / `ends_at` TEXT `'HH:MM'`, nullable — the `period` convention, so text order is chronological |
| `detail` | `detail` TEXT |

`violation_ref.entity_id` is **polymorphic, so no foreign key** — the `constraint_ref` precedent,
stated in `DATA-MODEL.md` §7 and accepted there.

### Unverified counts

Four `INTEGER NOT NULL` columns on `timetable`, named exactly as `UnverifiedCounts`' four
fields so the mapping is one to one and there is no prefix to keep in step. **Not nullable
and never omitted**: ADR 0032 rule 4 makes them a mandatory part of describing a timetable, and
all-zeroes is a real state that still means *"no **confirmed** clash"* rather than conflict-free.
They belong on `timetable` rather than `solve_run` because they describe **the timetable**, and a
hand-built one has them too.

### The tension with the project format, and why it is not a conflict

`PROJECT-FORMAT.md` §3: *"`violation` rows are rebuilt on first score"* and are **not exported**.
That is about *transfer*, and remains correct. Storing them in the project database is about
*restart*, which is a different question: reopening a 2 000-entry timetable and re-evaluating is
fast (M6 measured 57 ms for the whole of `medium`), but the stored rows are what let a view
render violations **without** loading a `Problem` at all — which Phase 7's score health panel
needs and Phase 9's export deliberately does not.

So: stored for restart, rebuilt on import. No change to `PROJECT-FORMAT.md` is needed beyond
adding `timetables.json`'s already-listed content.

### What a stored score means

A stored `solve_run` score is **a historical figure**: what this arrangement scored when it was
generated. The first manual edit in Phase 8 makes it wrong. So:

- **No score columns on `timetable`.** The sketch has three; they would go stale on the first
  edit and a stale score is worse than no score.
- A current score is always a fresh `evaluate` (§7), and any UI showing a stored one labels it
  *"when generated"*.

---

## 5. Q5 — Identity, foreign keys, concurrency, triggers, transactions

### 5.1 Proposed DDL — `V17__timetable_persistence.sql`

```sql
-- A timetable for one cycle and one term. Several may coexist; that is the versioning.
CREATE TABLE timetable (
    id              TEXT    PRIMARY KEY NOT NULL,
    cycle_id        TEXT    NOT NULL REFERENCES cycle (id)    ON DELETE RESTRICT,
    term_id         TEXT             REFERENCES term (id)     ON DELETE RESTRICT,
    name            TEXT    NOT NULL CHECK (length(trim(name)) > 0),
    generated_at    TEXT    NOT NULL,
    -- ADR 0032 rule 4: never omitted, never nullable. All zeroes is still "no CONFIRMED
    -- clash". Named exactly as `UnverifiedCounts`' fields, so the serde mapping is one to one.
    indeterminate_student_pairs INTEGER NOT NULL DEFAULT 0
                                CHECK (indeterminate_student_pairs >= 0),
    unchecked_capacities        INTEGER NOT NULL DEFAULT 0
                                CHECK (unchecked_capacities >= 0),
    shared_entities             INTEGER NOT NULL DEFAULT 0
                                CHECK (shared_entities >= 0),
    unenforced_restrictions     INTEGER NOT NULL DEFAULT 0
                                CHECK (unenforced_restrictions >= 0),
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    rev             INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (cycle_id, COALESCE(term_id, ''), name)
) STRICT;

-- One row per placeable occurrence (D1). The start slot; the run is derived.
CREATE TABLE timetable_entry (
    id           TEXT    PRIMARY KEY NOT NULL,
    timetable_id TEXT    NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    activity_id  TEXT    NOT NULL REFERENCES activity  (id) ON DELETE RESTRICT,
    timeslot_id  TEXT    NOT NULL REFERENCES timeslot  (id) ON DELETE RESTRICT,
    room_id      TEXT             REFERENCES room      (id) ON DELETE RESTRICT,
    is_locked    INTEGER NOT NULL DEFAULT 0 CHECK (is_locked IN (0,1)),
    is_manual    INTEGER NOT NULL DEFAULT 0 CHECK (is_manual IN (0,1)),
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    rev          INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (timetable_id, activity_id, timeslot_id),
    -- One placement per occurrence: the type `Assignment` already enforces, enforced here too.
    UNIQUE (timetable_id, activity_id)
) STRICT;

CREATE TABLE solve_run (
    id             TEXT    PRIMARY KEY NOT NULL,
    timetable_id   TEXT    NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    engine         TEXT    NOT NULL,
    seed           INTEGER NOT NULL,
    status         TEXT    NOT NULL CHECK (status IN
                     ('COMPLETE','PARTIAL','INFEASIBLE','CANCELLED','FAILED')),
    termination    TEXT    NOT NULL CHECK (termination IN
                     ('complete','exhausted','candidate-limit-reached','step-limit-reached',
                      'pass-limit-reached','cancelled','input-rejected','internal-check-failed')),
    settings       TEXT    NOT NULL DEFAULT '{}'
                     CHECK (json_valid(settings) AND json_type(settings) = 'object'),
    score_hard     INTEGER NOT NULL,
    score_medium   INTEGER NOT NULL,
    score_soft     INTEGER NOT NULL,
    -- `SolveStatistics`' six counters, named as its fields are.
    candidate_evaluations  INTEGER NOT NULL CHECK (candidate_evaluations  >= 0),
    candidates_rejected    INTEGER NOT NULL CHECK (candidates_rejected    >= 0),
    placements_committed   INTEGER NOT NULL CHECK (placements_committed   >= 0),
    placements_withdrawn   INTEGER NOT NULL CHECK (placements_withdrawn   >= 0),
    construction_steps     INTEGER NOT NULL CHECK (construction_steps     >= 0),
    passes                 INTEGER NOT NULL CHECK (passes                 >= 0),
    started_at     TEXT    NOT NULL,
    finished_at    TEXT    NOT NULL,
    app_version    TEXT    NOT NULL,
    CHECK (finished_at >= started_at)
) STRICT;

CREATE TABLE solve_run_unscheduled (
    id              TEXT PRIMARY KEY NOT NULL,
    solve_run_id    TEXT NOT NULL REFERENCES solve_run (id) ON DELETE RESTRICT,
    activity_id     TEXT NOT NULL REFERENCES activity  (id) ON DELETE RESTRICT,
    cause           TEXT NOT NULL CHECK (cause IN
                      ('no-resolved-bell-schedule','no-contiguous-run','no-candidate-room',
                       'all-candidates-unavailable','blocked-by-placements','not-reached',
                       'input-rejected')),
    diagnostic_code TEXT,
    UNIQUE (solve_run_id, activity_id)
) STRICT;

CREATE TABLE violation (   -- derived: rebuilt by an evaluation, not exported
    id            TEXT PRIMARY KEY NOT NULL,
    timetable_id  TEXT NOT NULL REFERENCES timetable (id) ON DELETE RESTRICT,
    solve_run_id  TEXT          REFERENCES solve_run (id) ON DELETE RESTRICT,
    constraint_id TEXT          REFERENCES constraint_instance (id) ON DELETE RESTRICT,
    constraint_type TEXT NOT NULL,
    level         TEXT NOT NULL CHECK (level IN ('HARD','MEDIUM','SOFT')),
    penalty       INTEGER NOT NULL,                 -- INTEGER, not the sketch's REAL
    cycle_day_id  TEXT          REFERENCES cycle_day (id) ON DELETE RESTRICT,
    starts_at     TEXT CHECK (starts_at IS NULL OR starts_at GLOB '[0-2][0-9]:[0-5][0-9]'),
    ends_at       TEXT CHECK (ends_at   IS NULL OR ends_at   GLOB '[0-2][0-9]:[0-5][0-9]'),
    detail        TEXT NOT NULL,
    CHECK ((starts_at IS NULL) = (ends_at IS NULL)),
    CHECK (ends_at IS NULL OR ends_at > starts_at)
) STRICT;

CREATE TABLE violation_ref (
    id          TEXT PRIMARY KEY NOT NULL,
    violation_id TEXT NOT NULL REFERENCES violation (id) ON DELETE RESTRICT,
    role        TEXT NOT NULL CHECK (role IN ('activity','subject','related')),
    entity_kind TEXT NOT NULL,
    entity_id   TEXT NOT NULL                       -- polymorphic, so no FK
) STRICT;

CREATE INDEX idx_entry_timetable_slot ON timetable_entry (timetable_id, timeslot_id);
CREATE INDEX idx_entry_timetable_act  ON timetable_entry (timetable_id, activity_id);
CREATE INDEX idx_entry_room           ON timetable_entry (timetable_id, room_id, timeslot_id);
CREATE INDEX idx_timetable_cycle      ON timetable (cycle_id, term_id);
CREATE INDEX idx_solve_run_timetable  ON solve_run (timetable_id);
CREATE INDEX idx_violation_timetable  ON violation (timetable_id, level);
CREATE INDEX idx_violation_ref_v      ON violation_ref (violation_id);
```

**Three indexes are `DATA-MODEL.md` §9's own "PLANNED" block, written as specified.**

### 5.2 If D1 is answered "one row per slot"

`timetable_entry` keeps everything above, drops `UNIQUE (timetable_id, activity_id)`, and gains:

```sql
    span_ordinal INTEGER NOT NULL CHECK (span_ordinal >= 0),   -- 0 = the start slot
    UNIQUE (timetable_id, activity_id, span_ordinal),
```

`span_ordinal` is there so *"which slot of the run is this?"* is stored rather than inferred by
re-sorting on the period's ordinal, and so a gap in the run is detectable. A trigger cannot check
that the row count equals `duration_periods` without a scan per insert, so §8 makes it a
repository check and a test instead.

### 5.3 No `school_id` on `timetable` — departure from the sketch

`CLAUDE.md` §3: *"A child's school is resolved through its parent chain, never by adding it a
second `school_id` column."* `cycle.school_id` is `NOT NULL`, so a timetable's school is
determined. The sketch's `school_id` would be the second place for it to be wrong — exactly the
argument ADR 0029 §1 used to keep it off five tables and `CLAUDE.md` uses to forbid
`teacher.department_id`.

**Consequence**, stated honestly: every school-scoped read joins `cycle`, and a
`require_school_owner`-style resolution is needed for `timetable` as `V15`'s chain needed one.
`repo::require_school_owner` already exists and generalises.

**A trigger pair must enforce that `term` and `cycle` agree about the school** — a timetable for
Northgate's cycle and Southmoor's term is representable otherwise, and `ADR 0032` rule 1 forbids
it. The loader already refuses it as `WrongSchool`; the schema should too, because a trigger is
what makes it true of a file rather than of a code path.

### 5.4 `ON DELETE RESTRICT`, and the repository clears the rows — departure from the sketch

The sketch has `timetable_entry … ON DELETE CASCADE`. **ADR 0040 §2 settles that this is now
wrong**, for a reason that applies identically here:

> SQLite fires a `DELETE` trigger for a row removed by a foreign-key action only when
> `recursive_triggers` is on, and this project never enables it. A `CASCADE` would therefore
> remove scope rows with no change-log entry — silently, and only for the rows reached by the
> cascade, which is the worst possible shape for a gap in an audit trail.

So every child edge is `RESTRICT`, and `Timetables::delete` removes the children itself — one
`repo::delete` each, inside the timetable's transaction, at `Expected::Any`, which is ADR 0027
§4's first documented kind (*a cascade inside another operation's transaction*).

`timetable_entry.timeslot_id → timeslot ON DELETE RESTRICT` is **required by ADR 0011 §5**, which
says in as many words: *"It relies on `RESTRICT` to refuse any slot a future `timetable_entry`
references, which is why timetable entries will not need a cascade: the deletion simply fails,
with `StillReferenced`, and the user is told what is in the way."*

**The cost, measured in advance:** deleting a 2 000-entry timetable writes 2 000 `DELETE`
statements and 2 000 `change_log` rows. A `change_log` row is seven small columns with no payload
(~100 bytes), so ≈200 KB per deleted timetable and the same again per generated one. For a school
generating twenty times that is ≈8 MB of history. Acceptable, and worth knowing — it is **D5**.

### 5.5 Which tables are entity tables

Two tests in `tests_structure.rs` discover tables from the live schema and will fail for any new
table that is neither tracked nor exempt: `every_entity_table_has_all_three_change_log_triggers`
and `every_entity_table_carries_the_common_metadata_columns`. The exemption list is
`NON_ENTITY_TABLES`, and the failure message itself offers it *"if it genuinely holds no school
data"*.

| Table | Proposal | Why |
| --- | --- | --- |
| `timetable` | **Entity** — audit columns, three triggers, `TRACKED_TABLES` | A school's artefact, named and deleted by a user, and exactly what a future sync reconciles |
| `timetable_entry` | **Entity** | Phase 8 edits these directly; `is_manual` exists to record that a human placed one |
| `solve_run` | **Entity** | Provenance. ADR 0003's sync-ready requirement and ADR 0040's own argument — *"a table whose rows carry a school's configuration decisions is exactly a table a future synchronisation has to reconcile"* — apply to a record of how a timetable came to be |
| `solve_run_unscheduled` | **`NON_ENTITY_TABLES`** | Derived from one run and immutable with it |
| `violation` | **`NON_ENTITY_TABLES`** | Derived. `PROJECT-FORMAT.md` §3 already calls these *"derived data … rebuilt on first score"* |
| `violation_ref` | **`NON_ENTITY_TABLES`** | Same |

Making `solve_run` an entity means it gains `created_at`/`updated_at`/`rev` **on top of**
`started_at`/`finished_at`. Those are different facts — when the row was written versus when the
run ran — and the duplication is only apparent. This split is **D4**.

### 5.6 Concurrency

| Mutation | Contract | ADR 0027 §3 category |
| --- | --- | --- |
| `create_timetable` (with entries, run, violations) | **none** — there is no previous version of a row that does not exist | 8 — Creation |
| `rename_timetable` | `expectedRev` of the timetable | 1 — Single-record update |
| `delete_timetable` | `expectedRev` of the timetable; children at `Expected::Any` | 2 + the §4 cascade |
| `replace_violations` (after a re-evaluation) | `expectedRev` of the **timetable**, children at `Any` | 3 — specialised update: *the row that changes* |
| Phase 8's `apply_move` | `expectedRev` of the **entry** | 1 — not M7's |

`CLAUDE.md`: *"A new mutation is added to one of those inventories, never left out of both."*
M7's ADR carries its own inventory section, as ADR 0029 §8 did for Stage C's sixteen.

### 5.7 Transactions

**One `db.write` per save.** `timetable` + N `timetable_entry` + `solve_run` + N
`solve_run_unscheduled` + N `violation` + N `violation_ref`, all or nothing. A failure on the last
entry must leave none of the others — the same reason ADR 0030 put the import engine inside
`evara-db` rather than composing it from public repository methods.

`Database::write` is the only route and is `pub(crate)`, so this is structural.

---

## 6. Q6 — Revalidating a stored timetable when inputs change

### The problem is not referential

`RESTRICT` guarantees a stored entry's activity, timeslot and room still **exist**. It guarantees
nothing about their **facts**. All of these invalidate a stored timetable without breaking a
single foreign key:

- a teacher's availability gains an `UNAVAILABLE` rule covering a placed slot
- a room's `is_bookable` goes to 0, or it loses the campus association
- a course's `required_room_type` changes
- an `activity.duration_periods` grows, so the stored start no longer has a run
- a `period`'s `starts_at` moves, changing which intervals overlap
- a group's `children_are_disjoint` is answered, turning `Indeterminate` into `Overlaps`
- a `class_teacher` row is added, so two lessons now share a teacher

### Proposal: revalidation is a function, not a flag

**Do not store a validity flag.** A stored boolean is a claim about the world at the moment it was
written, and every one of the changes above would falsify it silently — which is the class of
defect `CLAUDE.md` rejects whenever it says *"a second place for a fact to be wrong"*.

Instead, revalidation is: **load the problem for the timetable's `(cycle, term)`, read the entries
back into an `Assignment`, and run M5's `evaluate`.** That introduces *no new scheduling
semantics* — it is the existing engine over an arrangement read from disk, which is exactly what
`evara evaluate` already does for an arrangement it is handed.

`evara evaluate --project` today evaluates the **empty** arrangement, because *"a project has no
stored timetable to score — `timetable` arrives in M7"*. M7 makes that sentence obsolete:
`evara evaluate --project --timetable <id>` scores the stored one, and the command's own
documentation loses its caveat.

### What a stored evaluation means

`violation` rows are a **snapshot**: what the rules found when the evaluation ran. So each stored
violation set is attributed — `solve_run_id` where it came from a generation, `NULL` where it came
from a later re-evaluation — and any view showing them says when. A re-evaluation **replaces** the
set for that timetable in one transaction; there is no merging, because two snapshots of a
changing world are not additive.

### Deliberately not proposed: a staleness fingerprint

A cheap `inputs_fingerprint` on `timetable`, compared on open, would let a screen say *"these
inputs have changed since this timetable was generated"* without a full evaluation. It is
tempting and I am **not** proposing it for M7, for two reasons:

1. ADR 0030 is explicit that *"Never compare a fingerprint in the interface — that is
   check-then-act"*, and the only safe use of one is inside the transaction that acts on it.
   Advisory display is a third use that record does not sanction.
2. A fingerprint over *every fact a timetable depends on* is most of the loader. Getting it subtly
   narrow would produce the worst outcome — a timetable reported as current that is not.

Evaluation is fast enough to need no shortcut: M6 measured the whole `medium` school at 57 ms
including the solve. **D8** records this as a decision rather than an omission.

---

## 7. Q7 — Mapping `SolveOutcome` and `ProblemIndex` without duplicating semantics

### The translation, and nothing more

```
SolveOutcome.assignment : Vec<Option<Placement>>   indexed by ActivityIx
     + ProblemIndex.activity_id(ActivityIx) -> Uuid
     + ProblemIndex.timeslot_id(TimeslotIx) -> Uuid
     + ProblemIndex.room_id(RoomIx)         -> Uuid
     =  timetable_entry rows
```

`ProblemIndex`'s reverse accessors are already `pub` (generated by the `problem_index!` macro) and
`Assembly::index` is a `pub` field, so the writer needs no new API from M4. **This is the whole
reason M4 returned the maps beside the problem** rather than inside it.

The writer's signature is therefore:

```rust
// evara-db, repo/timetables.rs
pub fn save_generated(
    &mut self,
    assembly: &Assembly,          // the problem the solve ran against, with its index
    outcome: &SolveOutcome,       // what the solver produced
    run: &RunProvenance,          // engine name, settings JSON, started_at, finished_at
    violations: &[Violation],     // from the caller's own evaluate(), already structured
    name: &str,
) -> Result<Timetable>;
```

### Four guards, because a plausible wrong timetable is the failure mode

| Guard | Refusal |
| --- | --- |
| `outcome.problem` must equal `assembly.selection` | `DbError::WrongSchool`-shaped. A `SolveOutcome` written against a *different* assembly's index would translate every index to the wrong UUID and produce a complete, plausible, wrong timetable |
| `outcome.assignment.len()` must equal `assembly.problem.activities.len()` | the same failure, one step subtler |
| every index must resolve to a UUID | `DbError::CorruptIdentifier`. The reverse of `Problem::integrity`, which checks the forward direction |
| `outcome.status` must be usable, or the caller must say it means to store an unusable one | §8's edge case |

### What the writer does **not** do

- **It does not evaluate anything.** The solver refused every hard-violating placement and
  `generate` re-checked with the reference scorer; a third check in `evara-db` would be a fourth
  caller of the rules in a crate forbidden to depend on them.
- **It does not resolve a span** (under D1) — there is nothing to resolve.
- **It does not compute the unverified counts.** `generate` computes them from the problem; the
  writer copies four integers.
- **It does not author a message.** No `reason` string, no explanation. Those are rendered where
  the rule lives.

### Violations cross as `evara-core` types

`Violation` is `evara_core::scheduling` and `Serialize`, so the caller — which has both crates —
hands the writer a `&[Violation]`. `evara-db` names the vocabulary and never the engine, which is
the same division ADR 0039 §5 set up for `StoredConstraint`: *"the vocabulary two crates share has
to sit underneath both."*

---

## 8. Edge cases

| # | Case | Proposed behaviour |
| --- | --- | --- |
| 1 | Saving a timetable with **zero** entries (every occurrence unplaced) | Allowed. A `Partial` result with nothing placed is still a record of an attempt, and its `solve_run_unscheduled` rows are the useful part |
| 2 | Saving a **refused** run (`input-rejected`) | **Refused by the repository.** Nothing was attempted; there is no timetable. The findings belong to the report, not to a stored artefact. Needs approval — it is a judgement, not a record |
| 3 | Saving a run whose status is `Failed` / `internal-check-failed` | **Refused.** Storing a timetable the engine's own check rejected would put an invalid artefact in the file |
| 4 | Two generations racing on one `(cycle, term)` | Both succeed as separate rows. `UNIQUE (cycle_id, COALESCE(term_id,''), name)` makes a name collision a `DuplicateValue`, which the caller resolves by naming |
| 5 | Deleting a `timeslot` a stored entry references | Refused by `RESTRICT` as `StillReferenced` — ADR 0011 §5's stated intent |
| 6 | Deleting an `activity` a stored entry references | Refused by `RESTRICT`. Phase 8 will want "delete the lesson and its placements", which is a deliberate two-step, not a cascade |
| 7 | Deleting a `room` a stored entry references | Refused. A room in use is not deletable, which matches `V15`'s posture everywhere else |
| 8 | A grid **rematerialisation** that would orphan a slot an entry uses | `apply_materialisation` never deletes (ADR 0011 case C); `delete_timeslots` is refused by `RESTRICT`. No new code |
| 9 | `term_id IS NULL` (a whole-year grid) | Ordinary. `COALESCE` in the uniqueness index; the loader already treats `None` as a real selection |
| 10 | A `cycle` and a `term` belonging to **different schools** | Refused by the trigger pair of §5.3 |
| 11 | Re-evaluating a timetable whose inputs now make it infeasible | Succeeds, and stores hard violations. A stored timetable is allowed to be invalid — that is what the user needs to see |
| 12 | Re-evaluating when the problem no longer **loads** (no grid, ambiguous schedule) | The evaluation fails with the loader's error. No violations are written and the previous set is left alone |
| 13 | An entry whose `activity` now resolves to a different bell schedule | Not a referential problem; surfaces as a `duration-contiguous` violation on re-evaluation |
| 14 | A read-only project (`open_for_inspection`) | `Database::write` refuses with `ReadOnlyProject` before a transaction opens. `evara generate --project` stays read-only; **saving is a separate, explicitly-writing command** |
| 15 | `.evara` export/import | `violation`, `violation_ref`, `solve_run_unscheduled` are **not exported** (derived); `timetable`, `timetable_entry`, `solve_run` are. Step 6 of `PROJECT-FORMAT.md` §5 rebuilds the violations |
| 16 | A `constraint_instance` deleted after a violation referenced it | `violation.constraint_id` is `RESTRICT`, so the delete is refused while a stored violation names it. Alternative — `SET NULL` — would rewrite rows without bumping `rev`, which R10 forbids |

---

## 9. Enabling change: promote span resolution to `evara-core` (D2)

Independent of D1, and worth doing either way.

`Occupancy::resolve` turns a `Placement` into a `PlacementSpan` plus a `SpanDefect`. It reads only
`Problem::slots_of`, `TimeslotFact` and `PlacementSpan::over` — **all `evara-core`** — so nothing
blocks the move. Proposal:

- `evara_core::scheduling::placement::resolve_span(problem, activity, placement) -> Option<ResolvedSpan>`
- `evara-constraints::Occupancy` calls it instead of owning it; `SpanDefect` moves with it.
- `evara-db`'s reader and `evara-report`'s projections call it for *"what occupies this slot?"*.

This is the fourth application of a pattern the repository already has three records for: ADR 0036
(one bell-schedule resolver), M6's `Rng` promotion, and ADR 0041 (the scorer kept with the
evaluators). The rule it serves is the same — one implementation, several callers, so generation,
validation, the editor and now the **reports** cannot disagree.

Without it, Phase 7's grid has to answer a span question with no span function, and the most
likely outcome is a fifth implementation in TypeScript, which `CLAUDE.md` §2 calls an architecture
bug by name.

---

## 10. Decisions requiring approval

| # | Decision | Recommendation | Cost if the other way |
| --- | --- | --- | --- |
| **D1** | `timetable_entry`: one row per **occurrence**, or one per occupied **slot**? Contradicts a stated reason in `DATA-MODEL.md` §7 and ADR 0029 §2 either way (C1, C3) | **Per occurrence.** Nothing derived is stored, so a stored timetable cannot be internally inconsistent and a partial multi-period move is unrepresentable | Per slot needs D2 *and* a repository check that the row count matches `duration_periods`; §5.2 has the DDL |
| **D2** | Promote span resolution from `evara-constraints` to `evara-core` | **Yes.** Needed by Phase 7 regardless, and the only alternative under per-slot is a forbidden second implementation | Phase 7 invents its own span logic |
| **D3** | Omit `status` (`DRAFT`/`PUBLISHED`/`ARCHIVED`) and `based_on_timetable_id` from `V17`, deferring the lifecycle per §6 of the M1 record (C2) | **Omit both.** Versioning is by rows; a `CHECK` with three values only one of which is ever written is a column that lies | Implementing the lifecycle contradicts an accepted record; two columns later is a one-line migration |
| **D4** | `solve_run` as a tracked **entity** (audit columns + triggers), with `violation`, `violation_ref` and `solve_run_unscheduled` in `NON_ENTITY_TABLES` | **As stated.** Provenance is reconcilable; derived findings are not | Everything tracked ⇒ change-log noise per violation; nothing tracked ⇒ a generated artefact outside the change feed |
| **D5** | Accept ≈200 KB of `change_log` per 2 000-entry timetable written **and** per one deleted (§5.4) | **Accept.** The audit gap a `CASCADE` would leave is worse, and ADR 0040 already rejected that trade once | A `CASCADE` needs `recursive_triggers`, which ADR 0040 rejected for blast radius |
| **D6** | No `school_id` on `timetable`; resolve through `cycle` (§5.3) | **No column**, plus a trigger pair so `cycle` and `term` agree about the school | Every school-scoped read gains a join — which `V15`'s chain already pays |
| **D7** | `ON DELETE RESTRICT` on every child edge, repository clears (§5.4) | **As stated**, per ADR 0040 §2 and ADR 0011 §5 | — |
| **D8** | No staleness fingerprint on `timetable`; revalidation is always a real evaluation (§6) | **No fingerprint.** ADR 0030 sanctions one only inside the transaction that acts on it | A subtly narrow fingerprint reports a stale timetable as current, which is the worst outcome |
| **D9** | Refuse to save a run that was `input-rejected` or `Failed` (§8 cases 2–3) | **Refuse.** Nothing was attempted, or the engine's own check rejected it | Storing it puts an invalid artefact in the user's file |
| **D10** | Is saving part of `evara generate`, or a separate command? | **Separate.** `generate` keeps M6's read-only guarantee and its byte-comparison test; a new `evara save` / Tauri command writes. A `--save` flag on a command documented as read-only would make that documentation false | One command is fewer steps, at the cost of M6's guarantee |

---

## 11. Migration strategy

- **One migration, `V17__timetable_persistence.sql`.** Forward-only; `V1`–`V16` untouched. A
  released migration is immutable (refinery checksums), so a mistake is fixed by `V18`.
- **Purely additive.** Six new tables, seven indexes, one trigger pair. **No existing row is
  rewritten and no existing column changes**, so every existing project opens unchanged and reads
  the new tables as empty — which is the correct behaviour for a project that has never generated.
- **A deviations header**, as `V16` has. The three departures from `DATA-MODEL.md` §7 (`school_id`,
  `RESTRICT`, `penalty INTEGER`) plus whatever D1 decides, recorded **in the migration**, because a
  migration is immutable and its next reader is the person who needs to know.
- **`change_log::TRACKED_TABLES`** gains `timetable`, `timetable_entry`, `solve_run` under a
  `-- V17` comment. `install_all` runs after migrations on every open, so the triggers arrive
  without the migration creating them.
- **`NON_ENTITY_TABLES`** in `tests_structure.rs` gains `violation`, `violation_ref`,
  `solve_run_unscheduled`, each with a one-line reason.
- **No backfill.** There is nothing to backfill.

### Documentation to update

`DATA-MODEL.md` §7 and §9 (sketch → built, with the deviations) · `PROJECT-FORMAT.md` §3
(which tables are derived) · `ROADMAP.md` Phase 6 (the solution writer and `solve_run`
persistence) · `SOLVER.md` §8 Layer 2 · `CONSTRAINT-ENGINE.md` §8 and §9 (`evara evaluate
--project` scores a stored timetable; "persisted violations" leaves the not-here table) ·
`TIMETABLE-CONSTRUCTION-SOLVER.md` §17 (**the §17 item 1 correction of C3**) · `CLAUDE.md` ·
a new **ADR 0042** carrying D1–D10 and M7's mutation inventory.

---

## 12. Testing plan

**Schema and infrastructure**

- The two discovery tests pass: every new entity table has three triggers and the four metadata
  columns; every tracked table exists.
- A `V16` project opens at `V17` with the six tables empty and no row rewritten — the
  backward-compatibility shape every migration test in this repo already uses
  (`write_a_pre_v13_project` and friends).
- `STRICT` on every table; every `CHECK` exercised by a refused insert.

**The writer**

- `save_generated` over each M2 fixture: entry count equals `placed_count()`; every
  `activity_id`/`timeslot_id`/`room_id` resolves; the four unverified counts round-trip.
- **Round-trip equality**: read the entries back into an `Assignment` and assert it equals
  `outcome.assignment`. Under D1 this is exact; it is the single most valuable test in the suite.
- The four guards of §7 each refused, with the right error: mismatched identity, mismatched
  activity count, an unresolvable index, a refused status.
- One transaction: a forced failure on the last entry leaves **no** `timetable` row.
- Every mutation's revision contract, as `tests_concurrency.rs` does for the existing 100.

**Versioning**

- Two generations on one `(cycle, term)` produce two timetables; the first is byte-identical
  afterwards.
- A name collision is `DuplicateValue`.
- Delete removes entries, run, unscheduled and violations, writes a `change_log` entry per row, and
  a stale `expectedRev` is `Conflict` with nothing removed.

**Referential refusals** — §8 cases 5, 6, 7, 10, 16, each asserting the specific error.

**Revalidation**

- Generate, save, re-evaluate: the stored violation set equals a fresh `evaluate`.
- Mutate an input (add an `UNAVAILABLE` rule over a placed slot), re-evaluate, and assert the new
  violation appears and the old set is **replaced**, not merged.
- Re-evaluate a timetable whose problem no longer loads: the error propagates and the stored
  violations are untouched.

**End to end, over a real project** — the `tests_walkthrough.rs` shape: create a school, lessons,
grid; load; generate; save; reopen the file; read the timetable back; evaluate it; delete it.

**CLI** — `evara evaluate --project --timetable <id>` scores the stored arrangement; `evara
generate --project` still changes **no byte** of the file (M6's test, which must keep passing).

**Determinism** — the same seed, saved twice into two projects, produces identical entry sets
modulo the row `id`s.

---

## 13. What this proposal deliberately does not include

Manual editing and `apply_move` (Phase 8) · locks as a *solver* input (`activity.is_locked`,
Phase 8) · repair and `based_on_timetable_id` (`SOLVER.md` §7) · the published/archived lifecycle
(§6 of the M1 record) · timetable **views** and projections (Phase 7) · `.evara` export of the new
tables (Phase 9) · `coverage` and `absence` (`DATA-MODEL.md` §8) · a Generate screen · any
soft-constraint or local-search work.

---

**Nothing in this document is implemented.** No migration, no repository, no test, no schema
change. The branch holds only this file. Awaiting a decision on D1–D10.
