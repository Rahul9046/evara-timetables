//! Turning a parsed spreadsheet into a reviewed plan, and applying the plan atomically.
//!
//! # Why the whole engine is in the crate that owns SQL
//!
//! Not because it is convenient. `Database::write` is `pub(crate)` and hands out a
//! `rusqlite::Transaction`, so **an atomic multi-entity write is only expressible from
//! inside this crate** — a public repository method opens its own transaction, and
//! composing twelve of them would be twelve transactions and eleven places to fail
//! half-way. An import that created forty teachers and then failed on the forty-first has
//! to leave none of them, which is the whole of
//! [ADR 0030](../../../docs/adr/0030-an-import-is-a-reviewed-plan.md) §5.
//!
//! The *parser* is deliberately not here: `evara-import` reads the bytes and knows nothing
//! about schools, and this module resolves rows and knows nothing about worksheets. The
//! vocabulary they meet in is [`evara_core::import`].
//!
//! # Four stages, and only the last one writes
//!
//! ```text
//! read (evara-import)  →  resolve + validate  →  plan  →  apply
//!                         └──────── build() ────────┘     └ one transaction
//! ```
//!
//! [`Import::plan`] runs `build` against a read connection and **writes nothing at all** —
//! `tests_import::previewing_writes_nothing_at_all` asserts that over the change feed, not
//! over the row counts, because a write that was rolled back would still leave a feed
//! entry.
//!
//! [`Import::apply`] runs `build` **again, inside the write transaction**, and compares the
//! fingerprint with the one the user approved. That is the staleness guarantee and it is
//! not merely a token check: the plan is recomputed from the database as it is now, so a
//! row somebody created in another window since the preview changes the plan and the
//! mismatch is reported rather than applied. The token exists so the disagreement can be
//! *named*; the recomputation is what makes it safe.
//!
//! # What matching does and does not assume
//!
//! [ADR 0031](../../../docs/adr/0031-import-identity-matching.md) is the whole of it. In
//! one sentence: **a code matches, a name asks.** Where a row carries a code the planner
//! resolves it within the selected school and proposes a create or an update; where it
//! carries only names and something similar exists, the planner refuses to decide and the
//! user does. Nothing here merges two identities on the strength of a shared name, and
//! nothing silently overwrites a record that disagrees with the file.

use evara_core::import::{
    Choice, ColumnMapping, ImportFingerprint, ImportKind, ImportPlan, ImportSummary, PlanCounts,
    PlannedRow, RowAction, RowDecision, RowFinding, Severity, SourceCell, SourceTable,
};
use rusqlite::{Connection, Transaction};
use uuid::Uuid;

use crate::Database;
use crate::error::{DbError, Result};
use crate::repo::Stamp;
use crate::repo::row::text;

mod kinds;
mod lookup;
mod report;

pub use report::report_csv;

/// Handle for previewing and applying an import.
///
/// Obtained from [`Database::import`].
#[derive(Debug)]
pub struct Import<'a> {
    db: &'a mut Database,
}

impl<'a> Import<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

/// Everything the planner and the executor are given.
///
/// One struct rather than six parameters threaded through twenty functions, and it is what
/// makes "apply recomputes exactly what preview computed" checkable: both paths build a
/// `Request` from the same inputs and hand it to the same `build`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Request<'a> {
    /// The school every record lands in. Never inferred from the file.
    pub(crate) school_id: Uuid,
    /// What is being imported.
    pub(crate) kind: ImportKind,
    /// The parsed file.
    pub(crate) table: &'a SourceTable,
    /// Which column fills which field.
    pub(crate) mapping: &'a ColumnMapping,
    /// What the user decided about the rows the planner could not decide alone.
    pub(crate) decisions: &'a [RowDecision],
}

impl Request<'_> {
    /// The decision recorded for one line, if any.
    fn decision(&self, line: i64) -> Option<Choice> {
        self.decisions
            .iter()
            .find(|decision| decision.line == line)
            .map(|decision| decision.choice)
    }
}

/// One row, planned.
pub(crate) struct Built {
    /// What to tell the user.
    pub(crate) row: PlannedRow,
    /// What to do, when there is something to do.
    pub(crate) intent: Option<Intent>,
}

impl Import<'_> {
    /// Builds a preview. **Writes nothing.**
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] if a required field is not mapped, or if one column is mapped
    ///   to two fields. Both are about the *mapping* rather than about a row, so they stop
    ///   the whole plan rather than producing two hundred identical row errors.
    /// - [`DbError::NotFound`] if the school does not exist.
    /// - Any database failure.
    pub fn plan(
        &mut self,
        school_id: Uuid,
        kind: ImportKind,
        table: &SourceTable,
        mapping: &ColumnMapping,
        decisions: &[RowDecision],
    ) -> Result<ImportPlan> {
        let request = Request {
            school_id,
            kind,
            table,
            mapping,
            decisions,
        };
        self.db.read(|conn| {
            require_school(conn, school_id)?;
            check_mapping(&request)?;
            let built = build(conn, &request)?;
            Ok(assemble(&request, built))
        })
    }

    /// Applies an approved plan, in one transaction.
    ///
    /// # Errors
    ///
    /// - [`DbError::ReviewRequired`] if the plan has moved since it was shown. The project
    ///   is untouched; the caller re-previews.
    /// - [`DbError::Invalid`] if the plan still holds an unresolved error or conflict, or
    ///   if the mapping is incomplete.
    /// - Any constraint failure from a row, which rolls the **whole** import back.
    pub fn apply(
        &mut self,
        school_id: Uuid,
        kind: ImportKind,
        table: &SourceTable,
        mapping: &ColumnMapping,
        decisions: &[RowDecision],
        expected: &ImportFingerprint,
    ) -> Result<ImportSummary> {
        let request = Request {
            school_id,
            kind,
            table,
            mapping,
            decisions,
        };
        self.db.write(|tx| {
            require_school(tx, school_id)?;
            check_mapping(&request)?;

            // Recomputed *inside* the transaction, against the database as it is now. The
            // fingerprint comparison below is how a disagreement gets named; this is what
            // makes the answer current. ADR 0030 §6.
            let built = build(tx, &request)?;
            let plan = assemble(&request, built);

            if &plan.fingerprint != expected {
                return Err(DbError::ReviewRequired {
                    reviewed: expected.to_string(),
                    current: plan.fingerprint.to_string(),
                });
            }
            if plan.counts.blocked() {
                return Err(DbError::Invalid {
                    message: "This import still has rows that need attention. Resolve them \
                              in the preview and try again."
                        .to_owned(),
                });
            }

            // Rebuilt rather than kept, because `assemble` consumed the intents into the
            // plan's rows. One more pass is cheaper than threading both halves through.
            for item in build(tx, &request)? {
                if let Some(intent) = item.intent {
                    execute(tx, school_id, &intent)?;
                }
            }

            Ok(ImportSummary {
                kind,
                created: plan.counts.create,
                updated: plan.counts.update,
                unchanged: plan.counts.unchanged,
                skipped: plan.counts.skipped,
                warnings: plan.counts.warnings,
                rows_processed: plan.counts.total(),
                rows: plan.rows,
            })
        })
    }
}

/// Refuses a school that is not in this project.
fn require_school(conn: &Connection, school_id: Uuid) -> Result<()> {
    let found: i64 = conn.query_row(
        "SELECT COUNT(*) FROM school WHERE id = ?",
        [text(school_id)],
        |row| row.get(0),
    )?;
    if found == 0 {
        return Err(DbError::NotFound { entity: "school" });
    }
    Ok(())
}

/// Refuses a mapping that cannot produce a plan.
///
/// Both failures are about the mapping rather than about a row, so they stop the whole plan
/// rather than attaching the same message to every line. A user looking at two hundred
/// identical errors learns less than one looking at a sentence naming the missing column.
fn check_mapping(request: &Request<'_>) -> Result<()> {
    let clashes = request.mapping.duplicated_columns();
    if let Some((column, fields)) = clashes.first() {
        let heading = request
            .table
            .headers
            .get(*column)
            .map_or("that column", String::as_str);
        return Err(DbError::Invalid {
            message: format!(
                "“{heading}” is mapped to {} fields at once ({}). One column cannot hold \
                 two different things — give each field its own column, or leave the \
                 others unmapped.",
                fields.len(),
                fields.join(", ")
            ),
        });
    }

    let missing: Vec<&str> = evara_import_fields(request.kind)
        .into_iter()
        .filter(|(key, required)| *required && request.mapping.column_of(key).is_none())
        .map(|(key, _)| key)
        .collect();
    if !missing.is_empty() {
        return Err(DbError::Invalid {
            message: format!(
                "This import still needs a column for: {}. Map them, or start from the \
                 downloadable template.",
                missing.join(", ")
            ),
        });
    }
    Ok(())
}

/// Which fields one kind has, and which are required.
///
/// Mirrors `evara_import::template`, which cannot be depended on from here: that crate
/// depends on `evara-core` and this one, so a dependency the other way would be a cycle.
/// The two lists are held in step by
/// `tests_import::every_required_field_matches_the_template`, which reads the template's
/// own answer across the crate boundary from the test rather than from the library.
#[cfg(test)]
pub(crate) fn fields_for_test(kind: ImportKind) -> Vec<(&'static str, bool)> {
    evara_import_fields(kind)
}

fn evara_import_fields(kind: ImportKind) -> Vec<(&'static str, bool)> {
    match kind {
        ImportKind::Teachers => vec![
            ("code", false),
            ("givenName", true),
            ("familyName", true),
            ("email", false),
            ("isActive", false),
            ("departmentCodes", false),
            ("campusCodes", false),
        ],
        ImportKind::Students => vec![
            ("code", false),
            ("givenName", true),
            ("familyName", true),
            ("yearLevel", false),
            ("campusCode", false),
            ("status", false),
            ("enrolledOn", false),
            ("email", false),
        ],
        ImportKind::Departments => {
            vec![("code", false), ("name", true), ("description", false)]
        }
        ImportKind::Subjects => vec![
            ("code", false),
            ("name", true),
            ("departmentCode", false),
            ("description", false),
        ],
        ImportKind::Courses => vec![
            ("code", false),
            ("name", true),
            ("subjectCode", true),
            ("yearLevel", false),
            ("periodsPerCycle", false),
            ("defaultDurationPeriods", false),
            ("roomType", false),
        ],
        ImportKind::ClassSections => vec![
            ("courseCode", true),
            ("name", true),
            ("code", false),
            ("campusCode", false),
        ],
        ImportKind::ClassTeachers => vec![
            ("courseCode", true),
            ("sectionCode", true),
            ("teacherCode", true),
        ],
        ImportKind::ClassGroups => vec![
            ("courseCode", true),
            ("sectionCode", true),
            ("groupCode", true),
        ],
        ImportKind::ClassEnrolments => vec![
            ("courseCode", true),
            ("sectionCode", true),
            ("studentCode", true),
        ],
        ImportKind::Activities => vec![
            ("courseCode", true),
            ("sectionCode", true),
            ("durationPeriods", true),
            ("occurrences", false),
            ("name", false),
        ],
    }
}

/// Plans every row.
fn build(conn: &Connection, request: &Request<'_>) -> Result<Vec<Built>> {
    let mut seen = SeenKeys::default();
    let mut built = Vec::with_capacity(request.table.rows.len());
    for source in &request.table.rows {
        let mut row = Row::new(request, &source.cells, source.line);
        let item = kinds::plan_row(conn, request, &mut row, &mut seen)?;
        built.push(item);
    }
    Ok(built)
}

/// Keys already used by an earlier row of the same file.
///
/// In-file duplicates are their own category and must not be confused with a clash against
/// the database: two rows both claiming code `T-0007` is a mistake in the *file*, and
/// applying both would make the second overwrite the first silently.
#[derive(Debug, Default)]
pub(crate) struct SeenKeys {
    keys: Vec<(String, i64)>,
}

impl SeenKeys {
    /// Records a key, returning the earlier line that used it.
    pub(crate) fn claim(&mut self, key: &str, line: i64) -> Option<i64> {
        if key.is_empty() {
            return None;
        }
        if let Some((_, earlier)) = self.keys.iter().find(|(seen, _)| seen == key) {
            return Some(*earlier);
        }
        self.keys.push((key.to_owned(), line));
        None
    }
}

/// Assembles the planned rows into a plan, counting as it goes.
fn assemble(request: &Request<'_>, built: Vec<Built>) -> ImportPlan {
    let mut counts = PlanCounts::default();
    let rows: Vec<PlannedRow> = built.into_iter().map(|item| item.row).collect();

    for row in &rows {
        match row.action {
            RowAction::Create => counts.create += 1,
            RowAction::Update => counts.update += 1,
            RowAction::Unchanged => counts.unchanged += 1,
            RowAction::Skipped => counts.skipped += 1,
            RowAction::Conflict => counts.conflict += 1,
            RowAction::Invalid => counts.invalid += 1,
        }
        counts.warnings += i64::try_from(
            row.findings
                .iter()
                .filter(|finding| finding.severity == Severity::Warning)
                .count(),
        )
        .unwrap_or(i64::MAX);
    }

    ImportPlan {
        kind: request.kind,
        school_id: request.school_id,
        fingerprint: ImportFingerprint::of(request.kind, request.school_id, &rows),
        rows,
        counts,
    }
}

// =============================================================================== rows

/// One source row, read through the mapping.
///
/// Every accessor records a finding rather than returning an error, so a row with three bad
/// cells reports three problems instead of the first one. That is the difference between a
/// user fixing a file in one pass and fixing it in three.
pub(crate) struct Row<'a> {
    request: &'a Request<'a>,
    cells: &'a [SourceCell],
    /// The row's line in the sheet, as the user sees it.
    pub(crate) line: i64,
    findings: Vec<RowFinding>,
}

impl<'a> Row<'a> {
    fn new(request: &'a Request<'a>, cells: &'a [SourceCell], line: i64) -> Self {
        Self {
            request,
            cells,
            line,
            findings: Vec::new(),
        }
    }

    /// The cell a field is mapped to, if it is mapped and the row is long enough.
    fn cell(&self, field: &str) -> Option<&SourceCell> {
        let column = self.request.mapping.column_of(field)?;
        self.cells.get(column)
    }

    /// The heading of the column a field is mapped to, for a message.
    fn heading(&self, field: &str) -> String {
        self.request
            .mapping
            .column_of(field)
            .and_then(|column| self.request.table.headers.get(column))
            .filter(|heading| !heading.is_empty())
            .cloned()
            .unwrap_or_else(|| field.to_owned())
    }

    /// Records a finding.
    pub(crate) fn note(&mut self, finding: RowFinding) {
        self.findings.push(finding);
    }

    /// Whether anything recorded so far stops the row.
    pub(crate) fn has_error(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }

    /// Everything recorded, taken.
    ///
    /// Drained rather than consuming the row, because the planner holds a `&mut Row`
    /// throughout and the alternative was threading ownership through twenty branches.
    pub(crate) fn take_findings(&mut self) -> Vec<RowFinding> {
        std::mem::take(&mut self.findings)
    }

    /// Whether a field has a column at all.
    ///
    /// The difference between *mapped and blank* — "clear this" — and *unmapped* — "leave
    /// it alone". Without it an import of two columns would propose blanking the other
    /// eight, which is the quiet data loss this whole stage exists to avoid.
    pub(crate) fn cell_mapped(&self, field: &str) -> bool {
        self.request.mapping.column_of(field).is_some()
    }

    /// An identifier, or `None` when the column is unmapped or the cell is blank.
    pub(crate) fn code(&mut self, field: &str) -> Option<String> {
        let cell = self.cell(field)?;
        match cell.code() {
            Ok(value) => value.map(ToOwned::to_owned),
            Err(problem) => {
                let message = problem.message();
                self.note(RowFinding::error(
                    field,
                    format!("{}: {message}", self.heading(field)),
                ));
                None
            }
        }
    }

    /// An identifier that must be there.
    pub(crate) fn required_code(&mut self, field: &str) -> Option<String> {
        let value = self.code(field);
        if value.is_none() && !self.has_error() {
            self.note(RowFinding::error(
                field,
                format!(
                    "{} is required and this row leaves it blank.",
                    self.heading(field)
                ),
            ));
        }
        value
    }

    /// Free text, or `None`.
    pub(crate) fn text(&mut self, field: &str) -> Option<String> {
        let cell = self.cell(field)?;
        match cell {
            SourceCell::Unsupported(reason) => {
                let reason = reason.clone();
                self.note(RowFinding::error(
                    field,
                    format!("{}: {reason}", self.heading(field)),
                ));
                None
            }
            other => other.text().map(ToOwned::to_owned),
        }
    }

    /// Free text that must be there.
    pub(crate) fn required_text(&mut self, field: &str) -> Option<String> {
        let value = self.text(field);
        if value.is_none() && !self.has_error() {
            self.note(RowFinding::error(
                field,
                format!(
                    "{} is required and this row leaves it blank.",
                    self.heading(field)
                ),
            ));
        }
        value
    }

    /// A yes/no answer, or `None` for the default.
    pub(crate) fn flag(&mut self, field: &str) -> Option<bool> {
        let cell = self.cell(field)?;
        match cell.flag() {
            Ok(value) => value,
            Err(problem) => {
                let message = problem.message();
                self.note(RowFinding::error(
                    field,
                    format!("{}: {message}", self.heading(field)),
                ));
                None
            }
        }
    }

    /// A whole number, or `None`.
    pub(crate) fn number(&mut self, field: &str) -> Option<i64> {
        let cell = self.cell(field)?;
        match cell.whole_number() {
            Ok(value) => value,
            Err(problem) => {
                let message = problem.message();
                self.note(RowFinding::error(
                    field,
                    format!("{}: {message}", self.heading(field)),
                ));
                None
            }
        }
    }

    /// A calendar date, or `None`.
    pub(crate) fn date(&mut self, field: &str) -> Option<String> {
        let cell = self.cell(field)?;
        match cell.date() {
            Ok(value) => value,
            Err(problem) => {
                let message = problem.message();
                self.note(RowFinding::error(
                    field,
                    format!("{}: {message}", self.heading(field)),
                ));
                None
            }
        }
    }

    /// A semicolon-separated list of identifiers.
    ///
    /// Semicolons rather than commas, because a CSV field containing a comma has to be
    /// quoted and a school typing one into a spreadsheet cell usually does not.
    pub(crate) fn code_list(&mut self, field: &str) -> Vec<String> {
        let Some(raw) = self.code(field) else {
            return Vec::new();
        };
        raw.split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The user's decision about this row, if they made one.
    pub(crate) fn decision(&self) -> Option<Choice> {
        self.request.decision(self.line)
    }
}

// ============================================================================ intents

/// What applying one row would do.
///
/// Built by the planner and executed by [`execute`]. Deliberately a *resolved* description
/// — every reference is already a `Uuid` — so the executor does no lookups and cannot
/// disagree with what the preview said.
pub(crate) enum Intent {
    /// A teacher this school has never employed.
    CreateTeacher {
        input: crate::repo::people::TeacherInput,
        campuses: Vec<Uuid>,
        departments: Vec<Uuid>,
    },
    /// A teacher identity the project already holds, employed here for the first time.
    EmployTeacher {
        teacher_id: Uuid,
        is_active: bool,
        campuses: Vec<Uuid>,
        departments: Vec<Uuid>,
    },
    /// A teacher this school already employs.
    UpdateTeacher {
        id: Uuid,
        rev: i64,
        input: crate::repo::people::TeacherInput,
        campuses: Vec<Uuid>,
        departments: Vec<Uuid>,
    },
    /// A student this school has never enrolled.
    CreateStudent {
        input: crate::repo::people::StudentInput,
        spell: crate::repo::membership::StudentEnrolmentInput,
    },
    /// A student identity the project already holds, enrolled here for the first time.
    EnrolStudent {
        spell: crate::repo::membership::StudentEnrolmentInput,
    },
    /// A student this school already has.
    UpdateStudent {
        id: Uuid,
        rev: i64,
        input: crate::repo::people::StudentInput,
    },
    /// A new department.
    CreateDepartment {
        input: crate::repo::curriculum::DepartmentInput,
    },
    /// An existing department, changed.
    UpdateDepartment {
        id: Uuid,
        rev: i64,
        input: crate::repo::curriculum::DepartmentInput,
    },
    /// A new subject.
    CreateSubject {
        input: crate::repo::curriculum::SubjectInput,
    },
    /// An existing subject, changed.
    UpdateSubject {
        id: Uuid,
        rev: i64,
        input: crate::repo::curriculum::SubjectInput,
    },
    /// A new course.
    CreateCourse {
        input: crate::repo::lessons::CourseInput,
    },
    /// An existing course, changed.
    UpdateCourse {
        id: Uuid,
        rev: i64,
        input: crate::repo::lessons::CourseInput,
    },
    /// A new class section.
    CreateSection {
        input: crate::repo::lessons::ClassSectionInput,
    },
    /// An existing class section, changed.
    UpdateSection {
        id: Uuid,
        rev: i64,
        input: crate::repo::lessons::ClassSectionInput,
    },
    /// A relationship row: which two rows, in which table.
    ///
    /// One variant for `class_teacher`, `class_group` and `class_enrolment` because the
    /// three are the same shape — two identifiers and the audit columns — and the schema's
    /// trigger pairs are what enforce the rules each one has.
    Link {
        table: &'static str,
        left_column: &'static str,
        left: Uuid,
        right_column: &'static str,
        right: Uuid,
        entity: &'static str,
    },
    /// Occurrences to add to a class section.
    ///
    /// `count` is the *difference* between what the file asks for and what the section
    /// already has, so a second run of the same file adds nothing. ADR 0031 §5.
    AddOccurrences {
        class_section_id: Uuid,
        name: Option<String>,
        duration_periods: i64,
        count: i64,
    },
}

/// Applies one intent inside the caller's transaction.
///
/// Every write goes through the repositories' own transaction-scoped helpers, so the import
/// path and the screen path share one statement per table. That is what stops a column
/// added to `teacher` from being set by one and missed by the other.
fn execute(tx: &Transaction<'_>, school_id: Uuid, intent: &Intent) -> Result<()> {
    use crate::repo::Expected;
    use crate::repo::membership::mirror_student_onto_enrolment;
    use crate::repo::membership::{insert_enrolment, insert_membership};
    use crate::repo::people::{
        insert_student, insert_teacher, update_student_row, update_teacher_row,
    };

    match intent {
        Intent::CreateTeacher {
            input,
            campuses,
            departments,
        } => {
            let stamp = Stamp::new();
            insert_teacher(tx, &stamp, input)?;
            // The membership inherits the project-level flag, exactly as `create_teacher`
            // does: before memberships existed that flag *was* the per-school answer.
            insert_membership(
                tx,
                &Stamp::new(),
                stamp.id,
                input.school_id,
                input.is_active,
            )?;
            narrowings(tx, stamp.id, campuses, departments)
        }
        Intent::EmployTeacher {
            teacher_id,
            is_active,
            campuses,
            departments,
        } => {
            insert_membership(tx, &Stamp::new(), *teacher_id, school_id, *is_active)?;
            narrowings(tx, *teacher_id, campuses, departments)
        }
        Intent::UpdateTeacher {
            id,
            rev,
            input,
            campuses,
            departments,
        } => {
            update_teacher_row(tx, *id, Expected::Rev(*rev), input)?;
            narrowings(tx, *id, campuses, departments)
        }
        Intent::CreateStudent { input, spell } => {
            let stamp = Stamp::new();
            insert_student(tx, &stamp, input)?;
            let mut spell = spell.clone();
            spell.student_id = stamp.id;
            insert_enrolment(tx, &Stamp::new(), &spell)
        }
        Intent::EnrolStudent { spell } => insert_enrolment(tx, &Stamp::new(), spell),
        Intent::UpdateStudent { id, rev, input } => {
            update_student_row(tx, *id, Expected::Rev(*rev), input)?;
            // `update_student` does this too: `V13`'s compatibility policy keeps
            // `student.code` and the current spell's `admission_no` in step through the
            // repository rather than by dropping one of them.
            mirror_student_onto_enrolment(tx, *id)
        }
        other => execute_curriculum(tx, other),
    }
}

/// The curriculum and lesson half of [`execute`].
///
/// Split only because the two halves together are past the line limit. The seam is the one
/// a reader already uses: people above, what a school teaches below.
fn execute_curriculum(tx: &Transaction<'_>, intent: &Intent) -> Result<()> {
    use crate::repo::Expected;
    use crate::repo::curriculum::{
        insert_department, insert_subject, update_department_row, update_subject_row,
    };
    use crate::repo::lessons::{
        insert_class_section, insert_course, insert_occurrences, update_class_section_row,
        update_course_row,
    };

    match intent {
        Intent::CreateDepartment { input } => insert_department(tx, &Stamp::new(), input),
        Intent::UpdateDepartment { id, rev, input } => {
            update_department_row(tx, *id, Expected::Rev(*rev), input)
        }
        Intent::CreateSubject { input } => insert_subject(tx, &Stamp::new(), input),
        Intent::UpdateSubject { id, rev, input } => {
            update_subject_row(tx, *id, Expected::Rev(*rev), input)
        }
        Intent::CreateCourse { input } => insert_course(tx, &Stamp::new(), input),
        Intent::UpdateCourse { id, rev, input } => {
            update_course_row(tx, *id, Expected::Rev(*rev), input)
        }
        Intent::CreateSection { input } => insert_class_section(tx, &Stamp::new(), input),
        Intent::UpdateSection { id, rev, input } => {
            update_class_section_row(tx, *id, Expected::Rev(*rev), input)
        }
        Intent::Link {
            table,
            left_column,
            left,
            right_column,
            right,
            entity,
        } => insert_link(tx, table, left_column, *left, right_column, *right, entity),
        Intent::AddOccurrences {
            class_section_id,
            name,
            duration_periods,
            count,
        } => {
            insert_occurrences(
                tx,
                *class_section_id,
                name.as_deref(),
                *duration_periods,
                *count,
            )?;
            Ok(())
        }
        // Handled by `execute`; naming them here would be a second place to keep in step.
        Intent::CreateTeacher { .. }
        | Intent::EmployTeacher { .. }
        | Intent::UpdateTeacher { .. }
        | Intent::CreateStudent { .. }
        | Intent::EnrolStudent { .. }
        | Intent::UpdateStudent { .. } => unreachable!("handled by execute"),
    }
}

/// Writes a teacher's campus eligibility and department associations.
///
/// Both are *narrowings* of membership, both are first-class rows with their own audit
/// columns, and both are already-present-safe: the planner only ever puts an association
/// here that the database does not hold, so there is nothing to upsert.
fn narrowings(
    tx: &Transaction<'_>,
    teacher_id: Uuid,
    campuses: &[Uuid],
    departments: &[Uuid],
) -> Result<()> {
    for campus_id in campuses {
        insert_link(
            tx,
            "teacher_campus",
            "teacher_id",
            teacher_id,
            "campus_id",
            *campus_id,
            "campus eligibility",
        )?;
    }
    for department_id in departments {
        insert_link(
            tx,
            "teacher_department",
            "teacher_id",
            teacher_id,
            "department_id",
            *department_id,
            "department association",
        )?;
    }
    Ok(())
}

/// Writes one relationship row.
///
/// One statement for five tables, because `teacher_campus`, `teacher_department`,
/// `class_teacher`, `class_group` and `class_enrolment` are genuinely the same shape: two
/// identifiers and the three audit columns. Five extracted helpers would be five copies of
/// this.
///
/// Table and column names are `&'static str` from this crate, never caller input, and go
/// through [`crate::repo::identifier`] before interpolation anyway.
fn insert_link(
    tx: &Transaction<'_>,
    table: &'static str,
    left_column: &'static str,
    left: Uuid,
    right_column: &'static str,
    right: Uuid,
    entity: &'static str,
) -> Result<()> {
    let table = crate::repo::identifier(table)?;
    let left_column = crate::repo::identifier(left_column)?;
    let right_column = crate::repo::identifier(right_column)?;
    let stamp = Stamp::new();

    tx.execute(
        &format!(
            "INSERT INTO {table} (id, {left_column}, {right_column},
                 created_at, updated_at, rev)
             VALUES (?, ?, ?, ?, ?, ?)"
        ),
        rusqlite::params![
            text(stamp.id),
            text(left),
            text(right),
            &stamp.created_at,
            &stamp.updated_at,
            stamp.rev,
        ],
    )
    .map_err(|error| crate::repo::translate(error, entity))?;
    Ok(())
}
