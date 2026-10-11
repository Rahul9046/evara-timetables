/**
 * Form inputs, declared rather than written out.
 *
 * Every School Setup form is the same thing with different fields, so the fields are a
 * data structure and this module renders them. That is what keeps eight entity editors
 * consistent: one place decides how a label, a hint, a required marker and an error
 * message are associated with an input, so they cannot drift apart screen by screen.
 *
 * ## Accessibility is structural here, not decorative
 *
 * Each input gets a real `<label for>`, `aria-describedby` for its hint, and
 * `aria-invalid` plus `aria-errormessage` when it is wrong. Doing it in the renderer
 * rather than in each form is the only way it stays true of all of them.
 */
import { WEEKDAYS } from "../setup/weekdays";
import type { Draft, Problems } from "../validation";
import { Combobox } from "./Combobox";
import { DateInput } from "./DateInput";
import { MultiCombobox } from "./MultiCombobox";

/** One option of a select. */
export interface Choice {
  readonly value: string;
  readonly label: string;
}

/** How one field is presented and validated. */
export type FieldSpec =
  | { kind: "text"; key: string; label: string; hint?: string; required?: boolean; placeholder?: string }
  | { kind: "textarea"; key: string; label: string; hint?: string; placeholder?: string }
  | { kind: "number"; key: string; label: string; hint?: string; required?: boolean; min?: number; max?: number }
  | { kind: "time"; key: string; label: string; hint?: string; required?: boolean }
  | { kind: "date"; key: string; label: string; hint?: string; required?: boolean }
  | { kind: "checkbox"; key: string; label: string; hint?: string }
  | {
      kind: "select";
      key: string;
      label: string;
      hint?: string;
      required?: boolean;
      /** Label for the null option. Omit to make the field non-nullable. */
      emptyLabel?: string;
      choices: readonly Choice[];
    }
  /**
   * A `select` with a search box, for when the list is too long to scroll.
   *
   * Same contract as `select` — the stored value is `Choice.value`, never the label —
   * but typing filters instead of jumping to the first matching initial. Use it past
   * roughly thirty options; below that a native `select` is faster and more familiar.
   */
  | {
      kind: "combobox";
      key: string;
      label: string;
      hint?: string;
      required?: boolean;
      placeholder?: string;
      /** Shown when the query matches nothing. */
      emptyLabel?: string;
      choices: readonly Choice[];
    }
  /**
   * The seven weekdays, any combination selectable. The draft holds a `string[]` of ISO
   * weekday numbers as strings — `["1", "2", "3", "4", "5"]` for Monday to Friday.
   *
   * Its own kind rather than a `multicombobox` with seven choices: a fixed set of seven
   * short labels wants to be visible and one click away, not behind a search box. The
   * vocabulary is `setup/weekdays`, and nothing about it reaches scheduling — see that
   * module's header.
   */
  | { kind: "weekdays"; key: string; label: string; hint?: string; required?: boolean }
  /**
   * A searchable multi-select. The draft holds a `string[]` of chosen values.
   *
   * Use it where the model has a real many-to-many — the campuses a building serves, say
   * — never as a shortcut for several booleans.
   */
  | {
      kind: "multicombobox";
      key: string;
      label: string;
      hint?: string;
      placeholder?: string;
      /** Shown when the query matches nothing. */
      emptyLabel?: string;
      /** Shown in place of chips when nothing is chosen. */
      noneLabel?: string;
      choices: readonly Choice[];
    };

/** Renders one field of a form. */
export function Field(props: {
  spec: FieldSpec;
  draft: Draft;
  problems: Problems;
  disabled: boolean;
  onChange: (key: string, value: string | boolean | string[]) => void;
  /** Prefix for element ids, so two forms on one screen do not collide. */
  idPrefix: string;
}) {
  const { spec, draft, problems, disabled, onChange, idPrefix } = props;
  const id = `${idPrefix}-${spec.key}`;
  const hintId = spec.hint ? `${id}-hint` : undefined;
  const error = problems[spec.key];
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  const value = draft[spec.key];
  const asText = typeof value === "string" ? value : "";

  const shared = {
    id,
    disabled,
    "aria-invalid": error ? true : undefined,
    "aria-errormessage": errorId,
    "aria-describedby": describedBy,
    className: error ? "invalid" : undefined,
  } as const;

  // A weekday picker is seven inputs, so there is nothing for a `<label for>` to point
  // at. It gets a real `<fieldset>` with a `<legend>` instead, which is what associates a
  // group's name with its controls — see WeekdayPicker.
  if (spec.kind === "weekdays") {
    return (
      <div className="field field--wide">
        <fieldset className="weekdayGroup">
          <legend>
            {spec.label}
            {spec.required ? (
              <span className="requiredMark" aria-hidden="true">
                {" *"}
              </span>
            ) : null}
          </legend>
          <WeekdayPicker
            id={id}
            values={Array.isArray(value) ? value : []}
            disabled={disabled}
            invalid={error !== undefined}
            describedBy={describedBy}
            errorId={errorId}
            onChange={(values) => onChange(spec.key, values)}
          />
          {spec.hint && (
            <p className="hint" id={hintId}>
              {spec.hint}
            </p>
          )}
          {error && (
            <p className="fieldError" id={errorId} role="alert">
              {error}
            </p>
          )}
        </fieldset>
      </div>
    );
  }

  return (
    <div className={spec.kind === "checkbox" ? "field field--inline" : "field"}>
      {spec.kind === "checkbox" ? (
        <label htmlFor={id} className="checkboxRow">
          <input
            {...shared}
            type="checkbox"
            checked={value === true}
            onChange={(e) => onChange(spec.key, e.currentTarget.checked)}
          />
          <span>{spec.label}</span>
        </label>
      ) : (
        <>
          <label htmlFor={id}>
            {spec.label}
            {"required" in spec && spec.required ? (
              <span className="requiredMark" aria-hidden="true">
                {" *"}
              </span>
            ) : null}
          </label>

          {spec.kind === "textarea" ? (
            <textarea
              {...shared}
              rows={3}
              value={asText}
              placeholder={spec.placeholder}
              onChange={(e) => onChange(spec.key, e.currentTarget.value)}
            />
          ) : spec.kind === "multicombobox" ? (
            <MultiCombobox
              id={id}
              values={Array.isArray(value) ? value : []}
              options={spec.choices}
              disabled={disabled}
              placeholder={spec.placeholder}
              emptyLabel={spec.emptyLabel}
              noneLabel={spec.noneLabel}
              aria-invalid={error ? true : undefined}
              aria-errormessage={errorId}
              aria-describedby={describedBy}
              className={error ? "invalid" : undefined}
              onChange={(values) => onChange(spec.key, values)}
            />
          ) : spec.kind === "date" ? (
            <DateInput
              id={id}
              value={asText}
              label={spec.label}
              disabled={disabled}
              aria-invalid={error ? true : undefined}
              aria-errormessage={errorId}
              aria-describedby={describedBy}
              className={error ? "invalid" : undefined}
              // The draft keeps the storage form, `YYYY-MM-DD`. See DateInput for what
              // reaches it while a date is half typed, and why.
              onChange={(value) => onChange(spec.key, value)}
            />
          ) : spec.kind === "combobox" ? (
            <Combobox
              id={id}
              value={asText}
              options={spec.choices}
              disabled={disabled}
              placeholder={spec.placeholder}
              emptyLabel={spec.emptyLabel}
              aria-invalid={error ? true : undefined}
              aria-errormessage={errorId}
              aria-describedby={describedBy}
              className={error ? "invalid" : undefined}
              // Only an explicit selection reaches the draft. See Combobox for why.
              onSelect={(value) => onChange(spec.key, value)}
            />
          ) : spec.kind === "select" ? (
            <select
              {...shared}
              value={asText}
              onChange={(e) => onChange(spec.key, e.currentTarget.value)}
            >
              {spec.emptyLabel !== undefined && <option value="">{spec.emptyLabel}</option>}
              {spec.choices.map((choice) => (
                <option key={choice.value} value={choice.value}>
                  {choice.label}
                </option>
              ))}
            </select>
          ) : (
            <input
              {...shared}
              // `text` for times on purpose: the native picker disagrees across
              // platforms about format and locale, and the model wants a literal
              // `HH:MM`. The placeholder and the validator say so.
              type="text"
              inputMode={spec.kind === "number" ? "numeric" : undefined}
              value={asText}
              placeholder={placeholderFor(spec)}
              onChange={(e) => onChange(spec.key, e.currentTarget.value)}
            />
          )}
        </>
      )}

      {spec.hint && (
        <p className="hint" id={hintId}>
          {spec.hint}
        </p>
      )}
      {error && (
        <p className="fieldError" id={errorId} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * Seven weekday toggles.
 *
 * Real `<input type="checkbox">`es, styled as chips. That is the whole accessibility
 * story and it is why they are not buttons with `aria-pressed`: a checkbox is reachable by
 * Tab, toggles on Space, announces its own checked state, and is understood by every
 * assistive technology without a role being asserted. Each has a visible `<label>`
 * carrying the full day name, with the two-letter form shown and the name available to a
 * screen reader, so "Mo" is never what gets read out.
 *
 * Order is Monday first, matching ISO numbering, so what is displayed and what is stored
 * cannot disagree. Any combination is selectable; "at least one" is enforced by
 * `validateSchool` for the message and by the column's own `CHECK` for the invariant, not
 * by disabling the last remaining checkbox — a user clearing the row to start again is
 * doing something reasonable, and a control that refuses to turn off is confusing.
 */
function WeekdayPicker(props: {
  id: string;
  values: readonly string[];
  disabled: boolean;
  invalid: boolean;
  describedBy: string | undefined;
  errorId: string | undefined;
  onChange: (values: string[]) => void;
}) {
  const { id, values, disabled, invalid, describedBy, errorId, onChange } = props;
  const chosen = new Set(values);

  const toggle = (value: string, on: boolean) => {
    const next = new Set(chosen);
    if (on) {
      next.add(value);
    } else {
      next.delete(value);
    }
    // Sorted so the draft is in a stable order whatever order the boxes were clicked in,
    // which keeps "did this change?" comparisons honest.
    onChange([...next].sort((a, b) => Number(a) - Number(b)));
  };

  return (
    <div className={invalid ? "weekdayRow invalid" : "weekdayRow"}>
      {WEEKDAYS.map((day) => {
        const value = String(day.value);
        const boxId = `${id}-${value}`;
        return (
          <label key={day.value} className="weekdayToggle" htmlFor={boxId}>
            <input
              id={boxId}
              type="checkbox"
              value={value}
              checked={chosen.has(value)}
              disabled={disabled}
              aria-invalid={invalid ? true : undefined}
              aria-errormessage={errorId}
              aria-describedby={describedBy}
              onChange={(e) => toggle(value, e.currentTarget.checked)}
            />
            <span aria-hidden="true">{day.short}</span>
            <span className="srOnly">{day.label}</span>
          </label>
        );
      })}
    </div>
  );
}

/**
 * The placeholder for the plain inputs this renderer still draws itself.
 *
 * `date` is absent on purpose: a date field is a {@link DateInput}, which owns its own
 * placeholder because the placeholder *is* the format contract — it says `DD/MM/YYYY`
 * rather than an example date, so there is no day, month or year to read the order off
 * backwards.
 */
function placeholderFor(spec: FieldSpec): string | undefined {
  switch (spec.kind) {
    case "time":
      return "09:30";
    case "text":
      return spec.placeholder;
    default:
      return undefined;
  }
}

/**
 * A complete form: the fields, a submit and a cancel.
 *
 * Submitting with Enter works because this is a real `<form>` with a real submit button,
 * which is also why `onSubmit` must not be wired to a click handler on a `<div>`.
 */
export function EntityForm(props: {
  idPrefix: string;
  legend: string;
  fields: readonly FieldSpec[];
  draft: Draft;
  problems: Problems;
  busy: boolean;
  submitLabel: string;
  onChange: (key: string, value: string | boolean | string[]) => void;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  const { idPrefix, legend, fields, draft, problems, busy, submitLabel } = props;

  return (
    <form
      className="entityForm"
      onSubmit={(e) => {
        e.preventDefault();
        props.onSubmit();
      }}
      noValidate
    >
      <fieldset disabled={busy}>
        <legend>{legend}</legend>
        <div className="fieldGrid">
          {fields.map((spec) => (
            <Field
              key={spec.key}
              spec={spec}
              draft={draft}
              problems={problems}
              disabled={busy}
              onChange={props.onChange}
              idPrefix={idPrefix}
            />
          ))}
        </div>
        <div className="actions">
          <button type="submit">{busy ? "Saving…" : submitLabel}</button>
          <button type="button" className="secondary" onClick={props.onCancel}>
            Cancel
          </button>
        </div>
      </fieldset>
    </form>
  );
}
