//! Adapters for external school and timetabling systems. Empty by design.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and **nothing else**. Adapters produce core types; the core
//! never references a vendor.
//!
//! Intentionally empty. Adapters for PowerSchool, Veracross, aSc, Timetabling Solutions,
//! Edval, TimeTabler, CSV and LISS arrive one at a time after Milestone 1, each behind
//! its own cargo feature and backed by its own synthetic fixtures.
//!
//! Keeping this crate's dependency list to a single internal crate is what guarantees an
//! integration cannot leak a vendor concept into the domain, the database or the
//! solver.
