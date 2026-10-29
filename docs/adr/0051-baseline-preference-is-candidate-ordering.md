# ADR 0051 — Baseline preference is candidate ordering, not a constraint

- Status: **Accepted** (2026-10-25)
- Date: 2026-10-25
- Decides: M9B decision D3, and the status of `minimise-disruption` (`OPS-001`)
- Builds on: [ADR 0039](0039-a-mandatory-constraint-lives-in-the-registry.md) (a rule whose
  absence would make a timetable wrong lives in the registry, and a stored row only tunes it),
  [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) §5 (a
  `GENERATION` scoring is immutable, so scores are comparable across time)
- Amends: `SOLVER.md` §7's second mechanism; `CONSTRAINTS.md` §3's phase for `OPS-001`
- Binds: `evara_solver::construct::place_one`, `Prepared::baseline`

## Context

`SOLVER.md` §7 specified repair's second mechanism as a disruption penalty: *"A built-in
`MEDIUM`-level constraint (`OPS-001`, `minimise-disruption`) penalises every activity whose
placement differs from the baseline … The solver then prefers the smallest edit that fixes the
problem."*

The requirement behind it is right and is one of M9B's mandatory guarantees: a repair should
prefer existing placements within the scope when they are still feasible. The mechanism is
wrong for this engine.

## Decision

**For each in-scope occurrence with a baseline placement, probe that placement first. If it is
still legal, the greedy walk's first-feasible rule takes it and the occurrence does not move.**

Nine lines in `place_one`, before the seeded walk. No score change, no new constraint type, no
new artefacts under `CLAUDE.md` §4, and fully deterministic — the baseline is data, not a random
choice.

`minimise-disruption` (`OPS-001`) is **not implemented in M9B** and moves from `M1` to the
milestone that brings local search.

**What this promises, exactly.** It delivers *"prefers existing placements within the repair
scope when feasible"*. It gives **no** minimal-disruption guarantee and cannot: once an
occurrence is displaced, the occurrences placed after it see a different timetable, and greedy
construction has no mechanism to reconsider. Every record and every screen states this plainly
rather than implying otherwise.

**The baseline probe is not budget-checked**, matching the pinned branch it is modelled on. It
is the one candidate the caller already chose, and refusing it for budget would displace a lesson
that was legally where it already was. The cost is one candidate evaluation per in-scope
occurrence with a baseline, which is why a repair's `SolveStatistics` are not comparable to a
generation's at the same seed — recorded on `solve_run` rather than explained afterwards.

## Consequences

Measured: a clean whole-timetable repair of 1 600 occurrences spends **exactly 1 600 candidate
evaluations** — one baseline probe each, and the walk never runs — and reproduces the baseline
byte-for-byte in 126 ms debug, 90 ms release.

A narrow scope is cheaper than a wide one rather than merely smaller, because the frozen set is
placed into the scorer rather than searched: 2.5 ms for one occurrence of 400 against 17.7 ms
for all 400 (debug).

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **`minimise-disruption` as a real constraint type now** | Four separate reasons, below |
| **A soft penalty threaded through `CompiledConstraint::params`** | `params` is JSON configuration. A baseline is one placement per activity — problem-sized per-activity data. Threading it in means widening either `CompiledConstraint` or `Problem`, and ADR 0037 §2 and ADR 0044 have each already refused to widen `Problem` for a caller's field |
| **Best-of-all-candidates instead of first-feasible** | A real minimisation over the same walk, which `SOLVER.md` §5 notes the deltas are already there for — but it is a search change, and no soft rule exists yet for it to minimise. It belongs with local search |

### Why not `minimise-disruption`, in detail

| Objection | Detail |
| --- | --- |
| **It has nowhere to put the baseline** | The `params` row above |
| **It would make scores incomparable** | A `MEDIUM` penalty for differing from a baseline means a repaired timetable's score is not on the same scale as a generated one's. ADR 0042 §5 keeps the `GENERATION` scoring immutable precisely so scores *can* be compared across time; a baseline-relative penalty quietly undoes that |
| **Greedy cannot minimise it** | First-feasible takes the first legal candidate. A disruption penalty changes `delta`, which the walk does not consult. Real minimisation is local search, which `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 keeps out of the MVP |
| **It would be a rule that cannot fire for most callers** | `CONSTRAINT-ENGINE.md` §3's argument against registering `activity-fixed` early: *"a registered rule that cannot fire tells the user something is being checked when it is not"*. A generation has no baseline |
