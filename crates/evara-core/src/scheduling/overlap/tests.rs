//! ADR 0035's truth table, row by row, plus the properties the relation must hold.
//!
//! The record says *"Every row is a case a fixture must cover"* and that the twenty rows are
//! *"unit tests of the function itself"*. So each row below names its number, its rule and
//! its evidence, and a failure points at the line of the ADR it broke rather than at an
//! opaque assertion.
//!
//! Row 20 is the one row that is not a test of this function: two groups of different schools
//! are **never compared**, because `student_group.school_id` is `NOT NULL` and a parent must
//! share its child's school, so a subtree never crosses a school boundary and a `Problem`
//! holds one school by construction. It is asserted in `evara-db`'s loader suite, where the
//! two schools can actually exist.

use uuid::Uuid;

use super::{Contradiction, GroupMembers, evaluate};
use crate::scheduling::index::{GroupIx, StudentIx};
use crate::scheduling::population::{DisjointnessDeclaration, Overlap};
use crate::scheduling::problem::{GroupFact, GroupKind};

// =======================================================================================
// Building a hierarchy to ask about
// =======================================================================================

/// A group, named so a failing assertion reads like the truth table.
fn group(
    name: &str,
    parent: Option<u32>,
    size: Option<i64>,
    declares: DisjointnessDeclaration,
) -> GroupFact {
    GroupFact {
        id: Uuid::from_u128(
            u128::try_from(name.len()).expect("a short name") * 1_000
                + u128::from(name.as_bytes()[0]),
        ),
        name: name.to_owned(),
        kind: GroupKind::Class,
        parent: parent.map(GroupIx::new),
        declared_size: size,
        // Set from the member sets by the evaluator's caller; the evaluator reads the sets.
        effective_members: 0,
        children_are_disjoint: declares,
    }
}

/// Shorthand for the common case: a sized group that declares nothing about its children.
fn sized(name: &str, parent: Option<u32>, size: i64) -> GroupFact {
    group(name, parent, Some(size), DisjointnessDeclaration::NotStated)
}

fn g(ix: u32) -> GroupIx {
    GroupIx::new(ix)
}

fn student(ix: u32) -> StudentIx {
    StudentIx::new(ix)
}

/// Evaluates, with the given direct member rows applied first.
fn answer(groups: &[GroupFact], rows: &[(u32, u32)]) -> super::OverlapOutcome {
    let students = rows.iter().map(|(_, s)| *s + 1).max().unwrap_or(0);
    let students = usize::try_from(students).expect("a small fixture");
    let mut members = GroupMembers::new(groups.len(), students);
    for (group, pupil) in rows {
        members.add(g(*group), student(*pupil));
    }
    evaluate(groups, &mut members)
}

/// The verdict for one pair.
fn verdict(groups: &[GroupFact], rows: &[(u32, u32)], a: u32, b: u32) -> Overlap {
    answer(groups, rows).relation.get(g(a), g(b))
}

// =======================================================================================
// The truth table
// =======================================================================================

#[test]
fn row_1_a_group_against_itself_overlaps() {
    // Rule 2 — identity. The diagonal M2's fixtures and M4's loader left unstated.
    let groups = [sized("9A", None, 28)];
    assert_eq!(verdict(&groups, &[], 0, 0), Overlap::Overlaps);
}

#[test]
fn row_2_a_provably_empty_group_is_disjoint_from_everything() {
    // Rule 1. `size = 0`, no members, and nothing in the subtree declares more.
    let groups = [
        group("Empty", None, Some(0), DisjointnessDeclaration::NotStated),
        sized("9A", None, 28),
    ];
    assert_eq!(verdict(&groups, &[], 0, 1), Overlap::Disjoint);
}

#[test]
fn row_3_emptiness_precedes_identity() {
    // Rule 1 before rule 2, which is the ordering this row exists to pin: an empty group
    // competes for nobody, *including itself*.
    let groups = [group(
        "Empty",
        None,
        Some(0),
        DisjointnessDeclaration::NotStated,
    )];
    assert_eq!(verdict(&groups, &[], 0, 0), Overlap::Disjoint);
}

#[test]
fn row_4_a_parent_overlaps_its_child() {
    // Rule 4 — containment.
    let groups = [sized("Year 9", None, 120), sized("9A", Some(0), 28)];
    assert_eq!(verdict(&groups, &[], 0, 1), Overlap::Overlaps);
}

#[test]
fn row_5_an_ancestor_overlaps_a_grandchild() {
    // Rule 4 again, two levels down — a one-step parent read would miss this.
    let groups = [
        sized("Year 9", None, 120),
        sized("Year 9 Maths Sets", Some(0), 120),
        sized("9-Mat-1", Some(1), 28),
    ];
    assert_eq!(verdict(&groups, &[], 0, 2), Overlap::Overlaps);
}

#[test]
fn row_6_an_empty_child_is_disjoint_from_its_own_parent() {
    // Rule 1 beats rule 4. A parent and a child normally overlap; an empty child holds
    // nobody to overlap with.
    let groups = [
        sized("Year 9", None, 120),
        group("9A", Some(0), Some(0), DisjointnessDeclaration::NotStated),
    ];
    assert_eq!(verdict(&groups, &[], 0, 1), Overlap::Disjoint);
}

#[test]
fn row_7_siblings_under_a_disjoint_declaration_are_disjoint() {
    // Rule 6.
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    assert_eq!(verdict(&groups, &[], 1, 2), Overlap::Disjoint);
}

#[test]
fn row_8_siblings_with_an_unanswered_parent_are_indeterminate() {
    // Rule 7. The commonest pair in any school, and the reason the column is required.
    let groups = [
        sized("Year 9", None, 120),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    assert_eq!(verdict(&groups, &[], 1, 2), Overlap::Indeterminate);
}

#[test]
fn row_9_a_shared_member_beats_a_disjoint_declaration_and_is_reported() {
    // Rule 3 before rule 6 — the ordering that makes a member row what happened and the flag
    // what was intended.
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    let rows = [(1, 0), (2, 0)];
    let outcome = answer(&groups, &rows);

    assert_eq!(outcome.relation.get(g(1), g(2)), Overlap::Overlaps);
    assert_eq!(
        outcome.contradictions,
        vec![Contradiction {
            a: g(1),
            b: g(2),
            parent: g(0),
            student: student(0),
        }],
        "the contradiction names both groups, the declaring parent and the student"
    );
}

#[test]
fn row_10_cousins_under_a_may_overlap_parent_are_indeterminate() {
    // Rule 7. Their paths separate under Year 9, which declares `0`, so rule 6 does not
    // fire — and the honest answer is that nothing stored proves it either way.
    let groups = [
        group(
            "Year 9",
            None,
            Some(120),
            DisjointnessDeclaration::MayOverlap,
        ),
        group(
            "Maths Sets",
            Some(0),
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        group(
            "French Sets",
            Some(0),
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9-Mat-1", Some(1), 28),
        sized("9-Fre-1", Some(2), 25),
    ];
    assert_eq!(verdict(&groups, &[], 3, 4), Overlap::Indeterminate);
}

#[test]
fn row_11_a_set_against_a_class_is_indeterminate() {
    // Rule 7, the other half of ADR 0035 §4's worked example.
    let groups = [
        group(
            "Year 9",
            None,
            Some(120),
            DisjointnessDeclaration::MayOverlap,
        ),
        group(
            "Classes",
            Some(0),
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        group(
            "Maths Sets",
            Some(0),
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(1), 28),
        sized("9-Mat-1", Some(2), 28),
    ];
    assert_eq!(verdict(&groups, &[], 3, 4), Overlap::Indeterminate);
}

#[test]
fn row_12_disjointness_propagates_to_descendants() {
    // Rule 6 matched on the *ancestors*: 9A-Reading and 9B-Reading are disjoint because 9A
    // and 9B are. A rule that compared only the two groups' own parents would miss this.
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
        sized("9A-Reading", Some(1), 12),
        sized("9B-Reading", Some(2), 12),
    ];
    assert_eq!(verdict(&groups, &[], 3, 4), Overlap::Disjoint);
}

#[test]
fn row_13_the_pigeonhole_proves_overlap_with_no_member_row() {
    // Rule 5. 70 + 70 > 120, so they must intersect — and this fixture contains no member
    // row anywhere, which is the ADR 0006 school.
    let groups = [
        sized("Year 9", None, 120),
        sized("Set X", Some(0), 70),
        sized("Set Y", Some(0), 70),
    ];
    assert_eq!(verdict(&groups, &[], 1, 2), Overlap::Overlaps);
}

#[test]
fn row_14_the_pigeonhole_proves_nothing_when_the_sizes_fit() {
    // Rule 7. 28 + 25 ≤ 120. The arithmetic proves overlap when it fails and nothing when it
    // holds, which is the asymmetry that keeps it sound.
    let groups = [
        sized("Year 9", None, 120),
        sized("Set X", Some(0), 28),
        sized("Set Y", Some(0), 25),
    ];
    assert_eq!(verdict(&groups, &[], 1, 2), Overlap::Indeterminate);
}

#[test]
fn row_15_partial_rosters_with_no_shared_row_are_indeterminate() {
    // Rule 7 — "absent rows are not evidence of absence". The single most important negative
    // in this file: an unknown membership is never an empty one.
    let groups = [
        group("Year 9", None, None, DisjointnessDeclaration::NotStated),
        group("Set X", Some(0), None, DisjointnessDeclaration::NotStated),
        group("Set Y", Some(0), None, DisjointnessDeclaration::NotStated),
    ];
    let rows = [(1, 0), (1, 1), (1, 2), (2, 3), (2, 4), (2, 5)];
    assert_eq!(verdict(&groups, &rows, 1, 2), Overlap::Indeterminate);
}

#[test]
fn row_16_one_shared_member_proves_overlap_with_no_declaration() {
    // Rule 3, standing alone. No flag was needed and none is reported as contradicted,
    // because no ancestor declared its children disjoint.
    let groups = [
        group("Year 9", None, None, DisjointnessDeclaration::NotStated),
        group("Set X", Some(0), None, DisjointnessDeclaration::NotStated),
        group("Set Y", Some(0), None, DisjointnessDeclaration::NotStated),
    ];
    let rows = [(1, 0), (1, 1), (1, 7), (2, 7), (2, 3), (2, 4)];
    let outcome = answer(&groups, &rows);
    assert_eq!(outcome.relation.get(g(1), g(2)), Overlap::Overlaps);
    assert_eq!(
        outcome.contradictions,
        Vec::new(),
        "nothing was declared, so nothing is contradicted"
    );
}

#[test]
fn row_17_unrelated_is_not_disjoint() {
    // Rule 7. Different year groups share no ancestor at all, so no declaration can ever
    // reach them — which is why ADR 0038 forbids promising that answering declarations will
    // clear every pair.
    let groups = [sized("9A", None, 28), sized("10A", None, 28)];
    assert_eq!(verdict(&groups, &[], 0, 1), Overlap::Indeterminate);
}

#[test]
fn row_18_a_member_who_has_left_is_not_shared() {
    // The §2 filter is applied by whatever reads the rows, so "has left" reaches this
    // function as an absent member row. Asserted here as the behaviour that absence then
    // produces: the declaration stands and the pair is Disjoint.
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    // The shared student is filtered out upstream, so only 9A keeps a row.
    let rows = [(1, 0)];
    let outcome = answer(&groups, &rows);
    assert_eq!(outcome.relation.get(g(1), g(2)), Overlap::Disjoint);
    assert_eq!(outcome.contradictions, Vec::new());
}

#[test]
fn row_19_an_inactive_member_is_not_shared_either() {
    // Row 18's sibling case, and the same mechanism: `is_active = 0` is filtered upstream.
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    let rows = [(2, 4)];
    assert_eq!(verdict(&groups, &rows, 1, 2), Overlap::Disjoint);
}

// =======================================================================================
// Properties the relation must hold
// =======================================================================================

#[test]
fn the_relation_is_symmetric_everywhere() {
    let groups = [
        group(
            "Year 9",
            None,
            Some(120),
            DisjointnessDeclaration::MayOverlap,
        ),
        group(
            "Classes",
            Some(0),
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(1), 28),
        sized("9B", Some(1), 28),
        sized("Set X", Some(0), 70),
        sized("Set Y", Some(0), 70),
        group(
            "Empty",
            Some(0),
            Some(0),
            DisjointnessDeclaration::NotStated,
        ),
    ];
    let rows = [(2, 0), (4, 1)];
    let relation = answer(&groups, &rows).relation;
    let count = u32::try_from(groups.len()).expect("a small fixture");
    for a in 0..count {
        for b in 0..count {
            assert_eq!(
                relation.get(g(a), g(b)),
                relation.get(g(b), g(a)),
                "{} vs {} is not symmetric",
                groups[a as usize].name,
                groups[b as usize].name
            );
        }
    }
}

#[test]
fn the_diagonal_is_overlaps_unless_the_group_is_provably_empty() {
    // The correction M3 and M4 both deferred. `PRE-SOLVE-VALIDATION.md` §14's limitation 9
    // is closed by this assertion.
    let groups = [
        sized("9A", None, 28),
        group("Empty", None, Some(0), DisjointnessDeclaration::NotStated),
        group("Unsized", None, None, DisjointnessDeclaration::NotStated),
    ];
    let relation = answer(&groups, &[]).relation;
    assert_eq!(relation.get(g(0), g(0)), Overlap::Overlaps);
    assert_eq!(relation.get(g(1), g(1)), Overlap::Disjoint);
    assert_eq!(
        relation.get(g(2), g(2)),
        Overlap::Overlaps,
        "a group that has declared no size is not provably empty"
    );
}

#[test]
fn evaluating_twice_gives_an_identical_outcome() {
    let groups = [
        group(
            "Year 9 Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
    ];
    let rows = [(1, 3), (2, 3), (1, 9)];
    assert_eq!(answer(&groups, &rows), answer(&groups, &rows));
}

#[test]
fn contradictions_are_returned_in_pair_index_order() {
    // A report that named the same rows in a different order on two runs would be a
    // reproducibility failure, not a cosmetic one.
    let groups = [
        group(
            "Classes",
            None,
            Some(120),
            DisjointnessDeclaration::Disjoint,
        ),
        sized("9A", Some(0), 28),
        sized("9B", Some(0), 28),
        sized("9C", Some(0), 28),
    ];
    // One student in all three classes: three contradicted pairs.
    let rows = [(1, 0), (2, 0), (3, 0)];
    let found = answer(&groups, &rows).contradictions;
    let pairs: Vec<(u32, u32)> = found.iter().map(|c| (c.a.raw(), c.b.raw())).collect();
    assert_eq!(pairs, vec![(1, 2), (1, 3), (2, 3)]);
    assert!(found.iter().all(|c| c.parent == g(0)));
}

#[test]
fn a_group_with_no_declaration_and_no_members_decides_nothing() {
    // The honest default. Two unrelated, unsized, member-less groups are Indeterminate — not
    // Disjoint, which would be the silent-clash failure, and not Overlaps, which would make
    // a year group hold one lesson at a time.
    let groups = [
        group("A", None, None, DisjointnessDeclaration::NotStated),
        group("B", None, None, DisjointnessDeclaration::NotStated),
    ];
    assert_eq!(verdict(&groups, &[], 0, 1), Overlap::Indeterminate);
}

#[test]
fn an_empty_problem_evaluates_to_an_empty_relation() {
    let outcome = answer(&[], &[]);
    assert_eq!(outcome.relation.groups(), 0);
    assert_eq!(outcome.contradictions, Vec::new());
}

#[test]
fn effective_membership_is_the_subtree_union() {
    // Truth table row 5's mechanism, measured directly: members entered only at a leaf count
    // towards every ancestor. This is also what `GroupFact::effective_members` is set from,
    // so the count and the set cannot disagree.
    let groups = [
        group("Year 9", None, None, DisjointnessDeclaration::NotStated),
        group("Classes", Some(0), None, DisjointnessDeclaration::NotStated),
        group("9A", Some(1), None, DisjointnessDeclaration::NotStated),
    ];
    let mut members = GroupMembers::new(3, 4);
    for pupil in 0..4 {
        members.add(g(2), student(pupil));
    }
    let _ = evaluate(&groups, &mut members);

    assert_eq!(members.effective_count(g(2)), 4, "the leaf itself");
    assert_eq!(members.effective_count(g(1)), 4, "its parent");
    assert_eq!(members.effective_count(g(0)), 4, "and the cohort");
}

#[test]
fn a_deep_hierarchy_overlaps_from_cohort_to_leaf() {
    // ADR 0035 future acceptance test 14: eight levels, members only at the leaf, and the
    // cohort must still overlap it. Asserts the subtree union rather than a parent read.
    let depth = 8usize;
    let mut groups = vec![group("L0", None, None, DisjointnessDeclaration::NotStated)];
    for level in 1..depth {
        groups.push(group(
            &format!("L{level}"),
            Some(u32::try_from(level - 1).expect("a shallow fixture")),
            None,
            DisjointnessDeclaration::NotStated,
        ));
    }
    let leaf = u32::try_from(depth - 1).expect("small");

    // Rule 4 reaches it by containment with no member rows at all.
    assert_eq!(verdict(&groups, &[], 0, leaf), Overlap::Overlaps);

    // And the union carries the leaf's members all the way up.
    let mut members = GroupMembers::new(groups.len(), 2);
    members.add(g(leaf), student(0));
    let _ = evaluate(&groups, &mut members);
    assert_eq!(members.effective_count(g(0)), 1);
}

#[test]
fn a_cycle_in_the_hierarchy_does_not_hang_the_evaluator() {
    // A damaged file only; the schema's one-step CHECK and the repository's pre-write walk
    // prevent it otherwise. What matters is that evaluation terminates.
    let groups = [
        group("A", Some(1), Some(10), DisjointnessDeclaration::NotStated),
        group("B", Some(2), Some(10), DisjointnessDeclaration::NotStated),
        group("C", Some(0), Some(10), DisjointnessDeclaration::NotStated),
    ];
    let outcome = answer(&groups, &[]);
    assert_eq!(outcome.relation.groups(), 3);
}

// =======================================================================================
// The mask
// =======================================================================================

#[test]
fn a_mask_finds_the_lowest_shared_student() {
    use super::StudentMask;
    let mut a = StudentMask::with_capacity(200);
    let mut b = StudentMask::with_capacity(200);
    for n in [3u32, 70, 130] {
        a.insert(student(n));
    }
    for n in [70u32, 130, 199] {
        b.insert(student(n));
    }
    assert_eq!(a.first_shared(&b), Some(student(70)));
    assert_eq!(b.first_shared(&a), Some(student(70)));
    assert!(a.intersects(&b));
    assert_eq!(a.len(), 3);
}

#[test]
fn disjoint_masks_share_nobody() {
    use super::StudentMask;
    let mut a = StudentMask::with_capacity(64);
    let mut b = StudentMask::with_capacity(64);
    a.insert(student(1));
    b.insert(student(2));
    assert_eq!(a.first_shared(&b), None);
    assert!(!a.intersects(&b));
    assert!(!a.is_empty());
    assert!(StudentMask::with_capacity(64).is_empty());
}

#[test]
fn a_mask_grows_past_its_initial_capacity() {
    use super::StudentMask;
    let mut mask = StudentMask::with_capacity(1);
    mask.insert(student(500));
    assert!(mask.contains(student(500)));
    assert_eq!(mask.iter().collect::<Vec<_>>(), vec![student(500)]);
}
