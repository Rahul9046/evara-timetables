//! Configured scheduling rules: `constraint_instance` and its scope rows.
//!
//! # What a row is *for*, when every rule already runs
//!
//! The eight implemented types are mandatory in the **registry** — `evara-constraints` runs
//! them whatever this table holds, so an unconfigured project is fully checked and no row's
//! deletion can switch a safety rule off. A row here therefore does not *add* a rule; it
//! **tunes** one. A weight, a term, a level a school has raised.
//!
//! That is ADR 0005's reason for constraints-as-data standing unchanged — *"schools genuinely
//! disagree about which rules are hard, and that disagreement is the product"* — with ADR
//! 0039's answer to the question ADR 0005 left open: a rule whose absence would make a
//! timetable wrong is not stored data to begin with.
//!
//! # Where a stored value is validated, and where it is not
//!
//! Every refusal this repository makes comes from `evara-core`: an unknown `type_key`, a level
//! that is not one of the three, a mandatory rule weakened or disabled, a narrowing the type
//! does not support. [`Registry::compile`] makes the same refusals when it reads the row back,
//! so a row this repository accepted is a row the engine can compile.
//!
//! The one exception is the **shape of `params`**. ADR 0005 says the JSON Schema *"validates
//! writes and generates the configuration form"*, and the schemas live beside the evaluators
//! in `evara-constraints` — which `evara-db` may not depend on (`ARCHITECTURE.md` §3). So this
//! crate checks what it can, which is that `params` is a JSON object, and the *shape* is
//! checked by the layer that can see both crates: `Registry::compile` refuses the whole
//! compilation on a malformed parameter, and a caller that is about to write a row calls it
//! first. The gap is recorded in ADR 0039 §5 rather than closed by a dependency that
//! `ARCHITECTURE.md` forbids.
//!
//! # `constraint_ref` is readable and writable, and in M5 every write to it is refused
//!
//! None of the eight types can be narrowed — a teacher clash is a clash wherever it happens,
//! and a `constraint_ref` scoping one would either do nothing or silently switch a safety rule
//! off for part of a school. [`ConstraintType::supports_narrowing`] says so and
//! [`Constraints::add_scope`] refuses accordingly, naming the rule. The table exists anyway
//! because the first narrowable type needs it, because a project written by a newer build may
//! carry rows this build must not lose, and because [`Constraints::delete`] has to clear them
//! — which it does itself, row by row through [`repo::delete`](super::delete), since a
//! `CASCADE` would remove them with no change-log entry (V16's deviation 2).
//!
//! The third of the three things that stand in for the foreign key `entity_id` cannot have —
//! *the repository that deletes a referable entity clears its scope rows*, as every repository
//! deleting an entity that can carry availability rules calls `clear_for_entity` — is
//! deliberately **not** here. Nothing in this crate can create a scope row, so there would be
//! nothing for those eleven call sites to clear, and code with no reachable behaviour is code
//! no test can hold honest. The first narrowable type brings them with it;
//! [`Constraints::orphans`] is the sweep that covers a newer build's rows until then.
//!
//! [`Registry::compile`]: https://docs.rs/evara-constraints

use evara_core::scheduling::{
    ConstraintLevel, ConstraintType, EntityKind, ScopeRef, StoredConstraint,
};
use rusqlite::{Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use super::row::{entity, opt_text, text};
use super::{Expected, Stamp, delete, require_same_school, translate, update};
use crate::Database;
use crate::error::{DbError, Result};

/// The entity name errors from this repository carry.
const ENTITY: &str = "constraint";

/// The entity name a scope row's errors carry.
const SCOPE: &str = "constraint scope";

/// Handle for reading and writing configured constraints.
#[derive(Debug)]
pub struct Constraints<'a> {
    db: &'a mut Database,
}

impl<'a> Constraints<'a> {
    pub(crate) fn new(db: &'a mut Database) -> Self {
        Self { db }
    }
}

/// What a scope row is *for*.
///
/// `CONSTRAINTS.md` §2's roles. A rule with no scope row at all is school-wide, which is every
/// rule in the MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ScopeRole {
    /// The entities the rule is about.
    AppliesTo,
    /// Narrowed to a subject.
    Subject,
    /// Narrowed to a year level.
    YearLevel,
    /// Narrowed to a campus.
    Campus,
    /// Narrowed to one materialised slot.
    Timeslot,
    /// Narrowed to one position in the cycle.
    CycleDay,
    /// Narrowed to one period of a bell schedule.
    Period,
}

impl ScopeRole {
    /// The value stored in `constraint_ref.role`.
    ///
    /// `camelCase`, which is what the schema's `CHECK` lists and what `CONSTRAINTS.md` spells
    /// — the one place in this schema where a stored enumeration is not
    /// `SCREAMING_SNAKE_CASE`, because the role names come from the constraint parameter
    /// vocabulary rather than from a column of this crate's own invention.
    const fn as_str(self) -> &'static str {
        match self {
            Self::AppliesTo => "appliesTo",
            Self::Subject => "subject",
            Self::YearLevel => "yearLevel",
            Self::Campus => "campus",
            Self::Timeslot => "timeslot",
            Self::CycleDay => "cycleDay",
            Self::Period => "period",
        }
    }
}

impl super::row::FromColumn for ScopeRole {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "appliesTo" => Ok(Self::AppliesTo),
            "subject" => Ok(Self::Subject),
            "yearLevel" => Ok(Self::YearLevel),
            "campus" => Ok(Self::Campus),
            "timeslot" => Ok(Self::Timeslot),
            "cycleDay" => Ok(Self::CycleDay),
            "period" => Ok(Self::Period),
            _ => Err(corrupt("constraint_ref.role", index, raw)),
        }
    }
}

/// What kind of row a scope reference names.
///
/// Polymorphic, so no foreign key covers `entity_id` — the same hole `availability.entity_id`
/// has. [`Constraints::orphans`] is what makes a mistake findable rather than invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ScopeEntity {
    /// A member of staff.
    Teacher,
    /// A teaching space.
    Room,
    /// An audience.
    StudentGroup,
    /// One deliverable instance of a course.
    ClassSection,
    /// One placeable occurrence.
    Activity,
    /// A subject the school teaches.
    Subject,
    /// A year level.
    YearLevel,
    /// A site.
    Campus,
    /// One materialised slot.
    Timeslot,
    /// One position in the cycle.
    CycleDay,
    /// One period of a bell schedule.
    Period,
}

impl ScopeEntity {
    /// The value stored in `constraint_ref.entity_kind`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Teacher => "TEACHER",
            Self::Room => "ROOM",
            Self::StudentGroup => "STUDENT_GROUP",
            Self::ClassSection => "CLASS_SECTION",
            Self::Activity => "ACTIVITY",
            Self::Subject => "SUBJECT",
            Self::YearLevel => "YEAR_LEVEL",
            Self::Campus => "CAMPUS",
            Self::Timeslot => "TIMESLOT",
            Self::CycleDay => "CYCLE_DAY",
            Self::Period => "PERIOD",
        }
    }

    /// The table the row lives in.
    ///
    /// Every one of the eleven exists at `V16`, which is why this returns a `&'static str`
    /// where `AvailabilityEntity::table` returns an `Option` — `availability` predates the
    /// tables three of its kinds name, and this does not.
    const fn table(self) -> &'static str {
        match self {
            Self::Teacher => "teacher",
            Self::Room => "room",
            Self::StudentGroup => "student_group",
            Self::ClassSection => "class_section",
            Self::Activity => "activity",
            Self::Subject => "subject",
            Self::YearLevel => "year_level",
            Self::Campus => "campus",
            Self::Timeslot => "timeslot",
            Self::CycleDay => "cycle_day",
            Self::Period => "period",
        }
    }

    /// The scheduling vocabulary's name for this kind.
    ///
    /// The mapping exists so a scope row reaches `evara-constraints` as a
    /// [`ScopeRef`] rather than as two strings — one vocabulary for an entity reference across
    /// the whole scheduling path, which is what `evara_core::scheduling::diagnostics` is for.
    const fn kind(self) -> EntityKind {
        match self {
            Self::Teacher => EntityKind::Teacher,
            Self::Room => EntityKind::Room,
            Self::StudentGroup => EntityKind::StudentGroup,
            Self::ClassSection => EntityKind::ClassSection,
            Self::Activity => EntityKind::Activity,
            Self::Subject => EntityKind::Subject,
            Self::YearLevel => EntityKind::YearLevel,
            Self::Campus => EntityKind::Campus,
            Self::Timeslot => EntityKind::Timeslot,
            Self::CycleDay => EntityKind::CycleDay,
            Self::Period => EntityKind::Period,
        }
    }
}

impl super::row::FromColumn for ScopeEntity {
    fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Self> {
        let raw: String = row.get(index)?;
        match raw.as_str() {
            "TEACHER" => Ok(Self::Teacher),
            "ROOM" => Ok(Self::Room),
            "STUDENT_GROUP" => Ok(Self::StudentGroup),
            "CLASS_SECTION" => Ok(Self::ClassSection),
            "ACTIVITY" => Ok(Self::Activity),
            "SUBJECT" => Ok(Self::Subject),
            "YEAR_LEVEL" => Ok(Self::YearLevel),
            "CAMPUS" => Ok(Self::Campus),
            "TIMESLOT" => Ok(Self::Timeslot),
            "CYCLE_DAY" => Ok(Self::CycleDay),
            "PERIOD" => Ok(Self::Period),
            _ => Err(corrupt("constraint_ref.entity_kind", index, raw)),
        }
    }
}

/// A stored enumeration value the schema's `CHECK` should have refused.
///
/// Corruption, not input — [`row::parse_id`](super::row)'s rule for identifiers, applied to an
/// enumeration. Decoding an unknown role as a plausible one would make a damaged file look
/// like a working one.
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
    /// One configured rule.
    ConstraintInstance,
    /// Values for writing a configured rule.
    ConstraintInstanceInput,
    {
        /// Owning school. Generation is scoped to one school (ADR 0032 rule 1), so a rule is
        /// too.
        school_id: Uuid,
        /// The registry's identity for the rule, as stored.
        ///
        /// A `String` rather than a [`ConstraintType`], because a project written by a newer
        /// build keeps rules this one does not implement rather than failing to read the
        /// table at all. Creating one is refused; reading one back is not.
        type_key: String,
        /// `HARD`, `MEDIUM` or `SOFT`, as stored.
        ///
        /// Also a `String`, and for a weaker version of the same reason: the three levels are
        /// `CHECK`ed, so a fourth can only arrive with a future migration, and a build that
        /// met one should report an unknown level rather than refuse to list the table.
        level: String,
        /// The integer penalty one violation contributes. Never a float — `CLAUDE.md` §3.
        weight: i64,
        /// Whether the school wants it evaluated. A mandatory rule may not be turned off.
        enabled: bool,
        /// Typed parameters as stored JSON text.
        params: String,
        /// The term it applies to, or `None` for the whole year.
        term_id: Option<Uuid>,
    }
}

entity! {
    /// One entity a rule is narrowed to.
    ConstraintScopeEntry,
    /// Values for writing a scope row.
    ConstraintScopeEntryInput,
    {
        /// The rule this narrows.
        constraint_id: Uuid,
        /// What the reference is for.
        role: ScopeRole,
        /// What kind of row `entity_id` names.
        entity_kind: ScopeEntity,
        /// Which row. Polymorphic, so not a foreign key.
        entity_id: Uuid,
    }
}

const CONSTRAINT_COLUMNS: &str = "id, school_id, type_key, level, weight, enabled, params, \
     term_id, created_at, updated_at, rev";

const CONSTRAINT_ASSIGNMENTS: &str = "level = ?, weight = ?, enabled = ?, params = ?, \
     term_id = ?";

const SCOPE_COLUMNS: &str = "id, constraint_id, role, entity_kind, entity_id, created_at, \
     updated_at, rev";

impl Constraints<'_> {
    /// Configures a rule.
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] if the `type_key` is not a rule this build implements, the level
    ///   is not one of the three, `params` is not a JSON object, the weight is negative, or
    ///   the row would weaken or disable a mandatory rule. Each message names the remedy;
    ///   together they are exactly the refusals `Registry::compile` makes, so a row that goes
    ///   in is a row that comes back out compilable.
    /// - [`DbError::Invalid`] if the term belongs to another school, raised by the trigger
    ///   pair V16 installs for it.
    /// - [`DbError::NotFound`] if the school or term does not exist.
    pub fn create(&mut self, input: &ConstraintInstanceInput) -> Result<ConstraintInstance> {
        validate(input)?;

        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_term_in_school(tx, input.term_id, input.school_id)?;
            tx.execute(
                "INSERT INTO constraint_instance (id, school_id, type_key, level, weight,
                     enabled, params, term_id, created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.school_id),
                    &input.type_key,
                    &input.level,
                    input.weight,
                    input.enabled,
                    &input.params,
                    opt_text(input.term_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, ENTITY))?;
            Ok(())
        })?;

        self.instance(stamp.id)?
            .ok_or(DbError::NotFound { entity: ENTITY })
    }

    /// One configured rule by identifier.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn instance(&self, id: Uuid) -> Result<Option<ConstraintInstance>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CONSTRAINT_COLUMNS} FROM constraint_instance WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(id)], ConstraintInstance::from_row)?;
            rows.next().transpose().map_err(DbError::from)
        })
    }

    /// Every rule one school has configured, in a stable order.
    ///
    /// Ordered by `(type_key, id)`, which is what makes `Registry::compile`'s "a type with
    /// several rows keeps the first" deterministic rather than whatever the query planner
    /// returned today.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn list(&self, school_id: Uuid) -> Result<Vec<ConstraintInstance>> {
        self.db.read(|conn| instances(conn, school_id))
    }

    /// One school's configuration, in the form the registry compiles.
    ///
    /// The hand-over `ARCHITECTURE.md` §3 shapes: this crate reads the rows,
    /// `evara-constraints` makes sense of them, and [`StoredConstraint`] is the type they both
    /// name because it lives in the crate underneath both. Nothing is interpreted here — the
    /// `type_key`, the level and the `params` text travel exactly as stored, and a value this
    /// build cannot read becomes a named error from the registry rather than a row quietly
    /// dropped on the way.
    ///
    /// Scope rows travel with their instance, with the **role dropped**: [`ScopeRef`] names a
    /// kind and a row, and every role is a form of narrowing, so an instance that carries any
    /// scope row at all is narrowed. That is the distinction `Registry::compile` acts on, and
    /// preserving a role it cannot use would be carrying a field to be ignored.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn stored_for(&self, school_id: Uuid) -> Result<Vec<StoredConstraint>> {
        self.db.read(|conn| {
            let mut stored = Vec::new();
            for instance in instances(conn, school_id)? {
                let applies_to = scope_rows(conn, instance.id)?
                    .into_iter()
                    .map(|row| ScopeRef {
                        kind: row.entity_kind.kind(),
                        id: row.entity_id,
                    })
                    .collect();
                stored.push(StoredConstraint {
                    id: instance.id,
                    type_key: instance.type_key,
                    level: instance.level,
                    weight: instance.weight,
                    enabled: instance.enabled,
                    params: instance.params,
                    term: instance.term_id,
                    applies_to,
                });
            }
            Ok(stored)
        })
    }

    /// Retunes a configured rule.
    ///
    /// The rule's **type cannot change**. An instance is a configuration *of* a rule, and
    /// turning a tuned `room-clash` into a tuned `teacher-clash` in place would carry that
    /// rule's parameters, scope and history onto a different rule. The remedy is two
    /// operations, and the error says so.
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] for everything [`Constraints::create`] refuses, plus a changed
    ///   `type_key`.
    /// - [`DbError::WrongSchool`] if the row belongs to another school.
    /// - [`DbError::Conflict`] if it has moved on since `expected` was read.
    /// - [`DbError::NotFound`] if it is not there.
    pub fn update(
        &mut self,
        id: Uuid,
        expected: Expected,
        input: &ConstraintInstanceInput,
    ) -> Result<ConstraintInstance> {
        validate(input)?;

        self.db.write(|tx| {
            require_same_school(tx, ENTITY, "constraint_instance", id, input.school_id)?;
            require_term_in_school(tx, input.term_id, input.school_id)?;

            let held: String = tx.query_row(
                "SELECT type_key FROM constraint_instance WHERE id = ?",
                [text(id)],
                |row| row.get(0),
            )?;
            if held != input.type_key {
                return Err(DbError::Invalid {
                    message: format!(
                        "this constraint configures `{held}`; to configure `{}` instead, \
                         create one for it and remove this",
                        input.type_key
                    ),
                });
            }

            update(
                tx,
                ENTITY,
                "constraint_instance",
                id,
                expected,
                CONSTRAINT_ASSIGNMENTS,
                rusqlite::params![
                    &input.level,
                    input.weight,
                    input.enabled,
                    &input.params,
                    opt_text(input.term_id),
                ],
            )?;
            Ok(())
        })?;

        self.instance(id)?
            .ok_or(DbError::NotFound { entity: ENTITY })
    }

    /// Removes a configured rule and its scope rows.
    ///
    /// The rule itself does **not** stop being evaluated: the eight types are mandatory in the
    /// registry, so this returns the school to the shipped level and weight rather than
    /// switching anything off. ADR 0039.
    ///
    /// Scope rows go first, one `DELETE` each through [`repo::delete`](super::delete), inside
    /// this transaction. V16's `constraint_instance_delete_requires_no_scope` trigger refuses
    /// the instance while any remain, which is what keeps that from being a convention — a
    /// `CASCADE` would have removed them with no change-log entry, because SQLite fires a
    /// `DELETE` trigger for a foreign-key action only under `recursive_triggers` and this
    /// project never enables it.
    ///
    /// They are removed at [`Expected::Any`]: the caller read the *instance's* revision, not
    /// each scope row's, and demanding one would mean inventing it. That is the first of the
    /// three documented kinds — a cascade inside another operation's transaction.
    ///
    /// # Errors
    ///
    /// - [`DbError::WrongSchool`] if the row belongs to another school.
    /// - [`DbError::Conflict`] if it has moved on since `expected` was read, in which case
    ///   **nothing** is removed: the scope deletions share the transaction and roll back with
    ///   it.
    /// - [`DbError::NotFound`] if it is not there.
    pub fn delete(&mut self, school_id: Uuid, id: Uuid, expected: Expected) -> Result<()> {
        self.db.write(|tx| {
            require_same_school(tx, ENTITY, "constraint_instance", id, school_id)?;
            for scope in scope_rows(tx, id)? {
                delete(tx, SCOPE, "constraint_ref", scope.id, Expected::Any)?;
            }
            delete(tx, ENTITY, "constraint_instance", id, expected)
        })
    }

    /// What one rule is narrowed to, in a stable order.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn scope_of(&self, constraint_id: Uuid) -> Result<Vec<ConstraintScopeEntry>> {
        self.db.read(|conn| scope_rows(conn, constraint_id))
    }

    /// Narrows a rule to one entity.
    ///
    /// # Refused for every rule this build implements
    ///
    /// None of the eight can be narrowed — [`ConstraintType::supports_narrowing`] — because
    /// each is a property of the whole timetable. Narrowing `teacher-clash` to one teacher
    /// does not mean "check this teacher carefully"; it means "stop checking everybody else",
    /// and a safety rule that is off for part of a school is worse than one that is off
    /// everywhere, because nothing says so.
    ///
    /// So this refuses, by name, rather than storing a row `Registry::compile` would then
    /// refuse to compile — which would leave a school's whole configuration unusable because
    /// of one scope row somebody added in a screen that let them. The method exists because
    /// the refusal has to be explicit and because the first narrowable type arrives through
    /// here.
    ///
    /// # Errors
    ///
    /// - [`DbError::Invalid`] if the rule's type does not support narrowing, or is not a type
    ///   this build implements.
    /// - [`DbError::WrongSchool`] if the rule belongs to another school.
    /// - [`DbError::NotFound`] if the rule is not there.
    pub fn add_scope(
        &mut self,
        school_id: Uuid,
        input: &ConstraintScopeEntryInput,
    ) -> Result<ConstraintScopeEntry> {
        let stamp = Stamp::new();
        self.db.write(|tx| {
            require_same_school(
                tx,
                ENTITY,
                "constraint_instance",
                input.constraint_id,
                school_id,
            )?;

            let key: String = tx.query_row(
                "SELECT type_key FROM constraint_instance WHERE id = ?",
                [text(input.constraint_id)],
                |row| row.get(0),
            )?;
            let Some(constraint_type) = ConstraintType::from_key(&key) else {
                return Err(DbError::Invalid {
                    message: format!(
                        "this constraint configures `{key}`, which this version of Evara does \
                         not implement, so it cannot say what narrowing it would mean"
                    ),
                });
            };
            if !constraint_type.supports_narrowing() {
                return Err(DbError::Invalid {
                    message: format!(
                        "`{}` applies to the whole school and cannot be narrowed — narrowing \
                         it would switch it off everywhere it was not named",
                        constraint_type.key()
                    ),
                });
            }

            tx.execute(
                "INSERT INTO constraint_ref (id, constraint_id, role, entity_kind, entity_id,
                     created_at, updated_at, rev)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    text(stamp.id),
                    text(input.constraint_id),
                    input.role.as_str(),
                    input.entity_kind.as_str(),
                    text(input.entity_id),
                    &stamp.created_at,
                    &stamp.updated_at,
                    stamp.rev,
                ],
            )
            .map_err(|error| translate(error, SCOPE))?;
            Ok(())
        })?;

        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SCOPE_COLUMNS} FROM constraint_ref WHERE id = ?"
            ))?;
            let mut rows = stmt.query_map([text(stamp.id)], ConstraintScopeEntry::from_row)?;
            rows.next()
                .transpose()
                .map_err(DbError::from)?
                .ok_or(DbError::NotFound { entity: SCOPE })
        })
    }

    /// Scope rows whose entity no longer exists.
    ///
    /// `entity_id` is polymorphic and so carries no foreign key — the same hole
    /// `availability.entity_id` has, and the same integrity sweep `DATA-MODEL.md` §6 calls
    /// for. Nothing in this crate can *create* one today, so anything this finds came from a
    /// newer build or from a hand-edited file, and either way a dangling scope row is a fact
    /// about a row that is gone.
    ///
    /// # Errors
    ///
    /// Propagates any database failure.
    pub fn orphans(&self) -> Result<Vec<ConstraintScopeEntry>> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SCOPE_COLUMNS} FROM constraint_ref ORDER BY constraint_id, role, id"
            ))?;
            let all = stmt
                .query_map([], ConstraintScopeEntry::from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut found = Vec::new();
            for row in all {
                let present: i64 = conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE id = ?",
                        super::identifier(row.entity_kind.table())?
                    ),
                    [text(row.entity_id)],
                    |held| held.get(0),
                )?;
                if present == 0 {
                    found.push(row);
                }
            }
            Ok(found)
        })
    }
}

/// Everything about a stored row this crate can judge on its own.
///
/// The same four refusals `Registry::compile` makes from the same `evara-core` answers, moved
/// to the write so a school hears about a mistake while it is making it rather than when the
/// next solve refuses to start. What is **not** here is the shape of `params` — see this
/// module's header.
fn validate(input: &ConstraintInstanceInput) -> Result<()> {
    let Some(constraint_type) = ConstraintType::from_key(&input.type_key) else {
        let known: Vec<&str> = ConstraintType::all().iter().map(|k| k.key()).collect();
        return Err(DbError::Invalid {
            message: format!(
                "`{}` is not a rule this version of Evara implements; it knows {}",
                input.type_key,
                known.join(", ")
            ),
        });
    };

    let Some(level) = ConstraintLevel::from_stored(&input.level) else {
        return Err(DbError::Invalid {
            message: format!(
                "`{}` is not a level; a constraint is HARD, MEDIUM or SOFT",
                input.level
            ),
        });
    };

    if input.weight < 0 {
        return Err(DbError::Invalid {
            message: String::from("a constraint's weight is a penalty and cannot be negative"),
        });
    }

    if constraint_type.is_mandatory() {
        let shipped = constraint_type.default_level();
        if !input.enabled {
            return Err(DbError::Invalid {
                message: format!(
                    "`{}` is always evaluated and cannot be turned off; remove the \
                     configuration to return it to {}",
                    constraint_type.key(),
                    shipped.stored()
                ),
            });
        }
        if level > shipped {
            return Err(DbError::Invalid {
                message: format!(
                    "`{}` is evaluated at {} or stronger and cannot be weakened to {}",
                    constraint_type.key(),
                    shipped.stored(),
                    level.stored()
                ),
            });
        }
    }

    // The floor the schema also enforces, checked here so the message names the field rather
    // than reporting "a value the project does not allow". The *shape* of the object is the
    // registry's business: `evara-db` cannot see the schemas, which live beside the
    // evaluators.
    let parsed: serde_json::Value =
        serde_json::from_str(&input.params).map_err(|error| DbError::Invalid {
            message: format!("a constraint's parameters must be JSON ({error})"),
        })?;
    if !parsed.is_object() {
        return Err(DbError::Invalid {
            message: String::from(
                "a constraint's parameters must be a JSON object, even an empty one",
            ),
        });
    }

    Ok(())
}

/// Refuses a term belonging to another school, or accepts `None`.
///
/// V16 installs a trigger pair for this, because `term` reaches its school through
/// `academic_year` and no foreign key can say it. This is the repository half of that
/// division, and it exists for the reason `require_course_code_free`'s does:
/// [`translate`](super::translate) reads SQLite's own constraint text and cannot see a
/// `RAISE` message, so without this the user gets "the project database reported an error"
/// where they should get a sentence naming the remedy.
///
/// `Invalid` rather than `WrongSchool`: the *write* is for the right school, and it is one
/// field within it that names somebody else's row. `WrongSchool` would advise switching
/// school, which is the wrong advice when the fix is to pick a different term.
fn require_term_in_school(
    tx: &Transaction<'_>,
    term_id: Option<Uuid>,
    school_id: Uuid,
) -> Result<()> {
    let Some(term_id) = term_id else {
        return Ok(());
    };
    let owner: Option<String> = tx
        .query_row(
            "SELECT academic_year.school_id FROM term
             JOIN academic_year ON academic_year.id = term.academic_year_id
             WHERE term.id = ?",
            [text(term_id)],
            |row| row.get(0),
        )
        .optional()?;
    match owner {
        None => Err(DbError::NotFound { entity: "term" }),
        Some(raw) if raw == school_id.to_string() => Ok(()),
        Some(_) => Err(DbError::Invalid {
            message: String::from(
                "That term belongs to a different school. Choose one of this school's terms,                  or leave the constraint applying all year.",
            ),
        }),
    }
}

/// One school's instances, in `(type_key, id)` order.
fn instances(conn: &Connection, school_id: Uuid) -> Result<Vec<ConstraintInstance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CONSTRAINT_COLUMNS} FROM constraint_instance
         WHERE school_id = ? ORDER BY type_key, id"
    ))?;
    let rows = stmt.query_map([text(school_id)], ConstraintInstance::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::from)
}

/// One instance's scope rows, in `(role, entity_kind, id)` order.
///
/// Takes a [`Connection`], so the read inside [`Constraints::delete`]'s transaction and the
/// one behind [`Constraints::scope_of`] are the same query — `&Transaction` coerces.
fn scope_rows(conn: &Connection, constraint_id: Uuid) -> Result<Vec<ConstraintScopeEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SCOPE_COLUMNS} FROM constraint_ref
         WHERE constraint_id = ? ORDER BY role, entity_kind, id"
    ))?;
    let rows = stmt.query_map([text(constraint_id)], ConstraintScopeEntry::from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::from)
}
