//! Dense indices into a [`Problem`](super::Problem)'s tables.
//!
//! # Why indices rather than identifiers
//!
//! [`SOLVER.md`](../../../../docs/SOLVER.md) §2 states the rule: a `Problem` is "entities
//! flattened into `Vec`s and addressed by dense `u32` indexes, not UUIDs. UUID ↔ index
//! translation happens once, in `evara-db`'s loader. The inner loop touches only integers
//! and bitsets." A `Uuid` is sixteen bytes and compares byte-wise; an index is four bytes
//! and indexes an array. Over the tens of millions of comparisons a local search performs,
//! the difference is the difference between a usable product and an unusable one.
//!
//! The identifiers are not thrown away: every fact in a `Problem` carries its own
//! [`Uuid`](uuid::Uuid), because a violation has to name a row the user can click on
//! (`SOLVER.md` §6's `Explanation`) and a solution has to be written back against the rows
//! it came from. The index is the *address*; the identifier is the *identity*.
//!
//! # Why a newtype per table
//!
//! Every one of these would otherwise be a bare `u32`, and a room index would be
//! indistinguishable from a timeslot index to the compiler. Mixing two of them is a bug
//! that produces a plausible wrong answer rather than a crash — the worst kind in a
//! product whose value is that its answers are right. The newtypes cost nothing at runtime
//! and make the mix-up a type error.
//!
//! This is a deliberate departure from `SOLVER.md` §2's sketch, which wrote
//! `Placement { timeslot: u32, room: Option<u32> }`. The shape is unchanged; only the
//! element type is sharper, and `SOLVER.md` is updated to match.

use serde::{Deserialize, Serialize};

/// Declares one dense-index newtype over `u32`.
///
/// Each is `Copy`, totally ordered (so collections of them have a deterministic order), and
/// serialises as a bare number.
macro_rules! dense_index {
    ($(#[$meta:meta])* $name:ident, $what:literal) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            #[doc = concat!("Addresses the ", $what, " at position `ix`.")]
            #[must_use]
            pub const fn new(ix: u32) -> Self {
                Self(ix)
            }

            /// The position, for indexing a `Vec`.
            #[must_use]
            pub const fn get(self) -> usize {
                self.0 as usize
            }

            /// The position as the `u32` it is stored as.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}#{}", $what, self.0)
            }
        }
    };
}

dense_index!(
    /// A campus of the school being solved.
    CampusIx,
    "campus"
);
dense_index!(
    /// A bell schedule (`period_structure`) with slots materialised into this grid.
    ///
    /// The members of `G` in
    /// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
    /// §1 — derived from what is materialised, never from what merely exists.
    BellScheduleIx,
    "bellSchedule"
);
dense_index!(
    /// A period of one bell schedule.
    PeriodIx,
    "period"
);
dense_index!(
    /// A position in the cycle.
    ///
    /// `cycle_day.ordinal` is a day's only scheduling identity. **Nothing reads a weekday.**
    CycleDayIx,
    "cycleDay"
);
dense_index!(
    /// A materialised slot: one `(cycle_day, period)` of one term.
    TimeslotIx,
    "timeslot"
);
dense_index!(
    /// A room that may host a lesson of this school.
    RoomIx,
    "room"
);
dense_index!(
    /// A kind of room, as a requirement and as a room's classification.
    RoomTypeIx,
    "roomType"
);
dense_index!(
    /// A teacher with an active membership at the school being solved.
    TeacherIx,
    "teacher"
);
dense_index!(
    /// A student group of the school being solved.
    ///
    /// The scheduling population is a set of these, never an expanded set of students —
    /// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
    /// §5.
    GroupIx,
    "group"
);
dense_index!(
    /// A student of the school being solved, where the school has entered any.
    ///
    /// **Not part of the scheduling population**, which is a set of groups
    /// ([ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
    /// §5). This index exists for one purpose: rule 3 of §3 proves two groups overlap from a
    /// shared `student_group_member` row, and a *count* of members cannot answer that. It
    /// addresses the member sets the overlap evaluator intersects, and a `Contradiction`
    /// carries one so a report can name the student whose membership breaks a declaration.
    ///
    /// A school with no `student` rows has none of these, which is ADR 0006's case and is
    /// ordinary: rules 1, 4, 5 and 6 decide pairs without a single member row.
    StudentIx,
    "student"
);
dense_index!(
    /// A class section: the thing that has teachers, an audience and a room requirement.
    SectionIx,
    "section"
);
dense_index!(
    /// One placeable occurrence.
    ///
    /// One `activity` row is one of these — ADR 0029. Four single periods a cycle is four
    /// activities, not one activity with a count.
    ActivityIx,
    "activity"
);

#[cfg(test)]
mod tests {
    use super::{ActivityIx, RoomIx, TimeslotIx};

    #[test]
    fn an_index_round_trips_through_its_position() {
        let ix = TimeslotIx::new(7);
        assert_eq!(ix.get(), 7);
        assert_eq!(ix.raw(), 7);
    }

    #[test]
    fn indices_order_by_position_so_collections_are_deterministic() {
        let mut ixs = vec![ActivityIx::new(3), ActivityIx::new(1), ActivityIx::new(2)];
        ixs.sort_unstable();
        assert_eq!(
            ixs,
            vec![ActivityIx::new(1), ActivityIx::new(2), ActivityIx::new(3)]
        );
    }

    #[test]
    fn an_index_names_its_table_when_rendered() {
        // So a diagnostic that carries one reads as "room#4" rather than as a bare 4.
        assert_eq!(RoomIx::new(4).to_string(), "room#4");
        assert_eq!(TimeslotIx::new(0).to_string(), "timeslot#0");
    }

    #[test]
    fn an_index_serialises_as_a_bare_number() {
        let json = serde_json::to_string(&RoomIx::new(12)).expect("an index serialises");
        assert_eq!(json, "12");
        let back: RoomIx = serde_json::from_str(&json).expect("and reads back");
        assert_eq!(back, RoomIx::new(12));
    }
}
