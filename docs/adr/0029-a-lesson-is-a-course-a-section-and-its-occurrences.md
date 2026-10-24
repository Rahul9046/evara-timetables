# ADR 0029 — A lesson is a course, a class section, and one row per occurrence

- Status: **Accepted**
- Date: 2026-10-15
- Implements: `ROADMAP.md`'s Phase 3, Stage C. Its two instructions are the shape of this
  decision: *"resolve the school through `subject.school_id` rather than adding a second
  `school_id` to `course`"*, and *"expand a course's `periods_per_cycle` into activities
  with `split_index`, respecting `default_duration_periods`"*.
- Builds on: [ADR 0006](0006-group-based-scheduling-first.md) (group-based first, request
  schema preserved), [ADR 0010](0010-structure-deletion-semantics.md) (deletion follows
  ownership), [ADR 0019](0019-multiple-schools-in-one-project.md) (what is school-owned),
  [ADR 0027](0027-revisions-are-checked-on-write.md) (a write states the revision it was
  built on). The teacher-assignment half is [ADR 0028](0028-assigning-a-lesson-warns-about-qualifications.md).
- Corrects: `DATA-MODEL.md` §5, which sketched `course (id, school_id, subject_id, ...)`.

## Context

Stage C turns a curriculum into scheduling demand. `V14` left the subject; everything
between a subject and a timetable had to be designed, and `DATA-MODEL.md` §5 and §7 carried
a sketch rather than a decision. Four questions in that sketch had no settled answer, and
each of them is cheap now and expensive once projects are populated.

## Decision

### 1. The chain carries the school. No table in it holds a `school_id`

```
subject ──→ course ──→ class_section ──→ activity
(school-owned)                            (one placeable occurrence)
```

`V15` adds six tables — `course`, `class_section`, `class_teacher`, `class_group`,
`class_enrolment`, `activity` — and **not one of them has a `school_id` column.** Every
school-owned read takes an explicit `school_id` and resolves it through the chain;
`require_school_owner` takes an owner query per table, and `lessons.rs` writes three of
them out (`COURSE_OWNER`, `SECTION_OWNER`, `ACTIVITY_OWNER`).

This corrects `DATA-MODEL.md` rather than implementing it. The roadmap had already said so
for `course` specifically, and `CLAUDE.md` says it in general: *a child's school is resolved
through its parent chain, never by adding it a second `school_id` column.* A second place to
record ownership is a second place for it to be wrong, and the foreign keys already enforce
the chain.

The cost is honest and visible: a three-table join appears in five of `V15`'s triggers, and
`setup_course_update` takes `schoolId` as a parameter where `setup_subject_update` lifts it
off the input. Neither is a defect. The join is the domain statement — a course belongs to a
school only by way of its subject — and the extra parameter is the one place the design
shows through to the interface.

`tests_lessons::a_course_has_no_school_column_and_resolves_one_through_its_subject` asserts
the absence of the column on all six tables, so a future migration cannot quietly add one.

### 2. One activity row is one occurrence. There is no `occurrences_per_cycle`

**A class section needing four single periods a cycle holds four `activity` rows.** One
needing a double period once a cycle holds **one** row with `duration_periods = 2`. The
occurrence count is how many rows there are; the demand is `SUM(duration_periods)`.

This is the decision most likely to be read the other way round, and three things depend on
it:

- **One activity is one placeable unit, so the solver's variable set *is* the activity
  rows.** A column saying "and three more like me" would make a row mean a variable
  sometimes and a template other times, and every evaluator would have to know which.
- **`timetable_entry` is already unique on `(timetable_id, activity_id, timeslot_id)`**
  (`DATA-MODEL.md` §7) *because a multi-period activity occupies several slots*. Four
  occurrences of one row would need a fifth column in that key to tell them apart, and
  every constraint reading an audience would have to carry it.
- **The commonest real constraint in this product category addresses the occurrences
  individually**: "spread these four Mathematics periods across four different days". They
  have to *be* individual rows for `DIFFERENT_DAY` to have anything to point at.

`duration_periods` is consecutive periods within one occurrence — D6 in `OPEN-DECISIONS`,
one activity rather than N linked ones, so that "this must not be split" is structural
rather than a rule that can be switched off. It is not a teaching load.

**The interface still asks two numbers**, because "four times a cycle, one period each" is
how a school thinks. The Occurrences panel takes *periods per lesson* and *times per cycle*,
states their product in words before the save, and writes `count` rows in one transaction.
That is a **creation**, so it carries no revision precondition (ADR 0027 §5); editing one
occurrence and deleting one occurrence are ordinary single-record writes carrying that row's
own revision. No new concurrency model was needed, which is the practical payoff of rows
over a count.

`split_index` is assigned from `MAX + 1` within the section and **never renumbered**.
Deleting the second of four leaves 1, 3, 4. Renumbering would rewrite two rows the user did
not touch and consume their revisions, which would make a concurrent edit to one of them
fail for no reason the user could see — so the stored positions are stable and the interface
numbers the rows it displays.

### 3. The course declares; the section's rows decide; a difference is advice

`course.periods_per_cycle` is the **declaration** — what one section of this course is
expected to need — and it is nullable, because "we have not decided yet" is a real state and
a form that insisted would be answered with a guess. `SUM(activity.duration_periods)` over a
section is the **actual** demand.

The two are allowed to differ, and `DeclaredComparison` reports the difference as `Short` or
`Over` with the figures. Nothing refuses it: a section may legitimately need less than its
course's default, which is what a small set or an accelerated group *is*. The activity
generator reads the declaration, refuses while it is null rather than inventing a number,
refuses a section that already has occurrences rather than silently doubling the demand, and
refuses a declaration that does not divide into the default duration rather than rounding it.

### 4. The groups are the scheduling population. The roster is an overlay

ADR 0006 is explicit: *"A school must be able to build a full timetable using only
`student_group.size`, without entering a single student."* Stage C must not quietly cost
that, so:

- **`class_group` is the audience.** A section with groups and no named individuals is
  **complete**, not half-finished, and nothing anywhere treats an empty roster as unfinished.
- **`class_enrolment` is a register**, and has no scheduling meaning in Milestone 1. It is a
  roster, a report, and the input Milestone 2's sectioning will derive.
- **The two are never added.** A section taught to a group of 28 that also names four
  children has an audience of **28**, because those four are among the 28. Adding them would
  double-count every student a school had bothered to name — a wrong answer that looks
  right, which is the worst kind.

`ClassSectionView` therefore carries `groupPopulation` and `enrolledStudents` as separate
fields under separate headings, and `RosteredStudent.coveredByGroup` answers "is this child
being counted twice?" per row rather than resolving it silently. A `false` there is not an
error; it means somebody is named whom no attending group includes, which is worth seeing.
`tests_lessons::a_group_size_and_a_roster_are_never_added_together` pins the figures apart.

A group with no declared size contributes its counted membership instead — `V10`'s own
words for the nullable column are *"NULL means membership is the truth; a number means there
are this many and I am not listing them"*.

**`class_enrolment` names the student, not the enrolment spell.** The alternative was
tempting: pointing at a `student_school_enrolment` row would make the school context
structural rather than checked. It is rejected because `V13` keeps unlimited `LEFT` history,
so a student may hold several spells at one school and a spell-keyed roster could list the
same child twice in one class. The school context is enforced instead, by a trigger pair
requiring a **non-`LEFT`** spell at the section's school — `PROSPECTIVE` included, because a
school sectioning next September's intake is doing precisely that. A departure does **not**
clear the roster: "she was in 9MAT-A" is history, and ADR 0018's rule is that it is kept.

### 5. What Stage C does not model, and why each is cheap later

`DATA-MODEL.md` §5 and §7 sketch more than `V15` builds. Each omission is a decision, and
each is one nullable `ALTER TABLE` or one new table away — the same judgement `V14` made
about `subject.colour`:

| Not built | Why, and what adding it costs |
| --- | --- |
| `activity.school_id`, `activity.kind` | Only an activity with **no** class section needs either — an assembly, a meeting, a yard duty — and Stage C does not model one. `class_section_id` is `NOT NULL` instead, which is what makes the chain in §1 sufficient. Widening it is an [ADR 0020](0020-in-place-table-rebuild.md) rebuild, which `V12` has done once already. |
| `activity_teacher`, `activity_group` | Stage C's activities inherit both from their class section. Two places to say who teaches a lesson is two places for them to disagree, and the per-occurrence override has no reader. |
| `activity_room_candidate`, `activity_resource`, `activity.preferred_room_id` | Room and resource **allocation** is Phase 6. A requirement is not an assignment, and `course.required_room_type_id` is the requirement. |
| `activity.is_locked` | A lock is about a *placed* timetable, which is Phase 8's. **Settled in M9A and never to be added here:** the column is `timetable_entry.is_locked`, because several timetables coexist for one `(cycle, term)` and a lock on the lesson would pin it in all of them at once — [ADR 0045](0045-a-lock-is-a-property-of-a-placement.md). |
| `class_teacher.role` | `PRIMARY`/`CO`/`ASSISTANT` with no reader, and the sketch's `(section, teacher, role)` key would let one person be a section's primary *and* its assistant at once — two contradictory rows rather than a staffing arrangement. |
| `class_section.max_size`, `class_enrolment.status` | ADR 0006 asks for the **shape** of `class_section` and the roster to exist before populated projects need migrating, and that is what `V15` delivers. These two columns have no reader in Milestone 1. |
| `course_request`, `line`, `line_member`, `activity_link` | Request-based sectioning is Milestone 2 (ADR 0006, D5); elective lines and activity relationships are later in Phase 3 and Phase 4. None is referenced by anything `V15` writes, so none is a migration risk. |
| `class_section.student_group_id` | The sketch has it *and* `class_group`. Two places to say who attends; the column is dropped and the many-to-many kept, which is what the brief and co-teaching both need. |

`V15` also **renames** the sketch's `enrolment` to `class_enrolment`. `V13` already made
"enrolment" mean *a spell at a school* throughout the crate — `Membership::enrol_student`,
`StudentEnrolment`, `EnrolmentStatus` — and a second table called `enrolment` meaning
something else is the kind of name that produces a wrong join years later.

### 6. Nothing claims a lesson can be scheduled

`evara_core::lessons` answers two questions about a demand and refuses a third.

- **Structurally impossible** is reported: an occurrence needing four consecutive periods
  where no day offers more than three in a row can never be placed, by any solver. That is
  arithmetic, and `DemandFinding` says it in those words — *"No timetable can place it."*
- **Merely demanding** is not reported. Twenty-nine periods in a thirty-period cycle is
  probably infeasible once clashes, rooms and staff availability are accounted for, and
  nothing here knows. `a_demand_that_is_merely_difficult_is_not_reported` pins the silence.
- **Feasible is never reported at all.** The absence of findings is shown as *"That is not a
  promise that it can be timetabled"*, because a screen that says "this will work" and is
  then contradicted by the solver has spent the user's trust on a guess.

Before the school has a cycle and a bell schedule, `CycleCapacity::is_known` is false and
**no finding is produced**. Lessons is section 10 of School Setup and the time model is
14–17 (ADR 0024), so entering lessons first is the expected path and must not be told it is
wrong.

This lives in `evara-core`, not `evara-constraints`. That crate is Phase 4's registry of
`type_key`-addressed evaluators with JSON Schemas and `explain()` implementations, each
scored against a candidate timetable (`CLAUDE.md` §4). Nothing here is evaluated against a
timetable: these are data-entry rules about whether the demand a school has just typed in is
coherent at all, which has an answer before any placement exists. Putting them in the
constraint registry would mean either five incomplete constraint types a stage early, or a
second kind of thing living in a crate whose whole value is holding one kind.

### 7. Deletion: a course refuses, a section cascades, and the asymmetry is the point

ADR 0010's rule is that ownership edges refuse. `V15` uses it twice in opposite directions,
and which an edge got was decided one at a time.

| Operation | Behaviour | Because |
| --- | --- | --- |
| `delete_subject` | **refused** while any course is an offering of it | a course with no subject teaches nothing |
| `delete_course` | **refused** while any class section delivers it, with counts | a section is a substantial thing a school built, with staff and a roster on it |
| `delete_class_section` | **cascades** its teachers, groups, roster and occurrences, with counts shown first | each of those is *about* the section and means nothing without it — `delete_teacher`'s treatment of qualifications, and ADR 0025 §5's reasoning |
| `delete_teacher` | **cascades** their class assignments | destroying the person is the one operation that may take them |
| `remove_teacher_from_school` | **refused** while they teach any of that school's sections | ADR 0028 §5 |
| `delete_student_group` | **refused** while any class section is taught to it | a group with members, a parent, a size and availability is substantial |
| `delete_student` | **refused** while they are on any class roster | ADR 0018: where the dependent row *is* the history, the delete is refused |
| `delete_year_level`, `delete_room_type` | **clear** `course.year_level_id` / `course.required_room_type_id` first, through `repo::update` | ADR 0010's optional-classification case, cleared by the repository so each affected course's `rev` stays truthful (R10) |

Every cascade goes through `repo::delete` inside the deleting transaction, so each removal is
a truthful change-log entry rather than a silent orphan — which a schema-level `CASCADE`
could not be, because it would not fire the trigger at all.
`every_cascaded_removal_appears_in_the_change_feed` asserts it.

The last row departs from `V14`'s treatment of `subject.department_id`, which is `RESTRICT`
despite looking like a classification. ADR 0025 §6 gave two grounds; only the first applies
here. R10's "a database-level `SET NULL` rewrites rows without incrementing `rev`" is
answered by the repository clearing them first, which is the existing mechanism for these two
exact columns. The second ground — that refusing is what a school *wants* from a department
still in use — does not carry over: a course outlives losing its year level perfectly well,
and the year level is optional on it.

### 8. The revision inventory, extended

ADR 0027 §3 classified all 84 mutation commands that existed when it was written, and said
the shape was *"the shape to extend, not a one-off"*. Stage C adds **16**, and every one is
classified here so that neither inventory has a hole.

| ADR 0027 category | Stage C's | Contract |
| --- | --- | --- |
| 1. Single-record update | `course`, `class_section`, `activity` | `expectedRev`, checked atomically |
| 2. Single-record delete | `course`, `class_section`, `activity` | `expectedRev`, checked atomically |
| 4. Relationship creation | `class_teacher`, `class_group`, `class_enrolment` | existence: duplicate on re-add |
| 5. Relationship removal | `class_teacher`, `class_group`, `class_enrolment` | `expectedRev` — each row is displayed in a list the user is acting on, which is §4's rule for when a relationship carries one |
| 8. Creation | `course`, `class_section`, `activity_add`, `activity_generate` | none: there is no previous version of a row that does not exist |

**Nine of the sixteen take `expectedRev`**, bringing the project to **100 mutations of which
53 carry a revision**. No command handler constructs an `Expected::Any`; the four uses of it
in `lessons.rs` are all ADR 0027 §4's first documented kind — cascades inside
`delete_class_section`'s own transaction, whose revisions the user never saw.

`setup_activity_add` is worth naming because it looks like an exception and is not. It writes
several rows in one transaction and still carries no precondition, because it is a
**creation** (§5): there is no previous version of rows that do not exist. The transaction is
for atomicity — a failure part-way leaves none of them rather than three — not for
concurrency.

`tests_lessons::every_stage_c_update_takes_a_revision` and
`::every_stage_c_removal_takes_a_revision` enumerate rather than sample, because the claim is
about coverage.

## Consequences

- A school can express what it teaches, to whom, by whom and how often, and nothing in that
  path requires a single student record.
- Phase 6's solver reads `activity` as its variable set with no expansion step, because the
  expansion happened at data entry where a user could see it.
- Phase 4's relationship constraints have individually addressable occurrences to point at.
- **The Lessons screen has three levels and says so.** A flatter screen was available and
  would have made the second section of a course inexpressible.
- **Stage D's imports have stable identities to match on**: `course.id`, `class_section.id`
  and `activity.id` are UUIDv7 and never renumbered, and `UNIQUE (subject_id, name)`,
  `UNIQUE (course_id, name)` plus the school-wide course-code trigger give an importer a
  natural key to look a row up by without display names becoming foreign keys.
- **17 of the 17 School Setup sections now render a screen.** The `comingLater` mechanism
  that announced this one went with the stage that made it unnecessary; a branch nothing
  takes is a branch nothing tests.
- **Not claimed**: that a demand entered here can be met. See §6, and the wording the screen
  actually uses.
- **Known limitation.** An occurrence cannot yet carry its own room requirement, its own
  teacher or its own name-level override of the section's staffing. A school needing the
  Thursday lesson in the laboratory and the other three in a classroom models it today by
  splitting the course. Each of those is a nullable column or a junction table away, listed
  in §5, and none is foreclosed.
- **Also not here**: elective lines, course requests, and any notion of a timetable. Those
  are the rest of Phase 3 and Milestone 2.
