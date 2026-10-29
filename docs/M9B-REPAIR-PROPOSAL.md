# M9B — Automated Timetable Repair and Partial Regeneration · Architecture proposal

- Status: **Proposed.** Nothing below is implemented. No code was written for this document.
- Branch: `feat/m9b-timetable-repair`, from `334a21d` (the PR #33 merge) — verified to contain
  `7c03b2d`, M9A's commit.
- Audited against the tree at that commit. Every line number and signature quoted here was read,
  not recalled.
- Phase: ROADMAP **Phase 8B**. Phase 8A shipped as M9A — [TIMETABLE-EDITING.md](TIMETABLE-EDITING.md).
  ROADMAP Phase 8 records that 8A *"Closes M1-13"* and that **M1-14 waits on 8B's repair** — so
  this milestone is what closes M1-14.
- **Re-examined against the tree on 2026-10-25** — §0.1.
- **Conditionally approved, and corrected on three correctness questions** — §0.2 and Part I. The
  architecture is unchanged; one of the three was a blocking defect that would have made repair
  inoperable in its primary use case.

> **Placed here, not in `docs/prompt-runs/`.** The precedent is
> [M7-PERSISTENCE-PROPOSAL.md](M7-PERSISTENCE-PROPOSAL.md) and
> [M8-VIEWING-PROPOSAL.md](M8-VIEWING-PROPOSAL.md), both of which live in the repository because
> a milestone's design record is referenced by the ADRs it earns. It also survives: M9A's own
> proposal was written to the gitignored folder and no longer exists, which is why §1.6's
> decision log below had to be reconstructed from the ADRs rather than read.

---

## 0. The one-paragraph summary

Repair is the existing construction solver run over a **baseline arrangement** with a **movable
set**. Out-of-scope and locked placements are *frozen*: present in the arrangement, visible to
every constraint, and never candidates. In-scope occurrences are re-placed with their **current
placement tried first**, so an unchanged placement wins whenever it is still legal. The run
writes nothing; it produces a diff the user approves, and approval writes a **new timetable
version** that preserves locks, manual flags and provenance. One migration (`V18`) and no new
constraint type.

The three things this proposal refuses to do are as important as the three it does: it does not
pin out-of-scope placements (§2.1 — that drops them), it does not add `minimise-disruption`
(§2.3 — the greedy solver cannot minimise, and the score would stop being comparable), and it
does not apply a repair in place (§2.4 — a new version makes the stale-session guarantee
structural rather than guarded).

---

## 0.1 Re-examination of 2026-10-25

This document was re-read against the working tree and every load-bearing claim re-verified from
source rather than from this text. **Thirteen of the fourteen audit findings reconfirmed exactly**,
including the two the architecture rests on: `start` is still accepted and discarded
(`evara-solver/src/lib.rs:142`) and the pinned branch still returns `Attempt::Refused` on a failed
probe (`construct.rs:269-277`), so **A.2's trap is real and D1 stands**. The six decisions below
are unchanged in substance. What the re-examination changed:

| # | Where | Change | Severity |
| --- | --- | --- | --- |
| 1 | **A.13** | **Factual error corrected.** `UnplacedCause` has **seven** variants, not six, and `is_impossible` is false for **three** of them — `BlockedByPlacements`, `NotReached` and `InputRejected`. V17's own `solve_run_unscheduled.cause` CHECK lists all seven and its comment draws the four/three line explicitly. No conclusion changes | fact |
| 2 | **§2.2 / D2** | **Refusal criterion narrowed — the one substantive correction.** The original refused a repair on *any* HARD violation in the frozen set. That refuses repair on exactly the timetables repair exists for. It now refuses only on a HARD violation **involving a locked occurrence** | **defect** |
| 3 | **§2.2 / D2** | **Gap closed.** A frozen-only evaluation leaves every in-scope occurrence unplaced, so MEDIUM `activity-scheduled` fires once per movable occurrence. The pre-flight must filter by level, and the levels are verified: `ActivityScheduled` is `Medium`, `ActivityFixed` is `Hard` | gap |
| 4 | **§3.2 / V18** | **Gap closed.** `timetable_evaluation.kind`'s CHECK admits only `('GENERATION', 'REVALIDATION')`. A repair writes `GENERATION`; widening that CHECK would be a table rebuild on a `STRICT` table and *would* engage ADR 0020, contrary to §3.2's claim | gap |
| 5 | **§8.2** | **Understated.** `SOLVER.md` §7's *scope list* also changes, not only its two mechanisms | scope |
| 6 | **§2.3** | **Note added** on `max_candidate_evaluations`, which the original snippet silently skipped | precision |
| 7 | **§2.7 / D7** | **Strengthened** by a fact the original argued without: `timetable_entry` is in `SOLVE_OUTPUT_TABLES`, so `guard_freshness` *provably cannot* see a baseline edit | evidence |
| 8 | **Appendix** | *"working tree clean"* was true of the code and false of this file, which is untracked — which is why it appeared to have been lost | record |

## 0.2 Corrections round 2 — the three correctness questions

The architecture is **conditionally approved** as it stands: movable set, frozen out-of-scope and
locked placements, baseline-first ordering, a new version per repair, explicit preview and
approval, an additive `V18`, the three guards, and no local search or `minimise-disruption` in
M9B. All of that survives this round unchanged.

Three correctness questions were put to it. **All three are real**, and one is fatal as written:

| | Question | Verdict | Where |
| --- | --- | --- | --- |
| **C1** | Does the algorithm initialise the scorer with movable occurrences not yet reconsidered? | **Real, and the original text did not say otherwise.** The working assignment must hold the frozen placements *only*. Also uncovered: `construct`'s closing invariant does not hold for a repair, because a frozen occurrence can be **unplaced** in the baseline | §2.8 |
| **C2** | Does a pre-existing HARD violation among frozen, unlocked occurrences refuse every subsequent candidate? | **`probe`: no. `generate_with`: yes — and that is a blocking defect.** `probe` is correctly scoped to `involving(activity)`, so candidate acceptance is safe. But `status_of` gates on `report.score.is_feasible()`, a **whole-arrangement** property, so any pre-existing hard violation anywhere in the frozen set would return `SolveStatus::Failed` / `Termination::InternalCheckFailed` — which §2.5 makes **not applyable**. Every repair of a timetable that has a hard violation outside the scope would be reported as a defect in Evara and refused. Repair exists for exactly those timetables | §2.9 |
| **C3** | Can an unexpressible lock be preserved exactly? | **No — the data model proves the opposite.** `pins` drops it from `Problem::pinned` and the projection cannot write it back, so a warning would let a lock be silently discarded, or worse *moved*. It becomes **blocking** | §2.2 |

Nothing in this round came from re-reading this document. Each verdict is from the source named
beside it, and Part I is the decision log.

Also newly confirmed in support of the existing text, each from source: the V18 DDL is legal on a
`STRICT` table and its JSON CHECK follows the `constraint_instance.params` / `solve_run.settings`
precedent verbatim; V18 needs **no** change-log work, because `timetable` and `solve_run` are
already tracked and the triggers *"interpolate only the table name, `id` and `rev`"*
(`change_log.rs`); `IncrementalScorer::of`'s own doc comment already names `SOLVER.md` §7 repair as
its reason for existing; and `TIMETABLE-PERSISTENCE.md:372` names the missing
`based_on_timetable_id` with the reason *"repair is what needs it"*.

---

# Part A — Existing-code audit

Fourteen integration points, each confirmed by reading the code at `334a21d` and re-verified on
2026-10-25 (§0.1).

## A.1 `start` is declared, accepted and discarded

`crates/evara-solver/src/lib.rs:133-142`:

```rust
fn solve(&self, problem: &Problem, start: Option<&Assignment>, …) -> Solution {
    // A starting arrangement is the repair entry point. Nothing in M6 passes one …
    let _ = start;
    let finished = construct::construct(problem, constraints, config, cancel, progress);
```

`construct()` (`construct.rs:99-107`) takes **no** starting arrangement and opens with
`IncrementalScorer::empty(problem, constraints)`. So the parameter is a reserved seam, not a
working feature, and the doc comment's claim that the construction *"places what is missing and
leaves what is there"* is **not true today** — it cannot be, because `construct` never sees
`start`.

**Consequence.** M9B's smallest change is to make that comment true: `construct` must accept a
starting arrangement and a movable set. This is the only change to the solver's algorithm in
this proposal.

## A.2 The pinned branch drops an impossible pin — correct for locks, fatal for scope

`construct.rs:255-277`:

```rust
if let Some(pinned) = prepared.pinned {
    statistics.candidate_evaluations += 1;
    return match scorer.probe(prepared.activity, pinned) {
        verdict if verdict.allowed => { scorer.place(…); Attempt::Placed }
        verdict => Attempt::Refused(cause_of(&verdict)),   // ← left UNPLACED
    };
}
```

For a lock this is right and M9A documented it: an *impossible* pin is reported rather than
forced. For a repair scope it is fatal. SOLVER.md §7 proposes that out-of-scope activities be
*"treated as pinned for this run"*; if two out-of-scope placements already clash with each other
— an ordinary state for a timetable somebody is repairing *because* it has clashes — the second
one's probe fails and the repair **silently drops a placement outside the scope**, breaking the
first mandatory guarantee.

**This is the single most important finding in the audit**, and §2.1 is the decision it forces.

## A.3 The order already puts immovable things first

`constrainedness` (`construct.rs:425`) sorts on `(pinned?, −duration, range, −degree, index)`.
Key 1 is pinned-first, and the fifth key is the activity index, which is what makes the order
**total** — two runs at one seed sort identically. A frozen set slots into key 1 at no cost, and
determinism is unaffected because the ordering function stays total.

`construct.rs:153-160` (M9A's edit) records that key 1 became reachable when locks landed.

## A.4 The candidate walk is a seeded generator with a cheap insertion point

`place_one` (`construct.rs:255-341`) walks `Candidates::new(seed, activity, runs, rooms)`,
applies an availability pre-filter that costs no candidate evaluation, probes, and **takes the
first feasible candidate**:

```rust
let verdict = scorer.probe(prepared.activity, placement);
if verdict.allowed { scorer.place(…); return Attempt::Placed; }
```

Because it is first-feasible rather than best-of-all, *"prefer the existing placement"* is
achieved by **trying the existing placement first** — no score change, no new rule. §2.3.

## A.5 `IncrementalScorer::of` exists and starts a repair cheaply

M9A established (`TIMETABLE-EDITING.md` §3) that `of(problem, assignment, constraints) ->
Option<Self>` has existed since M6, with a test asserting it starts at the reference score, and
an M9A test asserting `of` ≡ `empty` + `place` per entry over every fixture. A repair's starting
scorer is therefore already available and already proven equivalent.

Measured in `editing_budget.rs`: **3.8 ms** over a clean 1 600-occurrence arrangement (debug),
2.1 ms release. `None` means the arrangement does not describe the problem.

## A.6 `Problem::pinned` is the caller's, and the loader will never fill it

ADR 0045 §4 and `SCHEDULING-PROBLEM-ASSEMBLY.md` §4. `Timetables::pins(timetable_id, assembly)`
(`repo/timetables.rs:2088`) returns `StoredPins { pinned, locked, not_in_problem }`. A repair
populates `pinned` the same way M9A's editing session does. **No loader change.**

`StoredPins::not_in_problem` already reports a lock on a lesson the campus file no longer holds
— which is one of the two "impossible lock" classes §2.2 must handle.

## A.7 `Timetables::arrangement` gives the baseline, and already reports what it dropped

`repo/timetables.rs:477` → `StoredArrangement { assignment, entries, not_in_problem }`. The
baseline a repair starts from is one call, and `not_in_problem` is the honest statement that the
baseline is *less than what was stored*.

## A.8 `save_generated` would destroy every lock and manual flag

`repo/timetables.rs:356-366`:

```rust
insert_entry(tx, timetable.id, *activity_id, placement,
    EntryFlags { is_locked: false, is_manual: false })?;
```

Hard-coded, with a comment saying so deliberately. A repair saved through this path loses every
lock the user set and every manual placement they made — which contradicts two mandatory
guarantees. **M9B needs its own write path**, not a flag on this one. §3.3.

## A.9 The freshness guard is already inside the writing transaction

`save_generated` calls `guard_freshness(tx, watermark)` as the first statement inside
`self.db.write(|tx| …)`. ADR 0042 §4, and ADR 0030's rule that a precondition is never compared
in the interface. A repair's save reuses it unchanged, and adds `guard_edit`'s arrangement stamp
(§4.3).

## A.10 `ArrangementStamp` is exactly the guard a repair needs, for a reason M9A did not need

ADR 0047 gave editing an exact `(entry id, rev)` list because `expectedRev` cannot see a change
to a *different* entry. A repair is the same problem one level up: the whole baseline is the
precondition, so the stamp is the natural guard and `EditGuard` already carries it beside the
watermark (`repo/timetables.rs:1556`).

`ArrangementStamp::differences(&other) -> usize` (`:1515`) already returns a count, which the
"somebody edited this while you were repairing" message needs.

## A.11 The generation session is the state machine to copy, not to extend

`commands/session.rs` opens with the five hazards and the transition diagram:

```text
start ──► Running ──► Finished ──► Saving ──► Saved
                         ^           │
                         └─ failed ──┘   (the write is retryable)
```

and the four guarantees M9B needs verbatim: cancellation is a **request** that sets an
`AtomicBool` and no state; a save is claimed by a **compare-and-swap** into `Saving`; `Saved` is
terminal; and a session records the **project epoch** it began under. §4.

Its header also records the figure a repair budget starts from: *"the ~900 ms a 2 000-occurrence
solve takes"*, and the architectural permission — *"the command thread … **releases the lock**,
and moves the owned assembly to a dedicated thread"*, kept honest by
`evara_db::scheduling`'s `send_assertions` test.

**Copy rather than extend.** A repair session carries a baseline, a scope and a stamp that a
generation session has no field for, and three of the five hazards differ in their guard. §4.1.

## A.12 Nine constraint types, and `minimise-disruption` is not one of them

`ConstraintType` (`evara-core/src/scheduling/constraints.rs:85-101`) holds exactly nine:
`TeacherClash`, `StudentClash`, `RoomClash`, `RoomTypeRequired`, `EntityUnavailable`,
`DurationContiguous`, `NoTeachingInPeriod`, `ActivityFixed`, `ActivityScheduled`.

`CONSTRAINTS.md` §3's Operations table lists `minimise-disruption` (`OPS-001`, MEDIUM) as `M1`
and unimplemented. §2.3 recommends **not** implementing it in M9B.

## A.13 `UnplacedCause` and `is_impossible` already answer "could this ever work?"

`evara-core/src/scheduling/outcome.rs:578-633`. **Seven** variants; `is_impossible` is true for
four of them — `NoResolvedBellSchedule`, `NoContiguousRun`, `NoCandidateRoom`,
`AllCandidatesUnavailable` — and deliberately false for the other **three**:
`BlockedByPlacements`, `NotReached` and `InputRejected`. Because *"a greedy construction that
leaves a lesson unplaced has discovered that **its** arrangement had no room, not that no
timetable exists"* (`CLAUDE.md` §3). A repair inherits this and must not weaken it.

The database draws the same line and says so: `solve_run_unscheduled.cause`'s CHECK
(`V17__timetable_persistence.sql:318-321`) lists all seven keys, with the comment *"The first
four are properties of the PROBLEM and may be read as 'no timetable can place this'; the last
three are properties of this RUN and may not."* A repair's unscheduled rows go into that table
unchanged, so the four/three split is already the stored vocabulary and M9B adds no cause.

## A.14 There is no diff machinery anywhere

`evara-report` exposes `findings` and `projection` and nothing that compares two arrangements. A
repair preview is new code, and §2.6 places it in `evara-report` because it is a projection
rather than a scheduling rule.

---

# Part B — Recommended architecture

## 2.1 D1 — Scope is a movable set, never a set of pins

**Decision.** The solver receives a **baseline** `&Assignment` and a **movable set** of activity
indices. Everything not in the movable set is *frozen*: it is in the starting arrangement, every
constraint sees it, and it is never a candidate, never probed and never dropped.

> **The scope limits what may move. It does not limit what is checked.**

That single sentence answers the brief's hardest question — how scope boundaries interact with
constraints spanning several resources. A `teacher-clash` between an in-scope and an out-of-scope
occurrence is caught with no special handling, because the out-of-scope placement is in the
arrangement the scorer holds. No constraint needs to know the scope exists.

### Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **Pin everything out of scope** (SOLVER.md §7's wording) | A.2. A pre-existing clash *between two out-of-scope placements* makes the second probe fail, and the repair drops a placement it promised not to touch. Repair is used *because* a timetable has clashes, so this is the common case, not the edge case |
| **Compile a sub-problem of in-scope activities only** | Needs re-indexing (`ActivityIx` is dense and problem-relative), a new "reserved occupancy" vocabulary in `Problem` for the out-of-scope placements, and `Occupancy` rebuilt against it. ADR 0037 §2 refused to widen `Problem` for a diagnostic field and ADR 0044 refused it for a display field; widening it for repair bookkeeping is the same move a third time |
| **Delete out-of-scope placements and re-add them after** | The solver would place in-scope lessons into slots that are actually occupied, and the arrangement would be illegal between the two halves. Also unrecoverable on cancellation |

### Structural scope and positional scope are different, and the difference is user-visible

| Scope | Derived from | Closed under constraints? |
| --- | --- | --- |
| Whole timetable | — | yes, trivially |
| Class section | `Problem::sections` — a property of the inputs | **no** |
| Teacher | `SectionFact.teachers` — a property of the inputs | **no** |
| Room | the **baseline** — which occurrences are currently in that room | **no** |
| Cycle day | the **baseline** — `timeslot.cycle_day` of the current placement | **no** |
| Explicit occurrences | the user's selection | **no** |

Only the whole-timetable scope is closed. For every other scope, fixing an in-scope occurrence
may require moving an out-of-scope one, and the repair will report the in-scope occurrence as
unplaced rather than reach outside its scope. **This is reported, never worked around by silently
widening the scope** — a repair that moved a lesson the user excluded would be worse than one
that failed. §2.10 makes that a rule rather than a preference, and gives the UI the one
permitted response: offering a *separate* repair at a wider scope.

An occurrence that was **unplaced in the baseline** is in no room and on no day, so it is a member
of neither positional scope — §2.10 tabulates what each scope kind does with it.

The two baseline-derived scopes (room, day) carry a second subtlety: a room or a day is a
*position*, and repair changes positions. An occurrence repaired out of Monday leaves the
Monday scope; one repaired into Monday was never in it. So the movable set is computed **once**,
from the baseline, before the solve — not re-derived as the solve proceeds. Recomputing it would
make the scope depend on the search order, which is a different repair for the same request.

## 2.2 D2 — Frozen is `out-of-scope ∪ locked`, and impossible locks are found before solving

**Decision.** A locked placement is frozen *and* stays in `Problem::pinned`.

- **Frozen** so it can never be dropped (A.2).
- **`pinned`** so `activity-fixed` still judges it, which is what keeps the editor, the generator
  and the repair unable to disagree about what a lock means (ADR 0045 §5, `CLAUDE.md` §2).

Because a frozen occurrence is never probed, `activity-fixed` can only fire on it if something
*else* was placed on top of it — which the clash rules refuse first. So the rule becomes a
backstop here exactly as it is in the editor, and that is the intended relationship.

### Pre-flight lock validation — three classes, before any solving

Run `evaluate` (the **reference** scorer, not the incremental one) over a baseline restricted to
the frozen set. This is the canonical registry answering the canonical question; no second engine
and no new rule.

| Class | Detected by | Reported as |
| --- | --- | --- |
| **Unexpressible** — a lock the compiled problem cannot represent | `StoredPins::not_in_problem` (A.6), **widened** — see §2.2.1 | a **blocking** finding, one per lock, each naming its own remedy. **Corrected from a warning** |
| **Contradictory** — two locks clash with each other | a HARD violation in the frozen-only evaluation **naming two locked occurrences**, with the rule's own `explain()` | a **blocking** finding: repair is refused until a lock is removed |
| **Impossible in context** — a lock clashes with a frozen *out-of-scope* placement | the same evaluation, a HARD violation **naming one locked occurrence** | a **blocking** finding, with both sides named |

A repair over a frozen set whose *locks* are already contradictory cannot succeed, so refusing
before solving is both cheaper and clearer than reporting it afterwards as an unexplained failure.

### 2.2.1 ⚠ Corrected — an unexpressible lock blocks, because it cannot be preserved

The original text made this a warning that did not block. That is incompatible with the mandatory
guarantee that a locked placement is never moved, removed or silently discarded, and the data
model settles it rather than leaving it to judgement.

**What `pins` actually does** (`repo/timetables.rs:2088-2117`). For each entry with
`is_locked = 1` it resolves three indices, and on **any** of the three failing it pushes
`entry.activity_id` onto `not_in_problem` and `continue`s — so the lock is **absent from
`Problem::pinned`**:

| Cause | The resolution that fails | What then happens to the lock |
| --- | --- | --- |
| **Activity not in the problem** | `assembly.index.activity_index(entry.activity_id)` | The occurrence has no `ActivityIx`. It cannot be frozen, cannot be placed, and `project_entries` — which walks the `Assignment` — cannot write it back. **The lock vanishes from the new version** |
| **Timeslot not in the problem** | `assembly.index.timeslot_index(entry.timeslot_id)` | Worse. The *activity* may well resolve, so the occurrence is an ordinary member of the problem with **no pin and nothing marking it frozen** — a repair whose scope includes it will **move it**, which is the guarantee broken outright rather than quietly |
| **Room not in the problem** | `entry.room_id.is_some()` and `room_index(...)` is `None` | Same as the timeslot case: the activity resolves, the lock does not, and the occurrence is movable |

**Why it cannot simply be carried across.** Every foreign key on `timetable_entry` is
`ON DELETE RESTRICT` — `activity_id`, `timeslot_id` and `room_id` alike
(`V17__timetable_persistence.sql:197-206`) — so the referenced rows **still exist**; a deletion
would have been refused while the entry referenced them. The row is therefore physically
insertable into the new timetable. It is still wrong to insert it:

- The repair solved a problem that **does not contain that occurrence**. Nothing checked the
  carried-over placement against the repaired arrangement, so writing it produces a timetable
  with a row no evaluation scored and no preview showed — a timetable described as having no
  confirmed clash when one of its own placements was never examined. ADR 0032 rule 4 exists to
  prevent exactly that sentence.
- It would make `save_repaired` a second writer into `timetable_entry` alongside the projection,
  and the evaluation's `placed_count` / `required_count` would disagree with the row count.

So the model does not prove the lock can be preserved **exactly**; it proves it cannot. Blocking
is the only answer that keeps the guarantee, and it is also the honest one: the user is told
before anything runs, rather than discovering afterwards that a lock is gone.

### The remedy, per cause

`not_in_problem: Vec<Uuid>` is not enough to say any of this — it carries one activity id and not
which of the three resolutions failed. `StoredPins` therefore gains a richer report (§3.2), and
the screen names the remedy:

| Cause | What the user is told, and what to do |
| --- | --- |
| **Activity not in the problem** | *"This locked lesson is not part of the campus, cycle and term being repaired."* Usually the lesson belongs to another term or was moved to another school. Remedy: repair the timetable for the cycle and term the lesson belongs to, or remove the lock on it |
| **Timeslot not in the problem** | *"This lock names a period that is no longer in the bell schedule."* The slot row survives — ADR 0011's materialisation *"never deletes; it reports orphan candidates and leaves them alone"* — so this is the realistic case after a grid change. Remedy: unschedule or re-place that lesson in a slot that exists, then lock it again |
| **Room not in the problem** | *"This lock names a room that is no longer available to this campus."* Typically a shared room withdrawn via `room_campus`. Remedy: re-place the lesson in an available room and lock it again, or share the room with this campus again |

Each finding names the lesson, the slot or room involved, and offers the lock's own removal as the
one-click alternative — because the user may simply no longer want it. **Nothing in the repair
path writes or clears a lock on the user's behalf**: `set_lock` is the editor's command, and a
pre-flight that silently unlocked something to let itself proceed would be the discarding this
correction exists to prevent.

> **Asymmetry with M9A, restated.** M9A *reports* an impossible lock and lets the user carry on,
> because the editor is answering about one placement the user can see and is about to write one
> row. A repair is about to write a whole new version from a compiled problem that cannot
> represent the lock at all. The editor's tolerance is right for the editor and wrong here.

### ⚠ Corrected 2026-10-25 — only a lock may block, and the level must be filtered

The original text of this section refused a repair on **any** HARD violation in the frozen-only
evaluation. Re-examination shows that is wrong twice, and both corrections are load-bearing.

**1. A HARD violation among unlocked out-of-scope placements must be reported, never blocking.**
A frozen set holds every out-of-scope placement, and a timetable somebody is repairing routinely
has pre-existing damage there — an availability rule entered since it was saved, a room whose type
changed, two out-of-scope lessons that already clash (A.2's own case). None of that is a lock
problem, none of it is in scope, and none of it is something the repair was asked to fix.
Blocking on it would refuse repair on **precisely the timetables repair exists for**, which
inverts the feature. So the refusal predicate is:

> A HARD violation blocks only if `Violation::activities` intersects the **locked** set.
> Everything else in the frozen set is reported as pre-existing, with the rule's own sentence and
> the note that the chosen scope cannot reach it.

`Violation::activities` is *"the occurrences at fault, in index order … two for a clash, one for
everything else"* (`constraints.rs:462-466`) — and `CLAUDE.md` §4 makes "at most two" a
precondition of the whole incremental design, asserted by `differential.rs`. So the intersection
is a two-element set test and needs no new vocabulary. Where the question is only *"does anything
involve these locks?"*, `evaluate_involving` — a trait member since M6, so every registered type
has one — asks it narrowly rather than sweeping and filtering.

**2. The pre-flight must filter by level, because `activity-scheduled` will fire en masse.**
A frozen-only arrangement leaves every in-scope occurrence unplaced, and `activity-scheduled`
exists to notice exactly that. Verified levels: `ActivityScheduled` is `ConstraintLevel::Medium`
and `ActivityFixed` is `ConstraintLevel::Hard`
(`evara-core/src/scheduling/constraints.rs:166,174`). So reading HARD only is both correct and
sufficient — but it has to be *stated*, because an implementation that reported every finding
from this evaluation would show the user one MEDIUM finding per movable occurrence and call a
healthy timetable catastrophic.

A locked occurrence that is frozen is placed at its pin, so `activity-fixed` is satisfied; a
pinned occurrence that is *unplaced* is deliberately not a violation of it (ADR 0045,
`CLAUDE.md` §3), so nothing in the frozen-only evaluation makes locks look broken.

**Note the asymmetry with M9A, deliberately.** The editor *reports* an impossible lock and lets
the user carry on; repair *refuses*. The difference is that an editor is answering about one
placement the user can see, while a repair is about to rearrange a timetable on a foundation that
cannot hold.

## 2.3 D3 — Baseline preference is candidate ordering, not a constraint

**Decision.** For each in-scope occurrence with a baseline placement, probe that placement
**first**. If it is still legal, the greedy walk's first-feasible rule takes it and the
occurrence does not move.

```rust
// in place_one, before the seeded walk
if let Some(at) = prepared.baseline {
    statistics.candidate_evaluations += 1;
    let verdict = scorer.probe(prepared.activity, at);
    if verdict.allowed { scorer.place(prepared.activity, at); return Attempt::Placed; }
    statistics.candidates_rejected += 1;
}
```

Nine lines. No score change, no new constraint type, no new artefacts under `CLAUDE.md` §4, and
fully deterministic — the baseline is data, not a random choice.

> **One note on the budget, added 2026-10-25.** The seeded walk tests
> `statistics.candidate_evaluations >= config.limits.max_candidate_evaluations` *before* each
> evaluation and returns `Attempt::OutOfBudget` (`construct.rs:319-322`); the snippet above does
> not, and nor does the pinned branch it is modelled on (`construct.rs:269-271`). That is the
> right precedent — a baseline probe is the one candidate the caller already chose, and refusing
> it for budget would leave an occurrence unplaced that was legally where it already was. But it
> means a repair spends **one evaluation per in-scope occurrence with a baseline** before the
> walk begins, so a repair's counters are not comparable to a generation's at the same seed.
> Deterministic, and worth recording in `solve_run.statistics` rather than explaining later.

**What this does and does not promise.** It gives *"prefers existing placements within the repair
scope when feasible"* exactly. It gives **no** minimal-disruption guarantee, and cannot: once an
occurrence is displaced, the occurrences placed after it see a different timetable, and greedy
construction has no mechanism to reconsider. The proposal states this plainly rather than
implying otherwise, as the brief requires.

### Why not `minimise-disruption` (OPS-001)

| Objection | Detail |
| --- | --- |
| **It has nowhere to put the baseline** | `CompiledConstraint` carries `params` as JSON. A baseline is one placement per activity — problem-sized per-activity data, not configuration. Threading it in means either widening `CompiledConstraint` or widening `Problem`, and ADR 0037 §2 refused the latter twice already |
| **It would make scores incomparable** | A MEDIUM penalty for differing from a baseline means a repaired timetable's score is not on the same scale as a generated one's. ADR 0042 §5 keeps the GENERATION scoring immutable so that scores *can* be compared across time; a baseline-relative penalty quietly undoes that |
| **Greedy cannot minimise it** | First-feasible takes the first legal candidate. A disruption penalty changes `delta`, which the walk does not consult. SOLVER.md §5's own note says the deltas *"are already there"* for when a real minimisation exists — and a real minimisation is local search, which `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 keeps out of the MVP |
| **It would be a rule that cannot fire for most callers** | `CONSTRAINT-ENGINE.md` §3's argument against registering `activity-fixed` early: *"a registered rule that cannot fire tells the user something is being checked when it is not"*. A generation has no baseline |

`minimise-disruption` becomes worth implementing when local search arrives. §7 places it there.

## 2.4 D4 — A repair produces a new immutable version; there is no in-place apply

**Decision.** Approving a repair **inserts a new timetable** carrying the repaired arrangement,
its locks, its manual flags and a `REPAIR` run record that names the timetable it was based on.
The original is untouched.

This is the brief's either/or answered as *new version*, and it is chosen because it makes two
mandatory guarantees **structural rather than guarded**:

- *"Failed, cancelled or stale repair sessions cannot modify a saved timetable"* — no code path
  exists that writes into the baseline. There is nothing to get wrong.
- *"Historical generation and evaluation records remain immutable"* — the original's `solve_run`
  and `GENERATION` scoring are not reachable from the repair's write.

It also fits ADR 0042 without amendment: *a save always inserts*, several timetables coexist for
one `(cycle, term)`, and *"replacement is a save followed by an explicit delete"*. A repair is a
save; discarding the original is the user's separate decision.

### What is preserved, and how

| Carried from the baseline | Mechanism |
| --- | --- |
| `is_locked` | frozen placements are unchanged by construction, and the flag is copied |
| `is_manual` | copied. A placement a human made and the repair did not move is still the human's — M9A established `is_manual` as a high-water mark (`TIMETABLE-EDITING.md` §5) |
| `is_manual` on a **moved** occurrence | **cleared to 0.** The solver chose this placement, not a person. This is the one flag transition a repair makes, and it is the honest one |
| lineage | a new `timetable.based_on_timetable_id` and `solve_run.kind = 'REPAIR'` — §3.2, and the migration that needs approval |

### Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **In-place apply** to the baseline | Every write path into a live timetable is a path a stale session could take. It would also need the arrangement stamp as a hard guard on a multi-row write, and a partial failure mid-apply would leave a timetable that is neither the original nor the repair. ADR 0046 permits in-place *editing* because one edit is one row in one transaction; a repair is hundreds |
| **Both, user's choice** | Two write paths, one of which is only safe when several guards hold. Offer it in a later milestone if users ask, with the diff as evidence that they understood what they were replacing |
| **A `draft` flag on `timetable`** | A lifecycle column. `TIMETABLE-PERSISTENCE.md` §12 records that the published/archived lifecycle is deliberately out of the MVP, and a draft flag is the first half of one |

## 2.5 D5 — Partial, unsuccessful and unchanged outcomes reuse M6's vocabulary

**Decision.** A repair returns a `SolveOutcome` exactly as a generation does, wrapped with the
diff. No new status vocabulary.

| Result | Representation |
| --- | --- |
| Complete | `SolveStatus::Complete`, diff shows what moved |
| Partial | `SolveStatus::Partial` with `UnplacedCause` per occurrence. **A repair that leaves an in-scope occurrence unplaced is a partial repair, not a failure**, and is applyable if the user accepts it |
| Refused before solving | `Termination::InputRejected` — a blocking lock finding (§2.2), a malformed request (§2.10), or a baseline that is not readable against the problem |
| Cancelled | `Termination::Cancelled`, and **not applyable**. The one outcome this workspace documents as not reproducible |
| Internally inconsistent | `Termination::InternalCheckFailed` — **V1, V2 or V3 of §2.9**, not `score.is_feasible()`. A **defect in Evara**, reported as one, never applyable (`CLAUDE.md` §3) |
| **No change made** | `moved`, `newly_scheduled` and `unscheduled` all empty. A real, reportable result — not a failure and not an error |

**The *no change* case is worded as a statement about this attempt, not about the timetable.**
Corrected: the original said *"nothing could be improved within this scope"*, which is a claim
the engine is not entitled to make. A greedy construction that changed nothing has discovered
that *its* walk, at *this* seed, found the baseline placements legal and reached no better
arrangement — it has not established that none exists. This is the same discipline
`UnplacedCause::is_impossible` enforces for a single occurrence, applied to the whole run. The
screen says **"This repair attempt made no changes."**, and may add what was examined (*"47
lessons were reconsidered; each is still legal where it was"*). It may not say *improved*,
*optimal*, *best possible* or *nothing could be done*.

### Applyability, enumerated

Clarification 8, stated as a closed list so nothing unsafe becomes applyable by omission. A
repair is applyable **only** when the session is in `Finished` **and** every one of these holds:

| Requirement | Why |
| --- | --- |
| `termination` is not `Cancelled` | A cancelled run's arrangement is a partial walk stopped at an arbitrary point, and it is the one result this workspace does not call reproducible |
| `termination` is not `InputRejected` | Nothing was attempted |
| `termination` is not `InternalCheckFailed` | §2.9's V1–V3. A defect, never a timetable |
| `termination` is not `CandidateLimitReached`, `StepLimitReached` or `PassLimitReached` **unless** the user is shown that a limit stopped it | These produce an *honest* partial arrangement and are applyable — but silently applying a budget-truncated repair presents a limit as a conclusion. Applyable, with the limit named in the preview |
| §2.9's V2 set is empty and V3 holds | Which is the previous row restated as a predicate rather than a status |

`SolveStatus::Partial` is **applyable** — a partial repair is an ordinary, useful result, and
§2.8 shows that a repair of a timetable with out-of-scope unplaced lessons is *always* `Partial`
whatever it achieved. Refusing `Partial` would refuse almost every real repair. `Infeasible` is
unreachable from this engine (`lib.rs:319-323`) and is refused if it ever appears.

A HARD violation in the final arrangement is therefore **not** on its own a bar to applying, and
must not be turned into one: the frozen pre-existing set is expected to be non-empty. What bars
applying is a hard violation **involving a movable occurrence** — V2 — which is a defect rather
than an outcome.

**`is_impossible` is not weakened.** A repair that cannot place an in-scope occurrence reports
`BlockedByPlacements`, which `is_impossible` deliberately returns false for. The scope makes this
*more* important: an occurrence may be unplaceable only because the scope forbids moving what is
in its way, which is a statement about the request and not about the timetable.

## 2.6 D6 — The preview is a projection, in `evara-report`

**Decision.** A new `evara_report::repair` module computes the before/after comparison. It is a
projection: it resolves nothing, judges nothing and decides no legality — it compares two
`Assignment`s against one `Problem` and labels each occurrence.

Five categories, which are exhaustive and disjoint:

| Category | Definition |
| --- | --- |
| `Unchanged` | placed in both, same `Placement` |
| `Moved` | placed in both, different `Placement`. Carries both, and whether the slot, the room or both changed |
| `NewlyScheduled` | unplaced before, placed after |
| `Unscheduled` | placed before, unplaced after — **with the `UnplacedCause`**, because this is the category a user needs explained most |
| `StillUnplaced` | unplaced in both |

Plus, per the brief: the score before and after at all three levels, the findings before and
after through `evara_report::explain_all` so each carries **the rule's own sentence** (ADR 0043),
and the frozen/movable counts so the diff states what the scope actually was.

**And §2.9's partition**, which is the same module's work and the same two reports. The diff
carries the four buckets — eliminated, pre-existing-frozen, new-involving-movable, and
not-fixable-within-this-scope — alongside the full arrangement's score and hard-violation counts,
never instead of them. The violation **totals the user sees are the whole repaired timetable's**;
the buckets only say which are attributable to the repair. `Unscheduled` entries carry their
`UnplacedCause`, and `StillUnplaced` covers both an in-scope occurrence the repair could not place
and a frozen occurrence that was never in scope to place — distinguished by a flag, because the
remedies differ (§2.10).

Each entry is labelled with `LabelSet` (ADR 0044), so the preview can say *"9 Mathematics B moved
from Day A Period 1 to Day C Period 4, Room B12 → Room C04"* without the view layer resolving
anything.

## 2.7 D7 — One new guard on the session: the baseline's arrangement stamp

A repair holds a baseline for the length of a solve the user then reviews — possibly minutes.
Three things can move underneath it, and each already has a guard:

| What moved | Guard | Already exists |
| --- | --- | --- |
| A scheduling **input** | `assembly.watermark` + `guard_freshness` inside the write | ADR 0042 §4 |
| The **baseline arrangement** (somebody edited it in another window) | `ArrangementStamp` + `guard_edit` inside the write | ADR 0047 |
| The **project** | `ProjectEpoch`, compared before the transaction opens | `commands/session.rs` |

All three are compared **inside the writing transaction**, in `evara-db`, never in the command
layer and never in the interface — ADR 0027's rule and ADR 0030's.

The stamp is the guard M9A invented for a reason that applies here with more force: a repair's
every verdict was computed against one snapshot of the arrangement, so if *any* placement moved,
the whole diff is a claim about a timetable that no longer exists.

**Why the watermark cannot stand in for the stamp — now a verified fact, not an argument.**
`inputs_changed_since` (`change_log.rs:295-314`) asks the feed for a change on any table **not
in** `SOLVE_OUTPUT_TABLES`, and that list is `timetable`, `timetable_entry` and `solve_run`
(`change_log.rs:285-287`). That exclusion is correct and deliberate — a save must not be refused
because some *other* timetable was written — but it has a consequence this proposal depends on:
**an edit to the baseline's own entries provably cannot trip `guard_freshness`.** The two guards
are therefore not redundant and neither can be dropped; the stamp is the only thing watching the
arrangement, exactly as ADR 0047 found for a single edit. It also means a repair's own write
cannot invalidate a later repair's freshness check, which is what makes repair-then-repair work.

---

## 2.8 ⚠ Corrected — the repair construction algorithm, exactly

**C1.** The original §3.1 said *"opens with `IncrementalScorer::of` when a baseline is given"*
without saying **which** arrangement `of` receives. The only correct answer is *the frozen
placements alone*. Initialising from the full baseline would be wrong in three compounding ways,
and all three are properties of the real APIs:

1. **Each movable occurrence would be reconsidered against the stale positions of every other
   movable occurrence.** Occurrence 1 would avoid a slot that occurrence 7 is about to vacate, so
   the repair would be strictly worse than it needs to be and the result would depend on the
   order in a way the baseline-first rule cannot compensate for.
2. **A pre-existing movable-to-movable clash would refuse both.** `PlacementVerdict::allowed` is
   `false` whenever the result *carries* a hard violation naming the occurrence — the stricter of
   the two readings, deliberately (`incremental.rs:80-90`). Two movable occurrences already
   clashing with each other are each, from the other's point of view, an immovable obstacle.
   Probing either at its own baseline returns `allowed: false`, and the walk then has to find a
   third place for a lesson whose own partner was about to move away.
3. **`place` does not withdraw first.** `IncrementalScorer::place` (`incremental.rs:291-299`)
   measures `involving` before, calls `occupancy.place`, measures after and applies the delta. It
   is `probe` that withdraws and restores (`:247-281`). So an algorithm that left movable
   occurrences in the arrangement would have to withdraw each one explicitly before re-placing
   it, and every occurrence it had not yet reached would still be sitting in the way.

With the frozen-only initialisation all three disappear, and the self-clash question answers
itself: a movable occurrence is **unplaced** in the working arrangement, so `probe`'s
`occupancy.withdraw(activity)` is a no-op, `before` is its `activity-scheduled` violation alone,
and nothing can clash with itself. It also means every probe a repair makes is a probe of an
*unplaced* occurrence — the only kind M6 makes — so the "carries" versus "adds" distinction in
`PlacementVerdict::allowed` never arises and M6's semantics are inherited unchanged.

### The seven steps, against the real signatures

```text
 0. PRE-FLIGHT          §2.2 — locks validated and unexpressible ones refused, before anything
 1. BASELINE            baseline: Assignment            (immutable for the whole run)
 2. MOVABLE             movable: Vec<ActivityIx>        (derived once, validated, sorted, deduped)
 3. FROZEN              frozen = all activities \ movable, locked ⊆ frozen
 4. WORKING             working = baseline.clone(); for ix in &movable { working.clear(ix) }
 5. SCORER              IncrementalScorer::of(problem, &working, constraints)?   -> else refuse
 6. CONSTRUCT           order = prepare(..) filtered to movable; baseline-probed first
 7. VERIFY              §2.9's scope-aware acceptance, then the diff against `baseline`
```

| Step | What it calls, verified | Why it is this and not something else |
| --- | --- | --- |
| **1. Preserve an immutable baseline** | `Timetables::repair_baseline` returns one `StoredArrangement` whose `assignment` is never mutated after it is read | It is both the comparison for the diff (§2.6) and the candidate preference (§2.3). A single mutable arrangement serving as working state *and* as the thing being compared against is the classic version of this bug |
| **2. Derive and validate the movable set once** | computed from `baseline` + `Problem` (§2.1's six scopes), then `sort_unstable()` and `dedup()`; validated per §2.10 | Computed once, **never re-derived as the solve proceeds** — §2.1. Sorted and deduped because a duplicated index would place one occurrence twice and `Assignment` would silently keep only the last placement (`place` returns the prior value, which the caller would discard) |
| **3. Derive the frozen set** | `frozen = (0..problem.activities.len()) \ movable`; assert `locked ∩ movable = ∅` | Locked ⊆ frozen is D2, and asserted rather than assumed. A locked index that reached `movable` would be a repair moving a lock, so it is a `debug_assert` *and* a release-mode refusal, not a comment |
| **4. Build the working assignment** | `let mut working = baseline.clone();` then `working.clear(ix)` for each movable `ix` | **Subtraction, not construction.** `Assignment::clear(&mut self, ActivityIx) -> Option<Placement>` (`placement.rs:232`) is the exact operation. Building up from `Assignment::empty(n)` and placing the frozen set would work, but it makes "every frozen placement survived" depend on the frozen derivation being right; cloning and clearing makes it depend on the *movable* derivation being right, which step 2 has already validated. It also inherits `len()` from the baseline, so the dimension cannot drift |
| **5. Initialise the scorer** | `IncrementalScorer::of(problem, &working, constraints) -> Option<Self>` (`incremental.rs:156-173`) | `of` builds `Occupancy::of` and returns `None` when `invalid_references()` is non-empty. `None` is a **refusal** — `Termination::InputRejected` — not a fallback to `empty`. A repair that quietly started from an empty arrangement would regenerate the whole timetable while reporting itself as a repair, which is the most destructive thing in this document |
| **6. Re-place the movable set** | `prepare(problem, cache, config)` then `.filter(\|p\| movable.contains(&p.activity))`; `place_one` probes `prepared.baseline` first | Frozen occurrences are **removed from the order**, never pushed to the front of it. This refines A.3: the pinned-first key still exists and the order stays total, but it is inert during a repair because every pinned occurrence is frozen and so absent from `pending`. `place_one`'s pinned branch is therefore unreachable in a repair, which is correct — a lock is honoured by being in `working` at step 4, and `activity-fixed` judges it from there |
| **7. Preserve every frozen placement exactly** | asserted, not trusted — §2.9 and the test plan | Including pre-existing conflicts: a frozen clash is carried into the new version unchanged and reported as pre-existing. The repair is not permitted to "fix" it by moving something it was told not to touch |

### The closing invariant does not hold for a repair, and must be rewritten

`construct` ends with (`construct.rs:212-216`):

```rust
debug_assert_eq!(
    unplaced.len() + scorer.placed_count(),
    total,                                  // = problem.activities.len()
    "every occurrence is either placed or reported as unplaced"
);
```

For a repair this is **false**, and not marginally. `scorer.placed_count()` counts frozen and
movable placements together, while `unplaced` can only ever contain movable occurrences — frozen
ones never enter `pending`. So any occurrence that is **frozen and was unplaced in the baseline**
is neither placed nor reported, and the assertion fails. An out-of-scope lesson that the source
timetable never scheduled is an entirely ordinary state, so this is the common case, not an edge.

The invariant a repair must assert instead, over the movable set only:

```rust
debug_assert_eq!(
    movable_unplaced.len() + movable_placed,
    movable.len(),
    "every movable occurrence is either placed or reported as unplaced"
);
debug_assert_eq!(
    frozen_placed_now, frozen_placed_in_baseline,
    "a repair placed or dropped a frozen occurrence"
);
```

The second line is the scope guarantee as an assertion rather than a promise, and it is cheap:
both figures are counts over the same index set.

**Consequence for `SolveOutcome::unscheduled`, and why no new `UnplacedCause` is needed.** A
repair's `unscheduled` covers the **movable set only** — those are the occurrences this run
attempted. Frozen occurrences that were already unplaced appear in the preview as
`StillUnplaced` (§2.6 already has the category) and are **not** written to
`solve_run_unscheduled`, because that table records what a run could not place and this run never
tried. This keeps `UnplacedCause` at its seven variants: an eighth — `OutOfRepairScope`, say —
would require adding a value to `solve_run_unscheduled.cause`'s `CHECK`, and a `CHECK` cannot be
altered in SQLite, so `V18` would need a 12-step rebuild on a `STRICT` table and would stop being
additive. Reusing `StillUnplaced` costs nothing and keeps the approved migration intact.

`SolveStatus` stays honest about the **arrangement** rather than about the repair:
`Assignment::is_complete` (`placement.rs:263`) is *"every activity is placed"*, so a timetable
with out-of-scope unplaced lessons reports `Partial` whatever the repair achieved. That is the
truth about the timetable; the diff is what attributes it, and §5.2 requires the screen to say so
in words rather than leaving `Partial` to imply the repair failed.

## 2.9 ⚠ Corrected — hard-violation acceptance semantics

**C2, and the blocking defect.** Two questions were conflated in the original text. Separated,
the answers are opposite.

### Candidate acceptance is already safe — `probe` is scoped, and nothing changes

`IncrementalScorer::probe` (`incremental.rs:247-281`) computes
`let after = self.involving(activity)` and sets `blocking` to the HARD subset **of that list**.
`involving` (`:345-365`) runs each rule's `evaluate_involving` for the probed occurrence only —
the trait member `CLAUDE.md` §4 item 6 makes mandatory. So:

> A HARD violation between two frozen occurrences names neither the probed occurrence nor its
> candidate, never appears in `blocking`, and **cannot refuse any candidate**.

A pre-existing frozen clash is invisible to candidate acceptance. The only way a frozen placement
refuses a candidate is the correct way: the candidate would clash *with it*, producing a violation
that does name the movable occurrence. This is the scope-aware acceptance policy the correction
asks for, and it already exists — `evaluate_involving` is it. **No new policy, no second engine,
no change to `probe`.**

### The final verification is **not** safe, and this is the defect

`generate_with` (`lib.rs:227-272`) runs the independent reference check and then:

```rust
let report = evaluate(request.problem, &solution.assignment, request.constraints);
let status = status_of(&solution, &report);
```

and `status_of` (`lib.rs:324-336`) opens with:

```rust
if !report.is_readable() || !report.score.is_feasible() {
    return SolveStatus::Failed;
}
```

`Score::is_feasible` is `self.hard == 0` (`outcome.rs:93-95`) — a property of the **whole
arrangement**. A repaired arrangement contains every frozen out-of-scope placement, pre-existing
hard violations included. So:

> Every repair of a timetable holding **any** hard violation outside the repair scope would come
> back `SolveStatus::Failed` with `Termination::InternalCheckFailed` — which §2.5 lists as *"a
> defect in Evara … never applyable"*. The feature would refuse to apply its own correct work on
> precisely the timetables it exists for.

It is worth being exact about how bad the failure mode is: the repair would be *correct*, the
arrangement would be *better than the baseline*, and the product would report an internal defect
and discard it.

### The canonical fix: partition the reference report by the movable set

The reference evaluator stays the only judge of legality. What changes is the **question asked of
its report** — one comparison, in `evara-report`, using vocabulary that already exists.

Evaluate twice with the reference `evaluate`, which the preview needs anyway (§2.6):

```text
baseline_report = evaluate(problem, baseline, constraints)
final_report    = evaluate(problem, repaired, constraints)
```

Partition each report's HARD violations on whether `Violation::activities` intersects the movable
set. `activities` is *"the occurrences at fault, in index order … two for a clash, one for
everything else"* (`constraints.rs:462-466`), and `CLAUDE.md` §4 makes "at most two" a
precondition of the whole incremental design, asserted by `differential.rs` — so the intersection
is a one- or two-element test. `ViolationKey` (`constraints.rs:507-514`, deriving `Eq`, `Ord` and
`Hash`) is the identity for comparing the two reports, and `Violation::key()` (`:490`) builds it.

**The acceptance rules, precisely.** A repair is `InternalCheckFailed` — a genuine defect — if and
only if any of:

| # | Rule | What it catches |
| --- | --- | --- |
| **V1** | `!final_report.is_readable()` | The arrangement does not describe the problem. Unchanged from M6 |
| **V2** | any HARD violation in `final_report` whose `activities` **intersects the movable set** | The real incremental-versus-reference disagreement. `probe` refused every candidate carrying a hard violation naming the occurrence, so one surviving here means the two scorers disagree. **Stricter than M6's whole-arrangement check in the dimension that matters** |
| **V3** | the HARD violations in `final_report` **disjoint from** the movable set, compared by `ViolationKey`, are not **exactly** the same set as those in `baseline_report` | A frozen placement moved, vanished, or acquired a new conflict. Impossible by construction — which is why it is the assertion that proves step 7 rather than a comment claiming it |

Everything else is an ordinary outcome. In particular a non-empty frozen pre-existing HARD set is
**not** a failure, is **not** a reason to withhold apply, and is reported as what it is.

Note that V2 and V3 together are *strictly stronger* than `score.is_feasible()`, not weaker: the
global check cannot distinguish a frozen placement being disturbed from a pre-existing problem,
and V3 catches the former while tolerating the latter. The repair therefore verifies more than a
generation does, not less.

### The four distinctions, as one comparison

Required of the implementation, and each is a set operation on the two reports — no rule is
re-implemented and no violation is re-judged:

| Bucket | Definition | Shown as |
| --- | --- | --- |
| **Pre-existing, confined to frozen** | key ∈ both reports, `activities ∩ movable = ∅` | *"Already present, and outside the scope you chose"* — with the rule's own `explain()` (ADR 0043) and the note that this scope could not reach it |
| **New, involving a movable occurrence** | key ∈ `final` only, `activities ∩ movable ≠ ∅` | At HARD: **must be empty** — V2. At MEDIUM or SOFT: legal, reported as a cost of the repair |
| **Eliminated by the repair** | key ∈ `baseline` only | *"Fixed by this repair"* — the headline number the user is looking for |
| **Remaining because the scope cannot fix it** | the pre-existing frozen set, **plus** every in-scope occurrence left unplaced with `UnplacedCause::BlockedByPlacements` | *"This scope could not fix these"*, with §2.5's point that the scope may be the reason rather than the timetable |

The fourth bucket deserves its construction explained. At HARD level a violation cannot both
involve a movable occurrence and survive: if the occurrence was placed, `probe` guaranteed no hard
violation names it; if it was left unplaced, it has no placement to clash from. So "the scope
could not fix this" is expressed correctly as *pre-existing frozen damage* plus *in-scope
occurrences the scope's own boundary left unplaced* — which is also why
`UnplacedCause::is_impossible` must not be read as impossibility here (§2.5).

### Equivalence and reporting are both preserved

- **Reference-versus-incremental equivalence is untouched.** `differential.rs` is unchanged,
  `probe` is unchanged, `place`/`withdraw`/`apply` are unchanged, and the identity the gate
  asserts is unchanged. The frozen-only initialisation is just `of()` over a different
  arrangement, which `of` already supports and whose equality to `empty` + `place` per entry M9A
  already asserts over every fixture. What changed is only the **interpretation** of the final
  reference report, and that interpretation lives in `evara-report` beside the diff.
- **The preview reports the full arrangement's actual violations.** `SolveOutcome::score` and
  `hard_violations` continue to come from `evaluate` over the **whole** repaired arrangement, as
  `generate_with` already does via `hard_counts(&report)` — the partition is carried *beside*
  them for attribution, never instead of them. A preview that showed only in-scope violations
  would be describing a timetable the user is not about to save.

### Where the code goes

`generate_with` cannot be reused as-is: it hardcodes `start: None` and gates on `status_of`. The
repair gets a sibling, `repair_with`, which performs steps 1–7 and applies V1–V3 — so M6's
generation path is bit-for-bit untouched and no existing caller changes behaviour. `status_of`
itself is **not** modified, for the same reason.

## 2.10 Scope boundaries, request validation, and one consistent snapshot

### The scope is never widened automatically

Restated as a rule because §2.1 left it as a preference. **No code path may add an index to the
movable set that the user's chosen scope did not put there** — not to resolve a clash, not to
place an occurrence that would otherwise be left unplaced, not as a retry. A repair that moved a
lesson the user excluded is worse than one that failed, because the user cannot see what they did
not ask about.

Where a wider scope would plainly have succeeded, the **UI may offer to start a separate repair**
at that scope (§5.2) — a new session, a new baseline read, a new preview and a new approval. It
may never silently continue the current one. That also answers §8.4 without changing the engine's
behaviour: the strict answer is what the solver does, and the friendlier answer is a second
button rather than a different algorithm.

### Originally-unplaced occurrences, per scope kind

Clarified because the positional scopes have a consequence that will otherwise surprise. An
occurrence with no baseline placement has **no room and no cycle day**, so it cannot be a member
of any scope derived from a position:

| Scope | Is an originally-unplaced occurrence movable? | What the repair does with it |
| --- | --- | --- |
| Whole timetable | **yes** | Attempted. Appears as `NewlyScheduled` if placed, `StillUnplaced` with its cause if not |
| Class section | **yes**, if the occurrence's section matches | Attempted |
| Teacher | **yes**, if the section names that teacher | Attempted |
| **Room** | **no** — it is in no room | Frozen and left unplaced. `StillUnplaced` in the diff |
| **Cycle day** | **no** — it is on no day | Frozen and left unplaced. `StillUnplaced` in the diff |
| Explicit occurrences | **yes**, if the user selected it | Attempted. The one way to schedule an unplaced lesson without widening to a structural scope |

The two positional rows are the surprising ones, and §5.2 requires them stated **before** the
repair runs, not discovered in the diff: *"Repairing Monday will not schedule your 3 unscheduled
lessons — an unscheduled lesson is not on any day. Repair the whole timetable, or select those
lessons, to place them."* This follows from §2.1's rule that the movable set is computed once from
the baseline: an occurrence repaired *into* Monday was never in the Monday scope, and one repaired
*out of* it leaves — which is why the set cannot be re-derived mid-solve.

### Request validation, before any work

`repair_preflight` and `repair_start` both refuse a malformed request rather than compiling one.
Each is `Termination::InputRejected` with a named reason:

| Checked | Refused when |
| --- | --- |
| **Timetable identity** | the timetable id is unknown, or its `cycle_id` / `term_id` do not match the `Assembly`'s `selection` — a repair compiled against one selection and applied to another |
| **School ownership** | the timetable's school, resolved through `cycle.school_id`, is not the school the `Assembly` compiled. `DbError::WrongSchool`, and never a `LIMIT 1` read of `school` (`CLAUDE.md` §3) |
| **Scope identifiers** | a named section, teacher, room or cycle day does not exist, or exists but belongs to another school. A room is the case to watch: `room_campus` reaches across schools by design (ADR 0019), so a room id resolving is **not** proof it is this school's to repair by |
| **Duplicate activity indices** | the derived movable set contains an index twice, after `sort_unstable` + `dedup`. A duplicate means the derivation is wrong, so it is refused rather than silently collapsed |
| **Empty scope** | the movable set is empty. Refused as a request error — *"nothing in this scope can move"* — and **not** run as a repair that trivially changes nothing, because the two are different answers and §2.5's *unchanged* result would otherwise be unreadable |
| **Out-of-range indices** | any index ≥ `problem.activities.len()` |
| **Baseline compatibility** | `IncrementalScorer::of` returns `None` — the arrangement cannot be read against the problem (§2.8 step 5) |
| **Locked ∩ movable** | non-empty. A lock in the movable set is D2 broken; refused, not repaired around |

### One consistent snapshot

`Timetables::arrangement`, `pins`, `entries` and `arrangement_stamp` each call `self.db.read(…)`
separately today, and `arrangement` and `pins` each call `entries` again — so reading them one
after another opens **four transactions** and reads the entry rows three times. The baseline, the
lock flags and the stamp could then come from three different moments, and the stamp would be
guarding an arrangement that is not the one the repair was computed from. That is the exact race
`ArrangementStamp` exists to close, reintroduced by the way its inputs are gathered.

`repair_baseline` therefore reads the entries **once inside a single `db.read`** and derives all
four from that one read (§3.2). Nothing else is acceptable, and it is also faster.

Ordering against the `Assembly` matters too, and in one direction only. The `Assembly` is
compiled **first**, capturing `assembly.watermark`; the baseline snapshot is read **after**. A
scheduling input that moves in between is then *older* than the watermark, so `guard_freshness`
refuses the save — safe. The reverse order would leave such a change invisible to both guards,
because `timetable_entry` is in `SOLVE_OUTPUT_TABLES` and the stamp only watches entries. So:
**compile, then snapshot** — stated as a requirement, since the two reads look interchangeable.

## 3.1 `evara-solver` — a baseline, a movable set, and its own verification

```rust
/// What a repair may move, and what it must not.
///
/// Frozen placements are in the arrangement, visible to every rule, and never candidates.
/// `movable` is computed once from the baseline before the solve — never re-derived as it
/// proceeds, or the scope would depend on the search order.
pub struct RepairScope<'a> {
    /// The arrangement to start from and to prefer.
    pub baseline: &'a Assignment,
    /// The occurrences the solver may place. Everything else is frozen.
    pub movable: &'a [ActivityIx],
}

impl SolverEngine for ConstructionSolver {
    fn solve(&self, problem, start: Option<&Assignment>, …) -> Solution;   // unchanged signature
}

/// Repair is its own entry point rather than a mode of `solve`.
pub fn repair(
    problem: &Problem,
    scope: &RepairScope<'_>,
    constraints: &[CompiledConstraint],
    config: &SolveConfig,
    progress: &dyn ProgressSink,
    cancel: &dyn Cancel,
) -> Result<RepairSolution, RepairRefused>;

/// What a repair reached, with the accounting §2.8's invariant needs.
pub struct RepairSolution {
    pub assignment: Assignment,
    pub termination: Termination,
    pub statistics: SolveStatistics,
    /// **Movable occurrences only** — §2.8. A frozen occurrence unplaced in the baseline is
    /// not this run's failure and is not reported as one.
    pub unplaced: Vec<(ActivityIx, UnplacedCause)>,
    /// Asserted against the baseline, so the scope guarantee is checked and not promised.
    pub frozen_placed: usize,
    pub movable_placed: usize,
}

/// The repair entry point that also verifies. `generate_with`'s sibling, not a change to it.
pub fn repair_with(
    engine: &dyn SolverEngine,
    request: &RepairRequest<'_>,
    progress: &dyn ProgressSink,
    cancel: &dyn Cancel,
) -> SolveOutcome;
```

Changes inside `construct.rs`:

1. `construct` gains `start: Option<&Assignment>` and `movable: Option<&[ActivityIx]>`. When a
   baseline is given it opens with `IncrementalScorer::of(problem, working, constraints)` over
   the **frozen-only working assignment** of §2.8 step 4 — *not* over the baseline — and a
   `None` from `of` is returned as a refusal rather than falling back to `empty`.
2. `Prepared` gains `baseline: Option<Placement>`. **No `frozen: bool`** — the original proposed
   one, but a frozen occurrence never reaches `Prepared`, because `prepare`'s output is filtered
   to the movable set before `pending` is built. A flag would imply frozen occurrences are
   attempted and skipped, which is the design §2.8 rejects.
3. `prepare` filters the order to the movable set, so frozen occurrences are **absent** from
   `pending` rather than sorted to the front of it.
4. `place_one` probes `prepared.baseline` first (§2.3), without the budget check, as the pinned
   branch does.
5. `constrainedness` is **unchanged**, and its pinned-first key is inert during a repair — every
   pinned occurrence is frozen and therefore absent. The order stays total, so determinism is
   unaffected.
6. The closing `debug_assert_eq!` is **replaced** by §2.8's two assertions. The existing one is
   false for any repair whose frozen set contains an unplaced occurrence, which is the ordinary
   case rather than an edge one.

**`SolverEngine::solve` keeps its signature**, and `start` stays as it is — honestly documented
as unused by `ConstructionSolver`. Changing the trait would force every future engine to
implement repair; a free function keeps the seam that `SOLVER.md` §9 calls load-bearing.

## 3.2 `evara-db` — one read, one write, and the migration

```rust
impl Timetables<'_> {
    /// Everything a repair session needs about one timetable, in one call under `db.read`.
    pub fn repair_baseline(&self, timetable_id: Uuid, assembly: &Assembly)
        -> Result<RepairBaseline>;

    /// Writes a repaired arrangement as a NEW timetable. Never touches the source.
    ///
    /// # Errors
    /// - `NotSaveable` — cancelled, refused, or internally inconsistent
    /// - `InputsChanged` — a scheduling input moved
    /// - `ArrangementChanged` — the baseline was edited since the repair began
    /// - `Conflict` — the source timetable's own revision moved
    pub fn save_repaired(
        &mut self,
        assembly: &Assembly,
        source: Uuid,
        expected: i64,
        guard: EditGuard<'_>,
        outcome: &SolveOutcome,
        violations: &[Violation],
        flags: &[(Uuid, EntryFlags)],   // preserved per occurrence — A.8
        run: &RunProvenance,
        name: &str,
    ) -> Result<SavedTimetable>;
}

pub struct RepairBaseline {
    pub arrangement: StoredArrangement,
    pub pins: StoredPins,
    pub stamp: ArrangementStamp,
    pub flags: Vec<(Uuid, EntryFlags)>,
    /// The source's own revision at the moment of the snapshot — §3.2's guard.
    pub source_rev: i64,
}

/// Why a lock cannot be represented in the compiled problem — §2.2.1.
///
/// Replaces `StoredPins::not_in_problem: Vec<Uuid>`, which named the activity and not the
/// cause, so no screen built on it could state a remedy.
pub struct UnexpressibleLock {
    pub entry_id: Uuid,
    pub activity_id: Uuid,
    pub cause: UnexpressibleLockCause,
}

pub enum UnexpressibleLockCause {
    /// The activity is not in this school/cycle/term's compiled problem.
    ActivityNotInProblem,
    /// The slot is not in the materialised grid. Carries it, so the screen can name it.
    TimeslotNotInProblem { timeslot_id: Uuid },
    /// The room is not available to this campus. Carries it, for the same reason.
    RoomNotInProblem { room_id: Uuid },
}
```

**`StoredPins` is widened, not replaced.** `not_in_problem: Vec<Uuid>` becomes
`unexpressible: Vec<UnexpressibleLock>`. M9A's editing session is the other caller and gains the
detail for free; its *behaviour* is unchanged, because an editor still reports and does not
block (§2.2.1's asymmetry). The three causes are already distinguished inside `pins` — they are
three separate `continue`s — so this surfaces information the function computes and discards
rather than computing anything new.

**One read, one transaction.** `repair_baseline` reads the entry rows **once** inside a single
`self.db.read(…)` and derives the arrangement, the pins, the stamp, the flags and `source_rev`
from that one read — §2.10. Composing it from the existing `arrangement`, `pins`, `entries` and
`arrangement_stamp` methods would open four transactions and read the entries three times, and
the stamp could then describe a different moment from the baseline it is guarding.

`save_repaired` is a sibling of `save_generated`, not a parameter on it. The two differ in what
they write (flags, lineage), what they guard (the stamp as well as the watermark) and what they
refuse — and A.8's hard-coded flags are a correct decision for a generation that would be a bug
if made configurable.

### The three guards, in order, inside the one writing transaction

Clarification 6. `save_repaired` opens its `self.db.write(|tx| …)` with, in this order and before
any insert:

```rust
guard_edit(tx, source, guard.watermark, guard.stamp)?;   // watermark, then the stamp
guard_source_rev(tx, source, expected)?;                 // the source row itself
```

`guard_edit` (`repo/timetables.rs:1878-1893`) already does the first two: `guard_freshness` on
the watermark, then `read_stamp` compared by `ArrangementStamp::differences`, returning
`DbError::ArrangementChanged { entries_changed }`. It is reused unchanged.

**On the source revision, and being precise about what kind of check it is.** A repair writes a
*new* timetable and never updates the source row, so there is no statement whose `WHERE` clause
could carry `rev = ?`. ADR 0027's predicate form is therefore unavailable here, and it would be
wrong to claim otherwise. What makes a `SELECT rev FROM timetable WHERE id = ?` inside the write
transaction sound is the same thing that makes `guard_freshness` and `read_stamp` sound, and the
workspace already relies on it in both:

- `Database` owns a **single** `Connection` (`lib.rs:140-141`) and `write` takes `&mut self`
  while `read` takes `&self`, so no in-process read or write can interleave with the transaction.
- A read inside a transaction that then writes cannot be invalidated and still commit: SQLite
  refuses the upgrade rather than committing on a stale snapshot.

So it is a **snapshot consistency assertion in the writing transaction**, not a revision
predicate, and labelling it honestly is the point — a comment calling it an ADR 0027 check would
invite someone to copy the pattern to a mutation where it genuinely would be check-then-act. A
mismatch is `DbError::Conflict`, deliberately not `NotFound`.

What it actually catches is narrow and worth stating: the source being **renamed or deleted**
between `repair_start` and `repair_apply`. Its entries moving is the stamp's job; its inputs
moving is the watermark's. A rename alone does not invalidate a repair, so this guard is the one
of the three that could be dropped without a correctness loss — it is kept because `based_on_timetable_id`
is about to point at that row, and a lineage edge to a row that changed identity underneath is a
record nobody can read later.

### ⚠ Migration `V18` — requires approval

```sql
ALTER TABLE timetable  ADD COLUMN based_on_timetable_id TEXT
                           REFERENCES timetable (id) ON DELETE RESTRICT;
ALTER TABLE solve_run  ADD COLUMN kind  TEXT NOT NULL DEFAULT 'GENERATION'
                           CHECK (kind IN ('GENERATION', 'REPAIR'));
ALTER TABLE solve_run  ADD COLUMN scope TEXT NOT NULL DEFAULT '{}'
                           CHECK (json_valid(scope) AND json_type(scope) = 'object');
CREATE INDEX idx_timetable_based_on ON timetable (based_on_timetable_id);
```

Three columns, one index, `ON DELETE RESTRICT` per `CLAUDE.md` §3's ownership-edge rule, and
defaults that make every existing row correct without a data migration. No table rebuild, so
ADR 0020 is not engaged.

**Verified legal against the tree, 2026-10-25.** Both tables are `STRICT`
(`V17__timetable_persistence.sql:143,302`), which constrains what `ADD COLUMN` may do, and all
four statements comply: `TEXT` is a permitted `STRICT` type; the two `NOT NULL` columns carry
non-NULL defaults; `based_on_timetable_id` is nullable with a NULL default, which is what SQLite
requires of an added `REFERENCES` column while `foreign_keys = ON`; and none of them is `UNIQUE`,
`PRIMARY KEY` or generated-stored. The JSON CHECK is not a new idea either — it is
`solve_run.settings`' own predicate verbatim (`:278-279`), itself following
`constraint_instance.params` (`V16:100`).

**No change-log work.** `timetable` and `solve_run` are both already in
`change_log::TRACKED_TABLES` (`change_log.rs:216-218`), V18 adds no table, and the triggers
*"interpolate only the table name, `id` and `rev`"* — V16's note on
`student_group.children_are_disjoint` is the precedent for exactly this case. `install_all` runs
after migrations on every open, so nothing has to be recreated by hand.

### ⚠ `timetable_evaluation.kind` is a different `kind`, and a repair writes `GENERATION`

Added 2026-10-25, because the original §3.2 left it unsaid and the cheap reading of it is wrong.

`timetable_evaluation.kind` already exists and its CHECK admits **only**
`('GENERATION', 'REVALIDATION')` (`V17:341`). It is not the column V18 adds; V18's `kind` is on
`solve_run`. So a repair's save writes:

| Column | Value | Why |
| --- | --- | --- |
| `timetable_evaluation.kind` | **`GENERATION`** | It is this version's immutable, original scoring — ADR 0042 §5's meaning exactly. A repair *did* produce this timetable, so `GENERATION` is the honest value and `REVALIDATION` would be a lie about a scoring that is not a re-score of anything |
| `solve_run.kind` | `REPAIR` | **This** is the discriminator. How the version was produced is a property of the run, which is also where the seed, the engine and the statistics already live |

Doing it the other way — widening `timetable_evaluation.kind`'s CHECK to admit `'REPAIR'` — is
the trap. A CHECK constraint cannot be altered in SQLite, so it would require a 12-step in-place
table rebuild on a `STRICT` table, which **does** engage
[ADR 0020](adr/0020-in-place-table-rebuild.md) and contradicts the paragraph above. It would also
split one question across two columns. Writing `GENERATION` and reading the run for provenance
keeps V18 purely additive, which is the property that makes it safe to release.

`TIMETABLE-PERSISTENCE.md` §12 names the absence of `based_on_timetable_id` as a known
limitation — *"repair is what needs it"* — so this is the migration that record anticipated.

**Why lineage must be a column.** The alternative is encoding it in the name
(*"Draft A (repaired)"*), which is a string and not a reference: it cannot be followed, it breaks
on rename, and it cannot be checked by a foreign key.

### ⚠ What `ON DELETE RESTRICT` does to deleting a source timetable

Clarification 7, and the proposal did not address it. `Timetables::delete`
(`repo/timetables.rs:729-775`) deletes a timetable's own children deepest-first — violations,
evaluations, unscheduled rows, runs, entries — and then deletes the `timetable` row with
`Expected::Rev(expected)`. It knows nothing about a **sibling row referencing it**.

So with `based_on_timetable_id … ON DELETE RESTRICT`, deleting a source that has repaired
descendants fails on the final statement with a foreign-key violation. Because no driver type
crosses `evara-db`'s public API (`CLAUDE.md` §3), that surfaces as `InternalError` — the user
asked to delete a timetable and is told something internal went wrong, when the real answer is
*"three repairs descend from this one"*. The transaction rolls back, so nothing is corrupted; the
message is simply useless.

**The decision: keep `RESTRICT`, and add an explicit typed pre-check.** `delete` gains, as its
first statement inside the transaction and **before** any child is removed:

```rust
let descendants = count_descendants(tx, id)?;   // SELECT COUNT(*) … WHERE based_on_timetable_id = ?
if descendants > 0 {
    return Err(DbError::HasDescendants { timetables: descendants });
}
```

with the screen offering the two real remedies: delete the repairs first, or keep this one. The
alternatives are both worse and both are refused by existing rules:

| Alternative | Why not |
| --- | --- |
| `ON DELETE SET NULL` | It rewrites rows without bumping `rev`, which `CLAUDE.md` §3 forbids and which R10 / ADR 0025 §6 already made `subject.department_id` a `RESTRICT` to avoid. It would also silently destroy the lineage this column exists to record |
| `ON DELETE CASCADE` | Deleting one timetable would delete every repair derived from it. `CLAUDE.md` §3: *"Never default to `CASCADE`."* Here it is not even a defensible non-default |
| Leave it to the FK | Correct behaviour, unreadable message. The FK stays as the backstop; the pre-check is what makes the refusal explicable |

The pre-check is a `SELECT` inside the writing transaction, sound for the reason given above, and
the FK remains in place so a future writer that forgets the check still cannot orphan a lineage
edge. `idx_timetable_based_on` makes the count an index lookup.

## 3.3 `src-tauri` — eight commands

| Command | Does | Writes |
| --- | --- | --- |
| `repair_scopes(timetableId)` | what scopes this timetable offers, with the occurrence count each would make movable | no |
| `repair_preflight(timetableId, scope)` | the lock validation of §2.2 — unexpressible, contradictory, impossible-in-context — before anything is solved | no |
| `repair_start(timetableId, scope, settings)` | compiles under the lock, releases it, moves the assembly to a worker. Returns a session id | no |
| `repair_status(sessionId)` | phase and progress | no |
| `repair_result(sessionId)` | the `SolveOutcome` and the §2.6 diff | no |
| `repair_cancel(sessionId)` | sets the `AtomicBool`. Changes no state | no |
| `repair_apply(sessionId, name)` | the one write. CAS into `Applying`; `save_repaired` | **yes** |
| `repair_discard(sessionId)` | drops the session | no |

Eight, not six — `repair_preflight` and `repair_scopes` are separate because the first is the
answer to *"may I repair this at all?"* and the second to *"what could I repair?"*, and a UI
needs the second before the user has chosen a scope.

Generated TypeScript types follow from the Rust, as always — `RepairScopeView`, `RepairDiffView`,
`RepairPreflightView`, `RepairSessionView`. No hand-written entity type (`CLAUDE.md` §2).

---

# Part D — The repair state machine

## 4.1 States and transitions

```text
                   ┌──────────────────── discard ───────────────────┐
                   v                                                │
  start ──► Running ──► Finished ──► Applying ──► Applied           │
                           ^            │                          │
                           └─ failed ───┘   (the write is retryable)
```

`Applied` and `Discarded` are terminal; **nothing leaves `Applied`**. This is
`commands/session.rs`'s shape with `Saving`/`Saved` renamed, because the hazards are the same
hazards.

## 4.2 The six hazards and their guards

| Hazard | Guard |
| --- | --- |
| Applying an unfinished repair | `Running` refuses |
| Applying a cancelled or refused repair | an `applyable` predicate: `Termination::Cancelled`, `InputRejected` and `InternalCheckFailed` are all refused |
| Two applies from two clicks | compare-and-swap into `Applying` **before** any database work |
| A cancel racing a finish | `request_cancel` sets an `AtomicBool` and **no state**. One path out of `Running` |
| Applying into a different project | `ProjectEpoch`, compared before the transaction opens |
| **Applying over an arrangement that has since been edited** | `ArrangementStamp`, compared **inside** the writing transaction. **New in M9B** — a generation has no baseline to be stale about |

## 4.3 No database lock is held while the solver runs

`ARCHITECTURE.md` §3.3, and the pattern `commands/session.rs` already proves: take the workspace
lock, compile the `Assembly`, read labels, compile rules, read the baseline, take the stamp,
**release the lock**, move the owned assembly to a worker thread. `evara_db::scheduling`'s
`send_assertions` test is what keeps `Assembly: Send` true.

A repair additionally moves the baseline `Assignment` and the movable set — both plain owned data.

## 4.4 Determinism

A repair is a solve, so `CLAUDE.md` §3's rule binds it: one seeded RNG, no clock, no time limit,
every execution limit a count. Two repairs of one `(timetable, scope, seed)` over an unchanged
baseline must produce byte-identical outcomes. The movable set is computed from the baseline
before the solve and is sorted, so it contributes no ordering nondeterminism; `constrainedness`
stays total.

`Termination::Cancelled` remains the one documented non-reproducible result.

---

# Part E — Desktop UX

## 5.1 The workflow

Reached from the editor and from the read-only view, as a screen of its own — the same two-screens
decision M9A made, for the same reason: a repair session holds a compiled problem, a baseline and
a stamp from one snapshot.

1. **Choose a scope.** A radio group — whole timetable, class, teacher, room, day, selected
   lessons — with the count of occurrences each would make movable, and the count frozen. Chosen,
   never defaulted, because a repair over a scope the user did not pick is a rearrangement they
   did not ask for.
2. **Pre-flight.** Blocking lock contradictions appear here with each rule's own sentence and the
   two placements named. Repair is not offered until they are resolved. Unexpressible locks appear
   as warnings and do not block.
3. **Repair.** Progress, and a Cancel that is honest about being a request.
4. **Preview.** The §2.6 diff: counts first, then the four lists, then the score change at three
   levels, then the findings before and after. A grid view with moved cells marked, reusing
   `TimetableGrid` rather than a second renderer.
5. **Apply** — explicit, named, and never automatic. The button says it makes a new timetable and
   leaves this one alone.
6. **Discard** — equally explicit, and the default on leaving.

## 5.2 Warnings the screen owes the user

- *"Lessons outside the scope were not moved, and could not be"* — stated before the repair, not
  after, because it is the thing most likely to surprise.
- **For a room or day scope, before the repair runs:** that unscheduled lessons will not be
  scheduled by it, because an unscheduled lesson is in no room and on no day — §2.10. With the
  count, and with the two scopes that *would* place them named.
- **Pre-existing problems outside the scope**, listed as such: *"Already present, and outside the
  scope you chose."* With the rule's own sentence (ADR 0043). These are expected, they are not
  the repair's doing, and they do **not** prevent applying it — §2.9.
- Where an in-scope occurrence came back unplaced: the rule's sentence, **and** that the scope may
  be the reason — §2.5's point about `is_impossible`.
- **"This repair attempt made no changes."** — never *"nothing could be improved"*, which claims
  something the engine cannot know (§2.5).
- Where a limit stopped the run: that a limit stopped it, beside the result. A budget-truncated
  repair is applyable, and presenting a limit as a conclusion is how that becomes dishonest.
- Where a wider scope would plainly help: an offer to **start a separate repair** at that scope.
  Never a widening of the one that just ran — §2.10.
- **Blocking lock findings, with their remedy** — §2.2.1's three causes, each naming the lesson
  and the slot or room, each offering the lock's removal as the alternative. Repair is not
  offered until they are resolved, and nothing unlocks anything on the user's behalf.
- Never *"conflict-free"*. `UsabilityLine`'s existing wording — *"no confirmed clash within
  <school>"* — with the unverified counts beside it (ADR 0032 rule 4).

## 5.3 Accessibility

Held to M9A's standard, which is structural parity rather than review: the scope picker is a real
`role="radiogroup"` with arrow-key navigation; the diff is a real `<table>` with row and column
scopes; every category count is text before it is a colour; moved cells carry the change in their
`aria-label`, not only a tint; progress is a live region; and nothing is reachable by pointer
alone.

---

# Part F — Test plan and performance

## 6.1 Test plan

| Area | Cases |
| --- | --- |
| **Scope correctness** | every out-of-scope placement byte-identical after a repair, for each of the six scope kinds. **The primary guarantee, asserted per scope** |
| **The A.2 trap** | a baseline with a pre-existing clash *between two out-of-scope placements* — both survive. This is the test that would have failed under the pinned design, and it is the one that must exist |
| **Locks** | slot **and** room preserved; a locked occurrence inside the scope is not moved; contradictory locks refuse before solving; an unexpressible lock warns and does not block |
| **The pre-flight's refusal boundary** (§2.2's correction) | two clashing **locks** → refused before solving. A lock clashing with an unlocked frozen placement → refused, both named. **Two unlocked out-of-scope placements clashing → the repair proceeds**, and the clash is reported as pre-existing. That third case is the one the original criterion got wrong, and it is A.2's fixture again at the pre-flight instead of in the solver |
| **The pre-flight's level filter** | a frozen-only evaluation over a healthy timetable reports **no** finding to the user, despite producing one MEDIUM `activity-scheduled` violation per movable occurrence. Asserted on the count, so a regression that leaks them is a red test rather than a frightening screen |
| **Baseline preference** | an in-scope occurrence whose placement is still legal does not move; one whose placement is no longer legal does |
| **School isolation** | a repair of Northgate's timetable moves nothing of Southmoor's, and a Southmoor edit mid-repair is not Northgate's conflict |
| **Shared rooms** | `room-clash` is project-wide (ADR 0019), so a repair must not place an in-scope lesson into a room another school is using at those minutes |
| **Multiple campuses** | two bell schedules over the same minutes; a repair must compare **minutes**, never `timeslot_id` |
| **Multi-period lessons** | a double moves as one unit to an unbroken run; one row carries the start slot (ADR 0042) |
| **Custom cycles** | a non-weekly cycle; nothing reads a weekday |
| **Determinism** | same `(timetable, scope, seed)` twice → identical outcome, whole-outcome equality, as M6 asserts |
| **Concurrency** | an edit to the baseline between `repair_start` and `repair_apply` → `ArrangementChanged`, nothing written; an input change → `InputsChanged`; a project switch → refused on the epoch |
| **Cancellation** | cancel mid-solve → `Cancelled`, not applyable, nothing written; cancel after finish → `false`, no state change |
| **Rollback** | a failure inside `save_repaired` leaves **no** new timetable, no run, no evaluation, no change-feed entry — the shape `a_failed_save_leaves_no_timetable_at_all` already asserts for generation |
| **Provenance** | the source's `GENERATION` scoring and `solve_run` are byte-identical after a repair is applied; the new timetable's lineage points at the source; `is_manual` preserved on unmoved, cleared on moved |
| **The diff** | the five categories are exhaustive and disjoint over every fixture; counts reconcile with the two arrangements |

### Regression tests the three corrections require

These are not additions to the list above; they are the tests without which the corrections are
not actually in the product. Each names the failure it would have caught.

**C1 — scorer initialisation and the movable set** (`evara-solver`)

| Test | Asserts |
| --- | --- |
| `the_working_scorer_holds_only_frozen_placements` | after step 5, `scorer.placed_count() == frozen_placed_in_baseline`, and `scorer.assignment().placement(ix).is_none()` for every movable `ix`. **The direct test for C1**: initialising from the full baseline makes it fail immediately |
| `two_movable_occurrences_that_clash_can_both_be_re_placed` | a baseline where movable A and movable B clash with each other, both in scope. Both end placed, legally. Under a full-baseline initialisation each probe refuses the other's position and the repair returns two unplaced lessons it should have fixed — the exact movable-to-movable case |
| `a_movable_occurrence_does_not_clash_with_itself` | an occurrence whose baseline placement is still legal is re-placed **there**, with `candidate_evaluations == 1` for it. Catches a self-clash both ways: by the stale-position route, and by a future refactor that stops withdrawing before probing |
| `a_single_period_move_within_its_own_run` | a multi-period occurrence re-placed overlapping its own baseline run. The arrangement-level version of the self-clash, and the case `CLAUDE.md` §3 names for the editor |
| `a_frozen_occurrence_unplaced_in_the_baseline_is_not_reported_as_this_runs_failure` | it stays unplaced, is **absent** from `SolveOutcome::unscheduled`, and appears as `StillUnplaced` in the diff. **This is the test that fails under the old `debug_assert_eq!`**, which is why §2.8 replaces it |
| `the_scope_guarantee_holds_as_an_assertion` | `frozen_placed_now == frozen_placed_in_baseline`, and every frozen `Placement` is byte-identical, for each of the six scope kinds |
| `a_duplicated_movable_index_is_refused` | not collapsed, not placed twice |
| `an_empty_movable_set_is_refused_not_run` | `InputRejected`, distinguishable from a *no change made* result |

**C2 — hard-violation acceptance** (`evara-solver` and `evara-report`)

| Test | Asserts |
| --- | --- |
| `a_pre_existing_frozen_clash_does_not_refuse_any_candidate` | a baseline with two frozen unlocked occurrences clashing; a movable occurrence is still placed elsewhere. Proves `probe`'s scoping, and would fail if a future change widened `involving` |
| `a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable` | **The test for the blocking defect.** Status is not `Failed`, termination is not `InternalCheckFailed`, and `applyable` is true. Under `score.is_feasible()` this fails, which is the point |
| `a_frozen_pre_existing_violation_set_is_identical_before_and_after` | V3, by `ViolationKey`. The assertion that proves step 7 |
| `a_hard_violation_involving_a_movable_occurrence_is_a_defect` | V2. Injected by a deliberately wrong stub scorer, so the check is proven to fire rather than assumed to |
| `the_four_buckets_are_exhaustive_and_disjoint` | over every fixture, every HARD violation in either report lands in exactly one bucket, and the counts reconcile with `baseline_report` and `final_report` |
| `the_preview_reports_the_whole_arrangements_violations` | `SolveOutcome::score` and `hard_violations` equal `evaluate` over the full repaired arrangement — not the in-scope subset. Catches the plausible mistake of reporting only what the repair touched |
| `the_reference_and_incremental_scorers_agree_after_a_repair` | `scorer.score() == scorer.report().score` at the end of a repair, over every fixture. `differential.rs`'s gate, extended to the frozen-only starting point |

**C3 — unexpressible locks** (`evara-db`)

| Test | Asserts |
| --- | --- |
| `a_lock_on_an_activity_outside_the_problem_blocks_the_repair` | `InputRejected` with `ActivityNotInProblem`, naming the lesson |
| `a_lock_on_an_unmaterialised_slot_blocks_the_repair` | `TimeslotNotInProblem`. The realistic case after a grid change, and the dangerous one: without it the occurrence is movable and the lock is **moved** |
| `a_lock_on_a_withdrawn_shared_room_blocks_the_repair` | `RoomNotInProblem` |
| `an_unexpressible_lock_is_never_cleared_by_the_pre_flight` | the `is_locked` flag on the source is untouched after a refusal. The pre-flight must not unlock to let itself proceed |
| `the_editor_still_only_warns` | M9A's behaviour is unchanged by the widening of `StoredPins`. The asymmetry is deliberate, so it is pinned |

**Clarifications**

| Test | Asserts |
| --- | --- |
| `the_baseline_snapshot_comes_from_one_read` | the arrangement, pins, stamp, flags and `source_rev` are mutually consistent; a concurrent write between them is not representable because there is one transaction |
| `a_room_scope_does_not_schedule_an_unplaced_occurrence` | and the day scope likewise — §2.10's two surprising rows, pinned so a later "helpful" change cannot quietly widen them |
| `the_movable_set_is_never_widened` | no outcome places an occurrence outside the derived set, for each scope kind |
| `deleting_a_source_with_descendants_is_refused_readably` | `DbError::HasDescendants`, **not** `InternalError`, and the source and its descendants all survive |
| `a_cancelled_repair_is_not_applyable_and_writes_nothing` | and a budget-limited one **is** applyable, with the limit carried into the preview |
| `a_scope_identifier_from_another_school_is_refused` | including a room that resolves through `room_campus` but is not this school's |

## 6.2 Performance acceptance criteria

Grounded in figures already measured rather than invented. Sources: `commands/session.rs`'s
*"~900 ms a 2 000-occurrence solve takes"*, and M9A's `editing_budget.rs`.

| Measurement | Budget (debug) | Basis |
| --- | --- | --- |
| Baseline read + stamp, 2 000 occurrences | < 150 ms | one indexed read of entries plus `(id, rev)` |
| Pre-flight lock validation | < 50 ms | one reference `evaluate` over the frozen set only |
| Starting scorer, clean baseline | **< 20 ms** | measured 3.8 ms at 1 600 occurrences |
| Whole-timetable repair, 2 000 occurrences, clean baseline | **< 2 s** | a full solve is ~900 ms; a repair adds one baseline probe per occurrence |
| Narrow-scope repair (one teacher, ~40 occurrences) | **< 250 ms** | the frozen set is placed into the scorer, not searched |
| Diff of two 2 000-occurrence arrangements | < 100 ms | a linear comparison; M8 projects 2 000 occurrences in 3.8 ms |
| **Heavily conflicted baseline** (≥ 10 000 hard penalties) | **< 10 s**, and reported | see below |

**The heavily-conflicted case is the one to watch, and M9A measured why.** An arrangement holding
61 640 hard penalties costs **1.5 s** just to build a scorer, and a release build barely helps
(1.51 s) because the time is spent **allocating `Violation` structs with formatted `detail`
strings**, not comparing intervals. A repair builds one scorer and then probes; at that violation
count the budget above is the honest expectation and the UI must show progress rather than appear
hung. Making `Violation` defer its formatting is the fix and it is **M5's surface, not M9B's** —
listed in §7 as a risk, not adopted as a task.

Every figure is to be **measured and printed** in a budget test, as `editing_budget.rs` and
`evara-report`'s `budget.rs` do, with loose bounds as hang detectors. A tight timing assertion on
CI hardware is a flaky test, and a flaky test is worse than a recorded number.

---

# Part G — Risks, boundaries, and staging

## 7.1 Risks and limitations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| **No minimal-disruption guarantee.** Greedy construction cannot reconsider a displacement | by design | Stated in the UI, in this proposal and in the record it earns. Real minimisation is local search, which the MVP excludes |
| **A positional scope does not close under constraints.** Repairing "Monday" may fail where repairing everything would succeed | medium | §2.1. Reported with the reason; never worked around by widening the scope the user chose |
| **A heavily conflicted baseline is slow**, and the cost is allocation rather than compute | medium | §6.2. Progress and cancellation; the real fix is M5's |
| **Repair over a stale baseline is refused, not merged** | by design | ADR 0047's position, one level up. Merging two arrangements is unspecified |
| **A new version per repair grows the list** | low | ADR 0042's versioning is already row-based, and the user names each one. The Duplicate affordance already taught this shape |
| **`save_repaired` duplicates some of `save_generated`** | low | Deliberate. Their guards and their flag handling differ, and A.8 shows what a shared parameterised path would have to get right in both directions |
| **Clearing `is_manual` on a moved occurrence loses a fact** | low | It is the correct fact to lose: the solver chose that placement. The change feed records both writes, so the history survives even though the flag does not |
| **A blocking unexpressible lock can leave a user stuck** | medium | §2.2.1. The lock must be removed or the lesson re-placed before repair is available, and on a timetable with many stale locks that is real work. Mitigated by naming each remedy and offering removal in one click — and the alternative is a lock silently moved or lost, which is not a trade this product makes |
| **A repair's statistics are not comparable to a generation's at the same seed** | low | §2.3: the baseline probe spends one candidate evaluation per in-scope occurrence. Recorded in `solve_run.statistics` and in `solve_run.kind = 'REPAIR'`, so a reader can tell the two apart rather than comparing them by mistake |
| **§2.9's partition is a second place that reasons about violations** | low | It reasons about *attribution*, never legality: it partitions a report the reference evaluator produced, by set membership and `ViolationKey`. No rule is restated, which is the line `CLAUDE.md` §2 draws. The risk is a future reader mistaking it for a rules engine, which is what ADR 4 of §8.3 exists to prevent |

## 7.2 In M9B

Scope selection and the movable set · the **frozen-only working assignment** (§2.8) · frozen
placements preserved byte-identically, asserted · pre-flight lock validation, **blocking** for an
unexpressible lock (§2.2.1) · request validation (§2.10) · the baseline-first probe · **scope-aware
verification V1–V3** (§2.9) · the repair session with its six guards · the **one-read** baseline
snapshot · `delete`'s descendant pre-check · the diff, the four violation buckets and the preview ·
explicit apply as a new version · `V18` · the eight commands · the test plan including the twenty
regression tests of §6.1 · the budget test.

## 7.3 Not in M9B

| Deferred | Where it belongs |
| --- | --- |
| `minimise-disruption` (OPS-001) as a real constraint type | with local search, when a score can actually be minimised |
| Local search of any kind | `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 — out of the MVP |
| Interactive move **suggestions** (*"here are three places this could go"*) | a later milestone. `edit_candidates` already returns ranked verdicts, so it is additive over data that exists |
| In-place apply | §2.4. Offer it only if users ask, with the diff as evidence |
| Published/archived lifecycle, scenario comparison across more than two versions | `TIMETABLE-PERSISTENCE.md` §12 |
| Deferred `Violation` formatting | M5's surface — §6.2 |
| Cross-school coordinated repair | ADR 0032 §4 defers cross-school comparison for generation too |

## 7.4 Staging

| Stage | Delivers | Verifiable by |
| --- | --- | --- |
| **B1** | `construct` accepts a baseline and a movable set; the **frozen-only** working assignment (§2.8); the baseline-first probe; the rewritten invariants. Headless, no UI | `evara-solver` tests + the A.2 trap test + **all eight C1 regression tests** + determinism |
| **B2** | `repair_baseline` as one read, `save_repaired` with its three guards, the widened `StoredPins`, the blocking lock pre-flight, `delete`'s descendant check, `V18`. Headless | `evara-db` tests: rollback, lineage, flags, the three guards, **all five C3 tests**, the snapshot and delete clarifications |
| **B3** | the diff in `evara-report`, **and §2.9's partition and V1–V3 acceptance rules** | the five categories exhaustive and disjoint over every fixture; **all seven C2 tests** — in particular `a_repair_of_a_timetable_with_out_of_scope_hard_violations_is_applyable`, which is the gate on the whole milestone being usable |
| **B4** | the session, the eight commands, generated types | command-layer tests on the six hazards |
| **B5** | the desktop screen | frontend tests: scope picker, preview, explicit apply, keyboard parity |
| **B6** | the budget test, `TIMETABLE-REPAIR.md`, the ADRs | measured figures; records |

B1 and B2 are independently valuable and independently testable, which is what makes this
stageable. A CLI `evara repair --project <p> --timetable <id> --scope …` that writes nothing falls
out of B1 + B2 for free and is worth shipping as the headless proof, exactly as `evara generate`
was.

---

# Part H — What needs your approval

Four items. Nothing below is actioned.

## 8.1 ⚠ Migration `V18` — approval required

Three columns and one index (§3.2): `timetable.based_on_timetable_id`,
`solve_run.kind`, `solve_run.scope`. Additive, defaulted so every existing row is already
correct, no table rebuild. **A released migration is immutable**, so this is the decision that
cannot be taken back cheaply.

## 8.2 ⚠ ADR changes — approval required

| Record | Change | Kind |
| --- | --- | --- |
| **ADR 0042** | §3's *"a save always inserts"* now has a second inserting writer. No rule changes; the record gains a repair row | **extension**, no amendment |
| **ADR 0045** | §4's *"the caller populates `pinned`"* gains a second caller. No rule changes | **extension** |
| **ADR 0047** | the stamp becomes a guard on a *solve*, not only an edit. The reasoning transfers unchanged | **extension** |
| **`SOLVER.md` §7** | **amended in three places, not two.** Its two mechanisms both change — out-of-scope activities are *"treated as pinned for this run"*, which A.2 shows drops them, and the disruption penalty, which §2.3 defers. Its **scope list** changes too: §7 offers *"a term, a year level, a teacher's load, an explicit selection"*, and §2.1 offers whole-timetable, class section, teacher, room, cycle day and explicit occurrences. A **term** is not a repair scope at all — a timetable is already per `(cycle, term)`, so a term either is the whole timetable or is another one — and a **year level** is a derived set this proposal did not include. Both the mechanisms and the menu need the decision | **amendment — the one that needs a decision** |
| **`CONSTRAINTS.md` §3** | `minimise-disruption` moves from `M1` to a later phase, with the reason | **status change** |

### 8.2.1 What round 2 changed in the records

| Record | Change | Kind |
| --- | --- | --- |
| **`V18`** | **unchanged, still additive.** §2.8 deliberately avoids an eighth `UnplacedCause`, which would have required altering `solve_run_unscheduled.cause`'s `CHECK` — a table rebuild on a `STRICT` table. The only V18-adjacent change is the **typed descendant pre-check in `delete`**, which is Rust, not DDL | no migration change |
| **ADR 0045** | still an extension, and now a **stronger** one: §2.2.1 makes an unexpressible lock blocking, which is the first place the workspace refuses rather than reports one. Worth a sentence in the new ADR rather than an amendment to 0045, since 0045's rule — a lock is a property of a placement — is untouched | extension |
| **ADR 0047** | unchanged as an extension, with §2.10's requirement that the stamp and the baseline come from **one read** added as the condition under which the extension is sound | extension |
| **`SOLVER.md` §7** | the amendment grows a third item: §7's `start = Some(current)` is right, but it must be read as *"start from the frozen subset of current"*, not *"start from current"*. That is C1 | **amendment** |
| **`CONSTRAINT-ENGINE.md`** | no change. §2.9's fix is an interpretation of the reference report in `evara-report`, not a new rule, a new level or a change to any evaluator | none |

## 8.3 Five new ADRs this milestone would earn

1. **Repair scope is a movable set, not a set of pins** — §2.1, and the A.2 finding that forces it.
   Now also carrying §2.8: the working arrangement holds the frozen placements *alone*.
2. **A repair produces a new version and is never applied in place** — §2.4.
3. **Baseline preference is candidate ordering, not a constraint** — §2.3, and why
   `minimise-disruption` waits for local search.
4. **A repair's verification is scope-aware, and a pre-existing violation is not a defect** —
   §2.9. The one that earns an ADR on its own merits: it records why `score.is_feasible()` is the
   wrong gate for a repair, and why V2 + V3 are stronger rather than more permissive. Without the
   record, the next person to read `status_of` will "fix" the repair path back into the defect.
5. **An unexpressible lock blocks a repair and only warns an edit** — §2.2.1. The asymmetry is
   deliberate and looks like an inconsistency, which is precisely what an ADR is for.

## 8.4 One question I cannot answer for you

**Should a narrow scope be allowed to report failure where a wider one would succeed?** §2.1 says
yes and reports it. The alternative — offering *"repairing Monday alone cannot fix this; widen to
the whole week?"* — is friendlier and is a scheduling judgement about the user's intent. I have
specified the strict behaviour because it is the one that cannot surprise, but the softer version
is defensible and would change the UX in §5.

---

# Part I — Final decision log

Round 2's corrections, each with the source that settled it. Nothing here was decided from
reading this document.

| # | Decision | Why | Source |
| --- | --- | --- | --- |
| **D8** | The working assignment holds **frozen placements only**; movable occurrences are cleared from a clone of the baseline before the scorer is built | Initialising from the full baseline reconsiders each movable occurrence against the stale positions of the others, and makes a pre-existing movable-to-movable clash refuse both — because `allowed` is `false` whenever the result *carries* a hard violation naming the occurrence, not only when it adds one | `incremental.rs:80-90`, `:247-281`, `:291-299`; `placement.rs:223-236` |
| **D9** | A frozen occurrence **unplaced in the baseline** is not reported as this run's failure, and `construct`'s closing invariant is rewritten over the movable set | `unplaced.len() + scorer.placed_count() == total` is false whenever the frozen set contains an unplaced occurrence, which is ordinary. The old assertion would fire on the common case | `construct.rs:212-216`; `placement.rs:263` |
| **D10** | **No eighth `UnplacedCause`.** Out-of-scope unplaced occurrences are `StillUnplaced` in the diff and absent from `solve_run_unscheduled` | A new cause needs a new value in that table's `CHECK`; a `CHECK` cannot be altered in SQLite, so `V18` would need a rebuild on a `STRICT` table and stop being additive. The approved migration stays approved | `V17:314-321`, `:328`; ADR 0020 |
| **D11** | Candidate acceptance needs **no change**. `probe` is already scope-aware | `probe` sets `blocking` from `involving(activity)`, which runs each rule's `evaluate_involving` for the probed occurrence only. A frozen-to-frozen violation names neither the occurrence nor its candidate | `incremental.rs:247-281`, `:345-365`; `CLAUDE.md` §4 item 6 |
| **D12** | The **final verification is replaced** for repair: V1 readability, V2 no HARD violation involving a movable occurrence, V3 the frozen HARD set identical to the baseline's by `ViolationKey`. `status_of` and `generate_with` are untouched; `repair_with` is a sibling | `status_of` gates on `report.score.is_feasible()`, and `is_feasible` is `self.hard == 0` over the **whole arrangement**. Every repair of a timetable with an out-of-scope hard violation would have returned `InternalCheckFailed` and been unapplyable. V2 + V3 are strictly stronger than the global check, because V3 catches a frozen placement being disturbed, which feasibility cannot distinguish | `lib.rs:227-272`, `:324-336`; `outcome.rs:93-95`; `constraints.rs:462-466`, `:490`, `:507-514` |
| **D13** | An **unexpressible lock blocks** the repair, with a remedy per cause, and `StoredPins` carries the cause | `pins` drops such a lock from `Problem::pinned` on any of three resolution failures. If the activity resolves but the slot or room does not, the occurrence is movable with no pin — so a repair would **move** the lock, not merely fail to honour it. The rows all still exist (every FK is `RESTRICT`), so the lock is physically insertable but was never checked against the repaired arrangement — writing it would produce a placement no evaluation scored | `repo/timetables.rs:2088-2117`; `V17:197-206`; ADR 0032 rule 4 |
| **D14** | The baseline, flags, stamp and `source_rev` come from **one read**; the `Assembly` is compiled **first** | Four separate `db.read` calls can describe four moments, so the stamp would guard an arrangement the repair was not computed from. And because `timetable_entry` is in `SOLVE_OUTPUT_TABLES`, an input change between a baseline-first read and a later compile is invisible to **both** guards | `repo/timetables.rs:477`, `:729`, `:1592`, `:2088`; `change_log.rs:285-314` |
| **D15** | The source-revision check is a **snapshot assertion inside the writing transaction**, not an ADR 0027 predicate, and is labelled as such | A repair never updates the source row, so there is no `WHERE` clause to carry `rev = ?`. It is sound for the reason `guard_freshness` and `read_stamp` are — one owned `Connection`, `write` takes `&mut self`, and SQLite refuses to commit a write upgraded from a stale read. Calling it a predicate would invite the pattern being copied to a real mutation | `lib.rs:140-141`, `:562-575`; `repo/timetables.rs:1878-1893`; ADR 0027 |
| **D16** | Lineage keeps `ON DELETE RESTRICT`, and `delete` gains a **typed descendant pre-check** | `delete` knows nothing about a sibling referencing it, so the FK fires on the last statement and surfaces as `InternalError` — a useless message for a legitimate refusal. `SET NULL` would rewrite rows without bumping `rev`; `CASCADE` would delete a user's repairs | `repo/timetables.rs:729-775`; `CLAUDE.md` §3; ADR 0025 §6 |
| **D17** | *No change made* replaces *"nothing could be improved"*, and applyability is a closed enumerated list | A greedy construction that changed nothing has established nothing about what exists — the same discipline `is_impossible` enforces per occurrence. And `Partial` must stay applyable, since §2.8 shows a repair of a timetable with out-of-scope unplaced lessons is always `Partial` | `outcome.rs:617-631`; `lib.rs:319-336` |
| **D18** | The movable set is **never** widened automatically; a wider scope is a **new session** | A repair that moved an excluded lesson is worse than one that failed, because the user cannot review what they did not ask about. It also settles §8.4 without changing the engine: strict in the solver, friendlier as a second button | §2.1, §2.10 |

**What round 2 did not change:** the movable-set architecture, frozen out-of-scope and locked
placements, baseline-first ordering, a new version per repair, explicit preview and approval, the
additive `V18`, the three guards, and the exclusion of local search and `minimise-disruption`.
D12 is the only correction that would have shipped as a broken feature; D8, D9 and D13 would have
shipped as wrong results, and D13 as a lost lock.

---

## Appendix — verification state

No code was written, so there is nothing to verify beyond the branch:

```
branch              feat/m9b-timetable-repair
base                334a21d  (Merge pull request #33 … )  = origin/develop
PR #33 included     yes — 7c03b2d is an ancestor
tracked files       clean — no tracked file is modified, added or deleted
this file           UNTRACKED (docs/M9B-REPAIR-PROPOSAL.md), never committed
stash@{0}           intact ("ROADMAP Phase 1E test counts")
```

`target/` untouched, nothing force-pushed, `develop` not altered, no advisory suppressed. The
pre-existing `zip 8.6.0` advisories (RUSTSEC-2026-0368/0369) remain `develop`'s and are unrelated
to this branch, whose `Cargo.lock` is byte-identical to it.

`npm run check` was **not** run for the 2026-10-25 re-examination and is not claimed: no Rust,
TypeScript or SQL was touched, so there is nothing for `fmt`, `clippy`, `tsc` or either test
runner to have an opinion about. The verification that *was* done is source-reading, and every
file and line it rests on is cited inline.

### Why this file looked lost

The original line above read *"working tree clean"*, which was true of the code and false of this
document: it has **never been committed on any branch** — `git log --all -- docs/M9B-REPAIR-PROPOSAL.md`
is empty — so it existed only as a single untracked working-tree file. Anything reading the
repository through Git rather than the filesystem therefore could not see it, and it would have
been destroyed by a clean or a checkout that swept untracked files. It was recovered intact, not
reconstructed. **Committing it is the fix**, and the §0 note about M9A's proposal being written to
a gitignored folder and lost is the same lesson arriving twice.
