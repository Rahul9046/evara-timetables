/**
 * Weekday names and numbers, for the one setting that has them.
 *
 * ## This is a calendar fact, not a scheduling one
 *
 * The rest of School Setup is careful never to know what a weekday is: a cycle day's
 * `ordinal` is its whole scheduling identity, `weekdayHint` is display only, and
 * `CyclesAndCycleDays` offers no Monday–Friday template because the model has no such
 * concept. None of that changes.
 *
 * "School working days" is a different kind of statement — which weekdays the institution
 * normally teaches on — and `evara_core::calendar::WorkingDays` is where it is defined.
 * This module is only its vocabulary: the ISO number 1..7 that crosses IPC, and the
 * English name a person reads. No rule lives here, and nothing derives a cycle position
 * from any of it.
 *
 * ISO-8601 numbering throughout — 1 Monday through 7 Sunday — because that is what the
 * backend carries and what `cycle_day.weekday_hint` has always used.
 */

/** One weekday, as the interface needs to show it. */
export interface Weekday {
  /** ISO number: 1 Monday through 7 Sunday. */
  readonly value: number;
  /** Full name, for a label and for a screen reader. */
  readonly label: string;
  /** Two letters, for a compact toggle. */
  readonly short: string;
}

/**
 * The week, Monday first.
 *
 * Monday first because the ISO numbering is, so the display order and the stored numbers
 * agree and there is no mapping to get wrong. A school whose week starts on Sunday selects
 * Sunday; it does not need the row reordered to do so.
 */
export const WEEKDAYS: readonly Weekday[] = [
  { value: 1, label: "Monday", short: "Mo" },
  { value: 2, label: "Tuesday", short: "Tu" },
  { value: 3, label: "Wednesday", short: "We" },
  { value: 4, label: "Thursday", short: "Th" },
  { value: 5, label: "Friday", short: "Fr" },
  { value: 6, label: "Saturday", short: "Sa" },
  { value: 7, label: "Sunday", short: "Su" },
];

/** Monday to Friday — what a new school starts with. Matches the column's own default. */
export const MONDAY_TO_FRIDAY: readonly number[] = [1, 2, 3, 4, 5];

/** The name of one ISO weekday, or the number itself if it names no day. */
export function weekdayName(value: number): string {
  return WEEKDAYS.find((day) => day.value === value)?.label ?? String(value);
}

/**
 * A set of ISO weekday numbers, ascending and deduplicated.
 *
 * The form holds its selection as strings, in click order. This is the one conversion to
 * the thing the backend wants, and it drops anything that is not a weekday rather than
 * sending it on to be refused — a draft can only contain what the checkboxes put there, so
 * a stray value means a bug here rather than a user mistake worth a message.
 */
export function toWeekdayNumbers(values: readonly string[]): number[] {
  const found = new Set<number>();
  for (const value of values) {
    const parsed = Number(value);
    if (WEEKDAYS.some((day) => day.value === parsed)) found.add(parsed);
  }
  return [...found].sort((a, b) => a - b);
}

/** The same set as the strings a form's draft holds. */
export function toWeekdayValues(numbers: readonly number[]): string[] {
  return [...numbers]
    .filter((value) => WEEKDAYS.some((day) => day.value === value))
    .sort((a, b) => a - b)
    .map(String);
}

/**
 * A week in prose: "Monday to Friday", "Monday, Wednesday and Friday".
 *
 * Contracts a run of three or more consecutive days, because "Monday to Friday" is how
 * people say it and a list of five names is not. Wrapping runs — Sunday to Thursday — are
 * deliberately *not* contracted: "Sunday to Thursday" would have to be written as a range
 * that reads backwards against the Monday-first order the numbers impose, and spelling the
 * days out is unambiguous where cleverness would not be.
 */
export function describeWeek(numbers: readonly number[]): string {
  const days = [...new Set(numbers)]
    .filter((value) => WEEKDAYS.some((day) => day.value === value))
    .sort((a, b) => a - b);

  if (days.length === 0) return "No working days";
  if (days.length === WEEKDAYS.length) return "Every day";

  const consecutive = days.every(
    (value, index) => index === 0 || value === (days[index - 1] as number) + 1,
  );
  if (consecutive && days.length >= 3) {
    return `${weekdayName(days[0] as number)} to ${weekdayName(days[days.length - 1] as number)}`;
  }

  const names = days.map(weekdayName);
  if (names.length === 1) return names[0] as string;
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}
