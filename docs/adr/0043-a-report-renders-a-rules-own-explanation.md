# ADR 0043 — A report renders a rule's own explanation; `evara-report` may depend on `evara-constraints`

- Status: **Accepted** (2026-10-23)
- Date: 2026-10-23
- Decides: M8 architecture proposal D9
- Changes: `CLAUDE.md` §2's dependency table — `evara-report` goes from `-> core` to
  `-> core, constraints`
- Builds on: [ADR 0005](0005-constraints-as-data.md) (constraints are data with a registry),
  [ADR 0039](0039-a-mandatory-constraint-lives-in-the-registry.md) (a rule is mandatory in the
  registry), [ADR 0042](0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md)
  C3 (a stored violation hangs off an evaluation)
- Binds: `evara-report`, and the command layer that feeds it

## Context

M8 renders findings in two places: beside a generated preview, and beside a saved timetable.
A finding has to read as a sentence — *"A. Okafor already teaches 9 Mathematics B in Day A
period 1"* — and that sentence has exactly one author.

`CONSTRAINT-ENGINE.md` §6: *"A `Violation` is structured — the rule, the level, the
occurrences, the subject entity, the related entities, the cycle day, the wall-clock span, the
integer penalty — and never a pre-rendered string. `explain()` turns one into a sentence, and
that wording lives with the rule so it cannot drift from the check that produced it."*

`explain()` is a member of `ConstraintEvaluator`, reached through
`evara_constraints::evaluator_for`. `evara-report` could not see it:
`CLAUDE.md` §2 pinned the crate at `evara-report -> core`.

So the sentence had to come from somewhere, and there were only three places it could.

## Decision

**`evara-report` depends on `evara-constraints`, and calls `explain()`.** It composes no
rule's wording of its own.

`CLAUDE.md` §2's table becomes:

```
crates/evara-report     projections and renderers   -> core, constraints
```

`evara_report::findings::explain_all(problem, violations)` is the entry point. It maps each
violation through `evaluator_for(violation.constraint_type).explain(problem, violation)` and
formats nothing itself.

### What the edge does and does not admit

`evara-constraints` depends on `evara-core` alone, holds no SQL, opens no file and reaches no
network. Its public surface is the registry, the evaluators, the evaluation API and the
parameter schemas. So the edge adds nothing `evara-report` should not have — it does not, for
instance, give a report a route to a database, which is the property the original `-> core`
constraint was protecting.

What it does admit is `evara-report` *running* a rule. That is accepted and bounded: the
crate calls `explain` and nothing else from the evaluator trait. It does not call `evaluate`,
does not hold a registry, and does not decide what is legal — the command layer scores, and
the report renders what the scoring found.

## The alternatives, and why each is worse

**The command layer explains, and hands `Vec<Explanation>` to `evara-report` as data.** Keeps
the dependency table unchanged, and was the proposal's named fallback. Rejected because it
moves a join into `src-tauri`, which `ARCHITECTURE.md` §3 rule 7 reserves for orchestration:
the command layer would be assembling a report's content rather than wiring one crate to
another, and the same join would then be written again by `evara-cli` and by Phase 10's
renderers. One dependency edge is cheaper than three call sites of the same loop.

**`evara-report` writes the sentences.** Rejected outright. It is a second implementation of
every rule's reasoning, in a crate that cannot see the check it is describing, and it goes
stale the first time a rule's logic changes and its prose does not. This is `CLAUDE.md` §2's
most important rule — *"If generation and the editor can ever disagree about whether a
placement is legal, the product is broken"* — applied to what a user is *told*, which is the
only part of a rule they ever see.

**Store the sentence with the violation.** Already rejected, by
`TIMETABLE-PERSISTENCE.md` §7 for the unplaced cause and for the same reason here: *"a wording
improvement would leave old rows reading the old way"*. It is also why the stored tier of
`FindingView` carries no `message` at all.

## Consequences

**Good.**

- One author per sentence, and it is the rule. A wording change reaches every screen and
  every renderer at once.
- `FindingView` can carry both fidelities honestly: `message: Some(_)` when a problem was
  compiled, `None` when only the stored row was available. A caller cannot mistake one for
  the other.
- Phase 10's HTML, CSV and PDF renderers inherit the explanation rather than each needing
  their own.

**Bad, and accepted.**

- `evara-report` is no longer the leaf it was. A change to `ConstraintEvaluator`'s signature
  now rebuilds it.
- The crate can, in principle, call more of `evara-constraints` than it should. Nothing
  enforces the narrowness above except review and the module's own documentation, which says
  it.

**Neutral.**

- `evara-cli` and the desktop crate already depend on both, so no new crate enters any
  binary's dependency tree.

## What this does not change

- **`evara-report` still opens nothing.** No SQLite, no file, no network. Its header says so
  and the `Cargo.toml` makes it true.
- **A report still judges nothing.** It does not evaluate, does not score and does not decide
  whether a placement is legal. `explain` describes a violation somebody else found.
- **`evara-core` is untouched.** It gains no dependency, and `Violation` stays structured with
  no pre-rendered string.
