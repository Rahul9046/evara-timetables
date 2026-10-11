//! The real-world week, as distinct from the repeating cycle.
//!
//! # Why a weekday is allowed to appear in this module
//!
//! `CLAUDE.md` is categorical: nothing in scheduling may read a weekday, and
//! [ADR 0011](../../../docs/adr/0011-stable-timeslot-identity.md) §6 makes
//! `cycle_day.ordinal` a day's only scheduling identity. [`time`](crate::time) honours
//! that by not knowing what a week is at all.
//!
//! This module names weekdays on purpose, and the distinction is worth stating precisely
//! because it is the one a future change is most likely to erode.
//!
//! A **cycle day** is a position in a rotation: Day 1, Day A, the sixth day. It is what a
//! timetable is defined over, and it has no weekday.
//!
//! A **working day** is a fact about the institution: this school teaches on Mondays. It
//! describes the calendar a school lives in, not the pattern it timetables in. It exists
//! here to answer two advisory questions — what pattern should a calendar generator offer
//! by default, and does an explicitly mapped teaching date look like a typo — and it
//! answers nothing else.
//!
//! The load-bearing consequence: **a working day never implies a cycle day.** Which
//! rotation position a real date carries is settled by one thing only, an explicit
//! `calendar_day` row, and this module cannot create, alter or remove one. A six-day
//! rotation across a five-day working week is an ordinary school, and the two numbers
//! never meet.

use serde::{Deserialize, Serialize};

/// Which weekdays a school normally teaches on.
///
/// Monday to Friday across most of Europe and the Americas, Sunday to Thursday across
/// much of the Middle East, six days in parts of South Asia. Any combination is legal;
/// an empty week is not.
///
/// # Representation
///
/// A seven-bit set — bit 0 Monday through bit 6 Sunday — stored as one integer. That is
/// not merely compact: it buys a real invariant. A non-empty week is exactly `1..=127`,
/// so the column's own `CHECK (working_days BETWEEN 1 AND 127)` enforces "at least one
/// working day", and no repository, command handler or form is the only thing standing
/// between a school and a week it never teaches in.
///
/// Across the IPC boundary and in an export it is neither bits nor booleans but an
/// ascending list of ISO weekday numbers — `[1, 2, 3, 4, 5]` for Monday to Friday. That
/// is what an interface wants to render, and it matches the 1..7 that
/// `cycle_day.weekday_hint` has always used. The bit layout is a storage decision and
/// stops at the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkingDays(u8);

/// Why a working week was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkingDaysError {
    /// Every school teaches on some day. An empty week is a mistake, not a closed school.
    #[error("a school must have at least one working day")]
    Empty,
    /// A number outside 1..7, which names no day of any week.
    #[error("{weekday} is not a weekday: expected 1 (Monday) to 7 (Sunday)")]
    NotAWeekday {
        /// The offending value, as it arrived.
        weekday: i64,
    },
    /// A stored value outside `1..=127`, which the column's `CHECK` should have refused.
    ///
    /// Reported rather than masked. A project repaired by hand in a SQLite browser, or
    /// written by a build that encoded this differently, is damaged in a way the user
    /// needs told about — and quietly substituting Monday to Friday would hide it behind
    /// a plausible-looking week, which is the failure mode `CLAUDE.md` names for a
    /// persisted value that will not parse.
    #[error("{bits} is not a working-day set: expected 1 to 127")]
    CorruptBits {
        /// The value read from storage.
        bits: i64,
    },
}

impl WorkingDays {
    /// The ISO number of the first weekday, Monday.
    pub const MONDAY: i64 = 1;
    /// The ISO number of the last weekday, Sunday.
    pub const SUNDAY: i64 = 7;

    /// Monday to Friday.
    ///
    /// The default for a new school, and what a project upgraded from a schema with no
    /// such setting is given. It is a *default*, never an inference: a five-day cycle does
    /// not mean this, and this does not mean a five-day cycle.
    pub const MONDAY_TO_FRIDAY: Self = Self(0b001_1111);

    /// Every day of the week.
    pub const EVERY_DAY: Self = Self(0b111_1111);

    /// Builds a week from ISO weekday numbers, 1 Monday through 7 Sunday.
    ///
    /// Order and repetition do not matter — the input is a set however it arrives, which
    /// is what lets a form hand over its checkbox order unsorted.
    ///
    /// # Errors
    ///
    /// [`WorkingDaysError::NotAWeekday`] for a value outside 1..7, and
    /// [`WorkingDaysError::Empty`] when nothing is left.
    pub fn from_days(days: &[i64]) -> Result<Self, WorkingDaysError> {
        let mut bits: u8 = 0;
        for &day in days {
            if !(Self::MONDAY..=Self::SUNDAY).contains(&day) {
                return Err(WorkingDaysError::NotAWeekday { weekday: day });
            }
            // `day` is 1..=7 here, so the shift is 0..=6 and cannot overflow.
            bits |= 1u8 << (day - 1);
        }
        if bits == 0 {
            return Err(WorkingDaysError::Empty);
        }
        Ok(Self(bits))
    }

    /// Reads the stored seven-bit set.
    ///
    /// # Errors
    ///
    /// [`WorkingDaysError::CorruptBits`] outside `1..=127`, zero included — the schema
    /// forbids zero, so meeting it means the file is damaged.
    pub fn from_bits(bits: i64) -> Result<Self, WorkingDaysError> {
        u8::try_from(bits)
            .ok()
            .filter(|value| (1..=0b111_1111).contains(value))
            .map(Self)
            .ok_or(WorkingDaysError::CorruptBits { bits })
    }

    /// The set as the integer the column stores. Always `1..=127`.
    #[must_use]
    pub const fn bits(self) -> i64 {
        self.0 as i64
    }

    /// The working weekdays as ascending ISO numbers. Never empty.
    #[must_use]
    pub fn days(self) -> Vec<i64> {
        (Self::MONDAY..=Self::SUNDAY)
            .filter(|day| self.contains(*day))
            .collect()
    }

    /// Whether an ISO weekday, 1 Monday through 7 Sunday, is a working day.
    ///
    /// `false` for anything outside that range rather than a panic: the argument usually
    /// comes from a date whose weekday was computed elsewhere, and a surprising input is
    /// better surfaced by the caller than by a crash behind a warning banner.
    #[must_use]
    pub const fn contains(self, weekday: i64) -> bool {
        if weekday < Self::MONDAY || weekday > Self::SUNDAY {
            return false;
        }
        // Widened to i64 and masked there rather than narrowing `weekday` to a shift
        // width. The guard above already proves 1..=7, but a cast would still need either
        // a fallible conversion -- not available in a `const fn` -- or an `allow`, and
        // `CLAUDE.md` forbids silencing a lint that the code can simply not trip.
        (self.0 as i64) & (1i64 << (weekday - 1)) != 0
    }

    /// How many days a week the school teaches. `1..=7`.
    ///
    /// Display only. Nothing may compare this against `cycle.day_count` — see the module
    /// header for why those two numbers are unrelated.
    #[must_use]
    pub const fn count(self) -> i64 {
        self.0.count_ones() as i64
    }
}

impl Default for WorkingDays {
    /// Monday to Friday. See [`Self::MONDAY_TO_FRIDAY`].
    fn default() -> Self {
        Self::MONDAY_TO_FRIDAY
    }
}

/// Serialised as an ascending list of ISO weekday numbers, never as bits.
///
/// Written out rather than derived, so the storage layout and the wire format stay
/// genuinely independent facts rather than the same fact twice.
impl Serialize for WorkingDays {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.days().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WorkingDays {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let days = Vec::<i64>::deserialize(deserializer)?;
        Self::from_days(&days).map_err(serde::de::Error::custom)
    }
}

/// Typed as `number[]` in TypeScript, matching the serde representation above.
///
/// Hand-written because the serde form is hand-written: a derive would describe the
/// struct, which is the storage layout, and the interface would then disagree with the
/// JSON it actually receives.
#[cfg(feature = "ts")]
impl ts_rs::TS for WorkingDays {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    /// `number[]` rather than a named alias, so it inlines wherever it is used.
    ///
    /// Deliberately not a declared type of its own: there is no `WorkingDays.ts` to
    /// generate and nothing to keep in step. `School.workingDays` simply reads
    /// `number[]`, which is exactly what the JSON above carries. `decl` is therefore left
    /// to the trait's default, which panics — if a future `evara-typegen` tries to export
    /// this as a standalone file, that panic is the right answer and says so loudly.
    fn name(_: &ts_rs::Config) -> String {
        "number[]".to_owned()
    }

    fn inline(_: &ts_rs::Config) -> String {
        "number[]".to_owned()
    }

    fn inline_flattened(_: &ts_rs::Config) -> String {
        "number[]".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkingDays, WorkingDaysError};

    #[test]
    fn monday_to_friday_is_the_default() {
        assert_eq!(WorkingDays::default(), WorkingDays::MONDAY_TO_FRIDAY);
        assert_eq!(WorkingDays::MONDAY_TO_FRIDAY.days(), vec![1, 2, 3, 4, 5]);
        assert_eq!(WorkingDays::MONDAY_TO_FRIDAY.count(), 5);
    }

    #[test]
    fn the_stored_default_is_thirty_one() {
        // The migration's DEFAULT is written as a literal, so this is the test that keeps
        // the two spellings of Monday-to-Friday in step.
        assert_eq!(WorkingDays::MONDAY_TO_FRIDAY.bits(), 31);
    }

    #[test]
    fn a_sunday_to_thursday_week_is_ordinary() {
        let week = WorkingDays::from_days(&[7, 1, 2, 3, 4]).expect("a real week");
        assert_eq!(week.days(), vec![1, 2, 3, 4, 7]);
        assert!(week.contains(7));
        assert!(!week.contains(5));
        assert!(!week.contains(6));
        assert_eq!(week.count(), 5);
    }

    #[test]
    fn a_six_day_week_is_ordinary_too() {
        let week = WorkingDays::from_days(&[1, 2, 3, 4, 5, 6]).expect("a real week");
        assert_eq!(week.count(), 6);
        assert!(week.contains(6));
        assert!(!week.contains(7));
    }

    #[test]
    fn one_day_is_enough() {
        let week = WorkingDays::from_days(&[3]).expect("a real week");
        assert_eq!(week.days(), vec![3]);
        assert_eq!(week.bits(), 0b100);
    }

    #[test]
    fn order_and_repetition_do_not_matter() {
        let tidy = WorkingDays::from_days(&[1, 2, 3, 4, 5]).expect("a real week");
        let messy = WorkingDays::from_days(&[5, 1, 3, 1, 2, 4, 5]).expect("a real week");
        assert_eq!(tidy, messy);
    }

    #[test]
    fn an_empty_week_is_refused() {
        assert_eq!(WorkingDays::from_days(&[]), Err(WorkingDaysError::Empty));
    }

    #[test]
    fn a_number_that_is_not_a_weekday_is_refused() {
        assert_eq!(
            WorkingDays::from_days(&[0]),
            Err(WorkingDaysError::NotAWeekday { weekday: 0 })
        );
        assert_eq!(
            WorkingDays::from_days(&[8]),
            Err(WorkingDaysError::NotAWeekday { weekday: 8 })
        );
        assert_eq!(
            WorkingDays::from_days(&[1, -3]),
            Err(WorkingDaysError::NotAWeekday { weekday: -3 })
        );
    }

    #[test]
    fn every_non_empty_set_round_trips_through_its_bits() {
        for bits in 1..=127i64 {
            let week = WorkingDays::from_bits(bits).expect("in range");
            assert_eq!(week.bits(), bits);
            let rebuilt = WorkingDays::from_days(&week.days()).expect("non-empty");
            assert_eq!(rebuilt, week, "{bits} did not survive days()");
        }
    }

    #[test]
    fn a_stored_set_outside_the_range_is_corruption_not_a_default() {
        // Zero is the important one: substituting Monday-to-Friday here would turn a
        // damaged file into a plausible-looking school.
        assert_eq!(
            WorkingDays::from_bits(0),
            Err(WorkingDaysError::CorruptBits { bits: 0 })
        );
        assert_eq!(
            WorkingDays::from_bits(128),
            Err(WorkingDaysError::CorruptBits { bits: 128 })
        );
        assert_eq!(
            WorkingDays::from_bits(-1),
            Err(WorkingDaysError::CorruptBits { bits: -1 })
        );
    }

    #[test]
    fn contains_is_false_outside_the_week_rather_than_a_panic() {
        let week = WorkingDays::EVERY_DAY;
        assert!(!week.contains(0));
        assert!(!week.contains(8));
        assert!(!week.contains(i64::MIN));
        assert!(!week.contains(i64::MAX));
    }

    #[test]
    fn the_wire_format_is_weekday_numbers_not_bits() {
        let json = serde_json::to_string(&WorkingDays::MONDAY_TO_FRIDAY).expect("serialises");
        assert_eq!(json, "[1,2,3,4,5]");

        let back: WorkingDays = serde_json::from_str("[7,1,2,3,4]").expect("deserialises");
        assert_eq!(back.days(), vec![1, 2, 3, 4, 7]);
    }

    #[test]
    fn the_wire_format_refuses_an_empty_or_impossible_week() {
        assert!(serde_json::from_str::<WorkingDays>("[]").is_err());
        assert!(serde_json::from_str::<WorkingDays>("[9]").is_err());
    }
}
