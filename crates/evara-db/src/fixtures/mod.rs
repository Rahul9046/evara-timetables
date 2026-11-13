//! Synthetic projects on disk, for the tests that need a real `.evaraproj` to read.
//!
//! # Why a feature, and why off by default
//!
//! `evara-core` reached this point in M2 and answered it the same way: a fixture generator
//! is test material, and a shipped binary should not carry it. Off by default, so
//! `cargo build` compiles none of this and `evara-cli`, the desktop binary and the solver
//! never link it.
//!
//! It became necessary in **M10 Stage 2**. Until then every project fixture was
//! `#[cfg(test)]` inside this crate, which is the right place for the loader's own tests and
//! the wrong place for a transfer: an export and a re-import live in `evara-project`, and
//! there was nothing in a `.evaraproj` for them to carry. Stage 1 deferred the feature here
//! *with its only consumer*, rather than adding a feature nothing used.
//!
//! # The one dependency it adds
//!
//! `tempfile`, which was already a dev-dependency and is already in the lockfile. A fixture
//! owns the directory its project lives in, so the project file outlives every borrow and
//! disappears with the test; threading a caller-supplied directory through every one of the
//! twelve builders instead would have changed 500 lines of loader tests to avoid a
//! dependency that is not shipped.
//!
//! # What is here
//!
//! - [`seed`] — the builder: a [`Project`](seed::Project) wrapping a real workspace, and a
//!   [`School`](seed::School) that writes a school through the ordinary repositories. No raw
//!   SQL, no bypassed trigger, one documented exception
//!   ([`School::orphan_availability_rule`](seed::School::orphan_availability_rule)).
//! - [`catalogue`] — the twelve named schools M4's loader is tested against, which are also
//!   M10's round-trip corpus.
//! - [`transfer`] — one project at §7.3's realistic upper end, with the edit history T-60
//!   and T-83 could not be measured without.
//!
//! # Deterministic, and synthetic
//!
//! No clock is read and no randomness is used: every name, size and time is a constant or a
//! function of an index, so two runs build identical schools. Identifiers differ between
//! runs — they are UUIDv7 and the repositories mint them — which is why every assertion
//! against a fixture is written about order, counts and resolved facts rather than about a
//! particular value.
//!
//! Every name is invented, per `CLAUDE.md` rule 3. No real school, teacher, parent or
//! student.

pub mod catalogue;
pub mod historical;
pub mod seed;
pub mod transfer;
