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
//! # Why the pair sweep is not a nested loop over everything
//!
//! A school with 500 occurrences has 124 750 unordered pairs, and almost none of them share a
//! day. [`Occupancy::overlapping_pairs`] sorts the subset by `(cycle day, start)` and keeps an
//! active list of spans whose interval has not yet ended, so it compares only spans that
//! genuinely coexist. Teacher and room clash narrow the subset first — by teacher, by room —
//! which is what the brief means by grouping before sweeping.

use evara_core::scheduling::{
    ActivityIx, Assignment, CycleDayIx, Placement, PlacementSpan, Problem, RoomIx, SectionFact,
    SectionIx, TimeInterval, TimeslotIx,
};

/// Why an occurrence's span is not what its duration asked for.
///
/// Carried rather than acted on: the span is still built, and the rule that cares reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SpanDefect {
    /// How many slots the occurrence wanted.
    pub(crate) wanted: usize,
    /// How many ordinal-consecutive slots were available from where it was placed.
    pub(crate) got: usize,
    /// Whether a non-teaching period falls inside the run, break included.
    pub(crate) crosses_non_teaching: bool,
    /// Whether the starting slot itself is not a teaching period.
    pub(crate) starts_outside_teaching: bool,
    /// Whether the starting slot belongs to a schedule the section did not resolve to.
    pub(crate) wrong_bell_schedule: bool,
}

impl SpanDefect {
    /// Whether the run is as long as the duration asked for.
    pub(crate) const fn is_complete(self) -> bool {
        self.got >= self.wanted
    }
}

/// One placed occurrence, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    /// Which occurrence.
    pub(crate) activity: ActivityIx,
    /// The section it belongs to.
    pub(crate) section: SectionIx,
    /// What it occupies.
    pub(crate) span: PlacementSpan,
    /// What is wrong with the run, if anything.
    pub(crate) defect: SpanDefect,
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
        let mut placed: Vec<Option<Placed>> = vec![None; problem.activities.len()];
        let mut unknown = Vec::new();
        let mut dangling = Vec::new();

        for (activity, placement) in assignment.iter() {
            let Some(fact) = problem.activities.get(activity.get()) else {
                unknown.push(activity.get());
                continue;
            };
            match resolve(
                problem,
                activity,
                fact.section,
                fact.duration.as_usize(),
                placement,
            ) {
                Some(found) => placed[activity.get()] = Some(found),
                None => dangling.push(activity),
            }
        }

        // An assignment longer than the problem names activities that do not exist.
        for extra in problem.activities.len()..assignment.len() {
            if assignment
                .placement(ActivityIx::new(u32::try_from(extra).unwrap_or(u32::MAX)))
                .is_some()
            {
                unknown.push(extra);
            }
        }
        unknown.sort_unstable();
        unknown.dedup();
        dangling.sort_unstable();

        Self {
            problem,
            placed,
            unknown,
            dangling,
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

    /// The section one occurrence belongs to.
    #[must_use]
    pub(crate) fn section_of(&self, activity: ActivityIx) -> Option<&'p SectionFact> {
        let section = self.problem.activities.get(activity.get())?.section;
        self.problem.sections.get(section.get())
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

    /// The placed occurrences whose section names `teacher`.
    #[must_use]
    pub(crate) fn taught_by(&self, teacher: evara_core::scheduling::TeacherIx) -> Vec<ActivityIx> {
        self.all()
            .filter(|placed| {
                self.problem
                    .sections
                    .get(placed.section.get())
                    .is_some_and(|section| section.teachers.contains(&teacher))
            })
            .map(|placed| placed.activity)
            .collect()
    }

    /// The placed occurrences booked into `room`.
    #[must_use]
    pub(crate) fn booked_into(&self, room: RoomIx) -> Vec<ActivityIx> {
        self.all()
            .filter(|placed| placed.span.room == Some(room))
            .map(|placed| placed.activity)
            .collect()
    }
}

/// Resolves one placement into the span it occupies.
///
/// Returns `None` only when the starting timeslot is not in the problem at all, which is an
/// invalid reference. Everything else — a short run, a break in the middle, the wrong
/// schedule — produces a span with a [`SpanDefect`] for a rule to report.
fn resolve(
    problem: &Problem,
    activity: ActivityIx,
    section: SectionIx,
    wanted: usize,
    placement: Placement,
) -> Option<Placed> {
    let start = problem.timeslots.get(placement.timeslot.get())?;
    let day = start.cycle_day;

    // The run is taken from the schedule the *slot* belongs to, not the one the section
    // resolved to. Those can differ, and when they do that is the defect — resolving against
    // the section's schedule instead would silently relocate the lesson.
    let ordered: Vec<TimeslotIx> = problem.slots_of(start.bell_schedule, day);
    let from = ordered
        .iter()
        .position(|slot| *slot == placement.timeslot)
        .unwrap_or(0);

    let run: Vec<TimeslotIx> = ordered.into_iter().skip(from).take(wanted).collect();
    let intervals: Vec<TimeInterval> = run
        .iter()
        .filter_map(|slot| problem.timeslots.get(slot.get()).map(|s| s.interval))
        .collect();

    let span = PlacementSpan::over(day, run.clone(), &intervals, placement.room)?;

    let section_schedule = problem
        .sections
        .get(section.get())
        .and_then(|s| s.bell_schedule.schedule());
    let defect = SpanDefect {
        wanted,
        got: run.len(),
        // A non-teaching period *inside* the run, which is what breaks a contiguous run.
        // ADR 0033 §5: "a run is broken by any non-teaching period between two teaching ones".
        crosses_non_teaching: run.iter().skip(1).any(|slot| {
            problem
                .timeslots
                .get(slot.get())
                .is_some_and(|s| !s.is_teaching)
        }),
        starts_outside_teaching: !start.is_teaching,
        wrong_bell_schedule: section_schedule
            .is_some_and(|resolved| resolved != start.bell_schedule),
    };

    Some(Placed {
        activity,
        section,
        span,
        defect,
    })
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
