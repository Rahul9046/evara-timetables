//! Where an occurrence goes: the two planning variables, what a placement covers, and a
//! whole timetable's worth of them.
//!
//! # Two planning variables, and only two
//!
//! A [`Placement`] is a **timeslot** and an optional **room**. Teacher is not a variable:
//! a section's staffing is fixed input (D7), which is also why the solver cannot pick an
//! ineligible teacher — it picks none. `SOLVER.md` §2 notes that a third variable for
//! interchangeable staff is additive, and nothing here blocks adding it.
//!
//! # A placement names the *start*, and what it covers is derived
//!
//! A `duration_periods = 3` occurrence occupies three slots, and
//! [`Placement`] stores one. That is deliberate: the three must be
//! ordinal-consecutive teaching periods of one bell schedule on one cycle day
//! ([ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §5), so storing all three would let an assignment express a run that is not a run.
//! Storing the start makes the illegal state unrepresentable and leaves exactly one place —
//! `duration-contiguous`, in M5 — to decide which runs are legal.
//!
//! [`PlacementSpan`] is the *resolved* form: the slots a placement was found to cover, and
//! the wall-clock interval they add up to. **This module does not resolve one.** Choosing
//! the slots means answering four separate questions — are the periods ordinal-consecutive,
//! is the run long enough, does a non-teaching period interrupt it, does it cross a boundary
//! — and every one of those is a constraint with its own evaluator, its own test and its own
//! explanation. M2 provides the representation; M5 provides the answers.

use serde::{Deserialize, Serialize};

use super::clock::TimeInterval;
use super::index::{ActivityIx, CycleDayIx, RoomIx, TimeslotIx};

/// Where one occurrence starts, and in which room.
///
/// The solver's two planning variables, and the unit a move operates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// The slot the occurrence **begins** in.
    ///
    /// A multi-period occurrence continues into later slots of the same bell schedule and
    /// cycle day; see this module's header for why only the first is stored.
    pub timeslot: TimeslotIx,
    /// The room, where one was assigned.
    ///
    /// `None` is legal and means no room is booked — the state for a section whose course
    /// requires no room type and which was given none. A lesson with no room is worse than a
    /// lesson in a room of unverified size, which is why ADR 0034 §4 refuses to withhold
    /// rooms when attendance is unknown.
    pub room: Option<RoomIx>,
}

impl Placement {
    /// A placement in `timeslot` with no room.
    #[must_use]
    pub const fn unroomed(timeslot: TimeslotIx) -> Self {
        Self {
            timeslot,
            room: None,
        }
    }

    /// A placement in `timeslot` and `room`.
    #[must_use]
    pub const fn in_room(timeslot: TimeslotIx, room: RoomIx) -> Self {
        Self {
            timeslot,
            room: Some(room),
        }
    }
}

/// What a placement was found to cover, once its run of slots is resolved.
///
/// # Not resolved by this crate
///
/// Constructed from a slot list a caller has already decided on —
/// [`PlacementSpan::over`] — and it validates only that the list is non-empty. It does
/// **not** check that the slots are consecutive, that they are all teaching periods, that
/// they belong to one bell schedule, or that there are as many of them as the activity's
/// duration asks for. Those are `duration-contiguous` and `no-teaching-in-period` in M5,
/// and a span built from a wrong list is a wrong span rather than an error here.
///
/// The one piece of arithmetic it does own is the interval:
/// `[first period's starts_at, last period's ends_at)`, per
/// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §2.3. That figure is the clash key for every
/// shared resource (ADR 0032 rule 2), and having exactly one definition of it is the point
/// of the type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementSpan {
    /// The cycle position the occurrence runs on.
    ///
    /// One day: a run never spans two positions. `cycle_day.ordinal` is a day's only
    /// scheduling identity, and nothing here reads a weekday.
    pub cycle_day: CycleDayIx,
    /// Every slot the occurrence occupies, in the order given, first to last.
    pub slots: Vec<TimeslotIx>,
    /// The wall-clock span, from the first slot's start to the last slot's end.
    ///
    /// **Spans any gap in the middle**, which is correct for a room: a double period either
    /// side of a break does not release the room during it.
    pub interval: TimeInterval,
    /// The room booked for the whole span, where one was.
    pub room: Option<RoomIx>,
}

impl PlacementSpan {
    /// Builds a span over slots a caller has already chosen.
    ///
    /// `slots` is first-to-last and `intervals` is the matching wall-clock span of each, so
    /// that this function needs no access to a [`Problem`](super::Problem) and stays a pure
    /// fold. Returns `None` if the lists are empty or of different lengths, which is a
    /// caller bug rather than a data state.
    #[must_use]
    pub fn over(
        cycle_day: CycleDayIx,
        slots: Vec<TimeslotIx>,
        intervals: &[TimeInterval],
        room: Option<RoomIx>,
    ) -> Option<Self> {
        if slots.is_empty() || slots.len() != intervals.len() {
            return None;
        }
        let interval = intervals
            .iter()
            .copied()
            .reduce(TimeInterval::hull)
            .expect("the list is non-empty");
        Some(Self {
            cycle_day,
            slots,
            interval,
            room,
        })
    }

    /// How many slots the occurrence covers.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// The slot the occurrence begins in.
    #[must_use]
    pub fn start(&self) -> TimeslotIx {
        self.slots[0]
    }

    /// The placement this span resolves.
    #[must_use]
    pub fn placement(&self) -> Placement {
        Placement {
            timeslot: self.start(),
            room: self.room,
        }
    }

    /// Whether two spans occupy overlapping real time on the same cycle day.
    ///
    /// The comparison [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
    /// rule 2 specifies, and the reason it is here rather than written out at each call site:
    /// **different timeslot ids do not prove non-overlapping times.** A hall shared by two
    /// campuses on different bell schedules is booked at 09:00 under two different
    /// `timeslot_id`s, and ADR 0014 calls failing to detect that "a correctness failure".
    ///
    /// Same-day is part of the test because a cycle position is the day axis and both
    /// schedules of one school materialise against the same `cycle_day` rows — so no
    /// calendar date and no timezone is involved.
    #[must_use]
    pub fn clashes_in_time_with(&self, other: &Self) -> bool {
        self.cycle_day == other.cycle_day && self.interval.overlaps(other.interval)
    }
}

/// A candidate timetable: where every occurrence currently is.
///
/// Indexed by [`ActivityIx`], one entry per activity of the
/// [`Problem`](super::Problem) it belongs to. `None` means **unscheduled**, which is a
/// legitimate state of a partial solution rather than an error: `MEDIUM`-level
/// `activity-scheduled` is what penalises it, and that level exists so the solver can return
/// a usable partial timetable and say what it could not place, instead of returning nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    /// One entry per activity, in activity-index order.
    placements: Vec<Option<Placement>>,
}

impl Assignment {
    /// An assignment in which nothing is placed yet.
    #[must_use]
    pub fn empty(activities: usize) -> Self {
        Self {
            placements: vec![None; activities],
        }
    }

    /// How many activities the assignment covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.placements.len()
    }

    /// Whether the assignment covers no activities at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// Where one activity is, or `None` if it is unscheduled or out of range.
    #[must_use]
    pub fn placement(&self, activity: ActivityIx) -> Option<Placement> {
        self.placements.get(activity.get()).copied().flatten()
    }

    /// Places one activity, returning where it was before.
    ///
    /// # Panics
    ///
    /// If the activity is outside the assignment, which is a caller bug: an assignment is
    /// built for one problem and sized from its activity count.
    pub fn place(&mut self, activity: ActivityIx, placement: Placement) -> Option<Placement> {
        self.placements[activity.get()].replace(placement)
    }

    /// Unschedules one activity, returning where it was.
    ///
    /// # Panics
    ///
    /// As [`Assignment::place`].
    pub fn clear(&mut self, activity: ActivityIx) -> Option<Placement> {
        self.placements[activity.get()].take()
    }

    /// How many activities are placed.
    #[must_use]
    pub fn scheduled_count(&self) -> usize {
        self.placements.iter().filter(|p| p.is_some()).count()
    }

    /// The activities that are not placed, in index order.
    pub fn unscheduled(&self) -> impl Iterator<Item = ActivityIx> + '_ {
        self.placements
            .iter()
            .enumerate()
            .filter(|(_, placement)| placement.is_none())
            .map(|(ix, _)| ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX)))
    }

    /// Every placed activity and where it is, in index order.
    pub fn iter(&self) -> impl Iterator<Item = (ActivityIx, Placement)> + '_ {
        self.placements
            .iter()
            .enumerate()
            .filter_map(|(ix, placement)| {
                placement.map(|p| (ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX)), p))
            })
    }

    /// Whether every activity is placed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.placements.iter().all(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityIx, Assignment, CycleDayIx, Placement, PlacementSpan, RoomIx, TimeInterval,
        TimeslotIx,
    };
    use crate::scheduling::clock::TimeOfDay;

    fn span_text(start: &str, end: &str) -> TimeInterval {
        TimeInterval::new(
            TimeOfDay::parse(start).expect("well formed"),
            TimeOfDay::parse(end).expect("well formed"),
        )
        .expect("well ordered")
    }

    fn double_period(day: u32, first: &str, second: &str) -> PlacementSpan {
        let intervals = [span_text(first, "08:45"), span_text(second, "09:45")];
        PlacementSpan::over(
            CycleDayIx::new(day),
            vec![TimeslotIx::new(0), TimeslotIx::new(1)],
            &intervals,
            None,
        )
        .expect("two slots and two intervals")
    }

    #[test]
    fn a_placement_carries_a_start_and_an_optional_room() {
        let unroomed = Placement::unroomed(TimeslotIx::new(4));
        assert_eq!(unroomed.timeslot, TimeslotIx::new(4));
        assert_eq!(unroomed.room, None);

        let roomed = Placement::in_room(TimeslotIx::new(4), RoomIx::new(2));
        assert_eq!(roomed.room, Some(RoomIx::new(2)));
    }

    #[test]
    fn a_span_takes_its_interval_from_its_first_and_last_slot() {
        // SCHEDULING-ARCHITECTURE-DECISIONS.md §2.3, and it spans the break in the middle
        // because a room is not free during it.
        let span = double_period(0, "08:00", "09:00");
        assert_eq!(span.interval, span_text("08:00", "09:45"));
        assert_eq!(span.slot_count(), 2);
        assert_eq!(span.start(), TimeslotIx::new(0));
    }

    #[test]
    fn a_span_over_nothing_is_refused() {
        assert!(
            PlacementSpan::over(CycleDayIx::new(0), Vec::new(), &[], None).is_none(),
            "an occurrence covering no slot is a caller bug, not a state"
        );
        assert!(
            PlacementSpan::over(
                CycleDayIx::new(0),
                vec![TimeslotIx::new(0)],
                &[span_text("08:00", "08:45"), span_text("09:00", "09:45")],
                None,
            )
            .is_none(),
            "a slot list and an interval list of different lengths cannot be folded"
        );
    }

    #[test]
    fn a_span_reproduces_the_placement_it_resolves() {
        let intervals = [span_text("08:00", "08:45")];
        let span = PlacementSpan::over(
            CycleDayIx::new(1),
            vec![TimeslotIx::new(7)],
            &intervals,
            Some(RoomIx::new(3)),
        )
        .expect("one slot, one interval");
        assert_eq!(
            span.placement(),
            Placement::in_room(TimeslotIx::new(7), RoomIx::new(3))
        );
    }

    #[test]
    fn two_spans_on_different_days_never_clash() {
        // A cycle position is the day axis. Nothing here reads a date or a weekday.
        let monday = double_period(0, "08:00", "09:00");
        let tuesday = double_period(1, "08:00", "09:00");
        assert!(!monday.clashes_in_time_with(&tuesday));
    }

    #[test]
    fn two_spans_at_overlapping_minutes_on_one_day_clash() {
        // ADR 0032 rule 2 and ADR 0014's "correctness failure": these would carry different
        // timeslot ids at a two-campus school and must still be one violation.
        let north = PlacementSpan::over(
            CycleDayIx::new(0),
            vec![TimeslotIx::new(0)],
            &[span_text("09:00", "09:45")],
            Some(RoomIx::new(0)),
        )
        .expect("one slot");
        let south = PlacementSpan::over(
            CycleDayIx::new(0),
            vec![TimeslotIx::new(99)],
            &[span_text("09:30", "10:15")],
            Some(RoomIx::new(0)),
        )
        .expect("one slot");

        assert_ne!(
            north.start(),
            south.start(),
            "the point of the fixture: the slots differ"
        );
        assert!(north.clashes_in_time_with(&south));
        assert!(south.clashes_in_time_with(&north));
    }

    #[test]
    fn adjacent_spans_in_one_room_do_not_clash() {
        // ADR 0032 test 3: 08:00–08:45 then 08:50–09:35 is not a violation.
        let first = PlacementSpan::over(
            CycleDayIx::new(0),
            vec![TimeslotIx::new(0)],
            &[span_text("08:00", "08:45")],
            Some(RoomIx::new(0)),
        )
        .expect("one slot");
        let second = PlacementSpan::over(
            CycleDayIx::new(0),
            vec![TimeslotIx::new(1)],
            &[span_text("08:50", "09:35")],
            Some(RoomIx::new(0)),
        )
        .expect("one slot");
        assert!(!first.clashes_in_time_with(&second));
    }

    #[test]
    fn an_empty_assignment_schedules_nothing_and_is_not_complete() {
        let assignment = Assignment::empty(3);
        assert_eq!(assignment.len(), 3);
        assert_eq!(assignment.scheduled_count(), 0);
        assert!(!assignment.is_complete());
        assert_eq!(
            assignment.unscheduled().collect::<Vec<_>>(),
            vec![ActivityIx::new(0), ActivityIx::new(1), ActivityIx::new(2)]
        );
    }

    #[test]
    fn placing_and_clearing_report_what_was_there() {
        let mut assignment = Assignment::empty(2);
        let first = Placement::unroomed(TimeslotIx::new(0));
        let second = Placement::unroomed(TimeslotIx::new(5));

        assert_eq!(assignment.place(ActivityIx::new(0), first), None);
        assert_eq!(assignment.place(ActivityIx::new(0), second), Some(first));
        assert_eq!(assignment.placement(ActivityIx::new(0)), Some(second));
        assert_eq!(assignment.clear(ActivityIx::new(0)), Some(second));
        assert_eq!(assignment.placement(ActivityIx::new(0)), None);
    }

    #[test]
    fn an_unscheduled_activity_is_a_state_rather_than_an_error() {
        // MEDIUM-level activity-scheduled is what penalises this, so a partial timetable is
        // returnable and can say what it could not place.
        let mut assignment = Assignment::empty(2);
        assignment.place(ActivityIx::new(1), Placement::unroomed(TimeslotIx::new(3)));
        assert_eq!(assignment.scheduled_count(), 1);
        assert_eq!(
            assignment.unscheduled().collect::<Vec<_>>(),
            vec![ActivityIx::new(0)]
        );
        assert_eq!(
            assignment.iter().collect::<Vec<_>>(),
            vec![(ActivityIx::new(1), Placement::unroomed(TimeslotIx::new(3)))]
        );
    }

    #[test]
    fn a_fully_placed_assignment_is_complete() {
        let mut assignment = Assignment::empty(2);
        assignment.place(ActivityIx::new(0), Placement::unroomed(TimeslotIx::new(0)));
        assignment.place(ActivityIx::new(1), Placement::unroomed(TimeslotIx::new(1)));
        assert!(assignment.is_complete());
    }

    #[test]
    fn an_out_of_range_activity_reads_as_unscheduled_rather_than_panicking() {
        // A read is tolerant so a report can ask about anything; a write is not, because
        // writing out of range is a caller bug.
        let assignment = Assignment::empty(1);
        assert_eq!(assignment.placement(ActivityIx::new(9)), None);
    }
}
