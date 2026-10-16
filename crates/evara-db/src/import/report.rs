//! The downloadable import report.
//!
//! One row per source row, so a school can take the file away, correct it and import again.
//! Written here rather than in the interface because the plan is the backend's answer and a
//! second renderer would be a second answer.
//!
//! # Formula injection, and why quoting is not optional
//!
//! A report echoes values that came out of an untrusted spreadsheet. A cell beginning `=`,
//! `+`, `-` or `@` is a formula to Excel the moment the report is opened, so a file
//! containing `=HYPERLINK("http://…")` in a name column would become a live link in the
//! report Evara generated. The parser deliberately does **not** edit those values on the way
//! in — the file says what it says — so this is where they are made inert: a leading
//! formula character is prefixed with a single quote, which Excel reads as "this is text"
//! and which a plain-text reader sees for what it is.
//!
//! That is the only transformation here. Nothing else about a value is changed, because a
//! report that disagreed with the preview would be worse than no report.

use evara_core::import::{ImportSummary, RowAction, Severity};

/// Renders an import summary as CSV.
///
/// Columns are fixed and in this order: the original row number first, because that is the
/// thing a user acts on.
#[must_use]
pub fn report_csv(summary: &ImportSummary) -> String {
    let mut out = String::from("Row,Import,Source,Action,Target,Severity,Message\n");

    for row in &summary.rows {
        let action = describe(row.action);
        let target = row.target_label.clone().unwrap_or_default();

        if row.findings.is_empty() {
            push_record(
                &mut out,
                &[
                    &row.line.to_string(),
                    summary.kind.label(),
                    &row.source_label,
                    action,
                    &target,
                    "",
                    "",
                ],
            );
            continue;
        }
        // One line per finding, so a row with two problems is two lines a user can work
        // through rather than one cell holding a paragraph.
        for finding in &row.findings {
            push_record(
                &mut out,
                &[
                    &row.line.to_string(),
                    summary.kind.label(),
                    &row.source_label,
                    action,
                    &target,
                    match finding.severity {
                        Severity::Warning => "Warning",
                        Severity::Error => "Error",
                    },
                    &finding.message,
                ],
            );
        }
    }
    out
}

/// What an outcome is called in the report.
const fn describe(action: RowAction) -> &'static str {
    match action {
        RowAction::Create => "Created",
        RowAction::Update => "Updated",
        RowAction::Unchanged => "Unchanged",
        RowAction::Skipped => "Skipped",
        RowAction::Conflict => "Needs a decision",
        RowAction::Invalid => "Not imported",
    }
}

/// Appends one CSV record, quoting every field.
///
/// Written out rather than taken from the `csv` crate because `evara-db` has no business
/// depending on a CSV writer: the rule is four lines, and adding the dependency here would
/// put a file-format library in the crate whose charter is SQL.
fn push_record(out: &mut String, fields: &[&str]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        for character in neutralise(field).chars() {
            if character == '"' {
                out.push('"');
            }
            out.push(character);
        }
        out.push('"');
    }
    out.push('\n');
}

/// Makes a value that a spreadsheet would read as a formula inert.
///
/// A leading apostrophe is the long-standing convention: Excel and LibreOffice both read it
/// as "the rest of this cell is text" and do not display it. Applied only to a leading
/// formula character, so an ordinary value is untouched.
fn neutralise(field: &str) -> String {
    match field.chars().next() {
        Some('=' | '+' | '-' | '@') => format!("'{field}"),
        _ => field.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::report_csv;
    use evara_core::import::{ImportKind, ImportSummary, PlannedRow, RowAction, RowFinding};

    fn summary(rows: Vec<PlannedRow>) -> ImportSummary {
        ImportSummary {
            kind: ImportKind::Teachers,
            created: 0,
            updated: 0,
            unchanged: 0,
            skipped: 0,
            warnings: 0,
            rows_processed: i64::try_from(rows.len()).expect("a test fixture is small"),
            rows,
        }
    }

    #[test]
    fn a_clean_row_is_one_line_with_no_message() {
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 4,
            action: RowAction::Create,
            source_label: "T-0007 — Ada Lovelace".to_owned(),
            target_label: None,
            findings: Vec::new(),
        }]));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "a header and one row");
        assert!(lines[1].starts_with("\"4\",\"Teachers\""));
        assert!(lines[1].contains("\"Created\""));
    }

    #[test]
    fn a_row_with_two_findings_is_two_lines() {
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 9,
            action: RowAction::Invalid,
            source_label: "row nine".to_owned(),
            target_label: None,
            findings: vec![
                RowFinding::row_error("first problem"),
                RowFinding::warning("second thing".to_owned()),
            ],
        }]));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("Error"));
        assert!(lines[1].contains("first problem"));
        assert!(lines[2].contains("Warning"));
    }

    #[test]
    fn a_value_a_spreadsheet_would_run_is_made_inert() {
        // The whole point of `neutralise`. A name column holding `=HYPERLINK(...)` must not
        // become a live formula in the report Evara generated.
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 2,
            action: RowAction::Invalid,
            source_label: "=HYPERLINK(\"http://example.invalid\")".to_owned(),
            target_label: None,
            findings: Vec::new(),
        }]));
        assert!(
            csv.contains("\"'=HYPERLINK"),
            "a leading = must be prefixed: {csv}"
        );
    }

    #[test]
    fn every_formula_lead_character_is_covered() {
        for lead in ['=', '+', '-', '@'] {
            let csv = report_csv(&summary(vec![PlannedRow {
                line: 2,
                action: RowAction::Create,
                source_label: format!("{lead}danger"),
                target_label: None,
                findings: Vec::new(),
            }]));
            assert!(csv.contains(&format!("\"'{lead}danger\"")), "{lead}");
        }
    }

    #[test]
    fn an_ordinary_value_is_not_edited() {
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 2,
            action: RowAction::Create,
            source_label: "Ada Lovelace".to_owned(),
            target_label: None,
            findings: Vec::new(),
        }]));
        assert!(csv.contains("\"Ada Lovelace\""));
        assert!(!csv.contains("'Ada"));
    }

    #[test]
    fn a_quote_in_a_value_is_doubled_rather_than_breaking_the_file() {
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 2,
            action: RowAction::Create,
            source_label: "O\"Brien".to_owned(),
            target_label: None,
            findings: Vec::new(),
        }]));
        assert!(csv.contains("\"O\"\"Brien\""), "{csv}");
    }

    #[test]
    fn a_comma_and_a_newline_in_a_message_stay_inside_one_field() {
        let csv = report_csv(&summary(vec![PlannedRow {
            line: 2,
            action: RowAction::Invalid,
            source_label: "x".to_owned(),
            target_label: None,
            findings: vec![RowFinding::row_error("one, two\nthree")],
        }]));
        // Quoted, so the embedded newline does not end the record.
        assert!(csv.contains("\"one, two\nthree\""), "{csv}");
    }

    #[test]
    fn the_row_number_comes_first_because_it_is_what_a_user_acts_on() {
        let csv = report_csv(&summary(Vec::new()));
        assert!(csv.starts_with("Row,Import,Source,Action,"));
    }
}
