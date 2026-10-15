# ADR 0027 — A write states the revision it was built on, and is refused if the row has moved

- Status: **Accepted**
- Date: 2026-10-15
- Closes: the last open item of Phase 2. `ROADMAP.md` called it *"the last thing between
  this phase and done"*, and ADR 0023's closing note said *"`rev` is still not checked on
  any write … two windows can still overwrite each other. A frontend-only guarantee would
  be worse than none, because it would be believed."*
- Builds on: **D17** in [OPEN-DECISIONS](../OPEN-DECISIONS.md), which settled in Phase 1B
  that repositories maintain `rev` and `updated_at` explicitly. That decision is what makes
  this one a predicate rather than a rewrite.
- Related: [ADR 0011](0011-stable-timeslot-identity.md), whose preview/confirm fingerprint
  is the same idea for a computed plan rather than one row.

## Context

`Stamp::new` has written `rev = 1` on every row since Phase 1C, every update has
incremented it, and every change-log entry has carried it. Until this stage **no write
took it as a precondition**, and the consequence was concrete:

> Two windows open the same room. Both read revision 4. One renames it and saves; the
> other, still holding 4, changes its capacity and saves. The second write succeeds, the
> rename is gone, and **neither screen says anything.**

Three phases documented it and deferred it: Phase 1E exposed `rev` to the interface, Stage
0A and 0B rewrote most command signatures for the multi-school work, and the audit placed
the fix in Stage B so the churn would not land twice in one review. It also set the
deadline — *"it must not be deferred past Stage C: once lessons and multi-school assignment
exist, two windows overwriting each other stops being theoretical."*

## Decision

**The revision goes in the statement, as a predicate. Every update and delete the
interface can reach carries it, and a stale one is refused with a typed error.**

```sql
UPDATE room SET ..., updated_at = ?, rev = rev + 1 WHERE id = ? AND rev = ?
DELETE FROM room                                   WHERE id = ? AND rev = ?
```

`repo::update` and `repo::delete` take an `Expected`, and both own the clause, exactly as
they already own `rev = rev + 1`. A repository physically cannot write the domain columns
without it, because it never writes that part of the statement.

### 1. The predicate is the mechanism, not the check afterwards

The sequence *read the revision, compare it in Rust, then write* is explicitly **not** what
this is. It is not a concurrency control at all — it is the same race with a longer window,
and it would pass every sequential test while still losing updates in the field. So the
comparison **is** the `WHERE` clause, and a caller cannot reach the write without it.

When the predicate matches nothing, one extra read **inside the same transaction** decides
what to call the failure:

| Row state | Error | What the user is told |
| --- | --- | --- |
| present, different revision | `DbError::Conflict { entity, expected, actual }` | somebody else changed it; reload |
| absent | `DbError::NotFound` | it has gone; it may already be deleted |

Both names matter. Telling somebody a record no longer exists sends them looking for
something that is there; telling them their copy is out of date sends them looking for an
edit that never happened.

Because the statement matched nothing, **nothing happened**: no column changed, no revision
was consumed, and no trigger fired — so the change feed gains no entry describing a write
that was refused. Those last two are what a plausible-but-wrong implementation gets wrong,
and `tests_concurrency::the_second_window_to_save_is_refused_rather_than_winning` asserts
each of them.

### 2. `Expected` is an enum, and `Any` is a sentence

```rust
pub enum Expected {
    Rev(i64),
    Any,
}
```

Not `Option<i64>`. `None` reads as an argument somebody may have forgotten to thread
through; `Expected::Any` is a claim, it is greppable, and every use of it in the crate is
one of three documented kinds: a **cascade** inside another operation's transaction, a
mutation whose contract is not a revision (§4), or a **test** about something else.

**No command handler constructs one.** Every command the interface can call takes
`expectedRev: i64` and wraps it. That is what makes the coverage claim in §3 checkable
rather than hopeful: `grep expected_rev` over `commands/` enumerates the guarded surface,
and `grep Expected::Any` over `repo/` enumerates every deliberate exemption.

### 3. The inventory

84 mutation commands. Every one is classified, and none is unaccounted for.

| Category | Contract | Count |
| --- | --- | --- |
| 1. Single-record update | `expectedRev`, checked atomically | 20 |
| 2. Single-record delete | `expectedRev`, checked atomically | 19 |
| 3. Specialised single-record update | `expectedRev` of **the row that changes** | 2 |
| 4. Relationship creation | existence: duplicate on re-add | 8 |
| 5. Relationship removal | `expectedRev` where the row is displayed, else existence | 6 + 2 |
| 6. Atomic transfer | one call, `expectedRev` of the leaving spell | 1 |
| 7. Plan-guarded bulk | `PlanFingerprint` → `ReviewRequired` | 2 |
| 8. Creation | none: there is no previous version of a row that does not exist | 20 |
| 9. Documented exception | last write wins, argued in §4 | 4 |

**44 commands take `expectedRev`.** Categories 1–3 and 6 are the ones where a lost update
is possible, and all of them are covered:

- **Structure** — school, campus, building, room type, room, resource, academic year, term.
- **Time model** — cycle, cycle day, bell schedule, period, calendar day.
- **People** — year level, teacher, student, student group.
- **Membership** — the per-school employment flag, and a spell's placement, departure and
  transfer.
- **Curriculum** — department, subject, and the two `V14` relationship removals.

### 4. Where a revision is the wrong question

Each of these is a decision, not an omission.

**A relationship either exists or it does not.** `teacher_campus`, `building_campus`,
`room_campus`, `resource_campus`, `student_group_member` and `teacher_school` carry no
mutable domain state, so there is no previous value for a second writer to overwrite. Two
windows both adding the same association: the second gets `DuplicateValue`. Both removing
it: the second gets `NotFound`. Neither is a lost update, and a revision would be a
precondition on nothing.

The rule for whether one is nevertheless passed is simple and is about the interface, not
the schema: **pass the revision wherever the screen already holds the row.** A qualification
and a department association are rows in a list the user is acting on, displayed with
everything about them, so passing it costs nothing and does rule out one narrow case — a
removal built on a row since deleted and recreated. A membership toggle is addressed by its
two ends (`schoolId` + `teacherId`), which is what the screen has in hand; making it carry
a revision would mean an extra read to fetch a number nobody is looking at.

**`set_teacher_membership_active` is the exception that proves the category.** It *does*
mutate a field, so it takes a revision — and the revision of the **membership**, not of the
teacher, because the row that changes is the one with the precondition on it. The same logic
makes `leave_school` take the **spell's** revision.

**A transfer stays one call.** ADR 0021 §7 refused to split it into a leave and an enrol
because a failure between them leaves a real student enrolled nowhere. Giving each half its
own precondition would mean splitting it, so it keeps one: the leaving spell, which is the
only existing row a transfer rewrites. The arriving spell is an insert.

**The grid has something stronger.** `setup_grid_rebuild` and
`setup_grid_release_orphans` carry a `PlanFingerprint` of the plan the caller was shown; the
repository recomputes it inside the write transaction and refuses with `ReviewRequired` if
it has moved (ADR 0011). That guards a whole computed projection rather than one row's
revision, which is strictly more than this decision asks for. Adding a revision would
guard less and read as though it guarded more.

**An availability cell is last write wins, deliberately.** A cell is set by direct
manipulation in a grid — click, and it is unavailable. There is no composed value to lose,
and a precondition would make a two-window disagreement unresolvable without a reload the
user never asked for, in the one screen where a reload is most disruptive.
`tests_concurrency::an_availability_cell_is_last_write_wins_by_design` pins it so the
exception stays a decision.

**A school logo is last write wins**, and is the one residual risk worth naming. Two windows
both setting a logo: the second image wins, silently. It is a single-panel upsert keyed by
`school_id`, the row's revision is not displayed anywhere, and the consequence is a picture
rather than a record. Recorded here rather than fixed, so it is a known gap and not an
unknown one.

### 5. Creations need no precondition

There is no previous version of a row that does not exist. Two windows both creating a
campus called "Main" get a `DuplicateValue` for the second, from the uniqueness rule that
was always there — which is the right answer and not a concurrency control.

### 6. What the interface does with a refusal

The backend half is worthless if the screen then behaves badly, and the tempting wrong
behaviours here are specific. `CrudPanel` does none of them:

- **Nothing is retried.** Re-sending with the current revision would complete exactly the
  overwrite the check prevented, with the user's agreement neither asked for nor given.
- **Nothing the user typed is discarded.** Their edit now exists in no other place, so the
  form stays open, filled in. Re-reading and replacing it to "show them the latest" would
  destroy the one copy of their work.
- **The message says that nothing was saved**, in those words, because a screen that looks
  saved and is not is the failure this whole mechanism exists to prevent. The wording is the
  backend's, shown as written, so there is one place for it rather than one per screen.
- **Reloading is offered, never performed.** When to look at the other version is the user's
  decision; only they know whose change should survive.

`RevisionConflicts.test.tsx` asserts each of those four as *not happening*, which is the
only way a promise of this shape can be held.

## Consequences

- Two windows on one campus file can no longer overwrite each other on any of the 44
  guarded commands. The second save is refused, says so, and keeps the user's values.
- **Every signature that could carry a precondition now does**, which is a large one-time
  churn: 20 repository updates, 19 deletes, 44 commands, 44 IPC wrappers and 17 `CrudPanel`
  specs. `CrudSpec.update` and `.remove` take the **row** rather than its identifier,
  because the revision lives on the row and lives one level down on the projections some
  screens use.
- The existing frontend suite is **stronger**, not merely adjusted: every assertion that
  checked a payload now also pins that the screen sent the row's own revision, and fixtures
  moved off `rev: 1` wherever an assertion reads one, so a default cannot pass by
  coincidence.
- `rev` now has two readers — the change feed and the write path — which makes D17's "no
  self-updating triggers" rule load-bearing rather than tidy. A trigger that bumped `rev`
  would make every second write in a transaction conflict with itself.
- **This is application-level, and that is the right level.** SQLite's WAL journal and
  `busy_timeout` already stop two processes corrupting the file; what they cannot stop is
  one person's considered edit being replaced by another's. The predicate is what stops
  that, and it works across processes because it is evaluated in the database.
- **Not claimed**: automatic merging, field-level conflict resolution, a three-way diff, or
  any notion of *whose* edit is more important. The user is told what happened and decides.
  A merge Evara performed itself would be a guess about intent, which is the one thing a
  timetable cannot afford.
- **Not claimed either**: that a revision makes a write idempotent. Re-sending the same
  successful write with the new revision applies it again; what the predicate prevents is
  applying it *on top of somebody else's*.
