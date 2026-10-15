# Roadmap — Milestone 1

Milestone 1 is a **complete vertical slice**: create a project, define a school, generate
a timetable, inspect it, edit it by hand, understand why an edit is rejected, and move the
whole project to another machine.

The 19 acceptance items from the product brief are tracked as `M1-1` … `M1-19` and each
phase below states which it closes. Phases are ordered by dependency and each one ends
with something demonstrable — none of them is "build everything in one pass".

---

## Target repository structure

```
evara-timetables/
├── apps/
│   └── desktop/                    # Tauri 2 application (replaces apps/web)
│       ├── index.html
│       ├── vite.config.ts
│       ├── src/                    # React + TypeScript
│       │   ├── app/                # shell, routing, project lifecycle
│       │   ├── features/           # structure, people, curriculum, activities,
│       │   │                       # constraints, generate, timetable, reports
│       │   ├── ipc/                # typed Tauri command wrappers
│       │   └── ui/                 # primitives, grid, drag-and-drop
│       └── src-tauri/
│           ├── Cargo.toml
│           ├── tauri.conf.json
│           └── src/commands/       # thin: project, crud, solve, validate, report, io
├── crates/
│   ├── evara-core/                 # domain types, IDs, cycle/time model, Score (no I/O)
│   ├── evara-db/                   # SQLite: migrations, repositories, loader, writer
│   ├── evara-constraints/          # registry, evaluators, explainers, param schemas
│   ├── evara-solver/               # construction, local search, moves, scorers
│   ├── evara-project/              # .evara export/import + format migrations
│   ├── evara-report/               # view projections, HTML/CSV/PDF renderers
│   ├── evara-integrations/         # external adapters — empty, feature-gated
│   └── evara-cli/                  # headless solve / benchmark / migrate / fixture gen
├── packages/
│   └── domain/                     # GENERATED TypeScript types + UI-side guards
├── fixtures/                       # synthetic schools (JSON + generator + .evara)
├── docs/                           # this directory, plus docs/adr/
└── .github/workflows/              # CI: Windows + macOS, cargo + vitest + fixtures
```

`services/` disappears. `crates/` holds libraries, not services, because there is no
service.

---

## Phase 0 — Clear the ground ✅ complete

Removes what contradicts the architecture before anything is built on top of it.

- Scaffold the Cargo workspace and the Tauri 2 app at `apps/desktop`. Migrate the useful
  parts of `apps/web` (`index.html`, `styles.css` as a visual starting point) and delete
  `apps/web/src/main.tsx`'s hardcoded `lessons` mock.
- Delete `services/solver/`. Do **not** merge `feat/v0.1-solver-service`; tag it
  `archive/timefold-prototype` first so the prototype stays recoverable, then close it.
- Reduce `packages/domain/src/index.ts` to generated output. Nothing hand-written there.
- Extend `.gitignore` for Rust/Tauri (`target/`, `src-tauri/target/`, `*.evara`,
  `*.sqlite*`, `dist/`). Commit the root `package-lock.json` (currently untracked).
- CI on `windows-latest` and `macos-latest`: `cargo fmt --check`, `cargo clippy -D
  warnings`, `cargo test`, `tsc --noEmit`, frontend tests.
- `CLAUDE.md` / contributor notes recording the layering rules from
  [ARCHITECTURE.md](ARCHITECTURE.md) §3.

**Done when:** an empty Tauri window opens on both platforms from a clean clone, and CI is
green. No feature code yet.

**Outcome.** Workspace of 8 crates plus the Tauri app compiles; `npm run dev` opens the
window and a Phase 0 `app_info` command proves the IPC round trip end to end.
`cargo fmt`, `cargo clippy -D warnings`, `cargo test`, `tsc --noEmit`, `vitest` and the
Vite production build all pass on Windows. macOS is covered by CI but has not been run on
hardware yet. `services/` and `apps/web` are gone; the Java prototype survives only as the
constraint semantics recorded in `docs/CONSTRAINTS.md`.

---

## Phase 1 — Project lifecycle and persistence

Split in three.

**Phase 1A — persistence foundations ✅ complete.** Connection factory with verified
pragmas, forward-only embedded migrations with grouped transactions and a typed
newer-schema refusal, project identity in `app_meta`, and the `change_log` table with its
reusable trigger generator. No entity tables.

**Phase 1B — settings and project lifecycle ✅ complete.** Installation settings database
(`site_id`, recent projects); migration V2 correcting project identity to `project_id` +
`created_by_site_id`; per-connection `evara_site_id()` attribution; `Workspace` with
create/open/close/current/recent over `.evaraproj`; typed Tauri commands; minimal
lifecycle UI; advisory cloud-folder warning. 74 Rust + 15 frontend tests. Still no entity
tables. See [adr/0008](adr/0008-installation-identity-and-project-lifecycle.md).

**Phase 1C — repository foundation and static Structure ✅ complete.** Driver types
removed from the public `DbError` API; the repository write contract (`Stamp::new`,
`update`, `delete`) that owns `rev` and `updated_at`; migration `V3` adding school,
campus, building, room_type, room, resource, academic_year and term with deliberate
RESTRICT/SET NULL semantics; repositories for all eight; and a schema-level test that
discovers entity tables and fails if any lacks change-log triggers. 108 Rust tests. See
[adr/0009](adr/0009-typescript-type-generation.md) and
[adr/0010](adr/0010-structure-deletion-semantics.md).

**Phase 1D — the time model ✅ complete.** Migration `V4` adding cycle, cycle_day,
period_structure, period, timeslot and calendar_day; repositories for all six; and the
timeslot materialiser, which preserves the identifier of every logical slot that survives
a change to the cycle or the bell schedule. Nothing in it deletes a timeslot: a
rematerialisation reports orphan candidates and leaves them alone, and removing them is a
separate call taking explicit identifiers. The model makes no Monday–Friday assumption —
`cycle_day.ordinal` is a day's only scheduling identity, and real dates reach it solely
through explicit `calendar_day` rows. 151 Rust tests. See
[adr/0011](adr/0011-stable-timeslot-identity.md), which also records the four schema
questions settled before the migration was written.

Also in 1D, fixing a Phase 1C defect: `parse_id` substituted `Uuid::nil()` for a malformed
stored identifier, so a damaged project read as a valid one in which unrelated rows shared
the all-zero identifier. It now returns a typed `DbError::CorruptIdentifier`.

**Phase 1E — School Setup ✅ complete.** The interface half of the original 1D scope: a
nine-section School Setup workflow, and 68 typed Tauri commands behind it. School details,
campuses and buildings, rooms and room types, resources, academic years and terms, cycles
and cycle days, bell schedules and periods, calendar mapping, and the timetable grid
preview.

> **Amended twice.** The Resources step was first *removed* from School Setup: nothing
> schedules against a resource until constraints arrive, so it was configuration with no
> effect on anything the user could see, sitting among eight steps that each feed the grid.
> Only the screen went — the table, its repository, its commands and its generated types
> were untouched throughout.
>
> It **returns in Stage 0C**, because `V13` changed the answer: ADR 0022 gave resources the
> campus associations buildings and rooms already had, so lending equipment to another
> school's campus is now a decision with a visible consequence, and it is unreachable
> without a screen. Scheduling *against* resources is still Phase 4's.
>
> Stage 0C also added a Schools section; Stage A then split Year levels out of Students and
> announced Lessons; Stage B inserted Departments and Subjects before that announcement, so
> the workflow is **seventeen** sections, in configuration rather than dependency order —
> see [adr/0023](adr/0023-the-interface-scopes-to-one-school.md) for the school scoping, and
> the Stage A and Stage B entries below for the order.

Three things in it are more than CRUD:

- **Type generation is wired up**, pulled forward from Phase 2 because forty types had to
  cross the boundary at once. `ts-rs` generates `packages/domain/src/generated/` from the
  repository records themselves rather than from a parallel DTO layer — see
  [adr/0012](adr/0012-records-are-the-ipc-contract.md) — and `npm run types:check` fails
  the build when the committed output and the Rust disagree.
- **The grid preview implements a preview/confirm/apply contract.** A preview carries a
  fingerprint of the plan it reports; a rebuild sends it back and the repository recomputes
  the plan inside its write transaction, refusing with `ReviewRequired` if it has moved.
  Releasing stranded slots is a separate guarded call that deletes exactly what the
  recomputed plan names, which closes the hole a stale orphan list would otherwise open.
- **Cycle completeness is a domain rule, not a screen's arithmetic.**
  `evara_core::time::CycleCoverage` is the first thing to live in `evara-core`; the
  interface asks it over IPC rather than counting rows itself.

190 Rust tests, 147 frontend tests.

**Closes M1-1, M1-2, M1-15, M1-16.**

---

## Phase 2 — Resources

**People ✅ complete.** Migration `V10` adding `year_level`, `teacher`, `student`,
`student_group` and `student_group_member`, with repositories, commands, generated bindings
and three School Setup sections — Teachers, Students and year levels, Student groups.
Membership is many-to-many and edited per group. Deletion semantics and the two departures
from the documented schema are [adr/0018](adr/0018-people-identity-and-deactivation.md).

**Availability ✅ complete.** Migration `V11`; see
[adr/0018](adr/0018-people-identity-and-deactivation.md) for the `weight` departure.

**Multiple schools per project (Stage 0A) ✅ complete.** Migration `V12`: `school` rebuilt
without its singleton constraint, an optional unique school code, and trigger pairs refusing
every cross-school reference the schema could previously not have produced. School CRUD with
a dependent report and a refusal rather than a cascade; an explicit `school_id` on every
school-owned list and command; a same-school guard on every update; a verified safety copy of
the project taken before the first table rebuild in Evara's life. One minimal school selector
in School Setup, shown only when there is a choice to make. See
[adr/0019](adr/0019-multiple-schools-in-one-project.md) and
[adr/0020](adr/0020-in-place-table-rebuild.md).

**Shared identities and school memberships (Stage 0B) ✅ complete.** Migration `V13`,
additive: `teacher_school` (which schools employ a person — the authority),
`teacher_campus` (which campuses they may be based at, refused outside their member
schools by a trigger pair), `student_school_enrolment` (one row per spell, so a transfer
keeps history rather than rewriting it) and `resource_campus` (a resource shared by campus,
the way a building and a room already were). `V12`'s teacher trigger pair widened to
membership rather than being dropped; a new pair refuses a group member who has never been
enrolled in the group's school, which is a hole `V12` could not close. Every existing row
backfilled from what it already said, with its own `created_at` and no revision consumed.
School-scoped teacher and student listings now read the relationship, not the home column,
so a shared person appears in both schools' lists as **one** record. Repository and command
surface for membership, eligibility, enrolment, transfer and resource sharing; the two
People screens show this school's status and placement rather than the project-level flag.
See [adr/0021](adr/0021-project-level-people-and-school-membership.md) and
[adr/0022](adr/0022-resources-shared-across-campuses.md).

**Multi-school management interface (Stage 0C) ✅ complete.** The interface half of `V12`
and `V13`, and no migration: every command it calls already existed. School Setup is now
thirteen sections, with **Schools** first and **Resources** restored. See
[adr/0023](adr/0023-the-interface-scopes-to-one-school.md).

What it delivers, screen by screen:

- **Schools** — the project-level screen. List, create, rename, set or clear the optional
  `code`, change time zone, locale and working days, select which school the rest of setup
  is about, and delete an empty one. A school with anything in it is *explained*, naming
  each dependent kind from `setup_school_dependents` — and explaining a borrowed person
  separately, because that remedy lives in another screen. A new school is created empty:
  nothing is copied from a school that already exists.
- **School details** — the selected school's record and logo. Creation moved to Schools, so
  there is one way to add a school rather than two; the form is one definition shared with
  Schools (`setup/schoolForm.ts`), which is what keeps the newly exposed `code` from
  drifting between them.
- **Teachers** — the list reads `teacher_school`, so one person employed by two schools
  appears in both as the same record. "Add an existing teacher" searches the whole project
  and writes a membership, so **no screen can produce a second record for one real
  person**. A per-teacher panel edits the schools they belong to and the campuses they may
  be based at, keeping *deactivate*, *remove* and *delete* three distinct actions
  (ADR 0021 §8); the home school's membership offers no removal, with the remedy named on
  the control. A visiting teacher's identity is not editable here — the edit is disabled
  and the panel offers to switch to the school that holds it.
- **Students and year levels** — the list reads `student_school_enrolment`, so the
  admission number, year level, campus and status are *this* school's. "Enrol an existing
  student" writes a spell rather than a student. A per-student panel edits this school's
  placement, records a departure (with an optional date, never an invented one),
  re-enrols, and shows the whole history across every school, read-only. *Stage A split
  this in two: year levels became section 7, and the screen became Students.*
- **Transfer** — its own workflow, with a destination school, that school's own year levels
  and campuses, a confirmation summary, and one call to `transfer_student` naming both
  schools. Never a leave followed by an enrol. Distinguished from concurrent enrolment in
  both directions, each pointing at the other.
- **Resources** — restored, with sharing. A resource may be school-wide or based at a
  campus, and may be made available to any campus in the file, including another school's.
  Deletion is refused while another school shares it, and refused outright to a school that
  only borrows it; both are explained before the action.
- **Buildings and rooms** — the share pickers now offer every campus in the file, each
  labelled with the school that owns it, which makes ADR 0013/0014's cross-school sharing
  reachable for the first time. Ownership and access are shown as different things, the
  home campus cannot be withdrawn through unsharing, and deleting a facility another school
  shares stays blocked and explained.

Stale-state protection is four layers deep and documented in ADR 0023 §2: the shell
remounts a section when the school changes, `CrudPanel` closes its form when its
`reloadKey` changes, every read drops a reply that lands after its effect was torn down,
and the backend refuses a cross-school write regardless. 134 frontend tests added across
seven files; 832 frontend and 517 Rust tests pass.

> Deliberately **not** in Stage 0A, 0B or 0C, and not implied by any of them: a
> project-wide teacher code (ADR 0021 records the blocker and the better alternative),
> `department` and `teacher_qualification` (Stage B), project-level availability — a rule
> still names exactly one school, so a shared teacher's rules are recorded per school — a
> `program` or `division` entity below the school (ADR 0023 §7 settles it as the audit
> recommended), and cross-school scheduling. Detecting a shared teacher double-booked
> across two schools needs dated occurrences rather than cycle positions, and nothing here
> claims it works.
>
> **`rev` is still not checked on any write.** Stage 0C preserves and displays it and never
> sends it as a precondition, so two windows can still overwrite each other. A
> frontend-only guarantee would be worse than none, because it would be believed. This is
> the last item between Phase 2 and done, and the audit places it in Stage B.

**School Setup UX restructuring (Stage A) ✅ complete.** No migration, no new entity, and
no Rust change: the whole stage is the interface. Three things, and the reorder is the one
that matters.

**The workflow is reordered from a dependency order into a configuration order.** People
used to come last, after the grid, on the reasoning that nothing about them depends on it.
That was defensible and produced a workflow nobody follows — a school knows who it teaches
long before it has decided what its rotation looks like, and being asked for a six-day
cycle before being asked for a year level is being asked the abstract question first. The
fifteen sections ran: which institution (Schools, School details), where it is
(Campuses, Rooms, Resources), the shape of a school year (Academic years, Year levels),
what is taught (**Lessons** — Stage C, announced and disabled), who (Teachers, Students,
Student groups), and when (Cycles, Bell schedules, Calendar, Grid preview). Stage B then put
**Departments** and **Subjects** in front of the Lessons entry, making seventeen.

The one genuine dependency this inverts is **availability**, which is set against a cycle's
days and a bell schedule's periods and now appears before either exists. It degrades
correctly rather than silently: the grid says "No cycle yet", names the section, and offers
a button that opens it — and says that nothing is lost by configuring the rotation
afterwards, because everything is available until a rule says otherwise.

**Year levels left Students for their own section.** A room type is an attribute of rooms;
a year level is part of the structure of a school year. It exists whether or not a single
student is entered — ADR 0006's promise is that a school can timetable from group sizes
alone, and such a school still has a Grade 8 — and Stage C's lessons and Stage D's Excel
templates both reference year levels directly. Every Stage 0C student capability moved
across unchanged: enrolment, departure, re-enrolment, history, transfer, concurrent
enrolment, and the visiting-student restrictions. The student form reads the selected
school's levels and, when there are none, says so and offers to open the section — advice
rather than a block, because a year level is optional on an enrolment.

**The four teacher workload fields are off the form and still in the record.** Full-time
equivalent and the per-day, per-cycle and consecutive-period ceilings are no longer asked
for: a school entering a staff list is being asked to tune a solver it has not run. The
columns, the record, `TeacherInput`, the generated types and the command signature are
untouched, and the **stored values survive an edit exactly** — `toDraft` reads every one
into the draft and `toInput` writes every one back, rendered or not. That pass-through is
the stage's one silent-loss risk, so it is asserted against a teacher with four non-default
values rather than hoped for; FTE stays visible as a table column, and a note says once
that the values are kept and are never changed by editing a teacher. Phase 4's load
constraints will read exactly what is stored today, on a screen that can show what a
ceiling does.

Section identifiers are the only thing anything addresses a section by — `openSection`, the
empty states that navigate, the tests — so the reorder changed no behaviour anywhere it was
not meant to. 58 frontend tests added across three files; 920 frontend and 517 Rust tests
pass.

> **Not in Stage A, and not implied by it:** Subjects, Departments, teacher qualifications
> and `rev` checking (**Stage B**); Lessons and activity modelling (**Stage C**, which is
> what the disabled navigation entry announces); Excel roster imports (**Stage D**). Nothing
> in this stage creates a lessons table, form, placeholder row or scheduling model, and the
> navigation entry has no `render` at all so it cannot be opened.

**Curriculum prerequisites and revision checking (Stage B) ✅ complete.** Migration `V14`,
additive with no backfill: `department` and `subject` (school-owned), `teacher_qualification`
(project-level) and `teacher_department` (a narrowing of membership). And the item this phase
has carried since Phase 1E — **every write now states the revision it was built on.**

What it delivers, and the two decisions in it that are not obvious:

- **A qualification is a property of the person**, naming one school-owned subject. A
  teacher shared between two schools who teaches Mathematics at both holds *two*
  qualifications, one per subject record, and remains *one* teacher. It carries no
  `school_id` — the subject supplies it — requires no school membership, and **outlives
  one**: deactivating a teacher, withdrawing a membership, archiving them, changing their
  campuses and changing their department all leave it exactly where it was. Only an explicit
  removal, or deleting the person, takes it. See
  [adr/0025](adr/0025-teacher-qualifications-across-school-owned-subjects.md), which also
  settles **[OPEN 6]** of the audit and records one deliberate departure from ADR 0010.
- **`teacher.department_id` is not added, now or later.** `V10` reserved it and `V13` made it
  wrong: one column on a project-level identity cannot say "Mathematics at Northgate, Science
  at Southmoor". The audit's own `teacher_school.department_id` fixes the scope and not the
  cardinality — Mathematics *and* Physics at one school is just as ordinary. So the
  association is its own table, modelled on `teacher_campus` down to the trigger names.
  [adr/0026](adr/0026-teacher-departments-are-a-narrowing-of-membership.md).

**`rev` is now a precondition, not only a record.** The revision goes in the statement's
`WHERE` clause, so the check and the write are one atomic operation rather than a read
followed by a hopeful write:

```sql
UPDATE room SET ..., updated_at = ?, rev = rev + 1 WHERE id = ? AND rev = ?
```

A stale save matches nothing, so nothing is written: no column changes, no revision is
consumed, and no change-log entry is appended. It is reported as `DbError::Conflict`, which
the interface distinguishes from `NotFound` because the responses differ completely — one
means reload, the other means it has gone.

**44 of the 84 mutation commands take `expectedRev`**, which is every category where a lost
update is possible: 20 single-record updates, 19 deletes, the per-school employment flag, a
spell's placement and departure, and the atomic transfer. The other 40 are classified rather
than omitted — 20 creations (no previous version exists), 14 relationships (existence, not
revision), 2 plan-guarded grid calls (a `PlanFingerprint`, which is stronger), and 4
documented last-write-wins exceptions. The full inventory, including the one residual gap
(a school logo), is [adr/0027](adr/0027-revisions-are-checked-on-write.md) §3 and §4.

**What the interface does with a refusal** is the half a backend guarantee is worthless
without, and `CrudPanel` does none of the four tempting wrong things: it never retries with a
fresh revision, never discards what the user typed, never reports success, and offers a
reload rather than performing one. ADR 0027 §6, asserted in `RevisionConflicts.test.tsx` as
four things *not* happening.

**Two new School Setup sections, at 8 and 9** — before the Lessons entry that announces
Stage C rather than after it, because that is the dependency: a lesson is taught *of* a
subject. The workflow is **seventeen** sections.

- **Departments** — this school's own organisational units, never shared, so two schools may
  each hold a Mathematics Department. A delete is refused while subjects or staff still refer
  to it, and *explained with counts read before the button is offered* rather than failing
  when it is pressed.
- **Subjects** — what the school teaches. Two schools' Mathematics are two records that may
  share a name *and* a code; nothing merges them. The department picker offers this school's
  departments and no others, and a school with none is told so rather than blocked, because
  the field is optional. A read-only panel lists who is qualified.
- **Teachers** gains a *Curriculum* panel keeping the two kinds of fact visibly apart. Both
  lists are project-wide and grouped by owning school; another school's rows are shown,
  labelled and not editable, which is ADR 0023 §5's rule applied once more.

The existing frontend suite came out of this stronger rather than merely adjusted: every
assertion that checked a payload now also pins that the screen sent the row's own revision,
and fixtures moved off `rev: 1` wherever an assertion reads one, so a screen sending a
default cannot pass by coincidence. 62 Rust tests added across two new suites and 58 frontend
tests across three; **572 Rust and 980 frontend tests pass.**

> Deliberately **not** in Stage B, and not implied by it: qualification levels or
> certification expiry (ADR 0025's *Known limitation*), a head of department or a
> primary-department flag (ADR 0026's), a department hierarchy, a project-wide subject
> catalogue (rejected in ADR 0025 §2), `subject.colour` and `subject.default_room_type_id`
> (Phase 7 and Stage C respectively), **Lessons** and activity modelling (**Stage C**, which
> is what the disabled navigation entry still announces), and Excel roster imports
> (**Stage D**). Nothing here creates a lessons, course or class-section table.
>
> Nor is any kind of **merge** claimed. A conflict is reported and the user decides; a merge
> Evara performed itself would be a guess about intent, which is the one thing a timetable
> cannot afford.

Remaining in this phase: **nothing.** Both open items are closed.

**Closes M1-3, M1-4, M1-5, M1-6.**

---

## Phase 3 — Curriculum and activities

### What Stage B leaves Stage C to build on

`V14` deliberately stopped at the prerequisites, so the contract Stage C inherits is worth
stating once rather than rediscovering:

- **A lesson is taught *of* a subject**, by identifier. `subject.id` is UUIDv7 and never
  renumbered, so a school that entered its curriculum in Stage B finds it still means the
  same thing. `course.subject_id` is `RESTRICT`, like every ownership edge, and
  `Curriculum::subject_dependents` is the place to add the new count so a refused subject
  delete goes on explaining itself.
- **A subject is school-owned, and `course` must be too.** Two schools' "Mathematics" are
  two records; a course referencing one is a course of that school. Resolve the school
  through `subject.school_id` rather than adding a second `school_id` to `course` — the same
  rule `V14`'s own tables follow.
- **Assigning a teacher to a lesson requires the conjunction**, not the qualification alone:
  a `teacher_qualification` for the lesson's subject, an **active** `teacher_school`
  membership at the lesson's school, and `teacher_campus` eligibility where the lesson's
  campus is constrained. ADR 0025 §4 settles this and explains why a qualification on its own
  is never authority to timetable somebody: it outlives employment on purpose.
- **Offer qualified teachers, do not require them.** `Curriculum::subject_teachers` is the
  read a lesson's teacher picker wants; it is project-wide in the people it returns, so a
  visiting teacher qualified for this school's Chemistry appears. Whether an *unqualified*
  assignment is refused or merely warned about is Stage C's decision and should be an ADR,
  because a school mid-term with one absent specialist will want the warning.
- **A department is not a scheduling concept yet.** Phase 4's load constraints are its first
  reader. A lesson does not belong to a department; its subject may.
- **Every new update and delete takes `expectedRev`.** ADR 0027 §3's inventory is the shape
  to extend, not a one-off: a Stage C command that omits it would be the only unguarded
  single-record write in the project.

### The work

- Migration `0003`: Curriculum group, then Activities group (activity, activity_teacher,
  activity_group, activity_room_candidate, activity_resource, line, line_member,
  activity_link).
- Activity generator: expand a course's `periods_per_cycle` into activities with
  `split_index`, respecting `default_duration_periods`.
- Screens: Courses → Class sections → Lessons, with teacher and group assignment,
  duration, room requirements, and lock toggles. Lines editor (basic).
- `fixtures/`: a synthetic school generator in `evara-cli` plus 3 checked-in fixtures —
  tiny (feasible by hand), medium (~400 activities), and one deliberately infeasible.

**Closes M1-7.**

---

## Phase 4 — Constraints as data

- Migration `0004`: `constraint_instance`, `constraint_ref`.
- `evara-constraints`: the registry skeleton, `ConstraintEvaluator` trait, JSON Schema per
  type, param validation on write, and the default constraint set seeded into new projects.
- Screens: constraint list grouped by family; per-instance level, weight and enable
  toggle; scope picker writing `constraint_ref`; forms generated from the param schema.

**Closes M1-8** (data and configuration; evaluation is Phase 5).

---

## Phase 5 — Scorer, validator, diagnostics

- Implement the `M1` evaluators from [CONSTRAINTS.md](CONSTRAINTS.md) §3 — roughly 22
  types. One module and one test file each.
- `ReferenceScorer` (full evaluation) and `IncrementalScorer` (indexed), plus the
  property test asserting they agree after random move sequences. This test gates every
  later constraint.
- `Explanation` plumbing and the `validate_move` command.
- Pre-solve feasibility diagnostics ([SOLVER.md](SOLVER.md) §8 Layer 1).

**Done when:** the infeasible fixture produces specific, actionable diagnostics with no
solver involved.

---

## Phase 6 — Solver v1

- Problem loader: database → dense-index `Problem`. Solution writer: `Assignment` →
  `timetable_entry`, transactionally.
- Construction heuristic (most-constrained-first, locks placed first).
- Late Acceptance Hill Climbing with the move set from [SOLVER.md](SOLVER.md) §5, seeded
  RNG, termination config, cancel token.
- Background thread, `solver://progress` events, `solve_run` persistence.
- `evara-cli solve` + `bench`, with score baselines per fixture checked into CI so solver
  regressions are caught.
- UI: Generate screen with settings, live progress, score, cancel, and the hard-violation
  report on infeasibility.

**Closes M1-9.**

---

## Phase 7 — Timetable views

- `evara-report` projections: master, by teacher, by class/group, by room, by student.
- Grid component driven by cycle days × periods, not weekdays. Multi-period activities
  render as spans.
- Score health panel: hard / medium / soft totals with a per-constraint breakdown,
  clicking through to the offending entries.

**Closes M1-10, M1-11, M1-12.**

---

## Phase 8 — Manual editing

- Drag-and-drop in the grid, validating on hover: legal targets tinted, illegal targets
  showing the blocking reason.
- `apply_move` command writing through a transaction; undo/redo stack.
- Violation inspector listing every current violation with entity links.
- Lock/unlock an entry. Repair run: scope filter + `minimise-disruption`, offering a diff
  of what would change before committing.

**Closes M1-13, M1-14.**

---

## Phase 9 — Project transfer

- `evara-project`: writer, reader, manifest, checksums, format migration chain scaffold
  (currently `formatVersion: 1` only).
- Commands `project_export` / `project_import`, native file dialogs, progress for large
  schools.
- The full test set from [PROJECT-FORMAT.md](PROJECT-FORMAT.md) §7, round-trip included,
  in CI.

**Closes M1-17, M1-18.**

---

## Phase 10 — Output

- Print-oriented HTML + a print stylesheet per view; print and "Save as PDF" through the
  webview's print pipeline. A Rust vector-PDF renderer is a later refinement, not an M1
  requirement — see [OPEN-DECISIONS.md](OPEN-DECISIONS.md) D10.
- CSV export per view.
- A renderer trait so additional formats drop in without touching the projections.

**Closes M1-19.**

---

## Phase 11 — Packaging

- MSI/NSIS for Windows, DMG for macOS (universal binary), auto-updater **off**.
- Icons, bundle metadata, third-party licence manifest generated from `cargo-about` and
  `license-checker`.
- Reproducible release build in CI, artefacts attached to a tag.
- Smoke test: install from the artefact, create a project, generate, export, import.

**Closes the milestone.**

---

## The stage sequence after the multi-school foundation

Phase 2's remaining work is sequenced as stages, which is how the multi-school audit
(`docs/multi-school-architecture-audit.md` §15) planned it and how it has been delivered:

| Stage | Scope | Status |
| --- | --- | --- |
| 0A | Multiple schools per project (`V12`) | ✅ complete |
| 0B | Shared identities and school memberships (`V13`) | ✅ complete |
| 0C | Multi-school management interface | ✅ complete |
| **A** | **School Setup UX restructuring** | ✅ **complete** |
| **B** | **Subjects, departments, teacher qualifications, `rev` checking** | ✅ **complete** |
| C | Lessons and activity modelling | next |
| D | Excel roster imports | after C |

Lessons already appears in the School Setup navigation, disabled and labelled, so the order
reads correctly before Stage C builds it. Nothing else about it exists — no table, no form,
no placeholder row, and no `render`.

## After Milestone 1

Rough order, not a commitment: individual student sectioning and course requests
(`request-*`, `section-*`); the remaining relationship and multi-campus constraints;
relaxation-based infeasibility suggestions ([SOLVER.md](SOLVER.md) §8 Layer 3); the
daily-operations layer (absence, substitutions, room changes, merges, yard duties);
CSV import; then vendor adapters in `evara-integrations`, one at a time, each behind a
feature flag and its own fixtures.
