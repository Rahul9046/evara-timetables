/**
 * Input validation for the School Setup forms.
 *
 * ## What belongs here, and what emphatically does not
 *
 * `CLAUDE.md` forbids a scheduling rule in TypeScript, or any rule implemented twice.
 * That line is worth holding precisely, because "validate the form" is the usual way it
 * gets crossed.
 *
 * **Here:** the shape of a single field, and ordering between two fields of the same form.
 * Is the name blank? Is the capacity a non-negative whole number? Is the time
 * `HH:MM`? Is the end after the start? These exist so the user is told immediately
 * instead of after a round trip, and every one of them is a restatement of a `CHECK`
 * constraint the database also enforces — which is why a disagreement is impossible to
 * act on wrongly: the backend is authoritative and its message is what gets shown.
 *
 * **Not here, ever:**
 *
 * - uniqueness — needs the other rows, and `DuplicateValue` already names the field
 * - whether an ordinal fits the cycle's declared length — a model rule, in `evara-db`
 * - whether a cycle is complete — `evara_core::time::CycleCoverage`, asked over IPC
 * - whether a delete is allowed — `RESTRICT`, discovered by trying
 * - anything about timeslots, orphans or materialisation
 *
 * A function in this file never takes a list of existing records. If it needs one, it is
 * the wrong kind of rule and belongs in Rust.
 *
 * ## Shape
 *
 * Each validator returns a map of field key to message, empty when the draft is fine.
 * Pure, so the tests need no DOM and no backend.
 */

/** Field-keyed validation messages. Empty means valid. */
export type Problems = Record<string, string>;

/** Whether a draft passed validation. */
export function isValid(problems: Problems): boolean {
  return Object.keys(problems).length === 0;
}

// ------------------------------------------------------------------- field primitives

/** A value that is present and not only whitespace. */
export function required(value: string | null | undefined, label: string): string | null {
  return value !== null && value !== undefined && value.trim().length > 0
    ? null
    : `${label} is required.`;
}

/**
 * A whole number within bounds.
 *
 * Takes the raw string a text input holds, because `<input type="number">` reports an
 * empty or partially typed value as `""` and parsing it to `NaN` silently would turn a
 * blank field into a confusing backend error.
 */
export function wholeNumber(
  raw: string | number | null | undefined,
  label: string,
  options: { min?: number; max?: number } = {},
): string | null {
  const text = typeof raw === "number" ? String(raw) : (raw ?? "").trim();
  if (text.length === 0) return `${label} is required.`;
  if (!/^-?\d+$/.test(text)) return `${label} must be a whole number.`;

  const value = Number(text);
  const { min, max } = options;
  if (min !== undefined && value < min) return `${label} cannot be less than ${min}.`;
  if (max !== undefined && value > max) return `${label} cannot be more than ${max}.`;
  return null;
}

/**
 * A zero-padded 24-hour wall clock, `HH:MM`.
 *
 * The pattern matches the column's `CHECK`: `09:30` is accepted, `9:30` is not, and
 * neither is `25:00`. The schema rejects both too; this is only so the user finds out
 * while they are still looking at the field.
 */
export function wallClock(value: string | null | undefined, label: string): string | null {
  const text = (value ?? "").trim();
  if (text.length === 0) return `${label} is required.`;
  if (!/^\d{2}:\d{2}$/.test(text)) {
    return `${label} must be a 24-hour time like 09:30.`;
  }
  const [hours, minutes] = text.split(":").map(Number) as [number, number];
  if (hours > 23 || minutes > 59) return `${label} is not a real time.`;
  return null;
}

/**
 * A canonical `YYYY-MM-DD` date.
 *
 * Canonical matters: the column stores `date(date)` and compares as text, so `2027-9-1`
 * would sort and range-query wrongly even though a human reads it the same.
 */
export function isoDate(value: string | null | undefined, label: string): string | null {
  const text = (value ?? "").trim();
  if (text.length === 0) return `${label} is required.`;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(text)) {
    return `${label} must be a date like 2027-09-01.`;
  }

  // Round-tripping through Date catches 2027-02-31, which the pattern cannot.
  const [year, month, day] = text.split("-").map(Number) as [number, number, number];
  const parsed = new Date(Date.UTC(year, month - 1, day));
  const real =
    parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day;
  return real ? null : `${label} is not a real date.`;
}

/** An optional field: validated only when something was typed. */
export function optional(
  value: string | null | undefined,
  check: (value: string, label: string) => string | null,
  label: string,
): string | null {
  const text = (value ?? "").trim();
  return text.length === 0 ? null : check(text, label);
}

// ------------------------------------------------------------------------ form drafts

/**
 * A draft is the form's own state: every field a string, because that is what an input
 * holds. Converting to the typed input the backend wants happens once, on submit, after
 * validation has passed.
 */
export type Draft = Record<string, string | boolean | null>;

/** Reads a draft field as trimmed text. */
export function text(draft: Draft, key: string): string {
  const value = draft[key];
  return typeof value === "string" ? value.trim() : "";
}

/** Reads a draft field as text, or null when blank — for a nullable column. */
export function textOrNull(draft: Draft, key: string): string | null {
  const value = text(draft, key);
  return value.length === 0 ? null : value;
}

/** Reads a draft field as a number. Call only after `wholeNumber` has passed. */
export function number(draft: Draft, key: string): number {
  return Number(text(draft, key));
}

/** Reads a draft field as a boolean. */
export function flag(draft: Draft, key: string): boolean {
  return draft[key] === true;
}

/** Collects the non-null messages into a {@link Problems} map. */
export function problems(entries: Array<[string, string | null]>): Problems {
  const found: Problems = {};
  for (const [key, message] of entries) {
    if (message !== null) found[key] = message;
  }
  return found;
}

// ------------------------------------------------------------------ per-form validators

/** The school record. */
export function validateSchool(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "School name")],
    ["timezone", required(text(draft, "timezone"), "Time zone")],
    ["locale", required(text(draft, "locale"), "Locale")],
  ]);
}

/** A campus. */
export function validateCampus(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Campus name")],
    ["code", required(text(draft, "code"), "Code")],
  ]);
}

/** A building. */
export function validateBuilding(draft: Draft): Problems {
  return problems([["name", required(text(draft, "name"), "Building name")]]);
}

/** A room type. */
export function validateRoomType(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Room type name")],
    ["code", required(text(draft, "code"), "Code")],
  ]);
}

/** A room. `buildingId` and `roomTypeId` are deliberately not required. */
export function validateRoom(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Room name")],
    ["code", required(text(draft, "code"), "Code")],
    ["capacity", wholeNumber(text(draft, "capacity"), "Capacity", { min: 0 })],
  ]);
}

/** A resource. */
export function validateResource(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Resource name")],
    ["code", required(text(draft, "code"), "Code")],
    ["quantity", wholeNumber(text(draft, "quantity"), "Quantity", { min: 0 })],
  ]);
}

/** An academic year. */
export function validateAcademicYear(draft: Draft): Problems {
  const found = problems([
    ["name", required(text(draft, "name"), "Year name")],
    ["startsOn", isoDate(text(draft, "startsOn"), "Start date")],
    ["endsOn", isoDate(text(draft, "endsOn"), "End date")],
  ]);
  return withDateOrder(found, draft);
}

/** A term. */
export function validateTerm(draft: Draft): Problems {
  const found = problems([
    ["name", required(text(draft, "name"), "Term name")],
    ["ordinal", wholeNumber(text(draft, "ordinal"), "Position", { min: 1 })],
    ["startsOn", isoDate(text(draft, "startsOn"), "Start date")],
    ["endsOn", isoDate(text(draft, "endsOn"), "End date")],
  ]);
  return withDateOrder(found, draft);
}

/**
 * Adds the end-after-start check, but only when both dates are already well formed.
 *
 * Reporting "the end is before the start" on top of "that is not a real date" gives the
 * user two messages for one mistake.
 */
function withDateOrder(found: Problems, draft: Draft): Problems {
  if (found["startsOn"] !== undefined || found["endsOn"] !== undefined) return found;
  if (text(draft, "endsOn") < text(draft, "startsOn")) {
    return { ...found, endsOn: "The end date cannot be before the start date." };
  }
  return found;
}

/**
 * A cycle.
 *
 * `dayCount` is the declared length and may exceed the number of days that exist — that
 * is a legal intermediate state, reported by the coverage panel rather than refused here.
 */
export function validateCycle(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Cycle name")],
    ["dayCount", wholeNumber(text(draft, "dayCount"), "Days in the cycle", { min: 1 })],
    ["weekCount", wholeNumber(text(draft, "weekCount"), "Weeks spanned", { min: 1 })],
  ]);
}

/**
 * A cycle day.
 *
 * Nothing here compares `ordinal` against the cycle's declared length: that needs the
 * cycle, so it is the repository's rule and arrives as `Invalid` if broken.
 */
export function validateCycleDay(draft: Draft): Problems {
  return problems([
    ["ordinal", wholeNumber(text(draft, "ordinal"), "Position", { min: 1 })],
    ["label", required(text(draft, "label"), "Label")],
    [
      "weekdayHint",
      optional(
        text(draft, "weekdayHint"),
        (value, label) => wholeNumber(value, label, { min: 1, max: 7 }),
        "Weekday hint",
      ),
    ],
  ]);
}

/** A bell schedule. */
export function validatePeriodStructure(draft: Draft): Problems {
  return problems([["name", required(text(draft, "name"), "Schedule name")]]);
}

/** A period. */
export function validatePeriod(draft: Draft): Problems {
  const found = problems([
    ["ordinal", wholeNumber(text(draft, "ordinal"), "Position", { min: 1 })],
    ["label", required(text(draft, "label"), "Label")],
    ["startsAt", wallClock(text(draft, "startsAt"), "Start time")],
    ["endsAt", wallClock(text(draft, "endsAt"), "End time")],
  ]);

  if (found["startsAt"] !== undefined || found["endsAt"] !== undefined) return found;
  if (text(draft, "endsAt") <= text(draft, "startsAt")) {
    return { ...found, endsAt: "The end time must be after the start time." };
  }
  return found;
}

/** A calendar day. A blank cycle day is meaningful: it marks a day with no lessons. */
export function validateCalendarDay(draft: Draft): Problems {
  return problems([["date", isoDate(text(draft, "date"), "Date")]]);
}
