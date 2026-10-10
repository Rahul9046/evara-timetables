# Contributing to Evara Timetables

Thanks for helping build an open-source school timetabling application.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first, then
[docs/ROADMAP.md](docs/ROADMAP.md) for what is currently being built.

## Ground rules

- Open an issue for substantial features or new constraint families.
- Keep vendor-specific integration code outside the core domain and solver. Where a
  vendor concept does not fit, add a **generic** constraint type with a vendor-neutral
  name — never a vendor branch in the core.
- Every scheduling constraint must include automated tests and a documented scenario.
- Prefer small pull requests with a single clear purpose.
- **Never commit real student, teacher or school data.** Synthetic fixtures only. This
  applies to bug-report attachments too.
- No cloud services, no authentication, no network calls on normal code paths.
- Dependencies must be permissively licensed (MIT / Apache-2.0 / BSD / ISC). No paid
  SaaS, no mandatory external service.

## Layering rules

From [ARCHITECTURE.md](docs/ARCHITECTURE.md) §3. Violations are architecture bugs, not
style preferences:

1. `evara-core` — domain types only. No I/O, no dependency on the other crates.
2. `evara-constraints` — depends on `evara-core` only. Never touches SQL.
3. `evara-solver` — receives an in-memory problem, returns a solution. Never opens a
   database.
4. `evara-db` — the only crate that knows SQL.
5. `evara-integrations` — depends on `evara-core` and nothing else.
6. `src-tauri` commands orchestrate; business logic does not live in them.
7. The React frontend holds no scheduling rules and no persistence logic. It asks the
   backend whether something is legal; it never decides.

A scheduling rule in the UI, SQL in the solver, or a vendor name in the core will be sent
back.

## Definition of done for a constraint

A constraint type is not "supported" until all five exist:

1. a stable `type_key` and a registry entry in `crates/evara-constraints/`
2. a JSON Schema for its parameters
3. documented semantics in [docs/CONSTRAINTS.md](docs/CONSTRAINTS.md)
4. an isolated unit test, plus a synthetic fixture exercising it
5. an `explain()` implementation producing a human-readable reason

The reference-vs-incremental scorer property test must stay green.

## Development direction

The first milestone is a complete vertical slice: define a school, generate a feasible
timetable, inspect its score, edit it by hand and understand its violations — all
offline, on one machine.
