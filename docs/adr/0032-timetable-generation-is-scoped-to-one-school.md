# ADR 0032 — Timetable generation is scoped to one school; sharing is compared in real time within it, and reported across it

- Status: **Proposed** — drafted by [`SCHEDULING-READINESS-AUDIT.md`](../SCHEDULING-READINESS-AUDIT.md) §9, awaiting a decision. Nothing is implemented in either direction.
- Date: 2026-10-16
- Decides: [OPEN-DECISIONS](../OPEN-DECISIONS.md) D22, for the intra-school case. Defers its
  cross-school case explicitly rather than leaving it unanswered.
- Builds on: [ADR 0011](0011-stable-timeslot-identity.md) (a timeslot's identity is
  `(term, cycle_day, period)` and carries no campus),
  [ADR 0014](0014-rooms-shared-across-campuses.md) (one room, one identity, so occupancy is
  global), [ADR 0017](0017-weekly-and-custom-cycles.md) (a cycle carries no start date),
  [ADR 0019](0019-multiple-schools-in-one-project.md) (a project holds several schools),
  [ADR 0021](0021-project-level-people-and-school-membership.md) (people belong to the
  project, membership to the school),
  [ADR 0022](0022-resources-shared-across-campuses.md) (a resource may serve many schools),
  [ADR 0023](0023-the-interface-scopes-to-one-school.md) (the interface scopes to one
  school).
- Binds: the solver phase. The `Problem` loader's scope and the `room-clash` /
  `resource-clash` evaluators follow directly from this record.

## Context

A project may describe several schools (ADR 0019). A teacher may be employed by more than
one of them (ADR 0021). A room (ADR 0014) and a resource (ADR 0022) may each be one physical
thing serving campuses of more than one school. Each of those records says, in its own words,
that one identity means one occupancy record — and each of them was written before there was
anything that occupied it.

There is now a solver to design, so the question cannot be deferred further: **is a timetable
generated per school, per project, or something in between?**

### What the schema already decides

Three facts are in the migrations, not in anyone's preference, and they settle more of this
than a discussion would.

**Two schools cannot share a timeslot.** `cycle.school_id` is `NOT NULL`
(`V4__structure_time.sql:42`), `timeslot` derives `school_id`, `cycle_id` and `campus_id`
from the cycle day and period it is built from, and a trigger pair refuses a row where they
disagree (`V4:150-185`). The grid is materialised per `(cycle, period_structure, term)`
(`MaterialisationRequest`, `repo/time_model.rs:1651-1662`). Schools therefore own **disjoint
value ranges**. A solver asked to "solve the project" would be solving N independent problems
that share no variable domain.

**Two schools have no common clock.** `school.timezone` is `NOT NULL` and per school
(`V12__multiple_schools.sql`). No project-level calendar aligns one school's Day A to
another's; `calendar_day` is school-scoped and, since `V4:186-190`, entered explicitly per
date with no automatic rotation. So neither of the two available comparisons works across
schools: cycle positions are not commensurable, and wall-clock times are not commensurable
until a timezone and a date are both resolved. D22's *Widened by `V13`* paragraph says
exactly this.

**Within one school, real time *is* already computable.** One timezone. One set of
`period.starts_at` / `period.ends_at`, stored as `'HH:MM'` and `CHECK`-validated
(`V4:104-105`). Cycle days shared by every bell schedule of that school, because
`period_structure.campus_id` varies while `cycle_day` does not. The intra-school half of D22
needs no new table and no new column.

> D22's statement that *"`period_structure` is school-wide, and nothing ties a campus to
> exactly one"* understates the model: `period_structure.campus_id` exists (`V4:82`) and
> `timeslot.campus_id` is derived from it. D22's **conclusion** is unaffected — timeslot
> identity still carries no campus, so two structures still produce different `timeslot` rows
> for overlapping minutes — but the premise should be corrected when D22 is closed.

### Why the obvious answers are both wrong

**"Solve each school independently"** is implementable today and silently wrong for exactly
the configurations ADR 0014, ADR 0021 and ADR 0022 exist to support. Campus A books the
shared hall at 09:00; Campus B, on a different bell schedule, books it at 09:00 under a
different `timeslot_id`; both timetables report no violation; the hall has two classes in it
on the first day of term. ADR 0014 calls this "a correctness failure" in those words.

**"Solve the whole project"** is not expensive — it is **not implementable on the current
model**. There is no artefact that says two schools' Period 3 on Day A are the same minutes,
so a project-wide solver would have nothing to compare. Building that artefact (a
campus/school time-alignment table, D22's Option C) is the most honest answer and the most
work, and nobody has asked for it.

## Decision

**Generate per `(school, cycle, term)`. Within a school, compare shared occupancy in real
time. Across schools, report the sharing and refuse to pretend.**

### 1. The unit of generation is one school's grid

A solve names a school, a cycle, a term, and the bell schedules in play. This matches what
the schema scopes (`timeslot.school_id`), what the interface scopes (ADR 0023), and what the
repository layer already enforces school-by-school (`require_school_owner`,
`DbError::WrongSchool`).

It follows that **the solver never holds two schools' activities at once**, and so cannot
accidentally equate their cycle positions. The guarantee is structural rather than a rule
somebody must remember.

### 2. Within a school, `room-clash` and `resource-clash` compare intervals, not timeslot ids

Both evaluators key on `room_id` / `resource_id` — never on a campus, per ADR 0014 — and two
placements conflict when they fall on the same `cycle_day` and their `[starts_at, ends_at)`
intervals overlap. Not when they share a `timeslot_id`.

This is D22's **Option A**, restricted to the case where every input it needs already exists.
It costs an interval index built by the `Problem` loader and **no schema change**. It makes a
hall shared by two campuses on different bells safe, which Option B would have achieved only
by refusing a legitimate setup — the kind of refusal that makes a product unusable rather
than merely limited.

For a school whose campuses share one bell schedule, interval comparison and `timeslot_id`
comparison give the same answer, so nothing is paid for the general case.

### 3. Across schools, detect and report; never silently assume

Before a solve, the pre-flight check scans the entities in scope for sharing with another
school:

- a teacher with more than one active `teacher_school` row,
- a room whose `room_campus` rows reach campuses of more than one school,
- a resource whose `resource_campus` rows do likewise,
- a student with concurrent `ENROLLED` spells at more than one school.

Each is reported as a named warning that states the limit plainly — *"Ms Okafor is employed
by Northgate and by Southmoor. Evara cannot compare the two schools' timetables and will not
detect a clash between them."* — and the activity is scheduled anyway.

**It warns; it does not block.** Blocking would make a configuration three ADRs deliberately
support unusable. **It warns; it does not guess.** A silent per-school solve is how a shared
hall gets double-booked while both timetables report themselves clean.

### 4. What this defers, named rather than implied

Coordinated cross-school generation, cross-school clash detection, and project-level
availability (`availability.school_id` is `NOT NULL`, so a shared teacher's unavailability is
entered once per school and nothing reconciles the two). All three become possible once a
project-level time alignment exists — D22's Option C — and none should be built before a
school has asked for it.

## Consequences

**Good.**

- The solver's scope matches the schema's scope, so no code has to assert a boundary the data
  does not have.
- Performance is the best of the three options: N small problems rather than one problem N
  times larger with no shared structure.
- The shared-room correctness failure ADR 0014 named is fixed for the case that actually
  arises, with no migration.
- D22 is closed for the half that can be answered and explicitly, visibly open for the half
  that cannot.
- Unrelated schools stay uncoupled. Two independent schools in one project do not pay for
  each other.

**Bad, and accepted.**

- A shared teacher double-booked across two schools is **not detected**, only announced. This
  is a real hole and this record does not hide it; it is the same hole D22 already carries,
  now with a warning attached.
- The warning in rule 3 is only as good as its wording. A warning a user learns to dismiss is
  worse than none, so it must name the person, both schools and the consequence — not say
  "cross-school scheduling is unsupported".
- Interval comparison is marginally more expensive than integer equality on `timeslot_id`.
  Irrelevant at these problem sizes, and it buys correctness.

**Neutral.**

- `room-clash` remains a per-`room_id`, project-wide rule *in principle*, as ADR 0014
  requires; what this record decides is that the first implementation evaluates it only
  within the school being solved, and says so.

## Alternatives rejected

**Solve the whole project as one problem.** Not implementable: nothing aligns two schools'
cycle days or clocks. Rejected on feasibility, not on cost.

**Solve per school and say nothing (D22's status quo).** Rejected because it is wrong exactly
where the sharing features matter, and wrong silently, which is the failure mode this product
exists to avoid.

**Refuse to share a room between campuses that disagree on a bell schedule (D22 Option B).**
Cheap and safe, and it refuses a real school's setup. ADR 0014 made a room shareable because
modelling one hall as two rooms is a correctness failure; answering that by refusing the
share returns the school to the same place by a different route.

**Build the campus/school alignment table now (D22 Option C).** The most honest model and the
most work, and premature: no user has asked, and the intra-school case — which is the one
that arises — is answerable without it.

**Give `period` an authoritative interval *and* add a campus to timeslot identity.** Rejected
as a change to ADR 0011's identity tuple, which is load-bearing for rematerialisation. The
interval comparison in rule 2 achieves the same result without touching identity.
