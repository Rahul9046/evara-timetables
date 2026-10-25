//! The five categories, and that they account for every occurrence.
//!
//! The assertion that matters most is [`the_five_categories_are_exhaustive_over_every_fixture`]:
//! a projection that could silently drop an occurrence is one a screen would render as a lost
//! lesson, which is the same failure `GridPanel::is_well_formed` exists to prevent.
//!
//! # No solver here, deliberately
//!
//! A diff is a pure comparison of two arrangements, so these tests build both by hand. Reaching
//! for `evara-solver` would add a dependency this crate's layer does not have (`CLAUDE.md` §2)
//! and would test the solver's choices rather than the projection's arithmetic.

use std::collections::BTreeMap;

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::fixtures::FixtureKind;
use evara_core::scheduling::{
    ActivityIx, Assignment, LabelSet, MovableSet, Placement, Problem, RoomIx, TimeslotIx,
    UnplacedCause, frozen_only,
};

use super::{ChangeKind, RepairDiff, RepairDiffInput, diff};

fn at(raw: u32) -> ActivityIx {
    ActivityIx::new(raw)
}

/// A spread arrangement: occurrence `i` goes to slot `i / rooms` in room `i % rooms`.
///
/// One occurrence per `(slot, room)` cell, so it is legal for any fixture whose occurrences fit
/// — and where they do not, it simply leaves the overflow unplaced, which is an ordinary state
/// and one of the five categories.
fn spread(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem.activities.len());
    let slots = problem.timeslots.len();
    let rooms = problem.rooms.len().max(1);
    for ix in 0..problem.activities.len() {
        let slot = ix / rooms;
        if slot >= slots {
            break;
        }
        let room = (!problem.rooms.is_empty())
            .then(|| RoomIx::new(u32::try_from(ix % rooms).unwrap_or(0)));
        assignment.place(
            at(u32::try_from(ix).expect("fits")),
            Placement {
                timeslot: TimeslotIx::new(u32::try_from(slot).expect("fits")),
                room,
            },
        );
    }
    assignment
}

/// The diff of one baseline against one repaired arrangement.
fn diff_of(
    problem: &Problem,
    baseline: &Assignment,
    repaired: &Assignment,
    movable: &MovableSet,
) -> RepairDiff {
    let rules = Registry::mandatory();
    let before = evaluate(problem, baseline, &rules);
    let after = evaluate(problem, repaired, &rules);
    let causes = BTreeMap::new();
    let locked = BTreeMap::new();
    let labels = LabelSet::default();
    diff(&RepairDiffInput {
        problem,
        baseline,
        repaired,
        movable,
        labels: &labels,
        before: &before,
        after: &after,
        causes: &causes,
        locked: &locked,
    })
}

#[test]
fn the_five_categories_are_exhaustive_over_every_fixture() {
    for kind in FixtureKind::all() {
        let found = kind.generate(2026);
        let Some(problem) = found.problems.first() else {
            continue;
        };
        if problem.activities.is_empty() {
            continue;
        }
        let baseline = spread(problem);
        let movable = MovableSet::everything(problem.activities.len()).expect("movable");

        // A "repair" that withdrew the first occurrence and left the rest: enough to populate
        // several categories at once over every fixture shape.
        let narrow = MovableSet::new(problem.activities.len(), &[at(0)]).expect("movable");
        let repaired = frozen_only(&baseline, &narrow);

        for (label, m, r) in [
            ("unchanged", &movable, &baseline),
            ("withdrawn", &narrow, &repaired),
        ] {
            let d = diff_of(problem, &baseline, r, m);
            assert!(
                d.is_well_formed(),
                "{} / {label}: the diff is not well formed",
                found.name
            );
            assert_eq!(
                d.changes.len(),
                problem.activities.len(),
                "{} / {label}: every occurrence must appear exactly once",
                found.name
            );
            assert_eq!(
                d.counts.total(),
                problem.activities.len(),
                "{} / {label}: the counts must reconcile",
                found.name
            );
            assert_eq!(
                d.movable_count + d.frozen_count,
                problem.activities.len(),
                "{} / {label}: scope accounting",
                found.name
            );
        }
    }
}

#[test]
fn an_unchanged_repair_reports_no_changes() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");

    let d = diff_of(problem, &baseline, &baseline, &movable);

    assert!(d.made_no_changes());
    assert_eq!(d.counts.changed(), 0);
    assert_eq!(d.counts.moved, 0);
    assert!(
        d.changes
            .iter()
            .all(|c| matches!(c.kind, ChangeKind::Unchanged | ChangeKind::StillUnplaced)),
        "nothing moved anywhere"
    );
    assert!(
        d.changes.iter().all(|c| c.moved.is_none()),
        "so nothing carries moved parts"
    );
}

#[test]
fn a_moved_occurrence_says_which_variable_changed() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    assert!(
        problem.rooms.len() >= 2,
        "need two rooms for a room-only move"
    );
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");

    // A room-only move: the slot is identical, the room is not.
    let mut repaired = baseline.clone();
    let was = baseline.placement(at(0)).expect("placed");
    let other = problem
        .rooms
        .iter()
        .enumerate()
        .map(|(ix, _)| RoomIx::new(u32::try_from(ix).expect("fits")))
        .find(|room| Some(*room) != was.room)
        .expect("a second room");
    repaired.place(
        at(0),
        Placement {
            timeslot: was.timeslot,
            room: Some(other),
        },
    );

    let d = diff_of(problem, &baseline, &repaired, &movable);
    let change = d
        .changes
        .iter()
        .find(|c| c.activity_id == problem.activities[0].id)
        .expect("the moved occurrence");

    assert_eq!(change.kind, ChangeKind::Moved);
    let parts = change.moved.expect("a move carries its parts");
    assert!(!parts.slot, "the slot did not change");
    assert!(parts.room, "the room did");
    assert_eq!(d.counts.moved, 1);
    assert!(d.is_well_formed());
}

#[test]
fn a_slot_only_move_is_reported_as_one() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");

    let mut repaired = baseline.clone();
    let was = baseline.placement(at(0)).expect("placed");
    let last = TimeslotIx::new(u32::try_from(problem.timeslots.len() - 1).expect("fits"));
    assert_ne!(was.timeslot, last, "need a different slot");
    repaired.place(
        at(0),
        Placement {
            timeslot: last,
            room: was.room,
        },
    );

    let d = diff_of(problem, &baseline, &repaired, &movable);
    let parts = d
        .changes
        .iter()
        .find(|c| c.activity_id == problem.activities[0].id)
        .and_then(|c| c.moved)
        .expect("a move");
    assert!(parts.slot);
    assert!(!parts.room);
}

#[test]
fn an_unscheduled_occurrence_is_distinguished_from_one_never_in_scope() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);

    // Only occurrence 0 is movable; everything else is frozen. The "repaired" arrangement is
    // the frozen-only working one, in which occurrence 0 is unplaced.
    let movable = MovableSet::new(problem.activities.len(), &[at(0)]).expect("movable");
    let repaired = frozen_only(&baseline, &movable);

    let d = diff_of(problem, &baseline, &repaired, &movable);
    let zero = d
        .changes
        .iter()
        .find(|c| c.activity_id == problem.activities[0].id)
        .expect("occurrence 0");

    assert_eq!(zero.kind, ChangeKind::Unscheduled);
    assert!(zero.in_scope, "it was the one the repair could move");

    for change in &d.changes {
        if change.activity_id == problem.activities[0].id {
            continue;
        }
        assert!(!change.in_scope, "everything else was frozen: {change:?}");
    }
    assert_eq!(d.movable_count, 1);
    assert_eq!(d.frozen_count, problem.activities.len() - 1);
    assert!(d.is_well_formed());
}

#[test]
fn a_newly_scheduled_occurrence_is_reported_as_one() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let full = spread(problem);

    // The baseline has occurrence 0 missing; the repair places it.
    let movable = MovableSet::new(problem.activities.len(), &[at(0)]).expect("movable");
    let baseline = frozen_only(&full, &movable);

    let d = diff_of(problem, &baseline, &full, &movable);
    let zero = d
        .changes
        .iter()
        .find(|c| c.activity_id == problem.activities[0].id)
        .expect("occurrence 0");

    assert_eq!(zero.kind, ChangeKind::NewlyScheduled);
    assert!(zero.before.is_none());
    assert!(zero.after.is_some());
    assert_eq!(d.counts.newly_scheduled, 1);
    assert!(d.is_well_formed());
}

#[test]
fn the_totals_are_the_whole_arrangements_not_the_scopes() {
    // A clash between two FROZEN occurrences must appear in `findings_after` and `score_after`
    // and be attributed as pre-existing — never filtered out because it is out of scope.
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let rules = Registry::mandatory();

    let baseline = spread(problem);
    let mut clashing = baseline.clone();
    let zero = baseline.placement(at(0)).expect("placed");
    clashing.place(at(1), zero);

    // Occurrence 2 is the only movable one, so the 0/1 clash is frozen.
    let movable = MovableSet::new(problem.activities.len(), &[at(2)]).expect("movable");
    let d = diff_of(problem, &clashing, &clashing, &movable);

    let reference = evaluate(problem, &clashing, &rules);
    assert!(
        !reference.score.is_feasible(),
        "the fixture must really be clashing or this proves nothing"
    );
    assert_eq!(
        d.score_after,
        super::ScoreLine::of(reference.score),
        "the reported score is the whole arrangement's"
    );
    assert_eq!(d.findings_after.len(), reference.violations.len());
    assert!(
        !d.attribution.pre_existing_frozen.is_empty(),
        "and the frozen clash is attributed, not hidden"
    );
    assert_eq!(d.attribution.new_involving_movable, vec![]);
    assert_eq!(d.attribution.eliminated, vec![]);
}

#[test]
fn an_eliminated_violation_is_reported_as_fixed() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");

    let baseline = spread(problem);
    let mut clashing = baseline.clone();
    let zero = baseline.placement(at(0)).expect("placed");
    clashing.place(at(1), zero);

    // "Repairing" the clash back to the spread arrangement eliminates it.
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");
    let d = diff_of(problem, &clashing, &baseline, &movable);

    assert!(
        !d.attribution.eliminated.is_empty(),
        "the clash must be reported as fixed"
    );
    assert_eq!(d.attribution.new_involving_movable, vec![]);
    assert!(
        d.improved,
        "and the score improved: {:?} -> {:?}",
        d.score_before, d.score_after
    );
    assert!(d.score_after.is_better_than(d.score_before));
}

#[test]
fn an_unplaced_cause_is_carried_verbatim_and_never_re_derived() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");
    let rules = Registry::mandatory();

    let report = evaluate(problem, &baseline, &rules);
    let mut causes = BTreeMap::new();
    causes.insert(problem.activities[0].id, UnplacedCause::BlockedByPlacements);
    let locked = BTreeMap::new();
    let labels = LabelSet::default();

    let d = diff(&RepairDiffInput {
        problem,
        baseline: &baseline,
        repaired: &baseline,
        movable: &movable,
        labels: &labels,
        before: &report,
        after: &report,
        causes: &causes,
        locked: &locked,
    });

    let zero = d
        .changes
        .iter()
        .find(|c| c.activity_id == problem.activities[0].id)
        .expect("occurrence 0");
    assert_eq!(
        zero.cause.as_deref(),
        Some("blocked-by-placements"),
        "the engine's own key, not a re-derivation"
    );
}

#[test]
fn a_lock_is_reported_on_the_occurrence_it_holds() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");
    let rules = Registry::mandatory();
    let report = evaluate(problem, &baseline, &rules);

    let causes = BTreeMap::new();
    let mut locked = BTreeMap::new();
    locked.insert(problem.activities[0].id, true);
    let labels = LabelSet::default();

    let d = diff(&RepairDiffInput {
        problem,
        baseline: &baseline,
        repaired: &baseline,
        movable: &movable,
        labels: &labels,
        before: &report,
        after: &report,
        causes: &causes,
        locked: &locked,
    });

    assert!(
        d.changes
            .iter()
            .find(|c| c.activity_id == problem.activities[0].id)
            .expect("occurrence 0")
            .is_locked
    );
    assert_eq!(
        d.changes.iter().filter(|c| c.is_locked).count(),
        1,
        "and only that one"
    );
}

#[test]
fn a_placement_view_names_its_day_and_period() {
    let found = FixtureKind::Tiny.generate(2026);
    let problem = found.problems.first().expect("tiny");
    let baseline = spread(problem);
    let movable = MovableSet::everything(problem.activities.len()).expect("movable");

    let d = diff_of(problem, &baseline, &baseline, &movable);
    let placed = d
        .changes
        .iter()
        .find(|c| c.before.is_some())
        .expect("something is placed");
    let view = placed.before.as_ref().expect("a view");

    assert!(!view.day.is_empty(), "the day must be named");
    assert!(!view.period.is_empty(), "and the period");
    assert_ne!(
        view.timeslot_id,
        uuid::Uuid::nil(),
        "and the slot identifier must resolve"
    );
}

#[test]
fn the_change_kind_keys_are_stable() {
    assert_eq!(ChangeKind::Unchanged.key(), "unchanged");
    assert_eq!(ChangeKind::Moved.key(), "moved");
    assert_eq!(ChangeKind::NewlyScheduled.key(), "newly-scheduled");
    assert_eq!(ChangeKind::Unscheduled.key(), "unscheduled");
    assert_eq!(ChangeKind::StillUnplaced.key(), "still-unplaced");

    assert!(!ChangeKind::Unchanged.is_a_change());
    assert!(!ChangeKind::StillUnplaced.is_a_change());
    assert!(ChangeKind::Moved.is_a_change());
    assert!(ChangeKind::NewlyScheduled.is_a_change());
    assert!(ChangeKind::Unscheduled.is_a_change());
}
