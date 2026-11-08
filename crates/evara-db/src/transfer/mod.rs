//! Project transfer: the snapshot, the untrusted-payload boundary, and replacement.
//!
//! # Status — Stage 1
//!
//! This module is M10's engine half. `evara-db` owns it because every operation it performs
//! is `pub(crate)` here: the connection factory, the migration runner, the change-log
//! installer, and the SQL that reads `sqlite_master`. The **format** half — the `.evara`
//! container, its manifest and its checksums — belongs to `evara-project` and does not
//! exist yet.
//!
//! What Stage 1 provides:
//!
//! - [`snapshot`] — [`Snapshot`] and `Database::snapshot_into`, a consistent, owned, `Send`
//!   copy of a project taken with the workspace lock held for as little as possible.
//! - [`boundary`] — the hardened read-only constructor a *staged, untrusted* payload is
//!   opened through, and the audit it is subjected to.
//! - [`conformance`] — a payload's schema compared against a reference generated from this
//!   binary's own migrations.
//! - [`replacement`] — the recovery journal's persistence, and the **pure** decision
//!   function that drives recovery from observed filesystem facts.
//! - [`sidecars`] — reducing a WAL database to a single file, so there is no set to move.
//!
//! What it deliberately does not provide: no ZIP, no manifest, no checksums, no CLI command,
//! no Tauri command, and **no orchestration of the replacement protocol itself**. Stage 1
//! builds the primitives the protocol is composed from and tests each one directly; Stage 2
//! composes them, from the proposal's §0.2 safeguards and never from D11's superseded text.

pub mod boundary;
pub mod conformance;
pub mod replacement;
pub mod sidecars;
pub mod snapshot;

pub use boundary::{PayloadAudit, UntrustedPayload};
pub use conformance::{SchemaDifference, SchemaReference};
pub use replacement::{
    Ambiguity, DestinationState, DestinationVerdict, DiskFacts, PendingReplacement, RecoveryAction,
    ReplacementPhase, classify_destination, reconcile,
};
pub use sidecars::{
    SIDECAR_SUFFIXES, SidecarState, checkpoint_to_single_file, refuse_if_sidecars_present,
    sidecar_path,
};
pub use snapshot::{Snapshot, SnapshotProgress, ignore_progress};

#[cfg(test)]
mod tests_boundary;

#[cfg(test)]
mod tests_engine;

#[cfg(test)]
mod tests_replacement;

#[cfg(test)]
mod tests_snapshot;
