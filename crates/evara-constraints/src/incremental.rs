//! The incremental scorer: a running score a search can afford to ask about.
//!
//! # What `SOLVER.md` §4 asked for, and what makes it safe
//!
//! > Full re-evaluation of every constraint after every candidate move is too slow for local
//! > search. But hand-written delta scoring is the single most bug-prone part of any solver,
//! > and a wrong delta silently produces a wrong timetable.
//! >
//! > The mitigation is structural: **two scorers and differential testing.**
//!
//! M5 shipped the reference half — [`evaluate`](crate::evaluate), *"always correct, never
//! fast"*. This is the other half, and `OPEN-DECISIONS.md` R1 is explicit that it and the
//! property test between them are one deliverable: *"neither half may land without the
//! other."*
//!
//! # There is no hand-written delta in this file
//!
//! That is the whole design, and it is why the warning above does not apply to it. A delta is
//! usually written as *per-rule arithmetic* — "a teacher clash adds one, unless the pair was
//! already counted" — and that arithmetic is what goes wrong. Here the delta is derived from a
//! property of the rules instead:
//!
//! **Every violation any implemented rule produces names at most two occurrences.** So a
//! violation that does not name `a` cannot change when `a` moves, and
//!
//! ```text
//! score(after) − score(before) = involving(after, a) − involving(before, a)
//! ```
//!
//! is an identity rather than an approximation. [`ConstraintEvaluator::evaluate_involving`] is
//! the narrowed query, implemented by each rule **through the same body** its full sweep uses,
//! so there is no second statement of any predicate anywhere — `CLAUDE.md` §2's most important
//! rule, applied to the one place a solver usually breaks it.
//!
//! What remains to be checked is therefore much smaller than a delta: that each rule's narrowed
//! query really returns everything naming the occurrence. [`crate::tests`] asserts exactly that
//! over every fixture — the union of `evaluate_involving` over all occurrences, deduplicated,
//! equals `evaluate` — and then asserts the running score against a full re-evaluation after
//! every operation of a seeded random sequence.
//!
//! # What it maintains, and what it recomputes
//!
//! Maintained: the [`Occupancy`] (one placement at a time, not rebuilt), the running
//! [`Score`], and a count per [`ConstraintType`].
//!
//! Recomputed on demand: the violation **list**. [`IncrementalScorer::report`] runs the
//! reference scorer. That is deliberate rather than lazy — a solver asks for the score
//! thousands of times and for the list once, at the end, and a maintained list would be bug
//! surface bought for nothing. §6 of `CONSTRAINT-ENGINE.md` says a caller needs more than one
//! question answered; this type answers the cheap one cheaply and defers the expensive one to
//! the implementation that was always going to be the reference.
//!
//! # Tentative, accept, revert
//!
//! [`IncrementalScorer::probe`] answers *"what would this break?"* **without mutating
//! anything** a caller can observe — it places, asks, and puts the previous state back — so a
//! construction solver can try a dozen candidates and commit one. [`IncrementalScorer::place`]
//! and [`IncrementalScorer::withdraw`] are the committing pair.

use evara_core::scheduling::{
    ActivityIx, Assignment, CompiledConstraint, ConstraintLevel, ConstraintType, Placement,
    Problem, Score, Violation, ViolationSink,
};

use crate::evaluation::{EvaluationReport, evaluate};
use crate::occupancy::Occupancy;
use crate::registry::evaluator_for;

/// What placing one occurrence somewhere would do.
///
/// Returned by [`IncrementalScorer::probe`], which commits nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementVerdict {
    /// Whether the occurrence would break no hard rule where it was put.
    ///
    /// The question a construction solver asks, and the only one whose answer it must obey.
    /// `true` does **not** mean the placement is good, only that it breaks no hard rule — the
    /// same distinction [`EvaluationReport::is_usable`] draws, for the same reason.
    ///
    /// # Stricter than `SOLVER.md` §6's wording, deliberately
    ///
    /// That sketch defines `MoveVerdict::allowed` as *"false iff it **adds** a hard
    /// violation"*. This is `false` whenever the result *carries* one naming the occurrence,
    /// which is the same answer for every placement of an **unplaced** occurrence — the only
    /// kind M6 makes, and the reason the two readings have not had to be distinguished before
    /// now. They differ for a move of an already-placed lesson that was illegal before and is
    /// still illegal after: "adds" would call that allowed. A construction solver must never
    /// accept a placement that leaves the occurrence clashing, so the stricter reading is the
    /// one implemented, and M11's editor inherits it: a drop target that was already broken is
    /// not a legal drop target.
    pub allowed: bool,
    /// How the score would change, at all three levels.
    ///
    /// Negative-going, so a smaller delta is worse. Exact: see this module's header for why
    /// the identity it is computed from is not an approximation.
    pub delta: Score,
    /// The hard violations the occurrence would be in, in rule order.
    ///
    /// Empty exactly when [`PlacementVerdict::allowed`]. Carried rather than counted so a
    /// refusal can be explained — `SOLVER.md` §6's `MoveVerdict::blocking` is the same list,
    /// and M11's `validate_move` is the other caller this type exists for.
    pub blocking: Vec<Violation>,
    /// The medium and soft violations the placement would newly introduce.
    ///
    /// Never a reason to refuse anything. Note what is *not* here: placing an occurrence
    /// resolves its `activity-scheduled` violation rather than adding one, so a successful
    /// placement's `delta` improves at the medium level and this list stays empty.
    pub warnings: Vec<Violation>,
}

/// A candidate timetable whose score is kept up to date as it changes.
///
/// # Equality
///
/// Two scorers are equal when they hold the same occupancy and the same running figures. The
/// property that matters is stronger and is asserted rather than derived: a scorer reached by a
/// sequence of placements has the **same score** as a fresh [`evaluate`] of the assignment it
/// ended up with. See this module's header.
#[derive(Debug, Clone)]
pub struct IncrementalScorer<'p> {
    occupancy: Occupancy<'p>,
    constraints: Vec<CompiledConstraint>,
    /// The running total. Maintained by the identity in this module's header.
    score: Score,
    /// How many violations of each type the current arrangement carries, in
    /// [`ConstraintType::all`] order.
    counts: Vec<u32>,
}

impl<'p> IncrementalScorer<'p> {
    /// A scorer over an arrangement in which nothing is placed.
    ///
    /// The starting point of a construction solve. Not a cheap shortcut: the empty arrangement
    /// already carries one `activity-scheduled` violation per occurrence, and those are scored
    /// here exactly as [`evaluate`] would score them — which is what makes the first committed
    /// placement's delta meaningful.
    #[must_use]
    pub fn empty(problem: &'p Problem, constraints: &[CompiledConstraint]) -> Self {
        let mut scorer = Self {
            occupancy: Occupancy::empty(problem),
            constraints: constraints.to_vec(),
            score: Score::PERFECT,
            counts: vec![0; ConstraintType::all().len()],
        };
        scorer.recount();
        scorer
    }

    /// A scorer over an existing arrangement.
    ///
    /// For a repair run, which `SOLVER.md` §7 describes as a solve with `start = Some(current)`.
    /// Returns `None` when the assignment cannot be read against the problem, because a scorer
    /// over an unreadable arrangement would be maintaining a number about nothing — the same
    /// refusal [`EvaluationReport::is_readable`] makes.
    #[must_use]
    pub fn of(
        problem: &'p Problem,
        assignment: &Assignment,
        constraints: &[CompiledConstraint],
    ) -> Option<Self> {
        let occupancy = Occupancy::of(problem, assignment);
        if !occupancy.invalid_references().is_empty() {
            return None;
        }
        let mut scorer = Self {
            occupancy,
            constraints: constraints.to_vec(),
            score: Score::PERFECT,
            counts: vec![0; ConstraintType::all().len()],
        };
        scorer.recount();
        Some(scorer)
    }

    /// The running score.
    ///
    /// Maintained, not computed: this is the call a search makes thousands of times.
    #[must_use]
    pub const fn score(&self) -> Score {
        self.score
    }

    /// Whether the current arrangement breaks no hard rule.
    ///
    /// Deliberately not `is_feasible` or `is_valid`, for `CONSTRAINT-ENGINE.md` §6's reason: a
    /// usable *partial* timetable is still partial.
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        self.score.is_feasible()
    }

    /// How many violations of one type the current arrangement carries.
    #[must_use]
    pub fn count_of(&self, constraint_type: ConstraintType) -> u32 {
        position_of(constraint_type).map_or(0, |at| self.counts[at])
    }

    /// The occupancy, for a caller that needs to read what is placed.
    #[must_use]
    pub const fn occupancy(&self) -> &Occupancy<'p> {
        &self.occupancy
    }

    /// The arrangement reached so far.
    #[must_use]
    pub fn assignment(&self) -> Assignment {
        self.occupancy.assignment()
    }

    /// How many occurrences are placed.
    #[must_use]
    pub fn placed_count(&self) -> usize {
        self.occupancy.placed_count()
    }

    /// The full report, from the reference scorer.
    ///
    /// Runs [`evaluate`] over the current arrangement. The expensive question, asked once —
    /// see this module's header for why the violation list is not maintained.
    ///
    /// A caller comparing [`EvaluationReport::score`] with [`IncrementalScorer::score`] is
    /// performing the differential check this type is gated on, which is why the two are
    /// reachable side by side rather than one being hidden.
    #[must_use]
    pub fn report(&self) -> EvaluationReport {
        evaluate(
            self.occupancy.problem(),
            &self.assignment(),
            &self.constraints,
        )
    }

    /// What placing `activity` at `placement` would do, committing nothing.
    ///
    /// # Implemented by doing it and undoing it
    ///
    /// Rather than predicting the effect, the scorer applies the placement, asks each rule
    /// what it now finds about that occurrence, and puts the previous state back. That is both
    /// cheaper and far safer than a predictive delta: the question is answered by the rules
    /// themselves against a real arrangement, so a probe cannot disagree with the commit that
    /// follows it. The cost is bounded by the occurrence's own neighbourhood, not by the
    /// timetable.
    ///
    /// The scorer is observably unchanged afterwards — same occupancy, same score, same counts
    /// — which a test asserts over random sequences, because a probe that leaked state would
    /// corrupt every placement after it.
    pub fn probe(&mut self, activity: ActivityIx, placement: Placement) -> PlacementVerdict {
        // Measured against the arrangement as it stands, which for a placed occurrence means
        // "what would moving it there cost?" and not "what would putting it there from nowhere
        // cost?". The two differ by whatever the occurrence is already in.
        let before = self.involving(activity);
        let restore = self.occupancy.withdraw(activity);

        if !self.occupancy.place(activity, placement) {
            // The placement cannot be read against this problem. Put the previous state back
            // and refuse without a violation: an unreadable candidate is a caller bug, not a
            // rule being broken, and inventing a violation for it would misreport it.
            self.restore(activity, restore);
            return PlacementVerdict {
                allowed: false,
                delta: Score::PERFECT,
                blocking: Vec::new(),
                warnings: Vec::new(),
            };
        }
        let after = self.involving(activity);

        self.occupancy.withdraw(activity);
        self.restore(activity, restore);

        let blocking: Vec<Violation> = after
            .iter()
            .filter(|found| found.level == ConstraintLevel::Hard)
            .cloned()
            .collect();
        PlacementVerdict {
            allowed: blocking.is_empty(),
            delta: subtract(total(&after), total(&before)),
            blocking,
            warnings: introduced(&before, &after),
        }
    }

    /// Places `activity`, updating the running score.
    ///
    /// Returns the exact change to the score, which is the same figure
    /// [`IncrementalScorer::probe`] would have returned for the same placement — asserted by a
    /// test, because a probe that disagreed with its commit would make every refusal a guess.
    ///
    /// Returns `None`, changing nothing, when the placement cannot be read against the problem.
    pub fn place(&mut self, activity: ActivityIx, placement: Placement) -> Option<Score> {
        let before = self.involving(activity);
        if !self.occupancy.place(activity, placement) {
            return None;
        }
        let after = self.involving(activity);
        Some(self.apply(&before, &after))
    }

    /// Unschedules `activity`, updating the running score.
    ///
    /// Returns the change to the score, and where the occurrence had been.
    pub fn withdraw(&mut self, activity: ActivityIx) -> Option<(Score, Placement)> {
        let before = self.involving(activity);
        let was = self.occupancy.withdraw(activity)?;
        let after = self.involving(activity);
        Some((self.apply(&before, &after), was))
    }

    /// Applies the identity in this module's header to the running figures.
    fn apply(&mut self, before: &[Violation], after: &[Violation]) -> Score {
        let delta = subtract(total(after), total(before));
        self.score = self.score.plus(delta);
        for found in before {
            if let Some(at) = position_of(found.constraint_type) {
                self.counts[at] = self.counts[at].saturating_sub(1);
            }
        }
        for found in after {
            if let Some(at) = position_of(found.constraint_type) {
                self.counts[at] = self.counts[at].saturating_add(1);
            }
        }
        delta
    }

    /// Puts a withdrawn placement back, if there was one.
    fn restore(&mut self, activity: ActivityIx, was: Option<Placement>) {
        if let Some(placement) = was {
            self.occupancy.place(activity, placement);
        }
    }

    /// Every violation, of every configured rule, that names `activity`.
    ///
    /// Runs the rules in [`ConstraintType::all`] order so the list is deterministic, and
    /// deduplicates for the reason [`evaluate`] does: one evaluator can meet the same pair
    /// twice while narrowing.
    fn involving(&self, activity: ActivityIx) -> Vec<Violation> {
        let mut sink = ViolationSink::new();
        for constraint_type in ConstraintType::all() {
            for constraint in self
                .constraints
                .iter()
                .filter(|c| c.constraint_type == *constraint_type)
            {
                evaluator_for(*constraint_type).evaluate_involving(
                    self.occupancy.problem(),
                    &self.occupancy,
                    constraint,
                    activity,
                    &mut sink,
                );
            }
        }
        sink.deduplicate();
        sink.into_violations()
    }

    /// Recomputes the score and the counts from scratch.
    ///
    /// Used once, at construction. A pair violation names two occurrences and would be counted
    /// twice by a sum over `involving`, so this reads the reference scorer instead — which is
    /// also the strongest possible starting point, since the running figures are then
    /// **defined** to begin equal to the reference.
    fn recount(&mut self) {
        let report = self.report();
        self.score = report.score;
        self.counts = ConstraintType::all()
            .iter()
            .map(|kind| u32::try_from(report.count_of(*kind)).unwrap_or(u32::MAX))
            .collect();
    }
}

/// The index of one type in [`ConstraintType::all`]'s order.
fn position_of(constraint_type: ConstraintType) -> Option<usize> {
    ConstraintType::all()
        .iter()
        .position(|kind| *kind == constraint_type)
}

/// The score of a list of violations.
fn total(violations: &[Violation]) -> Score {
    violations
        .iter()
        .fold(Score::PERFECT, |sum, found| sum.plus(found.score()))
}

/// `left − right`, level by level.
const fn subtract(left: Score, right: Score) -> Score {
    Score::new(
        left.hard - right.hard,
        left.medium - right.medium,
        left.soft - right.soft,
    )
}

/// The non-hard violations in `after` that were not already in `before`.
///
/// Compared on [`Violation::key`] rather than on the whole violation, because a clash's
/// reported span is taken from the lower-indexed occurrence and moving the *other* one can
/// change the wording of a violation that is the same violation. The key is the rule, the
/// occurrences and the subject, which is exactly what "the same problem" means here.
fn introduced(before: &[Violation], after: &[Violation]) -> Vec<Violation> {
    let known: Vec<_> = before.iter().map(Violation::key).collect();
    after
        .iter()
        .filter(|found| found.level != ConstraintLevel::Hard)
        .filter(|found| !known.contains(&found.key()))
        .cloned()
        .collect()
}
