/**
 * A date field: a text box you can type into, and a calendar you can pick from.
 *
 * Used for every `date` field in School Setup, which is what keeps the four academic
 * year and term dates and the calendar mapping date behaving identically.
 *
 * ## Typing and picking are the same value, in two spellings
 *
 * The draft always holds the *storage* form, `YYYY-MM-DD`, exactly as the IPC contract
 * and the `starts_on` column want it. The input shows the *display* form, `DD/MM/YYYY`.
 * The conversion is `../dates`, and it happens here at the boundary and nowhere else.
 *
 * Two consequences worth being explicit about:
 *
 * - picking the 1st of September 2027 from the calendar and typing `01/09/2027` produce
 *   the identical draft value, so there is no "saved from the calendar" versus "saved by
 *   hand" difference to go wrong;
 * - opening an existing record cannot alter its dates. The ISO value arrives, is
 *   displayed as `DD/MM/YYYY`, and unless the user edits a field it is handed back
 *   unchanged — the component never normalises, re-derives or "corrects" a value it was
 *   given.
 *
 * ## A half-typed date is not a date, and is not thrown away either
 *
 * While the user is typing, the text goes through a parse. If it names a real day the
 * draft gets the canonical ISO form. If it does not — `01/09/202` on the way to a year,
 * or `31/02/2026`, which will never be a day — the draft gets **the raw text**.
 *
 * That is the important choice. The alternatives are both worse: holding the last valid
 * value would silently save a date the user has visibly replaced, and clearing the field
 * would discard what they typed. Passing the raw text through means `validateAcademicYear`
 * sees it, `isoDate` rejects it, and the user is told which field is wrong and why —
 * rather than an invalid date being rounded into a valid one behind their back.
 *
 * The display text is local state, so nothing reformats under the caret mid-edit. Blur is
 * when a parsable value is tidied to `DD/MM/YYYY`, and `1/9/2027` becoming `01/09/2027`
 * on the way out is the only rewrite that ever happens.
 *
 * ## Why the calendar is in a portal
 *
 * The setup body scrolls and the confirmation dialogue is a real `<dialog>`, so a popup
 * positioned inside the field's own box can be clipped by one and painted under the
 * other. It is rendered into `document.body` at a measured fixed position instead, and
 * flips above the field when there is no room below.
 */
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import {
  DATE_FORMAT,
  type CalendarDate,
  addDays,
  addMonths,
  daysInMonth,
  describeDate,
  fromIso,
  inMonth,
  monthChoices,
  monthGrid,
  monthName,
  parseTypedDate,
  sameDate,
  toDisplay,
  toIso,
  todayDate,
  WEEKDAYS,
} from "../dates";

/** How far either side of the shown year the year select reaches. */
const YEAR_REACH = 10;

/** Where the popup is painted. `null` until it has been measured. */
interface Position {
  readonly top: number;
  readonly left: number;
}

export function DateInput(props: {
  id: string;
  /** Canonical `YYYY-MM-DD`, the empty string, or the raw text of an edit in progress. */
  value: string;
  /** The field's own label, so the calendar and its trigger can name themselves. */
  label: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  "aria-invalid"?: true | undefined;
  "aria-errormessage"?: string | undefined;
  "aria-describedby"?: string | undefined;
  className?: string | undefined;
}) {
  const { id, value, label, disabled = false, onChange } = props;

  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<Position | null>(null);

  /**
   * The text in the box, and the value it was derived from.
   *
   * `echo` is how the component tells its own edits apart from a new value arriving from
   * outside — a different row being opened for editing, or the form being reset. Its own
   * edits are already on screen; an outside change has to be displayed.
   */
  const [text, setText] = useState(() => toDisplay(value));
  const [echo, setEcho] = useState(value);
  if (value !== echo) {
    setEcho(value);
    setText(toDisplay(value));
  }

  const selected = useMemo(() => fromIso(value), [value]);
  const today = useMemo(() => todayDate(), []);

  /** The day the calendar's keyboard cursor is on. Also decides which month is drawn. */
  const [cursor, setCursor] = useState<CalendarDate>(() => selected ?? today);

  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const cursorRef = useRef<HTMLButtonElement>(null);

  /**
   * Whether the day under the cursor should take focus when it next renders.
   *
   * Set by opening the calendar and by arrowing around the grid, and deliberately *not*
   * by the month and year selects: moving focus out of a select the moment it changes
   * would make it impossible to step through months with the keyboard.
   */
  const wantsFocus = useRef(false);

  const popupId = `${id}-calendar`;
  const gridLabelId = `${id}-calendar-month`;
  const cellPrefix = useId();

  /** Sends a value upwards, remembering that we were the ones who changed it. */
  const emit = useCallback(
    (next: string) => {
      setEcho(next);
      onChange(next);
    },
    [onChange],
  );

  /** Handles a keystroke in the text box. See the note on half-typed dates above. */
  const type = useCallback(
    (typed: string) => {
      setText(typed);
      const trimmed = typed.trim();
      if (trimmed.length === 0) {
        emit("");
        return;
      }
      emit(parseTypedDate(trimmed) ?? trimmed);
    },
    [emit],
  );

  /** Tidies a parsable value to `DD/MM/YYYY` once the user has finished with the field. */
  const tidy = useCallback(() => {
    const iso = parseTypedDate(text);
    if (iso !== null) setText(toDisplay(iso));
  }, [text]);

  const close = useCallback(
    (options: { restoreFocus: boolean }) => {
      setOpen(false);
      setPosition(null);
      if (options.restoreFocus) inputRef.current?.focus();
    },
    [],
  );

  const pick = useCallback(
    (date: CalendarDate) => {
      const iso = toIso(date);
      setText(toDisplay(iso));
      emit(iso);
      setCursor(date);
      close({ restoreFocus: true });
    },
    [close, emit],
  );

  const toggle = useCallback(() => {
    if (open) {
      close({ restoreFocus: true });
      return;
    }
    // Opens on the date in the field, or on this month when the field is empty or holds
    // text that is not yet a date.
    setCursor(fromIso(value) ?? today);
    wantsFocus.current = true;
    setOpen(true);
  }, [close, open, today, value]);

  // -------------------------------------------------------------------- positioning

  /**
   * Measures the field and parks the popup against it.
   *
   * `position: fixed` in a body-level portal, so the numbers are viewport coordinates and
   * no ancestor's `overflow` or stacking context applies. Flips above the field when the
   * calendar would not fit below it, and never leaves the left or right edge.
   */
  const place = useCallback(() => {
    const anchor = rootRef.current?.getBoundingClientRect();
    if (!anchor) return;

    const popup = popupRef.current;
    const width = popup?.offsetWidth ?? 0;
    const height = popup?.offsetHeight ?? 0;
    const gap = 4;

    const below = anchor.bottom + gap;
    const above = anchor.top - gap - height;
    const fitsBelow = below + height <= window.innerHeight;
    const top = fitsBelow || above < 0 ? below : above;

    const room = Math.max(window.innerWidth - width - 8, 8);
    const left = Math.max(Math.min(anchor.left, room), 8);

    setPosition({ top, left });
  }, []);

  // Measured after paint so the popup's real size is known, which is what makes the flip
  // and the clamp correct rather than approximate.
  useLayoutEffect(() => {
    if (!open) return undefined;
    place();

    // Capture, so a scroll in any ancestor is seen — the setup body scrolls, not the
    // window. Passive: this only reads layout.
    const update = () => place();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, { capture: true, passive: true });
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, { capture: true });
    };
  }, [open, place]);

  // Focus moves into the calendar on open, which is what makes it operable by keyboard
  // at all, and is why `close` puts focus back on the input. It then follows the cursor
  // around the grid, because with a roving tabindex the focused button *is* the cursor.
  useEffect(() => {
    if (!open || !wantsFocus.current) return;
    wantsFocus.current = false;
    cursorRef.current?.focus();
  }, [open, cursor]);

  useEffect(() => {
    if (!open) return undefined;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target)) return;
      if (popupRef.current?.contains(target)) return;
      // Clicking elsewhere abandons the calendar without taking focus off whatever was
      // clicked, so `restoreFocus` is wrong here.
      close({ restoreFocus: false });
    };
    document.addEventListener("mousedown", onPointerDown);
    return () => document.removeEventListener("mousedown", onPointerDown);
  }, [close, open]);

  // The field is inside a form, and `disabled` arrives while a save is in flight. A
  // calendar left open over a disabled field would still accept a click.
  useEffect(() => {
    if (disabled && open) close({ restoreFocus: false });
  }, [close, disabled, open]);

  // ---------------------------------------------------------------------- keyboard

  const onGridKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const move = (next: CalendarDate) => {
        event.preventDefault();
        wantsFocus.current = true;
        setCursor(next);
      };

      switch (event.key) {
        case "ArrowLeft":
          return move(addDays(cursor, -1));
        case "ArrowRight":
          return move(addDays(cursor, 1));
        case "ArrowUp":
          return move(addDays(cursor, -7));
        case "ArrowDown":
          return move(addDays(cursor, 7));
        case "Home":
          // Monday of the shown week, matching the grid's own first column.
          return move(addDays(cursor, -dayIndex(cursor)));
        case "End":
          return move(addDays(cursor, 6 - dayIndex(cursor)));
        case "PageUp":
          return move(addMonths(cursor, event.shiftKey ? -12 : -1));
        case "PageDown":
          return move(addMonths(cursor, event.shiftKey ? 12 : 1));
        case "Escape":
          // Stopped so Escape closes the calendar rather than the dialogue around it.
          event.preventDefault();
          event.stopPropagation();
          close({ restoreFocus: true });
          return;
        default:
          return;
      }
    },
    [close, cursor],
  );

  const onInputKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      // The documented way into a combobox-style popup, and the only one that does not
      // need a mouse. Enter is left alone: closed, it is still the form's submit key.
      if (event.key === "ArrowDown" && event.altKey) {
        event.preventDefault();
        if (!open) toggle();
      }
    },
    [open, toggle],
  );

  // ------------------------------------------------------------------------ render

  const grid = useMemo(() => monthGrid(cursor.year, cursor.month), [cursor]);
  const weeks = useMemo(
    () => Array.from({ length: 6 }, (_, week) => grid.slice(week * 7, week * 7 + 7)),
    [grid],
  );

  const years = useMemo(() => {
    const from = cursor.year - YEAR_REACH;
    return Array.from({ length: YEAR_REACH * 2 + 1 }, (_, index) => from + index);
  }, [cursor.year]);

  const calendar = (
    <div
      className="calendarPopup"
      id={popupId}
      ref={popupRef}
      role="dialog"
      aria-label={`${label} calendar`}
      style={
        position
          ? { top: `${position.top}px`, left: `${position.left}px` }
          : // Kept out of the way until measured, rather than flashing at 0,0.
            { top: "0", left: "0", visibility: "hidden" }
      }
    >
      <div className="calendarHead">
        <button
          type="button"
          className="calendarStep"
          aria-label="Previous year"
          onClick={() => setCursor(addMonths(cursor, -12))}
        >
          «
        </button>
        <button
          type="button"
          className="calendarStep"
          aria-label="Previous month"
          onClick={() => setCursor(addMonths(cursor, -1))}
        >
          ‹
        </button>

        <label className="srOnly" htmlFor={`${popupId}-month`}>
          Month
        </label>
        <select
          id={`${popupId}-month`}
          className="calendarSelect"
          value={String(cursor.month)}
          onChange={(event) =>
            setCursor(at(cursor.year, Number(event.currentTarget.value), cursor.day))
          }
        >
          {monthChoices().map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
        </select>

        <label className="srOnly" htmlFor={`${popupId}-year`}>
          Year
        </label>
        <select
          id={`${popupId}-year`}
          className="calendarSelect"
          value={String(cursor.year)}
          onChange={(event) =>
            setCursor(at(Number(event.currentTarget.value), cursor.month, cursor.day))
          }
        >
          {years.map((year) => (
            <option key={year} value={String(year)}>
              {year}
            </option>
          ))}
        </select>

        <button
          type="button"
          className="calendarStep"
          aria-label="Next month"
          onClick={() => setCursor(addMonths(cursor, 1))}
        >
          ›
        </button>
        <button
          type="button"
          className="calendarStep"
          aria-label="Next year"
          onClick={() => setCursor(addMonths(cursor, 12))}
        >
          »
        </button>
      </div>

      {/* Announces the month to a screen reader when the grid is paged, and names the
          grid itself, so arrowing into the next month is not silent. */}
      <p className="calendarMonth" id={gridLabelId} role="status">
        {monthName(cursor.month)} {cursor.year}
      </p>

      <div
        className="calendarGrid"
        role="grid"
        aria-labelledby={gridLabelId}
        onKeyDown={onGridKeyDown}
      >
        <div className="calendarWeekdays" role="row">
          {WEEKDAYS.map((weekday) => (
            <abbr
              key={weekday.long}
              role="columnheader"
              aria-label={weekday.long}
              title={weekday.long}
            >
              {weekday.short}
            </abbr>
          ))}
        </div>

        {weeks.map((week, index) => (
          <div className="calendarWeek" role="row" key={(week[0] && toIso(week[0])) ?? index}>
            {week.map((date) => {
              const isCursor = sameDate(date, cursor);
              const isSelected = sameDate(date, selected);
              const outside = !inMonth(date, cursor.year, cursor.month);
              const classes = ["calendarDay"];
              if (outside) classes.push("calendarDay--outside");
              if (isSelected) classes.push("calendarDay--on");
              if (sameDate(date, today)) classes.push("calendarDay--today");

              return (
                // `aria-selected` on the cell is where the grid pattern puts it; it is
                // repeated on the button because that is what takes focus, and so what
                // a screen reader actually reads out.
                <div
                  className="calendarCell"
                  role="gridcell"
                  aria-selected={isSelected}
                  key={toIso(date)}
                >
                  <button
                    type="button"
                    id={`${cellPrefix}-${toIso(date)}`}
                    ref={isCursor ? cursorRef : undefined}
                    className={classes.join(" ")}
                    // Roving tabindex: one stop for the whole grid, so Tab leaves the
                    // calendar instead of walking 42 buttons.
                    tabIndex={isCursor ? 0 : -1}
                    aria-selected={isSelected}
                    aria-current={sameDate(date, today) ? "date" : undefined}
                    aria-label={describeDate(date)}
                    onClick={() => pick(date)}
                  >
                    {date.day}
                  </button>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );

  return (
    <div className="dateInput" ref={rootRef}>
      <input
        id={id}
        ref={inputRef}
        type="text"
        // `text` rather than `date` deliberately. A native date input will not take a
        // `DD/MM/YYYY` placeholder, formats itself by the browser's locale rather than
        // the one this project has chosen, and on some platforms has no reliable typed
        // entry at all. See the module note.
        inputMode="numeric"
        autoComplete="off"
        spellCheck={false}
        className={props.className}
        disabled={disabled}
        value={text}
        placeholder={DATE_FORMAT}
        aria-invalid={props["aria-invalid"]}
        aria-errormessage={props["aria-errormessage"]}
        aria-describedby={props["aria-describedby"]}
        onChange={(event) => type(event.currentTarget.value)}
        onBlur={tidy}
        onKeyDown={onInputKeyDown}
      />

      <button
        type="button"
        className="dateInputTrigger"
        disabled={disabled}
        aria-label={`Choose ${label.toLowerCase()} from a calendar`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? popupId : undefined}
        onClick={toggle}
      >
        <CalendarIcon />
      </button>

      {open && createPortal(calendar, document.body)}
    </div>
  );
}

/**
 * A day in a named month, with the day clamped to one that exists.
 *
 * Jumping from the 31st of March to February with the month select lands on the 28th or
 * 29th rather than skidding into March, the same way `addMonths` behaves.
 */
function at(year: number, month: number, day: number): CalendarDate {
  return { year, month, day: Math.min(day, daysInMonth(year, month)) };
}

/** Where a date falls in a Monday-first week, 0–6. */
function dayIndex(date: CalendarDate): number {
  return (new Date(Date.UTC(date.year, date.month - 1, date.day)).getUTCDay() + 6) % 7;
}

/** The trigger's glyph. Inline so it needs no network and no asset. */
function CalendarIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      width="15"
      height="15"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="1.8" y="3.2" width="12.4" height="11" rx="1.6" />
      <path d="M1.8 6.6h12.4M5.2 1.8v2.4M10.8 1.8v2.4" />
    </svg>
  );
}
