# ADR 0004 — `*.evara` is a versioned ZIP of JSON, separate from external interchange

- Status: **Accepted**
- Date: 2026-10-06

## Context

Two distinct requirements exist and must not be conflated:

- **A. Project transfer.** Export a complete Evara project and import it on another
  installation, with format versioning so future versions can migrate old exports.
- **B. External data interchange.** Eventually import from PowerSchool, Veracross, aSc,
  TTS, Edval, TimeTabler, CSV and LISS.

A is a portable snapshot of *our own* model and must be supported for years. B is a set of
adapters for other people's models. They share almost no requirements.

## Decision

`*.evara` is a **ZIP archive** containing `manifest.json`, a `data/` directory of
**JSON** sections, optional `attachments/`, and `checksums.json` with SHA-256 per entry.
Full specification in [PROJECT-FORMAT.md](../PROJECT-FORMAT.md).

- `manifest.json` carries an integer `formatVersion` whose meaning may never change,
  because every future reader must parse it before knowing how to read anything else. It
  is independent of both the app version and the database schema version.
- A `capabilities` array lets an older app refuse an import it would silently mangle,
  naming the missing capability.
- UUIDs, `created_at`, `updated_at` and `rev` are preserved, not regenerated, so identity
  survives transfer and round-trip comparison is meaningful.
- `change_log` is not exported — it is installation-local and would dominate file size.
- Import runs a chain of pure format migrations `N → N+1 → … → current`, then inserts into
  a **fresh** project database inside one transaction with foreign keys on.
- Export finalises to a temporary path and renames atomically, so an interrupted export
  never leaves a plausible-looking corrupt file.
- Additive changes do not bump `formatVersion`; renames, removals, type changes and
  semantic changes do.

External interchange (B) lives in `crates/evara-integrations/`, depends only on
`evara-core`, produces core types, and is feature-gated. Nothing in the core references a
vendor. None of it is implemented now.

## Consequences

- A v1 export must open in a v5 app, which means a permanent, never-edited test fixture
  per historical `formatVersion` and a migration step for each.
- JSON is larger than a database copy; deflate and streaming writes keep this acceptable,
  and a large school must never be fully buffered in memory.
- JSON being inspectable and diffable means bug reports can include an export and
  fixtures can live in git under review.
- Round-trip equality (export → import → export, byte-identical after normalisation)
  becomes a CI gate on every fixture.
- A raw SQLite copy remains available as a separate "backup" action — a different feature
  with a different and weaker promise: exact, same-version-only.

## Alternatives considered

**Rename/copy the SQLite file as `.evara`.** Trivially exact, and that is its only
advantage. Supporting old exports would mean carrying every historical schema and its
migrations forever, and it couples the transfer format to the storage engine.

**A SQLite snapshot inside the ZIP alongside the manifest.** Keeps versioning but
inherits the migration burden, and tempts readers to query the snapshot directly instead
of going through the migration chain.

**A single large JSON file.** Simpler, but no attachments, no streaming per section, and
worse diffs.

**Protobuf / MessagePack / SQLite-backed binary.** Smaller and faster, but opaque at
exactly the moment it matters — diagnosing a user's broken project file.
