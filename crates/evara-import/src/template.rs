//! The column definitions, and the downloadable templates written from them.
//!
//! # One table per import kind, and one source of truth
//!
//! Everything about a column lives in one [`TemplateColumn`]: the field key a mapping
//! names, the label the interface shows, whether it is required, the hint that explains
//! what happens if it is left out, the headings the mapper will recognise, and the example
//! the template writes.
//!
//! That matters because the alternative drifts. A template that writes `Staff ID` while the
//! mapper recognises `Teacher Code` produces a file Evara generated and cannot map — which
//! is the single most embarrassing way this feature could fail. Deriving both from the same
//! row makes that impossible, and `a_template_maps_itself` asserts it for all ten kinds.
//!
//! # Why the templates are workbooks
//!
//! A CSV template would be two lines of code instead of these. It is offered, but `.xlsx`
//! is the default, for one concrete reason: a template's **code columns are formatted as
//! text**. Hand a school a CSV with `00124` in it, let them open it in Excel and save, and
//! the zeros are gone before the file comes back — the exact fidelity failure the import
//! then has to refuse, created by our own template. A workbook column formatted as text
//! survives the round trip.

use evara_core::import::{FieldSpec, ImportKind};

/// How a template is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateFormat {
    /// A workbook, with code columns formatted as text. The default, and the safe one.
    Xlsx,
    /// Plain text, for anyone who would rather not open a workbook.
    Csv,
}

/// What one column of a template is.
#[derive(Debug, Clone, Copy)]
pub struct TemplateColumn {
    /// The field key a [`evara_core::import::ColumnMapping`] names. Stable; never a label.
    pub key: &'static str,
    /// The heading the template writes, and the one the mapper prefers.
    pub heading: &'static str,
    /// Whether a mapping is required before a plan can be built.
    pub required: bool,
    /// Whether the column holds an identifier, and so must stay text.
    ///
    /// Drives both the workbook's text formatting and the per-cell rule that refuses a
    /// numeric cell here.
    pub is_code: bool,
    /// What the column means, and what happens when it is left out.
    pub hint: &'static str,
    /// Other headings the mapper will accept for this field.
    pub also_known_as: &'static [&'static str],
    /// A plausible value, for the example row. Synthetic — `CLAUDE.md` rule 3.
    pub example: &'static str,
}

impl TemplateColumn {
    /// Every heading this column answers to, its own first.
    fn headings(&self) -> Vec<&'static str> {
        let mut all = vec![self.heading];
        all.extend_from_slice(self.also_known_as);
        all
    }

    /// The column as the interface's field picker wants it.
    const fn spec(&self) -> FieldSpec {
        FieldSpec {
            key: self.key,
            label: self.heading,
            required: self.required,
            hint: self.hint,
        }
    }
}

/// The columns of one import kind, in the order the template writes them.
#[must_use]
pub fn template_columns(kind: ImportKind) -> &'static [TemplateColumn] {
    match kind {
        ImportKind::Teachers => TEACHERS,
        ImportKind::Students => STUDENTS,
        ImportKind::Departments => DEPARTMENTS,
        ImportKind::Subjects => SUBJECTS,
        ImportKind::Courses => COURSES,
        ImportKind::ClassSections => CLASS_SECTIONS,
        ImportKind::ClassTeachers => CLASS_TEACHERS,
        ImportKind::ClassGroups => CLASS_GROUPS,
        ImportKind::ClassEnrolments => CLASS_ENROLMENTS,
        ImportKind::Activities => ACTIVITIES,
    }
}

/// The fields of one import kind, for the interface's mapping step.
#[must_use]
pub fn field_specs(kind: ImportKind) -> Vec<FieldSpec> {
    template_columns(kind)
        .iter()
        .map(TemplateColumn::spec)
        .collect()
}

/// The headings the mapper will accept, per field.
///
/// Built from the same rows the template writes, which is what stops a generated template
/// from being unmappable.
#[must_use]
pub(crate) fn accepted_headings(
    kind: ImportKind,
) -> &'static [(&'static str, &'static [&'static str])] {
    // Lazily built once per kind. A `OnceLock` per kind rather than one map, so the
    // returned slice can be `'static` and the caller needs no lifetime.
    macro_rules! cached {
        ($name:ident, $kind:expr) => {{
            static CELL: std::sync::OnceLock<Vec<(&'static str, &'static [&'static str])>> =
                std::sync::OnceLock::new();
            CELL.get_or_init(|| {
                template_columns($kind)
                    .iter()
                    .map(|column| {
                        let headings: &'static [&'static str] =
                            Box::leak(column.headings().into_boxed_slice());
                        (column.key, headings)
                    })
                    .collect()
            })
            .as_slice()
        }};
    }

    match kind {
        ImportKind::Teachers => cached!(TEACHERS, ImportKind::Teachers),
        ImportKind::Students => cached!(STUDENTS, ImportKind::Students),
        ImportKind::Departments => cached!(DEPARTMENTS, ImportKind::Departments),
        ImportKind::Subjects => cached!(SUBJECTS, ImportKind::Subjects),
        ImportKind::Courses => cached!(COURSES, ImportKind::Courses),
        ImportKind::ClassSections => cached!(CLASS_SECTIONS, ImportKind::ClassSections),
        ImportKind::ClassTeachers => cached!(CLASS_TEACHERS, ImportKind::ClassTeachers),
        ImportKind::ClassGroups => cached!(CLASS_GROUPS, ImportKind::ClassGroups),
        ImportKind::ClassEnrolments => cached!(CLASS_ENROLMENTS, ImportKind::ClassEnrolments),
        ImportKind::Activities => cached!(ACTIVITIES, ImportKind::Activities),
    }
}

/// The file name a template downloads as.
#[must_use]
pub fn template_file_name(kind: ImportKind, format: TemplateFormat) -> String {
    let extension = match format {
        TemplateFormat::Xlsx => "xlsx",
        TemplateFormat::Csv => "csv",
    };
    format!("{}-template.{extension}", kind.template_stem())
}

/// Writes a template.
///
/// The workbook carries three things a CSV cannot: code columns formatted as **text**, so
/// an identifier survives the school editing and re-saving it; a frozen header row; and a
/// second worksheet documenting every column, so the instructions travel with the file
/// rather than living only in a screen the user has closed.
///
/// # Errors
///
/// Only a workbook-writer failure, which has no user-facing cause — it would mean the
/// library could not serialise its own output.
pub fn template_bytes(kind: ImportKind, format: TemplateFormat) -> Result<Vec<u8>, TemplateError> {
    match format {
        TemplateFormat::Csv => Ok(csv_template(kind)),
        TemplateFormat::Xlsx => xlsx_template(kind),
    }
}

/// Why a template could not be written.
#[derive(Debug, thiserror::Error)]
#[error("the template could not be written: {detail}")]
pub struct TemplateError {
    /// What the writer said.
    detail: String,
}

/// A CSV template: headings, then one example row.
fn csv_template(kind: ImportKind) -> Vec<u8> {
    let columns = template_columns(kind);
    let mut writer = csv::Writer::from_writer(Vec::new());

    // `unwrap` is sound: the writer's target is an in-memory `Vec`, which cannot fail to
    // accept bytes, and every value here is plain ASCII-safe text.
    writer
        .write_record(columns.iter().map(|column| column.heading))
        .expect("writing to a Vec cannot fail");
    writer
        .write_record(columns.iter().map(|column| column.example))
        .expect("writing to a Vec cannot fail");
    writer.flush().expect("flushing a Vec cannot fail");
    writer
        .into_inner()
        .expect("the writer owns an in-memory buffer")
}

/// A workbook template: a data sheet and a documentation sheet.
fn xlsx_template(kind: ImportKind) -> Result<Vec<u8>, TemplateError> {
    use rust_xlsxwriter::{Format, FormatAlign, Workbook};

    let columns = template_columns(kind);
    let mut workbook = Workbook::new();

    let heading_format = Format::new()
        .set_bold()
        .set_background_color(0x00_EE_EE_EE)
        .set_align(FormatAlign::Left);
    // The point of the whole workbook: a code column is text, so `00124` stays `00124`
    // through the school opening the file and saving it.
    let text_format = Format::new().set_num_format("@");

    let sheet = workbook.add_worksheet();
    sheet
        .set_name(kind.label())
        .map_err(|error| TemplateError {
            detail: error.to_string(),
        })?;

    for (index, column) in columns.iter().enumerate() {
        let position = u16::try_from(index).unwrap_or(u16::MAX);
        sheet
            .write_string_with_format(0, position, column.heading, &heading_format)
            .map_err(|error| write_failed(&error))?;
        // Generous but finite: a name column wants room, and `set_column_width` is the only
        // thing standing between a reader and a sheet of `####`.
        // `u16` first, so the cast to the width's `f64` is lossless. A heading longer than
        // 64k characters is not a heading.
        let width = f64::from(u16::try_from(column.heading.len().max(14)).unwrap_or(u16::MAX));
        sheet
            .set_column_width(position, width + 2.0)
            .map_err(|error| write_failed(&error))?;
        if column.is_code {
            sheet
                .set_column_format(position, &text_format)
                .map_err(|error| write_failed(&error))?;
            sheet
                .write_string_with_format(1, position, column.example, &text_format)
                .map_err(|error| write_failed(&error))?;
        } else {
            sheet
                .write_string(1, position, column.example)
                .map_err(|error| write_failed(&error))?;
        }
    }
    // The header stays visible while a school scrolls two thousand rows.
    sheet
        .set_freeze_panes(1, 0)
        .map_err(|error| write_failed(&error))?;

    write_notes_sheet(&mut workbook, kind, &heading_format)?;

    workbook
        .save_to_buffer()
        .map_err(|error| write_failed(&error))
}

/// Writes the second worksheet: what every column means.
///
/// Its own function because the data sheet and this one together are past the line limit,
/// and because they answer different questions — one is the shape of the file, this is the
/// instructions that travel with it.
fn write_notes_sheet(
    workbook: &mut rust_xlsxwriter::Workbook,
    kind: ImportKind,
    heading_format: &rust_xlsxwriter::Format,
) -> Result<(), TemplateError> {
    let columns = template_columns(kind);
    let notes = workbook.add_worksheet();
    notes
        .set_name("How to fill this in")
        .map_err(|error| TemplateError {
            detail: error.to_string(),
        })?;
    notes
        .write_string_with_format(0, 0, "Column", heading_format)
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string_with_format(0, 1, "Required", heading_format)
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string_with_format(0, 2, "What it means", heading_format)
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string_with_format(0, 3, "Also accepted as", heading_format)
        .map_err(|error| write_failed(&error))?;
    notes
        .set_column_width(0, 24.0)
        .map_err(|error| write_failed(&error))?;
    notes
        .set_column_width(1, 10.0)
        .map_err(|error| write_failed(&error))?;
    notes
        .set_column_width(2, 90.0)
        .map_err(|error| write_failed(&error))?;
    notes
        .set_column_width(3, 40.0)
        .map_err(|error| write_failed(&error))?;

    for (index, column) in columns.iter().enumerate() {
        let row = u32::try_from(index).unwrap_or(u32::MAX) + 1;
        notes
            .write_string(row, 0, column.heading)
            .map_err(|error| write_failed(&error))?;
        notes
            .write_string(row, 1, if column.required { "Yes" } else { "Optional" })
            .map_err(|error| write_failed(&error))?;
        notes
            .write_string(row, 2, column.hint)
            .map_err(|error| write_failed(&error))?;
        notes
            .write_string(row, 3, column.also_known_as.join(", "))
            .map_err(|error| write_failed(&error))?;
    }

    let footer = u32::try_from(columns.len()).unwrap_or(u32::MAX) + 2;
    notes
        .write_string(footer, 0, "Identifiers")
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string(
            footer,
            2,
            "Code columns in the first sheet are formatted as text so leading zeros \
             survive. If you retype one, keep it as text — Evara refuses a code that \
             arrived as a number, because the zeros are already lost by then.",
        )
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string(footer + 1, 0, "Duplicates")
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string(
            footer + 1,
            2,
            "A row matching a record Evara already holds is reported as unchanged or as an \
             update, never applied twice. Importing the same file again changes nothing.",
        )
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string(footer + 2, 0, "Dates")
        .map_err(|error| write_failed(&error))?;
    notes
        .write_string(
            footer + 2,
            2,
            "Write dates as YYYY-MM-DD, or format the column as a date in the spreadsheet. \
             Evara will not guess between 03/04 and 04/03.",
        )
        .map_err(|error| write_failed(&error))?;

    Ok(())
}

fn write_failed(error: &rust_xlsxwriter::XlsxError) -> TemplateError {
    TemplateError {
        detail: error.to_string(),
    }
}

// ========================================================================== the columns

/// Shorthand for one column, so the tables below read as data rather than as struct syntax.
macro_rules! column {
    (
        $key:literal, $heading:literal,
        required: $required:literal, code: $code:literal,
        example: $example:literal,
        aka: [$($aka:literal),* $(,)?],
        $hint:literal
    ) => {
        TemplateColumn {
            key: $key,
            heading: $heading,
            required: $required,
            is_code: $code,
            hint: $hint,
            also_known_as: &[$($aka),*],
            example: $example,
        }
    };
}

/// Teachers: a project-wide identity, and this school's employment of it.
static TEACHERS: &[TemplateColumn] = &[
    column!("code", "Teacher Code", required: false, code: true, example: "T-0007",
        aka: ["Staff ID", "Staff Code", "Teacher ID", "Payroll Number"],
        "This school's own reference for the member of staff. Optional, but strongly \
         recommended: it is what lets a second import of the same file recognise somebody \
         rather than create them again. Must be unique within this school."),
    column!("givenName", "First Name", required: true, code: false, example: "Ada",
        aka: ["Given Name", "Forename", "First"],
        "Required. Whatever the school calls the staff member day to day."),
    column!("familyName", "Last Name", required: true, code: false, example: "Lovelace",
        aka: ["Surname", "Family Name", "Last"],
        "Required. Staff lists throughout Evara are ordered by this."),
    column!("email", "Email", required: false, code: false, example: "a.lovelace@example.edu",
        aka: ["Email Address", "E-mail"],
        "Optional, and never a login — Evara has no accounts."),
    column!("isActive", "Active in School", required: false, code: false, example: "Yes",
        aka: ["Active", "Currently Teaching", "Status"],
        "Yes or No. Blank means Yes. No records somebody on the staff list who is not \
         currently teaching here: they keep every class they already have and are offered \
         no new ones."),
    column!("departmentCodes", "Department Codes", required: false, code: true, example: "MAT",
        aka: ["Departments", "Department", "Department Code"],
        "Optional. One or more of this school's existing department codes, separated by \
         commas or semicolons. Departments are never created by a teacher import — import \
         them first if they are missing."),
    column!("campusCodes", "Campus Codes", required: false, code: true, example: "NGM",
        aka: ["Campuses", "Campus", "Campus Code", "Site"],
        "Optional. One or more of this school's existing campus codes, separated by commas \
         or semicolons. Leaving this blank means the teacher is not narrowed to particular \
         campuses, which is what most schools want."),
];

/// Students: a project-wide identity, and this school's enrolment spell.
static STUDENTS: &[TemplateColumn] = &[
    column!("code", "Student Code", required: false, code: true, example: "00124",
        aka: ["Student ID", "Admission Number", "Admission No", "Roll Number"],
        "This school's own number for the student. Optional, but strongly recommended: it \
         is what lets a second import recognise somebody rather than enrol them twice. \
         Formatted as text so leading zeros survive."),
    column!("givenName", "First Name", required: true, code: false, example: "Grace",
        aka: ["Given Name", "Forename", "First"],
        "Required. A name alone never matches an existing student -- only a code does -- \
         so two children with the same name are two rows and two records."),
    column!("familyName", "Last Name", required: true, code: false, example: "Hopper",
        aka: ["Surname", "Family Name", "Last"],
        "Required. Student lists throughout Evara are ordered by this."),
    column!("yearLevel", "Year Level", required: false, code: false, example: "Year 9",
        aka: ["Grade", "Year", "Form", "Year Group"],
        "Optional. Must name one of this school's existing year levels. Year levels are \
         never created by a student import."),
    column!("campusCode", "Campus", required: false, code: true, example: "NGM",
        aka: ["Campus Code", "Site"],
        "Optional. One of this school's existing campus codes."),
    column!("status", "Enrolment Status", required: false, code: false, example: "Enrolled",
        aka: ["Status", "Enrollment Status"],
        "Enrolled, Prospective or Left. Blank means Enrolled. Prospective is an applicant \
         — recorded and placeable in classes, not yet attending."),
    column!("enrolledOn", "Enrolment Date", required: false, code: false, example: "2026-09-01",
        aka: ["Admission Date", "Start Date", "Date Enrolled"],
        "Optional, as YYYY-MM-DD. Plenty of schools do not record the day somebody joined, \
         and a blank is a complete answer."),
    column!("email", "Email", required: false, code: false, example: "",
        aka: ["Email Address", "E-mail"],
        "Optional, and never a login."),
];

/// Departments of one school.
static DEPARTMENTS: &[TemplateColumn] = &[
    column!("code", "Department Code", required: false, code: true, example: "MAT",
        aka: ["Code", "Dept Code", "Department ID"],
        "Optional but recommended: the code is what a subject or teacher import refers to. \
         Unique within this school."),
    column!("name", "Department Name", required: true, code: false, example: "Mathematics",
        aka: ["Name", "Department"],
        "Required. Unique within this school."),
    column!("description", "Description", required: false, code: false, example: "",
        aka: ["Notes", "Comment"],
        "Optional free text."),
];

/// Subjects of one school.
static SUBJECTS: &[TemplateColumn] = &[
    column!("code", "Subject Code", required: false, code: true, example: "MAT",
        aka: ["Code", "Subject ID"],
        "Optional but recommended: the code is what a course import refers to. Unique \
         within this school — another school in this project may use the same code for its \
         own subject."),
    column!("name", "Subject Name", required: true, code: false, example: "Mathematics",
        aka: ["Name", "Subject"],
        "Required. Unique within this school."),
    column!("departmentCode", "Department Code", required: false, code: true, example: "MAT",
        aka: ["Department", "Dept", "Faculty"],
        "Optional. Must name one of this school's existing departments. A subject import \
         never creates a department — import departments first."),
    column!("description", "Description", required: false, code: false, example: "",
        aka: ["Notes", "Comment"],
        "Optional free text."),
];

/// Courses: a school's offering of a subject.
static COURSES: &[TemplateColumn] = &[
    column!("code", "Course Code", required: false, code: true, example: "9MAT",
        aka: ["Code", "Course ID"],
        "Optional but recommended: the code is what a class-section import refers to. \
         Unique within this school, across every subject."),
    column!("name", "Course Name", required: true, code: false, example: "Year 9 Mathematics",
        aka: ["Name", "Course"],
        "Required. Unique within the subject."),
    column!("subjectCode", "Subject Code", required: true, code: true, example: "MAT",
        aka: ["Subject", "Subject Name"],
        "Required. Must name one of this school's existing subjects, by code or by name. A \
         course import never creates a subject."),
    column!("yearLevel", "Year Level", required: false, code: false, example: "Year 9",
        aka: ["Grade", "Year", "Year Group"],
        "Optional. Must name one of this school's existing year levels."),
    column!("periodsPerCycle", "Periods per Cycle", required: false, code: false, example: "4",
        aka: ["Periods", "Periods Per Week", "Lessons per Cycle"],
        "Optional. How many period slots one section of this course needs in a cycle. Left \
         blank, nothing guesses — and the lesson-occurrence import can still be given the \
         figure per section."),
    column!("defaultDurationPeriods", "Periods per Lesson", required: false, code: false,
        example: "1",
        aka: ["Lesson Length", "Duration", "Default Duration"],
        "Optional, default 1. How long one lesson is. 4 periods a cycle at 1 per lesson is \
         four single lessons; at 2 it is two doubles."),
    column!("roomType", "Required Room Type", required: false, code: true, example: "",
        aka: ["Room Type", "Room Requirement", "Space Type"],
        "Optional. Must name one of this school's existing room types, by code or by name. \
         A requirement, not a booking."),
];

/// Class sections of a course.
static CLASS_SECTIONS: &[TemplateColumn] = &[
    column!("courseCode", "Course Code", required: true, code: true, example: "9MAT",
        aka: ["Course", "Course Name"],
        "Required. Must name one of this school's existing courses, by code or by name."),
    column!("name", "Section Name", required: true, code: false, example: "A",
        aka: ["Section", "Class", "Class Name", "Set"],
        "Required. Unique within the course."),
    column!("code", "Section Code", required: false, code: true, example: "9MAT-A",
        aka: ["Class Code", "Section ID"],
        "Optional but recommended: the code is what the class-teacher, class-group and \
         enrolment imports refer to. Unique within the course."),
    column!("campusCode", "Campus", required: false, code: true, example: "",
        aka: ["Campus Code", "Site"],
        "Optional. One of this school's existing campus codes. Setting it means only \
         teachers eligible at that campus can be given this section."),
];

/// Which teachers teach which class sections.
static CLASS_TEACHERS: &[TemplateColumn] = &[
    column!("courseCode", "Course Code", required: true, code: true, example: "9MAT",
        aka: ["Course", "Course Name"],
        "Required. Must name one of this school's existing courses."),
    column!("sectionCode", "Section Code", required: true, code: true, example: "9MAT-A",
        aka: ["Section", "Class", "Section Name", "Class Code"],
        "Required. Must name an existing section of that course, by code or by name."),
    column!("teacherCode", "Teacher Code", required: true, code: true, example: "T-0007",
        aka: ["Staff ID", "Teacher", "Staff Code"],
        "Required. Must name a teacher this school actively employs. A teacher without a \
         recorded qualification for the subject is a warning, not a refusal."),
];

/// Which student groups attend which class sections.
static CLASS_GROUPS: &[TemplateColumn] = &[
    column!("courseCode", "Course Code", required: true, code: true, example: "9MAT",
        aka: ["Course", "Course Name"],
        "Required. Must name one of this school's existing courses."),
    column!("sectionCode", "Section Code", required: true, code: true, example: "9MAT-A",
        aka: ["Section", "Class", "Section Name", "Class Code"],
        "Required. Must name an existing section of that course."),
    column!("groupCode", "Student Group Code", required: true, code: true, example: "9A",
        aka: ["Group", "Student Group", "Group Code", "Class Group"],
        "Required. Must name one of this school's existing student groups, by code or by \
         name. A group's declared size is the audience a timetable is built around."),
];

/// Which named individuals are on a class section's roster.
static CLASS_ENROLMENTS: &[TemplateColumn] = &[
    column!("courseCode", "Course Code", required: true, code: true, example: "9MAT",
        aka: ["Course", "Course Name"],
        "Required. Must name one of this school's existing courses."),
    column!("sectionCode", "Section Code", required: true, code: true, example: "9MAT-A",
        aka: ["Section", "Class", "Section Name", "Class Code"],
        "Required. Must name an existing section of that course."),
    column!("studentCode", "Student Code", required: true, code: true, example: "00124",
        aka: ["Student ID", "Admission Number", "Student"],
        "Required. Must name a student currently enrolled at this school. This roster is \
         optional throughout Evara and is never added to the attending groups' size."),
];

/// How often a class section meets.
static ACTIVITIES: &[TemplateColumn] = &[
    column!("courseCode", "Course Code", required: true, code: true, example: "9MAT",
        aka: ["Course", "Course Name"],
        "Required. Must name one of this school's existing courses."),
    column!("sectionCode", "Section Code", required: true, code: true, example: "9MAT-A",
        aka: ["Section", "Class", "Section Name", "Class Code"],
        "Required. Must name an existing section of that course."),
    column!("durationPeriods", "Periods per Lesson", required: true, code: false, example: "1",
        aka: ["Duration", "Periods", "Length", "Duration in Periods"],
        "Required. Consecutive periods one lesson needs — 1 for a single, 2 for a double. \
         A double is one lesson that cannot be split, not two lessons."),
    column!("occurrences", "Occurrences per Cycle", required: false, code: false, example: "4",
        aka: ["Times per Cycle", "Occurrences", "Lessons per Cycle", "Count"],
        "Optional, default 1. How many lessons of that length the cycle needs. This is a \
         convenience: Evara stores one record per lesson, so 4 here creates four records. \
         The preview shows the expansion before anything is written."),
    column!("name", "Lesson Label", required: false, code: false, example: "",
        aka: ["Label", "Activity Name", "Name"],
        "Optional. Only where a lesson needs its own name — “Laboratory”, “Double”. Most \
         do not."),
];

#[cfg(test)]
mod tests {
    use super::{TemplateFormat, accepted_headings, template_bytes, template_columns};
    use evara_core::import::{ImportKind, SourceCell};

    #[test]
    fn every_kind_has_columns_and_at_least_one_required_field() {
        for kind in ImportKind::ALL {
            let columns = template_columns(*kind);
            assert!(!columns.is_empty(), "{kind:?} has no columns");
            assert!(
                columns.iter().any(|column| column.required),
                "{kind:?} has no required column, so an empty mapping would be valid"
            );
        }
    }

    #[test]
    fn a_template_maps_itself() {
        // The most embarrassing way this feature could fail: a file Evara generated that
        // Evara cannot map. Asserted for all ten kinds rather than spot-checked.
        for kind in ImportKind::ALL {
            let csv = template_bytes(*kind, TemplateFormat::Csv).expect("a CSV template");
            let text = String::from_utf8(csv).expect("templates are UTF-8");
            let headers: Vec<String> = text
                .lines()
                .next()
                .expect("a heading row")
                .split(',')
                .map(|value| value.trim().to_owned())
                .collect();

            let mapping = crate::suggest_mapping(*kind, &headers);
            for column in template_columns(*kind) {
                assert!(
                    mapping.column_of(column.key).is_some(),
                    "{kind:?}: the template writes “{}” and the mapper does not recognise it",
                    column.heading
                );
            }
        }
    }

    #[test]
    fn a_templates_example_row_round_trips_through_the_parser() {
        // A second, stronger form of the same guarantee: the generated file parses, and
        // the example row lands under the fields it was written for.
        for kind in ImportKind::ALL {
            let csv = template_bytes(*kind, TemplateFormat::Csv).expect("a CSV template");
            let name = super::template_file_name(*kind, TemplateFormat::Csv);
            let table =
                crate::read(&name, &csv, None, crate::Limits::default()).unwrap_or_else(|error| {
                    panic!("{kind:?}: its own template failed to parse: {error}")
                });
            assert_eq!(table.rows.len(), 1, "{kind:?}: one example row");

            let mapping = crate::suggest_mapping(*kind, &table.headers);
            for column in template_columns(*kind).iter().filter(|c| c.required) {
                let index = mapping.column_of(column.key).expect("mapped");
                let cell = &table.rows[0].cells[index];
                assert!(
                    !cell.is_blank(),
                    "{kind:?}: the example row leaves required column “{}” blank",
                    column.heading
                );
            }
        }
    }

    #[test]
    fn a_code_columns_example_is_readable_as_a_code() {
        // Which is to say: the example itself must not be the mistake the import refuses.
        for kind in ImportKind::ALL {
            for column in template_columns(*kind).iter().filter(|c| c.is_code) {
                if column.example.is_empty() {
                    continue;
                }
                let cell = SourceCell::Text(column.example.to_owned());
                assert!(
                    cell.code().is_ok(),
                    "{kind:?}: “{}” example is not a usable code",
                    column.heading
                );
            }
        }
    }

    #[test]
    fn no_two_columns_of_one_kind_share_a_key_or_a_heading() {
        for kind in ImportKind::ALL {
            let columns = template_columns(*kind);
            let mut keys: Vec<&str> = columns.iter().map(|c| c.key).collect();
            keys.sort_unstable();
            let count = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), count, "{kind:?} repeats a field key");

            let mut headings: Vec<&str> = columns.iter().map(|c| c.heading).collect();
            headings.sort_unstable();
            let count = headings.len();
            headings.dedup();
            assert_eq!(headings.len(), count, "{kind:?} repeats a heading");
        }
    }

    #[test]
    fn no_alias_of_one_kind_collides_with_another_fields_heading() {
        // An alias that matched a second field's own heading would make the suggestion
        // order decide which field got the column, which is exactly the silent mis-mapping
        // `ColumnMapping::suggest` is conservative to avoid.
        for kind in ImportKind::ALL {
            let columns = template_columns(*kind);
            for column in columns {
                for alias in column.also_known_as {
                    let clash = columns.iter().find(|other| {
                        other.key != column.key && other.heading.eq_ignore_ascii_case(alias)
                    });
                    assert!(
                        clash.is_none(),
                        "{kind:?}: “{alias}” is an alias of {} and the heading of {}",
                        column.key,
                        clash.map_or("", |c| c.key)
                    );
                }
            }
        }
    }

    #[test]
    fn the_accepted_headings_come_from_the_template_itself() {
        for kind in ImportKind::ALL {
            let accepted = accepted_headings(*kind);
            assert_eq!(accepted.len(), template_columns(*kind).len());
            for (column, (key, headings)) in template_columns(*kind).iter().zip(accepted) {
                assert_eq!(column.key, *key);
                assert_eq!(headings[0], column.heading, "its own heading comes first");
            }
        }
    }

    #[test]
    fn a_workbook_template_is_a_zip_and_names_its_sheet() {
        let bytes = template_bytes(ImportKind::Teachers, TemplateFormat::Xlsx)
            .expect("a workbook template");
        assert!(
            bytes.starts_with(b"PK\x03\x04"),
            "an xlsx is a ZIP container"
        );
        // And it reads back through our own parser, which is the only check that matters.
        let table = crate::read(
            "evara-teachers-template.xlsx",
            &bytes,
            None,
            crate::Limits::default(),
        )
        .expect("our own workbook parses");
        assert_eq!(table.sheet, "Teachers");
        assert!(
            table.available_sheets.len() == 2,
            "the data sheet and the instructions"
        );
        assert_eq!(table.rows.len(), 1, "one example row");
    }

    #[test]
    fn a_workbook_templates_code_column_survives_as_text() {
        // The entire reason the templates are workbooks. `00124` written into a text-
        // formatted column must come back as text, not as the number 124.
        let bytes = template_bytes(ImportKind::Students, TemplateFormat::Xlsx)
            .expect("a workbook template");
        let table = crate::read("t.xlsx", &bytes, None, crate::Limits::default()).expect("parses");
        let mapping = crate::suggest_mapping(ImportKind::Students, &table.headers);
        let index = mapping
            .column_of("code")
            .expect("the code column is mapped");
        assert_eq!(
            table.rows[0].cells[index]
                .code()
                .expect("readable as a code"),
            Some("00124"),
            "the leading zeros must survive our own template"
        );
    }

    #[test]
    fn the_template_file_name_says_what_it_is() {
        assert_eq!(
            super::template_file_name(ImportKind::Teachers, TemplateFormat::Xlsx),
            "evara-teachers-template.xlsx"
        );
        assert_eq!(
            super::template_file_name(ImportKind::Activities, TemplateFormat::Csv),
            "evara-activities-template.csv"
        );
    }

    #[test]
    fn the_activity_template_explains_that_occurrences_expand() {
        // ADR 0029 made an occurrence a row. The template's convenience column must say so
        // rather than implying Evara stores a count.
        let occurrences = template_columns(ImportKind::Activities)
            .iter()
            .find(|column| column.key == "occurrences")
            .expect("the occurrences column");
        assert!(occurrences.hint.contains("one record per lesson"));
        assert!(occurrences.hint.contains("preview"));
    }

    #[test]
    fn every_column_hint_is_a_sentence_about_what_happens_if_it_is_left_out() {
        for kind in ImportKind::ALL {
            for column in template_columns(*kind) {
                assert!(
                    column.hint.len() > 12,
                    "{kind:?}/{}: the hint has to say something",
                    column.key
                );
                if !column.required {
                    assert!(
                        column.hint.contains("Optional")
                            || column.hint.contains("optional")
                            || column.hint.contains("Blank")
                            || column.hint.contains("blank"),
                        "{kind:?}/{}: an optional column must say it is optional",
                        column.key
                    );
                }
            }
        }
    }
}
