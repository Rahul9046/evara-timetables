//! The constraint vocabulary: what a rule *is*, what breaking one looks like, and how that is
//! explained.
//!
//! # Why this is in `evara-core` and the evaluators are not
//!
//! `evara-constraints` owns the evaluators and the registry — ADR 0005, and
//! [`SOLVER.md`](../../../../docs/SOLVER.md) §4 names the file. It cannot own these *types*,
//! because three crates that cannot depend on it have to agree on them:
//!
//! | Crate | Needs |
//! | --- | --- |
//! | `evara-constraints` | all of it — it produces [`Violation`]s and [`Explanation`]s |
//! | `evara-solver` | [`Violation`], [`Score`], [`AffectedSet`] — it searches on them |
//! | `evara-db` | [`CompiledConstraint`] to produce, and [`Violation`] to persist in M7 |
//! | `evara-report` | [`Explanation`] to render |
//!
//! `evara-db` depends on `evara-core` and **not** on `evara-constraints` (`ARCHITECTURE.md`
//! §3), so a vocabulary shared by the loader and the evaluators has to sit underneath both.
//! That is the same argument that put `scheduling::Problem` here in M2 and the overlap
//! evaluator here in M5.
//!
//! # No JSON crosses this boundary
//!
//! `evara-core` has `serde_json` as a *dev*-dependency only, and that is deliberate rather
//! than incidental: it means [`CompiledConstraint`] cannot hold a `serde_json::Value`, so the
//! JSON Schema boundary is forced into `evara-constraints`, which owns the schemas.
//!
//! The consequence is the shape the brief asked for — *"strongly typed internal configuration
//! validated against a JSON Schema boundary"*. Stored `params` are parsed and validated once,
//! in `evara-constraints`, into the typed [`ConstraintOptions`] below; nothing downstream ever
//! re-parses a string, and a malformed parameter is an error at the boundary rather than a
//! surprise in an evaluator.
//!
//! # What is deliberately absent
//!
//! No weights-as-floats: `CLAUDE.md` §3 requires integer score arithmetic, so a weight is an
//! `i64` and a [`Violation`] carries the integer penalty it produced — *"multiplied into an
//! integer penalty at compile time, not at scoring time"* (`SOLVER.md` §3).
//!
//! No soft-constraint machinery. [`ConstraintLevel::Soft`] exists because a level is stored
//! per instance and a school may run a rule at any of the three, but no soft type is
//! implemented in the MVP and nothing here optimises.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::clock::TimeInterval;
use super::diagnostics::{EntityKind, EntityRef};
use super::index::{ActivityIx, CycleDayIx};
use super::outcome::Score;

/// A constraint type the engine implements.
///
/// # An enum, and why the key is spelled out rather than derived
///
/// `type_key` is what a `constraint_instance` row stores, so it is a contract: added to, never
/// renamed. Deriving it from a Rust type name would make a refactor a silent data migration,
/// which is why [`ConstraintType::key`] is an explicit `match` and a test asserts the keys are
/// distinct and kebab-case. This is exactly the discipline
/// [`DiagnosticCode`](super::DiagnosticCode) already follows, down to the serde representation.
///
/// # This is the *implemented* set, not the specified one
///
/// [`CONSTRAINTS.md`](../../../../docs/CONSTRAINTS.md) §3 specifies about sixty types. Eight
/// are implemented, and only those appear here: a variant with no evaluator would violate
/// `CLAUDE.md` §4's rule that no type is supported until all five of its artefacts exist, and
/// a registered key that never fires *"tells the user their rooms are being checked"* when
/// they are not.
///
/// So a stored row naming anything else is an **unknown type**, reported as such. That is the
/// honest answer while the set is small, and the enum grows one variant per type as the
/// evaluators land.
///
/// # `activity-fixed` is the eighth hard type and is not here
///
/// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's hard set is seven of these plus `activity-fixed`
/// *"once locks exist"*. `activity.is_locked` does not exist at `V16` — the roadmap keeps lock
/// toggles with Phase 8's manual editing, because a lock is about a *placed* timetable rather
/// than about the demand — and [`Problem::pinned`](super::Problem) is the declared hook that
/// nothing populates. Adding a variant for it would be adding a rule that cannot fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConstraintType {
    /// A teacher may not be in two activities at overlapping times.
    TeacherClash,
    /// Two activities may not overlap in time when their audiences confirmedly overlap.
    StudentClash,
    /// A room may not host two activities at overlapping wall-clock times.
    RoomClash,
    /// An assigned room must match the required room type.
    RoomTypeRequired,
    /// A placement must not fall where a hard availability rule forbids it.
    EntityUnavailable,
    /// A multi-period occurrence occupies consecutive teaching periods of one day.
    DurationContiguous,
    /// A lesson may not occupy a period that is not for teaching.
    NoTeachingInPeriod,
    /// Every activity in scope must be placed, exactly once.
    ActivityScheduled,
}

impl ConstraintType {
    /// The stored identity.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::TeacherClash => "teacher-clash",
            Self::StudentClash => "student-clash",
            Self::RoomClash => "room-clash",
            Self::RoomTypeRequired => "room-type-required",
            Self::EntityUnavailable => "entity-unavailable",
            Self::DurationContiguous => "duration-contiguous",
            Self::NoTeachingInPeriod => "no-teaching-in-period",
            Self::ActivityScheduled => "activity-scheduled",
        }
    }

    /// Reads a stored `type_key`.
    ///
    /// `None` for anything this build does not implement, which the caller reports as an
    /// unknown type rather than ignoring. A project written by a newer Evara keeps rules this
    /// one cannot evaluate; it must not pretend to evaluate them.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|kind| kind.key() == key)
    }

    /// Every implemented type, in evaluation order.
    ///
    /// The order is the order violations are reported in, and it is chosen so a reader meets
    /// causes before consequences: the four clash and compatibility rules about *where* a
    /// lesson sits, then the two about *when* it may sit there, then whether it sits anywhere
    /// at all.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::TeacherClash,
            Self::StudentClash,
            Self::RoomClash,
            Self::RoomTypeRequired,
            Self::EntityUnavailable,
            Self::DurationContiguous,
            Self::NoTeachingInPeriod,
            Self::ActivityScheduled,
        ]
    }

    /// The level this type ships at.
    ///
    /// **A default, not a law.** ADR 0005 stores the level per instance precisely because
    /// *"schools genuinely disagree about which rules are hard, and that disagreement is the
    /// product"*. What a *mandatory* type may not do is drop below this — see
    /// [`ConstraintType::is_mandatory`].
    ///
    /// `activity-scheduled` is **MEDIUM** and the other seven are HARD, which is
    /// `SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's split exactly. The level is what lets M6
    /// return a usable partial timetable and say what it could not place: an unplaced activity
    /// must not make every intermediate construction state infeasible.
    #[must_use]
    pub const fn default_level(self) -> ConstraintLevel {
        match self {
            Self::ActivityScheduled => ConstraintLevel::Medium,
            Self::TeacherClash
            | Self::StudentClash
            | Self::RoomClash
            | Self::RoomTypeRequired
            | Self::EntityUnavailable
            | Self::DurationContiguous
            | Self::NoTeachingInPeriod => ConstraintLevel::Hard,
        }
    }

    /// Whether the engine always evaluates this type, whatever the database holds.
    ///
    /// All eight are, and that is the answer to the question the brief's §8 asks: the MVP set
    /// is **mandatory in the registry, not materialised as rows**. Evaluation runs them
    /// because they are in [`ConstraintType::all`], so deleting a `constraint_instance` cannot
    /// disable a safety rule — there is no row to delete. See ADR 0039.
    ///
    /// A stored instance may still *tune* a mandatory type — a weight, a term — but may not
    /// disable it or weaken its level, and the repository refuses an attempt rather than
    /// accepting one that would do nothing.
    #[must_use]
    pub const fn is_mandatory(self) -> bool {
        true
    }

    /// Whether this type can be narrowed to particular entities by `constraint_ref` rows.
    ///
    /// None of the eight can. Each is a property of the whole timetable — a teacher clash is a
    /// clash wherever it happens — so a `constraint_ref` narrowing one would either do nothing
    /// or silently switch a safety rule off for part of a school.
    ///
    /// Stated as a method rather than assumed, because the refusal has to be explicit:
    /// compiling a narrowed instance of one of these is an error, not a scope quietly ignored.
    /// A later type that *is* narrowable returns `true` and the compiler makes every match
    /// arm reconsider.
    #[must_use]
    pub const fn supports_narrowing(self) -> bool {
        false
    }

    /// Whether a run of periods may cross a non-teaching one, for the type that asks.
    ///
    /// Only `duration-contiguous` reads [`ConstraintOptions`], and this says so in one place
    /// rather than leaving each evaluator to decide whether its options mean anything.
    #[must_use]
    pub const fn reads_options(self) -> bool {
        matches!(self, Self::DurationContiguous)
    }
}

/// How much a violation matters.
///
/// Three levels, compared lexicographically, `Hard` first — the order a report is printed in
/// and the order [`Score`] compares. Declaration order *is* the ordering, as it is for
/// [`Severity`](super::Severity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstraintLevel {
    /// A timetable violating this is not usable. Must reach zero.
    Hard,
    /// Must hold unless the alternative is no timetable at all.
    ///
    /// Unscheduled activities, unmet period counts, unstaffed classes. `CONSTRAINTS.md` §1:
    /// *"It lets the solver return a usable partial timetable and tell the user exactly what
    /// it could not place, instead of returning nothing."*
    Medium,
    /// A preference. Quality, not correctness. Nothing in the MVP is scored at this level.
    Soft,
}

impl ConstraintLevel {
    /// Whether a violation at this level makes a timetable unusable.
    #[must_use]
    pub const fn blocks_feasibility(self) -> bool {
        matches!(self, Self::Hard)
    }

    /// Reads a stored `level`.
    #[must_use]
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "HARD" => Some(Self::Hard),
            "MEDIUM" => Some(Self::Medium),
            "SOFT" => Some(Self::Soft),
            _ => None,
        }
    }

    /// The value a `constraint_instance` row stores.
    #[must_use]
    pub const fn stored(self) -> &'static str {
        match self {
            Self::Hard => "HARD",
            Self::Medium => "MEDIUM",
            Self::Soft => "SOFT",
        }
    }

    /// `penalty` added to this level's field of a score, the other two untouched.
    #[must_use]
    pub const fn penalty(self, penalty: i64) -> Score {
        match self {
            Self::Hard => Score::new(penalty, 0, 0),
            Self::Medium => Score::new(0, penalty, 0),
            Self::Soft => Score::new(0, 0, penalty),
        }
    }
}

/// The typed parameters a constraint was configured with.
///
/// # One field, because the MVP has one parameter
///
/// Seven of the eight implemented types take no parameters at all, and `duration-contiguous`
/// takes exactly one — `CONSTRAINTS.md` describes it as *"consecutive periods on one day, not
/// spanning a break unless permitted"*, and this is the permission.
///
/// A struct rather than an enum of eight variants, seven of them empty. When a second
/// parameterised type arrives it adds a field; when that becomes unwieldy it becomes an enum,
/// and the typed boundary means that change is a compiler error rather than a silent
/// misreading. The JSON Schema each type publishes lives beside its evaluator in
/// `evara-constraints`, which is the only place that parses a stored `params` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintOptions {
    /// Whether a multi-period occurrence may span a non-teaching period.
    ///
    /// `false` by default, which is the stricter reading and the one ADR 0033 §5 describes: a
    /// run is broken by any non-teaching period between two teaching ones. A school that
    /// genuinely timetables a double across morning break sets this.
    pub allow_break_span: bool,
}

/// What a constraint instance was narrowed to.
///
/// Compiled from `constraint_ref` rows and the instance's `term_id`. Empty for every mandatory
/// built-in, which is what makes the MVP's rules school-wide.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintScope {
    /// The term it applies to, or `None` for the whole year.
    pub term: Option<Uuid>,
    /// The entities it is narrowed to. Empty means school-wide.
    pub applies_to: Vec<ScopeRef>,
}

impl ConstraintScope {
    /// A scope that narrows nothing.
    #[must_use]
    pub const fn everything() -> Self {
        Self {
            term: None,
            applies_to: Vec::new(),
        }
    }

    /// Whether this narrows the rule to particular entities.
    #[must_use]
    pub fn is_narrowed(&self) -> bool {
        !self.applies_to.is_empty()
    }
}

/// One entity a constraint is scoped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeRef {
    /// What kind of row it names.
    pub kind: EntityKind,
    /// Which row.
    pub id: Uuid,
}

/// A constraint ready to evaluate: its type, its level, its weight and its scope.
///
/// The compiled form of a `constraint_instance` row, or of a mandatory built-in that has no
/// row at all. `SOLVER.md` §2 noted this type as *"not in the implemented `Problem`: the
/// registry is M5's"* — it arrives here with the registry that reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledConstraint {
    /// The stored row this came from, or `None` for a mandatory built-in.
    ///
    /// `None` is the ordinary case in the MVP and is not a missing value: the eight types are
    /// mandatory in the registry, so an unconfigured project evaluates all of them and has no
    /// rows at all. A violation carries this through so a future screen can link a finding to
    /// the instance that produced it, where there is one.
    pub id: Option<Uuid>,
    /// Which rule.
    pub constraint_type: ConstraintType,
    /// The level it is being evaluated at.
    pub level: ConstraintLevel,
    /// The integer penalty one violation of it contributes.
    pub weight: i64,
    /// Its typed parameters.
    pub options: ConstraintOptions,
    /// What it was narrowed to.
    pub scope: ConstraintScope,
}

impl CompiledConstraint {
    /// A mandatory built-in at its shipped level, with no stored row and no narrowing.
    ///
    /// What the registry hands an evaluator for a project that has configured nothing, which
    /// is every project today.
    #[must_use]
    pub fn mandatory(constraint_type: ConstraintType) -> Self {
        Self {
            id: None,
            constraint_type,
            level: constraint_type.default_level(),
            weight: 1,
            options: ConstraintOptions::default(),
            scope: ConstraintScope::everything(),
        }
    }

    /// Whether this constraint applies to the term a problem is about.
    ///
    /// `None` applies to every term. A stored instance naming a different term applies to
    /// none of this solve, and the registry drops it rather than evaluating it — which is
    /// the same decision the availability compiler makes about a rule for another term.
    #[must_use]
    pub fn applies_to_term(&self, term: Option<Uuid>) -> bool {
        self.scope.term.is_none() || self.scope.term == term
    }
}

/// One way a candidate timetable breaks one rule.
///
/// # Structured, never a string
///
/// `SOLVER.md` §6: *"An `Explanation` is structured, not a string"*, and the same is true one
/// level down. A violation names the activities at fault, the entity the rule is about and the
/// entities involved, and when it happens — enough for a future grid to highlight the right
/// cells without parsing prose, and enough for [`Explanation`] to write the prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    /// Which rule was broken.
    pub constraint_type: ConstraintType,
    /// At what level, which is the instance's level and not necessarily the shipped default.
    pub level: ConstraintLevel,
    /// The stored instance that produced it, where there is one.
    pub constraint_id: Option<Uuid>,
    /// The integer penalty it contributes, already multiplied by the instance's weight.
    pub penalty: i64,
    /// The occurrences at fault, in index order.
    ///
    /// Two for a clash, one for everything else. Sorted, so a pair is reported the same way
    /// whichever of the two the evaluator met first.
    pub activities: Vec<ActivityIx>,
    /// What the rule is about: the teacher, the room, the section, the activity.
    pub subject: EntityRef,
    /// Everything else a reader needs to see, in a stable order.
    pub related: Vec<EntityRef>,
    /// The cycle day it happens on, where the violation has a when.
    pub cycle_day: Option<CycleDayIx>,
    /// The wall-clock span involved, where the violation has one.
    ///
    /// The clash key for shared occupancy is an interval and never a timeslot identifier
    /// (ADR 0032 rule 2), so a clash violation carries the interval it was decided on.
    pub span: Option<TimeInterval>,
    /// The specific fact, for a message that has to be more than the rule's name.
    pub detail: String,
}

impl Violation {
    /// A deduplication key: the rule, the occurrences, and what it is about.
    ///
    /// Two evaluators cannot produce the same violation — each owns its type — but one
    /// evaluator can meet the same pair twice while sweeping, and a caller merging reports
    /// from a full and an incremental pass needs to recognise a repeat. The activity list is
    /// already sorted, so the key is stable.
    #[must_use]
    pub fn key(&self) -> ViolationKey {
        ViolationKey {
            constraint_type: self.constraint_type,
            activities: self.activities.clone(),
            subject: self.subject.id,
        }
    }

    /// The score this violation contributes.
    #[must_use]
    pub const fn score(&self) -> Score {
        self.level.penalty(self.penalty)
    }
}

/// What makes two violations the same violation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViolationKey {
    /// Which rule.
    pub constraint_type: ConstraintType,
    /// The occurrences, sorted.
    pub activities: Vec<ActivityIx>,
    /// What it is about.
    pub subject: Uuid,
}

/// Where an evaluator puts what it finds.
///
/// A sink rather than a returned `Vec` because `SOLVER.md` §4's `evaluate` signature takes one:
/// an evaluator sweeping a grid produces violations as it goes, and the caller decides whether
/// to collect them all, stop at the first, or merge several evaluators' output in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViolationSink {
    found: Vec<Violation>,
}

impl ViolationSink {
    /// An empty sink.
    #[must_use]
    pub const fn new() -> Self {
        Self { found: Vec::new() }
    }

    /// Records a violation.
    pub fn push(&mut self, violation: Violation) {
        self.found.push(violation);
    }

    /// What was found, in the order it was found.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.found
    }

    /// Takes what was found.
    #[must_use]
    pub fn into_violations(self) -> Vec<Violation> {
        self.found
    }

    /// How many were found.
    #[must_use]
    pub fn len(&self) -> usize {
        self.found.len()
    }

    /// Whether nothing was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    /// How many at one level.
    #[must_use]
    pub fn count_at(&self, level: ConstraintLevel) -> usize {
        self.found.iter().filter(|v| v.level == level).count()
    }

    /// The total score of everything found.
    #[must_use]
    pub fn score(&self) -> Score {
        self.found
            .iter()
            .fold(Score::PERFECT, |total, found| total.plus(found.score()))
    }

    /// Removes repeats, keeping the first of each.
    ///
    /// Order-preserving, so the result is still deterministic. Linear scans over a short list
    /// rather than a hash set, for the reason `duplicate_ids` in
    /// [`problem`](super::problem) uses one: a `HashSet`'s iteration order would leak into
    /// nothing here, but the list is short and the simpler code is easier to trust.
    pub fn deduplicate(&mut self) {
        let mut seen: Vec<ViolationKey> = Vec::new();
        self.found.retain(|found| {
            let key = found.key();
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
    }
}

/// A human-readable reason, structured so the frontend renders rather than authors it.
///
/// `SOLVER.md` §6's type. Two refinements to the shape declared there, both deliberate:
///
/// - `constraint_type` is a [`ConstraintType`] rather than a `String`. The runtime cost is nil
///   and a mistyped key becomes a compile error — the same sharpening M2 applied when it made
///   the dense indices newtypes over `u32` rather than bare `u32`s.
/// - `conflicting_entries` is named `conflicting_activities` and holds `activity.id`s.
///   `timetable_entry` does not exist until M7, so there are no entry identifiers to carry;
///   the activities are what a grid can highlight today, and M7 widens this rather than
///   reinterpreting it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    /// Which rule.
    pub constraint_type: ConstraintType,
    /// At what level.
    pub level: ConstraintLevel,
    /// The sentence, naming the entities involved.
    pub message: String,
    /// Clickable in the interface.
    pub entities: Vec<EntityRef>,
    /// Stored identifiers a grid can highlight.
    pub conflicting_activities: Vec<Uuid>,
}

impl Explanation {
    /// Whether this explanation describes something that makes a timetable unusable.
    #[must_use]
    pub const fn blocks(&self) -> bool {
        self.level.blocks_feasibility()
    }
}

/// The occurrences whose score a change can alter.
///
/// `SOLVER.md` §4's `affected`, which exists to scope incremental re-evaluation: moving one
/// activity cannot change a `room-clash` verdict about two activities in a different room, and
/// M6 must not re-evaluate the whole grid to discover that.
///
/// Returned rather than applied, so M5 computes it and M6 decides what to do with it. Nothing
/// in this milestone maintains an incremental index — `SOLVER.md` §4's whole argument is that
/// the reference implementation keeps the incremental one honest, and the reference one comes
/// first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedSet {
    /// The occurrences, sorted and deduplicated. Empty means nothing is affected.
    activities: Vec<ActivityIx>,
    /// Whether every occurrence is affected, whatever `activities` holds.
    everything: bool,
}

impl AffectedSet {
    /// Nothing is affected.
    #[must_use]
    pub const fn nothing() -> Self {
        Self {
            activities: Vec::new(),
            everything: false,
        }
    }

    /// Every occurrence is affected.
    ///
    /// What a rule about the timetable *as a whole* returns. `activity-scheduled` is the one
    /// in the MVP: placing anything changes how many things are unplaced.
    #[must_use]
    pub const fn everything() -> Self {
        Self {
            activities: Vec::new(),
            everything: true,
        }
    }

    /// Exactly these occurrences.
    #[must_use]
    pub fn of(mut activities: Vec<ActivityIx>) -> Self {
        activities.sort_unstable();
        activities.dedup();
        Self {
            activities,
            everything: false,
        }
    }

    /// Whether this names every occurrence.
    #[must_use]
    pub const fn is_everything(self_: &Self) -> bool {
        self_.everything
    }

    /// The occurrences named, which is empty when [`AffectedSet::everything`] was returned.
    #[must_use]
    pub fn activities(&self) -> &[ActivityIx] {
        &self.activities
    }

    /// Whether `activity` is affected.
    #[must_use]
    pub fn contains(&self, activity: ActivityIx) -> bool {
        self.everything || self.activities.binary_search(&activity).is_ok()
    }
}

/// A constraint that cannot be compiled from what is stored.
///
/// Every variant is a refusal rather than a fallback. `CLAUDE.md`'s rule about persisted values
/// applies to configuration as much as to identifiers: a parameter that cannot be read is not a
/// parameter to guess at, and silently evaluating a rule at the wrong level or over the wrong
/// scope is worse than refusing to evaluate it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConstraintError {
    /// The stored `type_key` is not one this build implements.
    #[error("`{key}` is not a constraint type this version of Evara implements")]
    UnknownType {
        /// What was stored.
        key: String,
    },
    /// The stored `level` is not one of the three.
    #[error("`{level}` is not a constraint level; expected HARD, MEDIUM or SOFT")]
    UnknownLevel {
        /// What was stored.
        level: String,
    },
    /// A mandatory type was stored disabled, or below the level it ships at.
    ///
    /// Refused rather than honoured: these are the safety rules, and `enabled = 0` on one would
    /// be a timetable silently stopping being checked for clashes. ADR 0039 §3.
    #[error(
        "`{key}` is a mandatory rule and cannot be disabled or weakened below {shipped}; \
         it was stored as {stored}"
    )]
    MandatoryWeakened {
        /// Which rule.
        key: &'static str,
        /// The level it ships at.
        shipped: &'static str,
        /// What the row asked for.
        stored: String,
    },
    /// A type that cannot be narrowed was given `constraint_ref` rows.
    ///
    /// Refused rather than ignored, because ignoring it would leave a school believing a rule
    /// had been scoped when it had not.
    #[error("`{key}` applies to the whole timetable and cannot be narrowed to particular entities")]
    ScopeNotSupported {
        /// Which rule.
        key: &'static str,
    },
    /// The stored `params` are not what the type's schema requires.
    #[error("the parameters stored for `{key}` are not valid: {reason}")]
    InvalidParams {
        /// Which rule.
        key: &'static str,
        /// What was wrong with them.
        reason: String,
    },
}

#[cfg(test)]
mod tests;
