# ADR 0028 — Assigning a lesson requires employment and eligibility, and *warns* about a missing qualification

- Status: **Accepted**
- Date: 2026-10-15
- Amends: [ADR 0025](0025-teacher-qualifications-across-school-owned-subjects.md) §4, whose
  rule for Stage C was a conjunction of three things. Two of them stand; the third becomes
  advisory. ADR 0025's **ownership** model is untouched.
- Asked for by: `ROADMAP.md`'s Phase 3 notes, which left this open on purpose — *"Whether an
  unqualified assignment is refused or merely warned about is Stage C's decision and should
  be an ADR, because a school mid-term with one absent specialist will want the warning."*

## Context

ADR 0025 built `teacher_qualification` as a property of the **person**: no `school_id`, no
membership requirement, and nothing short of deleting the person removes it. It then wrote
down what Stage C would have to implement:

> **Assigning a lesson requires the conjunction**: a qualification for the lesson's
> subject, an **active** `teacher_school` membership at the lesson's school, and campus
> eligibility where the lesson's campus is constrained. A qualification on its own is never
> authority to timetable somebody.

That sentence is right about what it denies — a qualification alone is not authority — and
wrong about what it requires. Stage C is the stage that had to find out which.

Three things forced the question:

**A school part-way through setup has incomplete qualification records.** Subjects arrived
one stage ago. A school entering its curriculum in the morning and its lessons in the
afternoon has qualified nobody for anything, and a rule requiring a qualification would
make the Lessons screen unusable until the Teachers screen had been worked through
subject by subject, person by person.

**Cover is ordinary and is not a data error.** A specialist is absent on Tuesday and the
head of department takes the class. A sixth-form teacher covers a Year 8 group for a term.
A newly qualified teacher starts before their paperwork arrives. Each of those is a real
staffing arrangement a timetable has to express, and in none of them is the school mistaken.

**A refusal is answered by falsifying the record.** This is the decisive argument. Given a
database that refuses to timetable an unqualified teacher, and a school that must timetable
one, what a school does is add the qualification. The field then says "she can teach
A-level Chemistry" when what the school meant was "she is covering 9C on Tuesdays", and the
one record this whole design rests on has been corrupted to get past a check. A rule that
is routinely worked around is worse than no rule, because it leaves the data looking
compliant.

## Decision

**Two of the three conditions are requirements, enforced by the schema and by the
repository. The qualification is reported and refuses nothing.**

| Condition | Effect | Where it is enforced |
| --- | --- | --- |
| An **active** `teacher_school` membership at the section's school | **refusal** | `V15`'s `class_teacher_requires_active_membership` trigger pair, and `require_assignable_teacher` |
| Campus eligibility where the section names a campus | **refusal** | `V15`'s `class_teacher_respects_campus_eligibility` trigger pair, and `campus_eligible` |
| A `teacher_qualification` for the course's subject | **warning** | `Lessons::assignable_teachers`, shown by the interface |

### 1. Why the membership requirement is *active*, where the other narrowings are not

`V13`'s `teacher_campus` and `V14`'s `teacher_department` both require a membership and
deliberately **not** an active one. Their headers say why: they are records of what was
true — which campuses she worked at, which departments he was in — and ADR 0018 is explicit
that such a record survives a deactivation.

A class assignment is not a record of the past. It is **new work**, and ADR 0018's own
sentence for an inactive person is that they "keep every assignment they ever had, and
simply stop being offered for new ones". `V15`'s trigger is the second half of that
sentence: it fires on an insert or an update of `class_teacher` and never on a change to
`teacher_school`, so deactivating somebody leaves every class they already teach exactly
where it is and makes a new one impossible.

Both halves are asserted together in
`tests_lessons::a_teacher_whose_membership_is_inactive_keeps_their_classes_and_gets_no_new_ones`,
because either on its own is the wrong behaviour.

### 2. Why campus eligibility is read as a narrowing

`teacher_campus` has no rows for most teachers, and ADR 0021 §3 calls it a *narrowing* of
membership. So the rule is three conditions in order, and the middle one is the subtlety:

1. the section names a campus — otherwise it is not campus-constrained;
2. the teacher has **some** eligibility recorded at this school — no rows means *not
   narrowed*, which is the state every teacher is in until somebody says otherwise;
3. and the section's campus is not among them.

Reading an empty set as "eligible nowhere" would make every teacher in a multi-campus
school unassignable until the eligibility table had been filled in, and would have broken
every project `V13` migrated. `tests_lessons::a_teacher_with_no_campus_eligibility_recorded_is_not_narrowed`
pins it.

The rule has a second direction the triggers cannot see. They fire on a write to
`class_teacher`, so a school could otherwise move a *section* to a campus its own staff are
not eligible at. `update_class_section` checks it, with counts, and
`a_section_cannot_be_moved_to_a_campus_that_would_strand_its_staff` asserts it.

### 3. What "warning" means concretely

Not a soft refusal, not a confirmation dialogue, and not a flag that has to be cleared.

- `Lessons::assignable_teachers` returns **every** member of the school with an
  `AssignmentEligibility` of three booleans — `employedAndActive`, `campusEligible`,
  `qualified` — and an `is_allowed()` that is the conjunction of the **first two only**.
  Keeping the three on a named type rather than loose on the candidate is what stops a
  screen treating the warning as a refusal.
- The picker labels an unqualified candidate "no qualification recorded" in the option
  itself, and shows a banner naming the subject as soon as the selection settles on one —
  **before** the save, so the user decides knowingly rather than being told what they have
  already done.
- The assignment saves. The teachers table then shows "Not recorded" against that row, with
  a title explaining that the assignment stands and where to record the qualification if it
  should be there.
- Nothing nags. There is no badge to dismiss and no count of unqualified assignments,
  because a school with a term of cover arrangements would be looking at a permanently
  unhappy screen about a situation it has already decided.

### 4. What is *not* weakened

ADR 0025's ownership model, in full. A qualification is still a property of the person,
still carries no `school_id`, still requires no membership and still survives a departure.
**Nothing in Stage C writes or removes one**:
`tests_lessons::a_qualification_is_untouched_by_everything_this_stage_does` assigns a
teacher, unassigns them, and asserts the qualification is exactly where it was.

Nor is the qualification made pointless by being advisory. It is read in three places that
matter:

- it orders and labels the teacher picker, so the right person is the easy choice;
- `Curriculum::subject_teachers` still answers "who can teach this?" on the Subjects
  screen;
- Phase 4's `teacher-qualified` constraint will read it as a **soft** rule with a weight,
  which is the honest home for a preference. That is also why this decision does not
  foreclose anything: a school that wants qualification enforced gets it as a constraint it
  can set to `HARD`, per school, rather than as a schema rule nobody can turn off.

### 5. Withdrawing a membership is refused while its classes remain

The other end of the membership requirement, and it departs from what `V13` and `V14` chose
for the two other narrowings. `remove_teacher_from_school` *clears* a teacher's campus
eligibility and department associations; it is **refused** while they still teach any of
that school's class sections, with the count in the message.

Campus eligibility and a department association are organisational labels that mean nothing
once the employment has gone. A class assignment is scheduling input a timetable will be
built on, and ADR 0018's rule applies: where the dependent row is real, the delete is
refused and the remedy is made visible. The remedy is also the one a school usually wants —
recording that somebody has left is `set_teacher_membership_active(false)`, which keeps
every class they have.

`V15`'s `teacher_school_delete_requires_no_class_assignment` makes that refusal a property
of the schema rather than of the repository.

## Consequences

- A school can enter its lessons before it has entered a single qualification, which is the
  order the setup workflow actually puts them in (Lessons is section 10; qualifications are
  recorded on Teachers, section 11).
- Cover, job-shares, trainees and incomplete records are all expressible without
  falsifying a qualification to get past a check.
- The qualification record stays meaningful, because nothing forces a school to write one
  it does not mean.
- **An unqualified assignment is possible and is visible.** It is not hidden, not silently
  accepted, and not reported as an error. A school that cares can see every one of them in
  the teachers panel of each section.
- **Not claimed**: that Evara knows who *should* teach a class. It knows who the school
  said may, who the school employs, and where they may work.
- **Known limitation.** There is no project-wide report of unqualified assignments. A head
  of timetabling who wants to audit them has to open each section. That belongs with Phase
  5's diagnostics, where the other "look over the whole project" questions live, and
  inventing a one-off list here would be the second place that knows what an unqualified
  assignment is.
- **Also not here**: a per-school setting that switches the warning to a refusal. It would
  be one boolean, and it is deliberately not added — Phase 4's constraint levels are the
  mechanism for exactly this, and a setting now would be a second one to reconcile with
  them later.

## Alternatives considered

**Refuse, as ADR 0025 §4 wrote it.** The strictest reading, and the one this amends. Its
real cost is argued above: it is routinely worked around by entering a qualification
somebody does not have, which destroys the record rather than protecting it. It also makes
the Lessons screen depend on a Teachers screen having been completed, which inverts the
order of the setup workflow ADR 0024 established.

**Refuse, with an override flag on the assignment.** `class_teacher.unqualified_override`,
set by a confirmation. Rejected on two grounds: the column records that somebody clicked
through a dialogue, which is not a fact about the school, and the flag would have to be
maintained — what happens to it when the qualification is added later? Nothing reads it, and
a column nothing reads with a lifecycle nobody has designed is worse than no column.

**Warn with no record of the warning.** What this decision does, and the alternative is to
also store "this assignment was made unqualified". Rejected: the state is *derivable* at
any moment by joining `class_teacher` to `teacher_qualification`, and a stored copy would
disagree with the join the first time a qualification was added or removed. Deriving it is
what `assignable_teachers` and `class_teachers` both do.

**Make it a Phase 4 constraint only, with no interface warning.** Technically clean — one
implementation, read by the scorer — and rejected because it is a year away from the user.
A screen that silently accepts an assignment and lets a solver complain about it in Phase 6
has made the school enter the data twice. The warning here and the constraint there are not
duplicates: one is about the record being incomplete now, the other is about the timetable
being worse for it.
