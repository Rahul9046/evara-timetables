//! Saved timetables: listing, opening, renaming, deleting and re-scoring one.
//!
//! The `report` row `super`'s table has reserved since Phase 1. **Four writes in the whole
//! module** — rename, delete, and the revalidation that replaces a current scoring — and
//! every read goes through `Scheduling`, whose receiver is a shared reference so the borrow
//! checker forbids it opening a transaction at all.
//!
//! # Opening a timetable compiles a problem
//!
//! Always, and that is a decision rather than an implementation detail.
//! `span::resolve_activity_span` takes a `&Problem`, and
//! [ADR 0042](../../../../docs/adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
//! D2 moved the resolver down precisely so there would be **one** implementation of *"which
//! slots does this occurrence occupy?"*. A viewer that derived spans from `timetable_entry`
//! rows alone would be the second copy, which `CLAUDE.md` §2 forbids in the strongest terms
//! the file contains.
//!
//! The load is cheap — M4's loader is a fixed number of statements per load, independent of
//! school size — and it is read-only.
//!
//! # When the problem cannot be compiled
//!
//! [`TimetableView::projection`] is `None` and [`TimetableView::unreadable`] says why. The
//! stored findings are still there, because
//! [TIMETABLE-PERSISTENCE.md](../../../../docs/TIMETABLE-PERSISTENCE.md) §7 keeps them for
//! exactly this: *"they let a view render findings without loading a `Problem` at all."*
//!
//! What the screen must **not** do in that state is draw a partial grid. It says the grid
//! could not be built and shows the reason — a timetable whose lessons were deleted is still
//! worth opening, and a reader that refused would leave the user unable to see what to fix.
//!
//! # Two scorings, and neither speaks for the other
//!
//! A `GENERATION` evaluation is immutable for the life of the timetable; a `REVALIDATION` is
//! replaced wholesale. Both are returned, each with its timestamp, because a screen showing
//! one number would have to choose which, and either choice is a lie about the other.

// See `project.rs` for why this lint is inapplicable to a module of command handlers.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{Problem, SolveOutcome, UnverifiedCounts};
use evara_db::scheduling::SchedulingSelection;
use evara_db::{
    DbError, Scoring, StoredViolation, Timetable, TimetableEvaluation, UnscheduledRecord,
};
use evara_report::findings::{FindingCounts, FindingEntity, FindingView, StoredFinding};
use evara_report::projection::{
    LessonFlags, ProjectionInput, ScopeGrid, ScopeKind, TimetableProjection,
};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use super::error::{SetupError, SetupResult};
use super::project::AppState;
use super::workspace;

/// One stored timetable, as a list row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TimetableSummary {
    /// Stable identifier.
    pub id: Uuid,
    /// The cycle its grid is over.
    pub cycle_id: Uuid,
    /// The term, or `null` for a whole-year grid.
    pub term_id: Option<Uuid>,
    /// What a human calls it. The only thing telling two versions apart.
    pub name: String,
    /// When it was generated, ISO-8601 UTC.
    pub generated_at: String,
    /// The revision a rename or a delete must carry.
    pub rev: i64,
    /// What this timetable could not check. Never omitted.
    pub unverified: UnverifiedCounts,
}

impl TimetableSummary {
    /// Flattens one stored row.
    #[must_use]
    pub fn of(timetable: &Timetable) -> Self {
        Self {
            id: timetable.id,
            cycle_id: timetable.cycle_id,
            term_id: timetable.term_id,
            name: timetable.name.clone(),
            generated_at: timetable.generated_at.clone(),
            rev: timetable.rev,
            unverified: timetable.unverified(),
        }
    }
}

/// One scoring of one timetable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScoringView {
    /// `GENERATION` or `REVALIDATION`.
    pub kind: String,
    /// Whether this is the immutable record of how the timetable was generated.
    pub is_generation: bool,
    /// When it was taken, ISO-8601 UTC. Shown beside every figure it produced.
    pub evaluated_at: String,
    /// Hard penalties. Negative-going.
    pub hard: i64,
    /// Medium penalties.
    pub medium: i64,
    /// Soft penalties.
    pub soft: i64,
    /// How many occurrences were placed.
    pub placed: usize,
    /// How many the problem held.
    pub required: usize,
}

impl ScoringView {
    /// Flattens one stored evaluation.
    fn of(evaluation: &TimetableEvaluation) -> Self {
        let score = evaluation.score();
        Self {
            kind: evaluation.kind.clone(),
            is_generation: evaluation.is_generation(),
            evaluated_at: evaluation.evaluated_at.clone(),
            hard: score.hard,
            medium: score.medium,
            soft: score.soft,
            placed: usize::try_from(evaluation.placed_count).unwrap_or(0),
            required: usize::try_from(evaluation.required_count).unwrap_or(0),
        }
    }
}

/// One occurrence a run could not place.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UnscheduledView {
    /// Which occurrence.
    pub activity_id: Uuid,
    /// What the lesson is called, where the labels hold it.
    pub label: Option<String>,
    /// The stored cause key.
    pub cause: String,
    /// Whether this cause proves **no** timetable can place it.
    ///
    /// The one place the difference between *"no timetable can place this"* and *"this
    /// arrangement did not"* is decided. An unrecognised key — written by a newer build —
    /// is deliberately **not** reported as impossible, because overclaiming is the failure
    /// the distinction exists to prevent.
    pub is_impossible: bool,
    /// The pre-solve finding that already proved it, where there was one.
    pub diagnostic_code: Option<String>,
    /// What the engine said, for a result still in memory.
    pub reason: Option<String>,
}

/// An entity this timetable shares with another school.
///
/// ADR 0032 rule 3 requires the warning to name the entity **and both schools** and to state
/// the consequence — *"a warning a user learns to dismiss is worse than none"*. The names
/// exist only in a compiled `Problem`, so this list is empty in the degraded view and the
/// screen says so rather than implying it knows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SharedEntity {
    /// `teacher` or `room`.
    pub kind: String,
    /// Which one.
    pub id: Uuid,
    /// What it is called.
    pub name: String,
    /// The other schools that also hold it.
    pub also_at: Vec<String>,
}

/// What a solve concluded, for a preview or a provenance panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutcomeView {
    /// `COMPLETE`, `PARTIAL`, `INFEASIBLE`, `CANCELLED` or `FAILED`.
    pub status: String,
    /// Why the engine stopped looking.
    pub termination: String,
    /// The seed, so the run can be reproduced — unless it was cancelled.
    pub seed: u64,
    /// Hard penalties. Negative-going.
    pub hard: i64,
    /// Medium penalties.
    pub medium: i64,
    /// Soft penalties.
    pub soft: i64,
    /// How many occurrences were placed.
    pub placed: usize,
    /// How many the problem held.
    pub required: usize,
    /// How many hard violations the result breaks, summed across rules.
    ///
    /// Non-zero means the timetable **cannot be run**. Kept apart from the `hard` score: a
    /// penalty is a magnitude and a count is how many things are wrong, and a screen needs
    /// the second to write a sentence.
    pub hard_violations: usize,
    /// What could not be placed.
    pub unscheduled: Vec<UnscheduledView>,
    /// What was not checked. Never omitted.
    pub unverified: UnverifiedCounts,
    /// Whether a cancelled run is why this stopped.
    ///
    /// Carried separately because `Termination::Cancelled` is the one termination this
    /// workspace documents as **not reproducible**, so the screen offers no seed for it.
    pub was_cancelled: bool,
}

impl OutcomeView {
    /// Flattens one in-memory outcome.
    #[must_use]
    pub fn of(outcome: &SolveOutcome, problem: &Problem) -> Self {
        use evara_core::scheduling::Termination;
        Self {
            status: format!("{:?}", outcome.status).to_uppercase(),
            termination: format!("{:?}", outcome.termination),
            seed: outcome.seed.get(),
            hard: outcome.score.hard,
            medium: outcome.score.medium,
            soft: outcome.score.soft,
            placed: outcome.assignment.scheduled_count(),
            required: problem.activity_count(),
            hard_violations: outcome
                .hard_violations
                .iter()
                .map(|count| usize::try_from(count.violations).unwrap_or(0))
                .sum(),
            unscheduled: outcome
                .unscheduled
                .iter()
                .map(|un| UnscheduledView {
                    activity_id: un.id,
                    label: Some(un.section.clone()),
                    cause: un.cause.key().to_owned(),
                    is_impossible: un.cause.is_impossible(),
                    diagnostic_code: un.diagnostic.as_ref().map(|d| d.code.key().to_owned()),
                    reason: Some(un.reason.clone()),
                })
                .collect(),
            unverified: outcome.unverified,
            was_cancelled: outcome.termination == Termination::Cancelled,
        }
    }
}

/// Everything a view needs about one stored timetable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TimetableView {
    /// The timetable row.
    pub timetable: TimetableSummary,
    /// Its scorings: the immutable generation one, and the current one.
    pub scorings: Vec<ScoringView>,
    /// What its run could not place.
    pub unscheduled: Vec<UnscheduledView>,
    /// Entities another school also holds, **named** — compiled view only.
    pub shared: Vec<SharedEntity>,
    /// The grid, or `null` when the problem could not be compiled.
    pub projection: Option<TimetableProjection>,
    /// Why there is no grid, when there is none.
    pub unreadable: Option<String>,
    /// Stored entries naming something the problem no longer holds.
    ///
    /// Non-empty means the timetable is **readable but incomplete**: those placements are
    /// not in the grid, and a screen that did not say so would show a short timetable as if
    /// it were whole.
    pub not_in_problem: Vec<Uuid>,
    /// How many entries were stored, against how many reached the grid.
    pub stored_entries: usize,
    /// The findings, at whichever fidelity was available.
    pub findings: Vec<FindingView>,
    /// How many findings there are at each level.
    pub counts: FindingCounts,
    /// How the engine ran, where a run is recorded.
    pub run: Option<RunView>,
}

/// How one timetable came to be.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunView {
    /// Which engine ran.
    pub engine: String,
    /// The seed every ordering decision derived from.
    pub seed: i64,
    /// What came back.
    pub status: String,
    /// Why it stopped.
    pub termination: String,
    /// When the caller started it.
    pub started_at: String,
    /// When the caller saw it finish.
    pub finished_at: String,
    /// Which build ran it.
    pub app_version: String,
}

// =======================================================================================
// Listing
// =======================================================================================

/// Every timetable of one school, newest first.
#[tauri::command]
pub fn timetable_list(
    state: State<'_, AppState>,
    school_id: Uuid,
) -> SetupResult<Vec<TimetableSummary>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .timetables()?
        .for_school(school_id)?
        .iter()
        .map(TimetableSummary::of)
        .collect())
}

/// Every timetable over one `(cycle, term)`, newest first.
#[tauri::command]
pub fn timetable_list_for_cycle(
    state: State<'_, AppState>,
    cycle_id: Uuid,
    term_id: Option<Uuid>,
) -> SetupResult<Vec<TimetableSummary>> {
    let mut ws = workspace(&state)?;
    Ok(ws
        .timetables()?
        .for_cycle(cycle_id, term_id)?
        .iter()
        .map(TimetableSummary::of)
        .collect())
}

// =======================================================================================
// Opening
// =======================================================================================

/// One stored timetable, with its grid where one can be built.
///
/// # Errors
///
/// [`SetupError::NotFound`] if the project holds no such timetable. A timetable whose
/// *problem* cannot be compiled is **not** an error — see this module's header.
#[tauri::command]
#[allow(
    clippy::too_many_lines,
    reason = "one read of one timetable, in the order the joins require; splitting it \
              would hide which facts depend on the problem compiling and which do not"
)]
pub fn timetable_open(state: State<'_, AppState>, id: Uuid) -> SetupResult<TimetableView> {
    let mut ws = workspace(&state)?;

    let timetable = ws.timetables()?.get(id)?;
    let evaluations = ws.timetables()?.evaluations(id)?;
    let run = ws.timetables()?.run_for(id)?;
    let stored_unscheduled = match &run {
        Some(run) => ws.timetables()?.unscheduled_for(run.id)?,
        None => Vec::new(),
    };
    let stored_violations = current_violations(&mut ws, &evaluations)?;
    let stored_entries = ws.timetables()?.entries(id)?.len();

    // The school is the cycle's: a timetable carries no `school_id`, and a child's school is
    // resolved through its parent chain (ADR 0042 D6).
    let school_id = ws
        .time_model()?
        .cycle(timetable.cycle_id)?
        .map(|cycle| cycle.school_id);

    let selection = school_id.map(|school_id| SchedulingSelection {
        school_id,
        cycle_id: timetable.cycle_id,
        term_id: timetable.term_id,
    });

    // Everything above is available whatever happens next. Everything below needs a problem.
    let compiled = match selection {
        None => Err("this timetable names a cycle the campus file no longer holds".to_owned()),
        Some(selection) => compile(&mut ws, id, &selection),
    };

    let (projection, unreadable, not_in_problem, shared, findings) = match compiled {
        Ok(view) => (
            Some(view.projection),
            None,
            view.not_in_problem,
            view.shared,
            view.findings,
        ),
        Err(why) => (
            None,
            Some(why),
            Vec::new(),
            Vec::new(),
            // The stored tier: every finding, with no sentence and no entity names.
            // TIMETABLE-PERSISTENCE §7 is why these rows exist.
            stored_violations.iter().map(stored_finding).collect(),
        ),
    };

    let labels_for_unscheduled: BTreeMap<Uuid, String> = projection
        .as_ref()
        .map(|projection| {
            projection
                .lessons
                .iter()
                .map(|lesson| {
                    (
                        lesson.activity_id,
                        lesson
                            .label
                            .as_ref()
                            .map_or_else(|| lesson.section_id.to_string(), |l| l.short.clone()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(TimetableView {
        timetable: TimetableSummary::of(&timetable),
        scorings: evaluations.iter().map(ScoringView::of).collect(),
        unscheduled: stored_unscheduled
            .iter()
            .map(|record| stored_unscheduled_view(record, &labels_for_unscheduled))
            .collect(),
        shared,
        projection,
        unreadable,
        not_in_problem,
        stored_entries,
        counts: FindingCounts::of(&findings),
        findings,
        run: run.map(|run| RunView {
            engine: run.engine,
            seed: run.seed,
            status: run.status,
            termination: run.termination,
            started_at: run.started_at,
            finished_at: run.finished_at,
            app_version: run.app_version,
        }),
    })
}

/// What compiling a problem for one stored timetable produced.
struct Compiled {
    projection: TimetableProjection,
    /// The labels the projection was built from.
    ///
    /// Returned rather than read again by a caller that needs them: `lay_out` takes the same
    /// set the projection was joined against, and reading a second one would be a second
    /// chance for the two to describe different scopes.
    labels: evara_core::scheduling::display::LabelSet,
    not_in_problem: Vec<Uuid>,
    shared: Vec<SharedEntity>,
    findings: Vec<FindingView>,
}

/// Compiles the problem, reads the arrangement back, projects it and explains its findings.
///
/// Returns the reason as a sentence rather than an error, because a timetable whose problem
/// cannot be compiled is still worth opening.
fn compile(
    ws: &mut std::sync::MutexGuard<'_, evara_db::Workspace>,
    id: Uuid,
    selection: &SchedulingSelection,
) -> Result<Compiled, String> {
    let assembly = ws
        .scheduling()
        .and_then(|s| s.load(selection))
        .map_err(describe_load)?;
    let labels = ws
        .scheduling()
        .and_then(|s| s.labels(selection))
        .map_err(describe_load)?;
    let arrangement = ws
        .timetables()
        .and_then(|t| t.arrangement(id, &assembly))
        .map_err(describe_load)?;
    let flags: BTreeMap<Uuid, LessonFlags> = ws
        .scheduling()
        .and_then(|s| s.entry_flags(id))
        .map_err(describe_load)?
        .into_iter()
        .map(|(activity, is_locked, is_manual)| {
            (
                activity,
                LessonFlags {
                    is_locked,
                    is_manual,
                },
            )
        })
        .collect();

    let projection = evara_report::projection::project(ProjectionInput {
        problem: &assembly.problem,
        assignment: &arrangement.assignment,
        labels: &labels,
        flags: &flags,
    });

    // The current findings, explained by the rules that produced them. Not the stored ones:
    // a `REVALIDATION` answers *"what is wrong with this timetable now?"*, and with a problem
    // in hand that question can be answered rather than recalled.
    let stored_rules = ws
        .constraints()
        .and_then(|c| c.stored_for(selection.school_id))
        .map_err(describe_load)?;
    let rules = Registry::compile(&stored_rules, selection.term_id).map_err(|error| {
        format!("this school's constraint configuration cannot be read: {error}")
    })?;
    let report = evaluate(&assembly.problem, &arrangement.assignment, &rules);
    let findings = evara_report::explain_all(&assembly.problem, &report.violations);

    Ok(Compiled {
        projection,
        labels,
        not_in_problem: arrangement.not_in_problem,
        shared: shared_entities(&assembly.problem),
        findings,
    })
}

/// A load failure as a sentence a screen can show beside an absent grid.
fn describe_load(error: DbError) -> String {
    match error {
        DbError::NotFound { entity } => {
            format!("the {entity} this timetable was built over is no longer in the campus file")
        }
        DbError::WrongSchool { .. } => {
            "this timetable's cycle now belongs to a different school".to_owned()
        }
        DbError::CorruptIdentifier { .. } => {
            "the campus file holds a damaged value that this timetable depends on".to_owned()
        }
        other => format!("the grid could not be rebuilt: {other}"),
    }
}

/// Every entity in the problem that another school also holds, **named**.
fn shared_entities(problem: &Problem) -> Vec<SharedEntity> {
    let mut out = Vec::new();
    for teacher in &problem.teachers {
        if teacher.also_at.is_empty() {
            continue;
        }
        out.push(SharedEntity {
            kind: "teacher".to_owned(),
            id: teacher.id,
            name: teacher.name.clone(),
            also_at: teacher.also_at.iter().map(|s| s.name.clone()).collect(),
        });
    }
    for room in &problem.rooms {
        if room.also_at.is_empty() {
            continue;
        }
        out.push(SharedEntity {
            kind: "room".to_owned(),
            id: room.id,
            name: room.name.clone(),
            also_at: room.also_at.iter().map(|s| s.name.clone()).collect(),
        });
    }
    out
}

/// The violations of the current scoring, preferring a revalidation over the generation one.
fn current_violations(
    ws: &mut std::sync::MutexGuard<'_, evara_db::Workspace>,
    evaluations: &[TimetableEvaluation],
) -> SetupResult<Vec<StoredViolation>> {
    let chosen = evaluations
        .iter()
        .find(|evaluation| !evaluation.is_generation())
        .or_else(|| evaluations.first());
    match chosen {
        Some(evaluation) => Ok(ws.timetables()?.violations_for(evaluation.id)?),
        None => Ok(Vec::new()),
    }
}

/// One stored violation as a finding, at the stored fidelity.
fn stored_finding(violation: &StoredViolation) -> FindingView {
    FindingView::from_stored(StoredFinding {
        type_key: &violation.constraint_type,
        level: &violation.level,
        penalty: violation.penalty,
        detail: &violation.detail,
        cycle_day_id: violation.cycle_day_id,
        starts_at: violation.starts_at.clone(),
        ends_at: violation.ends_at.clone(),
        entities: violation
            .entities
            .iter()
            .map(|entity| FindingEntity {
                role: entity.role.clone(),
                kind: entity.entity_kind.clone(),
                id: entity.entity_id.clone(),
            })
            .collect(),
    })
}

/// One stored unplaced record, with a label where the projection supplied one.
fn stored_unscheduled_view(
    record: &UnscheduledRecord,
    labels: &BTreeMap<Uuid, String>,
) -> UnscheduledView {
    UnscheduledView {
        activity_id: record.activity_id,
        label: labels.get(&record.activity_id).cloned(),
        cause: record.cause.clone(),
        is_impossible: record.is_impossible(),
        diagnostic_code: record.diagnostic_code.clone(),
        // Deliberately absent. The engine's sentence is not stored, because *"a wording
        // improvement would leave old rows reading the old way"* — TIMETABLE-PERSISTENCE §7.
        reason: None,
    }
}

/// Lays out one scope of one projection.
///
/// Shared by the saved view and the generated preview, so a grid cannot look different
/// depending on which screen asked for it.
///
/// # Errors
///
/// [`SetupError::NotFound`] if the projection holds no such scope — a teacher who has been
/// deleted since the screen listed them, say. Refused rather than answered with an empty
/// grid: an empty grid means *"this teacher has no lessons"*, which is a different statement.
pub(super) fn lay_out_scope(
    projection: &TimetableProjection,
    labels: &evara_core::scheduling::display::LabelSet,
    kind: ScopeKind,
    id: Option<Uuid>,
) -> SetupResult<ScopeGrid> {
    let scope = projection
        .scopes
        .iter()
        .find(|scope| scope.kind == kind && scope.id == id)
        .ok_or_else(|| SetupError::NotFound {
            entity: "timetable".to_owned(),
            message: "That view is no longer available for this timetable. It may name \
                      something that has since been removed."
                .to_owned(),
        })?;
    Ok(evara_report::projection::lay_out(projection, scope, labels))
}

/// One scope's laid-out grid, for a stored timetable.
///
/// Re-loads and re-projects rather than caching: a load is a fixed number of statements and a
/// projection of the realistic upper end is single-digit milliseconds, so a cache would buy
/// little and would introduce a lifetime nothing else in this crate has — invalidated by
/// every rename, delete, revalidation and project close.
///
/// See `solve::timetable_generate_scope` for why the layout is a second call at all.
#[tauri::command]
pub fn timetable_scope(
    state: State<'_, AppState>,
    id: Uuid,
    scope_kind: ScopeKind,
    scope_id: Option<Uuid>,
) -> SetupResult<ScopeGrid> {
    let mut ws = workspace(&state)?;
    let timetable = ws.timetables()?.get(id)?;
    let school_id = ws
        .time_model()?
        .cycle(timetable.cycle_id)?
        .map(|cycle| cycle.school_id)
        .ok_or_else(|| SetupError::NotFound {
            entity: "cycle".to_owned(),
            message: "This timetable names a cycle the campus file no longer holds, so no \
                      grid can be built for it."
                .to_owned(),
        })?;
    let selection = SchedulingSelection {
        school_id,
        cycle_id: timetable.cycle_id,
        term_id: timetable.term_id,
    };
    let compiled = compile(&mut ws, id, &selection).map_err(|why| SetupError::Invalid {
        message: capitalise(&why),
    })?;
    lay_out_scope(&compiled.projection, &compiled.labels, scope_kind, scope_id)
}

/// A sentence, with its first letter raised and a full stop added.
fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str() + ".",
        None => String::new(),
    }
}

// =======================================================================================
// Writes
// =======================================================================================

/// Renames one timetable.
///
/// Takes the caller's revision, which `repo::update` puts in the statement's `WHERE` clause.
/// A save built on a version somebody else has since changed is refused rather than applied.
#[tauri::command]
pub fn timetable_rename(
    state: State<'_, AppState>,
    id: Uuid,
    expected_rev: i64,
    name: String,
) -> SetupResult<TimetableSummary> {
    let mut ws = workspace(&state)?;
    let renamed = ws.timetables()?.rename(id, expected_rev, name.trim())?;
    Ok(TimetableSummary::of(&renamed))
}

/// What deleting one timetable removed.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeletedView {
    /// Placed occurrences.
    pub entries: usize,
    /// Runs.
    pub runs: usize,
    /// Unplaced-occurrence records.
    pub unscheduled: usize,
    /// Scorings.
    pub evaluations: usize,
    /// Violations.
    pub violations: usize,
}

/// Deletes one timetable and everything that belongs to it, in one transaction.
///
/// The counts come back so a confirmation can say what it removed. Every child edge is
/// `RESTRICT` and the repository deletes the rows itself, so each one appears in the change
/// feed — which a `CASCADE` would have skipped (ADR 0040 §2).
#[tauri::command]
pub fn timetable_delete(
    state: State<'_, AppState>,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<DeletedView> {
    let mut ws = workspace(&state)?;
    let removed = ws.timetables()?.delete(id, expected_rev)?;
    Ok(DeletedView {
        entries: removed.entries,
        runs: removed.runs,
        unscheduled: removed.unscheduled,
        evaluations: removed.evaluations,
        violations: removed.violations,
    })
}

/// Re-scores one stored timetable and replaces its current findings.
///
/// # Why this takes `expectedRev`
///
/// `record_revalidation` **consumes** the timetable's revision: the current findings are part
/// of what a timetable says about itself, so recording new ones moves it. The caller's
/// revision therefore has to be the predicate on that write
/// ([ADR 0027](../../../../docs/adr/0027-revisions-are-checked-on-write.md)), and it is the
/// *interface's* revision rather than one this handler reads — reading it here would be
/// read-compare-write, which ADR 0027 calls *"the same race with a longer window"*.
///
/// The `GENERATION` scoring is untouched, so re-scoring can never misrepresent how a
/// timetable was generated.
///
/// # Errors
///
/// [`SetupError::Conflict`] if the timetable moved since `expectedRev`; a sentence rather
/// than a grid if the problem cannot be compiled, because there is nothing to score against.
#[tauri::command]
pub fn timetable_revalidate(
    state: State<'_, AppState>,
    id: Uuid,
    expected_rev: i64,
) -> SetupResult<ScoringView> {
    let mut ws = workspace(&state)?;

    let timetable = ws.timetables()?.get(id)?;
    let school_id = ws
        .time_model()?
        .cycle(timetable.cycle_id)?
        .map(|cycle| cycle.school_id)
        .ok_or_else(|| SetupError::NotFound {
            entity: "cycle".to_owned(),
            message: "This timetable names a cycle the campus file no longer holds, so it \
                      cannot be re-scored."
                .to_owned(),
        })?;
    let selection = SchedulingSelection {
        school_id,
        cycle_id: timetable.cycle_id,
        term_id: timetable.term_id,
    };

    let assembly = ws.scheduling()?.load(&selection)?;
    let arrangement = ws.timetables()?.arrangement(id, &assembly)?;
    let stored_rules = ws.constraints()?.stored_for(school_id)?;
    let rules = Registry::compile(&stored_rules, selection.term_id).map_err(|error| {
        SetupError::Invalid {
            message: format!("this school's constraint configuration cannot be read: {error}"),
        }
    })?;
    let report = evaluate(&assembly.problem, &arrangement.assignment, &rules);

    let recorded = ws.timetables()?.record_revalidation(
        id,
        expected_rev,
        &Scoring {
            score: report.score,
            placed: arrangement.assignment.scheduled_count(),
            required: assembly.problem.activity_count(),
            violations: &report.violations,
        },
        &assembly,
    )?;
    Ok(ScoringView::of(&recorded))
}
