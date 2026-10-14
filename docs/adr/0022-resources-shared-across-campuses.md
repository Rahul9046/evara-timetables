# ADR 0022 — A resource may serve many campuses, and so many schools

- Status: **Accepted**
- Date: 2026-10-14
- Extends: [ADR 0013](0013-buildings-shared-across-campuses.md) and
  [ADR 0014](0014-rooms-shared-across-campuses.md), for the third kind of thing a school
  lends. The reasoning is theirs, applied once more.
- Completes: the *"Shared resources"* item under *Not in this decision* in
  [ADR 0019](0019-multiple-schools-in-one-project.md).
- Related: [ADR 0021](0021-project-level-people-and-school-membership.md), the people half
  of the same stage.

## Context

ADR 0019 listed what `V12` had left for later, and `resource` was on it: *"`resource` is
still owned by one school; `resource_campus` is proposed but not built."*

`V3` modelled a resource as owned by a school with one optional campus. That is right for a
school with several sites and wrong for the arrangement multi-school projects exist for: a
projector trolley, a minibus, a set of class instruments shared between two institutions on
one site could not be expressed at all. Modelling it as two resources gives one physical
object two identities and two independent availability records — the failure ADR 0014
rejected for rooms, where it observed that two room rows have two occupancy records and
neither notices the clash.

ADR 0019's audit had already established that the facility half of sharing needed no schema
work: `building_campus` and `room_campus` associate a facility with **campuses** and never
mention a school, so the moment campuses belong to different schools the existing tables
express cross-school sharing. Resources had no such table.

## Decision

**`V13` adds `resource_campus`, mirroring `room_campus` exactly**, and narrows
`resource.campus_id` to mean the home campus.

```sql
CREATE TABLE resource_campus (
    id          TEXT    PRIMARY KEY NOT NULL,
    resource_id TEXT    NOT NULL REFERENCES resource (id) ON DELETE RESTRICT,
    campus_id   TEXT    NOT NULL REFERENCES campus (id)   ON DELETE RESTRICT,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL,
    rev         INTEGER NOT NULL DEFAULT 1 CHECK (rev >= 1),
    UNIQUE (resource_id, campus_id)
) STRICT;
```

A first-class entity, not a bare junction, for the reason `V5`, `V6` and `V10` all give: the
change-log triggers read `NEW.id` and `NEW.rev`, and `repo::update`/`repo::delete` address
rows by `id`, so a composite-key table would sit outside the change feed.

`RESTRICT` on both edges, continuing [ADR 0010](0010-structure-deletion-semantics.md)
without exception. `delete_resource` removes these rows itself through `repo::delete` so
each is a truthful change-log entry; a `CASCADE` would remove them without firing the
trigger, because SQLite does not fire triggers for foreign-key actions unless
`recursive_triggers` is on.

### A shared resource is ONE resource

`resource.id` is its scheduling identity, and availability is a property of that identifier,
not of `(resource, campus)`. This is ADR 0014's binding invariant restated for the third
time, and it is the whole point: any future resource-clash rule **must be evaluated per
`resource_id` across the whole project, never per campus and never per school**. Nothing
enforces it here because nothing is scheduled yet, and recording it is what stops the
constraints phase getting it wrong.

`Structure::resources(school_id)` therefore returns resources the school **owns or has been
lent**, `DISTINCT`, so one minibus appears in two schools' lists as the same row with the
same identifier.

### The one difference from `room_campus`

`room.campus_id` is `NOT NULL`; `resource.campus_id` is not. `V3` made it optional
deliberately — *"a resource may be school-wide or tied to one campus"* — so:

- **`resource_campus` may be empty for a resource.** That is not an error state, and the
  "never reach zero campuses" rule from ADR 0013/0014 applies only to a resource that has a
  home campus to lose. The backfill writes one row per resource that has a `campus_id` and
  nothing for a school-wide one, which is the same statement that resource was already
  making.
- A school-wide resource can still be lent: sharing is additive, and gains it a row.

### The home campus may move, and the loans do not

Unlike a room's, a resource's home campus has always been editable, and that stays true.
What `V13` adds is that the association follows: `update_resource` withdraws the old home's
row and writes the new one, in the same transaction, through `repo::delete` so both are
truthful change-log entries. Campuses the resource was *shared* with are untouched — another
school's access is not this school's to revoke by editing a field.

If the new home is a campus it was already shared with, the existing row is kept rather than
rewritten: it says the same thing either way, and rewriting it would consume a revision to
record nothing.

### Ownership is not authority to destroy

ADR 0019's three-way split, applied to resources:

| Operation | Who may | Effect |
| --- | --- | --- |
| Withdraw one school's access | that school, or the owner | one association row |
| Remove the home campus | nobody | refused; it would contradict `resource.campus_id` |
| Destroy the resource | the owner, **and only while nobody else shares it** | the row and all its associations |

A trolley Northgate bought and lent to Southmoor is a trolley Southmoor is timetabling
around; Northgate deleting it would take Southmoor's equipment away without Southmoor being
asked. The refusal is `DbError::StillReferenced`, the error a blocked delete already
produces, so the way forward is to withdraw the other school's access first — an operation
that exists and that the other school can see.

`share_resource` and `unshare_resource` both accept **either** authority, through the same
`require_may_unshare` the rooms use: the school that owns the resource ("we are no longer
lending it") or the school that owns the campus ("we no longer use this"). A third school
has no say in either.

### Deliberately not constrained by school

`resource_campus` carries no school check, exactly as `V12` left `building_campus` and
`room_campus`. A resource shared between campuses of different schools is the entire point.

`resource.school_id` stays, narrowed to the home school, and `V12`'s
`resource_references_own_school_*` trigger pair stays with it: the home campus still belongs
to the home school, so that rule is still true and still worth asserting. What it no longer
means is "the only school that may use this".

### Deleting a campus withdraws its loans

`delete_campus` now clears the `resource_campus` rows pointing at the campus, through
`repo::delete`. A resource whose *home* campus it is still blocks the delete, through
`resource.campus_id`'s `RESTRICT`. Withdrawing a loan to a campus that is being closed is
not a judgement call — the campus cannot host anything — and refusing instead would make a
campus undeletable until somebody guessed which other school's equipment was in the way.

## Consequences

- **Existing projects are unaffected in behaviour.** One association per resource that had
  a campus, carrying the resource's own `created_at`, no revision consumed, no row rewritten.
  A school-wide resource gets none and behaves exactly as it did.
- **No resource screen exists yet**, so this stage is repository and command surface only.
  The Resources section was removed from School Setup before this work and returns with the
  sharing controls in Stage 0C. That is why the backfill and the guards are tested directly
  rather than through a screen.
- **Three sharing mechanisms now share one shape.** Buildings, rooms and resources all
  associate with campuses, all reach across schools by campus, all refuse removal of the
  home, and all refuse destruction while another school shares. A fourth would copy the
  same file.
- **Cross-school resource clash detection is not implemented.** The invariant above says
  what it must do when it arrives; nothing here claims it works.

## Alternatives considered

**Associate a resource with schools rather than campuses.** Reads more directly as "two
schools share this". Rejected: it would make resources the only shareable thing in the model
that does not use the campus association, and a resource is physically *somewhere* — the
campus is what says where, and ADR 0013's trigger already reasons about facilities by
campus. Two mechanisms for one idea is one too many.

**Duplicate the resource per school.** Needs no migration. Rejected for the reason ADR 0014
rejected it for rooms: two identities, two availability records, and neither notices the
clash.

**Drop `resource.campus_id` in favour of the association table alone.** Cleaner on paper.
Rejected, and for a sharper reason than ADR 0014 had: `resource.campus_id` is nullable, so
there is no "every resource has at least one campus" invariant to lean on, and the column is
the only thing distinguishing "based here, shared there" from an unordered set of campuses.
Dropping it would also require a table rebuild on a table nothing else needed changing.

**Require at least one campus, as rooms do.** Symmetric with `room_campus`. Rejected: it
would break every existing school-wide resource and force a school to invent a site for a
minibus that belongs to the school rather than to a building.
