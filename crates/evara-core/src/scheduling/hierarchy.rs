//! The student-group hierarchy, read once and asked many questions.
//!
//! # Why this is a type rather than three loops
//!
//! Four things read the parent edge: M3's count of unanswered declarations, and rules 1, 4,
//! 5 and 6 of
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) §3.
//! [ADR 0038](../../../../docs/adr/0038-counting-unanswered-questions-is-not-evaluating-overlap.md)
//! §3 asks M5 to reuse M3's walk rather than write a second — *"not because the walk is hard,
//! but because two readings of a hierarchy are two places for a `MAX_GROUP_DEPTH` assumption
//! to differ."*
//!
//! So there is one walk, here, and [`MAX_GROUP_DEPTH`] is defined once. `evara-db`'s
//! repository guard reads this constant rather than its own copy.
//!
//! # A damaged hierarchy stops rather than spins
//!
//! `student_group` carries a one-step cycle `CHECK` (`V10:141`) and the repository walks the
//! parents before any write, so a longer cycle should not exist. "Should not" is not
//! "cannot": a damaged file could hold one, and every walk here is depth-bounded so that it
//! terminates on such a file instead of looping. `Problem::integrity` reports the one-step
//! case as [`GroupIsItsOwnParent`](super::IntegrityError::GroupIsItsOwnParent).

use super::index::GroupIx;
use super::problem::GroupFact;

/// How far a walk of the parent edge will go before giving up.
///
/// Schools nest a few levels — school, cohort, band, set. Anything beyond this is a mistake
/// or a loop, and either way stopping is better than continuing.
///
/// **The single definition.** `evara_db::repo::people` walks the parents upward before a
/// write and reads this, rather than holding a second copy that could drift from it.
pub const MAX_GROUP_DEPTH: usize = 64;

/// The parent edge of one school's groups, indexed for the questions overlap asks.
///
/// Built once per solve. Every query is a slice read or a bounded walk; nothing re-scans the
/// group table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hierarchy {
    /// How many groups the hierarchy covers.
    groups: usize,
    /// Direct children of each group, in index order.
    children: Vec<Vec<GroupIx>>,
    /// Each group's ancestors, nearest first, **excluding** the group itself.
    ///
    /// Materialised rather than walked per query, because rules 5 and 6 ask for it once per
    /// pair and a school has far more pairs than groups.
    ancestors: Vec<Vec<GroupIx>>,
}

impl Hierarchy {
    /// Reads the parent edge of `groups`.
    ///
    /// A parent index out of range is ignored rather than panicking: `Problem::integrity`
    /// reports it as a dangling reference, and this type is sometimes asked about a problem
    /// that has not been checked yet.
    #[must_use]
    pub fn of(groups: &[GroupFact]) -> Self {
        let count = groups.len();
        let mut children: Vec<Vec<GroupIx>> = vec![Vec::new(); count];
        for (at, group) in groups.iter().enumerate() {
            if let Some(parent) = group.parent
                && parent.get() < count
                && parent.get() != at
            {
                children[parent.get()].push(ix(at));
            }
        }

        let ancestors = (0..count)
            .map(|at| {
                let mut path = Vec::new();
                let mut here = groups[at].parent;
                // Bounded, and additionally stopped by revisiting: a cycle the schema's
                // one-step CHECK does not cover would otherwise be walked MAX_GROUP_DEPTH
                // times for every group in it.
                while let Some(parent) = here {
                    if parent.get() >= count || path.contains(&parent) || parent.get() == at {
                        break;
                    }
                    path.push(parent);
                    if path.len() >= MAX_GROUP_DEPTH {
                        break;
                    }
                    here = groups[parent.get()].parent;
                }
                path
            })
            .collect();

        Self {
            groups: count,
            children,
            ancestors,
        }
    }

    /// How many groups this covers.
    #[must_use]
    pub const fn groups(&self) -> usize {
        self.groups
    }

    /// The direct children of `group`, in index order.
    #[must_use]
    pub fn children_of(&self, group: GroupIx) -> &[GroupIx] {
        self.children.get(group.get()).map_or(&[], Vec::as_slice)
    }

    /// How many direct children `group` has.
    ///
    /// What M3's count of unanswered declarations reads: a declaration about fewer than two
    /// children settles no pair, so asking for it would send the user to a field that cannot
    /// help (ADR 0038 §1).
    #[must_use]
    pub fn child_count(&self, group: GroupIx) -> usize {
        self.children_of(group).len()
    }

    /// `group`'s ancestors, nearest first, excluding itself.
    #[must_use]
    pub fn ancestors_of(&self, group: GroupIx) -> &[GroupIx] {
        self.ancestors.get(group.get()).map_or(&[], Vec::as_slice)
    }

    /// `group` and its ancestors, nearest first — ADR 0035 §3's `path(X)`.
    #[must_use]
    pub fn path_of(&self, group: GroupIx) -> Vec<GroupIx> {
        let mut path = vec![group];
        path.extend_from_slice(self.ancestors_of(group));
        path
    }

    /// Whether `ancestor` is a strict ancestor of `descendant`.
    #[must_use]
    pub fn is_ancestor_of(&self, ancestor: GroupIx, descendant: GroupIx) -> bool {
        self.ancestors_of(descendant).contains(&ancestor)
    }

    /// Whether `member` is in `subtree(root)` — `root` itself included.
    ///
    /// ADR 0035 §2's `subtree(g)`: *"`g` together with every transitive descendant through
    /// `parent_group_id`."*
    #[must_use]
    pub fn in_subtree(&self, root: GroupIx, member: GroupIx) -> bool {
        root == member || self.is_ancestor_of(root, member)
    }

    /// Every group of `subtree(root)`, `root` first, then descendants in index order.
    ///
    /// Deterministic: breadth-first over `children_of`, which is itself in index order.
    #[must_use]
    pub fn subtree_of(&self, root: GroupIx) -> Vec<GroupIx> {
        let mut found = vec![root];
        let mut at = 0;
        while at < found.len() {
            let here = found[at];
            at += 1;
            for child in self.children_of(here) {
                // A damaged file could point two parents at one child, or close a cycle the
                // one-step CHECK does not cover. Either way a group is visited once.
                if !found.contains(child) {
                    found.push(*child);
                }
            }
        }
        found
    }

    /// Groups with two or more children that have not answered their disjointness question.
    ///
    /// The one implementation of *"which parents have children and have not answered"*, which
    /// ADR 0038 §3 names. M3's warning reads it to name the fields a user can go and fill in.
    #[must_use]
    pub fn unanswered_parents(&self, groups: &[GroupFact]) -> Vec<GroupIx> {
        (0..self.groups.min(groups.len()))
            .map(ix)
            .filter(|group| {
                self.child_count(*group) >= 2
                    && groups[group.get()].children_are_disjoint.is_unanswered()
            })
            .collect()
    }
}

/// The index at `position`.
fn ix(position: usize) -> GroupIx {
    GroupIx::new(u32::try_from(position).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{Hierarchy, MAX_GROUP_DEPTH};
    use crate::scheduling::index::GroupIx;
    use crate::scheduling::population::DisjointnessDeclaration;
    use crate::scheduling::problem::{GroupFact, GroupKind};

    fn group(n: u128, parent: Option<u32>, declared: DisjointnessDeclaration) -> GroupFact {
        GroupFact {
            id: Uuid::from_u128(n),
            name: format!("G{n}"),
            kind: GroupKind::Class,
            parent: parent.map(GroupIx::new),
            declared_size: Some(10),
            effective_members: 0,
            children_are_disjoint: declared,
        }
    }

    /// `Year 9 → { Classes → { 9A, 9B }, Sets → { S1 } }`, the ADR 0035 §4 shape.
    fn cohort() -> Vec<GroupFact> {
        vec![
            group(0, None, DisjointnessDeclaration::MayOverlap), // 0 Year 9
            group(1, Some(0), DisjointnessDeclaration::Disjoint), // 1 Classes
            group(2, Some(1), DisjointnessDeclaration::NotStated), // 2 9A
            group(3, Some(1), DisjointnessDeclaration::NotStated), // 3 9B
            group(4, Some(0), DisjointnessDeclaration::Disjoint), // 4 Sets
            group(5, Some(4), DisjointnessDeclaration::NotStated), // 5 S1
        ]
    }

    #[test]
    fn children_are_read_in_index_order() {
        let h = Hierarchy::of(&cohort());
        assert_eq!(
            h.children_of(GroupIx::new(0)),
            [GroupIx::new(1), GroupIx::new(4)]
        );
        assert_eq!(
            h.children_of(GroupIx::new(1)),
            [GroupIx::new(2), GroupIx::new(3)]
        );
        assert_eq!(h.children_of(GroupIx::new(2)), []);
        assert_eq!(h.child_count(GroupIx::new(0)), 2);
        assert_eq!(h.child_count(GroupIx::new(2)), 0);
    }

    #[test]
    fn a_path_is_the_group_then_its_ancestors_nearest_first() {
        // ADR 0035 §3's `path(X)`, which rule 6 walks on both sides of the pair.
        let h = Hierarchy::of(&cohort());
        assert_eq!(
            h.path_of(GroupIx::new(2)),
            [GroupIx::new(2), GroupIx::new(1), GroupIx::new(0)]
        );
        assert_eq!(h.path_of(GroupIx::new(0)), [GroupIx::new(0)]);
    }

    #[test]
    fn ancestry_is_strict_and_subtree_membership_is_not() {
        let h = Hierarchy::of(&cohort());
        assert!(h.is_ancestor_of(GroupIx::new(0), GroupIx::new(2)));
        assert!(!h.is_ancestor_of(GroupIx::new(2), GroupIx::new(0)));
        assert!(
            !h.is_ancestor_of(GroupIx::new(2), GroupIx::new(2)),
            "a group is not its own ancestor"
        );
        assert!(
            h.in_subtree(GroupIx::new(2), GroupIx::new(2)),
            "but it is in its own subtree — ADR 0035 §2"
        );
    }

    #[test]
    fn a_subtree_is_deterministic_and_includes_its_root() {
        let h = Hierarchy::of(&cohort());
        assert_eq!(
            h.subtree_of(GroupIx::new(0)),
            [
                GroupIx::new(0),
                GroupIx::new(1),
                GroupIx::new(4),
                GroupIx::new(2),
                GroupIx::new(3),
                GroupIx::new(5),
            ]
        );
        assert_eq!(h.subtree_of(GroupIx::new(5)), [GroupIx::new(5)]);
    }

    #[test]
    fn only_a_parent_of_two_or_more_is_asked_the_question() {
        // ADR 0038 §1: a declaration about one child settles no pair.
        let mut groups = cohort();
        groups[1].children_are_disjoint = DisjointnessDeclaration::NotStated;
        let h = Hierarchy::of(&groups);
        assert_eq!(h.unanswered_parents(&groups), [GroupIx::new(1)]);

        // `Sets` has one child, so it is never counted however it answers.
        groups[4].children_are_disjoint = DisjointnessDeclaration::NotStated;
        let h = Hierarchy::of(&groups);
        assert_eq!(
            h.unanswered_parents(&groups),
            [GroupIx::new(1)],
            "a parent with a single child is not a question worth asking"
        );
    }

    #[test]
    fn an_answered_parent_is_not_counted() {
        let groups = cohort();
        let h = Hierarchy::of(&groups);
        assert_eq!(
            h.unanswered_parents(&groups),
            Vec::new(),
            "Year 9 answered MayOverlap and Classes answered Disjoint"
        );
    }

    #[test]
    fn a_dangling_parent_is_ignored_rather_than_panicking() {
        // `Problem::integrity` reports it; this type is asked about unchecked problems.
        let groups = vec![group(0, Some(9), DisjointnessDeclaration::NotStated)];
        let h = Hierarchy::of(&groups);
        assert_eq!(h.ancestors_of(GroupIx::new(0)), []);
        assert_eq!(h.subtree_of(GroupIx::new(0)), [GroupIx::new(0)]);
    }

    #[test]
    fn a_group_that_is_its_own_parent_terminates() {
        let groups = vec![group(0, Some(0), DisjointnessDeclaration::NotStated)];
        let h = Hierarchy::of(&groups);
        assert_eq!(h.ancestors_of(GroupIx::new(0)), []);
        assert_eq!(h.child_count(GroupIx::new(0)), 0);
    }

    #[test]
    fn a_longer_cycle_terminates_too() {
        // 0 → 1 → 2 → 0. The schema's one-step CHECK does not refuse this, and the
        // repository's pre-write walk is the only thing that normally prevents it, so a
        // damaged file can hold one.
        let groups = vec![
            group(0, Some(1), DisjointnessDeclaration::NotStated),
            group(1, Some(2), DisjointnessDeclaration::NotStated),
            group(2, Some(0), DisjointnessDeclaration::NotStated),
        ];
        let h = Hierarchy::of(&groups);
        for at in 0..3 {
            assert!(
                h.ancestors_of(GroupIx::new(at)).len() < MAX_GROUP_DEPTH,
                "the walk stopped"
            );
        }
        assert_eq!(h.subtree_of(GroupIx::new(0)).len(), 3, "each group once");
    }

    #[test]
    fn a_deep_chain_is_bounded_by_the_shared_limit() {
        let deep: Vec<GroupFact> = (0..(MAX_GROUP_DEPTH + 20))
            .map(|n| {
                group(
                    n as u128,
                    n.checked_sub(1).map(|p| u32::try_from(p).expect("small")),
                    DisjointnessDeclaration::NotStated,
                )
            })
            .collect();
        let h = Hierarchy::of(&deep);
        let deepest = GroupIx::new(u32::try_from(deep.len() - 1).expect("small"));
        assert_eq!(h.ancestors_of(deepest).len(), MAX_GROUP_DEPTH);
    }

    #[test]
    fn an_empty_hierarchy_answers_everything_emptily() {
        let h = Hierarchy::of(&[]);
        assert_eq!(h.groups(), 0);
        assert_eq!(h.children_of(GroupIx::new(0)), []);
        assert_eq!(h.unanswered_parents(&[]), Vec::new());
    }
}
