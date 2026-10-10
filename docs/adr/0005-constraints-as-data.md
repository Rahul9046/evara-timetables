# ADR 0005 — Constraints are data; code provides an evaluator registry

- Status: **Accepted**
- Date: 2026-10-06

## Context

Evara must eventually support 40+ constraint families spanning teachers, students, rooms,
resources, frequency, distribution, relationships, lines, electives, multi-campus travel
and daily operations. Each needs a configurable level (`HARD` / `MEDIUM` / `SOFT`), a
weight, an enable toggle and a scope. The explicit engineering rule is that there must be
no single giant function containing all scheduling rules, and that each constraint must be
individually testable.

## Decision

A constraint is a **row** in `constraint_instance`: `type_key`, `level`, `weight`,
`enabled`, JSON `params`, plus `constraint_ref` rows for scope. Code provides a
**registry** mapping `type_key` to a `ConstraintEvaluator` implementation.

Each evaluator supplies:

| Member | Purpose |
| --- | --- |
| `type_key` | stable identity, the thing stored in the database |
| `param_schema` | JSON Schema — validates writes **and** generates the configuration form |
| `evaluate` | full evaluation; the reference implementation, always correct |
| `affected` | which activities a change can alter the score of; scopes incremental work |
| `explain` | structured human-readable reason, powering "why is this move invalid?" |

Adding a constraint means one module, one registry line, one test file, one fixture and
one row in [CONSTRAINTS.md](../CONSTRAINTS.md). Nothing else changes.

Levels are stored per instance, not hardcoded: one school runs "teacher preferred times"
as `SOFT`, another as `HARD`. `MEDIUM` exists so the solver can return a usable partial
timetable and say exactly what it could not place — unscheduled activities, unmet period
counts, unstaffed classes, and repair disruption all live there.

Scope is expressed as `constraint_ref(constraint_id, role, entity_kind, entity_id)` rows
rather than entity IDs inside `params`, so scope references are real rows with a cascading
delete. `params` holds only scalars.

The same registry is consumed by the solver and by the manual-move validator, so there is
one implementation of every rule ([ADR 0002](0002-rust-in-process-solver.md)).

## Consequences

- No migration is needed to add a constraint type, which is what makes 60+ types
  tractable.
- JSON `params` get no foreign-key enforcement. `constraint_ref` pays most of that back;
  the remainder is covered by repository-level validation and an integrity sweep.
- JSON Schema is load-bearing: it is the single declaration of a constraint's parameters,
  used by validation and by form generation. A schema that drifts from its evaluator is a
  real defect class, so the schema lives next to the evaluator and is tested against it.
- `explain()` is mandatory, not optional. A constraint that cannot say why it was violated
  cannot ship, because the product's differentiator is explanation.
- Vendor features map onto generic types ([CONSTRAINTS.md](../CONSTRAINTS.md) §4). Where a
  vendor concept does not fit, the answer is a new generic type with a vendor-neutral
  name — never a vendor branch in the core.
- ~22 evaluators in Milestone 1 and 60+ eventually, each with an incremental path, is the
  dominant share of total effort — more than the solver core (OPEN-DECISIONS R3).

## Alternatives considered

**A column or table per constraint family.** Full referential integrity, but 40+
migrations and 40+ repositories, and every new rule becomes a schema change.

**Hardcoded constraints with only weights configurable.** Much simpler and far less
flexible; schools genuinely disagree about which rules are hard, and that disagreement is
the product.

**A scripting language for user-defined constraints (Lua, Rhai, a DSL).** Maximum
flexibility, but it puts arbitrary user code in the solver's inner loop, makes performance
unpredictable, and makes `explain()` impossible to do well. Reconsider only once the
built-in registry is mature.
