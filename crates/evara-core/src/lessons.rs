//! Pure rules about a lesson's demand: how much of a cycle it asks for, and whether the
//! school's time model has room for one occurrence of it at all.
//!
//! # Why this is not in `evara-db`, `evara-constraints` or the interface
//!
//! "Does this class section demand more periods than the cycle has?" is a question about
//! the scheduling model. Answering it in SQL would scatter it across queries; answering it
//! in TypeScript would put a scheduling rule in the view layer, which `CLAUDE.md` names as
//! an architecture bug. So it sits here, at the bottom of the stack, as a pure function
//! over counts — the same place and for the same reason as [`crate::time::CycleCoverage`].
//!
//! It is deliberately **not** in `evara-constraints`. That crate is Phase 4's, and what
//! lives in it is a registry of `type_key`-addressed evaluators with JSON Schemas and
//! `explain()` implementations, each scored against a candidate timetable
//! (`CLAUDE.md` §4). Nothing here is evaluated against a timetable: these are
//! data-entry rules about whether the demand a school has just typed in is coherent at all,
//! which is a question with an answer before any placement exists. Putting them in the
//! constraint registry would mean either five incomplete constraint types a stage early,
//! or a second kind of thing living in a crate whose whole value is that it holds one
//! kind.
//!
//! # What "fits" can and cannot mean
//!
//! Everything in this module distinguishes two findings, and the distinction is the point:
//!
//! - **Structurally impossible.** A three-period occurrence in a bell schedule whose
//!   longest unbroken run of teaching periods is two can never be placed, in any
//!   timetable, by any solver. That is arithmetic, and saying so is honest.
//! - **Merely demanding.** Twenty-eight periods of demand in a thirty-period cycle is
//!   probably hard and might be impossible once clashes, rooms and staff availability are
//!   accounted for. Nothing here claims to know, because only a solver can answer it.
//!
//! No function in this module ever reports that something *is* schedulable. ADR 0029 §6
//! records why that restraint matters: a screen that says "this will work" and is then
//! contradicted by the solver has spent the user's trust on a guess.

use serde::{Deserialize, Serialize};

/// What one class section asks of a cycle.
///
/// Built from the section's activity rows, each of which is **one occurrence** — see
/// [ADR 0029](../../../docs/adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md).
/// The arithmetic is therefore a sum rather than a product, and the distinction matters:
/// a section with four single periods and one double has five occurrences and six periods
/// of demand, which no single multiplication produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// No `#[ts(export)]`: that would make `cargo test` write TypeScript into the working tree
// as a side effect. `evara-typegen` exports deliberately instead — see ADR 0012.
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LessonDemand {
    /// How many separate occurrences have to be placed in one cycle.
    ///
    /// The count of the section's activity rows. This is the number a user means by
    /// "times per cycle", and it is derived rather than stored precisely so that it cannot
    /// disagree with the rows it describes.
    pub occurrences: i64,
    /// How many period slots those occurrences consume in total, per cycle.
    ///
    /// `SUM(duration_periods)`. Four singles and one double is 6, not 5 and not 8.
    pub periods_per_cycle: i64,
    /// The longest single occurrence, in consecutive periods.
    ///
    /// Carried because it is the one figure the structural check needs: a cycle has room
    /// for the *total* or it does not, but an individual occurrence needs an unbroken run
    /// and that is a different question with a different answer.
    pub longest_occurrence: i64,
}

impl LessonDemand {
    /// Sums the durations of one section's occurrences.
    ///
    /// `durations` is one entry per activity row, in any order. An empty section demands
    /// nothing, which is the ordinary state of a section somebody has just created and is
    /// not an error anywhere.
    #[must_use]
    pub fn of(durations: &[i64]) -> Self {
        Self {
            occurrences: i64::try_from(durations.len()).unwrap_or(i64::MAX),
            periods_per_cycle: durations.iter().copied().map(|d| d.max(0)).sum(),
            longest_occurrence: durations.iter().copied().max().unwrap_or(0),
        }
    }

    /// Whether this section asks for nothing yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.occurrences == 0
    }

    /// How this demand compares with what its course declared.
    ///
    /// `declared` is `course.periods_per_cycle`, or `None` for a course that has not said.
    /// A difference is **not** an error: a section may legitimately need more or less than
    /// its course's default, and ADR 0029 §3 keeps the declaration as advice rather than a
    /// constraint for exactly that reason. The interface reports it; nothing refuses it.
    #[must_use]
    pub const fn against_declared(&self, declared: Option<i64>) -> DeclaredComparison {
        match declared {
            None => DeclaredComparison::NotDeclared,
            Some(declared) if declared == self.periods_per_cycle => DeclaredComparison::Matches,
            Some(declared) if declared > self.periods_per_cycle => DeclaredComparison::Short {
                declared,
                shortfall: declared - self.periods_per_cycle,
            },
            Some(declared) => DeclaredComparison::Over {
                declared,
                excess: self.periods_per_cycle - declared,
            },
        }
    }
}

/// How a section's actual demand stands against its course's declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum DeclaredComparison {
    /// The course has no `periodsPerCycle`, so there is nothing to compare against.
    NotDeclared,
    /// The section's occurrences total exactly what the course asked for.
    Matches,
    /// The section asks for fewer periods than its course declared.
    Short {
        /// `course.periodsPerCycle`.
        declared: i64,
        /// How many more periods of occurrences would meet the declaration.
        shortfall: i64,
    },
    /// The section asks for more periods than its course declared.
    Over {
        /// `course.periodsPerCycle`.
        declared: i64,
        /// How many periods beyond the declaration this section demands.
        excess: i64,
    },
}

/// The shape of the time model a demand has to fit into.
///
/// Expressed as plain counts so that nothing here needs to know what a cycle day or a bell
/// schedule *is*. The repository reads them; this module does the arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CycleCapacity {
    /// How many days the cycle has rows for. `0` when the school has no cycle yet.
    pub cycle_days: i64,
    /// The most teaching periods any one of the school's bell schedules offers in a day.
    ///
    /// The *most*, not the fewest, because an occurrence only has to fit somewhere. A
    /// school with a short Friday has not thereby made a double period impossible.
    pub teaching_periods_per_day: i64,
    /// The longest unbroken run of teaching periods in any one day.
    ///
    /// Distinct from [`Self::teaching_periods_per_day`] and the figure a multi-period
    /// occurrence actually needs: a day of six teaching periods split by a break after the
    /// third offers no run longer than three, so a four-period occurrence cannot be placed
    /// in it however many teaching periods the day contains in total.
    pub longest_teaching_run: i64,
}

impl CycleCapacity {
    /// Measures the capacity from the school's bell schedules.
    ///
    /// `schedules` is one entry per bell schedule, each listing its periods **in ordinal
    /// order**, `true` for a teaching period. The repository reads the flags; the counting
    /// — and in particular "longest unbroken run", which SQL expresses badly and Rust
    /// expresses in four lines — happens here, where it can be tested without a database.
    ///
    /// Both figures take the **maximum** across schedules rather than the minimum or a
    /// sum. An occurrence only has to fit *somewhere*: a school whose Friday timetable is
    /// short has not thereby made a double period impossible, and reporting from the worst
    /// schedule would produce confident findings about placements that are perfectly
    /// available on Monday.
    #[must_use]
    pub fn from_schedules(cycle_days: i64, schedules: &[Vec<bool>]) -> Self {
        let teaching_periods_per_day = schedules
            .iter()
            .map(|periods| periods.iter().filter(|teaching| **teaching).count())
            .max()
            .unwrap_or(0);

        let longest_teaching_run = schedules
            .iter()
            .map(|periods| longest_run(periods))
            .max()
            .unwrap_or(0);

        Self {
            cycle_days,
            teaching_periods_per_day: i64::try_from(teaching_periods_per_day).unwrap_or(i64::MAX),
            longest_teaching_run: i64::try_from(longest_teaching_run).unwrap_or(i64::MAX),
        }
    }

    /// How many period slots the whole cycle offers, at best.
    ///
    /// An upper bound and nothing more: it assumes every day uses the longest bell
    /// schedule and that every teaching period is available, neither of which any real
    /// school satisfies. Used only to detect demand that exceeds even the optimistic
    /// figure, which is the one case that is certainly impossible.
    #[must_use]
    pub const fn optimistic_slots(&self) -> i64 {
        self.cycle_days * self.teaching_periods_per_day
    }

    /// Whether the time model is described well enough to judge anything.
    ///
    /// A school that has not reached Cycles and Bell schedules yet gets no findings at
    /// all, rather than a confident "impossible" derived from zeroes. The setup order puts
    /// Lessons at section 10 and the time model at 14–17 ([ADR 0024](../../../docs/adr/0024-setup-is-ordered-by-what-a-school-knows.md)),
    /// so entering lessons before a cycle exists is the expected path and must not be
    /// told it is wrong.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        self.cycle_days >= 1 && self.teaching_periods_per_day >= 1
    }
}

/// The longest unbroken run of `true` in a sequence.
///
/// A day of six teaching periods with a break after the third offers no run longer than
/// three, so a four-period occurrence cannot be placed in it however many teaching periods
/// it holds in total. That is the difference this function exists to measure.
fn longest_run(flags: &[bool]) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for teaching in flags {
        if *teaching {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Something about a demand that the time model cannot accommodate.
///
/// Only ever reports impossibility, never feasibility. See this module's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum DemandFinding {
    /// One occurrence needs more consecutive periods than any day offers in a row.
    ///
    /// Structurally impossible: no placement exists, so this is not a difficulty, it is an
    /// arithmetic fact. The remedy is to shorten the occurrence or lengthen the day.
    OccurrenceLongerThanAnyRun {
        /// Consecutive periods the longest occurrence needs.
        needed: i64,
        /// The longest unbroken run of teaching periods the school has in any day.
        longest_run: i64,
    },
    /// The section's total demand exceeds every slot in the cycle.
    ///
    /// Also structurally impossible, and measured against the *optimistic* bound — every
    /// day at the longest bell schedule, every teaching period free — so a finding here
    /// cannot be an artefact of pessimism.
    TotalExceedsCycle {
        /// Period slots the section demands per cycle.
        demanded: i64,
        /// Period slots the cycle offers at the most generous reading.
        available: i64,
    },
}

impl DemandFinding {
    /// Every impossibility in one section's demand, in the order a user should read them.
    ///
    /// Empty when the demand is coherent, when the section asks for nothing, or when the
    /// school has not described its time model yet. **An empty result does not mean the
    /// section can be timetabled** — only that nothing here can prove it cannot.
    #[must_use]
    pub fn of(demand: LessonDemand, capacity: CycleCapacity) -> Vec<Self> {
        if demand.is_empty() || !capacity.is_known() {
            return Vec::new();
        }

        let mut findings = Vec::new();

        // The per-occurrence check first: it names one row the user can act on, whereas
        // the total is about the section as a whole.
        if capacity.longest_teaching_run >= 1
            && demand.longest_occurrence > capacity.longest_teaching_run
        {
            findings.push(Self::OccurrenceLongerThanAnyRun {
                needed: demand.longest_occurrence,
                longest_run: capacity.longest_teaching_run,
            });
        }

        let available = capacity.optimistic_slots();
        if demand.periods_per_cycle > available {
            findings.push(Self::TotalExceedsCycle {
                demanded: demand.periods_per_cycle,
                available,
            });
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::{CycleCapacity, DeclaredComparison, DemandFinding, LessonDemand};

    /// A five-day week of six teaching periods with a break after the third.
    const ORDINARY: CycleCapacity = CycleCapacity {
        cycle_days: 5,
        teaching_periods_per_day: 6,
        longest_teaching_run: 3,
    };

    #[test]
    fn four_single_periods_demand_four_slots_in_four_occurrences() {
        let demand = LessonDemand::of(&[1, 1, 1, 1]);
        assert_eq!(demand.occurrences, 4);
        assert_eq!(demand.periods_per_cycle, 4);
        assert_eq!(demand.longest_occurrence, 1);
    }

    #[test]
    fn one_double_period_demands_two_slots_in_one_occurrence() {
        // The distinction the whole model rests on: this is *not* two occurrences.
        let demand = LessonDemand::of(&[2]);
        assert_eq!(demand.occurrences, 1);
        assert_eq!(demand.periods_per_cycle, 2);
    }

    #[test]
    fn a_mixed_section_sums_rather_than_multiplying() {
        // Four singles and a double: five occurrences, six periods. No single
        // multiplication of a duration by a count produces 6 here, which is why the
        // demand is a sum over rows.
        let demand = LessonDemand::of(&[1, 1, 1, 1, 2]);
        assert_eq!(demand.occurrences, 5);
        assert_eq!(demand.periods_per_cycle, 6);
        assert_eq!(demand.longest_occurrence, 2);
    }

    #[test]
    fn an_empty_section_demands_nothing_and_is_not_an_error() {
        let demand = LessonDemand::of(&[]);
        assert!(demand.is_empty());
        assert_eq!(demand.periods_per_cycle, 0);
        assert!(
            DemandFinding::of(demand, ORDINARY).is_empty(),
            "a section nobody has added activities to yet is mid-setup, not wrong"
        );
    }

    #[test]
    fn the_declaration_is_compared_but_never_enforced() {
        let demand = LessonDemand::of(&[1, 1, 1]);
        assert_eq!(
            demand.against_declared(None),
            DeclaredComparison::NotDeclared
        );
        assert_eq!(
            demand.against_declared(Some(3)),
            DeclaredComparison::Matches
        );
        assert_eq!(
            demand.against_declared(Some(4)),
            DeclaredComparison::Short {
                declared: 4,
                shortfall: 1
            }
        );
        assert_eq!(
            demand.against_declared(Some(2)),
            DeclaredComparison::Over {
                declared: 2,
                excess: 1
            }
        );
    }

    #[test]
    fn an_occurrence_longer_than_any_unbroken_run_is_impossible() {
        // Six teaching periods a day, but never more than three in a row, so a four-period
        // occurrence has nowhere to go — and the total, 4 of 30, is nowhere near the limit.
        let findings = DemandFinding::of(LessonDemand::of(&[4]), ORDINARY);
        assert_eq!(
            findings,
            vec![DemandFinding::OccurrenceLongerThanAnyRun {
                needed: 4,
                longest_run: 3
            }],
            "the total is fine; it is the single occurrence that cannot be placed"
        );
    }

    #[test]
    fn a_demand_within_the_longest_run_produces_no_finding() {
        assert!(
            DemandFinding::of(LessonDemand::of(&[3, 1, 1]), ORDINARY).is_empty(),
            "three in a row fits a three-period run exactly, and 5 of 30 is nothing"
        );
    }

    #[test]
    fn demand_beyond_every_slot_in_the_cycle_is_impossible() {
        // 31 single periods in a 5 x 6 cycle.
        let durations = [1_i64; 31];
        let findings = DemandFinding::of(LessonDemand::of(&durations), ORDINARY);
        assert_eq!(
            findings,
            vec![DemandFinding::TotalExceedsCycle {
                demanded: 31,
                available: 30
            }]
        );
    }

    #[test]
    fn a_demand_that_is_merely_difficult_is_not_reported() {
        // 29 of 30 slots. Almost certainly infeasible once clashes, rooms and staff are
        // accounted for — and nothing here says so, because nothing here knows. Only a
        // solver does.
        let durations = [1_i64; 29];
        assert!(
            DemandFinding::of(LessonDemand::of(&durations), ORDINARY).is_empty(),
            "difficulty is not impossibility, and claiming otherwise would be a guess"
        );
    }

    #[test]
    fn nothing_is_judged_before_the_school_has_a_time_model() {
        // Lessons is section 10 and the cycle is section 14, so this is the expected path
        // through setup rather than a mistake to report.
        let nothing = CycleCapacity {
            cycle_days: 0,
            teaching_periods_per_day: 0,
            longest_teaching_run: 0,
        };
        assert!(!nothing.is_known());
        assert!(
            DemandFinding::of(LessonDemand::of(&[4, 4, 4]), nothing).is_empty(),
            "zeroes are an unanswered question, not a cycle with no room in it"
        );
    }

    #[test]
    fn the_findings_cross_the_boundary_as_camel_case() {
        // `CLAUDE.md` requires camelCase in JSON, mapped in exactly one place with serde
        // attributes. For an enum that place is two attributes, not one: `rename_all`
        // renames the *variants* and `rename_all_fields` renames the fields inside them.
        // Without the second, `longest_run` reaches TypeScript as `longest_run` and the
        // generated type faithfully says so — which is drift that type generation cannot
        // catch, because both sides agree and both are wrong.
        let json = serde_json::to_string(&DemandFinding::OccurrenceLongerThanAnyRun {
            needed: 4,
            longest_run: 3,
        })
        .expect("serialise");
        assert_eq!(
            json,
            r#"{"kind":"occurrenceLongerThanAnyRun","needed":4,"longestRun":3}"#
        );

        let json = serde_json::to_string(&DeclaredComparison::Short {
            declared: 4,
            shortfall: 1,
        })
        .expect("serialise");
        assert_eq!(json, r#"{"kind":"short","declared":4,"shortfall":1}"#);
    }

    #[test]
    fn a_break_in_the_middle_of_a_day_shortens_the_longest_run() {
        // Six teaching periods, a break after the third: six in the day, three in a row.
        let split = vec![true, true, true, false, true, true, true];
        let capacity = CycleCapacity::from_schedules(5, &[split]);
        assert_eq!(capacity.teaching_periods_per_day, 6);
        assert_eq!(capacity.longest_teaching_run, 3);
    }

    #[test]
    fn capacity_takes_the_most_generous_bell_schedule_not_the_least() {
        // A short Friday must not make a double period impossible on Monday.
        let long = vec![true, true, true, true];
        let short = vec![true, false];
        let capacity = CycleCapacity::from_schedules(5, &[short, long]);
        assert_eq!(capacity.teaching_periods_per_day, 4);
        assert_eq!(capacity.longest_teaching_run, 4);
    }

    #[test]
    fn a_school_with_no_bell_schedule_has_no_known_capacity() {
        let capacity = CycleCapacity::from_schedules(5, &[]);
        assert!(!capacity.is_known());
        assert_eq!(capacity.longest_teaching_run, 0);
    }

    #[test]
    fn both_impossibilities_can_be_reported_at_once() {
        let tiny = CycleCapacity {
            cycle_days: 1,
            teaching_periods_per_day: 2,
            longest_teaching_run: 2,
        };
        let findings = DemandFinding::of(LessonDemand::of(&[3]), tiny);
        assert_eq!(findings.len(), 2, "got {findings:?}");
    }
}
