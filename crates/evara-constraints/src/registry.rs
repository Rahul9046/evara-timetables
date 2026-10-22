//! The registry: `type_key` to evaluator, and a stored row to a [`CompiledConstraint`].
//!
//! # A static table, not a plugin host
//!
//! `SOLVER.md` §4: *"Registration is a table in `evara-constraints/src/registry.rs` mapping
//! `type_key` to an evaluator. Adding a constraint means: one module, one registry line, one
//! test file, one row in `CONSTRAINTS.md`."* That is what [`evaluator_for`] is.
//!
//! No dynamic registration and no runtime code loading. ADR 0005's *Alternatives considered*
//! rejects a scripting language because it *"puts arbitrary user code in the solver's inner
//! loop, makes performance unpredictable, and makes `explain()` impossible to do well"*; a
//! plugin registry would have the first two problems without the flexibility that was supposed
//! to justify them.
//!
//! # Mandatory in the registry, not in the data
//!
//! [`Registry::mandatory`] returns one [`CompiledConstraint`] per implemented type, whatever
//! the database holds. That is the answer to *"how are the eight MVP constraints active
//! without the user creating eight rows?"*, and it has a property no amount of seeding gives:
//! **there is no row whose deletion disables a safety rule.** ADR 0039.
//!
//! A stored instance of a mandatory type does not *add* a rule — it *tunes* the one that is
//! already running, and [`Registry::compile`] refuses a row that would disable it or weaken
//! its level. Refusing rather than ignoring matters: a school that set `enabled = 0` on
//! `teacher-clash` and got no error would reasonably believe clash checking was off.

use evara_core::scheduling::{
    CompiledConstraint, ConstraintError, ConstraintLevel, ConstraintScope, ConstraintType,
    StoredConstraint,
};

use crate::ConstraintEvaluator;
use crate::availability::EntityUnavailable;
use crate::clash::{RoomClash, StudentClash, TeacherClash};
use crate::completeness::ActivityScheduled;
use crate::facilities::RoomTypeRequired;
use crate::params;
use crate::timing::{DurationContiguous, NoTeachingInPeriod};

/// The evaluator for one type.
///
/// Returns a `&'static dyn` rather than a boxed value: every evaluator is a unit struct with
/// no state, so there is exactly one of each for the life of the process and nothing needs to
/// allocate to reach it. The dispatch is a `match`, which is what makes "every type has an
/// evaluator" a compiler-checked property rather than a map lookup that can miss.
#[must_use]
pub fn evaluator_for(constraint_type: ConstraintType) -> &'static dyn ConstraintEvaluator {
    match constraint_type {
        ConstraintType::TeacherClash => &TeacherClash,
        ConstraintType::StudentClash => &StudentClash,
        ConstraintType::RoomClash => &RoomClash,
        ConstraintType::RoomTypeRequired => &RoomTypeRequired,
        ConstraintType::EntityUnavailable => &EntityUnavailable,
        ConstraintType::DurationContiguous => &DurationContiguous,
        ConstraintType::NoTeachingInPeriod => &NoTeachingInPeriod,
        ConstraintType::ActivityScheduled => &ActivityScheduled,
    }
}

/// The set of rules the engine knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registry;

impl Registry {
    /// Every implemented type, in evaluation order.
    #[must_use]
    pub fn types() -> &'static [ConstraintType] {
        ConstraintType::all()
    }

    /// Every evaluator, in evaluation order.
    pub fn evaluators() -> impl Iterator<Item = &'static dyn ConstraintEvaluator> {
        Self::types().iter().copied().map(evaluator_for)
    }

    /// The JSON Schema for one type's parameters.
    #[must_use]
    pub fn schema_of(constraint_type: ConstraintType) -> &'static str {
        params::schema_of(constraint_type)
    }

    /// The constraints a project evaluates when it has configured nothing.
    ///
    /// All eight, at their shipped levels, with weight 1 and no narrowing. **Every project
    /// evaluates these**, which is what makes the MVP set mandatory without a single stored
    /// row — see this module's header.
    #[must_use]
    pub fn mandatory() -> Vec<CompiledConstraint> {
        Self::types()
            .iter()
            .copied()
            .map(CompiledConstraint::mandatory)
            .collect()
    }

    /// The constraints to evaluate, given what a project has stored.
    ///
    /// # How a stored row and a mandatory built-in combine
    ///
    /// Each mandatory type runs exactly once. A stored row for one *replaces* the built-in
    /// rather than adding to it, so the type is never evaluated twice and a tuned weight or
    /// term takes effect. A stored row for a type that is not implemented is an error the
    /// caller reports; it is never skipped silently, because a school that configured a rule
    /// is entitled to know this build cannot run it.
    ///
    /// Rows naming a different term are dropped — the same decision the availability compiler
    /// makes about a rule for another term — and a type with several rows keeps the first in
    /// row order, which is deterministic because the repository lists them so.
    ///
    /// # Errors
    ///
    /// The first [`ConstraintError`] any row produces. Compilation is all-or-nothing: a
    /// partially configured engine is one that silently runs some of what was asked for.
    pub fn compile(
        stored: &[StoredConstraint],
        term: Option<uuid::Uuid>,
    ) -> Result<Vec<CompiledConstraint>, ConstraintError> {
        let mut compiled = Self::mandatory();

        for row in stored {
            let tuned = compile_one(row)?;
            if !tuned.applies_to_term(term) {
                continue;
            }
            if let Some(slot) = compiled
                .iter_mut()
                .find(|held| held.constraint_type == tuned.constraint_type && held.id.is_none())
            {
                *slot = tuned;
            }
        }
        Ok(compiled)
    }
}

/// Compiles one stored row, validating every stored value it reads.
fn compile_one(row: &StoredConstraint) -> Result<CompiledConstraint, ConstraintError> {
    let constraint_type =
        ConstraintType::from_key(&row.type_key).ok_or_else(|| ConstraintError::UnknownType {
            key: row.type_key.clone(),
        })?;
    let level =
        ConstraintLevel::from_stored(&row.level).ok_or_else(|| ConstraintError::UnknownLevel {
            level: row.level.clone(),
        })?;

    // A mandatory rule may be tuned and may not be turned off or weakened. `enabled = 0` and
    // a level below the shipped one are the same mistake wearing two hats, and both are
    // refused with the same error so the message can say what the floor is.
    if constraint_type.is_mandatory() {
        let shipped = constraint_type.default_level();
        if !row.enabled || level > shipped {
            return Err(ConstraintError::MandatoryWeakened {
                key: constraint_type.key(),
                shipped: shipped.stored(),
                stored: if row.enabled {
                    level.stored().to_owned()
                } else {
                    format!("{} and disabled", level.stored())
                },
            });
        }
    }

    if !row.applies_to.is_empty() && !constraint_type.supports_narrowing() {
        return Err(ConstraintError::ScopeNotSupported {
            key: constraint_type.key(),
        });
    }

    Ok(CompiledConstraint {
        id: Some(row.id),
        constraint_type,
        level,
        weight: row.weight,
        options: params::parse(constraint_type, &row.params)?,
        scope: ConstraintScope {
            term: row.term,
            applies_to: row.applies_to.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use evara_core::scheduling::{
        ConstraintError, ConstraintLevel, ConstraintType, EntityKind, ScopeRef, StoredConstraint,
    };
    use uuid::Uuid;

    use super::{Registry, evaluator_for};

    fn row(type_key: &str) -> StoredConstraint {
        StoredConstraint {
            id: Uuid::from_u128(1),
            type_key: type_key.to_owned(),
            level: "HARD".to_owned(),
            weight: 1,
            enabled: true,
            params: "{}".to_owned(),
            term: None,
            applies_to: Vec::new(),
        }
    }

    #[test]
    fn all_eight_keys_are_registered_and_dispatch_to_their_own_evaluator() {
        assert_eq!(Registry::types().len(), 8);
        for kind in Registry::types() {
            assert_eq!(
                evaluator_for(*kind).constraint_type(),
                *kind,
                "{} dispatches to the wrong evaluator",
                kind.key()
            );
        }
    }

    #[test]
    fn every_evaluator_publishes_the_schema_the_registry_publishes() {
        for kind in Registry::types() {
            assert_eq!(
                evaluator_for(*kind).param_schema(),
                Registry::schema_of(*kind),
                "{} has two schemas",
                kind.key()
            );
        }
    }

    #[test]
    fn dispatch_is_deterministic() {
        // Same key, same evaluator, every time — asserted on the pointer so a future map
        // cannot introduce a per-call allocation that breaks identity.
        for kind in Registry::types() {
            let first = evaluator_for(*kind);
            let again = evaluator_for(*kind);
            assert!(std::ptr::eq(
                std::ptr::from_ref::<dyn crate::ConstraintEvaluator>(first).cast::<u8>(),
                std::ptr::from_ref::<dyn crate::ConstraintEvaluator>(again).cast::<u8>()
            ));
        }
    }

    #[test]
    fn an_unconfigured_project_evaluates_all_eight() {
        // The mandatory model: no rows, every rule running.
        let mandatory = Registry::mandatory();
        assert_eq!(mandatory.len(), 8);
        assert!(mandatory.iter().all(|c| c.id.is_none()));
        assert_eq!(
            mandatory
                .iter()
                .filter(|c| c.level == ConstraintLevel::Hard)
                .count(),
            7
        );
        assert_eq!(
            mandatory
                .iter()
                .filter(|c| c.level == ConstraintLevel::Medium)
                .count(),
            1
        );
    }

    #[test]
    fn compiling_nothing_gives_the_mandatory_set() {
        assert_eq!(
            Registry::compile(&[], None).expect("no rows is always valid"),
            Registry::mandatory()
        );
    }

    #[test]
    fn a_stored_row_tunes_its_type_rather_than_adding_a_second() {
        let mut tuned = row("teacher-clash");
        tuned.weight = 9;
        let compiled = Registry::compile(&[tuned], None).expect("valid");

        assert_eq!(compiled.len(), 8, "still one rule per type");
        let teacher = compiled
            .iter()
            .find(|c| c.constraint_type == ConstraintType::TeacherClash)
            .expect("it is there");
        assert_eq!(teacher.weight, 9);
        assert_eq!(teacher.id, Some(Uuid::from_u128(1)), "the row it came from");
    }

    #[test]
    fn a_row_for_another_term_is_dropped_and_the_built_in_stands() {
        let mut other_term = row("room-clash");
        other_term.term = Some(Uuid::from_u128(5));
        other_term.weight = 99;

        let compiled = Registry::compile(&[other_term], Some(Uuid::from_u128(6))).expect("valid");
        let room = compiled
            .iter()
            .find(|c| c.constraint_type == ConstraintType::RoomClash)
            .expect("it is there");
        assert_eq!(room.weight, 1, "the term-scoped row does not apply");
        assert_eq!(room.id, None, "so the mandatory built-in is what runs");
    }

    #[test]
    fn an_unimplemented_type_is_an_error_and_not_a_skip() {
        // A school that configured `room-capacity` is entitled to know this build cannot run
        // it, rather than having the row quietly ignored.
        let error = Registry::compile(&[row("room-capacity")], None).expect_err("refused");
        assert!(
            matches!(error, ConstraintError::UnknownType { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("room-capacity"));
    }

    #[test]
    fn a_mandatory_rule_cannot_be_disabled() {
        let mut off = row("student-clash");
        off.enabled = false;
        let error = Registry::compile(&[off], None).expect_err("refused");
        assert!(
            matches!(error, ConstraintError::MandatoryWeakened { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("disabled"), "{error}");
    }

    #[test]
    fn a_mandatory_rule_cannot_be_weakened_below_its_shipped_level() {
        for weaker in ["MEDIUM", "SOFT"] {
            let mut soft = row("teacher-clash");
            soft.level = weaker.to_owned();
            let error = Registry::compile(&[soft], None).expect_err("refused");
            assert!(
                matches!(error, ConstraintError::MandatoryWeakened { .. }),
                "{weaker}: {error:?}"
            );
        }
    }

    #[test]
    fn a_medium_rule_may_be_raised_to_hard() {
        // Tuning upwards is exactly what ADR 0005 stores a level per instance for: a school
        // that will not accept an unplaced lesson runs `activity-scheduled` at HARD.
        let mut raised = row("activity-scheduled");
        raised.level = "HARD".to_owned();
        let compiled = Registry::compile(&[raised], None).expect("raising is allowed");
        let scheduled = compiled
            .iter()
            .find(|c| c.constraint_type == ConstraintType::ActivityScheduled)
            .expect("it is there");
        assert_eq!(scheduled.level, ConstraintLevel::Hard);
    }

    #[test]
    fn an_unknown_level_is_refused() {
        let mut odd = row("room-clash");
        odd.level = "CRITICAL".to_owned();
        let error = Registry::compile(&[odd], None).expect_err("refused");
        assert!(
            matches!(error, ConstraintError::UnknownLevel { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn narrowing_a_rule_that_cannot_be_narrowed_is_refused_not_ignored() {
        // Ignoring it would leave a school believing a rule had been scoped when it had not.
        let mut narrowed = row("room-clash");
        narrowed.applies_to.push(ScopeRef {
            kind: EntityKind::Room,
            id: Uuid::from_u128(3),
        });
        let error = Registry::compile(&[narrowed], None).expect_err("refused");
        assert!(
            matches!(error, ConstraintError::ScopeNotSupported { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn malformed_parameters_refuse_the_whole_compilation() {
        // All-or-nothing: a partially configured engine silently runs some of what was asked.
        let mut bad = row("duration-contiguous");
        bad.params = "{\"allowBreakSpan\": \"yes\"}".to_owned();
        let error = Registry::compile(&[bad], None).expect_err("refused");
        assert!(
            matches!(error, ConstraintError::InvalidParams { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn the_one_parameter_reaches_the_compiled_constraint() {
        let mut spanning = row("duration-contiguous");
        spanning.params = "{\"allowBreakSpan\": true}".to_owned();
        let compiled = Registry::compile(&[spanning], None).expect("valid");
        let duration = compiled
            .iter()
            .find(|c| c.constraint_type == ConstraintType::DurationContiguous)
            .expect("it is there");
        assert!(duration.options.allow_break_span);
    }
}
