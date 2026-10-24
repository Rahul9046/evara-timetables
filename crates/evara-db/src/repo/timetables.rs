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
use super::{Expected, Stamp, delete, require_school_owner, translate, update};
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
                // A generated placement is neither locked nor hand-placed, and saying so
                // explicitly is what makes M9A's two other writers readable beside it.
                insert_entry(
                    tx,
                    timetable.id,
                    *activity_id,
                    placement,
                    EntryFlags {
                        is_locked: false,
                        is_manual: false,
                    },
                )?;
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

    /// Every timetable over one `(cycle, term)`, newest first.
    ///
    /// The narrower read [`Timetables::for_school`] cannot express, and the one the Generate
    /// screen needs: before generating, a user is told how many timetables this grid already
    /// holds, because a save **always inserts** (ADR 0042 D3) and that is easier to accept
    /// when the count is visible beforehand than when a fourth version appears in a list.
    ///
    /// `COALESCE` on the term for the reason ADR 0042 C1 gives: SQLite treats `NULL`s as
    /// distinct, so `term_id = ?2` would silently match nothing for exactly the whole-year
    /// timetables, which are the common case.
    pub fn for_cycle(&self, cycle_id: Uuid, term_id: Option<Uuid>) -> Result<Vec<Timetable>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, cycle_id, term_id, name, generated_at,
                        indeterminate_student_pairs, unchecked_capacities,
                        shared_entities, unenforced_restrictions,
                        created_at, updated_at, rev
                 FROM timetable
                 WHERE cycle_id = ?1 AND COALESCE(term_id, '') = COALESCE(?2, '')
                 ORDER BY generated_at DESC, id",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![cycle_id.to_string(), term_id.map(|term| term.to_string())],
                Timetable::from_row,
            )?;
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
        self.db.read(|conn| read_entries(conn, timetable_id))
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
        /// Whether a repair must preserve this placement.
        ///
        /// Written by [`Timetables::set_lock`]. Read into `Problem::pinned` by the caller that
        /// opened the timetable, where `activity-fixed` judges it — ADR 0045.
        is_locked: bool,
        /// Whether a human placed it, rather than a solve.
        ///
        /// Written `1` by [`Timetables::place`] and [`Timetables::move_entry`], and untouched by
        /// a lock: locking where the solver put a lesson does not make it hand-placed.
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
) -> Result<Vec<(Uuid, EntryPlacement)>> {
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
            EntryPlacement {
                timeslot_id,
                room_id,
            },
        ));
    }
    // By activity identifier, so two saves of one arrangement write the rows in one order.
    rows.sort_by_key(|(activity, _)| *activity);
    Ok(rows)
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

/// Writes one `timetable_entry`, returning the identifier it was given.
///
/// The one insert for all three writers — a save, a manual placement and a duplicate — so the
/// column list exists once and a new column cannot be forgotten by one of them. The flags are a
/// parameter rather than literals because those are exactly what the three disagree about: a
/// save writes `0, 0`, a manual placement writes `is_manual = 1`, and a duplicate preserves
/// whatever the row it is copying held.
fn insert_entry(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    activity_id: Uuid,
    placement: &EntryPlacement,
    flags: EntryFlags,
) -> Result<Uuid> {
    let stamp = Stamp::new();
    tx.execute(
        "INSERT INTO timetable_entry (
             id, timetable_id, activity_id, timeslot_id, room_id,
             is_locked, is_manual, created_at, updated_at, rev)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            stamp.id.to_string(),
            timetable_id.to_string(),
            activity_id.to_string(),
            placement.timeslot_id.to_string(),
            placement.room_id.map(|id| id.to_string()),
            i64::from(flags.is_locked),
            i64::from(flags.is_manual),
            stamp.created_at,
            stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|e| translate(e, "timetable"))?;
    Ok(stamp.id)
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

// =======================================================================================
// Manual editing — M9A
// =======================================================================================

/// What an editing caller believes a timetable's whole arrangement is.
///
/// # The guard `expectedRev` cannot be
///
/// A revision protects **one row**. An edit session holds much more than one: a compiled
/// `Problem`, the whole arrangement, and a validation the user has already been shown. A second
/// window that moves a *different* entry of the same timetable leaves this session's target row
/// untouched — so its `expectedRev` still matches — and yet every verdict the session has
/// computed is now measured against an arrangement that no longer exists. The user would be told
/// a cell is free and would drop a lesson into a clash.
///
/// Nothing else would catch it. Every stored reference still resolves, `UNIQUE
/// (timetable_id, activity_id)` is still satisfied, the input watermark has not moved because
/// `timetable_entry` is a solve **output** (`SOLVE_OUTPUT_TABLES`), and the timetable row's own
/// revision does not move when an entry changes.
///
/// So an editing write carries this as well: every entry of the timetable with the revision the
/// caller last saw it at. It is compared **inside the writing transaction** and the write is
/// refused [`DbError::ArrangementChanged`] if anything differs — ADR 0030's rule, which
/// `CLAUDE.md` §3 restates, applied to an arrangement rather than to an import plan.
///
/// # Exact, rather than a digest
///
/// An `(id, rev)` pair per entry rather than a checksum, for two reasons. A count-and-sum digest
/// collides on a real sequence — unscheduling an entry at `rev = 1` and placing a new one gives
/// back the same count and the same total — and an exact list can say **what** changed, which is
/// what lets the interface tell a user that another window moved three lessons rather than that
/// something happened. At the 2 000-entry upper end the list is a few tens of kilobytes and the
/// read is one indexed scan.
///
/// A scoped change-log watermark was considered and rejected: `change_log` records
/// `(entity_table, entity_id)`, so a scoped query can find changes to entries that still exist
/// and cannot find the **deletion** of one it has never heard of — an unschedule in another
/// window would pass unnoticed, which is the case that matters most.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArrangementStamp {
    /// Every entry of the timetable, as `(id, rev)`, sorted by identifier.
    entries: Vec<(Uuid, i64)>,
}

impl ArrangementStamp {
    /// How many entries the arrangement held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the timetable held no placed occurrence at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries differ between two stamps: changed, added or removed.
    ///
    /// A count rather than a verdict, so a message can say how much moved. Zero means the two
    /// describe the same arrangement.
    #[must_use]
    pub fn differences(&self, other: &Self) -> usize {
        let mut changed = 0;
        for (id, rev) in &self.entries {
            match other.entries.binary_search_by_key(id, |(other, _)| *other) {
                Ok(at) if other.entries[at].1 == *rev => {}
                _ => changed += 1,
            }
        }
        for (id, _) in &other.entries {
            if self
                .entries
                .binary_search_by_key(id, |(mine, _)| *mine)
                .is_err()
            {
                changed += 1;
            }
        }
        changed
    }
}

/// Where one occurrence is placed, as stored identifiers.
///
/// The **start** slot and the room, which is the whole of a placement: rows 2…`d` of a
/// multi-period occurrence are derived by `evara_core::scheduling::span`, so moving a double
/// period is one `UPDATE` of one column and a half-moved double is unrepresentable
/// (ADR 0042 D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPlacement {
    /// The slot the occurrence starts in.
    pub timeslot_id: Uuid,
    /// The room booked, where one is.
    pub room_id: Option<Uuid>,
}

/// The two guards every editing write carries.
///
/// Grouped because they are one question asked of two different things — *has the world moved
/// under this session?* — and a caller holding them apart could pass one timetable's stamp with
/// another problem's watermark.
#[derive(Debug, Clone, Copy)]
pub struct EditGuard<'a> {
    /// Where the project's history stood when the session compiled its problem.
    ///
    /// M7's freshness check, unchanged: a scheduling input that moved makes every verdict the
    /// session computed untrue, and nothing referential would catch it.
    pub watermark: Watermark,
    /// What the session believes the arrangement is. See [`ArrangementStamp`].
    pub stamp: &'a ArrangementStamp,
}

/// What one applied edit wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEdit {
    /// The row as it now stands, or `None` for an unschedule.
    pub entry: Option<TimetableEntry>,
    /// The arrangement as it now stands, for the session to hold instead of its old one.
    ///
    /// Returned rather than left to the caller to re-read: a session that had to ask again
    /// would have a window between its own write and its new stamp, which is the race this
    /// type exists to close.
    pub stamp: ArrangementStamp,
}

/// A timetable copied, with its arrangement and without its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicatedTimetable {
    /// The new timetable row.
    pub timetable: Timetable,
    /// How many entries were copied.
    pub entries: usize,
}

impl Timetables<'_> {
    /// Every entry of one timetable with the revision it is at.
    ///
    /// What an edit session reads once, at the start, and compares on every write.
    pub fn arrangement_stamp(&self, timetable_id: Uuid) -> Result<ArrangementStamp> {
        self.db.read(|conn| read_stamp(conn, timetable_id))
    }

    /// Places an occurrence that is not placed in this timetable.
    ///
    /// # A creation, so no revision precondition
    ///
    /// There is no row to be stale about. `UNIQUE (timetable_id, activity_id)` is what refuses a
    /// second placement of the same occurrence, reported as [`DbError::DuplicateValue`] — which
    /// is also what a second click on the same empty cell receives, rather than two rows.
    ///
    /// `is_manual` is written `1`: a human put it there, and a later screen that said a solve
    /// had would be wrong.
    ///
    /// # Errors
    ///
    /// - [`DbError::InputsChanged`] if a scheduling input moved since the session compiled.
    /// - [`DbError::ArrangementChanged`] if another window changed this timetable's entries.
    /// - [`DbError::WrongSchool`] if the occurrence belongs to another school.
    /// - [`DbError::DuplicateValue`] if the occurrence is already placed in this timetable.
    /// - [`DbError::Invalid`] if the slot belongs to a different cycle than the timetable.
    pub fn place(
        &mut self,
        timetable_id: Uuid,
        activity_id: Uuid,
        at: EntryPlacement,
        guard: EditGuard<'_>,
    ) -> Result<AppliedEdit> {
        let EditGuard { watermark, stamp } = guard;
        let stamp = stamp.clone();
        self.db.write(|tx| {
            guard_edit(tx, timetable_id, watermark, &stamp)?;
            require_activity_of_timetable(tx, timetable_id, activity_id)?;
            require_slot_of_timetable(tx, timetable_id, at.timeslot_id)?;

            let id = insert_entry(
                tx,
                timetable_id,
                activity_id,
                &at,
                EntryFlags {
                    is_locked: false,
                    is_manual: true,
                },
            )?;
            applied(tx, timetable_id, Some(id))
        })
    }

    /// Moves a placed occurrence to another slot, another room, or both.
    ///
    /// # An `UPDATE`, never a delete and an insert
    ///
    /// `UNIQUE (timetable_id, activity_id)` makes the row the identity of *"this occurrence's
    /// placement in this timetable"*. Removing it and writing another would mint a new `id`,
    /// reset `rev` to 1, append two change-log entries for one user action, and drop
    /// `is_locked` as a side effect of moving. One `UPDATE` through
    /// [`update`](super::update) carries the revision predicate, bumps `rev` once and appends
    /// one entry.
    ///
    /// # Errors
    ///
    /// As [`Timetables::place`], plus [`DbError::Conflict`] if the entry moved since
    /// `expected`. A **locked** entry is refused by the command layer before this is called —
    /// see ADR 0045 §5 — and by `activity-fixed` if anything else ever tries.
    pub fn move_entry(
        &mut self,
        entry_id: Uuid,
        expected: i64,
        to: EntryPlacement,
        guard: EditGuard<'_>,
    ) -> Result<AppliedEdit> {
        let EditGuard { watermark, stamp } = guard;
        let stamp = stamp.clone();
        self.db.write(|tx| {
            let timetable_id = timetable_of_entry(tx, entry_id)?;
            guard_edit(tx, timetable_id, watermark, &stamp)?;
            require_slot_of_timetable(tx, timetable_id, to.timeslot_id)?;

            update(
                tx,
                "timetableEntry",
                "timetable_entry",
                entry_id,
                Expected::Rev(expected),
                "timeslot_id = ?, room_id = ?, is_manual = 1",
                &[
                    &to.timeslot_id.to_string(),
                    &to.room_id.map(|id| id.to_string()),
                ],
            )?;
            applied(tx, timetable_id, Some(entry_id))
        })
    }

    /// Takes a placed occurrence off the timetable.
    ///
    /// The occurrence returns to unplaced, where `activity-scheduled` reports it at MEDIUM. That
    /// is always permitted and always costs: a rule that refused it would make the obvious first
    /// step of a manual rearrangement impossible.
    ///
    /// # Errors
    ///
    /// As [`Timetables::move_entry`].
    pub fn unschedule(
        &mut self,
        entry_id: Uuid,
        expected: i64,
        guard: EditGuard<'_>,
    ) -> Result<AppliedEdit> {
        let EditGuard { watermark, stamp } = guard;
        let stamp = stamp.clone();
        self.db.write(|tx| {
            let timetable_id = timetable_of_entry(tx, entry_id)?;
            guard_edit(tx, timetable_id, watermark, &stamp)?;

            delete(
                tx,
                "timetableEntry",
                "timetable_entry",
                entry_id,
                Expected::Rev(expected),
            )?;
            applied(tx, timetable_id, None)
        })
    }

    /// Locks or unlocks one placement.
    ///
    /// # A lock is about a placement, not about a demand
    ///
    /// Hence the column on the entry: *"in **this** timetable, this placement is fixed"*. Two
    /// timetables of one cycle lock independently, which is what lets a user fix a lesson in the
    /// version they are publishing while leaving it free in the draft beside it. ADR 0045.
    ///
    /// `is_manual` is **not** touched. Locking is not placing, and a screen that said a lesson
    /// had been put there by hand because somebody locked where the solver put it would be
    /// wrong.
    ///
    /// # Errors
    ///
    /// As [`Timetables::move_entry`].
    pub fn set_lock(
        &mut self,
        entry_id: Uuid,
        expected: i64,
        locked: bool,
        guard: EditGuard<'_>,
    ) -> Result<AppliedEdit> {
        let EditGuard { watermark, stamp } = guard;
        let stamp = stamp.clone();
        self.db.write(|tx| {
            let timetable_id = timetable_of_entry(tx, entry_id)?;
            guard_edit(tx, timetable_id, watermark, &stamp)?;

            update(
                tx,
                "timetableEntry",
                "timetable_entry",
                entry_id,
                Expected::Rev(expected),
                "is_locked = ?",
                &[&i64::from(locked)],
            )?;
            applied(tx, timetable_id, Some(entry_id))
        })
    }

    /// Copies one timetable and its arrangement under a new name.
    ///
    /// # Why this exists, and what it deliberately does not copy
    ///
    /// Editing is **in place**: `is_manual` is a per-entry flag on the same timetable, and
    /// ADR 0042 §5 keeps the `GENERATION` scoring immutable precisely so that re-scoring can
    /// never misrepresent how a timetable was generated — a guarantee that is only needed
    /// *because* the arrangement beneath it can change. So a user who wants to keep the
    /// generated arrangement duplicates first, and this is that action.
    ///
    /// The copy carries **no `solve_run` and no evaluation of any kind**. It was not generated,
    /// and a `GENERATION` row cloned onto it would be a claim about a run that did not produce
    /// it. Re-scoring the copy records a `REVALIDATION`, which is the honest shape: *this is
    /// what the rules find about these placements now*.
    ///
    /// The entries are copied with fresh identifiers at `rev = 1` and with `is_locked` and
    /// `is_manual` **preserved** — those describe the placements, which is what was copied.
    ///
    /// # Errors
    ///
    /// - [`DbError::Conflict`] if the source moved since `expected`.
    /// - [`DbError::NotFound`] if there is no such timetable.
    /// - [`DbError::DuplicateValue`] if the scope already holds that name.
    pub fn duplicate(
        &mut self,
        id: Uuid,
        expected: i64,
        name: &str,
    ) -> Result<DuplicatedTimetable> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(DbError::Invalid {
                message: "a timetable needs a name".to_owned(),
            });
        }
        self.db.write(|tx| {
            // The source's revision is the precondition, and it is consumed: `updated_at`
            // moves on a row nothing else about has changed, which is the honest record that
            // something was done with it. ADR 0027 §3 category 3.
            update(
                tx,
                "timetable",
                "timetable",
                id,
                Expected::Rev(expected),
                "name = name",
                &[],
            )?;
            let source = read_timetable(tx, id)?.ok_or(DbError::NotFound {
                entity: "timetable",
            })?;

            let stamp = Stamp::new();
            tx.execute(
                "INSERT INTO timetable (
                     id, cycle_id, term_id, name, generated_at,
                     indeterminate_student_pairs, unchecked_capacities, shared_entities,
                     unenforced_restrictions, created_at, updated_at, rev)
                 SELECT ?1, cycle_id, term_id, ?2, generated_at,
                        indeterminate_student_pairs, unchecked_capacities, shared_entities,
                        unenforced_restrictions, ?3, ?4, ?5
                 FROM timetable WHERE id = ?6",
                rusqlite::params![
                    stamp.id.to_string(),
                    name,
                    stamp.created_at,
                    stamp.updated_at,
                    stamp.rev,
                    source.id.to_string(),
                ],
            )
            .map_err(|e| translate(e, "timetable"))?;

            let mut copied = 0;
            for entry in read_entries(tx, id)? {
                insert_entry(
                    tx,
                    stamp.id,
                    entry.activity_id,
                    &EntryPlacement {
                        timeslot_id: entry.timeslot_id,
                        room_id: entry.room_id,
                    },
                    EntryFlags {
                        is_locked: entry.is_locked,
                        is_manual: entry.is_manual,
                    },
                )?;
                copied += 1;
            }

            let timetable = read_timetable(tx, stamp.id)?.ok_or(DbError::NotFound {
                entity: "timetable",
            })?;
            Ok(DuplicatedTimetable {
                timetable,
                entries: copied,
            })
        })
    }
}

/// The stored flags one entry is written with.
///
/// A struct rather than two `bool` arguments, because two adjacent booleans at a call site are
/// the shape a reader cannot check and a writer can transpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntryFlags {
    is_locked: bool,
    is_manual: bool,
}

/// Both guards, in the transaction that writes.
///
/// Order matters for the message, not for correctness: a moved *input* makes the whole session
/// stale and is reported first, because reopening the timetable is the remedy for both and the
/// input case is the one that cannot be fixed by looking at the grid again.
fn guard_edit(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    watermark: Watermark,
    stamp: &ArrangementStamp,
) -> Result<()> {
    guard_freshness(tx, watermark)?;
    let current = read_stamp(tx, timetable_id)?;
    let changed = stamp.differences(&current);
    if changed > 0 {
        return Err(DbError::ArrangementChanged {
            entries_changed: i64::try_from(changed).unwrap_or(i64::MAX),
        });
    }
    Ok(())
}

/// Reads one timetable's `(entry id, rev)` pairs, sorted by identifier.
fn read_stamp(conn: &Connection, timetable_id: Uuid) -> Result<ArrangementStamp> {
    let mut stmt =
        conn.prepare("SELECT id, rev FROM timetable_entry WHERE timetable_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([timetable_id.to_string()], |row| {
        Ok((FromColumn::read(row, 0)?, row.get::<_, i64>(1)?))
    })?;
    let mut entries = rows
        .collect::<rusqlite::Result<Vec<(Uuid, i64)>>>()
        .map_err(|e| translate(e, "timetable"))?;
    // `ORDER BY id` is over the stored text; `differences` binary-searches on `Uuid`, so the
    // list is sorted again by the key it will be searched with rather than by the one SQLite
    // happened to use.
    entries.sort_unstable_by_key(|(id, _)| *id);
    Ok(ArrangementStamp { entries })
}

/// The timetable one entry belongs to.
fn timetable_of_entry(tx: &Transaction<'_>, entry_id: Uuid) -> Result<Uuid> {
    tx.query_row(
        "SELECT timetable_id FROM timetable_entry WHERE id = ?1",
        [entry_id.to_string()],
        |row| FromColumn::read(row, 0),
    )
    .optional()
    .map_err(|e| translate(e, "timetableEntry"))?
    .ok_or(DbError::NotFound {
        entity: "timetableEntry",
    })
}

/// Refuses a slot that is not in the timetable's own grid.
///
/// # The trigger is the backstop and this is the message
///
/// `V17`'s `timetable_entry_slot_in_cycle` trigger pair already refuses this, in the same
/// transaction, and it is what makes the rule true for every writer. But a `RAISE(ABORT)`
/// reaches a caller as an unclassified constraint failure, which the interface reports as
/// *"this campus could not be updated; it may be damaged"* — the wrong sentence for a boundary
/// working correctly. So the repository asks first and names the remedy, which is the same
/// division of labour the membership triggers and `assign_teacher` already use (`CLAUDE.md` §3).
fn require_slot_of_timetable(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    timeslot_id: Uuid,
) -> Result<()> {
    let same: Option<bool> = tx
        .query_row(
            "SELECT ts.cycle_id = t.cycle_id
             FROM timetable t, timeslot ts
             WHERE t.id = ?1 AND ts.id = ?2",
            [timetable_id.to_string(), timeslot_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match same {
        Some(true) => Ok(()),
        Some(false) => Err(DbError::Invalid {
            message: "that period belongs to a different cycle than this timetable, so no                       lesson can be placed there; open a timetable of that cycle instead"
                .to_owned(),
        }),
        // One of the two rows is not there. Which one matters to the message, and the
        // identifier the caller supplied is the likelier of the two.
        None => Err(DbError::NotFound {
            entity: "timeslot",
        }),
    }
}

/// Refuses an occurrence that is not the timetable's school's.
///
/// The chain is `activity -> class_section -> course -> subject.school_id`, and the timetable's
/// is `cycle.school_id`. Neither carries a second `school_id` column and neither will:
/// `CLAUDE.md` §3's rule is that a child's school is resolved through its parents.
///
/// `V17`'s trigger already refuses a slot of another cycle, so a timeslot needs no equivalent
/// check here. A **room** gets none deliberately: `room_campus` reaches across schools by
/// design, so a shared room is a legitimate booking for either.
fn require_activity_of_timetable(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    activity_id: Uuid,
) -> Result<()> {
    let school: Option<String> = tx
        .query_row(
            "SELECT c.school_id FROM timetable t
             JOIN cycle c ON c.id = t.cycle_id
             WHERE t.id = ?1",
            [timetable_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(school) = school else {
        return Err(DbError::NotFound {
            entity: "timetable",
        });
    };
    let school = Uuid::parse_str(&school).map_err(|_| DbError::CorruptIdentifier {
        column: "cycle.school_id".to_owned(),
        value: school.clone(),
    })?;

    require_school_owner(
        tx,
        "activity",
        "SELECT su.school_id FROM activity a
         JOIN class_section cs ON cs.id = a.class_section_id
         JOIN course       co ON co.id = cs.course_id
         JOIN subject      su ON su.id = co.subject_id
         WHERE a.id = ?",
        activity_id,
        school,
    )
}

/// The result of one applied edit: the row, where there still is one, and the new stamp.
fn applied(
    tx: &Transaction<'_>,
    timetable_id: Uuid,
    entry_id: Option<Uuid>,
) -> Result<AppliedEdit> {
    let entry = match entry_id {
        None => None,
        Some(id) => Some(read_entry(tx, id)?.ok_or(DbError::NotFound {
            entity: "timetableEntry",
        })?),
    };
    Ok(AppliedEdit {
        entry,
        stamp: read_stamp(tx, timetable_id)?,
    })
}

/// One entry by identifier.
fn read_entry(conn: &Connection, id: Uuid) -> Result<Option<TimetableEntry>> {
    Ok(conn
        .query_row(
            "SELECT id, timetable_id, activity_id, timeslot_id, room_id,
                    is_locked, is_manual, created_at, updated_at, rev
             FROM timetable_entry WHERE id = ?1",
            [id.to_string()],
            TimetableEntry::from_row,
        )
        .optional()?)
}

/// Every entry of one timetable, in activity order.
fn read_entries(conn: &Connection, timetable_id: Uuid) -> Result<Vec<TimetableEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, timetable_id, activity_id, timeslot_id, room_id,
                is_locked, is_manual, created_at, updated_at, rev
         FROM timetable_entry WHERE timetable_id = ?1 ORDER BY activity_id",
    )?;
    let rows = stmt.query_map([timetable_id.to_string()], TimetableEntry::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| translate(e, "timetable"))
}

/// The locked placements of one timetable, as `Problem::pinned` wants them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StoredPins {
    /// One entry per activity index, or **empty** when nothing is locked.
    ///
    /// Empty rather than all-`None` on purpose. `Problem::integrity` accepts either, and the
    /// empty list is what every other caller produces — a generation, a fixture — so a problem
    /// with no locks is byte-identical whether it was opened for editing or not.
    pub pinned: Vec<Option<Placement>>,
    /// How many entries were locked.
    pub locked: usize,
    /// Locked entries naming something the problem no longer holds.
    ///
    /// A lock on a lesson that has since been deleted, or in a slot the grid no longer
    /// materialises. Reported rather than refused, for the same reason
    /// [`StoredArrangement::not_in_problem`] is: a timetable whose inputs moved is still worth
    /// opening, and a lock that cannot be expressed as a pin is a thing to tell the user about.
    pub not_in_problem: Vec<Uuid>,
}

impl Timetables<'_> {
    /// The locked placements of one timetable, translated into dense indices.
    ///
    /// # Why the caller assigns this and the loader does not
    ///
    /// `Scheduling::load` compiles a school's **inputs**. A lock is a `timetable_entry` column,
    /// which is an *output* of a solve — it is in `SOLVE_OUTPUT_TABLES` for exactly that reason.
    /// A loader that read one would make the compiled `Problem` depend on which timetable
    /// happened to be open, and would put output rows under the `Assembly`'s watermark, which
    /// the freshness check deliberately excludes. So this is a second read, and the caller
    /// writes the result onto `problem.pinned`. ADR 0045 §4.
    ///
    /// # Errors
    ///
    /// Any SQLite failure, and [`DbError::CorruptIdentifier`] from reading the rows.
    pub fn pins(&self, timetable_id: Uuid, assembly: &Assembly) -> Result<StoredPins> {
        let entries = self.entries(timetable_id)?;
        let mut pinned: Vec<Option<Placement>> = Vec::new();
        let mut locked = 0;
        let mut not_in_problem = Vec::new();

        for entry in entries.iter().filter(|entry| entry.is_locked) {
            locked += 1;
            let (Some(activity), Some(timeslot)) = (
                assembly.index.activity_index(entry.activity_id),
                assembly.index.timeslot_index(entry.timeslot_id),
            ) else {
                not_in_problem.push(entry.activity_id);
                continue;
            };
            let room = entry
                .room_id
                .and_then(|room| assembly.index.room_index(room));
            if entry.room_id.is_some() && room.is_none() {
                not_in_problem.push(entry.activity_id);
                continue;
            }
            if pinned.is_empty() {
                pinned = vec![None; assembly.problem.activities.len()];
            }
            if let Some(slot) = pinned.get_mut(activity.get()) {
                *slot = Some(Placement { timeslot, room });
            }
        }

        Ok(StoredPins {
            pinned,
            locked,
            not_in_problem,
        })
    }
}
