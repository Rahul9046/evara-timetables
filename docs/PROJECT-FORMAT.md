# The `*.evara` Project Format

A single file that carries a complete Evara school project between installations.
Export on one machine, import on another, no network involved.

This is **Requirement A: project transfer**. It is deliberately separate from
**Requirement B: external data interchange** (PowerSchool, Veracross, aSc, TTS, Edval,
TimeTabler, CSV, LISS), which lives in `crates/evara-integrations/` and is not
implemented yet. Mixing the two is the mistake to avoid: a portable snapshot of our own
model has entirely different requirements from an adapter for someone else's.

---

## 1. Container

A ZIP archive (deflate) with the extension `.evara`.

```
school-project.evara
├── manifest.json            required, always first entry, never compressed
├── data/
│   ├── structure.json       school, campus, building, room, room_type, resource,
│   │                        academic_year, term, cycle, cycle_day,
│   │                        period_structure, period, timeslot, calendar_day
│   ├── people.json          department, teacher, year_level, student, student_group, …
│   ├── curriculum.json      subject, course, class_section, enrolment, course_request
│   ├── activities.json      activity, links, lines, availability
│   ├── constraints.json     constraint_instance + constraint_ref
│   └── timetables.json      timetable, timetable_entry, solve_run summaries
├── attachments/             optional: school logo, report templates, user notes
└── checksums.json           SHA-256 per entry, for integrity
```

### Why a ZIP of JSON and not a copy of the SQLite file

Copying the database would be trivially exact, and that is its only advantage. JSON wins
on the properties that matter for a format we must support for years:

- **Migratable.** A v1 export must open in a v5 app. Migrating JSON through a chain of
  transforms is routine; migrating an arbitrary old SQLite snapshot means carrying every
  historical schema and its migrations forever.
- **Inspectable and diffable.** Bug reports can include an export. Fixtures can live in
  git and be reviewed.
- **Decoupled from storage.** If the local store ever changes, the transfer format does
  not break.
- **Extensible.** Attachments and future sections slot in without touching the schema.

A raw SQLite copy remains available as a separate "backup" action, which is a different
feature with a different promise (exact, same-version-only).

---

## 2. Manifest

```json
{
  "format": "evara-project",
  "formatVersion": 1,
  "generator": { "app": "Evara Timetables", "version": "0.2.0" },
  "projectId": "018f3a2c-7c4e-7a19-9f2b-3c1d5e7a9b11",
  "siteId": "018f3a2c-7c4e-7a19-9f2b-000000000001",
  "exportedAt": "2026-10-06T09:14:22.113Z",
  "dbSchemaVersion": 7,
  "school": { "name": "Northgate Secondary (synthetic)", "campusCount": 2 },
  "counts": {
    "teachers": 84, "rooms": 61, "studentGroups": 47,
    "activities": 1320, "constraints": 29, "timetables": 3
  },
  "sections": ["structure", "people", "curriculum", "activities",
               "constraints", "timetables"],
  "capabilities": ["lines", "multi-campus", "multi-week-cycle"],
  "integrity": { "algorithm": "sha256", "file": "checksums.json" }
}
```

`formatVersion` is the **only** field whose meaning may never change, because every
future reader must parse it before it knows how to read anything else. It is an integer,
incremented on every breaking change, and is independent of both the app version and
`dbSchemaVersion`.

`capabilities` lets an older app refuse an import it would silently mangle: *"This
project uses multi-week cycles, which this version does not support. Update Evara to
open it."* That is a much better failure than a half-imported school.

---

## 3. Entity encoding

```json
{
  "section": "people",
  "formatVersion": 1,
  "teacher": [
    {
      "id": "018f3a2c-7c4e-7a19-9f2b-4a1b2c3d4e5f",
      "code": "T014",
      "givenName": "Ada",
      "familyName": "Teacher",
      "fte": 1.0,
      "maxPeriodsPerDay": 5,
      "departmentId": "018f3a2c-...",
      "createdAt": "2026-08-01T04:00:00.000Z",
      "updatedAt": "2026-09-30T11:22:01.907Z",
      "rev": 4
    }
  ],
  "teacherQualification": [
    { "teacherId": "018f3a2c-...", "subjectId": "018f3a2c-..." }
  ]
}
```

Rules:

- **UUIDs are preserved**, not regenerated. A project re-imported keeps its identity,
  which keeps future synchronisation and round-trip comparison possible. Importing *into*
  an existing project (a merge) is a separate, later feature that will remap; the default
  import always creates a new project file.
- `createdAt` / `updatedAt` / `rev` travel with the row. Provenance survives transfer.
- `camelCase` in JSON, `snake_case` in SQL. One mapping, applied by serde attributes in
  one place.
- Enum values are the SQL `CHECK` strings verbatim (`'HARD'`, `'TEACHING'`), so there is
  no second vocabulary to keep in step.
- `change_log` is **not** exported. It is installation-local history, it would dominate
  file size, and it is meaningless on another machine. `site_id` in the manifest records
  where the export came from.
- Derived data is not exported where it is cheap to recompute. `violation` rows are
  rebuilt on first score; `timeslot` *is* exported because locked entries reference its
  IDs.

---

## 4. Export

1. Open a read transaction so the snapshot is consistent.
2. Stream each section to a temporary file — a large school must not be fully buffered
   in memory.
3. Compute SHA-256 per entry into `checksums.json`.
4. Write `manifest.json` first, then sections, then attachments.
5. Finalise to a temporary path and rename atomically, so an interrupted export never
   leaves a plausible-looking corrupt `.evara`.

## 5. Import

1. Read and validate `manifest.json`. Reject unknown `format`, a `formatVersion` newer
   than supported, or an unsupported entry in `capabilities` — each with a specific
   message.
2. Verify checksums. Refuse a corrupt archive rather than importing part of it.
3. Run the **format migration chain** `formatVersion N → N+1 → … → current`. Each step is
   a pure function over the parsed sections, in `crates/evara-project/src/migrate/`, with
   a test fixture for every version it can read.
4. Create a fresh project database at the current schema version.
5. Insert in dependency order inside **one transaction**, with `foreign_keys = ON`, so a
   referential problem aborts the whole import.
6. Rebuild derived state (timeslot integrity, score, violations).
7. Report a summary: counts inserted, anything skipped and why.

Import never writes into the currently open project.

---

## 6. Compatibility policy

| Situation | Behaviour |
| --- | --- |
| Export `formatVersion` < current | Migrate forward silently; note the upgrade in the import summary. |
| Export `formatVersion` = current | Import directly. |
| Export `formatVersion` > current | Refuse, naming the app version needed. |
| Unknown `capabilities` entry | Refuse, naming the capability. |
| Unknown object field | Ignore, but record it in the summary — forward tolerance for additive changes. |
| Unknown section | Ignore with a warning. |
| Checksum mismatch | Refuse. |

Additive changes (a new optional field, a new section) do **not** bump `formatVersion`.
Renames, removals, semantic changes and type changes do.

---

## 7. Tests this format must have

- **Round-trip equality.** export → import → export, and the two archives match after
  normalisation (ordering, timestamps in the manifest). Every synthetic fixture, in CI.
- **Version migration.** A checked-in `v1` archive imports cleanly into the current app,
  for every historical `formatVersion`. These fixtures are permanent and never edited.
- **Corruption.** Truncated archive, bad checksum, missing manifest, missing section,
  bad UUID, dangling foreign key — each fails with a specific error and leaves no
  partial project behind.
- **Scale.** A synthetic 2,000-activity school exports and imports within a sane time and
  memory budget.
- **Determinism.** Two exports of an unchanged project are byte-identical apart from
  `exportedAt`.

All fixtures are synthetic. No real school, teacher or student data in the repository,
ever — including in bug-report attachments committed to git.
