# The `*.evara` project transfer format

One file that carries a complete Evara project between installations. Export on one machine,
import on another, no network involved — and nothing in the path makes a request, opens a port
or starts a process (ADR 0001).

This is **Requirement A: project transfer** — a portable snapshot of Evara's own model. It is
deliberately separate from **Requirement B: external data interchange** (PowerSchool, Veracross,
aSc, Timetabling Solutions, Edval, TimeTabler, CSV, LISS), which lives in
`crates/evara-integrations` and is not implemented. Conflating the two is the mistake this split
exists to prevent: an exact copy of our own model has entirely different requirements from an
adapter for somebody else's.

Implemented in M10. The authoritative design record is
[M10-PROJECT-TRANSFER-PROPOSAL.md](M10-PROJECT-TRANSFER-PROPOSAL.md), whose §0.2 is the
replacement specification; the decisions are
[ADR 0053](adr/0053-the-payload-is-a-verified-sqlite-snapshot.md)–[0059](adr/0059-archive-integrity-is-corruption-not-authenticity.md).

---

## 0. The two objects, and they are never confused

| Extension | What it is | Owned by |
| --- | --- | --- |
| `.evaraproj` | the **live project**: a SQLite database Evara opens, migrates and writes | `evara-db` |
| `.evara` | the **portable export**: a ZIP carrying a verified snapshot of one | `evara-project` |

`CLAUDE.md` §3 forbids using one where the other belongs and forbids a third. A spreadsheet is
neither: `.xlsx` and `.csv` are read by `evara-import` and are never written as a project.

On screen the live file is a **campus file** and the archive is a **portable campus file**; the
code says `project` throughout. That is vocabulary, not architecture — renaming the model would
mean a migration and a format change for a wording decision.

---

## 1. Container

A ZIP archive with the extension `.evara`, holding **exactly three entries** in
`formatVersion 1`:

```
Northgate.evara                        ZIP
├── manifest.json                      STORED   — what the archive says about itself
├── checksums.json                     DEFLATE  — one digest per covered entry
└── payload/project.evaraproj          DEFLATE  — a verified SQLite snapshot
```

`manifest.json` is stored uncompressed so a reader reaches it without inflating anything.

**The payload is a snapshot of the project database, not a directory of JSON sections.** That
reverses ADR 0004's container decision and is recorded in
[ADR 0053](adr/0053-the-payload-is-a-verified-sqlite-snapshot.md). The short version: ADR 0004
rejected a snapshot because it *"inherits the migration burden"*, and that burden is one
`CLAUDE.md` §3 already requires us to carry for the life of the product — a released migration
is immutable and there are no down migrations, so the chain that opens an old `.evaraproj` is the
same chain that opens an old payload. What the snapshot buys is that **completeness is structural
rather than vigilant**: there is no section list to forget a table from, and
`PRAGMA integrity_check` plus `PRAGMA foreign_key_check` are a far stronger statement than *"the
JSON parsed"*.

So a `.evara` carries **everything**: every school, every timetable version, every manual edit,
every lock, every repair-lineage edge, every stored finding and the complete change feed. There is
no table a reader has to remember to add.

**`attachments/` is removed from v1 and reserved by name.** A school logo is a `school_logo` row
(ADR 0016), so it travels inside the payload like every other row; a separate attachments tree
would have been a second place for the same bytes.

Three entries is not a hard cap on what a *reader* tolerates. An archive from a newer build may
carry additional safe entries; this build ignores them and reports their names, which is ADR
0004's forward tolerance applied to the container.

---

## 2. Manifest

```json
{
  "format": "evara-project",
  "formatVersion": 1,
  "payload": {
    "kind": "sqlite-snapshot",
    "entry": "payload/project.evaraproj",
    "schemaVersion": 18,
    "byteLength": 3956736
  },
  "generator": { "app": "Evara Timetables", "version": "0.1.0" },
  "projectId": "018f3a2c-7c4e-7a19-9f2b-3c1d5e7a9b11",
  "createdBySiteId": "018f3a2c-7c4e-7a19-9f2b-000000000001",
  "exportedBySiteId": "018f3a2c-7c4e-7a19-9f2b-000000000002",
  "exportedAt": "2026-11-22T09:14:22.113Z",
  "changeLog": { "entries": 4306, "watermark": 4306 },
  "schools": [{ "id": "018f…", "name": "Northgate Secondary", "campuses": 2 }],
  "counts": { "teacher": 84, "room": 61, "activity": 1320 },
  "capabilities": ["constraint:no-clash-teacher", "cycle-kind:WEEKLY", "lock:placement"],
  "integrity": { "algorithm": "sha256", "entry": "checksums.json", "sha256": "9f2b…" }
}
```

| Field | Acted on? | Notes |
| --- | --- | --- |
| `format` | **yes** | anything but `evara-project` is not an Evara archive |
| `formatVersion` | **yes** | the container axis — §7 |
| `payload.kind` | **yes** | `sqlite-snapshot` is the one kind v1 defines. Declared rather than assumed, so a future v2 can add another and read both |
| `payload.schemaVersion` | **yes** | the data axis — §7. A disagreement with the payload is a refusal |
| `payload.byteLength` | **yes** | bounds the payload read before any digest is known |
| `capabilities` | **yes** | §8 |
| `integrity.algorithm` | **yes** | exactly `sha256`; anything else is refused rather than guessed at |
| `projectId` | no | carried, never regenerated — §9 |
| `createdBySiteId` / `exportedBySiteId` | no | kept **distinct**: the exporting installation never becomes the creator, or provenance would be rewritten by whoever last passed the file on |
| `counts`, `changeLog`, `schools`, `generator`, `exportedAt` | no | description and diagnostics. A count that disagrees with the payload is **reported, not refused**: the payload is the authority |

Two properties are load-bearing:

- **`formatVersion`'s meaning may never change**, because every future reader parses it before it
  knows how to read anything else.
- **`payload.sha256` is deliberately absent.** It lives in `checksums.json` only, so one fact has
  one home. Recording it in both and having them verify each other is a cycle: neither can be
  checked before the other, so in practice neither is.

Unknown manifest fields are **kept and ignored**, so an archive a newer build wrote still reads.
Nothing the reader acts on is tolerant.

---

## 3. Integrity — two levels, one root

```
manifest.json          ← the deliberately unverified root
  └── integrity.sha256 → checksums.json
                           └── payload/project.evaraproj
```

One digest per fact, acyclic by construction. `manifest.json` carries the digest of
`checksums.json`, which carries the digest of the payload.

### What this proves, and what it does not

**These digests detect corruption, not authenticity.** They catch truncation, bit rot, a bad
copy, a partially written archive and a substituted payload. They establish nothing about who
produced the file: **anyone can rewrite the manifest and recompute every digest below it.**

There is no signature, and that is a decision rather than an omission — a signature needs key
distribution, and ADR 0001 forbids the service that would provide it.
[ADR 0059](adr/0059-archive-integrity-is-corruption-not-authenticity.md) records it,
`OPEN-DECISIONS.md` R11 tracks the residual risk, and the import screen and
`evara inspect` both say it in words at the point somebody is deciding whether to trust a file.

**A `.evara` carries exactly the authority of the person who sent it to you.**

### Verification order — each step gates the next

| # | Step | Refusal |
| --- | --- | --- |
| 1 | Open the ZIP; count entries; judge every name | `NotAnArchive`, `TooManyEntries`, `UnsafeEntryName`, `DuplicateEntryName` |
| 2 | Locate `manifest.json` by **exact name** | `ManifestMissing` |
| 3 | Read ≤ 64 KiB through a limited reader | `EntryTooLarge` — refused *before* being parsed |
| 4 | Parse; check `format` and `formatVersion` | `ManifestUnreadable`, `UnknownFormat`, `FormatTooNew` |
| 5 | Locate `checksums.json`; read ≤ 1 MiB; digest it; compare with `manifest.integrity.sha256` | `EntryMissing`, `EntryTooLarge`, `ChecksumMismatch` |
| 6 | Parse it; require `algorithm == "sha256"` exactly | `UnsupportedDigest` |
| 7 | Check `payload.schemaVersion` against this build | `SchemaTooNew` — **before a payload byte is written** |
| 8 | Check `capabilities` | `UnsupportedCapability` — also before any payload write |
| 9 | Stream the payload into staging through a reader limited to `payload.byteLength`, hashing as it goes | `EntryTooLarge`, `PayloadLengthMismatch`, `ChecksumMismatch` |
| 10 | Only now is the payload a file worth opening — §5 | — |

Steps 7 and 8 sit before step 9 deliberately: **an archive this build cannot use is refused
without writing anything at all.** That is the difference between an import that fails and an
import that fails having left a half-staged project behind.

### Size and shape limits

Every read is bounded by a constant **this build** chose, applied through a limited reader. A
declared size in the central directory is a claim by the sender, not a fact; and a digest is
known only after the bytes have been read, which is after the damage would have been done.

| Cap | Value | Why |
| --- | --- | --- |
| `MAX_MANIFEST_BYTES` | 64 KiB | a real manifest at ten schools is a few kilobytes |
| `MAX_CHECKSUMS_BYTES` | 1 MiB | one line per entry, and entries are capped at 64 |
| `MAX_PAYLOAD_BYTES` | 2 GiB | an order beyond the largest project anyone has described |
| `MAX_ENTRIES` | 64 | v1 writes three; the rest is forward tolerance |
| `MAX_ENTRY_NAME_BYTES` | 128 | the three names are 13, 14 and 25 bytes |
| `MAX_COMPRESSION_RATIO` | 2 000 | applied to **declared** sizes, before anything is decompressed |
| `COPY_BUFFER_BYTES` | 1 MiB | the one buffer a stream uses, so memory is O(1) in project size |

An archive over a cap is refused and named. It is never read "as far as the cap and then treated
as truncated", because that would report corruption for what is actually a resource refusal — two
different things to tell a user, with different remedies.

Only `Stored` and `Deflate` are accepted. An encrypted entry, an entry that is not a file, and a
duplicate entry name are each refused. **A Deflate entry cannot express a bomb**: the format's own
maximum expansion is 1032:1, so what actually bounds the payload is `MAX_PAYLOAD_BYTES` and the
declared `byteLength`, applied through the limited reader.

### Nothing is ever extracted to a path from the archive

Three entries, opened by **exact name**. The directory is never iterated to decide what to read,
no entry's stored attributes are honoured, and the payload is streamed to a path **Evara chose**.
`zip::ZipArchive::extract` — where all three of the advisories the dependency upgrade closed live
— is never called.

So path traversal is **unexpressible rather than defended against**: there is no code path in
which an archive's bytes contribute to a filesystem path.
[ADR 0055](adr/0055-an-archive-is-read-by-name-and-never-extracted.md).

---

## 4. Export

`evara_project::seal` takes an owned snapshot and writes an archive.

1. **Refuse the destination early** if it is not a `.evara`, or if its path is one Evara will not
   write to (a `file:` URI, a `..` component, a symlink at the leaf).
2. **Claim the destination** with `create_new(true)` — §6.
3. **Snapshot the project** under the workspace lock, and release the lock before a byte is
   compressed.
4. **Hash the snapshot** before the archive is opened, so a snapshot that cannot be read produces
   no partial file at all.
5. Build `checksums.json`, then `manifest.json` — every fact in it read from the **snapshot**,
   not from the live project, so the manifest describes the bytes being shipped.
6. Write the three entries to `<destination>.part`.
7. **Rename `.part` onto the destination.** An interrupted export therefore never leaves a
   plausible-looking corrupt `.evara`.

**The source project is never modified.** It is opened read-only — `open_for_inspection`, which
runs no migration, recreates no trigger and stamps no metadata — and the export leaves it
byte-for-byte unchanged with no `-wal` or `-shm` beside it.

### The snapshot

A project runs in WAL, so its committed state is split between the main file and its `-wal`
sidecar. Copying the main file would yield a database that opens cleanly and silently lacks the
most recent work, so the snapshot is always a **fully checkpointed single file**, taken with
SQLite's online backup API inside one pinned read transaction.

The lock is held for the snapshot alone, measured at ~46 ms against a ≤ 500 ms budget (~50 ms
under a concurrent writer). The backup API reproduces the source page for page, including free
pages, so a snapshot is about 6.9% larger than a vacuumed copy would be — of which the archive's
compression absorbs 88.7%. No compaction pass is added; the measurement is in the proposal's
§C.10. [ADR 0053](adr/0053-the-payload-is-a-verified-sqlite-snapshot.md) §3.

---

## 5. Import

`evara_project::import` stages, verifies, migrates and renames.

```
P0  validate the destination path and the archive's header   nothing exists that did not before
P1  decide: claim an absent destination, or require Replace  at most a zero-length claim Evara made
P2  stage: journal, stream, verify, migrate, verify          the destination is untouched except that claim
P3  Create finalise: one rename                              either the placeholder or the new project is at the name
P4  Replace finalise: four steps, each after its journal commit   one of {old intact} or {new in place, old kept beside it}
```

### The payload is verified before it is trusted — V0, V1, V2

First failure stops everything.
[ADR 0056](adr/0056-a-payload-is-audited-against-a-generated-reference.md).

- **V0 — audit, before anything writes.** The staged payload is opened file-level read-only with
  SQLite's own capabilities turned down: `trusted_schema`, `writable_schema`, views, triggers and
  both `ATTACH` capabilities off, and `SQLITE_LIMIT_ATTACHED` at zero. Then
  `PRAGMA integrity_check` and `PRAGMA foreign_key_check`, then a comparison of its schema against
  a reference **this binary generates from its own migrations**. Only then is `app_meta` read.
  Reading the identity first would mean trusting a row out of the file in order to decide whether
  to trust the file.
- **V1 — migrate.** A writable connection on the staged copy, the ordinary migration chain in its
  grouped transaction, then the change-log triggers reinstalled. `trusted_schema` stays **off**,
  which is why `evara_site_id()` is marked `SQLITE_INNOCUOUS` — the change-log triggers fire
  during migration and would otherwise be refused.
- **V2 — verify what migration produced.** `foreign_key_check` empty again, conformance at the
  *current* version, and a table census the caller can compare a round trip against.

**It is not a sandbox.** No arrangement of pragmas makes handing an untrusted file to a C parser
invulnerable. What the controls remove is the capabilities an attacker would otherwise simply ask
for. Corruption and hostility are detected; invulnerability is not claimed.

### An inspection is not a verification

`evara_project::inspect` — and `evara inspect`, and the import screen's first step — reads the
manifest and the digest record and **does not read the payload**. Streaming and hashing the
campus inside is step 9 of §3's order and belongs to the import.

So a readable summary means *this build understands the container*, never *the campus inside is
intact*. A damaged payload is caught during the import, before anything is written, and the
interface says which of the two it has established.

### What an import does not do

- **It rebuilds nothing.** Re-scoring a transferred timetable needs a compiled `Problem`, which
  can legitimately fail for a school mid-setup — an ordinary state that must still transfer — and
  it would write a `REVALIDATION` row where the archive carries an immutable `GENERATION` one.
  Scoring is an action the interface offers afterwards.
- **It takes no safety backup of the staged file.** The archive is an untouched, checksummed copy
  of the source, which is a better backup than a `VACUUM` of a throwaway.
  [ADR 0057](adr/0057-an-import-is-staged-verified-and-renamed.md).
- **It never invents a record, and never merges.** The whole project arrives as it was; there is
  no reconciliation and no identity matching — that is the *spreadsheet* import's problem
  (ADR 0030, ADR 0031), and this is a different act.
- **It does not open what it imported.** The new project is remembered in the recent list and the
  interface offers "Open now" as the user's own decision. Auto-opening would close whatever was
  open and discard an edit session as a side effect of an import.
- **It does not touch the open project.** A destination that *is* the open project is refused by
  name, because replacing a file SQLite still holds open is how a project ends up paired with a
  live `-wal` belonging to a database that no longer exists.

---

## 6. Destinations, replacement and recovery

### A destination is claimed, not checked

Both halves of transfer finish by **renaming** onto the name the user chose, and `rename` replaces
whatever is there. An `exists()` test leaves the whole snapshot-and-compress, or the whole
stage-and-verify, between the answer and the action — and a file created inside that window would
be destroyed without anybody being asked.

So the name is **created**, with `create_new(true)`: the question and the answer are one
filesystem operation, two callers racing for one name have exactly one winner, and the
zero-length placeholder is what the final rename replaces. It is removed on every path that is
not a successful finish, so a refusal, a failure or a cancellation leaves the folder as it was
found.

One implementation, three callers — `evara_project::claim::DestinationClaim`, used by the import,
by `evara export` and by the desktop export.

**An existing zero-length file is occupied, not creatable.** `create_new` fails if the path exists
at all, which is deliberately unlike `Database::create`: creating a project where the user just
said to is one act, and an import adopting an unknown empty file is writing over something nobody
has looked at.

### Replacement is journalled and non-destructive

An occupied destination needs an explicit Replace. The protocol journals its phase in the
**settings** database (not the project), and each step is committed before it is taken:

| Phase | State on disk |
| --- | --- |
| `STAGED` | staging exists and has been written to; nothing outside it touched |
| `ASIDE` | the old file has been moved into `staging/aside/` |
| `PLACED` | the new file has been renamed to the destination |

**The replaced project is kept as `<stem>.replaced-<stamp>.evaraproj`, and no code path in Evara
deletes it** — not on success, not on failure, not during recovery. So *"a failed import must not
destroy the previous project"* is a structural property rather than a promise about error
handling. The two things the protocol does remove are its own staging directory and its own
`.part` file, neither of which is a user document.
[ADR 0058](adr/0058-replacement-is-journalled-and-non-destructive.md).

Staging is always a **sibling of the destination**, so the finishing rename never crosses a
volume — a cross-volume rename fails at the worst possible moment, after everything has been
verified.

Each side is reduced to **one file before it moves**. There is no atomic multi-file move on either
platform, and a crash between moving `X.evaraproj` and moving `X.evaraproj-wal` would leave the
new main file beside a stale log — a database that opens cleanly and is wrong. So sidecars are
released first, and the destination is asserted clean immediately before the rename that puts the
new file there.

### Recovery

An interrupted replacement is recovered **at startup, before any project is open** — because the
project that needs recovering may be the one that will not open. Recovery reconciles the journal's
*intent* against **observed** facts on disk and acts on what it finds, never on the recorded phase
alone.

Its answer is one of: roll back and change nothing; put the previous project back; finish the move
and name the kept copy; or **report and touch nothing**. An ambiguous state — a project file, a
half-finished import and a previous copy all present at once — is reported and acted on by
nothing, because guessing is how the protocol would lose somebody's project. The interface shows
resolved outcomes as information and leaves unresolved ones on screen.

**Deciding what is at a path must not change it.** Whether a file is a project is answered from its
sixteen-byte header before anything is opened: `PRAGMA journal_mode = WAL` *creates a database* in
a zero-length file, and opening a path beside a stray `-wal` makes SQLite recover the log and then
remove it — discarding committed transactions the main file does not hold. Nothing in the transfer
path ever deletes a sidecar to make a destination look clean.

### Cancellation

Cancellation is a flag the engine reads between steps, and it is honoured throughout the snapshot,
the hash, the compression, the staging and the verification.

| Cancelled | Left behind |
| --- | --- |
| an export | **no archive and no `.part`** — the destination placeholder is removed too |
| an import | **no project at the destination**, and no staging beside it |

A cancellation is not a failure: it is the command not doing what was asked because it was asked
not to. The CLI exits 2 and says so; the interface says *"nothing was written"* rather than
reporting the user's own Stop as a fault.

---

## 7. Compatibility — two axes, one refusal each

| Axis | Means | Too new | Older |
| --- | --- | --- | --- |
| `formatVersion` | the **container**: entry layout, manifest schema, payload kind | refused, naming the Evara version needed | the container migration chain (empty at v1) |
| `payload.schemaVersion` | the **data**: which migration the payload was written at | `SchemaTooNew`, refused | the staged copy is migrated forward |

Keeping them independent is ADR 0004's hedge, made precise by
[ADR 0054](adr/0054-two-version-axes-one-refusal-each.md): a reader needs to know *which* number
refuses their import, and the second axis is a mechanism Evara already ships rather than a new
one.

The payload's own `refinery_schema_history` is the authority on the data axis; the manifest's copy
exists to refuse early and cheaply, before a payload byte is written. **A disagreement between the
two is itself a refusal** — it means the archive was not assembled by Evara.

| Situation | Behaviour |
| --- | --- |
| `formatVersion` < current | migrate the container; note it in the summary |
| `formatVersion` = current | read directly |
| `formatVersion` > current | **refuse**, naming the version needed |
| `payload.schemaVersion` < current | migrate the staged copy; the report says from and to |
| `payload.schemaVersion` > current | **refuse** |
| manifest and payload schema versions disagree | **refuse** |
| unknown `capabilities` entry | **refuse**, naming the capability |
| unknown manifest field | ignore |
| unknown but safe extra entry | ignore, and report the name |
| any digest mismatch | **refuse** |

A released migration is immutable and there are no down migrations, so a project can always move
forward and never back. An archive written by a newer build is refused rather than partially read.

---

## 8. Capabilities

`capabilities` lists what a reader must understand to render the project **honestly**. It is
**derived from the data at export time**, never hand-maintained: a one-row probe per feature, run
against the snapshot, so an archive declares exactly what it contains.

Three namespaced families, all read from the data:

| Form | From | Example |
| --- | --- | --- |
| `constraint:<type_key>` | the distinct `type_key`s `constraint_instance` holds | `constraint:no-clash-teacher` |
| `cycle-kind:<KIND>` | the distinct `cycle.kind` values | `cycle-kind:WEEKLY` |
| a feature marker | a one-row probe each: `lineage:repair`, `lock:placement`, `placement:manual`, `logo:embedded` | `lock:placement` |

An entry this build does not recognise is a **refusal naming it**, before any payload byte is
written. The alternative — importing anyway and dropping what it cannot evaluate — would produce a
project that looks complete and is not.

---

## 9. Identity

**`project_id` is never regenerated.** Not on export, not on import, not on replacement. ADR 0004
fixed that and it is what keeps round-trip comparison and any future synchronisation possible.
`created_by_site_id` is likewise preserved: an imported project keeps the provenance of the
installation that created it, and the exporting installation never becomes its creator.

So two project files on one machine can carry **one identity** — which is exactly what copying a
file or importing one twice produces, and is legitimate. Evara cannot merge two copies, so the
remedy is **disclosure, not prevention**:

- the import screen warns **before anything is staged**, naming the file already on this machine
  and saying that changes made in one will not appear in the other;
- a Replace onto that same path says so in sharper words, and names the copy being kept;
- the recent-project list marks every entry whose identity another entry shares.

Nothing regenerates a UUID, deletes a twin, hides one from the list or deduplicates the list.
There is no uniqueness constraint on a remembered project's identity: two legitimate copies must
both be listable.

---

## 10. What a user can do with it

| Command | Does |
| --- | --- |
| `evara export --project <p.evaraproj> --to <a.evara>` | write an archive. Read-only on the source; refuses an existing archive rather than overwriting it |
| `evara inspect <a.evara>` | what the archive says about itself and whether this build can read it. Writes nothing, imports nothing, does not read the payload |
| `evara import <a.evara> --to <p.evaraproj> [--replace]` | read an archive into a project file. Refuses an occupied destination without `--replace`, and never deletes the project it replaced |

Exit codes: `0` done, `1` refused for a reason the command correctly produced, `2` the command
failed. A refusal is not a failure — an archive from a newer Evara is a complete, correct answer
to the question asked.

In the desktop application: **Export campus file…** in the title bar of an open project, and
**Import campus…** on the start screen. Both show progress by step with a Stop button; the import
inspects first, shows what the archive claims, warns about a duplicate identity, requires an
explicit confirmation to replace, names the kept copy, and offers **Open now** afterwards rather
than opening anything itself.

---

## 11. Tests this format has

| Property | Where |
| --- | --- |
| Round trip: export → import → compare every table's census and the project identity | `evara-project` round-trip tests, over both synthetic fixture shapes |
| Timetable history, locks and repair lineage survive | `tests_round_trip`, `evara-cli` in-crate tests |
| Corruption: truncated archive, flipped byte, bad digest, missing manifest, not-a-ZIP | `tests_refusal` |
| Hostile shapes: traversal names, absolute names, encrypted entry, declared bomb ratio, oversize entries, duplicate names | `tests_refusal`, `evara-db`'s boundary tests |
| Replacement: the three phases, the kept copy, the sidecar refusal, every reachable recovery state | `tests_replacement`, `evara-db`'s `tests_replacement` |
| Recovery reconciles the filesystem rather than the recorded phase | `recovery_acts_on_the_filesystem_and_not_on_the_recorded_phase` |
| The destination claim: occupied whatever is there, released vs dropped, a competing claim | `tests_replacement` |
| Cancellation leaves no archive, no project, no staging, no placeholder | `evara-project`, `evara-cli`, `src-tauri` command tests |
| **Historical projects: V1…V18 open and migrate in this build** | `tests_historical::every_version_opens_and_migrates_in_this_build` |
| Archive round trip for every *describable* schema version | `tests_historical::every_describable_schema_version_exports_and_imports_into_this_build` — see [ROADMAP.md](ROADMAP.md) Phase 9 for why that set is `[18]` |
| Measurements: change-feed share, lock hold time, snapshot inflation | `tests_measurements` |

All fixtures are **synthetic and generated**, never checked in: no binary `.evaraproj` or `.evara`
enters git, and no real school, teacher, parent or student data appears anywhere — including in
bug-report attachments.
