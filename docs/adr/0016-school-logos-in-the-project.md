# 0016 — School logos live in the project, validated from their bytes

**Status:** Accepted
**Date:** 2026-10-11
**Extends:** [0003](0003-sqlite-document-model.md), [0010](0010-structure-deletion-semantics.md),
[0012](0012-records-are-the-ipc-contract.md)

## Context

Evara needed an application logo in its branding areas, and schools needed to be able to
upload their own for their screens and — later — their timetables and reports. A school
without one falls back to Evara's.

That is a small feature with four decisions worth recording, because each had a plausible
alternative that would have been worse in a way only visible later.

## Decision

### 1. The existing icon artwork is the logo; nothing new was drawn

`src-tauri/icons/` already held a complete, deliberate icon set — a 3×3 grid mark in the
application's own indigo, committed with the desktop foundation and generated from
`icons/source.png`. It is not a placeholder and not the Tauri default.

`src/assets/evara-logo.png` is a byte copy of `icons/128x128.png`. The copy exists because
the frontend bundle cannot reach into the Tauri crate's layout without coupling the web
build to it; one 3.7 KB PNG is cheaper than that coupling. `brand.test.ts` asserts the two
files are byte-identical, so a redraw that updates only one is a failing test rather than
an application whose window and taskbar disagree.

**No new brand artwork was created, and none should be without a decision to finalise the
logo.** See "Missing assets" below.

### 2. The image is a BLOB in the project file

Not a file beside the project, and not an absolute path to one.

The `.evaraproj` file *is* the unit a user copies, backs up and sends to a colleague. A
logo stored next to it is lost by every one of those actions, and a stored path breaks the
moment the project moves, is opened on another machine, or is restored from a backup into a
different folder. Putting the bytes in the file makes the logo as portable as everything
else in it, and a future `.evara` export reads the table like any other — nothing extra is
needed to carry the image.

It is its own table rather than a column on `school`, for one concrete reason: `school` is
read by almost every screen, and a 2 MiB BLOB in that row would cross the IPC boundary
every time anything asked for the school's name. `UNIQUE (school_id)` makes it strictly one
per school, so absence *is* the fallback condition — there is no `is_active` flag and no
way to end up with two and have to choose.

It is a first-class entity in the [ADR 0013](0013-buildings-shared-across-campuses.md)
sense — own UUIDv7, own audit columns, own change-log triggers, own `TRACKED_TABLES` entry
— because `repo::update`/`repo::delete` address rows by id and the triggers read `NEW.id`
and `NEW.rev`. Replacing a school's logo is a real edit and belongs in the change feed.

### 3. Format is decided from the content, never from a filename

`evara_core::branding` reads the file's own header: PNG by signature, JPEG by its Start of
Image marker, WebP by its RIFF container. A `.png` holding a JPEG is stored as a JPEG; a
`.png` holding an executable is refused. **No filename is taken by any layer** — the
command signature has no parameter for one, so there is nothing to disbelieve.

Dimensions are read from the same headers, which is why there is no image dependency.
Everything the module reads is a header field at a known offset; it never decodes pixel
data, so it cannot be made to allocate a frame buffer or spend time proportional to the
image's size. A header claiming a 60 000 × 60 000 canvas is refused by a 33-byte file in
constant time. A decoder would do neither of those things on untrusted input, to answer a
question we are not asking.

The limits — 2 MiB, 16–2048 px — are restated as `CHECK` constraints so they hold against a
write that bypasses the repository. The module is the authority for the *message*; the
schema is the backstop.

### 4. A file input, not a native dialogue and a filesystem read

The obvious Tauri answer is `plugin-dialog` plus a path read. A native picker hands back a
**path**, which means granting the frontend the capability to have an arbitrary named path
read — and then being careful about what will be read.

`<input type="file">` hands back the **bytes** of a file the user chose in an OS-level
dialogue the page cannot script. There is no path to traverse, no filesystem capability to
grant, and nothing for a bug to turn into an arbitrary-file read. It also works unchanged
under Vitest, where no Tauri runtime exists, which is why the panel is testable at all.

The bytes cross as base64. This is a deliberate, documented exception to ADR 0012: JSON has
no byte-array type, and serde would write a 2 MiB image as roughly 12 MB of decimal
numbers. `SchoolLogoView` carries a ready-to-use `data:` URL — `SchoolLogo` itself is
absent from the generated TypeScript, because its `Vec<u8>` has no business crossing as
JSON. A future Rust report uses the record directly and never sees the view.

### 5. Fallback is decided in exactly one place

`app/branding.tsx` holds `resolveLogo`, a pure function over the value the backend returns.
Every view showing a logo goes through it, including views that do not exist yet. It treats
an empty or missing `dataUrl` as absent rather than rendering a broken image, which is the
second of two guards — the backend already declines to render a media type it cannot name.

Evara's own branding — desktop icon, installer, start screen — is never affected by what a
school uploads.

## Consequences

**Good.**

- A project file is self-contained: copy it, and the logo comes with it. No path to break.
- No image dependency, and no decoder running on untrusted input.
- No filesystem capability was added to reach a user's files.
- The migration needs no data step. A project upgraded from V7 simply has no row, which is
  already "this school uses the Evara logo" — nothing is written and no revision consumed.

**Bad, or at least accepted.**

- A 2 MiB logo makes the project file 2 MiB bigger, and SQLite stores it inline. Acceptable
  for one small image; it would not be for a general attachment feature.
- Base64 means the largest logo is about 2.7 MB of JSON on the wire. Tauri v2 can carry raw
  bytes through `tauri::ipc::Response`, which would avoid it; that is a worthwhile
  optimisation only if logos ever get bigger or more numerous.
- Header parsing is hand-written for three formats. It is bounds-checked, tested against
  every prefix of every sample, and refuses rather than guesses — but it is still three
  parsers this project now owns.
- The mark exists twice on disk, guarded only by a test.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| A file beside the project, or an absolute path | Lost by copying, backing up or moving the project — the operations users actually perform |
| A column on `school` | Ships the image on every read of the school's name |
| An `image` crate for validation | A decoder on untrusted input to read four header fields |
| Trusting the filename or the browser's declared type | Exactly what "validate contents" forbids; a rename defeats it |
| `plugin-dialog` plus a filesystem read | Needs an arbitrary-path read capability the application otherwise does not want |
| Sending `Vec<u8>` through serde | ~12 MB of JSON for a 2 MiB image |
| Re-encoding uploads to a canonical size | Needs a decoder, loses transparency or sharpness, and the file out of an export stops matching the file in |

## Missing assets

The icon set is real artwork and is reused, but **it has not been confirmed as the final
Evara logo.** If it is a placeholder, what is needed is one master image — square,
transparent background, at least 1024 × 1024 — to replace `icons/source.png`, after which
`npx tauri icon` regenerates the set and `src/assets/evara-logo.png` is re-copied from
`128x128.png`. No other code changes. Until then the application ships the mark it has
always shipped, which is the status quo rather than a new invention.
