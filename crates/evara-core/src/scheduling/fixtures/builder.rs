//! Assembling a synthetic school into a [`Problem`].
//!
//! # What the builder is for
//!
//! Each fixture in [`super::catalogue`] is a short, readable description of a school: its
//! campuses, its bells, who teaches what to whom. The builder turns one into a `Problem`
//! with dense indices, bitsets and a compiled overlap relation, so that a fixture author
//! writes *intent* and never an index arithmetic bug.
//!
//! # The overlap relation is **declared**, not derived
//!
//! This is the single most important thing to understand about these fixtures.
//! [`SchoolBuilder::declare_overlap`] writes an [`Overlap`] into the problem's relation
//! because the fixture *states* what the answer must be. The seven ordered rules that
//! compute it from stored facts are
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
//! §3's, and they belong to M5 — this milestone does not implement them, and a builder that
//! did would be asserting its own output.
//!
//! The inputs those rules read are carried anyway, on every [`GroupFact`]: the parent edge,
//! the declared size, the effective member count, and the parent's
//! [`DisjointnessDeclaration`]. So when M5 writes the evaluator it can run it over these
//! fixtures and compare its derived answer against the declared one. That is the whole
//! design: **the fixture holds the inputs and the expected output, and the milestone in
//! between supplies the function.**
//!
//! The same split applies to bell-schedule resolution: a fixture states a
//! [`BellScheduleResolution`], and ADR 0033 §3's `resolve` is M4's.

use uuid::Uuid;

use super::rng::{IdGenerator, Rng};
use crate::scheduling::availability::{
    AvailabilityEntity, AvailabilityStatus, UnavailabilityIndex,
};
use crate::scheduling::clock::{DurationPeriods, TimeInterval, TimeOfDay};
use crate::scheduling::context::{
    BellScheduleFact, BellScheduleResolution, CampusFact, CycleDayFact, CycleFact, CycleKind,
    PeriodFact, PeriodKind, SchedulingContext, SchoolFact, TermFact,
};
use crate::scheduling::index::{
    ActivityIx, BellScheduleIx, CampusIx, CycleDayIx, GroupIx, PeriodIx, RoomIx, RoomTypeIx,
    SectionIx, TeacherIx, TimeslotIx,
};
use crate::scheduling::population::{
    AttendanceFloor, DisjointnessDeclaration, ExpectedAttendance, GroupMask, Overlap,
    OverlapRelation,
};
use crate::scheduling::problem::{
    ActivityFact, GroupFact, GroupKind, OtherSchool, Problem, RoomFact, RoomTypeFact, SectionFact,
    TeacherFact, TimeslotFact,
};

/// One period of a bell schedule, as a fixture states it.
#[derive(Debug, Clone)]
pub struct PeriodSpec {
    /// What the school calls it.
    pub label: String,
    /// Wall-clock span.
    pub interval: TimeInterval,
    /// What it is for.
    pub kind: PeriodKind,
}

impl PeriodSpec {
    /// A teaching period running `start..end`, as `'HH:MM'`.
    ///
    /// # Panics
    ///
    /// If the times are not well-formed `'HH:MM'` or are not in order. A fixture is written
    /// by hand in this repository, so a malformed literal is a bug to fix now rather than an
    /// error to propagate.
    #[must_use]
    pub fn teaching(label: &str, start: &str, end: &str) -> Self {
        Self::of(label, start, end, PeriodKind::Teaching)
    }

    /// A non-teaching period — a break, registration, anything a lesson cannot go in.
    ///
    /// # Panics
    ///
    /// As [`PeriodSpec::teaching`].
    #[must_use]
    pub fn non_teaching(label: &str, start: &str, end: &str, kind: PeriodKind) -> Self {
        assert!(
            !kind.is_teaching(),
            "{label} was declared non-teaching with a teaching kind"
        );
        Self::of(label, start, end, kind)
    }

    fn of(label: &str, start: &str, end: &str, kind: PeriodKind) -> Self {
        let start = TimeOfDay::parse(start).expect("a fixture's period times are well formed");
        let end = TimeOfDay::parse(end).expect("a fixture's period times are well formed");
        Self {
            label: label.to_owned(),
            interval: TimeInterval::new(start, end)
                .expect("a fixture's period ends after it starts"),
            kind,
        }
    }
}

/// A student group, as a fixture states it.
#[derive(Debug, Clone)]
pub struct GroupSpec {
    name: String,
    kind: GroupKind,
    parent: Option<GroupIx>,
    declared_size: Option<i64>,
    effective_members: i64,
    children_are_disjoint: DisjointnessDeclaration,
}

impl GroupSpec {
    /// A group with a declared size, no parent, no members and no declaration about its
    /// children — the shape of a group at a school that timetables without entering
    /// students.
    #[must_use]
    pub fn new(name: &str, kind: GroupKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            parent: None,
            declared_size: None,
            effective_members: 0,
            children_are_disjoint: DisjointnessDeclaration::NotStated,
        }
    }

    /// Declares how many students are in it. `None` means membership is the truth.
    #[must_use]
    pub const fn sized(mut self, size: i64) -> Self {
        self.declared_size = Some(size);
        self
    }

    /// Leaves the size unstated, so membership is the truth.
    #[must_use]
    pub const fn size_unstated(mut self) -> Self {
        self.declared_size = None;
        self
    }

    /// States it as part of another group.
    #[must_use]
    pub const fn child_of(mut self, parent: GroupIx) -> Self {
        self.parent = Some(parent);
        self
    }

    /// States how many students could actually be in the room — ADR 0035 §2's
    /// `effective_members`, already filtered for `is_active` and a non-`LEFT` spell.
    #[must_use]
    pub const fn with_members(mut self, members: i64) -> Self {
        self.effective_members = members;
        self
    }

    /// Answers "do my children share students?" — the future `children_are_disjoint`.
    #[must_use]
    pub const fn children(mut self, declaration: DisjointnessDeclaration) -> Self {
        self.children_are_disjoint = declaration;
        self
    }
}

/// A class section, as a fixture states it.
#[derive(Debug, Clone)]
pub struct SectionSpec {
    name: String,
    campus: Option<CampusIx>,
    bell_schedule: Option<BellScheduleResolution>,
    teachers: Vec<TeacherIx>,
    audience: Vec<GroupIx>,
    required_room_type: Option<RoomTypeIx>,
    expected_attendance: ExpectedAttendance,
    attendance_floor: AttendanceFloor,
    individually_enrolled: i64,
    declared_periods_per_cycle: Option<i64>,
}

impl SectionSpec {
    /// A section with no campus, no staff, no audience and no room requirement.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            campus: None,
            bell_schedule: None,
            teachers: Vec::new(),
            audience: Vec::new(),
            required_room_type: None,
            expected_attendance: ExpectedAttendance::Unknown,
            attendance_floor: AttendanceFloor { at_least: 0 },
            individually_enrolled: 0,
            declared_periods_per_cycle: None,
        }
    }

    /// Says which campus it is taught at.
    ///
    /// Leaving it unset means **not campus-constrained**, which is the ordinary state at a
    /// one-campus school and is not a missing value.
    #[must_use]
    pub const fn at(mut self, campus: CampusIx) -> Self {
        self.campus = Some(campus);
        self
    }

    /// States which bell schedule its lessons draw their slots from.
    ///
    /// Stated rather than resolved — see this module's header. Left unset, the builder
    /// stores a resolution against the only schedule in the grid, and refuses if there is
    /// more than one, so that a fixture cannot accidentally depend on a guess.
    #[must_use]
    pub fn schedule(mut self, resolution: BellScheduleResolution) -> Self {
        self.bell_schedule = Some(resolution);
        self
    }

    /// Adds a teacher.
    #[must_use]
    pub fn taught_by(mut self, teacher: TeacherIx) -> Self {
        self.teachers.push(teacher);
        self
    }

    /// Adds an attending group.
    #[must_use]
    pub fn taught_to(mut self, group: GroupIx) -> Self {
        self.audience.push(group);
        self
    }

    /// Requires a kind of room.
    #[must_use]
    pub const fn requires_room_type(mut self, room_type: RoomTypeIx) -> Self {
        self.required_room_type = Some(room_type);
        self
    }

    /// States how many people a lesson should be assumed to hold.
    #[must_use]
    pub const fn attendance(mut self, attendance: ExpectedAttendance) -> Self {
        self.expected_attendance = attendance;
        self
    }

    /// States a floor on the attendance, from member rows.
    #[must_use]
    pub const fn attendance_at_least(mut self, at_least: i64) -> Self {
        self.attendance_floor = AttendanceFloor { at_least };
        self
    }

    /// Names students on the roster, which contributes nothing to scheduling.
    #[must_use]
    pub const fn with_roster(mut self, students: i64) -> Self {
        self.individually_enrolled = students;
        self
    }

    /// Records what the course declared it needs per cycle.
    #[must_use]
    pub const fn declares_periods(mut self, periods: i64) -> Self {
        self.declared_periods_per_cycle = Some(periods);
        self
    }
}

/// Assembles one school into a [`Problem`].
///
/// Entities are appended in the order the fixture declares them, and the index returned by
/// each `add_*` is that position — so a fixture's own reading order *is* the problem's index
/// order, which is what makes a generated fixture diffable and its assertions stable.
#[derive(Debug)]
pub struct SchoolBuilder {
    ids: IdGenerator,
    rng: Rng,
    school: SchoolFact,
    cycle: CycleFact,
    term: Option<TermFact>,
    campuses: Vec<CampusFact>,
    cycle_days: Vec<CycleDayFact>,
    bell_schedules: Vec<BellScheduleFact>,
    periods: Vec<PeriodFact>,
    timeslots: Vec<TimeslotFact>,
    room_types: Vec<RoomTypeFact>,
    rooms: Vec<RoomFact>,
    teachers: Vec<TeacherFact>,
    groups: Vec<GroupFact>,
    sections: Vec<SectionFact>,
    activities: Vec<ActivityFact>,
    declared_overlaps: Vec<(GroupIx, GroupIx, Overlap)>,
    availability: Vec<(AvailabilityEntity, TimeslotIx, AvailabilityStatus)>,
}

/// A teacher, as a fixture states it.
///
/// Separate from `TeacherFact` only so that a fixture can hand over a project-wide identity
/// it already minted — which is how a teacher shared between two schools keeps **one**
/// identity in both problems.
#[derive(Debug, Clone)]
pub struct TeacherFactSpec {
    name: String,
    id: Option<Uuid>,
    eligible_campuses: Option<Vec<CampusIx>>,
    also_at: Vec<OtherSchool>,
}

impl TeacherFactSpec {
    /// A teacher with a fresh identity, unnarrowed campus eligibility and one school.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            id: None,
            eligible_campuses: None,
            also_at: Vec::new(),
        }
    }

    /// Reuses an identity minted elsewhere.
    ///
    /// The mechanism behind
    /// [ADR 0021](../../../../docs/adr/0021-project-level-people-and-school-membership.md): a
    /// person belongs to the project and a membership belongs to the school, so somebody
    /// employed by two schools appears in two problems under **one** `teacher.id`.
    #[must_use]
    pub const fn with_identity(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Narrows them to particular campuses.
    #[must_use]
    pub fn eligible_for(mut self, campuses: &[CampusIx]) -> Self {
        self.eligible_campuses = Some(campuses.to_vec());
        self
    }

    /// Records another school that also employs them.
    #[must_use]
    pub fn also_at(mut self, school: OtherSchool) -> Self {
        self.also_at.push(school);
        self
    }
}

/// A room, as a fixture states it.
#[derive(Debug, Clone)]
pub struct RoomFactSpec {
    name: String,
    id: Option<Uuid>,
    campuses: Vec<CampusIx>,
    room_type: Option<RoomTypeIx>,
    capacity: i64,
    is_bookable: bool,
    also_at: Vec<OtherSchool>,
}

impl RoomFactSpec {
    /// A bookable room of the given capacity, reaching no campus and of no particular type.
    #[must_use]
    pub fn new(name: &str, capacity: i64) -> Self {
        Self {
            name: name.to_owned(),
            id: None,
            campuses: Vec::new(),
            room_type: None,
            capacity,
            is_bookable: true,
            also_at: Vec::new(),
        }
    }

    /// Reuses an identity minted elsewhere — one physical room, one identity (ADR 0014).
    #[must_use]
    pub const fn with_identity(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Makes the room reachable from a campus.
    #[must_use]
    pub fn on_campus(mut self, campus: CampusIx) -> Self {
        self.campuses.push(campus);
        self
    }

    /// Classifies the room.
    #[must_use]
    pub const fn of_type(mut self, room_type: RoomTypeIx) -> Self {
        self.room_type = Some(room_type);
        self
    }

    /// Takes the room out of every candidate set.
    #[must_use]
    pub const fn not_bookable(mut self) -> Self {
        self.is_bookable = false;
        self
    }

    /// Records another school whose campuses also reach it.
    #[must_use]
    pub fn also_at(mut self, school: OtherSchool) -> Self {
        self.also_at.push(school);
        self
    }
}

impl SchoolBuilder {
    /// Starts a school.
    ///
    /// `seed` drives every generated identifier and every arbitrary choice, so two seeds
    /// produce two structurally identical schools with disjoint identities.
    #[must_use]
    pub fn new(seed: u64, school_name: &str, cycle_name: &str, cycle_kind: CycleKind) -> Self {
        let mut ids = IdGenerator::new(seed);
        let school = SchoolFact {
            id: ids.next_id(),
            name: school_name.to_owned(),
            // A plausible IANA name, carried verbatim and parsed by nothing. One school, one
            // zone, and the zone cancels on both sides of every comparison.
            timezone: "Europe/London".to_owned(),
        };
        let cycle = CycleFact {
            id: ids.next_id(),
            name: cycle_name.to_owned(),
            kind: cycle_kind,
        };
        Self {
            ids,
            // A different stream from the identifier generator's, so that adding an entity
            // does not shift every later arbitrary choice.
            rng: Rng::new(seed ^ 0x5CED_1E5F_A11D_0C7E),
            school,
            cycle,
            term: None,
            campuses: Vec::new(),
            cycle_days: Vec::new(),
            bell_schedules: Vec::new(),
            periods: Vec::new(),
            timeslots: Vec::new(),
            room_types: Vec::new(),
            rooms: Vec::new(),
            teachers: Vec::new(),
            groups: Vec::new(),
            sections: Vec::new(),
            activities: Vec::new(),
            declared_overlaps: Vec::new(),
            availability: Vec::new(),
        }
    }

    /// The school's identity, so another problem can name it as a sharing partner.
    #[must_use]
    pub fn school_reference(&self) -> OtherSchool {
        OtherSchool {
            id: self.school.id,
            name: self.school.name.clone(),
        }
    }

    /// Mints an identifier from this school's deterministic sequence.
    ///
    /// Used when a fixture needs a shared identity — a teacher or a room that two schools
    /// both hold — before the second school exists.
    pub fn mint_id(&mut self) -> Uuid {
        self.ids.next_id()
    }

    /// Names the term the grid is materialised for.
    pub fn term(&mut self, name: &str) {
        let id = self.ids.next_id();
        self.term = Some(TermFact {
            id,
            name: name.to_owned(),
        });
    }

    /// Adds a campus.
    pub fn add_campus(&mut self, name: &str) -> CampusIx {
        let id = self.ids.next_id();
        self.campuses.push(CampusFact {
            id,
            name: name.to_owned(),
        });
        CampusIx::new(last_index(self.campuses.len()))
    }

    /// Adds `count` cycle positions, labelled from `labels` where it reaches and numbered
    /// beyond it.
    ///
    /// The labels are display only. `ordinal` is a day's whole scheduling identity, and a
    /// six-day rotation is the same kind of thing to every rule here as a five-day week.
    pub fn add_cycle_days(&mut self, labels: &[&str], count: usize) -> Vec<CycleDayIx> {
        (0..count)
            .map(|n| {
                let id = self.ids.next_id();
                let ordinal = u16::try_from(self.cycle_days.len() + 1).unwrap_or(u16::MAX);
                let label = labels
                    .get(n)
                    .map_or_else(|| format!("Day {ordinal}"), |l| (*l).to_owned());
                self.cycle_days.push(CycleDayFact { id, ordinal, label });
                CycleDayIx::new(last_index(self.cycle_days.len()))
            })
            .collect()
    }

    /// Adds a bell schedule and its periods.
    ///
    /// `campus` of `None` makes it school-wide, which by its own definition serves every
    /// campus. Adding a schedule does **not** put slots in the grid — call
    /// [`SchoolBuilder::materialise`] for that, because `G` is derived from what is
    /// materialised and not from what exists (ADR 0033 §1).
    pub fn add_bell_schedule(
        &mut self,
        name: &str,
        campus: Option<CampusIx>,
        periods: &[PeriodSpec],
    ) -> BellScheduleIx {
        let schedule_ix =
            BellScheduleIx::new(u32::try_from(self.bell_schedules.len()).unwrap_or(0));
        let mut period_ixs = Vec::with_capacity(periods.len());
        for (n, spec) in periods.iter().enumerate() {
            let id = self.ids.next_id();
            self.periods.push(PeriodFact {
                id,
                bell_schedule: schedule_ix,
                ordinal: u16::try_from(n + 1).unwrap_or(u16::MAX),
                label: spec.label.clone(),
                interval: spec.interval,
                kind: spec.kind,
            });
            period_ixs.push(PeriodIx::new(last_index(self.periods.len())));
        }
        let id = self.ids.next_id();
        self.bell_schedules.push(BellScheduleFact {
            id,
            name: name.to_owned(),
            campus,
            periods: period_ixs,
        });
        schedule_ix
    }

    /// Materialises one bell schedule's slots across every cycle position.
    ///
    /// This is what puts a schedule in `G`. A schedule that is defined and never
    /// materialised has no slots, so it is a candidate for nothing — correctly, because
    /// there is nothing to place a lesson in (ADR 0033 §1, test 8).
    ///
    /// Non-teaching periods are materialised too: `no-teaching-in-period` remains a
    /// registered constraint because manual editing can name such a slot, so the slot has to
    /// exist to be refused.
    pub fn materialise(&mut self, schedule: BellScheduleIx) -> Vec<TimeslotIx> {
        let periods = self.bell_schedules[schedule.get()].periods.clone();
        let campus = self.bell_schedules[schedule.get()].campus;
        let days: Vec<CycleDayIx> = (0..self.cycle_days.len())
            .map(|d| CycleDayIx::new(last_index(d + 1)))
            .collect();

        let mut created = Vec::with_capacity(days.len() * periods.len());
        for day in days {
            for period in &periods {
                let id = self.ids.next_id();
                let fact = &self.periods[period.get()];
                self.timeslots.push(TimeslotFact {
                    id,
                    cycle_day: day,
                    period: *period,
                    bell_schedule: schedule,
                    campus,
                    is_teaching: fact.kind.is_teaching(),
                    interval: fact.interval,
                });
                created.push(TimeslotIx::new(last_index(self.timeslots.len())));
            }
        }
        created
    }

    /// Adds a kind of room.
    pub fn add_room_type(&mut self, name: &str) -> RoomTypeIx {
        let id = self.ids.next_id();
        self.room_types.push(RoomTypeFact {
            id,
            name: name.to_owned(),
        });
        RoomTypeIx::new(last_index(self.room_types.len()))
    }

    /// Adds a room.
    pub fn add_room(&mut self, spec: RoomFactSpec) -> RoomIx {
        let id = spec.id.unwrap_or_else(|| self.ids.next_id());
        self.rooms.push(RoomFact {
            id,
            name: spec.name,
            campuses: spec.campuses,
            room_type: spec.room_type,
            capacity: spec.capacity,
            is_bookable: spec.is_bookable,
            also_at: spec.also_at,
        });
        RoomIx::new(last_index(self.rooms.len()))
    }

    /// Adds a teacher.
    pub fn add_teacher(&mut self, spec: TeacherFactSpec) -> TeacherIx {
        let id = spec.id.unwrap_or_else(|| self.ids.next_id());
        self.teachers.push(TeacherFact {
            id,
            name: spec.name,
            eligible_campuses: spec.eligible_campuses,
            also_at: spec.also_at,
        });
        TeacherIx::new(last_index(self.teachers.len()))
    }

    /// Adds a student group.
    pub fn add_group(&mut self, spec: GroupSpec) -> GroupIx {
        let id = self.ids.next_id();
        self.groups.push(GroupFact {
            id,
            name: spec.name,
            kind: spec.kind,
            parent: spec.parent,
            declared_size: spec.declared_size,
            effective_members: spec.effective_members,
            children_are_disjoint: spec.children_are_disjoint,
        });
        GroupIx::new(last_index(self.groups.len()))
    }

    /// States what the overlap evaluator must conclude about one pair of groups.
    ///
    /// **Declared, not derived** — see this module's header. The pair is recorded in both
    /// directions when the problem is finished, because the relation is symmetric.
    pub fn declare_overlap(&mut self, a: GroupIx, b: GroupIx, overlap: Overlap) {
        self.declared_overlaps.push((a, b, overlap));
    }

    /// Adds a class section.
    ///
    /// # Panics
    ///
    /// If the section states no bell-schedule resolution and the grid does not hold exactly
    /// one materialised schedule. A fixture must not depend on a default here: ADR 0033's
    /// whole decision is that there is no fallback, so a builder that picked one would make
    /// the fixtures agree with a rule the product does not have.
    pub fn add_section(&mut self, spec: SectionSpec) -> SectionIx {
        let resolution = spec.bell_schedule.unwrap_or_else(|| {
            let materialised = self.materialised_schedules();
            assert_eq!(
                materialised.len(),
                1,
                "section {:?} must state its bell schedule: {} are materialised, and there is \
                 no default to fall back on",
                spec.name,
                materialised.len()
            );
            BellScheduleResolution::Resolved {
                schedule: materialised[0],
            }
        });

        let room_candidates = self.candidate_rooms(spec.campus, spec.required_room_type);
        let id = self.ids.next_id();
        self.sections.push(SectionFact {
            id,
            name: spec.name,
            campus: spec.campus,
            bell_schedule: resolution,
            teachers: spec.teachers,
            audience: spec.audience,
            // Filled in by `finish`, once every group exists and the relation is compiled.
            audience_mask: GroupMask::default(),
            clash_mask: GroupMask::default(),
            required_room_type: spec.required_room_type,
            room_candidates,
            expected_attendance: spec.expected_attendance,
            attendance_floor: spec.attendance_floor,
            individually_enrolled: spec.individually_enrolled,
            declared_periods_per_cycle: spec.declared_periods_per_cycle,
        });
        SectionIx::new(last_index(self.sections.len()))
    }

    /// Adds one occurrence per entry of `durations`.
    ///
    /// One row per occurrence, `split_index` assigned from 1 and never renumbered. Four
    /// single periods a cycle is `&[1, 1, 1, 1]` — four activities — and a double is a `2`,
    /// which is one activity of two consecutive periods and not two activities.
    ///
    /// # Panics
    ///
    /// On a duration of zero, which `CHECK (duration_periods >= 1)` refuses.
    pub fn add_activities(&mut self, section: SectionIx, durations: &[u16]) -> Vec<ActivityIx> {
        let next_split = self
            .activities
            .iter()
            .filter(|a| a.section == section)
            .count();
        durations
            .iter()
            .enumerate()
            .map(|(n, periods)| {
                let id = self.ids.next_id();
                self.activities.push(ActivityFact {
                    id,
                    section,
                    name: None,
                    duration: DurationPeriods::new(*periods)
                        .expect("a fixture's occurrence lasts at least one period"),
                    split_index: u16::try_from(next_split + n + 1).unwrap_or(u16::MAX),
                });
                ActivityIx::new(last_index(self.activities.len()))
            })
            .collect()
    }

    /// Records an availability rule, already compiled onto one slot.
    ///
    /// Compiled rather than stored, because the expansion from a stored rule's granularity
    /// onto the slots it names is M4's and widening it is exactly the mistake ADR 0033 §7
    /// forbids. A fixture therefore names the slots directly, which is also what makes its
    /// expectations checkable without a loader.
    pub fn restrict(
        &mut self,
        entity: AvailabilityEntity,
        slot: TimeslotIx,
        status: AvailabilityStatus,
    ) {
        self.availability.push((entity, slot, status));
    }

    /// The schedules that have slots in the grid: `G`.
    #[must_use]
    pub fn materialised_schedules(&self) -> Vec<BellScheduleIx> {
        let mut found: Vec<BellScheduleIx> =
            self.timeslots.iter().map(|s| s.bell_schedule).collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The rooms a section with this campus and requirement could use.
    ///
    /// Delegates to [`RoomFact::is_candidate_for`], which is the single implementation of the
    /// candidate domain — see that method for why it is not restated here.
    fn candidate_rooms(
        &self,
        campus: Option<CampusIx>,
        required_type: Option<RoomTypeIx>,
    ) -> Vec<RoomIx> {
        self.rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| room.is_candidate_for(campus, required_type))
            .map(|(ix, _)| RoomIx::new(last_index(ix + 1)))
            .collect()
    }

    /// A deterministic arbitrary choice, for the places a fixture does not care which.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Finishes the school into a problem.
    ///
    /// Compiles the overlap relation from the declared pairs, builds each section's audience
    /// and clash masks from it, and compiles the availability rules onto their slots.
    ///
    /// Every pair of groups the fixture did not mention stays
    /// [`Overlap::Indeterminate`], which is the honest default and the state every existing
    /// project is in. It is **not** `Disjoint`: a relation that defaulted to disjoint would
    /// make the fixtures agree with the silent-clash failure ADR 0035 §6 exists to prevent.
    #[must_use]
    pub fn finish(mut self) -> Problem {
        let mut overlap = OverlapRelation::unknown(self.groups.len());
        for (a, b, answer) in &self.declared_overlaps {
            overlap.set(*a, *b, *answer);
        }

        for section in &mut self.sections {
            let mut audience_mask = GroupMask::with_capacity(self.groups.len());
            for group in &section.audience {
                audience_mask.insert(*group);
            }
            section.clash_mask = overlap.clash_mask(&audience_mask);
            section.audience_mask = audience_mask;
        }

        let mut unavailable = UnavailabilityIndex::empty(self.timeslots.len());
        for (entity, slot, status) in &self.availability {
            unavailable.insert(*entity, *slot, *status);
        }

        Problem {
            context: SchedulingContext {
                school: self.school,
                cycle: self.cycle,
                term: self.term,
                campuses: self.campuses,
                cycle_days: self.cycle_days,
                bell_schedules: self.bell_schedules,
                periods: self.periods,
            },
            timeslots: self.timeslots,
            room_types: self.room_types,
            rooms: self.rooms,
            teachers: self.teachers,
            groups: self.groups,
            sections: self.sections,
            activities: self.activities,
            overlap,
            unavailable,
            // Nothing populates this: `activity.is_locked` is not in V15, and `activity-fixed`
            // joins the constraint set once locks exist.
            pinned: Vec::new(),
        }
    }
}

/// The index of the last element of a collection of `len` items.
///
/// A named helper because the saturating conversion would otherwise be repeated at twelve
/// call sites, and `len - 1` on an empty collection is the one way this file could panic.
fn last_index(len: usize) -> u32 {
    u32::try_from(len.saturating_sub(1)).unwrap_or(u32::MAX)
}
