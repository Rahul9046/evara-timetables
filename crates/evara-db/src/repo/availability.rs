//! When an entity cannot, or would rather not, be timetabled.
//!
//! One repository for all six entity kinds, matching the one table. A teacher, a room and a
//! cohort answer "when are you free?" the same way, and the solver will ask them all the
//! same question.
//!
//! # Available is the default
//!
//! Only exceptions are stored. An entity with no rows here is free in every slot, which is
//! what keeps "not configured yet" and "never free" from collapsing into the same empty
//! set. [`Availability::set`] therefore has a deliberate asymmetry with the rest of this
//! crate: writing a rule is an upsert, and *clearing* a cell is [`Availability::clear`],
//! which removes the row rather than storing an `AVAILABLE` status that does not exist.
//!
//! # The upsert preserves the row
//!
//! [`Availability::set`] updates the existing rule in place when one already covers that
//! scope, so its `id` and `created_at` survive and its `rev` advances. Delete-then-insert
//! would have been simpler and would have made every edit look like a new rule in the
//! change feed, which is the opposite of what a change feed is for.
//!
//! # The one reference the database cannot enforce
//!
//! `entity_id` is polymorphic, so no foreign key covers it. Three things stand in for one:
//!
//! 1. [`Availability::set`] checks the entity exists before writing.
//! 2. [`Availability::clear_for_entity`] is called by every repository that deletes an
//!    entity which can carry rules, inside the same transaction.
//! 3. [`Availability::orphans`] sweeps for anything that got through, so a bug here is
//!    findable rather than invisible.
//!
//! The time references — cycle day, period, timeslot, term — *are* enforced, and are
//! RESTRICT. Deleting a cycle day that availability is configured against is refused rather
//! than quietly taking the rules with it.

use rusqlite::{OptionalExtension, Transaction};
use uuid::Uuid;

use super::row::{entity, opt_text, text};
use super::{Stamp, delete, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// Handle for reading and writing availability rules.
#[derive(Debug)]
pub struct Availability<'a> {
    db: &'a mut Database,
}

impl<'a> Availability<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

/// What kind of thing a rule is about.
///
/// The schema checks the same list, so a project written by a newer build fails loudly here
/// rather than decoding an unknown kind as a plausible one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum AvailabilityEntity {
    /// A member of staff.
    Teacher,
    /// A teaching space.
    Room,
    /// An audience — a class, cohort, homeroom or set.
    StudentGroup,
    /// Shared equipment.
    Resource,
    /// A whole site.
    Campus,
    /// A subject. Reserved: `subject` arrives with Curriculum.
    Subject,
}

impl AvailabilityEntity {
    /// The value stored in `availability.entity_kind`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Teacher => "TEACHER",
            Self::Room => "ROOM",
            Self::StudentGroup => "STUDENT_GROUP",
            Self::Resource => "RESOURCE",
            Self::Campus => "CAMPUS",
            Self::Subject => "SUBJECT",
        }
    }

    /// The table holding the rows this kind refers to, or `None` when it does not exist yet.
    const fn table(self) -> Option<&'static str> {
        match self {
            Self::Teacher => Some("teacher"),
            Self::Room => Some("room"),
            Self::StudentGroup => Some("student_group"),
            Self::Resource => Some("resource"),
            Self::Campus => Some("campus"),
            // `subject` arrives with Curriculum. Accepting rules against a table that does
            // not exist would create orphans by construction.
            Self::Subject => None,
        }
    }

    /// Human-readable, for messages.
    const fn noun(self) -> &'static str {
        match self {
            Self::Teacher => "teacher",
            Self::Room => "room",
            Self::StudentGroup => "student group",
            Self::Resource => "resource",
            Self::Campus => "campus",
            Self::Subject => "subject",
        }
    }
}

impl super::row::FromColumn for AvailabilityEntity {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "TEACHER" => Ok(Self::Teacher),
            "ROOM" => Ok(Self::Room),
            "STUDENT_GROUP" => Ok(Self::StudentGroup),
            "RESOURCE" => Ok(Self::Resource),
            "CAMPUS" => Ok(Self::Campus),
            "SUBJECT" => Ok(Self::Subject),
            _ => Err(corrupt("availability.entity_kind", index, raw)),
        }
    }
}

/// What a rule says about its slot.
///
/// There is no `AVAILABLE`. Available is the absence of a rule — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum AvailabilityStatus {
    /// Cannot be timetabled here. A hard rule; `weight` is meaningless.
    Unavailable,
    /// Would rather be timetabled here. Soft.
    Preferred,
    /// Would rather not be timetabled here, but can be. Soft.
    Discouraged,
}

impl AvailabilityStatus {
    /// The value stored in `availability.status`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "UNAVAILABLE",
            Self::Preferred => "PREFERRED",
            Self::Discouraged => "DISCOURAGED",
        }
    }
}

impl super::row::FromColumn for AvailabilityStatus {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "UNAVAILABLE" => Ok(Self::Unavailable),
            "PREFERRED" => Ok(Self::Preferred),
            "DISCOURAGED" => Ok(Self::Discouraged),
            _ => Err(corrupt("availability.status", index, raw)),
        }
    }
}

/// A stored enumeration value the schema's `CHECK` should have refused.
///
/// Corruption, not input — the same rule identifiers follow. Guessing a plausible value
/// would make a damaged file look like a working one.
fn corrupt(column: &'static str, index: usize, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(crate::error::CorruptId {
            column: column.to_owned(),
            value,
        }),
    )
}

entity! {
    /// One availability rule.
    AvailabilityRule,
    /// Values for writing an availability rule.
    AvailabilityRuleInput,
    {
        /// Owning school.
        school_id: Uuid,
        /// What kind of thing this is about.
        entity_kind: AvailabilityEntity,
        /// Which row of that kind. Polymorphic, so not a foreign key.
        entity_id: Uuid,
        /// One exact materialised slot, when the rule is that specific.
        timeslot_id: Option<Uuid>,
        /// Which day of the cycle. With `period_id`, one recurring cell.
        cycle_day_id: Option<Uuid>,
        /// Which period. Alone, it means that period on every day.
        period_id: Option<Uuid>,
        /// Which term, or `None` for all year.
        term_id: Option<Uuid>,
        /// What the rule says.
        status: AvailabilityStatus,
        /// Soft weight for `PREFERRED` and `DISCOURAGED`. `None` is unset.
        weight: Option<i64>,
        /// Why, in the school's own words.
        reason: Option<String>,
        /// First date the rule applies, ISO-8601. `None` means always.
        effective_from: Option<String>,
        /// Last date the rule applies, ISO-8601. `None` means always.
        effective_to: Option<String>,
    }
}

const AVAILABILITY_COLUMNS: &str = "id, school_id, entity_kind, entity_id, timeslot_id, \
     cycle_day_id, period_id, term_id, status, weight, reason, effective_from, \
     effective_to, created_at, updated_at, rev";

const AVAILABILITY_ASSIGNMENTS: &str = "status = ?, weight = ?, reason = ?, \
     effective_from = ?, effective_to = ?";

impl Availability<'_> {
    /// Writes a rule, replacing whatever already covers the same scope.
    ///
    /// The scope is `(entity_kind, entity_id, timeslot_id, cycle_day_id, period_id,
    /// term_id)` — the same tuple the unique index enforces. An existing rule for that
    /// scope is **updated in place**, so its identity and `created_at` survive and its
    /// `rev` advances; only a genuinely new scope inserts.
    ///
    /// # Errors
    ///
    /// - [`DbError::NotFound`] if the entity does not exist.
    /// - [`DbError::Invalid`] if the rule names no time at all, or if the kind has no
    ///   table yet (`SUBJECT`) — a phase boundary rather than a missing row, and the
    ///   message says so.
    pub fn set(&mut self, input: &AvailabilityRuleInput) -> Result<AvailabilityRule> {
        if input.timeslot_id.is_none() && input.cycle_day_id.is_none() && input.period_id.is_none()
        {
            return Err(DbError::Invalid {
                message: "an availability rule must name a timeslot, a cycle day or a period"
                    .to_owned(),
            });
        }

        let stamp = Stamp::new();
        let id = self.db.write(|tx| {
            ensure_entity_exists(tx, input.entity_kind, input.entity_id)?;

            if let Some(existing) = find_scope(tx, input)? {
                update(
                    tx,
                    "availability",
                    "availability",
                    existing,
                    AVAILABILITY_ASSIGNMENTS,
                    rusqlite::params![
                        input.status.as_str(),
                        input.weight,
                        &input.reason,
                        &input.effective_from,
                        &input.effective_to,
                    ],
                )?;
                return Ok(existing);
            }

            tx.execute(
                "INSERT INTO availability (id, school_id, entity_kind, entity_id, timeslot_id,
                     cycle_day_id, period_id, term_id, status, weight, reason, effective_from,
                     effective_to, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    input.entity_kind.as_str(),
                    text(input.entity_id),
                    opt_text(input.timeslot_id),
                    opt_text(input.cycle_day_id),
                    opt_text(input.period_id),
                    opt_text(input.term_id),
                    input.status.as_str(),
                    input.weight,
                    &input.reason,
                    &input.effective_from,
                    &input.effective_to,
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, "availability"))?;
            Ok(stamp.id)
        })?;

        self.rule(id)?.ok_or(DbError::NotFound {
            entity: "availability",
        })
    }

    /// Removes the rule covering one scope, if there is one.
    ///
    /// Returns whether anything went. Clearing a cell that was already available is not an
    /// error — the grid sends the same request whichever way a cell was toggled, and
    /// "there was nothing to remove" is the correct outcome rather than a failure.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn clear(&mut self, scope: &AvailabilityRuleInput) -> Result<bool> {
        self.db.write(|tx| match find_scope(tx, scope)? {
            Some(id) => {
                delete(tx, "availability", "availability", id)?;
                Ok(true)
            }
            None => Ok(false),
        })
    }

    /// One rule by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn rule(&self, id: Uuid) -> Result<Option<AvailabilityRule>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {AVAILABILITY_COLUMNS} FROM availability WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], AvailabilityRule::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every rule for one entity, in a stable order.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn rules_for(
        &self,
        school_id: Uuid,
        entity_kind: AvailabilityEntity,
        entity_id: Uuid,
    ) -> Result<Vec<AvailabilityRule>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {AVAILABILITY_COLUMNS} FROM availability
                 WHERE school_id = ? AND entity_kind = ? AND entity_id = ?
                 ORDER BY cycle_day_id, period_id, id"
            ))?;
            let rows = stmt.query_map(
                [&text(school_id), entity_kind.as_str(), &text(entity_id)],
                AvailabilityRule::from_row,
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }

    /// How many rules one entity has. For a confirmation that says so before it asks.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn count_for(
        &self,
        school_id: Uuid,
        entity_kind: AvailabilityEntity,
        entity_id: Uuid,
    ) -> Result<i64> {
        self.db.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM availability
                 WHERE school_id = ? AND entity_kind = ? AND entity_id = ?",
                [&text(school_id), entity_kind.as_str(), &text(entity_id)],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// Rules whose entity no longer exists.
    ///
    /// The integrity sweep `DATA-MODEL.md` §6 calls for. `entity_id` is the one reference
    /// in this schema the database cannot enforce, so this is how a mistake in the code
    /// that is meant to enforce it becomes findable rather than invisible.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn orphans(&self) -> Result<Vec<AvailabilityRule>> {
        let mut found = Vec::new();
        for rule in self.all()? {
            let present = self.db.read(|conn| {
                let Some(table) = rule.entity_kind.table() else {
                    return Ok(false);
                };
                let hit: Option<String> = conn
                    .query_row(
                        &format!("SELECT id FROM {} WHERE id = ?", super::identifier(table)?),
                        [text(rule.entity_id)],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(hit.is_some())
            })?;
            if !present {
                found.push(rule);
            }
        }
        Ok(found)
    }

    /// Every rule in the project, for the integrity sweep.
    fn all(&self) -> Result<Vec<AvailabilityRule>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {AVAILABILITY_COLUMNS} FROM availability ORDER BY id"
            ))?;
            let rows = stmt.query_map([], AvailabilityRule::from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DbError::from)
        })
    }
}

/// Identifier of the rule covering one scope, if any.
///
/// Scoped by `school_id` as well as by the identity index's own columns. That index does
/// not include the school, so without this a clear sent by one school could match a rule
/// another school had recorded -- and `clear` deletes whatever this finds. V12's triggers
/// make the combination unreachable today, because a rule's time references must belong to
/// its own school; this closes it at the query rather than relying on that staying true.
fn find_scope(tx: &Transaction<'_>, scope: &AvailabilityRuleInput) -> Result<Option<Uuid>> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT id FROM availability
             WHERE school_id = ?
               AND entity_kind = ?
               AND entity_id = ?
               AND COALESCE(timeslot_id, '') = ?
               AND COALESCE(cycle_day_id, '') = ?
               AND COALESCE(period_id, '') = ?
               AND COALESCE(term_id, '') = ?",
            rusqlite::params![
                text(scope.school_id),
                scope.entity_kind.as_str(),
                text(scope.entity_id),
                opt_text(scope.timeslot_id).unwrap_or_default(),
                opt_text(scope.cycle_day_id).unwrap_or_default(),
                opt_text(scope.period_id).unwrap_or_default(),
                opt_text(scope.term_id).unwrap_or_default(),
            ],
            |row| row.get(0),
        )
        .optional()?;

    raw.map(|value| super::row::parse_id_text("availability.id", &value))
        .transpose()
}

/// Fails unless the entity this rule is about exists.
///
/// Stands in for the foreign key a polymorphic reference cannot have.
fn ensure_entity_exists(tx: &Transaction<'_>, kind: AvailabilityEntity, id: Uuid) -> Result<()> {
    let Some(table) = kind.table() else {
        return Err(DbError::Invalid {
            message: format!(
                "availability cannot be set for a {} yet — that part of the model arrives \
                 with Curriculum",
                kind.noun()
            ),
        });
    };

    let found: Option<String> = tx
        .query_row(
            &format!("SELECT id FROM {} WHERE id = ?", super::identifier(table)?),
            [text(id)],
            |row| row.get(0),
        )
        .optional()?;

    if found.is_none() {
        return Err(DbError::NotFound {
            entity: kind.noun(),
        });
    }
    Ok(())
}

/// Removes every rule about one entity, inside the caller's transaction.
///
/// Called by each repository that deletes something which can carry rules. Availability
/// belongs to its entity and is meaningless without it, so this is the one place where
/// removal is the right answer rather than a refusal — and it goes through
/// [`super::delete`] per row so each removal is a truthful change-log entry.
pub(crate) fn clear_for_entity(
    tx: &Transaction<'_>,
    kind: AvailabilityEntity,
    entity_id: Uuid,
) -> Result<usize> {
    let ids = {
        let mut stmt = tx.prepare(
            "SELECT id FROM availability WHERE entity_kind = ? AND entity_id = ? ORDER BY id",
        )?;
        let rows = stmt.query_map([kind.as_str(), &text(entity_id)], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let removed = ids.len();
    for raw in ids {
        let id = super::row::parse_id_text("availability.id", &raw)?;
        delete(tx, "availability", "availability", id)?;
    }
    Ok(removed)
}
