# Scheduling Readiness Audit

- Date: 2026-10-16
- Audited commit: `8d70096` (`origin/develop`, "Merge pull request #23 from
  Rahul9046/feat/excel-roster-imports")
- Schema version: `V15`
- Verification run: `npm run check` — **exit 0**. 780 Rust tests (2 CLI, 83 `evara-core`,
  639 `evara-db`, 17 desktop command layer, 38 `evara-import`, 1 doc-test) and 1042
  TypeScript tests across 44 files. `cargo fmt`, `cargo clippy`, `tsc --noEmit` and the
  generated-type check are all clean.

This document is an audit. It implements nothing, adds no migration and changes no
behaviour.

---

## 1. Executive summary

**Evara can describe a school completely and cannot schedule one at all.**

Everything a timetable is built *from* exists, is enforced in SQL, and is tested: fifteen
migrations, thirty-six tracked entity tables, 200 Tauri commands, eighteen School Setup
sections. Stage D's import engine fills them from a spreadsheet in one transaction. That
work is real and the test suite proves it.

Everything a timetable is built *by* is absent. Not thin — absent:

| Crate | Lines | Tests | Contents |
| --- | --- | --- | --- |
| `evara-solver` | 20 | 0 | A module doc-comment describing what Phase 6 will add |
| `evara-constraints` | 23 | 0 | A module doc-comment describing what Phase 4 and 5 will add |
| `evara-report` | 13 | 0 | A module doc-comment describing what Phase 7 and 10 will add |
| `evara-project` | 17 | 0 | A module doc-comment describing what Phase 9 will add |

`crates/evara-solver/src/lib.rs` and `crates/evara-constraints/src/lib.rs` contain no
`struct`, no `trait`, no `fn`. Both crates are already declared as dependencies of
`apps/desktop/src-tauri/Cargo.toml:66-67` and `crates/evara-cli/Cargo.toml:21-22`, so the
wiring is in place and there is nothing on the other end of it.

There is likewise no `timetable` table, no `timetable_entry` table, no `constraint_instance`
table, no `solve_run` table and no `violation` table. `change_log::TRACKED_TABLES`
(`crates/evara-db/src/change_log.rs:145-194`) lists thirty-six tables and none of them is a
scheduling output. `apps/desktop/src/features/` and `apps/desktop/src/ui/` are empty
directories.

**Readiness: 0% of generation, ~85% of the inputs generation needs.** The honest statement
is that Milestone 1 Phases 4 through 10 have not started, Phase 3 is complete except for its
fixtures, and the three decisions that must precede solver code (D12, D22, and a new one this
audit names) are open.

The single most valuable thing the repository has for this next step is not code: it is
`docs/SOLVER.md`, which already specifies the engine boundary, the score, the registry trait,
the algorithm and the three layers of infeasibility explanation in enough detail that the
implementation is a transcription rather than a design exercise. The gap is execution, not
direction.

### The three things that must be decided before the first line of solver code

1. **Which bell schedule does an activity schedule against?** (§4.1, new decision D34.) No
   stored fact answers this. Without it the loader cannot choose a value range.
2. **What does a room-capacity requirement compare against?** (§4.4, new decision D35.)
   There is no `class_section.max_size` and `student_group.size` is nullable, so
   `room-capacity` currently has no left-hand side.
3. **How do overlapping student groups clash?** (D12, open since Phase 0.) The
   `parent_group_id` hierarchy is stored and nothing reads it.

D22 (cross-campus time comparison for a shared room) is open too, but §9 shows it is
answerable today for the intra-school case from data the schema already stores, and
deferrable for the cross-school case.

---

## 2. Current system capabilities

### 2.1 What is built, by layer

**Database — `crates/evara-db`, 13,362 lines of repository code, 639 tests.**
Fifteen forward-only migrations (`V1`…`V15`). Thirty-six entity tables, every one with
`created_at`, `updated_at`, `rev` and change-log triggers, verified by a schema-level test
that discovers entity tables and fails on a missing trigger. Eight repositories:
`structure`, `time_model`, `people`, `membership`, `curriculum`, `lessons`, `availability`,
and the `import` engine. The revision precondition is a `WHERE` clause in `repo::update` /
`repo::delete` (ADR 0027), not a read-compare-write.

**Domain — `crates/evara-core`, 3,169 lines, 83 tests.** Five modules: `time`
(`CycleCoverage`, `CycleRotation`), `calendar` (`WorkingDays`, `WeeklyPosition`), `branding`,
`import` (the `SourceTable`/plan vocabulary four crates share), and `lessons`
(`LessonDemand`, `CycleCapacity`, `DemandFinding`). No `Score`, no `Problem`, no
`Placement`, no `Assignment`, no identifier newtypes — the types `docs/SOLVER.md` §2 declares
load-bearing do not exist yet.

**Import — `crates/evara-import` 1,871 lines + `crates/evara-db/src/import` ~3,750 lines,
38 + (within the 639) tests.** Ten record kinds from `.xlsx` and `.csv`, previewed as a plan
and applied in one transaction (ADR 0030, ADR 0031).

**Command layer — `apps/desktop/src-tauri`, 4,577 lines, 17 tests.** 200 commands in the
`generate_handler!` manifest (`apps/desktop/src-tauri/src/lib.rs:47-258`). Twelve modules,
all orchestration.

**Interface — `apps/desktop/src`, 44 test files, 1042 tests.** A project browser and an
eighteen-section School Setup (`apps/desktop/src/app/setup/SchoolSetup.tsx:166-301`). No
other screen exists.

### 2.2 What is not built

- Any scheduling rule, anywhere, in any language.
- Any representation of a *placement*: no table, no type, no command, no component.
- `evara-cli` is help text. `crates/evara-cli/src/main.rs:13-22` lists eight planned
  subcommands and implements `--version`.
- Fixtures. `fixtures/basic-school/problem.json` is the pre-Tauri prototype's flat model
  (`timeslots`, `teachers`, `studentGroups`, `rooms`, `lessons`); `fixtures/README.md`
  states nothing reads it. The Phase 3 generator and the tiny/medium/infeasible fixtures
  never landed, so `docs/ROADMAP.md` Phase 3's *"Closes M1-7 once the fixtures land"* has
  not fired.

---

## 3. End-to-end scheduling workflow status

Classification: **Tested** (implemented with tests) · **Incomplete** (works, with a stated
hole) · **Partial** · **Model only** (schema exists, nothing reads it) · **Stub** ·
**Absent**.

| # | Capability | State | Evidence |
| --- | --- | --- | --- |
| 1 | Project with multiple schools | **Tested** | `V12`, ADR 0019; `repo/structure.rs:create_school`; `commands/project.rs`, `commands/structure.rs:174-179`; `src/app/setup/Schools.tsx`; `tests_multi_school.rs` (2,050 lines) |
| 2 | Schools, campuses, buildings, rooms | **Tested** | `V3:38,53,63,74`, `V5`, `V6`; `repo/structure.rs` (2,807 lines); `CampusesAndBuildings.tsx`, `RoomsAndRoomTypes.tsx`; `tests_structure.rs`, `tests_shared_buildings.rs`, `tests_shared_rooms.rs` |
| 3 | Academic years and terms | **Tested** | `V3:130,145`; `AcademicYearsAndTerms.tsx` (+ 495-line test) |
| 4 | Weekly and custom rotating cycles | **Tested** | `V4:42,60` + `V9` (`cycle.kind`, immutability trigger, 7-day weekly invariant); ADR 0017; `CyclesAndCycleDays.tsx`; `tests_cycle_kinds.rs`, `tests_cycle_day_limit.rs` |
| 5 | Bell schedules and periods | **Tested** | `V4:79,97` (`period_structure`, `period` with `starts_at`/`ends_at`/`kind`/`counts_as_load`); `PeriodStructuresAndPeriods.tsx`; `tests_time_model.rs` |
| 6 | Map cycle positions to calendar dates | **Incomplete** | `V4:191` `calendar_day`; `CalendarMapping.tsx` (383 lines) writes one date at a time. `V4:186-190` states the omission outright: *"Phase 1D stores and reads these rows; it does not generate them. What Day follows a holiday … is policy DATA-MODEL.md does not state."* A 190-day year is 190 manual rows. **No scheduling consequence** — nothing in scheduling may read a weekday and the timetable is defined over the cycle — but it is a real usability hole, and Layer-1 diagnostics that count *dated* instruction days cannot run without it |
| 7 | Teachers and campus eligibility | **Tested** | `V10:51` `teacher`; `V13:95` `teacher_school`, `V13:153` `teacher_campus`; ADR 0021; `Teachers.tsx`, `TeacherSchools.tsx`; `tests_membership.rs` (2,452 lines) |
| 8 | Teacher availability | **Incomplete** | `V11:42` models six entity kinds, three granularities, `UNAVAILABLE`/`PREFERRED`/`DISCOURAGED` and an integer weight. `AvailabilityGrid.tsx` is wired into `Teachers.tsx`, `RoomsAndRoomTypes.tsx`, `StudentGroups.tsx` only — `RESOURCE`, `CAMPUS` and `SUBJECT` are **model only**. `effective_from`/`effective_to` are carried and collected by nothing (`V11:62-70`). `availability.school_id` is `NOT NULL`, so a teacher shared by two schools has two unavailability sets and nothing reconciles them (§9) |
| 9 | Student groups and membership | **Incomplete** | `V10:117`; `parent_group_id` hierarchy stored with cycle detection; `StudentGroups.tsx`, `Students.tsx`; `tests_people.rs`. **No read expands a group to its descendants or leaves** — `repo/people.rs` offers `student_group_members`, `groups_for_student` and a member count, nothing transitive. D12 is open |
| 10 | Subjects, courses, class sections | **Tested** | `V14:106,148` `department`, `subject`; `V15:171,408` `course`, `class_section`; ADR 0025, ADR 0029; `Subjects.tsx`, `Departments.tsx`, `ClassSections.tsx`, `Lessons.tsx`; `tests_curriculum.rs` (1,803), `tests_lessons.rs` (3,053) |
| 11 | Assign teachers to class sections | **Tested** | `V15:507` `class_teacher` with the membership and campus-eligibility trigger pairs; `repo/lessons.rs:1121 assignable_teachers`, `:1217 assign_class_teacher`; ADR 0028 (a missing qualification warns, employment refuses) |
| 12 | Associate student groups with classes | **Tested** | `V15:662` `class_group`; `repo/lessons.rs:1362-1533`; `ClassSectionDetail.tsx` |
| 13 | Define individual scheduling activities | **Tested** | `V15:825` `activity` — one row per occurrence, `duration_periods`, `split_index` never renumbered; `repo/lessons.rs:1762 add_activities`, `:1829 generate_activities`; ADR 0029 |
| 14 | Room requirements and resources | **Partial** | `course.required_room_type_id` (`V15:223`) with a same-school trigger pair is the **only** scheduling requirement that exists. No required capacity, no `activity_room_candidate`, no `activity_resource`, no per-section override. `resource` (`V3:115`, `quantity`) and `resource_campus` (`V13:516`) exist with **no link to anything schedulable** — `ResourcesAndSharing.tsx:14` says so: *"Scheduling against resources is still Phase 4's"* |
| 15 | Define scheduling constraints | **Absent** | No `constraint_instance`, no `constraint_ref` in any migration. `evara-constraints` is 23 lines of doc-comment. `docs/CONSTRAINTS.md` is a specification of 60+ types, 0 implemented |
| 16 | Validate scheduling inputs | **Partial** | `evara_core::lessons::DemandFinding` (`crates/evara-core/src/lessons.rs:244-300`) reports exactly two impossibilities, **per class section**: an occurrence longer than any unbroken teaching run, and a total beyond the cycle's optimistic slot count. Reached by `repo/lessons.rs:1720 section_demand` → `setup_section_demand`. **None of `docs/SOLVER.md` §8 Layer 1's six project-level checks exist**: per-teacher load vs. slots, room-type demand vs. supply, group period budget, empty candidate sets, line/room arithmetic, mutually conflicting pins |
| 17 | Generate a timetable | **Absent** | `evara-solver` has no code |
| 18 | Review scheduling conflicts | **Absent** | No violation type, table, command or component |
| 19 | Manually adjust placements | **Absent** | No `validate_move`, no `apply_move`, no grid interaction |
| 20 | Save and reopen timetables | **Absent** | No `timetable` / `timetable_entry` / `solve_run` table |
| 21 | View timetables by teacher/student/class/room | **Absent** | `evara-report` has no code; `apps/desktop/src/features/` is empty. The Timetable Grid Preview section (`GridPreviewSection.tsx`) draws the **time grid** — cycle days × periods, which cells are materialised — and holds no placements |
| 22 | Export or print | **Absent** | `evara-report` has no code |

**Score: 11 tested, 3 incomplete, 2 partial, 6 absent.** Items 15 and 17–22 — every one of
them a generation capability — are absent.

---

## 4. Data model assessment

The schema is the strongest part of the repository. This section records what it can
express, and the five places where an input the solver needs is missing or ambiguous.

### 4.1 Academic structure

| Concern | State | Note |
| --- | --- | --- |
| Academic years, terms | Complete | `V3:130,145` |
| Cycle definition, weekly and custom | Complete | `V4:42`, `V9`. `cycle.kind` immutable after creation; a `WEEKLY` cycle always holds seven days so one definition is shareable, and `WeeklyPosition::applies_to` answers "which of those does this school teach on?" without storing it (ADR 0017) |
| Rotation start day | **Absent by design** | ADR 0017: a cycle "carries no start date, no starting rotation position". The projection onto dates is `calendar_day`, explicit per date |
| Holiday behaviour | **Model only** | `calendar_day.kind IN ('SCHOOL','HOLIDAY','EXAM','EVENT','PD')`, `cycle_day_id` nullable for "no lessons". What a closure does to a rotation is undecided and unwritten (`V4:186-190`) |
| Calendar mapping | Incomplete | §3 item 6 |
| Bell schedules | Complete | `period_structure` is **campus-scoped** — `campus_id` nullable, `V4:82` — with at most one default per campus |
| Period definitions | Complete | `starts_at`/`ends_at` are `'HH:MM'` wall clock, `CHECK`-validated; `kind`; `counts_as_load`. `school.timezone` is `NOT NULL` (`V12`) |
| Campus-specific schedules | Supported | `period_structure.campus_id`, and `timeslot.campus_id` is derived from it under a trigger pair (`V4:150-185`) |
| Timeslot | Complete | `V4:124`. Identity is `(cycle_day_id, period_id, COALESCE(term_id,''))` enforced by `idx_timeslot_identity`; `id` assigned once; materialisation never deletes (ADR 0011) |

**The model can represent a real school without ambiguous scheduling semantics — with one
exception, which is a gap rather than an ambiguity.**

> **New decision D34 — which bell schedule does an activity schedule against?**
>
> The grid is materialised per `(cycle, period_structure, term)`:
> `MaterialisationRequest` (`crates/evara-db/src/repo/time_model.rs:1651-1662`) names all
> three. A school with two campuses on two bell schedules therefore has **two disjoint sets
> of `timeslot` rows over the same cycle days**.
>
> An `activity` reaches a campus only through `class_section.campus_id`, which is
> **nullable** (`V15:423`) and means "not campus-constrained". A `period_structure`'s
> `campus_id` is nullable too and means "school-wide". So for an activity whose section
> names no campus, or a school holding both a school-wide and a campus bell schedule, no
> stored fact says which value range the solver should draw from.
>
> This is not an edge case: it is the first question the loader must answer, for every
> activity, in every school. It must be settled before `Problem` assembly is written.

### 4.2 Teachers

| Concern | State | Evidence |
| --- | --- | --- |
| Active school membership | Complete | `teacher_school.is_active` (`V13:95`); effective eligibility is the conjunction with `teacher.is_active`, computed in one place (ADR 0021) |
| Multi-campus eligibility | Complete | `teacher_campus` (`V13:153`), held to membership by a trigger pair |
| Availability | Incomplete | §3 item 8 |
| Existing class assignments | Complete | `class_teacher` (`V15:507`); `repo/lessons.rs:1324 sections_of_teacher` |
| Qualification warning | Complete | ADR 0028; `assignable_teachers` returns `AssignmentEligibility` |
| Workload ceilings | **Model only** | `teacher.fte_percent`, `max_periods_per_day`, `max_periods_per_cycle`, `max_consecutive_periods` (`V10:62-70`), all integer, all carried, all read by nothing. `V10:63` says so: *"Carried now, read in Phase 4"* |
| Double-booking prevention | **Absent** | No constraint exists |

**Teachers shared between schools.** `teacher_school` makes one person employable by several
schools. Nothing detects or prevents a cross-school clash, and the model as it stands cannot:

- Each school owns its own `cycle` (`cycle.school_id`), so **two schools never share a
  `timeslot` row**. Comparing `timeslot_id` across schools is meaningless, not merely
  imprecise.
- `school.timezone` is per school, so comparing two schools' `period.starts_at` as wall
  clock is wrong before any period structure is considered — exactly what D22's *Widened by
  `V13`* paragraph records.
- No project-level calendar aligns two schools' cycle days to the same dates. Each school's
  `calendar_day` rows are its own.
- `availability.school_id` is `NOT NULL`, so a shared teacher's Thursday unavailability
  entered at School A is invisible at School B.

The correct reading: **matching cycle positions across schools imply nothing about matching
real-world time, and the repository contains no mechanism that could make them imply it.**
§9 recommends what to do.

### 4.3 Students and groups

| Concern | State | Evidence |
| --- | --- | --- |
| Student groups, kinds, hierarchy | Complete | `V10:117`; five kinds; `parent_group_id` `RESTRICT` |
| Group size | Complete | `student_group.size` nullable — "membership is the truth" vs. "there are this many" (ADR 0006) |
| Individual enrolment | Complete, **no scheduling meaning** | `class_enrolment` (`V15:735`); ADR 0029 §4 and D12 both state it is a roster overlay in M1 |
| Group overlap resolution | **Absent** | The hierarchy is stored; no read walks it |
| Student double-booking | **Absent** | D12 open since Phase 0 |
| Cross-school enrolment | Modelled | `student_school_enrolment` permits concurrent spells at two schools (`V13:311`) |
| Year level | Complete | `year_level`, referenced from `student`, `student_group`, `course`, `student_school_enrolment` |
| Student timetable derivation | **Absent** | Needs §7's tables first |

The group-first model (ADR 0006) is intact: a full timetable must be buildable from
`student_group.size` with no student rows at all, and `V15` did not weaken it.

### 4.4 Rooms and resources

| Concern | State | Evidence |
| --- | --- | --- |
| Room identity | Complete | `room` (`V3:74`); `room.campus_id` is the **home** campus, `room_campus` (`V6`) is the complete association set — one room, one `room.id`, however many campuses (ADR 0014) |
| Campus and building ownership | Complete | `building_campus` (`V5`), `room_campus` (`V6`), both reaching across schools |
| Room types | Complete | `room_type` is school-owned; `room.room_type_id` `SET NULL` |
| Room capacity | Column exists | `room.capacity NOT NULL` |
| Shared rooms | Complete, **unprotected** | ADR 0014 binds the future solver to a per-`room_id`, project-wide clash rule. Nothing implements it; D22 blocks the cross-bell-schedule case |
| Resource requirements | **Absent** | `resource.quantity` exists; no table links a resource to an activity |
| Room availability | Modelled + UI | `availability` kind `ROOM`, editable |
| Room double-booking | **Absent** | No constraint |
| Campus compatibility | Partial | Enforced for *staffing* (`class_teacher` trigger pair) and for *ownership* (`course.required_room_type_id` same-school trigger). Nothing yet constrains which room an activity may use |

> **New decision D35 — what does `room-capacity` compare against?**
>
> `docs/CONSTRAINTS.md` defines `room-capacity` as *"Room capacity ≥ activity's required
> capacity"*. The activity has no required capacity and nothing derives one:
>
> - `class_section.max_size` appears in `docs/DATA-MODEL.md` §5's sketch and `V15:102`
>   records that it was **deliberately not built**.
> - `student_group.size` is nullable, so summing a section's `class_group` rows yields
>   `NULL` for exactly the schools ADR 0006 says must work.
> - `class_enrolment` has no scheduling meaning in M1 (ADR 0029 §4).
>
> So `room-capacity` has no left-hand side today. Either it is excluded from the MVP
> constraint set, or one column is added. Deciding this before the registry is written
> avoids a constraint type that cannot be evaluated.

### 4.5 Lessons and activities

Every item the brief lists is present and correct, except where noted:

| Concern | State |
| --- | --- |
| Course ownership (school resolved through `subject`) | Complete, `V15:177` |
| Class sections, many per course | Complete, `V15:408` |
| Assigned teachers, many-to-many | Complete, `V15:507` |
| Associated student groups | Complete, `V15:662` |
| Activity duration | Complete, `activity.duration_periods` |
| Multiple occurrences per cycle | Complete — **one row per occurrence**, `V15:825`, ADR 0029 |
| Split indexes | Complete, assigned once from `MAX+1`, never renumbered |
| Stable activity IDs | Complete, UUIDv7 |
| Demand validation | Partial — §3 item 16 |
| Room requirements | Partial — `course.required_room_type_id` only |
| Resource requirements | **Absent** |
| Activity lock (`is_locked`) | **Absent** — `docs/ROADMAP.md` Phase 3 defers it to Phase 8 deliberately, "a lock is about a *placed* timetable" |

The design decision this audit most wants to preserve is `V15`'s: **one `activity` row is one
placeable occurrence.** It is what makes "spread these four across four days" addressable and
it is what `timetable_entry`'s intended `UNIQUE(timetable_id, activity_id, timeslot_id)` was
designed around.

---

## 5. Constraint coverage matrix

`docs/CONSTRAINTS.md` §3 specifies roughly 22 types at `M1` status and 40 more beyond it.
**Implemented: 0. Enforced: 0. Tested: 0. Configurable: 0.**

For each M1 type: *Domain* = can the inputs be expressed; *Validate* = is anything checked
today; *Solver* = enforced during generation; *UI* = configurable; *Tests*.

### 5.1 Hard constraints the brief asks about

| Constraint | Domain | Validate | Solver | UI | Tests | Missing work |
| --- | --- | --- | --- | --- | --- | --- |
| `teacher-clash` | ✅ `class_teacher` | ❌ | ❌ | ❌ | ❌ | Evaluator + `(teacher, timeslot)` occupancy index. **No blocker** |
| `student-clash` | ⚠️ `class_group` + `parent_group_id`, no expansion read | ❌ | ❌ | ❌ | ❌ | **Blocked on D12.** Needs a group-overlap design note and a dedicated fixture |
| `room-clash` | ⚠️ one `room.id`, shared across campuses and schools | ❌ | ❌ | ❌ | ❌ | **Blocked on D22** for cross-bell-schedule rooms. §9 shows the intra-school case is answerable now |
| `room-capacity` | ❌ no required capacity | ❌ | ❌ | ❌ | ❌ | **Blocked on new D35** |
| `room-type-required` | ✅ `course.required_room_type_id`, `room.room_type_id` | ❌ | ❌ | ❌ | ❌ | Evaluator only. **No blocker** |
| Teacher eligible for the school | ✅ enforced *at assignment* by a trigger pair | ✅ on write | ❌ | n/a | ✅ `tests_lessons.rs` | Already a database invariant; needs **no** constraint type, because the solver does not choose teachers in M1 (D7) |
| Teacher eligible for the campus | ✅ same | ✅ on write | ❌ | n/a | ✅ | As above |
| `duration-contiguous` | ✅ `duration_periods`, `period.kind`, `period.ordinal` | ⚠️ `DemandFinding::OccurrenceLongerThanAnyRun` catches the impossible case | ❌ | ❌ | partial | Evaluator + contiguous-run index. **No blocker** |
| `entity-unavailable` (teacher) | ✅ `availability` | ❌ | ❌ | ✅ entry UI | ❌ | Evaluator + bitset compilation. Note three granularities and `term_id` must all compile down |
| `entity-unavailable` (room) | ✅ | ❌ | ❌ | ✅ | ❌ | As above |
| Valid cycle positions | ✅ `timeslot` is the value range; `is_teaching` derived | ❌ | ❌ | n/a | ❌ | Becomes `no-teaching-in-period` plus the loader's value range. **Blocked on new D34** |
| Shared teacher / room across schools | ⚠️ representable, not comparable | ❌ | ❌ | ❌ | ❌ | **Blocked on D22 (widened).** §9 recommends detect-and-report, not solve |

### 5.2 Soft constraints the brief asks about

All six have **no domain reader, no evaluator, no UI and no test**. Inputs exist for four
of them:

| Constraint | Inputs present today | Verdict for the first solver |
| --- | --- | --- |
| `teacher-min-gaps` (avoid gaps) | `timeslot.ordinal`, `cycle_day.ordinal` | Defer |
| `teacher-max-consecutive` | `teacher.max_consecutive_periods` (unread) | Defer |
| `spread-across-days` | One `activity` row per occurrence — exactly what ADR 0029 built it for | Defer, but it is the **first** soft constraint worth adding: without it a four-period course lands four times on Monday |
| `preferred-time` | `availability.status`, `availability.weight` | Defer |
| `teacher-balanced-load` | `teacher.fte_percent` | Defer |
| `campus-travel-time` | **No `campus_travel` table exists** | Defer; needs a table |
| Minimise early/late periods | `period.ordinal`, `starts_at` | Defer |

**None of the seven is required for a first working solver.** A timetable with zero hard
violations is usable and verifiable; a timetable optimised against preferences that has not
yet been proven feasible is neither.

### 5.3 Required correctness vs. optimisation

**Required for a correct timetable (the MVP hard set):** `teacher-clash`, `student-clash`,
`room-clash`, `room-type-required`, `entity-unavailable`, `duration-contiguous`,
`no-teaching-in-period`, and `activity-fixed` once locks exist. Plus one `MEDIUM`:
`activity-scheduled`, which is what lets the solver return a usable partial timetable and
name what it could not place rather than returning nothing.

**Optimisation, all deferrable:** every `SOFT` type in `docs/CONSTRAINTS.md`.

**Already enforced elsewhere and therefore not constraint types in M1:** teacher school
membership and campus eligibility, because D7 keeps teachers fixed on activities — the
solver never chooses one, so it cannot choose an ineligible one. This is worth stating
because `docs/CONSTRAINTS.md` lists `teacher-qualified` at M2+ for exactly this reason and
the brief's list implies the other two should be solver constraints. They should not be; the
trigger pairs at `V15:507` already make them unsatisfiable-by-construction.

---

## 6. Solver assessment

**`crates/evara-solver/src/lib.rs` is 20 lines, all of them a doc-comment.** The audit
checklist, answered:

| Asked | Present |
| --- | --- |
| A real scheduling algorithm | ❌ |
| Candidate placement generation | ❌ |
| Hard-constraint checking | ❌ |
| Backtracking or search | ❌ |
| Constraint propagation | ❌ |
| Heuristics | ❌ |
| Soft-constraint scoring | ❌ |
| Deterministic seeds | ❌ |
| Time limits | ❌ |
| Cancellation | ❌ |
| Progress reporting | ❌ |
| Partial-solution handling | ❌ |
| Unschedulable diagnostics | ⚠️ Two per-section impossibility checks in `evara-core`, no solver involvement |
| Repeatable test fixtures | ❌ — the only fixture is a stale prototype file |

**Does it consume real data or synthetic fixtures?** Neither. There is no input type. The
`Problem`, `Assignment`, `Placement`, `Score` and `SolverEngine` declarations in
`docs/SOLVER.md` §2–§4 exist only in that document.

**The path, end to end:**

```
SQLite            ✅ complete, 36 tables, 639 tests
  ↓
Domain models     ⚠️ entity records exist; no Problem, Assignment, Placement, Score
  ↓
Solver input      ❌ no loader. Also blocked on D34 (which bell schedule?)
  ↓
Solver            ❌ no code
  ↓
Result            ❌ no type
  ↓
Persistence       ❌ no table
  ↓
UI                ❌ no screen
```

Five of the seven stages are missing entirely. **The solver is not operational and is not
partially operational; it has not been started.**

One thing is genuinely in hand, and it is the part teams usually get wrong: the *boundary* is
already specified and already honoured. `evara-solver` declares — and currently satisfies
trivially — that it opens no database and does no I/O. `evara-constraints` declares that one
evaluator serves generation, scoring, manual-move validation and explanation. Those are the
commitments that are expensive to retrofit, and the repository has not violated them because
it has not yet tested them.

---

## 7. Persistence assessment

| Capability | Supported today | Needed for a first usable timetable |
| --- | --- | --- |
| Timetable versions | ❌ | **Yes** — minimally one `timetable` row per generated result |
| Draft timetables | ❌ | Yes, as `status = 'DRAFT'` |
| Generated runs | ❌ | Yes — `solve_run` carries the seed, which is what makes determinism checkable |
| Activity placements | ❌ | **Yes** — `timetable_entry` is the whole output |
| Manual placement changes | ❌ | No — defer with manual editing |
| Locked placements | ❌ | No — defer |
| Conflict annotations | ❌ | No — recompute from the registry; a `violation` table is a cache |
| Published versions | ❌ | No |
| Historical versions | ❌ | No |
| Optimistic concurrency | ✅ *mechanism* exists | Yes — `repo::update`/`repo::delete` already own it; new tables inherit it for free |
| Change-log integration | ✅ *mechanism* exists | Yes — add the new tables to `TRACKED_TABLES`; a schema test fails if triggers are missing |

**Minimum migration (one, `V16`):** `timetable`, `timetable_entry`, `solve_run`. That is
three tables, the three indexes `docs/DATA-MODEL.md` §9 already names
(`idx_entry_timetable_slot`, `idx_entry_timetable_act`, `idx_entry_room`), and three
`TRACKED_TABLES` entries.

**Deferrable:** `timetable_entry_teacher` (D7 keeps teachers fixed), `violation` /
`violation_ref` (derivable), `based_on_timetable_id` scenarios, `absence` / `coverage`.

**`constraint_instance` / `constraint_ref` is a separate migration and a separate question.**
ADR 0005 commits to constraints-as-data, and the first solver needs a *default set* to exist.
The cheapest honest route is to land the table in the same PR as the registry — so the
`type_key`s and JSON Schemas it stores are real ones — and to defer the configuration UI.
Shipping the solver against a hardcoded constraint list would be a second place the rules
live, which is the one thing `CLAUDE.md` names as the worst possible bug in this product.

**Nothing in the existing schema needs changing.** `timeslot.id` is stable by construction
(ADR 0011), `activity.id` is stable, `room.id` is one identity per physical room (ADR 0014).
A `timetable_entry` referencing all three is additive, and the `RESTRICT` foreign key it
brings is what finally makes `commands/grid.rs`'s documented promise — that an orphan release
cannot delete a slot a timetable uses — true rather than anticipatory.

---

## 8. UI assessment

| Capability | State | Evidence |
| --- | --- | --- |
| View an empty timetable grid | **Different thing exists** | `GridPreviewSection.tsx` (702 lines, 587-line test) renders cycle days × periods and says which cells are materialised. It is the *time model* grid, not a timetable grid: it holds no activities and has no concept of a placement |
| View cycle days and periods | **Functional** | As above |
| Generate a timetable | **Missing** | No screen, no command |
| Generation progress | **Missing** | No event channel; `solver://progress` exists only in `docs/SOLVER.md` |
| Cancel generation | **Missing** | — |
| Review unscheduled activities | **Missing** | — |
| See placement conflicts | **Missing** | — |
| Drag or manually assign | **Missing** | `apps/desktop/src/ui/` (intended for grid and drag-and-drop primitives) is an empty directory |
| Lock placements | **Missing** | — |
| Switch teacher / class / student / room views | **Missing** | — |
| Export or print | **Missing** | — |

`apps/desktop/src/features/` — which `docs/ROADMAP.md`'s target structure reserves for
`constraints`, `generate`, `timetable` and `reports` — is empty.

**Recommended MVP interface, and nothing more.** Three screens:

1. **Pre-flight.** The Layer-1 diagnostics, listed with the remedy, plus a "nothing is
   obviously impossible" state that does *not* claim feasibility. This is the screen that
   earns trust, and `evara-core`'s existing restraint — never report that something *can* be
   scheduled — must carry over to it verbatim.
2. **Generate.** One button, a seed, a time limit, a progress line, a cancel. On completion:
   score, placed count, and the unplaced list with each activity's blocking reason.
3. **Timetable grid, read-only, four lenses.** Cycle days × periods, one lens at a time
   (teacher, class section, student group, room), multi-period activities rendered as spans.
   Read-only is the point: drag-and-drop without `validate_move` would be a rule in the view
   layer.

Explicitly **not** in the MVP interface: the constraint configuration forms, the violation
inspector, the score health panel, undo/redo, repair scoping, export. Each is a later PR with
a clear owner in `docs/ROADMAP.md`.

---

## 9. Multi-school scheduling recommendation

### 9.1 What the architecture already forces

Three facts decide this, and all three are in the schema rather than in anyone's preference:

1. **Two schools cannot share a timeslot.** `cycle.school_id` is `NOT NULL`, `timeslot`
   derives `school_id` from the cycle under a trigger pair (`V4:150-185`), and timeslot
   identity is `(cycle_day, period, term)`. Schools own disjoint grids. A solver that
   "solved the project" would still be solving N disjoint value ranges.
2. **Two schools have no common clock.** `school.timezone` is per school; no project-level
   calendar aligns cycle days to dates; `calendar_day` is per school. Comparing cycle
   positions across schools is meaningless and comparing wall-clock times across schools is
   wrong until timezones and dates are both resolved — which needs `calendar_day` coverage
   that today is entered by hand, one date at a time.
3. **Within one school, real time *is* already computable.** One timezone, one set of
   `period.starts_at`/`ends_at`, cycle days shared across that school's bell schedules. The
   intra-school half of D22 is answerable from stored data with no new table.

### 9.2 The three options, against this architecture

| | Independent per school | Across all schools | **Hybrid (recommended)** |
| --- | --- | --- | --- |
| Fits disjoint grids | ✅ naturally | ❌ needs an alignment model that does not exist | ✅ |
| Shared teacher safety | ❌ silently unprotected | ✅ if alignment existed | ⚠️ **detected and reported**, not solved |
| Shared room safety | ❌ silently unprotected | ✅ if alignment existed | ✅ within a school; reported across schools |
| Different bell schedules | ✅ | needs D22 Option A or C | ✅ solved by interval overlap within a school |
| Different calendars | ✅ | ❌ blocked on manual `calendar_day` | ✅ |
| Performance | Best — N small problems | Worst — one problem N× larger with no shared structure | Best |
| Couples unrelated schools | No | Yes, always | No |
| Incremental cost | Lowest | Highest | Low |

Option 2 is not merely expensive; it is **not implementable on the current model**, because
there is no artefact that says two schools' Period 3 on Day A are the same minutes. Option 1
is implementable and silently wrong for the exact configurations ADR 0014, ADR 0021 and
ADR 0022 were written to support.

### 9.3 Recommendation

**Generate per `(school, cycle, term)`. Make shared-resource occupancy correct inside a
school by comparing real time, and outside a school make it *visible* rather than guessed
at.**

Concretely, three rules:

1. **The unit of generation is one school's grid.** A solve names a school, a cycle, a term
   and (per D34) the bell schedules in play. This is what the schema already scopes and what
   the interface already scopes to (ADR 0023).
2. **Within a school, `room-clash` and `resource-clash` key on `room_id` / `resource_id` and
   compare `(cycle_day, [starts_at, ends_at))` intervals, not `timeslot_id`.** This is D22
   Option A, restricted to the case where the data needed to implement it already exists: one
   timezone, one cycle, periods carrying wall-clock. It costs an interval-overlap index in
   the loader and nothing in the schema. It makes a hall shared by two campuses on different
   bells safe — the case that ADR 0014 says is a correctness failure and that Option B would
   have solved by refusing a legitimate setup.
3. **Across schools, detect and report; never silently assume.** Before a solve, scan for
   entities in scope that are shared with another school — a teacher with more than one
   active `teacher_school`, a room whose `room_campus` rows span campuses of more than one
   school, a resource likewise, a student with concurrent `ENROLLED` spells. Report each as a
   named pre-flight warning: *"Ms Okafor is employed by Northgate and Southmoor. Evara cannot
   compare their timetables and will not detect a clash between them."* Place the activity
   anyway. Do not block, because blocking would make a supported configuration unusable; do
   not pretend, because pretending is how a hall gets double-booked.

**What this defers, stated so nobody assumes otherwise:** coordinated cross-school
generation, cross-school clash detection, and project-level availability. Each becomes
possible once a project-level time alignment exists — the honest version of which is D22
Option C, a campus/school alignment table, and which should not be built before a user has
asked for it.

A draft record is at [`adr/0032-timetable-generation-is-scoped-to-one-school.md`](adr/0032-timetable-generation-is-scoped-to-one-school.md),
**Status: Proposed**. Nothing is implemented in either direction.

---

## 10. Blocking gaps

Severity **Blocker** = the first usable timetable cannot be produced without it.

| # | Capability | Current implementation | Evidence | Deps | Milestone | Acceptance criteria |
| --- | --- | --- | --- | --- | --- | --- |
| B1 | A decision on which bell schedule an activity schedules against | None | `V15:423`, `V4:82`, `repo/time_model.rs:1651` | — | M1 | D34 resolved and recorded as an ADR; the rule is stated for a section with no campus and for a school holding both school-wide and campus schedules |
| B2 | Group overlap semantics (D12) | `parent_group_id` stored; no read walks it | `V10:128-130`; `repo/people.rs` has no transitive read | — | M1 | A design note plus a fixture covering combined classes, split sets and composite groups; `student-clash` produces no phantom and misses no real clash on it |
| B3 | Scheduling vocabulary in `evara-core` | Absent | `crates/evara-core/src/lib.rs:63-65` lists five modules, none of them scheduling | — | M2 | `Score` (3×`i64`, lexicographic, `is_feasible`), `Placement`, `Assignment`, `Problem`, `CompiledConstraint`, dense indices; unit-tested; no I/O |
| B4 | Project-level pre-solve diagnostics | Two per-section checks only | `crates/evara-core/src/lessons.rs:244-300`; `docs/SOLVER.md` §8 Layer 1 lists six, none built | B3 | M3 | The six Layer-1 checks run over a whole school and name a remedy; the infeasible fixture produces a specific cause with no solver involved |
| B5 | `Problem` loader (SQLite → in-memory) | Absent | `repo/lessons.rs:1696 activities` is per class section; no school-wide activity read exists | B1, B2, B3 | M4 | One call assembles a whole school's problem; UUID↔index translation happens once; loading the medium fixture is deterministic and round-trips |
| B6 | Constraint registry and the MVP hard evaluators | Absent | `crates/evara-constraints/src/lib.rs` — 23 lines, no code | B3 | M5 | 8 evaluators + `activity-scheduled`; one module, one registry line, one test file and one `CONSTRAINTS.md` row each; `ReferenceScorer` agrees with hand-computed scores on the tiny fixture |
| B7 | `constraint_instance` / `constraint_ref` + default seeding | Absent | No migration defines them | B6 | M5 | `V16`; both tables in `TRACKED_TABLES`; a new project opens with the default M1 set seeded; params validate against the registry's JSON Schema on write |
| B8 | A solving algorithm | Absent | `crates/evara-solver/src/lib.rs` — 20 lines, no code | B5, B6 | M6 | Construction heuristic places every activity of the tiny fixture with `hard == 0`; same seed reproduces byte-identical output; `evara solve` runs it headless |
| B9 | Timetable persistence | Absent | No `timetable`, `timetable_entry` or `solve_run` table | B8 | M7 | `V17`; a generated assignment is written in one transaction and reopened identically; the three §9 indexes exist; all three tables tracked |
| B10 | Generation orchestration | Absent | No background thread, no event, no cancel token | B8, B9 | M8 | Solve runs off the UI thread; progress events carry best score and step count; cancel returns the best-so-far within 100 ms; `solve_run` records seed, settings and outcome |
| B11 | A timetable view | Absent | `apps/desktop/src/features/` is empty | B9 | M9 | Four read-only lenses over cycle days × periods; multi-period activities span; unplaced activities listed with a reason |
| B12 | Fixtures | A stale prototype file | `fixtures/README.md` states nothing reads `basic-school/problem.json` | B3 | M2 | `tiny`, `medium` (~400 activities) and `infeasible`, generated by `evara fixture`, synthetic names only, checked in |

---

## 11. Non-blocking gaps

| # | Capability | Current | Severity | Milestone |
| --- | --- | --- | --- | --- |
| N1 | `room-capacity` has no left-hand side (D35) | `room.capacity` exists; nothing to compare it with | Important | Decide in M1; implement in M5 or exclude from the MVP set |
| N2 | `room-clash` across schools unprotected (D22, widened) | Representable, not comparable | Important | M3 reports it (§9 rule 3); solving it is post-M1 |
| N3 | Teacher workload ceilings unread | 4 columns carried since `V10:62-70` | Optional | M10, with the soft constraints |
| N4 | Resource scheduling | `resource` + `resource_campus` exist, nothing links them to an activity | Optional | Post-MVP; needs `activity_resource` |
| N5 | Per-occurrence overrides | `activity_teacher`, `activity_group`, `activity_room_candidate` not built, deliberately | Optional | Post-MVP (ADR 0029 §5) |
| N6 | Lines / blocks (`same-period`) | `line`, `line_member` documented, not built | Optional | Post-MVP |
| N7 | `activity.is_locked` | Absent | Optional | M11, with manual editing |
| N8 | Calendar auto-mapping | One date at a time | Important for usability, not for scheduling | Independent; could ship any time |
| N9 | Availability UI for `RESOURCE`, `CAMPUS`, `SUBJECT` | Model supports six kinds, UI covers three | Optional | With N4 |
| N10 | `availability.effective_from`/`_to` not collected | Columns carried, no screen | Optional | Post-MVP |
| N11 | Shared teacher's availability is per school | `availability.school_id NOT NULL` | Important for cross-school, not for MVP | Blocked behind the same alignment work as N2 |
| N12 | `evara-cli` subcommands | Help text only | Important for CI and benchmarking | M2 (`fixture`), M3 (`diagnose`), M6 (`solve`, `bench`) |
| N13 | `evara-report`, `evara-project` | Doc-comments | Optional for the MVP | M12 and Phase 9 respectively |

### Architectural defects to resolve before solver development

There is **no evidence of a defect requiring a rewrite**, and this audit does not propose
one. Four things should nonetheless be settled first, and all four are decisions or small
additions rather than restructuring:

1. **D34** (§4.1) — the loader cannot be written without it.
2. **D12** — `student-clash` cannot be written without it.
3. **D35** (§4.4) — decides whether `room-capacity` is in the MVP set at all.
4. **Fixtures (B12)** — the reference-vs-incremental property test, the benchmark baselines
   and the infeasibility diagnostics all consume them, and `CLAUDE.md` rule 3 forbids the
   shortcut of using real data.

One consistency point, not a defect: `V15:825`'s `activity` has no `school_id` and resolves
one through four tables. The loader will need a school-wide activity read, and it must use
`require_school_owner` through that chain rather than acquiring a shortcut column — the thing
ADR 0029 §1 exists to prevent.

---

## 12. Documentation vs. implementation discrepancies

Recorded because the brief asks for them explicitly, and because each one could mislead a
reader into thinking something is built.

| # | Document | Claim | Reality |
| --- | --- | --- | --- |
| D-1 | `CLAUDE.md` §3 | *"`timetable_entry` is unique on `(timetable_id, activity_id, timeslot_id)`"*, present tense, as a reason `occurrences_per_cycle` does not exist | The table does not exist in any migration. The *reasoning* is sound and `V15`'s design is correct; the sentence describes a future table as a current one |
| D-2 | `docs/OPEN-DECISIONS.md` D22 | *"`period_structure` is school-wide, and nothing ties a campus to exactly one"* | `period_structure.campus_id` exists (`V4:82`) and `timeslot.campus_id` is derived from it under a trigger pair (`V4:150-185`). A campus *can* be tied to a bell schedule. D22's **conclusion stands** — timeslot identity still carries no campus, and two structures still produce different rows for overlapping minutes — but its premise is stale and should be corrected when D22 is settled |
| D-3 | `apps/desktop/src-tauri/src/commands/grid.rs:35-38` | *"A slot a future `timetable_entry` references cannot be deleted at all: the foreign key is `RESTRICT`, so the release fails with `stillReferenced`"* | Correctly hedged with "future", but no such foreign key exists today. The only `RESTRICT` reference to `timeslot` is `availability.timeslot_id` (`V11:52`) |
| D-4 | `docs/ROADMAP.md` Phase 4 | *"Migration `0004`: `constraint_instance`, `constraint_ref`"* | Migrations are numbered `V1`…`V15`; the next is `V16`. A naming artefact from before Phase 1 |
| D-5 | `docs/DATA-MODEL.md` §9 | Lists `idx_entry_timetable_slot`, `idx_entry_timetable_act`, `idx_entry_room` among "indexes that matter" | They belong to the unbuilt `timetable_entry`. The section mixes built and planned indexes without marking which is which |
| D-6 | `docs/ARCHITECTURE.md` §3 rule 4 | *"`evara-db` … loads a `Problem` for the solver and persists a `Solution` back"*, present tense | No loader, no writer, no `Problem` type |
| D-7 | `docs/CONSTRAINTS.md` §3 | ~22 types marked `M1` | 0 implemented. The `Status` column is a plan, which the file's preamble makes clear, but a reader scanning the table will not see that |
| D-8 | `docs/ROADMAP.md` Phase 3 | Lists `fixtures/` as remaining and says *"Closes M1-7 once the fixtures land"* | Accurate. Noted here as confirmation: **M1-7 is not closed**, and the stage table's "Stage C ✅ complete" refers to the lessons model, not to the whole of Phase 3 |

None of these is a correctness bug. D-2 is the one worth fixing soonest, because it
understates what the model can already express and so makes D22 look harder than it is.

`docs/ROADMAP.md` completion statuses are **not** modified by this audit: every ✅ it carries
is supported by code and tests found in this review.

---

## 13. MVP definition — the first usable timetable generator

**Scope, stated as a sentence a user would recognise:** *for one school, one cycle and one
term, place every activity of every class section into a cycle day and period, assign a room
where the course requires a room type, never clash a teacher, a student group or a room,
respect what the school has marked unavailable, keep a double period together — and name
every activity it could not place, with the reason.*

**In scope**

- Generation for one `(school, cycle, term)`, bell schedules resolved per D34.
- Hard: `teacher-clash`, `student-clash`, `room-clash` (interval-based within the school),
  `room-type-required`, `entity-unavailable` (teacher, room, group),
  `duration-contiguous`, `no-teaching-in-period`.
- Medium: `activity-scheduled` — a partial timetable with a named shortfall beats nothing.
- Pre-solve Layer-1 diagnostics over the whole school, plus the cross-school sharing
  warnings of §9 rule 3.
- A construction heuristic only. Deterministic, seeded, cancellable, with progress.
- Persistence of one generated timetable and its run, reopenable.
- A read-only grid with four lenses, and the unplaced list.
- `evara fixture`, `evara diagnose`, `evara solve` headless.

**Out of scope, deliberately**

Every soft constraint. Local search and incremental scoring. Manual editing, drag-and-drop,
`validate_move`, locks, undo/redo. Repair and partial regeneration. Lines and activity links.
Resource scheduling. Room candidate lists and capacity (pending D35). Teacher as a planning
variable (D7). Cross-school coordinated generation. Export, print and `.evara` transfer.
Published/archived timetable lifecycle. Scenario lineage.

**The MVP is complete when** the medium fixture generates a timetable with `hard == 0` in one
run, the same seed reproduces it exactly, the infeasible fixture produces a specific named
cause before the solver starts, and a user can reopen a generated timetable and read it by
teacher, class, group and room.

---

## 14. Performance and scalability

**No benchmark has ever been run in this repository and none is invented here.** There is no
solver to measure, no fixture to measure it against, and `evara-cli bench` is help text. What
follows is a target set and a harness recommendation.

### 14.1 Expected problem sizes

Derived from the brief's scenarios and the schema's shape (a cycle of 5–10 days × 6–10
periods):

| Scenario | Teachers | Students | Activities | Timeslots | Rooms | (activity × slot × room) |
| --- | --- | --- | --- | --- | --- | --- |
| Small | 20 | 300 | 100 | ~40 | ~20 | 80 K |
| Medium | 80 | 1,000 | 500 | ~50 | ~60 | 1.5 M |
| Large | 200 | 2,500 | 1,500 | ~50 | ~200 | 15 M |

### 14.2 Assessment against the planned design

- **Memory is not a risk.** `docs/SOLVER.md` §2's dense-index `Problem` with bitset
  occupancy is a few MB at the large size — 1,500 activities × a handful of `u32` fields,
  plus `(entity, timeslot)` bitsets of (200+1,500 groups) × 50 bits. Nothing here approaches
  a desktop memory limit.
- **Construction is cheap.** Greedy most-constrained-first is O(activities × slots × rooms)
  in the worst case with early filtering — 15 M candidate evaluations at the large size, each
  a handful of bitset tests. Seconds, not minutes.
- **Local search is the open-ended cost.** `docs/OPEN-DECISIONS.md` R2 says so already. This
  is the argument for shipping construction-only first: it bounds the MVP's performance risk
  to something measurable.
- **The real risk is `validate_move` latency**, because `docs/SOLVER.md` §6 promises hover
  validation. That requires `affected()`-scoped evaluation, which requires the incremental
  scorer, which requires the property test. None of it is MVP work, but the `Problem` shape
  chosen in B3 decides whether it is cheap later.
- **Data loading is unmeasured and plausibly the first bottleneck.** The loader must read
  activities, sections, teachers, groups, the group hierarchy, availability (three
  granularities × term scoping) and timeslots for a whole school. `repo/lessons.rs:1696` is
  per class section; a naive loader doing one query per section would be 500+ round trips at
  the medium size. This should be designed as a small number of set-based reads.

### 14.3 Recommended targets

To be asserted by `evara-cli bench` with baselines in CI, not assumed:

| Measure | Small | Medium | Large |
| --- | --- | --- | --- |
| `Problem` load | < 200 ms | < 700 ms | < 2 s |
| Pre-solve diagnostics | < 100 ms | < 300 ms | < 1 s |
| Construction to first complete assignment | < 500 ms | < 3 s | < 15 s |
| Reach `hard == 0` (MVP, construction only) | < 1 s | < 10 s | best-effort, reported |
| Cancel acknowledged | < 100 ms | < 100 ms | < 100 ms |
| Progress event cadence | ≤ 250 ms | ≤ 250 ms | ≤ 250 ms |
| Peak RSS above idle | < 50 MB | < 150 MB | < 400 MB |
| `validate_move` p99 (later milestone) | < 1 ms | < 1 ms | < 2 ms |

### 14.4 Harness

`evara-cli bench` runs each checked-in fixture N times with fixed seeds and emits a JSON
record of wall time, peak RSS, step count and final score. Score baselines are committed next
to the fixtures from the first solver PR, so a regression fails CI rather than being noticed
later — `docs/ROADMAP.md` Phase 6 already requires this. Determinism is itself a test: the
same seed twice must produce byte-identical output, and that assertion costs nothing and
catches the whole class of accidental `Instant::now()` or `HashMap`-iteration-order bugs that
`CLAUDE.md` forbids.

---

## 15. Proposed milestone sequence

Twelve milestones, each one reviewable PR. The brief's candidate sequence is followed with
three changes, each justified by repository evidence:

- **A decisions-only milestone comes first (M1).** Three decisions block the loader and the
  registry. Making them inside an implementation PR would bury them.
- **Fixtures move earlier (M2), not to Phase 3's position.** The diagnostics, the evaluators,
  the property test and the benchmarks all consume them. Building them last means every
  earlier milestone is tested against ad-hoc data.
- **Diagnostics come before solver input assembly.** They are the highest value for the
  lowest effort (`docs/SOLVER.md` §8 says so), they are shippable on their own, and they
  surface data problems that would otherwise appear as mysterious solver failures.

Naming is descriptive, not "Stage E": these are not a continuation of the setup stages.

---

### M1 — Scheduling decisions

**Objective** Close the three decisions that block every subsequent milestone.
**Scope** D34 (which bell schedule an activity schedules against), D35 (what `room-capacity`
compares against, or that it is excluded), D12 (group overlap semantics). Accept or revise
the §9 recommendation as ADR 0032.
**Dependencies** None.
**DB / Rust / Tauri / Frontend** None.
**Tests** None.
**Done when** Three ADRs (or ADR + `OPEN-DECISIONS` resolutions) are merged, each naming the
alternative it rejects. D22's stale premise (D-2) corrected.
**Risks** D12 is genuinely subtle — combined classes and composite groups are where
competitor products differentiate. Budget a design note, not a paragraph.
**Excludes** Any code.

---

### M2 — Scheduling vocabulary and fixtures

**Objective** The types the solver is defined over, and the data everything after this is
tested against.
**Scope** `evara-core`: `Score`, `Placement`, `Assignment`, `Problem`, `ActivityFact`,
`TimeslotFact`, `RoomFact`, `TeacherFact`, `GroupFact`, `UnavailabilityIndex`,
`CompiledConstraint`, `CancelToken`, `ProgressSink`. `evara-cli fixture` generating `tiny`,
`medium` and `infeasible` into `fixtures/`.
**Dependencies** M1 (D34 shapes `TimeslotFact`; D35 shapes `ActivityFact`).
**DB** None. **Tauri** None. **Frontend** None.
**Tests** Unit tests for lexicographic `Score` ordering and `is_feasible`; a round-trip test
for fixture serialisation; a test asserting the generator is deterministic for a seed.
**Done when** Three fixtures are checked in with synthetic names only, `cargo test` covers the
new types, and `evara-core` still has no I/O.
**Risks** Over-designing `Problem` before a consumer exists. Mitigate by keeping it minimal
and letting M4 and M5 push back.
**Excludes** Any evaluator; any database access.

---

### M3 — Pre-solve input validation

**Objective** Tell a school what is impossible before anyone presses Generate.
**Scope** `docs/SOLVER.md` §8 Layer 1's six checks at school scope, plus §9 rule 3's
cross-school sharing warnings. Extends `evara_core::lessons` rather than replacing it — the
existing per-section findings stay.
**Dependencies** M2.
**DB** No migration; new read-only repository queries.
**Rust** `evara-core` diagnostics; `evara-db` the reads that feed them.
**Tauri** One command, `scheduling_check(school_id, cycle_id, term_id)`.
**Frontend** The Pre-flight screen (§8).
**Tests** The `infeasible` fixture produces the specific named cause; a feasible fixture
produces no finding and **no claim of feasibility**.
**Done when** `evara-cli diagnose` and the screen agree, and the restraint rule holds — the
output never says a timetable exists.
**Risks** Over-reporting. A check that fires on a merely-hard setup destroys trust faster than
no check at all; keep every finding a counting argument.
**Excludes** Any solver; any constraint evaluation.

---

### M4 — Solver input assembly

**Objective** SQLite → `Problem`, once, correctly.
**Scope** The loader: school-wide activity read through the `subject → course →
class_section → activity` chain, group-hierarchy expansion to leaf sets, availability
compiled to bitsets across all three granularities and term scoping, the value range chosen
per D34, UUID↔dense-index translation.
**Dependencies** M1, M2.
**DB** No migration. New set-based reads, designed to a small fixed query count.
**Rust** `evara-db` loader. **Tauri** None yet. **Frontend** None.
**Tests** Loading each fixture produces the expected counts; index translation round-trips;
`require_school_owner` refuses another school's activity; a performance test asserts the
query count does not scale with section count.
**Done when** `evara solve --dry-run` prints a problem summary from a real `.evaraproj`.
**Risks** The N+1 query shape in §14.2. Assert the query count in a test.
**Excludes** Any scoring.

---

### M5 — Constraint registry and the MVP hard set

**Objective** One implementation of each rule, in the crate four callers will share.
**Scope** `ConstraintEvaluator` trait; the registry; eight hard evaluators plus
`activity-scheduled`; a JSON Schema and an `explain()` each; `ReferenceScorer`. `V16` for
`constraint_instance` / `constraint_ref` with the default M1 set seeded into new projects.
**Dependencies** M2, M4, and M1's D35 (which decides whether `room-capacity` is in the set).
**DB** `V16`; both tables into `TRACKED_TABLES`.
**Rust** `evara-constraints` (new code), `evara-db` (constraint repository, param validation
against the registry's schema on write).
**Tauri** List/read commands only. **Frontend** None — configuration UI is later.
**Tests** One isolated unit test per evaluator; a synthetic fixture exercising each; a
schema-level test that every registered `type_key` has a schema, a `CONSTRAINTS.md` row and
an `explain()`; `ReferenceScorer` matches hand-computed scores on `tiny`.
**Done when** All five definition-of-done items in `CLAUDE.md` §4 hold for each of the nine.
**Risks** Nine evaluators is the largest PR in this plan. Split into two — clash/occupancy
first, then availability/duration — if review drags.
**Excludes** `IncrementalScorer`; every soft type; the configuration UI.

---

### M6 — Construction solver

**Objective** A timetable, from real data, deterministically.
**Scope** `SolverEngine` trait; greedy most-constrained-first construction; termination on
time limit and cancel; `ProgressSink`; `SolveOutcome` carrying the assignment, the score and
the unplaced list with reasons. `evara-cli solve` and `bench` with committed baselines.
**Dependencies** M4, M5.
**DB** None. **Tauri** None yet. **Frontend** None.
**Tests** `tiny` solves to `hard == 0`; the same seed produces byte-identical output; cancel
mid-run returns best-so-far; `infeasible` returns a partial assignment and names what it
could not place.
**Done when** `evara solve fixtures/medium` prints a feasible score and CI holds a baseline.
**Risks** Construction alone may not reach `hard == 0` on tight timetables. That is an
acceptable MVP outcome *provided* the unplaced list is specific — which is why
`activity-scheduled` is `MEDIUM` rather than `HARD`.
**Excludes** Local search, incremental scoring, soft constraints, repair.

---

### M7 — Timetable persistence

**Objective** A generated timetable survives closing the application.
**Scope** `V17`: `timetable`, `timetable_entry`, `solve_run`, plus the three
`docs/DATA-MODEL.md` §9 indexes. Writer (`Assignment` → rows, one transaction) and reader.
**Dependencies** M6.
**DB** `V17`; three `TRACKED_TABLES` entries; `timetable_entry.timeslot_id` `RESTRICT`, which
finally makes `commands/grid.rs`'s documented orphan-release guarantee true.
**Rust** `evara-db` repository. **Tauri** read commands. **Frontend** None.
**Tests** Write-then-read is identical; the unique key rejects a duplicate
`(timetable, activity, timeslot)`; deleting a timeslot a timetable uses is refused with
`StillReferenced`; change-log triggers fire; `rev` preconditions hold.
**Done when** A fixture solve round-trips through SQLite unchanged.
**Risks** Low. The mechanisms are all established.
**Excludes** Publishing, archiving, scenario lineage, `violation` tables.

---

### M8 — Generation orchestration

**Objective** Generate from the application without freezing it.
**Scope** Dedicated OS thread, `solver://progress` events, cancellation via `AtomicBool`,
`solve_run` lifecycle, settings (seed, time limit).
**Dependencies** M6, M7.
**DB** None. **Rust** thin. **Tauri** `solve_start`, `solve_cancel`, `solve_status`.
**Frontend** The Generate screen (§8).
**Tests** Command-layer tests for start/cancel/status; a UI test asserting progress renders
and cancel is honoured.
**Done when** A user presses Generate on a real project and watches it finish or cancels it.
**Risks** Tauri state and thread lifetime. Keep the solver thread owning no database handle —
`evara-solver` must not acquire one.
**Excludes** Repair, scoping, multi-start.

---

### M9 — Timetable review

**Objective** Read the thing that was generated.
**Scope** `evara-report` projections (master, by teacher, by class section, by group, by
room); a grid component driven by cycle days × periods with multi-period spans; the unplaced
list.
**Dependencies** M7.
**DB** None. **Rust** `evara-report` (first code). **Tauri** projection commands.
**Frontend** The timetable screen, read-only.
**Tests** Projection unit tests against a fixture timetable; component tests for span
rendering and for the empty state.
**Done when** All four lenses render the medium fixture correctly.
**Risks** The grid is the most-used component in the product; invest in it.
**Excludes** Drag-and-drop, conflict highlighting, the score panel.

---

### M10 — Local search and incremental scoring

**Objective** Better timetables, honestly.
**Scope** `IncrementalScorer` with per-constraint indexes; Late Acceptance Hill Climbing;
`ChangeTimeslot`, `ChangeRoom`, `SwapTimeslots`, `SwapRooms`; the first soft constraints —
`spread-across-days` and `teacher-min-gaps`; the reference-vs-incremental property test.
**Dependencies** M6.
**Tests** **The property test is the deliverable**: thousands of random moves on random
synthetic problems, asserting `IncrementalScorer == ReferenceScorer` after every one. It gates
every later constraint (R1, the highest-rated risk in `OPEN-DECISIONS`).
**Done when** The property test is green in CI and benchmark baselines improve.
**Risks** R1. This is the single most bug-prone part of any solver, and the two-scorer
structure is the only defence.
**Excludes** `Ruin&Recreate`, `ChainMove`, `LineMove`, multi-start.

---

### M11 — Manual editing

**Objective** Change a placement by hand, and be told why you cannot.
**Scope** `validate_move`, `apply_move`, drag-and-drop with hover validation, locks
(`activity.is_locked` + `activity-fixed`), the violation inspector, in-memory undo/redo.
**Dependencies** M9, M10 (`affected()` scoping is what makes hover validation fast enough).
**DB** One migration for `activity.is_locked`, or `timetable_entry.is_locked` alone.
**Done when** An illegal drop is refused with a structured `Explanation` naming the
conflicting entries, and generation and the editor cannot disagree because both call the same
registry.
**Risks** The `CLAUDE.md` rule this milestone exists to honour. Any rule computed in
TypeScript here is an architecture bug.

> **Delivered as M9A, and this plan was right about the thing that mattered.** The DB line's
> second option is what shipped — `timetable_entry.is_locked` alone, and no migration at all,
> because `V17` already held it ([adr/0045](adr/0045-a-lock-is-a-property-of-a-placement.md)).
> Two scope items were **not written** because they already existed: `validate_move` is
> `IncrementalScorer::probe`, and the `affected()` scoping this plan names as the dependency is
> what makes a whole candidate panel answerable in one call. The undo/redo stack is in the
> **session, in Rust**, not in memory in the view layer — an inverse command's precondition is a
> revision the database owns. One item is still outstanding: the violation inspector's
> **entity links** into the grid. [TIMETABLE-EDITING.md](TIMETABLE-EDITING.md).
**Excludes** Repair runs, `minimise-disruption`.

---

### M12 — Output

**Objective** Get a timetable out of the application.
**Scope** Print-oriented HTML per view, a print stylesheet, CSV export, the renderer trait.
**Dependencies** M9.
**Done when** Each of the four views prints legibly across pages and exports to CSV.
**Risks** D10 — webview printing may not survive multi-page timetables with repeated headers.
**Excludes** A Rust PDF renderer.

---

## 16. Risks and open decisions

### Open before solver work

| ID | Decision | Status | Blocks |
| --- | --- | --- | --- |
| **D34** *(new)* | Which bell schedule does an activity schedule against? | Open | M4 |
| **D35** *(new)* | What does `room-capacity` compare against? | Open | M5 |
| D12 | Student clash semantics when membership is not modelled | Open since Phase 0 | M5 (`student-clash`) |
| D22 | How cross-campus time is compared for a shared room | Open; §9 answers the intra-school half | M5 (`room-clash`), post-M1 for cross-school |
| D7 | Is the teacher a planning variable? | Recommendation standing: no, in M1 | Shapes `Placement` in M2 |

### Carried risks

- **R1 — incremental scoring correctness.** Highest-rated risk in `OPEN-DECISIONS`, and the
  reason M10 is a separate milestone whose deliverable is a property test.
- **R2 — solver quality is open-ended.** Mitigated by shipping construction-only in M6 and
  bounding the MVP's definition of success to `hard == 0`.
- **R3 — constraint count is the real scope.** 60+ types specified, 9 in the MVP. The plan is
  explicit about which.
- **R5 — performance unknown until measured.** §14 gives targets and a harness; no number in
  this document is a measurement.
- **R9 — fixture realism.** Naive fixtures hide the pathologies that break real timetables.
  M2 should carry the generator knobs `fixtures/README.md` already names: schedule tightness,
  specialist-room scarcity, part-time staff fraction.
- **R-shared-room-time** (D22's) — until §9 rule 2 ships, a school sharing a hall between two
  campuses on different bells has no protection. §9 rule 3 makes the cross-school version
  *visible*; it does not make it safe.

---

## 17. Recommended immediate next PR

**M1 — Scheduling decisions.** Three ADRs and no code.

It is the right next PR for three reasons the audit established rather than assumed. The
loader (M4) cannot choose a value range without D34, and D34 has no default that is obviously
right. The registry (M5) would otherwise ship `room-capacity` as a type that cannot be
evaluated, or silently omit it. And `student-clash` — one of the three constraints without
which a timetable is not a timetable — rests on a question open since Phase 0 that
`docs/OPEN-DECISIONS.md` itself says "needs its own design note and a dedicated fixture
before Phase 5".

If a code PR is wanted first instead, **M2 (scheduling vocabulary and fixtures)** is the only
one that is genuinely unblocked, and even it is shaped by D34 and D35. The decisions are two
days of thinking that save a rewrite.
