/**
 * The one definition of the school form.
 *
 * Two screens edit a school and they must ask for the same things in the same way: the
 * **Schools** section, which creates and deletes them for the project, and **School
 * details**, which edits the one that is currently selected and carries its logo. Writing
 * the fields twice would be the second definition `CLAUDE.md` forbids, and the field that
 * would drift first is `code` — the one Stage 0C adds.
 *
 * ## Why the option lists are a function of the school
 *
 * A school saved with a timezone this build does not offer — an older IANA spelling, a
 * zone added since, something typed by hand when these were free-text boxes — must still
 * show its own value, and show it as *its own value* rather than as a blank field that
 * quietly becomes something else on the next save. `withExistingValue` does that, and it
 * needs the record.
 */
import type { School, SchoolInput } from "@evara/domain";
import { withExistingValue } from "../ui/Combobox";
import type { FieldSpec } from "../ui/Field";
import { list, text, textOrNull, type Draft } from "../validation";
import { currentLocale, localeOptions } from "./locales";
import {
  canonicalTimezone,
  currentTimezone,
  isLegacyTimezone,
  timezoneName,
  timezoneOptions,
} from "./timezones";
import { MONDAY_TO_FRIDAY, toWeekdayNumbers, toWeekdayValues } from "./weekdays";

/**
 * How a stored timezone that the list does not offer is described.
 *
 * Says what it is as well as that it is unlisted, and where the name has since been
 * retired, says what replaced it — which is the one piece of information that turns
 * "this looks wrong" into "this is fine, and here is the modern spelling if you want it".
 * Still does not change the value: see {@link isLegacyTimezone}.
 */
export function existingTimezone(value: string): string {
  const name = timezoneName(value);
  const named = name === value ? value : `${name} — ${value}`;
  const renamed = isLegacyTimezone(value) ? `; renamed to ${canonicalTimezone(value)}` : "";
  return `${named} (existing value, not in this list${renamed})`;
}

/** The same, for a locale tag. There is no rename table for these. */
export function existingLocale(value: string): string {
  return `${value} (existing value, not in this list)`;
}

/**
 * The school form's fields, offering every value the given schools already hold.
 *
 * Takes a *list* rather than the one school being edited, because the Schools section's
 * form is built from the draft and cannot know which row it was opened on — and because
 * offering every stored value is harmless: an option already in the list is not added
 * twice, and a project where two schools both sit in unlisted zones needs both. An empty
 * list is the new-school case and gives the plain option lists.
 */
export function schoolFields(schools: readonly School[]): readonly FieldSpec[] {
  const timezones = schools.reduce(
    (options, school) => withExistingValue(options, school.timezone, existingTimezone),
    timezoneOptions(),
  );
  const locales = schools.reduce(
    (options, school) => withExistingValue(options, school.locale, existingLocale),
    localeOptions(),
  );

  return [
    {
      kind: "text",
      key: "name",
      label: "School name",
      required: true,
      placeholder: "Northgate Secondary",
      hint: "Unique within this campus file — it is what every school selector shows.",
    },
    {
      kind: "text",
      key: "code",
      label: "School code",
      placeholder: "NGS",
      hint: "Optional, and unique within the campus file when given. A short label for reports and selectors where a full name will not fit.",
    },
    {
      kind: "combobox",
      key: "timezone",
      label: "Time zone",
      required: true,
      placeholder: "Search by city or name",
      emptyLabel: "No time zone matches that",
      choices: timezones,
      hint: "An IANA time zone. Period times are local wall clock, so this is what they are local to.",
    },
    {
      kind: "combobox",
      key: "locale",
      label: "Locale",
      required: true,
      placeholder: "Search by language or region",
      emptyLabel: "No locale matches that",
      choices: locales,
      hint: "A BCP-47 tag, used for formatting dates and numbers in reports. Independent of the time zone.",
    },
    {
      kind: "weekdays",
      key: "workingDays",
      label: "School working days",
      required: true,
      hint: "The weekdays this school normally teaches on. This is the school's calendar week — it does not set the cycle's length, and it does not decide which cycle day a date is. A six-day rotation across a five-day week is perfectly ordinary, and dates are still mapped one at a time in Calendar Mapping.",
    },
  ];
}

/** The draft a school's form opens with. */
export function schoolDraft(school: School | null): Draft {
  return {
    name: school?.name ?? "",
    code: school?.code ?? "",
    // The machine's own settings as a starting point for a *new* school only. An
    // existing school keeps exactly what it was saved with; see `schoolFields`.
    timezone: school?.timezone ?? currentTimezone(),
    locale: school?.locale ?? currentLocale(),
    // Monday to Friday for a new school, and whatever is stored for an existing one. A
    // project upgraded from a schema without the column already reads as Monday to
    // Friday, so there is no second default to keep in step here — the migration's
    // `DEFAULT 31` is the one that matters.
    workingDays: toWeekdayValues(school?.workingDays ?? MONDAY_TO_FRIDAY),
  };
}

/**
 * What the backend is sent.
 *
 * `code` comes from the form now. Before Stage 0C the form did not collect one and
 * `SchoolDetails` carried the stored value through by hand so a save could not clear it;
 * the field replaces that workaround rather than sitting beside it. A blank box means *no
 * code*, which is a real state — `code` is nullable and unique only when given — so
 * `textOrNull` is the right reading and the empty string must never reach the column.
 */
export function schoolInput(draft: Draft): SchoolInput {
  return {
    name: text(draft, "name"),
    code: textOrNull(draft, "code"),
    timezone: text(draft, "timezone"),
    locale: text(draft, "locale"),
    workingDays: toWeekdayNumbers(list(draft, "workingDays")),
  };
}
