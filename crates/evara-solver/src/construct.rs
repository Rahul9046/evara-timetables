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
    ActivityIx, Assignment, CompiledConstraint, MovableSet, Placement, Problem, SolveStatistics,
    Termination, UnplacedCause, repair::frozen_only,
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
    /// In activity-index order, so a report is deterministic. For a **repair** this covers the
    /// movable set only — a frozen occurrence that was unplaced in the baseline is not this
    /// run's failure and is not reported as one.
    pub(crate) unplaced: Vec<(ActivityIx, UnplacedCause)>,
    /// How many frozen occurrences were placed when the run started.
    ///
    /// Zero for a generation. For a repair it is compared with the figure at the end, which is
    /// the scope guarantee as an assertion rather than a promise.
    pub(crate) frozen_placed_at_start: usize,
    /// How many of the attempted occurrences ended placed.
    pub(crate) attempted_placed: usize,
    /// How many occurrences the run was allowed to attempt.
    pub(crate) attempted_total: usize,
}

/// What a repair run may move, and what it must start from.
///
/// `baseline` is the arrangement as stored; the working arrangement is derived from it by
/// [`frozen_only`] and is **not** the baseline itself. See `M9B-REPAIR-PROPOSAL.md` §2.8 for why
/// starting from the full baseline is wrong three separate ways.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Repair<'a> {
    /// The arrangement to preserve outside the scope, and to prefer inside it.
    pub(crate) baseline: &'a Assignment,
    /// The occurrences the run may place. Everything else is frozen.
    pub(crate) movable: &'a MovableSet,
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
    /// Where it is in the baseline, for a repair run.
    ///
    /// Tried **first**, before the seeded walk, so an occurrence whose placement is still legal
    /// does not move — `M9B-REPAIR-PROPOSAL.md` §2.3. `None` for a generation, and for a repair
    /// of an occurrence the baseline left unplaced.
    ///
    /// There is deliberately no `frozen` flag beside it: a frozen occurrence never reaches
    /// `Prepared`, because [`prepare`] filters the order to the movable set.
    baseline: Option<Placement>,
}

/// What stays the same across every pass, bundled so [`run_pass`] takes five arguments.
struct Pass<'a, 'p> {
    problem: &'p Problem,
    config: &'a SolveConfig,
    cancel: &'a dyn Cancel,
    progress: &'a dyn ProgressSink,
    /// The problem's occurrence count, for the progress sink's denominator.
    total: usize,
}

/// What one retry pass established.
struct PassOutcome<'a> {
    /// Occurrences to try again next pass.
    still_pending: Vec<&'a Prepared>,
    /// The most recent refusal of each, and `NotReached` for a tail a stop never reached.
    refusals: Vec<(ActivityIx, UnplacedCause)>,
    /// How many were placed, which is what decides whether another pass is worth running.
    placed: usize,
    /// Why the pass stopped early, where it did.
    stopped: Option<Termination>,
}

/// One sweep over the pending occurrences.
///
/// Extracted from [`construct`] so each stays readable; the behaviour is M6's unchanged, and the
/// repair's only contribution is that `queue` was filtered to the movable set by [`prepare`].
fn run_pass<'a, 'p>(
    ctx: &Pass<'_, 'p>,
    cache: &mut RunCache,
    scorer: &mut IncrementalScorer<'p>,
    statistics: &mut SolveStatistics,
    queue: &[&'a Prepared],
) -> PassOutcome<'a> {
    let mut out = PassOutcome {
        still_pending: Vec::new(),
        refusals: Vec::new(),
        placed: 0,
        stopped: None,
    };

    for (at, prepared) in queue.iter().copied().enumerate() {
        if ctx.cancel.is_cancelled() {
            out.stopped = Some(Termination::Cancelled);
        } else if statistics.construction_steps >= ctx.config.limits.max_construction_steps {
            out.stopped = Some(Termination::StepLimitReached);
        } else {
            statistics.construction_steps += 1;
            match place_one(ctx.problem, cache, scorer, statistics, ctx.config, prepared) {
                Attempt::Placed => {
                    statistics.placements_committed += 1;
                    out.placed += 1;
                }
                Attempt::Refused(cause) => {
                    out.refusals.push((prepared.activity, cause));
                    out.still_pending.push(prepared);
                }
                Attempt::OutOfBudget => out.stopped = Some(Termination::CandidateLimitReached),
            }
            ctx.progress.placed(
                scorer.placed_count(),
                ctx.total,
                statistics.construction_steps,
            );
        }

        if out.stopped.is_some() {
            // This occurrence and **every one after it**. Nothing was established about any of
            // them, and a report that silently omitted the tail would be the engine bounding
            // its coverage without saying so.
            out.refusals.extend(not_reached(&queue[at..]));
            out.still_pending.extend(queue[at..].iter().copied());
            break;
        }
    }

    out
}

/// The accounting every run must satisfy, whatever it reached.
///
/// M6's invariant -- `unplaced.len() + placed_count() == total` -- is **false** for a repair
/// whose frozen set contains an unplaced occurrence, which is an entirely ordinary state: an
/// out-of-scope lesson the source timetable never scheduled is neither placed nor this run's
/// failure. So the accounting is asserted over what the run was allowed to touch, and the scope
/// guarantee is asserted beside it.
///
/// The full placement-by-placement comparison is
/// `evara_core::scheduling::repair::frozen_differences`, which the caller runs and which is the
/// authority; this is the cheap count that catches a gross failure here, at the source.
fn assert_accounting(
    unplaced: usize,
    attempted_placed: usize,
    attempted_total: usize,
    placed_now: usize,
    frozen_placed_at_start: usize,
) {
    debug_assert_eq!(
        unplaced + attempted_placed,
        attempted_total,
        "every attempted occurrence is either placed or reported as unplaced"
    );
    debug_assert_eq!(
        placed_now - attempted_placed,
        frozen_placed_at_start,
        "a repair placed or dropped a frozen occurrence"
    );
}

/// The scorer a run begins with.
///
/// `Some(working)` is a repair, and a `None` from [`IncrementalScorer::of`] is a **refusal**
/// rather than a fall back to `empty`: a repair that quietly started from an empty arrangement
/// would regenerate the whole timetable while reporting itself as a repair, which is the most
/// destructive thing this module could do.
fn starting_scorer<'p>(
    problem: &'p Problem,
    constraints: &[CompiledConstraint],
    working: Option<&Assignment>,
) -> Option<IncrementalScorer<'p>> {
    match working {
        Some(start) => IncrementalScorer::of(problem, start, constraints),
        None => Some(IncrementalScorer::empty(problem, constraints)),
    }
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
    repair: Option<&Repair<'_>>,
) -> Option<Construction<'p>> {
    let mut cache = RunCache::default();
    let total = problem.activities.len();

    // A repair starts from the FROZEN placements alone. Not from the baseline: every movable
    // occurrence must be absent, or each one would be reconsidered against the stale positions
    // of the others, and a pre-existing movable-to-movable clash would refuse both of them —
    // `PlacementVerdict::allowed` is false whenever the result *carries* a hard violation
    // naming the occurrence, not only when it adds one. `M9B-REPAIR-PROPOSAL.md` §2.8.
    let working = repair.map(|r| frozen_only(r.baseline, r.movable));
    let mut scorer = starting_scorer(problem, constraints, working.as_ref())?;
    let frozen_placed_at_start = scorer.placed_count();
    let mut statistics = SolveStatistics::default();

    let ctx = Pass {
        problem,
        config,
        cancel,
        progress,
        total,
    };

    let order = prepare(problem, &mut cache, config, repair);
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
        let pass = run_pass(&ctx, &mut cache, &mut scorer, &mut statistics, &queue);

        refusals.extend(pass.refusals);
        pending = pass.still_pending;
        if let Some(reason) = pass.stopped {
            termination = reason;
            break;
        }
        // A pass that placed nothing cannot be improved on by repeating it: the arrangement is
        // unchanged and the candidate order is a pure function of the seed, so the next pass
        // would make the identical decisions. This is the fixed point that ends the loop.
        if pass.placed == 0 {
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

    // How many occurrences this run was allowed to attempt, and how many of those it placed.
    // For a generation that is the whole problem; for a repair it is the movable set.
    let attempted_total = repair.map_or(total, |r| r.movable.len());
    let attempted_placed = scorer.placed_count().saturating_sub(frozen_placed_at_start);
    assert_accounting(
        unplaced.len(),
        attempted_placed,
        attempted_total,
        scorer.placed_count(),
        frozen_placed_at_start,
    );

    if scorer.placed_count() == total {
        termination = Termination::Complete;
    }

    Some(Construction {
        scorer,
        termination,
        statistics,
        unplaced,
        frozen_placed_at_start,
        attempted_placed,
        attempted_total,
    })
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
    // A pinned occurrence is placed where it is pinned, and nothing else is tried. Since M9A
    // `Problem::pinned` is populated by the caller that opened a timetable, from
    // `timetable_entry.is_locked`, so this branch is reachable — and `activity-fixed` is the
    // registered rule that judges the same thing, so a pin honoured here and a pin checked
    // there cannot disagree. The probe is what makes an *impossible* pin reported rather than
    // forced (`SOLVER.md` §5).
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

    // The baseline placement, tried before the seeded walk. If it is still legal the
    // first-feasible rule takes it and the occurrence does not move -- which is the whole of
    // "prefer existing placements", with no score change and no new rule
    // (`M9B-REPAIR-PROPOSAL.md` §2.3).
    //
    // Deliberately NOT budget-checked, exactly as the pinned branch above is not: this is the
    // one candidate the caller already chose, and refusing it for budget would displace a
    // lesson that was legally where it already was. The cost is one evaluation per in-scope
    // occurrence, which is why a repair's counters are not comparable to a generation's.
    if let Some(at) = prepared.baseline {
        statistics.candidate_evaluations += 1;
        let verdict = scorer.probe(prepared.activity, at);
        if verdict.allowed {
            scorer.place(prepared.activity, at);
            return Attempt::Placed;
        }
        statistics.candidates_rejected += 1;
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
            // A pin cannot be the reason a *candidate* was refused: the pinned branch tries
            // the pin and nothing else, so a violation of this rule means the pin itself is
            // impossible — which is a property of the arrangement around it, not of the
            // occurrence. `BlockedByPlacements` is already the default and is the honest
            // answer; naming an impossibility here would overclaim.
            ConstraintType::ActivityFixed | ConstraintType::ActivityScheduled => {}
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
fn prepare(
    problem: &Problem,
    cache: &mut RunCache,
    config: &SolveConfig,
    repair: Option<&Repair<'_>>,
) -> Vec<Prepared> {
    let degrees = conflict_degrees(problem);
    let mut prepared: Vec<Prepared> = problem
        .activity_indices()
        // A frozen occurrence is **absent** from the order rather than sorted to the front of
        // it. It is already in the starting arrangement, every rule can see it, and it is never
        // attempted -- so there is nothing for the walk to do with it. `constrainedness`' first
        // key is therefore inert during a repair, because every pinned occurrence is frozen.
        .filter(|activity| repair.is_none_or(|r| r.movable.contains(*activity)))
        .map(|activity| Prepared {
            activity,
            range: value_range(problem, cache, activity),
            conflict_degree: degrees.get(activity.get()).copied().unwrap_or_default(),
            duration: problem
                .activities
                .get(activity.get())
                .map_or(1, |fact| fact.duration.as_usize()),
            pinned: problem.pinned.get(activity.get()).copied().flatten(),
            baseline: repair.and_then(|r| r.baseline.placement(activity)),
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
