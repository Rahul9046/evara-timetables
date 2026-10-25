# ADR 0048 — Repair scope is a movable set, not a set of pins

- Status: **Accepted** (2026-10-25)
- Date: 2026-10-25
- Decides: M9B decision D1, and the audit finding that forces it
- Builds on: [ADR 0045](0045-a-lock-is-a-property-of-a-placement.md) (a lock is a property of a
  placement, and the loader compiles no pins),
  [ADR 0037](0037-stored-row-diagnostics-belong-to-the-loader.md) §2 (`Problem` is not widened
  for a caller's bookkeeping)
- Amends: `SOLVER.md` §7, which specified out-of-scope activities as *"treated as pinned for
  this run"*
- Binds: `evara_core::scheduling::repair::{MovableSet, RepairScope, derive_movable,
  frozen_only}`, `evara_solver::construct`, every repair command

## Context

`SOLVER.md` §7 described repair as a solve with `start = Some(current)` plus two extras, the
first being a scope filter: *"Only activities in the selected scope … are movable; everything
else is treated as pinned for this run."*

Reading the construction solver shows that cannot work. `place_one`'s pinned branch
(`construct.rs:269-277`) probes the pin and, if the probe refuses, returns
`Attempt::Refused` — leaving the occurrence **unplaced**. That is right for a lock, and M9A
documented it: an *impossible* pin is reported rather than forced.

For a repair scope it is fatal. If two out-of-scope placements already clash with each other,
the second one's probe fails and the repair **drops a placement outside the scope it promised
not to touch**. And a timetable somebody is repairing *because* it has clashes is the ordinary
case, not an edge one — so the mechanism fails exactly where the feature is needed.

## Decision

**The solver receives a baseline arrangement and a *movable set*. Everything not in that set is
frozen: present in the arrangement, visible to every constraint, never a candidate, never
probed, never dropped.**

> The scope limits what may **move**. It does not limit what is **checked**.

That sentence answers the hardest question the design faced — how a scope interacts with
constraints spanning several resources. A `teacher-clash` between an in-scope and an
out-of-scope occurrence is caught with no special handling, because the out-of-scope placement
is in the arrangement the scorer holds. **No constraint needs to know the scope exists.**

Three further commitments follow from it.

**The working arrangement holds the frozen placements alone.** Not the baseline. Initialising
from the full baseline is wrong three ways, and all three are properties of the existing APIs:
each movable occurrence would be reconsidered against the stale positions of the others; a
pre-existing movable-to-movable clash would refuse both, because `PlacementVerdict::allowed` is
false whenever the result *carries* a hard violation naming the occurrence rather than only
when it adds one (`incremental.rs:80-90`); and `IncrementalScorer::place` does not withdraw
first, so everything not yet reached would still be in the way. `frozen_only` derives it by
**subtraction** — clone the baseline, clear the movable set — so "every frozen placement
survived" depends on the movable derivation, which `MovableSet::new` has already validated.

**The movable set is derived once, before the solve.** A room or a cycle day is a *position*,
and repair changes positions: an occurrence repaired out of Monday leaves the Monday scope, one
repaired into Monday was never in it. Re-deriving as the run proceeded would make the scope
depend on the search order, which is a different repair for the same request.

**The scope is never widened automatically.** Not to resolve a clash, not to place an occurrence
that would otherwise be left unplaced, not as a retry. Where a wider scope would plainly help,
the interface offers to start a *separate* repair — a new session, a new baseline, a new preview,
a new approval.

## Consequences

Only the whole-timetable scope is closed under the constraints. For every other one, fixing an
in-scope occurrence may require moving an out-of-scope one, and the repair reports the in-scope
occurrence as unplaced rather than reaching outside. This is **reported**, with the note that
the scope may be the reason rather than the timetable.

An occurrence that is **unplaced** in the baseline is in no room and on no day, so it is a
member of neither positional scope. A room or day repair therefore cannot schedule an
unscheduled lesson. The interface states this *before* the run rather than letting it be
discovered in the diff, and names the two scopes that could reach it.

`constrainedness`' pinned-first key becomes inert during a repair: every pinned occurrence is
frozen and so absent from `pending`. The ordering function is unchanged and stays total, so
determinism is unaffected.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **Pin everything out of scope** (`SOLVER.md` §7's wording) | The Context above. A pre-existing clash between two out-of-scope placements makes the second probe fail, and the repair drops a placement it promised not to touch |
| **Compile a sub-problem of in-scope activities only** | `ActivityIx` is dense and problem-relative, so it needs re-indexing; it needs a new "reserved occupancy" vocabulary in `Problem` for the out-of-scope placements, and `Occupancy` rebuilt against it. ADR 0037 §2 refused to widen `Problem` for a diagnostic field and ADR 0044 refused it for a display field; widening it for repair bookkeeping is the same move a third time |
| **Delete out-of-scope placements and re-add them afterwards** | The solver would place in-scope lessons into slots that are actually occupied, the arrangement would be illegal between the two halves, and a cancellation would leave it that way |
| **Start from the full baseline and withdraw lazily** | The movable-to-movable case above. Also makes the result order-dependent in a way the baseline-first rule cannot compensate for |
