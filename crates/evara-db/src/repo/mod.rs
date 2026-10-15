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
//! # Optimistic concurrency — Stage B
//!
//! [`Expected`] is the other half of [`Stamp`]. A stamp says what a *new* row's revision
//! is; an expectation says what an *existing* row's revision was when the caller last
//! looked at it. Both [`update`] and [`delete`] take one, and both put it in the statement
//! as a predicate:
//!
//! ```sql
//! UPDATE room SET ..., updated_at = ?, rev = rev + 1 WHERE id = ? AND rev = ?
//! ```
//!
//! That ordering is the point. The sequence *read the revision, compare it in Rust, then
//! write* is not a concurrency control at all — it is a race with a longer window — so it
//! is not available here: the comparison is the `WHERE` clause, and a caller cannot reach
//! the write without it. When the predicate matches nothing, one extra read inside the same
//! transaction decides whether the row is absent ([`DbError::NotFound`]) or merely moved on
//! ([`DbError::Conflict`]), because the interface's next step differs completely between
//! the two. Nothing was written either way: no revision was consumed and no trigger fired,
//! so the change feed gains no entry for a conflict.
//!
//! Until Stage B no write took a precondition, and the roadmap called that *"the last thing
//! between this phase and done"* — two windows on one project could overwrite each other,
//! and the second save won silently. See
//! [adr/0027](../../../docs/adr/0027-revisions-are-checked-on-write.md) for the contract
//! each category of mutation carries, including the ones for which a revision is
//! deliberately the wrong question.
//!
//! # Transactions
//!
//! Every function here takes a [`Transaction`], so a multi-statement operation is one unit
//! and the change-log rows it produces share its fate. Nothing in this module opens or
//! commits a transaction of its own.

use rusqlite::{Connection, OptionalExtension, Transaction, types::ToSql};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::meta::now_iso8601;

pub mod availability;
pub mod curriculum;
pub mod lessons;
pub mod membership;
pub mod people;
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

/// The revision a caller believes a row is at.
///
/// The counterpart of [`Stamp`]: a stamp describes a row being created, an expectation
/// describes a row already there. Passed to [`update`] and [`delete`], which put it in the
/// statement's `WHERE` clause rather than comparing it beforehand — see this module's
/// header for why that distinction is the whole mechanism.
///
/// # Why an enum rather than `Option<i64>`
///
/// Because "I have no revision in hand" is a claim worth making out loud. `None` reads as
/// an absent argument that somebody may simply have forgotten to thread through;
/// [`Expected::Any`] is a sentence, it is greppable, and every use of it in this crate is
/// one of three documented kinds:
///
/// 1. A **cascade** inside another operation's transaction — clearing a classification
///    reference, mirroring a value onto a compatibility column. The user never saw that
///    row's revision, so demanding it would mean inventing one.
/// 2. An operation whose concurrency contract is **not** a revision, recorded in ADR 0027
///    §4: a relationship that either exists or does not, an availability cell, a
///    materialisation guarded by a plan fingerprint instead.
/// 3. A **test** that is about something else entirely.
///
/// No command handler constructs one. Every mutation the interface can reach takes a
/// revision from the record it is looking at, which is what makes the coverage claim in
/// ADR 0027 §3 checkable rather than hopeful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    /// The caller read the row at this revision and will not overwrite a newer one.
    Rev(i64),
    /// The caller has no revision in hand. One of the three kinds above, and said so.
    Any,
}

impl Expected {
    /// The revision to match, or `None` for [`Expected::Any`].
    const fn rev(self) -> Option<i64> {
        match self {
            Self::Rev(rev) => Some(rev),
            Self::Any => None,
        }
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
/// `expected` becomes part of the statement's `WHERE` clause, so the revision check and
/// the write are one atomic operation rather than a read followed by a hopeful write. A
/// row that has moved on since the caller read it matches nothing, so nothing is written:
/// no column changes, no revision is consumed and no change-log trigger fires.
///
/// Returns the new `updated_at` and `rev` for the row.
///
/// # Errors
///
/// - [`DbError::Conflict`] if the row is there at a different revision. Somebody else
///   saved first, and the caller's values would have overwritten theirs.
/// - [`DbError::NotFound`] if no row has that identifier, in which case nothing was
///   written and no revision was consumed.
/// - Any constraint failure, which aborts the statement and likewise leaves `rev`
///   untouched.
pub(crate) fn update(
    tx: &Transaction<'_>,
    entity: &'static str,
    table: &str,
    id: Uuid,
    expected: Expected,
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

    let predicate = match expected {
        Expected::Rev(_) => " AND rev = ?",
        Expected::Any => "",
    };
    let sql = format!(
        "UPDATE {table} SET {assignments}, updated_at = ?, rev = rev + 1 \
         WHERE id = ?{predicate}"
    );

    let mut bound: Vec<&dyn ToSql> = params.to_vec();
    bound.push(&updated_at);
    bound.push(&id_text);
    let rev_arg = expected.rev();
    if let Some(ref value) = rev_arg {
        bound.push(value);
    }

    let changed = tx
        .execute(&sql, bound.as_slice())
        .map_err(|error| translate(error, entity))?;

    if changed == 0 {
        return Err(why_nothing_matched(tx, entity, table, id, expected)?);
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

/// Explains a write that matched no row.
///
/// Called only after an `UPDATE` or `DELETE` carrying the revision predicate changed
/// nothing, and inside that same transaction, so the answer it reads is the state the
/// predicate was evaluated against. The write has already failed atomically; this decides
/// only what to *call* the failure — and the two names need completely different responses
/// from the interface. "That record no longer exists" sends somebody looking for something
/// that is in fact there, and "somebody else changed it" sends them looking for an edit
/// that never happened.
fn why_nothing_matched(
    tx: &Transaction<'_>,
    entity: &'static str,
    table: &str,
    id: Uuid,
    expected: Expected,
) -> Result<DbError> {
    let Some(wanted) = expected.rev() else {
        // No predicate was in play, so the row is simply not there.
        return Ok(DbError::NotFound { entity });
    };

    let actual: Option<i64> = tx
        .query_row(
            &format!("SELECT rev FROM {table} WHERE id = ?"),
            [&id.to_string()],
            |row| row.get(0),
        )
        .optional()?;

    Ok(match actual {
        None => DbError::NotFound { entity },
        Some(actual) => DbError::Conflict {
            entity,
            expected: wanted,
            actual,
        },
    })
}

/// Refuses a write whose target row belongs to a different school.
///
/// Called at the top of every `update_*` and every `delete_*` for a school-owned entity,
/// inside the same transaction as the write itself. The identifier alone stopped being
/// sufficient authority at `V12`: a project now holds several schools, and a screen scoped
/// to one of them must not be able to touch another's rows just because it was handed an
/// identifier from it.
///
/// For an update, `school_id` comes from the input the caller already supplies, so no
/// signature changed to get the guard. A delete has no input, so it takes the expected
/// school as a parameter — which is the whole of the follow-up to Stage 0A.
///
/// This form reads the column directly, for the tables that name their school. A child
/// whose ownership is only reachable through a parent uses [`require_school_owner`].
///
/// Takes a [`Connection`] rather than a [`Transaction`], which costs a write call site
/// nothing — `&Transaction` coerces — and lets a *read* refuse a wrong-school identifier
/// without opening a write transaction to ask a question. `Curriculum::subject_teachers`
/// is why: another school may not read this school's qualifications, and making that true
/// should not mean taking a write lock.
///
/// # Errors
///
/// - [`DbError::NotFound`] if no row has that identifier.
/// - [`DbError::WrongSchool`] if one does and another school owns it.
pub(crate) fn require_same_school(
    conn: &Connection,
    entity: &'static str,
    table: &str,
    id: Uuid,
    school_id: Uuid,
) -> Result<()> {
    let table = identifier(table)?;
    require_school_owner(
        conn,
        entity,
        &format!("SELECT school_id FROM {table} WHERE id = ?"),
        id,
        school_id,
    )
}

/// Refuses a write whose target row is owned, through whatever chain, by another school.
///
/// `owner_query` takes the row's identifier as its single parameter and returns the
/// identifier of the school that owns it, or no row at all when the target does not exist.
/// Each repository writes its own, next to the entity it is about, because the chain *is*
/// the domain statement: a period belongs to a school only by way of its bell schedule,
/// and a term only by way of its academic year.
///
/// Deliberately **not** solved by adding `school_id` columns to the children. A second
/// place to record the same ownership is a second place for it to be wrong, and the chain
/// is already enforced by foreign keys.
///
/// # Errors
///
/// - [`DbError::NotFound`] if the query returns no row — the target is not there, or its
///   parent chain is broken, and neither is a licence to delete anything.
/// - [`DbError::WrongSchool`] if it returns a different school.
pub(crate) fn require_school_owner(
    conn: &Connection,
    entity: &'static str,
    owner_query: &str,
    id: Uuid,
    school_id: Uuid,
) -> Result<()> {
    let owner: Option<String> = conn
        .query_row(owner_query, [&id.to_string()], |row| row.get(0))
        .optional()?;

    match owner {
        None => Err(DbError::NotFound { entity }),
        Some(raw) if raw == school_id.to_string() => Ok(()),
        Some(_) => Err(DbError::WrongSchool { entity }),
    }
}

/// Deletes one row, at the revision the caller expects it to be at.
///
/// The change-log `DELETE` trigger fires inside the caller's transaction, so a removed
/// row stays attributable even though the row itself is gone.
///
/// `expected` is in the `WHERE` clause for the same reason as in [`update`]: a row
/// somebody else has edited since the caller read it must not be deleted on the strength
/// of what it used to say. A delete is the one mutation where losing the other edit is
/// irreversible, so it is checked exactly as carefully as an update.
///
/// # Errors
///
/// - [`DbError::Conflict`] if the row is there at a different revision.
/// - [`DbError::NotFound`] if nothing matched.
/// - [`DbError::StillReferenced`] if a foreign key with `ON DELETE RESTRICT` blocks it.
pub(crate) fn delete(
    tx: &Transaction<'_>,
    entity: &'static str,
    table: &str,
    id: Uuid,
    expected: Expected,
) -> Result<()> {
    let table = identifier(table)?;
    let id_text = id.to_string();

    let predicate = match expected {
        Expected::Rev(_) => " AND rev = ?",
        Expected::Any => "",
    };
    let sql = format!("DELETE FROM {table} WHERE id = ?{predicate}");

    let mut bound: Vec<&dyn ToSql> = vec![&id_text];
    let rev_arg = expected.rev();
    if let Some(ref value) = rev_arg {
        bound.push(value);
    }

    let changed = tx
        .execute(&sql, bound.as_slice())
        .map_err(|error| translate(error, entity))?;

    if changed == 0 {
        return Err(why_nothing_matched(tx, entity, table, id, expected)?);
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
