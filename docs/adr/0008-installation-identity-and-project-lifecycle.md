# ADR 0008 — Installation identity, the settings database, and `.evaraproj`

- Status: **Accepted** (approved 2026-10-06, resolves OPEN-DECISIONS D15, D16, D17, D18)
- Date: 2026-10-06
- Amends: [ADR 0003](0003-sqlite-document-model.md)

## Context

Phase 1A stored `site_id` inside the project file. That conflated two different things:
the identity of a *school project* and the identity of an *Evara installation*. The
consequence was concrete — open a colleague's project and Evara adopted their identity,
so the change feed attributed your edits to them.

Phase 1B also had to settle what a live project file is called, and who keeps `rev` and
`updated_at` up to date.

## Decision

### Identity is split in three

| Identifier | Lives in | Lifetime |
| --- | --- | --- |
| `project_id` | the project file | created once, travels with the file, survives export/import |
| `created_by_site_id` | the project file | historical fact, written at creation, **never updated** |
| `site_id` | the **settings database** | per installation, created on first run, never changes |

A project never records who is *currently* using it.

### The installation is supplied per connection

The change-log triggers call an **application-defined SQL function**, `evara_site_id()`,
registered on each connection by the connection factory.

This was not the first design. A per-connection `TEMP` table was tried and SQLite rejects
it outright: *"trigger ... cannot reference objects in database temp"*. A real table in the
project would have made per-connection state persistent and shared, which is the bug being
fixed. A function is genuinely connection-scoped, requires no writes, and has a useful
failure mode: a connection without it **cannot write to a tracked table at all** (*"no such
function: evara_site_id"*). An unattributable change is worse than a refused one, and this
also stops another tool quietly inserting rows the change feed would then misreport.

### A second database for settings

Application settings live in their own SQLite database in the platform application data
directory, with its own migration set. It holds exactly two things: the installation
`site_id` and the recent-project list. No speculative settings — a setting nothing reads is
a schema change nobody can safely remove later. It never contains school data.

### `.evaraproj` for live projects

The live, editable project uses `.evaraproj`. `.evara` is reserved for the portable export
package ([ADR 0004](0004-evara-project-format.md)). `Workspace` rejects every other
extension, `.evara` included, so the two can never be confused: one is a working database
with WAL sidecars, the other a self-contained archive.

### Repositories maintain `rev` and `updated_at`

Not triggers. A trigger updating its own table interacts with SQLite's
`recursive_triggers` setting in ways that are easy to get subtly wrong, and the failure
mode — a change logged twice, or not at all — is precisely what the change feed exists to
prevent. Change-log triggers only *observe* what the repository wrote.

### The lifecycle lives in `evara-db`

`Workspace` owns the settings database and at most one open project. It sits in `evara-db`
because it coordinates two SQLite databases and SQL belongs in one crate. The Tauri command
layer stays a translation of requests into calls, and the whole lifecycle is testable
without a window.

## Consequences

- **Migration V2 performs the correction**: it renames `site_id` to `created_by_site_id`
  in `app_meta`, backfills `'unknown'` where provenance was never recorded, and deletes any
  leftover key. V1 is untouched, honouring the immutable-migration rule.
- `created_by_site_id` is `Option<Uuid>`: a project migrated from Phase 1A may legitimately
  have no recorded provenance, and `'unknown'` is more honest than a fabricated identifier.
- Opening a project read-only from a tool without `evara_site_id()` works; *writing* to a
  tracked table does not. This is intended, and it needs to be documented wherever external
  tooling is discussed.
- The settings database is a second thing that can fail to open. `Workspace::new` fails
  fast at startup rather than on first use, because an installation without an identity
  cannot attribute anything.
- Two migration sets now exist (`migrations/` and `settings-migrations/`), each with its
  own refinery history table in its own database.
- Recent-project paths are deduplicated by a normalised key — lowercased on Windows,
  because its filesystem is case-insensitive — while the original spelling is kept for
  display. `canonicalize` is deliberately not used: it fails for a deleted file, which is
  exactly the case the recent list must survive.

## Alternatives considered

**Keep `site_id` in the project, overwrite it on open.** Simplest, and destroys the
provenance of every project you open. Rejected.

**Store the active `site_id` in a project table written at open time.** Works for a single
connection, but makes session state persistent and shared, and dirties the file on every
open. It is the same category error as Phase 1A, one level down.

**Pass `site_id` as a parameter to every write.** Correct but unenforceable: one repository
forgetting it leaves a silent hole in the change feed, which is the thing triggers exist to
prevent.

**A `.evara` extension for both live projects and exports.** One fewer concept, and a
user would eventually mail someone a live WAL-mode database and be surprised when it did
not open cleanly.
