# ADR 0052 — An unexpressible lock blocks a repair and only warns an edit

- Status: **Accepted** (2026-10-25)
- Date: 2026-10-25
- Decides: M9B required correction 3
- Builds on: [ADR 0045](0045-a-lock-is-a-property-of-a-placement.md) (a lock is
  `timetable_entry.is_locked`, and `Problem::pinned` is filled by the caller that opened a
  timetable), [ADR 0011](0011-stable-timeslot-identity.md) (materialisation never deletes; it
  reports orphan candidates and leaves them alone),
  [ADR 0032](0032-timetable-generation-is-scoped-to-one-school.md) rule 4 (a timetable is never
  described as conflict-free)
- Binds: `evara_db::{StoredPins, UnexpressibleLock, UnexpressibleLockCause}`,
  `commands::repair::{repair_preflight, describe_unexpressible}`

## Context

`Timetables::pins` resolves three indices per locked entry — the activity, the timeslot, and the
room where one is booked. On **any** of the three failing it records the entry in
`not_in_problem` and continues, so the lock is **absent from `Problem::pinned`**.

M9A's editor reports such a lock and lets the user carry on, which is right for an editor: it is
answering about one placement the user can see and is about to write one row.

The M9B proposal copied that tolerance, and it is wrong for a repair. Three cases, and the second
is worse than the first:

| Cause | What happens to the lock |
| --- | --- |
| The **activity** is not in this problem | It has no `ActivityIx`, cannot be frozen, and `project_entries` — which walks the `Assignment` — cannot write it back. The lock **vanishes** from the new version |
| The **timeslot** is not in the materialised grid | The *activity* may well resolve, so the occurrence is an ordinary problem member with **no pin and nothing marking it frozen**. A repair whose scope includes it will **move it** — the guarantee broken outright rather than quietly |
| The **room** is no longer available to this campus | The same as the timeslot case |

The second is the realistic one. ADR 0011 requires materialisation never to delete, so a
`timeslot` row survives a grid change while ceasing to be part of the selection's grid.

**Could the lock be carried across instead?** Every foreign key on `timetable_entry` is
`ON DELETE RESTRICT`, so none of these means the referenced row was deleted — it still exists,
and the entry is physically insertable into the new timetable. It is still wrong to insert it:
the repair solved a problem that does not contain that occurrence, so nothing checked the
carried-over placement against the repaired arrangement. Writing it produces a timetable with a
row no evaluation scored and no preview showed — the sentence ADR 0032 rule 4 exists to prevent.

So the data model does not prove the lock can be preserved **exactly**; it proves it cannot.

## Decision

**An unexpressible lock is a blocking pre-flight finding. A repair is refused until the user
resolves it, and nothing in the repair path writes or clears a lock on their behalf.**

`StoredPins` gains `unexpressible: Vec<UnexpressibleLock>`, each carrying the entry, the
activity and a typed `UnexpressibleLockCause` naming what failed and what it named.
`not_in_problem` is kept, because M9A's session reports a count and nothing more. `pins`
already computed the three cases as three separate `continue`s and discarded the distinction, so
this surfaces information rather than deriving any.

Each cause carries a remedy the user can act on:

| Cause | Remedy offered |
| --- | --- |
| `ActivityNotInProblem` | Repair the timetable for the cycle and term the lesson belongs to, or remove the lock on it |
| `TimeslotNotInProblem` | Unschedule or re-place that lesson in a period that exists, then lock it again |
| `RoomNotInProblem` | Re-place the lesson in an available room and lock it again, or share the room with this campus again |

**The asymmetry with the editor is deliberate.** An editor tolerates it because it is about to
write one row the user can see; a repair is about to write a whole new version from a compiled
problem that cannot represent the lock at all. M9A's behaviour is unchanged, and a test pins it
so the widening of `StoredPins` cannot quietly change it.

## Consequences

A timetable with many stale locks needs real work before it can be repaired. That is the cost of
the guarantee, and it is mitigated by naming each remedy and offering the lock's removal as a
one-click alternative — because the user may simply no longer want it.

A pre-flight that silently unlocked something to let itself proceed would be the discarding this
record exists to prevent, so it is forbidden rather than merely avoided:
`an_unexpressible_lock_is_never_cleared_by_reading_it` asserts the `is_locked` flag and the row's
revision are both untouched after a refusal.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| **Warn and proceed**, as the editor does | The Context. In the timeslot and room cases the occurrence is movable with no pin, so the repair moves a locked lesson — the guarantee broken, not merely unmet |
| **Carry the row across outside the projection** | The placement would be in a timetable no evaluation scored and no preview showed, and `save_repaired` would become a second writer into `timetable_entry` alongside the projection, with the evaluation's `placed_count` disagreeing with the row count |
| **Clear the lock automatically and report it** | Answering "we cannot honour this lock" by removing it is the silent discarding the guarantee forbids. The user may want the lesson moved instead, and only they know |
| **Freeze the occurrence without a pin** | There is no `ActivityIx` in the activity case, so there is nothing to freeze. In the other two it would freeze the occurrence at a placement the compiled problem cannot express, which `Occupancy` would refuse to read |
