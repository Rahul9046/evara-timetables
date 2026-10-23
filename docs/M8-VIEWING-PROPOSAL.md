# M8 — Timetable Viewing and Reporting · Architecture Proposal

- Status: **Proposed — awaiting approval. Nothing is implemented.**
- Date: 2026-10-23
- Branch: `feat/m8-timetable-viewing`
- Base: `1571d1e` (`origin/develop`, "Merge pull request #31 … feat/m7-timetable-persistence")
- Schema version at base: **`V17`** · proposed: **no migration** (§11)
- Discharges: [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) §13 *"Owed by M8"* (four items),
  ROADMAP Phase 6 *"Still to do"* (the Generate screen, the background thread,
  `solver://progress`), ROADMAP Phase 7 in full, ADR 0032's future acceptance test **12**, and
  the grid-rendering half of [OPEN-DECISIONS](OPEN-DECISIONS.md) R5.

This proposes the projection layer, the IPC surface, the component structure, the test strategy
and an implementation sequence for the ten areas the brief names. It ends with the decisions that
need a yes before any code is written.

**Eight conflicts with accepted records were found and none is resolved unilaterally.** §0 lists
them; each is a numbered decision in §10.

---

## 0. Conflicts and gaps found, up front

| # | The conflict or gap | Where | §10 |
| --- | --- | --- | --- |
| **C1** | The brief's M8 is **two milestones** in the accepted sequence. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 sequences *"M8 orchestration"* and *"M9 review — renders §5 rule 4's counts"*; ROADMAP splits them the same way, into Phase 6's remainder and Phase 7. The brief merges both into M8. | SAD §6, ROADMAP Phases 6–7 | **D1** |
| **C2** | **`evara-report` cannot see a lesson's name.** It depends on `evara-core` only, and `SectionFact.name` is `class_section.name` alone (`scheduling/lessons.rs:54,87`) — the compiled `Problem` carries no subject, course, year level, building or student. A grid cell reading "B" is not a timetable. ADR 0037 §2 forbids widening `Problem` with *"the first field whose only consumer is a diagnostic"*, and the identical argument bars a field whose only consumer is a report. | CLAUDE.md §2, ARCHITECTURE §3.1, ADR 0037 §2 | **D2** |
| **C3** | **A grid cannot be drawn without a compiled `Problem`.** `span::resolve_span` takes `&Problem`; `MaterialisedGrid::of` and `Problem::slots_of` likewise. ADR 0042 D2 moved the resolver down *so that there would be one implementation*, so a "lightweight" viewer that derived spans from `timetable_entry` rows alone would be the second copy CLAUDE.md §2 calls the most important rule in the repository. | `span.rs:107`, `resolve.rs:64`, ADR 0042 D2 | **D3** |
| **C4** | ROADMAP Phase 7 lists a **by-student** projection. ADR 0006 requires a full timetable from `student_group.size` with no student records at all; SAD §7 defers *"individually enrolled students as a scheduling population"*; `class_enrolment` is optional everywhere and has no scheduling meaning in M1. A per-student grid is derivable and is **not** a scheduling projection. | ROADMAP Phase 7 vs ADR 0006, ADR 0029 §4, ADR 0035 §7 | **D5** |
| **C5** | Phase 7's score-health panel wants *"clicking through to the offending entries"*, which needs `explain()`, which needs a `Problem`. TIMETABLE-PERSISTENCE §7 says stored violations exist precisely so a view can render findings **without** loading one. Both are true; they are two different screens, and conflating them would make one of the two guarantees false. | ROADMAP Phase 7 vs TIMETABLE-PERSISTENCE §7 | **D6** |
| **C6** | **No `evara-core::scheduling` type carries a `ts` derive, and the typegen contract names none.** Forty-plus view types must be added to `write_all`, and `check_dependencies_are_listed` fails the build if one references an unlisted type. `Problem` must never enter that list — 2 000 activities of dense facts is not an IPC payload. | `typegen.rs:168–340`, ADR 0012 | **D4** |
| **C7** | **`SetupError` has no variant for the three refusals M8 must surface:** `InputsChanged`, `NotSaveable`, `ReadOnlyProject`. `ReviewRequired` is the closest and its remedy is wrong — *re-preview* rather than *generate again*, which TIMETABLE-PERSISTENCE §13 item 4 requires by name. | `commands/error.rs:29–140` | **D8** |
| **C8** | ADR 0032 test 12 requires a timetable's **views** to carry the shared-entity warnings. The stored `timetable` row carries only the **count** (`shared_entities`); the **names** live in `TeacherFact::also_at` / `RoomFact::also_at`, i.e. only in a compiled `Problem`. So the two tiers of §5 disclose different amounts, and a screen must not imply otherwise. | ADR 0032 test 12, `problem.rs:174,260` | **D3**, **D7** |

Two further gaps are not conflicts, only absences, and both are named here so they are not
discovered mid-implementation:

- **`Timetables` has no list-by-cycle and no count.** `for_school` returns every timetable of a
  school, newest first (`repo/timetables.rs:390`). That is enough for M8; a cycle filter is a
  `WHERE` clause and is proposed in §6.1 only because the Generate screen's "this grid already
  has three timetables" line needs it.
- **Nothing in `evara-report` exists.** Thirteen lines of module documentation and a
  `Cargo.toml` already wired into `evara-cli` and `evara-desktop`. The crate is a greenfield
  with its layering already fixed.

---

## 1. Q1 — Grid projections for teachers, sections, rooms and students

### 1.1 Where the projection lives, and the one thing that decides it

`evara-report` is the crate for projections and its dependency edge is `-> core` (CLAUDE.md §2).
The frontend holds no rules (CLAUDE.md §2, ARCHITECTURE §2). Between those two, the projection
has exactly one legal home: **Rust, in `evara-report`, fed by the command layer.**

What it needs is four things, and three of them already exist:

| Input | Source | Exists? |
| --- | --- | --- |
| `&Problem` — facts, context, teacher/room/group/section names, campuses, bell schedules, days, periods, intervals | `evara_db::scheduling::Scheduling::load` | ✅ M4 |
| `&Assignment` — which occurrence starts in which slot, in which room | `Timetables::arrangement` | ✅ M7 |
| the span each placement occupies | `evara_core::scheduling::span::resolve_activity_span` | ✅ M7 |
| **display labels the `Problem` does not carry** — subject, course, year level, building, student | — | ❌ **C2** |

### 1.2 The label gap, and the shape that resolves it (D2)

`Problem` must not grow a report-only field (ADR 0037 §2). `evara-report` cannot reach SQL.
`evara-db` cannot depend on `evara-report`. So the display vocabulary has to sit **underneath
both**, which is the same argument ARCHITECTURE §3.2 makes for `StoredConstraint` and ADR 0042 D2
makes for `span`:

```
evara-core      scheduling::display::LabelSet      <- new, pure data, no I/O
   ^        ^
   |        └── evara-report   reads it, joins it to the Problem, emits projections
   └── evara-db  populates it from SQL, beside the Assembly, never inside the Problem
```

`LabelSet` is a lookup from `Uuid` to the names a grid needs and a solve does not:

```rust
pub struct LabelSet {
    pub sections: Vec<SectionLabel>,   // subject, course, section, year level, campus
    pub rooms:    Vec<RoomLabel>,      // room, building
    pub students: Vec<StudentLabel>,   // only where class_enrolment rows exist — D5
}

pub struct SectionLabel {
    pub section_id: Uuid,
    pub subject: String,
    pub course: String,
    pub section: String,
    pub year_level: Option<String>,
    pub short: String,   // "9 MAT-B" — composed once, in Rust, so two screens cannot disagree
}
```

**Loaded on request, not on every assembly.** `Scheduling::labels(&selection)` is a second,
opt-in read (§6.1), so the solver path stays at M4's statement count and a solve never pays for
a vocabulary it does not use. Carrying it on `Assembly` beside `rules` was the alternative and is
rejected for exactly that reason.

### 1.3 The four scopes, and why the master grid is not a fifth

Each projection is a **grouping of the same resolved spans by a different key**. The spans are
resolved once:

| Scope | Key | Rows | Columns |
| --- | --- | --- | --- |
| **Section** (class) | `activity → section` | periods of the section's resolved schedule | cycle days |
| **Teacher** | `section.teachers` | periods of each schedule the teacher appears on | cycle days |
| **Room** | `placement.room` | periods of each schedule the room is booked on | cycle days |
| **Student** | `class_enrolment` → sections | as the sections they are in | cycle days |
| **Master** | nothing — every span | one block per section | cycle days |

A span appears in **as many teacher grids as it has teachers**, which is correct: a co-taught
lesson is on both teachers' timetables. `SectionFact.teachers` is a `Vec<TeacherIx>` already.

The master grid is the same data with no grouping key, so it is not a fifth projection — it is
the ungrouped set. It is the only one large enough to need a layout decision (§7.3).

### 1.4 Multi-period activities

A `duration_periods = 3` occurrence is **one** `timetable_entry` row (ADR 0042 D1) and
**one** `ProjectedLesson`, carrying the slots it occupies:

```rust
pub struct ProjectedLesson {
    pub activity_id: Uuid,
    pub section_id: Uuid,
    pub label: SectionLabel,
    pub activity_name: Option<String>,      // ActivityFact.name, where set
    pub teacher_ids: Vec<Uuid>,
    pub teachers: Vec<String>,
    pub group_ids: Vec<Uuid>,
    pub room_id: Option<Uuid>,
    pub room: Option<RoomLabel>,
    pub schedule_id: Uuid,                  // which bell schedule the run was taken from
    pub cycle_day_id: Uuid,
    pub day_ordinal: u16,
    pub occupies: Vec<ProjectedCell>,       // one per slot, in ordinal order
    pub starts_at: String,                  // 'HH:MM', the first period's
    pub ends_at: String,                    // 'HH:MM', the last period's
    pub duration_periods: u16,
    pub defect: Option<SpanDefect>,         // non-None is rendered, never swallowed — §1.5
    pub is_locked: bool,
    pub is_manual: bool,
}
```

The frontend renders `occupies.len()` as a `rowSpan` and never computes one. The **wall-clock
interval is derived from the periods covered, never the other way round** (CLAUDE.md §3), and it
is derived in Rust: `starts_at` and `ends_at` are the resolved span's, not a product of
`duration × period_length`.

### 1.5 A defect is rendered, never swallowed

`SpanDefect` exists because *"resolution is permissive; judging is a rule's job"* (`span.rs`
header). A stored timetable whose grid has since moved will produce spans that are short, broken
or on the wrong schedule. The viewer's obligation is the resolver's reason for existing:
**render what is there and say what is wrong with it.** A cell whose span is `!is_sound()` is
marked, and the lesson is listed in the findings panel with the remedy. A viewer that refused
would *"leave the user unable to see what to fix"* — `arrangement`'s own documentation.

### 1.6 Differing campus bell schedules

ADR 0033: an activity draws its slots from **one** resolved bell schedule. Two campuses on two
schedules hold different `timeslot` rows over the same minutes (ADR 0032 rule 2). A teacher
eligible for both campuses therefore has lessons on two schedules, whose period rows do not
align.

**One axis per bell schedule, never a merged one.** A merged axis would have to decide that
Northgate's Period 3 and Southmoor's Period 3 are the same row, which is the equation ADR 0032
spends three pages refusing to make *across schools* — and within a school it is merely false
rather than unanswerable.

```rust
pub struct ScheduleAxis {
    pub schedule_id: Uuid,
    pub schedule_name: String,
    pub campus_id: Option<Uuid>,
    pub campus_name: Option<String>,
    pub periods: Vec<ProjectedPeriod>,   // ordinal, label, starts_at, ends_at, kind
}
```

A scope touching one schedule renders one grid. A scope touching two renders **two stacked
grids, each captioned with its campus**, and a line saying the two cannot be read as one week.
That is the honest rendering and it is also the cheap one.

### 1.7 Non-teaching periods

`PeriodFact.kind` distinguishes teaching from break. A break row is rendered as a row — it is
part of the day a reader is looking for — and is styled as unavailable rather than empty, so
"nothing is scheduled here" and "nothing may be scheduled here" do not collapse into one
appearance.

---

## 2. Q2 — Cycles, day navigation and time display

### 2.1 Nothing in the view reads a weekday

CLAUDE.md §3 and ADR 0017: `cycle_day.ordinal` is a day's only scheduling identity; `label` and
`weekday_hint` are display. The columns are the cycle's days in `ordinal` order, straight from
`SchedulingContext.cycle_days`, which `AvailabilityGrid` already does and says so in its header.

A `WEEKLY` cycle and a six-day `CUSTOM` rotation render **through the same code path**. A weekly
cycle stores all seven weekdays; a rotation that drifts against the calendar renders exactly as a
week does. No branch on `CycleKind` exists in the grid, and that absence is the design.

`weekday_hint` may be shown **beside** a weekly cycle's label as a secondary line, because for a
`WEEKLY` cycle it is reliably the same weekday and users read it. It is never shown for `CUSTOM`,
where it would be a claim about a date that nothing has computed, and it is never an input to
anything.

### 2.2 Day navigation

Three views of the same columns, and the cycle's size decides the default:

| Mode | When | Shape |
| --- | --- | --- |
| **Whole cycle** | default up to 10 days | every day as a column |
| **One day** | a user choice, and the default above 10 days | one column, full period detail |
| **Day strip** | always available | a row of day buttons, roving tabindex, `aria-current` on the shown one |

The threshold is a display preference in the frontend, not a rule, and `cycle_day` is capped
elsewhere in the schema already (`tests_cycle_day_limit.rs`).

### 2.3 Time display

`period.starts_at` / `ends_at` are zero-padded `'HH:MM'` under `CHECK (ends_at > starts_at)`, so
no period crosses midnight and **text comparison is chronological comparison** (ADR 0032 rule 2).
The view therefore displays the stored strings and sorts on them. It does **not** parse them into
a date-time, and it never applies a timezone: `school.timezone` is validated as free text and
parsed by no code in the repository (ADR 0032 §"Two schools have no common clock"), so formatting
through it would be the first consumer of a dependency this project does not carry.

---

## 3. Q3 — The generation workflow in the desktop application

### 3.1 Threading, and the one problem worth solving carefully

`AppState.workspace` is a `Mutex<Workspace>` and every command takes the guard
(`commands/mod.rs:44`). A solve takes ~900 ms for 2 000 occurrences
(TIMETABLE-CONSTRUCTION-SOLVER §14). Holding that lock for a second would freeze every other
command, and `Scheduling` borrows `&Database` from the guard, so the borrow cannot cross a thread
boundary.

It does not have to. ARCHITECTURE §3.3: the solver *"receives a fully materialised problem in
memory and returns a solution. It never opens a database."* So:

```
command thread          lock → load Assembly + compile rules + read labels → unlock
                        move the owned Assembly into the worker
worker thread (OS)      evara_solver::generate_with(&engine, &request, &sink, &cancel)
                        no database handle, no lock, no borrow
command thread          lock → save_generated(&assembly, &outcome, …)   (on the user's command)
```

`Assembly` is plain owned data — `Problem`, `ProblemIndex`, `Vec<AvailabilityRule>`,
`Vec<Diagnostic>`, `Watermark`. **A compile-time assertion is part of the deliverable**
(`const fn assert_send<T: Send>()` over `Assembly`, `Problem` and `CompiledConstraint`), because
discovering this at the point of use would be a redesign rather than a fix.

### 3.2 The session, and why the Assembly is kept

`GenerateSessions` mirrors `ImportSessions`, which is the established precedent for
multi-call state in this crate (`commands/import.rs`, `lib.rs:44`):

```rust
pub struct GenerateSession {
    run_id: Uuid,
    selection: SchedulingSelection,
    assembly: Arc<Assembly>,          // the one the outcome was built from — see below
    labels: Arc<LabelSet>,
    cancel: Arc<AtomicBool>,
    started_at: String,
    state: Mutex<SessionState>,       // Running | Finished(SolveOutcome) | Failed(String)
}
```

**The Assembly is kept because the save requires it.** `save_generated` refuses
`DbError::Invalid` if the outcome's `ProblemIdentity` does not match the assembly it is offered
with, and that check exists because *"a `SolveOutcome` written against a different assembly's
index would translate every dense index to the wrong identifier and store a complete, plausible,
wrong timetable"* (TIMETABLE-PERSISTENCE §9). Reloading the assembly at save time would defeat
it. Keeping it also makes the preview free: the result is projected from the same objects.

A session is dropped on save, on explicit discard, and on project close.

### 3.3 Selection

School, cycle and term are all named by the caller and **none is defaulted** — ADR 0032 rule 1,
and `SchedulingSelection`'s own documentation says the absence of a default is the decision. The
screen therefore offers three pickers, and the school comes from the School Setup shell's
existing selection rather than a second one (ADR 0023: the interface scopes to one school).

Before the button is enabled the screen runs **pre-solve validation**, which is already a
single call: `Scheduling::validate(&selection)` → `AssemblyReport`, read-only. A blocking
finding disables generation and names the remedy; a warning is shown and does not. That ordering
is M3's and M6's gate restated in the interface: *"a problem whose inputs were refused is not
searched at all"*.

### 3.4 Progress and cancellation

`ProgressSink::placed(placed, total, steps)` is called once per committed placement and once per
occurrence given up on — about 2 000 times for the upper-end school. Emitting 2 000 Tauri events
is affordable and wasteful; the sink **coalesces** to at most one event per 50 ms *plus* the
final one, so a bar is smooth and the channel is not the bottleneck.

```
solver://progress   { runId, placed, total, steps, elapsedMs }
solver://finished   { runId, status, termination }
```

`elapsedMs` is the **caller's**, measured on the command side, exactly as `SOLVER.md` §5 requires
— the engine reads no clock, which is what keeps the same seed reproducible. Cancellation flips
the `AtomicBool` behind a `Cancel` impl; `Termination::Cancelled` is the one outcome this
workspace documents as not reproducible, and the screen says so rather than offering a seed.

### 3.5 Result preview, and the four things it must say

The preview is the projection of §1 over the in-memory outcome — **the same `evara-report` code
that renders a stored timetable**, which is what makes "what I previewed is what I saved" a
structural property rather than a hope.

Above the grid, in this order:

1. **What it is.** *"No confirmed clash within Northgate"* — never "conflict-free"
   (CLAUDE.md §3, SAD §5 rule 4).
2. **Completeness.** `Partial { placed, required }` is an ordinary state, not an error
   (CONSTRAINT-ENGINE §6). Unplaced occurrences are listed with
   `UnscheduledActivity::cause`, and **`is_impossible()` is the only thing allowed to phrase one
   as impossible** (CLAUDE.md §3).
3. **The score**, three integers, labelled as the generation figure with its timestamp.
4. **`UnverifiedCounts`**, all four, as a standing panel — §5.3.

### 3.6 Saving, and what happens after (D7)

Saving is an explicit second action. `timetable_save(runId, name)` → `save_generated`. It
**always inserts** (ADR 0042 D3); the screen says so, and never offers "overwrite".

`InputsChanged` is the refusal with a real remedy and TIMETABLE-PERSISTENCE §13 item 4 makes its
wording M8's obligation: *"A scheduling input changed since this timetable was generated, so
saving it could store a wrong timetable. Generate again."* with a **Generate again** button that
reloads the assembly and re-runs. Nothing in the interface compares a watermark — ADR 0030's rule
holds here (CLAUDE.md §3).

**Recommendation for M7 §13 item 3** (open the new timetable, or list it beside the others):
**open it, and leave the list showing it as the newest.** The user asked for a timetable and a
screen that answered with a list entry would make them click again for the thing they just
generated. There is still no *"current timetable"* pointer and none is proposed — the selection
is a screen's memory, which is what TIMETABLE-PERSISTENCE §12 said it was.

---

## 4. Q4 — Saved timetable management

A **Timetables** area, new top-level navigation beside School Setup (D11), with a versions list
and an opened timetable:

| Operation | Command | Contract |
| --- | --- | --- |
| **List** | `timetable_list(schoolId)` → `Vec<TimetableSummary>` | `for_school`, newest first. One row per `timetable` |
| **Open** | `timetable_open(id)` → `TimetableView` | §6.3. Compiles a `Problem`; discloses what it could not read |
| **Rename** | `timetable_rename(id, expectedRev, name)` | `expectedRev` of the timetable; `DuplicateValue` on a name collision in the `(cycle, term)` scope |
| **Delete** | `timetable_delete(id, expectedRev)` → `DeletedTimetable` | one transaction, children counted and change-logged |
| **Revalidate** | `timetable_revalidate(id)` → `TimetableScoring` | `record_revalidation`; **consumes** the timetable's revision |

Four properties the screen must carry, each from a record rather than from taste:

- **A list row is a version.** Several timetables coexist for one `(cycle, term)` and *that
  coexistence is the versioning* (ADR 0042 D3). No version number is invented, no "current"
  badge is drawn, and the list is ordered by `generated_at`.
- **Replacement is two decisions.** Save, then delete. The screen never offers "replace".
- **Delete names what it removes.** `DeletedTimetable` carries five counts; the confirmation
  shows them, because *"deleting a timetable removes its children and writes each one"* is a
  fact worth being told before it happens.
- **A rename takes the revision.** A second window that renamed it first gets `Conflict`, and
  the existing `RevisionConflicts` component is the precedent for the message.

### 4.1 Two scores, labelled as what they are

`evaluations()` returns up to two rows: an immutable `GENERATION` and a replaceable
`REVALIDATION` (ADR 0042 §5). The panel shows **both, side by side, each with its timestamp**:

> **As generated** · 23 Oct 2026 14:02 · hard 0 · medium −40 · soft 0
> **As it stands** · 23 Oct 2026 16:41 · hard −200 · medium −40 · soft 0

A screen showing one number would have to choose which, and either choice is a lie about the
other. `is_generation()` is the discriminator and no string comparison on `kind` happens in
TypeScript.

---

## 5. Q5 — Violations, partial solutions, unscheduled activities, unverified counts

### 5.1 Two fidelities, and the rule that keeps them apart (D6)

| Tier | Needs | Can say | Cannot say |
| --- | --- | --- | --- |
| **Stored** | nothing but `violations_for` | every finding, its level, its penalty, its `detail`, the entities by id, the day and the `'HH:MM'` span | nothing new; no `explain()` sentence; no entity **names**; no grid |
| **Compiled** | `Problem` + `Assignment` | everything above, plus `evaluator_for(t).explain(problem, v)`, entity names, click-through to cells, and the current findings | — |

The stored tier is what TIMETABLE-PERSISTENCE §7 bought: *"they let a view render findings
without loading a `Problem` at all, which Phase 7's score health panel needs."* It is the
fallback when `Scheduling::load` **fails** — a cycle deleted, a corrupt identifier,
`WrongSchool` — and in that state the screen shows the findings and the metadata and **says the
grid could not be built, with the reason**. It never draws a partial grid (C3): deriving spans
without a `Problem` would be the second span implementation, and CLAUDE.md §2 forbids it in the
strongest terms the file contains.

For the ordinary case the compiled tier is the one used, and it is cheap: M4's loader is 29
statements per load independent of school size.

### 5.2 Hard violations, partial solutions, unscheduled activities

Three distinct states that a single "problems" list would merge, and merging them is the failure
`is_usable()` is deliberately not called `is_feasible` to avoid:

| State | From | Phrasing |
| --- | --- | --- |
| **Hard violation** | `Violation` at `ConstraintLevel::Hard` | *"this timetable cannot be run"* — `is_usable() == false` |
| **Partial** | `Completeness::Partial { placed, required }` | *"usable, with N lessons unplaced"* — an ordinary state |
| **Unscheduled** | `UnscheduledActivity` / `UnscheduledRecord` | the cause, grouped. `is_impossible()` gates the word *impossible* |

`UnscheduledActivity` already carries `id: Uuid` and `section: String` *"so a report need not
hold the problem to name it"*, which is the pattern §1.2 generalises.

For a **stored** timetable the unplaced set comes from `unscheduled_for(run.id)`, whose
`is_impossible()` reads the stored key through `UnplacedCause` and deliberately **does not
report an unrecognised key as impossible** (TIMETABLE-PERSISTENCE §7). The view inherits that
and must not second-guess it: an unrecognised cause is shown as *"this run could not place it;
the reason was recorded by a newer version of Evara"*.

### 5.3 Unverified counts — a standing panel, never a toast

Four figures, from `Timetable::unverified()` for a stored one and `SolveOutcome::unverified` for
a fresh one — the same `UnverifiedCounts` shape either way, which is why
`Timetable::unverified()` reassembles the core type.

| Figure | Sentence |
| --- | --- |
| `indeterminateStudentPairs` | *"N pairs of student groups have not been declared disjoint. Two groups that share children may be timetabled together."* |
| `uncheckedCapacities` | *"N lessons have no expected attendance, so no room capacity was compared."* |
| `sharedEntities` | *"N teachers, rooms or resources are also held by another school. Evara cannot compare the two schools' timetables."* |
| `unenforcedRestrictions` | *"N stored availability restrictions are not enforced by this engine."* |

All four are shown **when zero as well**, reading *"nothing undisclosed"*, because a panel that
disappeared would make its absence unreadable — and even at all zeroes the correct phrasing is
*"no confirmed clash"*, not *"conflict-free"* (`UnverifiedCounts` documentation).

**ADR 0032 test 12 is discharged here**, and C8 is the limit: in the compiled tier the shared
entities are **named**, from `TeacherFact::also_at` and `RoomFact::also_at`, with both schools and
the consequence — ADR 0032 rule 3 requires exactly that and warns that *"a warning a user learns
to dismiss is worse than none"*. In the stored-only tier the count is all there is, and the panel
says *"N shared entities — open the timetable to see which"* rather than implying it knows.

---

## 6. Q6 — IPC contracts and TypeScript types

### 6.1 New Rust APIs (the smallest set that works)

| Crate | Addition | Why |
| --- | --- | --- |
| `evara-core` | `scheduling::display::{LabelSet, SectionLabel, RoomLabel, StudentLabel}` | the vocabulary two crates share sits underneath both — D2 |
| `evara-db` | `Scheduling::labels(&SchedulingSelection) -> Result<LabelSet>` | opt-in, so a solve pays nothing |
| `evara-db` | `Timetables::for_cycle(cycle_id, term_id) -> Result<Vec<Timetable>>` | the Generate screen's version count |
| `evara-report` | `projection::{TimetableProjection, ProjectedLesson, ProjectedCell, ScheduleAxis, ProjectedPeriod, ProjectionScope}` | the projection types |
| `evara-report` | `projection::project(&Problem, &Assignment, &LabelSet) -> TimetableProjection` | **the one function.** Calls `span::resolve_activity_span`; restates nothing |
| `evara-report` | `findings::explain_all(&Problem, &[Violation]) -> Vec<Explanation>` | calls `evaluator_for(t).explain` — which means `evara-report` gains an `evara-constraints` dependency. **D9** |

**D9 is a real layering question and is not decided here.** CLAUDE.md §2 pins
`evara-report -> core`. Rendering an `explain()` sentence needs the registry. Three options, §10.

### 6.2 Commands

Twelve, in two modules — `commands/solve.rs` and `commands/report.rs`, which are the `solve` and
`report` rows `commands/mod.rs`'s own table has reserved since Phase 1.

```rust
// solve.rs — orchestration
timetable_validate(selection)                       -> AssemblyReportView
timetable_generate_start(selection, settings)       -> GenerateStarted { runId }
timetable_generate_status(runId)                    -> GenerateStatus
timetable_generate_result(runId)                    -> GenerateResult        // outcome + projection
timetable_generate_cancel(runId)                    -> bool
timetable_generate_discard(runId)                   -> bool
timetable_save(runId, name)                         -> SavedTimetableView

// report.rs — viewing
timetable_list(schoolId)                            -> Vec<TimetableSummary>
timetable_open(id)                                  -> TimetableView
timetable_rename(id, expectedRev, name)             -> TimetableSummary
timetable_delete(id, expectedRev)                   -> DeletedTimetableView
timetable_revalidate(id)                            -> TimetableScoring
```

Two properties of this list are deliberate:

- **No `expectedRev` is invented and none is omitted.** `rename` and `delete` take it because
  TIMETABLE-PERSISTENCE §8 says so; `save_generated` does not because there is no previous
  version of a row that does not exist, and the watermark is its precondition. **No handler
  constructs `Expected::Any`** (ADR 0027 §2) — none of these needs to, because the repository
  owns the cascade.
- **No `validate_move`, no `apply_move`, no lock command.** §8.

### 6.3 `TimetableView` — one call per open

```rust
pub struct TimetableView {
    pub timetable: Timetable,                     // the record is the contract — ADR 0012
    pub run: Option<SolveRun>,
    pub evaluations: Vec<TimetableEvaluation>,    // GENERATION and/or REVALIDATION
    pub unscheduled: Vec<UnscheduledView>,
    pub unverified: UnverifiedCounts,
    pub shared: Vec<SharedEntity>,                // named, compiled tier only — C8
    pub projection: Option<TimetableProjection>,  // None iff the Problem could not be compiled
    pub unreadable: Option<String>,               // why, when projection is None — §5.1
    pub not_in_problem: Vec<Uuid>,                // entries the problem no longer holds
    pub findings: Vec<FindingView>,               // stored or explained, per §5.1
}
```

`projection: Option<_>` with a sibling `unreadable: Option<String>` rather than an error: a
timetable whose lessons were deleted is still worth opening, and `arrangement`'s documentation
makes that the house position.

### 6.4 TypeScript types (D4)

ADR 0012 governs: records are the contract, and *"translation earns a type; transcription does
not."* A projection **is** translation — dense indices and a span measure become labelled cells —
so the projection types are earned, and they live where they are computed.

- `evara-report` gains a default-off `ts` feature, exactly as `evara-core` and `evara-db` have,
  with the same comment about never putting a generator in the solver's dependency tree.
- `evara-core::scheduling` gains `ts` derives on the ~20 types that actually cross: `Score`,
  `ConstraintLevel`, `SolveStatus`, `Termination`, `SolveStatistics`, `UnverifiedCounts`,
  `UnplacedCause`, `SpanDefect`, `DiagnosticCode`, `Diagnostic`, `EntityRef`, `EntityKind`,
  `Explanation`, the context facts the axes need, and the `display` types. **Not** `Problem`,
  `Assignment`, `Placement`, or any `*Ix` — a dense index has no meaning outside the problem that
  assigned it, and exposing one would invite the frontend to do arithmetic on it.
- Every new type is added to `typegen.rs`'s `contract!` lists. `check_dependencies_are_listed`
  enforces closure, which is the mechanism that makes C6 a red build rather than a silent gap.
- `npm run types:check` is already in `npm run check` and in CI.

### 6.5 What keeps scheduling logic out of the frontend

Stated as a checklist, because this is the rule most easily broken by a convenience:

| The frontend may | The frontend may **not** |
| --- | --- |
| group `ProjectedLesson`s by `teacherIds`, `roomId`, `sectionId` — a lookup | compute a span, a `rowSpan` from a duration, or an end time |
| render `occupies.length` as a `rowSpan` | decide whether two lessons clash |
| sort on `'HH:MM'` strings and on `ordinal` | parse a time, apply a timezone, or read a weekday |
| show `defect`, `cause`, `level`, `penalty` | decide whether a cause is impossible, or author a violation's sentence |
| remember which day, scope and version are selected | decide which bell schedule a lesson belongs to |

---

## 7. Q7 — Efficiency at ~2 000 activities

### 7.1 What is already measured, and what M8 owes

R5 names three figures. Solve time is measured (900 ms at 2 000 occurrences, 5.0 MiB peak for
the 500 case). Load time has a strong argument and no measurement. **Grid rendering has neither,
and R5 says so: *"grid rendering, which has no grid to render until Phase 7."*** M8 owes both
numbers, as asserted budgets rather than prose — R2's complaint about M6 was precisely that its
figures *"a reader can check and CI cannot"*.

### 7.2 Projection cost

`project()` is `O(activities × duration)` for span resolution plus `O(activities)` for labelling.
`span::resolve_activity_span` is `O(timeslots)` because `slots_of` filters and sorts the whole
timeslot vector — about 60–200 slots for a realistic grid. So 2 000 × 200 ≈ 4 × 10⁵ operations,
which is microseconds-to-milliseconds territory and needs no optimisation.

**One optimisation is proposed and only because it is free:** `project()` builds the
per-`(schedule, day)` slot list **once** into a local index rather than calling `slots_of` per
activity. This is not a second implementation of anything — it is the same `slots_of`, memoised
inside one function — and it turns 2 000 sorts into roughly 40.

### 7.3 IPC and rendering

**One projection per timetable open, not one per view.** ~2 000 `ProjectedLesson`s at roughly
250–350 bytes of JSON is 500–700 KB, serialised once and then indexed in TypeScript. Re-calling
per scope switch would re-serialise the same data five times for the same screen.

| Grid | Cells | Approach |
| --- | --- | --- |
| Teacher / room / section / student | days × periods ≈ 6 × 10 = 60 | plain `<table>`. Nothing to optimise |
| **Master** | days × periods × sections — the only large one | **paginate by year level, then campus**, with a day-at-a-time fallback |

Pagination over virtualisation for the master grid, for two reasons: a virtualised `<table>`
breaks the semantics §9 depends on, and a timetabler reading a master grid reads it a year level
at a time anyway. If a measurement later shows a paginated page is still too large,
virtualisation is an additive change behind the same projection.

`useMemo` on the grouping index, keyed by `(timetableId, scopeKind, scopeId)`. `React.memo` per
row. No per-cell component for an empty cell.

### 7.4 The budgets to assert

| Budget | Where |
| --- | --- |
| `project()` over 2 000 occurrences, 40 rooms, 2 schedules | `evara-report` test, generous ceiling, counted work not wall-clock where possible |
| `Scheduling::load` + `labels` statement count, independent of school size | `evara-db` test, in M4's style |
| One scope grid renders ≤ a stated DOM node count for a 6 × 10 cycle | vitest |
| Master grid page for one year level stays under a stated node count | vitest |

---

## 8. Q8 — Read-only versus editing

**M8 is read-only, and the boundary is drawn in the type system rather than in a convention.**

- **No `apply_move`, no `validate_move`, no lock command, no repair.** SAD §6 lists manual
  editing, `validate_move`, locks and repair among what is *"not in the MVP"*, and ROADMAP
  Phase 8 owns them. `PlacementVerdict` already exists and is exactly what `validate_move`
  returns — M8 does not expose it, and ROADMAP Phase 5's *"no Tauri handler exposes it yet"*
  stays true.
- **`is_locked` and `is_manual` are rendered and never written.** They are columns the generator
  writes as 0 (TIMETABLE-PERSISTENCE §12). A badge is drawn if one is ever 1, so that when
  Phase 8 starts writing them the view already tells the truth.
- **No drag handlers, no drop targets, no hover validation.** Not disabled — absent. A disabled
  affordance teaches a user that editing exists here, and in M8 it does not.
- **The only writes in the whole milestone are four:** `save_generated`, `rename`, `delete`,
  `record_revalidation`. Everything else goes through `Scheduling` (`&Database`, so the
  borrow checker forbids a transaction) or through a `Timetables` read.

What M8 *does* build that Phase 8 will consume: the grid, the cell identity, the projection, the
findings panel and the explain path. A drop target is then a prop on an existing cell rather
than a new screen.

---

## 9. Q9 — Accessibility, empty states, errors, layout

### 9.1 Accessibility — the `AvailabilityGrid` precedent, extended

That component's header already states the house position and the reason: *"A grid of `<div>`s
with click handlers would have been fewer lines and unusable by half the people who have to
configure a school."* M8 follows it and adds what a timetable needs:

- A real `<table>` with `<caption>`, `scope="col"` on day headers and `scope="row"` on period
  headers. A multi-period lesson is one `<td rowSpan>`, which is what makes a screen reader
  announce it once rather than three times.
- Every occupied cell carries the full sentence, because position is not available to someone
  hearing it: *"Day A, periods 2 to 4, 09:50 to 12:05 — Mathematics, 9 MAT-B, Amara Okafor,
  Room B12. Three periods."*
- Roving tabindex with arrow keys across cells and across the day strip; `Home`/`End` to the
  ends of a row. Enter opens the lesson detail panel; there is nothing to toggle.
- Progress is an `aria-live="polite"` region updated at the coalesced rate, not per event, so a
  screen reader is not flooded. Cancellation is a button, focusable throughout the run.
- Findings are a list with `aria-describedby` from each cell that has one, so a violation is
  reachable from the lesson and the lesson from the violation.
- Colour never carries meaning alone: a hard violation is an icon and text as well as a tint; a
  span defect is a pattern as well as a colour.

### 9.2 Empty states that name the fix

The precedent is `AvailabilityGrid`'s: *"being told 'create one under Cycles and cycle days' is
better than a blank panel, and being taken there is better still."* Each state names the section
and offers to open it.

| State | Says |
| --- | --- |
| No school | Create one under **Schools** |
| No cycle | This school has no cycle — **Cycles and cycle days** |
| No materialised grid | The grid has not been built — **Timetable grid preview** |
| No lessons | Nothing to schedule — **Lessons** |
| Grid built, no timetable | **Generate** a timetable for this cycle |
| Timetable with zero entries | Saved and empty: nothing was placed. The unplaced list and the causes |
| Blocking pre-solve findings | Generation is unavailable; the findings, with remedies |

### 9.3 Errors

`SetupError` gains three variants (D8), each with a remedy rather than a restatement:

| Variant | Maps | Message |
| --- | --- | --- |
| `InputsChanged` | `DbError::InputsChanged` | *"A scheduling input changed since this timetable was generated… Generate again."* + the button |
| `NotSaveable` | `DbError::NotSaveable` | the two cases are different: inputs refused → the findings; internal check failed → **this is a defect in Evara**, with what to report |
| `ReadOnlyProject` | `DbError::ReadOnlyProject` | the project was opened for inspection |

`Termination::InternalCheckFailed` is the one that must not be softened. TIMETABLE-PERSISTENCE §9
and `generate`'s own documentation both call it a defect in Evara, *"never a timetable returned
as a success"*, and the screen says that in those terms.

### 9.4 Layout

A two-pane desktop layout: a versions/scope rail and the grid, with the rail collapsing below a
breakpoint. The grid itself scrolls in its own `overflow-x: auto` container with sticky period
and day headers — the page body never scrolls horizontally. Print is **out of scope** (Phase 10,
D10), and no print stylesheet is proposed; the grid is built so one can be added without
restructuring it.

---

## 10. Decisions requiring approval

| # | Decision | Recommendation |
| --- | --- | --- |
| **D1** | The brief's M8 merges SAD §6's M8 (orchestration) and M9 (review) — C1. | **Accept the merge, deliver as two sequenced stages on one branch: 8A orchestration, then 8B viewing.** 8A is what TIMETABLE-PERSISTENCE §13 lists as *owed by M8*, and nothing in the interface can produce a timetable to view until it exists. If the branch has to be cut short, 8A alone is shippable and 8B alone is not. |
| **D2** | Where the display labels live — C2. | **`evara_core::scheduling::display::LabelSet`**, populated by a new opt-in `evara-db` read, consumed by `evara-report`. `Problem` is **not** widened (ADR 0037 §2). Rejected: labels on `Assembly` (makes every solve pay), labels in `evara-report` (`evara-db` could not populate them). |
| **D3** | Whether a timetable can be viewed without compiling a `Problem` — C3. | **No. A grid always compiles one.** It is cheap, and the alternative is a second span implementation. The no-`Problem` path exists only for *failure*, and shows findings and metadata with the reason the grid is absent — never a partial grid. |
| **D4** | Which types cross the IPC boundary — C6. | **Purpose-built projection types in `evara-report` behind a `ts` feature, plus `ts` derives on ~20 `evara-core::scheduling` types.** Never `Problem`, `Assignment`, `Placement` or any `*Ix`. |
| **D5** | Whether to build the by-student projection — C4. | **Build it, from `class_enrolment`, and label it as a roster view rather than a scheduling one.** It must state that a section's audience is its student groups and that the individual roster has no scheduling meaning in M1 (CLAUDE.md §3, ADR 0006). An empty roster shows the groups' timetables instead. Alternative, if this reads as overreach: defer it and keep the four group-based scopes. |
| **D6** | Stored versus explained findings — C5. | **Two tiers, §5.1.** The stored tier is the fallback and the compiled tier is the ordinary case; neither claims the other's fidelity. |
| **D7** | What the Generate screen does after a save — M7 §13 item 3. | **Open the new timetable**, with the list showing it as newest. No *"current timetable"* pointer is introduced. |
| **D8** | Three new `SetupError` variants — C7. | **Add them.** `ReviewRequired`'s remedy is *re-preview*; `InputsChanged`'s is *generate again*, which §13 item 4 requires by name. |
| **D9** | **Does `evara-report` gain an `evara-constraints` dependency** so it can call `explain()`? This is a change to CLAUDE.md §2's dependency table and needs an explicit yes. | **Recommended: yes, `evara-report -> core, constraints`.** `explain()` is the registry's and *"that wording lives with the rule so it cannot drift from the check that produced it"* — a report that composed its own sentence would be the drift that rule exists to stop. `evara-constraints` has no I/O, so the edge adds nothing `evara-report` should not have. **Alternative if the edge is refused:** the command layer calls `evaluator_for(t).explain()` and hands `Vec<Explanation>` to `evara-report` as data — which keeps the table unchanged at the cost of orchestration doing a join. A third option, letting `evara-report` write the sentences, is **rejected outright** as a second implementation of every rule's wording. |
| **D10** | Where the new frontend code lives. `src/features/` and `src/ui/` exist and are **empty**; everything is under `src/app/setup/`. | **`src/app/timetables/`**, following the established `src/app/<area>/` shape, with shared grid primitives in `src/app/ui/`. Rejected: populating `src/features/` for one area, which would leave the codebase with two conventions and one example of each. |
| **D11** | Top-level navigation. The shell is today *start screen* or *School Setup* with no way to be anywhere else. | **Add a two-item workspace rail — School Setup · Timetables — inside the open-project shell**, with the selected school hoisted so both areas share one answer. The school selector must stay a single source of truth; ADR 0023 and `SchoolSetup`'s `key={activeSchoolId}` remount are the precedent, and the remount guarantee must extend to the new area. |
| **D12** | Whether `evara-report` gets its renderer trait now. | **No.** ROADMAP Phase 10 owns HTML/CSV/PDF. A trait with one implementation and no second caller is a guess about the second; the projections are designed so Phase 10 adds renderers without touching them, which is what the crate's own header already promises. |

---

## 11. Migration and documentation

**No migration.** `V17` holds everything M8 reads. M8 adds no table, no column, no trigger and no
index, and therefore no entry to `change_log::TRACKED_TABLES`. If that changes during
implementation it is a signal that scope has moved and the proposal should be revisited rather
than a migration quietly added.

Documents to write or correct on completion:

| Document | Change |
| --- | --- |
| `docs/TIMETABLE-VIEWS.md` | **new.** What the projections are, the two fidelities, the axis-per-bell-schedule rule, the measured figures |
| `docs/adr/0043-…` | **new**, if D9 is approved: the `evara-report -> constraints` edge, and why a report never authors a rule's wording |
| `docs/adr/0044-…` | **new**, if D2 is approved: the display vocabulary sits in `evara-core`, and `Problem` is not widened for a report |
| `CLAUDE.md` §2 | the dependency table, if D9 is approved |
| `CLAUDE.md` §3 | a bullet for the viewing rules: one axis per bell schedule; a grid always compiles a `Problem`; the frontend groups and never resolves |
| `ROADMAP.md` | Phase 6's *"still to do"* reduced to `evara bench`; Phase 7 marked complete with its departures |
| `TIMETABLE-PERSISTENCE.md` §12–§13 | the four *"owed by M8"* items discharged, and the *"no Tauri command"* limitation removed |
| `OPEN-DECISIONS.md` R5 | grid rendering and load time, measured |
| `ARCHITECTURE.md` §2 | `evara-report` is no longer *"at the edges"* and empty |
| `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 | the M8/M9 merge, recorded rather than left as a silent divergence |

---

## 12. Test strategy

Baseline on this branch: **1 463** Rust `#[test]`s, **44** vitest files. The counts below are
new tests, and every one of them is named for a behaviour rather than a function.

### `evara-report` (new crate, ~70 tests)

| Group | Covers |
| --- | --- |
| **Projection correctness** | every placement appears exactly once per scope it belongs to; a co-taught lesson is on both teachers' grids; a lesson with no room has no room grid entry and still has a section one; the ungrouped set equals the union of the grouped ones |
| **Multi-period spans** | `d = 1…4`; `occupies.len() == d` for a sound span; `starts_at`/`ends_at` are the **first and last period's**, never a computed product; `rowSpan` data matches the resolver's answer and is never recomputed |
| **Span defects** | each of the five `SpanDefect` fields surfaces; a short run renders short and is **marked**, never silently truncated; `!is_sound()` is always reported |
| **Cross-campus bell schedules** | two campuses, two schedules → **two axes**, never a merged one; a teacher on both gets two grids; two lessons at overlapping minutes under different `timeslot_id`s appear at their own schedules' rows |
| **Cycle kinds** | `WEEKLY` and six-day `CUSTOM` take the same path; no projection output varies with `weekday_hint`; a day with no lessons is a column, not an omission |
| **Labels** | a missing label degrades to a stated fallback rather than panicking; `short` is composed once |
| **Determinism** | same inputs → byte-identical projection, asserted as whole-value equality |
| **Budget** | 2 000 occurrences / 40 rooms / 2 schedules under a stated ceiling — §7.4 |

Driven by M2's fixture catalogue, including the `medium` 500-occurrence one, plus the 2 000
upper-end school built in the test that measures it (SCHEDULING-FIXTURES' precedent).

### `evara-db` (~15 tests)

- `labels` returns one entry per section in scope and **none from another school**; a section of
  another school is absent, not `WrongSchool`-adjacent.
- `labels` statement count is independent of school size, in M4's style.
- `for_cycle` filters by cycle and by `COALESCE`d term.
- **School isolation**, restated for the new reads: a cycle of another school refuses;
  `for_school` never returns another school's timetable.
- `Assembly`, `Problem` and `CompiledConstraint` are `Send` — the compile-time assertion of §3.1.

### `apps/desktop/src-tauri` (~35 tests)

| Group | Covers |
| --- | --- |
| **Generation lifecycle** | start → status → result → save → open; a session is dropped on save and on discard |
| **Cancellation** | a cancelled run reports `Termination::Cancelled`, is **not saveable as complete**, and the session survives for inspection; cancelling an unknown `runId` is `false`, not an error |
| **The lock is not held across a solve** | a second command completes while a generation is running — asserted, because this is the design's load-bearing claim |
| **Staleness** | an input moved between generate and save → `InputsChanged`, nothing written, and the error carries the *generate again* remedy |
| **Refusals mapped, not interpreted** | each of TIMETABLE-PERSISTENCE §9's eight refusals reaches a distinct `SetupError` |
| **Revisions** | `rename` and `delete` with a stale `expectedRev` → `Conflict`; **no handler constructs `Expected::Any`**, asserted over the two new modules |
| **Read-only** | no handler in `report.rs` reaches a write; `timetable_open` on a project opened for inspection still renders |
| **Degraded open** | a timetable whose cycle was deleted returns `projection: None` with `unreadable` set, the stored findings present, and **no grid** |
| **Persistence round trip** | save → reopen the file → `timetable_open` → the projection equals the preview's. The UI-level counterpart of M7's most valuable test |

### Frontend (~22 vitest files)

- Grid renders days × periods from the axes; a multi-period lesson is **one** `<td>` with the
  right `rowSpan`; a break row is present and marked unavailable.
- Two axes render two captioned grids and the sentence that they cannot be read as one week.
- Scope switching re-indexes without a second IPC call.
- **Accessibility**: table semantics, `scope` attributes, per-cell sentence, arrow-key roving,
  `aria-live` progress, focusable cancel, no colour-only meaning. Asserted, not reviewed.
- Progress events update a bar and are coalesced; `finished` settles the screen.
- Unverified counts render all four **including at zero**; the shared-entity panel names schools
  in the compiled tier and says *"open to see which"* in the stored tier.
- Hard violations, partial, and unscheduled render as **three** distinct states.
- `is_impossible()` gates the word *impossible*; an unrecognised cause is phrased as such.
- Every empty state of §9.2 names its section and offers to open it.
- `InputsChanged` renders the remedy and the button; `InternalCheckFailed` renders as a defect
  in Evara.
- School switch remounts the Timetables area (D11's extension of the existing guarantee).
- Rendering budgets of §7.4.

### `evara-cli`

No new subcommand. `evara evaluate --timetable` already covers the headless read. If projection
needs a headless exerciser for the budget test, it is a test in `evara-report` rather than a
command — a user-facing command exists for a user, and nobody has asked for this one.

---

## 13. Implementation sequence

Nine steps. Each ends green, and the first five are stage 8A.

| # | Step | Deliverable |
| --- | --- | --- |
| 1 | **The vocabulary.** `LabelSet` in `evara-core`; `Scheduling::labels` and `for_cycle` in `evara-db`; the `Send` assertions | Rust tests. Nothing visible |
| 2 | **The projection.** `evara-report::projection::project`, calling `resolve_activity_span` and restating nothing; the fixture-driven correctness suite and the budget | `evara-report` goes from 13 lines to a crate |
| 3 | **The solve orchestration.** `commands/solve.rs`, `GenerateSessions`, the worker thread, the coalescing `ProgressSink`, the `AtomicBool` `Cancel`, the three `SetupError` variants | Generation reachable over IPC; no UI |
| 4 | **Types across.** `ts` feature on `evara-report`, `ts` derives on the ~20 core types, the `typegen.rs` contract entries | `npm run types:check` green with the new surface |
| 5 | **The Generate screen.** Selection, pre-solve findings, progress, cancel, preview, explicit save, the `InputsChanged` remedy. **Stage 8A ends here** | A timetable can be produced and saved from the application — M7 §13 discharged |
| 6 | **The viewing commands.** `commands/report.rs`: list, open, rename, delete, revalidate | Rust tests only |
| 7 | **The grid.** `src/app/timetables/`, the shared grid primitives, the four scopes, day navigation, the accessibility suite | The grid renders a saved timetable |
| 8 | **Findings and disclosure.** The two fidelities, the score pair, the unverified panel, the named shared entities, the unplaced list | ADR 0032 test 12 discharged |
| 9 | **Measure and record.** The rendering and load budgets; `TIMETABLE-VIEWS.md`; the ADRs D2 and D9 earn; the eight documents of §11 | R5's remaining figures measured |

`npm run check` after each step, run sequentially.

---

## 14. What this proposal deliberately excludes

Named so none of it is read as an oversight.

| Excluded | Owner |
| --- | --- |
| Manual moves, drag-and-drop, hover validation, `apply_move`, `validate_move`, undo/redo | ROADMAP Phase 8, SAD §6 |
| Locking and unlocking an entry; repair; partial regeneration; `minimise-disruption` | Phase 8, `SOLVER.md` §7 |
| Print, print stylesheets, PDF, CSV, the renderer trait | Phase 10, D10, **D12** |
| `.evara` export of a timetable | Phase 9, `PROJECT-FORMAT.md` §3 |
| The constraint configuration screen | Phase 4's UI half |
| Local search, soft constraints, `evara bench` and committed score baselines | Phase 6's remainder, R2 |
| A published/archived lifecycle, `based_on_timetable_id`, a *"current timetable"* pointer | SAD §6, ADR 0042 D3 |
| Cross-school comparison of any kind | ADR 0032 §4 — the warning is the whole of what is possible |
| Individual student sectioning as a scheduling population | SAD §7, ADR 0029 §4 |
| Any new migration | §11 |
