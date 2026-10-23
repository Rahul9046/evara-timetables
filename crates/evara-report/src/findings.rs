//! Turning violations into sentences, without writing any of them here.
//!
//! # Why this crate may see `evara-constraints`
//!
//! `explain()` belongs to the rule that produced the violation, so *"that wording lives with
//! the check that produced it"* ([CONSTRAINT-ENGINE.md](../../../../docs/CONSTRAINT-ENGINE.md)
//! §6). A report that composed its own sentence would be a second implementation of every
//! rule's reasoning — the drift `CLAUDE.md` §2's most important rule exists to stop — and it
//! would go stale the first time a rule's check changed and its wording did not.
//!
//! So this module **calls** [`evara_constraints::evaluator_for`] and formats nothing.
//! [ADR 0043](../../../../docs/adr/0043-a-report-renders-a-rules-own-explanation.md) records
//! the dependency edge that buys it.
//!
//! # Two fidelities, and neither claims the other's
//!
//! | Tier | Needs | Carries |
//! | --- | --- | --- |
//! | [`FindingView::from_stored`] | nothing but the stored row | the level, the penalty, the stored `detail`, the entities **by identifier**, the day and the `'HH:MM'` span |
//! | [`explain_all`] | a `Problem` and the live violations | all of the above, plus the rule's own sentence and the occurrences it names |
//!
//! The stored tier is what [TIMETABLE-PERSISTENCE.md](../../../../docs/TIMETABLE-PERSISTENCE.md)
//! §7 bought: *"they let a view render findings without loading a `Problem` at all"*. It is
//! the fallback for a timetable whose problem **cannot** be compiled, and in that state a
//! screen shows the findings and says the grid could not be built. It never renders a partial
//! grid, because deriving a span without a problem would be the second span implementation.

use evara_core::scheduling::{
    ActivityIx, ConstraintLevel, ConstraintType, EntityRef, Problem, Violation,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where a finding's wording came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum FindingSource {
    /// The rule's own [`explain()`], over a compiled problem.
    Explained,
    /// A `violation` row, read without compiling a problem.
    ///
    /// Carries the `detail` the scoring stored and **no** rendered sentence: a wording
    /// improvement would otherwise leave old rows reading the old way.
    Stored,
}

/// One finding, at whichever fidelity was available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FindingView {
    /// The registry's stable `type_key`.
    ///
    /// A plain string rather than an enum: the registry is open, and a project written by a
    /// newer Evara may hold a key this build does not implement. Such a key is **reported as
    /// unknown**, never ignored and never guessed at.
    pub type_key: String,
    /// Whether this build implements the rule that produced it.
    pub known_type: bool,
    /// `HARD`, `MEDIUM` or `SOFT`, as stored or as evaluated.
    pub level: String,
    /// Negative-going, as every penalty in this workspace is.
    pub penalty: i64,
    /// The rule's own sentence, where a problem was available to produce one.
    pub message: Option<String>,
    /// The specific fact the rule recorded.
    pub detail: String,
    /// The cycle day it is about, where it has a when.
    pub cycle_day_id: Option<Uuid>,
    /// `'HH:MM'`.
    pub starts_at: Option<String>,
    /// `'HH:MM'`.
    pub ends_at: Option<String>,
    /// The occurrences it names, by stored identifier.
    ///
    /// Stored identifiers rather than dense indexes: an index has no meaning outside the
    /// problem that assigned it, and this type crosses the IPC boundary.
    pub activity_ids: Vec<Uuid>,
    /// The entities it names, as `(kind, id)` pairs a screen can link.
    pub entities: Vec<FindingEntity>,
    /// Which tier produced it.
    pub source: FindingSource,
}

/// One entity a finding names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FindingEntity {
    /// `activity`, `subject` or `related` — the part this entity plays.
    pub role: String,
    /// What kind of thing it is.
    pub kind: String,
    /// Which one.
    pub id: String,
}

/// How many findings there are at each level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FindingCounts {
    /// Findings that make the arrangement unusable.
    pub hard: usize,
    /// Findings that make it worse without making it unusable.
    pub medium: usize,
    /// Preferences.
    pub soft: usize,
    /// Findings whose rule this build does not implement.
    pub unknown: usize,
}

impl FindingCounts {
    /// Counts a list of findings.
    #[must_use]
    pub fn of(findings: &[FindingView]) -> Self {
        let mut counts = Self::default();
        for finding in findings {
            if !finding.known_type {
                counts.unknown += 1;
            }
            match finding.level.as_str() {
                "HARD" => counts.hard += 1,
                "MEDIUM" => counts.medium += 1,
                "SOFT" => counts.soft += 1,
                _ => {}
            }
        }
        counts
    }

    /// Whether any hard rule is broken.
    ///
    /// Deliberately not called `is_valid` or `is_feasible`: a usable **partial** timetable is
    /// still partial, and `CONSTRAINT-ENGINE.md` §6 keeps those questions apart.
    #[must_use]
    pub const fn is_usable(self) -> bool {
        self.hard == 0
    }
}

/// The stored spelling of a level.
const fn level_key(level: ConstraintLevel) -> &'static str {
    match level {
        ConstraintLevel::Hard => "HARD",
        ConstraintLevel::Medium => "MEDIUM",
        ConstraintLevel::Soft => "SOFT",
    }
}

/// An [`EntityKind`](evara_core::scheduling::EntityKind) as the string a screen sees.
///
/// Read through its serde attribute rather than restated in a second `match`, which is how
/// `evara-db` writes `violation_ref.entity_kind` — so the stored vocabulary and this one are
/// the same vocabulary and cannot drift. `CLAUDE.md`: the mapping is applied *"in exactly one
/// place with serde attributes"*, and this is that place being used rather than bypassed.
fn entity_kind_key(kind: evara_core::scheduling::EntityKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from("unknown"))
}

/// The `(kind, id)` of one entity reference, as a screen sees it.
fn entity_of(role: &str, reference: &EntityRef) -> FindingEntity {
    FindingEntity {
        role: role.to_owned(),
        kind: entity_kind_key(reference.kind),
        id: reference.id.to_string(),
    }
}

impl FindingView {
    /// A finding explained by the rule that produced it.
    ///
    /// The sentence comes from [`evara_constraints::evaluator_for`] and is not composed here.
    #[must_use]
    pub fn explained(problem: &Problem, violation: &Violation) -> Self {
        let explanation =
            evara_constraints::evaluator_for(violation.constraint_type).explain(problem, violation);
        Self {
            type_key: violation.constraint_type.key().to_owned(),
            known_type: true,
            level: level_key(violation.level).to_owned(),
            penalty: violation.penalty,
            message: Some(explanation.message),
            detail: violation.detail.clone(),
            cycle_day_id: violation
                .cycle_day
                .and_then(|ix| problem.context.cycle_days.get(ix.get()))
                .map(|day| day.id),
            starts_at: violation.span.map(|span| span.start.to_string()),
            ends_at: violation.span.map(|span| span.end.to_string()),
            activity_ids: violation
                .activities
                .iter()
                .filter_map(|ix: &ActivityIx| problem.activities.get(ix.get()))
                .map(|fact| fact.id)
                .collect(),
            entities: std::iter::once(entity_of("subject", &violation.subject))
                .chain(
                    violation
                        .related
                        .iter()
                        .map(|reference| entity_of("related", reference)),
                )
                .collect(),
            source: FindingSource::Explained,
        }
    }

    /// A finding read from a stored row, with no problem compiled.
    ///
    /// `message` is `None` and that is the point: the engine's sentence is deliberately not
    /// stored, because *"a wording improvement would leave old rows reading the old way"*
    /// (TIMETABLE-PERSISTENCE §7). The stored `detail` is the fact, and a screen renders it
    /// as one.
    ///
    /// An unrecognised `type_key` sets [`FindingView::known_type`] to `false` rather than
    /// being dropped: a rule this build cannot evaluate is still a finding somebody recorded.
    #[must_use]
    pub fn from_stored(stored: StoredFinding<'_>) -> Self {
        let StoredFinding {
            type_key,
            level,
            penalty,
            detail,
            cycle_day_id,
            starts_at,
            ends_at,
            entities,
        } = stored;
        Self {
            type_key: type_key.to_owned(),
            known_type: ConstraintType::from_key(type_key).is_some(),
            level: level.to_owned(),
            penalty,
            message: None,
            detail: detail.to_owned(),
            cycle_day_id,
            starts_at,
            ends_at,
            activity_ids: entities
                .iter()
                .filter(|e| e.role == "activity")
                .filter_map(|e| e.id.parse().ok())
                .collect(),
            entities,
            source: FindingSource::Stored,
        }
    }
}

/// One `violation` row, as the reader hands it over.
///
/// Grouped rather than passed as eight arguments, and the grouping is the stored row itself:
/// a caller holding the parts apart could pair one violation's level with another's span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFinding<'a> {
    /// `violation.constraint_type`, as stored.
    pub type_key: &'a str,
    /// `HARD`, `MEDIUM` or `SOFT`, as stored.
    pub level: &'a str,
    /// Negative-going.
    pub penalty: i64,
    /// The specific fact the scoring recorded.
    pub detail: &'a str,
    /// The cycle position, where the violation has a when.
    pub cycle_day_id: Option<Uuid>,
    /// `'HH:MM'`.
    pub starts_at: Option<String>,
    /// `'HH:MM'`.
    pub ends_at: Option<String>,
    /// The entities the row's `violation_ref` children name.
    pub entities: Vec<FindingEntity>,
}

/// Explains every violation, in the order they were found.
///
/// The order is the evaluators' own — `ConstraintType::all()`'s, *"chosen so a reader meets
/// causes before consequences"* — and is preserved rather than re-sorted, because a sort here
/// would be a second opinion about what matters.
#[must_use]
pub fn explain_all(problem: &Problem, violations: &[Violation]) -> Vec<FindingView> {
    violations
        .iter()
        .map(|violation| FindingView::explained(problem, violation))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_finding_carries_no_sentence() {
        let finding = FindingView::from_stored(StoredFinding {
            type_key: "teacher-clash",
            level: "HARD",
            penalty: -1000,
            detail: "two lessons at once",
            cycle_day_id: None,
            starts_at: None,
            ends_at: None,
            entities: Vec::new(),
        });
        assert!(
            finding.message.is_none(),
            "the engine's sentence is not stored, so none can be rendered"
        );
        assert_eq!(finding.detail, "two lessons at once");
        assert_eq!(finding.source, FindingSource::Stored);
        assert!(finding.known_type);
    }

    #[test]
    fn an_unrecognised_stored_type_is_reported_not_dropped() {
        let finding = FindingView::from_stored(StoredFinding {
            type_key: "a-rule-from-a-newer-build",
            level: "HARD",
            penalty: -1,
            detail: "",
            cycle_day_id: None,
            starts_at: None,
            ends_at: None,
            entities: Vec::new(),
        });
        assert!(
            !finding.known_type,
            "a key this build does not implement is reported as unknown"
        );
        assert_eq!(finding.type_key, "a-rule-from-a-newer-build");
    }

    #[test]
    fn counts_separate_the_three_levels_and_the_unknown() {
        let a_finding = |type_key, level| {
            FindingView::from_stored(StoredFinding {
                type_key,
                level,
                penalty: -1,
                detail: "",
                cycle_day_id: None,
                starts_at: None,
                ends_at: None,
                entities: Vec::new(),
            })
        };
        let findings = vec![
            a_finding("teacher-clash", "HARD"),
            a_finding("activity-scheduled", "MEDIUM"),
            a_finding("from-the-future", "HARD"),
        ];
        let counts = FindingCounts::of(&findings);
        assert_eq!(counts.hard, 2);
        assert_eq!(counts.medium, 1);
        assert_eq!(counts.soft, 0);
        assert_eq!(counts.unknown, 1);
        assert!(!counts.is_usable());
    }

    #[test]
    fn an_activity_role_entity_becomes_an_activity_id() {
        let id = Uuid::from_bytes([7; 16]);
        let finding = FindingView::from_stored(StoredFinding {
            type_key: "teacher-clash",
            level: "HARD",
            penalty: -1,
            detail: "",
            cycle_day_id: None,
            starts_at: None,
            ends_at: None,
            entities: vec![FindingEntity {
                role: "activity".to_owned(),
                kind: "activity".to_owned(),
                id: id.to_string(),
            }],
        });
        assert_eq!(finding.activity_ids, vec![id]);
    }
}
