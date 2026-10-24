# ADR 0041 — The incremental scorer lives with the evaluators, and has no hand-written delta

- Status: **Accepted**
- Date: 2026-10-23
- Milestone: M6 — Timetable construction solver
- Supersedes: nothing. Implements [SOLVER.md](../SOLVER.md) §4's two-scorer design and
  discharges [OPEN-DECISIONS.md](../OPEN-DECISIONS.md) R1.
- Related: [ADR 0005](0005-constraints-as-data.md),
  [ADR 0039](0039-a-mandatory-constraint-lives-in-the-registry.md),
  [CONSTRAINT-ENGINE.md](../CONSTRAINT-ENGINE.md),
  [TIMETABLE-CONSTRUCTION-SOLVER.md](../TIMETABLE-CONSTRUCTION-SOLVER.md)

---

## 1. Context

`SOLVER.md` §4 names the hardest problem in the project:

> Full re-evaluation of every constraint after every candidate move is too slow for local
> search. But hand-written delta scoring is the single most bug-prone part of any solver, and a
> wrong delta silently produces a wrong timetable.
>
> The mitigation is structural: **two scorers and differential testing.**

`OPEN-DECISIONS.md` R1 rates this the project's highest technical risk and adds a shipping rule:
*"M6 inherits `IncrementalScorer` **and** the property test in the same milestone. Neither half
may land without the other."*

M5 shipped the reference half. `evaluate` runs every rule over the whole arrangement, and
`evaluate_placement` selects by `affected()` and then evaluates each selected rule **in full** —
incremental-*friendly*, not incremental. A construction solver asks *"would this placement break
anything?"* once per candidate, which for the 500-occurrence `medium` fixture is fourteen
thousand times and for a 2 000-occurrence school is a hundred and ninety thousand. Re-running
even a narrowed set of rules over the whole timetable each time is not affordable.

Two questions had to be answered, and they turned out to be the same question.

## 2. Decision

**The incremental scorer lives in `evara-constraints`, beside the evaluators it must agree
with, and it contains no per-rule delta arithmetic.**

Two parts.

### 2.1 Where it lives

`IncrementalScorer` and `PlacementVerdict` are in `evara-constraints`, not in `evara-solver`.

`evara-solver` decides what to try, in what order, and when to stop. Deciding whether a
placement is legal was never its job, and the types that answer that question — `Occupancy`'s
span resolution, each rule's own narrowed query — are internal to the constraint engine.

### 2.2 How the delta is computed

Not by predicting the effect of a placement. By a property of the rules:

> **Every violation any implemented rule produces names at most two occurrences.**

Two for a clash, one for everything else. So a violation that does not name `a` cannot change
when `a` moves, and

```text
score(after) − score(before) = involving(after, a) − involving(before, a)
```

is an **identity**, not an approximation. The scorer applies a tentative placement, asks each
rule what it now finds *about that occurrence*, and puts the previous state back. The rules
answer against a real arrangement, so a probe cannot disagree with the commit that follows it.

`ConstraintEvaluator::evaluate_involving` is the narrowed query, and every implementation shares
its violation-building body with the rule's full sweep. There is no second statement of any
predicate.

## 3. Why the location is the decision and not a detail

A scorer that must agree with eight rules to the violation needs three things those rules own:

| What it needs | Who owns it |
| --- | --- |
| The resolved span of a placement — which `d` slots it occupies and what is wrong with the run | `Occupancy::resolve`, `pub(crate)` |
| Each rule's narrowed query | the evaluator, which is `pub(crate)` |
| The clash predicate, the candidate predicate, the run measure | `PlacementSpan::clashes_in_time_with`, `RoomFact::is_candidate_for`, `runs_on` |

Reaching those from `evara-solver` leaves two options and both are worse.

**Copy them.** That is `CLAUDE.md` §2's most important rule broken in the exact place a solver
usually breaks it: *"If generation and the editor can ever disagree about whether a placement is
legal, the product is broken."* A second span resolver that rounded a multi-period run
differently would make the generator place a lesson the editor refuses, and the symptom would be
a timetable that looks finished.

**Widen the public API until the copy is easy.** Making `Occupancy::resolve`, `SpanDefect` and
eight evaluators public to serve one caller would publish the engine's internals, and the second
copy would then be something a later contributor could write by accident.

Keeping the scorer inside the crate means it shares the predicates by construction rather than
by discipline, and the differential test that gates it sits beside both halves it compares.

## 4. Why there is no hand-written delta

The warning in `SOLVER.md` §4 is about a specific shape of code: per-rule arithmetic that
predicts a score change. *"A teacher clash adds one, unless the pair was already counted; a room
clash adds one per other booking, unless the room is the same as before."* That reasoning is
where solvers go wrong, it goes wrong silently, and it has to be re-derived for every new
constraint type — against the sixty-odd types `CONSTRAINTS.md` eventually names.

The do-and-undo identity has none of those properties:

- **It is derived, not reasoned.** The only assumption is the two-occurrence property, and that
  is asserted over every fixture rather than trusted.
- **It is per-rule-free.** A new constraint type needs `evaluate_involving`, which is its own
  sweep with the loop removed. It needs no delta arithmetic at all.
- **It cannot drift.** The narrowed query and the full sweep share a body, so a change to the
  check changes both.

The cost is that a probe is bounded by the occurrence's own neighbourhood rather than being
O(1), which is the right trade: for `medium`, 14 249 probes take 57 ms in total.

## 5. What keeps it honest

Three properties, in `evara-constraints`' `differential` module, over the whole fixture
catalogue and over seeded random operation sequences:

1. **The narrowed query is complete.** Per rule and per fixture, the union of
   `evaluate_involving` over every occurrence, deduplicated, is exactly what `evaluate`
   produces. A rule that narrowed too far fails by omission; one that narrowed wrongly fails by
   producing a violation the sweep does not. There is no third outcome.
2. **A mutated occupancy equals a rebuilt one.** `place` and `withdraw` maintain the indexes
   every narrowing reads, so an occupancy reached by two hundred operations is `PartialEq`-equal
   to one built from the resulting assignment.
3. **The running score equals the reference score after every single operation.** Adds,
   removals, replacements and rejected probes, checked after each step rather than at the end —
   a delta that was wrong and then wrong the other way would pass an end-only check.

Plus the two-occurrence property itself, asserted over the catalogue, because the identity is
unsound without it.

## 6. Consequences

- `ConstraintEvaluator` gains a sixth member. Every new constraint type implements
  `evaluate_involving`, and `CLAUDE.md` §4's definition of done gains it implicitly: a type
  without one cannot be registered, because the trait is not optional.
- `Occupancy` is mutable. `place` and `withdraw` join `of`, and the type carries three indexes —
  per teacher, per room, per cycle day. `taught_by` and `booked_into` were scans of the whole
  timetable and are now lookups, which speeds up the **reference** scorer too.
- `evara-solver` depends on `evara-constraints` for legality and owns none of it. The crate is
  smaller than the sketch in `SOLVER.md` §2 implied, and that is the point.
- The move validator has its implementation already: `PlacementVerdict` is
  `SOLVER.md` §6's `MoveVerdict` less the suggestions, and it comes from the same scorer the
  generator used — so the editor and the generator cannot disagree about a drop target.
- The one reading that changed: `PlacementVerdict::allowed` is `false` when the result *carries*
  a hard violation, where `SOLVER.md` §6 wrote *"adds"*. The two agree for every placement of an
  unplaced occurrence, which is all M6 makes. They differ for a move of an already-illegal
  lesson, and the stricter reading is correct for both callers — a construction solver must not
  accept a placement that leaves the occurrence clashing, and a drop target that was already
  broken is not a legal drop target.

## 7. Alternatives rejected

**`IncrementalScorer` in `evara-solver`, with the predicates copied.** Rejected by `CLAUDE.md`
§2. The failure mode is a wrong timetable that looks right.

**`IncrementalScorer` in `evara-solver`, with `evara-constraints`' internals made public.**
Rejected because it publishes the engine's internals to serve one caller and makes the forbidden
copy easy to write later. The API surface is a constraint on what future contributors can do
wrongly.

**Per-rule hand-written deltas.** Rejected as §4 describes. This is the thing `SOLVER.md` §4 and
`OPEN-DECISIONS.md` R1 exist to prevent, and the alternative costs a bounded-neighbourhood probe
rather than an O(1) one.

**Maintain the full violation list incrementally as well as the score.** Rejected as bug surface
bought for nothing: a search asks for the score thousands of times and for the list once, at the
end. `IncrementalScorer::report` runs the reference scorer, which is also the strongest possible
answer — the list a caller gets is the one `evaluate` produces.

**Use `evaluate_placement` and accept the cost.** Rejected on measurement. It evaluates each
selected rule over the whole timetable, and `activity-scheduled`'s `affected()` set is
*everything*, so it is always selected and always sweeps every occurrence. For `medium` that is
500 occurrences × 14 249 probes.
