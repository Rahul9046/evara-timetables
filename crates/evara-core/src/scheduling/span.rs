//! Resolving a placement into the minutes it occupies.
//!
//! # One implementation, three callers
//!
//! A [`Placement`] says which timeslot an occurrence *starts* in. What it **occupies** is `d`
//! ordinal-consecutive slots of its bell schedule on one cycle day
//! ([ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §5), and turning the first into the second is the same arithmetic for everyone who asks.
//!
//! It lived in `evara-constraints` until M7, where the evaluators owned it. Three callers need
//! it now and only one could reach it:
//!
//! | Caller | Asks |
//! | --- | --- |
//! | `evara-constraints` (M5) | *what is wrong with this arrangement?* — every clash and timing rule |
//! | `evara-db` (M7) | *which slots does this stored entry occupy?* — the reader, and the writer's guards |
//! | `evara-report` (Phase 7) | *what occupies Monday period 2?* — the grid renders multi-period lessons as spans |
//!
//! `evara-db` and `evara-report` both depend on `evara-core` **only** (`ARCHITECTURE.md` §3),
//! so leaving the resolver in `evara-constraints` would have forced either a second
//! implementation — `CLAUDE.md` §2's most important rule broken — or a public API wide enough
//! to make that copy easy to write by accident. Moving it down is the same decision ADR 0036
//! made for the bell-schedule resolver and M6 made for [`Rng`](super::rng::Rng).
//!
//! # Resolution is permissive; judging is a rule's job
//!
//! If an occurrence cannot take `d` slots from where it was placed — the day runs out, a
//! non-teaching period interrupts, the slot belongs to another schedule — the span is built
//! from what *is* there and [`SpanDefect`] records what went wrong. It is **not** an error
//! here.
//!
//! `duration-contiguous` reports the short or broken run and `no-teaching-in-period` the break.
//! A resolver that refused would make both of those unreachable and would turn a reportable
//! timetable into an unreadable failure — which is exactly what a user needs *not* to happen
//! when they open a timetable whose inputs have changed underneath it.

use serde::{Deserialize, Serialize};

use super::clock::TimeInterval;
use super::index::{ActivityIx, SectionIx, TimeslotIx};
use super::placement::{Placement, PlacementSpan};
use super::problem::Problem;

/// Why an occurrence's span is not what its duration asked for.
///
/// Carried rather than acted on: the span is still built, and the rule that cares reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDefect {
    /// How many slots the occurrence wanted.
    pub wanted: usize,
    /// How many ordinal-consecutive slots were available from where it was placed.
    pub got: usize,
    /// Whether a non-teaching period falls inside the run, break included.
    pub crosses_non_teaching: bool,
    /// Whether the starting slot itself is not a teaching period.
    pub starts_outside_teaching: bool,
    /// Whether the starting slot belongs to a schedule the section did not resolve to.
    pub wrong_bell_schedule: bool,
}

impl SpanDefect {
    /// Whether the run is as long as the duration asked for.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.got >= self.wanted
    }

    /// Whether anything at all is wrong with the run.
    ///
    /// What a reader uses to decide whether a stored timetable still describes something
    /// placeable. A caller that only wanted the slots can ignore it; a caller writing a report
    /// about a timetable whose grid has moved underneath it cannot.
    #[must_use]
    pub const fn is_sound(self) -> bool {
        self.is_complete()
            && !self.crosses_non_teaching
            && !self.starts_outside_teaching
            && !self.wrong_bell_schedule
    }
}

/// One placed occurrence, resolved into what it occupies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSpan {
    /// Which occurrence.
    pub activity: ActivityIx,
    /// The section it belongs to.
    pub section: SectionIx,
    /// What it occupies.
    pub span: PlacementSpan,
    /// What is wrong with the run, if anything.
    pub defect: SpanDefect,
}

/// Resolves one placement into the span it occupies.
///
/// Returns `None` only when the starting timeslot is not in the problem at all, which is an
/// invalid reference. Everything else — a short run, a break in the middle, the wrong schedule
/// — produces a span with a [`SpanDefect`] for a rule to report.
///
/// `wanted` is the occurrence's `duration_periods`. It is a parameter rather than read from the
/// activity so that a caller asking *"what would a `d`-period lesson occupy here?"* — which the
/// move validator does — needs no activity at all.
#[must_use]
pub fn resolve_span(
    problem: &Problem,
    activity: ActivityIx,
    section: SectionIx,
    wanted: usize,
    placement: Placement,
) -> Option<ResolvedSpan> {
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

    Some(ResolvedSpan {
        activity,
        section,
        span,
        defect,
    })
}

/// Resolves one placement, reading the duration and the section off the activity.
///
/// The form every caller that *has* an activity wants, so none of them has to look those two
/// facts up and none can look up the wrong ones. Returns `None` where the activity is not in
/// the problem, as well as where the timeslot is not.
#[must_use]
pub fn resolve_activity_span(
    problem: &Problem,
    activity: ActivityIx,
    placement: Placement,
) -> Option<ResolvedSpan> {
    let fact = problem.activities.get(activity.get())?;
    resolve_span(
        problem,
        activity,
        fact.section,
        fact.duration.as_usize(),
        placement,
    )
}

#[cfg(test)]
mod tests;
