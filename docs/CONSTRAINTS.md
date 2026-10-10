# Constraint Registry

Constraints are **first-class data**: rows in `constraint_instance` with a `type_key`, a
level, a weight, an enabled flag and JSON parameters. Code provides a registry of
evaluators keyed by `type_key`; it does not provide a list of hardcoded rules.

Every constraint type must have:

1. a stable `type_key` and a registry entry in `crates/evara-constraints/`
2. a JSON Schema for its parameters, which also generates its configuration form
3. documented semantics in this file
4. an isolated unit test, plus a synthetic fixture exercising it
5. a human-readable `explain()` implementation

No constraint is "supported" until all five exist.

---

## 1. Levels

| Level | Meaning | Solver behaviour |
| --- | --- | --- |
| `HARD` | A timetable violating this is not usable. | Must reach 0. Blocks manual moves. |
| `MEDIUM` | Must hold unless the alternative is no timetable at all — unscheduled activities, unmet period counts, unstaffed classes, disruption during repair. | Minimised after feasibility, before preferences. Warns on manual moves. |
| `SOFT` | Preference. Quality, not correctness. | Optimised last. Warns on manual moves. |

Levels are compared lexicographically and are **configurable per instance**: a school can
run "teacher preferred times" at `SOFT` while another runs it at `HARD`. The default level
shipped with each type is listed below, not baked in.

---

## 2. Scope

Every instance is scoped by `constraint_ref` rows rather than by parameters, so scoping
is uniform and referentially intact:

| `role` | Meaning |
| --- | --- |
| `appliesTo` | the entities the rule constrains (teachers, groups, rooms, activities, lines) |
| `subject` | restrict to these subjects |
| `yearLevel` | restrict to these year levels |
| `campus` | restrict to these campuses |
| `timeslot` / `cycleDay` / `period` | restrict to a part of the cycle |

No `appliesTo` rows means project-wide. `term_id` on the instance scopes it in time.

---

## 3. Core registry

Status: `M1` = Milestone 1, `M2+` = later, `R` = schema reserved only.

### Clashes and capacity

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `teacher-clash` | HARD | A teacher may not be in two activities in one timeslot. `CORE-001` | M1 |
| `student-clash` | HARD | A student group (resolved to leaf student sets) may not attend two activities in one timeslot. `CORE-002` | M1 |
| `room-clash` | HARD | A room may not host two activities in one timeslot. `CORE-003` | M1 |
| `resource-clash` | HARD | Demand for a shared resource may not exceed its quantity in a timeslot. | M1 |
| `room-capacity` | HARD | Room capacity ≥ activity's required capacity. `CORE-005` | M1 |
| `room-type-required` | HARD | Assigned room must match the required room type. `CORE-006` | M1 |
| `room-candidate-only` | HARD | If `activity_room_candidate` rows exist, the room must be one of them. | M1 |

### Availability and time windows

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `entity-unavailable` | HARD | Honours `availability.status = UNAVAILABLE` for teachers, rooms, groups, resources, campuses. `CORE-004` | M1 |
| `preferred-time` | SOFT | Reward/penalise against `PREFERRED` / `DISCOURAGED` availability. `CORE-012` | M1 |
| `subject-time-window` | SOFT | A subject prefers (or must take) certain periods — e.g. PE not in period 1, Maths in the morning. | M1 |
| `activity-fixed` | HARD | A locked activity keeps its timeslot and room. `CORE-007` | M1 |
| `no-teaching-in-period` | HARD | A period kind (`BREAK`, `REGISTRATION`) holds no lessons. | M1 |

### Frequency, duration, distribution

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `activity-scheduled` | MEDIUM | Every activity in scope must be placed somewhere. | M1 |
| `periods-per-cycle` | MEDIUM | A course's scheduled periods per cycle must equal its target. `CORE-008` | M1 |
| `duration-contiguous` | HARD | A multi-period activity occupies consecutive periods on one day, not spanning a break unless permitted. | M1 |
| `spread-across-days` | SOFT | Repeated lessons of one course for one group should fall on different days. `CORE-009` | M1 |
| `max-per-day` | SOFT | At most N periods of a subject for a group per day (`params: { max }`). | M1 |
| `different-day` | HARD/SOFT | Named activities must not share a day. | M2+ |
| `same-day` | SOFT | Named activities should share a day. | M2+ |
| `min-days-between` | SOFT | At least N cycle days between two activities. | M2+ |

### Relationships between activities

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `same-period` | HARD | Link members run in the same timeslot. Backs lines/blocks and electives. | M1 (lines) |
| `different-period` | HARD | Link members must not share a timeslot. | M2+ |
| `consecutive` | HARD/SOFT | Link members run back-to-back in order. | M2+ |
| `before` / `after` | HARD/SOFT | Ordering within a day or cycle, optional `gap_periods`. | M2+ |
| `same-room` / `different-room` | SOFT | Room relationship between linked activities. | M2+ |

### Teacher workload and comfort

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `teacher-max-per-day` | HARD | `params: { max }`, or the teacher's own column. | M1 |
| `teacher-max-per-cycle` | HARD | Total teaching load ceiling. | M1 |
| `teacher-max-consecutive` | SOFT | No more than N teaching periods in a row. | M1 |
| `teacher-min-gaps` | SOFT | Minimise idle periods between a teacher's first and last lesson of a day. `CORE-010` | M1 |
| `teacher-free-day` | SOFT | A teacher should have a whole cycle day free. | M2+ |
| `teacher-room-stability` | SOFT | Minimise a teacher's room changes. `CORE-011` | M1 |
| `teacher-qualified` | MEDIUM | The assigned teacher is qualified for the subject. Relevant once teacher is a planning variable. | M2+ |
| `teacher-balanced-load` | SOFT | Even distribution of load across a department. | M2+ |

### Students and requests

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `group-min-gaps` | SOFT | Minimise idle periods in a group's day. | M1 |
| `group-day-bounds` | SOFT | A group's lessons start/end within a window. | M2+ |
| `request-satisfied` | MEDIUM | A student's `course_request` is met by an enrolment. | M2+ |
| `request-weighted` | SOFT | Higher-priority / higher-weight requests count for more. | M2+ |
| `request-alternate` | SOFT | Falling back to an `ALTERNATE` request is allowed but penalised. | M2+ |
| `prerequisite` | HARD | A course requires another in an earlier term. | M2+ |
| `corequisite` | HARD | Two courses must be taken in the same term. | M2+ |
| `section-balance` | SOFT | Even enrolment across the sections of a course. | M2+ |
| `section-max-size` | HARD | Enrolment ≤ `class_section.max_size`. | M2+ |

### Multi-campus and non-lesson duties

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `campus-travel-time` | HARD | Consecutive activities for one person on different campuses need ≥ `campus_travel.minutes` between them. | M2+ |
| `campus-day-lock` | SOFT | A person stays on one campus for a whole day. | M2+ |
| `duty-roster-coverage` | MEDIUM | Each yard-duty slot is staffed. | M2+ |
| `duty-fair-share` | SOFT | Duties distributed evenly. | M2+ |
| `meeting-attendance` | HARD | All required attendees of a meeting are free. | M2+ |
| `assembly-attendance` | HARD | All required groups are free for an assembly. | M2+ |
| `homeroom-daily` | MEDIUM | Each group has homeroom every school day. | M2+ |

### Operations

| Key | Default | Semantics | Status |
| --- | --- | --- | --- |
| `minimise-disruption` | MEDIUM | Penalise placements that differ from a baseline timetable. Drives repair. (`OPS-001`) | M1 |
| `substitute-qualified` | SOFT | Prefer a qualified, free, under-loaded substitute. | R |
| `substitute-fair-share` | SOFT | Spread cover duty. | R |
| `absence-respected` | HARD | No teacher is scheduled during a recorded absence (dated layer). | R |

---

## 4. Vendor mapping

Competitor features are mapped to the generic types above and are **not** implemented as
vendor concepts. Recorded here so research stays traceable.

| Vendor concept | Evara representation |
| --- | --- |
| PowerScheduler course requests / "tally" | `course_request` + `request-satisfied`, `request-weighted`, `request-alternate` |
| PowerScheduler / Edval **lines**, "blocks" | `line` + `same-period` |
| aSc "cards", "divisions", "seminars" | `activity` with `duration_periods`; group hierarchy; `line` |
| aSc relations ("not after", "same day") | `activity_link` kinds |
| TTS "duty rosters", "year-level patterns" | `duty-roster-coverage`, `duty-fair-share`; `line` scoped by year level |
| Edval "clash tables", "composites" | derived from `student_group` hierarchy and `student-clash` |
| TimeTabler "schedule structure", "doubles" | `period_structure`; `duration_periods = 2` + `duration-contiguous` |
| Veracross "blocks / rotations" | `cycle` + `cycle_day` + `calendar_day` |
| Any vendor's "fixed lesson" | `activity.is_locked` + `activity-fixed` |

Where a vendor's concept cannot be expressed, the fix is a **new generic constraint type**
with a vendor-neutral name — never a vendor branch in the core.

---

## 5. Legacy identifiers

The prototype's Java `TimetableConstraintProvider` and the previous version of this file
used `CORE-0xx` identifiers. They are retained as cross-references in the tables above so
the original specification is traceable, but `type_key` is the identity going forward.

`CORE-001` → `teacher-clash` · `CORE-002` → `student-clash` · `CORE-003` → `room-clash` ·
`CORE-004` → `entity-unavailable` · `CORE-005` → `room-capacity` ·
`CORE-006` → `room-type-required` · `CORE-007` → `activity-fixed` ·
`CORE-008` → `periods-per-cycle` · `CORE-009` → `spread-across-days` ·
`CORE-010` → `teacher-min-gaps` · `CORE-011` → `teacher-room-stability` ·
`CORE-012` → `preferred-time`
