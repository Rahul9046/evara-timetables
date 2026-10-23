//! `evara generate` — construct a timetable, headlessly, and report what it could not place.
//!
//! # What this reports, and what it never claims
//!
//! It reports a timetable the construction heuristic built, and for every lesson it could not
//! place, which of two kinds of answer applies: *no timetable can place this*, or *this
//! arrangement did not*. The second is the common one and is never phrased as the first —
//! `evara_core::lessons`' rule binds this command through its mirror:
//!
//! > Never report that a demand *can* be scheduled. And never report that one cannot, when all
//! > you know is that a greedy constructor ran out of room.
//!
//! A complete result is described as having **no confirmed clash within the school solved**,
//! with the unverified counts beside it. Never as conflict-free — ADR 0032 §5 rule 4.
//!
//! # `generate`, and `solve` as an alias
//!
//! `SOLVER.md` §9 and `main.rs`'s planned list named this command `solve`. The M6 brief named
//! it `generate`, which is also what the user-facing screen will be called and what the roadmap
//! calls the phase. Both spellings work and there is one implementation; `generate` is the
//! documented name.
//!
//! # Two inputs, and the read-only guarantee
//!
//! A **fixture** is built in memory from its seed: no database, no file, no migration.
//!
//! A **project** is read from disk through
//! [`Database::open_for_inspection`](evara_db::Database::open_for_inspection) — the same route
//! `evara validate --project` and `evara evaluate --project` take, and for the same reason. The
//! ordinary open path runs migrations, recreates triggers and stamps
//! `last_opened_with_app_version` on *every* open; a command asked to generate a timetable from
//! somebody's live school file must not upgrade it as a side effect. That route hands back a
//! `query_only` connection, so the guarantee is SQLite's rather than a convention.
//!
//! **Nothing is persisted.** `timetable` and `timetable_entry` arrive in M7; this command prints
//! a timetable and forgets it. The project's revisions are untouched, which a test asserts by
//! comparing the file's bytes before and after.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0 | A complete timetable, with nothing hard broken |
//! | 1 | A **partial** timetable — usable, and some lessons are not placed |
//! | 2 | The command itself failed: an unknown fixture, a bad option, a configuration that does not compile, a file that cannot be read |
//! | 3 | Generation was refused or broke: the inputs were rejected by pre-solve validation, or the engine's own check failed |
//!
//! Four codes rather than three, and the reason is the brief's: *"avoid treating a partial
//! result as an unexplained generic error."* A partial timetable is the ordinary outcome of a
//! hard school and is the whole point of `activity-scheduled` being MEDIUM, so it cannot share
//! a code with "the command failed" — a script could not tell a usable result from a broken
//! one. It also cannot be `0`, because a caller that asked for a timetable and got eleven
//! twelfths of one needs to know without parsing prose.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use evara_constraints::Registry;
use evara_core::scheduling::fixtures::{FixtureKind, fixture};
use evara_core::scheduling::{
    CompiledConstraint, Problem, ProblemIdentity, Severity, SolveOutcome, SolveStatus,
    UnplacedCause,
};
use evara_db::{Database, SchedulingSelection};
use evara_solver::{
    GenerateRequest, SolveConfig, SolveLimits, generate, meets_completeness_requirement,
};
use uuid::Uuid;

/// The default seed, matching `evara fixture`, `evara validate` and `evara evaluate`.
const DEFAULT_SEED: u64 = 2026;

/// What the command decided, so `main` can choose an exit code without re-deriving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Every occurrence placed, nothing hard broken.
    Complete,
    /// A usable timetable with some occurrences unplaced.
    Partial,
    /// Generation was refused, or the engine's own check failed.
    Refused,
}

/// Runs the subcommand.
///
/// `args` is everything after `generate`.
pub(crate) fn run(args: &[String]) -> Result<Outcome, String> {
    match Request::parse(args)? {
        Request::Help => {
            print!("{USAGE}");
            Ok(Outcome::Complete)
        }
        Request::List => {
            for kind in FixtureKind::all() {
                println!("  {}", kind.name());
            }
            Ok(Outcome::Complete)
        }
        Request::Fixture {
            name,
            seed,
            limits,
            json,
        } => generate_fixture(&name, seed, limits, json),
        Request::Project {
            project,
            selection,
            seed,
            limits,
            json,
        } => generate_project(&project, &selection, seed, limits, json),
    }
}

/// Generates a timetable for one synthetic fixture, in memory.
///
/// Pre-solve validation runs first, from the problem alone — there is no loader to consult for
/// a fixture, so `may_solve` is M3's verdict and the integrity check `Problem::integrity`
/// performs.
fn generate_fixture(
    name: &str,
    seed: u64,
    limits: SolveLimits,
    json: bool,
) -> Result<Outcome, String> {
    let found = fixture(name, seed).ok_or_else(|| {
        format!("no fixture is called {name:?}. Run `evara generate --list` to see them.")
    })?;

    let rules = Registry::mandatory();
    let mut worst = Outcome::Complete;
    for (n, problem) in found.problems.iter().enumerate() {
        let preflight = evara_core::scheduling::validate(problem);
        let summary = evara_core::scheduling::ValidationSummary::of(problem, &preflight);
        let integrity = problem.integrity();

        let request = GenerateRequest {
            problem,
            identity: identity_of(problem),
            constraints: &rules,
            preflight,
            // Both halves, the same two `AssemblyReport::may_solve` reads: a blocking finding,
            // and an internally inconsistent problem. No catalogue fixture has the second.
            may_solve: summary.may_solve() && integrity.is_empty(),
            unenforced_restrictions: 0,
            config: SolveConfig::seeded(seed).with_limits(limits),
        };

        let started = Instant::now();
        let outcome = generate(&request);
        let elapsed = started.elapsed();

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
            print!("{}", as_json(&label, problem, &rules, &outcome, elapsed));
        } else {
            print!("{}", as_text(&label, problem, &rules, &outcome, elapsed));
        }
        worst = worst.and(decide(&outcome));
    }
    Ok(worst)
}

/// Generates a timetable for one school of a real project. Reads; never writes.
fn generate_project(
    path: &Path,
    selection: &SchedulingSelection,
    seed: u64,
    limits: SolveLimits,
    json: bool,
) -> Result<Outcome, String> {
    let mut db = Database::open_for_inspection(path).map_err(|error| describe(path, &error))?;

    // The school's own configuration, on top of the mandatory set. A row that cannot be
    // compiled fails the command rather than being skipped: a school that configured a rule is
    // entitled to know this build cannot run it, and generating against rules nobody asked for
    // would produce a timetable checked by the wrong engine.
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
    let report = evara_db::AssemblyReport::of(&assembly);

    let unenforced = match report.completeness {
        evara_db::Completeness::Complete => 0,
        evara_db::Completeness::RestrictionsNotEnforced { rules } => {
            u32::try_from(rules).unwrap_or(u32::MAX)
        }
    };

    let request = GenerateRequest {
        problem: &assembly.problem,
        identity: ProblemIdentity {
            school: selection.school_id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &rules,
        preflight: report.findings.clone(),
        // The one place both halves are known: a blocking finding, and the loader's integrity
        // result. `AssemblyReport::may_solve` is that answer and is not re-derived here.
        may_solve: report.may_solve(),
        unenforced_restrictions: unenforced,
        config: SolveConfig::seeded(seed).with_limits(limits),
    };

    let started = Instant::now();
    let outcome = generate(&request);
    let elapsed = started.elapsed();

    if json {
        print!(
            "{}",
            project_json(
                path,
                selection,
                &assembly.problem,
                &rules,
                &outcome,
                elapsed
            )
        );
    } else {
        print!(
            "{}",
            project_text(
                path,
                selection,
                &assembly.problem,
                &rules,
                &outcome,
                elapsed
            )
        );
    }
    Ok(decide(&outcome))
}

impl Outcome {
    /// The worse of two outcomes, for a multi-school fixture.
    fn and(self, other: Self) -> Self {
        match (&self, &other) {
            (Self::Refused, _) | (_, Self::Refused) => Self::Refused,
            (Self::Partial, _) | (_, Self::Partial) => Self::Partial,
            _ => Self::Complete,
        }
    }
}

/// Which exit code a result earns.
///
/// Read from the status rather than from the placement count, so the engine's verdict and the
/// command's agree by construction.
fn decide(outcome: &SolveOutcome) -> Outcome {
    match outcome.status {
        SolveStatus::Complete => Outcome::Complete,
        SolveStatus::Partial => Outcome::Partial,
        // Cancelled cannot be reached from this command — nothing here cancels — and is
        // reported as a refusal rather than a success if it ever is.
        SolveStatus::Infeasible | SolveStatus::Cancelled | SolveStatus::Failed => Outcome::Refused,
    }
}

/// The problem's own identity, for a fixture that has no selection to echo.
fn identity_of(problem: &Problem) -> ProblemIdentity {
    ProblemIdentity {
        school: problem.context.school.id,
        cycle: problem.context.cycle.id,
        term: problem.context.term.as_ref().map(|term| term.id),
    }
}

// =======================================================================================
// Rendering
// =======================================================================================

/// The human-readable report for a fixture.
fn as_text(
    label: &str,
    problem: &Problem,
    rules: &[CompiledConstraint],
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{label} — {}", problem.context.school.name);
    verdict(problem, rules, outcome, elapsed, &mut out);
    unplaced(outcome, &mut out);
    out.push('\n');
    out
}

/// The human-readable report for a project.
fn project_text(
    path: &Path,
    selection: &SchedulingSelection,
    problem: &Problem,
    rules: &[CompiledConstraint],
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
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
        "  read-only: nothing was written to this project, and no timetable was saved"
    );
    verdict(problem, rules, outcome, elapsed, &mut out);
    unplaced(outcome, &mut out);
    out.push('\n');
    out
}

/// The verdict both forms share, so neither can phrase it differently.
fn verdict(
    problem: &Problem,
    rules: &[CompiledConstraint],
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
    out: &mut String,
) {
    let total = problem.activities.len();
    let placed = outcome.placed_count();

    let _ = writeln!(
        out,
        "  seed {}  ·  {} rule{} evaluated",
        outcome.seed.get(),
        rules.len(),
        plural(rules.len())
    );
    let _ = writeln!(
        out,
        "  {placed} of {total} occurrence{} placed, {} unplaced",
        plural(total),
        outcome.unscheduled.len()
    );

    // The status, in the words the records allow. "No confirmed clash within the school
    // solved" and never "conflict-free" — ADR 0032 §5 rule 4.
    let headline = match outcome.status {
        SolveStatus::Complete => {
            "Complete. Every occurrence is placed and no confirmed clash was found within this \
             school."
        }
        SolveStatus::Partial => {
            "Partial. What was placed carries no confirmed clash within this school; the \
             lessons below are not placed."
        }
        SolveStatus::Infeasible => {
            "The best arrangement found still breaks a hard rule. This is not a proof that no \
             timetable exists."
        }
        SolveStatus::Cancelled => "Stopped before finishing. What was placed is below.",
        SolveStatus::Failed => {
            "Generation did not produce a timetable. See the reason and the findings below."
        }
    };
    let _ = writeln!(out, "  {headline}");

    let _ = writeln!(
        out,
        "  stopped because: {}{}",
        termination_label(outcome),
        if outcome.termination.is_reproducible() {
            ""
        } else {
            " (not reproducible — a cancellation depends on when it arrived)"
        }
    );

    if !meets_completeness_requirement(outcome, rules) {
        // The school raised `activity-scheduled` to HARD. The partial work is still reported;
        // what is withheld is the claim that it meets the requirement.
        let _ = writeln!(
            out,
            "  This school requires every lesson to be placed, and this result does not meet \
             that requirement."
        );
    }

    let _ = writeln!(
        out,
        "  score {} hard, {} medium, {} soft",
        outcome.score.hard, outcome.score.medium, outcome.score.soft
    );

    if outcome.hard_violations.is_empty() {
        let _ = writeln!(
            out,
            "  Independently re-checked with the full evaluator: no hard rule is broken."
        );
    } else {
        // The engine refuses every placement that would break a hard rule, so reaching this is
        // a defect in Evara rather than a property of the data.
        let _ = writeln!(
            out,
            "  The full evaluator found hard violations in the generated arrangement. This is \
             a defect in Evara, not in your data; please report it:"
        );
        for broken in &outcome.hard_violations {
            let _ = writeln!(out, "    {} × {}", broken.violations, broken.type_key);
        }
    }

    disclosure(outcome, elapsed, out);
}

/// What was not checked, what pre-solve found, and what the run cost.
///
/// Split from [`verdict`] because it answers a different question — *how much should you trust
/// this?* rather than *what is it?* — and because every line of it is mandatory: ADR 0032 §5
/// rule 4 requires the unverified counts beside the verdict whatever they say.
fn disclosure(outcome: &SolveOutcome, elapsed: std::time::Duration, out: &mut String) {
    // Never omitted, even when it is all zeroes: all-zeroes is the one state with nothing to
    // disclose, and even then the correct phrasing is "no *confirmed* clash".
    let unverified = outcome.unverified;
    if unverified.has_anything_to_disclose() {
        let _ = writeln!(out, "  Not checked:");
        for (count, what) in [
            (
                unverified.indeterminate_student_pairs,
                "group pair(s) whose shared students nobody has declared — those pairs were \
                 not kept apart",
            ),
            (
                unverified.unchecked_capacities,
                "occurrence(s) whose class size is unknown, so room capacity was not compared",
            ),
            (
                unverified.shared_entities,
                "entit(ies) another school also holds — no clash was compared across the \
                 boundary",
            ),
            (
                unverified.unenforced_restrictions,
                "stored availability restriction(s) this engine does not enforce",
            ),
        ] {
            if count > 0 {
                let _ = writeln!(out, "    {count} {what}");
            }
        }
    } else {
        let _ = writeln!(
            out,
            "  Nothing was left unchecked, and this is still described as no *confirmed* clash \
             within the school solved."
        );
    }

    let blocking = outcome.preflight.count_of(Severity::Blocking);
    let warnings = outcome.preflight.count_of(Severity::Warning);
    if blocking + warnings > 0 {
        let _ = writeln!(
            out,
            "  Pre-solve: {blocking} blocking, {warnings} warning(s) — run `evara validate` for \
             the detail"
        );
    }

    let statistics = outcome.statistics;
    let _ = writeln!(
        out,
        "  {} candidate evaluation(s), {} rejected, {} step(s), {} pass(es)",
        statistics.candidate_evaluations,
        statistics.candidates_rejected,
        statistics.construction_steps,
        statistics.passes
    );
    // Wall-clock time is measured here and not inside the solver: the engine reads no clock, so
    // that the same seed reproduces the same timetable on every machine.
    let _ = writeln!(
        out,
        "  {} ms wall clock (measured by this command; the engine reads no clock)",
        elapsed.as_millis()
    );
}

/// Why the run stopped, in a sentence.
fn termination_label(outcome: &SolveOutcome) -> &'static str {
    use evara_core::scheduling::Termination;

    match outcome.termination {
        Termination::Complete => "every occurrence was placed",
        Termination::Exhausted => {
            "every remaining lesson was tried against every slot it could use, and refused"
        }
        Termination::CandidateLimitReached => "the candidate-evaluation limit was reached",
        Termination::StepLimitReached => "the construction-step limit was reached",
        Termination::PassLimitReached => "the retry-pass limit was reached",
        Termination::Cancelled => "it was stopped",
        Termination::InputRejected => {
            "pre-solve validation refused the inputs, so nothing was attempted"
        }
        Termination::InternalCheckFailed => "the engine's own final check failed",
    }
}

/// The unplaced lessons, in the order a reader should act on them.
///
/// Three sections, because there are three remedies: *"nothing was attempted, fix the input"*,
/// *"no timetable can place this, change the setup"*, and *"this arrangement did not, try
/// again"*. A single list would leave a reader working out which applied to which, and the
/// third wording looks like the second unless they are kept apart.
fn unplaced(outcome: &SolveOutcome, out: &mut String) {
    if outcome.unscheduled.is_empty() {
        return;
    }

    // Three sections, because there are three remedies and a reader must not have to work
    // out which applies. The order is the order a user should act in.
    let refused: Vec<_> = outcome
        .unscheduled
        .iter()
        .filter(|u| u.cause == UnplacedCause::InputRejected)
        .collect();
    let (impossible, merely): (Vec<_>, Vec<_>) = outcome
        .unscheduled
        .iter()
        .filter(|u| u.cause != UnplacedCause::InputRejected)
        .partition(|u| u.cause.is_impossible());

    if !refused.is_empty() {
        let _ = writeln!(
            out,
            "\n  Nothing was attempted, because the inputs carry a blocking finding. Correct \
             it and generate again — `evara validate` lists them:"
        );
        // The findings pre-solve already proved, where any of them name a specific lesson. A
        // refusal is about the school's setup, so most runs print none of these; the ones that
        // do are the lessons a reader can go straight to.
        for lesson in &refused {
            if let Some(finding) = &lesson.diagnostic {
                let _ = writeln!(out, "    [{}] {}", finding.code.key(), lesson.section);
            }
        }
        let _ = writeln!(
            out,
            "    {} lesson(s) were not attempted. Nothing is claimed about whether they can \
             be placed.",
            refused.len()
        );
    }

    if !impossible.is_empty() {
        let _ = writeln!(
            out,
            "\n  Cannot be placed by any timetable — these need a setup change:"
        );
        for lesson in impossible {
            let _ = writeln!(out, "    [{}] {}", lesson.cause.key(), lesson.section);
            for line in wrap(&lesson.reason, 74) {
                let _ = writeln!(out, "        {line}");
            }
            if let Some(finding) = &lesson.diagnostic {
                let _ = writeln!(
                    out,
                    "        pre-solve already said: {}",
                    finding.code.key()
                );
            }
        }
    }

    if !merely.is_empty() {
        let _ = writeln!(
            out,
            "\n  Not placed by this run. This is NOT a proof that no complete timetable \
             exists:"
        );
        for lesson in &merely {
            let _ = writeln!(out, "    [{}] {}", lesson.cause.key(), lesson.section);
            for line in wrap(&lesson.reason, 74) {
                let _ = writeln!(out, "        {line}");
            }
        }
        if merely
            .iter()
            .any(|u| u.cause == UnplacedCause::BlockedByPlacements)
        {
            let _ = writeln!(
                out,
                "    Try another --seed: a different candidate order often places these."
            );
        }
    }
}

/// The structured form, built from the engine's own types rather than a second schema.
fn as_json(
    label: &str,
    problem: &Problem,
    rules: &[CompiledConstraint],
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
) -> String {
    let wrapper = serde_json::json!({
        "fixture": label,
        "school": problem.context.school.name,
        "outcome": outcome,
        "placed": outcome.placed_count(),
        "required": problem.activities.len(),
        "isComplete": outcome.is_complete(),
        "isAdoptable": outcome.is_adoptable(),
        "meetsCompletenessRequirement": meets_completeness_requirement(outcome, rules),
        "rules": rules,
        "elapsedMillis": elapsed.as_millis(),
    });
    render(&wrapper)
}

/// The structured form for a project, in the same vocabulary as the fixture form.
fn project_json(
    path: &Path,
    selection: &SchedulingSelection,
    problem: &Problem,
    rules: &[CompiledConstraint],
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
) -> String {
    let wrapper = serde_json::json!({
        "project": file_name(path),
        "school": {
            "id": selection.school_id,
            "name": problem.context.school.name,
        },
        "cycleId": selection.cycle_id,
        "termId": selection.term_id,
        "readOnly": true,
        "persisted": false,
        "outcome": outcome,
        "placed": outcome.placed_count(),
        "required": problem.activities.len(),
        "isComplete": outcome.is_complete(),
        "isAdoptable": outcome.is_adoptable(),
        "meetsCompletenessRequirement": meets_completeness_requirement(outcome, rules),
        "rules": rules,
        "elapsedMillis": elapsed.as_millis(),
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

// =======================================================================================
// Reading the request
// =======================================================================================

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// Print the usage text.
    Help,
    /// List the fixtures that can be generated for.
    List,
    /// Generate for one fixture.
    Fixture {
        /// Which fixture.
        name: String,
        /// The seed to generate the fixture *and* order the candidates from.
        seed: u64,
        /// When to stop.
        limits: SolveLimits,
        /// Whether to print the structured form.
        json: bool,
    },
    /// Generate for one school of a project.
    Project {
        /// The project file.
        project: PathBuf,
        /// Which school, cycle and term.
        selection: SchedulingSelection,
        /// The seed.
        seed: u64,
        /// When to stop.
        limits: SolveLimits,
        /// Whether to print the structured form.
        json: bool,
    },
}

impl Request {
    /// Reads the arguments.
    ///
    /// The same grammar as `evara validate` and `evara evaluate`, deliberately: the three
    /// commands take the same inputs and a reader who has learnt one should not have to learn
    /// the others. `--school` and `--cycle` are required beside `--project` because a project
    /// has held several schools since `V12` and ADR 0032 rule 1 scopes generation to one
    /// `(school, cycle, term)` — picking one would build a timetable over a grid the user did
    /// not choose.
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut json = false;
        let mut list = false;
        let mut project: Option<PathBuf> = None;
        let mut school: Option<Uuid> = None;
        let mut cycle: Option<Uuid> = None;
        let mut term: Option<Uuid> = None;
        let mut limits = SolveLimits::DEFAULT;
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
                "--max-candidates" => {
                    let value = rest.next().ok_or("--max-candidates needs a number")?;
                    limits.max_candidate_evaluations = value
                        .parse()
                        .map_err(|_| format!("--max-candidates {value:?} is not a number"))?;
                }
                "--max-steps" => {
                    let value = rest.next().ok_or("--max-steps needs a number")?;
                    limits.max_construction_steps = value
                        .parse()
                        .map_err(|_| format!("--max-steps {value:?} is not a number"))?;
                }
                "--max-passes" => {
                    let value = rest.next().ok_or("--max-passes needs a number")?;
                    limits.max_passes = value
                        .parse()
                        .map_err(|_| format!("--max-passes {value:?} is not a number"))?;
                }
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

        Self::resolve(Read {
            name,
            seed,
            json,
            list,
            project,
            school,
            cycle,
            term,
            limits,
        })
    }

    /// Turns what was read into the request it describes, or says why it is not one.
    ///
    /// Split from [`Request::parse`] so the reading and the resolving are separately legible:
    /// the first is a loop over flags, the second is where every refusal lives.
    fn resolve(read: Read) -> Result<Self, String> {
        let Read {
            name,
            seed,
            json,
            list,
            project,
            school,
            cycle,
            term,
            limits,
        } = read;

        if let Some(project) = project {
            if name.is_some() {
                return Err(String::from(
                    "a fixture and a project are two different things to generate for; name one",
                ));
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
                seed,
                limits,
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
            Some(name) => Ok(Self::Fixture {
                name,
                seed,
                limits,
                json,
            }),
            None => Err(format!(
                "name a fixture, or pass --project <path>, or --list\n\n{USAGE}"
            )),
        }
    }
}

/// Every option as read from the command line, before it is known what they describe.
///
/// A struct rather than nine locals threaded through two functions: the resolving half needs all
/// of them and a tuple of nine would be unreadable at both ends.
struct Read {
    name: Option<String>,
    seed: u64,
    json: bool,
    list: bool,
    project: Option<PathBuf>,
    school: Option<Uuid>,
    cycle: Option<Uuid>,
    term: Option<Uuid>,
    limits: SolveLimits,
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
Usage: evara generate <fixture> [--seed N] [--json]
       evara generate --fixture <fixture> [--seed N] [--json]
       evara generate --list
       evara generate --project <path.evaraproj> --school <id> --cycle <id>
                      [--term <id>] [--seed N] [--json]

Constructs a timetable with the construction heuristic and reports it: what was
placed, what was not, and which of those two kinds of answer each unplaced lesson
got — \"no timetable can place this\" or \"this arrangement did not\".

A complete result is described as having no CONFIRMED clash within the school
solved, with the unverified counts beside it. Never as conflict-free.

`solve` is an accepted alias for this command.

Generation NEVER writes to a project, and no timetable is saved: persistence is M7.

Options:
  --project, -p      A .evaraproj file to generate for instead of a fixture.
  --school ID        Which school. Required with --project; never defaulted.
  --cycle ID         Which cycle\'s grid. Required with --project.
  --term ID          Which term. Omit for a grid materialised for the whole year.
  --seed N           Seed for the fixture and for the candidate order. Default 2026.
  --max-candidates N Stop after this many candidate evaluations.
  --max-steps N      Stop after this many construction steps.
  --max-passes N     Stop after this many retry passes.
  --json             Print the structured form instead of prose.
  --list, -l         List the fixtures.
  --help, -h         Show this text.

Every limit is a count, never a duration: the same problem, seed and limits stop in
the same place on every machine.

Exit codes:
  0   a complete timetable, nothing hard broken
  1   a partial timetable — usable, and some lessons are not placed
  2   the command itself failed, or the configuration could not be read
  3   generation was refused, or the engine\'s own check failed

See docs/TIMETABLE-CONSTRUCTION-SOLVER.md for the algorithm and its limits,
docs/CONSTRAINT-ENGINE.md for what each rule checks, and
docs/SCHEDULING-FIXTURES.md for the fixtures.
";

#[cfg(test)]
mod tests;
