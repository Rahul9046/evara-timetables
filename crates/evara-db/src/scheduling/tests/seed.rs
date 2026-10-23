//! Database-backed scheduling fixtures: real `.evaraproj` files, built through the
//! repositories.
//!
//! # Why these exist beside M2's
//!
//! `evara_core::scheduling::fixtures` builds a [`Problem`](evara_core::scheduling::Problem)
//! in memory, which is what lets `evara-core`, `evara-constraints` and `evara-solver` all
//! use one — none of those three may open a database. That is the right shape for testing
//! the *vocabulary* and the rules over it, and it cannot test a loader at all: a loader's job
//! is precisely the translation those fixtures skip.
//!
//! So these are the other half. Each writes a synthetic school into a temporary project
//! **through the ordinary repository API** — no raw SQL, no bypassed trigger, no second
//! project format — and the test then closes the file, reopens it, and asks the loader what
//! it holds. A fixture that could only be built by writing rows the repositories refuse
//! would be testing the loader against a database the product cannot produce.
//!
//! The one deliberate exception is [`School::orphan_availability_rule`], which is documented
//! where it is defined: the row it writes is one no repository can create, and reaching it is
//! the whole point of the finding it provokes.
//!
//! # Deterministic, and synthetic
//!
//! No clock is read and no randomness is used: every name, size and time is a constant or a
//! function of an index, so two runs build identical schools. Identifiers differ between runs
//! — they are UUIDv7 and the repositories mint them — which is exactly why the loader's
//! contract is about *order* rather than about particular values, and why every assertion
//! here is written against order, counts and resolved facts rather than against an identifier
//! somebody wrote down.
//!
//! Every name is invented, per `CLAUDE.md` rule 3. No real school, teacher or student.

use evara_core::calendar::WorkingDays;
use uuid::Uuid;

use crate::repo::availability::{AvailabilityEntity, AvailabilityRuleInput, AvailabilityStatus};
use crate::repo::people::GroupKind;
use crate::repo::time_model::{CycleKind, MaterialisationRequest, PeriodKind};
use crate::scheduling::SchedulingSelection;
use crate::workspace::PROJECT_EXTENSION;
use crate::{
    AcademicYearInput, Assembly, CampusInput, ClassSectionInput, CourseInput, CycleInput, Database,
    PeriodInput, PeriodStructureInput, RoomInput, RoomTypeInput, SchoolInput, StudentGroupInput,
    StudentInput, SubjectInput, TeacherInput, TermInput, Workspace,
};

/// The version string every fixture project records.
const APP_VERSION: &str = "0.1.0-test";

/// A temporary project, and the workspace that owns it.
///
/// Holds the `TempDir` so the file outlives every borrow of the database and is removed when
/// the test ends. Nothing here touches a project the user owns.
pub(super) struct Project {
    /// Held so the project file outlives every borrow, and is removed with the test.
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    workspace: Workspace,
}

impl Project {
    /// Creates an empty project in a fresh temporary directory.
    pub(super) fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let settings = dir.path().join("settings.sqlite");
        let mut workspace =
            Workspace::with_settings_at(&settings, APP_VERSION).expect("settings database");
        let path = dir.path().join(format!("fixture.{PROJECT_EXTENSION}"));
        workspace.create_project(&path).expect("create project");
        Self {
            _dir: dir,
            path,
            workspace,
        }
    }

    /// The open database, for writing a fixture.
    pub(super) fn db(&mut self) -> &mut Database {
        self.workspace.open_database_for_test()
    }

    /// Closes the project and opens it again.
    ///
    /// The step the round-trip tests turn on: everything the loader reads has to survive the
    /// connection that wrote it being dropped, and a fixture validated through the same
    /// handle that built it would not prove that. Dropping the `Database` closes the file and
    /// checkpoints the WAL, so the reopened connection reads what is actually on disk.
    pub(super) fn reopen(&mut self) {
        assert!(self.workspace.close_project(), "a project was open");
        self.workspace
            .open_project(&self.path)
            .expect("reopen project");
    }

    /// The project file's path, for a test that opens it a second way.
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Compiles one selection, through the read-only handle.
    pub(super) fn load(&self, selection: &SchedulingSelection) -> crate::Result<Assembly> {
        self.workspace
            .scheduling()
            .expect("a project is open")
            .load(selection)
    }
}

/// One seeded school, and the identifiers a test needs to address it.
///
/// Fields rather than accessors: a fixture is data, and a test that has to call six methods
/// to find out what it built is harder to read than one that names the field.
pub(super) struct School {
    /// The school.
    pub(super) id: Uuid,
    /// Its campuses, in creation order.
    pub(super) campuses: Vec<Uuid>,
    /// The cycle the grid is defined over.
    pub(super) cycle: Uuid,
    /// The term, where the fixture made one.
    pub(super) term: Option<Uuid>,
    /// Its bell schedules (`period_structure` rows), in creation order.
    pub(super) schedules: Vec<Uuid>,
    /// Its room types, in creation order.
    pub(super) room_types: Vec<Uuid>,
    /// Its rooms, in creation order.
    pub(super) rooms: Vec<Uuid>,
    /// Its teachers, in creation order.
    pub(super) teachers: Vec<Uuid>,
    /// Its student groups, in creation order.
    pub(super) groups: Vec<Uuid>,
    /// Its class sections, in creation order.
    pub(super) sections: Vec<Uuid>,
}

impl School {
    /// What to pass the loader.
    pub(super) const fn selection(&self) -> SchedulingSelection {
        SchedulingSelection {
            school_id: self.id,
            cycle_id: self.cycle,
            term_id: self.term,
        }
    }

    /// Creates a school with `campuses` campuses and a cycle of `days` positions.
    ///
    /// A `CUSTOM` cycle rather than a `WEEKLY` one, because a weekly cycle is *the week* —
    /// all seven positions, always — and a fixture that wants a five-day grid should say
    /// five rather than materialise seven and ignore two.
    pub(super) fn new(project: &mut Project, name: &str, campuses: &[&str], days: i64) -> Self {
        let id = project
            .db()
            .structure()
            .create_school(&SchoolInput {
                name: name.to_owned(),
                code: None,
                timezone: "Europe/London".to_owned(),
                locale: "en-GB".to_owned(),
                working_days: WorkingDays::MONDAY_TO_FRIDAY,
            })
            .expect("create school")
            .id;

        let campuses: Vec<Uuid> = campuses
            .iter()
            .enumerate()
            .map(|(n, campus)| {
                project
                    .db()
                    .structure()
                    .create_campus(&CampusInput {
                        school_id: id,
                        name: (*campus).to_owned(),
                        code: format!("C{n}"),
                        address: None,
                    })
                    .expect("create campus")
                    .id
            })
            .collect();

        let cycle = project
            .db()
            .time_model()
            .create_cycle(&CycleInput {
                school_id: id,
                name: format!("{name} cycle"),
                kind: CycleKind::Custom,
                day_count: days,
                week_count: 1,
                is_default: true,
            })
            .expect("create cycle")
            .id;

        Self {
            id,
            campuses,
            cycle,
            term: None,
            schedules: Vec::new(),
            room_types: Vec::new(),
            rooms: Vec::new(),
            teachers: Vec::new(),
            groups: Vec::new(),
            sections: Vec::new(),
        }
    }

    /// Adds an academic year with one term, and selects that term.
    pub(super) fn with_term(&mut self, project: &mut Project) -> &mut Self {
        let year = project
            .db()
            .structure()
            .create_academic_year(&AcademicYearInput {
                school_id: self.id,
                name: "2026/27".to_owned(),
                starts_on: "2026-09-01".to_owned(),
                ends_on: "2027-07-20".to_owned(),
            })
            .expect("create academic year")
            .id;
        self.term = Some(
            project
                .db()
                .structure()
                .create_term(&TermInput {
                    academic_year_id: year,
                    name: "Autumn".to_owned(),
                    ordinal: 1,
                    starts_on: "2026-09-01".to_owned(),
                    ends_on: "2026-12-18".to_owned(),
                })
                .expect("create term")
                .id,
        );
        self
    }

    /// Adds a bell schedule and its periods, and materialises it into the grid.
    ///
    /// `periods` is `(label, start, end, teaching)`. `campus` of `None` makes the schedule
    /// school-wide, which is ADR 0033 §2 clause 1's case.
    ///
    /// Materialisation goes through [`TimeModel::apply_materialisation`], the only thing that
    /// may create a `timeslot`: a slot that did not come from materialisation would have no
    /// logical identity to preserve.
    pub(super) fn bells(
        &mut self,
        project: &mut Project,
        name: &str,
        campus: Option<usize>,
        periods: &[(&str, &str, &str, bool)],
    ) -> &mut Self {
        let schedule = project
            .db()
            .time_model()
            .create_period_structure(&PeriodStructureInput {
                school_id: self.id,
                campus_id: campus.map(|at| self.campuses[at]),
                name: name.to_owned(),
                is_default: self.schedules.is_empty(),
            })
            .expect("create period structure")
            .id;

        for (ordinal, (label, starts_at, ends_at, teaching)) in periods.iter().enumerate() {
            project
                .db()
                .time_model()
                .create_period(&PeriodInput {
                    period_structure_id: schedule,
                    ordinal: i64::try_from(ordinal).expect("small") + 1,
                    label: (*label).to_owned(),
                    starts_at: (*starts_at).to_owned(),
                    ends_at: (*ends_at).to_owned(),
                    kind: if *teaching {
                        PeriodKind::Teaching
                    } else {
                        PeriodKind::Break
                    },
                    counts_as_load: *teaching,
                })
                .expect("create period");
        }

        self.schedules.push(schedule);
        self
    }

    /// Materialises every schedule added so far into the selected grid.
    ///
    /// Separate from [`School::bells`] so a fixture can define a schedule and deliberately
    /// **not** materialise it — ADR 0033 test 8's case, and the one M3 found M2 had
    /// documented wrongly.
    pub(super) fn materialise(&mut self, project: &mut Project) -> &mut Self {
        for schedule in self.schedules.clone() {
            let request = MaterialisationRequest::new(self.cycle, schedule, self.term);
            project
                .db()
                .time_model()
                .apply_materialisation(&request)
                .expect("materialise grid");
        }
        self
    }

    /// Adds a room type.
    pub(super) fn room_type(&mut self, project: &mut Project, name: &str) -> Uuid {
        let id = project
            .db()
            .structure()
            .create_room_type(&RoomTypeInput {
                school_id: self.id,
                name: name.to_owned(),
                code: format!("RT{}", self.room_types.len()),
            })
            .expect("create room type")
            .id;
        self.room_types.push(id);
        id
    }

    /// Adds a room that may not be booked.
    ///
    /// `is_bookable = 0` removes a room from **every** activity's candidate set and is
    /// unrelated to how many people fit (ADR 0034 §6). Stored, editable, and read by nothing
    /// until the loader built a candidate domain.
    pub(super) fn unbookable_room(
        &mut self,
        project: &mut Project,
        name: &str,
        campus: usize,
        room_type: Option<Uuid>,
    ) -> Uuid {
        let id = project
            .db()
            .structure()
            .create_room(
                &RoomInput {
                    campus_id: self.campuses[campus],
                    building_id: None,
                    room_type_id: room_type,
                    name: name.to_owned(),
                    code: format!("R{}", self.rooms.len()),
                    capacity: 40,
                    is_bookable: false,
                    notes: None,
                },
                &[],
            )
            .expect("create room")
            .id;
        self.rooms.push(id);
        id
    }

    /// Adds a room on one campus, optionally shared with others.
    pub(super) fn room(
        &mut self,
        project: &mut Project,
        name: &str,
        campus: usize,
        room_type: Option<Uuid>,
        capacity: i64,
        shared_with: &[usize],
    ) -> Uuid {
        let shared: Vec<Uuid> = shared_with.iter().map(|at| self.campuses[*at]).collect();
        let id = project
            .db()
            .structure()
            .create_room(
                &RoomInput {
                    campus_id: self.campuses[campus],
                    building_id: None,
                    room_type_id: room_type,
                    name: name.to_owned(),
                    code: format!("R{}", self.rooms.len()),
                    capacity,
                    is_bookable: true,
                    notes: None,
                },
                &shared,
            )
            .expect("create room")
            .id;
        self.rooms.push(id);
        id
    }

    /// Adds a teacher, employed here and active.
    ///
    /// Created at this school, so this is their home school and their membership exists and
    /// is active — the ordinary state, and the one every other fixture varies from.
    pub(super) fn teacher(&mut self, project: &mut Project, family: &str) -> Uuid {
        let id = project
            .db()
            .people()
            .create_teacher(&TeacherInput {
                school_id: self.id,
                code: None,
                given_name: "A.".to_owned(),
                family_name: family.to_owned(),
                email: None,
                home_campus_id: None,
                fte_percent: 100,
                max_periods_per_day: None,
                max_periods_per_cycle: None,
                max_consecutive_periods: None,
                notes: None,
                is_active: true,
            })
            .expect("create teacher")
            .id;
        self.teachers.push(id);
        id
    }

    /// Adds a student group with a declared size.
    ///
    /// `size` of `None` means *membership is the truth*, which is a different statement from
    /// a declared zero and must not be merged with it — ADR 0006 requires a full timetable to
    /// be buildable from declared sizes with no student records at all.
    pub(super) fn group(
        &mut self,
        project: &mut Project,
        name: &str,
        size: Option<i64>,
        parent: Option<Uuid>,
    ) -> Uuid {
        let id = project
            .db()
            .people()
            .create_student_group(&StudentGroupInput {
                school_id: self.id,
                name: name.to_owned(),
                code: None,
                kind: GroupKind::Class,
                year_level_id: None,
                parent_group_id: parent,
                size,
                campus_id: None,
                children_are_disjoint: None,
            })
            .expect("create student group")
            .id;
        self.groups.push(id);
        id
    }

    /// Answers ADR 0035 §4's question for one group: do its children share students?
    ///
    /// `V16` added the column, and no screen offers a control for it yet — so a fixture sets
    /// it the way the eventual screen will, through `update_student_group` carrying the row's
    /// revision. Three states: `None` is "not stated", which is what every migrated project
    /// holds.
    pub(super) fn declare_children(project: &mut Project, group: Uuid, disjoint: Option<bool>) {
        let current = project
            .db()
            .people()
            .student_group(group)
            .expect("read the group")
            .expect("it is there");
        project
            .db()
            .people()
            .update_student_group(
                group,
                crate::Expected::Rev(current.rev),
                &StudentGroupInput {
                    school_id: current.school_id,
                    name: current.name,
                    code: current.code,
                    kind: current.kind,
                    year_level_id: current.year_level_id,
                    parent_group_id: current.parent_group_id,
                    size: current.size,
                    campus_id: current.campus_id,
                    children_are_disjoint: disjoint,
                },
            )
            .expect("declare children disjointness");
    }

    /// Adds a student and puts them in a group.
    ///
    /// `create_student` writes the `student_school_enrolment` spell itself — a student created
    /// at a school is enrolled there — so there is no second call to make, and making one is
    /// refused as a duplicate. That matters for the fixture because an effective member is a
    /// conjunction: `student.is_active` **and** a spell whose status is not `LEFT` **and** a
    /// membership row. Two of the three come free; the group membership is what this adds.
    pub(super) fn student_in(&mut self, project: &mut Project, group: Uuid, n: usize) -> Uuid {
        let student = project
            .db()
            .people()
            .create_student(&StudentInput {
                school_id: self.id,
                code: None,
                given_name: "B.".to_owned(),
                family_name: format!("Pupil{n}"),
                email: None,
                year_level_id: None,
                campus_id: None,
                is_active: true,
            })
            .expect("create student")
            .id;
        project
            .db()
            .people()
            .add_student_group_member(&crate::StudentGroupMemberInput {
                student_group_id: group,
                student_id: student,
            })
            .expect("add group member");
        student
    }

    /// Ends a student's spell at this school, keeping the membership row.
    ///
    /// The other half of ADR 0035 §2's filter: `student_group_member` deliberately keeps
    /// history, so a child who has left is still a member of the group they were in. Right
    /// for the record, wrong for a room — counting them would manufacture clashes between
    /// groups that no longer share anybody.
    pub(super) fn student_leaves(&mut self, project: &mut Project, student: Uuid) {
        project
            .db()
            .membership()
            .leave_school(self.id, student, crate::Expected::Any, Some("2026-10-01"))
            .expect("end the spell");
    }

    /// Adds a subject.
    ///
    /// Separate from [`School::section`] because a school teaches a handful of subjects and
    /// offers many courses of each, and a fixture that minted a subject per section would
    /// describe a school nobody has.
    pub(super) fn subject(&mut self, project: &mut Project, name: &str) -> Uuid {
        project
            .db()
            .curriculum()
            .create_subject(&SubjectInput {
                school_id: self.id,
                department_id: None,
                name: name.to_owned(),
                code: None,
                description: None,
            })
            .expect("create subject")
            .id
    }

    /// Adds a course of `subject_id` and one class section of it, and returns the section.
    ///
    /// One call because the rest of the chain has no branches in a fixture: `course →
    /// class_section`, and neither carries a `school_id` — ownership is resolved through
    /// `subject`, which is what `create_course` and `create_class_section` check.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn section(
        &mut self,
        project: &mut Project,
        subject_id: Uuid,
        name: &str,
        campus: Option<usize>,
        required_room_type: Option<Uuid>,
        periods_per_cycle: Option<i64>,
        term_scoped: bool,
    ) -> Uuid {
        let course = project
            .db()
            .lessons()
            .create_course(
                self.id,
                &CourseInput {
                    subject_id,
                    year_level_id: None,
                    term_id: if term_scoped { self.term } else { None },
                    name: format!("{name} course"),
                    code: None,
                    description: None,
                    periods_per_cycle,
                    default_duration_periods: 1,
                    required_room_type_id: required_room_type,
                },
            )
            .expect("create course")
            .id;
        let section = project
            .db()
            .lessons()
            .create_class_section(
                self.id,
                &ClassSectionInput {
                    course_id: course,
                    name: name.to_owned(),
                    code: None,
                    campus_id: campus.map(|at| self.campuses[at]),
                },
            )
            .expect("create class section")
            .id;
        self.sections.push(section);
        section
    }

    /// Gives a section a teacher.
    pub(super) fn teach(&mut self, project: &mut Project, section: Uuid, teacher: Uuid) {
        project
            .db()
            .lessons()
            .assign_class_teacher(self.id, section, teacher)
            .expect("assign class teacher");
    }

    /// Gives a section an attending group.
    pub(super) fn attend(&mut self, project: &mut Project, section: Uuid, group: Uuid) {
        project
            .db()
            .lessons()
            .assign_class_group(self.id, section, group)
            .expect("assign class group");
    }

    /// Adds `count` occurrences of `duration` periods each.
    ///
    /// One row per occurrence — ADR 0029. Four single periods a cycle is `(1, 4)`, which is
    /// four activities; a double is `(2, 1)`, which is one activity of two consecutive
    /// periods and **not** two activities.
    pub(super) fn occurrences(
        &mut self,
        project: &mut Project,
        section: Uuid,
        duration: i64,
        count: i64,
    ) {
        project
            .db()
            .lessons()
            .add_activities(self.id, section, None, duration, count)
            .expect("add activities");
    }

    /// Narrows a teacher to one campus.
    ///
    /// The call that makes `teacher-not-eligible-for-campus` reachable: it does not consult
    /// `class_teacher`, and `V15`'s eligibility trigger pair fires only on `class_teacher`,
    /// so narrowing somebody who already teaches elsewhere is permitted. See
    /// [`super::super::findings`] for the whole route.
    pub(super) fn narrow_to_campus(&mut self, project: &mut Project, teacher: Uuid, campus: usize) {
        project
            .db()
            .membership()
            .assign_teacher_campus(self.id, teacher, self.campuses[campus])
            .expect("assign teacher campus");
    }

    /// Records an availability rule for one entity on one cycle day.
    pub(super) fn unavailable_on_day(
        &mut self,
        project: &mut Project,
        kind: AvailabilityEntity,
        entity: Uuid,
        day: Uuid,
        status: AvailabilityStatus,
    ) -> Uuid {
        project
            .db()
            .availability()
            .set(&AvailabilityRuleInput {
                school_id: self.id,
                entity_kind: kind,
                entity_id: entity,
                timeslot_id: None,
                cycle_day_id: Some(day),
                period_id: None,
                term_id: None,
                status,
                weight: None,
                reason: Some("fixture".to_owned()),
                effective_from: None,
                effective_to: None,
            })
            .expect("set availability")
            .id
    }

    /// Records an availability rule for one entity in one period, every day.
    pub(super) fn unavailable_in_period(
        &mut self,
        project: &mut Project,
        kind: AvailabilityEntity,
        entity: Uuid,
        period: Uuid,
        status: AvailabilityStatus,
    ) -> Uuid {
        project
            .db()
            .availability()
            .set(&AvailabilityRuleInput {
                school_id: self.id,
                entity_kind: kind,
                entity_id: entity,
                timeslot_id: None,
                cycle_day_id: None,
                period_id: Some(period),
                term_id: None,
                status,
                weight: None,
                reason: Some("fixture".to_owned()),
                effective_from: None,
                effective_to: None,
            })
            .expect("set availability")
            .id
    }

    /// The cycle days of this school's cycle, in ordinal order.
    pub(super) fn days(&self, project: &mut Project) -> Vec<Uuid> {
        project
            .db()
            .time_model()
            .cycle_days(self.cycle)
            .expect("read cycle days")
            .into_iter()
            .map(|day| day.id)
            .collect()
    }

    /// The periods of one bell schedule, in ordinal order.
    pub(super) fn periods(&self, project: &mut Project, schedule: usize) -> Vec<Uuid> {
        project
            .db()
            .time_model()
            .periods(self.schedules[schedule])
            .expect("read periods")
            .into_iter()
            .map(|period| period.id)
            .collect()
    }

    /// Writes an `availability` row whose entity does not exist.
    ///
    /// # Why this one fixture writes SQL
    ///
    /// Everything else here goes through the repositories, deliberately. This row cannot:
    /// [`Availability::set`](crate::Availability::set) checks the entity exists before
    /// writing, and every repository that deletes an entity calls
    /// `availability::clear_for_entity` in the same transaction. Those two are exactly the
    /// guards that make an orphan unreachable through the supported API.
    ///
    /// But `availability.entity_id` is polymorphic and carries no foreign key — `V11`'s
    /// header calls it *"the one place in this schema where a reference cannot be enforced by
    /// the database"* — so an orphan is reachable through a *defect* in either guard, and
    /// [`Availability::orphans`](crate::Availability::orphans) exists because of that. ADR
    /// 0037's future acceptance test 3 posits precisely this project. Constructing it the
    /// only way it can arise is therefore the fixture, not a shortcut around one.
    ///
    /// The row is written through [`Database::write`], so it is a real transaction and the
    /// change-log trigger fires on it like any other insert.
    pub(super) fn orphan_availability_rule(
        &mut self,
        project: &mut Project,
        kind: &str,
        day: Uuid,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let missing = Uuid::now_v7();
        let school = self.id;
        project
            .db()
            .write(|tx| {
                tx.execute(
                    "INSERT INTO availability
                         (id, school_id, entity_kind, entity_id, timeslot_id, cycle_day_id,
                          period_id, term_id, status, weight, reason, effective_from,
                          effective_to, created_at, updated_at, rev)
                     VALUES (?, ?, ?, ?, NULL, ?, NULL, NULL, 'UNAVAILABLE', NULL,
                             'fixture', NULL, NULL, ?, ?, 1)",
                    rusqlite::params![
                        id.to_string(),
                        school.to_string(),
                        kind,
                        missing.to_string(),
                        day.to_string(),
                        "2026-01-01T00:00:00.000Z",
                        "2026-01-01T00:00:00.000Z",
                    ],
                )?;
                Ok(())
            })
            .expect("write orphan rule");
        id
    }

    /// Records an availability rule for an entity kind nothing schedules.
    ///
    /// A `RESOURCE` rule, written through the repository: a resource is a real entity and
    /// `Availability::set` accepts one, so this needs no bypass. What the engine does not do
    /// is schedule around it, and saying so is `availability-entity-kind-not-scheduled`'s
    /// whole job.
    pub(super) fn resource_rule(&mut self, project: &mut Project, day: Uuid) -> Uuid {
        let resource = project
            .db()
            .structure()
            .create_resource(&crate::ResourceInput {
                school_id: self.id,
                campus_id: None,
                name: "Projector trolley".to_owned(),
                code: "PROJ".to_owned(),
                quantity: 2,
            })
            .expect("create resource")
            .id;
        self.unavailable_on_day(
            project,
            AvailabilityEntity::Resource,
            resource,
            day,
            AvailabilityStatus::Unavailable,
        )
    }
}

/// Four teaching periods and a break, the shape most fixtures want.
///
/// The break sits between periods 2 and 3, so the longest unbroken run of teaching periods
/// is **two** rather than four. That matters: a run is broken by any non-teaching period
/// between two teaching ones, and a fixture whose periods were all teaching could not
/// demonstrate the difference.
pub(super) const STANDARD_DAY: &[(&str, &str, &str, bool)] = &[
    ("1", "09:00", "10:00", true),
    ("2", "10:00", "11:00", true),
    ("Break", "11:00", "11:30", false),
    ("3", "11:30", "12:30", true),
    ("4", "13:30", "14:30", true),
];

/// Six teaching periods with no break, for the fixtures that want a long run.
pub(super) const LONG_DAY: &[(&str, &str, &str, bool)] = &[
    ("1", "09:00", "10:00", true),
    ("2", "10:00", "11:00", true),
    ("3", "11:00", "12:00", true),
    ("4", "13:00", "14:00", true),
    ("5", "14:00", "15:00", true),
    ("6", "15:00", "16:00", true),
];
