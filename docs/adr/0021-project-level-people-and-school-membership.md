# ADR 0021 — People belong to the project; membership belongs to the school

- Status: **Accepted**
- Date: 2026-10-14
- Amends: [ADR 0018](0018-people-identity-and-deactivation.md). Everything it says about
  identity surviving an edit and leaving being a state rather than a deletion stands; what
  changes is that "leaving" now has two scopes.
- Completes: the first item under *Not in this decision* in
  [ADR 0019](0019-multiple-schools-in-one-project.md).
- Related: [ADR 0022](0022-resources-shared-across-campuses.md) for the equipment half of
  the same stage; [ADR 0006](0006-group-based-scheduling-first.md), which this must not
  break.

## Context

`V12` let a project hold several schools, and said exactly what it had not done: *"a
teacher or student belonging to several schools. Both still have exactly one `school_id`.
Project-level identities with school membership and enrolment history — `teacher_school`,
`teacher_campus`, `student_school_enrolment` — are the next stage, and the interface must
not suggest they already exist."*

The arrangement the multi-school work exists for is a trust, a group, or a school with a
separate junior programme: several institutions that **share staff**. With one `school_id`
per person, the only way to express a shared teacher was two teacher rows — which gives one
human being two identities, two availability records and two independent timetables. That
is the correctness failure [ADR 0014](0014-rooms-shared-across-campuses.md) rejected for
rooms, where it observed that two room rows have two occupancy records and neither notices
the clash. A person is worse, because a person also has a contract, a load ceiling and an
"is she free at ten?" that must have one answer.

`V12` had already made the facility half work, because `building_campus` and `room_campus`
associate a facility with *campuses* and never mention a school. People had no equivalent.

## Decision

**A teacher and a student are each one row, for one real person, for the life of the
project. Where they belong is a relationship.** `V13` adds three relationship tables.

| Table | Shape | Because |
| --- | --- | --- |
| `teacher_school` | one row per (teacher, school), **deactivated** rather than duplicated | "Has she left?" is a yes/no about now |
| `teacher_campus` | one row per (teacher, campus), a **narrowing** of membership | "Junior campus only" is not expressible by membership alone |
| `student_school_enrolment` | one row per **spell**, history kept | "When did he join, leave, come back?" is what a school is asked |

All three are **full entity tables, not bare junctions** — own UUIDv7 `id`, own audit
columns, own change-log triggers, own `TRACKED_TABLES` entry. `V5`, `V6` and `V10` all give
the reason: the triggers read `NEW.id` and `NEW.rev`, and `repo::update`/`repo::delete`
address rows by `id`, so a composite-key table would sit outside both — and therefore
outside the change feed. Putting a teacher into a school is a real edit to a real project.

### 1. The home column survives, with a narrowed meaning

Exactly as `room.campus_id` survived `V6`:

- `teacher_school` is the **complete** set of schools a teacher belongs to. Every school
  has a row, the home school included. **Nothing reads `teacher.school_id` to answer "which
  schools?"** — `People::teachers` reads membership, and a listing built on the column would
  miss every shared teacher a school had been given.
- `teacher.school_id` is the **home** school: the one they were created under. Immutable, it
  always has a matching membership, its membership cannot be withdrawn, it still scopes
  `teacher_code_unique`, and it guarantees a teacher can never reach zero schools and
  vanish from every screen.

The same split applies to `student.school_id`, and `People::students` reads enrolment.

### 2. Two levels of "active", with distinct meanings

| Flag | Means |
| --- | --- |
| `teacher.is_active` | in the project at all — an archive flag |
| `teacher_school.is_active` | employed by **this** school |

**Effective eligibility is the conjunction**, computed in exactly one place
(`Membership::teaches_at`, and `TeacherInSchool::is_effectively_active` for a listing).
Nothing may read the project-level flag alone and conclude a teacher is available to a
school: that is the mistake the split exists to make impossible. A teacher who has left
Southmoor and stayed at Northgate is the ordinary case, and `teacher.is_active` cannot say
it.

`update_teacher` never touches a membership, and `set_teacher_membership_active` never
touches the person. A project-level edit and a school-scoped edit are different acts with
different authority, and conflating them is how a screen scoped to one school ends up
rewriting a shared identity.

The `V13` backfill reads `teacher.is_active` into the membership, and `create_teacher`
does the same, because before memberships existed that flag *was* the per-school answer —
there was only one school to be employed by. Reading it as `1` regardless would have
silently re-employed every leaver in every existing project.

### 3. Eligibility is a narrowing, enforced by the schema

Three models were available. `teacher_school` alone cannot express "School A, junior campus
only", which is the common part-time arrangement. `teacher_campus` alone cannot express
membership of a school before its campuses are configured, and has nowhere to hang
per-school employment state. So **both**, with the drift between them refused by the
database rather than by convention:

```sql
CREATE TRIGGER teacher_campus_requires_school_membership_insert ...
    SELECT RAISE(ABORT, 'a teacher''s campus must belong to a school they are a member of')
```

Triggers rather than repository checks alone, for the reason `V3`, `V6`, `V9` and `V12` all
give: the rule is stated where the storage is, so a future repository, the CLI, an import or
a hand-repair cannot write a row that breaks it. The repository checks first anyway, because
a trigger cannot name the remedy — and *"add this teacher to the school before giving them
one of its campuses"* is the only ordering mistake this rule actually produces.

The invariant is **not** that the membership be *active*. A teacher who has left keeps the
record of which campuses she worked at, the way an inactive teacher keeps every assignment
she ever had. Removing a membership outright is what clears eligibility, and
`remove_teacher_from_school` does it in the same transaction, through `repo::delete`, so
each removal is a truthful change-log entry. A `BEFORE DELETE` trigger refuses a membership
delete that left eligibility behind — refuses rather than cascades, because a silent cascade
would leave no evidence in the feed that those rows had ever existed.

### 4. `V12`'s teacher trigger pair widens rather than goes

`V12` wrote *"ADR 0021 will replace this with school membership, and this trigger goes when
that arrives."* It does not go. The guarantee is unchanged — a teacher's home campus must
not be some other institution's — and what changes is which institutions count as theirs:
any school she is a member of.

The home-school disjunct is kept and is **not** redundant. `create_teacher` writes the
teacher and the membership in one transaction, and the teacher must be first because the
membership references it; at the moment the `BEFORE INSERT` trigger runs there is no
membership yet. Dropping the disjunct would make it impossible to create a teacher with a
home campus at all.

### 5. An enrolment is a spell, and uniqueness binds the current one

Teacher membership is deactivated; enrolment is **history**. A spell at a school is the
thing a school is actually asked about, so a transfer writes a `LEFT` row and an `ENROLLED`
row and never edits one in place. Status is `ENROLLED`, `LEFT` or `PROSPECTIVE`.

```sql
CREATE UNIQUE INDEX student_enrolment_one_current
    ON student_school_enrolment (student_id, school_id) WHERE status <> 'LEFT';
```

One index, and every requirement holds at once: a second current spell at one school is
refused; `LEFT` rows are unconstrained, so a student may leave and rejoin any number of
times and every spell is kept; two schools are two keys, so concurrent enrolment falls out
free.

**The audit's proposal is rejected.** It suggested
`UNIQUE (student_id, school_id, COALESCE(enrolled_on, ''))`. `enrolled_on` is optional —
plenty of schools do not record the day somebody joined — so `COALESCE` would fold every
undated spell onto one key and permit exactly one of them. A school that does not record
joining dates could not re-enrol anybody, which is the common case made impossible by a
constraint meant to protect it. This is `CLAUDE.md`'s `COALESCE` rule read in the direction
it actually points, the same reading [ADR 0018](0018-people-identity-and-deactivation.md) §4
applied to `teacher.code`.

Admission numbers are **school-specific**, never project-wide: two schools may legitimately
number the same person differently and neither number is "the" one. The index is partial on
both counts — unique when given, and scoped to current spells, so a student who leaves keeps
their old number on the historical row and a school that reissues a retired number has not
done anything wrong. Constraining history would have refused both.

### 6. One placement, two representations, kept in step

`V13` makes the enrolment row authoritative for a student's school-specific placement, and
leaves `student.code`, `student.year_level_id` and `student.campus_id` in place as the
**home-school** values — the treatment `room.campus_id` got in ADR 0014.

Two writable copies of one fact drift, so they are **not independent**:

- `People::update_student` writes the student row and mirrors the placement onto the
  student's current spell at their **home** school, in the same transaction.
- `Membership::update_enrolment` writes the spell and mirrors back, under the same
  condition.

Both go through `repo::update`, so each row's `rev` and each change-log entry stay truthful.
Only the home school's current spell participates, and that is not a limitation — it is
forced: `V12`'s trigger holds `student.year_level_id` to belong to `student.school_id`, so
copying Southmoor's year level onto a Northgate-created student would be refused, rightly,
because it would be a claim about the wrong institution. A student who has transferred away
has no current home spell, and the columns become a record of where they last were.

`is_active` and `status` are deliberately **not** mirrored, for the reason §2 keeps
`teacher.is_active` and `teacher_school.is_active` apart.

### 7. Authority, and the one deliberate exception

Every mutation names the school it acts for and refuses an identifier belonging to another,
inside the same transaction as the write. That is `V12`'s rule, and a shared identity makes
it matter *more*: holding a shared teacher's identifier is now completely ordinary, and must
still not be authority to touch another school's employment of them.

`Membership::transfer_student` is the exception, and it is explicit about being one. A
transfer is **one fact about two institutions**. Splitting it into `leave_school` plus
`enrol_student` would be two transactions, and a failure between them leaves a real student
enrolled nowhere — the outcome atomicity exists to prevent. So the authority is made
explicit in the signature rather than implied by a single acting school: the caller names
both ends, and the Stage 0C interface must offer it only where the user has standing in
both.

### 8. Deleting a person is a third operation

Three things are kept distinct, the way ADR 0019 kept them distinct for a shared room:

| Operation | Who may | Effect |
| --- | --- | --- |
| Deactivate a membership / end a spell | that school | the person stays, keeps everything, stops being offered there |
| Remove a membership | that school (not the home school) | that school's relationship goes; the person and every other relationship stay |
| Delete the identity | the home school, **and only while nobody else holds a relationship** | the person, their relationships, their availability |

`delete_teacher` and `delete_student` therefore take the authority a shared facility takes:
only the home school, and `DbError::StillReferenced` while another school employs or enrols
them. The way forward is for that school to let go first — an operation that exists, and
that the other school can see. Both checks run **before** anything is cleared, because
clearing a shared teacher's availability and *then* refusing the delete would be the worst
of both outcomes.

`Structure::school_dependents` gained `shared_teachers` and `shared_students`, counting
only people another school created, so the total a confirmation reads never double-counts
the school's own.

### 9. A group's members must be enrolled in the group's school

A hole `V12` left and could not close. `student_group_member` carries no `school_id` of its
own — the group owns it — so `V12`'s audit of columns that could disagree had nothing to
list. The disagreement is between its two *ends*: nothing stopped Northgate's student being
put into Southmoor's cohort, and a cohort containing another institution's students is not
an audience anything can be timetabled for.

Stated against **enrolment** rather than `student.school_id`, which is what makes it correct
under sharing: a student enrolled in both schools may legitimately be in a group in either.
Any spell counts, past ones included, for the reason ADR 0018 gives throughout — membership
of 9-MAT-A last year is the only evidence it happened, and a student who has left must not
have their history refused.

**[ADR 0006](0006-group-based-scheduling-first.md) is untouched.** A school that timetables
with `student_group.size` and no student records at all writes no rows in this table, so
this rule has nothing to say to it, and a project with zero students still works.

## Consequences

- **Existing projects are unaffected in behaviour.** One membership per teacher, one spell
  per student, both backfilled from what the row already said, with the row's own
  `created_at` and no revision consumed. Every list returns what it returned before.
- **`V13` is additive.** Nothing is rebuilt, so `backup::FIRST_REBUILDING_MIGRATION` stays
  at `12`: a project at `V12` needs no safety copy to reach `V13`, and a project at `V11`
  still gets one because its route here crosses the rebuild.
- **Two new reads are the interface's job to use.** `Membership::school_teachers` and
  `Membership::school_students` carry the school's own view alongside the person. A screen
  that shows per-school status must read them; the plain `teachers`/`students` lists carry
  only the project-level flag, and showing that as "Active" on a school where she no longer
  works is the wrong answer this stage exists to prevent.
- **Availability is still one school's statement.** A rule names a school and that school's
  cycle. A shared teacher's rules are therefore recorded per school, which is correct as far
  as it goes and is not yet project-level availability. ADR 0019's *Not in this decision*
  still holds for that.
- **Cross-school scheduling is not implemented and is not implied.** Detecting a shared
  teacher double-booked across two schools needs dated occurrences rather than cycle
  positions, and is a soundness problem in its own right. Nothing here claims it works.

## Known limitation, recorded rather than hidden

**A project-wide teacher code is not introduced.** The audit recommended it (decision 4:
*"one person, one employee code"*), and this stage declines, keeping
`teacher_code_unique (school_id, code)`.

The blocker is that tightening it is unsafe in a migration. Collisions are possible — two
schools in one project may each hold a teacher coded `T1` — and SQLite has no conditional
DDL, so a `.sql` migration can only create the project-wide index unconditionally. If it
collided, the migration would abort, which is the right *half* of the behaviour: no
identifier is rewritten. But refinery's grouped transaction means the project then **cannot
be opened at all**, and the user cannot edit the colliding codes in a project that will not
open. A released migration is immutable, so the trap would be permanent. Not attempting is
better than refusing, when the attempt is not needed for this stage's objective.

The compatible alternative, and the better model: a staff number is the **employer's**
identifier, not the person's. A teacher shared between two schools of a trust may hold two
payroll numbers, one per school, and a single project-wide `teacher.code` cannot express
that at all. The forward path is a `code` on `teacher_school` alongside the existing
column — additive, no collision possible, and it says the true thing. Reserved for Stage B
with the rest of the staff model.

One narrow consequence is visible today and is tested rather than wished away: at a
student's **home** school, reissuing a retired admission number is refused, because
`student_code_unique (school_id, code)` from `V10` spans every student a school has ever
entered and the §6 mirror brings it to bear. The same operation at any other school
succeeds. `V10`'s index is standalone, so a later migration can relax it to current
students; this stage does not, because changing a shipped uniqueness rule is its own
decision and not one this stage needs.

## Alternatives considered

**Duplicate the person per school.** The status quo before `V13`, and the thing the stage
exists to prevent. One human being, two identities, two availability records, two
timetables, and no clash detection between them.

**Derive membership from `teacher_campus` alone.** Cheaper by one table. Rejected: a school
can be joined before its campuses are configured, and per-school employment state has
nowhere to live.

**Derive eligibility from membership alone.** Also cheaper. Rejected: "School A, junior
campus only" is the common part-time arrangement and would be inexpressible.

**Model teacher membership as history too, like enrolment.** Symmetric, and answers "when
did she leave?" Rejected as unasked-for: nothing in the product reads a teacher's
employment dates, `is_active` already carries the fact that matters, and a history table
whose history nothing reads is a table that will be wrong by the time something does. The
asymmetry is deliberate and documented rather than accidental.

**Make `student.year_level_id` read-only once enrolments exist.** Would remove the mirror
entirely. Rejected because the existing Students screen writes it, and ADR 0019's promise
that existing projects are unaffected in behaviour includes the screens. The mirror is the
cost of keeping that promise, and it is one transaction in one place.

**Let a transfer act for a single school.** Consistent with every other mutation. Rejected:
whichever school acted, the other's rows would be written without its say, and splitting the
operation risks a student enrolled nowhere. Naming both schools makes the contract honest
instead of hiding it.
