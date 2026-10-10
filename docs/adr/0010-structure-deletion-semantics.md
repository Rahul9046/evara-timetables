# ADR 0010 — Structure deletion semantics, and an opaque driver boundary

- Status: **Accepted** (approved 2026-10-06, resolves OPEN-DECISIONS D19, D20)
- Date: 2026-10-06
- Amends: [ADR 0003](0003-sqlite-document-model.md)

## Context

Phase 1C introduced the first entity tables. Two things had to be settled first.

**`DATA-MODEL.md` §3 documented columns and parent links but never stated `ON DELETE`
behaviour.** Migrations are immutable once released and changing a foreign key in SQLite
means rebuilding the table, so guessing was expensive.

**`DbError` leaked the driver.** `DbError::Open` carried a `rusqlite::Error` as its source
and `DbError::Sqlite` wrapped one directly, which meant `evara-desktop` needed `rusqlite`
as a dev-dependency purely to construct an error in a test — a crate that must contain no
SQL depending on the SQL driver.

## Decision

### Deletion follows ownership, and defaults to refusing

| Edge | Behaviour |
| --- | --- |
| `campus.school_id` | `RESTRICT` |
| `building.campus_id` | `RESTRICT` |
| `room.campus_id` | `RESTRICT` |
| `resource.school_id`, `resource.campus_id` | `RESTRICT` |
| `academic_year.school_id` | `RESTRICT` |
| `term.academic_year_id` | `RESTRICT` |
| `room.building_id` | `SET NULL` |
| `room.room_type_id` | `SET NULL` |

Ownership edges refuse. School data is expensive to recreate and will shortly carry
timetables built on top of it; a cascade turns one mis-click into silent structural loss,
and the error that replaces it ("this campus still has 12 rooms") is more useful than the
deletion would have been.

Optional classification references are nulled. A room is an asset; which building it is in
and what kind of room it is are metadata, and both columns are already nullable.

`resource.campus_id` is RESTRICT despite being optional, so "delete a campus" means the
same thing on every edge rather than mostly-refuse-but-sometimes-detach.

### Repositories clear optional references before deleting

`delete_room_type` and `delete_building` first clear the referring rooms through the normal
rev-bumping update, then delete. The schema's `SET NULL` remains as a backstop but does not
fire in normal operation.

This matters because a database-level `SET NULL` modifies a row **without incrementing
`rev`**, so the change-log entry would carry a stale revision — exactly the inconsistency
[D17](../OPEN-DECISIONS.md) exists to prevent.

### The driver stops at `evara-db`'s edge

`DbError::Open` now carries a file name and an opaque `InternalError`; `DbError::Sqlite`
is replaced by `DbError::Internal(InternalError)`. `InternalError` boxes
`dyn Error + Send + Sync`, so the source chain survives for logs while no `rusqlite` or
`refinery` type appears in a public signature. `InternalError::from_message` lets any crate
synthesise one in a test.

Constraint failures are translated into domain errors — `StillReferenced`,
`DuplicateValue`, `Invalid`, `NotFound` — because "the project database reported an error"
tells a user nothing about what is in the way.

## Consequences

- `evara-desktop` has **no** `rusqlite` dependency, normal or dev. That is the test of
  whether the boundary holds, and it is now enforced by the build rather than by review.
- Deleting a campus requires removing its buildings, rooms and resources first. That is
  more steps for a genuine restructure, and the right default for data a school cannot
  easily reconstruct. If it proves annoying, the answer is a deliberate "delete campus and
  everything on it" operation in the repository — explicit, previewable, and still one
  transaction — not a schema cascade.
- Foreign key columns are indexed, because every RESTRICT delete scans children to decide
  whether to refuse.
- The ambiguities found in `DATA-MODEL.md` §3 were resolved and written back into it; the
  table is in [OPEN-DECISIONS](../OPEN-DECISIONS.md) D20. Notably: `school` is a singleton,
  `building` is identified by name within its campus, and a room's building must be on the
  room's own campus — enforced by a trigger, because a composite foreign key would try to
  null the `NOT NULL` `campus_id` on `SET NULL`.
- No table carries `deleted_at` yet. Deletes are hard, guarded by RESTRICT, and remain
  attributable through `change_log`. Soft deletion is revisited when synchronisation is
  designed, which is the only thing that needs tombstones.

## Alternatives considered

**CASCADE for owned children.** Fewer steps when restructuring, and one mis-click destroys
a campus's rooms along with every timetable entry that will later reference them.

**Mixed by ownership strength** — CASCADE where the child is meaningless alone (a term
without its year), RESTRICT elsewhere. Defensible, and rejected because two rules are
harder to predict than one, and a term still represents real scheduling intent.

**Keeping `rusqlite::Error` in the public enum.** Honest about the source and forces the
driver onto every consumer. The source chain is preserved inside `InternalError` anyway.
