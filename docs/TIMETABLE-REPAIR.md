# Timetable repair and partial regeneration

What M9B shipped: how a repair chooses what may move, how it proves it moved nothing else, and
why it writes a new version rather than changing the one you repaired.

ROADMAP **Phase 8B**. The architecture record is
[M9B-REPAIR-PROPOSAL.md](M9B-REPAIR-PROPOSAL.md); the decisions are
[adr/0048](adr/0048-repair-scope-is-a-movable-set.md) through
[adr/0052](adr/0052-an-unexpressible-lock-blocks-a-repair.md).

---

## 1. The one-paragraph version

A repair is the **existing construction solver** run over a baseline arrangement with a movable
set. Out-of-scope and locked placements are *frozen*: present in the arrangement, visible to
every constraint, never candidates. In-scope occurrences are re-placed with their current
placement **tried first**, so an unchanged placement wins whenever it is still legal. The run
writes nothing; it produces a diff the user approves, and approval **inserts a new timetable**
that preserves locks, manual flags and provenance. One migration, `V18`, additive. No new
constraint type, and no local search.

## 2. The scope says what may move, not what is checked

> The scope limits what may **move**. It does not limit what is **checked**.

That sentence is the whole mechanism. A `teacher-clash` between an in-scope and an out-of-scope
occurrence is caught with no special handling, because the out-of-scope placement is in the
arrangement the scorer holds. **No constraint knows the scope exists.**

### Six scopes, and the split that is user-visible

| Scope | Derived from | Reaches an unplaced lesson? |
| --- | --- | --- |
| Whole timetable | — | yes |
| Class section | `Problem::sections` — the **inputs** | yes |
| Teacher | `SectionFact::teachers` — the **inputs** | yes |
| Room | the **baseline** — what is currently in that room | **no** |
| Cycle day | the **baseline** — `timeslot.cycle_day` of the current placement | **no** |
| Selected lessons | the user's selection | yes |

A room or a day is a *position*, and repair changes positions. So an occurrence that is
**unplaced** is in no room and on no day, and neither positional scope can reach it. The screen
says so **before** the run, with the count and the two scopes that could, rather than letting it
be discovered in the diff.

Only the whole-timetable scope is closed under the constraints. For every other one, fixing an
in-scope occurrence may require moving an out-of-scope one — and the repair reports the in-scope
occurrence as unplaced rather than reaching outside. It is **never widened automatically**: a
wider scope is a separate repair, with its own baseline, preview and approval, because a lesson
moved outside the chosen scope is a change the user had no opportunity to review.

### The movable set is derived once

From the baseline, before the solve. Re-deriving it as the run proceeded would make the scope
depend on the search order, which is a different repair for the same request — an occurrence
repaired out of Monday leaves the Monday scope, one repaired into Monday was never in it.

`MovableSet::new` validates it once: empty, out-of-range and duplicated requests are refused
rather than collapsed, because a duplicate means whatever derived the set is wrong. An explicit
selection naming one lesson twice is the single tolerated case — a UI artefact, not a
derivation bug.

## 3. The working arrangement holds the frozen placements alone

Not the baseline. `frozen_only` clones the baseline and **clears** the movable set, so every
movable occurrence is unplaced when the walk begins.

Starting from the full baseline is wrong three ways, and all three are properties of the
existing APIs:

1. **Stale positions.** Each movable occurrence would be reconsidered against where the others
   still are, so it would avoid a slot that is about to be vacated.
2. **A movable-to-movable clash refuses both.** `PlacementVerdict::allowed` is false whenever the
   result *carries* a hard violation naming the occurrence — the stricter of the two readings,
   deliberately (`incremental.rs:80-90`). Two already-clashing lessons are each, from the other's
   point of view, an immovable obstacle.
3. **`place` does not withdraw first.** It is `probe` that withdraws and restores. So movable
   occurrences left in place would each need explicit withdrawal, with everything not yet
   reached still in the way.

Frozen-only initialisation dissolves all three, and answers the self-clash question
structurally: a movable occurrence is *unplaced*, so `probe`'s withdraw is a no-op and nothing
can clash with itself. Every probe a repair makes is a probe of an unplaced occurrence — M6's
only kind — so the carries-versus-adds distinction never arises.

**Subtraction, not construction.** Cloning and clearing makes "every frozen placement survived"
depend on the *movable* derivation, which has already been validated; building up from an empty
assignment would make it depend on the frozen derivation, which nothing checks. It also inherits
the baseline's dimension.

## 4. The baseline is tried first

For each in-scope occurrence with a baseline placement, that placement is probed **before** the
seeded walk. Because the walk is first-feasible, an occurrence whose placement is still legal
does not move. Nine lines, no score change, no new rule, fully deterministic — the baseline is
data, not a random choice.

Measured: a clean whole-timetable repair of 1 600 occurrences spends **exactly 1 600 candidate
evaluations** — one baseline probe each, and the walk never runs — and reproduces the baseline
byte-for-byte.

**No minimal-disruption guarantee**, and none is claimed. Once an occurrence is displaced, the
ones placed after it see a different timetable and greedy construction cannot reconsider.
`minimise-disruption` (`OPS-001`) waits for local search —
[adr/0051](adr/0051-baseline-preference-is-candidate-ordering.md) has the four reasons.

The baseline probe is **not** budget-checked, matching the pinned branch it is modelled on: it
is the one candidate the caller already chose, and refusing it for budget would displace a lesson
that was legally where it already was. The cost is one evaluation per in-scope occurrence, which
is why a repair's statistics are not comparable with a generation's at the same seed.

## 5. The verification, and the defect it exists to avoid

A repair does **not** go through `status_of`. That function's first line gates on
`report.score.is_feasible()` — `hard == 0` over the **whole arrangement** — which is false for
every timetable a repair exists to fix. Routing a repair through it would have reported each one
as `InternalCheckFailed`, never applyable: correct work, a better arrangement, discarded as our
own defect.

In its place, five checks. Any failing is an internal consistency failure that is never
applyable:

| | Check | Catches |
| --- | --- | --- |
| **V0** | `frozen_differences` over the **complete** assignments — every frozen occurrence, slot *and* room, and the unplaced state, in both directions | A frozen placement moved, dropped or invented |
| **V0b** | `locked_differences`, measured against the pin | A lock not honoured |
| **V1** | `!report.is_readable()` | An arrangement that does not describe the problem |
| **V2** | a HARD violation whose `activities` intersects the movable set | A real incremental-versus-reference disagreement |
| **V3** | the frozen-only HARD set differs from the baseline's, by `ViolationKey` | A frozen placement acquiring or losing a conflict |

**V0 is primary and is not derived from V3.** Two arrangements can carry identical violation sets
and still differ: a frozen swap, or a move to an equally-clashing slot, leaves every
`ViolationKey` the same. `a_swap_of_two_frozen_occurrences_is_caught` demonstrates it.

**V0 runs twice** — once in the solver, once inside `save_repaired`'s transaction. An outcome
that passed in memory and is then persisted from a different arrangement would write a timetable
nobody checked. One implementation, in `evara_core::scheduling::repair`, because `evara-db` may
not depend on `evara-solver`.

**V2 and V3 together are stronger than `is_feasible()`**, not weaker: the global check cannot
distinguish a disturbed frozen placement from pre-existing damage, and V3 catches the former
while tolerating the latter. A repair verifies **more** than a generation does.

### Candidate acceptance needed no change

`probe` sets `blocking` from `involving(activity)`, which runs each rule's `evaluate_involving`
for the probed occurrence only. A violation between two frozen occurrences names neither the
probed occurrence nor its candidate, so it never appears in `blocking` and cannot refuse
anything. `evaluate_involving` — mandatory since M6 — *is* the scope-aware acceptance policy.

## 6. The four attributions

A projection in `evara-report`, over two reference reports. Set membership, never a
re-judgement:

| Bucket | Definition | Shown as |
| --- | --- | --- |
| **Eliminated** | key ∈ baseline only | *"Fixed by this repair"* — the headline figure |
| **Pre-existing, frozen** | key ∈ both, naming no movable occurrence | *"Already present, and outside the scope you chose"* — not the repair's doing, and **not** a bar to applying |
| **New, involving a movable occurrence** | key ∈ final only, naming one. **Must be empty at HARD** (V2) | At MEDIUM or SOFT: a reported cost of the repair |
| **Not fixable within this scope** | the pre-existing frozen set, plus in-scope occurrences left unplaced | *"This scope could not fix these"*, with the note that the scope may be the reason |

The preview reports the **whole** arrangement's score and hard-violation counts, with the buckets
beside them rather than instead of them. A preview showing only in-scope violations would be
describing a timetable the user is not about to save.

### The five change categories

`Unchanged`, `Moved`, `NewlyScheduled`, `Unscheduled`, `StillUnplaced` — exhaustive and disjoint
over every occurrence in the problem, asserted by `RepairDiff::is_well_formed` over every
fixture. `MovedParts` is `Option`, present only for a `Moved` occurrence, so "the slot changed"
cannot be said of a lesson that did not move.

`StillUnplaced` carries `in_scope`, because the two kinds have opposite remedies: one the repair
tried and could not place (look at the clash), one the scope never let it touch (choose a wider
scope).

## 7. The pre-flight

Three classes, all before any solving. A frozen-only reference evaluation is read at **HARD
only** — a frozen-only arrangement leaves every in-scope occurrence unplaced, so MEDIUM
`activity-scheduled` fires once per movable occurrence, and reporting those would show one
finding per lesson and call a healthy timetable catastrophic.

| Class | Blocking? |
| --- | --- |
| **Unexpressible** — a lock the compiled problem cannot represent | **yes** |
| **Contradictory** — a HARD violation naming two locked occurrences | **yes** |
| **Impossible in context** — a HARD violation naming one locked occurrence | **yes** |
| **Pre-existing** — a HARD violation naming no locked occurrence | **no**, reported |

**Only a lock blocks.** A HARD violation among unlocked frozen placements is pre-existing damage
the chosen scope excludes; refusing it would refuse repair on exactly the timetables repair
exists for.

**An unexpressible lock blocks**, which is the one place M9B is stricter than M9A's editor. The
lock is absent from `Problem::pinned`, so nothing freezes it — and in the timeslot and room cases
the *activity* resolves, so the occurrence is an ordinary problem member that a scope containing
it would **move**. Each cause names its own remedy, and nothing in the repair path ever writes or
clears a lock. [adr/0052](adr/0052-an-unexpressible-lock-blocks-a-repair.md).

## 8. The session, and its six hazards

`commands::repair_session` copies the generation session rather than extending it: the same shape
with `Saving`/`Saved` renamed to `Applying`/`Applied`, because a repair carries a baseline, a
movable set, an arrangement stamp and a source revision a generation has no field for.

```text
                   ┌──────────────────── discard ───────────────────┐
                   v                                                │
  start ──► Running ──► Finished ──► Applying ──► Applied           │
                           ^            │                          │
                           └─ failed ───┘   (the write is retryable)
```

| Hazard | Guard |
| --- | --- |
| Applying an unfinished repair | `claim_apply` refuses from `Running` |
| Applying a cancelled, refused or defective repair | `RepairOutcome::applyable`, computed by the solver — never re-derived here |
| Two applies from two clicks | compare-and-swap into `Applying` **before** any database work |
| A cancel racing a finish | `request_cancel` sets an `AtomicBool` and **no state**; the worker is the only thing that leaves `Running` |
| Applying into a different project | `ProjectEpoch`, before a transaction opens |
| **Applying over an arrangement since edited** | `ArrangementStamp`, **inside** the writing transaction. New in M9B — a generation has no baseline to be stale about |

`Applied` and `Discarded` are terminal, and nothing leaves `Applied`.

**`Partial` is applyable.** A repair of a timetable with out-of-scope unplaced lessons is
*always* partial, so refusing partial would refuse almost every real repair. A budget-limited run
is applyable too, with the limit named — presenting a limit as a conclusion is the dishonest
alternative.

## 9. One read, three guards, one insert

**`repair_baseline` reads the entry rows once** and derives the arrangement, the pins, the stamp,
the flags and the source revision from that single read. Composing it from the four existing
accessors would open four transactions and read the entries three times, so the stamp could end
up guarding an arrangement the repair never used — the exact race ADR 0047 exists to close,
reintroduced by how its inputs are gathered.

**Order matters in one direction.** The `Assembly` is compiled **first**, capturing its
watermark; the baseline snapshot is read **after**. An input that moves in between is then older
than the watermark and the save refuses it. The reverse order would leave such a change invisible
to *both* guards, because `timetable_entry` is in `SOLVE_OUTPUT_TABLES` and the stamp only
watches entries.

**`save_repaired` opens with three preconditions**, before any insert:

1. `guard_freshness` on the input watermark — ADR 0042's, reused unchanged.
2. The `ArrangementStamp` — ADR 0047's, reused unchanged.
3. The source timetable's revision.

The third is labelled a **snapshot assertion**, not an ADR 0027 predicate, and the distinction is
load-bearing: a repair never updates the source row, so there is no statement whose `WHERE`
clause could carry `rev = ?`. It is sound for the reason `guard_freshness` and `read_stamp` are —
one owned `Connection`, `write` takes `&mut self`, and SQLite refuses to commit a write upgraded
from a stale read. Calling it a predicate would invite the pattern being copied somewhere it
genuinely would be check-then-act.

## 10. `V18`, and the two things it deliberately does not do

```sql
ALTER TABLE timetable ADD COLUMN based_on_timetable_id TEXT
    REFERENCES timetable (id) ON DELETE RESTRICT;
CREATE INDEX idx_timetable_based_on ON timetable (based_on_timetable_id);
ALTER TABLE solve_run ADD COLUMN kind TEXT NOT NULL DEFAULT 'GENERATION'
    CHECK (kind IN ('GENERATION', 'REPAIR'));
ALTER TABLE solve_run ADD COLUMN scope TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(scope) AND json_type(scope) = 'object');
```

Three columns, one index, nothing rebuilt, defaults that make every existing row correct with no
data migration. ADR 0020 is not engaged — and the two omissions below are what keep that true.

**`timetable_evaluation.kind` is not widened.** Its `CHECK` admits only
`('GENERATION', 'REVALIDATION')`, a `CHECK` cannot be altered in SQLite, and widening it would
mean a twelve-step rebuild on a `STRICT` table. A repair writes **`GENERATION`**, because that
*is* the new version's original immutable scoring; `solve_run.kind` is the discriminator.

**No eighth `UnplacedCause`.** A repair's `solve_run_unscheduled` rows cover the **movable set
only** — what this run attempted. A frozen occurrence that was already unplaced appears in the
preview as still-unplaced and is absent from the table, because the run never tried it.

**Lineage is `RESTRICT` with a typed pre-check.** `delete` counts repaired descendants first and
refuses `DbError::HasDescendants { timetables }`. The foreign key would refuse it anyway, but on
the final statement and as an opaque driver failure — a useless message for a legitimate
refusal. The key stays as the backstop.

**`duplicate` copies the lineage** rather than pointing at the row it copies: a duplicate of a
repair descends from whatever that repair was made from.

## 11. What the interface is held to

- The scope is **chosen, never defaulted**. Repair is unavailable until the user picks one.
- A **real** `role="radiogroup"` with arrow-key navigation and one tab stop, styled off
  `input:checked` so a keyboard move takes the highlight with it.
- Both warnings **before** the run: lessons outside the scope will not move, and a positional
  scope cannot schedule unscheduled lessons.
- **"This repair attempt made no changes."** Never *"nothing could be improved"* — a greedy
  construction that changed nothing has discovered that its own walk found the baseline legal,
  not that no better arrangement exists. The same discipline `UnplacedCause::is_impossible`
  enforces per occurrence, applied to a whole run.
- A limit is **named as a limit**, not presented as a conclusion.
- Never *"conflict-free"*: `UsabilityLine`'s existing wording, with the unverified counts beside
  it (ADR 0032 rule 4).
- A scope violation is reported as **our defect** — *"our bug to fix, not your data to
  correct"* — because the alternative is a user editing their own timetable in response to our
  bug.
- Every count is text before it is a colour; the change column carries its word as well as its
  tint; the score and change lists are real tables with row and column scopes; progress is a
  live region; wide tables scroll inside their own container.

## 12. What it cost

Measured by `crates/evara-solver/src/repair_budget.rs`, which prints its figures rather than
asserting them tightly — a tight timing assertion on shared CI hardware is a flaky test.

| Measurement | Debug | Release |
| --- | --- | --- |
| Clean whole-timetable repair, 1 600 occurrences (200 sections × 8) | **126 ms** | **90 ms** |
| Whole timetable, 400 occurrences | 17.7 ms | 10.3 ms |
| One occurrence of 400 — a narrow scope | **2.5 ms** | **1.6 ms** |
| Conflicted baseline, 500 occurrences, 3 280 violations, all resolved | **49 ms** | **37 ms** |
| Ten of 500 with the rest conflicted — applyable | 46 ms | 39 ms |

All well inside the proposal's §6.2 budgets. A narrow scope is **cheaper** than a wide one
rather than merely smaller, because the frozen set is placed into the scorer rather than
searched.

### The one slow shape, and why it is not the repair's

A fixture of **1 600 single-occurrence sections** takes ~194 s in debug. Attributed:

```text
reference evaluate      3.2 ms
build the scorer        3.5 ms
one probe              14.4 µs   (×1600 = 23 ms)
a whole GENERATION    193.99 s
```

The repair cost 194.25 s on the same problem, so it adds nothing: the time is M6's `prepare`,
whose `conflict_degrees` is documented as **quadratic in the sections** — *"a 500-occurrence
school has far fewer sections, and every occurrence of a section has the same degree"*. 1 600
sections of one occurrence is its worst case and no real school's shape, which is why the budget
test uses 200 sections of eight.

`where_the_time_goes_at_1600_occurrences` keeps the evidence and is `#[ignore]`d, because a
three-minute test in the ordinary suite is paid by everyone on every run.

## 13. Limitations

| Limitation | Why |
| --- | --- |
| **No minimal-disruption guarantee** | By design. Greedy construction cannot reconsider a displacement; real minimisation is local search, which the MVP excludes |
| **A positional scope does not close under the constraints** | Repairing "Monday" may fail where repairing everything would succeed. Reported with the reason, never worked around by widening |
| **A heavily conflicted baseline is slower**, and the cost is allocation | `Violation` formats its `detail` eagerly. Making it lazy is M5's surface, not M9B's |
| **A stale baseline is refused, not merged** | ADR 0047's position one level up. Merging two arrangements is unspecified |
| **An unexpressible lock must be resolved first** | ADR 0052. The cost of the guarantee, mitigated by naming each remedy |
| **A repair's statistics are not comparable with a generation's** | The baseline probe spends one evaluation per in-scope occurrence. Recorded on `solve_run.kind` so a reader can tell them apart |
| **No GUI smoke test** | As M9A. The screen is covered by 23 component tests; nothing drives the built binary |
| **Cross-school coordinated repair** | ADR 0032 defers cross-school comparison for generation too |
