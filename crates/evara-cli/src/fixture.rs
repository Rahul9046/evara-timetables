//! `evara fixture` — write a synthetic scheduling problem as JSON.
//!
//! # What this generates, and what it does not
//!
//! It generates **synthetic test data**: a school that does not exist, with invented names,
//! produced deterministically from a seed. It does **not** generate a timetable. Nothing in
//! this workspace generates a timetable yet — there is no solver — and a command that implied
//! otherwise would be the product claiming a capability it does not have.
//!
//! The output is the *input* to a solver: a `Problem` plus the conclusions later milestones
//! must reach about it. See `docs/SCHEDULING-FIXTURES.md`.
//!
//! # Why a command at all, when the generator is the fixture
//!
//! The fixtures live in `evara_core::scheduling::fixtures` and are a pure function of a
//! seed, so every test calls the generator directly and nothing in the repository reads a
//! checked-in JSON file. This command exists for the three things a library call cannot do:
//! let somebody **look** at a fixture, hand one to a benchmark harness or an external tool,
//! and diff two seeds to see what the seed actually changes.
//!
//! It reads no project, opens no database, applies no migration and touches nothing the user
//! already has unless they name an output directory.
//!
//! # Argument parsing, by hand
//!
//! No `clap`. Four options over one subcommand is not worth a dependency, a licence review
//! and a compile-time cost in a binary whose other seven commands do not exist yet. When the
//! second real command arrives, that trade is worth revisiting.

use std::fmt::Write as _;
use std::path::PathBuf;

use evara_core::scheduling::fixtures::{FixtureKind, SchedulingFixture, fixture};

/// The default seed, so `evara fixture tiny` is reproducible with no arguments.
///
/// Any fixed value would do; this one is the year the milestone was written and is not
/// meaningful to anything.
const DEFAULT_SEED: u64 = 2026;

/// Runs the subcommand.
///
/// `args` is everything after `fixture`.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let request = Request::parse(args)?;
    match request {
        Request::List => {
            print!("{}", listing());
            Ok(())
        }
        Request::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Request::Generate { name, seed, out } => generate(&name, seed, out.as_deref()),
    }
}

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// List the fixtures and what each is for.
    List,
    /// Print the usage text.
    Help,
    /// Generate one fixture.
    Generate {
        /// Which fixture.
        name: String,
        /// The seed.
        seed: u64,
        /// Where to write it, or `None` to write to standard output.
        out: Option<PathBuf>,
    },
}

impl Request {
    /// Reads the arguments.
    ///
    /// Strict: an unrecognised option is an error rather than something to ignore, because a
    /// mistyped `--sed 7` that silently generated seed 2026 would produce a file the user
    /// believed was something else.
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut out: Option<PathBuf> = None;
        let mut rest = args.iter();

        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--list" | "-l" => return Ok(Self::List),
                "--help" | "-h" => return Ok(Self::Help),
                "--seed" => {
                    let value = rest.next().ok_or("--seed needs a number")?;
                    seed = value
                        .parse()
                        .map_err(|_| format!("--seed {value:?} is not a number"))?;
                }
                "--out" => {
                    let value = rest.next().ok_or("--out needs a directory")?;
                    out = Some(PathBuf::from(value));
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
            Some(name) => Ok(Self::Generate { name, seed, out }),
            None => Err(format!("name a fixture, or pass --list\n\n{USAGE}")),
        }
    }
}

/// Generates one fixture and writes it where the request said.
fn generate(name: &str, seed: u64, out: Option<&std::path::Path>) -> Result<(), String> {
    let generated = fixture(name, seed)
        .ok_or_else(|| format!("no fixture is called {name:?}\n\n{}", listing()))?;

    let json = serde_json::to_string_pretty(&generated)
        .map_err(|error| format!("could not serialise {name}: {error}"))?;

    match out {
        None => {
            println!("{json}");
            Ok(())
        }
        Some(directory) => {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
            let path = directory.join(format!("{name}-{seed}.json"));
            std::fs::write(&path, &json)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
            eprintln!("{}", summary(&generated, &path));
            Ok(())
        }
    }
}

/// One line about what was written, on standard error so it never contaminates the JSON.
fn summary(generated: &SchedulingFixture, path: &std::path::Path) -> String {
    let activities = generated.activity_count();
    let problems = generated.problems.len();
    format!(
        "wrote {} — {problems} problem{}, {activities} occurrence{} of synthetic scheduling \
         data (not a timetable)",
        path.display(),
        if problems == 1 { "" } else { "s" },
        if activities == 1 { "" } else { "s" },
    )
}

/// Every fixture and what it is for.
fn listing() -> String {
    let mut text = String::from("Fixtures (synthetic scheduling data, generated from a seed):\n\n");
    for kind in FixtureKind::all() {
        let _ = writeln!(text, "  {}", kind.name());
        for line in wrap(kind.purpose(), 72) {
            let _ = writeln!(text, "      {line}");
        }
    }
    text
}

/// Breaks prose into lines of at most `width` characters, on word boundaries.
///
/// A terminal is not a document, and a 300-character purpose on one line is unreadable. Hand
/// rolled for the same reason the argument parsing is.
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
Usage: evara fixture <name> [--seed N] [--out DIR]
       evara fixture --list

Writes a synthetic scheduling problem as JSON: invented schools, invented
people, generated deterministically from a seed. This is test data for the
milestones that build the solver — it is not a timetable, and nothing in
Evara generates one yet.

Options:
  --seed N     Seed to generate from. Default 2026. The same seed always
               produces the same fixture, on every platform.
  --out DIR    Write <name>-<seed>.json into DIR, creating it if needed.
               Without this, the JSON goes to standard output.
  --list, -l   List the fixtures and what each one proves.
  --help, -h   Show this text.

Reads no project, opens no database and applies no migration.
See docs/SCHEDULING-FIXTURES.md.
";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use evara_core::scheduling::fixtures::FixtureKind;

    use super::{DEFAULT_SEED, Request, listing, wrap};

    fn parse(args: &[&str]) -> Result<Request, String> {
        let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        Request::parse(&owned)
    }

    #[test]
    fn a_bare_name_generates_at_the_default_seed() {
        assert_eq!(
            parse(&["tiny"]),
            Ok(Request::Generate {
                name: "tiny".to_owned(),
                seed: DEFAULT_SEED,
                out: None,
            })
        );
    }

    #[test]
    fn the_seed_and_the_output_directory_are_read() {
        assert_eq!(
            parse(&["medium", "--seed", "7", "--out", "build/fixtures"]),
            Ok(Request::Generate {
                name: "medium".to_owned(),
                seed: 7,
                out: Some(PathBuf::from("build/fixtures")),
            })
        );
    }

    #[test]
    fn options_may_come_before_the_name() {
        assert_eq!(
            parse(&["--seed", "3", "tiny"]),
            Ok(Request::Generate {
                name: "tiny".to_owned(),
                seed: 3,
                out: None,
            })
        );
    }

    #[test]
    fn a_mistyped_option_is_refused_rather_than_ignored() {
        // A silently ignored `--sed 7` would write a file the user believed was seed 7.
        let error = parse(&["tiny", "--sed", "7"]).expect_err("refused");
        assert!(error.contains("unknown option"));
    }

    #[test]
    fn a_seed_that_is_not_a_number_is_refused() {
        let error = parse(&["tiny", "--seed", "soon"]).expect_err("refused");
        assert!(error.contains("not a number"));
    }

    #[test]
    fn a_missing_option_value_is_refused() {
        assert!(parse(&["tiny", "--seed"]).is_err());
        assert!(parse(&["tiny", "--out"]).is_err());
    }

    #[test]
    fn no_name_and_no_list_is_refused_with_the_usage() {
        let error = parse(&[]).expect_err("refused");
        assert!(error.contains("name a fixture"));
        assert!(error.contains("--list"));
    }

    #[test]
    fn two_names_are_refused() {
        let error = parse(&["tiny", "medium"]).expect_err("refused");
        assert!(error.contains("one fixture at a time"));
    }

    #[test]
    fn list_and_help_short_circuit() {
        assert_eq!(parse(&["--list"]), Ok(Request::List));
        assert_eq!(parse(&["-l"]), Ok(Request::List));
        assert_eq!(parse(&["--help"]), Ok(Request::Help));
        assert_eq!(parse(&["tiny", "--list"]), Ok(Request::List));
    }

    #[test]
    fn the_listing_names_every_fixture() {
        let text = listing();
        for kind in FixtureKind::all() {
            assert!(text.contains(kind.name()), "{} is not listed", kind.name());
        }
    }

    #[test]
    fn the_listing_does_not_promise_a_timetable() {
        let text = listing();
        assert!(text.contains("synthetic"));
        assert!(
            !text.to_lowercase().contains("generates a timetable"),
            "fixture generation is not timetable generation"
        );
    }

    #[test]
    fn wrapping_keeps_words_whole_and_respects_the_width() {
        let lines = wrap("one two three four five six seven eight nine ten", 12);
        for line in &lines {
            assert!(line.chars().count() <= 12, "{line:?} is too long");
        }
        assert_eq!(
            lines.join(" "),
            "one two three four five six seven eight nine ten"
        );
    }

    #[test]
    fn wrapping_a_single_long_word_does_not_lose_it() {
        let lines = wrap("unsplittable", 4);
        assert_eq!(lines, vec!["unsplittable".to_owned()]);
    }

    #[test]
    fn wrapping_nothing_produces_nothing() {
        assert_eq!(wrap("", 10), Vec::<String>::new());
    }
}
