//! When an entity cannot, or would rather not, be timetabled.
//!
//! # Available is the default, so only exceptions are stored
//!
//! `V11`'s own header states the rule this module is built on: *"Available is the default,
//! so only exceptions are stored. A teacher with no rows here is available in every slot.
//! That is what keeps the table small and what makes 'I have not configured this yet' and
//! 'this teacher is never free' different states rather than the same empty set."*
//!
//! So **a missing rule means available** — checked against the authoritative schema comment
//! rather than assumed either way. [`SlotAvailability::Unconstrained`] is that state, and it
//! is kept distinct from [`SlotAvailability::Preferred`] because the schema offers no
//! "explicitly available" status: the nearest thing a school can say is `PREFERRED`, which
//! is a soft *wish* and not merely a confirmation. Collapsing the two would make
//! "nobody has said" indistinguishable from "somebody asked for this slot".
//!
//! # Granularity does not travel between bell schedules
//!
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §7 is the decision this module encodes most carefully. A rule scoped to a `cycle_day_id`
//! is structure-independent — "all of Monday" means all of Monday on every schedule — while
//! a rule scoped to a `period_id` **belongs to one bell schedule**, because a `period` row
//! belongs to one `period_structure`. A teacher unavailable "period 1, every day" at a
//! two-schedule school has therefore said something about *one* schedule, and the loader
//! compiles each rule onto the slots it actually names and **widens nothing**.
//!
//! That is a named limitation rather than a defect, and it comes with a warning: pre-flight
//! reports a teacher who teaches under more than one structure in scope, so the omission is
//! visible before a solve rather than discovered in the finished timetable.
//!
//! # What is **not** here
//!
//! The `entity-unavailable` evaluator is M5's, and the compilation of stored rows into an
//! index is M4's. This module declares the rule, the three-state answer, and the compiled
//! index both of those produce and consume.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::index::{CampusIx, GroupIx, RoomIx, TeacherIx, TimeslotIx};

/// The kind of thing an availability rule is about, as `availability.entity_kind` stores it.
///
/// All six of the column's values (`V11:44-45`), because a diagnostic about a damaged or
/// orphaned rule has to be able to name the kind it found — and only three of them are
/// compiled into the MVP's index. See [`AvailabilityEntity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AvailabilityEntityKind {
    /// A teacher.
    Teacher,
    /// A room.
    Room,
    /// A student group.
    StudentGroup,
    /// A bookable resource. **Not scheduled in the MVP** — no table links a resource to an
    /// activity, so nothing consumes its availability yet.
    Resource,
    /// A whole campus. Not compiled in the MVP.
    Campus,
    /// A subject. Not compiled in the MVP.
    Subject,
}

/// An entity whose availability the first solver actually reads, as a dense index.
///
/// Three of [`AvailabilityEntityKind`]'s six: teacher, room and student group — the
/// entities `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6 names for `entity-unavailable` in the
/// first constraint set. A campus is reachable too, because a rule may close a whole campus
/// for a slot and that restricts every room on it; it is carried here so the loader has
/// somewhere to put such a rule rather than discarding it silently.
///
/// `RESOURCE` and `SUBJECT` are deliberately absent. Nothing links a resource to an activity
/// (the audit's N4), and a subject-scoped rule has no agreed meaning for a placement; a
/// stored rule naming either is reported by pre-flight rather than compiled into an index
/// where it would appear to be enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum AvailabilityEntity {
    /// A teacher of this school.
    Teacher {
        /// Which teacher.
        teacher: TeacherIx,
    },
    /// A room reachable from this school.
    Room {
        /// Which room.
        room: RoomIx,
    },
    /// A student group of this school.
    Group {
        /// Which group.
        group: GroupIx,
    },
    /// A whole campus of this school.
    Campus {
        /// Which campus.
        campus: CampusIx,
    },
}

/// Which slots one availability rule names.
///
/// The four shapes `V11`'s header enumerates, each a distinct combination of the nullable
/// `timeslot_id`, `cycle_day_id` and `period_id` columns. At least one must be present; a
/// row naming none of them would be a rule about nothing, and is refused by the table.
///
/// The variants differ in **how far the rule travels**, which
/// [`AvailabilityScope::is_bell_schedule_bound`] answers and ADR 0033 §7 explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum AvailabilityScope {
    /// One exact materialised slot.
    ///
    /// Carries its own term, so a `term_id` beside it would be redundant — and nothing that
    /// writes here combines the two.
    Timeslot {
        /// The slot, by stored identity.
        timeslot: Uuid,
    },
    /// A whole cycle position: "all of Monday".
    ///
    /// **Structure-independent.** It binds every slot of that day under every bell schedule,
    /// because a `cycle_day` belongs to the cycle and not to a schedule.
    CycleDay {
        /// The position, by stored identity.
        cycle_day: Uuid,
    },
    /// One period of one bell schedule, on every day: "period 1, every day".
    ///
    /// **Bound to one schedule**, because a `period` row belongs to one `period_structure`.
    Period {
        /// The period, by stored identity.
        period: Uuid,
    },
    /// One recurring cell — what the grid editor writes.
    ///
    /// Also bound to one schedule, for the same reason as [`AvailabilityScope::Period`].
    CycleDayAndPeriod {
        /// The position, by stored identity.
        cycle_day: Uuid,
        /// The period, by stored identity.
        period: Uuid,
    },
}

impl AvailabilityScope {
    /// Whether this rule says something about **one** bell schedule only.
    ///
    /// True wherever a `period_id` is involved. ADR 0033 §7: such a rule binds the slots it
    /// names and is never widened to the same ordinal under another schedule, so a shared
    /// teacher's period-level unavailability must be entered once per schedule they teach
    /// under.
    #[must_use]
    pub const fn is_bell_schedule_bound(self) -> bool {
        matches!(
            self,
            Self::Period { .. } | Self::CycleDayAndPeriod { .. } | Self::Timeslot { .. }
        )
    }
}

/// What an availability rule says, as `availability.status` stores it.
///
/// Note what is **not** here: there is no `AVAILABLE`. Availability is the default and only
/// exceptions are stored, so the absence of a rule is the positive case — see
/// [`SlotAvailability::Unconstrained`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AvailabilityStatus {
    /// A hard rule: no lesson may be placed here.
    ///
    /// The only status the first constraint set enforces, through `entity-unavailable`.
    Unavailable,
    /// A soft wish *for* these slots. Not enforced in the MVP — every soft constraint is
    /// deferred.
    Preferred,
    /// A soft wish *against* these slots. Also deferred.
    Discouraged,
}

impl AvailabilityStatus {
    /// Whether this status refuses a placement outright.
    #[must_use]
    pub const fn is_hard(self) -> bool {
        matches!(self, Self::Unavailable)
    }
}

/// One stored availability rule, as the loader reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailabilityRule {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the rule is about.
    pub entity: AvailabilityEntity,
    /// Which slots it names.
    pub scope: AvailabilityScope,
    /// What it says.
    pub status: AvailabilityStatus,
    /// The term it applies to, or `None` for all year.
    ///
    /// `NULL` means the rule applies all year, which is the common case and not the exotic
    /// one. Redundant beside [`AvailabilityScope::Timeslot`], which carries its own term.
    pub term: Option<Uuid>,
    /// A soft weight for [`AvailabilityStatus::Preferred`] and
    /// [`AvailabilityStatus::Discouraged`], and meaningless for
    /// [`AvailabilityStatus::Unavailable`].
    ///
    /// `None` means unset; the constraint supplies its own default. An integer rather than a
    /// float, because `CLAUDE.md` requires score arithmetic to be integer and a real would
    /// make this record non-`Eq`.
    pub weight: Option<i64>,
    /// The date window this rule is confined to, when it has one.
    pub effective: EffectiveWindow,
}

/// The dates between which a rule applies.
///
/// `availability.effective_from` / `effective_to` are canonical `YYYY-MM-DD` text under a
/// `date()` `CHECK`, carried verbatim here for the same reason the columns exist: ADR 0006
/// asked for date-bounded exceptions — a term's maternity cover, a room closed for
/// refurbishment — and they are stored before anything collects them.
///
/// # Nothing in the MVP reads this
///
/// No clash decision reads a date. A cycle carries no dates
/// ([ADR 0017](../../../../docs/adr/0017-weekly-and-custom-cycles.md)), the timetable is
/// defined over the cycle, and
/// [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md) §5
/// states plainly that an incomplete or absent `calendar_day` mapping has no effect on any
/// rule the accepted decisions define. The window is carried so the loader does not have to
/// discard a stored fact, and so a later milestone that does project onto dates finds it
/// already modelled.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveWindow {
    /// First date the rule applies, `YYYY-MM-DD`, or `None` for unbounded.
    pub from: Option<String>,
    /// Last date the rule applies, `YYYY-MM-DD`, or `None` for unbounded.
    pub to: Option<String>,
}

impl EffectiveWindow {
    /// A window that bounds nothing: the ordinary state of every rule a screen writes today.
    #[must_use]
    pub const fn unbounded() -> Self {
        Self {
            from: None,
            to: None,
        }
    }

    /// Whether either end is set.
    #[must_use]
    pub const fn is_bounded(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }
}

/// What the stored rules say about one entity in one slot.
///
/// Four states, and the fourth is the one that is easy to get wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlotAvailability {
    /// A rule says the entity may not be timetabled here. Hard.
    Unavailable,
    /// A rule asks for this slot. Soft, and not enforced in the MVP.
    Preferred,
    /// A rule asks against this slot. Soft, and not enforced in the MVP.
    Discouraged,
    /// No rule names this entity and this slot.
    ///
    /// **This means available**, per `V11`'s header, and it is the default. It is a
    /// different state from [`SlotAvailability::Preferred`]: the schema has no
    /// "explicitly available" status, so the absence of a rule is the only way to say
    /// "nobody has an opinion", and a school that asked for a slot has said more than that.
    #[default]
    Unconstrained,
}

impl SlotAvailability {
    /// Whether a lesson may be placed here at all.
    ///
    /// Everything except [`SlotAvailability::Unavailable`]. A discouraged slot is legal and
    /// merely worse, which is what makes it soft.
    #[must_use]
    pub const fn permits_placement(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    /// Whether any rule at all names this entity and slot.
    #[must_use]
    pub const fn is_stated(self) -> bool {
        !matches!(self, Self::Unconstrained)
    }
}

/// A set of timeslot indices, as a bitset.
///
/// `SOLVER.md` §2's `UnavailabilityIndex` is described as a "bitset per (entity, timeslot)",
/// and this is the per-entity half of it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotMask {
    /// One bit per timeslot index, least-significant bit first within each word.
    words: Vec<u64>,
}

impl SlotMask {
    /// An empty mask with room for `slots` indices.
    #[must_use]
    pub fn with_capacity(slots: usize) -> Self {
        Self {
            words: vec![0; slots.div_ceil(64)],
        }
    }

    /// Adds a slot, growing the mask if it does not reach that far yet.
    pub fn insert(&mut self, slot: TimeslotIx) {
        let (word, bit) = (slot.get() / 64, slot.get() % 64);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1u64 << bit;
    }

    /// Whether a slot is in the set.
    #[must_use]
    pub fn contains(&self, slot: TimeslotIx) -> bool {
        let (word, bit) = (slot.get() / 64, slot.get() % 64);
        self.words.get(word).is_some_and(|w| w & (1u64 << bit) != 0)
    }

    /// How many slots are in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether no slot is in the set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// The slots in the set, in ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = TimeslotIx> + '_ {
        self.words.iter().enumerate().flat_map(|(word, bits)| {
            (0..64)
                .filter(move |bit| bits & (1u64 << bit) != 0)
                .map(move |bit| TimeslotIx::new(u32::try_from(word * 64 + bit).unwrap_or(u32::MAX)))
        })
    }
}

/// One entity's slots at one status.
///
/// A record rather than a map entry, because [`AvailabilityEntity`] is a tagged enum and JSON
/// map keys must be strings — a `BTreeMap` keyed on it serialises to an error rather than to
/// a document. A list of these serialises cleanly and keeps the ordering a reproducible solve
/// needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntitySlots {
    /// Whose slots these are.
    entity: AvailabilityEntity,
    /// The slots.
    slots: SlotMask,
}

/// Every entity's availability, compiled onto the slots the stored rules actually name.
///
/// # Compiled, never widened
///
/// Each rule is expanded onto its own slots and no others — ADR 0033 §7. A `period_id`-scoped
/// rule reaches the slots that period produces and not the same ordinal under another bell
/// schedule; a `cycle_day_id`-scoped rule reaches that day under every schedule. The
/// expansion is M4's work; this type is what it produces.
///
/// Entities with no rules are simply absent, which is the same statement as "available
/// everywhere" and costs nothing to store.
///
/// # Sorted lists, not hash maps
///
/// Each table is kept sorted by entity, so iteration order is deterministic and a lookup is a
/// binary search over the handful of entities a school actually restricts. A `HashMap` would
/// make the serialised form vary between runs, which would defeat the point of a fixture
/// whose output is meant to be byte-for-byte stable.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnavailabilityIndex {
    /// How many slots the masks are over, so a lookup can tell an out-of-range index from an
    /// unset bit.
    slots: usize,
    /// Slots each entity may not be placed in, sorted by entity.
    unavailable: Vec<EntitySlots>,
    /// Slots each entity asked for. Carried and unread by the first constraint set.
    preferred: Vec<EntitySlots>,
    /// Slots each entity asked against. Carried and unread by the first constraint set.
    discouraged: Vec<EntitySlots>,
}

impl UnavailabilityIndex {
    /// An index over `slots` timeslots in which nothing is restricted.
    ///
    /// The correct state for a school that has entered no availability at all, and it means
    /// everybody is available everywhere.
    #[must_use]
    pub const fn empty(slots: usize) -> Self {
        Self {
            slots,
            unavailable: Vec::new(),
            preferred: Vec::new(),
            discouraged: Vec::new(),
        }
    }

    /// How many slots the index covers.
    #[must_use]
    pub const fn slots(&self) -> usize {
        self.slots
    }

    /// Records that `entity` has `status` in `slot`.
    pub fn insert(
        &mut self,
        entity: AvailabilityEntity,
        slot: TimeslotIx,
        status: AvailabilityStatus,
    ) {
        let slots = self.slots;
        let table = match status {
            AvailabilityStatus::Unavailable => &mut self.unavailable,
            AvailabilityStatus::Preferred => &mut self.preferred,
            AvailabilityStatus::Discouraged => &mut self.discouraged,
        };
        let at = match table.binary_search_by(|held| held.entity.cmp(&entity)) {
            Ok(at) => at,
            Err(at) => {
                table.insert(
                    at,
                    EntitySlots {
                        entity,
                        slots: SlotMask::with_capacity(slots),
                    },
                );
                at
            }
        };
        table[at].slots.insert(slot);
    }

    /// What the rules say about one entity in one slot.
    ///
    /// `Unavailable` wins over both soft statuses: a slot both forbidden and preferred is
    /// forbidden, because a hard rule is not outweighed by a wish.
    #[must_use]
    pub fn status(&self, entity: AvailabilityEntity, slot: TimeslotIx) -> SlotAvailability {
        if Self::holds(&self.unavailable, entity, slot) {
            return SlotAvailability::Unavailable;
        }
        if Self::holds(&self.preferred, entity, slot) {
            return SlotAvailability::Preferred;
        }
        if Self::holds(&self.discouraged, entity, slot) {
            return SlotAvailability::Discouraged;
        }
        SlotAvailability::Unconstrained
    }

    /// The slots one entity may not be placed in.
    ///
    /// `None` when no rule restricts it, which is different from an empty mask only in
    /// storage: both mean "available everywhere".
    #[must_use]
    pub fn unavailable_slots(&self, entity: AvailabilityEntity) -> Option<&SlotMask> {
        Self::mask(&self.unavailable, entity)
    }

    /// Every entity carrying at least one hard restriction, in a deterministic order.
    pub fn restricted_entities(&self) -> impl Iterator<Item = AvailabilityEntity> + '_ {
        self.unavailable.iter().map(|held| held.entity)
    }

    /// One entity's mask in one table.
    fn mask(table: &[EntitySlots], entity: AvailabilityEntity) -> Option<&SlotMask> {
        table
            .binary_search_by(|held| held.entity.cmp(&entity))
            .ok()
            .map(|at| &table[at].slots)
    }

    /// Whether one table puts `entity` in `slot`.
    fn holds(table: &[EntitySlots], entity: AvailabilityEntity, slot: TimeslotIx) -> bool {
        Self::mask(table, entity).is_some_and(|mask| mask.contains(slot))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        AvailabilityEntity, AvailabilityEntityKind, AvailabilityScope, AvailabilityStatus,
        EffectiveWindow, SlotAvailability, SlotMask, TeacherIx, TimeslotIx, UnavailabilityIndex,
    };

    fn teacher(ix: u32) -> AvailabilityEntity {
        AvailabilityEntity::Teacher {
            teacher: TeacherIx::new(ix),
        }
    }

    fn slot(ix: u32) -> TimeslotIx {
        TimeslotIx::new(ix)
    }

    #[test]
    fn an_entity_with_no_rules_is_available_everywhere() {
        // V11's header: available is the default, so only exceptions are stored. This is the
        // assertion that stops a later reader "fixing" the empty case the other way.
        let index = UnavailabilityIndex::empty(10);
        assert_eq!(
            index.status(teacher(0), slot(3)),
            SlotAvailability::Unconstrained
        );
        assert!(index.status(teacher(0), slot(3)).permits_placement());
        assert!(index.unavailable_slots(teacher(0)).is_none());
    }

    #[test]
    fn unconstrained_is_not_the_same_state_as_preferred() {
        // The schema has no "explicitly available" status, so the absence of a rule is the
        // only way to say "nobody has an opinion". Collapsing the two loses that.
        assert_ne!(SlotAvailability::Unconstrained, SlotAvailability::Preferred);
        assert!(!SlotAvailability::Unconstrained.is_stated());
        assert!(SlotAvailability::Preferred.is_stated());
        assert!(SlotAvailability::Preferred.permits_placement());
    }

    #[test]
    fn an_unconstrained_slot_is_the_default_answer() {
        assert_eq!(SlotAvailability::default(), SlotAvailability::Unconstrained);
    }

    #[test]
    fn a_hard_rule_refuses_a_placement_and_the_soft_ones_do_not() {
        assert!(!SlotAvailability::Unavailable.permits_placement());
        assert!(SlotAvailability::Discouraged.permits_placement());
        assert!(AvailabilityStatus::Unavailable.is_hard());
        assert!(!AvailabilityStatus::Preferred.is_hard());
        assert!(!AvailabilityStatus::Discouraged.is_hard());
    }

    #[test]
    fn a_rule_binds_the_slot_it_names_and_no_other() {
        let mut index = UnavailabilityIndex::empty(10);
        index.insert(teacher(0), slot(4), AvailabilityStatus::Unavailable);
        assert_eq!(
            index.status(teacher(0), slot(4)),
            SlotAvailability::Unavailable
        );
        assert_eq!(
            index.status(teacher(0), slot(5)),
            SlotAvailability::Unconstrained,
            "a rule is compiled onto its own slots and widened to nothing"
        );
        assert_eq!(
            index.status(teacher(1), slot(4)),
            SlotAvailability::Unconstrained,
            "and it says nothing about another teacher"
        );
    }

    #[test]
    fn a_hard_rule_is_not_outweighed_by_a_wish() {
        let mut index = UnavailabilityIndex::empty(4);
        index.insert(teacher(0), slot(1), AvailabilityStatus::Preferred);
        index.insert(teacher(0), slot(1), AvailabilityStatus::Unavailable);
        assert_eq!(
            index.status(teacher(0), slot(1)),
            SlotAvailability::Unavailable
        );
    }

    #[test]
    fn restricted_entities_are_listed_deterministically() {
        let mut index = UnavailabilityIndex::empty(4);
        index.insert(teacher(2), slot(0), AvailabilityStatus::Unavailable);
        index.insert(teacher(0), slot(0), AvailabilityStatus::Unavailable);
        index.insert(teacher(1), slot(0), AvailabilityStatus::Unavailable);
        assert_eq!(
            index.restricted_entities().collect::<Vec<_>>(),
            vec![teacher(0), teacher(1), teacher(2)]
        );
    }

    #[test]
    fn a_period_scoped_rule_belongs_to_one_bell_schedule_and_a_day_scoped_one_does_not() {
        // ADR 0033 §7, which is the whole reason the scope is an enum rather than three
        // nullable fields.
        assert!(
            AvailabilityScope::Period {
                period: Uuid::nil()
            }
            .is_bell_schedule_bound(),
            "a period belongs to one period_structure"
        );
        assert!(
            AvailabilityScope::CycleDayAndPeriod {
                cycle_day: Uuid::nil(),
                period: Uuid::nil(),
            }
            .is_bell_schedule_bound()
        );
        assert!(
            AvailabilityScope::Timeslot {
                timeslot: Uuid::nil()
            }
            .is_bell_schedule_bound(),
            "one exact slot is one schedule and one term"
        );
        assert!(
            !AvailabilityScope::CycleDay {
                cycle_day: Uuid::nil()
            }
            .is_bell_schedule_bound(),
            "all of Monday means all of Monday on every schedule"
        );
    }

    #[test]
    fn an_unbounded_window_is_the_ordinary_state() {
        assert!(!EffectiveWindow::unbounded().is_bounded());
        assert_eq!(EffectiveWindow::default(), EffectiveWindow::unbounded());
        assert!(
            EffectiveWindow {
                from: Some("2027-01-05".into()),
                to: None,
            }
            .is_bounded()
        );
    }

    #[test]
    fn a_slot_mask_grows_past_its_first_word() {
        let mut mask = SlotMask::with_capacity(1);
        assert!(mask.is_empty());
        mask.insert(slot(130));
        assert!(mask.contains(slot(130)));
        assert!(!mask.contains(slot(129)));
        assert_eq!(mask.len(), 1);
        assert_eq!(mask.iter().collect::<Vec<_>>(), vec![slot(130)]);
    }

    #[test]
    fn the_stored_entity_kinds_are_all_representable() {
        // Six in the column, and the index compiles a subset deliberately. A rule naming an
        // uncompiled kind must still be nameable by a diagnostic.
        let kinds = [
            AvailabilityEntityKind::Teacher,
            AvailabilityEntityKind::Room,
            AvailabilityEntityKind::StudentGroup,
            AvailabilityEntityKind::Resource,
            AvailabilityEntityKind::Campus,
            AvailabilityEntityKind::Subject,
        ];
        assert_eq!(kinds.len(), 6);
        assert_eq!(
            serde_json::to_string(&AvailabilityEntityKind::StudentGroup)
                .expect("an entity kind serialises"),
            r#""STUDENT_GROUP""#
        );
    }
}
