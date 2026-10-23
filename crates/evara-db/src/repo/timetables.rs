//! Stored timetables: saving a solve, listing what exists, and scoring one again later.
//!
//! # A regeneration never overwrites
//!
//! Saving always **inserts**. Several timetables may exist for one `(cycle, term)`, and that
//! coexistence *is* the versioning — there is no version number, no status and no "current"
//! pointer. Replacement is not an operation: it is a save followed by an explicit delete,
//! which is two decisions by the user rather than one silent one.
//!
//! ADR 0010 makes deletion *"follow ownership, and default to refusing"*, and a regeneration
//! that quietly replaced the previous timetable would destroy the one thing a timetabler
//! compares against.
//!
//! # The freshness check is the whole point of [`Timetables::save_generated`]
//!
//! A `SolveOutcome` is produced from a snapshot of the inputs and saved some time later.
//! Between the two, anything may have moved — an availability rule, a room's bookability, a
//! lesson's duration — and **none of it breaks a foreign key**, so every stored reference would
//! still resolve and the save would succeed. The result is a plausible, wrong timetable, which
//! is the one failure a school cannot detect for itself.
//!
//! So the `Assembly`'s [`Watermark`] is compared against the change feed **inside the
//! transaction that writes**, and a move refuses [`DbError::InputsChanged`]. That is ADR 0030's
//! shape — *"rebuilds the plan inside the transaction and refuses if the fingerprint has
//! moved"* — and it is not check-then-act, because the comparison is a predicate on the write
//! rather than a question asked beforehand. ADR 0030's other rule applies too: **never compare
//! a watermark in the interface.**
//!
//! # Historical and current findings are different rows
//!
//! A `timetable_evaluation` of kind `GENERATION` is what the solver found, belongs to its
//! `solve_run`, and is **immutable for the life of the timetable**. A `REVALIDATION` is what
//! the rules find *now*, and is replaced wholesale — two scorings of a changing world are not
//! additive.
//!
//! Replacement touches only the `REVALIDATION` row, so re-scoring a timetable can never
//! misrepresent how it was generated. That is what the kind column is for; without it a
//! revalidation would have to delete the generation's findings to replace the current ones.
//!
//! # One transaction, and no scheduling rule in sight
//!
//! A save writes a `timetable`, N `timetable_entry`, a `solve_run`, N `solve_run_unscheduled`,
//! one `timetable_evaluation` and its violations — all or nothing. A failure on the last
//! violation must leave none of the rest, which is the reason ADR 0030 put the import engine
//! in this crate rather than composing it from public repository methods.
//!
//! **Nothing here evaluates a constraint.** The solver refused every hard-violating placement
//! and `evara_solver::generate` re-checked with the reference scorer; a third opinion in a
//! crate forbidden to depend on `evara-constraints` would be a fourth caller of the rules.
//! What this module does check is the **translation**: that every dense index resolves to a
//! stored identifier, which is the reverse of `Problem::integrity`.

use evara_core::scheduling::{
    Assignment, Placement, ProblemIdentity, Score, SolveOutcome, SolveStatus, Termination,
    UnplacedCause, UnverifiedCounts, Violation,
};
use rusqlite::{Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use super::row::{FromColumn, entity};
use super::{Expected, Stamp, delete, translate, update};
use crate::Database;
use crate::change_log::{Watermark, inputs_changed_since};
use crate::error::{DbError, Result};
use crate::meta::{self, KEY_LAST_OPENED_WITH};
use crate::scheduling::Assembly;

entity!(
    /// One stored timetable.
    ///
    /// The school is `cycle.school_id`: a child's school is resolved through its parent chain
    /// (`CLAUDE.md` §3), never by a second column.
    Timetable,
    /// What a caller supplies to create one.
    ///
    /// Not public: a timetable is created by [`Timetables::save_generated`] from a solve, never
    /// field by field. There is no hand-built timetable until Phase 8.
    TimetableInput,
    {
        /// The cycle whose grid this timetable is over.
        cycle_id: Uuid,
        /// The term, or `None` for a grid materialised for the whole year.
        term_id: Option<Uuid>,
        /// What a human calls it.
        name: String,
        /// When it was generated, ISO-8601 UTC.
        generated_at: String,
        /// Group pairs whose shared students nobody has declared.
        indeterminate_student_pairs: i64,
        /// Occurrences whose class size is unknown, so room capacity was not compared.
        unchecked_capacities: i64,
        /// Entities another school also holds.
        shared_entities: i64,
        /// Stored availability restrictions this engine does not enforce.
        unenforced_restrictions: i64,
    }
);

impl Timetable {
    /// What this timetable could **not** have checked.
    ///
    /// ADR 0032 rule 4: a timetable is described as having no *confirmed* clash within the
    /// school solved, with these counts beside it — never as conflict-free. Reassembled into
    /// the core type so a caller renders one shape whether the timetable came from a solve or
    /// from the database.
    #[must_use]
    pub fn unverified(&self) -> UnverifiedCounts {
        UnverifiedCounts {
            indeterminate_student_pairs: u32::try_from(self.indeterminate_student_pairs)
                .unwrap_or(u32::MAX),
            unchecked_capacities: u32::try_from(self.unchecked_capacities).unwrap_or(u32::MAX),
            shared_entities: u32::try_from(self.shared_entities).unwrap_or(u32::MAX),
            unenforced_restrictions: u32::try_from(self.unenforced_restrictions)
                .unwrap_or(u32::MAX),
        }
    }
}

entity!(
    /// How one timetable came to be.
    SolveRun,
    /// The provenance a caller supplies: the parts the engine does not know.
    SolveRunInput,
    {
        /// The timetable it produced.
        timetable_id: Uuid,
        /// Which engine ran.
        engine: String,
        /// The seed every ordering decision derived from.
        seed: i64,
        /// How it ended: what came back.
        status: String,
        /// How it ended: why the engine stopped looking.
        termination: String,
        /// The limits it ran under, as JSON.
        settings: String,
        /// Where the inputs stood when the problem was compiled.
        inputs_watermark: i64,
        /// Candidate placements whose legality was tested.
        candidate_evaluations: i64,
        /// Candidates tested and refused.
        candidates_rejected: i64,
        /// Placements committed to the working assignment.
        placements_committed: i64,
        /// Occurrences taken off the working assignment again.
        placements_withdrawn: i64,
        /// Occurrences attempted, retries included.
        construction_steps: i64,
        /// Sweeps over the unplaced occurrences.
        passes: i64,
        /// When the caller started it, ISO-8601 UTC.
        started_at: String,
        /// When the caller saw it finish, ISO-8601 UTC.
        finished_at: String,
        /// Which build ran it.
        app_version: String,
    }
);

entity!(
    /// One scoring of one timetable.
    TimetableEvaluation,
    /// What a caller supplies to record one.
    TimetableEvaluationInput,
    {
        /// Which timetable was scored.
        timetable_id: Uuid,
        /// `GENERATION` or `REVALIDATION`.
        kind: String,
        /// The run this scoring belongs to, for a `GENERATION`.
        solve_run_id: Option<Uuid>,
        /// When it was scored, ISO-8601 UTC.
        evaluated_at: String,
        /// Where the inputs stood when it was scored.
        inputs_watermark: i64,
        /// Hard penalties. Negative-going.
        score_hard: i64,
        /// Medium penalties.
        score_medium: i64,
        /// Soft penalties.
        score_soft: i64,
        /// How many occurrences were placed.
        placed_count: i64,
        /// How many the problem held.
        required_count: i64,
    }
);

impl TimetableEvaluation {
    /// The score this evaluation found.
    #[must_use]
    pub const fn score(&self) -> Score {
        Score::new(self.score_hard, self.score_medium, self.score_soft)
    }

    /// Whether this is the immutable record of how the timetable was generated.
    #[must_use]
    pub fn is_generation(&self) -> bool {
        self.kind == KIND_GENERATION
    }
}

/// An occurrence one run could not place.
///
/// Not an `entity!`: it carries no `rev` and is never updated. It is written once with its run
/// and removed only with it — see `NON_ENTITY_TABLES`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnscheduledRecord {
    /// Stable identifier.
    pub id: Uuid,
    /// The run that could not place it.
    pub solve_run_id: Uuid,
    /// Which occurrence.
    pub activity_id: Uuid,
    /// `UnplacedCause::key()`.
    pub cause: String,
    /// The pre-solve finding that already proved it, where there was one.
    pub diagnostic_code: Option<String>,
}

impl UnscheduledRecord {
    /// Whether this cause proves no timetable can place the occurrence.
    ///
    /// Reads the stored key through [`UnplacedCause`] rather than storing a second flag: the
    /// answer is a pure function of the cause, and a stored copy is a second place for it to
    /// disagree. An unrecognised key — written by a newer build — is **not** reported as
    /// impossible, because overclaiming is the failure this distinction exists to prevent.
    #[must_use]
    pub fn is_impossible(&self) -> bool {
        cause_from_key(&self.cause).is_some_and(UnplacedCause::is_impossible)
    }
}

/// `GENERATION`, as stored.
pub const KIND_GENERATION: &str = "GENERATION";
/// `REVALIDATION`, as stored.
pub const KIND_REVALIDATION: &str = "REVALIDATION";

/// The parts of a run only the caller knows.
///
/// The engine reads no clock and does not know which build is running, so these four cross the
/// boundary rather than being invented here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProvenance {
    /// Which engine ran — `"construction"` for M6's.
    pub engine: String,
    /// The limits it ran under, as a JSON object.
    pub settings: String,
    /// When the caller started it.
    pub started_at: String,
    /// When the caller saw it finish.
    pub finished_at: String,
}

/// Everything one save writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTimetable {
    /// The timetable row.
    pub timetable: Timetable,
    /// The run that produced it.
    pub run: SolveRun,
    /// The immutable `GENERATION` scoring.
    pub evaluation: TimetableEvaluation,
    /// How many entries were written.
    pub entries: usize,
    /// How many unplaced occurrences were recorded.
    pub unscheduled: usize,
    /// How many violations were stored.
    pub violations: usize,
}

/// What one scoring of a timetable concluded.
///
/// Grouped rather than passed as four arguments, because the four are one answer: a score, how
/// much of the timetable it was over, and what it found. A caller that had them apart could
/// pass a score from one evaluation with the violations of another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scoring<'v> {
    /// The score the rules produced. Negative-going.
    pub score: Score,
    /// How many occurrences were placed.
    pub placed: usize,
    /// How many the problem held.
    pub required: usize,
    /// What the rules found.
    pub violations: &'v [Violation],
}

/// Reads and writes stored timetables.
pub struct Timetables<'a> {
    db: &'a mut Database,
}

impl<'a> Timetables<'a> {
    /// Wraps an open project.
    pub(crate) const fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

impl Timetables<'_> {
    /// Saves a generated timetable, its run, and the generation scoring.
    ///
    /// # One transaction
    ///
    /// Every row goes in together or none does. A failure on the last violation leaves no
    /// `timetable`, which is what makes a half-saved timetable impossible rather than unlikely.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotSaveable`] if the outcome is not a timetable: a run refused at the
    ///   pre-solve gate attempted nothing, and one whose own final check failed produced an
    ///   arrangement the reference scorer rejected.
    /// - [`DbError::InputsChanged`] if any scheduling input moved since the problem was
    ///   compiled. Checked inside this transaction — see the module header.
    /// - [`DbError::Invalid`] if the outcome does not describe the assembly it is offered with.
    /// - [`DbError::CorruptIdentifier`] if a dense index resolves to no stored row.
    /// - [`DbError::DuplicateValue`] if a timetable of that name already exists for the scope.
    pub fn save_generated(
        &mut self,
        assembly: &Assembly,
        outcome: &SolveOutcome,
        violations: &[Violation],
        run: &RunProvenance,
        name: &str,
    ) -> Result<SavedTimetable> {
        refuse_unsaveable(outcome)?;
        require_same_problem(assembly, outcome)?;

        // The build that is running, which is what the sketch called `solver_version`.
        // `app_meta` already owns the key, so the honest value is the one `open_existing`
        // stamped rather than a second notion of a version.
        let app_version = self
            .db
            .read(|conn| meta::get(conn, KEY_LAST_OPENED_WITH))?
            .unwrap_or_else(|| String::from("unknown"));
        let plan = project_entries(assembly, &outcome.assignment)?;
        let unplaced = project_unscheduled(assembly, outcome)?;
        let watermark = assembly.watermark;
        let selection = assembly.selection;
        let counts = outcome.unverified;
        let score = outcome.score;
        let placed = plan.len();
        let required = assembly.problem.activities.len();
        let name = name.trim().to_owned();

        self.db.write(|tx| {
            // The freshness predicate. Inside the transaction that writes, so it is not
            // check-then-act: nothing can move between the question and the answer.
            guard_freshness(tx, watermark)?;

            let timetable = insert_timetable(tx, &selection, &name, counts, outcome)?;
            for (activity_id, placement) in &plan {
                insert_entry(tx, timetable.id, *activity_id, placement)?;
            }
            let run_row = insert_run(tx, timetable.id, outcome, run, watermark, &app_version)?;
            for record in &unplaced {
                insert_unscheduled(tx, run_row.id, record)?;
            }
            let evaluation = insert_evaluation(
                tx,
                timetable.id,
                KIND_GENERATION,
                Some(run_row.id),
                watermark,
                score,
                placed,
                required,
            )?;
            for violation in violations {
                insert_violation(tx, evaluation.id, assembly, violation)?;
            }

            Ok(SavedTimetable {
                timetable,
                run: run_row,
                evaluation,
                entries: plan.len(),
                unscheduled: unplaced.len(),
                violations: violations.len(),
            })
        })
    }

    /// Every timetable of one school, newest first.
    ///
    /// The school is resolved through `cycle`, which is the join deviation 2 of `V17` pays for
    /// not carrying a second `school_id`.
    pub fn for_school(&self, school_id: Uuid) -> Result<Vec<Timetable>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT t.id, t.cycle_id, t.term_id, t.name, t.generated_at,
                        t.indeterminate_student_pairs, t.unchecked_capacities,
                        t.shared_entities, t.unenforced_restrictions,
                        t.created_at, t.updated_at, t.rev
                 FROM timetable t
                 JOIN cycle c ON c.id = t.cycle_id
                 WHERE c.school_id = ?1
                 ORDER BY t.generated_at DESC, t.id",
            )?;
            let rows = stmt.query_map([school_id.to_string()], Timetable::from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// One timetable by identifier.
    ///
    /// # Errors
    ///
    /// [`DbError::NotFound`] if the project holds no such timetable.
    pub fn get(&self, id: Uuid) -> Result<Timetable> {
        self.db
            .read(|conn| read_timetable(conn, id))?
            .ok_or(DbError::NotFound {
                entity: "timetable",
            })
    }

    /// The arrangement a stored timetable describes, against the problem it belongs to.
    ///
    /// # The round trip that makes a stored timetable trustworthy
    ///
    /// Reading the entries back and rebuilding an [`Assignment`] is the exact inverse of
    /// saving one, and because an entry stores the **start** slot and nothing derived, the
    /// result is byte-identical to the `Assignment` that was saved. A test asserts that over
    /// every fixture, and it is the single most valuable test in this module: if it holds, no
    /// stored timetable can mean something other than what was generated.
    ///
    /// An entry naming an activity the problem no longer holds is **skipped and reported**
    /// rather than failing the read: a timetable whose lessons have been deleted is still worth
    /// opening, and a reader that refused would leave the user unable to see what to fix.
    ///
    /// # Errors
    ///
    /// As [`Timetables::get`], plus any SQLite failure.
    pub fn arrangement(&self, id: Uuid, assembly: &Assembly) -> Result<StoredArrangement> {
        let entries = self.entries(id)?;
        let mut assignment = Assignment::empty(assembly.problem.activities.len());
        let mut unknown = Vec::new();

        for entry in &entries {
            let (Some(activity), Some(timeslot)) = (
                assembly.index.activity_index(entry.activity_id),
                assembly.index.timeslot_index(entry.timeslot_id),
            ) else {
                unknown.push(entry.activity_id);
                continue;
            };
            let room = entry
                .room_id
                .and_then(|room| assembly.index.room_index(room));
            if entry.room_id.is_some() && room.is_none() {
                unknown.push(entry.activity_id);
                continue;
            }
            assignment.place(activity, Placement { timeslot, room });
        }

        Ok(StoredArrangement {
            assignment,
            entries: entries.len(),
            not_in_problem: unknown,
        })
    }

    /// The stored entries of one timetable, in a deterministic order.
    pub fn entries(&self, timetable_id: Uuid) -> Result<Vec<TimetableEntry>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timetable_id, activity_id, timeslot_id, room_id,
                        is_locked, is_manual, created_at, updated_at, rev
                 FROM timetable_entry
                 WHERE timetable_id = ?1
                 ORDER BY activity_id",
            )?;
            let rows = stmt.query_map([timetable_id.to_string()], TimetableEntry::from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// The run that produced one timetable, where there is one.
    ///
    /// `None` for a timetable built by hand, which Phase 8 makes possible and nothing does yet.
    pub fn run_for(&self, timetable_id: Uuid) -> Result<Option<SolveRun>> {
        self.db.read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, timetable_id, engine, seed, status, termination, settings,
                            inputs_watermark, candidate_evaluations, candidates_rejected,
                            placements_committed, placements_withdrawn, construction_steps,
                            passes, started_at, finished_at, app_version,
                            created_at, updated_at, rev
                     FROM solve_run WHERE timetable_id = ?1",
                    [timetable_id.to_string()],
                    SolveRun::from_row,
                )
                .optional()?)
        })
    }

    /// What one run could not place, in activity order.
    pub fn unscheduled_for(&self, solve_run_id: Uuid) -> Result<Vec<UnscheduledRecord>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, solve_run_id, activity_id, cause, diagnostic_code
                 FROM solve_run_unscheduled
                 WHERE solve_run_id = ?1
                 ORDER BY activity_id",
            )?;
            let rows = stmt.query_map([solve_run_id.to_string()], |row| {
                Ok(UnscheduledRecord {
                    id: FromColumn::read(row, 0)?,
                    solve_run_id: FromColumn::read(row, 1)?,
                    activity_id: FromColumn::read(row, 2)?,
                    cause: row.get(3)?,
                    diagnostic_code: row.get(4)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| translate(e, "timetable"))
        })
    }

    /// The scorings of one timetable: the immutable generation one, and the current one.
    pub fn evaluations(&self, timetable_id: Uuid) -> Result<Vec<TimetableEvaluation>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timetable_id, kind, solve_run_id, evaluated_at, inputs_watermark,
                        score_hard, score_medium, score_soft, placed_count, required_count,
                        created_at, updated_at, rev
                 FROM timetable_evaluation
                 WHERE timetable_id = ?1
                 ORDER BY kind",
            )?;
            let rows = stmt.query_map([timetable_id.to_string()], TimetableEvaluation::from_row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// The violations one scoring found.
    pub fn violations_for(&self, evaluation_id: Uuid) -> Result<Vec<StoredViolation>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, evaluation_id, constraint_type, level, constraint_id, penalty,
                        cycle_day_id, starts_at, ends_at, detail
                 FROM violation WHERE evaluation_id = ?1 ORDER BY id",
            )?;
            let found = stmt
                .query_map([evaluation_id.to_string()], |row| {
                    Ok(StoredViolation {
                        id: FromColumn::read(row, 0)?,
                        evaluation_id: FromColumn::read(row, 1)?,
                        constraint_type: row.get(2)?,
                        level: row.get(3)?,
                        constraint_id: FromColumn::read(row, 4)?,
                        penalty: row.get(5)?,
                        cycle_day_id: FromColumn::read(row, 6)?,
                        starts_at: row.get(7)?,
                        ends_at: row.get(8)?,
                        detail: row.get(9)?,
                        entities: Vec::new(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut with_refs = Vec::with_capacity(found.len());
            for mut violation in found {
                let mut stmt = conn.prepare(
                    "SELECT role, entity_kind, entity_id FROM violation_ref
                     WHERE violation_id = ?1 ORDER BY role, ordinal",
                )?;
                violation.entities = stmt
                    .query_map([violation.id.to_string()], |row| {
                        Ok(StoredViolationRef {
                            role: row.get(0)?,
                            entity_kind: row.get(1)?,
                            entity_id: row.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                with_refs.push(violation);
            }
            Ok(with_refs)
        })
    }

    /// Replaces the **current** scoring of a timetable, leaving its generation record alone.
    ///
    /// # What this may and may not touch
    ///
    /// Only the `REVALIDATION` evaluation and its violations. The `GENERATION` row is immutable
    /// for the life of the timetable, so re-scoring can never misrepresent how a timetable was
    /// generated — which is what the two kinds exist to keep apart.
    ///
    /// Two snapshots of a changing world are not additive, so the previous current scoring is
    /// **replaced** rather than merged, in the same transaction as the new one.
    ///
    /// # Errors
    ///
    /// - [`DbError::Conflict`] if the timetable has moved since `expected`.
    /// - [`DbError::NotFound`] if there is no such timetable.
    pub fn record_revalidation(
        &mut self,
        timetable_id: Uuid,
        expected: i64,
        scoring: &Scoring<'_>,
        assembly: &Assembly,
    ) -> Result<TimetableEvaluation> {
        let watermark = assembly.watermark;
        let Scoring {
            score,
            placed,
            required,
            violations,
        } = *scoring;
        self.db.write(|tx| {
            // The timetable's own revision guards the whole replacement, and **consumes** it:
            // recording a new current scoring changes what the timetable says about itself, so
            // its revision moves and the change feed records it. A caller revalidating twice
            // re-reads in between, which is what every other update in this crate asks of one.
            //
            // ADR 0027 §3 category 3 — the revision of the row that changes.
            update(
                tx,
                "timetable",
                "timetable",
                timetable_id,
                Expected::Rev(expected),
                "name = name",
                &[],
            )?;

            clear_revalidation(tx, timetable_id)?;
            let evaluation = insert_evaluation(
                tx,
                timetable_id,
                KIND_REVALIDATION,
                None,
                watermark,
                score,
                placed,
                required,
            )?;
            for violation in violations {
                insert_violation(tx, evaluation.id, assembly, violation)?;
            }
            Ok(evaluation)
        })
    }

    /// Renames one timetable.
    ///
    /// # Errors
    ///
    /// [`DbError::Conflict`] if it has moved since `expected`; [`DbError::DuplicateValue`] if
    /// the scope already holds that name.
    pub fn rename(&mut self, id: Uuid, expected: i64, name: &str) -> Result<Timetable> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(DbError::Invalid {
                message: "a timetable needs a name".to_owned(),
            });
        }
        self.db.write(|tx| {
            update(
                tx,
                "timetable",
                "timetable",
                id,
                Expected::Rev(expected),
                "name = ?",
                &[&name],
            )?;
            read_timetable(tx, id)?.ok_or(DbError::NotFound {
                entity: "timetable",
            })
        })
    }

    /// Deletes one timetable and everything that belongs to it.
    ///
    /// # The children are removed, not cascaded
    ///
    /// Every child edge is `RESTRICT` and this method deletes the rows itself — one
    /// [`delete`](super::delete) each, inside the timetable's transaction, at
    /// [`Expected::Any`]. ADR 0040 §2: SQLite fires a `DELETE` trigger for a foreign-key
    /// action only under `recursive_triggers`, which this project never enables, so a
    /// `CASCADE` would remove the rows **with no change-log entry** — silently, and only for
    /// the rows the cascade reached.
    ///
    /// `Expected::Any` on the children is ADR 0027 §4's first documented kind: a cascade inside
    /// another operation's transaction. The caller read the timetable's revision, not each
    /// entry's, and demanding one would mean inventing it.
    ///
    /// # Errors
    ///
    /// [`DbError::Conflict`] if the timetable has moved since `expected`.
    pub fn delete(&mut self, id: Uuid, expected: i64) -> Result<DeletedTimetable> {
        self.db.write(|tx| {
            // The children go first, because every edge is RESTRICT and the parent cannot be
            // removed while one survives. The revision is checked by the **parent's own
            // delete**, last, rather than by a no-op update first: an update would consume the
            // revision it was meant to check and then fail its own predicate. A stale caller
            // therefore reaches the end and is refused there, and the transaction rolls the
            // children back — which is the same guarantee, without a second write.
            let mut removed = DeletedTimetable::default();

            // Deepest first: violations before their evaluation, unscheduled before its run.
            for evaluation in ids_of(tx, "timetable_evaluation", "timetable_id", id)? {
                removed.violations += delete_violations_of(tx, evaluation)?;
                delete(
                    tx,
                    "timetableEvaluation",
                    "timetable_evaluation",
                    evaluation,
                    Expected::Any,
                )?;
                removed.evaluations += 1;
            }
            for run in ids_of(tx, "solve_run", "timetable_id", id)? {
                for unscheduled in ids_of(tx, "solve_run_unscheduled", "solve_run_id", run)? {
                    tx.execute(
                        "DELETE FROM solve_run_unscheduled WHERE id = ?1",
                        [unscheduled.to_string()],
                    )?;
                    removed.unscheduled += 1;
                }
                delete(tx, "solveRun", "solve_run", run, Expected::Any)?;
                removed.runs += 1;
            }
            for entry in ids_of(tx, "timetable_entry", "timetable_id", id)? {
                delete(
                    tx,
                    "timetableEntry",
                    "timetable_entry",
                    entry,
                    Expected::Any,
                )?;
                removed.entries += 1;
            }

            delete(tx, "timetable", "timetable", id, Expected::Rev(expected))?;
            Ok(removed)
        })
    }
}

entity!(
    /// One placed occurrence, as stored.
    TimetableEntry,
    /// What a caller supplies to place one.
    TimetableEntryInput,
    {
        /// The timetable it belongs to.
        timetable_id: Uuid,
        /// The occurrence placed.
        activity_id: Uuid,
        /// The slot it **starts** in. The run it occupies is derived.
        timeslot_id: Uuid,
        /// The room booked, where one was.
        room_id: Option<Uuid>,
        /// Whether the solver must preserve it. Phase 8's.
        is_locked: bool,
        /// Whether a human placed it. Phase 8's.
        is_manual: bool,
    }
);

/// What reading a stored timetable back produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArrangement {
    /// The arrangement, ready for the constraint engine.
    pub assignment: Assignment,
    /// How many entries were read.
    pub entries: usize,
    /// Entries naming something the problem no longer holds.
    ///
    /// Empty for a timetable whose lessons and grid are intact. Non-empty means the timetable
    /// is **readable but incomplete**: those placements are not in the arrangement, and a
    /// caller scoring it is scoring less than was stored. Reported rather than refused, because
    /// a timetable whose lessons were deleted is still worth opening.
    pub not_in_problem: Vec<Uuid>,
}

/// One stored violation, with the entities it names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredViolation {
    /// Stable identifier.
    pub id: Uuid,
    /// The scoring that found it.
    pub evaluation_id: Uuid,
    /// The registry's `type_key`.
    pub constraint_type: String,
    /// `HARD`, `MEDIUM` or `SOFT`.
    pub level: String,
    /// The stored instance that produced it, where there is one.
    pub constraint_id: Option<Uuid>,
    /// Negative-going.
    pub penalty: i64,
    /// The cycle position, where the violation has a when.
    pub cycle_day_id: Option<Uuid>,
    /// `'HH:MM'`.
    pub starts_at: Option<String>,
    /// `'HH:MM'`.
    pub ends_at: Option<String>,
    /// The specific fact.
    pub detail: String,
    /// The entities it names.
    pub entities: Vec<StoredViolationRef>,
}

/// One entity a stored violation names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredViolationRef {
    /// `activity`, `subject` or `related`.
    pub role: String,
    /// What kind of thing it is.
    pub entity_kind: String,
    /// Which one.
    pub entity_id: String,
}

/// What deleting a timetable removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeletedTimetable {
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

// =======================================================================================
// Guards
// =======================================================================================

/// Refuses an outcome that is not a timetable.
///
/// Two states, and neither is a timetable. A run refused at the pre-solve gate attempted
/// nothing — the findings belong to the report, not to a stored artefact. A run whose own final
/// check failed produced an arrangement the **reference** scorer rejected, and storing it would
/// put something in the user's file that no screen could honestly render.
fn refuse_unsaveable(outcome: &SolveOutcome) -> Result<()> {
    let reason = match (outcome.status, outcome.termination) {
        (_, Termination::InputRejected) => {
            Some("pre-solve validation refused this school's inputs, so nothing was attempted")
        }
        (_, Termination::InternalCheckFailed) => {
            Some("the engine's own final check found a hard violation in this arrangement")
        }
        (SolveStatus::Failed, _) => Some("this run did not produce a timetable"),
        _ => None,
    };
    match reason {
        Some(reason) => Err(DbError::NotSaveable {
            reason: reason.to_owned(),
        }),
        None => Ok(()),
    }
}

/// Refuses an outcome that does not describe the assembly it is offered with.
///
/// The failure this prevents is the worst shape available: a `SolveOutcome` written against a
/// *different* assembly's index would translate every dense index to the wrong identifier and
/// store a complete, plausible, **wrong** timetable. Both halves are checked, because the
/// second is the subtler one — two loads of the same selection can differ in activity count.
fn require_same_problem(assembly: &Assembly, outcome: &SolveOutcome) -> Result<()> {
    let expected = ProblemIdentity {
        school: assembly.problem.context.school.id,
        cycle: assembly.selection.cycle_id,
        term: assembly.selection.term_id,
    };
    if outcome.problem != expected {
        return Err(DbError::Invalid {
            message: format!(
                "this result was generated for a different school, cycle or term \
                 ({:?}) than the problem it is being saved against ({expected:?})",
                outcome.problem
            ),
        });
    }
    if outcome.assignment.len() != assembly.problem.activities.len() {
        return Err(DbError::Invalid {
            message: format!(
                "this result covers {} occurrences and the problem holds {}; it was \
                 generated against a different load",
                outcome.assignment.len(),
                assembly.problem.activities.len()
            ),
        });
    }
    Ok(())
}

/// Refuses the save if any scheduling input moved since the problem was compiled.
///
/// Inside the writing transaction, which is what makes it a predicate rather than a question.
fn guard_freshness(tx: &Transaction<'_>, watermark: Watermark) -> Result<()> {
    if !inputs_changed_since(tx, watermark)? {
        return Ok(());
    }
    let changes: i64 = tx.query_row(
        "SELECT COUNT(*) FROM change_log WHERE seq > ?1",
        [watermark.get()],
        |row| row.get(0),
    )?;
    Err(DbError::InputsChanged {
        generated_at_seq: watermark.get(),
        changes_since: changes,
    })
}

// =======================================================================================
// Projection: dense indices to stored identifiers
// =======================================================================================

/// Turns an [`Assignment`] into the rows to write.
///
/// The reverse of `Problem::integrity`: every dense index must resolve to a stored identifier,
/// and one that does not is corruption rather than input — substituting a default would make a
/// damaged load look like a working one.
fn project_entries(
    assembly: &Assembly,
    assignment: &Assignment,
) -> Result<Vec<(Uuid, StoredPlacement)>> {
    let index = &assembly.index;
    let mut rows = Vec::with_capacity(assignment.scheduled_count());

    for (activity, placement) in assignment.iter() {
        let activity_id = index
            .activity_id(activity)
            .ok_or_else(|| corrupt("activity", activity.raw()))?;
        let timeslot_id = index
            .timeslot_id(placement.timeslot)
            .ok_or_else(|| corrupt("timeslot", placement.timeslot.raw()))?;
        let room_id = match placement.room {
            None => None,
            Some(room) => Some(
                index
                    .room_id(room)
                    .ok_or_else(|| corrupt("room", room.raw()))?,
            ),
        };
        rows.push((
            activity_id,
            StoredPlacement {
                timeslot_id,
                room_id,
            },
        ));
    }
    // By activity identifier, so two saves of one arrangement write the rows in one order.
    rows.sort_by_key(|(activity, _)| *activity);
    Ok(rows)
}

/// A placement with its identifiers resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredPlacement {
    timeslot_id: Uuid,
    room_id: Option<Uuid>,
}

/// Turns the unplaced list into the rows to write.
fn project_unscheduled(
    assembly: &Assembly,
    outcome: &SolveOutcome,
) -> Result<Vec<UnscheduledProjection>> {
    let mut rows = Vec::with_capacity(outcome.unscheduled.len());
    for unplaced in &outcome.unscheduled {
        let activity_id = assembly
            .index
            .activity_id(unplaced.activity)
            .ok_or_else(|| corrupt("activity", unplaced.activity.raw()))?;
        rows.push(UnscheduledProjection {
            activity_id,
            cause: unplaced.cause.key().to_owned(),
            diagnostic_code: unplaced
                .diagnostic
                .as_ref()
                .map(|finding| finding.code.key().to_owned()),
        });
    }
    rows.sort_by_key(|row| row.activity_id);
    Ok(rows)
}

/// One unplaced occurrence, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnscheduledProjection {
    activity_id: Uuid,
    cause: String,
    diagnostic_code: Option<String>,
}

/// The cause a stored key names.
fn cause_from_key(key: &str) -> Option<UnplacedCause> {
    [
        UnplacedCause::NoResolvedBellSchedule,
        UnplacedCause::NoContiguousRun,
        UnplacedCause::NoCandidateRoom,
        UnplacedCause::AllCandidatesUnavailable,
        UnplacedCause::BlockedByPlacements,
        UnplacedCause::NotReached,
        UnplacedCause::InputRejected,
    ]
    .into_iter()
    .find(|cause| cause.key() == key)
}

/// A dense index that resolves to no stored row.
fn corrupt(what: &'static str, index: u32) -> DbError {
    DbError::CorruptIdentifier {
        column: what.to_owned(),
        value: format!("dense index {index} resolves to no stored {what}"),
    }
}

// =======================================================================================
// Inserts
// =======================================================================================

fn insert_timetable(
    tx: &Transaction<'_>,
    selection: &crate::scheduling::SchedulingSelection,
    name: &str,
    counts: UnverifiedCounts,
    outcome: &SolveOutcome,
) -> Result<Timetable> {
    let stamp = Stamp::new();
    let generated_at = stamp.created_at.clone();
    let _ = outcome;
    tx.execute(
        "INSERT INTO timetable (
             id, cycle_id, term_id, name, generated_at,
             indeterminate_student_pairs, unchecked_capacities, shared_entities,
             unenforced_restrictions, created_at, updated_at, rev)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            stamp.id.to_string(),
            selection.cycle_id.to_string(),
            selection.term_id.map(|id| id.to_string()),
            name,
            generated_at,
            i64::from(counts.indeterminate_student_pairs),
            i64::from(counts.unchecked_capacities),
            i64::from(counts.shared_entities),
            i64::from(counts.unenforced_restrictions),
            stamp.created_at,
            stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;

    read_timetable(tx, stamp.id)?.ok_or(DbError::NotFound {
        entity: "timetable",
    })
}

fn insert_entry(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    activity_id: Uuid,
    placement: &StoredPlacement,
) -> Result<()> {
    let stamp = Stamp::new();
    tx.execute(
        "INSERT INTO timetable_entry (
             id, timetable_id, activity_id, timeslot_id, room_id,
             is_locked, is_manual, created_at, updated_at, rev)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6, ?7, ?8)",
        rusqlite::params![
            stamp.id.to_string(),
            timetable_id.to_string(),
            activity_id.to_string(),
            placement.timeslot_id.to_string(),
            placement.room_id.map(|id| id.to_string()),
            stamp.created_at,
            stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;
    Ok(())
}

fn insert_run(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    outcome: &SolveOutcome,
    run: &RunProvenance,
    watermark: Watermark,
    app_version: &str,
) -> Result<SolveRun> {
    let stamp = Stamp::new();
    let statistics = outcome.statistics;
    tx.execute(
        "INSERT INTO solve_run (
             id, timetable_id, engine, seed, status, termination, settings, inputs_watermark,
             candidate_evaluations, candidates_rejected, placements_committed,
             placements_withdrawn, construction_steps, passes,
             started_at, finished_at, app_version, created_at, updated_at, rev)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, ?18, ?19, ?20)",
        rusqlite::params![
            stamp.id.to_string(),
            timetable_id.to_string(),
            run.engine,
            i64::try_from(outcome.seed.get()).unwrap_or(i64::MAX),
            status_key(outcome.status),
            outcome.termination.key(),
            run.settings,
            watermark.get(),
            i64::try_from(statistics.candidate_evaluations).unwrap_or(i64::MAX),
            i64::try_from(statistics.candidates_rejected).unwrap_or(i64::MAX),
            i64::try_from(statistics.placements_committed).unwrap_or(i64::MAX),
            i64::try_from(statistics.placements_withdrawn).unwrap_or(i64::MAX),
            i64::try_from(statistics.construction_steps).unwrap_or(i64::MAX),
            i64::from(statistics.passes),
            run.started_at,
            run.finished_at,
            app_version,
            stamp.created_at,
            stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;

    Ok(tx.query_row(
        "SELECT id, timetable_id, engine, seed, status, termination, settings,
                inputs_watermark, candidate_evaluations, candidates_rejected,
                placements_committed, placements_withdrawn, construction_steps, passes,
                started_at, finished_at, app_version, created_at, updated_at, rev
         FROM solve_run WHERE id = ?1",
        [stamp.id.to_string()],
        SolveRun::from_row,
    )?)
}

fn insert_unscheduled(
    tx: &Transaction<'_>,
    solve_run_id: Uuid,
    record: &UnscheduledProjection,
) -> Result<()> {
    tx.execute(
        "INSERT INTO solve_run_unscheduled (
             id, solve_run_id, activity_id, cause, diagnostic_code)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            Uuid::now_v7().to_string(),
            solve_run_id.to_string(),
            record.activity_id.to_string(),
            record.cause,
            record.diagnostic_code,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_evaluation(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    kind: &str,
    solve_run_id: Option<Uuid>,
    watermark: Watermark,
    score: Score,
    placed: usize,
    required: usize,
) -> Result<TimetableEvaluation> {
    let stamp = Stamp::new();
    tx.execute(
        "INSERT INTO timetable_evaluation (
             id, timetable_id, kind, solve_run_id, evaluated_at, inputs_watermark,
             score_hard, score_medium, score_soft, placed_count, required_count,
             created_at, updated_at, rev)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            stamp.id.to_string(),
            timetable_id.to_string(),
            kind,
            solve_run_id.map(|id| id.to_string()),
            stamp.created_at,
            watermark.get(),
            score.hard,
            score.medium,
            score.soft,
            i64::try_from(placed).unwrap_or(i64::MAX),
            i64::try_from(required).unwrap_or(i64::MAX),
            stamp.created_at,
            stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;

    Ok(tx.query_row(
        "SELECT id, timetable_id, kind, solve_run_id, evaluated_at, inputs_watermark,
                score_hard, score_medium, score_soft, placed_count, required_count,
                created_at, updated_at, rev
         FROM timetable_evaluation WHERE id = ?1",
        [stamp.id.to_string()],
        TimetableEvaluation::from_row,
    )?)
}

/// Writes one violation and the entities it names.
///
/// The references are stored with their ordinal, so a report reads the same way twice: the
/// engine produced them in an order and that order is part of the finding.
fn insert_violation(
    tx: &Transaction<'_>,
    evaluation_id: Uuid,
    assembly: &Assembly,
    violation: &Violation,
) -> Result<()> {
    let id = Uuid::now_v7();
    let cycle_day_id = violation
        .cycle_day
        .and_then(|day| assembly.index.cycle_day_id(day));
    let (starts_at, ends_at) = violation.span.map_or((None, None), |span| {
        (
            Some(clock_text(span.start.minutes())),
            Some(clock_text(span.end.minutes())),
        )
    });

    tx.execute(
        "INSERT INTO violation (
             id, evaluation_id, constraint_type, level, constraint_id, penalty,
             cycle_day_id, starts_at, ends_at, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id.to_string(),
            evaluation_id.to_string(),
            violation.constraint_type.key(),
            violation.level.stored(),
            violation.constraint_id.map(|c| c.to_string()),
            violation.penalty,
            cycle_day_id.map(|d| d.to_string()),
            starts_at,
            ends_at,
            violation.detail,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;

    for (ordinal, activity) in violation.activities.iter().enumerate() {
        let Some(activity_id) = assembly.index.activity_id(*activity) else {
            continue;
        };
        insert_violation_ref(
            tx,
            id,
            "activity",
            "activity",
            &activity_id.to_string(),
            ordinal,
        )?;
    }
    insert_violation_ref(
        tx,
        id,
        "subject",
        &entity_kind_key(violation.subject.kind),
        &violation.subject.id.to_string(),
        0,
    )?;
    for (ordinal, related) in violation.related.iter().enumerate() {
        insert_violation_ref(
            tx,
            id,
            "related",
            &entity_kind_key(related.kind),
            &related.id.to_string(),
            ordinal,
        )?;
    }
    Ok(())
}

fn insert_violation_ref(
    tx: &Transaction<'_>,
    violation_id: Uuid,
    role: &str,
    entity_kind: &str,
    entity_id: &str,
    ordinal: usize,
) -> Result<()> {
    tx.execute(
        "INSERT INTO violation_ref (id, violation_id, role, entity_kind, entity_id, ordinal)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            Uuid::now_v7().to_string(),
            violation_id.to_string(),
            role,
            entity_kind,
            entity_id,
            i64::try_from(ordinal).unwrap_or(i64::MAX),
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;
    Ok(())
}

// =======================================================================================
// Reads and deletes used by both paths
// =======================================================================================

fn read_timetable(conn: &Connection, id: Uuid) -> Result<Option<Timetable>> {
    Ok(conn
        .query_row(
            "SELECT id, cycle_id, term_id, name, generated_at,
                    indeterminate_student_pairs, unchecked_capacities, shared_entities,
                    unenforced_restrictions, created_at, updated_at, rev
             FROM timetable WHERE id = ?1",
            [id.to_string()],
            Timetable::from_row,
        )
        .optional()?)
}

/// The identifiers of every child row of one parent, oldest first.
fn ids_of(tx: &Transaction<'_>, table: &str, column: &str, parent: Uuid) -> Result<Vec<Uuid>> {
    let table = super::identifier(table)?;
    let column = super::identifier(column)?;
    let sql = format!("SELECT id FROM {table} WHERE {column} = ?1 ORDER BY id");
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map([parent.to_string()], |row| FromColumn::read(row, 0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| translate(e, "timetable"))
}

/// Removes one evaluation's violations and their references.
fn delete_violations_of(tx: &Transaction<'_>, evaluation_id: Uuid) -> Result<usize> {
    let violations = ids_of(tx, "violation", "evaluation_id", evaluation_id)?;
    for violation in &violations {
        tx.execute(
            "DELETE FROM violation_ref WHERE violation_id = ?1",
            [violation.to_string()],
        )?;
        tx.execute(
            "DELETE FROM violation WHERE id = ?1",
            [violation.to_string()],
        )?;
    }
    Ok(violations.len())
}

/// Removes the current scoring of one timetable, if it has one.
fn clear_revalidation(tx: &Transaction<'_>, timetable_id: Uuid) -> Result<()> {
    let existing: Option<Uuid> = tx
        .query_row(
            "SELECT id FROM timetable_evaluation
             WHERE timetable_id = ?1 AND kind = ?2",
            rusqlite::params![timetable_id.to_string(), KIND_REVALIDATION],
            |row| FromColumn::read(row, 0),
        )
        .optional()?;
    if let Some(evaluation) = existing {
        delete_violations_of(tx, evaluation)?;
        delete(
            tx,
            "timetableEvaluation",
            "timetable_evaluation",
            evaluation,
            Expected::Any,
        )?;
    }
    Ok(())
}

/// An [`EntityKind`] as the string a `violation_ref` stores.
///
/// Serialised through serde rather than a second `match`, so the stored vocabulary and the
/// wire vocabulary are the same one and cannot drift. `CLAUDE.md`: the `snake_case`/`camelCase`
/// mapping is applied "in exactly one place with serde attributes", and this is that place
/// being used rather than bypassed.
fn entity_kind_key(kind: evara_core::scheduling::EntityKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from("unknown"))
}

/// `SolveStatus` as the `CHECK` string.
const fn status_key(status: SolveStatus) -> &'static str {
    match status {
        SolveStatus::Complete => "COMPLETE",
        SolveStatus::Partial => "PARTIAL",
        SolveStatus::Infeasible => "INFEASIBLE",
        SolveStatus::Cancelled => "CANCELLED",
        SolveStatus::Failed => "FAILED",
    }
}

/// Minutes since midnight as `'HH:MM'`, zero-padded so text order is chronological.
fn clock_text(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}
