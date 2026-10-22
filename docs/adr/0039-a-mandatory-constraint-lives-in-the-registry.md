# ADR 0039 — A rule whose absence would make a timetable wrong lives in the registry, not in a row

- Status: **Accepted**
- Date: 2026-10-22
- Decides: whether the first constraint set is **seeded as `constraint_instance` rows** or
  **built into the evaluator registry**; what a stored row may and may not do to one of those
  rules; and where a stored `params` value is validated, given that the schemas cannot live in
  the crate that writes the row. Raised while implementing M5 against
  [ROADMAP.md](../ROADMAP.md) Phase 4's *"the default constraint set seeded into new
  projects"*.
- Amends: [ADR 0005](0005-constraints-as-data.md) — not its decision, which stands, but the
  scope of the word *data*. ADR 0005's §"Rejected alternatives" is untouched.
- Binds: `evara-constraints`' registry, `evara_db::repo::constraints`, and every caller that
  evaluates a timetable. Implemented in M5.

## Context

ADR 0005 settled that constraints are first-class data: a `constraint_instance` row carries a
`type_key`, a level, a weight, an enabled flag and JSON parameters, and code provides an
evaluator registry rather than a list of hardcoded rules. That decision is right and is not in
question here. Nothing about the MVP's rule set would be easier if the rules were `if`
statements.

Phase 4 then described *"the default constraint set seeded into new projects"*, and M5's brief
repeated it. Implementing that raised three problems, in ascending order of seriousness.

**A seeded row can be deleted.** `teacher-clash` as a row is a row with a delete button behind
it — reachable through the repository, through an import, through a `.evara` round trip, or by
someone opening the file in a SQLite browser. A project whose `teacher-clash` row has gone
produces timetables that double-book teachers and **reports no violation**, which is the exact
failure mode `CLAUDE.md` §2 calls the product being broken: *"If generation and the editor can
ever disagree about whether a placement is legal, the product is broken."* A rule that is
absent from the engine is a rule the editor and the generator agree about — they agree it does
not apply.

**A seeded row can be disabled, which is worse than deleted.** `enabled = 0` leaves the row
visible in the configuration list, so the screen shows `teacher-clash` and the engine does not
run it. There is no state in which a user is more confident their timetable is checked.

**Seeding has no honest author.** Every entity table carries change-log triggers, and the
`INSERT` trigger records the installation through the per-connection `evara_site_id()`
function. That function is registered by the connection factory and is **not** available while
refinery applies a migration, so rows seeded in `V16` would either be unattributable or refused
outright. Seeding from the first ordinary open instead means "a new project" is a moving target
— a project created by this build and a project created by the next would differ in rows
neither user chose.

Against that: the reason ADR 0005 exists in the first place is that *"schools genuinely
disagree about which rules are hard, and that disagreement is the product."* A design that
removed the disagreement to solve the problems above would be solving the wrong problem.

## Decision

**The implemented constraint types are mandatory in the registry. No rows are seeded. A stored
row *tunes* a mandatory rule and can neither remove nor weaken it.**

### 1. Evaluation runs every implemented type, whatever the database holds

`Registry::mandatory()` returns one `CompiledConstraint` per entry of
`ConstraintType::all()`, at the level the type ships with, weight 1, no narrowing. A project
that has configured nothing evaluates all eight. There is no row to delete, so no deletion can
disable a safety rule, and a configuration list that is empty is *correct* rather than
alarming: it says this school has not disagreed with anything.

### 2. A stored row replaces its built-in rather than adding a second

`Registry::compile(stored, term)` starts from the mandatory set and substitutes each stored
row over the built-in of the same type. So a type is evaluated exactly once, a tuned weight or
term takes effect, and two rows for one type are not two evaluations — the first in row order
wins, which is deterministic because the repository lists them by `(type_key, id)`.

### 3. What a row may change, and what it may not

| Field | A mandatory type | Why |
| --- | --- | --- |
| `weight` | free | A penalty is a school's own arithmetic. |
| `term_id` | free | A rule may apply to one term; a row for another term is dropped. |
| `params` | free, within the type's schema | The configuration ADR 0005 is for. |
| `level` | **raise only** | `activity-scheduled` may go MEDIUM → HARD. HARD → SOFT is refused. |
| `enabled` | **may not be `0`** | See the context: a disabled rule that is still listed is the most misleading state available. |
| `constraint_ref` | **refused for all eight** | §4. |

The level floor runs upwards only. A school may insist on something stronger than Evara ships
— that is the disagreement ADR 0005 protects — but "a teacher in two rooms at once is a
preference" is not a disagreement about policy, it is a request for a timetable that cannot be
used.

Both halves are enforced **twice, deliberately**: `Registry::compile` refuses such a row with
`ConstraintError::MandatoryWeakened`, and `Constraints::create` / `Constraints::update` refuse
to write one in the first place. The write-side refusal exists so a school hears about the
mistake while making it rather than when the next solve will not start; the read-side refusal
exists because a row can arrive from an import, a `.evara` archive, or a newer build, and the
engine must not trust the writer. Neither is redundant with the other.

### 4. None of the eight can be narrowed

Each of the implemented types is a property of the whole timetable. Narrowing `teacher-clash`
to three named teachers does not mean "check these carefully"; it means "stop checking
everybody else", and a safety rule that is off for part of a school is worse than one that is
off everywhere, because nothing says so.

`ConstraintType::supports_narrowing()` therefore returns `false` for all eight, compiling a
narrowed instance is an error rather than a scope quietly ignored, and `Constraints::add_scope`
refuses the write by name. The `constraint_ref` table is still the right schema — ADR 0005's
argument for normalised scope references is unaffected, the first narrowable type needs it, and
a project written by a newer build can hold rows this build must not lose.

The refusal is on the **write** as well as the compile for a specific reason: compilation is
all-or-nothing, so one scope row somebody added in a screen that allowed it would make a
school's entire configuration uncompilable, and the first they would hear of it is a solve
refusing to start.

### 5. `params` is validated in two places because it cannot be validated in one

ADR 0005 says the JSON Schema *"validates writes and generates the configuration form"*. The
schemas live beside the evaluators in `evara-constraints`, and `ARCHITECTURE.md` §3 forbids
`evara-db` from depending on that crate — the layering that keeps the scheduling rules out of
the persistence layer. The row is written by `evara-db`. So the sentence cannot be literally
true as stated, and the resolution is a division rather than a dependency:

| Checked | Where | How |
| --- | --- | --- |
| `type_key` is implemented; `level` is one of three; a mandatory rule is not weakened or disabled; `weight >= 0`; `params` is a JSON object | `evara_db::repo::constraints` | from `evara-core`, which both crates depend on |
| the **shape** of `params` against the type's JSON Schema | `evara-constraints` | `Registry::compile`, which every reader of a stored row goes through |

A malformed parameter therefore cannot reach an evaluator: compilation refuses the whole set
and names the rule. What it *can* do is sit in the database until the next compile, so a
caller with both crates in scope — the command layer, and `evara evaluate` — compiles the
candidate row before writing it. One of M5's CLI tests asserts this division rather than
describing it: the repository accepts `{"allowBreakSpan": "yes"}`, and the registry is what
refuses it.

The alternative — a JSON Schema validator in `evara-core` so the write could validate fully —
would put a schema engine and its licence review in the crate that is defined by having no
dependencies, to validate two shapes: an empty object, and an object with one optional
boolean. When a third parameterised type arrives that calculus changes.

## Consequences

- An unconfigured project is **fully checked**. This is the main thing bought.
- The configuration screen Phase 4 describes shows eight rules with no rows behind them, and a
  row appears when a school changes something. The screen's "reset" action is a delete.
- `constraint_instance` is empty in every project today, so `V16`'s two tables carry no rows
  until somebody tunes something. That is a feature of this decision and not evidence the
  migration was premature: the schema is what the tuning writes into.
- `SchoolDependents` counts configured constraints, because `constraint_instance.school_id` is
  a RESTRICT edge and a count that ignored it would report nothing in the way of a school
  delete that then failed.
- When a type arrives that *is* narrowable, `supports_narrowing()` returns `true` for it, the
  compiler makes every match arm reconsider, and `Constraints::add_scope` gains the
  existence-and-ownership checks that would be unreachable code today.

## Alternatives rejected

**Seed the eight rows and add a `is_protected` column.** This is the honest version of
seeding: the rows exist, and the schema refuses to delete or disable the protected ones. It
was rejected because it buys nothing and costs a column. The rows would be identical in every
project, could still be edited into uselessness through any path that bypasses the repository,
and the only question they answer — *is this rule evaluated?* — is answered by the registry
already. A column whose value is the same in every row of every project is not data.

**Seed the rows and trust the interface not to offer a delete.** The interface is not the only
writer: imports, `.evara` archives and SQLite browsers all exist, and `CLAUDE.md` §2's single
most important rule is about two code paths never disagreeing about legality. A guarantee that
holds only through one path is not a guarantee.

**Let a school weaken a hard rule if it insists.** Considered seriously, because it is the
most consistent reading of ADR 0005. Rejected on what the resulting artefact would be: a
timetable with a teacher in two rooms at once, which no school can run, produced by software
that reported no problem. Evara refusing to produce it is not Evara overruling a school's
judgement about its own policy; it is Evara declining to describe an impossible timetable as a
usable one. `evara_core::lessons`' rule — *never report that a demand can be scheduled* — is
the same instinct pointed the other way.

**Make the whole set configurable later and ship hardcoded rules now.** Rejected: it would
mean the evaluators reading nothing, and the first school that wants a weight would need a
schema, a repository and a screen before it could have one. The mechanism is built and used;
what is withheld is only the ability to turn a safety rule off.

## Verification

- `a_mandatory_rule_cannot_be_disabled`, `a_mandatory_rule_cannot_be_weakened_below_its_shipped_level`,
  `a_medium_rule_may_be_raised_to_hard`, `a_stored_row_tunes_its_type_rather_than_adding_a_second`,
  `an_unconfigured_project_evaluates_all_eight`, `compiling_nothing_gives_the_mandatory_set`,
  `narrowing_a_rule_that_cannot_be_narrowed_is_refused_not_ignored` — `evara-constraints`.
- `a_mandatory_rule_cannot_be_turned_off`, `a_rule_may_be_raised_but_not_weakened`,
  `an_unconfigured_school_hands_over_nothing`, `narrowing_a_rule_this_build_implements_is_refused_by_name`,
  `removing_a_configuration_returns_the_school_to_the_shipped_rule`,
  `a_configured_rule_blocks_deleting_its_school_and_the_count_says_so` — `evara-db`.
- `a_configuration_this_build_cannot_read_fails_the_command` — `evara-cli`, which is where §5's
  division is asserted end to end.
