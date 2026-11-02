//! T-84…T-90 — the three mandatory safeguards.
//!
//! # Two kinds of test, and why both are needed
//!
//! [`reconcile`](super::reconcile) is pure, so **every** row of S2's table can be driven
//! from fabricated [`DiskFacts`] — including the ambiguous combinations, which a
//! process-killing test would reach rarely or never. That is where coverage comes from.
//!
//! Coverage of a pure function proves the *logic* is right. It does not prove the
//! fabricated combinations are states that actually occur. So there is a second, smaller
//! set of tests that **genuinely kills a process** partway through the filesystem sequence
//! a replacement performs, then feeds the real disk state to the same function. Together:
//! the pure tests give breadth, the kill tests give fidelity.
//!
//! The *orchestration* of the protocol is Stage 2's. What is interrupted here is the
//! sequence of renames Stage 2 will perform, which is exactly what exercises the recovery
//! primitives Stage 1 owns.

use std::path::{Path, PathBuf};

use tempfile::TempDir;
use uuid::Uuid;

use crate::error::DbError;
use crate::transfer::replacement::{
    Ambiguity, DestinationState, DestinationVerdict, DiskFacts, PendingReplacement, RecoveryAction,
    ReplacementPhase, classify_destination, reconcile,
};
use crate::transfer::sidecars::{SidecarState, refuse_if_sidecars_present, sidecar_path};
use crate::{Database, PROJECT_EXTENSION, Settings};

const APP_VERSION: &str = "0.1.0-test";

/// Environment variable that turns the child test into the thing being killed.
const CHILD_SCENARIO: &str = "EVARA_TEST_REPLACEMENT_KILL_AT";
/// Where the child works.
const CHILD_DIR: &str = "EVARA_TEST_REPLACEMENT_DIR";

/// A journal row describing a replacement of `destination`.
fn row(destination: &Path, staging: &Path, phase: ReplacementPhase) -> PendingReplacement {
    PendingReplacement {
        id: Uuid::now_v7(),
        phase: Some(phase),
        destination: destination.to_path_buf(),
        staging_dir: staging.to_path_buf(),
        keep_name: Some(destination.with_extension("replaced-stamp.evaraproj")),
        started_at: "2026-11-01T00:00:00.000Z".to_owned(),
        updated_at: "2026-11-01T00:00:00.000Z".to_owned(),
    }
}

/// Facts with everything absent, to be adjusted per case.
const fn nothing() -> DiskFacts {
    DiskFacts {
        destination: DestinationState::Absent,
        staged_present: false,
        aside_present: false,
        keep_name_present: false,
        destination_sidecars: SidecarState {
            wal: false,
            shm: false,
            journal: false,
        },
        aside_sidecars: SidecarState {
            wal: false,
            shm: false,
            journal: false,
        },
    }
}

/// Facts with a sound project at the destination.
const fn good_destination() -> DiskFacts {
    DiskFacts {
        destination: DestinationState::Project,
        ..nothing()
    }
}

/// Facts with something at the destination that may not be blessed as a project.
const fn unknown_destination(zero_length: bool) -> DiskFacts {
    DiskFacts {
        destination: DestinationState::Unknown { zero_length },
        ..nothing()
    }
}

// ====================================================================== S1 — claiming

#[test]
fn t84_a_zero_length_destination_is_occupied_and_never_adopted() {
    // The safeguard proper. The approved text said a destination "absent, or present and
    // zero length" is claimed with `create_new(true)` — two halves that cannot both hold,
    // because `create_new` fails if the path exists at all.
    assert_eq!(
        classify_destination(false, false),
        DestinationVerdict::Claimable,
        "an absent destination is the only claimable one"
    );
    assert_eq!(
        classify_destination(true, true),
        DestinationVerdict::Occupied { zero_length: true },
        "an existing zero-length file is OCCUPIED -- adopting it would be writing over \
         something nobody has looked at"
    );
    assert_eq!(
        classify_destination(true, false),
        DestinationVerdict::Occupied { zero_length: false },
        "and so is one with bytes in it"
    );
}

#[test]
fn t84_the_divergence_from_database_create_is_real_and_deliberate() {
    // `Database::create` treats a zero-length file as creatable; claiming does not. The two
    // are different acts, and the difference is asserted so a later "consistency" refactor
    // has to confront it rather than quietly unify them.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("empty.{PROJECT_EXTENSION}"));
    std::fs::write(&path, b"").expect("create a zero-length file");

    Database::create(&path, APP_VERSION, Uuid::now_v7()).expect(
        "Database::create adopts a zero-length file -- making a project where the \
                 user just said to",
    );

    let path2 = dir.path().join(format!("empty2.{PROJECT_EXTENSION}"));
    std::fs::write(&path2, b"").expect("create another");
    assert_eq!(
        classify_destination(path2.exists(), true),
        DestinationVerdict::Occupied { zero_length: true },
        "an import does not -- it would be writing over an unknown file"
    );
}

#[test]
fn t85_create_new_closes_the_toctou_window() {
    // The claim exists to stop a second process creating the same name between our check
    // and our rename. `fs::rename` *replaces* an existing destination on both platforms, so
    // without an atomic claim that file would be silently destroyed.
    use std::fs::OpenOptions;

    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("contested.{PROJECT_EXTENSION}"));

    let first = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("the first claim must succeed");
    drop(first);

    let second = OpenOptions::new().write(true).create_new(true).open(&path);
    assert!(
        second.is_err(),
        "a second claim of the same name must fail -- this is the whole guarantee"
    );
    assert_eq!(
        second.unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists,
        "and fail specifically because it is taken"
    );
}

#[test]
fn t85_a_claim_that_is_still_zero_length_is_recognised_as_ours() {
    // Our own abandoned claim is the one existing file recovery may remove, and only
    // because it is empty *and* staging is alongside it, which together mean it was ours.
    let dir = TempDir::new().expect("temp dir");
    let staging = dir.path().join(".evara-import-x");
    let destination = dir.path().join(format!("claimed.{PROJECT_EXTENSION}"));

    let facts = DiskFacts {
        staged_present: true,
        ..unknown_destination(true)
    };
    let action = reconcile(facts, Some(ReplacementPhase::Staged));

    assert_eq!(
        action,
        RecoveryAction::RollBackDeletingStaging { remove_claim: true },
        "an empty destination with our staging beside it is our claim to clean up"
    );
    assert!(!action.releases_the_old_set());
    let _ = (staging, destination);
}

#[test]
fn an_unknown_non_empty_destination_is_never_deleted() {
    // The other side of S1: a file with bytes in it is somebody's, and recovery reports it
    // rather than removing it, even with staging alongside.
    let facts = DiskFacts {
        staged_present: true,
        ..unknown_destination(false)
    };
    assert_eq!(
        reconcile(facts, Some(ReplacementPhase::Staged)),
        RecoveryAction::Report(Ambiguity::UnknownFileAndNothingAside),
        "an unknown file with content is reported, never deleted"
    );
}

// ====================================================================== S2 — recovery

#[test]
fn t86_placed_with_an_absent_destination_restores_the_project() {
    // ---- The data-loss case the first draft of the design got wrong. ----
    //
    // The journal says PLACED, but the crash landed between that commit and the rename. On
    // disk: destination absent, staged file still in staging, old project in aside.
    //
    // Read literally, PLACED means "finish step 3 -- move the aside set to the keep-name",
    // which moves the user's only project to a `.replaced-` name and leaves NO project at
    // the destination at all.
    let facts = DiskFacts {
        staged_present: true,
        aside_present: true,
        ..nothing()
    };

    let action = reconcile(facts, Some(ReplacementPhase::Placed));

    assert_eq!(
        action,
        RecoveryAction::RestoreAside {
            then_delete_staging: true
        },
        "the filesystem says the rename never happened, so recovery must put the old \
         project BACK at the destination -- not move it to the keep-name"
    );
    assert!(
        !action.releases_the_old_set(),
        "and must not let go of the old set, because no good destination was observed"
    );
}

/// One row of S2's reconciliation table.
struct Case {
    name: &'static str,
    facts: DiskFacts,
    expect: RecoveryAction,
}

/// Every row of the table, as data.
///
/// Separated from the test body so the assertion loop stays readable — and so a row added
/// here is exercised by every check below without touching them.
fn reconciliation_table() -> Vec<Case> {
    vec![
        Case {
            name: "absent / staged / no aside -> roll back",
            facts: DiskFacts {
                staged_present: true,
                ..nothing()
            },
            expect: RecoveryAction::RollBackDeletingStaging {
                remove_claim: false,
            },
        },
        Case {
            name: "absent / staged / aside -> restore then clean",
            facts: DiskFacts {
                staged_present: true,
                aside_present: true,
                ..nothing()
            },
            expect: RecoveryAction::RestoreAside {
                then_delete_staging: true,
            },
        },
        Case {
            name: "absent / no staged / aside -> restore",
            facts: DiskFacts {
                aside_present: true,
                ..nothing()
            },
            expect: RecoveryAction::RestoreAside {
                then_delete_staging: false,
            },
        },
        Case {
            name: "absent / nothing / nothing -> report",
            facts: nothing(),
            expect: RecoveryAction::Report(Ambiguity::NothingRemains),
        },
        Case {
            name: "project / no staged / aside -> finish",
            facts: DiskFacts {
                aside_present: true,
                ..good_destination()
            },
            expect: RecoveryAction::FinishMovingAsideToKeepName,
        },
        Case {
            name: "project / no staged / no aside -> already finished",
            facts: good_destination(),
            expect: RecoveryAction::AlreadyFinished,
        },
        Case {
            name: "project / staged / aside -> ambiguous",
            facts: DiskFacts {
                staged_present: true,
                aside_present: true,
                ..good_destination()
            },
            expect: RecoveryAction::Report(Ambiguity::DestinationAndStagedAndAside),
        },
        Case {
            name: "project / staged / no aside -> finish by cleaning",
            facts: DiskFacts {
                staged_present: true,
                ..good_destination()
            },
            expect: RecoveryAction::FinishDeletingStaging,
        },
        Case {
            name: "not a project / staged / aside -> displace then restore",
            facts: DiskFacts {
                staged_present: true,
                aside_present: true,
                ..unknown_destination(false)
            },
            expect: RecoveryAction::DisplaceUnknownThenRestoreAside,
        },
        Case {
            name: "not a project / no staged / aside -> displace then restore",
            facts: DiskFacts {
                aside_present: true,
                ..unknown_destination(false)
            },
            expect: RecoveryAction::DisplaceUnknownThenRestoreAside,
        },
        Case {
            name: "zero-length claim / staged / no aside -> roll back, remove claim",
            facts: DiskFacts {
                staged_present: true,
                ..unknown_destination(true)
            },
            expect: RecoveryAction::RollBackDeletingStaging { remove_claim: true },
        },
        Case {
            name: "not a project / nothing aside -> report",
            facts: unknown_destination(false),
            expect: RecoveryAction::Report(Ambiguity::UnknownFileAndNothingAside),
        },
    ]
}

#[test]
fn t87_every_row_of_the_reconciliation_table() {
    // Driven from fabricated facts rather than from a crash, which is what makes covering
    // all of it -- ambiguous rows included -- affordable.
    for case in reconciliation_table() {
        // Every phase, because the point of S2 is that the phase does not decide. The same
        // facts must produce the same action whatever the journal intended.
        for phase in [
            ReplacementPhase::Staged,
            ReplacementPhase::Aside,
            ReplacementPhase::Placed,
        ] {
            assert_eq!(
                reconcile(case.facts, Some(phase)),
                case.expect,
                "{} -- under phase {:?}, the facts must decide, not the journal",
                case.name,
                phase
            );
        }
    }
}

#[test]
fn t88_the_old_set_is_released_only_with_a_good_destination() {
    // The dominating invariant, asserted over the whole cross-product rather than over the
    // rows somebody remembered to write down: 4 destination states x staged x aside x
    // keep-name x 4 phases = **128** combinations.
    //
    // Two fields are held at "no sidecars" throughout, deliberately rather than by omission:
    // `DestinationState::Project` *means* "no sidecar beside it" (`DiskFacts::observe` applies
    // that conjunction once), so a fabricated project-with-a-`-wal` is a state that cannot be
    // observed, and asserting over it would be asserting about nothing. The sidecar-bearing
    // destinations are covered by the T-89 tests, and the one combination this sweep reaches
    // without flagging -- an absent main file with a stray sidecar -- is pinned by
    // `an_absent_destination_with_a_stray_sidecar_still_blocks_the_rename`.
    let mut released = 0;

    for destination in [
        nothing(),
        good_destination(),
        unknown_destination(true),
        unknown_destination(false),
    ] {
        for staged in [false, true] {
            for aside in [false, true] {
                for keep in [false, true] {
                    let facts = DiskFacts {
                        staged_present: staged,
                        aside_present: aside,
                        keep_name_present: keep,
                        ..destination
                    };
                    for phase in [
                        None,
                        Some(ReplacementPhase::Staged),
                        Some(ReplacementPhase::Aside),
                        Some(ReplacementPhase::Placed),
                    ] {
                        let action = reconcile(facts, phase);
                        if action.releases_the_old_set() {
                            released += 1;
                            assert!(
                                facts.destination.is_blessable_project(),
                                "INVARIANT VIOLATED: the old set was released without a \
                                 destination that looks like a project. facts={facts:?} \
                                 phase={phase:?}"
                            );
                            assert!(
                                facts.aside_present,
                                "and there must actually be an aside set to release"
                            );
                        }
                    }
                }
            }
        }
    }

    assert!(
        released > 0,
        "the invariant must be exercised, not vacuously true"
    );
}

#[test]
fn an_absent_destination_with_a_stray_sidecar_still_blocks_the_rename() {
    // The one gap in `reconcile`'s own reading of S3, pinned rather than left implicit.
    //
    // The sidecar guard is written for a destination that is *present*: a stray `-wal` beside
    // something that is otherwise a project is reported rather than displaced. With the main
    // file **absent** the guard does not apply, and the facts route to `RestoreAside` -- which
    // is correct, because there is an old project to put back and refusing would leave the
    // user with nothing.
    //
    // What makes that safe is S3 step 4, not the decision function: whoever executes the
    // action calls `refuse_if_sidecars_present` immediately before the rename, so a restore
    // cannot land a main file beside a log belonging to a different database. Both halves are
    // asserted here so a later change that drops the call has a red test rather than a
    // corruption shape with no symptom.
    let dir = TempDir::new().expect("temp dir");
    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));

    let facts = DiskFacts {
        aside_present: true,
        destination_sidecars: SidecarState {
            wal: true,
            ..SidecarState::default()
        },
        ..nothing()
    };
    assert_eq!(
        reconcile(facts, Some(ReplacementPhase::Aside)),
        RecoveryAction::RestoreAside {
            then_delete_staging: false
        },
        "with the main file gone there is an old project to put back, and the sidecar guard \
         is not what decides that"
    );

    std::fs::write(sidecar_path(&destination, "-wal"), b"stale")
        .expect("a stray log at the destination's name");
    let error = refuse_if_sidecars_present(&destination)
        .expect_err("and the rename that would execute the restore must refuse");
    assert!(
        matches!(
            error,
            DbError::RenameBlocked {
                sidecar: "-wal",
                ..
            }
        ),
        "expected RenameBlocked naming -wal, got {error:?}"
    );
}

#[test]
fn an_unrecognised_phase_is_reported_and_acted_on_by_nothing() {
    // A row written by a newer build describes a protocol this one cannot see.
    for destination in [nothing(), good_destination(), unknown_destination(false)] {
        let action = reconcile(destination, None);
        assert_eq!(action, RecoveryAction::Report(Ambiguity::UnknownPhase));
        assert!(action.is_inert(), "and must change nothing");
    }
}

#[test]
fn a_phase_round_trips_through_its_stored_form() {
    for phase in [
        ReplacementPhase::Staged,
        ReplacementPhase::Aside,
        ReplacementPhase::Placed,
    ] {
        assert_eq!(ReplacementPhase::parse(phase.as_str()), Some(phase));
    }
    assert_eq!(ReplacementPhase::parse("REVERSED"), None);
    assert_eq!(ReplacementPhase::parse(""), None);
    assert_eq!(
        ReplacementPhase::parse("staged"),
        None,
        "the stored form is upper case, and a near miss is not silently accepted"
    );
}

// ====================================================================== S3 — sidecars

#[test]
fn t89_a_destination_with_a_stray_sidecar_is_not_a_project() {
    // S3 carried into recovery. A main file beside a foreign `-wal` opens cleanly and is
    // wrong, so the only safe reading of an unexplained sidecar is that the destination is
    // not trustworthy yet.
    // A main file with a `-wal` beside it is `Unknown`, never `Project` — the conjunction
    // is part of the state, so this combination cannot be fabricated wrongly.
    let facts = DiskFacts {
        destination: DestinationState::Unknown { zero_length: false },
        destination_sidecars: SidecarState {
            wal: true,
            ..SidecarState::default()
        },
        ..nothing()
    };

    let action = reconcile(facts, Some(ReplacementPhase::Placed));
    assert_eq!(
        action,
        RecoveryAction::Report(Ambiguity::UnaccountableSidecar),
        "an unexplained sidecar is reported -- never resolved by deleting the -wal, which \
         would discard committed transactions the main file does not hold"
    );
    assert!(action.is_inert());
}

#[test]
fn t89_observe_treats_a_main_file_with_a_sidecar_as_not_a_project() {
    // The conjunction applied for real, through `observe`, so the fabricated facts above
    // are known to correspond to something `observe` can produce.
    let dir = TempDir::new().expect("temp dir");
    let staging = dir.path().join(".evara-import-x");
    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let db = Database::create(&destination, APP_VERSION, Uuid::now_v7()).expect("create");
    drop(db);
    crate::transfer::checkpoint_to_single_file(&destination).expect("checkpoint");

    let journal = row(&destination, &staging, ReplacementPhase::Placed);
    let is_project = |path: &Path| Database::open_for_inspection(path).is_ok();

    let clean = DiskFacts::observe(&journal, &is_project);
    assert_eq!(
        clean.destination,
        DestinationState::Project,
        "a checkpointed project with no sidecar is a project"
    );

    // Now put a stray `-wal` beside it.
    std::fs::write(sidecar_path(&destination, "-wal"), b"stale log").expect("write");
    let fouled = DiskFacts::observe(&journal, &is_project);

    assert!(
        fouled.destination_sidecars.wal,
        "the sidecar must be observed"
    );
    assert!(
        !fouled.destination.is_blessable_project(),
        "and the destination must stop counting as a project while it is there"
    );
}

#[test]
fn t89_sidecars_are_observed_one_at_a_time() {
    // Read per file rather than as a set, because each moves on its own and an interrupted
    // replacement can leave any subset behind. A single `has_sidecars` boolean would
    // collapse exactly the distinction recovery needs.
    let dir = TempDir::new().expect("temp dir");
    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    std::fs::write(&destination, b"x").expect("create");

    std::fs::write(sidecar_path(&destination, "-shm"), b"x").expect("shm");
    let only_shm = SidecarState::observe(&destination);
    assert!(only_shm.shm && !only_shm.wal && !only_shm.journal);

    std::fs::write(sidecar_path(&destination, "-wal"), b"x").expect("wal");
    let both = SidecarState::observe(&destination);
    assert!(both.shm && both.wal && !both.journal);
    assert_eq!(
        both.first_present(),
        Some("-wal"),
        "the one that can hold data is named first"
    );
}

#[test]
fn t90_a_checkpoint_refuses_while_the_database_is_still_open() {
    // S3 step 1's refusal, and a measured correction to what this test first asserted.
    //
    // It was written to prove a checkpoint reduces a project to one file, and it created
    // the project, wrote to it, and leaked the handle so a `-wal` would survive. The
    // checkpoint then refused with `RenameBlocked` — which is the safeguard working: the
    // database is genuinely still open, SQLite will not release the log, and refusing
    // **before anything has moved** is exactly the specified behaviour.
    //
    // So the refusal is the assertion. The reduction itself is proven below, on a project
    // whose writer is really gone.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));

    let mut db = Database::create(&path, APP_VERSION, Uuid::now_v7()).expect("create");
    db.structure()
        .create_school(&crate::SchoolInput {
            name: "Northgate".to_owned(),
            code: None,
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
            working_days: evara_core::calendar::WorkingDays::MONDAY_TO_FRIDAY,
        })
        .expect("write so the WAL is not empty");

    let error = crate::transfer::checkpoint_to_single_file(&path)
        .expect_err("a database another handle still holds must refuse");

    match error {
        DbError::RenameBlocked { sidecar, name } => {
            assert_eq!(sidecar, "-wal", "the log is what cannot be released");
            assert_eq!(name, format!("school.{PROJECT_EXTENSION}"));
        }
        other => panic!("expected RenameBlocked, got {other:?}"),
    }

    // And the project is untouched: refusing before anything moves is the point.
    drop(db);
    let reopened = Database::open_for_inspection(&path);
    assert!(
        reopened.is_ok() || SidecarState::observe(&path).any(),
        "the project must still be readable, or still have its log -- either way intact"
    );
}

#[test]
fn t90_a_project_left_with_a_wal_is_reduced_without_losing_it() {
    // The reduction, on a project whose writer was killed mid-life and so genuinely left a
    // log behind. This is the state S3's step 1 exists for, and the assertion that matters
    // is that checkpointing moves the log INTO the file rather than discarding it.
    let dir = TempDir::new().expect("temp dir");
    kill_a_replacement_at("after-write-leaving-wal", dir.path());

    let path = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    assert!(path.exists(), "the killed child left its project");
    assert!(
        SidecarState::observe(&path).wal,
        "and left a -wal behind, which is the state being tested"
    );

    crate::transfer::checkpoint_to_single_file(&path)
        .expect("a project with no live handle must reduce to one file");

    assert!(!SidecarState::observe(&path).any(), "no sidecar may remain");

    let db = Database::open_for_inspection(&path).expect("the single file must open");
    let schools = db
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM school", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(Into::into)
        })
        .expect("count");
    assert_eq!(
        schools, 1,
        "the school the child wrote was in the log, and checkpointing must have moved it          into the main file -- never discarded it"
    );
}

#[test]
fn t90_an_unremovable_sidecar_refuses_before_anything_moves() {
    let dir = TempDir::new().expect("temp dir");
    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    std::fs::write(&destination, b"").expect("create");

    // A `-journal` from a pre-WAL build. `wal_checkpoint` does not remove one, so it
    // survives the close and must refuse.
    std::fs::write(sidecar_path(&destination, "-journal"), b"old journal").expect("write");

    let error = crate::transfer::refuse_if_sidecars_present(&destination)
        .expect_err("a sidecar at the destination must refuse the rename");

    match error {
        DbError::RenameBlocked { sidecar, name } => {
            assert_eq!(sidecar, "-journal");
            assert!(name.ends_with(PROJECT_EXTENSION));
        }
        other => panic!("expected RenameBlocked, got {other:?}"),
    }
}

// =============================================================== the journal (V2)

#[test]
fn the_settings_database_reaches_v2_and_keeps_its_rows() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");

    let site_id = {
        let mut settings = Settings::open_at(&path).expect("open");
        settings
            .remember_project(Path::new("/schools/north.evaraproj"), Some(Uuid::now_v7()))
            .expect("remember");
        settings.site_id()
    };

    // Reopened: the chain is idempotent and the V1 rows survive V2.
    let settings = Settings::open_at(&path).expect("reopen");
    assert_eq!(settings.site_id(), site_id, "the identity survives");
    assert_eq!(
        settings.recent_projects().expect("recent").len(),
        1,
        "and so does the recent list"
    );
    assert!(
        settings.pending_replacements().expect("pending").is_empty(),
        "with no replacement in flight"
    );
}

#[test]
fn a_journal_row_is_written_advanced_and_cleared() {
    let dir = TempDir::new().expect("temp dir");
    let mut settings = Settings::open_at(&dir.path().join("settings.sqlite")).expect("open");

    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
    let staging = dir.path().join(".evara-import-1");
    let opened = settings
        .begin_replacement(&destination, &staging)
        .expect("begin");

    assert_eq!(opened.phase, Some(ReplacementPhase::Staged));
    assert_eq!(opened.keep_name, None, "there is no keep-name yet");

    let keep = dir.path().join("school.replaced-2026.evaraproj");
    settings
        .advance_replacement(opened.id, ReplacementPhase::Aside, Some(&keep))
        .expect("advance to ASIDE");
    settings
        .advance_replacement(opened.id, ReplacementPhase::Placed, None)
        .expect("advance to PLACED");

    let pending = settings.pending_replacements().expect("list");
    assert_eq!(pending.len(), 1);
    let stored = &pending[0];
    assert_eq!(
        stored.phase,
        Some(ReplacementPhase::Placed),
        "a phase this build knows reaches the caller as itself"
    );
    assert_eq!(
        stored.keep_name.as_deref(),
        Some(keep.as_path()),
        "advancing to PLACED must not erase the keep-name recorded at ASIDE"
    );

    assert!(settings.clear_replacement(opened.id).expect("clear"));
    assert!(
        settings.pending_replacements().expect("list").is_empty(),
        "and the row is gone"
    );
    assert!(
        !settings.clear_replacement(opened.id).expect("clear again"),
        "clearing twice is success, not a conflict -- recovery may be asked to clear a \
         row another process already cleared"
    );
}

#[test]
fn two_replacements_of_one_destination_are_refused() {
    let dir = TempDir::new().expect("temp dir");
    let mut settings = Settings::open_at(&dir.path().join("settings.sqlite")).expect("open");
    let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));

    settings
        .begin_replacement(&destination, &dir.path().join(".evara-import-1"))
        .expect("the first begins");

    let error = settings
        .begin_replacement(&destination, &dir.path().join(".evara-import-2"))
        .expect_err("a second replacement of the same file must be refused");

    assert!(
        matches!(
            error,
            DbError::DuplicateValue {
                entity: "replacement",
                field: "destination"
            }
        ),
        "expected a duplicate-destination refusal, got {error:?}"
    );
}

#[test]
fn advancing_a_row_that_is_gone_is_refused() {
    // Another process finished or abandoned this replacement, and the caller must not keep
    // acting on it.
    let dir = TempDir::new().expect("temp dir");
    let mut settings = Settings::open_at(&dir.path().join("settings.sqlite")).expect("open");

    let error = settings
        .advance_replacement(Uuid::now_v7(), ReplacementPhase::Aside, None)
        .expect_err("advancing a missing row must be refused");
    assert!(
        matches!(
            error,
            DbError::NotFound {
                entity: "replacement"
            }
        ),
        "expected NotFound, got {error:?}"
    );
}

#[test]
fn a_fourth_phase_is_refused_by_the_schema() {
    // The CHECK constraint, asserted directly: the stored vocabulary is closed.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");
    let _ = Settings::open_at(&path).expect("open");

    let conn = crate::connection::open_plain(&path).expect("reopen plainly");
    let error = conn
        .execute(
            "INSERT INTO pending_replacement
                 (id, phase, destination, staging_dir, started_at, updated_at)
             VALUES ('x', 'REVERSED', '/a', '/b', 'now', 'now')",
            [],
        )
        .expect_err("a phase outside the three must be refused");
    assert!(
        error.to_string().contains("CHECK"),
        "expected a CHECK violation, got: {error}"
    );
}

#[test]
fn a_phase_a_newer_build_wrote_is_carried_as_unknown() {
    // The CHECK above closes the vocabulary *this* build writes. A later release that adds a
    // phase would widen it, and that row then reaches this build. It must arrive as `None`
    // and never as one of the three -- acting on a fabricated STAGED would delete the
    // staging directory of a protocol this build cannot see.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");
    let mut settings = Settings::open_at(&path).expect("open");
    let opened = settings
        .begin_replacement(
            &dir.path().join(format!("school.{PROJECT_EXTENSION}")),
            &dir.path().join(".evara-import-1"),
        )
        .expect("begin");

    {
        // Written past the CHECK, which is the only way to stand in for a newer build.
        let conn = crate::connection::open_plain(&path).expect("reopen plainly");
        conn.execute("PRAGMA writable_schema = ON", []).expect("on");
        conn.execute(
            "UPDATE sqlite_master
                SET sql = replace(sql, '''PLACED''', '''PLACED'', ''VERIFIED''')
              WHERE type = 'table' AND name = 'pending_replacement'",
            [],
        )
        .expect("widen the CHECK");
        conn.execute("PRAGMA writable_schema = OFF", [])
            .expect("off");
        drop(conn);

        let conn = crate::connection::open_plain(&path).expect("reopen for the write");
        conn.execute(
            "UPDATE pending_replacement SET phase = 'VERIFIED' WHERE id = ?1",
            [opened.id.to_string()],
        )
        .expect("a newer build's phase");
    }

    let pending = settings.pending_replacements().expect("list");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].phase, None,
        "a phase this build does not know must arrive as unknown, not as a default"
    );
    assert_eq!(
        reconcile(nothing(), pending[0].phase),
        RecoveryAction::Report(Ambiguity::UnknownPhase),
        "and must be reported rather than acted on"
    );
}

#[test]
fn a_journal_row_whose_id_will_not_parse_is_corruption() {
    // `CLAUDE.md`: a persisted identifier that will not parse is corruption, not input. A
    // nil-UUID fallback would make every damaged row compare equal to every other, and
    // `clear_replacement` would then never match the row it was asked to clear.
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("settings.sqlite");
    let settings = Settings::open_at(&path).expect("open");

    let conn = crate::connection::open_plain(&path).expect("reopen plainly");
    conn.execute(
        "INSERT INTO pending_replacement
             (id, phase, destination, staging_dir, started_at, updated_at)
         VALUES ('not-a-uuid', 'STAGED', '/a', '/b', 'now', 'now')",
        [],
    )
    .expect("the id column has no format check, so this is reachable");
    drop(conn);

    let error = settings
        .pending_replacements()
        .expect_err("a damaged identifier must be refused, not substituted");
    assert!(
        matches!(
            error,
            DbError::CorruptIdentifier { ref column, ref value }
                if column == "pending_replacement.id" && value == "not-a-uuid"
        ),
        "expected CorruptIdentifier naming the column and the value, got {error:?}"
    );
}

// ======================================================= interruption, for real

/// Steps of the filesystem sequence a Replace-mode finalise performs.
///
/// Named so a child process can be told exactly where to die. These are the windows S2
/// exists for: each one is *after* a journal commit and *before* the rename it authorised.
const KILL_POINTS: &[&str] = &[
    // After staging exists and the journal says STAGED, before anything else.
    "after-staged",
    // After the old project moved to aside and the journal says ASIDE, before the new file
    // is renamed in. This is the window that leaves the destination absent.
    "after-aside",
    // After the journal says PLACED but before the rename actually happened. **T-86's
    // data-loss case.**
    "after-placed-before-rename",
    // After the rename, before the aside set reached the keep-name.
    "after-placed-after-rename",
];

#[test]
fn replacement_child_process() {
    // Not a test when run normally. It is the body a killed child executes, and it is in
    // the test binary because that is the only executable a `cargo test` run is guaranteed
    // to have to hand.
    let Ok(kill_at) = std::env::var(CHILD_SCENARIO) else {
        return;
    };
    let dir = PathBuf::from(std::env::var(CHILD_DIR).expect("the child needs a directory"));

    let destination = dir.join(format!("school.{PROJECT_EXTENSION}"));
    let staging = dir.join(".evara-import-child");
    let aside = staging.join("aside");
    let staged = staging.join("staged.evaraproj");

    // A scenario of its own: build a project, write to it, and die with the handle open, so
    // the committed work is in the log and no clean close ever happened. That is how a real
    // project comes to have a `-wal` nobody is holding.
    if kill_at == "after-write-leaving-wal" {
        let mut db = Database::create(&destination, APP_VERSION, Uuid::now_v7()).expect("create");
        db.structure()
            .create_school(&crate::SchoolInput {
                name: "Northgate".to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: evara_core::calendar::WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("write");
        std::process::abort();
    }

    std::fs::create_dir_all(&aside).expect("staging");

    // A staged replacement, ready to go in.
    {
        let db = Database::create(&staged, APP_VERSION, Uuid::now_v7()).expect("stage");
        drop(db);
    }
    crate::transfer::checkpoint_to_single_file(&staged).expect("checkpoint the staged file");

    if kill_at == "after-staged" {
        std::process::abort();
    }

    // P4 step 1: the old project goes aside. Reduced to one file first — S3.
    crate::transfer::checkpoint_to_single_file(&destination).expect("checkpoint destination");
    std::fs::rename(
        &destination,
        aside.join(format!("school.{PROJECT_EXTENSION}")),
    )
    .expect("move aside");

    if kill_at == "after-aside" || kill_at == "after-placed-before-rename" {
        // Both leave the destination absent with the old set aside. The second is the one
        // whose *journal* would already say PLACED, which is precisely why the journal
        // cannot be trusted to describe the disk.
        std::process::abort();
    }

    // P4 step 2: the new file goes in.
    crate::transfer::refuse_if_sidecars_present(&destination).expect("no stray sidecar");
    std::fs::rename(&staged, &destination).expect("place");

    if kill_at == "after-placed-after-rename" {
        std::process::abort();
    }

    unreachable!("every scenario aborts");
}

/// Runs the child to its death at `kill_at`, in `dir`, and returns once it is gone.
fn kill_a_replacement_at(kill_at: &str, dir: &Path) {
    let exe = std::env::current_exe().expect("the test binary");
    let status = std::process::Command::new(exe)
        .args([
            "--exact",
            "transfer::tests_replacement::replacement_child_process",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_SCENARIO, kill_at)
        .env(CHILD_DIR, dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn the child");

    assert!(
        !status.success(),
        "the child was supposed to die at {kill_at}, not finish"
    );
}

#[test]
fn an_interrupted_replacement_is_reconciled_from_the_disk_it_left() {
    // The fidelity half. Every kill point produces a real on-disk state, and `reconcile`
    // is asked about it. What this proves that the pure tests cannot: the combinations
    // fabricated above are states that genuinely occur.
    for kill_at in KILL_POINTS {
        let dir = TempDir::new().expect("temp dir");
        let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
        let staging = dir.path().join(".evara-import-child");

        // The project about to be replaced.
        {
            let db = Database::create(&destination, APP_VERSION, Uuid::now_v7()).expect("create");
            drop(db);
        }

        kill_a_replacement_at(kill_at, dir.path());

        let journal = row(&destination, &staging, ReplacementPhase::Placed);
        let is_project = |path: &Path| Database::open_for_inspection(path).is_ok();
        let facts = DiskFacts::observe(&journal, &is_project);

        // Asked under the *worst* journal reading: PLACED, which is what the first draft of
        // the design would have acted on.
        let action = reconcile(facts, Some(ReplacementPhase::Placed));

        match *kill_at {
            "after-staged" => {
                assert_eq!(
                    facts.destination,
                    DestinationState::Project,
                    "{kill_at}: the original project is still in place"
                );
                assert_eq!(
                    action,
                    RecoveryAction::FinishDeletingStaging,
                    "{kill_at}: there is a good project and nothing aside, so the only \
                     work left is removing staging"
                );
            }
            "after-aside" | "after-placed-before-rename" => {
                assert_eq!(
                    facts.destination,
                    DestinationState::Absent,
                    "{kill_at}: the destination must be absent"
                );
                assert!(facts.aside_present, "{kill_at}: and the old set aside");
                assert_eq!(
                    action,
                    RecoveryAction::RestoreAside {
                        then_delete_staging: true
                    },
                    "{kill_at}: the old project must be put BACK, not moved to the \
                     keep-name -- following the journal here is how a project is lost"
                );
            }
            "after-placed-after-rename" => {
                assert_eq!(
                    facts.destination,
                    DestinationState::Project,
                    "{kill_at}: the new project is in place"
                );
                assert!(facts.aside_present, "{kill_at}: the old set is still aside");
                assert_eq!(
                    action,
                    RecoveryAction::FinishMovingAsideToKeepName,
                    "{kill_at}: now -- and only now -- may the old set be released"
                );
            }
            other => panic!("unhandled kill point {other}"),
        }

        // The invariant, at every real state rather than only at fabricated ones.
        if action.releases_the_old_set() {
            assert!(
                facts.destination.is_blessable_project(),
                "{kill_at}: INVARIANT VIOLATED on a real interrupted state"
            );
        }
    }
}

#[test]
fn no_interrupted_state_pairs_a_main_file_with_a_foreign_sidecar() {
    // T-90's core assertion, over real interruptions. Each side is checkpointed to a single
    // file before it moves, so there is no window in which a sidecar belonging to one
    // database sits beside another's main file.
    for kill_at in KILL_POINTS {
        let dir = TempDir::new().expect("temp dir");
        let destination = dir.path().join(format!("school.{PROJECT_EXTENSION}"));
        {
            let db = Database::create(&destination, APP_VERSION, Uuid::now_v7()).expect("create");
            drop(db);
        }

        kill_a_replacement_at(kill_at, dir.path());

        // Whatever state the kill left, the destination must not be a main file with a
        // sidecar beside it.
        if destination.exists() {
            let state = SidecarState::observe(&destination);
            assert!(
                !state.any(),
                "{kill_at}: the destination has {state:?} beside it -- a main file paired \
                 with a sidecar from another database opens cleanly and is WRONG"
            );
        }

        let aside = dir
            .path()
            .join(".evara-import-child")
            .join("aside")
            .join(format!("school.{PROJECT_EXTENSION}"));
        if aside.exists() {
            let state = SidecarState::observe(&aside);
            assert!(
                !state.any(),
                "{kill_at}: the aside set has {state:?} beside it"
            );
        }
    }
}
