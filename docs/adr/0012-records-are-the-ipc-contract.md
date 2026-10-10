# ADR 0012 — Repository records are the IPC contract; no parallel DTO layer

- Status: **Accepted** (approved 2026-10-10)
- Date: 2026-10-10
- Relates to: [ADR 0001](0001-local-first-desktop.md),
  [ADR 0009](0009-typescript-type-generation.md)

## Context

[ADR 0009](0009-typescript-type-generation.md) chose `ts-rs` and said the TypeScript the
interface uses is generated "from the Rust DTOs". It did not say what a DTO *is* in this
codebase, because at the time there were no entities to argue about. Phase 1E needs an
answer: fourteen entity types, each with a record and an input, plus the materialisation
types, plus the grid projection — forty types in all have to reach the frontend.

Two existing facts pull in opposite directions.

The Tauri crate already has a hand-written view layer: `ProjectView` and
`RecentProjectView` wrap `ProjectSummary` and `RecentProject`. That is the obvious
precedent to follow.

But ADR 0001's whole reason for generating types is that the previous repository
hand-maintained a parallel model and it had **already drifted** — `packages/domain`
claimed `teacherIds: ID[]` where the Rust side had a single `teacherId`. The failure mode
is not specific to TypeScript. It is what happens whenever one definition is restated
somewhere a compiler cannot check.

## Decision

**The `evara-db` record and input types are the IPC contract.** They carry
`serde::Serialize`, `serde::Deserialize`, `#[serde(rename_all = "camelCase")]` and — behind
a default-off `ts` feature — `ts_rs::TS`. The generated TypeScript comes from them
directly. There is **no parallel DTO struct** in the Tauri crate for an entity.

Hand-written view types survive only where the shape genuinely differs from the stored
one, which is the case `ProjectView` was written for: it splits a `PathBuf` into a path and
a folder, renders `Uuid` as a string, and flattens `SyncedFolder` to a provider name.
Translation earns a type; transcription does not.

Three things decide it.

**The alternative is the drift ADR 0001 exists to prevent, merely relocated.** A DTO layer
over fourteen entities is twenty-eight more structs plus their `From` impls — roughly six
hundred lines whose only job is to restate field lists that already exist eighty lines
away. Adding a column would mean editing the migration, the record, the DTO and the `From`
impl, and forgetting the last two is a silent omission from the frontend rather than a
compile error. Moving the duplication from Rust→TypeScript to Rust→Rust makes it
type-checked, which is better, but it does not make it *right*, and it does not make it
less work to keep honest.

**The shapes do not actually differ.** `uuid` with its `serde` feature writes a `Uuid` as a
string; `ts-rs` with `uuid-impl` declares it as `string`. The kind enums serialise as their
stored spellings. There is no conversion left for a DTO to perform — the mapping is
`rename_all = "camelCase"`, which `CLAUDE.md` requires to happen in exactly one place, and
one place is what the `entity!` macro gives.

**`rev`, `created_at` and `updated_at` are not leaks.** "Do not expose raw database
internals" means SQL text, file paths and driver errors, all of which are already
intercepted by `DbError` and by the command layer's error contract. A revision number is a
domain fact the interface will need for optimistic concurrency, and the timestamps are
displayed on the School Details screen today.

## Consequences

- `evara-db` and `evara-core` gain a `ts` feature, off by default, so the CLI and the
  solver never compile the generator. The desktop crate turns it on.
- The type generator lives at `apps/desktop/src-tauri/src/bin/typegen.rs` with
  `required-features = ["ts"]`, **not** in a crate of its own. A separate crate was built
  first and abandoned: a `staticlib`/`cdylib` library cannot be linked into a binary in
  another package on Windows MSVC — the attempt loses the default C runtime and fails with
  dozens of unresolved CRT symbols — and the error contract it must generate is defined in
  the Tauri crate anyway. `required-features` keeps it out of `cargo build` and out of
  `tauri build`, so no release bundle contains it.
- `npm run types:generate` writes `packages/domain/src/generated/`; `npm run types:check`
  fails if what is committed disagrees with the Rust. Both `npm run check` and CI run the
  latter, which is the enforcement ADR 0009 asked for.
- The generated output is **committed**, as ADR 0009 requires. The Phase 0 `.gitignore`
  entry that excluded it was written before that ADR existed and has been removed;
  `.gitattributes` pins the line endings so a byte comparison is meaningful on Windows.
- A schema change now reaches the frontend as a type error at the point of use, which is
  the whole point.
- The two kind enums are the one place a list is still written twice: `stored_as!` holds
  the SQL spelling and `#[serde(rename)]` holds the wire spelling. Generating the enums
  from a single macro would have cost their per-variant documentation, so drift is made a
  red test instead — `kinds_match_their_stored_spelling` asserts the two agree for every
  variant.

## Alternatives considered

**A DTO layer in the Tauri crate.** The textbook answer, and right when a wire format must
be insulated from a storage format — a public API, or two teams shipping on different
schedules. Here the two are one application built from one repository, and the insulation
would buy nothing while costing the drift this project has already been bitten by once.

**DTOs in `evara-core`.** The right home for shared domain types, and where the entity
model will eventually live. Rejected for Phase 1E as scope: `evara-db`'s records are
generated by the `entity!` macro and threading `evara-core` types through every repository
signature is a refactor of all 190 persistence tests, not the "smallest necessary
extension" the phase brief asked for.

**Deriving `TS` unconditionally rather than behind a feature.** Simpler, and the runtime
cost is negligible. Rejected because it would put a code generator in the dependency tree
of `evara-solver` and `evara-cli`, which have no business knowing TypeScript exists.
