# ADR 0018 — People keep their identity; leaving is a state, not a deletion

- Status: **Accepted**
- Date: 2026-10-12

## Context

Phase 2 introduces the People group — teachers, students, year levels, student groups and
membership. [DATA-MODEL.md](../DATA-MODEL.md) §4 already specified the tables, and
[ADR 0006](0006-group-based-scheduling-first.md) already settled that students are optional
throughout Milestone 1. Three questions were left open, and all three are about what
happens to a person who stops being current.

A school's roster churns every year. Teachers leave and come back; students arrive
mid-term and leave before it ends. Meanwhile a timetable that has already been built names
those people, and the record that a student was ever in 9-MAT-A exists in exactly one
place: `student_group_member`.

So "remove this person" is ambiguous in a way that "remove this room" is not. It can mean
*I typed this by mistake* or *they have left*, and those want opposite outcomes.

## Decision

**1. `is_active` on `teacher` and `student`**, which the documented model does not have.

Employment and enrolment state, not a soft-delete flag. An inactive person keeps every row
they ever had; they are simply not offered for new work. Nothing in the repository filters
on it — `teachers()` and `students()` return everyone — because deciding who to *offer* is
the interface's business, and a repository that hid inactive people would also hide a
teacher a timetable already names, making that timetable look corrupt.

**2. Deleting a student who is in any group is refused.**

`DbError::StillReferenced`, with the interface reading `groups_for_student` *before* it
offers the action so the confirmation explains it rather than the refusal arriving after
the user has pressed Delete. Deactivation is offered as the thing they almost certainly
meant.

**3. Deleting a group removes its own membership rows**, after the interface has been told
how many via `student_group_member_count`. The students are untouched. A group that is
still a parent of other groups is refused outright.

The asymmetry between 2 and 3 is the point. A membership has no meaning without the group
on one end of it, so deleting the group takes it. A membership is the only evidence a
student was ever taught in a particular set, so deleting the student destroys history —
and that is the case where deactivation was what was wanted.

**4. `code` is optional**, where the documented model has it `NOT NULL`. Enforced by a
partial unique index, `WHERE code IS NOT NULL`, rather than by `COALESCE`.

This is a deliberate exception to `CLAUDE.md`'s rule that *"a nullable column in a
uniqueness rule needs `COALESCE`"*. That rule exists for the case where unset rows must
still collide with each other. Here the opposite is true: a school that does not number its
staff must be able to enter more than one teacher, and `COALESCE(code, '')` would collapse
every un-coded teacher onto one key and permit exactly one.

**5. `fte_percent INTEGER` rather than `fte REAL`.**

An integer percentage — 100 is full time. The Phase 4 load constraints will compare and sum
this value, and `CLAUDE.md` already requires integer arithmetic on the scoring path. A
`REAL` would also make `Teacher` non-`Eq`, which every other record in `evara-db` is, and
which the `entity!` macro derives.

**Extended by V11 to `availability.weight`**, which `DATA-MODEL.md` §6 also writes as
`REAL`, for the same three reasons and with the same consequence. The rule this establishes
is general: *a number the solver will sum or compare is stored as an integer in this
schema.* Anything later that reaches for `REAL` should be read against that.

**6. `student_group_member` is a first-class row** with its own UUIDv7 and audit columns,
where the documented model writes `PRIMARY KEY (student_group_id, student_id)`.

The same reason V5 gives for `building_campus`: the change-log triggers read `NEW.id` and
`NEW.rev`, and `repo::update` / `repo::delete` address rows by id, so a composite-key table
would sit outside both — and therefore outside the change feed. Putting a student into a
group is a real edit and belongs in the feed like any other.

## Consequences

- A school can retire a teacher without losing a single assignment, which is the behaviour
  that makes year-on-year reuse of a project file safe.
- Two reads exist purely so a confirmation can be truthful before it asks —
  `student_group_member_count` and `groups_for_student`. That is a deliberate round trip:
  the alternative is a dialogue that guesses, or a refusal the user meets only after
  committing to the action.
- `is_active` is one more column every future reader must remember is not a filter. The
  repository doc comment says so, and the UI tests assert that an inactive teacher is
  listed rather than hidden, so the mistake is a red test rather than a silent omission.
- The partial-index exception needs explaining every time someone reads the rule in
  `CLAUDE.md` and then this schema. The migration comment carries that explanation next to
  the index rather than leaving it here.
- `teacher.department_id` is absent until Curriculum arrives with `department`. Adding a
  nullable reference later is `ALTER TABLE ... ADD COLUMN`, the cheap direction; ADR 0006's
  "carry the columns early" argument is about structures a later milestone would have to
  migrate *through*, which this is not.

## Dependency on Phase 3, and what changes when it lands

Consequence 2 above says *"a membership is the only record that a student was ever taught in
a particular set."* **That is true today and stops being true when `enrolment` arrives**, so
it is a statement about the current phase rather than about the architecture.

The documented model keeps time out of the group tables deliberately. `student_group` is a
reusable *definition* classified by `year_level`, not by academic year — "Grade 8A" persists
while the students in it churn — and `student_group_member` carries no time dimension
because it is **current state**. Historical scoping lives one level up, on the entities that
already carry `term_id`:

| Question | Answered by |
| --- | --- |
| Which section ran in which term? | `class_section.term_id` |
| Who was in that section? | `enrolment (class_section_id, student_id)` |
| What was scheduled, and when? | `activity.term_id`, `timetable.term_id` |

So a student moving from Grade 8A and French in 2026–27 to Grade 9B and Computer Science in
2027–28 rewrites only current-state membership; the previous year survives as enrolment rows
against sections in that year's terms. Adding `academic_year_id` or `term_id` to
`student_group` or `student_group_member` would duplicate scoping that already exists one
level up, and force a new group row per year for audiences that genuinely persist. **It is
not proposed and should not be added.**

The gap is temporal, not structural: **`enrolment` does not exist yet.** Until Phase 3
lands, a year rollover performed by emptying last year's groups is unrecoverable. No
timetable history can be lost — timetables arrive in Phase 7 — but a school entering real
data now and rolling over before Phase 3 would lose the previous year's group composition.

Two things follow, and both are deliberate:

- The refusal to delete a student who is in any group **should be revisited when `enrolment`
  exists**, not inherited unexamined. At that point the membership stops being the only
  record and the rule is protecting less than it does now.
- Year-rollover tooling belongs with Phase 3, after `enrolment` can absorb the history it
  would otherwise discard.

## Alternatives considered

**No `is_active`; delete and re-enter.** Rejected: it destroys exactly the history that
makes a project file worth keeping between years, and schools would do it anyway because
the interface gave them no other way to say "not this year".

**Soft-delete everything with a `deleted_at`.** Rejected as a bigger idea than the problem.
It would put a filter on every read in the codebase and make "did you mean deleted or
inactive?" a question about every entity rather than about people.

**Cascade membership deletion from both ends.** Rejected for the student end. It is the one
delete that silently destroys information nothing else records, and SQLite does not fire
triggers for foreign-key actions unless `recursive_triggers` is on — so a cascade would
also remove the rows without a change-log entry, which is the quiet version of the same
failure.
