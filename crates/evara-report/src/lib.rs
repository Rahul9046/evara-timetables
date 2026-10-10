//! Timetable view projections and output renderers (HTML, CSV, PDF).
//!
//! # Layering contract
//!
//! Depends on [`evara_core`]. Receives data; does not query for it.
//!
//! Renderers sit behind a trait so additional output formats drop in without touching
//! the projections that feed them.
//!
//! # What lives here (added in Phase 7 and Phase 10)
//!
//! - View projections: master, by teacher, by class/group, by room, by student
//! - Renderers: print-oriented HTML, CSV, and later a vector PDF backend
