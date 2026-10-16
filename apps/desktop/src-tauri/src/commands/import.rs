//! Commands for importing a roster from a spreadsheet.
//!
//! Thin, like every other module here: the parser is `evara-import`, the planner and the
//! transaction are `evara-db`, and what happens below is reading bytes into the session,
//! handing them to one of those two, and converting the error. There is no rule here.
//!
//! # The file arrives as bytes, never as a path
//!
//! `import_begin` takes the file's **name and contents**, exactly as
//! [`SchoolLogoPanel`](../../../../src/features/setup/sections/SchoolLogoPanel.tsx) does,
//! and for the same three reasons: the application declares no filesystem capability, so
//! there is no path for a crafted value to traverse; the interface uses the browser's own
//! `<input type="file">`, which is the only file picker that works under a component test;
//! and a name is enough to choose a parser, which is all the name is used for. A path
//! would have made this command the one place in the application that opens an arbitrary
//! file on the user's disk.
//!
//! # One import at a time, and the backend holds the file
//!
//! [`ImportSession`] lives in [`AppState`] and holds the parsed table. The interface sends
//! the bytes once and then previews as often as it likes, which is partly about not
//! shipping eight megabytes of base64 through the IPC boundary on every keystroke of the
//! mapper — but mostly it is a safety property. If the interface re-sent the file with
//! each call, the file applied could differ from the file reviewed, and the fingerprint
//! would not notice: it is computed over the *plan*, and a different file with the same
//! resulting actions has the same fingerprint. Holding the table makes the substitution
//! unexpressible.
//!
//! A second `import_begin` replaces the session. That is deliberate — a user who picks
//! another file has abandoned the first — and it is why there is no session identifier to
//! get wrong.
//!
//! # What is not here
//!
//! No command writes outside the school it is given, no command infers a school from the
//! file, and `import_preview` writes nothing at all. Those are properties of
//! `evara_db::import` rather than of this module, and they are asserted there, over the
//! change feed.

// Tauri's command macro dictates these signatures: `State` is a guard type taken by value
// and every other parameter is deserialised from the IPC payload rather than borrowed.
// See `project.rs` for the same note.
#![allow(clippy::needless_pass_by_value)]

use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use evara_core::import::{
    ColumnMapping, FieldSpec, ImportFingerprint, ImportKind, ImportPlan, ImportSummary,
    RowDecision, SourceTable,
};
use evara_db::report_csv;
use evara_import::{Limits, TemplateFormat};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use super::error::{SetupError, SetupResult};
use super::project::AppState;
use super::workspace;

/// The one import a window has in progress.
///
/// `Mutex<Option<_>>` rather than a map keyed by an identifier: a user has one import
/// dialogue open, and a design that allowed two would have to answer which of them a
/// school switch invalidates.
#[derive(Debug, Default)]
pub struct ImportSessions {
    current: Mutex<Option<ImportSession>>,
    /// The file last prepared for saving, if any.
    ///
    /// Its own lock rather than a field of [`ImportSession`] because a template is offered
    /// *before* a file has been chosen — downloading one is how a school starts — so it
    /// cannot depend on a session existing.
    prepared: Mutex<Option<PreparedFile>>,
}

impl ImportSessions {
    /// Forgets any session in progress.
    ///
    /// Called when the project closes. Takes no `Result`: a poisoned lock here means a
    /// previous command panicked, and the right answer is still "there is no session" --
    /// refusing to close the project because the import lock is poisoned would trap the
    /// user in the state they are trying to leave.
    pub fn clear(&self) {
        if let Ok(mut current) = self.current.lock() {
            *current = None;
        }
        if let Ok(mut prepared) = self.prepared.lock() {
            *prepared = None;
        }
    }
}

/// Bytes the backend produced, waiting for a path to be written to.
///
/// Held here rather than returned to the interface and sent back on the save, which is the
/// point: `import_save` takes a path and **no content**, so there is no way for the
/// interface to make the backend write something the backend did not generate. The only
/// authority the write has is the path, and that came from an OS save dialogue the page
/// cannot script — the same arrangement `project_create` has had since Phase 1B.
#[derive(Debug)]
struct PreparedFile {
    /// What to suggest in the dialogue.
    file_name: String,
    /// What to write.
    bytes: Vec<u8>,
}

/// A file that has been read and not yet applied.
#[derive(Debug)]
struct ImportSession {
    /// What is being imported. Fixed when the file is read, because the mapping and the
    /// required-field check are both about this.
    kind: ImportKind,
    /// The file's name, for messages and for the report's own file name.
    file_name: String,
    /// The bytes, kept so another worksheet of the same workbook can be read without the
    /// interface sending the file again.
    bytes: Vec<u8>,
    /// The sheet currently parsed.
    table: SourceTable,
    /// The last summary, so the report is rendered from what actually happened rather
    /// than from a value the interface handed back.
    last_summary: Option<ImportSummary>,
}

/// What the interface needs to show a worksheet and build a mapping.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportSourceView {
    /// What is being imported.
    pub kind: ImportKind,
    /// The file's name, as the user chose it.
    pub file_name: String,
    /// The worksheet read.
    pub sheet: String,
    /// Every worksheet the file holds, so another can be chosen without re-sending it.
    pub available_sheets: Vec<String>,
    /// The headings, in column order.
    pub headers: Vec<String>,
    /// How many data rows were read.
    pub row_count: i64,
    /// The row the parser stopped at, when the file was longer than the limit.
    pub truncated_at: Option<i64>,
    /// The fields this kind has, with their labels and hints.
    pub fields: Vec<FieldSpec>,
    /// Evara's suggestion, which the user may change entirely.
    pub suggested_mapping: ColumnMapping,
    /// The first few rows, so a mapping can be checked against real values before
    /// previewing two thousand of them.
    pub sample: Vec<SampleRow>,
}

/// One row of the mapper's sample, as text.
///
/// Text rather than [`evara_core::import::SourceCell`] because this is for a user to read
/// and because a sample must not become a second way of interpreting a cell. The plan is
/// the only interpretation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SampleRow {
    /// The row's number as the spreadsheet shows it.
    pub line: i64,
    /// One entry per column, blank where the cell was.
    pub cells: Vec<String>,
}

/// How many rows the mapper is shown.
const SAMPLE_ROWS: usize = 5;

/// Which template format is wanted.
///
/// Its own type rather than `evara_import::TemplateFormat` because that one is not
/// serialisable and should not become so: a parser crate has no business knowing about an
/// IPC boundary.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum TemplateKindFormat {
    /// A workbook, with code columns formatted as text.
    Workbook,
    /// Plain text.
    Csv,
}

impl From<TemplateKindFormat> for TemplateFormat {
    fn from(format: TemplateKindFormat) -> Self {
        match format {
            TemplateKindFormat::Workbook => Self::Xlsx,
            TemplateKindFormat::Csv => Self::Csv,
        }
    }
}

/// One kind of import, as the chooser lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImportKindView {
    /// The stable key.
    pub kind: ImportKind,
    /// What the user reads.
    pub label: &'static str,
    /// What has to exist first, in the user's words, so the chooser can say so rather
    /// than letting a school import enrolments into courses they have not created.
    pub depends_on: Option<&'static str>,
    /// The fields, with their labels and hints.
    pub fields: Vec<FieldSpec>,
}

/// Every kind of import, in the order a school would do them.
///
/// Infallible, and takes no state: the list is a property of the build, not of the open
/// project. The `Result` is for the interface's own uniformity.
#[tauri::command]
#[allow(clippy::unnecessary_wraps)]
pub fn import_kinds() -> SetupResult<Vec<ImportKindView>> {
    Ok(ImportKind::ALL
        .iter()
        .map(|kind| ImportKindView {
            kind: *kind,
            label: kind.label(),
            depends_on: kind.depends_on(),
            fields: evara_import::field_specs(*kind),
        })
        .collect())
}

/// Prepares a blank template, and returns the name to suggest for it.
///
/// Followed by [`import_save`] once the user has chosen where it goes. Two steps because
/// the save dialogue needs a name before it opens, and because the bytes never need to
/// cross the boundary at all.
#[tauri::command]
pub fn import_template(
    state: State<'_, AppState>,
    kind: ImportKind,
    format: TemplateKindFormat,
) -> SetupResult<String> {
    let format = TemplateFormat::from(format);
    let bytes = evara_import::template_bytes(kind, format).map_err(|error| SetupError::Failed {
        // The writer's own words, which name the failure without naming a path: the
        // template is written to memory and has no path to leak.
        message: format!("The template could not be prepared. {error}"),
    })?;
    let file_name = evara_import::template_file_name(kind, format);
    prepare(&state, &file_name, bytes)?;
    Ok(file_name)
}

/// Reads a chosen file and starts a session.
///
/// Replaces any session already in progress, because choosing another file is how a user
/// abandons the first.
#[tauri::command]
pub fn import_begin(
    state: State<'_, AppState>,
    kind: ImportKind,
    file_name: String,
    base64: String,
    sheet: Option<String>,
) -> SetupResult<ImportSourceView> {
    let bytes = decode_base64(&base64)?;
    let table = read_table(&file_name, &bytes, sheet.as_deref())?;
    let view = source_view(kind, &file_name, &table);

    let mut current = sessions(&state)?;
    *current = Some(ImportSession {
        kind,
        file_name,
        bytes,
        table,
        last_summary: None,
    });
    Ok(view)
}

/// Reads another worksheet of the file already chosen.
///
/// Separate from `import_begin` so the interface need not hold the bytes to offer the
/// sheet list it was given.
#[tauri::command]
pub fn import_sheet(state: State<'_, AppState>, sheet: String) -> SetupResult<ImportSourceView> {
    let mut current = sessions(&state)?;
    let session = current.as_mut().ok_or_else(no_session)?;
    let table = read_table(&session.file_name, &session.bytes, Some(&sheet))?;
    let view = source_view(session.kind, &session.file_name, &table);
    session.table = table;
    session.last_summary = None;
    Ok(view)
}

/// The file as it stands, without re-sending it.
///
/// For the case ADR 0023 is about: a school switch remounts the screen, and the interface
/// needs the mapper's state back without asking the user for the file again.
#[tauri::command]
pub fn import_current(state: State<'_, AppState>) -> SetupResult<Option<ImportSourceView>> {
    let current = sessions(&state)?;
    Ok(current
        .as_ref()
        .map(|session| source_view(session.kind, &session.file_name, &session.table)))
}

/// Builds a preview. **Writes nothing.**
///
/// `schoolId` is a parameter rather than session state on purpose: the school is the
/// user's current selection and may change between two previews of the same file, and a
/// session that remembered one would quietly plan against the wrong institution.
#[tauri::command]
pub fn import_preview(
    state: State<'_, AppState>,
    school_id: Uuid,
    mapping: ColumnMapping,
    decisions: Vec<RowDecision>,
) -> SetupResult<ImportPlan> {
    let (kind, table) = {
        let current = sessions(&state)?;
        let session = current.as_ref().ok_or_else(no_session)?;
        (session.kind, clone_table(&session.table))
    };
    let mut ws = workspace(&state)?;
    Ok(ws
        .import()?
        .plan(school_id, kind, &table, &mapping, &decisions)?)
}

/// Applies a previewed plan, in one transaction.
///
/// `fingerprint` is the one the preview returned. A plan that has moved since is refused
/// as [`SetupError::ReviewRequired`] with nothing written.
#[tauri::command]
pub fn import_apply(
    state: State<'_, AppState>,
    school_id: Uuid,
    mapping: ColumnMapping,
    decisions: Vec<RowDecision>,
    fingerprint: ImportFingerprint,
) -> SetupResult<ImportSummary> {
    let (kind, table) = {
        let current = sessions(&state)?;
        let session = current.as_ref().ok_or_else(no_session)?;
        (session.kind, clone_table(&session.table))
    };

    let summary = {
        let mut ws = workspace(&state)?;
        ws.import()?
            .apply(school_id, kind, &table, &mapping, &decisions, &fingerprint)?
    };

    // Kept so the report is rendered from what happened. Recorded only after the write
    // succeeded, so a refused apply leaves the previous answer — or none — in place.
    if let Some(session) = sessions(&state)?.as_mut() {
        session.last_summary = Some(summary.clone());
    }
    Ok(summary)
}

/// The report for the import just applied.
///
/// Rendered from the session's own record rather than from a summary the interface hands
/// back, so the file a school keeps cannot disagree with what Evara did.
#[tauri::command]
pub fn import_report(state: State<'_, AppState>) -> SetupResult<String> {
    let (file_name, bytes) = {
        let current = sessions(&state)?;
        let session = current.as_ref().ok_or_else(no_session)?;
        let summary = session.last_summary.as_ref().ok_or(SetupError::Invalid {
            message: "There is no completed import to report on yet.".to_owned(),
        })?;
        (
            format!("{}-import-report.csv", summary.kind.template_stem()),
            report_csv(summary).into_bytes(),
        )
    };
    prepare(&state, &file_name, bytes)?;
    Ok(file_name)
}

/// Writes the file last prepared, to a path the user chose in a save dialogue.
///
/// Takes no content. See [`PreparedFile`].
///
/// # Errors
///
/// [`SetupError::Invalid`] if nothing is prepared, and [`SetupError::Failed`] if the write
/// fails — with the operating system's reason but **not** the path, which the user already
/// knows and which has no business in a message that may be logged.
#[tauri::command]
pub fn import_save(state: State<'_, AppState>, path: String) -> SetupResult<String> {
    let prepared = state
        .imports
        .prepared
        .lock()
        .map_err(|_| poisoned())?
        .take()
        .ok_or(SetupError::Invalid {
            message: "There is nothing to save. Prepare the file again.".to_owned(),
        })?;

    std::fs::write(&path, &prepared.bytes).map_err(|error| SetupError::Failed {
        message: format!(
            "That file could not be written: {}. Choose another location and try again.",
            error.kind()
        ),
    })?;
    Ok(prepared.file_name)
}

/// Ends the session and forgets the file.
///
/// Called when the dialogue closes, and the reason the bytes do not outlive it.
#[tauri::command]
pub fn import_cancel(state: State<'_, AppState>) -> SetupResult<bool> {
    let mut current = sessions(&state)?;
    Ok(current.take().is_some())
}

// ---------------------------------------------------------------------------- plumbing

/// Locks the session.
fn sessions<'a>(
    state: &'a State<'_, AppState>,
) -> SetupResult<std::sync::MutexGuard<'a, Option<ImportSession>>> {
    state.imports.current.lock().map_err(|_| poisoned())
}

/// Holds bytes for the next [`import_save`].
///
/// Replaces whatever was prepared before: a user who asks for a different template has
/// abandoned the first, and keeping both would mean deciding which one a save means.
fn prepare(state: &State<'_, AppState>, file_name: &str, bytes: Vec<u8>) -> SetupResult<()> {
    let mut prepared = state.imports.prepared.lock().map_err(|_| poisoned())?;
    *prepared = Some(PreparedFile {
        file_name: file_name.to_owned(),
        bytes,
    });
    Ok(())
}

/// What a poisoned lock means, worded once.
fn poisoned() -> SetupError {
    SetupError::Failed {
        message: "Evara is in an inconsistent state. Please restart the application.".to_owned(),
    }
}

/// What to say when a command arrives with no file chosen.
fn no_session() -> SetupError {
    SetupError::Invalid {
        message: "No file is open for import. Choose a file and try again.".to_owned(),
    }
}

/// Reads bytes into a table, converting the parser's refusal into one the interface can
/// show.
///
/// Every variant of `ImportFileError` already names its own remedy, so the message is
/// forwarded rather than replaced — unlike a `DbError::Internal`, it carries no engine
/// detail and no path: the parser is given bytes and a name, and never opens anything.
fn read_table(name: &str, bytes: &[u8], sheet: Option<&str>) -> SetupResult<SourceTable> {
    evara_import::read(name, bytes, sheet, Limits::default()).map_err(|error| SetupError::Invalid {
        message: error.to_string(),
    })
}

/// Assembles what the mapper needs.
fn source_view(kind: ImportKind, file_name: &str, table: &SourceTable) -> ImportSourceView {
    ImportSourceView {
        kind,
        file_name: file_name.to_owned(),
        sheet: table.sheet.clone(),
        available_sheets: table.available_sheets.clone(),
        headers: table.headers.clone(),
        row_count: i64::try_from(table.rows.len()).unwrap_or(i64::MAX),
        truncated_at: table.truncated_at,
        fields: evara_import::field_specs(kind),
        suggested_mapping: evara_import::suggest_mapping(kind, &table.headers),
        sample: table
            .rows
            .iter()
            .take(SAMPLE_ROWS)
            .map(|row| SampleRow {
                line: row.line,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| cell.text().unwrap_or_default().to_owned())
                    .collect(),
            })
            .collect(),
    }
}

/// Copies a table out of the session so the session lock is not held across a database
/// call.
///
/// Two locks taken in one command is how a deadlock is written, and the import engine
/// takes the workspace's. Cheap enough: a table is bounded by [`Limits`], and a preview
/// is a user action rather than a loop.
fn clone_table(table: &SourceTable) -> SourceTable {
    table.clone()
}

/// Decodes the IPC payload's base64.
///
/// The same engine `structure::school_logo` uses, so the two halves of "bytes cross the
/// boundary as text" are decided once. A payload that is not base64 is a rejected file
/// and not a failure to explain: the interface built it, so the user cannot have got it
/// wrong, and the only honest advice is to choose the file again.
fn decode_base64(input: &str) -> SetupResult<Vec<u8>> {
    BASE64.decode(input).map_err(|_| SetupError::Invalid {
        message: "That file could not be read. Please choose it again, or try another file."
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::{BASE64, ImportKind, decode_base64, import_kinds};

    #[test]
    fn base64_decodes_every_byte_a_workbook_can_hold() {
        // A workbook is binary, and every byte value has to survive the upload intact — a
        // lost high bit is a file the school cannot open, reported as a parser failure.
        let all: Vec<u8> = (0..=255).collect();
        for length in [0, 1, 2, 3, 17, 255, 256] {
            let slice = &all[..length.min(all.len())];
            let decoded = decode_base64(&BASE64.encode(slice)).expect("decode");
            assert_eq!(decoded, slice, "length {length}");
        }
    }

    #[test]
    fn a_payload_that_is_not_base64_is_refused_rather_than_truncated() {
        let outcome = decode_base64("not base64 ***");
        assert!(outcome.is_err(), "a stray byte must not be skipped");
    }

    #[test]
    fn every_kind_is_offered_with_its_fields() {
        let kinds = import_kinds().expect("infallible");
        assert_eq!(kinds.len(), ImportKind::ALL.len());
        for view in kinds {
            assert!(!view.fields.is_empty(), "{:?}", view.kind);
            assert!(!view.label.is_empty(), "{:?}", view.kind);
        }
    }

    #[test]
    fn every_template_is_offered_in_both_formats_under_the_right_extension() {
        // `import_template` needs application state, which a unit test has no way to
        // build, so the two halves it composes are tested where they live: the bytes in
        // `evara_import::template`, and the naming here.
        for kind in ImportKind::ALL {
            let workbook =
                evara_import::template_file_name(*kind, evara_import::TemplateFormat::Xlsx);
            let csv = evara_import::template_file_name(*kind, evara_import::TemplateFormat::Csv);
            assert_eq!(extension_of(&workbook), Some("xlsx"), "{workbook}");
            assert_eq!(extension_of(&csv), Some("csv"), "{csv}");
        }
    }

    /// A file name's extension, for an assertion that is about the name and not the case.
    fn extension_of(name: &str) -> Option<&str> {
        std::path::Path::new(name)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
    }
}
