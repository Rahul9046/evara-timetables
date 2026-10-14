/**
 * Reading across every school in the project.
 *
 * Every repository list is scoped to one school, deliberately — ADR 0019 removed
 * `Structure::school()` precisely so no read could quietly be about an arbitrary
 * institution. Stage 0C's screens nonetheless need three project-wide answers:
 *
 * - **every campus**, because a building, a room or a resource may be shared with a campus
 *   of another school, and a picker that offered only the active school's campuses would
 *   make cross-school sharing unreachable from the interface that exists to expose it;
 * - **every teacher**, so "add an existing teacher" can find somebody this school has
 *   never employed;
 * - **every student**, for the same reason, and for the transfer workflow.
 *
 * ## Composed here rather than added to the backend
 *
 * None of these needs a new command. A teacher's membership set always contains their home
 * school, so the union of `listTeachers` over every school is exactly every teacher in the
 * project; the same holds for `listStudents`, since `create_student` writes a home spell.
 * Fanning out over a handful of schools is a few round trips over a local SQLite file, and
 * the alternative — a project-wide command — would be a second way to ask one question,
 * with the standing risk that a screen reaches for the unscoped one by accident.
 *
 * The functions here are therefore **reads only**. Nothing in this module writes, and no
 * mutation anywhere in Stage 0C is built on a project-wide identifier without naming the
 * school it acts for.
 */
import { useCallback, useMemo } from "react";

import type { Campus, School, Student, Teacher } from "@evara/domain";
import { listCampuses, listStudents, listTeachers } from "../../ipc/setup";
import type { Choice } from "../ui/Field";
import { useList } from "./useList";

/** A campus, with the school that owns it resolved. */
export interface CampusOfSchool {
  readonly campus: Campus;
  readonly school: School;
}

/**
 * Every campus in the project, grouped under its school, schools in list order.
 *
 * `listCampuses` is school-scoped, so this is the fan-out. One failing school does not
 * take the whole picker with it: the message is surfaced and the other schools' campuses
 * are still offered, because a share that cannot be *seen* is worse than one that cannot
 * be made.
 */
export function useProjectCampuses(schools: readonly School[]): {
  readonly all: readonly CampusOfSchool[] | null;
  readonly error: string | null;
  readonly reload: () => void;
} {
  // The identifiers, not the records: a re-read of the schools list produces new objects
  // for the same schools, and depending on their identity would refetch every campus in
  // the project on each one.
  const key = schools.map((school) => school.id).join(",");

  const load = useCallback(async (): Promise<readonly CampusOfSchool[]> => {
    const perSchool = await Promise.all(
      schools.map((school) =>
        listCampuses(school.id).then((campuses) =>
          campuses.map((campus) => ({ campus, school })),
        ),
      ),
    );
    return perSchool.flat();
    // `key` rather than `schools`: it is the identity of the list that actually decides
    // which campuses to read, and the array is a fresh object on every parent render.
  }, [key]);

  const listed = useList<CampusOfSchool>(load);
  return { all: listed.items, error: listed.error, reload: listed.reload };
}

/**
 * How a campus is named where one is being *read* — a table cell, a confirmation.
 *
 * Its own name, and the school's as well when the campus file holds more than one. A
 * single-school project sees exactly what it saw before Stage 0C: there is no other
 * institution to tell it apart from, and a suffix naming the only school would be noise on
 * every row.
 */
export function campusLabel(entry: CampusOfSchool, multiSchool: boolean): string {
  return multiSchool ? `${entry.campus.name} — ${entry.school.name}` : entry.campus.name;
}

/**
 * How a campus is offered where one is being *chosen*.
 *
 * The code as well, because a picker is where two similarly named sites have to be told
 * apart, and the school's name too when there is more than one — the combobox filters on
 * the label, so typing a school's name narrows to its campuses, which is how somebody
 * actually looks for "Southmoor's sports hall".
 */
export function campusChoiceLabel(entry: CampusOfSchool, multiSchool: boolean): string {
  const base = `${entry.campus.name} (${entry.campus.code})`;
  return multiSchool ? `${base} — ${entry.school.name}` : base;
}

/** Campus choices for a share picker, excluding the campuses already accounted for. */
export function campusChoices(
  all: readonly CampusOfSchool[],
  exclude: readonly string[],
  multiSchool: boolean,
): Choice[] {
  const skip = new Set(exclude);
  return all
    .filter((entry) => !skip.has(entry.campus.id))
    .map((entry) => ({
      value: entry.campus.id,
      label: campusChoiceLabel(entry, multiSchool),
    }));
}

/** A lookup from campus identifier to its name and owning school. */
export function campusIndex(
  all: readonly CampusOfSchool[] | null,
): ReadonlyMap<string, CampusOfSchool> {
  return new Map((all ?? []).map((entry) => [entry.campus.id, entry]));
}

/** The same, memoised, for a component that renders a share list per row. */
export function useCampusIndex(
  all: readonly CampusOfSchool[] | null,
): ReadonlyMap<string, CampusOfSchool> {
  return useMemo(() => campusIndex(all), [all]);
}

/**
 * Every teacher in the project, deduplicated by identifier.
 *
 * One person is one record however many schools employ them, so the same `teacher.id`
 * arriving from two schools' lists collapses to one entry — which is the whole point of
 * ADR 0021 and the thing an "add existing teacher" search must not get wrong.
 */
export async function listProjectTeachers(
  schools: readonly School[],
): Promise<readonly Teacher[]> {
  const perSchool = await Promise.all(schools.map((school) => listTeachers(school.id)));
  return dedupe(perSchool.flat());
}

/** Every student in the project, deduplicated by identifier, for the same reason. */
export async function listProjectStudents(
  schools: readonly School[],
): Promise<readonly Student[]> {
  const perSchool = await Promise.all(schools.map((school) => listStudents(school.id)));
  return dedupe(perSchool.flat());
}

/** Keeps the first record per identifier, in encounter order. */
function dedupe<T extends { id: string }>(rows: readonly T[]): readonly T[] {
  const byId = new Map<string, T>();
  for (const row of rows) {
    if (!byId.has(row.id)) byId.set(row.id, row);
  }
  return [...byId.values()];
}

/** A school's name by identifier, for the many places one has to be spelled out. */
export function schoolNames(schools: readonly School[]): ReadonlyMap<string, string> {
  return new Map(schools.map((school) => [school.id, school.name]));
}

/** How a school is named in running text: its name, with its code when it has one. */
export function describeSchool(school: School | undefined, fallback = "another school"): string {
  if (!school) return fallback;
  return school.code ? `${school.name} (${school.code})` : school.name;
}
