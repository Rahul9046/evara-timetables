//! Headless Evara.
//!
//! The CLI exists so that solving, benchmarking and project migration can run without a
//! window — in CI, in a test harness, or over a large fixture. It is the reason
//! `evara-solver` must never depend on Tauri.
//!
//! Phase 0 ships the binary and its help text only; subcommands arrive with the phases
//! that implement them.

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Subcommand, the roadmap phase that implements it, and what it will do.
const PLANNED: &[(&str, u8, &str)] = &[
    ("solve", 6, "solve a project and report its score"),
    ("bench", 6, "run fixtures against score baselines"),
    ("validate", 5, "score a project, list violations"),
    ("diagnose", 5, "report feasibility problems"),
    ("fixture", 3, "generate a synthetic school"),
    ("export", 9, "write a portable *.evara archive"),
    ("import", 9, "read a *.evara archive"),
    ("migrate", 1, "apply pending migrations"),
];

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => println!("evara {VERSION}"),
        Some(other) => {
            eprintln!("evara: `{other}` is not implemented yet\n");
            help();
            std::process::exit(2);
        }
        None => help(),
    }
}

fn help() {
    println!("evara {VERSION} — headless Evara Timetables");
    println!("\nUsage: evara <command> [options]");
    println!("\nNo command is implemented yet. Planned:\n");
    for (name, phase, blurb) in PLANNED {
        println!("  {name:<9} phase {phase:<2} {blurb}");
    }
    println!("\nSee docs/ROADMAP.md for the phase each command belongs to.");
}

#[cfg(test)]
mod tests {
    use super::PLANNED;

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
}
