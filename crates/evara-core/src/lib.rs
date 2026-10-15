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
//!
//! [`time::CycleRotation`] joins it with the other such rule: the order a cycle's positions
//! run in and where the rotation comes back round. Day 5 of a five-day cycle is followed by
//! Day 1, and "what follows the last day" is stated once, here, rather than worked out again
//! by whichever screen draws the sequence.
//!
//! [`branding::LogoImage`] decides whether an uploaded school logo is one the project
//! will carry, reading the bytes' own header rather than a filename or an extension.
//!
//! [`calendar::WorkingDays`] joins it: which weekdays a school normally teaches on. The
//! two modules are deliberately separate, because a cycle position and a weekday are
//! different kinds of thing and only one of them schedules anything — see the
//! [`calendar`] header.
//!
//! [`calendar::WeeklyPosition`] is the one place the two meet, and the header there sets out
//! why that is not a contradiction: a weekly cycle's positions are *named* by the calendar —
//! position 1 is Monday — while `ordinal` remains the whole of a day's scheduling identity.
//! All seven exist, always, so that one cycle definition can be shared by schools keeping
//! different weeks; [`calendar::WeeklyPosition::applies_to`] answers "and which of those does
//! this school teach on?" without storing the answer anywhere. See
//! [ADR 0017](../../../docs/adr/0017-weekly-and-custom-cycles.md).

//! [`lessons::LessonDemand`] is Stage C's addition, and the same kind of thing: how much of
//! a cycle one class section asks for, and whether the school's time model could hold one
//! occurrence of it at all. It is arithmetic over counts, so it belongs here rather than in
//! SQL or in a React component — and deliberately not in `evara-constraints`, which is
//! Phase 4's registry of timetable-scored evaluators. See that module's header.

pub mod branding;
pub mod calendar;
pub mod lessons;
pub mod time;
