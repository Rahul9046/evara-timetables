# ADR 0023 — The interface scopes to one school, chosen once

- Status: **Accepted**
- Date: 2026-10-14
- Completes: the school-management interface deferred by
  [ADR 0019](0019-multiple-schools-in-one-project.md) and listed as Stage 0C in
  `docs/multi-school-architecture-audit.md` §15.
- Related: [ADR 0021](0021-project-level-people-and-school-membership.md) and
  [ADR 0022](0022-resources-shared-across-campuses.md), whose capabilities this exposes;
  [ADR 0013](0013-buildings-shared-across-campuses.md) and
  [ADR 0014](0014-rooms-shared-across-campuses.md), whose cross-school sharing it makes
  reachable.

## Context

`V12` and `V13` made the model multi-school: several schools per campus file, people and
facilities shared between them, every read scoped and every write checked. The interface
caught up only as far as correctness required. Stage 0A shipped "one minimal school
selector… shown only when there is a choice to make", and Stage 0B's screens *disclosed*
that editing a visiting person would be refused and let the user send it anyway.

Three things were therefore true at the start of this stage, and all three are interface
problems rather than model ones:

1. **Capabilities the model has had since `V12` were unreachable.** `building_campus`,
   `room_campus` and `resource_campus` associate a facility with *campuses* and never
   mention a school — the whole point of ADR 0013/0014/0022 — but the share pickers read
   `listCampuses(school.id)`, so no screen could offer another school's campus. There was
   no Resources screen at all, and no way to create a second school except a repository
   call.
2. **The relationships had no editor.** `teacher_school`, `teacher_campus` and
   `student_school_enrolment` were readable and writable over IPC and reachable from
   nothing. The practical consequence is the one `V13` exists to prevent: with no way to
   *add* an existing teacher to a second school, a user's only option is to type them in
   again, and two teacher rows is one human being with two availability records and two
   timetables.
3. **Nothing protected a form from a school switch.** A section takes `school` as a prop
   and holds state that is *about* that school — an open edit form, a pending delete, a
   chosen teacher. Open a form, switch school, press Save: the sequence is ordinary and
   the result would be one school's values written under another's name.

## Decision

**The active school is chosen once, held by the setup shell, and every section is
unmounted when it changes.** Project-wide reads are composed in the view layer from the
scoped commands that already exist. The interface adds no state-management library, no
unscoped backend command, and no new rule of its own.

### 1. One answer to "which school", in one place

`SchoolSetup` owns `activeSchoolId` and hands the resolved `School` to each section. There
is no second source: a selector that scoped some screens and not others would be worse
than no selector at all, and this is the same reasoning that removed
`Structure::school()` from the repository.

Selection is keyed by **identifier**, never by name. A name is unique and would work right
up until somebody renames a school mid-session, at which point a name-keyed selection
points at nothing.

A project with one school sees no selector and no school heading, because there is no
choice to make and nothing to tell the data apart from. A project with several sees both:
the sidebar selector, and a line above the section body naming the school it is about.
That line is not decoration — "data from School A under School B's heading" is the failure
this stage exists to prevent, and the sidebar selector is the one thing a user scanning a
table is not looking at.

### 2. Switching schools remounts the sections

```tsx
<div key={school.id}>{active.render(school, context)}</div>
```

Deliberately a blunt instrument, and deliberately not the only layer:

| Layer | What it stops |
| --- | --- |
| The shell's `key` | the previous school's form, dialogue or selection surviving a switch |
| `CrudPanel`'s `reloadKey` effect | the same, for any caller that re-renders instead of remounting |
| `useList`/`CrudPanel` liveness flags | a slow reply landing after the switch and overwriting the new school's rows |
| `DbError::WrongSchool` | the write, if everything above somehow fails |

The alternative was for every section to remember to clear every piece of per-school
state. That is a rule that holds until the next section is written, and it is exactly the
kind of invariant this project pushes into one place rather than leaving to discipline.
The cost of remounting is a re-read the section was going to do anyway.

### 3. Project-wide reads are composed, not added

Three questions are genuinely project-level and the repository has no unscoped answer to
any of them: *every campus* (so a facility can be shared across schools), *every teacher*
and *every student* (so an existing person can be added to a second school).

They are composed in `setup/project.ts` and in the two people screens by fanning out the
scoped commands over the project's schools:

- `listCampuses` per school → every campus, each carrying its owning school;
- `listTeachers` per school → every teacher, deduplicated by identifier. Complete because
  every teacher has a home membership, which `create_teacher` writes in the same
  transaction as the person;
- `listStudents` per school → every student, for the same reason: `create_student` writes
  a home spell.

**A project-wide command was rejected.** It would be a second way to ask one question,
with the standing risk that a screen reaches for the unscoped one by accident — which is
the mistake ADR 0019 deleted `Structure::school()` to make impossible. A handful of extra
round trips over a local SQLite file is not a cost worth that risk. The fan-out is
**read-only**: no mutation anywhere in this stage is built on a project-wide identifier
without naming the school it acts for.

The same fan-out answers a second question for free, and this is why it is worth the
round trips rather than merely tolerable: *which other schools hold this person?* That is
what lets a delete be explained before it is offered, instead of refused after.

### 4. An operation the backend will certainly refuse is explained, not offered

`CrudSpec` gains `deleteBlockedBy` and `editBlockedBy`, and `ConfirmDialog` gains a
`blocked` mode with no destructive button. Six refusals are now ordinary rather than
exceptional, and all six are knowable before the action:

| Refusal | Read from |
| --- | --- |
| a school that still owns anything | `setup_school_dependents` |
| a person whose identity another school holds | `teacher.schoolId` / `student.schoolId` |
| a person another school still employs or enrols | the project-wide fan-out |
| a student still in a group | `listGroupsForStudent` |
| a facility or resource another school still shares | its campus associations |
| a facility or resource another school owns | its home campus's school |

A refusal that arrives after the user pressed Delete explains nothing, and the user has
already decided by then. This is `student_group_member_count`'s pattern applied to
everything multi-school made refusable.

**A disabled button was rejected for the delete case.** A disabled control cannot be
focused, so its `title` is unreachable by keyboard and by touch — and the explanation is
the entire value of the interaction. A dialogue that explains and offers only "Close" is
reachable by everybody. `editBlockedBy` *is* a disabled control, because a row's own
"Belongs to" column already carries the explanation beside it, and a form that opened only
to refuse every save would be worse than a button that does not open.

### 5. Deactivating, letting go, and deleting stay three separate actions

ADR 0021 §8 keeps them apart in the repository; the interface keeps them apart on screen,
in different controls with different confirmations:

| On screen | Operation |
| --- | --- |
| "Mark as left" / membership status | `set_teacher_membership_active`, `leave_school` |
| "Remove from this school" | `remove_teacher_from_school` |
| "Delete" in the people table | `delete_teacher`, `delete_student` |

Merging any two of them is how a school destroys a person it only meant to stop employing.
The home school's membership offers no removal at all, with the remedy named on the
control rather than left to a failed attempt.

### 6. Transfer and concurrent enrolment are different workflows

A **transfer** is reached from the student's enrolment panel at the school they are
leaving, and calls `transfer_student` — one operation naming both schools, because
splitting it into a leave and an enrol is two transactions and a failure between them
leaves a real student enrolled nowhere. **Concurrent enrolment** is reached from the other
school's Students screen, as "Enrol an existing student".

Two different places, two different buttons, and each says which it is and points at the
other. They are not a dropdown on one form: the two produce different history and the
difference is not recoverable by guessing what the user meant.

ADR 0021 §7 requires that a transfer be offered "only where the user has standing in
both". In a local-first application with no authentication (ADR 0001), standing is holding
the file: every school in the campus file is the user's. So the destination list is the
project's own schools, and the constraint is honoured by naming both ends explicitly in
the one call rather than by a permission check there is nobody to perform.

### 7. A school is the institution; no division below it

Audit decision 1, settled as the audit recommended. A `program` or `division` entity would
make every ownership question three-sided for a sub-division nothing has asked for. A
school's own campuses express its sites and `student_group` expresses its programmes;
a third level can be added later without redesign, and adding it now could not be undone
without one.

### 8. Resources return to School Setup

The section was removed when configuring a resource had no effect on anything the user
could see. `V13` changed that: ADR 0022 gave resources the campus associations buildings
and rooms already had, so lending a minibus to another school's campus is a decision with
a consequence, and it is unreachable without a screen. Scheduling *against* resources is
still Phase 4's, and the section says so.

## Consequences

- **A single-school campus file is unchanged.** No selector, no school heading, no
  ownership columns, no "add an existing person" button, and transfer disabled with the
  reason stated. Every one of those is suppressed by a count of schools, not by a flag.
- **School Setup is thirteen sections rather than eleven.** Schools first, because which
  institution the work is about is the first thing to settle; Resources with the other
  physical things.
- **The school form has one definition.** `setup/schoolForm.ts` is shared by the Schools
  section and School Details, which is what stops the newly exposed `code` field drifting
  between them.
- **Creation moved out of School Details.** It creates nothing now and says where to go;
  the first-run path is the Schools section's empty state.
- **`rev` is still not checked on any write.** Preserved and displayed, never sent as a
  precondition. Two windows can still overwrite each other and this stage does not pretend
  otherwise — a frontend-only guarantee would be worse than none, because it would be
  believed. The audit places `rev` checking in Stage B; see *Known limitations* in the
  roadmap.
- **Availability is still one school's statement**, and a shared teacher's rules are
  recorded per school. ADR 0021's deferral stands, and nothing in this stage implies
  otherwise.
- **Cross-school scheduling is neither implemented nor implied.** The screens make a
  shared teacher, room and resource visible as *one* record in two schools, which is the
  precondition for detecting a cross-school clash. It is not the detection.

## Alternatives considered

**A global state library for the active school.** Rejected: the shell already owns it, it
is one identifier, and a library would add a dependency to solve a problem `useState`
solves — while making "where is the school decided?" harder to answer, which is the one
question this ADR exists to have a single answer to.

**Per-section clearing of per-school state instead of remounting.** More surgical, and
wrong: it is a convention every future section has to remember, and the failure mode when
one forgets is a plausible wrong screen rather than a crash.

**Project-wide list commands.** Rejected in §3. The fan-out costs round trips; the command
costs an unscoped read that a screen can reach for by mistake for ever.

**Letting the user attempt a refused operation, with a warning first.** Stage 0B's
approach, and the thing this stage replaces. A warning the user can click through is a
screen that is right about the rule and useless about the remedy — and it leaves the user
to discover, from a failed save, what they should have done instead.

**A "copy configuration from" option when creating a school.** Tempting, and wrong more
often than right: two schools of a trust usually run different academic years, different
year levels and different rotations, which is the reason they are two schools rather than
two campuses of one. A new school is empty and its empty state says what to do first.

**One combined screen for a school and the project's schools.** Fewer sections. Rejected
because they are two different decisions — how many institutions this file describes, and
what this institution is called — and School Setup's whole shape is one screen per
decision.
