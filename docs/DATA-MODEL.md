# Data Model

Canonical store: **SQLite**, one file per school project. This document is a design
proposal; the authoritative schema will be the migration files under
`crates/evara-db/migrations/`.

---

## 1. Storage decisions

### One file = one project ("document model")

A project is a single **`.evaraproj`** SQLite file the user opens like a document.
Create / Open / Recent Projects, and "export" is a transformation of one file rather
than a query across a multi-tenant database.

`.evaraproj` is the live, editable database; **`.evara` is reserved for the portable
export package** ([PROJECT-FORMAT.md](PROJECT-FORMAT.md)). The two are never
interchangeable — one is a working database with WAL sidecar files, the other a
self-contained archive — and the lifecycle rejects the wrong extension rather than
guessing.

The app keeps a tiny separate settings database (`%APPDATA%` /
`~/Library/Application Support`) for the installation identity and the recent-project
list — never school data.

A project file still contains a `school` row (and multiple `campus` rows), so multi-campus
works inside one project. Multiple *schools* means multiple project files.

**Amended in V12.** A project may now hold **several `school` rows**, each owning its own
campuses, academic years, year levels, cycles and bell schedules, while buildings and rooms
are shared between them through the campus associations of ADR 0013/0014.
[adr/0019](adr/0019-multiple-schools-in-one-project.md) supersedes ADR 0003's "multiple
schools means multiple files" and records what is school-owned, what is shared, and what
deliberately stayed single-school. One file is still one project; it is no longer one
school. The rebuild that made room for it is
[adr/0020](adr/0020-in-place-table-rebuild.md).

`V13` closes two of the three things ADR 0019 deferred.
[adr/0021](adr/0021-project-level-people-and-school-membership.md) makes a `teacher` and a
`student` **project-level identities** with school membership, campus eligibility and
enrolment history; [adr/0022](adr/0022-resources-shared-across-campuses.md) makes a
`resource` shareable by campus the way a building and a room already were. An
`availability` rule still names exactly one school, and project-level availability remains a
later, additive change.

### Connection pragmas

```sql
PRAGMA journal_mode = WAL;        -- crash-safe, concurrent readers
PRAGMA foreign_keys = ON;         -- per connection, must be set every time
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;      -- WAL + NORMAL is durable enough for a desktop doc
```

Foreign keys are **off by default in SQLite** and are per-connection — enforced in the
connection factory in `evara-db`, covered by a test.

### Conventions on every entity table

| Column | Type | Notes |
| --- | --- | --- |
| `id` | `TEXT PRIMARY KEY` | UUIDv7, lowercase hyphenated. Time-ordered so index locality is good and insertion order is recoverable. Stable across export/import. |
| `created_at` | `TEXT NOT NULL` | ISO-8601 UTC, millisecond precision. Sorts lexicographically. |
| `updated_at` | `TEXT NOT NULL` | Written by the repository on every update, not by a trigger — see [OPEN-DECISIONS](OPEN-DECISIONS.md) D17. |
| `rev` | `INTEGER NOT NULL DEFAULT 1` | Incremented **by the repository**, never by a trigger — see [OPEN-DECISIONS](OPEN-DECISIONS.md) D17. **Checked on write since `V14`'s stage** — see below. Merge input later. |
| `deleted_at` | `TEXT NULL` | Soft delete, only on tables a future sync must tombstone. |

Natural keys (`code`) are unique *within* their parent, never the primary key — schools
rename and reuse codes.

#### `rev` is a precondition, not only a record

Since Stage B every update and delete the interface can reach puts the caller's revision in
the statement:

```sql
UPDATE room SET ..., updated_at = ?, rev = rev + 1 WHERE id = ? AND rev = ?
DELETE FROM room                                   WHERE id = ? AND rev = ?
```

A row that has moved on matches nothing, so nothing is written — no column changes, no
revision is consumed, and no change-log trigger fires. The failure is reported as
`DbError::Conflict`, deliberately distinct from `NotFound`: the record is still there, and
the next step is to re-read it rather than to give up.

`repo::update` and `repo::delete` own the clause, as they already own `rev = rev + 1`, so a
repository cannot write the domain columns without it.
[adr/0027](adr/0027-revisions-are-checked-on-write.md) records the full inventory —
which categories of mutation carry a revision, which carry something stronger, and the four
where last-write-wins is the deliberate answer. It also notes the one consequence for this
table: because `rev` now has two readers, D17's "no self-updating triggers" rule is
load-bearing rather than tidy. A trigger that bumped `rev` would make every second write in
a transaction conflict with itself.

### Sync-readiness

Every mutable entity table gets `AFTER INSERT/UPDATE/DELETE` triggers writing to:

```sql
CREATE TABLE change_log (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  site_id      TEXT NOT NULL,          -- the installation writing, via evara_site_id()
  entity_table TEXT NOT NULL,
  entity_id    TEXT NOT NULL,
  op           TEXT NOT NULL CHECK (op IN ('INSERT','UPDATE','DELETE')),
  rev          INTEGER NOT NULL,
  at           TEXT NOT NULL
);
```

Triggers rather than repository code, so coverage is guaranteed by the schema instead of
by developer discipline. This is the *only* concession made to future synchronisation —
no sync protocol, no vector clocks, no network code now.

Implemented in Phase 1A, corrected in Phase 1B. `change_log` and the trigger generator
(`evara_db::change_log::TrackedTable` / `install`) exist and are tested; a migration
that adds an entity table calls `install` for it.

Triggers obtain the writing installation from **`evara_site_id()`**, an
application-defined SQL function registered per connection — not from the project file.
A connection without it cannot write to a tracked table, so there is no such thing as
an unattributed change. See
[ADR 0008](adr/0008-installation-identity-and-project-lifecycle.md).

The triggers only *read* `rev`; they never maintain `rev` or `updated_at`, because a
trigger that updates its own table interacts badly with SQLite's `recursive_triggers`
setting. Repositories write both explicitly.

### Migrations

Numbered, embedded, forward-only SQL files applied inside a single transaction. Opening a
project whose schema version is **newer** than the binary fails with a clear typed error
(`DbError::SchemaTooNew`), never a partial read and never an automatic downgrade.

Implemented in Phase 1A with `refinery` ([ADR 0007](adr/0007-rusqlite-refinery.md)). The
applied set is recorded in refinery's `refinery_schema_history` table and mirrored into
`app_meta.schema_version` for diagnostics. All pending migrations run as **one group in
one transaction**, so a failure leaves the project untouched rather than half-migrated.

> **A released migration is immutable.** refinery checksums applied migrations, so editing
> a shipped file makes every existing project refuse to open. Corrections ship as a new
> numbered migration.

The set as shipped:

| | What it adds | Rebuilds a table | Backfills |
| --- | --- | --- | --- |
| `V1`–`V2` | bootstrap, installation identity | no | — |
| `V3`–`V4` | Structure: the static half, then the time model | no | — |
| `V5`–`V6` | `building_campus`, `room_campus` | no | yes, from the home campus |
| `V7`–`V9` | school working days, school logo, cycle kind | no | yes |
| `V10`–`V11` | the People group, then `availability` | no | — |
| `V12` | several schools per project | **yes**, `school` | — |
| `V13` | shared identities: membership, eligibility, enrolment history, `resource_campus` | no | yes, from what each row already said |
| `V14` | the Curriculum prerequisites: `department`, `subject`, `teacher_qualification`, `teacher_department` | no | **no** |
| `V15` | the Lessons group: `course`, `class_section`, `class_teacher`, `class_group`, `class_enrolment`, `activity` | no | **no** |

`V12` is the only migration that has ever rebuilt a table, and
`backup::FIRST_REBUILDING_MIGRATION` is set to 12 because of it: a project below that
version gets a **verified** safety copy before the upgrade proceeds, and the upgrade is
refused outright if the copy cannot be made or read back
([adr/0020](adr/0020-in-place-table-rebuild.md)). A project already at or past 12 needs no
copy, which is why `V13` and `V14` left the constant alone.

`V14` has **no backfill on purpose**: a project that has never been asked what it teaches
has no departments and no subjects, and inventing a "General" department would be putting
words in a school's mouth. `tests_curriculum::a_v13_project_upgrades_without_touching_a_single_row`
asserts that every row, revision and timestamp survives the upgrade unchanged.

`V15` has none either, and for the same reason one step further on: a subject with no course
is the ordinary state of a school part-way through setup, so one course per subject would be
putting words in its mouth *and* be wrong.
`tests_lessons::a_project_at_the_previous_schema_keeps_every_row_and_revision` and
`::the_migration_invents_no_lessons` assert both halves.

### Project metadata and installation identity

`app_meta` is a key/value table rather than `PRAGMA user_version`, because several
values are needed: `project_id`, `created_by_site_id`, `schema_version`,
`created_with_app_version`, `last_opened_with_app_version` and `created_at`.

Identity is split in three, and the split matters:

| Identifier | Lives in | Lifetime |
| --- | --- | --- |
| `project_id` | the project | created once, travels with the file |
| `created_by_site_id` | the project | historical, written at creation, never updated |
| `site_id` | the **settings database** | per installation, created on first run |

A project never records who is *currently* using it. Phase 1A did, which meant opening
a colleague's project adopted their identity; migration V2 corrects it.

### The settings database

A second SQLite database in the platform application data directory, with its own
migration set. Deliberately minimal:

```sql
app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)

recent_project (
  path_key       TEXT PRIMARY KEY,   -- normalised; lowercased on Windows
  path           TEXT NOT NULL,      -- original spelling, for display
  display_name   TEXT NOT NULL,
  project_id     TEXT,
  last_opened_at TEXT NOT NULL
)
```

It holds the installation `site_id` and the recent-project list, and never school data.
`path_key` is what deduplicates recents; `canonicalize` is deliberately not used because
it fails for a deleted file, which is exactly the case the list must survive.

---

## 2. Domain boundaries

Six bounded groups. The groups are the seams for modules, repositories and future
extraction; references run downward in this table only.

| Group | Owns | Depends on |
| --- | --- | --- |
| **Structure** | school, campus, building, building_campus, room, room_campus, room_type, resource, resource_campus, academic_year, term, cycle, cycle_day, period_structure, period, timeslot, calendar_day | — |
| **People** | teacher, teacher_school, teacher_campus, year_level, student, student_school_enrolment, student_group, student_group_member | Structure |
| **Curriculum** | department, subject, teacher_qualification, teacher_department, course, class_section, class_teacher, class_group, class_enrolment, course_request | Structure, People |
| **Activities** | activity, activity_teacher, activity_group, activity_room_candidate, activity_resource, line, line_member, activity_link, activity_link_member, availability | Structure, People, Curriculum |
| **Scheduling** | timetable, timetable_entry, timetable_evaluation, constraint_instance, constraint_ref, solve_run, solve_run_unscheduled, violation, violation_ref | all of the above |
| **Operations** | absence, coverage | Scheduling |

---

## 3. Structure — calendar and time

The time model deliberately avoids assuming Monday–Friday.

> **Implementation status.** The *static* half — school, campus, building, room_type,
> room, resource, academic_year, term — shipped in Phase 1C as migration `V3`. The *time*
> half — cycle, cycle_day, period_structure, period, timeslot and calendar_day — shipped
> in Phase 1D as migration `V4`, together with the timeslot materialiser. The whole
> Structure group now exists, and a test asserts that every *later* group's tables do not,
> so they cannot arrive by accident.
>
> `building_campus` arrived afterwards as migration `V5`, making a building shareable
> across campuses — see [adr/0013](adr/0013-buildings-shared-across-campuses.md) — and
> `room_campus` as migration `V6`, doing the same for rooms:
> [adr/0014](adr/0014-rooms-shared-across-campuses.md). `resource_campus` completes the set
> as migration `V13`: [adr/0022](adr/0022-resources-shared-across-campuses.md).
>
> All three associate a facility with **campuses** and never with a school, which is what
> makes cross-school sharing expressible without further schema work once campuses belong
> to different schools. A shared facility is **one** facility: its `id` is its scheduling
> identity, and occupancy is a property of that identifier, never of (facility, campus).

### Deletion semantics

Decided in Phase 1C and enforced by foreign keys — see
[adr/0010](adr/0010-structure-deletion-semantics.md):

| Edge | Behaviour |
| --- | --- |
| `campus.school_id`, `building.campus_id`, `room.campus_id`, `resource.school_id`, `resource.campus_id`, `academic_year.school_id`, `term.academic_year_id` | `RESTRICT` — a parent that still has children cannot be deleted |
| `room.building_id`, `room.room_type_id` | `SET NULL` — the room survives, unclassified |

Repositories clear the optional references with proper rev-bumping updates before
deleting a parent, so `SET NULL` is a backstop rather than the normal route.

`V5` added the sharing edge, decided in
[adr/0013](adr/0013-buildings-shared-across-campuses.md). Both halves are `RESTRICT`:

| Edge | Behaviour |
| --- | --- |
| `building_campus.building_id` | `RESTRICT` — `delete_building` removes the association rows itself, so each removal is a change-log entry. A `CASCADE` would remove them without firing the trigger |
| `building_campus.campus_id` | `RESTRICT` — a campus a building is shared *with* cannot be deleted until the share is withdrawn, so "delete a campus" means the same thing on every edge |

Withdrawing a share is **not** a deletion and detaches nothing: it is refused while rooms
on that campus are still in the building, and refused outright for the building's home
campus, so a building can never reach zero campuses.

`V6` added the room sharing edge, decided in
[adr/0014](adr/0014-rooms-shared-across-campuses.md). Both halves are `RESTRICT`, for the
reasons `building_campus` has them:

| Edge | Behaviour |
| --- | --- |
| `room_campus.room_id` | `RESTRICT` — `delete_room` removes the association rows itself, so each removal is a change-log entry |
| `room_campus.campus_id` | `RESTRICT` — a campus a room is shared *with* cannot be deleted until the share is withdrawn |

Withdrawing a room share removes one association and nothing else — not even the room's
own `rev`, since the room did not change. It is refused outright for the room's home
campus, so a room can never reach zero campuses. Deleting a *building* still detaches its
rooms through a real update; the rooms keep every campus they serve.

Phase 1D added the time edges, decided in
[adr/0011](adr/0011-stable-timeslot-identity.md). **Every one of them is `RESTRICT`**,
including the nullable ones:

| Edge | Why RESTRICT rather than SET NULL |
| --- | --- |
| `cycle.school_id`, `cycle_day.cycle_id`, `period_structure.school_id`, `period.period_structure_id`, `calendar_day.school_id`, `timeslot.school_id` | ownership, as everywhere else |
| `period_structure.campus_id`, `calendar_day.campus_id`, `calendar_day.term_id`, `timeslot.campus_id` | so that "delete a campus" and "delete a term" mean the same thing on every edge, as `resource.campus_id` already does |
| `timeslot.cycle_id`, `timeslot.cycle_day_id`, `timeslot.period_id` | a materialised grid *blocks* removing the day or period it came from. Removing either is a deliberate, confirmed operation, never a side effect |
| `timeslot.term_id` | `term_id` is part of the logical identity, so nulling it would silently rewrite **which slot a row is** — and could collide with an existing slot |
| `calendar_day.cycle_day_id` | `NULL` means "no lessons that day", so `SET NULL` would quietly reinterpret mapped teaching dates as non-teaching ones. A change of meaning, not a detached reference |

A timeslot is never cascade-deleted, by design. Since `timetable_entry` arrived in `V17` its
reference to `timeslot` will be `RESTRICT` too, so a deletion is refused rather than
silently taking placements with it.

### Uniqueness, as implemented

Following the documented rule that codes are unique within their parent:
`campus(school_id, code)` · `room_type(school_id, code)` · `room(campus_id, code)` ·
`resource(school_id, code)` · `building(campus_id, name)` ·
`academic_year(school_id, name)` · `term(academic_year_id, ordinal)` and
`term(academic_year_id, name)`.

Plus `building_campus(building_id, campus_id)` and `room_campus(room_id, campus_id)`,
which are what stop a campus being associated with the same building, or the same room,
twice. All four columns are `NOT NULL`, so those constraints need no `COALESCE` to
actually hold.

`school` is a singleton, enforced by a constant column with a unique index: a project
describes one school ([ADR 0003](adr/0003-sqlite-document-model.md)). A room's building
must be **available to** every campus the room serves, enforced by triggers that read
`building_campus` — which is what lets one shared building hold rooms belonging to
different campuses, and what stops a room being shared to a campus its building does not
reach. Sharing a building does not share its rooms, and sharing a room does not share its
building; that rule is the one place the two settings meet
([ADR 0014](adr/0014-rooms-shared-across-campuses.md)).

`building(campus_id, name)` scopes a name to a building's *home* campus only, and
`room(campus_id, code)` does the same for a room's code. The cross-campus case — sharing
to a campus that already has one of that name or code — needs the other rows, so both are
repository rules in `evara-db` rather than schema constraints.

```sql
-- working_days is a seven-bit set: bit 0 Monday through bit 6 Sunday, so 31 is
-- Monday-to-Friday and CHECK (BETWEEN 1 AND 127) is what makes "at least one working
-- day" a schema invariant. A CALENDAR default and an advisory only: nothing derives a
-- cycle day from it, nothing infers it from cycle.day_count, and an explicit calendar_day
-- row always outranks it. Carried over IPC as [1,2,3,4,5]. ADR 0015.
school (id, name, timezone, locale, working_days INTEGER DEFAULT 31,
        created_at, updated_at, rev)

-- A school's own logo, or no row at all when it uses Evara's. The image is a BLOB in
-- the project so it travels with the file; no absolute path to an external image is
-- ever stored. Format and dimensions are read from the bytes' own header by
-- evara_core::branding, never from a filename. ADR 0016.
school_logo (id, school_id -> school, media_type, bytes BLOB, width, height,
             UNIQUE(school_id))

campus (id, school_id -> school, name, code, address, UNIQUE(school_id, code))
campus_travel (from_campus_id, to_campus_id, minutes, PRIMARY KEY(from, to))
-- campus_id is the HOME campus: the one the building was created on. Immutable, always
-- present in building_campus, and never withdrawable. It scopes UNIQUE(campus_id, name)
-- and guarantees a building always has at least one campus. ADR 0013 records why it was
-- not dropped in favour of a fully symmetric join table.
building (id, campus_id -> campus, name, UNIQUE(campus_id, name))

-- The COMPLETE set of campuses a building serves, home campus included. Nothing reads
-- building.campus_id to answer "which campuses?". A full entity table rather than a bare
-- junction, because the change-log triggers read NEW.id and NEW.rev.
building_campus (id, building_id -> building, campus_id -> campus,
                 UNIQUE(building_id, campus_id))
room_type (id, school_id, name, code)                       -- "Science Lab", "Gym"
-- campus_id is the HOME campus, exactly as building.campus_id is: immutable, always
-- present in room_campus, never withdrawable. It scopes UNIQUE(campus_id, code) and
-- guarantees a room always has at least one campus. ADR 0014 records why it stayed.
room (id, campus_id -> campus, building_id -> building NULL, name, code,
      capacity INTEGER, room_type_id -> room_type NULL, is_bookable, notes,
      UNIQUE(campus_id, code))

-- The COMPLETE set of campuses a room serves, home campus included. A SHARED ROOM IS ONE
-- ROOM: room.id is its scheduling identity, so occupancy is a property of that identifier
-- and never of (room_id, campus_id). room-clash (CORE-003) must therefore be evaluated per
-- room_id across the whole project. See ADR 0014 — and D22, which must be settled first,
-- because two campuses can run different bell schedules and matching period ordinals do
-- not imply overlapping real times.
room_campus (id, room_id -> room, campus_id -> campus, UNIQUE(room_id, campus_id))
-- school_id is the HOME school and campus_id the HOME campus, both narrowed by V13.
-- resource_campus is the complete set of campuses it serves, and may be EMPTY: unlike a
-- room, a resource may be school-wide. ADR 0022.
resource (id, school_id, name, code, quantity INTEGER, campus_id NULL)
resource_campus (id, resource_id -> resource, campus_id -> campus,
                 UNIQUE(resource_id, campus_id))

academic_year (id, school_id, name, starts_on DATE, ends_on DATE)
term (id, academic_year_id -> academic_year, name, ordinal, starts_on, ends_on)
```

### Cycle, period structure, timeslot

```sql
-- A cycle is the repeating pattern. day_count = 7 for a week; 10 for a two-week cycle;
-- 6 for a six-day rotation that drifts against the calendar week. No upper bound.
-- day_count is the *declared* length: the materialiser uses the cycle_day rows that
-- actually exist, and the repository refuses an ordinal beyond it (ADR 0011 Q4).
--
-- kind decides who owns the positions (ADR 0017). WEEKLY means the week itself: seven
-- positions, position N being ISO weekday N, written by create_cycle from
-- evara_core::calendar::WeeklyPosition and not user-managed. CUSTOM means a numbered
-- rotation whose length and labels the user chooses. Creating a cycle of either kind lays
-- out its positions in the same transaction. The kind is immutable once set.
--
-- All seven, never the five a school happens to work. A cycle is a reusable definition and
-- holds NO school's calendar: no start date, no starting rotation position, no working
-- week, no holiday policy. Which of a weekly cycle's positions a given school teaches on is
-- asked (TimeModel::cycle_shape, with a WorkingDays argument) and never stored.
cycle (id, school_id, name, day_count INTEGER, week_count INTEGER DEFAULT 1, is_default,
       kind TEXT CHECK (kind IN ('WEEKLY','CUSTOM')) DEFAULT 'CUSTOM',  -- V9
       -- week_count and is_default are no longer collected by the interface (ADR 0015,
       -- ADR 0017 consequences). The columns stay; existing values are preserved on edit.
       UNIQUE(school_id, name))
cycle_day (id, cycle_id -> cycle, ordinal INTEGER, label,
           weekday_hint INTEGER NULL,          -- 1..7, display only, never logic
           UNIQUE(cycle_id, ordinal), UNIQUE(cycle_id, label))

-- A bell schedule. Campus-scoped so campuses can run different times; NULL = school-wide.
period_structure (id, school_id, name, campus_id -> campus NULL, is_default,
                  UNIQUE(school_id, name))
period (id, period_structure_id -> period_structure, ordinal INTEGER, label,
        starts_at TEXT, ends_at TEXT,          -- 'HH:MM' local, zero-padded
        kind TEXT CHECK (kind IN ('TEACHING','BREAK','REGISTRATION','OTHER')),
        counts_as_load BOOLEAN,
        CHECK (ends_at > starts_at),
        UNIQUE(period_structure_id, ordinal), UNIQUE(period_structure_id, label))

-- Materialised (cycle_day x period) grid: the solver's value range.
timeslot (id, school_id, cycle_id, cycle_day_id -> cycle_day, period_id -> period,
          campus_id -> campus NULL, term_id -> term NULL,
          ordinal INTEGER,                      -- global order within the cycle
          is_teaching BOOLEAN)

-- Maps real dates onto cycle days. Rotating cycles, holidays, campus closures.
calendar_day (id, school_id, date DATE, term_id -> term NULL, campus_id NULL,
              cycle_day_id -> cycle_day NULL,   -- NULL = no lessons that day
              kind TEXT CHECK (kind IN ('SCHOOL','HOLIDAY','EXAM','EVENT','PD')),
              note)
```

Three of the uniqueness rules are **expression indexes over `COALESCE`**, not plain
constraints, and the reason matters: SQLite treats `NULL`s as distinct in a `UNIQUE`
index, so a plain `UNIQUE(cycle_day_id, period_id, term_id)` would not have constrained
the term-less rows at all — and "the whole year" is the common case, not the exotic one.
The same applies to the campus-less calendar entries and to the single-default rule.

```sql
CREATE UNIQUE INDEX idx_timeslot_identity
    ON timeslot (cycle_day_id, period_id, COALESCE(term_id, ''));
CREATE UNIQUE INDEX idx_calendar_day_identity
    ON calendar_day (school_id, date, COALESCE(campus_id, ''));

-- At most one default each, the school-wide bucket separate from each campus.
CREATE UNIQUE INDEX idx_cycle_single_default ON cycle (school_id) WHERE is_default = 1;
CREATE UNIQUE INDEX idx_period_structure_single_default
    ON period_structure (school_id, COALESCE(campus_id, '')) WHERE is_default = 1;
```

A trigger additionally requires a timeslot's denormalised `school_id`, `cycle_id` and
`campus_id` to agree with the cycle day and period it was built from, so the grid cannot
drift from the cycle it claims to belong to. Same approach as room/building in V3.

**Two bell schedules mean two disjoint grids over the same cycle days, and which one an
activity uses is resolved rather than stored.** The grid is materialised per
`(cycle, period_structure, term)`, and `timeslot` identity carries no campus, so a school
running two campuses on two schedules holds two sets of `timeslot` rows covering the same
days and often the same minutes. Nothing joins an `activity` to either:
`class_section.campus_id` is nullable and `period_structure.campus_id` is nullable.
[adr/0033](adr/0033-an-activity-schedules-against-one-resolved-bell-schedule.md) decides the
resolution — campus compatibility against the structures actually materialised, with a
refusal rather than a default when it is not unique — and it needs **no column and no
migration**. Two consequences for anyone reading this section as a reference:

- A bell schedule serves either **one** campus or **every** campus. A subset is not
  expressible, because `campus_id` is a column rather than an association table.
- Because the two grids' periods are different `period` rows, a clash on a shared room or
  teacher is decided by comparing `[starts_at, ends_at)` within a `cycle_day`, never by
  comparing `timeslot_id` ([adr/0032](adr/0032-timetable-generation-is-scoped-to-one-school.md)
  rule 2). That comparison needs no timezone and no `calendar_day` row.

Four more triggers hold the cycle kinds (V9, [adr/0017](adr/0017-weekly-and-custom-cycles.md)):
a `WEEKLY` cycle is seven days long, a cycle's `kind` never changes, and a `WEEKLY` cycle's
day carries the weekday of its position — so `weekday_hint` and `ordinal` cannot disagree
there. `ordinal` is still the only scheduling identity; nothing in materialisation, ordering
or the solver reads a weekday. On a `CUSTOM` cycle `weekday_hint` remains optional, free and
display-only. These are triggers rather than table `CHECK`s because `ALTER TABLE` cannot add
one and `cycle` cannot be rebuilt inside refinery's transaction.

### Timeslot identity, and how the grid is rebuilt

Settled in [adr/0011](adr/0011-stable-timeslot-identity.md), and the most important rule
in this section.

A timeslot's **logical identity** is `(term_id, cycle_day_id, period_id)`. Its `id` is
assigned once, when that tuple first appears, and is never reassigned while the tuple
exists. Everything else on the row — `school_id`, `cycle_id`, `campus_id`, `ordinal`,
`is_teaching` — is a *derived projection*, recomputed on every rematerialisation.

`ordinal` is the trap worth naming: it is derived, and it **changes**. Inserting a period
in the middle of the day renumbers every later slot in the cycle while every one of those
slots keeps its `id`. Only `id` is a stable handle — which is also why `ordinal` carries
an index but no uniqueness constraint.

Materialisation works on one **grid**, identified by `(term, cycle, period_structure)`, so
two campuses on different bell schedules over the same cycle are two grids and neither
disturbs the other. Three cases, and the third is the point:

| Case | Action |
| --- | --- |
| desired tuple already exists | keep its `id`; refresh the derived columns, writing only if one actually changed |
| desired tuple does not exist | insert with a fresh UUIDv7 at `rev = 1` |
| existing tuple no longer desired | report it as an **orphan candidate**; change nothing |

`plan_materialisation` only reads. `apply_materialisation` creates and refreshes in one
transaction and still never deletes. `delete_timeslots` takes explicit identifiers and is
the only destructive call. Because `timeslot.cycle_day_id` and `timeslot.period_id` are
`RESTRICT`, an orphan can never be discovered by deleting something first and rebuilding
afterwards — so a request carries optional exclusion lists, which is how a caller asks
"what would I lose if I removed Day F?" *before* removing it.

A second rematerialisation of an unchanged grid writes nothing at all and consumes no
revisions. That is the honest statement of idempotence, and the test asserts it as such.

`timeslot` is **materialised rather than computed** because the solver needs a stable,
indexable value range with real IDs that a locked assignment can point at.

### calendar_day semantics

`calendar_day` is what makes rotating and multi-week cycles work: the timetable is defined
over the cycle, and the calendar projects it onto dates. One row per date per campus, with
`campus_id IS NULL` meaning the whole school.

The mapping is **explicit and stored**, one row per date, and Phase 1D does not generate
it. What Day follows a holiday, and whether a closure consumes a rotation slot, is policy
this document does not state; generating it from a guess would be indistinguishable from
having decided it. So a six-day rotation across a weekend is simply four rows that say so:

```
2027-09-01 -> Day A      2027-09-03 -> Day C
2027-09-02 -> Day B      2027-09-06 -> Day A   (the weekend is simply absent)
```

`cycle_day_id IS NULL` means no lessons that day. `kind` records why — `HOLIDAY`, `EXAM`,
`EVENT`, `PD` — but nothing in scheduling reads it yet. Automatic calendar generation is a
later phase.

---

## 4. People

**As implemented from V12**, every list of a school-owned entity below takes an explicit
`school_id`, and `update_*` refuses an identifier belonging to another school. A year level
is never shared: `UNIQUE (school_id, name)` and `UNIQUE (school_id, ordinal)` were already
written that way and need no change, so two schools may each hold their own "Grade 1" at
their own ordinal 1 as two records with two identifiers.

```sql
teacher (id, school_id, code NULL, given_name, family_name, email NULL,
         home_campus_id NULL, fte_percent INTEGER,
         max_periods_per_day INTEGER NULL, max_periods_per_cycle INTEGER NULL,
         max_consecutive_periods INTEGER NULL, notes, is_active,
         UNIQUE(school_id, code) WHERE code IS NOT NULL)

year_level (id, school_id, name, ordinal)
student (id, school_id, code NULL, given_name, family_name, email NULL,
         year_level_id NULL, campus_id NULL, is_active,
         UNIQUE(school_id, code) WHERE code IS NOT NULL)

-- One table for every audience: a class, a whole cohort, a homeroom, an elective set.
student_group (id, school_id, name, code,
               kind TEXT CHECK (kind IN ('CLASS','COHORT','HOMEROOM',
                                         'ELECTIVE_SET','TEACHING_SET')),
               year_level_id NULL, parent_group_id -> student_group NULL,
               size INTEGER,                    -- declared size when membership is not modelled
               campus_id NULL)
student_group_member (id, student_group_id, student_id,
                      UNIQUE(student_group_id, student_id))
```

`parent_group_id` gives the hierarchy that student clash detection reads, and `size` exists
so a school can timetable without entering a single student.

**Corrected 2026-10-21 by [adr/0035](adr/0035-student-group-overlap-is-a-three-state-answer.md).**
This paragraph previously read *"a cohort split into three teaching sets means those sets
conflict with the cohort but not with each other"*, and *"clash detection resolves a group to
its leaf student set"*. Both are wrong, and the record argues each at length:

- **Siblings are not disjoint.** The claim is true of three *classes* of a cohort and false
  of three *teaching sets of different subjects*, which are structurally identical in the
  stored data — same parent, same `kind`, nothing to tell them apart. A tree of containment
  cannot express two different divisions of one population, which is why the resolution needs
  one future nullable column (`student_group.children_are_disjoint`, asked per parent) rather
  than an inference. **It is not in `V10` and not in any migration.**
- **Resolution is not to a leaf student set.** [adr/0006](adr/0006-group-based-scheduling-first.md)
  requires a school with no `student` rows to get a full timetable, so its leaf sets are all
  empty and an expansion would make clash detection silent for exactly those schools. Overlap
  is a three-state function over *groups* — `Overlaps`, `Disjoint`, `Indeterminate` — proved
  from member rows where they exist, from containment and declared-size arithmetic where they
  do not, and reported rather than enforced when it cannot be proved.

Student records are optional in Milestone 1 and exist so individual request-driven
scheduling can be added without a schema break. Fixtures use synthetic names only.

**As implemented in V10** — see
[adr/0018](adr/0018-people-identity-and-deactivation.md) for all five departures from the
sketch above:

- `is_active` on `teacher` and `student`. Employment and enrolment state, not a soft
  delete: nothing filters on it, and an inactive person keeps every row they ever had.
- `code` is optional, with a **partial** unique index rather than a `COALESCE` one. This
  is the documented exception to CLAUDE.md's COALESCE rule: that rule is for unset rows
  that must still collide, and here a school with no staff numbers must be able to enter
  more than one teacher.
- `fte_percent INTEGER` rather than `fte REAL`, because the Phase 4 load constraints sum
  and compare it and that arithmetic is integer everywhere else.
- `student_group_member` carries its own `id` and audit columns rather than a composite
  primary key, so it is visible to the change-log triggers — the same reason
  `building_campus` does, recorded in V5.
- `department`, `subject`, `teacher_qualification` and `teacher_department` arrive with
  Curriculum in `V14`, and are documented in §5 rather than here. **`teacher.department_id`
  is not added at all** — see [adr/0026](adr/0026-teacher-departments-are-a-narrowing-of-membership.md),
  which records why a single column on a project-level identity cannot say "Mathematics at
  Northgate, Science at Southmoor" and is therefore the wrong shape rather than a deferred
  one.

`delete_student_group` removes its own membership rows and refuses while child groups
remain; `delete_student` refuses while the student is in any group, because a membership is
the only record that they ever were.

### Project-level people, and where belonging lives — V13

**As implemented in V13.** A teacher and a student are each one row, for one real person,
for the life of the project. Where they belong is a relationship, so that a trust sharing
staff does not have to model one human being twice — which would give them two identities,
two availability records and two independent timetables.

```sql
-- Which schools employ this person. THE AUTHORITY on "which schools?".
teacher_school (id, teacher_id, school_id, is_active,
                UNIQUE(teacher_id, school_id))

-- Which campuses they may be based at. A NARROWING of membership, never an independent
-- claim: a trigger pair refuses a campus of a school they do not belong to.
teacher_campus (id, teacher_id, campus_id, UNIQUE(teacher_id, campus_id))

-- One row per SPELL at one school. A transfer writes a LEFT row and an ENROLLED row; it
-- never edits one in place, so history survives.
student_school_enrolment (id, student_id, school_id, admission_no NULL,
                          year_level_id NULL, campus_id NULL,
                          status TEXT CHECK (status IN ('ENROLLED','LEFT','PROSPECTIVE')),
                          enrolled_on NULL, left_on NULL,
                          UNIQUE(student_id, school_id) WHERE status <> 'LEFT',
                          UNIQUE(school_id, admission_no)
                              WHERE admission_no IS NOT NULL AND status <> 'LEFT')
```

What each of those says, and the reasoning is in
[adr/0021](adr/0021-project-level-people-and-school-membership.md):

- **Home school versus membership.** `teacher.school_id` and `student.school_id` are the
  **home** school — the one the person was created under. Immutable, always has a matching
  relationship row, its relationship cannot be withdrawn. **Nothing reads them to answer
  "which schools?"**: `People::teachers` reads `teacher_school` and `People::students`
  reads `student_school_enrolment`, so a shared person appears in both schools' lists as
  one record with one identifier.
- **School-specific teacher status.** `teacher.is_active` says whether they are in the
  project at all; `teacher_school.is_active` says whether *this* school employs them.
  Effective eligibility is the **conjunction**, computed in one place. Nothing may read the
  project-level flag alone and conclude a teacher is available to a school.
- **Enrolment history.** Teacher membership is deactivated rather than duplicated;
  enrolment is history, because a spell is what a school is asked about. One current spell
  per (student, school) — `ENROLLED` or `PROSPECTIVE` — and any number of `LEFT` ones, so a
  student may leave and rejoin, and may be enrolled at two schools at once.
- **School-specific admission numbers.** An admission number is a *school's* identifier for
  a student, so it lives on the enrolment, never project-wide: two schools may legitimately
  number the same person differently. The index is partial twice over — unique when given,
  and scoped to current spells, so a retired number may be reissued and a past spell keeps
  the number it had. `COALESCE(enrolled_on, '')` was rejected for the uniqueness rule: it
  would fold every undated spell onto one key and stop a school that does not record dates
  from re-enrolling anybody.
- **One placement, kept in step.** `student.code`, `student.year_level_id` and
  `student.campus_id` remain as the **home-school** values, the treatment `room.campus_id`
  got. They are not independently writable: an edit through either `update_student` or
  `update_enrolment` writes both sides in one transaction, through `repo::update`, so no
  revision and no change-log entry lies. Only the home school's current spell participates.
- **A group's members must be enrolled in the group's school.** A trigger pair, stated
  against enrolment rather than `student.school_id`, which is what makes it right under
  sharing. Any spell counts, past ones included, because a membership is the only record
  that a student was ever in 9-MAT-A. ADR 0006 is untouched: a school timetabling from
  `student_group.size` writes no rows here.
- **Deleting a person is a third operation**, distinct from deactivating and from removing
  a relationship. Only the home school may, and only while nobody else employs or enrols
  them — `DbError::StillReferenced` otherwise, checked before anything is cleared.
  `Structure::school_dependents` gained `shared_teachers` and `shared_students` so a
  confirmation can say what is in the way.

**Not in V13**, and not implied by it: a project-wide teacher code (the blocker and the
better alternative are in ADR 0021's *Known limitation*), project-level availability, and
cross-school scheduling. `teacher_qualification` and `department` were on this list and
arrived in `V14`; see §5.

---

## 5. Curriculum

### What `V14` implements — departments, subjects and qualifications

The curriculum prerequisites, one stage ahead of the lessons that read them: Stage C's
`course` references a subject and cannot be designed around a table that does not exist.
Two decisions are recorded as ADRs because neither is obvious —
[adr/0025](adr/0025-teacher-qualifications-across-school-owned-subjects.md) for
qualifications and [adr/0026](adr/0026-teacher-departments-are-a-narrowing-of-membership.md)
for departments.

```sql
-- How ONE school organises the teaching of its curriculum. Never shared: two schools may
-- each hold a "Mathematics Department" as two records with two identifiers.
department (id, school_id, name, code NULL, description NULL,
            UNIQUE(school_id, name),
            UNIQUE(school_id, code) WHERE code IS NOT NULL)

-- What a school teaches. School-owned, so Northgate's Mathematics and Southmoor's are two
-- subjects; they may share a name AND a code without colliding, and nothing merges them.
subject (id, school_id, department_id NULL -> department, name, code NULL, description NULL,
         UNIQUE(school_id, name),
         UNIQUE(school_id, code) WHERE code IS NOT NULL)

-- A property of the PERSON, naming one school-owned subject. No school_id: the subject
-- supplies it. No membership requirement: a teacher who leaves does not become unqualified.
teacher_qualification (id, teacher_id, subject_id, UNIQUE(teacher_id, subject_id))

-- A NARROWING of school membership, modelled on teacher_campus. A trigger pair refuses a
-- department of a school the teacher does not belong to; removing the membership clears it.
teacher_department (id, teacher_id, department_id, UNIQUE(teacher_id, department_id))
```

Three departures from the sketch below, each deliberate:

- **`subject.department_id` is `ON DELETE RESTRICT`**, not ADR 0010's `SET NULL` for an
  optional classification. ADR 0025 §6 argues it: a database-level `SET NULL` rewrites rows
  without incrementing `rev`, which is R10 in [OPEN-DECISIONS](OPEN-DECISIONS.md) and
  contradicts the revision contract [adr/0027](adr/0027-revisions-are-checked-on-write.md)
  installs in the same stage. `delete_department` refuses while any subject or teacher
  refers to it, with `Curriculum::department_dependents` naming the counts first.
- **`subject.colour` and `subject.default_room_type_id` are absent.** A colour is a
  rendering decision belonging with the renderer in Phase 7; a default room type is a
  scheduling default belonging with `course.required_room_type_id` in Stage C, where there
  is something to default. Either is one nullable `ALTER TABLE` — the cheap direction.
- **`subject` is unique on `name` as well as `code`**, because `code` is optional. The
  documented sketch has `UNIQUE(school_id, code)` alone, which would let a school with no
  codes enter "Mathematics" twice.

`teacher_qualification` and `teacher_department` are **first-class entities** with their own
UUIDv7 `id`, audit columns and change-log triggers, rather than composite-key junctions —
the reason `V5`, `V6`, `V10` and `V13` all give: the triggers read `NEW.id` and `NEW.rev`,
and `repo::update` / `repo::delete` address rows by `id`.

**Not in `V14`**, and not implied by it: qualification levels or certification expiry
(ADR 0025's *Known limitation*), a head of department or a primary-department flag
(ADR 0026's), a department hierarchy, a project-wide subject catalogue (rejected in ADR 0025
§2), and everything below.

### What `V15` implements — courses, class sections, and the demand they make

Stage C, and the chain that turns a subject into something a timetable can be built from.
Two decisions are recorded as ADRs because neither is obvious —
[adr/0028](adr/0028-assigning-a-lesson-warns-about-qualifications.md) for who may be given a
lesson and [adr/0029](adr/0029-a-lesson-is-a-course-a-section-and-its-occurrences.md) for
what shape a lesson's demand has.

```sql
-- This school's offering of one subject: "Year 9 Mathematics".
--
-- NO school_id. `subject.school_id` is the only answer, and the chain
-- subject -> course -> class_section -> activity carries it the whole way down.
course (id, subject_id -> subject, year_level_id NULL, term_id NULL,
        name, code NULL, description NULL,
        periods_per_cycle INTEGER NULL,            -- the DECLARED demand; NULL = undecided
        default_duration_periods INTEGER NOT NULL DEFAULT 1,
        required_room_type_id NULL -> room_type,
        UNIQUE (subject_id, name))
-- `code` is unique within the SCHOOL, which needs a trigger pair rather than a partial
-- index, because this table deliberately does not name its school.

-- One deliverable instance of that offering: "Year 9 Mathematics A".
class_section (id, course_id -> course, name, code NULL, campus_id NULL -> campus,
               UNIQUE (course_id, name),
               UNIQUE (course_id, code) WHERE code IS NOT NULL)

-- Who teaches it. Many per section: co-teaching is ordinary.
class_teacher (id, class_section_id, teacher_id, UNIQUE (class_section_id, teacher_id))

-- Who attends it. The PRIMARY scheduling population (ADR 0006).
class_group (id, class_section_id, student_group_id,
             UNIQUE (class_section_id, student_group_id))

-- Which named individuals attend it. OPTIONAL, and a roster overlay rather than a second
-- population: nothing adds this count to the groups' size.
class_enrolment (id, class_section_id, student_id,
                 UNIQUE (class_section_id, student_id))

-- ONE placeable occurrence of that instance. Four single periods a cycle is FOUR rows.
activity (id, class_section_id -> class_section, name NULL,
          duration_periods INTEGER NOT NULL DEFAULT 1,   -- consecutive periods, one occurrence
          split_index INTEGER NOT NULL,                  -- assigned once, never renumbered
          UNIQUE (class_section_id, split_index))
```

Six departures from the sketch below, each deliberate and each argued in ADR 0029:

- **`course` has no `school_id`.** The sketch wrote one; `ROADMAP.md`'s Phase 3 notes had
  already corrected it, and `CLAUDE.md` states the rule in general. Every school-owned read
  and write resolves its school through the chain, and
  `tests_lessons::a_course_has_no_school_column_and_resolves_one_through_its_subject`
  asserts the absence on all six tables so a future migration cannot quietly add one.
- **An activity is one occurrence, not a count of them.** There is no
  `occurrences_per_cycle`: the count *is* the number of rows, and the demand is
  `SUM(duration_periods)`. `timetable_entry`'s `UNIQUE (timetable_id, activity_id,
  timeslot_id)` already assumes this, Phase 4's "spread these four across four days" needs
  individually addressable occurrences, and the solver's variable set is then the rows
  themselves with no expansion step. ADR 0029 §2.
- **`activity.class_section_id` is `NOT NULL`, and the table has no `school_id` and no
  `kind`.** The sketch's nullable column exists for an assembly or a yard duty, which
  Stage C does not model; carrying three columns no screen can explain is the mistake `V14`
  named about `subject.colour`. Adding them is an ADR 0020 rebuild, which `V12` has done
  once already.
- **`enrolment` is named `class_enrolment`**, because `V13` already made "enrolment" mean *a
  spell at a school* throughout the crate. It names the **student**, not the spell — a
  student may hold several spells at one school, so a spell-keyed roster could list the same
  child twice — and a trigger pair requires a non-`LEFT` spell at the section's school.
- **`class_section.student_group_id` is dropped.** The sketch has it *and* `class_group`,
  which is two places to say who attends. The many-to-many is what co-teaching and combined
  cohorts need.
- **`class_teacher.role` is absent**, and so are `class_section.max_size` and
  `class_enrolment.status`. All three have no reader in Milestone 1, and the sketch's
  `(section, teacher, role)` key would let one person be a section's primary *and* its
  assistant at once. ADR 0029 §5 lists every omission with what adding it costs.

  `class_section.max_size` **stays** reserved for Milestone 2's sectioning —
  `section-max-size` and `section-balance` in [adr/0006](adr/0006-group-based-scheduling-first.md).
  A room's occupancy figure is a *different* quantity and gets a different, also unbuilt,
  column: [adr/0034](adr/0034-expected-attendance-is-explicit-and-room-capacity-waits-for-it.md)
  specifies `class_section.expected_attendance` and defers `room-capacity` until it exists,
  because `room.capacity` currently has nothing to be compared against. **Neither column is
  in `V15` or in any migration.** Until then a section's expected attendance is either
  derived from declared group sizes over provably disjoint groups or is `Unknown` — and
  `Unknown` is never read as zero, which is what the displayed audience total on the Class
  section screen does and is labelled to do.

**Who may be given a lesson** is `V15`'s other substance, and it amends what ADR 0025 §4
promised. An **active** `teacher_school` membership and campus eligibility are *requirements*
— trigger pairs, plus a repository check that can name the remedy — and a missing
`teacher_qualification` is a **warning** the interface shows. ADR 0028 argues it; the short
version is that a database that refuses will be answered by entering a qualification
somebody does not have, which destroys the record rather than protecting it. Withdrawing a
membership is refused while any of that school's class sections remain assigned, which is
the opposite of what `V13` and `V14` chose for the other two narrowings and is argued in
ADR 0028 §5.

**Not in `V15`**, and not implied by it: elective lines, course requests, per-occurrence
rooms, resources or teachers, activity relationships, and any notion of a timetable. See
ADR 0029 §5.

### The rest of the group, as documented and not yet built

```sql
-- Individual demand, for request-driven (elective-line) scheduling later.
course_request (id, student_id, course_id, term_id NULL,
                kind TEXT CHECK (kind IN ('REQUIRED','ELECTIVE','ALTERNATE')),
                priority INTEGER, weight REAL,
                alternate_of_id -> course_request NULL)
```

Both scheduling traditions are representable: group-based (fill `student_group` and
`class_group`, which `V15` does) and request-based (fill `course_request` and let a
sectioning phase derive `class_enrolment`). Milestone 1 solves group-based only — see
[OPEN-DECISIONS.md](OPEN-DECISIONS.md) D5.

---

## 6. Activities — the thing that gets scheduled

An **activity** is one placeable unit of work, and since `V15` it is **one occurrence of one
class section**. A class meeting four times a cycle for one period holds four rows; a class
meeting once for a double holds one row of `duration_periods = 2`.

Lessons are the only case `V15` models. Assemblies, meetings, homeroom, sport and yard duties
are the same *shape*, which is why they are not special-cased — but they have no class
section, so they need the `school_id` and `kind` columns `V15` deliberately omits, and they
arrive with the stage that has a screen for them.

```sql
activity (id, class_section_id -> class_section, name NULL,
          duration_periods INTEGER NOT NULL DEFAULT 1,
          split_index INTEGER NOT NULL,
          created_at, updated_at, rev,
          UNIQUE (class_section_id, split_index))
```

Teachers and groups are attached to the **class section**, many-to-many from day one —
co-teaching and combined-cohort lessons are normal, and retro-fitting them is expensive. The
Java prototype's single `teacherId` is exactly the shortcut to avoid. The sketch's
`activity_teacher` and `activity_group` are a per-occurrence *override* of that and are not
built: two places to say who teaches a lesson is two places for them to disagree.

```sql
-- Not in V15. Per-occurrence overrides and room allocation; see ADR 0029 §5.
activity_teacher (activity_id, teacher_id, role)
activity_group (activity_id, student_group_id)
activity_room_candidate (activity_id, room_id)
activity_resource (activity_id, resource_id, quantity)
```

### Duration

A double period is **one activity with `duration_periods = 2`** occupying consecutive
periods, not two linked activities. Simpler constraints, simpler UI, and it makes
"this must not be split" structural rather than a rule that can be switched off.

### What can be said about a demand before there is a solver

`evara_core::lessons` answers two questions and refuses a third.
`LessonDemand::of` sums a section's occurrences; `CycleCapacity::from_schedules` measures
what the school's cycle and bell schedules offer, including the **longest unbroken run** of
teaching periods — which is the figure a multi-period occurrence actually needs and is not
the same as the periods in a day.

`DemandFinding` then reports only **impossibility**: an occurrence longer than any run, or a
total beyond every slot in the cycle. "Twenty-nine periods in a thirty-period cycle" is
probably infeasible and is deliberately **not** reported, because only a solver can tell
difficulty from impossibility. Feasibility is never reported at all, and the screen says so
in those words. Before the school has a cycle and a bell schedule, nothing is judged — the
setup order puts Lessons at section 10 and the time model at 14–17. ADR 0029 §6.

### Lines / blocks and relationships

```sql
-- An elective line: several activities that must run in the same slot so students
-- choose exactly one.
line (id, school_id, name, code, term_id NULL, year_level_id NULL)
line_member (line_id, activity_id, PRIMARY KEY(line_id, activity_id))

activity_link (id, school_id,
               kind TEXT CHECK (kind IN ('SAME_PERIOD','DIFFERENT_PERIOD','CONSECUTIVE',
                                         'BEFORE','AFTER','SAME_DAY','DIFFERENT_DAY',
                                         'SAME_ROOM','DIFFERENT_ROOM')),
               level, weight, gap_periods INTEGER NULL, enabled)
activity_link_member (activity_link_id, activity_id, ordinal,
                      PRIMARY KEY(activity_link_id, activity_id))
```

`line` is first-class because the UI needs it as an object users build and reason about;
internally it is evaluated as a `SAME_PERIOD` link group.

### Availability — one table for all entity kinds

```sql
availability (id, school_id,
              entity_kind TEXT CHECK (entity_kind IN ('TEACHER','ROOM','STUDENT_GROUP',
                                                      'RESOURCE','CAMPUS','SUBJECT')),
              entity_id TEXT NOT NULL,
              timeslot_id NULL, cycle_day_id NULL, period_id NULL,  -- any combination
              term_id NULL,
              status TEXT CHECK (status IN ('UNAVAILABLE','PREFERRED','DISCOURAGED')),
              weight REAL, reason,
              effective_from DATE NULL, effective_to DATE NULL)
```

Available is the default, so only exceptions are stored. The nullable triple expresses
"all of Monday" (`cycle_day_id` only), "period 1 every day" (`period_id` only) or one
exact slot. `entity_id` is polymorphic and therefore cannot carry a foreign key — the
repository validates it and an integrity check sweeps for orphans.

**As implemented in V11**, with two departures recorded in
[adr/0018](adr/0018-people-identity-and-deactivation.md):

- `weight` is `INTEGER`, not `REAL`, extending the same rule that made `teacher.fte_percent`
  an integer: the solver sums and compares it, that arithmetic is integer everywhere else,
  and a float would make the record the only non-`Eq` entity in the crate. `NULL` means
  unset — Phase 4 supplies the constraint's own default.
- A `CHECK` requires at least one of `timeslot_id`, `cycle_day_id` and `period_id`. A rule
  naming no time at all is a rule about nothing.

Identity is an expression index over `COALESCE`, and this is the **opposite** case to
`teacher_code_unique`:

```sql
CREATE UNIQUE INDEX idx_availability_identity ON availability (
    entity_kind, entity_id,
    COALESCE(timeslot_id, ''), COALESCE(cycle_day_id, ''),
    COALESCE(period_id, ''), COALESCE(term_id, ''));
```

A staff code is unique *when given*, so rows without one must not collide and the index is
partial. Two availability rules that both say "all of Monday, all year" are the same rule
written twice and **must** collide, so the unset columns are folded to a value that compares
equal. Both follow from the same reading of CLAUDE.md's rule; they differ because the
question differs.

The three time references and `term_id` are `RESTRICT`. That is what stops a cycle-day
deletion from silently discarding the availability configured against it: the delete is
refused and the user is told. `entity_id` has no such protection, so every repository that
deletes something which can carry rules clears them in the same transaction, and
`Availability::orphans` sweeps for any that got through.

The editor writes **recurring cell rules only** — `cycle_day_id` and `period_id` both set.
The whole-day, whole-period and exact-slot forms remain writable through the repository and
readable by the solver; the grid reports how many such rules exist rather than hiding them.

---

## 7. Scheduling

-- Built in V17. SIX details differ from the sketch this comment replaces, each recorded
-- in adr/0042 and in the migration's own header: one row per OCCURRENCE rather than per
-- occupied slot, no `school_id` (the cycle carries one), every child edge RESTRICT with the
-- repository clearing it, `penalty` INTEGER, no `status` / `based_on_timetable_id` / score
-- columns, and violations hanging off a `timetable_evaluation` so a revalidation cannot
-- overwrite how a timetable was generated.
--
-- `timetable_entry_teacher` is NOT built: per-occurrence staffing overrides are a later
-- phase (ADR 0029 §5), and a section's staffing is fixed input until then.
timetable (id, cycle_id -> cycle ON DELETE RESTRICT,   -- the school is cycle.school_id
           term_id -> term NULL ON DELETE RESTRICT,
           name, generated_at,
           indeterminate_student_pairs, unchecked_capacities,
           shared_entities, unenforced_restrictions,   -- ADR 0032 rule 4, never omitted
           created_at, updated_at, rev)
-- Two timetables for one scope may not share a name. A UNIQUE INDEX, because SQLite allows
-- an expression only in an index, and COALESCE because a NULL term is the common case.
CREATE UNIQUE INDEX idx_timetable_scope_name
    ON timetable (cycle_id, COALESCE(term_id, ''), name);

timetable_entry (id, timetable_id -> timetable ON DELETE RESTRICT,
                 activity_id -> activity ON DELETE RESTRICT,
                 timeslot_id -> timeslot ON DELETE RESTRICT,   -- the START slot
                 room_id -> room NULL ON DELETE RESTRICT,
                 is_locked BOOLEAN DEFAULT 0,       -- solver must preserve. Written by M9A's
                                                    -- editor; read into Problem::pinned by the
                                                    -- caller, never by the loader (ADR 0045)
                 is_manual BOOLEAN DEFAULT 0,       -- placed by a human. Written by M9A's
                                                    -- editor, and not by set_lock
                 created_at, updated_at, rev,
                 UNIQUE(timetable_id, activity_id, timeslot_id),
                 UNIQUE(timetable_id, activity_id))  -- one placement per occurrence

-- Built in V16. Three details differ from the sketch this comment replaces, each
-- recorded in adr/0040: `constraint_ref` is an entity with audit columns and change-log
-- triggers rather than a composite-key junction row, its edge is RESTRICT with the
-- repository clearing the rows itself rather than CASCADE, and `weight` is an INTEGER.
constraint_instance (id, school_id -> school ON DELETE RESTRICT,
                     type_key TEXT NOT NULL,
                     level TEXT CHECK (level IN ('HARD','MEDIUM','SOFT')),
                     weight INTEGER NOT NULL DEFAULT 1 CHECK (weight >= 0),
                     enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
                     params TEXT NOT NULL DEFAULT '{}'    -- JSON object, CHECKed as one
                       CHECK (json_valid(params) AND json_type(params) = 'object'),
                     term_id NULL -> term ON DELETE RESTRICT,  -- NULL = the whole year
                     created_at, updated_at, rev)         -- no timetable_id until M7
constraint_ref (id, constraint_id -> constraint_instance ON DELETE RESTRICT,
                role TEXT CHECK (role IN ('appliesTo','subject','yearLevel','campus',
                                          'timeslot','cycleDay','period')),
                entity_kind TEXT, entity_id TEXT,         -- polymorphic, so no FK
                created_at, updated_at, rev,
                UNIQUE(constraint_id, role, entity_id))

-- Built in V17. `best_*` and `log` are not: a score belongs to the EVALUATION that
-- produced it (one on `timetable` goes stale on the first manual edit), and nothing writes
-- a log. `inputs_watermark` is new -- the change-log sequence number the problem was
-- compiled at, which is what makes a stale save refusable.
solve_run (id, timetable_id -> timetable ON DELETE RESTRICT,
           engine, seed, status, termination, settings TEXT, inputs_watermark,
           candidate_evaluations, candidates_rejected, placements_committed,
           placements_withdrawn, construction_steps, passes,
           started_at, finished_at, app_version,
           created_at, updated_at, rev)
solve_run_unscheduled (id, solve_run_id -> solve_run ON DELETE RESTRICT,
                       activity_id -> activity, cause, diagnostic_code NULL,
                       UNIQUE(solve_run_id, activity_id))
-- One scoring. GENERATION is immutable provenance and belongs to a run; REVALIDATION is the
-- replaceable current one. Two partial unique indexes enforce at most one of each.
timetable_evaluation (id, timetable_id -> timetable ON DELETE RESTRICT,
                      kind TEXT CHECK (kind IN ('GENERATION','REVALIDATION')),
                      solve_run_id -> solve_run NULL, evaluated_at, inputs_watermark,
                      score_hard, score_medium, score_soft,   -- negative-going
                      placed_count, required_count,
                      created_at, updated_at, rev)
violation (id, evaluation_id -> timetable_evaluation ON DELETE RESTRICT,
           constraint_type, level, constraint_id NULL, penalty INTEGER,
           cycle_day_id NULL, starts_at NULL, ends_at NULL, detail)
violation_ref (id, violation_id -> violation ON DELETE RESTRICT,
               role, entity_kind, entity_id, ordinal)
```

`UNIQUE(timetable_id, activity_id, timeslot_id)` is kept verbatim, and `V17` adds
`UNIQUE(timetable_id, activity_id)` beside it. **The sketch's reason for the first key did not
survive**: it said the key was wider *"because a multi-period activity occupies several slots"*,
and `V17` stores **one row per occurrence** carrying the start slot, with the run derived by
`evara_core::scheduling::span`. Rows 2…`d` are derived, and storing derived data makes a corrupt
timetable representable — three rows for a two-period lesson — and makes a *partial* multi-period
move representable in Phase 8, which is the state this sketch itself called impossible.
[adr/0042](adr/0042-a-stored-timetable-is-a-version-and-a-save-refuses-stale-inputs.md) D1, and
[TIMETABLE-PERSISTENCE.md](TIMETABLE-PERSISTENCE.md) §2.

Three more things these tables do that the DDL cannot say:

- **A save refuses inputs that moved.** Between generating a timetable and saving it, an
  availability rule or a room's bookability may change — and **nothing referential breaks**, so
  nothing else would catch it. `solve_run.inputs_watermark` is the change-log sequence number the
  problem was compiled at, compared against the feed inside the writing transaction.
- **A regeneration never overwrites.** Several timetables coexist for one `(cycle, term)`, and
  that is the versioning. There is no `status`, because the published/archived lifecycle is not in
  the MVP.
- **A revalidation replaces only the current scoring.** The `GENERATION` evaluation is immutable
  for the life of the timetable, so re-scoring cannot misrepresent how it was generated.

### Why constraints store parameters as JSON

There are 40+ constraint families and the list grows. A column per family means a
migration per constraint; a table per family means 40 repositories. Instead:
`type_key` + JSON `params`, with the code-side registry publishing a **JSON Schema** per
type that both validates writes and generates the configuration form.

The cost is that entity references inside JSON get no foreign key. `constraint_ref`
exists to pay that back: scope references are normalised into real rows, and `params` holds
only scalars. See [CONSTRAINTS.md](CONSTRAINTS.md) for the catalogue and
[CONSTRAINT-ENGINE.md](CONSTRAINT-ENGINE.md) for what is implemented.

Two things about these tables that the DDL cannot say:

- **No rows are seeded, and the absence is the design.** The implemented types are mandatory
  in the registry, so a project with an empty `constraint_instance` is fully checked and a
  row here is a school's disagreement with a default.
  [adr/0039](adr/0039-a-mandatory-constraint-lives-in-the-registry.md).
- **Deleting an instance is written, not cascaded.** A trigger refuses an instance while any
  scope row survives, and the repository removes them itself so each is a truthful change-log
  entry — SQLite fires a `DELETE` trigger for a foreign-key action only under
  `recursive_triggers`, which this project never enables.
  [adr/0040](adr/0040-the-constraint-tables-are-entities-and-their-scope-is-cleared.md).

---

## 8. Daily operations (schema reserved, not implemented in M1)

```sql
absence (id, teacher_id, starts_on, ends_on, reason, notes)
coverage (id, timetable_entry_id -> timetable_entry, date DATE,
          kind TEXT CHECK (kind IN ('SUBSTITUTE','CANCELLED','MERGED',
                                    'ROOM_CHANGE','TIME_CHANGE')),
          substitute_teacher_id NULL, new_room_id NULL, new_timeslot_id NULL,
          merged_into_entry_id NULL, note)
```

The published timetable stays immutable; day-to-day reality is a **layer of dated
overrides** on top of it. That keeps "what was the plan" and "what actually happened"
separable, which is what substitution reporting and yard-duty rosters need.

---

## 9. Indexes that matter

**Built and planned are mixed below; each block says which.** Everything under *planned*
belongs to a table no migration defines, so none of it exists at `V15`.

```sql
-- BUILT in V17, as planned here.
CREATE INDEX idx_entry_timetable_slot ON timetable_entry(timetable_id, timeslot_id);
CREATE INDEX idx_entry_timetable_act  ON timetable_entry(timetable_id, activity_id);
CREATE INDEX idx_entry_room           ON timetable_entry(timetable_id, room_id, timeslot_id);
-- BUILT in V17, beside them.
CREATE INDEX idx_timetable_cycle      ON timetable(cycle_id, term_id);
CREATE INDEX idx_solve_run_timetable  ON solve_run(timetable_id);
CREATE INDEX idx_violation_evaluation ON violation(evaluation_id, level);
-- BUILT. V15's, for the RESTRICT scans and for the "what does this teacher teach?" and
-- "which classes is this group in?" reads the People screens ask per row.
CREATE INDEX idx_class_teacher_teacher ON class_teacher(teacher_id);
CREATE INDEX idx_class_group_group     ON class_group(student_group_id);
CREATE INDEX idx_class_enrolment_student ON class_enrolment(student_id);
CREATE INDEX idx_course_subject        ON course(subject_id);
CREATE INDEX idx_class_section_course  ON class_section(course_id);
-- PLANNED, later, with the per-occurrence overrides themselves (ADR 0029 §5).
CREATE INDEX idx_act_teacher_teacher  ON activity_teacher(teacher_id);
CREATE INDEX idx_act_group_group      ON activity_group(student_group_id);
-- BUILT.
CREATE INDEX idx_avail_entity         ON availability(entity_kind, entity_id);
CREATE INDEX idx_timeslot_cycle       ON timeslot(cycle_id, ordinal);
CREATE INDEX idx_change_log_seq       ON change_log(entity_table, entity_id, seq);
```

The solver does not query SQLite in its inner loop — it loads the whole problem into
flat, index-addressed arrays once. These indexes serve the UI and the loader.
