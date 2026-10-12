# 0017 — A cycle is weekly or custom, and holds no school's calendar

**Status:** Accepted
**Date:** 2026-10-12
**Supersedes:** nothing. Extends [0011](0011-stable-timeslot-identity.md) §6 and Q4, and
[0015](0015-school-working-days.md).

## Context

Until now every cycle was the same thing: a name, a declared `day_count`, and however many
`cycle_day` rows the user had got round to adding by hand. Two kinds of school were
therefore equally awkward to set up. One keeps an ordinary week and had to type "Monday"
through "Friday" into five forms. The other keeps a six-day rotation and had to type
"Day 1" through "Day 6" into six.

Both wanted something the model could supply. But the obvious shortcut — "a weekly cycle is
the five days this school works" — is the one that cannot be taken, and understanding why
is most of this decision.

A cycle is a **reusable definition**. A project describes one school today
(`school.only_one`), but the cycle it defines is already shared across campuses that keep
different bell schedules, and the `.evara` export exists so that definitions move between
projects. The moment a cycle stores "the five days this school works", two things become
true at once: the definition is a statement about one institution, and there is no record
of which institution. Open it somewhere with a Sunday-to-Thursday week and it is silently
wrong.

The same argument rules out everything else a cycle might be tempted to hold: a start date,
a starting rotation position ("term begins on a Day 3"), and a holiday policy ("does a snow
day consume a rotation slot?"). Each is a fact about one school's year, each differs between
two schools sharing one rotation, and each has an existing home — `calendar_day`, which ADR
0011 made the only bridge from real dates to the scheduling model.

## Decision

### 1. One column, `cycle.kind`, with two values

`WEEKLY` or `CUSTOM`, added by V9 as `ALTER TABLE cycle ADD COLUMN`.

The two kinds are the same entity. They have the same days, the same materialised grid, the
same calendar mapping and the same deletion semantics. What differs is **who decides what
the positions are** — the calendar, or the user — and that is one question, so it is one
discriminator rather than two tables, a subtype, or a scattering of booleans.

### 2. A weekly cycle is the whole week: seven positions, always

Position *N* is ISO weekday *N*. Monday is 1, Sunday is 7. `evara_core::calendar::WeeklyPosition`
publishes the mapping and the canonical labels, and the repository writes `cycle_day` rows
from it, setting `weekday_hint` to match.

Seven, not five, and this is the load-bearing choice. **Which of the seven a particular
school teaches on is a question, not a stored value.** `TimeModel::cycle_shape` takes a
`WorkingDays` as an *argument* and compares; it writes nothing. Ask about Monday-to-Friday
and five positions come back marked `applies`; ask about Sunday-to-Thursday and a different
five do; the cycle is byte-for-byte unchanged by either. That is what makes a weekly cycle
shareable, and it is why the working week travels with the question rather than being looked
up — a repository that read "the school" could not be shared, and no test could prove it was.

This follows ADR 0015 exactly, and for the same reason: a working day is a calendar fact
about an institution, never a scheduling identity, and `non_working_instructional_days`
already reports against a week handed in rather than one it fetched.

### 3. A weekly position is named by the calendar, and still schedules by its ordinal

`CLAUDE.md` forbids anything in scheduling from reading a weekday, and that is unchanged.
`ordinal` remains a day's whole scheduling identity: materialisation, the grid, ordering and
the solver read nothing else, and no real date acquires a cycle day from a weekday. What the
weekly kind adds is that for this kind of cycle the position-to-weekday mapping is *fixed and
published* rather than a free `weekday_hint` a user may set to anything — a statement about
what the labels mean, not a new scheduling input.

Two triggers make that a property of the file rather than a convention: a weekly cycle is
seven days long, and a weekly cycle's day carries the weekday of its position. On a custom
cycle `weekday_hint` stays exactly what it always was — optional, free, display-only.

### 4. Creating a cycle lays out its positions, in the same transaction

For `WEEKLY` this is an invariant, not a convenience. A weekly cycle whose days are not the
seven weekdays is not an unfinished weekly cycle, it is a false claim: every reader would
believe position 6 is Saturday when no such row exists. The only way to make that
unrepresentable is to write the cycle and its days together, which is why laying them out is
part of `create_cycle` rather than a second call a caller remembers to make.

For `CUSTOM` it is a convenience, and an honest one: declaring a six-day rotation *is* asking
for six days, and `Day 1` … `Day N` are labels the user may then rename. Raising `day_count`
later creates the positions it adds and touches nothing that exists.

ADR 0011 Q4 — a part-built cycle is legal — still holds. Removing a day leaves a gap, a
project written before V9 may have one, and `CycleCoverage` still reports it. What changed is
only that nobody has to *start* from empty.

### 5. A cycle's kind never changes

Refused by `update_cycle` with a message, and by a schema trigger behind it.

Becoming `WEEKLY` would require the days to be the seven weekdays, and the only route there
from a six-day rotation is deleting days — which `timeslot` and `calendar_day` hold with
`RESTRICT` precisely so that removing one is a deliberate, confirmed act, never a side effect
of an edit. Becoming `CUSTOM` would quietly drop the invariant the surviving rows were created
under. Either way the honest answer is a new cycle, and a refusal is better than a picker that
snaps back with no explanation.

### 6. Existing cycles are `CUSTOM`, and nothing about them is rewritten

`DEFAULT 'CUSTOM'` is the whole upgrade story, and it is deliberately the kind that *asserts
nothing*: a declared number of positions, each with a label the user chose, which is exactly
what every pre-V9 cycle already was. `WEEKLY` carries a real invariant that an existing
six-day rotation — or a five-day cycle whose days are called "Day A".."Day E" — satisfies none
of.

As in V7, `ALTER TABLE ADD COLUMN` with a constant default does not rewrite the stored rows, so
every cycle keeps its `id`, `rev`, `created_at`, `updated_at` and change-log history, and every
`cycle_day` keeps its `id`. The migration creates, deletes and renumbers nothing, consumes no
revision and appends nothing to the change feed. `tests_cycle_kinds::a_cycle_from_before_the_kind_existed_is_custom_and_untouched`
builds a V8 project by hand and asserts all of it.

### 7. Nothing school-specific is added, here or anywhere near here

No start date, no starting rotation position, no working-week selection, no holiday policy, and
no holiday management. Those belong to Calendar Mapping, scoped per school and per campus, and
`tests_cycle_kinds::the_cycle_table_holds_nothing_about_a_schools_calendar` names the forbidden
columns so a future one arrives as a failing test rather than as a quiet coupling.

## Alternatives rejected

**Store only the school's working days as the weekly cycle's positions.** The shortcut this
whole record exists to refuse. It makes the shared definition a statement about one
institution with no record of which, and it means changing a school's working days would have
to create and delete `cycle_day` rows — destroying the stable identities ADR 0011 protects and
stranding every timeslot built on them.

**A `starts_on` and `start_ordinal` on the cycle, so the rotation could be projected onto dates
automatically.** Tempting, and wrong at this layer: two schools sharing a six-day rotation
begin it on different dates, and what happens to the rotation across a closure is policy the
data model does not state. ADR 0011 settled that an explicit `calendar_day` row is the only
bridge; a guess here would be indistinguishable from a decision.

**Two tables, `weekly_cycle` and `custom_cycle`.** Everything downstream — `cycle_day`,
`timeslot`, `calendar_day`, the materialiser, the grid — would need both edges, and the one
thing that actually differs between the kinds is a handful of lines in `create_cycle`.

**A table-level `CHECK` for the weekly invariants.** `ALTER TABLE` cannot add one, and
rebuilding `cycle` is unavailable: it is a `STRICT` table with `RESTRICT` children and the
rebuild procedure needs `PRAGMA foreign_keys`, which is a no-op inside refinery's single
transaction. The same hazard ADR 0013 hit with `building.campus_id` and ADR 0015 §6 with
`cycle.week_count`. Triggers do the job, in the same style V4 already uses for `timeslot`.

**Let the interface decide that a weekly cycle is seven days.** It would be the second
implementation of a model rule in the layer `CLAUDE.md` forbids rules in.
`CycleKind::fixed_day_count` is the authority and `create_cycle` ignores whatever
`input.day_count` says for a weekly cycle. The form still sends 7, because the column is
`CHECK (day_count >= 1)` and `CycleInput` has no optional length — the same arrangement, and
the same reasoning, as the retired `week_count` field.

**Delete the trailing days when a custom cycle is shortened.** Refused instead, as before.
Deleting is what `RESTRICT` exists to prevent happening by accident, and a user who really
means it can remove the days first and find out what is in the way.

## Consequences

- A weekly cycle's grid has seven columns, including Saturday and Sunday. That is correct —
  the shared definition has seven positions — and harmless, because which dates get lessons
  is decided by `calendar_day` rows and a Monday-to-Friday school simply never maps a Saturday.
  Materialising only the applicable days was **not** done: `MaterialisationRequest.without_cycle_days`
  means "preview this day's removal" and is how orphans are discovered, so reusing it to mean
  "this school does not work Saturdays" would corrupt that. If weekend columns prove unwanted
  in practice, the right fix is a new, separate notion of an inapplicable day — not that field.
- `cycle.week_count` and `cycle.is_default` are still in the schema, still collected by nobody,
  still carried through an edit untouched. ADR 0015 §6 records why dropping them was deferred;
  V9 does not change that and adds no third such column.
- A weekly cycle's day *can* still be deleted. It has to be: `delete_cycle` is `RESTRICT`, so a
  cycle with days cannot be removed, and a weekly cycle whose days could never be removed could
  never be deleted at all. The interface does not offer adding, explains what removal costs, and
  `update_cycle` lays a missing weekday back — as a new row with a new identifier, which the
  screen says plainly.
- Labels are English. A weekly position's starting label is `"Monday"`, from `evara-core`, and the
  user may rename it. Localising a *stored* label would mean deciding what happens to it when the
  school's locale changes, which is a question nobody has asked yet.
