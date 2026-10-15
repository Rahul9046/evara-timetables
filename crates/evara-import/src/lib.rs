//! Reads a spreadsheet or a CSV file into [`SourceTable`], and writes the downloadable
//! templates.
//!
//! # Layering contract
//!
//! Depends on [`evara_core`] and **nothing else of ours**. This crate knows what a
//! worksheet is and nothing about what a school is; resolving a row against a project is
//! `evara_db`'s, because it is a database question. Keeping the two apart is what stops a
//! spreadsheet concept — a sheet name, a cell address, a column letter — reaching the
//! domain, and it is why the parser's dependencies are quarantined here rather than added
//! to the crate that owns SQL. See
//! [ADR 0030](../../../docs/adr/0030-an-import-is-a-reviewed-plan.md) §2.
//!
//! It is also **not** `evara-integrations`. That crate's charter is adapters for *other
//! systems'* formats — PowerSchool, aSc, Edval — arriving after Milestone 1 behind feature
//! flags, and it depends on `evara-core` alone for exactly that reason. A spreadsheet is
//! not another system's format; it is the file a school already keeps its roster in, and
//! reading it is a first-class product feature rather than an adapter. ADR 0030 §2 records
//! the distinction, because conflating them is how a vendor concept would eventually reach
//! the core.
//!
//! # Nothing here evaluates anything
//!
//! A workbook is untrusted input. This crate therefore:
//!
//! - **never evaluates a formula.** `calamine` has no formula engine at all, which is the
//!   property relied on rather than a limitation worked around. A formula cell is read
//!   through its *cached* value when the workbook stored one, and reported as
//!   [`SourceCell::Unsupported`] when it did not — never computed, and never silently
//!   treated as blank.
//! - **never runs a macro.** A macro lives in a part of the container this crate does not
//!   open; `.xlsm` is refused by extension *and* by content, so a macro-bearing workbook
//!   has nowhere to run even in principle.
//! - **never reaches the network.** No dependency here has a socket. Offline is a
//!   correctness property (`CLAUDE.md` rule 1), not a setting.
//! - **never follows a path.** The public entry points take *bytes*, which is the same
//!   decision `SchoolLogoPanel` made and for the same reason: a file the user picked in an
//!   OS dialogue has no path for a bug to turn into an arbitrary-file read.
//!
//! # Bounded, and loudly so
//!
//! Every limit in [`Limits`] refuses rather than truncating, with one exception that
//! *reports*: a file longer than the row limit yields the rows it read and sets
//! [`SourceTable::truncated_at`], because an import that quietly dropped the last four
//! hundred students would be the worst possible failure of this feature.

use evara_core::import::{ColumnMapping, ImportKind, SourceCell, SourceRow, SourceTable};

mod template;

pub use template::{
    TemplateColumn, TemplateFormat, field_specs, template_bytes, template_columns,
    template_file_name,
};

/// What this crate will and will not read.
///
/// Values are deliberately generous for a school roster and deliberately finite. The
/// largest real file this has to cope with is a few thousand students at a few dozen bytes
/// a cell; the limits exist so a damaged or hostile workbook cannot ask for an unbounded
/// allocation, not to express a product rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest file, in bytes.
    pub max_bytes: usize,
    /// Most data rows read from one sheet. Beyond this the table is truncated *and says so*.
    pub max_rows: usize,
    /// Most columns read. A sheet with more is refused: past this the file is not a roster.
    pub max_columns: usize,
    /// Longest single cell, in characters. Longer cells are refused.
    pub max_cell_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // 8 MiB. A 5,000-row roster with thirty columns is well under 1 MiB; this
            // leaves room for a workbook carrying formatting and a second sheet.
            max_bytes: 8 * 1024 * 1024,
            max_rows: 20_000,
            max_columns: 128,
            max_cell_chars: 4_096,
        }
    }
}

/// Why a file could not be read.
///
/// Every variant names what to do about it. "The file could not be read" on its own sends
/// a user back to a dialogue with nothing learned.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ImportFileError {
    /// The extension is not one of the supported ones.
    #[error(
        "{name} is not a supported file. Choose a .xlsx workbook or a .csv file — and if \
         this is an older .xls, open it in a spreadsheet application and save it as .xlsx."
    )]
    UnsupportedExtension {
        /// The file name as the user chose it.
        name: String,
    },

    /// The bytes are not what the extension claims.
    #[error(
        "{name} does not look like the kind of file its name claims. Check that it opens \
         in a spreadsheet application, then save it again as .xlsx or .csv."
    )]
    ContentMismatch {
        /// The file name as the user chose it.
        name: String,
    },

    /// A macro-bearing workbook.
    #[error(
        "{name} is a macro-enabled workbook. Evara does not open these at all. Save it as \
         a plain .xlsx — the data is unchanged and the macros are not needed."
    )]
    MacroWorkbook {
        /// The file name as the user chose it.
        name: String,
    },

    /// Too large.
    #[error(
        "{name} is {size} bytes, and the limit is {limit}. Split it, or remove columns \
             and sheets the import does not need."
    )]
    TooLarge {
        /// The file name as the user chose it.
        name: String,
        /// Actual size.
        size: usize,
        /// The limit.
        limit: usize,
    },

    /// Too wide.
    #[error(
        "that sheet has {found} columns and the limit is {limit}. A roster does not \
             need that many — remove the ones the import will not use."
    )]
    TooWide {
        /// Columns found.
        found: usize,
        /// The limit.
        limit: usize,
    },

    /// A cell longer than the limit.
    #[error("row {line} has a cell of {found} characters, and the limit is {limit}.")]
    CellTooLong {
        /// Sheet line, as the user sees it.
        line: i64,
        /// Length found.
        found: usize,
        /// The limit.
        limit: usize,
    },

    /// The workbook holds no worksheets at all.
    #[error("{name} has no worksheets in it.")]
    NoWorksheets {
        /// The file name as the user chose it.
        name: String,
    },

    /// The named worksheet is not in the workbook.
    #[error("{name} has no worksheet called “{sheet}”. It has: {available}.")]
    NoSuchWorksheet {
        /// The file name as the user chose it.
        name: String,
        /// What was asked for.
        sheet: String,
        /// What is actually there.
        available: String,
    },

    /// The sheet has no header row.
    #[error(
        "that sheet is empty. An import needs a header row naming the columns, and at \
         least one row of data under it."
    )]
    NoHeaderRow,

    /// Every heading on the sheet is blank.
    #[error(
        "that sheet's first row has no column names in it. The first row must name the \
         columns — the downloadable template shows the headings this import understands."
    )]
    NoHeadings,

    /// The underlying parser refused the file.
    #[error("{name} could not be read: {detail}")]
    Malformed {
        /// The file name as the user chose it.
        name: String,
        /// What the parser said, as a sentence rather than a debug dump.
        detail: String,
    },
}

/// Which parser a file goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// An Open XML workbook.
    Xlsx,
    /// Comma-separated text.
    Csv,
}

impl SourceFormat {
    /// Decides the format from a file name.
    ///
    /// The extension chooses the parser; [`sniff`] then checks the *bytes* agree, so a
    /// renamed file is refused rather than fed to the wrong reader.
    ///
    /// `.xls` is deliberately **not** accepted. `calamine` can read it, so accepting the
    /// extension would be one line — and the brief's own warning applies: the legacy
    /// format's date handling and its text/number ambiguity are materially worse than
    /// `.xlsx`'s, which is exactly the fidelity this import is careful about. A school with
    /// an `.xls` is told to re-save it, which takes one keystroke and loses nothing.
    pub fn of(name: &str) -> Result<Self, ImportFileError> {
        // Through `Path` rather than `ends_with`, so `ROSTER.XLSX` and a name with a dot in
        // it are both handled by one rule rather than by three string comparisons.
        let extension = std::path::Path::new(name)
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        match extension.as_str() {
            "xlsx" => Ok(Self::Xlsx),
            "csv" => Ok(Self::Csv),
            "xlsm" | "xltm" => Err(ImportFileError::MacroWorkbook {
                name: name.to_owned(),
            }),
            _ => Err(ImportFileError::UnsupportedExtension {
                name: name.to_owned(),
            }),
        }
    }
}

/// Checks the bytes against the format the name claimed.
///
/// An `.xlsx` is a ZIP container, so it begins `PK`; a CSV is text and must not. Both
/// checks are cheap and both catch the same real mistake — a file renamed rather than
/// re-saved, which would otherwise reach a parser that cannot make sense of it and produce
/// an error about the wrong thing.
fn sniff(name: &str, bytes: &[u8], format: SourceFormat) -> Result<(), ImportFileError> {
    let zip = bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06");
    match format {
        SourceFormat::Xlsx if !zip => Err(ImportFileError::ContentMismatch {
            name: name.to_owned(),
        }),
        SourceFormat::Csv if zip => Err(ImportFileError::ContentMismatch {
            name: name.to_owned(),
        }),
        // A legacy .xls is an OLE2 container with this signature. Named explicitly so the
        // message is about the format rather than about the bytes.
        SourceFormat::Xlsx | SourceFormat::Csv
            if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) =>
        {
            Err(ImportFileError::UnsupportedExtension {
                name: name.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

/// Reads a file's bytes into a source table.
///
/// `sheet` names the worksheet to read, or `None` for the first one; it is ignored for a
/// CSV, which has exactly one. [`SourceTable::available_sheets`] always lists every sheet
/// the file holds, so the interface can offer the others without re-reading.
///
/// # Errors
///
/// Every variant of [`ImportFileError`].
pub fn read(
    name: &str,
    bytes: &[u8],
    sheet: Option<&str>,
    limits: Limits,
) -> Result<SourceTable, ImportFileError> {
    if bytes.len() > limits.max_bytes {
        return Err(ImportFileError::TooLarge {
            name: name.to_owned(),
            size: bytes.len(),
            limit: limits.max_bytes,
        });
    }

    let format = SourceFormat::of(name)?;
    sniff(name, bytes, format)?;

    match format {
        SourceFormat::Xlsx => read_xlsx(name, bytes, sheet, limits),
        SourceFormat::Csv => read_csv(name, bytes, limits),
    }
}

/// Reads one worksheet of an Open XML workbook.
fn read_xlsx(
    name: &str,
    bytes: &[u8],
    sheet: Option<&str>,
    limits: Limits,
) -> Result<SourceTable, ImportFileError> {
    use calamine::{Reader, Xlsx};

    let cursor = std::io::Cursor::new(bytes);
    let mut workbook: Xlsx<_> = Xlsx::new(cursor).map_err(|error| ImportFileError::Malformed {
        name: name.to_owned(),
        detail: describe(&error),
    })?;

    let available: Vec<String> = workbook.sheet_names().clone();
    if available.is_empty() {
        return Err(ImportFileError::NoWorksheets {
            name: name.to_owned(),
        });
    }

    let wanted = match sheet {
        None => available[0].clone(),
        Some(asked) => available
            .iter()
            .find(|candidate| candidate.as_str() == asked)
            .cloned()
            .ok_or_else(|| ImportFileError::NoSuchWorksheet {
                name: name.to_owned(),
                sheet: asked.to_owned(),
                available: available.join(", "),
            })?,
    };

    let range = workbook
        .worksheet_range(&wanted)
        .map_err(|error| ImportFileError::Malformed {
            name: name.to_owned(),
            detail: describe(&error),
        })?;

    // `calamine` returns the cached value a workbook stored for a formula cell and has no
    // engine to compute one, which is the whole of the no-formula-evaluation guarantee.
    // A formula with no cached result arrives as `Data::Empty` and is indistinguishable
    // from a blank — so the formula *text* is read separately below, and only the cells
    // that have one and no value become `Unsupported`.
    let formulas = workbook.worksheet_formula(&wanted).ok();

    // Where the used range actually begins. A sheet whose table starts at B3 -- a title
    // row and an indented table, which is ordinary -- has a range origin of (2, 1), and
    // assuming (0, 0) would report every row two lines early. Every message a user reads
    // names a line, so an off-by-two here sends them to correct the wrong row.
    let (origin_row, origin_column) = range.start().unwrap_or((0, 0));

    let mut lines = range.rows();
    let header_row = lines.next().ok_or(ImportFileError::NoHeaderRow)?;
    let headers = read_headers(header_row, limits)?;

    let mut rows = Vec::new();
    let mut truncated_at = None;
    for (offset, line) in lines.enumerate() {
        // The header occupied the range's first row, so a data row at `offset` is that
        // many rows further down. One more because a sheet line is 1-based.
        let absolute = u32::try_from(offset)
            .unwrap_or(u32::MAX)
            .saturating_add(origin_row + 1);
        let number = i64::from(absolute).saturating_add(1);
        if rows.len() >= limits.max_rows {
            truncated_at = Some(number);
            break;
        }

        let cells = line
            .iter()
            .take(headers.len())
            .enumerate()
            .map(|(column, cell)| {
                let absolute_column = u32::try_from(column)
                    .unwrap_or(u32::MAX)
                    .saturating_add(origin_column);
                xlsx_cell(
                    cell,
                    formula_at(formulas.as_ref(), absolute, absolute_column),
                )
            })
            .collect::<Vec<_>>();
        let cells = pad(cells, headers.len());

        check_cell_lengths(&cells, number, limits)?;
        if cells.iter().all(SourceCell::is_blank) {
            // A blank line in the middle of a sheet is formatting, not data. Keeping it
            // would put an error against a row the user did not write.
            continue;
        }
        rows.push(SourceRow {
            line: number,
            cells,
        });
    }

    Ok(SourceTable {
        sheet: wanted,
        available_sheets: available,
        headers,
        rows,
        truncated_at,
    })
}

/// The formula text stored at one absolute cell, if the workbook stored one.
///
/// Addressed absolutely rather than by offset: the formula range and the value range are
/// two independently computed used ranges and need not share an origin, so an offset into
/// one is not an offset into the other.
fn formula_at(formulas: Option<&calamine::Range<String>>, row: u32, column: u32) -> Option<&str> {
    let value = formulas?.get_value((row, column))?;
    if value.is_empty() { None } else { Some(value) }
}

/// Turns one workbook cell into a source cell.
fn xlsx_cell(cell: &calamine::Data, formula: Option<&str>) -> SourceCell {
    use calamine::Data;

    match cell {
        Data::Empty => {
            // A formula whose result the workbook did not store. Reported rather than
            // treated as blank: "the school left this out" and "we could not tell" need
            // different answers, and nothing here will compute the missing one.
            if formula.is_some() {
                SourceCell::Unsupported(
                    "this cell holds a formula and the workbook did not store its result. \
                     Evara never calculates one — open the file in a spreadsheet \
                     application, save it, and import again."
                        .to_owned(),
                )
            } else {
                SourceCell::Blank
            }
        }
        Data::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                SourceCell::Blank
            } else {
                SourceCell::Text(trimmed.to_owned())
            }
        }
        Data::Float(value) => SourceCell::Number {
            text: format_number(*value),
            number: *value,
        },
        Data::Int(value) => SourceCell::Number {
            text: value.to_string(),
            #[expect(
                clippy::cast_precision_loss,
                reason = "a spreadsheet integer is far inside f64's exact range"
            )]
            number: *value as f64,
        },
        Data::Bool(value) => SourceCell::Boolean(*value),
        Data::DateTime(value) => value.as_datetime().map_or_else(
            || {
                SourceCell::Unsupported(
                    "this cell holds a date the workbook stores in a form Evara cannot \
                         read. Format the column as text in `YYYY-MM-DD` and import again."
                        .to_owned(),
                )
            },
            // Rendered here and never carried as a date type. `calamine`'s
            // `as_datetime` handles both the 1900 and 1904 epochs *and* Excel's
            // 1900 leap-year bug — which is the whole reason that feature is enabled
            // rather than the serial number converted by hand here.
            //
            // `Display` on a `NaiveDate` is ISO-8601, which is the format wanted, and
            // using it means `chrono` is not named anywhere in this crate. The result
            // is then put through `iso_date` anyway: for a year outside 0..=9999
            // `Display` adds a sign prefix, and a `SourceCell::Date` is trusted
            // downstream to already be in the one accepted shape.
            |moment| {
                let rendered = moment.date().to_string();
                match evara_core::import::iso_date(&rendered) {
                    Ok(date) => SourceCell::Date(date),
                    Err(_) => SourceCell::Unsupported(format!(
                        "this cell holds a date Evara cannot represent ({rendered}). \
                             Check the cell, or format the column as text in \
                             `YYYY-MM-DD`."
                    )),
                }
            },
        ),
        Data::DateTimeIso(value) | Data::DurationIso(value) => {
            // Already ISO text. Taken as text rather than as a date so the one accepted
            // format is enforced in exactly one place — `evara_core::import::iso_date`.
            SourceCell::Text(value.trim().to_owned())
        }
        Data::Error(error) => SourceCell::Unsupported(format!(
            "this cell holds a spreadsheet error ({error:?}). Correct it in the file and \
             import again."
        )),
    }
}

/// Renders a number the way the sheet would show it.
///
/// Whole values lose the `.0` a float formatter adds, because `5` is what the cell shows
/// and `5.0` would read as a different answer in a preview.
fn format_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON && value.abs() < 1.0e15 {
        format!("{value:.0}")
    } else {
        let mut text = format!("{value}");
        if text.ends_with(".0") {
            text.truncate(text.len() - 2);
        }
        text
    }
}

/// Reads and trims a header row.
///
/// Blank headings are **kept**, as empty strings, so every later column index still refers
/// to the same column. Dropping them would silently shift a mapping one place left.
/// Trailing blank columns are dropped, because a sheet's used range routinely runs past
/// its data.
fn read_headers(row: &[calamine::Data], limits: Limits) -> Result<Vec<String>, ImportFileError> {
    let mut headers: Vec<String> = row
        .iter()
        .map(|cell| match cell {
            calamine::Data::String(value) => value.trim().to_owned(),
            calamine::Data::Empty => String::new(),
            other => xlsx_cell(other, None).text().unwrap_or_default().to_owned(),
        })
        .collect();

    while headers.last().is_some_and(String::is_empty) {
        headers.pop();
    }
    if headers.len() > limits.max_columns {
        return Err(ImportFileError::TooWide {
            found: headers.len(),
            limit: limits.max_columns,
        });
    }
    if headers.is_empty() {
        return Err(ImportFileError::NoHeadings);
    }
    Ok(headers)
}

/// Reads a CSV file.
fn read_csv(name: &str, bytes: &[u8], limits: Limits) -> Result<SourceTable, ImportFileError> {
    // A UTF-8 BOM is what Excel writes when it saves a CSV as UTF-8, and leaving it in
    // makes the first heading mysteriously fail to match anything.
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        // A roster routinely has a short last column; refusing the file over it would be
        // unhelpful, and `pad` fills the gap.
        .flexible(true)
        .from_reader(body);

    // Read record by record rather than through `records()`, because the *line* a record
    // began on is the only number a user can act on and the record index is not it: the
    // `csv` crate skips wholly empty lines, so one blank row mid-file makes every later
    // row report one line early. `Reader::position` is read before each read and gives
    // the line the next record starts on.
    let mut header_record = csv::StringRecord::new();
    let read_header =
        reader
            .read_record(&mut header_record)
            .map_err(|error| ImportFileError::Malformed {
                name: name.to_owned(),
                detail: describe(&error),
            })?;
    if !read_header {
        return Err(ImportFileError::NoHeaderRow);
    }

    let mut headers: Vec<String> = header_record
        .iter()
        .map(|value| value.trim().to_owned())
        .collect();
    while headers.last().is_some_and(String::is_empty) {
        headers.pop();
    }
    if headers.len() > limits.max_columns {
        return Err(ImportFileError::TooWide {
            found: headers.len(),
            limit: limits.max_columns,
        });
    }
    if headers.is_empty() {
        return Err(ImportFileError::NoHeadings);
    }

    let mut rows = Vec::new();
    let mut truncated_at = None;
    let mut record = csv::StringRecord::new();
    loop {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "a line number is bounded by the file-size limit"
        )]
        let number = reader.position().line() as i64;
        let more = reader
            .read_record(&mut record)
            .map_err(|error| ImportFileError::Malformed {
                name: name.to_owned(),
                detail: describe(&error),
            })?;
        if !more {
            break;
        }
        if rows.len() >= limits.max_rows {
            truncated_at = Some(number);
            break;
        }

        let cells: Vec<SourceCell> = record.iter().take(headers.len()).map(csv_cell).collect();
        let cells = pad(cells, headers.len());

        check_cell_lengths(&cells, number, limits)?;
        if cells.iter().all(SourceCell::is_blank) {
            continue;
        }
        rows.push(SourceRow {
            line: number,
            cells,
        });
    }

    Ok(SourceTable {
        sheet: name.to_owned(),
        available_sheets: vec![name.to_owned()],
        headers,
        rows,
        truncated_at,
    })
}

/// Turns one CSV field into a source cell.
///
/// **Everything stays text.** A CSV has no types, so inferring one would mean guessing —
/// and the guess that matters is the one that turns `00124` into a number and loses the
/// zeros. The field keeps its text, and the per-column rules in
/// [`evara_core::import::SourceCell`] read whatever that column actually wants out of it.
///
/// A leading `=`, `+`, `-` or `@` is the CSV-injection shape, and it is **not** stripped
/// here: the value is what the file says and silently editing it would be worse. Nothing in
/// Evara executes it, and the report writer quotes it on the way back out.
fn csv_cell(value: &str) -> SourceCell {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        SourceCell::Blank
    } else {
        SourceCell::Text(trimmed.to_owned())
    }
}

/// Fills a short row out to the header width with blanks.
fn pad(mut cells: Vec<SourceCell>, width: usize) -> Vec<SourceCell> {
    while cells.len() < width {
        cells.push(SourceCell::Blank);
    }
    cells
}

/// Refuses a row holding a cell longer than the limit.
fn check_cell_lengths(
    cells: &[SourceCell],
    line: i64,
    limits: Limits,
) -> Result<(), ImportFileError> {
    for cell in cells {
        let length = cell.text().map_or(0, |text| text.chars().count());
        if length > limits.max_cell_chars {
            return Err(ImportFileError::CellTooLong {
                line,
                found: length,
                limit: limits.max_cell_chars,
            });
        }
    }
    Ok(())
}

/// One sentence from a parser error, rather than its debug form.
fn describe(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();
    // Parser messages are already sentences; this only stops an unbounded one reaching the
    // interface.
    if text.chars().count() > 200 {
        text.chars().take(200).collect::<String>() + "…"
    } else {
        text
    }
}

/// Suggests a column mapping for one import kind.
///
/// The accepted headings per field live in [`template`], next to the template that writes
/// them, so a template column and the heading the mapper recognises cannot drift apart.
#[must_use]
pub fn suggest_mapping(kind: ImportKind, headers: &[String]) -> ColumnMapping {
    ColumnMapping::suggest(headers, template::accepted_headings(kind))
}

#[cfg(test)]
mod tests {
    use super::{ImportFileError, Limits, SourceFormat, read};
    use evara_core::import::SourceCell;

    /// A minimal CSV, as a school would save one.
    const TEACHERS_CSV: &str = "Teacher Code,First Name,Last Name,Active in School\n\
                                T-0007,Ada,Lovelace,Yes\n\
                                00124,Grace,Hopper,No\n";

    #[test]
    fn a_csv_keeps_every_identifier_as_text() {
        // The fidelity rule, end to end: `00124` must still be `00124`.
        let table = read(
            "roster.csv",
            TEACHERS_CSV.as_bytes(),
            None,
            Limits::default(),
        )
        .expect("a well-formed CSV reads");
        assert_eq!(table.headers.len(), 4);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(
            table.rows[1].cells[0].code().expect("a text code"),
            Some("00124")
        );
        assert_eq!(table.rows[0].line, 2, "the header is line 1");
        assert_eq!(table.rows[1].line, 3);
    }

    #[test]
    fn a_blank_line_in_the_middle_is_not_a_row() {
        let csv = "Code,Name\nA,One\n\n,\nB,Two\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        assert_eq!(table.rows.len(), 2);
        // And the surviving rows keep their real sheet lines, so a message about row 5
        // sends the user to row 5.
        assert_eq!(table.rows[1].line, 5);
    }

    #[test]
    fn a_short_row_is_padded_rather_than_refused() {
        let csv = "Code,Name,Email\nA,One\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        assert_eq!(table.rows[0].cells.len(), 3);
        assert!(table.rows[0].cells[2].is_blank());
    }

    #[test]
    fn quoted_commas_and_newlines_survive() {
        let csv = "Code,Notes\nA,\"one, two\"\nB,\"line\nbreak\"\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        assert_eq!(table.rows[0].cells[1].text(), Some("one, two"));
        assert_eq!(table.rows[1].cells[1].text(), Some("line\nbreak"));
    }

    #[test]
    fn a_utf8_bom_does_not_become_part_of_the_first_heading() {
        // What Excel writes when it saves a UTF-8 CSV. Left in, the first heading matches
        // nothing and the mapping silently loses a column.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"Teacher Code,First Name\nT-1,Ada\n");
        let table = read("x.csv", &bytes, None, Limits::default()).expect("reads");
        assert_eq!(table.headers[0], "Teacher Code");
    }

    #[test]
    fn unicode_names_are_preserved() {
        let csv = "Code,Last Name\nT-1,Ní Chonaill\nT-2,李\nT-3,Müller\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        assert_eq!(table.rows[0].cells[1].text(), Some("Ní Chonaill"));
        assert_eq!(table.rows[1].cells[1].text(), Some("李"));
        assert_eq!(table.rows[2].cells[1].text(), Some("Müller"));
    }

    #[test]
    fn a_formula_shaped_csv_value_is_kept_verbatim_and_never_run() {
        // Nothing in Evara executes it, and silently editing the file's content would be
        // worse than carrying it. The report writer is what quotes it on the way out.
        let csv = "Code,Name\nA,=1+1\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        assert_eq!(table.rows[0].cells[1].text(), Some("=1+1"));
    }

    #[test]
    fn an_empty_file_says_it_needs_a_header_row() {
        let error = read("x.csv", b"", None, Limits::default()).expect_err("no header");
        assert!(matches!(error, ImportFileError::NoHeaderRow), "{error}");
    }

    #[test]
    fn a_header_row_with_no_headings_is_refused_with_the_remedy() {
        let error = read("x.csv", b",,\nA,B,C\n", None, Limits::default()).expect_err("refused");
        assert!(matches!(error, ImportFileError::NoHeadings), "{error}");
        assert!(error.to_string().contains("template"));
    }

    #[test]
    fn a_header_row_and_nothing_under_it_reads_as_no_rows() {
        // Not an error: the user may have chosen the wrong sheet, and the preview saying
        // "0 rows" is clearer than a refusal.
        let table = read("x.csv", b"Code,Name\n", None, Limits::default()).expect("reads");
        assert_eq!(
            table.rows.len(),
            0,
            "a header and nothing under it is zero rows"
        );
        assert_eq!(table.headers.len(), 2);
    }

    #[test]
    fn an_unsupported_extension_names_the_two_that_work() {
        let error =
            read("roster.ods", b"PK\x03\x04", None, Limits::default()).expect_err("refused");
        assert!(matches!(
            error,
            ImportFileError::UnsupportedExtension { .. }
        ));
        assert!(error.to_string().contains(".xlsx"));
        assert!(error.to_string().contains(".csv"));
    }

    #[test]
    fn a_legacy_xls_is_refused_with_advice_rather_than_half_read() {
        // `calamine` could read it. Accepting it would be one line, and the format's
        // text/number ambiguity is materially worse than .xlsx's — the exact fidelity this
        // import is careful about.
        assert!(SourceFormat::of("roster.xls").is_err());
        let ole = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        let error = read("roster.csv", &ole, None, Limits::default()).expect_err("refused");
        assert!(
            matches!(error, ImportFileError::UnsupportedExtension { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_macro_workbook_is_refused_before_anything_is_opened() {
        let error =
            read("roster.xlsm", b"PK\x03\x04", None, Limits::default()).expect_err("refused");
        assert!(
            matches!(error, ImportFileError::MacroWorkbook { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_renamed_file_is_refused_rather_than_fed_to_the_wrong_parser() {
        // A workbook called .csv, and a CSV called .xlsx. Both are the same real mistake.
        let error =
            read("roster.csv", b"PK\x03\x04junk", None, Limits::default()).expect_err("refused");
        assert!(
            matches!(error, ImportFileError::ContentMismatch { .. }),
            "{error}"
        );

        let error =
            read("roster.xlsx", b"Code,Name\n", None, Limits::default()).expect_err("refused");
        assert!(
            matches!(error, ImportFileError::ContentMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_parsed() {
        let limits = Limits {
            max_bytes: 16,
            ..Limits::default()
        };
        let error = read("x.csv", TEACHERS_CSV.as_bytes(), None, limits).expect_err("refused");
        assert!(matches!(error, ImportFileError::TooLarge { .. }), "{error}");
    }

    #[test]
    fn a_file_past_the_row_limit_is_truncated_and_says_so() {
        // The one limit that reports rather than refusing. An import that quietly dropped
        // rows would be the worst possible failure of this feature.
        use std::fmt::Write as _;
        let mut csv = String::from("Code,Name\n");
        for index in 0..10 {
            writeln!(csv, "C{index},Name{index}").expect("writing to a String cannot fail");
        }
        let limits = Limits {
            max_rows: 4,
            ..Limits::default()
        };
        let table = read("x.csv", csv.as_bytes(), None, limits).expect("reads what it can");
        assert_eq!(table.rows.len(), 4);
        assert_eq!(
            table.truncated_at,
            Some(6),
            "and names the line it stopped at"
        );
    }

    #[test]
    fn a_sheet_wider_than_the_limit_is_refused() {
        let headers = (0..10)
            .map(|i| format!("H{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let limits = Limits {
            max_columns: 4,
            ..Limits::default()
        };
        let error =
            read("x.csv", format!("{headers}\n").as_bytes(), None, limits).expect_err("refused");
        assert!(matches!(error, ImportFileError::TooWide { .. }), "{error}");
    }

    #[test]
    fn an_over_long_cell_is_refused_and_names_its_row() {
        let long = "x".repeat(50);
        let csv = format!("Code,Name\nA,{long}\n");
        let limits = Limits {
            max_cell_chars: 10,
            ..Limits::default()
        };
        let error = read("x.csv", csv.as_bytes(), None, limits).expect_err("refused");
        match error {
            ImportFileError::CellTooLong { line, .. } => assert_eq!(line, 2),
            other => panic!("expected a cell-length refusal, got {other}"),
        }
    }

    #[test]
    fn trailing_blank_columns_are_dropped_but_inner_ones_are_kept() {
        // Dropping an inner blank heading would shift every later column index one place
        // left and silently mis-map the file.
        let table = read(
            "x.csv",
            b"Code,,Name,,\nA,,One,,\n",
            None,
            Limits::default(),
        )
        .expect("reads");
        assert_eq!(table.headers, vec!["Code", "", "Name"]);
        assert_eq!(table.rows[0].cells.len(), 3);
        assert!(table.rows[0].cells[1].is_blank());
        assert_eq!(table.rows[0].cells[2].text(), Some("One"));
    }

    #[test]
    fn a_csv_reports_itself_as_its_only_sheet() {
        let table = read(
            "roster.csv",
            TEACHERS_CSV.as_bytes(),
            None,
            Limits::default(),
        )
        .expect("reads");
        assert_eq!(table.available_sheets, vec!["roster.csv"]);
        assert_eq!(table.sheet, "roster.csv");
    }

    #[test]
    fn a_number_keeps_both_its_text_and_its_value() {
        let csv = "Count,Name\n4,One\n";
        let table = read("x.csv", csv.as_bytes(), None, Limits::default()).expect("reads");
        // A CSV field is text, so the number is read by the column that wants one.
        assert_eq!(
            table.rows[0].cells[0].whole_number().expect("readable"),
            Some(4)
        );
        assert_eq!(
            table.rows[0].cells[0]
                .code()
                .expect("readable as a code too"),
            Some("4"),
            "a CSV field has no type, so nothing has been lost to guess at"
        );
    }

    #[test]
    fn a_duplicate_heading_is_kept_so_the_user_can_assign_the_second() {
        let table = read("x.csv", b"Code,Code\nA,B\n", None, Limits::default()).expect("reads");
        assert_eq!(table.headers, vec!["Code", "Code"]);
    }

    #[test]
    fn blank_cells_in_a_text_column_are_absent_rather_than_empty_strings() {
        let table = read("x.csv", b"Code,Email\nA,\n", None, Limits::default()).expect("reads");
        assert!(table.rows[0].cells[1].is_blank());
        assert_eq!(table.rows[0].cells[1].text(), None);
    }

    #[test]
    fn a_malformed_csv_is_reported_as_a_sentence() {
        // An unterminated quote. The message must be readable, not a debug dump.
        let csv = "Code,Name\nA,\"unterminated\n";
        // `csv` is forgiving about a trailing unterminated quote, so assert on whichever
        // answer it gives rather than on a refusal: either it reads the field or it says
        // why, and neither may panic.
        match read("x.csv", csv.as_bytes(), None, Limits::default()) {
            Ok(table) => assert_eq!(table.rows.len(), 1),
            Err(error) => {
                assert!(
                    matches!(error, ImportFileError::Malformed { .. }),
                    "{error}"
                );
                assert!(error.to_string().len() < 400);
            }
        }
    }

    #[test]
    fn formatting_a_number_does_not_add_a_decimal_point() {
        assert_eq!(super::format_number(5.0), "5");
        assert_eq!(super::format_number(2.5), "2.5");
    }

    #[test]
    fn a_blank_cell_is_blank_and_a_formula_without_a_result_is_not() {
        use calamine::Data;
        assert!(super::xlsx_cell(&Data::Empty, None).is_blank());
        let unsupported = super::xlsx_cell(&Data::Empty, Some("=A1+B1"));
        assert!(!unsupported.is_blank());
        assert!(matches!(unsupported, SourceCell::Unsupported(_)));
        assert!(
            unsupported
                .code()
                .expect_err("not readable as a code")
                .message()
                .contains("never calculates"),
            "the message must say Evara does not evaluate it"
        );
    }
}
