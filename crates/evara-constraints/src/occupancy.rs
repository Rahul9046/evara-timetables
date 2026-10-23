//! What a candidate timetable actually occupies, and the sweep every clash rule shares.
//!
//! # Resolved once, read by every evaluator
//!
//! An [`Assignment`] says which timeslot each occurrence *starts* in. A clash is about the
//! minutes it *occupies*, which for a `duration_periods = d` occurrence is `d`
//! ordinal-consecutive slots of its resolved bell schedule (ADR 0033 §5). Turning the first
//! into the second is the same work for all three clash rules plus the two timing ones, so it
//! happens once, here.
//!
//! # Resolution is permissive; judging is the rule's job
//!
//! If an occurrence cannot take `d` slots from where it was placed — the day runs out, a
//! non-teaching period interrupts, the slot belongs to another schedule — the span is built
//! from what *is* there and records what went wrong. It is not an error here.
//! `duration-contiguous` is the rule that reports a short or broken run, and
//! `no-teaching-in-period` the one that reports a break; a resolver that refused would make
//! those two unreachable and would turn a reportable timetable into an unreadable failure.
//!
//! [`PlacementSpan::over`] owns the one piece of arithmetic involved — the interval is
//! `[first period's start, last period's end)` — and this module calls it rather than
//! computing a hull of its own.
//!
//! # The resolver moved down in M7, and this module calls it
//!
//! `Occupancy::resolve` used to live here. M7 promoted it to
//! [`evara_core::scheduling::span`], because two new callers need the same answer and neither
//! can reach this crate: `evara-db`'s reader, turning stored `timetable_entry` rows back into an
//! arrangement, and `evara-report`'s grid, which renders a multi-period lesson as a span. Both
//! depend on `evara-core` only.
//!
//! So [`Placed`] is now [`ResolvedSpan`] and [`resolve_activity_span`] is the one
//! implementation. Nothing about the answer changed — the move is asserted by the evaluators'
//! own tests, which were written against the behaviour and did not need editing.
//!
//! # Why the pair sweep is not a nested loop over everything
//!
//! A school with 500 occurrences has 124 750 unordered pairs, and almost none of them share a
//! day. [`Occupancy::overlapping_pairs`] sorts the subset by `(cycle day, start)` and keeps an
//! active list of spans whose interval has not yet ended, so it compares only spans that
//! genuinely coexist. Teacher and room clash narrow the subset first — by teacher, by room —
//! which is what the brief means by grouping before sweeping.
//!
//! # Mutable since M6, and why that is the same resolver
//!
//! [`Occupancy::of`] builds the whole thing from an assignment; [`Occupancy::place`] and
//! [`Occupancy::withdraw`] move one occurrence without rebuilding the rest. The second pair
//! exists because a construction solver asks *"what would this placement break?"* thousands of
//! times per solve and [`Occupancy::of`] is linear in the timetable.
//!
//! Both routes call the same [`resolve`] and maintain the same indexes, so an occupancy reached
//! by a thousand placements is **equal** to one built from the resulting assignment — asserted
//! by a test over random operation sequences, because that equality is what lets
//! [`crate::IncrementalScorer`] be trusted. The indexes are what make the narrowing cheap:
//! `taught_by` and `booked_into` were scans of the whole timetable and are now lookups.

use evara_core::scheduling::{
    ActivityIx, Assignment, CycleDayIx, Placement, Problem, ResolvedSpan, RoomIx, SectionFact,
    TeacherIx, resolve_activity_span,
};

/// One placed occurrence, resolved.
///
/// [`ResolvedSpan`] under its original name. The alias is kept because eight evaluators and
/// their tests name it, and because *"what this crate calls a placed occurrence"* is a useful
/// thing for this crate to be able to say.
pub(crate) type Placed = ResolvedSpan;

/// Which way a placement could not be read against the problem.
///
/// Two cases and they are not the same remedy: an activity index past the end of the table
/// means the assignment describes a different problem, while a dangling timeslot means this
/// problem's grid does not hold the slot named. Both are reported as sentences by
/// [`Occupancy::invalid_references`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unreadable {
    /// The problem holds no activity at that index.
    UnknownActivity,
    /// The grid holds no timeslot at the placement's start.
    DanglingTimeslot,
}

/// A candidate timetable, resolved into the minutes it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupancy<'p> {
    problem: &'p Problem,
    /// One entry per activity index; `None` is unscheduled, which is a legitimate state.
    placed: Vec<Option<Placed>>,
    /// Activities named by the assignment that the problem does not hold.
    unknown: Vec<usize>,
    /// Activities whose placement names a timeslot the problem does not hold.
    dangling: Vec<ActivityIx>,
    /// Placed occurrences per teacher, per room and per cycle day, each sorted by activity
    /// index.
    ///
    /// Maintained rather than derived, because this is exactly how the three clash rules
    /// narrow — `teacher-clash` by the teacher, `room-clash` by the room, and `student-clash`
    /// by the day, since two occurrences on different days cannot clash whatever their
    /// audiences — and a solver asks them thousands of times. Sorted so every caller sees the
    /// same order a scan in index order would have produced; `SOLVER.md` §5's determinism
    /// requirement reaches the order violations come out in.
    by_teacher: Vec<Vec<ActivityIx>>,
    by_room: Vec<Vec<ActivityIx>>,
    by_day: Vec<Vec<ActivityIx>>,
}

impl<'p> Occupancy<'p> {
    /// Resolves an assignment against a problem.
    ///
    /// # Invalid references are recorded, not panicked on
    ///
    /// An assignment can arrive from outside — a file handed to `evara evaluate`, a future
    /// IPC call — so an activity index past the end of the problem, or a placement naming a
    /// timeslot that is not in the grid, has to be *reported*. Both are collected and
    /// [`Occupancy::invalid_references`] hands them to the caller, which refuses the
    /// evaluation rather than scoring a timetable it could not read.
    #[must_use]
    pub fn of(problem: &'p Problem, assignment: &Assignment) -> Self {
        let mut occupancy = Self::empty(problem);

        for (activity, placement) in assignment.iter() {
            match occupancy.readable(activity, placement) {
                Ok(found) => occupancy.file(found),
                Err(Unreadable::UnknownActivity) => occupancy.unknown.push(activity.get()),
                Err(Unreadable::DanglingTimeslot) => occupancy.dangling.push(activity),
            }
        }

        // An assignment longer than the problem names activities that do not exist.
        for extra in problem.activities.len()..assignment.len() {
            if assignment
                .placement(ActivityIx::new(u32::try_from(extra).unwrap_or(u32::MAX)))
                .is_some()
            {
                occupancy.unknown.push(extra);
            }
        }
        occupancy.unknown.sort_unstable();
        occupancy.unknown.dedup();
        occupancy.dangling.sort_unstable();

        occupancy
    }

    /// An occupancy of `problem` in which nothing is placed.
    ///
    /// The starting point a construction solver builds from, and what [`Occupancy::of`] fills
    /// in. Carries no invalid reference, because nothing has been read.
    #[must_use]
    pub fn empty(problem: &'p Problem) -> Self {
        Self {
            problem,
            placed: vec![None; problem.activities.len()],
            unknown: Vec::new(),
            dangling: Vec::new(),
            by_teacher: vec![Vec::new(); problem.teachers.len()],
            by_room: vec![Vec::new(); problem.rooms.len()],
            by_day: vec![Vec::new(); problem.context.cycle_days.len()],
        }
    }

    /// Places one occurrence, replacing where it was.
    ///
    /// Returns `false` when the placement cannot be read against this problem — an unknown
    /// activity, or a timeslot the grid does not hold — and then **nothing changed**, not even
    /// the previous placement. An unreadable placement is a caller bug here rather than a data
    /// state: a solver offers only candidates it generated from this problem, and the route
    /// that has to tolerate one from outside is [`Occupancy::of`], which records it as an
    /// invalid reference instead.
    pub fn place(&mut self, activity: ActivityIx, placement: Placement) -> bool {
        let Ok(found) = self.readable(activity, placement) else {
            return false;
        };
        self.withdraw(activity);
        self.file(found);
        true
    }

    /// Unschedules one occurrence, returning what it had occupied.
    pub fn withdraw(&mut self, activity: ActivityIx) -> Option<Placement> {
        let gone = self.placed.get_mut(activity.get())?.take()?;
        let placement = gone.span.placement();
        if let Some(section) = self.problem.sections.get(gone.section.get()) {
            for teacher in &section.teachers {
                Self::forget(&mut self.by_teacher, teacher.get(), activity);
            }
        }
        if let Some(room) = placement.room {
            Self::forget(&mut self.by_room, room.get(), activity);
        }
        Self::forget(&mut self.by_day, gone.span.cycle_day.get(), activity);
        Some(placement)
    }

    /// Resolves one placement, or says which way it could not be read.
    ///
    /// Pure: it changes nothing, so a caller can decide between recording the failure and
    /// refusing the write. The one call site of [`resolve`].
    fn readable(&self, activity: ActivityIx, placement: Placement) -> Result<Placed, Unreadable> {
        // The activity has to exist *before* the resolver is asked, because the two failures
        // are different remedies: an unknown activity means the assignment describes another
        // problem, and a dangling timeslot means this grid does not hold the slot named.
        if self.problem.activities.get(activity.get()).is_none() {
            return Err(Unreadable::UnknownActivity);
        }
        resolve_activity_span(self.problem, activity, placement).ok_or(Unreadable::DanglingTimeslot)
    }

    /// Records a resolved occurrence and files it in every index.
    ///
    /// The one place an index is widened, so [`Occupancy::of`] and [`Occupancy::place`] cannot
    /// come to maintain different bookkeeping — which is the equality the incremental scorer
    /// rests on.
    fn file(&mut self, found: Placed) {
        let activity = found.activity;
        if let Some(section) = self.problem.sections.get(found.section.get()) {
            for teacher in &section.teachers {
                Self::remember(&mut self.by_teacher, teacher.get(), activity);
            }
        }
        if let Some(room) = found.span.room {
            Self::remember(&mut self.by_room, room.get(), activity);
        }
        Self::remember(&mut self.by_day, found.span.cycle_day.get(), activity);
        self.placed[activity.get()] = Some(found);
    }

    /// Adds `activity` to one bucket, keeping it sorted and free of repeats.
    fn remember(table: &mut [Vec<ActivityIx>], at: usize, activity: ActivityIx) {
        if let Some(bucket) = table.get_mut(at)
            && let Err(insert_at) = bucket.binary_search(&activity)
        {
            bucket.insert(insert_at, activity);
        }
    }

    /// Removes `activity` from one bucket.
    fn forget(table: &mut [Vec<ActivityIx>], at: usize, activity: ActivityIx) {
        if let Some(bucket) = table.get_mut(at)
            && let Ok(found) = bucket.binary_search(&activity)
        {
            bucket.remove(found);
        }
    }

    /// The problem this is an occupancy of.
    #[must_use]
    pub const fn problem(&self) -> &'p Problem {
        self.problem
    }

    /// References the problem does not hold, as human-readable sentences.
    ///
    /// Empty for a sound assignment. Non-empty means the caller was handed something that
    /// does not describe this problem, which is an execution failure rather than a violation.
    #[must_use]
    pub fn invalid_references(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .unknown
            .iter()
            .map(|at| {
                format!(
                    "the assignment places activity #{at}, and this problem holds {}",
                    self.problem.activities.len()
                )
            })
            .collect();
        found.extend(self.dangling.iter().map(|at| {
            format!(
                "activity #{} is placed in a timeslot this problem's grid does not hold",
                at.raw()
            )
        }));
        found
    }

    /// What one occurrence occupies, or `None` where it is unscheduled.
    #[must_use]
    pub(crate) fn at(&self, activity: ActivityIx) -> Option<&Placed> {
        self.placed.get(activity.get())?.as_ref()
    }

    /// Every placed occurrence, in activity-index order.
    pub(crate) fn all(&self) -> impl Iterator<Item = &Placed> + '_ {
        self.placed.iter().filter_map(Option::as_ref)
    }

    /// How many occurrences are placed.
    #[must_use]
    pub fn placed_count(&self) -> usize {
        self.placed.iter().filter(|slot| slot.is_some()).count()
    }

    /// Whether every occurrence of the problem is placed.
    ///
    /// What distinguishes a complete timetable from a partial one. It is **not** the same
    /// question as whether the timetable is any good: `activity-scheduled` reports the gap at
    /// `MEDIUM`, and a partial assignment is a legitimate intermediate state of M6's
    /// construction rather than an invalid input.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.placed_count() == self.problem.activities.len()
    }

    /// The assignment this occupancy is of.
    ///
    /// The inverse of [`Occupancy::of`] for everything it could read, which is what lets a
    /// solver keep one mutable occupancy and hand out an [`Assignment`] at the end rather than
    /// maintaining both and risking disagreement.
    #[must_use]
    pub fn assignment(&self) -> Assignment {
        let mut assignment = Assignment::empty(self.problem.activities.len());
        for placed in self.all() {
            assignment.place(placed.activity, placed.span.placement());
        }
        assignment
    }

    /// Every pair among `among` that occupies overlapping real time on one cycle day.
    ///
    /// # An interval sweep, not a nested loop
    ///
    /// Sorted by `(cycle day, interval start, activity)` and swept with an active list, so the
    /// comparisons are proportional to the number of pairs that genuinely coexist rather than
    /// to the square of the input. The trailing `activity` in the sort key is what makes the
    /// output order deterministic when two spans start at the same minute.
    ///
    /// Pairs come out with the lower activity index first, and in sorted order, so a caller
    /// reporting them produces the same report twice.
    ///
    /// Overlap is [`PlacementSpan::clashes_in_time_with`] — the one implementation of ADR 0032
    /// rule 2's test. Adjacent lessons do not clash, because the intervals are half-open; and
    /// two different `timeslot_id`s prove nothing, which is the whole reason the comparison is
    /// on minutes.
    #[must_use]
    pub(crate) fn overlapping_pairs(&self, among: &[ActivityIx]) -> Vec<(ActivityIx, ActivityIx)> {
        let mut spans: Vec<(&Placed, ActivityIx)> = among
            .iter()
            .filter_map(|at| self.at(*at).map(|placed| (placed, *at)))
            .collect();
        spans.sort_by_key(|(placed, at)| {
            (
                placed.span.cycle_day.raw(),
                placed.span.interval.start.minutes(),
                at.raw(),
            )
        });

        let mut pairs = Vec::new();
        // Spans whose interval has not yet ended, on the day currently being swept.
        let mut active: Vec<(&Placed, ActivityIx)> = Vec::new();
        for (placed, at) in spans {
            active.retain(|(open, _)| {
                open.span.cycle_day == placed.span.cycle_day
                    && open.span.interval.end.minutes() > placed.span.interval.start.minutes()
            });
            for (open, other) in &active {
                if open.span.clashes_in_time_with(&placed.span) {
                    let (low, high) = if other.raw() <= at.raw() {
                        (*other, at)
                    } else {
                        (at, *other)
                    };
                    pairs.push((low, high));
                }
            }
            active.push((placed, at));
        }

        pairs.sort_unstable();
        pairs.dedup();
        pairs
    }

    /// The members of `among` whose span overlaps `activity`'s, excluding itself.
    ///
    /// The half of [`Occupancy::overlapping_pairs`] a per-occurrence question needs: *which
    /// already-placed occurrences does **this** one collide with?* Same overlap test, same
    /// sorted output, and a linear scan rather than a sweep because `among` is already narrowed
    /// to one teacher, one room or one group and the answer is a handful of spans.
    ///
    /// Returns nothing when `activity` is unplaced, which is correct: an unplaced occurrence
    /// occupies no minutes and cannot clash with anything.
    #[must_use]
    pub(crate) fn overlapping_with(
        &self,
        activity: ActivityIx,
        among: &[ActivityIx],
    ) -> Vec<ActivityIx> {
        let Some(mine) = self.at(activity) else {
            return Vec::new();
        };
        let mut found: Vec<ActivityIx> = among
            .iter()
            .copied()
            .filter(|other| *other != activity)
            .filter(|other| {
                self.at(*other)
                    .is_some_and(|theirs| theirs.span.clashes_in_time_with(&mine.span))
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The section one occurrence belongs to.
    #[must_use]
    pub(crate) fn section_of(&self, activity: ActivityIx) -> Option<&'p SectionFact> {
        let section = self.problem.activities.get(activity.get())?.section;
        self.problem.sections.get(section.get())
    }

    /// The placed occurrences whose section names `teacher`.
    ///
    /// An index lookup rather than a scan of the timetable. The bucket is maintained by
    /// [`Occupancy::place`] and [`Occupancy::withdraw`] and is sorted by activity index, so the
    /// order matches what a scan would have produced.
    #[must_use]
    pub(crate) fn taught_by(&self, teacher: TeacherIx) -> &[ActivityIx] {
        self.by_teacher
            .get(teacher.get())
            .map_or(&[], Vec::as_slice)
    }

    /// The placed occurrences booked into `room`.
    #[must_use]
    pub(crate) fn booked_into(&self, room: RoomIx) -> &[ActivityIx] {
        self.by_room.get(room.get()).map_or(&[], Vec::as_slice)
    }

    /// The placed occurrences running on one cycle position.
    ///
    /// What makes `student-clash`'s per-occurrence question cheap, and the narrowing is sound
    /// for a reason worth stating: a span never crosses a cycle position
    /// ([`PlacementSpan::cycle_day`]), and
    /// [`PlacementSpan::clashes_in_time_with`] requires the days to be equal. So every
    /// occurrence that could clash with one on this day is in this bucket, and nothing outside
    /// it needs testing. The audience test is still ADR 0035 §5's and is applied by the rule,
    /// not here.
    #[must_use]
    pub(crate) fn on_day(&self, day: CycleDayIx) -> &[ActivityIx] {
        self.by_day.get(day.get()).map_or(&[], Vec::as_slice)
    }
}

/// The cycle day a span runs on, for a message that has to name it.
#[must_use]
pub(crate) fn day_label(problem: &Problem, day: CycleDayIx) -> String {
    problem
        .context
        .cycle_days
        .get(day.get())
        .map_or_else(|| format!("day #{}", day.raw()), |d| d.label.clone())
}
