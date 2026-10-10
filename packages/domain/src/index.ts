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
 * Type generation was wired up in Phase 1E. `./generated/` now holds the whole IPC
 * contract — forty types, written by `npm run types:generate` and verified by
 * `npm run types:check`, which CI runs. See
 * [ADR 0009](../../../docs/adr/0009-typescript-type-generation.md) and
 * [ADR 0012](../../../docs/adr/0012-records-are-the-ipc-contract.md).
 */

export type {
  AcademicYear,
  AcademicYearInput,
  AppInfo,
  Building,
  BuildingInput,
  CalendarDay,
  CalendarDayInput,
  CalendarDayKind,
  Campus,
  CampusInput,
  Cycle,
  CycleCoverage,
  CycleDay,
  CycleDayInput,
  CycleInput,
  GridCell,
  GridPreview,
  MaterialisationCounts,
  MaterialisationPlan,
  MaterialisationRequest,
  Period,
  PeriodInput,
  PeriodKind,
  PeriodStructure,
  PeriodStructureInput,
  PlanFingerprint,
  PlannedSlot,
  Resource,
  ResourceInput,
  Room,
  RoomInput,
  RoomType,
  RoomTypeInput,
  School,
  SchoolInput,
  SetupError,
  Term,
  TermInput,
  Timeslot,
  TimeslotInput,
} from './generated/index';

import type { AppInfo } from './generated/index';

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
