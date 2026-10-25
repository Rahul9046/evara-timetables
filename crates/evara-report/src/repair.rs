//! What a repair would change, as a projection.
//!
//! # A projection, not a judgement
//!
//! This module compares two [`Assignment`]s against one [`Problem`] and labels each occurrence.
//! It resolves nothing, judges no legality, and decides nothing about whether a cause is
//! impossible — the same discipline the grid projection is held to. Every sentence it carries
//! came from the evaluator that produced the violation, through
//! [`explain_all`](crate::explain_all), because a report that composed its own wording would be
//! a second implementation of every rule's reasoning (ADR 0043).
//!
//! # The five categories are exhaustive and disjoint
//!
//! Every occurrence in the problem lands in exactly one of them, which is asserted rather than
//! assumed — see [`RepairDiff::is_well_formed`].
//!
//! | Category | Placed before | Placed after |
//! | --- | --- | --- |
//! | [`ChangeKind::Unchanged`] | yes, same placement | yes |
//! | [`ChangeKind::Moved`] | yes, different placement | yes |
//! | [`ChangeKind::NewlyScheduled`] | no | yes |
//! | [`ChangeKind::Unscheduled`] | yes | no |
//! | [`ChangeKind::StillUnplaced`] | no | no |
//!
//! # Two kinds of still-unplaced, and why they are distinguished
//!
//! An occurrence that is unplaced in both arrangements is either one the repair **tried** and
//! could not place, or one the scope never let it touch. The remedies differ — widen the scope
//! versus look at the clash — so [`OccurrenceChange::in_scope`] carries which, and the screen
//! says so. Collapsing them would tell a user to fix a timetable when the answer is to choose a
//! different scope.
//!
//! # The totals are the whole arrangement's
//!
//! [`RepairDiff::score_after`] and [`RepairDiff::findings_after`] describe the **complete**
//! repaired timetable, pre-existing out-of-scope violations included. The attribution buckets
//! sit beside them, never instead of them: a preview that showed only in-scope violations would
//! be describing a timetable the user is not about to save.

use std::collections::BTreeMap;

use evara_constraints::EvaluationReport;
use evara_core::scheduling::{
    ActivityIx, Assignment, LabelSet, MovableSet, Placement, Problem, Score, UnplacedCause,
    Violation,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::findings::{FindingView, explain_all};

#[cfg(test)]
mod tests;

/// What happened to one occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ChangeKind {
    /// Placed in both, in the same place.
    Unchanged,
    /// Placed in both, in different places.
    Moved,
    /// Unplaced before, placed after.
    NewlyScheduled,
    /// Placed before, unplaced after.
    Unscheduled,
    /// Unplaced in both.
    StillUnplaced,
}

impl ChangeKind {
    /// The stable key, for a test that pins the vocabulary.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Moved => "moved",
            Self::NewlyScheduled => "newly-scheduled",
            Self::Unscheduled => "unscheduled",
            Self::StillUnplaced => "still-unplaced",
        }
    }

    /// Whether this is a change at all, for a count the screen leads with.
    #[must_use]
    pub const fn is_a_change(self) -> bool {
        matches!(self, Self::Moved | Self::NewlyScheduled | Self::Unscheduled)
    }
}

/// Where an occurrence sits, named rather than indexed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlacementView {
    /// The slot's identifier, for a grid that wants to highlight it.
    pub timeslot_id: Uuid,
    /// The cycle day, as a label.
    pub day: String,
    /// The period, as a label.
    pub period: String,
    /// The room's name, where one is booked.
    pub room: Option<String>,
    /// The room's identifier, where one is booked.
    pub room_id: Option<Uuid>,
}

/// What changed for one occurrence, and what it is called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OccurrenceChange {
    /// The occurrence's stored identifier.
    pub activity_id: Uuid,
    /// Which of the five things happened.
    pub kind: ChangeKind,
    /// Whether the repair was allowed to move it.
    ///
    /// `false` for a frozen occurrence. For [`ChangeKind::StillUnplaced`] this is the whole
    /// difference between *"the repair could not place this"* and *"the scope you chose does
    /// not include it"*.
    pub in_scope: bool,
    /// Whether a lock holds it where it is.
    pub is_locked: bool,
    /// What a human calls the lesson — subject, course, section, year level.
    pub lesson: Option<String>,
    /// How many periods it occupies.
    pub periods: u16,
    /// Where it was, for everything but [`ChangeKind::NewlyScheduled`] and `StillUnplaced`.
    pub before: Option<PlacementView>,
    /// Where it is now, for everything but `Unscheduled` and `StillUnplaced`.
    pub after: Option<PlacementView>,
    /// Which of the two planning variables changed, for a [`ChangeKind::Moved`] occurrence.
    ///
    /// `None` for every other kind, which makes "the slot changed" unrepresentable for an
    /// occurrence that did not move — a flag pair that could say so would be a state no
    /// screen should have to defend against.
    pub moved: Option<MovedParts>,
    /// Why it is not placed, where it is not.
    ///
    /// The engine's own cause. Never re-derived here, and never turned into a claim about
    /// whether a timetable exists — [`UnplacedCause::is_impossible`] is the one place that line
    /// is drawn and this module does not draw it.
    pub cause: Option<String>,
}

/// A score, at all three levels, flattened for the interface.
///
/// `Score` itself does not cross the IPC boundary -- M9A's `ScoreView` set the convention and
/// the reason is the same: the interface renders three integers and must not be handed a type
/// whose ordering semantics it would have to reimplement to compare two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScoreLine {
    /// Hard penalties. Negative-going, so zero is the best reachable.
    pub hard: i64,
    /// Medium penalties.
    pub medium: i64,
    /// Soft penalties.
    pub soft: i64,
}

impl ScoreLine {
    /// Flattens one score.
    #[must_use]
    pub const fn of(score: Score) -> Self {
        Self {
            hard: score.hard,
            medium: score.medium,
            soft: score.soft,
        }
    }

    /// Whether this score is better than `other`, lexicographically.
    ///
    /// Computed in Rust so the interface never has to know the comparison is lexicographic
    /// rather than a sum.
    #[must_use]
    pub fn is_better_than(&self, other: Self) -> bool {
        (self.hard, self.medium, self.soft) > (other.hard, other.medium, other.soft)
    }
}

/// Which planning variables a move changed.
///
/// At least one is always `true`: a "move" that changed neither is an unchanged placement, and
/// [`diff`] classifies it as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MovedParts {
    /// The occurrence is in a different slot.
    pub slot: bool,
    /// The occurrence is in a different room.
    pub room: bool,
}

/// How the repaired timetable's hard violations divide.
///
/// Set membership over two reference reports, never a re-judgement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Attribution {
    /// Present before and after, naming no movable occurrence.
    ///
    /// Expected, not the repair's doing, and **not** a reason to withhold applying it.
    pub pre_existing_frozen: Vec<FindingView>,
    /// Present after only, naming a movable occurrence. Always empty for a valid repair.
    pub new_involving_movable: Vec<FindingView>,
    /// Present before only. What the repair fixed, and the number a user looks for first.
    pub eliminated: Vec<FindingView>,
}

/// The whole before-and-after, ready to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RepairDiff {
    /// One entry per occurrence in the problem, in activity-index order.
    pub changes: Vec<OccurrenceChange>,
    /// How many of each kind, so the screen can lead with counts.
    pub counts: ChangeCounts,
    /// How many occurrences the repair could move.
    pub movable_count: usize,
    /// How many were frozen.
    pub frozen_count: usize,
    /// The baseline's score, at all three levels.
    pub score_before: ScoreLine,
    /// The repaired arrangement's score. The **whole** arrangement's.
    pub score_after: ScoreLine,
    /// Whether the repaired arrangement scores better than the baseline.
    ///
    /// Decided here because the comparison is lexicographic over three levels, not a sum --
    /// an interface that added them up would rank a timetable with one hard clash above one
    /// with several soft ones.
    pub improved: bool,
    /// Every finding in the baseline, with each rule's own sentence.
    pub findings_before: Vec<FindingView>,
    /// Every finding in the repaired arrangement. The **whole** arrangement's.
    pub findings_after: Vec<FindingView>,
    /// How the hard violations divide.
    pub attribution: Attribution,
}

/// How many occurrences fell into each category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChangeCounts {
    /// Placed in both, same place.
    pub unchanged: usize,
    /// Placed in both, different place.
    pub moved: usize,
    /// Newly placed.
    pub newly_scheduled: usize,
    /// No longer placed.
    pub unscheduled: usize,
    /// Unplaced in both.
    pub still_unplaced: usize,
}

impl ChangeCounts {
    /// How many occurrences the repair actually changed.
    #[must_use]
    pub const fn changed(&self) -> usize {
        self.moved + self.newly_scheduled + self.unscheduled
    }

    /// Every occurrence accounted for.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.unchanged + self.moved + self.newly_scheduled + self.unscheduled + self.still_unplaced
    }
}

impl RepairDiff {
    /// Whether this repair changed anything at all.
    ///
    /// A `true` here is reported as *"this repair attempt made no changes"* and **never** as
    /// *"nothing could be improved"*: a greedy construction that changed nothing has discovered
    /// that its own walk found the baseline legal, not that no better arrangement exists. The
    /// same discipline [`UnplacedCause::is_impossible`] enforces per occurrence, applied to the
    /// whole run.
    #[must_use]
    pub const fn made_no_changes(&self) -> bool {
        self.counts.changed() == 0
    }

    /// Every category is populated consistently, and the counts reconcile.
    ///
    /// Asserted by the tests over every fixture, in the shape `GridPanel::is_well_formed` set:
    /// a projection that could silently drop an occurrence is one a screen would render as a
    /// lost lesson.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        if self.counts.total() != self.changes.len() {
            return false;
        }
        if self.movable_count + self.frozen_count != self.changes.len() {
            return false;
        }
        self.changes.iter().all(|change| match change.kind {
            ChangeKind::Unchanged => {
                change.before.is_some() && change.after == change.before && change.moved.is_none()
            }
            ChangeKind::Moved => {
                change.before.is_some()
                    && change.after.is_some()
                    && change.moved.is_some_and(|parts| parts.slot || parts.room)
            }
            ChangeKind::NewlyScheduled => {
                change.before.is_none() && change.after.is_some() && change.moved.is_none()
            }
            ChangeKind::Unscheduled => {
                change.before.is_some() && change.after.is_none() && change.moved.is_none()
            }
            ChangeKind::StillUnplaced => {
                change.before.is_none() && change.after.is_none() && change.moved.is_none()
            }
        })
    }
}

/// Everything the diff needs. Nothing is read or resolved inside.
#[derive(Debug, Clone, Copy)]
pub struct RepairDiffInput<'a> {
    /// The compiled problem both arrangements are over.
    pub problem: &'a Problem,
    /// The arrangement as stored.
    pub baseline: &'a Assignment,
    /// The arrangement the repair reached.
    pub repaired: &'a Assignment,
    /// What the repair was allowed to move.
    pub movable: &'a MovableSet,
    /// The names a grid needs, populated by `evara-db` on request (ADR 0044).
    pub labels: &'a LabelSet,
    /// The reference evaluation of the baseline.
    pub before: &'a EvaluationReport,
    /// The reference evaluation of the repaired arrangement.
    pub after: &'a EvaluationReport,
    /// Why each occurrence the repair attempted is unplaced, where it is.
    pub causes: &'a BTreeMap<Uuid, UnplacedCause>,
    /// Which occurrences a lock holds.
    pub locked: &'a BTreeMap<Uuid, bool>,
}

/// Compares two arrangements of one problem.
#[must_use]
pub fn diff(input: &RepairDiffInput<'_>) -> RepairDiff {
    let mut changes = Vec::with_capacity(input.problem.activities.len());
    let mut counts = ChangeCounts::default();

    for (ix, fact) in input.problem.activities.iter().enumerate() {
        let activity = ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX));
        let before = input.baseline.placement(activity);
        let after = input.repaired.placement(activity);

        let kind = match (before, after) {
            (Some(was), Some(now)) if was == now => ChangeKind::Unchanged,
            (Some(_), Some(_)) => ChangeKind::Moved,
            (None, Some(_)) => ChangeKind::NewlyScheduled,
            (Some(_), None) => ChangeKind::Unscheduled,
            (None, None) => ChangeKind::StillUnplaced,
        };
        match kind {
            ChangeKind::Unchanged => counts.unchanged += 1,
            ChangeKind::Moved => counts.moved += 1,
            ChangeKind::NewlyScheduled => counts.newly_scheduled += 1,
            ChangeKind::Unscheduled => counts.unscheduled += 1,
            ChangeKind::StillUnplaced => counts.still_unplaced += 1,
        }

        // `SectionLabel::short` is already composed by `SectionLabel::compose_short`, which
        // is the one implementation -- recomposing it here would be the second.
        let lesson = input
            .labels
            .section(section_id(input.problem, ix))
            .map(|label| label.short.clone());

        changes.push(OccurrenceChange {
            activity_id: fact.id,
            kind,
            in_scope: input.movable.contains(activity),
            is_locked: input.locked.get(&fact.id).copied().unwrap_or(false),
            lesson,
            periods: u16::try_from(fact.duration.as_usize()).unwrap_or(u16::MAX),
            before: before.map(|at| view_of(input, at)),
            after: after.map(|at| view_of(input, at)),
            moved: match (kind, before, after) {
                (ChangeKind::Moved, Some(was), Some(now)) => Some(MovedParts {
                    slot: was.timeslot != now.timeslot,
                    room: was.room != now.room,
                }),
                _ => None,
            },
            cause: input.causes.get(&fact.id).map(|c| c.key().to_owned()),
        });
    }

    RepairDiff {
        changes,
        counts,
        movable_count: input.movable.len(),
        frozen_count: input.movable.frozen_count(),
        score_before: ScoreLine::of(input.before.score),
        score_after: ScoreLine::of(input.after.score),
        improved: ScoreLine::of(input.after.score)
            .is_better_than(ScoreLine::of(input.before.score)),
        findings_before: explain_all(input.problem, &input.before.violations),
        findings_after: explain_all(input.problem, &input.after.violations),
        attribution: attribute(input),
    }
}

/// The three buckets, as set operations on the two reports.
fn attribute(input: &RepairDiffInput<'_>) -> Attribution {
    let hard_before: Vec<&Violation> = hard(input.before).collect();
    let hard_after: Vec<&Violation> = hard(input.after).collect();
    let keys_before: Vec<_> = hard_before.iter().map(|v| v.key()).collect();
    let keys_after: Vec<_> = hard_after.iter().map(|v| v.key()).collect();

    let pre_existing: Vec<Violation> = hard_after
        .iter()
        .filter(|v| !touches_movable(v, input.movable))
        .filter(|v| keys_before.contains(&v.key()))
        .map(|v| (*v).clone())
        .collect();
    let introduced: Vec<Violation> = hard_after
        .iter()
        .filter(|v| touches_movable(v, input.movable))
        .filter(|v| !keys_before.contains(&v.key()))
        .map(|v| (*v).clone())
        .collect();
    let gone: Vec<Violation> = hard_before
        .iter()
        .filter(|v| !keys_after.contains(&v.key()))
        .map(|v| (*v).clone())
        .collect();

    Attribution {
        pre_existing_frozen: explain_all(input.problem, &pre_existing),
        new_involving_movable: explain_all(input.problem, &introduced),
        eliminated: explain_all(input.problem, &gone),
    }
}

fn hard(report: &EvaluationReport) -> impl Iterator<Item = &Violation> + '_ {
    report
        .violations
        .iter()
        .filter(|v| v.level == evara_core::scheduling::ConstraintLevel::Hard)
}

/// Whether a violation names an occurrence the repair was allowed to move.
fn touches_movable(found: &Violation, movable: &MovableSet) -> bool {
    found.activities.iter().any(|at| movable.contains(*at))
}

/// One placement, with the names a reader needs.
fn view_of(input: &RepairDiffInput<'_>, at: Placement) -> PlacementView {
    let slot = input.problem.timeslots.get(at.timeslot.get());
    let day = slot
        .and_then(|s| input.problem.context.cycle_days.get(s.cycle_day.get()))
        .map_or_else(String::new, |d| d.label.clone());
    let period = slot
        .and_then(|s| input.problem.context.periods.get(s.period.get()))
        .map_or_else(String::new, |p| p.label.clone());
    let room_id = at
        .room
        .and_then(|room| input.problem.rooms.get(room.get()))
        .map(|room| room.id);

    PlacementView {
        timeslot_id: slot.map(|s| s.id).unwrap_or_default(),
        day,
        period,
        room: room_id
            .and_then(|id| input.labels.room(id))
            .map(|r| r.name.clone()),
        room_id,
    }
}

/// The section identifier of one occurrence.
fn section_id(problem: &Problem, activity: usize) -> Uuid {
    problem
        .activities
        .get(activity)
        .and_then(|fact| problem.sections.get(fact.section.get()))
        .map(|section| section.id)
        .unwrap_or_default()
}
