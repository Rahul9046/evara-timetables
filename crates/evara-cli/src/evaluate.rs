//! `evara evaluate` — score a candidate arrangement against the constraint engine.
//!
//! # What this reports, and what it never claims
//!
//! It reports what the eight implemented rules find in an arrangement it was **given**. It
//! does not search, does not place anything, and does not improve anything: there is no
//! solver, and a command that implied otherwise would be the product claiming a capability it
//! does not have.
//!
//! The strongest thing a clean run says is *"nothing hard is broken"*, and even then it says
//! separately whether every occurrence is placed. A usable partial timetable is still partial
//! — `evara_core::lessons`' rule binds this command too:
//!
//! > Never report that a demand *can* be scheduled.
//!
//! # Two inputs, and what each has to evaluate
//!
//! A **fixture** is built in memory. Where it ships a known-feasible arrangement, that is what
//! is evaluated, and finding nothing hard in it is the sharpest check in the suite — M2 built
//! that arrangement against the ADRs and verified it independently. Where it ships none, the
//! empty arrangement is evaluated and the report says so.
//!
//! A **project** is read from disk through
//! [`Database::open_for_inspection`](evara_db::Database::open_for_inspection), which runs no
//! migration and whose connection SQLite refuses every write through. There is no stored
//! timetable to score — `timetable` arrives in M7 — so what is evaluated is the **empty**
//! arrangement, and the useful half of the answer is the one above the findings: the rules
//! this school will actually be judged by, compiled from its own `constraint_instance` rows on
//! top of the mandatory set. That is the whole configuration path exercised against a real
//! file, which is what `evara score` will need and cannot have until there is a timetable to
//! score.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0 | Evaluation completed and nothing hard is broken |
//! | 1 | Evaluation completed and a hard rule is broken |
//! | 2 | The command itself failed — an unknown fixture, a bad option, a configuration that does not compile |
//!
//! A medium violation leaves the code at 0. `activity-scheduled` is MEDIUM precisely so that
//! an unplaced lesson makes a timetable **incomplete** rather than unusable, and an exit code
//! that could not tell the two apart would undo that distinction at the last step.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use evara_constraints::{Completeness, EvaluationReport, Registry, evaluate, evaluator_for};
use evara_core::scheduling::fixtures::{FixtureKind, fixture};
use evara_core::scheduling::{
    Assignment, CompiledConstraint, ConstraintLevel, Problem, StoredConstraint,
};
use evara_db::{Database, SchedulingSelection};
use uuid::Uuid;

/// The default seed, matching `evara fixture` and `evara validate`.
const DEFAULT_SEED: u64 = 2026;

/// What the command decided, so `main` can choose an exit code without re-deriving it.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Nothing hard is broken.
    Usable,
    /// A hard rule is broken.
    Broken,
}

/// Runs the subcommand.
///
/// `args` is everything after `evaluate`.
pub(crate) fn run(args: &[String]) -> Result<Outcome, String> {
    match Request::parse(args)? {
        Request::Help => {
            print!("{USAGE}");
            Ok(Outcome::Usable)
        }
        Request::List => {
            for kind in FixtureKind::all() {
                println!("  {}", kind.name());
            }
            Ok(Outcome::Usable)
        }
        Request::Fixture { name, seed, json } => evaluate_fixture(&name, seed, json),
        Request::Project {
            project,
            selection,
            json,
        } => evaluate_project(&project, &selection, json),
        Request::StoredTimetable {
            project,
            timetable,
            json,
        } => evaluate_stored(&project, timetable, json),
    }
}

/// Evaluates one synthetic fixture's own arrangement, in memory.
fn evaluate_fixture(name: &str, seed: u64, json: bool) -> Result<Outcome, String> {
    let found = fixture(name, seed).ok_or_else(|| {
        format!("no fixture is called {name:?}. Run `evara evaluate --list` to see them.")
    })?;

    let rules = Registry::mandatory();
    let mut broken = false;
    for (n, problem) in found.problems.iter().enumerate() {
        // A fixture's arrangement names which problem it is for, and only one of a
        // multi-school fixture's problems has one.
        let arrangement = found
            .expectations
            .feasible
            .as_ref()
            .filter(|known| known.problem == n)
            .map(|known| known.assignment(problem.activities.len()));
        let source = if arrangement.is_some() {
            Source::KnownFeasible
        } else {
            Source::Empty
        };
        let assignment = arrangement.unwrap_or_else(|| Assignment::empty(problem.activities.len()));

        let report = evaluate(problem, &assignment, &rules);
        broken |= !report.is_usable();

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
            print!("{}", as_json(&label, problem, source, &rules, &report));
        } else {
            print!("{}", as_text(&label, problem, source, &rules, &report));
        }
    }

    Ok(if broken {
        Outcome::Broken
    } else {
        Outcome::Usable
    })
}

/// Compiles one school of a real project and evaluates the empty arrangement over it.
///
/// Read-only throughout, by the same route `evara validate --project` takes and for the same
/// reason: a command asked to look at somebody's live school file must not change it.
fn evaluate_project(
    path: &Path,
    selection: &SchedulingSelection,
    json: bool,
) -> Result<Outcome, String> {
    let mut db = Database::open_for_inspection(path).map_err(|error| describe(path, &error))?;

    // The school's own configuration, on top of the mandatory set. A row that cannot be
    // compiled fails the command rather than being skipped: a school that configured a rule is
    // entitled to know this build cannot run it, and an evaluation that silently dropped it
    // would report a score for rules nobody asked for.
    let stored = db
        .constraints()
        .stored_for(selection.school_id)
        .map_err(|error| describe(path, &error))?;
    let rules = Registry::compile(&stored, selection.term_id).map_err(|error| {
        format!(
            "{} — this school's constraint configuration cannot be read: {error}",
            file_name(path)
        )
    })?;

    let assembly = db
        .scheduling()
        .load(selection)
        .map_err(|error| describe(path, &error))?;
    let integrity = assembly.integrity();
    if !integrity.is_empty() {
        // An internal inconsistency is a defect in Evara, not an input a user can correct, and
        // scoring a problem that refers to rows it does not hold would produce a number about
        // nothing. ADR 0036 §4.
        return Err(format!(
            "{} — the loaded problem refers to {} thing{} it does not contain. This is a \
             defect in Evara, not in your data; please report it.",
            file_name(path),
            integrity.len(),
            plural(integrity.len())
        ));
    }

    let assignment = Assignment::empty(assembly.problem.activities.len());
    let report = evaluate(&assembly.problem, &assignment, &rules);

    if json {
        print!(
            "{}",
            project_json(path, selection, &assembly.problem, &stored, &rules, &report)
        );
    } else {
        print!(
            "{}",
            project_text(path, selection, &assembly.problem, &rules, &report)
        );
    }
    Ok(if report.is_usable() {
        Outcome::Usable
    } else {
        Outcome::Broken
    })
}

/// Scores a **stored** timetable against the inputs as they now stand. Reads; never writes.
///
/// # What a changed input looks like here
///
/// A stored timetable references activities, timeslots and rooms that `RESTRICT` guarantees
/// still exist — but their *facts* may have moved since it was generated. An availability rule
/// added over a placed slot, a room made unbookable, a period's times changed: each shows up as
/// a violation here and in nothing else, which is why re-scoring is the honest way to ask
/// whether a stored timetable is still usable rather than a flag on the row.
///
/// Placements naming a lesson the problem no longer holds are **reported, not refused**: a
/// timetable whose lessons were deleted is still worth opening, and a reader that refused would
/// leave the user unable to see what to fix.
fn evaluate_stored(path: &Path, timetable_id: Uuid, json: bool) -> Result<Outcome, String> {
    let mut db = Database::open_for_inspection(path).map_err(|error| describe(path, &error))?;

    let timetable = db
        .timetables()
        .get(timetable_id)
        .map_err(|error| describe(path, &error))?;
    // The school is the cycle's: a timetable carries no `school_id` (ADR 0042 D6).
    let school_id = db
        .time_model()
        .cycle(timetable.cycle_id)
        .map_err(|error| describe(path, &error))?
        .map(|cycle| cycle.school_id)
        .ok_or_else(|| {
            format!(
                "{} — this timetable names a cycle the project no longer holds",
                file_name(path)
            )
        })?;
    let selection = SchedulingSelection {
        school_id,
        cycle_id: timetable.cycle_id,
        term_id: timetable.term_id,
    };

    let stored_rules = db
        .constraints()
        .stored_for(school_id)
        .map_err(|error| describe(path, &error))?;
    let rules = Registry::compile(&stored_rules, selection.term_id).map_err(|error| {
        format!(
            "{} — this school's constraint configuration cannot be read: {error}",
            file_name(path)
        )
    })?;

    let assembly = db
        .scheduling()
        .load(&selection)
        .map_err(|error| describe(path, &error))?;
    let arrangement = db
        .timetables()
        .arrangement(timetable_id, &assembly)
        .map_err(|error| describe(path, &error))?;
    let report = evaluate(&assembly.problem, &arrangement.assignment, &rules);

    if json {
        print!(
            "{}",
            render(&serde_json::json!({
                "project": file_name(path),
                "timetable": { "id": timetable_id, "name": timetable.name },
                "arrangement": "the stored timetable",
                "storedEntries": arrangement.entries,
                "entriesNotInProblem": arrangement.not_in_problem,
                "isReadable": report.is_readable(),
                "isUsable": report.is_usable(),
                "isAdoptable": report.is_adoptable(),
                "score": report.score,
                "completeness": report.completeness,
                "unverified": timetable.unverified(),
                "rules": rules,
                "violations": report.violations,
                "explanations": report.explanations(&assembly.problem),
                "stored": false,
            }))
        );
    } else {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} — stored timetable \"{}\"",
            file_name(path),
            timetable.name
        );
        let _ = writeln!(
            out,
            "  {} stored entr{} read, scored against the inputs as they now stand",
            arrangement.entries,
            if arrangement.entries == 1 { "y" } else { "ies" }
        );
        if !arrangement.not_in_problem.is_empty() {
            let _ = writeln!(
                out,
                "  {} stored placement(s) name a lesson this problem no longer holds, so they \
                 were not scored",
                arrangement.not_in_problem.len()
            );
        }
        verdict(&rules, &report, &mut out);
        findings(&assembly.problem, &report, &mut out);
        let _ = writeln!(
            out,
            "\n  Nothing was written. `evara save --revalidate {timetable_id}` stores this \
             scoring as the timetable's current findings."
        );
        out.push('\n');
        print!("{out}");
    }

    Ok(if report.is_usable() {
        Outcome::Usable
    } else {
        Outcome::Broken
    })
}

/// Where the arrangement being evaluated came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The fixture's own known-feasible arrangement.
    KnownFeasible,
    /// Nothing placed.
    Empty,
}

impl Source {
    const fn blurb(self) -> &'static str {
        match self {
            Self::KnownFeasible => "the fixture's own known-feasible arrangement",
            Self::Empty => "the empty arrangement — nothing is placed",
        }
    }
}

/// The human-readable report for a fixture.
fn as_text(
    label: &str,
    problem: &Problem,
    source: Source,
    rules: &[CompiledConstraint],
    report: &EvaluationReport,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{label} — {}", problem.context.school.name);
    let _ = writeln!(out, "  evaluating {}", source.blurb());
    verdict(rules, report, &mut out);
    findings(problem, report, &mut out);
    out.push('\n');
    out
}

/// The human-readable report for a project.
fn project_text(
    path: &Path,
    selection: &SchedulingSelection,
    problem: &Problem,
    rules: &[CompiledConstraint],
    report: &EvaluationReport,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} — {}", file_name(path), problem.context.school.name);
    let _ = writeln!(out, "  school {}", selection.school_id);
    let _ = writeln!(out, "  cycle  {}", selection.cycle_id);
    let _ = match selection.term_id {
        Some(term) => writeln!(out, "  term   {term}"),
        None => writeln!(out, "  term   (none — a grid for the whole year)"),
    };
    let _ = writeln!(
        out,
        "  evaluating {} — there is no stored timetable to score until M7",
        Source::Empty.blurb()
    );
    verdict(rules, report, &mut out);
    findings(problem, report, &mut out);
    out.push('\n');
    out
}

/// The three sentences every report ends up making, in one place.
///
/// Shared so the fixture and project forms cannot phrase the same verdict two ways — the same
/// reason `validate`'s `render_findings` is shared.
fn verdict(rules: &[CompiledConstraint], report: &EvaluationReport, out: &mut String) {
    let _ = writeln!(
        out,
        "  {} rule{} evaluated: {}",
        rules.len(),
        plural(rules.len()),
        describe_rules(rules)
    );

    if !report.is_readable() {
        // Never scored. A number about activities this problem does not hold looks like an
        // answer, which is worse than silence.
        let _ = writeln!(
            out,
            "  The arrangement does not describe this problem, so nothing was scored:"
        );
        for reference in &report.invalid_references {
            let _ = writeln!(out, "    {reference}");
        }
        return;
    }

    let _ = writeln!(
        out,
        "  score {} hard, {} medium, {} soft",
        report.score.hard, report.score.medium, report.score.soft
    );

    if report.is_usable() {
        // "Usable" and never "feasible": no hard rule is broken, which is a different and
        // much weaker statement than a timetable existing.
        let _ = writeln!(
            out,
            "  No hard rule is broken. This is not a promise that a complete timetable exists."
        );
    } else {
        let _ = writeln!(
            out,
            "  {} hard violation{} — this arrangement is not usable.",
            report.count_at(ConstraintLevel::Hard),
            plural(report.count_at(ConstraintLevel::Hard))
        );
    }

    match report.completeness {
        Completeness::Complete => {
            let _ = writeln!(out, "  Every occurrence is placed.");
        }
        Completeness::Partial { placed, required } => {
            let _ = writeln!(
                out,
                "  {placed} of {required} occurrence{} placed — incomplete, which is not the \
                 same as unusable.",
                plural(required)
            );
        }
    }
}

/// Names the configured rules, so a reader can see what was and was not applied.
fn describe_rules(rules: &[CompiledConstraint]) -> String {
    let tuned = rules.iter().filter(|rule| rule.id.is_some()).count();
    if tuned == 0 {
        return String::from("the shipped set, none of them configured by this school");
    }
    let mut named: Vec<&str> = rules
        .iter()
        .filter(|rule| rule.id.is_some())
        .map(|rule| rule.constraint_type.key())
        .collect();
    named.sort_unstable();
    format!(
        "{tuned} tuned by this school ({}), the rest at their shipped levels",
        named.join(", ")
    )
}

/// Writes the violations, hard first, grouped by rule.
///
/// Each line is the evaluator's own [`explain`](evara_constraints::ConstraintEvaluator::explain)
/// output, so the wording lives with the rule rather than being restated here — which is what
/// keeps a message from drifting from the check that produced it.
fn findings(problem: &Problem, report: &EvaluationReport, out: &mut String) {
    if !report.is_readable() || report.violations.is_empty() {
        return;
    }
    for level in [
        ConstraintLevel::Hard,
        ConstraintLevel::Medium,
        ConstraintLevel::Soft,
    ] {
        let at_level: Vec<_> = report.at_level(level).collect();
        if at_level.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n  {}:", heading(level));
        for violation in at_level {
            let _ = writeln!(
                out,
                "    [{}] {}",
                violation.constraint_type.key(),
                violation.subject.label
            );
            let explained = evaluator_for(violation.constraint_type)
                .explain(problem, violation)
                .message;
            for line in wrap(&explained, 76) {
                let _ = writeln!(out, "        {line}");
            }
        }
    }
}

/// The structured form, built from the engine's own types rather than a second schema.
fn as_json(
    label: &str,
    problem: &Problem,
    source: Source,
    rules: &[CompiledConstraint],
    report: &EvaluationReport,
) -> String {
    let wrapper = serde_json::json!({
        "fixture": label,
        "school": problem.context.school.name,
        "arrangement": source.blurb(),
        "isReadable": report.is_readable(),
        "isUsable": report.is_usable(),
        "isAdoptable": report.is_adoptable(),
        "score": report.score,
        "completeness": report.completeness,
        "invalidReferences": report.invalid_references,
        "rules": rules,
        "violations": report.violations,
        "explanations": report.explanations(problem),
    });
    render(&wrapper)
}

/// The structured form for a project, in the same vocabulary as the fixture form.
fn project_json(
    path: &Path,
    selection: &SchedulingSelection,
    problem: &Problem,
    stored: &[StoredConstraint],
    rules: &[CompiledConstraint],
    report: &EvaluationReport,
) -> String {
    let wrapper = serde_json::json!({
        "project": file_name(path),
        "school": {
            "id": selection.school_id,
            "name": problem.context.school.name,
        },
        "cycleId": selection.cycle_id,
        "termId": selection.term_id,
        "arrangement": Source::Empty.blurb(),
        "isReadable": report.is_readable(),
        "isUsable": report.is_usable(),
        "isAdoptable": report.is_adoptable(),
        "score": report.score,
        "completeness": report.completeness,
        "configured": stored,
        "rules": rules,
        "violations": report.violations,
        "explanations": report.explanations(problem),
    });
    render(&wrapper)
}

/// Serialises a wrapper, reporting a serialisation failure rather than panicking on one.
fn render(wrapper: &serde_json::Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(wrapper)
            .unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
    )
}

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// Print the usage text.
    Help,
    /// List the fixtures that can be evaluated.
    List,
    /// Evaluate one fixture.
    Fixture {
        /// Which fixture.
        name: String,
        /// The seed to generate it from.
        seed: u64,
        /// Whether to print the structured form.
        json: bool,
    },
    /// Evaluate one school of a project.
    Project {
        /// The project file.
        project: PathBuf,
        /// Which school, cycle and term.
        selection: SchedulingSelection,
        /// Whether to print the structured form.
        json: bool,
    },
    /// Evaluate a **stored** timetable of a project.
    ///
    /// The selection is the timetable's own: a stored timetable knows its cycle and term, and
    /// taking them from the command line would let the two disagree.
    StoredTimetable {
        /// The project file.
        project: PathBuf,
        /// Which timetable.
        timetable: Uuid,
        /// Whether to print the structured form.
        json: bool,
    },
}

impl Request {
    /// Reads the arguments.
    ///
    /// The same grammar as `evara validate`, deliberately: the two commands take the same
    /// inputs and a reader who has learnt one should not have to learn the other. `--school`
    /// and `--cycle` are required beside `--project` for the reason they are there — a project
    /// has held several schools since `V12`, and choosing one would scope the whole report to
    /// an institution the user did not name.
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut json = false;
        let mut list = false;
        let mut project: Option<PathBuf> = None;
        let mut school: Option<Uuid> = None;
        let mut cycle: Option<Uuid> = None;
        let mut term: Option<Uuid> = None;
        let mut timetable: Option<Uuid> = None;
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
                "--fixture" => {
                    let value = rest.next().ok_or("--fixture needs a name")?;
                    name = Some(value.clone());
                }
                "--project" | "-p" => {
                    let value = rest.next().ok_or("--project needs a path")?;
                    project = Some(PathBuf::from(value));
                }
                "--timetable" => timetable = Some(identifier("--timetable", rest.next())?),
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
                    "a fixture and a project are two different things to evaluate; name one",
                ));
            }
            if let Some(timetable) = timetable {
                refuse_selection_beside_a_timetable(school, cycle, term)?;
                return Ok(Self::StoredTimetable {
                    project,
                    timetable,
                    json,
                });
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
            ("--timetable", timetable.is_some()),
        ] {
            if given {
                return Err(format!("{flag} only means something with --project"));
            }
        }
        if list {
            return Ok(Self::List);
        }

        match name {
            Some(name) => Ok(Self::Fixture { name, seed, json }),
            None => Err(format!(
                "name a fixture, or pass --project <path>, or --list\n\n{USAGE}"
            )),
        }
    }
}

/// Refuses a selection given beside `--timetable`.
///
/// Rather than ignoring it. A stored timetable knows its own cycle and term, so accepting a
/// selection would let the two disagree about which grid is being scored -- and a user would
/// have no way to tell which one won.
fn refuse_selection_beside_a_timetable(
    school: Option<Uuid>,
    cycle: Option<Uuid>,
    term: Option<Uuid>,
) -> Result<(), String> {
    for (flag, given) in [
        ("--school", school.is_some()),
        ("--cycle", cycle.is_some()),
        ("--term", term.is_some()),
    ] {
        if given {
            return Err(format!(
                "{flag} means nothing with --timetable: a stored timetable already knows its                  school, cycle and term"
            ));
        }
    }
    Ok(())
}

/// Reads one identifier argument.
fn identifier(flag: &str, value: Option<&String>) -> Result<Uuid, String> {
    let value = value.ok_or_else(|| format!("{flag} needs an identifier"))?;
    Uuid::parse_str(value).map_err(|_| format!("{flag} {value:?} is not an identifier"))
}

/// Turns a database failure into a sentence, keeping the file name and not the path.
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

/// What to call a level in a printed report.
const fn heading(level: ConstraintLevel) -> &'static str {
    match level {
        ConstraintLevel::Hard => "Hard — this arrangement is not usable",
        ConstraintLevel::Medium => "Medium — the timetable is incomplete, not unusable",
        ConstraintLevel::Soft => "Soft — preferences",
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
Usage: evara evaluate <fixture> [--seed N] [--json]
       evara evaluate --fixture <fixture> [--seed N] [--json]
       evara evaluate --list
       evara evaluate --project <path.evaraproj> --school <id> --cycle <id>
                      [--term <id>] [--json]
       evara evaluate --project <path.evaraproj> --timetable <id> [--json]

Evaluates an arrangement against the eight implemented constraint types and prints
what they found: hard violations, which make an arrangement unusable, and medium
ones, which make a timetable incomplete.

A fixture is evaluated against its own known-feasible arrangement where it ships
one, and against the empty arrangement otherwise.

With --timetable, a STORED timetable is scored against the school's inputs as they
now stand -- which is how to find out whether a timetable generated last week is
still usable, because an availability rule or a changed period breaks nothing
referential and shows up nowhere else.

With --school and --cycle and no --timetable, the empty arrangement is evaluated and
the useful half of the report is the rule set: the mandatory eight, with this
school's own configuration applied on top.

It does NOT generate, improve or search for a timetable, and never writes to a project --
not even the scoring of a stored one. `evara save --revalidate` is what stores that.
A clean run means no hard rule is broken by what it was given.

Options:
  --project, -p  A .evaraproj file to evaluate instead of a fixture.
  --school ID    Which school. Required with --project; never defaulted.
  --cycle ID     Which cycle\'s grid. Required with --project.
  --term ID      Which term. Omit for a grid materialised for the whole year.
  --timetable ID Score a stored timetable instead. It knows its own cycle and term,
                 so --school, --cycle and --term mean nothing beside it.
  --seed N       Seed to generate a fixture from. Default 2026.
  --json         Print the structured form instead of prose.
  --list, -l     List the fixtures.
  --help, -h     Show this text.

Exit codes:
  0   nothing hard is broken
  1   a hard rule is broken
  2   the command itself failed, or the configuration could not be read

See docs/CONSTRAINT-ENGINE.md for what each rule checks, and
docs/SCHEDULING-FIXTURES.md for the fixtures.
";

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use evara_constraints::{Registry, evaluate as score};
    use evara_core::calendar::WorkingDays;
    use evara_core::scheduling::fixtures::{FixtureKind, fixture};
    use evara_core::scheduling::{Assignment, Placement, RoomIx, TimeslotIx};
    use evara_db::{
        ConstraintInstanceInput, CycleInput, CycleKind, PROJECT_EXTENSION, SchoolInput, Workspace,
    };
    use uuid::Uuid;

    use super::{
        DEFAULT_SEED, Outcome, Request, SchedulingSelection, Source, USAGE, as_text, wrap,
    };

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
    fn a_bare_name_evaluates_at_the_default_seed() {
        assert_eq!(
            parse(&["tiny"]),
            Ok(Request::Fixture {
                name: "tiny".to_owned(),
                seed: DEFAULT_SEED,
                json: false,
            })
        );
    }

    #[test]
    fn the_long_and_short_spellings_agree() {
        // `evara validate` accepts both, and a reader who has learnt one command should not
        // have to learn the other.
        assert_eq!(parse(&["--fixture", "tiny"]), parse(&["tiny"]));
    }

    #[test]
    fn a_seed_and_json_are_read() {
        assert_eq!(
            parse(&["tiny", "--seed", "7", "--json"]),
            Ok(Request::Fixture {
                name: "tiny".to_owned(),
                seed: 7,
                json: true,
            })
        );
    }

    #[test]
    fn a_mistyped_option_is_refused_rather_than_ignored() {
        // The rule the other two commands follow: a silently ignored `--sed 7` would report on
        // a different school from the one the user believed they asked about.
        let error = parse(&["tiny", "--sed", "7"]).expect_err("refused");
        assert!(error.contains("--sed"), "{error}");
    }

    #[test]
    fn a_project_needs_a_school_and_a_cycle() {
        let error = parse(&["--project", "school.evaraproj"]).expect_err("refused");
        assert!(error.contains("--school"), "{error}");
        assert!(error.contains("--cycle"), "{error}");
    }

    #[test]
    fn a_stored_timetable_is_addressed_by_itself_and_nothing_else() {
        // It knows its own cycle and term, so accepting a selection beside it would let the
        // two disagree about which grid is being scored.
        let timetable = an_id();
        assert_eq!(
            parse(&[
                "--project",
                "school.evaraproj",
                "--timetable",
                &timetable.to_string(),
            ]),
            Ok(Request::StoredTimetable {
                project: PathBuf::from("school.evaraproj"),
                timetable,
                json: false,
            })
        );

        for flag in ["--school", "--cycle", "--term"] {
            let error = parse(&[
                "--project",
                "school.evaraproj",
                "--timetable",
                &timetable.to_string(),
                flag,
                &an_id().to_string(),
            ])
            .expect_err("refused");
            assert!(error.contains(flag), "{error}");
            assert!(error.contains("already knows"), "{error}");
        }
    }

    #[test]
    fn a_timetable_without_a_project_is_refused() {
        let error = parse(&["tiny", "--timetable", &an_id().to_string()]).expect_err("refused");
        assert!(error.contains("--timetable"), "{error}");
        assert!(error.contains("--project"), "{error}");
    }

    #[test]
    fn the_usage_text_no_longer_claims_there_is_nothing_stored_to_score() {
        // The caveat M5 shipped -- "a project has no stored timetable to score until M7" -- is
        // false now, and a help text that still said it would be the product understating what
        // it can do.
        assert!(!USAGE.contains("until M7"), "{USAGE}");
        assert!(USAGE.contains("STORED timetable"), "{USAGE}");
        assert!(
            USAGE.contains("never writes to a project"),
            "and it is still read-only: {USAGE}"
        );
    }

    #[test]
    fn a_selection_without_a_project_is_refused() {
        let error = parse(&["tiny", "--school", &an_id().to_string()]).expect_err("refused");
        assert!(error.contains("--school"), "{error}");
        assert!(error.contains("--project"), "{error}");
    }

    #[test]
    fn a_fixture_and_a_project_at_once_is_refused() {
        let error = parse(&["tiny", "--project", "school.evaraproj"]).expect_err("refused");
        assert!(error.contains("name one"), "{error}");
    }

    #[test]
    fn a_project_selection_is_read_with_and_without_a_term() {
        let id = an_id().to_string();
        assert_eq!(
            parse(&["--project", "s.evaraproj", "--school", &id, "--cycle", &id]),
            Ok(Request::Project {
                project: PathBuf::from("s.evaraproj"),
                selection: SchedulingSelection {
                    school_id: Uuid::parse_str(&id).expect("an id"),
                    cycle_id: Uuid::parse_str(&id).expect("an id"),
                    term_id: None,
                },
                json: false,
            })
        );
        let with_term = parse(&[
            "--project",
            "s.evaraproj",
            "--school",
            &id,
            "--cycle",
            &id,
            "--term",
            &id,
        ]);
        assert!(matches!(
            with_term,
            Ok(Request::Project { ref selection, .. }) if selection.term_id.is_some()
        ));
    }

    #[test]
    fn an_identifier_that_is_not_one_is_refused() {
        let error =
            parse(&["--project", "s.evaraproj", "--school", "northgate"]).expect_err("refused");
        assert!(error.contains("not an identifier"), "{error}");
    }

    // ------------------------------------------------------------------- the fixture form

    #[test]
    fn every_fixtures_own_arrangement_breaks_no_hard_rule() {
        // The sharpest check the command can make, over the whole catalogue: M2 built these
        // arrangements against ADRs 0032-0035 and verified them independently, so the eight
        // evaluators agreeing is two readings of the same records agreeing.
        //
        // A fixture that ships no arrangement is evaluated empty, which also breaks nothing
        // hard — and the report says which was evaluated, so the two are not confused.
        for kind in FixtureKind::all() {
            let outcome = super::run(&owned(&[kind.name()])).expect("the command ran");
            assert!(
                matches!(outcome, Outcome::Usable),
                "{} broke a hard rule",
                kind.name()
            );
        }
    }

    #[test]
    fn the_json_form_runs_over_every_fixture_too() {
        for kind in FixtureKind::all() {
            let outcome = super::run(&owned(&[kind.name(), "--json"])).expect("the command ran");
            assert!(matches!(outcome, Outcome::Usable), "{}", kind.name());
        }
    }

    #[test]
    fn an_unknown_fixture_is_a_command_failure_and_not_a_violation() {
        // Exit code 2, not 1. The request could not be served, which is a different thing from
        // an arrangement being wrong.
        let error = super::run(&owned(&["enormous"])).expect_err("the command failed");
        assert!(error.contains("enormous"), "{error}");
        assert!(error.contains("--list"), "{error}");
    }

    #[test]
    fn listing_the_fixtures_breaks_nothing() {
        let outcome = super::run(&owned(&["--list"])).expect("the command ran");
        assert!(matches!(outcome, Outcome::Usable));
    }

    // --------------------------------------------------------------------- what it reports

    #[test]
    fn a_clean_report_says_usable_and_never_feasible() {
        let found = fixture("tiny", DEFAULT_SEED).expect("tiny exists");
        let problem = found.problem();
        let rules = Registry::mandatory();
        let assignment = found
            .expectations
            .feasible
            .as_ref()
            .expect("tiny ships an arrangement")
            .assignment(problem.activities.len());
        let report = score(problem, &assignment, &rules);

        let text = as_text("tiny", problem, Source::KnownFeasible, &rules, &report);
        assert!(text.contains("No hard rule is broken"), "{text}");
        assert!(text.contains("not a promise"), "{text}");
        // The fixture's own claim may be named — M2 established it — but the verdict must
        // never make one. "No hard rule is broken" is a much weaker statement than a
        // timetable existing, and the gap between them is the product's honesty.
        assert!(
            !text.contains("is feasible"),
            "a clean evaluation is not a feasibility claim: {text}"
        );
        assert!(
            !text.contains("timetable is"),
            "nor any variation of one: {text}"
        );
        assert!(text.contains("Every occurrence is placed"), "{text}");
        assert!(text.contains("the shipped set"), "{text}");
    }

    #[test]
    fn a_broken_arrangement_is_reported_as_unusable_with_each_rule_named() {
        // Everything in one slot. Whatever else that breaks, it is not usable, and the report
        // has to say so and explain each finding in the rule's own words.
        let found = fixture("tiny", DEFAULT_SEED).expect("tiny exists");
        let problem = found.problem();
        let rules = Registry::mandatory();
        let mut assignment = Assignment::empty(problem.activities.len());
        let slot = problem
            .timeslots
            .iter()
            .position(|held| held.is_teaching)
            .expect("a teaching slot");
        let slot = TimeslotIx::new(u32::try_from(slot).expect("small"));
        for activity in problem.activity_indices() {
            assignment.place(activity, Placement::in_room(slot, RoomIx::new(0)));
        }
        let report = score(problem, &assignment, &rules);
        assert!(!report.is_usable(), "everything at once must clash");

        let text = as_text("tiny", problem, Source::KnownFeasible, &rules, &report);
        assert!(text.contains("not usable"), "{text}");
        assert!(text.contains("Hard —"), "{text}");
        for violation in &report.violations {
            assert!(
                text.contains(violation.constraint_type.key()),
                "{} is missing from the report",
                violation.constraint_type.key()
            );
        }
    }

    #[test]
    fn a_partial_arrangement_is_called_incomplete_and_not_unusable() {
        // `activity-scheduled` is MEDIUM, and the wording is where that distinction either
        // survives or is quietly lost.
        let found = fixture("tiny", DEFAULT_SEED).expect("tiny exists");
        let problem = found.problem();
        let rules = Registry::mandatory();
        let assignment = Assignment::empty(problem.activities.len());
        let report = score(problem, &assignment, &rules);

        let text = as_text("tiny", problem, Source::Empty, &rules, &report);
        assert!(
            text.contains("incomplete, which is not the same as unusable"),
            "{text}"
        );
        assert!(text.contains("No hard rule is broken"), "{text}");
        assert!(text.contains("Medium —"), "{text}");
        assert!(report.is_usable(), "nothing placed breaks no hard rule");
    }

    #[test]
    fn the_usage_text_does_not_claim_to_generate_anything() {
        assert!(USAGE.contains("does NOT generate"), "{USAGE}");
        assert!(USAGE.contains("never writes to a project"), "{USAGE}");
        assert!(!USAGE.contains("is feasible"), "{USAGE}");
    }

    #[test]
    fn prose_wraps_on_word_boundaries() {
        assert_eq!(
            wrap("one two three four", 9),
            vec!["one two".to_owned(), "three".to_owned(), "four".to_owned()]
        );
    }

    // ------------------------------------------------------- end to end, over a real file

    /// A temporary project holding one school with a cycle and nothing else.
    ///
    /// Deliberately minimal, as `validate`'s is: what this proves is the command's path
    /// through a real file — the read-only open, the configuration compile, the exit code.
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

    /// Configures one rule in an existing project, then closes it again.
    fn configure(path: &std::path::Path, school: Uuid, input: &ConstraintInstanceInput) {
        let settings = path.with_file_name("settings.sqlite");
        let mut workspace =
            Workspace::with_settings_at(&settings, "0.1.0-test").expect("workspace");
        workspace.open_project(path).expect("open");
        let _ = school;
        workspace
            .constraints()
            .expect("constraints")
            .create(input)
            .expect("configure a rule");
        assert!(workspace.close_project(), "a project was open");
    }

    #[test]
    fn evaluating_a_real_project_with_nothing_to_place_is_usable() {
        // A school with no lessons has nothing to clash, so the honest answer is "nothing hard
        // is broken" with every occurrence — all nought of them — placed.
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
        assert!(matches!(outcome, Outcome::Usable));
    }

    #[test]
    fn a_schools_own_configuration_reaches_the_report() {
        let (_dir, path, school, cycle) = a_project();
        configure(
            &path,
            school,
            &ConstraintInstanceInput {
                school_id: school,
                type_key: "teacher-clash".to_owned(),
                level: "HARD".to_owned(),
                weight: 50,
                enabled: true,
                params: "{}".to_owned(),
                term_id: None,
            },
        );

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
        assert!(matches!(outcome, Outcome::Usable));
    }

    #[test]
    fn a_configuration_this_build_cannot_read_fails_the_command() {
        // Exit code 2. A school that configured a rule is entitled to know this build cannot
        // run it, and an evaluation that silently dropped it would report a score for a rule
        // set nobody asked for.
        //
        // It also asserts the documented division of labour rather than describing it: the
        // repository accepts this row, because all it can judge is that `params` is an object,
        // and the registry — which owns the schemas — is what refuses the shape.
        let (_dir, path, school, cycle) = a_project();
        configure(
            &path,
            school,
            &ConstraintInstanceInput {
                school_id: school,
                type_key: "duration-contiguous".to_owned(),
                level: "HARD".to_owned(),
                weight: 1,
                enabled: true,
                params: "{\"allowBreakSpan\": \"yes\"}".to_owned(),
                term_id: None,
            },
        );

        let error = super::run(&owned(&[
            "--project",
            &path.to_string_lossy(),
            "--school",
            &school.to_string(),
            "--cycle",
            &cycle.to_string(),
        ]))
        .expect_err("the command failed");
        assert!(error.contains("cannot be read"), "{error}");
        assert!(error.contains("duration-contiguous"), "{error}");
        assert!(
            !error.contains("Temp"),
            "the message leaked a path: {error}"
        );
    }

    #[test]
    fn evaluating_a_real_project_writes_nothing_to_it() {
        // The read-only guarantee, from this command's side.
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

    #[test]
    fn a_school_of_no_project_is_a_command_failure() {
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
}
