//! SQLite persistence: migrations, repositories, problem loading and solution writing.
//!
//! # Layering contract
//!
//! The **only** crate that knows SQL. Depends on `evara-core`.
//!
//! Every connection must set `foreign_keys = ON` (SQLite defaults it off, per
//! connection), `journal_mode = WAL` and a `busy_timeout`. This is enforced in one
//! connection factory and covered by a test, never left to call sites.
//!
//! # What lives here (added from Phase 1)
//!
//! - The connection factory and pragma enforcement
//! - Forward-only numbered migrations embedded in the binary
//! - Repositories, one per domain boundary group
//! - The problem loader (database to dense-index `Problem`), in [`scheduling`] since M4
//! - The solution writer, which is M7's and does not exist
//!
//! # Status
//!
//! `V15` holds the entity tables through Stage C's lessons and activities. Since M4 this
//! crate also compiles one school into the solver's [`scheduling::Assembly`]; there is still
//! no solver, no constraint registry and no `timetable_entry`.
//!
//! # Two ways to open a project, and the difference matters
//!
//! [`Database::open_existing`] **writes**: it runs pending migrations, recreates any missing
//! change-log trigger, and stamps `last_opened_with` on every open. That is right for the
//! application. [`Database::open_for_inspection`] does none of the three and returns a
//! connection SQLite refuses every write through, which is what lets `evara validate
//! --project` answer questions about a live school file without touching it.
//!
//! A [`Workspace`] is the normal entry point â€” it owns the installation settings and the
//! one open project, and attributes every write to this installation:
//!
//! ```no_run
//! # fn main() -> Result<(), evara_db::DbError> {
//! let mut workspace = evara_db::Workspace::new("0.1.0")?;
//! let project = workspace.create_project(std::path::Path::new("school.evaraproj"))?;
//! println!("project {} at schema v{}", project.project_id, project.schema_version);
//!
//! if let Some(provider) = project.synced_folder {
//!     eprintln!("warning: this project is stored in {}", provider.provider());
//! }
//! # Ok(())
//! # }
//! ```

mod backup;
pub mod change_log;

// The one implementation of this workspace's timestamp format: ISO-8601 UTC to
// millisecond precision, which `CLAUDE.md` §3 requires and every stored column uses. Public
// so the command layer and the CLI stamp a `solve_run` the same way the repositories stamp
// `created_at`, rather than each formatting its own.
pub use meta::now_iso8601;
pub mod cloud_sync;
mod connection;
mod error;
pub mod import;
pub mod meta;
mod migrations;
pub mod repo;
pub mod scheduling;
pub mod settings;
pub mod workspace;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

pub use crate::change_log::{ChangeRecord, TrackedTable, Watermark};
pub use crate::cloud_sync::SyncedFolder;
pub use crate::connection::{BUSY_TIMEOUT, PragmaReport};
pub use crate::error::{DbError, InternalError, Result};
pub use crate::import::{Import, report_csv};
pub use crate::meta::ProjectIdentity;
pub use crate::migrations::supported_schema_version;
pub use crate::repo::availability::{
    Availability, AvailabilityEntity, AvailabilityRule, AvailabilityRuleInput, AvailabilityStatus,
};
pub use crate::repo::constraints::{
    ConstraintInstance, ConstraintInstanceInput, ConstraintScopeEntry, ConstraintScopeEntryInput,
    Constraints, ScopeEntity, ScopeRole,
};
pub use crate::repo::curriculum::{
    Curriculum, Department, DepartmentDependents, DepartmentInput, QualifiedSubject,
    QualifiedTeacher, Subject, SubjectDependents, SubjectInput, TeacherDepartment,
    TeacherDepartmentInput, TeacherDepartmentView, TeacherQualification, TeacherQualificationInput,
};
pub use crate::repo::lessons::{
    Activity, ActivityInput, AssignedTeacher, AssignmentEligibility, AttendingGroup,
    ClassEnrolment, ClassEnrolmentInput, ClassGroup, ClassGroupInput, ClassSection,
    ClassSectionDependents, ClassSectionInput, ClassSectionView, ClassTeacher, ClassTeacherInput,
    Course, CourseDependents, CourseInput, CourseView, Lessons, RosteredStudent, SectionDemand,
    TaughtSection, TeacherCandidate,
};
pub use crate::repo::membership::{
    EnrolmentStatus, Membership, StudentEnrolment, StudentEnrolmentInput, StudentInSchool,
    TeacherCampus, TeacherCampusInput, TeacherInSchool, TeacherSchool, TeacherSchoolInput,
};
pub use crate::repo::people::{
    GroupKind, People, Student, StudentGroup, StudentGroupInput, StudentGroupMember,
    StudentGroupMemberInput, StudentInput, Teacher, TeacherInput, YearLevel, YearLevelInput,
};
pub use crate::repo::structure::{
    AcademicYear, AcademicYearInput, Building, BuildingInput, BuildingWithCampuses, Campus,
    CampusInput, Resource, ResourceInput, Room, RoomInput, RoomType, RoomTypeInput,
    RoomWithCampuses, School, SchoolDependents, SchoolInput, SchoolLogo, Structure, Term,
    TermInput,
};
pub use crate::repo::time_model::{
    CalendarDay, CalendarDayInput, CalendarDayKind, Cycle, CycleDay, CycleDayInput, CycleInput,
    CycleKind, CycleShape, GridCell, GridPreview, MaterialisationCounts, MaterialisationPlan,
    MaterialisationRequest, NonWorkingInstructionalDay, Period, PeriodInput, PeriodKind,
    PeriodStructure, PeriodStructureInput, PlanFingerprint, PlannedSlot, TimeModel, Timeslot,
    TimeslotInput, WeeklyCycleDay,
};
pub use crate::repo::timetables::{
    AppliedEdit, ArrangementStamp, DeletedTimetable, DuplicatedTimetable, EditGuard,
    EntryPlacement, KIND_GENERATION, KIND_REVALIDATION, RunProvenance, SavedTimetable, Scoring,
    SolveRun, StoredArrangement, StoredPins, StoredViolation, StoredViolationRef, Timetable,
    TimetableEntry, TimetableEvaluation, Timetables, UnscheduledRecord,
};
pub use crate::repo::{Expected, Stamp};
pub use crate::scheduling::{
    Assembly, AssemblyReport, AssemblyStats, Completeness, ProblemIndex, ProblemIndexCounts,
    Scheduling, SchedulingSelection,
};
pub use crate::settings::{MAX_RECENT_PROJECTS, RecentProject, Settings};
pub use crate::workspace::{PROJECT_EXTENSION, ProjectSummary, Workspace};

/// An open project database.
///
/// Owns its [`rusqlite::Connection`] and never hands it out. That is what makes the
/// connection policy in [`connection`] enforceable: there is no way for another crate,
/// or a future call site in this one, to obtain a connection that skipped it.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
    path: PathBuf,
    identity: ProjectIdentity,
    schema_version: u32,
    site_id: Uuid,
    /// Safety copy taken by this open, if a schema rebuild needed one.
    ///
    /// Set only on the open that actually performed the upgrade, which is the only time
    /// the interface has anything to tell the user about it.
    safety_backup: Option<PathBuf>,
    /// Whether this project was opened for inspection only.
    ///
    /// `true` only for [`Database::open_for_inspection`]. [`Database::write`] refuses
    /// immediately on one, so a caller that reaches for a repository on an inspection handle
    /// gets a sentence rather than SQLite's *"attempt to write a readonly database"*.
    read_only: bool,
}

/// File name of a path, for messages that must not carry the full path.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || String::from("the selected file"),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl Database {
    /// Creates a new project file and migrates it to the current schema.
    ///
    /// Separate from [`Database::open_existing`] on purpose. A single create-or-open
    /// entry point would happily migrate any SQLite file the user picked by mistake into
    /// something that looks like an Evara project.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectAlreadyExists`] if a non-empty file is already there.
    /// - [`DbError::Migration`] if migration failed, in which case nothing was applied.
    pub fn create(path: impl AsRef<Path>, app_version: &str, site_id: Uuid) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() && std::fs::metadata(path).map_or(0, |m| m.len()) > 0 {
            return Err(DbError::ProjectAlreadyExists {
                name: file_name_of(path),
            });
        }
        Self::open_or_create(path, app_version, site_id)
    }

    /// Opens an existing project and migrates it to the current schema.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectNotFound`] if nothing is there.
    /// - [`DbError::NotAnEvaraProject`] if the file is not an Evara project â€” a different
    ///   SQLite database, or not a database at all.
    /// - [`DbError::SchemaTooNew`] if written by a newer Evara. Checked **before** any
    ///   migration runs, so an unsupported project is never modified.
    /// - [`DbError::PragmaRejected`] if a required connection setting did not take effect.
    pub fn open_existing(path: impl AsRef<Path>, app_version: &str, site_id: Uuid) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(DbError::ProjectNotFound {
                name: file_name_of(path),
            });
        }
        if !Self::looks_like_a_project(path)? {
            return Err(DbError::NotAnEvaraProject {
                name: file_name_of(path),
            });
        }
        Self::open_or_create(path, app_version, site_id)
    }

    /// Shared body of [`Database::create`] and [`Database::open_existing`].
    fn open_or_create(path: &Path, app_version: &str, site_id: Uuid) -> Result<Self> {
        let mut conn = connection::open_configured(path, site_id)?;

        // Before anything irreversible. `V12` is the first migration that rebuilds an
        // existing table, so a project below that version gets a verified copy of itself
        // first, and the upgrade is refused rather than run unprotected if the copy cannot
        // be made. Checked before `migrations::run` so the refusal arrives before the
        // project has been touched -- and *after* `open_configured`, which is what rejects
        // a file that is not a database at all.
        let applied = migrations::applied_schema_version(&conn)?;
        migrations::reject_if_newer_than_supported(&conn)?;
        let safety_backup = backup::before_schema_rebuild(&conn, path, applied)?;

        let schema_version = migrations::run(&mut conn)?;

        // Triggers are not data: recreating a missing one is always safe, so coverage is
        // ensured centrally on every open rather than trusted to each migration author.
        let tx = conn.transaction()?;
        change_log::install_all(&tx)?;
        tx.commit()?;

        let identity = meta::ensure(&mut conn, app_version, schema_version, site_id)?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
            identity,
            schema_version,
            site_id,
            safety_backup,
            read_only: false,
        })
    }

    /// Whether a file is an Evara project, without modifying it.
    ///
    /// A valid but *empty* SQLite file is not a project â€” that is the case `create`
    /// produces and `open_existing` must reject.
    fn looks_like_a_project(path: &Path) -> Result<bool> {
        let conn = match connection::open_plain(path) {
            Ok(conn) => conn,
            // "file is not a database" and friends mean the user picked the wrong file.
            Err(DbError::Open { .. } | DbError::Internal(_)) => return Ok(false),
            Err(other) => return Err(other),
        };
        let found: std::result::Result<i64, _> = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_meta'",
            [],
            |row| row.get(0),
        );
        Ok(matches!(found, Ok(count) if count > 0))
    }

    /// Opens an existing project for **inspection only**, migrating nothing.
    ///
    /// # Why this exists beside [`Database::open_existing`]
    ///
    /// An ordinary open writes. It runs pending migrations, recreates any missing change-log
    /// trigger, and `meta::ensure` stamps `last_opened_with` on every single open. All three
    /// are right for the application and wrong for a headless inspection of somebody's live
    /// school file: `evara validate --project` must be able to answer questions about a
    /// project without touching it, and a command that quietly upgraded the file it was asked
    /// to look at would be the worst kind of surprise.
    ///
    /// So this path does none of the three. It verifies the file is an Evara project, refuses
    /// one that is not at exactly the supported schema version — because reaching that
    /// version is itself a write — reads the identity rather than ensuring it, and returns a
    /// connection SQLite will not let anything write through. See
    /// [`connection::open_query_only`] for the three independent guarantees.
    ///
    /// # Errors
    ///
    /// - [`DbError::ProjectNotFound`], [`DbError::NotAnEvaraProject`] as
    ///   [`Database::open_existing`].
    /// - [`DbError::SchemaTooNew`] if written by a newer Evara.
    /// - [`DbError::SchemaUpgradeRequired`] if the project is behind this build. Opening it
    ///   normally would upgrade it, so the choice is the user's to make deliberately.
    /// - [`DbError::MissingMetadata`] if the project has no identity, which means it is not
    ///   one.
    pub fn open_for_inspection(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(DbError::ProjectNotFound {
                name: file_name_of(path),
            });
        }
        if !Self::looks_like_a_project(path)? {
            return Err(DbError::NotAnEvaraProject {
                name: file_name_of(path),
            });
        }

        let conn = connection::open_query_only(path)?;
        migrations::reject_if_newer_than_supported(&conn)?;
        let schema_version = migrations::applied_schema_version(&conn)?;
        let supported = migrations::supported_schema_version();
        if schema_version != supported {
            return Err(DbError::SchemaUpgradeRequired {
                found: schema_version,
                supported,
            });
        }

        let identity = meta::read_identity(&conn)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            identity,
            schema_version,
            // No installation is attributed, because nothing can be written. The site-id
            // function is deliberately not registered on this connection either.
            site_id: Uuid::nil(),
            safety_backup: None,
            read_only: true,
        })
    }

    /// Whether this project was opened for inspection only.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Reads a project's schema version without migrating it.
    ///
    /// Used by project opening to explain *why* a file cannot be opened before attempting
    /// to change it.
    ///
    /// # Errors
    ///
    /// [`DbError::SchemaTooNew`] if the project is from a newer Evara.
    pub fn inspect(path: impl AsRef<Path>) -> Result<u32> {
        let conn = connection::open_plain(path.as_ref())?;
        migrations::reject_if_newer_than_supported(&conn)?;
        migrations::applied_schema_version(&conn)
    }

    /// Path this project was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stable identifier of the school project, preserved across export and import.
    #[must_use]
    pub const fn project_id(&self) -> Uuid {
        self.identity.project_id
    }

    /// Installation that created this project, when recorded.
    ///
    /// Historical provenance, not the installation currently writing â€” see
    /// [`Database::site_id`].
    #[must_use]
    pub const fn created_by_site_id(&self) -> Option<Uuid> {
        self.identity.created_by_site_id
    }

    /// Installation attributed for writes through this connection.
    #[must_use]
    pub const fn site_id(&self) -> Uuid {
        self.site_id
    }

    /// Schema version currently applied.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The safety copy this open took before rebuilding a table, if it took one.
    ///
    /// `Some` only on the open that performed the upgrade — reopening an
    /// already-upgraded project returns `None`, because there is nothing new to tell the
    /// user. The interface shows the path so the copy is findable rather than mysterious;
    /// nothing in Evara ever deletes it.
    #[must_use]
    pub fn safety_backup(&self) -> Option<&Path> {
        self.safety_backup.as_deref()
    }

    /// The cloud provider this project appears to be stored under, if any.
    ///
    /// Advisory only. See [`cloud_sync`] for why this never blocks opening a project.
    #[must_use]
    pub fn synced_folder(&self) -> Option<SyncedFolder> {
        cloud_sync::detect(&self.path)
    }

    /// Reads and writes the Structure entities â€” campuses, rooms, terms and the rest.
    ///
    /// The only route to that data. Callers never see SQL or a connection.
    pub fn structure(&mut self) -> Structure<'_> {
        Structure::new(self)
    }

    /// Reads and writes the time model — cycles, bell schedules, the timeslot grid.
    ///
    /// The only route to that data, and the only way to materialise the grid.
    pub fn time_model(&mut self) -> TimeModel<'_> {
        TimeModel::new(self)
    }

    /// Reads and writes the People entities — teachers, students, groups and memberships.
    ///
    /// The only route to that data. Callers never see SQL or a connection.
    pub fn people(&mut self) -> People<'_> {
        People::new(self)
    }

    /// Reads and writes availability rules, for every entity kind that can carry them.
    pub fn availability(&mut self) -> Availability<'_> {
        Availability::new(self)
    }

    /// Reads and writes configured constraints.
    ///
    /// Separate from [`Database::scheduling`] because the two are opposite directions through
    /// the same subject: this reads and writes the rows a school *configures*, and that
    /// compiles a school into the problem those rules are evaluated against. A row here tunes
    /// a rule rather than adding one — every implemented type is evaluated whatever this table
    /// holds (ADR 0039) — which is why there is no `enable` method and why removing a
    /// configuration returns the school to the shipped level rather than switching a check
    /// off. See [`Constraints`].
    pub fn constraints(&mut self) -> Constraints<'_> {
        Constraints::new(self)
    }

    /// Reads and writes stored timetables, their runs and their scorings.
    ///
    /// Separate from [`Database::scheduling`] for the same reason [`Database::constraints`] is:
    /// the two are opposite directions through one subject. `scheduling` **compiles** a school
    /// into a problem and writes nothing — its receiver is a shared reference so it cannot —
    /// and this **persists** what a solve produced.
    ///
    /// Saving always inserts. Several timetables may exist for one `(cycle, term)` and that
    /// coexistence is the versioning; a regeneration never overwrites, and replacement is a
    /// save followed by an explicit delete. See [`Timetables`].
    pub fn timetables(&mut self) -> Timetables<'_> {
        Timetables::new(self)
    }

    /// Reads and writes school membership, campus eligibility and enrolment history.
    ///
    /// Separate from [`Database::people`] because the two answer different questions. A
    /// person is one row for one real human being; which schools employ or enrol them is a
    /// relationship, and since `V13` a project may hold several of those per person. See
    /// [`Membership`] for the whole shape.
    pub fn membership(&mut self) -> Membership<'_> {
        Membership::new(self)
    }

    /// Reads and writes departments, subjects, qualifications and department membership.
    ///
    /// Separate from [`Database::people`] for the reason [`Database::membership`] is: a
    /// teacher is one row for one person, while what they are *qualified* to teach names a
    /// school-owned subject and what *department* they are in names one school's internal
    /// organisation. Those are two different scopes meeting on one identity, and
    /// [adr/0025](../../docs/adr/0025-teacher-qualifications-across-school-owned-subjects.md)
    /// is about keeping them apart.
    pub fn curriculum(&mut self) -> Curriculum<'_> {
        Curriculum::new(self)
    }

    /// Reads and writes courses, class sections, their staffing and audience, and the
    /// occurrences they demand.
    ///
    /// Separate from [`Database::curriculum`] because the two answer different questions. A
    /// subject is *what* a school teaches; a course is an **offering** of one, a class
    /// section is one deliverable instance of that offering, and an activity is one
    /// placeable occurrence of that instance. None of those four tables carries a
    /// `school_id` — every one of them resolves its school through the chain
    /// `subject -> course -> class_section -> activity`, which is
    /// [adr/0029](../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
    /// and `ROADMAP.md`'s Phase 3 note made concrete.
    pub fn lessons(&mut self) -> Lessons<'_> {
        Lessons::new(self)
    }

    /// Reads one school's scheduling inputs and compiles them into a solver `Problem`.
    ///
    /// **The only handle on this type that takes `&self`**, and deliberately so. Every other
    /// repository can write, so each takes `&mut Database` and can reach
    /// [`Database::write`]; this one cannot, which makes "the loader performs no writes" a
    /// borrow-checker property rather than a convention. `evara validate --project` needs
    /// exactly that guarantee before it is safe to point at a real school's file.
    ///
    /// Separate from [`Database::lessons`] and [`Database::time_model`] because it answers a
    /// different question. Those read and write the rows a school edits; this one compiles a
    /// whole school into the dense-index form a solver searches over, which is the one place
    /// UUIDs become positions — `ARCHITECTURE.md` §3 rule 4. See
    /// [`scheduling`](crate::scheduling) for the pipeline and the three kinds of failure it
    /// keeps apart.
    pub const fn scheduling(&self) -> Scheduling<'_> {
        Scheduling::new(self)
    }

    /// Previews and applies a spreadsheet import.
    ///
    /// Separate from every repository because an import is not an entity: it reads a file
    /// the school already has, resolves every row against what the project holds, and
    /// writes the whole approved result in **one** transaction. That last part is why the
    /// engine lives in this crate at all — `Database::write` is `pub(crate)`, so an
    /// atomic multi-entity write is only expressible from inside it. See
    /// [adr/0030](../../docs/adr/0030-an-import-is-a-reviewed-plan.md).
    pub fn import(&mut self) -> Import<'_> {
        Import::new(self)
    }

    /// Reads the connection settings back from SQLite, for diagnostics and tests.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn pragma_report(&self) -> Result<PragmaReport> {
        connection::report(&self.conn)
    }

    /// Reads one project metadata value.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn metadata(&self, key: &str) -> Result<Option<String>> {
        meta::get(&self.conn, key)
    }

    /// Reads the change feed in order, from an exclusive sequence number.
    ///
    /// # Errors
    ///
    /// Propagates any SQLite failure.
    pub fn changes_since(&self, after: i64, limit: i64) -> Result<Vec<ChangeRecord>> {
        change_log::read_since(&self.conn, after, limit)
    }

    /// Runs `work` inside a transaction, committing if it succeeds.
    ///
    /// The only route to writing, so no caller can accidentally write outside a
    /// transaction. Repositories in later phases are built on this.
    ///
    /// Deliberately `pub(crate)`: it hands out a [`rusqlite::Transaction`], and SQL
    /// outside this crate is an architecture bug. Other crates get repository methods,
    /// not a cursor into the database.
    ///
    /// # Errors
    ///
    /// Propagates whatever `work` returns, having rolled back.
    pub(crate) fn write<T>(
        &mut self,
        work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        // Refused before a transaction is even opened. SQLite would refuse it too —
        // `query_only` is on — and so would every change-log trigger, for want of
        // `evara_site_id()`; this is the typed first of the three.
        if self.read_only {
            return Err(DbError::ReadOnlyProject);
        }
        let tx = self.conn.transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Reads with a prepared statement, for repositories.
    ///
    /// `pub(crate)` for the same reason as [`Database::write`].
    ///
    /// # Errors
    ///
    /// Propagates whatever `work` returns.
    pub(crate) fn read<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        work(&self.conn)
    }
}

/// Helpers for the suites written when a project held exactly one school.
#[cfg(test)]
mod tests_support;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_concurrency;

#[cfg(test)]
mod tests_curriculum;

#[cfg(test)]
mod tests_lifecycle;

#[cfg(test)]
mod tests_structure;

#[cfg(test)]
mod tests_shared_buildings;

#[cfg(test)]
mod tests_shared_rooms;

#[cfg(test)]
mod tests_time_model;
#[cfg(test)]
mod tests_timetables;

#[cfg(test)]
mod tests_views;

/// Manual editing: place, move, unschedule, lock, duplicate, and the three guards.
#[cfg(test)]
mod tests_editing;

#[cfg(test)]
mod tests_setup;

#[cfg(test)]
mod tests_walkthrough;

#[cfg(test)]
mod tests_working_days;

#[cfg(test)]
mod tests_cycle_day_limit;

#[cfg(test)]
mod tests_cycle_kinds;

#[cfg(test)]
mod tests_school_logo;

/// Phase 2: teachers, students, groups, membership, and what survives a reopen.
#[cfg(test)]
mod tests_people;

/// Phase 2: availability, and what the polymorphic reference needs to stay honest.
#[cfg(test)]
mod tests_availability;

/// Stage 0A: several schools in one project, and what the table rebuild had to preserve.
#[cfg(test)]
mod tests_multi_school;

/// Stage 0B: shared teachers, students and resources, and what sharing must never permit.
#[cfg(test)]
mod tests_membership;

/// Stage C: courses, class sections, who teaches and attends them, and what they demand.
#[cfg(test)]
mod tests_lessons;

/// Stage D: a spreadsheet becomes a reviewed plan, and the plan is applied or it is not.
#[cfg(test)]
mod tests_import;

/// M5: a configured rule, and the refusals that keep a stored row compilable.
#[cfg(test)]
mod tests_constraints;
