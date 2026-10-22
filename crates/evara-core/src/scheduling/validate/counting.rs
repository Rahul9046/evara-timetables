//! Stage 3: the counting arguments of `SOLVER.md` §8 Layer 1.
//!
//! # What a counting argument is allowed to conclude
//!
//! Each check compares a **demand** with a **capacity** in the same unit, and fires only when
//! demand provably exceeds capacity. The direction is the whole discipline:
//!
//! - **Demand is exact or under-estimated.**
//! - **Capacity is exact or over-estimated.**
//!
//! Under those two, `demand > capacity` implies no timetable exists. The converse implies
//! nothing at all: sufficient aggregate capacity is not a feasible timetable, because
//! aggregate counting cannot see clashes, contiguity or room types interacting. So a check
//! that stays silent has proved nothing, which is `evara_core::lessons`' rule applied to
//! arithmetic:
//!
//! > Never report that a demand *can* be scheduled.
//!
//! A conservative check that misses some impossible configurations is acceptable. A check
//! that calls a valid configuration impossible is not, and every estimate below is biased in
//! the direction that makes a false refusal impossible.
//!
//! # The unit
//!
//! Every quantity here is a **period-slot**: one period of one cycle day. A double period is
//! two, so an activity contributes its `duration_periods` and never `1`. Counting activities
//! instead would under-state demand wherever durations differ, which is the mistake the
//! brief's example names.
//!
//! # Missing data
//!
//! Where an input needed to bound capacity is absent — most often a section that did not
//! resolve to a bell schedule — the entity is **skipped entirely** rather than bounded with a
//! guess. A skipped entity produces no finding, which is the conservative outcome. The
//! unresolved section has already produced its own blocking finding in stage 1.
//!
//! # Deferred checks
//!
//! `SOLVER.md` §8 lists six. Two cannot be implemented because their input vocabulary does
//! not exist, and inventing it is not M3's job:
//!
//! - **Lines whose members exceed the rooms of the required type.** There is no `line` table.
//!   `V15`'s header names `line` and `line_member` among the tables it deliberately did not
//!   create, and `ROADMAP.md` Phase 3 still lists the lines editor as remaining work.
//! - **Pinned activities that already conflict.** There is no `activity.is_locked` column.
//!   [`Problem::pinned`](crate::scheduling::Problem) is the declared hook and nothing
//!   populates it, so a check over it would be unreachable code asserting nothing.
//!
//! Both are recorded in `docs/PRE-SOLVE-VALIDATION.md` rather than stubbed.

use crate::scheduling::availability::{AvailabilityEntity, SlotAvailability};
use crate::scheduling::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticReport, EntityKind, EntityRef,
};
use crate::scheduling::index::{
    BellScheduleIx, GroupIx, RoomIx, RoomTypeIx, TeacherIx, TimeslotIx,
};

use super::{Pass, push};

/// Runs the three implementable counting checks, in a fixed order.
pub(super) fn findings(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    teacher_load(pass, report);
    room_type_supply(pass, report);
    group_budget(pass, report);
}

// =======================================================================================
// Check 1 — teacher load against usable teaching slots
// =======================================================================================

/// Teachers assigned more period-slots than the grid can offer them.
///
/// | | |
/// | --- | --- |
/// | **Counted** | demand: period-slots of every occurrence of every section the teacher is assigned to. capacity: distinct teaching slots of the schedules those sections resolve to, less the slots a hard availability rule removes |
/// | **Unit** | period-slots |
/// | **Inequality** | `Σ duration(a) > |usable slots|` |
/// | **Demand is exact** | every occurrence must be placed, and the teacher must be present for all `duration` periods of it. Where a section names two teachers, the demand counts for each — both are present |
/// | **Capacity is an over-estimate** | three ways. Slots of two schedules are counted as distinct even where they cover the same minutes, so a teacher working under two bell schedules is credited with more time than exists. Only `UNAVAILABLE` is subtracted; a `DISCOURAGED` slot is still counted. And nothing else competing for the teacher is considered |
/// | **Missing data** | a teacher any of whose sections did not resolve is skipped: without a schedule there is no slot set to bound, and guessing one could refuse a valid school |
/// | **Converse is invalid** | a teacher with 20 slots and 20 periods of demand is not thereby schedulable — every period could fall on one day, or clash with their groups' other lessons |
fn teacher_load(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (ix, teacher) in pass.problem.teachers.iter().enumerate() {
        let teacher_ix = TeacherIx::new(u32::try_from(ix).unwrap_or(u32::MAX));

        let mut demand: i64 = 0;
        let mut schedules: Vec<BellScheduleIx> = Vec::new();
        let mut unresolved = false;

        for (section_ix, section) in pass.sections() {
            if !section.teachers.contains(&teacher_ix) {
                continue;
            }
            match pass.schedule_of(section_ix) {
                None => unresolved = true,
                Some(schedule) => {
                    if !schedules.contains(&schedule) {
                        schedules.push(schedule);
                    }
                }
            }
            demand += demand_of_section(pass, section_ix);
        }

        if unresolved || demand == 0 {
            continue;
        }

        let usable = usable_slots(pass, &schedules, |slot| {
            pass.problem.unavailable.status(
                AvailabilityEntity::Teacher {
                    teacher: teacher_ix,
                },
                slot,
            ) != SlotAvailability::Unavailable
        });
        let capacity = i64::try_from(usable).unwrap_or(i64::MAX);

        if demand > capacity {
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::TeacherLoadExceedsSlots,
                    EntityRef::new(EntityKind::Teacher, teacher.id, teacher.name.clone()),
                    format!(
                        "{} is assigned {demand} periods a cycle, and at most {capacity} \
                         teaching periods are available to them. No timetable can fit the \
                         difference.",
                        teacher.name
                    ),
                )
                .with_remedy(format!(
                    "Move {} periods of {}'s lessons to another teacher, or widen their \
                     availability.",
                    demand - capacity,
                    teacher.name
                )),
            );
        }
    }
}

// =======================================================================================
// Check 2 — room-type demand against supply
// =======================================================================================

/// A kind of room asked for more period-slots than every room of that kind could provide.
///
/// | | |
/// | --- | --- |
/// | **Counted** | demand: period-slots of every occurrence whose section requires the type. supply: eligible rooms × distinct teaching slots of the schedules those sections resolve to |
/// | **Unit** | period-slots (room-slots, which is the same unit: one room for one period) |
/// | **Inequality** | `Σ duration(a) > rooms × |slots|` |
/// | **Demand is exact** | each occurrence needs one room of the type for all `duration` of its periods |
/// | **Supply is an over-estimate** | a room hosts at most one lesson per slot, so `rooms × slots` is the ceiling. It ignores availability, ignores that a room shared between campuses cannot serve both at once, and counts two schedules' overlapping minutes twice |
/// | **Missing data** | a type any of whose requiring sections did not resolve is skipped. A type with **zero** eligible rooms is also skipped here, because that degenerate case is reported per section as `no-room-of-required-type`, where the remedy is different |
/// | **Converse is invalid** | enough room-slots in aggregate says nothing about whether they are free at the times the lessons need |
fn room_type_supply(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (ix, room_type) in pass.problem.room_types.iter().enumerate() {
        let type_ix = RoomTypeIx::new(u32::try_from(ix).unwrap_or(u32::MAX));

        let mut demand: i64 = 0;
        let mut schedules: Vec<BellScheduleIx> = Vec::new();
        let mut rooms: Vec<RoomIx> = Vec::new();
        let mut unresolved = false;

        for (section_ix, section) in pass.sections() {
            if section.required_room_type != Some(type_ix) {
                continue;
            }
            match pass.schedule_of(section_ix) {
                None => unresolved = true,
                Some(schedule) => {
                    if !schedules.contains(&schedule) {
                        schedules.push(schedule);
                    }
                }
            }
            demand += demand_of_section(pass, section_ix);
            for room in &section.room_candidates {
                if !rooms.contains(room) {
                    rooms.push(*room);
                }
            }
        }

        // Zero eligible rooms is the degenerate case, reported per section with its own
        // remedy. Reporting it twice under two codes would be noise.
        if unresolved || demand == 0 || rooms.is_empty() {
            continue;
        }

        let slots = usable_slots(pass, &schedules, |_| true);
        let supply = i64::try_from(rooms.len())
            .unwrap_or(i64::MAX)
            .saturating_mul(i64::try_from(slots).unwrap_or(i64::MAX));

        if demand > supply {
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::RoomTypeDemandExceedsSupply,
                    EntityRef::new(EntityKind::RoomType, room_type.id, room_type.name.clone()),
                    format!(
                        "Lessons needing a {} ask for {demand} periods a cycle, and this \
                         school's {} such room{} can offer at most {supply}. No timetable can \
                         fit the difference.",
                        room_type.name,
                        rooms.len(),
                        if rooms.len() == 1 { "" } else { "s" }
                    ),
                )
                .with_remedy(format!(
                    "Add another bookable {}, reduce the lessons that require one, or add \
                     teaching periods to the cycle.",
                    room_type.name
                )),
            );
        }
    }
}

// =======================================================================================
// Check 3 — student-group period budget
// =======================================================================================

/// Student groups whose lessons ask for more period-slots than the grid can offer them.
///
/// The check `SOLVER.md` §8's own example is about: *"Year 9 needs 42 periods per cycle but
/// only 40 teaching slots exist."*
///
/// | | |
/// | --- | --- |
/// | **Counted** | demand: period-slots of every occurrence of every section the group attends. capacity: distinct teaching slots of the schedules those sections resolve to, less the slots a hard availability rule removes |
/// | **Unit** | period-slots |
/// | **Inequality** | `Σ duration(a) > |usable slots|` |
/// | **Demand is an under-estimate** | it counts only sections whose audience *names* this group. A section taught to a group that overlaps this one also consumes its students' time, and attributing that would need [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) §3's overlap function, which is M5's. Under-estimating is the safe direction, so the check is conservative rather than wrong |
/// | **Capacity is an over-estimate** | as check 1 |
/// | **Missing data** | a group any of whose sections did not resolve is skipped. An `Indeterminate` overlap is simply not counted, never assumed disjoint |
/// | **Converse is invalid** | 20 periods of demand against 30 slots proves nothing: the group's lessons may all need the one laboratory, or the one teacher |
fn group_budget(pass: &Pass<'_>, report: &mut DiagnosticReport) {
    for (ix, group) in pass.problem.groups.iter().enumerate() {
        let group_ix = GroupIx::new(u32::try_from(ix).unwrap_or(u32::MAX));

        let mut demand: i64 = 0;
        let mut schedules: Vec<BellScheduleIx> = Vec::new();
        let mut unresolved = false;

        for (section_ix, section) in pass.sections() {
            if !section.audience.contains(&group_ix) {
                continue;
            }
            match pass.schedule_of(section_ix) {
                None => unresolved = true,
                Some(schedule) => {
                    if !schedules.contains(&schedule) {
                        schedules.push(schedule);
                    }
                }
            }
            demand += demand_of_section(pass, section_ix);
        }

        if unresolved || demand == 0 {
            continue;
        }

        let usable = usable_slots(pass, &schedules, |slot| {
            pass.problem
                .unavailable
                .status(AvailabilityEntity::Group { group: group_ix }, slot)
                != SlotAvailability::Unavailable
        });
        let capacity = i64::try_from(usable).unwrap_or(i64::MAX);

        if demand > capacity {
            push(
                report,
                Diagnostic::new(
                    DiagnosticCode::GroupDemandExceedsSlots,
                    EntityRef::new(EntityKind::StudentGroup, group.id, group.name.clone()),
                    format!(
                        "{} needs {demand} periods a cycle and at most {capacity} teaching \
                         periods are available to it. No timetable can fit the difference.",
                        group.name
                    ),
                )
                .with_remedy(
                    "Reduce a course's periods per cycle for this group, or add teaching \
                     periods to the cycle."
                        .to_owned(),
                ),
            );
        }
    }
}

// =======================================================================================
// Shared arithmetic
// =======================================================================================

/// The period-slots every occurrence of one section consumes.
///
/// A **sum of durations**, never a count of rows: four singles and a double is six, which no
/// multiplication of a count by a duration produces. The same arithmetic
/// `evara_core::lessons::LessonDemand` performs for one section's screen.
fn demand_of_section(pass: &Pass<'_>, section: crate::scheduling::index::SectionIx) -> i64 {
    pass.problem
        .activities
        .iter()
        .filter(|activity| activity.section == section)
        .map(|activity| i64::from(activity.duration.get()))
        .sum()
}

/// How many distinct teaching slots of `schedules` satisfy `keep`.
///
/// Distinct by slot index, so a schedule listed twice is not counted twice. Slots of two
/// different schedules are **both** counted even where their minutes overlap, which
/// over-states capacity — the safe direction, as each check's table records.
fn usable_slots(
    pass: &Pass<'_>,
    schedules: &[BellScheduleIx],
    keep: impl Fn(TimeslotIx) -> bool,
) -> usize {
    let mut seen: Vec<TimeslotIx> = Vec::new();
    for schedule in schedules {
        for slot in pass.teaching_slots(*schedule) {
            if keep(*slot) && !seen.contains(slot) {
                seen.push(*slot);
            }
        }
    }
    seen.len()
}
