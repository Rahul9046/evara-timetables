/**
 * Section 1 — School Details.
 *
 * The one record every other section hangs off. Not a `CrudPanel`: there is exactly one
 * school per project, it is created rather than added, and it is never deleted — the way
 * to discard a school is to discard the project file, which is why the repository offers
 * no `delete_school` and this screen offers no delete button.
 */
import { useCallback, useMemo, useState } from "react";

import type { School } from "@evara/domain";
import { createSchool, describeSetupError, getSchool, updateSchool } from "../../ipc/setup";
import { withExistingValue } from "../ui/Combobox";
import { EntityForm, type FieldSpec } from "../ui/Field";
import { Banner, EmptyState, Facts, Loading } from "../ui/Shell";
import { isValid, text, validateSchool, type Draft, type Problems } from "../validation";
import { currentLocale, localeLabel, localeOptions } from "./locales";
import {
  canonicalTimezone,
  currentTimezone,
  isLegacyTimezone,
  timezoneLabel,
  timezoneName,
  timezoneOptions,
} from "./timezones";
import { useValue } from "./useList";

/**
 * How a stored timezone that the list does not offer is described.
 *
 * Says what it is as well as that it is unlisted, and where the name has since been
 * retired, says what replaced it — which is the one piece of information that turns
 * "this looks wrong" into "this is fine, and here is the modern spelling if you want it".
 * Still does not change the value: see {@link isLegacyTimezone}.
 */
function existingTimezone(value: string): string {
  const name = timezoneName(value);
  const named = name === value ? value : `${name} — ${value}`;
  const renamed = isLegacyTimezone(value)
    ? `; renamed to ${canonicalTimezone(value)}`
    : "";
  return `${named} (existing value, not in this list${renamed})`;
}

/** The same, for a locale tag. There is no rename table for these. */
function existingLocale(value: string): string {
  return `${value} (existing value, not in this list)`;
}

/**
 * The fields, given the school being edited.
 *
 * A function rather than a constant because the option lists have to include whatever
 * the project already holds. A school saved with a timezone this build does not offer —
 * an older IANA spelling, a zone added since, something typed by hand when these were
 * free-text boxes — must still show its own value, and show it as *its own value* rather
 * than as a blank field that quietly becomes something else on the next save.
 */
function fieldsFor(school: School | null): readonly FieldSpec[] {
  return [
    {
      kind: "text",
      key: "name",
      label: "School name",
      required: true,
      placeholder: "Northgate Secondary",
    },
    {
      kind: "combobox",
      key: "timezone",
      label: "Time zone",
      required: true,
      placeholder: "Search by city or name",
      emptyLabel: "No time zone matches that",
      choices: withExistingValue(timezoneOptions(), school?.timezone, existingTimezone),
      hint: "An IANA time zone. Period times are local wall clock, so this is what they are local to.",
    },
    {
      kind: "combobox",
      key: "locale",
      label: "Locale",
      required: true,
      placeholder: "Search by language or region",
      emptyLabel: "No locale matches that",
      choices: withExistingValue(localeOptions(), school?.locale, existingLocale),
      hint: "A BCP-47 tag, used for formatting dates and numbers in reports. Independent of the time zone.",
    },
  ];
}

function draftOf(school: School | null): Draft {
  return {
    name: school?.name ?? "",
    // The machine's own settings as a starting point for a *new* school only. An
    // existing school keeps exactly what it was saved with; see fieldsFor.
    timezone: school?.timezone ?? currentTimezone(),
    locale: school?.locale ?? currentLocale(),
  };
}

export function SchoolDetails(props: { onChanged: () => void }) {
  const load = useCallback(() => getSchool(), []);
  const { value: school, error: loadError, reload } = useValue<School | null>(load);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Four hundred labels, each a formatter call. Built once per school rather than on
  // every keystroke in the form above them.
  const fields = useMemo(() => fieldsFor(school ?? null), [school]);

  const open = useCallback((current: School | null) => {
    setDraft(draftOf(current));
    setProblems({});
    setSaveError(null);
    setEditing(true);
  }, []);

  const submit = useCallback(async () => {
    const found = validateSchool(draft);
    setProblems(found);
    if (!isValid(found)) return;

    const input = {
      name: text(draft, "name"),
      timezone: text(draft, "timezone"),
      locale: text(draft, "locale"),
    };

    setBusy(true);
    setSaveError(null);
    try {
      if (school) {
        await updateSchool(school.id, input);
      } else {
        await createSchool(input);
      }
      reload();
      props.onChanged();
      setEditing(false);
    } catch (e: unknown) {
      setSaveError(describeSetupError(e));
    } finally {
      setBusy(false);
    }
  }, [draft, school, reload, props]);

  if (loadError) return <Banner tone="error">{loadError}</Banner>;
  if (school === undefined) return <Loading what="the school record" />;

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>School details</h2>
        <p>
          The project describes one school. Everything else in setup — campuses, rooms,
          years, the timetable cycle — belongs to it.
        </p>
      </header>

      {saveError && <Banner tone="error">{saveError}</Banner>}

      {editing ? (
        <EntityForm
          idPrefix="form-school"
          legend={school ? "Edit school details" : "Describe the school"}
          fields={fields}
          draft={draft}
          problems={problems}
          busy={busy}
          submitLabel={school ? "Save changes" : "Create school"}
          onChange={(key, value) => {
            setDraft((d) => ({ ...d, [key]: value }));
            setProblems(({ [key]: _removed, ...rest }) => rest);
          }}
          onSubmit={() => void submit()}
          onCancel={() => setEditing(false)}
        />
      ) : school === null ? (
        <EmptyState
          title="Start here"
          action={{ label: "Describe the school", onClick: () => open(null) }}
        >
          Nothing has been set up in this project yet. Give the school a name, a time zone
          and a locale, and the rest of the setup sections will open up.
        </EmptyState>
      ) : (
        <>
          <Facts
            items={[
              ["Name", school.name],
              // The label, so the summary reads as names; the identifier is inside it,
              // because the identifier is what is actually stored.
              ["Time zone", timezoneLabel(school.timezone)],
              ["Locale", localeLabel(school.locale)],
              ["Revision", String(school.rev)],
              ["Last changed", school.updatedAt],
            ]}
          />
          <div className="actions">
            <button type="button" onClick={() => open(school)}>
              Edit details
            </button>
          </div>
        </>
      )}
    </div>
  );
}
