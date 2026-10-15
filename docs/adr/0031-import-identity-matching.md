# ADR 0031 — A code matches, a name asks

- Status: **Accepted**
- Date: 2026-10-15
- Implements: `ROADMAP.md`'s Phase 3, Stage D — *Excel roster imports*.
- Builds on: [ADR 0018](0018-people-identity-and-deactivation.md) (people keep their
  identity), [ADR 0021](0021-project-level-people-and-school-membership.md) (people belong
  to the project, membership to the school), [ADR 0027](0027-revisions-are-checked-on-write.md)
  (a stale write is refused, never merged),
  [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) (one activity row
  is one occurrence).
- Companion: [ADR 0030](0030-an-import-is-a-reviewed-plan.md), which is where the engine
  lives and what it guarantees. This record is how a row finds the record it is about.

## Context

Every row of an imported file is a claim about a record, and the import has to decide which
record — or that there is not one yet. Get it wrong in one direction and a school ends up
with two of every teacher; get it wrong in the other and one teacher's details are written
over another's.

The hard part is that the two mistakes do not have symmetric costs. A duplicate is
visible, embarrassing and repairable: the school sees two Ada Lovelaces and deletes one.
A wrong merge is invisible and permanent: the data that was overwritten is gone, and
nothing in the project records that two people were ever involved.

So the policy is not "match as well as possible". It is **match only on evidence that is
an identifier, and ask about everything else.**

## Decision

### 1. A code is an identifier within its school. A name is not an identifier at all

Where a row carries a code, the planner resolves it **within the selected school** and
proposes a create or an update. Codes are what `teacher_code_unique (school_id, code)` and
its counterparts already make unique, which is exactly what makes them usable here.

Where a row carries only names, a name is evidence and nothing more. Two members of staff
can share a name; two children certainly can. A name match is never a match.

### 2. Matching is scoped to the selected school at every depth

`teachers_by_code` reads through `teacher_school`, so it finds the people this school
employs. A subject, course or section code is resolved within the school, through the
chain ADR 0029 established — so Southmoor's Mathematics is simply **not there** to be
found when importing into Northgate, and the row reads as "no such subject" rather than as
a hit. `a_course_resolving_a_subject_from_another_school_is_refused` asserts it.

Section codes are resolved **within their course**, not across the school: "A" is a section
of each of twenty courses, and resolving it school-wide would be ambiguous for every file
anybody actually has.
`a_section_code_is_resolved_within_its_course_and_not_across_the_school` asserts the right
one is used.

### 3. A record that disagrees with the file is a question, not a write

A row whose code matches an existing record, and whose mapped fields differ from it,
becomes `RowAction::Conflict` and **blocks Apply** until the user answers. The four answers
are offered equally, with no default and no primary styling: *use the file*, *keep what
Evara has*, *create a separate record* (where that is possible), *skip the row*.

Evara has no opinion about which is right, and making one look like the expected answer
would be an opinion. "Keep what Evara has" is a complete answer and leaves the record at
its current revision.

This is the brief's rule — *existing records must not be silently overwritten* — and it is
why the engine's update path goes through `settle_update` rather than straight to an
intent.

### 4. Reuse across schools is offered on the narrowest evidence, and never assumed

A trust shares staff, and ADR 0021 made that expressible: one `teacher` row, several
`teacher_school` memberships. So when a row's code matches nobody at this school, the
planner looks project-wide for the **narrowest possible match: the same code *and* the same
given name *and* the same family name.**

Even that is a `Conflict`. The message names the schools it found them at — "at another
school" is not something a user can act on, and the decision they are being asked for turns
on which school it is — and offers *employ the same person here* or *create a separate
identity*. Choosing the first adds a membership; the person stays one person, with one
identity and one timetable. `a_teacher_from_another_school_is_offered_rather_than_duplicated`
asserts both halves: one `teacher` row, two active memberships.

A shared code is strong evidence and not proof. Two schools in a trust can both number
their staff from `T-0001`, and the import says so rather than deciding.

### 5. Two rows claiming one code is the file's problem, reported by row number

`SeenKeys` claims each code as the plan is built. The second row to claim one is
`RowAction::Invalid`, and **the message names the row it collides with** — "row 2" meaning
the row the user can see when they open the file.

Last-one-wins was rejected outright. It makes the import's result depend on row order, in a
way nothing in the preview would show.

### 6. Employment and enrolment drift is reported, never applied

A file whose `isActive` column disagrees with a teacher's membership, or whose student
already has a current enrolment spell at **another** school, produces a warning or a
conflict — never a write to `teacher_school` or `student_school_enrolment`.

Changing a membership or moving a child between schools is a transaction with its own
workflow (ADR 0021 §7) and, for a student, a leaving date the spreadsheet does not carry. A
column cannot express it, so the import does not pretend to.
`a_students_enrolment_moving_school_is_reported_rather_than_applied` asserts the spell is
untouched.

### 7. An activity has no natural key, so it is matched by multiplicity

An `activity` row is one occurrence (ADR 0029) and carries nothing unique: four single
periods of one section are four identical rows. So an activities row is matched by
counting: *how many occurrences of this shape — this section, this duration, this label —
does the section already hold?*

- Held equals wanted → `Unchanged`. This is what makes re-importing the same file a no-op.
- Held fewer than wanted → `Create`, adding the difference, with a warning naming the
  arithmetic: "becomes 2 new records (4 of the 6 already exist)".
- Held **more** than wanted → `Conflict`. **An import never deletes a lesson.**

The last one is the important one. Deciding which two of six lessons to destroy — and
destroying their future placements with them — is not a decision a spreadsheet cell makes,
and `split_index` is never renumbered. `activities_are_counted_to_a_target_and_never_deleted`
asserts all three arms, including that the activity count is still six after the conflict.

### 8. A relationship either exists or it does not

`class_teacher`, `class_group`, `class_enrolment`, `teacher_department` and
`teacher_campus` have no fields beyond the pair, so a row is `Create` or `Unchanged` and
there is no update path. Re-importing a staffing file adds nothing.

ADR 0028's split is preserved exactly: an inactive membership or an ineligible campus
**refuses** the row, a missing qualification **warns** and the row imports. A warning must
not block, or a school would have to record every qualification before it could import a
single timetable.

### 9. A row with no code is imported, with a warning that says why it will not repeat

A names-only row that matches nothing is created, and carries a warning: *this row has no
staff code, so importing the same file again will create a second record rather than
recognising this one.*

Imported rather than refused, because a school's first file often has no codes and
refusing would make the feature unusable for them. Warned, because "importing twice is
safe" is a promise this screen makes, and it is not true of a row with no identifier. The
user is told which rows the promise does not cover, at the point where they can still add
a column.

## Consequences

- **Idempotency is a property of the mapping, not of the file.** Importing the same file
  twice is a no-op *for the fields that were mapped*. That is the honest statement, and it
  is why ADR 0030 §7 keeps unmapped and mapped-blank distinct.
- **A school that wants repeatable imports needs codes**, and is told so by every template
  (the code column is first) and by a per-row warning where one is missing. Evara does not
  invent them: a generated code would be an identifier the school's own system does not
  know, and the next export would not carry it.
- **Conflicts can cascade, so answering one re-previews the whole plan.** Choosing "create
  a separate record" claims a code the next row wanted, which changes that row's outcome.
  The interface therefore sends the decision and takes a fresh plan rather than patching
  its copy — which is also what keeps the fingerprint belonging to the plan the backend
  computed.
- **A name match on a student is a conflict even within one school**, which will annoy a
  school importing a file with no codes and two children called Grace Hopper. The
  alternative is merging two children's timetables, and there is no version of that which
  is acceptable.

## Alternatives rejected

- **Fuzzy name matching** (edit distance, phonetic keys, nickname tables). It converts a
  conflict a user would have answered correctly into a merge nobody reviewed, and the
  failure is silent. `ColumnMapping::suggest` is deliberately exact-after-normalisation for
  the same reason — it suggests a *column*, which a user can see and change, and even that
  never guesses.
- **Match on name plus year level, or name plus email.** A narrower conjunction is still a
  conjunction of non-identifiers, and the whole class of mistake is the same. Where a
  school has an identifier, the answer is to map it.
- **Automatically merge on an exact code-and-name match across schools.** Rejected by §4.
  This is the case *most* likely to be a genuine shared teacher, and still not certain
  enough — and the cost of asking is one click.
- **Let the file name the school** (a `School` column, say). Rejected: it would let one
  file write into institutions the user was not looking at, and "every import targets an
  explicitly selected school" is the property the whole multi-school design rests on.
- **Delete activities down to the file's count.** Rejected by §7. An import that can
  destroy lessons is an import nobody can safely run twice.
