/**
 * `@evara/domain` — TypeScript view of the Evara domain.
 *
 * ## This package is mostly build output
 *
 * The domain is defined **once, in Rust** (`crates/evara-core`). DTO types that cross
 * the Tauri IPC boundary are generated from those Rust types into `./generated/` and
 * re-exported here.
 *
 * Do not hand-write an entity type in this package. The previous version of this file
 * did, and it had already drifted out of step with the Rust/Java model it mirrored —
 * `teacherIds: ID[]` on one side, a single `teacherId` on the other. One definition,
 * generated, is the fix.
 *
 * Hand-written code is allowed here only for things that have no Rust counterpart:
 * type guards, discriminated-union helpers and display formatting.
 *
 * Type generation is wired up in Phase 2, together with the first entities. Until then
 * `./generated/` is empty and this module exports only the IPC envelope below.
 */

/**
 * Build and version facts reported by the backend.
 *
 * Mirrors `AppInfo` in `apps/desktop/src-tauri/src/commands/mod.rs`. This is the one
 * hand-written mirror in the codebase, and it exists only so Phase 0 can prove the IPC
 * round trip before the generator is in place. Phase 2 replaces it with generated output.
 */
export interface AppInfo {
  /** Application version, from the Rust crate manifest. */
  readonly version: string;
  /** Whether the backend is a debug build. */
  readonly debug: boolean;
  /** Roadmap phase the codebase has reached. */
  readonly phase: string;
}

/** Narrows an unknown IPC payload to {@link AppInfo}. */
export function isAppInfo(value: unknown): value is AppInfo {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v['version'] === 'string' &&
    typeof v['debug'] === 'boolean' &&
    typeof v['phase'] === 'string'
  );
}
