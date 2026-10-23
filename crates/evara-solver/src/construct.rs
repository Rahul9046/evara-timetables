//! Phase A: greedy insertion, hardest first.
//!
//! # The algorithm `SOLVER.md` §5 specifies
//!
//! > **Phase A — Construction.** Greedy insertion, hardest first. Activities are ordered by
//! > descending *constrainedness*: pinned, then long duration, then few feasible
//! > (timeslot, room) pairs, then high conflict degree (shares teachers/groups with many
//! > others). Each is placed in its cheapest feasible slot. Locked activities are placed first
//! > and never moved.
//!
//! That is implemented as written, and the four keys are in that order in [`constrainedness`].
//! The roadmap's Phase 6 line calls the same thing *"most-constrained-first, locks placed
//! first"*.
//!
//! # Static order, plus retry passes
//!
//! The order above is computed **once**, from the empty timetable. The textbook alternative is
//! to recompute it at every step and always take the occurrence with the fewest *currently*
//! feasible placements, which is a better heuristic and is what the M6 brief suggested.
//! It is not what is implemented, for two reasons.
//!
//! The first is that `SOLVER.md` is the accepted record and specifies a ranking rather than a
//! re-ranking. The second is cost: proving an occurrence now has **zero** feasible placements
//! means testing all of them, so a dynamic re-rank is `O(occurrences × candidates)` per step —
//! for a 500-occurrence school, hundreds of millions of constraint queries to place 500
//! lessons.
//!
//! What recovers most of the benefit for almost nothing is the **retry pass**: after the first
//! sweep, the occurrences that failed are tried again, because the timetable has changed since
//! they were refused. Passes repeat until one places nothing, which is a deterministic
//! fixed-point. An occurrence that failed early because the grid was still filling gets another
//! look; what the loop cannot recover is a *choice* made early that blocks something later,
//! and recovering that needs backtracking or the local search of Phase B — out of M6's scope by
//! the brief and `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6, which keeps local search out of the
//! MVP.
//!
//! The cost of the difference is visible and reported rather than hidden:
//! [`Termination::Exhausted`] means every remaining occurrence was refused at every candidate
//! *in the arrangement the constructor had reached*, and
//! [`UnplacedCause::BlockedByPlacements`] says exactly that. Neither claims the problem is
//! impossible.
//!
//! # No placement is accepted without the registry's permission
//!
//! Candidate generation pre-filters four of the seven hard rules by calling the functions that
//! own them ([`crate::candidates`]). The other three are about other placements, and the
//! engine answers them: every candidate goes through
//! [`IncrementalScorer::probe`](evara_constraints::IncrementalScorer), and a candidate whose
//! verdict is not `allowed` is never committed. There is no fast path around that check and no
//! approximation of it — `CLAUDE.md` §2's most important rule — and
//! [`crate::generate`] re-evaluates the finished arrangement with the reference scorer to prove
//! it.

use evara_constraints::{IncrementalScorer, PlacementVerdict};
use evara_core::scheduling::{
    ActivityIx, CompiledConstraint, Placement, Problem, SolveStatistics, Termination, UnplacedCause,
};

use crate::candidates::{
    Candidates, RoomChoices, RunCache, ValueRange, placement_of, run_is_available, value_range,
};
use crate::config::{Cancel, ProgressSink, SolveConfig};

/// What the construction phase ended up with.
#[derive(Debug)]
pub(crate) struct Construction<'p> {
    /// The scorer holding the arrangement, so the caller can report and verify it.
    pub(crate) scorer: IncrementalScorer<'p>,
    /// Why it stopped.
    pub(crate) termination: Termination,
    /// What it did, counted.
    pub(crate) statistics: SolveStatistics,
    /// Every occurrence that is not placed, and the most specific cause known for it.
    ///
    /// In activity-index order, so a report is deterministic.
    pub(crate) unplaced: Vec<(ActivityIx, UnplacedCause)>,
}

/// Everything precomputed per occurrence, once, before the first placement.
#[derive(Debug)]
struct Prepared {
    /// The occurrence.
    activity: ActivityIx,
    /// Its value range over the empty timetable.
    range: ValueRange,
    /// How many other occurrences share a teacher or an audience with it.
    conflict_degree: usize,
    /// How many periods it needs.
    duration: usize,
    /// Whether it is pinned, and so placed first and never moved.
    pinned: Option<Placement>,
}

/// Runs the construction phase.
///
/// Returns whatever it reached, always: a partial arrangement is the result `MEDIUM`-level
/// `activity-scheduled` exists to make returnable, and discarding it would leave a school with
/// nothing to look at and nothing to fix.
pub(crate) fn construct<'p>(
    problem: &'p Problem,
    constraints: &[CompiledConstraint],
    config: &SolveConfig,
    cancel: &dyn Cancel,
    progress: &dyn ProgressSink,
) -> Construction<'p> {
    let mut cache = RunCache::default();
    let mut scorer = IncrementalScorer::empty(problem, constraints);
    let mut statistics = SolveStatistics::default();
    let total = problem.activities.len();

    let order = prepare(problem, &mut cache, config);
    // Occurrences whose value range is empty cannot be placed by any arrangement, so they are
    // settled before the first candidate is tested and never retried. Keeping them in the retry
    // set would cost a sweep per pass to reach the same answer.
    let mut unplaced: Vec<(ActivityIx, UnplacedCause)> = order
        .iter()
        .filter_map(|p| p.range.empty_because.map(|cause| (p.activity, cause)))
        .collect();

    let mut pending: Vec<&Prepared> = order
        .iter()
        .filter(|p| p.range.empty_because.is_none())
        .collect();

    let mut termination = Termination::Exhausted;
    // The cause of each pending occurrence's most recent refusal. Cleared at the end of a
    // productive pass, because the next pass re-attempts every one of them against a changed
    // timetable and a stale cause would describe an arrangement that no longer exists.
    let mut refusals: Vec<(ActivityIx, UnplacedCause)> = Vec::new();

    loop {
        if pending.is_empty() {
            break;
        }
        if statistics.passes >= config.limits.max_passes {
            termination = Termination::PassLimitReached;
            refusals = not_reached(&pending);
            break;
        }
        statistics.passes += 1;

        // Taken rather than iterated in place, so a stop can put the untouched tail back.
        let queue = std::mem::take(&mut pending);
        let mut still_pending: Vec<&Prepared> = Vec::new();
        let mut placed_this_pass = 0_usize;
        let mut stopped: Option<Termination> = None;

        for (at, prepared) in queue.iter().copied().enumerate() {
            if cancel.is_cancelled() {
                stopped = Some(Termination::Cancelled);
            } else if statistics.construction_steps >= config.limits.max_construction_steps {
                stopped = Some(Termination::StepLimitReached);
            } else {
                statistics.construction_steps += 1;
                match place_one(
                    problem,
                    &mut cache,
                    &mut scorer,
                    &mut statistics,
                    config,
                    prepared,
                ) {
                    Attempt::Placed => {
                        statistics.placements_committed += 1;
                        placed_this_pass += 1;
                    }
                    Attempt::Refused(cause) => {
                        refusals.push((prepared.activity, cause));
                        still_pending.push(prepared);
                    }
                    Attempt::OutOfBudget => stopped = Some(Termination::CandidateLimitReached),
                }
                progress.placed(scorer.placed_count(), total, statistics.construction_steps);
            }

            if stopped.is_some() {
                // This occurrence and **every one after it**. Nothing was established about any
                // of them, and a report that silently omitted the tail would be the engine
                // bounding its coverage without saying so.
                refusals.extend(not_reached(&queue[at..]));
                still_pending.extend(queue[at..].iter().copied());
                break;
            }
        }

        pending = still_pending;
        if let Some(reason) = stopped {
            termination = reason;
            break;
        }
        // A pass that placed nothing cannot be improved on by repeating it: the arrangement is
        // unchanged and the candidate order is a pure function of the seed, so the next pass
        // would make the identical decisions. This is the fixed point that ends the loop.
        if placed_this_pass == 0 {
            break;
        }
        refusals.clear();
    }

    // Every pending occurrence kept the cause of its most recent refusal, and a stop filled the
    // tail with `NotReached`. The second loop is a belt-and-braces guarantee of the property the
    // caller depends on — **every unplaced occurrence is reported** — so a future change to the
    // bookkeeping cannot make the list silently short.
    unplaced.extend(refusals);
    for prepared in &pending {
        if !unplaced.iter().any(|(at, _)| *at == prepared.activity) {
            unplaced.push((prepared.activity, UnplacedCause::NotReached));
        }
    }
    unplaced.sort_by_key(|(at, _)| at.raw());
    unplaced.dedup_by_key(|(at, _)| *at);
    debug_assert_eq!(
        unplaced.len() + scorer.placed_count(),
        total,
        "every occurrence is either placed or reported as unplaced"
    );

    if scorer.placed_count() == total {
        termination = Termination::Complete;
    }

    Construction {
        scorer,
        termination,
        statistics,
        unplaced,
    }
}

/// How one attempt at one occurrence ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// It was placed.
    Placed,
    /// Every candidate was refused, for this reason.
    Refused(UnplacedCause),
    /// The candidate budget ran out before every candidate was tried.
    OutOfBudget,
}

/// Tries every candidate for one occurrence and commits the first the engine allows.
///
/// # "Cheapest feasible slot", and what that means while nothing is soft
///
/// `SOLVER.md` §5 says each occurrence is placed *"in its cheapest feasible slot"*. Cost is the
/// score, and no soft constraint is implemented — `CONSTRAINT-ENGINE.md` §9 — so every feasible
/// candidate for one occurrence has an identical delta: it resolves that occurrence's one
/// `activity-scheduled` violation and adds nothing. There is nothing to compare, and a
/// comparison would be a tie broken by a pretence of preference.
///
/// So the first feasible candidate in the seeded order wins, and the **order** is where the
/// spreading comes from ([`crate::candidates`]). When a soft rule exists this becomes a real
/// minimisation over the same candidate walk, and the walk does not have to change: the deltas
/// are already there in [`PlacementVerdict::delta`].
fn place_one(
    problem: &Problem,
    cache: &mut RunCache,
    scorer: &mut IncrementalScorer<'_>,
    statistics: &mut SolveStatistics,
    config: &SolveConfig,
    prepared: &Prepared,
) -> Attempt {
    // A pinned occurrence is placed where it is pinned, and nothing else is tried. Nothing
    // populates `Problem::pinned` at `V16` — there is no `activity.is_locked` column — so this
    // branch is unreachable today and is written because repair is the mechanism partial
    // regeneration is built from (`SOLVER.md` §7) and a pin honoured by accident later is a pin
    // not honoured.
    if let Some(pinned) = prepared.pinned {
        statistics.candidate_evaluations += 1;
        return match scorer.probe(prepared.activity, pinned) {
            verdict if verdict.allowed => {
                scorer.place(prepared.activity, pinned);
                Attempt::Placed
            }
            verdict => Attempt::Refused(cause_of(&verdict)),
        };
    }

    let Some(fact) = problem.activities.get(prepared.activity.get()) else {
        return Attempt::Refused(UnplacedCause::NoResolvedBellSchedule);
    };
    let Some(section) = problem.sections.get(fact.section.get()) else {
        return Attempt::Refused(UnplacedCause::NoResolvedBellSchedule);
    };
    let Some(schedule) = section.bell_schedule.schedule() else {
        return Attempt::Refused(UnplacedCause::NoResolvedBellSchedule);
    };

    let runs = cache
        .runs(problem, schedule, fact.duration.as_usize())
        .to_vec();
    let choices = RoomChoices::of(section);
    let mut walk = Candidates::new(
        config.seed.get(),
        prepared.activity,
        runs.len(),
        choices.len(),
    );

    // The most specific reason seen so far. A clash is more informative than an availability
    // refusal, because availability would have emptied the value range before the solve; so a
    // refusal reached here is nearly always a clash, and the field exists so an unusual one is
    // reported rather than overwritten.
    let mut cause = UnplacedCause::BlockedByPlacements;
    let mut tried_any = false;

    while let Some((run_at, room_at)) = walk.next() {
        let run = &runs[run_at];
        let room = choices.at(room_at);
        // The availability pre-filter, from the same index `entity-unavailable` reads. Costs no
        // candidate evaluation because no rule was asked: it is the generator declining to
        // offer a placement, not the engine refusing one.
        if !run_is_available(problem, section, &run.slots, room) {
            continue;
        }

        if statistics.candidate_evaluations >= config.limits.max_candidate_evaluations {
            return Attempt::OutOfBudget;
        }
        statistics.candidate_evaluations += 1;
        tried_any = true;

        let placement = placement_of(run, room);
        let verdict = scorer.probe(prepared.activity, placement);
        if verdict.allowed {
            scorer.place(prepared.activity, placement);
            return Attempt::Placed;
        }
        statistics.candidates_rejected += 1;
        cause = cause_of(&verdict);
    }

    Attempt::Refused(if tried_any {
        cause
    } else {
        // Every candidate was filtered out by availability. The value range was non-empty when
        // it was measured, so this is reachable only when a stored rule leaves no run free in
        // combination with the rooms still available — reported as the availability cause
        // because that is what did the filtering.
        UnplacedCause::AllCandidatesUnavailable
    })
}

/// Which cause a refusal's blocking violations point at.
///
/// Reads the rules the engine named rather than guessing, so the cause and the explanation
/// cannot disagree. A clash with something already placed is a property of this arrangement;
/// the other two are properties the value range should already have caught, and are reported
/// honestly if they appear.
fn cause_of(verdict: &PlacementVerdict) -> UnplacedCause {
    use evara_core::scheduling::ConstraintType;

    let mut cause = UnplacedCause::BlockedByPlacements;
    for violation in &verdict.blocking {
        match violation.constraint_type {
            ConstraintType::TeacherClash
            | ConstraintType::StudentClash
            | ConstraintType::RoomClash => return UnplacedCause::BlockedByPlacements,
            ConstraintType::EntityUnavailable => {
                cause = UnplacedCause::AllCandidatesUnavailable;
            }
            ConstraintType::RoomTypeRequired => cause = UnplacedCause::NoCandidateRoom,
            ConstraintType::DurationContiguous | ConstraintType::NoTeachingInPeriod => {
                cause = UnplacedCause::NoContiguousRun;
            }
            ConstraintType::ActivityScheduled => {}
        }
    }
    cause
}

/// Every occurrence in the pending list, as "not reached".
fn not_reached(pending: &[&Prepared]) -> Vec<(ActivityIx, UnplacedCause)> {
    pending
        .iter()
        .map(|p| (p.activity, UnplacedCause::NotReached))
        .collect()
}

/// Measures every occurrence and sorts them hardest first.
fn prepare(problem: &Problem, cache: &mut RunCache, config: &SolveConfig) -> Vec<Prepared> {
    let degrees = conflict_degrees(problem);
    let mut prepared: Vec<Prepared> = problem
        .activity_indices()
        .map(|activity| Prepared {
            activity,
            range: value_range(problem, cache, activity),
            conflict_degree: degrees.get(activity.get()).copied().unwrap_or_default(),
            duration: problem
                .activities
                .get(activity.get())
                .map_or(1, |fact| fact.duration.as_usize()),
            pinned: problem.pinned.get(activity.get()).copied().flatten(),
        })
        .collect();

    prepared.sort_by_key(constrainedness);
    let _ = config;
    prepared
}

/// `SOLVER.md` §5's ranking, as a sort key. Smaller sorts earlier, so each term is negated
/// where "more constrained" means "larger".
///
/// The four keys, in the record's own order:
///
/// 1. **Pinned first.** They cannot move, so everything else must fit around them.
/// 2. **Long duration.** A three-period occurrence has strictly fewer homes than a single, and
///    placing it after the singles is how a day ends up with three gaps of one period.
/// 3. **Few feasible pairs.** The value range measured over the empty timetable — an occurrence
///    with four candidates must be placed before one with four hundred.
/// 4. **High conflict degree.** How many other occurrences share a teacher or an audience with
///    it, which is how much of the timetable its placement constrains.
///
/// The fifth key is the activity index, and it is not a heuristic: it is what makes the order
/// **total**, so two runs at the same seed sort identically. `SOLVER.md` §5 calls
/// reproducibility non-negotiable, and a sort with ties broken by whatever order the input
/// happened to be in is not reproducible across a change in the loader.
fn constrainedness(p: &Prepared) -> (u8, isize, usize, isize, u32) {
    (
        u8::from(p.pinned.is_none()),
        -isize::try_from(p.duration).unwrap_or(isize::MIN),
        p.range.size,
        -isize::try_from(p.conflict_degree).unwrap_or(isize::MIN),
        p.activity.raw(),
    )
}

/// How many other occurrences each occurrence shares a teacher or an audience with.
///
/// `SOLVER.md` §5's *"shares teachers/groups with many others"*. Computed from the section
/// facts, so it is a property of the demand rather than of any arrangement, and it uses the
/// same masks `student-clash` reads — `clash_mask` against `audience_mask` — rather than a
/// second notion of "shares students".
///
/// Quadratic in the sections, not in the occurrences: a 500-occurrence school has far fewer
/// sections, and every occurrence of a section has the same degree. Measured per section and
/// then multiplied out.
fn conflict_degrees(problem: &Problem) -> Vec<usize> {
    let occurrences_per_section: Vec<usize> = problem
        .sections
        .iter()
        .enumerate()
        .map(|(at, _)| {
            problem
                .activities
                .iter()
                .filter(|fact| fact.section.get() == at)
                .count()
        })
        .collect();

    let section_degrees: Vec<usize> = problem
        .sections
        .iter()
        .enumerate()
        .map(|(at, mine)| {
            problem
                .sections
                .iter()
                .enumerate()
                .filter(|(other, theirs)| {
                    *other != at
                        && (theirs.teachers.iter().any(|t| mine.teachers.contains(t))
                            || mine.clash_mask.intersects(&theirs.audience_mask)
                            || theirs.clash_mask.intersects(&mine.audience_mask))
                })
                .map(|(other, _)| occurrences_per_section.get(other).copied().unwrap_or(0))
                .sum()
        })
        .collect();

    problem
        .activities
        .iter()
        .map(|fact| {
            section_degrees
                .get(fact.section.get())
                .copied()
                .unwrap_or_default()
        })
        .collect()
}
