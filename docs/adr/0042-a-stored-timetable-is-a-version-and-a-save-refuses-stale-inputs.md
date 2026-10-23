# ADR 0042 — A stored timetable is a version, a save refuses stale inputs, and history is not overwritten

- Status: **Accepted**
- Date: 2026-10-23
- Milestone: M7 — Timetable persistence and versioning
- Schema: **`V17`**
- Related: [ADR 0010](0010-structure-deletion-semantics.md),
  [ADR 0011](0011-stable-timeslot-identity.md),
  [ADR 0027](0027-revisions-are-checked-on-write.md),
  [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md),
  [ADR 0030](0030-an-import-is-a-reviewed-plan.md),
  [ADR 0032](0032-timetable-generation-is-scoped-to-one-school.md),
  [ADR 0040](0040-the-constraint-tables-are-entities-and-their-scope-is-cleared.md),
  [ADR 0041](0041-the-incremental-scorer-lives-with-the-evaluators.md)

---

## 1. Context

M6 produces a `SolveOutcome` and persists nothing. `DATA-MODEL.md` §7 sketches `timetable`,
`timetable_entry`, `solve_run`, `violation` and `violation_ref` as *"PLANNED"* — tables no
migration defines — and the sketch predates ADR 0011, ADR 0027, ADR 0030, ADR 0040 and the two
schema-coverage tests. Several of its details are no longer right, and one of them cannot be
implemented at all under the current layering.

Seven questions were put to an architecture review; this records the ten decisions that came
out of it, with the four corrections the review was approved subject to.

## 2. Decisions

### D1 — One `timetable_entry` row per occurrence, not per occupied slot

`DATA-MODEL.md` §7 explains its key as existing *"because a multi-period activity occupies
several slots"*, and ADR 0029 §2 repeats that as supporting evidence. **The key is kept verbatim
and the reason is not followed.** One row per occurrence, carrying the **start** slot.

Two arguments, and the second is decisive.

**Rows 2…`d` are derived.** They follow from the start slot and `activity.duration_periods`, and
`CLAUDE.md` fixes the direction: *"a duration is a count of periods and the wall-clock interval
is derived from the periods covered — never the other way round."* Storing them makes a corrupt
timetable representable — three rows for a two-period lesson, and something must then decide
which is true — and makes a **partial multi-period move** representable in Phase 8, which is the
state the sketch itself calls impossible (*"written and moved as a unit"*).

**The layering forbids the obvious implementation.** Storing per slot requires the writer to
resolve which slots an occurrence covers. There was exactly one implementation of that,
`Occupancy::resolve`, `pub(crate)` in `evara-constraints` — a crate `evara-db` must not depend on
(`ARCHITECTURE.md` §3 rule 4). The alternatives were a second copy of the measure, which is
`CLAUDE.md` §2's most important rule broken, or a public API wide enough to make that copy easy
for somebody else to write later.

`UNIQUE (timetable_id, activity_id)` is added alongside the sketch's key, so the schema states
what the type already enforces: an `Assignment` is one `Option<Placement>` per activity.

### D2 — Span resolution moves to `evara-core`

`Occupancy::resolve` becomes `evara_core::scheduling::span::resolve_span`, beside `runs_on` and
`PlacementSpan`, which it already used. `evara-constraints` calls it; `Placed` becomes an alias
for `ResolvedSpan` so the eight evaluators are untouched — and the proof the move was faithful is
that all 69 of their tests passed unedited.

Done regardless of D1, because three callers need the same answer and two of them could not
reach it: `evara-db`'s reader, and `evara-report`'s Phase 7 grid, which renders a multi-period
lesson as a span. Both depend on `evara-core` only. The fourth application of a pattern this
repository already has three records for — ADR 0036 (one bell-schedule resolver), M6's `Rng`
promotion, and ADR 0041.

Without it the most likely outcome was a fifth implementation in TypeScript, which `CLAUDE.md`
§2 names as an architecture bug.

### D3 — Versioning is by rows. No `status`, no lifecycle

A `timetable` row is one timetable for one `(cycle, term)`. **Several may coexist, and that is
the versioning.** No version number, no status column, no "current" pointer.

| Operation | Behaviour |
| --- | --- |
| Generate | always **inserts**. Never overwrites, never asks |
| Regenerate | the same thing; the previous timetable is untouched and still openable |
| Replace | not an operation — a save, then an explicit delete. Two decisions, not one silent one |
| Delete | explicit, carries the timetable's revision, removes its children in one transaction |
| Preserve | the default |

`status` with `DRAFT`/`PUBLISHED`/`ARCHIVED` is the published-archived lifecycle, which
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 lists among what is **not in the MVP**. A `CHECK` with
three values of which only one is ever written is a column that lies about being used.
`based_on_timetable_id` is scenario lineage, which is what **repair** needs (`SOLVER.md` §7), and
repair is not built. Both are one additive column later.

Preservation is the default and not a setting because ADR 0010 makes deletion *"follow ownership,
and default to refusing"*, and a regeneration that silently replaced the previous timetable would
destroy the one thing a timetabler compares against.

### D4 — Three of the six new tables are entities

| Table | Tracked? | Why |
| --- | --- | --- |
| `timetable` | ✅ | a school's artefact: named and deleted by a user, edited by Phase 8 |
| `timetable_entry` | ✅ | Phase 8 edits these directly; `is_manual` exists to record that a human placed one |
| `solve_run` | ✅ | provenance. ADR 0040's own argument reaches it: *"a table whose rows carry a school's configuration decisions is exactly a table a future synchronisation has to reconcile"* |
| `timetable_evaluation` | ✅ | not the findings, but the record that a scoring happened and what it concluded. Replacing the current one is a change a reader of the feed should see |
| `violation`, `violation_ref` | `NON_ENTITY_TABLES` | derived. `PROJECT-FORMAT.md` §3 already calls them *"derived data … rebuilt on first score"*, and re-running `evaluate` reproduces them |
| `solve_run_unscheduled` | `NON_ENTITY_TABLES` | written once, never updated, removed only with its run. No independent lifecycle and no `rev` to race on |

Two schema-coverage tests discover tables from the live schema and fail for any that is neither
tracked nor exempt, so this split is enforced rather than remembered.

### D5 — The change-log cost of the `RESTRICT` deletes is accepted

Deleting a 2 000-entry timetable writes 2 000 `DELETE` statements and 2 000 `change_log` rows,
and saving one writes the same again. A `change_log` row is seven small columns with no payload,
so ≈200 KB per timetable written and per one deleted.

Accepted. The audit gap a `CASCADE` would leave is worse, and ADR 0040 already rejected that
trade once.

### D6 — No `school_id` on `timetable`

`CLAUDE.md` §3: *"A child's school is resolved through its parent chain, never by adding it a
second `school_id` column."* `cycle.school_id` is `NOT NULL`, so a timetable's school is
determined. The sketch's column would be the second place for it to be wrong — the argument
ADR 0029 §1 used to keep it off six tables and `CLAUDE.md` uses to forbid `teacher.department_id`.

The cost is a `cycle` join on every school-scoped read, which `V15`'s lesson chain already pays.
A **trigger pair** enforces that `cycle` and `term` agree about the school, because ADR 0032 rule
1 scopes a timetable to one school and nothing else would refuse a row naming Northgate's cycle
and Southmoor's term. A second trigger pair enforces that an entry's timeslot belongs to the
timetable's own cycle.

### D7 — Every child edge is `RESTRICT`, and the repository clears the rows

The sketch has `timetable_entry … ON DELETE CASCADE`. ADR 0040 §2 settles that this is now wrong:
SQLite fires a `DELETE` trigger for a foreign-key action only under `recursive_triggers`, which
this project never enables, so a `CASCADE` would remove rows **with no change-log entry** —
silently, and only for the rows the cascade reached.

`timetable_entry.timeslot_id` is `RESTRICT` for a second, independent reason: ADR 0011 §5 says so
by name — *"It relies on `RESTRICT` to refuse any slot a future `timetable_entry` references,
which is why timetable entries will not need a cascade."*

### D8 — No staleness flag and no fingerprint on `timetable`; revalidation is a function

A stored validity flag is a claim about the world at the moment it was written, and every change
below would falsify it silently:

- a teacher's availability gains an `UNAVAILABLE` rule covering a placed slot
- a room's `is_bookable` goes to 0, or it loses its campus association
- a course's `required_room_type` changes; an `activity.duration_periods` grows
- a `period`'s `starts_at` moves, changing which intervals overlap
- a group's `children_are_disjoint` is answered, turning `Indeterminate` into `Overlaps`

**None of these breaks a foreign key.** So revalidation is: load the problem, read the entries
back into an `Assignment`, and run M5's `evaluate`. That introduces no new scheduling semantics —
it is the existing engine over an arrangement read from disk.

A cheap `inputs_fingerprint` compared on open was considered and rejected: ADR 0030 is explicit
that *"Never compare a fingerprint in the interface — that is check-then-act"*, and a fingerprint
over *every fact a timetable depends on* is most of the loader. Getting it subtly narrow produces
the worst outcome — a stale timetable reported as current.

### D9 — A refused or broken outcome is not saveable

`Termination::InputRejected` attempted nothing; the findings belong to the report, not to a
stored artefact. `Termination::InternalCheckFailed` produced an arrangement the **reference**
scorer rejected, and storing it would put something in the user's file that no screen could
honestly render. Both are `DbError::NotSaveable`.

### D10 — Saving is a separate command from generating

`evara generate` stays read-only, with its byte-comparison test intact. `evara save` writes. A
`--save` flag would make `generate`'s documentation true for some invocations and false for
others, which is the worst state for a guarantee: a user reading the help text could not tell
whether the command they are about to run touches their file.

The asymmetry runs deeper than naming. `generate` opens the project itself through
`Database::open_for_inspection`; `save` must go through `Workspace`, because every change-log
trigger calls `evara_site_id()` and that identity lives in the **settings** database
(`CLAUDE.md`). A command that opened the project directly would have no site id to offer and
every insert would be refused.

## 3. The four corrections the approval was subject to

### C1 — `UNIQUE` with `COALESCE` is not table-level SQLite syntax

Correct. SQLite permits an expression only in an **index**, so the rule is:

```sql
CREATE UNIQUE INDEX idx_timetable_scope_name
    ON timetable (cycle_id, COALESCE(term_id, ''), name);
```

The `COALESCE` is what `CLAUDE.md` requires of a nullable column in a uniqueness rule: SQLite
treats `NULL`s as distinct, so without it the constraint would silently fail to hold for exactly
the **whole-year** timetables — which are the common case.

### C2 — Stale generation protection

The mechanism is the **change-log watermark**, and it required no new fingerprint.

`change_log.seq` is `INTEGER PRIMARY KEY AUTOINCREMENT` and every tracked table writes to it, so
the maximum is an **exact** statement of where a project stood — not a sample of it, and not a
hash over a chosen set of facts that could be one table short. `AUTOINCREMENT` also guarantees a
sequence number is never reused, so a watermark taken before a delete cannot be matched after one.

- The loader records it on every `Assembly`, read last so it covers everything the load read.
- `save_generated` compares it against the feed **inside the writing transaction** and refuses
  `DbError::InputsChanged`. That is ADR 0030's shape — *"rebuilds the plan inside the transaction
  and refuses if the fingerprint has moved"* — and it is a predicate on the write rather than a
  question asked beforehand, so it is not check-then-act. ADR 0030's other rule holds too: no
  caller compares a watermark itself.
- **Safe by default.** Only the four tables M7 itself writes are classified as *outputs*;
  everything else counts as an input. A table added by a future migration and never classified is
  therefore treated as an input, so the check refuses where it need not have. The opposite default
  would accept a save it should have refused, silently — and refusing too often is a nuisance
  while accepting too often is a wrong timetable.

Why a fingerprint could not do this job: a fingerprint has to enumerate what a `Problem` depends
on, and a watermark cannot be too narrow.

The limitation, named: an edit to a scheduling table that the problem happens not to read still
invalidates a pending outcome. The remedy is to generate again, which the refusal says.

### C3 — Historical versus current violations

The proposal hung violations off the timetable and would have deleted the generation's findings to
replace the current ones — destroying the provenance `solve_run` exists to record. Corrected by a
table the proposal did not have:

```
timetable_evaluation (id, timetable_id, kind, solve_run_id NULL, evaluated_at,
                      inputs_watermark, score_hard/medium/soft,
                      placed_count, required_count, …)
violation            (id, evaluation_id, …)
```

- **`GENERATION`** is what the solver found, belongs to its run, and is **immutable for the life
  of the timetable**.
- **`REVALIDATION`** is what the rules find now, and is replaced wholesale — two scorings of a
  changing world are not additive.
- `record_revalidation` touches **only** the `REVALIDATION` row, so re-scoring can never
  misrepresent how a timetable was generated.
- Two partial unique indexes enforce at most one of each kind per timetable, and a `CHECK` makes
  `solve_run_id` present exactly for a `GENERATION`.

This also makes *"a stored score is a historical figure"* structurally true: a score belongs to
the evaluation that produced it, so there is no score on `timetable` to go stale on the first
manual edit.

### C4 — Export of unscheduled causes

`solve_run_unscheduled` is **historical generation information, not derived**: once the inputs
move, nothing can reconstruct which lessons a past run failed on or why. So the planned export
contract is:

| Table | Exported? |
| --- | --- |
| `timetable`, `timetable_entry`, `solve_run`, `timetable_evaluation` | **yes** |
| `solve_run_unscheduled` | **yes** — historical, not reconstructible |
| `violation`, `violation_ref` | **no** — derived, rebuilt by an evaluation |

Recorded in `PROJECT-FORMAT.md` §3 and **not implemented**: export is Phase 9's.

## 4. The mutation inventory

`CLAUDE.md`: *"A new mutation is added to one of those inventories, never left out of both."*
M7 adds five, bringing the total to 105.

| Mutation | Contract | ADR 0027 §3 category |
| --- | --- | --- |
| `save_generated` | none — there is no previous version of a row that does not exist. Guarded instead by the **watermark**, which is a precondition on the whole problem rather than on one row | 8 — Creation |
| `rename` | `expectedRev` of the timetable | 1 — Single-record update |
| `delete` | `expectedRev` of the timetable; children at `Expected::Any` | 2, plus §4's cascade |
| `record_revalidation` | `expectedRev` of the timetable, which it **consumes**: the timetable's current findings are part of what it says about itself | 3 — the revision of the row that changes |
| `arrangement` / the reads | none; `Scheduling`-style read-only | — |

One implementation note worth recording, because it was a real defect caught by a test: `delete`
originally checked the revision with a no-op `update` before removing the children. That
**consumes** the revision it was meant to check and then fails its own predicate. The parent's
own delete carries the revision instead, last, and the transaction rolls the children back if it
does not match — the same guarantee without a second write.

## 5. Consequences

- `V17` is additive: six tables, nine indexes, four triggers. No existing row is rewritten and
  every existing project reads the new tables as empty.
- `evara-db` gains `evara-constraints` and `evara-solver` as **dev**-dependencies, so its tests
  can save a *real* `SolveOutcome` rather than a hand-built one that could drift from what the
  solver returns. The `evara-import` dev-dependency records the same argument, and neither crate
  depends on `evara-db`, so there is no cycle.
- `evara evaluate --project` can now score a **stored** timetable, so the caveat in
  `CONSTRAINT-ENGINE.md` §8 — *"A project has no stored timetable to score"* — is retired.
- Phase 7's grid has a span function to call. Phase 8's editor has `timetable_entry` rows with
  revisions, `is_locked` and `is_manual` already in place.
- Nothing in M7 narrows what the solver may do, and no scheduling rule exists in `evara-db`.

## 6. Alternatives rejected

**One row per occupied slot (D1's alternative).** Rejected on the derived-data argument and on
the layering. Had it been chosen, D2 would have been mandatory rather than merely wise, and a
repository check that the row count matches `duration_periods` would have been needed — a check
that exists only because the representation allows the error.

**`status` with the published-archived lifecycle.** Rejected as out of the MVP by
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6. Adding the column and only ever writing `DRAFT` was
also rejected: a column that lies about being used is worse than an absent one.

**A score cached on `timetable`.** Rejected: it goes stale on the first manual edit, and a stale
score is worse than no score. The score belongs to its evaluation.

**A validity flag or an inputs fingerprint on `timetable`.** Rejected per D8.

**A single `violation` set per timetable, replaced on revalidation.** Rejected by C3: it cannot
express the difference between what the generator found and what is wrong now, so a revalidation
would have to destroy provenance to record a current state.

**`CASCADE` plus `PRAGMA recursive_triggers = ON`.** Rejected for the second time, on ADR 0040's
reasoning: the pragma changes trigger behaviour for the whole database, including every existing
`RESTRICT` edge and every change-log trigger written on the assumption that it is off.

**A `--save` flag on `evara generate`.** Rejected per D10.

**Storing the engine's unplaced-reason sentence.** Rejected: a wording improvement would leave
old rows reading the old way. The cause is stored and survives a wording change; the section name
a sentence needs is joined through `activity → class_section` rather than copied.
