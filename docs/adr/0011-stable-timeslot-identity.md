# ADR 0011 — Stable timeslot identity and non-destructive materialisation

- Status: **Accepted** (approved 2026-10-09, resolves OPEN-DECISIONS D21)
- Date: 2026-10-09
- Relates to: [ADR 0003](0003-sqlite-document-model.md),
  [ADR 0010](0010-structure-deletion-semantics.md),
  [OPEN-DECISIONS R6](../OPEN-DECISIONS.md)

## Context

`timeslot` is the solver's value range and, once Phase 7 exists, the thing every
`timetable_entry` points at. [DATA-MODEL.md §3](../DATA-MODEL.md) specifies it as
**materialised rather than computed**, "rebuilt transactionally whenever the cycle or
period structure changes, preserving the IDs of slots that still exist".

Risk R6 is the reason that sentence exists. If rebuilding the grid reassigns identifiers,
every placement that pointed at "Day B, period 3" now points at a different slot or at
nothing, and the corruption is silent — the timetable still loads, it is just wrong. A
migration cannot fix it afterwards, because the information needed to repair it was the
identifier itself.

So the identity rule has to be settled before the migration is written, not discovered
while writing the materialiser.

## Decision

### 1. Logical identity

A timeslot's logical identity is the tuple

```
(term_id, cycle_day_id, period_id)
```

which is the natural key the brief proposed and matches the uniqueness constraint already
documented in DATA-MODEL.md (`UNIQUE(cycle_day_id, period_id, term_id)` — the same three
columns in a different order).

**The invariant.** `timeslot.id` is assigned once, when its tuple first appears, and is
never reassigned while that tuple still exists. Rematerialising a grid is therefore
identity-preserving by construction, not by luck.

Everything else on the row is a **derived projection**, recomputed on every
rematerialisation and never part of identity:

| Column | Derived from |
| --- | --- |
| `school_id` | the cycle's school |
| `cycle_id` | `cycle_day.cycle_id`, denormalised for `idx_timeslot_cycle` |
| `campus_id` | `period_structure.campus_id` |
| `ordinal` | position in `(cycle_day.ordinal, period.ordinal)` order, numbered from 1 |
| `is_teaching` | `period.kind = 'TEACHING'` |

`ordinal` deserves emphasis: **it is not identity and it is expected to change.**
Inserting a period in the middle of the day renumbers every later slot in the cycle while
every one of those slots keeps its `id`. Code that treats `ordinal` as a stable handle is
wrong; that is what `id` is for. For the same reason `ordinal` carries an index but no
uniqueness constraint — renumbering inside one transaction would otherwise collide with
itself halfway through.

### 2. A grid, and the scope of a rematerialisation

Materialisation operates on one **grid**, identified by `(term, cycle, period_structure)`.
A school with two campuses on different bell schedules has two grids over the same cycle;
materialising one must not report the other's slots as orphans.

The scope of a rematerialisation is therefore exactly:

- **desired** = `cycle_day` of that cycle × `period` of that period structure, at that term
- **existing** = stored timeslots whose `term_id` and `cycle_id` match and whose
  `period_id` belongs to that period structure

Slots outside that scope are not looked at, not touched, and not counted.

### 3. The algorithm

Three cases, which is the whole of it:

| Case | Action |
| --- | --- |
| **A.** desired tuple already exists | keep its `id`; refresh the derived columns |
| **B.** desired tuple does not exist | insert with a fresh UUIDv7, `rev = 1` |
| **C.** existing tuple no longer desired | report it as an orphan candidate; **change nothing** |

Case A refreshes through `repo::update`, so `rev` and `updated_at` move under the D17
contract — but **only when a derived value actually changed**. A rematerialisation that
finds the grid already correct writes nothing at all, which is what makes "running it
twice is idempotent" a meaningful test rather than a tautology about row counts.

Case C never deletes. Not in this phase, and not in a later one without a separate,
explicitly invoked call.

### 4. Why the request carries exclusions

This is the part that is not obvious, and it falls directly out of §5.

`timeslot.cycle_day_id` and `timeslot.period_id` are `RESTRICT`. That means a materialised
grid *blocks* the deletion of a cycle day or a period — which is the point, but it also
means an orphan can never be discovered by deleting something first and rematerialising
afterwards. The deletion is refused before the rematerialisation can run, and a stored
slot's own rows always exist, so no slot is ever *intrinsically* orphaned. Orphan-hood is
only ever relative to a **proposed** configuration.

So a request states the proposal:

```rust
pub struct MaterialisationRequest {
    pub cycle_id: Uuid,
    pub period_structure_id: Uuid,
    pub term_id: Option<Uuid>,
    /// Cycle days to leave out of the desired grid, to preview their removal.
    pub without_cycle_days: Vec<Uuid>,
    /// Periods to leave out of the desired grid, to preview their removal.
    pub without_periods: Vec<Uuid>,
}
```

With both exclusion lists empty — the normal case — the desired grid is whatever the cycle
and the period structure currently say. Populated, it answers "what would I lose if I
removed Day F?" *before* anything is removed, which is the question a confirmation dialogue
has to ask.

### 5. The API, and where destruction lives

```rust
plan_materialisation(&MaterialisationRequest)  -> MaterialisationPlan   // reads only
apply_materialisation(&MaterialisationRequest) -> MaterialisationPlan   // creates + refreshes
delete_timeslots(&[Uuid])                      -> ()                    // the only destructive call
```

`apply_materialisation` **recomputes the plan inside its own write transaction** rather
than accepting a plan computed earlier. A plan is a snapshot of a world that may have moved
on, and applying a stale one would act on tuples that no longer describe anything. This
also removes the need for the caller to hold a plan across a transaction boundary.

`apply_materialisation` performs cases A and B and returns the full plan, orphans included.
It never performs case C. Inspection before destruction is therefore structural: the
destructive step is a *different function*, it takes explicit identifiers rather than a
grid, and it cannot be reached by passing a flag to the safe one.

`delete_timeslots` takes the identifiers from a plan's `orphaned` list after a human has
agreed to them. It cannot verify orphan-hood itself — per §4 that is a property of a
proposal, not of a row — so it is honest about being a plain deletion and says so in its
name. It relies on `RESTRICT` to refuse any slot a future `timetable_entry` references,
which is why timetable entries will not need a cascade: the deletion simply fails, with
`StillReferenced`, and the user is told what is in the way.

### 6. The cycle does not know about weekdays

`cycle_day.ordinal` is the scheduling identity of a day. `cycle_day.label` ("Monday",
"Day A", "Day 1") is a display string, and `weekday_hint` is explicitly documented as
"display only, never logic". Nothing in materialisation, ordering or identity reads either
of them. A six-day rotation, a ten-day fortnight and a one-day cycle are the same code
path with a different row count.

Real dates enter only through `calendar_day`, which stores an **explicit** `date →
cycle_day` mapping. Phase 1D stores and reads those rows; it does not generate them.
Automatic progression — what Day follows a holiday, whether a closure consumes a rotation
slot — is policy that DATA-MODEL.md does not state, so inventing it here would freeze a
guess into the schema. Later phase.

## Consequences

- A timetable can survive adding a period, adding a cycle day, renaming a day, or
  reordering the bell schedule, because none of those changes the identity tuple of an
  existing slot.
- Removing a period or a cycle day is deliberately a multi-step, confirmed operation.
  There is no one-call "just fix the grid" that can lose a placement.
- The grid is per `(term, cycle, period_structure)`, so per-campus bell schedules work
  without the materialiser needing to know about campuses at all.
- `ordinal` churn is the price of keeping identity stable. Any future projection that
  caches ordinals must invalidate on rematerialisation.

## How this is tested

The brief's test list, mapped to what each one actually pins down:

| Test | Pins down |
| --- | --- |
| initial materialisation creates the expected slots | the desired set is the full product |
| running it twice is idempotent | no inserts, **no `rev` bumps** on the second run |
| the second run preserves every `id` | the invariant, directly |
| adding a period preserves old ids, creates only new tuples | case A vs. case B |
| adding a cycle day preserves old ids, creates only new tuples | same, on the other axis |
| removing a period reports orphans without deleting | case C, via `without_periods` |
| removing a cycle day reports orphans without deleting | case C, via `without_cycle_days` |
| a failed apply rolls back completely | one transaction, proven by a mid-way constraint failure |
| reopening the project preserves ids | identity is persisted, not in-memory |
| a cycle with more than five days | no weekly assumption |
| cycle-day and period ordering are deterministic | `(cycle_day.ordinal, period.ordinal)` |
| uniqueness constraints are scoped correctly | including the `NULL` term case (Q1) |
| invalid foreign keys fail | enforcement is on for these tables too |
| created slots start at `rev = 1` | D17 |
| every new entity table has its three triggers | the existing schema-level guard |
| arbitrary named cycle days, explicit date mappings, cycle length ≠ 7 | no weekday leakage |

## Questions that were asked rather than guessed

Four points were genuinely ambiguous and all four change the migration, so they were put
to the maintainer before V4 was written rather than defaulted.

### Q1 — a nullable `term_id` left the identity rule unenforceable

**Decided: `NULL` means "the whole year", and the constraint is fixed to actually hold.**

SQLite treats `NULL`s as distinct in a `UNIQUE` index, so the constraint DATA-MODEL.md
documents — `UNIQUE(cycle_day_id, period_id, term_id)` — does not prevent two identical
term-less slots. The documented column stays nullable, because a year-wide grid is the
common case and a school whose bell schedule never varies by term should not be made to
materialise one grid per term. Identity is enforced instead by a unique index over

```sql
UNIQUE (cycle_day_id, period_id, COALESCE(term_id, ''))
```

so the term-less case is constrained exactly like any other. The plain three-column
constraint is **not** also created: it would be redundant where `term_id` is set and
useless where it is not.

Identity therefore contains a nullable column, which the materialiser handles because the
term scope is an input to the request, not something it infers from the rows.

### Q2 — `ON DELETE` for `calendar_day.cycle_day_id`

**Decided: `RESTRICT`.** `SET NULL` was available — `NULL` is documented as "no lessons
that day" — but it would silently reinterpret mapped teaching dates as non-teaching ones,
which is a change of meaning rather than a detached reference. Deleting a cycle day is
refused while any date still maps to it, and the user remaps first. This follows D19:
ownership edges refuse.

### Q3 — whether `is_default` is constrained

**Decided: at most one default, enforced in the schema.** A default with two candidates is
a bug that surfaces much later as "the app picked the wrong cycle". One default cycle per
school, and one default period structure per campus — with the school-wide `NULL` campus
treated as its own bucket via the same `COALESCE` technique as Q1, for the same reason.

### Q4 — `cycle.day_count` versus the actual `cycle_day` rows

**Decided: declared length, checked in the repository.** `day_count` is the intended cycle
length; the materialiser uses the `cycle_day` rows that actually exist. The repository
rejects an ordinal above `day_count`, but does not require the rows to be complete or
contiguous, so a half-built cycle is a legal intermediate state — a trigger demanding
exact agreement would make it impossible to add days one at a time. No cross-table trigger
goes into V4.
