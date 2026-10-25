//! Manual editing: place, move, unschedule, lock, duplicate — and every way one is refused.
//!
//! # The three guards, and why each needs its own test
//!
//! An editing write is refused by three independent predicates, and they catch different
//! things. Conflating them in a test would leave two of the three unproven:
//!
//! | Guard | Catches | Error |
//! | --- | --- | --- |
//! | `expectedRev` on the entry | somebody moved **this** placement | `Conflict` |
//! | the `ArrangementStamp` | somebody moved **another** placement of the same timetable | `ArrangementChanged` |
//! | the input watermark | a scheduling **input** moved — an availability rule, a duration | `InputsChanged` |
//!
//! The middle one is the one `expectedRev` cannot do and is the reason this module exists. A
//! second window that unschedules a *different* lesson leaves this session's target row at the
//! revision it was read at, so a revision check passes — and the verdict the user was shown
//! ("this cell is free") is now false. Nothing referential breaks, the timetable row's own
//! revision does not move, and `timetable_entry` is a solve **output** so the input watermark
//! does not move either.
//!
//! # And the two properties that must *not* become conflicts
//!
//! - **A sequence of edits does not trip its own freshness check.** `timetable_entry` is in
//!   `SOLVE_OUTPUT_TABLES`, so editing does not look like an input change. Asserted, because
//!   the opposite would make the second edit of every session fail.
//! - **An edit to another timetable is not this one's conflict.** The stamp is per-timetable.

use evara_constraints::{Registry, evaluate};
use evara_core::scheduling::{ProblemIdentity, Violation};
use evara_solver::{GenerateRequest, SolveConfig, generate};
use uuid::Uuid;

use crate::scheduling::seed::{Project, School};
use crate::{
    ArrangementStamp, DbError, EditGuard, EntryPlacement, KIND_GENERATION, RunProvenance,
    SavedTimetable, SchedulingSelection, TimetableEntry,
};

// =======================================================================================
// Scaffolding
// =======================================================================================

/// Six teaching periods a day, with a break in the middle.
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

/// A school with a grid, four rooms, four teachers, two groups and six occurrences.
///
/// The same shape `tests_timetables` uses, and deliberately so: a second builder would be a
/// second place for a fixture to drift from the schema. Two of the six are doubles, so every
/// test here exercises the multi-period case for free.
fn a_solvable_school(project: &mut Project) -> School {
    let mut school = School::new(project, "Northgate", &["Main"], 5);
    school.with_term(project);
    school.bells(project, "Standard Day", Some(0), standard_day());
    school.materialise(project);

    let general = school.room_type(project, "General Classroom");
    for n in 0..4 {
        school.room(project, &format!("Room {n}"), 0, Some(general), 30, &[]);
    }
    let teachers: Vec<Uuid> = (0..4)
        .map(|n| school.teacher(project, &format!("Teacher{n}")))
        .collect();
    let groups = [
        school.group(project, "7A", Some(26), None),
        school.group(project, "7B", Some(26), None),
    ];

    let subject = school.subject(project, "Mathematics");
    for (n, group) in groups.into_iter().enumerate() {
        let section = school.section(
            project,
            subject,
            &format!("Mathematics {n}"),
            None,
            None,
            None,
            false,
        );
        school.teach(project, section, teachers[n]);
        school.attend(project, section, group);
        school.occurrences(project, section, 1, 2);
        school.occurrences(project, section, 2, 1);
    }
    school
}

/// A second school in the same project, for the isolation tests.
fn a_second_school(project: &mut Project) -> School {
    let mut school = School::new(project, "Southmoor", &["Main"], 5);
    school.with_term(project);
    school.bells(project, "Standard Day", Some(0), standard_day());
    school.materialise(project);
    let general = school.room_type(project, "General Classroom");
    school.room(project, "Room A", 0, Some(general), 30, &[]);
    let teacher = school.teacher(project, "Teacher");
    let group = school.group(project, "8A", Some(26), None);
    let subject = school.subject(project, "Science");
    let section = school.section(project, subject, "Science 8A", None, None, None, false);
    school.teach(project, section, teacher);
    school.attend(project, section, group);
    school.occurrences(project, section, 1, 2);
    school
}

fn provenance() -> RunProvenance {
    RunProvenance {
        engine: "construction".to_owned(),
        settings: "{}".to_owned(),
        started_at: "2026-10-23T12:00:00.000Z".to_owned(),
        finished_at: "2026-10-23T12:00:01.000Z".to_owned(),
        kind: crate::RUN_GENERATION.to_owned(),
        scope: "{}".to_owned(),
    }
}

/// Generates and saves one timetable for `selection`.
fn saved(project: &mut Project, selection: &SchedulingSelection, name: &str) -> SavedTimetable {
    let assembly = project.load(selection).expect("load the problem");
    let rules = Registry::mandatory();
    let report = crate::AssemblyReport::of(&assembly);
    let outcome = generate(&GenerateRequest {
        problem: &assembly.problem,
        identity: ProblemIdentity {
            school: assembly.problem.context.school.id,
            cycle: selection.cycle_id,
            term: selection.term_id,
        },
        constraints: &rules,
        preflight: report.findings.clone(),
        may_solve: report.may_solve(),
        unenforced_restrictions: 0,
        config: SolveConfig::seeded(2026),
    });
    let violations: Vec<Violation> =
        evaluate(&assembly.problem, &outcome.assignment, &rules).violations;
    project
        .db()
        .timetables()
        .save_generated(&assembly, &outcome, &violations, &provenance(), name)
        .expect("the save succeeds")
}

/// An editing session over one saved timetable: what the guards are built from.
///
/// The two halves a real `EditSession` holds about the world outside itself, and nothing else:
/// these tests are about the writes, so the compiled problem and the undo stack are the command
/// layer's to test.
struct Session {
    watermark: crate::Watermark,
    stamp: ArrangementStamp,
}

impl Session {
    fn guard(&self) -> EditGuard<'_> {
        EditGuard {
            watermark: self.watermark,
            stamp: &self.stamp,
        }
    }
}

/// Opens an editing session: the assembly's watermark and the arrangement as it stands.
fn open(project: &mut Project, selection: &SchedulingSelection, timetable: Uuid) -> Session {
    let assembly = project.load(selection).expect("load the problem");
    let stamp = project
        .db()
        .timetables()
        .arrangement_stamp(timetable)
        .expect("read the stamp");
    let _ = timetable;
    Session {
        watermark: assembly.watermark,
        stamp,
    }
}

fn entries(project: &mut Project, timetable: Uuid) -> Vec<TimetableEntry> {
    project
        .db()
        .timetables()
        .entries(timetable)
        .expect("read the entries")
}

/// A teaching slot of this school's grid, on the second cycle day, that `entry` is not in.
///
/// Day 2 rather than any day, so the move is to a position the occurrence was not already on —
/// and a *teaching* slot, so the move is refused by a clash if at all and never by
/// `no-teaching-in-period`. Nothing here judges legality; these tests are about the writes.
fn another_slot(project: &mut Project, school: &School, entry: &TimetableEntry) -> Uuid {
    let day = school.days(project)[1];
    let slots = project
        .db()
        .time_model()
        .timeslots(school.cycle, school.term)
        .expect("read the slots");
    slots
        .iter()
        .find(|slot| slot.cycle_day_id == day && slot.is_teaching && slot.id != entry.timeslot_id)
        .expect("another teaching slot on the second day")
        .id
}

// =======================================================================================
// Place
// =======================================================================================

#[test]
fn placing_an_unplaced_occurrence_writes_one_row_marked_manual() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    // Take one off, then put it back by hand.
    let held = entries(&mut project, written.timetable.id);
    let victim = held.first().expect("something is placed").clone();
    let session = open(&mut project, &selection, written.timetable.id);
    let after = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule");

    let session = Session {
        watermark: session.watermark,
        stamp: after.stamp,
    };
    let applied = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            victim.activity_id,
            EntryPlacement {
                timeslot_id: victim.timeslot_id,
                room_id: victim.room_id,
            },
            session.guard(),
        )
        .expect("place it back");

    let entry = applied.entry.expect("a placement returns its row");
    assert_eq!(entry.activity_id, victim.activity_id);
    assert_eq!(entry.timeslot_id, victim.timeslot_id);
    assert!(entry.is_manual, "a human put it there");
    assert!(!entry.is_locked, "placing does not lock");
    assert_eq!(entry.rev, 1, "a creation starts at revision 1");
    assert_ne!(entry.id, victim.id, "and it is a new row");
    assert_eq!(
        applied.stamp.len(),
        held.len(),
        "the arrangement is back to its original size"
    );
}

#[test]
fn placing_an_occurrence_that_is_already_placed_is_refused() {
    // What a second click on the same cell receives, rather than two rows.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let held = entries(&mut project, written.timetable.id);
    let already = held.first().expect("something is placed").clone();
    let session = open(&mut project, &selection, written.timetable.id);

    let error = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            already.activity_id,
            EntryPlacement {
                timeslot_id: already.timeslot_id,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::DuplicateValue { .. }), "{error:?}");
}

#[test]
fn placing_another_schools_occurrence_is_refused() {
    // The chain is `activity -> class_section -> course -> subject.school_id`, and the
    // timetable's school is `cycle.school_id`. Neither carries a second column.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let selection = north.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let southern = saved(&mut project, &south.selection(), "Southmoor 1");
    let theirs = entries(&mut project, southern.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let ours = entries(&mut project, written.timetable.id);
    let slot = ours.first().expect("something is placed").timeslot_id;

    let session = open(&mut project, &selection, written.timetable.id);
    let error = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            theirs.activity_id,
            EntryPlacement {
                timeslot_id: slot,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::WrongSchool { .. }), "{error:?}");
}

#[test]
fn placing_into_another_cycles_slot_is_refused_with_a_remedy() {
    // The repository asks and names the remedy; `V17`'s trigger pair is the backstop that makes
    // the rule true for every writer. A bare `RAISE(ABORT)` would reach the interface as "this
    // campus may be damaged", which is the wrong sentence for a boundary working.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let selection = north.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let elsewhere = project
        .db()
        .time_model()
        .timeslots(south.cycle, south.term)
        .expect("read the slots")
        .first()
        .expect("the other cycle has slots")
        .id;

    let held = entries(&mut project, written.timetable.id);
    let victim = held.first().expect("something is placed").clone();
    let session = open(&mut project, &selection, written.timetable.id);
    let after = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule");
    let session = Session {
        watermark: session.watermark,
        stamp: after.stamp,
    };

    let error = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            victim.activity_id,
            EntryPlacement {
                timeslot_id: elsewhere,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
    assert_eq!(
        entries(&mut project, written.timetable.id).len(),
        held.len() - 1,
        "and nothing was written"
    );
}

// =======================================================================================
// Move
// =======================================================================================

#[test]
fn a_move_updates_the_row_rather_than_replacing_it() {
    // The property the whole module rests on. A delete-and-insert would mint a new `id`, reset
    // `rev`, write two change-log rows for one action and drop `is_locked` as a side effect.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let before = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &before);

    let session = open(&mut project, &selection, written.timetable.id);
    let applied = project
        .db()
        .timetables()
        .move_entry(
            before.id,
            before.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: before.room_id,
            },
            session.guard(),
        )
        .expect("the move succeeds");

    let after = applied.entry.expect("a move returns its row");
    assert_eq!(after.id, before.id, "the same row");
    assert_eq!(after.rev, before.rev + 1, "one revision, not a reset");
    assert_eq!(after.timeslot_id, target);
    assert!(after.is_manual, "a human moved it");
    assert_eq!(after.created_at, before.created_at, "it was not recreated");
    assert_eq!(
        entries(&mut project, written.timetable.id).len(),
        written.entries,
        "and the arrangement is the same size"
    );
}

#[test]
fn a_move_appends_exactly_one_change_log_entry() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let before = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &before);

    let mark = project
        .db()
        .read(crate::change_log::watermark)
        .expect("watermark");
    let session = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .move_entry(
            before.id,
            before.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: before.room_id,
            },
            session.guard(),
        )
        .expect("the move succeeds");

    let since: i64 = project
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM change_log WHERE seq > ?1",
                [mark.get()],
                |row| row.get(0),
            )?)
        })
        .expect("count");
    assert_eq!(since, 1, "one user action, one entry in the feed");
}

#[test]
fn moving_a_lesson_to_another_room_is_a_move() {
    // `room_id` is nullable and `None` means no room booked, which ADR 0034 §4 prefers to no
    // lesson. So clearing a room is an edit rather than a deletion.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let before = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.room_id.is_some())
        .expect("the solver books rooms");

    let session = open(&mut project, &selection, written.timetable.id);
    let applied = project
        .db()
        .timetables()
        .move_entry(
            before.id,
            before.rev,
            EntryPlacement {
                timeslot_id: before.timeslot_id,
                room_id: None,
            },
            session.guard(),
        )
        .expect("the move succeeds");
    let after = applied.entry.expect("a row");
    assert_eq!(after.room_id, None);
    assert_eq!(after.timeslot_id, before.timeslot_id);
}

#[test]
fn a_stale_revision_refuses_the_move_and_writes_nothing() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let before = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &before);

    let session = open(&mut project, &selection, written.timetable.id);
    let error = project
        .db()
        .timetables()
        .move_entry(
            before.id,
            before.rev + 7,
            EntryPlacement {
                timeslot_id: target,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");

    let unchanged = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.id == before.id)
        .expect("still there");
    assert_eq!(unchanged.timeslot_id, before.timeslot_id);
    assert_eq!(unchanged.rev, before.rev, "nothing was written");
}

// =======================================================================================
// Unschedule
// =======================================================================================

#[test]
fn unscheduling_removes_the_row_and_leaves_the_occurrence_unplaced() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let victim = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();

    let session = open(&mut project, &selection, written.timetable.id);
    let applied = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule");

    assert!(
        applied.entry.is_none(),
        "there is no row to return afterwards"
    );
    assert_eq!(applied.stamp.len(), written.entries - 1);
    assert!(
        !entries(&mut project, written.timetable.id)
            .iter()
            .any(|entry| entry.id == victim.id)
    );
}

#[test]
fn a_stale_revision_refuses_the_unschedule() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let victim = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();

    let session = open(&mut project, &selection, written.timetable.id);
    let error = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev + 1, session.guard())
        .expect_err("refused");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
    assert_eq!(
        entries(&mut project, written.timetable.id).len(),
        written.entries
    );
}

// =======================================================================================
// Locking
// =======================================================================================

#[test]
fn locking_sets_the_flag_without_claiming_a_human_placed_it() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let entry = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    assert!(!entry.is_locked && !entry.is_manual, "as generated");

    let session = open(&mut project, &selection, written.timetable.id);
    let applied = project
        .db()
        .timetables()
        .set_lock(entry.id, entry.rev, true, session.guard())
        .expect("lock it");
    let locked = applied.entry.expect("a row");
    assert!(locked.is_locked);
    assert!(
        !locked.is_manual,
        "locking where the solver put a lesson does not make it hand-placed"
    );
    assert_eq!(locked.rev, entry.rev + 1);
}

#[test]
fn unlocking_is_the_same_write_in_reverse() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let entry = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();

    let session = open(&mut project, &selection, written.timetable.id);
    let applied = project
        .db()
        .timetables()
        .set_lock(entry.id, entry.rev, true, session.guard())
        .expect("lock it");
    let locked = applied.entry.expect("a row");
    let session = Session {
        watermark: session.watermark,
        stamp: applied.stamp,
    };
    let applied = project
        .db()
        .timetables()
        .set_lock(locked.id, locked.rev, false, session.guard())
        .expect("unlock it");
    assert!(!applied.entry.expect("a row").is_locked);
}

#[test]
fn a_lock_becomes_a_pin_on_the_problem_and_nothing_else_does() {
    // The one route from a stored lock to the constraint engine, and it is the **caller's**:
    // the loader compiles inputs, and `timetable_entry` is an output. ADR 0045 §4.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let entry = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();

    let assembly = project.load(&selection).expect("load");
    let none = project
        .db()
        .timetables()
        .pins(written.timetable.id, &assembly)
        .expect("read the pins");
    assert!(
        none.pinned.is_empty() && none.locked == 0,
        "a generated timetable locks nothing"
    );
    assert!(
        assembly.problem.pinned.is_empty(),
        "and the loader compiles no pins"
    );

    let session = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .set_lock(entry.id, entry.rev, true, session.guard())
        .expect("lock it");

    let assembly = project.load(&selection).expect("load again");
    let pins = project
        .db()
        .timetables()
        .pins(written.timetable.id, &assembly)
        .expect("read the pins");
    assert_eq!(pins.locked, 1);
    assert_eq!(pins.pinned.len(), assembly.problem.activities.len());
    assert_eq!(
        pins.pinned.iter().filter(|p| p.is_some()).count(),
        1,
        "one pin, for the one lock"
    );
    assert!(
        assembly.problem.pinned.is_empty(),
        "the loader still compiles none of it"
    );

    // And the pinned problem is a sound one, which is what `activity-fixed` needs.
    let mut pinned = assembly.problem.clone();
    pinned.pinned = pins.pinned;
    let broken = pinned.integrity();
    assert_eq!(broken.len(), 0, "{broken:?}");
}

// =======================================================================================
// The arrangement guard — what `expectedRev` cannot do
// =======================================================================================

#[test]
fn a_second_session_moving_another_entry_refuses_the_first() {
    // The whole reason `ArrangementStamp` exists. Both sessions hold a correct `expectedRev`
    // for their own target, and the first session's verdicts are stale anyway.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let held = entries(&mut project, written.timetable.id);
    let mine = held[0].clone();
    let theirs = held[1].clone();
    assert_ne!(mine.id, theirs.id, "two different entries");

    // Both windows open at the same arrangement.
    let first = open(&mut project, &selection, written.timetable.id);
    let second = open(&mut project, &selection, written.timetable.id);

    // The second window unschedules a lesson the first has no interest in.
    project
        .db()
        .timetables()
        .unschedule(theirs.id, theirs.rev, second.guard())
        .expect("the second session writes");

    // The first window's own target is untouched and its revision still matches — and it is
    // refused, because what it was shown is no longer true.
    let target = another_slot(&mut project, &school, &mine);
    let error = project
        .db()
        .timetables()
        .move_entry(
            mine.id,
            mine.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: None,
            },
            first.guard(),
        )
        .expect_err("refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { entries_changed: 1 }),
        "{error:?}"
    );

    let unchanged = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.id == mine.id)
        .expect("still there");
    assert_eq!(unchanged.rev, mine.rev, "and nothing was written");
}

#[test]
fn a_second_session_placing_a_lesson_refuses_the_first() {
    // The added-entry direction: the stamp must notice a row it has never seen, not only a
    // revision that moved.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let held = entries(&mut project, written.timetable.id);
    let victim = held[0].clone();
    let other = held[1].clone();

    let first = open(&mut project, &selection, written.timetable.id);
    let second = open(&mut project, &selection, written.timetable.id);

    // The second window takes one off and puts it back, which replaces the row.
    let after = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, second.guard())
        .expect("unschedule");
    let second = Session {
        watermark: second.watermark,
        stamp: after.stamp,
    };
    project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            victim.activity_id,
            EntryPlacement {
                timeslot_id: victim.timeslot_id,
                room_id: victim.room_id,
            },
            second.guard(),
        )
        .expect("place it back");

    // The arrangement is the same size and is **not** the same arrangement.
    let error = project
        .db()
        .timetables()
        .set_lock(other.id, other.rev, true, first.guard())
        .expect_err("refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { .. }),
        "{error:?}"
    );
}

#[test]
fn an_edit_to_another_timetable_is_not_this_ones_conflict() {
    // The stamp is per-timetable, so two users editing two versions of one cycle do not block
    // each other.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let first_version = saved(&mut project, &selection, "Version 1");
    let second_version = saved(&mut project, &selection, "Version 2");

    let mine = entries(&mut project, first_version.timetable.id)[0].clone();
    let theirs = entries(&mut project, second_version.timetable.id)[0].clone();

    let session = open(&mut project, &selection, first_version.timetable.id);
    let other = open(&mut project, &selection, second_version.timetable.id);

    project
        .db()
        .timetables()
        .unschedule(theirs.id, theirs.rev, other.guard())
        .expect("the other version is edited");

    let target = another_slot(&mut project, &school, &mine);
    project
        .db()
        .timetables()
        .move_entry(
            mine.id,
            mine.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: None,
            },
            session.guard(),
        )
        .expect("and this one is still editable");
}

#[test]
fn a_sequence_of_edits_does_not_trip_its_own_freshness_check() {
    // `timetable_entry` is in `SOLVE_OUTPUT_TABLES`, so editing does not look like an input
    // change. The opposite would make the second edit of every session fail.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let held = entries(&mut project, written.timetable.id);

    let mut session = open(&mut project, &selection, written.timetable.id);
    for entry in held.iter().take(3) {
        let target = another_slot(&mut project, &school, entry);
        let applied = project
            .db()
            .timetables()
            .move_entry(
                entry.id,
                entry.rev,
                EntryPlacement {
                    timeslot_id: target,
                    room_id: entry.room_id,
                },
                session.guard(),
            )
            .expect("each edit in turn, with the stamp the last one returned");
        session = Session {
            watermark: session.watermark,
            stamp: applied.stamp,
        };
    }
}

#[test]
fn a_moved_scheduling_input_refuses_every_edit() {
    // M7's guarantee, preserved. An availability rule that changed after the session opened
    // makes the verdict the user saw untrue, and nothing referential would catch it.
    let mut project = Project::new();
    let mut school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let entry = entries(&mut project, written.timetable.id)[0].clone();
    let session = open(&mut project, &selection, written.timetable.id);

    // An input moves: a teacher becomes unavailable on a day. Nothing referential breaks, which
    // is the whole reason the watermark exists.
    let day = school.days(&mut project)[0];
    let teacher = school.teachers[0];
    school.unavailable_on_day(
        &mut project,
        crate::repo::availability::AvailabilityEntity::Teacher,
        teacher,
        day,
        crate::repo::availability::AvailabilityStatus::Unavailable,
    );

    let target = another_slot(&mut project, &school, &entry);
    let error = project
        .db()
        .timetables()
        .move_entry(
            entry.id,
            entry.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::InputsChanged { .. }), "{error:?}");
}

// =======================================================================================
// Duplicate
// =======================================================================================

#[test]
fn duplicating_copies_the_arrangement_and_claims_no_generation() {
    // The affordance that makes in-place editing safe. The copy must not claim a run or an
    // evaluation that did not produce it.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let copy = project
        .db()
        .timetables()
        .duplicate(written.timetable.id, written.timetable.rev, "Before I edit")
        .expect("duplicate");

    assert_eq!(copy.entries, written.entries);
    assert_ne!(copy.timetable.id, written.timetable.id);
    assert_eq!(copy.timetable.name, "Before I edit");
    assert_eq!(copy.timetable.cycle_id, written.timetable.cycle_id);
    assert_eq!(copy.timetable.term_id, written.timetable.term_id);

    assert!(
        project
            .db()
            .timetables()
            .run_for(copy.timetable.id)
            .expect("read")
            .is_none(),
        "a copy was not generated, so it names no run"
    );
    assert!(
        project
            .db()
            .timetables()
            .evaluations(copy.timetable.id)
            .expect("read")
            .is_empty(),
        "and carries no GENERATION scoring"
    );

    // The original is untouched, provenance included.
    assert!(
        project
            .db()
            .timetables()
            .run_for(written.timetable.id)
            .expect("read")
            .is_some()
    );
    let originals = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("read");
    assert!(originals.iter().any(|e| e.kind == KIND_GENERATION));
}

#[test]
fn a_duplicate_preserves_the_placements_and_their_flags() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let entry = entries(&mut project, written.timetable.id)[0].clone();

    let session = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .set_lock(entry.id, entry.rev, true, session.guard())
        .expect("lock one");

    let timetable = project
        .db()
        .timetables()
        .get(written.timetable.id)
        .expect("re-read");
    let copy = project
        .db()
        .timetables()
        .duplicate(timetable.id, timetable.rev, "A copy")
        .expect("duplicate");

    let copied = entries(&mut project, copy.timetable.id);
    let original = entries(&mut project, written.timetable.id);
    assert_eq!(copied.len(), original.len());
    assert_eq!(
        copied.iter().filter(|e| e.is_locked).count(),
        1,
        "a lock describes a placement, which is what was copied"
    );
    for (one, other) in copied.iter().zip(original.iter()) {
        assert_eq!(one.activity_id, other.activity_id);
        assert_eq!(one.timeslot_id, other.timeslot_id);
        assert_eq!(one.room_id, other.room_id);
        assert_ne!(one.id, other.id, "fresh identifiers");
        assert_eq!(one.rev, 1);
    }
}

#[test]
fn a_duplicate_under_a_stale_revision_is_refused() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let error = project
        .db()
        .timetables()
        .duplicate(written.timetable.id, written.timetable.rev + 3, "A copy")
        .expect_err("refused");
    assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");
    assert_eq!(
        project
            .db()
            .timetables()
            .for_cycle(selection.cycle_id, selection.term_id)
            .expect("read")
            .len(),
        1,
        "and nothing was written"
    );
}

#[test]
fn a_duplicate_cannot_take_a_name_the_scope_already_holds() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let error = project
        .db()
        .timetables()
        .duplicate(written.timetable.id, written.timetable.rev, "Version 1")
        .expect_err("refused");
    assert!(matches!(error, DbError::DuplicateValue { .. }), "{error:?}");
}

// =======================================================================================
// The stamp itself
// =======================================================================================

#[test]
fn a_stamp_of_the_same_arrangement_has_no_differences() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let one = project
        .db()
        .timetables()
        .arrangement_stamp(written.timetable.id)
        .expect("read");
    let again = project
        .db()
        .timetables()
        .arrangement_stamp(written.timetable.id)
        .expect("read");
    assert_eq!(one.differences(&again), 0);
    assert_eq!(one.len(), written.entries);
    assert!(!one.is_empty());
}

#[test]
fn an_unschedule_and_a_place_of_the_same_occurrence_is_a_difference() {
    // The collision a count-and-sum digest would miss: one row removed at `rev = 1` and one
    // added at `rev = 1` leaves both the count and the total unchanged.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");
    let victim = entries(&mut project, written.timetable.id)[0].clone();

    let before = project
        .db()
        .timetables()
        .arrangement_stamp(written.timetable.id)
        .expect("read");

    let session = open(&mut project, &selection, written.timetable.id);
    let after = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule");
    let session = Session {
        watermark: session.watermark,
        stamp: after.stamp,
    };
    let applied = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            victim.activity_id,
            EntryPlacement {
                timeslot_id: victim.timeslot_id,
                room_id: victim.room_id,
            },
            session.guard(),
        )
        .expect("place it back");

    assert_eq!(applied.stamp.len(), before.len(), "the same size");
    assert_eq!(
        before.differences(&applied.stamp),
        2,
        "one row gone and one row new, which a digest on count and total would miss"
    );
}

#[test]
fn an_empty_timetable_has_an_empty_stamp() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let mut session = open(&mut project, &selection, written.timetable.id);
    for entry in entries(&mut project, written.timetable.id) {
        let applied = project
            .db()
            .timetables()
            .unschedule(entry.id, entry.rev, session.guard())
            .expect("unschedule");
        session = Session {
            watermark: session.watermark,
            stamp: applied.stamp,
        };
    }
    assert!(session.stamp.is_empty());
    assert_eq!(session.stamp.len(), 0);
}

// =======================================================================================
// Audit — cross-session consistency
// =======================================================================================

/// Counts the change-feed entries written since a mark.
fn appended_since(project: &mut Project, mark: crate::change_log::Watermark) -> i64 {
    project
        .db()
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM change_log WHERE seq > ?1",
                [mark.get()],
                |row| row.get(0),
            )?)
        })
        .expect("count the feed")
}

fn mark(project: &mut Project) -> crate::change_log::Watermark {
    project
        .db()
        .read(crate::change_log::watermark)
        .expect("watermark")
}

#[test]
fn a_lock_by_another_session_refuses_this_one() {
    // The audit's question: does *locking* a lesson invalidate a session that was looking at it?
    // It must. A lock is an `UPDATE`, so it bumps the entry's `rev`, so the stamp moves — and
    // every drop target the other session has on screen was computed on the assumption that this
    // lesson could be moved. Nothing about this is implied by the move and place cases: a lock
    // changes no placement, and a guard that compared only `(id, timeslot, room)` would miss it.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let held = entries(&mut project, written.timetable.id);
    let mine = held.first().expect("something is placed").clone();
    let theirs = held.get(1).expect("a second placement").clone();

    // Both sessions read the same arrangement.
    let first = open(&mut project, &selection, written.timetable.id);
    let second = open(&mut project, &selection, written.timetable.id);

    project
        .db()
        .timetables()
        .set_lock(theirs.id, theirs.rev, true, second.guard())
        .expect("the second session locks another lesson");

    let target = another_slot(&mut project, &school, &mine);
    let error = project
        .db()
        .timetables()
        .move_entry(
            mine.id,
            mine.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: mine.room_id,
            },
            first.guard(),
        )
        .expect_err("the first session is refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { entries_changed: 1 }),
        "{error:?}"
    );
}

#[test]
fn an_unlock_by_another_session_refuses_this_one_too() {
    // The mirror, and not redundant: unlocking is the write that *grants* a freedom, so a session
    // that missed it would refuse a move the user had just made legal — the opposite error and
    // the more confusing one.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let held = entries(&mut project, written.timetable.id);
    let mine = held.first().expect("something is placed").clone();
    let theirs = held.get(1).expect("a second placement").clone();

    let setup = open(&mut project, &selection, written.timetable.id);
    let locked = project
        .db()
        .timetables()
        .set_lock(theirs.id, theirs.rev, true, setup.guard())
        .expect("locked");

    // Both sessions now read the arrangement *with* the lock in it.
    let first = open(&mut project, &selection, written.timetable.id);
    let second = open(&mut project, &selection, written.timetable.id);
    let _ = locked;

    let their_now = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.id == theirs.id)
        .expect("still there");
    project
        .db()
        .timetables()
        .set_lock(their_now.id, their_now.rev, false, second.guard())
        .expect("the second session unlocks it");

    let target = another_slot(&mut project, &school, &mine);
    let error = project
        .db()
        .timetables()
        .move_entry(
            mine.id,
            mine.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: mine.room_id,
            },
            first.guard(),
        )
        .expect_err("the first session is refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { .. }),
        "{error:?}"
    );
}

#[test]
fn a_refused_edit_appends_nothing_to_the_change_feed() {
    // What "writes nothing" has to mean. A refusal that consumed a revision, or appended an
    // entry to the feed, would be a write — and the feed is what a future synchronisation
    // replays, so a phantom entry there is worse than a phantom row.
    //
    // Each of the three guards is refused separately, because they fail at different points in
    // the transaction and only one of them is reached before any statement runs.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let held = entries(&mut project, written.timetable.id);
    let mine = held.first().expect("something is placed").clone();
    let theirs = held.get(1).expect("a second placement").clone();
    let target = another_slot(&mut project, &school, &mine);
    let to = EntryPlacement {
        timeslot_id: target,
        room_id: mine.room_id,
    };

    // 1 — a stale revision on this entry.
    let session = open(&mut project, &selection, written.timetable.id);
    let before = mark(&mut project);
    let _ = project
        .db()
        .timetables()
        .move_entry(mine.id, mine.rev + 9, to, session.guard())
        .expect_err("refused on the revision");
    assert_eq!(
        appended_since(&mut project, before),
        0,
        "a conflict wrote nothing"
    );

    // 2 — another session moved a different entry.
    let stale = open(&mut project, &selection, written.timetable.id);
    let other = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .set_lock(theirs.id, theirs.rev, true, other.guard())
        .expect("their write lands");
    let before = mark(&mut project);
    let _ = project
        .db()
        .timetables()
        .move_entry(mine.id, mine.rev, to, stale.guard())
        .expect_err("refused on the stamp");
    assert_eq!(
        appended_since(&mut project, before),
        0,
        "an arrangement change wrote nothing"
    );

    // 3 — the whole arrangement is still exactly as it was, bar the one write that succeeded.
    let after = entries(&mut project, written.timetable.id);
    assert_eq!(after.len(), held.len());
    let still = after
        .iter()
        .find(|entry| entry.id == mine.id)
        .expect("still there");
    assert_eq!(still.timeslot_id, mine.timeslot_id, "and nothing moved");
    assert_eq!(still.rev, mine.rev, "and no revision was consumed");
}

#[test]
fn a_refusal_after_the_guard_rolls_the_whole_transaction_back() {
    // The guard is checked **inside** the write transaction, which is the thing this asserts
    // rather than reads off the source. `place` runs `guard_edit`, then the two ownership
    // checks, then the insert; so a failure in the *third* step has to undo the first two. If
    // the guard ran outside the transaction the row would survive, and if the insert were not in
    // the same transaction as the guard the timetable would hold a placement nothing checked.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let selection = north.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let elsewhere = project
        .db()
        .time_model()
        .timeslots(south.cycle, south.term)
        .expect("read the slots")
        .first()
        .expect("the other cycle has slots")
        .id;

    let held = entries(&mut project, written.timetable.id);
    let victim = held.first().expect("something is placed").clone();
    let session = open(&mut project, &selection, written.timetable.id);
    let after = project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule");
    let session = Session {
        watermark: session.watermark,
        stamp: after.stamp,
    };

    let before = mark(&mut project);
    let error = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            victim.activity_id,
            EntryPlacement {
                timeslot_id: elsewhere,
                room_id: None,
            },
            session.guard(),
        )
        .expect_err("refused");
    assert!(matches!(error, DbError::Invalid { .. }), "{error:?}");
    assert_eq!(
        appended_since(&mut project, before),
        0,
        "the guard passed, the insert was refused, and the transaction left no trace"
    );
    assert!(
        !entries(&mut project, written.timetable.id)
            .iter()
            .any(|entry| entry.activity_id == victim.activity_id),
        "and the occurrence is still unplaced"
    );
}

#[test]
fn an_edit_to_another_schools_timetable_is_not_this_ones_conflict() {
    // School isolation at the *stamp* level, which is a different question from the write-time
    // `WrongSchool` checks. Two schools' timetables share nothing, so a busy session at Southmoor
    // must not make Northgate's editor refuse — otherwise a project holding two schools would be
    // single-user by accident.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let northgate = north.selection();
    let mine = saved(&mut project, &northgate, "Northgate v1");

    let southmoor = south.selection();
    let theirs = saved(&mut project, &southmoor, "Southmoor v1");

    let session = open(&mut project, &northgate, mine.timetable.id);

    // Southmoor edits its own timetable.
    let their_session = open(&mut project, &southmoor, theirs.timetable.id);
    let their_entry = entries(&mut project, theirs.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    project
        .db()
        .timetables()
        .set_lock(their_entry.id, their_entry.rev, true, their_session.guard())
        .expect("their write lands");

    // Northgate's session is untouched.
    let entry = entries(&mut project, mine.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &north, &entry);
    project
        .db()
        .timetables()
        .move_entry(
            entry.id,
            entry.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: entry.room_id,
            },
            session.guard(),
        )
        .expect("the other school's edit is none of this session's business");
}

// =======================================================================================
// Audit — undo and redo, through the database
// =======================================================================================
//
// The session's own tests prove `Command::inverse` is an involution. These prove the *writes*
// round-trip: that applying an inverse through the repository restores the row, its flags and
// its identity, and that a sequence of them does not drift. Nothing below asserts anything about
// the stack, which is the command layer's; what is asserted is that the inverse of each write
// is the write the stack hands back.

#[test]
fn an_unlock_a_move_and_two_undos_restore_the_lock_and_the_slot() {
    // The brief's sequence, and the one that catches an inverse applied in the wrong order. A
    // locked lesson has to be unlocked before it can be moved, so undoing has to re-lock *after*
    // moving back — the reverse order, which is what a stack gives for free and what a
    // hand-written "undo everything" would get wrong.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let origin = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &origin);
    let home = EntryPlacement {
        timeslot_id: origin.timeslot_id,
        room_id: origin.room_id,
    };
    let away = EntryPlacement {
        timeslot_id: target,
        room_id: origin.room_id,
    };

    let session = open(&mut project, &selection, written.timetable.id);
    let locked = project
        .db()
        .timetables()
        .set_lock(origin.id, origin.rev, true, session.guard())
        .expect("lock it first, as a user would");
    let mut session = Session {
        watermark: session.watermark,
        stamp: locked.stamp,
    };
    let at_lock = locked.entry.expect("a lock returns the row");

    // Unlock → move.
    let unlocked = project
        .db()
        .timetables()
        .set_lock(at_lock.id, at_lock.rev, false, session.guard())
        .expect("unlock");
    session.stamp = unlocked.stamp;
    let at_unlock = unlocked.entry.expect("row");

    let moved = project
        .db()
        .timetables()
        .move_entry(at_unlock.id, at_unlock.rev, away, session.guard())
        .expect("move");
    session.stamp = moved.stamp;
    let at_move = moved.entry.expect("row");
    assert_eq!(at_move.timeslot_id, target);
    assert!(!at_move.is_locked);

    // Undo the move, then undo the unlock. Each precondition is the revision the *forward* write
    // produced, which is what the stack records.
    let back = project
        .db()
        .timetables()
        .move_entry(at_move.id, at_move.rev, home, session.guard())
        .expect("undo the move");
    session.stamp = back.stamp;
    let at_back = back.entry.expect("row");

    let relocked = project
        .db()
        .timetables()
        .set_lock(at_back.id, at_back.rev, true, session.guard())
        .expect("undo the unlock");
    let finished = relocked.entry.expect("row");

    assert_eq!(finished.id, origin.id, "one row throughout, never replaced");
    assert_eq!(
        finished.timeslot_id, origin.timeslot_id,
        "back where it was"
    );
    assert_eq!(finished.room_id, origin.room_id);
    assert!(finished.is_locked, "and locked again");

    // **`is_manual` does not come back, and should not.** `set_lock` never writes it — locking a
    // generated lesson does not make a human have placed it — but `move_entry` does, and the two
    // moves in this sequence are the user putting the lesson where it now is. Undoing a move is
    // *also* a person deciding where the lesson goes, so the flag is a high-water mark rather
    // than a position: once a human has moved a placement, the record that a human was involved
    // is true forever, and resetting it on an undo would make the grid claim the solver chose a
    // cell a person chose.
    //
    // Asserted here because the first version of this test assumed the opposite and failed,
    // which is how the question got asked at all.
    assert!(
        !origin.is_manual,
        "a generated placement starts out the solver's"
    );
    assert!(
        finished.is_manual,
        "and stays the user's once they have moved it, undo included"
    );
    assert_eq!(
        finished.rev,
        origin.rev + 5,
        "five writes, five revisions — an undo is a new transaction, not a rollback"
    );
}

#[test]
fn a_move_undone_and_redone_returns_to_where_it_went() {
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let origin = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &origin);
    let home = EntryPlacement {
        timeslot_id: origin.timeslot_id,
        room_id: origin.room_id,
    };
    let away = EntryPlacement {
        timeslot_id: target,
        room_id: origin.room_id,
    };

    let session = open(&mut project, &selection, written.timetable.id);
    let mut session = Session {
        watermark: session.watermark,
        stamp: session.stamp,
    };

    let moved = project
        .db()
        .timetables()
        .move_entry(origin.id, origin.rev, away, session.guard())
        .expect("move");
    session.stamp = moved.stamp;
    let at_move = moved.entry.expect("row");

    let undone = project
        .db()
        .timetables()
        .move_entry(at_move.id, at_move.rev, home, session.guard())
        .expect("undo");
    session.stamp = undone.stamp;
    let at_undo = undone.entry.expect("row");
    assert_eq!(at_undo.timeslot_id, origin.timeslot_id);

    let redone = project
        .db()
        .timetables()
        .move_entry(at_undo.id, at_undo.rev, away, session.guard())
        .expect("redo");
    let at_redo = redone.entry.expect("row");
    assert_eq!(at_redo.timeslot_id, target, "redo goes where the move went");
    assert_eq!(at_redo.id, origin.id);
    assert_eq!(at_redo.rev, origin.rev + 3);
}

#[test]
fn an_unschedule_undone_and_redone_keeps_one_placement_and_a_new_row() {
    // The asymmetric case. An unschedule deletes the row, so its inverse is an **insert** — a
    // new `id` and `rev = 1`, which is correct and is the one place in this module where entry
    // identity legitimately changes. What must hold is that the *placement* comes back exactly,
    // and that `is_manual` becomes true: a human put it back.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let origin = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let home = EntryPlacement {
        timeslot_id: origin.timeslot_id,
        room_id: origin.room_id,
    };

    let session = open(&mut project, &selection, written.timetable.id);
    let mut session = Session {
        watermark: session.watermark,
        stamp: session.stamp,
    };

    let removed = project
        .db()
        .timetables()
        .unschedule(origin.id, origin.rev, session.guard())
        .expect("unschedule");
    session.stamp = removed.stamp;
    assert!(removed.entry.is_none(), "there is no row to return");

    let back = project
        .db()
        .timetables()
        .place(
            written.timetable.id,
            origin.activity_id,
            home,
            session.guard(),
        )
        .expect("undo the unschedule");
    session.stamp = back.stamp;
    let restored = back.entry.expect("a place returns the row");
    assert_eq!(restored.timeslot_id, origin.timeslot_id);
    assert_eq!(restored.room_id, origin.room_id);
    assert_ne!(
        restored.id, origin.id,
        "a new row, because the old one was deleted"
    );
    assert_eq!(restored.rev, 1);
    assert!(restored.is_manual, "a human put it back");
    assert!(
        !restored.is_locked,
        "and a restored placement is not locked by implication"
    );

    // Redo: unschedule it again, on the revision the undo produced.
    let again = project
        .db()
        .timetables()
        .unschedule(restored.id, restored.rev, session.guard())
        .expect("redo the unschedule");
    assert!(again.entry.is_none());
    assert!(
        !entries(&mut project, written.timetable.id)
            .iter()
            .any(|entry| entry.activity_id == origin.activity_id),
        "unplaced again"
    );
}

#[test]
fn a_multi_period_move_with_a_room_change_is_one_row_and_inverts() {
    // Two of the fixture's six occurrences are doubles, so this is the real multi-period case
    // rather than a constructed one. ADR 0042: one row carries the **start** slot and the later
    // slots are derived, so a two-period move is one `UPDATE` of one row — and a room change in
    // the same move must not split into two writes, or an undo would restore half of it.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    // The compiled problem is where a duration lives, so that is where the double is found —
    // `timetable_entry` deliberately carries only the start slot (ADR 0042).
    let assembly = project.load(&selection).expect("compile the problem");
    let double = assembly
        .problem
        .activities
        .iter()
        .find(|activity| activity.duration.get() > 1)
        .expect("the fixture has two doubles")
        .clone();
    let periods = double.duration.get();

    let origin = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.activity_id == double.id)
        .expect("the double is placed");

    // A different room this school holds, so the move changes both halves of the placement.
    let elsewhere = *school
        .rooms
        .iter()
        .find(|room| Some(**room) != origin.room_id)
        .expect("a second room");

    let target = another_slot(&mut project, &school, &origin);
    let home = EntryPlacement {
        timeslot_id: origin.timeslot_id,
        room_id: origin.room_id,
    };
    let away = EntryPlacement {
        timeslot_id: target,
        room_id: Some(elsewhere),
    };

    let session = open(&mut project, &selection, written.timetable.id);
    let mut session = Session {
        watermark: session.watermark,
        stamp: session.stamp,
    };
    let before = mark(&mut project);

    let moved = project
        .db()
        .timetables()
        .move_entry(origin.id, origin.rev, away, session.guard())
        .expect("move the double and rebook it");
    session.stamp = moved.stamp;
    let at_move = moved.entry.expect("row");
    assert_eq!(
        at_move.id, origin.id,
        "one row for a run of {periods} periods, carrying the start slot only"
    );
    assert_eq!(at_move.timeslot_id, target);
    assert_eq!(at_move.room_id, Some(elsewhere));
    assert_eq!(
        appended_since(&mut project, before),
        1,
        "slot and room together are one user action and one feed entry"
    );

    let back = project
        .db()
        .timetables()
        .move_entry(at_move.id, at_move.rev, home, session.guard())
        .expect("undo");
    let at_back = back.entry.expect("row");
    assert_eq!(
        at_back.timeslot_id, origin.timeslot_id,
        "both halves restored"
    );
    assert_eq!(at_back.room_id, origin.room_id);
    assert_eq!(at_back.id, origin.id, "and no row was minted along the way");
    assert_eq!(
        at_back.rev,
        origin.rev + 2,
        "two writes, and the second is a real transaction rather than a rollback"
    );
}

#[test]
fn an_undo_is_refused_when_another_session_changed_the_arrangement() {
    // The case the stack cannot reason about on its own. The inverse's precondition is the
    // revision this session's forward write produced, and that revision is *still current* — the
    // other session touched a different row. So only the stamp catches it, and it must, because
    // the inverse was computed against an arrangement that has moved.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let held = entries(&mut project, written.timetable.id);
    let mine = held.first().expect("something is placed").clone();
    let theirs = held.get(1).expect("a second placement").clone();
    let target = another_slot(&mut project, &school, &mine);
    let home = EntryPlacement {
        timeslot_id: mine.timeslot_id,
        room_id: mine.room_id,
    };
    let away = EntryPlacement {
        timeslot_id: target,
        room_id: mine.room_id,
    };

    let session = open(&mut project, &selection, written.timetable.id);
    let mut session = Session {
        watermark: session.watermark,
        stamp: session.stamp,
    };
    let moved = project
        .db()
        .timetables()
        .move_entry(mine.id, mine.rev, away, session.guard())
        .expect("move");
    session.stamp = moved.stamp;
    let at_move = moved.entry.expect("row");

    // Another window locks a different lesson.
    let other = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .set_lock(theirs.id, theirs.rev, true, other.guard())
        .expect("their write lands");

    let error = project
        .db()
        .timetables()
        .move_entry(at_move.id, at_move.rev, home, session.guard())
        .expect_err("the undo is refused");
    assert!(
        matches!(error, DbError::ArrangementChanged { .. }),
        "not a revision conflict — the revision is current. {error:?}"
    );

    let unchanged = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.id == at_move.id)
        .expect("still there");
    assert_eq!(
        unchanged.timeslot_id, target,
        "a refused undo leaves the forward edit standing"
    );
    assert_eq!(unchanged.rev, at_move.rev, "and consumes no revision");
}

// =======================================================================================
// Audit — historical provenance
// =======================================================================================

#[test]
fn editing_leaves_the_generation_record_untouched() {
    // ADR 0042 §5's guarantee, under the thing it exists for. The `GENERATION` scoring describes
    // the arrangement the solver produced; the arrangement beneath it can now change, which is
    // exactly why that record is immutable. If an edit rewrote it, a school could never find out
    // how the timetable it is running was actually generated.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    let before = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("read the scorings");
    let generation = before
        .iter()
        .find(|scoring| scoring.kind == KIND_GENERATION)
        .expect("a saved timetable has one")
        .clone();

    let origin = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let target = another_slot(&mut project, &school, &origin);
    let session = open(&mut project, &selection, written.timetable.id);
    let mut session = Session {
        watermark: session.watermark,
        stamp: session.stamp,
    };
    let moved = project
        .db()
        .timetables()
        .move_entry(
            origin.id,
            origin.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: origin.room_id,
            },
            session.guard(),
        )
        .expect("move it");
    session.stamp = moved.stamp;
    let at_move = moved.entry.expect("row");

    // And unschedule something, which changes `placed` for any honest re-score.
    let victim = entries(&mut project, written.timetable.id)
        .into_iter()
        .find(|entry| entry.id != at_move.id)
        .expect("a second placement");
    project
        .db()
        .timetables()
        .unschedule(victim.id, victim.rev, session.guard())
        .expect("unschedule it");

    let after = project
        .db()
        .timetables()
        .evaluations(written.timetable.id)
        .expect("read the scorings again");
    let still = after
        .iter()
        .find(|scoring| scoring.kind == KIND_GENERATION)
        .expect("still there");
    assert_eq!(still.id, generation.id, "the same row");
    assert_eq!(still.rev, generation.rev, "and no revision was consumed");
    assert_eq!(still.score_hard, generation.score_hard);
    assert_eq!(still.score_medium, generation.score_medium);
    assert_eq!(still.score_soft, generation.score_soft);
    assert_eq!(
        still.placed_count, generation.placed_count,
        "and it still says what was generated, not what is placed now"
    );
    assert_eq!(still.required_count, generation.required_count);
    assert_eq!(still.evaluated_at, generation.evaluated_at);
    assert_eq!(still.solve_run_id, generation.solve_run_id);
    assert_eq!(
        after.len(),
        before.len(),
        "editing adds no scoring either — a re-score is an explicit act"
    );
}

#[test]
fn a_duplicate_and_its_source_are_edited_independently() {
    // The affordance is only worth offering if it actually isolates. Two timetables, one
    // arrangement each, and an edit to one must leave the other's rows, revisions and flags
    // exactly as they were — including the `is_locked` flags the copy inherited.
    let mut project = Project::new();
    let school = a_solvable_school(&mut project);
    let selection = school.selection();
    let written = saved(&mut project, &selection, "Version 1");

    // Lock something before copying, so the copy inherits a pin.
    let first = entries(&mut project, written.timetable.id)
        .first()
        .expect("something is placed")
        .clone();
    let setup = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .set_lock(first.id, first.rev, true, setup.guard())
        .expect("lock");

    let source = project
        .db()
        .timetables()
        .for_school(school.id)
        .expect("list the school's timetables")
        .into_iter()
        .find(|timetable| timetable.id == written.timetable.id)
        .expect("still there");
    let copy = project
        .db()
        .timetables()
        .duplicate(source.id, source.rev, "Version 1 (copy)")
        .expect("duplicate");

    let originals = entries(&mut project, written.timetable.id);
    let copies = entries(&mut project, copy.timetable.id);
    assert_eq!(copies.len(), originals.len());
    assert_eq!(
        copies.iter().filter(|entry| entry.is_locked).count(),
        1,
        "the lock came with the arrangement"
    );

    // Edit the copy.
    let in_the_copy = copies
        .iter()
        .find(|entry| !entry.is_locked)
        .expect("something movable")
        .clone();
    let target = another_slot(&mut project, &school, &in_the_copy);
    let session = open(&mut project, &selection, copy.timetable.id);
    project
        .db()
        .timetables()
        .move_entry(
            in_the_copy.id,
            in_the_copy.rev,
            EntryPlacement {
                timeslot_id: target,
                room_id: in_the_copy.room_id,
            },
            session.guard(),
        )
        .expect("the copy is edited");

    // The source is untouched, row for row.
    let now = entries(&mut project, written.timetable.id);
    assert_eq!(now.len(), originals.len());
    for before in &originals {
        let after = now
            .iter()
            .find(|entry| entry.id == before.id)
            .expect("the same row");
        assert_eq!(after.timeslot_id, before.timeslot_id);
        assert_eq!(after.room_id, before.room_id);
        assert_eq!(after.rev, before.rev, "and no revision was consumed");
        assert_eq!(after.is_locked, before.is_locked);
        assert_eq!(after.is_manual, before.is_manual);
    }

    // And the other direction: editing the source leaves the copy alone.
    let copy_before = entries(&mut project, copy.timetable.id);
    let movable = now
        .iter()
        .find(|entry| !entry.is_locked)
        .expect("something movable")
        .clone();
    let elsewhere = another_slot(&mut project, &school, &movable);
    let session = open(&mut project, &selection, written.timetable.id);
    project
        .db()
        .timetables()
        .move_entry(
            movable.id,
            movable.rev,
            EntryPlacement {
                timeslot_id: elsewhere,
                room_id: movable.room_id,
            },
            session.guard(),
        )
        .expect("the source is edited");
    assert_eq!(entries(&mut project, copy.timetable.id), copy_before);
}

#[test]
fn a_duplicate_belongs_to_its_source_school_and_no_other() {
    // School isolation survives the copy. A duplicate inherits `cycle_id` and `term_id` from its
    // source, so it is reachable from one school's list and not the other's — and nothing about
    // copying offers a route to move a timetable between schools.
    let mut project = Project::new();
    let north = a_solvable_school(&mut project);
    let south = a_second_school(&mut project);
    let northgate = north.selection();
    let written = saved(&mut project, &northgate, "Northgate v1");

    let copy = project
        .db()
        .timetables()
        .duplicate(written.timetable.id, written.timetable.rev, "Northgate v2")
        .expect("duplicate");
    assert_eq!(copy.timetable.cycle_id, written.timetable.cycle_id);
    assert_eq!(copy.timetable.term_id, written.timetable.term_id);

    let theirs = project
        .db()
        .timetables()
        .for_school(south.id)
        .expect("list Southmoor's");
    assert!(
        !theirs
            .iter()
            .any(|timetable| timetable.id == copy.timetable.id),
        "a copy of Northgate's timetable is not Southmoor's"
    );
    let mine = project
        .db()
        .timetables()
        .for_school(north.id)
        .expect("list Northgate's");
    assert!(
        mine.iter()
            .any(|timetable| timetable.id == copy.timetable.id)
    );
}
