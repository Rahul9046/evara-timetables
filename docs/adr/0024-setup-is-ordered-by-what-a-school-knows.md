# ADR 0024 — School Setup is ordered by what a school knows, not by what depends on what

- Status: **Accepted**
- Date: 2026-10-14
- Reverses: the ordering rationale recorded in `SchoolSetup.tsx` since Phase 1E —
  *"People come after the grid because nothing about them depends on it… Entering staff
  before that is work done on an unverified foundation."* That reasoning is restated and
  rejected below.
- Related: [ADR 0023](0023-the-interface-scopes-to-one-school.md), which this builds on
  without changing; [ADR 0006](0006-group-based-scheduling-first.md), which decides that a
  school with no students at all is a first-class case.
- Implements: Stage A of `docs/multi-school-architecture-audit.md` §15.

## Context

School Setup has been ordered by dependency since it was built. Rooms need a campus, terms
need a year, the grid needs a cycle and a bell schedule, so each section came after the
things it references — and people came **last**, after the grid, because nothing about a
teacher or a student depends on the time model at all.

That order is defensible and it is also the order nobody follows. A school setting itself
up has a staff list and a roll before it has an opinion about whether its rotation is a
week or ten days. Being asked for a six-day cycle before being asked for a year level is
being asked the abstract question first — and the sections that came late were the ones a
school could actually answer immediately.

Three smaller problems came from the same place:

1. **Year levels lived on the Students screen**, on the reasoning that School Setup pairs
   an entity with the small table that classifies it — rooms with room types, cycles with
   cycle days. Year levels are not that kind of thing, and the next two stages make it
   obvious: Stage C's lessons are taught *to* year levels, and Stage D's Excel templates
   reference them as a sheet of their own.
2. **The teacher form asked for four workload numbers** — full-time equivalent and three
   load ceilings — on the first screen a school meets its staff on. Nothing reads them
   until Phase 4's load constraints exist.
3. **The workflow looked shorter than it is.** Lessons is the single largest thing still to
   come and nothing on screen said so, which makes the order of everything around it
   read as final when it is not.

## Decision

**The sections are ordered by what a school knows, then by when it happens.**

| | Sections | What the school is being asked |
| --- | --- | --- |
| 1–2 | Schools, School details | which institution |
| 3–5 | Campuses, Rooms, Resources | where it is, and what is in it |
| 6–7 | Academic years and terms, Year levels | the shape of a school year |
| 8 | Lessons | *Stage C* — what is taught |
| 9–11 | Teachers, Students, Student groups | who |
| 12–15 | Cycles, Bell schedules, Calendar, Grid preview | when |

### 1. The dependency order was answering the wrong question

Dependency order is the right answer to *"what will fail if I do this now?"*. Almost
nothing in School Setup fails: the sections that genuinely require a prerequisite already
say so and refuse (`CrudSpec.blockedBy` — a room needs a campus, a term needs a year), and
those are all *within* a group that stays in dependency order. Across groups there is
nothing to protect.

So ordering the whole workflow by dependency bought a guarantee that was already held
locally, and paid for it with a sequence that asks for the hardest decisions first. The new
order asks what a school can answer, in the order it can answer it, and leaves the local
refusals exactly where they were.

### 2. Availability now precedes its prerequisite, and says so

The one real inversion. Availability is set against a cycle's days and a bell schedule's
periods, and the panel lives on Teachers, Rooms and Student groups — so it is now routinely
reached before either exists.

This was always a legal state: a rule is stored against `(cycle_day, period)` and can be
written before the grid has ever been materialised. What changes is that it stops being
unusual, so the empty state has to be good:

- it names what is missing — "No cycle yet";
- it **opens the section that fixes it**, rather than only naming it;
- and it says that **nothing is lost by coming back later**, because everything is
  available until a rule says otherwise, so a school that enters staff first has not
  created work for itself.

A dependency-ordered workflow would have hidden this at the cost of asking the abstract
question first. Stating it and navigating out of it is the better trade, and it is the only
place in the stage where an order was chosen against a dependency.

### 3. A year level is structure, not an attribute

It moves to section 7, beside Academic years and terms.

A room type exists *because* rooms needed classifying and has no life outside them. A year
level is part of the structure of a school year: it exists whether or not a single student
is entered — ADR 0006's whole promise is that a school can timetable from group sizes
alone, and such a school still has a Grade 8 — and two later stages reference it directly.

It stays **school-owned and never shared**, exactly as ADR 0019 settled: two schools in one
campus file each hold their own "Grade 1" at their own ordinal 1, and `UNIQUE (school_id,
name)` refuses a duplicate within one school. The split is presentational; the ownership
rule is untouched.

The cost is a dependency between two *screens* where there was one screen with two panels:
the student form offers year levels it can no longer create. The answer is an actionable
empty state — a note naming the section and a button that opens it — and deliberately
**not** a block, because a year level is optional on an enrolment and a school that does
not track years must be able to ignore the whole section.

### 4. Hidden fields keep their values, and the mechanism is the decision

The four workload fields leave the teacher form. Nothing else about them changes: the
columns, the record, `TeacherInput`, the generated types and the command signature are
untouched, and Phase 4's load constraints will read exactly what is stored today.

**The decision is that a hidden field is carried, not dropped.** `toDraft` reads every
stored value into the draft and `toInput` writes every one back, rendered or not, so an
edit sends the stored number. That is the whole mechanism and it is one line each — but it
is also the stage's only route to silent data loss, because the failure mode is a
*successful* save that has replaced a part-time contract with a full-time one. The
repository cannot catch it: what arrives is a valid record.

Two consequences follow, and both are deliberate:

- **The validator covers only what is on screen.** Validating a hidden numeric field would
  pin an error to a control the user cannot see or fix, which is an unsubmittable form with
  no visible reason. `validateTeacherIdentity` checks the two names; the `CHECK`
  constraints in `V10` remain the authority on the rest, as they always were.
  `validateTeacherWorkload` is kept and exported so the rules are not lost — a blank
  ceiling means *no ceiling* rather than zero — ready for the Phase 4 screen.
- **A new teacher gets the schema's defaults**, not the frontend's idea of them:
  `fte_percent DEFAULT 100` and three nullable ceilings, from `V10`. The interface matches
  the column rather than choosing a number.

FTE stays visible as a **table column**, because the value is real and a school that set
one must be able to see it; and a standing note says once that the values are kept and are
never changed by editing a teacher. A hidden field that is also a secret would be worse
than the field.

### 5. Home campus and campus eligibility stay different questions

`teacher.home_campus_id` is nullable and stays on the form, described as *where somebody is
normally based* — explicitly not the set of campuses they may work at, which is
`teacher_campus` and lives in the Stage 0C eligibility panel. Neither writes the other:
editing the teacher record never changes eligibility, and granting eligibility never
rewrites the home campus. Both directions are tested, because "it happens not to at the
moment" is not a contract.

### 6. Lessons is announced, not built

One navigation entry carries a `comingLater` reason, has **no `render`**, and is disabled.
It is in the list because the *order* is what this ADR is for: somebody configuring a
school should be able to see that lessons come after year levels and before teachers.

An **empty functional screen was rejected**: it would invite a user to look for a feature
that does not exist, and would need deleting rather than filling in when Stage C arrives.
Its blurb reads "Coming later" rather than "Needs a school", because conflating the two
reasons would promise that adding a school makes it appear.

### 7. Sections are addressed by identifier, never by position

Already true, and this stage is what proves it was worth doing: the list was reordered and
one section split in two, and every caller survived — `SetupContext.openSection`, the empty
states that navigate, the tests. An index would have made the reorder a silent behaviour
change in six places, because `openSection(11)` still resolves, just to the wrong screen.

The new section identifiers are `yearLevels` and `lessons`; every existing one is unchanged.

## Consequences

- **No migration, no new entity, and no Rust change.** The whole stage is the interface.
- **Nothing was lost in the split.** Every Stage 0C student capability — enrolment,
  departure, re-enrolment, history, transfer, concurrent enrolment, the visiting-student
  restrictions, group history — moved across unchanged, and the tests that covered them
  moved with it.
- **Existing projects are unaffected.** No stored value is read differently, written
  differently, or defaulted. A single-school campus file sees no selector, no school
  heading, and the same screens in a new order.
- **`rev` is still not checked on any write.** Unchanged by this stage and still the last
  item before Phase 2 is done.
- **The Phase 1E "dependency order" comment is gone** rather than left to contradict the
  code. This ADR is where that reasoning now lives, alongside the reason it was dropped.

## Alternatives considered

**Keep the dependency order.** The status quo, and the thing this reverses. Rejected
because the guarantee it bought was already held locally by the sections that need it,
while the cost — asking a school for its rotation before its year levels — was paid on
every first run.

**Reorder, but move the availability panels to the time-model sections.** Would remove the
one inversion. Rejected: availability is a property of a *person, room or group*, and it
belongs on the screen where that person is. Moving it would mean choosing a teacher twice,
in two different places, to answer one question about them.

**Leave year levels on the Students screen and merely reorder.** Cheaper. Rejected because
it leaves a school that timetables from group sizes — ADR 0006's case — unable to reach
year levels without visiting a screen about students it has deliberately not entered.

**Block student creation until a year level exists.** Would make the dependency explicit.
Rejected: the placement is optional in the model, so the block would enforce a rule the
database does not have, and it would stop a school that does not track years from entering
students at all.

**Delete the four workload columns instead of hiding the fields.** Simpler to reason about.
Rejected outright and not seriously considered: the values are real school data, the
constraints that read them are a known Phase 4 deliverable, and a released migration is
immutable.

**Drop the fields from `TeacherInput` so they cannot be sent wrongly.** Superficially
safer. Rejected: it would make the input a second, narrower definition of a teacher, which
is what [ADR 0012](0012-records-are-the-ipc-contract.md) exists to prevent — and the update
would then have to merge against the stored row somewhere, which is the same preservation
problem moved into Rust where the test is harder to write.

**A separate setup wizard for first-run ordering.** Would let the section list stay in
dependency order while the wizard taught the other one. Rejected: two navigation systems
for one workflow, and the one a user returns to would be the one that was not designed for
the task.
