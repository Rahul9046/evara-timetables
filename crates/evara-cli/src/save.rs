//! `evara save` — the one command that writes a timetable into a project.
//!
//! # Why this is not a flag on `evara generate`
//!
//! `evara generate` is documented as never writing, and a test compares the project file's
//! bytes before and after to prove it. A `--save` flag would make that documentation false for
//! some invocations and true for others, which is the worst state for a guarantee to be in: a
//! user reading the help text could not tell whether the command they are about to run touches
//! their file.
//!
//! So writing lives in a command named for writing. `generate` keeps its guarantee, and
//! anything that modifies a project is one word the user typed.
//!
//! # Two modes, one write
//!
//! | Mode | What it does |
//! | --- | --- |
//! | `--school --cycle [--term]` | loads, generates and **saves a new timetable**. Never overwrites: several may exist for one scope, and that is the versioning |
//! | `--revalidate <id>` | re-scores a **stored** timetable against the current inputs and replaces its current findings, leaving the generation record alone |
//!
//! # The freshness guarantee
//!
//! Generation reads a snapshot of the inputs; saving happens afterwards. Between the two
//! anything may have moved, and **nothing referential would break** — every activity, timeslot
//! and room the result names still exists. `evara_db` compares the problem's change-log
//! watermark against the feed *inside the writing transaction* and refuses
//! `InputsChanged` if it moved. This command never compares a watermark itself: ADR 0030's rule
//! is that a fingerprint is only safe where it is acted on.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0 | saved: a complete timetable, or a revalidation recorded |
//! | 1 | saved, and the timetable is **partial** — usable, with lessons unplaced |
//! | 2 | the command itself failed, or the project refused the write |
//! | 3 | nothing was saved because generation was refused or produced no timetable |

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{ProblemIdentity, SolveOutcome};
use evara_db::{
    AssemblyReport, RunProvenance, SavedTimetable, SchedulingSelection, Scoring, Workspace,
};
use evara_solver::{GenerateRequest, SolveConfig, SolveLimits, generate};
use uuid::Uuid;

/// The default seed, matching every other command.
const DEFAULT_SEED: u64 = 2026;

/// This build, recorded on every run and stamped on the project when it is opened.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which engine produced a run, as recorded on `solve_run`.
const ENGINE: &str = "construction";

/// What the command decided.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Saved, and every occurrence is placed.
    Saved,
    /// Saved, and some occurrences are not placed.
    SavedPartial,
    /// Nothing was saved, because there was no timetable to save.
    NothingToSave,
}

/// Runs the subcommand.
pub(crate) fn run(args: &[String]) -> Result<Outcome, String> {
    match Request::parse(args)? {
        Request::Help => {
            print!("{USAGE}");
            Ok(Outcome::Saved)
        }
        Request::Generate {
            project,
            selection,
            seed,
            limits,
            name,
            json,
        } => save_new(&project, &selection, seed, limits, name.as_deref(), json),
        Request::Revalidate {
            project,
            timetable,
            json,
        } => revalidate(&project, timetable, json),
    }
}

/// Generates a timetable and saves it.
fn save_new(
    path: &Path,
    selection: &SchedulingSelection,
    seed: u64,
    limits: SolveLimits,
    name: Option<&str>,
    json: bool,
) -> Result<Outcome, String> {
    // Through a `Workspace`, because saving writes and every change-log trigger calls
    // `evara_site_id()` — which only a connection opened with this installation's identity
    // registers, and that identity lives in the settings database. `evara generate` can open
    // the file itself precisely because it writes nothing.
    let mut workspace = open_workspace(path)?;
    let db = &mut workspace;

    let stored = db
        .constraints()
        .map_err(|error| describe(path, &error))?
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
        .map_err(|error| describe(path, &error))?
        .load(selection)
        .map_err(|error| describe(path, &error))?;
    let report = AssemblyReport::of(&assembly);
    let unenforced = match report.completeness {
        evara_db::Completeness::Complete => 0,
        evara_db::Completeness::RestrictionsNotEnforced { rules } => {
            u32::try_from(rules).unwrap_or(u32::MAX)
        }
    };

    let started = now_iso8601();
    let clock = Instant::now();
    let outcome = generate(&GenerateRequest {
        problem: &assembly.problem,
        identity: ProblemIdentity {
            school: selection.school_id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &rules,
        preflight: report.findings.clone(),
        may_solve: report.may_solve(),
        unenforced_restrictions: unenforced,
        config: SolveConfig::seeded(seed).with_limits(limits),
    });
    let elapsed = clock.elapsed();
    let finished = now_iso8601();

    // The independent scoring, by the reference scorer. What is stored as the generation
    // evaluation is this, not the engine's own belief about its result.
    let reference = evaluate(&assembly.problem, &outcome.assignment, &rules);

    let provenance = RunProvenance {
        engine: ENGINE.to_owned(),
        settings: settings_json(limits),
        started_at: started,
        finished_at: finished,
    };
    let chosen = name.map_or_else(|| default_name(&outcome), str::to_owned);

    let saved = match db
        .timetables()
        .map_err(|error| describe(path, &error))?
        .save_generated(
            &assembly,
            &outcome,
            &reference.violations,
            &provenance,
            &chosen,
        ) {
        Ok(saved) => saved,
        // Not a command failure: the command worked and there was no timetable to store. The
        // inputs were refused at the gate, or the engine's own final check rejected what it
        // built. Exit 3 rather than 2, so a script can tell "nothing to save" from "broken".
        Err(evara_db::DbError::NotSaveable { reason }) => {
            println!("{} — nothing was saved: {reason}.", file_name(path));
            println!("  Run `evara validate --project` to see what needs correcting first.");
            return Ok(Outcome::NothingToSave);
        }
        Err(error) => return Err(describe(path, &error)),
    };

    if json {
        print!("{}", saved_json(path, selection, &saved, &outcome, elapsed));
    } else {
        print!("{}", saved_text(path, selection, &saved, &outcome, elapsed));
    }

    Ok(if outcome.is_complete() {
        Outcome::Saved
    } else {
        Outcome::SavedPartial
    })
}

/// Re-scores a stored timetable and replaces its current findings.
fn revalidate(path: &Path, timetable_id: Uuid, json: bool) -> Result<Outcome, String> {
    let mut workspace = open_workspace(path)?;
    let db = &mut workspace;

    let timetable = db
        .timetables()
        .map_err(|error| describe(path, &error))?
        .get(timetable_id)
        .map_err(|error| describe(path, &error))?;
    let selection = SchedulingSelection {
        // The school is the cycle's: a timetable carries no `school_id`, which is what keeps
        // the ownership from having a second place to be wrong.
        school_id: school_of_cycle(db, timetable.cycle_id, path)?,
        cycle_id: timetable.cycle_id,
        term_id: timetable.term_id,
    };

    let stored = db
        .constraints()
        .map_err(|error| describe(path, &error))?
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
        .map_err(|error| describe(path, &error))?
        .load(&selection)
        .map_err(|error| describe(path, &error))?;
    let arrangement = db
        .timetables()
        .map_err(|error| describe(path, &error))?
        .arrangement(timetable_id, &assembly)
        .map_err(|error| describe(path, &error))?;
    let report = evaluate(&assembly.problem, &arrangement.assignment, &rules);

    let evaluation = db
        .timetables()
        .map_err(|error| describe(path, &error))?
        .record_revalidation(
            timetable_id,
            timetable.rev,
            &Scoring {
                score: report.score,
                placed: arrangement.entries,
                required: assembly.problem.activities.len(),
                violations: &report.violations,
            },
            &assembly,
        )
        .map_err(|error| describe(path, &error))?;

    if json {
        print!(
            "{}",
            render(&serde_json::json!({
                "project": file_name(path),
                "timetable": { "id": timetable_id, "name": timetable.name },
                "evaluation": {
                    "id": evaluation.id,
                    "kind": evaluation.kind,
                    "score": report.score,
                    "placed": arrangement.entries,
                    "required": assembly.problem.activities.len(),
                },
                "entriesNotInProblem": arrangement.not_in_problem,
                "violations": report.violations.len(),
                "generationRecordPreserved": true,
            }))
        );
    } else {
        print!(
            "{}",
            revalidated_text(path, &timetable.name, &arrangement, &report)
        );
    }
    Ok(Outcome::Saved)
}

/// The school a cycle belongs to.
///
/// A timetable carries no `school_id` — a child's school is resolved through its parent chain
/// (`CLAUDE.md` §3) — so revalidating one reads the cycle to find out which school's
/// constraint configuration and problem to compile. This is the join that costs, and the
/// reason it is worth paying is that the ownership has exactly one home.
fn school_of_cycle(db: &mut Workspace, cycle_id: Uuid, path: &Path) -> Result<Uuid, String> {
    db.time_model()
        .map_err(|error| describe(path, &error))?
        .cycle(cycle_id)
        .map_err(|error| describe(path, &error))?
        .map(|cycle| cycle.school_id)
        .ok_or_else(|| {
            format!(
                "{} — this timetable names a cycle the project no longer holds",
                file_name(path)
            )
        })
}

/// A name for a timetable the caller did not name.
///
/// The **caller's** default, never the database's: only a caller knows what to call one, and a
/// column default would put a naming decision in the schema.
fn default_name(outcome: &SolveOutcome) -> String {
    format!(
        "Generated {} (seed {})",
        now_iso8601().get(..16).unwrap_or("now").replace('T', " "),
        outcome.seed.get()
    )
}

/// The limits, as the JSON `solve_run.settings` stores.
fn settings_json(limits: SolveLimits) -> String {
    serde_json::json!({
        "maxCandidateEvaluations": limits.max_candidate_evaluations,
        "maxConstructionSteps": limits.max_construction_steps,
        "maxPasses": limits.max_passes,
    })
    .to_string()
}

/// Opens the project for writing, through this installation's settings database.
fn open_workspace(path: &Path) -> Result<Workspace, String> {
    let mut workspace = Workspace::new(VERSION).map_err(|error| describe(path, &error))?;
    workspace
        .open_project(path)
        .map_err(|error| describe(path, &error))?;
    Ok(workspace)
}

/// ISO-8601 UTC to millisecond precision, which is what every stored timestamp uses.
fn now_iso8601() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = i64::try_from(now.as_secs()).unwrap_or_default();
    let millis = now.subsec_millis();
    let date =
        time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
    )
}

// =======================================================================================
// Rendering
// =======================================================================================

fn saved_text(
    path: &Path,
    selection: &SchedulingSelection,
    saved: &SavedTimetable,
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} — saved", file_name(path));
    let _ = writeln!(
        out,
        "  timetable {}  \"{}\"",
        saved.timetable.id, saved.timetable.name
    );
    let _ = writeln!(out, "  school    {}", selection.school_id);
    let _ = writeln!(out, "  cycle     {}", selection.cycle_id);
    let _ = match selection.term_id {
        Some(term) => writeln!(out, "  term      {term}"),
        None => writeln!(out, "  term      (none — a grid for the whole year)"),
    };
    let _ = writeln!(
        out,
        "  {} entr{} written, {} unplaced occurrence{} recorded, {} violation{} stored",
        saved.entries,
        if saved.entries == 1 { "y" } else { "ies" },
        saved.unscheduled,
        plural(saved.unscheduled),
        saved.violations,
        plural(saved.violations)
    );
    let _ = writeln!(
        out,
        "  run {}  seed {}  {}",
        saved.run.id, saved.run.seed, saved.run.termination
    );
    let _ = writeln!(
        out,
        "  generation scoring {} — score {} hard, {} medium, {} soft",
        saved.evaluation.id,
        saved.evaluation.score_hard,
        saved.evaluation.score_medium,
        saved.evaluation.score_soft
    );

    if outcome.is_complete() {
        let _ = writeln!(
            out,
            "  Complete: every occurrence is placed, with no confirmed clash within this school."
        );
    } else {
        let _ = writeln!(
            out,
            "  Partial: {} of {} occurrences placed. The rest are recorded with a cause each.",
            outcome.placed_count(),
            outcome.placed_count() + outcome.unscheduled.len()
        );
    }

    // ADR 0032 rule 4, carried onto the stored timetable rather than left on the screen.
    let unverified = saved.timetable.unverified();
    if unverified.has_anything_to_disclose() {
        let _ = writeln!(
            out,
            "  Not checked, and stored with the timetable: {} indeterminate group pair(s), \
             {} unchecked capacit(ies), {} shared entit(ies), {} unenforced restriction(s)",
            unverified.indeterminate_student_pairs,
            unverified.unchecked_capacities,
            unverified.shared_entities,
            unverified.unenforced_restrictions
        );
    }
    let _ = writeln!(
        out,
        "  A regeneration will not overwrite this; it saves another timetable beside it."
    );
    let _ = writeln!(out, "  {} ms wall clock", elapsed.as_millis());
    out.push('\n');
    out
}

fn revalidated_text(
    path: &Path,
    name: &str,
    arrangement: &evara_db::StoredArrangement,
    report: &evara_constraints::EvaluationReport,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} — revalidated \"{name}\"", file_name(path));
    let _ = writeln!(
        out,
        "  {} stored entr{} read",
        arrangement.entries,
        if arrangement.entries == 1 { "y" } else { "ies" }
    );
    if !arrangement.not_in_problem.is_empty() {
        let _ = writeln!(
            out,
            "  {} stored placement(s) name a lesson this problem no longer holds, so they were \
             not scored",
            arrangement.not_in_problem.len()
        );
    }
    let _ = writeln!(
        out,
        "  score now {} hard, {} medium, {} soft",
        report.score.hard, report.score.medium, report.score.soft
    );
    if report.is_usable() {
        let _ = writeln!(
            out,
            "  No hard rule is broken by the stored arrangement as the inputs now stand."
        );
    } else {
        let _ = writeln!(
            out,
            "  {} hard violation(s): the inputs have moved since this was generated, and the \
             stored timetable is no longer usable as it stands.",
            report.count_at(evara_core::scheduling::ConstraintLevel::Hard)
        );
    }
    let _ = writeln!(
        out,
        "  The record of how this timetable was generated is unchanged; only its current \
         findings were replaced."
    );
    out.push('\n');
    out
}

fn saved_json(
    path: &Path,
    selection: &SchedulingSelection,
    saved: &SavedTimetable,
    outcome: &SolveOutcome,
    elapsed: std::time::Duration,
) -> String {
    render(&serde_json::json!({
        "project": file_name(path),
        "timetable": saved.timetable,
        "run": saved.run,
        "generationEvaluation": saved.evaluation,
        "entries": saved.entries,
        "unscheduled": saved.unscheduled,
        "violations": saved.violations,
        "school": selection.school_id,
        "cycleId": selection.cycle_id,
        "termId": selection.term_id,
        "isComplete": outcome.is_complete(),
        "overwrote": false,
        "elapsedMillis": elapsed.as_millis(),
    }))
}

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

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Generate {
        project: PathBuf,
        selection: SchedulingSelection,
        seed: u64,
        limits: SolveLimits,
        name: Option<String>,
        json: bool,
    },
    Revalidate {
        project: PathBuf,
        timetable: Uuid,
        json: bool,
    },
}

impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut project: Option<PathBuf> = None;
        let mut school: Option<Uuid> = None;
        let mut cycle: Option<Uuid> = None;
        let mut term: Option<Uuid> = None;
        let mut revalidate: Option<Uuid> = None;
        let mut name: Option<String> = None;
        let mut seed = DEFAULT_SEED;
        let mut limits = SolveLimits::DEFAULT;
        let mut json = false;
        let mut rest = args.iter();

        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(Self::Help),
                "--json" => json = true,
                "--project" | "-p" => {
                    let value = rest.next().ok_or("--project needs a path")?;
                    project = Some(PathBuf::from(value));
                }
                "--school" => school = Some(identifier("--school", rest.next())?),
                "--cycle" => cycle = Some(identifier("--cycle", rest.next())?),
                "--term" => term = Some(identifier("--term", rest.next())?),
                "--revalidate" => revalidate = Some(identifier("--revalidate", rest.next())?),
                "--name" => {
                    let value = rest.next().ok_or("--name needs a name")?;
                    name = Some(value.clone());
                }
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
                other => return Err(format!("unknown option {other:?}\n\n{USAGE}")),
            }
        }

        let Some(project) = project else {
            return Err(format!("--project is required\n\n{USAGE}"));
        };

        if let Some(timetable) = revalidate {
            // Refused rather than ignored: a selection beside `--revalidate` would read as
            // though it chose what to revalidate, and it does not — a stored timetable already
            // knows its own scope.
            for (flag, given) in [
                ("--school", school.is_some()),
                ("--cycle", cycle.is_some()),
                ("--term", term.is_some()),
                ("--name", name.is_some()),
            ] {
                if given {
                    return Err(format!(
                        "{flag} means nothing with --revalidate: a stored timetable already \
                         knows its school, cycle and term"
                    ));
                }
            }
            return Ok(Self::Revalidate {
                project,
                timetable,
                json,
            });
        }

        let (Some(school_id), Some(cycle_id)) = (school, cycle) else {
            return Err(format!(
                "--school and --cycle are required. Run `evara validate --project <path> \
                 --list` to see what the project holds.\n\n{USAGE}"
            ));
        };
        Ok(Self::Generate {
            project,
            selection: SchedulingSelection {
                school_id,
                cycle_id,
                term_id: term,
            },
            seed,
            limits,
            name,
            json,
        })
    }
}

fn identifier(flag: &str, value: Option<&String>) -> Result<Uuid, String> {
    let value = value.ok_or_else(|| format!("{flag} needs an identifier"))?;
    Uuid::parse_str(value).map_err(|_| format!("{flag} {value:?} is not an identifier"))
}

fn describe(path: &Path, error: &evara_db::DbError) -> String {
    format!("{} — {error}", file_name(path))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("the selected file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

const fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

const USAGE: &str = "\
Usage: evara save --project <path.evaraproj> --school <id> --cycle <id> [--term <id>]
                  [--name TEXT] [--seed N] [--json]
                  [--max-candidates N] [--max-steps N] [--max-passes N]
       evara save --project <path.evaraproj> --revalidate <timetable-id> [--json]

Generates a timetable and SAVES it into the project, or re-scores one already stored.

This command WRITES. `evara generate` is the read-only form and never touches the
file; saving lives in a command named for saving so the guarantee is one word the
user typed rather than a flag they have to notice.

A save never overwrites. Several timetables may exist for one cycle and term, and
that coexistence is the versioning; replacing one is a save followed by an explicit
delete.

If the school's scheduling inputs changed after the timetable was generated, the save
is refused: every stored reference would still resolve, so nothing else would catch it,
and the result would be a plausible but wrong timetable. Generate again.

Options:
  --project, -p      The .evaraproj file to write into.
  --school ID        Which school. Required unless --revalidate; never defaulted.
  --cycle ID         Which cycle\'s grid. Required unless --revalidate.
  --term ID          Which term. Omit for a grid materialised for the whole year.
  --revalidate ID    Re-score a stored timetable and replace its current findings.
                     The record of how it was generated is left untouched.
  --name TEXT        What to call it. Defaults to the date, the time and the seed.
  --seed N           Seed for the candidate order. Default 2026.
  --max-candidates N Stop after this many candidate evaluations.
  --max-steps N      Stop after this many construction steps.
  --max-passes N     Stop after this many retry passes.
  --json             Print the structured form instead of prose.
  --help, -h         Show this text.

Exit codes:
  0   saved, and every occurrence is placed (or a revalidation was recorded)
  1   saved, and the timetable is partial — usable, with lessons unplaced
  2   the command itself failed, or the project refused the write
  3   nothing was saved: generation was refused, or produced no timetable

See docs/TIMETABLE-PERSISTENCE.md for what is stored and why.
";

#[cfg(test)]
mod tests;
