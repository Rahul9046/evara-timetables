//! Row decoding, shared by every repository.
//!
//! Written for the static Structure entities in Phase 1C and lifted here in Phase 1D, when
//! the time model needed the same pieces. A second copy of the identifier-decoding rule is
//! exactly the kind of duplication that lets one of the copies quietly go wrong.
//!
//! This is **not** an ORM. The [`entity!`] macro generates the plain data structs and the
//! `Row` mapping — pure boilerplate with no decisions in it. Every query, every constraint
//! and every deletion rule is still written out by hand in the repository.

use rusqlite::Row;
use uuid::Uuid;

use crate::error::{CorruptId, DbError, Result};

/// Reads one column into a domain type.
///
/// Exists because identifiers are stored as TEXT, while rusqlite's optional `uuid` feature
/// maps `Uuid` to a 16-byte BLOB. Keeping the conversion here means the storage choice
/// (readable in any SQLite browser — ADR 0003) is not dictated by the driver.
pub(crate) trait FromColumn: Sized {
    fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self>;
}

macro_rules! from_column_via_driver {
    ($($ty:ty),* $(,)?) => {
        $(impl FromColumn for $ty {
            fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self> { row.get(index) }
        })*
    };
}

from_column_via_driver!(String, Option<String>, i64, Option<i64>, bool);

impl FromColumn for Uuid {
    fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self> {
        parse_id(row, index, &row.get::<_, String>(index)?)
    }
}

impl FromColumn for Option<Uuid> {
    fn read(row: &Row<'_>, index: usize) -> rusqlite::Result<Self> {
        match row.get::<_, Option<String>>(index)? {
            Some(raw) => parse_id(row, index, &raw).map(Some),
            None => Ok(None),
        }
    }
}

/// Declares the record and input types for one entity, plus the row reader.
///
/// The generated `from_row` reads columns positionally in declaration order, so the
/// `SELECT` list in the repository must name them in the same order: `id`, then the
/// declared fields, then `created_at`, `updated_at`, `rev`.
macro_rules! entity {
    (
        $(#[$record_doc:meta])* $record:ident,
        $(#[$input_doc:meta])* $input:ident,
        { $( $(#[$field_doc:meta])* $field:ident : $ty:ty ),* $(,)? }
    ) => {
        $(#[$record_doc])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $record {
            /// Stable identifier.
            pub id: uuid::Uuid,
            $( $(#[$field_doc])* pub $field: $ty, )*
            /// When the row was first written, ISO-8601 UTC.
            pub created_at: String,
            /// When the row last changed, ISO-8601 UTC.
            pub updated_at: String,
            /// Revision, starting at 1.
            pub rev: i64,
        }

        $(#[$input_doc])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $input {
            $( $(#[$field_doc])* pub $field: $ty, )*
        }

        impl $record {
            fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
                let mut index = 0usize;
                let mut next = || { let i = index; index += 1; i };
                Ok(Self {
                    id: $crate::repo::row::FromColumn::read(row, next())?,
                    $( $field: $crate::repo::row::FromColumn::read(row, next())?, )*
                    created_at: $crate::repo::row::FromColumn::read(row, next())?,
                    updated_at: $crate::repo::row::FromColumn::read(row, next())?,
                    rev: $crate::repo::row::FromColumn::read(row, next())?,
                })
            }
        }
    };
}

pub(crate) use entity;

/// Reads a stored identifier, failing rather than inventing one.
///
/// Identifiers are written by this crate, so a malformed one is corruption, not input.
/// Substituting [`Uuid::nil`] — which this did until Phase 1D — hid that: every damaged row
/// decoded to the *same* identifier, so unrelated rows compared equal, `find_by_id` looked
/// up a row that does not exist, and the next update would have written the nil value back
/// as though it were real. A corrupt file must be visibly corrupt.
///
/// The failure travels as [`rusqlite::Error::FromSqlConversionFailure`] carrying a
/// [`CorruptId`], because row decoding is handed to the driver and cannot return a
/// [`DbError`] directly. `From<rusqlite::Error>` unwraps it back into
/// [`DbError::CorruptIdentifier`], so every caller sees the typed error rather than a
/// generic internal one.
fn parse_id(row: &Row<'_>, index: usize, raw: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(raw).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(CorruptId {
                column: column_name(row, index),
                value: raw.to_owned(),
            }),
        )
    })
}

/// Parses a stored identifier read as text outside row decoding.
///
/// Same rule as [`parse_id`], for the places that select an identifier column on its own
/// rather than through a record's row reader. Reporting [`DbError::NotFound`] here — which
/// `detach_rooms` did until Phase 1D — claimed the row was absent when it is in fact
/// present and damaged, and would have left the caller hunting for a deletion that never
/// happened.
pub(crate) fn parse_id_text(column: &'static str, raw: &str) -> Result<Uuid> {
    Uuid::parse_str(raw).map_err(|_| DbError::CorruptIdentifier {
        column: column.to_owned(),
        value: raw.to_owned(),
    })
}

/// Names the column a value came from, for the error message.
///
/// Best effort: the index is a usable fallback when the driver cannot name it, and this
/// runs only on the corruption path where failing to produce a message would be absurd.
fn column_name(row: &Row<'_>, index: usize) -> String {
    let statement: &rusqlite::Statement<'_> = row.as_ref();
    statement
        .column_name(index)
        .map_or_else(|_| format!("column {index}"), ToOwned::to_owned)
}

/// Binds an identifier as the TEXT the schema stores.
pub(crate) fn text(id: Uuid) -> String {
    id.to_string()
}

/// Binds an optional identifier, keeping `NULL` as `NULL`.
pub(crate) fn opt_text(id: Option<Uuid>) -> Option<String> {
    id.map(|value| value.to_string())
}
