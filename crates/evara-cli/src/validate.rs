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
//! # No database
//!
//! It builds a fixture in memory and validates it. It opens no SQLite database, reads no
//! `.evaraproj`, applies no migration and starts no solver. Until M4's loader exists there is
//! no way to obtain a `Problem` from a real project, which is why the only input is a fixture.
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

use evara_core::scheduling::diagnostics::{Diagnostic, Severity};
use evara_core::scheduling::fixtures::{FixtureKind, fixture};
use evara_core::scheduling::{Problem, ValidationSummary, validate};

/// The default seed, matching `evara fixture` so the two commands describe the same school.
const DEFAULT_SEED: u64 = 2026;

/// What the command decided, so `main` can choose an exit code without re-deriving it.
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
        Request::Validate { name, seed, json } => {
            let found = fixture(&name, seed).ok_or_else(|| {
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
    }
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
}

impl Request {
    /// Reads the arguments.
    ///
    /// Strict, for the reason `evara fixture` is strict: a silently ignored `--sed 7` would
    /// report on a different school from the one the user believed they asked about.
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut json = false;
        let mut rest = args.iter();

        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(Self::Help),
                "--list" | "-l" => return Ok(Self::List),
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

        match name {
            Some(name) => Ok(Self::Validate { name, seed, json }),
            None => Err(format!("name a fixture, or pass --list\n\n{USAGE}")),
        }
    }
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
    out.push('\n');
    out
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

Runs pre-solve validation over a synthetic scheduling fixture and prints what
it found: inputs that refuse a solve, limitations that are reported but do not
block, and figures that merely disagree.

It does NOT report that a timetable is feasible. Nothing in Evara generates a
timetable yet, and a clean run only means no input could be proved impossible.

Options:
  --seed N       Seed to generate the fixture from. Default 2026.
  --json         Print the structured form instead of prose.
  --list, -l     List the fixtures that can be validated.
  --help, -h     Show this text.

Exit codes:
  0   nothing blocking was found
  1   blocking findings were found
  2   the command itself failed

Reads no project, opens no database and applies no migration.
See docs/PRE-SOLVE-VALIDATION.md.
";

#[cfg(test)]
mod tests {
    use evara_core::scheduling::fixtures::FixtureKind;

    use super::{DEFAULT_SEED, Request, wrap};

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
}
