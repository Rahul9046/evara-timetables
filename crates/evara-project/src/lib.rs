//! The portable `*.evara` project format: export, import and format migrations.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and [`evara_db`]. Owns the portable `*.evara` format and nothing
//! else.
//!
//! This is **project transfer** — a versioned snapshot of Evara's own model, moved between
//! installations. It is deliberately *not* the place for reading other vendors' formats; that
//! is `evara-integrations`. Conflating the two is the mistake this split exists to prevent.
//!
//! **No driver type crosses this crate's boundary.** No `zip` type and no `rusqlite` type
//! appears in any public signature or in [`TransferError`], asserted the way `evara-db`
//! asserts the same rule — by not having the dev-dependency, so a test could not name one.
//!
//! # The two objects, and they are never confused
//!
//! | Extension | What it is | Where it lives |
//! | --- | --- | --- |
//! | `.evaraproj` | the **live project**: a SQLite database Evara opens, migrates and writes | `evara_db::PROJECT_EXTENSION` |
//! | `.evara` | the **portable export**: a ZIP carrying a verified snapshot of one | [`ARCHIVE_EXTENSION`] |
//!
//! `CLAUDE.md` §3 forbids using one where the other belongs and forbids a third.
//!
//! # What an archive is
//!
//! ```text
//! Northgate.evara                        ZIP
//! ├── manifest.json                      STORED — what the archive says about itself
//! ├── checksums.json                     DEFLATE — one digest per covered entry
//! └── payload/project.evaraproj          DEFLATE — a verified SQLite snapshot
//! ```
//!
//! Three entries, exactly, in `formatVersion 1`. The payload is a **snapshot** rather than a
//! directory of JSON sections, which supersedes ADR 0004's container decision: completeness is
//! then structural rather than vigilant, the migration chain is the one already shipped, and
//! `integrity_check` plus `foreign_key_check` are a stronger verification than "the JSON
//! parsed" (ADR 0053, D1).
//!
//! # Corruption is detected; authenticity is not
//!
//! The manifest is the **deliberately unverified root** of a two-level digest tree. These
//! digests detect truncation, bit rot, a bad copy, a partially written archive and a
//! substituted payload. They do **not** establish authenticity: anyone can rewrite the
//! manifest and recompute every digest below it, and there is no signature because a signature
//! needs key distribution and ADR 0001 forbids the service that would provide it.
//!
//! **A `.evara` carries exactly the authority of the person who sent it.** ADR 0059, §6.2,
//! `OPEN-DECISIONS.md` R11, and the import screen says so too.
//!
//! # The three things to read before changing anything here
//!
//! - [`entry`] — why nothing is ever extracted to a path taken from the archive, which is how
//!   the whole traversal class is made unexpressible rather than defended against (ADR 0055).
//! - [`reader`] — the verification order, where each step gates the next and the two cheapest
//!   refusals sit before the first byte is written.
//! - [`import`] — the replacement protocol, and the three safeguards that were conditions of
//!   the design's approval. **Nothing in Evara deletes the project it replaced.**
//!
//! # Status — M10 Stage 2
//!
//! The format, the writer, the reader, the staging lifecycle, the replacement protocol and its
//! recovery. **No CLI command, no Tauri command and no screen** — those are Stages 3, 4 and 5,
//! and this crate's API is what they call.

pub mod checksums;
pub mod entry;
pub mod error;
pub mod import;
pub mod limits;
pub mod manifest;
pub mod migrate;
pub mod paths;
pub mod progress;
pub mod reader;
pub mod recovery;
pub mod staging;
pub mod writer;

pub use crate::error::{Recovered, Result, TransferError};
pub use crate::import::{CountDisagreement, ImportReport, Importer, OnConflict, import};
pub use crate::manifest::{FORMAT, FORMAT_VERSION, Manifest};
pub use crate::progress::{IgnoreProgress, Phase, TransferProgress};
pub use crate::reader::{ArchiveSummary, Readability, inspect};
pub use crate::recovery::{abandoned_staging, recover_pending};
pub use crate::staging::ReplacementJournal;
pub use crate::writer::{ARCHIVE_EXTENSION, ExportReport, seal};

#[cfg(test)]
mod tests_fixture;

#[cfg(test)]
mod tests_round_trip;

#[cfg(test)]
mod tests_refusal;

#[cfg(test)]
mod tests_replacement;

#[cfg(test)]
mod tests_measurements;

#[cfg(test)]
mod tests_hygiene;

#[cfg(test)]
mod tests_historical;
