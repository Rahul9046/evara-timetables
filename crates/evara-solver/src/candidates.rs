//! Where one occurrence could go: the value range, enumerated rather than guessed at.
//!
//! # Every candidate is legal by construction for four of the seven rules
//!
//! A candidate is a `(start slot, room)` pair, and the generator builds only pairs that
//! already satisfy:
//!
//! | Rule | Why it cannot be broken by a candidate |
//! | --- | --- |
//! | `duration-contiguous` | the slots come from [`runs_on`], which **is** ADR 0033 §5's measure |
//! | `no-teaching-in-period` | a run is made of `TEACHING` periods, so its first slot is one |
//! | `room-type-required` | the room comes from `room_candidates`, built by [`RoomFact::is_candidate_for`] |
//! | `entity-unavailable` | every slot of the run is checked against the index for the teachers, the audience and the room |
//!
//! The three clash rules are the ones that cannot be pre-filtered, because they are about
//! *other* placements and the generator knows nothing about them. Those are the constraint
//! engine's to answer, per candidate, through
//! [`IncrementalScorer::probe`](evara_constraints::IncrementalScorer).
//!
//! **This is a pre-filter, not a second implementation.** Each row above is a *call* to the one
//! function that owns the question — `runs_on`, `is_candidate_for`,
//! `SlotAvailability::permits_placement` — and the registered rule still runs over the finished
//! timetable and over every manual edit. `CLAUDE.md` §2's most important rule is that generation
//! and the editor must not be able to disagree about legality; the way to pre-filter safely is
//! to call the same predicate, which is what makes "the generator never offers a placement the
//! editor would refuse" a property of one function rather than of two that happen to agree.
//!
//! A test asserts the consequence directly: every candidate this module yields is accepted by
//! the full evaluator when it is the only thing placed.
//!
//! # Lazy, and why that is not an optimisation detail
//!
//! A school with 500 occurrences, 40 rooms and 30 runs a cycle has 600 000 `(run, room)` pairs.
//! Materialising them all would cost more memory than the whole problem and would be wasted:
//! construction accepts the first legal candidate it meets, which for most occurrences is among
//! the first few. So [`Candidates`] holds the run list and the room list — tens of entries each
//! — and walks their product on demand.
//!
//! The run list is cached per `(bell schedule, duration)` in [`RunCache`], because every
//! occurrence of a section shares a schedule and most share a duration: the `tiny` fixture has
//! ten occurrences and two distinct keys.
//!
//! # Seeded order, and why the order is not index order
//!
//! Candidates are walked in an order derived from the seed and the occurrence's own index. The
//! alternative — index order — is worse than arbitrary: every occurrence would try Day 1 Period
//! 1 first, every teacher and room would fill from the same corner, and a greedy constructor
//! would strand the occurrences it happened to consider last. A seeded permutation spreads
//! placements across the grid for free, and is what makes `--seed` explore genuinely different
//! timetables (`SOLVER.md` §5).
//!
//! The permutation is a pure function of `(seed, activity index)`, so it is identical on every
//! machine and every run — which is the half of determinism a shuffle usually loses.

use evara_core::scheduling::{
    ActivityIx, BellScheduleIx, CycleDayIx, Placement, Problem, Rng, RoomIx, SectionFact,
    TimeslotIx, UnplacedCause, runs_on,
};

/// The runs of consecutive teaching periods, cached per `(schedule, duration)`.
///
/// A plain `Vec` of keys rather than a `HashMap`: the key count is the number of distinct
/// `(schedule, duration)` pairs a school uses, which is a handful, and nothing in the solver
/// may iterate a hash map (`CLAUDE.md`).
#[derive(Debug, Default)]
pub(crate) struct RunCache {
    entries: Vec<(BellScheduleIx, usize, Vec<Run>)>,
}

/// One legal run of slots: a day and the consecutive teaching periods on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    /// The cycle position the run is on.
    pub(crate) day: CycleDayIx,
    /// The slot the occurrence would start in.
    pub(crate) start: TimeslotIx,
    /// Every slot the run covers, first to last.
    pub(crate) slots: Vec<TimeslotIx>,
}

impl RunCache {
    /// Every legal run for an occurrence of `duration` periods in `schedule`.
    ///
    /// Computed once per key by [`runs_on`], which is ADR 0033 §5's one implementation of the
    /// measure. The order is `(day, start)`, which [`runs_on`] already guarantees per day and
    /// which the day loop extends across days.
    pub(crate) fn runs(
        &mut self,
        problem: &Problem,
        schedule: BellScheduleIx,
        duration: usize,
    ) -> &[Run] {
        if let Some(at) = self
            .entries
            .iter()
            .position(|(s, d, _)| *s == schedule && *d == duration)
        {
            return &self.entries[at].2;
        }

        let mut found = Vec::new();
        for day in 0..problem.context.cycle_days.len() {
            let day = CycleDayIx::new(u32::try_from(day).unwrap_or(u32::MAX));
            for slots in runs_on(problem, schedule, day, duration) {
                if let Some(start) = slots.first().copied() {
                    found.push(Run { day, start, slots });
                }
            }
        }
        self.entries.push((schedule, duration, found));
        &self.entries[self.entries.len() - 1].2
    }
}

/// The rooms an occurrence may be given, in the order they will be tried.
///
/// # Roomless is a candidate, and it comes last
///
/// A section with no `required_room_type` may be timetabled without a room — `Placement::room`
/// is `Option<RoomIx>` and `room-type-required` has no opinion about an unroomed lesson with no
/// requirement. But a lesson *with* a room is better than one without, so every candidate room
/// is tried first and `None` is the fallback.
///
/// Both halves matter. Without the rooms, a school's unroomed lessons would stay unroomed even
/// where rooms were free; without the fallback, an occurrence would go **unplaced** rather than
/// unroomed once the rooms filled up, which trades a lesson with no room for no lesson at all.
///
/// A section that *does* require a type gets no fallback: an unroomed lesson that needs a
/// laboratory is a hard violation, so offering `None` would be offering an illegal candidate.
#[derive(Debug, Clone)]
pub(crate) struct RoomChoices {
    rooms: Vec<RoomIx>,
    /// Whether no room at all is a legal choice for this section.
    roomless_allowed: bool,
}

impl RoomChoices {
    /// The choices for one section.
    pub(crate) fn of(section: &SectionFact) -> Self {
        Self {
            rooms: section.room_candidates.clone(),
            roomless_allowed: section.required_room_type.is_none(),
        }
    }

    /// How many choices there are, the roomless one included.
    pub(crate) fn len(&self) -> usize {
        self.rooms.len() + usize::from(self.roomless_allowed)
    }

    /// The `n`th choice in try order: every room, then the roomless one where it is allowed.
    ///
    /// `None` is the roomless choice, which is why there is no outer `Option` to say "no such
    /// choice": `n` is always below [`RoomChoices::len`], because every caller draws it from
    /// `0..len()` or from a permutation of that range.
    ///
    /// That precondition is what makes the roomless answer safe rather than a silent hole. A
    /// section that requires a room type has `roomless_allowed == false`, so `len()` is exactly
    /// the room count and `n < len()` guarantees a room — the one case where offering `None`
    /// would be offering an illegal candidate is the one case it cannot be reached.
    pub(crate) fn at(&self, n: usize) -> Option<RoomIx> {
        debug_assert!(n < self.len(), "a room choice index is drawn from 0..len()");
        self.rooms.get(n).copied()
    }
}

/// Every placement one occurrence could legally take, walked in a seeded order.
///
/// Legal here means *the four rules of this module's header*, not the three clash rules — see
/// there. Yielded one at a time; nothing materialises the product.
#[derive(Debug)]
pub(crate) struct Candidates {
    /// A seeded permutation of the run list.
    runs: Vec<usize>,
    /// A seeded permutation of the room choices.
    rooms: Vec<usize>,
    /// How far through the product the walk has got.
    at: usize,
}

impl Candidates {
    /// The candidate walk for one occurrence.
    ///
    /// Both permutations are drawn from an [`Rng`] seeded on `(seed, activity)`, so the order is
    /// a pure function of the request — and two occurrences of one section walk *different*
    /// orders, which is what stops four periods of the same lesson from queueing for the same
    /// slot.
    pub(crate) fn new(
        seed: u64,
        activity: ActivityIx,
        run_count: usize,
        room_count: usize,
    ) -> Self {
        let mut rng = Rng::new(seed ^ (u64::from(activity.raw()).wrapping_mul(0x9E37_79B9)));
        Self {
            runs: permutation(run_count, &mut rng),
            rooms: permutation(room_count, &mut rng),
            at: 0,
        }
    }

    /// How many candidates the walk will yield in total.
    pub(crate) fn total(&self) -> usize {
        self.runs.len() * self.rooms.len()
    }

    /// The next `(run, room choice)` index pair, or `None` when the walk is done.
    ///
    /// Walks room-major within each run: every room of the first run, then the second run. A
    /// construction solver wants to stay on a slot it has already proved available and vary the
    /// room, rather than the other way round — changing the slot invalidates the availability
    /// work, changing the room does not.
    pub(crate) fn next(&mut self) -> Option<(usize, usize)> {
        let rooms = self.rooms.len();
        if rooms == 0 || self.at >= self.total() {
            return None;
        }
        let pair = (self.runs[self.at / rooms], self.rooms[self.at % rooms]);
        self.at += 1;
        Some(pair)
    }
}

/// A seeded permutation of `0..n`.
///
/// Fisher–Yates over [`Rng`], which draws exactly `n - 1` times — a fixed count, so the
/// generator's position after the shuffle does not depend on the values drawn. A rejection-based
/// shuffle would make the next caller's draws depend on this one's luck, which is a subtle way
/// to lose reproducibility when a problem's size changes.
fn permutation(n: usize, rng: &mut Rng) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i + 1);
        order.swap(i, j);
    }
    order
}

/// Whether every slot of a run is one all the occurrence's entities may be placed in.
///
/// Reads [`SlotAvailability::permits_placement`](evara_core::scheduling::SlotAvailability) —
/// the one implementation of *"may a lesson go here?"*, which admits everything except
/// `Unavailable`, so a `DISCOURAGED` slot is legal and merely worse. Three entity kinds, because
/// those are the three an occurrence occupies: its teachers, its audience, and its room.
///
/// The whole run is tested, not only the start: a teacher free for the first half of a double
/// and busy for the second cannot teach the double.
pub(crate) fn run_is_available(
    problem: &Problem,
    section: &SectionFact,
    slots: &[TimeslotIx],
    room: Option<RoomIx>,
) -> bool {
    use evara_core::scheduling::AvailabilityEntity;

    for slot in slots {
        for teacher in &section.teachers {
            if !problem
                .unavailable
                .status(AvailabilityEntity::Teacher { teacher: *teacher }, *slot)
                .permits_placement()
            {
                return false;
            }
        }
        for group in &section.audience {
            if !problem
                .unavailable
                .status(AvailabilityEntity::Group { group: *group }, *slot)
                .permits_placement()
            {
                return false;
            }
        }
        if let Some(room) = room
            && !problem
                .unavailable
                .status(AvailabilityEntity::Room { room }, *slot)
                .permits_placement()
        {
            return false;
        }
    }
    true
}

/// What an occurrence's value range looks like before anything is placed.
///
/// Computed once per occurrence, at the start of a solve. Two uses: the constrainedness order
/// (`SOLVER.md` §5 Phase A ranks by *"few feasible (timeslot, room) pairs"*), and the honest
/// cause for an occurrence that is unplaceable whatever else happens — which is why the
/// emptiness reasons are kept apart rather than collapsed into "no candidates".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueRange {
    /// How many `(run, room)` pairs pass the four pre-filtered rules.
    pub(crate) size: usize,
    /// Why it is empty, where it is.
    ///
    /// `None` means there is at least one candidate. Each cause is a property of the
    /// **problem**, so [`UnplacedCause::is_impossible`] holds for all of them — this is the one
    /// place a solver may say *"no timetable can place this"*, and it says it from the inputs
    /// rather than from a failed search.
    pub(crate) empty_because: Option<UnplacedCause>,
}

/// Measures one occurrence's value range, over the empty timetable.
///
/// Counts rather than collects. The cost is `runs × rooms` availability checks per occurrence,
/// paid once per solve, which is what buys both the ordering and an honest refusal.
pub(crate) fn value_range(
    problem: &Problem,
    cache: &mut RunCache,
    activity: ActivityIx,
) -> ValueRange {
    let Some(fact) = problem.activities.get(activity.get()) else {
        return ValueRange {
            size: 0,
            empty_because: Some(UnplacedCause::NoResolvedBellSchedule),
        };
    };
    let Some(section) = problem.sections.get(fact.section.get()) else {
        return ValueRange {
            size: 0,
            empty_because: Some(UnplacedCause::NoResolvedBellSchedule),
        };
    };
    // ADR 0033 §3: anything other than a single resolved schedule is a hard pre-flight error,
    // and an occurrence in that state has no value range at all rather than a small one.
    let Some(schedule) = section.bell_schedule.schedule() else {
        return ValueRange {
            size: 0,
            empty_because: Some(UnplacedCause::NoResolvedBellSchedule),
        };
    };

    let choices = RoomChoices::of(section);
    if choices.len() == 0 {
        // The section requires a kind of room and no room is a candidate for it. M3's
        // `no-room-of-required-type` says the same thing before a solve.
        return ValueRange {
            size: 0,
            empty_because: Some(UnplacedCause::NoCandidateRoom),
        };
    }

    let runs = cache
        .runs(problem, schedule, fact.duration.as_usize())
        .to_vec();
    if runs.is_empty() {
        // Arithmetic: no run of that many consecutive teaching periods exists anywhere in the
        // occurrence's own schedule. ADR 0033 §5, and M3's `duration-exceeds-longest-run`.
        return ValueRange {
            size: 0,
            empty_because: Some(UnplacedCause::NoContiguousRun),
        };
    }

    let mut size = 0;
    for run in &runs {
        for n in 0..choices.len() {
            if run_is_available(problem, section, &run.slots, choices.at(n)) {
                size += 1;
            }
        }
    }

    ValueRange {
        size,
        // Runs exist and rooms exist, so what emptied the range was availability.
        empty_because: (size == 0).then_some(UnplacedCause::AllCandidatesUnavailable),
    }
}

/// The placement one `(run, room choice)` pair describes.
pub(crate) fn placement_of(run: &Run, room: Option<RoomIx>) -> Placement {
    Placement {
        timeslot: run.start,
        room,
    }
}
