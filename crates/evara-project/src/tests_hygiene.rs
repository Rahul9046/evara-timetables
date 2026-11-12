//! The properties that are about the repository and the API rather than about one archive.
//!
//! Three kinds of thing live here, and none of them is about a particular export:
//!
//! - **T-61** — no binary project or archive fixture is checked in. Every one is generated.
//! - **T-62** — no driver type crosses this crate's public boundary.
//! - **T-56** — the container's portability rules, asserted against a real archive: three
//!   fixed ASCII names, fixed timestamps, LF-only JSON.

use std::io::Read;

use evara_core::scheduling::NeverCancel;
use evara_db::fixtures::transfer::Shape;
use zip::ZipArchive;

use crate::entry::{CHECKSUMS_ENTRY, MANIFEST_ENTRY, PAYLOAD_ENTRY, REQUIRED_ENTRIES};
use crate::tests_fixture::Fixture;
use crate::{IgnoreProgress, seal};

/// The repository root, from this crate's manifest directory.
fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the repository root is two levels up from this crate")
}

// T-61
#[test]
fn no_binary_project_or_archive_fixture_is_checked_in() {
    // A checked-in `.evaraproj` or `.evara` would be a binary blob nobody can review, and —
    // worse — one that could carry a real school's data past `CLAUDE.md` rule 3 without
    // anybody noticing. Every fixture in this workspace is generated deterministically, which
    // is what makes that rule enforceable rather than aspirational.
    //
    // `target/` is excluded because it holds whatever the tests just wrote.
    let root = repository_root();
    let mut found = Vec::new();
    walk(&root, &mut found);

    assert!(
        found.is_empty(),
        "no `.evaraproj` or `.evara` file may be checked in; every fixture is generated. \
         Found: {found:?}"
    );
}

/// Collects every `.evaraproj` or `.evara` outside `target/` and `.git/`.
fn walk(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // `node_modules` as well as the two obvious ones: it holds thousands of files and
            // none of them is ours, so walking it would make this test slow for nothing.
            if matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                continue;
            }
            walk(&path, found);
        } else if path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case(evara_db::PROJECT_EXTENSION)
                || extension.eq_ignore_ascii_case(crate::ARCHIVE_EXTENSION)
        }) {
            found.push(path);
        }
    }
}

// T-62
#[test]
fn no_driver_type_crosses_the_public_boundary() {
    // `rusqlite` is asserted by **absence**: it is not a dependency of this crate in any
    // form, direct or dev, so there is no line to remove and a test could not name one of its
    // types even to try. `CLAUDE.md`'s rule is that no crate but `evara-db` depends on the
    // SQLite driver, even for tests.
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("read this crate's manifest");

    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        assert!(
            !line.starts_with("rusqlite"),
            "this crate must not depend on the SQLite driver, even for tests: `{line}`"
        );
    }

    // `zip` **is** a dependency, because this crate is the one that reads and writes the
    // container. What the rule forbids is a `zip` type crossing the *public* boundary, and
    // `TransferError` is where that would show. Asserted by matching every variant a `zip`
    // failure can become, which compiles only while each one carries plain data.
    let refusals = [
        crate::TransferError::NotAnArchive {
            name: String::from("x.evara"),
        },
        crate::TransferError::UnsupportedCompression {
            entry: String::from("payload/project.evaraproj"),
        },
        crate::TransferError::EntryEncrypted {
            entry: String::from("payload/project.evaraproj"),
        },
        crate::TransferError::EntryNotAFile {
            entry: String::from("payload/"),
        },
        crate::TransferError::DuplicateEntryName {
            entry: String::from("manifest.json"),
        },
        crate::TransferError::Truncated,
    ];
    for refusal in &refusals {
        // Rendering each one proves it is a sentence rather than a wrapped driver error: a
        // variant carrying a `ZipError` would have had to name the type to be constructed
        // here, and this file cannot.
        let message = refusal.to_string();
        assert!(
            !message.is_empty() && !message.contains("ZipError"),
            "a refusal must be a sentence, not a driver error: {message}"
        );
        assert!(
            refusal.left_everything_alone(),
            "and every reader refusal leaves the user's files alone: {message}"
        );
    }
}

// T-56
#[test]
fn the_container_is_portable_by_construction() {
    let mut fixture = Fixture::build(Shape::Varied);
    let source = fixture.project.path().to_path_buf();
    fixture.project.close();

    let dir = tempfile::TempDir::new().expect("temp dir");
    let archive_path = dir.path().join("northgate.evara");
    let db = evara_db::Database::open_for_inspection(&source).expect("open for inspection");
    let snapshot = db
        .snapshot_into(
            &dir.path().join("staging"),
            &NeverCancel,
            &mut evara_db::transfer::ignore_progress,
        )
        .expect("snapshot");
    drop(db);
    seal(snapshot, &archive_path, &IgnoreProgress, &NeverCancel).expect("seal");

    let mut archive =
        ZipArchive::new(std::fs::File::open(&archive_path).expect("open")).expect("read");

    assert_eq!(
        archive.len(),
        REQUIRED_ENTRIES.len(),
        "`formatVersion 1` writes exactly three entries"
    );

    let names: Vec<String> = archive
        .file_names()
        .map(|name| name.expect("utf-8").into_owned())
        .collect();
    for (at, expected) in REQUIRED_ENTRIES.iter().enumerate() {
        assert_eq!(
            names.get(at).map(String::as_str),
            Some(*expected),
            "the entries are written in a fixed order, and the manifest is first so a reader \
             meets it at the start of the file"
        );
    }
    for name in &names {
        assert!(
            name.is_ascii() && name.to_lowercase() == *name,
            "every entry name is lowercase ASCII, so no platform's case folding or Unicode \
             normalisation can change it: {name}"
        );
        assert!(
            !name.starts_with('/') && !name.contains('\\'),
            "and relative, `/`-separated: {name}"
        );
    }

    // Fixed timestamps. Without these two exports of an unchanged project would differ in
    // every local header, which is what T-05 would then be unable to assert.
    for at in 0..archive.len() {
        let entry = archive.by_index_data(at).expect("entry metadata");
        let stamp = entry.last_modified();
        assert_eq!(
            stamp.map(|stamp| (stamp.year(), stamp.month(), stamp.day())),
            Some((1980, 1, 1)),
            "every entry timestamp is the ZIP epoch, not the clock"
        );
    }

    // The manifest is `Stored`, so it is readable with no decompressor: it is parsed before
    // anything else is trusted, and a reader should not have to inflate anything to find out
    // whether it is looking at an Evara archive.
    let manifest_entry = archive
        .by_index_data(0)
        .expect("the manifest is the first entry");
    assert_eq!(
        manifest_entry.compression(),
        zip::CompressionMethod::Stored,
        "the manifest is stored uncompressed"
    );

    // LF only, in both JSON entries. A CRLF would make the digest platform-dependent and let
    // git rewrite the bytes under us.
    for name in [MANIFEST_ENTRY, CHECKSUMS_ENTRY] {
        let mut bytes = Vec::new();
        archive
            .by_name(name)
            .expect("entry")
            .read_to_end(&mut bytes)
            .expect("read");
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "{name} must be LF-only so its digest is the same on Windows and macOS"
        );
        assert_eq!(bytes.last(), Some(&b'\n'), "{name} ends in a newline");
    }

    // And the payload is a SQLite database, which is byte-identical across platforms: the
    // file format is defined big-endian on disk, so portability is a property of the payload
    // rather than something the container has to arrange.
    let mut header = [0_u8; 16];
    archive
        .by_name(PAYLOAD_ENTRY)
        .expect("payload")
        .read_exact(&mut header)
        .expect("read the header");
    assert_eq!(
        &header,
        evara_db::transfer::SQLITE_HEADER,
        "the payload is a SQLite database"
    );
}

#[test]
fn the_two_extensions_stay_different_kinds_of_object() {
    // `CLAUDE.md` §3: `.evaraproj` is the live project, `.evara` is the portable export,
    // never one where the other belongs and never a third.
    assert_ne!(crate::ARCHIVE_EXTENSION, evara_db::PROJECT_EXTENSION);
    assert_eq!(crate::ARCHIVE_EXTENSION, "evara");
    assert_eq!(evara_db::PROJECT_EXTENSION, "evaraproj");

    // An archive is not a destination, and a project file is not an archive. Both directions,
    // because a reader of one constant should not have to check the other.
    assert!(
        crate::import::require_project_extension(std::path::Path::new("x.evara")).is_err(),
        "a `.evara` is not somewhere an import may put a project"
    );
    assert!(
        crate::writer::require_archive_extension(std::path::Path::new("x.evaraproj")).is_err(),
        "a `.evaraproj` is not somewhere an export may put an archive"
    );
}
