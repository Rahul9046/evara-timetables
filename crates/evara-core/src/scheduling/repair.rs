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

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::index::ActivityIx;
use super::placement::{Assignment, Placement};
use super::problem::{ActivityFact, Problem};

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

/// Which occurrences a repair may move, as the user chose it.
///
/// # Structural and positional scopes are different, and the difference is user-visible
///
/// A class section or a teacher is a property of the **inputs**: the same occurrences are in
/// scope whatever the timetable looks like. A room or a cycle day is a property of the
/// **baseline** — which occurrences are currently there — so it is a *position*, and repair
/// changes positions.
///
/// Two consequences, both deliberate:
///
/// - The movable set is derived **once**, from the baseline, before the solve. Re-deriving it
///   as the run proceeded would make the scope depend on the search order, which is a different
///   repair for the same request.
/// - An occurrence that is **unplaced** in the baseline is in no room and on no day, so it is a
///   member of neither positional scope. A room or day repair therefore cannot schedule an
///   unscheduled lesson, which the interface must say *before* the run rather than leave to be
///   discovered in the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// `rename_all` camelCases the variant tags; `rename_all_fields` camelCases the payload fields.
// Both are needed: `CLAUDE.md` §3 requires camelCase in JSON, and without the second the
// interface would receive `section_id` beside `classSection`.
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RepairScope {
    /// Every occurrence in the problem.
    ///
    /// The only scope that is closed under the constraints: for every other one, fixing an
    /// in-scope occurrence may require moving an out-of-scope one, and the repair will report
    /// the in-scope occurrence as unplaced rather than reach outside the scope it was given.
    Whole,
    /// Every occurrence of one class section. Structural.
    ClassSection {
        /// The section's stored identifier.
        section_id: Uuid,
    },
    /// Every occurrence of every section this teacher teaches. Structural.
    Teacher {
        /// The teacher's stored identifier.
        teacher_id: Uuid,
    },
    /// Every occurrence the baseline places in one room. Positional.
    Room {
        /// The room's stored identifier.
        room_id: Uuid,
    },
    /// Every occurrence the baseline places on one cycle day. Positional.
    CycleDay {
        /// The cycle day's stored identifier.
        cycle_day_id: Uuid,
    },
    /// Exactly the occurrences the user selected.
    ///
    /// The one scope that can schedule an unplaced lesson without widening to a structural one.
    Occurrences {
        /// The occurrences' stored identifiers.
        activity_ids: Vec<Uuid>,
    },
}

impl RepairScope {
    /// A stable key, for a message and for a test that pins the vocabulary.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::ClassSection { .. } => "class-section",
            Self::Teacher { .. } => "teacher",
            Self::Room { .. } => "room",
            Self::CycleDay { .. } => "cycle-day",
            Self::Occurrences { .. } => "occurrences",
        }
    }

    /// Whether this scope is derived from the baseline rather than from the inputs.
    ///
    /// `true` for a room or a day. The interface uses this to decide whether to warn that
    /// unscheduled lessons cannot be reached by it.
    #[must_use]
    pub const fn is_positional(&self) -> bool {
        matches!(self, Self::Room { .. } | Self::CycleDay { .. })
    }
}

/// Why a scope could not be turned into a movable set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// The named section, teacher, room or cycle day is not in this problem.
    ///
    /// Not merely unknown: a room that resolves through `room_campus` but is not this school's
    /// to repair by reaches here too, because the compiled problem is one school's.
    NotInProblem {
        /// What kind of thing was named.
        entity: &'static str,
        /// Which one.
        id: Uuid,
    },
    /// The scope selects no occurrence at all.
    ///
    /// Refused rather than run: a repair over an empty scope would "succeed" having changed
    /// nothing, which is indistinguishable from one that found everything already legal.
    Empty {
        /// The scope that selected nothing.
        scope: &'static str,
    },
    /// The movable set itself is malformed.
    Movable(MovableSetError),
}

/// Turns a chosen scope into a validated movable set.
///
/// `baseline` is required even for a structural scope, because a positional one is defined by
/// it — see [`RepairScope`]. Nothing here reads a weekday: a cycle day is matched by its stored
/// identifier, never by what day of the week it resembles (`CLAUDE.md` §3).
///
/// # Errors
///
/// [`ScopeError`] when the scope names something the problem does not hold, selects nothing, or
/// produces a malformed set.
pub fn derive_movable(
    problem: &Problem,
    baseline: &Assignment,
    scope: &RepairScope,
) -> Result<MovableSet, ScopeError> {
    let total = problem.activities.len();
    let selected: Vec<ActivityIx> = match scope {
        RepairScope::Whole => (0..total)
            .map(|at| ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
            .collect(),

        RepairScope::ClassSection { section_id } => {
            let section = problem
                .sections
                .iter()
                .position(|s| s.id == *section_id)
                .ok_or(ScopeError::NotInProblem {
                    entity: "classSection",
                    id: *section_id,
                })?;
            activities_where(problem, |fact| fact.section.get() == section)
        }

        RepairScope::Teacher { teacher_id } => {
            let teacher = problem
                .teachers
                .iter()
                .position(|t| t.id == *teacher_id)
                .ok_or(ScopeError::NotInProblem {
                    entity: "teacher",
                    id: *teacher_id,
                })?;
            activities_where(problem, |fact| {
                problem
                    .sections
                    .get(fact.section.get())
                    .is_some_and(|section| section.teachers.iter().any(|at| at.get() == teacher))
            })
        }

        RepairScope::Room { room_id } => {
            let room = problem.rooms.iter().position(|r| r.id == *room_id).ok_or(
                ScopeError::NotInProblem {
                    entity: "room",
                    id: *room_id,
                },
            )?;
            // Positional: from the BASELINE, not from the inputs. An unplaced occurrence is in
            // no room and so is not selected.
            placed_where(problem, baseline, |placement| {
                placement.room.is_some_and(|at| at.get() == room)
            })
        }

        RepairScope::CycleDay { cycle_day_id } => {
            let day = problem
                .context
                .cycle_days
                .iter()
                .position(|d| d.id == *cycle_day_id)
                .ok_or(ScopeError::NotInProblem {
                    entity: "cycleDay",
                    id: *cycle_day_id,
                })?;
            placed_where(problem, baseline, |placement| {
                problem
                    .timeslots
                    .get(placement.timeslot.get())
                    .is_some_and(|slot| slot.cycle_day.get() == day)
            })
        }

        RepairScope::Occurrences { activity_ids } => {
            let mut found = Vec::with_capacity(activity_ids.len());
            for wanted in activity_ids {
                let at = problem
                    .activities
                    .iter()
                    .position(|fact| fact.id == *wanted)
                    .ok_or(ScopeError::NotInProblem {
                        entity: "activity",
                        id: *wanted,
                    })?;
                found.push(ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)));
            }
            found
        }
    };

    if selected.is_empty() {
        return Err(ScopeError::Empty { scope: scope.key() });
    }
    // Deduplicated here rather than refused: an explicit selection naming one occurrence twice
    // is a UI artefact, not a derivation bug, so it is the one place a duplicate is tolerated.
    let mut selected = selected;
    selected.sort_unstable_by_key(|at| at.raw());
    selected.dedup();
    MovableSet::new(total, &selected).map_err(ScopeError::Movable)
}

/// Every occurrence whose fact satisfies `wanted`.
fn activities_where(problem: &Problem, wanted: impl Fn(&ActivityFact) -> bool) -> Vec<ActivityIx> {
    problem
        .activities
        .iter()
        .enumerate()
        .filter(|(_, fact)| wanted(fact))
        .map(|(at, _)| ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
        .collect()
}

/// Every occurrence the baseline places somewhere satisfying `wanted`.
fn placed_where(
    problem: &Problem,
    baseline: &Assignment,
    wanted: impl Fn(Placement) -> bool,
) -> Vec<ActivityIx> {
    (0..problem.activities.len())
        .map(|at| ActivityIx::new(u32::try_from(at).unwrap_or(u32::MAX)))
        .filter(|at| baseline.placement(*at).is_some_and(&wanted))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityIx, Assignment, FrozenMismatch, MovableSet, MovableSetError, Placement,
        RepairScope, ScopeError, derive_movable, frozen_differences, frozen_only,
        locked_differences,
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

    // ===================================================================================
    // Scope derivation
    // ===================================================================================
    //
    // Behind the `fixtures` feature, as every test built on the synthetic catalogue is.
    // `npm run rust:test` runs `--all-features`, so these are part of the ordinary gate.

    /// A fixture problem and a legal arrangement over it.
    #[cfg(feature = "fixtures")]
    fn tiny() -> (crate::scheduling::Problem, Assignment) {
        let found = crate::scheduling::fixtures::FixtureKind::Tiny.generate(2026);
        let problem = found.problems.first().expect("tiny has a problem").clone();
        let mut baseline = Assignment::empty(problem.activities.len());
        let rooms = problem.rooms.len().max(1);
        for ix in 0..problem.activities.len() {
            let slot = ix / rooms;
            if slot >= problem.timeslots.len() {
                break;
            }
            baseline.place(
                at(u32::try_from(ix).expect("fits")),
                Placement {
                    timeslot: crate::scheduling::index::TimeslotIx::new(
                        u32::try_from(slot).expect("fits"),
                    ),
                    room: Some(crate::scheduling::index::RoomIx::new(
                        u32::try_from(ix % rooms).expect("fits"),
                    )),
                },
            );
        }
        (problem, baseline)
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn the_whole_scope_selects_every_occurrence() {
        let (problem, baseline) = tiny();
        let movable = derive_movable(&problem, &baseline, &RepairScope::Whole).expect("derived");
        assert_eq!(movable.len(), problem.activities.len());
        assert_eq!(movable.frozen_count(), 0);
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_class_section_scope_selects_that_sections_occurrences() {
        let (problem, baseline) = tiny();
        let section = problem.sections.first().expect("a section");
        let movable = derive_movable(
            &problem,
            &baseline,
            &RepairScope::ClassSection {
                section_id: section.id,
            },
        )
        .expect("derived");

        assert!(!movable.is_empty());
        for ix in movable.indices() {
            let fact = &problem.activities[ix.get()];
            assert_eq!(
                problem.sections[fact.section.get()].id,
                section.id,
                "only that section's occurrences"
            );
        }
        // And every occurrence of that section is in it.
        let expected = problem
            .activities
            .iter()
            .filter(|fact| problem.sections[fact.section.get()].id == section.id)
            .count();
        assert_eq!(movable.len(), expected);
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_teacher_scope_selects_every_section_they_teach() {
        let (problem, baseline) = tiny();
        let teacher = problem.teachers.first().expect("a teacher");
        let movable = derive_movable(
            &problem,
            &baseline,
            &RepairScope::Teacher {
                teacher_id: teacher.id,
            },
        )
        .expect("derived");

        for ix in movable.indices() {
            let fact = &problem.activities[ix.get()];
            let section = &problem.sections[fact.section.get()];
            assert!(
                section
                    .teachers
                    .iter()
                    .any(|at| problem.teachers[at.get()].id == teacher.id),
                "only sections this teacher teaches"
            );
        }
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_room_scope_is_derived_from_the_baseline_not_the_inputs() {
        let (problem, baseline) = tiny();
        let room = problem.rooms.first().expect("a room");
        let movable = derive_movable(&problem, &baseline, &RepairScope::Room { room_id: room.id })
            .expect("derived");

        for ix in movable.indices() {
            let placement = baseline.placement(*ix).expect("in scope means placed");
            assert_eq!(
                problem.rooms[placement.room.expect("roomed").get()].id,
                room.id,
                "only what the baseline puts in that room"
            );
        }
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_cycle_day_scope_matches_the_stored_identifier_and_never_a_weekday() {
        let (problem, baseline) = tiny();
        let day = problem.context.cycle_days.first().expect("a day");
        let movable = derive_movable(
            &problem,
            &baseline,
            &RepairScope::CycleDay {
                cycle_day_id: day.id,
            },
        )
        .expect("derived");

        for ix in movable.indices() {
            let placement = baseline.placement(*ix).expect("in scope means placed");
            let slot = &problem.timeslots[placement.timeslot.get()];
            assert_eq!(
                problem.context.cycle_days[slot.cycle_day.get()].id,
                day.id,
                "only what the baseline puts on that day"
            );
        }
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn an_unplaced_occurrence_is_in_no_positional_scope() {
        // The surprising consequence the interface has to state BEFORE the run: an unscheduled
        // lesson is in no room and on no day, so a room or day repair cannot schedule it.
        let (problem, mut baseline) = tiny();
        let orphan = at(0);
        let was = baseline.placement(orphan).expect("placed by the helper");
        baseline.clear(orphan);

        let room_id = problem.rooms[was.room.expect("roomed").get()].id;
        let room_scope =
            derive_movable(&problem, &baseline, &RepairScope::Room { room_id }).expect("derived");
        assert!(
            !room_scope.contains(orphan),
            "an unplaced occurrence is in no room"
        );

        let day = problem.timeslots[was.timeslot.get()].cycle_day;
        let cycle_day_id = problem.context.cycle_days[day.get()].id;
        let day_scope =
            derive_movable(&problem, &baseline, &RepairScope::CycleDay { cycle_day_id })
                .expect("derived");
        assert!(!day_scope.contains(orphan), "and on no day");

        // But a structural scope still reaches it, and so does an explicit selection.
        let section_id = problem.sections[problem.activities[0].section.get()].id;
        let structural = derive_movable(
            &problem,
            &baseline,
            &RepairScope::ClassSection { section_id },
        )
        .expect("derived");
        assert!(
            structural.contains(orphan),
            "a structural scope is a property of the inputs"
        );

        let explicit = derive_movable(
            &problem,
            &baseline,
            &RepairScope::Occurrences {
                activity_ids: vec![problem.activities[0].id],
            },
        )
        .expect("derived");
        assert!(explicit.contains(orphan));
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_scope_naming_something_the_problem_does_not_hold_is_refused() {
        let (problem, baseline) = tiny();
        let stranger = uuid::Uuid::nil();

        for (scope, entity) in [
            (
                RepairScope::ClassSection {
                    section_id: stranger,
                },
                "classSection",
            ),
            (
                RepairScope::Teacher {
                    teacher_id: stranger,
                },
                "teacher",
            ),
            (RepairScope::Room { room_id: stranger }, "room"),
            (
                RepairScope::CycleDay {
                    cycle_day_id: stranger,
                },
                "cycleDay",
            ),
            (
                RepairScope::Occurrences {
                    activity_ids: vec![stranger],
                },
                "activity",
            ),
        ] {
            assert_eq!(
                derive_movable(&problem, &baseline, &scope),
                Err(ScopeError::NotInProblem {
                    entity,
                    id: stranger
                }),
                "{} must be refused",
                scope.key()
            );
        }
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn a_positional_scope_that_selects_nothing_is_refused_not_run() {
        let (problem, _) = tiny();
        // An empty baseline: nothing is in any room, so a room scope selects nothing.
        let empty = Assignment::empty(problem.activities.len());
        let room_id = problem.rooms.first().expect("a room").id;
        assert_eq!(
            derive_movable(&problem, &empty, &RepairScope::Room { room_id }),
            Err(ScopeError::Empty { scope: "room" })
        );
    }

    #[test]
    #[cfg(feature = "fixtures")]
    fn an_explicit_selection_tolerates_a_repeated_identifier() {
        // A UI artefact, not a derivation bug, so it is deduplicated rather than refused --
        // the one place a duplicate is tolerated.
        let (problem, baseline) = tiny();
        let id = problem.activities[0].id;
        let movable = derive_movable(
            &problem,
            &baseline,
            &RepairScope::Occurrences {
                activity_ids: vec![id, id, id],
            },
        )
        .expect("derived");
        assert_eq!(movable.len(), 1);
    }

    #[test]
    fn the_scope_keys_and_positional_flags_are_stable() {
        assert_eq!(RepairScope::Whole.key(), "whole");
        assert!(!RepairScope::Whole.is_positional());
        let nil = uuid::Uuid::nil();
        assert_eq!(
            RepairScope::ClassSection { section_id: nil }.key(),
            "class-section"
        );
        assert!(!RepairScope::ClassSection { section_id: nil }.is_positional());
        assert_eq!(RepairScope::Teacher { teacher_id: nil }.key(), "teacher");
        assert!(!RepairScope::Teacher { teacher_id: nil }.is_positional());
        assert_eq!(RepairScope::Room { room_id: nil }.key(), "room");
        assert!(RepairScope::Room { room_id: nil }.is_positional());
        assert_eq!(
            RepairScope::CycleDay { cycle_day_id: nil }.key(),
            "cycle-day"
        );
        assert!(RepairScope::CycleDay { cycle_day_id: nil }.is_positional());
        assert_eq!(
            RepairScope::Occurrences {
                activity_ids: vec![]
            }
            .key(),
            "occurrences"
        );
        assert!(
            !RepairScope::Occurrences {
                activity_ids: vec![]
            }
            .is_positional()
        );
    }
}
