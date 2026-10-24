# ADR 0036 — One bell-schedule resolver, shared by validation and loading, and a declared resolution is checked against it

- Status: **Accepted**
- Date: 2026-10-21
- Decides: where [ADR 0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
  §2–§3's resolution function lives, and what happens when a compiled problem's recorded
  answer disagrees with it. Raised as gap 8.1 of the M3 brief.
- Builds on: [ADR 0033](0033-an-activity-schedules-against-one-resolved-bell-schedule.md)
  (the compatibility relation and the three failures),
  [ADR 0011](0011-stable-timeslot-identity.md) (a timeslot's identity carries no campus),
  [ADR 0014](0014-rooms-shared-across-campuses.md) (the single-implementation discipline this
  follows).
- Binds: M4's `Problem` loader, M5's `duration-contiguous`, and the move validator —
  delivered in **M9A** as `IncrementalScorer::probe`'s callers rather than as M11's
  `validate_move`; see [TIMETABLE-EDITING.md](../TIMETABLE-EDITING.md) §2.
  Implemented by M3 as `evara_core::scheduling::resolve`.

## Context

M2 declared `SectionFact.bell_schedule` as a `BellScheduleResolution` and documented
resolution as M4's work. The consequence was not noticed until M3 came to use it.

**A resolution arrived already computed, and nothing checked it.** In a fixture it is written
by the fixture author; in a real project it would be written by the loader. Either way the
`Problem` carries an *answer*, and no code compared that answer with the relation ADR 0033 §2
defines. A fixture could declare any resolution it liked — including one the accepted rule
would never produce — and every test would pass.

That is a hole in exactly the place the product cannot afford one. ADR 0033's whole decision
is that there is no fallback and no guess; a resolution nobody checks is a guess with better
manners.

**Every input the relation needs is already in a `Problem`.** The materialised slots give `G`;
`context.bell_schedules` gives each schedule's campus; `context.campuses` gives the count that
clause 3 turns on; `section.campus` gives the section's side. So the function is expressible as
a pure function over the problem, with no database and no new field.

**And M2's own documentation of `G` was wrong**, which is the sharper half of the problem.
`SchedulingContext::bell_schedules` was documented as *"the bell schedules with slots
materialised into this grid — `G` of ADR 0033 §1"*. It is not: it holds every schedule the
school has **defined**, and three fixtures deliberately define one they never materialise. A
resolver reading that field would have reported an ambiguity at `unmaterialised-bell-schedule`,
which ADR 0033 test 8 exists to forbid. The field is needed — a schedule's campus and name are
read from it — but `G` has to be derived from the slots.

## Decision

**One pure resolver, in `evara-core`, derived from the materialised grid. M3 validates that a
recorded resolution agrees with it. A disagreement is a compiled-problem defect, not a user
finding.**

### 1. The resolver is `evara_core::scheduling::resolve`

`MaterialisedGrid::of(problem)` derives `G` from `Problem::timeslots` — never from
`context.bell_schedules`. `MaterialisedGrid::resolve(context, section_campus)` returns the
`BellScheduleResolution`, and `is_compatible` is ADR 0033 §2's three clauses and nothing else.

It is pure: no I/O, no clock, no mutation. That is why it can sit in `evara-core` beside the
vocabulary, and it is why all four callers can reach it.

### 2. Every caller uses it; none reimplements it

| Caller | Use |
| --- | --- |
| **M3** validation | Computes the resolution and drives `no-bell-schedule` / `ambiguous-bell-schedule` from the **computed** value, never from the recorded one |
| **M4** loader | Calls it to *produce* the value it records on each `SectionFact` |
| **M5** `duration-contiguous` | Reads `runs_on` for the runs a multi-period occurrence may occupy |
| **M9A** manual editing | `edit_candidates` and `edit_validate` resolve the same way the generator did, so the editor and the solver cannot disagree. Delivered earlier than this record expected, and through `IncrementalScorer::probe` rather than a `validate_move` of its own |

This is `CLAUDE.md` §2's rule — one implementation, several callers — applied to a resolution
rather than to a constraint, and it is the same discipline
[`RoomFact::is_candidate_for`](../../crates/evara-core/src/scheduling/problem.rs) applies to the
room candidate domain.

### 3. Validation never trusts the recorded resolution

M3's findings are computed from the relation. A problem that recorded `Resolved` for a section
the relation says is ambiguous still gets an `ambiguous-bell-schedule` finding, because a wrong
declaration must not be able to suppress a true finding.

### 4. A disagreement is `IntegrityError`, not a diagnostic

`Problem::integrity` gains `BellScheduleResolutionDisagrees { at, declared, computed }`.

The distinction is the point, and it is the one the brief asked to be preserved:

| Situation | Channel | Why |
| --- | --- | --- |
| A section resolves to none or to several | `DiagnosticCode::NoBellSchedule` / `AmbiguousBellSchedule`, **blocking** | An ordinary state of a real school. The remedy is a field: name the campus, or make a schedule school-wide |
| A problem *claims* an answer the relation does not give | `IntegrityError`, reported by `Problem::integrity` | No school can type this. Only a loader or a fixture can write it, and asking a user to act on it would be asking them to fix somebody else's bug |

`IntegrityError` already exists for exactly this class — its own documentation says these are
*"loader and fixture bugs, not data states"* — so the disagreement goes where its siblings
already live rather than into a report a user reads.

The check runs only when no dangling index was found first: a dangling campus or schedule index
makes the relation's answer meaningless, and reporting a disagreement on top of it would bury
the cause under a symptom.

### 5. `SchedulingContext::bell_schedules` is re-documented

Its doc comment now says what it is — every **defined** schedule — and names `MaterialisedGrid`
as the way to get `G`. The M2 wording is corrected rather than quietly replaced, because a
reader who learned the wrong thing from it deserves to see that it was wrong.

## Consequences

**Good.**

- A fixture can no longer declare a resolution the accepted rule would not produce. A test over
  all 24 fixtures at three seeds asserts declared equals computed, and it passed on the first
  run — so M2's fixtures were right, which was worth knowing rather than assuming.
- M4 cannot drift from M3: there is one function, and the loader's job becomes calling it.
- The false ambiguity at `unmaterialised-bell-schedule` is structurally impossible, because `G`
  is derived from slots and a schedule with no slots cannot be a candidate.
- `runs_on` lands in the same place, so M5's `duration-contiguous` inherits the one definition
  of "ordinal-consecutive teaching periods of one schedule" rather than writing a second.

**Bad, and accepted.**

- M3 implements something M2's documentation attributed to M4. That is a change of plan, and
  this record is it; `SOLVER.md` and `SCHEDULING-FIXTURES.md` are corrected rather than left to
  contradict the code.
- `Problem::integrity` now costs a resolution per section. It is a scan of the slots once plus
  a filter per section, which is nothing at the sizes in question, and it runs where integrity
  was already being checked.
- Two channels for one subject — a diagnostic and an integrity error — is a distinction a
  future reader could collapse. §4's table exists to stop that.

**Neutral.**

- `MaterialisedGrid::from_schedules` is crate-visible so a unit test can exercise the relation
  without building a grid of timeslots. The supported route stays `of`.

## Alternatives rejected

**Leave resolution to M4 and have M3 trust the recorded value.** The status quo, and it leaves
the hole open: a fixture or a loader could write anything and validation would agree with it.
It also guarantees two implementations eventually, because M5 and M11 each need the same answer.

**Have M3 *overwrite* a disagreeing resolution.** Tempting, and wrong twice. It would hide a
loader bug by papering over its output, and it would make `Problem` mutable at validation time,
which the whole design refuses.

**Report a disagreement as a blocking diagnostic.** Rejected because the remedy is not a field
the user can fill in — it is a code fix — and §3.2 reserves a blocking finding for inputs whose
remedy the user owns. A user told to "correct this section's bell-schedule resolution" has been
handed somebody else's defect.

**Add `G` to `Problem` as a stored field.** It is derivable in one scan and a stored copy could
disagree with the slots it summarises. ADR 0033 §1 already says `G` is *"derived from what is
materialised, not from what exists"*; a field would invite exactly the staleness this record is
fixing elsewhere.

**Put the resolver in `evara-constraints`.** It is not a constraint: it has an answer before any
placement exists, so `ARCHITECTURE.md` §3 rule 2 excludes it for the same reason
`lessons::DemandFinding` is excluded.

## Future acceptance tests

Implemented by M3 except where noted.

| # | Case | Expectation |
| --- | --- | --- |
| 1 | Each of ADR 0033 §2's three clauses, in isolation | Compatible. Clause 3 only at a school with exactly one campus |
| 2 | Clause 3 at a two-campus school | **Not** compatible; the section must name a campus |
| 3 | A schedule defined and not materialised | Not in `G`, not a candidate, no ambiguity |
| 4 | Candidate ordering, from a grid built out of order | Index order, always |
| 5 | Every catalogue fixture | Declared resolution equals computed, at every seed |
| 6 | A problem whose recorded resolution is altered by hand | `IntegrityError::BellScheduleResolutionDisagrees`, and **no** user-facing diagnostic |
| 7 | A section that resolves to none, with a correct recording | A blocking `no-bell-schedule` naming one of three causes, and no integrity error |
| 8 | M4's loader over a real project | Produces resolutions this function would; asserted by calling it rather than by a second implementation |
