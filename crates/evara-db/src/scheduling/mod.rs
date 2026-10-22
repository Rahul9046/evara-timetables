//! Compiling one school's stored rows into a scheduling [`Problem`].
//!
//! ```text
//! SQLite ──► Problem ──► integrity ──► validation ──► constraints ──► solver ──► solution
//!   M4         M2            M2            M3             M5           M6         M7
//! ```
//!
//! # Why the loader is here
//!
//! `ARCHITECTURE.md` §3 rule 4: `evara-db` is the only crate that knows SQL, and *"the
//! translation from UUIDs to the solver's dense indexes happens here, once, and no other
//! crate acquires a database handle to avoid it."* `evara-core` owns the vocabulary and
//! opens nothing; `evara-constraints` and `evara-solver` are handed a problem already
//! materialised. This module is the one place the two worlds meet.
//!
//! # What it does not do
//!
//! It does not solve, score, evaluate a constraint, write a timetable, or read a weekday.
//! It performs **no writes at all** — see [`Scheduling`], whose receiver is a shared
//! reference for exactly that reason.
//!
//! # The three things it is asked for, kept apart
//!
//! | Category | Channel | Example |
//! | --- | --- | --- |
//! | **The request cannot be served** | `Err(DbError)` | no such school; a cycle belonging to another school; SQLite failed |
//! | **A stored row is wrong** | a loader [`Diagnostic`] | an availability rule naming a deleted teacher; a teacher ineligible at their section's campus |
//! | **The school's inputs are incomplete** | M3's [`validate`] | no grid; an ambiguous bell schedule; an occurrence longer than any run |
//!
//! A fatal failure is never dressed as a scheduling diagnostic, and a scheduling input
//! problem never crashes the loader. The middle row is [ADR
//! 0037](../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md)'s split, and
//! both halves use one vocabulary.
//!
//! # Determinism
//!
//! Every query carries an explicit `ORDER BY` over columns a unique index makes total;
//! nothing reads `rowid`; the identifier maps are [`BTreeMap`](std::collections::BTreeMap)s.
//! Two loads of one snapshot produce equal problems and equal reports, which a test asserts
//! and which `SOLVER.md` §5 calls non-negotiable — the same seed over a different index
//! assignment is a different timetable.
//!
//! # Queries
//!
//! Twenty-eight statements per load, thirty when a term is named, **independent of school
//! size**. Every relationship is read with one bulk query and resolved in memory against the
//! identifier maps; nothing asks a question per row. The per-stage breakdown is in
//! [`SCHEDULING-PROBLEM-ASSEMBLY.md`](../../../docs/SCHEDULING-PROBLEM-ASSEMBLY.md) §18.

mod context;
mod facilities;
mod findings;
pub(crate) mod index;
mod lessons;
mod population;
mod report;
mod rules;

use evara_core::scheduling::{
    AvailabilityRule, Diagnostic, IntegrityError, MaterialisedGrid, OverlapRelation, Problem,
};
use rusqlite::Connection;
use uuid::Uuid;

use crate::Database;
use crate::error::{DbError, Result};

pub use findings::PRODUCED as LOADER_DIAGNOSTICS;
pub use index::{ProblemIndex, ProblemIndexCounts};
pub use report::AssemblyReport;

/// The term predicate every in-scope read of the lesson chain carries.
///
/// `?2` is the selected term. A course with no term runs all year and is always in scope; a
/// course with a term is in scope when it is the selected one; and when no term is selected
/// every course is in scope. See [`lessons`]'s header for why that last branch is a choice
/// and not an oversight.
///
/// Interpolated into a format string rather than written out five times, so the five reads
/// of the chain cannot come to disagree about what the selection means. Every query aliases
/// `course` as `co`.
const IN_SCOPE_TERM: &str = "(?2 IS NULL OR co.term_id IS NULL OR co.term_id = ?2)";

/// Which school, cycle and term a load is about.
///
/// All three are named by the caller. None is defaulted, and the absence of a default is the
/// decision rather than an omission:
/// [ADR 0032](../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md) rule 1
/// scopes generation to one `(school, cycle, term)`, a project has held several schools since
/// `V12` so there is no "the school" to fall back on, and a solve that picked a cycle would be
/// building a timetable over a grid the user did not choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingSelection {
    /// The school being solved. Every school-owned row in the problem belongs to it.
    pub school_id: Uuid,
    /// The cycle the grid is defined over.
    pub cycle_id: Uuid,
    /// The term, or `None` for a grid materialised for the whole year.
    ///
    /// `None` is an ordinary selection and not a missing value: `timeslot.term_id` is
    /// nullable, and a school may hold one year-long grid.
    pub term_id: Option<Uuid>,
}

/// Whether every stored restriction in scope reached the compiled problem.
///
/// Exists so that an incomplete problem cannot be mistaken for a complete one. A caller that
/// ignores this still cannot be misled about *feasibility* — that is
/// [`AssemblyReport::may_solve`]'s job — but it could be misled about how much was checked,
/// and a timetable described as clean when a stored rule was never enforced is the failure
/// `V11`'s store-only-exceptions design exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Completeness {
    /// Every row in scope was compiled, and every stored restriction is being enforced.
    Complete,
    /// Some stored restrictions are recorded and are not enforced by this engine.
    ///
    /// Availability rules naming an entity kind nothing schedules — a `RESOURCE` or a
    /// `SUBJECT`. Each is reported individually as `availability-entity-kind-not-scheduled`,
    /// and the count is here so a caller can say "N restrictions were not applied" without
    /// filtering a report.
    ///
    /// An orphaned rule does **not** reach this variant: its entity is gone, so there is
    /// nothing it could have restricted and nothing is unenforced by dropping it.
    RestrictionsNotEnforced {
        /// How many stored rules are not being enforced.
        rules: usize,
    },
}

impl Completeness {
    /// Whether every stored restriction in scope is being enforced.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// What one load found that is worth counting but is not a finding.
///
/// Counted rather than reported, because each is an ordinary state with no remedy the user
/// owns — but each is also a thing a careful reader would want to know, and a number nobody
/// can see is a number nobody can check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyStats {
    /// Availability rules compiled onto at least one slot of this grid.
    pub rules_compiled: usize,
    /// Rules naming an entity that is no longer in the project.
    pub rules_orphaned: usize,
    /// Rules naming an entity kind this engine does not schedule.
    pub rules_not_scheduled: usize,
    /// Rules about something real that this solve does not schedule.
    ///
    /// A teacher whose membership was deactivated and who teaches nothing in scope; a cycle
    /// day of another cycle; a period of a schedule this grid was not built from; a rule
    /// naming a different term. Each restricts nothing that can be placed here.
    pub rules_out_of_scope: usize,
    /// Teachers in scope with no active membership at this school.
    ///
    /// Not an error and not corruption. `V15`'s `class_teacher_requires_active_membership`
    /// pair fires only on `class_teacher`, because
    /// [ADR 0018](../../../docs/adr/0018-people-identity-and-deactivation.md) requires a
    /// deactivated teacher to keep every assignment they already had and merely stop being
    /// offered for new ones. So a section legitimately in scope may be taught by one, and
    /// they are loaded — see [`population`]'s header for why loading only the employed set
    /// would produce a dangling index.
    pub teachers_without_active_membership: usize,
}

/// One school's compiled scheduling problem, and everything the loader learned making it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembly {
    /// What was asked for, echoed back so a report can say which grid it is about.
    pub selection: SchedulingSelection,
    /// The compiled problem.
    pub problem: Problem,
    /// Every identifier the problem holds, and the index each was given.
    pub index: ProblemIndex,
    /// The availability rules that were compiled, as stored.
    ///
    /// Beside the problem rather than inside it: ADR 0037 §2 refused to widen `Problem` with
    /// a field whose only consumer is a diagnostic, and the same argument applies to a field
    /// whose only consumer is a report. A rule that could **not** be compiled is absent from
    /// this list and present in [`Assembly::findings`].
    pub rules: Vec<AvailabilityRule>,
    /// The loader's own findings, most severe first.
    pub findings: Vec<Diagnostic>,
    /// Whether every stored restriction in scope is being enforced.
    pub completeness: Completeness,
    /// What was worth counting.
    pub stats: AssemblyStats,
}

impl Assembly {
    /// Every way the compiled problem refers to something it does not contain.
    ///
    /// Empty for a sound load. A non-empty result is a **loader bug** and not a data state —
    /// `IntegrityError`'s own documentation says so — which is why it is checked here rather
    /// than left for a caller to remember.
    #[must_use]
    pub fn integrity(&self) -> Vec<IntegrityError> {
        self.problem.integrity()
    }
}

/// Reads a school's scheduling inputs. **Writes nothing.**
///
/// # The read-only guarantee is in the type
///
/// Every other repository handle takes `&mut Database`, because every other repository can
/// write. This one takes `&Database`, so it cannot reach
/// [`Database::write`](crate::Database) — which needs `&mut self` — and therefore cannot
/// open a transaction, consume a revision or append a change-log entry. The guarantee is a
/// borrow-checker property rather than a convention somebody must keep, which is what
/// `evara validate --project` needs in order to be safe to point at a real school's file.
///
/// Opening the project is a separate matter: [`Database::open_existing`] runs migrations, so
/// a genuinely read-only inspection path goes through
/// [`Database::inspect`](crate::Database::inspect) first and refuses a project that would be
/// upgraded. See the CLI for how that is assembled.
#[derive(Debug)]
pub struct Scheduling<'a> {
    db: &'a Database,
}

impl<'a> Scheduling<'a> {
    /// Wraps an open project.
    pub(crate) const fn new(db: &'a Database) -> Self {
        Self { db }
    }
}

impl Scheduling<'_> {
    /// Compiles one school's rows into a [`Problem`].
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if the school, cycle or term is not in the project.
    /// - [`DbError::WrongSchool`] if the cycle, or the term's academic year, belongs to
    ///   another school. Checked before anything else is read, because a request that mixes
    ///   two schools cannot be served by editing either one's data.
    /// - [`DbError::CorruptIdentifier`] if a stored identifier, enumeration value or bell
    ///   time cannot be read. A persisted value that cannot be true is corruption and not
    ///   input: substituting a default would make a damaged file look like a working one, and
    ///   a lesson placed against a guessed bell time is the wrong answer this product exists
    ///   to avoid.
    /// - Any SQLite failure, as [`DbError::Internal`].
    pub fn load(&self, selection: &SchedulingSelection) -> Result<Assembly> {
        self.db.read(|conn| assemble(conn, selection))
    }

    /// Compiles the problem and reports everything that is wrong with its inputs.
    ///
    /// The orchestration `PRE-SOLVE-VALIDATION.md` §15 sketches, in one call: load, check
    /// integrity, validate, and combine. A caller that wants the problem itself as well
    /// should use [`Scheduling::load`] and [`AssemblyReport::of`], which is what this does.
    ///
    /// # Errors
    ///
    /// As [`Scheduling::load`].
    pub fn validate(&self, selection: &SchedulingSelection) -> Result<AssemblyReport> {
        Ok(AssemblyReport::of(&self.load(selection)?))
    }
}

/// Compiles one school's rows, in the order the references between them require.
///
/// The ordering is not arbitrary and each step needs the one before it:
///
/// 1. **Context and grid.** Everything else resolves campuses, days and periods against it.
/// 2. **Facilities.** A section's room candidates need every room first.
/// 3. **Section rows.** Interned before staffing, because a staffing row names a section.
/// 4. **Teachers.** The set is the active memberships *plus* anyone an in-scope section
///    names, so step 3 has to have happened.
/// 5. **Groups**, then staffing and audience, which name both.
/// 6. **Activities**, which name sections.
/// 7. **Section facts**, last, because a bell schedule is resolved against the grid and the
///    masks are built from the overlap relation — `PRE-SOLVE-VALIDATION.md` §9 describes
///    exactly this shape, and it is what lets the resolver be called rather than copied.
/// 8. **Availability**, which names every other table and the slots.
fn assemble(conn: &Connection, selection: &SchedulingSelection) -> Result<Assembly> {
    let mut index = ProblemIndex::default();

    let (context, timeslots) = context::load(conn, selection, &mut index)?;
    let room_types = facilities::room_types(conn, selection, &mut index)?;
    let rooms = facilities::rooms(conn, selection, &mut index)?;

    let section_rows = lessons::section_rows(conn, selection, &mut index)?;
    let teachers = population::teachers(conn, selection, &mut index)?;
    let groups = population::groups(conn, selection, &mut index)?;
    let staffing = lessons::staffing(conn, selection, &index)?;
    let audience = lessons::audience(conn, selection, &index)?;
    let activities = lessons::activities(conn, selection, &mut index)?;

    let overlap = overlap(groups.len());
    let compiled = rules::load(conn, selection, &index, &timeslots)?;

    // Assembled with no sections so that `MaterialisedGrid::of` can derive `G` from the
    // slots. The grid reads only `Problem::timeslots`, and resolution reads only the
    // context and a section's campus, so the sections can be finished against both without
    // anything being resolved twice or recorded before it is known.
    let mut problem = Problem {
        context,
        timeslots,
        room_types,
        rooms,
        teachers: teachers.facts,
        groups,
        sections: Vec::new(),
        activities,
        overlap,
        unavailable: compiled.index,
        // `activity.is_locked` is not in `V15` — the roadmap keeps lock toggles with Phase
        // 8's manual editing, because a lock is about a *placed* timetable rather than about
        // the demand — so there is nothing to populate this from and an empty list is the
        // honest value. `Problem::integrity` accepts empty or one-per-activity and nothing
        // else.
        pinned: Vec::new(),
    };
    let grid = MaterialisedGrid::of(&problem);
    problem.sections = lessons::sections(
        section_rows,
        &problem.context,
        &grid,
        &problem.rooms,
        &problem.groups,
        &problem.overlap,
        staffing,
        audience,
    );

    let findings = findings::of(&problem, &compiled.orphans, &compiled.unscheduled);
    let completeness = if compiled.unscheduled.is_empty() {
        Completeness::Complete
    } else {
        Completeness::RestrictionsNotEnforced {
            rules: compiled.unscheduled.len(),
        }
    };

    Ok(Assembly {
        selection: *selection,
        problem,
        index,
        stats: AssemblyStats {
            rules_compiled: compiled.rules.len(),
            rules_orphaned: compiled.orphans.len(),
            rules_not_scheduled: compiled.unscheduled.len(),
            rules_out_of_scope: compiled.out_of_scope,
            teachers_without_active_membership: teachers.without_active_membership,
        },
        rules: compiled.rules,
        findings,
        completeness,
    })
}

/// The three-state overlap answer for every pair of the school's groups.
///
/// # Every pair is `Indeterminate`, and that is this milestone's answer
///
/// [ADR 0035](../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) §3's
/// seven ordered rules are **M5's**, and a second implementation of them in the loader is the
/// duplication `CLAUDE.md` §2 forbids — with the sharpest consequence available, since rule 3
/// before rule 6 is what makes a member row beat a declaration and two evaluators that
/// disagreed would have the product honouring a clash in one place and not the other.
///
/// So M4 assembles the *inputs* §3 reads — the parent edge, the declared size, the effective
/// membership count, and the parent's disjointness declaration — and compiles no answers.
/// [`GroupFact`](evara_core::scheduling::GroupFact)'s own documentation says where the answer
/// lives: *"in `Problem::overlap`, compiled once per solve, because it is a property of a
/// pair and not of a group."* M5 supplies that compilation.
///
/// # What an undecided relation does, and does not, permit
///
/// `clash_mask` admits only [`Overlaps`](evara_core::scheduling::Overlap::Overlaps), so an
/// undecided pair is **not** treated as disjoint and **not** treated as a clash: each
/// section's clash mask is exactly its own audience, which protects every group from being
/// double-booked against itself and claims nothing about any other pair.
/// [ADR 0038](../../../docs/adr/0038-counting-unanswered-questions-is-not-evaluating-overlap.md)
/// §4 states that this is the correct compiled behaviour — *"`clash_mask` already admits only
/// `Overlaps`, so the compiled form enforces exactly what is proved"* — and M3's
/// `group-overlap-indeterminate` is what makes the gap loud rather than silent.
///
/// # The diagonal is left alone
///
/// ADR 0035 rule 2 makes a group against itself `Overlaps`, so the diagonal is a certainty
/// this relation leaves unstated — exactly as M2's fixtures do, which
/// `PRE-SOLVE-VALIDATION.md` §14 records as known limitation 9. Setting it here would be
/// implementing one of the seven rules in the one place that must not, and would make a
/// database-backed problem differ from the synthetic fixture it was seeded from for no gain:
/// `clash_mask` already contains the audience, and `indeterminate_pairs` counts `a < b` only,
/// so the diagonal is read by nothing. It is M5's to declare along with the other six.
fn overlap(groups: usize) -> OverlapRelation {
    OverlapRelation::unknown(groups)
}

/// Resolves a stored reference to a dense index, or reports the project as damaged.
///
/// Used for every foreign key whose target the loader has already put in scope. A reference
/// that does not resolve is corruption: the schema's own foreign keys and the repository's
/// same-school guards are what make the target present, so a miss means the file no longer
/// says what those rules required.
///
/// Reported rather than skipped, because the alternative is worse in a specific way.
/// Dropping an unresolvable campus narrowing would widen a section's bell-schedule
/// resolution; dropping an unresolvable room type would make a laboratory satisfy "needs a
/// gymnasium". A silently reduced problem that validates cleanly is the one outcome this
/// milestone must not produce.
fn require_index<I: index::DenseIndex>(
    table: &index::Interner<I>,
    id: Uuid,
    column: &'static str,
) -> Result<I> {
    table
        .index_of(id)
        .ok_or_else(|| corrupt(column, id.to_string()))
}

/// A stored value the schema should have refused.
///
/// Follows [`parse_id_text`](crate::repo::row::parse_id_text)'s rule, and
/// [`WorkingDays`](evara_core::calendar::WorkingDays)'s precedent for a column that is not an
/// identifier: a persisted value that cannot be true is corruption, not input, and an
/// invented default makes a damaged file look like a working one.
fn corrupt(column: &'static str, value: String) -> DbError {
    DbError::CorruptIdentifier {
        column: column.to_owned(),
        value,
    }
}

#[cfg(test)]
mod tests;
