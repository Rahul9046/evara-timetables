//! The portable `*.evara` project format: export, import and format migrations.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and [`evara_db`]. Owns the portable `*.evara` format and
//! nothing else.
//!
//! This is **project transfer** — a versioned snapshot of Evara's own model, moved
//! between installations. It is deliberately *not* the place for reading other vendors'
//! formats; that is [`evara_integrations`]. Conflating the two is the mistake this split
//! exists to prevent.
//!
//! # What lives here (added in Phase 9)
//!
//! - ZIP container writer and reader, manifest and per-entry checksums
//! - The format migration chain, one pure function per `formatVersion` step
//! - Import into a fresh project database inside a single transaction
