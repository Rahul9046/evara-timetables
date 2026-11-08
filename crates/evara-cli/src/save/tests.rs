//! What `evara save` accepts, refuses, and promises about the file it writes to.
//!
//! The end-to-end tests build a real project through `Workspace`, generate into it and read the
//! timetable back, because the whole point of the command is the write — a test that stopped at
//! argument parsing would prove nothing about it.

use std::path::{Path, PathBuf};

use evara_core::calendar::WorkingDays;
use evara_db::{
    CycleInput, CycleKind, PROJECT_EXTENSION, SchedulingSelection, SchoolInput, Workspace,
};
use evara_solver::SolveLimits;
use uuid::Uuid;

use super::{Outcome, Request, USAGE, run};

// ------------------------------------------------- the settings database a test writes to

thread_local! {
    /// Where this test's `Workspace` takes its settings database from.
    ///
    /// Thread-local rather than a `static`: the test harness gives each test its own thread,
    /// so this is per-test state with no lock and no interference between tests running at
    /// once. `super::new_workspace` reads it and panics when it is unset, which is what makes
    /// reaching the installation's own settings database impossible rather than merely
    /// discouraged — see that function for why it matters.
    static SETTINGS: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Points this test's `save` run at `path`.
fn use_settings_at(path: &Path) {
    SETTINGS.with_borrow_mut(|slot| *slot = Some(path.to_path_buf()));
}

/// What [`use_settings_at`] last named on this thread, for `super::new_workspace`.
pub(super) fn settings_override() -> Option<PathBuf> {
    SETTINGS.with_borrow(Clone::clone)
}

fn parse(args: &[&str]) -> Result<Request, String> {
    let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    Request::parse(&owned)
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn an_id() -> Uuid {
    Uuid::now_v7()
}

// ----------------------------------------------------------------- reading the request

#[test]
fn a_generate_and_save_needs_a_project_a_school_and_a_cycle() {
    let school = an_id();
    let cycle = an_id();
    assert_eq!(
        parse(&[
            "--project",
            "school.evaraproj",
            "--school",
            &school.to_string(),
            "--cycle",
            &cycle.to_string(),
        ]),
        Ok(Request::Generate {
            project: PathBuf::from("school.evaraproj"),
            selection: SchedulingSelection {
                school_id: school,
                cycle_id: cycle,
                term_id: None,
            },
            seed: super::DEFAULT_SEED,
            limits: SolveLimits::DEFAULT,
            name: None,
            json: false,
        })
    );
}

#[test]
fn a_project_is_always_required() {
    // Unlike `generate`, there is no fixture mode: a save needs somewhere to save to.
    let error = parse(&["--school", &an_id().to_string()]).expect_err("refused");
    assert!(error.contains("--project"), "{error}");
}

#[test]
fn a_school_and_cycle_are_required_unless_revalidating() {
    let error = parse(&["--project", "school.evaraproj"]).expect_err("refused");
    assert!(error.contains("--school"), "{error}");
    assert!(error.contains("--cycle"), "{error}");
}

#[test]
fn revalidating_needs_only_a_timetable() {
    let timetable = an_id();
    assert_eq!(
        parse(&[
            "--project",
            "school.evaraproj",
            "--revalidate",
            &timetable.to_string(),
        ]),
        Ok(Request::Revalidate {
            project: PathBuf::from("school.evaraproj"),
            timetable,
            json: false,
        })
    );
}

#[test]
fn a_selection_beside_revalidate_is_refused_rather_than_ignored() {
    // A stored timetable already knows its school, cycle and term. Accepting a selection would
    // read as though it chose what to revalidate, and it does not.
    for flag in ["--school", "--cycle", "--term"] {
        let error = parse(&[
            "--project",
            "school.evaraproj",
            "--revalidate",
            &an_id().to_string(),
            flag,
            &an_id().to_string(),
        ])
        .expect_err("refused");
        assert!(error.contains(flag), "{error}");
        assert!(error.contains("already knows"), "{error}");
    }
}

#[test]
fn a_name_beside_revalidate_is_refused() {
    let error = parse(&[
        "--project",
        "school.evaraproj",
        "--revalidate",
        &an_id().to_string(),
        "--name",
        "Something",
    ])
    .expect_err("refused");
    assert!(error.contains("--name"), "{error}");
}

#[test]
fn the_seed_the_name_and_the_limits_are_read() {
    let Ok(Request::Generate {
        seed, limits, name, ..
    }) = parse(&[
        "--project",
        "school.evaraproj",
        "--school",
        &an_id().to_string(),
        "--cycle",
        &an_id().to_string(),
        "--seed",
        "7",
        "--name",
        "Spring draft",
        "--max-candidates",
        "99",
    ])
    else {
        panic!("it parses");
    };
    assert_eq!(seed, 7);
    assert_eq!(name.as_deref(), Some("Spring draft"));
    assert_eq!(limits.max_candidate_evaluations, 99);
}

#[test]
fn a_mistyped_option_is_refused_rather_than_ignored() {
    let error = parse(&["--project", "p.evaraproj", "--sed", "7"]).expect_err("refused");
    assert!(error.contains("--sed"), "{error}");
}

#[test]
fn a_malformed_identifier_is_refused() {
    let error = parse(&[
        "--project",
        "school.evaraproj",
        "--school",
        "not-an-id",
        "--cycle",
        &an_id().to_string(),
    ])
    .expect_err("refused");
    assert!(error.contains("--school"), "{error}");
}

// ----------------------------------------------------------------- the usage text

#[test]
fn the_usage_text_says_this_command_writes() {
    // The asymmetry with `generate` is the guarantee, so it has to be in the help text where a
    // user sees it before running anything.
    assert!(USAGE.contains("WRITES"), "{USAGE}");
    assert!(USAGE.contains("read-only form"), "{USAGE}");
}

#[test]
fn the_usage_text_says_a_save_never_overwrites() {
    assert!(USAGE.contains("never overwrites"), "{USAGE}");
}

#[test]
fn the_usage_text_explains_the_freshness_refusal() {
    assert!(USAGE.contains("inputs changed"), "{USAGE}");
    assert!(USAGE.contains("Generate again"), "{USAGE}");
}

#[test]
fn the_usage_text_states_the_four_exit_codes() {
    for code in ["0 ", "1 ", "2 ", "3 "] {
        assert!(
            USAGE.contains(code),
            "the usage text omits exit code {code}"
        );
    }
}

// ----------------------------------------------------------------- end to end

/// A project holding one school with a cycle and nothing else.
///
/// Deliberately minimal, as `evaluate`'s and `generate`'s fixtures are: what these tests prove
/// is the command's path through a real file, not the solver's behaviour.
fn a_project() -> (tempfile::TempDir, PathBuf, Uuid, Uuid) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
    // The command under test opens its own `Workspace`, so it has to be told the same
    // temporary settings database this fixture builds — otherwise it would resolve the
    // installation's own and migrate it. `super::new_workspace` panics if this is not set.
    use_settings_at(&settings);
    let mut workspace = Workspace::with_settings_at(&settings, "0.1.0-test").expect("workspace");
    let path = dir.path().join(format!("cli.{PROJECT_EXTENSION}"));
    workspace.create_project(&path).expect("create project");

    let school = workspace
        .structure()
        .expect("structure")
        .create_school(&SchoolInput {
            name: "Northgate Secondary".to_owned(),
            code: None,
            timezone: "Europe/London".to_owned(),
            locale: "en-GB".to_owned(),
            working_days: WorkingDays::MONDAY_TO_FRIDAY,
        })
        .expect("create school")
        .id;
    let cycle = workspace
        .time_model()
        .expect("time model")
        .create_cycle(&CycleInput {
            school_id: school,
            name: "Standard week".to_owned(),
            kind: CycleKind::Custom,
            day_count: 5,
            week_count: 1,
            is_default: true,
        })
        .expect("create cycle")
        .id;

    assert!(workspace.close_project(), "a project was open");
    (dir, path, school, cycle)
}

#[test]
fn a_school_with_no_grid_has_nothing_to_save() {
    // Pre-solve validation refuses a school with no materialised grid, so there is no
    // timetable. Exit 3, not 2: the command worked and the answer was "nothing yet".
    let (_dir, path, school, cycle) = a_project();

    let outcome = run(&owned(&[
        "--project",
        path.to_str().expect("a utf-8 path"),
        "--school",
        &school.to_string(),
        "--cycle",
        &cycle.to_string(),
    ]))
    .expect("the command itself works");

    assert!(matches!(outcome, Outcome::NothingToSave), "{outcome:?}");
}

#[test]
fn a_missing_project_fails_the_command() {
    // No project, but still a settings database of its own: the refusal is about the
    // project file, and proving that must not migrate the installation's settings.
    let dir = tempfile::TempDir::new().expect("temp dir");
    use_settings_at(&dir.path().join("settings.sqlite"));

    let error = run(&owned(&[
        "--project",
        "no-such-file.evaraproj",
        "--school",
        &an_id().to_string(),
        "--cycle",
        &an_id().to_string(),
    ]))
    .expect_err("refused");
    assert!(!error.is_empty(), "the failure must say something");
}

#[test]
fn a_run_that_names_no_settings_database_panics_instead_of_reaching_the_installation() {
    // The guard that makes the leak unrepresentable, pinned. If this stops panicking,
    // `super::new_workspace` has regained a fallback to `Workspace::new`, and the next
    // `save` test written without the fixture will migrate the developer's settings
    // database forward -- silently, and invisibly to CI, whose runners have none.
    //
    // Deliberately no `use_settings_at` call: the override is thread-local and the harness
    // gives this test its own thread, so it starts unset.
    //
    // The panic hook is replaced for the duration so the expected panic does not print a
    // backtrace that reads like a failure. It is process-global, which is why it is put back
    // immediately rather than left for the rest of the run.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(|| {
        run(&owned(&[
            "--project",
            "unreachable.evaraproj",
            "--school",
            &an_id().to_string(),
            "--cycle",
            &an_id().to_string(),
        ]))
    });
    std::panic::set_hook(hook);

    let payload = outcome.expect_err("a run with no settings path must panic, not proceed");
    let message = payload
        .downcast_ref::<String>()
        .map_or("", String::as_str)
        .to_owned();
    assert!(
        message.contains("use_settings_at"),
        "the panic must name the fixture helper that is missing, so the omission is \
         obvious at the point it is made; got: {message}"
    );
}

#[test]
fn revalidating_a_timetable_that_does_not_exist_fails_the_command() {
    let (_dir, path, _school, _cycle) = a_project();
    let error = run(&owned(&[
        "--project",
        path.to_str().expect("a utf-8 path"),
        "--revalidate",
        &an_id().to_string(),
    ]))
    .expect_err("refused");
    assert!(!error.is_empty(), "{error}");
}
