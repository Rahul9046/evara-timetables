//! Domain types, identifiers, the cycle/calendar time model and scoring primitives.
//!
//! # Layering contract
//!
//! `evara-core` is the bottom of the stack. It **must not**:
//!
//! - perform any I/O (no files, no sockets, no database)
//! - depend on any other `evara-*` crate
//! - mention a vendor's name or a vendor-specific concept
//!
//! Everything here is a plain data type or a pure function, so it is trivially
//! testable and can compile to any target including `wasm32`.
//!
//! # What lives here (added in later phases)
//!
//! - Identifier newtypes over UUIDv7
//! - The structure model: school, campus, term, cycle, cycle day, period, timeslot
//! - People, curriculum and activity types
//! - `Score` (hard/medium/soft, lexicographic) and `Violation`
//! - `Problem` / `Assignment` / `Placement`, the solver's in-memory contract
//!
//! Phase 1E adds the first of them: [`time::CycleCoverage`], the rule that decides
//! whether a repeating cycle is finished. It is here rather than in `evara-db` or the
//! interface because it is a question about the model, and a scheduling rule in the view
//! layer is an architecture bug.

pub mod time;
