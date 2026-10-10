//! The repository write contract.
//!
//! [D17](../../../docs/OPEN-DECISIONS.md) settled that **repositories** maintain `rev` and
//! `updated_at`, not triggers. The change-log triggers only observe what the repository
//! wrote. That decision is only worth anything if it is hard to get wrong, so the rules
//! are implemented once, here, rather than restated in every repository:
//!
//! - [`Stamp::new`] produces identity and audit values for an insert: `rev = 1`,
//!   `created_at == updated_at`.
//! - [`update`] writes the domain columns **and** appends `updated_at` and `rev = rev + 1`
//!   itself. A repository physically cannot forget them, because it never writes that
//!   part of the statement.
//! - [`delete`] removes one row and reports whether anything went.
//!
//! This is deliberately *not* an ORM. Repositories still write their own SQL for the
//! columns they own; what is centralised is only the part that must never vary.
//!
//! # Transactions
//!
//! Every function here takes a [`Transaction`], so a multi-statement operation is one unit
//! and the change-log rows it produces share its fate. Nothing in this module opens or
//! commits a transaction of its own.

use rusqlite::{Transaction, types::ToSql};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::meta::now_iso8601;

pub(crate) mod row;
pub mod structure;
pub mod time_model;

/// Identity and audit values for a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// Stable identifier.
    pub id: Uuid,
    /// When the row was first written, ISO-8601 UTC.
    pub created_at: String,
    /// When the row last changed, ISO-8601 UTC.
    pub updated_at: String,
    /// Revision, starting at 1 and incremented on every update.
    pub rev: i64,
}

impl Stamp {
    /// Values for a brand-new row.
    ///
    /// `rev` starts at 1 — not 0 — so "never updated" and "updated once" are
    /// distinguishable, and `created_at == updated_at` so the two sort identically
    /// until something changes.
    #[must_use]
    pub fn new() -> Self {
        let now = now_iso8601();
        Self {
            id: Uuid::now_v7(),
            created_at: now.clone(),
            updated_at: now,
            rev: 1,
        }
    }
}

impl Default for Stamp {
    fn default() -> Self {
        Self::new()
    }
}

/// Accepts only a plain lowercase SQL identifier.
///
/// Table and column names are interpolated into statements and cannot be parameter-bound,
/// so everything that reaches [`update`] or [`delete`] is checked first.
pub(crate) fn identifier(value: &str) -> Result<&str> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    if valid {
        Ok(value)
    } else {
        Err(DbError::InvalidIdentifier {
            value: value.to_owned(),
        })
    }
}

/// Updates one row, maintaining `updated_at` and `rev` automatically.
///
/// `assignments` sets only the columns the repository owns, using anonymous `?`
/// placeholders bound from `params` in order — for example `"name = ?, code = ?"`. The
/// `updated_at` and `rev` clauses are appended here and must **not** appear in
/// `assignments`.
///
/// Returns the new [`Stamp`] values for the row.
///
/// # Errors
///
/// - [`DbError::NotFound`] if no row has that identifier, in which case nothing was
///   written and no revision was consumed.
/// - Any constraint failure, which aborts the statement and likewise leaves `rev`
///   untouched.
pub(crate) fn update(
    tx: &Transaction<'_>,
    entity: &'static str,
    table: &str,
    id: Uuid,
    assignments: &str,
    params: &[&dyn ToSql],
) -> Result<(String, i64)> {
    debug_assert!(
        !assignments.contains("rev") && !assignments.contains("updated_at"),
        "repositories must not set rev or updated_at themselves; `update` owns those"
    );

    let table = identifier(table)?;
    let updated_at = now_iso8601();
    let id_text = id.to_string();

    let sql =
        format!("UPDATE {table} SET {assignments}, updated_at = ?, rev = rev + 1 WHERE id = ?");

    let mut bound: Vec<&dyn ToSql> = params.to_vec();
    bound.push(&updated_at);
    bound.push(&id_text);

    let changed = tx
        .execute(&sql, bound.as_slice())
        .map_err(|error| translate(error, entity))?;

    if changed == 0 {
        return Err(DbError::NotFound { entity });
    }

    // Read the revision back rather than assuming: the row is the authority, and this
    // also catches a trigger or future rule that interfered.
    let rev: i64 = tx.query_row(
        &format!("SELECT rev FROM {table} WHERE id = ?"),
        [&id_text],
        |row| row.get(0),
    )?;

    Ok((updated_at, rev))
}

/// Deletes one row.
///
/// The change-log `DELETE` trigger fires inside the caller's transaction, so a removed
/// row stays attributable even though the row itself is gone.
///
/// # Errors
///
/// - [`DbError::NotFound`] if nothing matched.
/// - [`DbError::StillReferenced`] if a foreign key with `ON DELETE RESTRICT` blocks it.
pub(crate) fn delete(
    tx: &Transaction<'_>,
    entity: &'static str,
    table: &str,
    id: Uuid,
) -> Result<()> {
    let table = identifier(table)?;
    let changed = tx
        .execute(
            &format!("DELETE FROM {table} WHERE id = ?"),
            [&id.to_string()],
        )
        .map_err(|error| translate(error, entity))?;

    if changed == 0 {
        return Err(DbError::NotFound { entity });
    }
    Ok(())
}

/// Turns a SQLite constraint failure into something the interface can act on.
///
/// Without this every rejected delete reads as "the project database reported an error",
/// which tells a user nothing about what is actually in the way.
pub(crate) fn translate(error: rusqlite::Error, entity: &'static str) -> DbError {
    use rusqlite::ErrorCode;

    let rusqlite::Error::SqliteFailure(failure, ref detail) = error else {
        return DbError::from(error);
    };
    if failure.code != ErrorCode::ConstraintViolation {
        return DbError::from(error);
    }

    let detail = detail.as_deref().unwrap_or_default().to_lowercase();
    if detail.contains("foreign key") {
        DbError::StillReferenced { entity }
    } else if detail.contains("unique") {
        // SQLite names the offending column in the message. Guessing "name" for everything
        // produced "another cycle day already uses that name" for a duplicate ordinal,
        // which sends the user looking at the wrong field.
        DbError::DuplicateValue {
            entity,
            field: if detail.contains(".code") {
                "code"
            } else if detail.contains(".ordinal") {
                "position"
            } else if detail.contains(".label") {
                "label"
            } else if detail.contains(".date") {
                "date"
            } else {
                "name"
            },
        }
    } else if detail.contains("check") {
        DbError::Invalid {
            message: format!("that {entity} has a value the project does not allow"),
        }
    } else {
        DbError::from(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{Stamp, identifier};

    #[test]
    fn a_new_stamp_starts_at_revision_one() {
        let stamp = Stamp::new();
        assert_eq!(stamp.rev, 1, "rev must start at 1, not 0");
        assert_eq!(
            stamp.created_at, stamp.updated_at,
            "an untouched row was created and updated at the same moment"
        );
        assert_eq!(stamp.id.get_version_num(), 7, "identifiers are UUIDv7");
    }

    #[test]
    fn stamps_are_unique() {
        let a = Stamp::new();
        let b = Stamp::new();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn identifiers_that_could_carry_sql_are_rejected() {
        for bad in [
            "room; DROP TABLE school",
            "Room",
            "1room",
            "",
            "room name",
            "\"room\"",
        ] {
            assert!(identifier(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["room", "room_type", "academic_year"] {
            assert!(identifier(good).is_ok(), "{good:?} should be accepted");
        }
    }
}
