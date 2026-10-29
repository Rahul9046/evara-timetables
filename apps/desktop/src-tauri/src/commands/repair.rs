//! The eight repair commands: choose a scope, check the locks, run, review, apply.
//!
//! # The shape, and what each command is for
//!
//! | Command | Answers | Writes |
//! | --- | --- | --- |
//! | [`repair_scopes`] | *"what could I repair?"* — the scopes this timetable offers, with the count each would make movable | no |
//! | [`repair_preflight`] | *"may I repair this at all?"* — the lock validation | no |
//! | [`repair_start`] | runs it, on a worker thread | no |
//! | [`repair_status`] | phase and progress | no |
//! | [`repair_result`] | the outcome and the diff | no |
//! | [`repair_cancel`] | sets the flag. Changes no state | no |
//! | [`repair_apply`] | the one write | **yes** |
//! | [`repair_discard`] | drops the session | no |
//!
//! `repair_scopes` and `repair_preflight` are separate because the first is asked *before* the
//! user has chosen a scope and the second *after*: a screen needs the menu before it can
//! validate a selection from it.
//!
//! # No database lock is held while the solver runs
//!
//! `ARCHITECTURE.md` §3.3, and the pattern [`super::solve`] already proves: take the workspace
//! lock, compile the `Assembly`, read the labels, compile the rules, read the baseline snapshot,
//! **release the lock**, move the owned data to a worker thread.
//!
//! Note the ordering, which matters in one direction only: the `Assembly` is compiled **first**,
//! capturing its watermark, and the baseline snapshot is read **after**. An input that moves in
//! between is then older than the watermark and the save refuses it. The reverse order would
//! leave such a change invisible to both guards, because `timetable_entry` is in
//! `SOLVE_OUTPUT_TABLES` and the stamp only watches entries.
//!
//! # Nothing here compares a guard
//!
//! The watermark, the arrangement stamp and the source revision are all compared by `evara-db`
//! inside the transaction that writes — ADR 0030's rule, which `CLAUDE.md` §3 restates. The
//! project epoch is this layer's, and is checked before a transaction opens.

// See `project.rs` for why this lint is inapplicable to a module of command handlers: Tauri
// deserialises every parameter, so each one must be owned.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;
use std::sync::Arc;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{
    ConstraintLevel, LabelSet, ProblemIdentity, RepairScope, ScopeError, derive_movable,
};
use evara_db::scheduling::{Assembly, SchedulingSelection};
use evara_db::{EditGuard, RUN_REPAIR, RepairSave, RunProvenance, UnexpressibleLockCause};
use evara_report::{RepairDiff, RepairDiffInput, diff};
use evara_solver::{RepairRequest, repair_with};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use uuid::Uuid;

use super::error::{SetupError, SetupResult};
use super::project::AppState;
use super::repair_session::{ApplyRefusal, RepairPhase, RepairSession, RepairSessionInput};
use super::session::{CoalescingSink, Progress, ProgressReporter};
use super::solve::GenerateSettings;
use super::workspace;

/// Progress events, so a screen can show a bar rather than a spinner.
pub const PROGRESS_EVENT: &str = "repair://progress";
/// Emitted once, when the worker returns.
pub const FINISHED_EVENT: &str = "repair://finished";

/// One scope the user could choose, with what it would reach.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScopeOption {
    /// The scope itself, to pass back to `repair_start`.
    pub scope: RepairScope,
    /// Its stable key.
    pub key: String,
    /// What a human calls it.
    pub label: String,
    /// How many occurrences it would make movable.
    pub movable: usize,
    /// How many would stay frozen.
    pub frozen: usize,
    /// Whether it is derived from the baseline rather than the inputs.
    ///
    /// `true` for a room or a day. A positional scope cannot reach an unscheduled lesson, and
    /// the screen must say so **before** the run rather than leave it to the diff.
    pub positional: bool,
    /// How many unplaced occurrences this scope cannot reach.
    ///
    /// Always `0` for a structural scope. Non-zero for a positional one whenever the timetable
    /// has unscheduled lessons, which is exactly when the warning is needed.
    pub unreachable_unplaced: usize,
}

/// What the scope picker needs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairScopes {
    /// The timetable these are about.
    pub timetable_id: Uuid,
    /// Every scope on offer, whole-timetable first.
    pub options: Vec<ScopeOption>,
    /// How many occurrences the problem holds.
    pub total: usize,
    /// How many the baseline leaves unplaced.
    pub unplaced: usize,
}

/// One reason a repair is refused before it runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PreflightFinding {
    /// A stable key, for a message catalogue.
    pub code: String,
    /// Whether this stops the repair.
    pub blocking: bool,
    /// What a human reads.
    pub message: String,
    /// What to do about it.
    pub remedy: String,
    /// The lesson involved, where there is one.
    pub activity_id: Option<Uuid>,
}

/// Whether a repair may proceed, and what stands in the way.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairPreflight {
    /// Whether `repair_start` would be accepted.
    pub may_repair: bool,
    /// Everything found, blocking first.
    pub findings: Vec<PreflightFinding>,
    /// How many occurrences the chosen scope would make movable.
    pub movable: usize,
    /// How many would stay frozen.
    pub frozen: usize,
    /// How many locks the baseline holds.
    pub locked: usize,
}

/// What `repair_start` hands back.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairStarted {
    /// The session identifier every later command names.
    pub run_id: Uuid,
    /// How many occurrences the run will attempt.
    pub movable: usize,
    /// How many are frozen.
    pub frozen: usize,
}

/// How a repair is doing.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairStatus {
    /// Which session.
    pub run_id: Uuid,
    /// Where it is.
    pub phase: RepairPhase,
    /// Occurrences placed so far.
    pub placed: usize,
    /// Occurrences it will attempt.
    pub total: usize,
    /// Attempts made.
    pub steps: u64,
    /// Whether a cancel has been asked for.
    pub cancel_requested: bool,
}

/// The finished repair, ready to review.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairResult {
    /// Which session.
    pub run_id: Uuid,
    /// Where it is.
    pub phase: RepairPhase,
    /// The scope it ran over.
    pub scope: RepairScope,
    /// Why it cannot be applied, or `None` if it can.
    pub not_applyable: Option<String>,
    /// The before-and-after.
    pub diff: RepairDiff,
    /// How the engine stopped.
    pub termination: String,
    /// What came back.
    pub status: String,
    /// Whether a limit stopped it, so the preview can say so rather than imply a conclusion.
    pub stopped_at_a_limit: bool,
    /// The internal consistency failure, where there was one. A defect in Evara.
    pub defect: Option<String>,
    /// The timetable it became, once applied.
    pub applied_as: Option<Uuid>,
}

/// What `repair_apply` wrote.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairApplied {
    /// The new timetable.
    pub timetable_id: Uuid,
    /// The timetable it was repaired from, untouched.
    pub based_on: Uuid,
    /// How many entries were written.
    pub entries: usize,
}

/// A progress event.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairProgressEvent {
    /// Which session.
    pub run_id: Uuid,
    /// Which project.
    pub epoch: u64,
    /// Occurrences placed so far.
    pub placed: usize,
    /// Occurrences being attempted.
    pub total: usize,
    /// Attempts made.
    pub steps: u64,
}

/// Emitted once, when the worker returns.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairFinishedEvent {
    /// Which session.
    pub run_id: Uuid,
    /// Which project.
    pub epoch: u64,
    /// Whether there is a result to read.
    pub has_result: bool,
}

/// What a caller asks to repair.
///
/// Internal: the commands take `selection`, `timetableId` and `scope` as separate parameters,
/// the way [`super::solve`]'s do, because `SchedulingSelection` deliberately does not cross the
/// boundary as a named type. This bundles them for the functions that need all three.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairRequestInput {
    /// Which school, cycle and term.
    pub selection: SchedulingSelection,
    /// Which timetable.
    pub timetable_id: Uuid,
    /// What may move.
    pub scope: RepairScope,
}

// =======================================================================================
// Reading
// =======================================================================================

/// What scopes this timetable offers.
///
/// # Errors
///
/// Anything the loader refuses, or an unknown timetable.
#[tauri::command]
pub fn repair_scopes(
    state: State<'_, AppState>,
    selection: SchedulingSelection,
    timetable_id: Uuid,
) -> SetupResult<RepairScopes> {
    let (assembly, labels, baseline) = workspace_parts(&state, &selection, timetable_id)?;
    let problem = &assembly.problem;
    let total = problem.activities.len();
    let unplaced = total - baseline.scheduled_count();

    let mut options = Vec::new();
    push_option(
        &mut options,
        problem,
        &baseline,
        RepairScope::Whole,
        "Whole timetable",
    );

    for section in &problem.sections {
        push_option(
            &mut options,
            problem,
            &baseline,
            RepairScope::ClassSection {
                section_id: section.id,
            },
            &labels
                .section(section.id)
                .map_or_else(|| section.name.clone(), |label| label.short.clone()),
        );
    }
    for teacher in &problem.teachers {
        push_option(
            &mut options,
            problem,
            &baseline,
            RepairScope::Teacher {
                teacher_id: teacher.id,
            },
            &teacher.name,
        );
    }
    for room in &problem.rooms {
        push_option(
            &mut options,
            problem,
            &baseline,
            RepairScope::Room { room_id: room.id },
            &labels
                .room(room.id)
                .map_or_else(|| room.name.clone(), |label| label.name.clone()),
        );
    }
    for day in &problem.context.cycle_days {
        push_option(
            &mut options,
            problem,
            &baseline,
            RepairScope::CycleDay {
                cycle_day_id: day.id,
            },
            &day.label,
        );
    }

    Ok(RepairScopes {
        timetable_id,
        options,
        total,
        unplaced,
    })
}

/// Whether a repair may proceed over this scope.
///
/// # The lock validation
///
/// Three classes, all before any solving:
///
/// - **Unexpressible** — a lock the compiled problem cannot represent. **Blocking**, because the
///   lock is absent from `Problem::pinned`, so nothing would freeze it and a repair whose scope
///   included the occurrence would *move* it.
/// - **Contradictory** — two locks that clash with each other. Blocking.
/// - **Impossible in context** — a lock that clashes with a frozen out-of-scope placement.
///   Blocking, with both sides named.
///
/// Read at HARD only. A frozen-only evaluation leaves every in-scope occurrence unplaced, so
/// MEDIUM `activity-scheduled` fires once per movable occurrence; reporting those would show the
/// user one finding per lesson and call a healthy timetable catastrophic.
///
/// A HARD violation among **unlocked** frozen occurrences is reported and does **not** block:
/// that is pre-existing damage the scope excludes, and refusing it would refuse repair on
/// exactly the timetables repair exists for.
///
/// # Errors
///
/// Anything the loader refuses, an unknown timetable, or a malformed scope.
#[tauri::command]
pub fn repair_preflight(
    state: State<'_, AppState>,
    selection: SchedulingSelection,
    timetable_id: Uuid,
    scope: RepairScope,
) -> SetupResult<RepairPreflight> {
    preflight_of(
        &state,
        &RepairRequestInput {
            selection,
            timetable_id,
            scope,
        },
    )
}

/// The pre-flight, over an already-bundled request.
fn preflight_of(
    state: &State<'_, AppState>,
    input: &RepairRequestInput,
) -> SetupResult<RepairPreflight> {
    let (assembly, _labels, baseline, pins, rules) =
        load_for_repair(state, &input.selection, input.timetable_id)?;

    let mut findings = Vec::new();

    // 1. Unexpressible locks. Blocking.
    for lock in &pins.unexpressible {
        let (message, remedy) = describe_unexpressible(lock.cause);
        findings.push(PreflightFinding {
            code: lock.cause.key().to_owned(),
            blocking: true,
            message,
            remedy,
            activity_id: Some(lock.activity_id),
        });
    }

    let movable = match derive_movable(&assembly.problem, &baseline, &input.scope) {
        Ok(movable) => movable,
        Err(error) => {
            return Ok(RepairPreflight {
                may_repair: false,
                findings: vec![scope_finding(&error)],
                movable: 0,
                frozen: assembly.problem.activities.len(),
                locked: pins.locked,
            });
        }
    };

    // 2 and 3. The frozen set, evaluated by the reference scorer -- the canonical registry
    // answering the canonical question, with no second engine and no new rule.
    let frozen_only = evara_core::scheduling::frozen_only(&baseline, &movable);
    let report = evaluate(&assembly.problem, &frozen_only, &rules);
    let locked_set: Vec<usize> = pins
        .pinned
        .iter()
        .enumerate()
        .filter_map(|(at, pin)| pin.map(|_| at))
        .collect();

    for violation in report
        .violations
        .iter()
        .filter(|v| v.level == ConstraintLevel::Hard)
    {
        let involves_lock = violation
            .activities
            .iter()
            .any(|at| locked_set.contains(&at.get()));
        let names = violation
            .activities
            .iter()
            .filter_map(|at| assembly.index.activity_id(*at))
            .collect::<Vec<_>>();
        findings.push(PreflightFinding {
            code: violation.constraint_type.key().to_owned(),
            // ONLY a lock blocks. Pre-existing damage among unlocked frozen placements is
            // reported, because the chosen scope cannot reach it and refusing would make the
            // feature useless on the timetables it is for.
            blocking: involves_lock,
            message: if involves_lock {
                format!(
                    "A locked lesson cannot stay where it is: {}",
                    violation.detail
                )
            } else {
                format!(
                    "Already present, and outside the scope you chose: {}",
                    violation.detail
                )
            },
            remedy: if involves_lock {
                "Remove the lock, or move the lesson it conflicts with, then repair again."
                    .to_owned()
            } else {
                "This scope cannot reach it. Repair a wider scope to address it.".to_owned()
            },
            activity_id: names.first().copied(),
        });
    }

    findings.sort_by_key(|finding| !finding.blocking);
    let may_repair = !findings.iter().any(|finding| finding.blocking);

    Ok(RepairPreflight {
        may_repair,
        findings,
        movable: movable.len(),
        frozen: movable.frozen_count(),
        locked: pins.locked,
    })
}

// =======================================================================================
// Running
// =======================================================================================

/// Starts a repair on a worker thread.
///
/// # Errors
///
/// Anything the loader refuses, an unknown timetable, a malformed scope, a baseline that cannot
/// be read against the problem, or a blocking pre-flight finding.
#[tauri::command]
pub fn repair_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    selection: SchedulingSelection,
    timetable_id: Uuid,
    scope: RepairScope,
    settings: Option<GenerateSettings>,
) -> SetupResult<RepairStarted> {
    let input = RepairRequestInput {
        selection,
        timetable_id,
        scope,
    };
    let epoch = state.epoch.get();
    let config = settings.unwrap_or_default().solve_config();

    // The pre-flight is re-run here rather than trusted from the interface: a caller that
    // skipped it must not be able to start a repair over contradictory locks.
    let checked = preflight_of(&state, &input)?;
    if !checked.may_repair {
        return Err(SetupError::Invalid {
            message: blocking_message(&checked),
        });
    }

    let (assembly, labels, baseline, pins, rules, stamp, flags, source_rev) =
        load_for_start(&state, &input.selection, input.timetable_id)?;

    let movable = derive_movable(&assembly.problem, &baseline, &input.scope).map_err(|error| {
        SetupError::Invalid {
            message: scope_message(&error),
        }
    })?;

    // The locks become pins, so `activity-fixed` judges the same thing the freeze enforces.
    // The loader never fills this -- a lock is an output of a solve (ADR 0045 §4).
    let problem = {
        let mut problem = assembly.problem.clone();
        problem.pinned = pins.pinned;
        problem
    };

    let identity = ProblemIdentity {
        school: assembly.problem.context.school.id,
        cycle: input.selection.cycle_id,
        term: input.selection.term_id,
    };
    let report = evara_db::AssemblyReport::of(&assembly);
    let may_solve = report.may_solve();
    let preflight = report.findings.clone();
    let unenforced = unenforced_count(&assembly);

    let run_id = Uuid::now_v7();
    let movable_count = movable.len();
    let frozen_count = movable.frozen_count();

    let session = Arc::new(RepairSession::new(RepairSessionInput {
        run_id,
        selection: input.selection,
        source: input.timetable_id,
        source_rev,
        scope: input.scope.clone(),
        assembly: Arc::new(assembly),
        labels: Arc::new(labels),
        constraints: Arc::new(rules),
        baseline: Arc::new(baseline),
        movable: Arc::new(movable),
        stamp: Arc::new(stamp),
        flags: Arc::new(flags),
        epoch,
        started_at: now(),
    }));
    state.repairs.insert(Arc::clone(&session));

    spawn_worker(
        &app,
        &session,
        problem,
        WorkerInput {
            identity,
            preflight,
            may_solve,
            unenforced,
            config,
            epoch,
        },
    )?;

    Ok(RepairStarted {
        run_id,
        movable: movable_count,
        frozen: frozen_count,
    })
}

/// What the worker needs that is not the session.
struct WorkerInput {
    identity: ProblemIdentity,
    preflight: evara_core::scheduling::DiagnosticReport,
    may_solve: bool,
    unenforced: u32,
    config: evara_solver::SolveConfig,
    epoch: u64,
}

/// Moves the owned problem to a worker thread and runs the repair there.
///
/// The workspace lock is already released by the time this is called -- `ARCHITECTURE.md` §3.3.
fn spawn_worker(
    app: &tauri::AppHandle,
    session: &Arc<RepairSession>,
    problem: evara_core::scheduling::Problem,
    input: WorkerInput,
) -> SetupResult<()> {
    let life = session.lifecycle();
    let cancel = session.cancellation();
    let worker = Arc::clone(session);
    let handle = app.clone();
    let run_id = session.run_id;
    let epoch = input.epoch;
    let pinned_problem = problem;

    std::thread::Builder::new()
        .name(format!("evara-repair-{run_id}"))
        .spawn(move || {
            let emitter = handle.clone();
            let sink = CoalescingSink::new(
                Arc::clone(&life) as Arc<dyn ProgressReporter>,
                move |progress: Progress| {
                    let _ = emitter.emit(
                        PROGRESS_EVENT,
                        RepairProgressEvent {
                            run_id,
                            epoch,
                            placed: progress.placed,
                            total: progress.total,
                            steps: progress.steps,
                        },
                    );
                },
            );

            let outcome = repair_with(
                &RepairRequest {
                    problem: &pinned_problem,
                    identity: input.identity,
                    constraints: &worker.constraints,
                    preflight: input.preflight,
                    may_solve: input.may_solve,
                    unenforced_restrictions: input.unenforced,
                    config: input.config,
                    baseline: &worker.baseline,
                    movable: &worker.movable,
                },
                &sink,
                &cancel,
            );

            worker.finish(outcome);
            let has_result = worker.phase() == RepairPhase::Finished;
            let _ = handle.emit(
                FINISHED_EVENT,
                RepairFinishedEvent {
                    run_id,
                    epoch,
                    has_result,
                },
            );
        })
        .map_err(|error| SetupError::Failed {
            message: format!("Evara could not start the repair thread: {error}"),
        })?;

    Ok(())
}

/// How a repair is doing.
///
/// # Errors
///
/// [`SetupError::NotFound`] if this process no longer holds the session.
#[tauri::command]
pub fn repair_status(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<RepairStatus> {
    let session = require_session(&state, run_id)?;
    let progress = session.progress();
    Ok(RepairStatus {
        run_id,
        phase: session.phase(),
        placed: progress.placed,
        total: if progress.total == 0 {
            session.movable.len()
        } else {
            progress.total
        },
        steps: progress.steps,
        cancel_requested: session.cancel_requested(),
    })
}

/// The finished repair, with its diff.
///
/// # Errors
///
/// [`SetupError::NotFound`] if the session is gone, or [`SetupError::Invalid`] if it has not
/// finished.
#[tauri::command]
pub fn repair_result(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<RepairResult> {
    let session = require_session(&state, run_id)?;
    let Some(outcome) = session.outcome() else {
        return Err(SetupError::Invalid {
            message: "this repair has not finished".to_owned(),
        });
    };

    let rules = &session.constraints;
    let before = evaluate(&session.assembly.problem, &session.baseline, rules);
    let after = evaluate(
        &session.assembly.problem,
        &outcome.outcome.assignment,
        rules,
    );

    let causes: BTreeMap<Uuid, evara_core::scheduling::UnplacedCause> = outcome
        .outcome
        .unscheduled
        .iter()
        .map(|record| (record.id, record.cause))
        .collect();
    let locked: BTreeMap<Uuid, bool> = session
        .flags
        .iter()
        .filter(|(_, flags)| flags.is_locked)
        .map(|(id, _)| (*id, true))
        .collect();

    let diff = diff(&RepairDiffInput {
        problem: &session.assembly.problem,
        baseline: &session.baseline,
        repaired: &outcome.outcome.assignment,
        movable: &session.movable,
        labels: &session.labels,
        before: &before,
        after: &after,
        causes: &causes,
        locked: &locked,
    });

    Ok(RepairResult {
        run_id,
        phase: session.phase(),
        scope: session.scope.clone(),
        not_applyable: session.applyable(),
        diff,
        termination: outcome.outcome.termination.key().to_owned(),
        status: status_key(outcome.outcome.status),
        stopped_at_a_limit: stopped_at_a_limit(outcome.outcome.termination),
        defect: outcome.defect.as_ref().map(|d| d.key().to_owned()),
        applied_as: session.applied_as(),
    })
}

/// Asks a repair to stop.
///
/// Returns whether something was still running. Changes no state: the worker is the only thing
/// that leaves `Running`.
///
/// # Errors
///
/// [`SetupError::NotFound`] if the session is gone.
#[tauri::command]
pub fn repair_cancel(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<bool> {
    Ok(require_session(&state, run_id)?.request_cancel())
}

/// Forgets a repair session.
///
/// # Errors
///
/// Never fails; returns `false` if the session was already gone.
#[tauri::command]
pub fn repair_discard(state: State<'_, AppState>, run_id: Uuid) -> SetupResult<bool> {
    Ok(state.repairs.remove(run_id))
}

/// Applies a repair, as a **new** timetable. The one command that writes.
///
/// # Errors
///
/// [`SetupError::NotFound`] if the session is gone, [`SetupError::Invalid`] if it cannot be
/// applied or belongs to another project, or anything `save_repaired` refuses — a moved input,
/// an edited baseline, a stale source revision, a duplicate name, or a scope violation.
#[tauri::command]
pub fn repair_apply(
    state: State<'_, AppState>,
    run_id: Uuid,
    name: String,
) -> SetupResult<RepairApplied> {
    let session = require_session(&state, run_id)?;

    // The epoch, before a transaction opens. A repair begun against one campus file cannot be
    // applied into another.
    if session.epoch != state.epoch.get() {
        return Err(SetupError::Invalid {
            message: "this repair was begun against a campus file that is no longer open"
                .to_owned(),
        });
    }

    // Claimed by a compare-and-swap BEFORE any database work, so two clicks cannot both write.
    let outcome = session.claim_apply().map_err(refusal)?;

    let provenance = RunProvenance {
        engine: "construction".to_owned(),
        settings: String::from("{}"),
        started_at: session.started_at.clone(),
        finished_at: now(),
        kind: RUN_REPAIR.to_owned(),
        scope: serde_json::to_string(&session.scope).unwrap_or_else(|_| String::from("{}")),
    };

    let written = {
        let mut ws = workspace(&state)?;
        ws.timetables()?.save_repaired(
            &session.assembly,
            &RepairSave {
                source: session.source,
                expected: session.source_rev,
                guard: EditGuard {
                    watermark: session.assembly.watermark,
                    stamp: &session.stamp,
                },
                outcome: &outcome.outcome,
                violations: &outcome.violations,
                baseline: &session.baseline,
                movable: &session.movable,
                flags: &session.flags,
                run: &provenance,
                name: &name,
            },
        )
    };

    match written {
        Ok(saved) => {
            session.apply_succeeded(saved.timetable.id);
            Ok(RepairApplied {
                timetable_id: saved.timetable.id,
                based_on: session.source,
                entries: saved.entries,
            })
        }
        Err(error) => {
            // Back to `Finished`, so a transient refusal can be retried after the user fixes
            // what caused it.
            session.apply_failed();
            Err(error.into())
        }
    }
}

// =======================================================================================
// Helpers
// =======================================================================================

type RepairParts = (
    Assembly,
    LabelSet,
    evara_core::scheduling::Assignment,
    evara_db::StoredPins,
    Vec<evara_core::scheduling::CompiledConstraint>,
);

type StartParts = (
    Assembly,
    LabelSet,
    evara_core::scheduling::Assignment,
    evara_db::StoredPins,
    Vec<evara_core::scheduling::CompiledConstraint>,
    evara_db::ArrangementStamp,
    Vec<(Uuid, evara_db::EntryFlags)>,
    i64,
);

/// The assembly, labels and baseline, for a read-only command.
fn workspace_parts(
    state: &State<'_, AppState>,
    selection: &SchedulingSelection,
    timetable_id: Uuid,
) -> SetupResult<(Assembly, LabelSet, evara_core::scheduling::Assignment)> {
    let mut ws = workspace(state)?;
    let assembly = ws.scheduling()?.load(selection)?;
    let labels = ws.scheduling()?.labels(selection)?;
    let base = ws.timetables()?.repair_baseline(timetable_id, &assembly)?;
    Ok((assembly, labels, base.arrangement.assignment))
}

/// Everything the pre-flight needs, in one trip under the lock.
fn load_for_repair(
    state: &State<'_, AppState>,
    selection: &SchedulingSelection,
    timetable_id: Uuid,
) -> SetupResult<RepairParts> {
    let mut ws = workspace(state)?;
    // The Assembly FIRST, so its watermark precedes the baseline snapshot.
    let assembly = ws.scheduling()?.load(selection)?;
    let labels = ws.scheduling()?.labels(selection)?;
    let stored = ws.constraints()?.stored_for(selection.school_id)?;
    let rules =
        Registry::compile(&stored, selection.term_id).map_err(|error| SetupError::Invalid {
            message: format!("this school's constraint configuration cannot be read: {error}"),
        })?;
    let base = ws.timetables()?.repair_baseline(timetable_id, &assembly)?;
    Ok((
        assembly,
        labels,
        base.arrangement.assignment,
        base.pins,
        rules,
    ))
}

/// Everything `repair_start` needs, including the guards.
fn load_for_start(
    state: &State<'_, AppState>,
    selection: &SchedulingSelection,
    timetable_id: Uuid,
) -> SetupResult<StartParts> {
    let mut ws = workspace(state)?;
    let assembly = ws.scheduling()?.load(selection)?;
    let labels = ws.scheduling()?.labels(selection)?;
    let stored = ws.constraints()?.stored_for(selection.school_id)?;
    let rules =
        Registry::compile(&stored, selection.term_id).map_err(|error| SetupError::Invalid {
            message: format!("this school's constraint configuration cannot be read: {error}"),
        })?;
    // ONE read for the baseline, the pins, the stamp, the flags and the source revision.
    let base = ws.timetables()?.repair_baseline(timetable_id, &assembly)?;
    Ok((
        assembly,
        labels,
        base.arrangement.assignment,
        base.pins,
        rules,
        base.stamp,
        base.flags,
        base.source_rev,
    ))
}

/// Adds one scope option, skipping any that selects nothing.
fn push_option(
    options: &mut Vec<ScopeOption>,
    problem: &evara_core::scheduling::Problem,
    baseline: &evara_core::scheduling::Assignment,
    scope: RepairScope,
    label: &str,
) {
    let Ok(movable) = derive_movable(problem, baseline, &scope) else {
        // A scope that reaches nothing is not offered: choosing it would be refused, and a
        // menu entry that cannot be used is worse than an absent one.
        return;
    };
    let unreachable = if scope.is_positional() {
        problem.activities.len() - baseline.scheduled_count()
    } else {
        0
    };
    options.push(ScopeOption {
        key: scope.key().to_owned(),
        label: label.to_owned(),
        movable: movable.len(),
        frozen: movable.frozen_count(),
        positional: scope.is_positional(),
        unreachable_unplaced: unreachable,
        scope,
    });
}

/// The message and remedy for one unexpressible lock.
fn describe_unexpressible(cause: UnexpressibleLockCause) -> (String, String) {
    match cause {
        UnexpressibleLockCause::ActivityNotInProblem => (
            "This locked lesson is not part of the campus, cycle and term being repaired."
                .to_owned(),
            "Repair the timetable for the cycle and term the lesson belongs to, or remove the \
             lock on it."
                .to_owned(),
        ),
        UnexpressibleLockCause::TimeslotNotInProblem { .. } => (
            "This lock names a period that is no longer in the bell schedule.".to_owned(),
            "Unschedule or re-place that lesson in a period that exists, then lock it again."
                .to_owned(),
        ),
        UnexpressibleLockCause::RoomNotInProblem { .. } => (
            "This lock names a room that is no longer available to this campus.".to_owned(),
            "Re-place the lesson in an available room and lock it again, or share the room with \
             this campus again."
                .to_owned(),
        ),
    }
}

/// A scope error as a pre-flight finding.
fn scope_finding(error: &ScopeError) -> PreflightFinding {
    PreflightFinding {
        code: "scope-unusable".to_owned(),
        blocking: true,
        message: scope_message(error),
        remedy: "Choose a different scope.".to_owned(),
        activity_id: None,
    }
}

/// A scope error, as a sentence.
fn scope_message(error: &ScopeError) -> String {
    match error {
        ScopeError::NotInProblem { entity, .. } => {
            format!("that {entity} is not part of the timetable being repaired")
        }
        ScopeError::Empty { scope } => {
            format!("nothing in the {scope} scope can move, so there is no repair to run")
        }
        ScopeError::Movable(inner) => format!("that scope cannot be used: {inner:?}"),
    }
}

/// The blocking findings, as one sentence.
fn blocking_message(checked: &RepairPreflight) -> String {
    let blocking: Vec<&str> = checked
        .findings
        .iter()
        .filter(|f| f.blocking)
        .map(|f| f.message.as_str())
        .collect();
    format!(
        "this timetable cannot be repaired yet: {}",
        blocking.join(" ")
    )
}

/// An apply refusal as a command error.
fn refusal(refusal: ApplyRefusal) -> SetupError {
    match refusal {
        ApplyRefusal::StillRunning => SetupError::Invalid {
            message: "this repair has not finished".to_owned(),
        },
        ApplyRefusal::AlreadyApplying => SetupError::Invalid {
            message: "this repair is already being applied".to_owned(),
        },
        ApplyRefusal::AlreadyApplied { timetable_id } => SetupError::Invalid {
            message: format!("this repair was already applied as timetable {timetable_id}"),
        },
        ApplyRefusal::Discarded => SetupError::Invalid {
            message: "this repair was discarded".to_owned(),
        },
        ApplyRefusal::NotApplyable { reason } => SetupError::Invalid { message: reason },
    }
}

/// The session, or a not-found error.
fn require_session(state: &State<'_, AppState>, run_id: Uuid) -> SetupResult<Arc<RepairSession>> {
    state.repairs.get(run_id).ok_or_else(|| SetupError::NotFound {
        entity: "repairSession".to_owned(),
        message: "this repair is no longer open — it was discarded, or the campus file was                   closed and reopened"
            .to_owned(),
    })
}

/// Whether a limit stopped the run, rather than the search finishing.
const fn stopped_at_a_limit(termination: evara_core::scheduling::Termination) -> bool {
    matches!(
        termination,
        evara_core::scheduling::Termination::CandidateLimitReached
            | evara_core::scheduling::Termination::StepLimitReached
            | evara_core::scheduling::Termination::PassLimitReached
    )
}

/// The stored key for a status.
fn status_key(status: evara_core::scheduling::SolveStatus) -> String {
    use evara_core::scheduling::SolveStatus;
    match status {
        SolveStatus::Complete => "COMPLETE",
        SolveStatus::Partial => "PARTIAL",
        SolveStatus::Infeasible => "INFEASIBLE",
        SolveStatus::Cancelled => "CANCELLED",
        SolveStatus::Failed => "FAILED",
    }
    .to_owned()
}

/// Stored availability restrictions this engine does not enforce.
///
/// The same reading [`super::solve`] takes, so a repaired timetable discloses what a generated
/// one does.
fn unenforced_count(assembly: &Assembly) -> u32 {
    use evara_db::scheduling::Completeness;
    match assembly.completeness {
        Completeness::Complete => 0,
        Completeness::RestrictionsNotEnforced { rules } => u32::try_from(rules).unwrap_or(u32::MAX),
    }
}

/// The current instant, ISO-8601 UTC.
fn now() -> String {
    evara_db::now_iso8601()
}

#[cfg(test)]
mod tests;
