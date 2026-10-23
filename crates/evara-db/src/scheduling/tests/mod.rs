//! What the loader must produce from a real project, and what it must refuse.
//!
//! # Every test is a round trip
//!
//! Each fixture is written into a temporary `.evaraproj` **through the repositories**, the
//! project is closed, it is reopened, and only then is the problem assembled. That sequence
//! is the point: a loader validated through the handle that wrote the rows would not prove
//! that anything reached the file, and the whole value of this milestone is that it reads
//! what a school actually has.
//!
//! # What is asserted, and what deliberately is not
//!
//! Identifiers are minted by the repositories and differ between runs, so nothing here
//! asserts a particular `Uuid`. What is asserted is **order, counts, resolved facts and
//! diagnostic codes** — the things the loader's contract is actually about — plus, for every
//! fixture, that [`Problem::integrity`] is empty and that two loads agree.
//!
//! # The expectations come out one short of M2's, on purpose
//!
//! A database-backed school with two or more student groups **always** carries exactly one
//! `group-overlap-indeterminate` warning, because M4 compiles no overlap answers at all
//! (ADR 0035 §3's evaluator is M5's). So the database-backed `tiny` is not findings-free the
//! way M2's `tiny` is, and that difference is a true statement about the product rather than
//! a gap in the fixture: until M5 exists, Evara does not know whether any two of a school's
//! groups share a student, and M3's aggregate warning is where it says so.

mod seed;

use evara_core::scheduling::{
    BellScheduleResolution, DiagnosticCode, ExpectedAttendance, NoBellScheduleCause, Severity,
};
use uuid::Uuid;

use crate::repo::availability::{AvailabilityEntity, AvailabilityStatus};
use crate::scheduling::{AssemblyReport, Completeness, SchedulingSelection};
use crate::{Assembly, DbError};
use seed::{LONG_DAY, Project, STANDARD_DAY, School};

// =======================================================================================
// The fixtures
// =======================================================================================

/// A small, correctly configured school: one campus, one school-wide bell schedule.
///
/// Five positions, five periods of which four teach, so the grid holds 25 slots and 20
/// teaching ones. Two sections, each taught by one teacher to exactly one sized group — the
/// configuration that lets `expected_attendance` be `Known` at `V15`, since a one-element
/// audience is vacuously pairwise disjoint.
fn tiny() -> (Project, School) {
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
fn medium() -> (Project, School) {
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
fn infeasible_duration() -> (Project, School) {
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
fn infeasible_room_type() -> (Project, School) {
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
fn multiple_campuses() -> (Project, School) {
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
fn ambiguous_bells() -> (Project, School) {
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
fn unmaterialised_bells() -> (Project, School) {
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
fn shared_teacher() -> (Project, School, School) {
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
fn unknown_overlap() -> (Project, School) {
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
fn missing_grid() -> (Project, School) {
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
fn availability_restrictions() -> (Project, School) {
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
fn ineligible_teacher() -> (Project, School) {
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

/// Loads a fixture and asserts the things every sound load must satisfy.
///
/// Three properties, checked for every fixture rather than once: the compiled problem is
/// internally consistent, the identifier maps cover exactly the tables they describe, and
/// loading twice gives the same answer.
fn assemble(project: &Project, school: &School) -> Assembly {
    let assembly = project
        .load(&school.selection())
        .expect("the fixture assembles");

    assert_eq!(
        assembly.integrity(),
        Vec::new(),
        "a sound load has no internal inconsistency"
    );

    let counts = assembly.index.counts();
    let problem = &assembly.problem;
    assert_eq!(counts.campuses, problem.context.campuses.len());
    assert_eq!(counts.cycle_days, problem.context.cycle_days.len());
    assert_eq!(counts.bell_schedules, problem.context.bell_schedules.len());
    assert_eq!(counts.periods, problem.context.periods.len());
    assert_eq!(counts.timeslots, problem.timeslots.len());
    assert_eq!(counts.room_types, problem.room_types.len());
    assert_eq!(counts.rooms, problem.rooms.len());
    assert_eq!(counts.teachers, problem.teachers.len());
    assert_eq!(counts.groups, problem.groups.len());
    assert_eq!(counts.sections, problem.sections.len());
    assert_eq!(counts.activities, problem.activities.len());

    let again = project.load(&school.selection()).expect("loads again");
    assert_eq!(
        assembly.problem, again.problem,
        "two loads of one snapshot must produce the same problem"
    );
    assert_eq!(assembly.index, again.index, "and the same identifier maps");

    assembly
}

/// Loads and reports, for a test about diagnostics.
fn report(project: &Project, school: &School) -> AssemblyReport {
    AssemblyReport::of(&assemble(project, school))
}

/// The codes a report carries, in report order.
fn codes(report: &AssemblyReport) -> Vec<DiagnosticCode> {
    report.findings.findings.iter().map(|f| f.code).collect()
}

// =======================================================================================
// tiny
// =======================================================================================

#[test]
fn tiny_compiles_every_table_at_the_size_it_was_seeded() {
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);
    let problem = &assembly.problem;

    assert_eq!(problem.context.school.name, "Northgate Secondary");
    assert_eq!(problem.context.school.timezone, "Europe/London");
    assert_eq!(problem.context.campuses.len(), 1);
    assert_eq!(problem.context.cycle_days.len(), 5);
    assert_eq!(problem.context.bell_schedules.len(), 1);
    assert_eq!(problem.context.periods.len(), 5);
    // Five positions times five periods, the non-teaching one included: a break is a slot
    // and `no-teaching-in-period` stays a constraint because manual editing can name it.
    assert_eq!(problem.timeslots.len(), 25);
    assert_eq!(
        problem.timeslots.iter().filter(|s| s.is_teaching).count(),
        20
    );
    assert_eq!(problem.room_types.len(), 2);
    assert_eq!(problem.rooms.len(), 3);
    assert_eq!(problem.teachers.len(), 2);
    assert_eq!(problem.groups.len(), 2);
    assert_eq!(problem.sections.len(), 2);
    assert_eq!(problem.activities.len(), 8);
}

#[test]
fn tiny_identifies_the_school_cycle_and_term_it_was_asked_for() {
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    assert_eq!(assembly.selection, school.selection());
    assert_eq!(assembly.problem.context.school.id, school.id);
    assert_eq!(assembly.problem.context.cycle.id, school.cycle);
    assert_eq!(
        assembly.problem.context.term.as_ref().map(|t| t.id),
        school.term,
    );
    assert_eq!(
        assembly
            .problem
            .context
            .term
            .as_ref()
            .map(|t| t.name.as_str()),
        Some("Autumn"),
    );
}

#[test]
fn tiny_resolves_every_section_to_the_one_materialised_schedule() {
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    for section in &assembly.problem.sections {
        assert!(
            section.bell_schedule.is_resolved(),
            "{} should resolve under clause 1",
            section.name
        );
    }
}

#[test]
fn tiny_blocks_nothing_and_warns_only_about_overlap() {
    let (project, school) = tiny();
    let found = report(&project, &school);

    assert!(found.may_solve(), "{}", found.headline());
    assert_eq!(found.summary.blocking, 0);
    assert_eq!(found.summary.informational, 0);
    // The one warning a database-backed school with two groups always carries: M4 compiles
    // no overlap answers, so no pair is decided. See this module's header.
    assert_eq!(
        codes(&found),
        vec![DiagnosticCode::GroupOverlapIndeterminate]
    );
    assert_eq!(found.summary.indeterminate_group_pairs, 1);
    assert!(found.completeness.is_complete());
}

#[test]
fn a_section_taught_to_one_sized_group_knows_its_attendance() {
    // ADR 0034 §3 clause 3, in the only form `V15` can reach: "pairwise disjoint" over a
    // one-element audience is vacuously true, and that is the commonest configuration.
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    let attendance: Vec<ExpectedAttendance> = assembly
        .problem
        .sections
        .iter()
        .map(|s| s.expected_attendance)
        .collect();
    assert!(
        attendance
            .iter()
            .all(|a| matches!(a, ExpectedAttendance::Known { .. })),
        "{attendance:?}"
    );
    let mut people: Vec<i64> = attendance.iter().filter_map(|a| a.people()).collect();
    people.sort_unstable();
    assert_eq!(people, vec![26, 28]);
}

#[test]
fn a_room_requirement_narrows_the_candidate_set_to_the_rooms_that_satisfy_it() {
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    let lab_section = assembly
        .problem
        .sections
        .iter()
        .find(|s| s.required_room_type.is_some())
        .expect("one section requires a laboratory");
    assert_eq!(
        lab_section.room_candidates.len(),
        1,
        "only the laboratory satisfies it"
    );

    let open_section = assembly
        .problem
        .sections
        .iter()
        .find(|s| s.required_room_type.is_none())
        .expect("one section requires nothing");
    assert_eq!(
        open_section.room_candidates.len(),
        3,
        "no requirement admits every bookable room"
    );
}

#[test]
fn one_activity_row_is_one_occurrence() {
    // ADR 0029. Four single periods a cycle is four activities, each of duration 1, with
    // `split_index` 1 to 4 and never renumbered.
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    for (at, section) in assembly.problem.sections.iter().enumerate() {
        let ix = evara_core::scheduling::SectionIx::new(u32::try_from(at).expect("small"));
        let mine: Vec<&evara_core::scheduling::ActivityFact> = assembly
            .problem
            .activities
            .iter()
            .filter(|a| a.section == ix)
            .collect();
        assert_eq!(mine.len(), 4, "{} has four occurrences", section.name);
        assert!(mine.iter().all(|a| a.duration.get() == 1));
        let mut splits: Vec<u16> = mine.iter().map(|a| a.split_index).collect();
        splits.sort_unstable();
        assert_eq!(splits, vec![1, 2, 3, 4]);
    }
}

#[test]
fn every_activity_is_traceable_back_to_its_stored_row() {
    let (project, school) = tiny();
    let assembly = assemble(&project, &school);

    for (at, activity) in assembly.problem.activities.iter().enumerate() {
        let ix = evara_core::scheduling::ActivityIx::new(u32::try_from(at).expect("small"));
        assert_eq!(
            assembly.index.activity_id(ix),
            Some(activity.id),
            "the reverse map must name the row the fact came from"
        );
        assert_eq!(assembly.index.activity_index(activity.id), Some(ix));
    }
}

// =======================================================================================
// medium
// =======================================================================================

#[test]
fn medium_compiles_five_hundred_occurrences_over_thirty_slots() {
    let (project, school) = medium();
    let assembly = assemble(&project, &school);
    let problem = &assembly.problem;

    assert_eq!(problem.teachers.len(), 80);
    assert_eq!(problem.groups.len(), 25);
    assert_eq!(problem.sections.len(), 125);
    assert_eq!(problem.activities.len(), 500);
    assert_eq!(problem.timeslots.len(), 30);
    assert_eq!(
        problem.timeslots.iter().filter(|s| s.is_teaching).count(),
        30
    );
    assert_eq!(problem.rooms.len(), 20);

    // A group-level population of 1 000 with no `student` rows at all — the ADR 0006 school.
    let declared: i64 = problem.groups.iter().filter_map(|g| g.declared_size).sum();
    assert_eq!(declared, 1_000);
    assert!(problem.groups.iter().all(|g| g.effective_members == 0));
}

#[test]
fn medium_blocks_nothing() {
    let (project, school) = medium();
    let found = report(&project, &school);
    assert!(found.may_solve(), "{}", found.headline());
    assert_eq!(found.summary.blocking, 0);
    assert_eq!(
        codes(&found),
        vec![DiagnosticCode::GroupOverlapIndeterminate]
    );
    // 25 groups: every unordered pair is undecided.
    assert_eq!(found.summary.indeterminate_group_pairs, 25 * 24 / 2);
}

// =======================================================================================
// The conclusively impossible pair
// =======================================================================================

#[test]
fn an_occurrence_longer_than_any_run_is_refused() {
    let (project, school) = infeasible_duration();
    let found = report(&project, &school);

    assert!(!found.may_solve());
    assert!(
        found
            .findings
            .has(DiagnosticCode::DurationExceedsLongestRun)
    );
    let finding = found
        .of_severity(Severity::Blocking)
        .find(|f| f.code == DiagnosticCode::DurationExceedsLongestRun)
        .expect("the finding");
    assert!(
        finding.remedy.is_some(),
        "a blocking finding states a remedy"
    );
}

#[test]
fn a_room_type_nothing_satisfies_is_refused_once_per_section() {
    let (project, school) = infeasible_room_type();
    let found = report(&project, &school);

    assert!(!found.may_solve());
    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::NoRoomOfRequiredType),
        1,
        "once per section, never once per occurrence"
    );
    let assembly = assemble(&project, &school);
    assert!(
        assembly.problem.sections[0].room_candidates.is_empty(),
        "nothing satisfies the requirement"
    );
}

// =======================================================================================
// Bell-schedule resolution — ADR 0033 and ADR 0036
// =======================================================================================

#[test]
fn a_campus_scoped_schedule_resolves_the_section_at_that_campus() {
    let (project, school) = multiple_campuses();
    let assembly = assemble(&project, &school);

    assert_eq!(assembly.problem.context.campuses.len(), 2);
    assert_eq!(assembly.problem.context.bell_schedules.len(), 2);
    for section in &assembly.problem.sections {
        let BellScheduleResolution::Resolved { schedule } = section.bell_schedule else {
            panic!(
                "{} did not resolve: {:?}",
                section.name, section.bell_schedule
            );
        };
        let campus = section.campus.expect("each section names its campus");
        assert_eq!(
            assembly.problem.context.bell_schedules[schedule.get()].campus,
            Some(campus),
            "{} must draw from its own campus's schedule",
            section.name
        );
    }
    assert!(report(&project, &school).may_solve());
}

#[test]
fn a_shared_room_is_one_room_reachable_from_both_campuses() {
    // ADR 0014: modelling it as two rooms would give one physical space two occupancy
    // records, which that record calls a correctness failure.
    let (project, school) = multiple_campuses();
    let assembly = assemble(&project, &school);

    let shared: Vec<&evara_core::scheduling::RoomFact> = assembly
        .problem
        .rooms
        .iter()
        .filter(|r| r.campuses.len() == 2)
        .collect();
    assert_eq!(shared.len(), 1, "exactly one room reaches both campuses");
    assert_eq!(shared[0].name, "Shared Hall");
    assert_eq!(assembly.problem.rooms.len(), 3, "and it is not duplicated");
}

#[test]
fn two_compatible_schedules_are_never_chosen_between() {
    let (project, school) = ambiguous_bells();
    let found = report(&project, &school);

    assert!(!found.may_solve());
    assert!(found.findings.has(DiagnosticCode::AmbiguousBellSchedule));

    let assembly = assemble(&project, &school);
    let BellScheduleResolution::Ambiguous { candidates } =
        &assembly.problem.sections[0].bell_schedule
    else {
        panic!("expected an ambiguous resolution");
    };
    assert_eq!(candidates.len(), 2, "both candidates are named");
}

#[test]
fn a_defined_but_unmaterialised_schedule_is_not_a_candidate() {
    // ADR 0033 test 8, and the M2 documentation defect ADR 0036 §5 corrected: `G` is
    // derived from the slots, so a schedule with none cannot cause an ambiguity.
    let (project, school) = unmaterialised_bells();
    let assembly = assemble(&project, &school);

    assert_eq!(
        assembly.problem.context.bell_schedules.len(),
        2,
        "both are defined, and the context carries both"
    );
    let materialised: std::collections::BTreeSet<_> = assembly
        .problem
        .timeslots
        .iter()
        .map(|s| s.bell_schedule)
        .collect();
    assert_eq!(materialised.len(), 1, "only one has slots");

    assert!(assembly.problem.sections[0].bell_schedule.is_resolved());
    let found = report(&project, &school);
    assert!(!found.findings.has(DiagnosticCode::AmbiguousBellSchedule));
    assert!(found.may_solve(), "{}", found.headline());
}

#[test]
fn an_empty_grid_is_reported_once_about_the_school() {
    let (project, school) = missing_grid();
    let assembly = assemble(&project, &school);
    assert_eq!(assembly.problem.timeslots, Vec::new());
    assert_eq!(
        assembly.problem.sections[0].bell_schedule,
        BellScheduleResolution::None {
            cause: NoBellScheduleCause::GridEmpty
        }
    );

    let found = report(&project, &school);
    assert!(!found.may_solve());
    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::NoGridMaterialised),
        1
    );
    assert!(
        !found.findings.has(DiagnosticCode::NoBellSchedule),
        "the per-section stage is skipped entirely when the grid is empty"
    );
}

// =======================================================================================
// Shared identities — ADR 0021 and ADR 0032 rule 3
// =======================================================================================

#[test]
fn a_teacher_employed_by_two_schools_is_one_identity_in_both() {
    let (project, north, south) = shared_teacher();
    let from_north = assemble(&project, &north);
    let from_south = assemble(&project, &south);

    let north_teacher = &from_north.problem.teachers[0];
    let south_teacher = &from_south.problem.teachers[0];
    assert_eq!(
        north_teacher.id, south_teacher.id,
        "a person belongs to the project, a membership to the school"
    );
    assert_eq!(north_teacher.also_at.len(), 1);
    assert_eq!(north_teacher.also_at[0].name, "Southmoor College");
    assert_eq!(south_teacher.also_at[0].name, "Northgate Secondary");
}

#[test]
fn sharing_across_schools_is_warned_about_and_never_blocks() {
    let (project, north, _south) = shared_teacher();
    let found = report(&project, &north);

    assert!(
        found.may_solve(),
        "blocking would make a configuration three ADRs support unusable: {}",
        found.headline()
    );
    assert!(
        found
            .findings
            .has(DiagnosticCode::SharedEntityAcrossSchools)
    );
    for finding in found.of_severity(Severity::Warning) {
        if finding.code == DiagnosticCode::SharedEntityAcrossSchools {
            assert!(
                finding.message.contains("Southmoor College"),
                "the warning must name the other school: {}",
                finding.message
            );
        }
    }
}

#[test]
fn a_room_shared_across_schools_keeps_one_identity_and_is_reported() {
    let (project, north, south) = shared_teacher();
    let from_north = assemble(&project, &north);
    let from_south = assemble(&project, &south);

    let hall_north = from_north
        .problem
        .rooms
        .iter()
        .find(|r| r.name == "Joint Sports Hall")
        .expect("the hall is in scope at Northgate");
    let hall_south = from_south
        .problem
        .rooms
        .iter()
        .find(|r| r.name == "Joint Sports Hall")
        .expect("and at Southmoor");
    assert_eq!(
        hall_north.id, hall_south.id,
        "one physical room, one identity"
    );
    assert_eq!(hall_north.also_at.len(), 1);
    assert_eq!(hall_south.also_at.len(), 1);
}

#[test]
fn one_school_never_sees_another_schools_activities() {
    let (project, north, south) = shared_teacher();
    let from_north = assemble(&project, &north);
    let from_south = assemble(&project, &south);

    assert_eq!(from_north.problem.sections.len(), 1);
    assert_eq!(from_south.problem.sections.len(), 1);
    assert_eq!(from_north.problem.sections[0].name, "9A Mathematics");
    assert_eq!(
        from_south.problem.sections[0].name,
        "12A Further Mathematics"
    );
    assert_eq!(from_north.problem.groups.len(), 1, "9A only");
    assert_eq!(from_south.problem.groups.len(), 1, "12A only");
}

// =======================================================================================
// Student groups — ADR 0029 and ADR 0035
// =======================================================================================

#[test]
fn the_parent_edge_and_the_declared_size_are_both_loaded() {
    let (project, school) = unknown_overlap();
    let assembly = assemble(&project, &school);

    assert_eq!(assembly.problem.groups.len(), 5);
    let cohort = assembly
        .problem
        .groups
        .iter()
        .position(|g| g.name == "Year 9")
        .expect("the cohort");
    for (at, group) in assembly.problem.groups.iter().enumerate() {
        if at == cohort {
            assert_eq!(group.parent, None);
            assert_eq!(group.declared_size, Some(120));
        } else {
            assert_eq!(
                group.parent.map(evara_core::scheduling::GroupIx::get),
                Some(cohort),
                "{} is a child of the cohort",
                group.name
            );
            assert_eq!(group.declared_size, Some(30));
        }
    }
}

#[test]
fn containment_is_decided_and_sibling_pairs_are_not() {
    // **Changed by M5, and this is the record of it.** M4 compiled
    // `OverlapRelation::unknown`, so this fixture's ten pairs were all undecided and the
    // test asserted exactly that. ADR 0035 §3's rules now run, and four of the ten are
    // decided: the cohort contains each of its four children, which is rule 4.
    //
    // The remaining six are the four children against each other, and they stay
    // `Indeterminate` because `Year 9` has not answered §4's question — which is the state
    // every project is in until the `children_are_disjoint` column exists and is filled in.
    // That is the honest answer, not a gap: `9-Mat-1` and `9-Fre-1` are structurally
    // identical to `9A` and `9B` in the stored data.
    let (project, school) = unknown_overlap();
    let assembly = assemble(&project, &school);
    let groups = &assembly.problem.groups;

    let cohort = groups
        .iter()
        .position(|g| g.name == "Year 9")
        .expect("the cohort");
    let cohort_ix = evara_core::scheduling::GroupIx::new(u32::try_from(cohort).expect("small"));

    for (at, group) in groups.iter().enumerate() {
        let ix = evara_core::scheduling::GroupIx::new(u32::try_from(at).expect("small"));
        if at == cohort {
            continue;
        }
        assert_eq!(
            assembly.problem.overlap.get(cohort_ix, ix),
            evara_core::scheduling::Overlap::Overlaps,
            "the cohort contains {} — rule 4",
            group.name
        );
    }

    // Six child-against-child pairs, none of them settled.
    assert_eq!(assembly.problem.overlap.indeterminate_pairs(), 6);

    assert!(
        groups
            .iter()
            .all(|g| g.children_are_disjoint.is_unanswered()),
        "`student_group.children_are_disjoint` is still a future column, so every group \
         reads NotStated and rule 6 cannot fire"
    );
}

#[test]
fn the_diagonal_says_a_group_overlaps_itself() {
    // `PRE-SOLVE-VALIDATION.md` §14's limitation 9, closed. M2's fixtures and M4's loader
    // both left `overlap(g, g)` unstated; ADR 0035 rule 2 makes it `Overlaps`.
    let (project, school) = unknown_overlap();
    let assembly = assemble(&project, &school);

    for (at, group) in assembly.problem.groups.iter().enumerate() {
        let ix = evara_core::scheduling::GroupIx::new(u32::try_from(at).expect("small"));
        assert_eq!(
            assembly.problem.overlap.get(ix, ix),
            evara_core::scheduling::Overlap::Overlaps,
            "{} must overlap itself",
            group.name
        );
    }
}

#[test]
fn a_clash_mask_claims_what_is_proved_and_no_more() {
    // **Changed by M5.** Under M4 nothing was decided, so every section's clash mask equalled
    // its own audience and the test asserted that. Rule 4 now proves the cohort contains each
    // child, so a section taught to one child clashes with anything taught to the cohort —
    // which is the whole point of the constraint and is what M4 could not yet express.
    //
    // What has *not* changed is the discipline: `clash_mask` admits only `Overlaps`, so the
    // six undecided sibling pairs are still absent from it. An undecided pair is neither a
    // clash nor a licence (ADR 0038 §4).
    let (project, school) = unknown_overlap();
    let assembly = assemble(&project, &school);
    let groups = &assembly.problem.groups;
    let cohort = groups
        .iter()
        .position(|g| g.name == "Year 9")
        .expect("the cohort");
    let cohort_ix = evara_core::scheduling::GroupIx::new(u32::try_from(cohort).expect("small"));

    for section in &assembly.problem.sections {
        assert_eq!(section.audience_mask.len(), 1, "one group each");
        let attending = section.audience[0];

        // Its own audience, and the cohort above it. Nothing else: the sibling pairs are
        // undecided and are not promoted into a clash.
        assert!(section.clash_mask.contains(attending));
        assert!(
            section.clash_mask.contains(cohort_ix),
            "{} is taught to a child of the cohort, so it clashes with the cohort",
            section.name
        );
        assert_eq!(
            section.clash_mask.len(),
            2,
            "{} must claim exactly what is proved — itself and its cohort",
            section.name
        );
    }
}

#[test]
fn an_indeterminate_relation_produces_one_aggregate_warning() {
    let (project, school) = unknown_overlap();
    let found = report(&project, &school);

    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::GroupOverlapIndeterminate),
        1,
        "one finding per school, never one per pair"
    );
    let finding = found
        .of_severity(Severity::Warning)
        .find(|f| f.code == DiagnosticCode::GroupOverlapIndeterminate)
        .expect("the finding");
    assert!(
        // Six, not ten: M5's rule 4 settled the four cohort-against-child pairs. The count
        // is read from the relation, so it falls as the evaluator decides more.
        finding.message.contains("6 pairs"),
        "it carries the arithmetic: {}",
        finding.message
    );
    assert!(
        finding.message.contains("would settle")
            || finding.message.contains("would narrow")
            || finding.message.contains("already answered"),
        "and says what answering buys, now that rule 6 can predict it: {}",
        finding.message
    );
    assert!(
        !finding.message.contains("resolves"),
        "a prediction about a hypothesis, not a promise: {}",
        finding.message
    );
}

#[test]
fn effective_membership_counts_only_students_who_could_be_in_the_room() {
    // ADR 0035 §2's conjunction: active, enrolled here, and not LEFT. The subtree union
    // matters too — a student named only in a child counts towards the parent.
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Vale Park School", &["Vale Park"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    let cohort = school.group(&mut project, "Year 10", None, None);
    let class = school.group(&mut project, "10A", None, Some(cohort));
    let students: Vec<Uuid> = (0..3)
        .map(|n| school.student_in(&mut project, class, n))
        .collect();
    project.reopen();

    assert_eq!(members(&project, &school, "10A"), 3);
    assert_eq!(
        members(&project, &school, "Year 10"),
        3,
        "containment is what the parent edge means, so the subtree is counted"
    );

    // One child leaves. Their `student_group_member` row stays — that is the record — and
    // they stop being somebody who could be in the room.
    school.student_leaves(&mut project, students[0]);
    project.reopen();
    assert_eq!(members(&project, &school, "10A"), 2);
    assert_eq!(members(&project, &school, "Year 10"), 2);

    // And a child deactivated project-wide is likewise not competing for a timeslot.
    let remaining = project
        .db()
        .people()
        .student(students[1])
        .expect("read the student")
        .expect("it is there");
    project
        .db()
        .people()
        .update_student(
            students[1],
            crate::Expected::Rev(remaining.rev),
            &crate::StudentInput {
                is_active: false,
                school_id: remaining.school_id,
                code: remaining.code,
                given_name: remaining.given_name,
                family_name: remaining.family_name,
                email: remaining.email,
                year_level_id: remaining.year_level_id,
                campus_id: remaining.campus_id,
            },
        )
        .expect("deactivate the student");
    project.reopen();
    assert_eq!(members(&project, &school, "10A"), 1);
}

/// One group's effective membership, as the loader computes it.
fn members(project: &Project, school: &School, name: &str) -> i64 {
    assemble(project, school)
        .problem
        .groups
        .iter()
        .find(|g| g.name == name)
        .expect("the group")
        .effective_members
}

// =======================================================================================
// Availability — ADR 0033 §7 and ADR 0037
// =======================================================================================

#[test]
fn a_cycle_day_rule_covers_that_day_on_every_schedule() {
    let (project, school) = availability_restrictions();
    let assembly = assemble(&project, &school);

    let teacher = evara_core::scheduling::AvailabilityEntity::Teacher {
        teacher: evara_core::scheduling::TeacherIx::new(0),
    };
    let restricted = assembly
        .problem
        .unavailable
        .unavailable_slots(teacher)
        .expect("the teacher is restricted");
    // One cycle day of five, and every period of it — the break included, because a rule
    // about a day is a rule about the day.
    assert_eq!(restricted.len(), 5);
}

#[test]
fn a_period_rule_covers_that_period_on_every_day() {
    let (project, school) = availability_restrictions();
    let assembly = assemble(&project, &school);

    let room = evara_core::scheduling::AvailabilityEntity::Room {
        room: evara_core::scheduling::RoomIx::new(0),
    };
    let restricted = assembly
        .problem
        .unavailable
        .unavailable_slots(room)
        .expect("the room is restricted");
    assert_eq!(restricted.len(), 5, "one period across five days");
}

#[test]
fn a_discouraged_rule_is_soft_and_is_not_an_unavailability() {
    // "Do not assume DISCOURAGED means hard unavailable": it is legal and merely worse.
    let (project, school) = availability_restrictions();
    let assembly = assemble(&project, &school);

    let group = evara_core::scheduling::AvailabilityEntity::Group {
        group: evara_core::scheduling::GroupIx::new(0),
    };
    assert!(
        assembly
            .problem
            .unavailable
            .unavailable_slots(group)
            .is_none(),
        "a discouraged slot is not an unavailable one"
    );
    let discouraged = assembly
        .problem
        .timeslots
        .iter()
        .enumerate()
        .filter(|(at, _)| {
            assembly.problem.unavailable.status(
                group,
                evara_core::scheduling::TimeslotIx::new(u32::try_from(*at).expect("small")),
            ) == evara_core::scheduling::SlotAvailability::Discouraged
        });
    assert_eq!(discouraged.count(), 5, "the whole of one cycle day");
}

#[test]
fn an_availability_rule_naming_a_deleted_entity_is_reported_not_dropped() {
    // ADR 0037 future acceptance test 3.
    let (project, school) = availability_restrictions();
    let found = report(&project, &school);

    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::OrphanAvailabilityReference),
        1
    );
    let finding = found
        .of_severity(Severity::Warning)
        .find(|f| f.code == DiagnosticCode::OrphanAvailabilityReference)
        .expect("the finding");
    assert_eq!(
        finding.subject.kind,
        evara_core::scheduling::EntityKind::AvailabilityRule,
        "the subject is the row, which is what exists and what can be removed"
    );
    assert!(finding.message.contains("teacher"), "{}", finding.message);
    assert!(finding.remedy.is_some(), "and it says what to do about it");
    assert_eq!(found.stats.rules_orphaned, 1);
}

#[test]
fn an_availability_rule_for_an_unscheduled_kind_says_it_is_not_enforced() {
    // ADR 0037 future acceptance test 4.
    let (project, school) = availability_restrictions();
    let found = report(&project, &school);

    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::AvailabilityEntityKindNotScheduled),
        1
    );
    let finding = found
        .of_severity(Severity::Info)
        .find(|f| f.code == DiagnosticCode::AvailabilityEntityKindNotScheduled)
        .expect("the finding");
    assert!(finding.message.contains("resource"), "{}", finding.message);
    assert!(
        finding.message.contains("not being enforced"),
        "it must say plainly that the rule does nothing: {}",
        finding.message
    );

    // And the assembly is marked incomplete, so a caller cannot read it as a problem with
    // every stored restriction applied.
    assert_eq!(
        found.completeness,
        Completeness::RestrictionsNotEnforced { rules: 1 }
    );
    assert!(!found.completeness.is_complete());
    assert!(
        found.may_solve(),
        "an unenforced soft restriction does not block: {}",
        found.headline()
    );
}

#[test]
fn an_unenforceable_rule_is_absent_from_the_compiled_rule_list() {
    let (project, school) = availability_restrictions();
    let assembly = assemble(&project, &school);

    // Three of the five rules compiled; the orphan and the resource did not.
    assert_eq!(assembly.rules.len(), 3);
    assert_eq!(assembly.stats.rules_compiled, 3);
    assert_eq!(assembly.stats.rules_orphaned, 1);
    assert_eq!(assembly.stats.rules_not_scheduled, 1);
    assert!(
        assembly.rules.iter().all(|r| r.effective.from.is_none()),
        "the fixture bounds no rule by date"
    );
}

// =======================================================================================
// Teacher eligibility — ADR 0028 and ADR 0037 §4
// =======================================================================================

#[test]
fn a_teacher_stranded_by_a_later_narrowing_is_reported_as_corruption() {
    // ADR 0037 future acceptance test 6, reached without bypassing a trigger — see
    // `ineligible_teacher` for the two supported calls that get there.
    let (project, school) = ineligible_teacher();
    let found = report(&project, &school);

    assert!(!found.may_solve(), "a damaged project refuses the solve");
    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::TeacherNotEligibleForCampus),
        1
    );
    let finding = found
        .of_severity(Severity::Blocking)
        .find(|f| f.code == DiagnosticCode::TeacherNotEligibleForCampus)
        .expect("the finding");
    assert_eq!(
        finding.subject.kind,
        evara_core::scheduling::EntityKind::Teacher
    );
    assert!(
        finding.message.contains("9A Physics"),
        "{}",
        finding.message
    );
    assert!(
        finding.message.contains("Thornbury North"),
        "{}",
        finding.message
    );
    assert!(finding.remedy.is_some());
    assert_eq!(
        finding.related.len(),
        2,
        "the section and the campus are both named"
    );
}

#[test]
fn an_ordinary_project_performs_no_eligibility_check() {
    // ADR 0037 future acceptance test 7: none of the three loader codes fires, and an
    // unnarrowed teacher is never reported — reading an empty narrowing as "eligible
    // nowhere" is the mistake `V15`'s own trigger comment warns against.
    for (project, school) in [tiny(), multiple_campuses()] {
        let found = report(&project, &school);
        for code in crate::scheduling::LOADER_DIAGNOSTICS {
            assert!(
                !found.findings.has(*code),
                "{} fired on an ordinary project",
                code.key()
            );
        }
    }
}

#[test]
fn an_unnarrowed_teacher_is_eligible_everywhere() {
    let (project, school) = multiple_campuses();
    let assembly = assemble(&project, &school);
    assert!(
        assembly
            .problem
            .teachers
            .iter()
            .all(|t| t.eligible_campuses.is_none()),
        "no `teacher_campus` row means not narrowed, which is not the same as nowhere"
    );
}

#[test]
fn a_deactivated_teacher_keeps_the_sections_they_already_had() {
    // ADR 0018, and the reason `Problem::teachers` cannot be only the active memberships:
    // `V15`'s trigger pair fires on `class_teacher` and never on `teacher_school`, so
    // deactivating somebody who already teaches is permitted and must not dangle.
    let (mut project, school) = tiny();
    let teacher = school.teachers[0];
    project
        .db()
        .membership()
        .set_teacher_membership_active(school.id, teacher, crate::Expected::Any, false)
        .expect("deactivate the membership");
    project.reopen();

    let assembly = assemble(&project, &school);
    assert_eq!(
        assembly.problem.teachers.len(),
        2,
        "the deactivated teacher is still in scope because a section names them"
    );
    assert_eq!(assembly.stats.teachers_without_active_membership, 1);
    assert_eq!(
        assembly.integrity(),
        Vec::new(),
        "and nothing dangles as a result"
    );
    assert!(
        assembly
            .problem
            .sections
            .iter()
            .any(|s| s.teachers.len() == 1),
        "the assignment survived"
    );
}

// =======================================================================================
// Scope, failure, and determinism
// =======================================================================================

#[test]
fn a_cycle_of_another_school_is_refused_before_anything_is_read() {
    let (project, north, south) = shared_teacher();
    let crossed = SchedulingSelection {
        school_id: north.id,
        cycle_id: south.cycle,
        term_id: None,
    };
    assert!(matches!(
        project.load(&crossed),
        Err(DbError::WrongSchool { entity: "cycle" })
    ));
}

#[test]
fn a_term_of_another_school_is_refused() {
    let mut project = Project::new();
    let mut north = School::new(&mut project, "Northgate Secondary", &["Northgate Main"], 5);
    let mut south = School::new(&mut project, "Southmoor College", &["Southmoor Main"], 5);
    north.with_term(&mut project);
    south.with_term(&mut project);
    project.reopen();

    let crossed = SchedulingSelection {
        school_id: north.id,
        cycle_id: north.cycle,
        term_id: south.term,
    };
    assert!(matches!(
        project.load(&crossed),
        Err(DbError::WrongSchool { entity: "term" })
    ));
}

#[test]
fn an_unknown_school_is_not_found() {
    let (project, school) = tiny();
    let missing = SchedulingSelection {
        school_id: Uuid::now_v7(),
        cycle_id: school.cycle,
        term_id: None,
    };
    // The cycle guard answers first: it belongs to a school, and that school is not the one
    // asked about. Either way it is an execution failure and not a diagnostic.
    assert!(matches!(
        project.load(&missing),
        Err(DbError::WrongSchool { .. } | DbError::NotFound { .. })
    ));
}

#[test]
fn an_empty_school_assembles_to_an_empty_problem_rather_than_an_error() {
    // A school part-way through setup is not an error, and the loader must not invent one.
    let mut project = Project::new();
    let school = School::new(&mut project, "Quarry Bank School", &["Quarry Bank"], 5);
    project.reopen();

    let assembly = assemble(&project, &school);
    assert_eq!(assembly.problem.sections, Vec::new());
    assert_eq!(assembly.problem.activities, Vec::new());
    assert_eq!(assembly.problem.timeslots, Vec::new());
    assert_eq!(assembly.integrity(), Vec::new());

    let found = AssemblyReport::of(&assembly);
    assert!(!found.may_solve(), "there is no grid to place anything in");
    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::NoGridMaterialised),
        1
    );
}

#[test]
fn a_report_is_identical_across_two_runs() {
    let (project, school) = availability_restrictions();
    let first = report(&project, &school);
    let second = report(&project, &school);
    assert_eq!(first, second, "a report must be reproducible");
}

#[test]
fn severity_never_decreases_down_a_combined_report() {
    // The property M3 asserts over its own report, over a report that holds both producers'
    // findings at all three severities.
    let (project, school) = availability_restrictions();
    let found = report(&project, &school);
    let severities: Vec<Severity> = found
        .findings
        .findings
        .iter()
        .map(evara_core::scheduling::Diagnostic::severity)
        .collect();
    let mut sorted = severities.clone();
    sorted.sort_unstable();
    assert_eq!(severities, sorted, "{severities:?}");
}

#[test]
fn integrity_blocks_a_solve_even_with_no_blocking_finding() {
    // The distinction ADR 0036 §4 draws, asserted on the combining side: an internal
    // inconsistency is somebody else's defect and is not shown as a field to fix, but it
    // must still stop a solve.
    let (project, school) = tiny();
    let mut assembly = assemble(&project, &school);
    assembly.problem.activities[0].section = evara_core::scheduling::SectionIx::new(99);

    let found = AssemblyReport::of(&assembly);
    assert_ne!(found.integrity, Vec::new());
    assert!(
        !found.may_solve(),
        "a problem with a dangling index is not one to search over"
    );
    assert!(
        !found
            .findings
            .findings
            .iter()
            .any(|f| f.severity() == Severity::Blocking),
        "and it is not dressed up as something the user should fix"
    );
}

#[test]
fn the_loader_writes_nothing() {
    // The read-only guarantee, measured rather than asserted from the type: the change feed
    // is the record of every mutation, so a load that appended to it would be visible here.
    let (mut project, school) = tiny();
    let before = project
        .db()
        .changes_since(0, 100_000)
        .expect("read the change feed")
        .len();
    let revision_before = project
        .db()
        .structure()
        .school(school.id)
        .expect("read the school")
        .expect("it is there")
        .rev;

    for _ in 0..3 {
        let found = report(&project, &school);
        assert!(found.may_solve());
    }

    let after = project
        .db()
        .changes_since(0, 100_000)
        .expect("read the change feed")
        .len();
    assert_eq!(before, after, "loading appended to the change feed");
    assert_eq!(
        revision_before,
        project
            .db()
            .structure()
            .school(school.id)
            .expect("read the school")
            .expect("it is there")
            .rev,
        "loading consumed a revision"
    );
}

#[test]
fn a_term_selects_its_own_courses_and_the_year_long_ones() {
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Ashcroft School", &["Ashcroft"], 5);
    school
        .with_term(&mut project)
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);
    let teacher = school.teacher(&mut project, "Mowbray");
    let group = school.group(&mut project, "9A", Some(28), None);
    let subject = school.subject(&mut project, "Mathematics");

    let all_year = school.section(
        &mut project,
        subject,
        "All year",
        None,
        None,
        Some(1),
        false,
    );
    school.teach(&mut project, all_year, teacher);
    school.attend(&mut project, all_year, group);
    school.occurrences(&mut project, all_year, 1, 1);

    let autumn = school.section(
        &mut project,
        subject,
        "Autumn only",
        None,
        None,
        Some(1),
        true,
    );
    school.teach(&mut project, autumn, teacher);
    school.attend(&mut project, autumn, group);
    school.occurrences(&mut project, autumn, 1, 1);
    project.reopen();

    // The selected term: the year-long course and its own.
    let in_term = project.load(&school.selection()).expect("loads");
    assert_eq!(in_term.problem.sections.len(), 2);

    // No term named: every course, because there is no basis on which to exclude one and
    // dropping a term-specific course would under-count demand.
    let year_long = project
        .load(&SchedulingSelection {
            term_id: None,
            ..school.selection()
        })
        .expect("loads");
    assert_eq!(
        year_long.problem.sections.len(),
        2,
        "a term-less selection excludes nothing"
    );
    assert!(
        year_long.problem.timeslots.is_empty(),
        "and there is no term-less grid, which is a separate matter"
    );
}

// =======================================================================================
// Performance
// =======================================================================================

/// Measures assembly and validation, and fails only on a collapse.
///
/// # Why the bound is loose
///
/// A wall-clock assertion on a shared CI runner is a flaky test waiting to happen, so this
/// does not pin a number. What it does catch is the failure mode that actually threatens
/// this loader: an N+1 query. The loader issues a fixed number of statements per load —
/// twenty-nine, thirty-one with a term — independent of school size, so `medium` at 125
/// sections and 500 occurrences must stay within an order of magnitude of `tiny` at 2 and 8.
/// A per-row query would put `medium` into seconds, and the ceiling here is comfortably
/// below that while being far above any honest machine.
///
/// The figures themselves are printed, so `cargo test -- --nocapture` reports them rather
/// than leaving a reviewer to guess.
#[test]
fn assembling_a_realistic_school_does_not_scale_with_its_size() {
    use std::time::Instant;

    let (tiny_project, tiny_school) = tiny();
    let at = Instant::now();
    let tiny_assembly = tiny_project
        .load(&tiny_school.selection())
        .expect("tiny assembles");
    let tiny_load = at.elapsed();

    let (medium_project, medium_school) = medium();
    let at = Instant::now();
    let medium_assembly = medium_project
        .load(&medium_school.selection())
        .expect("medium assembles");
    let medium_load = at.elapsed();

    let at = Instant::now();
    let medium_report = AssemblyReport::of(&medium_assembly);
    let medium_validate = at.elapsed();

    println!(
        "tiny     assemble: {tiny_load:?}  ({} activities)",
        tiny_assembly.problem.activities.len()
    );
    println!(
        "medium   assemble: {medium_load:?}  ({} activities)",
        medium_assembly.problem.activities.len()
    );
    println!(
        "medium   validate: {medium_validate:?}  ({})",
        medium_report.headline()
    );

    assert!(
        medium_load < std::time::Duration::from_secs(2),
        "assembling 125 sections and 500 occurrences took {medium_load:?}, which looks like a \
         query per row"
    );
    assert!(
        medium_validate < std::time::Duration::from_secs(2),
        "validating took {medium_validate:?}"
    );
}

// =======================================================================================
// The read-only inspection path
// =======================================================================================

#[test]
fn a_project_opened_for_inspection_still_assembles() {
    let (project, school) = tiny();
    let inspected =
        crate::Database::open_for_inspection(project.path()).expect("open for inspection");

    assert!(inspected.is_read_only());
    let assembly = inspected
        .scheduling()
        .load(&school.selection())
        .expect("assembles through an inspection handle");
    assert_eq!(assembly.problem.activities.len(), 8);
    assert_eq!(assembly.integrity(), Vec::new());
}

#[test]
fn a_project_opened_for_inspection_refuses_every_write() {
    // The typed first of three guarantees. The other two are SQLite's `query_only` pragma
    // and the absent `evara_site_id()` function, and neither depends on this one.
    let (project, school) = tiny();
    let mut inspected =
        crate::Database::open_for_inspection(project.path()).expect("open for inspection");

    let refused = inspected.structure().create_campus(&crate::CampusInput {
        school_id: school.id,
        name: "Should Not Exist".to_owned(),
        code: "SNE".to_owned(),
        address: None,
    });
    assert!(
        matches!(refused, Err(DbError::ReadOnlyProject)),
        "{refused:?}"
    );
}

#[test]
fn inspecting_a_project_does_not_stamp_it() {
    // `meta::ensure` writes `last_opened_with` on every ordinary open, which is exactly the
    // mutation a headless inspection of somebody's live file must not perform. Asserted
    // against the ordinary path so the test proves a difference rather than a constant.
    let (project, school) = tiny();
    let path = project.path().to_path_buf();
    let before = {
        let db = crate::Database::open_for_inspection(&path).expect("inspect");
        db.metadata(crate::meta::KEY_LAST_OPENED_WITH)
            .expect("read metadata")
    };

    for _ in 0..3 {
        let db = crate::Database::open_for_inspection(&path).expect("inspect");
        let _ = db
            .scheduling()
            .load(&school.selection())
            .expect("assembles");
        assert_eq!(
            db.metadata(crate::meta::KEY_LAST_OPENED_WITH)
                .expect("read metadata"),
            before,
            "inspection must not stamp the project"
        );
    }

    // And the ordinary path does, which is what makes the comparison meaningful.
    let stamped = crate::Database::open_existing(&path, "99.0.0-test", Uuid::now_v7())
        .expect("ordinary open");
    assert_eq!(
        stamped
            .metadata(crate::meta::KEY_LAST_OPENED_WITH)
            .expect("read metadata"),
        Some("99.0.0-test".to_owned())
    );
    assert_ne!(before, Some("99.0.0-test".to_owned()));
}

#[test]
fn inspection_refuses_a_file_that_is_not_a_project() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let stray = dir.path().join("notes.evaraproj");
    std::fs::write(&stray, b"this is not a database").expect("write the stray file");
    assert!(matches!(
        crate::Database::open_for_inspection(&stray),
        Err(DbError::NotAnEvaraProject { .. })
    ));

    let absent = dir.path().join("absent.evaraproj");
    assert!(matches!(
        crate::Database::open_for_inspection(&absent),
        Err(DbError::ProjectNotFound { .. })
    ));
}

#[test]
fn a_shared_rooms_classification_survives_and_does_not_match_a_same_named_type() {
    // §10: `create_room` does not hold a room's type to its campus's school, and a shared
    // room's type belongs to whichever school created it. Dropping the classification would
    // be wrong in the dangerous direction — an unclassified room is a candidate for *every*
    // requirement — so it is loaded even though the other school does not own it.
    let (project, north, south) = shared_teacher();
    let from_south = assemble(&project, &south);

    let hall = from_south
        .problem
        .rooms
        .iter()
        .find(|r| r.name == "Joint Sports Hall")
        .expect("the hall is in scope at Southmoor");
    let classified = hall.room_type.expect("its classification is kept");

    // Two room types are in scope at Southmoor: its own, and Northgate's, which the shared
    // hall is classified under. They share a name and are different records.
    assert_eq!(from_south.problem.room_types.len(), 2);
    let hall_type = &from_south.problem.room_types[classified.get()];
    assert_eq!(hall_type.name, "Sports Hall");

    let own = from_south
        .problem
        .room_types
        .iter()
        .position(|t| t.id != hall_type.id)
        .expect("Southmoor's own type of the same name");
    assert_ne!(
        own,
        classified.get(),
        "Southmoor has not said the shared hall is one of its sports halls"
    );

    // And the identity is Northgate's, so a section of Southmoor requiring *its* sports hall
    // would not be offered this room.
    let from_north = assemble(&project, &north);
    let north_type = from_north
        .problem
        .rooms
        .iter()
        .find(|r| r.name == "Joint Sports Hall")
        .and_then(|r| r.room_type)
        .map(|ix| from_north.problem.room_types[ix.get()].id)
        .expect("Northgate classifies it too");
    assert_eq!(hall_type.id, north_type, "one classification, one identity");
}

#[test]
fn an_unbookable_room_is_in_no_candidate_set() {
    // ADR 0034 §6: `is_bookable = 0` removes a room from every activity's candidate set, and
    // is unrelated to how many people fit. The loader reads it through
    // `RoomFact::is_candidate_for`, which is the one place the three conditions live.
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Oakhanger School", &["Oakhanger"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    let kind = school.room_type(&mut project, "Drama Studio");
    school.room(&mut project, "Studio 1", 0, Some(kind), 30, &[]);
    school.unbookable_room(&mut project, "Studio 2 (closed)", 0, Some(kind));

    let teacher = school.teacher(&mut project, "Nevin");
    let group = school.group(&mut project, "9A", Some(28), None);
    let drama = school.subject(&mut project, "Drama");
    let section = school.section(
        &mut project,
        drama,
        "9A Drama",
        None,
        Some(kind),
        Some(2),
        false,
    );
    school.teach(&mut project, section, teacher);
    school.attend(&mut project, section, group);
    school.occurrences(&mut project, section, 1, 2);
    project.reopen();

    let assembly = assemble(&project, &school);
    assert_eq!(assembly.problem.rooms.len(), 2, "both rooms are loaded");
    assert!(
        assembly.problem.rooms.iter().any(|r| !r.is_bookable),
        "and the stored flag is carried, not dropped"
    );
    assert_eq!(
        assembly.problem.sections[0].room_candidates.len(),
        1,
        "only the bookable studio is a candidate"
    );

    // Not a blocking finding: a candidate set of one is enough, and the closed studio is a
    // deliberate state rather than a problem to report.
    let found = report(&project, &school);
    assert!(!found.findings.has(DiagnosticCode::NoRoomOfRequiredType));
    assert!(found.may_solve(), "{}", found.headline());
}

#[test]
fn two_schools_groups_are_never_compared() {
    // ADR 0035 truth table row 20, and the one row that cannot be a unit test of the
    // evaluator: it needs two schools to exist. `student_group.school_id` is `NOT NULL` and
    // `V12`'s trigger pair holds a parent to its child's school, so a subtree never crosses a
    // school boundary — the restriction is structural rather than remembered, and this
    // asserts the structure rather than a guard.
    let (project, north, south) = shared_teacher();
    let from_north = assemble(&project, &north);
    let from_south = assemble(&project, &south);

    assert_eq!(from_north.problem.groups.len(), 1, "9A only");
    assert_eq!(from_south.problem.groups.len(), 1, "12A only");
    assert_ne!(
        from_north.problem.groups[0].id, from_south.problem.groups[0].id,
        "two schools teaching a same-named year hold two different groups"
    );

    // Each relation covers its own school's groups and no more, so there is no index at
    // which the other school's group could be asked about.
    assert_eq!(from_north.problem.overlap.groups(), 1);
    assert_eq!(from_south.problem.overlap.groups(), 1);
}

#[test]
fn a_shared_member_row_proves_overlap_without_any_declaration() {
    // ADR 0035 rule 3, from a real project: two sibling groups sharing one student, with no
    // `children_are_disjoint` answer anywhere — the column does not exist yet. The member row
    // alone decides it, which is the half of the specification that works at `V15`.
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Brackwell School", &["Brackwell"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);

    let cohort = school.group(&mut project, "Year 9", None, None);
    let set_x = school.group(&mut project, "Set X", None, Some(cohort));
    let set_y = school.group(&mut project, "Set Y", None, Some(cohort));

    // Three children in Set X, one of whom is also in Set Y.
    let shared = school.student_in(&mut project, set_x, 0);
    school.student_in(&mut project, set_x, 1);
    school.student_in(&mut project, set_y, 2);
    project
        .db()
        .people()
        .add_student_group_member(&crate::StudentGroupMemberInput {
            student_group_id: set_y,
            student_id: shared,
        })
        .expect("the shared membership");
    project.reopen();

    let assembly = assemble(&project, &school);
    let at = |name: &str| {
        evara_core::scheduling::GroupIx::new(
            u32::try_from(
                assembly
                    .problem
                    .groups
                    .iter()
                    .position(|g| g.name == name)
                    .expect("the group"),
            )
            .expect("small"),
        )
    };

    assert_eq!(
        assembly.problem.overlap.get(at("Set X"), at("Set Y")),
        evara_core::scheduling::Overlap::Overlaps,
        "one shared member row is a fact, and rule 3 reads it"
    );

    // The effective counts come from the same sets rule 3 intersected, so the subtree union
    // is visible here too: the cohort holds all three children though it has no member row.
    let count = |name: &str| assembly.problem.groups[at(name).get()].effective_members;
    assert_eq!(count("Set X"), 2);
    assert_eq!(count("Set Y"), 2);
    assert_eq!(
        count("Year 9"),
        3,
        "the subtree union, with no row of its own"
    );

    // No contradiction: nothing was declared, so nothing is contradicted.
    assert!(
        !AssemblyReport::of(&assembly)
            .findings
            .has(DiagnosticCode::GroupOverlapContradiction)
    );
}

// =======================================================================================
// V16 — the declaration rule 6 reads
// =======================================================================================

#[test]
fn an_unanswered_declaration_is_not_an_answer_of_no() {
    // `NULL` and `0` give the same overlap answer today and are different states, which is
    // why `DisjointnessDeclaration` keeps them apart and why `V16`'s column has no DEFAULT.
    // A migrated project reads `NULL`; a school that has said "they may overlap" reads `0`,
    // and pre-flight should stop asking it.
    let (mut project, school) = unknown_overlap();
    let groups = school.groups.clone();

    let before = assemble(&project, &school);
    assert!(
        before
            .problem
            .groups
            .iter()
            .all(|g| g.children_are_disjoint.is_unanswered()),
        "every group of a project migrated from V15 reads NotStated"
    );

    School::declare_children(&mut project, groups[0], Some(false));
    project.reopen();

    let after = assemble(&project, &school);
    let cohort = after
        .problem
        .groups
        .iter()
        .find(|g| g.name == "Year 9")
        .expect("the cohort");
    assert_eq!(
        cohort.children_are_disjoint,
        evara_core::scheduling::DisjointnessDeclaration::MayOverlap,
        "an explicit `0` is MayOverlap, not NotStated"
    );
    assert!(!cohort.children_are_disjoint.is_unanswered());
    assert!(!cohort.children_are_disjoint.proves_children_disjoint());

    // And it changes no verdict: rule 6 needs a *true* declaration, so the six sibling
    // pairs are still Indeterminate.
    assert_eq!(after.problem.overlap.indeterminate_pairs(), 6);
}

#[test]
fn a_declared_disjoint_parent_settles_its_childrens_pairs() {
    // ADR 0035 rule 6, from a real project — the half of `student-clash` that could not work
    // before `V16`. Six Indeterminate pairs become Disjoint on one answer, which is the
    // "linear number of answers" the record chose a per-parent flag to get.
    let (mut project, school) = unknown_overlap();
    let cohort = school.groups[0];

    assert_eq!(
        assemble(&project, &school)
            .problem
            .overlap
            .indeterminate_pairs(),
        6
    );

    School::declare_children(&mut project, cohort, Some(true));
    project.reopen();

    let after = assemble(&project, &school);
    assert_eq!(
        after.problem.overlap.indeterminate_pairs(),
        0,
        "one answer on one parent settles every sibling pair beneath it"
    );

    // The cohort still contains each child — a declaration about siblings never reaches a
    // parent-child pair, because rule 4 is consulted first.
    let at = |name: &str| {
        evara_core::scheduling::GroupIx::new(
            u32::try_from(
                after
                    .problem
                    .groups
                    .iter()
                    .position(|g| g.name == name)
                    .expect("the group"),
            )
            .expect("small"),
        )
    };
    assert_eq!(
        after.problem.overlap.get(at("Year 9"), at("9A")),
        evara_core::scheduling::Overlap::Overlaps
    );
    assert_eq!(
        after.problem.overlap.get(at("9A"), at("9B")),
        evara_core::scheduling::Overlap::Disjoint
    );

    // Nothing is left to warn about, so the aggregate warning stops firing entirely.
    let found = report(&project, &school);
    assert!(
        !found
            .findings
            .has(DiagnosticCode::GroupOverlapIndeterminate)
    );
    assert!(found.may_solve(), "{}", found.headline());
}

#[test]
fn a_member_row_beats_a_declaration_and_the_disagreement_is_named() {
    // ADR 0035 truth table row 9 and §6.3, end to end: rule 3 before rule 6. The pair
    // overlaps — the member row is what happened and the flag is what was intended — and the
    // contradiction is reported rather than resolved.
    let mut project = Project::new();
    let mut school = School::new(&mut project, "Calderstone School", &["Calderstone"], 5);
    school
        .bells(&mut project, "Standard Day", None, STANDARD_DAY)
        .materialise(&mut project);
    school.room(&mut project, "Room 1", 0, None, 30, &[]);

    let classes = school.group(&mut project, "Year 9 Classes", Some(120), None);
    let nine_a = school.group(&mut project, "9A", Some(28), Some(classes));
    let nine_b = school.group(&mut project, "9B", Some(28), Some(classes));
    School::declare_children(&mut project, classes, Some(true));

    // One child in both, which the declaration says cannot happen.
    let shared = school.student_in(&mut project, nine_a, 0);
    project
        .db()
        .people()
        .add_student_group_member(&crate::StudentGroupMemberInput {
            student_group_id: nine_b,
            student_id: shared,
        })
        .expect("the contradicting membership");
    project.reopen();

    let assembly = assemble(&project, &school);
    let at = |name: &str| {
        evara_core::scheduling::GroupIx::new(
            u32::try_from(
                assembly
                    .problem
                    .groups
                    .iter()
                    .position(|g| g.name == name)
                    .expect("the group"),
            )
            .expect("small"),
        )
    };
    assert_eq!(
        assembly.problem.overlap.get(at("9A"), at("9B")),
        evara_core::scheduling::Overlap::Overlaps,
        "the member row wins — rule 3 precedes rule 6"
    );

    let found = AssemblyReport::of(&assembly);
    assert_eq!(
        found
            .findings
            .count_code(DiagnosticCode::GroupOverlapContradiction),
        1
    );
    let finding = found
        .of_severity(Severity::Warning)
        .find(|f| f.code == DiagnosticCode::GroupOverlapContradiction)
        .expect("the finding");

    // Named, not resolved: it says which parent declared it, which two groups, and who.
    assert_eq!(finding.subject.label, "Year 9 Classes");
    assert!(finding.message.contains("9A"), "{}", finding.message);
    assert!(finding.message.contains("9B"), "{}", finding.message);
    assert!(finding.message.contains("Pupil0"), "{}", finding.message);
    assert!(finding.remedy.is_some());
    assert_eq!(
        finding.related.len(),
        3,
        "both groups and the student are all clickable"
    );

    // It warns and does not block: the clash is already prevented, and refusing to generate
    // over a data error the engine has handled correctly would be the wrong trade.
    assert!(found.may_solve(), "{}", found.headline());
}

#[test]
fn a_section_taught_to_two_proved_disjoint_sized_groups_knows_its_attendance() {
    // ADR 0034 §3 clause 3 in full, which is item 4 of M4's §20 contract. Before `V16` no
    // *pair* of groups could be proved disjoint, so a section taught to two of them was
    // `Unknown` however much its groups declared.
    let (mut project, school) = unknown_overlap();
    let cohort = school.groups[0];
    let classes = [school.groups[1], school.groups[2]];

    let mut school = school;
    let lecture_subject = school.subject(&mut project, "Further Mathematics");
    let teacher = school.teacher(&mut project, "Pellow");
    let section = school.section(
        &mut project,
        lecture_subject,
        "9 Mathematics lecture",
        None,
        None,
        Some(1),
        false,
    );
    school.teach(&mut project, section, teacher);
    for group in classes {
        school.attend(&mut project, section, group);
    }
    school.occurrences(&mut project, section, 1, 1);
    project.reopen();

    let lecture = |project: &Project, school: &School| {
        assemble(project, school)
            .problem
            .sections
            .iter()
            .find(|s| s.name == "9 Mathematics lecture")
            .expect("the lecture")
            .expected_attendance
    };

    // Undeclared: the two groups might share students, and summing would count somebody
    // twice. An over-stated attendance is a room a school is told is too small.
    assert_eq!(lecture(&project, &school), ExpectedAttendance::Unknown);

    School::declare_children(&mut project, cohort, Some(true));
    project.reopen();

    assert_eq!(
        lecture(&project, &school),
        ExpectedAttendance::Known { people: 60 },
        "two groups of thirty, proved disjoint, is sixty people"
    );
}

#[test]
fn the_summary_predicts_how_many_pairs_an_answer_would_settle() {
    // ADR 0038's future acceptance test 7: the two figures M3 shipped are unchanged, and the
    // prediction is added beside them rather than rewording either.
    let (mut project, school) = unknown_overlap();
    let before = report(&project, &school).summary;
    assert_eq!(before.indeterminate_group_pairs, 6);
    assert_eq!(before.unanswered_disjointness, 1);
    assert_eq!(
        before.resolvable_by_declaration, 6,
        "one answer on the cohort would settle all six"
    );

    School::declare_children(&mut project, school.groups[0], Some(true));
    project.reopen();

    let after = report(&project, &school).summary;
    assert_eq!(after.indeterminate_group_pairs, 0);
    assert_eq!(after.unanswered_disjointness, 0);
    assert_eq!(
        after.resolvable_by_declaration, 0,
        "nothing is left to predict once the question has been answered"
    );
}
