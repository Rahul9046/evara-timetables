# ADR 0014 — Rooms shared across campuses

- Status: **Accepted** (approved 2026-10-11)
- Date: 2026-10-11
- Extends: [ADR 0013](0013-buildings-shared-across-campuses.md)
- Amends: [ADR 0010](0010-structure-deletion-semantics.md)
- Binds: the constraints phase — see *Scheduling identity* below

## Context

[ADR 0013](0013-buildings-shared-across-campuses.md) made a *building* shareable. A room
was still owned by exactly one campus:

```sql
campus_id TEXT NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
UNIQUE (campus_id, code)
```

Real schools break that for the same reason they broke it for buildings, and for one
worse reason. A sports hall on a shared site, a lecture theatre a sixth-form college and
a school both timetable into, a specialist lab two campuses book — these are **one
physical room** serving several campuses.

Modelling it as two rooms gives one space two identities. For a building that is an
inconvenience: two rows to rename, two sets of rooms to keep in step. For a room it is a
correctness failure, because a room is the thing lessons are *scheduled into*. Two room
rows have two independent occupancy records, so Campus A and Campus B can each book
"their" copy for the same timeslot, each see a conflict-free timetable, and discover on
the first day of term that one hall has two classes in it. No constraint can catch that,
because from the solver's point of view nothing is wrong.

So a room needs many campuses. The questions are what to do with `room.campus_id`, how
room sharing interacts with building sharing, and what this commits the future solver to.

## Decision

### `room_campus` is the association set; `room.campus_id` is the home campus

A new entity table holds the many-to-many edge:

```sql
CREATE TABLE room_campus (
    id         TEXT    PRIMARY KEY NOT NULL,
    room_id    TEXT    NOT NULL REFERENCES room (id)   ON DELETE RESTRICT,
    campus_id  TEXT    NOT NULL REFERENCES campus (id) ON DELETE RESTRICT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    rev        INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (room_id, campus_id)
) STRICT;
```

Same shape and same reasoning as `building_campus`: a **full entity table, not a bare
junction**. Own UUIDv7 `id`, own audit columns, own change-log triggers, own entry in
`change_log::TRACKED_TABLES`. The change-log triggers read `NEW.id` and `NEW.rev`, and
`repo::update` / `repo::delete` address rows by `id`; a composite-key junction would sit
outside both, which means outside the change feed.

**`room_campus` is the complete association set.** Every campus a room serves has a row,
including the campus it was created under. Nothing queries `room.campus_id` to answer
"which campuses?" — `rooms(campus_id)` joins through this table only.

**`room.campus_id` survives as the *home* campus**, and the reason is ADR 0013's,
unchanged: V3 is shipped and immutable, refinery runs all pending migrations inside one
transaction, `PRAGMA foreign_keys` is a **no-op inside a transaction**, so the documented
SQLite table-rebuild procedure is unavailable. `DROP COLUMN` cannot touch `campus_id`
either — SQLite refuses a column used in an index, a `UNIQUE` constraint or a trigger, and
`campus_id` appears in all three. It keeps earning its place beyond being undroppable: it
scopes `UNIQUE (campus_id, code)`, and it guarantees a room can never reach zero campuses
and become unreachable from every screen.

So the shape is **one home campus, plus any number of shared campuses**, matching
buildings exactly. A developer who has read ADR 0013 already knows how this works.

### Buildings and rooms are independent, and meet at exactly one rule

A shared building does **not** share the rooms inside it, and a shared room does **not**
share its building. Both are deliberate. A trust may share a sports centre between two
campuses while each keeps its own offices inside it; a hall may be shared between campuses
while belonging to a building only one of them administers — as long as that building
reaches both.

The two settings meet at one rule, and it is the only coupling in the design:

> **A room may serve a campus only if its building — when it has one — is available to
> that campus too.**

Rooms with no building are unaffected; every check is guarded on `building_id` being
non-null, and a room with no building shares freely. A room with no building stays a
first-class, supported state, as ADR 0010 already established.

That rule has three routes into it, and all three are closed:

| Route | Enforced by |
| --- | --- |
| Sharing a room to a campus its building does not reach | `room_campus` insert/update trigger, plus a repository pre-check that names the building and the campus |
| Moving a room into a building that does not reach every campus the room serves | the replaced `room_building_must_share_campus_update` trigger, plus the same pre-check |
| Withdrawing a building share that a shared room depends on | `unshare_building`, whose room count now joins through `room_campus` |

V5's two room triggers compared the building against `room.campus_id` alone. V6 replaces
them: the rule is unchanged, its scope widens from the home campus to every campus the
room serves. The abort message is unchanged on purpose, so `structure::room_error` keeps
translating it into a typed error.

The third route is the subtle one. Before V6, `unshare_building` counted rooms by
`room.campus_id`. A room homed on North and *shared* to South says nothing about South in
that column, so the old count would have reported zero and allowed the building to be
withdrawn from South while South was still listing rooms inside it. The count now joins
through `room_campus`, which contains the home campus too and therefore subsumes the old
check rather than sitting beside it.

### Pre-checks in the repository, triggers as the backstop

The triggers raise one fixed message for every route, because a `RAISE(ABORT)` string
cannot be composed. The repository checks the same rule first and produces something a
user can act on — naming the room, the building and the campus, and saying what to do:

> Sports Hall is in Science Block, which South does not have. Share Science Block with
> South first, or clear the room's building.

This is ADR 0010's treatment of `ON DELETE SET NULL` applied to a rule rather than an
edge: the repository does the work properly, the schema guarantees it was done. Neither is
redundant. Dropping the trigger would make the invariant a convention of one code path;
dropping the pre-check would make every refusal unactionable.

### Deletion stays RESTRICT everywhere, and the repository clears first

Continuing ADR 0010 without exception:

| Edge | Behaviour | Why |
| --- | --- | --- |
| `room_campus.room_id` | `RESTRICT` | `delete_room` removes the association rows itself, through `repo::delete`, so each removal is a truthful change-log entry. A `CASCADE` would delete them without firing the change-log trigger — SQLite does not fire triggers for foreign-key actions unless `recursive_triggers` is on — leaving the feed claiming associations that no longer exist. |
| `room_campus.campus_id` | `RESTRICT` | Deleting a campus that a room is shared with is refused until the share is removed, so "delete a campus" means the same thing on every edge. |

No cascade is introduced anywhere, so no deletion path can reach a shared room by
accident. Nothing is silently moved, duplicated or deleted on any path: deleting a
*building* still detaches its rooms through a real update (their `rev` advances) and the
rooms keep every campus they serve; withdrawing a *room* share removes one association and
touches nothing else — not even the room's own `rev`, which would be a lie, since the room
did not change.

### One rule lives in the repository rather than the schema

**The home campus cannot be withdrawn.** As with buildings, a trigger could express it,
but the trigger would then also have to let `delete_room` through, and the only way to
distinguish "the room is going away" from "someone is withdrawing the home campus" inside
a `BEFORE DELETE` trigger is a flag column or a temp table. `unshare_room` refusing with
`Invalid` is simpler and the message is better.

**A room's code must be free on every campus it is shared to**, for the same reason a
building's name must be: `UNIQUE (campus_id, code)` only scopes the home campus, and the
cross-campus case needs the other rows — which by the rule in `validation.ts` makes it a
repository rule in Rust, never a frontend check.

### Scheduling identity — the invariant this ADR exists to protect

**A shared room is one room. Occupancy is a property of `room.id`, never of
`(room_id, campus_id)`.**

This is the whole reason the feature is modelled as an association table instead of a
second room row, and it is a **binding constraint on the constraints phase**:

- `room-clash` (`CORE-003`, see [CONSTRAINTS.md](../CONSTRAINTS.md)) **must** be evaluated
  per `room_id` across the entire project. Not per campus, not per campus-pair, not with a
  campus term in the key. A shared room booked by Campus A at a given time is booked, full
  stop.
- The same applies to every other room-scoped rule — `room-capacity`,
  `room-type-required`, `entity-unavailable` for a room, `activity-fixed` — because they
  all key on the room, and the room is one thing.
- Nothing in this phase implements any of that, and nothing may. Per
  [CLAUDE.md](../../CLAUDE.md) and [ADR 0002](0002-rust-in-process-solver.md), the
  constraint evaluators live in `evara-constraints` and are shared by generation, scoring,
  manual-move validation and explanation. One implementation, four callers. A room-clash
  check written in TypeScript now — or in `evara-db` now — would be the second
  implementation that rule is allowed to have, which is zero.

What V6 *can* do, and does, is make the invariant structurally true and test it: there is
exactly one `room` row however many campuses list it, every campus reads the identical
record, and the room count does not grow when a room is shared.
`crates/evara-db/src/tests_shared_rooms.rs` asserts each of those directly, so the fact the
future rule depends on cannot drift before the rule exists.

## Consequences

- Existing room `id`s are untouched. V6 adds a table, backfills it from `room.campus_id`,
  and replaces two triggers while adding two. No row in `room` is rewritten, so no
  identity moves and no `rev` is consumed.
- The backfill mints UUIDv7 identifiers **in SQL**, as V5's did, because refinery
  migrations are `.sql` files and there is no `uuid()` in SQLite. A test asserts
  `get_version_num() == 7` on every backfilled row.
- `setup_room_list` now returns `RoomWithCampuses` rather than `Room`. The same deliberate
  break ADR 0013 took for buildings, for the same reason: the alternative is the rooms
  table issuing one extra call per row to discover whether it is shared.
- `setup_room_create` takes `sharedCampusIds` alongside `input`. The share set is not a
  field of `RoomInput` because the associations are their own table and `RoomInput` is
  generated from the Rust record — inventing a field for it would be the parallel DTO
  layer [ADR 0012](0012-records-are-the-ipc-contract.md) forbids.
- `delete_campus` gains a reason to be refused that users will meet: a room shared *to*
  this campus.
- Editing a room is up to three calls — withdraw, update, share — in that order. The order
  is load-bearing: moving a room into a campus-local building *and* dropping the campus
  that cannot reach it is one edit to the user, and it only succeeds if the withdrawal
  lands before the building changes. A refusal part-way leaves earlier steps applied, which
  is why the panel re-reads the list after every mutation rather than patching it.
- `unshare_room` has nothing to block it today beyond the home-campus rule. When
  activities arrive it will: an activity scheduled into this room for a group on that
  campus is exactly the dependency that must refuse a withdrawal, the way rooms refuse
  `unshare_building`. Recorded here rather than guessed at now, because the shape of that
  reference belongs to a later phase.

## Deferred, deliberately: cross-campus time alignment

A shared room is one room, so its occupancy is global. But "the same time" is not yet a
well-defined question across campuses, and this ADR does **not** answer it.

Campuses may run different bell schedules. `period_structure` is school-wide, and nothing
in the model ties a campus to one, so two campuses can timetable against different period
structures whose periods carry the same ordinals and different wall-clock times. Period 3
on North and Period 3 on South are **not** necessarily the same minutes, and
[ADR 0011](0011-stable-timeslot-identity.md) is explicit that a timeslot's identity is
`(term_id, cycle_day_id, period_id)` with no campus in it.

So `room-clash` cannot be implemented as "same timeslot id" for a shared room without
first deciding how cross-campus time is compared. Inventing that rule here would be
inventing a scheduling rule in the wrong phase, in the wrong crate, on the strength of a
guess. The decision is recorded as **D22** in
[OPEN-DECISIONS.md](../OPEN-DECISIONS.md) and must be settled before `room-clash` is
implemented for shared rooms.

Naming it is not an idle precaution: the obvious implementation — compare `timeslot_id` —
is wrong exactly when the feature matters, and it would be wrong silently.

## Alternatives considered

**Rebuild `room` without `campus_id`, so the join table is fully symmetric.** The honest
model, and unreachable for ADR 0013's reasons: it needs foreign keys off inside a
transaction that cannot turn them off. Revisiting it means a non-transactional migration
path, a much larger decision than this feature deserves.

**Keep `room.campus_id` as the only home marker and store *additional* campuses in the
join table**, so associations are `{campus_id} ∪ room_campus`. Rejected for ADR 0013's
reason: every query about campuses becomes a `UNION`, and a call site that forgets one
half silently fails to show shared rooms. Backfilling the home row costs one
`INSERT … SELECT` and buys a single source of truth.

**Duplicate the room per campus and link the copies.** Rejected outright. This is the
problem being solved: two occupancy records for one physical space, and a double-booking
no constraint can see.

**Derive room sharing from building sharing** — a room in a shared building is
automatically shared. Rejected: it is wrong in both directions. A shared sports centre
does not mean each campus may book the other's offices inside it, and a hall two campuses
share may well sit in a building only one of them administers. It would also leave rooms
with no building — a fully supported state — with no way to be shared at all.

**Let a shared room's attributes vary by campus** (a different capacity or bookability per
campus). Rejected: `capacity` and `is_bookable` are properties of the physical space. A
campus that needs a different answer needs a different room, which is the case V6 exists
to stop people faking.

**Enforce the building-availability rule only in Rust, with no trigger.** Rejected for
ADR 0010's reason: it is the rule that makes "a shared building holds rooms belonging to
different campuses" safe rather than merely intended, and a rule that matters belongs in
the schema as well as the code path that normally maintains it.
