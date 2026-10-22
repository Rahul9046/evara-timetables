# ADR 0037 — A finding about a stored row belongs to the loader, not to the validator

- Status: **Accepted**
- Date: 2026-10-21
- Decides: which milestone produces `orphan-availability-reference` and
  `availability-entity-kind-not-scheduled`, and where a violated storage invariant is
  reported. Raised as gap 8.2 of the M3 brief.
- Builds on: [ADR 0028](0028-assigning-a-lesson-warns-about-qualifications.md) and
  [ADR 0025](0025-teacher-qualifications-across-school-owned-subjects.md) (what the database
  already refuses, and why), `V11`'s header (the one polymorphic reference the schema cannot
  enforce).
- Binds: M4's `Problem` loader. Nothing is implemented by this record; M3 implements the
  split and produces neither finding.

## Context

M2 declared 22 diagnostic codes. Two of them are about **stored rows**:

- `orphan-availability-reference` — an `availability` row naming an entity that no longer
  exists. `availability.entity_id` is polymorphic and so carries no foreign key, which `V11`'s
  header calls *"the one place in this schema where a reference cannot be enforced by the
  database"*.
- `availability-entity-kind-not-scheduled` — a row naming a `RESOURCE` or a `SUBJECT`, neither
  of which anything schedules.

M3 validates a compiled `Problem`. A `Problem` holds `UnavailabilityIndex`, which is rules
already **compiled onto the slots they name**. A rule naming a deleted teacher compiles onto
nothing; a rule naming a resource has no index to compile into. By the time a problem exists,
both rows are gone.

So a pure validator over a `Problem` can never emit either code — not because it was not
written, but because the information is not there to emit it from. Two of the 22 codes were
declared with no reachable trigger.

A third code has the same shape for a different reason. `teacher-not-eligible-for-campus` is
documented as unreachable: `V15:507`'s trigger pairs refuse the rows, so a consistent database
cannot contain one. M2 declared it so that a loader finding one anyway reports a damaged
project rather than building a timetable on it.

## Decision

**A finding about a row belongs to whatever reads rows. All three are M4's, and M3 produces
none of them.**

### 1. Two kinds of diagnostic, one vocabulary

| Kind | Produced from | By | Examples |
| --- | --- | --- | --- |
| **Problem diagnostics** | the compiled in-memory `Problem` | **M3** | `no-bell-schedule`, `expected-attendance-unknown`, `group-overlap-indeterminate` |
| **Assembly diagnostics** | stored project data, while reading and translating it | **M4** | `orphan-availability-reference`, `availability-entity-kind-not-scheduled`, `teacher-not-eligible-for-campus` |

Both use the **same** `Diagnostic`, `DiagnosticCode`, `Severity` and `EntityRef`. One
vocabulary, two producers: a user reading a pre-flight report should not be able to tell which
stage found what, and a caller assembling a report should not have to merge two shapes.

### 2. The `Problem` is not widened to carry rejected rows

The tempting fix is to give `Problem` a list of rules the loader could not compile, so M3 can
report them. Rejected: `Problem` is *"everything a solve needs, and nothing else"*, and a list
of things the solve must ignore is the opposite of that. It would also be the first field whose
only consumer is a diagnostic, which invites more of them.

### 3. M4's obligations, stated now so they are not discovered later

When the loader meets a row it cannot compile it must:

- produce a finding under the declared code, with the row's own identifier in the `EntityRef`;
- name the entity kind it found, which is why `AvailabilityEntityKind` carries all six of the
  column's values rather than the three the index compiles;
- **not silently discard the row.** A rule that vanishes without explanation is indistinguishable
  from one that was never entered, and the two are different states — which is the whole reason
  `V11` stores only exceptions.

### 4. A violated storage invariant is corruption, not a check

`teacher-not-eligible-for-campus` is reported **only** as a damaged project. M4 does not check
eligibility as a matter of course: the trigger pairs are the enforcement, and a solver-side
duplicate would be a second implementation of a rule the database owns — which is what
ADR 0028 settled when it decided employment and campus eligibility are database invariants and
a qualification merely warns.

The spirit is `DbError::CorruptIdentifier`'s: a persisted value that cannot be true is not
input to be interpreted, and substituting a default for it is worse than failing.

### 5. The codes are not renamed and not deleted

All three keep their keys. A code with a documented owner and a later trigger is a contract;
deleting it to make a coverage count tidy would lose the decision. M3's test asserts that every
declared code is either implemented by M3 or deferred **with a named owner**, and that no
deferred code is ever produced — so neither can drift into being quietly abandoned or quietly
fabricated.

## Consequences

**Good.**

- Each code has exactly one producer, so no finding can be emitted twice by two stages with
  different wording.
- `Problem` stays the compiled form of what a solve needs.
- M4 inherits a written obligation rather than a gap, including the reason
  `AvailabilityEntityKind` is wider than the compiled index.
- The unreachable code stays unreachable by design, and is documented as corruption rather than
  masquerading as a routine check.

**Bad, and accepted.**

- **Three of the 22 codes produce nothing until M4.** That is a real gap in coverage, and this
  record is the reason rather than an excuse: the information they report is not in a `Problem`.
- A project with an orphaned availability rule gets no warning from `evara validate`, because
  that command only has fixtures to validate and a fixture has no stored rows. The warning
  arrives with the loader.

**Neutral.**

- `AvailabilityEntity` compiles four kinds and `AvailabilityEntityKind` names six. The asymmetry
  is deliberate and now has a record explaining it.

## Alternatives rejected

**Give `Problem` a `rejected_rules: Vec<AvailabilityRule>` field.** §2. It would make the
problem carry what the solve must ignore, and the field's only reader would be a diagnostic.

**Have M3 take a second argument — the raw rules — alongside the problem.** This makes
`validate` impure in spirit: its signature would advertise that it needs something no `Problem`
contains, and every caller would have to supply it or pass an empty list, which is a quiet way
to skip the check.

**Delete the three codes and re-add them in M4.** Loses the decision and the wording, and
`DiagnosticCode`'s keys are a contract — *"added to, never renamed"*. A code with a documented
owner is better than a code that disappears and comes back.

**Implement `teacher-not-eligible-for-campus` as a routine solver-side check.** Rejected on
ADR 0028: eligibility is a database invariant under two trigger pairs, and a second
implementation in the solver is the architecture bug `CLAUDE.md` §2 names — with the added
cost that it would make a damaged project look like an ordinary one.

## Future acceptance tests

M4's, except the first two which M3 implements.

| # | Case | Expectation |
| --- | --- | --- |
| 1 | Every declared code | Implemented by M3 **or** deferred with a named owner; never both, never neither |
| 2 | Every catalogue fixture, at every seed | No deferred code is produced |
| 3 | A project with an `availability` row naming a deleted teacher | `orphan-availability-reference`, carrying the row's own identifier, and the row is not silently dropped |
| 4 | A row naming a `RESOURCE` | `availability-entity-kind-not-scheduled`, naming the kind |
| 5 | A row naming a `SUBJECT` | As 4 |
| 6 | A project whose triggers have been bypassed so an ineligible assignment exists | `teacher-not-eligible-for-campus`, reported as a damaged project, and the solve refused |
| 7 | An ordinary project | None of the three fires, and no eligibility check is performed as a matter of course |
