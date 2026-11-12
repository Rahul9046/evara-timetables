//! One project at the realistic upper end, with the edit history the §7 figures need.
//!
//! # Why this fixture exists, and why it could not exist in Stage 1
//!
//! Two of the proposal's measurements were deferred from Stage 1 to Stage 2 for want of this
//! project, and the reason is worth keeping: **T-60** reports the change feed's share of a
//! payload and **T-83** the lock duration of a stepped backup, and both are figures about
//! *size and use*. Stage 1's projects were a handful of rows created inside a test, so
//! measuring either against one would have produced a number with no relationship to the
//! budget it was supposed to test — which is worse than an honest gap, because a figure gets
//! quoted and a gap does not.
//!
//! So this builds a project at [§7.3](../../../../docs/M10-PROJECT-TRANSFER-PROPOSAL.md)'s
//! *realistic upper end*: two schools, three campuses, both cycle kinds, ~125 sections and
//! ~500 occurrences, a four-figure group-level population with real `student` rows beside
//! it, a shared room, an embedded logo, and a feed that grew by **being used** rather than
//! by being inserted into.
//!
//! # Edit history is made by editing, not by writing more rows
//!
//! The distinction is the whole point of T-60. A project with 10 000 rows inserted once has
//! 10 000 change-log entries; a project whose rooms were renamed, whose teachers changed
//! campus, and whose sections were re-timed has more entries than it has rows. The second is
//! what a school looks like after a year, so [`Shape::Realistic`] does a pass of updates over
//! what it created and the measurement reports both numbers.
//!
//! # What this fixture deliberately does not build
//!
//! **No timetable.** A stored timetable needs a `SolveOutcome`, and the only honest source of
//! one is the engine that produces them — which `evara-db` may not depend on as a library
//! (`ARCHITECTURE.md` §3 rule 4). Hand-building one here would be a fixture that could drift
//! from what the solver actually returns, which is the drift every fixture in this crate is
//! written to avoid. So the timetable layer is added by the crate whose *tests* may run the
//! solver — the same argument this crate's own `Cargo.toml` already records for its
//! `evara-solver` dev-dependency.
//!
//! Every name is invented, per `CLAUDE.md` rule 3. No real school, teacher or student.

use uuid::Uuid;

use super::seed::{LONG_DAY, Project, STANDARD_DAY, School};
use crate::repo::availability::{AvailabilityEntity, AvailabilityStatus};
use crate::repo::constraints::ConstraintInstanceInput;
use crate::repo::curriculum::DepartmentInput;
use crate::repo::structure::BuildingInput;
use crate::repo::time_model::CycleKind;
use crate::{CycleInput, Expected, RoomInput};

/// Which schools the fixture built, and the identifiers a test addresses them by.
///
/// [`super::seed::School`] is a builder rather than a record and is neither `Clone` nor
/// `Debug`, so this carries the identifiers rather than the builders: a test asserting a
/// round trip wants the school's `id`, its cycle and its term, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// The school.
    pub id: Uuid,
    /// Its campuses, in creation order.
    pub campuses: Vec<Uuid>,
    /// The cycle its grid is defined over.
    pub cycle: Uuid,
    /// Its term, where the fixture made one.
    pub term: Option<Uuid>,
    /// Its class sections, in creation order.
    pub sections: Vec<Uuid>,
    /// Its teachers, in creation order.
    pub teachers: Vec<Uuid>,
    /// Its rooms, in creation order.
    pub rooms: Vec<Uuid>,
}

impl Seeded {
    /// What to pass the loader.
    #[must_use]
    pub const fn selection(&self) -> crate::scheduling::SchedulingSelection {
        crate::scheduling::SchedulingSelection {
            school_id: self.id,
            cycle_id: self.cycle,
            term_id: self.term,
        }
    }

    /// Takes the identifiers off a builder.
    fn of(school: &School) -> Self {
        Self {
            id: school.id,
            campuses: school.campuses.clone(),
            cycle: school.cycle,
            term: school.term,
            sections: school.sections.clone(),
            teachers: school.teachers.clone(),
            rooms: school.rooms.clone(),
        }
    }
}

/// How big to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Small enough for a test that only needs the *variety*: every table populated, both
    /// cycle kinds, a shared room, a logo, a few of everything.
    ///
    /// What the round-trip preservation tests use. They assert that every row survives, and
    /// a thousand identical sections prove that no better than four do while costing every
    /// run the time to build them.
    Varied,
    /// §7.3's realistic upper end: ~125 sections, ~500 occurrences, ~30 teaching slots.
    ///
    /// What T-60, T-83 and the snapshot-inflation figure are measured against. Slower to
    /// build, so only the measurements use it.
    Realistic,
}

impl Shape {
    /// How many class sections to create.
    const fn sections(self) -> usize {
        match self {
            Self::Varied => 8,
            Self::Realistic => 125,
        }
    }

    /// How many teachers.
    const fn teachers(self) -> usize {
        match self {
            Self::Varied => 6,
            Self::Realistic => 80,
        }
    }

    /// How many student groups, each of forty.
    const fn groups(self) -> usize {
        match self {
            Self::Varied => 4,
            Self::Realistic => 25,
        }
    }

    /// How many named students to enrol, beside the group-level population.
    ///
    /// Few, even at the realistic shape. ADR 0006 requires a full timetable to be buildable
    /// from `student_group.size` with no `student` rows at all, so an individual roster is
    /// the *optional* half of the model and a fixture that made it the bulk would be
    /// measuring a shape the product does not require.
    const fn students(self) -> usize {
        match self {
            Self::Varied => 6,
            Self::Realistic => 60,
        }
    }

    /// How many rooms.
    const fn rooms(self) -> usize {
        match self {
            Self::Varied => 6,
            Self::Realistic => 20,
        }
    }
}

/// What was built, so a test can assert against it without re-deriving it.
#[derive(Debug, Clone)]
pub struct Built {
    /// The school the lessons hang off, on a `CUSTOM` five-position cycle.
    pub main: Seeded,
    /// A second school in the same project, on a `WEEKLY` cycle.
    ///
    /// Two schools because `V12` removed "the school" — every school-owned read says which —
    /// and a round trip over a single-school project would not carry an identity shared
    /// across two, which is ADR 0019's deliberate exception and T-08's subject.
    pub second: Seeded,
    /// The second school's weekly cycle, which has no materialised grid.
    ///
    /// Carried so a test can assert that both `cycle-kind:` capabilities travel. A cycle
    /// with no grid is an ordinary state — a school mid-setup — and must transfer.
    pub weekly_cycle: Uuid,
    /// The building both schools' campuses reach.
    pub shared_building: Uuid,
    /// The room both schools' campuses reach.
    pub shared_room: Uuid,
    /// The students enrolled individually, in creation order.
    pub students: Vec<Uuid>,
    /// How many updates the history pass applied, for the T-60 report.
    pub updates_applied: usize,
}

/// Builds the project, returning the open workspace and what it holds.
///
/// The project is **left open**. A caller that wants to export a closed file calls
/// [`Project::close`]; one that wants to snapshot a live project under the workspace lock
/// wants it exactly as it is.
#[must_use]
pub fn build(shape: Shape) -> (Project, Built) {
    build_named(shape, "northgate")
}

/// Builds the project with a chosen file stem.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one fixture read top to bottom beats six helpers that each need their \
              intermediate identifiers threaded through; the loader fixtures next door \
              are written the same way"
)]
pub fn build_named(shape: Shape, stem: &str) -> (Project, Built) {
    let mut project = Project::named(stem);

    // ---- The main school: two campuses, one term, two bell schedules, a CUSTOM cycle ----
    let mut main = School::new(
        &mut project,
        "Northgate Secondary (synthetic)",
        &["Northgate Main", "Northgate Annexe"],
        5,
    );
    main.with_term(&mut project)
        .bells(&mut project, "Main Day", Some(0), LONG_DAY)
        .bells(&mut project, "Annexe Day", Some(1), STANDARD_DAY)
        .materialise(&mut project);

    // ---- The second school: one campus, a WEEKLY cycle, its own grid ----
    let mut second = School::new(
        &mut project,
        "Southmoor College (synthetic)",
        &["Southmoor"],
        5,
    );
    // A `WEEKLY` cycle beside the `CUSTOM` one, so the archive declares both
    // `cycle-kind:` capabilities and an importing build has to understand both. A weekly
    // cycle is *the week*: seven positions, and the repository derives the length from the
    // kind rather than trusting a caller, so no `day_count` is passed.
    let weekly = project
        .db()
        .time_model()
        .create_cycle(&CycleInput {
            school_id: second.id,
            name: String::from("Weekly"),
            kind: CycleKind::Weekly,
            day_count: 7,
            week_count: 1,
            is_default: false,
        })
        .expect("create a weekly cycle")
        .id;
    second
        .with_term(&mut project)
        .bells(&mut project, "Southmoor Day", None, STANDARD_DAY)
        .materialise(&mut project);

    // ---- A building and a room both schools reach ----
    //
    // Facilities are ADR 0019's deliberate exception: `building_campus` and `room_campus`
    // reach across schools, so a shared building is destroyable only by its home school and
    // only while nobody else shares it. A transfer that lost one of those rows would
    // silently give one school sole ownership of the other's room.
    let shared_building = project
        .db()
        .structure()
        .create_building(
            &BuildingInput {
                campus_id: main.campuses[0],
                name: String::from("Science Block"),
            },
            &[main.campuses[1], second.campuses[0]],
        )
        .expect("create a shared building")
        .id;

    let science = main.room_type(&mut project, "Laboratory");
    let shared_room = project
        .db()
        .structure()
        .create_room(
            &RoomInput {
                campus_id: main.campuses[0],
                building_id: Some(shared_building),
                room_type_id: Some(science),
                name: String::from("Lab 1"),
                code: String::from("SHARED-1"),
                capacity: 32,
                is_bookable: true,
                notes: None,
            },
            &[main.campuses[1], second.campuses[0]],
        )
        .expect("create a shared room")
        .id;
    main.rooms.push(shared_room);

    // ---- Facilities, people and curriculum at the chosen size ----
    for n in 0..shape.rooms() {
        main.room(&mut project, &format!("Room {n:02}"), 0, None, 30, &[]);
    }
    // One room nothing can book, because `is_bookable` is a property of the *room* and not
    // of how many people fit, and a transfer that lost the flag would make a cupboard
    // schedulable.
    main.unbookable_room(&mut project, "Store", 1, None);

    for n in 0..shape.teachers() {
        main.teacher(&mut project, &format!("Teacher{n:02}"));
    }
    for n in 0..shape.groups() {
        main.group(&mut project, &format!("Group {n:02}"), Some(40), None);
    }
    let mut students = Vec::new();
    for n in 0..shape.students() {
        let group = main.groups[n % main.groups.len()];
        let student = main.student_in(&mut project, group, n);

        // `create_student` already opens the spell at the school, so this **updates** it
        // rather than adding a second — a student with two current spells at one school is
        // refused, and rightly. Two different facts are involved and both must travel: group
        // membership is the scheduling audience, `student_school_enrolment` is the spell at
        // the institution, and a transfer that lost the second would leave a child in a class
        // at a school they are not recorded as attending.
        //
        // Filling in the admission number and the campus is also history: each update
        // consumes a revision and appends a change-log row, which is what T-60 measures.
        let enrolment = project
            .db()
            .membership()
            .current_enrolment_at(main.id, student)
            .expect("read the spell `create_student` opened")
            .expect("a new student has a current spell");
        project
            .db()
            .membership()
            .update_enrolment(
                main.id,
                enrolment.id,
                Expected::Rev(enrolment.rev),
                &crate::repo::membership::StudentEnrolmentInput {
                    school_id: main.id,
                    student_id: student,
                    admission_no: Some(format!("ADM{n:04}")),
                    year_level_id: None,
                    campus_id: Some(main.campuses[n % main.campuses.len()]),
                    status: crate::repo::membership::EnrolmentStatus::Enrolled,
                    enrolled_on: Some(String::from("2026-09-01")),
                    left_on: None,
                },
            )
            .expect("record the admission number and campus");
        students.push(student);
    }

    let department = project
        .db()
        .curriculum()
        .create_department(&DepartmentInput {
            school_id: main.id,
            name: String::from("Mathematics"),
            code: Some(String::from("MA")),
            description: None,
        })
        .expect("create a department")
        .id;

    let subjects: Vec<Uuid> = (0..6)
        .map(|n| main.subject(&mut project, &format!("Subject {n}")))
        .collect();

    // A qualification is a property of the person and survives a withdrawn membership; a
    // department association is a narrowing of membership and is cleared when it goes. The
    // two look alike and are governed oppositely (ADRs 0025 and 0026), so both travel and
    // T-09 asserts the difference still holds after a transfer.
    for (n, teacher) in main.teachers.iter().take(4).enumerate() {
        project
            .db()
            .curriculum()
            .qualify_teacher(main.id, *teacher, subjects[n % subjects.len()])
            .expect("qualify a teacher");
        project
            .db()
            .curriculum()
            .assign_teacher_department(main.id, *teacher, department)
            .expect("assign a department");
    }

    for n in 0..shape.sections() {
        let section = main.section(
            &mut project,
            subjects[n % subjects.len()],
            &format!("Section {n:03}"),
            // Scoped to the main campus, and that is not incidental. The school has **two**
            // campus-scoped bell schedules, so a section naming no campus resolves to both —
            // which the loader reports as `NoBellSchedule`, blocking, because two compatible
            // schedules are never chosen between. A fixture that left it unscoped would be
            // unsolvable, and the timetable layer a round trip needs would not exist.
            Some(0),
            None,
            Some(4),
            false,
        );
        let teacher = main.teachers[n % main.teachers.len()];
        let group = main.groups[n % main.groups.len()];
        main.teach(&mut project, section, teacher);
        main.attend(&mut project, section, group);
        // One row per occurrence, with a `split_index` that is assigned once and never
        // renumbered — ADR 0029, and T-11's subject.
        main.occurrences(&mut project, section, 1, 4);
    }

    // A section taught to a group that *also* names individual children. The audience is the
    // group's 40, because those children are among the 40 — `class_enrolment` has no
    // scheduling meaning in Milestone 1 and is optional everywhere, which is exactly why it
    // has to survive a transfer unchanged rather than be rebuilt from anything.
    let roster_section = main.sections[0];
    for student in students.iter().take(4) {
        main.enrol(&mut project, roster_section, *student);
    }

    // A double period, so a `duration` other than one travels.
    main.occurrences(&mut project, main.sections[1], 2, 1);

    // ---- The second school gets lessons of its own, on its own grid ----
    let second_subject = second.subject(&mut project, "Southmoor Subject");
    second.teacher(&mut project, "Southmoor1");
    second.group(&mut project, "Southmoor Group", Some(28), None);
    let second_section = second.section(
        &mut project,
        second_subject,
        "Southmoor Section",
        // Unscoped is right here: Southmoor has one school-wide schedule, so there is nothing
        // to choose between. One axis per bell schedule, never merged — two campuses' Period 3
        // is not one row — and this project holds three such axes across two schools.
        None,
        None,
        Some(2),
        false,
    );
    second.teach(&mut project, second_section, second.teachers[0]);
    second.attend(&mut project, second_section, second.groups[0]);
    second.occurrences(&mut project, second_section, 1, 2);

    // A teacher employed by both schools: one identity, two memberships. V13's shared
    // identity, and the thing a per-school export could not split without deciding something
    // nobody has decided (A7).
    let shared_teacher = main.teachers[0];
    project
        .db()
        .membership()
        .add_teacher_to_school(&crate::repo::membership::TeacherSchoolInput {
            school_id: second.id,
            teacher_id: shared_teacher,
            is_active: true,
        })
        .expect("employ one teacher at both schools");

    // ---- Availability, constraints, a logo, a calendar day ----
    let days = main.days(&mut project);
    main.unavailable_on_day(
        &mut project,
        AvailabilityEntity::Teacher,
        main.teachers[1],
        days[0],
        HARD_REFUSAL,
    );
    let periods = main.periods(&mut project, 0);
    main.unavailable_in_period(
        &mut project,
        AvailabilityEntity::Room,
        main.rooms[0],
        periods[0],
        HARD_REFUSAL,
    );

    for constraint_type in [
        evara_core::scheduling::ConstraintType::RoomClash,
        evara_core::scheduling::ConstraintType::ActivityScheduled,
    ] {
        project
            .db()
            .constraints()
            .create(&ConstraintInstanceInput {
                school_id: main.id,
                type_key: constraint_type.key().to_owned(),
                level: constraint_type.default_level().stored().to_owned(),
                weight: 7,
                enabled: true,
                params: String::from("{}"),
                term_id: None,
            })
            .expect("tune a constraint");
    }

    // A logo, so `school_logo.bytes` travels as a BLOB. A one-pixel PNG: the smallest thing
    // that is genuinely a PNG, because `set_school_logo` reads the header for the dimensions
    // and a placeholder of arbitrary bytes would be refused.
    project
        .db()
        .structure()
        .set_school_logo(main.id, &logo_png(64, 64, 4096))
        .expect("store a logo");

    let updates_applied = apply_history(&mut project, &main, shape);

    let built = Built {
        main: Seeded::of(&main),
        second: Seeded::of(&second),
        weekly_cycle: weekly,
        shared_building,
        shared_room,
        students,
        updates_applied,
    };
    (project, built)
}

/// Edits what was just created, so the change feed grows by *use*.
///
/// Returns how many updates were applied. Every one goes through `repo::update`, so each
/// consumes a revision and appends a change-log row — which is the difference T-60 measures
/// between a project that was filled in once and one that has been run for a year.
fn apply_history(project: &mut Project, main: &School, shape: Shape) -> usize {
    let mut applied = 0;

    let passes = match shape {
        Shape::Varied => 1,
        Shape::Realistic => 4,
    };

    for pass in 0..passes {
        // Rooms get renamed and re-capacitied, which is the commonest real edit and the one
        // that touches a table every grid reads.
        for (n, room) in main.rooms.iter().enumerate() {
            let current = project
                .db()
                .structure()
                .room(*room)
                .expect("read a room")
                .expect("the room exists");
            // The revision is the predicate, never a comparison in Rust first — ADR 0027.
            project
                .db()
                .structure()
                .update_room(
                    *room,
                    Expected::Rev(current.rev),
                    &RoomInput {
                        campus_id: current.campus_id,
                        building_id: current.building_id,
                        room_type_id: current.room_type_id,
                        name: format!("{} (rev {})", current.name.trim(), pass + 1),
                        code: current.code.clone(),
                        capacity: current.capacity + i64::try_from(n % 3).unwrap_or(0),
                        is_bookable: current.is_bookable,
                        notes: Some(format!("reviewed in pass {}", pass + 1)),
                    },
                )
                .expect("rename a room");
            applied += 1;
        }
    }

    applied
}

/// A PNG header of `width` by `height`, padded to `bytes` with a recognisable filler.
///
/// `set_school_logo` reads the IHDR for the dimensions and refuses anything under 16 by 16, so
/// a placeholder of arbitrary bytes would be refused — which is the behaviour being relied on
/// rather than worked around. The pixel data is filler: nothing in Evara decodes a logo, and a
/// fixture that embedded a real image would be carrying bytes nobody can inspect.
///
/// Deterministic, so two runs of a round trip compare identical BLOBs.
#[must_use]
pub fn logo_png(width: u32, height: u32, bytes: usize) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    // 8-bit, RGBA, no interlace.
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.resize(bytes.max(png.len()), 0x5A);
    png
}

/// A hard availability refusal reaches the model only through a stored rule, so the fixture
/// says which status it used rather than leaving a reader to infer it.
pub const HARD_REFUSAL: AvailabilityStatus = AvailabilityStatus::Unavailable;
