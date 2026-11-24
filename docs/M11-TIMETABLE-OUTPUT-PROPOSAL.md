# M11 — Timetable Output · Architecture proposal

**Status:** proposal, awaiting review. Nothing in it is implemented.
**Phase:** 10 — Output. **Closes:** M1-19.
**Branch:** `docs/m11-timetable-output-proposal`. **Base:** `origin/develop` at `b12fdb2`
(PR #50, M10 Stage 6, merged 2026-11-24).

---

## How to read this document

Four labels are used throughout and they are not decoration. The brief asks that assumptions
are never presented as verified facts, and the print-pipeline section in particular is a place
where that distinction decides the plan.

| Label | Means |
| --- | --- |
| **EXISTS** | in `develop` today, with a file and line reference |
| **PROPOSED** | this document's recommendation; not built |
| **VERIFIED** | checked against source or vendor documentation, with the source named |
| **ASSUMPTION** | believed, with a reason, and **not** yet proven — each one has a named way to settle it |

Where a figure is quoted it is either a measurement already in the record (with its source) or
marked as a target to be measured. No new measurement is claimed in this document, because no
code was written for it.

---

## 1. Executive summary

M11 gives a timetable three ways out of the application: a machine-readable **CSV**, a
human-readable **Excel workbook**, and a **printed page or PDF** through the platform's own
print pipeline. It adds bulk output so a school can produce every teacher's timetable without
opening every teacher's timetable. It adds no scheduling semantics, no solver behaviour and no
schema migration.

Seven findings shape the plan, and three of them change it materially.

1. **The projections are ready and are the source of truth.** `evara_report::projection`
   already produces a well-formed grid per bell schedule per scope, with `rowSpan` decided in
   Rust, conflicts named, breaks marked and broken runs carried rather than discarded. M11 is a
   second consumer of that output, not a second producer of it. (§2.1)

2. **There is no campus-level projection scope, and there should not be one.**
   `ScopeKind` is `Master | Section | Teacher | Room | Student`. A campus is not a scope — it is
   an *axis*: `GridPanel::campus_name` names the campus a bell schedule serves, and a scope
   touching two schedules gets two panels. The brief's "campus-level timetable views" is
   satisfied by the **master grid of a school, panelled by bell schedule**, which is what the
   existing model already says. Adding a campus scope would require merging two campuses'
   Period 3 into one row, which ADR 0033 forbids. (§3.1, §9)

3. **`window.print()` is silently denied on macOS in this repository today.** Not "may behave
   differently" — denied. Tauri 2.12.1 replaces `window.print` on macOS with
   `invoke('plugin:webview|print')`, which requires `core:webview:allow-print`;
   `core:webview:default` grants four permissions and that is not one of them, and
   `capabilities/default.json` asks for `core:default` only. One permission line fixes it, and
   it routes to a real `NSPrintOperation` panel in `wry`. This is **VERIFIED from the vendored
   source of the exact versions this repository pins**, and it is the single most useful thing
   this inspection found. (§7)

4. **Bulk student output is quadratic today, and that is a real blocker rather than a
   performance note.** `projection::in_scope` answers *"is this student in this lesson?"* with a
   linear scan of `LabelSet::students` per lesson, so laying out one student's grid is
   `O(students × lessons)` and laying out every student's is `O(students² × lessons)`. For 3 000
   students and 2 000 lessons that is ~10¹⁰ comparisons. Stage 1 must add an index; the fix is
   small and local. (§14.2)

5. **No renderer should be a single trait across all three formats.** CSV consumes a *row
   stream*; Excel consumes a *grid with merges*; print consumes a *document in a browser*. What
   they genuinely share is one level lower: the already-composed text of a cell. So the shared
   artefact is an **`OutputDocument`** built once in Rust, and the trait is declared only over
   the two Rust byte-producing renderers. The print view is a React consumer of the **same
   document over IPC**, which is what makes the screen, the workbook and the page agree about
   what a lesson is called. (§4, §5)

6. **No ZIP, no per-entity files, and therefore no generated filenames.** Bulk output is one
   document: a multi-sheet workbook, a long-form CSV, a multi-page printed document. The user
   names that one file in a native dialog. That removes the whole Windows-reserved-name,
   duplicate-name and Unicode-filename class of problems rather than defending against it — the
   same move M10 made when it decided an archive is read by exact name. (§8, §10)

7. **Nothing here needs a session, a worker registry or a cancellation protocol — probably.**
   The projection of the realistic upper end measures 3.8 ms and a scope grid is 40 cells
   whatever the school's size (R5, `TIMETABLE-VIEWS.md` §9). Rendering is a traversal of that.
   Stage 2 therefore ships a **synchronous** command with the workspace lock released before
   rendering, and carries one explicit measurement gate: if a full-school bulk workbook exceeds
   **1.5 s**, Stage 4 adds a session on the pattern `TransferSessions` already establishes. The
   brief forbids introducing asynchronous infrastructure that measurement does not justify, and
   this is how that is honoured without guessing. (§14)

The dependency assessment adds **nothing** for CSV and Excel (`csv` and `rust_xlsxwriter` are
already in the workspace and in the lockfile) and **one optional** official Tauri plugin for
printing convenience, which §13 recommends against for M11.

---

## 2. Existing architecture findings

Every claim in this section is **EXISTS**, with a reference.

### 2.1 `evara-report` is where output belongs, and it was shaped for this

`crates/evara-report/src/lib.rs:40-44` says so in as many words:

> The renderer trait and the HTML, CSV and PDF backends are ROADMAP Phase 10. A trait with one
> implementation and no second caller is a guess about the second, so the projections are shaped
> to be rendered without being shaped by a renderer — which is what the original header promised
> and what Phase 10 collects on.

The crate's layering contract (`lib.rs:5-8`) is *"receives data; does not query for it. No
SQLite, no file, no network."* `Cargo.toml` depends on `evara-core`, `evara-constraints`,
`serde`, `serde_json`, `thiserror`, `uuid`. Its package description already reads *"Timetable
view projections and output renderers (HTML, CSV, PDF)."*

That contract has one consequence M11 must respect: **`evara-report` cannot write a file.** A
renderer produces bytes, or writes into a caller-supplied `impl Write`. Choosing a destination,
claiming it and renaming onto it belongs to the caller. §10 builds on that rather than arguing
with it.

### 2.2 What a projection already answers

`crates/evara-report/src/projection.rs`. `project(ProjectionInput)` → `TimetableProjection`,
then `lay_out(&projection, &scope, &labels)` → `ScopeGrid` per selected entity.

| Type | Carries | Line |
| --- | --- | --- |
| `TimetableProjection` | `school_name`, `cycle_name`, `term_name`, `days`, `axes`, flat `lessons`, `scopes`, `has_enrolments` | `495` |
| `ScheduleAxis` | `schedule_id`, `schedule_name`, `campus_id`, `campus_name`, `periods` | `524` |
| `ProjectedPeriod` | `period_id`, `ordinal`, `label`, `starts_at`, `ends_at`, **`is_teaching`** | `65` |
| `ProjectedDay` | `cycle_day_id`, `ordinal`, `label` — and **no weekday** | `87` |
| `ProjectedLesson` | section label, `activity_name`, `teacher_ids`+`teachers`, `group_ids`+`groups`, `room_id`+`room`, `schedule_id`, `cycle_day_id`, `occupies`, `starts_at`, `ends_at`, `duration_periods`, `defect`, `flags` | `129` |
| `ScopeGrid` | `scope`, `panels` (one per bell schedule touched), `conflicts`, `defective` | `468` |
| `GridPanel` | `schedule_id`, `schedule_name`, `campus_name`, `days`, `periods`, `cells` row-major | `306` |
| `CellContent` | `Empty` \| `Lesson { lesson, rows }` \| `Continuation` \| `Pinned { lessons, reason }` | `261` |
| `PinReason` | `Conflict` \| `Shared` \| `BrokenSpan` | `239` |

`GridPanel::is_well_formed` (`345`) and `malformations` (`354`) assert the table invariant, and a
test runs them over every fixture. **A renderer that trusts `rowSpan` and `Continuation` cannot
produce a broken table**, which is precisely what the model was built to guarantee.

### 2.3 What a projection does *not* carry, and what that costs M11

Four gaps, each with a consequence for output.

1. **Unplaced occurrences are absent.** `project` skips any activity with no placement
   (`projection.rs:629-631`). What could not be placed lives in
   `OutcomeView::unscheduled` / `TimetableView::unscheduled`
   (`apps/desktop/src-tauri/src/commands/views.rs:142`, read from `solve_run`'s stored rows).
   An exported timetable that showed only the grid would **silently omit the lessons that have no
   place**, which §4.4's rule against silent omission forbids. The `OutputDocument` must carry
   them as a section of its own, with `is_impossible` and `diagnostic_code` as stored.

2. **A scope with no lessons has no panels.** `lay_out` skips an axis whose lesson list is empty
   (`projection.rs:863-866`), so a teacher who teaches nothing gets `panels: []`. On screen
   that is an empty area; in a bulk workbook it would be a **missing worksheet**, which reads as
   "this teacher was not exported" rather than "this teacher has no lessons". The document model
   must distinguish *empty* from *absent* explicitly. (§8.4)

3. **One room per occurrence, several teachers per occurrence.**
   `Placement { timeslot, room: Option<RoomIx> }`
   (`crates/evara-core/src/scheduling/placement.rs:38-51`) — a single optional room. Teachers are
   a `Vec` on the section. So "multiple rooms associated with an activity" is **not representable
   today**, and M11 must not invent a representation for it. "Multiple teachers" is, and both CSV
   and Excel must carry all of them rather than the first. (§4.3)

4. **Resources are not in the problem.** `setup_resource_*` commands exist and
   `crates/evara-core/src/scheduling/problem.rs:395` records that *"room candidate lists and
   resource requirements are not in"* the compiled problem. So a timetable cannot be exported
   with its resource bookings, because there are none to export.

### 2.4 The display vocabulary is already the one a report needs

`crates/evara-core/src/scheduling/display.rs`, added by ADR 0044 for exactly this reason: a field
whose only consumer is a report does not widen `Problem`.

- `SectionLabel { subject, course, section, year_level, campus, short }`, with
  `compose_short(year_level, subject, section)` as **the one implementation** of a cell caption
  (`display.rs:78-91`).
- `RoomLabel { name, building }`.
- `StudentLabel { student_id, name, code, sections }` — straight from `class_enrolment`, and
  nothing else. `has_enrolments()` is what a screen asks before offering the view at all.

The module header states the rule M11 inherits: *"Nothing in this module is read by a constraint,
a solver or a diagnostic… If a label is missing the grid says so and renders; it never
substitutes a default that could be mistaken for data."*

### 2.5 How the desktop reaches a timetable today

`apps/desktop/src-tauri/src/commands/views.rs`.

- `timetable_open(id)` → `TimetableView` (`views.rs:362`). Reads the row, its evaluations, its
  run, its stored unscheduled rows and its stored violations; resolves the school **through the
  cycle** because a timetable carries no `school_id`; then `compile()` loads an `Assembly`, a
  `LabelSet`, the arrangement and the entry flags, and projects.
- `timetable_scope(id, scope_kind, scope_id)` → `ScopeGrid` (`views.rs:674`). **Re-loads and
  re-projects** rather than caching, with the reason given in the doc comment: a load is a fixed
  statement count and a projection is single-digit milliseconds, so a cache would buy little and
  introduce a lifetime nothing else in the crate has.
- `Scheduling::load` / `labels` / `entry_flags`
  (`crates/evara-db/src/scheduling/mod.rs:311-326`), read-only.

The second bullet is the one M11 must *not* copy. A bulk export of 400 teachers through
`timetable_scope` would compile the problem 400 times. §4.2's whole shape follows from that.

### 2.6 Spreadsheet writing already exists, and its lesson is already learned

`crates/evara-import/src/template.rs` writes the ten downloadable import templates.
`rust_xlsxwriter 0.102` with `default-features = false` is in the workspace
(`Cargo.toml:146`), `csv = "1"` is at `Cargo.toml:137`, and both are used only by
`evara-import` today.

`template.rs:201-255` is the pattern to follow: a `Format` per role, `set_column_width`,
`set_freeze_panes(1, 0)`, a second worksheet of instructions, `save_to_buffer()` — bytes, not a
path, which is what an IPC command and a CLI both want. And `template.rs:17-23` records the
reason the templates are workbooks rather than CSVs, which M11 inherits in mirror image: a code
column is written as text (`set_num_format("@")`) because `00124` must survive a round trip
through a school's copy of Excel.

### 2.7 Branding exists and a Rust renderer can use it directly

`setup_school_logo_get/set/clear/limits`
(`apps/desktop/src-tauri/src/commands/structure.rs:199-258`), over
`evara_db::structure::SchoolLogo { bytes, media_type, width, height, updated_at }` and
`evara_core::branding` for the format sniffing and limits.

`structure.rs:132-133` anticipates M11 exactly: *"A future report or export in Rust uses
[`SchoolLogo`] directly and never sees this type; raw bytes are what those want anyway."*
`rust_xlsxwriter::Image::new_from_buffer` takes bytes, and the sniffed media type is already one
of the formats it recognises. For the print view, `SchoolLogoView::data_url` already produces a
`data:` URL, and the window CSP admits `img-src 'self' data:`.

### 2.8 Progress, cancellation and destination safety are all solved — elsewhere

| Device | Where | Reusable for M11? |
| --- | --- | --- |
| `Coalescer` / `CoalescingSink` (50 ms floor) | `commands/session.rs:644-744` | Yes, if a session is needed at all (§14.4) |
| `Session` / `Lifecycle` state machine, `CancelFlag` | `commands/session.rs:114-630` | Yes, same condition |
| `TransferSessions` registry, `transfer://` events | `commands/transfer_session.rs` | The pattern, if needed |
| `DestinationClaim` (`create_new(true)`) | `crates/evara-project/src/claim.rs` | **Yes, directly** (§10.1) |
| `require_safe_destination`, `rename_with_retry` | `crates/evara-project/src/paths.rs:92,166` | Yes, from the command layer |
| Safe **filename derivation** | — | **Does not exist.** M10 never generated a filename; the user always chose one |

### 2.9 Two statements in the accepted record that M11 corrects

**`OPEN-DECISIONS.md` D10 understates the problem.** It frames webview printing as *"limited
control over pagination and page headers"* and flags that it *"may not survive contact"* with
repeated headers. The real first obstacle is smaller and harder: on macOS the call is **denied
by the capability set** before pagination is reached (§7.2). D10's recommendation survives; its
risk statement should be amended to name the permission and the `respondsToSelector` silent
fallback.

**`ROADMAP.md` Phase 10 asks for "a print stylesheet per view".** Per *view* is the wrong unit.
The five scopes differ in what a cell says and in nothing about the page; what genuinely differs
per *panel* is the axis. One stylesheet with a per-scope modifier class is the shape; five
stylesheets would be four copies of the same `@page` rules. Recorded as a departure in §16.

---

## 3. Product scope

### 3.1 Views — the real list, after inspection

The brief asks for the supported list to be confirmed against the projections rather than
assumed. It is:

| Brief's ask | What exists | M11 |
| --- | --- | --- |
| Teacher timetables | `ScopeKind::Teacher` | ✅ in scope |
| Individual student timetables | `ScopeKind::Student`, **roster-derived** | ✅ in scope, with the caveat carried into every output |
| Class and class-section timetables | `ScopeKind::Section` | ✅ in scope. Note: **a "class" in Evara is a class *section*** — there is one scope kind, not two |
| Room timetables | `ScopeKind::Room` | ✅ in scope |
| Campus-level views | **no such scope** | ✅ satisfied as the **master grid, panelled per bell schedule**, each panel naming its campus |

Two things follow from the last row and should be read as answers rather than omissions.

**A campus is an axis, not a scope.** `ScheduleAxis::campus_name` and `GridPanel::campus_name`
exist; `ScopeKind` has no `Campus`. A school's master grid already contains one panel per bell
schedule, and a bell schedule names at most one campus
(`crates/evara-db/src/scheduling/*`, `ADR 0033`). So "the Northgate timetable" is *the master
grid's Northgate panel*, and it is already produced. What M11 adds is the ability to
**select** panels for output, which is a filter over `ScopeGrid::panels` and not a new scope.

**A whole-project view across schools is out of scope, with a reason.** A `Problem` is compiled
for one `SchedulingSelection { school_id, cycle_id, term_id }`
(ADR 0032, `SCHEDULING-PROBLEM-ASSEMBLY.md`). Two schools are two problems, two cycles and two
sets of timeslot rows over the same minutes. Merging them into one document would need the
merged axis ADR 0033 refuses. §9.4 proposes the honest alternative.

### 3.2 What is deliberately not in M11

Out of scope by the brief, restated so no stage quietly adopts one: cloud or sharing, accounts,
email, calendar integration, vendor formats, new solver work, individual student sectioning,
substitutions and absence, a report designer, packaging. Added by this document:

- **No multi-room representation.** §2.3 finding 3 — there is nothing to render.
- **No resource bookings.** §2.3 finding 4 — same reason.
- **No Rust vector-PDF renderer.** D10 keeps it as a later refinement; §7.6 states the condition
  under which M11 would have to reopen that, and §19 prices the alternative.
- **No per-entity file output and no ZIP.** §8.2.
- **No cross-campus merged timetable.** §3.1, §9.4.
- **No new constraint type, no migration, no schema change.** M11 reads V18 and writes files.

---

## 4. Proposed architecture

### 4.1 One sentence

`evara-report` gains an `output` module that turns a projection into a format-independent
**`OutputDocument`**, and two renderers that turn that document into CSV bytes and XLSX bytes;
the print view is a React page that consumes the same `OutputDocument` over IPC; the command
layer and the CLI own destination choice, claiming and writing.

```
                        ┌─────────────────────────────────────────┐
  evara-db              │ evara-report                            │
  ────────              │ ─────────────                           │
  Scheduling::load   ──►│ projection::project   (EXISTS)          │
  Scheduling::labels ──►│        │                                │
  timetables::        ──►│        ▼                                │
    arrangement        │ projection::lay_out   (EXISTS, per scope)│
  structure::         ──►│        │                                │
    school_logo        │        ▼                                │
                        │ output::compose  ───►  OutputDocument   │   PROPOSED
                        │                            │            │
                        │          ┌─────────────────┼──────────┐ │
                        │          ▼                 ▼          │ │
                        │  output::csv        output::xlsx       │ │   PROPOSED
                        │  (Vec<u8>)          (Vec<u8>)         │ │
                        └──────────┼─────────────────┼──────────┬┘
                                   │                 │          │
            ┌──────────────────────┴─────┬───────────┘          │ OutputDocument
            ▼                            ▼                      ▼  over IPC
   evara-cli `report`          src-tauri commands        React print page
   (claims + writes)           (claims + writes)         (window.print())
```

Three properties this shape buys, each of which is the reason for it:

- **One projection per document, however many entities.** The command compiles once, projects
  once, lays out per selected entity from that one projection, and composes one document. §2.5's
  re-projecting read path is not on this route.
- **One implementation of a cell's text.** Composed in `output::compose`, consumed by CSV, by
  XLSX and by the print page. The screen's `TimetableGrid.tsx` composes the same three parts
  today (`TimetableGrid.tsx:517-526`); §5.6 proposes it move to reading `CellLines` so the
  screen cannot drift from the page either.
- **`evara-report` still writes no files.** The renderers return bytes. §10 is therefore a
  command-layer concern, which is where `DestinationClaim` already is.

### 4.2 The composition pass, and why it is not in a renderer

`output::compose` is where every judgement that a renderer would otherwise make separately is
made once:

1. Which panels, days and periods are in the document (the options, §6.4).
2. What each cell's lines say — the section caption, the teachers, the room — and in what order.
3. How a `Pinned` cell is described: a clash, several lessons, or a broken run, named by
   `PinReason` and **never collapsed** (§4.4).
4. Which lessons a merged Excel range may cover: straight from `CellContent::Lesson { rows }`,
   never recomputed.
5. The header block: school, campus, cycle, term, timetable name, version provenance, logo
   presence, the generated-at stamp supplied by the caller.
6. The appendices: unscheduled occurrences, conflicts, defective runs, findings, and the
   cross-campus disclosure (§9.3).

A renderer after that pass is a traversal with no decisions in it, which is what makes "the same
selection produces consistent data across the view, CSV, Excel and printed output" a structural
property rather than a test that passes today.

### 4.3 Timetable semantics — the twelve cases the brief lists, and how each is represented

**EXISTS** unless marked. This table is the answer to §4.3 of the brief, and the rule under it is
that none of these is made easier by changing scheduling semantics.

| Case | Representation today | What output does |
| --- | --- | --- |
| **Weekly and custom rotation cycles** | `CycleKind::{Weekly, Custom}` (`context.rs:144`); the column axis is `ProjectedDay` in `ordinal` order and carries **no weekday** | Columns are the cycle's days by `ordinal` and `label`. Nothing reads a weekday. A `Custom` cycle of 10 days is 10 columns, not two weeks |
| **Cycle day ordering** | `ProjectedDay::ordinal` is a day's only scheduling identity | The document's day order is `ordinal`; `label` is text |
| **Period ordering** | `ProjectedPeriod::ordinal` within a `ScheduleAxis` | Row order is `ordinal`, per panel |
| **Breaks / non-teaching periods** | `ProjectedPeriod::is_teaching` (`projection.rs:78-81`): a break is a row, marked unavailable, so *"nothing is scheduled"* and *"nothing may be scheduled"* do not look alike | A break is a row in every format, styled distinctly in XLSX and print, and flagged in CSV by a dedicated column. **Never dropped** — dropping it would renumber the rows a reader counts |
| **Different period structures across campuses** | one `ScheduleAxis` per bell schedule; a scope touching two gets two `GridPanel`s | One sheet / one page section / one CSV panel column per panel, each naming its schedule and campus. **Never merged** (ADR 0033) |
| **Shared teachers and rooms** | the teacher or room is in this school's `Problem`; their commitments at another school are not. `TimetableView::shared` names the entity and the other schools (`views.rs:171`) | §9.3: the document carries the disclosure, and a teacher's page says which campus it is for |
| **Activities spanning multiple periods** | `ProjectedLesson::occupies` is every slot, first to last; `CellContent::Lesson { rows }` is the span, earned only when the lesson is alone and contiguous | XLSX `merge_range` over exactly `rows`; print `rowspan`; CSV emits **one row per occupied slot** with a part index, because a merge is not expressible in a flat file |
| **Multiple teachers on an activity** | `teacher_ids` / `teachers` are `Vec`, same order | All of them, joined; CSV gets a count column and a joined column, so a consumer can detect >1 without parsing |
| **Multiple rooms on an activity** | **not representable** (§2.3) | Nothing invented. CSV carries one room column; the format specification records the limitation by name |
| **Unscheduled activities** | not in the projection at all; in `TimetableView::unscheduled` from `solve_run` | A required section of the document. Carries `cause`, `is_impossible` and `diagnostic_code` **as stored**, and the word *impossible* stays gated the way `UnscheduledView` gates it |
| **Empty cells** | `CellContent::Empty`, distinct from a non-teaching row | Blank in XLSX and print; an explicit empty row in CSV only when the caller asks for the dense form (§6.3) |
| **Versions and manual edits** | `timetable_entry.is_locked` / `is_manual` → `LessonFlags`; `timetable.based_on_timetable_id` lineage; two scorings, `GENERATION` immutable | The header names the timetable, its revision and its run provenance; a locked or manual placement is marked in every format |
| **Violations and conflicting placements** | `ScopeGrid::conflicts` (`GridConflict`, naming day, period and every lesson); `FindingView` from `findings::explain_all`, rendering **the rule's own `explain()`** (ADR 0043) | A conflicting cell carries **every** lesson involved, never the first. Findings are an appendix, in the rule's own words, never re-worded here |

### 4.4 Two rules the document model enforces structurally

**Nothing is omitted to make a format render.** `GridPanel` already guarantees it for cells —
`CellContent::Pinned` carries every occupant. `OutputDocument` extends it: a `compose` that
dropped a lesson, a panel, an unscheduled occurrence or a conflict is a bug, and §15 proposes a
conservation test per format that counts what went in and what came out.

**Nothing is resolved twice.** `compose` calls nothing it could restate. The span is
`CellContent::Lesson { rows }`, already decided. The clash is `ScopeGrid::conflicts`, already
found. The caption is `SectionLabel::short`, already composed. The explanation is the rule's own
`explain()`. A `compose` that recomputed any of these would be `CLAUDE.md` §2's most important
rule broken in the one crate whose header promises it will not be.

### 4.5 Module and crate boundaries

| Where | What | New? |
| --- | --- | --- |
| `crates/evara-report/src/output/mod.rs` | `OutputDocument` and friends, `compose`, `OutputOptions`, `OutputError` | new |
| `crates/evara-report/src/output/csv.rs` | the CSV renderer and its column schema | new |
| `crates/evara-report/src/output/xlsx.rs` | the workbook renderer | new |
| `crates/evara-report/Cargo.toml` | `+ csv`, `+ rust_xlsxwriter` (both already in the workspace) | edited |
| `apps/desktop/src-tauri/src/commands/report.rs` | the Tauri commands (§12.2) | new |
| `apps/desktop/src/app/timetables/PrintView.tsx` + `print.css` | the print page and its stylesheet | new |
| `crates/evara-cli/src/report.rs` | `evara report` (§12.1) | new |

`evara-report`'s dependency row in `CLAUDE.md` §2 is `evara-report -> core, constraints`. Adding
`csv` and `rust_xlsxwriter` adds no `evara-*` edge and no I/O: both write into an in-memory
buffer. The layering table needs no amendment; §17's ADR records the reasoning so a later reader
does not read the new dependencies as a drift.

### 4.6 Extension points

- **A new format** implements the renderer trait (§5.5) over the existing `OutputDocument`. No
  projection change, no `compose` change. That is the roadmap's "additional formats drop in
  without touching the projections", discharged.
- **A Rust vector-PDF renderer** (D10's later refinement) is exactly that case: a third
  implementation, which is also the point at which the trait has earned itself.
- **A new scope kind** — were one ever added — flows through `ScopeKind`, `in_scope` and
  `lay_out` and reaches output without an output change, because the document is composed from
  `ScopeGrid` and not from a scope-specific shape.

---

## 5. Output models and renderer contracts

Shapes below are **PROPOSED** and are a specification, not final code.

### 5.1 `OutputDocument`

```rust
/// One output document: what is rendered, already decided.
pub struct OutputDocument {
    pub header: DocumentHeader,
    /// One per selected entity, in the caller's selection order.
    pub sheets: Vec<OutputSheet>,
    /// Occurrences with no placement, for the whole timetable. Never per sheet:
    /// an unplaced occurrence is in no scope's grid.
    pub unscheduled: Vec<UnscheduledLine>,
    /// Findings, in the words of the rules that produced them.
    pub findings: Vec<FindingLine>,
    /// Disclosures a reader must see for the document to be honest (§9.3).
    pub disclosures: Vec<Disclosure>,
    /// What the options asked for, carried so a format can print its own provenance.
    pub options: OutputOptions,
}

pub struct DocumentHeader {
    pub school_name: String,
    pub cycle_name: String,
    pub term_name: Option<String>,
    pub timetable_name: String,
    pub timetable_id: Uuid,
    pub timetable_rev: i64,
    /// Engine, seed, status, termination — `None` when no run is recorded.
    pub provenance: Option<RunProvenance>,
    /// Supplied by the caller. `evara-report` reads no clock.
    pub generated_at: String,
    pub app_version: String,
    /// Bytes and media type, when the school has a logo and the options asked for it.
    pub logo: Option<DocumentLogo>,
}

pub struct OutputSheet {
    pub scope: ProjectionScope,
    /// Empty when the scope has no lessons — which is a state, not an absence (§8.4).
    pub panels: Vec<OutputPanel>,
    /// True when `panels` is empty because the scope holds nothing.
    pub is_empty: bool,
    pub conflicts: Vec<GridConflict>,
    pub defective: Vec<DefectLine>,
    pub lesson_count: usize,
}

pub struct OutputPanel {
    pub schedule_id: Uuid,
    pub schedule_name: String,
    pub campus_name: Option<String>,
    pub days: Vec<ProjectedDay>,
    pub periods: Vec<ProjectedPeriod>,
    /// Row-major, `periods × days`, exactly as `GridPanel::cells`.
    pub cells: Vec<OutputCell>,
}

pub struct OutputCell {
    pub day_ordinal: u16,
    pub period_ordinal: u16,
    pub kind: OutputCellKind,
    /// Every lesson in this cell, in `GridPanel`'s order. Never truncated.
    pub lessons: Vec<CellLesson>,
    /// Merge height. 1 for everything but a `Lesson` that earned a span.
    pub rows: u16,
}

pub enum OutputCellKind {
    /// Nothing placed, and teaching is permitted here.
    Empty,
    /// Nothing placed, and nothing may be (a break).
    NonTeaching,
    /// One lesson, alone and contiguous. The only kind with `rows > 1`.
    Lesson,
    /// Covered by a `Lesson` above. Renderers skip it; CSV still emits a part row.
    Continuation,
    /// Held to one cell. `reason` is `PinReason`, carried verbatim.
    Pinned { reason: PinReason },
}

pub struct CellLesson {
    pub activity_id: Uuid,
    pub section_id: Uuid,
    /// The already-composed lines, in reading order. The one implementation (§5.6).
    pub lines: CellLines,
    pub teacher_ids: Vec<Uuid>,
    pub group_ids: Vec<Uuid>,
    pub room_id: Option<Uuid>,
    pub starts_at: String,
    pub ends_at: String,
    pub duration_periods: u16,
    /// 1-based position within the run, for a broken or continued occurrence.
    pub part: u16,
    pub parts: u16,
    pub flags: LessonFlags,
    pub defect: Option<SpanDefect>,
}
```

### 5.2 `CellLines` — the shared caption

```rust
pub struct CellLines {
    /// `SectionLabel::short`, or the section id when the label set does not hold it.
    /// Never a substituted name (§2.4).
    pub caption: String,
    /// The occurrence's own name, where one was given.
    pub activity_name: Option<String>,
    /// Every teacher, in the section's order.
    pub teachers: Vec<String>,
    /// Every attending student group.
    pub groups: Vec<String>,
    /// The room's label, where a room was booked.
    pub room: Option<String>,
    /// `'HH:MM–HH:MM'`, derived from the periods covered.
    pub time: String,
}
```

`CellLines::one_line(&self, detail: CellDetail) -> String` is what CSV and a narrow Excel column
use; the structured fields are what print and a wide Excel cell use. One implementation, four
consumers.

### 5.3 `OutputOptions`

```rust
pub struct OutputOptions {
    pub format: OutputFormat,          // Csv | Xlsx  (print takes the document, not a format)
    pub scope_kind: ScopeKind,
    /// Empty means "every entity of this kind that the projection offers".
    pub scope_ids: Vec<Uuid>,
    /// Empty means every bell schedule the selection touches.
    pub schedule_ids: Vec<Uuid>,
    /// Empty means every day of the cycle, in `ordinal` order.
    pub day_ordinals: Vec<u16>,
    pub include_non_teaching: bool,    // default true
    pub detail: CellDetail,            // what a cell says (§6.4)
    pub include_logo: bool,            // default true; ignored when the school has none
    pub page: PageSetup,               // size + orientation, honoured by XLSX and print
    pub include_unscheduled: bool,     // default true
    pub include_findings: bool,        // default true
}

pub enum PaperSize { A4, Letter }
pub enum Orientation { Portrait, Landscape }
pub struct PageSetup { pub paper: PaperSize, pub orientation: Orientation }
```

Defaults are chosen so that the common request needs no configuration: current scope, every
panel, every day, breaks shown, standard detail, logo on, A4 landscape, unscheduled and findings
included. `PaperSize` is deliberately two values: the brief asks for A4 and for
portrait/landscape, and a general paper-size list is a report designer's feature.

### 5.4 Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    /// The options named a scope the projection does not offer.
    #[error("{kind:?} {id} is not in this timetable")]
    UnknownScope { kind: ScopeKind, id: Uuid },
    /// The options named a bell schedule the selection does not touch.
    #[error("bell schedule {id} is not in this timetable")]
    UnknownSchedule { id: Uuid },
    /// Every selected day or panel was filtered away, so the document would be blank.
    #[error("the selection leaves nothing to render: {detail}")]
    EmptySelection { detail: String },
    /// The workbook writer refused. Carries its sentence.
    #[error("the workbook could not be written: {detail}")]
    Workbook { detail: String },
    /// A worksheet name could not be made unique within Excel's rules (§6.2).
    #[error("worksheet names could not be made unique: {detail}")]
    SheetNames { detail: String },
}
```

Three notes. `csv::Writer` over a `Vec<u8>` cannot fail, which is why there is no `Csv` variant —
`evara-import` relies on the same fact with a documented `expect` (`template.rs:186-196`).
`EmptySelection` is a refusal rather than an empty file, because an empty file reads as "there
was nothing" rather than "you filtered everything out". And `OutputError` names no path, because
this crate never has one.

### 5.5 The renderer trait — declared narrowly, on purpose

```rust
/// A renderer that turns a composed document into a file's bytes.
///
/// Declared over the two Rust byte-producing formats and no further. CSV is a row
/// stream and XLSX is a grid, and this is the whole of what they share: a document in,
/// bytes out, a media type and an extension to name the result.
pub trait DocumentRenderer {
    /// A stable key for the format: `"csv"`, `"xlsx"`.
    fn key(&self) -> &'static str;
    fn extension(&self) -> &'static str;
    fn media_type(&self) -> &'static str;
    /// Renders the whole document. Bytes, because this crate writes no files.
    fn render(&self, document: &OutputDocument) -> Result<Vec<u8>, OutputError>;
}
```

**Why the print path is not an implementation of it.** It would have to return the bytes of an
HTML document, and then the thing that paginates, hyphenates, measures and prints it is the
webview — so the trait would be returning an input to a renderer rather than a rendered artefact,
and the one format the user most cares about would be the one the trait described least. The
honest shape is: the print page is a **third consumer of `OutputDocument`**, over IPC, in the
layer that owns the window. The trait is for formats Evara renders itself, and it has two
implementations on day one, which is what `lib.rs:40-46` asked for before a trait was allowed to
exist.

**ASSUMPTION** worth stating: that two implementations are enough to shape the trait correctly
for a third. The way to settle it is D10's Rust PDF renderer, and the trait is small enough that
being wrong costs a signature change rather than a redesign.

### 5.6 The one change proposed to existing code

`TimetableGrid.tsx:517-526` composes a cell's caption, teacher list and room name in TypeScript.
That is three lines of joining rather than projection logic, so it does not breach `CLAUDE.md`
§2 today. But once Rust composes `CellLines` for three other consumers, the screen is the only
place composing them separately, and *"the screen says one thing and the printout says
another"* is precisely the drift this milestone must not introduce.

**PROPOSED:** `ProjectedLesson` gains a `lines: CellLines` field, populated by `project`, and
`TimetableGrid.tsx` renders it. Additive, no behaviour change, no migration, and it makes the
on-screen grid and every output format share one caption by construction. Stage 1 owns it. The
cost is that `TimetableProjection` grows by a few strings per lesson; §14.1 prices it.

---

## 6. CSV and Excel format specifications

### 6.1 CSV — long form, one row per occupied slot

**Why long form and not a visual grid.** A grid-shaped CSV is a picture of a table in a file
format that cannot express a merge, a `rowSpan` or two lessons in one cell — so it would either
lie about a clash or drop one of the two. The long form says one true thing per row and lets a
spreadsheet pivot it. A reader who wants the picture has the workbook.

**Columns — stable, documented, and the contract.** Added only at the end, never reordered,
never renamed; `IMPORT.md`'s discipline for template columns applied to an output.

| # | Column | Notes |
| --- | --- | --- |
| 1 | `view` | `master` \| `section` \| `teacher` \| `room` \| `student` |
| 2 | `view_entity` | the scope's name, e.g. the teacher |
| 3 | `view_entity_id` | the scope's UUID; empty for `master` |
| 4 | `campus` | the panel's campus, where the schedule names one |
| 5 | `bell_schedule` | the panel's schedule name |
| 6 | `day_ordinal` | the cycle day's **only** scheduling identity |
| 7 | `day_label` | display |
| 8 | `period_ordinal` | within this schedule |
| 9 | `period_label` | display |
| 10 | `starts_at` / 11 `ends_at` | `HH:MM`, the period's |
| 12 | `is_teaching` | `yes` / `no` — a break is a row |
| 13 | `cell_state` | `empty` \| `non_teaching` \| `lesson` \| `conflict` \| `shared` \| `broken_run` |
| 14 | `occupants` | how many lessons are in this cell. `2` is how a consumer finds a clash |
| 15 | `lesson_caption` | `SectionLabel::short`, or the id when unlabelled |
| 16 | `subject` / 17 `course` / 18 `section` / 19 `year_level` | the label's parts, unjoined |
| 20 | `activity_name` | where one was given |
| 21 | `teacher_count` / 22 `teachers` | the count, then `"; "`-joined — **all** of them |
| 23 | `group_count` / 24 `groups` | likewise |
| 25 | `room` / 26 `building` | one room; §2.3 finding 3 is recorded in the format doc |
| 27 | `duration_periods` | what the occurrence asked for |
| 28 | `part` / 29 `parts` | `2 of 3` for a multi-period run's second slot |
| 30 | `is_locked` / 31 `is_manual` | from `LessonFlags` |
| 32 | `run_defect` | `SpanDefect`'s key, where the run is not sound |
| 33 | `activity_id` / 34 `section_id` / 35 `timeslot_id` | stable identifiers, last |

**Row rules.**
- A multi-period occurrence emits **one row per occupied slot**, with `part`/`parts` — so a
  reader summing `duration_periods` is not double-counting and a reader counting rows gets
  occupied slots. Both questions are answerable; neither is guessed at.
- A cell with two occupants emits **two rows** with `occupants = 2` and the same
  `cell_state`. **Nothing is collapsed** (§4.4).
- A break emits one row with `is_teaching = no`, `cell_state = non_teaching`.
- An empty teaching cell emits a row **only when `include_empty` is set**; the default is to
  omit it, because a full grid of empties is 80% of the file for a half-full timetable. Omission
  is safe here and only here, because rows 6–9 make every position addressable and absence is
  therefore unambiguous. Said plainly in the format doc.
- Unscheduled occurrences follow the grid rows, with `cell_state = unscheduled` and the
  position columns empty, carrying the cause and `is_impossible`. An export that ended at the
  grid would omit them silently.

**Encoding and safety.**
- **UTF-8, with a BOM.** `csv::WriterBuilder` writes no BOM and does not need one; Excel on
  Windows, opening a `.csv` by double-click, interprets a BOM-less file in the system codepage
  and mangles every non-ASCII name. A BOM is three bytes that make the common path correct, and
  `csv`-family readers tolerate it. **ASSUMPTION** on the second half (reader tolerance across
  the tools schools actually use); settled by the §15 round-trip test, which reads Evara's own
  output back with `calamine`/`csv`.
- CRLF line endings, `csv`'s default on this terminator setting — chosen for the same audience.
- Quoting, embedded commas, quotes and newlines are `csv`'s job and are already relied on by
  `evara-import` (`template.rs:182`).
- **Formula injection.** A cell whose text begins with `=`, `+`, `-`, `@`, a tab or a carriage
  return is prefixed with a single apostrophe before it is written. This is a real exposure
  rather than a theoretical one: a teacher's display name is user-controlled text that reaches a
  CSV a school opens in Excel. The one implementation lives in
  `output::guard::defuse_formula` and is called by **both** the CSV and the XLSX renderer — a
  second copy is the duplication `CLAUDE.md` §2 names. §15 tests it per format, and a mutation
  check (remove the guard, see red) is proposed for the same reason the materialiser has one.

### 6.2 Excel — a readable grid per entity

**Workbook shape.**

| Sheet | Contents |
| --- | --- |
| 1 | **Cover.** School, campus, cycle, term, timetable name and revision, run provenance, generated-at, app version, the selection that produced the document, the logo, and every disclosure in §9.3 |
| 2…n | **One sheet per selected entity**, one grid block per panel, stacked with the schedule and campus named above each |
| n+1 | **Not placed** — the unscheduled occurrences, cause and whether impossible |
| n+2 | **Findings** — level, rule, and the rule's own sentence |

Two sheets are always present (cover, and at least one grid). "Not placed" and "Findings" are
written **even when empty**, carrying the sentence *"nothing"* — an absent sheet reads as a
feature that was not run.

**The grid block.** Days across, periods down, matching the screen. Period label, start and end
in the left columns; day labels in the header row, frozen with `set_freeze_panes`. A lesson that
earned a span is `merge_range` over exactly `OutputCell::rows`; everything else is one cell
carrying every occupant. `set_column_width` per column and `set_row_height` scaled to the detail
level, with `set_text_wrap` so a three-line cell reads as three lines rather than as `####`.

**Formats, and monochrome legibility.** A `Format` per role: heading, period gutter, teaching
cell, **non-teaching cell**, conflict, shared, broken run, locked, manual. Colour carries no
information on its own: a conflict also gets a bold border and a leading marker in its text, a
break also gets a distinct fill *and* an italic `—`, so a photocopied page is still readable.
That is §3.2's monochrome requirement discharged in the format that is most tempted to ignore it.

**Worksheet names are a real constraint, VERIFIED.** From
`rust_xlsxwriter-0.102.0/src/utility.rs:1112-1138`, `validate_sheetname` refuses a name that is
empty, longer than **31 UTF-16 code units**, contains any of `* ? : [ ] \ /`, or starts or ends
with an apostrophe; `Workbook` additionally refuses a reused name
(`XlsxError::SheetnameReused`). So the proposed rule, in one function with one test matrix:

1. Start from the entity's display name.
2. Replace each forbidden character with a space; collapse runs of spaces; trim.
3. Trim leading and trailing apostrophes.
4. Truncate to 31 UTF-16 code units **on a character boundary**, not a byte one.
5. If empty after all that — a name of only forbidden characters — use the kind and the ordinal:
   `Teacher 12`.
6. Deduplicate by appending ` (2)`, ` (3)`, … re-truncating to fit. Two teachers genuinely
   called the same thing is ordinary in a large school (§9.5), and `SheetnameReused` would fail
   the whole export for it.
7. If dedup cannot converge, `OutputError::SheetNames` names the clash rather than writing a
   workbook with an arbitrary name in it.

**Print setup, VERIFIED available.** `rust_xlsxwriter 0.102` with `default-features = false`
exposes `set_paper_size`, `set_landscape`/`set_portrait`, `set_margins`, `set_print_area`,
`set_repeat_rows`, `set_repeat_columns`, `set_header`/`set_footer`,
`set_print_fit_to_pages`, `set_print_scale`, `set_print_gridlines`, `set_page_breaks`,
`insert_image`, `merge_range`, `set_row_height`, `set_freeze_panes` — all checked against
`src/worksheet.rs` in the vendored crate. So the brief's "print settings where supported by the
existing library" is: A4 or Letter via `set_paper_size` (A4 is paper index 9), orientation,
margins, `set_repeat_rows` for the day header on every page, `set_print_fit_to_pages(1, 0)` so a
wide cycle fits the page width, and a footer carrying the school, the timetable and the page
number. **A workbook's print setup is advisory** — Excel honours it, another reader may not — and
the format doc says so rather than implying the page is fixed.

**The logo** is `insert_image` on the cover sheet from `SchoolLogo::bytes`.
`Image::new_from_buffer` recognises PNG, JPEG, GIF and BMP (`src/image.rs`), and
`evara_core::branding` has already sniffed the format and bounded the dimensions, so a logo that
reached the database is one the writer can take. A format it cannot take is **omitted with a
disclosure line**, never substituted.

**No feature flag changes.** `constant_memory` (which would stream a workbook through a
`tempfile`) is deliberately **not** enabled: §14.3's budget shows the largest proposed workbook
is tens of megabytes of cells at most, and a temporary file is a new failure mode for a
local-first application to acquire without a measurement demanding it.

### 6.3 What CSV and Excel each refuse to do

| | CSV | Excel |
| --- | --- | --- |
| Shows a merge | no — `part`/`parts` instead | yes, `merge_range` |
| Shows two lessons in one cell | yes, two rows, `occupants = 2` | yes, one cell, every occupant, conflict format |
| Omits empty teaching cells | by default, yes — positions stay addressable | no |
| Omits breaks | never | never |
| Defuses a leading `=` | yes | yes, same function |
| Carries unscheduled occurrences | yes, as rows | yes, as a sheet |

---

## 7. Print and PDF platform feasibility

This is the section the brief asks to be most careful in, so every claim below carries its
provenance. The versions referred to are the ones this repository pins: `tauri 2.12.1`,
`wry 0.57.0`, `tao 0.37.1` (`Cargo.lock`).

### 7.1 Windows / WebView2 — **VERIFIED**

`wry-0.57.0/src/webview2/mod.rs:1796-1802`:

```rust
pub fn print(&self) -> Result<()> {
    self.eval("window.print()", None::<...>)
}
```

So on Windows the Rust API *is* `window.print()`, and the JavaScript call is the primary path
rather than a fallback. Tauri injects no interception script on Windows: the `print.js` shim is
`#[cfg(any(target_os = "macos", target_os = "ios"))]`
(`tauri-2.12.1/src/webview/plugin.rs:198-202`).

Microsoft's own documentation ([Printing from WebView2 apps][ms-print]) lists `ShowPrintUI`,
`Print`, `PrintToPdf` and `PrintToPdfStream` as the host-side APIs and describes `ShowPrintUI`
as opening *"the WebView2 **Print Preview** dialog or the operating system's **Print** dialog"*;
WebView2's own documentation notes that `ShowPrintUI` behaves as the dialogs opened by
`window.print()` do. The preview is the Edge print preview, which carries a destination list
including PDF.

**ASSUMPTION remaining:** that the preview's PDF destination, our `@page` rules and our repeated
`<thead>` combine into the page a school expects. Settled only by §7.5's spike.

### 7.2 macOS / WKWebView — **VERIFIED, and currently broken in this repository**

Three facts, each from the vendored source.

**(a) `window.print()` is not WKWebView's.** Tauri replaces it. `tauri-2.12.1/src/webview/
scripts/print.js`, injected only on macOS and iOS:

```js
window.print = function () {
  return window.__TAURI_INTERNALS__.invoke('plugin:webview|print')
}
```

with the comment at `plugin.rs:198`: *"window.print works on Linux/Windows; need to use the API
on macOS"*. This matches the long-standing WebKit position that WKWebView does not implement
`window.print()` on macOS; the Apple developer-forum threads on
[printing WKWebView to PDF][apple-pdf] and [printing in Sequoia][apple-seq] are the public
record of it.

**(b) That invoke is denied by our capability set.** From the vendored permission reference,
`tauri-2.12.1/permissions/webview/autogenerated/reference.md:1-11`, the default set is exactly:

```
- allow-get-all-webviews
- allow-webview-position
- allow-webview-size
- allow-internal-toggle-devtools
```

`core:webview:allow-print` exists (`reference.md:154`) and is **not** in that list.
`apps/desktop/src-tauri/capabilities/default.json` requests
`["core:default", "dialog:allow-open", "dialog:allow-save", "dialog:allow-message"]`. So
`invoke('plugin:webview|print')` is rejected, and the rejection surfaces as a failed promise
inside Tauri's own shim — the user sees a Print button that does nothing. This is the same defect
reported publicly against other Tauri 2 applications
([Pake #1396][pake], [gitpass #384][gitpass]).

**(c) With the permission granted, it reaches a real print panel.**
`wry-0.57.0/src/wkwebview/mod.rs:879-918` calls `printOperationWithPrintInfo:` on the
`WKWebView` (macOS 11+ — which matches our `minimumSystemVersion: "11.0"`), sets the margins from
`PrintOptions`, `setCanSpawnSeparateThread(true)`, and
`runOperationModalForWindow_delegate_didRunSelector_contextInfo`. That is the native AppKit print
panel, whose **PDF ▸ Save as PDF** is macOS's own and needs nothing from us.

**The `unsafe` question, answered.** `CLAUDE.md` §3 forbids `unsafe_code` workspace-wide. The
Objective-C calls are inside `wry`, not inside this workspace, so **M11 needs no `unsafe` and no
exception**: the whole macOS print path is one permission string in a JSON file.

**One thing to be careful about, VERIFIED.** `print_with_options` guards on
`respondsToSelector(printOperationWithPrintInfo:)` and, when the selector is absent, **falls
through and returns `Ok(())`** having printed nothing. So a successful return is not evidence
that a print panel appeared. A "Printing…" message that trusted the return value would be a
screen claiming something it does not know, which is the failure `TIMETABLE-VIEWS.md` §10 is
written against. §11.4 handles it.

**Two capabilities macOS does not give us, VERIFIED.** `print_with_options` reads only
`options.margins` and ignores everything else, and `tauri::Webview::print()` takes no options at
all (`tauri-2.12.1/src/webview/mod.rs:1626`). It uses `NSPrintInfo::sharedPrintInfo()`. So
**orientation and paper size cannot be set from Rust on macOS**, and there is no completion
callback — the operation is modal and detached. Consequences: page setup must come from CSS
`@page`, and the application cannot report that a print succeeded, was cancelled or failed.

### 7.3 The two risks CSS has to carry

Because page setup must be CSS on macOS, the `@page` rules do the work on both platforms rather
than one. Two **ASSUMPTION**s follow, and both are things a spike settles rather than an argument.

| Assumption | Why it matters | How it is settled |
| --- | --- | --- |
| `@page { size: A4 landscape; margin: … }` is honoured by WebKit in a WKWebView print operation | if it is not, orientation becomes the user's problem in the panel on macOS while working on Windows — an inconsistency the brief asks us not to assume away | §7.5 spike, both platforms, A4 and Letter, both orientations |
| `thead` repeats on every page and a `tr` does not split mid-row in both engines | a timetable broken across a page boundary with no day header is unusable, and this is exactly what D10 flagged | same spike, with a cycle long enough to force three pages |

Chromium's print layout repeats `thead` and honours `@page size`; WebKit's `@page size` support
has historically been narrower. Neither statement is tested here, so neither is a finding.

### 7.4 Recommendation

**Keep D10's webview print pipeline for M11.** It is free, it uses the print stylesheet the
roadmap wants anyway, it reaches each platform's own Save-as-PDF, and the one concrete obstacle
turns out to be a permission rather than a missing capability. Specifically:

1. Add `core:webview:allow-print` to `capabilities/default.json`, with a comment naming the
   macOS reroute — the file already documents each capability by the phase that needed it.
2. Print from the frontend with **`window.print()`**, one call site. On Windows it is native; on
   macOS Tauri's shim routes it to `wry`. No new command, no second mechanism.
3. Page setup lives in `print.css` as `@page`, because macOS offers no programmatic route.
4. The print page is a dedicated React view over `OutputDocument` (§11.3), so what is printed is
   the document and not the application chrome.

### 7.5 The print spike — proposed as Stage 0, before anything is built

A departure from the brief's stage order (§16), and the reason is that if §7.3's assumptions
fail, Stage 3 is a different piece of work and Stage 1's document model may need a page-break
vocabulary it would otherwise not have. The spike is small: a static HTML page with a realistic
grid, the proposed `@page` rules, three pages' worth of rows, and the permission added. It
answers five questions and writes them into this document before Stage 1 starts:

1. Does a Print action reach a panel on macOS with the permission added? Without it?
2. Does Save as PDF produce a correct multi-page document on each platform?
3. Is `@page size`/orientation honoured on each platform?
4. Does `thead` repeat, and do rows stay whole?
5. Does a 10-day custom cycle fit a landscape A4 page at a legible size, and if not, does
   per-panel or per-day pagination read acceptably?

**This is manual, on both platforms, and cannot be automated** — §15.5 keeps it in the manual
column and §19 carries the risk if it is deferred.

### 7.6 If the spike fails

Compared honestly, smallest first.

| Option | Cost | Licence | Verdict |
| --- | --- | --- | --- |
| **Narrow the promise.** Print what a browser does paginate well: one panel per page, day headers repeated per panel rather than per page | none; a layout change | — | **First response.** Most of D10's risk is about *one long table*, and a timetable does not have to be one |
| **Open the print HTML in the user's browser** via `tauri-plugin-opener`, where Ctrl/Cmd+P and Save as PDF are the platform's own | one official plugin (Apache-2.0 OR MIT), a temporary file to clean up, and a second place output can come from | permissive | **Second.** Honest and offline, but it is a second output mechanism and the brief warns against those |
| **Rust vector PDF** (`printpdf`, MIT) | a real renderer: layout, pagination, font embedding, metrics. Weeks, and a font licence question | MIT, but transitive font/PDF crates are where `OPEN-DECISIONS.md` R7 says GPL risk lives | **Not M11.** D10's later refinement, and the trait in §5.5 is where it lands |
| `PrintToPdf` via WebView2's host API | Windows only, and reaching `ICoreWebView2_7` from Tauri needs `unsafe` | — | **Refused.** `unsafe_code` is forbidden workspace-wide, and a Windows-only path is half a feature |

---

## 8. Individual and bulk export behaviour

### 8.1 Individual

From the timetable screen, with the current scope preselected: one entity, one format, one file
the user names. `OutputOptions { scope_kind, scope_ids: vec![id] }`. The individual case is the
bulk case with one entity, and there is one code path.

### 8.2 Bulk — one document, not many files

**The decision, and why.** The brief asks for individual files, multi-sheet workbooks,
multi-page documents or ZIPs to be evaluated, with ZIP only where justified, and warns against
competing mechanisms. The recommendation is **one document per request, per format**:

| Format | Bulk shape | Why not the alternative |
| --- | --- | --- |
| **CSV** | one file, every entity, `view_entity` discriminating | 400 CSVs is 400 files to manage; one file pivots |
| **XLSX** | one workbook, one sheet per entity (cover + grids + two appendices) | one workbook per teacher means a folder nobody opens; and a sheet per entity is what a school already does by hand |
| **Print/PDF** | one document, entities in sequence, a page break between them | a PDF per teacher needs either a Rust PDF writer or 400 print dialogs |

**No ZIP.** It is justified only when the output genuinely is many files, and the shapes above
mean it never is. That also removes generated filenames entirely (§10.2), which is a larger win
than the ZIP would have been. `zip 9` stays in the tree for `evara-project`, available if a later
milestone earns it.

### 8.3 Students — where one-sheet-per-entity stops being right

The brief says not to assume one worksheet per student suits every school, and it is right.

- **Excel's own limit** is memory, not a row count: a worksheet per student at 3 000 students is
  3 000 sheets, which Excel will open and no human will navigate, and §14.3's estimate puts it at
  hundreds of megabytes.
- **The quadratic layout cost** (§14.2) is the harder half, and must be fixed either way.

**PROPOSED:** a threshold, named and visible, not a silent cap.

| Entities selected | XLSX shape |
| --- | --- |
| ≤ **60** | one grid sheet per entity. 60 is comfortably above a school's teacher count per campus and above any realistic class or room list |
| > 60 | **one long-form sheet**, the CSV's columns as a formatted table with filters and frozen headers, plus the cover and the two appendices |

The cover sheet states which shape was used and why, in a sentence. The user may override in
either direction from the bulk dialogue, and choosing grids for 3 000 students shows the
estimated size first. The one thing that must not happen is a silent truncation: §15 asserts that
every selected entity appears in the output in both shapes, and `log`-style silence is not an
option the code has.

### 8.4 An entity with no lessons

`lay_out` gives it no panels (§2.3 finding 2). The document carries it as
`OutputSheet { panels: [], is_empty: true }` and each format renders it as a **present and
empty** thing: a worksheet with the entity's name and the sentence *"No lessons are placed for
this teacher in this timetable."*, a CSV row with `cell_state = no_lessons`, a printed page with
the same sentence. A school exporting every teacher needs to see which teachers have nothing — it
is often the finding they were looking for.

### 8.5 Consistency — structural, not guarded

The brief asks that a bulk export cannot mix inconsistent timetable states if the project changes
mid-generation. It cannot, by construction: **one command compiles one `Assembly`, builds one
`LabelSet`, reads one arrangement, projects once, and composes one document in memory.** A
concurrent write cannot reach any of that. No change-log watermark is needed here, and adding one
would imply a risk that does not exist — unlike ADR 0042's save, which writes.

Two honesty requirements come with that:

- The document's header carries the **timetable's revision** and the generated-at stamp, so a
  file on somebody's desk can be matched to a version. An exported document is a **snapshot**,
  and both the cover sheet and the printed footer say so.
- The command reads the timetable row **inside the same workspace lock** as the compile, so
  header and grid describe the same revision.

---

## 9. Multi-campus handling

### 9.1 School isolation is structural

A `Problem` is one `SchedulingSelection { school_id, cycle_id, term_id }`, and `labels` is scoped
by the same `school_id` and `IN_SCOPE_TERM` (`crates/evara-db/src/scheduling/labels.rs:52-61`).
The projection sees nothing else, so **a document cannot contain another school's data** —
there is no code path by which it could. `timetable_open` resolves the school through the cycle
and refuses when the cycle is gone (`views.rs:390`, and `views.rs:684-693` for the scope read). M11 inherits all of it and adds no
school-scoped query of its own.

### 9.2 Campus selection

A campus reaches output as `GridPanel::campus_name` / `ScheduleAxis::campus_id`.
`OutputOptions::schedule_ids` filters panels, so "just the Northgate grid" is expressible without
a campus scope. Every grid block names its schedule and its campus above it, in every format,
because two panels in one document with no label is the one way this model can confuse a reader.

### 9.3 Teachers and rooms that work across campuses — the disclosure

This is the case where output can mislead while being correct. A teacher with memberships at two
schools has, in *this* document, only the lessons of the school solved. A printed sheet headed
with their name reads as *their timetable*, and it is not — it is their timetable **here**.

`TimetableView::shared` already names such entities and both schools
(`views.rs:162-180`), built because ADR 0032 rule 3 requires a warning to name the entity, both
schools and the consequence. **PROPOSED:** `OutputDocument::disclosures` carries it, and:

- every teacher and room sheet's header names the **school and campus** the grid is for;
- when the selected entity is in `shared`, the sheet and the printed page carry one sentence
  naming the other schools and stating that commitments there are not shown;
- the same sentence is on the cover sheet when any selected entity is shared;
- **this is not suppressible by an option.** An option to hide it would be an option to produce a
  misleading document.

Cross-campus *clashes* are not computed here and are not claimed to be: a clash is overlapping
minutes within one compiled problem, and this document has one.

### 9.4 A teacher's commitments across campuses — deferred, with the honest reason

Merging would mean compiling a second `Problem` for the other school and placing two bell
schedules on one axis, which ADR 0033 forbids and §3.1 explains. The nearest honest thing is
**two documents, one per campus, and the disclosure that says so** — which is what §9.3
delivers. A genuine cross-campus view needs a *cross-school* scheduling model, and that is after
Milestone 1 along with the remaining multi-campus constraints (`ROADMAP.md`, After Milestone 1).

### 9.5 Entities with identical display names

Ordinary in a large school. Three places it matters, all handled:

- **Worksheet names** — dedup by ` (2)`, ` (3)` (§6.2 rule 6), because `SheetnameReused` would
  fail the export.
- **CSV** — `view_entity_id` disambiguates; a consumer never has to rely on the name.
- **Print and the cover** — the entity's secondary line is shown (`ProjectionScope::secondary`:
  a building for a room, a course for a section, a code for a student), which is what the
  on-screen picker already does (`ScopeView.tsx:97-100`).

### 9.6 School-specific logos

`school_logo(school_id)` is per school and the document is per school, so the right logo is the
only one reachable. A school with none gets no logo and no substitute: the existing branding code
already treats "no logo" as the ordinary case (`structure.rs:194-197`), and a document is not the
place to introduce an Evara watermark on somebody's timetable.

---

## 10. File safety and cancellation

### 10.1 Destination ownership — M10's device, reused without coupling

`DestinationClaim::take` (`crates/evara-project/src/claim.rs:84-105`) creates the destination with
`create_new(true)`, so *"may I write here?"* and *"I now hold it"* are one filesystem operation;
`released()` gives it up after the rename; `Drop` removes the placeholder otherwise. Its module
header already names three callers and says a fourth implementation would be an architecture bug.

**The coupling question the brief asks** — can it be reused without tying timetable output to the
archive protocol? **Yes, and without moving it.** The claim is in `evara-project`, and the two
callers that need it (`src-tauri` and `evara-cli`) both already depend on `evara-project`.
`evara-report` does not and must not, which is exactly why the renderers return bytes. So:

```
render bytes (evara-report)
  → judge the path          (paths::require_safe_destination)
  → claim it                (DestinationClaim::take)
  → write a sibling `.part`
  → fsync, rename onto the claim   (paths::rename_with_retry)
  → claim.released()
```

Ordering matters and is M10's: **judge, then claim** — a path Evara will not write to is refused
before a file is made at it (`claim.rs:38-42`). The only `evara-project` types that reach the
output command are `DestinationClaim`, two path functions and `TransferError`, and nothing of the
archive protocol — no manifest, no checksum record, no staging, no journal. A one-paragraph ADR
(§17, ADR-B) records that the claim is a general-purpose device rather than part of the transfer
protocol, so a later reader does not infer a dependency that is not there.

**No overwrite, ever.** An occupied destination is `DestinationExists` and the dialogue offers
another name — the same answer the Export screen gives (`ExportCampus.tsx:11-17`), for the same
reason. A document somebody may still need is not replaced because a name collided. A
zero-length file is **occupied**, not adoptable, because `create_new` fails if the path exists at
all.

### 10.2 Filenames — the problem M11 mostly does not have

Because bulk output is one file (§8.2), **Evara derives no filenames.** The user names the file in
`save()`. That removes at a stroke: Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`…,
`LPT1`…, and their `name.ext` forms), trailing dots and spaces, duplicate entity names colliding
on disk, and path traversal through an entity name.

What remains is the **suggested** name in the dialogue's `defaultPath`, built from the timetable
and scope names. A suggestion the user accepts becomes a real path, so it needs one small guard —
`output::names::safe_file_stem` — doing exactly what §6.2's sheet rule does for a different
character set: strip `< > : " / \ | ? *` and control characters, collapse spaces, trim trailing
dots and spaces, refuse the Windows device names case-insensitively (stem only, so `CON.csv` is
caught), bound the length, and fall back to `timetable` when nothing is left. **~40 lines and a
table-driven test**, and this is the one genuinely new piece of file-safety work in M11.

Path traversal is not reachable: a path comes from the native dialog or a CLI argument, and
`require_safe_destination` already refuses a `..` component, a `file:` prefix and a symlink at the
leaf (`paths.rs:47-92`).

### 10.3 Partial output

Nothing is ever written to the destination except by a rename, so a failure mid-render leaves the
destination with the zero-length claim, which `Drop` removes. A failure after the `.part` is open
removes the `.part` on the same `Drop` idiom `Snapshot` and the archive writer use
(`claim.rs:123-133`). **Disk-full** surfaces as `io::Error` while writing the `.part` and is
reported with the action that failed, per `TransferError::Io { action, source }`'s existing shape.
The invariant, asserted in §15: **a refused or failed output leaves the folder exactly as it was
found.**

### 10.4 Cancellation

If §14.4's measurement gate leaves output synchronous, there is no cancellation to offer and the
dialogue says nothing about one — a Cancel button that cannot cancel is worse than none. If the
gate trips, cancellation is `CancelFlag` and `Lifecycle`'s existing contract: a **request** that
sets an `AtomicBool`, read between entities, changing no state; the worker is the only thing that
leaves `Running`. `compose` and the renderers take `&dyn Cancel` and check **between sheets**,
which is the natural granularity and keeps `evara-report` clock-free. A cancelled output leaves
nothing: the claim is still held and its `Drop` releases it.

---

## 11. Desktop UX

**PROPOSED**, and not implemented in this PR. Wireframes are text, with component-level notes.

### 11.1 Three things the user must be able to tell apart

The brief is explicit, and the application already has one of the three. The vocabulary:

| Action | Where | Says |
| --- | --- | --- |
| **Export campus file** (EXISTS) | title-bar action, Export screen | the whole project, every school, as a portable `.evara` |
| **Download timetable** (new) | on the open timetable | this timetable, as a spreadsheet or CSV |
| **Print timetable** (new) | on the open timetable, beside Download | this timetable, on paper or as a PDF |

The distinction is carried by **placement and wording**, which is how M10 Stage 5 settled D9
(export is about the open campus file, so it is a title-bar action). A timetable action belongs to
the timetable on screen. The words *export* and *campus* stay reserved for the archive; the
timetable actions are **Download** and **Print**. One sentence under each in the dialogue
completes it.

### 11.2 On the open timetable

`Timetables.tsx`'s `ttActions` row already holds Edit, Repair, Rename, Revalidate, Delete
(`Timetables.tsx:418-470`). Two buttons join it, after Repair and before Rename:

```
┌──────────────────────────────────────────────────────────────────────┐
│ ‹ Back to timetables                                                 │
│                                                                      │
│ Spring 2027 — Draft 3                              [read-only badge] │
│ Northgate · Standard week · Term 1 · saved 14 Nov 2026               │
│                                                                      │
│ [Edit this timetable] [Repair this timetable]                        │
│ [Download…] [Print…]   [Rename] [Re-check] [Delete]                  │
│                                                                      │
│ What to show:  (All lessons) (Classes) (Teachers) (Rooms) (Students) │
│ Teacher: ▼ A. Okafor — 24 lessons                                    │
│ ...grid...                                                           │
└──────────────────────────────────────────────────────────────────────┘
```

Both actions open with the **current scope and entity preselected**, so the common case —
"print what I am looking at" — is two clicks and no configuration.

### 11.3 The Download dialogue

A `<dialog>` via the existing `Modal` in `ui/Shell.tsx`, which is what School Setup and both
transfer screens already use.

```
┌─ Download timetable ─────────────────────────────────────────┐
│                                                              │
│ Spring 2027 — Draft 3 · Northgate · Term 1                   │
│ A snapshot of this timetable as it stands now.               │
│                                                              │
│ What            ( ) Just this one:  A. Okafor                │
│                 (•) Every teacher  (38)                      │
│                 ( ) Choose…                                  │
│                                                              │
│ View            [ Teachers ▾ ]                               │
│ Campus          [ All campuses ▾ ]   (2 bell schedules)      │
│ Days            [ Whole cycle ▾ ]                            │
│                                                              │
│ Format          (•) Excel workbook (.xlsx)                   │
│                 ( ) Data file (.csv)                         │
│                                                              │
│ Cell shows      [ Class, teacher and room ▾ ]                │
│ Page            [ A4 ▾ ]  [ Landscape ▾ ]                    │
│                 [x] Show the school logo                     │
│                 [x] Include lessons that could not be placed │
│                 [x] Include the findings list                │
│                                                              │
│ ⓘ 38 teachers, one worksheet each. About 1.2 MB.             │
│ ⓘ Two teachers also work at Southmoor. Their sheets will     │
│   say that commitments there are not shown.                  │
│                                                              │
│                                   [ Cancel ] [ Choose file… ]│
└──────────────────────────────────────────────────────────────┘
```

Notes that are decisions rather than layout:

- **Choose file… is the last step.** The native `save()` dialog comes after the options, so a
  user who changes their mind has not already named a file. Same order as the Export screen.
- **The two ⓘ lines are computed from the selection**, not static text. The first is §8.3's
  shape and estimate, shown *before* a 3 000-sheet workbook is attempted. The second is §9.3's
  disclosure, surfaced here as well as in the file.
- **Page setup appears for `.xlsx` only** and is hidden for `.csv`, where it means nothing.
- Everything has a default; the dialogue is usable without touching anything below **Format**.

### 11.4 The Print flow

```
Print… ──► a dedicated print view fills the window
           (the document, no navigation, no controls, no drag affordances)
           ┌──────────────────────────────────────────┐
           │ [‹ Back]  [Page setup ▾]      [ Print ]  │  ← screen only; @media print hides it
           ├──────────────────────────────────────────┤
           │ [logo]  Northgate — Spring 2027 Draft 3  │
           │         Teacher: A. Okafor · Term 1      │
           │  ┌────┬─────┬─────┬─────┬─────┬─────┐    │
           │  │    │ Mon │ Tue │ Wed │ Thu │ Fri │    │
           │  ├────┼─────┼─────┼─────┼─────┼─────┤    │
           │  │ P1 │ 9 Mathematics B           │ …   │
           │  ...                                     │
           └──────────────────────────────────────────┘
                           │  [ Print ] → window.print()
                           ▼
            Windows: WebView2 print preview (PDF destination available)
            macOS:   AppKit print panel (PDF ▸ Save as PDF)
```

Four decisions:

- **A view, not a hidden iframe.** The user sees what will print before the dialog opens, which
  is the cheapest possible answer to D10's pagination risk: a bad break is visible.
- **`@media print` hides the toolbar, the shell and every interactive affordance**, satisfying
  "no application navigation, interactive controls or editing overlays in printed output". The
  print view mounts outside the editing session, so no drag handles exist to hide.
- **No success message.** §7.2 established that a successful return proves nothing on macOS and
  that there is no completion callback. The view therefore says nothing about the outcome and
  stays on screen so the user can print again — the alternative is a dialogue claiming a print
  that may not have happened.
- **One failure message, and it is specific.** If `window.print()` rejects — which is what a
  missing permission looks like — the view says the print dialog could not be opened and names
  the platform, rather than reporting success.

### 11.5 Bulk without walking through every timetable

The brief asks for a bulk route that does not require opening each individual timetable. The
Download dialogue is already it: *Every teacher* is the default when the current scope is
Teachers, so bulk is the default rather than a separate feature. A second entry point — Download
from the timetable **list** row, which opens the same dialogue against that timetable — covers
"produce the lot for Draft 3" without opening Draft 3. **No third mechanism.**

### 11.6 Accessibility

The existing bar: `role="group"` with `aria-label` on the scope picker, `aria-pressed` on
toggles, `aria-current` on the day strip, a keyboard path to everything the mouse reaches
(`TIMETABLE-VIEWS.md` §11, and M9A's requirement that drag has a keyboard equivalent).

- Every control in the dialogue is a labelled native element — radio group, `<select>`,
  checkbox — reachable by Tab, with a visible focus ring; the entity multi-select reuses
  `MultiCombobox` (`ui/MultiCombobox.tsx`), which is already keyboard-tested.
- The dialogue is the existing `Modal`: focus is trapped, Escape closes, focus returns to the
  button that opened it.
- The two ⓘ lines live in an `aria-live="polite"` region so a size or disclosure change is
  announced when the selection changes.
- The print view's table keeps `<th scope>` headers and a `<caption>` naming the entity and the
  panel — the same semantics the on-screen grid relies on, and the reason
  `TIMETABLE-VIEWS.md` §9 refuses a virtualised table.
- A conflict is never colour alone, on screen or on paper (§6.2).

---

## 12. CLI and Tauri contracts

### 12.1 CLI — recommended, and narrow

**Recommended**, for the reason the CLI exists: *"so that solving, benchmarking and project
migration can run without a window"* (`crates/evara-cli/src/main.rs:3-5`). A school producing
every teacher's timetable each term is the same kind of job, and a CLI caller is the second caller
that keeps the renderer honest — Stage 3 of M10 made exactly this argument and it held.

**The name.** `export` is taken by the project archive and must stay taken.
`report` is free, matches the crate, and reads correctly: `evara report`. `score` stays reserved
for its planned meaning (`main.rs:44-52`).

```
evara report --project <path.evaraproj> --timetable <id> \
             --view teacher [--entity <id>]... [--all] \
             --format xlsx|csv \
             --to <file> \
             [--campus <id>] [--days 1,2,3] [--detail full|standard|compact]
             [--paper a4|letter] [--orientation landscape|portrait]
             [--no-logo] [--no-unscheduled] [--no-findings] [--json]

evara report --project <path> --list        # what this project's timetables are
```

- **Read-only on the project**, through `Database::open_for_inspection` — migrating nothing,
  writing nothing to the project, exactly as `validate --project` and `generate --project` do.
  A `SchemaUpgradeRequired` is reported, not worked around.
- **Writes exactly one file**, the one `--to` names, through §10.1's claim-and-rename. An
  occupied destination is refused rather than overwritten, as `export` is.
- **Exit codes follow the existing discipline** — a code per outcome, and "the command worked and
  the answer was no" never shares a code with "the command failed":

  | Code | Means |
  | --- | --- |
  | 0 | the document was written |
  | 1 | written, and it reports something the reader must see — an unplaced occurrence, a hard finding, a broken run |
  | 2 | the command failed: bad arguments, unreadable project, occupied destination, write error |
  | 3 | refused: the timetable's problem could not be compiled, so no grid exists |

  Code 1 is the `validate`/`evaluate` pattern (`main.rs:116-131`) applied to output, and it is
  what makes `evara report` usable in a termly script that must not silently ship a timetable
  with a hole in it.
- Argument parsing follows the existing hand-rolled style; no `clap`.
- **No print subcommand.** Printing needs a window. A CLI that wrote HTML nobody paginates would
  be a fourth output mechanism with no reader.

### 12.2 Tauri commands — three, and no new session

```rust
/// What this build can write, so the dialogue does not hardcode a list.
#[tauri::command]
fn report_formats() -> SetupResult<Vec<ReportFormatView>>;

/// The composed document for one request. What the print view renders.
#[tauri::command]
fn timetable_report_document(
    state: State<'_, AppState>,
    id: Uuid,
    request: ReportRequest,
) -> SetupResult<OutputDocument>;

/// Writes one document to one path the user chose. The only command here that writes.
#[tauri::command]
fn timetable_report_write(
    state: State<'_, AppState>,
    id: Uuid,
    request: ReportRequest,
    destination: String,
) -> SetupResult<ReportWriteView>;
```

```rust
pub struct ReportRequest {        // the IPC face of OutputOptions
    pub scope_kind: ScopeKind,
    pub scope_ids: Vec<Uuid>,     // empty = every entity of that kind
    pub schedule_ids: Vec<Uuid>,  // empty = every panel
    pub day_ordinals: Vec<u16>,   // empty = whole cycle
    pub format: Option<String>,   // "csv" | "xlsx"; None for the print document
    pub detail: String,
    pub include_non_teaching: bool,
    pub include_logo: bool,
    pub include_unscheduled: bool,
    pub include_findings: bool,
    pub paper: String,
    pub orientation: String,
}

pub struct ReportWriteView {
    pub path: String,
    pub bytes: i64,
    pub sheets: usize,
    pub entities: usize,
    pub lessons: usize,
    /// Counts a caller must surface rather than discover (§4.4).
    pub unscheduled: usize,
    pub conflicts: usize,
    pub defective: usize,
    pub hard_findings: usize,
    /// The shape §8.3 chose, and whether the caller's preference was overridden.
    pub shape: String,
    pub disclosures: Vec<String>,
}
```

Design notes, each answering a question the brief asks:

- **File selection is the frontend's**, via `tauri_plugin_dialog`'s `save()`, as both transfer
  screens do. The capability set already allows it; `open({ directory: true })` would also work
  under the existing `dialog:allow-open`, but §8.2 means no folder is ever needed.
- **Errors are `SetupError`**, the existing enum (`commands/error.rs`). The mapping:
  `Invalid` for a bad request, `NotFound` for a timetable or cycle that is gone, `Failed` for a
  write failure and for a destination that is occupied — with the backend's own sentence, which
  the screens then phrase an *offer* around and never re-word, exactly as Stage 5 settled.
  No new error kind is needed; if the Download dialogue needs to branch on "name taken" the way
  `isDestinationConflict` does today, one additional variant is cheaper than parsing a message,
  and that is a Stage 4 call.
- **The lock is released before rendering.** The command takes the workspace mutex, reads the
  row, compiles the `Assembly`, builds the `LabelSet`, reads the arrangement and the flags, and
  **drops the guard**; then projects, composes, renders and writes. That is `ARCHITECTURE.md`
  §3.3's division and the same thing `timetable_generate_start` does — and `evara_db::
  scheduling`'s `send_assertions` test already proves the types can cross a thread.
- **No progress and no cancellation in Stage 2**, per §14.4's gate. If the gate trips, a session
  on `TransferSessions`' pattern with a `report://` event is added in Stage 4 — and the
  `Coalescer` is reused rather than copied, which is why it was generalised in M10 Stage 4.
- **Typegen.** `OutputDocument`, `ReportRequest`, `ReportWriteView` and every type they reach go
  behind the `ts` feature and into `typegen.rs`'s `contract!` list.
  `check_dependencies_are_listed` fails the build if one is missed (ADR 0009), so the list is a
  deliberate edit rather than a thing to remember.

### 12.3 One renderer, two callers

`evara report` and `timetable_report_write` both call `output::compose` and the same renderer, and
neither owns any part of the pipeline. That is M10 Stage 3's property restated — *"the first
callers of Stage 2's API, with no protocol of their own"* — and it is what makes a second caller
free instead of a thing to keep in step.

---

## 13. Dependency assessment

### 13.1 Nothing new for CSV and Excel

| Need | Crate | Already in tree? | Licence | Verdict |
| --- | --- | --- | --- | --- |
| CSV writing | `csv = "1"` (`Cargo.toml:137`) | yes, `evara-import` | MIT OR Unlicense | **Reuse.** New edge: `evara-report` |
| Workbook writing | `rust_xlsxwriter = "0.102"`, `default-features = false` (`Cargo.toml:146`) | yes, `evara-import` | MIT | **Reuse.** New edge: `evara-report` |
| Logo bytes in a workbook | `rust_xlsxwriter::Image::new_from_buffer` | yes | — | **Reuse** |
| ZIP, if bulk ever needs it | `zip = "9"` | yes, `evara-project` | MIT | **Not needed** (§8.2) |
| Printing | WebView2 / WKWebView via `wry 0.57` through `tauri 2.12.1` | yes | Apache-2.0 OR MIT | **Reuse.** One capability line (§7.4) |

Both new edges are additions to an existing workspace dependency in a new crate, so
`cargo deny check licenses` has already accepted both, no new transitive tree arrives, and
`Cargo.lock` does not move. Neither crate does I/O in the way `evara-report` uses it: `csv::Writer`
over a `Vec<u8>` and `Workbook::save_to_buffer()`, which is the pattern `evara-import` already
uses (`template.rs:189, 255`).

**No `rust_xlsxwriter` features are enabled.** `serde`, `chrono`, `polars`, `constant_memory` and
`enhanced_autofit` are all deliberately off: §6.2 explains `constant_memory`, and the rest would
be features for a problem M11 does not have.

### 13.2 The one optional dependency, and the recommendation against it

`tauri-plugin-opener` (Apache-2.0 OR MIT, official, maintained in `tauri-apps/plugins-workspace`)
would let "Open the file" and "Show in folder" follow a successful download, and is §7.6's second
fallback if printing fails.

**Recommendation: not in M11.** The download dialogue reporting the path it wrote is enough, and a
plugin adopted for a convenience becomes the shape of a fallback nobody chose. It is named here so
that if the §7.5 spike fails, the fallback has a known, permissive, already-vetted dependency
rather than a search.

### 13.3 Policy

All of §7 and §13 is offline. No service, no API, no font download, no subscription. Windows and
macOS support is the same support the existing dependencies already have, since nothing new is
added. `OPEN-DECISIONS.md` R7 names *"PDF and font libraries specifically"* as the likeliest route
for an incidental GPL dependency, and M11 adds neither — which is a reason to keep D10's webview
decision beyond its convenience.

---

## 14. Performance targets and measurement strategy

### 14.1 The budget M11 inherits

`OPEN-DECISIONS.md` R5's upper end is a 2 000-activity secondary school. Already measured
(`TIMETABLE-VIEWS.md` §9, `evara-report`'s `projection::budget`, debug build):

| Figure | Measured |
| --- | --- |
| project 2 000 occurrences, 40 rooms, 2 schedules, dense | **3.8 ms** |
| lay out four scope grids | **417 µs** (160 cells) |
| one scope grid | **40 cells**, 5 days × 8 periods |
| cells per scope grid as the school grows 10× | **unchanged** |

The last row is what makes bulk output tractable at all: 400 teacher grids is 400 × 40 cells, not
400 × the school. §5.6's `CellLines` addition widens `ProjectedLesson` by roughly 100–200 bytes
per lesson — about 0.2–0.4 MB at 2 000 lessons — which is the only cost M11 adds to a path that
already exists, and it is below the noise of the IPC payload that path already sends.

### 14.2 The one real performance defect found — and it blocks bulk students

`projection::in_scope` (`projection.rs:812-840`) for a student scope:

```rust
(ScopeKind::Student, Some(id)) => labels
    .students
    .iter()
    .find(|s| s.student_id == id)        // O(students)
    .is_some_and(|s| s.sections.contains(&lesson.section_id)),
```

`lay_out` calls `in_scope` once per lesson, so one student's grid is `O(students × lessons)` and
every student's is `O(students² × lessons)`. At 3 000 students and 2 000 lessons that is ~1.8 × 10¹⁰
comparisons — minutes to hours, not a slow export. `LabelSet::section()` and `room()`
(`display.rs:177-186`) are linear too, used by `build_scopes` per section and per room, which is
`O(sections²)` on a path that already runs on every projection.

Today this is invisible: the desktop lays out **one** scope at a time. Bulk output is the first
caller that would multiply it.

**PROPOSED, Stage 1:**

1. A `StudentIndex` built once from the `LabelSet` — `BTreeMap<Uuid, &StudentLabel>` on the model
   of the existing `section_index()` / `room_index()` — and an `in_scope_indexed` that takes it.
   `in_scope` stays as the one definition of membership and delegates, so there is still exactly
   one implementation of *"is this lesson in this scope?"*.
2. `lay_out_many(&projection, &[ProjectionScope], &labels)` building the index once for the whole
   batch. Bulk becomes `O(students × lessons)` overall.
3. A regression test in `projection::budget` asserting that laying out **every** scope of a
   2 000-occurrence, 3 000-student school completes within a hang-detector bound, and printing the
   figure the way the existing budget test does.

### 14.3 Estimated output sizes — **targets to be measured, not measurements**

Every number below is an estimate from the cell counts above. Stage 2 replaces each with a
measured figure, in a budget test beside the existing one, and this table is rewritten from it.

| Case | Entities | Estimated cells | Estimated size | Target |
| --- | --- | --- | --- | --- |
| one teacher, XLSX | 1 | ~40 | < 20 KB | < 50 ms |
| one teacher, CSV | 1 | ~40 rows | < 5 KB | < 20 ms |
| every teacher, XLSX, 2 000-occurrence school | ~80 | ~3 200 | ~0.5–1.5 MB | **< 1.5 s** |
| every class section, XLSX | ~500 | ~20 000 | ~3–8 MB | < 3 s |
| every room, XLSX | ~40 | ~1 600 | < 1 MB | < 1 s |
| every student, long-form XLSX, 3 000 students | 3 000 | ~120 000 rows | ~10–25 MB | **< 10 s** |
| every student, grid-per-sheet, 3 000 students | 3 000 | ~120 000 | ~100–300 MB | **declined by default** (§8.3) |
| master grid, print | 1 | 40 cells, 2 000 stacked lessons | — | pagination is the question, not time |

The second-to-last row is why §8.3's threshold exists, and the last row is why the master grid's
printed form needs the §7.5 spike's answer rather than a size estimate.

### 14.4 The measurement gate — a decision deferred to data, not to preference

**Stage 2 ships synchronous output.** Stage 2 also ships the budget test above. The gate:

> If the **every-teacher XLSX** case at the R5 upper end exceeds **1.5 s** wall-clock in a release
> build on either platform, Stage 4 adds a report session — `Lifecycle`, `CancelFlag`, a
> `report://` progress event through the existing `Coalescer`, and a progress bar with Cancel.
> Below 1.5 s it ships with a busy state and no cancellation.

1.5 s is the threshold at which a frozen window stops reading as a click and starts reading as a
hang. Writing the gate down is what keeps §4.6 of the brief — *"do not introduce asynchronous
infrastructure where measured workload does not justify it"* — from being decided by taste in a
review.

Two things hold either way: the workspace lock is released before rendering (§12.2), so other
commands are never blocked for the duration; and the Tauri command runs on a pool thread, so a
synchronous render does not block the webview's event loop.

### 14.5 What will remain unmeasured, stated plainly

- **Print layout at scale on a real window.** §7.5's spike is manual and its result is an
  observation, not a CI figure. §15.5 keeps it in the manual column.
- **`Scheduling::load` wall-clock.** Still the open figure from R5, untouched by M11, and now on
  the critical path of a bulk export — which is a reason to measure it, recorded as a risk
  (§19, R-6) rather than quietly adopted as fine.
- **A real school's printer.** Nobody has printed an Evara timetable on a school's photocopier.
  §15.5.

---

## 15. Test matrix

`npm run check` is the gate (`fmt`, `cargo lint`, `cargo test --workspace --all-features`,
`types:check`, `tsc`, `vitest`). Semantic assertions throughout; **no binary snapshot of an
`.xlsx`** — a ZIP of XML whose byte order is the library's business, and a test asserting it would
fail on every `rust_xlsxwriter` bump while proving nothing about the document. The workbook is
**read back** with `calamine` (already a dev-reachable dependency through `evara-import`) and
asserted by cell.

### 15.1 `evara-report` — the composition and the renderers

| # | Test | Asserts |
| --- | --- | --- |
| T-1 | every one of M2's **24 fixtures** composes and renders in both formats | nothing in the catalogue produces an error |
| T-2 | **conservation**, per fixture, per format | lessons in = lessons out; panels, conflicts, defective runs and unscheduled occurrences all survive. §4.4, and the test that makes it a property |
| T-3 | `custom-rotating-cycle` | columns are `ordinal` order; no weekday text appears anywhere in the output. A grep over the rendered CSV, which is the only way to prove the negative |
| T-4 | `two-campuses-different-bells` | two panels, each naming its campus and schedule; **no merged axis**; periods are not interleaved |
| T-5 | `two-campuses-identical-bells` | still two panels. Identical periods are not evidence of one schedule |
| T-6 | a multi-period occurrence | XLSX `merge_range` height == `CellContent::Lesson { rows }`; CSV emits `parts` rows with `part` 1..=n |
| T-7 | a **broken run** (`unmaterialised-bell-schedule`) | no merge; every occupied cell carries the lesson with its part index; `run_defect` is populated |
| T-8 | two lessons in one teacher cell | `occupants = 2`, two CSV rows, one XLSX cell with **both**, conflict format, `GridConflict` in the appendix. Nothing collapsed |
| T-9 | the **master** grid with several lessons per cell | `PinReason::Shared`, not `Conflict`; no conflict appendix entry |
| T-10 | `empty-grid` | the document renders; sheets are present and marked empty; no panic, no empty file |
| T-11 | breaks | a non-teaching period is a row in both formats; `is_teaching = no`; never dropped |
| T-12 | **formula injection** | `=`, `+`, `-`, `@`, tab and CR leading a teacher name, a room name, a section name and a timetable name are defused in CSV **and** in XLSX |
| T-13 | **mutation check on the guard** | removing `defuse_formula` turns T-12 red in both formats. The device `evara-db`'s materialiser already uses to prove a check has teeth |
| T-14 | CSV escaping | a comma, a double quote, a newline and an emoji in a display name round-trip through `csv`'s reader |
| T-15 | **encoding** | the file begins with a UTF-8 BOM; non-ASCII names survive a read-back |
| T-16 | **worksheet names** | 31-UTF-16 truncation on a char boundary; each of `* ? : [ ] \ /`; a leading and a trailing apostrophe; a name of only forbidden characters; three identical names dedup to ` (2)`, ` (3)`; a CJK name truncating mid-codepoint |
| T-17 | `safe_file_stem` | `CON`, `con.csv`, `COM1`, `LPT1`, `nul`, each reserved word as a *suffix* (allowed), `<>:"/\|?*`, a trailing dot, a trailing space, an all-forbidden name, a 400-character name |
| T-18 | multiple teachers | all of them appear; `teacher_count` is right; a single-teacher lesson has no trailing separator |
| T-19 | the **one-room** limitation | the format doc's statement matches the model: `Placement::room` is `Option`, and one column is complete |
| T-20 | **unscheduled** occurrences | present in both formats; `is_impossible` matches `UnscheduledView`'s gating; an unrecognised cause key is **not** reported as impossible |
| T-21 | findings | the rendered sentence is byte-identical to the rule's own `explain()`. ADR 0043, asserted rather than trusted |
| T-22 | `shared-teacher-across-schools` | the disclosure is present and names the other school; it cannot be switched off by any `OutputOptions` value |
| T-23 | options | each of `schedule_ids`, `day_ordinals`, `include_non_teaching`, `detail`, `include_*` changes the document as specified; an empty selection is `EmptySelection`, not an empty file |
| T-24 | **determinism** | the same document rendered twice is byte-identical. Mandatory for XLSX: the writer must not embed a timestamp, which `evara-import`'s templates already depend on |
| T-25 | locked / manual | `LessonFlags` reach a visible marker in both formats |
| T-26 | `roster-without-group` / no enrolments | `has_enrolments == false` → the student view is refused with the sentence, never an empty picker or a blank grid |
| T-27 | a student's grid | exactly the sections `class_enrolment` names; **never** a group's membership or a group's size. The one test whose failure is a child being shown a guess |
| T-28 | `CellLines` | the composed caption equals `SectionLabel::short`; an unlabelled section falls back to the id and not to a placeholder name |

### 15.2 Projection consistency across formats — the brief's first row

| # | Test | Asserts |
| --- | --- | --- |
| T-29 | one document → CSV and XLSX | the set of `(entity, day, period, activity_id)` tuples is **identical** between the two |
| T-30 | the same document → the print view's props | the React test renders from the same `OutputDocument` JSON and produces the same tuples. This is what makes "the view, the CSV, the workbook and the page agree" a test rather than a claim |
| T-31 | `ScopeGrid` → `OutputSheet` | one `OutputCell` per `TimetableCell`, same order, same content kind. `compose` adds nothing and loses nothing |

### 15.3 Command layer and CLI

| # | Test | Asserts |
| --- | --- | --- |
| T-32 | `timetable_report_write` writes exactly one file, at the path given | and no `.part`, no placeholder, nothing else in the folder |
| T-33 | an **occupied destination** is refused | including a zero-length one, and **nothing is overwritten** |
| T-34 | a destination removed by `require_safe_destination` | refused before any file is created at it. Judge, then claim |
| T-35 | a write failure mid-render | the folder is byte-for-byte as it was found |
| T-36 | the lock is released before rendering | a second command completes while a large report renders |
| T-37 | a timetable whose problem will not compile | `SetupError`, the backend's own sentence, no file |
| T-38 | `evara report` exit codes | 0 / 1 / 2 / 3 against a fixture project, one case each |
| T-39 | `evara report` is read-only | the project is byte-identical afterwards, and **no `-wal` or `-shm` appears** — the assertion M10's export test already makes |
| T-40 | `evara report --list` | names the timetables with their identifiers |
| T-41 | CLI and command produce **identical bytes** for one request | the one-renderer-two-callers property, asserted |
| T-42 | cancellation, **if** §14.4's gate trips | a cancelled report leaves no file, no `.part` and no claim |

### 15.4 Frontend

| # | Test | Asserts |
| --- | --- | --- |
| T-43 | the Download dialogue | every control labelled; Tab reaches each; Escape closes; focus returns to the opener |
| T-44 | format switching | page setup appears for `.xlsx` and is absent for `.csv` |
| T-45 | the size and shape notice | §8.3's threshold crossing changes the notice; the `aria-live` region announces it |
| T-46 | the disclosure notice | shown when a selected entity is in `shared` |
| T-47 | the print view | no navigation, no buttons, no editing affordance in the printed DOM; `@media print` hides the toolbar |
| T-48 | a `window.print()` rejection | the specific failure message, and **no success message** |
| T-49 | `ipc/report.test.ts` | the command names and argument keys, as `ipc/transfer.test.ts` does |
| T-50 | the print view's table semantics | `<th scope>` and `<caption>` survive; a conflict is marked by text as well as by class |

### 15.5 Manual, and **not** claimed as verified

Deliberately separated, per the brief, and **manual GUI testing stays deferred**.

| # | Check | Platform |
| --- | --- | --- |
| M-1 | Print reaches a dialog **with** `core:webview:allow-print` | Windows, macOS |
| M-2 | Print does nothing **without** it | macOS — confirms §7.2's diagnosis rather than assuming it |
| M-3 | Save as PDF produces a correct multi-page document | Windows (WebView2 preview), macOS (AppKit panel) |
| M-4 | `@page` size and orientation are honoured | both |
| M-5 | `thead` repeats; rows do not split | both |
| M-6 | a 10-day custom cycle is legible on landscape A4 | both |
| M-7 | a bulk workbook opens in Excel, LibreOffice and Numbers; merges, widths, wrapping and print setup survive | both |
| M-8 | a monochrome print is readable: conflicts and breaks distinguishable | both |
| M-9 | a real printer | both |

**No statement anywhere in this repository may say native printing is verified until M-1 to M-5
have been run and recorded.** M10 Stage 5's limitation 1 — nothing in CI drives a real Tauri
window — is unchanged by M11, and M11 must not narrow it by implication.

---

## 16. Implementation stages

The brief's four stages survive, with **one insertion** and **two moves**, each explained. The
brief invites a better dependency order where inspection reveals one.

### Stage 0 — Print feasibility spike *(inserted)*

The §7.5 spike, before Stage 1. **Why it moves ahead of everything:** §7.3's two assumptions are
the only ones in this document that can change the architecture rather than the implementation. If
`@page` and repeated headers do not survive WebKit, Stage 3 becomes §7.6's comparison instead of a
stylesheet, and Stage 1's document model may need a page-break vocabulary it would otherwise not
carry. Discovering that in Stage 3 means rebuilding Stage 1.

Deliverable: the five answers written into §7 of this document, and the permission line proven
necessary and sufficient. A day or two, mostly manual. No production code.

### Stage 1 — Output foundation

- `evara_report::output`: `OutputDocument` and its types, `OutputOptions`, `OutputError`,
  `DocumentRenderer`.
- `output::compose`, with §4.4's two rules and §4.2's six judgements.
- `CellLines`, and `ProjectedLesson::lines` populated by `project` (§5.6).
- **The §14.2 index fix** and `lay_out_many`, with the regression test. *Moved into Stage 1 from
  "Performance verification" in Stage 2*, because it is not a performance improvement — bulk
  output of students does not function without it.
- `output::guard::defuse_formula` and `output::names::safe_file_stem`, with their test matrices.
  *Moved into Stage 1 from Stage 2's "File safety"*, because both are pure functions that the
  renderers call and both want their tables green before a renderer exists.
- T-1, T-2, T-3, T-28, T-31, and the Stage 1 halves of T-12/T-13, T-16, T-17.

Exit: a document composes for all 24 fixtures, conservation holds, nothing renders yet.

### Stage 2 — CSV and Excel

- `output::csv` to §6.1's column contract; `output::xlsx` to §6.2.
- `csv` and `rust_xlsxwriter` added to `evara-report`.
- Bulk, both shapes, with §8.3's threshold and §8.4's empty case.
- `timetable_report_document`, `timetable_report_write`, `report_formats`; typegen.
- `DestinationClaim` wired at the command layer per §10.1.
- `evara report`, as the second caller.
- The §14.3 budget test and **the §14.4 gate decided on its figures**.
- T-4 … T-27, T-29, T-32 … T-42.

Exit: both formats, individually and in bulk, from the window and from the CLI, with measured
figures and a recorded gate decision.

### Stage 3 — Print and PDF

- `print.css`: `@page`, `@media print`, the monochrome rules, breaks per panel and per entity.
  **One stylesheet with per-scope modifiers**, not one per view (§2.9).
- `PrintView.tsx` over `OutputDocument`; branding; the header and footer blocks.
- `core:webview:allow-print` in `capabilities/default.json`, commented.
- `window.print()`, one call site.
- T-47, T-48, T-50; M-1 … M-6 run and **recorded**.
- The platform compatibility assessment written up, replacing §7's assumptions with observations.

Exit: a timetable prints and saves as PDF on both platforms, or §7.6's fallback is chosen with the
spike's evidence behind it.

### Stage 4 — Desktop integration and closure

- The Download dialogue and the Print action on the timetable screen and the list row.
- Bulk from the list row (§11.5).
- The session and progress, **only if §14.4's gate tripped**.
- T-43 … T-46, T-49.
- `docs/TIMETABLE-OUTPUT.md` — what shipped, as `TIMETABLE-VIEWS.md` is for M8.
- `docs/CSV-OUTPUT-FORMAT.md` — the column contract, its stability promise, and the one-room and
  omitted-empty-cell limitations stated by name.
- The ADRs of §17; `ROADMAP.md` Phase 10 closed; `OPEN-DECISIONS.md` D10 and R5 amended;
  `TIMETABLE-VIEWS.md` §16's "No print, PDF or CSV" row struck; `README.md`'s status line
  updated; `CLAUDE.md` given its output bullet.
- M1-19 closed, or the gap named (§18).

### Why no stage is earlier than it looks

CSV before Excel within Stage 2, because the column contract *is* the long-form document model and
the workbook's long-form shape reuses it. Print after both, because the print view consumes the
document those two proved out — but its *feasibility* is Stage 0, which is the whole point of
splitting the spike from the stage.

---

## 17. Proposed ADRs

Numbering continues from 0059.

| # | Title | Decides, and what it rejects |
| --- | --- | --- |
| **0060** | *An output document is composed once and rendered many times* | The shared artefact is `OutputDocument`, not a renderer trait spanning all three formats. Rejects: one trait for CSV, Excel and HTML; and per-format composition, which drifts. Records why the print view is a consumer rather than an implementation (§5.5) |
| **0061** | *A cell's caption has one implementation* | `CellLines` is composed in Rust and read by the screen, the CSV, the workbook and the page. Rejects: TypeScript composing the on-screen caption while Rust composes the exported one (§5.6) |
| **0062** | *Bulk output is one document, never many files* | One workbook, one CSV, one printed document. Rejects: a file per entity, and a ZIP. Records that this is what removes generated filenames, and that `zip` remains available (§8.2, §10.2) |
| **0063** | *A campus is an axis, not a scope* | The brief's campus-level view is the master grid panelled per bell schedule. Rejects: `ScopeKind::Campus`, which would require the merged axis ADR 0033 forbids (§3.1) |
| **0064** | *Printing goes through the platform's own pipeline, and macOS needs a permission* | D10 upheld for M11, with the `core:webview:allow-print` finding, the `respondsToSelector` silent fallback, and the fact that page setup must be CSS because macOS exposes no programmatic orientation. Rejects: a Rust PDF renderer in M11; `PrintToPdf` via `unsafe` FFI (§7) |
| **0065** | *A destination claim is a general-purpose device, not part of the transfer protocol* | `DestinationClaim` is reused from `evara-project` by the command layer; `evara-report` returns bytes and writes nothing. Rejects: a fourth implementation of the claim; and moving it into a new crate to avoid an edge that does not exist (§10.1) |
| **0066** | *An exported timetable discloses what it does not contain* | Unscheduled occurrences, conflicts, broken runs, cross-campus commitments and the roster-derived caveat are required sections, not options. Rejects: a clean-looking document that omits them; and an option to suppress the cross-campus sentence (§4.4, §9.3) |

**Amendments rather than new records:** `OPEN-DECISIONS.md` D10 (its risk statement, per §2.9) and
R5 (its grid-rendering row extended with output figures); `ROADMAP.md` Phase 10's "a print
stylesheet per view" (§2.9); `TIMETABLE-VIEWS.md` §16's limitations table.

---

## 18. Phase 10 acceptance criteria

Proposed, in the shape Phase 9's table takes, so closure is an evidence table rather than a
judgement.

| Criterion | Evidence required |
| --- | --- |
| Every supported view exports to CSV | T-1, T-23, T-29 over all five scope kinds; the column contract documented |
| Every supported view exports to Excel | T-1, T-16, T-29; read back with `calamine`, asserted by cell |
| CSV and Excel agree with the on-screen grid | T-29, T-30, T-31 — the same `(entity, day, period, activity)` tuples in all three |
| Nothing is silently omitted | T-2's conservation property, per fixture, per format; T-8, T-20 |
| Breaks, multi-period runs, clashes and broken runs are represented | T-6, T-7, T-8, T-11 |
| Weekly and custom cycles both work, and nothing reads a weekday | T-3, including the grep for weekday text |
| Multi-campus output is panelled, never merged | T-4, T-5 |
| A document cannot contain another school's data | structural (§9.1); asserted over `shared-teacher-across-schools` in T-22 |
| A student's timetable is roster-derived and says so | T-26, T-27 |
| Bulk output covers every selected entity | T-23; both shapes of §8.3; no silent cap |
| A bulk export cannot mix timetable states | structural (§8.5): one projection per document |
| Spreadsheet content is safe | T-12 **and** T-13's mutation check, in both formats |
| No existing file is ever overwritten | T-33, including the zero-length case |
| A failed or refused output leaves nothing behind | T-32, T-34, T-35, T-42 |
| A timetable prints | M-1, M-3 on both platforms, **recorded** |
| Save as PDF works | M-3, both platforms, recorded |
| A4 and orientation are honoured | M-4, or the limitation documented with the spike's evidence |
| Repeated headers and sane pagination | M-5, M-6, or §7.6's narrowed promise documented |
| Printed output carries no application chrome | T-47 |
| Reachable by a user, and distinguishable from the archive export | T-43 … T-46; §11.1's three-way vocabulary |
| Reachable headlessly | T-38 … T-41 |
| Works offline | no network code path exists; `cargo deny` sources clean |
| A renderer trait exists with more than one implementation | `DocumentRenderer`, CSV and XLSX — which is what `lib.rs:40-46` asked for |
| `npm run check` green | CI |

**M1-19 — "print and export timetable views"** (`README.md`'s wording) closes when the export rows
are green and M-1, M-3 are recorded as passing. If the §7.5 spike forces §7.6's first or second
option, M1-19 still closes — a timetable still prints and still saves as PDF — and the *route* is
what changed. If printing cannot be made to work on either platform at all, M1-19 does **not**
close on the export rows alone, and that must be said rather than interpreted away. The precedent
is Phase 9's treatment of M1-17/M1-18: the interpretation is recorded, not assumed.

---

## 19. Risks and alternatives

| # | Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- | --- |
| **R-1** | **WebKit does not honour `@page size`/orientation**, so macOS differs from Windows | medium | orientation becomes the user's problem in the panel on macOS | Stage 0 settles it. Fallback: a landscape-biased layout that reads acceptably either way, and the panel's own orientation control documented as the macOS route |
| **R-2** | **Pagination and repeated headers do not survive** — D10's original fear | medium | a long timetable prints unusably | Stage 0. Fallback ladder in §7.6, cheapest first: one panel per page before any new dependency |
| **R-3** | A school's master grid of 2 000 lessons is **unprintable at any page size** | high — and not really a risk, a fact | the master grid is a screen artefact, not a page one | Printing defaults to a *per-entity* document; the master grid prints as one page per day, and the dialogue says so. Not a surprise discovered at the printer |
| **R-4** | **The bulk student case is slow even after §14.2** | medium | a 10-minute export | §14.3's target, measured in Stage 2; §14.4's gate adds progress and cancellation if it trips; §8.3's threshold keeps the default shape cheap |
| **R-5** | **Excel disagrees with LibreOffice or Numbers** about merges, widths or print setup | medium | a workbook that looks wrong in the school's actual tool | M-7, all three readers. Print setup is documented as **advisory** rather than implied to be fixed |
| **R-6** | `Scheduling::load` is still unmeasured (R5's open figure) and is now on a bulk export's critical path | medium | a bulk export's cost is dominated by a figure nobody has | measure it in Stage 2's budget test — M11 is the milestone with the clearest reason to, and it closes R5's last row |
| **R-7** | The **CSV column contract** turns out wrong after schools use it | medium | either a breaking change or a bad format kept forever | append-only from the start, documented as such, `view_entity_id`/`activity_id` present so a consumer never keys on a name. The contract is a document (`CSV-OUTPUT-FORMAT.md`), not a comment |
| **R-8** | Adding `core:webview:allow-print` **widens the window's capability surface** | low | a page could invoke print | the page is ours, `script-src 'self'` admits no third-party script, and the capability enables a print dialog and nothing else. Recorded in ADR 0064 rather than adopted silently |
| **R-9** | `CellLines` on `ProjectedLesson` grows the IPC payload for every existing screen | low | ~0.2–0.4 MB at the upper end | measured in Stage 1; if it matters, `lines` becomes a flag on `ProjectionInput` — but a conditional field is worse than 300 KB and should not be reached for first |
| **R-10** | **A printed timetable is mistaken for a teacher's whole week** when they work at two campuses | medium | a real scheduling mistake in a real school | §9.3's disclosure is mandatory and unsuppressible; T-22 asserts it |

### Alternatives considered and rejected

| Alternative | Why not |
| --- | --- |
| **One renderer trait across CSV, Excel and print HTML** | The roadmap's literal wording, and wrong: it would force the print format to be "bytes of HTML", describing the one format users care about least accurately (§5.5) |
| **Generate print HTML in Rust** | Would put the markup and the stylesheet in different languages and different crates, and duplicate what `TimetableGrid.tsx` already does well. The document over IPC gets the same consistency guarantee more cheaply |
| **Render CSV and Excel in TypeScript from the existing `ScopeGrid`** | A second composition of every cell, in the layer `CLAUDE.md` §2 keeps rules out of. Also puts formula-injection defence in the renderer most likely to be rewritten |
| **A file per entity, in a ZIP** | §8.2. Buys nothing the workbook does not, and brings back every filename problem §10.2 removes |
| **A Rust vector-PDF renderer in M11** | §7.6. Weeks of layout work, a font licence question, and R7's named GPL-risk area — to replace a pipeline that, on this inspection, needs one permission string |
| **A session, progress and cancellation from the start** | The brief forbids unjustified asynchronous infrastructure, and 3.8 ms per projection says it is unjustified until measured otherwise. §14.4 makes the decision data's rather than taste's |
| **A campus scope kind** | §3.1, ADR 0033. Would require asserting that two campuses' Period 3 is one row |
| **A general page-size list, custom margins, a template editor** | A report designer, which §3.2 excludes. Two paper sizes and two orientations is what the brief asks for |

---

## 20. Decisions requiring product approval

Twelve. The first four change the plan's shape; the rest are confirmations that are cheap now and
expensive after Stage 2.

| # | Decision | Recommendation | If rejected |
| --- | --- | --- | --- |
| **A1** | **Stage 0 print spike before Stage 1** | **Approve.** §7.3's assumptions can change the architecture, not just the implementation | Stage 3 carries the risk, and a failure there means rebuilding Stage 1's document model |
| **A2** | **"Campus-level view" = the master grid panelled per bell schedule**, not a new scope kind | **Approve.** ADR 0033; §3.1 | A campus scope needs a merged axis, which is a scheduling-semantics change this milestone excludes |
| **A3** | **Bulk output is one document per format. No per-entity files, no ZIP** | **Approve.** §8.2 removes a whole class of file-safety work rather than defending against it | Reinstates generated filenames, reserved-name handling, collision policy and partial-cleanup-across-N-files |
| **A4** | **D10 upheld: webview printing, with `core:webview:allow-print` added** | **Approve**, conditional on A1's result. No `unsafe`, no new dependency, each platform's own Save as PDF | §7.6's ladder, priced there; a Rust PDF renderer is weeks and touches R7's risk area |
| **A5** | **The 60-entity threshold** between sheet-per-entity and long-form XLSX | **Approve**, overridable, never silent | A different number, or a prompt every time. The requirement is that it is visible, not what it is |
| **A6** | **CSV omits empty teaching cells by default** | **Approve.** Positions stay addressable by day/period ordinal, so absence is unambiguous | Files roughly 3–5× larger for a half-full timetable; `include_empty` already exists either way |
| **A7** | **A UTF-8 BOM on CSV output** | **Approve.** Three bytes that make the Windows-Excel path correct | Non-ASCII names mangle for the majority of this audience |
| **A8** | **The CSV column contract is append-only from day one** | **Approve.** §19 R-7 | A format that can be reordered is one that will be, and then cannot be |
| **A9** | **Cross-campus disclosure is mandatory and unsuppressible** | **Approve.** §9.3, R-10 | An option to produce a misleading document |
| **A10** | **`evara report` in the CLI**, and the name `report` | **Approve.** §12.1; `export` stays the archive's | No headless route, and no second caller keeping the renderer honest |
| **A11** | **No success message after printing** | **Approve.** §7.2 proved the return value proves nothing on macOS | A dialogue claiming a print that may not have happened |
| **A12** | **`CellLines` added to `ProjectedLesson`**, with `TimetableGrid.tsx` switched to it | **Approve.** §5.6; the screen and the page cannot drift | Two compositions of every caption, in two languages |

### Blockers

**None to writing Stage 1.** Two things must be settled before the stages that depend on them:

1. **A1's spike must run before Stage 3 is planned in detail**, and should run before Stage 1 is
   written. It needs a macOS machine and a Windows machine with a printer or a PDF destination.
   This is the only item on the critical path that this proposal cannot resolve by inspection.
2. **A2, A3 and A4 should be answered before Stage 1 starts**, because each changes what
   `OutputDocument` has to carry. A5 to A12 can be answered during Stage 1.

**Not a blocker but carried forward:** M10 Stage 5's limitation 1 — no CI test drives a real Tauri
window — applies to M11's print path with more force than it did to transfer, because printing is
the one feature whose entire value is in what the window does. M11 does not close that gap and
must not imply it has.

---

## Appendix — provenance of every verified claim

| Claim | Source |
| --- | --- |
| Tauri replaces `window.print` on macOS/iOS | `tauri-2.12.1/src/webview/scripts/print.js`; `src/webview/plugin.rs:198-202` |
| `core:webview:default` grants four permissions, not `allow-print` | `tauri-2.12.1/permissions/webview/autogenerated/reference.md:1-11`, and `allow-print` at `:154` |
| Our capability set asks for `core:default` only | `apps/desktop/src-tauri/capabilities/default.json` |
| `Webview::print()` takes no options | `tauri-2.12.1/src/webview/mod.rs:1626` |
| macOS print goes through `NSPrintOperation`, macOS 11+, margins only, silent fallback, no completion callback | `wry-0.57.0/src/wkwebview/mod.rs:879-918` |
| Windows `print()` *is* `window.print()` | `wry-0.57.0/src/webview2/mod.rs:1796-1802` |
| Our macOS floor is 11.0 | `apps/desktop/src-tauri/tauri.conf.json`, `bundle.macOS.minimumSystemVersion` |
| WebView2 print APIs and preview behaviour | [Printing from WebView2 apps][ms-print] |
| WKWebView printing on macOS, and `NSPrintOperation` as the route | [Apple: printing WKWebView to PDF][apple-pdf], [printing in Sequoia][apple-seq] |
| The same defect in other Tauri 2 apps | [Pake #1396][pake], [gitpass #384][gitpass] |
| Worksheet name rules: non-empty, ≤31 UTF-16, no `* ? : [ ] \ /`, no edge apostrophe, unique | `rust_xlsxwriter-0.102.0/src/utility.rs:1112-1138`; `src/error.rs:44-56` |
| Print setup, merge, image and row-height APIs available with `default-features = false` | `rust_xlsxwriter-0.102.0/src/worksheet.rs`; `Cargo.toml` features list — none of them gate these |
| `Image::new_from_buffer` takes PNG, JPEG, GIF, BMP | `rust_xlsxwriter-0.102.0/src/image.rs:1226-1241` |
| `csv` and `rust_xlsxwriter` already in the workspace | `Cargo.toml:137,146`; used by `crates/evara-import` |
| `zip 9` already in the workspace | `Cargo.toml:159`; `crates/evara-project/Cargo.toml` |
| The window CSP admits `data:` images | `apps/desktop/src-tauri/tauri.conf.json`, `app.security.csp` |
| Only the dialog plugin is installed | `apps/desktop/src-tauri/src/lib.rs:50`; `capabilities/default.json` |
| `tauri-plugin-opener` is official and permissive | [Tauri plugins workspace][plugins], [Opener][opener] |
| Projection measured figures | `docs/TIMETABLE-VIEWS.md` §9; `crates/evara-report/src/projection/budget.rs` |
| `in_scope`'s linear student scan | `crates/evara-report/src/projection.rs:812-840` |
| `lay_out` skips an empty axis | `crates/evara-report/src/projection.rs:863-866` |
| `project` skips unplaced occurrences | `crates/evara-report/src/projection.rs:629-631` |
| One room per placement | `crates/evara-core/src/scheduling/placement.rs:38-51` |
| Resources are not in the problem | `crates/evara-core/src/scheduling/problem.rs:395` |
| `DestinationClaim` and the judge-then-claim order | `crates/evara-project/src/claim.rs` |
| `require_safe_destination`'s four rules | `crates/evara-project/src/paths.rs:47-92` |
| 24 scheduling fixtures, including `custom-rotating-cycle` and `two-campuses-different-bells` | `crates/evara-core/src/scheduling/fixtures/catalogue.rs` |
| `SchoolLogo` is what a Rust export should use | `apps/desktop/src-tauri/src/commands/structure.rs:132-133` |

[ms-print]: https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/print
[apple-pdf]: https://developer.apple.com/forums/thread/705138
[apple-seq]: https://developer.apple.com/forums/thread/803249
[pake]: https://github.com/tw93/Pake/issues/1396
[gitpass]: https://github.com/itsd0/gitpass/issues/384
[plugins]: https://github.com/tauri-apps/plugins-workspace
[opener]: https://v2.tauri.app/plugin/opener/
