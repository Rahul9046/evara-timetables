//! Phase 1B tests: installation settings, site-id attribution and project lifecycle.
//!
//! Every test runs against its own temporary settings database, so none of them touch the
//! real installation's settings or recent-project list.

use std::path::PathBuf;

use tempfile::TempDir;

use crate::cloud_sync::SyncedFolder;
use crate::settings::{MAX_RECENT_PROJECTS, Settings};
use crate::workspace::Workspace;
use crate::{DbError, PROJECT_EXTENSION};

const APP_VERSION: &str = "0.1.0-test";

/// An isolated installation: its own settings database in its own directory.
fn installation() -> (TempDir, Workspace) {
    let dir = TempDir::new().expect("temp dir");
    let workspace = Workspace::with_settings_at(&dir.path().join("settings.sqlite"), APP_VERSION)
        .expect("open workspace");
    (dir, workspace)
}

fn project_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(format!("{name}.{PROJECT_EXTENSION}"))
}

// -------------------------------------------------------------- installation identity

#[test]
fn site_id_persists_across_settings_reopen() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");

    let first = Settings::open_at(&path).expect("open").site_id();
    let second = Settings::open_at(&path).expect("reopen").site_id();

    assert_eq!(
        first, second,
        "an installation keeps its identity across restarts"
    );
    assert_eq!(first.get_version_num(), 7, "site_id must be UUIDv7");
}

#[test]
fn separate_installations_get_different_site_ids() {
    let a = TempDir::new().expect("temp dir");
    let b = TempDir::new().expect("temp dir");

    let one = Settings::open_at(&a.path().join("s.sqlite"))
        .expect("open")
        .site_id();
    let two = Settings::open_at(&b.path().join("s.sqlite"))
        .expect("open")
        .site_id();

    assert_ne!(one, two, "two installations must not share an identity");
}

#[test]
fn settings_are_created_in_a_missing_directory() {
    let dir = TempDir::new().expect("temp dir");
    let nested = dir.path().join("a").join("b").join("settings.sqlite");
    assert!(
        Settings::open_at(&nested).is_ok(),
        "first run must create its own directory"
    );
    assert!(nested.is_file());
}

#[test]
fn settings_database_holds_no_project_data() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");
    let settings = Settings::open_at(&path).expect("open");
    assert_eq!(settings.path(), path);

    let conn = rusqlite::Connection::open(&path).expect("raw open");
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare");
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<std::result::Result<_, _>>()
        .expect("rows");

    assert_eq!(
        tables,
        vec![
            "app_settings".to_owned(),
            // V2, M10. The replacement recovery journal: paths and a phase, so an import
            // interrupted by a crash can be reconciled on the next launch. It lives here
            // because this is the only store reachable at startup without knowing which
            // project to look at -- and the project that needs recovering may be the one
            // that will not open.
            //
            // Still no school data: a destination path and a phase name, nothing from
            // inside any `.evaraproj`.
            "pending_replacement".to_owned(),
            "recent_project".to_owned(),
            "refinery_schema_history".to_owned(),
        ],
        "settings must stay minimal and hold no school data"
    );
}

// --------------------------------------------------------------------- attribution

#[test]
fn both_projects_record_the_same_installation_as_creator() {
    let (dir, mut workspace) = installation();
    let site = workspace.site_id();

    let first = workspace
        .create_project(&project_path(&dir, "one"))
        .expect("create one");
    let second = workspace
        .create_project(&project_path(&dir, "two"))
        .expect("create two");

    assert_eq!(first.created_by_site_id, Some(site));
    assert_eq!(second.created_by_site_id, Some(site));
    assert!(first.created_here && second.created_here);
}

#[test]
fn project_ids_differ_between_projects() {
    let (dir, mut workspace) = installation();

    let first = workspace
        .create_project(&project_path(&dir, "one"))
        .expect("create one");
    let second = workspace
        .create_project(&project_path(&dir, "two"))
        .expect("create two");

    assert_ne!(
        first.project_id, second.project_id,
        "each project is its own identity"
    );
}

/// The correction D16 asked for.
#[test]
fn opening_another_installations_project_does_not_change_our_site_id() {
    let shared = TempDir::new().expect("temp dir");
    let path = project_path(&shared, "shared");

    let (_a_dir, mut a) = installation();
    let a_site = a.site_id();
    let created = a.create_project(&path).expect("create");
    assert_eq!(created.created_by_site_id, Some(a_site));
    a.close_project();

    let (_b_dir, mut b) = installation();
    let b_site = b.site_id();
    assert_ne!(a_site, b_site);

    let opened = b.open_project(&path).expect("open");
    assert_eq!(
        b.site_id(),
        b_site,
        "opening someone else's project must not adopt their identity"
    );
    assert_eq!(
        opened.created_by_site_id,
        Some(a_site),
        "their provenance is preserved"
    );
    assert!(!opened.created_here, "B did not create this project");
}

#[test]
fn change_log_attributes_to_the_opening_installation() {
    let shared = TempDir::new().expect("temp dir");
    let path = project_path(&shared, "shared");

    let (_a_dir, mut a) = installation();
    a.create_project(&path).expect("create");
    a.close_project();

    let (_b_dir, mut b) = installation();
    let b_site = b.site_id();
    b.open_project(&path).expect("open");

    let db = b.open_database_for_test();
    db.write(|tx| {
        tx.execute_batch(
            "CREATE TABLE probe (id TEXT PRIMARY KEY NOT NULL, rev INTEGER NOT NULL) STRICT;",
        )?;
        crate::change_log::install(tx, &crate::TrackedTable::new("probe").expect("identifier"))?;
        tx.execute("INSERT INTO probe (id, rev) VALUES ('p1', 1)", [])?;
        Ok(())
    })
    .expect("tracked write");

    let changes = db.changes_since(0, 10).expect("read feed");
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].site_id,
        b_site.to_string(),
        "the installation holding the file open must be attributed, not the creator"
    );
}

#[test]
fn a_tracked_write_without_the_site_function_is_rejected() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "school");
    workspace.create_project(&path).expect("create");

    let db = workspace.open_database_for_test();
    db.write(|tx| {
        tx.execute_batch(
            "CREATE TABLE probe (id TEXT PRIMARY KEY NOT NULL, rev INTEGER NOT NULL) STRICT;",
        )?;
        crate::change_log::install(tx, &crate::TrackedTable::new("probe").expect("identifier"))?;
        Ok(())
    })
    .expect("install triggers");
    workspace.close_project();

    // A bare connection, as another tool would open it: no evara_site_id() registered.
    let bare = rusqlite::Connection::open(&path).expect("raw open");
    let result = bare.execute("INSERT INTO probe (id, rev) VALUES ('x', 1)", []);
    assert!(
        result.is_err(),
        "a write that cannot be attributed must fail rather than create an unattributed row"
    );
}

#[test]
fn migration_v2_removed_project_level_site_id() {
    let (dir, mut workspace) = installation();
    workspace
        .create_project(&project_path(&dir, "school"))
        .expect("create");
    let db = workspace.open_database_for_test();

    assert_eq!(db.metadata("site_id").expect("read"), None);
    assert!(db.metadata("created_by_site_id").expect("read").is_some());
    assert!(db.schema_version() >= 2, "V2 must have been applied");
}

// ----------------------------------------------------------------------- lifecycle

#[test]
fn create_open_close_round_trip() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "school");

    assert!(!workspace.has_open_project());

    let created = workspace.create_project(&path).expect("create");
    assert!(workspace.has_open_project());
    assert_eq!(
        workspace.current_project().map(|p| p.project_id),
        Some(created.project_id)
    );
    assert!(path.is_file(), "the project file must exist on disk");

    assert!(
        workspace.close_project(),
        "closing an open project reports true"
    );
    assert!(!workspace.has_open_project());
    assert!(workspace.current_project().is_none());
    assert!(!workspace.close_project(), "closing twice is a no-op");

    let reopened = workspace.open_project(&path).expect("reopen");
    assert_eq!(
        reopened.project_id, created.project_id,
        "identity survives close and reopen"
    );
}

#[test]
fn creating_over_an_existing_project_is_refused() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "school");
    workspace.create_project(&path).expect("create");
    workspace.close_project();

    let err = workspace
        .create_project(&path)
        .expect_err("must not overwrite a project");
    assert!(
        matches!(err, DbError::ProjectAlreadyExists { .. }),
        "got {err}"
    );
}

#[test]
fn opening_a_missing_project_reports_not_found() {
    let (dir, mut workspace) = installation();
    let err = workspace
        .open_project(&project_path(&dir, "nowhere"))
        .expect_err("must not invent a project");
    assert!(matches!(err, DbError::ProjectNotFound { .. }), "got {err}");
}

#[test]
fn opening_a_non_evara_file_is_refused_not_migrated() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "impostor");
    std::fs::write(&path, b"this is not a database").expect("write file");

    let err = workspace
        .open_project(&path)
        .expect_err("must not open a foreign file");
    assert!(
        matches!(err, DbError::NotAnEvaraProject { .. }),
        "got {err}"
    );
}

#[test]
fn opening_an_unrelated_sqlite_database_is_refused() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "other-app");
    {
        let conn = rusqlite::Connection::open(&path).expect("make a sqlite file");
        conn.execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .expect("schema");
    }

    let err = workspace
        .open_project(&path)
        .expect_err("must not adopt a foreign database");
    assert!(
        matches!(err, DbError::NotAnEvaraProject { .. }),
        "a valid SQLite file that is not ours must be refused, not migrated; got {err}"
    );
}

#[test]
fn a_failed_open_leaves_nothing_open() {
    let (dir, mut workspace) = installation();
    workspace
        .create_project(&project_path(&dir, "good"))
        .expect("create");
    assert!(workspace.has_open_project());

    let _ = workspace.open_project(&project_path(&dir, "missing"));
    assert!(
        !workspace.has_open_project(),
        "a failed open must not leave the previous project loaded as stale state"
    );
}

// ----------------------------------------------------------------------- extension

#[test]
fn only_the_project_extension_is_accepted() {
    let (dir, mut workspace) = installation();

    for bad in ["school.evara", "school.sqlite", "school"] {
        let err = workspace
            .create_project(&dir.path().join(bad))
            .expect_err("a non-project extension must be rejected");
        assert!(
            matches!(err, DbError::WrongExtension { .. }),
            "{bad}: got {err}"
        );
    }

    assert!(
        workspace
            .create_project(&project_path(&dir, "school"))
            .is_ok()
    );
}

#[test]
fn the_export_extension_is_not_a_live_project() {
    let (dir, mut workspace) = installation();
    // `.evara` is reserved for the portable package; opening one as a live project would
    // be a category error.
    let err = workspace
        .create_project(&dir.path().join("archive.evara"))
        .expect_err("`.evara` is not a live project");
    assert!(matches!(err, DbError::WrongExtension { expected } if expected == PROJECT_EXTENSION));
}

#[test]
fn the_extension_is_accepted_case_insensitively() {
    let (dir, mut workspace) = installation();
    assert!(
        workspace
            .create_project(&dir.path().join("School.EvaraProj"))
            .is_ok()
    );
}

// ------------------------------------------------------------------ schema too new

#[test]
fn schema_too_new_propagates_through_the_project_layer() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "future");
    workspace.create_project(&path).expect("create");
    workspace.close_project();

    {
        let conn = rusqlite::Connection::open(&path).expect("open raw");
        conn.execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
             VALUES (?1, 'from_the_future', '2099-01-01T00:00:00.000Z', '0')",
            [i64::from(crate::supported_schema_version()) + 10],
        )
        .expect("insert future migration");
    }

    let err = workspace
        .open_project(&path)
        .expect_err("must refuse a newer project");
    assert!(err.is_schema_too_new(), "got {err}");
    assert!(
        err.to_string().contains("Update Evara"),
        "the message must tell the user what to do: {err}"
    );
    assert!(!workspace.has_open_project());
}

// ------------------------------------------------------------------ recent projects

#[test]
fn recent_projects_are_most_recently_opened_first() {
    let (dir, mut workspace) = installation();
    for name in ["first", "second", "third"] {
        workspace
            .create_project(&project_path(&dir, name))
            .expect("create");
    }

    let recents = workspace.recent_projects().expect("read recents");
    let names: Vec<&str> = recents.iter().map(|r| r.display_name.as_str()).collect();
    assert_eq!(names, vec!["third", "second", "first"]);
}

#[test]
fn reopening_a_project_moves_it_to_the_top_without_duplicating() {
    let (dir, mut workspace) = installation();
    let first = project_path(&dir, "first");
    workspace.create_project(&first).expect("create first");
    workspace
        .create_project(&project_path(&dir, "second"))
        .expect("create second");

    workspace.open_project(&first).expect("reopen first");

    let recents = workspace.recent_projects().expect("read recents");
    assert_eq!(recents.len(), 2, "reopening must not add a duplicate entry");
    assert_eq!(
        recents[0].display_name, "first",
        "the reopened project goes to the top"
    );
}

#[test]
fn recent_list_is_capped() {
    let (dir, mut workspace) = installation();
    for i in 0..(MAX_RECENT_PROJECTS + 5) {
        workspace
            .create_project(&project_path(&dir, &format!("p{i:02}")))
            .expect("create");
    }

    let recents = workspace.recent_projects().expect("read recents");
    assert_eq!(recents.len(), MAX_RECENT_PROJECTS);
    assert_eq!(
        recents[0].display_name,
        format!("p{:02}", MAX_RECENT_PROJECTS + 4),
        "the newest must survive the trim"
    );
}

#[test]
fn a_deleted_project_is_reported_as_missing_not_dropped() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "vanishing");
    workspace.create_project(&path).expect("create");
    workspace.close_project();

    // Remove the database and its WAL sidecars, as deleting it outside Evara would.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", path.display())));
    }

    let recents = workspace.recent_projects().expect("read recents");
    let entry = recents
        .iter()
        .find(|r| r.display_name == "vanishing")
        .expect("still listed");
    assert!(
        !entry.exists,
        "a missing file must be flagged, not silently removed"
    );
}

#[test]
fn a_recent_project_can_be_forgotten() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "unwanted");
    workspace.create_project(&path).expect("create");
    workspace.close_project();

    assert!(workspace.forget_recent_project(&path).expect("forget"));
    assert!(
        workspace.recent_projects().expect("read").is_empty(),
        "forgetting removes the entry"
    );
    assert!(
        path.is_file(),
        "forgetting must never delete the user's file"
    );
    assert!(
        !workspace
            .forget_recent_project(&path)
            .expect("forget again"),
        "forgetting an absent entry reports false"
    );
}

#[test]
fn recent_entries_carry_what_is_needed_to_reopen() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "school");
    let created = workspace.create_project(&path).expect("create");

    let recents = workspace.recent_projects().expect("read recents");
    let entry = &recents[0];
    assert_eq!(entry.path, path);
    assert_eq!(entry.project_id, Some(created.project_id));
    assert!(entry.exists);
    assert!(entry.last_opened_at.ends_with('Z'));
}

#[test]
fn recent_list_is_per_installation() {
    let shared = TempDir::new().expect("temp dir");
    let path = project_path(&shared, "shared");

    let (_a_dir, mut a) = installation();
    a.create_project(&path).expect("create");

    let (_b_dir, b) = installation();
    assert!(
        b.recent_projects().expect("read").is_empty(),
        "a fresh installation starts with an empty recent list"
    );
}

// ------------------------------------------------------------------- cloud folders

#[test]
fn a_project_in_a_sync_folder_is_flagged_but_still_opens() {
    let (dir, mut workspace) = installation();
    let synced = dir.path().join("Dropbox");
    std::fs::create_dir_all(&synced).expect("create folder");
    let path = synced.join(format!("school.{PROJECT_EXTENSION}"));

    let summary = workspace
        .create_project(&path)
        .expect("creating must still succeed");
    assert_eq!(
        summary.synced_folder.map(SyncedFolder::provider),
        Some("Dropbox"),
        "the warning must be surfaced"
    );
    assert!(
        workspace.has_open_project(),
        "the warning is advisory, never blocking"
    );
}

#[test]
fn an_ordinary_folder_is_not_flagged() {
    let (dir, mut workspace) = installation();
    let summary = workspace
        .create_project(&project_path(&dir, "school"))
        .expect("create");
    assert_eq!(summary.synced_folder, None);
}

#[test]
fn the_warning_survives_close_and_reopen() {
    let (dir, mut workspace) = installation();
    let synced = dir.path().join("OneDrive");
    std::fs::create_dir_all(&synced).expect("create folder");
    let path = synced.join(format!("school.{PROJECT_EXTENSION}"));

    workspace.create_project(&path).expect("create");
    workspace.close_project();

    let reopened = workspace.open_project(&path).expect("reopen");
    assert_eq!(
        reopened.synced_folder.map(SyncedFolder::provider),
        Some("OneDrive")
    );
}

// ---------------------------------------------- D13: duplicate project identity (T-64/T-65)

/// Two remembered copies of one campus, plus a third project with its own identity.
fn three_remembered() -> (TempDir, Workspace, uuid::Uuid, uuid::Uuid) {
    let (dir, mut workspace) = installation();
    let twin = uuid::Uuid::now_v7();
    let other = uuid::Uuid::now_v7();

    workspace
        .remember_imported_project(&project_path(&dir, "Northgate"), twin)
        .expect("remember the first copy");
    workspace
        .remember_imported_project(&project_path(&dir, "Northgate-copy"), twin)
        .expect("remember the second copy");
    workspace
        .remember_imported_project(&project_path(&dir, "Southmoor"), other)
        .expect("remember an unrelated project");

    (dir, workspace, twin, other)
}

#[test]
fn remembering_an_imported_project_does_not_open_it() {
    // D9's rule, asserted at the layer that owns it: the recent list gains an entry and the
    // workspace stays exactly as it was. Auto-opening would discard whatever the user had
    // open as a side effect of an import.
    let (dir, mut workspace) = installation();
    let imported = project_path(&dir, "Imported");

    workspace
        .remember_imported_project(&imported, uuid::Uuid::now_v7())
        .expect("remember");

    assert!(
        workspace.current_project().is_none(),
        "remembering a project must never open it"
    );
    assert!(
        workspace
            .recent_projects()
            .expect("read the recent list")
            .iter()
            .any(|entry| entry.path == imported),
        "the imported project must be findable in the recent list"
    );
}

#[test]
fn duplicate_identity_is_true_for_both_copies_and_false_otherwise() {
    // T-65. Both entries are marked, because neither is "the original" — Evara cannot merge
    // two copies, so it discloses the ambiguity rather than choosing a winner.
    let (dir, workspace, _twin, _other) = three_remembered();
    let listed = workspace.recent_projects().expect("read the recent list");

    let marked: Vec<&str> = listed
        .iter()
        .filter(|entry| entry.duplicate_identity)
        .map(|entry| entry.display_name.as_str())
        .collect();

    assert_eq!(
        marked,
        vec!["Northgate-copy", "Northgate"],
        "both copies are marked and the unrelated project is not; got {listed:#?}"
    );
    drop(dir);
}

#[test]
fn an_unrecorded_identity_is_never_a_duplicate() {
    // `None` is unknown, not a value. Two entries whose identity was never recorded are not
    // evidence of a twin, and marking them would put a warning on every old entry.
    let (dir, workspace) = installation();
    let mut settings = Settings::open_at(&dir.path().join("settings.sqlite")).expect("settings");
    settings
        .remember_project(&project_path(&dir, "Unknown-a"), None)
        .expect("remember");
    settings
        .remember_project(&project_path(&dir, "Unknown-b"), None)
        .expect("remember");
    drop(settings);

    let listed = workspace.recent_projects().expect("read the recent list");
    assert!(
        listed.iter().all(|entry| !entry.duplicate_identity),
        "an unknown identity is not a duplicate: {listed:#?}"
    );
    // Touching the workspace keeps it alive past the settings handle above.
    assert!(!workspace.has_open_project());
}

#[test]
fn projects_with_identity_names_every_copy_and_nothing_else() {
    let (_dir, workspace, twin, other) = three_remembered();

    let copies = workspace
        .projects_with_identity(twin)
        .expect("read by identity");
    assert_eq!(copies.len(), 2, "both copies must be named: {copies:#?}");
    assert!(
        copies.iter().all(|entry| entry.duplicate_identity),
        "an entry returned here is by definition one of several"
    );

    let single = workspace
        .projects_with_identity(other)
        .expect("read by identity");
    assert_eq!(single.len(), 1);
    assert!(
        !single[0].duplicate_identity,
        "a sole copy is not a duplicate"
    );

    assert!(
        workspace
            .projects_with_identity(uuid::Uuid::now_v7())
            .expect("read by identity")
            .is_empty(),
        "a project new to this installation has no conflicts"
    );
}

#[test]
fn an_identity_is_never_rewritten_by_remembering_a_second_copy() {
    // T-64. `project_id` is ADR 0004's, and regenerating one would destroy round-trip
    // comparison and any future synchronisation. Remembering a twin records a second row; it
    // does not renumber the first.
    let (dir, workspace, twin, _other) = three_remembered();

    let copies = workspace
        .projects_with_identity(twin)
        .expect("read by identity");
    assert!(
        copies.iter().all(|entry| entry.project_id == Some(twin)),
        "both rows must still carry the identity they were given: {copies:#?}"
    );
    drop(dir);
}

#[test]
fn the_open_project_is_recognised_whatever_the_path_is_spelled_like() {
    let (dir, mut workspace) = installation();
    let path = project_path(&dir, "Northgate");
    workspace.create_project(&path).expect("create");

    assert!(
        workspace.is_open_project(&path),
        "the project just opened must be recognised"
    );
    assert!(
        workspace.is_open_project(&PathBuf::from(path.to_string_lossy().replace('\\', "/"))),
        "a forward-slashed spelling of the same path is the same project"
    );
    assert!(
        !workspace.is_open_project(&project_path(&dir, "Southmoor")),
        "a different file is not the open project"
    );

    workspace.close_project();
    assert!(
        !workspace.is_open_project(&path),
        "nothing is the open project once nothing is open"
    );
}

#[test]
fn the_settings_path_is_the_one_the_workspace_was_built_over() {
    // The transfer worker opens its own journal handle at this path. If it pointed at the
    // installation's real settings database a test would migrate it — the defect
    // `evara-cli`'s `open_settings` seam exists to prevent.
    let dir = TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    let workspace = Workspace::with_settings_at(&settings, APP_VERSION).expect("workspace");

    assert_eq!(workspace.settings_path(), settings.as_path());
}
