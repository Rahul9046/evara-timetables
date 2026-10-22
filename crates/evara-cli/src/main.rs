//! Headless Evara.
//!
//! The CLI exists so that solving, benchmarking and project migration can run without a
//! window — in CI, in a test harness, or over a large fixture. It is the reason
//! `evara-solver` must never depend on Tauri.
//!
//! Phase 0 shipped the binary and its help text only. Subcommands arrive with the phases
//! that implement them: [`fixture`] (M2) and [`validate`] (M3, extended in M4 to read a real
//! `.evaraproj` — read-only, migrating nothing).
//!
//! `validate` took the name an earlier `PLANNED` entry held. That entry described *scoring a
//! stored project and listing its violations*, which is M5's scorer and a different command,
//! so it keeps its phase under the name `score`. The planned `diagnose` is gone because
//! `validate` is it.

mod fixture;
mod validate;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Subcommand, the roadmap phase that implements it, and what it will do.
///
/// Implemented commands are **not** listed here; see [`IMPLEMENTED`].
const PLANNED: &[(&str, u8, &str)] = &[
    ("solve", 6, "solve a project and report its score"),
    ("bench", 6, "run fixtures against score baselines"),
    ("score", 5, "score a stored project and list its violations"),
    ("export", 9, "write a portable *.evara archive"),
    ("import", 9, "read a *.evara archive"),
    ("migrate", 1, "apply pending migrations"),
];

/// Subcommand and what it does, for the commands that exist.
const IMPLEMENTED: &[(&str, &str)] = &[
    (
        "fixture",
        "write a synthetic scheduling problem as JSON (test data, not a timetable)",
    ),
    (
        "validate",
        "validate a fixture or a project's inputs (reports inputs, not a timetable)",
    ),
];

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("evara {VERSION}");
            std::process::ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            help();
            std::process::ExitCode::SUCCESS
        }
        Some("fixture") => match fixture::run(&args[1..]) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("evara fixture: {message}");
                std::process::ExitCode::from(2)
            }
        },
        // Three outcomes, three codes. A blocking finding is not a command failure — the
        // command did its job and the answer was "no" — so they cannot share an exit code.
        Some("validate") => match validate::run(&args[1..]) {
            Ok(validate::Outcome::Clean) => std::process::ExitCode::SUCCESS,
            Ok(validate::Outcome::Blocked) => std::process::ExitCode::from(1),
            Err(message) => {
                eprintln!("evara validate: {message}");
                std::process::ExitCode::from(2)
            }
        },
        Some(other) => {
            eprintln!("evara: `{other}` is not implemented yet\n");
            help();
            std::process::ExitCode::from(2)
        }
    }
}

fn help() {
    println!("evara {VERSION} — headless Evara Timetables");
    println!("\nUsage: evara <command> [options]");
    println!("\nAvailable:\n");
    for (name, blurb) in IMPLEMENTED {
        println!("  {name:<9}          {blurb}");
    }
    println!("\nPlanned:\n");
    for (name, phase, blurb) in PLANNED {
        println!("  {name:<9} phase {phase:<2} {blurb}");
    }
    println!("\nSee docs/ROADMAP.md for the phase each command belongs to,");
    println!("docs/SCHEDULING-FIXTURES.md for what `fixture` generates,");
    println!("docs/PRE-SOLVE-VALIDATION.md for what `validate` checks, and");
    println!("docs/SCHEDULING-PROBLEM-ASSEMBLY.md for how a project becomes a problem.");
}

#[cfg(test)]
mod tests {
    use super::{IMPLEMENTED, PLANNED};

    #[test]
    fn planned_commands_are_unique() {
        let mut names: Vec<&str> = PLANNED.iter().map(|(n, _, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate subcommand name in PLANNED");
    }

    #[test]
    fn planned_commands_are_documented() {
        for (name, phase, blurb) in PLANNED {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "subcommand {name:?} should be lowercase kebab-case"
            );
            assert!(
                (1..=11).contains(phase),
                "{name} names roadmap phase {phase}, which does not exist"
            );
            assert!(blurb.len() > 10, "{name} needs a real description");
        }
    }

    #[test]
    fn a_command_is_either_planned_or_implemented_and_never_both() {
        // Otherwise the help text would list it twice, once as missing and once as present.
        for (implemented, _) in IMPLEMENTED {
            assert!(
                !PLANNED.iter().any(|(planned, _, _)| planned == implemented),
                "{implemented} is listed as both planned and implemented"
            );
        }
    }

    #[test]
    fn implemented_commands_are_documented() {
        for (name, blurb) in IMPLEMENTED {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "subcommand {name:?} should be lowercase kebab-case"
            );
            assert!(blurb.len() > 10, "{name} needs a real description");
        }
    }

    #[test]
    fn the_validate_command_does_not_describe_itself_as_producing_a_timetable() {
        // It validates inputs. Nothing in Evara produces a timetable yet, and the help text
        // is the first place that claim could leak in.
        let (_, blurb) = IMPLEMENTED
            .iter()
            .find(|(name, _)| *name == "validate")
            .expect("validate is implemented");
        assert!(blurb.contains("not a timetable"));
    }

    #[test]
    fn the_fixture_command_does_not_describe_itself_as_generating_a_timetable() {
        // It generates *input* to a solver that does not exist yet. A blurb that implied
        // otherwise would be the product claiming a capability it does not have.
        let (_, blurb) = IMPLEMENTED
            .iter()
            .find(|(name, _)| *name == "fixture")
            .expect("fixture is implemented");
        assert!(blurb.contains("synthetic"));
        assert!(blurb.contains("not a timetable"));
    }
}
