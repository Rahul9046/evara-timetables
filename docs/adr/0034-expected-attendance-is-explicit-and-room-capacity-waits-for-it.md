# ADR 0034 — Expected attendance is a stated number or it is unknown; `room-capacity` leaves the first constraint set

- Status: **Accepted**
- Date: 2026-10-21
- Decides: [OPEN-DECISIONS](../OPEN-DECISIONS.md) **D35**, raised by
  [`SCHEDULING-READINESS-AUDIT.md`](../SCHEDULING-READINESS-AUDIT.md) §4.4 and N1.
- Builds on: [ADR 0006](0006-group-based-scheduling-first.md) (a full timetable must be
  buildable from `student_group.size` with no student records at all),
  [ADR 0014](0014-rooms-shared-across-campuses.md) (one room, one identity),
  [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §4 (the groups are
  the scheduling population; the roster is an overlay and the two are never added) and §5
  (`class_section.max_size` was deliberately not built),
  [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md) (whether two groups
  share students is a three-state answer, and usually *indeterminate*).
- Binds: the constraint registry (M5) and `CONSTRAINTS.md`'s `room-capacity` row. Nothing
  is implemented by this record, and **no migration is written by it.**

## Context

`CONSTRAINTS.md` §3 defines `room-capacity` as *"Room capacity ≥ activity's required
capacity"* and marks it `M1`. The right-hand side does not exist.

### What is stored, checked rather than assumed

| Fact | Where |
| --- | --- |
| `room.capacity` is `NOT NULL CHECK (capacity >= 0)` — so **0 is a legal stored value** | `V3:81` |
| `room.is_bookable` exists, is edited by the Rooms screen, and is read by nothing | `V3:82`; no reader outside `repo/structure.rs` and the form |
| `class_section.max_size` **does not exist** | `V15:102-106` records the omission; ADR 0029 §5 argues it |
| `student_group.size` is **nullable**; `NULL` means "membership is the truth" | `V10:131-133` |
| A section may be taught to **several** groups | `class_group`, `V15:662`, `UNIQUE (class_section_id, student_group_id)` |
| `class_enrolment` is a roster overlay with **no scheduling meaning** in Milestone 1 | `V15:716-726`, ADR 0029 §4 |
| The only room *requirement* that exists is `course.required_room_type_id` | `V15:223` |

### The figure that already exists, and why it cannot be the one

`Lessons::class_groups` returns an `AttendingGroup` per attending group carrying
`population: group.size.unwrap_or(named_members)` (`repo/lessons.rs:1396`), and
`ClassSectionDetail.tsx:535` sums those into a displayed **audience**. It is a good number
for the screen it is on and it is unusable as the left-hand side of a hard constraint, for
two independent reasons:

1. **It collapses unknown into zero.** A group with `size IS NULL` and no member rows
   contributes `0`. A section taught only to such groups has a displayed audience of `0`,
   which would satisfy every room in the school including a cupboard.
2. **It double-counts overlap.** Summing a section's groups is correct only if those groups
   are pairwise disjoint, and [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md)
   establishes that the current schema can almost never prove that. A section taught to
   "Year 9" (120) and to "Year 9 Set 1" (28) has an audience of 120, not 148.

Neither is a defect in that screen: it is labelled *"Counts as"* beside *"Declared size"*
and *"Members listed"*, and its note says what it did. Promoting it to a scheduling input is
what would be the defect.

### The three candidate sources, each eliminated on its own terms

- **`class_section.max_size`** is not built, and ADR 0029 §5 did not merely postpone it — it
  reserved the name. ADR 0006's table assigns `class_section.max_size` to **sectioning**:
  the cap used when allocating students to sections, evaluated by `section-max-size` and
  `section-balance` at `M2+`. A cap on allocation and an occupancy figure for a room are
  different quantities that are usually different numbers — a set capped at 30 that 24
  children actually take needs a room for 24 — and giving one column both jobs is how a
  field acquires two meanings and then disagrees with itself.
- **Derivation from `student_group.size`** is sound only where every attending group declares
  a size *and* the groups are provably disjoint. That is a real set of schools, and it is not
  all of them.
- **Derivation from `class_enrolment`** contradicts ADR 0029 §4 and ADR 0006 at once, and
  counts a roster whose completeness nothing asserts.

## Decision

### 1. `room-capacity` is **not** in the first solver's constraint set

Its `type_key` is not registered in M5, it gets no evaluator, no JSON Schema and no
`explain()`, and `CONSTRAINTS.md` moves it from `M1` to `M2+`.

The reason is `CLAUDE.md` §4. A constraint type is not supported until all five of its
artefacts exist, and the fourth is an isolated unit test. A `room-capacity` evaluator
written today could only be tested against a left-hand side that is `Unknown` for every
activity in every honest fixture, so the test would assert that the constraint does nothing.
A registered `type_key` that is configurable in a future UI, appears in a score breakdown
and never fires is worse than an absent one: it tells the user their rooms are being checked.

**What a timetable without it still is.** The MVP hard set keeps `room-clash` and
`room-type-required`, so no room hosts two lessons at once and a lesson that needs a
laboratory gets one. What is not checked is whether the laboratory seats the class. That is
a real gap, it is stated in the generated timetable's provenance (§5), and it is not a
correctness failure of the kind `room-clash` prevents.

### 2. Expected attendance is a distinct, explicit, optional field

The authoritative left-hand side, when `room-capacity` arrives, is

```sql
-- FUTURE. Not in V15 and not written by this record.
ALTER TABLE class_section
    ADD COLUMN expected_attendance INTEGER
        CHECK (expected_attendance IS NULL OR expected_attendance >= 0);
```

- **It means what a room has to hold**: how many people the timetable should assume are in
  the room for a lesson of this section. Not planned capacity, not a cap, not enrolment.
- **It is optional**, and `NULL` means *unknown* — never zero. ADR 0006 forbids making any
  of this a prerequisite for a timetable, and a form that insisted would be answered with a
  guess, which is the failure `course.periods_per_cycle` is nullable to avoid.
- **It is a section-level field, not a course-level one**, because two sections of one course
  routinely differ in size and the room each needs follows the section.
- `max_size` stays reserved for M2's sectioning. Two columns, two meanings, neither doing
  the other's job.

**Migration cost, stated so it is not discovered later.** One `ALTER TABLE ... ADD COLUMN`.
`class_section` is already in `change_log::TRACKED_TABLES` and the change-log triggers
interpolate only the table name, `id` and `rev` (`change_log.rs:106-134`), so adding a column
needs no trigger work and no ADR 0020 rebuild. Existing projects read `NULL` and are
unaffected. It carries no revision question: it is a new column on existing rows, and setting
it afterwards is an ordinary `update_class_section` carrying that row's `rev`.

### 3. Derivation is allowed only where it is sound, and yields three states

```
expected_attendance(section) -> Known(n) | Unknown
```

```
if section.expected_attendance IS NOT NULL      -> Known(that)      # explicit always wins
groups := class_group rows of the section
if groups is empty                              -> Unknown
if every g in groups has g.size IS NOT NULL
   and groups are pairwise Disjoint (ADR 0035)   -> Known(Σ g.size)
otherwise                                       -> Unknown
```

Four properties of that rule are deliberate.

- **The explicit value wins even when a derivation is available, and the two are never
  reconciled.** A school that states 24 for a section taught to a group of 28 has said
  something — a set that shares a room with another, a cohort with absentees — and a
  derivation that silently overrode it would be refusing a true statement. Where they
  differ, the difference is *reported as advice*, which is exactly how ADR 0029 §3 treats
  `course.periods_per_cycle` against `SUM(duration_periods)`.
- **A declared `size` is required; a member count is not accepted.** `V10` says `NULL`
  means membership is the truth, and that is right for the screen that displays it, but
  nothing in the schema asserts a roster is *finished*. A partially entered roster
  understates the audience, which makes a hard capacity check too **permissive** — it would
  pass a room too small while reporting no violation. The remedy is one number on one group,
  and the pre-flight names it.
- **Overlapping groups make it `Unknown`, not a sum.** A sum over groups that may share
  students is not a conservative estimate in either direction; it is a wrong number. ADR 0035
  supplies the three-state disjointness answer this clause consumes, and `Indeterminate`
  reaches here as `Unknown`.
- **`Unknown` is never `0` and never a default.** It is a third value that propagates, and
  every consumer below handles it explicitly.

**A lower bound is useful and is not the same thing.** Where a group's `size` is `NULL` and
it has `k > 0` member rows, `k` is a floor on that group's population. The maximum floor
across a section's groups is a sound lower bound on its attendance, and it may be used for a
**warning** — *"Room Lab 2 seats 24 and at least 26 students are listed in this section's
groups"* — never for a hard refusal and never as `Known`. A bound that can only be too small
can raise a flag; it cannot decide a HARD constraint.

### 4. When attendance is unknown, the constraint does not apply to that activity

Of the three behaviours the question admits — block generation, block room assignment, or
exclude the constraint — the answer is **exclude it, per activity, and say so by name.**

- **It does not block generation.** ADR 0006 guarantees a timetable to a school that has
  entered no students and may have declared no sizes. A capacity decision that made such a
  school unable to generate would revoke a commitment three records deep, to gain a check
  the school never asked for.
- **It does not block room assignment.** A lesson with no room is worse than a lesson in a
  room of unverified size, and `room-type-required`, `room-clash` and campus eligibility all
  still constrain which room it gets.
- **It is not "satisfied".** `Unknown` is not a pass. The activity is recorded as
  *not evaluated* for this rule, which is a different outcome from zero violations and must
  be rendered as one.

### 5. A timetable says what was checked

A generated timetable carries, and its views show, the count of activities whose capacity was
**not** evaluated and why. Pre-flight reports the same before the solve, per class section,
with the remedy:

> *"12 class sections have no expected attendance and no complete set of declared group
> sizes. Room sizes will not be checked for them. Enter an expected attendance, or declare
> a size for Year 9 Set 1 and Year 9 Set 2."*

And, because a stored number is never reinterpreted but a suspicious one is worth naming:

> *"4 bookable rooms have a capacity of 0. If that is not deliberate, enter their capacity."*

`room.capacity = 0` means zero seats, and a `Known` check against it fails. Nothing reads 0
as "not entered" — substituting a default for a stored value is the failure
`DbError::CorruptIdentifier` exists to refuse in the identifier case, and the same principle
applies to a number.

### 6. `is_bookable` is part of the candidate set, not of capacity

`room.is_bookable = 0` removes a room from every activity's candidate set. It is stored,
editable and currently unread; it belongs to the loader's room domain alongside campus
eligibility, and it is unrelated to how many people fit. Stated here because the audit found
the column unread and a reader of this record will be building the room domain.

## Consequences

**Good.**

- M5 ships no constraint it cannot test. The definition of done in `CLAUDE.md` §4 holds for
  every registered type, which is the property that makes the registry trustworthy.
- ADR 0006 survives untouched: a size-only school and a student-less school both generate.
- The expensive mistake — overloading `class_section.max_size` with two meanings, or
  summing overlapping groups into a confident number — is not made.
- The eventual schema change is one nullable column with no trigger work, no rebuild and no
  effect on existing projects. Deciding it now is what keeps it that small.
- A school that *does* want capacity checking has a short, named path to it: one number per
  section, or declared sizes on provably disjoint groups.

**Bad, and accepted.**

- The first solver can place a class of 32 in a room for 20. The school will see it in the
  grid and in the unchecked-capacity count, and nothing will have claimed otherwise.
- `CONSTRAINTS.md`'s `M1` count drops by one, and a reader comparing Evara's constraint list
  to a competitor's will find `room-capacity` missing. That is accurate rather than
  flattering.
- The lower-bound warning of §3 is a second, weaker number in the interface. It earns its
  place only if it is labelled as a floor; a warning read as a measurement would be worse
  than no warning.

**Neutral.**

- Nothing in the first solver reads `student_group.size` for capacity. It is read by
  `AttendingGroup.population` for display and by ADR 0035's emptiness test, and those stay
  as they are.

## Alternatives rejected

**Add `class_section.max_size` now and use it.** Rejected twice over: the name is reserved
for M2 sectioning with different semantics (ADR 0006), and a migration in a documentation
milestone would be a migration written before its reader exists — the mistake `V14` named
about `subject.colour` and `V15` named about `class_teacher.role`.

**Derive from groups and accept that it is absent for size-less groups.** This is the
tempting one, because it ships `room-capacity` in M5 for most schools. Rejected because
"absent" was going to arrive as `0` through the existing `size.unwrap_or(named_members)`
path, and because it would sum overlapping groups — making the constraint wrong, not merely
unavailable, for combined classes, which are ordinary.

**Derive from `class_enrolment`.** Rejected on ADR 0029 §4 and ADR 0006: it has no
scheduling meaning in Milestone 1, it is optional everywhere, and counting it would make a
school that names students get a different timetable from one that does not.

**Treat `Unknown` as `0`.** Every room passes. This is the specific failure the decision
exists to prevent, and it is the current displayed behaviour, which is why the record says
so explicitly rather than assuming nobody would.

**Treat `Unknown` as "fits nothing" and refuse to assign a room.** Safe and unusable: it
would deny rooms to exactly the schools ADR 0006 protects, and produce a timetable of
roomless lessons.

**Block generation until every section states an attendance.** Rejected as a change of
product: it converts an optional field into a prerequisite and contradicts ADR 0006's
promise in one step.

**Put the capacity requirement on the activity rather than the section.** Rejected for the
same reason ADR 0029 §5 rejected `activity_teacher` and `activity_group`: four occurrences
of one section are attended by the same people, so a per-occurrence figure is four places
for one fact to disagree with itself.

## Future acceptance tests

Fixtures are M2's; these are the cases they must carry. None exists yet, and the first four
must hold **before** `room-capacity` is implemented, because they are assertions about the
derivation, not about the constraint.

| # | Fixture | Expectation |
| --- | --- | --- |
| 1 | Section with explicit `expected_attendance = 24`, one group of 28 | `Known(24)`. The explicit value wins; the 4-person difference is reported as advice, not as an error, and nothing is overridden. |
| 2 | Section with no explicit value, two groups, both with declared sizes, **provably disjoint** | `Known(sum)`. |
| 3 | Section with no explicit value, two groups, both with declared sizes, overlap `Indeterminate` | `Unknown`. A sum is never produced. |
| 4 | Section with no explicit value, one group, `size IS NULL`, **no** member rows | `Unknown` — never `0`. Pins the exact collapse the existing display performs. |
| 5 | As 4 but with 26 member rows | `Unknown` for the hard constraint, **and** a lower bound of 26 available to the warning. |
| 6 | Section with no attending groups and 20 `class_enrolment` rows | `Unknown`. The roster contributes nothing (ADR 0029 §4), and the section is reported as having named students but no attending group. |
| 7 | Section taught to "Year 9" (120) **and** "Year 9 Set 1" (28), a parent and its child | `Unknown`, not 148 and not 120 — the pair is `Overlaps`, so the set is not disjoint and no sum is sound. |
| 8 | `Known(30)` against a room of capacity 30 | Satisfied. Boundary: `capacity >= attendance`, not `>`. |
| 9 | `Known(31)` against a room of capacity 30 | One violation, explained with both figures and the room's name. |
| 10 | `Known(n)` against a bookable room of capacity 0 | Violation. 0 is a number, not an absence. Separately, pre-flight names the room as a likely unentered capacity. |
| 11 | `Unknown` against any room | **Not evaluated.** Distinguishable in the score breakdown and the violation report from "evaluated, zero violations". |
| 12 | A whole school where every section is `Unknown` | Generation succeeds, produces a timetable, and reports the unchecked count. It does not fail, and it does not claim capacity was checked. |
| 13 | `room.is_bookable = 0` | The room is in no candidate set, independent of capacity. |
| 14 | Backward compatibility: a project created before the future column exists | Reads as `NULL` for every section, so `Unknown` or a sound derivation. No re-entry, no re-materialisation. |
| 15 | A room shared between two campuses of one school, `Known(n)` on both sides | Capacity is a property of the one `room.id` (ADR 0014); both activities compare against the same number. |
