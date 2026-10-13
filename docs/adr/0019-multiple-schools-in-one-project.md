# ADR 0019 — A project may describe several schools

- Status: **Accepted**
- Date: 2026-10-13
- Supersedes: one claim in [ADR 0003](0003-sqlite-document-model.md) — *"Multiple schools
  means multiple files"*. Everything else in ADR 0003 stands.
- Related: [ADR 0020](0020-in-place-table-rebuild.md) for how the schema change was made
  safely; [ADR 0013](0013-buildings-shared-across-campuses.md) and
  [ADR 0014](0014-rooms-shared-across-campuses.md) for the sharing this depends on.

## Context

ADR 0003 settled that one SQLite file is one school project, and that a school needing
several institutions should use several files. `V3` enforced it:

```sql
only_one INTEGER NOT NULL DEFAULT 1 CHECK (only_one = 1) UNIQUE
```

That holds for a single school with several sites — which is why `campus` exists — and
breaks for the arrangement it did not anticipate: a trust, a group, or a school running a
separate junior programme. Those are several institutions that **share staff, rooms and
equipment**, each with its own academic year, year levels, cycle and bell schedule.

Split across files, the shared half becomes unrepresentable. Two project files cannot tell
each other that the same teacher is already teaching at ten o'clock, or that both have
booked the one sports hall. Modelling the shared teacher twice gives one person two
identities, two availability records and two independent timetables — the same correctness
failure ADR 0014 rejected for rooms, where it observed that two room rows have two
occupancy records and neither notices the clash.

A pre-implementation audit (`docs/multi-school-architecture-audit.md`) established two
things that made this cheap:

1. **Only one table enforced the restriction.** Every other uniqueness rule in the schema
   is already scoped by `school_id` — `UNIQUE (school_id, name)` on `year_level`,
   `(school_id, code)` on `campus`, and so on. Those become correct, not wrong, the moment
   a second school row exists.
2. **Facility sharing already spans schools.** `building_campus` and `room_campus`
   associate a facility with *campuses*, never with a school. Once campuses belong to
   different schools, a shared classroom needs no schema change at all.

## Decision

**A project may hold any number of schools.** `V12` rebuilds `school` without `only_one`.

### What a school owns, and what is shared

| Scope | Entities |
| --- | --- |
| **School-owned, never shared** | `campus`, `room_type`, `academic_year` → `term`, `year_level`, `cycle` → `cycle_day`, `period_structure` → `period`, `calendar_day`, `timeslot`, `student_group`, `school_logo` |
| **Shared across schools** | `building` and `room`, through the campus associations of ADR 0013/0014 |
| **One school, for now** | `teacher`, `student`, `resource`, `availability` — see *Not in this decision* |

A **year level is never shared**. Two schools may each hold their own "Grade 1" at their
own ordinal 1, as two records with two identifiers, and `UNIQUE (school_id, name)` keeps a
duplicate *within* one school refused. This was the requirement the change existed for and
it needed no schema work.

### No `project` table

Project identity already lives in `app_meta` (`project_id`, `created_by_site_id`, `V2` and
ADR 0008). A table would be a second source of truth for it. "Project-level" means *not
scoped by `school_id` in a query*, not *owned by a project row*.

### Identity, ownership and uniqueness

- A school keeps a stable UUIDv7 `id`, as every entity does. The school an existing project
  already had keeps the identifier it had.
- `UNIQUE (name)` on `school`. A name is what a selector shows, so two schools sharing one
  would make every choice ambiguous.
- `code` is **optional and unique when given**, through a partial index. This is ADR 0018
  §4's reading of CLAUDE.md's COALESCE rule applied again: a `COALESCE(code, '')` index
  would fold every un-coded school onto one key and permit exactly one of them.

### Reads are scoped; writes are checked

Every repository list of a school-owned entity takes an explicit `school_id`. There is no
`Structure::school()`: a `LIMIT 1` read would hand the interface an arbitrary institution
and silently scope everything downstream to it, which is the most damaging thing this
change could have left behind.

**Every write that names a record checks it belongs to the acting school**, inside the same
transaction as the write. An identifier stopped being sufficient authority the moment a
project held more than one school: a screen scoped to one of them must not be able to touch
another's rows because it was handed an identifier from it.

- `update_*` uses the `school_id` the caller's input already carries, so no signature
  changed to get the guard.
- `delete_*` takes the expected school as a parameter, because a delete has no input to
  carry one. The refusal is `DbError::WrongSchool`, and it is raised **before** anything is
  cleared: a check that ran after removing a teacher's availability rules would still read
  as a refusal while having already done the damage.
- A child whose school is reachable only through a parent is resolved through that chain —
  a term through its academic year, a period through its bell schedule, a cycle day through
  its cycle, a group membership through its group. Deliberately **not** solved by adding
  `school_id` columns to the children: a second place to record the same ownership is a
  second place for it to go wrong, and the chain is already enforced by foreign keys.

### Cross-school references are refused by the schema

`V12` adds trigger pairs refusing a student in another school's year level, a group with
another school's parent, a teacher based on another school's campus, a room typed by
another school's room type, a calendar day naming another school's term or cycle day, and
an availability rule naming another school's time. Triggers rather than repository checks,
for the reason `V3`, `V6` and `V9` all give: the rule is stated where the storage is, so a
future repository, the CLI, an import or a hand-repair cannot write a row that breaks it.
The repositories validate too, because a trigger cannot say *which field* is wrong.

`building_campus` and `room_campus` are **deliberately left unconstrained by school**. A
facility shared between campuses of different schools is the entire point of ADR 0013 and
ADR 0014, and `room-clash` remains a per-`room_id` rule across the whole project.

That makes a shared facility the one place where ownership is not sufficient authority to
*destroy* something, so three operations are kept distinct:

| Operation | Who may | Effect |
| --- | --- | --- |
| Withdraw one school's access | that school, or the home school | one association row |
| Remove the home campus | nobody | refused; a facility must never reach zero campuses |
| Destroy the facility | the home school, **and only while nobody else shares it** | the row and all its associations |

A hall created on Northgate's campus and shared with Southmoor's is a hall Southmoor is
timetabling into; Northgate deleting it would take Southmoor's room away without Southmoor
being asked. The refusal is `DbError::StillReferenced`, the error a blocked delete already
produces, so the way forward is to withdraw the other school's access first — an operation
that exists and that the other school can see.

### Deleting a school

Refused while it owns anything, with `Structure::school_dependents` reporting what is in
the way so the interface can explain it before offering the action — the pattern
`student_group_member_count` established. There is **no** "delete this school and
everything in it": every school-owned table keeps `ON DELETE RESTRICT` exactly as ADR 0010
set it, because a school is years of configuration and one mis-click must not take a
campus, a timetable grid and a staff list with it.

The one exception is the school's own **logo**, removed in the same transaction. A logo is
part of the school record rather than something the school owns, and leaving it to block
the delete would mean a school that had ever been given one could not be removed without
the user first guessing that a decoration was in the way.

### A safety copy, once

Because `V12` is the first migration to rebuild a table, the open path takes a verified
copy of the project before applying it, and **refuses the upgrade if the copy cannot be
made or verified**. `VACUUM INTO` rather than a file copy, because a project runs in WAL
mode and copying the main file alone yields a database missing every transaction still in
the log. Details in [ADR 0020](0020-in-place-table-rebuild.md).

## Consequences

- **Existing projects are unaffected in behaviour.** One school in, one school out; the
  interface shows no school selector until there is more than one to choose between.
- **Every list query and command gained a school parameter.** That was the bulk of the
  work — 119 commands and around twenty components — not the migration.
- **`campus` is still a physical site, not a school.** Nothing in this ADR makes a campus
  an institution, and a shared facility still reaches across schools by campus.
- **Two schools in one project may be in different timezones.** `school.timezone` was
  always per school; it now matters, because comparing two schools' wall-clock bell times
  is wrong. Nothing in Milestone 1 does that comparison yet, and the constraint on when it
  becomes possible is recorded in the audit rather than solved here.
- **Multiple *projects* remain the right answer for unrelated schools.** This is for
  institutions that share people and places. Two schools with nothing in common gain
  nothing from sharing a file.

## Not in this decision

Deliberately out of scope, and none of it is implied by anything above:

- **A teacher or student belonging to several schools.** Both still have exactly one
  `school_id`. Project-level identities with school membership and enrolment history —
  `teacher_school`, `teacher_campus`, `student_school_enrolment` — are the next stage, and
  the interface must not suggest they already exist.
- **Project-level availability.** A rule still names one school and one school's cycle, and
  existing rules keep that exact meaning. A date-and-time form for shared entities is a
  later, additive change.
- **Shared resources.** `resource` is still owned by one school; `resource_campus` is
  proposed but not built.
- **Cross-school scheduling.** Detecting and preventing a shared teacher or room being
  double-booked across schools needs dated occurrences rather than cycle positions, and is
  a soundness problem in its own right. Nothing here implements it, and nothing here
  claims it works.

## Alternatives considered

**Leave ADR 0003 alone; use one file per school.** The status quo, and correct for
unrelated schools. Rejected because it makes shared staff and shared rooms
unrepresentable, which is exactly what the arrangement needs.

**Reinterpret `campus` as the institution.** Free, and wrong. A campus is a physical site:
buildings, rooms and resources hang off it, and a trigger already asserts that a room's
building is on the room's campus. Overloading it would have made "two schools on one site"
and "one school on two sites" the same thing, and neither expressible.

**A `program` or `division` entity below the school.** Keeps ADR 0003 intact and adds a
third level to every ownership question. Rejected as premature: nothing asked for a
sub-division of a school that owns its own year levels, and a school-level answer can gain
one later without redesign.

**Add `school_id` columns and keep the singleton as a "project" row.** Avoids the table
rebuild. Rejected because every school-scoped uniqueness rule would then be project-wide —
`UNIQUE (school_id, name)` on `year_level` would still forbid two Grade 1s — so the
rebuild would have been needed on `year_level`, `student_group`, `campus` and more instead
of on `school` alone.
