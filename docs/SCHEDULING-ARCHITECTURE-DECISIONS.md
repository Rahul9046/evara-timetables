# Scheduling Architecture Decisions — M1

- Date: 2026-10-21
- Base commit: `57cf4ac` (`origin/develop`, "Merge pull request #24 from
  Rahul9046/docs/scheduling-readiness-audit")
- Schema version: **`V15`** — unchanged by this milestone
- Milestone: **M1 — Scheduling decisions**, the decisions-only milestone
  [`SCHEDULING-READINESS-AUDIT.md`](SCHEDULING-READINESS-AUDIT.md) §15 and §17 recommend
  before any solver code

**This milestone implements nothing.** No migration, no Rust, no TypeScript, no constraint,
no solver, no screen. It closes four decisions, writes the contracts the next milestones
build against, and records which future schema and domain changes those contracts require.

Throughout, three categories are kept visibly apart and never blurred:

| Marker | Meaning |
| --- | --- |
| **Implemented** | Exists in the repository at `V15` and is covered by tests |
| **Accepted future design** | Decided here, built later, in the milestone named |
| **Deferred** | Not decided, not built, and explicitly not promised |

---

## 1. The four decisions

| ID | Question | Answer | Record |
| --- | --- | --- | --- |
| **D34** | Which bell schedule does an activity schedule against? | Exactly one, resolved by **campus compatibility** against the structures actually materialised into the grid. A non-unique resolution is a hard pre-flight error naming the candidates. No fallback, no default. | [ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) |
| **D35** | What does `room-capacity` compare against? | Nothing yet — **`room-capacity` leaves the first constraint set.** Expected attendance becomes an explicit optional `class_section.expected_attendance`, derived only from declared sizes over provably disjoint groups, and `Unknown` otherwise. `Unknown` excludes the constraint for that activity and is reported. | [ADR 0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) |
| **D12** | When do two student groups clash? | Overlap is a **three-state** function: `Overlaps` / `Disjoint` / `Indeterminate`. Only `Overlaps` is a hard clash; `Disjoint` must be proved; `Indeterminate` is not enforced, never silently converted, and reported with an actionable remedy. Needs one nullable column. | [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) |
| **D22 / ADR 0032** | Is a timetable generated per school or per project? | **Per `(school, cycle, term)`.** Within a school, shared occupancy compares real time. Across schools, sharing is reported and never guessed at. Status moved **Proposed → Accepted**; every premise re-verified and one strengthened. | [ADR 0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) |

### How they depend on one another

```
ADR 0032  one school, one cycle, one term
   │          the boundary everything else is stated inside
   ├──► ADR 0033  which bell schedule → the value range, and the wall-clock intervals
   │              that ADR 0032 rule 2 compares
   │                    │
   │                    └──► duration-contiguous, no-teaching-in-period, room/resource clash
   │
   ├──► ADR 0035  group overlap → student-clash
   │                    │
   │                    └──► ADR 0034  a sum over groups is sound only when they are Disjoint
   │
   └──► ADR 0034  expected attendance → room-capacity, deferred
```

Three of the four edges are load-bearing and not merely topical:

- **0032 → 0033.** Rule 2 compares `[starts_at, ends_at)` intervals. That is only meaningful
  once each activity has *one* bell schedule to take its times from, which is 0033's job.
  Without 0033, 0032 rule 2 has two possible interpretations per activity.
- **0035 → 0034.** 0034's derivation sums a section's groups. A sum over groups that might
  share students is a wrong number, so 0034 consumes 0035's `Disjoint` verdict directly and
  degrades to `Unknown` on `Indeterminate`.
- **0032 → 0035 §7.** Overlap is computed only among one school's groups. That is structural
  rather than remembered: `student_group.school_id` is `NOT NULL` and a parent must share its
  child's school (`V12:172-176`), so a subtree never crosses a school boundary.

### They do not contradict each other

Checked deliberately, because three records written together is where a contradiction hides.

| Possible conflict | Resolution |
| --- | --- |
| 0033 refuses a solve on ambiguity; 0032 and 0035 only warn | Different kinds of unknown. 0033's remedy is **one field on one record** and the solve cannot proceed meaningfully without it (there is no value range). 0032's cross-school remedy **does not exist** and 0035's is a setup task that every school starts without. Refusing where the user can answer, warning where they cannot, is one rule applied three times — stated at [ADR 0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) §5, last paragraph. |
| 0034 defers `room-capacity`; 0035 keeps `student-clash` | Deliberate and argued at [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md), "Alternatives rejected", last entry: an unverified room size is a timetable with a caveat; a child in two rooms at once is not a timetable. |
| 0034 and 0035 each require a column; the records say no migration is written | Both specify a **future** `ALTER TABLE ... ADD COLUMN` and neither writes one. §4 collects them. |
| 0033 narrows the value range by `is_teaching`; `no-teaching-in-period` is still a constraint | Pre-filter, not replacement. Both read the single stored `timeslot.is_teaching`, derived from the single `period.kind`, so they cannot disagree — `CLAUDE.md` §2's rule applied to a domain restriction. |
| 0035's `effective_members` excludes `LEFT` students; `V13` admits them to `student_group_member` on purpose | Both are right about different questions. The row is history (ADR 0018); the room is now. The filter is at read time, in one place, and changes no stored data. |

---

## 2. The scheduling context

**Accepted future design.** The minimum a solve needs, every reference resolved from `V15`
facts. This is the contract M4's loader implements; it defines no Rust type and the field
names below are descriptive, not a declaration.

### 2.1 The solve's own scope

| Input | Source | Resolution |
| --- | --- | --- |
| **School** | named by the caller | ADR 0032 rule 1. Every school-owned read takes it explicitly and refuses another school's identifier (`require_school_owner`, `DbError::WrongSchool`) |
| **Term** | named by the caller, nullable | `NULL` means the grid that applies all year. Part of timeslot identity, so it selects the grid rather than filtering it |
| **Cycle** | named by the caller | `cycle.school_id` must be the school. Never defaulted — `GridPreviewSection.tsx:93-99` already refuses to default it, and for the reason given there |
| **Bell schedules** | **derived** | `G` = the distinct `period_structure` rows with materialised slots in `(cycle, term)`, per [ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §1. Optionally narrowed by the caller to resolve an ambiguity |

### 2.2 Per activity

| Input | Source | Resolution |
| --- | --- | --- |
| **Activity identity** | `activity.id`, UUIDv7, stable | One row is one placeable occurrence (ADR 0029 §2). The variable set **is** the activity rows |
| **Class section** | `activity.class_section_id`, `NOT NULL` | The school comes through `class_section → course → subject` (ADR 0029 §1). No shortcut column is acquired |
| **Duration** | `activity.duration_periods`, `NOT NULL DEFAULT 1` | Consecutive teaching periods of the resolved structure, one cycle day ([ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §5) |
| **Bell schedule** | resolved | Campus compatibility against `G` ([ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §2–§3). One per section, so every occurrence of a section resolves identically |
| **Candidate timeslots** | `timeslot` rows of the resolved structure in `(cycle, term)` | Restricted to `is_teaching = 1` |
| **Teachers** | `class_teacher` of the section | **Fixed, not chosen** (D7). Employment and campus eligibility are already database invariants under two trigger pairs (`V15:507`, ADR 0028), so the solver cannot pick an ineligible teacher because it picks none |
| **Student population** | `class_group` of the section | A set of **groups**, not of students ([ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) §5). `class_enrolment` contributes nothing (ADR 0029 §4) |
| **Room requirement** | `course.required_room_type_id`, nullable | The only requirement that exists (`V15:223`). `NULL` means no room type is required |
| **Room candidates** | derived | Rooms associated through `room_campus` with a campus of this school; narrowed to the section's campus where it names one; `is_bookable = 1`; matching the required room type where one is named |
| **Expected attendance** | `Known(n)` or `Unknown` | [ADR 0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) §3. Not consulted by the MVP, because `room-capacity` is not in it |
| **Availability** | `availability` rows for the teachers, rooms and groups in scope | Compiled per granularity onto the slots each rule actually names, never widened ([ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §7). `school_id` is `NOT NULL`, so a shared teacher's other school's rules are invisible — reported, per ADR 0032 rule 3 |

### 2.3 Candidate placement

**Accepted future design**, conceptual only — M2 declares the type, and
[`SOLVER.md`](SOLVER.md) §2's `Placement { timeslot: u32, room: Option<u32> }` already has
the right shape. What this milestone fixes is what a placement *means*:

| Element | Content | Note |
| --- | --- | --- |
| Activity | one `activity.id`, as a dense index | The whole occurrence, not a fraction of one |
| Cycle position | the `cycle_day` of the starting slot | `cycle_day.ordinal` is a day's only scheduling identity. **Nothing reads a weekday** |
| Start period | the starting `timeslot` | Carries its structure through `period → period_structure` |
| Duration | `duration_periods` slots | Ordinal-consecutive teaching periods of **one** structure. A placement occupies all of them |
| Physical interval | `[first.starts_at, last.ends_at)` | Wall clock, one school's timezone, derived not stored. The comparison key for shared occupancy |
| Room | `Option<room_id>` | `None` where the course requires no room type and none was assigned |

Two planning variables — timeslot and room. Teacher is not one (D7), and
[`SOLVER.md`](SOLVER.md) §2's note that a third is additive stands.

---

## 3. Validation contracts

### 3.1 Conflict semantics

**Accepted future design.** The rule that makes all five rows safe:
**different `timeslot_id`s do not mean non-overlapping activities.**

| Compare | Key | Test |
| --- | --- | --- |
| **Teachers** | `teacher_id` | Same `cycle_day` **and** overlapping `[starts_at, ends_at)`. Not `timeslot_id` equality — a teacher may teach under two bell schedules of one school |
| **Students** | group indices | `clash_mask(A) ∩ audience_mask(B) ≠ ∅`, where the masks are built from [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) §3's three-state relation. Only `Overlaps` contributes. Time compared as above |
| **Rooms** | `room_id`, project-wide in principle (ADR 0014), evaluated within the solved school (ADR 0032) | Same `cycle_day` **and** overlapping interval. **Never per campus and never with a campus in the key** |
| **Resources** | `resource_id` against `resource.quantity` | As rooms. Not in the MVP — no table links a resource to an activity (audit N4) |
| **Period intervals** | `(cycle_day_id, starts_at, ends_at)` | `a.starts_at < b.ends_at AND b.starts_at < a.ends_at`. Zero-padded `'HH:MM'` under `CHECK (ends_at > starts_at)`, so text order is chronological and no period crosses midnight |
| **Campus eligibility** | — | Already a database invariant for staffing (`V15:507` trigger pairs). For rooms it is a **candidate-set** restriction (§2.2), so an ineligible room is never offered |

Where interval comparison and `timeslot_id` equality agree — a school whose campuses share
one bell schedule, which is most schools — nothing is paid for the general case.

**One requirement M5 must not lose.** The room candidate-set rules of §2.2 (`is_bookable`,
campus association, required room type) are a *domain*, and a domain the manual editor could
bypass is a rule living in two places. `room-type-required` already covers the third;
`is_bookable` and campus association need either their own registered `type_key` or a
documented guarantee that M11's `validate_move` consults the same loader-built candidate set.
This is named here so M5 decides it rather than discovers it; it is not decided by this
milestone.

### 3.2 Unknown and incomplete inputs, classified

**Accepted future design.** Every row is one of four outcomes, and none is "assume it is
fine". The classification exists so that no future reader has to re-derive it per case.

> **Implemented in M3**, except where the row's information is not in a compiled `Problem`.
> Every *hard validation error* and every *explicit warning* below is produced by
> `evara_core::scheduling::validate`, with two exceptions owned by M4's loader — the two
> availability rows, per [ADR 0037](adr/0037-stored-row-diagnostics-belong-to-the-loader.md) —
> and one owned by M5, the group-overlap contradiction, which needs member identities. Every
> *no effect* row produces no finding, and every *deferred* row is still unimplemented.
> [PRE-SOLVE-VALIDATION.md](PRE-SOLVE-VALIDATION.md) §6 is the per-code ownership table.

| Input condition | Classification | Behaviour | Record |
| --- | --- | --- | --- |
| No grid materialised for `(cycle, term)` | **Hard validation error** | Solve refused once, before any activity is examined | 0033 §3 |
| Activity resolves to **no** bell schedule | **Hard validation error** | Solve refused, naming the section and one of three causes | 0033 §3 |
| Activity resolves to **more than one** bell schedule | **Hard validation error** | Solve refused, naming every candidate and two remedies | 0033 §3 |
| Section names no campus, school has one campus | *not an unknown* | Resolves (clause 3). The ordinary case | 0033 §2 |
| Section names no campus, school has several, no school-wide schedule | **Hard validation error** | Solve refused. Remedy: name the campus, or make a schedule school-wide | 0033 §3 |
| `duration_periods` exceeds the resolved structure's longest teaching run | **Hard validation error** | That activity cannot be placed by any solver. Arithmetic, reported as such | 0033 §5 |
| Expected attendance `Unknown` | **Explicit warning** | `room-capacity` is not evaluated for that activity — not satisfied, not violated. Counted in pre-flight and on the timetable | 0034 §4–§5 |
| A group has `size IS NULL` with member rows | **Explicit warning** | A lower bound only. Never `Known`, never a hard refusal | 0034 §3 |
| Bookable room with `capacity = 0` | **Explicit warning** | The stored number is used as written. Never reinterpreted as "unset" | 0034 §5 |
| Group overlap `Indeterminate` | **Explicit warning** | Not enforced. Pre-flight gives the pair count *and* the number of answers that would clear it; the timetable carries the unverified count | 0035 §6 |
| Group overlap contradiction (shared member against a `children_are_disjoint = 1` parent) | **Explicit warning** | Facts win: the pair is `Overlaps`, and the contradiction is named with its rows | 0035 §3 |
| Section with `class_enrolment` rows and no `class_group` | **Explicit warning** | Empty audience, no clash protection, remedy named | 0035 §7 |
| Teacher teaching under more than one bell schedule in scope | **Explicit warning** | Period-level unavailability does not travel between schedules | 0033 §7 |
| Entity shared with another school (teacher, room, resource, student) | **Explicit warning** | Never blocks. The other school's timetable is not read, whatever its status | 0032 §5, rule 3 |
| `calendar_day` coverage incomplete or absent | *no effect* | No clash decision reads a date. Stated so nobody treats it as a prerequisite | 0032 §5 |
| Cross-school clash in real time | **Deferred, unsupported** | Not computed. Needs a time alignment **and** a timezone database, neither of which exists | 0032 §4 |
| Project-level availability for a shared teacher | **Deferred, unsupported** | `availability.school_id` is `NOT NULL`; nothing reconciles two schools' rules | 0032 §4 |
| Individually enrolled student's clashes | **Deferred, unsupported** | `class_enrolment` has no scheduling meaning in M1 (ADR 0029 §4) | 0035 §7 |

Read as a whole, the table says one thing: **a hard error is reserved for an input whose
remedy is a field the user can fill in; everything else is counted and announced; nothing is
assumed.**

---

## 4. Required future changes

### 4.1 Database

**Accepted future design — not written by this milestone, and `V15` is unchanged.**

| Change | Decision | Milestone | Cost |
| --- | --- | --- | --- |
| `student_group.children_are_disjoint INTEGER NULL CHECK (… IN (0,1))` | [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) §4 | **M5**, in the migration that already adds `constraint_instance` / `constraint_ref` | One `ADD COLUMN`. **Required** — `student-clash` degenerates without it |
| `class_section.expected_attendance INTEGER NULL CHECK (… >= 0)` | [ADR 0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) §2 | **M2+**, with `room-capacity` | One `ADD COLUMN`. **Optional** — nothing in the MVP reads it |

Both tables are already in `change_log::TRACKED_TABLES`, and the change-log triggers
interpolate only the table name, `id` and `rev` (`change_log.rs:106-134`), so neither needs
trigger work and neither is an [ADR 0020](adr/0020-in-place-table-rebuild.md) rebuild.
Existing projects read `NULL` on both, which is the behaviour each record defines for them.

**No change is needed to anything that exists.** `timeslot.id` is stable by construction
(ADR 0011), `activity.id` is stable, `room.id` is one identity per physical room (ADR 0014),
and all four decisions are stated over `V15` as it stands. The audit's §7 conclusion —
*"nothing in the existing schema needs changing"* — survives M1.

**Unchanged by deliberate choice**, so a later reader does not propose them as oversights:

- No `activity.period_structure_id` and no `class_section.period_structure_id`. The
  compatibility relation answers D34 without one, and
  [ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) names the
  single future case — two bell schedules on one campus — that would justify it.
- No association table letting a *subset* of campuses share one bell schedule. Duplication
  works and interval comparison makes it safe.
- No `class_section.max_size`. Reserved for M2 sectioning with different semantics (ADR 0006).
- No containment trigger between a child group's members and its parent's. Rejected with
  reasons in [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md).
- No campus in timeslot identity. ADR 0011's tuple is load-bearing.

### 4.2 Domain model

**Accepted future design.** M2 declares these in `evara-core` (audit B3); M1 fixes their
semantics.

| Type | Shape this milestone decides | Record |
| --- | --- | --- |
| `TimeslotFact` | Carries its `period_structure` index, its `cycle_day`, and `starts_at` / `ends_at` as comparable minutes — because the clash key is an interval, not an id | 0033 §6, 0032 §5 |
| `ActivityFact` | Carries a **resolved** bell-schedule index and a group **audience mask**. No required capacity field in the MVP | 0033 §3, 0035 §5, 0034 §1 |
| `GroupFact` | **Not** an expanded leaf student set. A group index plus the overlap relation | 0035 §5 |
| Overlap relation | A symmetric three-state function, compiled to `overlaps_with` bitsets once per solve | 0035 §3 |
| `RoomFact` | `capacity` is carried (it is `NOT NULL` and free), and is read by no MVP constraint | 0034 §1 |
| `UnavailabilityIndex` | Compiled per granularity onto the slots each rule names; never widened across structures | 0033 §7 |
| Expected attendance | A two-state value — `Known(i64)` or `Unknown` — where `Unknown` is a variant and not a sentinel | 0034 §3 |

**One correction to [`SOLVER.md`](SOLVER.md) §2**, applied in this PR: `groups: Vec<GroupFact>
// expanded to leaf student sets` was incompatible with ADR 0006, because a school with no
`student` rows has empty leaf sets and would get a silent `student-clash`.

---

## 5. Multi-school safeguards

**Accepted future design**, from [ADR 0032](adr/0032-timetable-generation-is-scoped-to-one-school.md).
Four scopes, kept apart:

| Scope | Extent |
| --- | --- |
| **Optimisation** | one school's activities over one `(cycle, term)` grid |
| **Validation** | the same school's activities, plus full occupancy of every `room_id` / `resource_id` its placements touch |
| **Persistence** | one school, one cycle, one term. A timetable row is never project-wide |
| **Warning** | entities in scope that another school also holds |

The rules, compressed:

1. Generate per `(school, cycle, term)`. The solver never holds two schools' activities, so
   it cannot equate their cycle positions — structural, not remembered.
2. Within a school, shared occupancy compares **real time**, which needs no timezone (one
   school, one zone, it cancels) and no calendar date (one cycle, shared `cycle_day` rows).
3. Across schools, **detect and report**, naming the entity, both schools and the
   consequence. Never block, because the remedy does not exist. Never read the other
   school's timetable, whether it is absent, a `DRAFT`, or several.
4. **A timetable is never described as conflict-free.** It is described as having no
   *confirmed* clash within the school solved, with the unverified counts beside it — from
   shared entities (0032), `Indeterminate` group pairs (0035) and unchecked capacities
   (0034).

**Deferred, named:** coordinated cross-school generation, cross-school clash detection,
project-level availability. Each needs D22 Option C's alignment table *and* a timezone
database the workspace does not have.

---

## 6. MVP scope, after these decisions

**Accepted future design.** The audit's §13 MVP, with the three changes M1 makes to it.

**Hard constraints in the first set — eight:** `teacher-clash`, `student-clash`,
`room-clash`, `room-type-required`, `entity-unavailable` (teacher, room, group),
`duration-contiguous`, `no-teaching-in-period`, and `activity-fixed` once locks exist.
**Medium — one:** `activity-scheduled`.

| Change from the audit | Why |
| --- | --- |
| `room-capacity` **removed** from the first set; `CONSTRAINTS.md` moves it `M1 → M2+` | D35. It has no evaluable left-hand side, so it could not pass `CLAUDE.md` §4's unit-test requirement |
| `student-clash` **kept**, and now carries a required migration | D12. A room of unverified size is a caveat; a child in two rooms is not a timetable |
| `room-clash` **unblocked** for the intra-school cross-bell case | ADR 0032 rule 2, now Accepted. The cross-school case stays a warning |

**Not in the MVP, unchanged from the audit:** every soft constraint, local search and
incremental scoring, manual editing and `validate_move`, locks, repair, lines, resource
scheduling, room candidate lists, teacher as a planning variable (D7), cross-school
coordinated generation, export, and the published/archived lifecycle.

### Implementation dependencies

```
M1  decisions  (this PR)              ── no code
 │
 ├─► M2  vocabulary + fixtures         0033 shapes TimeslotFact; 0034 shapes ActivityFact;
 │                                     0035 shapes GroupFact and the overlap relation
 ├─► M3  pre-solve diagnostics  ✅     consumes every "explicit warning" row of §3.2,
 │                                     plus the one shared bell-schedule resolver M4 reuses
 ├─► M4  Problem loader                consumes 0033 §1–§4 and 0035 §2–§5. Blocked without both
 ├─► M5  registry + 8 evaluators       consumes 0034 §1 (one fewer type) and
 │                                     0035 §4 (one extra column in its migration)
 ├─► M6  construction solver
 ├─► M7  persistence
 ├─► M8  orchestration
 └─► M9  review                        renders §5 rule 4's counts
```

M2 is the only unblocked milestone before M1 and was still shaped by it, which is the
audit's §17 argument and the reason this PR exists. It has since been built —
[SCHEDULING-FIXTURES.md](SCHEDULING-FIXTURES.md).

---

## 7. Deferred functionality

Collected so that nothing below is mistaken for an oversight.

| Deferred | Until | Record |
| --- | --- | --- |
| `room-capacity` and any capacity reasoning | `class_section.expected_attendance` exists and a school asks | 0034 §1 |
| Enforcing `Indeterminate` group overlap | Never as a hard rule. A soft "avoid unverified concurrency" is a candidate for M10 | 0035 §6 |
| Cross-school clash detection in real time | D22 Option C **and** a timezone database | 0032 §4 |
| Project-level availability for a shared person | As above | 0032 §4 |
| Individually enrolled students as a scheduling population | M2's sectioning | ADR 0029 §4, 0035 §7 |
| A bell schedule shared by a *subset* of campuses | A user reporting duplication as a problem | 0033, alternatives |
| Two bell schedules on one campus | A school needing it; then `activity`/`class_section` gains the structure reference | 0033, alternatives |
| A sharper per-structure capacity diagnostic in `evara-core` | M3 | 0033, consequences |
| Resource scheduling, lines, per-occurrence overrides, locks | Post-MVP, per the audit's N4–N7 | audit §11 |

---

## 8. Remaining risks

| # | Risk | Mitigation decided here |
| --- | --- | --- |
| **A** | A school ignores the `Indeterminate` warning and ships a timetable with a real student clash in it | The warning carries arithmetic and a remedy count, not an adjective (0035 §6). The remaining exposure is accepted and recorded in that ADR's consequences |
| **B** | A warning the user learns to dismiss is worse than none — the risk ADR 0032 already names for its own | Three warnings now share the pattern (0032 rule 3, 0034 §5, 0035 §6). Their **wording is a deliverable** of M3, not an afterthought of it |
| **C** | A multi-campus school is blocked at its first solve by `NoBellSchedule` and reads it as a bug | The diagnostic names the section, the cause and the remedy, in three distinct forms (0033 §3) |
| **D** | The MVP places a class of 32 in a room for 20 | Known, counted and never claimed otherwise (0034 §5). Not a correctness failure of the kind `room-clash` prevents |
| **E** | `children_are_disjoint` is answered carelessly — a school ticks "disjoint" to clear the warning | Facts beat declarations (0035 §3 rule 3 before rule 6) and contradictions are reported. A careless answer that is contradicted by any member row is caught; one that is not contradicted is not |
| **F** | M5's migration grows beyond review size by carrying 0035's column | One `ADD COLUMN` in a migration already creating two tables. If M5 is split as the audit's §15 suggests, the column goes with the clash half |
| **G** | The audit's carried risks R1 (incremental scoring), R2, R3, R5, R9 | Untouched by M1 and still open in [OPEN-DECISIONS](OPEN-DECISIONS.md) |

---

## 9. Future fixtures and acceptance tests

**Not implemented in M1.** Fixtures are M2's deliverable (audit B12), synthetic names only
(`CLAUDE.md` rule 3).

> **Built in M2.** The fifteen cases below became 24 fixtures in
> `evara_core::scheduling::fixtures`, each carrying the acceptance tests it covers and the
> expected outcome of each. See [SCHEDULING-FIXTURES.md](SCHEDULING-FIXTURES.md) §4 for the
> mapping, and §1 for where the vocabulary §4.2 specifies ended up. No rule that would satisfy
> those expectations is implemented: a fixture holds the inputs and the expected output, and
> M3, M4 and M5 supply the functions in between. Each ADR carries its own acceptance table; the fifteen the brief names
map onto them as follows, and every one has an expected outcome written down so M2 implements
rather than re-decides.

| # | Fixture | Expected outcomes defined in |
| --- | --- | --- |
| 1 | Single-campus school | [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) tests 1–2 — resolves under clause 3 or clause 1, no finding |
| 2 | Two campuses, identical bell schedules | [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) tests 3–4; [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) test 1 |
| 3 | Two campuses, different bell schedules | [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) test 5; [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) tests 2–3 |
| 4 | Ambiguous class-section schedule | [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) tests 6–7, 9 — refused with named candidates |
| 5 | Shared teacher across campuses | [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) test 5; [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) tests 11–12 (availability granularity) |
| 6 | Shared room across campuses | [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) tests 2–4 — one violation across differing `timeslot_id`s |
| 7 | Known class attendance | [0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) tests 1–2, 8–9 |
| 8 | Unknown class attendance | [0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) tests 3–6, 11–12 — `Unknown`, never `0`, never a pass |
| 9 | Parent-child student groups | [0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) truth table 4–6, test 3 |
| 10 | Overlapping group memberships | [0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) truth table 9, 13, 16; tests 4–5, 8 |
| 11 | Disjoint student groups | [0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) truth table 7, 12, 18–19; test 6 |
| 12 | Incomplete group memberships | [0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) truth table 15; tests 2, 7 |
| 13 | Shared teacher across schools | [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) tests 6–7 — warned, never compared |
| 14 | Different academic calendars | [0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) tests 10–11 — independent, and no rule reads a date |
| 15 | Custom rotating cycles | [0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) test 14; [0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) test 16 — resolution and overlap read no weekday |

Backward compatibility has a case in each record —
[0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) test 13,
[0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) test 14,
[0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) test 15 — and all three
assert the same property: a project created before these records opens, reads `NULL` where a
future column will be, needs no re-materialisation and no re-entry, and behaves as the
record defines for unstated data.

---

## 10. Documentation corrections made in this milestone

The audit's §12 listed eight places documentation described a future thing in the present
tense. Four are corrected here because an accepted decision now contradicts them; the rest
are left, with their reason.

| Audit ref | Correction | Where |
| --- | --- | --- |
| D-2 | D22's stale premise — *"`period_structure` is school-wide, and nothing ties a campus to exactly one"* | [OPEN-DECISIONS](OPEN-DECISIONS.md) D22, now closed |
| D-5 | `DATA-MODEL.md` §9 mixed built and planned indexes without marking which | [DATA-MODEL](DATA-MODEL.md) §9 |
| D-6 | `ARCHITECTURE.md` §3 rule 4's present tense — *"loads a `Problem` … persists a `Solution`"* | [ARCHITECTURE](ARCHITECTURE.md) §3 |
| D-7 | `CONSTRAINTS.md` §3's `M1` column read as a claim of implementation; `room-capacity`'s status and `student-clash`'s wording were also wrong after D35 and D12 | [CONSTRAINTS](CONSTRAINTS.md) §3 |
| — | `SOLVER.md` §2's *"expanded to leaf student sets"* contradicts ADR 0006 | [SOLVER](SOLVER.md) §2 |
| D-1 | `CLAUDE.md` §3 describes `timetable_entry`'s unique key in the present tense. **Not corrected** — the reasoning is sound and `CLAUDE.md` is outside this milestone's deliverables. It resolves itself when M7 creates the table | — |
| D-3 | `commands/grid.rs`'s comment is already hedged with "future" | — |
| D-4 | `ROADMAP.md` Phase 4's *"Migration `0004`"* naming artefact. **Not corrected** — `ROADMAP.md` is outside this milestone's deliverables | — |
| D-8 | Accurate as written | — |

One discrepancy this milestone **found** rather than inherited, recorded at
[ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §"Why no
default is obviously right": `GridPreviewSection.tsx:103` defaults the bell schedule to the
first `is_default` row in a name-ordered list, and `is_default` is unique **per campus
bucket** rather than per school. A school holding both a school-wide default and a campus
default has two, and which one is picked depends on their names. It is survivable there,
because materialisation is previewed and confirmed; it is cited as the evidence that D34's
ambiguity is real rather than hypothetical. **No code is changed by this milestone**, and
fixing it is a separate, small PR.
