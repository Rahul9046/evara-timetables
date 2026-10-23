# Timetable Viewing and Reporting

What M8 projects, what it refuses to claim, and how a grid stays a well-formed table when the
inputs behind it have moved.

Shipped in **M8**. This is the first milestone in which anything in the interface generates
or displays a timetable.

[M8-VIEWING-PROPOSAL.md](M8-VIEWING-PROPOSAL.md) is the design record;
[ADR 0043](adr/0043-a-report-renders-a-rules-own-explanation.md) and
[ADR 0044](adr/0044-the-display-vocabulary-sits-beside-the-problem.md) are the two decisions
it earned. This file is the **behaviour**.

---

## 1. The shape of it

```
evara-core      vocabulary:  scheduling::display::LabelSet          <- new in M8
                             span::resolve_activity_span            (M7)
   ^        ^
   |        └── evara-report   projection::project, lay_out         <- the crate becomes real
   |            findings::explain_all                               -> core, constraints
   └── evara-db  scheduling::labels, Timetables::for_cycle
   ^
src-tauri       commands::solve   (generate, preview, save)
                commands::views   (list, open, rename, delete, revalidate)
                commands::session (the lifecycle state machine)
   ^
apps/desktop    app/timetables/   Generate · Timetables · TimetableGrid · Disclosure
```

**No migration.** `V17` holds everything M8 reads. M8 adds no table, no column, no trigger and
no index.

| Stage | Delivers |
| --- | --- |
| **8A** | generation off the main thread, progress, cancellation, preview, explicit save |
| **8B** | saved-timetable listing, opening, renaming, deleting, revalidation, and the grids |

---

## 2. One projection, two callers

`evara_report::projection::project` is called by the Generate screen's preview **and** by the
saved-timetable view. There is deliberately no second renderer, which is what makes *"what I
previewed is what I saved"* a property of the code rather than a hope.

It resolves nothing of its own. `span::resolve_activity_span` is the one implementation of
*"which slots does this occurrence occupy?"* (ADR 0042 D2), and the projection calls it once
per placed occurrence.

### What crosses, and what never does

| Crosses the IPC boundary | Never crosses |
| --- | --- |
| stored identifiers (`Uuid`) | any `*Ix` dense index |
| resolved spans, as `(day, period)` pairs | `Problem`, `Assignment`, `Placement` |
| `'HH:MM'` strings, as stored | a parsed date-time, or a timezone |
| a `rowSpan` the backend decided | a duration for the view to multiply |

A dense index has meaning only inside the one problem that assigned it, so a TypeScript value
holding one would invite arithmetic that is right for one load and wrong for the next.

---

## 3. A cell holds a list, and `rowSpan` is decided in Rust

This is the part with no analogue in earlier milestones, and the reason it exists is that
**the obvious implementation produces a broken table exactly when a school needs the screen
most.**

Two things break a naive grid, and both are reachable without any bug:

- **Two occurrences in one cell.** Ordinary in a master grid — that is what a working
  timetable looks like. A clash in a teacher's. Either way, two overlapping `rowSpan`s in one
  column produce malformed HTML and a lost lesson.
- **A run that is not contiguous in the axis.** If a slot inside the run was never
  materialised, the occupied periods can be 2 and 4, and no single `rowSpan` describes that.

So every `(day, period)` of every panel gets exactly one `TimetableCell`, and the content is
tagged:

| `CellContent` | `rowSpan` | Rendered as |
| --- | --- | --- |
| `Empty` | — | an empty `<td>`, labelled *free* or *not a teaching period* |
| `Lesson { lesson, rows }` | `rows` | one `<td>` holding the lesson |
| `Continuation` | — | **nothing**. The span above covers this position |
| `Pinned { lessons, reason }` | **always 1** | one `<td>` holding **every** lesson in the cell |

**A lesson earns a `rowSpan` only when both hold: it is alone in every cell it occupies, and
its occupied periods are consecutive rows of this axis.** Everything else is `Pinned`.

### `PinReason`, and why there are three

| Reason | When | Is it a finding? |
| --- | --- | --- |
| `Conflict` | two or more occupants, and this scope expects exclusive use | **yes** — reported in `ScopeGrid::conflicts` |
| `Shared` | two or more occupants, and that is ordinary here | no. The master grid |
| `BrokenSpan` | one occupant whose run is not contiguous in this axis | no, but the lesson's `SpanDefect` is rendered |

`ScopeKind::expects_exclusive_use` is the one place the first two are told apart: a teacher, a
room, a section and a student can each be in one place at a time; the master grid is every
occurrence at once, and calling several in one cell a clash there would make the word useless.

### Nothing is discarded to make the table render

A `Pinned` cell carries every lesson that occupies it, and a degraded multi-period lesson
appears in a cell for each period it touches. The requirement is not *"the common case
renders"* — it is **no arrangement renders as a broken table, and no lesson disappears.**

### The invariant is asserted, not assumed

`GridPanel::is_well_formed` checks four things, and `GridPanel::malformations` says which one
failed:

1. there is exactly one cell per `(day, period)`, and no others;
2. every `rowSpan` fits inside the panel;
3. the cells a `rowSpan` covers are all `Continuation`;
4. no `Continuation` is uncovered.

Together: summing one row's rendered cells gives the day count, for every row. A test runs it
over every fixture, over every scope, and over deliberately overlapping arrangements.

---

## 4. One axis per bell schedule, never a merged one

[ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md): an
occurrence draws its slots from exactly one resolved bell schedule. Two campuses on two
schedules hold different `timeslot` rows over the same minutes
([ADR 0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) rule 2), so their period
rows do not align.

A scope touching two schedules gets **two panels**, each captioned with its campus, and a
sentence saying the two cannot be read as one week. A merged axis would have to assert that
Northgate's Period 3 and Southmoor's Period 3 are the same row. They are not.

The schedule a lesson is drawn on is the **slot's**, not the section's — which is the
resolver's decision, and is why `wrong_bell_schedule` is a defect a reader is shown rather
than a relocation the grid performs quietly.

---

## 5. Cycles, days and time

**Nothing in the view can read a weekday.** Not as a discipline — as a fact about the inputs:
`CycleDayFact` carries `id`, `ordinal` and `label` and no weekday at all. The columns are the
cycle's days in `ordinal` order and there is nothing else they could be.

A `WEEKLY` cycle and a six-day `CUSTOM` rotation go through the **same code path**. There is
no branch on `CycleKind` in the projection or in the grid, and that absence is the design.

`period.starts_at` / `ends_at` are zero-padded `'HH:MM'` under `CHECK (ends_at > starts_at)`,
so text comparison *is* chronological comparison. The view displays the stored strings and
sorts on them. It parses no date-time and applies no timezone: `school.timezone` is validated
as free text and parsed by no code in this repository.

A lesson's clock span is **the first period's start to the last period's end**, taken from the
resolved span — never a duration multiplied by a period length.

A break is rendered as a **row**, hatched and labelled *no teaching*, because an empty cell
says *free* and a break is not free.

---

## 6. Five scopes, and one of them is labelled differently

| Scope | Key | Several per cell is |
| --- | --- | --- |
| **All lessons** (master) | — | ordinary |
| **Class** | `activity → section` | a clash |
| **Teacher** | `section.teachers` | a clash |
| **Room** | `placement.room` | a clash |
| **Student** | `class_enrolment` | a clash |

A co-taught lesson appears on **every** teacher's grid, which is correct.

### A student's timetable is roster-derived, and says so

`StudentLabel::sections` is exactly what `class_enrolment` says, and a student no enrolment
row names is absent from the label set entirely — ADR 0044 §4.

So a per-student view:

- shows **only** the sections an enrolment row names;
- is marked `rosterDerived` and carries the caveat on screen;
- shows an empty grid, with an explanation, when the roster is empty — **never** the timetable
  of a group the student might be in.

ADR 0006 makes the roster optional, so its absence is an ordinary state. Substituting a
group's timetable would present a guess as a child's confirmed schedule, which is the one
failure of this view a parent would act on.

### Why the layout is a second call

`timetable_open` and `timetable_generate_result` return the projection — axes, the flat lesson
list, the selectable scopes — **once**. The laid-out grid for one scope comes from
`timetable_scope` / `timetable_generate_scope`.

A `ScopeGrid` is `days × periods` cells and there is one per teacher, room, section and
student: a large school has hundreds of scopes and one is on screen. Returning every grid
would send tens of thousands of cells to render forty. Laying out in TypeScript is the one
alternative that is not available, for §3's reason.

Neither call needs a cache. A load is a fixed number of statements and a projection of the
realistic upper end is single-digit milliseconds — §9.

---

## 7. Generation runs off the main thread

`AppState.workspace` is a `Mutex<Workspace>` and every command takes the guard. A
2 000-occurrence solve takes about 900 ms, and holding that lock for a second would freeze the
whole application.

It does not have to be held. `ARCHITECTURE.md` §3.3: the solver *"receives a fully
materialised problem in memory and returns a solution. It never opens a database."*

```
command thread   lock → load Assembly + labels + compile rules + validate → unlock
                 move the owned Assembly to a worker
worker thread    evara_solver::generate_with(engine, request, sink, cancel)
                 no database handle, no lock, no borrow
command thread   lock → save_generated(&assembly, &outcome, …)   (on the user's command)
```

`evara_db::scheduling`'s `send_assertions` test asserts `Assembly`, `Problem`, `Assignment`,
`CompiledConstraint`, `LabelSet` and `SolveOutcome` are all `Send`, so the design stops
compiling in the crate that broke it rather than in the one that depends on it.

### Progress is coalesced

`ProgressSink::placed` is called once per committed placement — about 2 000 times for the
upper end. `CoalescingSink` emits at most one `solver://progress` event per **50 ms**, plus
the final one, and records every call so `timetable_generate_status` is current between
events.

The clock it reads is the **caller's**. `SOLVER.md` §5 keeps every engine limit a count, so
the same seed stops in the same place; elapsed time is this layer's to measure.

```
solver://progress   { runId, epoch, placed, total, steps, elapsedMs }
solver://finished   { runId, epoch, hasResult, elapsedMs }
```

Both carry `runId`, and the frontend filters on it — which is what stops a session the user
has moved on from driving the screen in front of them.

---

## 8. The session lifecycle is an explicit state machine

```
                 ┌──────────────────── discard ───────────────┐
                 v                                            │
  start ──► Running ──► Finished ──► Saving ──► Saved         │
                           ^           │                     │
                           └─ failed ──┘   (the write is retryable)
```

`Saved` and `Discarded` are terminal. **Nothing leaves `Saved`.**

Five hazards, each with a guard and each with a test:

| Hazard | Guard |
| --- | --- |
| saving an unfinished run | `Running` refuses |
| saving a refused or broken run | `unsaveable_reason` — `InputRejected` and `InternalCheckFailed` |
| **two saves from two clicks** | `claim_save`'s compare-and-swap into `Saving` |
| a cancel racing a finish | cancellation sets a flag and **never** sets a state |
| a save into a different project | `ProjectEpoch`, bumped on every create, open and close |

### Cancellation is a request, not a transition

`request_cancel` sets an `AtomicBool` the engine reads between construction steps and changes
no state. The worker then finishes normally and reports `Termination::Cancelled`, so there is
exactly one path out of `Running` and no window in which a session is both running and
finished. A cancel after the run finished returns `false` — the honest answer, because there
was nothing left to stop.

A cancelled run **is** saveable: what it placed is real, and the user may want it.
`Termination::Cancelled` is the one termination this workspace documents as not reproducible,
so the screen offers no seed for it.

### Project close and switch

`project_create`, `project_open` and `project_close` each discard every session and bump the
epoch. Discarding cancels a running worker rather than leaving it computing an answer nothing
can save, and the epoch means a solve begun against one campus file can never be saved into
another — refused as `InputsChanged`, whose remedy is *generate again*, which is the true one.

---

## 9. Measured figures

`OPEN-DECISIONS.md` R5 names *"a 2,000-activity secondary school with electives"* as the
realistic upper end and asks for its numbers to be treated as a budget. Two of its three
figures were unmeasured after M6; **grid rendering had no grid to render until this
milestone.** These are the projection half, from
`evara-report`'s `projection::budget` (debug build, so a release build is faster):

| Figure | Measured |
| --- | --- |
| project 2 000 occurrences, 40 rooms, 2 schedules, dense arrangement | **3.8 ms** |
| lay out four scope grids from that projection | **417 µs** (160 cells) |
| one scope grid | **40 cells**, for a 5-day × 8-period school |
| cells in a scope grid as the school grows 10× | **unchanged** — `days × periods`, not lessons |

The last row is the claim the desktop design rests on, and it is asserted rather than argued:
a teacher's grid is the same size in a 200-occurrence school and a 2 000-occurrence one, which
is why switching scope is cheap and why the master grid is the only one that needs a layout
decision at all.

**The master grid is one panel of 40 cells too.** Lessons stack inside them rather than
multiplying the cell count, and a test asserts all 2 000 are reachable from it with none
discarded. That is why pagination, not virtualisation, is the master grid's answer when one
is needed — a virtualised `<table>` would break the semantics §11 depends on.

### What is still not measured

`Scheduling::load` wall-clock time. M4's argument — a fixed statement count per load,
independent of school size — is a strong one and is not a measurement, and M8 does not turn it
into one.

---

## 10. What a view says, and what it refuses to say

### Three states, never merged

| State | Means |
| --- | --- |
| a hard violation | the timetable **cannot be run** |
| partial | usable, with lessons unplaced — an **ordinary state**, not an error |
| unscheduled | which lessons, and whether anything proves it impossible |

A single "problems" list would merge them, which is the failure `is_usable()` is deliberately
not called `is_feasible` to avoid.

### The word *impossible* is gated

`UnplacedCause::is_impossible` decides it, in Rust, and the view renders the two groups under
different headings. Everything else reads *"this run could not place these"*, because a greedy
construction that ran out of room has learnt that **its** arrangement had none.

A stored cause key this build does not recognise is **not** reported as impossible, and the
screen says the reason was recorded by a newer version of Evara.

### Four figures, always four

`UnverifiedPanel` renders all four `UnverifiedCounts` **including at zero**, where it reads
*"nothing undisclosed"* and still says **no confirmed clash within this school** rather than
*conflict-free*.

| Figure | Said as |
| --- | --- |
| `indeterminateStudentPairs` | N pairs not declared disjoint; groups that share children may be together |
| `uncheckedCapacities` | N lessons have no expected attendance, so no capacity was compared |
| `sharedEntities` | N entities are also held by another school; Evara cannot compare the two |
| `unenforcedRestrictions` | N stored restrictions are not enforced by this engine |

**ADR 0032's acceptance test 12 is discharged here.** In a compiled view the shared entities
are **named**, with both schools and the consequence, from `TeacherFact::also_at` and
`RoomFact::also_at` — rule 3 requires exactly that, and warns that *"a warning a user learns
to dismiss is worse than none"*. In a stored-only view the count is all there is, and the
panel says *"open this timetable to see which"* rather than implying it knows.

### Two scorings, and neither speaks for the other

A `GENERATION` evaluation is immutable; a `REVALIDATION` is replaced wholesale. Both are shown
side by side, each with its timestamp. A screen showing one number would have to choose which,
and either choice is a lie about the other.

### Findings, at two fidelities

| Tier | Needs | Carries |
| --- | --- | --- |
| **Stored** | a `violation` row | level, penalty, the stored `detail`, entities by id, the day and the span |
| **Compiled** | `Problem` + `Assignment` | all of the above, plus **the rule's own sentence** |

The compiled tier calls `explain()` — ADR 0043 — and composes no wording of its own. The
stored tier carries **no** `message`, because the engine's sentence is deliberately not
stored: *"a wording improvement would leave old rows reading the old way."*

---

## 11. Accessibility

`AvailabilityGrid`'s rule, extended: *"A grid of `<div>`s with click handlers would have been
fewer lines and unusable by half the people who have to configure a school."* It applies with
more force here, because a timetable is read far more often than it is configured.

- A real `<table>` with `<caption>`, `scope="col"` on day headers and `scope="row"` on period
  headers. A multi-period lesson is one `<td rowSpan>`, which is what makes a screen reader
  announce it **once** rather than three times.
- Every cell carries the whole sentence, because its position on screen is not available to
  somebody listening: *"Day A, 2 periods from 1 — 9 Mathematics B, A. Okafor, Room B12, 09:00
  to 11:00."* A clash is prefixed *"Clash."*; a broken run *"Broken run."*
- Roving tabindex with arrow keys, `Home` and `End`; exactly one tab stop in the grid.
- Progress is `aria-live="polite"` at the coalesced rate, so 2 000 placements are not 2 000
  announcements. The progress bar carries `aria-valuenow` / `aria-valuemax`.
- The rail and both tab strips mark the showing item with `aria-current`.
- **Colour never carries meaning alone.** Every pinned cell renders an icon and a word; a
  clash adds a hatch a monochrome display still shows; a break row is hatched as well as
  labelled.

Every one of these is asserted in `TimetableGrid.test.tsx` rather than reviewed.

---

## 12. Read-only, and visibly so

M8 displays timetables and does not edit them. The boundary is drawn by **absence**, not by
disabling: a greyed-out drag handle teaches a user that editing exists here, and it does not.

| Not built | Owner |
| --- | --- |
| `apply_move`, `validate_move`, drag-and-drop, hover validation, undo/redo | ROADMAP Phase 8 |
| locking, repair, partial regeneration, `minimise-disruption` | Phase 8, `SOLVER.md` §7 |
| print, print stylesheets, PDF, CSV, the renderer trait | Phase 10 |
| `.evara` export of a timetable | Phase 9 |

`PlacementVerdict` already exists and is exactly what `validate_move` returns. M8 does not
expose it, so ROADMAP Phase 5's *"no Tauri handler exposes it yet"* stays true.

`is_locked` and `is_manual` are **rendered where set and written nowhere**, so that when
Phase 8 begins writing them the grid already tells the truth.

**The four writes in the whole milestone** are `save_generated`, `rename`, `delete` and
`record_revalidation`. Every other command goes through `Scheduling`, whose receiver is a
shared reference — so the borrow checker, not a convention, forbids it opening a transaction.

---

## 13. Revisions

| Mutation | Contract |
| --- | --- |
| `timetable_save` | **no revision**: there is no previous version of a row that does not exist. Guarded by the watermark, inside the writing transaction |
| `timetable_rename` | `expectedRev` of the timetable |
| `timetable_delete` | `expectedRev` of the timetable; the children at `Expected::Any` inside its transaction |
| `timetable_revalidate` | `expectedRev` of the timetable, which it **consumes** |

The revision is always the **interface's**, never one the handler read for itself: reading it
here would be read-compare-write, which ADR 0027 calls *"the same race with a longer window"*.
**No handler constructs `Expected::Any`.**

---

## 14. A timetable whose grid has moved is still worth opening

If `Scheduling::load` fails — a cycle deleted, a corrupt stored identifier, a cycle that now
belongs to another school — `timetable_open` returns `projection: None` with `unreadable` set
to a sentence, **not** an error. The stored findings are still there, which is what
`TIMETABLE-PERSISTENCE.md` §7 keeps them for.

The screen then shows the findings, the scorings, the disclosure and the reason there is no
grid. It draws **no** grid rather than a partial one: deriving spans without a problem would
be a second implementation of the span measure.

A timetable that *is* readable but whose entries name things the project no longer holds says
so explicitly — `notInProblem` against `storedEntries` — because a short grid shown as whole
is the one failure a school cannot detect for itself.

---

## 15. Tests

| Where | Count | Covers |
| --- | --- | --- |
| `evara-core::scheduling::display` | 7 | the caption composed once, and that an absent label stays absent |
| `evara-report::projection` | 28 | the well-formedness invariant over every fixture and scope; conflicts; broken spans; two axes; cycle kinds; roster semantics; determinism |
| `evara-report::projection::budget` | 2 | R5's upper end, and that a scope grid does not grow with the school |
| `evara-report::findings` | 4 | the two fidelities, and that an unrecognised rule is reported rather than dropped |
| `evara-db::tests_views` | 11 | the label read, school isolation both ways, the roster, the `COALESCE`d cycle list |
| `evara-db::scheduling::send_assertions` | 1 | the worker-thread design, as a compile-time property |
| `src-tauri::commands::session` | 23 | the state machine's five hazards, including 16 concurrent save claims |
| frontend | 70 across 5 files | the grid's rendering and accessibility, the disclosure wording, the IPC contract, the Generate workflow, the saved-timetable area, the workspace rail |

Named assertions worth knowing:

- `every_scope_of_every_fixture_renders_a_well_formed_table` — §3's invariant, swept.
- `a_multi_period_lesson_overlapping_a_single_one_pins_every_cell_it_touches` — the broken
  table this design exists to prevent.
- `two_single_period_lessons_in_one_slot_are_pinned_and_neither_is_discarded` — nothing is
  thrown away to make a table render.
- `a_student_grid_is_derived_from_enrolment_rows_and_nothing_else`, and
  `a_student_enrolled_in_nothing_has_an_empty_grid_rather_than_a_groups`.
- `concurrent_claims_from_many_threads_yield_exactly_one_winner` — the double-click race, as
  the race it actually is.
- `nothing_leaves_the_saved_state`.
- `a_cancel_after_the_run_finished_changes_nothing_and_says_so`.
- `the_realistic_upper_end_projects_within_budget` — §9's figures, reproducible with
  `--nocapture`.

---

## 16. Known limitations

| Limitation | Why | Where it goes |
| --- | --- | --- |
| **No editing.** No move, no lock, no repair | §12 | ROADMAP Phase 8 |
| **No print, PDF or CSV** | the renderer trait is Phase 10's, and a trait with one implementation is a guess about the second | Phase 10, OPEN-DECISIONS D10 |
| **The master grid is not paginated yet** | measured at 40 cells for the upper end, so nothing needs it; the shape is ready if a measurement changes | §9 |
| **Opening a timetable compiles a problem every time** | the alternative is a cache whose lifetime nothing else in the crate has; a load is cheap | §6 |
| **A scope switch re-projects** | same reason. 3.8 ms at the upper end | §9 |
| **`Scheduling::load` wall-clock time is unmeasured** | M4's statement-count argument stands and is not a measurement | OPEN-DECISIONS R5 |
| **No day-at-a-time default above ten days** | `DayStrip` exists and the threshold is defined; the grid renders the whole cycle, which is correct up to the sizes the schema permits | a display preference, not a rule |
| **A generation session does not survive a restart** | it holds a compiled problem in memory, and the project's own state is what persists | by design |
