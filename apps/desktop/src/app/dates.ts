/**
 * Calendar dates, as the interface has to deal with them.
 *
 * Everything here is pure integer arithmetic on a `{ year, month, day }` triple. That is
 * deliberate and it is the whole point of the module.
 *
 * ## These are dates, not instants
 *
 * `academic_year.starts_on` is "the first day of term", not "midnight somewhere". The
 * column stores `YYYY-MM-DD` and compares as text. A `Date` is a point on a timeline, and
 * the two usual ways of mixing them silently move the date by a day:
 *
 * - `new Date("2027-09-01")` parses as *UTC* midnight. West of Greenwich,
 *   `.getDate()` on it is the 31st of August.
 * - `new Date(2027, 8, 1).toISOString()` builds *local* midnight and then prints it in
 *   UTC. East of Greenwich that is `2027-08-31T…`.
 *
 * So no function in this file ever converts between a `Date` and a date string. `Date` is
 * used only as a calendar calculator, always through `Date.UTC` and `getUTC*`, where the
 * input and the output are in the same frame and the arithmetic is exact — UTC has no
 * daylight saving, so a day really is 86 400 000 milliseconds.
 *
 * The only function that reads a clock is {@link todayDate}, and all it decides is which
 * month a freshly opened calendar shows.
 *
 * ## Two formats, converted in one place
 *
 * `YYYY-MM-DD` is what the model stores and what the IPC contract carries, unchanged.
 * `DD/MM/YYYY` is what a person sees and types. The conversion is this module, called
 * from the date field and from its validator, so neither the backend nor the rest of the
 * frontend has an opinion about display format.
 */

/** What every date field shows, and what the placeholder says. */
export const DATE_FORMAT = "DD/MM/YYYY";

/** A day in the proleptic Gregorian calendar. `month` is 1–12, `day` is 1–31. */
export interface CalendarDate {
  readonly year: number;
  readonly month: number;
  readonly day: number;
}

/** Canonical storage form: four-digit year, zero-padded month and day. */
const ISO = /^(\d{4})-(\d{2})-(\d{2})$/;

/**
 * A typed date, in any of the shapes a person reasonably produces.
 *
 * `/`, `-` and `.` all separate, and the day and month may be one digit, so `1/9/2027`
 * and `01/09/2027` are the same date. A leading four-digit group means the text is
 * year-first instead — which is what a value pasted out of an export, a spreadsheet or
 * this project's own storage looks like.
 */
const TYPED = /^(\d{1,4})[/\-.](\d{1,2})[/\-.](\d{1,4})$/;

/** Month names, for the calendar's heading and its spoken day labels. */
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
] as const;

/** The name of a month, 1-based. */
export function monthName(month: number): string {
  return MONTHS[month - 1] ?? "";
}

/** Every month, as `{ value, label }` for a select. */
export function monthChoices(): ReadonlyArray<{ value: string; label: string }> {
  return MONTHS.map((label, index) => ({ value: String(index + 1), label }));
}

/**
 * Weekday headings, Monday first.
 *
 * Monday first because this is a school calendar and a school week starts on Monday.
 * Note what this is *not*: nothing in scheduling may read a weekday, and nothing does —
 * these seven strings are column headings on a picker, and the value the picker produces
 * is a date, never a day of the week.
 */
export const WEEKDAYS = [
  { short: "Mo", long: "Monday" },
  { short: "Tu", long: "Tuesday" },
  { short: "We", long: "Wednesday" },
  { short: "Th", long: "Thursday" },
  { short: "Fr", long: "Friday" },
  { short: "Sa", long: "Saturday" },
  { short: "Su", long: "Sunday" },
] as const;

/** How many days a month has, accounting for leap years. */
export function daysInMonth(year: number, month: number): number {
  // Day 0 of the *next* month is the last day of this one, which is how this avoids
  // spelling out the leap-year rule a second time.
  return new Date(Date.UTC(year, month, 0)).getUTCDate();
}

/** Whether a triple names a day that exists. Rejects `2027-02-31` and `2027-02-29`. */
export function isRealDate(date: CalendarDate): boolean {
  const { year, month, day } = date;
  if (!Number.isInteger(year) || !Number.isInteger(month) || !Number.isInteger(day)) {
    return false;
  }
  if (year < 1 || year > 9999) return false;
  if (month < 1 || month > 12) return false;
  return day >= 1 && day <= daysInMonth(year, month);
}

/** Formats a triple as canonical `YYYY-MM-DD`. Does not check that the day exists. */
export function toIso(date: CalendarDate): string {
  const year = String(date.year).padStart(4, "0");
  const month = String(date.month).padStart(2, "0");
  const day = String(date.day).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** Reads canonical `YYYY-MM-DD`. Returns `null` for anything else, or for a day that does not exist. */
export function fromIso(value: string | null | undefined): CalendarDate | null {
  const found = ISO.exec((value ?? "").trim());
  if (!found) return null;
  const date = {
    year: Number(found[1]),
    month: Number(found[2]),
    day: Number(found[3]),
  };
  return isRealDate(date) ? date : null;
}

/**
 * Reads a typed date as the numbers the user meant, without judging whether the day exists.
 *
 * Separate from {@link parseTypedDate} so a validator can tell "that is not a date" from
 * "that is a date but there is no such day" — `hello` and `31/02/2026` deserve different
 * messages, and only this distinction can produce them.
 */
export function readTypedDate(value: string | null | undefined): CalendarDate | null {
  const found = TYPED.exec((value ?? "").trim());
  if (!found) return null;

  const first = found[1] as string;
  const middle = Number(found[2]);
  const last = found[3] as string;

  // A four-digit leading group is a year, so the text is `YYYY-MM-DD`; otherwise the
  // leading group is the day. Only one of the two can be four digits long, so there is
  // nothing ambiguous left to guess at.
  if (first.length === 4) {
    return { year: Number(first), month: middle, day: Number(last) };
  }
  if (last.length !== 4) return null;
  return { year: Number(last), month: middle, day: Number(first) };
}

/**
 * Converts what the user typed into the form the model stores.
 *
 * Returns `null` when the text is not a date at all, and also when it is shaped like one
 * but names no real day — the caller must not invent a substitute for either. Silently
 * turning `31/02/2026` into the 3rd of March is the specific failure this guards.
 */
export function parseTypedDate(value: string | null | undefined): string | null {
  const date = readTypedDate(value);
  return date && isRealDate(date) ? toIso(date) : null;
}

/**
 * Renders a stored date for a person: `2027-09-01` becomes `01/09/2027`.
 *
 * Anything that is not canonical storage comes back untouched. That matters more than it
 * looks: while a field is being typed in, its draft value is the half-finished text, and
 * reformatting it would fight the user for the caret.
 */
export function toDisplay(value: string | null | undefined): string {
  const text = (value ?? "").trim();
  const date = fromIso(text);
  if (!date) return text;
  return `${String(date.day).padStart(2, "0")}/${String(date.month).padStart(2, "0")}/${String(date.year).padStart(4, "0")}`;
}

/** A date spelled out, for a button's accessible name: `1 September 2027`. */
export function describeDate(date: CalendarDate): string {
  return `${date.day} ${monthName(date.month)} ${date.year}`;
}

/** Moves by whole days. Exact, because UTC days are all the same length. */
export function addDays(date: CalendarDate, delta: number): CalendarDate {
  const moved = new Date(Date.UTC(date.year, date.month - 1, date.day) + delta * 86_400_000);
  return {
    year: moved.getUTCFullYear(),
    month: moved.getUTCMonth() + 1,
    day: moved.getUTCDate(),
  };
}

/**
 * Moves by whole months, keeping the day of the month where one exists.
 *
 * A month back from the 31st of March is the 28th or 29th of February, not the 2nd or 3rd
 * of March. Clamping rather than overflowing is what makes `PageUp` on a calendar behave
 * the way a person expects.
 */
export function addMonths(date: CalendarDate, delta: number): CalendarDate {
  const zeroBased = date.year * 12 + (date.month - 1) + delta;
  const year = Math.floor(zeroBased / 12);
  const month = (zeroBased % 12) + 1;
  return { year, month, day: Math.min(date.day, daysInMonth(year, month)) };
}

/** Whether two triples are the same day. */
export function sameDate(a: CalendarDate | null, b: CalendarDate | null): boolean {
  if (!a || !b) return false;
  return a.year === b.year && a.month === b.month && a.day === b.day;
}

/** Whether a triple falls in a given month. */
export function inMonth(date: CalendarDate, year: number, month: number): boolean {
  return date.year === year && date.month === month;
}

/**
 * The six-week block a month is drawn in, Monday first.
 *
 * Always 42 cells, including the tail of the previous month and the head of the next, so
 * the popup is the same height whichever month is shown. A calendar that changes height
 * as you page through it moves the control you are about to click.
 */
export function monthGrid(year: number, month: number): CalendarDate[] {
  const first: CalendarDate = { year, month, day: 1 };
  // `getUTCDay` is Sunday-based; shifting by 6 rotates it to Monday-based.
  const offset = (new Date(Date.UTC(year, month - 1, 1)).getUTCDay() + 6) % 7;
  const start = addDays(first, -offset);
  return Array.from({ length: 42 }, (_, index) => addDays(start, index));
}

/**
 * Today, in the machine's own calendar.
 *
 * Read from the local clock rather than UTC on purpose: "today" means the date on the
 * wall, and this is the one place in the module where that is the right frame. It decides
 * which month an empty calendar opens on and which cell is marked as today, and nothing
 * else — it is never stored and never saved.
 */
export function todayDate(): CalendarDate {
  const now = new Date();
  return { year: now.getFullYear(), month: now.getMonth() + 1, day: now.getDate() };
}
