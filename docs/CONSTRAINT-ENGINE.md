# The Constraint Engine

What `evara-constraints` is, what each implemented rule checks, what it deliberately does not,
and what a caller is entitled to conclude from a clean report.

[CONSTRAINTS.md](CONSTRAINTS.md) is the **catalogue**: every constraint type Evara intends to
support, with a status column that is a plan. This file is the **engine**: the eight types that
exist, and the machinery they run in. Where the two disagree about a semantic detail, this one
is the record of what the code does.

Shipped in M5. Nothing here generates, improves or searches for a timetable — there is no
solver. The engine answers one question about an arrangement it is **given**: *what is wrong
with this?*

---

## 1. The shape of it

```
evara-core          the vocabulary:  ConstraintType, ConstraintLevel, CompiledConstraint,
                                     Violation, Explanation, AffectedSet, StoredConstraint
   ^
evara-constraints   the engine:      Registry, ConstraintEvaluator, Occupancy,
                                     eight evaluators, evaluate(), evaluate_placement()
   ^
evara-solver        M6:              searches, using AffectedSet and Score
```

`evara-db` depends on `evara-core` and **not** on `evara-constraints` (`ARCHITECTURE.md` §3),
which is why the vocabulary is one crate lower than the engine: the loader produces a
`Problem`, the repository produces `StoredConstraint` rows, and both have to name types the
evaluators also name.

| File | Holds |
| --- | --- |
| `registry.rs` | `type_key` → evaluator, and `compile` from stored rows |
| `occupancy.rs` | what an assignment occupies, resolved once, plus the interval sweep |
| `params.rs` | the JSON Schema per type, and the one parser of a stored `params` |
| `clash.rs` | `teacher-clash`, `student-clash`, `room-clash` |
| `facilities.rs` | `room-type-required` |
| `availability.rs` | `entity-unavailable` |
| `timing.rs` | `duration-contiguous`, `no-teaching-in-period` |
| `completeness.rs` | `activity-scheduled` |
| `evaluation.rs` | `evaluate`, `evaluate_placement`, `EvaluationReport` |

---

## 2. The eight rules

Seven hard, one medium. This is
[SCHEDULING-ARCHITECTURE-DECISIONS.md](SCHEDULING-ARCHITECTURE-DECISIONS.md) §6's first
constraint set, less `activity-fixed` — see §3.

| `type_key` | Level | What it refuses |
| --- | --- | --- |
| `teacher-clash` | HARD | One teacher in two occurrences whose minutes overlap |
| `student-clash` | HARD | Two occurrences whose audiences **confirmedly** overlap, at overlapping minutes |
| `room-clash` | HARD | One `room_id` holding two occurrences at overlapping minutes |
| `room-type-required` | HARD | A room that is not a candidate for the section — wrong type, unbookable, wrong campus |
| `entity-unavailable` | HARD | A placement in a slot a stored rule marks `UNAVAILABLE` for the teacher, the audience or the room |
| `duration-contiguous` | HARD | A multi-period occurrence not occupying one unbroken run of teaching periods on one day |
| `no-teaching-in-period` | HARD | A lesson in a slot whose period is not for teaching |
| `activity-scheduled` | MEDIUM | Nothing. It reports each occurrence that is **not** placed |

### Clashes compare minutes, never timeslot identifiers

All three clash rules ask the same question — *do these two occurrences occupy overlapping
minutes?* — and differ only in which pairs are worth asking about. ADR 0032 rule 2: a
timeslot's identity is `(term_id, cycle_day_id, period_id)` with no campus in it, and
`period_structure` is campus-scoped, so a school running two campuses on two bell schedules
holds two disjoint slot sets over the same minutes. A shared hall booked at 09:00 from each
campus has two different `timeslot_id`s and one physical double-booking. Comparing identifiers
is wrong *exactly* when sharing matters, and wrong silently.

Intervals are half-open, `[start, end)`, so a 09:00–10:00 lesson and a 10:00–11:00 lesson do
not clash.

### `student-clash` enforces what was proved, and nothing more

Overlap is ADR 0035's three-state answer, and the rule consults it through
`Overlap::clashes()` rather than through `!= Disjoint`. So:

- `Overlaps` — a hard violation.
- `Disjoint` — legal, and not even a warning.
- `Indeterminate` — **not** a violation. ADR 0035 §6: enforce what is proved. A year group
  whose children's overlap nobody has declared would otherwise be able to hold one lesson at a
  time, which is the failure that record rejects by name. The *count* of such pairs is M3's
  `group-overlap-indeterminate`, reported before a solve so the gap is impossible to miss.

Two occurrences of the **same** section never produce a student clash: they are the same
people, and the rule is consulted only between different sections. They do produce a teacher
clash and a room clash, correctly.

### `room-type-required` calls the candidate predicate rather than restating it

`RoomFact::is_candidate_for` is the one implementation of the room candidate domain —
bookable, reachable from the section's campus, of the required type. The evaluator asks that
function and reports its answer; it does not compare a room's type to a section's requirement
itself, which would be a second implementation and would miss the other two conditions.
`SCHEDULING-ARCHITECTURE-DECISIONS.md` §3.1: *"a domain the manual editor could bypass is a
rule living in two places."*

Three failures are this one rule: a required type with no room booked, a room that is not a
candidate, and a section whose candidate set is empty. The last overlaps M3's
`no-room-of-required-type` deliberately — M3 says *this can never work*, and this says *this
placement does not*.

**Capacity is not here.** ADR 0034 defers `room-capacity` until a section's expected attendance
exists to compare against, and `ExpectedAttendance::Unknown` holds no number precisely so that
no `0` can leak into a comparison.

### `entity-unavailable` is about `UNAVAILABLE` alone

`availability.status` has three values and this rule enforces one. A `DISCOURAGED` slot is
**legal** and merely worse, which is what makes it soft; the soft half is `preferred-time`,
which is not in the MVP. A missing rule is not a restriction — `V11` stores only exceptions, so
an entity with no rows is free everywhere, and *unconstrained* is a distinct fourth state
rather than an empty set of permissions.

The rule reads the index M4 compiled, which holds each stored rule against the slots it
actually names (ADR 0033 §7). It never re-expands a rule's granularity: a teacher unavailable
"period 1, every day" at a two-schedule school has said something about one schedule, and the
engine inherits that limitation rather than guessing past it. M3's
`teacher-under-several-bell-schedules` is what makes the gap visible.

No date is read. `effective_from` and `effective_to` are carried and consulted by nothing,
because a cycle carries no dates (ADR 0032 §5).

### `duration-contiguous` uses ADR 0033 §5's measure

`runs_on` is the one implementation of *"`d` ordinal-consecutive `TEACHING` periods of its own
resolved schedule, on one cycle day"*, and this rule asks it whether the run an occurrence
actually occupies is a legal one. Three things that measure already knows, and this rule
therefore inherits: a non-teaching period between two teaching ones breaks a run; a mere
wall-clock gap between consecutive bells does **not**, because a gap between bells is a gap
between bells; and a run never spans two schedules even where their minutes are adjacent.

`allowBreakSpan` is the MVP's single parameter, and it is the permission
`CONSTRAINTS.md` describes as *"not spanning a break unless permitted"*.

### `no-teaching-in-period` exists for the editor

The loader restricts an activity's candidate slots to teaching ones, so a generated timetable
should never place a lesson in a break. ADR 0033 §4 keeps the constraint anyway, *"because
manual editing can name a non-teaching slot and the editor must refuse for the same reason the
generator never offered it"*. Both read the stored `timeslot.is_teaching` rather than inferring
teachability from a period's label.

### `activity-scheduled` is MEDIUM, and that is the point of it

One violation per unplaced occurrence, because each is a separate thing to place and a repair
needs to know which. `CONSTRAINTS.md` §1 on the level: *"It lets the solver return a usable
partial timetable and tell the user exactly what it could not place, instead of returning
nothing."*

M6 constructs incrementally and every intermediate state has unplaced occurrences. Promoting
this to HARD would make the first of them infeasible and every one after it, which is the same
as having no construction phase. **M5's brief classified it as hard; the accepted record does
not, and `ConstraintType::default_level` is where that is settled.** A school that will not
accept an unplaced lesson can still raise its own instance to HARD — ADR 0005 stores the level
per instance for exactly that disagreement, and ADR 0039 allows raising.

A duplicate placement cannot be represented: an `Assignment` is one `Option<Placement>` per
activity index, so an occurrence has one placement or none. The duplicate the brief asks about
is prevented by the type rather than detected by a rule.

---

## 3. `activity-fixed` is deferred, and not quietly

`SCHEDULING-ARCHITECTURE-DECISIONS.md` §6's hard set is seven of the above **plus
`activity-fixed`**, *"once locks exist"*. They do not exist at `V16`: there is no
`activity.is_locked` column, the roadmap keeps lock toggles with Phase 8's manual editing
because a lock is about a *placed* timetable rather than about the demand, and
`Problem::pinned` is a declared hook that nothing populates.

So `ConstraintType` has no variant for it. A registered rule that can never fire *"tells the
user their rooms are being checked"* when they are not, and `CLAUDE.md` §4 is explicit that no
type is supported until all five of its artefacts exist.

**It is not promoted to hard by implication.** When locks land, `activity-fixed` arrives with
its evaluator, its schema, its tests and its row in CONSTRAINTS.md, and the hard set becomes
eight.

---

## 4. A rule is mandatory in the registry, not in a row

This is [ADR 0039](adr/0039-a-mandatory-constraint-lives-in-the-registry.md) and the single
most important thing to know about configuring the engine.

- Evaluation runs every implemented type, whatever the database holds. `Registry::mandatory()`
  is what an unconfigured project evaluates, and **no rows are seeded**.
- A `constraint_instance` row *tunes* a type rather than adding one: `Registry::compile`
  substitutes it over the built-in, so a type is evaluated exactly once.
- A row may change the weight, the term and the parameters freely, and may **raise** a level.
  It may not disable a mandatory rule or weaken it below the shipped level, and it may not
  narrow one. Both the repository and the compiler refuse — the first so a school hears about
  the mistake while making it, the second because a row can arrive from an import or a newer
  build and the engine must not trust the writer.
- Removing a configuration returns the school to the shipped level and weight. It does not
  switch a check off, because there was never a row making the check happen.

### Where a stored value is validated

| Checked | Where |
| --- | --- |
| `type_key` is implemented; `level` is one of three; mandatory floor; `weight >= 0`; `params` is a JSON object | `evara_db::repo::constraints`, from `evara-core` |
| the **shape** of `params` against the type's JSON Schema | `evara-constraints`, in `Registry::compile` |

The schemas live beside the evaluators and `evara-db` may not depend on that crate, so ADR
0005's *"validates writes"* becomes a division of labour rather than one call. A malformed
parameter cannot reach an evaluator — compilation refuses the whole set and names the rule —
and a caller with both crates in scope compiles a candidate row before writing it. ADR 0039 §5
is the record.

---

## 5. Occupancy: resolved once, read by every rule

An `Assignment` says which timeslot each occurrence *starts* in. A clash is about the minutes
it *occupies*, which for a `duration_periods = d` occurrence is `d` ordinal-consecutive slots
of its resolved bell schedule. Turning the first into the second is the same work for all three
clash rules and both timing ones, so `Occupancy::of` does it once and every evaluator reads the
result. No rule re-derives a span, so no two rules can disagree about one.

**Resolution is permissive; judging is a rule's job.** If an occurrence cannot take `d` slots
from where it was placed — the day runs out, a non-teaching period interrupts, the slot belongs
to another schedule — the span is built from what is there and records what went wrong.
`duration-contiguous` reports the short or broken run and `no-teaching-in-period` the break; a
resolver that refused would make both unreachable and turn a reportable timetable into an
unreadable failure.

**The pair sweep is not a nested loop.** A school with 500 occurrences has 124 750 unordered
pairs and almost none of them share a day. `overlapping_pairs` sorts by
`(cycle day, start, activity)` and keeps an active list of spans whose interval has not ended,
so it compares only spans that genuinely coexist. Teacher and room clash narrow by teacher and
by room first.

---

## 6. What a report says

`EvaluationReport` answers four separate questions, and a caller needs more than one of them:

| Question | Method | Meaning |
| --- | --- | --- |
| Could the assignment be read at all? | `is_readable()` | `false` means it names an activity or timeslot this problem does not hold. **Nothing is scored** — a number about rows from somewhere else looks like an answer |
| Is any hard rule broken? | `is_usable()` | `false` means the arrangement cannot be run |
| Is everything placed? | `completeness` | `Partial { placed, required }` is an ordinary state, not an error |
| Both? | `is_adoptable()` | Complete and nothing hard broken. The strongest thing M5 can say |

`is_usable()` is deliberately not called `is_feasible` or `is_valid`. A usable *partial*
timetable is still partial, and `evara_core::lessons`' rule binds this crate too: **never
report that a demand can be scheduled.**

A `Violation` is structured — the rule, the level, the occurrences, the subject entity, the
related entities, the cycle day, the wall-clock span, the integer penalty — and never a
pre-rendered string. `explain()` turns one into a sentence, and that wording lives with the
rule so it cannot drift from the check that produced it.

### Scores are integers, compared lexicographically

Three levels, `hard` before `medium` before `soft`. A weight becomes an integer penalty at
compile time and not at scoring time, so the same arrangement scores identically on two
machines. There is no float anywhere in the path — see
[ADR 0040](adr/0040-the-constraint-tables-are-entities-and-their-scope-is-cleared.md) §3 for
why the database column is an `INTEGER` against an older sketch.

### Determinism

Rules run in `ConstraintType::all()`'s order, chosen so a reader meets causes before
consequences. Each evaluator visits entities in index order, the pair sweep sorts, and nothing
iterates a hash map. Same problem, same assignment, same configuration, identical report —
`SOLVER.md` §5 calls this non-negotiable, because a seed over a varying order is not a seed.

---

## 7. Two entry points, one implementation

`evaluate(problem, assignment, constraints)` runs everything. It is the reference
implementation `SOLVER.md` §4 calls *"always correct, never fast"*.

`evaluate_placement(problem, assignment, constraints, moved)` answers the question M6's search
and M11's `validate_move` actually ask — *would putting this occurrence here break anything?* —
by running only the rules whose `affected()` set contains the occurrence that moved. For
example `room-type-required` is entirely local, so moving one lesson cannot change its verdict
about another; `activity-scheduled` depends on the whole timetable and is always selected.

It is **incremental-friendly, not incremental**. It holds no cache and maintains no index, and
it evaluates each selected rule in full rather than computing a delta. That is the deliberate
stopping point: a hand-written delta is *"the single most bug-prone part of any solver"*, and
`SOLVER.md` §4's mitigation is to build the reference first and gate the incremental one on a
property test against it. M6 writes `IncrementalScorer`; M5 writes the thing it will be checked
against.

---

## 8. Running it

```
evara evaluate <fixture> [--seed N] [--json]
evara evaluate --project <path.evaraproj> --school <id> --cycle <id> [--term <id>] [--json]
```

A fixture is evaluated against its **own known-feasible arrangement** where it ships one. That
is the sharpest check in the suite: M2 built those arrangements against ADRs 0032–0035 and
verified them independently, so the eight evaluators finding nothing hard in them is two
readings of the same records agreeing.

A project has no stored timetable to score — `timetable` arrives in M7 — so the empty
arrangement is evaluated and the useful half of the report is above the findings: the rules
this school will be judged by, its own rows compiled on top of the mandatory eight. The file is
opened read-only, by the same route `evara validate --project` takes.

Exit codes: `0` nothing hard broken, `1` a hard rule broken, `2` the command failed or the
configuration could not be compiled. A medium violation leaves it at `0`, because an unplaced
lesson makes a timetable incomplete rather than unusable and a code that could not tell those
apart would undo §2's distinction at the last step.

---

## 9. What is not here

| Not implemented | Where it goes |
| --- | --- |
| `activity-fixed` | §3 — with Phase 8's locks |
| `room-capacity` | ADR 0034 — with `class_section.expected_attendance` |
| every soft type (`preferred-time`, `spread-across-days`, `teacher-min-gaps`, …) | After the MVP. Nothing in the engine optimises, and `ConstraintLevel::Soft` exists only because a level is stored per instance |
| `resource-clash`, `same-period`, the workload ceilings | CONSTRAINTS.md §3's `M1` set beyond the first nine |
| `IncrementalScorer` and the property test between the two scorers | M6. §7 |
| a configuration screen | Phase 4's UI half. The repository and the schemas exist; nothing in the interface writes a row yet |
| persisted violations | M7, with `timetable` and `violation` |

A clean report from this engine means: *of the eight rules implemented, none is broken by the
arrangement you gave me.* It does not mean the timetable is good, complete, or possible to
improve on — and where the engine cannot check something, M3's pre-solve validation says so
out loud rather than leaving the silence to be read as approval.
