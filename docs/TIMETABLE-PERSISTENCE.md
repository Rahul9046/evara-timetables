# Timetable Persistence and Versioning

What `V17` stores, what a save refuses, and what a stored timetable does and does not claim.

Shipped in **M7**. This is the first milestone in which a generated timetable survives closing
the application.

[DATA-MODEL.md](DATA-MODEL.md) §7 is the **schema**; this file is the **behaviour**.
[ADR 0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) is the
decision record, including the six places `V17` departs from the §7 sketch and why.

---

## 1. The shape of it

```
evara-core          vocabulary:  SolveOutcome, Assignment, Placement, Violation,
                                 UnverifiedCounts, span::resolve_span        <- moved in M7
   ^
evara-db            the only SQL: V17, repo/timetables.rs, change_log::Watermark
   ^
evara-cli           the harness:  evara save, evara evaluate --timetable
```

Seven tables, twelve indexes and four triggers, all additive.

| Table | Holds | Tracked? |
| --- | --- | --- |
| `timetable` | one timetable for one `(cycle, term)` | ✅ entity |
| `timetable_entry` | one row per placeable occurrence: where it **starts**, and in which room | ✅ entity |
| `solve_run` | how it came to be: engine, seed, limits, six counters, termination, watermark | ✅ entity |
| `timetable_evaluation` | one scoring: `GENERATION` or `REVALIDATION` | ✅ entity |
| `solve_run_unscheduled` | what a run could not place, and which kind of answer each lesson got | derived-ish — §7 |
| `violation` + `violation_ref` | what a scoring found, structured | derived |

---

## 2. One row per occurrence

A `timetable_entry` stores the **start** slot. A `duration_periods = 3` occurrence is **one row**,
and the three slots it occupies are derived by `evara_core::scheduling::span::resolve_span` — the
one implementation, shared by the evaluators, this crate's reader and Phase 7's grid.

The `DATA-MODEL.md` §7 sketch explained its key as existing *"because a multi-period activity
occupies several slots"*, and ADR 0029 §2 repeats that. **The key is kept and the reason is not
followed**, for two reasons ADR 0042 D1 records:

- Rows 2…`d` are **derived**, and storing derived data makes a corrupt timetable representable —
  three rows for a two-period lesson, and something must then decide which is true. It also makes
  a *partial* multi-period move representable in Phase 8, which is the state the sketch itself
  calls impossible: *"written and moved as a unit"*.
- `evara-db` depends on `evara-core` only, and the span resolver was `pub(crate)` in
  `evara-constraints`. Storing per slot would have needed either a second copy of the measure —
  `CLAUDE.md` §2's most important rule broken — or a public API wide enough to make that copy easy
  to write by accident.

`UNIQUE (timetable_id, activity_id)` says in the schema what the type already enforces: an
`Assignment` is one `Option<Placement>` per activity.

### The round trip is exact

Reading the entries back rebuilds an `Assignment` **identical** to the one the solver produced.
That holds *because* nothing derived is stored: there is no span to reconstruct and no row count
to reconcile against `duration_periods`.
`tests_timetables::the_arrangement_read_back_is_the_one_that_was_saved` asserts it through a
reopened file, and it is the single most valuable test in the milestone — if it holds, a stored
timetable cannot mean something other than what was generated.

---

## 3. Versioning: by rows, with no lifecycle

**A `timetable` row is one timetable. Several may coexist for one `(cycle, term)`, and that
coexistence is the versioning.** No version number, no `status`, no "current" pointer.

| Operation | Behaviour |
| --- | --- |
| **Save** | always **inserts**. Never overwrites, never asks |
| **Regenerate** | the same thing. The previous timetable is untouched and still openable |
| **Replace** | not an operation — save, then delete. Two decisions rather than one silent one |
| **Delete** | explicit, carries the timetable's revision, removes its children in one transaction |
| **Preserve** | the default |

A human tells two apart by `name`, and `UNIQUE (cycle_id, COALESCE(term_id, ''), name)` stops two
in one scope sharing one. The `COALESCE` matters: SQLite treats `NULL`s as distinct, so without it
the rule would silently fail to hold for exactly the **whole-year** timetables, which are the
common case. It is a `UNIQUE INDEX` rather than a table constraint because SQLite permits an
expression only in an index.

### What is deliberately not here

`status` with `DRAFT`/`PUBLISHED`/`ARCHIVED` is the published-archived lifecycle, which
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 lists as **not in the MVP**. `based_on_timetable_id` is
scenario lineage, which is what **repair** needs (`SOLVER.md` §7) — added by `V18` in M9B
([adr/0050](adr/0050-a-repair-produces-a-new-version.md)). Both are
one additive column later; a `CHECK` with three values of which only one is ever written would be
a column that lies about being used.

There is **no score on `timetable`**, because a score goes stale on the first manual edit. A score
belongs to the evaluation that produced it — §5.

---

## 4. A save refuses stale inputs

This is the part with no analogue in earlier milestones, and the reason it exists is that
**nothing else would catch the failure.**

A `SolveOutcome` is produced from a snapshot of the inputs and saved some time later. In between,
any of these may change:

- a teacher's availability gains an `UNAVAILABLE` rule covering a placed slot
- a room's `is_bookable` goes to 0, or it loses its campus association
- a course's `required_room_type` changes; an `activity.duration_periods` grows
- a `period`'s `starts_at` moves, changing which intervals overlap
- a group's `children_are_disjoint` is answered, turning `Indeterminate` into `Overlaps`

**None of them breaks a foreign key.** Every activity, timeslot and room the outcome names still
exists, so `RESTRICT` is satisfied, the save succeeds, and the file now holds a plausible, wrong
timetable.

### The mechanism: the change-log watermark

`change_log.seq` is `INTEGER PRIMARY KEY AUTOINCREMENT` and **every** tracked table writes to it
through its triggers, so the maximum is an **exact** statement of where a project stood — not a
sample, and not a hash over a chosen set of facts.

1. The loader records it on every `Assembly`, read **last** so it covers everything the load read.
2. `save_generated` compares it against the feed **inside the writing transaction** and refuses
   `DbError::InputsChanged`.

That is ADR 0030's shape — *"rebuilds the plan inside the transaction and refuses if the
fingerprint has moved"* — and it is **not check-then-act**: the comparison is a predicate on the
write, not a question asked beforehand. ADR 0030's other rule holds as well: **no caller compares
a watermark itself.**

### Safe by default

Only the four tables M7 writes are classified as *outputs*; everything else counts as an **input**.
So saving one timetable does not make a second pending outcome stale, and a table added by a
future migration and never classified is treated as an input — the check refuses where it need not
have.

The direction of that default is the decision. Refusing too often is a nuisance; accepting too
often is a wrong timetable.

### Why not a fingerprint over the scheduling facts

A fingerprint has to **enumerate** what a `Problem` depends on, and getting that list one table
short produces the worst outcome: a stale timetable reported as current. A watermark cannot be too
narrow. ADR 0042 C2.

### The limitation, named

An edit to a scheduling table the problem happens not to read still invalidates a pending outcome
— renaming a building, say. The remedy is to generate again, and the refusal says so. Narrowing
this later is a safe-direction change, because it can only be done with evidence.

---

## 5. Historical and current findings are different rows

A timetable has up to two scorings, and they answer different questions.

| Kind | Question | Lifetime |
| --- | --- | --- |
| `GENERATION` | *what did the solver find when it built this?* | **immutable** for the life of the timetable, and belongs to its `solve_run` |
| `REVALIDATION` | *what is wrong with this timetable now?* | replaced wholesale on each revalidation |

`record_revalidation` touches **only** the `REVALIDATION` row, so re-scoring a timetable can never
misrepresent how it was generated. Two partial unique indexes enforce at most one of each kind, and
a `CHECK` makes `solve_run_id` present exactly for a `GENERATION`.

Two snapshots of a changing world are not additive, which is why a revalidation **replaces** rather
than merges.

This also makes *"a stored score is a historical figure"* structurally true rather than a wording
convention: a score hangs off the evaluation that produced it, and any screen showing the
generation score is showing a figure that is labelled with when it was taken.

---

## 6. Revalidating a stored timetable

There is **no validity flag and no staleness fingerprint**. A stored boolean is a claim about the
world at the moment it was written, and every change in §4 would falsify it silently.

Revalidation is a *function*: load the problem for the timetable's `(cycle, term)`, read the
entries back into an `Assignment`, and run M5's `evaluate`. **No new scheduling semantics** — it is
the existing engine over an arrangement read from disk.

```
evara evaluate --project <p> --timetable <id>     # scores it. Writes nothing
evara save     --project <p> --revalidate <id>    # scores it and stores the current findings
```

A placement naming a lesson the problem no longer holds is **reported, not refused** —
`StoredArrangement::not_in_problem`. A timetable whose lessons were deleted is still worth
opening, and a reader that refused would leave the user unable to see what to fix.

---

## 7. What survives a restart, and what survives an export

| | Restart | Export (Phase 9) |
| --- | --- | --- |
| `timetable`, `timetable_entry` | ✅ stored | ✅ exported |
| `solve_run`, `timetable_evaluation` | ✅ stored | ✅ exported |
| `solve_run_unscheduled` | ✅ stored | ✅ **exported** |
| `violation`, `violation_ref` | ✅ stored | ❌ rebuilt by an evaluation |

The third row is the one worth explaining. An unplaced cause is **historical generation
information, not derived**: once the inputs move, nothing can reconstruct which lessons a past run
failed on or why. Violations *are* derived — re-running `evaluate` reproduces them exactly, which
is why `PROJECT-FORMAT.md` §3 has always called them *"rebuilt on first score"*.

Stored violations still earn their place in the database: they let a view render findings
**without** loading a `Problem` at all, which Phase 7's score health panel needs.

**Export is not implemented in M7.** The contract above is recorded in `PROJECT-FORMAT.md` §3 for
Phase 9.

### The unplaced cause, and the sentence that is not stored

`solve_run_unscheduled` stores `UnplacedCause::key()` and the pre-solve `DiagnosticCode` where M6
attached one. It does **not** store the engine's sentence: a wording improvement would leave old
rows reading the old way, and a cause survives a wording change where a string does not. The
section name a sentence needs is joined through `activity → class_section` rather than copied.

`UnscheduledRecord::is_impossible()` reads the stored key through `UnplacedCause` rather than
storing a second flag — and an **unrecognised** key, written by a newer build, is deliberately
*not* reported as impossible, because overclaiming is the failure that distinction exists to
prevent.

---

## 8. Identity, keys and concurrency

**No `school_id` on `timetable`.** The school is `cycle.school_id`: *"a child's school is resolved
through its parent chain, never by adding it a second `school_id` column"* (`CLAUDE.md` §3). The
cost is a `cycle` join on every school-scoped read, which `V15`'s lesson chain already pays, and
the payoff is that the ownership has exactly one home.

Two trigger pairs hold what the foreign keys cannot:

- a timetable's `term` must belong to its `cycle`'s school — ADR 0032 rule 1 scopes a timetable to
  **one** school, and nothing else would refuse a row naming Northgate's cycle and Southmoor's term
- an entry's `timeslot` must belong to the timetable's own cycle

**Every child edge is `RESTRICT`, and the repository deletes the rows itself.** ADR 0040 §2: SQLite
fires a `DELETE` trigger for a foreign-key action only under `recursive_triggers`, which this
project never enables, so a `CASCADE` would remove rows **with no change-log entry** — silently,
and only for the rows the cascade reached. `timetable_entry.timeslot_id` is `RESTRICT` for a second
reason as well: ADR 0011 §5 says so by name.

The cost is measured and accepted: ≈2 000 `change_log` rows per 2 000-entry timetable written, and
the same again per one deleted — about 200 KB each way.

### Revisions

| Mutation | Contract |
| --- | --- |
| `save_generated` | no revision: there is no previous version of a row that does not exist. Guarded by the **watermark**, which is a precondition on the whole problem |
| `rename` | `expectedRev` of the timetable |
| `delete` | `expectedRev` of the timetable; the children at `Expected::Any` — ADR 0027 §4's cascade-inside-a-transaction |
| `record_revalidation` | `expectedRev` of the timetable, which it **consumes**: the timetable's current findings are part of what it says about itself |

One implementation note, because a test caught it: `delete` first checked the revision with a no-op
`update`. That **consumes** the revision it was meant to check and then fails its own predicate.
The parent's own delete carries the revision instead, last, and the transaction rolls the children
back if it does not match — the same guarantee without a second write.

### One transaction

A save writes the `timetable`, N `timetable_entry`, the `solve_run`, N `solve_run_unscheduled`, the
`GENERATION` evaluation and its violations — **all or nothing**. A failure on the last violation
leaves no timetable, which is why the writer is in `evara-db` rather than composed from public
repository methods (ADR 0030's argument).

---

## 9. What the writer refuses

| Refusal | When |
| --- | --- |
| `NotSaveable` | `Termination::InputRejected` — nothing was attempted, so there is no timetable; the findings belong to the report |
| `NotSaveable` | `Termination::InternalCheckFailed` — the reference scorer rejected what the engine built |
| `InputsChanged` | a scheduling input moved since the problem was compiled — §4 |
| `Invalid` | the outcome's `ProblemIdentity` does not match the assembly it is offered with |
| `Invalid` | the outcome covers a different number of occurrences than the problem holds |
| `CorruptIdentifier` | a dense index resolves to no stored row — the reverse of `Problem::integrity` |
| `DuplicateValue` | a timetable of that name already exists in the scope |
| `ReadOnlyProject` | the project was opened for inspection, refused before a transaction opens |

The identity and length checks exist because of the **shape** of the failure they prevent: a
`SolveOutcome` written against a *different* assembly's index would translate every dense index to
the wrong identifier and store a complete, plausible, wrong timetable.

**Nothing here evaluates a constraint.** The solver refused every hard-violating placement and
`generate` re-checked with the reference scorer; a third opinion in a crate forbidden to depend on
`evara-constraints` would be a fourth caller of the rules. What the writer checks is the
*translation*.

---

## 10. Running it

```
evara save --project <p> --school <id> --cycle <id> [--term <id>]
           [--name TEXT] [--seed N] [--json]
           [--max-candidates N] [--max-steps N] [--max-passes N]
evara save --project <p> --revalidate <timetable-id> [--json]
```

**`evara generate` stays read-only** and its byte-comparison test is unchanged. Writing lives in a
command named for writing: a `--save` flag would make `generate`'s documentation true for some
invocations and false for others, which is the worst state for a guarantee — a user reading the
help text could not tell whether the command they are about to run touches their file.

The asymmetry goes deeper than naming. `generate` opens the project itself through
`Database::open_for_inspection`; `save` must go through `Workspace`, because every change-log
trigger calls `evara_site_id()` and that identity lives in the **settings** database
(`CLAUDE.md`). A command that opened the project directly would have no site id to offer and every
insert would be refused.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | saved, and every occurrence is placed (or a revalidation was recorded) |
| 1 | saved, and the timetable is **partial** — usable, with lessons unplaced |
| 2 | the command itself failed, or the project refused the write |
| 3 | nothing was saved: generation was refused, or produced no timetable |

Code 3 rather than 2 for the last row, because the command worked and the answer was *"there is no
timetable yet"* — which a script needs to tell from *"the command is broken"*.

---

## 11. Tests

| Where | Count | Covers |
| --- | --- | --- |
| `evara-core::scheduling::span` | 10 | the promoted resolver: spans, the four defects, the two unresolvable cases |
| `evara-db::tests_timetables` | 31 | the round trip, versioning, freshness, refusals, referential refusals, school isolation, revalidation, transaction boundary, migration |
| `evara-cli::save` | 16 | the grammar, the four exit codes, the help text's promises, the end-to-end write |
| `evara-cli::evaluate` | +3 | scoring a stored timetable, read-only |

The promotion in D2 is proved behaviour-neutral by what did **not** change: all 69
`evara-constraints` tests passed unedited.

Named assertions worth knowing:

- `the_arrangement_read_back_is_the_one_that_was_saved` — §2.
- `an_input_that_changed_after_generation_refuses_the_save` — and that nothing was written.
- `saving_one_timetable_does_not_make_another_outcome_stale` — the output-table exclusion.
- `a_second_generation_never_overwrites_the_first` — the previous entries are byte-identical after.
- `a_revalidation_adds_a_current_scoring_and_leaves_the_generation_one_alone`, and the second
  revalidation replacing only the first.
- `deleting_a_timetable_removes_its_children_and_writes_each_one` — counted in the change feed,
  which is what a `CASCADE` would have skipped.
- `a_v16_project_upgrades_to_v17_with_the_new_tables_empty` — the migration, by rewinding a real
  project and reopening it.

---

## 12. Known limitations

| Limitation | Why | Where it goes |
| --- | --- | --- |
| ~~**No manual editing.** `is_locked` and `is_manual` are columns the generator writes as 0~~ | ✅ **done in M9A**, with **no migration** — `V17` already held both columns, both uniqueness rules, the cross-cycle trigger, all three indexes and the change-log triggers. An edit carries an `ArrangementStamp` beside its `expectedRev`, because a second window moving a *different* entry passes a revision check ([adr/0047](adr/0047-a-cross-session-guard-is-an-exact-placement-list.md)) | [TIMETABLE-EDITING.md](TIMETABLE-EDITING.md) |
| **No published/archived lifecycle** | out of the MVP (§6 of the M1 record) | a later `status` column |
| ~~**No scenario lineage.** `based_on_timetable_id` is not in `V17`~~ | **Paid by M9B.** `V18` adds it, `ON DELETE RESTRICT`, with a typed `DbError::HasDescendants` refusal rather than an opaque foreign-key failure | [adr/0050](adr/0050-a-repair-produces-a-new-version.md) |
| **No export.** The contract is recorded, not built | Phase 9 | `PROJECT-FORMAT.md` §3 |
| **A watermark refuses on an unrelated edit** | safe-by-default classification — §4 | narrowable later, with evidence |
| **No "current timetable" pointer.** The UI lists by `generated_at` | that is a selection a screen remembers — and since M8 the Generate screen remembers it by opening what it just saved | settled; no pointer was added |
| **Revalidation keeps one snapshot, not a history** | a history of scorings grows without bound and nothing asks for one | if a reason appears |
| ~~**No Tauri command.** Nothing in the interface saves a timetable~~ | ✅ **done in M8** | [TIMETABLE-VIEWS.md](TIMETABLE-VIEWS.md) |

---

## 13. The M8 orchestration contract — ✅ discharged

**Supplied by M7.**

- `Workspace::timetables()` — the writing path, with the installation identity already attached.
- `save_generated`, `for_school`, `get`, `entries`, `arrangement`, `run_for`, `unscheduled_for`,
  `evaluations`, `violations_for`, `record_revalidation`, `rename`, `delete`.
- Every refusal in §9 as a typed `DbError`, so a command handler maps rather than interprets.
- `StoredArrangement`, which turns stored rows back into an `Assignment` the constraint engine
  takes unchanged.

**Owed by M8 — all four paid.**

1. ✅ **Tauri command handlers.** `commands::solve` (seven) and `commands::views` (six).
   `rename`, `delete` and `revalidate` take `expectedRev`; `save` takes none, because there is no
   previous version of a row that does not exist and the watermark is its precondition.
   **No handler constructs `Expected::Any`** — ADR 0027 §2.
2. ✅ **A background thread and `solver://progress`.** The command thread compiles the `Assembly`
   under the workspace lock, releases it, and **moves** it to a dedicated worker; a
   `send_assertions` test in `evara_db::scheduling` keeps that possible as the types change.
   Progress coalesces to one event per 50 ms plus the final one, so 2 000 placements are not
   2 000 events — and `elapsed_ms` is the **caller's**, because the engine still reads no clock.
3. ✅ **What the Generate screen does after a save: it opens the new timetable.** The user asked
   for a timetable, and a screen that answered with a list entry would make them click again for
   what they just made. **No "current timetable" pointer was introduced** — §12's question is
   settled by a screen remembering a selection, which is what it said the answer would be.
4. ✅ **The `InputsChanged` message.** Its own `SetupError` kind, deliberately not
   `ReviewRequired`: that one means *re-read the plan and confirm it*, and this one means
   **generate again** — there is no plan to re-read. The screen renders the remedy as a button.

One thing M8 found that this record did not anticipate, and it is worth stating because the
failure is silent: a solve takes about a second, and in that second a user can close the campus
file and open another. The session's `Assembly` then describes a database that is no longer open.
`save_generated`'s identity check would catch it, but only after a transaction had opened and
only if the identifiers happened not to collide — so M8 added a **project epoch**, bumped on
every create, open and close, and refuses the save earlier with the same *generate again* remedy.
