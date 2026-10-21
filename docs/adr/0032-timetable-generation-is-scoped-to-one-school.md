# ADR 0032 — Timetable generation is scoped to one school; sharing is compared in real time within it, and reported across it

- Status: **Accepted** (2026-10-21). Drafted by
  [`SCHEDULING-READINESS-AUDIT.md`](../SCHEDULING-READINESS-AUDIT.md) §9 as **Proposed**;
  every premise was re-verified against the implementation in M1 and one was strengthened
  (§"Two schools have no common clock" — the workspace holds no timezone database at all).
  Nothing is implemented in either direction.
- Date: 2026-10-16; accepted and revised 2026-10-21
- Decides: [OPEN-DECISIONS](../OPEN-DECISIONS.md) D22, for the intra-school case. Defers its
  cross-school case explicitly rather than leaving it unanswered.
- Completed by: [ADR 0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
  (which bell schedule an activity draws its slots from — rule 2's interval comparison needs
  that resolved first), [ADR 0034](0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
  (a shared room's capacity), [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md)
  (a shared student's clashes, and why rule 3's student case is a warning).
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
(`V12:76`). No project-level calendar aligns one school's Day A to another's; `calendar_day`
is school-scoped and, since `V4:186-190`, entered explicitly per date with no automatic
rotation. So neither of the two available comparisons works across schools: cycle positions
are not commensurable, and wall-clock times are not commensurable until a timezone and a
date are both resolved. D22's *Widened by `V13`* paragraph says exactly this.

It is in fact one step worse than D22 recorded, and the stronger fact is why this record can
close the question rather than keep weighing it. **The workspace cannot resolve a timezone at
all.** `school.timezone` is `TEXT` with a non-empty `CHECK` and nothing else — documented as
IANA at `repo/structure.rs:70`, validated as free text, and parsed by no code in the
repository. The only date-time dependency is `time = { version = "0.3", features =
["formatting", "parsing", "macros"] }` (`Cargo.toml:47`), which ships no timezone database;
`chrono` exists behind `calamine`'s `dates` feature and, per `Cargo.toml:78`, *"never leaves
`evara-import`"*. Comparing two schools' wall-clock bells is therefore not an unwritten
algorithm but an unavailable one: it would need a new dependency, a licence review under
`CLAUDE.md` rule 4, and a decision about what a project does when a stored zone name is not
a zone. None of that is work this record should trigger, and all of it is work the
intra-school case does not need, because one school has one zone and the zone cancels on
both sides of every comparison.

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
availability (`availability.school_id` is `NOT NULL` — `V11:44` — so a shared teacher's
unavailability is entered once per school and nothing reconciles the two). All three become
possible once a project-level time alignment exists — D22's Option C — and none should be
built before a school has asked for it.

### 5. Four scopes, which are not the same scope

The word "scope" does four jobs in a solve, and conflating any two of them is how a
timetable comes to be described as something it is not. Added on acceptance, because the
rules above are only safe if these are kept apart.

| Scope | Extent | Authority |
| --- | --- | --- |
| **Optimisation** — what the solver may move | One school's activities, over one `(cycle, term)` grid, drawing slots per [ADR 0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md) | rule 1 |
| **Validation** — what a violation can be computed over | The same school's activities, plus the full occupancy of every `room_id` and `resource_id` *that school's* placements touch | rule 2 |
| **Persistence** — what a saved timetable covers | One school, one cycle, one term. A timetable row is never project-wide | rule 1 |
| **Warning** — what is *announced* but not computed | Entities in scope that another school also holds | rule 3 |

**What can be validated within one school**, from stored data and with no new table:
teacher clash, student clash (to the extent [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md)
proves it), room clash and resource clash by real-time interval, room type, campus
eligibility, availability, duration contiguity and non-teaching periods.

**What can be compared across schools: nothing in time.** Two facts can be *detected* —
that an entity is shared, and that another school holds placements naming it — and neither
is a comparison. A shared `room_id` with placements in two schools' timetables cannot be
checked for overlap, because deciding whether School A's Day 3 Period 2 and School B's Day
1 Period 5 are the same minutes needs a date mapping and a timezone resolution, and §"Two
schools have no common clock" shows both are absent.

**Calendar-date alignment is not required for anything this record decides, and is not
sufficient for what it defers.** Rule 2's comparison is within one school, where every bell
schedule materialises against the same `cycle_day` rows (`cycle_day` belongs to the cycle,
not to a `period_structure`), so the day axis is shared without `calendar_day` being
consulted at all. **An incomplete calendar mapping therefore has no effect on any clash
decision** — which matters, because `calendar_day` is entered one date at a time
(`V4:186-190`) and is incomplete in most real projects. It would become *necessary* for the
cross-school case and still not be *enough*, since a timezone database would be needed too.

**Timezone differences are handled by not crossing them.** One school, one
`school.timezone`; both sides of every interval comparison are in it; the zone cancels and
is never read. No code resolves a zone name, and nothing in the MVP needs to.

**Physical-time interval comparison, exactly.** Two placements of one school overlap when
they fall on the same `cycle_day` and
`a.starts_at < b.ends_at AND b.starts_at < a.ends_at`. `period.starts_at` / `ends_at` are
zero-padded `'HH:MM'` under `CHECK (ends_at > starts_at)` (`V4:104-105`, `V4:111`), so no period
crosses midnight and text comparison *is* chronological comparison. A multi-period
occurrence's interval runs from its first period's `starts_at` to its last period's
`ends_at`. Different cycles of one school are different grids and are never compared; a
solve names one cycle.

**When the other school has no timetable, has a draft, or has several**, the answer is the
same in all three cases, and it is the same answer as when it has a published one:
**rule 3's warning does not read it.** The warning is a property of the *sharing* — a second
active `teacher_school` row, a `room_campus` set spanning two schools' campuses — not of any
other school's output. This is deliberate: a warning that appeared only once a second
timetable existed would be absent exactly while the first school was generating, which is
when it is needed, and a warning whose text depended on another timetable's `status` would
imply Evara had compared them. It has not and cannot.

**The warning is informational and does not block.** Rule 3 says so and the reason bears
repeating here because it is the one place this record chooses visibility over safety:
blocking would make a configuration ADR 0014, ADR 0021 and ADR 0022 each deliberately
support unusable, and the remedy a block would demand — coordinate two schools' timetables —
is the thing Evara cannot do. Compare
[ADR 0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §3, where an
unresolved bell schedule **does** refuse the solve: there the remedy exists and is one
field, so refusing is a question the user can answer rather than a wall.

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

## Future acceptance tests

Added on acceptance. Fixtures are M2's (audit B12); these are the cases they must carry.
None exists yet.

| # | Fixture | Expectation |
| --- | --- | --- |
| 1 | One school, one campus, one bell schedule, a room used twice in one slot | One `room-clash` violation. Interval comparison and `timeslot_id` equality agree, so nothing is paid for the general case. |
| 2 | One school, two campuses on **different** bells, a hall shared by both, booked at overlapping minutes under **different** `timeslot_id`s | **One violation.** The case ADR 0014 calls a correctness failure, and the whole reason rule 2 compares intervals. |
| 3 | As 2, booked at adjacent but non-overlapping minutes (08:00–08:45 and 08:50–09:35) | No violation. Asserts the comparison is half-open, not merely same-day. |
| 4 | As 2 for a `resource` with `quantity = 1` | Same outcome. Whatever answers this for a room answers it for a resource (D22, *Widened by `V13`*). |
| 5 | One school, two campuses, a teacher eligible for both, placed at overlapping minutes under different `timeslot_id`s | One `teacher-clash` violation, by interval. |
| 6 | **Two schools**, one shared teacher with two active `teacher_school` rows | Generation for school A succeeds; a pre-flight warning names the teacher **and both schools** and states the limit. School B's timetable is never read, whether it exists, is a `DRAFT`, or there are several. |
| 7 | As 6 with no timetable at all for school B | Identical warning. Asserts the warning is a property of the sharing, not of another timetable. |
| 8 | **Two schools** sharing a room via `room_campus` across their campuses | Warning naming the room and both schools; the room still clashes correctly *within* the school being solved. |
| 9 | **Two schools** with concurrent `ENROLLED` spells for one student, in groups at both | Warning naming the student and both schools. No cross-school clash is computed ([ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md) §7). |
| 10 | **Two schools with different academic calendars** — different `academic_year`, `term` and `calendar_day` rows — sharing a teacher | Both generate independently. Nothing compares their grids, and no finding claims otherwise. |
| 11 | One school whose `calendar_day` coverage is **empty** | Every clash decision is unaffected; generation succeeds. Asserts that an incomplete calendar mapping is not an input to any rule this record decides. |
| 12 | A generated timetable for a school with shared entities | Its provenance and its views carry the shared-entity warnings. It is described as having no confirmed clash *within this school*, never as conflict-free. |
| 13 | Two unrelated schools in one project, nothing shared | Neither solve reads the other's rows; no warning is produced; neither pays for the other. |
