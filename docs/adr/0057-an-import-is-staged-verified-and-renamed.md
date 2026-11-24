# ADR 0057 — An import is staged, verified and renamed; the archive is its own safety copy

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal D7, D8, D9
- Amends: [ADR 0008](0008-installation-identity-and-project-lifecycle.md) in one respect — the
  import path does not take `backup::before_schema_rebuild`'s safety copy. Everything else 0008
  decided about identity and the project lifecycle is untouched
- Builds on: [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) (the payload is a
  checksummed copy of the source),
  [ADR 0056](0056-a-payload-is-audited-against-a-generated-reference.md) (V0/V1/V2),
  [ADR 0030](0030-an-import-is-a-reviewed-plan.md) (the *spreadsheet* import's contract, which
  this is deliberately not)
- Binds: `evara_project::import`, `evara_db::transfer::stage`, `evara_project::staging`

## Context

A project transfer import has to end with a usable project at a path the user chose. Two
obvious shapes were available.

The **insert** shape is what ADR 0004 specified and what ADR 0030 does for spreadsheets: create
a fresh database, then insert every row in dependency order inside one transaction. The
**rename** shape stages a complete file elsewhere, verifies it, and moves it into place.

Once the payload is a database (ADR 0053), the insert shape stops making sense: it would mean
reading fifty tables out of a verified database and writing them into another one, which is a
slower, lossier way of producing the file already in hand — and every row read and rewritten is
a row whose `rev`, `created_at` and provenance can be got wrong.

Two further questions had to be settled, and both have a tempting wrong answer: whether to take
a safety backup before migrating the staged copy, and whether to rebuild derived state after.

## Decision

**Stage, verify, migrate, verify, then rename. The destination is only ever written by a
rename.**

```
P0  validate the destination path and the archive's header
P1  claim an absent destination, or require an explicit Replace
P2  stage: journal, stream, V0, V1, V2
P3  Create finalise: one rename
P4  Replace finalise: the journalled protocol — ADR 0058
```

### 1. Staging is a sibling of the destination

Not the system temporary directory. A cross-volume rename fails, and it fails at the worst
possible moment: after the payload has been streamed, verified and migrated. Staging beside the
destination means the finishing rename is a same-volume operation, and it also means the copy is
written where the user has already decided there is room.

### 2. No safety backup of the staged file

`backup::before_schema_rebuild` fires on any open below V12 and would litter staging with a
`.pre-vN.evaraproj`. On this path it is **redundant**: the archive is an untouched, checksummed
copy of the source, which is a better backup than a `VACUUM` of a throwaway file that is about
to be renamed or deleted.

This is the one respect in which V1 is not `Database::open_existing`, and it is recorded here
because it reads like an omission and is not.

The safety copy that *does* matter on this path is the one
[ADR 0058](0058-replacement-is-journalled-and-non-destructive.md) keeps: the project being
replaced.

### 3. An import rebuilds nothing

Re-scoring a transferred timetable would need a compiled `Problem`, and compiling one can
legitimately fail — a school mid-setup with no bell schedule yet is an ordinary state that must
still transfer. An import that rebuilt scores would therefore fail for projects that are
perfectly valid.

It would also write the **wrong kind of row**. A transferred evaluation is an immutable
`GENERATION` record (ADR 0042); re-scoring produces a `REVALIDATION`. Rebuilding would either
misrepresent how the timetable was generated or require a special case that exists for nothing
else.

So scoring is an action the interface offers **afterwards**, and a failure to score is a finding
about the project rather than a failed import. ADR 0004's step 6 — *"rebuild derived state
(timeslot integrity, score, violations)"* — is withdrawn; stored findings travel in the payload
instead, which is the only way they *can* travel given that a `GENERATION` scoring is immutable.

### 4. The import does not open what it imported

The new project is remembered in the recent list, and the interface offers "Open now" as the
user's own click. Auto-opening would close whatever was open and discard an edit session, a
generation and a repair as a side effect of an import.

The interface places the two actions accordingly: export acts on the open project and lives in
its title bar, import produces a new one and lives on the start screen.

### 5. A destination that is the open project is refused by name

Replacing a file SQLite still holds open is how a project ends up paired with a live `-wal`
belonging to a database that no longer exists. The refusal names the file and the remedy —
close it — and it is the command layer's, because only that layer knows what is open.

### 6. The failure of a transaction is not what makes this safe

The insert shape's safety property was *"one transaction, so a referential problem aborts the
whole import"*. The rename shape's is stronger and differently shaped: **until the final rename,
nothing exists at the destination except possibly a zero-length placeholder Evara made.** A
failure at any point before it leaves the folder as it was found, and a failure after it is
[ADR 0058](0058-replacement-is-journalled-and-non-destructive.md)'s problem.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| ADR 0004's insert-into-a-fresh-database | Slower, and every rewritten row is a chance to lose a `rev`, a timestamp or a provenance field. The verified file is already in hand |
| Stage in the system temporary directory | A cross-volume rename fails after all the work is done. Staging beside the destination makes the finish a same-volume move |
| Take `before_schema_rebuild`'s backup in staging | Redundant against a checksummed archive, and it litters staging with files that are about to be thrown away |
| Rebuild scores and violations after import | Fails for projects mid-setup, and writes a `REVALIDATION` where the archive carries an immutable `GENERATION` |
| Open the imported project automatically | Discards the user's open edit session as a side effect of an import. "Open now" costs one click and is theirs to make |
| Allow replacing the open project, closing it first | A close is a user decision with unsaved-state consequences; doing it implicitly to serve an import inverts who is in charge |

## Consequences

- **The destination is only ever written by a rename.** Every other write goes to staging.
- **An import never touches the source archive**, and never deletes it.
- **A transferred timetable arrives with its findings and its generation record intact**, and
  with no revalidation attached — so the stored tier renders exactly what the exporting
  installation saw.
- **`ImportReport` states what happened rather than implying it**: the schema version found and
  the one it now stands at, whether a migration ran, the census, any count disagreement, the
  kept copy's name, and whether the recent-list entry was written. A failure to write that entry
  is reported and does not fail the import.
- **Staging is cleaned up on every path**, and the two things the protocol removes are its own
  staging directory and its own `.part` file — neither of which is a user document.
