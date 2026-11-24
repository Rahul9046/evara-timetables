# ADR 0055 — An archive is read by exact name and never extracted to a path

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal D6
- Builds on: [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) (three entries, one
  of which is the payload)
- Binds: `evara_project::{entry, reader}`, `evara_db::transfer::paths`
- Relates to: the `zip` dependency upgrade that removed three advisories from the tree

## Context

Every path-traversal vulnerability in every archive-handling library in every language has the
same shape: the archive declares a name, the extractor joins it to a destination directory, and
the name turns out to contain `..`, an absolute prefix, a drive letter, a symlink, a UNC path,
a NUL, or an encoding that normalises into one of those after validation.

The defence everybody reaches for is validation — iterate the entries, sanitise each name,
reject the bad ones. The industry's experience is that this defence fails repeatedly, not
because the idea is wrong but because the set of "bad names" is platform-specific, encoding-
specific and open-ended. The three advisories the dependency upgrade removed were all in
`zip`'s `extract`, which is precisely that code.

M10 only ever needs **three** entries, and it knows all three names in advance.

## Decision

**Nothing in the transfer path ever derives a filesystem path from the archive, and
`ZipArchive::extract` is never called.**

### 1. Three entries, opened by exact name

```rust
pub const MANIFEST_ENTRY:  &str = "manifest.json";
pub const CHECKSUMS_ENTRY: &str = "checksums.json";
pub const PAYLOAD_ENTRY:   &str = "payload/project.evaraproj";
```

The reader asks the archive for these three names. It does not iterate the directory to decide
what to read, and it does not honour any stored attribute — no mode bits, no symlink flag, no
timestamps.

### 2. The payload is streamed to a path **Evara** minted

The destination of the payload stream is a file inside a staging directory Evara created beside
the user's chosen destination. No byte of archive content contributes to it.

So traversal is **unexpressible rather than defended against**. There is no code path in which
a hostile name could be applied to the filesystem, which means there is no validation to get
wrong, no platform difference to miss and no normalisation step to be bypassed.

### 3. Names are still judged — for the archive's *shape*, not for safety

The reader counts entries, bounds each name's length, refuses duplicates, and refuses an
obviously unsafe name. That is **not** the traversal defence; the defence is §2. It exists so
an archive that cannot be what it claims is refused early with a sentence a user can act on,
rather than read as far as a missing entry and reported as a missing entry.

### 4. A path Evara mints is still asserted

`evara_db::transfer::paths::require_plain_database_path` refuses a `file:` prefix, a `?`, a
`#`, a `%` escape, an interior NUL or a `..` component on the path a payload is **opened** by.
Those paths are Evara's own, so this cannot fire in normal operation — it is an assertion, kept
because the bundled engine is compiled with `-DSQLITE_USE_URI`, which makes URI interpretation
the global default. `SQLITE_OPEN_URI` is **not** a control in this build: omitting the flag
changes nothing, and the record previously claimed otherwise (finding F-2). What bounds URI
interpretation is that every such path is one Evara chose, plus this assertion.

### 5. Unknown entries are tolerated and reported, never read

An archive from a newer build may carry extra entries. They are counted against the entry cap,
their names are reported so a user knows something was ignored, and they are never opened.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| `ZipArchive::extract` to a temporary directory, then read the three files | This is the function the advisories were in. It honours names and attributes from the archive, which is the whole class of problem |
| Iterate entries and sanitise each name | The industry's standard defence and the one that keeps failing. Open-ended, platform-specific, and unnecessary when all three names are known in advance |
| Canonicalise the joined path and check it is still under the destination | Correct in principle and still a check against a path built from hostile input, with a TOCTOU window of its own. Not needing a path at all is strictly stronger |
| Keep `SQLITE_OPEN_URI` out of the open flags as the URI control | Factually ineffective in this build — `-DSQLITE_USE_URI` makes URI parsing the default, so the flag can only add the capability. Reintroducing it as a security claim would restate a disproven one |

## Consequences

- **The three entry names are part of `formatVersion 1`.** Renaming one is a breaking container
  change.
- **A payload cannot be read without staging it**, because reading it means streaming it to a
  path Evara chose. That is also what `inspect` relies on in order *not* to read it.
- **`zip`'s `extract` must stay unused.** A future contributor reaching for it would reintroduce
  the entire class. The crate's own module documentation says so where the entry names are
  defined.
- **Tests assert the refusals directly**: traversal names, absolute names, a drive-letter name,
  a duplicate name, an over-long name and an oversize entry each have a case, and they assert
  the *refusal*, not that the file was written somewhere harmless.
