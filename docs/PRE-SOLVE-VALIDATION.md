# Pre-solve validation — M3

- Date: 2026-10-21
- Base commit: `552ef4f` (`origin/develop`, "Merge pull request #26 …")
- Schema version: **`V15`** — unchanged by this milestone
- Milestone: **M3 — Pre-solve input validation**

M1 decided; M2 declared 22 diagnostic codes and produced none. M3 produces 19 of them, and
gives the other three a named owner.

It adds **no solver, no constraint evaluator, no registry entry, no loader and no migration.**
`evara-constraints` and `evara-solver` remain documentation-only crates.

---

## 1. Validator ownership

**Implemented.** `crates/evara-core/src/scheduling/validate/`, four modules:

| Module | Stage | Holds |
| --- | --- | --- |
| `mod.rs` | — | `validate`, `ValidationSummary`, the shared `Pass`, the ordering policy |
| `schedule.rs` | 1 and 2 | grid, bell-schedule resolution, duration, room requirement, candidate sets |
| `counting.rs` | 3 | the three implementable Layer-1 counting checks |
| `uncertainty.rs` | 4 and 5 | every warning, and the two informational findings |

Beside it, `crates/evara-core/src/scheduling/resolve.rs` — the one bell-schedule resolver
([ADR 0036](adr/0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md)).

### Why `evara-core` and not `evara-constraints`

`ARCHITECTURE.md` §3 rule 2 reserves `evara-constraints` for *"`type_key`-addressed evaluators
with a JSON Schema and an `explain()`, each scored against a candidate timetable"*, and adds:
*"A rule that has an answer **before** any placement exists is not one of those and belongs in
`evara-core`."*

Every finding here has an answer before any placement exists. `evara_core::lessons`' module
header makes the identical argument for `DemandFinding`, and this is the same kind of thing at
a larger scale. Putting it in `evara-constraints` would mean either nineteen incomplete
constraint types two milestones early, or a second kind of thing in a crate whose whole value
is that it holds one kind.

No new crate was introduced, because no dependency boundary required one. The three layering
tests M2 added still pass: nothing in `evara-core` reaches a database, a window or a sibling
crate.

## 2. Public validation API

```rust
// evara_core::scheduling
pub fn validate(problem: &Problem) -> DiagnosticReport;

impl DiagnosticReport {
    pub fn may_solve(&self) -> bool;        // added in M3
    pub fn blocks_solve(&self) -> bool;
    pub fn count_of(&self, Severity) -> usize;
    pub fn count_code(&self, DiagnosticCode) -> usize;
    pub fn of_severity(&self, Severity) -> impl Iterator<Item = &Diagnostic>;
    pub fn has(&self, DiagnosticCode) -> bool;
}

pub struct ValidationSummary { /* … */ }
impl ValidationSummary {
    pub fn of(problem: &Problem, report: &DiagnosticReport) -> Self;
    pub const fn may_solve(&self) -> bool;
    pub const fn is_clean(&self) -> bool;
}
```

`validate` is pure: no I/O, no clock, no mutation, deterministic. A test asserts the problem is
unchanged after a call, and another asserts two calls produce an identical report.

**`may_solve` is the one place a caller asks whether a solve is worth starting.** It exists so
no caller re-derives it by filtering severities — a caller that did could admit a blocking
finding by forgetting a variant.

### `ValidationSummary`, and why it is not `UnverifiedCounts`

`UnverifiedCounts` counts *concurrently placed* activity pairs, so it needs an assignment and
belongs to M6 and M9. `ValidationSummary` is the **pre-solve** analogue, about inputs:

| Field | Meaning |
| --- | --- |
| `blocking` / `warnings` / `informational` | finding counts by severity |
| `indeterminate_group_pairs` | unordered group pairs nothing settles |
| `unanswered_disjointness` | parent groups with ≥2 children and no answer |
| `unchecked_attendance` | sections whose room size will not be checked |
| `shared_entities` | teachers and rooms another school also holds |

It reads the **problem** for the last four, because one aggregate finding stands for however
many pairs it aggregates — which is the whole point of aggregating it.

## 3. Blocking, warning, informational

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2 is the specification, and read as a whole it says
one thing:

> A hard error is reserved for an input whose remedy is a field the user can fill in;
> everything else is counted and announced; nothing is assumed.

| Severity | Blocks | Meaning |
| --- | --- | --- |
| **Blocking** | yes | The remedy is a field the user can fill in, and without it a solve cannot produce a meaningful answer |
| **Warning** | no | Something real will not be checked, or is shared. Always counted |
| **Informational** | no | Two legitimate stored figures disagree. Nothing is overridden |

**Severity comes from the code, not from the reporting site.** `DiagnosticCode::severity()` is
a `const fn` and there is no way to override it, because the classification is §3.2's decision
rather than a judgement call where a finding is raised — a caller that could choose could
quietly downgrade a refusal.

### Rows classified as *no effect* produce nothing

§3.2 classifies incomplete `calendar_day` coverage as having **no effect** on any decision.
M3 therefore emits no finding about dates at all, and the `different-school-calendars` fixture
— which carries no calendar mapping — asserts it.

### Rows classified as *deferred, unsupported* are not turned into checks

Cross-school clash detection, project-level availability for a shared person and individually
enrolled students as a scheduling population stay unimplemented. The sharing is **warned**
about (ADR 0032 rule 3) and nothing claims a comparison was made.

## 4. Diagnostic ordering

Five stages, in this order, so a reader fixes causes before symptoms:

1. **Scheduling context** — no grid, no bell schedule, an ambiguous one. Nothing downstream is
   meaningful until these are answered.
2. **Structure and duration** — an occurrence longer than any run, a required room type nothing
   satisfies, a candidate set availability has emptied.
3. **Proven aggregate impossibility** — the counting arguments.
4. **Uncertainty warnings.**
5. **Informational discrepancies.**

Within a stage, entities are visited in **index order** — the problem's canonical order, and
the order the interface lists them in. That is the stable secondary key. Sorting by `Uuid`
would also be deterministic and would be less useful to a reader, since a UUIDv7 orders by
creation time rather than by anything on screen.

No `HashMap` is iterated anywhere in the validator; every collection is a `Vec` in index order.
A test asserts the report is byte-identical across two calls, and another asserts severity is
non-decreasing down the report.

## 5. Deduplication

| Rule | Where |
| --- | --- |
| **A missing grid is reported once, about the school** — never once per section | ADR 0033 test 9. When `G` is empty, stage 1 emits one finding and the per-section resolution stage is **skipped entirely** |
| **A room requirement nothing satisfies is reported once per section** — never per occurrence | The requirement lives on the section's course and every occurrence fails identically |
| **Indeterminate overlap is one aggregate finding per school** — never one per pair | ADR 0035 §6 forbids a list of pairs; a modest school has hundreds |
| **The three unplaceable-occurrence codes have a precedence** | `duration-exceeds-longest-run` → `no-room-of-required-type` → `empty-candidate-set`. All three could fire on one badly configured section; the most specific is the actionable one |
| **Zero room supply is the degenerate case only** | `no-room-of-required-type` covers it; `room-type-demand-exceeds-supply` skips a type with no eligible rooms so the same fact is not reported twice under two codes with different remedies |

**Deduplication key.** For a solve-level finding, the school. For a section-level finding, the
section. For an occurrence-level finding, the activity. Distinct actionable problems are never
suppressed for sharing a code: `unknown-attendance` produces four
`expected-attendance-unknown` findings because the remedy is per section.

## 6. Every diagnostic code, its trigger and its owner

19 produced by M3, 3 owned by M4, 1 by M5.

### Blocking — produced by M3

| Code | Trigger | Subject |
| --- | --- | --- |
| `no-grid-materialised` | `G` is empty | School |
| `no-bell-schedule` | A section resolves to no schedule; carries one of three causes | Class section |
| `ambiguous-bell-schedule` | A section resolves to two or more | Class section |
| `duration-exceeds-longest-run` | An occurrence needs more consecutive teaching periods than its resolved schedule offers in a row | Activity |
| `no-room-of-required-type` | A section requires a room type and its candidate set is empty | Class section |
| `room-type-demand-exceeds-supply` | Demand for a type exceeds `eligible rooms × usable slots`. **Added in M3** | Room type |
| `empty-candidate-set` | Runs and rooms exist, and availability removes every option | Activity |
| `teacher-load-exceeds-slots` | A teacher's assigned periods exceed the slots available to them | Teacher |
| `group-demand-exceeds-slots` | A group's lessons exceed the slots available to it | Student group |

### Blocking — produced by M4's loader

| Code | Why not M3 |
| --- | --- |
| `teacher-not-eligible-for-campus` | A violated database invariant, reported as corruption. ADR 0028 makes eligibility a trigger pair's job, and a solver-side duplicate would be a second implementation. [ADR 0037](adr/0037-stored-row-diagnostics-belong-to-the-loader.md) §4 |

**Implemented in M4**, and it turned out to be *reachable* — ADR 0037 §4's claim that a
consistent database cannot hold one is wrong at `V15`. The eligibility trigger pair fires on
`class_teacher` and never on `teacher_campus`, and neither `assign_teacher_campus` nor
`remove_teacher_campus` consults `class_teacher`, so narrowing a teacher who already teaches
elsewhere reaches it through two ordinary calls. See
[SCHEDULING-PROBLEM-ASSEMBLY.md](SCHEDULING-PROBLEM-ASSEMBLY.md) §12, which also names the
follow-up the mutation layer should take.

### Warning — produced by M3

| Code | Trigger | Subject |
| --- | --- | --- |
| `shared-entity-across-schools` | A teacher or room another school also holds | Teacher / Room |
| `group-overlap-indeterminate` | Any group pair the relation does not settle | School |
| `section-without-attending-group` | A section with an empty audience | Class section |
| `section-without-teacher` | A section with nobody assigned | Class section |
| `expected-attendance-unknown` | A section whose attendance cannot be derived | Class section |
| `group-size-unknown-with-members` | A group with no declared size and ≥1 member | Student group |
| `bookable-room-zero-capacity` | A bookable room recorded as holding 0 | Room |
| `teacher-under-several-bell-schedules` | A teacher teaching under ≥2 schedules in scope | Teacher |

### Warning — owned by M4 or M5

| Code | Owner | Why not M3 |
| --- | --- | --- |
| `orphan-availability-reference` | M4 — **implemented** | About a stored row a compiled `Problem` has already dropped — [ADR 0037](adr/0037-stored-row-diagnostics-belong-to-the-loader.md) |
| `group-overlap-contradiction` | M5 — **implemented** | Needs member **identities** and rule 3 beaten against rule 6. A `Problem` carries a member *count*, so M3 could not detect it even if it owned the rule. The overlap evaluator returns the contradictions it meets and the loader reports them |

### Informational

| Code | Owner | Trigger |
| --- | --- | --- |
| `declared-periods-disagree` | M3 | A course's declared periods ≠ its section's actual demand |
| `expected-attendance-disagrees-with-groups` | M3 | A stated attendance ≠ the sum of its groups' declared sizes |
| `availability-entity-kind-not-scheduled` | M4 — **implemented** | A rule naming a `RESOURCE` or `SUBJECT`. ADR 0037 |

A test asserts every declared code is **implemented or deferred with a named owner, never both
and never neither**, and that no deferred code is ever produced by any fixture at any seed. So
a code cannot drift into being quietly abandoned, nor be fabricated to reach full coverage.

**After M5, all 23 codes have a producer.** The loader implements its four —
`evara_db::scheduling::LOADER_DIAGNOSTICS` names them, so a test asserts the two producers'
sets are disjoint rather than trusting it — and the last of them,
`group-overlap-contradiction`, arrived with the overlap evaluator that can see a member row
contradicting a declaration.

## 7. Required message content

Every finding carries a stable code, its code-derived severity, a structured `EntityRef`
subject (id **and** label), related references, prose, and a remedy where one exists.

Asserted over every fixture:

| Property | Test |
| --- | --- |
| The message names its subject's label | `every_message_names_its_subject` |
| Every blocking finding states a remedy of real length | `every_blocking_finding_states_a_remedy` |
| No message carries `<`, `>`, `{`, `}`, `%`, `TODO` or a double space | `no_message_carries_markup_or_an_unresolved_placeholder` |
| Every message is a capitalised sentence over 40 characters, ending in a stop | `every_message_reads_as_a_sentence` |
| Every finding names a row the problem holds | `every_finding_names_a_row_the_problem_holds` |

Messages are authored in Rust. `SOLVER.md` §6: *"The frontend renders and localises; it does
not author the reason."* The structured fields are carried **as well as** the prose, never
instead of it.

### The three load-bearing warnings, as produced

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §8 risk **B** names their wording as a deliverable.

**A. Cross-school sharing** — names the entity, **both** schools, the consequence, and
deliberately **no remedy**, because none exists and inventing one would be worse than omitting
it. That is also why it does not block.

> P. Larkmoor is employed by Westbarrow School and by Eastvale Academy. Evara cannot compare
> the two schools' timetables and will not detect a clash between them.

**B. Indeterminate group overlap** — one aggregate finding with the pair count, the questions
that would narrow it, and the parents named.

> 1 pair of Westbarrow School's student groups cannot be compared: Evara does not know whether
> they share students. Until they can be, lessons for those groups may be timetabled at the
> same time. Answering "do these groups share students?" on 1 parent group — Year 10 — would
> narrow this; some pairs may still need membership information as well.
> → Answer whether each parent group's children share students, or record the memberships that
> settle it.

Where every parent has already answered, the second half changes and so does the remedy — see
§12.

**C. Unknown attendance** — names the section, why, what will not be checked, and both
remedies. A floor is described as a floor.

> How many people attend Art 10Y is not known, because no size is declared for 10Y. At least
> 26 are listed in its groups, which is a floor and not a measurement. Room sizes will not be
> checked for its lessons.
> → Enter an expected attendance for Art 10Y, or declare a size for 10Y.

### The three blocking bell-schedule findings

`no-bell-schedule` has **three distinct messages**, one per cause, because three causes have
three remedies:

| Cause | Message | Remedy |
| --- | --- | --- |
| `GridEmpty` | *"No timetable grid has been built for this cycle and term, so X has nowhere to be placed."* | Build one under Timetable Grid Preview |
| `NoScheduleForCampus` | *"X is taught at Northgate Senior, and no bell schedule in this grid applies there."* | Build the grid from that campus's schedule, or make one school-wide |
| `SectionHasNoCampus` | *"X does not say which campus it is taught at, and this school's bell schedules are each tied to one campus."* | Give the section a campus, or make a schedule school-wide |

`ambiguous-bell-schedule` names **every** candidate, with its campus, and both remedies:

> Mathematics 9A could be timetabled against 2 bell schedules: Standard Day (school-wide) and
> Northgate Day (Northgate Senior). Evara will not choose between them.
> → Build this grid from one of them, or restrict this generation to one of them.

`duration-exceeds-longest-run` states both figures:

> Design Technology 8A (lesson 1) needs 3 consecutive teaching periods, and Standard Day never
> offers more than 2 in a row. No timetable can place it.
> → Shorten the lesson to 2 periods, or lengthen the run of teaching periods in Standard Day.

## 8. The bell-schedule resolver contract

**Implemented.** `evara_core::scheduling::resolve`. Full reasoning in
[ADR 0036](adr/0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md).

```rust
MaterialisedGrid::of(problem) -> MaterialisedGrid        // G, derived from the SLOTS
grid.resolve(context, section_campus) -> BellScheduleResolution
is_compatible(context, schedule, section_campus) -> bool // ADR 0033 §2's three clauses
only_campus(context) -> Option<CampusIx>                 // clause 3's precondition
teaching_slots(problem, schedule) -> Vec<TimeslotIx>
runs_on(problem, schedule, day, duration) -> Vec<Vec<TimeslotIx>>
longest_teaching_run(problem, schedule) -> usize
```

### `G` is derived from the slots

**An M2 defect found and fixed here.** `SchedulingContext::bell_schedules` was documented as
`G`; it is in fact every schedule the school has **defined**, and three fixtures deliberately
define one they never materialise. Resolving against that field would report an ambiguity at
`unmaterialised-bell-schedule`, which ADR 0033 test 8 exists to forbid. The doc comment is
corrected and `MaterialisedGrid::of` reads `Problem::timeslots`.

### A declared resolution is checked, not trusted

- **Validation computes** the resolution and drives its findings from the computed value. A
  wrong recorded value cannot suppress a true finding or create a false one.
- **A disagreement is `IntegrityError::BellScheduleResolutionDisagrees`**, reported by
  `Problem::integrity` — the channel that exists for loader and fixture bugs. It is not a
  user-facing diagnostic, because the remedy is a code fix and §3.2 reserves a blocking
  finding for remedies the user owns.
- A test asserts declared equals computed for **all 24 fixtures at three seeds**. It passed on
  the first run, so M2's fixtures were right — which was worth establishing rather than
  assuming.

## 9. M4 reuse requirements — met

M4's loader **must** call this resolver to produce the value it records on each `SectionFact`.
It must not implement a second compatibility algorithm; ADR 0036 §2 is the record, and
`Problem::integrity` will report any disagreement if it does.

**It does.** `evara_db::scheduling` builds the context and the slots, derives the grid with
`MaterialisedGrid::of`, and calls `resolve` once per section — which is why sections are
assembled last. There is no compatibility logic in `evara-db`, and the database-backed
fixtures cover all nine of §8's cases.

The design permits resolution before the final `SectionFact` is built: `resolve` needs only a
`SchedulingContext` and the section's campus, so a loader can build the context, derive the
grid, resolve, and then construct the sections. No circular dependency: the resolver is in
`evara-core` and the loader is in `evara-db`, which already depends on it.

M4 also owns the three stored-row codes of §6 — ADR 0037 §3 lists its obligations.

## 10. Student uncertainty counting, and the M5 boundary

**Implemented**, narrowly. Full reasoning in
[ADR 0038](adr/0038-counting-unanswered-questions-is-not-evaluating-overlap.md).

M3 reads two stored facts and evaluates no rule:

| Figure | Read from |
| --- | --- |
| Undecided pairs | `OverlapRelation::indeterminate_pairs` — the relation is already compiled |
| Unanswered declarations | the parent edge and `children_are_disjoint`, for groups with **≥2 children** |

**M3 does not** decide whether two groups share a student, decide whether two placements clash,
treat an undecided pair as disjoint, or produce `group-overlap-contradiction`.

### What the warning may claim

ADR 0035 §6's example promises *"resolves 412 of them"*. Computing that means deciding which
pairs rule 6 would settle — M5's function. So M3 says **"would narrow this"** and adds that
*"some pairs may still need membership information as well"*, because rules 1, 3 and 5 decide
pairs that no declaration affects.

Where every parent has answered and pairs remain — `medium`, with 360 — the message says so
instead, and **the remedy changes with it**:

> Every parent group has already answered, so the pairs that remain are ones with no common
> parent to declare about. Only the memberships themselves would settle those.
> → Record the memberships of the groups involved, so Evara can see which students they share.

Those pairs are classes in different year levels, which share no ancestor: ADR 0035 truth
table 17, *"unrelated is not disjoint"*. No declaration can ever settle them, and telling the
user to answer one would send them to a field that cannot help.

`unanswered_declarations` is the one implementation of the hierarchy walk. M5 should reuse it.

**M5 did, and added the figure this section says it could not produce.** Rule 6 exists in
`evara_core::scheduling::overlap`, and `resolvable_by_declaration` — written beside it, using
its path matching — counts the undecided pairs that answering every open declaration would
settle. So the warning now says *"would settle 6 of them"* rather than *"would narrow this"*,
`ValidationSummary` carries the figure as a third beside the two it already had, and both
earlier figures are unchanged. It is still a prediction about a stated hypothesis, never
*"resolves"*: a parent that answers *may overlap* has answered and settled nothing.

## 11. The Layer-1 counting checks

**Four of six implemented; two deferred for a missing table.**

### The discipline

Demand is **exact or under-estimated**; capacity is **exact or over-estimated**. Under those
two, `demand > capacity` implies no timetable exists. The converse implies nothing: sufficient
aggregate capacity is not a feasible timetable, because aggregate counting cannot see clashes,
contiguity or room types interacting.

A conservative check that misses some impossible configurations is acceptable. A check that
calls a valid configuration impossible is not.

**The unit is a period-slot** — one period of one cycle day. An activity contributes its
`duration_periods`, never `1`; counting activities would under-state demand wherever durations
differ.

**Missing data skips the entity.** A teacher or group any of whose sections did not resolve has
no slot set to bound, so it is skipped rather than bounded with a guess. The unresolved section
has already produced its own blocking finding.

### Check 1 — teacher load

| | |
| --- | --- |
| Demand | Σ `duration` over every occurrence of every section the teacher is assigned to. **Exact** — the teacher must be present for all `duration` periods, and a section with two teachers counts for each |
| Capacity | distinct teaching slots of the schedules those sections resolve to, less hard-unavailable slots |
| Inequality | `demand > capacity` |
| Capacity over-estimates | three ways: two schedules' slots are counted as distinct even where they cover the same minutes; only `UNAVAILABLE` is subtracted, not `DISCOURAGED`; nothing else competing for the teacher is considered |
| Converse invalid | 20 slots against 20 periods is not schedulable — every period could fall on one day |

### Check 2 — room-type supply

| | |
| --- | --- |
| Demand | Σ `duration` over occurrences whose section requires the type. **Exact** |
| Supply | `eligible rooms × distinct usable slots` |
| Inequality | `demand > supply` |
| Supply over-estimates | a room hosts at most one lesson per slot, so this is the ceiling; it ignores availability and counts two schedules' overlapping minutes twice |
| Skipped | a type with **zero** eligible rooms — that is the degenerate case, reported per section as `no-room-of-required-type` with a different remedy |
| Converse invalid | enough room-slots in aggregate says nothing about whether they are free when the lessons need them |

### Check 3 — group period budget

`SOLVER.md` §8's own example: *"Year 9 needs 42 periods per cycle but only 40 teaching slots
exist."*

| | |
| --- | --- |
| Demand | Σ `duration` over sections whose audience **names** this group. **Under-estimated** |
| Capacity | distinct teaching slots of those sections' schedules, less hard-unavailable slots |
| Inequality | `demand > capacity` |
| Why under-estimated | a section taught to an *overlapping* group also consumes these students' time, and attributing that needs ADR 0035 §3's overlap function — M5's. Under-estimating is the safe direction, so the check is conservative rather than wrong. An `Indeterminate` overlap is simply not counted, never assumed disjoint |
| Converse invalid | 20 periods against 30 slots proves nothing: the lessons may all need the one laboratory |

### Check 4 — empty candidate sets

Reads only hard `UNAVAILABLE` rules on the section's teachers, its attending groups and — where
a room is required — its candidate rooms, over the runs its duration could occupy. It answers
one question soundly in one direction: *is every option already removed?* If any run survives,
no finding. It never decides that a run **is** a legal placement, and it considers no other
activity.

M5's `entity-unavailable` — **implemented**, see
[CONSTRAINT-ENGINE.md](CONSTRAINT-ENGINE.md) §2 — enforces the same stored rules against a
candidate timetable; both
read the one compiled index.

### Checks 5 and 6 — deferred, with evidence

| Check | Why | Evidence |
| --- | --- | --- |
| Lines whose members exceed the rooms of the required type | **There is no `line` table.** `V15`'s header names `line` and `line_member` among tables it deliberately did not create, and `ROADMAP.md` Phase 3 still lists the lines editor as remaining work | `grep "CREATE TABLE line"` over `crates/evara-db/migrations/` matches nothing |
| Pinned activities that already conflict | **Still absent, and no longer for this reason.** When this was written there was no lock column and `Problem::pinned` was a hook nothing populated. M9A added `timetable_entry.is_locked` and the editing session fills `pinned` from it — but a *pre-solve* check runs over a compiled `Problem`, and the loader compiles no pins ([adr/0045](adr/0045-a-lock-is-a-property-of-a-placement.md) §3), so the field is still empty for every caller that reaches these checks. The editor asks the same question per candidate instead, through `IncrementalScorer::probe` | `Problem::pinned` is empty after any `evara_db::scheduling` load; `integrity` asserts it |

Neither was stubbed. Inventing the vocabulary is not M3's job.

## 12. CLI usage

```text
evara validate <fixture> [--seed N] [--json]
evara validate --fixture <fixture> [--seed N] [--json]
evara validate --list
evara validate --project <path.evaraproj> --school <id> --cycle <id> [--term <id>] [--json]
evara validate --project <path.evaraproj> --list
```

Both spellings of the fixture argument work, and a test asserts they agree. The fixture form
reads no project, opens no database, applies no migration and starts no solver.

**The project form arrived with M4** and reads a real `.evaraproj`. It likewise applies no
migration and starts no solver, and it **writes nothing**: it opens through
`Database::open_for_inspection`, not `open_existing`, which stamps
`last_opened_with_app_version` on every open. `--school` and `--cycle` are required and never
defaulted — a project holds several schools, and a cycle chosen for the user would validate a
grid they did not choose — while `--term` is optional, because a grid materialised for the
whole year has no term. `--project --list` prints the identifiers to pass back.
[SCHEDULING-PROBLEM-ASSEMBLY.md](SCHEDULING-PROBLEM-ASSEMBLY.md) §15 has the four read-only
guarantees and how each is tested.

```text
$ evara validate tiny
tiny — Westbarrow School
  4 sections, 10 occurrences, 20 teaching slots
  0 blocking, 0 warnings, 0 informational
  Pre-solve validation found no blocking input issues. This is not a promise that a
  timetable exists.
```

The clean phrasing is exact and the disclaimer is attached to it. It never says *"the timetable
is feasible"*; a test asserts the usage text carries the disclaimer too.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Validation completed and found nothing blocking |
| 1 | Validation completed and found blocking findings |
| 2 | The command itself failed — unknown fixture, bad option, a file that is not a project, a school or cycle the project does not hold |

Warnings and informational findings leave the code at **0**. A blocking finding is not a
command failure: the command did its job and the answer was "no", so the two cannot share a
code.

### `--json`

Built from the **same** `DiagnosticReport` serialisation rather than a second schema, wrapped
with the counts the text form prints above the findings.

### The `validate` / `score` name

`validate` was in the CLI's `PLANNED` table describing *"score a project, list violations"* —
post-solve scoring, which is M5's. M3 took the name the brief asked for, so that entry keeps
its phase under the name `score`. The planned `diagnose` is removed, because `validate` is it.

## 13. Fixture expectations

M2's `ExpectedDiagnostic` entries are a **lower bound**: a fixture records findings validation
*must* produce, not the complete set it may produce. The catalogue-wide test asserts every
recorded expectation is produced about the row it names; per-fixture tests assert the rest.

`tiny` is the exception and is asserted **exhaustively** — zero findings at every seed. It was
built to be clean, so a false positive there is as much a defect as a miss.

### What every fixture produces

| Fixture | exit | Findings |
| --- | --- | --- |
| `tiny` | 0 | none |
| `medium` | 0 | 1 warning: `group-overlap-indeterminate` |
| `infeasible-duration` | **1** | `duration-exceeds-longest-run` |
| `infeasible-room-type` | **1** | `no-room-of-required-type` |
| `single-campus-school-wide-bells` | 0 | none |
| `two-campuses-identical-bells` | 0 | `group-overlap-indeterminate`, `teacher-under-several-bell-schedules` |
| `two-campuses-different-bells` | 0 | as above |
| `ambiguous-bell-schedule` | **1** | `ambiguous-bell-schedule` |
| `unmaterialised-bell-schedule` | 0 | none |
| `empty-grid` | **1** | `no-grid-materialised` (once) |
| `shared-teacher-across-schools` | 0 | `shared-entity-across-schools`, in **both** problems |
| `different-school-calendars` | 0 | as above |
| `overlapping-group-membership` | 0 | 2 × `expected-attendance-unknown`, 2 × `group-size-unknown-with-members` |
| `disjoint-sibling-groups` | 0 | `group-overlap-indeterminate`, 2 × `expected-attendance-unknown` |
| `unknown-group-overlap` | 0 | `group-overlap-indeterminate`, 2 × `expected-attendance-unknown` |
| `declared-disjoint-contradicted` | 0 | 2 × `expected-attendance-unknown`, 2 × `group-size-unknown-with-members` |
| `pigeonhole-overlap` | 0 | `group-overlap-indeterminate`, 2 × `expected-attendance-unknown` |
| `two-divisions-of-one-cohort` | 0 | `group-overlap-indeterminate`, 2 × `expected-attendance-unknown` |
| `deep-group-hierarchy` | 0 | 2 × `expected-attendance-unknown`, `group-size-unknown-with-members` |
| `known-attendance` | 0 | `bookable-room-zero-capacity`; `expected-attendance-disagrees-with-groups` |
| `unknown-attendance` | 0 | `group-overlap-indeterminate`, 4 × `expected-attendance-unknown`, `group-size-unknown-with-members` |
| `roster-without-group` | 0 | `section-without-attending-group`, `expected-attendance-unknown` |
| `missing-availability-context` | 0 | none |
| `custom-rotating-cycle` | 0 | none |

No counting check fires anywhere in the catalogue: every fixture is either comfortable or
impossible for a reason reported more specifically.

### The one fixture correction M3 made

**`medium` declared 4 periods per cycle for every section while giving every fifth one a
double**, so 25 sections' declarations disagreed with their demand and `medium` reported 25
informational findings. ADR 0029 §3 makes a disagreement legitimate, so the findings were
*correct* — and 25 of them misrepresent a realistic school, and nobody put them there on
purpose.

Fixed by declaring the periods the fixture actually generates, with a named
`takes_a_double(position)` read by **both** the declaration and the activity list so they
cannot drift again. `medium` now produces exactly one warning. A `GroupOverlapIndeterminate`
expectation was added to it, so the claim already in its notes is checked rather than asserted
in prose.

## 14. Known limitations

1. **Three codes produce nothing until M4**, and one until M5. Their information is not in a
   `Problem` — ADR 0037.
2. ~~**No real project can be validated yet.**~~ **Closed by M4.** `evara validate --project
   <path> --school <id> --cycle <id> [--term <id>]` compiles a real `.evaraproj` and runs these
   checks over it, through an open that migrates nothing and writes nothing. The same three
   exit codes.
3. **Checks 5 and 6 are absent.** `line` does not exist; the pin check is absent because the
   loader compiles no pins, which is still true now that locks do exist — §11.
4. **Check 3 under-counts demand**, deliberately: overlapping groups' lessons are not
   attributed without M5's evaluator.
5. **The indeterminate warning does not say how much each answer buys** — ADR 0038.
6. **No Tauri command and no TypeScript binding.** A `Problem` can be obtained from a real
   project since M4, but the Generate screen is Phase 6's, so a command would have nothing to
   render. Neither M3 nor M4 adds anything to `evara-typegen`'s contract list, and `npm run
   types:check` is unaffected.
7. **`validate` is `O(sections × activities)` in the demand sums.** 125 × 500 on `medium` is
   trivial; a school an order of magnitude larger would want an index. Not optimised, because
   no profile asked for it.
8. **Wording is English only.** `SOLVER.md` §6 puts localisation in the frontend, which cannot
   yet reach these messages.
9. **The group-overlap relation's diagonal is `Indeterminate` in several M2 fixtures.** ADR
   0035 rule 2 makes a group against itself `Overlaps` unless provably empty, so the diagonal
   is a certainty those fixtures leave unstated. It affects nothing M3 does —
   `indeterminate_pairs` counts `a < b` only — and M5 will want it declared. **M4's loader
   leaves it unstated too, and deliberately**: setting it would mean implementing one of ADR
   0035 §3's seven rules in the one place that must not, and nothing reads it.
10. **A loaded project has no overlap answers at all.** M4 compiles
    `OverlapRelation::unknown`, because the evaluator is M5's. The visible effect is that every
    real school with two or more student groups carries exactly one
    `group-overlap-indeterminate` warning, and that `expected-attendance-unknown` fires for
    every section with two or more attending groups.

## 15. How M4 and M8 should call it

M4 implements the first three lines, and a caller should use its combined form rather than
assembling its own:

```rust
let report = db.scheduling().validate(&selection)?;   // M4: load + integrity + M3 + merge
if !report.may_solve() {
    return Err(refused(report));                      // M8 surfaces the blocking findings
}
let outcome = engine.solve(&problem, …)?;             // M6
// carry `report` onto the timetable so it states what was not checked — §5 rule 4
```

`AssemblyReport::may_solve()` is `false` for a blocking finding **or** a non-empty
`Problem::integrity`, so a caller cannot be told a structurally broken problem is ready. The
two channels stay apart: ADR 0036 §4 keeps an integrity error out of the user-facing findings,
because its remedy is a code fix. The longhand is still available —
`Scheduling::load` then `AssemblyReport::of` — for a caller that wants the `Problem` as well.
See [SCHEDULING-PROBLEM-ASSEMBLY.md](SCHEDULING-PROBLEM-ASSEMBLY.md) §14 for the merge rule.

M8 must **not** re-derive whether a solve may proceed from severities; `may_solve` is the
question. M9 renders the warnings beside the generated timetable, so it is described as having
no *confirmed* clash within the school solved and never as conflict-free.
