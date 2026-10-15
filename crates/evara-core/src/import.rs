//! The vocabulary of a spreadsheet import: what a source table is, what a plan proposes,
//! and the per-cell rules that need no database to decide.
//!
//! # Why this is at the bottom of the stack
//!
//! Four crates have to agree on these words. `evara-import` produces a [`SourceTable`] and
//! knows nothing about schools; `evara-db` resolves one into an [`ImportPlan`] and owns
//! every rule that needs a query; the command layer carries both across IPC; the interface
//! renders the plan. A type three of them pass around belongs under all of them, which is
//! the same reason [`crate::lessons::LessonDemand`] is here.
//!
//! It also keeps the parser out of `evara-db`. The crate that owns SQL has no business
//! knowing what a worksheet is, and the crate that reads worksheets has no business
//! knowing what a school is — see
//! [ADR 0030](../../../docs/adr/0030-an-import-is-a-reviewed-plan.md) §2.
//!
//! # Everything here is pure
//!
//! No I/O, no parser, no database. The rules in this module are the ones a cell can be
//! judged by on its own — *is this blank, is it a number where a code was wanted, is this
//! date in the one format we accept* — and nothing more. Every rule that needs to know
//! what the project already holds lives in `evara_db::import`, because it is a question
//! about the database.
//!
//! # A code is text, and stays text
//!
//! The single most important rule in the file, and the one a spreadsheet breaks by
//! default. `00124` is a student number, not the integer 124; `T-0007` is a staff code.
//! [`SourceCell`] therefore keeps the *text* a cell held alongside whatever the parser
//! made of it, and [`SourceCell::code`] reads the text. A number that reached a code column
//! is reported rather than coerced, because `124` and `00124` are different schools'
//! answers and guessing which was meant is how an import corrupts a roster.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ============================================================================== source

/// What one cell of a spreadsheet held.
///
/// Carries the **text** as well as the interpretation, because the two disagree exactly
/// where it matters. A cell holding `00124` arrives from a `.csv` as the string `00124`
/// and from an `.xlsx` as the float `124` with a display format — so the text is the only
/// thing that can be trusted for an identifier, and the number is the only thing that can
/// be trusted for a count.
// No `Eq`: a cell can hold an `f64`. `PartialEq` is all a test or a change check needs,
// and deriving `Eq` would mean rounding a number to get it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum SourceCell {
    /// Nothing in the cell, or only whitespace.
    Blank,
    /// Text, already trimmed of surrounding whitespace.
    Text(String),
    /// A number, with the text the sheet displayed for it.
    ///
    /// Both halves are kept. `Number { text: "00124", .. }` cannot happen from a
    /// spreadsheet — a numeric cell has no leading zeros to display — which is precisely
    /// why a numeric cell in a code column is a finding rather than a value.
    Number {
        /// What the sheet displayed, as text.
        text: String,
        /// The value, for the columns that want a count.
        number: f64,
    },
    /// `TRUE`/`FALSE`, however the sheet spelled it.
    Boolean(bool),
    /// A date, already rendered as ISO-8601 `YYYY-MM-DD` by the parser.
    ///
    /// Only a cell the *sheet* says is a date reaches this. A text cell that merely looks
    /// like one stays [`SourceCell::Text`] and is parsed by [`iso_date`], which accepts one
    /// format and refuses to guess between the others.
    Date(String),
    /// A cell the parser could not represent, with why.
    ///
    /// A formula is the common case: nothing here evaluates one, and a formula cell with no
    /// cached value has no value to read. Reported rather than treated as blank, because
    /// blank means "the school left this out" and this means "we could not tell".
    Unsupported(String),
}

impl SourceCell {
    /// Whether the cell is empty as far as an import is concerned.
    #[must_use]
    pub const fn is_blank(&self) -> bool {
        matches!(self, Self::Blank)
    }

    /// The cell as a trimmed string, for a free-text column.
    ///
    /// A number becomes its displayed text, which is right for a name or a description and
    /// wrong for a code — [`SourceCell::code`] is the one to use there.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            // A boolean has no text to carry into a name or a description, which is what
            // this reader is for. `flag` is the one that reads it.
            Self::Blank | Self::Unsupported(_) | Self::Boolean(_) => None,
            Self::Text(value) | Self::Number { text: value, .. } | Self::Date(value) => {
                Some(value.as_str())
            }
        }
    }

    /// The cell as an identifier, refusing a numeric cell.
    ///
    /// `Err` carries the reason, which the validator turns into a row finding. A number
    /// where a code was wanted is a *real* problem and not a formatting nicety: Excel
    /// stores `00124` as the number 124 unless the column was formatted as text, so the
    /// leading zeros are already gone by the time the file reaches us and no amount of
    /// re-padding can recover how many there were. Saying so is the only honest answer, and
    /// it is why the downloadable templates format their code columns as text.
    pub fn code(&self) -> Result<Option<&str>, CellProblem> {
        match self {
            Self::Blank => Ok(None),
            Self::Text(value) => Ok(Some(value.as_str())),
            Self::Number { text, .. } => Err(CellProblem::NumericCode {
                shown: text.clone(),
            }),
            Self::Boolean(_) => Err(CellProblem::WrongType {
                wanted: "a code",
                found: "TRUE/FALSE",
            }),
            Self::Date(_) => Err(CellProblem::WrongType {
                wanted: "a code",
                found: "a date",
            }),
            Self::Unsupported(reason) => Err(CellProblem::Unsupported {
                reason: reason.clone(),
            }),
        }
    }

    /// The cell as a whole number, for a count or a duration.
    pub fn whole_number(&self) -> Result<Option<i64>, CellProblem> {
        match self {
            Self::Blank => Ok(None),
            Self::Number { number, text } => {
                if number.fract().abs() > f64::EPSILON {
                    return Err(CellProblem::NotAWholeNumber {
                        shown: text.clone(),
                    });
                }
                // The bound is generous and exists only so a nonsense cell cannot become a
                // nonsense row count. Nothing in a school roster is near it.
                if !(-1.0e9..=1.0e9).contains(number) {
                    return Err(CellProblem::NotAWholeNumber {
                        shown: text.clone(),
                    });
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "bounded above and checked for a fractional part"
                )]
                Ok(Some(*number as i64))
            }
            Self::Text(value) => {
                value
                    .parse::<i64>()
                    .map(Some)
                    .map_err(|_| CellProblem::NotAWholeNumber {
                        shown: value.clone(),
                    })
            }
            Self::Boolean(_) | Self::Date(_) => Err(CellProblem::WrongType {
                wanted: "a whole number",
                found: match self {
                    Self::Boolean(_) => "TRUE/FALSE",
                    _ => "a date",
                },
            }),
            Self::Unsupported(reason) => Err(CellProblem::Unsupported {
                reason: reason.clone(),
            }),
        }
    }

    /// The cell as a yes/no answer.
    ///
    /// Accepts the spellings a school actually types, and refuses everything else rather
    /// than treating an unrecognised word as `false` — "Actve" should be a finding, not a
    /// silent withdrawal of somebody's employment.
    pub fn flag(&self) -> Result<Option<bool>, CellProblem> {
        match self {
            Self::Blank => Ok(None),
            Self::Boolean(value) => Ok(Some(*value)),
            Self::Number { number, text } => match *number {
                n if (n - 1.0).abs() < f64::EPSILON => Ok(Some(true)),
                n if n.abs() < f64::EPSILON => Ok(Some(false)),
                _ => Err(CellProblem::NotAFlag {
                    shown: text.clone(),
                }),
            },
            Self::Text(value) => match value.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" | "true" | "t" | "1" | "active" => Ok(Some(true)),
                "n" | "no" | "false" | "f" | "0" | "inactive" => Ok(Some(false)),
                _ => Err(CellProblem::NotAFlag {
                    shown: value.clone(),
                }),
            },
            Self::Date(_) => Err(CellProblem::WrongType {
                wanted: "yes or no",
                found: "a date",
            }),
            Self::Unsupported(reason) => Err(CellProblem::Unsupported {
                reason: reason.clone(),
            }),
        }
    }

    /// The cell as an ISO-8601 calendar date.
    pub fn date(&self) -> Result<Option<String>, CellProblem> {
        match self {
            Self::Blank => Ok(None),
            Self::Date(value) => Ok(Some(value.clone())),
            Self::Text(value) => iso_date(value).map(Some),
            Self::Number { text, .. } => Err(CellProblem::AmbiguousDate {
                shown: text.clone(),
            }),
            Self::Boolean(_) => Err(CellProblem::WrongType {
                wanted: "a date",
                found: "TRUE/FALSE",
            }),
            Self::Unsupported(reason) => Err(CellProblem::Unsupported {
                reason: reason.clone(),
            }),
        }
    }
}

/// Why one cell could not be read as the column wanted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CellProblem {
    /// A numeric cell in a column that holds identifiers.
    NumericCode {
        /// What the sheet displayed.
        shown: String,
    },
    /// Not a whole number where one was wanted.
    NotAWholeNumber {
        /// What the sheet displayed.
        shown: String,
    },
    /// Not a recognised yes/no spelling.
    NotAFlag {
        /// What the sheet displayed.
        shown: String,
    },
    /// A date Evara will not guess at.
    AmbiguousDate {
        /// What the sheet displayed.
        shown: String,
    },
    /// The wrong kind of cell altogether.
    WrongType {
        /// What the column wanted.
        wanted: &'static str,
        /// What the cell held.
        found: &'static str,
    },
    /// The parser could not represent the cell.
    Unsupported {
        /// Why.
        reason: String,
    },
}

impl CellProblem {
    /// What to show the user, naming the remedy where there is one.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NumericCode { shown } => format!(
                "“{shown}” arrived as a number, so any leading zeros are already gone and \
                 cannot be recovered. Format this column as text in the spreadsheet and \
                 export again — the downloadable template already does."
            ),
            Self::NotAWholeNumber { shown } => {
                format!("“{shown}” is not a whole number.")
            }
            Self::NotAFlag { shown } => format!(
                "“{shown}” is not a yes or no. Use Yes/No, TRUE/FALSE, or leave it blank \
                 for the default."
            ),
            Self::AmbiguousDate { shown } => format!(
                "“{shown}” could be more than one date, so Evara will not guess. Use \
                 {DATE_FORMAT_HINT}, or format the column as a date in the spreadsheet."
            ),
            Self::WrongType { wanted, found } => {
                format!("this column wants {wanted} and the cell holds {found}.")
            }
            Self::Unsupported { reason } => reason.clone(),
        }
    }
}

/// The one text date format an import accepts.
pub const DATE_FORMAT_HINT: &str = "YYYY-MM-DD";

/// Reads a calendar date from text, accepting exactly one format.
///
/// `YYYY-MM-DD` and nothing else. The alternative is to guess between `03/04/2026` as the
/// third of April and the fourth of March, and the two are both ordinary in the schools
/// this product serves — so a wrong guess silently files a leaver's date a month out. A
/// cell the *sheet* itself typed as a date never reaches here: the parser renders those
/// from the workbook's own serial number, where there is no ambiguity to resolve.
pub fn iso_date(value: &str) -> Result<String, CellProblem> {
    let text = value.trim();
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if !shaped {
        return Err(CellProblem::AmbiguousDate {
            shown: text.to_owned(),
        });
    }

    let month: u32 = text[5..7].parse().unwrap_or(0);
    let day: u32 = text[8..10].parse().unwrap_or(0);
    let year: i32 = text[0..4].parse().unwrap_or(0);
    if !is_real_date(year, month, day) {
        return Err(CellProblem::AmbiguousDate {
            shown: text.to_owned(),
        });
    }
    Ok(text.to_owned())
}

/// Whether a `YYYY-MM-DD` triple names a day that exists.
///
/// Spelled out rather than taken from a date library: this crate carries no date type on
/// purpose, and the whole question is four lines.
fn is_real_date(year: i32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap => 29,
        _ => 28,
    };
    day <= length
}

/// One row of a source file, by its position in the sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SourceRow {
    /// The row's number **as the spreadsheet shows it**, counting the header as row 1.
    ///
    /// Not an index. Every message a user reads names this, because "row 14" has to mean
    /// the row they can see when they open the file — an off-by-one here sends somebody to
    /// correct the wrong line.
    pub line: i64,
    /// The cells, one per column of [`SourceTable::headers`], padded where the sheet was
    /// short.
    pub cells: Vec<SourceCell>,
}

/// A parsed file, ready to be mapped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SourceTable {
    /// The worksheet this came from, or the file name for a CSV.
    pub sheet: String,
    /// Every worksheet the file holds, so the interface can offer the others.
    pub available_sheets: Vec<String>,
    /// The header row, trimmed. Empty headers are kept so column positions still line up.
    pub headers: Vec<String>,
    /// The data rows, header excluded.
    pub rows: Vec<SourceRow>,
    /// Rows the parser stopped at, when the file was longer than the row limit.
    ///
    /// Reported rather than silently truncated: an import that quietly dropped the last
    /// 400 students would be the worst possible failure of this feature.
    pub truncated_at: Option<i64>,
}

impl SourceTable {
    /// How many columns the table has.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.headers.len()
    }

    /// How many data rows it has.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

// ============================================================================= mapping

/// One field an import can fill, as the interface offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FieldSpec {
    /// Stable key the mapping names. Never a position, never a label.
    pub key: &'static str,
    /// What the user reads.
    pub label: &'static str,
    /// Whether a mapping is required before the plan can be built.
    pub required: bool,
    /// One line on what the column holds and what happens if it is left out.
    pub hint: &'static str,
}

/// Which source column fills which field.
///
/// Keyed by [`FieldSpec::key`], valued by the column's **index** in
/// [`SourceTable::headers`]. An index rather than a heading, because two columns may share
/// a heading and a heading may be blank — and because the user is allowed to override a
/// suggestion, at which point the heading stops being the answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ColumnMapping(pub BTreeMap<String, usize>);

impl ColumnMapping {
    /// The column index a field is mapped to, if any.
    #[must_use]
    pub fn column_of(&self, field: &str) -> Option<usize> {
        self.0.get(field).copied()
    }

    /// Suggests a mapping by matching headings against each field's accepted names.
    ///
    /// Deliberately conservative: it matches on a normalised form — lowercase, with
    /// everything but letters and digits removed — so `Staff ID`, `staff_id` and `StaffID`
    /// all find the same field, and nothing else does. A fuzzy match would be worse than
    /// none, because a wrong suggestion the user does not notice is how a column of
    /// surnames ends up in the staff-code field.
    ///
    /// The first unclaimed column wins, so a file with two `Code` columns maps the leftmost
    /// and leaves the other for the user to assign.
    #[must_use]
    pub fn suggest(headers: &[String], fields: &[(&'static str, &[&'static str])]) -> Self {
        let normalised: Vec<String> = headers.iter().map(|h| normalise_heading(h)).collect();
        let mut taken = vec![false; headers.len()];
        let mut mapping = BTreeMap::new();

        for (key, accepted) in fields {
            let found = normalised.iter().enumerate().find(|(index, heading)| {
                !taken[*index]
                    && !heading.is_empty()
                    && accepted
                        .iter()
                        .any(|name| normalise_heading(name) == **heading)
            });
            if let Some((index, _)) = found {
                taken[index] = true;
                mapping.insert((*key).to_owned(), index);
            }
        }
        Self(mapping)
    }

    /// Fields mapped to the same column, which is always a mistake.
    ///
    /// Reported rather than resolved: one column cannot be both a first name and a code,
    /// and picking one for the user would file half a roster under the wrong field.
    #[must_use]
    pub fn duplicated_columns(&self) -> Vec<(usize, Vec<String>)> {
        let mut by_column: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (field, column) in &self.0 {
            by_column.entry(*column).or_default().push(field.clone());
        }
        by_column
            .into_iter()
            .filter(|(_, fields)| fields.len() > 1)
            .collect()
    }
}

/// Lowercases a heading and strips everything that is not a letter or a digit.
fn normalise_heading(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

// ================================================================================ plan

/// What an import proposes to do with one source row.
///
/// Six outcomes, and the distinctions between them are the whole value of a preview. In
/// particular **`Unchanged` is not `Skipped`**: the first means the project already says
/// what the row says, which is what a second run of the same file should produce; the
/// second means the user or a policy chose to leave the row alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RowAction {
    /// Nothing matches; the row would create a record.
    Create,
    /// A record matches and the row would change it.
    Update,
    /// A record matches and already says exactly this.
    Unchanged,
    /// Deliberately left alone, by policy or by the user.
    Skipped,
    /// A record matches and disagrees, and the user has not said what to do.
    Conflict,
    /// The row cannot be acted on at all.
    Invalid,
}

impl RowAction {
    /// Whether this outcome stops the whole import.
    ///
    /// Only the two that mean "a human has not decided yet". A `Skipped` row has been
    /// decided — to do nothing — and `Unchanged` needs no decision.
    #[must_use]
    pub const fn blocks_apply(self) -> bool {
        matches!(self, Self::Invalid | Self::Conflict)
    }

    /// Whether applying the plan writes anything for this row.
    #[must_use]
    pub const fn writes(self) -> bool {
        matches!(self, Self::Create | Self::Update)
    }
}

/// How serious a finding about a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Severity {
    /// Worth saying; does not stop the import. ADR 0028's unqualified teacher is one.
    Warning,
    /// Stops the import until it is resolved.
    Error,
}

/// One thing to say about one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RowFinding {
    /// How serious.
    pub severity: Severity,
    /// The field it is about, when it is about one.
    pub field: Option<String>,
    /// What to tell the user, naming the remedy.
    pub message: String,
}

impl RowFinding {
    /// An error about one field.
    #[must_use]
    pub fn error(field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            field: Some(field.to_owned()),
            message: message.into(),
        }
    }

    /// An error about the row as a whole.
    #[must_use]
    pub fn row_error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            field: None,
            message: message.into(),
        }
    }

    /// A warning about the row as a whole.
    #[must_use]
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            field: None,
            message: message.into(),
        }
    }
}

/// What a user decided about one row the planner could not decide alone.
///
/// Sent back with the mapping so the plan is recomputed *with* the decision rather than
/// patched after the fact. That matters for the fingerprint: a decided row has a different
/// outcome, so the plan the user approves is the plan the apply path recomputes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RowDecision {
    /// The row's line in the sheet, as the preview showed it.
    pub line: i64,
    /// What to do with it.
    pub choice: Choice,
}

/// The ways out of a [`RowAction::Conflict`].
///
/// Deliberately explicit rather than a global "overwrite everything" switch: a file of two
/// hundred rows routinely has three the school wants to update and one it does not, and a
/// single policy for the whole import would make the user choose between losing an edit and
/// abandoning the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Choice {
    /// Write the file's values over the record Evara holds.
    Update,
    /// Leave the existing record exactly as it is. The row becomes `Skipped`.
    KeepExisting,
    /// Do nothing with this row at all. Also `Skipped`, and said differently on purpose:
    /// "I do not want this row" is not "I want the record left alone".
    Skip,
    /// Treat the match as a different person or thing, and create a new record.
    ///
    /// Only meaningful where the conflict was about *identity* — two people with the same
    /// name — and refused elsewhere, because "create another one" is not an answer to
    /// "this subject code already exists with a different name".
    CreateNew,
}

impl Choice {
    /// The outcome a decided row ends up with.
    #[must_use]
    pub const fn outcome(self) -> RowAction {
        match self {
            Self::Update => RowAction::Update,
            Self::KeepExisting | Self::Skip => RowAction::Skipped,
            Self::CreateNew => RowAction::Create,
        }
    }
}

/// What the plan says about one source row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlannedRow {
    /// The row's line in the sheet, as the user sees it.
    pub line: i64,
    /// What would happen.
    pub action: RowAction,
    /// How the row identifies itself, for the preview and the report: a code, or a name.
    pub source_label: String,
    /// What it resolved to, when it resolved to something that already exists.
    pub target_label: Option<String>,
    /// Everything worth saying about it.
    pub findings: Vec<RowFinding>,
}

impl PlannedRow {
    /// Whether this row has an error.
    #[must_use]
    pub fn has_error(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }
}

/// How many rows fall into each outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlanCounts {
    /// Rows that would create a record.
    pub create: i64,
    /// Rows that would change one.
    pub update: i64,
    /// Rows the project already agrees with.
    pub unchanged: i64,
    /// Rows deliberately left alone.
    pub skipped: i64,
    /// Rows waiting on a decision.
    pub conflict: i64,
    /// Rows that cannot be acted on.
    pub invalid: i64,
    /// Findings that do not stop the import.
    pub warnings: i64,
}

impl PlanCounts {
    /// Every row the plan considered.
    #[must_use]
    pub const fn total(&self) -> i64 {
        self.create + self.update + self.unchanged + self.skipped + self.conflict + self.invalid
    }

    /// Whether anything is waiting on a human.
    #[must_use]
    pub const fn blocked(&self) -> bool {
        self.conflict > 0 || self.invalid > 0
    }
}

/// A reviewed, immutable proposal.
///
/// Produced by `evara_db::import` and sent to the interface. Building one **writes
/// nothing**: that is the property the whole staged workflow rests on, and
/// `tests_import::previewing_writes_nothing_at_all` asserts it over the change feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportPlan {
    /// What is being imported.
    pub kind: ImportKind,
    /// The school every record lands in.
    pub school_id: uuid::Uuid,
    /// One entry per source row, in sheet order.
    pub rows: Vec<PlannedRow>,
    /// The tally.
    pub counts: PlanCounts,
    /// Agreement token, recomputed on apply. See [`ImportFingerprint`].
    pub fingerprint: ImportFingerprint,
}

impl ImportPlan {
    /// Whether Apply may be offered.
    #[must_use]
    pub const fn can_apply(&self) -> bool {
        !self.counts.blocked()
    }
}

/// A token for "the plan you were shown is still the plan".
///
/// The same mechanism ADR 0011 gave the timeslot grid, and for the same reason: a user may
/// preview a file, switch to another window, change the project, and come back to press
/// Apply. The apply path **recomputes the plan from the current database** and refuses if
/// the fingerprint has moved, so an outdated preview can never be applied — it is not a
/// substitute for that recomputation but the thing that makes its disagreement reportable.
///
/// FNV-1a over a canonical, order-independent encoding of every row's outcome. Not a
/// security primitive: a forged token can only cause a spurious re-review, never an
/// unreviewed write.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportFingerprint(String);

impl ImportFingerprint {
    /// The fingerprint as it travels to the interface and back.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Computes one over a plan's rows.
    ///
    /// Material: the kind, the school, and every row's line, action and resolved target.
    /// **Not** the findings' wording, which is presentation — a reworded message must not
    /// invalidate a plan a user has already reviewed.
    #[must_use]
    pub fn of(kind: ImportKind, school_id: uuid::Uuid, rows: &[PlannedRow]) -> Self {
        let mut lines: Vec<String> = rows
            .iter()
            .map(|row| {
                format!(
                    "{}:{:?}:{}",
                    row.line,
                    row.action,
                    row.target_label.as_deref().unwrap_or("-")
                )
            })
            .collect();
        lines.sort_unstable();

        let mut payload = format!("{kind:?}|{school_id}|");
        for line in lines {
            payload.push_str(&line);
            payload.push('\n');
        }
        Self(format!("{:016x}", fnv1a(payload.as_bytes())))
    }
}

impl std::fmt::Display for ImportFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// FNV-1a, 64-bit.
///
/// Spelled out rather than taken from `DefaultHasher` for the reason `time_model`'s copy
/// gives: this value crosses the IPC boundary and comes back, so it has to mean the same
/// thing on both sides of a round trip, and `DefaultHasher`'s output is explicitly not
/// stable between Rust releases.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// What the import applied, once it had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportSummary {
    /// What was imported.
    pub kind: ImportKind,
    /// How many records were created.
    pub created: i64,
    /// How many were changed.
    pub updated: i64,
    /// How many were already as the file said.
    pub unchanged: i64,
    /// How many were left alone.
    pub skipped: i64,
    /// How many warnings the plan carried.
    pub warnings: i64,
    /// Every row the file held.
    pub rows_processed: i64,
    /// The rows, so a report can be exported without re-reading the file.
    pub rows: Vec<PlannedRow>,
}

// =============================================================================== kinds

/// What a file is being imported as.
///
/// Ten kinds, grouped in the interface by the four problems they actually pose: who the
/// people are, what the school teaches, who teaches and attends a class, and how often it
/// meets. One engine serves all of them — see
/// [ADR 0030](../../../docs/adr/0030-an-import-is-a-reviewed-plan.md) §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ImportKind {
    /// Teacher identities and this school's employment of them.
    Teachers,
    /// Student identities and this school's enrolment of them.
    Students,
    /// Departments of this school.
    Departments,
    /// Subjects of this school.
    Subjects,
    /// Courses, as offerings of this school's subjects.
    Courses,
    /// Class sections of this school's courses.
    ClassSections,
    /// Which teachers teach which class sections.
    ClassTeachers,
    /// Which student groups attend which class sections.
    ClassGroups,
    /// Which named individuals are on a class section's roster.
    ClassEnrolments,
    /// How often a class section meets, expanded into one activity per occurrence.
    Activities,
}

impl ImportKind {
    /// Every kind, in the order the interface offers them.
    ///
    /// Dependency order, which is also the order a school would run them in: people and
    /// the curriculum before the classes that reference both.
    pub const ALL: &'static [Self] = &[
        Self::Teachers,
        Self::Students,
        Self::Departments,
        Self::Subjects,
        Self::Courses,
        Self::ClassSections,
        Self::ClassTeachers,
        Self::ClassGroups,
        Self::ClassEnrolments,
        Self::Activities,
    ];

    /// What the user reads.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Teachers => "Teachers",
            Self::Students => "Students",
            Self::Departments => "Departments",
            Self::Subjects => "Subjects",
            Self::Courses => "Courses",
            Self::ClassSections => "Class sections",
            Self::ClassTeachers => "Class teachers",
            Self::ClassGroups => "Class groups",
            Self::ClassEnrolments => "Class enrolments",
            Self::Activities => "Lesson occurrences",
        }
    }

    /// A file name stem for the downloadable template.
    #[must_use]
    pub const fn template_stem(self) -> &'static str {
        match self {
            Self::Teachers => "evara-teachers",
            Self::Students => "evara-students",
            Self::Departments => "evara-departments",
            Self::Subjects => "evara-subjects",
            Self::Courses => "evara-courses",
            Self::ClassSections => "evara-class-sections",
            Self::ClassTeachers => "evara-class-teachers",
            Self::ClassGroups => "evara-class-groups",
            Self::ClassEnrolments => "evara-class-enrolments",
            Self::Activities => "evara-activities",
        }
    }

    /// What this kind must have before it can run, in the user's words.
    ///
    /// Not a database check — the planner does that per row. This is the sentence the
    /// type picker shows, so a school running them out of order is told why the file it
    /// just chose is going to be all errors.
    #[must_use]
    pub const fn depends_on(self) -> Option<&'static str> {
        match self {
            Self::Teachers | Self::Students | Self::Departments => None,
            Self::Subjects => Some("departments, if a subject names one"),
            Self::Courses => Some("subjects"),
            Self::ClassSections => Some("courses"),
            Self::ClassTeachers => Some("class sections, and teachers employed here"),
            Self::ClassGroups => Some("class sections, and this school's student groups"),
            Self::ClassEnrolments => Some("class sections, and students enrolled here"),
            Self::Activities => Some("class sections"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CellProblem, ColumnMapping, ImportKind, PlanCounts, RowAction, SourceCell, iso_date,
    };

    #[test]
    fn a_leading_zero_code_survives_as_text() {
        let cell = SourceCell::Text("00124".to_owned());
        assert_eq!(cell.code().expect("a text code is readable"), Some("00124"));
    }

    #[test]
    fn a_numeric_cell_in_a_code_column_is_refused_rather_than_padded() {
        // The whole of the fidelity rule. Excel stored `00124` as 124, the zeros are gone
        // before the file reached us, and no amount of re-padding can recover how many
        // there were — so the only honest answer is to say so.
        let cell = SourceCell::Number {
            text: "124".to_owned(),
            number: 124.0,
        };
        let problem = cell.code().expect_err("a numeric code must be refused");
        assert!(matches!(problem, CellProblem::NumericCode { .. }));
        assert!(
            problem.message().contains("leading zeros"),
            "the message must name the cause: {}",
            problem.message()
        );
        assert!(
            problem.message().contains("template"),
            "and the remedy: {}",
            problem.message()
        );
    }

    #[test]
    fn a_blank_cell_is_absent_rather_than_empty_text() {
        assert_eq!(SourceCell::Blank.code().expect("blank is fine"), None);
        assert!(SourceCell::Blank.is_blank());
    }

    #[test]
    fn an_unreadable_cell_is_not_treated_as_blank() {
        // "The school left this out" and "we could not tell" need different answers.
        let cell = SourceCell::Unsupported("a formula with no stored result".to_owned());
        assert!(!cell.is_blank());
        assert!(cell.code().is_err());
    }

    #[test]
    fn only_one_text_date_format_is_accepted() {
        assert_eq!(
            iso_date("2026-09-01").expect("ISO is accepted"),
            "2026-09-01"
        );
        for ambiguous in ["01/09/2026", "09/01/2026", "1 Sep 2026", "2026-9-1"] {
            assert!(
                iso_date(ambiguous).is_err(),
                "{ambiguous} must not be guessed at"
            );
        }
    }

    #[test]
    fn a_date_that_does_not_exist_is_refused() {
        assert!(iso_date("2026-02-30").is_err());
        assert!(iso_date("2026-13-01").is_err());
        // And a real leap day is not.
        assert_eq!(
            iso_date("2028-02-29").expect("2028 is a leap year"),
            "2028-02-29"
        );
        assert!(iso_date("2027-02-29").is_err());
    }

    #[test]
    fn a_flag_accepts_what_a_school_types_and_refuses_the_rest() {
        for yes in ["Y", "yes", "TRUE", "Active", "1"] {
            assert_eq!(
                SourceCell::Text(yes.to_owned()).flag().expect("readable"),
                Some(true),
                "{yes}"
            );
        }
        for no in ["N", "no", "FALSE", "inactive", "0"] {
            assert_eq!(
                SourceCell::Text(no.to_owned()).flag().expect("readable"),
                Some(false),
                "{no}"
            );
        }
        // A typo must not read as a withdrawal of somebody's employment.
        assert!(SourceCell::Text("Actve".to_owned()).flag().is_err());
    }

    #[test]
    fn a_whole_number_refuses_a_fraction() {
        let two = SourceCell::Number {
            text: "2".to_owned(),
            number: 2.0,
        };
        assert_eq!(two.whole_number().expect("readable"), Some(2));
        let half = SourceCell::Number {
            text: "2.5".to_owned(),
            number: 2.5,
        };
        assert!(half.whole_number().is_err());
    }

    #[test]
    fn a_mapping_is_suggested_from_headings_however_they_are_spelled() {
        let headers = vec![
            "Staff ID".to_owned(),
            "First Name".to_owned(),
            "Surname".to_owned(),
        ];
        let fields: &[(&str, &[&str])] = &[
            ("code", &["Teacher Code", "Staff ID", "staff_id"]),
            ("givenName", &["First Name", "Given Name"]),
            ("familyName", &["Last Name", "Surname", "Family Name"]),
        ];
        let mapping = ColumnMapping::suggest(&headers, fields);
        assert_eq!(mapping.column_of("code"), Some(0));
        assert_eq!(mapping.column_of("givenName"), Some(1));
        assert_eq!(mapping.column_of("familyName"), Some(2));
    }

    #[test]
    fn a_heading_that_matches_nothing_is_left_unmapped() {
        // A fuzzy match would be worse than none: a wrong suggestion nobody notices is how
        // a column of surnames ends up in the staff-code field.
        let headers = vec!["Who".to_owned(), "Thing".to_owned()];
        let fields: &[(&str, &[&str])] = &[("code", &["Teacher Code"])];
        assert_eq!(
            ColumnMapping::suggest(&headers, fields).column_of("code"),
            None
        );
    }

    #[test]
    fn two_columns_with_the_same_heading_claim_one_field_each() {
        let headers = vec!["Code".to_owned(), "Code".to_owned()];
        let fields: &[(&str, &[&str])] = &[("code", &["Code"]), ("other", &["Code"])];
        let mapping = ColumnMapping::suggest(&headers, fields);
        assert_eq!(mapping.column_of("code"), Some(0));
        assert_eq!(
            mapping.column_of("other"),
            Some(1),
            "the second is not left out"
        );
    }

    #[test]
    fn a_column_mapped_twice_is_reported_rather_than_resolved() {
        let mut mapping = ColumnMapping::default();
        mapping.0.insert("givenName".to_owned(), 1);
        mapping.0.insert("code".to_owned(), 1);
        let clashes = mapping.duplicated_columns();
        assert_eq!(clashes.len(), 1);
        assert_eq!(clashes[0].0, 1);
        assert_eq!(clashes[0].1.len(), 2);
    }

    #[test]
    fn unchanged_and_skipped_are_different_answers() {
        // A second run of the same file produces Unchanged; a user's decision produces
        // Skipped. Conflating them would make a no-op import look like a refused one.
        assert!(!RowAction::Unchanged.blocks_apply());
        assert!(!RowAction::Skipped.blocks_apply());
        assert!(!RowAction::Unchanged.writes());
        assert!(!RowAction::Skipped.writes());
        assert!(RowAction::Conflict.blocks_apply());
        assert!(RowAction::Invalid.blocks_apply());
    }

    #[test]
    fn counts_block_only_on_a_decision_a_human_has_not_made() {
        let clean = PlanCounts {
            create: 3,
            unchanged: 1,
            skipped: 2,
            warnings: 4,
            ..PlanCounts::default()
        };
        assert!(!clean.blocked());
        assert_eq!(clean.total(), 6);
        assert!(
            PlanCounts {
                conflict: 1,
                ..clean
            }
            .blocked()
        );
    }

    #[test]
    fn every_kind_has_a_label_and_a_template_name() {
        for kind in ImportKind::ALL {
            assert_ne!(kind.label(), "", "{kind:?} has no label");
            assert!(
                kind.template_stem().starts_with("evara-"),
                "{kind:?} template name must be recognisable in a downloads folder"
            );
        }
        assert_eq!(ImportKind::ALL.len(), 10);
    }
}
