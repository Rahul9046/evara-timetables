/**
 * A searchable multi-select.
 *
 * The sibling of {@link Combobox}, and deliberately the same interaction: a text input
 * that filters, a listbox of matches, full keyboard control. The differences are that an
 * option toggles rather than replaces, the list stays open while you pick, and what is
 * already chosen is shown as removable chips.
 *
 * ## Typing is still only searching
 *
 * Same rule as the single-select, for the same reason: typing filters and nothing else.
 * The selection changes only on an explicit toggle — Enter on the active option, a click,
 * a chip's remove button, or Backspace on an empty query, which is the one shortcut worth
 * having because it is what every tag input in the world does.
 *
 * ## Why the chips live inside the control
 *
 * A separate list of "currently selected" below the box reads as output rather than as
 * state you can edit, and people stop looking for the remove affordance there. Keeping
 * them in the control, before the input, makes the whole thing one object: this is the
 * value, and these are its parts.
 */
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";

import { matchesQuery } from "./Combobox";
import type { Choice } from "./Field";

/** Scrolls an option into view, where the environment can. */
function reveal(element: HTMLElement | null): void {
  // jsdom has no layout and does not implement scrollIntoView.
  if (element && typeof element.scrollIntoView === "function") {
    element.scrollIntoView({ block: "nearest" });
  }
}

export function MultiCombobox(props: {
  id: string;
  /** The chosen values, in the order they should read. */
  values: readonly string[];
  options: readonly Choice[];
  disabled?: boolean;
  placeholder?: string;
  /** Text shown when nothing matches the query. */
  emptyLabel?: string;
  /** Text shown in place of chips when nothing is chosen. */
  noneLabel?: string;
  /** Called with the complete new selection. Never on typing. */
  onChange: (values: string[]) => void;
  "aria-invalid"?: true | undefined;
  "aria-errormessage"?: string | undefined;
  "aria-describedby"?: string | undefined;
  className?: string | undefined;
}) {
  const { id, values, options, disabled = false, placeholder, onChange } = props;
  const emptyLabel = props.emptyLabel ?? "No matches";

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);

  const listId = `${id}-listbox`;
  const statusId = `${id}-status`;
  const optionPrefix = useId();
  const activeRef = useRef<HTMLLIElement | null>(null);

  const chosen = useMemo(() => new Set(values), [values]);

  const labelOf = useMemo(() => {
    const map = new Map(options.map((option) => [option.value, option.label]));
    return (value: string) => map.get(value) ?? value;
  }, [options]);

  const matches = useMemo(
    () => options.filter((option) => matchesQuery(option, query)),
    [options, query],
  );

  // Keep the cursor inside the list as the query narrows it, so Enter can never toggle an
  // option that is no longer on screen.
  useEffect(() => {
    setActive((current) => (current < matches.length ? current : 0));
  }, [matches.length]);

  useEffect(() => {
    if (open) reveal(activeRef.current);
  }, [open, active]);

  const toggle = useCallback(
    (value: string) => {
      onChange(
        chosen.has(value) ? values.filter((kept) => kept !== value) : [...values, value],
      );
      // The list stays open: picking several campuses is the normal case, and closing
      // after each one would mean reopening to add the next.
      setQuery("");
    },
    [chosen, onChange, values],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      const last = matches.length - 1;

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          if (!open) {
            setOpen(true);
            setActive(0);
          } else {
            setActive((current) => (current >= last ? 0 : current + 1));
          }
          return;

        case "ArrowUp":
          event.preventDefault();
          if (!open) {
            setOpen(true);
            setActive(Math.max(last, 0));
          } else {
            setActive((current) => (current <= 0 ? Math.max(last, 0) : current - 1));
          }
          return;

        case "Home":
          if (!open) return;
          event.preventDefault();
          setActive(0);
          return;

        case "End":
          if (!open) return;
          event.preventDefault();
          setActive(Math.max(last, 0));
          return;

        case "Enter": {
          // Only swallowed when it actually toggles something, so a closed box leaves
          // Enter as the form's submit key like every other field.
          if (!open) return;
          const choice = matches[active];
          event.preventDefault();
          if (choice) toggle(choice.value);
          return;
        }

        case "Backspace": {
          // Only on an empty query, so it never eats a character someone is mid-typing.
          if (query.length > 0 || values.length === 0) return;
          event.preventDefault();
          onChange(values.slice(0, -1));
          return;
        }

        case "Escape":
          if (!open) return;
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          setQuery("");
          return;

        default:
          return;
      }
    },
    [active, matches, onChange, open, query, toggle, values],
  );

  const activeId = open && matches[active] ? `${optionPrefix}-${active}` : undefined;

  return (
    <div className="combobox">
      <div className={props.className ? `chipField ${props.className}` : "chipField"}>
        {values.length === 0 && props.noneLabel !== undefined && (
          <span className="chipFieldNone">{props.noneLabel}</span>
        )}

        {values.map((value) => (
          <span key={value} className="chipToken">
            {labelOf(value)}
            <button
              type="button"
              className="chipRemove"
              disabled={disabled}
              // Named rather than decorative: a row of identical "×" buttons is
              // unusable with a screen reader.
              aria-label={`Remove ${labelOf(value)}`}
              onClick={() => toggle(value)}
            >
              <span aria-hidden="true">×</span>
            </button>
          </span>
        ))}

        <input
          id={id}
          type="text"
          role="combobox"
          className="chipFieldInput"
          autoComplete="off"
          spellCheck={false}
          disabled={disabled}
          value={query}
          placeholder={values.length === 0 ? placeholder : undefined}
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={activeId}
          aria-invalid={props["aria-invalid"]}
          aria-errormessage={props["aria-errormessage"]}
          aria-describedby={[props["aria-describedby"], statusId].filter(Boolean).join(" ")}
          onChange={(event) => {
            setQuery(event.currentTarget.value);
            setOpen(true);
            setActive(0);
          }}
          onKeyDown={onKeyDown}
          onClick={() => setOpen(true)}
          onBlur={() => {
            setOpen(false);
            setQuery("");
          }}
        />
      </div>

      <span className="srOnly" id={statusId} role="status">
        {open ? `${matches.length} of ${options.length} options, ${values.length} chosen` : ""}
      </span>

      <ul
        className="comboboxList"
        id={listId}
        role="listbox"
        aria-label="Options"
        aria-multiselectable="true"
        hidden={!open}
        // Keeps focus in the input so the blur handler does not cancel the click.
        onMouseDown={(event) => event.preventDefault()}
      >
        {matches.length === 0 ? (
          <li className="comboboxEmpty" role="presentation">
            {emptyLabel}
          </li>
        ) : (
          matches.map((option, index) => {
            const isActive = index === active;
            const isChosen = chosen.has(option.value);
            return (
              <li
                key={option.value}
                id={`${optionPrefix}-${index}`}
                role="option"
                aria-selected={isChosen}
                ref={isActive ? activeRef : undefined}
                className={
                  isActive ? "comboboxOption comboboxOption--active" : "comboboxOption"
                }
                onMouseEnter={() => setActive(index)}
                onClick={() => toggle(option.value)}
              >
                {option.label}
              </li>
            );
          })
        )}
      </ul>
    </div>
  );
}
