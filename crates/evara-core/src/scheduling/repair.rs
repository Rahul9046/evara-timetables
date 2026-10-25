//! What a repair may move, and the proof that it moved nothing else.
//!
//! # Why this is in `evara-core` and not in the solver
//!
//! Three crates have to agree about the same two questions, and `CLAUDE.md` §2's most important
//! rule is that a predicate with two implementations is a defect waiting to happen:
//!
//! - **`evara-solver`** derives the frozen-only arrangement, verifies its own answer, and
//!   refuses one that disturbed a frozen placement.
//! - **`evara-db`** verifies it **again** inside the writing transaction, because a repair that
//!   passed in memory and was then persisted from a different arrangement would write a
//!   timetable nobody checked.
//! - **`evara-report`** partitions violations by the same movable set to attribute them.
//!
//! `evara-db` may not depend on `evara-solver`, so the only place all three can share one
//! implementation is here. [`frozen_differences`] is that implementation, and neither caller
//! restates it.
//!
//! # The scope limits what may move; it does not limit what is checked
//!
//! A frozen occurrence is in the arrangement, visible to every constraint, and never a
//! candidate. That is the whole of the scope mechanism — see
//! [M9B-REPAIR-PROPOSAL.md](../../../../../docs/M9B-REPAIR-PROPOSAL.md) §2.1. Nothing in this
//! module judges legality; it judges only whether a placement the repair promised not to touch
//! is byte-identical to what it started as.
//!
//! # Equality is over the whole `Placement`, not a digest
//!
//! The proposal's first draft compared violation sets by `ViolationKey` and inferred scope
//! preservation from them. That is not a proof: two arrangements can carry identical violation
//! sets and still differ — a frozen occurrence moved to an equally-clashing slot, or swapped
//! with another frozen occurrence, produces the same keys. So the comparison here is over the
//! **assignments**, occurrence by occurrence, including the room and including the unplaced
//! state. The violation-set comparison is kept as a second, independent check rather than as
//! the primary one.

use super::index::ActivityIx;
use super::placement::{Assignment, Placement};

/// The occurrences a repair may place, validated once.
///
/// Sorted and deduplicated on construction, with a membership bitmap so
/// [`MovableSet::contains`] is `O(1)` rather than a scan per probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovableSet {
    /// Sorted, deduplicated, every index `< total`.
    indices: Vec<ActivityIx>,
    /// `member[i]` is whether activity `i` may move. Length is the problem's activity count.
    member: Vec<bool>,
}

/// Why a requested movable set is not usable.
///
/// Each is a malformed *request* rather than a scheduling outcome, which is why they are
/// refused before anything is compiled rather than reported as a failed repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovableSetError {
    /// Nothing in the chosen scope can move.
    ///
    /// Refused rather than run. A repair over an empty scope would "succeed" having changed
    /// nothing, which is indistinguishable from a repair that found everything already legal —
    /// and those are different answers to different questions.
    Empty,
    /// An index that no activity has.
    OutOfRange {
        /// The offending index.
        activity: u32,
        /// How many activities the problem holds.
        total: usize,
    },
    /// One occurrence named twice.
    ///
    /// Refused rather than collapsed: a duplicate means whatever derived the set is wrong, and
    /// silently deduplicating would hide it. Note that [`MovableSet::new`] *does* deduplicate
    /// after this check, so the invariant holds for a set built any other way.
    Duplicate {
        /// The index named more than once.
        activity: u32,
    },
}

impl MovableSet {
    /// Validates and canonicalises a requested movable set.
    ///
    /// # Errors
    ///
    /// [`MovableSetError`] for an empty, out-of-range or duplicated request — the validation
    /// the proposal's §2.10 requires before a repair is compiled.
    pub fn new(total: usize, requested: &[ActivityIx]) -> Result<Self, MovableSetError> {
        if requested.is_empty() {
            return Err(MovableSetError::Empty);
        }
        let mut member = vec![false; total];
        for activity in requested {
            let at = activity.get();
            let slot = member.get_mut(at).ok_or(MovableSetError::OutOfRange {
                activity: activity.raw(),
                total,
            })?;
            if *slot {
                return Err(MovableSetError::Duplicate {
                    activity: activity.raw(),
                });
            }
            *slot = true;
        }
        let mut indices = requested.to_vec();
        // Sorted so the construction order — and therefore the whole outcome — cannot depend on
        // the order a scope derivation happened to produce. `SOLVER.md` §5's reproducibility.
        indices.sort_unstable_by_key(|activity| activity.raw());
        Ok(Self { indices, member })
    }

    /// The whole problem, as one movable set. The whole-timetable scope.
    ///
    /// # Errors
    ///
    /// [`MovableSetError::Empty`] when the problem holds no activities.
    pub fn everything(total: usize) -> Result<Self, MovableSetError> {
        let all: Vec<ActivityIx> = (0..total)
            .map(|at| ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
            .collect();
        Self::new(total, &all)
    }

    /// Whether this occurrence may move.
    #[must_use]
    pub fn contains(&self, activity: ActivityIx) -> bool {
        self.member.get(activity.get()).copied().unwrap_or(false)
    }

    /// The movable occurrences, sorted.
    #[must_use]
    pub fn indices(&self) -> &[ActivityIx] {
        &self.indices
    }

    /// How many may move.
    #[must_use]
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether nothing may move. Never true for a set built by [`MovableSet::new`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// How many occurrences are frozen.
    #[must_use]
    pub fn frozen_count(&self) -> usize {
        self.member.len().saturating_sub(self.indices.len())
    }

    /// How many activities the problem this set was built for holds.
    #[must_use]
    pub fn total(&self) -> usize {
        self.member.len()
    }

    /// The frozen occurrences, in index order.
    pub fn frozen(&self) -> impl Iterator<Item = ActivityIx> + '_ {
        self.member
            .iter()
            .enumerate()
            .filter(|(_, movable)| !**movable)
            .map(|(at, _)| ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
    }
}

/// The arrangement a repair starts from: the baseline with every movable occurrence withdrawn.
///
/// **Subtraction, not construction.** Cloning the baseline and clearing the movable set makes
/// "every frozen placement survived" depend on the *movable* derivation, which
/// [`MovableSet::new`] has already validated; building up from an empty assignment and placing
/// the frozen set would make it depend on the frozen derivation instead, which nothing checks.
/// It also inherits the baseline's dimension, so the assignment cannot be the wrong size.
#[must_use]
pub fn frozen_only(baseline: &Assignment, movable: &MovableSet) -> Assignment {
    let mut working = baseline.clone();
    for activity in movable.indices() {
        working.clear(*activity);
    }
    working
}

/// One frozen occurrence that a repair changed, which it is never allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenMismatch {
    /// It is placed in both, in different places.
    Moved {
        /// Which occurrence.
        activity: ActivityIx,
        /// Where it was.
        baseline: Placement,
        /// Where the repair left it.
        repaired: Placement,
    },
    /// It was placed and is now unplaced — a lesson lost.
    Dropped {
        /// Which occurrence.
        activity: ActivityIx,
        /// Where it was.
        baseline: Placement,
    },
    /// It was unplaced and is now placed — a lesson invented outside the scope.
    Added {
        /// Which occurrence.
        activity: ActivityIx,
        /// Where the repair put it.
        repaired: Placement,
    },
}

impl FrozenMismatch {
    /// Which occurrence this is about.
    #[must_use]
    pub const fn activity(&self) -> ActivityIx {
        match self {
            Self::Moved { activity, .. }
            | Self::Dropped { activity, .. }
            | Self::Added { activity, .. } => *activity,
        }
    }

    /// A stable key, for a message and for a test that pins the vocabulary.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Moved { .. } => "frozen-placement-moved",
            Self::Dropped { .. } => "frozen-placement-dropped",
            Self::Added { .. } => "frozen-placement-added",
        }
    }
}

/// Every frozen occurrence whose placement is not byte-identical to the baseline's.
///
/// Empty is the only acceptable answer. A non-empty result is an **internal consistency
/// failure** — the repair changed something it was told not to touch — and is never an
/// applyable outcome, whatever else the run achieved.
///
/// Compares the whole [`Placement`] (slot *and* room) and the unplaced state in both
/// directions, so a move, a loss and an invention are each caught and distinguished. This is
/// the comparison the solver performs before accepting an outcome and `evara-db` performs
/// again inside the writing transaction.
#[must_use]
pub fn frozen_differences(
    baseline: &Assignment,
    repaired: &Assignment,
    movable: &MovableSet,
) -> Vec<FrozenMismatch> {
    let mut found = Vec::new();
    for activity in movable.frozen() {
        match (baseline.placement(activity), repaired.placement(activity)) {
            (Some(was), Some(now)) if was != now => found.push(FrozenMismatch::Moved {
                activity,
                baseline: was,
                repaired: now,
            }),
            (Some(was), None) => found.push(FrozenMismatch::Dropped {
                activity,
                baseline: was,
            }),
            (None, Some(now)) => found.push(FrozenMismatch::Added {
                activity,
                repaired: now,
            }),
            // Identical placement, or unplaced in both. Both are preservation.
            (Some(_), Some(_)) | (None, None) => {}
        }
    }
    found
}

/// Every locked occurrence whose placement is not byte-identical to the baseline's.
///
/// A lock is frozen, so [`frozen_differences`] already covers it — this is the narrower
/// question asked separately because the guarantee is separate, the remedy is different, and a
/// future change that made a lock movable would otherwise be caught only by the general check.
///
/// `pinned` is `Problem::pinned`'s shape: one entry per activity, or empty when nothing is
/// locked.
#[must_use]
pub fn locked_differences(
    baseline: &Assignment,
    repaired: &Assignment,
    pinned: &[Option<Placement>],
) -> Vec<FrozenMismatch> {
    let mut found = Vec::new();
    for (at, pin) in pinned.iter().enumerate() {
        let Some(pin) = pin else { continue };
        let activity = ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX));
        let now = repaired.placement(activity);
        // Measured against the pin itself rather than against the baseline, because the pin is
        // what the lock promises. They agree for a well-formed baseline, and where they do not
        // the pin is the authority.
        let _ = baseline;
        match now {
            Some(now) if now != *pin => found.push(FrozenMismatch::Moved {
                activity,
                baseline: *pin,
                repaired: now,
            }),
            None => found.push(FrozenMismatch::Dropped {
                activity,
                baseline: *pin,
            }),
            Some(_) => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityIx, Assignment, FrozenMismatch, MovableSet, MovableSetError, Placement,
        frozen_differences, frozen_only, locked_differences,
    };
    use crate::scheduling::index::{RoomIx, TimeslotIx};

    fn at(raw: u32) -> ActivityIx {
        ActivityIx::new(raw)
    }

    fn slot(raw: u32) -> TimeslotIx {
        TimeslotIx::new(raw)
    }

    fn placed(timeslot: u32, room: u32) -> Placement {
        Placement::in_room(slot(timeslot), RoomIx::new(room))
    }

    #[test]
    fn a_movable_set_is_sorted_and_its_membership_is_constant_time() {
        let set = MovableSet::new(5, &[at(3), at(1), at(4)]).expect("valid");
        assert_eq!(set.indices(), &[at(1), at(3), at(4)]);
        assert!(set.contains(at(1)));
        assert!(!set.contains(at(0)));
        assert!(!set.contains(at(2)));
        assert_eq!(set.len(), 3);
        assert_eq!(set.frozen_count(), 2);
        assert_eq!(set.total(), 5);
        assert_eq!(set.frozen().collect::<Vec<_>>(), vec![at(0), at(2)]);
    }

    #[test]
    fn an_empty_movable_set_is_refused_not_run() {
        assert_eq!(MovableSet::new(4, &[]), Err(MovableSetError::Empty));
    }

    #[test]
    fn a_duplicated_movable_index_is_refused_not_collapsed() {
        assert_eq!(
            MovableSet::new(4, &[at(1), at(2), at(1)]),
            Err(MovableSetError::Duplicate { activity: 1 })
        );
    }

    #[test]
    fn an_out_of_range_movable_index_is_refused() {
        assert_eq!(
            MovableSet::new(2, &[at(0), at(7)]),
            Err(MovableSetError::OutOfRange {
                activity: 7,
                total: 2
            })
        );
    }

    #[test]
    fn everything_is_the_whole_problem() {
        let set = MovableSet::everything(3).expect("valid");
        assert_eq!(set.indices(), &[at(0), at(1), at(2)]);
        assert_eq!(set.frozen_count(), 0);
        assert_eq!(set.frozen().count(), 0);
        assert_eq!(MovableSet::everything(0), Err(MovableSetError::Empty));
    }

    #[test]
    fn the_working_arrangement_holds_only_frozen_placements() {
        let mut baseline = Assignment::empty(4);
        baseline.place(at(0), placed(10, 1));
        baseline.place(at(1), placed(11, 2));
        baseline.place(at(2), placed(12, 3));
        // at(3) deliberately left unplaced.

        let movable = MovableSet::new(4, &[at(1), at(3)]).expect("valid");
        let working = frozen_only(&baseline, &movable);

        assert_eq!(working.placement(at(0)), Some(placed(10, 1)), "frozen kept");
        assert_eq!(working.placement(at(2)), Some(placed(12, 3)), "frozen kept");
        assert_eq!(working.placement(at(1)), None, "movable withdrawn");
        assert_eq!(working.placement(at(3)), None, "movable still unplaced");
        assert_eq!(working.scheduled_count(), 2);
        assert_eq!(working.len(), baseline.len(), "dimension inherited");
    }

    #[test]
    fn an_unchanged_frozen_set_has_no_differences() {
        let mut baseline = Assignment::empty(3);
        baseline.place(at(0), placed(10, 1));
        baseline.place(at(2), placed(12, 3));
        let movable = MovableSet::new(3, &[at(1)]).expect("valid");

        let mut repaired = baseline.clone();
        repaired.place(at(1), placed(20, 4));

        assert_eq!(frozen_differences(&baseline, &repaired, &movable), vec![]);
    }

    #[test]
    fn a_moved_frozen_placement_is_caught_including_a_room_only_change() {
        let mut baseline = Assignment::empty(2);
        baseline.place(at(0), placed(10, 1));
        let movable = MovableSet::new(2, &[at(1)]).expect("valid");

        let mut repaired = baseline.clone();
        // Same slot, different room. A digest over slots alone would miss this.
        repaired.place(at(0), placed(10, 9));

        assert_eq!(
            frozen_differences(&baseline, &repaired, &movable),
            vec![FrozenMismatch::Moved {
                activity: at(0),
                baseline: placed(10, 1),
                repaired: placed(10, 9),
            }]
        );
    }

    #[test]
    fn a_dropped_frozen_placement_is_caught() {
        let mut baseline = Assignment::empty(2);
        baseline.place(at(0), placed(10, 1));
        let movable = MovableSet::new(2, &[at(1)]).expect("valid");

        let mut repaired = baseline.clone();
        repaired.clear(at(0));

        assert_eq!(
            frozen_differences(&baseline, &repaired, &movable),
            vec![FrozenMismatch::Dropped {
                activity: at(0),
                baseline: placed(10, 1),
            }]
        );
    }

    #[test]
    fn a_frozen_occurrence_placed_by_the_repair_is_caught() {
        let baseline = Assignment::empty(2);
        let movable = MovableSet::new(2, &[at(1)]).expect("valid");

        let mut repaired = baseline.clone();
        repaired.place(at(0), placed(10, 1));

        assert_eq!(
            frozen_differences(&baseline, &repaired, &movable),
            vec![FrozenMismatch::Added {
                activity: at(0),
                repaired: placed(10, 1),
            }]
        );
    }

    #[test]
    fn an_unplaced_frozen_occurrence_staying_unplaced_is_preservation() {
        let baseline = Assignment::empty(2);
        let movable = MovableSet::new(2, &[at(1)]).expect("valid");
        let repaired = baseline.clone();
        assert_eq!(frozen_differences(&baseline, &repaired, &movable), vec![]);
    }

    #[test]
    fn a_swap_of_two_frozen_occurrences_is_caught() {
        // The case a violation-set comparison cannot see: both arrangements carry the same
        // violations, and two frozen lessons have exchanged places.
        let mut baseline = Assignment::empty(3);
        baseline.place(at(0), placed(10, 1));
        baseline.place(at(1), placed(11, 2));
        let movable = MovableSet::new(3, &[at(2)]).expect("valid");

        let mut repaired = Assignment::empty(3);
        repaired.place(at(0), placed(11, 2));
        repaired.place(at(1), placed(10, 1));

        let found = frozen_differences(&baseline, &repaired, &movable);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|m| m.key() == "frozen-placement-moved"));
    }

    #[test]
    fn a_locked_placement_is_checked_against_its_pin() {
        let mut baseline = Assignment::empty(3);
        baseline.place(at(0), placed(10, 1));
        let pinned = vec![Some(placed(10, 1)), None, None];

        let mut repaired = baseline.clone();
        assert_eq!(locked_differences(&baseline, &repaired, &pinned), vec![]);

        repaired.place(at(0), placed(44, 1));
        assert_eq!(
            locked_differences(&baseline, &repaired, &pinned),
            vec![FrozenMismatch::Moved {
                activity: at(0),
                baseline: placed(10, 1),
                repaired: placed(44, 1),
            }]
        );

        repaired.clear(at(0));
        assert_eq!(
            locked_differences(&baseline, &repaired, &pinned),
            vec![FrozenMismatch::Dropped {
                activity: at(0),
                baseline: placed(10, 1),
            }]
        );
    }

    #[test]
    fn an_empty_pinned_list_locks_nothing() {
        let baseline = Assignment::empty(2);
        let repaired = Assignment::empty(2);
        assert_eq!(locked_differences(&baseline, &repaired, &[]), vec![]);
    }
}
