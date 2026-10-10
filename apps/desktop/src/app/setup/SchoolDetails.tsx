/**
 * Section 1 — School Details.
 *
 * The one record every other section hangs off. Not a `CrudPanel`: there is exactly one
 * school per project, it is created rather than added, and it is never deleted — the way
 * to discard a school is to discard the project file, which is why the repository offers
 * no `delete_school` and this screen offers no delete button.
 */
import { useCallback, useState } from "react";

import type { School } from "@evara/domain";
import { createSchool, describeSetupError, getSchool, updateSchool } from "../../ipc/setup";
import { EntityForm, type FieldSpec } from "../ui/Field";
import { Banner, EmptyState, Facts, Loading } from "../ui/Shell";
import { isValid, text, validateSchool, type Draft, type Problems } from "../validation";
import { useValue } from "./useList";

const FIELDS: readonly FieldSpec[] = [
  {
    kind: "text",
    key: "name",
    label: "School name",
    required: true,
    placeholder: "Northgate Secondary",
  },
  {
    kind: "text",
    key: "timezone",
    label: "Time zone",
    required: true,
    placeholder: "Europe/London",
    hint: "An IANA time zone name. Period times are local wall clock, so this is what they are local to.",
  },
  {
    kind: "text",
    key: "locale",
    label: "Locale",
    required: true,
    placeholder: "en-GB",
    hint: "A BCP-47 tag, used for formatting dates and numbers in reports.",
  },
];

function draftOf(school: School | null): Draft {
  return {
    name: school?.name ?? "",
    // Sensible starting points rather than blanks, since both are almost always the
    // machine's own settings and retyping them is pure friction.
    timezone: school?.timezone ?? guessTimezone(),
    locale: school?.locale ?? guessLocale(),
  };
}

/**
 * The machine's time zone, as a starting suggestion only.
 *
 * Reading it is not a network call and not telemetry — it is `Intl`, which is part of the
 * JavaScript runtime. The user can change it, and nothing is stored until they save.
 */
function guessTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
  } catch {
    return "";
  }
}

function guessLocale(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().locale || "";
  } catch {
    return "";
  }
}

export function SchoolDetails(props: { onChanged: () => void }) {
  const load = useCallback(() => getSchool(), []);
  const { value: school, error: loadError, reload } = useValue<School | null>(load);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
          fields={FIELDS}
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
              ["Time zone", school.timezone],
              ["Locale", school.locale],
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
