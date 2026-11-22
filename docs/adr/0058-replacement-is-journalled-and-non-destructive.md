# ADR 0058 — Replacement is journalled and non-destructive; the replaced project is kept, never deleted

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal D11 **as amended by §0.2's three safeguards**, and approved decisions
  A3 and A4
- Builds on: [ADR 0057](0057-an-import-is-staged-verified-and-renamed.md) (the destination is
  only ever written by a rename),
  [ADR 0008](0008-installation-identity-and-project-lifecycle.md) (the settings database, which
  owns the journal)
- Binds: `evara_db::transfer::{replacement, sidecars, paths}`, `evara_project::{staging,
  recovery}`, `evara_project::claim::DestinationClaim`, settings migration
  `V2__pending_replacement.sql`

## Context

An import into an occupied path is the one operation in Evara that can destroy a user's work
irrecoverably. `fs::rename` replaces whatever is at the destination on both platforms, so the
naive implementation — rename the staged file over the old one — is one system call away from
losing a school's entire timetable.

Revision 2 of the design said "journalled and recoverable" and got three things wrong, each
found at the second architecture review and each made a condition of approval. Two of them would
have lost a user's project:

- It claimed a destination that was *"absent, or present and zero length"* with
  `create_new(true)` — and `create_new` fails if the path exists **at all**. The two halves of
  that sentence cannot both hold.
- It had recovery act on the journal's recorded **phase**. A journal records *intent*; the
  filesystem records what happened. A crash between committing a phase and performing it means
  the two disagree, and acting on intent would move a file that was never moved.
- It moved a project's files one at a time. A project is up to four files and **there is no
  atomic multi-file move on either platform**.

## Decision

**A destination is claimed, not checked. A replacement is journalled phase by phase, acts on
observed facts when it recovers, and the project it replaces is kept — permanently.**

### 1. The claim is `create_new(true)`, and a zero-length file is occupied

The question *"may I write here?"* and the answer are **one filesystem operation**. Two callers
racing for one name have exactly one winner; the loser is told the name is occupied, which is
true. The zero-length placeholder the winner holds is what the final rename replaces, so the
rename has nothing of anybody else's to destroy.

Between an `exists()` test and the rename sit a payload stream, V0, V1 and V2 — seconds to
minutes — and a file created inside that window would be destroyed without anybody being asked.

**An existing zero-length file is therefore occupied, not creatable.** This diverges from
`Database::create`, which treats one as creatable, and the divergence is deliberate:
`Database::create` is making a project where the user just said to, and an import adopting an
unknown empty file is writing over something nobody has looked at. It needs an explicit
Replace, and the Replace path keeps it like any other file.

One implementation, three callers — the import, `evara export` and the desktop export all take
`DestinationClaim`.

### 2. Three phases, each committed before it is taken

| Phase | What is true on disk |
| --- | --- |
| `STAGED` | staging exists and has been written to; nothing outside it touched, except possibly our own claim |
| `ASIDE` | the destination's old file has been moved into `staging/aside/` |
| `PLACED` | the new file has been renamed to the destination |

The journal is a row in the **settings** database — the installation's, not the project's,
because the project in question may be the one that will not open. It costs one settings
migration (`V2__pending_replacement.sql`) and **no project migration**: V18 is untouched by
M10.

The journal row is cleared **last**. Until it is, an interruption is recoverable; after it, the
import is finished and there is nothing to recover.

### 3. The replaced project is kept as `<stem>.replaced-<stamp>.evaraproj`, and nothing deletes it

Not on success. Not on failure. Not during recovery. Not later, not by a cleanup pass, not by
a retention policy.

So *"a failed import must not destroy the previous project"* is a **structural property** rather
than a promise about error handling: there is no delete to get wrong. The two things the
protocol does remove are its own staging directory and its own `.part` file.

An unknown file at the destination — something Evara did not write — is kept under the same
rule. Evara does not delete a file it did not make, however little sense it makes at that path.

### 4. Recovery reconciles the journal against **observed** facts

Recovery runs once, at startup, before any project is open. It reads the filesystem — what is at
the destination, what is in `aside/`, what is in staging, what sidecars exist — and decides from
that, using the recorded phase only as a hint about what was being attempted.

Its answers are: roll back and change nothing; put the previous project back; finish the move
and name the kept copy; or **report and touch nothing**. The last is not a failure of the design
— an ambiguous state, such as a project file, a half-finished import and a previous copy all
present at once, is reported and acted on by nothing, because guessing is how the protocol would
lose somebody's project. The record is left on the journal so a person or a newer build can look
at it.

**Observing must not mutate.** Whether a file is a project is answered from its sixteen-byte
header before anything is opened: `PRAGMA journal_mode = WAL` *creates a database* in a
zero-length file, and opening a path beside a stray `-wal` makes SQLite recover the log and then
**remove it**, discarding committed transactions the main file does not hold. Recovery
reconciles observed facts against a journal's intent, so an observation that mutated would have
it act on a state it created itself.

### 5. Each side is reduced to one file before it moves

`checkpoint_to_single_file` releases sidecars first and **refuses before anything has moved** if
one cannot be released; `refuse_if_sidecars_present` asserts the destination is clean
immediately before the rename. The second is the safeguard proper: it makes the bad pairing —
a new main file beside a stale `-wal` — unrepresentable rather than merely unlikely, and it
holds even if a future change reintroduces a multi-file move.

A database that opens cleanly and is wrong is the worst failure available here, because nothing
reports it.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Rename the staged file over the destination | One system call from losing a school's work, with no record that anything was there |
| `exists()` then rename | The window is the whole stage-and-verify. A file created inside it is destroyed silently |
| Adopt an existing zero-length destination, as the design said | `create_new` cannot express it, and adopting an unknown empty file means writing over something nobody has looked at |
| Delete the replaced project on success | The one irreversible act in the feature, performed at the moment the user is least able to check. Keeping it costs disk and nothing else — A3 made this a condition of approval |
| Delete it after N days, or on next launch | Same decision with a delay and a worse failure mode: the user has by then stopped thinking about the import |
| Recover from the recorded phase | A journal records intent; a crash between commit and action makes the two disagree. Acting on intent moves a file that was never moved |
| Journal in the project being replaced | That project may be the one that will not open, and the journal has to be readable before any project is |
| Move the project's files one at a time, carefully | There is no atomic multi-file move. A crash mid-sequence leaves a new main file beside a stale log |
| A lock file to detect another installation holding the project open | macOS allows a rename over a file another process holds open, so it cannot be detected reliably — OPEN-DECISIONS R12. The kept copy is the mitigation |

## Consequences

- **Disk use grows by one project per replacement**, and the interface says where the copy is
  and that Evara will not remove it. Deleting it is the user's decision, in their file manager.
- **Durability is crash-*recoverable*, not crash-proof.** The guarantee relied on is that
  `rename` is atomic with respect to observers on both platforms; the guarantee **not** claimed
  is that a commit survives power loss without an fsync of the directory entry.
- **An interrupted replacement can leave a staging directory with no journal row** — power loss
  before the first commit. Recovery reports it and deletes nothing: it may be another
  installation's live staging.
- **An unresolved record stays visible.** The interface shows resolved outcomes as dismissible
  information and leaves unresolved ones on screen, because a warning the application quietly
  forgets is a warning nobody acts on.
- **The settings chain advanced to V2**, which is a one-way door for the installation's settings
  database and is recorded in the migration file itself.
