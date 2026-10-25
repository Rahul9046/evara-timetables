//! Generation orchestration: validate, generate off the main thread, preview, save.
//!
//! # What this module is, and is not
//!
//! It is the `solve` row `super`'s table has reserved since Phase 1, and it **orchestrates**.
//! It holds no scheduling rule, chooses no heuristic and writes no SQL. Every decision it
//! makes is about *which crate to call in what order and what to do with the answer*:
//!
//! ```text
//!  validate   evara_db::Scheduling::validate      -> AssemblyReport      (read-only)
//!  generate   evara_db::Scheduling::load   ──┐
//!             evara_db::Scheduling::labels   │  on the command thread, lock held
//!             evara_constraints::Registry  ──┘
//!             evara_solver::generate_with       -> SolveOutcome          on a worker, no lock
//!  preview    evara_report::projection::project -> TimetableProjection   pure
//!  save       evara_db::Timetables::save_generated                       lock held, one txn
//! ```
//!
//! # The lock is not held while the engine runs
//!
//! `AppState.workspace` is a `Mutex<Workspace>` and every command in this crate takes the
//! guard. A 2 000-occurrence solve takes about 900 ms
//! ([TIMETABLE-CONSTRUCTION-SOLVER.md](../../../../docs/TIMETABLE-CONSTRUCTION-SOLVER.md)
//! §14), and holding the lock for that long would freeze the whole application.
//!
//! It does not have to be held. `ARCHITECTURE.md` §3.3: the solver *"receives a fully
//! materialised problem in memory and returns a solution. It never opens a database."* So
//! [`timetable_generate_start`] compiles under the lock, releases it, and **moves** the owned
//! `Assembly` to a dedicated thread. `evara_db::scheduling`'s `send_assertions` test is what
//! keeps that possible as the types change.
//!
//! # The freshness guarantee is M7's and is untouched
//!
//! A save compares the `Assembly`'s change-log watermark against the feed **inside the
//! writing transaction** and refuses `DbError::InputsChanged`. Nothing here compares a
//! watermark — ADR 0030's rule, which `CLAUDE.md` §3 restates for this milestone — and
//! nothing here re-loads the assembly at save time, because the identity check that catches a
//! mismatched index depends on it being the same one.

// See `project.rs` for why this lint is inapplicable to a module of command handlers.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;
use std::sync::Arc;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{ProblemIdentity, SolveOutcome, SolveStatus, Termination};
use evara_db::RunProvenance;
use evara_db::scheduling::{Assembly, SchedulingSelection};
use evara_report::projection::{
    LessonFlags, ProjectionInput, ScopeGrid, ScopeKind, TimetableProjection,
};
use evara_solver::{ConstructionSolver, GenerateRequest, SolveConfig, SolveLimits, generate_with};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use uuid::Uuid;

use super::error::{SetupError, SetupResult};
use super::project::AppState;
use super::session::{CoalescingSink, Progress, SaveRefusal, Session, SessionPhase};
use super::views::{self, TimetableSummary};
use super::workspace;

/// The event a running solve reports progress on.
pub const PROGRESS_EVENT: &str = "solver://progress";
/// The event a finished solve announces itself on.
pub const FINISHED_EVENT: &str = "solver://finished";

/// What pre-solve validation found, for the screen that decides whether to offer Generate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ValidationView {
    /// Whether the inputs are fit to solve at all.
    ///
    /// `false` disables generation: ADR 0033 §3's refusals are inputs with **no value range**,
    /// so a solve over one could not place the affected occurrences and would report them as
    /// *"could not be placed"* — which reads as a difficulty rather than the unanswered setup
    /// question it is.
    pub may_solve: bool,
    /// Every finding, most severe first, as the validator ordered them.
    pub findings: Vec<FindingLine>,
    /// How many occurrences the problem holds.
    pub occurrences: usize,
    /// Stored restrictions this engine does not enforce.
    pub unenforced_restrictions: u32,
    /// How many timetables this `(cycle, term)` already holds.
    ///
    /// Shown before generating because a save **always inserts** (ADR 0042 D3), and a count
    /// seen beforehand is easier to accept than a fourth version appearing in a list.
    pub existing_timetables: usize,
}

/// One pre-solve finding, flattened for the interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FindingLine {
    /// The stable diagnostic code.
    pub code: String,
    /// `blocking`, `warning` or `info`.
    pub severity: String,
    /// What is wrong, in the validator's own words.
    pub message: String,
    /// What to do about it, where the validator names a remedy.
    pub remedy: Option<String>,
    /// Whether this finding alone stops the solve.
    pub blocks: bool,
}

/// What starting a generation returns.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GenerateStarted {
    /// The run this is, carried on every event about it.
    pub run_id: Uuid,
    /// How many occurrences it will try to place.
    pub total: usize,
}

/// How a run is doing, for a poll between events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GenerateStatus {
    /// Which run.
    pub run_id: Uuid,
    /// Where in its lifecycle it is.
    pub phase: SessionPhase,
    /// Occurrences placed so far.
    pub placed: usize,
    /// Occurrences the problem holds.
    pub total: usize,
    /// Attempts made, retries included.
    pub steps: u64,
    /// Whether a cancel has been requested.
    pub cancel_requested: bool,
    /// Why the result cannot be saved, or `null` if it can.
    ///
    /// A sentence rather than a flag, because the two unsaveable cases need different things
    /// said about them: a refused run points at the findings, and a failed internal check is
    /// a defect in Evara.
    pub not_saveable: Option<String>,
    /// The timetable this was saved as, once it has been.
    pub saved_as: Option<Uuid>,
}

/// A finished run, with its preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GenerateResult {
    /// Which run.
    pub run_id: Uuid,
    /// Where in its lifecycle it is.
    pub phase: SessionPhase,
    /// What the engine concluded.
    pub outcome: views::OutcomeView,
    /// The preview, projected by the **same** code that renders a saved timetable.
    pub projection: TimetableProjection,
    /// Why the result cannot be saved, or `null` if it can.
    pub not_saveable: Option<String>,
    /// The timetable this was saved as, once it has been.
    pub saved_as: Option<Uuid>,
}

/// What a save wrote.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SavedView {
    /// The timetable row.
    pub timetable: TimetableSummary,
    /// How many entries were written.
    pub entries: usize,
    /// How many unplaced occurrences were recorded.
    pub unscheduled: usize,
    /// How many violations were stored.
    pub violations: usize,
}

/// Limits a caller may override. All counts — never a duration.
///
/// `SOLVER.md` §5: a time limit reads a clock and stops in a different place on a different
/// machine, so the same seed would produce different timetables and a committed baseline would
/// be meaningless. A wall clock reaches the engine only as the user's cancel.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GenerateSettings {
    /// The seed every ordering decision derives from. Defaults to 2026, as the CLI does.
    pub seed: Option<u64>,
    /// The most candidate placements whose legality may be tested.
    pub max_candidates: Option<u64>,
    /// The most occurrences the constructor may attempt.
    pub max_steps: Option<u64>,
    /// The most sweeps over the unplaced occurrences.
    pub max_passes: Option<u32>,
}

impl GenerateSettings {
    /// The engine configuration these settings describe.
    fn config(self) -> SolveConfig {
        let mut limits = SolveLimits::DEFAULT;
        if let Some(candidates) = self.max_candidates {
            limits.max_candidate_evaluations = candidates;
        }
        if let Some(steps) = self.max_steps {
            limits.max_construction_steps = steps;
        }
        if let Some(passes) = self.max_passes {
            limits.max_passes = passes;
        }
        SolveConfig::seeded(self.seed.unwrap_or(2026)).with_limits(limits)
    }

    /// The settings, as the JSON `solve_run.settings` stores.
    ///
    /// Takes only the resolved configuration, because that is what actually ran: a caller's
    /// `None` became a default long before the engine saw it, and recording the request
    /// rather than the resolution would make a stored run unreproducible.
    fn as_json(config: &SolveConfig) -> String {
        format!(
            "{{\"seed\":{},\"maxCandidateEvaluations\":{},\"maxConstructionSteps\":{},\
             \"maxPasses\":{}}}",
            config.seed.get(),
            config.limits.max_candidate_evaluations,
            config.limits.max_construction_steps,
            config.limits.max_passes
        )
    }
}

/// The progress payload, as the frontend receives it.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProgressEvent {
    /// Which run. The frontend ignores every other, which is what stops a stale session's
    /// events reaching the current project's screen.
    pub run_id: Uuid,
    /// Which project epoch this run belongs to.
    pub epoch: u64,
    /// Occurrences placed so far.
    pub placed: usize,
    /// Occurrences the problem holds.
    pub total: usize,
    /// Attempts made, retries included.
    pub steps: u64,
    /// Milliseconds since the run started, measured by the **caller**.
    ///
    /// The engine reads no clock — that is what keeps the same seed reproducible — so elapsed
    /// time is this layer's to measure and report.
    pub elapsed_ms: u64,
}

/// The finished payload.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FinishedEvent {
    /// Which run.
    pub run_id: Uuid,
    /// Which project epoch.
    pub epoch: u64,
    /// Whether a result is there to fetch.
    ///
    /// `false` when the session was discarded or the project closed while the worker ran, so
    /// the frontend knows not to ask for a result that will not come.
    pub has_result: bool,
    /// Milliseconds the run took, measured by the caller.
    pub elapsed_ms: u64,
}

// =======================================================================================
// Pre-solve validation
// =======================================================================================

/// Validates one school's scheduling inputs. **Writes nothing.**
///
/// Runs before the Generate button is offered, which is M3's and M6's gate restated in the
/// interface: *"a problem whose inputs were refused is not searched at all"*.
#[tauri::command]
pub fn timetable_validate(
    state: State<'_, AppState>,
    selection: SchedulingSelection,
) -> SetupResult<ValidationView> {
    let mut ws = workspace(&state)?;
    // One load, not two. `Scheduling::validate` is `load` followed by `AssemblyReport::of`,
    // and this screen needs the assembly as well — for the occurrence count and the
    // unenforced-restriction figure — so it does the two halves itself rather than loading
    // once for the report and again for the problem. Validation re-runs on every change of
    // selection, so the difference is not academic.
    let assembly = ws.scheduling()?.load(&selection)?;
    let report = evara_db::AssemblyReport::of(&assembly);
    let existing = ws
        .timetables()?
        .for_cycle(selection.cycle_id, selection.term_id)?
        .len();

    Ok(ValidationView {
        may_solve: report.may_solve(),
        findings: report
            .findings
            .findings
            .iter()
            .map(|finding| FindingLine {
                code: finding.code.key().to_owned(),
                severity: severity_key(finding.severity()),
                message: finding.message.clone(),
                remedy: finding.remedy.clone(),
                blocks: finding.severity().blocks_solve(),
            })
            .collect(),
        occurrences: assembly.problem.activity_count(),
        unenforced_restrictions: unenforced(&assembly),
        existing_timetables: existing,
    })
}

/// A [`Severity`](evara_core::scheduling::Severity) as the interface spells it.
fn severity_key(severity: evara_core::scheduling::Severity) -> String {
    use evara_core::scheduling::Severity;
    match severity {
        Severity::Blocking => "blocking",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
    .to_owned()
}

/// How many stored restrictions this engine does not enforce.
fn unenforced(assembly: &Assembly) -> u32 {
    use evara_db::scheduling::Completeness;
    match assembly.completeness {
        Completeness::Complete => 0,
        Completeness::RestrictionsNotEnforced { rules } => u32::try_from(rules).unwrap_or(u32::MAX),
    }
}

// =======================================================================================
// Generation
// =======================================================================================

/// Starts a generation on its own thread and returns immediately.
///
/// # The sequence, and why the lock is released before the engine runs
///
/// Everything that needs the database happens first, under the lock: the problem is compiled,
/// the labels are read, the school's constraint rows are compiled over the mandatory set, and
/// pre-solve validation is run. The lock is then dropped and the owned `Assembly` is moved to
/// a worker. See this module's header.
///
/// # Errors
///
/// Anything the loader refuses — an unknown school, a cycle belonging to another school, a
/// corrupt stored identifier. A problem whose *inputs* are unfit is **not** an error: it
/// produces a `Termination::InputRejected` outcome whose findings the preview renders, which
/// is what lets a user see what to fix.
#[tauri::command]
pub fn timetable_generate_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    selection: SchedulingSelection,
    settings: Option<GenerateSettings>,
) -> SetupResult<GenerateStarted> {
    let settings = settings.unwrap_or_default();
    let config = settings.config();
    let epoch = state.epoch.get();

    // --- under the lock -------------------------------------------------------------
    let (assembly, labels, rules, report) = {
        let mut ws = workspace(&state)?;
        let assembly = ws.scheduling()?.load(&selection)?;
        let labels = ws.scheduling()?.labels(&selection)?;
        let stored = ws.constraints()?.stored_for(selection.school_id)?;
        let rules =
            Registry::compile(&stored, selection.term_id).map_err(|error| SetupError::Invalid {
                message: format!("this school's constraint configuration cannot be read: {error}"),
            })?;
        let report = evara_db::AssemblyReport::of(&assembly);
        (assembly, labels, rules, report)
    };
    // --- lock released --------------------------------------------------------------

    let total = assembly.problem.activity_count();
    let identity = ProblemIdentity {
        school: assembly.problem.context.school.id,
        cycle: selection.cycle_id,
        term: selection.term_id,
    };
    let unenforced_restrictions = unenforced(&assembly);
    let may_solve = report.may_solve();
    let preflight = report.findings.clone();

    let run_id = Uuid::now_v7();
    let session = Arc::new(Session::new(
        run_id,
        selection,
        Arc::new(assembly),
        Arc::new(labels),
        Arc::new(rules),
        epoch,
        now(),
    ));
    state.generations.insert(Arc::clone(&session));

    let life = session.lifecycle();
    let cancel = session.cancellation();
    let worker = Arc::clone(&session);
    let handle = app.clone();
    let started = std::time::Instant::now();

    std::thread::Builder::new()
        .name(format!("evara-solve-{run_id}"))
        .spawn(move || {
            let emitter = handle.clone();
            let sink = CoalescingSink::new(Arc::clone(&life), move |progress: Progress| {
                // A failed emit is not worth failing a solve over: the window may have gone.
                let _ = emitter.emit(
                    PROGRESS_EVENT,
                    ProgressEvent {
                        run_id,
                        epoch,
                        placed: progress.placed,
                        total: progress.total,
                        steps: progress.steps,
                        elapsed_ms: elapsed_ms(started),
                    },
                );
            });

            let outcome = generate_with(
                &ConstructionSolver,
                &GenerateRequest {
                    problem: &worker.assembly.problem,
                    identity,
                    constraints: &worker.constraints,
                    preflight: preflight.clone(),
                    may_solve,
                    unenforced_restrictions,
                    config,
                },
                &sink,
                &cancel,
            );

            worker.finish(outcome);
            let has_result = worker.phase() == SessionPhase::Finished;
            let _ = handle.emit(
                FINISHED_EVENT,
                FinishedEvent {
                    run_id,
                    epoch,
                    has_result,
                    elapsed_ms: elapsed_ms(started),
                },
            );
        })
        .map_err(|error| SetupError::Failed {
            message: format!("Evara could not start the generation thread: {error}"),
        })?;

    Ok(GenerateStarted { run_id, total })
}

/// How a run is doing.
#[tauri::command]
pub fn timetable_generate_status(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> SetupResult<GenerateStatus> {
    let session = live_session(&state, run_id)?;
    let progress = session.progress();
    Ok(GenerateStatus {
        run_id,
        phase: session.phase(),
        placed: progress.placed,
        total: progress.total,
        steps: progress.steps,
        cancel_requested: session.cancel_requested(),
        not_saveable: session.saveable().map(str::to_owned),
        saved_as: session.saved_as(),
    })
}

/// The finished run and its preview.
///
/// The projection comes from the **same** `evara-report` call a saved timetable's view uses,
/// which is what makes *"what I previewed is what I saved"* structural rather than hoped for.
///
/// # Errors
///
/// [`SetupError::Conflict`] while the run is still going — the result does not exist yet, and
/// a half-built arrangement is not a preview.
#[tauri::command]
pub fn timetable_generate_result(
    state: State<'_, AppState>,
    run_id: Uuid,
) -> SetupResult<GenerateResult> {
    let session = live_session(&state, run_id)?;
    let Some(outcome) = session.outcome() else {
        return Err(match session.phase() {
            SessionPhase::Running => SetupError::Invalid {
                message: "This generation has not finished yet.".to_owned(),
            },
            _ => SetupError::NotFound {
                entity: "timetable".to_owned(),
                message: "This generation was discarded.".to_owned(),
            },
        });
    };

    let flags: BTreeMap<Uuid, LessonFlags> = BTreeMap::new();
    let projection = evara_report::projection::project(ProjectionInput {
        problem: &session.assembly.problem,
        assignment: &outcome.assignment,
        labels: &session.labels,
        flags: &flags,
    });

    Ok(GenerateResult {
        run_id,
        phase: session.phase(),
        outcome: views::OutcomeView::of(&outcome, &session.assembly.problem),
        projection,
        not_saveable: session.saveable().map(str::to_owned),
        saved_as: session.saved_as(),
    })
}

/// One scope's laid-out grid, for a run still in memory.
///
/// # Why the layout is a second call rather than part of the result
///
/// A `ScopeGrid` is `days × periods` cells — 40 for a five-day, eight-period school — and
/// there is one per teacher, room, section and student. A large school has hundreds of
/// scopes, so returning every grid with the result would send tens of thousands of cells to
/// render one of them.
///
/// The alternative — laying out in TypeScript — is the one thing this must not do: the
/// `rowSpan` decision and the overlap detection behind it are scheduling questions with a
/// single implementation, and a second copy in the view layer is what `CLAUDE.md` §2 forbids.
///
/// So the projection crosses once and the **layout** is asked for per scope. It needs no
/// database: the session already holds the problem, the labels and the outcome, and
/// re-projecting 2 000 occurrences takes single-digit milliseconds.
#[tauri::command]
pub fn timetable_generate_scope(
    state: State<'_, AppState>,
    run_id: Uuid,
    scope_kind: ScopeKind,
    scope_id: Option<Uuid>,
) -> SetupResult<ScopeGrid> {
    let session = live_session(&state, run_id)?;
    let outcome = session.outcome().ok_or_else(|| SetupError::Invalid {
        message: "This generation has no result to lay out yet.".to_owned(),
    })?;
    let flags: BTreeMap<Uuid, LessonFlags> = BTreeMap::new();
    let projection = evara_report::projection::project(ProjectionInput {
        problem: &session.assembly.problem,
        assignment: &outcome.assignment,
        labels: &session.labels,
        flags: &flags,
    });
    views::lay_out_scope(&projection, &session.labels, scope_kind, scope_id)
}

/// Asks a run to stop.
///
/// Returns whether there was something running to stop. `false` for a run that had already
/// finished — not an error, and reporting success would tell the user their cancel did
/// something it did not.
#[tauri::command]
pub fn timetable_generate_cancel(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<bool> {
    match state.generations.get(run_id) {
        Some(session) => Ok(session.request_cancel()),
        // An unknown run is `false` rather than an error: a cancel arriving after a project
        // close has nothing to act on, and that is the honest answer rather than a failure.
        None => Ok(false),
    }
}

/// Abandons a run, cancelling it if it is still going.
#[tauri::command]
pub fn timetable_generate_discard(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<bool> {
    Ok(state.generations.discard(run_id))
}

// =======================================================================================
// Saving
// =======================================================================================

/// Saves a generated timetable. **The one command here that writes.**
///
/// # What is guarded, and where
///
/// | Guard | Where | Why |
/// | --- | --- | --- |
/// | unfinished, failed, refused, discarded | [`Session::claim_save`] | before a transaction opens |
/// | a second concurrent save | [`Session::claim_save`]'s compare-and-swap | two clicks must not write twice |
/// | a different project | [`Session::epoch`] | a solve begun against one file cannot be saved into another |
/// | **a scheduling input moved** | `save_generated`, inside the transaction | M7's watermark. Nothing here compares one |
/// | a mismatched problem index | `save_generated` | the assembly held by the session is the one the outcome was built from |
///
/// # Always inserts
///
/// ADR 0042 D3: several timetables coexist for one `(cycle, term)` and that coexistence *is*
/// the versioning. There is no overwrite and no "replace" — replacement is a save followed by
/// an explicit delete.
///
/// # Errors
///
/// [`SetupError::InputsChanged`] if anything the problem was compiled from has moved. The
/// remedy is to generate again, and the message says so — a screen that showed a raw error
/// would make a correct refusal look like a bug.
#[tauri::command]
pub fn timetable_save(
    state: State<'_, AppState>,
    run_id: Uuid,
    name: String,
) -> SetupResult<SavedView> {
    let session = live_session(&state, run_id)?;
    let outcome = session.claim_save().map_err(refusal)?;

    let config_seed = outcome.seed.get();
    let provenance = RunProvenance {
        // A generation, and no scope: `V18`'s two columns, defaulted explicitly here rather
        // than relying on the schema default, so a reader of this call sees what is recorded.
        kind: evara_db::RUN_GENERATION.to_owned(),
        scope: String::from("{}"),
        engine: "construction".to_owned(),
        settings: GenerateSettings::as_json(&SolveConfig::seeded(config_seed)),
        started_at: session.started_at.clone(),
        finished_at: now(),
    };

    // The independent scoring, by the reference scorer — the same call `evara save` makes.
    // What is stored as the generation evaluation is this, not the engine's own belief about
    // its result. Computed before the lock is taken, because it needs no database.
    let reference = evaluate(
        &session.assembly.problem,
        &outcome.assignment,
        &session.constraints,
    );

    let written = {
        let mut ws = match workspace(&state) {
            Ok(ws) => ws,
            Err(error) => {
                session.save_failed();
                return Err(error);
            }
        };
        match ws.timetables() {
            Ok(mut timetables) => timetables.save_generated(
                &session.assembly,
                &outcome,
                &reference.violations,
                &provenance,
                name.trim(),
            ),
            Err(error) => Err(error),
        }
    };

    match written {
        Ok(saved) => {
            session.save_succeeded(saved.timetable.id);
            Ok(SavedView {
                timetable: TimetableSummary::of(&saved.timetable),
                entries: saved.entries,
                unscheduled: saved.unscheduled,
                violations: saved.violations,
            })
        }
        Err(error) => {
            // Put the session back so a refusal the user can act on — a duplicate name, a
            // read-only project — leaves the result they can see on screen still saveable.
            session.save_failed();
            Err(error.into())
        }
    }
}

// =======================================================================================
// Helpers
// =======================================================================================

/// The session for one run, if it belongs to the project that is open now.
///
/// # Why the epoch is checked here
///
/// A solve runs for about a second. In that time a user can close the project and open
/// another, and the session's `Assembly` then describes a database that is no longer open.
/// Saving it would offer one file's arrangement to another file's writer — which
/// `save_generated` would refuse on identity, but only after a transaction had opened, and
/// only if the identifiers happened not to collide. Refusing here is the earlier and clearer
/// of the two.
fn live_session(state: &State<'_, AppState>, run_id: Uuid) -> SetupResult<Arc<Session>> {
    let session = state
        .generations
        .get(run_id)
        .ok_or_else(|| SetupError::NotFound {
            entity: "timetable".to_owned(),
            message: "Evara has no record of this generation. It may have been discarded, \
                      or the campus file may have been closed since."
                .to_owned(),
        })?;
    if session.epoch != state.epoch.get() {
        // The remedy really is *generate again*, which is what `InputsChanged` means: the
        // arrangement was built against a database that is no longer the open one, so there
        // is nothing to re-read and nothing to confirm.
        return Err(SetupError::InputsChanged {
            message: "This timetable was generated against a campus file that is no longer \
                      open, so nothing was saved. Generate again."
                .to_owned(),
        });
    }
    Ok(session)
}

/// A [`SaveRefusal`] as the interface's error contract.
fn refusal(refusal: SaveRefusal) -> SetupError {
    match refusal {
        SaveRefusal::StillRunning => SetupError::Invalid {
            message: "This generation has not finished yet, so there is nothing to save."
                .to_owned(),
        },
        SaveRefusal::AlreadySaving => SetupError::Invalid {
            message: "This timetable is already being saved.".to_owned(),
        },
        // Not an error for the user: it already succeeded. The identifier comes back so the
        // interface can offer to open it rather than reporting a failure.
        SaveRefusal::AlreadySaved { timetable_id } => SetupError::AlreadySaved {
            timetable_id: timetable_id.to_string(),
            message: "This timetable has already been saved.".to_owned(),
        },
        SaveRefusal::Discarded => SetupError::NotFound {
            entity: "timetable".to_owned(),
            message: "This generation was discarded.".to_owned(),
        },
        SaveRefusal::NotSaveable { reason } => SetupError::NotSaveable {
            message: capitalise(reason),
        },
    }
}

/// A sentence, with its first letter raised.
fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str() + ".",
        None => String::new(),
    }
}

/// Milliseconds since an instant, saturating.
fn elapsed_ms(since: std::time::Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The current instant, ISO-8601 UTC with millisecond precision.
///
/// The format `CLAUDE.md` §3 requires, and the one the repositories store.
fn now() -> String {
    evara_db::now_iso8601()
}

/// Whether an outcome describes a timetable at all.
///
/// Re-exported for the views module, which says the same thing about a *stored* timetable.
#[must_use]
pub const fn is_a_timetable(outcome: &SolveOutcome) -> bool {
    !matches!(
        outcome.termination,
        Termination::InputRejected | Termination::InternalCheckFailed
    ) && !matches!(
        outcome.status,
        SolveStatus::Failed | SolveStatus::Infeasible
    )
}
