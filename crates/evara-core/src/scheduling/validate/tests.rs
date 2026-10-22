//! Tests for pre-solve validation.
//!
//! # The primary oracle
//!
//! M2's fixtures carry `ExpectedDiagnostic` entries recording a code **and** the row it must
//! be about. Those are a **lower bound**: a fixture states findings validation must produce,
//! not the complete set it may produce. So the catalogue-wide test asserts every recorded
//! expectation is produced, and the per-fixture tests assert the rest.
//!
//! `tiny` is the exception and is asserted exhaustively: it was built to be clean, so a false
//! positive there is as much a defect as a miss.

use std::collections::BTreeSet;

use super::{ValidationSummary, validate};
use crate::scheduling::diagnostics::{DiagnosticCode, EntityKind, Severity};
use crate::scheduling::fixtures::{FixtureKind, all_fixtures};
use crate::scheduling::problem::Problem;

/// Seeds the catalogue-wide properties are checked at.
const SEEDS: [u64; 3] = [0, 7, 2026];

/// A single seed for the tests that need only one.
const SEED: u64 = 2026;

/// Every code validation produces findings for, in M3.
const IMPLEMENTED: &[DiagnosticCode] = &[
    DiagnosticCode::NoGridMaterialised,
    DiagnosticCode::NoBellSchedule,
    DiagnosticCode::AmbiguousBellSchedule,
    DiagnosticCode::DurationExceedsLongestRun,
    DiagnosticCode::NoRoomOfRequiredType,
    DiagnosticCode::RoomTypeDemandExceedsSupply,
    DiagnosticCode::EmptyCandidateSet,
    DiagnosticCode::TeacherLoadExceedsSlots,
    DiagnosticCode::GroupDemandExceedsSlots,
    DiagnosticCode::SharedEntityAcrossSchools,
    DiagnosticCode::GroupOverlapIndeterminate,
    DiagnosticCode::SectionWithoutAttendingGroup,
    DiagnosticCode::SectionWithoutTeacher,
    DiagnosticCode::ExpectedAttendanceUnknown,
    DiagnosticCode::GroupSizeUnknownWithMembers,
    DiagnosticCode::BookableRoomZeroCapacity,
    DiagnosticCode::TeacherUnderSeveralBellSchedules,
    DiagnosticCode::DeclaredPeriodsDisagree,
    DiagnosticCode::ExpectedAttendanceDisagreesWithGroups,
];

/// Codes M3 deliberately produces no finding for, and who owns each.
///
/// Every one is documented in `docs/PRE-SOLVE-VALIDATION.md`. None is abandoned.
const DEFERRED: &[(DiagnosticCode, &str)] = &[
    // Needs member identities and ADR 0035 §3's rule 3 against rule 6. A `Problem` carries a
    // member *count*, not identities, so M3 could not detect it even if it owned the rule.
    (DiagnosticCode::GroupOverlapContradiction, "M5"),
    // About stored rows a compiled problem has already dropped — ADR 0037.
    (DiagnosticCode::OrphanAvailabilityReference, "M4"),
    (DiagnosticCode::AvailabilityEntityKindNotScheduled, "M4"),
    // A violated database invariant, reported as corruption by whatever reads the rows.
    (DiagnosticCode::TeacherNotEligibleForCampus, "M4"),
];

fn codes_in(problem: &Problem) -> BTreeSet<DiagnosticCode> {
    validate(problem).findings.iter().map(|f| f.code).collect()
}

// =======================================================================================
// Ownership: every code has one
// =======================================================================================

#[test]
fn every_declared_code_is_either_implemented_or_deferred_with_an_owner() {
    // The guard against a code being silently abandoned, and against M3 quietly fabricating a
    // finding to reach full coverage.
    for code in DiagnosticCode::all() {
        let implemented = IMPLEMENTED.contains(code);
        let deferred = DEFERRED.iter().any(|(c, _)| c == code);
        assert!(
            implemented ^ deferred,
            "{} is neither owned nor deferred, or is both",
            code.key()
        );
    }
    assert_eq!(
        IMPLEMENTED.len() + DEFERRED.len(),
        DiagnosticCode::all().len()
    );
}

#[test]
fn no_deferred_code_is_ever_produced() {
    // A deferred code that fired would be a finding nobody designed the wording or the
    // trigger for.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for problem in &found.problems {
                let produced = codes_in(problem);
                for (code, owner) in DEFERRED {
                    assert!(
                        !produced.contains(code),
                        "{} at seed {seed} produced {}, which is {owner}'s",
                        found.name,
                        code.key()
                    );
                }
            }
        }
    }
}

// =======================================================================================
// The fixtures' own expectations
// =======================================================================================

#[test]
fn every_recorded_expectation_is_produced_about_the_row_it_names() {
    // The primary oracle. A diagnostic that fired about the wrong row would be worse than one
    // that did not fire: it would send the user to fix something that is not broken.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for expectation in &found.expectations.diagnostics {
                let problem = &found.problems[expectation.problem];
                let report = validate(problem);
                let matched = report.findings.iter().any(|finding| {
                    finding.code == expectation.code
                        && finding.subject.id == expectation.subject
                        && finding.subject.kind == expectation.subject_kind
                });
                assert!(
                    matched,
                    "{} problem {} at seed {seed}: expected {} about {:?} {} — {}\nproduced: \
                     {:?}",
                    found.name,
                    expectation.problem,
                    expectation.code.key(),
                    expectation.subject_kind,
                    expectation.subject,
                    expectation.because,
                    report
                        .findings
                        .iter()
                        .map(|f| (f.code.key(), f.subject.label.as_str()))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn the_tiny_fixture_produces_no_finding_at_all() {
    // It was built to be clean: attendance stated for every section, every declaration
    // answered, nothing shared, every section staffed and attended, every demand inside
    // capacity. A false positive here is as much a defect as a miss.
    for seed in SEEDS {
        let found = FixtureKind::Tiny.generate(seed);
        let report = validate(found.problem());
        assert_eq!(
            report.findings.len(),
            0,
            "tiny at seed {seed} produced {:?}",
            report
                .findings
                .iter()
                .map(|f| (f.code.key(), f.message.as_str()))
                .collect::<Vec<_>>()
        );
        assert!(report.may_solve());
        assert!(ValidationSummary::of(found.problem(), &report).is_clean());
    }
}

#[test]
fn validation_never_contradicts_a_consistent_problem() {
    // A problem `Problem::integrity` calls consistent must not produce a finding that implies
    // it is not. Integrity is about loader bugs; validation is about a school's inputs.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            assert_eq!(
                found.integrity(),
                Vec::new(),
                "{} is inconsistent at seed {seed}",
                found.name
            );
            for problem in &found.problems {
                let _ = validate(problem);
            }
        }
    }
}

// =======================================================================================
// Blocking: the scheduling context
// =======================================================================================

#[test]
fn an_empty_grid_is_reported_once_about_the_school() {
    // ADR 0033 test 9: once for the solve rather than once per section. The fixture has a
    // section, so a per-section implementation would produce two findings.
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    assert_eq!(report.count_code(DiagnosticCode::NoGridMaterialised), 1);
    assert_eq!(
        report.count_code(DiagnosticCode::NoBellSchedule),
        0,
        "the per-section form is suppressed when the cause is the solve's"
    );
    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::NoGridMaterialised)
        .expect("just asserted");
    assert_eq!(finding.subject.kind, EntityKind::School);
    assert_eq!(finding.subject.id, problem.context.school.id);
    assert!(!report.may_solve());
}

#[test]
fn an_ambiguous_section_names_every_candidate_and_both_remedies() {
    let found = FixtureKind::AmbiguousBellSchedule.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::AmbiguousBellSchedule)
        .expect("the fixture is ambiguous");

    // Both candidates, by name, in the message and as structured references.
    for schedule in &problem.context.bell_schedules {
        assert!(
            finding.message.contains(&schedule.name),
            "the message must name {}",
            schedule.name
        );
    }
    let named: BTreeSet<&str> = finding
        .related
        .iter()
        .filter(|r| r.kind == EntityKind::BellSchedule)
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(named.len(), 2);

    let remedy = finding.remedy.as_deref().expect("a remedy");
    assert!(
        remedy.contains("Build"),
        "remedy one: build from one of them"
    );
    assert!(remedy.contains("restrict"), "remedy two: narrow the solve");
    assert!(!report.may_solve());
}

#[test]
fn a_resolving_school_produces_no_bell_schedule_finding() {
    // The false-positive guard for the whole resolution stage.
    for kind in [
        FixtureKind::Tiny,
        FixtureKind::SingleCampusSchoolWideBells,
        FixtureKind::TwoCampusesIdenticalBells,
        FixtureKind::TwoCampusesDifferentBells,
        FixtureKind::UnmaterialisedBellSchedule,
        FixtureKind::CustomRotatingCycle,
    ] {
        let found = kind.generate(SEED);
        let produced = codes_in(found.problem());
        for code in [
            DiagnosticCode::NoGridMaterialised,
            DiagnosticCode::NoBellSchedule,
            DiagnosticCode::AmbiguousBellSchedule,
        ] {
            assert!(
                !produced.contains(&code),
                "{} must not report {}",
                kind.name(),
                code.key()
            );
        }
    }
}

#[test]
fn an_unmaterialised_schedule_causes_no_false_ambiguity() {
    // The M2 defect this milestone found: `context.bell_schedules` is every *defined*
    // schedule, so resolving against it would report an ambiguity here. ADR 0033 test 8.
    let found = FixtureKind::UnmaterialisedBellSchedule.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.context.bell_schedules.len(), 2);
    assert!(!codes_in(problem).contains(&DiagnosticCode::AmbiguousBellSchedule));
}

// =======================================================================================
// Blocking: structure and duration
// =======================================================================================

#[test]
fn the_duration_fixture_reports_the_offending_occurrence_and_both_figures() {
    let found = FixtureKind::InfeasibleDuration.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::DurationExceedsLongestRun)
        .collect();
    assert_eq!(findings.len(), 1, "exactly one occurrence is impossible");

    let finding = findings[0];
    assert_eq!(finding.subject.kind, EntityKind::Activity);
    assert!(finding.message.contains('3'), "the duration needed");
    assert!(finding.message.contains('2'), "the longest run available");
    assert!(finding.remedy.is_some());
    assert_eq!(finding.severity(), Severity::Blocking);
    assert!(!report.may_solve());
}

#[test]
fn the_sections_other_occurrence_is_not_condemned() {
    // A finding about the whole school or the whole section would be wrong: the single-period
    // occurrence is perfectly placeable.
    let found = FixtureKind::InfeasibleDuration.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.activities.len(), 2);
    assert_eq!(
        validate(problem).count_code(DiagnosticCode::DurationExceedsLongestRun),
        1
    );
}

#[test]
fn the_room_type_fixture_reports_the_section_and_not_each_occurrence() {
    // The requirement lives on the section's course and every occurrence fails identically,
    // so one finding per section is the actionable report.
    let found = FixtureKind::InfeasibleRoomType.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NoRoomOfRequiredType)
        .collect();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].subject.kind, EntityKind::ClassSection);
    assert!(findings[0].message.contains("Laboratory"));
    assert!(findings[0].remedy.is_some());

    // And the residual code does not also fire: the more specific finding wins.
    assert_eq!(report.count_code(DiagnosticCode::EmptyCandidateSet), 0);
    assert_eq!(
        report.count_code(DiagnosticCode::RoomTypeDemandExceedsSupply),
        0,
        "zero supply is the degenerate case, reported under the specific code only"
    );
}

// =======================================================================================
// Warnings
// =======================================================================================

#[test]
fn a_shared_teacher_warning_names_both_schools_and_offers_no_remedy() {
    // ADR 0032 rule 3's wording requirement, and the one place a missing remedy is correct.
    for kind in [
        FixtureKind::SharedTeacherAcrossSchools,
        FixtureKind::DifferentSchoolCalendars,
    ] {
        let found = kind.generate(SEED);
        for (n, problem) in found.problems.iter().enumerate() {
            let report = validate(problem);
            let finding = report
                .findings
                .iter()
                .find(|f| f.code == DiagnosticCode::SharedEntityAcrossSchools)
                .unwrap_or_else(|| panic!("{} problem {n} shares a teacher", kind.name()));

            assert!(
                finding.message.contains(&problem.context.school.name),
                "{}: names this school",
                kind.name()
            );
            // The other school's name, read from the teacher the finding is actually about
            // rather than from a position in the related list.
            let teacher = problem
                .teachers
                .iter()
                .find(|t| t.id == finding.subject.id)
                .unwrap_or_else(|| panic!("{}: the subject exists", kind.name()));
            assert_eq!(teacher.also_at.len(), 1);
            assert!(
                finding.message.contains(&teacher.also_at[0].name),
                "{}: names the other school",
                kind.name()
            );
            assert!(
                finding.message.contains("cannot compare"),
                "{}: states the consequence",
                kind.name()
            );
            assert_eq!(
                finding.remedy,
                None,
                "{}: the remedy does not exist, so none is invented",
                kind.name()
            );
            assert!(report.may_solve(), "{}: sharing never blocks", kind.name());
        }
    }
}

#[test]
fn the_indeterminate_warning_is_one_aggregate_finding_with_arithmetic() {
    // ADR 0035 §6: a count plus the questions that would narrow it, never a list of pairs.
    let found = FixtureKind::UnknownGroupOverlap.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::GroupOverlapIndeterminate)
        .collect();
    assert_eq!(findings.len(), 1, "aggregated, not one per pair");

    let finding = findings[0];
    let pairs = problem.overlap.indeterminate_pairs();
    assert!(pairs > 0);
    assert!(
        finding.message.contains(&pairs.to_string()),
        "the pair count is in the message"
    );
    assert!(
        finding
            .message
            .contains("may be timetabled at the same time"),
        "the consequence is stated"
    );
    assert!(
        finding.message.contains("Year 10"),
        "the parent to answer is named"
    );
    assert!(
        finding.message.contains("would narrow"),
        "it must not claim the answers resolve everything"
    );
    assert!(
        !finding.message.contains("resolves"),
        "the exact resolved count needs M5's evaluator and must not be claimed"
    );
    assert!(report.may_solve(), "indeterminate never blocks");
}

#[test]
fn a_school_that_has_answered_everything_gets_no_indeterminate_warning() {
    for kind in [FixtureKind::Tiny, FixtureKind::DisjointSiblingGroups] {
        let found = kind.generate(SEED);
        let produced = codes_in(found.problem());
        if found.problem().overlap.indeterminate_pairs() == 0 {
            assert!(
                !produced.contains(&DiagnosticCode::GroupOverlapIndeterminate),
                "{} has nothing unsettled",
                kind.name()
            );
        }
    }
}

#[test]
fn unknown_attendance_is_reported_per_section_and_never_as_zero() {
    let found = FixtureKind::UnknownAttendance.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    assert_eq!(
        report.count_code(DiagnosticCode::ExpectedAttendanceUnknown),
        4,
        "one per affected section, because the remedy is per section"
    );
    for finding in report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ExpectedAttendanceUnknown)
    {
        assert_eq!(finding.subject.kind, EntityKind::ClassSection);
        assert!(
            finding.message.contains("not be checked"),
            "says what will not be checked"
        );
        assert!(finding.remedy.is_some());
        assert!(
            !finding.message.contains(" 0 "),
            "a missing attendance is never rendered as zero: {}",
            finding.message
        );
    }
    assert!(
        report.may_solve(),
        "ADR 0006: such a school still generates"
    );
}

#[test]
fn an_attendance_floor_is_described_as_a_floor() {
    // ADR 0034 §3: a bound that can only be too small may warn and may never decide.
    let found = FixtureKind::UnknownAttendance.generate(SEED);
    let report = validate(found.problem());
    let floors: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            f.code == DiagnosticCode::ExpectedAttendanceUnknown && f.message.contains("At least")
        })
        .collect();
    assert_eq!(floors.len(), 1);
    assert!(floors[0].message.contains("26"));
    assert!(
        floors[0].message.contains("not a measurement"),
        "the floor must not read as a count"
    );
}

#[test]
fn a_zero_capacity_room_is_named_without_being_reinterpreted() {
    let found = FixtureKind::KnownAttendance.generate(SEED);
    let report = validate(found.problem());
    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::BookableRoomZeroCapacity)
        .expect("the fixture has one");
    assert_eq!(finding.subject.kind, EntityKind::Room);
    assert!(finding.message.contains("as entered"));
    assert!(finding.remedy.is_some());
    assert_eq!(finding.severity(), Severity::Warning);
}

#[test]
fn a_teacher_under_two_schedules_is_warned_about() {
    let found = FixtureKind::TwoCampusesDifferentBells.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);
    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::TeacherUnderSeveralBellSchedules)
        .expect("the shared teacher teaches under both");
    assert_eq!(finding.subject.kind, EntityKind::Teacher);
    assert_eq!(finding.related.len(), 2, "both schedules are referenced");
    assert!(finding.message.contains("does not apply to the others"));
    assert!(report.may_solve());
}

#[test]
fn a_roster_without_a_group_is_warned_about_and_names_the_roster() {
    let found = FixtureKind::RosterWithoutGroup.generate(SEED);
    let report = validate(found.problem());
    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::SectionWithoutAttendingGroup)
        .expect("one section has no group");
    assert!(finding.message.contains("20"), "the roster size is named");
    assert!(
        finding.message.contains("no scheduling meaning"),
        "ADR 0029 §4's consequence is stated"
    );
    assert!(finding.remedy.is_some());
}

// =======================================================================================
// Informational
// =======================================================================================

#[test]
fn a_stated_attendance_that_disagrees_is_advice_and_nothing_is_overridden() {
    let found = FixtureKind::KnownAttendance.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);
    let finding = report
        .findings
        .iter()
        .find(|f| f.code == DiagnosticCode::ExpectedAttendanceDisagreesWithGroups)
        .expect("24 against a group of 28");

    assert_eq!(finding.severity(), Severity::Info);
    assert!(finding.message.contains("24"));
    assert!(finding.message.contains("28"));
    assert!(
        finding.message.contains("uses the figure you entered"),
        "the explicit value wins, and the message says so"
    );
    assert!(report.may_solve());
}

#[test]
fn a_matching_declaration_produces_no_advice() {
    // tiny declares 3 periods and has 3. A false positive here would make the informational
    // channel noise.
    let found = FixtureKind::Tiny.generate(SEED);
    assert!(!codes_in(found.problem()).contains(&DiagnosticCode::DeclaredPeriodsDisagree));
}

// =======================================================================================
// The counting checks
// =======================================================================================

#[test]
fn no_catalogue_fixture_trips_a_counting_check() {
    // Every fixture is either comfortable or impossible for a reason M3 reports more
    // specifically. A counting check firing here would mean the arithmetic is wrong, because
    // none of these schools is aggregate-impossible.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for problem in &found.problems {
                let produced = codes_in(problem);
                for code in [
                    DiagnosticCode::TeacherLoadExceedsSlots,
                    DiagnosticCode::GroupDemandExceedsSlots,
                    DiagnosticCode::RoomTypeDemandExceedsSupply,
                ] {
                    assert!(
                        !produced.contains(&code),
                        "{} at seed {seed} trips {}",
                        found.name,
                        code.key()
                    );
                }
            }
        }
    }
}

#[test]
fn the_medium_fixtures_counting_arguments_hold_under_validation() {
    // The fixture's notes claim it is comfortable. This is the claim, checked by the same
    // arithmetic a refusal would use.
    let found = FixtureKind::Medium.generate(SEED);
    let report = validate(found.problem());
    assert!(
        report.may_solve(),
        "medium must not be refused: {:?}",
        report
            .of_severity(Severity::Blocking)
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn sufficient_aggregate_capacity_does_not_prove_a_timetable_exists() {
    // The property the brief requires a test for, and the discipline of the whole stage: an
    // empty report is not feasibility. `tiny` has ample aggregate capacity and validation
    // says nothing about whether a solver will succeed — only M6 can.
    let found = FixtureKind::Tiny.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);

    assert!(report.may_solve());
    let summary = ValidationSummary::of(problem, &report);
    assert!(summary.may_solve());
    // The arrangement the fixture attaches is the *only* evidence a timetable exists, and it
    // comes from the fixture rather than from validation.
    assert!(
        found.expectations.feasible.is_some(),
        "tiny's feasibility is witnessed by an attached arrangement, not by a clean report"
    );
}

#[test]
fn an_unresolved_section_is_skipped_by_the_counting_checks() {
    // Missing data must not be bounded with a guess: a section with no schedule has no slot
    // set, so its teacher and groups are skipped rather than refused.
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let produced = codes_in(found.problem());
    for code in [
        DiagnosticCode::TeacherLoadExceedsSlots,
        DiagnosticCode::GroupDemandExceedsSlots,
        DiagnosticCode::RoomTypeDemandExceedsSupply,
        DiagnosticCode::DurationExceedsLongestRun,
        DiagnosticCode::EmptyCandidateSet,
    ] {
        assert!(
            !produced.contains(&code),
            "{} must not fire when nothing resolved",
            code.key()
        );
    }
}

// =======================================================================================
// Ordering, determinism and robustness
// =======================================================================================

#[test]
fn validation_is_deterministic_in_findings_and_in_order() {
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for problem in &found.problems {
                let first = validate(problem);
                let second = validate(problem);
                assert_eq!(first, second, "{} at seed {seed}", found.name);
            }
        }
    }
}

#[test]
fn blocking_findings_come_before_warnings_and_warnings_before_advice() {
    // The ordering policy: causes before symptoms, and severity descending across stages.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            let report = validate(problem);
            let ranks: Vec<u8> = report
                .findings
                .iter()
                .map(|f| match f.severity() {
                    Severity::Blocking => 0,
                    Severity::Warning => 1,
                    Severity::Info => 2,
                })
                .collect();
            assert!(
                ranks.windows(2).all(|w| w[0] <= w[1]),
                "{}: findings are out of severity order: {ranks:?}",
                found.name
            );
        }
    }
}

#[test]
fn validation_does_not_mutate_the_problem() {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            let before = problem.clone();
            let _ = validate(problem);
            assert_eq!(&before, problem);
        }
    }
}

#[test]
fn validation_survives_a_problem_with_nothing_in_it() {
    // A school part-way through setup. No grid, no sections, no groups — and no panic.
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let mut problem = found.problem().clone();
    problem.sections.clear();
    problem.activities.clear();
    problem.groups.clear();
    problem.teachers.clear();
    problem.rooms.clear();
    problem.overlap = crate::scheduling::population::OverlapRelation::unknown(0);

    let report = validate(&problem);
    assert_eq!(
        report.count_code(DiagnosticCode::NoGridMaterialised),
        1,
        "the one finding that is about the school itself"
    );
}

#[test]
fn every_finding_names_a_row_the_problem_holds() {
    // A finding about a row that does not exist would send a user nowhere.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for finding in &validate(problem).findings {
                let exists = match finding.subject.kind {
                    EntityKind::School => problem.context.school.id == finding.subject.id,
                    EntityKind::RoomType => problem
                        .room_types
                        .iter()
                        .any(|t| t.id == finding.subject.id),
                    EntityKind::Room => problem.rooms.iter().any(|r| r.id == finding.subject.id),
                    EntityKind::Teacher => {
                        problem.teachers.iter().any(|t| t.id == finding.subject.id)
                    }
                    EntityKind::StudentGroup => {
                        problem.groups.iter().any(|g| g.id == finding.subject.id)
                    }
                    EntityKind::ClassSection => {
                        problem.sections.iter().any(|s| s.id == finding.subject.id)
                    }
                    EntityKind::Activity => problem
                        .activities
                        .iter()
                        .any(|a| a.id == finding.subject.id),
                    other => panic!("{}: unexpected subject kind {other:?}", found.name),
                };
                assert!(
                    exists,
                    "{}: {} names a row the problem does not hold",
                    found.name,
                    finding.code.key()
                );
            }
        }
    }
}

// =======================================================================================
// Wording
// =======================================================================================

#[test]
fn every_message_names_its_subject() {
    // A diagnostic with a generic message makes the structured subject the only way to know
    // what it is about, which a printed report cannot show.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for finding in &validate(problem).findings {
                assert!(
                    finding.message.contains(&finding.subject.label),
                    "{}: {} does not name {:?}\n  {}",
                    found.name,
                    finding.code.key(),
                    finding.subject.label,
                    finding.message
                );
            }
        }
    }
}

#[test]
fn every_blocking_finding_states_a_remedy() {
    // §3.2 reserves blocking for an input whose remedy is a field the user can fill in. A
    // blocking finding without one would contradict the reason it blocks.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for finding in validate(problem).of_severity(Severity::Blocking) {
                let remedy = finding.remedy.as_deref().unwrap_or("");
                assert!(
                    remedy.len() > 20,
                    "{}: {} blocks without a usable remedy",
                    found.name,
                    finding.code.key()
                );
            }
        }
    }
}

#[test]
fn no_message_carries_markup_or_an_unresolved_placeholder() {
    // `SOLVER.md` §6: the frontend renders and localises; it does not author the reason. So a
    // message is prose, and never markup or a template.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for finding in &validate(problem).findings {
                for text in [Some(&finding.message), finding.remedy.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    for forbidden in ['<', '>', '{', '}', '%'] {
                        assert!(
                            !text.contains(forbidden),
                            "{}: {} contains {forbidden:?}: {text}",
                            found.name,
                            finding.code.key()
                        );
                    }
                    assert!(!text.contains("TODO"), "{}", found.name);
                    assert!(!text.contains("  "), "double space in: {text}");
                }
            }
        }
    }
}

#[test]
fn every_message_reads_as_a_sentence() {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for finding in &validate(problem).findings {
                let message = &finding.message;
                assert!(
                    message.len() > 40,
                    "{}: {} is too terse to act on: {message}",
                    found.name,
                    finding.code.key()
                );
                assert!(
                    message.ends_with('.') || message.ends_with('?'),
                    "{}: {} is not a sentence: {message}",
                    found.name,
                    finding.code.key()
                );
                assert!(
                    message
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_uppercase() || c.is_numeric()),
                    "{}: {} does not start a sentence: {message}",
                    found.name,
                    finding.code.key()
                );
            }
        }
    }
}

// =======================================================================================
// Summary
// =======================================================================================

#[test]
fn the_summary_counts_pairs_rather_than_findings() {
    // One aggregate finding stands for however many pairs it aggregates, which is why the
    // summary reads the problem rather than counting findings.
    let found = FixtureKind::UnknownGroupOverlap.generate(SEED);
    let problem = found.problem();
    let report = validate(problem);
    let summary = ValidationSummary::of(problem, &report);

    assert_eq!(
        report.count_code(DiagnosticCode::GroupOverlapIndeterminate),
        1
    );
    assert_eq!(
        summary.indeterminate_group_pairs,
        problem.overlap.indeterminate_pairs()
    );
    assert!(summary.indeterminate_group_pairs >= 1);
    assert_eq!(summary.unanswered_disjointness, 1, "one parent to answer");
    assert!(summary.may_solve());
    assert!(!summary.is_clean());
}

#[test]
fn the_summary_agrees_with_the_report_on_severity_counts() {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            let report = validate(problem);
            let summary = ValidationSummary::of(problem, &report);
            assert_eq!(summary.blocking, report.count_of(Severity::Blocking));
            assert_eq!(summary.warnings, report.count_of(Severity::Warning));
            assert_eq!(summary.informational, report.count_of(Severity::Info));
            assert_eq!(summary.may_solve(), report.may_solve());
        }
    }
}

#[test]
fn the_summary_counts_shared_entities_and_unchecked_attendance() {
    let found = FixtureKind::SharedTeacherAcrossSchools.generate(SEED);
    let problem = found.problem();
    let summary = ValidationSummary::of(problem, &validate(problem));
    assert_eq!(summary.shared_entities, 1);

    let found = FixtureKind::UnknownAttendance.generate(SEED);
    let problem = found.problem();
    let summary = ValidationSummary::of(problem, &validate(problem));
    assert_eq!(summary.unchecked_attendance, 4);
}
