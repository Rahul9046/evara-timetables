# Manual Timetable Editing and Locking

What M9A lets a timetabler change by hand, what refuses them and in whose words, and why the
editor cannot disagree with the generator about whether a placement is legal.

Shipped in **M9A**. This is the first milestone in which the interface **writes** a placement.

[ADR 0045](adr/0045-a-lock-is-a-property-of-a-placement.md),
[ADR 0046](adr/0046-an-edit-changes-the-version-it-is-open-on.md) and
[ADR 0047](adr/0047-a-cross-session-guard-is-an-exact-placement-list.md) are the three
decisions it earned. This file is the **behaviour**.

---

## 1. The shape of it

```
evara-core      vocabulary:  ConstraintType::ActivityFixed            <- the ninth type
                             Problem::pinned                           (already existed, now used)
   ^        ^
   |        └── evara-constraints  pinning.rs                        <- the evaluator
   |            differential.rs    pinned_variant                     <- the gate extended
   |            IncrementalScorer::probe                              (M6 — it was already the
   |                                                                   move validator)
   └── evara-db  Timetables::{place, move_entry, unschedule, set_lock,
   ^                          duplicate, pins, arrangement_stamp}
src-tauri       commands::edit_session   the session, the undo stack
                commands::edit           13 handlers
   ^
apps/desktop    ipc/edit.ts              13 wrappers, no legality
                app/timetables/          EditTimetable · TimetableGrid's `editing` prop
```

**No migration.** `V17` already shipped `timetable_entry.is_locked` and `is_manual`, both
uniqueness rules, the `BEFORE UPDATE` cross-cycle trigger, all three indexes and the change-log
triggers. M9A adds no table, no column, no trigger and no index.

One thing `V17` did *not* do was give the cross-cycle trigger a user-facing error path: a bare
`RAISE(ABORT)` reaches the interface as *"this campus file may be damaged"*. So the repository
asks first and names the remedy, with the trigger as the backstop — the same division of labour
the membership triggers already use.

---

## 2. The one thing this milestone is about

`CLAUDE.md` §2's most important rule:

> the constraint evaluators in `evara-constraints` are shared by automatic generation,
> timetable scoring, manual move validation and violation explanation. One implementation,
> four callers. If generation and the editor can ever disagree about whether a placement is
> legal, the product is broken.

M9A is the milestone that adds the third caller, and the readiness audit named it this
milestone's defining risk. The answer is that **the move validator already existed** and M9A
wrote none of it.

`IncrementalScorer::probe` is what M6 built for candidate evaluation, and it is precisely a move
validator:

- it **withdraws the occurrence first**, then places it at the candidate, then restores — so
  moving a lesson one period later does not report it as clashing with itself, which is the
  thing a hand-written move validator gets wrong;
- its `allowed` is stricter than `SOLVER.md` §6's wording in exactly the way an editor needs: a
  drop target that was *already* broken is not a legal drop target;
- it is the scorer the generator used, so the two cannot disagree.

So `edit_candidates` and `edit_validate` **call** `probe`. There is no `validate_move`
implementation anywhere in the workspace, and the four records that promised one as M11's work
have been corrected to say so.

---

## 3. The ninth constraint type

`ConstraintType::ActivityFixed`, `type_key = "activity-fixed"`, HARD, no parameters, in
`crates/evara-constraints/src/pinning.rs`. The hard set is now eight of nine, which is
[SCHEDULING-ARCHITECTURE-DECISIONS.md](SCHEDULING-ARCHITECTURE-DECISIONS.md) §6's split **in
full** for the first time. [CONSTRAINT-ENGINE.md](CONSTRAINT-ENGINE.md) §3 had deferred it in
these words — *"when locks land, `activity-fixed` arrives with its evaluator, its schema, its
tests and its row in CONSTRAINTS.md, and the hard set becomes eight"* — and locks have landed.

### An unplaced pinned occurrence is not a violation of it

This is the decision in the rule's shape and the one thing not to change without re-reading the
argument.

A pin says *where*, not *that*. `activity-scheduled` already says *that*, at MEDIUM, so an
incomplete arrangement is **incomplete** rather than infeasible. If this rule also fired on an
unplaced pinned occurrence, then `IncrementalScorer::empty` over a pinned problem would start
**infeasible** — and so would every intermediate state of a construction. That is exactly the
failure M5 refused to introduce when it kept completeness at MEDIUM, and a pin is not a licence
to reintroduce it.

So: placed away from the pin is a HARD violation; placed at the pin is silence; not placed is
silence here and a MEDIUM `activity-scheduled` violation there. The test is named
`an_unplaced_pinned_occurrence_is_incomplete_and_not_infeasible`.

### A pin names a slot **and** a room

`Placement` is the unit the solver moves, so it is the unit the pin protects. A repair that kept
a locked lesson's period and took its room away has moved it.

### The gate now covers it rather than running beside it

The fixture catalogue pins nothing — pins come from a timetable, which is an output — so the
existing differential property test could never have exercised `activity-fixed`.
`differential.rs` gained `pinned_variant`, which pins one occurrence where a crowded arrangement
put it and a second somewhere it is not, and runs the union property, the running-score property
and the at-most-two-occurrences precondition over every fixture in that state.

`IncrementalScorer::of` **already existed** and needed no addition; what M9A added is the
equivalence test the brief asked for — `of` equals `empty` + `place` per entry, over every
fixture, for score, assignment and per-rule counts — plus the pinned variant of the same.

---

## 4. A lock is a property of a placement

`timetable_entry.is_locked`, per version, never `activity.is_locked`.
[ADR 0045](adr/0045-a-lock-is-a-property-of-a-placement.md) is the record and the reasoning; the
short form is that several timetables coexist for one `(cycle, term)`, so *"is this lesson
locked?"* has no single answer, and a lock with no placement in it does not say which slot or
which room it means.

The loader compiles **no** pins. `Problem::pinned` is populated by the editing session from
`Timetables::pins`, because a lock is an output of a solve and a `Problem` is compiled from
inputs only.

---

## 5. The seven writes

Each is one transaction in `evara-db`, with every guard a predicate on the write.

| Method | Shape | Revision |
| --- | --- | --- |
| `place` | `INSERT`, `is_manual = 1` | none — a creation carries no precondition |
| `move_entry` | **`UPDATE`** of `timeslot_id` / `room_id`, never delete-and-insert | the entry's |
| `unschedule` | `DELETE` | the entry's |
| `set_lock` | `UPDATE is_locked`, and **not** `is_manual` | the entry's |
| `duplicate` | copies the timetable and its entries; **no run, no evaluation** | the timetable's |
| `pins` | locked entries → `Vec<Option<Placement>>` | read |
| `arrangement_stamp` | the cross-session guard's input | read |

A move is an `UPDATE` so the placement keeps its identity, its lock and its creation time, and
the change feed records one entry for one user action — slot and room together are **one** feed
entry, not two. `set_lock` does not touch `is_manual`: locking a generated lesson does not make a
human have placed it.

**`is_manual` is a high-water mark, not a position.** `move_entry` and `place` set it and nothing
clears it, so once a human has moved a placement the record that a human was involved stays true
— **including after an undo**, because undoing a move is also a person deciding where the lesson
goes. Clearing it on an undo would make the grid claim the solver chose a cell a person chose.
The M9A audit's own test assumed the opposite and failed, which is how the question got asked;
`an_unlock_a_move_and_two_undos_restore_the_lock_and_the_slot` now asserts it with the reasoning.

---

## 6. Four refusals, four remedies

| Refusal | Means | What the screen offers |
| --- | --- | --- |
| `Locked` | the placement is pinned in this timetable | **Unlock it** |
| `Conflict` | somebody changed *this* placement | reload |
| `ArrangementChanged` | somebody changed *another* placement of this timetable | **Reopen** |
| `InputsChanged` | a scheduling input moved since the session opened | **Reopen** |

Nothing was written in any of them, and each message says so first.

`ArrangementChanged` is [ADR 0047](adr/0047-a-cross-session-guard-is-an-exact-placement-list.md)
and is the guard `expectedRev` cannot be: a second window that moves a **different** entry of
the same timetable leaves this session's target row at the revision it was read at, so a
revision check *passes* — and every verdict the first session has shown the user is now false.
Nothing else catches it. The stamp is the exact `(entry id, rev)` list; a count-and-sum digest
collides on unschedule-then-place and a scoped change-log watermark cannot see the deletion of
an entry it never heard of.

The last two end the **session** rather than the edit: the compiled problem, the verdicts on
screen and the undo stack were all computed from one snapshot, so they are stale together and a
retry would answer with the old one.

**A locked placement is refused at the command layer, before any rule runs**, with its own
`SetupError::Locked` so the screen can offer *Unlock*. `activity-fixed` is the registry's
backstop for anything that tries anyway — routing the refusal through the rule engine would make
unlocking feel like overriding a clash, which is the opposite of what it is.

---

## 7. The session

`commands::edit_session` holds one session per open editor: the compiled `Assembly`, the
arrangement, the stamp, the undo stack and the project epoch it began under. Four design points.

1. **It caches no scorer.** One is built per command from the entries it holds — one reference
   evaluation and **no database access**. A cached scorer would be a second statement of what is
   placed, and the moment it disagreed with the rows every verdict would be about a timetable
   nobody can see. (It also avoids a self-referential struct, but that is the lesser reason.)
2. **The undo stack is in Rust.** An inverse command is a statement about scheduling state and
   its precondition is a revision the database owns; a stack in the view layer would be a second
   place deciding what reversing a move means.
3. **Each applied command records the revision its own write produced**, not the one read before
   it — the naive version fails every undo, because the forward write consumed that revision.
4. **A refused undo clears both stacks.** Everything below it was computed against a state that
   no longer holds.

Lock order is **session then workspace**, in that order and never the other way. No other module
takes a session lock, so there is no cycle to deadlock on, and the session's own `Mutex` is what
serialises two commands of one session — which they must be, or their stamps and their undo
stacks would interleave.

`AppState.edits` is cleared on every project create, open and close, so a session can never
outlive the campus file it was compiled against; `require_epoch` is the second half of that.

---

## 8. `edit_candidates` answers a whole panel in one call

One scorer, one `probe` per slot, every slot of the lesson's own bell schedule — **including**
the non-teaching and the unavailable ones — each carrying the rule's own sentence for why it is
refused.

A pre-filter is right for a generator, which is choosing what to try, and wrong for an editor,
which is answering a user who can see the cell. *"Break is not a teaching period"* and
*"A. Okafor is unavailable on Day 2"* are the answers that cell deserves.
`Problem::timeslots` keeps the non-teaching slots for exactly this (ADR 0033 §4).

One call rather than sixty because a realistic panel is forty to sixty cells and sixty IPC round
trips per drag is not a design. Each probe is microseconds.

The rooms offered are `room_candidates` — the loader's record of
`RoomFact::is_candidate_for`, which is the **one** implementation of the room candidate domain,
called rather than restated. A room is part of the **proposal**, not a hint: changing it re-asks
the backend for the whole panel, because `room-clash` and the candidate domain both depend on it.

---

## 9. The interface computes no legality, and is given no way to

`ipc/edit.ts` wraps all thirteen commands and the one function in it with any logic is
`verdictAt`, a lookup into an answer Rust already gave. A position the backend sent no verdict
for returns `null` — not `allowed: false`, and emphatically not `allowed: true`: no verdict is a
*missing answer*, and rendering it as either would be deciding legality in the view layer.

`TimetableGrid`'s `editing` prop turns cells into pick-up and drop targets and decides nothing:
every tinted cell is one Rust called legal, and every refused one carries the refusing rule's
sentence in its `aria-label`. A panel of a **different** bell schedule offers no drop target at
all, which is the truthful answer rather than a dead cell — a lesson may only sit in slots of
the schedule its class resolved to, and `duration-contiguous` owns that; the grid simply has no
verdict to render.

`EditTimetable` is the screen: the duplicate affordance with its sentence, the undo/redo bar, the
unplaced tray, the held-lesson panel with its room picker, the grid, and the findings. It holds
no verdict state it could infer anything from — `candidates` is replaced wholesale, never edited
in place, because a half-updated verdict grid would tint a cell from one question with the answer
to another.

---

## 10. The keyboard reaches everything the mouse does

A grid that can only be edited by dragging is a grid half the people who have to use it cannot
edit, so parity is structural rather than asserted-by-review.

| Key | On a cell |
| --- | --- |
| Enter / Space | pick up the lesson here, or drop the one in hand |
| Escape | put down what is in hand, changing nothing |
| Delete / Backspace | unschedule the lesson here |
| `l` | lock or unlock the lesson here |
| arrows, Home, End | move the focus, as in the read-only grid |

The accelerators act on the **first** lesson of a cell, which is why every lesson also carries
its own **Pick up / Lock / Unschedule** buttons: a shared cell's second lesson — ordinary in a
master grid, a clash in a teacher's — is otherwise mouse-only. There is a test named for it.

A drop verdict never relies on colour: a legal target is a ring **and** a tint, a refused one a
hatch **and** a tint, and both say which they are out loud. The unplaced tray exists because an
unplaced occurrence is in no cell and so cannot be picked up from one.

---

## 11. Editing is in place

[ADR 0046](adr/0046-an-edit-changes-the-version-it-is-open-on.md). An edit changes the version
it is open on; nothing forks. ADR 0042 §5 keeps the `GENERATION` scoring immutable precisely so
re-scoring can never misrepresent how a timetable was generated — a guarantee that is only
needed *because* the arrangement beneath it can change.

**Duplicate** is the affordance that makes that safe, and it is offered at the top of the editor
with the sentence that explains it, before the first drag rather than after it. The copy claims
no `solve_run` and no generation scoring, because it was not generated.

An undo is a **new transaction**, not a rollback, so the change feed records both the move and
its reversal — which is what an audit trail means.

---

## 12. What the editor says, and what it refuses to say

- **It never says a lesson cannot be placed.** The tray says how many occurrences are not on the
  grid and that a full tray *"is not a statement that these cannot be placed"*. Only a solver can
  say a timetable exists; `evara_core::lessons`' rule and the solver's mirror of it both bind
  here.
- **It never says a timetable is conflict-free.** `UsabilityLine` says *"no confirmed clash
  within <school>"*, with the unverified counts beside it, exactly as the read-only view does.
- **A warning is never a refusal.** `MoveVerdictView.warnings` carries the medium and soft
  violations a placement would newly introduce and is **never** a reason to refuse one.
- **A timetable that is readable but incomplete says so.** `notInProblem` names stored
  placements the campus file no longer holds; they are not in the grid and cannot be edited, and
  the screen says that what is on screen is less than what was saved. `pinsNotInProblem` adds
  that a locked one among them cannot be given to a later repair.
- **There is no degraded editing mode.** `edit_open` refuses when the problem cannot be compiled,
  and the read-only screen's **Edit this timetable** is disabled with the reason beside it. An
  editor that could validate nothing would be offering to write placements nothing had checked.

---

## 13. What M9A deliberately does not do

Honoured by omission — nothing in the tree implements any of it:

- **repair**, or partial regeneration against a baseline;
- `minimise-disruption`, baseline-first ordering, or any change to the solver's `start`
  parameter;
- a scope filter on generation;
- cross-school reservation consulting for shared entities (the multi-school audit's row 8 —
  `room-clash` remains a per-`room_id` rule for the whole project, as ADR 0019 has it);
- an `activity_pin` table, a lock on `activity`, or a project-level lock of any kind.

---

## 14. Measured figures

[OPEN-DECISIONS.md](OPEN-DECISIONS.md) R5 names *"a 2,000-activity secondary school with
electives"* as the realistic upper end and asks for its numbers to be treated as a budget. M8
paid the projection half in `evara-report`'s `budget.rs`; M9A pays the **editing** half, in
`evara-constraints`' `editing_budget.rs`.

A pick-up costs exactly two things, and `edit_candidates` does no database work at all: one
`IncrementalScorer::of` over the current arrangement — a single reference evaluation — and one
`probe` per slot of the lesson's bell schedule.

| Arrangement | Occurrences | Hard penalties | Scorer | Panel of 40 cells | Per cell |
| --- | --- | --- | --- | --- | --- |
| A clean timetable | 1 600 | 0 | **3.8 ms** / 2.1 ms | **298 µs** / 266 µs | 7.4 µs / 6.6 µs |
| Over-subscribed worst case | 2 000 | 61 640 | 1.58 s / 1.51 s | 2.3 ms / 2.5 ms | 58 µs / 63 µs |

Debug figure first, release second, because `cargo test` builds without optimisation and that is
the number a developer sees.

**The cost is dominated by what is *wrong* with the arrangement, not by its size**, and that is
the figure worth carrying forward. A realistic pick-up — a school that has just generated and
saved a timetable — is about **4 ms end to end** inside Rust, for the whole panel. The
over-subscribed case is 400× slower on a school only 25% larger, because the scorer builds the
violation list and that fixture holds 61 640 of them.

Release barely helps the worst case (1.58 s → 1.51 s), which says the time is spent **allocating
violations** rather than comparing intervals: each `Violation` carries entity references and a
formatted `detail` string. That is a characteristic of M5's design and not a defect of this
milestone; it is recorded in §16 as a limitation, because an editor opened on a catastrophically
broken timetable *will* feel it.

What is not in these figures: the IPC round trip, and the session's own bookkeeping. Neither is
`evara-constraints`' and neither scales with the school.

---

## 15. Tests

| Where | Count | What it owns |
| --- | --- | --- |
| `evara-constraints/src/tests.rs` | 8 | `activity-fixed` in isolation: honoured, broken, room-only move, unplaced, unpinned, `affected`, `explain` |
| `evara-constraints/src/differential.rs` | 6 | the gate over **pinned** problems: union property, running score, the one-occurrence precondition, `of` ≡ `empty` + `place`, and a probe of a locked occurrence |
| `evara-db/src/tests_editing.rs` | 25 | the seven writes; two simultaneous sessions in both directions; school isolation through the `activity → class_section → course → subject` chain; a moved availability rule; the digest-collision case; the property that a sequence of edits does not trip its own freshness check |
| `src-tauri/commands/edit_session/tests.rs` | 10 | the session, the stack, the produced-revision rule, the both-stacks-cleared rule |
| `ipc/edit.test.ts` | 23 | thirteen command names and argument keys; the four refusal predicates; `verdictAt`'s missing-answer case |
| `TimetableGrid.test.tsx` (editing) | 20 | drop targets where allowed and nowhere else; the rule's sentence; no verdict ≠ permitted; a different bell schedule; mouse/keyboard parity; the shared cell's second lesson; a locked placement |
| `EditTimetable.test.tsx` | 38 | the session lifetime; the room re-ask; the four remedies; a refused undo re-reading state; the tray as a pick-up source; Duplicate and its sentence |
| `Timetables.test.tsx` (hand-over) | 4 | the read-only screen stays read-only and hands over |

The reference-vs-incremental property test staying green is the gate, and `activity-fixed` is
now **inside** it rather than beside it.

---

## 16. Known limitations

| Limitation | Why it is acceptable now |
| --- | --- |
| No repair. An edit that leaves hard violations elsewhere is applied and reported, never automatically fixed | Repair is Phase 8's second half, and `minimise-disruption` is the rule it needs. An editor that silently moved a third lesson to make room would be making a scheduling decision the user did not ask for |
| A room change is a move to the same slot with a different room, not its own command | It is the same write, and `probe` answers it identically. A separate command would be a second path to one transaction |
| The editor edits one school's timetable, as everything else does | ADR 0019 and ADR 0032. `room-clash` is still project-wide, so a shared room's bookings are compared; a cross-school *reservation* view is not in this milestone |
| Two windows editing one timetable is detected, not merged | ADR 0047. Merging two arrangements is a decision nobody has specified, and refusing with the count of what moved is the honest answer |
| Multi-period occurrences move as a unit, by their start slot | ADR 0042: one row carries the start and rows 2…*d* are derived. Storing them would make a partial multi-period move representable, which is a corrupt timetable |
| **An editor opened on a catastrophically broken timetable is slow.** Every command builds a scorer, and a scorer builds the violation list — 1.5 s for an arrangement holding 61 640 hard penalties, barely improved by a release build because the time is allocation | §14. A realistic timetable is ~4 ms, and the session caching a scorer is *not* the fix — `CLAUDE.md` and §7.1 both refuse it, because a cached scorer is a second statement of what is placed. Making `Violation` defer its formatted `detail` would be the fix, and that is M5's surface rather than M9A's |
