# ADR 0049 — A repair's verification is scope-aware, and a pre-existing violation is not a defect

- Status: **Accepted** (2026-10-25)
- Date: 2026-10-25
- Decides: M9B required correction 2, and the mandatory safeguard added at approval
- Builds on: [ADR 0041](0041-the-incremental-scorer-lives-with-the-evaluators.md) (the
  incremental scorer lives with the evaluators and has no hand-written per-rule delta),
  [ADR 0048](0048-repair-scope-is-a-movable-set.md) (the scope limits what may move, not what is
  checked), [ADR 0032](0032-timetable-generation-is-scoped-to-one-school.md) rule 4 (a timetable is never
  described as conflict-free)
- Binds: `evara_solver::repair::{verify, RepairDefect}`,
  `evara_core::scheduling::repair::{frozen_differences, locked_differences}`,
  `evara_db::Timetables::save_repaired`, `DbError::ScopeViolated`

## Context

Two questions were conflated in the M9B proposal, and separated the answers are opposite.

**Does a pre-existing hard violation among frozen occurrences refuse every subsequent
candidate?** No. `IncrementalScorer::probe` computes `let after = self.involving(activity)` and
sets `blocking` to the HARD subset **of that list** (`incremental.rs:247-281`). `involving` runs
each rule's `evaluate_involving` for the probed occurrence only — the trait member `CLAUDE.md`
§4 item 6 makes mandatory. A violation between two frozen occurrences names neither the probed
occurrence nor its candidate, never appears in `blocking`, and cannot refuse anything. Candidate
acceptance is already scope-aware, and `evaluate_involving` is the reason.

**Does the final verification accept such a repair?** No — and that was a blocking defect.
`generate_with` hands the reference report to `status_of`, whose first line is

```rust
if !report.is_readable() || !report.score.is_feasible() { return SolveStatus::Failed; }
```

and `Score::is_feasible` is `self.hard == 0` (`outcome.rs:93-95`) — a property of the **whole
arrangement**. A repaired arrangement contains every frozen out-of-scope placement,
pre-existing hard violations included. So every repair of a timetable holding a hard violation
outside the chosen scope would have returned `SolveStatus::Failed` with
`Termination::InternalCheckFailed`, which is never applyable. The repair would have been
correct, the arrangement better than the baseline, and the product would have reported an
internal defect and discarded it — on precisely the timetables repair exists for.

## Decision

**A repair verifies its own answer with a scope-aware check that is *stronger* than the
generation gate, not more permissive. `generate_with` and `status_of` are untouched;
`repair_with` is a sibling.**

The reference evaluator remains the only judge of legality. What changes is the question asked
of its report. Five checks, and any of them failing is an internal consistency failure that is
never an applyable result:

| | Check | Catches |
| --- | --- | --- |
| **V0** | `frozen_differences` over the **complete** baseline and repaired assignments, occurrence by occurrence, slot *and* room, in both directions | A frozen placement moved, dropped or invented |
| **V0b** | `locked_differences` — the same question narrowed to locks, measured against the pin | A lock not honoured, caught separately because the guarantee is separate |
| **V1** | `!report.is_readable()` | An arrangement that does not describe the problem. Unchanged from M6 |
| **V2** | any HARD violation whose `activities` intersects the movable set | The genuine incremental-versus-reference disagreement: `probe` refused every candidate carrying a hard violation naming the occurrence |
| **V3** | the frozen-only HARD set differs from the baseline's, by `ViolationKey` | A frozen placement acquiring or losing a conflict |

**V0 is the primary proof, and is deliberately not derived from V3.** Two arrangements can carry
identical violation sets and still differ — a frozen occurrence moved to an equally-clashing
slot, or two frozen occurrences swapped, produce the same keys. The violation-set comparison is
kept as a second, independent check because the two fail differently.

**V0 is performed twice: once by the solver before it returns, and again by `evara-db` inside
the writing transaction.** An outcome that passed in memory and is then persisted from a
different arrangement would write a timetable nobody checked. One implementation —
`evara_core::scheduling::repair::frozen_differences` — called from both places, which is why it
lives in `evara-core`: `evara-db` may not depend on `evara-solver`.

**A non-empty frozen pre-existing HARD set is not a failure, is not a reason to withhold
applying, and is reported as what it is.** The four attributions are set operations on the two
reference reports, in `evara-report`: pre-existing-frozen, new-involving-movable, eliminated,
and not-fixable-within-this-scope. No rule is restated and no violation re-judged — the
partition reasons about *attribution*, never legality.

## Consequences

V2 and V3 together are strictly stronger than `score.is_feasible()`. The global check cannot
distinguish a frozen placement being disturbed from a pre-existing problem; V3 catches the
former while tolerating the latter, and V0 catches it whether or not it changes any violation.
A repair therefore verifies **more** than a generation does.

The preview reports the **whole** arrangement's score and hard-violation counts, with the
buckets beside them rather than instead of them. A preview showing only in-scope violations
would be describing a timetable the user is not about to save.

Reference-versus-incremental equivalence is untouched: `differential.rs` is unchanged, `probe`,
`place`, `withdraw` and `apply` are unchanged, and the identity the gate asserts is unchanged.
The frozen-only initialisation is `of()` over a different arrangement, which `of` already
supports and whose equality to `empty` + `place` per entry M9A already asserts over every
fixture.

`SolveStatus::Partial` stays applyable. A repair of a timetable with out-of-scope unplaced
lessons is *always* partial, so refusing partial would refuse almost every real repair. A
budget-limited run is applyable too, with the limit named — presenting a limit as a conclusion
is the dishonest alternative.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **Route a repair through `status_of`** | The Context. Inoperable on the timetables the feature is for |
| **Relax `status_of` for everyone** | A generation that produced a hard violation *is* a defect, and ADR 0032 rule 4 depends on that staying true. Weakening the shared gate to admit a repair would weaken the generation's guarantee as well |
| **Infer scope preservation from the violation sets alone** | Does not prove it. A frozen swap and a move to an equally-clashing slot are both invisible to a `ViolationKey` comparison, which is why V0 exists and is primary |
| **Check V0 only in the solver** | An outcome is data; it can be constructed, mutated or mismatched between the verification and the write. The database is the last place that can still refuse |
| **Report a frozen pre-existing violation as the repair's** | Untrue, and it would make every repair of a conflicted timetable look like a failure |
