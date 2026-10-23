//! UUID to dense-index translation, done once per load, and the reverse map it leaves
//! behind.
//!
//! # Why this exists as its own module
//!
//! [`SOLVER.md`](../../../../docs/SOLVER.md) §2 states the rule: a
//! [`Problem`](evara_core::scheduling::Problem) is *"entities flattened into `Vec`s and
//! addressed by dense `u32` indexes, not UUIDs. UUID ↔ index translation happens once, in
//! `evara-db`'s loader."* This module is that translation, and it is deliberately the only
//! place in the loader that performs it — eleven tables each needing the same three
//! operations is eleven chances to write a subtly different one.
//!
//! # The index is the address; the identifier is the identity
//!
//! Both directions are kept. The forward direction (identifier to index) is what the loader
//! needs while it is resolving one row's references against another table. The reverse
//! (index to identifier) is what a *diagnostic* needs, because a finding has to name a row
//! the user can click on, and what M7's solution writer will need, because a
//! `timetable_entry` is written against stored identifiers rather than against positions in
//! a `Vec` that no longer exists.
//!
//! Every fact in a `Problem` already carries its own `Uuid`, so the reverse direction is
//! also derivable from the problem. [`ProblemIndex`] holds it anyway, for two reasons: a
//! caller that has an identifier and wants an index would otherwise have to scan, and the
//! map outlives the borrow of any particular problem.
//!
//! # Determinism
//!
//! An index is assigned by *offer order*, and every query that offers rows carries an
//! explicit `ORDER BY` over columns a unique index makes total. Nothing here reads
//! `rowid`, and the lookup table is a [`BTreeMap`] rather than a `HashMap` so that even an
//! accidental iteration of it would be ordered. Two loads of one database snapshot
//! therefore produce identical indices, which `SOLVER.md` §5 calls non-negotiable: the same
//! seed over a different index assignment is a different timetable.

use std::collections::BTreeMap;

use evara_core::scheduling::{
    ActivityIx, BellScheduleIx, CampusIx, CycleDayIx, GroupIx, PeriodIx, RoomIx, RoomTypeIx,
    SectionIx, StudentIx, TeacherIx, TimeslotIx,
};
use uuid::Uuid;

/// A dense index that can be built from, and reduced to, its position.
///
/// `evara-core`'s index newtypes already offer `new` and `raw`; this trait is what lets one
/// generic interner serve all eleven of them rather than one interner per table.
pub(crate) trait DenseIndex: Copy + Ord {
    /// The index at `position`.
    fn at(position: u32) -> Self;
    /// The position this index addresses.
    fn position(self) -> u32;
}

/// Implements [`DenseIndex`] for `evara-core`'s newtypes, which already have the methods.
macro_rules! dense_index_for {
    ($($ty:ty),* $(,)?) => {
        $(impl DenseIndex for $ty {
            fn at(position: u32) -> Self { Self::new(position) }
            fn position(self) -> u32 { self.raw() }
        })*
    };
}

dense_index_for!(
    StudentIx,
    ActivityIx,
    BellScheduleIx,
    CampusIx,
    CycleDayIx,
    GroupIx,
    PeriodIx,
    RoomIx,
    RoomTypeIx,
    SectionIx,
    TeacherIx,
    TimeslotIx,
);

/// One table's identifiers and the dense indices they were given.
///
/// Built by offering rows in query order. Offering the same identifier twice returns the
/// index it already has rather than appending a second entry, which is what makes a teacher
/// employed by this school *and* named by one of its sections a single [`TeacherIx`] —
/// [ADR 0021](../../../../docs/adr/0021-project-level-people-and-school-membership.md)'s
/// requirement that a person is one identity, expressed as a property of the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interner<I> {
    /// Identifiers in index order. Position *is* the index.
    order: Vec<Uuid>,
    /// Identifier to index, for resolving one table's references into another.
    by_id: BTreeMap<Uuid, I>,
}

impl<I: DenseIndex> Default for Interner<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: DenseIndex> Interner<I> {
    /// An empty table.
    pub(crate) const fn new() -> Self {
        Self {
            order: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    /// Gives `id` the next index, or returns the one it already has.
    pub(crate) fn intern(&mut self, id: Uuid) -> I {
        if let Some(existing) = self.by_id.get(&id) {
            return *existing;
        }
        // `Problem` addresses its tables with `u32`, and no school reaches four billion of
        // anything; the saturating conversion matches the one `evara-core` uses at every
        // index construction site rather than introducing a panic this crate would own.
        let index = I::at(u32::try_from(self.order.len()).unwrap_or(u32::MAX));
        self.order.push(id);
        self.by_id.insert(id, index);
        index
    }

    /// The index `id` was given, or `None` if it was never offered.
    pub(crate) fn index_of(&self, id: Uuid) -> Option<I> {
        self.by_id.get(&id).copied()
    }

    /// The identifier at `index`, or `None` if it addresses nothing.
    pub(crate) fn id_at(&self, index: I) -> Option<Uuid> {
        self.order.get(index.position() as usize).copied()
    }

    /// How many rows the table holds.
    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }
}

/// Declares [`ProblemIndex`], its fields and the two accessors each table needs.
///
/// A macro rather than eleven hand-written pairs, for the reason this module exists: the
/// bodies are identical and a typo in one of twenty-two would be invisible.
macro_rules! problem_index {
    ($( $field:ident : $ty:ty, $id_fn:ident, $ix_fn:ident, $what:literal ),* $(,)?) => {
        /// Every identifier a loaded [`Problem`](evara_core::scheduling::Problem) holds, and
        /// the dense index each was given.
        ///
        /// Returned beside the problem rather than inside it. A `Problem` is *"everything a
        /// solve needs, and nothing else"*, and a solve needs indices; identifiers are what
        /// the stages on either side of it need — a diagnostic naming a row, and a solution
        /// written back against the rows it came from.
        #[derive(Debug, Clone, PartialEq, Eq, Default)]
        pub struct ProblemIndex {
            $( pub(crate) $field: Interner<$ty>, )*
        }

        impl ProblemIndex {
            $(
                #[doc = concat!("The stored identifier of the ", $what, " at `index`.")]
                ///
                /// `None` where the index addresses nothing, which
                /// [`Problem::integrity`](evara_core::scheduling::Problem::integrity)
                /// reports as a dangling reference.
                #[must_use]
                pub fn $id_fn(&self, index: $ty) -> Option<Uuid> {
                    self.$field.id_at(index)
                }

                #[doc = concat!("The dense index of the ", $what, " stored as `id`.")]
                ///
                /// `None` where the row is not in this problem — a different school's, or
                /// outside the selected term.
                #[must_use]
                pub fn $ix_fn(&self, id: Uuid) -> Option<$ty> {
                    self.$field.index_of(id)
                }
            )*
        }
    };
}

problem_index!(
    campuses: CampusIx, campus_id, campus_index, "campus",
    cycle_days: CycleDayIx, cycle_day_id, cycle_day_index, "cycle day",
    bell_schedules: BellScheduleIx, bell_schedule_id, bell_schedule_index, "bell schedule",
    periods: PeriodIx, period_id, period_index, "period",
    timeslots: TimeslotIx, timeslot_id, timeslot_index, "timeslot",
    room_types: RoomTypeIx, room_type_id, room_type_index, "room type",
    rooms: RoomIx, room_id, room_index, "room",
    teachers: TeacherIx, teacher_id, teacher_index, "teacher",
    groups: GroupIx, group_id, group_index, "student group",
    students: StudentIx, student_id, student_index, "student",
    sections: SectionIx, section_id, section_index, "class section",
    activities: ActivityIx, activity_id, activity_index, "activity",
);

impl ProblemIndex {
    /// How many rows each table holds, for an assertion that the maps and the problem agree.
    ///
    /// Used by the loader's own self-check: a map one row short of the table it describes
    /// would make the last row's identifier unrecoverable, which is the class of bug
    /// `Problem::integrity` catches on the index side and nothing would catch on this side.
    #[must_use]
    pub fn counts(&self) -> ProblemIndexCounts {
        ProblemIndexCounts {
            campuses: self.campuses.len(),
            cycle_days: self.cycle_days.len(),
            bell_schedules: self.bell_schedules.len(),
            periods: self.periods.len(),
            timeslots: self.timeslots.len(),
            room_types: self.room_types.len(),
            rooms: self.rooms.len(),
            teachers: self.teachers.len(),
            groups: self.groups.len(),
            students: self.students.len(),
            sections: self.sections.len(),
            activities: self.activities.len(),
        }
    }
}

/// How many rows each of [`ProblemIndex`]'s tables holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProblemIndexCounts {
    /// Campuses of the school being loaded.
    pub campuses: usize,
    /// Positions in the cycle.
    pub cycle_days: usize,
    /// Bell schedules the school has defined.
    pub bell_schedules: usize,
    /// Periods across every defined schedule.
    pub periods: usize,
    /// Materialised slots of the selected grid.
    pub timeslots: usize,
    /// Room classifications.
    pub room_types: usize,
    /// Rooms reachable from the school's campuses.
    pub rooms: usize,
    /// Teachers in scope.
    pub teachers: usize,
    /// Student groups of the school.
    pub groups: usize,
    /// Students whose membership the overlap evaluator read.
    ///
    /// Not the school's roll: only those in a group of this school who could be in the room —
    /// ADR 0035 §2's filter. A school with no `student` rows has none, which is ordinary.
    pub students: usize,
    /// Class sections in scope.
    pub sections: usize,
    /// Occurrences to be placed.
    pub activities: usize,
}

#[cfg(test)]
mod tests {
    use evara_core::scheduling::{ActivityIx, TeacherIx};
    use uuid::Uuid;

    use super::{Interner, ProblemIndex};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn indices_are_assigned_in_offer_order() {
        let mut table: Interner<ActivityIx> = Interner::new();
        assert_eq!(table.intern(id(30)), ActivityIx::new(0));
        assert_eq!(table.intern(id(10)), ActivityIx::new(1));
        assert_eq!(table.intern(id(20)), ActivityIx::new(2));
        // Offer order, not identifier order: the query's ORDER BY decides, and a UUIDv7
        // sorts by creation time rather than by anything on screen.
        let order: Vec<Uuid> = (0..3)
            .map(|at| table.id_at(ActivityIx::new(at)).expect("in range"))
            .collect();
        assert_eq!(order, [id(30), id(10), id(20)]);
    }

    #[test]
    fn offering_one_identifier_twice_returns_the_same_index() {
        // The property that makes a teacher who is both employed here and named by a
        // section one `TeacherIx` rather than two.
        let mut table: Interner<TeacherIx> = Interner::new();
        let first = table.intern(id(7));
        let again = table.intern(id(7));
        assert_eq!(first, again);
        assert_eq!(table.len(), 1, "no second entry was appended");
    }

    #[test]
    fn both_directions_agree() {
        let mut table: Interner<TeacherIx> = Interner::new();
        let ix = table.intern(id(42));
        assert_eq!(table.index_of(id(42)), Some(ix));
        assert_eq!(table.id_at(ix), Some(id(42)));
    }

    #[test]
    fn an_unknown_identifier_and_an_out_of_range_index_are_both_none() {
        // Neither is a panic: a row of another school is an ordinary question to ask, and a
        // dangling index is what `Problem::integrity` exists to report.
        let mut table: Interner<TeacherIx> = Interner::new();
        table.intern(id(1));
        assert_eq!(table.index_of(id(2)), None);
        assert_eq!(table.id_at(TeacherIx::new(5)), None);
    }

    #[test]
    fn an_empty_index_reports_empty_counts() {
        let counts = ProblemIndex::default().counts();
        assert_eq!(counts.activities, 0);
        assert_eq!(counts.teachers, 0);
        assert_eq!(counts.timeslots, 0);
    }

    #[test]
    fn the_public_accessors_round_trip_an_identifier() {
        let mut index = ProblemIndex::default();
        let ix = index.activities.intern(id(99));
        assert_eq!(index.activity_index(id(99)), Some(ix));
        assert_eq!(index.activity_id(ix), Some(id(99)));
        assert_eq!(index.counts().activities, 1);
    }
}
