# Solver Architecture

How timetable generation, scoring, validation and repair work in a local-only desktop
application.

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
`evara-db`'s loader. The inner loop touches only integers and bitsets.

```rust
pub struct Problem {
    pub timeslots:  Vec<TimeslotFact>,     // cycle_day, period, ordinal, campus
    pub rooms:      Vec<RoomFact>,         // capacity, room_type, campus
    pub teachers:   Vec<TeacherFact>,
    pub groups:     Vec<GroupFact>,        // expanded to leaf student sets
    pub activities: Vec<ActivityFact>,     // duration, teachers, groups, requirements
    pub unavailable: UnavailabilityIndex,  // bitset per (entity, timeslot)
    pub constraints: Vec<CompiledConstraint>,
    pub pinned:     Vec<Option<Placement>>, // locked activities
}

pub struct Assignment {                    // the solution
    pub placements: Vec<Option<Placement>>, // indexed by activity index
}
pub struct Placement { pub timeslot: u32, pub room: Option<u32> }
```

Two planning variables per activity — **timeslot** and **room** — matching the prototype,
plus a third later for teacher selection when a class has interchangeable staff
(see [OPEN-DECISIONS.md](OPEN-DECISIONS.md) D7).

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

---

## 5. Algorithm

Phased, in the usual construct-then-improve shape.

**Phase A — Construction.** Greedy insertion, hardest first. Activities are ordered by
descending *constrainedness*: pinned, then long duration, then few feasible
(timeslot, room) pairs, then high conflict degree (shares teachers/groups with many
others). Each is placed in its cheapest feasible slot. Locked activities are placed
first and never moved. Fast, deterministic, and produces a feasible or near-feasible
starting point for most schools.

**Phase B — Local search.** Late Acceptance Hill Climbing. Chosen over simulated
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

**Phase C — Polish.** Soft-only hill climb with hard constraints frozen, so the final
run cannot trade feasibility for preference.

### Termination and determinism

Termination on any of: time limit, unimproved-step limit, score target, or user cancel.
The RNG is explicitly seeded and stored on `solve_run.seed`; the same seed, problem and
config must reproduce the same timetable byte-for-byte. This is non-negotiable — it is
what makes solver regressions debuggable and benchmark baselines meaningful.

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

**Layer 1 — Pre-solve diagnostics (cheap, runs before every solve).** Counting arguments
that catch most genuinely impossible setups, with actionable messages:

- teacher load vs. available teaching slots, per teacher and per term
- room-type demand vs. supply, per `(room_type, timeslot)`
- group period budget vs. slots available to that group
- activities whose candidate (timeslot, room) set is empty — often a single bad
  availability entry
- lines whose members exceed the number of rooms of the required type
- pinned activities that already conflict with each other

Example output: *"Year 9 needs 42 periods per cycle but only 40 teaching slots exist.
Reduce a course's periods per cycle, or add a period."* This layer is high value for low
effort and should ship in Milestone 1.

**Layer 2 — Post-solve hard-violation report.** When the best solution is infeasible,
return the hard violations with full entity references and let the user click through to
the exact clash. Also Milestone 1.

**Layer 3 — Relaxation search (later).** Iteratively soften one constraint group at a
time, re-solve with a short budget, and report which relaxations restore feasibility:
*"Feasible if Ms Patel's Thursday unavailability is lifted, or if Lab 2's capacity
requirement is dropped."* This is the "suggest fixes" requirement. It needs a fast solver
first, so it is explicitly post-M1.

---

## 9. Reversibility

Every commitment above is behind the `SolverEngine` trait and a JSON-serialisable
`Problem`/`Assignment` pair. Consequences:

- `evara-cli solve fixtures/<name>` runs the engine headless for CI, benchmarking and
  regression baselines.
- A different engine — including CP-SAT as an optional exact mode for a hard
  sub-problem such as elective-line construction, or Timefold if it ever becomes the
  right call — can be added without touching the database, the UI or the constraint
  definitions.
- `evara-solver` has no Tauri dependency, so it can also compile to `wasm32` if a
  browser-based viewer is ever wanted.

What is *not* reversible cheaply, and so should be challenged now rather than later: the
three-level integer score, two planning variables per activity, and dense index addressing
in `Problem`. These are load-bearing.
