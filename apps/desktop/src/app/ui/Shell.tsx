/**
 * The small, shared pieces every setup screen is built from.
 *
 * Banners, empty states, tables and the confirmation dialogue. Together with
 * `Field.tsx` these are the whole visual vocabulary of School Setup, which is what makes
 * nine sections look like one application rather than nine.
 */
import { useEffect, useRef, type ReactNode } from "react";

/** An error, a warning or a note, with the right role for a screen reader. */
export function Banner(props: {
  tone: "error" | "warning" | "info" | "ok";
  children: ReactNode;
}) {
  // An error interrupts; a warning or a note does not. `alert` on an advisory message
  // makes a screen reader shout about something the user did not do wrong.
  const role = props.tone === "error" ? "alert" : "status";
  return (
    <p className={`banner banner--${props.tone}`} role={role}>
      {props.children}
    </p>
  );
}

/**
 * What a section shows before it has anything to show.
 *
 * Always names the next action. "No campuses" tells the user what is true; "Add the first
 * campus — a school needs at least one before rooms can be recorded" tells them what to
 * do, which is the only useful thing an empty screen can say.
 */
export function EmptyState(props: {
  title: string;
  children: ReactNode;
  action?: { label: string; onClick: () => void; disabled?: boolean };
}) {
  return (
    <div className="emptyState">
      <h3>{props.title}</h3>
      <p>{props.children}</p>
      {props.action && (
        <button type="button" onClick={props.action.onClick} disabled={props.action.disabled}>
          {props.action.label}
        </button>
      )}
    </div>
  );
}

/** A section is loading. */
export function Loading(props: { what: string }) {
  return (
    <p className="loading" role="status" aria-live="polite">
      Loading {props.what}…
    </p>
  );
}

/** One column of a {@link DataTable}. */
export interface Column<T> {
  readonly header: string;
  readonly render: (row: T) => ReactNode;
  /** Right-align and tabular-figure a numeric column. */
  readonly numeric?: boolean;
}

/**
 * A table with per-row edit and delete.
 *
 * Wrapped in its own scroll container so a wide table scrolls instead of pushing the
 * window sideways.
 */
export function DataTable<T extends { id: string }>(props: {
  caption: string;
  columns: readonly Column<T>[];
  rows: readonly T[];
  busy: boolean;
  describe: (row: T) => string;
  onEdit: (row: T) => void;
  onDelete: (row: T) => void;
  /** Marks the row currently being edited. */
  activeId?: string | null;
}) {
  const { caption, columns, rows, busy, describe, onEdit, onDelete, activeId } = props;

  return (
    <div className="tableScroll">
      <table className="dataTable">
        <caption className="srOnly">{caption}</caption>
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.header} scope="col" className={column.numeric ? "num" : undefined}>
                {column.header}
              </th>
            ))}
            <th scope="col" className="rowActions">
              <span className="srOnly">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id} className={row.id === activeId ? "editing" : undefined}>
              {columns.map((column) => (
                <td key={column.header} className={column.numeric ? "num" : undefined}>
                  {column.render(row)}
                </td>
              ))}
              <td className="rowActions">
                {/* `aria-label` rather than a visually-hidden span: a table of twenty
                    rows otherwise has twenty buttons all announced as "Edit", and the
                    label gives each one an unambiguous name without duplicating the
                    row's text into the DOM. */}
                <button
                  type="button"
                  className="linkish"
                  aria-label={`Edit ${describe(row)}`}
                  disabled={busy}
                  onClick={() => onEdit(row)}
                >
                  Edit
                </button>
                <button
                  type="button"
                  className="linkish danger"
                  aria-label={`Delete ${describe(row)}`}
                  disabled={busy}
                  onClick={() => onDelete(row)}
                >
                  Delete
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * Confirmation before something irreversible.
 *
 * A real `<dialog>` rather than `window.confirm`, for two reasons. A native dialog blocks
 * the WebView and, per the project's own notes, can wedge the automation bridge; and
 * `confirm` cannot show a list — which the orphan release has to, because "delete 12
 * timeslots" is not enough information to agree to.
 *
 * Focus moves to the dialogue on open and Escape cancels, both of which `showModal`
 * gives us.
 */
export function ConfirmDialog(props: {
  open: boolean;
  title: string;
  confirmLabel: string;
  tone?: "danger" | "normal";
  busy?: boolean;
  children: ReactNode;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;

    if (props.open && !dialog.open) {
      // `showModal` is what gives focus capture, Escape-to-cancel and the backdrop. It
      // is not universally implemented — jsdom, which the tests run in, has `<dialog>`
      // but not `showModal` — so fall back to the `open` attribute. Without the
      // fallback the element stays out of the accessibility tree entirely, which means
      // a screen reader on such a runtime would not announce the confirmation at all.
      // Degrading to a visible, non-modal dialogue is much better than degrading to an
      // invisible one.
      if (typeof dialog.showModal === "function") {
        dialog.showModal();
      } else {
        dialog.open = true;
      }
    }

    if (!props.open && dialog.open) {
      if (typeof dialog.close === "function") {
        dialog.close();
      } else {
        dialog.open = false;
      }
    }
  }, [props.open]);

  if (!props.open) return null;

  return (
    <dialog
      ref={ref}
      className="confirm"
      aria-labelledby="confirmTitle"
      onCancel={(e) => {
        e.preventDefault();
        props.onCancel();
      }}
    >
      <h3 id="confirmTitle">{props.title}</h3>
      <div className="confirmBody">{props.children}</div>
      <div className="actions">
        <button
          type="button"
          className={props.tone === "danger" ? "danger" : undefined}
          disabled={props.busy}
          onClick={props.onConfirm}
        >
          {props.busy ? "Working…" : props.confirmLabel}
        </button>
        <button type="button" className="secondary" disabled={props.busy} onClick={props.onCancel}>
          Cancel
        </button>
      </div>
    </dialog>
  );
}

/** A labelled group within a section. */
export function Panel(props: { title: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section className="subPanel">
      <h3>{props.title}</h3>
      {props.note && <p className="muted">{props.note}</p>}
      {props.children}
    </section>
  );
}

/** A read-only key/value list, for details that are not editable in place. */
export function Facts(props: { items: ReadonlyArray<[string, ReactNode]> }) {
  return (
    <dl className="facts">
      {props.items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
