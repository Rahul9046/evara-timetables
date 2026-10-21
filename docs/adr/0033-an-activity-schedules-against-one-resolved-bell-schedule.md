# ADR 0033 — An activity schedules against exactly one bell schedule, resolved by campus compatibility and never guessed

- Status: **Accepted**
- Date: 2026-10-21
- Decides: [OPEN-DECISIONS](../OPEN-DECISIONS.md) **D34**, raised by
  [`SCHEDULING-READINESS-AUDIT.md`](../SCHEDULING-READINESS-AUDIT.md) §4.1 as the first
  question the `Problem` loader must answer.
- Builds on: [ADR 0011](0011-stable-timeslot-identity.md) (a timeslot's identity is
  `(term, cycle_day, period)` and carries no campus),
  [ADR 0017](0017-weekly-and-custom-cycles.md) (a cycle carries no dates and no working
  week), [ADR 0019](0019-multiple-schools-in-one-project.md) (a project holds several
  schools), [ADR 0023](0023-the-interface-scopes-to-one-school.md) (the interface scopes to
  one school), [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) (one
  `activity` row is one placeable occurrence, and the chain carries the school),
  [ADR 0032](0032-timetable-generation-is-scoped-to-one-school.md) (generation is scoped to
  one school; shared occupancy is compared in real time within it).
- Binds: the `Problem` loader (M4), the value range it builds, and the
  `duration-contiguous` and `no-teaching-in-period` evaluators (M5). Nothing is
  implemented by this record.

## Context

The grid is materialised per `(cycle, period_structure, term)` — `MaterialisationRequest`
(`repo/time_model.rs:1652-1662`) names all three. A school running two campuses on two bell
schedules therefore holds **two disjoint sets of `timeslot` rows over the same cycle days**,
and `TimeModel::timeslots(cycle_id, term_id)` (`repo/time_model.rs:2139`) returns both in
one list. That list is the solver's value range, and something has to say which half of it
an activity may be placed in.

### What the schema says, checked rather than assumed

| Fact | Where |
| --- | --- |
| A bell schedule names **at most one** campus; `NULL` means school-wide | `V4:77-93`, `period_structure.campus_id` |
| `is_default` is per **campus bucket**, the school-wide rows forming their own | `idx_period_structure_single_default` — `(school_id, COALESCE(campus_id, ''))` |
| A `timeslot`'s `campus_id` **is** its structure's, enforced `IS`-equal by a trigger pair | `V4:149-184` |
| A timeslot's identity carries **no campus**: `(cycle_day_id, period_id, COALESCE(term_id,''))` | `idx_timeslot_identity`, ADR 0011 |
| An activity reaches a campus only through `class_section.campus_id`, which is **nullable** | `V15:423`, `V15:825` |
| A section with no campus is **not campus-constrained** — the ordinary state at a one-campus school | `V15:417-419` |
| Period times are `'HH:MM'` wall clock, `CHECK`-validated, and cannot cross midnight | `V4:104-105`, plus `CHECK (ends_at > starts_at)` at `V4:111` |
| `timeslot.is_teaching` is derived from `period.kind = 'TEACHING'` | `repo/time_model.rs:1993` |

Two consequences of that table are worth stating because they are easy to get backwards.

**A bell schedule cannot be shared by a *subset* of campuses.** `campus_id` is one nullable
column, not an association table. Two campuses that run identical timings are expressed
either as **one school-wide structure** (`campus_id IS NULL`, serving every campus) or as
**two equivalent campus structures** — and in the second case the school holds two disjoint
`timeslot` sets covering the same minutes. This is a real modelling limit and it is not
widened here; §"Alternatives rejected" says why.

**Period 1 is not a time.** Two structures may both carry `ordinal = 1` with different
`starts_at`. Because they are different `period` rows, they produce different `timeslot`
rows, so the solver can never *confuse* them — but neither can it compare them by
`timeslot_id`. ADR 0032 rule 2 already settled that shared occupancy compares
`[starts_at, ends_at)` intervals; this record settles which structure an activity draws
from in the first place.

### Why no default is obviously right

The audit's Option A — "resolve through the section's campus, default to the school-wide
schedule" — is wrong in a way that only shows up in the finished timetable: at a two-campus
school whose sections name no campus, every activity would be placed on whichever schedule
the fallback happened to reach, and both campuses' lessons would be scheduled against one
set of bells. Nothing in the result would say so.

The interface already contains a smaller version of the same ambiguity, which is the
evidence that this is not hypothetical. `GridPreviewSection.tsx:103` defaults the bell
schedule to `pick(structures.items, (s) => s.isDefault)`, and `pick` returns the **first**
match in a list ordered by name (`repo/time_model.rs:1160-1166`). A school holding both a
school-wide default and a campus default has two `is_default = 1` rows, and which one that
expression returns depends on their names. It is survivable there — materialisation is a
previewed, confirmed operation — and it would not be survivable in a loader that runs
without a confirmation step.

## Decision

**A solve resolves exactly one bell schedule for every activity it places, by campus
compatibility against the structures actually present in the grid. A resolution that is not
unique is a hard pre-flight error naming the candidates and the remedy. There is no
fallback, no default and no guess.**

### 1. The scheduling grid of a solve

A solve names `(school_id, cycle_id, term_id)` — ADR 0032 rule 1 — and may *optionally*
narrow itself to a named subset of bell schedules. From that:

```
G = { the distinct period_structure rows that have materialised timeslot rows
      in (cycle_id, term_id) }                                     -- derived, never stored
  = SELECT DISTINCT ps.id, ps.campus_id
      FROM timeslot t
      JOIN period p            ON p.id  = t.period_id
      JOIN period_structure ps ON ps.id = p.period_structure_id
     WHERE t.cycle_id = :cycle AND t.term_id IS :term
```

`G` is **derived from what is materialised**, not from what exists. A structure a school
defined and never materialised into this grid has no slots, so it is not a candidate — which
is correct, because there is nothing to place anything in. The optional narrowing intersects
`G` and is the user's answer to an ambiguity, never the loader's.

### 2. Compatibility, as a relation

A structure `s ∈ G` is **compatible** with a class section `cs` when **any** of:

1. `s.campus_id IS NULL` — a school-wide schedule serves every campus, by its own
   definition (`V4:77-82`);
2. `s.campus_id = cs.campus_id` — the section is at that campus;
3. `cs.campus_id IS NULL` **and the school has exactly one campus** and `s.campus_id` is
   that campus.

Clause 3 is the one that needs defending, and it is not a fallback. A school with one campus
has one place. "This section is not campus-constrained" and "this section is at the only
campus there is" are the same statement about such a school, and the count of `campus` rows
for the school is a stored fact, not an inference. Without clause 3 the commonest
configuration in the product — one campus, one bell schedule scoped to it, sections that
leave `campus_id` null because there is nothing to choose — would resolve to nothing.

Clause 3 does **not** generalise. At a school with two or more campuses, a section naming no
campus is compatible only with a school-wide schedule, because a campus-scoped schedule is a
statement about one campus and the section is not at a known one.

### 3. Resolution, and the three ways it fails

```
resolve(activity) :=
    cs  := class_section of activity            -- ADR 0029 §1, through course → subject
    C   := { s ∈ G : compatible(s, cs) }
    match C.len():
        1 => Resolved(s)
        0 => Err(NoBellSchedule   { ... })
        _ => Err(AmbiguousBellSchedule { candidates: C, ... })
```

Both errors are **hard pre-flight errors**. They are raised once per class section (not once
per occurrence — every activity of a section resolves identically, because resolution reads
only the section) and they refuse the solve rather than excluding the activity, because an
activity that cannot be placed anywhere is not a scheduling difficulty, it is an unanswered
setup question.

`NoBellSchedule` carries the reason, because the three causes have three different remedies:

| Cause | Diagnostic |
| --- | --- |
| `G` is empty | *"No timetable grid has been built for this cycle and term. Build one under Timetable Grid Preview before generating."* |
| The section names campus *c*, and `G` holds neither a schedule for *c* nor a school-wide one | *"Year 9 Mathematics A is at Northgate Senior, and no bell schedule in this grid applies there. Build the grid from Northgate Senior's bell schedule, or make a schedule school-wide."* |
| The section names no campus, the school has more than one, and `G` holds no school-wide schedule | *"Year 9 Mathematics A does not say which campus it is taught at, and this school's bell schedules are each tied to one campus. Give the section a campus, or make a bell schedule school-wide."* |

`AmbiguousBellSchedule` has one shape and one remedy pair:

> *"Year 9 Mathematics A could be timetabled against two bell schedules: Standard Day
> (school-wide) and Northgate Senior Day (Northgate Senior). Evara will not choose. Build
> this grid from one of them, or restrict this generation to one of them."*

It names every candidate. A message that says "ambiguous" without saying between what is a
message the user cannot act on.

### 4. The value range, and what is excluded from it

For a resolved structure `s`, the activity's candidate timeslots are the rows of `G`'s grid
belonging to `s`, further restricted to `is_teaching = 1`.

The `is_teaching` restriction is a **pre-filter of a constraint that still exists**, not a
replacement for it. `no-teaching-in-period` remains a registered evaluator, because manual
editing (M11) and a locked placement can both name a non-teaching slot and the editor must
refuse for the same reason the generator never offered it. Both read the one stored
`timeslot.is_teaching`, derived from the one `period.kind`, so they cannot disagree —
which is the whole of `CLAUDE.md` §2's single most important rule applied to a domain
restriction.

### 5. Duration, as runs within the resolved structure

A `duration_periods = d` occurrence needs **`d` ordinal-consecutive `TEACHING` periods of
its own resolved structure, on one cycle day.** A run is broken by any non-teaching period
between two teaching ones; a mere wall-clock gap between consecutive teaching periods does
not break it, because a gap between bells is a gap between bells and not a non-teaching
period. This is exactly the measure `evara_core::lessons::longest_run`
(`crates/evara-core/src/lessons.rs:230`) already computes, so `duration-contiguous` and
`DemandFinding::OccurrenceLongerThanAnyRun` agree by construction.

A run **never spans two structures**, even when their minutes are adjacent. Two structures
are two grids; an occurrence lives in one.

### 6. Interval comparison, and what it does and does not need

Within a school, two placements occupy overlapping real time when they fall on the same
`cycle_day` and their `[starts_at, ends_at)` intervals intersect — ADR 0032 rule 2. Because
`starts_at` and `ends_at` are zero-padded `'HH:MM'` with `CHECK (ends_at > starts_at)`, no
period crosses midnight and the test is `a.starts_at < b.ends_at AND b.starts_at < a.ends_at`
on text that sorts chronologically. A multi-period occurrence's interval is
`[starts_at of its first period, ends_at of its last]`.

This needs **no timezone and no calendar date**, and that is why it is implementable now:

- One school has one `school.timezone`, so both sides of every comparison are in the same
  zone and the zone cancels. Across schools it would not cancel — and the workspace holds
  no timezone database at all (`time = { version = "0.3", features = ["formatting",
  "parsing", "macros"] }`, `Cargo.toml:47`; nothing parses `school.timezone`, which is
  `TEXT` with only a non-empty `CHECK`). Cross-school wall-clock comparison is therefore not
  merely undesigned, it is unavailable without a new dependency. ADR 0032 §3 is the record
  that defers it.
- A cycle carries no dates (ADR 0017), the timetable is defined over the cycle, and both
  structures of one school materialise against **the same `cycle_day` rows** — `cycle_day`
  belongs to the cycle and not to a structure. So the day axis is shared without
  `calendar_day` being involved. Missing calendar mapping does not affect any comparison in
  this record.
- Different cycles of one school are different grids and are never compared. A solve names
  one cycle.

### 7. Availability compiles against the resolved structure

`availability` expresses three granularities (`V11:11-18`), and they are not equally
portable across bell schedules:

| Granularity | Scope |
| --- | --- |
| `cycle_day_id` alone | structure-independent — "all of Monday" means all of Monday on every schedule |
| `period_id` alone, or with `cycle_day_id` | **belongs to one structure**, because a `period` belongs to one `period_structure` |
| `timeslot_id` alone | one exact slot, and therefore one structure and one term |

So a teacher unavailable "period 1, every day" at a two-schedule school has said something
about **one** schedule. The loader compiles each rule onto the slots it actually names and
widens nothing. The consequence is a named limitation, not a defect: a shared teacher's
period-level unavailability must be entered once per bell schedule they teach under, and the
pre-flight reports a teacher who teaches under more than one structure in scope so the
omission is visible rather than discovered in the timetable.

## Consequences

**Good.**

- The loader has a total, deterministic answer for every activity, from stored facts only.
  No column, no table, no migration.
- The commonest configurations resolve with no campus reasoning at all: one structure in the
  grid, or one school-wide structure, resolves everything under clause 1 or clause 3.
- Scenario C — two campuses, two bell schedules whose Period 1 differ — is *structurally*
  safe. Two structures are two `period` sets are two `timeslot` sets, and a conflict between
  them is decided by minutes, never by ordinal.
- The ambiguous case produces a sentence a user can act on, before a solve rather than after
  one.
- The decision costs nothing it does not need: campus narrows the grid, and it never becomes
  a term in a clash key — which ADR 0014 forbids for rooms.

**Bad, and accepted.**

- A multi-campus school whose sections do not name campuses, and whose bell schedules are
  all campus-scoped, **cannot generate until it finishes saying which campus teaches what.**
  That is the audit's Option B cost, taken deliberately: the alternative is a timetable built
  on a guess the result does not record.
- Two campuses that share timings must either accept one school-wide structure for *all*
  campuses or duplicate the structure per campus. There is no "these two campuses, same
  bells" expression. Duplication works and costs a second `timeslot` set; the interval
  comparison of ADR 0032 rule 2 keeps a shared room safe across it, which is the only place
  the duplication could have been dangerous.
- Period-level availability does not travel between bell schedules. Reported, not solved.

**Neutral.**

- `evara_core::lessons::CycleCapacity` takes the **maximum** across all of a school's
  schedules (`lessons.rs:175-192`), and `capacity_of` picks a cycle by
  `is_default DESC, COUNT(*) DESC LIMIT 1` (`repo/lessons.rs:2416-2428`). Both are
  *optimistic*: they can only under-report impossibility, never invent one, so they stay
  correct under this record and keep honouring "never report that a demand *can* be
  scheduled". A sharper per-structure check becomes possible once resolution exists, and
  belongs with M3's diagnostics rather than here.

## Alternatives rejected

**Resolve through the section's campus and fall back to the school-wide default (the
audit's Option A).** Rejected because the fallback is silent and the configuration it
mishandles — a multi-campus school whose sections leave `campus_id` null — is ordinary
rather than exotic. A wrong answer that looks right is the failure mode this product exists
to avoid.

**Pick the default bell schedule (`is_default = 1`).** Rejected on the schema: `is_default`
is unique per *campus bucket*, not per school, so a school with a school-wide default and a
campus default has two and "the default" is not a value. The interface's own
`pick(..., isDefault)` shows what that produces.

**Add `activity.period_structure_id` or `class_section.period_structure_id` (the audit's
Option C).** Rejected for now. It is a migration, it puts campus information in a third
place, and it would have to be written for every section at a school where the compatibility
relation already answers the question uniquely — which is most of them. It stays the right
answer if a school ever needs two bell schedules **on one campus** for different sections
(a sixth-form running later bells in the same buildings); nothing in this record blocks
adding it, and clause 2 of §2 becomes an override rather than a rule when it arrives.

**Give `period_structure` an association table so several campuses can share one schedule.**
The honest widening, and premature: duplication already works, the audit found no school
asking, and `room-clash` by interval makes duplication safe. Revisit when a user reports the
duplication as a problem, not before.

**Add a campus to timeslot identity so a campus-scoped grid is addressable directly.**
Rejected as a change to ADR 0011's identity tuple, which is load-bearing for
rematerialisation. `G` is derivable by join; identity does not need to carry it.

**Let the solve name its bell schedules and require it always.** Rejected as the *only*
mechanism, because it would make every solve of an unambiguous school answer a question with
one possible answer. Kept as the optional narrowing in §1, which is how an ambiguous school
proceeds without a migration.

## Future acceptance tests

Fixtures are M2's (audit B12); these are the cases they must carry and the outcomes M4 and
M5 must produce. None exists yet.

| # | Fixture | Expectation |
| --- | --- | --- |
| 1 | One campus, one campus-scoped structure, sections with `campus_id = NULL` | Every activity resolves (clause 3). No finding. |
| 2 | One campus, one school-wide structure, sections with `campus_id = NULL` | Every activity resolves (clause 1). |
| 3 | Two campuses, one school-wide structure, sections naming each campus and sections naming none | All resolve to the school-wide structure. |
| 4 | Two campuses, two structures with **identical** timings, every section naming a campus | Each resolves to its own campus's structure. Two disjoint value ranges; a room shared by both campuses still clashes by interval (ADR 0032 rule 2). |
| 5 | Two campuses, two structures with **different** timings, every section naming a campus | As 4. A placement at North P1 08:00–08:45 and one at South P1 08:30–09:15 in the same shared room is **one violation**, although the `timeslot_id`s differ. |
| 6 | As 5, but one section names no campus | `NoBellSchedule`, cause 3, naming the section. The solve is refused. |
| 7 | One school-wide structure **and** the section's campus structure, both materialised into the grid | `AmbiguousBellSchedule` listing both by name. Re-running with the solve narrowed to one of them resolves. |
| 8 | Structure defined but never materialised into `(cycle, term)` | Not in `G`; not a candidate; produces no ambiguity. |
| 9 | Empty grid for `(cycle, term)` | `NoBellSchedule`, cause 1, once for the solve rather than once per section. |
| 10 | `duration_periods = 3` where the resolved structure's longest teaching run is 2, and another structure in the school offers 4 | Refused for that activity. The other structure is irrelevant: a run never spans structures. (Note the *existing* `DemandFinding` takes the school-wide maximum and so stays silent here — correct, because it only under-reports.) |
| 11 | Teacher with `period_id`-scoped unavailability under structure A, teaching sections under A and B | The rule binds A's slots only. A pre-flight warning names the teacher and both structures. |
| 12 | Teacher with `cycle_day_id`-only unavailability, teaching under A and B | The rule binds every slot of that day under both structures. |
| 13 | Backward compatibility: a project created before this record, single campus, grid already materialised | Resolves under clause 3 with no migration, no re-materialisation and no re-entry. |
| 14 | A custom rotating cycle (`kind = 'CUSTOM'`, `day_count = 6`) with two structures | Resolution is unchanged — it reads campus and structure, never a weekday and never `day_count`. |
