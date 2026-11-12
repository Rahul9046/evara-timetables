//! The twelve synthetic schools M4's loader is tested against, as a catalogue.
//!
//! # Why these are not in the test module any more
//!
//! They were, until M10 Stage 2. Each one is a *school*, built through the ordinary
//! repository API into a real `.evaraproj` — which makes them exactly what a transfer round
//! trip needs, and they were unreachable outside this crate's own test binary. Moving them
//! here changes none of them: the loader tests import them from here and assert the same
//! things, so a drift between "the fixture the loader is tested against" and "the fixture a
//! transfer is tested against" is not representable.
//!
//! # What each one is for
//!
//! Twelve, and no two are a variation on one theme. Four are sound schools at different
//! shapes ([`tiny`], [`medium`], [`multiple_campuses`], [`shared_teacher`]); five are a
//! named refusal or warning the loader must produce ([`infeasible_duration`],
//! [`infeasible_room_type`], [`ambiguous_bells`], [`unmaterialised_bells`],
//! [`missing_grid`]); three are a narrowing that changes what is legal
//! ([`unknown_overlap`], [`availability_restrictions`], [`ineligible_teacher`]).
//!
//! A transfer that preserves all twelve has carried a sound school, an unsound one, a
//! multi-school project, a shared identity, an orphaned availability row and an empty grid —
//! which is a far better round-trip corpus than one hand-built project, and T-07 is the test
//! that spends it.
//!
//! Every name is invented, per `CLAUDE.md` rule 3. No real school, teacher or student.

use uuid::Uuid;

use super::seed::{LONG_DAY, Project, STANDARD_DAY, School};
use crate::repo::availability::{AvailabilityEntity, AvailabilityStatus};

/// A small, correctly configured school: one campus, one school-wide bell schedule.
///
/// Five positions, five periods of which four teach, so the grid holds 25 slots and 20
/// teaching ones. Two sections, each taught by one teacher to exactly one sized group — the
/// configuration that lets `expected_attendance` be `Known` at `V15`, since a one-element
/// audience is vacuously pairwise disjoint.
pub fn tiny() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Northgate Secondary", &["Northgate Main"], 5);
    school
        .with_term(&mut project)
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);

    let general = school.room_type(&mut project, "General Teaching");
    let lab = school.room_type(&mut project, "Science Laboratory");
    school.room(&mut project, "Room 1", 0, Some(general), 30, &[]);
    school.room(&mut project, "Room 2", 0, Some(general), 30, &[]);
    school.room(&mut project, "Lab 1", 0, Some(lab), 24, &[]);

    let maths_teacher = school.teacher(&mut project, "Abara");
    let science_teacher = school.teacher(&mut project, "Benson");
    let nine_a = school.group(&mut project, "9A", Some(28), None);
    let nine_b = school.group(&mut project, "9B", Some(26), None);

    let maths = school.subject(&mut project, "Mathematics");
    let science = school.subject(&mut project, "Science");
    let maths_section = school.section(
        &mut project,
        maths,
        "9A Mathematics",
        None,
        None,
        Some(4),
        false,
    );
    school.teach(&mut project, maths_section, maths_teacher);
    school.attend(&mut project, maths_section, nine_a);
    school.occurrences(&mut project, maths_section, 1, 4);

    let science_section = school.section(
        &mut project,
        science,
        "9B Science",
        None,
        Some(lab),
        Some(4),
        false,
    );
    school.teach(&mut project, science_section, science_teacher);
    school.attend(&mut project, science_section, nine_b);
    school.occurrences(&mut project, science_section, 1, 4);

    project.reopen();
    (project, school)
}

/// A realistic school at the size M2's `medium` fixture describes.
///
/// 80 teachers, 25 groups of 40 — a group-level population of 1 000 with no `student` rows
/// at all, which is the ADR 0006 school — 125 sections and **500** occurrences over a grid
/// of 30 teaching slots. Comfortable rather than tight: until a solver exists nobody can
/// tell a hard fixture from an impossible one, which is `OPEN-DECISIONS.md` R9's point.
pub fn medium() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Southmoor College", &["Southmoor Main"], 5);
    school
        .with_term(&mut project)
        .bells(&mut project, "Standard Day", None, LONG_DAY)
        .materialise(&mut project);

    for n in 0..20 {
        school.room(&mut project, &format!("Room {n}"), 0, None, 30, &[]);
    }
    for n in 0..80 {
        school.teacher(&mut project, &format!("Staff{n:02}"));
    }
    for n in 0..25 {
        school.group(&mut project, &format!("Group {n:02}"), Some(40), None);
    }

    let subjects: Vec<Uuid> = (0..10)
        .map(|n| school.subject(&mut project, &format!("Subject {n}")))
        .collect();
    for n in 0..125_usize {
        let section = school.section(
            &mut project,
            subjects[n % subjects.len()],
            &format!("Section {n:03}"),
            None,
            None,
            Some(4),
            false,
        );
        let teacher = school.teachers[n % school.teachers.len()];
        let group = school.groups[n % school.groups.len()];
        school.teach(&mut project, section, teacher);
        school.attend(&mut project, section, group);
        school.occurrences(&mut project, section, 1, 4);
    }

    project.reopen();
    (project, school)
}

/// An occurrence longer than any unbroken run of teaching periods in its schedule.
///
/// `STANDARD_DAY` puts a break between periods 2 and 3, so the longest run is two. A
/// four-period occurrence cannot be placed by any solver, and that is arithmetic rather than
/// a difficulty — ADR 0033 §5.
pub fn infeasible_duration() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Eastvale Academy", &["Eastvale"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Hall", 0, None, 60, &[]);
    let teacher = school.teacher(&mut project, "Carrow");
    let group = school.group(&mut project, "10A", Some(30), None);
    let games = school.subject(&mut project, "Games");
    let section = school.section(&mut project, games, "10A Games", None, None, Some(4), false);
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 4, 1);

    project.reopen();
    (project, school)
}

/// A section requiring a kind of room the school has none of.
///
/// Conclusively impossible, and reported once per section rather than once per occurrence:
/// the requirement lives on the section's course and every occurrence fails identically.
pub fn infeasible_room_type() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Westcombe High", &["Westcombe"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    let general = school.room_type(&mut project, "General Teaching");
    let pool = school.room_type(&mut project, "Swimming Pool");
    school.room(&mut project, "Room 1", 0, Some(general), 30, &[]);

    let teacher = school.teacher(&mut project, "Danforth");
    let group = school.group(&mut project, "8A", Some(29), None);
    let swimming = school.subject(&mut project, "Swimming");
    let section = school.section(
        &mut project,
        swimming,
        "8A Swimming",
        None,
        Some(pool),
        Some(2),
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    project.reopen();
    (project, school)
}

/// Two campuses, a campus-scoped bell schedule each, and a shared hall.
///
/// Each section names its campus, so ADR 0033 §2 clause 2 resolves it. The hall is **one**
/// room reachable from both campuses — ADR 0014 — so it must load as one `RoomFact` with two
/// campus indices and not as two rooms.
pub fn multiple_campuses() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(
        &mut project,
        "Fairmead Trust School",
        &["Fairmead Lower", "Fairmead Upper"],
        5,
    );
    school
        .with_term(&mut project)
        .bells(&mut project, "Lower Day", Some(0), STANDARD_DAY)
        .bells(&mut project, "Upper Day", Some(1), LONG_DAY)
        .materialise(&mut project);

    school.room(&mut project, "Lower 1", 0, None, 30, &[]);
    school.room(&mut project, "Upper 1", 1, None, 30, &[]);
    school.room(&mut project, "Shared Hall", 0, None, 120, &[1]);

    let lower_teacher = school.teacher(&mut project, "Everly");
    let upper_teacher = school.teacher(&mut project, "Fenwick");
    let lower_group = school.group(&mut project, "7A", Some(27), None);
    let upper_group = school.group(&mut project, "12A", Some(18), None);

    let english = school.subject(&mut project, "English");
    let lower = school.section(
        &mut project,
        english,
        "7A English",
        Some(0),
        None,
        Some(3),
        false,
    );
    school.teach(&mut project, lower, lower_teacher);
    school.attend(&mut project, lower, lower_group);
    school.occurrences(&mut project, lower, 1, 3);

    let upper = school.section(
        &mut project,
        english,
        "12A English",
        Some(1),
        None,
        Some(3),
        false,
    );
    school.teach(&mut project, upper, upper_teacher);
    school.attend(&mut project, upper, upper_group);
    school.occurrences(&mut project, upper, 1, 3);

    project.reopen();
    (project, school)
}

/// Two school-wide bell schedules, both materialised.
///
/// Both are compatible with every section under clause 1, so no section resolves to one and
/// Evara refuses rather than choosing — ADR 0033's whole decision is that there is no
/// fallback and no default.
pub fn ambiguous_bells() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Harbourview School", &["Harbourview"], 5);
    school
        .bells(&mut project, "Early Day", None, STANDARD_DAY)
        .bells(&mut project, "Late Day", None, LONG_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);
    let teacher = school.teacher(&mut project, "Garrick");
    let group = school.group(&mut project, "9C", Some(25), None);
    let history = school.subject(&mut project, "History");
    let section = school.section(
        &mut project,
        history,
        "9C History",
        None,
        None,
        Some(2),
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    project.reopen();
    (project, school)
}

/// One bell schedule defined and never materialised, beside one that is.
///
/// ADR 0033 test 8: the unmaterialised schedule is **not** in `G`, so it is not a candidate
/// and there is no ambiguity. This is the case M3 found M2's documentation had wrong, and the
/// loader must get it right for the same reason — `G` comes from the slots.
pub fn unmaterialised_bells() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Kelsmoor School", &["Kelsmoor"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    // Defined after materialising, and never materialised itself.
    school.bells(&mut project, "Sixth Form Day", None, LONG_DAY);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);
    let teacher = school.teacher(&mut project, "Hallam");
    let group = school.group(&mut project, "11A", Some(24), None);
    let geography = school.subject(&mut project, "Geography");
    let section = school.section(
        &mut project,
        geography,
        "11A Geography",
        None,
        None,
        Some(2),
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    project.reopen();
    (project, school)
}

/// Two schools in one project, sharing a teacher and a room.
///
/// ADR 0032 rule 3's exact condition on both counts: a second active `teacher_school` row,
/// and a `room_campus` set spanning two schools' campuses. The teacher is **one**
/// `teacher.id` in both, which is what makes the warning writable.
pub fn shared_teacher() -> (Project, School, School) {
    let mut project = Project::new();
    let mut north = School::new(&mut project, "Northgate Secondary", &["Northgate Main"], 5);
    let mut south = School::new(&mut project, "Southmoor College", &["Southmoor Main"], 5);
    north
        .bells(&mut project, "Northgate Day", None, STANDARD_DAY)
        .materialise(&mut project);
    south
        .bells(&mut project, "Southmoor Day", None, STANDARD_DAY)
        .materialise(&mut project);

    // Classified under Northgate, which is where it was created. Southmoor declares a type
    // of its own with the same name, so the two are same-named and different records — the
    // case §10 of the assembly document is about.
    let north_hall_type = north.room_type(&mut project, "Sports Hall");
    south.room_type(&mut project, "Sports Hall");
    let hall = north.room(
        &mut project,
        "Joint Sports Hall",
        0,
        Some(north_hall_type),
        200,
        &[],
    );
    project
        .db()
        .structure()
        .share_room(hall, south.campuses[0])
        .expect("share the hall with the other school");
    south.rooms.push(hall);

    // Created at Northgate, so that is the home school, and then given a second active
    // membership at Southmoor — the supported way to record somebody employed by both.
    let shared = north.teacher(&mut project, "Okafor");
    project
        .db()
        .membership()
        .add_teacher_to_school(&crate::TeacherSchoolInput {
            teacher_id: shared,
            school_id: south.id,
            is_active: true,
        })
        .expect("second membership");
    south.teachers.push(shared);

    let north_group = north.group(&mut project, "9A", Some(28), None);
    let north_maths = north.subject(&mut project, "Mathematics");
    let north_section = north.section(
        &mut project,
        north_maths,
        "9A Mathematics",
        None,
        None,
        Some(2),
        false,
    );
    north.teach(&mut project, north_section, shared);
    north.attend(&mut project, north_section, north_group);
    north.occurrences(&mut project, north_section, 1, 2);

    let south_group = south.group(&mut project, "12A", Some(15), None);
    let south_maths = south.subject(&mut project, "Further Mathematics");
    let south_section = south.section(
        &mut project,
        south_maths,
        "12A Further Mathematics",
        None,
        None,
        Some(2),
        false,
    );
    south.teach(&mut project, south_section, shared);
    south.attend(&mut project, south_section, south_group);
    south.occurrences(&mut project, south_section, 1, 2);

    project.reopen();
    (project, north, south)
}

/// A cohort with children whose overlap nothing settles.
///
/// Four sibling groups under one cohort. ADR 0035's worked example: `9-Mat-1` and `9-Fre-1`
/// are structurally identical to `9A` and `9B` in the stored data, and a tree of containment
/// cannot express two different divisions of one population — so none of the pairs is
/// decidable at `V15`, and `student_group.children_are_disjoint` is the future column that
/// would let M5 settle some of them.
pub fn unknown_overlap() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Marlowe Grammar", &["Marlowe"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);
    school.room(&mut project, "Room 2", 0, None, 30, &[]);

    let cohort = school.group(&mut project, "Year 9", Some(120), None);
    let classes = [
        school.group(&mut project, "9A", Some(30), Some(cohort)),
        school.group(&mut project, "9B", Some(30), Some(cohort)),
    ];
    let sets = [
        school.group(&mut project, "9-Mat-1", Some(30), Some(cohort)),
        school.group(&mut project, "9-Fre-1", Some(30), Some(cohort)),
    ];

    let maths = school.subject(&mut project, "Mathematics");
    for (n, group) in classes.iter().chain(sets.iter()).enumerate() {
        let teacher = school.teacher(&mut project, &format!("Ingram{n}"));
        let section = school.section(
            &mut project,
            maths,
            &format!("Mathematics set {n}"),
            None,
            None,
            Some(2),
            false,
        );
        school.teach(&mut project, section, teacher);
        school.attend(&mut project, section, *group);
        school.occurrences(&mut project, section, 1, 2);
    }

    project.reopen();
    (project, school)
}

/// A school with a cycle, a bell schedule and no materialised grid.
///
/// Reported **once, about the school** — never once per section. ADR 0033 test 9, and
/// `PRE-SOLVE-VALIDATION.md` §5's first deduplication rule.
pub fn missing_grid() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Pelham Fields School", &["Pelham"], 5);
    // Defined, never materialised.
    school.bells(&mut project, "Standard Day", None, STANDARD_DAY);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);
    let teacher = school.teacher(&mut project, "Jarrow");
    let group = school.group(&mut project, "7A", Some(28), None);
    let art = school.subject(&mut project, "Art");
    let section = school.section(&mut project, art, "7A Art", None, None, Some(2), false);
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    project.reopen();
    (project, school)
}

/// Availability of every shape the loader compiles, plus the two it cannot.
///
/// A teacher unavailable for a whole cycle day; a room unavailable in one period on every
/// day; a group discouraged from a day. Then a rule naming a deleted teacher — an orphan —
/// and a rule naming a resource, which nothing schedules.
pub fn availability_restrictions() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Rookwood School", &["Rookwood"], 5);
    school
        .with_term(&mut project)
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    let room = school.room(&mut project, "Room 1", 0, None, 30, &[]);
    school.room(&mut project, "Room 2", 0, None, 30, &[]);
    let teacher = school.teacher(&mut project, "Kemble");
    let group = school.group(&mut project, "8A", Some(26), None);
    let music = school.subject(&mut project, "Music");
    let section = school.section(&mut project, music, "8A Music", None, None, Some(2), false);
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    let days = school.days(&mut project);
    let periods = school.periods(&mut project, 0);

    school.unavailable_on_day(
        &mut project,
        AvailabilityEntity::Teacher,
        teacher,
        days[0],
        AvailabilityStatus::Unavailable,
    );
    school.unavailable_in_period(
        &mut project,
        AvailabilityEntity::Room,
        room,
        periods[0],
        AvailabilityStatus::Unavailable,
    );
    school.unavailable_on_day(
        &mut project,
        AvailabilityEntity::StudentGroup,
        group,
        days[4],
        AvailabilityStatus::Discouraged,
    );
    school.orphan_availability_rule(&mut project, "TEACHER", days[1]);
    school.resource_rule(&mut project, days[2]);

    project.reopen();
    (project, school)
}

/// A teacher narrowed to a campus they do not teach at.
///
/// Built with **no bypassed trigger**: the section is given to an unnarrowed teacher, which
/// `V15`'s eligibility trigger permits because no `teacher_campus` row means "not narrowed",
/// and the teacher is *then* narrowed to the other campus. Neither
/// `Membership::assign_teacher_campus` nor the trigger pair looks at `class_teacher`, so the
/// project ends up holding a state M2 documented as unreachable.
pub fn ineligible_teacher() -> (Project, School) {
    let mut project = Project::new();
    let mut school = School::new(
        &mut project,
        "Thornbury Federation",
        &["Thornbury North", "Thornbury South"],
        5,
    );
    school
        .bells(&mut project, "Federation Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "North 1", 0, None, 30, &[]);
    school.room(&mut project, "South 1", 1, None, 30, &[]);

    let teacher = school.teacher(&mut project, "Lanyon");
    let group = school.group(&mut project, "9A", Some(28), None);
    let physics = school.subject(&mut project, "Physics");
    let section = school.section(
        &mut project,
        physics,
        "9A Physics",
        Some(0),
        None,
        Some(2),
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);

    // The narrowing that strands the assignment already made above.
    school.narrow_to_campus(&mut project, teacher, 1);

    project.reopen();
    (project, school)
}

// =======================================================================================
// Helpers
// =======================================================================================
