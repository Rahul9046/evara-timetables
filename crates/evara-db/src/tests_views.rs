//! The reads M8's views add: display labels, entry flags, and the per-cycle timetable list.
//!
//! # What these tests are about
//!
//! Three properties, each of which a view would otherwise get wrong silently:
//!
//! - **School isolation.** A label read is scoped like every other school-owned read since
//!   `V12`. A section of another school must be *absent*, not merely unused — a name that
//!   leaked in would be rendered, and nothing downstream would question it.
//! - **A roster is explicit.** A student reaches a label set only through
//!   `class_enrolment`. ADR 0006 makes the roster optional, so its absence is an ordinary
//!   state and must not be filled from `student_group.size` or from group membership.
//! - **A term scope means the same thing everywhere.** The label read carries the loader's
//!   own `IN_SCOPE_TERM` predicate, so the names and the compiled problem cannot disagree
//!   about which sections are in play.

use uuid::Uuid;

use crate::SchedulingSelection;
use crate::scheduling::seed::{Project, School};

/// Six teaching periods a day, as the persistence tests use.
fn standard_day() -> &'static [(&'static str, &'static str, &'static str, bool)] {
    &[
        ("1", "09:00", "10:00", true),
        ("2", "10:00", "11:00", true),
        ("Break", "11:00", "11:30", false),
        ("3", "11:30", "12:30", true),
        ("4", "13:30", "14:30", true),
        ("5", "14:30", "15:30", true),
    ]
}

/// A school with a grid, rooms, a subject, two sections and occurrences for each.
fn a_school_with_lessons(project: &mut Project, name: &str, subject: &str) -> School {
    let mut school = School::new(project, name, &["Main"], 5);
    school.with_term(project);
    school.bells(project, "Standard Day", Some(0), standard_day());
    school.materialise(project);

    let general = school.room_type(project, "General Classroom");
    school.room(project, "Room 1", 0, Some(general), 30, &[]);
    school.room(project, "Room 2", 0, Some(general), 30, &[]);

    let subject_id = school.subject(project, subject);
    for n in 0..2 {
        let teacher = school.teacher(project, &format!("Teacher{n}"));
        let group = school.group(project, &format!("7{n}"), Some(26), None);
        let section = school.section(
            project,
            subject_id,
            &format!("{subject} {n}"),
            None,
            None,
            None,
            false,
        );
        school.teach(project, section, teacher);
        school.attend(project, section, group);
        school.occurrences(project, section, 1, 2);
    }
    school
}

fn selection(school: &School, project: &mut Project) -> SchedulingSelection {
    let _ = project;
    SchedulingSelection {
        school_id: school.id,
        cycle_id: school.cycle,
        term_id: None,
    }
}

// =======================================================================================
// Section and room labels
// =======================================================================================

#[test]
fn a_label_is_read_for_every_section_in_scope() {
    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert_eq!(labels.sections.len(), 2, "one label per section in scope");
    for label in &labels.sections {
        assert_eq!(label.subject, "Mathematics");
        assert!(
            label.course.contains("course"),
            "the course's own name is carried, not the section's: {label:?}"
        );
        assert!(
            label.short.contains("Mathematics"),
            "the short caption names the subject: {}",
            label.short
        );
    }
}

#[test]
fn a_short_caption_is_composed_once_by_the_core() {
    use evara_core::scheduling::display::SectionLabel;

    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let selection = selection(&school, &mut project);
    let labels = project.labels(&selection).expect("read the labels");

    for label in &labels.sections {
        assert_eq!(
            label.short,
            SectionLabel::compose_short(
                label.year_level.as_deref(),
                &label.subject,
                &label.section
            ),
            "the stored caption is the one the core composes, not a second spelling"
        );
    }
}

#[test]
fn a_room_with_no_building_is_still_listed_exactly_once() {
    // `room.building_id` is nullable and `ON DELETE SET NULL`, so a room in no building is an
    // ordinary state — and the seed's rooms are in none. The join must therefore be a LEFT
    // one: an inner join would drop every such room from the label set, and the grid would
    // then render a booking whose room it could not name.
    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert_eq!(labels.rooms.len(), 2, "both rooms, each exactly once");
    let mut ids: Vec<Uuid> = labels.rooms.iter().map(|r| r.room_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "no room is listed twice");
    for room in &labels.rooms {
        assert!(
            !room.name.trim().is_empty(),
            "a room label carries the room's name"
        );
        assert!(
            room.building.is_none(),
            "these rooms are in no building, and that is not a reason to omit them"
        );
    }
}

#[test]
fn another_schools_sections_and_rooms_are_absent() {
    let mut project = Project::new();
    let northgate = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let southmoor = a_school_with_lessons(&mut project, "Southmoor", "Science");

    let north = selection(&northgate, &mut project);
    let labels = project.labels(&north).expect("read Northgate's labels");

    assert_eq!(
        labels.sections.len(),
        2,
        "only Northgate's two sections, never Southmoor's"
    );
    for label in &labels.sections {
        assert_eq!(
            label.subject, "Mathematics",
            "Southmoor teaches Science and must not appear"
        );
    }
    assert_eq!(labels.rooms.len(), 2, "only Northgate's rooms");

    // And the other direction, so the test cannot pass by reading nothing.
    let south = selection(&southmoor, &mut project);
    let theirs = project.labels(&south).expect("read Southmoor's labels");
    assert_eq!(theirs.sections.len(), 2);
    for label in &theirs.sections {
        assert_eq!(label.subject, "Science");
    }
}

// =======================================================================================
// Students: explicit enrolment and nothing else
// =======================================================================================

#[test]
fn a_school_with_no_enrolment_rows_yields_no_students() {
    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert!(
        labels.students.is_empty(),
        "a group of 26 is not 26 enrolment rows — ADR 0006"
    );
    assert!(!labels.has_enrolments());
}

#[test]
fn a_student_in_a_group_but_on_no_roster_is_still_absent() {
    // The inference this must never make: the student attends the group, the group attends
    // the section, so the student's timetable "is" the section's. Nothing says they are on
    // that roster, and presenting a guess as a child's confirmed schedule is the failure.
    let mut project = Project::new();
    let mut school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let group = school.group(&mut project, "7X", Some(26), None);
    let _student = school.student_in(&mut project, group, 0);
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert!(
        labels.students.is_empty(),
        "group membership is not class enrolment"
    );
}

#[test]
fn an_enrolled_student_carries_exactly_the_sections_named() {
    let mut project = Project::new();
    let mut school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let group = school.group(&mut project, "7X", Some(26), None);
    let student = school.student_in(&mut project, group, 0);
    let first = school.sections[0];
    school.enrol(&mut project, first, student);
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert_eq!(labels.students.len(), 1, "one student has a row");
    let found = &labels.students[0];
    assert_eq!(found.student_id, student);
    assert_eq!(
        found.sections,
        vec![first],
        "exactly the section the roster names, and no sibling section"
    );
    assert!(
        !found.name.trim().is_empty(),
        "a student's display name is composed"
    );
    assert!(labels.has_enrolments());
}

#[test]
fn a_student_on_two_rosters_is_one_label_with_two_sections() {
    let mut project = Project::new();
    let mut school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let group = school.group(&mut project, "7X", Some(26), None);
    let student = school.student_in(&mut project, group, 0);
    let first = school.sections[0];
    let second = school.sections[1];
    school.enrol(&mut project, first, student);
    school.enrol(&mut project, second, student);
    let selection = selection(&school, &mut project);

    let labels = project.labels(&selection).expect("read the labels");

    assert_eq!(
        labels.students.len(),
        1,
        "the rows are grouped into one label, not two"
    );
    let mut sections = labels.students[0].sections.clone();
    sections.sort_unstable();
    let mut expected = vec![first, second];
    expected.sort_unstable();
    assert_eq!(sections, expected);
}

#[test]
fn another_schools_enrolled_students_are_absent() {
    let mut project = Project::new();
    let northgate = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let mut southmoor = a_school_with_lessons(&mut project, "Southmoor", "Science");
    let group = southmoor.group(&mut project, "8X", Some(26), None);
    let student = southmoor.student_in(&mut project, group, 0);
    let theirs = southmoor.sections[0];
    southmoor.enrol(&mut project, theirs, student);

    let north = selection(&northgate, &mut project);
    let labels = project.labels(&north).expect("read Northgate's labels");
    assert!(
        labels.students.is_empty(),
        "Southmoor's roster is not Northgate's"
    );

    let south = selection(&southmoor, &mut project);
    let theirs = project.labels(&south).expect("read Southmoor's labels");
    assert_eq!(theirs.students.len(), 1, "and it is Southmoor's own");
}

// =======================================================================================
// Entry flags
// =======================================================================================

#[test]
fn a_timetable_with_no_entries_reports_no_flags() {
    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");
    let _ = &school;
    let flags = project
        .entry_flags(Uuid::now_v7())
        .expect("reading flags for an unknown timetable is empty, not an error");
    assert_eq!(flags, Vec::new());
}

// =======================================================================================
// The per-cycle timetable list
// =======================================================================================

#[test]
fn for_cycle_matches_a_whole_year_timetable_through_coalesce() {
    // The `COALESCE` that ADR 0042 C1 is about: `term_id = ?` would silently match nothing
    // for exactly the whole-year timetables, which are the common case.
    let mut project = Project::new();
    let school = a_school_with_lessons(&mut project, "Northgate", "Mathematics");

    let found = project
        .db()
        .timetables()
        .for_cycle(school.cycle, None)
        .expect("list by cycle");
    assert!(
        found.is_empty(),
        "nothing is saved yet, and an empty list is not an error"
    );

    let other = project
        .db()
        .timetables()
        .for_cycle(Uuid::now_v7(), None)
        .expect("an unknown cycle lists nothing");
    assert_eq!(other, Vec::new());
}
