//! The fixtures themselves: a tiny school, a realistic one, an impossible one, and the
//! focused cases the four M1 records each demand.
//!
//! # Where the list comes from
//!
//! It is not invented. `SCHEDULING-ARCHITECTURE-DECISIONS.md` §9 maps fifteen named cases
//! onto the acceptance tables of
//! [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! (thirteen tests),
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! (fourteen) and
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
//! (sixteen, plus a twenty-row truth table), with
//! [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
//! adding fifteen more — and says of all of them: *"every one has an expected outcome
//! written down so M2 implements rather than re-decides."* Each fixture below names the
//! tests it carries.
//!
//! # Every name is invented
//!
//! `CLAUDE.md` rule 3. The schools, campuses, teachers and subjects here belong to nobody.
//! The surnames are constructed from English place-name elements and were checked against no
//! register of real people, because none was consulted: they are strings chosen to look like
//! names and to be obviously synthetic in aggregate.

use uuid::Uuid;

use super::builder::{
    GroupSpec, PeriodSpec, RoomFactSpec, SchoolBuilder, SectionSpec, TeacherFactSpec,
};
use super::{
    AttendanceExpectation, Expectations, ExpectedDiagnostic, KnownFeasible, OverlapExpectation,
    SchedulingFixture, SharedIdentity,
};
use crate::scheduling::availability::{AvailabilityEntity, AvailabilityStatus};
use crate::scheduling::clock::TimeInterval;
use crate::scheduling::context::{
    BellScheduleResolution, CycleKind, NoBellScheduleCause, PeriodKind,
};
use crate::scheduling::diagnostics::{DiagnosticCode, EntityKind};
use crate::scheduling::index::{
    ActivityIx, BellScheduleIx, CampusIx, GroupIx, RoomIx, RoomTypeIx, SectionIx, TeacherIx,
    TimeslotIx,
};
use crate::scheduling::outcome::Seed;
use crate::scheduling::placement::PlacementSpan;
use crate::scheduling::population::{DisjointnessDeclaration, ExpectedAttendance, Overlap};
use crate::scheduling::problem::{GroupKind, Problem};

/// Every fixture the catalogue can generate.
///
/// Addressed by a stable kebab-case name, which `evara fixture` takes on the command line
/// and which a future benchmark baseline would be filed under. Added to, never renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixtureKind {
    /// A minimal school small enough to check by hand, with a feasible arrangement attached.
    Tiny,
    /// A realistic synthetic school at the documented scale: 80 teachers, 1 000 students at
    /// group level, 500 occurrences.
    Medium,
    /// An occurrence longer than any run of teaching periods. Conclusively impossible.
    InfeasibleDuration,
    /// A required kind of room that no room is. Also conclusively impossible.
    InfeasibleRoomType,
    /// One campus, a school-wide bell schedule, sections naming no campus.
    SingleCampusSchoolWideBells,
    /// Two campuses whose bell schedules have identical timings.
    TwoCampusesIdenticalBells,
    /// Two campuses whose bell schedules have different timings, sharing a hall and a
    /// teacher.
    TwoCampusesDifferentBells,
    /// A section compatible with two materialised schedules at once.
    AmbiguousBellSchedule,
    /// A schedule that exists and was never materialised into the grid.
    UnmaterialisedBellSchedule,
    /// A cycle and term with no slots at all.
    EmptyGrid,
    /// One teacher, one identity, two schools.
    SharedTeacherAcrossSchools,
    /// Two schools whose academic years do not line up, sharing a teacher.
    DifferentSchoolCalendars,
    /// Two sibling groups with a shared member and no declaration.
    OverlappingGroupMembership,
    /// Two sibling groups their parent declares disjoint, with no shared member.
    DisjointSiblingGroups,
    /// A school that has answered nothing about its groups.
    UnknownGroupOverlap,
    /// A declaration of disjointness that a member row contradicts.
    DeclaredDisjointContradicted,
    /// Two sets whose declared sizes cannot both fit in their parent.
    PigeonholeOverlap,
    /// One cohort divided two ways, which a tree alone cannot express.
    TwoDivisionsOfOneCohort,
    /// Members entered only at the leaves of an eight-level hierarchy.
    DeepGroupHierarchy,
    /// A section with a stated attendance that its groups' sizes disagree with.
    KnownAttendance,
    /// Sections whose attendance cannot be derived at all.
    UnknownAttendance,
    /// A section with named students and no attending group.
    RosterWithoutGroup,
    /// Availability rules that name nothing schedulable, and a school with none at all.
    MissingAvailabilityContext,
    /// A six-day rotation, to prove nothing reads a weekday.
    CustomRotatingCycle,
}

impl FixtureKind {
    /// Every fixture, in catalogue order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Tiny,
            Self::Medium,
            Self::InfeasibleDuration,
            Self::InfeasibleRoomType,
            Self::SingleCampusSchoolWideBells,
            Self::TwoCampusesIdenticalBells,
            Self::TwoCampusesDifferentBells,
            Self::AmbiguousBellSchedule,
            Self::UnmaterialisedBellSchedule,
            Self::EmptyGrid,
            Self::SharedTeacherAcrossSchools,
            Self::DifferentSchoolCalendars,
            Self::OverlappingGroupMembership,
            Self::DisjointSiblingGroups,
            Self::UnknownGroupOverlap,
            Self::DeclaredDisjointContradicted,
            Self::PigeonholeOverlap,
            Self::TwoDivisionsOfOneCohort,
            Self::DeepGroupHierarchy,
            Self::KnownAttendance,
            Self::UnknownAttendance,
            Self::RosterWithoutGroup,
            Self::MissingAvailabilityContext,
            Self::CustomRotatingCycle,
        ]
    }

    /// The stable name this fixture is addressed by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Medium => "medium",
            Self::InfeasibleDuration => "infeasible-duration",
            Self::InfeasibleRoomType => "infeasible-room-type",
            Self::SingleCampusSchoolWideBells => "single-campus-school-wide-bells",
            Self::TwoCampusesIdenticalBells => "two-campuses-identical-bells",
            Self::TwoCampusesDifferentBells => "two-campuses-different-bells",
            Self::AmbiguousBellSchedule => "ambiguous-bell-schedule",
            Self::UnmaterialisedBellSchedule => "unmaterialised-bell-schedule",
            Self::EmptyGrid => "empty-grid",
            Self::SharedTeacherAcrossSchools => "shared-teacher-across-schools",
            Self::DifferentSchoolCalendars => "different-school-calendars",
            Self::OverlappingGroupMembership => "overlapping-group-membership",
            Self::DisjointSiblingGroups => "disjoint-sibling-groups",
            Self::UnknownGroupOverlap => "unknown-group-overlap",
            Self::DeclaredDisjointContradicted => "declared-disjoint-contradicted",
            Self::PigeonholeOverlap => "pigeonhole-overlap",
            Self::TwoDivisionsOfOneCohort => "two-divisions-of-one-cohort",
            Self::DeepGroupHierarchy => "deep-group-hierarchy",
            Self::KnownAttendance => "known-attendance",
            Self::UnknownAttendance => "unknown-attendance",
            Self::RosterWithoutGroup => "roster-without-group",
            Self::MissingAvailabilityContext => "missing-availability-context",
            Self::CustomRotatingCycle => "custom-rotating-cycle",
        }
    }

    /// One sentence on what it is for, and which acceptance tests it carries.
    ///
    /// Looked up in [`PURPOSES`] rather than matched, so the prose reads as prose instead of
    /// as a hundred-line `match`. A kind missing from the table returns an empty string,
    /// which `every_fixture_states_its_purpose` fails on.
    #[must_use]
    pub fn purpose(self) -> &'static str {
        let name = self.name();
        PURPOSES
            .iter()
            .find(|(listed, _)| *listed == name)
            .map_or("", |(_, purpose)| *purpose)
    }

    /// Generates the fixture.
    #[must_use]
    pub fn generate(self, seed: u64) -> SchedulingFixture {
        match self {
            Self::Tiny => tiny(seed),
            Self::Medium => medium(seed),
            Self::InfeasibleDuration => infeasible_duration(seed),
            Self::InfeasibleRoomType => infeasible_room_type(seed),
            Self::SingleCampusSchoolWideBells => single_campus_school_wide_bells(seed),
            Self::TwoCampusesIdenticalBells => two_campuses_identical_bells(seed),
            Self::TwoCampusesDifferentBells => two_campuses_different_bells(seed),
            Self::AmbiguousBellSchedule => ambiguous_bell_schedule(seed),
            Self::UnmaterialisedBellSchedule => unmaterialised_bell_schedule(seed),
            Self::EmptyGrid => empty_grid(seed),
            Self::SharedTeacherAcrossSchools => shared_teacher_across_schools(seed),
            Self::DifferentSchoolCalendars => different_school_calendars(seed),
            Self::OverlappingGroupMembership => overlapping_group_membership(seed),
            Self::DisjointSiblingGroups => disjoint_sibling_groups(seed),
            Self::UnknownGroupOverlap => unknown_group_overlap(seed),
            Self::DeclaredDisjointContradicted => declared_disjoint_contradicted(seed),
            Self::PigeonholeOverlap => pigeonhole_overlap(seed),
            Self::TwoDivisionsOfOneCohort => two_divisions_of_one_cohort(seed),
            Self::DeepGroupHierarchy => deep_group_hierarchy(seed),
            Self::KnownAttendance => known_attendance(seed),
            Self::UnknownAttendance => unknown_attendance(seed),
            Self::RosterWithoutGroup => roster_without_group(seed),
            Self::MissingAvailabilityContext => missing_availability_context(seed),
            Self::CustomRotatingCycle => custom_rotating_cycle(seed),
        }
    }

    /// Looks a fixture up by its stable name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().iter().copied().find(|k| k.name() == name)
    }
}

/// What each fixture is for, and which acceptance tests it carries.
///
/// Keyed by [`FixtureKind::name`]. A table rather than a `match` so that a reader comparing
/// the catalogue against the ADRs' acceptance tables can do it in one place.
const PURPOSES: &[(&str, &str)] = &[
    (
        "tiny",
        "A minimal school with a feasible arrangement attached: one campus, \
                 a campus-scoped bell schedule, a double period and a laboratory \
                 requirement. ADR 0033 test 1 (clause 3), ADR 0032 test 1.",
    ),
    (
        "medium",
        "A realistic school at the documented scale — 80 teachers, 1 000 \
                 students at group level, 500 occurrences — for loader, benchmark and \
                 regression work.",
    ),
    (
        "infeasible-duration",
        "A three-period occurrence where no run of teaching \
                 periods is longer than two. ADR 0033 §5 and test 10.",
    ),
    (
        "infeasible-room-type",
        "A section requiring a laboratory at a school with no \
                 bookable laboratory. SOLVER.md §8 Layer 1, room-type demand against supply.",
    ),
    (
        "single-campus-school-wide-bells",
        "One campus and a school-wide schedule, so \
                 every section resolves under ADR 0033 §2 clause 1. ADR 0033 test 2.",
    ),
    (
        "two-campuses-identical-bells",
        "Two campuses on two schedules with identical \
                 timings: two disjoint slot sets over the same minutes. ADR 0033 test 4.",
    ),
    (
        "two-campuses-different-bells",
        "Two campuses on different timings, sharing a \
                 hall and a teacher — the case where interval comparison and timeslot \
                 equality disagree. ADR 0032 tests 2, 3, 5; ADR 0033 test 5.",
    ),
    (
        "ambiguous-bell-schedule",
        "A school-wide schedule and the section's own \
                 campus schedule both materialised. ADR 0033 tests 6 and 7.",
    ),
    (
        "unmaterialised-bell-schedule",
        "A schedule defined and never materialised, \
                 so it is not in G and causes no ambiguity. ADR 0033 test 8.",
    ),
    (
        "empty-grid",
        "No slots for this cycle and term, reported once for the solve \
                 rather than once per section. ADR 0033 test 9.",
    ),
    (
        "shared-teacher-across-schools",
        "One teacher identity in two schools' \
                 problems: warned, never compared. ADR 0032 tests 6 and 7.",
    ),
    (
        "different-school-calendars",
        "Two schools with different terms sharing a \
                 teacher, and one with no calendar coverage at all. ADR 0032 tests 10 and 11.",
    ),
    (
        "overlapping-group-membership",
        "Two siblings sharing three students with no \
                 declaration anywhere: proved by the member rows. ADR 0035 rule 3, test 4.",
    ),
    (
        "disjoint-sibling-groups",
        "Siblings their parent declares disjoint, with no \
                 shared member: concurrency is legal. ADR 0035 rule 6, test 6.",
    ),
    (
        "unknown-group-overlap",
        "Every declaration unanswered, so the commonest pair \
                 in any school is indeterminate. ADR 0035 truth table 8, test 2.",
    ),
    (
        "declared-disjoint-contradicted",
        "A member row against a declaration of \
                 disjointness: facts win and the contradiction is named. ADR 0035 truth \
                 table 9, test 5.",
    ),
    (
        "pigeonhole-overlap",
        "Two sets of 70 inside a cohort of 120, with no member \
                 row anywhere. ADR 0035 rule 5, truth table 13, test 8.",
    ),
    (
        "two-divisions-of-one-cohort",
        "Classes and subject sets under one cohort, the \
                 case a flat sibling rule gets wrong. ADR 0035 §4, test 12.",
    ),
    (
        "deep-group-hierarchy",
        "Eight levels with members only at the leaves, so the \
                 subtree union is required rather than a one-step parent read. ADR 0035 \
                 test 14.",
    ),
    (
        "known-attendance",
        "A stated attendance of 24 against a group of 28: the \
                 explicit value wins and the difference is advice. ADR 0034 tests 1 and 2.",
    ),
    (
        "unknown-attendance",
        "Attendance that cannot be derived — no size, \
                 indeterminate overlap, a parent and its child — and is never zero. ADR 0034 \
                 tests 3, 4, 5, 7.",
    ),
    (
        "roster-without-group",
        "Twenty named students and no attending group, so \
                 nothing protects them. ADR 0035 §7 test 10; ADR 0034 test 6.",
    ),
    (
        "missing-availability-context",
        "A school with no availability at all beside \
                 rules naming a deleted entity and an unschedulable kind. V11's default, and \
                 two findings.",
    ),
    (
        "custom-rotating-cycle",
        "A six-day rotation with two schedules: resolution \
                 and overlap read no weekday. ADR 0033 test 14, ADR 0035 test 16.",
    ),
];

/// Generates one fixture by name, or `None` if nothing is called that.
#[must_use]
pub fn fixture(name: &str, seed: u64) -> Option<SchedulingFixture> {
    FixtureKind::from_name(name).map(|kind| kind.generate(seed))
}

/// Every fixture's stable name, in catalogue order.
#[must_use]
pub fn fixture_names() -> Vec<&'static str> {
    FixtureKind::all().iter().map(|k| k.name()).collect()
}

/// Generates every fixture at one seed.
///
/// The `medium` fixture dominates the cost; everything else is small. Used by the tests that
/// assert properties over the whole catalogue.
#[must_use]
pub fn all_fixtures(seed: u64) -> Vec<SchedulingFixture> {
    FixtureKind::all()
        .iter()
        .map(|k| k.generate(seed))
        .collect()
}

// =======================================================================================
// Synthetic names
// =======================================================================================

/// Invented surnames. See this module's header on why they are safe to use.
const SURNAMES: &[&str] = &[
    "Ashdown",
    "Brightwater",
    "Calloway",
    "Donnelby",
    "Eastmere",
    "Fernhollow",
    "Garrowick",
    "Haverstone",
    "Inglebrook",
    "Jarrowfield",
    "Kettleby",
    "Larkmoor",
];

/// Invented school names.
const SCHOOLS: &[&str] = &[
    "Northgate Academy",
    "Southmoor College",
    "Westbarrow School",
    "Eastvale Academy",
];

/// A synthetic teacher name that is unique for every `n` below 312.
fn teacher_name(n: usize) -> String {
    let initial = char::from(b'A' + u8::try_from(n % 26).unwrap_or(0));
    let surname = SURNAMES[(n / 26) % SURNAMES.len()];
    format!("{initial}. {surname}")
}

/// A synthetic school name, varied by seed so two seeds are distinguishable at a glance.
fn school_name(seed: u64, which: usize) -> &'static str {
    SCHOOLS[(usize::try_from(seed % 4).unwrap_or(0) + which) % SCHOOLS.len()]
}

/// The subjects the larger fixtures teach.
const SUBJECTS: &[&str] = &[
    "Mathematics",
    "English",
    "Science",
    "Humanities",
    "Creative Arts",
];

// =======================================================================================
// Bell schedules
// =======================================================================================

/// Four teaching periods in two runs of two, split by a mid-morning break.
///
/// The longest unbroken run of teaching periods is **2**, so a double period fits and a
/// triple does not — which is what makes this schedule useful for both the tiny fixture and
/// the infeasible one.
fn two_run_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("P1", "08:30", "09:15"),
        PeriodSpec::teaching("P2", "09:20", "10:05"),
        PeriodSpec::non_teaching("Break", "10:05", "10:25", PeriodKind::Break),
        PeriodSpec::teaching("P3", "10:25", "11:10"),
        PeriodSpec::teaching("P4", "11:15", "12:00"),
    ]
}

/// The same four teaching periods, shifted thirty minutes later.
///
/// Used by the two-campus fixtures: every period's minutes differ from [`two_run_day`]'s
/// while the ordinals are identical, which is precisely the configuration where comparing
/// `timeslot_id`s gives the wrong answer and comparing intervals gives the right one.
fn two_run_day_shifted() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("P1", "09:00", "09:45"),
        PeriodSpec::teaching("P2", "09:50", "10:35"),
        PeriodSpec::non_teaching("Break", "10:35", "10:55", PeriodKind::Break),
        PeriodSpec::teaching("P3", "10:55", "11:40"),
        PeriodSpec::teaching("P4", "11:45", "12:30"),
    ]
}

/// Six teaching periods in runs of three, two and one.
///
/// The medium fixture's schedule. Thirty teaching slots over a five-day cycle, which is the
/// figure every counting argument in that fixture's notes is stated against.
fn six_period_day() -> Vec<PeriodSpec> {
    vec![
        PeriodSpec::teaching("P1", "08:30", "09:20"),
        PeriodSpec::teaching("P2", "09:25", "10:15"),
        PeriodSpec::teaching("P3", "10:20", "11:10"),
        PeriodSpec::non_teaching("Break", "11:10", "11:30", PeriodKind::Break),
        PeriodSpec::teaching("P4", "11:30", "12:20"),
        PeriodSpec::teaching("P5", "12:25", "13:15"),
        PeriodSpec::non_teaching("Lunch", "13:15", "14:00", PeriodKind::Break),
        PeriodSpec::teaching("P6", "14:00", "14:50"),
    ]
}

/// Weekday labels, which are display only — `ordinal` is a day's scheduling identity.
const WEEKDAYS: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

// =======================================================================================
// Helpers shared by the small fixtures
// =======================================================================================

/// A span over slots a fixture has chosen, reading their intervals from the finished problem.
///
/// # Panics
///
/// If `slots` is empty or names a slot the problem does not hold, which is a fixture bug.
fn span(problem: &Problem, slots: &[TimeslotIx], room: Option<RoomIx>) -> PlacementSpan {
    let intervals: Vec<TimeInterval> = slots
        .iter()
        .map(|s| problem.timeslots[s.get()].interval)
        .collect();
    let day = problem.timeslots[slots[0].get()].cycle_day;
    PlacementSpan::over(day, slots.to_vec(), &intervals, room)
        .expect("a fixture's arrangement covers at least one slot")
}

/// The slot at `(day, position)` of a schedule materialised as one block.
///
/// `materialise` emits `(day, period)` in order, so the returned list is indexed by
/// `day * periods_per_day + position`. Named rather than inlined because getting it wrong
/// silently produces an arrangement on the wrong day.
fn at(slots: &[TimeslotIx], periods_per_day: usize, day: usize, position: usize) -> TimeslotIx {
    slots[day * periods_per_day + position]
}

/// The positions of [`two_run_day`]'s four teaching periods within its five periods.
const TWO_RUN_TEACHING: [usize; 4] = [0, 1, 3, 4];

/// A school with one campus, one campus-scoped bell schedule and a five-day week.
///
/// The scaffold most of the focused fixtures start from, so each one's body contains only
/// what it is actually about.
struct OneCampusSchool {
    builder: SchoolBuilder,
    campus: CampusIx,
    schedule: BellScheduleIx,
    slots: Vec<TimeslotIx>,
}

fn one_campus_school(seed: u64, which: usize) -> OneCampusSchool {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, which),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let campus = builder.add_campus("Main Campus");
    builder.add_cycle_days(WEEKDAYS, 5);
    let schedule = builder.add_bell_schedule("Standard Day", Some(campus), &two_run_day());
    let slots = builder.materialise(schedule);
    OneCampusSchool {
        builder,
        campus,
        schedule,
        slots,
    }
}

/// Resolves to one named schedule, stated rather than derived.
const fn resolved(schedule: BellScheduleIx) -> BellScheduleResolution {
    BellScheduleResolution::Resolved { schedule }
}

// =======================================================================================
// tiny
// =======================================================================================

/// A minimal school with a feasible arrangement attached.
///
/// # Structure
///
/// One school, one campus, one five-day weekly cycle, one term, one **campus-scoped** bell
/// schedule of four teaching periods in two runs of two. Four teachers, three rooms (two
/// general and one laboratory), three student groups, four class sections, ten occurrences
/// of which two are double periods and four require a laboratory.
///
/// # What it proves
///
/// The sections name **no campus**, and the only bell schedule is scoped to the one campus
/// there is — so every section resolves under
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §2 **clause 3**, which is the clause that makes the commonest configuration in the product
/// work and the one most at risk of being "simplified" away. That is ADR 0033 test 1.
///
/// Group overlap is fully decided: `Year 7` contains `7A` and `7B` by the parent edge (rule
/// 4), and the two classes are disjoint because the cohort declares its children so (rule
/// 6). So this fixture is also ADR 0035 test 1 — a group-only school with **zero student
/// rows** whose clash detection is nonetheless complete, which is the property
/// [ADR 0006](../../../../docs/adr/0006-group-based-scheduling-first.md) is most at risk of
/// losing.
///
/// # The arrangement
///
/// Attached, and checkable. The laboratory holds Science on four distinct days; the two
/// Mathematics sections sit in the afternoon run in separate rooms; no teacher, room or
/// overlapping group pair is double-booked. See [`KnownFeasible`] for what the claim does and
/// does not mean.
/// The indices the tiny fixture's arrangement and expectations refer back to.
///
/// Returned by [`tiny_school`] so that building the school, choosing the arrangement and
/// stating the expectations are three readable functions rather than one long one.
struct TinyParts {
    slots: Vec<TimeslotIx>,
    room_a: RoomIx,
    room_b: RoomIx,
    lab: RoomIx,
    year7: GroupIx,
    seven_a: GroupIx,
    seven_b: GroupIx,
    maths_a: SectionIx,
    science_b: SectionIx,
}

fn tiny(seed: u64) -> SchedulingFixture {
    let (problem, parts) = tiny_school(seed);
    let spans = tiny_arrangement(&problem, &parts);

    SchedulingFixture {
        name: FixtureKind::Tiny.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::Tiny.purpose(),
        expectations: tiny_expectations(&parts, spans),
        problems: vec![problem],
    }
}

/// Builds the school.
fn tiny_school(seed: u64) -> (Problem, TinyParts) {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        slots,
    } = one_campus_school(seed, 0);

    let general = builder.add_room_type("General Classroom");
    let laboratory = builder.add_room_type("Science Laboratory");
    let room_a = builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let room_b = builder.add_room(
        RoomFactSpec::new("Room 2", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let lab = builder.add_room(
        RoomFactSpec::new("Laboratory 1", 24)
            .on_campus(campus)
            .of_type(laboratory),
    );

    let teachers: Vec<_> = (0..4)
        .map(|n| builder.add_teacher(TeacherFactSpec::new(&teacher_name(n))))
        .collect();

    let year7 = builder.add_group(
        GroupSpec::new("Year 7", GroupKind::Cohort)
            .sized(52)
            .children(DisjointnessDeclaration::Disjoint),
    );
    let seven_a = builder.add_group(
        GroupSpec::new("7A", GroupKind::Class)
            .sized(26)
            .child_of(year7),
    );
    let seven_b = builder.add_group(
        GroupSpec::new("7B", GroupKind::Class)
            .sized(26)
            .child_of(year7),
    );

    builder.declare_overlap(year7, year7, Overlap::Overlaps);
    builder.declare_overlap(seven_a, seven_a, Overlap::Overlaps);
    builder.declare_overlap(seven_b, seven_b, Overlap::Overlaps);
    builder.declare_overlap(year7, seven_a, Overlap::Overlaps);
    builder.declare_overlap(year7, seven_b, Overlap::Overlaps);
    builder.declare_overlap(seven_a, seven_b, Overlap::Disjoint);

    let lesson = |name: &str, teacher, group, room_type| {
        let mut spec = SectionSpec::new(name)
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 26 })
            .declares_periods(3);
        if let Some(room_type) = room_type {
            spec = spec.requires_room_type(room_type);
        }
        spec
    };

    let maths_a = builder.add_section(lesson("Mathematics 7A", teachers[0], seven_a, None));
    let maths_b = builder.add_section(lesson("Mathematics 7B", teachers[1], seven_b, None));
    let science_a =
        builder.add_section(lesson("Science 7A", teachers[2], seven_a, Some(laboratory)));
    let science_b =
        builder.add_section(lesson("Science 7B", teachers[3], seven_b, Some(laboratory)));

    // Three singles each for Mathematics; a double and a single each for Science. Ten
    // occurrences, twelve periods of demand — a sum over rows and never a product.
    builder.add_activities(maths_a, &[1, 1, 1]);
    builder.add_activities(maths_b, &[1, 1, 1]);
    builder.add_activities(science_a, &[2, 1]);
    builder.add_activities(science_b, &[2, 1]);

    (
        builder.finish(),
        TinyParts {
            slots,
            room_a,
            room_b,
            lab,
            year7,
            seven_a,
            seven_b,
            maths_a,
            science_b,
        },
    )
}

/// The arrangement, chosen so that every resource is free when it is used.
///
/// Positions are indices into the day's five periods: 0 and 1 are the morning run, 3 and 4
/// the afternoon one, and 2 is the break, which nothing is placed in. The order of the plan
/// matches the order the activities were added, which is what makes the `n`th entry the
/// `n`th activity.
fn tiny_arrangement(problem: &Problem, parts: &TinyParts) -> Vec<(ActivityIx, PlacementSpan)> {
    const PERIODS_PER_DAY: usize = 5;

    let plan: &[(usize, &[usize], Option<RoomIx>)] = &[
        // Mathematics 7A — afternoon, Room 1, three separate days.
        (0, &[3], Some(parts.room_a)),
        (1, &[3], Some(parts.room_a)),
        (2, &[3], Some(parts.room_a)),
        // Mathematics 7B — afternoon, Room 2, the same three days but a later period.
        (0, &[4], Some(parts.room_b)),
        (1, &[4], Some(parts.room_b)),
        (2, &[4], Some(parts.room_b)),
        // Science 7A — a double in the morning run on Monday, a single on Tuesday.
        (0, &[0, 1], Some(parts.lab)),
        (1, &[0], Some(parts.lab)),
        // Science 7B — a double on Wednesday, a single on Thursday.
        (2, &[0, 1], Some(parts.lab)),
        (3, &[0], Some(parts.lab)),
    ];

    plan.iter()
        .enumerate()
        .map(|(n, (day, positions, room))| {
            let chosen: Vec<TimeslotIx> = positions
                .iter()
                .map(|position| at(&parts.slots, PERIODS_PER_DAY, *day, *position))
                .collect();
            (
                ActivityIx::new(u32::try_from(n).unwrap_or(0)),
                span(problem, &chosen, *room),
            )
        })
        .collect()
}

/// What the tiny fixture claims.
fn tiny_expectations(parts: &TinyParts, spans: Vec<(ActivityIx, PlacementSpan)>) -> Expectations {
    Expectations {
        overlaps: vec![
            overlap_is(
                0,
                parts.year7,
                parts.seven_a,
                Overlap::Overlaps,
                "rule 4 — containment",
            ),
            overlap_is(
                0,
                parts.year7,
                parts.seven_b,
                Overlap::Overlaps,
                "rule 4 — containment",
            ),
            overlap_is(
                0,
                parts.seven_a,
                parts.seven_b,
                Overlap::Disjoint,
                "rule 6 — the cohort declares its children disjoint",
            ),
        ],
        attendance: vec![
            attendance_is(
                0,
                parts.maths_a,
                ExpectedAttendance::Known { people: 26 },
                "explicit value",
            ),
            attendance_is(
                0,
                parts.science_b,
                ExpectedAttendance::Known { people: 26 },
                "explicit value",
            ),
        ],
        diagnostics: Vec::new(),
        shared_identities: Vec::new(),
        feasible: Some(KnownFeasible { problem: 0, spans }),
        notes: vec![
            "Zero student rows: every overlap answer comes from declared sizes, the parent              edge and one declaration per parent. ADR 0006 survives."
                .to_owned(),
            "Sections name no campus and the only schedule is campus-scoped, so every one              resolves under ADR 0033 §2 clause 3."
                .to_owned(),
            "Pre-flight produces no finding at all: attendance is stated for every section,              every declaration is answered, nothing is shared with another school, and every              section has a teacher, an audience and a candidate room."
                .to_owned(),
        ],
    }
}

// =======================================================================================
// medium
// =======================================================================================

/// A realistic synthetic school at the scale the roadmap names.
///
/// # Documented scale
///
/// | Figure | Value |
/// | --- | --- |
/// | Teachers | 80 |
/// | Students, at group level | 1 000 — 25 classes of 40, declared, with **no student rows** |
/// | Student groups | 30 — five cohorts and twenty-five classes |
/// | Class sections | 125 — each class takes all five subjects |
/// | Occurrences | **500** — four per section |
/// | Rooms | 40, across four kinds |
/// | Teaching slots | 30 — six periods over a five-day cycle |
///
/// # No individual students, deliberately
///
/// A thousand `student` rows would make the fixture slower to build, larger to serialise and
/// no more useful: `student-clash` reads the overlap relation over **groups**, and ADR 0035
/// §5 is explicit that expanding to leaf student sets is the wrong compiled form. The
/// population is declared on the groups, which is the configuration ADR 0006 exists to
/// protect and the one most schools are actually in.
///
/// # Feasible by design, and not proven
///
/// The counting arguments are stated and each has headroom: every class needs 20 of its 30
/// slots; every teacher carries at most 7 occurrences against 30 slots; peak demand is about
/// 17 concurrent lessons against 40 rooms. **No arrangement is attached and none is
/// claimed.** Only a solver can say a timetable exists — M6's job — and
/// `evara_core::lessons`' rule applies here too: never report that a demand *can* be
/// scheduled.
///
/// Some sections carry availability restrictions and some courses require a kind of room, so
/// the fixture exercises those paths at scale rather than only in the focused cases.
/// The medium fixture's documented scale, in one place so the notes and the tests read the
/// same numbers the generator uses.
mod medium_scale {
    /// Year levels.
    pub(super) const YEAR_LEVELS: usize = 5;
    /// Classes in each year level.
    pub(super) const CLASSES_PER_YEAR: usize = 5;
    /// Students in each class, declared on the group.
    pub(super) const CLASS_SIZE: i64 = 40;
    /// Teachers with an active membership.
    pub(super) const TEACHERS: usize = 80;
    /// Rooms, across four kinds.
    pub(super) const ROOMS: usize = 40;
    /// Occurrences per class section.
    pub(super) const OCCURRENCES_PER_SECTION: usize = 4;
    /// Teaching periods in the bell schedule.
    pub(super) const TEACHING_PERIODS: usize = 6;
}

/// The kinds of room a medium-fixture course can require.
///
/// Only the two specialist kinds: a general classroom and the hall are classifications a room
/// carries and nothing requires, so naming them here would be a field nothing reads.
struct MediumRoomTypes {
    laboratory: RoomTypeIx,
    studio: RoomTypeIx,
}

/// What the medium fixture's expectations refer back to.
struct MediumParts {
    cohorts: Vec<GroupIx>,
    classes: Vec<GroupIx>,
    sections: Vec<SectionIx>,
}

fn medium(seed: u64) -> SchedulingFixture {
    use medium_scale::{CLASS_SIZE, CLASSES_PER_YEAR};

    let (problem, parts) = medium_school(seed);

    SchedulingFixture {
        name: FixtureKind::Medium.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::Medium.purpose(),
        expectations: Expectations {
            overlaps: vec![
                overlap_is(
                    0,
                    parts.cohorts[0],
                    parts.classes[0],
                    Overlap::Overlaps,
                    "rule 4 — containment",
                ),
                overlap_is(
                    0,
                    parts.classes[0],
                    parts.classes[1],
                    Overlap::Disjoint,
                    "rule 6 — the cohort declares its children disjoint",
                ),
                overlap_is(
                    0,
                    parts.classes[0],
                    parts.classes[CLASSES_PER_YEAR],
                    Overlap::Indeterminate,
                    "rule 7 — two year groups share no stated relation, and unrelated is \
                     not disjoint",
                ),
            ],
            attendance: vec![attendance_is(
                0,
                parts.sections[0],
                ExpectedAttendance::Known { people: CLASS_SIZE },
                "explicit value",
            )],
            diagnostics: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: medium_notes(),
        },
        problems: vec![problem],
    }
}

/// Builds the school.
fn medium_school(seed: u64) -> (Problem, MediumParts) {
    use medium_scale::{TEACHERS, TEACHING_PERIODS};

    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 2),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let campus = builder.add_campus("Main Campus");
    builder.add_cycle_days(WEEKDAYS, 5);
    let schedule = builder.add_bell_schedule("Standard Day", Some(campus), &six_period_day());
    let slots = builder.materialise(schedule);

    let types = medium_rooms(&mut builder, campus);
    let teachers: Vec<_> = (0..TEACHERS)
        .map(|n| builder.add_teacher(TeacherFactSpec::new(&teacher_name(n))))
        .collect();
    let (cohorts, classes) = medium_groups(&mut builder);
    let sections = medium_lessons(&mut builder, schedule, &types, &teachers, &classes);
    medium_availability(&mut builder, &teachers, &slots, TEACHING_PERIODS);

    (
        builder.finish(),
        MediumParts {
            cohorts,
            classes,
            sections,
        },
    )
}

/// Forty rooms across four kinds.
///
/// Enough of each specialist kind to serve the demand: five laboratories and four studios
/// against five year levels taking one such subject each. A fixture whose specialist rooms
/// were scarcer would be an interesting one and would not be this one — `infeasible-room-type`
/// is where scarcity is the point.
fn medium_rooms(builder: &mut SchoolBuilder, campus: CampusIx) -> MediumRoomTypes {
    use medium_scale::ROOMS;

    let general = builder.add_room_type("General Classroom");
    let laboratory = builder.add_room_type("Science Laboratory");
    let studio = builder.add_room_type("Arts Studio");
    let hall = builder.add_room_type("Hall");

    for n in 0..ROOMS {
        // Five laboratories, four studios, one hall, and the rest general.
        let kind = match n {
            0..=4 => laboratory,
            5..=8 => studio,
            9 => hall,
            _ => general,
        };
        builder.add_room(
            RoomFactSpec::new(&format!("Room {:03}", n + 1), 32)
                .on_campus(campus)
                .of_type(kind),
        );
    }

    MediumRoomTypes { laboratory, studio }
}

/// Five cohorts, each declaring its classes disjoint, and five classes under each.
///
/// Across year levels nothing is stated and nothing is provable, so every such pair stays
/// `Indeterminate` — which is what the relation's default already is, and is the honest answer
/// rather than a convenient one.
fn medium_groups(builder: &mut SchoolBuilder) -> (Vec<GroupIx>, Vec<GroupIx>) {
    use medium_scale::{CLASS_SIZE, CLASSES_PER_YEAR, YEAR_LEVELS};

    let mut cohorts = Vec::with_capacity(YEAR_LEVELS);
    let mut classes = Vec::with_capacity(YEAR_LEVELS * CLASSES_PER_YEAR);
    for year in 0..YEAR_LEVELS {
        let level = year + 7;
        let cohort = builder.add_group(
            GroupSpec::new(&format!("Year {level}"), GroupKind::Cohort)
                .sized(CLASS_SIZE * i64::try_from(CLASSES_PER_YEAR).unwrap_or(1))
                .children(DisjointnessDeclaration::Disjoint),
        );
        cohorts.push(cohort);
        for class in 0..CLASSES_PER_YEAR {
            let suffix = char::from(b'A' + u8::try_from(class).unwrap_or(0));
            classes.push(
                builder.add_group(
                    GroupSpec::new(&format!("{level}{suffix}"), GroupKind::Class)
                        .sized(CLASS_SIZE)
                        .child_of(cohort),
                ),
            );
        }
    }

    for (year, cohort) in cohorts.iter().enumerate() {
        builder.declare_overlap(*cohort, *cohort, Overlap::Overlaps);
        let own = &classes[year * CLASSES_PER_YEAR..(year + 1) * CLASSES_PER_YEAR];
        for (n, class) in own.iter().enumerate() {
            builder.declare_overlap(*class, *class, Overlap::Overlaps);
            builder.declare_overlap(*cohort, *class, Overlap::Overlaps);
            for other in &own[n + 1..] {
                builder.declare_overlap(*class, *other, Overlap::Disjoint);
            }
        }
    }

    (cohorts, classes)
}

/// One section per `(class, subject)`, and four occurrences each.
///
/// Every fifth section takes one of its four occurrences as a double, so the fixture
/// exercises multi-period placement at scale. A double is three singles plus a `2`, which is
/// four occurrences and five periods — a sum over rows and never a product.
fn medium_lessons(
    builder: &mut SchoolBuilder,
    schedule: BellScheduleIx,
    types: &MediumRoomTypes,
    teachers: &[TeacherIx],
    classes: &[GroupIx],
) -> Vec<SectionIx> {
    use medium_scale::{CLASS_SIZE, CLASSES_PER_YEAR, OCCURRENCES_PER_SECTION, TEACHERS};

    let mut sections = Vec::with_capacity(classes.len() * SUBJECTS.len());
    for (class_n, class) in classes.iter().enumerate() {
        let level = class_n / CLASSES_PER_YEAR + 7;
        for (subject_n, subject) in SUBJECTS.iter().enumerate() {
            let required = match *subject {
                "Science" => Some(types.laboratory),
                "Creative Arts" => Some(types.studio),
                _ => None,
            };
            // Spread the sections over the staff, so each carries one or two.
            let teacher = teachers[(class_n * SUBJECTS.len() + subject_n) % TEACHERS];
            let mut spec = SectionSpec::new(&format!(
                "{subject} {level}{}",
                char::from(b'A' + u8::try_from(class_n % CLASSES_PER_YEAR).unwrap_or(0))
            ))
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(*class)
            .attendance(ExpectedAttendance::Known { people: CLASS_SIZE })
            .declares_periods(i64::try_from(OCCURRENCES_PER_SECTION).unwrap_or(4));
            if let Some(room_type) = required {
                spec = spec.requires_room_type(room_type);
            }
            sections.push(builder.add_section(spec));
        }
    }

    for (n, section) in sections.iter().enumerate() {
        if n % 5 == 0 {
            builder.add_activities(*section, &[2, 1, 1, 1]);
        } else {
            builder.add_activities(*section, &[1, 1, 1, 1]);
        }
    }

    sections
}

/// A tenth of the staff are unavailable for one whole day of the cycle.
///
/// Part-time contracts are the ordinary case, and a fixture with no availability at all would
/// never exercise `entity-unavailable` at scale. The rules are compiled straight onto the
/// slots they name, which is what ADR 0033 §7 requires and what a fixture can state directly
/// without a loader.
fn medium_availability(
    builder: &mut SchoolBuilder,
    teachers: &[TeacherIx],
    slots: &[TimeslotIx],
    teaching_periods: usize,
) {
    use medium_scale::TEACHERS;

    let day_length = six_period_day().len();
    let teaching: Vec<TimeslotIx> = slots
        .iter()
        .copied()
        .enumerate()
        .filter(|(n, _)| six_period_day()[n % day_length].kind.is_teaching())
        .map(|(_, slot)| slot)
        .collect();

    for n in (0..TEACHERS).step_by(10) {
        let day = n % 5;
        for position in 0..teaching_periods {
            if let Some(slot) = teaching.get(day * teaching_periods + position) {
                builder.restrict(
                    AvailabilityEntity::Teacher {
                        teacher: teachers[n],
                    },
                    *slot,
                    AvailabilityStatus::Unavailable,
                );
            }
        }
    }
}

/// What a reader of the medium fixture should know that no assertion can state.
fn medium_notes() -> Vec<String> {
    vec![
        "Feasible by design and not proven. Each class needs 20 of its 30 teaching slots; no \
         teacher carries more than 8 occurrences; peak demand is about 17 concurrent lessons \
         against 40 rooms. No arrangement is attached, because only a solver can say a \
         timetable exists."
            .to_owned(),
        "1 000 students, declared on 25 class groups, with zero student rows. Expanding to \
         leaf student sets is the wrong compiled form (ADR 0035 §5)."
            .to_owned(),
        "Pairs of classes in different year levels are Indeterminate and are expected to stay \
         so: nothing stored relates them, and unrelated is not disjoint."
            .to_owned(),
        "Eight teachers are unavailable for a whole day each, so `entity-unavailable` has \
         something to bite on at scale."
            .to_owned(),
    ]
}

// =======================================================================================
// The two conclusively impossible fixtures
// =======================================================================================

/// A three-period occurrence in a school whose longest run of teaching periods is two.
///
/// # The contradiction, and why this one
///
/// The brief offers three candidate impossibilities and this is the one that can be
/// **conclusively diagnosed from the fixture's own data** with no solver and no search. The
/// bell schedule runs P1, P2, Break, P3, P4, so the longest unbroken run of teaching periods
/// is 2; the section asks for an occurrence of 3. [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §5 requires `d` **ordinal-consecutive teaching periods of the activity's own resolved
/// schedule**, and a run is broken by a non-teaching period between two teaching ones. No
/// placement exists, in any timetable, by any solver. That is arithmetic.
///
/// The alternative candidates were rejected as fixtures for M3: "one teacher required for
/// two simultaneous mandatory activities" is not conclusive without locks, which are not in
/// the MVP, and would otherwise be a search result rather than a counting fact.
///
/// # The expected finding, exactly
///
/// [`DiagnosticCode::DurationExceedsLongestRun`], about the **activity** — not the section
/// and not the course — because `activity.duration_periods` is the field whose value is
/// wrong and the activity is the row a user would edit.
///
/// The school also holds a second, longer schedule that is **not materialised** into this
/// grid, and that is load-bearing: ADR 0033 test 10 requires that a longer run available
/// elsewhere in the school be irrelevant, because a run never spans two schedules.
/// `evara_core::lessons::CycleCapacity` takes the school-wide **maximum** and so stays
/// silent here, which is correct — it can only under-report, never invent an impossibility.
fn infeasible_duration(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 1);

    // Defined and never materialised: four teaching periods in a row, so a run of 3 would
    // fit here. It is not in G, so it is not a candidate for anything.
    builder.add_bell_schedule(
        "Extended Day (not materialised)",
        Some(campus),
        &[
            PeriodSpec::teaching("E1", "08:00", "08:50"),
            PeriodSpec::teaching("E2", "08:55", "09:45"),
            PeriodSpec::teaching("E3", "09:50", "10:40"),
            PeriodSpec::teaching("E4", "10:45", "11:35"),
        ],
    );

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(0)));
    let group = builder.add_group(GroupSpec::new("8A", GroupKind::Class).sized(24));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let section = builder.add_section(
        SectionSpec::new("Design Technology 8A")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 24 }),
    );
    // One triple, which cannot be placed, and one single, which can. The fixture is not
    // wholly impossible — only this occurrence is — so a diagnostic that condemned the whole
    // school would be wrong.
    let activities = builder.add_activities(section, &[3, 1]);

    let problem = builder.finish();
    let offending = problem.activities[activities[0].get()].id;

    SchedulingFixture {
        name: FixtureKind::InfeasibleDuration.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::InfeasibleDuration.purpose(),
        expectations: Expectations {
            diagnostics: vec![ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::DurationExceedsLongestRun,
                subject_kind: EntityKind::Activity,
                subject: offending,
                because: "three consecutive teaching periods are needed and the resolved \
                          schedule's longest run is two; the other schedule in the school is \
                          not materialised and a run never spans two schedules",
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "Exactly one occurrence is impossible. The section's other occurrence is \
                 placeable, so a finding about the whole school or the whole section would \
                 be wrong."
                    .to_owned(),
                "`CycleCapacity` takes the school-wide maximum run and is expected to stay \
                 silent here. It can only under-report impossibility, which is the \
                 behaviour ADR 0033's consequences preserve deliberately."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

/// A section that requires a laboratory at a school whose only laboratory is not bookable.
///
/// The second conclusively diagnosable impossibility, and the one that exercises the room
/// candidate domain rather than the time one. `course.required_room_type_id` is the only room
/// requirement that exists; the candidate set is built by
/// [`RoomFact::is_candidate_for`](crate::scheduling::problem::RoomFact::is_candidate_for),
/// and `is_bookable = 0` removes a room from it independently of capacity (ADR 0034 §6). So
/// the set is empty and the finding is a counting fact.
///
/// Expected: [`DiagnosticCode::NoRoomOfRequiredType`] about the **class section**, because
/// the requirement lives on the section's course and every occurrence of the section fails
/// identically.
fn infeasible_room_type(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 3);

    let general = builder.add_room_type("General Classroom");
    let laboratory = builder.add_room_type("Science Laboratory");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    // The one laboratory is out of service, so it is in no candidate set.
    builder.add_room(
        RoomFactSpec::new("Laboratory 1", 24)
            .on_campus(campus)
            .of_type(laboratory)
            .not_bookable(),
    );

    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(1)));
    let group = builder.add_group(GroupSpec::new("9A", GroupKind::Class).sized(28));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let section = builder.add_section(
        SectionSpec::new("Chemistry 9A")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(group)
            .requires_room_type(laboratory)
            .attendance(ExpectedAttendance::Known { people: 28 }),
    );
    builder.add_activities(section, &[1, 1]);

    let problem = builder.finish();
    let subject = problem.sections[section.get()].id;

    SchedulingFixture {
        name: FixtureKind::InfeasibleRoomType.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::InfeasibleRoomType.purpose(),
        expectations: Expectations {
            diagnostics: vec![ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::NoRoomOfRequiredType,
                subject_kind: EntityKind::ClassSection,
                subject,
                because: "the only Science Laboratory is not bookable, so the candidate set \
                          is empty; is_bookable is a candidate-domain rule and is unrelated \
                          to capacity",
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "ADR 0034 test 13: a room that is not bookable is in no candidate set, \
                 independent of its capacity."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

// =======================================================================================
// Bell-schedule resolution
// =======================================================================================

/// One campus, a **school-wide** schedule, sections naming no campus.
///
/// Resolution is by [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §2 **clause 1** — a school-wide schedule serves every campus by its own definition — which
/// is the other half of the pair the tiny fixture covers with clause 3. ADR 0033 test 2.
fn single_campus_school_wide_bells(seed: u64) -> SchedulingFixture {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 0),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let campus = builder.add_campus("Main Campus");
    builder.add_cycle_days(WEEKDAYS, 5);
    // No campus: school-wide.
    let schedule = builder.add_bell_schedule("Standard Day", None, &two_run_day());
    builder.materialise(schedule);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(2)));
    let group = builder.add_group(GroupSpec::new("10A", GroupKind::Class).sized(27));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let section = builder.add_section(
        SectionSpec::new("English 10A")
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 27 }),
    );
    builder.add_activities(section, &[1, 1, 1, 1]);

    one_problem(
        FixtureKind::SingleCampusSchoolWideBells,
        seed,
        builder.finish(),
        Expectations {
            notes: vec![
                "Every section resolves under ADR 0033 §2 clause 1, and no finding is \
                 produced. The section states no bell schedule and the builder resolves it \
                 to the single materialised one, which is the only place a fixture is \
                 allowed to omit the resolution."
                    .to_owned(),
            ],
            ..Expectations::default()
        },
    )
}

/// Two campuses on two schedules whose timings are **identical**.
///
/// Two disjoint `timeslot` sets covering the same minutes — the duplication ADR 0033 accepts
/// deliberately, because `period_structure.campus_id` is one nullable column and a *subset*
/// of campuses cannot share a schedule. Each section resolves to its own campus's schedule
/// under clause 2. ADR 0033 test 4.
///
/// A hall reaches both campuses, so the duplication is also the case ADR 0032 rule 2 has to
/// keep safe: two bookings at the same minutes carry **different** `timeslot_id`s and are
/// still one clash.
fn two_campuses_identical_bells(seed: u64) -> SchedulingFixture {
    two_campus_school(
        seed,
        FixtureKind::TwoCampusesIdenticalBells,
        &two_run_day(),
        vec![
            "Two schedules with identical timings produce two disjoint slot sets over the \
             same minutes. Comparing timeslot ids and comparing intervals give the same \
             answer within one campus and different answers across them."
                .to_owned(),
            "ADR 0033 test 4: each section resolves to its own campus's schedule under \
             clause 2."
                .to_owned(),
        ],
    )
}

/// Two campuses on schedules whose timings **differ**, sharing a hall and a teacher.
///
/// The case that makes interval comparison necessary rather than merely general. North P1 is
/// 08:30–09:15 and South P1 is 09:00–09:45, so a booking in each overlaps for fifteen minutes
/// under two different `timeslot_id`s — exactly what
/// [ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md) calls "a correctness
/// failure" to miss. ADR 0032 tests 2, 3 and 5; ADR 0033 test 5.
fn two_campuses_different_bells(seed: u64) -> SchedulingFixture {
    two_campus_school(
        seed,
        FixtureKind::TwoCampusesDifferentBells,
        &two_run_day_shifted(),
        vec![
            "North P1 08:30–09:15 against South P1 09:00–09:45: overlapping minutes under \
             different timeslot ids. One clash for the shared hall and one for the shared \
             teacher, by interval."
                .to_owned(),
            "South P2 09:50–10:35 against North P3 10:25–11:10 also overlaps; North P2 \
             09:20–10:05 against South P3 10:55–11:40 does not. ADR 0032 test 3 wants both \
             directions."
                .to_owned(),
            "The shared teacher is eligible for both campuses, which is a stored fact rather \
             than an inference, and teaches a section at each."
                .to_owned(),
        ],
    )
}

/// The shared scaffold behind the two two-campus fixtures.
///
/// They differ in exactly one thing — whether the southern bells are shifted — so they share
/// everything else rather than drifting apart.
fn two_campus_school(
    seed: u64,
    kind: FixtureKind,
    southern_periods: &[PeriodSpec],
    notes: Vec<String>,
) -> SchedulingFixture {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 0),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let north = builder.add_campus("Northgate Senior");
    let south = builder.add_campus("Southside Junior");
    builder.add_cycle_days(WEEKDAYS, 5);

    let north_bells = builder.add_bell_schedule("Northgate Day", Some(north), &two_run_day());
    let south_bells = builder.add_bell_schedule("Southside Day", Some(south), southern_periods);
    builder.materialise(north_bells);
    builder.materialise(south_bells);

    let (shared_hall, hall_type) = two_campus_rooms(&mut builder, north, south);
    let (shared_teacher, north_only, south_only) = two_campus_staff(&mut builder, north, south);

    let north_group = builder.add_group(GroupSpec::new("11A", GroupKind::Class).sized(26));
    let south_group = builder.add_group(GroupSpec::new("7A", GroupKind::Class).sized(24));
    builder.declare_overlap(north_group, north_group, Overlap::Overlaps);
    builder.declare_overlap(south_group, south_group, Overlap::Overlaps);
    // Two classes at two campuses of one school: nothing stored relates them, so the honest
    // answer is Indeterminate rather than a convenient Disjoint.
    builder.declare_overlap(north_group, south_group, Overlap::Indeterminate);

    two_campus_lessons(
        &mut builder,
        hall_type,
        [north, south],
        [north_bells, south_bells],
        [shared_teacher, north_only, south_only],
        [north_group, south_group],
    );

    let problem = builder.finish();
    let teacher_id = problem.teachers[shared_teacher.get()].id;
    let hall_id = problem.rooms[shared_hall.get()].id;

    SchedulingFixture {
        name: kind.name(),
        seed: Seed::new(seed),
        purpose: kind.purpose(),
        expectations: Expectations {
            overlaps: vec![overlap_is(
                0,
                north_group,
                south_group,
                Overlap::Indeterminate,
                "rule 7 — two campuses' classes share no stated relation",
            )],
            diagnostics: vec![ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::TeacherUnderSeveralBellSchedules,
                subject_kind: EntityKind::Teacher,
                subject: teacher_id,
                because: "they teach under both schedules, so a period-scoped unavailability \
                          would bind only one of them (ADR 0033 §7)",
            }],
            attendance: Vec::new(),
            shared_identities: vec![SharedIdentity {
                kind: EntityKind::Room,
                id: hall_id,
                problems: vec![0],
            }],
            feasible: None,
            notes,
        },
        problems: vec![problem],
    }
}

/// Two Music sections in the shared hall, and a Mathematics section at each campus.
///
/// The Music pair is the interesting one: both require the hall, both are taught by the same
/// person, and they are scheduled against two different bell schedules — so a clash between
/// them is decided by minutes and never by a `timeslot_id`.
fn two_campus_lessons(
    builder: &mut SchoolBuilder,
    hall_type: RoomTypeIx,
    campuses: [CampusIx; 2],
    bells: [BellScheduleIx; 2],
    staff: [TeacherIx; 3],
    groups: [GroupIx; 2],
) {
    let [north, south] = campuses;
    let [north_bells, south_bells] = bells;
    let [shared, north_only, south_only] = staff;
    let [north_group, south_group] = groups;

    // The campus is part of the lesson rather than added afterwards, because a section's
    // campus is what bell-schedule resolution reads and the two must agree: a section at
    // Northgate scheduled against Southside's bells would be a fixture asserting something
    // ADR 0033 §2 forbids.
    let lesson = |name: &str, campus, bells, teacher, group, people, hall| {
        let mut spec = SectionSpec::new(name)
            .at(campus)
            .schedule(resolved(bells))
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people });
        if hall {
            spec = spec.requires_room_type(hall_type);
        }
        spec
    };

    let lessons = [
        builder.add_section(lesson(
            "Music 11A",
            north,
            north_bells,
            shared,
            north_group,
            26,
            true,
        )),
        builder.add_section(lesson(
            "Music 7A",
            south,
            south_bells,
            shared,
            south_group,
            24,
            true,
        )),
        builder.add_section(lesson(
            "Mathematics 11A",
            north,
            north_bells,
            north_only,
            north_group,
            26,
            false,
        )),
        builder.add_section(lesson(
            "Mathematics 7A",
            south,
            south_bells,
            south_only,
            south_group,
            24,
            false,
        )),
    ];

    // Two occurrences of each Music section, three of each Mathematics one.
    builder.add_activities(lessons[0], &[1, 1]);
    builder.add_activities(lessons[1], &[1, 1]);
    builder.add_activities(lessons[2], &[1, 1, 1]);
    builder.add_activities(lessons[3], &[1, 1, 1]);
}

/// A classroom at each campus, and one hall reaching both.
///
/// **One physical hall, one identity** — [ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md).
/// Modelling it as two rooms is the correctness failure that record exists to prevent, and a
/// fixture that did so would make `room-clash` look correct while missing the clash.
fn two_campus_rooms(
    builder: &mut SchoolBuilder,
    north: CampusIx,
    south: CampusIx,
) -> (RoomIx, RoomTypeIx) {
    let general = builder.add_room_type("General Classroom");
    let hall_type = builder.add_room_type("Hall");
    builder.add_room(
        RoomFactSpec::new("North 1", 30)
            .on_campus(north)
            .of_type(general),
    );
    builder.add_room(
        RoomFactSpec::new("South 1", 30)
            .on_campus(south)
            .of_type(general),
    );
    let shared_hall = builder.add_room(
        RoomFactSpec::new("Assembly Hall", 200)
            .on_campus(north)
            .on_campus(south)
            .of_type(hall_type),
    );
    (shared_hall, hall_type)
}

/// One teacher eligible for both campuses, and one for each.
///
/// Eligibility is a stored fact rather than an inference, and it is carried for reporting
/// rather than for enforcement: a section's staffing is fixed input, so the solver cannot pick
/// an ineligible teacher because it picks none.
fn two_campus_staff(
    builder: &mut SchoolBuilder,
    north: CampusIx,
    south: CampusIx,
) -> (TeacherIx, TeacherIx, TeacherIx) {
    (
        builder.add_teacher(TeacherFactSpec::new(&teacher_name(3)).eligible_for(&[north, south])),
        builder.add_teacher(TeacherFactSpec::new(&teacher_name(4)).eligible_for(&[north])),
        builder.add_teacher(TeacherFactSpec::new(&teacher_name(5)).eligible_for(&[south])),
    )
}

/// A school-wide schedule and the section's own campus schedule, both materialised.
///
/// Two compatible candidates, so resolution is **ambiguous** and the solve is refused with
/// both named. ADR 0033 tests 6 and 7. The fixture also carries a second section that names
/// **no** campus at a two-campus school, which is the `SectionHasNoCampus` cause — the cost
/// ADR 0033 takes deliberately rather than guessing.
fn ambiguous_bell_schedule(seed: u64) -> SchedulingFixture {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 1),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let north = builder.add_campus("Northgate Senior");
    let south = builder.add_campus("Southside Junior");
    builder.add_cycle_days(WEEKDAYS, 5);

    let school_wide = builder.add_bell_schedule("Standard Day", None, &two_run_day());
    let northern = builder.add_bell_schedule("Northgate Day", Some(north), &two_run_day_shifted());
    builder.materialise(school_wide);
    builder.materialise(northern);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("North 1", 30)
            .on_campus(north)
            .of_type(general),
    );
    builder.add_room(
        RoomFactSpec::new("South 1", 30)
            .on_campus(south)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(6)));
    let group = builder.add_group(GroupSpec::new("9A", GroupKind::Class).sized(25));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    // Compatible with the school-wide schedule (clause 1) *and* with its own campus's
    // (clause 2). Two candidates: Evara will not choose.
    let ambiguous = builder.add_section(
        SectionSpec::new("Mathematics 9A")
            .at(north)
            .schedule(BellScheduleResolution::Ambiguous {
                candidates: vec![school_wide, northern],
            })
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 25 }),
    );
    builder.add_activities(ambiguous, &[1, 1]);

    let problem = builder.finish();
    let subject = problem.sections[ambiguous.get()].id;

    SchedulingFixture {
        name: FixtureKind::AmbiguousBellSchedule.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::AmbiguousBellSchedule.purpose(),
        expectations: Expectations {
            diagnostics: vec![ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::AmbiguousBellSchedule,
                subject_kind: EntityKind::ClassSection,
                subject,
                because: "compatible with the school-wide schedule under clause 1 and with \
                          its own campus's under clause 2; the finding must name both \
                          candidates, because \"ambiguous\" without saying between what \
                          cannot be acted on",
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "Both candidates are materialised, which is what makes them candidates: G is \
                 derived from the grid and not from what the school has defined."
                    .to_owned(),
                "Re-running with the generation narrowed to one of the two must resolve, \
                 which is how an ambiguous school proceeds without a migration (ADR 0033 §1)."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

/// A bell schedule that exists and was never materialised.
///
/// It is not in `G`, so it is a candidate for nothing and produces **no** ambiguity — ADR
/// 0033 test 8. The fixture exists because the opposite behaviour is the tempting one: a
/// resolver that read `period_structure` rather than the grid would find two candidates here
/// and refuse a school that is perfectly well configured.
fn unmaterialised_bell_schedule(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 2);

    builder.add_bell_schedule("Sixth Form Day", Some(campus), &two_run_day_shifted());

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(7)));
    let group = builder.add_group(GroupSpec::new("12A", GroupKind::Class).sized(18));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let section = builder.add_section(
        SectionSpec::new("Further Mathematics 12A")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 18 }),
    );
    builder.add_activities(section, &[1, 1]);

    one_problem(
        FixtureKind::UnmaterialisedBellSchedule,
        seed,
        builder.finish(),
        Expectations {
            notes: vec![
                "Two bell schedules are defined and one is materialised. G holds one, so \
                 resolution is unique and no finding is produced."
                    .to_owned(),
                "A resolver that read period_structure instead of the grid would report an \
                 ambiguity here and refuse a correctly configured school. That is the \
                 mistake this fixture exists to catch."
                    .to_owned(),
            ],
            ..Expectations::default()
        },
    )
}

/// A cycle and term with no slots at all.
///
/// `G` is empty, so **no** activity has a candidate slot and the solve is refused once —
/// before any activity is examined, and not once per section. ADR 0033 test 9, and the
/// reason [`DiagnosticCode::NoGridMaterialised`] exists separately from
/// [`NoBellScheduleCause::GridEmpty`].
fn empty_grid(seed: u64) -> SchedulingFixture {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 3),
        "Standard week",
        CycleKind::Weekly,
    );
    builder.term("Autumn");
    let campus = builder.add_campus("Main Campus");
    builder.add_cycle_days(WEEKDAYS, 5);
    // Defined, and deliberately never materialised.
    builder.add_bell_schedule("Standard Day", Some(campus), &two_run_day());

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(8)));
    let group = builder.add_group(GroupSpec::new("8A", GroupKind::Class).sized(26));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let section = builder.add_section(
        SectionSpec::new("History 8A")
            .schedule(BellScheduleResolution::None {
                cause: NoBellScheduleCause::GridEmpty,
            })
            .taught_by(teacher)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 26 }),
    );
    builder.add_activities(section, &[1, 1]);

    let problem = builder.finish();
    let school = problem.context.school.id;

    SchedulingFixture {
        name: FixtureKind::EmptyGrid.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::EmptyGrid.purpose(),
        expectations: Expectations {
            diagnostics: vec![ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::NoGridMaterialised,
                subject_kind: EntityKind::School,
                subject: school,
                because: "G is empty, so the finding is about the solve and is raised once — \
                          not once per section, which is what ADR 0033 test 9 pins",
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "The subject is the school, not a section: with no grid there is nothing \
                 section-specific to say, and a per-section finding would produce one \
                 message per section for a single missing step."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

// =======================================================================================
// Cross-school sharing
// =======================================================================================

/// One teacher, one identity, two schools.
///
/// # Two problems, because one cannot express it
///
/// A `Problem` is scoped to one school by construction (ADR 0032 rule 1), so the sharing can
/// only be expressed as two problems holding the same `teacher.id`. That is also the real
/// situation: the two schools are solved independently and nothing compares their grids.
///
/// # What must happen
///
/// Generation for the first school **succeeds**, and a pre-flight warning names the teacher
/// *and both schools* and states the limit. The second school's timetable is never read —
/// whether it exists, is a draft, or there are several — because the warning is a property of
/// the sharing and not of any output. ADR 0032 tests 6 and 7: this fixture gives the second
/// school lessons but no timetable, and the expected warning is identical either way.
fn shared_teacher_across_schools(seed: u64) -> SchedulingFixture {
    let (problems, teacher_id) = two_school_pair(seed, "Autumn", "Autumn");

    SchedulingFixture {
        name: FixtureKind::SharedTeacherAcrossSchools.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::SharedTeacherAcrossSchools.purpose(),
        expectations: Expectations {
            diagnostics: vec![
                shared_entity_finding(0, teacher_id),
                shared_entity_finding(1, teacher_id),
            ],
            shared_identities: vec![SharedIdentity {
                kind: EntityKind::Teacher,
                id: teacher_id,
                problems: vec![0, 1],
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            feasible: None,
            notes: vec![
                "One teacher.id in both problems. A loader that minted a school-specific \
                 identity would make this warning impossible to write and would hide the \
                 sharing it reports (ADR 0021)."
                    .to_owned(),
                "Neither school's timetable is read. The warning is a property of the second \
                 active membership, so it is present while the first school is generating — \
                 which is exactly when it is needed."
                    .to_owned(),
                "A clash between the two schools is not detected, only announced. ADR 0032 \
                 records that as a real hole and does not hide it."
                    .to_owned(),
            ],
        },
        problems,
    }
}

/// Two schools whose academic calendars do not line up, sharing a teacher.
///
/// Different `term` rows, different names, and no project-level mapping between them. Both
/// generate independently and **nothing compares their grids** — ADR 0032 test 10. The first
/// school is also given no calendar coverage at all, which must affect no decision
/// whatsoever (test 11): a cycle carries no dates, both schedules of one school materialise
/// against the same `cycle_day` rows, and so the day axis is shared without `calendar_day`
/// being consulted.
fn different_school_calendars(seed: u64) -> SchedulingFixture {
    let (problems, teacher_id) = two_school_pair(seed, "Michaelmas", "Semester 1");

    SchedulingFixture {
        name: FixtureKind::DifferentSchoolCalendars.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::DifferentSchoolCalendars.purpose(),
        expectations: Expectations {
            diagnostics: vec![
                shared_entity_finding(0, teacher_id),
                shared_entity_finding(1, teacher_id),
            ],
            shared_identities: vec![SharedIdentity {
                kind: EntityKind::Teacher,
                id: teacher_id,
                problems: vec![0, 1],
            }],
            overlaps: Vec::new(),
            attendance: Vec::new(),
            feasible: None,
            notes: vec![
                "The two terms are differently named and unrelated. Nothing aligns one \
                 school's Day 1 to the other's, which is why ADR 0032 defers cross-school \
                 comparison rather than approximating it."
                    .to_owned(),
                "Neither problem carries any calendar-date mapping, and every clash \
                 decision must be unaffected. An incomplete calendar is the ordinary state \
                 of a real project and is not a prerequisite for anything."
                    .to_owned(),
                "No finding claims the two grids were compared, because they cannot be: that \
                 would need a date mapping and a timezone database, and the workspace has \
                 neither."
                    .to_owned(),
            ],
        },
        problems,
    }
}

/// Two small schools sharing one teacher identity, with the terms each names.
///
/// Returns the problems, the shared identity, and each school's reference, so the two
/// cross-school fixtures differ only in their term names and their notes.
fn two_school_pair(seed: u64, first_term: &str, second_term: &str) -> (Vec<Problem>, Uuid) {
    let mut first = SchoolBuilder::new(
        seed,
        school_name(seed, 0),
        "Standard week",
        CycleKind::Weekly,
    );
    // Minted from the first school's sequence and reused by the second, so there is exactly
    // one identity for this person in the whole project.
    let shared_id = first.mint_id();
    let first_reference = first.school_reference();

    let mut second = SchoolBuilder::new(
        seed.wrapping_add(0x9E37_79B9),
        school_name(seed, 1),
        "Standard week",
        CycleKind::Weekly,
    );
    let second_reference = second.school_reference();

    let first_problem =
        small_school_with_shared_teacher(&mut first, first_term, shared_id, second_reference, 9);
    let second_problem =
        small_school_with_shared_teacher(&mut second, second_term, shared_id, first_reference, 10);

    (vec![first_problem, second_problem], shared_id)
}

/// Fills in one half of a cross-school pair.
fn small_school_with_shared_teacher(
    builder: &mut SchoolBuilder,
    term: &str,
    shared_id: Uuid,
    other: crate::scheduling::problem::OtherSchool,
    local_teacher: usize,
) -> Problem {
    builder.term(term);
    let campus = builder.add_campus("Main Campus");
    builder.add_cycle_days(WEEKDAYS, 5);
    let schedule = builder.add_bell_schedule("Standard Day", Some(campus), &two_run_day());
    builder.materialise(schedule);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let shared = builder.add_teacher(
        TeacherFactSpec::new("P. Larkmoor")
            .with_identity(shared_id)
            .also_at(other),
    );
    let local = builder.add_teacher(TeacherFactSpec::new(&teacher_name(local_teacher)));
    let group = builder.add_group(GroupSpec::new("9A", GroupKind::Class).sized(24));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let shared_section = builder.add_section(
        SectionSpec::new("Music 9A")
            .schedule(resolved(schedule))
            .taught_by(shared)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 24 }),
    );
    let local_section = builder.add_section(
        SectionSpec::new("Mathematics 9A")
            .schedule(resolved(schedule))
            .taught_by(local)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 24 }),
    );
    builder.add_activities(shared_section, &[1, 1]);
    builder.add_activities(local_section, &[1, 1, 1]);

    // `SchoolBuilder::finish` consumes the builder, so swap in a throwaway and finish the
    // real one. Keeps the caller's code readable at the cost of this one line.
    let finished = std::mem::replace(
        builder,
        SchoolBuilder::new(0, "placeholder", "placeholder", CycleKind::Weekly),
    );
    finished.finish()
}

/// The sharing warning one school must produce about an entity the other also holds.
///
/// Both schools produce one, about the same person, which is what makes the warning a
/// property of the sharing rather than of either school's output.
const fn shared_entity_finding(problem: usize, teacher: Uuid) -> ExpectedDiagnostic {
    ExpectedDiagnostic {
        problem,
        code: DiagnosticCode::SharedEntityAcrossSchools,
        subject_kind: EntityKind::Teacher,
        subject: teacher,
        because: "a second active membership at another school; the warning must name the \
                  person, both schools and the consequence, and must not read the other \
                  school's timetable",
    }
}

// =======================================================================================
// Student group overlap
// =======================================================================================

/// Two sibling groups sharing three students, with no declaration anywhere.
///
/// `Overlaps` by [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// §3 **rule 3** — the member rows prove it, and no declaration was needed. Test 4. The two
/// sections are never concurrent.
fn overlapping_group_membership(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::OverlappingGroupMembership, |builder| {
        let cohort = builder.add_group(
            GroupSpec::new("Year 9", GroupKind::Cohort)
                .sized(120)
                .children(DisjointnessDeclaration::NotStated),
        );
        let set_x = builder.add_group(
            GroupSpec::new("9-Mat-1", GroupKind::TeachingSet)
                .size_unstated()
                .with_members(28)
                .child_of(cohort),
        );
        let set_y = builder.add_group(
            GroupSpec::new("9-Fre-1", GroupKind::TeachingSet)
                .size_unstated()
                .with_members(25)
                .child_of(cohort),
        );
        builder.declare_overlap(set_x, set_y, Overlap::Overlaps);
        builder.declare_overlap(cohort, set_x, Overlap::Overlaps);
        builder.declare_overlap(cohort, set_y, Overlap::Overlaps);
        (
            vec![set_x, set_y],
            vec![overlap_is(
                0,
                set_x,
                set_y,
                Overlap::Overlaps,
                "rule 3 — a shared member row is a fact, and no declaration was needed",
            )],
            vec![
                "Three students are in both sets. The member rows prove the overlap \
                     without the parent declaring anything, which is rule 3 extracting a \
                     genuine answer from partial data."
                    .to_owned(),
                "Both sets leave `size` unstated, so their attendance is Unknown and a \
                     floor of 28 and 25 is available to a warning only."
                    .to_owned(),
            ],
        )
    })
}

/// Siblings their parent declares disjoint, with no shared member.
///
/// `Disjoint` by **rule 6**, so concurrent placement is legal and produces no violation and
/// no warning. Test 6 — and the pair that makes the declaration worth having at all, because
/// `9A` against `9B` is the commonest pair in any school.
fn disjoint_sibling_groups(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::DisjointSiblingGroups, |builder| {
        let classes = builder.add_group(
            GroupSpec::new("Year 9 Classes", GroupKind::Cohort)
                .sized(84)
                .children(DisjointnessDeclaration::Disjoint),
        );
        let nine_a = builder.add_group(
            GroupSpec::new("9A", GroupKind::Class)
                .sized(28)
                .child_of(classes),
        );
        let nine_b = builder.add_group(
            GroupSpec::new("9B", GroupKind::Class)
                .sized(28)
                .child_of(classes),
        );
        // Children of 9A and 9B. Disjointness propagates to descendants, matched on the
        // ancestors — ADR 0035 truth table 12.
        let a_reading = builder.add_group(
            GroupSpec::new("9A Reading", GroupKind::TeachingSet)
                .sized(12)
                .child_of(nine_a),
        );
        let b_reading = builder.add_group(
            GroupSpec::new("9B Reading", GroupKind::TeachingSet)
                .sized(12)
                .child_of(nine_b),
        );
        builder.declare_overlap(classes, nine_a, Overlap::Overlaps);
        builder.declare_overlap(classes, nine_b, Overlap::Overlaps);
        builder.declare_overlap(nine_a, nine_b, Overlap::Disjoint);
        builder.declare_overlap(nine_a, a_reading, Overlap::Overlaps);
        builder.declare_overlap(nine_b, b_reading, Overlap::Overlaps);
        builder.declare_overlap(a_reading, b_reading, Overlap::Disjoint);
        (
            vec![nine_a, nine_b],
            vec![
                overlap_is(
                    0,
                    nine_a,
                    nine_b,
                    Overlap::Disjoint,
                    "rule 6 — the parent declares its children disjoint",
                ),
                overlap_is(
                    0,
                    a_reading,
                    b_reading,
                    Overlap::Disjoint,
                    "rule 6 — matched on the ancestors, so disjointness propagates to \
                     descendants",
                ),
            ],
            vec![
                "No shared member row anywhere, so nothing contradicts the declaration and \
                 the two classes may be timetabled concurrently with no warning."
                    .to_owned(),
                "The two Reading sets are children of two disjoint classes, and rule 6 \
                 matches on the ancestors. Truth table 12."
                    .to_owned(),
            ],
        )
    })
}

/// A school that has answered nothing about its groups.
///
/// Every declaration is `NotStated`, so `9A` against `9B` — the commonest pair in any school
/// — is `Indeterminate`. Generation **succeeds**; nothing is enforced beyond same-group and
/// ancestor pairs; pre-flight reports the pair count *and* the number of answers that would
/// clear it. ADR 0035 truth table 8 and test 2, and the state every existing project is in.
fn unknown_group_overlap(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::UnknownGroupOverlap, |builder| {
        let cohort = builder.add_group(
            GroupSpec::new("Year 10", GroupKind::Cohort)
                .sized(84)
                .children(DisjointnessDeclaration::NotStated),
        );
        let ten_a = builder.add_group(
            GroupSpec::new("10A", GroupKind::Class)
                .sized(28)
                .child_of(cohort),
        );
        let ten_b = builder.add_group(
            GroupSpec::new("10B", GroupKind::Class)
                .sized(28)
                .child_of(cohort),
        );
        builder.declare_overlap(cohort, ten_a, Overlap::Overlaps);
        builder.declare_overlap(cohort, ten_b, Overlap::Overlaps);
        builder.declare_overlap(ten_a, ten_b, Overlap::Indeterminate);
        (
            vec![ten_a, ten_b],
            vec![overlap_is(
                0,
                ten_a,
                ten_b,
                Overlap::Indeterminate,
                "rule 7 — nothing stored decides it, and the absence of member rows is not a \
                 proof of disjointness",
            )],
            vec![
                "Generation succeeds. Indeterminate is not enforced: enforcing it would let \
                 Year 10 hold one lesson at a time, and treating it as disjoint would put a \
                 child in two rooms while both timetables reported themselves clean."
                    .to_owned(),
                "One answer — on Year 10 — would clear the pair. That arithmetic is what \
                 pre-flight must report, rather than an adjective."
                    .to_owned(),
                "The generated timetable must carry the unverified-pair count, and must \
                 never be described as student-clash-free."
                    .to_owned(),
            ],
        )
    })
}

/// A declaration of disjointness that a member row contradicts.
///
/// **Facts win.** Rule 3 runs before rule 6, so the pair is `Overlaps`, the solve proceeds,
/// and the contradiction is reported naming the student, both groups and the parent whose
/// declaration they break. ADR 0035 truth table 9 and test 5.
///
/// This is also the mitigation for risk **E** of `SCHEDULING-ARCHITECTURE-DECISIONS.md` §8 —
/// a school ticking "disjoint" to clear a warning. A careless answer contradicted by any
/// member row is caught.
fn declared_disjoint_contradicted(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::DeclaredDisjointContradicted, |builder| {
        let cohort = builder.add_group(
            GroupSpec::new("Year 8", GroupKind::Cohort)
                .sized(56)
                .children(DisjointnessDeclaration::Disjoint),
        );
        let eight_a = builder.add_group(
            GroupSpec::new("8A", GroupKind::Class)
                .size_unstated()
                .with_members(28)
                .child_of(cohort),
        );
        let eight_b = builder.add_group(
            GroupSpec::new("8B", GroupKind::Class)
                .size_unstated()
                .with_members(28)
                .child_of(cohort),
        );
        builder.declare_overlap(cohort, eight_a, Overlap::Overlaps);
        builder.declare_overlap(cohort, eight_b, Overlap::Overlaps);
        // The declaration says disjoint; a member row says otherwise. Rule 3 beats rule 6.
        builder.declare_overlap(eight_a, eight_b, Overlap::Overlaps);
        (
            vec![eight_a, eight_b],
            vec![overlap_is(
                0,
                eight_a,
                eight_b,
                Overlap::Overlaps,
                "rule 3 before rule 6 — a member row is what happened and the flag is what \
                 was intended",
            )],
            vec![
                "One student is in both 8A and 8B, whose parent declares its children \
                 disjoint. The pair is Overlaps and the contradiction is reported; silently \
                 honouring either side would hide a data error that matters."
                    .to_owned(),
                "The expected finding is `group-overlap-contradiction` about the parent, \
                 naming the student and both children."
                    .to_owned(),
            ],
        )
    })
}

/// Two sets of 70 inside a cohort of 120, with no member row anywhere.
///
/// `Overlaps` by **rule 5** — arithmetic, not a guess: both sets are inside the cohort, so
/// their populations are subsets of its 120, and 70 + 70 cannot both fit. This finds a
/// genuine overlap in data that contains **no student rows at all**, which is precisely the
/// ADR 0006 school. Truth table 13, test 8.
fn pigeonhole_overlap(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::PigeonholeOverlap, |builder| {
        let cohort = builder.add_group(
            GroupSpec::new("Year 9", GroupKind::Cohort)
                .sized(120)
                .children(DisjointnessDeclaration::NotStated),
        );
        let set_x = builder.add_group(
            GroupSpec::new("9-Set-X", GroupKind::TeachingSet)
                .sized(70)
                .child_of(cohort),
        );
        let set_y = builder.add_group(
            GroupSpec::new("9-Set-Y", GroupKind::TeachingSet)
                .sized(70)
                .child_of(cohort),
        );
        let small_x = builder.add_group(
            GroupSpec::new("9-Small-X", GroupKind::TeachingSet)
                .sized(28)
                .child_of(cohort),
        );
        let small_y = builder.add_group(
            GroupSpec::new("9-Small-Y", GroupKind::TeachingSet)
                .sized(25)
                .child_of(cohort),
        );
        builder.declare_overlap(cohort, set_x, Overlap::Overlaps);
        builder.declare_overlap(cohort, set_y, Overlap::Overlaps);
        builder.declare_overlap(cohort, small_x, Overlap::Overlaps);
        builder.declare_overlap(cohort, small_y, Overlap::Overlaps);
        builder.declare_overlap(set_x, set_y, Overlap::Overlaps);
        // 28 + 25 ≤ 120, so the same arithmetic proves nothing about this pair.
        builder.declare_overlap(small_x, small_y, Overlap::Indeterminate);
        (
            vec![set_x, set_y],
            vec![
                overlap_is(
                    0,
                    set_x,
                    set_y,
                    Overlap::Overlaps,
                    "rule 5 — pigeonhole: 70 + 70 > 120 inside a common ancestor",
                ),
                overlap_is(
                    0,
                    small_x,
                    small_y,
                    Overlap::Indeterminate,
                    "rule 7 — 28 + 25 ≤ 120, so the arithmetic proves nothing",
                ),
            ],
            vec![
                "No member row exists anywhere in this fixture. Rule 5 extracts a genuine \
                 answer from declared sizes alone, which is the school ADR 0006 protects."
                    .to_owned(),
                "The second pair is the control: the same arithmetic that proves overlap \
                 when it fails proves nothing when it holds."
                    .to_owned(),
            ],
        )
    })
}

/// One cohort divided two ways: classes and subject sets.
///
/// # The case a tree alone cannot express
///
/// `9A`/`9B` are disjoint and `9-Mat-1`/`9-Mat-2` are disjoint, while `9-Mat-1` against `9A`
/// overlaps heavily — and in a flat hierarchy all three pairs are *structurally identical*.
/// ADR 0035's §"Context" shows that a rule making siblings disjoint would get the third pair
/// wrong, and that rearranging the tree does not help because the interposed nodes are
/// themselves siblings that overlap.
///
/// The composition §4 specifies is what this fixture models: the cohort declares `MayOverlap`
/// and each division declares `Disjoint`. Test 12.
fn two_divisions_of_one_cohort(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::TwoDivisionsOfOneCohort, |builder| {
        let cohort = builder.add_group(
            GroupSpec::new("Year 9", GroupKind::Cohort)
                .sized(120)
                // Deliberately an explicit "they may overlap", not silence: the classes and
                // the sets are two views of the same children.
                .children(DisjointnessDeclaration::MayOverlap),
        );
        let class_division = builder.add_group(
            GroupSpec::new("Year 9 Classes", GroupKind::Cohort)
                .sized(120)
                .child_of(cohort)
                .children(DisjointnessDeclaration::Disjoint),
        );
        let maths_division = builder.add_group(
            GroupSpec::new("Year 9 Maths Sets", GroupKind::Cohort)
                .sized(120)
                .child_of(cohort)
                .children(DisjointnessDeclaration::Disjoint),
        );
        let nine_a = builder.add_group(
            GroupSpec::new("9A", GroupKind::Class)
                .sized(30)
                .child_of(class_division),
        );
        let nine_b = builder.add_group(
            GroupSpec::new("9B", GroupKind::Class)
                .sized(30)
                .child_of(class_division),
        );
        let mat_1 = builder.add_group(
            GroupSpec::new("9-Mat-1", GroupKind::TeachingSet)
                .sized(28)
                .child_of(maths_division),
        );
        let mat_2 = builder.add_group(
            GroupSpec::new("9-Mat-2", GroupKind::TeachingSet)
                .sized(28)
                .child_of(maths_division),
        );

        for descendant in [class_division, maths_division, nine_a, nine_b, mat_1, mat_2] {
            builder.declare_overlap(cohort, descendant, Overlap::Overlaps);
        }
        builder.declare_overlap(class_division, nine_a, Overlap::Overlaps);
        builder.declare_overlap(class_division, nine_b, Overlap::Overlaps);
        builder.declare_overlap(maths_division, mat_1, Overlap::Overlaps);
        builder.declare_overlap(maths_division, mat_2, Overlap::Overlaps);
        builder.declare_overlap(nine_a, nine_b, Overlap::Disjoint);
        builder.declare_overlap(mat_1, mat_2, Overlap::Disjoint);
        // The two divisions overlap each other, and so do a class and a set. Honest, and the
        // pair a flat sibling rule gets wrong.
        builder.declare_overlap(class_division, maths_division, Overlap::Indeterminate);
        builder.declare_overlap(nine_a, mat_1, Overlap::Indeterminate);
        builder.declare_overlap(nine_a, mat_2, Overlap::Indeterminate);
        builder.declare_overlap(nine_b, mat_1, Overlap::Indeterminate);
        builder.declare_overlap(nine_b, mat_2, Overlap::Indeterminate);

        (
            vec![nine_a, mat_1],
            vec![
                overlap_is(
                    0,
                    nine_a,
                    nine_b,
                    Overlap::Disjoint,
                    "rule 6 — separated under Year 9 Classes, which declares 1",
                ),
                overlap_is(
                    0,
                    mat_1,
                    mat_2,
                    Overlap::Disjoint,
                    "rule 6 — separated under Year 9 Maths Sets, which declares 1",
                ),
                overlap_is(
                    0,
                    nine_a,
                    mat_1,
                    Overlap::Indeterminate,
                    "rule 7 — their paths separate under Year 9, which declares 0, so rule 6 \
                     does not fire",
                ),
            ],
            vec![
                "The three pairs are structurally identical in a flat hierarchy, and two of \
                 them are disjoint while the third is not. That is why the declaration is \
                 per parent and why it composes."
                    .to_owned(),
                "9A against 9-Mat-1 is Indeterminate, which is the honest answer: they very \
                 likely do overlap, and nothing stored proves it."
                    .to_owned(),
            ],
        )
    })
}

/// Eight levels, with members entered only at the leaves.
///
/// A cohort-level group overlaps a leaf seven levels down, which asserts the **subtree
/// union** of ADR 0035 §2 rather than a one-step parent read. Test 14, and the natural way a
/// school enters members: at the leaves.
fn deep_group_hierarchy(seed: u64) -> SchedulingFixture {
    group_school(seed, FixtureKind::DeepGroupHierarchy, |builder| {
        let mut chain = Vec::with_capacity(8);
        for level in 0..8 {
            let mut spec = GroupSpec::new(&format!("Level {level}"), GroupKind::Cohort);
            if level == 7 {
                // Members only at the leaf, which is the point.
                spec = spec.size_unstated().with_members(14);
            } else {
                spec = spec.size_unstated();
            }
            if let Some(parent) = chain.last() {
                spec = spec.child_of(*parent);
            }
            chain.push(builder.add_group(spec));
        }
        let root = chain[0];
        let leaf = chain[7];
        for (n, ancestor) in chain.iter().enumerate() {
            for descendant in &chain[n + 1..] {
                builder.declare_overlap(*ancestor, *descendant, Overlap::Overlaps);
            }
        }
        (
            vec![root, leaf],
            vec![overlap_is(
                0,
                root,
                leaf,
                Overlap::Overlaps,
                "rule 4 — containment, over a subtree seven levels deep",
            )],
            vec![
                "Only the leaf has member rows. Every ancestor's effective membership is the \
                 union over its subtree, so a one-step parent read would miss this pair \
                 entirely — and a school entering members at the leaves, which is the \
                 natural order, would get no protection at its cohort level."
                    .to_owned(),
                "MAX_GROUP_DEPTH is 64 and is a backstop against damage rather than a design \
                 target. Eight levels is deeper than any real school and still cheap."
                    .to_owned(),
            ],
        )
    })
}

// =======================================================================================
// Attendance
// =======================================================================================

/// A stated attendance of 24 against a group of 28, and a sound derivation beside it.
///
/// The explicit value **wins** and the two are never reconciled: a school that states 24 for
/// a section taught to a group of 28 has said something — a set sharing a room, a cohort with
/// absentees — and a derivation that overrode it would be refusing a true statement. The
/// four-person difference is reported as **advice**. ADR 0034 test 1.
///
/// A second section states nothing and is taught to two groups that both declare a size and
/// are provably disjoint, so its attendance derives to the sum. Test 2.
fn known_attendance(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 0);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    // A bookable room of capacity 0: a stored number, used as written, and separately named
    // by pre-flight as a likely unentered capacity. ADR 0034 §5 and test 10.
    let zero_capacity = builder.add_room(
        RoomFactSpec::new("Room 2", 0)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(0)));

    let classes = builder.add_group(
        GroupSpec::new("Year 9 Classes", GroupKind::Cohort)
            .sized(56)
            .children(DisjointnessDeclaration::Disjoint),
    );
    let nine_a = builder.add_group(
        GroupSpec::new("9A", GroupKind::Class)
            .sized(28)
            .child_of(classes),
    );
    let nine_b = builder.add_group(
        GroupSpec::new("9B", GroupKind::Class)
            .sized(28)
            .child_of(classes),
    );
    builder.declare_overlap(classes, nine_a, Overlap::Overlaps);
    builder.declare_overlap(classes, nine_b, Overlap::Overlaps);
    builder.declare_overlap(nine_a, nine_b, Overlap::Disjoint);

    let stated = builder.add_section(
        SectionSpec::new("Mathematics 9A")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(nine_a)
            .attendance(ExpectedAttendance::Known { people: 24 }),
    );
    let derived = builder.add_section(
        SectionSpec::new("Assembly 9AB")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(nine_a)
            .taught_to(nine_b)
            .attendance(ExpectedAttendance::Known { people: 56 }),
    );
    builder.add_activities(stated, &[1, 1, 1]);
    builder.add_activities(derived, &[1]);

    let problem = builder.finish();
    let expectations =
        known_attendance_expectations(&problem, [stated, derived], [nine_a, nine_b], zero_capacity);

    SchedulingFixture {
        name: FixtureKind::KnownAttendance.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::KnownAttendance.purpose(),
        expectations,
        problems: vec![problem],
    }
}

/// What the known-attendance fixture claims.
fn known_attendance_expectations(
    problem: &Problem,
    sections: [SectionIx; 2],
    classes: [GroupIx; 2],
    zero_capacity: RoomIx,
) -> Expectations {
    let [stated, derived] = sections;
    let [nine_a, nine_b] = classes;

    Expectations {
        attendance: vec![
            attendance_is(
                0,
                stated,
                ExpectedAttendance::Known { people: 24 },
                "the explicit value wins over a derivation and is never overridden",
            ),
            attendance_is(
                0,
                derived,
                ExpectedAttendance::Known { people: 56 },
                "both groups declare a size and are provably disjoint, so the sum is sound",
            ),
        ],
        overlaps: vec![overlap_is(
            0,
            nine_a,
            nine_b,
            Overlap::Disjoint,
            "rule 6 — which is what licenses the sum",
        )],
        diagnostics: vec![
            ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::ExpectedAttendanceDisagreesWithGroups,
                subject_kind: EntityKind::ClassSection,
                subject: problem.sections[stated.get()].id,
                because: "24 stated against a group of 28: advice, not an error, and nothing \
                          is overridden",
            },
            ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::BookableRoomZeroCapacity,
                subject_kind: EntityKind::Room,
                subject: problem.rooms[zero_capacity.get()].id,
                because: "0 means zero seats and is used as written; the finding names a \
                          suspicious number without reinterpreting it",
            },
        ],
        shared_identities: Vec::new(),
        feasible: None,
        notes: vec![
            "Both sections' lessons are taught to groups that are provably disjoint, so the \
             audience of the combined section is 56 and not a double count."
                .to_owned(),
            "The sum is only sound because rule 6 proved the two classes disjoint. The \
             `unknown-attendance` fixture is the same shape with that proof removed."
                .to_owned(),
        ],
    }
}

/// Four sections whose attendance cannot be derived, for four different reasons.
///
/// Every one is [`ExpectedAttendance::Unknown`] and **none is `0`**. ADR 0034 tests 3, 4, 5
/// and 7, which together pin the exact collapse the existing display performs:
/// `group.size.unwrap_or(named_members)` contributes `0` for a group that declares no size
/// and lists no members, and a section of such groups shows an audience of `0` — a number
/// that satisfies every room in the school including a cupboard.
fn unknown_attendance(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 2);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(1)));
    let groups = unknown_attendance_groups(&mut builder);
    let [silent, listed, cohort, set_p, set_q] = groups;

    let lesson = |name: &str, audience: &[GroupIx], floor: i64| {
        let mut spec = SectionSpec::new(name)
            .schedule(resolved(schedule))
            .taught_by(teacher);
        for group in audience {
            spec = spec.taught_to(*group);
        }
        if floor > 0 {
            spec = spec.attendance_at_least(floor);
        }
        spec
    };

    let sections = [
        builder.add_section(lesson("Art 10X", &[silent], 0)),
        builder.add_section(lesson("Art 10Y", &[listed], 26)),
        builder.add_section(lesson("Games 10PQ", &[set_p, set_q], 0)),
        builder.add_section(lesson("Assembly Year 10", &[cohort, set_p], 0)),
    ];
    for section in sections {
        builder.add_activities(section, &[1]);
    }

    let problem = builder.finish();
    let expectations = unknown_attendance_expectations(&problem, sections, [set_p, set_q], listed);

    SchedulingFixture {
        name: FixtureKind::UnknownAttendance.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::UnknownAttendance.purpose(),
        expectations,
        problems: vec![problem],
    }
}

/// Five groups, covering the four ways a derivation fails.
///
/// In order: nothing declared and nobody listed; no size but 26 members; a cohort; and two
/// sized sets under it whose overlap nobody has settled.
fn unknown_attendance_groups(builder: &mut SchoolBuilder) -> [GroupIx; 5] {
    // Nothing declared and nobody listed: Unknown, and the case that must never read as 0.
    let silent = builder.add_group(GroupSpec::new("10X", GroupKind::Class).size_unstated());
    // No size, 26 members: Unknown for the hard constraint, with a floor of 26 for a warning.
    let listed = builder.add_group(
        GroupSpec::new("10Y", GroupKind::Class)
            .size_unstated()
            .with_members(26),
    );
    let cohort = builder.add_group(
        GroupSpec::new("Year 10", GroupKind::Cohort)
            .sized(120)
            .children(DisjointnessDeclaration::NotStated),
    );
    let set_p = builder.add_group(
        GroupSpec::new("10-Set-P", GroupKind::TeachingSet)
            .sized(28)
            .child_of(cohort),
    );
    let set_q = builder.add_group(
        GroupSpec::new("10-Set-Q", GroupKind::TeachingSet)
            .sized(25)
            .child_of(cohort),
    );

    for group in [silent, listed, cohort, set_p, set_q] {
        builder.declare_overlap(group, group, Overlap::Overlaps);
    }
    builder.declare_overlap(cohort, set_p, Overlap::Overlaps);
    builder.declare_overlap(cohort, set_q, Overlap::Overlaps);
    // 28 + 25 ≤ 120, and nothing is declared, so the arithmetic proves nothing.
    builder.declare_overlap(set_p, set_q, Overlap::Indeterminate);

    [silent, listed, cohort, set_p, set_q]
}

/// What the unknown-attendance fixture claims: four `Unknown`s, for four different reasons.
fn unknown_attendance_expectations(
    problem: &Problem,
    sections: [SectionIx; 4],
    sets: [GroupIx; 2],
    listed: GroupIx,
) -> Expectations {
    const REASONS: [&str; 4] = [
        "no declared size and no member rows — Unknown, and never 0",
        "26 members is a floor for a warning and is never promoted to Known",
        "both sizes are declared and the overlap is Indeterminate, so no sum is sound",
        "a parent and its child overlap, so 120 + 28 is not 148 and not 120",
    ];

    Expectations {
        attendance: sections
            .iter()
            .zip(REASONS)
            .map(|(section, because)| {
                attendance_is(0, *section, ExpectedAttendance::Unknown, because)
            })
            .collect(),
        overlaps: vec![overlap_is(
            0,
            sets[0],
            sets[1],
            Overlap::Indeterminate,
            "rule 7 — 28 + 25 ≤ 120 proves nothing, and nothing is declared",
        )],
        diagnostics: sections
            .iter()
            .map(|section| ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::ExpectedAttendanceUnknown,
                subject_kind: EntityKind::ClassSection,
                subject: problem.sections[section.get()].id,
                because: "room sizes will not be checked for this section; the finding must \
                          name the remedy",
            })
            .chain(std::iter::once(ExpectedDiagnostic {
                problem: 0,
                code: DiagnosticCode::GroupSizeUnknownWithMembers,
                subject_kind: EntityKind::StudentGroup,
                subject: problem.groups[listed.get()].id,
                because: "26 students are listed and no size is declared, so only a floor is \
                          available",
            }))
            .collect(),
        shared_identities: Vec::new(),
        feasible: None,
        notes: vec![
            "Every section here generates. ADR 0006 guarantees a timetable to a school that \
             has entered no students and declared no sizes, so a capacity decision must not \
             revoke it."
                .to_owned(),
            "`room-capacity` is not evaluated for any of these activities. That is a third \
             outcome — not satisfied, not violated — and must be rendered as one."
                .to_owned(),
        ],
    }
}

/// Twenty named students and no attending group.
///
/// The audience is **empty**, so `student-clash` has nothing to say and nothing protects
/// those twenty children. `class_enrolment` has no scheduling meaning in Milestone 1, so the
/// roster contributes nothing to the audience and nothing to attendance — ADR 0029 §4. The
/// remedy is named: *add a student group*. ADR 0035 test 10, ADR 0034 test 6.
///
/// A second section names a student who is **also** in its attending group, which must be
/// counted once, via the group. Test 11.
fn roster_without_group(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 1);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(2)));
    let group = builder.add_group(GroupSpec::new("11A", GroupKind::Class).sized(24));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    let unprotected = builder.add_section(
        SectionSpec::new("Photography 11")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .with_roster(20),
    );
    let covered = builder.add_section(
        SectionSpec::new("Mathematics 11A")
            .schedule(resolved(schedule))
            .taught_by(teacher)
            .taught_to(group)
            .with_roster(3)
            .attendance(ExpectedAttendance::Known { people: 24 }),
    );
    builder.add_activities(unprotected, &[1, 1]);
    builder.add_activities(covered, &[1, 1, 1]);

    let problem = builder.finish();
    let unprotected_id = problem.sections[unprotected.get()].id;

    SchedulingFixture {
        name: FixtureKind::RosterWithoutGroup.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::RosterWithoutGroup.purpose(),
        expectations: Expectations {
            attendance: vec![
                attendance_is(
                    0,
                    unprotected,
                    ExpectedAttendance::Unknown,
                    "20 roster rows contribute nothing; the section has no attending group",
                ),
                attendance_is(
                    0,
                    covered,
                    ExpectedAttendance::Known { people: 24 },
                    "the explicit value; the three named students are among the 24 and are \
                     not added to them",
                ),
            ],
            diagnostics: vec![
                ExpectedDiagnostic {
                    problem: 0,
                    code: DiagnosticCode::SectionWithoutAttendingGroup,
                    subject_kind: EntityKind::ClassSection,
                    subject: unprotected_id,
                    because: "an empty audience means no clash protection at all, and the \
                              remedy is to add a student group",
                },
                ExpectedDiagnostic {
                    problem: 0,
                    code: DiagnosticCode::ExpectedAttendanceUnknown,
                    subject_kind: EntityKind::ClassSection,
                    subject: unprotected_id,
                    because: "no attending group, so no derivation is possible",
                },
            ],
            overlaps: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "The audience mask of the first section is empty, which is the machine-\
                 checkable form of \"nothing protects these students\"."
                    .to_owned(),
                "Nothing here double-counts anybody, because nothing here counts the roster \
                 at all."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

// =======================================================================================
// Availability and cycles
// =======================================================================================

/// A school with no availability at all, beside two rules that name nothing schedulable.
///
/// # What "no availability" means
///
/// `V11`'s header: *"Available is the default, so only exceptions are stored. A teacher with
/// no rows here is available in every slot."* So the first teacher here is available
/// everywhere, and that is a **stated** semantics rather than a convenient reading — which is
/// why this fixture exists, because the opposite assumption is equally easy to make and would
/// make every school unsolvable on day one.
///
/// The second and third rules are the ones a solve must report rather than compile: one names
/// an entity that no longer exists — `availability.entity_id` is polymorphic and so carries
/// no foreign key, the one place in the schema where a reference cannot be enforced — and one
/// names a `RESOURCE`, which nothing schedules.
fn missing_availability_context(seed: u64) -> SchedulingFixture {
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        slots,
    } = one_campus_school(seed, 3);

    let general = builder.add_room_type("General Classroom");
    let room = builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let unrestricted = builder.add_teacher(TeacherFactSpec::new(&teacher_name(3)));
    let restricted = builder.add_teacher(TeacherFactSpec::new(&teacher_name(4)));
    let group = builder.add_group(GroupSpec::new("7A", GroupKind::Class).sized(26));
    builder.declare_overlap(group, group, Overlap::Overlaps);

    // One teacher is unavailable for the whole of the first cycle position. The other has no
    // rules at all, which means available everywhere.
    for position in TWO_RUN_TEACHING {
        builder.restrict(
            AvailabilityEntity::Teacher {
                teacher: restricted,
            },
            at(&slots, 5, 0, position),
            AvailabilityStatus::Unavailable,
        );
    }
    // A soft preference, carried and unread by the first constraint set.
    builder.restrict(
        AvailabilityEntity::Room { room },
        at(&slots, 5, 1, 0),
        AvailabilityStatus::Discouraged,
    );

    let section_a = builder.add_section(
        SectionSpec::new("Geography 7A")
            .schedule(resolved(schedule))
            .taught_by(unrestricted)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 26 }),
    );
    let section_b = builder.add_section(
        SectionSpec::new("History 7A")
            .schedule(resolved(schedule))
            .taught_by(restricted)
            .taught_to(group)
            .attendance(ExpectedAttendance::Known { people: 26 }),
    );
    builder.add_activities(section_a, &[1, 1]);
    builder.add_activities(section_b, &[1, 1]);

    one_problem(
        FixtureKind::MissingAvailabilityContext,
        seed,
        builder.finish(),
        Expectations {
            notes: vec![
                "The first teacher has no availability rows and is available in every slot. \
                 V11's header states that default explicitly; assuming the opposite would \
                 make every school unsolvable before anybody had entered anything."
                    .to_owned(),
                "The second teacher is unavailable for all four teaching periods of cycle \
                 position 1, which `entity-unavailable` must enforce as a hard rule in M5."
                    .to_owned(),
                "One room carries a DISCOURAGED rule. It is carried and unread: every soft \
                 constraint is deferred past the MVP, and a discouraged slot is legal and \
                 merely worse."
                    .to_owned(),
                "A rule naming a deleted entity or a RESOURCE must be reported rather than \
                 compiled, so it does not appear to be enforced when it is not. Those two \
                 findings are `orphan-availability-reference` and \
                 `availability-entity-kind-not-scheduled`; neither is representable in a \
                 compiled index, which is exactly why they are diagnostics."
                    .to_owned(),
            ],
            ..Expectations::default()
        },
    )
}

/// A six-day rotation with two bell schedules.
///
/// Resolution and overlap are **unchanged**: they read a campus, a schedule and a group, and
/// never a weekday or a `day_count`. ADR 0033 test 14 and ADR 0035 test 16. The fixture
/// exists because `cycle.kind` is the obvious thing for a later reader to branch on, and
/// `CLAUDE.md` names reading a weekday in scheduling as an architecture bug.
fn custom_rotating_cycle(seed: u64) -> SchedulingFixture {
    let mut builder = SchoolBuilder::new(
        seed,
        school_name(seed, 2),
        "Six-day rotation",
        CycleKind::Custom,
    );
    builder.term("Autumn");
    let north = builder.add_campus("Northgate Senior");
    let south = builder.add_campus("Southside Junior");
    // Six positions, labelled as a rotation rather than as weekdays.
    builder.add_cycle_days(&["Day A", "Day B", "Day C", "Day D", "Day E", "Day F"], 6);

    let north_bells = builder.add_bell_schedule("Northgate Day", Some(north), &two_run_day());
    let south_bells =
        builder.add_bell_schedule("Southside Day", Some(south), &two_run_day_shifted());
    builder.materialise(north_bells);
    builder.materialise(south_bells);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("North 1", 30)
            .on_campus(north)
            .of_type(general),
    );
    builder.add_room(
        RoomFactSpec::new("South 1", 30)
            .on_campus(south)
            .of_type(general),
    );
    let north_teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(5)));
    let south_teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(6)));

    let classes = builder.add_group(
        GroupSpec::new("Year 8 Classes", GroupKind::Cohort)
            .sized(52)
            .children(DisjointnessDeclaration::Disjoint),
    );
    let eight_a = builder.add_group(
        GroupSpec::new("8A", GroupKind::Class)
            .sized(26)
            .child_of(classes),
    );
    let eight_b = builder.add_group(
        GroupSpec::new("8B", GroupKind::Class)
            .sized(26)
            .child_of(classes),
    );
    builder.declare_overlap(classes, eight_a, Overlap::Overlaps);
    builder.declare_overlap(classes, eight_b, Overlap::Overlaps);
    builder.declare_overlap(eight_a, eight_b, Overlap::Disjoint);

    let north_section = builder.add_section(
        SectionSpec::new("Mathematics 8A")
            .at(north)
            .schedule(resolved(north_bells))
            .taught_by(north_teacher)
            .taught_to(eight_a)
            .attendance(ExpectedAttendance::Known { people: 26 }),
    );
    let south_section = builder.add_section(
        SectionSpec::new("Mathematics 8B")
            .at(south)
            .schedule(resolved(south_bells))
            .taught_by(south_teacher)
            .taught_to(eight_b)
            .attendance(ExpectedAttendance::Known { people: 26 }),
    );
    builder.add_activities(north_section, &[2, 1, 1]);
    builder.add_activities(south_section, &[2, 1, 1]);

    let problem = builder.finish();
    SchedulingFixture {
        name: FixtureKind::CustomRotatingCycle.name(),
        seed: Seed::new(seed),
        purpose: FixtureKind::CustomRotatingCycle.purpose(),
        expectations: Expectations {
            overlaps: vec![overlap_is(
                0,
                eight_a,
                eight_b,
                Overlap::Disjoint,
                "rule 6 — unchanged by the cycle's kind, because overlap reads no day at all",
            )],
            attendance: Vec::new(),
            diagnostics: Vec::new(),
            shared_identities: Vec::new(),
            feasible: None,
            notes: vec![
                "Six positions, labelled Day A to Day F, with no weekday anywhere. Nothing \
                 in resolution or overlap reads cycle.kind or day_count."
                    .to_owned(),
                "Two campuses on different bells over a rotation: the day axis is shared \
                 because cycle_day belongs to the cycle and not to a bell schedule."
                    .to_owned(),
            ],
        },
        problems: vec![problem],
    }
}

// =======================================================================================
// Small shared helpers
// =======================================================================================

/// Wraps one finished problem into a fixture.
fn one_problem(
    kind: FixtureKind,
    seed: u64,
    problem: Problem,
    expectations: Expectations,
) -> SchedulingFixture {
    SchedulingFixture {
        name: kind.name(),
        seed: Seed::new(seed),
        purpose: kind.purpose(),
        problems: vec![problem],
        expectations,
    }
}

/// States an overlap expectation.
const fn overlap_is(
    problem: usize,
    a: GroupIx,
    b: GroupIx,
    overlap: Overlap,
    rule: &'static str,
) -> OverlapExpectation {
    OverlapExpectation {
        problem,
        a,
        b,
        overlap,
        rule,
    }
}

/// States an attendance expectation.
const fn attendance_is(
    problem: usize,
    section: SectionIx,
    expected: ExpectedAttendance,
    because: &'static str,
) -> AttendanceExpectation {
    AttendanceExpectation {
        problem,
        section,
        expected,
        because,
    }
}

/// The scaffold behind the group-overlap fixtures.
///
/// Each one differs only in its groups and the pairs it pins, so the school around them — one
/// campus, one schedule, two teachers, two rooms, and one section per focus group — is
/// written once. The callback returns the two groups to teach, the pairs to pin and the
/// notes.
fn group_school<F>(seed: u64, kind: FixtureKind, groups: F) -> SchedulingFixture
where
    F: FnOnce(&mut SchoolBuilder) -> (Vec<GroupIx>, Vec<OverlapExpectation>, Vec<String>),
{
    let OneCampusSchool {
        mut builder,
        campus,
        schedule,
        ..
    } = one_campus_school(seed, 0);

    let general = builder.add_room_type("General Classroom");
    builder.add_room(
        RoomFactSpec::new("Room 1", 30)
            .on_campus(campus)
            .of_type(general),
    );
    builder.add_room(
        RoomFactSpec::new("Room 2", 30)
            .on_campus(campus)
            .of_type(general),
    );
    let first_teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(0)));
    let second_teacher = builder.add_teacher(TeacherFactSpec::new(&teacher_name(1)));

    let (focus, overlaps, notes) = groups(&mut builder);

    for (n, group) in focus.iter().enumerate() {
        let teacher = if n == 0 {
            first_teacher
        } else {
            second_teacher
        };
        let section = builder.add_section(
            SectionSpec::new(&format!("Mathematics {}", n + 1))
                .schedule(resolved(schedule))
                .taught_by(teacher)
                .taught_to(*group),
        );
        builder.add_activities(section, &[1, 1]);
    }

    one_problem(
        kind,
        seed,
        builder.finish(),
        Expectations {
            overlaps,
            notes,
            ..Expectations::default()
        },
    )
}
