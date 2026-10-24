# ADR 0046 — An edit changes the version it is open on, and duplicating is the user's decision

- Status: **Accepted** (2026-10-24)
- Date: 2026-10-24
- Decides: M9A architecture proposal D2, D3, D11; required correction 1
- Builds on: [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
  §5 (a `GENERATION` scoring is immutable; a revalidation replaces only the current one),
  [ADR 0027](0027-revisions-are-checked-on-write.md) (a write states the revision it was built
  on), [ADR 0030](0030-an-import-is-a-reviewed-plan.md) (never compare a precondition in the
  interface)
- Binds: `commands::edit`, `commands::edit_session`,
  `evara_db::Timetables::{place, move_entry, unschedule, set_lock, duplicate}`,
  `apps/desktop/src/app/timetables/EditTimetable.tsx`

## Context

ADR 0042 settled that a stored timetable is a version and that saving always inserts. It did
not settle what an **edit** does. Two readings were available and they are not compatible:

1. an edit changes the arrangement of the timetable it is open on;
2. an edit forks — the first change writes a new version and leaves the original untouched.

Reading 2 is attractive because it makes every edit undoable by deleting a row, and because it
sounds like the versioning ADR 0042 established. It is also what a user asking *"will this
destroy what the solver gave me?"* is hoping to hear.

## Decision

**An edit changes the version it is open on.** `edit_apply`, `edit_unschedule` and
`edit_set_lock` mutate `timetable_entry` rows of the open timetable, one transaction each. No
command forks a version, and `is_manual` is a per-entry flag on *that* timetable.

**`timetable_duplicate` is the affordance that makes this safe, and it is offered before the
first edit rather than after it.** The editor screen opens with a panel saying, in words, that
editing changes this timetable, that nothing here makes a new version, and that a copy costs
nothing but a name.

### Why in place

ADR 0042 §5 already assumed it. A `GENERATION` scoring is immutable and a revalidation replaces
only the *current* scoring — a guarantee that is **only needed because the arrangement beneath
it can change**. If an edit forked, the generation record could never be out of step with the
arrangement and §5 would be solving nothing.

Three further reasons:

- **Forking makes the list unreadable.** A timetabler making forty adjustments produces forty
  versions, and the screen whose whole premise is *"a row is a version"* fills with rows nobody
  named and nobody wants. Versioning is worth having because each version is a deliberate act.
- **Forking makes `is_manual` meaningless.** The flag says *a human placed this*, which is a
  fact about one arrangement. On a fork chain every entry is manual by construction.
- **The user's decision is the user's.** Whether to keep the generated arrangement is a
  judgement about their own work. An editor that silently forked would be making it for them —
  and so would one that silently did not, which is why the sentence is on screen rather than in
  this record.

### An undo is a new transaction, not a rollback

The session holds an undo stack in Rust and an undo applies the **inverse command** as its own
write. So the change feed records both the move and its reversal, which is what an audit trail
means; a rollback would leave no trace that the user had done and undone something.

It follows that an undo can be **refused** — on a revision conflict, on moved inputs, or on
another window's edit — and that refusing is the honest answer. When one is refused, both
stacks are cleared: every entry below the one that failed was computed against a state that no
longer holds.

Each applied command records the revision **its own write produced**, not the one read before
it. The naive version fails every undo, because the forward write consumed that revision.

### There is no degraded editing mode

`edit_open` refuses with `SetupError::Invalid` when the problem cannot be compiled. A read-only
view of such a timetable still opens — it shows the stored findings and says why there is no
grid — but an editor with no compiled problem could validate nothing, and offering one would be
offering to write placements nothing had checked. The read-only screen therefore renders
**Edit this timetable** disabled, with the reason beside it.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Copy-on-write: the first edit forks a version | Forty adjustments, forty versions; `is_manual` becomes vacuous; and ADR 0042 §5 exists only because in-place editing is possible |
| An explicit "save edits" step, with changes held in memory until then | A session would hold an uncommitted arrangement that no constraint evaluation in the database could see, and a crash would lose work the user watched being accepted. It also reintroduces the two-phase staleness ADR 0042 spent a whole record removing |
| Undo as a transaction rollback | An editing session is many transactions by then, and a rollback would hide from the change feed that anything happened |
| Keep the undo stack in the view layer | An inverse command is a statement about scheduling state and its precondition is a revision the database owns. A stack in React would be a second place deciding what reversing a move means |
| Fork automatically but only on the first *destructive* edit | "Destructive" is a judgement the code would have to make about the user's intent. Unscheduling a lesson to place it elsewhere is the ordinary first step of a rearrangement |

## Consequences

- **A regeneration still never overwrites.** Nothing here changes ADR 0042: generation inserts,
  and editing is a different operation on a timetable that already exists.
- **The `GENERATION` scoring of an edited timetable still describes the generated
  arrangement**, and is still immutable. Its *current* scoring is what the editor keeps up to
  date, which is exactly the split ADR 0042 §5 drew.
- **A duplicate claims no provenance.** The copy carries no `solve_run` and no generation
  scoring, because it was not generated — a cloned `GENERATION` row would be a claim about a run
  that did not produce it.
- **Leaving the editor discards the undo stack, not the edits**, because the session holds the
  stack and the session is the screen's lifetime. The screen says so at the top.
- **Every write echoes back the whole state.** One `EditStateView` — grid, lessons, unplaced,
  findings, score, lock count, history — so a screen cannot render a lesson list from one moment
  and a score from another.
