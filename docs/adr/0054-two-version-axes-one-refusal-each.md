# ADR 0054 — Two version axes, and each one refuses on its own terms

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal approved decision 4, D2
- Amends: [ADR 0004](0004-evara-project-format.md), which made the two numbers independent
  without saying which one refuses what
- Builds on: [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) (the payload is a
  database, so it has a schema version of its own),
  [ADR 0007](0007-rusqlite-refinery.md) (forward-only migrations, typed newer-schema refusal)
- Binds: `evara_project::manifest::{FORMAT_VERSION, PayloadDescriptor}`,
  `evara_project::reader`, `evara_db::supported_schema_version`

## Context

ADR 0004 said `formatVersion` *"is independent of both the app version and the database schema
version"*, which is correct and insufficient. Independence says the numbers do not move
together; it does not say what either one does when it is too high, and once the payload is a
database (ADR 0053) there are genuinely two numbers that can be too high for different
reasons.

A user who is refused an import needs to know **which** number refused them, because the
remedy differs in kind even where the sentence looks the same. "This archive was written by a
newer Evara" and "the project inside was saved by a newer Evara" are different facts about
their file, and one of them can be true while the other is not: a v1 container can carry a
V99 payload, and a hypothetical v2 container could carry a V18 one.

## Decision

**Two axes, each with its own refusal, and neither is ever inferred from the other.**

| Axis | Means | Too new | Older |
| --- | --- | --- | --- |
| `manifest.formatVersion` | the **container**: entry layout, manifest schema, payload kind | refused as `FormatTooNew`, naming the version needed | the container migration chain — empty at v1 |
| `manifest.payload.schemaVersion` | the **data**: which migration the payload stands at | refused as `SchemaTooNew` | the staged copy is migrated forward by the project's own chain |

### 1. `formatVersion` is parsed before anything else, and its meaning is frozen

Every future reader must read it before it knows how to read anything else, so its *meaning*
may never change — ADR 0004's rule, kept verbatim. It is checked at step 4 of the verification
order, immediately after the manifest parses and before `checksums.json` is even located.

### 2. The schema axis is refused **before a payload byte is written**

Step 7, two steps before the payload is streamed. An archive whose data this build cannot
understand is refused without staging anything, which is the difference between an import that
fails and an import that fails having left a half-staged project behind.

This is also the cheap half of the check. The manifest's copy of the schema version exists for
exactly this: to refuse early, without reading 60 MiB first.

### 3. The payload is the authority, and a disagreement is itself a refusal

The manifest's `payload.schemaVersion` is a **claim**; the payload's own
`refinery_schema_history` is the fact. They are compared after staging — the only point at
which the payload can be asked — and a mismatch is `ManifestDisagreesWithPayload`.

That refusal is not about version support at all. Evara assembles both numbers from the same
snapshot, so they cannot disagree in an archive Evara wrote: a disagreement means the archive
was **edited by hand**, and reading on would mean acting on a description somebody adjusted.

### 4. Older is migrated, never refused — and never reversed

A released migration is immutable and there are no down migrations, so the asymmetry is
inherent rather than a policy choice: forward always, backward never. An older payload is
migrated **in staging**, so the user's own file is never the thing being upgraded by an
import.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| One version number for both | They move for unrelated reasons. A manifest-schema change would force every project to look newer, and a `V19` migration would force the container to look newer to builds that read it fine |
| Derive the schema axis from `formatVersion` | Only possible if the two are locked together, which is §1's problem; and the payload would still have to be asked, because its own history is the authority |
| Read the schema version from the payload only, with nothing in the manifest | Then the refusal arrives *after* a full payload stream, having written to staging. The manifest copy buys an early, cheap no |
| Treat a manifest/payload disagreement as advisory, like a count | A count is a convenience and the payload is the authority, so a count disagreement is reportable. A *version* disagreement means the description was edited, which is a different claim about the file |
| Refuse an older payload and require the sender to upgrade first | Hostile, and it would make a `.evara` useless as an archive format: the whole point is that a file written years ago still opens |

## Consequences

- **`FormatTooNew` and `SchemaTooNew` are distinct error variants** all the way to the
  interface, and the frontend's `isTooOldToRead` predicate covers both while the messages name
  which.
- **`inspect` reports a version refusal as a value, not an exception** (`Readability`), because
  deciding whether to *offer* an import is a question whose "no" a screen has to render.
- **The container migration chain exists and is empty.** That is deliberate: the hook is
  present and tested as a no-op, so a v2 does not have to invent it under pressure.
- **A V1…V18 payload is migration-tested** through the project path. The archive path can only
  be tested for schema versions a manifest can *describe* — see
  [ROADMAP.md](../ROADMAP.md) Phase 9's T-41 note, which is the honest limit of that coverage.
