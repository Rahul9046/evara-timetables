//! Tests over the whole catalogue, and over each fixture's own claims.
//!
//! # What is being asserted, and what is not
//!
//! These tests check that the fixtures are **internally consistent, deterministic and
//! correctly structured**, and that each one's encoded expectations say what the accepted
//! records require. They do not check that any rule produces those expectations, because no
//! rule exists yet — that is the point of the split described in
//! [`super::builder`]'s header.
//!
//! Where a property is checked over "every fixture", it is checked at several seeds, because
//! a determinism bug that only shows at one seed is the bug most likely to survive review.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::SchedulingFixture;
use super::catalogue::{FixtureKind, all_fixtures, fixture, fixture_names};
use crate::scheduling::clock::DurationPeriods;
use crate::scheduling::diagnostics::{DiagnosticCode, EntityKind, Severity};
use crate::scheduling::index::{BellScheduleIx, GroupIx, TimeslotIx};
use crate::scheduling::population::{ExpectedAttendance, Overlap};
use crate::scheduling::problem::Problem;

/// Seeds the catalogue-wide properties are checked at.
///
/// Four rather than one, and deliberately including `0`: a generator that special-cased an
/// empty seed, or whose first draw degenerated, would pass at any other value.
const SEEDS: [u64; 4] = [0, 1, 7, 2026];

/// A single seed for the tests that only need one.
const SEED: u64 = 2026;

fn tiny() -> SchedulingFixture {
    FixtureKind::Tiny.generate(SEED)
}

// =======================================================================================
// The catalogue
// =======================================================================================

#[test]
fn every_fixture_has_a_distinct_stable_name() {
    // Names are the contract with `evara fixture` and with any future benchmark baseline, so
    // a copy-pasted name has to fail here rather than confuse a report later.
    let names = fixture_names();
    let distinct: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(distinct.len(), names.len(), "two fixtures share a name");
    for name in names {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{name} is not kebab-case"
        );
    }
}

#[test]
fn every_fixture_is_reachable_by_name_and_nothing_else_is() {
    for name in fixture_names() {
        let found = fixture(name, SEED).expect("a listed fixture generates");
        assert_eq!(found.name, name);
    }
    assert!(fixture("no-such-fixture", SEED).is_none());
}

#[test]
fn every_fixture_states_its_purpose() {
    for kind in FixtureKind::all() {
        let purpose = kind.purpose();
        assert!(
            purpose.len() > 40,
            "{} needs a real description of what it proves",
            kind.name()
        );
    }
}

#[test]
fn every_fixture_is_internally_consistent_at_every_seed() {
    // The one class of loader-and-fixture bug that produces a plausible wrong timetable
    // rather than a crash: a dense index into the wrong table.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            assert_eq!(
                found.integrity(),
                Vec::new(),
                "{} is not internally consistent at seed {seed}",
                found.name
            );
        }
    }
}

#[test]
fn every_fixture_has_at_least_one_problem() {
    for found in all_fixtures(SEED) {
        assert!(
            !found.problems.is_empty(),
            "{} has no problem at all",
            found.name
        );
    }
}

#[test]
fn the_same_seed_produces_an_identical_catalogue() {
    // Determinism, asserted on the whole structure rather than on a summary: two runs must
    // agree on every identifier, every index and every expectation.
    for seed in SEEDS {
        assert_eq!(
            all_fixtures(seed),
            all_fixtures(seed),
            "the catalogue is not deterministic at seed {seed}"
        );
    }
}

#[test]
fn different_seeds_produce_the_same_structure_with_disjoint_identities() {
    // Two properties at once, and both matter. The *shape* must not depend on the seed, or a
    // test written against one seed would be meaningless at another; the *identities* must,
    // or two "different" fixtures would collide when loaded into one project.
    let first = all_fixtures(1);
    let second = all_fixtures(2);
    assert_eq!(first.len(), second.len());

    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(
            a.problems.len(),
            b.problems.len(),
            "{}: the problem count must not depend on the seed",
            a.name
        );
        for (pa, pb) in a.problems.iter().zip(b.problems.iter()) {
            assert_eq!(pa.activities.len(), pb.activities.len(), "{}", a.name);
            assert_eq!(pa.sections.len(), pb.sections.len(), "{}", a.name);
            assert_eq!(pa.teachers.len(), pb.teachers.len(), "{}", a.name);
            assert_eq!(pa.groups.len(), pb.groups.len(), "{}", a.name);
            assert_eq!(pa.timeslots.len(), pb.timeslots.len(), "{}", a.name);
        }

        let ids_a = every_identity(a);
        let ids_b = every_identity(b);
        assert!(
            ids_a.is_disjoint(&ids_b),
            "{} reuses identities between seed 1 and seed 2",
            a.name
        );
    }
}

#[test]
fn no_identity_is_used_twice_within_one_fixture() {
    // Fatal for persistence: a solution is written back against these identifiers, so two
    // rows claiming one would overwrite each other. `Problem::integrity` checks it per table;
    // this checks it across tables of one problem too.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for (n, problem) in found.problems.iter().enumerate() {
                let mut seen: BTreeMap<Uuid, &'static str> = BTreeMap::new();
                let mut record = |id: Uuid, what: &'static str| {
                    if let Some(previous) = seen.insert(id, what) {
                        panic!(
                            "{} problem {n}: {id} is used by both {previous} and {what}",
                            found.name
                        );
                    }
                };
                for campus in &problem.context.campuses {
                    record(campus.id, "campus");
                }
                for day in &problem.context.cycle_days {
                    record(day.id, "cycleDay");
                }
                for schedule in &problem.context.bell_schedules {
                    record(schedule.id, "bellSchedule");
                }
                for period in &problem.context.periods {
                    record(period.id, "period");
                }
                for slot in &problem.timeslots {
                    record(slot.id, "timeslot");
                }
                for room_type in &problem.room_types {
                    record(room_type.id, "roomType");
                }
                for room in &problem.rooms {
                    record(room.id, "room");
                }
                for teacher in &problem.teachers {
                    record(teacher.id, "teacher");
                }
                for group in &problem.groups {
                    record(group.id, "group");
                }
                for section in &problem.sections {
                    record(section.id, "section");
                }
                for activity in &problem.activities {
                    record(activity.id, "activity");
                }
            }
        }
    }
}

#[test]
fn every_reference_points_at_something_that_exists() {
    // `Problem::integrity` covers the index side. This covers the relationships a fixture
    // author could get structurally right and semantically wrong.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for n in 0..problem.activities.len() {
                let section = problem
                    .section_of(crate::scheduling::index::ActivityIx::new(
                        u32::try_from(n).expect("a fixture has fewer than 4 billion activities"),
                    ))
                    .unwrap_or_else(|| panic!("{}: activity {n} has no section", found.name));
                assert!(
                    !section.name.is_empty(),
                    "{}: activity {n}'s section has no name",
                    found.name
                );
            }
            for section in &problem.sections {
                for teacher in &section.teachers {
                    assert!(
                        teacher.get() < problem.teachers.len(),
                        "{}: {} names a teacher that does not exist",
                        found.name,
                        section.name
                    );
                }
                for group in &section.audience {
                    assert!(
                        group.get() < problem.groups.len(),
                        "{}: {} names a group that does not exist",
                        found.name,
                        section.name
                    );
                }
            }
        }
    }
}

#[test]
fn every_occurrence_lasts_at_least_one_period() {
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for problem in &found.problems {
                for activity in &problem.activities {
                    assert!(
                        activity.duration.get() >= 1,
                        "{}: an occurrence of zero periods is not a lesson",
                        found.name
                    );
                    assert!(
                        activity.split_index >= 1,
                        "{}: split_index starts at 1",
                        found.name
                    );
                }
            }
        }
    }
}

#[test]
fn split_indices_are_unique_within_a_section_and_never_renumbered() {
    // Assigned once, from MAX + 1, and never rewritten — ADR 0029. A fixture that reused one
    // would violate `UNIQUE (class_section_id, split_index)` the moment it was persisted.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            let mut seen: BTreeSet<(usize, u16)> = BTreeSet::new();
            for activity in &problem.activities {
                assert!(
                    seen.insert((activity.section.get(), activity.split_index)),
                    "{}: section {} has two occurrences at split_index {}",
                    found.name,
                    activity.section.get(),
                    activity.split_index
                );
            }
        }
    }
}

#[test]
fn every_timeslot_belongs_to_a_period_of_the_schedule_it_claims() {
    // The invariant V4's trigger pair enforces in the database, checked here because a
    // fixture bypasses the database entirely.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for slot in &problem.timeslots {
                let period = &problem.context.periods[slot.period.get()];
                assert_eq!(
                    period.bell_schedule, slot.bell_schedule,
                    "{}: a slot's schedule disagrees with its period's",
                    found.name
                );
                assert_eq!(
                    slot.is_teaching,
                    period.kind.is_teaching(),
                    "{}: is_teaching must be derived from period.kind and nothing else",
                    found.name
                );
                assert_eq!(
                    slot.interval, period.interval,
                    "{}: a slot's interval is its period's",
                    found.name
                );
                let schedule = &problem.context.bell_schedules[slot.bell_schedule.get()];
                assert_eq!(
                    slot.campus, schedule.campus,
                    "{}: a slot's campus is its schedule's",
                    found.name
                );
            }
        }
    }
}

#[test]
fn the_overlap_relation_covers_exactly_the_groups_and_is_symmetric() {
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            assert_eq!(
                problem.overlap.groups(),
                problem.groups.len(),
                "{}",
                found.name
            );
            for a in 0..problem.groups.len() {
                for b in 0..problem.groups.len() {
                    let (a, b) = (
                        GroupIx::new(u32::try_from(a).expect("small")),
                        GroupIx::new(u32::try_from(b).expect("small")),
                    );
                    assert_eq!(
                        problem.overlap.get(a, b),
                        problem.overlap.get(b, a),
                        "{}: the relation is not symmetric",
                        found.name
                    );
                }
            }
        }
    }
}

#[test]
fn an_unstated_group_pair_stays_indeterminate_and_is_never_quietly_disjoint() {
    // The silent-clash failure ADR 0035 §6 exists to prevent. If an unmentioned pair ever
    // defaulted to Disjoint, every fixture would start agreeing with the one behaviour the
    // product must not have.
    let found = FixtureKind::UnknownGroupOverlap.generate(SEED);
    let problem = found.problem();
    let decided = (0..problem.groups.len())
        .flat_map(|a| (0..problem.groups.len()).map(move |b| (a, b)))
        .filter(|(a, b)| a != b)
        .filter(|(a, b)| {
            problem.overlap.get(
                GroupIx::new(u32::try_from(*a).expect("small")),
                GroupIx::new(u32::try_from(*b).expect("small")),
            ) == Overlap::Disjoint
        })
        .count();
    assert_eq!(
        decided, 0,
        "a school that has answered nothing must prove nothing disjoint"
    );
    assert!(problem.overlap.indeterminate_pairs() >= 1);
}

#[test]
fn every_sections_masks_agree_with_its_audience_and_the_relation() {
    // The compiled form is what the hot path reads, so a mask that disagreed with the
    // relation would make the search enforce something other than what was decided.
    //
    // The rule, from ADR 0035 §5, is an exact biconditional:
    //
    //     g in clash_mask(s)  <=>  g in audience(s)  OR  overlap(g, a) = Overlaps
    //                                                    for some a in audience(s)
    //
    // Both directions matter. Missing a proved overlap would let a real clash through;
    // including a merely indeterminate pair would enforce something nothing proved, which is
    // the behaviour ADR 0035 §6 rejects as "no timetable".
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for section in &problem.sections {
                assert_eq!(
                    section.audience_mask.len(),
                    section.audience.len(),
                    "{}: {}'s audience mask has the wrong population",
                    found.name,
                    section.name
                );
                for group in &section.audience {
                    assert!(section.audience_mask.contains(*group));
                }

                for candidate in 0..problem.groups.len() {
                    let candidate = GroupIx::new(u32::try_from(candidate).expect("small"));
                    let should_clash = section.audience_mask.contains(candidate)
                        || section
                            .audience
                            .iter()
                            .any(|a| problem.overlap.get(candidate, *a) == Overlap::Overlaps);
                    assert_eq!(
                        section.clash_mask.contains(candidate),
                        should_clash,
                        "{}: {}'s clash mask disagrees with the relation about {}",
                        found.name,
                        section.name,
                        problem.groups[candidate.get()].name
                    );
                }
            }
        }
    }
}

#[test]
fn an_indeterminate_pair_never_reaches_a_clash_mask_on_its_own() {
    // The specific half of the biconditional above that ADR 0035 §6 is about, asserted on the
    // fixture built for it: a school that has answered nothing enforces nothing beyond
    // same-group and ancestor pairs.
    let found = FixtureKind::UnknownGroupOverlap.generate(SEED);
    let problem = found.problem();

    for section in &problem.sections {
        for candidate in 0..problem.groups.len() {
            let candidate = GroupIx::new(u32::try_from(candidate).expect("small"));
            if section.audience_mask.contains(candidate) {
                continue;
            }
            let any_proved = section
                .audience
                .iter()
                .any(|a| problem.overlap.get(candidate, *a) == Overlap::Overlaps);
            let any_indeterminate = section
                .audience
                .iter()
                .any(|a| problem.overlap.get(candidate, *a) == Overlap::Indeterminate);
            if any_indeterminate && !any_proved {
                assert!(
                    !section.clash_mask.contains(candidate),
                    "{}: an indeterminate pair must not be enforced",
                    section.name
                );
            }
        }
    }
}

#[test]
fn every_rooms_candidate_membership_follows_the_one_candidate_rule() {
    // The candidate set is a domain, and a domain stated twice is a rule living in two
    // places. This asserts the fixtures agree with `RoomFact::is_candidate_for` rather than
    // with a second copy of its predicate.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for section in &problem.sections {
                for (ix, room) in problem.rooms.iter().enumerate() {
                    let expected =
                        room.is_candidate_for(section.campus, section.required_room_type);
                    let listed =
                        section
                            .room_candidates
                            .contains(&crate::scheduling::index::RoomIx::new(
                                u32::try_from(ix).expect("small"),
                            ));
                    assert_eq!(
                        listed, expected,
                        "{}: {}'s candidate list disagrees with the candidate rule for {}",
                        found.name, section.name, room.name
                    );
                }
            }
        }
    }
}

#[test]
fn an_unbookable_room_is_in_no_candidate_set() {
    // ADR 0034 test 13, independent of capacity.
    let found = FixtureKind::InfeasibleRoomType.generate(SEED);
    let problem = found.problem();
    let unbookable: Vec<_> = problem
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| !room.is_bookable)
        .map(|(ix, _)| crate::scheduling::index::RoomIx::new(u32::try_from(ix).expect("small")))
        .collect();
    assert!(!unbookable.is_empty(), "the fixture has an unbookable room");
    for section in &problem.sections {
        for room in &unbookable {
            assert!(!section.room_candidates.contains(room));
        }
    }
}

#[test]
fn generating_a_fixture_twice_mutates_nothing_outside_itself() {
    // Nothing here reads a clock, an environment variable or a global, so two interleaved
    // generations must not influence one another. The assertion is cheap and the failure it
    // guards against — a hidden counter — would be very hard to find later.
    let first = FixtureKind::Tiny.generate(SEED);
    let _other = FixtureKind::Medium.generate(99);
    let _another = FixtureKind::PigeonholeOverlap.generate(5);
    let again = FixtureKind::Tiny.generate(SEED);
    assert_eq!(first, again);
}

#[test]
fn a_problem_round_trips_through_json() {
    // `SOLVER.md` §9 requires a JSON-serialisable `Problem`/`Assignment` pair, because that
    // is what makes `evara-cli solve fixtures/<name>` and a benchmark baseline possible. The
    // tiny fixture rather than the medium one, because this asserts the shape and not the
    // throughput.
    let found = tiny();
    let json = serde_json::to_string(found.problem()).expect("a problem serialises");
    let back: Problem = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(found.problem(), &back);
    assert_eq!(back.integrity(), Vec::new(), "and survives the trip intact");
}

#[test]
fn an_assignment_round_trips_through_json() {
    let found = tiny();
    let feasible = found.expectations.feasible.as_ref().expect("attached");
    let assignment = feasible.assignment(found.problem().activity_count());
    let json = serde_json::to_string(&assignment).expect("an assignment serialises");
    let back: crate::scheduling::placement::Assignment =
        serde_json::from_str(&json).expect("and reads back");
    assert_eq!(assignment, back);
}

#[test]
fn a_fixture_serialises_for_inspection() {
    // `evara fixture` writes this. It is deliberately write-only — see this module's header
    // on why a parsed fixture would be a second source of truth.
    let json = serde_json::to_string(&tiny()).expect("a fixture serialises");
    assert!(json.contains("\"name\":\"tiny\""));
    assert!(json.contains("\"expectations\""));
}

#[test]
fn serialisation_is_byte_for_byte_stable() {
    // Two serialisations of the same seed must be identical, or a committed baseline could
    // not be compared. `BTreeMap` in the availability index rather than `HashMap` is what
    // makes this hold.
    let first = serde_json::to_string(&FixtureKind::Medium.generate(3)).expect("serialises");
    let second = serde_json::to_string(&FixtureKind::Medium.generate(3)).expect("serialises");
    assert_eq!(first, second);
}

// =======================================================================================
// tiny
// =======================================================================================

#[test]
fn the_tiny_fixture_has_the_shape_its_documentation_claims() {
    let found = tiny();
    let problem = found.problem();
    assert_eq!(problem.context.campuses.len(), 1);
    assert!(problem.context.is_single_campus());
    assert_eq!(problem.context.cycle_day_count(), 5);
    assert_eq!(problem.context.bell_schedules.len(), 1);
    assert_eq!(problem.teachers.len(), 4);
    assert_eq!(problem.rooms.len(), 3);
    assert_eq!(problem.groups.len(), 3);
    assert_eq!(problem.sections.len(), 4);
    assert_eq!(problem.activity_count(), 10);
    assert!(problem.context.term.is_some());
}

#[test]
fn the_tiny_fixture_has_a_double_period_and_a_room_type_requirement() {
    // Both are named requirements of the brief, and both are easy to lose in a refactor.
    let found = tiny();
    let problem = found.problem();
    assert!(
        problem
            .activities
            .iter()
            .any(|a| a.duration == DurationPeriods::new(2).expect("two is positive")),
        "the tiny fixture must contain a double period"
    );
    assert!(
        problem
            .sections
            .iter()
            .any(|s| s.required_room_type.is_some()),
        "the tiny fixture must contain a room-type requirement"
    );
}

#[test]
fn the_tiny_fixtures_sections_name_no_campus_and_resolve_anyway() {
    // ADR 0033 §2 clause 3: a school with one campus has one place, so "not
    // campus-constrained" and "at the only campus there is" are the same statement. Without
    // clause 3 the commonest configuration in the product resolves to nothing.
    let found = tiny();
    let problem = found.problem();
    let schedule = &problem.context.bell_schedules[0];
    assert!(
        !schedule.is_school_wide(),
        "the only schedule is campus-scoped, which is what makes clause 3 necessary"
    );
    for section in &problem.sections {
        assert_eq!(section.campus, None, "{} names a campus", section.name);
        assert!(
            section.bell_schedule.is_resolved(),
            "{} must still resolve",
            section.name
        );
    }
}

#[test]
fn the_tiny_fixture_needs_no_student_rows_to_decide_every_group_pair() {
    // ADR 0035 test 1, and the property ADR 0006 is most at risk of losing: real clash
    // detection at a school that has entered no students at all.
    let found = tiny();
    let problem = found.problem();
    for group in &problem.groups {
        assert_eq!(
            group.effective_members, 0,
            "{} has member rows; this fixture is the group-only case",
            group.name
        );
        assert!(group.declared_size.is_some());
    }
    assert_eq!(
        problem.overlap.indeterminate_pairs(),
        0,
        "every pair must be decided from sizes, the parent edge and one declaration"
    );
}

#[test]
fn the_tiny_fixtures_arrangement_places_every_occurrence_exactly_once() {
    let found = tiny();
    let feasible = found
        .expectations
        .feasible
        .as_ref()
        .expect("the tiny fixture attaches an arrangement");
    let problem = &found.problems[feasible.problem];

    assert_eq!(feasible.spans.len(), problem.activity_count());
    let placed: BTreeSet<usize> = feasible.spans.iter().map(|(a, _)| a.get()).collect();
    assert_eq!(
        placed.len(),
        problem.activity_count(),
        "an activity repeats"
    );

    let assignment = feasible.assignment(problem.activity_count());
    assert!(assignment.is_complete());
    assert_eq!(assignment.unscheduled().count(), 0);
}

#[test]
fn the_tiny_fixtures_arrangement_covers_a_real_run_of_teaching_periods() {
    // Every one of the four ways a multi-period placement can be wrong, checked against the
    // slots the arrangement actually names: the right number of them, all teaching, all of
    // the section's resolved schedule, all on one day, and ordinal-consecutive.
    let found = tiny();
    let feasible = found.expectations.feasible.as_ref().expect("attached");
    let problem = &found.problems[feasible.problem];

    for (activity, span) in &feasible.spans {
        let fact = &problem.activities[activity.get()];
        let section = &problem.sections[fact.section.get()];
        let schedule = section
            .bell_schedule
            .schedule()
            .expect("every section of the tiny fixture resolves");

        assert_eq!(
            span.slot_count(),
            fact.duration.as_usize(),
            "{}: covers the wrong number of slots",
            section.name
        );

        let mut ordinals = Vec::with_capacity(span.slots.len());
        for slot in &span.slots {
            let slot = &problem.timeslots[slot.get()];
            assert!(
                slot.is_teaching,
                "{}: placed in a non-teaching period",
                section.name
            );
            assert_eq!(slot.bell_schedule, schedule, "{}", section.name);
            assert_eq!(slot.cycle_day, span.cycle_day, "{}", section.name);
            ordinals.push(problem.context.periods[slot.period.get()].ordinal);
        }
        for pair in ordinals.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "{}: the periods are not ordinal-consecutive",
                section.name
            );
        }
    }
}

#[test]
fn the_tiny_fixtures_arrangement_double_books_no_teacher_room_or_group() {
    // The claim "feasible by construction", checked. The interval test comes from
    // `PlacementSpan::clashes_in_time_with` — the one implementation — rather than from a
    // second copy of any rule, so this test cannot drift from what M5 will enforce.
    let found = tiny();
    let feasible = found.expectations.feasible.as_ref().expect("attached");
    let problem = &found.problems[feasible.problem];

    for (n, (left, left_span)) in feasible.spans.iter().enumerate() {
        for (right, right_span) in &feasible.spans[n + 1..] {
            if !left_span.clashes_in_time_with(right_span) {
                continue;
            }
            let a = &problem.sections[problem.activities[left.get()].section.get()];
            let b = &problem.sections[problem.activities[right.get()].section.get()];

            for teacher in &a.teachers {
                assert!(
                    !b.teachers.contains(teacher),
                    "{} and {} overlap in time and share a teacher",
                    a.name,
                    b.name
                );
            }
            assert!(
                left_span.room.is_none()
                    || right_span.room.is_none()
                    || left_span.room != right_span.room,
                "{} and {} overlap in time and share a room",
                a.name,
                b.name
            );
            assert!(
                !a.clash_mask.intersects(&b.audience_mask),
                "{} and {} overlap in time and share students",
                a.name,
                b.name
            );
        }
    }
}

// =======================================================================================
// medium
// =======================================================================================

#[test]
fn the_medium_fixture_meets_its_documented_scale() {
    let found = FixtureKind::Medium.generate(SEED);
    let problem = found.problem();

    assert_eq!(problem.teachers.len(), 80, "80 teachers");
    assert_eq!(problem.activity_count(), 500, "500 occurrences");
    assert_eq!(problem.sections.len(), 125, "125 class sections");
    assert_eq!(problem.rooms.len(), 40, "40 rooms");
    assert_eq!(
        problem.groups.len(),
        30,
        "five cohorts and twenty-five classes"
    );

    // 1 000 students, declared at group level and with no student rows at all.
    let population: i64 = problem
        .groups
        .iter()
        .filter(|g| g.parent.is_some())
        .filter_map(|g| g.declared_size)
        .sum();
    assert_eq!(population, 1_000, "1 000 students across the class groups");
    assert!(
        problem.groups.iter().all(|g| g.effective_members == 0),
        "the medium fixture declares its population and names no individual students"
    );
}

#[test]
fn the_medium_fixture_has_thirty_teaching_slots_and_the_counting_arguments_hold() {
    // The notes claim feasibility by design. These are the arithmetic claims behind that,
    // asserted so the notes cannot quietly become false.
    let found = FixtureKind::Medium.generate(SEED);
    let problem = found.problem();

    let teaching = problem.timeslots.iter().filter(|s| s.is_teaching).count();
    assert_eq!(teaching, 30, "six teaching periods over a five-day cycle");

    // Per class group: five subjects of four occurrences, with some doubles.
    let mut per_group: BTreeMap<usize, i64> = BTreeMap::new();
    for activity in &problem.activities {
        let section = &problem.sections[activity.section.get()];
        for group in &section.audience {
            *per_group.entry(group.get()).or_default() += i64::from(activity.duration.get());
        }
    }
    for (group, periods) in &per_group {
        assert!(
            *periods <= 30,
            "group {group} demands {periods} periods of 30 available"
        );
    }

    // Per teacher.
    let mut per_teacher: BTreeMap<usize, i64> = BTreeMap::new();
    for activity in &problem.activities {
        let section = &problem.sections[activity.section.get()];
        for teacher in &section.teachers {
            *per_teacher.entry(teacher.get()).or_default() += i64::from(activity.duration.get());
        }
    }
    for (teacher, periods) in &per_teacher {
        assert!(
            *periods <= 30,
            "teacher {teacher} carries {periods} periods of 30 available"
        );
    }
}

#[test]
fn the_medium_fixture_claims_no_arrangement() {
    // "Designed to be feasible" and "proven feasible" are different statements, and only a
    // solver can make the second. Attaching an arrangement here would be a claim nobody can
    // check until M6.
    let found = FixtureKind::Medium.generate(SEED);
    assert!(found.expectations.feasible.is_none());
    assert!(
        found
            .expectations
            .notes
            .iter()
            .any(|n| n.contains("not proven")),
        "the medium fixture must say so in its own notes"
    );
}

#[test]
fn the_medium_fixture_exercises_availability_and_room_requirements_at_scale() {
    let found = FixtureKind::Medium.generate(SEED);
    let problem = found.problem();
    assert!(
        problem.unavailable.restricted_entities().count() >= 5,
        "some staff must be unavailable, or `entity-unavailable` is never exercised"
    );
    assert!(
        problem
            .sections
            .iter()
            .filter(|s| s.required_room_type.is_some())
            .count()
            >= 25,
        "the specialist-room path must be exercised at scale"
    );
    assert!(
        problem.activities.iter().any(|a| a.duration.get() > 1),
        "multi-period placement must be exercised at scale"
    );
}

#[test]
fn the_medium_fixture_leaves_cross_year_pairs_indeterminate() {
    // The honest answer, and the one a convenience default would get wrong. Classes within a
    // year are disjoint by declaration; classes in different years are related by nothing.
    let found = FixtureKind::Medium.generate(SEED);
    let problem = found.problem();
    assert!(
        problem.overlap.indeterminate_pairs() > 0,
        "a realistic school has pairs nothing settles"
    );
    for expectation in &found.expectations.overlaps {
        assert_eq!(
            problem.overlap.get(expectation.a, expectation.b),
            expectation.overlap,
            "the relation disagrees with the fixture's own expectation ({})",
            expectation.rule
        );
    }
}

// =======================================================================================
// The impossible fixtures
// =======================================================================================

#[test]
fn the_infeasible_duration_fixture_contains_its_contradiction() {
    // The contradiction must be in the data, conclusively, with no solver involved: an
    // occurrence of three periods where no run of teaching periods is longer than two.
    let found = FixtureKind::InfeasibleDuration.generate(SEED);
    let problem = found.problem();

    let longest = longest_teaching_run(problem);
    assert_eq!(longest, 2, "the resolved schedule's longest run");

    let offending: Vec<_> = problem
        .activities
        .iter()
        .filter(|a| usize::from(a.duration.get()) > longest)
        .collect();
    assert_eq!(offending.len(), 1, "exactly one occurrence is impossible");
    assert_eq!(offending[0].duration.get(), 3);

    // And the fixture records the exact code and the exact row.
    let expected = &found.expectations.diagnostics;
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].code, DiagnosticCode::DurationExceedsLongestRun);
    assert_eq!(expected[0].subject_kind, EntityKind::Activity);
    assert_eq!(expected[0].subject, offending[0].id);
    assert_eq!(expected[0].code.severity(), Severity::Blocking);
}

#[test]
fn the_longer_schedule_in_the_infeasible_fixture_is_not_in_the_grid() {
    // ADR 0033 test 10: a longer run available elsewhere in the school is irrelevant, because
    // a run never spans two schedules. If that schedule were materialised the fixture would
    // be ambiguous instead of infeasible, which is a different test.
    let found = FixtureKind::InfeasibleDuration.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.context.bell_schedules.len(), 2);

    let materialised: BTreeSet<usize> = problem
        .timeslots
        .iter()
        .map(|s| s.bell_schedule.get())
        .collect();
    assert_eq!(materialised.len(), 1, "only one schedule is in G");
}

#[test]
fn the_infeasible_room_type_fixture_has_an_empty_candidate_set() {
    let found = FixtureKind::InfeasibleRoomType.generate(SEED);
    let problem = found.problem();

    let requiring: Vec<_> = problem
        .sections
        .iter()
        .filter(|s| s.required_room_type.is_some())
        .collect();
    assert_eq!(requiring.len(), 1);
    assert!(
        requiring[0].room_candidates.is_empty(),
        "no bookable room is of the required type, so the set is empty"
    );

    let expected = &found.expectations.diagnostics;
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].code, DiagnosticCode::NoRoomOfRequiredType);
    assert_eq!(expected[0].subject_kind, EntityKind::ClassSection);
    assert_eq!(expected[0].subject, requiring[0].id);
}

// =======================================================================================
// Bell-schedule resolution
// =======================================================================================

#[test]
fn the_two_campus_fixtures_hold_two_disjoint_slot_sets_over_the_same_days() {
    for kind in [
        FixtureKind::TwoCampusesIdenticalBells,
        FixtureKind::TwoCampusesDifferentBells,
    ] {
        let found = kind.generate(SEED);
        let problem = found.problem();
        assert_eq!(problem.context.campuses.len(), 2, "{}", kind.name());
        assert_eq!(problem.context.bell_schedules.len(), 2, "{}", kind.name());

        let schedules: BTreeSet<usize> = problem
            .timeslots
            .iter()
            .map(|s| s.bell_schedule.get())
            .collect();
        assert_eq!(schedules.len(), 2, "{}: both are materialised", kind.name());

        // The day axis is shared: cycle_day belongs to the cycle, not to a schedule. That is
        // what makes an interval comparison meaningful with no calendar date involved.
        for schedule in 0..2 {
            let days: BTreeSet<usize> = problem
                .timeslots
                .iter()
                .filter(|s| s.bell_schedule.get() == schedule)
                .map(|s| s.cycle_day.get())
                .collect();
            assert_eq!(days.len(), 5, "{}", kind.name());
        }
    }
}

#[test]
fn only_the_different_bells_fixture_has_different_physical_intervals() {
    // The property the brief asks for by name, and the reason the two fixtures exist as a
    // pair: identical ordinals with different minutes is where comparing ids goes wrong.
    let identical = FixtureKind::TwoCampusesIdenticalBells.generate(SEED);
    let different = FixtureKind::TwoCampusesDifferentBells.generate(SEED);

    assert_eq!(
        first_period_intervals(identical.problem()).len(),
        1,
        "identical bells: both schedules' first periods run at the same minutes"
    );
    assert_eq!(
        first_period_intervals(different.problem()).len(),
        2,
        "different bells: the two schedules' first periods run at different minutes"
    );
}

#[test]
fn a_shared_room_is_one_identity_reaching_both_campuses() {
    // ADR 0014: one physical room, one identity. Modelling a shared hall as two rooms is the
    // correctness failure that record exists to prevent, and a fixture that did so would
    // make `room-clash` look correct while missing the clash.
    let found = FixtureKind::TwoCampusesDifferentBells.generate(SEED);
    let problem = found.problem();
    let shared: Vec<_> = problem
        .rooms
        .iter()
        .filter(|r| r.campuses.len() > 1)
        .collect();
    assert_eq!(shared.len(), 1, "exactly one room reaches both campuses");
    assert_eq!(shared[0].campuses.len(), 2);

    // And both campuses' Music sections can use it, which is what makes the clash reachable.
    let using = problem
        .sections
        .iter()
        .filter(|s| s.required_room_type == shared[0].room_type)
        .count();
    assert_eq!(using, 2);
}

#[test]
fn a_teacher_under_two_bell_schedules_is_flagged() {
    // ADR 0033 §7's named limitation: a period-scoped unavailability would bind only one of
    // the two schedules, so the omission has to be visible before a solve.
    let found = FixtureKind::TwoCampusesDifferentBells.generate(SEED);
    let problem = found.problem();
    let expected = &found.expectations.diagnostics;
    assert!(
        expected
            .iter()
            .any(|d| d.code == DiagnosticCode::TeacherUnderSeveralBellSchedules),
        "the shared teacher must be flagged"
    );

    let flagged = expected
        .iter()
        .find(|d| d.code == DiagnosticCode::TeacherUnderSeveralBellSchedules)
        .expect("just asserted");
    let teacher = problem
        .teachers
        .iter()
        .find(|t| t.id == flagged.subject)
        .expect("the flagged teacher exists");
    let schedules: BTreeSet<usize> = problem
        .sections
        .iter()
        .filter(|s| {
            s.teachers
                .iter()
                .any(|t| problem.teachers[t.get()].id == teacher.id)
        })
        .filter_map(|s| s.bell_schedule.schedule())
        .map(BellScheduleIx::get)
        .collect();
    assert_eq!(schedules.len(), 2, "they really do teach under both");
}

#[test]
fn the_ambiguous_fixture_names_every_candidate() {
    // A message that says "ambiguous" without saying between what cannot be acted on.
    let found = FixtureKind::AmbiguousBellSchedule.generate(SEED);
    let problem = found.problem();
    let ambiguous: Vec<_> = problem
        .sections
        .iter()
        .filter(|s| !s.bell_schedule.is_resolved())
        .collect();
    assert_eq!(ambiguous.len(), 1);

    match &ambiguous[0].bell_schedule {
        crate::scheduling::context::BellScheduleResolution::Ambiguous { candidates } => {
            assert_eq!(candidates.len(), 2);
            // One school-wide and one campus-scoped: clause 1 and clause 2 both apply.
            let school_wide = candidates
                .iter()
                .filter(|c| problem.context.bell_schedules[c.get()].is_school_wide())
                .count();
            assert_eq!(school_wide, 1);
        }
        other => panic!("expected an ambiguity, got {other:?}"),
    }

    assert_eq!(
        found.expectations.diagnostics[0].code,
        DiagnosticCode::AmbiguousBellSchedule
    );
}

#[test]
fn the_unmaterialised_schedule_is_not_a_candidate() {
    // ADR 0033 test 8. A resolver that read `period_structure` rather than the grid would
    // report an ambiguity here and refuse a correctly configured school.
    let found = FixtureKind::UnmaterialisedBellSchedule.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.context.bell_schedules.len(), 2);

    let materialised: BTreeSet<usize> = problem
        .timeslots
        .iter()
        .map(|s| s.bell_schedule.get())
        .collect();
    assert_eq!(materialised.len(), 1);
    for section in &problem.sections {
        assert!(section.bell_schedule.is_resolved());
    }
}

#[test]
fn the_empty_grid_fixture_has_no_slots_and_reports_once_about_the_school() {
    // ADR 0033 test 9: once for the solve rather than once per section, because with no grid
    // there is nothing section-specific to say.
    let found = FixtureKind::EmptyGrid.generate(SEED);
    let problem = found.problem();
    assert_eq!(
        problem.timeslots.len(),
        0,
        "the grid is deliberately never materialised"
    );
    assert_eq!(problem.unavailable.slots(), 0);

    let expected = &found.expectations.diagnostics;
    assert_eq!(expected.len(), 1, "one finding, not one per section");
    assert_eq!(expected[0].code, DiagnosticCode::NoGridMaterialised);
    assert_eq!(expected[0].subject_kind, EntityKind::School);
    assert_eq!(expected[0].subject, problem.context.school.id);
}

// =======================================================================================
// Cross-school sharing
// =======================================================================================

#[test]
fn a_shared_teacher_keeps_one_identity_in_both_schools() {
    // ADR 0021, as a machine-checkable assertion. A loader that minted a school-specific
    // identity would break this, which is why it is a test and not a comment.
    for kind in [
        FixtureKind::SharedTeacherAcrossSchools,
        FixtureKind::DifferentSchoolCalendars,
    ] {
        let found = kind.generate(SEED);
        assert_eq!(found.problems.len(), 2, "{}", kind.name());

        let shared = found
            .expectations
            .shared_identities
            .iter()
            .find(|s| s.kind == EntityKind::Teacher)
            .unwrap_or_else(|| panic!("{} shares a teacher", kind.name()));
        assert_eq!(shared.problems, vec![0, 1]);

        for problem in &found.problems {
            let holder = problem
                .teachers
                .iter()
                .find(|t| t.id == shared.id)
                .unwrap_or_else(|| panic!("{}: both schools hold the identity", kind.name()));
            assert_eq!(
                holder.also_at.len(),
                1,
                "{}: and each names the other school",
                kind.name()
            );
        }

        // The two schools are genuinely different schools, and share nothing else.
        assert_ne!(
            found.problems[0].context.school.id,
            found.problems[1].context.school.id
        );
        let first: BTreeSet<Uuid> = found.problems[0].groups.iter().map(|g| g.id).collect();
        let second: BTreeSet<Uuid> = found.problems[1].groups.iter().map(|g| g.id).collect();
        assert!(first.is_disjoint(&second), "{}", kind.name());
    }
}

#[test]
fn the_sharing_warning_does_not_depend_on_the_other_schools_output() {
    // ADR 0032 test 7: the warning is a property of the sharing, so it is present while the
    // first school is generating — which is when it is needed. Nothing in the fixture carries
    // another school's timetable, because a `Problem` has nowhere to put one.
    let found = FixtureKind::SharedTeacherAcrossSchools.generate(SEED);
    let findings: Vec<_> = found
        .expectations
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::SharedEntityAcrossSchools)
        .collect();
    assert_eq!(findings.len(), 2, "each school warns about the sharing");
    assert_eq!(findings[0].code.severity(), Severity::Warning);
    assert!(
        !findings[0].code.severity().blocks_solve(),
        "it must never block: the remedy does not exist"
    );
}

#[test]
fn two_schools_with_different_calendars_share_no_term_and_no_grid() {
    // ADR 0032 tests 10 and 11. Nothing aligns one school's positions to the other's, and no
    // clash decision reads a date — so the absence of any calendar mapping must be harmless.
    let found = FixtureKind::DifferentSchoolCalendars.generate(SEED);
    let first = &found.problems[0];
    let second = &found.problems[1];

    let first_term = first.context.term.as_ref().expect("named");
    let second_term = second.context.term.as_ref().expect("named");
    assert_ne!(first_term.id, second_term.id);
    assert_ne!(first_term.name, second_term.name);
    assert_ne!(first.context.cycle.id, second.context.cycle.id);

    let first_days: BTreeSet<Uuid> = first.context.cycle_days.iter().map(|d| d.id).collect();
    let second_days: BTreeSet<Uuid> = second.context.cycle_days.iter().map(|d| d.id).collect();
    assert!(
        first_days.is_disjoint(&second_days),
        "two schools' cycle positions are different rows and are never equated"
    );
}

// =======================================================================================
// Group overlap
// =======================================================================================

#[test]
fn every_overlap_expectation_matches_the_compiled_relation() {
    // The fixture's stated expectation and the relation it ships must agree, or M5 would be
    // handed a specification that contradicts its own input.
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for expectation in &found.expectations.overlaps {
                let problem = &found.problems[expectation.problem];
                assert_eq!(
                    problem.overlap.get(expectation.a, expectation.b),
                    expectation.overlap,
                    "{} at seed {seed}: {} disagrees with the compiled relation",
                    found.name,
                    expectation.rule
                );
            }
        }
    }
}

#[test]
fn every_overlap_expectation_names_the_rule_that_decides_it() {
    // Which rule fires is part of the specification, not an implementation detail: rule 3
    // before rule 6 is what makes a member row beat a declaration, and a test that only
    // checked the verdict would pass on an evaluator that got there by the wrong route.
    for found in all_fixtures(SEED) {
        for expectation in &found.expectations.overlaps {
            assert!(
                expectation.rule.starts_with("rule "),
                "{}: an expectation must name its rule, got {:?}",
                found.name,
                expectation.rule
            );
        }
    }
}

#[test]
fn the_three_overlap_states_are_each_exercised_by_some_fixture() {
    // All three, or the vocabulary has a variant nothing tests.
    // A `Vec` rather than a set: a three-state answer has no meaningful total order, and
    // giving the domain type an arbitrary `Ord` to satisfy a test would be the test leaking
    // into the vocabulary.
    let mut seen: Vec<Overlap> = Vec::new();
    for found in all_fixtures(SEED) {
        for expectation in &found.expectations.overlaps {
            if !seen.contains(&expectation.overlap) {
                seen.push(expectation.overlap);
            }
        }
    }
    for state in [Overlap::Overlaps, Overlap::Disjoint, Overlap::Indeterminate] {
        assert!(seen.contains(&state), "no fixture pins {state:?}");
    }
}

#[test]
fn the_declaration_column_is_represented_in_all_three_of_its_states() {
    // NULL, 1 and 0. `NotStated` and `MayOverlap` give the same overlap answer and are
    // different states, which is the whole reason the future column is nullable rather than
    // NOT NULL DEFAULT 0 — so a fixture has to carry both.
    use crate::scheduling::population::DisjointnessDeclaration;
    let mut seen: Vec<DisjointnessDeclaration> = Vec::new();
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for group in &problem.groups {
                if !seen.contains(&group.children_are_disjoint) {
                    seen.push(group.children_are_disjoint);
                }
            }
        }
    }
    for state in [
        DisjointnessDeclaration::NotStated,
        DisjointnessDeclaration::Disjoint,
        DisjointnessDeclaration::MayOverlap,
    ] {
        assert!(seen.contains(&state), "no fixture carries {state:?}");
    }
}

#[test]
fn the_pigeonhole_fixture_proves_an_overlap_with_no_member_row_anywhere() {
    // Rule 5 extracting a genuine answer from declared sizes alone, which is the ADR 0006
    // school. If any group here had members the fixture would be proving something else.
    let found = FixtureKind::PigeonholeOverlap.generate(SEED);
    let problem = found.problem();
    assert!(
        problem.groups.iter().all(|g| g.effective_members == 0),
        "no member row may exist in this fixture"
    );

    let overlapping = found
        .expectations
        .overlaps
        .iter()
        .find(|e| e.overlap == Overlap::Overlaps)
        .expect("one pair overlaps");
    assert!(overlapping.rule.contains("rule 5"));

    let (a, b) = (
        &problem.groups[overlapping.a.get()],
        &problem.groups[overlapping.b.get()],
    );
    let parent = a.parent.expect("both sets have a parent");
    assert_eq!(b.parent, Some(parent));
    let total = a.declared_size.expect("sized") + b.declared_size.expect("sized");
    assert!(
        total > problem.groups[parent.get()].declared_size.expect("sized"),
        "the sizes must actually overflow the parent"
    );
}

#[test]
fn the_contradiction_fixture_lets_the_member_row_win() {
    // Rule 3 before rule 6. The declaration says disjoint and the pair is Overlaps, which is
    // also the mitigation for the "tick disjoint to clear the warning" risk.
    use crate::scheduling::population::DisjointnessDeclaration;
    let found = FixtureKind::DeclaredDisjointContradicted.generate(SEED);
    let problem = found.problem();

    let expectation = &found.expectations.overlaps[0];
    assert_eq!(expectation.overlap, Overlap::Overlaps);
    assert!(expectation.rule.contains("rule 3"));

    let parent = problem.groups[expectation.a.get()]
        .parent
        .expect("the pair are siblings");
    assert_eq!(
        problem.groups[parent.get()].children_are_disjoint,
        DisjointnessDeclaration::Disjoint,
        "the parent really does declare them disjoint"
    );
    assert!(
        problem.groups[expectation.a.get()].effective_members > 0,
        "and there really are member rows to contradict it"
    );
}

#[test]
fn the_deep_hierarchy_fixture_needs_a_subtree_walk_and_not_a_parent_read() {
    // Members only at the leaf, eight levels down. A one-step parent read would miss the
    // cohort-level pair entirely.
    let found = FixtureKind::DeepGroupHierarchy.generate(SEED);
    let problem = found.problem();
    assert_eq!(problem.groups.len(), 8);

    let leaves: Vec<_> = problem
        .groups
        .iter()
        .filter(|g| g.effective_members > 0)
        .collect();
    assert_eq!(leaves.len(), 1, "members exist only at the leaf");

    // The chain really is eight deep.
    let mut depth = 0;
    let mut current = problem
        .groups
        .iter()
        .position(|g| g.effective_members > 0)
        .expect("the leaf exists");
    while let Some(parent) = problem.groups[current].parent {
        current = parent.get();
        depth += 1;
    }
    assert_eq!(depth, 7, "seven edges above the leaf");
}

#[test]
fn the_two_divisions_fixture_distinguishes_the_three_structurally_identical_pairs() {
    // Two disjoint, one indeterminate, and in a flat hierarchy all three look the same. This
    // is the case a sibling-means-disjoint rule gets wrong, and gets wrong silently.
    let found = FixtureKind::TwoDivisionsOfOneCohort.generate(SEED);
    let disjoint = found
        .expectations
        .overlaps
        .iter()
        .filter(|e| e.overlap == Overlap::Disjoint)
        .count();
    let indeterminate = found
        .expectations
        .overlaps
        .iter()
        .filter(|e| e.overlap == Overlap::Indeterminate)
        .count();
    assert_eq!(disjoint, 2);
    assert_eq!(indeterminate, 1);
}

// =======================================================================================
// Attendance
// =======================================================================================

#[test]
fn every_attendance_expectation_matches_the_section_it_is_about() {
    for seed in SEEDS {
        for found in all_fixtures(seed) {
            for expectation in &found.expectations.attendance {
                let problem = &found.problems[expectation.problem];
                let section = &problem.sections[expectation.section.get()];
                assert_eq!(
                    section.expected_attendance, expectation.expected,
                    "{} at seed {seed}: {} — {}",
                    found.name, section.name, expectation.because
                );
            }
        }
    }
}

#[test]
fn unknown_attendance_is_never_represented_as_zero() {
    // The exact collapse ADR 0034 §3 names in the existing display path. A section whose
    // attendance cannot be derived must carry the Unknown variant, not Known { people: 0 },
    // because 0 satisfies every room in the school including a cupboard.
    let found = FixtureKind::UnknownAttendance.generate(SEED);
    let problem = found.problem();
    assert_eq!(
        found.expectations.attendance.len(),
        4,
        "four distinct reasons"
    );

    for expectation in &found.expectations.attendance {
        assert_eq!(expectation.expected, ExpectedAttendance::Unknown);
        let section = &problem.sections[expectation.section.get()];
        assert_eq!(section.expected_attendance.people(), None);
        assert_ne!(
            section.expected_attendance,
            ExpectedAttendance::Known { people: 0 },
            "{}: Unknown and a stated zero are different values",
            section.name
        );
    }
}

#[test]
fn an_attendance_floor_is_available_without_becoming_known() {
    // A bound that can only be too small may raise a warning and may never decide a hard
    // constraint — ADR 0034 §3.
    let found = FixtureKind::UnknownAttendance.generate(SEED);
    let problem = found.problem();
    let with_floor: Vec<_> = problem
        .sections
        .iter()
        .filter(|s| s.attendance_floor.is_informative())
        .collect();
    assert_eq!(with_floor.len(), 1);
    assert_eq!(with_floor[0].attendance_floor.at_least, 26);
    assert_eq!(
        with_floor[0].expected_attendance,
        ExpectedAttendance::Unknown,
        "a floor is not a measurement"
    );
}

#[test]
fn an_explicit_attendance_wins_over_a_derivation_and_the_difference_is_advice() {
    // ADR 0034 test 1. 24 stated against a group of 28: the stated value stands and the
    // difference is Info, because a derivation that overrode it would be refusing a true
    // statement.
    let found = FixtureKind::KnownAttendance.generate(SEED);
    let problem = found.problem();

    let stated = found
        .expectations
        .attendance
        .iter()
        .find(|e| e.expected == ExpectedAttendance::Known { people: 24 })
        .expect("one section states 24");
    let section = &problem.sections[stated.section.get()];
    let group = &problem.groups[section.audience[0].get()];
    assert_eq!(group.declared_size, Some(28), "taught to a group of 28");

    let advice = found
        .expectations
        .diagnostics
        .iter()
        .find(|d| d.code == DiagnosticCode::ExpectedAttendanceDisagreesWithGroups)
        .expect("the difference is reported");
    assert_eq!(advice.code.severity(), Severity::Info);
    assert_eq!(advice.subject, section.id);
}

#[test]
fn a_sum_over_groups_is_only_claimed_where_they_are_proved_disjoint() {
    // The 0035 → 0034 edge, which is load-bearing: a sum over groups that might share
    // students is a wrong number, not a conservative estimate.
    let found = FixtureKind::KnownAttendance.generate(SEED);
    let problem = found.problem();
    let summed = found
        .expectations
        .attendance
        .iter()
        .find(|e| e.because.contains("sum"))
        .expect("one section derives a sum");
    let section = &problem.sections[summed.section.get()];
    assert_eq!(section.audience.len(), 2);
    assert_eq!(
        problem
            .overlap
            .get(section.audience[0], section.audience[1]),
        Overlap::Disjoint,
        "the sum is licensed by a proof, not by convenience"
    );
    assert_eq!(
        section.expected_attendance,
        ExpectedAttendance::Known { people: 56 }
    );
}

#[test]
fn a_bookable_room_of_zero_capacity_is_named_and_not_reinterpreted() {
    // ADR 0034 §5: 0 means zero seats. Nothing reads it as "not entered", and the finding is
    // a warning about a suspicious number rather than a substitution.
    let found = FixtureKind::KnownAttendance.generate(SEED);
    let problem = found.problem();
    let zero: Vec<_> = problem
        .rooms
        .iter()
        .filter(|r| r.is_bookable && r.capacity == 0)
        .collect();
    assert_eq!(zero.len(), 1);

    let finding = found
        .expectations
        .diagnostics
        .iter()
        .find(|d| d.code == DiagnosticCode::BookableRoomZeroCapacity)
        .expect("named by pre-flight");
    assert_eq!(finding.subject, zero[0].id);
    assert_eq!(finding.code.severity(), Severity::Warning);
    assert!(
        zero[0].is_candidate_for(None, None),
        "a capacity of 0 does not remove a room from a candidate set: room-capacity is not \
         in the first constraint set"
    );
}

#[test]
fn a_roster_without_a_group_leaves_an_empty_audience() {
    // ADR 0035 §7: class_enrolment has no scheduling meaning, so twenty named students and
    // no group means nothing protects them. The remedy is named rather than implied.
    let found = FixtureKind::RosterWithoutGroup.generate(SEED);
    let problem = found.problem();
    let unprotected: Vec<_> = problem
        .sections
        .iter()
        .filter(|s| s.audience.is_empty())
        .collect();
    assert_eq!(unprotected.len(), 1);
    assert_eq!(unprotected[0].individually_enrolled, 20);
    assert!(unprotected[0].audience_mask.is_empty());
    assert!(unprotected[0].clash_mask.is_empty());

    assert!(
        found
            .expectations
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::SectionWithoutAttendingGroup
                && d.subject == unprotected[0].id)
    );
}

#[test]
fn a_named_student_who_is_also_in_a_group_is_not_counted_twice() {
    // The roster contributes nothing, so there is nothing to double-count. Asserted on the
    // attendance rather than on a count, because the attendance is what a room would be
    // compared against.
    let found = FixtureKind::RosterWithoutGroup.generate(SEED);
    let problem = found.problem();
    let covered = problem
        .sections
        .iter()
        .find(|s| !s.audience.is_empty() && s.individually_enrolled > 0)
        .expect("one section has both");
    assert_eq!(
        covered.expected_attendance,
        ExpectedAttendance::Known { people: 24 },
        "the three named students are among the 24 and are not added to them"
    );
}

// =======================================================================================
// Availability
// =======================================================================================

#[test]
fn an_entity_with_no_rules_is_available_and_one_with_a_rule_is_not() {
    // V11's documented default, which is the opposite of the equally plausible assumption —
    // and assuming the other way would make every school unsolvable on day one.
    use crate::scheduling::availability::{AvailabilityEntity, SlotAvailability};
    let found = FixtureKind::MissingAvailabilityContext.generate(SEED);
    let problem = found.problem();

    let restricted: Vec<_> = problem.unavailable.restricted_entities().collect();
    assert_eq!(restricted.len(), 1, "exactly one teacher is restricted");

    let AvailabilityEntity::Teacher { teacher } = restricted[0] else {
        panic!("the restriction is on a teacher");
    };
    let other = problem
        .teachers
        .iter()
        .enumerate()
        .map(|(ix, _)| crate::scheduling::index::TeacherIx::new(u32::try_from(ix).expect("small")))
        .find(|t| *t != teacher)
        .expect("there is another teacher");

    for slot in 0..problem.timeslots.len() {
        let slot = TimeslotIx::new(u32::try_from(slot).expect("small"));
        assert_eq!(
            problem
                .unavailable
                .status(AvailabilityEntity::Teacher { teacher: other }, slot),
            SlotAvailability::Unconstrained,
            "a teacher with no rows is available in every slot"
        );
    }

    let blocked = (0..problem.timeslots.len())
        .map(|ix| TimeslotIx::new(u32::try_from(ix).expect("small")))
        .filter(|slot| {
            problem
                .unavailable
                .status(AvailabilityEntity::Teacher { teacher }, *slot)
                == SlotAvailability::Unavailable
        })
        .count();
    assert_eq!(
        blocked, 4,
        "all four teaching periods of one cycle position"
    );
}

#[test]
fn a_soft_availability_rule_is_carried_and_permits_a_placement() {
    // Every soft constraint is deferred past the MVP, so a discouraged slot is legal and
    // merely worse. Carrying it without enforcing it is the honest state.
    use crate::scheduling::availability::{AvailabilityEntity, SlotAvailability};
    let found = FixtureKind::MissingAvailabilityContext.generate(SEED);
    let problem = found.problem();

    let discouraged = (0..problem.timeslots.len())
        .map(|ix| TimeslotIx::new(u32::try_from(ix).expect("small")))
        .filter_map(|slot| {
            (0..problem.rooms.len())
                .map(|ix| crate::scheduling::index::RoomIx::new(u32::try_from(ix).expect("small")))
                .find(|room| {
                    problem
                        .unavailable
                        .status(AvailabilityEntity::Room { room: *room }, slot)
                        == SlotAvailability::Discouraged
                })
                .map(|room| (room, slot))
        })
        .count();
    assert_eq!(discouraged, 1);
    assert!(SlotAvailability::Discouraged.permits_placement());
}

#[test]
fn an_availability_rule_binds_only_the_slots_it_names() {
    // ADR 0033 §7: the loader compiles each rule onto the slots it actually names and widens
    // nothing. A rule that leaked to the same ordinal under another schedule would silently
    // over-constrain a shared teacher.
    use crate::scheduling::availability::{AvailabilityEntity, SlotAvailability};
    let found = FixtureKind::MissingAvailabilityContext.generate(SEED);
    let problem = found.problem();
    let restricted = problem
        .unavailable
        .restricted_entities()
        .next()
        .expect("one entity is restricted");

    let blocked_days: BTreeSet<usize> = (0..problem.timeslots.len())
        .map(|ix| TimeslotIx::new(u32::try_from(ix).expect("small")))
        .filter(|slot| {
            problem.unavailable.status(restricted, *slot) == SlotAvailability::Unavailable
        })
        .map(|slot| problem.timeslots[slot.get()].cycle_day.get())
        .collect();
    assert_eq!(
        blocked_days.len(),
        1,
        "the rule names one cycle position and reaches no other"
    );

    let AvailabilityEntity::Teacher { .. } = restricted else {
        panic!("the restriction is on a teacher");
    };
}

// =======================================================================================
// Cycles
// =======================================================================================

#[test]
fn a_rotating_cycle_changes_nothing_about_resolution_or_overlap() {
    // ADR 0033 test 14 and ADR 0035 test 16. `cycle.kind` is the obvious thing for a later
    // reader to branch on, and CLAUDE.md names reading a weekday in scheduling as an
    // architecture bug.
    use crate::scheduling::context::CycleKind;
    let found = FixtureKind::CustomRotatingCycle.generate(SEED);
    let problem = found.problem();

    assert_eq!(problem.context.cycle.kind, CycleKind::Custom);
    assert_eq!(problem.context.cycle_day_count(), 6);
    for (n, day) in problem.context.cycle_days.iter().enumerate() {
        assert_eq!(
            day.ordinal,
            u16::try_from(n + 1).expect("small"),
            "ordinals run from 1 with no gaps"
        );
        assert!(
            !WEEKDAY_NAMES.contains(&day.label.as_str()),
            "a rotation's positions must not be named after weekdays"
        );
    }
    for section in &problem.sections {
        assert!(section.bell_schedule.is_resolved());
    }
    for expectation in &found.expectations.overlaps {
        assert_eq!(
            problem.overlap.get(expectation.a, expectation.b),
            expectation.overlap
        );
    }
}

#[test]
fn ordinals_run_from_one_with_no_gaps_in_every_fixture() {
    // A day's ordinal is its whole scheduling identity, and a period's ordinal decides what
    // "consecutive" means. A gap in either would make a run test quietly wrong.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            for (n, day) in problem.context.cycle_days.iter().enumerate() {
                assert_eq!(
                    day.ordinal,
                    u16::try_from(n + 1).expect("small"),
                    "{}",
                    found.name
                );
            }
            for schedule in &problem.context.bell_schedules {
                for (n, period) in schedule.periods.iter().enumerate() {
                    assert_eq!(
                        problem.context.periods[period.get()].ordinal,
                        u16::try_from(n + 1).expect("small"),
                        "{}",
                        found.name
                    );
                }
            }
        }
    }
}

// =======================================================================================
// Expectation hygiene
// =======================================================================================

#[test]
fn every_expected_diagnostic_names_a_row_that_exists() {
    // A diagnostic about the wrong row would be worse than one that did not fire: it would
    // send the user to fix something that is not broken.
    for found in all_fixtures(SEED) {
        for expectation in &found.expectations.diagnostics {
            let problem = &found.problems[expectation.problem];
            let exists = match expectation.subject_kind {
                EntityKind::School => problem.context.school.id == expectation.subject,
                EntityKind::ClassSection => {
                    problem.sections.iter().any(|s| s.id == expectation.subject)
                }
                EntityKind::Activity => problem
                    .activities
                    .iter()
                    .any(|a| a.id == expectation.subject),
                EntityKind::Teacher => problem.teachers.iter().any(|t| t.id == expectation.subject),
                EntityKind::Room => problem.rooms.iter().any(|r| r.id == expectation.subject),
                EntityKind::StudentGroup => {
                    problem.groups.iter().any(|g| g.id == expectation.subject)
                }
                other => panic!(
                    "{}: no fixture expects a finding about {other:?}",
                    found.name
                ),
            };
            assert!(
                exists,
                "{}: an expected {} names a row the problem does not hold",
                found.name,
                expectation.code.key()
            );
            assert!(
                !expectation.because.is_empty(),
                "{}: every expectation says why",
                found.name
            );
        }
    }
}

#[test]
fn the_three_severities_are_each_exercised_by_some_fixture() {
    let mut seen: BTreeSet<Severity> = BTreeSet::new();
    for found in all_fixtures(SEED) {
        for expectation in &found.expectations.diagnostics {
            seen.insert(expectation.code.severity());
        }
    }
    for severity in [Severity::Blocking, Severity::Warning, Severity::Info] {
        assert!(
            seen.contains(&severity),
            "no fixture expects a {severity:?}"
        );
    }
}

#[test]
fn every_fixture_either_expects_a_finding_or_explains_itself() {
    // A fixture with neither expectations nor notes is data nobody can review.
    for found in all_fixtures(SEED) {
        let has_claims = !found.expectations.diagnostics.is_empty()
            || !found.expectations.overlaps.is_empty()
            || !found.expectations.attendance.is_empty()
            || found.expectations.feasible.is_some();
        assert!(
            has_claims || !found.expectations.notes.is_empty(),
            "{} makes no claim and explains nothing",
            found.name
        );
    }
}

#[test]
fn no_fixture_pins_a_placement_or_implies_a_solver() {
    // M2 ships no solver, so the only arrangement in the catalogue is the tiny fixture's
    // existence witness. A second one appearing here would mean somebody had started
    // scheduling.
    let attached: Vec<&str> = all_fixtures(SEED)
        .iter()
        .filter(|f| f.expectations.feasible.is_some())
        .map(|f| f.name)
        .collect();
    assert_eq!(attached, vec!["tiny"]);
}

#[test]
fn nothing_in_the_catalogue_is_pinned_because_locks_do_not_exist() {
    // `activity.is_locked` is not in V15, and `activity-fixed` joins the constraint set once
    // locks do. A fixture that pinned something would be depending on a column that is not
    // there.
    for found in all_fixtures(SEED) {
        for problem in &found.problems {
            assert!(
                problem.pinned.is_empty(),
                "{} pins a placement, and locks do not exist yet",
                found.name
            );
        }
    }
}

// =======================================================================================
// Helpers
// =======================================================================================

/// Weekday names, so the rotating-cycle fixture can assert it uses none of them.
const WEEKDAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Every stored identity in a fixture, across every problem and table.
fn every_identity(found: &SchedulingFixture) -> BTreeSet<Uuid> {
    let mut ids = BTreeSet::new();
    for problem in &found.problems {
        ids.insert(problem.context.school.id);
        ids.insert(problem.context.cycle.id);
        if let Some(term) = &problem.context.term {
            ids.insert(term.id);
        }
        ids.extend(problem.context.campuses.iter().map(|c| c.id));
        ids.extend(problem.context.cycle_days.iter().map(|d| d.id));
        ids.extend(problem.context.bell_schedules.iter().map(|s| s.id));
        ids.extend(problem.context.periods.iter().map(|p| p.id));
        ids.extend(problem.timeslots.iter().map(|s| s.id));
        ids.extend(problem.room_types.iter().map(|t| t.id));
        ids.extend(problem.rooms.iter().map(|r| r.id));
        ids.extend(problem.teachers.iter().map(|t| t.id));
        ids.extend(problem.groups.iter().map(|g| g.id));
        ids.extend(problem.sections.iter().map(|s| s.id));
        ids.extend(problem.activities.iter().map(|a| a.id));
    }
    ids
}

/// The longest unbroken run of teaching periods in the schedules materialised into the grid.
///
/// Measured over the slots of one cycle position, in period-ordinal order, which is the
/// measure ADR 0033 §5 defines: a run is broken by a non-teaching period between two teaching
/// ones.
fn longest_teaching_run(problem: &Problem) -> usize {
    let mut longest = 0;
    for schedule in 0..problem.context.bell_schedules.len() {
        let schedule =
            crate::scheduling::index::BellScheduleIx::new(u32::try_from(schedule).expect("small"));
        for day in 0..problem.context.cycle_days.len() {
            let day = crate::scheduling::index::CycleDayIx::new(u32::try_from(day).expect("small"));
            let mut current = 0;
            for slot in problem.slots_of(schedule, day) {
                if problem.timeslots[slot.get()].is_teaching {
                    current += 1;
                    longest = longest.max(current);
                } else {
                    current = 0;
                }
            }
        }
    }
    longest
}

/// The distinct wall-clock intervals of every schedule's first period.
///
/// One entry means the schedules agree on when their first period runs; two means they do
/// not, which is the configuration where comparing timeslot ids gives the wrong answer.
fn first_period_intervals(problem: &Problem) -> BTreeSet<(u16, u16)> {
    problem
        .context
        .bell_schedules
        .iter()
        .filter_map(|schedule| schedule.periods.first())
        .map(|period| {
            let interval = problem.context.periods[period.get()].interval;
            (interval.start.minutes(), interval.end.minutes())
        })
        .collect()
}
