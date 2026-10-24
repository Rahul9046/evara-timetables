# ADR 0045 — A lock is a property of a placement, not of a lesson

- Status: **Accepted** (2026-10-24)
- Date: 2026-10-24
- Decides: M9A architecture proposal D1, D5
- Builds on: [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
  (a stored timetable is a version among several, and one `timetable_entry` row is one
  occurrence), [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §1 (an
  `activity` row is one placeable occurrence),
  [ADR 0039](0039-a-mandatory-constraint-lives-in-the-registry.md) (a rule whose absence would
  make a timetable wrong lives in the registry),
  [ADR 0037](0037-stored-row-diagnostics-belong-to-the-loader.md) §2 (`Problem` is not widened
  for something that is not a scheduling input)
- Binds: `timetable_entry.is_locked`, `evara_db::Timetables::{set_lock, pins}`,
  `evara_core::scheduling::Problem::pinned`, `evara-constraints`' `activity-fixed`
- Corrects: five shipped records that said `activity.is_locked` — [CONSTRAINTS.md](../CONSTRAINTS.md)
  §3 and §4, [CONSTRAINT-ENGINE.md](../CONSTRAINT-ENGINE.md) §3,
  [SCHEDULING-PROBLEM-ASSEMBLY.md](../SCHEDULING-PROBLEM-ASSEMBLY.md) §4,
  [SCHEDULING-READINESS-AUDIT.md](../SCHEDULING-READINESS-AUDIT.md) §9

## Context

Manual editing needs a way for a user to say *"leave this one alone"*. Every record written
before M7 called that column `activity.is_locked`, and said `activity-fixed` would read it:

> `activity-fixed` is the ninth type of the first set and is **deferred**: there is no
> `activity.is_locked` column at `V16`, locks arrive with Phase 8's manual editing…
> — CONSTRAINTS.md §3, as shipped in M5

That was written before ADR 0042, and ADR 0042 changed the thing it assumed. A timetable is
now a **version**: several coexist for one `(cycle, term)`, a save always inserts, and
replacement is a save followed by an explicit delete. The question *"is this lesson locked?"*
therefore no longer has one answer.

## Decision

**`is_locked` is a column on `timetable_entry`, and there is no lock anywhere else.** A lock
names a placement — this occurrence, in this timetable, in this slot, in this room — and it is
meaningless without one.

Four consequences, each load-bearing:

### 1. A lock is per version, because a placement is

`activity.is_locked` would have said *"occurrence 3 of 9 Mathematics B is locked"* with no
statement of **where**. Applied to a project holding Draft A and Draft B over the same cycle,
it would pin the lesson in both — including in a draft generated before anybody locked
anything, and including in a draft where that occurrence sits somewhere else entirely. A user
who locked a placement in Draft A would discover that Draft B had acquired a hard violation
they never touched.

It is also the wrong arity. Locking means *this slot and this room*, and `activity` carries
neither; the lock would have had to be read together with whichever `timetable_entry` happened
to be in front of the reader, which is the per-entry column with extra steps.

### 2. The lock column sits beside `is_manual`, which is already per entry

`V17` shipped both on `timetable_entry` for the same reason: *who placed this* and *may it be
moved* are facts about one arrangement. Nothing in M9A needed a migration — the column, both
uniqueness rules, the `BEFORE UPDATE` cross-cycle trigger, all three indexes and the
change-log triggers were already there.

### 3. `activity-fixed` reads `Problem::pinned`, and the **caller** populates it

The compiled `Problem` holds no pins. `evara_db::scheduling` compiles a problem from
**inputs** — the cycle, the bell schedules, the lessons, the availability rules — and a lock is
an *output* of a solve, stored on a row a solve produced. A loader that read `timetable_entry`
would make the compiled problem depend on which timetable happened to be open, so two users
generating against the same setup would be solving different problems.

So `Timetables::pins` turns one timetable's locked entries into
`Vec<Option<Placement>>`, the editing session hands that to `Problem::pinned`, and
`activity-fixed` compares against it. This is ADR 0037 §2's refusal to widen `Problem` for a
non-input, applied to a second case.

Generation from scratch therefore pins nothing, which is correct: nothing has been placed yet,
so nothing has been locked.

### 4. A pin compares the whole `Placement` — slot **and** room

A repair that kept a locked lesson's period and took its room away has moved it, and a user who
locked a lesson into the laboratory did not lock it into *a* period. `Placement` is the unit the
solver moves, so it is the unit the pin protects.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| `activity.is_locked`, as every pre-M7 record said | §1. A lock with no placement in it, applied to every version at once |
| A separate `activity_pin` table keyed by `(timetable_id, activity_id)` | That is `timetable_entry` with a second row per placement, a second uniqueness rule to keep consistent with it, and a way for a pin to outlive the placement it names. The column is the same fact in the row that holds the placement |
| A lock on the timetable as a whole | Locks a user's answer to *"which of these may the computer change?"* into all-or-nothing. The interesting case is three lessons out of four hundred |
| Enforce the lock only in the editor, with no constraint type | The editor and a later repair would then disagree about what a lock means, which is `CLAUDE.md` §2's most important rule broken exactly where it matters. The command layer refuses a locked move *and* `activity-fixed` is the registry's backstop |

## Consequences

- **Deleting a timetable removes its locks**, which is right: they were statements about that
  arrangement.
- **`timetable_duplicate` copies the locks**, because it copies the arrangement. A user who
  duplicates before editing keeps their pins.
- **The vendor mapping changes.** "Any vendor's fixed lesson" maps to
  `timetable_entry.is_locked` + `activity-fixed`, not to an activity column.
- **A locked placement naming something the campus file no longer holds is reported
  separately** (`EditStateView.pinsNotInProblem`): it cannot be given to `Problem::pinned`, so
  a later repair could not be told to preserve it, and the screen says so rather than quietly
  dropping a user's instruction.
- **Nothing is seeded.** `activity-fixed` is mandatory in the registry per ADR 0039, so no row
  grants or revokes it; a `constraint_instance` row may raise its level and may not weaken it.
