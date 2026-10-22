# Database-to-solver input assembly

How one school's stored rows become the scheduling `Problem` the solver will search over,
and what the loader reports on the way.

Implemented in **M4**. There is still no solver, no constraint registry, no
`timetable_entry` and no new migration — `V15` is unchanged.

```text
SQLite ──► Problem ──► integrity ──► validation ──► constraints ──► solver ──► solution
  M4         M2            M2            M3             M5           M6         M7
```

---

## 1. The public API

```rust
use evara_db::{Database, SchedulingSelection};

let db = Database::open_for_inspection(path)?;        // read-only; see §15
let report = db.scheduling().validate(&SchedulingSelection {
    school_id,
    cycle_id,
    term_id,                                          // Option<Uuid>
})?;
if !report.may_solve() { /* surface report.findings */ }
```

| Item | What it is |
| --- | --- |
| `Database::scheduling()` → `Scheduling<'_>` | The loader handle. **Takes `&self`** — see §15 |
| `Workspace::scheduling()` | The same, for the open project |
| `Scheduling::load(&SchedulingSelection)` → `Assembly` | Compiles the problem |
| `Scheduling::validate(&SchedulingSelection)` → `AssemblyReport` | Compiles it, checks integrity, runs M3, merges |
| `AssemblyReport::of(&Assembly)` | The same combination, when the caller wants the problem too |
| `evara_db::scheduling::LOADER_DIAGNOSTICS` | The three codes the loader owns (ADR 0037 §1) |

`Assembly` carries the `Problem`, the `ProblemIndex` (§5), the availability rules that were
compiled, the loader's own findings, a `Completeness`, and `AssemblyStats`.

There is deliberately **no** `load_or_default`, no `load_the_school`, and no overload that
guesses a cycle. All three of school, cycle and term are named by the caller — ADR 0032 rule
1 scopes a solve to one of each, a project has held several schools since `V12`, and a solve
over a grid the user did not choose is a timetable built on a guess.

## 2. Crate and module ownership

`crates/evara-db/src/scheduling/`, because `ARCHITECTURE.md` §3 rule 4 says so: `evara-db` is
the only crate that knows SQL, and *"the translation from UUIDs to the solver's dense indexes
happens here, once, and no other crate acquires a database handle to avoid it."*

| Module | Owns |
| --- | --- |
| `mod.rs` | The API, the selection, the assembly order, and the `overlap` decision (§9) |
| `index.rs` | UUID ↔ dense index, and `ProblemIndex` |
| `context.rs` | School, cycle, term, campuses, cycle days, bell schedules, periods, timeslots |
| `facilities.rs` | Room types and rooms |
| `population.rs` | Teachers and student groups |
| `lessons.rs` | Class sections and activities |
| `rules.rs` | Availability compilation |
| `findings.rs` | The three loader-owned diagnostics |
| `report.rs` | The combined report and the merge rule |
| `tests/` | The database-backed fixtures and the round-trip suite |

Nothing here depends on React, Tauri, a network, cloud storage, a solver or a constraint
evaluator. The scheduling domain in `evara-core` stays free of SQLite, and no driver type
crosses `evara-db`'s public API.

## 3. Database to domain

| Stored | Compiled to | Notes |
| --- | --- | --- |
| `school` | `SchoolFact` | `timezone` is carried and read by nothing — one school, one zone, and it cancels on both sides of every comparison (ADR 0032 §5) |
| `cycle` | `CycleFact` | `kind` maps one to one |
| `term` | `Option<TermFact>` | `None` is an ordinary selection, not a missing value |
| `campus` | `CampusFact` | |
| `cycle_day` | `CycleDayFact` | `ordinal` is the day's only scheduling identity. **`weekday_hint` is not read** |
| `period_structure` | `BellScheduleFact` | Every schedule the school has **defined** — not `G`, see §6 |
| `period` | `PeriodFact` | `starts_at`/`ends_at` become a comparable `TimeInterval` |
| `timeslot` | `TimeslotFact` | Interval taken from its period; `is_teaching` read as stored |
| `room_type` | `RoomTypeFact` | The school's own, plus any an in-scope room is classified under (§10) |
| `room` + `room_campus` | `RoomFact` | One physical room, one identity (ADR 0014) |
| `teacher` + `teacher_school` + `teacher_campus` | `TeacherFact` | §8 |
| `student_group` (+ `student_group_member`, `student`, `student_school_enrolment`) | `GroupFact` | §9 |
| `class_section` + `course` + `subject` | `SectionFact` | §7 |
| `activity` | `ActivityFact` | One row, one occurrence |
| `availability` | `UnavailabilityIndex` + `Vec<AvailabilityRule>` | §11 |

Not read at all, and each for a stated reason: `building` (a room's campus set is complete in
`room_campus`), `resource` / `resource_campus` (`resource-clash` is not in the first
constraint set), `calendar_day` (no clash decision reads a date — ADR 0032 §5),
`class_enrolment` beyond a count (no scheduling meaning in Milestone 1 — ADR 0029 §4),
`teacher_qualification` and `teacher_department` (a qualification warns at assignment time and
is not a scheduling input — ADR 0028), `school_logo`, `year_level`, `department`.

`Problem::pinned` is empty: `activity.is_locked` does not exist, and `activity-fixed` joins
the constraint set once locks do.

## 4. School scoping

Everything school-owned is read with an explicit `school_id`. Nothing takes a `LIMIT 1` read
of `school`, which would hand the caller an arbitrary institution and silently scope the
report to it.

- **The selection is checked against itself first.** A cycle belonging to another school, or a
  term belonging to another school's academic year, is `DbError::WrongSchool` before any other
  row is read — through the same `require_school_owner` every write uses.
- **The lesson chain has no `school_id`.** `subject → course → class_section → activity`, so
  every read joins up to `subject`. That is the chain ADR 0029 establishes and the one
  `require_school_owner` already walks.
- **Term scope.** A section is in scope when its course's `term_id` is `NULL` (all year) or is
  the selected term. **When no term is selected, every course is in scope** — there is no basis
  on which to exclude one, and excluding term-specific courses would *under-count demand*,
  which is the direction `PRE-SOLVE-VALIDATION.md` §11 forbids for a sound counting check.
- **Two schools never meet in one `Problem`.** `SchedulingContext` holds one school by
  construction, so the solver cannot accidentally equate two schools' cycle positions.
- **Cross-school sharing is retained and reported, never guessed.** A teacher with a second
  active `teacher_school` row, and a room whose `room_campus` set reaches another school's
  campus, carry an `also_at` list so ADR 0032 rule 3's warning can name the other school. **No
  row of the other school is read** beyond its name, and no cross-school clash is computed —
  that needs a time alignment *and* a timezone database, and ADR 0032 §4 defers both.

Filtering a query to one school does **not** make a resource exclusive to it, and nothing here
claims that school-scoped validation implies cross-school conflict freedom.

## 5. UUID to dense index

`SOLVER.md` §2: a `Problem` is addressed by dense `u32` indices, and the translation happens
once, here.

- **One index per identity.** `Interner::intern` returns the index an identifier already has,
  so a teacher who is both employed here and named by a section is one `TeacherIx`.
- **Offer order is index order**, and every query carries an explicit `ORDER BY` over columns a
  unique index makes total. Nothing reads `rowid`.
- **The order is the interface's order**, so a section's index is its position in the list the
  user is looking at — the stable secondary key `PRE-SOLVE-VALIDATION.md` §4 asks diagnostics
  to be sorted by.

| Table | Ordered by |
| --- | --- |
| Campuses, room types, rooms, groups, bell schedules | `name, id` |
| Cycle days | `ordinal` |
| Periods | schedule position, then `ordinal` |
| Timeslots | `cycle_day.ordinal`, schedule position, `period.ordinal` |
| Teachers | `family_name, given_name, id`, active memberships first |
| Sections | `subject.name, course.name, class_section.name` (+ `id` tie-breaks) |
| Activities | section order, then `split_index` |

**Reverse mappings** are in `ProblemIndex`: `activity_id(ActivityIx) -> Option<Uuid>` and
`activity_index(Uuid) -> Option<ActivityIx>`, and the same pair for all eleven tables. A
diagnostic can name the row a user clicks on, and M7's solution writer can write a
`timetable_entry` against the identifiers the facts came from.

No display name is ever an identity key, and no new `Uuid` is minted for an existing entity.
`ProblemIndex::counts()` reports each table's size, and the loader's own test asserts the maps
cover exactly the tables they describe — a map one row short would make the last row's
identifier unrecoverable, which nothing else would catch.

## 6. Bell-schedule resolution

**The M3 resolver is reused. There is no second compatibility algorithm.**

```rust
let grid = MaterialisedGrid::of(&problem);            // G, derived from the slots
section.bell_schedule = grid.resolve(&context, section.campus);
```

ADR 0036 §2 requires it and `Problem::integrity` reports any drift as
`IntegrityError::BellScheduleResolutionDisagrees`. The loader *produces* the value M3
*validates*, with one implementation between them, and M5's `duration-contiguous` and M11's
`validate_move` will read the same module.

Sections are therefore built **last**: the context and the slots exist, `G` is derived from
`Problem::timeslots`, and only then is each section's resolution computed and recorded. That
is the shape `PRE-SOLVE-VALIDATION.md` §9 describes.

**`G` comes from the slots, never from the defined list.** `SchedulingContext::bell_schedules`
holds every `period_structure` the school has defined, because a schedule's campus and name
have to be readable even for one that was never materialised. A schedule defined and not
materialised has no slots, so it is not a candidate and causes no ambiguity — ADR 0033 test 8,
and the defect M3 found in M2's documentation of that field.

Covered by database-backed fixtures: one valid materialised schedule; a defined-but-unmaterialised
one beside it; an empty grid; two compatible schedules; two campuses with a campus-scoped
schedule each; a school-wide schedule; a section with no campus; and schedules whose physical
period intervals differ. Two compatible schedules produce `Ambiguous` carrying both candidates
— the loader never chooses.

## 7. Activities and lessons

**One `activity` row is one placeable occurrence** (ADR 0029). The loader does not read
`course.periods_per_cycle` and multiply, does not split a `duration_periods = 2` row into two
activities, and does not merge occurrences. Four single periods a cycle is four
`ActivityFact`s with `split_index` 1–4, never renumbered.

`SectionFact` holds what every occurrence of a section shares — staffing, audience, room
requirement, resolved bell schedule — once rather than four times, which is the argument
ADR 0029 §5 used to reject `activity_teacher` and `activity_group`.

`declared_periods_per_cycle` is carried as a declaration. It is allowed to disagree with the
sum of the section's activity durations, and M3 reports the difference as advice.

An activity cannot be silently dropped: it is read by joining through its own section, so a
section in scope implies its activities are, and a reference that does not resolve is
`DbError::CorruptIdentifier` rather than a quietly reduced problem.

## 8. Teachers

Project-wide identity (ADR 0021): somebody employed by two schools is **one** `teacher.id` in
both problems, which is what makes the shared-teacher warning writable.

**The set in scope is the active memberships *plus* every teacher an in-scope section names.**
That is wider than `TeacherFact`'s own documentation, and the reason is a case the obvious
answer gets wrong. `V15`'s `class_teacher_requires_active_membership` trigger pair fires on
`class_teacher` and **never on a change to `teacher_school`** — deliberately, because ADR 0018
requires a deactivated teacher to keep every assignment they already had and merely stop being
offered new ones. `set_teacher_membership_active(…, false)` therefore succeeds while they still
teach. Loading only the employed set would leave `SectionFact::teachers` dangling, which
`Problem::integrity` would report as a loader bug — correctly. `AssemblyStats::teachers_without_active_membership`
counts them.

`eligible_campuses` is `None` where nothing narrows the teacher, which is the ordinary state
and is **not** the same as an empty list. Reading an empty narrowing as "eligible nowhere"
would make every teacher at a multi-campus school unassignable, which `V15`'s own trigger
comment warns against.

Eligibility is not enforced here — ADR 0028 makes it a trigger pair's job and a solver-side
duplicate would be a second implementation of a rule the database owns. It is *asserted* over
facts already loaded, with no query of its own, and a violation is reported as corruption:
`teacher-not-eligible-for-campus`, blocking. See §12.

## 9. Student groups

Groups are the scheduling population (ADR 0035 §5) — never an expanded set of students, which
ADR 0006 requires because a school with no `student` rows at all must still get a timetable.
Individual `class_enrolment` rows are an overlay with no scheduling meaning: they contribute
nothing to the audience and nothing to attendance, and are carried only as a count so M3 can
say *"this section names 20 students and no attending group"*.

Every group of the school is loaded, not only the ones a section is taught to: a parent that
attends nothing is still the group whose future `children_are_disjoint` answer rule 6 will
read. `V12`'s trigger pair holds a parent to its child's school, so the parent set is closed
and no parent edge can dangle.

`effective_members` is ADR 0035 §2's count: `student_group_member` rows over the group's whole
**subtree**, restricted to students who are `is_active` **and** hold a
`student_school_enrolment` spell at this school whose status is not `LEFT`. One recursive query
for the school, bounded by the same `MAX_GROUP_DEPTH` the repository's parent walk uses. Both
halves of the filter are load-bearing: the subtree union because containment is what the parent
edge means, and the `LEFT` filter because `student_group_member` keeps history and counting a
child who has left would manufacture clashes between groups that no longer share anybody.

### `children_are_disjoint`

**The column is not added in M4.** ADR 0035 §4 defines it and
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1 schedules it for M5's migration. Every group
therefore reads `DisjointnessDeclaration::from_column(None)` — `NotStated`, which is exactly
what every existing project holds. Defaulting to either answer would invent a statement the
school has not made, and `NotStated` and `MayOverlap` are deliberately different states.

### The overlap relation

**Every pair is `Indeterminate`, and M4 compiles no answers at all.** ADR 0035 §3's seven
ordered rules are M5's; a second implementation here is the duplication `CLAUDE.md` §2 forbids,
with the sharpest consequence available — rule 3 before rule 6 is what makes a member row beat
a declaration, and two evaluators that disagreed would have the product honouring a clash in
one place and not the other. M4 assembles the *inputs* §3 reads and leaves the function to M5.

What that does **not** mean: an undecided pair is not treated as disjoint. `clash_mask` admits
only `Overlaps`, so each section's clash mask is exactly its own audience — which protects
every group from being double-booked against itself and claims nothing about any other pair.
ADR 0038 §4 states that this is the correct compiled behaviour, and M3's
`group-overlap-indeterminate` is where the gap is announced.

**The diagonal is left unstated**, matching M2's fixtures and `PRE-SOLVE-VALIDATION.md` §14's
known limitation 9. ADR 0035 rule 2 makes a group against itself `Overlaps`; setting it here
would be implementing one of the seven rules in the one place that must not, and would make a
database-backed problem differ from the synthetic fixture it mirrors for no gain — `clash_mask`
already contains the audience, and `indeterminate_pairs` counts `a < b` only, so nothing reads
the diagonal. **M5 should declare it along with the other six.**

The visible consequence, stated so it is not mistaken for a defect: a school with two or more
groups always carries exactly one `group-overlap-indeterminate` warning. The database-backed
`tiny` fixture is therefore not findings-free the way M2's in-memory `tiny` is.

## 10. Rooms

**One physical room, one identity** (ADR 0014), however many campuses or schools reach it.
Rooms in scope are read through `room_campus`, which `V6`'s header establishes as the complete
set of campuses a room serves, the home campus included; nothing reads `room.campus_id`, which
since `V6` means only "the campus it was created under".

`room_candidates` is computed by `RoomFact::is_candidate_for`, which is the **one**
implementation of the candidate domain — bookable, reachable from the section's campus, of the
required type. The loader calls it and does not restate the three conditions, and neither will
M11's move validator. That is the guarantee `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.1 asks
for.

`course.required_room_type_id` is the **only** room requirement that exists. No room-candidate
list and no resource requirement is invented.

**`Problem::room_types` is not only the school's own.** `create_room` does not require a room's
type to belong to its campus's school, and a shared room's type belongs to whichever school
created it. Dropping such a classification is wrong in the dangerous direction — an
unclassified room is a candidate for *every* requirement, so a shared laboratory would silently
satisfy "needs a gymnasium" — so the type is loaded alongside the school's own. A shared
laboratory classified under Northgate's "Science Laboratory" therefore does not match
Southmoor's: two different records, and Southmoor has not said the shared room is one of its
laboratories.

**Capacity is loaded and enforced by nothing.** `room-capacity` is not in the first constraint
set (ADR 0034 §1). `room.capacity` is `NOT NULL`, so there is no unknown capacity to
represent — `0` means zero seats and is never reinterpreted as "not entered"; M3 warns about a
bookable room recorded as holding 0 rather than guessing. The uncertainty is entirely on the
attendance side, which is why `ExpectedAttendance` has an `Unknown` variant and capacity is a
plain `i64`.

`class_section.expected_attendance` is also a future column (ADR 0034 §2), so clause 1 of §3's
derivation reads nothing yet. What M4 can derive is clause 3, in the only form `V15` reaches: a
section taught to exactly **one** group that declares a size is `Known(size)`, because
"pairwise disjoint" over a one-element audience is vacuously true — and that happens to be the
commonest configuration in the product. A section taught to two sized groups is `Unknown` and
becomes `Known(Σ)` once M5 can prove the pair disjoint. `Unknown` is never `0`.

`attendance_floor` is the maximum, across the section's groups, of the effective membership of
a group with no declared size — ADR 0034 §3's sound lower bound. The maximum and not the sum,
because the groups may share students. It may raise a warning and may never decide a hard
constraint.

## 11. Availability

Read from the `V15` `availability` table, all six `entity_kind` values.

**A rule compiles onto the slots it names and is never widened** (ADR 0033 §7):

| Stored as | Names |
| --- | --- |
| `cycle_day_id` alone | every slot of the grid on that day, on **every** schedule |
| `period_id`, alone or with a day | the slots of that one `period` row — so **one** schedule |
| `timeslot_id` | one exact slot, and therefore one schedule and one term |

So a teacher unavailable "period 1, every day" at a two-schedule school has said something
about one schedule. The resulting limitation is named rather than solved: M3's
`teacher-under-several-bell-schedules` is what makes the omission visible.

**All three statuses are compiled.** `UNAVAILABLE` is hard; `PREFERRED` and `DISCOURAGED` are
soft and unenforced in the MVP, and are still written into the index, because discarding them
would make "the school said something here" unrecoverable. A discouraged slot is legal and
merely worse. Missing availability means **available** — `V11` stores only exceptions, and
`SlotAvailability::Unconstrained` is a distinct fourth state.

**A rule naming a term this grid is not for** restricts nothing in it, and is counted as out of
scope. Every slot of the grid carries the selected term, so the comparison is against the
selection rather than against each slot.

### Dates

`effective_from` / `effective_to` are carried onto each `AvailabilityRule` verbatim and are
**not** consulted when deciding which slots a rule covers. That is the accepted contract rather
than a shortcut: a cycle carries no dates (ADR 0017), the timetable is defined over the cycle,
ADR 0032 §5 states that an incomplete or absent `calendar_day` mapping has no effect on any
rule the accepted decisions define, and `EffectiveWindow`'s own documentation says the window
*"is carried so the loader does not have to discard a stored fact"*.

**The limitation, stated:** a date-bounded restriction is applied for the whole cycle. For an
`UNAVAILABLE` rule that is the conservative direction — it removes options rather than
inventing them — and a milestone that projects the cycle onto dates will find the window
already modelled. No new date-mapping model is invented here.

### Three outcomes for a rule that cannot be compiled

ADR 0037 §3 requires that such a row is **not silently discarded** and that the finding names
the kind.

1. **The kind is not scheduled** (`RESOURCE`, `SUBJECT`) → `availability-entity-kind-not-scheduled`,
   informational, carrying the rule's identifier and the kind, and saying plainly that the rule
   is recorded and not being enforced. The loader does not then ask whether the entity still
   exists: one row, one finding, and "this is not enforced" is the actionable fact.
2. **The entity is gone** → `orphan-availability-reference`, a warning, carrying the rule's
   identifier, the kind, the referenced identifier and the school. `availability.entity_id` is
   polymorphic and carries no foreign key, which `V11`'s header calls the one reference the
   database cannot enforce.
3. **The entity is real and out of scope** — a teacher whose membership was deactivated and who
   teaches nothing in scope, a cycle day of another cycle, a period of an unmaterialised
   schedule, a rule for a different term. **Counted, not reported.** Such a rule restricts
   nothing this solve can place, so there is no violation it could prevent and no remedy to
   offer; and reporting it as an orphan would state that the entity no longer exists, which is
   false and would send somebody looking for a deletion that never happened.

A rule is recorded in `Assembly::rules` only once it has actually been compiled, because
`AvailabilityEntity` is a dense index by construction and an unresolved rule has no honest
value to carry.

## 12. Loader diagnostics

Three codes, exactly those ADR 0037 §1 assigns, in the **same** vocabulary M3 uses —
`Diagnostic`, `DiagnosticCode`, `Severity`, `EntityRef`. There is no parallel diagnostic type,
and the two producers' code sets are disjoint, which a test asserts rather than trusts.

| Code | Severity | Subject | Trigger |
| --- | --- | --- | --- |
| `teacher-not-eligible-for-campus` | Blocking | Teacher | A teacher is given a section at a campus their stated eligibility excludes |
| `orphan-availability-reference` | Warning | Availability rule | The rule's entity is no longer in the project |
| `availability-entity-kind-not-scheduled` | Info | Availability rule | The rule is about a kind nothing schedules |

### `teacher-not-eligible-for-campus` is reachable, and M2's documentation of it is wrong

M2 documented the code as *"unreachable — trigger pairs prevent it"*, and ADR 0037 §4 repeats
that a consistent database cannot contain one. **Neither is true at `V15`.**
`class_teacher_respects_campus_eligibility` fires on an INSERT or UPDATE of `class_teacher` and
never on a change to `teacher_campus`, and neither `Membership::assign_teacher_campus` nor
`Membership::remove_teacher_campus` consults `class_teacher`. Two ordinary supported calls reach
it:

1. Give a section to an **unnarrowed** teacher — permitted, because no `teacher_campus` row
   means "not narrowed" — then narrow them to a different campus. The first
   `assign_teacher_campus` turns an empty eligibility set into `{B}`, and the existing
   assignment at campus A is now excluded.
2. Remove the one eligibility row an existing assignment relies on.

`Lessons::update_class_section` closes the third route — moving a section to a campus its staff
are not eligible at is refused, and its comment says why — so the gap is on the `teacher_campus`
side only. The M4 fixture reaches it with **no bypassed trigger**, and the finding is
load-bearing rather than theoretical. **Recommended follow-up, in its own PR:** have
`assign_teacher_campus` and `remove_teacher_campus` refuse a narrowing that would strand an
existing `class_teacher` row, as `update_class_section` already does from the other side. M4
reports the state; it does not change a mutation path.

## 13. Error categories

Three channels, kept apart. A fatal failure is never dressed as a scheduling diagnostic, and a
scheduling input problem never crashes the loader.

| Category | Channel | Examples |
| --- | --- | --- |
| **A — the request cannot be served** | `Err(DbError)` | `NotFound` (no such school, cycle or term); `WrongSchool` (a cycle or term of another school); `Open`, `NotAnEvaraProject`, `SchemaTooNew`, `SchemaUpgradeRequired`; any SQLite failure as `Internal` |
| **B — a stored row is wrong** | a loader `Diagnostic` | The three codes of §12 |
| **C — the school's inputs are incomplete** | M3's `validate` | No grid; an ambiguous bell schedule; an occurrence longer than any run; a required room type nothing satisfies; the counting arguments |

A fourth, narrower channel exists for the loader's **own** defects: `Problem::integrity`
returns `IntegrityError`s, which ADR 0036 §4 keeps out of the user-facing report because the
remedy is a code fix and not a field. They still refuse the solve — see §14.

**Nothing panics on malformed stored data.** A stored identifier, enumeration value, ordinal or
bell time that cannot be read is `DbError::CorruptIdentifier`, following `parse_id_text`'s rule
and `WorkingDays`' precedent: a persisted value that cannot be true is corruption rather than
input, and an invented default makes a damaged file look like a working one.

## 14. The combined pipeline

`AssemblyReport::of` runs `Problem::integrity`, then M3's `validate`, then merges.

**Ordering: by severity first, then by stage.** Not a concatenation.
`PRE-SOLVE-VALIDATION.md` §4 orders a report so severity is non-decreasing down it, and M3 has
a test asserting that; appending the loader's findings — which span all three severities — to
either end would break it, and a reader would meet an informational note above a blocking
refusal. Within a severity the loader's findings come first, which puts causes before symptoms:
a stored row that should not exist is upstream of anything a compiled problem can say about
itself.

**No deduplication is needed or performed.** The two producers' code sets are disjoint, so no
finding can arrive twice with two wordings. Distinct findings sharing a code are kept —
`PRE-SOLVE-VALIDATION.md` §5 is explicit that distinct actionable problems are never suppressed
for sharing one.

**Blocking behaviour.** `AssemblyReport::may_solve()` is the question a caller asks, and it is
`false` when either a blocking finding exists **or** integrity is non-empty. Warnings and
informational findings never block on their own. `true` is not a claim that a timetable exists:
nothing in Evara can make that claim, and `evara_core::lessons`' rule binds this type as it
binds the modules behind it.

**Partial assembly.** There is none. Every row in scope is either compiled or the load fails
with a typed error; the only rows that do not reach the problem are availability rules that
*cannot* be compiled, each reported individually. `Completeness` marks the one case where a
stored restriction the school actually stated is not being enforced:

```rust
enum Completeness {
    Complete,
    RestrictionsNotEnforced { rules: usize },   // RESOURCE / SUBJECT availability
}
```

An orphaned rule does not reach that variant: its entity is gone, so nothing is unenforced by
dropping it. `Completeness::is_complete()` is what stops an incomplete problem being read as a
complete one.

## 15. The read-only CLI

```text
evara validate <fixture> [--seed N] [--json]                     # unchanged
evara validate --list                                            # unchanged
evara validate --project <path.evaraproj> --school <id> --cycle <id> [--term <id>] [--json]
evara validate --project <path.evaraproj> --list
```

`--project --list` prints the schools, cycles and terms with their identifiers. It is not
decoration: a selection is three UUIDs, nobody knows one by heart, and without it the project
form would be unusable from a terminal.

`--school` and `--cycle` are **required** beside `--project` and never defaulted. `--term` is
genuinely optional, because a grid materialised for the whole year has no term. A selection
flag given without `--project` is refused rather than ignored, and a fixture and a project
together are refused — the parser is strict for the reason `evara fixture` is: a silently
ignored flag would report on a different school from the one the user believed they asked
about.

### Read-only guarantees

`Database::open_existing` **writes**: it runs pending migrations, recreates any missing
change-log trigger, and `meta::ensure` stamps `last_opened_with_app_version` on every open. All
three are right for the application and wrong for a headless inspection of somebody's live
school file. So M4 adds `Database::open_for_inspection`, which does none of the three, and
four independent guarantees hold:

1. **`PRAGMA query_only = ON`**, read back and verified like every other pragma. SQLite refuses
   any statement that would change the file.
2. **No `evara_site_id()` function** on the connection. Every change-log trigger calls it, so a
   write to any tracked table would fail with *"no such function"* even if the pragma were
   lifted.
3. **`Database::write` refuses immediately** with `DbError::ReadOnlyProject`, so a caller that
   reaches for a repository gets a sentence rather than a SQLite error.
4. **`Scheduling` takes `&Database`.** It cannot reach `Database::write`, which needs `&mut
   self`, so "the loader performs no writes" is a borrow-checker property rather than a
   convention.

A project behind the current schema is refused with `DbError::SchemaUpgradeRequired` rather
than migrated, because reaching the supported version is itself a write. The remedy belongs to
the user: open the project in Evara once, under the safety copy the ordinary path takes.

Tested four ways: `last_opened_with_app_version` is unchanged across repeated inspections
(asserted against the ordinary path, which does change it, so the test proves a difference);
the change feed gains no entry; a repository write returns `ReadOnlyProject`; and no revision
is consumed.

### Exit codes — M3's contract, preserved

| Code | Meaning |
| --- | --- |
| `0` | Validation completed with no blocking findings |
| `1` | Validation completed with blocking findings |
| `2` | The command itself failed |

A blocking finding is not a command failure: the command did its job and the answer was "no".
Warnings and informational findings leave the code at `0`.

### JSON

The same diagnostic vocabulary, not a second schema. `DiagnosticReport` serialises itself; the
wrapper adds the selection (project file name, school id and name, cycle id, term id), the
counts the text form shows, `completeness`, `assembly` stats, and `integrity`.

The text form prints integrity errors under their own heading, prefaced by *"please report
this; it is a defect in Evara, not in your data"* — never as something to fix.

## 16. Database-backed fixtures

`crates/evara-db/src/scheduling/tests/`. Each fixture writes a synthetic school into a
temporary `.evaraproj` **through the ordinary repository API** — no raw SQL, no bypassed
trigger, no second project format — then the project is closed, reopened, and the problem
assembled. A fixture that could only be built by writing rows the repositories refuse would be
testing the loader against a database the product cannot produce.

| Fixture | Demonstrates |
| --- | --- |
| `tiny` | A correctly configured school: 1 campus, 1 school-wide schedule, 25 slots of which 20 teach, 2 sections, 8 occurrences |
| `medium` | 80 teachers, 25 groups of 40 (a group-level population of 1 000 with **no** `student` rows), 125 sections, **500** occurrences, 30 teaching slots |
| `infeasible_duration` | A 4-period occurrence where the longest run is 2 |
| `infeasible_room_type` | A required room type the school has none of |
| `multiple_campuses` | 2 campuses, a campus-scoped schedule each, and a hall shared between them |
| `ambiguous_bells` | Two school-wide schedules, both materialised |
| `unmaterialised_bells` | A schedule defined and never materialised — ADR 0033 test 8 |
| `shared_teacher` | Two schools in one project sharing a teacher and a room |
| `unknown_overlap` | A cohort with four children, none of whose pairs is decidable |
| `missing_grid` | A cycle and a schedule with nothing materialised |
| `availability_restrictions` | A day rule, a period rule, a `DISCOURAGED` rule, an orphan, and a `RESOURCE` rule |
| `ineligible_teacher` | A teacher stranded by a later narrowing — §12 |

Deterministic and reproducible: no clock is read and no randomness is used, so two runs build
identical schools. **Identifiers differ between runs** — they are UUIDv7 minted by the
repositories — which is precisely why the loader's contract is about *order*, and why every
assertion is written against order, counts, resolved facts and diagnostic codes rather than
against an identifier somebody wrote down. Temporary files only; no production project is
touched; no cloud and no paid service.

### Where V15 cannot represent an M2 fixture

Stated rather than forced, per the rule that schema semantics are not bent for fixture parity:

| M2 fixture carries | V15 | Equivalent used |
| --- | --- | --- |
| A declared `Overlap` per pair | No column, and the evaluator is M5's | Every pair `Indeterminate`; the group-shape fixtures assert the *inputs* |
| `children_are_disjoint = Disjoint` / `MayOverlap` | Future column (ADR 0035 §4) | `NotStated` everywhere; `unknown_overlap` asserts that |
| An explicit `ExpectedAttendance::Known` | Future column (ADR 0034 §2) | Derived from a single sized group; `tiny` asserts `Known(28)` / `Known(26)` |
| A declared `AttendanceFloor` with no members | Floor is derived from member rows | Real `student_group_member` rows, in the effective-membership fixture |
| `activity.is_locked` / `pinned` | Column does not exist | `pinned` is empty, asserted by `integrity` |

### Round-trip and determinism

Every fixture goes through one helper that asserts three things, so no test can forget them:

1. `Problem::integrity()` is empty.
2. `ProblemIndex::counts()` matches every table of the compiled problem.
3. Loading twice produces an equal `Problem` **and** an equal `ProblemIndex`.

A separate test asserts the whole `AssemblyReport` is equal across two runs. Beyond that, the
suite verifies school/cycle/term identity, campus references, bell-schedule resolution in all
of §6's cases, activity occurrence counts and stable `split_index`, teacher assignments and
project-wide identity, parent edges and declared sizes, room requirements and candidate sets,
shared-resource identity across two schools, availability restrictions per granularity,
diagnostic codes, and diagnostic entity references.

## 17. Determinism guarantees

- Every query carries an explicit `ORDER BY` over columns a unique index makes total.
- Nothing reads `rowid` or relies on insertion order.
- The identifier maps are `BTreeMap`s, and the only thing iterated is the index-ordered `Vec`.
- Each section's teacher and group lists are sorted **by index**, not left in query order,
  because the teacher table is built in two passes.
- No clock and no randomness anywhere in the loader.

Two loads of one snapshot therefore produce equal problems and equal reports. `SOLVER.md` §5
calls this non-negotiable: the same seed over a different index assignment is a different
timetable.

## 18. Performance

**Twenty-eight statements per load, thirty when a term is named, independent of school
size.** Every relationship is read with one bulk query and resolved in memory against the
identifier maps; nothing asks a question per row.

| Stage | Statements |
| --- | --- |
| Context — campuses, days, schedules, periods, slots (5 prepared); school, cycle, cycle owner (3) | 8, or **10** with a term (its own row plus its owner check) |
| Facilities — own room types, borrowed room types, rooms, room campuses, room sharing | 5 |
| Population — effective members, groups, employed teachers, assigned teachers, eligibility, sharing | 6 |
| Lessons — sections, staffing, audience, activities | 4 |
| Availability — 4 entity-existence sweeps, 1 rule read | 5 |

Counted from the code rather than measured by instrumentation, which is why the breakdown is
spelled out: a reader can check the figure against the source, and a stage that gains a query
has one table to update. Nothing enforces the number, so treat it as documentation rather than
a guarantee. **The guarantee is the shape** — no count here depends on how many rows a school
has, which is the property the performance test is set to catch a regression in.

Measured on the fixtures above (Windows 11, debug profile with `opt-level = 1`):

| Measurement | Time |
| --- | --- |
| `tiny` assembly (8 occurrences) | **1.6 ms** |
| `medium` assembly (500 occurrences, 125 sections, 80 teachers) | **3.0 ms** |
| `medium` validation (integrity + M3 + merge) | **1.8 ms** |

62× the occurrences for 1.9× the time, which is the shape a fixed statement count predicts. A test
asserts a loose ceiling rather than a number — a wall-clock assertion on a shared runner is a
flaky test — and it is set to catch the one failure that matters here, an accidental N+1, which
would put `medium` into seconds.

**Scaling concerns, honestly:** two in-memory passes are `O(rules × slots)` (compiling a day or
period rule onto the slots it names) and `O(sections × rooms)` (the candidate sets). At the
sizes in question — 30 slots, 20 rooms — both are nothing. A school an order of magnitude
larger in *rooms* would want the candidate sets indexed by `(campus, room_type)`. Not optimised,
because no profile asked for it, and `PRE-SOLVE-VALIDATION.md` §14's limitation 7 records the
same judgement for the validator.

## 19. Known limitations

1. **No overlap answers.** Every group pair is `Indeterminate` until M5. The visible effect is
   one `group-overlap-indeterminate` warning per school with two or more groups, and
   `expected_attendance` being `Unknown` for every section with two or more attending groups.
2. **The relation's diagonal is unstated** — §9. M5 should declare it.
3. **`children_are_disjoint` and `class_section.expected_attendance` do not exist.** Both are
   accepted future columns; the loader reads the absence honestly rather than defaulting.
4. **Date windows on availability rules are carried and not evaluated** — §11. A date-bounded
   restriction applies for the whole cycle.
5. **Period-level availability does not travel between bell schedules** — ADR 0033 §7, reported
   by M3 rather than solved.
6. **Nothing cross-school is computed.** Sharing is detected and reported; a cross-school clash
   needs a time alignment and a timezone database, and ADR 0032 §4 defers both. A clean report
   for one school says nothing about another school's timetable.
7. **`teacher-not-eligible-for-campus` reports a state the mutation layer should refuse** —
   §12's recommended follow-up.
8. **No Tauri command and no TypeScript binding.** The Generate screen is Phase 6's, so a
   command would have nothing to render. `evara-typegen`'s contract list is unchanged and `npm
   run types:check` is unaffected.
9. **Wording is English only**, as `SOLVER.md` §6 puts localisation in the frontend.
10. **A fatal load reports no diagnostics at all.** When the selection cannot be served there
    is no problem to validate, which is correct and means a caller must handle `Err` as well as
    a report.

## 20. The M5 integration contract

What M5 may rely on, and what it must supply.

**Supplied by M4.**

- A `Problem` whose `integrity()` is empty for every valid project, with dense indices, stable
  reverse maps, and resolution already computed by the shared resolver.
- `GroupFact`'s four overlap inputs: `parent`, `declared_size`, `effective_members`,
  `children_are_disjoint`.
- `SectionFact::audience` and `audience_mask`, and `room_candidates` from the one candidate
  predicate.
- `UnavailabilityIndex` compiled per granularity, with all three statuses.
- Database-backed fixtures to evaluate against, beside M2's in-memory ones.

**Owed by M5.**

1. **The overlap evaluator** — ADR 0035 §3's seven ordered rules, replacing
   `OverlapRelation::unknown(groups)` in `scheduling::mod.rs`'s `overlap` function. That
   function is the single call site, and it is documented as the place to change.
2. **The relation's diagonal**, per rule 2.
3. **`student_group.children_are_disjoint`**, in the migration that adds `constraint_instance`
   / `constraint_ref`. The loader's one conversion is already
   `DisjointnessDeclaration::from_column(None)`, so the change is a column read.
4. **Re-deriving `expected_attendance` once pairs can be proved disjoint** — ADR 0034 §3 clause
   3 in full, where M4 reaches only the one-group case. `lessons::attendance` is the single
   place.
5. **`group-overlap-contradiction`**, which needs member identities rather than a count.
6. **Reusing the hierarchy walk** rather than writing a second — ADR 0038 §3. `MAX_GROUP_DEPTH`
   is already shared between the repository's upward walk and the loader's downward one.
7. **The predicted-resolution count** ADR 0038 defers, added as a third figure beside the two
   `ValidationSummary` already carries.

Unchanged by M5, and not to be relitigated: the loader owns the three stored-row diagnostics;
`Problem` is not widened to carry rejected rows; and there is one bell-schedule resolver.
