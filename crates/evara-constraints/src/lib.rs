//! The constraint registry: evaluators, parameter schemas and explanations.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and nothing else. It **must not** touch SQL or the
//! filesystem: a constraint is evaluated against an in-memory problem, never a database.
//!
//! This crate is consumed by **both** [`evara_solver`] and the manual-move validator in
//! the desktop application, which is the whole reason it exists as a separate crate.
//! There is exactly one implementation of every scheduling rule, shared by:
//!
//! 1. automatic timetable generation
//! 2. timetable scoring
//! 3. manual move validation
//! 4. violation explanation
//!
//! A rule that exists in the UI, or twice anywhere, is an architecture bug.
//!
//! # What lives here (added in Phase 4 and Phase 5)
//!
//! - The `ConstraintEvaluator` trait: `evaluate`, `affected`, `explain`, `param_schema`
//! - The registry mapping a stable `type_key` to an evaluator
//! - One module per constraint family, each independently testable
