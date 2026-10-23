//! The contract the constraint vocabulary has to keep.
//!
//! Three of these tests exist because the thing they check is a *contract with stored data* —
//! a `type_key`, a level name, a serialised form — and the cheapest way for one of those to
//! break is a rename nobody noticed.

use uuid::Uuid;

use super::{
    AffectedSet, CompiledConstraint, ConstraintError, ConstraintLevel, ConstraintOptions,
    ConstraintScope, ConstraintType, Explanation, ScopeRef, Violation, ViolationSink,
};
use crate::scheduling::clock::{TimeInterval, TimeOfDay};
use crate::scheduling::diagnostics::{EntityKind, EntityRef};
use crate::scheduling::index::{ActivityIx, CycleDayIx};
use crate::scheduling::outcome::Score;

fn subject() -> EntityRef {
    EntityRef::new(EntityKind::Teacher, Uuid::from_u128(1), "A. Abara")
}

fn violation(kind: ConstraintType, activities: &[u32]) -> Violation {
    Violation {
        constraint_type: kind,
        level: kind.default_level(),
        constraint_id: None,
        penalty: 1,
        activities: activities.iter().copied().map(ActivityIx::new).collect(),
        subject: subject(),
        related: Vec::new(),
        cycle_day: Some(CycleDayIx::new(0)),
        span: TimeInterval::new(
            TimeOfDay::new(9, 0).expect("valid"),
            TimeOfDay::new(10, 0).expect("valid"),
        ),
        detail: String::from("09:00–10:00 on Day 1"),
    }
}

// =======================================================================================
// Type keys are a contract
// =======================================================================================

#[test]
fn every_type_has_a_distinct_stable_kebab_case_key() {
    let mut keys: Vec<&str> = ConstraintType::all().iter().map(|k| k.key()).collect();
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(before, keys.len(), "two constraint types share a key");

    for kind in ConstraintType::all() {
        let key = kind.key();
        assert!(
            key.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{key} is not kebab-case"
        );
        assert!(!key.starts_with('-') && !key.ends_with('-'), "{key}");
    }
}

#[test]
fn a_key_round_trips_through_from_key() {
    for kind in ConstraintType::all() {
        assert_eq!(ConstraintType::from_key(kind.key()), Some(*kind));
    }
}

#[test]
fn the_serialised_name_matches_the_key() {
    // Otherwise a violation read back from JSON would carry a different identifier from the
    // one a `constraint_instance` row stores, and the two would drift.
    for kind in ConstraintType::all() {
        let json = serde_json::to_string(kind).expect("a type serialises");
        assert_eq!(json, format!("\"{}\"", kind.key()));
        let back: ConstraintType = serde_json::from_str(&json).expect("and reads back");
        assert_eq!(back, *kind);
    }
}

#[test]
fn an_unimplemented_key_is_unknown_rather_than_guessed_at() {
    // `CONSTRAINTS.md` specifies about sixty types and eight are implemented. A stored row
    // naming one of the other fifty-two must be reported, not approximated.
    for key in [
        "activity-fixed",
        "room-capacity",
        "resource-clash",
        "teacher-max-per-day",
        "spread-across-days",
        "",
        "Teacher-Clash",
    ] {
        assert_eq!(
            ConstraintType::from_key(key),
            None,
            "{key:?} must not resolve to an implemented type"
        );
    }
}

// =======================================================================================
// The levels are the accepted split
// =======================================================================================

#[test]
fn seven_types_are_hard_and_activity_scheduled_is_medium() {
    // `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's split, asserted rather than assumed. The
    // original M5 brief called `activity-scheduled` hard; the accepted record makes it MEDIUM,
    // and this is the test that stops it drifting back.
    let hard: Vec<&str> = ConstraintType::all()
        .iter()
        .filter(|k| k.default_level() == ConstraintLevel::Hard)
        .map(|k| k.key())
        .collect();
    assert_eq!(hard.len(), 7, "{hard:?}");
    assert_eq!(
        ConstraintType::ActivityScheduled.default_level(),
        ConstraintLevel::Medium,
        "an unplaced activity must not make every partial timetable infeasible"
    );
    assert!(
        ConstraintType::all()
            .iter()
            .all(|k| k.default_level() != ConstraintLevel::Soft),
        "nothing in the MVP is soft"
    );
}

#[test]
fn only_hard_blocks_feasibility() {
    assert!(ConstraintLevel::Hard.blocks_feasibility());
    assert!(!ConstraintLevel::Medium.blocks_feasibility());
    assert!(!ConstraintLevel::Soft.blocks_feasibility());
}

#[test]
fn levels_order_hard_first() {
    // The order a report is printed in, and the order a score compares.
    let mut levels = vec![
        ConstraintLevel::Soft,
        ConstraintLevel::Hard,
        ConstraintLevel::Medium,
    ];
    levels.sort_unstable();
    assert_eq!(
        levels,
        vec![
            ConstraintLevel::Hard,
            ConstraintLevel::Medium,
            ConstraintLevel::Soft
        ]
    );
}

#[test]
fn a_stored_level_round_trips_and_an_unknown_one_does_not_resolve() {
    for level in [
        ConstraintLevel::Hard,
        ConstraintLevel::Medium,
        ConstraintLevel::Soft,
    ] {
        assert_eq!(ConstraintLevel::from_stored(level.stored()), Some(level));
    }
    assert_eq!(ConstraintLevel::from_stored("hard"), None);
    assert_eq!(ConstraintLevel::from_stored("CRITICAL"), None);
}

#[test]
fn a_penalty_lands_only_in_its_own_level() {
    assert_eq!(ConstraintLevel::Hard.penalty(3), Score::new(3, 0, 0));
    assert_eq!(ConstraintLevel::Medium.penalty(3), Score::new(0, 3, 0));
    assert_eq!(ConstraintLevel::Soft.penalty(3), Score::new(0, 0, 3));
}

// =======================================================================================
// Mandatory, and what that forbids
// =======================================================================================

#[test]
fn every_implemented_type_is_mandatory_and_unnarrowable() {
    // The answer to the brief's §8: the MVP set is mandatory in the registry, not stored as
    // rows, so there is no row whose deletion could disable a safety rule.
    for kind in ConstraintType::all() {
        assert!(kind.is_mandatory(), "{} must be mandatory", kind.key());
        assert!(
            !kind.supports_narrowing(),
            "{} is a property of the whole timetable",
            kind.key()
        );
    }
}

#[test]
fn only_duration_contiguous_reads_options() {
    let readers: Vec<&str> = ConstraintType::all()
        .iter()
        .filter(|k| k.reads_options())
        .map(|k| k.key())
        .collect();
    assert_eq!(readers, vec!["duration-contiguous"]);
}

#[test]
fn a_mandatory_constraint_compiles_with_no_stored_row() {
    let compiled = CompiledConstraint::mandatory(ConstraintType::TeacherClash);
    assert_eq!(compiled.id, None, "an unconfigured project has no rows");
    assert_eq!(compiled.level, ConstraintLevel::Hard);
    assert_eq!(compiled.weight, 1);
    assert!(!compiled.scope.is_narrowed());
    assert_eq!(compiled.options, ConstraintOptions::default());
    assert!(
        !compiled.options.allow_break_span,
        "the stricter reading is the default — ADR 0033 §5"
    );
}

#[test]
fn a_term_scoped_constraint_applies_to_that_term_and_the_whole_year_one_to_all() {
    let term = Uuid::from_u128(7);
    let other = Uuid::from_u128(8);

    let mut scoped = CompiledConstraint::mandatory(ConstraintType::RoomClash);
    scoped.scope.term = Some(term);
    assert!(scoped.applies_to_term(Some(term)));
    assert!(!scoped.applies_to_term(Some(other)));
    assert!(!scoped.applies_to_term(None));

    let all_year = CompiledConstraint::mandatory(ConstraintType::RoomClash);
    assert!(all_year.applies_to_term(Some(term)));
    assert!(all_year.applies_to_term(None));
}

#[test]
fn a_narrowed_scope_is_recognised_as_narrowed() {
    let mut scope = ConstraintScope::everything();
    assert!(!scope.is_narrowed());
    scope.applies_to.push(ScopeRef {
        kind: EntityKind::Teacher,
        id: Uuid::from_u128(2),
    });
    assert!(scope.is_narrowed());
}

#[test]
fn every_constraint_error_says_what_was_wrong_and_names_its_subject() {
    // The same discipline M3's wording tests found four real defects with: an error that does
    // not name its own subject cannot be acted on.
    let errors = [
        ConstraintError::UnknownType {
            key: "room-capacity".to_owned(),
        },
        ConstraintError::UnknownLevel {
            level: "CRITICAL".to_owned(),
        },
        ConstraintError::MandatoryWeakened {
            key: "teacher-clash",
            shipped: "HARD",
            stored: "SOFT".to_owned(),
        },
        ConstraintError::ScopeNotSupported { key: "room-clash" },
        ConstraintError::InvalidParams {
            key: "duration-contiguous",
            reason: "allowBreakSpan must be a boolean".to_owned(),
        },
    ];
    for error in &errors {
        let message = error.to_string();
        assert!(message.len() > 20, "{message}");
        assert!(
            !message.starts_with("error"),
            "a message is a sentence, not a label: {message}"
        );
    }
    assert!(
        errors[2].to_string().contains("teacher-clash"),
        "{}",
        errors[2]
    );
}

// =======================================================================================
// Violations and the sink
// =======================================================================================

#[test]
fn a_violation_scores_at_its_own_level() {
    let clash = violation(ConstraintType::TeacherClash, &[0, 1]);
    assert_eq!(clash.score(), Score::new(1, 0, 0));
    assert!(!clash.score().is_feasible());

    let unplaced = violation(ConstraintType::ActivityScheduled, &[4]);
    assert_eq!(unplaced.score(), Score::new(0, 1, 0));
    assert!(
        unplaced.score().is_feasible(),
        "an unplaced activity does not make a timetable infeasible — it makes it incomplete"
    );
}

#[test]
fn a_weighted_violation_contributes_its_weight() {
    let mut heavy = violation(ConstraintType::RoomClash, &[2, 3]);
    heavy.penalty = 5;
    assert_eq!(heavy.score(), Score::new(5, 0, 0));
}

#[test]
fn a_sink_keeps_what_it_was_given_in_the_order_it_was_given() {
    let mut sink = ViolationSink::new();
    assert!(sink.is_empty());
    sink.push(violation(ConstraintType::TeacherClash, &[0, 1]));
    sink.push(violation(ConstraintType::ActivityScheduled, &[9]));

    assert_eq!(sink.len(), 2);
    assert_eq!(sink.count_at(ConstraintLevel::Hard), 1);
    assert_eq!(sink.count_at(ConstraintLevel::Medium), 1);
    assert_eq!(sink.count_at(ConstraintLevel::Soft), 0);
    assert_eq!(sink.score(), Score::new(1, 1, 0));
    assert_eq!(
        sink.violations()[0].constraint_type,
        ConstraintType::TeacherClash
    );
}

#[test]
fn deduplication_keeps_the_first_of_each_and_preserves_order() {
    let mut sink = ViolationSink::new();
    sink.push(violation(ConstraintType::TeacherClash, &[0, 1]));
    sink.push(violation(ConstraintType::RoomClash, &[0, 1]));
    sink.push(violation(ConstraintType::TeacherClash, &[0, 1]));

    sink.deduplicate();
    let kinds: Vec<ConstraintType> = sink
        .violations()
        .iter()
        .map(|v| v.constraint_type)
        .collect();
    assert_eq!(
        kinds,
        vec![ConstraintType::TeacherClash, ConstraintType::RoomClash],
        "the same rule about the same pair is one violation; a different rule is not"
    );
}

#[test]
fn a_key_is_the_rule_the_occurrences_and_the_subject() {
    let a = violation(ConstraintType::TeacherClash, &[0, 1]);
    let b = violation(ConstraintType::TeacherClash, &[0, 1]);
    assert_eq!(a.key(), b.key());

    let other_pair = violation(ConstraintType::TeacherClash, &[0, 2]);
    assert_ne!(a.key(), other_pair.key());

    let mut other_teacher = violation(ConstraintType::TeacherClash, &[0, 1]);
    other_teacher.subject = EntityRef::new(EntityKind::Teacher, Uuid::from_u128(99), "B. Benson");
    assert_ne!(
        a.key(),
        other_teacher.key(),
        "two teachers clashing over the same pair are two problems to fix"
    );
}

#[test]
fn an_explanation_blocks_exactly_when_it_is_hard() {
    let hard = Explanation {
        constraint_type: ConstraintType::TeacherClash,
        level: ConstraintLevel::Hard,
        message: String::from("A. Abara already teaches 9B Science then."),
        entities: vec![subject()],
        conflicting_activities: vec![Uuid::from_u128(3)],
    };
    assert!(hard.blocks());

    let medium = Explanation {
        level: ConstraintLevel::Medium,
        ..hard.clone()
    };
    assert!(!medium.blocks());
}

// =======================================================================================
// The affected set
// =======================================================================================

#[test]
fn an_affected_set_is_sorted_deduplicated_and_answers_membership() {
    let set = AffectedSet::of(vec![
        ActivityIx::new(5),
        ActivityIx::new(1),
        ActivityIx::new(5),
    ]);
    assert_eq!(set.activities(), [ActivityIx::new(1), ActivityIx::new(5)]);
    assert!(set.contains(ActivityIx::new(1)));
    assert!(!set.contains(ActivityIx::new(2)));
    assert!(!AffectedSet::is_everything(&set));
}

#[test]
fn everything_contains_everything_and_nothing_contains_nothing() {
    let all = AffectedSet::everything();
    assert!(AffectedSet::is_everything(&all));
    assert!(all.contains(ActivityIx::new(0)));
    assert!(all.contains(ActivityIx::new(9_999)));

    let none = AffectedSet::nothing();
    assert!(!none.contains(ActivityIx::new(0)));
    assert_eq!(none.activities(), []);
}
