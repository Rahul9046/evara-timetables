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
import type { Draft, Problems } from "../validation";
import { Combobox } from "./Combobox";

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
    };

/** Renders one field of a form. */
export function Field(props: {
  spec: FieldSpec;
  draft: Draft;
  problems: Problems;
  disabled: boolean;
  onChange: (key: string, value: string | boolean) => void;
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
              // `text` for times and dates on purpose: the native pickers disagree
              // across platforms about format and locale, and the model wants a literal
              // `HH:MM` or `YYYY-MM-DD`. The placeholder and the validator say which.
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

function placeholderFor(spec: FieldSpec): string | undefined {
  switch (spec.kind) {
    case "time":
      return "09:30";
    case "date":
      return "2027-09-01";
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
  onChange: (key: string, value: string | boolean) => void;
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
