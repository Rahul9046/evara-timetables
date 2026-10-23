//! What `evara generate` reads, what it refuses, and the one thing it must never do.
//!
//! # The read-only assertion is a byte comparison
//!
//! [`generating_for_a_project_changes_no_byte_of_it`] hashes the `.evaraproj` file before and
//! after. Not a check that no repository method was called, and not a trust in
//! `Database::open_for_inspection`'s documentation: the bytes. Generation reads somebody's live
//! school file, and `Database::open_existing` runs migrations and stamps
//! `last_opened_with_app_version` on *every* open, so the wrong open path would upgrade a file
//! the user only asked to look at — and would do it silently.

use std::path::PathBuf;

use evara_core::calendar::WorkingDays;
use evara_core::scheduling::fixtures::FixtureKind;
use evara_db::{CycleInput, CycleKind, PROJECT_EXTENSION, SchoolInput, Workspace};
use evara_solver::SolveLimits;
use uuid::Uuid;

use super::{DEFAULT_SEED, Outcome, Request, SchedulingSelection, USAGE, run};

fn parse(args: &[&str]) -> Result<Request, String> {
    let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    Request::parse(&owned)
}

/// Owns a slice of arguments, which is what `run` takes.
fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn an_id() -> Uuid {
    Uuid::now_v7()
}

// ----------------------------------------------------------------- reading the request

#[test]
fn a_bare_name_generates_at_the_default_seed_and_the_shipped_limits() {
    assert_eq!(
        parse(&["tiny"]),
        Ok(Request::Fixture {
            name: "tiny".to_owned(),
            seed: DEFAULT_SEED,
            limits: SolveLimits::DEFAULT,
            json: false,
        })
    );
}

#[test]
fn the_long_and_short_spellings_agree() {
    // The grammar `evara validate` and `evara evaluate` already use.
    assert_eq!(parse(&["--fixture", "tiny"]), parse(&["tiny"]));
}

#[test]
fn a_seed_and_json_are_read() {
    assert_eq!(
        parse(&["tiny", "--seed", "7", "--json"]),
        Ok(Request::Fixture {
            name: "tiny".to_owned(),
            seed: 7,
            limits: SolveLimits::DEFAULT,
            json: true,
        })
    );
}

#[test]
fn every_limit_is_readable_from_the_command_line() {
    let Ok(Request::Fixture { limits, .. }) = parse(&[
        "tiny",
        "--max-candidates",
        "10",
        "--max-steps",
        "20",
        "--max-passes",
        "3",
    ]) else {
        panic!("the limits parse");
    };
    assert_eq!(limits.max_candidate_evaluations, 10);
    assert_eq!(limits.max_construction_steps, 20);
    assert_eq!(limits.max_passes, 3);
}

#[test]
fn a_limit_that_is_not_a_number_is_refused_rather_than_defaulted() {
    // Defaulting would run a different solve from the one that was asked for, and report it as
    // the one that was asked for.
    for flag in ["--max-candidates", "--max-steps", "--max-passes"] {
        let error = parse(&["tiny", flag, "soon"]).expect_err("refused");
        assert!(error.contains(flag), "{error}");
    }
}

#[test]
fn a_mistyped_option_is_refused_rather_than_ignored() {
    let error = parse(&["tiny", "--sed", "7"]).expect_err("refused");
    assert!(error.contains("--sed"), "{error}");
}

#[test]
fn a_project_needs_a_school_and_a_cycle() {
    // ADR 0032 rule 1 scopes generation to one (school, cycle, term), and a project has held
    // several schools since V12: picking one would build a timetable over a grid the user did
    // not choose.
    let error = parse(&["--project", "school.evaraproj"]).expect_err("refused");
    assert!(error.contains("--school"), "{error}");
    assert!(error.contains("--cycle"), "{error}");
}

#[test]
fn a_selection_without_a_project_is_refused() {
    let error = parse(&["tiny", "--school", &an_id().to_string()]).expect_err("refused");
    assert!(error.contains("--school"), "{error}");
    assert!(error.contains("--project"), "{error}");
}

#[test]
fn a_fixture_and_a_project_at_once_is_refused() {
    let error = parse(&[
        "tiny",
        "--project",
        "school.evaraproj",
        "--school",
        &an_id().to_string(),
        "--cycle",
        &an_id().to_string(),
    ])
    .expect_err("refused");
    assert!(error.contains("name one"), "{error}");
}

#[test]
fn a_project_selection_is_read_in_full() {
    let school = an_id();
    let cycle = an_id();
    let term = an_id();
    assert_eq!(
        parse(&[
            "--project",
            "school.evaraproj",
            "--school",
            &school.to_string(),
            "--cycle",
            &cycle.to_string(),
            "--term",
            &term.to_string(),
            "--seed",
            "9",
        ]),
        Ok(Request::Project {
            project: PathBuf::from("school.evaraproj"),
            selection: SchedulingSelection {
                school_id: school,
                cycle_id: cycle,
                term_id: Some(term),
            },
            seed: 9,
            limits: SolveLimits::DEFAULT,
            json: false,
        })
    );
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

#[test]
fn help_and_list_are_requests_of_their_own() {
    assert_eq!(parse(&["--help"]), Ok(Request::Help));
    assert_eq!(parse(&["-h"]), Ok(Request::Help));
    assert_eq!(parse(&["--list"]), Ok(Request::List));
}

#[test]
fn naming_nothing_at_all_is_refused_with_the_usage_text() {
    let error = parse(&[]).expect_err("refused");
    assert!(error.contains("name a fixture"), "{error}");
}

// ----------------------------------------------------------------- the usage text

#[test]
fn the_usage_text_states_the_four_exit_codes() {
    for code in ["0 ", "1 ", "2 ", "3 "] {
        assert!(
            USAGE.contains(code),
            "the usage text omits exit code {code}"
        );
    }
    assert!(USAGE.contains("partial timetable"));
}

#[test]
fn the_usage_text_never_promises_a_feasible_timetable() {
    // `evara_core::lessons`' rule reaches the help text: it is the first place a claim the
    // product cannot make would leak in.
    let lowered = USAGE.to_lowercase();
    for forbidden in ["feasible", "conflict-free", "optimal", "best possible"] {
        assert!(
            !lowered.contains(forbidden) || lowered.contains("never as conflict-free"),
            "the usage text claims {forbidden:?}"
        );
    }
    assert!(
        USAGE.contains("CONFIRMED clash"),
        "the usage text must say what a complete result actually means"
    );
}

#[test]
fn the_usage_text_says_it_writes_nothing() {
    assert!(USAGE.contains("NEVER writes"), "{USAGE}");
    assert!(USAGE.contains("persistence is M7"));
}

#[test]
fn the_usage_text_says_a_limit_is_a_count_and_not_a_duration() {
    // The property that makes a limit reproducible, and the reason there is no `--timeout`.
    assert!(USAGE.contains("never a duration"), "{USAGE}");
}

#[test]
fn the_usage_text_mentions_the_alias() {
    assert!(USAGE.contains("solve"), "{USAGE}");
}

// ----------------------------------------------------------------- running it

#[test]
fn every_fixture_generates_without_the_command_failing() {
    // Every fixture runs the command to completion. The *outcome* varies on purpose, and all
    // three are correct answers: complete, partial, or refused because pre-solve validation
    // found a blocking input. What must never happen is `Err` — the command itself breaking.
    for kind in FixtureKind::all() {
        run(&owned(&[kind.name(), "--seed", "2026"]))
            .unwrap_or_else(|e| panic!("{}: the command failed: {e}", kind.name()));
    }
}

#[test]
fn a_fixture_whose_inputs_are_refused_is_not_solved_and_says_which_it_is() {
    // The gate, through the command. `infeasible-duration` carries a blocking
    // `duration-exceeds-longest-run`, so ADR 0033 §3 refuses the solve: the remedy is a field
    // the user can fill in, and searching anyway would report an unanswered setup question as a
    // scheduling difficulty.
    //
    // Exit code 3 rather than 1, because this is not a partial timetable — nothing was tried.
    let outcome = run(&owned(&["infeasible-duration"])).expect("the command works");
    assert!(matches!(outcome, Outcome::Refused), "{outcome:?}");

    let report = report_of("infeasible-duration", 2026);
    assert!(
        report.contains("Nothing was attempted"),
        "a refused run must not read as a failed search: {report}"
    );
    assert!(
        report.contains("duration-exceeds-longest-run"),
        "and must name the finding that refused it: {report}"
    );
    assert!(
        !report.contains("run again with a larger budget"),
        "the remedy for a blocking input is not a bigger budget: {report}"
    );
}

#[test]
fn the_tiny_fixture_generates_a_complete_timetable() {
    let outcome = run(&owned(&["tiny"])).expect("tiny generates");
    assert!(matches!(outcome, Outcome::Complete), "{outcome:?}");
}

#[test]
fn a_hard_school_is_partial_rather_than_a_failure() {
    // Exit code 1, not 2 and not 3. `medium` has 500 occurrences and passes pre-solve
    // validation, so it is solved — and whether it finishes is the constructor's business, not
    // the command's. Either answer is a working command.
    let outcome = run(&owned(&["medium"])).expect("the command works");
    assert!(
        matches!(outcome, Outcome::Complete | Outcome::Partial),
        "{outcome:?}"
    );
}

#[test]
fn an_unknown_fixture_fails_the_command_and_names_the_remedy() {
    let error = run(&owned(&["no-such-fixture"])).expect_err("refused");
    assert!(error.contains("--list"), "{error}");
}

#[test]
fn the_json_form_carries_the_whole_outcome() {
    // The structured form is the engine's own types rather than a second schema, so a field
    // added to `SolveOutcome` appears without this command being edited.
    let outcome = run(&owned(&["tiny", "--json"])).expect("tiny generates");
    assert!(matches!(outcome, Outcome::Complete));
}

#[test]
fn listing_the_fixtures_succeeds() {
    assert!(matches!(
        run(&owned(&["--list"])).expect("listing works"),
        Outcome::Complete
    ));
}

#[test]
fn the_same_seed_prints_the_same_report() {
    // Not a property of the solver — that is asserted in `evara-solver` — but of this command:
    // nothing in the rendering may vary between runs, which rules out a clock or a hash order
    // leaking into the output. The wall-clock line is the one exception and is excluded.
    let first = report_of("tiny", 2026);
    let again = report_of("tiny", 2026);
    assert_eq!(first, again);
}

#[test]
fn a_different_seed_can_print_a_different_report() {
    let problem = report_of("medium", 1);
    let other = report_of("medium", 2);
    assert_ne!(
        problem, other,
        "a seed that changed nothing in a 500-occurrence school is not reaching the decisions"
    );
}

/// The rendered report for one fixture at one seed, with the timing line removed.
///
/// Re-renders through the same functions `run` uses rather than capturing stdout, which keeps
/// the assertion about the content and not about the terminal.
fn report_of(name: &str, seed: u64) -> String {
    use evara_constraints::Registry;
    use evara_core::scheduling::fixtures::fixture;
    use evara_core::scheduling::{ValidationSummary, validate};
    use evara_solver::{GenerateRequest, SolveConfig, generate};

    let found = fixture(name, seed).expect("in the catalogue");
    let problem = found.problem();
    let rules = Registry::mandatory();
    let preflight = validate(problem);
    let summary = ValidationSummary::of(problem, &preflight);

    let outcome = generate(&GenerateRequest {
        problem,
        identity: super::identity_of(problem),
        constraints: &rules,
        preflight,
        may_solve: summary.may_solve() && problem.integrity().is_empty(),
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(seed),
    });

    super::as_text(
        name,
        problem,
        &rules,
        &outcome,
        std::time::Duration::from_millis(0),
    )
    .lines()
    .filter(|line| !line.contains("wall clock"))
    .collect::<Vec<_>>()
    .join("\n")
}

#[test]
fn an_unplaced_lesson_is_never_described_as_impossible_when_it_is_merely_unplaced() {
    // The mirror of `evara_core::lessons`' rule, asserted on the rendered prose: a greedy
    // constructor that ran out of room must not tell a school its curriculum cannot be
    // timetabled.
    let report = report_of("medium", 2026);
    if report.contains("Not placed by this run") {
        assert!(
            report.contains("NOT a proof that no complete timetable exists"),
            "{report}"
        );
    }
}

#[test]
fn a_complete_report_never_says_conflict_free() {
    let report = report_of("tiny", 2026);
    assert!(report.contains("no confirmed clash"), "{report}");
    assert!(!report.to_lowercase().contains("conflict-free"), "{report}");
}

#[test]
fn the_report_states_what_was_not_checked() {
    // ADR 0032 §5 rule 4: the unverified counts go beside the verdict, never omitted.
    let disclosing = report_of("unknown-group-overlap", 2026);
    assert!(disclosing.contains("Not checked:"), "{disclosing}");

    let clean = report_of("tiny", 2026);
    assert!(
        clean.contains("Nothing was left unchecked"),
        "even all-zeroes is stated rather than silent: {clean}"
    );
}

// ----------------------------------------------------------------- a real project

/// A temporary project holding one school with a cycle and nothing else.
///
/// The same shape `evara evaluate`'s tests use, deliberately: what it proves is this command's
/// path through a real file — the read-only open, the configuration compile, the exit code —
/// and a second way of building one would be a second thing to keep in step.
fn a_project() -> (tempfile::TempDir, PathBuf, Uuid, Uuid) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let settings = dir.path().join("settings.sqlite");
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
fn generating_for_a_project_changes_no_byte_of_it() {
    // The guarantee this command lives or dies by.
    let (_dir, path, school, cycle) = a_project();

    let before = std::fs::read(&path).expect("the project is readable");
    let outcome = run(&owned(&[
        "--project",
        path.to_str().expect("a utf-8 path"),
        "--school",
        &school.to_string(),
        "--cycle",
        &cycle.to_string(),
    ]));
    let after = std::fs::read(&path).expect("the project is still readable");

    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(
        before, after,
        "generating for a project changed its bytes - it must be read-only"
    );
}

#[test]
fn a_school_with_no_grid_is_refused_even_when_it_has_no_lessons() {
    // A project with a cycle and nothing else. M3's `no-grid-materialised` is blocking and
    // fires on the grid rather than on the demand, so generation is refused — correctly: there
    // is no value range for anything, and the remedy is a bell schedule and a set of cycle
    // days, which is a field the user can fill in.
    //
    // Exit code 3. The alternative — "a complete timetable of nothing" — would tell a school
    // that had built no grid that its timetable was finished.
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

    assert!(matches!(outcome, Outcome::Refused), "{outcome:?}");
}

#[test]
fn the_json_form_of_a_project_run_declares_that_nothing_was_written() {
    // Two claims a caller cannot otherwise check: the file was opened read-only, and no
    // timetable was saved. Both are M7's to change and neither may be silently true today.
    let (_dir, path, school, cycle) = a_project();
    let outcome = run(&owned(&[
        "--project",
        path.to_str().expect("a utf-8 path"),
        "--school",
        &school.to_string(),
        "--cycle",
        &cycle.to_string(),
        "--json",
    ]));
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[test]
fn a_missing_project_fails_the_command() {
    let error = run(&owned(&[
        "--project",
        "no-such-file.evaraproj",
        "--school",
        &an_id().to_string(),
        "--cycle",
        &an_id().to_string(),
    ]))
    .expect_err("refused");
    assert!(
        error.contains("no-such-file") || error.to_lowercase().contains("not"),
        "{error}"
    );
}

#[test]
fn a_school_that_is_not_in_the_project_fails_the_command() {
    let (_dir, path, _school, _cycle) = a_project();

    let error = run(&owned(&[
        "--project",
        path.to_str().expect("a utf-8 path"),
        "--school",
        &an_id().to_string(),
        "--cycle",
        &an_id().to_string(),
    ]))
    .expect_err("refused");
    assert!(!error.is_empty(), "the failure must say something");
}
