//! Whether two student groups share a student: ADR 0035 §3's seven ordered rules.
//!
//! # One implementation, three callers
//!
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md) §3
//! defines the function; this module is the **only** implementation of it. The callers are:
//!
//! - **`evara-db`'s loader**, which computes the relation once per solve and records it on
//!   [`Problem::overlap`](super::Problem) — ADR 0035 §5's *"computed once per solve and never
//!   inside the search loop"*.
//! - **`evara-constraints`' `student-clash`**, which reads that compiled relation and never
//!   re-derives it.
//! - **M2's fixtures**, which declare an expected relation and can now be checked against the
//!   function rather than asserting their own output.
//!
//! A second copy in `evara-db` or `evara-constraints` would be the duplication `CLAUDE.md` §2
//! forbids, in the place it would hurt most: rule 3 precedes rule 6, which is what makes a
//! member row beat a declaration, and two evaluators that disagreed would have the product
//! honouring a clash in one place and ignoring it in the other.
//!
//! # Why it lives in `evara-core`
//!
//! It has an answer **before any placement exists**, so `ARCHITECTURE.md` §3 rule 2 excludes
//! it from `evara-constraints` for exactly the reason `lessons::DemandFinding` and
//! [`resolve`](super::resolve) are excluded. It is pure: no I/O, no clock, no mutation.
//!
//! # The order is the specification
//!
//! ```text
//! overlap(A, B):
//!   1. provably_empty(A) or provably_empty(B)                   -> Disjoint
//!   2. A == B                                                   -> Overlaps
//!   3. members(A) ∩ members(B) ≠ ∅                              -> Overlaps      [fact]
//!   4. A ∈ subtree(B) or B ∈ subtree(A)                         -> Overlaps      [containment]
//!   5. ∃ P : A,B ∈ subtree(P), A≠P, B≠P, sizes all declared,
//!        and size(A) + size(B) > size(P)                        -> Overlaps      [pigeonhole]
//!   6. ∃ a ∈ path(A), b ∈ path(B) : a ≠ b,
//!        parent(a) = parent(b) = P, P declares children disjoint -> Disjoint     [§4]
//!   7. otherwise                                                -> Indeterminate
//! ```
//!
//! Three orderings are load-bearing and a reader changing this file should know why.
//!
//! **1 before 2.** A provably empty group against *itself* is `Disjoint`, not `Overlaps` —
//! truth table row 3, *"emptiness precedes identity"*. An empty group competes for nobody,
//! including itself.
//!
//! **3 before 6.** *"If a school declares a parent's children disjoint and then puts one
//! student in two of them, the member row is what happened and the flag is what was
//! intended."* The pair is `Overlaps` and the contradiction is reported — see
//! [`Contradiction`].
//!
//! **4 before 5 and 6.** A parent and a child always overlap, and no declaration about
//! siblings reaches that pair.
//!
//! # What the absence of a member row does not prove
//!
//! Nothing in the schema asserts a roster is finished, so rule 3 can only ever prove
//! *overlap*. Two groups with partial rosters and no shared row are `Indeterminate`, not
//! `Disjoint` — truth table row 15, *"absent rows are not evidence of absence"*. This module
//! never treats an unknown membership as an empty one.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use super::hierarchy::Hierarchy;
use super::index::{GroupIx, StudentIx};
use super::population::{Overlap, OverlapRelation};
use super::problem::GroupFact;

/// Which students are in each group, as the evaluator needs them.
///
/// # Direct rows in, subtree unions out
///
/// A caller adds the **direct** `student_group_member` rows of each group, already filtered
/// to the students who could be in the room — ADR 0035 §2's conjunction: `student.is_active`
/// **and** a `student_school_enrolment` spell at this school whose status is not `LEFT`. That
/// filter is a property of the rows and belongs in whatever reads them.
///
/// [`GroupMembers::resolve`] then unions each group's subtree, because *"a student named in
/// `9-Mat-1` is in Year 9, whether or not a Year 9 member row exists"*. Doing the union here
/// rather than in the caller is what makes `effective_members` and rule 3 read the same set
/// by construction instead of by agreement.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupMembers {
    /// Direct members per group, as a bitset over student indices.
    direct: Vec<StudentMask>,
    /// Subtree unions, filled in by [`GroupMembers::resolve`].
    effective: Vec<StudentMask>,
    /// How many students the masks are sized for.
    students: usize,
}

impl GroupMembers {
    /// An empty membership over `groups` groups and `students` students.
    ///
    /// A school with no `student` rows passes `students = 0`, which is the ADR 0006 case and
    /// is not a degenerate one: rules 1, 4, 5 and 6 all still decide pairs without a single
    /// member row.
    #[must_use]
    pub fn new(groups: usize, students: usize) -> Self {
        Self {
            direct: vec![StudentMask::with_capacity(students); groups],
            effective: Vec::new(),
            students,
        }
    }

    /// Records that `student` is a direct member of `group`.
    ///
    /// Out-of-range indices are ignored rather than panicking, for the reason
    /// [`Hierarchy::of`] ignores a dangling parent: this type is sometimes built from data
    /// `Problem::integrity` has not yet been run over.
    pub fn add(&mut self, group: GroupIx, student: StudentIx) {
        if let Some(mask) = self.direct.get_mut(group.get()) {
            mask.insert(student);
        }
    }

    /// Unions each group's subtree, after which the evaluator may be run.
    ///
    /// Idempotent, and cheap: one pass per group over its subtree, with the masks ORed. At
    /// the sizes in question — a few hundred groups, a few thousand students — this is tens
    /// of microseconds.
    pub fn resolve(&mut self, hierarchy: &Hierarchy) {
        self.effective = (0..self.direct.len())
            .map(|at| {
                let root = GroupIx::new(u32::try_from(at).unwrap_or(u32::MAX));
                let mut union = StudentMask::with_capacity(self.students);
                for member in hierarchy.subtree_of(root) {
                    if let Some(mask) = self.direct.get(member.get()) {
                        union.union_with(mask);
                    }
                }
                union
            })
            .collect();
    }

    /// The students of `subtree(group)` — ADR 0035 §2's `effective_members`, as a set.
    ///
    /// Empty before [`GroupMembers::resolve`] has been called, which is why the evaluator
    /// calls it rather than trusting a caller to.
    #[must_use]
    pub fn effective(&self, group: GroupIx) -> Option<&StudentMask> {
        self.effective.get(group.get())
    }

    /// How many students are in `subtree(group)`.
    ///
    /// What [`GroupFact::effective_members`] is set from, so the count and the set cannot
    /// disagree.
    #[must_use]
    pub fn effective_count(&self, group: GroupIx) -> i64 {
        self.effective(group)
            .map_or(0, |mask| i64::try_from(mask.len()).unwrap_or(i64::MAX))
    }

    /// Whether the two groups' subtrees share a student, and which one.
    ///
    /// Returns the **lowest** shared student index, so a contradiction report names the same
    /// student on every run.
    #[must_use]
    pub fn first_shared(&self, a: GroupIx, b: GroupIx) -> Option<StudentIx> {
        match (self.effective(a), self.effective(b)) {
            (Some(a), Some(b)) => a.first_shared(b),
            _ => None,
        }
    }
}

/// A set of students, as a bitset over dense student indices.
///
/// Written out rather than generalised over the index type, which would mean reworking
/// [`GroupMask`](super::GroupMask) and [`SlotMask`](super::SlotMask) — both of which carry a
/// `serde(transparent)` wire format M2 fixed. A generic bitset is a worthwhile tidy-up and is
/// not M5's business; this one deliberately offers only what rule 3 and `effective_members`
/// need.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StudentMask {
    words: Vec<u64>,
}

impl StudentMask {
    /// A mask sized for `students` students.
    #[must_use]
    pub fn with_capacity(students: usize) -> Self {
        Self {
            words: vec![0; students.div_ceil(64)],
        }
    }

    /// Adds a student.
    pub fn insert(&mut self, student: StudentIx) {
        let (word, bit) = (student.get() / 64, student.get() % 64);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1u64 << bit;
    }

    /// Whether the student is in the set.
    #[must_use]
    pub fn contains(&self, student: StudentIx) -> bool {
        let (word, bit) = (student.get() / 64, student.get() % 64);
        self.words.get(word).is_some_and(|w| w & (1u64 << bit) != 0)
    }

    /// Adds every student of `other`.
    pub fn union_with(&mut self, other: &Self) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (slot, word) in self.words.iter_mut().zip(other.words.iter()) {
            *slot |= *word;
        }
    }

    /// How many students are in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// The lowest student in both sets, or `None` where they are disjoint.
    ///
    /// One pass of word-wise `AND`, so rule 3 costs `O(students / 64)` per pair rather than a
    /// scan of two rosters.
    #[must_use]
    pub fn first_shared(&self, other: &Self) -> Option<StudentIx> {
        for (at, (a, b)) in self.words.iter().zip(other.words.iter()).enumerate() {
            let both = a & b;
            if both != 0 {
                let bit = both.trailing_zeros() as usize;
                return Some(StudentIx::new(
                    u32::try_from(at * 64 + bit).unwrap_or(u32::MAX),
                ));
            }
        }
        None
    }

    /// Whether the two sets share anybody.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.first_shared(other).is_some()
    }

    /// The students in the set, in index order.
    pub fn iter(&self) -> impl Iterator<Item = StudentIx> + '_ {
        self.words.iter().enumerate().flat_map(|(word, bits)| {
            (0..64)
                .filter(move |bit| bits & (1u64 << bit) != 0)
                .map(move |bit| StudentIx::new(u32::try_from(word * 64 + bit).unwrap_or(u32::MAX)))
        })
    }
}

/// A school's declaration contradicted by its own member rows.
///
/// ADR 0035 §6.3: *"Contradictions are named, not resolved."* A parent declared its children
/// disjoint and a student is in two of them, so rule 3 beats rule 6, the pair is `Overlaps`,
/// and this says which rows disagree. Nothing is overridden and nothing is silently
/// preferred — the alternative, honouring the flag, would hide a data error that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contradiction {
    /// One of the two groups. Always the lower index, so the report is stable.
    pub a: GroupIx,
    /// The other.
    pub b: GroupIx,
    /// The ancestor whose declaration the member row breaks.
    pub parent: GroupIx,
    /// A student in both groups' subtrees. The lowest such index, for a stable message.
    pub student: StudentIx,
}

/// The compiled relation and everything the evaluation noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlapOutcome {
    /// The three-state answer for every pair.
    pub relation: OverlapRelation,
    /// Declarations the member rows contradict, in `(a, b)` index order.
    pub contradictions: Vec<Contradiction>,
}

/// Evaluates ADR 0035 §3 for every pair of `groups`.
///
/// `members` is mutated only to compute its subtree unions — see [`GroupMembers::resolve`] —
/// so a caller may hand over a freshly built set of direct rows and get the effective sets
/// back for `GroupFact::effective_members`.
///
/// # Determinism
///
/// Pairs are visited in `(a, b)` index order with `a <= b`, the relation is written
/// symmetrically, and contradictions are returned in that same order. Nothing iterates a
/// hash map. Two runs over one problem produce an identical outcome.
///
/// # The diagonal is answered, not skipped
///
/// `overlap(g, g)` is `Overlaps` by rule 2 — or `Disjoint` by rule 1 where the group is
/// provably empty. M2's fixtures and M4's loader both left it `Indeterminate`, which
/// `PRE-SOLVE-VALIDATION.md` §14's limitation 9 records as a certainty left unstated. It is
/// stated here.
#[must_use]
pub fn evaluate(groups: &[GroupFact], members: &mut GroupMembers) -> OverlapOutcome {
    let hierarchy = Hierarchy::of(groups);
    members.resolve(&hierarchy);

    let empty: Vec<bool> = (0..groups.len())
        .map(|at| provably_empty(groups, &hierarchy, members, index(at)))
        .collect();

    let mut relation = OverlapRelation::unknown(groups.len());
    let mut contradictions = Vec::new();

    for a in 0..groups.len() {
        for b in a..groups.len() {
            let (ia, ib) = (index(a), index(b));
            let verdict = decide(
                groups,
                &hierarchy,
                members,
                &empty,
                ia,
                ib,
                &mut contradictions,
            );
            relation.set(ia, ib, verdict);
        }
    }

    OverlapOutcome {
        relation,
        contradictions,
    }
}

/// The seven rules, in order, first match winning.
///
/// Split out so the ordering is one readable block rather than a nest of conditions, and so a
/// reader comparing it against ADR 0035 §3 can do so line by line.
fn decide(
    groups: &[GroupFact],
    hierarchy: &Hierarchy,
    members: &GroupMembers,
    empty: &[bool],
    a: GroupIx,
    b: GroupIx,
    contradictions: &mut Vec<Contradiction>,
) -> Overlap {
    // Rule 1 — an empty group competes for nobody, including itself.
    if empty.get(a.get()).copied().unwrap_or(false) || empty.get(b.get()).copied().unwrap_or(false)
    {
        return Overlap::Disjoint;
    }

    // Rule 2 — identity.
    if a == b {
        return Overlap::Overlaps;
    }

    // Rule 3 — a shared member row is a fact, and it beats rule 6's declaration. Where it
    // does so, the disagreement is recorded rather than resolved.
    if let Some(student) = members.first_shared(a, b) {
        if let Some(parent) = declared_disjoint_ancestor(groups, hierarchy, a, b) {
            contradictions.push(Contradiction {
                a,
                b,
                parent,
                student,
            });
        }
        return Overlap::Overlaps;
    }

    // Rule 4 — containment. A non-empty descendant's students are in its ancestor by
    // declaration, and rule 1 has already removed the emptiness subtlety.
    if hierarchy.in_subtree(a, b) || hierarchy.in_subtree(b, a) {
        return Overlap::Overlaps;
    }

    // Rule 5 — pigeonhole. Both are inside P, so their populations are subsets of P's; if the
    // declared sizes sum past it they must intersect. Arithmetic, not a guess.
    if pigeonhole(groups, hierarchy, a, b) {
        return Overlap::Overlaps;
    }

    // Rule 6 — the parent's declaration, matched on the ancestors so that it propagates to
    // descendants (truth table row 12).
    if declared_disjoint_ancestor(groups, hierarchy, a, b).is_some() {
        return Overlap::Disjoint;
    }

    // Rule 7.
    Overlap::Indeterminate
}

/// Whether `group` provably holds nobody — ADR 0035 §2's `provably_empty`.
///
/// Three conditions, all required: the group declares `size = 0`, its effective membership is
/// empty, and **no group in its subtree declares a size above zero**. The third is what stops
/// a cohort declared `0` by mistake from silencing every clash beneath it.
///
/// A group that has declared *no* size is never provably empty: `None` means "I have not
/// said", which is not a statement that nobody is there.
fn provably_empty(
    groups: &[GroupFact],
    hierarchy: &Hierarchy,
    members: &GroupMembers,
    group: GroupIx,
) -> bool {
    let Some(fact) = groups.get(group.get()) else {
        return false;
    };
    if fact.declared_size != Some(0) {
        return false;
    }
    if members.effective(group).is_some_and(|set| !set.is_empty()) {
        return false;
    }
    hierarchy
        .subtree_of(group)
        .into_iter()
        .filter_map(|member| groups.get(member.get()))
        .all(|member| member.declared_size.is_none_or(|size| size <= 0))
}

/// Rule 5's arithmetic.
///
/// Looks for a common ancestor `P` — which may be either group's own ancestor, and is never
/// one of the pair itself — under which the two declared sizes do not fit.
fn pigeonhole(groups: &[GroupFact], hierarchy: &Hierarchy, a: GroupIx, b: GroupIx) -> bool {
    let (Some(size_a), Some(size_b)) = (declared(groups, a), declared(groups, b)) else {
        return false;
    };
    hierarchy
        .ancestors_of(a)
        .iter()
        .filter(|p| **p != a && **p != b && hierarchy.in_subtree(**p, b))
        .filter_map(|p| declared(groups, *p))
        .any(|room| size_a + size_b > room)
}

/// Rule 6's search: an ancestor pair separated under a parent that declares its children
/// disjoint.
///
/// Matched on `path(A)` against `path(B)` rather than on the two groups directly, which is
/// what makes disjointness **propagate to descendants** — truth table row 12, where
/// `9A-Reading` and `9B-Reading` are disjoint because `9A` and `9B` are.
///
/// Returns the declaring parent, because rule 3 needs it to name the contradiction.
fn declared_disjoint_ancestor(
    groups: &[GroupFact],
    hierarchy: &Hierarchy,
    a: GroupIx,
    b: GroupIx,
) -> Option<GroupIx> {
    for up_a in hierarchy.path_of(a) {
        for up_b in hierarchy.path_of(b) {
            if up_a == up_b {
                continue;
            }
            let (Some(fact_a), Some(fact_b)) = (groups.get(up_a.get()), groups.get(up_b.get()))
            else {
                continue;
            };
            let (Some(parent_a), Some(parent_b)) = (fact_a.parent, fact_b.parent) else {
                continue;
            };
            if parent_a != parent_b {
                continue;
            }
            if groups
                .get(parent_a.get())
                .is_some_and(|parent| parent.children_are_disjoint.proves_children_disjoint())
            {
                return Some(parent_a);
            }
        }
    }
    None
}

/// A group's declared size, where it has one.
fn declared(groups: &[GroupFact], group: GroupIx) -> Option<i64> {
    groups.get(group.get()).and_then(|g| g.declared_size)
}

/// The index at `position`.
fn index(position: usize) -> GroupIx {
    GroupIx::new(u32::try_from(position).unwrap_or(u32::MAX))
}
