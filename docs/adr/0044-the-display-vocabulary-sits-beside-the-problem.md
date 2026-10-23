# ADR 0044 — The display vocabulary sits beside the problem, not inside it

- Status: **Accepted** (2026-10-23)
- Date: 2026-10-23
- Decides: M8 architecture proposal D2
- Builds on: [ADR 0037](0037-stored-row-diagnostics-belong-to-the-loader.md) §2 (`Problem` is
  not widened to carry rejected rows),
  [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) D2 (span
  resolution moves to `evara-core` because two crates need it),
  [ADR 0029](0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §1 (the
  `subject → course → class_section → activity` chain),
  [ADR 0006](0006-group-based-scheduling-first.md) (a timetable is buildable with no student
  records)
- Binds: `evara-core::scheduling::display`, `evara_db::scheduling::labels`, `evara-report`

## Context

A timetable grid has to say what is in a cell. The compiled `Problem` carries the names a
*rule* needs in order to explain itself — a teacher, a room, a group, a section — and nothing
more, because nothing more is an input to a scheduling decision.

In particular `SectionFact.name` is `class_section.name` **alone**
(`evara-db/src/scheduling/lessons.rs`), and `class_section` is unique on `(course_id, name)`,
so a school whose sections are named `A`, `B`, `C` produces a grid of cells reading `B`. The
`Problem` holds no subject, no course, no year level, no building and no student.

So M8 needed four names the solver does not: subject, course, year level, building — and, for
the per-student view, a student's own name and their roster.

The obvious move is to widen `Problem`. ADR 0037 §2 refused that move for a different
consumer, in terms that transfer exactly:

> `Problem` is *"everything a solve needs, and nothing else"*, and a list of things the solve
> must ignore is the opposite of that. It would also be the first field whose only consumer is
> a diagnostic, which invites more of them.

A field whose only consumer is a **report** is the same field.

## Decision

**The display vocabulary is its own type, in `evara-core`, loaded on request.**

### 1. The type lives in `evara-core::scheduling::display`

`LabelSet`, `SectionLabel`, `RoomLabel`, `StudentLabel`. Pure data, no I/O, read by no rule.

It is in `evara-core` because **two crates have to agree on it and neither can see the
other**:

| Crate | Role |
| --- | --- |
| `evara-db` | **populates** it from SQL — the only crate that may |
| `evara-report` | **consumes** it, joining it to the problem to project a grid |

`evara-db -> core` and `evara-report -> core, constraints` (ADR 0043), so the shared
vocabulary has to sit underneath both. This is the same argument `ARCHITECTURE.md` §3.2 makes
for `StoredConstraint` and ADR 0042 D2 makes for `span`.

### 2. It is **not** on `Problem`, and **not** on `Assembly` either

Not on `Problem`, for ADR 0037 §2's reason above.

Not on `Assembly` either, which was the closer call — `Assembly` already carries `rules` and
`findings` beside the problem for exactly this sort of reason. Rejected because every
`Assembly` is built by every solve, and a solve reads no subject name: carrying the labels
there would make the loader's statement count per load grow for a consumer the solver never
has. `SCHEDULING-PROBLEM-ASSEMBLY.md` §18 publishes that count as a figure worth keeping.

So `Scheduling::labels(&selection)` is a **second, opt-in read**, called by the viewing paths
and by nothing else.

### 3. A label is never a scheduling input, and every lookup is fallible

Nothing in `display` is read by a constraint, a solver or a diagnostic. Every lookup returns
`Option`, and a missing label renders as absent rather than as a substituted default: a grid
that invented a name would be showing a fact nobody entered.

`SectionLabel::short` — the caption a cell shows — is composed **once**, by
`SectionLabel::compose_short`, so a cell, a row header and a findings entry cannot disagree
about what a lesson is called.

### 4. A student reaches a `LabelSet` only through `class_enrolment`

`StudentLabel::sections` is exactly what `class_enrolment` says, and a student no enrolment
row names is **absent from the set entirely**.

ADR 0006 requires a full timetable to be buildable from `student_group.size` with no student
records at all, and `SCHEDULING-ARCHITECTURE-DECISIONS.md` §7 defers *"individually enrolled
students as a scheduling population"*. So an absent roster is an **ordinary state**, and the
remedy is to enter one rather than to infer one.

Nothing in the label read touches `student_group_member`, `student_group.size` or
`class_group`. A group's timetable is the group's; presenting it as a named child's confirmed
schedule would be a claim nobody made, and it is the one failure a per-student view can
produce that a parent would act on.

`LabelSet::has_enrolments` exists so a screen can say *"no roster has been entered"* rather
than offering a picker of blank timetables.

## Consequences

**Good.**

- `Problem` stays the compiled form of what a solve needs. ADR 0037 §2's line holds, and the
  precedent it set is now cited rather than quietly broken.
- A solve pays nothing. The labels are read only when a grid is drawn.
- `evara-report` keeps its *"receives data; does not query for it"* contract, and the only
  crate that touches SQL still is `evara-db`.
- The per-student view is honest by construction: the type cannot express an inferred roster,
  because the only field it has is the list of rows that exist.

**Bad, and accepted.**

- A viewing path makes two reads where one would do, and the second is a second chance for
  the two to be about different scopes. Both take the same `SchedulingSelection`, which is
  what keeps them aligned; nothing enforces that a caller passes the same one.
- `LabelSet` is a flat list and its lookups are linear. `section_index` and `room_index` build
  `BTreeMap`s for the projection's repeated use; a caller doing one lookup pays a scan. At
  school sizes this is irrelevant, and it is `BTreeMap` rather than `HashMap` because every
  projection output has to be deterministic.

**Neutral.**

- `evara-core` gains a module with no scheduling meaning. Its header says so in the first
  sentence, so a reader looking for the vocabulary a rule reads does not find this by mistake.
