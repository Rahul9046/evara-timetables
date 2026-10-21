//! Which bell schedule a class section schedules against, and what runs of periods exist in
//! it.
//!
//! # One implementation, several callers
//!
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §2 defines a compatibility relation and §3 defines the resolution built on it. This module
//! is the **only** implementation of both, and that is the point of it existing separately
//! from either caller:
//!
//! - **M3** validates that a compiled problem's declared resolutions agree with it, and
//!   produces the hard pre-flight findings when a section resolves to none or to several.
//! - **M4**'s loader calls it to *produce* the resolution it records on each
//!   [`SectionFact`](super::SectionFact).
//! - **M5**'s `duration-contiguous` reads [`runs_on`] for the runs a multi-period
//!   occurrence may occupy.
//!
//! A second copy in the loader would be the failure `CLAUDE.md` §2 names: generation and
//! validation disagreeing about what is legal. It is the same discipline
//! [`RoomFact::is_candidate_for`](super::RoomFact::is_candidate_for) applies to the room
//! candidate domain.
//!
//! # `G` is derived from the grid, never from what exists
//!
//! ADR 0033 §1 defines `G` as *the distinct `period_structure` rows that have materialised
//! `timeslot` rows in `(cycle, term)`*. [`SchedulingContext::bell_schedules`](super::SchedulingContext)
//! holds every schedule the **school has defined**, which is a different set: a school may
//! define a sixth-form timetable and never build a grid from it.
//!
//! Reading the defined set would report an ambiguity at a school that is correctly
//! configured, and ADR 0033 test 8 exists to forbid exactly that. So [`MaterialisedGrid::of`]
//! derives `G` from [`Problem::timeslots`](super::Problem), and nothing here consults the
//! defined list except to read a schedule's campus and name.
//!
//! # Nothing is guessed
//!
//! There is no fallback and no default. A section resolves to exactly one schedule, or the
//! answer is an error naming the cause. ADR 0033's *"Alternatives rejected"* records why
//! picking `is_default` cannot work: the column is unique per *campus bucket*, so a school
//! with a school-wide default and a campus default has two and "the default" is not a value.

use super::context::{
    BellScheduleFact, BellScheduleResolution, NoBellScheduleCause, SchedulingContext,
};
use super::index::{BellScheduleIx, CampusIx, CycleDayIx, TimeslotIx};
use super::problem::Problem;

/// The bell schedules that have slots in this grid: `G` of ADR 0033 §1.
///
/// Built once per solve and then asked about each section, because deriving it is a scan of
/// every timeslot and a school has one grid and many sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialisedGrid {
    /// The schedules with at least one slot, in index order and deduplicated.
    schedules: Vec<BellScheduleIx>,
}

impl MaterialisedGrid {
    /// Derives `G` from the problem's materialised slots.
    ///
    /// **Not** from `context.bell_schedules` — see this module's header for why that would be
    /// wrong.
    #[must_use]
    pub fn of(problem: &Problem) -> Self {
        Self::from_schedules(
            problem
                .timeslots
                .iter()
                .map(|slot| slot.bell_schedule)
                .collect(),
        )
    }

    /// The schedules in `G`, in index order.
    ///
    /// Index order is definition order, which makes a candidate list deterministic — and a
    /// diagnostic that names its candidates has to name them in a stable order or two runs
    /// produce two different messages.
    #[must_use]
    pub fn schedules(&self) -> &[BellScheduleIx] {
        &self.schedules
    }

    /// Whether no grid has been materialised at all.
    ///
    /// The condition of [`NoBellScheduleCause::GridEmpty`], and the one finding that is about
    /// the solve rather than about a section.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty()
    }

    /// How many schedules the grid draws from.
    #[must_use]
    pub fn len(&self) -> usize {
        self.schedules.len()
    }

    /// Builds a grid from a known schedule set, sorting and deduplicating it.
    ///
    /// Crate-visible rather than public: the supported way to obtain a grid is
    /// [`MaterialisedGrid::of`], which derives it from the slots that actually exist. This
    /// exists so a unit test can exercise the compatibility relation without constructing a
    /// whole grid of timeslots, and the sort is here rather than at the call site so a test
    /// cannot accidentally assert an order the real constructor would not produce.
    pub(crate) fn from_schedules(mut schedules: Vec<BellScheduleIx>) -> Self {
        schedules.sort_unstable();
        schedules.dedup();
        Self { schedules }
    }

    /// Resolves the bell schedule for a section at `section_campus`.
    ///
    /// `section_campus` of `None` means the section is **not campus-constrained**, which is the
    /// ordinary state at a one-campus school and is not a missing value.
    ///
    /// Reads only the section's campus, so every occurrence of a section resolves identically
    /// and the question is asked once per section rather than once per activity.
    #[must_use]
    pub fn resolve(
        &self,
        context: &SchedulingContext,
        section_campus: Option<CampusIx>,
    ) -> BellScheduleResolution {
        if self.is_empty() {
            return BellScheduleResolution::None {
                cause: NoBellScheduleCause::GridEmpty,
            };
        }

        let candidates: Vec<BellScheduleIx> = self
            .schedules
            .iter()
            .copied()
            .filter(|ix| {
                context
                    .bell_schedules
                    .get(ix.get())
                    .is_some_and(|schedule| is_compatible(context, schedule, section_campus))
            })
            .collect();

        match candidates.len() {
            1 => BellScheduleResolution::Resolved {
                schedule: candidates[0],
            },
            0 => BellScheduleResolution::None {
                cause: no_schedule_cause(section_campus),
            },
            _ => BellScheduleResolution::Ambiguous { candidates },
        }
    }
}

/// The teaching slots of one schedule, in `(cycle day, period ordinal)` order.
///
/// ADR 0033 §4: an activity's candidate timeslots are the rows of its resolved schedule,
/// restricted to `is_teaching = 1`. That restriction is a **pre-filter of a constraint that
/// still exists** — `no-teaching-in-period` stays registered because manual editing can name
/// a non-teaching slot — and both read the one stored `timeslot.is_teaching`.
///
/// A free function rather than a method on [`MaterialisedGrid`], because it needs the schedule
/// and the problem and nothing the grid holds.
#[must_use]
pub fn teaching_slots(problem: &Problem, schedule: BellScheduleIx) -> Vec<TimeslotIx> {
    let mut slots = Vec::new();
    for day in 0..problem.context.cycle_days.len() {
        let day = CycleDayIx::new(u32::try_from(day).unwrap_or(u32::MAX));
        slots.extend(
            problem
                .slots_of(schedule, day)
                .into_iter()
                .filter(|ix| problem.timeslots[ix.get()].is_teaching),
        );
    }
    slots
}

/// Every run of `duration` ordinal-consecutive teaching periods of `schedule` on `day`.
///
/// # The measure, exactly
///
/// ADR 0033 §5: a `duration_periods = d` occurrence needs **`d` ordinal-consecutive
/// `TEACHING` periods of its own resolved schedule, on one cycle day.** A run is broken by any
/// non-teaching period between two teaching ones; a mere wall-clock gap between consecutive
/// teaching periods does **not** break it, because a gap between bells is a gap between bells
/// and not a non-teaching period.
///
/// A run never spans two schedules, even where their minutes are adjacent — two schedules are
/// two grids, and an occurrence lives in one.
///
/// Returns the runs in start order, each listed first slot to last. Empty when `duration` is
/// zero or no run is long enough.
///
/// This is the one implementation of the measure. M5's `duration-contiguous` reads it rather
/// than restating it.
#[must_use]
pub fn runs_on(
    problem: &Problem,
    schedule: BellScheduleIx,
    day: CycleDayIx,
    duration: usize,
) -> Vec<Vec<TimeslotIx>> {
    if duration == 0 {
        return Vec::new();
    }
    // The day's slots in ordinal order, teaching and non-teaching alike: a non-teaching period
    // between two teaching ones has to be *seen* to break the run, so filtering it out first
    // would be the exact mistake `Problem::slots_of` warns about.
    let ordered = problem.slots_of(schedule, day);

    let mut runs = Vec::new();
    let mut current: Vec<TimeslotIx> = Vec::new();
    for slot in ordered {
        if problem.timeslots[slot.get()].is_teaching {
            current.push(slot);
        } else {
            current.clear();
        }
        if current.len() >= duration {
            runs.push(current[current.len() - duration..].to_vec());
        }
    }
    runs
}

/// The longest unbroken run of teaching periods anywhere in one schedule.
///
/// The figure `duration-exceeds-longest-run` compares against. Measured **per schedule**,
/// because a run never spans two — which is why this takes a schedule rather than scanning the
/// whole grid, and why a longer run elsewhere in the school is irrelevant (ADR 0033 test 10).
///
/// Distinct from `evara_core::lessons::CycleCapacity::longest_teaching_run`, which takes the
/// **maximum across all of a school's schedules** and is therefore optimistic. That one can
/// only under-report impossibility, which keeps it honest for the screen it serves; this one
/// is per-schedule and is what a sound refusal needs.
#[must_use]
pub fn longest_teaching_run(problem: &Problem, schedule: BellScheduleIx) -> usize {
    let mut longest = 0;
    for day in 0..problem.context.cycle_days.len() {
        let day = CycleDayIx::new(u32::try_from(day).unwrap_or(u32::MAX));
        let mut current = 0;
        for slot in problem.slots_of(schedule, day) {
            if problem.timeslots[slot.get()].is_teaching {
                current += 1;
                longest = longest.max(current);
            } else {
                current = 0;
            }
        }
    }
    longest
}

/// Whether a schedule may serve a section at `section_campus`.
///
/// ADR 0033 §2's relation, and nothing else. A schedule is compatible when **any** of:
///
/// 1. `s.campus_id IS NULL` — a school-wide schedule serves every campus, by its own
///    definition.
/// 2. `s.campus_id = cs.campus_id` — the section is at that campus.
/// 3. `cs.campus_id IS NULL` **and the school has exactly one campus** and `s.campus_id` is
///    that campus.
///
/// # Clause 3 is not a fallback
///
/// A school with one campus has one place. *"This section is not campus-constrained"* and
/// *"this section is at the only campus there is"* are the same statement about such a school,
/// and the count of campuses is a stored fact rather than an inference. Without clause 3 the
/// commonest configuration in the product — one campus, one bell schedule scoped to it,
/// sections that leave `campus_id` null because there is nothing to choose — would resolve to
/// nothing.
///
/// **It does not generalise.** At a school with two or more campuses, a section naming no
/// campus is compatible only with a school-wide schedule, because a campus-scoped schedule is
/// a statement about one campus and the section is not at a known one.
#[must_use]
pub fn is_compatible(
    context: &SchedulingContext,
    schedule: &BellScheduleFact,
    section_campus: Option<CampusIx>,
) -> bool {
    match (schedule.campus, section_campus) {
        // Clause 1.
        (None, _) => true,
        // Clause 2.
        (Some(theirs), Some(ours)) => theirs == ours,
        // Clause 3, and only at a school with exactly one campus.
        (Some(theirs), None) => only_campus(context) == Some(theirs),
    }
}

/// The school's single campus, or `None` where it has none or several.
///
/// The precondition of clause 3, expressed as a value so no caller has to re-derive it — and
/// so "has exactly one campus" cannot drift into "has at least one".
#[must_use]
pub fn only_campus(context: &SchedulingContext) -> Option<CampusIx> {
    match context.campuses.len() {
        1 => Some(CampusIx::new(0)),
        _ => None,
    }
}

/// Which of ADR 0033 §3's three causes applies when no schedule is compatible.
///
/// Three causes, three remedies. They are kept apart because a message that merged them would
/// be one the user cannot act on.
const fn no_schedule_cause(section_campus: Option<CampusIx>) -> NoBellScheduleCause {
    match section_campus {
        // The section names a campus, and the grid holds neither a schedule for it nor a
        // school-wide one.
        Some(_) => NoBellScheduleCause::NoScheduleForCampus,
        // The section names no campus. At a one-campus school clause 3 would have matched any
        // campus-scoped schedule and clause 1 any school-wide one, so reaching here means the
        // school has several campuses and the grid holds no school-wide schedule.
        //
        // A school with *zero* campuses cannot reach here either: every schedule of such a
        // school must be school-wide, because a campus-scoped one would carry a campus index
        // the problem does not hold, which `Problem::integrity` reports as dangling.
        None => NoBellScheduleCause::SectionHasNoCampus,
    }
}

#[cfg(test)]
mod tests;
