# ADR 0004 — `*.evara` is a versioned ZIP of JSON, separate from external interchange

- Status: **Accepted** — the **container decision is superseded in part** by
  [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) and the version axes are made
  precise by [ADR 0054](0054-two-version-axes-one-refusal-each.md)
- Date: 2026-10-06

> ## What of this record still stands
>
> **The container's contents changed; nothing else did.** `*.evara` is still a versioned ZIP
> with a `manifest.json` and a `checksums.json`, still separate from external interchange, and
> every clause below about `formatVersion`'s inviolable meaning, `capabilities`, UUID and
> provenance preservation, the atomic finalise by rename, and additive-versus-breaking changes
> survives unaltered.
>
> Four clauses are superseded, each by a record that says why:
>
> | Here | Actually | Record |
> | --- | --- | --- |
> | a `data/` directory of JSON sections, and optional `attachments/` | one entry, `payload/project.evaraproj`, carrying a verified SQLite snapshot. `attachments/` is removed from v1 and reserved by name — a school logo is a `school_logo` row (ADR 0016) | [0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) |
> | *"`change_log` is not exported"* | **it travels complete.** Excluding it from a page-level copy would mean post-processing a verified snapshot, giving up the property the snapshot is for. Measured at 26.2% of a realistic project's occupied pages | [0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) §1 |
> | a chain of pure `formatVersion` migrations, then inserts into a fresh database in one transaction | the payload is migrated **in staging** by the project's own migration chain, then **renamed** into place. Nothing is inserted row by row | [0057](0057-an-import-is-staged-verified-and-renamed.md) |
> | *"rebuild derived state (timeslot integrity, score, violations)"* | **an import rebuilds nothing.** Re-scoring needs a compiled `Problem`, which can legitimately fail for a school mid-setup, and it would write a `REVALIDATION` row where the archive carries an immutable `GENERATION` one. Stored findings travel in the payload | [0057](0057-an-import-is-staged-verified-and-renamed.md) §3 |
>
> The *"Alternatives considered"* entry that rejected a SQLite snapshot is the one ADR 0053
> argues with directly: the migration burden it names is one `CLAUDE.md` §3 already requires
> the product to carry, so it is not inherited by this choice.
>
> [PROJECT-FORMAT.md](../PROJECT-FORMAT.md) documents the implemented format.

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
