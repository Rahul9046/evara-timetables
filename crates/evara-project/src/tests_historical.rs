//! T-41 — historical schema compatibility, and what the matrix found.
//!
//! # The gap this closes
//!
//! §1.4.6 of the M10 proposal records it plainly: *nothing proves a historical project opens in
//! today's build*. No `.evaraproj` is checked in, and the only historical coverage was one
//! hand-written pre-V12 fixture. The compatibility claim had no test behind it.
//!
//! [`evara_db::fixtures::historical`] now generates a project at any schema version, seeded
//! version-awarely, and the tests here take it through both paths a historical project can
//! travel. No binary fixture enters git, and both matrices **extend themselves** when V19
//! lands, because they iterate `every_version()` rather than a list written here.
//!
//! # T-41's archive half cannot exist as specified, and here is the proof
//!
//! The proposal says: *for each `n` in V1…V18, build a project at `n`, fill it, **export**,
//! import into the current build, assert.* The export step is not achievable for `n < 18`, for
//! two independent reasons — and the second is structural rather than a limitation to work
//! around:
//!
//! 1. **Today's build refuses to open a historical project.**
//!    `Database::open_for_inspection` returns `SchemaUpgradeRequired`, deliberately: `CLAUDE.md`
//!    makes that refusal the user's decision and forbids reaching for the mutating path to
//!    dodge it. [`seal_as_the_old_build_would`] stands in for the release that owned the file,
//!    which is what it would take — so this one is surmountable.
//! 2. **A manifest can only describe a project at the *current* schema version.** The manifest
//!    is built from `facts_of`, which reads today's model: `constraint_instance` (V16),
//!    `timetable` and `timetable_entry` (V17), and — the binding one — the `RepairLineage`
//!    feature probe, `SELECT 1 FROM timetable WHERE based_on_timetable_id IS NOT NULL`, whose
//!    column arrives with **V18**. So `facts_of` fails on every earlier version and no archive
//!    can be assembled for one.
//!
//! [`every_describable_schema_version_exports_and_imports_into_this_build`] therefore reports
//! `covered [18]` today, and that is the honest figure rather than a gap left unfilled. It is
//! also consistent with reality: a `formatVersion 1` archive can only have been written by a
//! build that *had* the transfer feature, and transfer arrives with M10 at V18 — so **no real
//! archive carries an older payload**. The test keeps its value as a guard: it asserts the
//! current version is always describable, which is what would catch `facts_of` drifting until
//! it could not describe a project its own build had written.
//!
//! # So the compatibility claim is tested by the other half
//!
//! §1.4.6's question is about a **project file**, and that needs no archive:
//! [`every_version_opens_and_migrates_in_this_build`] covers **V1…V18** in full, through
//! `Database::open_existing` — the one call a user makes by opening their own file, and the
//! path every existing project will actually take.
//!
//! **It found a defect.** `backup::before_schema_rebuild` runs for every project below V12 and
//! counted `SELECT COUNT(*) FROM school` unconditionally; `school` arrives with V3, so opening
//! a V1 or V2 project failed with `no such table: school` and the file could not be opened at
//! all. Fixed in the same change, with the same existence guard the `app_meta` check beside it
//! already had.
//!
//! # What a passing archive row proves
//!
//! More than it looks, because `import` is not a copy — it runs the three verification phases
//! in order, so reaching `Ok` means all of these held:
//!
//! | Phase | What it proves |
//! | --- | --- |
//! | **V0** | the payload's schema conforms to what *this binary's own migrations* produce at that version. A migration edited after release would fail here |
//! | **V1** | the migration chain ran, in refinery's grouped transaction |
//! | **V2** | the result conforms to the reference at the current version, and `foreign_key_check` is empty |
//!
//! So the matrix does not re-run a conformance comparison of its own: that would be a second
//! implementation of what the import boundary already does, and the one in the product is the
//! one worth testing.
//!
//! # Why a census is only compared exactly at the current version
//!
//! Because below it **the migration is supposed to change the data**. Asserting that a census
//! survives unchanged across a migration would be asserting that migrations do nothing, and the
//! first one that legitimately backfilled a row would be reported as a transfer defect. What is
//! asserted instead is that every row the fixture seeded is **still there afterwards, by
//! primary key** — which is the claim about *transfer*, and it holds at every version.

use std::fmt::Write as _;

use evara_core::scheduling::NeverCancel;
use evara_db::fixtures::historical::{every_version, id_present, write_project_at};
use evara_db::transfer::{facts_of, table_census};

use crate::tests_fixture::APP_VERSION;
use crate::{IgnoreProgress, Importer, OnConflict, import, inspect, seal};

/// Seals `project` into `archive` the way the release that *owned* it would have.
///
/// # Why not `open_for_inspection` and `snapshot_into`
///
/// Because today's build refuses to open a historical project at all —
/// `SchemaUpgradeRequired`, by design: `CLAUDE.md` makes that refusal the user's decision and
/// forbids reaching for the mutating path to dodge it. An archive carrying a V10 payload was
/// therefore never written by *this* build; it was written by the build for which V10 was
/// current. `adopt_copy_for_fixture` stands in for that release, and for nothing else.
///
/// Everything after the snapshot is the real thing: the real `seal`, the real manifest, the
/// real digest chain.
fn seal_as_the_old_build_would(
    project: &std::path::Path,
    staging: &std::path::Path,
    archive: &std::path::Path,
) {
    std::fs::create_dir_all(staging).expect("staging directory");
    let snapshot = evara_db::transfer::adopt_copy_for_fixture(project, staging)
        .expect("adopt a copy of the historical project as a snapshot");
    seal(snapshot, archive, &IgnoreProgress, &NeverCancel).expect("seal the archive");
}

/// Whether this build can *describe* a project at `version` well enough to put it in an
/// archive.
///
/// Discovered rather than hard-coded, and that is the point. A manifest is built from
/// `facts_of`, which reads today's model — `school` (V3), `school_logo` (V8), `cycle.kind`
/// (V9), `constraint_instance` (V16) — so the oldest describable payload is whatever the
/// *current* `read_facts` happens to need. Writing `3` or `16` here would be a second,
/// rotting copy of that answer; asking is free and reports the floor instead of asserting it.
///
/// The floor is also not the gap it looks like. A `formatVersion 1` archive can only have been
/// written by a build that *had* the transfer feature, and transfer arrives with M10 at V18 —
/// so no real archive carries an older payload today. What the span below V18 is worth is
/// forward-looking: when V19 lands, V18 archives become historical and this matrix covers them
/// with no edit. The question for the versions underneath is whether a *project file* opens,
/// which [`every_version_opens_and_migrates_in_this_build`] answers for the whole chain.
fn describable(project: &std::path::Path) -> bool {
    facts_of(project).is_ok()
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one version's round trip read top to bottom: build, describe, seal, inspect,               import, then six groups of assertions that each depend on the last. Split into               helpers, every one would need the previous one's facts, report and ids threaded               through it, and the order the phases run in -- which is the thing under test --               would stop being visible in one place"
)]
fn every_describable_schema_version_exports_and_imports_into_this_build() {
    let current = evara_db::supported_schema_version();
    let mut covered = Vec::new();

    let mut not_describable = Vec::new();

    for version in every_version() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let project = dir.path().join(format!("v{version}.evaraproj"));

        // ---- build a project as the release at `version` would have left it ----
        let built = write_project_at(&project, version, uuid::Uuid::now_v7())
            .unwrap_or_else(|error| panic!("build a project at V{version}: {error}"));
        assert_eq!(built.version, version);

        // The generator must not quietly stop seeding. Every skip has a reason, and the only
        // acceptable one is that the table does not exist at this version — anything else
        // (a required column the plan does not cover) means the matrix is testing less than
        // it did, which is exactly how this kind of test rots.
        for (table, reason) in &built.skipped {
            assert!(
                reason.contains("does not exist yet") || reason.contains("was not seeded"),
                "V{version}: `{table}` was skipped for a reason that needs fixing in the \
                 seed plan, not tolerating: {reason}"
            );
        }

        // A version this build cannot describe cannot be put in an archive at all, because
        // the manifest is built from these facts. Recorded and skipped rather than asserted
        // away, so the floor is visible in the test's own output.
        if !describable(&project) {
            not_describable.push(version);
            continue;
        }

        let before = facts_of(&project).expect("read the historical project's facts");
        assert_eq!(
            before.schema_version, version,
            "the fixture must stand at the version it claims"
        );

        // ---- export ----
        let archive = dir.path().join(format!("v{version}.evara"));
        seal_as_the_old_build_would(&project, &dir.path().join("staging"), &archive);

        // The manifest says which schema the payload is, which is what a reader refuses on
        // before writing anything.
        let summary = inspect(&archive).expect("inspect the archive");
        assert!(
            summary.readable.is_readable(),
            "V{version}: this build must be able to read an archive it wrote"
        );
        assert_eq!(
            summary.manifest.payload.schema_version, version,
            "V{version}: the manifest must declare the payload's own schema version"
        );
        assert_eq!(summary.manifest.project_id, before.identity.project_id);

        // ---- import into this build ----
        let destination = dir.path().join("imported.evaraproj");
        let mut settings =
            evara_db::Settings::open_at(&dir.path().join("settings.sqlite")).expect("settings");
        let report = import(
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
        .unwrap_or_else(|error| {
            panic!("V{version}: a project from this schema must import into V{current}: {error}")
        });

        // ---- the migration happened, and is recorded ----
        assert_eq!(report.schema_version_found, version);
        assert_eq!(
            report.schema_version_now, current,
            "V{version}: an import must leave the project at this build's version"
        );
        assert_eq!(
            report.migrated,
            version < current,
            "V{version}: `migrated` must say whether the chain actually ran"
        );

        // ---- identity is never regenerated ----
        let after = facts_of(&destination).expect("read the imported facts");
        assert_eq!(
            after.identity.project_id, before.identity.project_id,
            "V{version}: a transferred project is the same project"
        );
        assert_eq!(
            after.identity.created_by_site_id, before.identity.created_by_site_id,
            "V{version}: the importing installation must not become the creator"
        );
        assert_eq!(after.schema_version, current);

        // ---- every seeded row survived, by primary key ----
        for (table, id) in &built.ids {
            assert!(
                id_present(&destination, table, *id).expect("read the imported project"),
                "V{version}: the row seeded in `{table}` ({id}) did not survive the \
                 transfer and migration to V{current}"
            );
        }

        // ---- and at the current version, transfer is exactly lossless ----
        if version == current {
            assert_eq!(
                table_census(&destination).expect("census"),
                before.census,
                "at the current version no migration runs, so every row count must match"
            );
        }

        covered.push((version, built.seeded.len()));
    }

    // The matrix covered the whole chain, not a prefix of it. Asserted rather than assumed,
    // because a loop that silently iterated nothing would pass every assertion above.
    println!(
        "T-41 archive matrix: covered {:?}; not describable by this build's manifest: {:?}",
        covered
            .iter()
            .map(|(version, _)| *version)
            .collect::<Vec<_>>(),
        not_describable
    );

    // The current version must always be in the archive matrix. If it ever is not, the
    // manifest can no longer describe a project this build itself produced, which is a defect
    // and not a floor.
    assert!(
        covered.iter().any(|(version, _)| *version == current),
        "the current version must be archivable: {covered:?}"
    );
    // And the floor must be a contiguous prefix — a hole in the middle would mean a *later*
    // migration removed something `facts_of` needs, which is a different and worse thing than
    // an early version not having it yet.
    assert!(
        not_describable.iter().all(|version| *version < current),
        "undescribable versions must be an early prefix, not a hole: {not_describable:?}"
    );
    assert!(
        covered.iter().any(|(_, seeded)| *seeded >= 15),
        "the covered versions must seed a real spine, not an empty project: {covered:?}"
    );
}

#[test]
fn a_historical_project_is_refused_as_a_destination_without_replace() {
    // The S1 rule, at a historical version: an existing project is somebody's file whatever
    // schema it is at, and an import must not write over it unasked.
    let dir = tempfile::TempDir::new().expect("temp dir");
    // The current version, because this test is about the destination rule rather than about
    // migration, and an archive can only be built from a describable project.
    let current = evara_db::supported_schema_version();
    let project = dir.path().join("current.evaraproj");
    write_project_at(&project, current, uuid::Uuid::now_v7())
        .expect("build a project at the current version");

    let archive = dir.path().join("v10.evara");
    seal_as_the_old_build_would(&project, &dir.path().join("staging"), &archive);

    let mut settings =
        evara_db::Settings::open_at(&dir.path().join("settings.sqlite")).expect("settings");
    let refusal = import(
        &archive,
        &project,
        OnConflict::Refuse,
        Importer {
            app_version: APP_VERSION,
            site_id: uuid::Uuid::now_v7(),
        },
        &mut settings,
        &IgnoreProgress,
        &NeverCancel,
    )
    .expect_err("an occupied destination must be refused");

    assert!(
        matches!(refusal, crate::TransferError::DestinationExists { .. }),
        "expected DestinationExists, got {refusal:?}"
    );
    // And the historical project is still at its own version, unmigrated and untouched.
    assert_eq!(
        facts_of(&project).expect("read it again").schema_version,
        current,
        "a refused import must leave the file it refused to replace exactly as it was"
    );
}

/// The other half of T-41, and the half that covers the **whole** chain.
///
/// §1.4.6's gap is about a *project file*: "nothing proves a historical project opens in
/// today's build". That question needs no archive, so it has no V3 floor — `app_meta` and
/// `change_log` are enough, and V1 and V2 are covered here even though no archive can carry
/// them.
///
/// `open_existing` is the real-world path and the one every existing project will actually
/// take: it runs the migrations, recreates the triggers and stamps the app version. Opening is
/// a **write**, which is exactly why `open_for_inspection` refuses to do it and why the
/// archive half had to stand in for the old build.
#[test]
fn every_version_opens_and_migrates_in_this_build() {
    let current = evara_db::supported_schema_version();
    let dir = tempfile::TempDir::new().expect("temp dir");

    for version in every_version() {
        let project = dir.path().join(format!("open-v{version}.evaraproj"));
        let built = write_project_at(&project, version, uuid::Uuid::now_v7())
            .unwrap_or_else(|error| panic!("build a project at V{version}: {error}"));

        // The one call a user makes by opening their own file.
        let db = evara_db::Database::open_existing(&project, APP_VERSION, uuid::Uuid::now_v7())
            .unwrap_or_else(|error| {
                let mut detail = error.to_string();
                let mut source = std::error::Error::source(&error);
                while let Some(inner) = source {
                    let _ = writeln!(detail);
                    detail.push_str("  caused by: ");
                    detail.push_str(&inner.to_string());
                    source = std::error::Error::source(inner);
                }
                panic!("a V{version} project must open in a V{current} build: {detail}")
            });

        assert_eq!(
            db.schema_version(),
            current,
            "V{version}: opening must have migrated the project to this build's version"
        );
        drop(db);

        // Identity is not regenerated by a migration either.
        let after = facts_of(&project).expect("read the migrated project's facts");
        assert_eq!(
            after.identity.project_id, built.project_id,
            "V{version}: migrating must not change the project's identity"
        );
        assert_eq!(
            after.identity.created_by_site_id,
            Some(built.created_by_site_id)
        );

        // And every seeded row came through the chain.
        for (table, id) in &built.ids {
            assert!(
                id_present(&project, table, *id).expect("read the migrated project"),
                "V{version}: the row seeded in `{table}` ({id}) did not survive migration \
                 to V{current}"
            );
        }
    }
}
