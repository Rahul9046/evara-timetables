//! Timetable generation: construction heuristic, local search and scorers.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and [`evara_constraints`]. It **must not** open a database
//! or read a file: it receives a fully materialised problem in memory and returns a
//! solution. It also carries no Tauri dependency, so it stays usable from the headless
//! CLI and could target `wasm32`.
//!
//! Solving is deterministic given a seed. The same problem, configuration and seed must
//! reproduce the same timetable, which is what makes regressions debuggable and
//! benchmark baselines meaningful.
//!
//! # What lives here (added in Phase 6)
//!
//! - The `SolverEngine` trait, so an alternative engine (for example CP-SAT as an
//!   optional exact mode) can be introduced without redesigning the application
//! - A construction heuristic placing the most constrained activities first
//! - Late Acceptance Hill Climbing over a set of undoable moves
//! - `ReferenceScorer` and `IncrementalScorer`, kept honest by a property test
