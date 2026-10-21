//! The immutable scheduling problem: everything a solve needs, and nothing else.
//!
//! # The pipeline this sits in
//!
//! ```text
//! SQLite  ──►  Problem  ──►  Validation  ──►  Constraints  ──►  Solver  ──►  Solution
//!   M4           M2             M3               M5              M6          M7
//! ```
//!
//! [`Problem`] is the compiled form of one school's database rows. It is built once, by
//! `evara-db`'s loader (M4), and then read without being changed: the search mutates an
//! [`Assignment`](super::Assignment), never the problem it is solving.
//!
//! # What it is not allowed to be
//!
//! - **No I/O.** `evara-core` depends on no other `evara-*` crate and opens nothing. A
//!   `Problem` is handed to the solver already materialised — `SOLVER.md` §2.
//! - **No driver types.** No `rusqlite`, no `refinery`, no SQL. The loader translates at the
//!   boundary.
//! - **No Tauri and no UI.** `evara-solver` must stay usable from the headless CLI and must
//!   stay compilable to `wasm32`, so nothing here may reach a window.
//! - **No global mutable state, and no clock.** Solving is deterministic given a seed, which
//!   forbids `Instant::now()` and unseeded randomness anywhere a decision is made.
//!
//! # Deterministic ordering is a correctness property, not a tidiness one
//!
//! Every collection here is a `Vec` in a defined order, and every index is dense. Two loads
//! of the same database must produce the same `Problem`, or the same seed produces a
//! different timetable and the reproducibility guarantee `SOLVER.md` §5 calls
//! "non-negotiable" is lost. Where a set is needed, it is a sorted `Vec` or a bitset — never
//! a `HashMap` whose iteration order varies.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::availability::UnavailabilityIndex;
use super::clock::{DurationPeriods, TimeInterval};
use super::context::{BellScheduleResolution, SchedulingContext};
use super::index::{
    ActivityIx, BellScheduleIx, CampusIx, CycleDayIx, GroupIx, PeriodIx, RoomIx, RoomTypeIx,
    SectionIx, TeacherIx, TimeslotIx,
};
use super::placement::Placement;
use super::population::{
    AttendanceFloor, DisjointnessDeclaration, ExpectedAttendance, GroupMask, OverlapRelation,
};
use super::resolve::MaterialisedGrid;

/// Another school that also holds an entity in this problem.
///
/// Carried so that
/// [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
/// rule 3's warning can be written: *"Ms Okafor is employed by Northgate and by Southmoor.
/// Evara cannot compare the two schools' timetables and will not detect a clash between
/// them."* That sentence needs both school names, and ADR 0032's consequences say in as many
/// words that a warning which merely said "cross-school scheduling is unsupported" would be
/// one the user learns to dismiss.
///
/// **No row of the other school is ever read.** The sharing is a property of the membership
/// or the campus association — a second active `teacher_school` row, a `room_campus` set
/// spanning two schools' campuses — not of the other school's timetable, which may be
/// absent, a draft, or several. ADR 0032 §5 is explicit that the warning must not depend on
/// it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherSchool {
    /// The other school's stable identity.
    pub id: Uuid,
    /// What it is called, so the warning can name it.
    pub name: String,
}

/// One materialised slot of the grid: the solver's value range, one element at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeslotFact {
    /// Stable identity, as stored.
    ///
    /// Assigned once and preserved across rematerialisation — a timeslot's logical identity
    /// is `(term, cycle_day, period)` and its `id` survives the grid being rebuilt
    /// ([ADR 0011](../../../../docs/adr/0011-stable-timeslot-identity.md)). A timetable entry
    /// written against it stays valid.
    pub id: Uuid,
    /// Which position in the cycle.
    pub cycle_day: CycleDayIx,
    /// Which period of which bell schedule.
    pub period: PeriodIx,
    /// The bell schedule this slot belongs to.
    ///
    /// Derived through `period → period_structure`. Carried on the slot because an
    /// activity's value range is **one** schedule's slots
    /// ([ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
    /// §4), and filtering by it must not need a join at every step.
    pub bell_schedule: BellScheduleIx,
    /// The campus this slot's schedule serves, or `None` for a school-wide schedule.
    ///
    /// Display and reporting only. **A campus is never part of a clash key** — ADR 0014
    /// forbids it for rooms, because one physical room is one occupancy record however many
    /// campuses reach it.
    pub campus: Option<CampusIx>,
    /// Whether a lesson may be placed here.
    ///
    /// Derived from `period.kind = 'TEACHING'` and from nothing else, so the loader's
    /// value-range restriction and M5's `no-teaching-in-period` evaluator read one stored
    /// fact and cannot disagree (ADR 0033 §4).
    pub is_teaching: bool,
    /// The wall-clock span, in the school's single timezone.
    ///
    /// **The clash key for shared occupancy.** Two placements of one school conflict when
    /// they fall on the same cycle day and these intervals overlap — never when they share
    /// an `id`. ADR 0032 rule 2.
    pub interval: TimeInterval,
}

/// A kind of room, as a requirement and as a room's classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomTypeFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the type is called — "Science Laboratory", "Gymnasium".
    pub name: String,
}

/// A room that may host a lesson of the school being solved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomFact {
    /// Stable identity, as stored.
    ///
    /// **One physical room, one identity**, however many campuses or schools reach it
    /// ([ADR 0014](../../../../docs/adr/0014-rooms-shared-across-campuses.md)). Modelling a
    /// shared hall as two rooms is the correctness failure that record exists to prevent, so
    /// nothing here duplicates a room per campus.
    pub id: Uuid,
    /// What the room is called.
    pub name: String,
    /// The campuses of this school that reach the room, in index order.
    ///
    /// More than one is ordinary and is the configuration ADR 0014 exists to support.
    pub campuses: Vec<CampusIx>,
    /// How the room is classified, or `None` for an unclassified room.
    pub room_type: Option<RoomTypeIx>,
    /// Seats, as stored.
    ///
    /// # Carried, and read by no constraint in the first set
    ///
    /// `room.capacity` is `NOT NULL CHECK (capacity >= 0)`, so it is always a stated number
    /// and there is no "unknown capacity" to represent — the uncertainty in capacity
    /// checking is entirely on the other side of the comparison, which is why
    /// [`ExpectedAttendance`] has an `Unknown` variant and this field is a plain `i64`.
    ///
    /// **`0` means zero seats and is never reinterpreted as "not entered".** ADR 0034 §5 is
    /// explicit: substituting a default for a stored value is the failure
    /// `DbError::CorruptIdentifier` exists to refuse in the identifier case, and the same
    /// principle applies to a number. A bookable room of capacity 0 is instead *named* by
    /// pre-flight as a likely unentered capacity — a warning, not an interpretation.
    ///
    /// `room-capacity` itself is **not** in the first constraint set
    /// ([ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
    /// §1), so nothing in the MVP compares this to anything.
    pub capacity: i64,
    /// Whether the room may be booked at all.
    ///
    /// `is_bookable = 0` removes the room from **every** activity's candidate set, and is
    /// unrelated to how many people fit — ADR 0034 §6. Stored, editable, and until now read
    /// by nothing.
    pub is_bookable: bool,
    /// Other schools whose campuses also reach this room.
    ///
    /// Empty for the ordinary case. Non-empty means ADR 0032 rule 3 warns: the room still
    /// clashes correctly *within* the school being solved, and no comparison is made against
    /// another school's placements.
    pub also_at: Vec<OtherSchool>,
}

impl RoomFact {
    /// Whether a lesson of a section with this campus and this room requirement could be
    /// placed here.
    ///
    /// # One implementation, by design
    ///
    /// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.1 names this as a requirement M5 must not
    /// lose: the room candidate rules are a **domain**, and *"a domain the manual editor
    /// could bypass is a rule living in two places."* `room-type-required` covers the type;
    /// `is_bookable` and campus association need either their own registered `type_key` or a
    /// documented guarantee that M11's `validate_move` consults the same candidate set.
    ///
    /// This method is that guarantee. It is the single place the three conditions are
    /// stated, and the loader (M4), the editor's move validator (M11) and the fixtures all
    /// call it rather than each repeating the predicate. That is `CLAUDE.md` §2's single
    /// most important rule applied to a candidate domain: one implementation, several
    /// callers, and generation and the editor cannot disagree about which rooms exist.
    ///
    /// The three conditions, and nothing else:
    ///
    /// 1. **Bookable.** `is_bookable = 0` removes the room from every candidate set, and is
    ///    unrelated to how many people fit (ADR 0034 §6).
    /// 2. **Reachable from the section's campus.** `section_campus` of `None` means the
    ///    section is not campus-constrained, which permits any campus this school reaches —
    ///    not "no campus".
    /// 3. **Of the required type.** `required_type` of `None` means no type is required, so
    ///    any classification passes, including an unclassified room.
    ///
    /// **Capacity is deliberately not a condition.** `room-capacity` is not in the first
    /// constraint set, and withholding a room because its size could not be verified would
    /// produce a timetable of roomless lessons — the alternative ADR 0034 §4 rejects by name.
    #[must_use]
    pub fn is_candidate_for(
        &self,
        section_campus: Option<CampusIx>,
        required_type: Option<RoomTypeIx>,
    ) -> bool {
        if !self.is_bookable {
            return false;
        }
        if let Some(campus) = section_campus
            && !self.campuses.contains(&campus)
        {
            return false;
        }
        if let Some(required) = required_type
            && self.room_type != Some(required)
        {
            return false;
        }
        true
    }
}

/// A teacher who may be given a lesson at the school being solved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeacherFact {
    /// Stable identity, as stored — **project-wide**.
    ///
    /// A teacher belongs to the project and a membership belongs to the school
    /// ([ADR 0021](../../../../docs/adr/0021-project-level-people-and-school-membership.md)),
    /// so somebody employed by two schools is **one** `teacher.id` in both. A loader that
    /// minted a school-specific identity would make the shared-teacher warning impossible to
    /// write and would hide the very sharing ADR 0032 rule 3 reports.
    pub id: Uuid,
    /// What to call them in a diagnostic.
    pub name: String,
    /// The campuses they are eligible for, or `None` where their eligibility is unnarrowed.
    ///
    /// `None` and an empty `Vec` are different: `None` means no `teacher_campus` row narrows
    /// them, which is the ordinary state and permits every campus; an empty list would mean
    /// they are eligible for none.
    ///
    /// Carried for reporting rather than for enforcement. Employment and campus eligibility
    /// are already database invariants under two trigger pairs (`V15:507`, ADR 0028), so the
    /// solver cannot pick an ineligible teacher **because it picks none** — a section's
    /// staffing is fixed input, not a planning variable (D7).
    pub eligible_campuses: Option<Vec<CampusIx>>,
    /// Other schools that also employ them, through a second active `teacher_school` row.
    ///
    /// Empty for the ordinary case. Non-empty is the exact condition of ADR 0032 rule 3's
    /// first bullet, and of its acceptance tests 6 and 7.
    pub also_at: Vec<OtherSchool>,
}

/// A student group of the school being solved.
///
/// # The inputs to overlap, not its answer
///
/// This fact carries what
/// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// §3's seven rules read — the parent edge, the declared size, how many students could be in
/// the room, and the parent's disjointness declaration. The **answer** lives in
/// [`Problem::overlap`], compiled once per solve, because it is a property of a *pair* and
/// not of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the group is called.
    pub name: String,
    /// What kind of group the school says it is.
    ///
    /// Display and data entry only. It carries **no scheduling meaning**, and ADR 0035
    /// records why it was the tempting shortcut for sibling disjointness and cannot work:
    /// `9-Mat-1` and `9-Fre-1` are both `TEACHING_SET` and they overlap heavily.
    pub kind: GroupKind,
    /// The group this one is declared to be part of.
    ///
    /// `parent_group_id` is **an assertion by the school**, not an enforced containment: no
    /// trigger and no repository check requires a child's members to be a subset of its
    /// parent's. ADR 0035 §"Two things nothing enforces" is the record of that, and it is
    /// why rule 4 is labelled *containment* rather than *fact*.
    pub parent: Option<GroupIx>,
    /// The size the school declared, or `None` where membership is the truth.
    ///
    /// `0` is legal and means an empty group. `None` means "I have not said", which is a
    /// different statement, and the two must not be merged:
    /// [ADR 0006](../../../../docs/adr/0006-group-based-scheduling-first.md) requires a full
    /// timetable to be buildable from declared sizes with **no student records at all**.
    pub declared_size: Option<i64>,
    /// How many students could actually be in the room.
    ///
    /// ADR 0035 §2's `effective_members`, as a count: member rows over the group's whole
    /// subtree, restricted to students who are `is_active` and hold a
    /// `student_school_enrolment` spell at this school whose status is not `LEFT`.
    ///
    /// The filter matters in both directions. `student_group_member` deliberately keeps
    /// history — *"membership of 9-MAT-A last year is the only evidence it happened"* — and
    /// that is right for the record and wrong for a room: counting a child who has left
    /// would manufacture clashes between groups that no longer share anybody.
    ///
    /// **A count of zero is not a proof of emptiness.** Nothing in the schema asserts a
    /// roster is finished, so an absent member row is not evidence of an absent student;
    /// that is why rule 3 can only prove overlap and never disjointness.
    pub effective_members: i64,
    /// What this group declares about **its children**: the future
    /// `student_group.children_are_disjoint`.
    ///
    /// Asked of the parent, once, and read by rule 6. `NotStated` is the value every
    /// existing project has, and is [`Overlap::Indeterminate`](super::Overlap::Indeterminate)
    /// for every sibling pair under it.
    pub children_are_disjoint: DisjointnessDeclaration,
}

/// What kind of group a school says a group is.
///
/// Mirrors `student_group.kind`'s five values exactly (`V10:122-123`). It gains no
/// scheduling meaning here and should not acquire one — see [`GroupFact::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GroupKind {
    /// A registration or teaching class.
    Class,
    /// A whole year or cohort.
    Cohort,
    /// A homeroom or form.
    Homeroom,
    /// An elective set.
    ElectiveSet,
    /// A set within a subject.
    TeachingSet,
}

/// A class section: the thing that has staff, an audience, a room requirement and a resolved
/// bell schedule.
///
/// # Why this is separate from an activity
///
/// Four occurrences of one section are taught by the same people to the same groups in the
/// same kind of room, and resolve to the same bell schedule — resolution reads only the
/// section ([ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §3). Holding those facts once rather than four times is the same argument ADR 0029 §5
/// used to reject `activity_teacher` and `activity_group`: a per-occurrence copy is four
/// places for one fact to disagree with itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the section is called, for a diagnostic that has to name it.
    pub name: String,
    /// The campus it is taught at, when the school has said.
    ///
    /// `None` means **not campus-constrained**, which is the ordinary state at a one-campus
    /// school and is not a missing value. It is the input to ADR 0033 §2's clause 3.
    pub campus: Option<CampusIx>,
    /// Which bell schedule this section's lessons draw their slots from.
    ///
    /// Resolved once per section. Anything other than
    /// [`BellScheduleResolution::Resolved`](super::BellScheduleResolution::Resolved) is a
    /// hard pre-flight error that refuses the whole solve — because an activity that cannot
    /// be placed anywhere is not a scheduling difficulty, it is an unanswered setup
    /// question.
    pub bell_schedule: BellScheduleResolution,
    /// Who teaches it, in index order.
    ///
    /// **Fixed input, not a planning variable** (D7). An empty list is a section nobody has
    /// been given, which pre-flight reports.
    pub teachers: Vec<TeacherIx>,
    /// The groups taught, in index order — the scheduling population.
    ///
    /// A set of **groups**, never an expanded set of students (ADR 0035 §5). An empty list is
    /// a section with no clash protection at all, and is reported with its remedy.
    pub audience: Vec<GroupIx>,
    /// [`SectionFact::audience`] as a bitset, for the hot path.
    pub audience_mask: GroupMask,
    /// Every group this section's lessons may not run beside.
    ///
    /// `audience_mask ∪ ⋃ overlaps_with[g]` over the audience — ADR 0035 §5. Two activities
    /// conflict exactly when one's `clash_mask` intersects the other's `audience_mask`.
    /// Compiled once per solve and never recomputed inside the search.
    pub clash_mask: GroupMask,
    /// The kind of room a lesson of this section needs, from `course.required_room_type_id`.
    ///
    /// `None` means no room type is required. **This is the only room requirement that
    /// exists** (`V15:223`); room candidate lists and resource requirements are not in the
    /// MVP and are not invented here.
    pub required_room_type: Option<RoomTypeIx>,
    /// The rooms this section's lessons may be placed in, in index order.
    ///
    /// Derived by the loader: rooms reachable from a campus of this school, narrowed to the
    /// section's campus where it names one, `is_bookable = 1`, and matching
    /// [`SectionFact::required_room_type`] where one is named.
    ///
    /// **An empty list where a room type is required is conclusively impossible**, and is
    /// reported as such rather than left for a solver to fail at.
    pub room_candidates: Vec<RoomIx>,
    /// How many people a lesson of this section should be assumed to hold.
    ///
    /// `Unknown` is ordinary and is never `0`. Nothing in the first constraint set reads it,
    /// because `room-capacity` is not in that set (ADR 0034 §1).
    pub expected_attendance: ExpectedAttendance,
    /// A floor on the attendance, from member rows.
    ///
    /// May raise a warning and may never decide a hard constraint — ADR 0034 §3.
    pub attendance_floor: AttendanceFloor,
    /// How many students are named on this section's roster.
    ///
    /// `class_enrolment` has **no scheduling meaning** in Milestone 1 (ADR 0029 §4), so this
    /// contributes nothing to [`SectionFact::audience`] and nothing to attendance. It is
    /// carried for one reason: a section with named students and **no** attending group has
    /// no clash protection, and pre-flight has to be able to say so — *"Year 9 Mathematics A
    /// names 20 students and no attending group; its students are not protected from
    /// clashes. Add a student group."* (ADR 0035 §7).
    pub individually_enrolled: i64,
    /// What the course declared it needs per cycle, or `None` where it has not said.
    ///
    /// `course.periods_per_cycle` is a **declaration**; the sum of the section's activity
    /// durations is the actual demand. The two are allowed to disagree and the difference is
    /// reported as advice, never enforced (ADR 0029 §3).
    pub declared_periods_per_cycle: Option<i64>,
}

/// One placeable occurrence: one `activity` row.
///
/// # One row is one occurrence
///
/// Four single periods a cycle is **four** of these, not one with a count.
/// [ADR 0029](../../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md)
/// decides it, and there is deliberately no `occurrences_per_cycle` field here or in the
/// persisted model: `timetable_entry` is unique on `(timetable_id, activity_id, timeslot_id)`
/// and a later "spread these across four days" constraint has to address the occurrences
/// individually.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityFact {
    /// Stable identity, as stored. What a `timetable_entry` is written against.
    pub id: Uuid,
    /// The section this is an occurrence of.
    ///
    /// Staffing, audience, room requirement and bell schedule all come from there. This is
    /// the only route to them, so they cannot be stated twice.
    pub section: SectionIx,
    /// A name for this occurrence, where it has one — "Laboratory", "Double".
    pub name: Option<String>,
    /// How many ordinal-consecutive teaching periods it needs.
    ///
    /// A count of periods and never a span of minutes — see
    /// [`DurationPeriods`](super::DurationPeriods).
    pub duration: DurationPeriods,
    /// Which occurrence this is within its section.
    ///
    /// Assigned once and **never renumbered**: deleting the second of four leaves 1, 3, 4.
    /// Carried for stable ordering and because a later spread constraint needs the
    /// occurrences individually addressable.
    pub split_index: u16,
}

/// One school's complete scheduling problem.
///
/// Immutable once built. See this module's header for the pipeline it sits in and the four
/// things it is not allowed to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// Which school, cycle and term, and the bell schedules the grid was built from.
    pub context: SchedulingContext,
    /// The value range: every materialised slot of the grid, teaching and non-teaching.
    ///
    /// Non-teaching slots are **kept**. The loader restricts an activity's candidates to the
    /// teaching ones, but `no-teaching-in-period` remains a registered constraint because
    /// manual editing can name a non-teaching slot and the editor must refuse for the same
    /// reason the generator never offered it (ADR 0033 §4).
    pub timeslots: Vec<TimeslotFact>,
    /// The kinds of room this school classifies by.
    pub room_types: Vec<RoomTypeFact>,
    /// Every room reachable from this school's campuses.
    pub rooms: Vec<RoomFact>,
    /// Every teacher with an active membership at this school.
    pub teachers: Vec<TeacherFact>,
    /// Every student group of this school.
    pub groups: Vec<GroupFact>,
    /// Every class section of this school in scope.
    pub sections: Vec<SectionFact>,
    /// Every occurrence to be placed. **The variable set.**
    pub activities: Vec<ActivityFact>,
    /// The three-state overlap answer for every pair of [`Problem::groups`].
    pub overlap: OverlapRelation,
    /// What the stored availability rules say, compiled onto the slots they name.
    pub unavailable: UnavailabilityIndex,
    /// Placements the solver may not change, indexed by activity.
    ///
    /// **Nothing populates this yet.** `activity.is_locked` is not in `V15` — the roadmap
    /// keeps lock toggles with Phase 8's manual editing, because a lock is about a *placed*
    /// timetable rather than about the demand — and `activity-fixed` enters the constraint
    /// set "once locks exist". The field is declared because `SOLVER.md` §2 declares it and
    /// because repair (`start = Some(current)` plus a scope filter) treats out-of-scope
    /// activities as pinned, which is the mechanism partial regeneration is built from.
    pub pinned: Vec<Option<Placement>>,
}

impl Problem {
    /// How many occurrences have to be placed.
    #[must_use]
    pub fn activity_count(&self) -> usize {
        self.activities.len()
    }

    /// The slots of one bell schedule on one cycle day, in period-ordinal order.
    ///
    /// # An index, not a check
    ///
    /// This answers *"which slots exist here, in what order?"* and nothing more. Whether a
    /// given run of them is a legal home for a `duration_periods = d` occurrence is a
    /// different question with four separate ways to fail — the periods must be
    /// ordinal-consecutive, the run must be long enough, no non-teaching period may interrupt
    /// it, and it must not cross a boundary — and answering it is `duration-contiguous`'s
    /// job in M5, not this crate's in M2.
    ///
    /// What this guarantees is that the ordering such a check needs is available and
    /// deterministic. In particular **adjacency in this list is not adjacency in the bell
    /// schedule**: a non-teaching period between two teaching ones appears here, so a caller
    /// that filters it out and then treats the survivors as consecutive would get the wrong
    /// answer. That is exactly the mistake the ordering is laid out to make visible.
    #[must_use]
    pub fn slots_of(&self, schedule: BellScheduleIx, day: CycleDayIx) -> Vec<TimeslotIx> {
        let mut slots: Vec<TimeslotIx> = self
            .timeslots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.bell_schedule == schedule && slot.cycle_day == day)
            .map(|(ix, _)| TimeslotIx::new(u32::try_from(ix).unwrap_or(u32::MAX)))
            .collect();
        slots.sort_by_key(|ix| self.period_ordinal(*ix));
        slots
    }

    /// The period ordinal of one slot, or `0` if the slot's period is out of range.
    ///
    /// `0` is not a valid ordinal — `CHECK (ordinal >= 1)` — so it sorts a damaged reference
    /// first rather than panicking inside a comparator. [`Problem::integrity`] is what
    /// reports such a reference.
    fn period_ordinal(&self, slot: TimeslotIx) -> u16 {
        self.timeslots
            .get(slot.get())
            .and_then(|slot| self.context.periods.get(slot.period.get()))
            .map_or(0, |period| period.ordinal)
    }

    /// Every way this problem refers to something it does not contain.
    ///
    /// # Why a loader self-check belongs here
    ///
    /// A `Problem` is addressed by dense indices, and a dense index into the wrong table is
    /// the one class of loader bug that produces a plausible wrong timetable instead of a
    /// crash. The check is cheap, it runs over data this crate owns, and it needs no
    /// database — so it can gate both M4's loader and M2's fixtures, which is what makes
    /// "no dangling references" an assertion rather than a hope.
    ///
    /// This is **not** a scheduling rule and reports nothing about feasibility. An empty
    /// result means the problem is internally consistent, not that a timetable exists.
    #[must_use]
    pub fn integrity(&self) -> Vec<IntegrityError> {
        let mut errors = Vec::new();
        self.time_model_integrity(&mut errors);
        self.facility_integrity(&mut errors);
        self.population_integrity(&mut errors);
        self.lesson_integrity(&mut errors);
        self.companion_integrity(&mut errors);
        self.resolution_integrity(&mut errors);
        errors.extend(self.duplicate_identities());
        errors
    }

    /// Every section whose recorded bell-schedule resolution the relation does not produce.
    ///
    /// Runs only when the indices it would read are sound: a dangling campus or schedule index
    /// would make the relation's answer meaningless, and reporting a disagreement on top of a
    /// dangling reference would bury the cause under a symptom.
    fn resolution_integrity(&self, errors: &mut Vec<IntegrityError>) {
        if !errors.is_empty() {
            return;
        }
        let grid = MaterialisedGrid::of(self);
        for (at, section) in self.sections.iter().enumerate() {
            let computed = grid.resolve(&self.context, section.campus);
            if section.bell_schedule != computed {
                errors.push(IntegrityError::BellScheduleResolutionDisagrees {
                    at,
                    declared: section.bell_schedule.clone(),
                    computed,
                });
            }
        }
    }

    /// The grid: schedules, periods and slots.
    fn time_model_integrity(&self, errors: &mut Vec<IntegrityError>) {
        let campuses = self.context.campuses.len();
        let schedules = self.context.bell_schedules.len();
        let periods = self.context.periods.len();
        let days = self.context.cycle_days.len();

        for (at, schedule) in self.context.bell_schedules.iter().enumerate() {
            if schedule.campus.is_some_and(|c| c.get() >= campuses) {
                errors.push(dangling("bellSchedule", at, "campus", usize::MAX));
            }
            for period in &schedule.periods {
                if period.get() >= periods {
                    errors.push(dangling("bellSchedule", at, "periods", period.get()));
                }
            }
        }
        for (at, period) in self.context.periods.iter().enumerate() {
            if period.bell_schedule.get() >= schedules {
                errors.push(dangling(
                    "period",
                    at,
                    "bellSchedule",
                    period.bell_schedule.get(),
                ));
            }
        }
        for (at, slot) in self.timeslots.iter().enumerate() {
            if slot.cycle_day.get() >= days {
                errors.push(dangling("timeslot", at, "cycleDay", slot.cycle_day.get()));
            }
            if slot.period.get() >= periods {
                errors.push(dangling("timeslot", at, "period", slot.period.get()));
            }
            if slot.bell_schedule.get() >= schedules {
                errors.push(dangling(
                    "timeslot",
                    at,
                    "bellSchedule",
                    slot.bell_schedule.get(),
                ));
            }
            if slot.campus.is_some_and(|c| c.get() >= campuses) {
                errors.push(dangling("timeslot", at, "campus", usize::MAX));
            }
        }
    }

    /// Rooms and their classifications.
    fn facility_integrity(&self, errors: &mut Vec<IntegrityError>) {
        let campuses = self.context.campuses.len();
        for (at, room) in self.rooms.iter().enumerate() {
            if room
                .room_type
                .is_some_and(|t| t.get() >= self.room_types.len())
            {
                errors.push(dangling("room", at, "roomType", usize::MAX));
            }
            for campus in &room.campuses {
                if campus.get() >= campuses {
                    errors.push(dangling("room", at, "campuses", campus.get()));
                }
            }
        }
    }

    /// Teachers and student groups.
    fn population_integrity(&self, errors: &mut Vec<IntegrityError>) {
        let campuses = self.context.campuses.len();
        for (at, teacher) in self.teachers.iter().enumerate() {
            for campus in teacher.eligible_campuses.iter().flatten() {
                if campus.get() >= campuses {
                    errors.push(dangling("teacher", at, "eligibleCampuses", campus.get()));
                }
            }
        }
        for (at, group) in self.groups.iter().enumerate() {
            if group.parent.is_some_and(|p| p.get() >= self.groups.len()) {
                errors.push(dangling("group", at, "parent", usize::MAX));
            }
            if group.parent == Some(GroupIx::new(u32::try_from(at).unwrap_or(u32::MAX))) {
                errors.push(IntegrityError::GroupIsItsOwnParent { at });
            }
        }
    }

    /// Sections and the occurrences that belong to them.
    fn lesson_integrity(&self, errors: &mut Vec<IntegrityError>) {
        let campuses = self.context.campuses.len();
        let schedules = self.context.bell_schedules.len();

        for (at, section) in self.sections.iter().enumerate() {
            if section.campus.is_some_and(|c| c.get() >= campuses) {
                errors.push(dangling("section", at, "campus", usize::MAX));
            }
            if let Some(schedule) = section.bell_schedule.schedule()
                && schedule.get() >= schedules
            {
                errors.push(dangling("section", at, "bellSchedule", schedule.get()));
            }
            for teacher in &section.teachers {
                if teacher.get() >= self.teachers.len() {
                    errors.push(dangling("section", at, "teachers", teacher.get()));
                }
            }
            for group in &section.audience {
                if group.get() >= self.groups.len() {
                    errors.push(dangling("section", at, "audience", group.get()));
                }
            }
            for room in &section.room_candidates {
                if room.get() >= self.rooms.len() {
                    errors.push(dangling("section", at, "roomCandidates", room.get()));
                }
            }
            if section
                .required_room_type
                .is_some_and(|t| t.get() >= self.room_types.len())
            {
                errors.push(dangling("section", at, "requiredRoomType", usize::MAX));
            }
        }
        for (at, activity) in self.activities.iter().enumerate() {
            if activity.section.get() >= self.sections.len() {
                errors.push(dangling("activity", at, "section", activity.section.get()));
            }
        }
    }

    /// The three structures sized from a table rather than indexing into one.
    ///
    /// Each would otherwise fail silently: a relation one group short makes every pair
    /// involving the last group unaddressable, and an index one slot short reads an unset bit
    /// where it should read out of range.
    fn companion_integrity(&self, errors: &mut Vec<IntegrityError>) {
        if self.overlap.groups() != self.groups.len() {
            errors.push(IntegrityError::OverlapRelationWrongSize {
                groups: self.groups.len(),
                relation: self.overlap.groups(),
            });
        }
        if self.unavailable.slots() != self.timeslots.len() {
            errors.push(IntegrityError::UnavailabilityIndexWrongSize {
                slots: self.timeslots.len(),
                index: self.unavailable.slots(),
            });
        }
        if !self.pinned.is_empty() && self.pinned.len() != self.activities.len() {
            errors.push(IntegrityError::PinnedWrongLength {
                activities: self.activities.len(),
                pinned: self.pinned.len(),
            });
        }
    }

    /// Every identifier used twice within one table.
    fn duplicate_identities(&self) -> Vec<IntegrityError> {
        let mut errors = Vec::new();
        errors.extend(duplicate_ids(
            "timeslot",
            self.timeslots.iter().map(|t| t.id),
        ));
        errors.extend(duplicate_ids("room", self.rooms.iter().map(|r| r.id)));
        errors.extend(duplicate_ids("teacher", self.teachers.iter().map(|t| t.id)));
        errors.extend(duplicate_ids("group", self.groups.iter().map(|g| g.id)));
        errors.extend(duplicate_ids("section", self.sections.iter().map(|s| s.id)));
        errors.extend(duplicate_ids(
            "activity",
            self.activities.iter().map(|a| a.id),
        ));
        errors.extend(duplicate_ids(
            "roomType",
            self.room_types.iter().map(|t| t.id),
        ));
        errors
    }

    /// The activity indices, in order. Convenient for a caller iterating the variable set.
    pub fn activity_indices(&self) -> impl Iterator<Item = ActivityIx> + '_ {
        (0..self.activities.len()).map(|ix| ActivityIx::new(u32::try_from(ix).unwrap_or(u32::MAX)))
    }

    /// The section one activity is an occurrence of, or `None` for a dangling reference.
    #[must_use]
    pub fn section_of(&self, activity: ActivityIx) -> Option<&SectionFact> {
        let section = self.activities.get(activity.get())?.section;
        self.sections.get(section.get())
    }
}

/// A dangling-index error, named once so the five per-table checks read the same way.
///
/// `index` is `usize::MAX` where the offending field is optional and the exact value adds
/// nothing a reader could act on.
const fn dangling(
    what: &'static str,
    at: usize,
    field: &'static str,
    index: usize,
) -> IntegrityError {
    IntegrityError::DanglingIndex {
        what,
        at,
        field,
        index,
    }
}

/// Every identifier that appears more than once in one table.
///
/// Duplicates are reported in first-seen order so the result is deterministic.
fn duplicate_ids(what: &'static str, ids: impl Iterator<Item = Uuid>) -> Vec<IntegrityError> {
    let mut seen: Vec<Uuid> = Vec::new();
    let mut reported: Vec<Uuid> = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            if !reported.contains(&id) {
                reported.push(id);
                errors.push(IntegrityError::DuplicateIdentity { what, id });
            }
        } else {
            seen.push(id);
        }
    }
    errors
}

/// A way a [`Problem`] refers to something it does not contain.
///
/// These are **loader and fixture bugs**, not data states. Nothing a school can type
/// produces one: the database's own foreign keys and triggers prevent the rows from
/// existing, so an error here means the translation from rows to indices went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum IntegrityError {
    /// An index points past the end of the table it addresses.
    DanglingIndex {
        /// Which table the holder is in.
        what: &'static str,
        /// The holder's position in that table.
        at: usize,
        /// Which field of it.
        field: &'static str,
        /// The offending index, or `usize::MAX` where the field is optional and the exact
        /// value adds nothing to the report.
        index: usize,
    },
    /// Two rows of one table share a stored identity.
    ///
    /// Fatal for persistence: a solution is written back against these identifiers, so two
    /// rows claiming one would overwrite each other.
    DuplicateIdentity {
        /// Which table.
        what: &'static str,
        /// The repeated identifier.
        id: Uuid,
    },
    /// A group is declared to be its own parent.
    ///
    /// The database's one-step cycle `CHECK` refuses this (`V10:141`), so it can only be a
    /// translation error. Longer cycles are walked by the repository before any write.
    GroupIsItsOwnParent {
        /// The group's position.
        at: usize,
    },
    /// The overlap relation does not cover exactly the problem's groups.
    OverlapRelationWrongSize {
        /// How many groups the problem holds.
        groups: usize,
        /// How many the relation covers.
        relation: usize,
    },
    /// The availability index does not cover exactly the problem's slots.
    UnavailabilityIndexWrongSize {
        /// How many slots the problem holds.
        slots: usize,
        /// How many the index covers.
        index: usize,
    },
    /// The pinned list is neither empty nor one entry per activity.
    PinnedWrongLength {
        /// How many activities the problem holds.
        activities: usize,
        /// How long the pinned list is.
        pinned: usize,
    },
    /// A section's recorded bell-schedule resolution is not what the compatibility relation
    /// produces.
    ///
    /// # Why this is an integrity error and not a diagnostic
    ///
    /// [`SectionFact::bell_schedule`] is an *answer*, computed by whatever built the problem.
    /// A section that resolves to none or to several schedules is an ordinary — if blocking —
    /// state of a real school, and M3 reports it as `no-bell-schedule` or
    /// `ambiguous-bell-schedule` from the **computed** result.
    ///
    /// This variant is the different failure: the problem *claims* an answer the accepted
    /// relation does not give. No school can type that; only a loader or a fixture can write
    /// it. So it belongs in the channel this type exists for — loader and fixture bugs — and
    /// not in a report a user is asked to act on.
    ///
    /// M2 left this unchecked, which meant a fixture could declare any resolution it liked.
    /// See [ADR 0036](../../../../docs/adr/0036-one-bell-schedule-resolver-shared-by-validation-and-loading.md).
    BellScheduleResolutionDisagrees {
        /// The section's position.
        at: usize,
        /// What the section records.
        declared: BellScheduleResolution,
        /// What [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
        /// §2's relation produces.
        computed: BellScheduleResolution,
    },
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        ActivityFact, DurationPeriods, GroupFact, GroupIx, GroupKind, IntegrityError, Problem,
        RoomTypeFact, SectionIx,
    };
    use crate::scheduling::population::DisjointnessDeclaration;

    /// The smallest problem that is internally consistent: no entities at all.
    fn empty_problem() -> Problem {
        use crate::scheduling::availability::UnavailabilityIndex;
        use crate::scheduling::context::{CycleFact, CycleKind, SchedulingContext, SchoolFact};
        use crate::scheduling::population::OverlapRelation;

        Problem {
            context: SchedulingContext {
                school: SchoolFact {
                    id: Uuid::from_u128(1),
                    name: "Northgate".into(),
                    timezone: "Europe/London".into(),
                },
                cycle: CycleFact {
                    id: Uuid::from_u128(2),
                    name: "Standard week".into(),
                    kind: CycleKind::Weekly,
                },
                term: None,
                campuses: Vec::new(),
                cycle_days: Vec::new(),
                bell_schedules: Vec::new(),
                periods: Vec::new(),
            },
            timeslots: Vec::new(),
            room_types: Vec::new(),
            rooms: Vec::new(),
            teachers: Vec::new(),
            groups: Vec::new(),
            sections: Vec::new(),
            activities: Vec::new(),
            overlap: OverlapRelation::unknown(0),
            unavailable: UnavailabilityIndex::empty(0),
            pinned: Vec::new(),
        }
    }

    fn group(id: u128, parent: Option<GroupIx>) -> GroupFact {
        GroupFact {
            id: Uuid::from_u128(id),
            name: format!("Group {id}"),
            kind: GroupKind::Class,
            parent,
            declared_size: Some(28),
            effective_members: 0,
            children_are_disjoint: DisjointnessDeclaration::NotStated,
        }
    }

    #[test]
    fn an_empty_problem_is_internally_consistent() {
        // A school part-way through setup loads to a problem with nothing in it. That is
        // not an error, and integrity must not invent one.
        assert_eq!(empty_problem().integrity(), Vec::new());
        assert_eq!(empty_problem().activity_count(), 0);
    }

    #[test]
    fn a_dangling_section_reference_is_reported() {
        let mut problem = empty_problem();
        problem.activities.push(ActivityFact {
            id: Uuid::from_u128(10),
            section: SectionIx::new(0),
            name: None,
            duration: DurationPeriods::SINGLE,
            split_index: 1,
        });
        assert_eq!(
            problem.integrity(),
            vec![IntegrityError::DanglingIndex {
                what: "activity",
                at: 0,
                field: "section",
                index: 0,
            }]
        );
    }

    #[test]
    fn two_rows_sharing_an_identity_are_reported() {
        // Fatal for persistence: a solution is written back against these identifiers.
        let mut problem = empty_problem();
        problem.room_types.push(RoomTypeFact {
            id: Uuid::from_u128(5),
            name: "Laboratory".into(),
        });
        problem.room_types.push(RoomTypeFact {
            id: Uuid::from_u128(5),
            name: "Also laboratory".into(),
        });
        assert_eq!(
            problem.integrity(),
            vec![IntegrityError::DuplicateIdentity {
                what: "roomType",
                id: Uuid::from_u128(5),
            }]
        );
    }

    #[test]
    fn a_group_that_is_its_own_parent_is_reported() {
        let mut problem = empty_problem();
        problem.groups.push(group(20, Some(GroupIx::new(0))));
        problem.overlap = crate::scheduling::population::OverlapRelation::unknown(1);
        assert_eq!(
            problem.integrity(),
            vec![IntegrityError::GroupIsItsOwnParent { at: 0 }]
        );
    }

    #[test]
    fn a_relation_that_does_not_cover_the_groups_is_reported() {
        // The mistake this catches: adding a group and forgetting to widen the relation,
        // which would make every pair involving it silently unaddressable.
        let mut problem = empty_problem();
        problem.groups.push(group(21, None));
        assert_eq!(
            problem.integrity(),
            vec![IntegrityError::OverlapRelationWrongSize {
                groups: 1,
                relation: 0,
            }]
        );
    }

    #[test]
    fn a_parent_edge_within_range_is_accepted() {
        let mut problem = empty_problem();
        problem.groups.push(group(22, None));
        problem.groups.push(group(23, Some(GroupIx::new(0))));
        problem.overlap = crate::scheduling::population::OverlapRelation::unknown(2);
        assert_eq!(problem.integrity(), Vec::new());
    }
}
