# ADR 0009 — `ts-rs` for generated TypeScript types; keep hand-written command wrappers

- Status: **Accepted** (approved 2026-10-06, resolves OPEN-DECISIONS D3)
- Date: 2026-10-06

## Context

[ADR 0001](0001-local-first-desktop.md) settled that the domain is defined once, in Rust,
and that TypeScript types crossing the IPC boundary are *generated* rather than
hand-maintained — because the original repository had already proved the alternative,
with `packages/domain` claiming `teacherIds: ID[]` while the Rust model carried a single
`teacherId`.

D3 left the tool open: `ts-rs` (types only) versus `tauri-specta` (types **and** typed
command wrappers). It was deferred until real entities existed. Phase 1C produced eight of
them, so the comparison can now be made against something concrete.

## Decision

**`ts-rs`.** Generate TypeScript types from the Rust DTOs; keep writing the per-command
wrappers in `apps/desktop/src/ipc/` by hand.

Three things decide it.

**Stability.** `ts-rs` is at a stable 12.0.1. `tauri-specta` is still `2.0.0-rc.25`, and
`specta` with it — a release candidate, pinned to a Tauri version. An open-source project
that must stay buildable by contributors for years should not take a long-running RC as a
load-bearing build dependency, particularly one that would sit in the path of every
command.

**The wrappers already earn their keep.** `tauri-specta`'s headline benefit is generating
the call layer. We have that layer already, and it does something generated bindings do
not: it *validates*. `isAppInfo` and `isProjectError` check what actually came back
instead of asserting a type over an `unknown`. A generated wrapper would give compile-time
safety against a signature mismatch while still trusting the payload at runtime. Phase 1B's
IPC tests exist precisely because that distinction matters.

**Blast radius.** `specta` requires a `Type` derive on every type that crosses the
boundary, including error enums, and it reaches into the command macro. `ts-rs` is a derive
and a build step over plain structs; if it is ever wrong, it is wrong in one direction and
can be replaced by checking in the generated file.

## Consequences

- `packages/domain/src/generated/` fills from a `ts-rs` export step; `packages/domain`
  stays what ADR 0001 said it is — build output plus hand-written guards.
- The command *name* and argument shape stay unchecked across the boundary. That is the
  real cost of this choice, and it is paid by the per-command wrappers plus their tests:
  one wrapper per command, each asserting the name and payload it sends.
- Generated output must be committed, so a clean checkout type-checks without running
  Cargo, and a diff shows when a Rust change alters the frontend contract.
- A CI check should regenerate and fail on drift. Without it, "generated" degrades into
  "hand-maintained with extra steps", which is the failure mode ADR 0001 exists to avoid.
- Revisitable: if `tauri-specta` reaches a stable release and the command surface has grown
  enough that mismatches become a real cost, swapping is additive — the generated types
  would simply come from a different generator.

**Nothing is wired up in Phase 1C.** This is a decision, not an implementation: the brief
was explicit that no broad frontend entity bindings arrive in this phase. Generation is set
up in Phase 2, alongside the first screens that consume the entities.

## Alternatives considered

**`tauri-specta`.** The better tool the day it is stable and the command surface is large.
Today it would mean taking an RC dependency to replace a layer that already exists and does
more than the generated version would.

**Hand-written types.** Already tried in this repository, already drifted.

**JSON Schema as the interchange, generating both sides.** Another generator and another
format in the middle, for a boundary that is internal to one application.
