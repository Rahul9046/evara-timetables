# ADR 0026 — A teacher's department is a narrowing of school membership, and is its own table

- Status: **Accepted**
- Date: 2026-10-15
- Supersedes: the `teacher.department_id` column reserved by
  [DATA-MODEL.md](../DATA-MODEL.md) §4 and by `V10`'s header, and the
  `teacher_school.department_id` column proposed in §6 of the
  [multi-school architecture audit](../multi-school-architecture-audit.md). Neither is
  implemented, and neither should be.
- Related: [ADR 0021](0021-project-level-people-and-school-membership.md) §4, whose
  `teacher_campus` this is modelled on down to the trigger names;
  [ADR 0025](0025-teacher-qualifications-across-school-owned-subjects.md), the other half
  of `V14` and deliberately the opposite answer.

## Context

`V10` reserved a column and said why it was safe to defer:

> `teacher.department_id` is the one column that implies, and adding it later is
> `ALTER TABLE ... ADD COLUMN` against a nullable reference — the cheap direction.

The column is still cheap. It is also **wrong**, and `V13` is what made it wrong.

A teacher is a project-level identity that may be employed by several schools. One column
on that identity has to hold one answer, and the arrangement `V13` exists to serve
routinely has two:

> Teacher T1 belongs to **Mathematics at School A** and to **Science at School B**.

A single `teacher.department_id` cannot say that. It forces either a second teacher record —
the duplication `V13` removed, and the correctness failure ADR 0014 rejected for rooms — or
an arbitrary choice of which school's answer survives.

The audit's own proposal, `teacher_school.department_id`, fixes the **scope** and not the
**cardinality**. Scoping the column to the membership makes "Mathematics at A, Science at B"
expressible, and leaves a second ordinary arrangement unsayable:

> Teacher T1 is in **both Mathematics and Physics** at School A.

That is not an edge case in a secondary school; it is how most science and maths faculties
are staffed.

## Decision

**`teacher_department (id, teacher_id, department_id)`, with the school reached through
`department.school_id`.**

```sql
CREATE TABLE teacher_department (
    id            TEXT    PRIMARY KEY NOT NULL,
    teacher_id    TEXT    NOT NULL REFERENCES teacher (id)    ON DELETE RESTRICT,
    department_id TEXT    NOT NULL REFERENCES department (id) ON DELETE RESTRICT,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    rev           INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (teacher_id, department_id)
) STRICT;
```

A first-class entity rather than a bare junction, for the reason every join table since `V5`
has been one: the change-log triggers read `NEW.id` and `NEW.rev`.

**Uniqueness is per `(teacher, department)`, not per `(teacher, school)`**, which is what
permits the Mathematics-and-Physics case. One association is the degenerate case of many,
never the other way round — and a school that wants exactly one per person can simply
record one. No schema rule is needed to express a convention, and one would make the
schema wrong for the schools that do not hold it.

No second `school_id` column, for the reason ADR 0025 gives about its own table:
`department.school_id` already says it, and `CLAUDE.md` requires a child's school to be
resolved through its parent chain.

### It is a narrowing of membership, modelled on `teacher_campus`

This is the half that is *not* like a qualification, and the distinction is the substance of
both ADRs:

| | What it says | Scope | Requires membership | Survives withdrawal |
| --- | --- | --- | --- | --- |
| Qualification | what this person can teach | the person | no | **yes** |
| Department | where they sit in one school's organisation | per school | **yes** | no |

A department association is a statement about one institution's internal arrangement. It is
meaningless away from that institution, so it is held to it exactly as `V13` held campus
eligibility:

- **A trigger pair** refuses a department of a school the teacher does not belong to, so the
  drift is rejected by the database rather than by convention — and therefore also for a
  future repository, the CLI, an import or a hand-repair.
- **The repository checks it first**, because a trigger cannot name the remedy and *"add
  this teacher to the school first"* is the one ordering mistake the rule actually produces.
- **Membership, not *active* membership.** A teacher who has left Southmoor keeps the record
  of which Southmoor departments she worked in, the way she keeps which campuses she worked
  at. Re-engaging her should not require re-entering it.
- **Removing the membership outright clears it**, in the same transaction, through
  `repo::delete` so each removal is a truthful change-log entry — and a third trigger
  refuses a membership removal that would leave one behind, which is what makes that path
  the only one.

A teacher's **qualifications are deliberately absent from every one of those rules.** They
are not a narrowing of membership and they do not go when it does; ADR 0025 §4 and §5.

### Authority is the department's school

`assign_teacher_department` and `remove_teacher_department` name an acting school and refuse
a department belonging to another (`DbError::WrongSchool`). So a school that *shares* a
teacher can see where she sits at the school that employs her too, and cannot change it:
one institution does not get to reorganise another.

## Consequences

- The two arrangements the rejected designs could not express both work, and are asserted:
  `a_teacher_may_belong_to_different_departments_at_two_schools` and
  `a_teacher_may_belong_to_two_departments_at_one_school`.
- `teacher.department_id` is **not added**, now or later. DATA-MODEL.md is amended rather
  than left to imply it.
- Phase 4's load constraints get a real faculty to balance across, per school, and can read
  several departments per person without a modelling change.
- Deleting a department is refused while anybody is in it, with
  `Curriculum::department_dependents` naming the count so the confirmation explains before
  it asks (ADR 0023 §4). Deleting the **person** takes their associations, like every other
  row that is only about them.
- **Known limitation.** There is no notion of a *head* of department, and no ordering among
  a teacher's departments — so "primary department", if a school ever needs one, is not
  expressible today. It would be a nullable flag or ordinal on this table, which is the
  cheap direction; adding it now would be a column no screen can explain and no constraint
  reads. A school needing the distinction records the one department that matters.
- **Also not here**: a department hierarchy (a faculty containing departments), department
  ownership of rooms or budgets, and anything that would make a department a third level of
  structural ownership — ADR 0023 §7 settled that there is none.
