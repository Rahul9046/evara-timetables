# ADR 0002 — Rust in-process solver; do not adopt Timefold or a solver service

- Status: **Accepted** (approved 2026-10-06)
- Date: 2026-10-06
- Supersedes: `services/solver/` and branch `feat/v0.1-solver-service`

## Context

The repository contains an unmerged prototype: Java 21 + Spring Boot + Timefold Solver
exposing `POST /api/timetables/solve`, with `@CrossOrigin(origins="*")`. It is a sound
server design. The product is a local-only desktop application, which changes the
evaluation entirely.

Milestone 1 also requires immediate validation of manual moves and a human-readable
explanation of why a move is invalid. That makes the constraint evaluator an
*interactive* component, not only a batch one.

The comparison table is in [SOLVER.md](../SOLVER.md) §1.

## Decision

Implement the solver in **Rust, in-process**, as `crates/evara-solver`, behind a
`SolverEngine` trait. Do not adopt Timefold. Do not ship a solver service or a sidecar
process.

The constraint evaluators live in `crates/evara-constraints` and are consumed by **all
four** callers, so there is exactly one implementation of every scheduling rule:

1. automatic timetable generation
2. timetable scoring
3. manual move validation
4. violation explanation

The `SolverEngine` abstraction is retained as a condition of approval, so an alternative
engine such as CP-SAT can be introduced later without redesigning the application.

Two findings drive this:

1. **The validator forces it.** With the engine in a JVM, every drag-and-drop either
   crosses an HTTP boundary into a second runtime, or the rules get reimplemented in
   TypeScript for the UI — at which point the generator and the editor can disagree. That
   is the worst defect class in a timetabling product. The existing repository already
   shows the drift starting: `packages/domain` has `teacherIds: ID[]` while the Java
   `Lesson` has a single `teacherId`.
2. **Rust is already required.** Tauri needs it. The engine in Rust adds no toolchain, no
   bundled runtime, no port and no second notarisation target. A JVM sidecar adds all
   four — for a capability we must build regardless.

Timefold's licensing reinforces it: the Community Edition is Apache-2.0, but
multi-threaded incremental solving is Enterprise-only and commercial, which is a poor fit
for an open-source project that will eventually want to use more than one core.

## Consequences

- We write metaheuristics instead of configuring a mature engine: construction heuristic,
  late-acceptance local search, move generation, and incremental scoring. This is the
  largest piece of net-new engineering in the plan.
- **Incremental scoring correctness becomes the top technical risk.** Mitigated
  structurally by shipping two scorers — a simple reference implementation and an indexed
  incremental one — and a CI property test asserting they agree after random move
  sequences ([SOLVER.md](../SOLVER.md) §4). Any new constraint must pass it.
- Optimisation quality will start below the incumbents'. Benchmark fixtures with committed
  score baselines make that measurable from Phase 6 rather than a matter of opinion.
- Gains: single signed binary, no JVM startup, sub-millisecond move validation, a headless
  `evara-cli` for CI and benchmarking, deterministic seeded runs, and one language fewer
  to maintain.
- Reversible: the `SolverEngine` trait plus a JSON-serialisable `Problem`/`Assignment`
  pair means an alternative engine — including OR-Tools CP-SAT as an optional exact mode
  for a hard sub-problem such as elective-line construction — can be added without
  touching the database, the UI or the constraint definitions.

## Migration of the existing prototype

1. Tag `feat/v0.1-solver-service` as `archive/timefold-prototype` so it stays recoverable,
   then stop merging toward it.
2. Delete `services/solver/` from `main`.
3. Keep the prototype's constraint *semantics* as the specification for the Rust ports.
   `teacherConflict`, `studentGroupConflict`, `roomConflict`, `roomCapacity` and
   `spreadSubjects` are carried forward as `teacher-clash`, `student-clash`, `room-clash`,
   `room-capacity` and `spread-across-days` in [CONSTRAINTS.md](../CONSTRAINTS.md), with
   the `CORE-0xx` identifiers retained as cross-references.
4. Correct the prototype's shortcuts in the port: single `teacherId` becomes
   many-to-many, single `studentGroupId` becomes many-to-many, and a two-level
   `HardSoftScore` becomes three levels so unscheduled activities and unmet period counts
   have somewhere to live.

## Alternatives considered

**Timefold as a bundled sidecar.** Strongest optimisation and good explanation APIs, but a
45–70 MB jlink'd JRE per platform, JVM startup before the first solve, a second
notarisation target, duplicated rules for the validator, three languages, and Enterprise
licensing for multi-threading.

**TypeScript solver in a Web Worker.** Shares types with the UI and needs no new language,
but 5–20x slower, and it puts the single most performance-sensitive and correctness-
sensitive code in the least suitable runtime.

**OR-Tools CP-SAT.** The strongest pure-feasibility engine and Apache-2.0, but a C++
dependency that is painful to vendor and notarise, batch-oriented rather than interactive,
and weak at per-constraint explanation and minimal-disruption repair — which are explicit
product requirements. Retained as a possible optional engine behind the trait.
