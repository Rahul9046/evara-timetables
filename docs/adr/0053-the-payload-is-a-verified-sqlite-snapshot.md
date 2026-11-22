# ADR 0053 — A transfer archive carries a verified SQLite snapshot, not a directory of JSON

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal approved decision 1, D1
- Supersedes: [ADR 0004](0004-evara-project-format.md)'s **container decision only**. Every
  other clause of 0004 survives — the A/B split, `formatVersion`'s inviolable meaning,
  `capabilities`, UUID and provenance preservation, the atomic finalise by rename, and the
  rule that additive changes do not bump the version
- Builds on: [ADR 0003](0003-sqlite-document-model.md) (SQLite is the canonical store),
  [ADR 0007](0007-rusqlite-refinery.md) (forward-only embedded migrations, one connection
  factory)
- Binds: `evara_project::{seal, inspect, import}`, `evara_db::transfer::{snapshot, stage}`,
  `manifest::PAYLOAD_KIND`
- Corrects: [PROJECT-FORMAT.md](../PROJECT-FORMAT.md), rewritten in full

## Context

ADR 0004 specified a ZIP of JSON sections — `data/structure.json`, `data/people.json`, and
four more — and explicitly rejected the alternative it called *"a SQLite snapshot inside the
ZIP alongside the manifest"*, on the grounds that it *"inherits the migration burden, and
tempts readers to query the snapshot directly instead of going through the migration chain"*.

Both halves of that objection turn out to be wrong, and the first is wrong in an interesting
way: **the migration burden is not inherited, because it is already owned.** `CLAUDE.md` §3
requires a released migration to be immutable and forbids down migrations; refinery checksums
every applied migration, so the chain that opens a V3 `.evaraproj` written in 2026 must keep
working for the life of the product regardless of what the transfer format does. A snapshot
payload reuses exactly that chain. A JSON container would have needed a **second**, parallel
migration mechanism — one pure function per `formatVersion` step over parsed sections, with a
permanent fixture per version — to express migrations the project already performs.

The second half is an argument against a mistake nobody is required to make: the payload is
migrated by `migrations::run` before anything reads a row out of it (ADR 0056), which is
*going through the chain*.

Meanwhile the JSON design's real cost became visible as the model grew. M10 must carry fifty
tables in four groups. A section list is a thing a contributor can forget to add a table to,
and the failure mode is silent: the export succeeds, the import succeeds, and a school
discovers eighteen months of availability rules did not travel.

## Decision

**A `.evara` is a ZIP of exactly three entries, and the payload is a fully checkpointed,
verified copy of the project database.**

```
Northgate.evara
├── manifest.json                  STORED
├── checksums.json                 DEFLATE
└── payload/project.evaraproj      DEFLATE
```

### 1. Completeness becomes structural rather than vigilant

There is no section list, so there is nothing to forget. Every table, every row, every
`created_at`, `updated_at` and `rev`, every timetable version, lock, manual placement,
repair-lineage edge, stored evaluation and finding, and the **complete** change feed travel
because they are in the file being copied.

This is also why `change_log` travels, reversing another of 0004's clauses: not because the
decision was revisited on its merits, but because excluding it from a page-level copy would
mean post-processing the snapshot — turning a verified copy back into a curated extract and
giving up the property this decision is for. The feed measures 26.2% of a realistic project's
occupied pages, which is real and not dominating, and it deflates with everything else.

### 2. Verification is stronger than parsing

`PRAGMA integrity_check` and `PRAGMA foreign_key_check` on the staged payload are a statement
about structure *and* references. The JSON design's equivalent was *"the JSON parsed"* plus
whatever the insert transaction's foreign keys caught on the way in — which is a check of the
importer, not of the archive.

### 3. The snapshot is taken by the backup API, in one pinned read transaction

A project runs in WAL, so copying the main file yields a database that opens cleanly and
silently lacks the most recent work. Whatever mechanism is used must produce a **single fully
checkpointed file**.

The approved design specified two mechanisms — `VACUUM INTO` from a read-write handle and the
online backup API from a `query_only` one, the latter because `VACUUM INTO` is *refused* under
`query_only`. The implementation uses the **backup API for both**, and the reason is
cancellation: `VACUUM INTO` is one opaque statement, interruptible only through SQLite's
progress handler, whose closure rusqlite requires to be `Send + 'static` — and a borrowed
`&dyn Cancel` is neither. Honouring the split literally would have shipped a desktop export
that cannot be cancelled, on the one path where a user is most likely to want to.

What is given up is stated rather than hidden: the backup API reproduces the source page for
page, free pages included, so a snapshot is **+6.9%** over a vacuumed copy. The archive's
compression absorbs 88.7% of that, and no compaction pass is added.

### 4. `payload.kind` is declared, which preserves 0004's hedge

The manifest says `"kind": "sqlite-snapshot"` rather than leaving it implicit. ADR 0004's
decoupling argument — *"if the local store ever changes, the transfer format does not
break"* — survives as a **versioned fact**: a future `formatVersion 2` can add
`"json-sections"` and read both, and this build will not have to guess what it is looking at.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| ADR 0004's `data/*.json` sections | A section list is a place to forget a table, and the failure is silent. It also needs a second migration mechanism for migrations the project already performs |
| A snapshot with the change feed stripped | Post-processing a verified copy gives up the property this decision exists for. The feed is 26.2% of occupied pages and deflates with the rest; `--without-history` is deferred, with that number rather than a gap (OPEN-DECISIONS D37) |
| Rename the `.evaraproj` to `.evara` | No manifest, so no `formatVersion`, no `capabilities`, no digest and no way to refuse an archive this build cannot read before opening it. It is also not checkpointed: the `-wal` would be left behind |
| `VACUUM INTO` for the snapshot, as approved | §3. It cannot be cancelled through a borrowed `Cancel`, and an uncancellable export was the worse trade |
| A compaction pass after the backup | Measured at +6.9% inflation, 88.7% of which the archive absorbs. Adding a second full copy of the database to recover that is not worth it |

## Consequences

- **The payload migration chain is the project's own.** No `formatVersion` migration exists
  at v1, and the container chain is deliberately empty.
- **A `.evara` is not diffable or inspectable by eye**, which 0004 listed as a benefit of
  JSON. `evara inspect` replaces it for the question people actually ask — *what is in this
  file and can this build read it?* — without reading the payload at all.
- **No fixture archive is checked in.** 0004's *"permanent, never-edited test fixture per
  historical `formatVersion`"* is replaced by a **generator**: synthetic projects are built at
  any schema version and sealed in-test, so no binary enters git. See
  [ROADMAP.md](../ROADMAP.md) Phase 9 for what that can and cannot cover.
- **An archive is larger than a vacuumed copy and smaller than the JSON equivalent would
  have been.** §7.2's budget is measured, not estimated.
- **Round-trip equality is asserted as a census and an identity comparison**, not as
  byte-identical archives: two snapshots of one project are not byte-identical, because the
  backup API copies live pages.
