# Solver Architecture

How timetable generation, scoring, validation and repair work in a local-only desktop
application.

> **This file is the design. The shipped engine is
> [TIMETABLE-CONSTRUCTION-SOLVER.md](TIMETABLE-CONSTRUCTION-SOLVER.md).** M6 built §5's Phase A
> and §4's incremental scorer; §5's Phases B and C, §6's `validate_move` and §7's repair are not
> built. Where this file and that one disagree about a detail, that one is the record of what the
> code does, and its §16 lists every departure with its reason.

---

## 1. The question the desktop requirement forces

The previous direction (branch `feat/v0.1-solver-service`) was **Java 21 + Spring Boot +
Timefold Solver behind a REST API**. That is a good architecture for a server product and
a poor one for this product. Evaluated honestly:

| Criterion | Timefold sidecar (JVM) | Rust, in-process | TypeScript, Web Worker | OR-Tools CP-SAT sidecar |
| --- | --- | --- | --- | --- |
| Extra runtime to ship | Bundled JRE, ~45–70 MB jlink'd, per platform | None — already in the Tauri binary | None | ~40–90 MB native lib, C++ build chain |
| Offline guarantee | Yes, but a localhost HTTP server and a port | Absolute: a function call | Absolute | Local process + IPC |
| Packaging on macOS | JRE must be signed and notarised too; largest source of release pain | Single signed binary | Single binary | Hard: notarising a vendored C++ lib |
| Startup latency | 1–3 s JVM boot before the first solve | None | None | ~0.2 s process spawn |
| Shares code with the move validator | No — rules would exist twice | **Yes** | Yes | No |
| Incremental / interactive repair | Good (Timefold supports it) | We implement it | We implement it | Poor fit — CP-SAT is batch-oriented |
| Per-constraint explanation | Good (score explanation API) | We implement it | We implement it | Weak — proof-less infeasibility |
| Raw optimisation strength | Very strong, mature | Good if written carefully | Adequate, 5–20x slower | Strongest for pure feasibility |
| Licence | Community Edition Apache-2.0, but **multi-threaded solving is Enterprise-only and commercial** | Ours, Apache-2.0 | Ours | Apache-2.0 |
| Languages to maintain | 3 (TS, Rust, Java) | 2 (TS, Rust) | 1.5 | 3 |

Two findings decide it.

**First, the validator forces the issue.** Milestone 1 requires "validate manual moves
immediately" and "explain why a move is invalid". If the solver lives in a JVM, either
every drag-and-drop crosses an HTTP boundary into a second runtime, or the rules get
reimplemented for the UI — and then the generator and the editor disagree, which is the
worst possible bug in a timetabling product. The constraint code must be callable
cheaply and synchronously from the same process as the editor.

**Second, Tauri already requires Rust.** Putting the engine in Rust adds no toolchain,
no bundled runtime, no port, and no second notarisation target. The JVM route adds all
four for a capability we need anyway.

### Decision

**Write the solver in Rust, in-process, as `crates/evara-solver`.** Timefold is not
adopted. Its constraint semantics from the prototype are kept as the specification for
the Rust ports (`CORE-001..CORE-012` in [CONSTRAINTS.md](CONSTRAINTS.md)).

The engine sits behind a trait so this is reversible, and so an *optional* exact engine
can be added later for hard sub-problems (see §9).

---

## 2. Boundary

`evara-solver` receives a fully materialised, in-memory problem and returns a solution.
It never opens a database, never does I/O, and is deterministic given a seed.

```rust
pub trait SolverEngine {
    fn solve(
        &self,
        problem: &Problem,
        start: Option<&Assignment>,   // Some(..) = repair an existing timetable
        config: &SolveConfig,
        progress: &dyn ProgressSink,  // best-score updates, step counts
        cancel: &CancelToken,
    ) -> Result<SolveOutcome, SolveError>;
}
```

`Problem` is the compiled form of the database: entities flattened into `Vec`s and
addressed by dense `u32` indexes, not UUIDs. UUID ↔ index translation happens once, in
`evara-db`'s loader — **implemented in M4** as `evara_db::scheduling`, which also keeps the
reverse maps a diagnostic and the solution writer need
([SCHEDULING-PROBLEM-ASSEMBLY.md](SCHEDULING-PROBLEM-ASSEMBLY.md)). The inner loop touches
only integers and bitsets.

```rust
// Implemented in M2 as `evara_core::scheduling`. Abridged: every field carries doc comments
// naming the record that fixes its meaning.
pub struct Problem {
    pub context:     SchedulingContext,    // school, cycle, term, campuses, bell schedules
    pub timeslots:   Vec<TimeslotFact>,    // cycle_day, period, bell schedule, campus,
                                           //   is_teaching, and its wall-clock interval
    pub room_types:  Vec<RoomTypeFact>,
    pub rooms:       Vec<RoomFact>,        // capacity, room_type, campuses, is_bookable
    pub teachers:    Vec<TeacherFact>,
    pub groups:      Vec<GroupFact>,       // the inputs to overlap, NOT expanded student
                                           //   sets — see below
    pub sections:    Vec<SectionFact>,     // staffing, audience + masks, room requirement,
                                           //   resolved bell schedule, expected attendance
    pub activities:  Vec<ActivityFact>,    // one placeable occurrence each: duration,
                                           //   split_index, and its section
    pub overlap:     OverlapRelation,      // three-state, per pair of group indices
    pub unavailable: UnavailabilityIndex,  // bitset per (entity, timeslot)
    pub pinned:      Vec<Option<Placement>>, // locked activities; nothing writes it yet
}

pub struct Assignment { /* one Option<Placement> per activity index */ }
pub struct Placement { pub timeslot: TimeslotIx, pub room: Option<RoomIx> }
```

`CompiledConstraint` is **not** in the implemented `Problem`: the registry is M5's and a
field for it would be a promise this milestone cannot keep. It joins the struct with the
evaluators that read it.

The indices are newtypes (`TimeslotIx`, `RoomIx`, `ActivityIx`, …) over `u32` rather than bare
`u32`s. The shape is unchanged and the runtime cost is nil; what it buys is that passing a
room index where a timeslot index belongs becomes a type error rather than a plausible wrong
answer — the worst failure mode in a product whose value is that its answers are right.

A section exists between an activity and its facts because four occurrences of one section
are taught by the same people to the same groups in the same kind of room and resolve to the
same bell schedule. Holding that once rather than four times is the argument
[adr/0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) §5 used to reject
`activity_teacher` and `activity_group`.

Two planning variables per activity — **timeslot** and **room** — matching the prototype,
plus a third later for teacher selection when a class has interchangeable staff
(see [OPEN-DECISIONS.md](OPEN-DECISIONS.md) D7).

**Three things M1's decisions fix about this shape.** The types are implemented (M2) and the
loader that fills them in is implemented (M4); nothing evaluates or searches yet. Two fields
are deliberately still empty after a load: `overlap` is `Indeterminate` for every pair, because
ADR 0035 §3's evaluator is M5's, and `pinned` is empty, because `activity.is_locked` does not
exist.

- **An activity's value range is one bell schedule's slots, resolved before the solve.** The
  grid of a `(cycle, term)` may hold slots from several `period_structure` rows, and
  [adr/0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) decides
  which one an activity draws from — by campus compatibility, refusing an ambiguous case
  rather than guessing. `TimeslotFact` therefore carries its structure, and its
  `starts_at` / `ends_at`, because the clash key for a shared room or teacher is a wall-clock
  interval and not a `timeslot_id`
  ([adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) rule 2).
- **`groups` is not expanded to leaf student sets**, which is what this section said before
  and which [adr/0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) §5
  corrects. [ADR 0006](adr/0006-group-based-scheduling-first.md) requires a school with **no
  `student` rows at all** to get a full timetable, and such a school's leaf sets are all
  empty — so an expansion would make every pair compare disjoint and `student-clash` silent
  for precisely the schools group-first scheduling was chosen for. The compiled form is a
  three-state overlap relation over group indices, held as bitsets, derived from member rows
  where they exist and from declarations where they do not.
- **`RoomFact.capacity` is carried and read by no first-set constraint.**
  [adr/0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
  defers `room-capacity` because no stored field supplies its left-hand side, and makes
  expected attendance an explicit two-state value rather than a number that defaults to zero.

The whole set is summarised in
[SCHEDULING-ARCHITECTURE-DECISIONS.md](SCHEDULING-ARCHITECTURE-DECISIONS.md), and
[SCHEDULING-FIXTURES.md](SCHEDULING-FIXTURES.md) records how M2 turned it into types and the
fixtures each later milestone is tested against.

---

## 3. Score

Three levels, compared **lexicographically**: hard dominates medium dominates soft.

```rust
pub struct Score { pub hard: i64, pub medium: i64, pub soft: i64 }
impl Score { pub fn is_feasible(&self) -> bool { self.hard == 0 } }
```

Penalties are negative-going integers (never floats) so comparison and equality are
exact and reproducible across platforms. A constraint's `weight` (a float in the
database) is multiplied into an integer penalty at **compile** time, not at scoring time.

`MEDIUM` is not decoration. It is the level for "must be satisfied unless the alternative
is an infeasible timetable" — unscheduled activities, unmet period counts, unstaffed
classes. It lets the solver return a *usable partial* timetable and tell the user exactly
what it could not place, instead of returning nothing.

---

## 4. Constraints: a registry, not a function

The explicit requirement is no single giant scheduling function. Each constraint family
is an independent unit, individually testable.

> **Implemented in M5**, with four signature changes from the sketch below, each because a
> type that did not exist when it was written now does.
> [CONSTRAINT-ENGINE.md](CONSTRAINT-ENGINE.md) is the shipped engine; this section is the
> design it was built from.
>
> - `fn constraint_type(&self) -> ConstraintType` rather than `type_key(&self) -> &str`. The
>   key is still the stored identity and still spelled out in a `match`; dispatch on a
>   `&'static str` would have made a typo a silent no-op.
> - `evaluate` takes an `&Occupancy` rather than an `&Assignment`. Resolving which minutes
>   each occurrence occupies is the same work for five of the eight rules, so it happens once
>   and no two rules can disagree about a span. §5 of that file.
> - `affected(…, changed: ActivityIx)` rather than `changed: u32`, for the reason M2 made the
>   indices newtypes: mixing two kinds of index would be a plausible wrong answer rather than
>   a crash.
> - `explain(&self, p, v: &Violation)` — no `&Assignment` and no `&CompiledConstraint`. A
>   `Violation` already carries the occurrences, the entities, the day, the span and the
>   penalty, so an explanation that needed the assignment back would be re-deriving what the
>   violation was built from.

```rust
pub trait ConstraintEvaluator: Send + Sync {
    fn type_key(&self) -> &'static str;
    fn param_schema(&self) -> &'static str;          // JSON Schema, drives the UI form

    /// Full evaluation. The reference implementation — always correct, never fast.
    fn evaluate(&self, p: &Problem, a: &Assignment, c: &CompiledConstraint,
                out: &mut ViolationSink);

    /// Which activities this constraint's score depends on, given a changed activity.
    /// Used to scope incremental re-evaluation.
    fn affected(&self, p: &Problem, c: &CompiledConstraint, changed: u32) -> AffectedSet;

    /// Human-readable reason a specific placement breaks this constraint.
    /// Powers "why is this move invalid?".
    fn explain(&self, p: &Problem, a: &Assignment, c: &CompiledConstraint,
               v: &Violation) -> Explanation;
}
```

Registration is a table in `evara-constraints/src/registry.rs` mapping `type_key` to an
evaluator. Adding a constraint means: one module, one registry line, one test file,
one row in `CONSTRAINTS.md`. Nothing else changes.

### Incremental scoring, safely

Full re-evaluation of every constraint after every candidate move is too slow for local
search. But hand-written delta scoring is the single most bug-prone part of any solver,
and a wrong delta silently produces a wrong timetable.

The mitigation is structural: **two scorers and differential testing.**

1. `ReferenceScorer` — calls `evaluate` on everything. Simple, obviously correct.
2. `IncrementalScorer` — maintains per-constraint indexes (occupancy counts per
   `(teacher, timeslot)`, per `(room, timeslot)`, per `(group, timeslot)`; per-day
   counts per `(group, subject, day)`) and recomputes only `affected()`.

A property test applies thousands of random moves to random synthetic problems and
asserts `IncrementalScorer == ReferenceScorer` after every single one. The incremental
path is only trusted because the cheap path keeps proving it right. This test runs in CI
and is the gate on any new constraint.

> **M5 shipped the reference half; M6 shipped the other.** `evaluate` is `ReferenceScorer`;
> `evaluate_placement` runs only the rules whose `affected()` set contains the occurrence that
> moved and then evaluates each **in full**, holding no cache and no index — incremental-*friendly*,
> not incremental.
>
> **`IncrementalScorer` and the property test both landed in M6**, which is the gate this
> paragraph set and `OPEN-DECISIONS.md` R1 stated as a shipping rule. Two things about it differ
> from the sketch above, and both are
> [ADR 0041](adr/0041-the-incremental-scorer-lives-with-the-evaluators.md):
>
> - **It lives in `evara-constraints`**, beside the evaluators it must agree with, not in
>   `evara-solver`. It needs `Occupancy`'s span resolver and each rule's own narrowed query, and
>   reaching those from another crate would mean either a second copy of the predicates — the
>   failure `CLAUDE.md` §2 exists to prevent — or a public API wide enough to make that copy easy
>   for somebody else to write later.
> - **There is no hand-written delta**, which is what makes the warning above not apply to it.
>   Every violation the eight rules produce names at most two occurrences, so
>   `score(after) − score(before) = involving(after, a) − involving(before, a)` is an identity;
>   the scorer applies a tentative placement, asks each rule what it now finds about that
>   occurrence, and puts the previous state back. `evaluate_involving` is a sixth trait member and
>   shares its body with each rule's full sweep, so no predicate is stated twice.
>
> The property test is `evara-constraints`' `differential` module: the narrowed query against the
> full sweep per rule and per fixture, a mutated occupancy against a rebuilt one, and the running
> score against the reference score **after every single operation** of seeded random sequences.

---

## 5. Algorithm

Phased, in the usual construct-then-improve shape.

**Phase A — Construction.** ✅ **Built in M6.** Greedy insertion, hardest first. Activities are
ordered by descending *constrainedness*: pinned, then long duration, then few feasible
(timeslot, room) pairs, then high conflict degree (shares teachers/groups with many
others). Each is placed in its cheapest feasible slot. Locked activities are placed
first and never moved. Fast, deterministic, and produces a feasible or near-feasible
starting point for most schools.

Three notes on what shipped, and
[TIMETABLE-CONSTRUCTION-SOLVER.md](TIMETABLE-CONSTRUCTION-SOLVER.md) §3 is the detail.

The order is computed **once**, from the empty timetable, and the occurrences that fail are
retried in later passes until a pass places nothing. A dynamic re-rank — always take the
occurrence with the fewest *currently* feasible placements — is a better heuristic and costs
`O(occurrences × candidates)` per step, which is hundreds of millions of constraint queries for a
500-occurrence school. The retry pass recovers most of it; in practice the first pass places
almost everything, which is the evidence.

*"Its cheapest feasible slot"* degenerates while no soft constraint exists: every feasible
candidate for one occurrence has an identical delta, so the first in a **seeded candidate order**
wins and the order is where the spreading comes from. The minimisation arrives with the first
soft rule and needs no change to the walk.

Locks are honoured and nothing populates them: there is no `activity.is_locked` column at `V16`,
so the pinned branch is unreachable today and is written because a pin honoured by accident later
is a pin not honoured.

**Phase B — Local search.** **Not built.**
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 keeps local search out of the MVP, so M6 constructs and
does not improve. Late Acceptance Hill Climbing. Chosen over simulated
annealing because it has effectively one parameter (the late-acceptance list length)
instead of a cooling schedule that needs tuning per school size, and over plain hill
climbing because it escapes local optima. Tabu search can be added behind the same
interface later.

Move types, each a small struct with `do`/`undo`:

| Move | Effect |
| --- | --- |
| `ChangeTimeslot` | one activity to another slot |
| `ChangeRoom` | one activity to another room |
| `SwapTimeslots` | exchange two activities' slots |
| `SwapRooms` | exchange two activities' rooms |
| `LineMove` | move every activity in a line/block together |
| `ChainMove` | 3-cycle rotation, for tightly packed timetables where no pairwise swap is legal |
| `Ruin&Recreate` | unassign a cluster (a teacher's week, a year level) and rebuild it |

Moves that break a *structural* invariant (a pinned activity, a non-candidate room, a
hard unavailability) are never generated — filtering is cheaper than scoring and
rejecting.

**Phase C — Polish.** **Not built, and could not be:** no soft constraint is implemented, so
there is nothing to polish against. Soft-only hill climb with hard constraints frozen, so the
final run cannot trade feasibility for preference.

### Termination and determinism

Termination on any of: time limit, unimproved-step limit, score target, or user cancel.
The RNG is explicitly seeded and stored on `solve_run.seed`; the same seed, problem and
config must reproduce the same timetable byte-for-byte. This is non-negotiable — it is
what makes solver regressions debuggable and benchmark baselines meaningful.

> **M6 implements this with one change: every limit is a count, never a duration.** Candidate
> evaluations, construction steps and retry passes. A time limit reads a clock and stops in a
> different place on a different machine, so the same seed would produce different timetables and
> a committed baseline would be meaningless.
>
> A wall clock is still supported and it is the **caller's**: the engine asks a `Cancel` trait
> between steps and the caller decides what the answer depends on. That is also why
> `Termination::Cancelled` is the one termination this workspace documents as not reproducible.
> Determinism is asserted as whole-outcome equality, which is possible only because
> `SolveStatistics` carries counters and deliberately no elapsed time.

### Threading

One solve runs on a dedicated OS thread, so the UI never blocks. Progress is streamed to
the frontend as Tauri events (`solver://progress`) carrying best score, step count and
elapsed time; cancellation flips an `AtomicBool` checked each step. Parallelism comes
later as **multi-start**: N independent seeded solvers across cores, best result wins —
embarrassingly parallel, no shared mutable state, and it does not need the kind of
multi-threaded incremental solving that Timefold charges for.

---

## 6. Manual moves and immediate validation

The UI never decides legality. It asks:

```rust
#[tauri::command]
fn validate_move(timetable_id: Uuid, m: ProposedMove) -> MoveVerdict
```

```rust
pub struct MoveVerdict {
    pub allowed: bool,                 // false iff it adds a hard violation
    pub score_delta: Score,
    pub blocking: Vec<Explanation>,    // why not
    pub warnings: Vec<Explanation>,    // medium/soft it would worsen
    pub suggestions: Vec<Placement>,   // nearby legal slots
}
```

Implementation is the same constraint registry: apply the move to a scratch assignment,
evaluate only `affected()` constraints, collect violations, `undo`. Sub-millisecond for
realistic schools, so the grid can validate on *hover* and tint legal drop targets before
the user commits — not only after.

An `Explanation` is structured, not a string:

```rust
pub struct Explanation {
    pub constraint_type: String,
    pub level: ConstraintLevel,
    pub message: String,                 // "J. Smith already teaches 9SCI-B in Mon P3"
    pub entities: Vec<EntityRef>,        // clickable in the UI
    pub conflicting_entries: Vec<Uuid>,  // highlight them in the grid
}
```

The frontend renders and localises; it does not author the reason.

---

## 7. Repair and partial regeneration

Both are the same mechanism: solve with `start = Some(current)` plus two extras.

1. **A scope filter.** Only activities in the selected scope (a term, a year level, a
   teacher's load, an explicit selection) are movable; everything else is treated as
   pinned for this run. That *is* partial regeneration.
2. **A disruption penalty.** A built-in `MEDIUM`-level constraint (`OPS-001`,
   `minimise-disruption`) penalises every activity whose placement differs from the
   baseline, weighted higher for already-published timetables. The solver then prefers
   the smallest edit that fixes the problem rather than a different-but-equally-good
   timetable, which is what makes repair acceptable to a school mid-term.

Disruption weight is user-facing: "keep the current timetable as much as possible" ↔
"find the best timetable".

---

## 8. Explaining infeasibility

A bare "no solution found" is the failure mode this product exists to avoid. Three
layers, in increasing cost:

**Layer 1 — Pre-solve diagnostics (cheap, runs before every solve).** **Implemented in M3**
as `evara_core::scheduling::validate`, a pure function from a `Problem` to a
`DiagnosticReport`; see [PRE-SOLVE-VALIDATION.md](PRE-SOLVE-VALIDATION.md). Counting arguments
that catch most genuinely impossible setups, with actionable messages:

- teacher load vs. available teaching slots, per teacher and per term
- room-type demand vs. supply, per `(room_type, timeslot)`
- group period budget vs. slots available to that group
- activities whose candidate (timeslot, room) set is empty — often a single bad
  availability entry
- lines whose members exceed the number of rooms of the required type — **deferred**, there
  is no `line` table
- pinned activities that already conflict with each other — **deferred**, there is no
  `activity.is_locked` column

Four of those six are implemented. Each carries its demand/capacity inequality and the
direction it is sound in: demand is exact or under-estimated, capacity exact or
over-estimated, so a finding proves impossibility and silence proves nothing. The two
deferred checks were not stubbed, because their input vocabulary does not exist.

Example output: *"Year 9 needs 42 periods per cycle but only 40 teaching slots exist.
Reduce a course's periods per cycle, or add a period."* This layer is high value for low
effort and shipped in M3.

M1's decisions add two more kinds of output to this layer, and the distinction between them
matters as much as the checks themselves:

- **Hard validation errors that refuse the solve**, because the remedy is a field the user
  can fill in and without it there is no value range at all: no grid materialised, an
  activity that resolves to no bell schedule, and an activity that resolves to more than one
  ([adr/0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) §3).
- **Explicit warnings that do not block**, because the remedy is a setup task or does not
  exist: group pairs whose overlap is `Indeterminate`
  ([adr/0035](adr/0035-student-group-overlap-is-a-three-state-answer.md) §6), sections whose
  expected attendance is `Unknown`
  ([adr/0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) §5),
  and entities shared with another school
  ([adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md) rule 3).

Every warning is counted and carried onto the generated timetable, so a timetable is
described as having no *confirmed* clash within the school solved — never as conflict-free.
[SCHEDULING-ARCHITECTURE-DECISIONS.md](SCHEDULING-ARCHITECTURE-DECISIONS.md) §3.2 classifies
every incomplete input into one of four outcomes.

**Layer 2 — Post-solve hard-violation report.** When the best solution is infeasible,
return the hard violations with full entity references and let the user click through to
the exact clash. **The shape arrived with M6** — `SolveOutcome::hard_violations` and the
structured `Violation` behind it — and the condition it reports is **unreachable from M6's
engine**: a construction that refuses every illegal placement never ends with a hard violation,
so it leaves an occurrence unplaced instead. The list is non-empty only when the reference
scorer disagrees with the engine, which is a defect and is reported as one. Phase B's local
search is what will reach the condition honestly.

M6 adds the half a greedy constructor does need, which Layer 2 did not cover: a **per-occurrence
cause** for every lesson it could not place, separating *"no timetable can place this"* from
*"this arrangement did not"*. TIMETABLE-CONSTRUCTION-SOLVER.md §11.

**Layer 3 — Relaxation search (later).** Iteratively soften one constraint group at a
time, re-solve with a short budget, and report which relaxations restore feasibility:
*"Feasible if Ms Patel's Thursday unavailability is lifted, or if Lab 2's capacity
requirement is dropped."* This is the "suggest fixes" requirement. It needs a fast solver
first, so it is explicitly post-M1.

---

## 9. Reversibility

Every commitment above is behind the `SolverEngine` trait and a JSON-serialisable
`Problem`/`Assignment` pair. Consequences:

- `evara generate <fixture>` runs the engine headless for CI, benchmarking and regression
  baselines — shipped in M6. The command is spelled `generate`, with `solve` kept as an accepted
  alias so the spelling this file used still works; TIMETABLE-CONSTRUCTION-SOLVER.md §16. Since
  M7, `evara save` is the one that writes a timetable into a project, and `generate` still writes
  nothing — [TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) §10.
- A different engine — including CP-SAT as an optional exact mode for a hard
  sub-problem such as elective-line construction, or Timefold if it ever becomes the
  right call — can be added without touching the database, the UI or the constraint
  definitions.
- `evara-solver` has no Tauri dependency, so it can also compile to `wasm32` if a
  browser-based viewer is ever wanted.

What is *not* reversible cheaply, and so should be challenged now rather than later: the
three-level integer score, two planning variables per activity, and dense index addressing
in `Problem`. These are load-bearing.
