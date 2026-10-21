//! The scheduling context: which school, cycle and term a solve is about, and which bell
//! schedules its slots were built from.
//!
//! # One school, one cycle, one term
//!
//! [ADR 0032](../../../../docs/adr/0032-timetable-generation-is-scoped-to-one-school.md)
//! rule 1: generation is scoped to one `(school, cycle, term)`. That is not a policy the
//! solver enforces, it is a shape it cannot violate — a [`Problem`](super::Problem) holds
//! one [`SchedulingContext`], so the solver never holds two schools' activities at once and
//! so cannot accidentally equate their cycle positions.
//!
//! # The bell schedules are derived, not configured
//!
//! [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
//! §1 defines `G` as *the distinct `period_structure` rows that have materialised `timeslot`
//! rows in `(cycle, term)`* — derived from what is materialised, never from what exists. A
//! bell schedule a school defined and never materialised into this grid has no slots, so it
//! is not a candidate for anything, which is correct because there is nothing to place a
//! lesson in. [`SchedulingContext::bell_schedules`] is that set.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::clock::TimeInterval;
use super::index::{BellScheduleIx, CampusIx, PeriodIx};

/// Which school, cycle and term a solve is about.
///
/// Every identifier here is named by the caller rather than defaulted. `cycle` in
/// particular is never guessed: `GridPreviewSection.tsx` already refuses to default it, and
/// a solve that picked one would be building a timetable over a grid the user did not
/// choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingContext {
    /// The school being solved. Every school-owned row in the problem belongs to it.
    pub school: SchoolFact,
    /// The cycle whose positions the timetable is defined over.
    pub cycle: CycleFact,
    /// The term the grid was materialised for.
    ///
    /// `None` means the grid that applies all year. It is part of timeslot identity
    /// (`idx_timeslot_identity` coalesces it), so it **selects** the grid rather than
    /// filtering one.
    pub term: Option<TermFact>,
    /// The campuses of this school.
    ///
    /// Carried because bell-schedule resolution reads the *count* as well as the
    /// identities: [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
    /// §2 clause 3 applies only at a school with exactly one campus, and that is a stored
    /// fact rather than an inference.
    pub campuses: Vec<CampusFact>,
    /// The cycle's positions, in ordinal order.
    pub cycle_days: Vec<CycleDayFact>,
    /// The bell schedules with slots materialised into this grid — `G` of ADR 0033 §1.
    pub bell_schedules: Vec<BellScheduleFact>,
    /// Every period of every schedule in [`SchedulingContext::bell_schedules`].
    ///
    /// Flat and shared, so an availability rule scoped to a `period_id` addresses one entry
    /// here and binds exactly the slots that period produces — never the same ordinal under
    /// another schedule (ADR 0033 §7).
    pub periods: Vec<PeriodFact>,
}

impl SchedulingContext {
    /// Whether the school has exactly one campus.
    ///
    /// The precondition of
    /// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
    /// §2 clause 3, which is the clause that makes the commonest configuration in the
    /// product — one campus, one campus-scoped bell schedule, sections leaving `campus_id`
    /// null because there is nothing to choose — resolve at all.
    #[must_use]
    pub fn is_single_campus(&self) -> bool {
        self.campuses.len() == 1
    }

    /// How many positions the cycle has.
    #[must_use]
    pub fn cycle_day_count(&self) -> usize {
        self.cycle_days.len()
    }
}

/// The school a solve is scoped to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the school is called, for a diagnostic that has to name it.
    pub name: String,
    /// The school's IANA timezone name, carried verbatim.
    ///
    /// **Nothing in this crate parses it**, and nothing in the MVP needs to: a solve is
    /// scoped to one school, so both sides of every interval comparison are in this zone
    /// and it cancels. It is carried so a report can say which zone the times are in, and
    /// so the field is visibly present rather than appearing to have been forgotten. ADR
    /// 0032 §"Two schools have no common clock" records that the workspace ships no
    /// timezone database at all, which is why cross-school comparison is deferred rather
    /// than approximated.
    pub timezone: String,
}

/// The cycle the timetable repeats over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the cycle is called.
    pub name: String,
    /// Whether its positions are named by the calendar or numbered.
    pub kind: CycleKind,
}

/// Whether a cycle's positions are weekdays or a rotation.
///
/// [ADR 0017](../../../../docs/adr/0017-weekly-and-custom-cycles.md). The distinction is
/// for display and for data entry; **nothing in scheduling reads it**, because a position's
/// `ordinal` is the whole of its scheduling identity either way. A six-day rotation and a
/// five-day week are the same kind of thing to every rule in this crate, which is what
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// test 14 and
/// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// test 16 exist to assert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CycleKind {
    /// Positions are named by weekday. All seven exist, always.
    Weekly,
    /// A rotation of `day_count` positions, named by the school.
    Custom,
}

/// The term the grid was materialised for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the term is called.
    pub name: String,
}

/// A campus of the school being solved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the campus is called.
    pub name: String,
}

/// One position in the cycle.
///
/// # Nothing here is a weekday
///
/// `ordinal` is a day's only scheduling identity. `label` is what the school calls it and
/// `weekday_hint` — which this type deliberately does **not** carry — is display. A rule
/// that read a weekday would break the moment a school adopted a six-day rotation, and
/// `CLAUDE.md` names reading one in scheduling as an architecture bug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleDayFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// Position within the cycle, from 1. The day's whole scheduling identity.
    pub ordinal: u16,
    /// What the school calls this day — "Monday", "Day A". For display only.
    pub label: String,
}

/// A bell schedule with slots in this grid: one `period_structure` row of `G`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BellScheduleFact {
    /// Stable identity, as stored.
    pub id: Uuid,
    /// What the schedule is called — quoted verbatim by `AmbiguousBellSchedule`, which has
    /// to list its candidates by name or the user cannot act on it.
    pub name: String,
    /// The campus this schedule serves, or `None` for a school-wide one.
    ///
    /// `period_structure.campus_id` is one nullable column and not an association table, so
    /// a schedule serves exactly one campus or every campus. A *subset* of campuses cannot
    /// be expressed; ADR 0033 records that limit and why it is not widened.
    pub campus: Option<CampusIx>,
    /// This schedule's periods, in ordinal order.
    pub periods: Vec<PeriodIx>,
}

impl BellScheduleFact {
    /// Whether this schedule serves every campus of the school.
    ///
    /// Clause 1 of
    /// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
    /// §2's compatibility relation.
    #[must_use]
    pub const fn is_school_wide(&self) -> bool {
        self.campus.is_none()
    }
}

/// What a period is for.
///
/// Derived from `period.kind`, whose `CHECK` admits exactly these four (`V4:106`).
/// `timeslot.is_teaching` is derived from `kind = 'TEACHING'` and from nothing else, so a
/// restriction on one and a constraint on the other cannot disagree — which is
/// `CLAUDE.md` §2's single most important rule applied to a domain restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PeriodKind {
    /// A lesson may be placed here.
    Teaching,
    /// A break. Breaks a run of teaching periods in two.
    Break,
    /// Registration or form time.
    Registration,
    /// Anything else the school timetables but does not teach in.
    Other,
}

impl PeriodKind {
    /// Whether a lesson may be placed in a period of this kind.
    ///
    /// Only [`PeriodKind::Teaching`]. This is the single source of `is_teaching`, and both
    /// the loader's value-range restriction and M5's `no-teaching-in-period` evaluator read
    /// it — ADR 0033 §4.
    #[must_use]
    pub const fn is_teaching(self) -> bool {
        matches!(self, Self::Teaching)
    }
}

/// One period of one bell schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodFact {
    /// Stable identity, as stored. An availability rule may be scoped to it.
    pub id: Uuid,
    /// The schedule this period belongs to.
    ///
    /// A period belongs to exactly one, which is why a `period_id`-scoped availability rule
    /// says something about one schedule and is never widened to another (ADR 0033 §7).
    pub bell_schedule: BellScheduleIx,
    /// Position within the schedule, from 1.
    ///
    /// **Not a time.** Two schedules may both carry `ordinal = 1` with different
    /// [`PeriodFact::interval`]s; the ordinal orders periods *within one schedule* and
    /// decides what "consecutive" means for a multi-period occurrence.
    pub ordinal: u16,
    /// What the school calls this period — "P1", "Registration".
    pub label: String,
    /// The wall-clock span, in the school's timezone.
    pub interval: TimeInterval,
    /// What the period is for.
    pub kind: PeriodKind,
}

/// Which bell schedule an activity's class section schedules against.
///
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §3's result. **There is no fallback, no default and no guess**: a section resolves to
/// exactly one schedule, or the solve is refused with a diagnostic that names the cause and
/// the remedy.
///
/// Resolution reads only the section, so every occurrence of a section resolves identically
/// and the question is asked once per section rather than once per activity.
///
/// The resolution *function* is not implemented in this milestone — it is M4's, and it needs
/// the loader's view of `G`. This type is the answer it produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum BellScheduleResolution {
    /// Exactly one compatible schedule. The only state a solve may proceed in.
    Resolved {
        /// The schedule this section's lessons draw their slots from.
        schedule: BellScheduleIx,
    },
    /// No compatible schedule. A hard pre-flight error; the solve is refused.
    None {
        /// Which of the three situations this is, because each has a different remedy.
        cause: NoBellScheduleCause,
    },
    /// More than one compatible schedule. A hard pre-flight error; the solve is refused.
    ///
    /// Evara will not choose. The diagnostic names **every** candidate, because a message
    /// that says "ambiguous" without saying between what is a message nobody can act on.
    Ambiguous {
        /// Every compatible schedule, in index order.
        candidates: Vec<BellScheduleIx>,
    },
}

impl BellScheduleResolution {
    /// The resolved schedule, when there is one.
    #[must_use]
    pub fn schedule(&self) -> Option<BellScheduleIx> {
        match self {
            Self::Resolved { schedule } => Some(*schedule),
            Self::None { .. } | Self::Ambiguous { .. } => None,
        }
    }

    /// Whether a solve may proceed for this section.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved { .. })
    }
}

/// Why a class section resolved to no bell schedule.
///
/// Three causes, three remedies —
/// [ADR 0033](../../../../docs/adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
/// §3's table. Carried separately from the diagnostic's text so the three cannot be merged
/// into one unhelpful sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoBellScheduleCause {
    /// `G` is empty: no grid has been materialised for this cycle and term.
    ///
    /// Raised **once for the solve** rather than once per section — see
    /// [`DiagnosticCode::NoGridMaterialised`](super::diagnostics::DiagnosticCode::NoGridMaterialised),
    /// which is the form pre-flight emits. This variant exists so a per-section resolution
    /// can still report the true cause when it is asked.
    GridEmpty,
    /// The section names a campus, and the grid holds neither a schedule for that campus nor
    /// a school-wide one.
    ///
    /// Remedy: build the grid from that campus's bell schedule, or make a schedule
    /// school-wide.
    NoScheduleForCampus,
    /// The section names no campus, the school has more than one, and the grid holds no
    /// school-wide schedule.
    ///
    /// Remedy: give the section a campus, or make a bell schedule school-wide. This is the
    /// cost ADR 0033 takes deliberately: a multi-campus school finishes saying which campus
    /// teaches what, rather than getting a timetable built on a guess.
    SectionHasNoCampus,
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        BellScheduleFact, BellScheduleIx, BellScheduleResolution, CampusFact, CampusIx,
        CycleDayFact, CycleFact, CycleKind, NoBellScheduleCause, PeriodKind, SchedulingContext,
        SchoolFact,
    };

    fn context(campuses: usize) -> SchedulingContext {
        SchedulingContext {
            school: SchoolFact {
                id: Uuid::nil(),
                name: "Northgate".into(),
                timezone: "Europe/London".into(),
            },
            cycle: CycleFact {
                id: Uuid::nil(),
                name: "Standard week".into(),
                kind: CycleKind::Weekly,
            },
            term: None,
            campuses: (0..campuses)
                .map(|n| CampusFact {
                    id: Uuid::nil(),
                    name: format!("Campus {n}"),
                })
                .collect(),
            cycle_days: vec![CycleDayFact {
                id: Uuid::nil(),
                ordinal: 1,
                label: "Day 1".into(),
            }],
            bell_schedules: Vec::new(),
            periods: Vec::new(),
        }
    }

    #[test]
    fn a_single_campus_school_is_recognised_from_the_stored_count() {
        // ADR 0033 §2 clause 3 turns on this and on nothing inferred.
        assert!(context(1).is_single_campus());
        assert!(!context(2).is_single_campus());
        assert!(
            !context(0).is_single_campus(),
            "a school with no campus is not a school with one"
        );
    }

    #[test]
    fn a_school_wide_schedule_is_the_one_with_no_campus() {
        let school_wide = BellScheduleFact {
            id: Uuid::nil(),
            name: "Standard Day".into(),
            campus: None,
            periods: Vec::new(),
        };
        let campus_scoped = BellScheduleFact {
            campus: Some(CampusIx::new(0)),
            ..school_wide.clone()
        };
        assert!(school_wide.is_school_wide());
        assert!(!campus_scoped.is_school_wide());
    }

    #[test]
    fn only_a_teaching_period_may_hold_a_lesson() {
        assert!(PeriodKind::Teaching.is_teaching());
        for kind in [
            PeriodKind::Break,
            PeriodKind::Registration,
            PeriodKind::Other,
        ] {
            assert!(!kind.is_teaching(), "{kind:?} is not a teaching period");
        }
    }

    #[test]
    fn only_a_unique_resolution_lets_a_solve_proceed() {
        let resolved = BellScheduleResolution::Resolved {
            schedule: BellScheduleIx::new(0),
        };
        assert!(resolved.is_resolved());
        assert_eq!(resolved.schedule(), Some(BellScheduleIx::new(0)));

        let none = BellScheduleResolution::None {
            cause: NoBellScheduleCause::SectionHasNoCampus,
        };
        assert!(!none.is_resolved());
        assert_eq!(none.schedule(), None);

        let ambiguous = BellScheduleResolution::Ambiguous {
            candidates: vec![BellScheduleIx::new(0), BellScheduleIx::new(1)],
        };
        assert!(!ambiguous.is_resolved());
        assert_eq!(
            ambiguous.schedule(),
            None,
            "Evara will not choose between two candidates"
        );
    }

    #[test]
    fn an_ambiguous_resolution_carries_every_candidate() {
        // A message that says "ambiguous" without saying between what cannot be acted on.
        let ambiguous = BellScheduleResolution::Ambiguous {
            candidates: vec![BellScheduleIx::new(0), BellScheduleIx::new(3)],
        };
        let BellScheduleResolution::Ambiguous { candidates } = &ambiguous else {
            panic!("constructed as ambiguous");
        };
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn the_three_causes_of_no_schedule_stay_distinct() {
        // Three causes, three remedies — ADR 0033 §3. Merging them produces a sentence the
        // user cannot act on.
        let causes = [
            NoBellScheduleCause::GridEmpty,
            NoBellScheduleCause::NoScheduleForCampus,
            NoBellScheduleCause::SectionHasNoCampus,
        ];
        for (i, a) in causes.iter().enumerate() {
            for b in &causes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
