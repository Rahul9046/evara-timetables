//! The scheduling vocabulary: what a timetable problem *is*, before anything solves one.
//!
//! # Why this is in `evara-core`
//!
//! Four crates have to agree on these types. `evara-db` compiles a school's rows into a
//! [`Problem`] (M4); `evara-constraints` evaluates rules against one (M5); `evara-solver`
//! searches over one (M6); the command layer and the headless CLI carry the result. A
//! vocabulary shared by all four belongs underneath all four, which is `evara-core` — the
//! same argument [`crate::import`] records for the import vocabulary and
//! [`crate::lessons`] for the demand arithmetic.
//!
//! `SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.2 settles it in one line: *"M2 declares these
//! in `evara-core`."*
//!
//! # What this milestone is, and is not
//!
//! **It is a data contract and a set of fixtures.** It is not a solver, not a constraint
//! registry, and not a loader. There is deliberately no `solve`, no `evaluate`, no
//! `resolve_bell_schedule` and no `overlap(a, b)` here: each of those is a named deliverable
//! of a later milestone, and writing a placeholder for one now would mean either a rule
//! implemented twice or a type that appears to work and does not.
//!
//! | Milestone | Deliverable | Consumes from here |
//! | --- | --- | --- |
//! | **M2** | this vocabulary, and [`fixtures`] | — |
//! | M3 | pre-solve diagnostics | [`diagnostics`], every "explicit warning" of §3.2 |
//! | M4 | the `Problem` loader | [`problem`], [`context`], [`availability`], [`population`] |
//! | M5 | the constraint registry, the hard evaluators, and [`overlap`] | [`problem`], [`placement`], [`population`], [`hierarchy`] |
//! | M6 | the construction solver | [`placement`], [`outcome`], [`resolve`], [`rng`] |
//! | M7 | timetable persistence | [`span`], [`outcome`], [`placement`] |
//! | **M8** | timetable viewing and reporting | [`display`], [`span`], [`outcome`], [`diagnostics`] |
//!
//! # The four accepted decisions this vocabulary is shaped by
//!
//! Each is an ADR, and each shows up as a type rather than as a comment.
//!
//! | Decision | Record | Where it appears |
//! | --- | --- | --- |
//! | Generation is scoped to one `(school, cycle, term)`; shared occupancy compares **real time**; cross-school sharing is reported and never guessed | [ADR 0032](../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md) | [`SchedulingContext`], [`clock::TimeInterval`], [`PlacementSpan::clashes_in_time_with`], [`OtherSchool`] |
//! | An activity schedules against **exactly one** bell schedule, resolved by campus compatibility and never guessed | [ADR 0033](../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) | [`BellScheduleResolution`], [`NoBellScheduleCause`], [`clock::DurationPeriods`] |
//! | Expected attendance is a stated number **or it is unknown**; `room-capacity` leaves the first constraint set | [ADR 0034](../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md) | [`ExpectedAttendance`], [`AttendanceFloor`], [`RoomFact::capacity`] |
//! | Group overlap is a **three-state** answer, and *indeterminate* is one of them | [ADR 0035](../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) | [`Overlap`], [`DisjointnessDeclaration`], [`OverlapRelation`], [`GroupMask`] |
//!
//! # Three invariants worth stating once
//!
//! **One activity row is one placeable occurrence.** Four single periods a cycle is four
//! [`ActivityFact`]s. There is no `occurrences_per_cycle` field here and none is added to
//! the persisted model — ADR 0029, and `split_index` keeps its stable meaning.
//!
//! **A duration is a count of periods, not a span of minutes.** The elapsed minutes of a
//! placement are derived from the periods it covers; see [`clock::DurationPeriods`].
//!
//! **Different timeslot ids do not prove non-overlapping times.** The clash key for a shared
//! room, resource or teacher is a wall-clock interval on a cycle day — never an id — because
//! a school running two bell schedules holds two disjoint slot sets over the same minutes.

pub mod availability;
pub mod clock;
pub mod constraints;
pub mod context;
pub mod diagnostics;
pub mod display;
pub mod hierarchy;
pub mod index;
pub mod outcome;
pub mod overlap;
pub mod placement;
pub mod population;
pub mod problem;
pub mod repair;
pub mod resolve;
pub mod rng;
pub mod span;
pub mod validate;

// Fixtures are test and tooling infrastructure, not part of the shipped domain, so they are
// behind a feature the desktop application does not enable. `test` is included so that this
// crate's own `cargo test` always exercises them without a self-dependency; CI runs
// `cargo test --workspace --all-features` and `cargo clippy --all-features`, so the gated
// code is both tested and linted.
#[cfg(any(feature = "fixtures", test))]
pub mod fixtures;

pub use availability::{
    AvailabilityEntity, AvailabilityEntityKind, AvailabilityRule, AvailabilityScope,
    AvailabilityStatus, EffectiveWindow, SlotAvailability, SlotMask, UnavailabilityIndex,
};
pub use clock::{DurationPeriods, TimeInterval, TimeOfDay};
pub use constraints::{
    AffectedSet, CompiledConstraint, ConstraintError, ConstraintLevel, ConstraintOptions,
    ConstraintScope, ConstraintType, Explanation, ScopeRef, StoredConstraint, Violation,
    ViolationKey, ViolationSink,
};
pub use context::{
    BellScheduleFact, BellScheduleResolution, CampusFact, CycleDayFact, CycleFact, CycleKind,
    NoBellScheduleCause, PeriodFact, PeriodKind, SchedulingContext, SchoolFact, TermFact,
};
pub use diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef, Severity,
};
pub use display::{LabelSet, RoomLabel, SectionLabel, StudentLabel};
pub use hierarchy::{Hierarchy, MAX_GROUP_DEPTH};
pub use index::{
    ActivityIx, BellScheduleIx, CampusIx, CycleDayIx, GroupIx, PeriodIx, RoomIx, RoomTypeIx,
    SectionIx, StudentIx, TeacherIx, TimeslotIx,
};
pub use outcome::{
    HardViolationCount, ProblemIdentity, Score, Seed, SolveOutcome, SolveStatistics, SolveStatus,
    Termination, UnplacedCause, UnscheduledActivity, UnverifiedCounts,
};
pub use overlap::{
    Contradiction, GroupMembers, OverlapOutcome, StudentMask, evaluate as evaluate_overlap,
};
pub use placement::{Assignment, Placement, PlacementSpan};
pub use population::{
    AttendanceFloor, DisjointnessDeclaration, ExpectedAttendance, GroupMask, Overlap,
    OverlapRelation,
};
pub use problem::{
    ActivityFact, GroupFact, GroupKind, IntegrityError, OtherSchool, Problem, RoomFact,
    RoomTypeFact, SectionFact, TeacherFact, TimeslotFact,
};
pub use repair::{
    FrozenMismatch, MovableSet, MovableSetError, RepairScope, ScopeError, derive_movable,
    frozen_differences, frozen_only, locked_differences,
};
pub use resolve::{
    MaterialisedGrid, is_compatible, longest_teaching_run, only_campus, runs_on, teaching_slots,
};
pub use rng::Rng;
pub use span::{ResolvedSpan, SpanDefect, resolve_activity_span, resolve_span};
pub use validate::{ValidationSummary, validate};
