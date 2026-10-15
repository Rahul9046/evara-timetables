# ADR 0025 — A qualification is a property of the person, and names one school's subject

- Status: **Accepted**
- Date: 2026-10-15
- Completes: the `department`, `subject` and `teacher_qualification` item left open in
  [ROADMAP.md](../ROADMAP.md) Phase 2 and in [DATA-MODEL.md](../DATA-MODEL.md) §4, and
  settles **[OPEN 6]** of the [multi-school architecture audit](../multi-school-architecture-audit.md).
- Extends: [ADR 0021](0021-project-level-people-and-school-membership.md), which separated
  a person's identity from where they belong. This is the same separation applied to what
  they can teach.
- Related: [ADR 0026](0026-teacher-departments-are-a-narrowing-of-membership.md), the other
  half of `V14` and deliberately the opposite answer; [ADR 0010](0010-structure-deletion-semantics.md),
  from which §6 departs.

## Context

`V14` has to put a **project-level identity** and a **school-owned entity** on either end
of the same row. That is the one place in Evara's model where the two scopes genuinely
collide, and the audit flagged it rather than deciding it:

> **Is `teacher_qualification` project-level or per school?** Project-level — a
> qualification is a property of the person. But `subject` is school-owned, so the join is
> per (teacher, subject-of-a-school). Flagging because it is the one place the two scopes
> genuinely collide.

Three facts constrain the answer, and all three are already settled:

1. **A teacher is one row for one real person**, for the life of the project
   (ADR 0021). Two rows for one human being gives them two identities, two availability
   records and two independent timetables — the correctness failure ADR 0014 rejected for
   rooms.
2. **A subject is school-owned.** Northgate's Mathematics and Southmoor's Mathematics are
   different curricula, belonging to different institutions, and from Stage C they carry
   different courses. There is no project-wide subject catalogue, and ADR 0023 §7 already
   refused to add a level of ownership above or below the school.
3. **A school's screens are scoped to one school** (ADR 0023), so whatever this model is,
   the interface has to be able to show *this* school's part of it without hiding the rest.

## Decision

**`teacher_qualification (id, teacher_id, subject_id)` — project-level identity on one
end, a school-owned subject on the other, and no `school_id` of its own.**

```sql
CREATE TABLE teacher_qualification (
    id         TEXT    PRIMARY KEY NOT NULL,
    teacher_id TEXT    NOT NULL REFERENCES teacher (id) ON DELETE RESTRICT,
    subject_id TEXT    NOT NULL REFERENCES subject (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (teacher_id, subject_id)
) STRICT;
```

A first-class entity rather than a bare junction, for the reason `V5`, `V6`, `V10` and `V13`
all give: the change-log triggers read `NEW.id` and `NEW.rev`, and `repo::update` /
`repo::delete` address rows by `id`, so a composite-key table would sit outside the change
feed. Qualifying a teacher is a real edit to a real project and belongs in the feed.

### 1. The subject supplies the school

A teacher shared between two schools who teaches Mathematics at both holds **two**
qualifications, one per subject record, and remains **one** teacher. That looks like
redundancy and is not: the two subjects are different curricula owned by different
institutions, and a school may perfectly well qualify somebody for its Mathematics and not
for its Further Mathematics.

There is deliberately **no `school_id` column**. It would be a second place to record what
`subject.school_id` already says, which is `CLAUDE.md`'s rule against resolving a child's
school any way but through its parent chain — and a second place for the same fact is a
second place for it to be wrong.

### 2. A project-wide subject catalogue was rejected

The alternative shape is a project-level `subject`, qualified once, with each school opting
into the subjects it teaches. It was rejected for three reasons:

| | Why not |
| --- | --- |
| It needs an owner above the school | ADR 0023 §7 settled that there is none. A project is a file, not an institution with a curriculum. |
| Two schools' "Mathematics" are not the same thing | Different departments, different codes, different room requirements, and from Stage C different courses and lessons. Merging them forces one school's answer on the other. |
| It makes the common case harder | A single-school project — most of them — would have to maintain a catalogue *and* an adoption list to say what it teaches. |

### 3. Authority comes from owning the subject

Every qualification write names an acting school, and the repository refuses one whose
subject belongs to a different school (`DbError::WrongSchool`). So a school may record and
withdraw qualifications against its **own** curriculum and nobody else's, for any teacher
in the project.

The interface mirrors that rather than restating it: a teacher's qualification list is
project-wide and grouped by the school that owns each subject, with another school's rows
shown, labelled, and not editable — ADR 0023 §5's rule, *visible, explained, not silently
modifiable*.

### 4. A qualification requires no membership, and creates none

Both halves are deliberate.

**It requires none** because tying competence to current employment would make it expire
when the employment did. A teacher who leaves Northgate does not become unqualified to
teach Northgate's Chemistry, and a school re-engaging her next September should not have to
re-enter what it already knew. The practical shape of that: a qualification survives a
deactivated membership, a withdrawn membership, an archived person, a campus change, a
department change and a rename. Each of those is asserted in
`tests_curriculum::nothing_but_an_explicit_removal_clears_a_qualification`, because "we
would never do that" is not a guarantee.

**It creates none** because qualifying somebody is not employing them. A call that quietly
did both would make the Teachers screen's careful distinction between identity, membership
and eligibility meaningless.

What follows is the rule Stage C must implement, and recording it here is the point of
saying any of this now:

> **Assigning a lesson requires the conjunction**: a qualification for the lesson's
> subject, an **active** `teacher_school` membership at the lesson's school, and campus
> eligibility where the lesson's campus is constrained. A qualification on its own is
> never authority to timetable somebody.

### 5. Removal is always explicit

`unqualify_teacher` is the only operation that removes a qualification short of deleting
the person. Deactivating a teacher, withdrawing a school membership, changing campus
eligibility, editing a name and changing a department association all leave every
qualification exactly where it was.

Deleting the **person** does take them, and that is the one correct exception: a
qualification with nobody to hold it is not history, it is a dangling fact. It goes through
`repo::delete` inside `delete_teacher`'s transaction, so each removal is a truthful
change-log entry rather than a silent orphan.

### 6. Deleting a subject somebody is qualified for is refused

Not `SET NULL`, and not a cascade. A school clearing out a subject it no longer offers must
not take with it the record that four of its staff were qualified to teach it — that is the
same judgement ADR 0018 made about `delete_student` and group membership: *where the
dependent row **is** the history, the delete is refused and the remedy is made visible
rather than performed.*

`Curriculum::subject_dependents` reports the count, so the confirmation explains before the
user presses anything (ADR 0023 §4). The remedy is to remove the qualifications — each one
explicitly — and the delete is then offered.

**The same reasoning extends to `subject.department_id`, and there it departs from
ADR 0010.** That rule says optional classification references are `SET NULL`, cleared by the
repository with a proper update first; `subject.department_id` is shaped exactly like one
and is nevertheless `RESTRICT`, with `delete_department` refused while any subject or
teacher refers to it. Two reasons:

- **R10 in [OPEN-DECISIONS](../OPEN-DECISIONS.md)**: a database-level `SET NULL` rewrites
  rows without incrementing `rev`, which silently bypasses the revision contract that
  [ADR 0027](0027-revisions-are-checked-on-write.md) installs in the very same stage.
  `RESTRICT` cannot do that; it refuses.
- **It is what a school wants.** Deleting a department that still has subjects and staff in
  it is almost always the wrong row; the useful answer names what is in the way.
  `Curriculum::department_dependents` provides the counts.

Nothing is lost by the stricter edge: clearing a subject's department is an ordinary edit
the user can make on the Subjects screen, through a real update, so each affected row's
revision stays truthful.

## Consequences

- One real person is one `teacher` row, however many schools and however many subjects.
  `tests_curriculum::one_teacher_may_be_qualified_at_two_schools_and_stays_one_teacher`
  asserts both halves.
- A school's curriculum is wholly its own: `UNIQUE (school_id, name)` and a partial unique
  index on `(school_id, code)`, so two schools may hold "Mathematics (MAT)" as two records
  and a duplicate *within* one school is refused.
- Qualification history outlives employment. A school that loses a teacher and re-engages
  them keeps everything it recorded.
- Stage C has a settled rule to implement, written above, rather than a design question to
  reopen.
- **Known limitation.** A qualification carries no level, no certification expiry and no
  grading-scale scope. A school distinguishing "can teach GCSE Chemistry" from "can teach
  A-level Chemistry" models it as two subjects today. Adding a `level` would be a nullable
  `ALTER TABLE` — the cheap direction — and nothing here forecloses it; inventing it now
  would ship a column no screen can explain and no constraint reads.
- **Also not here**, and not implied: a project-wide subject catalogue (rejected above),
  qualification approval workflow (Evara has no accounts — ADR 0001), and cross-school
  scheduling, which still needs dated occurrences rather than cycle positions.
