# Open Decisions and Risks

Decisions that are **not** settled, and risks worth naming before code depends on them.
Resolved items move into [adr/](adr/).

---

## Decisions needed before coding

### D1 — Solver engine: Rust in-process vs. JVM sidecar ✅ RESOLVED 2026-10-06

**Decided: Rust, in-process.** See [adr/0002](adr/0002-rust-in-process-solver.md).
Approved with two conditions, both now recorded in the ADR: keep the `SolverEngine`
abstraction so CP-SAT or another engine can be introduced later without redesigning the
application, and share the constraint evaluators across generation, scoring, manual move
validation and violation explanation.

Original analysis, retained: **Rust, in-process.** Full analysis in [SOLVER.md](SOLVER.md) §1. The
deciding arguments are that the manual-move validator must share the constraint code with
the generator, and that Tauri already requires Rust while a JVM sidecar adds a bundled
runtime, a second notarisation target and 1–3 s of startup latency. Timefold's
multi-threaded solving is also Enterprise-only and commercial, which does not fit an
open-source project.

### D2 — Rust-owned data layer vs. TypeScript-owned

**Recommendation: Rust owns the database and the domain; TypeScript is the view layer**,
with DTO types generated from Rust into `packages/domain`.

Cost: a meaningful amount of Rust CRUD. (The toolchain itself is no longer a cost —
Rust 1.99 was installed in Phase 0.) The alternative — `tauri-plugin-sql` with queries in
TypeScript — would be faster to start and worse within weeks: the solver would then need
its own loader anyway, and transactions would be spread across UI handlers.

### D3 — Type generation tool ✅ RESOLVED 2026-10-06

**Decided: `ts-rs`**, with the per-command wrappers staying hand-written. Reviewed in
Phase 1C once real entities existed; see
[adr/0009](adr/0009-typescript-type-generation.md). The deciding facts: `tauri-specta`
is still `2.0.0-rc.25` with no stable release, and its main benefit — generating the
call layer — would replace wrappers that already do more, because they validate the
payload at runtime rather than asserting a type over it.

Not wired up in Phase 1C by design; generation arrives in Phase 2 with the first
screens that consume entities.

### D4 — SQLite driver and migration runner ✅ RESOLVED 2026-10-06

**Decided: `rusqlite` 0.37 (`bundled`) + `refinery` 0.10.** Implemented in Phase 1A; see
[adr/0007](adr/0007-rusqlite-refinery.md) for the reasoning and its consequences —
notably that released migrations are immutable, and that `refinery` pins `rusqlite` 0.37
so we do not take 0.40 until it moves.

### D5 — Group-based vs. request-based scheduling first ✅ RESOLVED 2026-10-06

**Decided: group-based first, request-based schema preserved from day one.** See
[adr/0006](adr/0006-group-based-scheduling-first.md). Both traditions must ultimately be
supported. Original analysis retained below.

These are different optimisation problems, not variations:

- **Group-based** (aSc, TimeTabler, much of Europe/Australia): classes are pre-formed;
  the solver places lessons. This is what [SOLVER.md](SOLVER.md) describes.
- **Request-based** (PowerScheduler, Veracross, Edval electives, much of the US): students
  submit course requests; the system must *build* sections, assign students to them
  (**sectioning**), construct elective lines, and schedule — typically as separate
  interacting phases.

**Recommendation:** model both in the database from day one (the schema in
[DATA-MODEL.md](DATA-MODEL.md) does), but solve group-based in Milestone 1 and add a
sectioning phase in Milestone 2. Sectioning on top of a working group-based engine is an
addition; retro-fitting the data model later is not.

~~This needs your answer on market priority.~~ Answered: build group-based first, and
preserve the schema for individual requests, weighted requests, alternate requests,
elective line generation, sectioning, class balancing and student allocation. No
request-based algorithm is implemented in Milestone 1.

### D6 — Multi-period activities: one activity or several linked ✅ RESOLVED 2026-10-15 (Stage C)

**Decided as recommended: one activity with `duration_periods = N`** occupying consecutive
periods, shipped in `V15`. The alternative (N single-period activities + a `CONSECUTIVE`
link) makes "double period" a rule that can be switched off, which it is not. Consequence:
every evaluator must reason about slot *ranges*, and room/teacher occupancy indexes are
built over spans.

Not to be confused with **D30**, which is the orthogonal question of how many *occurrences*
a cycle's demand is. A double period is one activity of duration 2; three doubles a cycle is
three such activities. The two decisions together are ADR 0029 §2.

### D7 — Is the teacher a planning variable?

Milestone 1 fixes teachers on activities (`activity_teacher`) and plans timeslot + room.
Real staffing ("find any qualified Maths teacher who is free") needs teacher as a third
planning variable, which changes move generation and adds `teacher-qualified` and
`teacher-balanced-load` to the hot path. **Recommendation:** keep teachers fixed in M1,
but shape `Problem`/`Placement` so a third variable is additive rather than a rewrite.

### D8 — UUID version and storage

**Recommendation: UUIDv7 stored as `TEXT`.** v7 is time-ordered, so primary-key index
locality is good and insertion order is recoverable — both useful for a future change
feed. `TEXT` over `BLOB(16)` costs ~20 bytes per key but keeps the database readable in
any SQLite browser, which matters for debugging and for user-submitted project files.
Reconsider only if a measured index-size problem appears.

### D9 — Undo/redo

Options: in-memory inverse-command stack (simple, lost on close), a persisted command
log (survives restart, enables an audit trail, more work), or SQLite savepoints (natural
for transactions, awkward across UI sessions). **Recommendation:** in-memory inverse
commands for M1, designed so a persisted log can back it later. `change_log` already
captures the data-level history.

### D10 — PDF generation

Webview print-to-PDF (free, zero dependencies, uses the print stylesheet we need anyway,
but gives limited control over pagination and page headers) vs. a Rust renderer
(`printpdf`/`typst` — real control, real work). **Recommendation:** webview printing for
M1, a renderer later behind `evara-report`'s renderer trait. Flagged because
multi-page timetable printing with repeated headers is a common hard requirement in this
product category and webview printing may not survive contact with it.

### D11 — macOS distribution

Notarisation requires an Apple Developer account (~US$99/year) and CI secrets. Without
it, macOS users must bypass Gatekeeper manually, which for a school-admin audience is
effectively a blocker. Windows code signing has a similar cost. Not a dependency problem
— a distribution-cost decision that needs an owner before Phase 11.

### D12 — Student clash semantics when membership is not modelled ✅ RESOLVED 2026-10-21 (M1)

**Three answers, not two: `Overlaps`, `Disjoint`, `Indeterminate`.**

Recorded as [adr/0035](adr/0035-student-group-overlap-is-a-three-state-answer.md). Only
`Overlaps` is a hard `student-clash`; `Disjoint` must be *proved*; `Indeterminate` is never
silently converted to either, is not enforced, and is reported with the count of answers
that would clear it.

Overlap is proved from facts first and declarations second — shared effective membership
over the group's subtree (filtered to students who are `is_active` and hold a non-`LEFT`
spell), containment through `parent_group_id`, and a pigeonhole test on declared sizes
within a common ancestor. Disjointness is proved only by provable emptiness or by one future
nullable column, `student_group.children_are_disjoint`, asked **per parent** rather than per
pair.

The record also establishes what this decision was open for: the hierarchy alone **cannot**
express two different divisions of one cohort, so `V10:124-126`'s assumption that sibling
teaching sets do not clash is true of three classes and false of three sets of different
subjects. That is why a column is required rather than optional, and why it is carried by
M5's migration. `SOLVER.md` §2's *"expanded to leaf student sets"* and `CONSTRAINTS.md`'s
*"resolved to leaf student sets"* are corrected by the same record: an expansion to students
would make `student-clash` silent for exactly the schools [ADR 0006](adr/0006-group-based-scheduling-first.md)
protects.

The fixture this entry asked for is specified — sixteen cases plus a twenty-row truth table
— and **M2 built it**: nine of the 24 fixtures in `evara_core::scheduling::fixtures` carry
the group-overlap cases, each recording the verdict *and* which of ADR 0035 §3's seven rules
decides it, so M5 can assert the route as well as the answer. The three-state `Overlap` and
the three-state `DisjointnessDeclaration` are declared; the function that derives them is
still M5's, and the column is still unwritten. See
[SCHEDULING-FIXTURES.md](SCHEDULING-FIXTURES.md).

### D13 — Minimum supported Rust version ✅ RESOLVED 2026-10-06 (Phase 0)

Set to **1.90**, matching Tauri 2.12. Started at 1.85 (the edition-2024 floor), which the
MSRV-aware resolver then used to hold the whole tree back — including three security
advisories it refused to resolve past. Nobody compiles Evara with an old toolchain
because we ship the binary, so a conservative MSRV costs patches and buys nothing.

### D14 — cargo-deny policy for unmaintained crates ✅ RESOLVED 2026-10-06 (Phase 0)

Vulnerabilities always fail the build. "Unmaintained" is scoped to workspace
dependencies (`unmaintained = "workspace"`), because `proc-macro-error` is reachable only
through Tauri's Linux GTK backend — not compiled on Windows or macOS, build-time only,
and with no upgrade available. Failing CI on something unfixable trains people to ignore
the check.

---

### D15 — Project file extension ✅ RESOLVED 2026-10-06

The portable export is `*.evara` ([ADR 0004](adr/0004-evara-project-format.md)). The
*live* project database needs its own extension so the two are never confused — opening a
raw SQLite file expecting an archive, or mailing a WAL-mode database to a colleague, are
both bad outcomes.

**Decided: `.evaraproj` for the live editable project; `.evara` reserved for the portable
exchange package.** The distinction is intentional and enforced in code: a live project is
a working SQLite database with WAL sidecar files, an export is a self-contained archive.
`Workspace` rejects any other extension, including `.evara`, with
`DbError::WrongExtension`. The Phase 1A placeholder `.evdb` is gone from the codebase.

### D16 — Where `site_id` really belongs ✅ RESOLVED 2026-10-06

`site_id` identifies the *installation*, but Phase 1A stores it inside the project file,
because no application-level settings database exists yet. The consequence is visible:
open a project created on another machine and it keeps that machine's `site_id`, so the
change feed will attribute your edits to them.

**Decided and implemented in Phase 1B.** `site_id` identifies an installation and now
lives in the application settings database. A project stores `project_id` and
`created_by_site_id` (historical, never updated). The installation doing the writing is
supplied per connection through the application-defined `evara_site_id()` SQL function,
which the change-log triggers call — see
[adr/0008](adr/0008-installation-identity-and-project-lifecycle.md). Migration V2 performs
the correction; V1 was left untouched.

### D17 — Who maintains `updated_at` and `rev` ✅ RESOLVED 2026-10-06

[DATA-MODEL.md](DATA-MODEL.md) originally said `updated_at` is "maintained by trigger".
Phase 1A deliberately did **not** do that: a trigger that updates its own table interacts
with SQLite's `recursive_triggers` setting in ways that are easy to get subtly wrong, and
the failure mode — a change logged twice, or not at all — is exactly what the change feed
exists to prevent.

**Decided: repositories maintain `rev` and `updated_at` explicitly.** No self-updating
SQLite triggers for those fields. The change-log triggers only *observe* the values the
repository wrote. This is predictable and testable, and it avoids the `recursive_triggers`
interaction whose failure mode — a change logged twice, or not at all — is exactly what
the change feed exists to prevent.

Enforcement lands with the first repository in Phase 1C: a shared update helper, plus a
test asserting no entity table is written without bumping `rev`.

**Stage B made this load-bearing rather than tidy.** `rev` now has two readers — the change
feed and the write path, which puts it in every update's `WHERE` clause (D28). A trigger
that bumped `rev` would make every second write in a transaction conflict with itself, so
"no self-updating triggers" is now a correctness requirement and not only a preference.

### D18 — Where the project lifecycle lives ✅ RESOLVED 2026-10-06 (Phase 1B)

`Workspace` is in `evara-db` rather than in a new orchestration crate or in the Tauri
command layer. It coordinates two SQLite databases, and SQL belongs in exactly one crate;
putting it in `src-tauri` would have made the lifecycle untestable without a window and
would have broken the "no business logic in command handlers" rule.

Revisit if a second non-SQL concern attaches itself to the lifecycle, at which point an
`evara-app` crate becomes the right home.

### D19 — Structure deletion semantics ✅ RESOLVED 2026-10-06 (Phase 1C)

DATA-MODEL.md §3 documented the Structure parent links but never stated `ON DELETE`
behaviour, so it was decided explicitly rather than defaulted:

- **Ownership edges are RESTRICT** — campus→school, building→campus, room→campus,
  resource→school, resource→campus, academic_year→school, term→academic_year. Deleting a
  parent that still has children fails and says what is in the way.
- **Optional classification references are SET NULL** — `room.building_id` and
  `room.room_type_id`. The room is the asset; where it sits and what kind it is are
  metadata.

`resource.campus_id` is RESTRICT despite being optional, so that "delete a campus" means
the same thing on every edge. See
[adr/0010](adr/0010-structure-deletion-semantics.md).

### D20 — Ambiguities found in DATA-MODEL.md §3 ✅ RESOLVED 2026-10-06 (Phase 1C)

Resolved while implementing, and now reflected in the document:

| Gap | Resolution |
| --- | --- |
| No uniqueness scopes except `campus(school_id, code)` | Applied the documented rule — codes are unique within their parent: `room_type(school_id, code)`, `room(campus_id, code)`, `resource(school_id, code)`, `academic_year(school_id, name)`, `term(academic_year_id, ordinal)` and `(academic_year_id, name)` |
| `building` has no `code` | Its `name` is unique within the campus instead |
| Is `school` a singleton? | **No longer.** It was, enforced with a constant column and a unique index; `V12` removed both and [ADR 0019](adr/0019-multiple-schools-in-one-project.md) supersedes ADR 0003's claim. A project may describe several schools that share staff and facilities |
| `room` names both a campus and an optional building, which could disagree | Enforced by a trigger. A composite foreign key cannot be used because `ON DELETE SET NULL` would try to null the NOT NULL `campus_id` |
| `deleted_at` is "only on tables a future sync must tombstone" — which ones? | None in Phase 1C. Deletes are hard, guarded by RESTRICT, and remain attributable through `change_log`. Revisit when synchronisation is designed |

### D21 — Timeslot identity, and ambiguities in DATA-MODEL.md's time model ✅ RESOLVED 2026-10-09 (Phase 1D)

The identity rule had to be settled before `V4` was written, because a released migration
is immutable and reassigning slot identifiers is silent corruption (R6). Decided:
**logical identity is `(term_id, cycle_day_id, period_id)`, and `timeslot.id` is assigned
once and never reassigned while that tuple exists.** See
[adr/0011](adr/0011-stable-timeslot-identity.md).

Four further ambiguities were put to the maintainer rather than defaulted:

| Gap | Resolution |
| --- | --- |
| `timeslot.term_id` is nullable, so the documented `UNIQUE(cycle_day_id, period_id, term_id)` does not constrain the term-less rows at all — SQLite treats `NULL`s as distinct | `NULL` keeps its meaning, "the whole year", and identity is enforced by a unique index over `COALESCE(term_id, '')`. The plain three-column constraint is not also created: redundant where the term is set, useless where it is not |
| `ON DELETE` for `calendar_day.cycle_day_id` | `RESTRICT`. `SET NULL` was available, since `NULL` means "no lessons that day", but it would silently reinterpret mapped teaching dates as non-teaching ones |
| Whether `is_default` is constrained | At most one, enforced in the schema: one default cycle per school, one default period structure per campus — with the school-wide `NULL` campus as its own bucket, via the same `COALESCE` technique |
| What `cycle.day_count` means next to the actual `cycle_day` rows | The *declared* length. The materialiser uses the rows that exist and the repository refuses an ordinal beyond `day_count`, but does not demand the set be complete or contiguous — so building a cycle one day at a time stays legal. No cross-table trigger |

Two things were decided while implementing, by applying rules this document already
states rather than by inventing policy, and are recorded here for visibility:

- `cycle_day(cycle_id, label)` and `period(period_structure_id, label)` are unique,
  applying D20's rule that a name is unique within its parent (as `building` already
  does). Two "Day A"s in one cycle is a mistake, not a configuration.
- `calendar_day(school_id, date, COALESCE(campus_id, ''))` gets the same `COALESCE`
  treatment as `timeslot`, for the same reason: the campus-less rows are the common case.

### D22 — How cross-campus time is compared for a shared room ✅ RESOLVED 2026-10-21 (M1)

**Option A, within a school: compare `[starts_at, ends_at)` intervals, not `timeslot_id`.
The cross-school case is explicitly deferred.**

Recorded as [adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md), accepted
2026-10-21 after every premise below was re-verified against `V15`. Two placements of one
school conflict on a shared `room_id` or `resource_id` when they fall on the same `cycle_day`
and their wall-clock intervals overlap — which needs **no schema change**, no timezone
(one school has one zone, so it cancels on both sides) and no `calendar_day` coverage (both
bell schedules of a school materialise against the same `cycle_day` rows). Which schedule an
activity draws its times from is [adr/0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md).

Option B was rejected for refusing a setup [ADR 0014](adr/0014-rooms-shared-across-campuses.md)
deliberately supports; Option C was rejected as premature. The cross-school half is not
merely undesigned but **unavailable**: the workspace ships no timezone database and nothing
parses `school.timezone`, so it would need a new dependency and a licence review before any
algorithm. It is reported by name instead, per that record's rule 3.

**The premise correction this entry carried is applied below.** The original text read
*"`period_structure` is school-wide, and nothing ties a campus to exactly one"*. That
understated the model and is struck: `period_structure.campus_id` exists (`V4:82`), a
campus-scoped schedule is expressible, and `timeslot.campus_id` is derived from it under a
trigger pair (`V4:149-184`). The **conclusion was unaffected** — timeslot identity still
carries no campus, so two schedules still produce different `timeslot` rows for overlapping
minutes, which is exactly why the answer compares minutes.

The original statement of the problem is kept below, with that one sentence corrected, because
the three options and the `R-shared-room-time` risk it names are what the ADR argues against.

---

**Original entry, as it stood before resolution.** Settled before `room-clash` is
implemented for shared rooms.

`V6` made a room shareable across campuses
([adr/0014](adr/0014-rooms-shared-across-campuses.md)). A shared room is **one** room with
one `room.id`, so its occupancy is global: if one campus has booked it at a given time, no
other campus may. That part is decided and is recorded as a binding invariant in
[CONSTRAINTS.md](CONSTRAINTS.md) and ADR 0014.

What is *not* decided is what "at a given time" means across campuses.

- `period_structure` is **campus-scoped** — `campus_id` is nullable and `NULL` means
  school-wide (`V4:82`) — and `is_default` is unique per campus bucket rather than per
  school, so two campuses can timetable against different bell schedules and a school may
  hold both a school-wide schedule and a campus one at once. *(Corrected 2026-10-21; the
  original sentence claimed a campus could not be tied to a schedule at all.)*
- Those schedules can carry the same period ordinals with different wall-clock times.
  **Period 3 on North is not necessarily Period 3 on South.**
- A timeslot's identity is `(term_id, cycle_day_id, period_id)` with no campus in it
  ([adr/0011](adr/0011-stable-timeslot-identity.md)), so two campuses using different
  period structures produce *different* `timeslot` rows for overlapping real minutes.

So the obvious implementation of `room-clash` — "two activities in the same `timeslot_id`"
— is correct when both campuses share a period structure and **wrong exactly when the
sharing feature matters**, silently. It would report no violation for a hall genuinely
double-booked by two campuses running different bells.

Options, not yet weighed with the maintainer:

| Option | Shape | Cost |
| --- | --- | --- |
| **A — compare real times** | Give `period` an authoritative wall-clock interval and clash-check shared rooms by interval overlap within a `(term, cycle_day)` | Needs a decision on what the authoritative interval is, and on how two campuses' cycle days are aligned to each other at all |
| **B — require one bell schedule per shared room's campuses** | Refuse the share, in the repository, when the campuses do not agree on a period structure | Cheap and safe; may refuse a real school's setup, which is the kind of thing that makes a product unusable rather than merely limited |
| **C — a campus-alignment table** | Model explicitly which `(campus, cycle_day, period)` tuples overlap in time | Most honest, most work, and premature before anyone has asked for it |

Nothing is implemented in either direction. Per [CLAUDE.md](../CLAUDE.md) the constraint
evaluators have exactly one implementation, in `evara-constraints`, so a provisional
answer written anywhere else now would be the second one.

Related risk: **R-shared-room-time.** Until this is settled, a school that shares a room
between campuses on different bell schedules has no protection against double-booking it,
and will not be warned by anything. The model permits the configuration; the solver cannot
yet reason about it.

> **Status 2026-10-21.** The risk is **answered for the intra-school case and still carried
> for the cross-school one.** [adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md)
> rule 2 decides the comparison; nothing implements it, so the exposure lasts until M5 ships
> `room-clash`. Across schools the risk is permanent until a time alignment and a timezone
> database both exist, and rule 3 converts it from silent to announced.

**Widened by `V13`.** `resource_campus` ([adr/0022](adr/0022-resources-shared-across-campuses.md))
makes a resource shareable on the same terms, so whatever answers this for `room-clash`
answers it for a resource too. `teacher_school`
([adr/0021](adr/0021-project-level-people-and-school-membership.md)) makes it worse in
kind rather than in degree: two *schools* may be in different timezones, so comparing their
wall-clock bell times is wrong before any period structure is considered. A shared teacher
double-booked across two schools is the same unprotected configuration as a shared hall,
and is why cross-school scheduling is explicitly not claimed by Stage 0B.

---

### D23 — Is a teacher's staff code project-wide or per school? ✅ RESOLVED 2026-10-14 (Stage 0B)

**Neither, for now: it stays per school, and the honest model is per membership.**

The pre-implementation audit recommended project-wide — *"one person, one employee code"* —
and Stage 0B declined, keeping `teacher_code_unique (school_id, code)` from `V10`.

Two reasons, recorded in full in ADR 0021's *Known limitation*:

1. **Tightening it in a migration is unsafe.** Collisions are possible, SQLite has no
   conditional DDL, and a `.sql` migration can only create the project-wide index
   unconditionally. On a collision the migration aborts — which rewrites no identifier, the
   right half of the behaviour — but refinery's grouped transaction then leaves the project
   **unopenable**, and the user cannot edit the colliding codes in a project that will not
   open. A released migration is immutable, so the trap would be permanent.
2. **A staff number is the employer's identifier, not the person's.** A teacher shared
   between two schools of a trust may hold two payroll numbers, one per school, and a single
   project-wide `teacher.code` cannot express that at all.

The forward path is therefore a `code` on `teacher_school` alongside the existing column —
additive, no collision possible, and it says the true thing. Reserved for **Stage B** with
the rest of the staff model.

One narrow consequence is live and tested: at a student's **home** school, reissuing a
retired admission number is refused, because `student_code_unique (school_id, code)` spans
every student a school has ever entered and ADR 0021 §6's mirror brings it to bear. The same
operation at any other school succeeds. `V10`'s index is standalone, so a later migration can
relax it to current students; Stage 0B does not, because changing a shipped uniqueness rule
is its own decision.

### D24 — Is a "school" the institution, or is there a division below it? ✅ RESOLVED 2026-10-14 (Stage 0C)

**The institution. No `program` or `division` entity.**

The pre-implementation audit raised this as its first open decision and recommended the
institution; ADR 0019 rejected a sub-division as premature when it rebuilt `school`; Stage
0C is the point at which the interface had to commit, because a division would have been a
third level in every selector on every screen.

One `school` row per institution. A school's own **campuses** express its physical sites,
and `student_group` expresses the programmes inside it — a junior programme that shares
staff and rooms with the senior school but runs its own year and its own rotation is a
*second school* in the same campus file, which is exactly the arrangement
[ADR 0019](adr/0019-multiple-schools-in-one-project.md) exists for.

Adding a third level now would make every ownership question three-sided for something
nothing has asked for. A school-level answer can gain one later without redesign; the
reverse is not true. Recorded in [ADR 0023](adr/0023-the-interface-scopes-to-one-school.md)
§7.

### D25 — Which "active" does a school's screen show? ✅ RESOLVED 2026-10-14 (Stage 0C)

**The conjunction, with the two flags distinguished by name.**

Audit decision 3 settled the model half in Stage 0B — both flags, with distinct meanings
(ADR 0021 §2) — and left the interface half open. Stage 0C answers it:

- A teacher's Status column reads **"Archived"** when `teacher.is_active` is false,
  **"Not teaching here"** when the membership is inactive, and **"Active"** only when both
  hold. Three answers, not two, because there are two flags and the project-level one
  outranks: somebody archived is offered to nobody, whatever any school's employment record
  says.
- The project-level flag's own checkbox is labelled **"In this campus file"**, not
  "Currently teaching". It is an archive flag and the old wording claimed the per-school
  answer, which is the mistake the split exists to prevent.
- Whether *this* school employs them is changed under *Schools and campuses*, never by that
  checkbox.

The same shape applies to a student: the project-level flag archives the person, and the
spell's status is this school's own answer. `TeacherInSchool::is_effectively_active` stays
the single place the conjunction is computed for anything that has to *decide* rather than
display.

### D26 — Is `teacher_qualification` project-level or per school? ✅ RESOLVED 2026-10-15 (Stage B)

**Project-level, with the subject supplying the school.** Audit decision 6 recommended it
and flagged it as *"the one place the two scopes genuinely collide"*; Stage B implements it
and records the reasoning as [adr/0025](adr/0025-teacher-qualifications-across-school-owned-subjects.md).

`teacher_qualification (id, teacher_id, subject_id)` — no `school_id`, because
`subject.school_id` already says it, and no membership requirement, because competence does
not expire when employment does. A teacher shared between two schools who teaches
Mathematics at both holds **two** qualifications, one per subject record, and remains
**one** teacher.

A project-wide subject catalogue was the alternative and was rejected: it needs an owner
above the school, which ADR 0023 §7 settled there is none of; two schools' "Mathematics" are
not the same thing; and it would make the single-school case — most of them — maintain a
catalogue *and* an adoption list.

The rule this hands to Stage C, which is the point of settling it now: **assigning a lesson
requires the conjunction** — a qualification for the subject, an *active* membership at the
school, and campus eligibility where the lesson's campus is constrained.

### D27 — Where does a teacher's department live? ✅ RESOLVED 2026-10-15 (Stage B)

**Its own table, and `teacher.department_id` is not added at all.**

`V10` reserved the column and `V13` made it wrong: one column on a project-level identity
cannot say "Mathematics at Northgate, Science at Southmoor", and would force either a second
teacher record — the duplication `V13` removed — or an arbitrary choice of whose answer
survives. The audit's own `teacher_school.department_id` fixes the scope and not the
cardinality: Mathematics *and* Physics at one school is just as ordinary, and is how most
science faculties are staffed.

So `teacher_department (id, teacher_id, department_id)`, unique per *(teacher, department)*,
with the school reached through `department.school_id` — and held to membership by a trigger
pair, exactly as `V13` held campus eligibility.
[adr/0026](adr/0026-teacher-departments-are-a-narrowing-of-membership.md).

### D28 — What happens when two windows save the same record? ✅ RESOLVED 2026-10-15 (Stage B)

**The second is refused with a typed error, and nothing is merged.**

`ROADMAP.md` called this *"the last thing between this phase and done"* through three
phases. The revision goes in the statement's `WHERE` clause, so the check and the write are
one atomic operation rather than a read followed by a hopeful write — the sequence *read,
compare in Rust, write* is explicitly not what this is, because it is the same race with a
longer window.

44 of the 84 mutation commands take `expectedRev`: every category where a lost update is
possible. The other 40 are classified rather than omitted, and
[adr/0027](adr/0027-revisions-are-checked-on-write.md) §3 and §4 is the inventory, including
the four deliberate last-write-wins exceptions and the one residual gap.

**No merging, no field-level resolution, no notion of whose edit matters more.** The user is
told what happened and decides. A merge Evara performed itself would be a guess about
intent, which is the one thing a timetable cannot afford.

### D29 — Is an unqualified teacher assignment refused or warned about? ✅ RESOLVED 2026-10-15 (Stage C)

**Warned about. Employment and campus eligibility are the requirements; the qualification is
advisory.**

`ROADMAP.md` left this open deliberately and asked for an ADR, because ADR 0025 §4 had
written Stage C's rule as a conjunction of three conditions and a school mid-term with one
absent specialist wants the third to be a warning.

The decisive argument is not convenience. Given a database that refuses to timetable an
unqualified teacher and a school that must timetable one, what the school does is **add the
qualification** — so the field then says "she can teach A-level Chemistry" when what was
meant was "she is covering 9C on Tuesdays", and the one record the whole design rests on has
been corrupted to get past a check. A rule that is routinely worked around is worse than no
rule, because it leaves the data looking compliant.

So an **active** `teacher_school` membership and campus eligibility are refusals — each a
`V15` trigger pair *and* a repository check that can name the remedy — and a missing
`teacher_qualification` is reported by `Lessons::assignable_teachers`, labelled in the
picker, and shown as a banner naming the subject *before* the save.
[adr/0028](adr/0028-assigning-a-lesson-warns-about-qualifications.md) has the reasoning, what
"warning" means concretely, and the four alternatives rejected.

ADR 0025's **ownership** model is untouched: nothing in Stage C writes or removes a
qualification, and `tests_lessons::a_qualification_is_untouched_by_everything_this_stage_does`
asserts it. A school that wants enforcement gets it in Phase 4, as a `teacher-qualified`
constraint it can set to `HARD` per school — which is the honest home for a preference, and
the reason this decision forecloses nothing.

### D30 — Is a cycle's worth of lesson demand one row or several? ✅ RESOLVED 2026-10-15 (Stage C)

**Several. One `activity` row is one placeable occurrence, and there is no
`occurrences_per_cycle` column.**

A class needing four single periods a cycle holds four rows; one needing a double holds one
row with `duration_periods = 2`. The occurrence count is how many rows there are and the
demand is `SUM(duration_periods)`, which is a sum rather than a product — four singles and a
double is five occurrences and six period slots, and no single multiplication says that.

Three things depend on it, and [adr/0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
§2 sets them out: the solver's variable set becomes the rows with no expansion step;
`timetable_entry`'s `UNIQUE (timetable_id, activity_id, timeslot_id)` already assumes it,
*because* a multi-period activity occupies several slots; and the commonest real constraint in
this product category — "spread these four Mathematics periods across four different days" —
addresses the occurrences individually and needs them to be individual rows.

### D31 — Where does the spreadsheet import engine live? ✅ RESOLVED 2026-10-15 (Stage D)

**Three crates. A new `evara-import` for the bytes, `evara-core::import` for the vocabulary,
and the engine itself inside `evara-db`.**

This one was decided by reading the code rather than by preference. `Database::write` is
`pub(crate)` and hands out a `rusqlite::Transaction`, so **an atomic multi-entity write is
only expressible from inside `evara-db`** — a public repository method opens its own
transaction, and composing twelve of them would be twelve transactions and eleven places to
fail half-way. An import that created forty teachers and failed on the forty-first has to
leave none of them, so the engine has nowhere else to go.

The *parser* is not there, because it carries `calamine`, `csv` and `rust_xlsxwriter` and the
crate whose charter is SQL has no business linking three file-format libraries. The
*vocabulary* is in `evara-core` because two crates share it and neither can own it: a type
either of them owned would make the other depend on it, and both directions are wrong.

`evara-project` and `evara-integrations` were both considered and rejected on what they say
about themselves — the first is for `.evara` and says so, the second depends on `evara-core`
alone and so could not write at all.
[adr/0030](adr/0030-an-import-is-a-reviewed-plan.md) §1–2.

### D32 — When an imported row looks like a record Evara already holds, what happens? ✅ RESOLVED 2026-10-15 (Stage D)

**A code matches. A name asks.**

The two mistakes available here do not have symmetric costs, which is what settles it. A
duplicate is visible, embarrassing and repairable — the school sees two Ada Lovelaces and
deletes one. A wrong merge is invisible and permanent: the overwritten data is gone and
nothing records that two people were ever involved.

So a code is matched within its school, because that is what the uniqueness indexes already
make it; a name is evidence and produces a `Conflict` the user answers, with four equally
weighted answers and no default. Reuse of a person across schools is *offered* on the
narrowest evidence available — the same code **and** both names — and still never assumed,
because two schools in a trust can both number their staff from `T-0001`.

Fuzzy matching was rejected outright: it converts a conflict a user would have answered
correctly into a merge nobody reviewed, and the failure is silent.
[adr/0031](adr/0031-import-identity-matching.md).

### D33 — How is an activity row matched, when an activity has no natural key? ✅ RESOLVED 2026-10-15 (Stage D)

**By multiplicity, and an import never deletes one.**

ADR 0029 made one `activity` row one occurrence, so four single periods of a section are four
identical rows with nothing unique between them. An activities row is therefore matched by
counting how many occurrences of its shape — section, duration, label — the section already
holds: equal is `Unchanged`, fewer is a create of the difference with a warning naming the
arithmetic, and **more is a `Conflict`**.

The last arm is the decision. Deciding which two of six lessons to destroy, and destroying
their future placements with them, is not something a spreadsheet cell does; `split_index` is
never renumbered, and `tests_import::activities_are_counted_to_a_target_and_never_deleted`
asserts the count is still six after the refusal.
[adr/0031](adr/0031-import-identity-matching.md) §7.

The interface still asks the two numbers a school thinks in and states their product, so the
model's shape is not the user's problem. Writing `count` rows is a **creation** under
ADR 0027 §5, so no new concurrency model was needed — which is the practical payoff of rows
over a count.

Related, and settled with it: `course.periods_per_cycle` is a *declaration* and a section's
rows are the *actual* demand; a difference between them is reported as advice and refuses
nothing (ADR 0029 §3). And `class_enrolment` is a **roster overlay**, never added to the
attending groups' size (ADR 0029 §4), which is what keeps ADR 0006's promise that a timetable
can be built without a single student record.

---

### D34 — Which bell schedule does an activity schedule against? ✅ RESOLVED 2026-10-21 (M1)

**Exactly one, resolved by campus compatibility against the schedules actually materialised
into the grid. A non-unique answer is a refusal, never a guess.**

Recorded as [adr/0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md).
The solve derives `G`, the set of `period_structure` rows with slots in `(cycle, term)`; a
structure is compatible with a class section when it is school-wide, when it names the
section's campus, or when the section names no campus **and the school has exactly one**.
Exactly one compatible structure resolves; zero and more-than-one are hard pre-flight errors
that name the section, the cause and the remedy.

Option A was rejected: its fallback is silent and mishandles an ordinary configuration — a
multi-campus school whose sections leave `campus_id` null. Option C (a column on the
activity or section) was rejected as premature, and the record names the one future case
that would justify it: two bell schedules on one campus. The answer costs **no migration**.

Two findings came out of resolving it. `is_default` cannot serve as the tie-break, because
it is unique per *campus bucket* and a school may hold two defaults; and the interface
already contains the smaller version of that ambiguity at `GridPreviewSection.tsx:103`,
which picks the first `is_default` row in a name-ordered list. The second is survivable where
it is — materialisation is previewed and confirmed — and is the evidence that this decision
was not hypothetical. **No code was changed by M1**; fixing that default is a separate PR.

The grid is materialised per `(cycle, period_structure, term)` —
`MaterialisationRequest` in `repo/time_model.rs` names all three — so a school running two
campuses on two bell schedules holds **two disjoint sets of `timeslot` rows over the same
cycle days**. That is the solver's value range, and something has to choose which half of it
an activity may be placed in.

Nothing stored answers it:

- An `activity` reaches a campus only through `class_section.campus_id` (`V15`), which is
  **nullable** and means *not campus-constrained* — the ordinary state of a section at a
  single-campus school and a legitimate one elsewhere.
- `period_structure.campus_id` (`V4`) is nullable too and means *school-wide*, so a school
  may hold a school-wide schedule and a campus schedule at once, and `is_default` is per
  campus bucket rather than per school.
- `timeslot.campus_id` is derived from the period structure, so the two are consistent —
  but an activity is not joined to either.

This is not an edge case. It is the first question the loader asks, for every activity, in
every school. Options, not yet weighed:

| Option | Shape | Cost |
| --- | --- | --- |
| **A — resolve through the section's campus, default to the school-wide schedule** | `class_section.campus_id` picks the campus schedule; a section with no campus uses the school-wide default | Cheap, and silently wrong for a multi-campus school whose sections do not name a campus |
| **B — a solve names its bell schedule(s) and refuses an ambiguous activity** | The pre-flight check reports every activity that could belong to two grids, with the remedy ("give this section a campus") | Honest, and makes the user finish a setup they may consider finished |
| **C — the activity names its grid** | A column, or a per-solve mapping table | Most explicit, a migration, and a third place campus information lives |

The options above are the weighing that produced Option B; they are kept because the rejected
shapes are the useful part of the record.

**Implemented in M4, and in a way that honours the last sentence above.** A provisional answer
in the loader would indeed have been a scheduling rule in the crate that owns SQL — so the
rule is not there. `evara_core::scheduling::resolve` is the one implementation (ADR 0036), and
`evara_db::scheduling` *calls* it: it builds the context, derives `G` from the materialised
slots, and resolves once per section, which is why sections are the last table it assembles.
`Problem::integrity` reports any drift between the recorded answer and the relation, so a
second copy could not survive a test run.

---

### D35 — What does `room-capacity` compare against? ✅ RESOLVED 2026-10-21 (M1)

**Nothing yet: `room-capacity` leaves the first constraint set.** Expected attendance
becomes an explicit, optional `class_section.expected_attendance`; it is derived only from
declared group sizes over provably disjoint groups, and is `Unknown` otherwise.

Recorded as [adr/0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md).
**Honoured in M5:** `ConstraintType` has no `RoomCapacity` variant, so nothing claims to check
a room's size, and `CONSTRAINT-ENGINE.md` §9 lists it among what is deliberately absent. What
M5 did add is the left-hand side's missing half — `lessons::attendance` now sums a section's
groups where they are **proved** pairwise disjoint, which is clause 3 in full rather than the
one-group case M4 could reach.
The type is **not registered** in M5 and `CONSTRAINTS.md` moves it from `M1` to `M2+`,
because an evaluator written now could only be tested against a left-hand side that is
`Unknown` everywhere — failing [CLAUDE.md](../CLAUDE.md) §4's fourth requirement — and a
registered type that never fires tells the user their rooms are being checked.

`Unknown` is a third value, never `0` and never a pass: the constraint is **not evaluated**
for that activity, generation is not blocked, room assignment is not blocked, and the count
is reported in pre-flight and carried on the generated timetable. The reason it cannot be
`0` is already in the repository — `AttendingGroup.population` is
`size.unwrap_or(named_members)` (`repo/lessons.rs:1396`) and `ClassSectionDetail.tsx:535`
sums those, so a section whose groups declare no size displays an audience of `0`. That is
correct for the screen it is on and would satisfy every room in the school.

The name matters and was chosen rather than reused: `class_section.max_size` stays reserved
for M2 sectioning (`section-max-size`, `section-balance` in
[ADR 0006](adr/0006-group-based-scheduling-first.md)). A cap on allocation and an occupancy
figure for a room are different quantities that are usually different numbers.

[CONSTRAINTS.md](CONSTRAINTS.md) defines `room-capacity` as *"Room capacity ≥ activity's
required capacity"* and marks it `M1`. `room.capacity` exists and is `NOT NULL`. **The
right-hand side does not exist**, and none of the three candidates supplies it:

- `class_section.max_size` appears in DATA-MODEL.md §5's sketch and `V15` records that it
  was deliberately **not built**.
- `student_group.size` is nullable, so summing a section's `class_group` rows yields nothing
  for precisely the schools [ADR 0006](adr/0006-group-based-scheduling-first.md) promises
  will keep working.
- `class_enrolment` is a roster overlay with no scheduling meaning in Milestone 1
  ([ADR 0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §4), so
  counting it would contradict D12 as well as ADR 0006.

So the choice is: add one column (`class_section.max_size`, a nullable `ALTER TABLE`), derive
a figure and accept that it is absent for size-less groups, or **leave `room-capacity` out of
the first constraint set** and say so in CONSTRAINTS.md. Deciding before the registry is
written avoids shipping a registered `type_key` that cannot be evaluated — which would fail
[CLAUDE.md](../CLAUDE.md) §4's definition of done on its unit test.

---

### D36 — Is the first constraint set seeded as rows, or built into the registry? ✅ RESOLVED 2026-10-22 (M5)

**Decided: built into the registry, and no rows are seeded.** Recorded as
[adr/0039](adr/0039-a-mandatory-constraint-lives-in-the-registry.md).

Raised while implementing [ROADMAP.md](ROADMAP.md) Phase 4's *"the default constraint set
seeded into new projects"*. Three things were wrong with it, in ascending order: a seeded row
can be **deleted**, which produces timetables that double-book teachers and report no
violation; it can be **disabled**, which leaves the rule visible in the configuration list
while the engine does not run it; and it has **no honest author**, because `evara_site_id()`
is not registered while refinery applies a migration, so a row seeded in `V16` would be
unattributable in the change feed or refused outright.

So evaluation runs every implemented type whatever the database holds, and a
`constraint_instance` row *tunes* one — weight, term, parameters, and a level it may **raise**
but not weaken. Constraints remain data, which is what ADR 0005 decided and this does not
revisit; what is withheld is only the ability to turn a safety rule off. The configuration
list of an unconfigured school is empty, and that is the correct reading: it has not disagreed
with anything.

The related question — where a stored `params` value is validated, given that the JSON Schemas
live in a crate `evara-db` may not depend on — is settled in that record's §5 as a division of
labour rather than a dependency.

---

### D22 — note added 2026-10-16, discharged 2026-10-21

[adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) was **Proposed** when
this note was written. It is now **Accepted**, so D22 is closed for the intra-school case and
its cross-school case is deferred by that record rather than left open here. The premise
correction this note asked for has been applied to D22's text above and is marked there.

## Technical risks

### R1 — Incremental scoring correctness (highest)

A wrong delta score silently produces a wrong timetable, and it is the hardest part of
any solver to debug. **Mitigation:** the two-scorer design plus the property test in
[SOLVER.md](SOLVER.md) §4, in CI, as a gate on every new constraint. Non-negotiable.

**M5 shipped the reference half and nothing else, deliberately.** `evaluate` is the reference
scorer; `evaluate_placement` selects by `affected()` and then evaluates each selected rule in
full, holding no cache and no index — incremental-*friendly*, not incremental. The mitigation
above only works if the thing a delta is checked against exists first, so M6 inherits
`IncrementalScorer` **and** the property test in the same milestone. Neither half may land
without the other.

### R2 — Solver quality is open-ended work

"Generate a timetable" is binary; "generate a timetable a timetabler accepts" is years of
refinement, and incumbents have had decades. **Mitigation:** benchmark fixtures with
committed score baselines from Phase 6, so progress and regressions are measurable rather
than argued about. Set expectations that M1 produces a *feasible* timetable, not a
competitive one.

### R3 — Constraint count is the real scope

**Eight exist as of M5**, against the twenty-two `M1` names and the sixty-odd eventual ones —
and the per-constraint definition of done the mitigation below asks for is what the eight
were held to: a registry entry, a JSON Schema, documented semantics, an isolated test with a
fixture, and an `explain()`. `CONSTRAINTS.md` §3 marks which are real with ✅ M5, so a
coverage claim can be checked rather than believed.

~22 constraint types in M1, 60+ eventually, each needing an evaluator, an incremental
path, a param schema, an explanation, a test and a fixture. This dominates the effort —
more than the solver core. **Mitigation:** the registry pattern, and a strict
definition-of-done per constraint so coverage claims stay honest.

### R4 — Rust/TypeScript boundary churn

Every schema change touches a migration, a repository, a DTO, generated types, an IPC
wrapper and a form. **Mitigation:** generated types, generic CRUD conventions established
in Phase 2, and batching schema changes per phase rather than per field.

### R5 — Performance at scale, unknown until measured

A 2,000-activity secondary school with electives is the realistic upper end. Nothing in
the design should break, but load time, solve time and grid rendering are all unmeasured.
**Mitigation:** generate a large synthetic fixture in Phase 3 — *before* the solver — and
treat its numbers as a budget.

### R6 — Timeslot rematerialisation — *mitigation shipped in Phase 1D*

Changing the cycle or period structure after a timetable exists invalidates placements.
Silent data loss is the worst outcome. **Mitigation:** the materialiser preserves surviving
slot IDs, and any destructive change requires explicit confirmation showing exactly how
many entries would be orphaned.

Implemented in Phase 1D ([adr/0011](adr/0011-stable-timeslot-identity.md)): identity is
the `(term_id, cycle_day_id, period_id)` tuple, `apply_materialisation` never deletes,
orphan candidates come back in the returned plan with their identifiers, and removing them
is a separate call. A mutation check confirms the invariant has teeth — breaking
preservation in the materialiser fails eight tests.

**What is still open:** the confirmation *interface* does not exist yet, because there is
no UI for the time model and no `timetable_entry` to count. The repository exposes
everything that interface will need; nobody is obliged to call it until Phase 7.

### R7 — Licence hygiene

Apache-2.0 obliges us to keep shipped dependencies permissive. Tauri (MIT/Apache-2.0),
rusqlite (MIT), React (MIT) are fine. The risk is an incidental GPL/AGPL transitive
dependency — plausible in PDF and font libraries specifically. **Mitigation:**
`cargo-deny` licence allow-list in CI from Phase 0, and a generated third-party notice
file in Phase 11.

### R10 — A database-level SET NULL bypasses the revision contract

`ON DELETE SET NULL` modifies a row without incrementing `rev`, so the change-log entry
would record a stale revision. Repositories therefore clear `room.building_id` and
`room.room_type_id` with proper rev-bumping updates *before* deleting the parent, making
the schema rule a backstop that never fires in normal operation.

It would still fire for a write that bypassed the repository — which cannot happen today,
since a connection without `evara_site_id()` cannot write to a tracked table at all. Worth
revisiting if any future edge genuinely needs SET NULL on a hot path.

**Stage B chose `RESTRICT` rather than SET NULL for a new edge because of this risk.**
`subject.department_id` is shaped exactly like an optional classification reference and is
nevertheless `RESTRICT`, with `delete_department` refusing while any subject or teacher
refers to it. ADR 0025 §6 argues it: a `SET NULL` there would rewrite every affected subject
without incrementing a single revision, in the same stage that made the revision a
precondition. The remedy — clearing a subject's department — is an ordinary edit through a
real update, so each row's revision stays truthful.

### R8 — Single-file SQLite on synced folders — *detection and warning shipped*

Users will put project files in OneDrive, iCloud Drive or Dropbox. WAL-mode SQLite on a
cloud-synced folder is a known corruption source. **Mitigation:** detect common synced
paths and warn on open; make `.evara` export the blessed way to move a project; document
it. Worth deciding in Phase 1, not Phase 11.

### R9 — Fixture realism without real data

Synthetic fixtures are mandatory, and naive ones are too easy — they hide the pathologies
(uneven subject loads, specialist-room bottlenecks, part-time staff) that break real
timetables. **Mitigation:** a parameterised generator in `evara-cli` with knobs for
tightness, specialist-room scarcity and part-time fraction, plus hand-built adversarial
cases.

**Partly mitigated in M2, and the mitigation changed shape.** The generator is in
`evara_core::scheduling::fixtures` rather than `evara-cli`, because `evara-constraints` and
`evara-solver` must be able to use it and neither can depend on a binary crate. It took the
*hand-built adversarial cases* half of the mitigation and not the *knobs* half: 22 of the 24
fixtures are a named pathology each — an unresolvable bell schedule, a contradicted
disjointness declaration, an unbookable specialist room, an occurrence longer than any
teaching run — which a reviewer can read in full and a failing assertion can name. A dial
produces a school nobody can characterise, and "tightness 0.8 failed" is not a diagnosis.

**M4 widened the fixture surface without closing the risk either.** Twelve database-backed
fixtures now exist in `evara-db`, including a `medium` at the same scale written through the
repositories — so the *loader* is exercised at 125 sections and 500 occurrences. They are
hand-built named cases like M2's, for the same reason, and the one below still stands.

**The risk is not closed.** The `medium` fixture is deliberately comfortable — every class
needs 20 of its 30 slots, no teacher carries more than 8 occurrences — so it exercises scale
and not difficulty. A genuinely tight fixture is still wanted, and is best written once a
solver exists to be beaten by it, because until then nobody can tell a hard fixture from an
impossible one. Phase 6, with the score baselines.
