/**
 * A searchable single-select.
 *
 * A native `<select>` stops being usable somewhere around thirty options, and the
 * timezone list is four hundred. This is the ARIA 1.2 combobox-with-listbox pattern: a
 * text input that filters, a listbox of matches, and full keyboard control.
 *
 * ## Typing is searching, not editing
 *
 * The one rule that shapes everything here: **the stored value changes only on an
 * explicit selection** — Enter on the active option, or a click. Typing filters the list
 * and nothing more; abandoning the box by pressing Escape, or by clicking away, puts the
 * display back to whatever is still selected and leaves the value untouched.
 *
 * That is deliberate rather than merely cautious. A school's timezone is already set in
 * an existing project, and a picker that treats a half-typed query as a new value would
 * let a stray keystroke silently repoint every period time in the school. So a
 * half-typed query is never a value, and there is no code path from `query` to
 * `onSelect`.
 *
 * ## Why `filtering` is separate from `query`
 *
 * When the list opens by click or arrow key, the input still reads the current selection
 * and the list shows *everything* — if it filtered by the selected label you would have
 * to clear the box before you could browse. The first keystroke flips `filtering` on, and
 * from then on the input reads the query and the list is filtered. Closing flips it back.
 * One flag, and both behaviours come out right.
 */
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";

import type { Choice } from "./Field";

/**
 * Folds a string into something worth comparing a query against.
 *
 * Underscores become spaces so `new york` finds `America/New_York`, and the slash goes
 * too so `asia kolkata` works. Lowercasing is locale-insensitive on purpose: these are
 * ASCII identifiers, and `toLocaleLowerCase` under a Turkish locale would turn `I` into
 * a dotless `ı` and stop `India` from matching itself.
 */
function fold(value: string): string {
  return value.toLowerCase().replace(/[_/]+/g, " ");
}

/**
 * Whether a choice matches a query.
 *
 * Every whitespace-separated term must appear somewhere in the label or the value, in
 * any order, so `kolkata india` and `india kolkata` both find the same row. Substring
 * rather than prefix matching is what makes `york` find `America/New_York` without the
 * user having to know the continent prefix.
 */
export function matchesQuery(choice: Choice, query: string): boolean {
  const terms = fold(query).split(" ").filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = `${fold(choice.label)} ${fold(choice.value)}`;
  return terms.every((term) => haystack.includes(term));
}

/**
 * Adds a stored value that is not in the list, marked as what it is.
 *
 * A project saved by an older build — or by hand, or on a machine whose timezone
 * database knew a name this one does not — can hold a value the options do not offer.
 * Dropping it would be data loss disguised as a default, and silently substituting the
 * nearest match would be worse, so it is prepended as a real option and labelled. The
 * user can then keep it or choose a replacement deliberately.
 */
export function withExistingValue(
  options: readonly Choice[],
  stored: string | null | undefined,
  /**
   * Builds the whole label for the unknown value.
   *
   * A callback rather than a fixed suffix because the caller knows more about the value
   * than this function does — a retired timezone name, say, is worth saying the current
   * spelling of, and only the timezone module knows that.
   */
  describe: (value: string) => string = (value) =>
    `${value} — existing value, not in the list`,
): readonly Choice[] {
  const value = (stored ?? "").trim();
  if (value.length === 0) return options;
  if (options.some((option) => option.value === value)) return options;
  return [{ value, label: describe(value) }, ...options];
}

/** Scrolls an option into view, where the environment can. */
function reveal(element: HTMLElement | null): void {
  // jsdom has no layout and does not implement scrollIntoView, so this is a no-op under
  // test rather than a crash.
  if (element && typeof element.scrollIntoView === "function") {
    element.scrollIntoView({ block: "nearest" });
  }
}

export function Combobox(props: {
  id: string;
  value: string;
  options: readonly Choice[];
  disabled?: boolean;
  placeholder?: string;
  /** Text shown when nothing matches the query. */
  emptyLabel?: string;
  /** Called only on an explicit selection. Never on typing. */
  onSelect: (value: string) => void;
  "aria-invalid"?: true | undefined;
  "aria-errormessage"?: string | undefined;
  "aria-describedby"?: string | undefined;
  className?: string | undefined;
}) {
  const { id, value, options, disabled = false, placeholder, onSelect } = props;
  const emptyLabel = props.emptyLabel ?? "No matches";

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [filtering, setFiltering] = useState(false);
  const [active, setActive] = useState(0);

  const listId = `${id}-listbox`;
  const statusId = `${id}-status`;
  const optionPrefix = useId();
  const activeRef = useRef<HTMLLIElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const selected = useMemo(
    () => options.find((option) => option.value === value) ?? null,
    [options, value],
  );
  const selectedLabel = selected?.label ?? "";

  /** What the input shows: the query while searching, the selection otherwise. */
  const display = filtering ? query : selectedLabel;

  const matches = useMemo(
    () => (filtering ? options.filter((option) => matchesQuery(option, query)) : options),
    [filtering, options, query],
  );

  // Keep the active row inside the list after the matches change, so Enter can never
  // commit an option that is no longer on screen.
  useEffect(() => {
    setActive((current) => (current < matches.length ? current : 0));
  }, [matches.length]);

  useEffect(() => {
    if (open) reveal(activeRef.current);
  }, [open, active]);

  /** Closes without touching the value. */
  const cancel = useCallback(() => {
    setOpen(false);
    setFiltering(false);
    setQuery("");
  }, []);

  const commit = useCallback(
    (choice: Choice) => {
      onSelect(choice.value);
      setOpen(false);
      setFiltering(false);
      setQuery("");
    },
    [onSelect],
  );

  const openAt = useCallback(
    (index: number) => {
      setOpen(true);
      setActive(index);
    },
    [],
  );

  /** Where the cursor should land when the list opens: on the current selection. */
  const indexOfSelected = useCallback(() => {
    const found = options.findIndex((option) => option.value === value);
    return found >= 0 ? found : 0;
  }, [options, value]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      const last = matches.length - 1;

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          if (!open) openAt(indexOfSelected());
          else setActive((current) => (current >= last ? 0 : current + 1));
          return;

        case "ArrowUp":
          event.preventDefault();
          if (!open) openAt(indexOfSelected());
          else setActive((current) => (current <= 0 ? Math.max(last, 0) : current - 1));
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
          // Only swallowed when it actually selects something. Closed, Enter stays the
          // form's submit key, which is how the rest of the setup forms behave.
          if (!open) return;
          const choice = matches[active];
          event.preventDefault();
          if (choice) commit(choice);
          return;
        }

        case "Escape":
          if (!open) return;
          // Stopped so Escape closes the list rather than the dialog around it.
          event.preventDefault();
          event.stopPropagation();
          cancel();
          return;

        default:
          return;
      }
    },
    [active, cancel, commit, indexOfSelected, matches, open, openAt],
  );

  const activeId = open && matches[active] ? `${optionPrefix}-${active}` : undefined;

  return (
    <div className="combobox" ref={rootRef}>
      <input
        id={id}
        type="text"
        role="combobox"
        className={props.className}
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        value={display}
        placeholder={placeholder}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={activeId}
        aria-invalid={props["aria-invalid"]}
        aria-errormessage={props["aria-errormessage"]}
        aria-describedby={[props["aria-describedby"], statusId].filter(Boolean).join(" ")}
        onChange={(event) => {
          // Typing searches. It does not select, and it does not call onSelect.
          setQuery(event.currentTarget.value);
          setFiltering(true);
          setOpen(true);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
        onClick={() => {
          if (!open) openAt(indexOfSelected());
        }}
        onBlur={() => {
          // Reached only when focus truly leaves: the listbox swallows its own mousedown
          // below, so clicking an option does not blur the input first.
          if (open) cancel();
        }}
      />

      {/* Announces the result count to a screen reader without stealing focus. */}
      <span className="srOnly" id={statusId} role="status">
        {open ? `${matches.length} of ${options.length} options` : ""}
      </span>

      <ul
        className="comboboxList"
        id={listId}
        role="listbox"
        aria-label="Options"
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
            const isSelected = option.value === value;
            return (
              <li
                key={option.value}
                id={`${optionPrefix}-${index}`}
                role="option"
                aria-selected={isSelected}
                ref={isActive ? activeRef : undefined}
                className={
                  isActive ? "comboboxOption comboboxOption--active" : "comboboxOption"
                }
                onMouseEnter={() => setActive(index)}
                onClick={() => commit(option)}
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
