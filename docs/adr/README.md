# Architecture Decision Records

One file per decision. A record is written when a decision is hard to reverse, when it
rejects a reasonable alternative, or when a future contributor would otherwise be
tempted to undo it without knowing why.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-local-first-desktop.md) | Local-first Tauri desktop app, no server, no authentication | Accepted |
| [0002](0002-rust-in-process-solver.md) | Rust in-process solver; do not adopt Timefold or a solver service | Accepted |
| [0003](0003-sqlite-document-model.md) | SQLite canonical store, one file per project, sync-ready schema | Accepted |
| [0004](0004-evara-project-format.md) | `*.evara` is a versioned ZIP of JSON, separate from external interchange | Accepted |
| [0005](0005-constraints-as-data.md) | Constraints are data; code provides an evaluator registry | Accepted |
| [0006](0006-group-based-scheduling-first.md) | Group-based scheduling first; request/sectioning schema preserved | Accepted |
| [0007](0007-rusqlite-refinery.md) | rusqlite + refinery, bundled SQLite, one connection factory | Accepted |
| [0008](0008-installation-identity-and-project-lifecycle.md) | Installation identity, settings database, `.evaraproj` | Accepted |
| [0009](0009-typescript-type-generation.md) | `ts-rs` for generated TypeScript types | Accepted |
| [0010](0010-structure-deletion-semantics.md) | RESTRICT ownership, SET NULL classification; opaque driver errors | Accepted |
| [0011](0011-stable-timeslot-identity.md) | Timeslot logical identity; materialisation never deletes | Accepted |

Statuses: **Proposed** (awaiting a decision), **Accepted**, **Superseded by NNNN**,
**Deprecated**. Accepted records are not edited to reflect a change of mind — a new
record supersedes them.

Decisions still open are tracked in [../OPEN-DECISIONS.md](../OPEN-DECISIONS.md) and
graduate to an ADR once settled.
