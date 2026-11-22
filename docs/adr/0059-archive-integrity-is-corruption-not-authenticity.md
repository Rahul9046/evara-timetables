# ADR 0059 — An archive's integrity is a two-level tree, and it proves corruption rather than authenticity

- Status: **Accepted** (2026-11-22)
- Date: 2026-11-22
- Decides: M10 proposal §3.3, §6.2
- Builds on: [ADR 0001](0001-local-first-desktop.md) (local-first, no server, no accounts —
  which is what forbids the key distribution a signature would need),
  [ADR 0053](0053-the-payload-is-a-verified-sqlite-snapshot.md) (one payload entry to cover)
- Binds: `evara_project::{manifest::Integrity, checksums, reader}`,
  `INTEGRITY_NOTICE` in the desktop command layer, `evara inspect`'s output
- Relates to: OPEN-DECISIONS **R11**

## Context

An archive that describes itself has to be checked somehow, and the obvious instinct is to
cover everything with everything. Revision 1 of the design recorded the payload's digest in
**both** `manifest.json` and `checksums.json` and had the two verify each other.

That is a cycle. Neither file can be checked before the other, so a reader has to pick one to
trust first — at which point the second check adds nothing except the appearance of rigour.

The harder question is what any of it proves. A digest computed over a file, by a reader, using
an algorithm the file names, in a record the file carries, establishes that the bytes are
internally consistent. It establishes nothing whatsoever about **who produced them**, and the
distinction matters because a `.evara` is a file a user receives from somebody else — by email,
on a USB stick, from a shared drive.

## Decision

**A two-level digest tree with exactly one root, and the root is deliberately unverified.**

```
manifest.json          ← the deliberately unverified root
  └── integrity.sha256 → checksums.json
                           └── payload/project.evaraproj
```

One digest per fact, acyclic by construction. `manifest.json` carries the digest of
`checksums.json`; `checksums.json` carries the digest of the payload. `payload.sha256` does
**not** appear in the manifest.

`payload.byteLength` stays in the manifest, because the payload read has to be bounded before
any digest is known.

### 1. The order is fixed, and each step gates the next

The manifest is parsed first because `formatVersion` decides how everything else is read. Then
`checksums.json` is read, digested and compared against the manifest's claim. Then the payload
is streamed through a reader limited to its declared length, hashed as it goes, and compared.
The two cheapest refusals — schema too new, unsupported capability — sit **before** the payload
stream, so an archive this build cannot use is refused without writing anything.

### 2. `integrity.algorithm` admits exactly one value

`"sha256"`, and anything else is `UnsupportedDigest`. Not a negotiation: an algorithm field a
reader honours is an algorithm field an attacker chooses, and the historical lesson from
signature formats is that *"algorithm: none"* is a vulnerability waiting for a careless reader.
A future stronger digest arrives as a `formatVersion` bump, which is a decision made by Evara
rather than by the file.

### 3. Corruption is detected; authenticity is not — and the product says so

| Detected | Not detected |
| --- | --- |
| truncation, bit rot, a bad copy | who made the archive |
| a partially written archive | whether the manifest's description is honest about provenance |
| a substituted payload | whether the project inside was tampered with *before* it was exported |
| a payload longer or shorter than declared | |

**Anyone can rewrite the manifest and recompute every digest below it.** A `.evara` carries
exactly the authority of the person who sent it.

That sentence is not left for a reader of the code to infer. It is a constant in Rust —
`INTEGRITY_NOTICE` — so there is one source of the wording, and it is shown:

- by `evara inspect`, every time, before it says whether the build can read the file;
- by the desktop import dialogue, every time, at the point somebody is deciding whether to
  trust the file.

### 4. There is no signature, and the reason is ADR 0001

A signature needs key distribution: a keyserver, a trust root, a revocation story, or a
registration flow. ADR 0001 forbids the service that would provide any of them, and ADR 0002's
local-first premise means there is no authority to ask. Shipping a signature with a key
distributed out of band would be security theatre with a maintenance burden.

The honest position is the one taken: say what the digests prove, say what they do not, and
leave the trust decision with the person who received the file — where, in a local-first
product with no accounts, it necessarily sits anyway.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Record the payload digest in both files, cross-verifying | A cycle. Neither can be checked first, so in practice neither is |
| Cover the manifest with a digest inside itself | Impossible without a key or a convention that excludes the digest field — and the latter is a parsing subtlety that has broken real formats |
| A detached digest file beside the `.evara` | Travels separately, so it is lost exactly when it is needed, and it still proves nothing about authorship |
| Sign the manifest | Needs key distribution, which ADR 0001 forbids. Out-of-band keys would be theatre |
| Allow `integrity.algorithm` to vary | An algorithm the file chooses is an algorithm an attacker chooses. A bump is Evara's decision to make |
| Say nothing about authenticity and let the digests imply it | The failure this record exists to prevent: a green tick that a user reads as "this file is safe" |
| Treat a digest mismatch as a warning | A mismatch means the bytes are not what the archive describes. There is nothing useful to do with that except refuse |

## Consequences

- **A hostile archive is a supported threat model and a bounded one.** It can be refused for
  shape, size, version, capability and digest, and if it passes all of those it is still a
  *database from elsewhere* — which is
  [ADR 0056](0056-a-payload-is-audited-against-a-generated-reference.md)'s problem and the
  highest-ranked residual risk of the feature.
- **The manifest's advisory fields are advisory in the record as well as in the code.** A count
  disagreement is reported after a successful import, never used to refuse one: the payload is
  the authority.
- **One manifest field is a refusal on disagreement** — `payload.schemaVersion`, because the two
  numbers come from one snapshot in any archive Evara wrote, so a disagreement means the file
  was edited by hand ([ADR 0054](0054-two-version-axes-one-refusal-each.md) §3).
- **R11 stays open as a risk rather than being closed as solved.** It is a product limitation
  with a stated mitigation (say so, clearly, at the moment of decision), not a defect to fix.
- **Two exports of an unchanged project are not byte-identical.** `exportedAt` differs, and the
  snapshot copies live pages; round-trip equality is asserted as a census and an identity
  comparison instead.
