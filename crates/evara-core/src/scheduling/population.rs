//! Who is in the room: student groups, whether two of them share a child, and how many
//! people a section's lesson should be assumed to hold.
//!
//! # The two decisions this module carries
//!
//! [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
//! — *whether two student groups share students has three answers, and indeterminate is one
//! of them.* Only [`Overlap::Overlaps`] is a clash; [`Overlap::Disjoint`] must be **proved**;
//! everything else is [`Overlap::Indeterminate`], which is never silently converted to
//! either.
//!
//! [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
//! — *expected attendance is a stated number or it is unknown.*
//! [`ExpectedAttendance::Unknown`] is a variant, not a sentinel, and in particular it is
//! never `0`.
//!
//! # What is **not** here
//!
//! The overlap *algorithm* — ADR 0035 §3's seven ordered rules — is not implemented in this
//! milestone. It belongs with `student-clash` in M5, and it needs a database column that
//! does not exist yet (§4, and [`DisjointnessDeclaration`] below). What is here is the
//! vocabulary those rules produce and consume: the three-state answer, the declaration the
//! future column stores, the inputs each rule reads, and the compiled bitset form the inner
//! loop tests against.
//!
//! The same applies to attendance: [`ExpectedAttendance`] is the two-state result ADR 0034
//! §3 defines, and the derivation that produces it is M4's.

use serde::{Deserialize, Serialize};

use super::index::GroupIx;

/// Whether two student groups have a student in common.
///
/// Three values, and the third is the point.
/// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// §6 sets out why neither of the two-valued readings works: treating the unproven case as
/// a clash means a year group can hold one lesson at a time, and treating it as disjoint
/// puts a child in two rooms at once while both timetables report themselves clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Overlap {
    /// At least one student is in both. A hard `student-clash`.
    Overlaps,
    /// No student is in both, **proved** from stored facts — a member intersection that is
    /// empty is not a proof, because nothing asserts a roster is finished.
    Disjoint,
    /// The stored data does not decide it.
    ///
    /// Not a clash, not a licence, and counted. ADR 0035 §6: enforce what is proved, and
    /// make what is not proved impossible to miss.
    Indeterminate,
}

impl Overlap {
    /// Whether this answer makes two concurrent placements a hard violation.
    ///
    /// Only [`Overlap::Overlaps`] does. Written as a method so no caller can reach for
    /// `!= Disjoint` and quietly promote [`Overlap::Indeterminate`] into a clash.
    #[must_use]
    pub const fn is_clash(self) -> bool {
        matches!(self, Self::Overlaps)
    }

    /// Whether the stored data settled the question either way.
    ///
    /// The count of pairs for which this is `false` is what pre-flight reports, and what a
    /// generated timetable carries beside its clash figure — ADR 0035 §6.
    #[must_use]
    pub const fn is_decided(self) -> bool {
        !matches!(self, Self::Indeterminate)
    }
}

/// What a parent group declares about its children: the future
/// `student_group.children_are_disjoint` column.
///
/// # The column this mirrors is not written yet
///
/// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// §4 specifies it, and `SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1 schedules it for **M5**,
/// in the migration that already adds `constraint_instance` / `constraint_ref`:
///
/// ```sql
/// -- FUTURE. Not in V10, and not written by M2.
/// ALTER TABLE student_group
///     ADD COLUMN children_are_disjoint INTEGER
///         CHECK (children_are_disjoint IS NULL OR children_are_disjoint IN (0, 1));
/// ```
///
/// This milestone declares the in-memory value so that the fixtures can express the cases
/// ADR 0035's truth table requires, and so M5 implements rule 6 against a named type rather
/// than against an `Option<bool>` whose `None` somebody will eventually read as "no".
///
/// # The mapping, which is exact and must not be collapsed
///
/// | Column | Variant | Meaning |
/// | --- | --- | --- |
/// | `NULL` | [`DisjointnessDeclaration::NotStated`] | Nobody has answered. Yields [`Overlap::Indeterminate`] |
/// | `1` | [`DisjointnessDeclaration::Disjoint`] | *No two of my children share a student.* Feeds rule 6 |
/// | `0` | [`DisjointnessDeclaration::MayOverlap`] | An explicit *they may overlap* — worth distinguishing from silence, because it tells pre-flight to stop asking |
///
/// `NotStated` is **not** `MayOverlap`. They produce the same overlap answer and different
/// pre-flight behaviour, which is the whole reason the column is nullable rather than
/// `NOT NULL DEFAULT 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisjointnessDeclaration {
    /// `NULL`: not stated. The state every existing project is in, and the default.
    #[default]
    NotStated,
    /// `1`: no two children of this group share a student.
    Disjoint,
    /// `0`: the children may overlap, said deliberately.
    MayOverlap,
}

impl DisjointnessDeclaration {
    /// Whether this declaration can prove two of the group's children disjoint.
    ///
    /// True only for [`DisjointnessDeclaration::Disjoint`]. Note that a declaration never
    /// *wins*: ADR 0035 §3 puts the member-row check (rule 3) before the declaration
    /// (rule 6) on purpose, so a shared member row makes the pair `Overlaps` and the
    /// contradiction is reported.
    #[must_use]
    pub const fn proves_children_disjoint(self) -> bool {
        matches!(self, Self::Disjoint)
    }

    /// Whether pre-flight should still be asking the school this question.
    ///
    /// Only [`DisjointnessDeclaration::NotStated`] is an unanswered question. An explicit
    /// `MayOverlap` has been answered, even though the answer does not resolve any pair.
    #[must_use]
    pub const fn is_unanswered(self) -> bool {
        matches!(self, Self::NotStated)
    }

    /// Reads the column's three stored states.
    ///
    /// `None` is `NULL`. Provided so the M5 loader has one place to do this rather than a
    /// `match` at each call site.
    #[must_use]
    pub const fn from_column(value: Option<bool>) -> Self {
        match value {
            None => Self::NotStated,
            Some(true) => Self::Disjoint,
            Some(false) => Self::MayOverlap,
        }
    }

    /// The value this would be stored as.
    #[must_use]
    pub const fn to_column(self) -> Option<bool> {
        match self {
            Self::NotStated => None,
            Self::Disjoint => Some(true),
            Self::MayOverlap => Some(false),
        }
    }
}

/// A set of group indices, as a bitset.
///
/// [ADR 0035](../../../../docs/adr/0035-student-group-overlap-is-a-three-state-answer.md)
/// §5's compiled form. Over the groups a school attaches to class sections — hundreds at
/// most — a membership test is a handful of `u64` words, so the student-clash check costs
/// the same whether the school has thirty groups or three hundred.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupMask {
    /// One bit per group index, least-significant bit first within each word.
    words: Vec<u64>,
}

impl GroupMask {
    /// An empty mask with room for `groups` indices.
    #[must_use]
    pub fn with_capacity(groups: usize) -> Self {
        Self {
            words: vec![0; groups.div_ceil(64)],
        }
    }

    /// Adds a group, growing the mask if it does not reach that far yet.
    pub fn insert(&mut self, group: GroupIx) {
        let (word, bit) = (group.get() / 64, group.get() % 64);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1u64 << bit;
    }

    /// Whether a group is in the set.
    #[must_use]
    pub fn contains(&self, group: GroupIx) -> bool {
        let (word, bit) = (group.get() / 64, group.get() % 64);
        self.words.get(word).is_some_and(|w| w & (1u64 << bit) != 0)
    }

    /// Whether the two sets share a group.
    ///
    /// The whole student-clash test: two activities conflict exactly when one's clash mask
    /// intersects the other's audience mask.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(other.words.iter())
            .any(|(a, b)| a & b != 0)
    }

    /// Adds every group of `other`.
    pub fn union_with(&mut self, other: &Self) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (slot, word) in self.words.iter_mut().zip(other.words.iter()) {
            *slot |= *word;
        }
    }

    /// How many groups are in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether no group is in the set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    /// The groups in the set, in ascending index order.
    ///
    /// Ascending so that anything built from a mask — a diagnostic's entity list, a
    /// serialised fixture — is deterministic.
    pub fn iter(&self) -> impl Iterator<Item = GroupIx> + '_ {
        self.words.iter().enumerate().flat_map(|(word, bits)| {
            (0..64)
                .filter(move |bit| bits & (1u64 << bit) != 0)
                .map(move |bit| GroupIx::new(u32::try_from(word * 64 + bit).unwrap_or(u32::MAX)))
        })
    }
}

/// The three-state overlap answer for every pair of a school's groups.
///
/// Symmetric and total: every ordered pair has an answer, and `get(a, b) == get(b, a)`. A
/// group against itself is [`Overlap::Overlaps`] unless it is provably empty — ADR 0035 §3
/// rules 1 and 2 — which is why the diagonal is stored rather than assumed.
///
/// # Compiled once, never inside the search loop
///
/// The relation is `n²` three-state values over the few hundred groups a school attaches to
/// sections. The inner loop does not consult it: it tests the bitsets
/// [`OverlapRelation::clash_mask`] builds from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlapRelation {
    /// How many groups the relation covers.
    groups: usize,
    /// Row-major `groups × groups`, kept symmetric by [`OverlapRelation::set`].
    states: Vec<Overlap>,
}

impl OverlapRelation {
    /// A relation over `groups` groups in which nothing has been decided yet.
    ///
    /// Every pair starts [`Overlap::Indeterminate`], including the diagonal, so a relation
    /// that is never populated reports ignorance rather than safety.
    #[must_use]
    pub fn unknown(groups: usize) -> Self {
        Self {
            groups,
            states: vec![Overlap::Indeterminate; groups * groups],
        }
    }

    /// How many groups the relation covers.
    #[must_use]
    pub const fn groups(&self) -> usize {
        self.groups
    }

    /// The answer for one pair.
    ///
    /// # Panics
    ///
    /// If either index is outside the relation, which is a loader bug rather than a data
    /// state: a `Problem`'s group indices are dense and generated alongside the relation.
    #[must_use]
    pub fn get(&self, a: GroupIx, b: GroupIx) -> Overlap {
        self.states[self.offset(a, b)]
    }

    /// Records the answer for one pair, in both directions.
    ///
    /// # Panics
    ///
    /// As [`OverlapRelation::get`].
    pub fn set(&mut self, a: GroupIx, b: GroupIx, overlap: Overlap) {
        let forward = self.offset(a, b);
        let backward = self.offset(b, a);
        self.states[forward] = overlap;
        self.states[backward] = overlap;
    }

    /// Every group that is known to overlap `group`, as a bitset.
    ///
    /// ADR 0035 §5's `overlaps_with`. Only [`Overlap::Overlaps`] contributes —
    /// [`Overlap::Indeterminate`] deliberately does not, which is what makes the compiled
    /// form enforce exactly what is proved.
    #[must_use]
    pub fn overlaps_with(&self, group: GroupIx) -> GroupMask {
        let mut mask = GroupMask::with_capacity(self.groups);
        for other in 0..self.groups {
            let other = GroupIx::new(u32::try_from(other).unwrap_or(u32::MAX));
            if self.get(group, other).is_clash() {
                mask.insert(other);
            }
        }
        mask
    }

    /// The groups whose lessons may not run beside an audience of `audience`.
    ///
    /// ADR 0035 §5: `clash_mask(a) = audience_mask(a) ∪ ⋃ { overlaps_with[g] : g ∈ audience(a) }`.
    /// Two activities conflict exactly when
    /// `clash_mask(A).intersects(&audience_mask(B))`.
    #[must_use]
    pub fn clash_mask(&self, audience: &GroupMask) -> GroupMask {
        let mut mask = audience.clone();
        for group in audience.iter() {
            mask.union_with(&self.overlaps_with(group));
        }
        mask
    }

    /// How many unordered pairs the stored data did not settle.
    ///
    /// The figure pre-flight reports and a generated timetable carries beside its clash
    /// count — ADR 0035 §6. The diagonal is excluded: a group against itself is not a pair.
    #[must_use]
    pub fn indeterminate_pairs(&self) -> usize {
        let mut count = 0;
        for a in 0..self.groups {
            for b in (a + 1)..self.groups {
                let (a, b) = (
                    GroupIx::new(u32::try_from(a).unwrap_or(u32::MAX)),
                    GroupIx::new(u32::try_from(b).unwrap_or(u32::MAX)),
                );
                if !self.get(a, b).is_decided() {
                    count += 1;
                }
            }
        }
        count
    }

    /// Row-major offset of one ordered pair.
    fn offset(&self, a: GroupIx, b: GroupIx) -> usize {
        assert!(
            a.get() < self.groups && b.get() < self.groups,
            "group index out of range for a relation over {} groups",
            self.groups
        );
        a.get() * self.groups + b.get()
    }
}

/// How many people a lesson of one class section should be assumed to put in a room.
///
/// [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
/// §3's two-state result.
///
/// # `Unknown` is not zero, and that is the whole point
///
/// The figure the interface already displays collapses the two:
/// `AttendingGroup.population` is `group.size.unwrap_or(named_members)`, so a section taught
/// only to groups that declare no size and list no members shows an audience of `0` — a
/// number that satisfies every room in the school including a cupboard. ADR 0034 §3 names
/// that collapse as the specific failure this type exists to prevent, which is why
/// [`ExpectedAttendance::Unknown`] is a variant and not a sentinel value.
///
/// # Nothing in the first solver reads it
///
/// `room-capacity` is **not** in the first constraint set (ADR 0034 §1) and the stored
/// column it would read — `class_section.expected_attendance` — does not exist yet
/// (`SCHEDULING-ARCHITECTURE-DECISIONS.md` §4.1, scheduled for M2+). This type is declared
/// now because the fixtures have to express the derivation's cases, and because a constraint
/// written later against a well-named two-state value cannot accidentally treat an absence
/// as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ExpectedAttendance {
    /// A stated number: either the section's own explicit figure, or a sum over groups that
    /// all declare a size and are **provably** pairwise [`Overlap::Disjoint`].
    Known {
        /// How many people the timetable should assume are in the room.
        people: i64,
    },
    /// The stored data does not say.
    ///
    /// The ordinary state, and the default. `room-capacity` is **not evaluated** for an
    /// activity whose section is `Unknown` — which is a third outcome, distinguishable in a
    /// score breakdown from "evaluated, zero violations" (ADR 0034 §4).
    #[default]
    Unknown,
}

impl ExpectedAttendance {
    /// The number, when there is one.
    ///
    /// Returns `None` for [`ExpectedAttendance::Unknown`] — never `Some(0)`.
    #[must_use]
    pub const fn people(self) -> Option<i64> {
        match self {
            Self::Known { people } => Some(people),
            Self::Unknown => None,
        }
    }

    /// Whether a capacity rule can be evaluated against this section at all.
    #[must_use]
    pub const fn is_known(self) -> bool {
        matches!(self, Self::Known { .. })
    }
}

/// A floor on a section's attendance, derived from member rows.
///
/// [ADR 0034](../../../../docs/adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
/// §3's last clause. Where a group declares no `size` but lists `k > 0` members, `k` is a
/// floor on that group's population, and the largest floor across a section's groups is a
/// sound lower bound on its attendance.
///
/// **It may raise a warning and it may never decide a hard constraint.** A bound that can
/// only be too small can say *"Lab 2 seats 24 and at least 26 students are listed"*; it
/// cannot say the room is too small, because the true figure is unknown and the
/// constraint would be deciding on a number nobody stated. The type is separate from
/// [`ExpectedAttendance`] so the two cannot be passed to one another by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendanceFloor {
    /// The largest number of students listed in any one of the section's groups.
    ///
    /// Zero means no member rows were found, which is not evidence of an empty section.
    pub at_least: i64,
}

impl AttendanceFloor {
    /// Whether this floor says anything at all.
    #[must_use]
    pub const fn is_informative(self) -> bool {
        self.at_least > 0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AttendanceFloor, DisjointnessDeclaration, ExpectedAttendance, GroupIx, GroupMask, Overlap,
        OverlapRelation,
    };

    fn g(ix: u32) -> GroupIx {
        GroupIx::new(ix)
    }

    #[test]
    fn only_a_proved_overlap_is_a_clash() {
        assert!(Overlap::Overlaps.is_clash());
        assert!(!Overlap::Disjoint.is_clash());
        assert!(
            !Overlap::Indeterminate.is_clash(),
            "ADR 0035 §6: indeterminate is not enforced"
        );
    }

    #[test]
    fn indeterminate_is_the_only_undecided_answer() {
        assert!(Overlap::Overlaps.is_decided());
        assert!(Overlap::Disjoint.is_decided());
        assert!(!Overlap::Indeterminate.is_decided());
    }

    #[test]
    fn a_declaration_round_trips_through_its_three_stored_states() {
        for (column, declaration) in [
            (None, DisjointnessDeclaration::NotStated),
            (Some(true), DisjointnessDeclaration::Disjoint),
            (Some(false), DisjointnessDeclaration::MayOverlap),
        ] {
            assert_eq!(DisjointnessDeclaration::from_column(column), declaration);
            assert_eq!(declaration.to_column(), column);
        }
    }

    #[test]
    fn not_stated_and_may_overlap_differ_in_what_preflight_does() {
        // They give the same overlap answer and are different states. The column is
        // nullable rather than NOT NULL DEFAULT 0 for exactly this reason — ADR 0035 §4.
        assert!(!DisjointnessDeclaration::NotStated.proves_children_disjoint());
        assert!(!DisjointnessDeclaration::MayOverlap.proves_children_disjoint());
        assert!(DisjointnessDeclaration::NotStated.is_unanswered());
        assert!(!DisjointnessDeclaration::MayOverlap.is_unanswered());
    }

    #[test]
    fn an_unstated_declaration_is_the_default_for_every_existing_project() {
        assert_eq!(
            DisjointnessDeclaration::default(),
            DisjointnessDeclaration::NotStated
        );
    }

    #[test]
    fn a_mask_holds_the_groups_put_into_it() {
        let mut mask = GroupMask::with_capacity(4);
        assert!(mask.is_empty());
        mask.insert(g(0));
        mask.insert(g(3));
        assert!(mask.contains(g(0)));
        assert!(!mask.contains(g(1)));
        assert!(mask.contains(g(3)));
        assert_eq!(mask.len(), 2);
        assert_eq!(mask.iter().collect::<Vec<_>>(), vec![g(0), g(3)]);
    }

    #[test]
    fn a_mask_grows_past_its_first_word() {
        // 64 groups per word; a school with more must not silently lose the later ones.
        let mut mask = GroupMask::with_capacity(1);
        mask.insert(g(200));
        assert!(mask.contains(g(200)));
        assert_eq!(mask.len(), 1);
        assert_eq!(mask.iter().collect::<Vec<_>>(), vec![g(200)]);
    }

    #[test]
    fn masks_of_different_widths_intersect_correctly() {
        let mut wide = GroupMask::with_capacity(200);
        wide.insert(g(3));
        let mut narrow = GroupMask::with_capacity(4);
        narrow.insert(g(3));
        assert!(wide.intersects(&narrow));
        assert!(narrow.intersects(&wide));

        let mut elsewhere = GroupMask::with_capacity(4);
        elsewhere.insert(g(2));
        assert!(!elsewhere.intersects(&wide));
    }

    #[test]
    fn an_unpopulated_relation_reports_ignorance_rather_than_safety() {
        // The failure mode this guards: a relation nobody filled in must not read as
        // "everything is disjoint", which is the silent-clash failure of ADR 0035 §6.
        let relation = OverlapRelation::unknown(3);
        assert_eq!(relation.get(g(0), g(1)), Overlap::Indeterminate);
        assert_eq!(relation.get(g(0), g(0)), Overlap::Indeterminate);
        assert_eq!(relation.indeterminate_pairs(), 3);
    }

    #[test]
    fn the_relation_is_symmetric_however_it_is_written() {
        let mut relation = OverlapRelation::unknown(3);
        relation.set(g(0), g(2), Overlap::Overlaps);
        assert_eq!(relation.get(g(0), g(2)), Overlap::Overlaps);
        assert_eq!(relation.get(g(2), g(0)), Overlap::Overlaps);
    }

    #[test]
    fn only_proved_overlap_reaches_the_compiled_bitsets() {
        // Year 9 overlaps 9A; 9A against 9B is merely unproven. The mask must carry the
        // first and not the second, which is what makes the compiled form enforce exactly
        // what is proved.
        let (year9, nine_a, nine_b) = (g(0), g(1), g(2));
        let mut relation = OverlapRelation::unknown(3);
        relation.set(year9, nine_a, Overlap::Overlaps);
        relation.set(nine_a, nine_b, Overlap::Indeterminate);

        let overlaps = relation.overlaps_with(nine_a);
        assert!(overlaps.contains(year9));
        assert!(!overlaps.contains(nine_b));
    }

    #[test]
    fn a_clash_mask_covers_the_audience_and_everything_proved_to_overlap_it() {
        // ADR 0035 §5, transcribed: a section taught to 9A must not run beside one taught
        // to Year 9, because 9A is part of Year 9.
        let (year9, nine_a, ten_a) = (g(0), g(1), g(2));
        let mut relation = OverlapRelation::unknown(3);
        relation.set(year9, nine_a, Overlap::Overlaps);

        let mut audience = GroupMask::with_capacity(3);
        audience.insert(nine_a);
        let clash = relation.clash_mask(&audience);

        let mut year9_audience = GroupMask::with_capacity(3);
        year9_audience.insert(year9);
        assert!(clash.intersects(&year9_audience), "9A is part of Year 9");

        let mut ten_a_audience = GroupMask::with_capacity(3);
        ten_a_audience.insert(ten_a);
        assert!(
            !clash.intersects(&ten_a_audience),
            "an unrelated year group is unproven, not proven to clash"
        );
    }

    #[test]
    fn unknown_attendance_is_never_zero() {
        // The exact collapse ADR 0034 §3 names in the existing display path.
        assert_eq!(ExpectedAttendance::Unknown.people(), None);
        assert!(!ExpectedAttendance::Unknown.is_known());
        assert_eq!(
            ExpectedAttendance::Known { people: 0 }.people(),
            Some(0),
            "a stated zero is a number, and is not the same value as Unknown"
        );
        assert_ne!(
            ExpectedAttendance::Known { people: 0 },
            ExpectedAttendance::Unknown
        );
    }

    #[test]
    fn unknown_is_the_default_attendance() {
        assert_eq!(ExpectedAttendance::default(), ExpectedAttendance::Unknown);
    }

    #[test]
    fn a_floor_with_no_member_rows_says_nothing() {
        assert!(!AttendanceFloor { at_least: 0 }.is_informative());
        assert!(AttendanceFloor { at_least: 26 }.is_informative());
    }

    #[test]
    fn attendance_serialises_with_its_kind_tag() {
        let json = serde_json::to_string(&ExpectedAttendance::Known { people: 24 })
            .expect("attendance serialises");
        assert_eq!(json, r#"{"kind":"known","people":24}"#);
        let json =
            serde_json::to_string(&ExpectedAttendance::Unknown).expect("attendance serialises");
        assert_eq!(json, r#"{"kind":"unknown"}"#);
    }
}
