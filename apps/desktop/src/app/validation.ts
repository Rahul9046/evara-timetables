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

import { DATE_FORMAT, isRealDate, readTypedDate } from "./dates";

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
 * A canonical `YYYY-MM-DD` date — the form a draft holds and the backend is sent.
 *
 * Canonical matters: the column stores `date(date)` and compares as text, so `2027-9-1`
 * would sort and range-query wrongly even though a human reads it the same. The user
 * never types this form — the date field shows and accepts `DD/MM/YYYY` and converts —
 * so a value reaching here uncanonical means the conversion declined it.
 *
 * Which is why there are two different complaints rather than one. The field passes text
 * it could not parse straight through instead of guessing, so this has to tell apart:
 *
 * - `31/02/2026` or `2027-02-29`: a date, spelled plainly, naming a day that does not
 *   exist. Never silently corrected to the 3rd of March or the 1st of March — the user
 *   is told, because only they know which day they meant.
 * - anything else: not a date, and the format is what needs saying.
 */
export function isoDate(value: string | null | undefined, label: string): string | null {
  const text = (value ?? "").trim();
  if (text.length === 0) return `${label} is required.`;

  const typed = readTypedDate(text);
  if (typed !== null && !isRealDate(typed)) return `${label} is not a real date.`;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(text)) {
    return `${label} must be a date like ${DATE_FORMAT}.`;
  }
  return null;
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
export type Draft = Record<string, string | boolean | readonly string[] | null>;

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

/**
 * Reads a draft field as a number, or null when blank — for a nullable numeric column.
 *
 * Blank means *unset*, never zero. A teacher with no period ceiling and a teacher who may
 * teach no periods are different claims, and defaulting the first to the second would
 * quietly write a rule the school never asked for.
 */
export function numberOrNull(draft: Draft, key: string): number | null {
  const value = text(draft, key);
  return value.length === 0 ? null : Number(value);
}

/** Reads a draft field as a boolean. */
export function flag(draft: Draft, key: string): boolean {
  return draft[key] === true;
}

/**
 * Reads a draft field as a list of identifiers — what a multi-select holds.
 *
 * Returns a fresh array so a caller cannot mutate the draft by accident, and an empty one
 * for a field that was never touched, because "nothing chosen" and "no such field" mean
 * the same thing to every caller that exists.
 */
export function list(draft: Draft, key: string): string[] {
  const value = draft[key];
  return Array.isArray(value) ? [...value] : [];
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

/**
 * The school record.
 *
 * `workingDays` is here for the same reason every other check in this file is: so the user
 * is told immediately rather than after a round trip. It restates a `CHECK` the schema
 * also enforces — `working_days BETWEEN 1 AND 127`, where the lower bound *is* "at least
 * one day" — so a disagreement is impossible to act on wrongly, and the backend remains
 * the authority.
 */
export function validateSchool(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "School name")],
    ["timezone", required(text(draft, "timezone"), "Time zone")],
    ["locale", required(text(draft, "locale"), "Locale")],
    [
      "workingDays",
      list(draft, "workingDays").length === 0
        ? "Choose at least one working day."
        : null,
    ],
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

/** A year level. */
export function validateYearLevel(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Year level name")],
    ["ordinal", wholeNumber(text(draft, "ordinal"), "Position", { min: 0 })],
  ]);
}

/**
 * A teacher, as the setup form asks for them.
 *
 * Only the two names are required. A staff code is optional because plenty of schools do
 * not issue one, and an email is optional because Evara has no accounts and never sends
 * anything — requiring either would be asking for data to satisfy a form rather than the
 * school.
 *
 * **The four workload values are deliberately not checked here.** Stage A took them off the
 * form; they are carried from the stored record into the draft and back out unchanged, so
 * they are no longer *input*, and validating them would be validating something the user
 * can neither see nor correct. An error attached to an absent field is worse than no
 * check: it is a form that will not submit with nothing on screen to explain why.
 *
 * Their `CHECK` constraints in `V10` — `fte_percent BETWEEN 0 AND 100` and `>= 0` on each
 * ceiling — are the authority, as they always were. {@link validateTeacherWorkload} is
 * waiting for the Phase 4 screen that asks the question properly.
 */
export function validateTeacherIdentity(draft: Draft): Problems {
  return problems([
    ["givenName", required(text(draft, "givenName"), "First name")],
    ["familyName", required(text(draft, "familyName"), "Last name")],
  ]);
}

/**
 * The workload half: full-time equivalent and the three load ceilings.
 *
 * Not reachable from any screen today — Stage A removed the fields, and Phase 4's load
 * constraints are what will put them back, on a screen that can show what a ceiling *does*.
 * Kept because the rules are not obvious and were worked out once: a blank ceiling means
 * *no ceiling* rather than zero, and FTE is a percentage rather than a fraction because the
 * Phase 4 constraints sum and compare it as an integer.
 *
 * Exported and tested rather than commented out, so it cannot rot unnoticed in the way a
 * block of dead code would.
 */
export function validateTeacherWorkload(draft: Draft): Problems {
  return problems([
    ["ftePercent", wholeNumber(text(draft, "ftePercent"), "Full-time equivalent", { min: 0, max: 100 })],
    ["maxPeriodsPerDay", optional(text(draft, "maxPeriodsPerDay"), (value, name) => wholeNumber(value, name, { min: 0 }), "Periods per day")],
    ["maxPeriodsPerCycle", optional(text(draft, "maxPeriodsPerCycle"), (value, name) => wholeNumber(value, name, { min: 0 }), "Periods per cycle")],
    ["maxConsecutivePeriods", optional(text(draft, "maxConsecutivePeriods"), (value, name) => wholeNumber(value, name, { min: 0 }), "Consecutive periods")],
  ]);
}

/**
 * One student's spell at one school.
 *
 * Everything is optional, and that is the model rather than a lenient form. An admission
 * number is the school's own and plenty of schools do not issue one; a year level and a
 * campus are classifications a school may not track; and a joining date is a fact some
 * schools simply do not record — `V13` made `enrolled_on` nullable for that reason, and a
 * form that insisted would be answered with invented dates.
 *
 * So the only checks are shape: a date that was typed must be a date, and it must not end
 * before it began. Which year levels and campuses are *this school's* is a cross-school
 * rule the repository owns and refuses by name; nothing here guesses at it.
 */
export function validateEnrolment(draft: Draft): Problems {
  const found = problems([
    [
      "enrolledOn",
      optional(text(draft, "enrolledOn"), (value, label) => isoDate(value, label), "Joining date"),
    ],
    [
      "leftOn",
      optional(text(draft, "leftOn"), (value, label) => isoDate(value, label), "Leaving date"),
    ],
  ]);

  if (found["enrolledOn"] !== undefined || found["leftOn"] !== undefined) return found;
  const joined = text(draft, "enrolledOn");
  const left = text(draft, "leftOn");
  if (joined.length > 0 && left.length > 0 && left < joined) {
    return { ...found, leftOn: "A student cannot leave before the date they joined." };
  }
  return found;
}

/** A student. Only the two names are required, for the same reasons as a teacher. */
export function validateStudent(draft: Draft): Problems {
  return problems([
    ["givenName", required(text(draft, "givenName"), "First name")],
    ["familyName", required(text(draft, "familyName"), "Last name")],
  ]);
}

/**
 * A student group.
 *
 * `size` is optional and means something specific when left blank: "the membership list is
 * the truth". A blank is therefore not an incomplete form, and is not defaulted to zero.
 */
export function validateStudentGroup(draft: Draft): Problems {
  return problems([
    ["name", required(text(draft, "name"), "Group name")],
    ["kind", required(text(draft, "kind"), "Kind")],
    ["size", optional(text(draft, "size"), (value, name) => wholeNumber(value, name, { min: 0 }), "Size")],
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
 *
 * `weekCount` is deliberately absent. The form no longer asks for it, so there is no
 * input to attach a message to and nothing a user could do about one. The value a save
 * sends is supplied by `CyclesAndCycleDays.weekCountFor`, which cannot produce an invalid
 * one; the column's `CHECK (week_count >= 1)` remains the authority either way.
 */
export function validateCycle(draft: Draft): Problems {
  // A weekly cycle has no length to give. It is seven days because a week is seven days,
  // the backend derives it from the type, and the form does not show the field — so
  // insisting on a value here would mark a field that is not on screen. The *rule* that a
  // week is seven days stays in `evara_core::calendar`; this only declines to ask.
  if (text(draft, "kind") === "WEEKLY") {
    return problems([["name", required(text(draft, "name"), "Cycle name")]]);
  }
  return problems([
    ["name", required(text(draft, "name"), "Cycle name")],
    ["dayCount", wholeNumber(text(draft, "dayCount"), "Rotation length", { min: 1 })],
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
