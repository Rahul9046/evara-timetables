//! Export, import, and assert that nothing was lost.
//!
//! # What "nothing was lost" is asserted as
//!
//! Not "the project opens". A project that opens and is missing a term's worth of lessons
//! opens perfectly well, and that is the failure mode a transfer actually has — so every test
//! here compares **discovered** facts:
//!
//! - the **census**, read from `sqlite_master` rather than from a list, so a table a future
//!   migration adds is covered by discovery and not by somebody remembering to add it (T-01);
//! - the **schema**, compared against a reference this binary generates from its own
//!   migrations, which is the same comparison the import boundary makes (T-01);
//! - the **identity**, which is never regenerated (T-03, T-64);
//! - and, for the pieces where losing a row would be silent rather than loud, the rows
//!   themselves.
//!
//! # The round trip is a real one
//!
//! The source project is built through the ordinary repositories, closed, snapshotted,
//! sealed into a real `.evara`, read back through the real reader, staged through the real
//! boundary and renamed into place. No step is stubbed. That is what makes the assertions
//! about the product rather than about a test harness.

use evara_core::scheduling::NeverCancel;
use evara_db::fixtures::transfer::Shape;
use evara_db::transfer::{PayloadFeature, facts_of, table_census};

use crate::tests_fixture::{APP_VERSION, Fixture};
use crate::{IgnoreProgress, ImportReport, Importer, OnConflict, import, inspect, seal};

/// One complete round trip, with everything it produced.
struct RoundTrip {
    /// Where the fixture's temporary directory is, held so it outlives the test.
    _dir: tempfile::TempDir,
    /// Where the archive was written.
    archive: std::path::PathBuf,
    /// Where the imported project is.
    destination: std::path::PathBuf,
    /// What the export reported.
    export: crate::ExportReport,
    /// What the import reported.
    import: ImportReport,
    /// The source project's facts, read before the export.
    source: evara_db::transfer::ProjectFacts,
}

/// Builds a fixture, exports it, imports it, and hands back everything.
fn round_trip(shape: Shape) -> RoundTrip {
    let mut fixture = Fixture::build(shape);
    let source_path = fixture.project.path().to_path_buf();

    // Closed before the snapshot, which is the headless export's shape: nothing holds the
    // file, so the copy is of a checkpointed database and the test is not accidentally
    // proving something about a live WAL.
    fixture.project.close();
    let source = facts_of(&source_path).expect("read the source project's facts");

    let dir = tempfile::TempDir::new().expect("temp dir");
    let archive = dir.path().join("northgate.evara");
    let export = export_to(&source_path, &dir.path().join("staging"), &archive);

    let destination = dir.path().join("imported.evaraproj");
    let mut settings = evara_db::Settings::open_at(&dir.path().join("settings.sqlite"))
        .expect("settings database");
    let import_report = import(
        &archive,
        &destination,
        OnConflict::Refuse,
        Importer {
            app_version: APP_VERSION,
            site_id: uuid::Uuid::now_v7(),
        },
        &mut settings,
        &IgnoreProgress,
        &NeverCancel,
    )
    .expect("import the archive");

    RoundTrip {
        _dir: dir,
        archive,
        destination,
        export: export.expect("seal the archive"),
        import: import_report,
        source,
    }
}

/// The headless export path, end to end.
///
/// `open_for_inspection` rather than `open_existing`: a headless export must not migrate,
/// must not recreate a trigger and must not stamp `last_opened_with_app_version` on somebody's
/// live file. E-01 is why the snapshot on that handle cannot be a `VACUUM INTO` — it is
/// refused under `query_only` — and E-02 is why the stepped backup works there.
fn export_to(
    project: &std::path::Path,
    staging: &std::path::Path,
    archive: &std::path::Path,
) -> crate::Result<crate::ExportReport> {
    let db = evara_db::Database::open_for_inspection(project).expect("open for inspection");
    let snapshot = db
        .snapshot_into(
            staging,
            &NeverCancel,
            &mut evara_db::transfer::ignore_progress,
        )
        .expect("snapshot the project");
    // Dropped before the archive is written, so the inspection handle is not open across the
    // whole seal — the arrangement D8 asks for, where the lock covers the snapshot and
    // nothing more.
    drop(db);
    seal(snapshot, archive, &IgnoreProgress, &NeverCancel)
}

// T-01
#[test]
fn every_table_discovered_from_the_schema_has_the_same_row_count() {
    let trip = round_trip(Shape::Varied);
    let after = table_census(&trip.destination).expect("census the imported project");

    assert_eq!(
        after, trip.source.census,
        "a transfer must preserve every row of every table the schema declares; a future \
         table is covered by discovery rather than by memory"
    );
    // And the census is not vacuously equal because both are empty.
    assert!(
        after.values().sum::<i64>() > 100,
        "the fixture must actually hold something: {after:?}"
    );
}

// T-03, T-64
#[test]
fn the_project_identity_is_preserved_and_never_regenerated() {
    let trip = round_trip(Shape::Varied);
    let after = facts_of(&trip.destination).expect("read the imported project's facts");

    assert_eq!(
        after.identity.project_id, trip.source.identity.project_id,
        "a transferred project is the same project; regenerating the identity would destroy \
         round-trip comparison and any future synchronisation (ADR 0004)"
    );
    assert_eq!(
        after.identity.created_by_site_id, trip.source.identity.created_by_site_id,
        "the exporting installation must not become the creator"
    );
    assert_eq!(
        trip.import.project_id, trip.source.identity.project_id,
        "and the report says so too"
    );
}

// T-21
#[test]
fn the_change_feed_travels_complete_and_its_sequence_continues() {
    let trip = round_trip(Shape::Varied);
    let after = facts_of(&trip.destination).expect("read the imported project's facts");

    assert_eq!(
        after.change_log.entries, trip.source.change_log.entries,
        "a feed with a hole is worse than a full one or none, because `seq` continuity is \
         what makes it readable"
    );
    assert_eq!(
        after.change_log.watermark, trip.source.change_log.watermark,
        "every stored `inputs_watermark` points into this sequence; a watermark in the \
         future would make \"N changes since\" negative"
    );
    assert_eq!(
        trip.export.manifest.change_log.watermark, trip.source.change_log.watermark,
        "and the manifest describes the payload rather than the live project"
    );
}

// T-08, T-09, T-12, T-13, T-14, T-15, T-16, T-19
#[test]
fn every_table_the_record_names_as_travelling_arrives_populated() {
    let trip = round_trip(Shape::Varied);
    let after = facts_of(&trip.destination).expect("read the imported project's facts");

    // Named rather than summed, so a failure says *which* table came through empty. The
    // census equality above already proves the counts match; what this adds is that the
    // fixture exercised each one at all — a round trip over an empty table asserts nothing,
    // and these are the tables where losing a row would be silent rather than loud.
    for table in [
        // structure and sharing — ADR 0019's deliberate exception
        "school",
        "campus",
        "building",
        "building_campus",
        "room",
        "room_campus",
        "room_type",
        "school_logo",
        // time
        "academic_year",
        "term",
        "cycle",
        "cycle_day",
        "period_structure",
        "period",
        "timeslot",
        // people and curriculum
        "teacher",
        "teacher_school",
        "student",
        "student_school_enrolment",
        "student_group",
        "student_group_member",
        "department",
        "subject",
        "teacher_qualification",
        "teacher_department",
        "availability",
        // lessons
        "course",
        "class_section",
        "class_teacher",
        "class_group",
        "class_enrolment",
        "activity",
        // rules
        "constraint_instance",
        // timetables, runs and findings — decision 3, which corrects PROJECT-FORMAT §3
        "timetable",
        "timetable_entry",
        "solve_run",
        "timetable_evaluation",
        // identity and history
        "app_meta",
        "change_log",
    ] {
        assert!(
            after.count_of(table) > 0,
            "the fixture must populate `{table}` for the round trip to say anything about \
             it, and the import must carry it"
        );
    }
}

// T-18, T-15 — the feature evidence, which is also what the capabilities are derived from
#[test]
fn lineage_and_locks_survive_and_are_declared() {
    let trip = round_trip(Shape::Varied);
    let after = facts_of(&trip.destination).expect("read the imported project's facts");

    assert!(
        after.features.contains(&PayloadFeature::RepairLineage),
        "a `based_on_timetable_id` edge must survive: lineage is not derivable from \
         `created_at` once duplication and repair chains are involved"
    );
    assert!(
        after.features.contains(&PayloadFeature::PlacementLocks),
        "a lock is a property of a placement and travels with the version it is on"
    );
    assert!(
        after.features.contains(&PayloadFeature::EmbeddedLogo),
        "`school_logo.bytes` is a BLOB like any other column and travels in the snapshot"
    );
    assert_eq!(
        after.features, trip.source.features,
        "the feature evidence read from the imported project must be the evidence read from \
         the source: it is what the capabilities are derived from"
    );

    for capability in ["lineage:repair", "lock:placement", "logo:embedded"] {
        assert!(
            trip.export
                .manifest
                .capabilities
                .contains(&capability.to_owned()),
            "the archive must declare `{capability}`, so a build that cannot read it refuses \
             rather than dropping it: {:?}",
            trip.export.manifest.capabilities
        );
    }
}

// T-43
#[test]
fn both_cycle_kinds_travel_and_are_declared() {
    let trip = round_trip(Shape::Varied);
    let after = facts_of(&trip.destination).expect("read the imported project's facts");

    assert_eq!(
        after.cycle_kinds,
        vec![String::from("CUSTOM"), String::from("WEEKLY")],
        "a project holding both kinds must carry both"
    );
    for capability in ["cycle-kind:CUSTOM", "cycle-kind:WEEKLY"] {
        assert!(
            trip.export
                .manifest
                .capabilities
                .contains(&capability.to_owned()),
            "{capability} must be declared"
        );
    }
}

// T-44
#[test]
fn an_export_writes_nothing_to_the_source() {
    let mut fixture = Fixture::build(Shape::Varied);
    let source_path = fixture.project.path().to_path_buf();
    fixture.project.close();

    let before_bytes = std::fs::read(&source_path).expect("read the source");
    let before_facts = facts_of(&source_path).expect("read the source's facts");

    let dir = tempfile::TempDir::new().expect("temp dir");
    export_to(
        &source_path,
        &dir.path().join("staging"),
        &dir.path().join("out.evara"),
    )
    .expect("seal");

    let after_facts = facts_of(&source_path).expect("read the source's facts again");
    assert_eq!(
        after_facts.change_log, before_facts.change_log,
        "an export must not append to the change feed"
    );
    assert_eq!(
        after_facts.census, before_facts.census,
        "and must not change a row count"
    );

    // The bytes themselves, which is the strongest form of the claim available: an export
    // goes through `open_for_inspection`, which migrates nothing, recreates no trigger and
    // stamps no version — so the file is byte-identical and not merely equivalent.
    let after_bytes = std::fs::read(&source_path).expect("read the source again");
    assert_eq!(
        after_bytes, before_bytes,
        "an export must leave the source project byte-identical"
    );
}

// T-04, T-05
#[test]
fn two_exports_of_one_project_differ_only_in_when_and_by_whom() {
    let mut fixture = Fixture::build(Shape::Varied);
    let source_path = fixture.project.path().to_path_buf();
    fixture.project.close();

    let dir = tempfile::TempDir::new().expect("temp dir");
    let mut manifests = Vec::new();
    let mut payload_digests = Vec::new();
    for n in 0..2 {
        let report = export_to(
            &source_path,
            &dir.path().join(format!("staging-{n}")),
            &dir.path().join(format!("out-{n}.evara")),
        )
        .expect("seal the archive");
        payload_digests.push(report.payload_sha256.clone());
        manifests.push(report.manifest);
    }

    assert_eq!(
        payload_digests[0], payload_digests[1],
        "two snapshots of an unchanged project must be byte-identical: a page-for-page copy \
         inside a pinned read transaction is a function of the project and of nothing else"
    );

    let mut first = manifests[0].clone();
    let second = manifests[1].clone();
    // The two fields that are *allowed* to differ, normalised away. Everything else must be
    // equal, which is a stronger assertion than listing the fields that must match: a field
    // added later is covered without anybody remembering to add it here.
    first.exported_at = second.exported_at.clone();
    first.exported_by_site_id = second.exported_by_site_id;
    assert_eq!(
        first, second,
        "two exports of an unchanged project must differ only in `exportedAt` and \
         `exportedBySiteId`"
    );
}

// T-06
#[test]
fn the_integrity_tree_is_acyclic_and_each_level_matches() {
    let trip = round_trip(Shape::Varied);
    let summary = inspect(&trip.archive).expect("inspect the archive");

    // `inspect` has already compared the manifest's `integrity.sha256` with the digest of
    // `checksums.json`, and `stream_payload` has compared `checksums.json`'s entry with the
    // payload. What is left to assert is the shape: that the manifest is covered by nothing.
    assert_eq!(
        summary.manifest.integrity.entry,
        crate::entry::CHECKSUMS_ENTRY,
        "the manifest's one digest is of the checksums record"
    );
    assert_eq!(summary.manifest.integrity.algorithm, "sha256");

    let text = String::from_utf8(
        summary
            .manifest
            .to_bytes()
            .expect("the manifest round-trips through serde"),
    )
    .expect("utf-8");
    assert!(
        !text.contains(crate::entry::MANIFEST_ENTRY),
        "the manifest must not name itself: that is the cycle revision 1 had, where neither \
         file could be verified before the other and so neither was"
    );
}

// T-36
#[test]
fn an_unknown_but_safe_entry_is_accepted_and_reported() {
    let trip = round_trip(Shape::Varied);
    // The archive this build wrote has exactly three entries, so forward tolerance is tested
    // against one the test adds — in `tests_refusal`, where the archive surgery lives. Here
    // the assertion is the complement: a clean archive reports no unknown entries, so a
    // non-empty list in that test means something.
    assert!(
        trip.import.unknown_entries.is_empty(),
        "an archive this build wrote has nothing in it this build does not know: {:?}",
        trip.import.unknown_entries
    );
    assert!(
        trip.import.count_disagreements.is_empty(),
        "and the manifest's counts must agree with the payload it describes: {:?}",
        trip.import.count_disagreements
    );
}

#[test]
fn the_imported_project_opens_as_an_ordinary_project() {
    // The weakest assertion in the file, and it is here deliberately: everything above
    // proves the *contents* survived, and this proves the result is a project the
    // application can open rather than only a database the tests can read.
    let trip = round_trip(Shape::Varied);
    let dir = tempfile::TempDir::new().expect("temp dir");
    let mut workspace =
        evara_db::Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
            .expect("settings database");
    let summary = workspace
        .open_project(&trip.destination)
        .expect("the imported project opens");
    assert_eq!(summary.project_id, trip.import.project_id);
    assert_eq!(summary.schema_version, evara_db::supported_schema_version());
    assert_eq!(
        trip.import.schema_version_now,
        evara_db::supported_schema_version()
    );
    assert!(
        !trip.import.migrated,
        "a payload written at the current version needs no migration"
    );
}
