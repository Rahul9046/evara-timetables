# The Timetable Construction Solver

What `evara-solver` does, what it refuses to claim, and the figures it was measured at.

Shipped in **M6**. This is the first milestone in which Evara produces a timetable.

[SOLVER.md](SOLVER.md) is the **design**: three phases, a move set, a score, a trait. This file
is the **engine**: Phase A, as built. Where the two disagree about a detail, this one is the
record of what the code does, and §16 lists every place they differ and why.

---

## 1. The shape of it

```
evara-core          the vocabulary:  Problem, Assignment, Placement, Score, Seed,
                                     SolveOutcome, SolveStatus, Termination,
                                     SolveStatistics, UnplacedCause, Rng
   ^
evara-constraints   the engine:      Registry, Occupancy, eight evaluators,
                                     evaluate(), IncrementalScorer  <- ADR 0041
   ^
evara-solver        the search:      candidate generation, construction, explanation
   ^
evara-cli           the harness:     evara generate
```

| File | Holds |
| --- | --- |
| `config.rs` | `SolveConfig`, `SolveLimits`, the `Cancel` and `ProgressSink` traits |
| `candidates.rs` | the value range: legal runs, room choices, the seeded walk |
| `construct.rs` | Phase A — the constrainedness order and the placement loop |
| `explain.rs` | why a lesson is not placed, in a sentence somebody can act on |
| `lib.rs` | `SolverEngine`, `ConstructionSolver`, and `generate`'s gate and verification |

**The incremental scorer is not here.** It lives in `evara-constraints` beside the evaluators it
must agree with — [ADR 0041](adr/0041-the-incremental-scorer-lives-with-the-evaluators.md). This
crate decides what to try, in what order, and when to stop; deciding whether a placement is
legal was never its job.

### What M6 is, and what it is not

`SOLVER.md` §5 describes three phases.

| Phase | State |
| --- | --- |
| **A — construction.** Greedy insertion, hardest first | ✅ shipped |
| B — local search. Late Acceptance Hill Climbing over a move set | **Not built.** `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 keeps local search out of the MVP |
| C — polish. Soft-only hill climb | **Not built**, and could not be: no soft constraint exists |

So this engine **constructs** a timetable and does not improve one. That distinction is visible
in the result rather than implied: a `Termination` says why the constructor stopped, and an
exhausted construction is never reported as a proof that no better timetable exists.

Nothing in **this crate** is persisted, and `evara generate` still writes nothing at all. M7
added the persistence beside it: `evara save` is the command that writes, and
[TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) is what it stores.

---

## 2. Candidate generation

A candidate is a `(start slot, room)` pair for one occurrence. The generator builds only pairs
that already satisfy **four of the seven hard rules**:

| Rule | Why a candidate cannot break it |
| --- | --- |
| `duration-contiguous` | the slots come from `runs_on`, which **is** ADR 0033 §5's measure |
| `no-teaching-in-period` | a run is made of `TEACHING` periods, so its first slot is one |
| `room-type-required` | the room comes from `room_candidates`, built by `RoomFact::is_candidate_for` |
| `entity-unavailable` | every slot of the run is checked against the index, for the teachers, the audience and the room |

The three clash rules cannot be pre-filtered: they are about *other* placements and the
generator knows nothing about them. Those are the engine's to answer, per candidate.

### This is a pre-filter, not a second implementation

Each row above is a **call** to the one function that owns the question. `CLAUDE.md` §2's most
important rule is that generation and the editor must not be able to disagree about legality;
the way to pre-filter safely is to call the same predicate. So *"the generator never offers a
placement the editor would refuse"* is a property of one function rather than of two that happen
to agree — and the registered rule still runs over the finished timetable and over every manual
edit.

A test asserts the consequence directly: no fixture in the catalogue, at any seed, is generated
with a hard violation in it, re-checked by the reference scorer from outside the engine.

### Lazy, because the product is large

A school with 500 occurrences, 40 rooms and 30 runs a cycle has 600 000 `(run, room)` pairs.
Materialising them would cost more than the whole problem and would be wasted: construction
accepts the first legal candidate it meets. So the walk holds the run list and the room list —
tens of entries each — and walks their product on demand. Runs are cached per
`(bell schedule, duration)`; `tiny` has ten occurrences and two distinct keys.

### Roomless is a candidate, and it comes last

A section with no `required_room_type` may be timetabled without a room. Every candidate room is
tried first and `None` is the fallback, and both halves matter: without the rooms, unroomed
lessons would stay unroomed where rooms were free; without the fallback, an occurrence would go
**unplaced** rather than unroomed once the rooms filled — trading a lesson with no room for no
lesson at all, which is the opposite of what ADR 0034 §4 decided.

A section that *does* require a type gets no fallback: an unroomed lesson that needs a laboratory
is a hard violation, so offering `None` would be offering an illegal candidate.

### Seeded order, and why it is not index order

Candidates are walked in a permutation derived from `(seed, activity index)`. Index order is
worse than arbitrary: every occurrence would try Day 1 Period 1 first, every teacher and room
would fill from the same corner, and a greedy constructor would strand whatever it considered
last. A seeded permutation spreads placements across the grid for free, and is what makes
`--seed` explore genuinely different timetables.

The permutation is a pure function of the request, so it is identical on every machine — which is
the half of determinism a shuffle usually loses. Two occurrences of one section walk *different*
orders, which stops four periods of the same lesson from queueing for the same slot.

### Zero candidates is a cause, not a number

An occurrence's value range is measured once, before the first placement. Where it is empty, the
reason is kept rather than collapsed into "no candidates", because each has a different remedy:

| Cause | Means |
| --- | --- |
| `no-resolved-bell-schedule` | its section resolves to no single schedule, so it has no value range (ADR 0033 §3) |
| `no-contiguous-run` | no run that long exists in its schedule. Arithmetic (ADR 0033 §5) |
| `no-candidate-room` | it requires a kind of room that no bookable room of its campus is |
| `all-candidates-unavailable` | every slot that would fit is one a stored rule marks `UNAVAILABLE` |

All four are properties of the **problem**, so `UnplacedCause::is_impossible` holds for them.
They are settled before the first candidate is tested and never retried — see §11.

---

## 3. The construction heuristic

`SOLVER.md` §5's Phase A, implemented as written:

> Greedy insertion, hardest first. Activities are ordered by descending *constrainedness*:
> pinned, then long duration, then few feasible (timeslot, room) pairs, then high conflict
> degree (shares teachers/groups with many others). Each is placed in its cheapest feasible
> slot. Locked activities are placed first and never moved.

The four keys, in that order, plus a fifth:

1. **Pinned first.** They cannot move, so everything else must fit around them.
2. **Long duration.** A three-period occurrence has strictly fewer homes than a single, and
   placing it after the singles is how a day ends up with three gaps of one period.
3. **Few feasible pairs.** The value range over the empty timetable.
4. **High conflict degree.** How many other occurrences share a teacher or an audience with it,
   measured with the same masks `student-clash` reads rather than a second notion of "shares
   students". Computed per section and multiplied out, so it is quadratic in the sections and
   not in the occurrences.
5. **Activity index.** Not a heuristic: it is what makes the order **total**, so two runs at one
   seed sort identically. A sort whose ties fall out in whatever order the loader produced is
   not reproducible across a change to the loader.

Nothing populates `Problem::pinned` at `V16` — there is no `activity.is_locked` column — so key 1
is unreachable today. It is implemented anyway, because repair is the mechanism partial
regeneration is built from (`SOLVER.md` §7) and a pin honoured by accident later is a pin not
honoured.

### Static order, plus retry passes

The order is computed **once**, from the empty timetable. The textbook alternative — recompute at
every step and always take the occurrence with the fewest *currently* feasible placements — is a
better heuristic and is not what is implemented, for two reasons.

The first is that `SOLVER.md` specifies a ranking rather than a re-ranking. The second is cost:
proving an occurrence now has **zero** feasible placements means testing all of them, so a
dynamic re-rank is `O(occurrences × candidates)` per step — hundreds of millions of constraint
queries to place 500 lessons.

What recovers most of the benefit for almost nothing is the **retry pass**: after the first
sweep, the occurrences that failed are tried again, because the timetable has changed since they
were refused. Passes repeat until one places nothing, which is a deterministic fixed point — the
arrangement is unchanged and the candidate order is a pure function of the seed, so the next pass
would make identical decisions.

In practice this costs little and buys little, because the first pass places almost everything:
`medium` finishes in one pass and the 2 000-occurrence school in two (§14).

What the loop **cannot** recover is a choice made early that blocks something later. Recovering
that needs backtracking or Phase B, and both are out of M6's scope. The cost is reported rather
than hidden: `Termination::Exhausted` means every remaining occurrence was refused at every
candidate *in the arrangement the constructor had reached*, and `blocked-by-placements` says
exactly that. Neither claims the problem is impossible.

### "Cheapest feasible slot", while nothing is soft

Cost is the score, and no soft constraint is implemented, so every feasible candidate for one
occurrence has an identical delta: it resolves that occurrence's one `activity-scheduled`
violation and adds nothing. There is nothing to compare, and a comparison would be a tie broken
by a pretence of preference. So the **first** feasible candidate in the seeded order wins, and
the order is where the spreading comes from.

When a soft rule exists this becomes a real minimisation over the same candidate walk, and the
walk does not change: the deltas are already in `PlacementVerdict::delta`.

### No placement is accepted without the registry's permission

Every candidate goes through `IncrementalScorer::probe`, and one whose verdict is not `allowed`
is never committed. There is no fast path around that check and no approximation of it. `generate`
then re-evaluates the finished arrangement with the **reference** scorer to prove it — §6.

---

## 4. Constraint integration

One constraint engine, four callers (`CLAUDE.md` §2). M6 is the first of the four to exist, and
it adds no rule, no second evaluator and no approximation.

| Question | Answered by | Where |
| --- | --- | --- |
| Which runs of periods may a `d`-period occurrence occupy? | `runs_on` | `evara-core` |
| Which rooms may this section use? | `RoomFact::is_candidate_for` | `evara-core` |
| May a lesson go in this slot for this entity? | `SlotAvailability::permits_placement` | `evara-core` |
| Do these two occurrences overlap in time? | `PlacementSpan::clashes_in_time_with` | `evara-core` |
| Would this placement break anything? | `IncrementalScorer::probe` | `evara-constraints` |
| What is wrong with this whole arrangement? | `evaluate` | `evara-constraints` |

The rules themselves are M5's and unchanged in substance. What M6 changed in `evara-constraints`
is mechanical and is listed in §16.

### Mandatory means mandatory here too

`generate` is handed what `Registry::compile` produced — the mandatory eight with the school's
own rows on top — and evaluates all of it. The solver never narrows the set it judges by: a rule
the engine declined to run is a check the user believes happened. ADR 0039.

A school that **raised** `activity-scheduled` to HARD gets its partial work back **and** is told
the result does not meet its own completeness requirement — `meets_completeness_requirement` is
that second answer. Discarding the work would leave a school that raised the level strictly worse
off than one that did not, which is the opposite of what raising a level is for.

---

## 5. Incremental evaluation

[ADR 0041](adr/0041-the-incremental-scorer-lives-with-the-evaluators.md) is the record. In short:

**There is no hand-written delta.** The delta is derived from a property of the rules — every
violation names at most two occurrences, so

```text
score(after) − score(before) = involving(after, a) − involving(before, a)
```

is an identity. The scorer applies a tentative placement, asks each rule what it now finds about
that occurrence, and puts the previous state back. `evaluate_involving` is the narrowed query and
shares its body with each rule's full sweep, so no predicate is stated twice.

Maintained: the occupancy, the running `Score`, a count per type. Recomputed on demand: the
violation **list**, by the reference scorer — a search asks for the score thousands of times and
for the list once.

### Full-versus-incremental correctness

`OPEN-DECISIONS.md` R1: *"neither half may land without the other."* Both landed. The gate is
`evara-constraints`' `differential` module, over the whole catalogue and over seeded sequences:

| Property | Asserted |
| --- | --- |
| Every violation names at most two occurrences | over every fixture — the identity is unsound without it |
| Each rule's narrowed query is complete | per rule, per fixture: `⋃ₐ involving(a)` deduplicated `== evaluate` |
| A narrowed query names only the occurrence it was asked about | per rule, per occurrence |
| A mutated occupancy equals a rebuilt one | 200 random place/withdraw operations, `PartialEq` after each |
| An unreadable placement changes nothing | a slot past the grid, an activity past the table |
| The running score equals the reference score | 5 fixtures × 3 seeds × 120 operations, **checked after every single one** |
| The per-type counts equal the reference counts | same sequence, same frequency |
| A probe changes nothing observable | score, assignment and placed count unchanged, 60 probes |
| A probe predicts exactly what the commit does | 80 probe-then-commit pairs, deltas compared |
| A scorer over an existing arrangement starts at the reference score | every fixture — the repair entry point |

The brief's operation list is covered: empty assignment, one placement, many, replacement,
removal, conflicting, non-conflicting, two bell schedules
(`two-campuses-different-bells`), shared rooms (same fixture), indeterminate student overlap
(`unknown-group-overlap`).

**Checked after every operation, not at the end.** A delta that was wrong and then wrong the
other way would pass an end-only check and produce a wrong timetable in between.

---

## 6. Partial solutions, and the verification

### Partial is the point of MEDIUM

`activity-scheduled` is MEDIUM, so an unplaced occurrence makes a timetable **incomplete** rather
than unusable. The engine returns whatever it reached, always. A partial result can be
*adoptable*: eight of nine lessons with no clash among them is a timetable a school can adopt and
place the ninth by hand, and `SolveOutcome::is_adoptable` says so while `is_complete` says the
other half.

The engine never accepts a placement that breaks a hard rule, so it leaves an occurrence unplaced
instead of placing it illegally. That is why `SolveStatus::Infeasible` is **unreachable** from
this engine: a construction that refuses every illegal placement never ends with one. It is kept
for the local search that will reach it.

### The independent check

Construction accepts a placement on the incremental scorer's verdict. The finished arrangement is
then re-evaluated with `evaluate` — the reference implementation — and the two must agree that
nothing hard is broken.

They are checked against each other by the property test on every build, so a disagreement here
is **not an expected outcome**: it is a defect in Evara. It is reported as
`Termination::InternalCheckFailed` with `SolveStatus::Failed`, the violations come back so the
defect can be diagnosed, and the result is never presented as a success — which is the one
failure a school cannot detect for itself.

### The gate

`generate` refuses to search when pre-solve validation refused the inputs: `SolveStatus::Failed`,
`Termination::InputRejected`, the findings attached, no candidate tested.

Searching anyway would be worse than useless. ADR 0033 §3's refusals are inputs with **no value
range at all**, so a solve over one cannot place the affected occurrences and would report them
as *"could not be placed"* — which reads as a difficulty rather than as the unanswered setup
question it is. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.2: *"a hard error is reserved for an
input whose remedy is a field the user can fill in."*

**A warning never blocks.** Indeterminate group pairs, unknown attendance, shared entities — each
is counted, carried and solved over, because the remedy is a setup task or does not exist.

### What was not checked is always said

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4: *"A timetable is never described as
conflict-free. It is described as having no confirmed clash within the school solved, with the
unverified counts beside it."* Four figures, every one a real hole an accepted record chose to
make visible rather than close:

| Figure | Hole | Record |
| --- | --- | --- |
| `indeterminate_student_pairs` | group pairs two different sections' audiences bring together whose overlap nobody declared — not kept apart | ADR 0035 §6 |
| `unchecked_capacities` | occurrences whose expected attendance is `Unknown`, so `room-capacity` was **not evaluated** | ADR 0034 §5 |
| `shared_entities` | teachers and rooms another school also holds — no comparison across the boundary | ADR 0032 rule 3 |
| `unenforced_restrictions` | stored availability rows naming an entity kind nothing schedules | the loader's `Completeness` |

Three are computed from the problem rather than trusted to a caller that might forget one. The
fourth is the caller's, because only the loader knows it. **The fourth figure is new in M6** —
`UnverifiedCounts` had three.

All zeroes is still reported, as *"nothing was left unchecked, and this is still described as no
**confirmed** clash"*.

---

## 7. Status and termination

Two enums, because they answer different questions and a school needs both.

**`SolveStatus`** — what you have. M2's five, untouched.

| Status | Means | Reachable from this engine |
| --- | --- | --- |
| `Complete` | every occurrence placed, nothing hard broken | yes |
| `Partial` | some unplaced, nothing hard broken among the rest | yes, ordinarily |
| `Infeasible` | the best arrangement found breaks a hard rule | **no** — see §6 |
| `Cancelled` | the caller stopped it; the work is returned | yes |
| `Failed` | the inputs were refused, or the internal check failed | yes |

**`Termination`** — why the engine stopped looking. New in M6.

| Termination | Means | A bigger budget might help |
| --- | --- | --- |
| `complete` | nothing left to do | — |
| `exhausted` | every remaining occurrence tried at every candidate, and refused | no |
| `candidate-limit-reached` | the cap on candidate evaluations | yes |
| `step-limit-reached` | the cap on construction steps | yes |
| `pass-limit-reached` | the cap on retry passes, while a pass was still placing | yes |
| `cancelled` | the caller stopped it | yes |
| `input-rejected` | pre-solve refused; nothing attempted | no — correct the input |
| `internal-check-failed` | the reference scorer disagreed. A defect | no |

### Why `Termination` is not a sixth status

A `Partial` result that was *exhausted* and one that hit a *budget* are the same timetable and
two different things to tell a user: the first has been told every remaining lesson was tried and
refused, the second has been told nothing of the kind and should be run again with more budget.
Folding them into `SolveStatus` would make a timetable's completeness and the engine's budget the
same field, which is the collapse that type's own header exists to prevent.

`Termination::was_cut_short` is the one-word form of the distinction, and
`is_reproducible` is `false` for exactly one variant — §8.

---

## 8. Determinism

`SOLVER.md` §5 calls this non-negotiable. Asserted as **whole-outcome equality**: for every
fixture, two runs at the same seed produce `PartialEq`-equal `SolveOutcome`s, statistics
included.

What makes it hold:

- **Stable activity ordering.** The constrainedness key ends in the activity index, so the sort
  is total.
- **Stable candidate ordering.** A permutation of `(seed, activity index)`, drawn by Fisher–Yates
  over `Rng` with a fixed draw count — so the generator's position afterwards does not depend on
  the values drawn.
- **No hash-map iteration.** The run cache is a `Vec` of keys; the occupancy's indexes are sorted
  `Vec`s; the engine's own dispatch is a `match`.
- **No clock in any decision.** `SolveStatistics` carries counters and deliberately no elapsed
  time: a duration on the outcome would make whole-outcome equality machine-dependent, and
  whole-outcome equality is how determinism is tested. Wall-clock time is measured by the
  **caller** and printed beside the result.
- **No threads.** One solve, one thread. `SOLVER.md` §5's background thread and multi-start
  parallelism are the orchestration milestone's; nothing here races.

### Seeds

The seed is recorded on the result, because *"a run whose seed was not recorded cannot be
reproduced"*. The same problem, configuration and seed reproduce the same assignment; **different
seeds may produce different timetables and are not required to**, and both halves are asserted —
over a school with real choice, at least one seed pair must differ, or the seed is decorative.

For a hard school, trying another seed is the cheapest remedy there is, and the report says so
where `blocked-by-placements` appears.

### The one irreproducible run

`Termination::Cancelled`. Cancellation is the caller's to decide and a wall-clock budget reaches
the engine as one it cannot tell from a user pressing stop, so where it stopped depends on how
fast the machine was. `is_reproducible` is `false` for that variant and only that variant, and
the CLI prints the caveat.

---

## 9. Execution limits

Every limit is a **count of decisions**, never a duration.

| Limit | Counts | Default |
| --- | --- | --- |
| `max_candidate_evaluations` | candidate placements whose legality was tested | 50 000 000 |
| `max_construction_steps` | occurrences attempted, retries included | 1 000 000 |
| `max_passes` | sweeps over the unplaced occurrences | 8 |

A time limit would violate `CLAUDE.md` twice over — it reads a clock, and the place it stops
depends on the machine, so the same seed produces different timetables on a laptop and in CI, and
a baseline committed against such a run is meaningless.

**A wall clock is still supported, and it is the caller's.** The engine asks `Cancel::is_cancelled`
between steps and the caller decides what the answer depends on — a button, a deadline, anything.
Keeping the clock outside is what lets the engine stay honest about which of its results can be
replayed.

The defaults are a backstop against a pathological problem, not a budget to tune: they are about
two orders of magnitude above what the 2 000-occurrence upper end needs (§14). Reaching one is
**reported, never silent** — the termination says which, and every occurrence the run did not
reach is `not-reached`, about which nothing is claimed at all.

---

## 10. The result model

`SolveOutcome`, declared by M2 and filled in by M6. Every field M2 declared still means what it
meant; M6 added two about the **run** rather than the timetable.

| Field | Holds |
| --- | --- |
| `problem` | the school, cycle and term — ADR 0032 rule 1, echoed back |
| `seed` | so the run can be reproduced exactly |
| `status` | what came back |
| `termination` | **new** — why the engine stopped |
| `assignment` | what was placed where. Carried for every status |
| `unscheduled` | each unplaced occurrence: its id, its section, a structured cause, a sentence, and the pre-solve finding that already proved it where there is one |
| `score` | from the reference scorer, not the engine's own belief |
| `hard_violations` | which hard rules the result breaks and how often. Empty for a feasible result, and the two must agree |
| `unverified` | what was **not** checked. Never omitted, now four figures |
| `preflight` | the findings, carried onto the result |
| `statistics` | **new** — deterministic counters |

`UnscheduledActivity` gained `section` and `cause`; `UnverifiedCounts` gained
`unenforced_restrictions`; `UnplacedCause`, `Termination` and `SolveStatistics` are new.

---

## 11. Unplaced-activity explanations

A partial timetable is only useful if it is actionable. Each unplaced occurrence carries a
structured cause and a sentence, and the causes fall into two halves that must never be confused.

| Cause | Kind | What the sentence says |
| --- | --- | --- |
| `no-resolved-bell-schedule` | the **problem** | no slots at all. Name the campus, or make a schedule school-wide |
| `no-contiguous-run` | the **problem** | no run that long exists. Shorten the lesson, or add a period |
| `no-candidate-room` | the **problem** | no bookable room of that type. Add one, or change the requirement |
| `all-candidates-unavailable` | the **problem** | a stored rule empties the range. Review the restrictions |
| `blocked-by-placements` | this **run** | every slot clashed with something already placed. *"This is not a proof that no complete timetable exists — try another seed"* |
| `not-reached` | this **run** | a limit stopped it first. Nothing established; run again with more budget |
| `input-rejected` | this **run** | nothing was attempted. Correct the blocking finding |

`UnplacedCause::is_impossible` is the one place the difference is decided, and it is `true` only
for the first four.

### The claim this engine does not make

`evara_core::lessons` carries a rule the workspace inherits: **never report that a demand can be
scheduled.** A solver needs its mirror. A greedy construction that leaves twelve lessons unplaced
has discovered that *its* arrangement had no room for them, and nothing more. Telling a school
*"this cannot be timetabled"* on that evidence would spend the user's trust on a guess, which is
the failure `SOLVER.md` §8 opens by naming.

So the three run-level causes say so in as many words, and the CLI prints them under a heading
that repeats it.

### A pre-solve finding is attached, not re-worded

`UnscheduledActivity::diagnostic` carries the finding that already proved an occurrence
impossible, rather than a second wording of it. Attached only when the finding is one of the four
codes that **prove impossibility** *and* its subject is this lesson or its section — so a stray
warning about the same section can never be attached, and `teacher-load-exceeds-slots` is
excluded by name: it proves *some* lesson cannot be placed and not *this* one, and attaching it
would send a user to fix something that is not broken.

---

## 12. Running it

```
evara generate <fixture> [--seed N] [--json]
evara generate --list
evara generate --project <path.evaraproj> --school <id> --cycle <id> [--term <id>]
               [--seed N] [--json]
               [--max-candidates N] [--max-steps N] [--max-passes N]
```

`solve` is an accepted alias — §16.

A fixture is built in memory from its seed. A project is read from disk, and the school, cycle
and term are **named explicitly**: ADR 0032 rule 1 scopes a solve to one of each, and a project
has held several schools since `V12`, so picking one would build a timetable over a grid the user
did not choose. `evara validate --project <path> --list` prints the identifiers.

### Read-only, and nothing persisted

A project is opened through `Database::open_for_inspection` — the same route `evara validate
--project` and `evara evaluate --project` take. The ordinary path runs migrations, recreates
triggers and stamps `last_opened_with_app_version` on *every* open, so a command asked to
generate from somebody's live school file must not take it. That route hands back a `query_only`
connection, so the guarantee is SQLite's rather than a convention.

**Nothing is written.** A test asserts it by comparing the file's bytes before and after, which
is stronger than trusting the open path's documentation.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | a complete timetable, nothing hard broken |
| 1 | a **partial** timetable — usable, and some lessons are not placed |
| 2 | the command itself failed: an unknown fixture, a bad option, a configuration that does not compile, a file that cannot be read |
| 3 | generation was refused or broke: the inputs were rejected, or the internal check failed |

Four rather than three. A partial timetable is the ordinary outcome of a hard school and the whole
point of `activity-scheduled` being MEDIUM, so it cannot share a code with "the command failed" —
a script could not tell a usable result from a broken one. It also cannot be `0`, because a
caller that asked for a timetable and got eleven twelfths of one needs to know without parsing
prose.

### What a run looks like

```
$ evara generate medium --seed 2026
medium — Northgate Academy
  seed 2026  ·  8 rules evaluated
  500 of 500 occurrences placed, 0 unplaced
  Complete. Every occurrence is placed and no confirmed clash was found within this school.
  stopped because: every occurrence was placed
  score 0 hard, 0 medium, 0 soft
  Independently re-checked with the full evaluator: no hard rule is broken.
  Not checked:
    250 group pair(s) whose shared students nobody has declared — those pairs were not kept apart
  Pre-solve: 0 blocking, 1 warning(s) — run `evara validate` for the detail
  14249 candidate evaluation(s), 13749 rejected, 500 step(s), 1 pass(es)
  57 ms wall clock (measured by this command; the engine reads no clock)
```

```
$ evara generate infeasible-duration
  0 of 2 occurrences placed, 2 unplaced
  Generation did not produce a timetable. See the reason and the findings below.
  stopped because: pre-solve validation refused the inputs, so nothing was attempted
  ...
  Nothing was attempted, because the inputs carry a blocking finding. Correct it and
  generate again — `evara validate` lists them:
    [duration-exceeds-longest-run] Design Technology 8A
    2 lesson(s) were not attempted. Nothing is claimed about whether they can be placed.
```

---

## 13. Tests

| Where | Count | Covers |
| --- | --- | --- |
| `evara-constraints::differential` | 11 | the full-versus-incremental gate — §5 |
| `evara-solver::tests` | 38 | candidate generation, construction, determinism, limits, the gate, the disclosure, the raised level, scale |
| `evara-cli::generate::tests` | 37 | the grammar, the four exit codes, the prose's honesty, and the read-only byte comparison |

The whole M2 catalogue is swept: every fixture, at three seeds each, generated and then
re-evaluated by the reference scorer from outside the engine.

**No fixture is asserted to be fully schedulable except `tiny`.** Only `tiny` ships a
known-feasible arrangement, so only `tiny` is held to completeness — a test that demanded a
complete timetable from `infeasible-duration` would assert a falsehood, and one that demanded it
from the rest would assert something no record claims.

Three tests assert the prose rather than the data, which is the discipline M3's wording tests
found four real defects with: a complete report never says *conflict-free*, an unplaced lesson is
never described as impossible when it is merely unplaced, and a refused run never offers a bigger
budget as the remedy.

---

## 14. Performance

Measured on the development machine (Windows 11, release profile, `lto = "thin"`,
`codegen-units = 1`), single-threaded. Wall-clock figures are the CLI's own measurement around
the call; every other figure is a deterministic counter and is identical on any machine.

| Workload | Occurrences | Placed | Candidate evaluations | Rejected | Passes | Wall clock | Termination |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `tiny` | 10 | 10 (100%) | 14 | 4 | 1 | < 1 ms | complete |
| `two-campuses-different-bells` | 10 | 10 (100%) | 13 | 3 | 1 | < 1 ms | complete |
| `deep-group-hierarchy` | 4 | 4 (100%) | 4 | 0 | 1 | < 1 ms | complete |
| `medium` | **500** | 500 (100%) | 14 249 | 13 749 | 1 | **57 ms** | complete |
| synthetic upper end | **2 000** | 1 993 (99.7%) | 191 111 | 189 118 | 2 | **~900 ms** | exhausted |

Peak working set for the `medium` run, measured as the process's `PeakWorkingSet64`:
**5.0 MiB** — the whole command, the problem and the timetable included. No figure was measured
for the upper-end case, which runs in-process in a test; its problem is four times `medium`'s and
the engine's own state is linear in the timetable, so nothing suggests a different shape.

### What the numbers say

- **No hard violation at any scale.** Every row above was re-evaluated by the reference scorer
  and found clean. That is the figure that matters.
- **The shipped limits are not the binding constraint.** The upper end uses 0.4% of the candidate
  budget and 0.2% of the step budget, and no row terminates on a limit.
- **Most candidates are rejected, and that is the engine working.** 96% at `medium`, 99% at the
  upper end: the schools are built so teachers and rooms are shared, so the clash rules do real
  work per placement. The cost per probe is bounded by the occurrence's own neighbourhood, which
  is why 191 111 probes take under a second.
- **Retries are rare.** One pass for `medium`, two for the upper end. The static constrainedness
  order places almost everything on the first attempt, which is the evidence that a dynamic
  re-rank would buy little for its cost (§3).
- **The 2 000 case leaves seven lessons unplaced**, all `blocked-by-placements`. That is the
  known limitation of a greedy constructor with no local search, it is reported as a property of
  the run rather than of the problem, and §15 is where it belongs.

`OPEN-DECISIONS.md` R5 asked for these numbers as a **budget** rather than a target, and they are
recorded here for that purpose. R2 asks for committed score baselines per fixture; `evara bench`
is still planned and is where they will live.

---

## 15. Known limitations

| Limitation | Why | Where it goes |
| --- | --- | --- |
| **No local search.** A greedy construction that strands a lesson cannot un-choose the placement that stranded it | Phase B is out of the MVP (`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6). Retry passes recover some of it; backtracking recovers the rest | `SOLVER.md` §5 Phase B |
| **No optimisation at all.** Nothing prefers a good timetable to a merely legal one | No soft constraint is implemented, so there is nothing to optimise against | after the MVP |
| **A partial result is never a proof of impossibility** | A property of greedy construction, stated rather than hidden | §11; a proof needs `SOLVER.md` §8's counting diagnostics or Layer 3 |
| **`room-capacity` is not checked.** The engine can place a class of 32 in a room for 20 | ADR 0034 defers it until expected attendance exists. Counted and disclosed | ADR 0034 |
| **Indeterminate group pairs are not kept apart** | ADR 0035 §6: enforce what is proved. Counted and disclosed | ADR 0035 |
| **No cross-school clash detection.** A teacher shared with another school can be double-booked across the two | ADR 0032 §4 — needs a time alignment and a timezone database. Counted and disclosed | ADR 0032 |
| **`RESOURCE` and `SUBJECT` availability rules are not enforced** | Nothing schedules those entity kinds. Counted and disclosed as the fourth unverified figure | the loader's `Completeness` |
| **Locks are not honoured because they do not exist.** `Problem::pinned` is read and nothing fills it | No `activity.is_locked` column at `V16` | Phase 8 |
| **Teacher is not a planning variable.** A section's staffing is fixed input | D7. The solver cannot pick an ineligible teacher because it picks none | OPEN-DECISIONS D7 |
| **No progress events, no background thread** | The traits exist (`ProgressSink`, `Cancel`) and nothing in M6 drives them from a UI | `SOLVER.md` §5; the orchestration milestone |
| **No committed score baselines** | `evara bench` is still planned | R2 |
| **The upper-end measurement is not a catalogue fixture** | Widening a documented 24-fixture catalogue is M2's surface. It is built in the test that measures it | §14 |
| ~~**Nothing is persisted**~~ | ✅ **Resolved in M7.** `evara save` writes a timetable, its run and its findings; `evara generate` is unchanged and still writes nothing | [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) |

---

## 16. What changed elsewhere, and the four places M6 departs from a record

### In `evara-core`

- **`Rng` moved out of `fixtures` into `scheduling::rng`.** The solver needs the same seeded
  generator in the *shipped* product and cannot reach a module behind the `fixtures` feature;
  copying twenty lines of SplitMix64 would have given two generators that could drift and two
  seeds that meant different things. `fixtures::ids` keeps the identifier generator built on it.
- **`Termination`, `SolveStatistics`, `UnplacedCause`** are new; `SolveOutcome`,
  `UnscheduledActivity` and `UnverifiedCounts` gained the fields §10 lists. The five
  `SolveStatus` variants are untouched.

### In `evara-constraints`

- **`Occupancy` is mutable and indexed.** `place` and `withdraw` join `of`; three indexes — per
  teacher, per room, per cycle day — make the narrowing cheap. `taught_by` and `booked_into` were
  scans of the whole timetable and are now lookups, which speeds up the reference scorer too. A
  test asserts a mutated occupancy equals a rebuilt one.
- **`ConstraintEvaluator::evaluate_involving`** is a sixth trait member, implemented by all eight
  rules through the same body their full sweep uses.
- **`IncrementalScorer`** — ADR 0041.

### The penalty sign, corrected

M5 pushed violations with `penalty: constraint.weight` — a positive cost. `SOLVER.md` §3 says
*"Penalties are negative-going integers"* and `Score`'s own doc comment says *"a better score is
a greater score and `Ord` is the comparison the solver wants with no wrapper"*. With positive
costs, `Score::PERFECT` — all zeroes, documented as the best — is the **smallest** score, and
`Ord` ranks a broken timetable above a clean one.

Nothing in M5 noticed, because nothing in M5 compares two scores. M6 is the first milestone that
does, and a reversed `Ord` is a trap for the local search that will use it for *"best score
wins"* and for multi-start's *"best result wins"*. The code now matches both records: a weight is
a non-negative magnitude, an evaluator negates it once when it builds a violation, and
`Score::PERFECT` is the top of the order.

### The four departures, named

1. **The command is `generate`, with `solve` as an alias.** `SOLVER.md` §9 and the CLI's planned
   list spelled it `solve`; M6's brief, the roadmap's phase and the user-facing screen all call it
   `generate`. Both spellings work, there is one implementation, and the alias is listed in the
   help text rather than hidden — so a reader following the accepted record is not told their
   command does not exist.
2. **The construction order is static, not dynamic.** §3 — `SOLVER.md` specifies a ranking, and a
   re-ranking costs `O(occurrences × candidates)` per step. The retry pass is what recovers most
   of the difference, and what it cannot recover is reported.
3. **`IncrementalScorer` is in `evara-constraints`, not `evara-solver`.** ADR 0041. The brief
   implied the solver crate; the predicates it must share are the engine's.
4. **`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 lists "local search and incremental scoring" as
   out of the MVP**, while `SOLVER.md` §4, `CONSTRAINT-ENGINE.md` §7 and `OPEN-DECISIONS.md` R1
   each assign `IncrementalScorer` to M6 by name, R1 as a rule about shipping. The three specific
   records win for the scorer; local search stays out.

### `SolverEngine`, against `SOLVER.md` §2's sketch

Four signature changes, each because a type now exists or a decision has been made.

- **`constraints` is a parameter.** The sketch had none; the registry was M5's. An engine that
  could not be handed a school's own configuration would be one that ignored it.
- **No `SolveError`.** Every way a run can fail is a `SolveStatus` and a `Termination`. A
  `Result` would give a caller two places to look for *"what happened?"* and a reason to throw
  away the partial timetable on the error path — the outcome MEDIUM exists to prevent.
- **`progress` and `cancel` are `&dyn`.** Same shape; it keeps the crate free of any particular
  event system.
- **`start` is accepted and nothing passes one.** Repair arrives with locks; the construction
  already reads a pin where it finds one.

---

## 17. The M7 persistence integration contract

> **Discharged.** M7 is built; [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) is what it
> does and [ADR 0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
> is why. Each item below records what landed, and item 1 records a mistake this file made.

What M7 may rely on, and what it must supply.

**Supplied by M6.**

- `evara_solver::generate` returns a `SolveOutcome` for one `(school, cycle, term)` —
  `ProblemIdentity` is exactly the three identifiers a `timetable` row needs.
- `SolveOutcome::assignment` is one `Option<Placement>` per `ActivityIx`, and `ProblemIndex`
  (M4) turns an index back into the `activity.id` and a `TimeslotIx` into the `timeslot.id`. So a
  `timetable_entry` row is a mechanical projection and needs no second lookup path.
- A `Placement` carries the **start** slot and an optional room. The slots a multi-period
  occurrence covers are derived by `Occupancy`, and `PlacementSpan` is the resolved form where a
  writer needs all of them.
- Every accepted placement satisfies the seven mandatory hard rules, re-checked by the reference
  scorer before the outcome is returned.
- `SolveOutcome::seed` and `SolveStatistics` are what a `solve_run` row records, and both are
  deterministic — so a stored run can be replayed and compared.
- `UnverifiedCounts` is what a stored timetable must carry forward, so ADR 0032 rule 4's wording
  survives being persisted and re-rendered.
- `SolveOutcome` and `Assignment` are `Serialize` **and** `Deserialize`, which `SOLVER.md` §9
  requires.

**Owed by M7.**

1. **The solution writer.** `Assignment` → `timetable_entry`, transactionally, in one write.
   ✅ **Built in M7**, as `evara_db::Timetables::save_generated`.

   > **This item was wrong, and the correction is worth keeping.** It said the per-occurrence
   > versus per-slot choice was *"a decision this milestone deliberately does not make"*. Two
   > records had already asserted an answer — `DATA-MODEL.md` §7 and ADR 0029 §2 both explain
   > the key as existing *"because a multi-period activity occupies several slots"* — so the
   > decision had been made and M6 simply had not looked. M7's review found it.
   >
   > The answer went the **other way** after all: **one row per occurrence**, carrying the start
   > slot, because rows 2…`d` are derived and storing them makes a corrupt timetable
   > representable. [ADR 0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
   > D1 is the record, and it keeps the key verbatim while declining its stated reason.
2. **`solve_run`**, with `seed`, the statistics, the termination and the status. ✅ **Built in
   M7** — `V17`, with the change-log watermark beside them so a stored run records which snapshot
   of the inputs it was built from.
3. **Persisted violations**, so a stored timetable's hard-violation report survives a restart
   without re-evaluating. ✅ **Built in M7**, with one thing this item did not anticipate: they
   hang off a `timetable_evaluation` rather than off the timetable, so *what the generator found*
   and *what is wrong now* are different rows and a revalidation cannot overwrite the first.
   [ADR 0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) C3.
4. **The unverified counts on the stored timetable**, per ADR 0032 rule 4. ✅ **Built in M7** —
   four `INTEGER NOT NULL` columns, named exactly as `UnverifiedCounts`' fields.
5. **Whatever `UnscheduledActivity` becomes when it is stored.** ✅ **Built in M7**, and the
   recommendation was followed: `solve_run_unscheduled` stores the **cause** and the pre-solve
   diagnostic code, and **not** the sentence. M7 added one thing this item did not: those rows are
   *historical and not derived*, so Phase 9 exports them — nothing can reconstruct which lessons a
   past run failed on once the inputs move.
6. **A decision about re-generation.** ✅ **Decided in M7: it versions.** A save always inserts,
   several timetables coexist for one `(cycle, term)`, and replacement is a save followed by an
   explicit delete. `evara generate` **stays read-only** — writing lives in `evara save`, so the
   guarantee is one word the user typed rather than a flag they have to notice.

   M7 also added a protection this item did not foresee: a save is **refused** if any scheduling
   input moved since the problem was compiled. Nothing referential breaks when that happens, so
   nothing else would catch it. [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) §4.

Unchanged by M6 and not to be relitigated: generation is scoped to one `(school, cycle, term)`;
a clash is overlapping minutes and never a `timeslot_id`; `activity-scheduled` is MEDIUM; the
constraint engine is the one authority on legality; and nothing in the solver reads a clock.
