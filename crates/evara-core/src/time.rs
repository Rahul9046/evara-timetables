//! Pure rules about the repeating cycle.
//!
//! # Why this is not in `evara-db` or the interface
//!
//! "Is this cycle finished?" is a question about the scheduling model, not about storage
//! and not about presentation. Answering it in SQL would scatter it across queries;
//! answering it in TypeScript would put a scheduling rule in the view layer, which
//! `CLAUDE.md` names as an architecture bug. It is a pure function over a declared length
//! and a set of ordinals, so it belongs at the bottom of the stack where both the
//! repository and — eventually — the solver can reach it.
//!
//! # No weekday ever appears here
//!
//! A cycle day's ordinal is its whole scheduling identity ([ADR
//! 0011](../../../docs/adr/0011-stable-timeslot-identity.md) §6). Nothing in this module
//! knows what a week is, so a one-day cycle, a six-day rotation and a ten-day fortnight
//! differ only in the numbers passed in.

use serde::{Deserialize, Serialize};

/// Which ordinals of a cycle's declared length actually have a day.
///
/// `cycle.day_count` is the *declared* intended length, and ADR 0011 Q4 settled that the
/// rows need not be complete: adding days one at a time has to be legal, or a cycle could
/// never be built. So a gap is not an error — it is a state the interface has to be able
/// to show, and a state a timetable grid must not be considered finished in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// No `#[ts(export)]`: that would make `cargo test` write TypeScript into the working tree
// as a side effect. `evara-typegen` exports deliberately instead — see ADR 0012.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CycleCoverage {
    /// The cycle's declared length, straight from `cycle.day_count`.
    pub declared_day_count: i64,
    /// How many days exist, however they are numbered.
    pub present: i64,
    /// Declared positions with no day, ascending. Empty for a finished cycle.
    pub missing_ordinals: Vec<i64>,
    /// Days numbered past the declared length, ascending.
    ///
    /// The repository refuses to create one, so this is normally empty. It is reported
    /// rather than ignored because a project edited by a future build, or repaired by
    /// hand in a SQLite browser, could still contain one, and silently dropping it from
    /// the count would make a broken cycle look finished.
    pub beyond_declared: Vec<i64>,
}

impl CycleCoverage {
    /// Works out the coverage of one cycle.
    ///
    /// `ordinals` may arrive in any order and may contain duplicates — the schema's
    /// `UNIQUE (cycle_id, ordinal)` makes duplicates impossible in practice, and counting
    /// them twice would be the one way this could overstate completeness.
    #[must_use]
    pub fn of(declared_day_count: i64, ordinals: &[i64]) -> Self {
        let mut seen: Vec<i64> = ordinals.to_vec();
        seen.sort_unstable();
        seen.dedup();

        let missing_ordinals = (1..=declared_day_count)
            .filter(|ordinal| !seen.contains(ordinal))
            .collect();
        let beyond_declared = seen
            .iter()
            .copied()
            .filter(|ordinal| *ordinal > declared_day_count || *ordinal < 1)
            .collect();

        Self {
            declared_day_count,
            present: i64::try_from(seen.len()).unwrap_or(i64::MAX),
            missing_ordinals,
            beyond_declared,
        }
    }

    /// Whether every declared position has exactly one day and nothing sits outside.
    ///
    /// This is the gate §4 of the Phase 1E brief asks for: contiguous ordinals from 1
    /// through `day_count` before a grid counts as complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.declared_day_count >= 1
            && self.missing_ordinals.is_empty()
            && self.beyond_declared.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::CycleCoverage;

    #[test]
    fn a_full_five_day_cycle_is_complete() {
        let coverage = CycleCoverage::of(5, &[1, 2, 3, 4, 5]);
        assert!(coverage.is_complete());
        assert_eq!(coverage.present, 5);
        assert_eq!(coverage.missing_ordinals, Vec::<i64>::new());
    }

    #[test]
    fn insertion_order_does_not_matter() {
        assert_eq!(
            CycleCoverage::of(5, &[4, 1, 5, 2, 3]),
            CycleCoverage::of(5, &[1, 2, 3, 4, 5])
        );
    }

    #[test]
    fn a_ten_day_fortnight_is_complete_when_full() {
        let ordinals: Vec<i64> = (1..=10).collect();
        assert!(CycleCoverage::of(10, &ordinals).is_complete());
    }

    #[test]
    fn a_one_day_cycle_is_complete_with_one_day() {
        assert!(CycleCoverage::of(1, &[1]).is_complete());
    }

    #[test]
    fn a_six_day_rotation_is_not_special() {
        let coverage = CycleCoverage::of(6, &[1, 2, 3, 4, 5, 6]);
        assert!(coverage.is_complete(), "nothing here knows what a week is");
    }

    #[test]
    fn a_half_built_cycle_names_the_positions_it_is_missing() {
        let coverage = CycleCoverage::of(6, &[1, 2, 5]);
        assert!(!coverage.is_complete());
        assert_eq!(coverage.missing_ordinals, vec![3, 4, 6]);
        assert_eq!(coverage.present, 3);
    }

    #[test]
    fn an_empty_cycle_is_missing_everything() {
        let coverage = CycleCoverage::of(3, &[]);
        assert!(!coverage.is_complete());
        assert_eq!(coverage.missing_ordinals, vec![1, 2, 3]);
        assert_eq!(coverage.present, 0);
    }

    #[test]
    fn the_right_number_of_days_in_the_wrong_places_is_not_complete() {
        // The count alone would say "5 of 5". It is the gap that matters.
        let coverage = CycleCoverage::of(5, &[1, 2, 3, 4, 7]);
        assert!(!coverage.is_complete());
        assert_eq!(coverage.present, 5);
        assert_eq!(coverage.missing_ordinals, vec![5]);
        assert_eq!(coverage.beyond_declared, vec![7]);
    }

    #[test]
    fn duplicates_are_counted_once() {
        let coverage = CycleCoverage::of(3, &[1, 1, 2]);
        assert_eq!(coverage.present, 2);
        assert_eq!(coverage.missing_ordinals, vec![3]);
    }

    #[test]
    fn a_cycle_declared_as_zero_days_is_never_complete() {
        assert!(!CycleCoverage::of(0, &[]).is_complete());
    }
}
