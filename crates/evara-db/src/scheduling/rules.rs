//! Availability rules: what the school has said about when something cannot, or would
//! rather not, be timetabled.
//!
//! # Available is the default, so only exceptions are stored
//!
//! `V11`'s header: an entity with no rows here is free in every slot. That is what keeps
//! *"not configured yet"* and *"never free"* from collapsing into the same empty set, and it
//! is why [`SlotAvailability::Unconstrained`](evara_core::scheduling::SlotAvailability) is a
//! distinct fourth state rather than a synonym for available. Nothing in this module invents
//! an `AVAILABLE` row and nothing reads a missing rule as a restriction.
//!
//! # A rule compiles onto the slots it names, and is never widened
//!
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §7 sets the rule and the limitation it accepts. The three stored granularities are not
//! equally portable across bell schedules:
//!
//! | Stored as | Names |
//! | --- | --- |
//! | `cycle_day_id` alone | every slot of the grid on that day, on **every** schedule |
//! | `period_id`, alone or with a day | the slots of that one `period` row — so **one** schedule |
//! | `timeslot_id` | one exact slot, and therefore one schedule and one term |
//!
//! So a teacher unavailable "period 1, every day" at a two-schedule school has said
//! something about one schedule. This loader compiles each rule onto the slots it actually
//! names and widens nothing; M3's `teacher-under-several-bell-schedules` is what makes the
//! consequent omission visible rather than discovered in a finished timetable.
//!
//! # All three statuses are compiled
//!
//! `UNAVAILABLE` is hard; `PREFERRED` and `DISCOURAGED` are soft and are not enforced in the
//! MVP. All three are nonetheless written into the index, because discarding the soft ones
//! would make "the school said something here" unrecoverable and
//! [`UnavailabilityIndex`](evara_core::scheduling::UnavailabilityIndex) already keeps them
//! apart. A `DISCOURAGED` slot is legal and merely worse, which is what makes it soft.
//!
//! # Nothing here reads a date
//!
//! `effective_from` / `effective_to` are carried onto each
//! [`AvailabilityRule`](evara_core::scheduling::AvailabilityRule) verbatim and are **not**
//! consulted when deciding which slots a rule covers. That is the accepted contract, not a
//! shortcut: a cycle carries no dates (ADR 0017), the timetable is defined over the cycle,
//! and ADR 0032 §5 states that an incomplete or absent `calendar_day` mapping has no effect
//! on any rule the accepted decisions define. `EffectiveWindow`'s own documentation says the
//! window *"is carried so the loader does not have to discard a stored fact"*.
//!
//! The consequence, stated so it is not discovered later: a date-bounded restriction is
//! applied for the whole cycle. For an `UNAVAILABLE` rule that is the conservative
//! direction — it removes options rather than inventing them — and a milestone that projects
//! the cycle onto dates will find the window already modelled.

use evara_core::scheduling::{
    AvailabilityEntity, AvailabilityEntityKind, AvailabilityRule, AvailabilityScope,
    AvailabilityStatus, EffectiveWindow, TimeslotFact, UnavailabilityIndex,
};
use rusqlite::Connection;
use uuid::Uuid;

use super::index::ProblemIndex;
use super::{SchedulingSelection, corrupt};
use crate::error::Result;
use crate::repo::row::{parse_id_text, text};

/// One stored rule, and what the loader could do with it.
pub(super) struct CompiledRules {
    /// The rules as stored, in reporting order.
    pub(super) rules: Vec<AvailabilityRule>,
    /// What the compiled rules say, slot by slot.
    pub(super) index: UnavailabilityIndex,
    /// Rules naming an entity that no longer exists, with the kind each named.
    pub(super) orphans: Vec<Orphan>,
    /// Rules naming an entity kind the scheduling index does not model.
    pub(super) unscheduled: Vec<Orphan>,
    /// Rules about something real that this solve does not schedule.
    ///
    /// Counted and not reported — see [`out_of_scope`](CompiledRules::out_of_scope).
    pub(super) out_of_scope: usize,
}

/// A rule the loader could not compile, and enough to name it in a finding.
pub(super) struct Orphan {
    /// The rule's own identifier, so the finding names the row rather than its subject.
    pub(super) id: Uuid,
    /// What kind of thing it claimed to be about.
    pub(super) kind: AvailabilityEntityKind,
    /// Which row of that kind.
    pub(super) entity_id: Uuid,
}

/// Reads this school's availability rules and compiles the ones it can.
///
/// # What happens to a rule that cannot be compiled
///
/// Three outcomes, kept apart because they have three different remedies —
/// [ADR 0037](../../../../docs/adr/0037-stored-row-diagnostics-belong-to-the-loader.md) §3
/// requires that such a row *"not silently discard"* and that the finding name the kind:
///
/// 1. **The kind is not scheduled** (`RESOURCE`, `SUBJECT`). Reported as
///    `availability-entity-kind-not-scheduled`. The loader does not then go on to ask
///    whether the row still exists: one row gets one finding, and "this is not being
///    enforced" is the fact the user can act on.
/// 2. **The entity is gone.** Reported as `orphan-availability-reference`. `entity_id` is
///    the one reference in this schema no foreign key covers (`V11`'s header), so this is
///    reachable despite `Availability::clear_for_entity` existing to prevent it — which is
///    exactly why it is reported rather than assumed impossible.
/// 3. **The entity is real and out of scope.** A teacher whose membership was deactivated
///    and who teaches nothing in this term, a slot of another cycle, a rule naming a term
///    this grid is not for. Counted, **not** reported.
///
/// The third needs its reason on the record, because reporting it is the tempting choice.
/// Such a rule restricts nothing this solve can place, so there is no violation it could
/// prevent and no remedy to offer; and reporting it under `orphan-availability-reference`
/// would state that the entity no longer exists, which is false and would send somebody
/// looking for a deletion that never happened. That is the failure
/// [`parse_id_text`](crate::repo::row::parse_id_text)'s own documentation warns about,
/// applied to a reference rather than to an identifier.
pub(super) fn load(
    conn: &Connection,
    selection: &SchedulingSelection,
    index: &ProblemIndex,
    timeslots: &[TimeslotFact],
) -> Result<CompiledRules> {
    let present = existing_entities(conn, selection)?;

    let mut compiled = CompiledRules {
        rules: Vec::new(),
        index: UnavailabilityIndex::empty(timeslots.len()),
        orphans: Vec::new(),
        unscheduled: Vec::new(),
        out_of_scope: 0,
    };

    let mut stmt = conn.prepare(
        "SELECT id, entity_kind, entity_id, timeslot_id, cycle_day_id, period_id, term_id,
                status, weight, effective_from, effective_to
         FROM availability
         WHERE school_id = ?
         ORDER BY entity_kind, entity_id, COALESCE(cycle_day_id, ''),
                  COALESCE(period_id, ''), COALESCE(timeslot_id, ''), id",
    )?;
    let rows = stmt.query_map([text(selection.school_id)], |row| {
        Ok(StoredRule {
            id: row.get(0)?,
            kind: row.get(1)?,
            entity_id: row.get(2)?,
            timeslot_id: row.get(3)?,
            cycle_day_id: row.get(4)?,
            period_id: row.get(5)?,
            term_id: row.get(6)?,
            status: row.get(7)?,
            weight: row.get(8)?,
            effective_from: row.get(9)?,
            effective_to: row.get(10)?,
        })
    })?;

    for row in rows {
        let row = row?;
        let id = parse_id_text("availability.id", &row.id)?;
        let kind = entity_kind(&row.kind)?;
        let entity_id = parse_id_text("availability.entity_id", &row.entity_id)?;
        let status = status(&row.status)?;
        let scope = scope(&row)?;
        let term = row
            .term_id
            .as_deref()
            .map(|raw| parse_id_text("availability.term_id", raw))
            .transpose()?;

        if !is_scheduled(kind) {
            compiled.unscheduled.push(Orphan {
                id,
                kind,
                entity_id,
            });
            continue;
        }
        let Some(entity) = resolve(kind, entity_id, index) else {
            if present.holds(kind, entity_id) {
                compiled.out_of_scope += 1;
            } else {
                compiled.orphans.push(Orphan {
                    id,
                    kind,
                    entity_id,
                });
            }
            continue;
        };

        // A rule naming a term this grid is not for restricts nothing in it. Every slot of
        // the grid carries the selected term, so the comparison is against the selection
        // rather than against each slot.
        if term.is_some() && term != selection.term_id {
            compiled.out_of_scope += 1;
            continue;
        }

        let named = slots_named(&scope, index, timeslots);
        if named.is_empty() {
            compiled.out_of_scope += 1;
            continue;
        }
        for slot in &named {
            compiled.index.insert(entity, *slot, status);
        }

        // Recorded only once it has actually been compiled. `AvailabilityEntity` is a dense
        // index by construction, so a rule the loader could not resolve has no honest value
        // to carry here — and a placeholder index would be a dangling reference sitting in
        // a list whose whole purpose is to be trustworthy. The rules that could not be
        // compiled are reported instead, by identifier, which is what ADR 0037 §3 asks for.
        compiled.rules.push(AvailabilityRule {
            id,
            entity,
            scope,
            status,
            term,
            weight: row.weight,
            effective: EffectiveWindow {
                from: row.effective_from,
                to: row.effective_to,
            },
        });
    }

    Ok(compiled)
}

/// One `availability` row, as read.
struct StoredRule {
    id: String,
    kind: String,
    entity_id: String,
    timeslot_id: Option<String>,
    cycle_day_id: Option<String>,
    period_id: Option<String>,
    term_id: Option<String>,
    status: String,
    weight: Option<i64>,
    effective_from: Option<String>,
    effective_to: Option<String>,
}

/// Which entity rows this school's rules name that still exist.
///
/// One query per scheduled kind rather than one per rule: `Availability::orphans` asks row
/// by row, which is right for an integrity sweep over a whole project and is the N+1 a
/// loader must not perform.
///
/// The two unscheduled kinds are deliberately absent. Their rows get a finding about the
/// *kind*, so whether the entity survives changes nothing the user would do.
fn existing_entities(
    conn: &Connection,
    selection: &SchedulingSelection,
) -> Result<ExistingEntities> {
    let mut found = ExistingEntities::default();
    for (kind, table) in [
        (AvailabilityEntityKind::Teacher, "teacher"),
        (AvailabilityEntityKind::Room, "room"),
        (AvailabilityEntityKind::StudentGroup, "student_group"),
        (AvailabilityEntityKind::Campus, "campus"),
    ] {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT a.entity_id
             FROM availability a
             WHERE a.school_id = ?1 AND a.entity_kind = ?2
               AND EXISTS (SELECT 1 FROM {} t WHERE t.id = a.entity_id)",
            crate::repo::identifier(table)?
        ))?;
        let rows = stmt.query_map(
            rusqlite::params![text(selection.school_id), stored_kind(kind)],
            |row| row.get::<_, String>(0),
        )?;
        for row in rows {
            let id = parse_id_text("availability.entity_id", &row?)?;
            found.push(kind, id);
        }
    }
    Ok(found)
}

/// The entity rows this school's rules name that are still there.
#[derive(Debug, Default)]
struct ExistingEntities {
    /// `(kind, id)` pairs, sorted so membership is a binary search.
    present: Vec<(u8, Uuid)>,
}

impl ExistingEntities {
    fn push(&mut self, kind: AvailabilityEntityKind, id: Uuid) {
        let key = (rank(kind), id);
        if let Err(at) = self.present.binary_search(&key) {
            self.present.insert(at, key);
        }
    }

    fn holds(&self, kind: AvailabilityEntityKind, id: Uuid) -> bool {
        self.present.binary_search(&(rank(kind), id)).is_ok()
    }
}

/// A sortable discriminant for an entity kind.
const fn rank(kind: AvailabilityEntityKind) -> u8 {
    match kind {
        AvailabilityEntityKind::Teacher => 0,
        AvailabilityEntityKind::Room => 1,
        AvailabilityEntityKind::StudentGroup => 2,
        AvailabilityEntityKind::Resource => 3,
        AvailabilityEntityKind::Campus => 4,
        AvailabilityEntityKind::Subject => 5,
    }
}

/// The value `availability.entity_kind` stores.
const fn stored_kind(kind: AvailabilityEntityKind) -> &'static str {
    match kind {
        AvailabilityEntityKind::Teacher => "TEACHER",
        AvailabilityEntityKind::Room => "ROOM",
        AvailabilityEntityKind::StudentGroup => "STUDENT_GROUP",
        AvailabilityEntityKind::Resource => "RESOURCE",
        AvailabilityEntityKind::Campus => "CAMPUS",
        AvailabilityEntityKind::Subject => "SUBJECT",
    }
}

/// Whether the compiled scheduling index models this kind at all.
///
/// Four of the column's six. The asymmetry is deliberate and ADR 0037's *Neutral* note is
/// the record of it: `AvailabilityEntityKind` names six so that a loader meeting the other
/// two can say which it found, while [`AvailabilityEntity`] compiles four.
const fn is_scheduled(kind: AvailabilityEntityKind) -> bool {
    match kind {
        AvailabilityEntityKind::Teacher
        | AvailabilityEntityKind::Room
        | AvailabilityEntityKind::StudentGroup
        | AvailabilityEntityKind::Campus => true,
        // A resource is stored, shareable and unscheduled: `resource-clash` is not in the
        // first constraint set. A subject is not a schedulable thing at all —
        // `Availability::set` refuses to write one, so such a row can only be legacy.
        AvailabilityEntityKind::Resource | AvailabilityEntityKind::Subject => false,
    }
}

/// Resolves a stored reference to the dense index the compiled problem addresses it by.
fn resolve(
    kind: AvailabilityEntityKind,
    entity_id: Uuid,
    index: &ProblemIndex,
) -> Option<AvailabilityEntity> {
    match kind {
        AvailabilityEntityKind::Teacher => index
            .teacher_index(entity_id)
            .map(|teacher| AvailabilityEntity::Teacher { teacher }),
        AvailabilityEntityKind::Room => index
            .room_index(entity_id)
            .map(|room| AvailabilityEntity::Room { room }),
        AvailabilityEntityKind::StudentGroup => index
            .group_index(entity_id)
            .map(|group| AvailabilityEntity::Group { group }),
        AvailabilityEntityKind::Campus => index
            .campus_index(entity_id)
            .map(|campus| AvailabilityEntity::Campus { campus }),
        AvailabilityEntityKind::Resource | AvailabilityEntityKind::Subject => None,
    }
}

/// The slots of the grid a rule's scope names.
///
/// Empty where the scope names something outside this grid — a cycle day of another cycle, a
/// period of a schedule that was never materialised, a slot of another term. That is the
/// "out of scope" outcome, and it is how a rule about an unmaterialised bell schedule stops
/// mattering without anything having to detect it specially.
fn slots_named(
    scope: &AvailabilityScope,
    index: &ProblemIndex,
    timeslots: &[TimeslotFact],
) -> Vec<evara_core::scheduling::TimeslotIx> {
    let all = || {
        (0..timeslots.len()).map(|at| {
            evara_core::scheduling::TimeslotIx::new(u32::try_from(at).unwrap_or(u32::MAX))
        })
    };
    match *scope {
        AvailabilityScope::Timeslot { timeslot } => {
            index.timeslot_index(timeslot).into_iter().collect()
        }
        AvailabilityScope::CycleDay { cycle_day } => index
            .cycle_day_index(cycle_day)
            .map(|day| {
                all()
                    .filter(|slot| timeslots[slot.get()].cycle_day == day)
                    .collect()
            })
            .unwrap_or_default(),
        AvailabilityScope::Period { period } => index
            .period_index(period)
            .map(|period| {
                all()
                    .filter(|slot| timeslots[slot.get()].period == period)
                    .collect()
            })
            .unwrap_or_default(),
        AvailabilityScope::CycleDayAndPeriod { cycle_day, period } => {
            match (index.cycle_day_index(cycle_day), index.period_index(period)) {
                (Some(day), Some(period)) => all()
                    .filter(|slot| {
                        timeslots[slot.get()].cycle_day == day
                            && timeslots[slot.get()].period == period
                    })
                    .collect(),
                _ => Vec::new(),
            }
        }
    }
}

/// Reads which slots a row claims, most specific first.
///
/// `V11`'s `CHECK` requires at least one of the three, so a row naming none is corruption.
fn scope(row: &StoredRule) -> Result<AvailabilityScope> {
    if let Some(raw) = row.timeslot_id.as_deref() {
        return Ok(AvailabilityScope::Timeslot {
            timeslot: parse_id_text("availability.timeslot_id", raw)?,
        });
    }
    let day = row
        .cycle_day_id
        .as_deref()
        .map(|raw| parse_id_text("availability.cycle_day_id", raw))
        .transpose()?;
    let period = row
        .period_id
        .as_deref()
        .map(|raw| parse_id_text("availability.period_id", raw))
        .transpose()?;
    match (day, period) {
        (Some(cycle_day), Some(period)) => {
            Ok(AvailabilityScope::CycleDayAndPeriod { cycle_day, period })
        }
        (Some(cycle_day), None) => Ok(AvailabilityScope::CycleDay { cycle_day }),
        (None, Some(period)) => Ok(AvailabilityScope::Period { period }),
        (None, None) => Err(corrupt("availability.timeslot_id", String::from("NULL"))),
    }
}

/// Reads `availability.entity_kind`.
fn entity_kind(raw: &str) -> Result<AvailabilityEntityKind> {
    match raw {
        "TEACHER" => Ok(AvailabilityEntityKind::Teacher),
        "ROOM" => Ok(AvailabilityEntityKind::Room),
        "STUDENT_GROUP" => Ok(AvailabilityEntityKind::StudentGroup),
        "RESOURCE" => Ok(AvailabilityEntityKind::Resource),
        "CAMPUS" => Ok(AvailabilityEntityKind::Campus),
        "SUBJECT" => Ok(AvailabilityEntityKind::Subject),
        other => Err(corrupt("availability.entity_kind", other.into())),
    }
}

/// Reads `availability.status`.
fn status(raw: &str) -> Result<AvailabilityStatus> {
    match raw {
        "UNAVAILABLE" => Ok(AvailabilityStatus::Unavailable),
        "PREFERRED" => Ok(AvailabilityStatus::Preferred),
        "DISCOURAGED" => Ok(AvailabilityStatus::Discouraged),
        other => Err(corrupt("availability.status", other.into())),
    }
}
