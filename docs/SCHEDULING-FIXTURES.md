# Scheduling vocabulary and fixtures — M2

- Date: 2026-10-21
- Base commit: `4dce32f` (`origin/develop`, "Merge pull request #25 from
  Rahul9046/docs/m1-scheduling-architecture-decisions")
- Schema version: **`V15`** — unchanged by this milestone
- Milestone: **M2 — Scheduling vocabulary and test fixtures**

This milestone adds **no solver, no constraint evaluator, no loader, no migration and no
screen.** It adds a strongly typed scheduling vocabulary in `evara-core`, a deterministic
synthetic-school generator beside it, and a `fixture` subcommand that writes one out for a
human to read.

Throughout, the same three categories [`SCHEDULING-ARCHITECTURE-DECISIONS.md`](SCHEDULING-ARCHITECTURE-DECISIONS.md)
uses are kept apart:

| Marker | Meaning |
| --- | --- |
| **Implemented** | Exists in the repository and is covered by tests |
| **Accepted future design** | Decided, built later, in the milestone named |
| **Deferred** | Not decided, not built, and explicitly not promised |

---

## 1. Where the vocabulary lives, and why

**Implemented.** `crates/evara-core/src/scheduling/`, one module per concern:

| Module | Holds |
| --- | --- |
| `clock` | `TimeOfDay`, `TimeInterval`, `DurationPeriods` |
| `index` | the eleven dense-index newtypes |
| `context` | `SchedulingContext`, `BellScheduleFact`, `PeriodFact`, `CycleDayFact`, `BellScheduleResolution`, `NoBellScheduleCause` |
| `problem` | `Problem` and its facts, `RoomFact::is_candidate_for`, `IntegrityError` |
| `placement` | `Placement`, `PlacementSpan`, `Assignment` |
| `population` | `Overlap`, `DisjointnessDeclaration`, `OverlapRelation`, `GroupMask`, `ExpectedAttendance`, `AttendanceFloor` |
| `availability` | `AvailabilityRule`, `AvailabilityScope`, `SlotAvailability`, `UnavailabilityIndex`, `SlotMask` |
| `diagnostics` | `Diagnostic`, `DiagnosticCode`, `Severity`, `EntityRef`, `DiagnosticReport` |
| `outcome` | `Score`, `Seed`, `SolveStatus`, `SolveOutcome`, `UnverifiedCounts` |
| `fixtures` | the generator and the catalogue, behind a feature |

`evara-core` is the only place all four consumers can reach:

```
evara-db ──┐
evara-constraints ──┤
evara-solver ──┤──► evara-core::scheduling
src-tauri / evara-cli ──┘
```

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.2 settles it in one line — *"M2 declares these in
`evara-core`"* — and it is the same argument `evara_core::import` records for the import
vocabulary and `evara_core::lessons` for the demand arithmetic. No new crate was introduced,
because no dependency boundary requires one.

### What is deliberately absent

Nothing here resolves a bell schedule, computes an overlap, evaluates a constraint or solves
anything. There is no `solve`, no `evaluate`, no `resolve_bell_schedule` and no
`overlap(a, b)`. Each is a named deliverable of a later milestone, and a placeholder for one
would mean either a rule implemented twice or a type that appears to work and does not.

`evara-solver` and `evara-constraints` are **unchanged**: both are still documentation-only
crates.

---

## 2. The decisions the vocabulary encodes

Each of M1's four decisions shows up as a type rather than as a comment.

| Decision | Record | Where it appears |
| --- | --- | --- |
| Generation is scoped to one `(school, cycle, term)`; shared occupancy compares **real time**; cross-school sharing is reported | [ADR 0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) | `SchedulingContext`, `TimeInterval::overlaps`, `PlacementSpan::clashes_in_time_with`, `OtherSchool` |
| An activity schedules against **exactly one** bell schedule, resolved by campus compatibility | [ADR 0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) | `BellScheduleResolution`, `NoBellScheduleCause`, `DurationPeriods`, `AvailabilityScope::is_bell_schedule_bound` |
| Expected attendance is a stated number **or unknown**; `room-capacity` leaves the first set | [ADR 0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) | `ExpectedAttendance`, `AttendanceFloor`, `RoomFact::capacity`, `RoomFact::is_bookable` |
| Group overlap is a **three-state** answer | [ADR 0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) | `Overlap`, `DisjointnessDeclaration`, `OverlapRelation`, `GroupMask` |

### Three places the type system carries the decision

**`Unknown` is a variant, never a sentinel.** `ExpectedAttendance::Unknown` has no number in
it, so there is no `0` to leak into a comparison. ADR 0034 §3 names the existing display path
— `group.size.unwrap_or(named_members)` — as the exact collapse this prevents: a section of
size-less, member-less groups shows an audience of `0`, which satisfies every room in the
school including a cupboard.

**`Indeterminate` cannot be reached by negation.** `Overlap::is_clash()` is a method rather
than a comparison, so no caller can write `!= Disjoint` and quietly promote the unproven case
into a clash. `OverlapRelation::unknown(n)` starts every pair — the diagonal included — as
`Indeterminate`, so a relation nobody populated reports ignorance rather than safety.

**A duration is a count of periods and the interval is derived.** `DurationPeriods` is a
count; `PlacementSpan::over` folds the covered periods' intervals into
`[first.start, last.end)` with `TimeInterval::hull`. The two cannot be confused, and the
clash key has exactly one definition.

### `children_are_disjoint`, represented and not migrated

`DisjointnessDeclaration` has three variants mapping exactly onto the future nullable column:

| Column | Variant | Meaning |
| --- | --- | --- |
| `NULL` | `NotStated` | Nobody has answered. Yields `Indeterminate` |
| `1` | `Disjoint` | *No two of my children share a student* |
| `0` | `MayOverlap` | An explicit *they may overlap* |

`NotStated` and `MayOverlap` give the same overlap answer and are different states — which is
why the column is nullable rather than `NOT NULL DEFAULT 0`, and why
`DisjointnessDeclaration::is_unanswered` exists for pre-flight to read. **No migration is
written by M2.** The column remains `SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1's M5 item.

### One candidate-domain rule, in one place

`RoomFact::is_candidate_for(section_campus, required_type)` is the single implementation of
"could a lesson of this section go in this room?" — bookable, reachable from the section's
campus, of the required type, and **nothing about capacity**.

§3.1 of the M1 record names this as a requirement M5 must not lose: *"a domain the manual
editor could bypass is a rule living in two places."* The method is that guarantee. The
loader (M4), the move validator (M11) and the fixtures all call it; a test asserts that every
fixture's stated candidate list agrees with it for every `(section, room)` pair.

---

## 3. Fixture generation

**Implemented.** `evara_core::scheduling::fixtures`.

### In memory, never database-backed

A fixture is a `Problem` and never a `.evaraproj`. Building one needs no database, no
migration and no file, which is what lets `evara-core`, `evara-constraints` and `evara-solver`
all use them — none of those three may open a database.

M4's loader tests will also need **database-backed** fixtures, and seeding a project from one
of these descriptions is the natural way to get them. That is M4's work and no part of it is
done here.

### Behind a feature

`evara-core`'s `fixtures` feature is off by default, so the desktop binary does not carry a
synthetic-school generator. `evara-cli` enables it. The module is also compiled whenever
`evara-core`'s own tests are built, so `cargo test -p evara-core` exercises it with no flag,
and CI's `cargo test --workspace --all-features` and `cargo clippy --all-features` cover it
either way.

### Seed behaviour

- **The same seed produces the same fixture**, on every platform and every toolchain version.
  Asserted on the whole catalogue at four seeds, including `0`.
- **Different seeds produce the same structure with disjoint identities.** The shape — how
  many teachers, sections, occurrences, slots — does not depend on the seed, so a test written
  against one seed is meaningful at another. Every generated `Uuid` does, so two fixtures
  never collide if both were loaded into one project.
- **Nothing reads a clock, an environment variable or a global.** The randomness is twenty
  lines of SplitMix64 written down in `fixtures::rng` rather than a dependency, because `rand`
  does not promise output stability across major versions and a fixture whose bytes changed on
  a dependency bump would invalidate every solver baseline committed against it.
- **Identifiers are well-formed UUIDv7** with a synthetic timestamp derived from the seed,
  advancing one millisecond per identifier — so they sort in creation order the way real ones
  do, and are a pure function of the seed. `CLAUDE.md` requires v7; a v4 fixture could not be
  inserted into a project without being rewritten.

### Why the generator *is* the fixture

Nothing is checked into `fixtures/` as JSON, deliberately. A generator that is a pure function
of a seed already **is** a fixture, so committing its output would add a second copy that
could drift from the first. ADR 0009 names that exact failure for generated TypeScript —
*"otherwise generated degrades into hand-maintained with extra steps"* — and pays for a CI
staleness check because that output is an IPC contract a human reads. A fixture's output is
read by tests, which can call the generator.

### What a fixture carries

Three things, and the middle one is the point.

1. **The inputs** — one or more `Problem`s, with every stored fact the later milestones read.
2. **The expected conclusions** — `Expectations`, recording what M3's diagnostics, M4's
   resolution and M5's overlap evaluator must each decide, *with the ADR rule that decides it*.
3. **A known-feasible arrangement**, where the fixture claims one.

So a fixture is a specification with its data attached. When M5 writes the overlap evaluator
it runs it over these problems and compares against `Expectations::overlaps`; the recorded
`rule` field means a test can assert *which* of ADR 0035 §3's seven rules fired, not merely
that the verdict came out right. Rule 3 before rule 6 is load-bearing — a member row beats a
declaration — and a test that only checked the verdict would pass on an evaluator that got
there by the wrong route.

**The overlap relation in a fixture is declared, not derived.** The fixture states what the
answer must be; the function that computes it from stored facts is M5's. The inputs those
rules read — parent edge, declared size, effective member count, the parent's declaration —
are carried on every `GroupFact` so M5 can derive and compare.

### Two problems, where the case is cross-school

A `Problem` is scoped to one school by construction, so ADR 0032 rule 3's case — an entity two
schools both hold — can only be expressed as **two** problems sharing one identity. That is
also the real situation: the two schools are solved independently and nothing compares their
grids.

---

## 4. The catalogue

24 fixtures. `evara fixture --list` prints this with each one's purpose.

### Tiny — `tiny`

One school, one campus, one five-day weekly cycle, one term, one **campus-scoped** bell
schedule of four teaching periods in two runs of two. Four teachers, three rooms (two general,
one laboratory), three student groups, four class sections, **ten occurrences** of which two
are double periods and four require a laboratory.

What it proves:

- Sections name **no campus** and the only schedule is scoped to the one campus there is, so
  every section resolves under ADR 0033 §2 **clause 3** — the clause that makes the commonest
  configuration in the product work and the one most at risk of being simplified away
  (ADR 0033 test 1).
- **Zero student rows**, and every group pair is nonetheless decided: `Year 7` contains `7A`
  and `7B` by the parent edge (rule 4), and the two classes are disjoint because the cohort
  declares its children so (rule 6). That is ADR 0035 test 1, and the property ADR 0006 is
  most at risk of losing.
- Pre-flight should produce **no finding at all**.

**A known-feasible arrangement is attached.** The laboratory holds Science on four distinct
days; the two Mathematics sections sit in the afternoon run in separate rooms. The tests check
it directly: every occurrence placed exactly once, every multi-period occurrence on a genuine
run of ordinal-consecutive teaching periods of its own resolved schedule on one day, and no
teacher, room or proven-overlapping group pair double-booked.

The interval test used by those assertions is `PlacementSpan::clashes_in_time_with` — the one
implementation — rather than a second copy of any rule, so the test cannot drift from what M5
will enforce.

> **What "by construction" does and does not claim.** It is an existence witness: a timetable
> exists for this problem, and here it is. It does not claim a solver would find it, that it
> is the best arrangement, or that feasibility has been proven in any stronger sense.

### Medium — `medium`

| Figure | Value |
| --- | --- |
| Teachers | **80** |
| Students, at group level | **1 000** — 25 classes of 40, declared, with **no student rows** |
| Student groups | 30 — five cohorts and twenty-five classes |
| Class sections | 125 — each class takes all five subjects |
| Occurrences | **500** — four per section |
| Rooms | 40, across four kinds |
| Teaching slots | 30 — six periods over a five-day cycle |

No individual students, deliberately. A thousand `student` rows would make the fixture slower
to build, larger to serialise and no more useful: `student-clash` reads the overlap relation
over **groups**, and ADR 0035 §5 is explicit that expanding to leaf student sets is the wrong
compiled form.

Eight teachers are unavailable for a whole day each, so `entity-unavailable` has something to
bite on at scale; 50 sections require a specialist room; every fifth section takes one of its
four occurrences as a double.

**Feasible by design, and not proven.** Each class needs 20 of its 30 teaching slots; no
teacher carries more than 8 occurrences; peak demand is about 17 concurrent lessons against 40
rooms. Those counting arguments are asserted by tests so the claim cannot quietly become
false. **No arrangement is attached and none is claimed** — only a solver can say a timetable
exists, and `evara_core::lessons`' rule applies here too.

Pairs of classes in **different** year levels are `Indeterminate` and are expected to stay so:
nothing stored relates them, and unrelated is not disjoint.

### Infeasible — `infeasible-duration`

A three-period occurrence in a school whose bell schedule runs P1, P2, Break, P3, P4, so the
longest unbroken run of teaching periods is **2**.

| | |
| --- | --- |
| **Expected diagnostic** | `duration-exceeds-longest-run` |
| **Severity** | Blocking |
| **Affected entity** | the **activity** — `EntityKind::Activity`, by its `activity.id` |
| **Why that entity** | `activity.duration_periods` is the field whose value is wrong, and the activity is the row a user would edit |

Conclusive from the fixture's own data, with no solver and no search: ADR 0033 §5 requires `d`
ordinal-consecutive teaching periods of the activity's **own resolved schedule**, and a run is
broken by a non-teaching period between two teaching ones.

Two further properties are load-bearing:

- The section's **other** occurrence is placeable, so a finding about the whole school or the
  whole section would be wrong.
- The school holds a second, longer schedule that is **not materialised** into the grid. ADR
  0033 test 10 requires a longer run available elsewhere in the school to be irrelevant,
  because a run never spans two schedules. `evara_core::lessons::CycleCapacity` takes the
  school-wide maximum and so stays silent here — correct, because it can only under-report.

The other candidate impossibilities the brief offered were considered and not used as the
primary case. "One teacher required for two simultaneous mandatory activities" is not
conclusive without locks, which are not in the MVP; the room-type case became its own fixture.

### Second impossibility — `infeasible-room-type`

A section requiring a laboratory at a school whose only laboratory is **not bookable**.
Expected: `no-room-of-required-type`, Blocking, about the **class section** — the requirement
lives on the section's course and every occurrence fails identically. Exercises the room
candidate domain rather than the time one, and ADR 0034 test 13: `is_bookable = 0` removes a
room from every candidate set, independent of capacity.

### Focused fixtures

| Fixture | Covers |
| --- | --- |
| `single-campus-school-wide-bells` | ADR 0033 §2 clause 1; test 2 |
| `two-campuses-identical-bells` | ADR 0033 test 4 — two disjoint slot sets over the same minutes |
| `two-campuses-different-bells` | ADR 0032 tests 2, 3, 5; ADR 0033 test 5 — shared hall and shared teacher at overlapping minutes under **different** `timeslot_id`s |
| `ambiguous-bell-schedule` | ADR 0033 tests 6–7 — `AmbiguousBellSchedule` naming both candidates |
| `unmaterialised-bell-schedule` | ADR 0033 test 8 — defined, never materialised, not a candidate, no ambiguity |
| `empty-grid` | ADR 0033 test 9 — `no-grid-materialised` once, about the school |
| `shared-teacher-across-schools` | ADR 0032 tests 6–7 — one `teacher.id` in two problems; warned, never compared |
| `different-school-calendars` | ADR 0032 tests 10–11 — different terms, no calendar coverage, no rule affected |
| `overlapping-group-membership` | ADR 0035 rule 3, test 4 — proved by member rows, no declaration needed |
| `disjoint-sibling-groups` | ADR 0035 rule 6, test 6, truth table 12 — disjointness propagates to descendants |
| `unknown-group-overlap` | ADR 0035 truth table 8, test 2 — the state every existing project is in |
| `declared-disjoint-contradicted` | ADR 0035 truth table 9, test 5 — facts beat declarations |
| `pigeonhole-overlap` | ADR 0035 rule 5, truth table 13, test 8 — proved with no member row anywhere |
| `two-divisions-of-one-cohort` | ADR 0035 §4, test 12 — the case a flat sibling rule gets wrong |
| `deep-group-hierarchy` | ADR 0035 test 14 — eight levels, members only at the leaves |
| `known-attendance` | ADR 0034 tests 1–2, 10 — explicit value wins; sound sum; a bookable room of capacity 0 |
| `unknown-attendance` | ADR 0034 tests 3–5, 7 — four reasons, all `Unknown`, none `0` |
| `roster-without-group` | ADR 0035 §7 test 10; ADR 0034 test 6; ADR 0029 §4 test 11 |
| `missing-availability-context` | `V11`'s documented default — no rules means available |
| `custom-rotating-cycle` | ADR 0033 test 14, ADR 0035 test 16 — six-day rotation, no weekday read |

The edge cases are deliberately **not** piled into the medium fixture. Each focused fixture is
small enough to read in full, which is what makes a failing assertion diagnosable.

---

## 5. Diagnostics

**Implemented as a contract; populated by nobody.** 22 codes, each with a stable kebab-case
key, a fixed severity and documentation naming the record it comes from.

| Severity | Count | Meaning |
| --- | --- | --- |
| **Blocking** | 9 | The remedy is a field the user can fill in, and until they do the solve cannot produce a meaningful answer |
| **Warning** | 10 | Something real is not being checked. Never blocks. Always counted |
| **Info** | 3 | Two legitimate stored figures disagree; neither is overridden |

Severity comes from the **code**, not from the reporting site, because the classification is a
decision (§3.2 of the M1 record) rather than a judgement call — a caller that could pick its
own severity could quietly downgrade a refusal.

Every code is grounded in §3.2's classification table or `SOLVER.md` §8's Layer-1 checks; none
is speculative. One is documented as **unreachable** —
`teacher-not-eligible-for-campus`, which trigger pairs already prevent — and exists so a
loader that encounters it reports a damaged project rather than building a timetable on it.

M3 populates these. No rule is implemented in M2, and nothing in `diagnostics` inspects a
`Problem`.

---

## 6. Solver-result vocabulary

**Implemented as a contract; produced by nobody.** No function in the workspace returns a
`SolveOutcome`.

`SolveStatus` has five variants rather than a boolean, because "the problem is impossible",
"the user pressed cancel" and "the engine broke" are not one state — and a user told only that
generation failed cannot tell which happened, which is the failure mode `SOLVER.md` §8 opens
by naming.

`UnverifiedCounts` is a **mandatory** field of an outcome, not an optional extra:
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4 requires a timetable to be described as
having no *confirmed* clash within the school solved, with the unverified counts beside it —
`Indeterminate` student pairs, unchecked capacities and shared entities. All zeroes means
there is nothing to disclose; it still does not license the phrase "conflict-free".

---

## 7. The `fixture` command

**Implemented.** `evara fixture <name> [--seed N] [--out DIR]`, plus `--list` and `--help`.

```text
evara fixture --list                      # every fixture and what it proves
evara fixture tiny                        # pretty JSON on standard output
evara fixture medium --seed 7 --out build # writes build/medium-7.json
```

- It generates **synthetic test data**, not a timetable. Nothing in Evara generates a
  timetable yet, and the command's help text, its listing and its completion message all say
  so. A test asserts the blurb never claims otherwise.
- It reads no project, opens no database, applies no migration and writes nothing unless
  `--out` is given. The summary line goes to standard error so it cannot contaminate the JSON.
- Argument parsing is hand-rolled: no `clap`. Four options over one subcommand is not worth a
  dependency and a licence review in a binary whose other seven commands do not exist yet.
  An unrecognised option is an **error** rather than something to ignore — a silently dropped
  `--sed 7` would write a file the user believed was seed 7.
- `serde_json` is the only new dependency on `evara-cli`, and it was already in the tree.

### Serialisation

`Problem` and `Assignment` round-trip in **both** directions, because `SOLVER.md` §9 requires
a JSON-serialisable pair for `evara-cli solve fixtures/<name>` and for benchmark baselines.

The fixture *wrapper* types — `SchedulingFixture`, `Expectations` and the expectation records
— are `Serialize` only. That asymmetry is deliberate: a fixture's prose is a compile-time
constant, so the only way to obtain a fixture is to call the generator, and a parsed one could
disagree with the code that describes it. Writing one out is useful; reading one back would be
a second source of truth.

Output is byte-for-byte stable for a given seed, asserted by a test. The availability index is
sorted `Vec`s rather than `HashMap`s for exactly this reason — and because `AvailabilityEntity`
is a tagged enum, a map keyed on it would not serialise to JSON at all. (That bug was real and
the medium fixture caught it.)

---

## 8. Tests

**Implemented.** 255 tests in `evara-core` and 19 in `evara-cli`, up from 83 and 2.

| Area | Asserts |
| --- | --- |
| `clock` | `'HH:MM'` round trip; an unpadded time refused rather than interpreted; text order and minute order agree; abutting intervals do **not** overlap; a double period's hull spans the break |
| `index` | round trip; ordering; serialises as a bare number |
| `population` | only a proved overlap is a clash; the declaration's three states round-trip; `NotStated` ≠ `MayOverlap`; an unpopulated relation reports ignorance; `Unknown` ≠ `Known { people: 0 }` |
| `availability` | an entity with no rules is available everywhere; `Unconstrained` ≠ `Preferred`; a hard rule is not outweighed by a wish; a rule binds only its own slots; period scope is schedule-bound and day scope is not |
| `context` | a single-campus school is recognised from the stored count; only a unique resolution lets a solve proceed; the three `NoBellSchedule` causes stay distinct |
| `problem` | an empty problem is consistent; dangling indices, duplicate identities, a self-parenting group and a mis-sized relation are each reported |
| `placement` | a span's interval is its first and last slot's; two spans at overlapping minutes under **different** slot ids clash; adjacent ones do not; unscheduled is a state |
| `diagnostics` | every code has a distinct kebab-case key; the serialised name matches the key; severity comes from the code |
| `outcome` | only a zero hard score is feasible; scores compare lexicographically with hard dominating; the five outcomes stay distinct |
| `fixtures::rng` | same seed, same sequence; different seeds diverge; a zero seed does not degenerate; identifiers are well-formed v7, unique, sorted, and disjoint between seeds |
| fixture catalogue | determinism at four seeds; same structure with disjoint identities; no duplicate identity within a fixture; no dangling reference; every slot's derived fields agree with its period; masks agree with the relation **in both directions**; candidate lists agree with the one candidate rule; ordinals run from 1 with no gaps; generation mutates nothing outside itself |
| per fixture | the tiny fixture's shape, its clause-3 resolution, its zero-student-row completeness and its arrangement; the medium fixture's scale and counting arguments; each impossibility's contradiction and exact expected diagnostic; the three overlap states and the three declaration states each exercised; the three severities each exercised |
| guards | exactly one fixture attaches an arrangement; nothing is pinned, because locks do not exist |

Property-style assertions run across four seeds including `0` — determinism, referential
integrity, unique identities, positive durations and valid period references — rather than
introducing a property-testing dependency for them.

---

## 9. How later milestones should consume this

| Milestone | Use |
| --- | --- |
| **M3** pre-solve diagnostics | Produce `Diagnostic`s; compare against `Expectations::diagnostics`, which records the exact code **and** the affected row. `infeasible-duration` and `infeasible-room-type` must produce a specific cause with no solver involved |
| **M4** `Problem` loader | Build a `Problem` from SQLite; implement ADR 0033 §3's `resolve`; assert `Problem::integrity()` is empty on real projects. Seeding a project from a fixture description gives the database-backed fixtures this milestone does not provide |
| **M5** registry + evaluators | Implement ADR 0035 §3's seven rules; run them over every fixture and compare against `Expectations::overlaps`, including the `rule` each cites. Call `RoomFact::is_candidate_for` rather than restating the candidate predicate. Carries the `children_are_disjoint` migration |
| **M6** construction solver | Search over `Problem`, return `SolveOutcome`. The tiny fixture's attached arrangement is a lower bound on what a solver must achieve |
| **M7** persistence | Write an `Assignment` back against the `Uuid` each fact carries |
| **M9** review | Render `UnverifiedCounts`; never describe a timetable as conflict-free |
| Benchmarks | `evara fixture medium --out …`, or call the generator; score baselines arrive with Phase 6 |

---

## 10. Known limitations

1. **No fixture is database-backed.** M4 needs that and will build it. Nothing here opens a
   database, by design.
2. **The medium fixture's feasibility is designed, not proven.** Its counting arguments are
   asserted; no arrangement is exhibited.
3. **The overlap relation and the bell-schedule resolution in every fixture are *declared*.**
   They are the expected output, not a derivation. That is the point of the split, and it means
   a fixture cannot catch an evaluator bug until the evaluator exists.
4. **`Problem::pinned` is always empty.** `activity.is_locked` is not in `V15`, and
   `activity-fixed` joins the constraint set once locks exist.
5. **Room capacity has no `Unknown`.** `room.capacity` is `NOT NULL`, so it is always a stated
   number; `0` means zero seats and is reported as suspicious rather than reinterpreted
   (ADR 0034 §5). The uncertainty in capacity checking is entirely on the attendance side. The
   brief asked for unknown room capacity to be distinguishable from zero; the accepted decision
   says there is no unknown to distinguish, and the accepted decision governs.
6. **`class_section.expected_attendance` does not exist**, so every fixture's attendance is
   stated by the fixture rather than read from a column. The column is M2+ and optional
   (§4.1).
7. **Resource scheduling, lines, per-occurrence overrides and soft constraints have no
   vocabulary here.** All are post-MVP, and inventing types for them would be the mistake `V14`
   named about `subject.colour`.
8. **No TypeScript bindings.** Nothing in `scheduling` crosses the IPC boundary yet, so nothing
   is added to `evara-typegen`'s contract list. M3's diagnostics will be the first to need them.
9. **The `zip` advisories are unresolved and pre-existing.** Three RUSTSEC advisories reach the
   tree through `calamine` and `rust_xlsxwriter` in `evara-import`. They predate this milestone,
   are untouched by it, and need a separate maintenance PR; `deny.toml` is not weakened.

---

## 11. What M2 did not do

Stated so nothing below is mistaken for an oversight.

- No migration, and `V15` is unchanged.
- No solver, no engine, no stub that returns a fabricated timetable.
- No constraint evaluator and no registry entry. `evara-constraints` is untouched.
- No `Problem` loader and no database access from `evara-core`.
- No generation command, no timetable persistence and no timetable UI.
- No checked-in fixture JSON, and `fixtures/basic-school/problem.json` is left where it is —
  still read by nothing.
- No dependency upgrades, and one new dependency edge (`serde_json` on `evara-cli`) to a crate
  already in the tree.
