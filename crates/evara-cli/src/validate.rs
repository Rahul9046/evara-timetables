//! `evara validate` — run pre-solve validation over a synthetic fixture.
//!
//! # What this reports, and what it never claims
//!
//! It reports what pre-solve validation found in a scheduling problem's **inputs**: what
//! refuses a solve, what will not be checked, and what two stored figures disagree about.
//!
//! It does **not** report that a timetable exists. Nothing in Evara can: there is no solver.
//! A clean run prints *"Pre-solve validation found no blocking input issues"* and deliberately
//! never *"the timetable is feasible"* — `evara_core::lessons`' rule binds this command as it
//! binds the module behind it:
//!
//! > Never report that a demand *can* be scheduled.
//!
//! # Two inputs, one report
//!
//! A **fixture** is built in memory and validated: no database, no file, no migration.
//!
//! A **project** is read from disk, through
//! [`Database::open_for_inspection`](evara_db::Database::open_for_inspection) — which runs no
//! migration, recreates no trigger, stamps no metadata, and hands back a connection SQLite
//! refuses every write through. The ordinary open path writes on every open, so taking it
//! here would mean a command that quietly upgraded the file it was asked to look at. The
//! school, cycle and term are named explicitly, because
//! [ADR 0032](../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md) rule 1
//! scopes a solve to one of each and a project has held several schools since `V12`.
//!
//! Neither input starts a solver, and neither writes a timetable.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0 | Validation completed and found nothing blocking |
//! | 1 | Validation completed and found blocking findings |
//! | 2 | The command itself failed — an unknown fixture, a bad option |
//!
//! Warnings and informational findings leave the code at 0. They are not failures: §3.2
//! reserves a refusal for an input whose remedy is a field the user can fill in, and a warning
//! by definition is not one.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use evara_core::scheduling::diagnostics::{Diagnostic, DiagnosticReport, Severity};
use evara_core::scheduling::fixtures::{FixtureKind, fixture};
use evara_core::scheduling::{Problem, ValidationSummary, validate};
use evara_db::{AssemblyReport, Completeness, Database, SchedulingSelection};
use uuid::Uuid;

/// The default seed, matching `evara fixture` so the two commands describe the same school.
const DEFAULT_SEED: u64 = 2026;

/// What the command decided, so `main` can choose an exit code without re-deriving it.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Nothing blocking was found.
    Clean,
    /// Blocking findings were found.
    Blocked,
}

/// Runs the subcommand.
///
/// `args` is everything after `validate`.
pub(crate) fn run(args: &[String]) -> Result<Outcome, String> {
    match Request::parse(args)? {
        Request::Help => {
            print!("{USAGE}");
            Ok(Outcome::Clean)
        }
        Request::List => {
            for kind in FixtureKind::all() {
                println!("  {}", kind.name());
            }
            Ok(Outcome::Clean)
        }
        Request::Validate { name, seed, json } => validate_fixture(&name, seed, json),
        Request::Inventory { project } => inventory(&project),
        Request::Project {
            project,
            selection,
            json,
        } => validate_project(&project, &selection, json),
    }
}

/// Validates one synthetic fixture, in memory.
fn validate_fixture(name: &str, seed: u64, json: bool) -> Result<Outcome, String> {
    let found = fixture(name, seed).ok_or_else(|| {
        format!("no fixture is called {name:?}. Run `evara validate --list` to see them.")
    })?;

    let mut blocked = false;
    for (n, problem) in found.problems.iter().enumerate() {
        let report = validate(problem);
        let summary = ValidationSummary::of(problem, &report);
        blocked |= !summary.may_solve();

        let label = if found.problems.len() == 1 {
            found.name.to_owned()
        } else {
            format!(
                "{} — school {} of {}",
                found.name,
                n + 1,
                found.problems.len()
            )
        };
        if json {
            print!("{}", as_json(&label, problem, &report, &summary));
        } else {
            print!("{}", as_text(&label, problem, &report, &summary));
        }
    }

    Ok(if blocked {
        Outcome::Blocked
    } else {
        Outcome::Clean
    })
}

/// Validates one school of a real project, reading it and nothing more.
///
/// Opened through [`Database::open_for_inspection`], so no migration runs, no trigger is
/// recreated, no metadata is stamped, and SQLite refuses every write on the connection. The
/// ordinary open path performs all three of the first, which is exactly why it is not used
/// here: a command asked to look at somebody's live school file must not change it.
fn validate_project(
    path: &Path,
    selection: &SchedulingSelection,
    json: bool,
) -> Result<Outcome, String> {
    let db = Database::open_for_inspection(path).map_err(|error| describe(path, &error))?;
    let report = db
        .scheduling()
        .validate(selection)
        .map_err(|error| describe(path, &error))?;

    if json {
        print!("{}", project_json(path, &report));
    } else {
        print!("{}", project_text(path, &report));
    }
    Ok(if report.may_solve() {
        Outcome::Clean
    } else {
        Outcome::Blocked
    })
}

/// Lists a project's schools, cycles and terms, with the identifiers to pass back.
///
/// Not decoration. A selection is three identifiers and nobody knows one by heart, so without
/// this the project form of the command would be unusable from a terminal: the identifiers are
/// otherwise visible only inside the application, which is the thing a headless command exists
/// to avoid needing.
fn inventory(path: &Path) -> Result<Outcome, String> {
    let mut db = Database::open_for_inspection(path).map_err(|error| describe(path, &error))?;

    let schools = db
        .structure()
        .schools()
        .map_err(|error| describe(path, &error))?;
    if schools.is_empty() {
        println!("{} holds no schools yet.", file_name(path));
        return Ok(Outcome::Clean);
    }

    for school in schools {
        println!("school  {}  {}", school.id, school.name);
        let cycles = db
            .time_model()
            .cycles(school.id)
            .map_err(|error| describe(path, &error))?;
        for cycle in cycles {
            println!("  cycle {}  {}", cycle.id, cycle.name);
        }
        let years = db
            .structure()
            .academic_years(school.id)
            .map_err(|error| describe(path, &error))?;
        for year in years {
            let terms = db
                .structure()
                .terms(year.id)
                .map_err(|error| describe(path, &error))?;
            for term in terms {
                println!("  term  {}  {} / {}", term.id, year.name, term.name);
            }
        }
        println!("  (omit --term for a grid materialised for the whole year)");
    }
    Ok(Outcome::Clean)
}

/// Turns a database failure into a sentence, keeping the file name and not the path.
///
/// `DbError`'s own messages already avoid leaking a directory, which is what that type's test
/// asserts; this adds the file name where the failure is about the file.
fn describe(path: &Path, error: &evara_db::DbError) -> String {
    format!("{} — {error}", file_name(path))
}

/// A path's file name, for a message that must not carry the directory.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("the selected file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// Print the usage text.
    Help,
    /// List the fixtures that can be validated.
    List,
    /// Validate one fixture.
    Validate {
        /// Which fixture.
        name: String,
        /// The seed to generate it from.
        seed: u64,
        /// Whether to print the structured form.
        json: bool,
    },
    /// List a project's schools, cycles and terms.
    Inventory {
        /// The project file.
        project: PathBuf,
    },
    /// Validate one school of a project.
    Project {
        /// The project file.
        project: PathBuf,
        /// Which school, cycle and term.
        selection: SchedulingSelection,
        /// Whether to print the structured form.
        json: bool,
    },
}

impl Request {
    /// Reads the arguments.
    ///
    /// Strict, for the reason `evara fixture` is strict: a silently ignored `--sed 7` would
    /// report on a different school from the one the user believed they asked about.
    ///
    /// The same rule is why `--school` and `--cycle` are **required** beside `--project`
    /// rather than defaulted. A project has held several schools since `V12`, so picking one
    /// would scope the entire report to an institution the user did not name; and a cycle
    /// chosen for them would validate a grid they did not choose, which is the mistake
    /// `GridPreviewSection.tsx` already refuses to make. `--term` is genuinely optional,
    /// because a grid materialised for the whole year has no term.
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut json = false;
        let mut list = false;
        let mut project: Option<PathBuf> = None;
        let mut school: Option<Uuid> = None;
        let mut cycle: Option<Uuid> = None;
        let mut term: Option<Uuid> = None;
        let mut rest = args.iter();

        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(Self::Help),
                "--list" | "-l" => list = true,
                "--json" => json = true,
                "--seed" => {
                    let value = rest.next().ok_or("--seed needs a number")?;
                    seed = value
                        .parse()
                        .map_err(|_| format!("--seed {value:?} is not a number"))?;
                }
                // Accepted so `evara validate --fixture tiny` and `evara validate tiny` both
                // work: the brief's suggested spelling and the shorter one people will type.
                "--fixture" => {
                    let value = rest.next().ok_or("--fixture needs a name")?;
                    name = Some(value.clone());
                }
                "--project" | "-p" => {
                    let value = rest.next().ok_or("--project needs a path")?;
                    project = Some(PathBuf::from(value));
                }
                "--school" => school = Some(identifier("--school", rest.next())?),
                "--cycle" => cycle = Some(identifier("--cycle", rest.next())?),
                "--term" => term = Some(identifier("--term", rest.next())?),
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other:?}\n\n{USAGE}"));
                }
                other => {
                    if name.is_some() {
                        return Err(format!(
                            "only one fixture at a time; {other:?} is a second one"
                        ));
                    }
                    name = Some(other.to_owned());
                }
            }
        }

        if let Some(project) = project {
            if name.is_some() {
                return Err(String::from(
                    "a fixture and a project are two different things to validate; name one",
                ));
            }
            if list {
                return Ok(Self::Inventory { project });
            }
            let (Some(school_id), Some(cycle_id)) = (school, cycle) else {
                return Err(format!(
                    "--project needs --school and --cycle. Run `evara validate --project \
                     <path> --list` to see what the project holds.\n\n{USAGE}"
                ));
            };
            return Ok(Self::Project {
                project,
                selection: SchedulingSelection {
                    school_id,
                    cycle_id,
                    term_id: term,
                },
                json,
            });
        }

        // Refused rather than ignored: a selection without a project is a command that would
        // silently report on something else entirely.
        for (flag, given) in [
            ("--school", school.is_some()),
            ("--cycle", cycle.is_some()),
            ("--term", term.is_some()),
        ] {
            if given {
                return Err(format!("{flag} only means something with --project"));
            }
        }
        if list {
            return Ok(Self::List);
        }

        match name {
            Some(name) => Ok(Self::Validate { name, seed, json }),
            None => Err(format!(
                "name a fixture, or pass --project <path>, or --list\n\n{USAGE}"
            )),
        }
    }
}

/// Reads one identifier argument.
fn identifier(flag: &str, value: Option<&String>) -> Result<Uuid, String> {
    let value = value.ok_or_else(|| format!("{flag} needs an identifier"))?;
    Uuid::parse_str(value).map_err(|_| format!("{flag} {value:?} is not an identifier"))
}

/// The human-readable report.
fn as_text(
    label: &str,
    problem: &Problem,
    report: &evara_core::scheduling::DiagnosticReport,
    summary: &ValidationSummary,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{label} — {}", problem.context.school.name);
    let _ = writeln!(
        out,
        "  {} section{}, {} occurrence{}, {} teaching slot{}",
        problem.sections.len(),
        plural(problem.sections.len()),
        problem.activities.len(),
        plural(problem.activities.len()),
        problem.timeslots.iter().filter(|s| s.is_teaching).count(),
        plural(problem.timeslots.iter().filter(|s| s.is_teaching).count()),
    );
    let _ = writeln!(
        out,
        "  {} blocking, {} warning{}, {} informational",
        summary.blocking,
        summary.warnings,
        plural(summary.warnings),
        summary.informational
    );

    if summary.may_solve() {
        // The exact phrasing, and never "the timetable is feasible": validation proved
        // nothing impossible, which is a different and much weaker statement.
        let _ = writeln!(
            out,
            "  Pre-solve validation found no blocking input issues. This is not a promise \
             that a timetable exists."
        );
    } else {
        let _ = writeln!(
            out,
            "  Generation is refused: {} input{} must be corrected first.",
            summary.blocking,
            plural(summary.blocking)
        );
    }

    if summary.indeterminate_group_pairs > 0 || summary.unchecked_attendance > 0 {
        let _ = writeln!(
            out,
            "  Not checked: {} group pair{} of unknown overlap, {} section{} of unknown \
             attendance",
            summary.indeterminate_group_pairs,
            plural(summary.indeterminate_group_pairs),
            summary.unchecked_attendance,
            plural(summary.unchecked_attendance),
        );
    }

    render_findings(report, &mut out);
    out.push('\n');
    out
}

/// Writes the findings, grouped by severity.
///
/// Shared by the fixture and the project forms so the two describe one finding identically.
/// A second copy would drift, and the whole point of one diagnostic vocabulary is that a
/// reader cannot tell which stage or which input produced a given line.
fn render_findings(report: &DiagnosticReport, out: &mut String) {
    for severity in [Severity::Blocking, Severity::Warning, Severity::Info] {
        let findings: Vec<&Diagnostic> = report.of_severity(severity).collect();
        if findings.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n  {}:", heading(severity));
        for finding in findings {
            let _ = writeln!(
                out,
                "    [{}] {}",
                finding.code.key(),
                finding.subject.label
            );
            for line in wrap(&finding.message, 76) {
                let _ = writeln!(out, "        {line}");
            }
            if let Some(remedy) = &finding.remedy {
                // The arrow marks the remedy once; a continuation line is indented to match
                // rather than repeating it, which would read as a second remedy.
                for (n, line) in wrap(remedy, 72).into_iter().enumerate() {
                    let marker = if n == 0 { "→" } else { " " };
                    let _ = writeln!(out, "        {marker} {line}");
                }
            }
        }
    }
}

/// The structured form, built from the same diagnostic types rather than a second schema.
fn as_json(
    label: &str,
    problem: &Problem,
    report: &evara_core::scheduling::DiagnosticReport,
    summary: &ValidationSummary,
) -> String {
    // `DiagnosticReport` already serialises; the wrapper adds only what the text form shows
    // above the findings. Inventing a separate schema would be a second contract to keep in
    // step with the first.
    let wrapper = serde_json::json!({
        "fixture": label,
        "school": problem.context.school.name,
        "maySolve": summary.may_solve(),
        "counts": {
            "blocking": summary.blocking,
            "warnings": summary.warnings,
            "informational": summary.informational,
            "indeterminateGroupPairs": summary.indeterminate_group_pairs,
            "unansweredDisjointness": summary.unanswered_disjointness,
            "uncheckedAttendance": summary.unchecked_attendance,
            "sharedEntities": summary.shared_entities,
        },
        "findings": report,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&wrapper)
            .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
    )
}

/// The human-readable report for a project.
///
/// Says which school, cycle and term it is about before anything else, because a project
/// holds several of the first and a reader scrolling past a report needs to know which one
/// they are looking at.
fn project_text(path: &Path, report: &AssemblyReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} — {}", file_name(path), report.school);
    let _ = writeln!(out, "  school {}", report.selection.school_id);
    let _ = writeln!(out, "  cycle  {}", report.selection.cycle_id);
    let _ = match report.selection.term_id {
        Some(term) => writeln!(out, "  term   {term}"),
        None => writeln!(out, "  term   (none — a grid for the whole year)"),
    };
    let _ = writeln!(
        out,
        "  {} blocking, {} warning{}, {} informational",
        report.summary.blocking,
        report.summary.warnings,
        plural(report.summary.warnings),
        report.summary.informational
    );

    if report.may_solve() {
        // The exact phrasing, and never "the timetable is feasible".
        let _ = writeln!(
            out,
            "  Pre-solve validation found no blocking input issues. This is not a promise \
             that a timetable exists."
        );
    } else {
        let _ = writeln!(
            out,
            "  Generation is refused: {} input{} must be corrected first.",
            report.summary.blocking,
            plural(report.summary.blocking)
        );
    }

    if report.summary.indeterminate_group_pairs > 0 || report.summary.unchecked_attendance > 0 {
        let _ = writeln!(
            out,
            "  Not checked: {} group pair{} of unknown overlap, {} section{} of unknown \
             attendance",
            report.summary.indeterminate_group_pairs,
            plural(report.summary.indeterminate_group_pairs),
            report.summary.unchecked_attendance,
            plural(report.summary.unchecked_attendance),
        );
    }

    if let Completeness::RestrictionsNotEnforced { rules } = report.completeness {
        let _ = writeln!(
            out,
            "  Not enforced: {rules} stored availability rule{} name something this engine \
             does not schedule.",
            plural(rules)
        );
    }

    // Reported separately from the findings and never dressed as one. An internal
    // inconsistency is a defect in Evara, not an input a user can correct, and ADR 0036 §4
    // is explicit that asking them to act on it would hand them somebody else's bug. It
    // still refuses the solve, which `may_solve` above has already said.
    if !report.integrity.is_empty() {
        let _ = writeln!(
            out,
            "\n  Internal inconsistency — please report this; it is a defect in Evara, not \
             in your data:"
        );
        for error in &report.integrity {
            let _ = writeln!(out, "    {error:?}");
        }
    }

    render_findings(&report.findings, &mut out);
    out.push('\n');
    out
}

/// The structured form for a project, in the same vocabulary as the fixture form.
///
/// `DiagnosticReport` serialises itself, so the wrapper adds only the selection and the
/// counts the text form shows above the findings. Inventing a second schema would be a second
/// contract to keep in step with the first.
fn project_json(path: &Path, report: &AssemblyReport) -> String {
    let wrapper = serde_json::json!({
        "project": file_name(path),
        "school": {
            "id": report.selection.school_id,
            "name": report.school,
        },
        "cycleId": report.selection.cycle_id,
        "termId": report.selection.term_id,
        "maySolve": report.may_solve(),
        "completeness": report.completeness,
        "counts": {
            "blocking": report.summary.blocking,
            "warnings": report.summary.warnings,
            "informational": report.summary.informational,
            "indeterminateGroupPairs": report.summary.indeterminate_group_pairs,
            "unansweredDisjointness": report.summary.unanswered_disjointness,
            "uncheckedAttendance": report.summary.unchecked_attendance,
            "sharedEntities": report.summary.shared_entities,
        },
        "assembly": report.stats,
        "integrity": report.integrity,
        "findings": report.findings,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&wrapper)
            .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
    )
}

/// What to call a severity in a printed report.
const fn heading(severity: Severity) -> &'static str {
    match severity {
        Severity::Blocking => "Blocking — generation cannot proceed",
        Severity::Warning => "Warnings — reported, and not blocking",
        Severity::Info => "Worth knowing",
    }
}

/// `"s"` unless the count is one.
const fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Breaks prose into lines of at most `width` characters, on word boundaries.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// The usage text.
const USAGE: &str = "\
Usage: evara validate <fixture> [--seed N] [--json]
       evara validate --fixture <fixture> [--seed N] [--json]
       evara validate --list
       evara validate --project <path.evaraproj> --school <id> --cycle <id>
                      [--term <id>] [--json]
       evara validate --project <path.evaraproj> --list

Runs pre-solve validation and prints what it found: inputs that refuse a solve,
limitations that are reported but do not block, and figures that merely disagree.

A fixture is built in memory. A project is read from disk and NOT modified: no
migration is run, no trigger is recreated, no metadata is stamped, and SQLite
itself refuses every write on the connection. Use --project --list first to see
the schools, cycles and terms a project holds, with the identifiers to pass back.

It does NOT report that a timetable is feasible. Nothing in Evara generates a
timetable yet, and a clean run only means no input could be proved impossible.

Options:
  --project, -p  A .evaraproj file to validate instead of a fixture.
  --school ID    Which school. Required with --project; never defaulted, because
                 a project holds several and picking one would scope the whole
                 report to an institution you did not name.
  --cycle ID     Which cycle\'s grid. Required with --project.
  --term ID      Which term. Omit for a grid materialised for the whole year.
  --seed N       Seed to generate a fixture from. Default 2026.
  --json         Print the structured form instead of prose.
  --list, -l     List the fixtures, or a project\'s schools, cycles and terms.
  --help, -h     Show this text.

Exit codes:
  0   nothing blocking was found
  1   blocking findings were found
  2   the command itself failed

Never generates a timetable and never writes to a project.
See docs/PRE-SOLVE-VALIDATION.md and docs/SCHEDULING-PROBLEM-ASSEMBLY.md.
";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use evara_core::calendar::WorkingDays;
    use evara_core::scheduling::fixtures::FixtureKind;
    use evara_db::{CycleInput, CycleKind, PROJECT_EXTENSION, SchoolInput, Workspace};
    use uuid::Uuid;

    use super::{DEFAULT_SEED, Outcome, Request, SchedulingSelection, wrap};

    fn parse(args: &[&str]) -> Result<Request, String> {
        let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        Request::parse(&owned)
    }

    #[test]
    fn a_bare_name_validates_at_the_default_seed() {
        assert_eq!(
            parse(&["tiny"]),
            Ok(Request::Validate {
                name: "tiny".to_owned(),
                seed: DEFAULT_SEED,
                json: false,
            })
        );
    }

    #[test]
    fn the_fixture_flag_and_the_bare_name_mean_the_same_thing() {
        // The brief suggests `--fixture tiny`; people will type `tiny`. Both work, and a test
        // pins that they agree rather than one quietly winning.
        assert_eq!(parse(&["--fixture", "tiny"]), parse(&["tiny"]));
    }

    #[test]
    fn the_seed_and_the_json_flag_are_read() {
        assert_eq!(
            parse(&["medium", "--seed", "7", "--json"]),
            Ok(Request::Validate {
                name: "medium".to_owned(),
                seed: 7,
                json: true,
            })
        );
    }

    #[test]
    fn a_mistyped_option_is_refused_rather_than_ignored() {
        let error = parse(&["tiny", "--sed", "7"]).expect_err("refused");
        assert!(error.contains("unknown option"));
    }

    #[test]
    fn a_missing_option_value_is_refused() {
        assert!(parse(&["tiny", "--seed"]).is_err());
        assert!(parse(&["--fixture"]).is_err());
    }

    #[test]
    fn no_name_is_refused_with_the_usage() {
        let error = parse(&[]).expect_err("refused");
        assert!(error.contains("name a fixture"));
    }

    #[test]
    fn two_names_are_refused() {
        assert!(parse(&["tiny", "medium"]).is_err());
    }

    #[test]
    fn list_and_help_short_circuit() {
        assert_eq!(parse(&["--list"]), Ok(Request::List));
        assert_eq!(parse(&["--help"]), Ok(Request::Help));
    }

    #[test]
    fn every_fixture_can_be_named() {
        for kind in FixtureKind::all() {
            assert_eq!(
                parse(&[kind.name()]),
                Ok(Request::Validate {
                    name: kind.name().to_owned(),
                    seed: DEFAULT_SEED,
                    json: false,
                })
            );
        }
    }

    #[test]
    fn the_usage_text_does_not_promise_feasibility() {
        // The one claim this command must never make.
        assert!(super::USAGE.contains("does NOT report that a timetable is feasible"));
        assert!(
            !super::USAGE
                .to_lowercase()
                .contains("guarantees a timetable")
        );
    }

    #[test]
    fn the_usage_text_documents_all_three_exit_codes() {
        for code in ["0", "1", "2"] {
            assert!(super::USAGE.contains(code));
        }
    }

    #[test]
    fn wrapping_keeps_words_whole() {
        let lines = wrap("one two three four five", 9);
        for line in &lines {
            assert!(line.chars().count() <= 9, "{line:?}");
        }
        assert_eq!(lines.join(" "), "one two three four five");
    }

    // ------------------------------------------------------------------ the project form

    fn an_id() -> Uuid {
        Uuid::parse_str("018f3a2c-7c4e-7a19-9f2b-0000000000ab").expect("valid")
    }

    #[test]
    fn a_project_needs_a_school_and_a_cycle() {
        // Never defaulted: a project holds several schools, and choosing one would scope the
        // whole report to an institution the user did not name.
        let error = parse(&["--project", "school.evaraproj"]).expect_err("refused");
        assert!(error.contains("--school"), "{error}");
        assert!(error.contains("--cycle"), "{error}");

        let error = parse(&[
            "--project",
            "school.evaraproj",
            "--school",
            &an_id().to_string(),
        ])
        .expect_err("refused");
        assert!(error.contains("--cycle"), "{error}");
    }

    #[test]
    fn a_project_selection_is_read_with_and_without_a_term() {
        let id = an_id().to_string();
        assert_eq!(
            parse(&[
                "--project",
                "school.evaraproj",
                "--school",
                &id,
                "--cycle",
                &id,
                "--term",
                &id,
            ]),
            Ok(Request::Project {
                project: PathBuf::from("school.evaraproj"),
                selection: SchedulingSelection {
                    school_id: an_id(),
                    cycle_id: an_id(),
                    term_id: Some(an_id()),
                },
                json: false,
            })
        );
        assert_eq!(
            parse(&[
                "--project",
                "school.evaraproj",
                "--school",
                &id,
                "--cycle",
                &id
            ]),
            Ok(Request::Project {
                project: PathBuf::from("school.evaraproj"),
                selection: SchedulingSelection {
                    school_id: an_id(),
                    cycle_id: an_id(),
                    term_id: None,
                },
                json: false,
            })
        );
    }

    #[test]
    fn a_project_and_a_list_asks_what_the_project_holds() {
        assert_eq!(
            parse(&["--project", "school.evaraproj", "--list"]),
            Ok(Request::Inventory {
                project: PathBuf::from("school.evaraproj"),
            })
        );
    }

    #[test]
    fn a_malformed_identifier_is_refused_rather_than_ignored() {
        let error = parse(&[
            "--project",
            "school.evaraproj",
            "--school",
            "not-a-uuid",
            "--cycle",
            &an_id().to_string(),
        ])
        .expect_err("refused");
        assert!(error.contains("is not an identifier"), "{error}");
    }

    #[test]
    fn a_selection_without_a_project_is_refused() {
        // Otherwise the command would validate a fixture while appearing to have been given
        // a school, which is the silently-wrong-subject failure this parser is strict about.
        let error = parse(&["tiny", "--school", &an_id().to_string()]).expect_err("refused");
        assert!(
            error.contains("only means something with --project"),
            "{error}"
        );
    }

    #[test]
    fn a_fixture_and_a_project_together_are_refused() {
        let id = an_id().to_string();
        let error = parse(&[
            "tiny",
            "--project",
            "school.evaraproj",
            "--school",
            &id,
            "--cycle",
            &id,
        ])
        .expect_err("refused");
        assert!(error.contains("name one"), "{error}");
    }

    #[test]
    fn the_usage_text_states_that_a_project_is_not_modified() {
        // The guarantee the command is only safe to point at a live school file because of.
        assert!(super::USAGE.contains("NOT modified"));
        assert!(super::USAGE.contains("never writes to a project"));
    }

    // ------------------------------------------------------- end to end, over a real file

    /// A temporary project holding one school with a cycle and nothing else.
    ///
    /// Deliberately minimal: what this proves is the command's path through a real file —
    /// the read-only open, the selection, the exit code — and the loader's own suite in
    /// `evara-db` is where a fully configured school is asserted against.
    fn a_project() -> (tempfile::TempDir, PathBuf, Uuid, Uuid) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let settings = dir.path().join("settings.sqlite");
        let mut workspace =
            Workspace::with_settings_at(&settings, "0.1.0-test").expect("workspace");
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
    fn validating_a_real_project_reports_the_missing_grid_and_exits_one() {
        let (_dir, path, school, cycle) = a_project();
        let outcome = super::run(&owned(&[
            "--project",
            &path.to_string_lossy(),
            "--school",
            &school.to_string(),
            "--cycle",
            &cycle.to_string(),
        ]))
        .expect("the command ran");
        // A blocking finding is not a command failure: the command did its job and the
        // answer was "no". The school has a cycle and no materialised grid.
        assert!(matches!(outcome, Outcome::Blocked));
    }

    #[test]
    fn listing_a_real_project_succeeds_and_blocks_nothing() {
        let (_dir, path, _school, _cycle) = a_project();
        let outcome = super::run(&owned(&["--project", &path.to_string_lossy(), "--list"]))
            .expect("the command ran");
        assert!(matches!(outcome, Outcome::Clean));
    }

    #[test]
    fn validating_a_real_project_as_json_also_runs() {
        let (_dir, path, school, cycle) = a_project();
        let outcome = super::run(&owned(&[
            "--project",
            &path.to_string_lossy(),
            "--school",
            &school.to_string(),
            "--cycle",
            &cycle.to_string(),
            "--json",
        ]))
        .expect("the command ran");
        assert!(matches!(outcome, Outcome::Blocked));
    }

    #[test]
    fn a_school_of_no_project_is_a_command_failure_and_not_a_finding() {
        // Exit code 2, not 1. The request could not be served, which is a different thing
        // from the school's inputs being wrong.
        let (_dir, path, _school, cycle) = a_project();
        let error = super::run(&owned(&[
            "--project",
            &path.to_string_lossy(),
            "--school",
            &an_id().to_string(),
            "--cycle",
            &cycle.to_string(),
        ]))
        .expect_err("the command failed");
        assert!(error.contains("cli.evaraproj"), "{error}");
        assert!(
            !error.contains("Temp"),
            "the message leaked a path: {error}"
        );
    }

    #[test]
    fn a_file_that_is_not_a_project_is_a_command_failure() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let stray = dir.path().join("notes.evaraproj");
        std::fs::write(&stray, b"not a database").expect("write");
        let id = an_id().to_string();
        let error = super::run(&owned(&[
            "--project",
            &stray.to_string_lossy(),
            "--school",
            &id,
            "--cycle",
            &id,
        ]))
        .expect_err("the command failed");
        assert!(error.contains("not an Evara project"), "{error}");
    }

    #[test]
    fn validating_a_real_project_writes_nothing_to_it() {
        // The read-only guarantee, from the command's side. `last_opened_with_app_version`
        // is what an ordinary open stamps, so an unchanged value proves the command did not
        // take that path.
        let (_dir, path, school, cycle) = a_project();
        let stamped = |path: &std::path::Path| -> Option<String> {
            evara_db::Database::open_for_inspection(path)
                .expect("inspect")
                .metadata(evara_db::meta::KEY_LAST_OPENED_WITH)
                .expect("read metadata")
        };
        let before = stamped(&path);

        for _ in 0..2 {
            let _ = super::run(&owned(&[
                "--project",
                &path.to_string_lossy(),
                "--school",
                &school.to_string(),
                "--cycle",
                &cycle.to_string(),
            ]))
            .expect("the command ran");
        }
        assert_eq!(before, stamped(&path), "the command stamped the project");
    }

    /// Owns a slice of arguments, which is what `run` takes.
    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }
}
