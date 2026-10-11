//! A school's own logo, stored in the project file.
//!
//! `evara_core::branding` owns what counts as an acceptable image and tests that on its
//! own. This file owns what only a real project file can show:
//!
//! - absence is the ordinary state, and it is what "use the Evara logo" looks like
//! - the bytes survive a close and reopen unchanged, because they are *in* the project
//! - replacing keeps the row's identity and advances its revision
//! - removing restores absence, and removing nothing is not an error
//! - two project files never see each other's logo
//! - a row the schema would not allow cannot be written, even behind the repository
//!
//! Real temporary project files throughout, never `:memory:`.

use evara_core::branding::{LogoFormat, MAX_BYTES, MIN_EDGE};

use crate::tests_time_model::Project;
use crate::{DbError, Structure};

/// A PNG header with the given dimensions, padded to `size` bytes.
///
/// Only the header is real, which is all `LogoImage::inspect` reads. The padding exists so
/// a test can describe a file of a particular size without carrying one.
fn png(width: u32, height: u32, size: usize) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes.resize(size.max(bytes.len()), 0x5A);
    bytes
}

/// A JPEG header with the given dimensions.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    bytes
}

/// The school this project describes.
fn school_id(project: &mut Project) -> uuid::Uuid {
    project
        .db()
        .structure()
        .school()
        .expect("read school")
        .expect("a school exists")
        .id
}

// ============================================================== absence is the default

#[test]
fn a_new_project_has_no_logo() {
    let mut project = Project::new();
    // Not an error, not an empty row: nothing at all. This is what the interface reads as
    // "show the Evara logo".
    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn a_school_can_be_created_and_edited_without_ever_touching_a_logo() {
    // A logo is optional and must never be in the way of completing setup.
    let mut project = Project::new();
    let school = school_id(&mut project);
    assert_ne!(school, uuid::Uuid::nil());
    assert_eq!(project.db().structure().school_logo().expect("read"), None);

    // And the school's own record is untouched by the logo table existing.
    let before = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");
    assert_eq!(before.rev, 1);
}

// ======================================================================= storing one

#[test]
fn a_valid_image_is_stored_byte_for_byte() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let bytes = png(256, 256, 4096);

    let stored = project
        .db()
        .structure()
        .set_school_logo(school, &bytes)
        .expect("set");

    assert_eq!(stored.media_type, "image/png");
    assert_eq!((stored.width, stored.height), (256, 256));
    assert_eq!(stored.rev, 1);
    // Unaltered. Re-encoding would need a decoder and would mean the file a user gets back
    // out of an export is not the one they put in.
    assert_eq!(stored.bytes, bytes);
}

#[test]
fn the_format_is_taken_from_the_content() {
    let mut project = Project::new();
    let school = school_id(&mut project);

    // Nothing in the signature carries a filename, so there is no extension to disagree
    // with. The stored type is whatever the header says.
    let stored = project
        .db()
        .structure()
        .set_school_logo(school, &jpeg(300, 200))
        .expect("set");
    assert_eq!(stored.media_type, "image/jpeg");
    assert_eq!(Structure::logo_format(&stored), Some(LogoFormat::Jpeg));
    assert_eq!((stored.width, stored.height), (300, 200));
}

#[test]
fn an_unsupported_file_is_refused_and_nothing_is_stored() {
    let mut project = Project::new();
    let school = school_id(&mut project);

    for bytes in [
        b"GIF89a................".to_vec(),
        b"%PDF-1.7................".to_vec(),
        b"MZ\x90\x00\x03\x00\x00\x00".to_vec(),
        Vec::new(),
    ] {
        let outcome = project.db().structure().set_school_logo(school, &bytes);
        assert!(
            matches!(outcome, Err(DbError::Invalid { .. })),
            "{:?} was accepted",
            &bytes[..8.min(bytes.len())]
        );
    }

    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn an_oversized_file_is_refused_and_nothing_is_stored() {
    let mut project = Project::new();
    let school = school_id(&mut project);

    let outcome = project
        .db()
        .structure()
        .set_school_logo(school, &png(64, 64, MAX_BYTES + 1));

    match outcome {
        Err(DbError::Invalid { ref message }) => {
            assert!(
                message.contains("limit"),
                "the refusal must name the limit: {message}"
            );
        }
        other => panic!("expected an Invalid refusal, got {other:?}"),
    }
    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn an_image_with_impossible_dimensions_is_refused() {
    let mut project = Project::new();
    let school = school_id(&mut project);

    for bytes in [png(4, 4, 256), png(9000, 9000, 256)] {
        assert!(
            matches!(
                project.db().structure().set_school_logo(school, &bytes),
                Err(DbError::Invalid { .. })
            ),
            "out-of-range dimensions were accepted"
        );
    }
    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn a_refused_upload_leaves_an_existing_logo_alone() {
    // The important half of refusal: a bad second attempt must not destroy a good first.
    let mut project = Project::new();
    let school = school_id(&mut project);
    let good = png(128, 128, 1024);
    let before = project
        .db()
        .structure()
        .set_school_logo(school, &good)
        .expect("set");

    assert!(
        project
            .db()
            .structure()
            .set_school_logo(school, b"not an image at all")
            .is_err()
    );

    let after = project
        .db()
        .structure()
        .school_logo()
        .expect("read")
        .expect("still there");
    assert_eq!(after, before);
    assert_eq!(after.bytes, good);
}

// ========================================================================= replacing

#[test]
fn replacing_keeps_the_row_and_advances_its_revision() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let first = project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("first");

    let second = project
        .db()
        .structure()
        .set_school_logo(school, &jpeg(640, 480))
        .expect("second");

    // Same logo, changed. An update rather than a delete and re-insert, so `rev` tells the
    // truth about how many times it has changed.
    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.rev, first.rev + 1);
    assert_eq!(second.media_type, "image/jpeg");
    assert_eq!((second.width, second.height), (640, 480));

    // And exactly one row, whatever happens.
    assert_eq!(
        project
            .db()
            .structure()
            .school_logo()
            .expect("read")
            .expect("exists"),
        second
    );
}

#[test]
fn a_logo_can_be_replaced_many_times() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let mut last_id = None;

    for edge in [MIN_EDGE, 64, 128, 512, 2048] {
        let stored = project
            .db()
            .structure()
            .set_school_logo(school, &png(edge, edge, 512))
            .expect("set");
        if let Some(id) = last_id {
            assert_eq!(stored.id, id, "the logo changed identity on replacement");
        }
        last_id = Some(stored.id);
        assert_eq!(stored.width, i64::from(edge));
    }
}

// ========================================================================== removing

#[test]
fn removing_a_logo_restores_absence() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("set");

    project.db().structure().clear_school_logo().expect("clear");

    // Absence, which is what the interface reads as "use the Evara logo".
    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn removing_a_logo_that_is_not_there_succeeds() {
    // The state asked for is already true. Reporting NotFound would make the interface
    // guard a button for no reason.
    let mut project = Project::new();
    project.db().structure().clear_school_logo().expect("first");
    project
        .db()
        .structure()
        .clear_school_logo()
        .expect("second");
    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn a_logo_can_be_set_again_after_being_removed() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let first = project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("set");
    project.db().structure().clear_school_logo().expect("clear");

    let second = project
        .db()
        .structure()
        .set_school_logo(school, &png(256, 256, 1024))
        .expect("set again");

    // A new row, so a new identity and a revision starting over. That is honest: the old
    // logo was deleted, and the change feed recorded the delete.
    assert_ne!(second.id, first.id);
    assert_eq!(second.rev, 1);
    assert_eq!(second.width, 256);
}

#[test]
fn removing_a_logo_leaves_the_school_untouched() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let before = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");

    project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("set");
    project.db().structure().clear_school_logo().expect("clear");

    let after = project
        .db()
        .structure()
        .school()
        .expect("read")
        .expect("exists");
    // A logo is not part of the school record: neither setting nor removing one consumes
    // the school's revision or rewrites its row.
    assert_eq!(after, before);
}

// ======================================================================= persistence

#[test]
fn a_logo_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let bytes = png(300, 300, 8192);
    let stored = project
        .db()
        .structure()
        .set_school_logo(school, &bytes)
        .expect("set");

    project.reopen();

    let after = project
        .db()
        .structure()
        .school_logo()
        .expect("read")
        .expect("still there");
    // The whole point of storing it in the project rather than beside it.
    assert_eq!(after, stored);
    assert_eq!(after.bytes, bytes);
}

#[test]
fn removal_survives_closing_and_reopening_the_project() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("set");
    project.db().structure().clear_school_logo().expect("clear");

    project.reopen();

    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn two_projects_never_see_each_other_s_logo() {
    // A logo lives inside one project file. Two schools in two projects are two unrelated
    // things, and neither can read or disturb the other.
    let mut first = Project::new();
    let mut second = Project::new();
    let first_school = school_id(&mut first);

    first
        .db()
        .structure()
        .set_school_logo(first_school, &png(128, 128, 1024))
        .expect("set");

    assert!(
        first
            .db()
            .structure()
            .school_logo()
            .expect("read")
            .is_some()
    );
    assert_eq!(second.db().structure().school_logo().expect("read"), None);

    // And the second project can have its own, different, logo.
    let second_school = school_id(&mut second);
    second
        .db()
        .structure()
        .set_school_logo(second_school, &jpeg(640, 480))
        .expect("set");

    let a = first
        .db()
        .structure()
        .school_logo()
        .expect("read")
        .expect("exists");
    let b = second
        .db()
        .structure()
        .school_logo()
        .expect("read")
        .expect("exists");
    assert_ne!(a.id, b.id);
    assert_eq!(a.media_type, "image/png");
    assert_eq!(b.media_type, "image/jpeg");
}

// ============================================== the schema as a backstop, and damage

#[test]
fn the_schema_refuses_a_row_the_module_would_have_refused() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    let school_text = school.to_string();

    // Behind the repository, which is the only way to attempt these. The CHECKs restate
    // evara_core::branding's limits so they hold against a hand-repaired file or a future
    // build with a bug, not merely against the path the interface takes.
    for (label, media_type, width, height) in [
        ("an unsupported media type", "image/gif", 64, 64),
        ("a width past the limit", "image/png", 4096, 64),
        ("a height below the limit", "image/png", 64, 2),
    ] {
        let outcome = project.db().write(|tx| {
            tx.execute(
                "INSERT INTO school_logo
                     (id, school_id, media_type, bytes, width, height, created_at, updated_at, rev)
                 VALUES ('0192f0a0-0000-7000-8000-00000000beef', ?, ?, X'89504E47', ?, ?,
                         '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
                rusqlite::params![&school_text, media_type, width, height],
            )?;
            Ok(())
        });
        assert!(outcome.is_err(), "{label} was accepted by the schema");
    }

    assert_eq!(project.db().structure().school_logo().expect("read"), None);
}

#[test]
fn the_schema_refuses_a_second_logo_for_the_same_school() {
    let mut project = Project::new();
    let school = school_id(&mut project);
    project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 1024))
        .expect("set");

    let school_text = school.to_string();
    let outcome = project.db().write(|tx| {
        tx.execute(
            "INSERT INTO school_logo
                 (id, school_id, media_type, bytes, width, height, created_at, updated_at, rev)
             VALUES ('0192f0a0-0000-7000-8000-00000000cafe', ?, 'image/png', X'89504E47', 64, 64,
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 1)",
            rusqlite::params![&school_text],
        )?;
        Ok(())
    });

    // UNIQUE (school_id). There is no way to end up with two and have to choose.
    assert!(outcome.is_err(), "a second logo was accepted");
}

#[test]
fn a_media_type_this_build_cannot_name_is_reported_as_unreadable() {
    // Not reachable through the schema today, which is the point: this is the decoder
    // being asked about a row a *future* build might have written. It must say "I cannot
    // read this" so the caller falls back to the Evara logo, rather than guessing a format
    // and handing a viewer bytes it will not render.
    let mut project = Project::new();
    let school = school_id(&mut project);
    let logo = project
        .db()
        .structure()
        .set_school_logo(school, &png(64, 64, 512))
        .expect("set");

    assert_eq!(Structure::logo_format(&logo), Some(LogoFormat::Png));

    let future = crate::SchoolLogo {
        media_type: "image/avif".to_owned(),
        ..logo
    };
    assert_eq!(Structure::logo_format(&future), None);
}

#[test]
fn the_change_feed_records_every_logo_write() {
    // school_logo is in TRACKED_TABLES, so setting, replacing and removing a logo are all
    // real edits in the change feed — which is what keeps a future synchronisation able to
    // merge them.
    let mut project = Project::new();
    let school = school_id(&mut project);

    project
        .db()
        .structure()
        .set_school_logo(school, &png(64, 64, 512))
        .expect("set");
    project
        .db()
        .structure()
        .set_school_logo(school, &png(128, 128, 512))
        .expect("replace");
    project.db().structure().clear_school_logo().expect("clear");

    let ops = project
        .db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT op FROM change_log WHERE entity_table = 'school_logo' ORDER BY seq",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
        .expect("read change log");

    assert_eq!(ops, vec!["INSERT", "UPDATE", "DELETE"]);
}
