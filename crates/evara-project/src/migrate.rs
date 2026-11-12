//! The container migration chain, which is empty at `formatVersion 1`.
//!
//! # Why an empty chain exists at all
//!
//! There is exactly one container version, so there is nothing to migrate and this module
//! could have been left out. It is here because the *shape* of the answer is the decision, and
//! deciding it now costs nothing while deciding it later costs a reader working out whether a
//! `formatVersion 1` archive was ever migrated.
//!
//! The shape: **one pure function per `formatVersion` step**, applied in order, each taking
//! the previous version's manifest and producing the next version's. No step reads a file, no
//! step touches the payload — the payload's own axis is `schemaVersion` and
//! `migrations::run` is its chain. Two axes, two chains, and they do not meet.
//!
//! # What bumps `formatVersion`, and what does not
//!
//! | Bumps it | Does not |
//! | --- | --- |
//! | renaming or removing an entry | adding an optional manifest field |
//! | changing the payload kind | adding a `capabilities` prefix |
//! | changing the meaning of a manifest field | adding an entry — unknown safe entries are ignored |
//! | changing the digest algorithm | anything inside the payload, which is `schemaVersion`'s axis |
//!
//! The third row of the left column is the one worth saying out loud: a field whose meaning
//! changed is indistinguishable from one that did not, so a reader with no version bump would
//! read the new meaning with the old code and be confidently wrong.

use crate::error::Result;
use crate::manifest::Manifest;

/// Brings a manifest up to [`FORMAT_VERSION`](crate::manifest::FORMAT_VERSION).
///
/// Returns whether anything was applied, so a report can say *"this archive was written in an
/// older format"* rather than leaving a user to infer it.
///
/// **The chain is empty.** There is one container version, so a manifest the reader accepted
/// is already current — anything above `FORMAT_VERSION` was refused as
/// [`FormatTooNew`](crate::TransferError::FormatTooNew) before this is reached, and there is
/// nothing below it. A second version adds one step and one line:
///
/// ```ignore
/// if manifest.format_version == 1 {
///     step_1_to_2(manifest)?;
/// }
/// ```
///
/// # Errors
///
/// None today, because there is no step that can fail. The signature is `Result` because a
/// fallible step is the ordinary case for a migration, and widening it later would be a
/// breaking change to every caller for no reason.
pub fn bring_forward(manifest: &mut Manifest) -> Result<bool> {
    // The precondition the reader has already established, asserted rather than assumed: a
    // manifest from the future must never reach a migration chain, because a chain that met
    // one would have no step for it and would report "nothing to do" about an archive it
    // cannot read.
    debug_assert!(
        manifest.format_is_readable(),
        "a container version newer than this build reads is refused by the reader, not \
         migrated"
    );

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::bring_forward;
    use crate::manifest::FORMAT_VERSION;

    #[test]
    fn a_current_archive_is_not_migrated() {
        let mut manifest = sample();
        assert!(!bring_forward(&mut manifest).expect("no step can fail"));
        assert_eq!(manifest.format_version, FORMAT_VERSION);
    }

    fn sample() -> crate::manifest::Manifest {
        crate::manifest::Manifest {
            format: String::from(crate::manifest::FORMAT),
            format_version: FORMAT_VERSION,
            payload: crate::manifest::PayloadDescriptor {
                kind: String::from(crate::manifest::PAYLOAD_KIND),
                entry: String::from(crate::entry::PAYLOAD_ENTRY),
                schema_version: 18,
                byte_length: 1,
            },
            generator: crate::manifest::Generator {
                app: String::from("Evara Timetables"),
                version: String::from("0.1.0"),
            },
            project_id: uuid::Uuid::nil(),
            created_by_site_id: None,
            exported_by_site_id: None,
            exported_at: String::from("2026-11-12T00:00:00.000Z"),
            change_log: crate::manifest::ChangeLogSummary {
                entries: 0,
                watermark: 0,
            },
            schools: Vec::new(),
            counts: std::collections::BTreeMap::new(),
            capabilities: Vec::new(),
            integrity: crate::manifest::Integrity {
                algorithm: String::from(crate::manifest::DIGEST_ALGORITHM),
                entry: String::from(crate::entry::CHECKSUMS_ENTRY),
                sha256: String::from("ab"),
            },
        }
    }
}
