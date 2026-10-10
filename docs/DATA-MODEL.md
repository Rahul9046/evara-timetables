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
| `rev` | `INTEGER NOT NULL DEFAULT 1` | Incremented **by the repository**, never by a trigger — see [OPEN-DECISIONS](OPEN-DECISIONS.md) D17. Optimistic concurrency now, merge input later. |
| `deleted_at` | `TEXT NULL` | Soft delete, only on tables a future sync must tombstone. |

Natural keys (`code`) are unique *within* their parent, never the primary key — schools
rename and reuse codes.

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
| **Structure** | school, campus, building, room, room_type, resource, academic_year, term, cycle, cycle_day, period_structure, period, timeslot, calendar_day | — |
| **People** | teacher, department, teacher_qualification, year_level, student, student_group, student_group_member | Structure |
| **Curriculum** | subject, course, class_section, class_teacher, class_group, enrolment, course_request | Structure, People |
| **Activities** | activity, activity_teacher, activity_group, activity_room_candidate, activity_resource, line, line_member, activity_link, activity_link_member, availability | Structure, People, Curriculum |
| **Scheduling** | timetable, timetable_entry, constraint_instance, constraint_ref, solve_run, violation | all of the above |
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

### Deletion semantics

Decided in Phase 1C and enforced by foreign keys — see
[adr/0010](adr/0010-structure-deletion-semantics.md):

| Edge | Behaviour |
| --- | --- |
| `campus.school_id`, `building.campus_id`, `room.campus_id`, `resource.school_id`, `resource.campus_id`, `academic_year.school_id`, `term.academic_year_id` | `RESTRICT` — a parent that still has children cannot be deleted |
| `room.building_id`, `room.room_type_id` | `SET NULL` — the room survives, unclassified |

Repositories clear the optional references with proper rev-bumping updates before
deleting a parent, so `SET NULL` is a backstop rather than the normal route.

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

A timeslot is never cascade-deleted, by design. When `timetable_entry` arrives its
reference to `timeslot` will be `RESTRICT` too, so a deletion is refused rather than
silently taking placements with it.

### Uniqueness, as implemented

Following the documented rule that codes are unique within their parent:
`campus(school_id, code)` · `room_type(school_id, code)` · `room(campus_id, code)` ·
`resource(school_id, code)` · `building(campus_id, name)` ·
`academic_year(school_id, name)` · `term(academic_year_id, ordinal)` and
`term(academic_year_id, name)`.

`school` is a singleton, enforced by a constant column with a unique index: a project
describes one school ([ADR 0003](adr/0003-sqlite-document-model.md)). A room's building
must be on the room's own campus, enforced by a trigger.

```sql
school (id, name, timezone, locale, created_at, updated_at, rev)

campus (id, school_id -> school, name, code, address, UNIQUE(school_id, code))
campus_travel (from_campus_id, to_campus_id, minutes, PRIMARY KEY(from, to))
building (id, campus_id -> campus, name)
room_type (id, school_id, name, code)                       -- "Science Lab", "Gym"
room (id, campus_id -> campus, building_id -> building NULL, name, code,
      capacity INTEGER, room_type_id -> room_type NULL, is_bookable, notes)
resource (id, school_id, name, code, quantity INTEGER, campus_id NULL)

academic_year (id, school_id, name, starts_on DATE, ends_on DATE)
term (id, academic_year_id -> academic_year, name, ordinal, starts_on, ends_on)
```

### Cycle, period structure, timeslot

```sql
-- A cycle is the repeating pattern. day_count = 5 weekly; 10 for a two-week cycle;
-- 6 for a six-day rotation that drifts against the calendar week. No upper bound.
-- day_count is the *declared* length: the materialiser uses the cycle_day rows that
-- actually exist, and the repository refuses an ordinal beyond it (ADR 0011 Q4).
cycle (id, school_id, name, day_count INTEGER, week_count INTEGER DEFAULT 1, is_default,
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

```sql
department (id, school_id, name, code)
teacher (id, school_id, code, given_name, family_name, email NULL,
         department_id NULL, home_campus_id NULL, fte REAL,
         max_periods_per_day INTEGER NULL, max_periods_per_cycle INTEGER NULL,
         max_consecutive_periods INTEGER NULL, notes,
         UNIQUE(school_id, code))
teacher_qualification (teacher_id, subject_id, PRIMARY KEY(teacher_id, subject_id))

year_level (id, school_id, name, ordinal)
student (id, school_id, code, given_name, family_name, year_level_id NULL,
         campus_id NULL, UNIQUE(school_id, code))

-- One table for every audience: a class, a whole cohort, a homeroom, an elective set.
student_group (id, school_id, name, code,
               kind TEXT CHECK (kind IN ('CLASS','COHORT','HOMEROOM',
                                         'ELECTIVE_SET','TEACHING_SET')),
               year_level_id NULL, parent_group_id -> student_group NULL,
               size INTEGER,                    -- declared size when membership is not modelled
               campus_id NULL)
student_group_member (student_group_id, student_id,
                      PRIMARY KEY(student_group_id, student_id))
```

`parent_group_id` gives the hierarchy that drives student clash detection: a cohort split
into three teaching sets means those sets conflict with the cohort but not with each
other. Clash detection resolves a group to its **leaf student set** — either real
`student_group_member` rows or, when membership is not modelled, the declared ancestry.
`size` exists so a school can timetable without entering a single student.

Student records are optional in Milestone 1 and exist so individual request-driven
scheduling can be added without a schema break. Fixtures use synthetic names only.

---

## 5. Curriculum

```sql
subject (id, school_id, name, code, colour, department_id NULL,
         default_room_type_id NULL, UNIQUE(school_id, code))

-- What is taught: "Year 9 Mathematics".
course (id, school_id, subject_id -> subject, name, code, year_level_id NULL,
        term_id NULL,                           -- NULL = runs all year
        periods_per_cycle INTEGER,
        default_duration_periods INTEGER DEFAULT 1,
        required_room_type_id NULL, credits REAL NULL)

-- A deliverable instance: "9MAT-A", one teacher set, one audience.
class_section (id, course_id -> course, name, code, term_id NULL, campus_id NULL,
               max_size INTEGER NULL, student_group_id -> student_group NULL)
class_teacher (class_section_id, teacher_id,
               role TEXT CHECK (role IN ('PRIMARY','CO','ASSISTANT')),
               PRIMARY KEY(class_section_id, teacher_id, role))
class_group (class_section_id, student_group_id,
             PRIMARY KEY(class_section_id, student_group_id))
enrolment (id, class_section_id, student_id, status,
           UNIQUE(class_section_id, student_id))

-- Individual demand, for request-driven (elective-line) scheduling later.
course_request (id, student_id, course_id, term_id NULL,
                kind TEXT CHECK (kind IN ('REQUIRED','ELECTIVE','ALTERNATE')),
                priority INTEGER, weight REAL,
                alternate_of_id -> course_request NULL)
```

Both scheduling traditions are representable: group-based (fill `student_group` and
`class_group`) and request-based (fill `course_request` and let a sectioning phase derive
`enrolment`). Milestone 1 solves group-based only — see
[OPEN-DECISIONS.md](OPEN-DECISIONS.md) D5.

---

## 6. Activities — the thing that gets scheduled

An **activity** is one placeable unit of work. Lessons are the common case; assemblies,
meetings, homeroom, sport and yard duties are the same shape, which is why they are not
special-cased.

```sql
activity (id, school_id, class_section_id -> class_section NULL,
          kind TEXT CHECK (kind IN ('LESSON','ASSEMBLY','MEETING','HOMEROOM',
                                    'SPORT','YARD_DUTY','EXAM','OTHER')),
          name, duration_periods INTEGER DEFAULT 1,
          required_capacity INTEGER NULL, required_room_type_id NULL,
          preferred_room_id NULL, needs_room BOOLEAN DEFAULT 1,
          term_id NULL, campus_id NULL,
          split_index INTEGER,                  -- 1..n of a course's periods_per_cycle
          is_locked BOOLEAN DEFAULT 0,
          created_at, updated_at, rev)

activity_teacher (activity_id, teacher_id, role,
                  PRIMARY KEY(activity_id, teacher_id, role))
activity_group (activity_id, student_group_id,
                PRIMARY KEY(activity_id, student_group_id))
activity_room_candidate (activity_id, room_id, PRIMARY KEY(activity_id, room_id))
activity_resource (activity_id, resource_id, quantity,
                   PRIMARY KEY(activity_id, resource_id))
```

Teachers and groups are **many-to-many from day one** — co-teaching and combined-cohort
lessons are normal, and retro-fitting them is expensive. The Java prototype's single
`teacherId` is exactly the shortcut to avoid.

### Duration

A double period is **one activity with `duration_periods = 2`** occupying consecutive
periods, not two linked activities. Simpler constraints, simpler UI, and it makes
"this must not be split" structural rather than a rule that can be switched off.

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

---

## 7. Scheduling

```sql
timetable (id, school_id, name, cycle_id, term_id NULL,
           status TEXT CHECK (status IN ('DRAFT','PUBLISHED','ARCHIVED')),
           based_on_timetable_id -> timetable NULL,  -- scenario / what-if lineage
           score_hard INTEGER, score_medium INTEGER, score_soft INTEGER,
           generated_at, solver_seed, solver_version,
           created_at, updated_at, rev)

timetable_entry (id, timetable_id -> timetable ON DELETE CASCADE,
                 activity_id -> activity, timeslot_id -> timeslot,
                 room_id -> room NULL,
                 is_locked BOOLEAN DEFAULT 0,       -- solver must preserve
                 is_manual BOOLEAN DEFAULT 0,       -- placed by a human
                 UNIQUE(timetable_id, activity_id, timeslot_id))
timetable_entry_teacher (timetable_entry_id, teacher_id)  -- overrides activity_teacher

constraint_instance (id, school_id, type_key TEXT NOT NULL,
                     level TEXT CHECK (level IN ('HARD','MEDIUM','SOFT')),
                     weight REAL NOT NULL DEFAULT 1,
                     enabled BOOLEAN NOT NULL DEFAULT 1,
                     params TEXT NOT NULL DEFAULT '{}',   -- JSON, schema-validated
                     term_id NULL, timetable_id NULL,     -- NULL = project-wide
                     created_at, updated_at, rev)
constraint_ref (constraint_id -> constraint_instance ON DELETE CASCADE,
                role TEXT,                               -- 'subject', 'teacher', 'appliesTo'
                entity_kind TEXT, entity_id TEXT,
                PRIMARY KEY(constraint_id, role, entity_id))

solve_run (id, timetable_id, status, engine, seed, settings TEXT,
           started_at, finished_at,
           best_hard, best_medium, best_soft, steps, log)
violation (id, timetable_id, solve_run_id NULL, constraint_id NULL,
           constraint_type, level, penalty REAL, message)
violation_ref (violation_id, entity_kind, entity_id, role)
```

`UNIQUE(timetable_id, activity_id, timeslot_id)` rather than
`(timetable_id, activity_id)` because a multi-period activity occupies several slots; the
entry rows for one activity are written and moved as a unit.

### Why constraints store parameters as JSON

There are 40+ constraint families and the list grows. A column per family means a
migration per constraint; a table per family means 40 repositories. Instead:
`type_key` + JSON `params`, with the code-side registry publishing a **JSON Schema** per
type that both validates writes and generates the configuration form.

The cost is that entity references inside JSON get no foreign key. `constraint_ref`
exists to pay that back: scope references are normalised into real rows with a cascade,
and `params` holds only scalars. See [CONSTRAINTS.md](CONSTRAINTS.md).

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

```sql
CREATE INDEX idx_entry_timetable_slot ON timetable_entry(timetable_id, timeslot_id);
CREATE INDEX idx_entry_timetable_act  ON timetable_entry(timetable_id, activity_id);
CREATE INDEX idx_entry_room           ON timetable_entry(timetable_id, room_id, timeslot_id);
CREATE INDEX idx_act_teacher_teacher  ON activity_teacher(teacher_id);
CREATE INDEX idx_act_group_group      ON activity_group(student_group_id);
CREATE INDEX idx_avail_entity         ON availability(entity_kind, entity_id);
CREATE INDEX idx_timeslot_cycle       ON timeslot(cycle_id, ordinal);
CREATE INDEX idx_change_log_seq       ON change_log(entity_table, entity_id, seq);
```

The solver does not query SQLite in its inner loop — it loads the whole problem into
flat, index-addressed arrays once. These indexes serve the UI and the loader.
