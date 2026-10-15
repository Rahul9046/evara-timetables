# Architecture Decision Records

One file per decision. A record is written when a decision is hard to reverse, when it
rejects a reasonable alternative, or when a future contributor would otherwise be
tempted to undo it without knowing why.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-local-first-desktop.md) | Local-first Tauri desktop app, no server, no authentication | Accepted |
| [0002](0002-rust-in-process-solver.md) | Rust in-process solver; do not adopt Timefold or a solver service | Accepted |
| [0003](0003-sqlite-document-model.md) | SQLite canonical store, one file per project, sync-ready schema | Accepted — *"multiple schools means multiple files"* superseded by [0019](0019-multiple-schools-in-one-project.md) |
| [0004](0004-evara-project-format.md) | `*.evara` is a versioned ZIP of JSON, separate from external interchange | Accepted |
| [0005](0005-constraints-as-data.md) | Constraints are data; code provides an evaluator registry | Accepted |
| [0006](0006-group-based-scheduling-first.md) | Group-based scheduling first; request/sectioning schema preserved | Accepted |
| [0007](0007-rusqlite-refinery.md) | rusqlite + refinery, bundled SQLite, one connection factory | Accepted |
| [0008](0008-installation-identity-and-project-lifecycle.md) | Installation identity, settings database, `.evaraproj` | Accepted |
| [0009](0009-typescript-type-generation.md) | `ts-rs` for generated TypeScript types | Accepted |
| [0010](0010-structure-deletion-semantics.md) | RESTRICT ownership, SET NULL classification; opaque driver errors | Accepted |
| [0011](0011-stable-timeslot-identity.md) | Timeslot logical identity; materialisation never deletes | Accepted |
| [0012](0012-records-are-the-ipc-contract.md) | Repository records are the IPC contract; no parallel DTO layer | Accepted |
| [0013](0013-buildings-shared-across-campuses.md) | A building may serve many campuses; `building_campus` is the association set | Accepted |
| [0014](0014-rooms-shared-across-campuses.md) | A room may serve many campuses; one identity, so occupancy is global | Accepted |
| [0015](0015-school-working-days.md) | School working days are a calendar default, stored as a seven-bit set; `cycle.week_count` retired from the UI only | Accepted |
| [0016](0016-school-logos-in-the-project.md) | School logos are BLOBs in the project, validated from their bytes; Evara's mark is the fallback | Accepted |
| [0017](0017-weekly-and-custom-cycles.md) | A cycle is weekly or custom; a weekly cycle holds all seven weekdays and no school's calendar | Accepted |
| [0018](0018-people-identity-and-deactivation.md) | People keep their identity; leaving is a state, not a deletion | Accepted — amended by [0021](0021-project-level-people-and-school-membership.md), which gives *leaving* two scopes |
| [0019](0019-multiple-schools-in-one-project.md) | A project may describe several schools, each owning its own academic structure | Accepted |
| [0020](0020-in-place-table-rebuild.md) | A table can be rebuilt in place under deferred foreign keys, inside a migration | Accepted |
| [0021](0021-project-level-people-and-school-membership.md) | People belong to the project; membership, eligibility and enrolment belong to the school | Accepted |
| [0022](0022-resources-shared-across-campuses.md) | A resource may serve many campuses, and so many schools; one identity, so availability is global | Accepted |
| [0023](0023-the-interface-scopes-to-one-school.md) | The interface scopes to one school, chosen once; project-wide reads are composed, and a certain refusal is explained rather than offered | Accepted |
| [0024](0024-setup-is-ordered-by-what-a-school-knows.md) | School Setup is ordered by what a school knows rather than by dependency; a hidden field is carried, not dropped | Accepted |

Statuses: **Proposed** (awaiting a decision), **Accepted**, **Superseded by NNNN**,
**Deprecated**. Accepted records are not edited to reflect a change of mind — a new
record supersedes them.

Decisions still open are tracked in [../OPEN-DECISIONS.md](../OPEN-DECISIONS.md) and
graduate to an ADR once settled.
