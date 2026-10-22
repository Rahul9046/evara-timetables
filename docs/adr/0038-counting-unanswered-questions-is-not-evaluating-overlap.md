# ADR 0038 — Counting unanswered questions is not evaluating overlap, and the warning says only what it can prove

- Status: **Accepted**
- Date: 2026-10-21
- Decides: where the boundary sits between M3's report of group-overlap uncertainty and M5's
  overlap evaluator, and what
  [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md) §6's *"number of answers
  that would clear it"* may honestly claim. Raised as gap 8.3 of the M3 brief.
- Builds on: [ADR 0035](0035-student-group-overlap-is-a-three-state-answer.md) (the
  three-state answer, the seven ordered rules, and §6's requirement that the warning be
  actionable arithmetic), [ADR 0006](0006-group-based-scheduling-first.md) (a school with no
  student rows still gets a timetable).
- Binds: M5's `student-clash`, which reuses the hierarchy walk rather than restating it.

## Context

ADR 0035 §6 requires the indeterminate-overlap warning to be arithmetic rather than an
adjective, and gives the sentence:

> *"431 pairs of this school's attending groups cannot be compared … Answering 'do these
> overlap?' on 7 parent groups — Year 9, Year 10 … — resolves 412 of them."*

Two numbers, and they are not the same kind of number.

**431 is a count of the relation.** `OverlapRelation` already holds a three-state answer per
pair, so counting the undecided ones reads a value the problem carries. No rule is evaluated.

**412 is a prediction about rule 6.** It says: *if these declarations were answered, this many
pairs would become decided.* Computing it means deciding, for each undecided pair, whether a
`children_are_disjoint` answer on some common ancestor would settle it — which is rule 6 of
ADR 0035 §3, inside the function M5 owns. A second implementation of rule 6 in M3 is exactly
the duplication `CLAUDE.md` §2 forbids, and the one that would be most damaging: if M3's
prediction and M5's verdict ever disagreed, the product would be telling the user that
answering a question will help and then not honouring the answer.

There is a third fact that makes the prediction harder than it looks. A declaration is not the
only route to a decided pair. Rules 1, 3 and 5 decide pairs from provable emptiness, from a
shared member row and from declared sizes under a common ancestor — none of which a
declaration affects. So even a correct count of "pairs rule 6 would settle" would understate
what the school could do, and a count that silently included the other routes would be
claiming more than rule 6 delivers.

And some pairs no declaration can ever settle. Two classes in different year levels share no
ancestor at all — ADR 0035 truth table 17: *"unrelated is not disjoint"* — so no answer on any
parent reaches them. The `medium` fixture is exactly this case: every cohort has answered, and
360 pairs remain.

## Decision

**M3 counts what is unsettled and counts the questions that are unanswered. It does not predict
how many pairs an answer would settle, and its wording does not imply that it does.**

### 1. What M3 reports

Two figures, both read from stored facts:

| Figure | Read from | Rule evaluated |
| --- | --- | --- |
| Undecided pairs | `OverlapRelation::indeterminate_pairs` | none — the relation is already compiled |
| Unanswered declarations | the parent edge and `GroupFact::children_are_disjoint` | none — a property of a *group*, not of a pair |

`unanswered_declarations` returns the groups that have **two or more children** and have not
answered. Fewer than two children is excluded: a declaration about one child settles no pair,
so asking for it would send the user to a field that cannot help.

### 2. The wording says "would narrow", never "resolves"

The warning reads:

> *N pairs of {school}'s student groups cannot be compared: Evara does not know whether they
> share students. Until they can be, lessons for those groups may be timetabled at the same
> time. Answering "do these groups share students?" on K parent groups — {names} — would
> narrow this; some pairs may still need membership information as well.*

Three properties of that sentence are deliberate:

- **"would narrow", not "resolves N of them".** The number M3 cannot compute is not claimed.
- **"some pairs may still need membership information".** Rules 1, 3 and 5 exist, and a user
  told that declarations are the only lever would be misinformed.
- **The parents are named**, up to four with a count of the rest. ADR 0035 §6 forbids a list of
  pairs; it does not forbid naming the handful of fields to go and fill in, and a warning
  without them is one the user cannot act on.

Where every parent has answered and pairs remain, the second half changes rather than
disappearing:

> *Every parent group has already answered, so the pairs that remain are ones with no common
> parent to declare about. Only the memberships themselves would settle those.*

That is the `medium` case, and saying "answer a declaration" there would send the user to a
field that cannot help.

### 3. The hierarchy walk is M5's to reuse

`unanswered_declarations` is the one implementation of *"which parents have children and have
not answered"*. When M5 implements rule 6 it reads the same parent edge and the same column,
and it should reuse this walk rather than writing a second — not because the walk is hard, but
because two readings of a hierarchy are two places for a `MAX_GROUP_DEPTH` assumption to
differ.

### 4. What M3 does not do, restated because it is the point

- It does not decide whether two groups share a student.
- It does not decide whether two placements clash.
- It does not treat an undecided pair as disjoint. `clash_mask` already admits only
  `Overlaps`, so the compiled form enforces exactly what is proved, and M3 changes nothing
  about it.
- It does not produce `group-overlap-contradiction`. That needs rule 3 beaten against rule 6
  **and** member identities, and a `Problem` carries a member *count* rather than identities —
  so M3 could not detect it even if it owned the rule. It is M5's, recorded in
  [ADR 0037](0037-stored-row-diagnostics-belong-to-the-loader.md)'s ownership table.

## Consequences

**Good.**

- Rule 6 has one implementation, and it arrives with the evaluator that enforces it.
- The warning is actionable — it names the fields — without claiming an arithmetic it cannot
  perform.
- The honest second branch turns out to matter: `medium` has answered every declaration and
  still has 360 undecided pairs, and a message promising that declarations would clear them
  would have been simply false for a realistic school.
- `ADR 0006` is untouched: a school with no student rows still generates, and still gets this
  warning rather than a refusal.

**Bad, and accepted.**

- The user does not learn how much each answer buys. ADR 0035 §6's example promises that
  figure and this record does not deliver it; the number arrives when M5 can compute it
  correctly, and until then the warning says less rather than saying something untrue.
- "Would narrow this" is vaguer than "resolves 412 of them", and a vague warning is closer to
  the dismissible kind §8 risk B warns about. Naming the parent groups is what keeps it
  actionable; the names are the remedy even when the arithmetic is absent.

**Neutral.**

- `ValidationSummary` carries both figures separately, so a Generate screen can show them
  without re-deriving either, and M5 can add the predicted count as a third without changing
  the two that exist.

## Alternatives rejected

**Implement rule 6 in M3 to compute the exact figure.** The duplication this record exists to
prevent. Two implementations of rule 6 that can disagree is worse than one missing number, and
the disagreement would surface as the product promising an answer would help and then ignoring
it.

**Report only the pair count and no questions.** Satisfies the arithmetic requirement and
fails the actionability one: §6 is explicit that the count alone is an adjective with a number
attached, and a user given 431 and no next step has been told they have a problem and not what
to do.

**Claim the figure with a caveat — "up to 412".** An upper bound computed by the wrong
component is still the wrong component computing it, and "up to" is the kind of hedge a reader
drops.

**Count every parent with any children, including one.** Rejected: a declaration about a single
child settles no pair, so it inflates the count with questions that cannot help and makes the
remedy longer than it needs to be.

**Wait for M5 and emit no warning in M3.** Rejected on what §3.2 classifies this as: an
explicit warning, counted and announced. A milestone that implements the classification and
skips the row it was most explicitly asked for has not implemented it.

## Future acceptance tests

Implemented by M3 except 6 and 7.

| # | Case | Expectation |
| --- | --- | --- |
| 1 | `unknown-group-overlap` | One aggregate finding carrying the pair count, naming the parent to answer, saying "would narrow", and never "resolves" |
| 2 | A school with nothing undecided | No finding at all |
| 3 | `medium` — every declaration answered, pairs remain | The second branch: no parent left to answer, and membership named as the only lever |
| 4 | A parent with exactly one child, unanswered | Not counted among the questions |
| 5 | Hundreds of undecided pairs | Still one finding. Never a list of pairs |
| 6 | M5's rule 6, over the same fixtures | Agrees with the relation each fixture declares; reuses the hierarchy walk rather than restating it |
| 7 | M5 adds the predicted count | The existing two figures are unchanged, and the warning gains the third rather than rewording the first two |
