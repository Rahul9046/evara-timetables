//! What an archive says about itself, and the two version axes it says it on.
//!
//! # The manifest is the deliberately unverified root
//!
//! It carries the digest of `checksums.json`, which carries the digest of the payload. One
//! root, one digest per fact, acyclic by construction — and the root itself is covered by
//! nothing, because **a self-describing archive cannot verify its own root without a key**
//! and ADR 0001 forbids the service that would distribute one.
//!
//! So, plainly, in the one place a reader of this code will look: these digests detect
//! **corruption** — truncation, bit rot, a bad copy, a partially written archive, a
//! substituted payload. They do **not** establish **authenticity**. Anyone can rewrite this
//! manifest and recompute every digest below it. A `.evara` carries exactly the authority of
//! the person who sent it. ADR 0059, §6.2, and `OPEN-DECISIONS.md` R11.
//!
//! # Two version axes, one refusal each
//!
//! | Axis | Means | Refusal |
//! | --- | --- | --- |
//! | [`Manifest::format_version`] | the **container**: entry layout, manifest schema, payload kind | newer than [`FORMAT_VERSION`] → refuse, naming the Evara version needed. Older → the container migration chain |
//! | [`PayloadDescriptor::schema_version`] | the **data**: which migration the payload was written at | newer than `supported_schema_version()` → `SchemaTooNew`. Older → migrate the staged copy |
//!
//! `formatVersion`'s *meaning* may never change, because every future reader parses it before
//! it knows how to read anything else. The payload's own `refinery_schema_history` is the
//! authority on the second; the manifest's copy exists to refuse early and cheaply, before a
//! payload byte is written, and a disagreement between the two is itself a refusal — it means
//! the archive was not assembled by Evara.
//!
//! # `payload.sha256` is deliberately absent
//!
//! It lives in `checksums.json` only, so there is one place for one fact. Revision 1 of the
//! design recorded it twice and made the two files verify each other, which is a cycle:
//! neither could be checked before the other, so in practice neither was.
//! `payload.byteLength` **stays**, because the payload read has to be bounded before any
//! digest is known.
//!
//! # Unknown fields are kept, not refused
//!
//! `serde` is left in its default tolerant mode for additions: a field a newer build writes
//! is ignored and the archive still reads. What is *not* tolerant is anything the reader acts
//! on — `format`, `formatVersion`, `payload.kind`, `integrity.algorithm` — each of which
//! admits exactly the values this build understands.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The `format` every Evara archive declares.
pub const FORMAT: &str = "evara-project";

/// The container version this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// The one payload kind `formatVersion 1` defines.
///
/// Declared rather than assumed, which preserves ADR 0004's storage-decoupling hedge as a
/// versioned fact: a future `formatVersion 2` can add `"json-sections"` and read both,
/// without this build having to guess what it is looking at.
pub const PAYLOAD_KIND: &str = "sqlite-snapshot";

/// The one digest algorithm, and it is not negotiable.
pub const DIGEST_ALGORITHM: &str = "sha256";

/// What the archive carries, and how to bound reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadDescriptor {
    /// What kind of payload it is. [`PAYLOAD_KIND`] in `formatVersion 1`.
    pub kind: String,
    /// Which entry holds it.
    pub entry: String,
    /// Which migration the payload stands at.
    pub schema_version: u32,
    /// How many bytes it is, uncompressed.
    ///
    /// The bound on the payload read, which is why it is in the manifest rather than only in
    /// the central directory: the directory is part of the archive and the manifest is what
    /// this build parsed first.
    pub byte_length: u64,
}

/// Which build wrote the archive. Diagnostics, and it decides nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    /// Application name.
    pub app: String,
    /// Application version.
    pub version: String,
}

/// What the change feed looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeLogSummary {
    /// How many rows the feed holds.
    pub entries: i64,
    /// Highest `seq`, which is what a stored `inputs_watermark` points at.
    pub watermark: i64,
}

/// One school, so a user can see what they are about to import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSummary {
    /// Stable school identifier.
    pub id: Uuid,
    /// The school's name.
    pub name: String,
    /// How many campuses it holds.
    pub campuses: i64,
}

/// The integrity root: where the digest record is, and its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integrity {
    /// The algorithm. Exactly [`DIGEST_ALGORITHM`]; anything else is a refusal.
    pub algorithm: String,
    /// Which entry holds the digest record.
    pub entry: String,
    /// Its digest, lowercase hex.
    pub sha256: String,
}

/// Everything an archive says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// [`FORMAT`]. Anything else is not an Evara archive.
    pub format: String,
    /// The container version.
    pub format_version: u32,
    /// The payload descriptor.
    pub payload: PayloadDescriptor,
    /// Which build wrote it.
    pub generator: Generator,
    /// The project's identity, **never regenerated** (ADR 0004, T-03/T-64).
    pub project_id: Uuid,
    /// The installation that created the project, if it was ever recorded.
    ///
    /// Distinct from [`Manifest::exported_by_site_id`] and it must stay distinct: the
    /// exporting installation does **not** become the creator, or a project's provenance
    /// would be rewritten by whoever last sent it on.
    pub created_by_site_id: Option<Uuid>,
    /// The installation that produced this archive.
    pub exported_by_site_id: Option<Uuid>,
    /// When, ISO-8601 UTC.
    ///
    /// One of the two fields that differ between two exports of an unchanged project, which
    /// is what T-05 asserts.
    pub exported_at: String,
    /// The change feed.
    pub change_log: ChangeLogSummary,
    /// Every school in the project.
    pub schools: Vec<SchoolSummary>,
    /// Row counts per table.
    ///
    /// **Advisory.** A disagreement with the payload is reported, not refused: the payload is
    /// the authority and a count is a convenience. The one exception is
    /// [`PayloadDescriptor::schema_version`], which *is* a refusal on disagreement.
    pub counts: std::collections::BTreeMap<String, i64>,
    /// What a reader must understand to render this project honestly.
    ///
    /// Derived from the data at export time, never hand-maintained — D3. An entry this build
    /// does not know is a refusal naming it, before any payload byte is written.
    pub capabilities: Vec<String>,
    /// The integrity root.
    pub integrity: Integrity,
}

impl Manifest {
    /// Whether the container version is one this build reads.
    #[must_use]
    pub const fn format_is_readable(&self) -> bool {
        self.format_version <= FORMAT_VERSION
    }

    /// Serialises to the exact bytes an archive carries.
    ///
    /// Pretty-printed with a trailing newline and **LF only**, so a digest is identical on
    /// Windows and macOS and git cannot rewrite it. `serde_json::to_string_pretty` emits `\n`
    /// and never `\r\n`, which is the property being relied on; the test below is what keeps
    /// that from being an assumption.
    ///
    /// # Errors
    ///
    /// Propagates a serialisation failure, which for this type means a `BTreeMap` key that
    /// is not a string — unreachable, and reported rather than unwrapped.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::{
        ChangeLogSummary, DIGEST_ALGORITHM, FORMAT, FORMAT_VERSION, Generator, Integrity, Manifest,
        PAYLOAD_KIND, PayloadDescriptor,
    };

    fn sample() -> Manifest {
        let mut counts = BTreeMap::new();
        counts.insert(String::from("school"), 2);
        Manifest {
            format: String::from(FORMAT),
            format_version: FORMAT_VERSION,
            payload: PayloadDescriptor {
                kind: String::from(PAYLOAD_KIND),
                entry: String::from(crate::entry::PAYLOAD_ENTRY),
                schema_version: 18,
                byte_length: 6_291_456,
            },
            generator: Generator {
                app: String::from("Evara Timetables"),
                version: String::from("0.1.0"),
            },
            project_id: Uuid::nil(),
            created_by_site_id: None,
            exported_by_site_id: None,
            exported_at: String::from("2026-11-12T09:14:22.113Z"),
            change_log: ChangeLogSummary {
                entries: 48_210,
                watermark: 48_210,
            },
            schools: Vec::new(),
            counts,
            capabilities: vec![String::from("constraint:room-clash")],
            integrity: Integrity {
                algorithm: String::from(DIGEST_ALGORITHM),
                entry: String::from(crate::entry::CHECKSUMS_ENTRY),
                sha256: String::from("4c7e"),
            },
        }
    }

    #[test]
    fn the_bytes_are_lf_only_and_end_in_a_newline() {
        let bytes = sample().to_bytes().expect("serialise");
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "a CRLF in the manifest would make its digest platform-dependent"
        );
        assert_eq!(bytes.last(), Some(&b'\n'));
    }

    #[test]
    fn json_is_camel_case_at_every_level() {
        let text = String::from_utf8(sample().to_bytes().expect("serialise")).expect("utf-8");
        for expected in [
            "\"formatVersion\"",
            "\"schemaVersion\"",
            "\"byteLength\"",
            "\"projectId\"",
            "\"createdBySiteId\"",
            "\"exportedBySiteId\"",
            "\"exportedAt\"",
            "\"changeLog\"",
        ] {
            assert!(text.contains(expected), "expected {expected} in {text}");
        }
        assert!(
            !text.contains("\"format_version\""),
            "snake_case must not reach the JSON: CLAUDE.md maps the two in one place"
        );
    }

    #[test]
    fn the_payload_digest_is_not_in_the_manifest() {
        // The acyclicity assertion's manifest half, as a property of the type rather than of
        // one instance: there is nowhere to put a payload digest here.
        let text = String::from_utf8(sample().to_bytes().expect("serialise")).expect("utf-8");
        let payload_section = text
            .split("\"payload\"")
            .nth(1)
            .expect("the manifest has a payload section");
        let payload_section = payload_section
            .split("\"generator\"")
            .next()
            .expect("and something after it");
        assert!(
            !payload_section.contains("sha256"),
            "the payload digest lives in checksums.json only -- one place for one fact"
        );
    }

    #[test]
    fn an_unknown_field_does_not_stop_it_parsing() {
        // Forward tolerance, tested rather than assumed.
        let text = r#"{
            "format": "evara-project",
            "formatVersion": 1,
            "somethingNewer": {"nested": true},
            "payload": {"kind": "sqlite-snapshot", "entry": "payload/project.evaraproj",
                        "schemaVersion": 18, "byteLength": 1},
            "generator": {"app": "Evara Timetables", "version": "0.2.0"},
            "projectId": "00000000-0000-0000-0000-000000000000",
            "createdBySiteId": null,
            "exportedBySiteId": null,
            "exportedAt": "2026-11-12T00:00:00.000Z",
            "changeLog": {"entries": 0, "watermark": 0},
            "schools": [],
            "counts": {},
            "capabilities": [],
            "integrity": {"algorithm": "sha256", "entry": "checksums.json", "sha256": "ab"}
        }"#;
        let manifest: Manifest = serde_json::from_str(text).expect("an unknown field is ignored");
        assert_eq!(manifest.format_version, 1);
        assert_eq!(manifest.generator.version, "0.2.0");
    }

    #[test]
    fn a_newer_container_version_is_not_readable() {
        let mut manifest = sample();
        assert!(manifest.format_is_readable());
        manifest.format_version = FORMAT_VERSION + 1;
        assert!(!manifest.format_is_readable());
    }
}
