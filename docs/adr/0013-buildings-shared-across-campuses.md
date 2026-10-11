# ADR 0013 — Buildings shared across campuses

- Status: **Accepted** (approved 2026-10-10)
- Date: 2026-10-10
- Amends: [ADR 0010](0010-structure-deletion-semantics.md)

## Context

`building` was modelled in V3 as owned by exactly one campus:

```sql
campus_id TEXT NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
UNIQUE (campus_id, name)
```

Real schools break that. A sixth-form block on a shared site, a sports centre between two
campuses of the same trust, a science building that two campuses timetable into — these are
**one physical building** serving several campuses. Modelling it as two buildings means two
identities for one thing: two sets of rooms to keep in step, two rows to rename, and a
solver that cannot tell that a teacher moving between them does not actually move.

So a building needs many campuses. The question is what to do with `building.campus_id`,
because the answer is constrained by things that cannot be renegotiated:

1. **A released migration is immutable.** V3 ships; the fix is V5.
2. **refinery runs all pending migrations in one transaction**, and `PRAGMA foreign_keys`
   is a **no-op inside a transaction**. The documented SQLite procedure for rebuilding a
   table requires turning foreign keys off, so a rebuild of `building` is not available to
   us.
3. **`DROP COLUMN` cannot touch `campus_id`**: SQLite refuses when a column appears in an
   index, a `UNIQUE` constraint or a trigger, and `campus_id` appears in all three
   (`UNIQUE (campus_id, name)`, `idx_building_campus`, and both
   `room_building_must_share_campus_*` triggers).
4. **`DROP TABLE building` is actively dangerous.** With foreign keys on, `DROP TABLE`
   performs an implicit `DELETE` of every row, which fires `room.building_id`'s
   `ON DELETE SET NULL` — silently detaching every room in the school from its building
   on the way past.

## Decision

### `building_campus` is the association set; `building.campus_id` is the home campus

A new entity table holds the many-to-many edge:

```sql
CREATE TABLE building_campus (
    id          TEXT    PRIMARY KEY NOT NULL,
    building_id TEXT    NOT NULL REFERENCES building (id) ON DELETE RESTRICT,
    campus_id   TEXT    NOT NULL REFERENCES campus (id)   ON DELETE RESTRICT,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    rev         INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (building_id, campus_id)
) STRICT;
```

It is a **full entity table, not a bare junction**: its own UUIDv7 `id`, its own
`created_at` / `updated_at` / `rev`, its own change-log triggers, and a place in
`change_log::TRACKED_TABLES`. That is not ceremony. `change_log`'s triggers read
`NEW.id` and `NEW.rev`, and `repo::update` / `repo::delete` address rows by `id`; a
composite-key junction table would have been outside both, which means outside the change
feed — the exact silent hole that registry exists to prevent. Sharing a building is a real
edit to a real school and belongs in the feed like any other.

**`building_campus` is the complete association set.** Every campus a building serves has
a row, including the campus it was created under. Nothing queries
`building.campus_id` to answer "which campuses?" — there is one source of truth for that
question, and `buildings_for_campus` joins through this table only.

**`building.campus_id` survives as the *home* campus.** It is the campus the building was
created under, it is immutable, it always has a matching `building_campus` row, and it
cannot be unshared. It keeps earning its place for two reasons beyond being undroppable:
it scopes `UNIQUE (campus_id, name)`, and it guarantees a building can never reach zero
campuses and become unreachable from every screen in the application.

So the shape is: **one home campus, plus any number of shared campuses.** Adding and
removing shares is free; the home association is structural.

### The room/campus trigger now reads the association table

V3 enforced "a room's building must be on the room's campus" with a trigger comparing
`building.campus_id` to `room.campus_id`. Under sharing that is wrong — it would reject a
room on a campus the building genuinely serves. V5 drops both triggers and recreates them
against `building_campus`, with the abort message unchanged so the existing error
translation in `structure::translate_room` keeps matching it.

This is the integrity rule that actually matters for rooms, and it stays in the schema
rather than moving to Rust: it is what makes "a shared building contains rooms belonging to
different campuses" safe rather than merely intended.

### Deletion stays RESTRICT everywhere, and the repository clears first

Continuing ADR 0010 without exception:

| Edge | Behaviour | Why |
| --- | --- | --- |
| `building_campus.building_id` | `RESTRICT` | `delete_building` removes the association rows itself, through `repo::delete`, so each removal is a truthful change-log entry. A `CASCADE` would delete them without firing the change-log trigger — SQLite does not fire triggers for foreign-key actions unless `recursive_triggers` is on — leaving the feed claiming associations that no longer exist. |
| `building_campus.campus_id` | `RESTRICT` | Deleting a campus that a building is shared with is refused until the share is removed, so "delete a campus" means the same thing on every edge. |

No cascade is introduced anywhere, so no deletion path can reach a shared building by
accident.

### Two rules live in the repository rather than the schema

- **The home campus cannot be unshared.** A trigger could express it, but the trigger would
  then also have to let `delete_building` through, and the only way to distinguish "the
  building is going away" from "someone is unsharing the home campus" inside a `BEFORE
  DELETE` trigger is a flag column or a temp table. `unshare_building` refusing with
  `Invalid` is simpler and the message is better.
- **A building's name must be free on every campus it is shared to.** `UNIQUE (campus_id,
  name)` only scopes the home campus; the cross-campus case needs the other rows, which by
  the rule in `validation.ts` makes it a repository rule in Rust, never a frontend check.

## Consequences

- Existing building and room `id`s are untouched. V5 adds a table, backfills it from
  `building.campus_id`, and replaces two triggers. No row in `building` or `room` is
  rewritten, so no identity moves and no `rev` is consumed.
- The backfill mints UUIDv7 identifiers **in SQL**, because refinery migrations are `.sql`
  files and there is no `uuid()` in SQLite. The expression lays out the v7 fields
  explicitly — 48-bit millisecond timestamp, version nibble `7`, two-bit variant — and a
  test asserts `get_version_num() == 7` on every backfilled row rather than trusting the
  printf.
- `setup_building_list` now returns `BuildingWithCampuses` rather than `Building`. That is
  a breaking change to one IPC shape, taken deliberately: the alternative is the building
  table issuing one extra call per row to discover whether it is shared.
- `delete_campus` gains a reason to be refused that users will meet: a building shared *to*
  this campus. The message names it.
- A building shared to four campuses appears in four campus listings, and is the same row
  with the same `id` in all four. Renaming it from any of them renames it everywhere, which
  is the point.

## Alternatives considered

**Rebuild `building` with no `campus_id`, so the join table is fully symmetric.** The
honest model, and unreachable: see constraints 2–4 above. It needs foreign keys off inside
a transaction that cannot turn them off, and the `DROP TABLE` step would detach every room
in the school. Revisiting it would mean a non-transactional migration path, which is a much
larger decision than this feature deserves.

**Keep `building.campus_id` as the only home marker and store *additional* campuses in the
join table**, so associations are `{campus_id} ∪ building_campus`. Rejected: every query
about campuses becomes a `UNION`, and a call site that forgets one half silently fails to
show shared buildings. Backfilling the home row costs one `INSERT … SELECT` and buys a
single source of truth.

**A nullable `shared_from_building_id` on a second building row**, i.e. model the share as a
linked duplicate. Rejected outright — it is two identities for one physical thing, which is
the problem being solved.

**`CASCADE` on `building_campus.building_id`.** Fewer lines in `delete_building`, at the
cost of association deletions that never reach the change feed. ADR 0010 already settled
that repositories clear references explicitly so the feed stays truthful; this is the same
rule applied to a new edge.
