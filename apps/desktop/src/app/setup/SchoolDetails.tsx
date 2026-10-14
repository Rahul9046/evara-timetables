/**
 * Section 2 — School Details.
 *
 * The record every section after it hangs off: the **selected** school's name, code, time
 * zone, locale, working days and logo. Which school that is belongs to the setup shell,
 * which owns the project's list of them; this screen never looks one up, because a
 * `LIMIT 1` read would quietly edit an arbitrary institution.
 *
 * ## Why this is not the Schools section
 *
 * Stage 0C added **Schools**, which is about the project: how many institutions it
 * describes, which one is selected, and creating or deleting one. This screen is about one
 * institution, and it is where the logo lives — a thing that is uploaded and removed
 * immediately rather than being part of a draft that is saved or cancelled. Two screens,
 * two decisions; the *form* is one definition, in `schoolForm.ts`, so they cannot drift.
 *
 * The first-run path stays here. A campus file with no schools at all opens on Schools,
 * which is where the first one is added, and this screen says so rather than offering a
 * second way to do it.
 */
import { useCallback, useMemo, useState } from "react";

import type { School } from "@evara/domain";
import { describeSetupError, offendingField, updateSchool } from "../../ipc/setup";
import { EntityForm } from "../ui/Field";
import { Banner, EmptyState, Facts, Loading } from "../ui/Shell";
import { isValid, validateSchool, type Draft, type Problems } from "../validation";
import { localeLabel } from "./locales";
import { schoolDraft, schoolFields, schoolInput } from "./schoolForm";
import { SchoolLogoPanel } from "./SchoolLogoPanel";
import { timezoneLabel } from "./timezones";
import { describeWeek } from "./weekdays";

export function SchoolDetails(props: {
  school: School | null;
  onChanged: () => void;
  /** Sends the user to the Schools section, where a school is created. */
  onOpenSchools?: () => void;
}) {
  const { school, onChanged, onOpenSchools } = props;

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Four hundred labels, each a formatter call. Built once per school rather than on
  // every keystroke in the form above them.
  const fields = useMemo(() => schoolFields(school ? [school] : []), [school]);

  const open = useCallback((current: School | null) => {
    setDraft(schoolDraft(current));
    setProblems({});
    setSaveError(null);
    setEditing(true);
  }, []);

  const submit = useCallback(async () => {
    if (!school) return;

    const found = validateSchool(draft);
    setProblems(found);
    if (!isValid(found)) return;

    setBusy(true);
    setSaveError(null);
    try {
      await updateSchool(school.id, schoolInput(draft));
      onChanged();
      setEditing(false);
    } catch (e: unknown) {
      // The form stays open with the values intact: a duplicate name or code is the likely
      // refusal and retyping everything to fix one box is not a reasonable ask. Where the
      // backend named the offending field, the message goes on that field rather than in a
      // banner above a form with five boxes in it — the same division `CrudPanel` makes.
      const field = offendingField(e);
      if (field === "name" || field === "code") {
        setProblems({ [field]: describeSetupError(e) });
      } else {
        setSaveError(describeSetupError(e));
      }
    } finally {
      setBusy(false);
    }
  }, [draft, school, onChanged]);

  if (school === undefined) return <Loading what="the school record" />;

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>School details</h2>
        <p>
          One institution: its name, the time zone its bells ring in, the locale its reports
          are formatted for, and the weekdays it normally teaches on. That school&rsquo;s
          own campuses, rooms, years and timetable cycle belong to it and are set up in the
          sections below.
        </p>
      </header>

      {saveError && <Banner tone="error">{saveError}</Banner>}

      {school === null ? (
        <EmptyState
          title="No school selected"
          action={
            onOpenSchools
              ? { label: "Go to Schools", onClick: onOpenSchools }
              : undefined
          }
        >
          This campus file does not describe a school yet. Add one under{" "}
          <strong>Schools</strong> — give it a name, a time zone, a locale and its working
          days, and the rest of the setup sections open up.
        </EmptyState>
      ) : editing ? (
        <EntityForm
          idPrefix="form-school"
          legend={`Edit ${school.name}`}
          fields={fields}
          draft={draft}
          problems={problems}
          busy={busy}
          submitLabel="Save changes"
          onChange={(key, value) => {
            setDraft((d) => ({ ...d, [key]: value }));
            setProblems(({ [key]: _removed, ...rest }) => rest);
          }}
          onSubmit={() => void submit()}
          onCancel={() => setEditing(false)}
        />
      ) : (
        <>
          <Facts
            items={[
              ["Name", school.name],
              // The code is editable here now rather than carried silently through a save
              // the form never showed. A school without one says so.
              ["Code", school.code ?? "Not set"],
              // The label, so the summary reads as names; the identifier is inside it,
              // because the identifier is what is actually stored.
              ["Time zone", timezoneLabel(school.timezone)],
              ["Locale", localeLabel(school.locale)],
              ["Working days", describeWeek(school.workingDays)],
              ["Revision", String(school.rev)],
              ["Last changed", school.updatedAt],
            ]}
          />
          <div className="actions">
            <button type="button" onClick={() => open(school)}>
              Edit details
            </button>
          </div>

          {/* Outside the details form on purpose: a logo is uploaded and removed
              immediately rather than being part of a draft that is saved or cancelled,
              and it is optional in a way the other fields are not. */}
          <SchoolLogoPanel school={school} />
        </>
      )}
    </div>
  );
}
