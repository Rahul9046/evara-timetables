//! Wall-clock primitives: a time of day, a half-open interval, and a count of periods.
//!
//! # Why minutes rather than the stored text
//!
//! `period.starts_at` and `period.ends_at` are `'HH:MM'` text under
//! `CHECK (starts_at LIKE '__:__' AND time(starts_at) IS NOT NULL)` and
//! `CHECK (ends_at > starts_at)` (`V4:104-105`, `V4:111`), so no period crosses midnight and
//! text order *is* chronological order. Comparison could therefore be done on the strings.
//!
//! It is not, and
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §6 is the reason: the comparison the solver performs thousands of times per step is
//! `a.start < b.end && b.start < a.end`, and doing that on two heap strings in the inner
//! loop would be a measurable cost paid for nothing. `SCHEDULING-ARCHITECTURE-DECISIONS.md`
//! §4.2 settles it in one line — a timeslot carries its times "as comparable minutes". The
//! text is parsed once, at the edge, by [`TimeOfDay::parse`], which is also where a stored
//! value that is not a time becomes an error rather than a guess.
//!
//! # No timezone, and why that is not an omission
//!
//! Nothing here resolves a timezone, because nothing in the MVP needs to. A solve is scoped
//! to one school
//! ([ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! rule 1), one school has one `school.timezone`, and so both sides of every interval
//! comparison are in the same zone and the zone cancels. Across schools it would not cancel
//! — and the workspace holds no timezone database at all, which is why ADR 0032 §4 defers
//! cross-school comparison rather than approximating it.
//!
//! A [`TimeOfDay`] is therefore a wall-clock reading, not an instant. It is meaningful only
//! beside the school it came from.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A wall-clock time of day, as minutes since midnight.
///
/// Holds `0..=1439`. Constructed from the `'HH:MM'` text the database stores, and ordered
/// chronologically by its own integer ordering.
///
/// This is a *reading on a clock*, not a point in time: it carries no date and no timezone,
/// for the reason this module's header gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeOfDay(u16);

/// The number of minutes in a day, and so the exclusive upper bound of a [`TimeOfDay`].
const MINUTES_PER_DAY: u16 = 24 * 60;

impl TimeOfDay {
    /// Midnight, the earliest representable reading.
    pub const MIDNIGHT: Self = Self(0);

    /// Builds a time from an hour and a minute, or `None` if either is out of range.
    ///
    /// `24:00` is rejected. A bell schedule that runs to midnight ends at `23:59`, and a
    /// period that crossed midnight is already refused by `CHECK (ends_at > starts_at)`.
    #[must_use]
    pub const fn new(hour: u8, minute: u8) -> Option<Self> {
        if hour > 23 || minute > 59 {
            return None;
        }
        Some(Self(hour as u16 * 60 + minute as u16))
    }

    /// Reads the `'HH:MM'` form the database stores.
    ///
    /// Strict on purpose: exactly five characters, both parts zero-padded. `'9:30'` is
    /// refused rather than interpreted, because the column's own `CHECK` already refuses it
    /// (`V4:104`), so a value in that shape reaching here means the row is damaged and a
    /// lenient parse would hide it. This is the same principle as
    /// `DbError::CorruptIdentifier`: a persisted value that will not parse is corruption,
    /// not input.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 5 || bytes[2] != b':' {
            return None;
        }
        let digit = |b: u8| -> Option<u8> { b.is_ascii_digit().then(|| b - b'0') };
        let hour = digit(bytes[0])? * 10 + digit(bytes[1])?;
        let minute = digit(bytes[3])? * 10 + digit(bytes[4])?;
        Self::new(hour, minute)
    }

    /// Minutes since midnight.
    #[must_use]
    pub const fn minutes(self) -> u16 {
        self.0
    }

    /// This reading `minutes` later, or `None` if it would leave the day.
    ///
    /// Returning `None` rather than wrapping is deliberate: a bell schedule that wrapped
    /// past midnight would make `start < end` false for a real period and quietly break
    /// every interval comparison built on it.
    #[must_use]
    pub const fn plus_minutes(self, minutes: u16) -> Option<Self> {
        let sum = self.0 + minutes;
        if sum >= MINUTES_PER_DAY {
            return None;
        }
        Some(Self(sum))
    }
}

impl fmt::Display for TimeOfDay {
    /// Renders the `'HH:MM'` form, so a round trip through [`TimeOfDay::parse`] is exact.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

/// A half-open span of wall-clock time within one day: `[start, end)`.
///
/// Half-open is the whole point. Two periods that abut — one ending at 08:45 and the next
/// starting at 08:45 — do not overlap, and a closed interval would report that they do,
/// making every consecutive pair of lessons a clash.
/// [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
/// test 3 exists to pin exactly this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeInterval {
    /// When it begins, inclusive.
    pub start: TimeOfDay,
    /// When it ends, exclusive.
    pub end: TimeOfDay,
}

impl TimeInterval {
    /// Builds an interval, or `None` unless `start < end`.
    ///
    /// Empty and reversed intervals are both refused. `CHECK (ends_at > starts_at)`
    /// (`V4:111`) already guarantees it for a stored period, so `None` here means the same
    /// thing [`TimeOfDay::parse`] returning `None` means.
    #[must_use]
    pub const fn new(start: TimeOfDay, end: TimeOfDay) -> Option<Self> {
        if start.0 >= end.0 {
            return None;
        }
        Some(Self { start, end })
    }

    /// Whether two intervals share any minute.
    ///
    /// `a.start < b.end && b.start < a.end` — the test
    /// [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
    /// rule 2 names, and the one that makes a room shared by two campuses on different bell
    /// schedules safe. **Two different `timeslot_id`s prove nothing about time**; this does.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start.0 < other.end.0 && other.start.0 < self.end.0
    }

    /// How long it lasts, in minutes.
    #[must_use]
    pub const fn minutes(self) -> u16 {
        self.end.0 - self.start.0
    }

    /// The smallest interval containing both, which is how a multi-period occurrence gets
    /// its span.
    ///
    /// `[first period's start, last period's end)`, per
    /// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §2.3. Note that this **spans any gap between
    /// them**: a double period either side of a fifteen-minute break occupies the break as
    /// far as a room is concerned, which is correct — the room is not free during it.
    #[must_use]
    pub const fn hull(self, other: Self) -> Self {
        Self {
            start: if self.start.0 <= other.start.0 {
                self.start
            } else {
                other.start
            },
            end: if self.end.0 >= other.end.0 {
                self.end
            } else {
                other.end
            },
        }
    }
}

impl fmt::Display for TimeInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}–{}", self.start, self.end)
    }
}

/// How many **ordinal-consecutive teaching periods** one occurrence needs.
///
/// # This is a count of periods, not a span of minutes
///
/// `activity.duration_periods` is `NOT NULL DEFAULT 1 CHECK (duration_periods >= 1)`
/// (`V15:825`), and it counts periods.
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §5 fixes what those periods must be: `d` ordinal-consecutive `TEACHING` periods **of the
/// activity's own resolved bell schedule**, on one cycle day. A run is broken by a
/// non-teaching period between two teaching ones; a mere wall-clock gap between consecutive
/// teaching periods does not break it, because a gap between bells is a gap between bells.
///
/// The elapsed minutes of a placement are therefore *derived* — see [`TimeInterval::hull`]
/// — and are never the duration itself. Two schools whose Period 3 is forty minutes and
/// sixty minutes both express a double period as `2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DurationPeriods(u16);

impl DurationPeriods {
    /// One period: the default, and what almost every lesson is.
    pub const SINGLE: Self = Self(1);

    /// Builds a duration, or `None` for zero.
    ///
    /// Zero is refused rather than normalised, mirroring `CHECK (duration_periods >= 1)`.
    /// An occurrence of no periods is not a lesson nobody attends; it is a damaged row.
    #[must_use]
    pub const fn new(periods: u16) -> Option<Self> {
        if periods == 0 {
            return None;
        }
        Some(Self(periods))
    }

    /// The count of periods.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The count as a `usize`, for indexing and slicing.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for DurationPeriods {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} period{}", self.0, if self.0 == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationPeriods, TimeInterval, TimeOfDay};

    fn at(text: &str) -> TimeOfDay {
        TimeOfDay::parse(text).expect("test times are well formed")
    }

    fn span(start: &str, end: &str) -> TimeInterval {
        TimeInterval::new(at(start), at(end)).expect("test intervals are well formed")
    }

    #[test]
    fn a_stored_time_round_trips_through_text() {
        for text in ["00:00", "08:45", "09:05", "13:30", "23:59"] {
            assert_eq!(at(text).to_string(), text);
        }
    }

    #[test]
    fn an_unpadded_time_is_refused_rather_than_interpreted() {
        // The column's own CHECK refuses '9:30' (V4:104), so a value in that shape reaching
        // the parser means the row is damaged. Guessing would hide that.
        assert_eq!(TimeOfDay::parse("9:30"), None);
        assert_eq!(TimeOfDay::parse("08:5"), None);
        assert_eq!(TimeOfDay::parse("084:5"), None);
        assert_eq!(TimeOfDay::parse(""), None);
        assert_eq!(TimeOfDay::parse("25:00"), None);
        assert_eq!(TimeOfDay::parse("08:60"), None);
        assert_eq!(TimeOfDay::parse("ab:cd"), None);
    }

    #[test]
    fn text_order_and_minute_order_agree() {
        // ADR 0033 §6 relies on this: the database compares the text, the solver compares
        // the minutes, and they must not be able to disagree.
        let mut texts = ["13:30", "08:45", "09:05", "08:00"];
        let mut times: Vec<TimeOfDay> = texts.iter().map(|t| at(t)).collect();
        texts.sort_unstable();
        times.sort_unstable();
        let rendered: Vec<String> = times.iter().map(ToString::to_string).collect();
        assert_eq!(rendered, texts);
    }

    #[test]
    fn midnight_is_never_reached_by_addition() {
        assert_eq!(at("23:00").plus_minutes(59), Some(at("23:59")));
        assert_eq!(at("23:00").plus_minutes(60), None, "the day does not wrap");
    }

    #[test]
    fn abutting_intervals_do_not_overlap() {
        // ADR 0032 test 3. A closed interval would make every consecutive pair of lessons a
        // clash, which is the specific error half-open comparison exists to avoid.
        let first = span("08:00", "08:45");
        let second = span("08:45", "09:30");
        assert!(!first.overlaps(second));
        assert!(!second.overlaps(first));
    }

    #[test]
    fn overlapping_intervals_are_detected_in_both_directions() {
        // ADR 0032 test 2 in miniature: North P1 08:00–08:45 against South P1 08:30–09:15.
        let north = span("08:00", "08:45");
        let south = span("08:30", "09:15");
        assert!(north.overlaps(south));
        assert!(south.overlaps(north), "the relation is symmetric");
    }

    #[test]
    fn containment_counts_as_overlap() {
        let long = span("08:00", "10:00");
        let short = span("08:30", "08:45");
        assert!(long.overlaps(short));
        assert!(short.overlaps(long));
    }

    #[test]
    fn an_interval_overlaps_itself() {
        let only = span("08:00", "08:45");
        assert!(only.overlaps(only));
    }

    #[test]
    fn an_empty_or_reversed_interval_is_refused() {
        assert_eq!(TimeInterval::new(at("08:00"), at("08:00")), None);
        assert_eq!(TimeInterval::new(at("09:00"), at("08:00")), None);
    }

    #[test]
    fn the_hull_of_a_double_period_spans_the_break_between_them() {
        // Deliberate: a room is not free during the break in the middle of a double.
        let first = span("08:00", "08:45");
        let second = span("09:00", "09:45");
        let whole = first.hull(second);
        assert_eq!(whole, span("08:00", "09:45"));
        assert_eq!(whole.minutes(), 105);
    }

    #[test]
    fn a_duration_counts_periods_and_never_minutes() {
        // Two schools with 40-minute and 60-minute periods both express a double as 2.
        assert_eq!(DurationPeriods::new(2).map(DurationPeriods::get), Some(2));
        assert_eq!(DurationPeriods::SINGLE.get(), 1);
        assert_eq!(
            DurationPeriods::new(0),
            None,
            "zero periods is not a lesson"
        );
    }

    #[test]
    fn a_duration_renders_for_a_reader() {
        assert_eq!(DurationPeriods::SINGLE.to_string(), "1 period");
        assert_eq!(
            DurationPeriods::new(2)
                .expect("two is positive")
                .to_string(),
            "2 periods"
        );
    }
}
