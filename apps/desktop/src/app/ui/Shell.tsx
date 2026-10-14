/**
 * The small, shared pieces every setup screen is built from.
 *
 * Banners, empty states, tables and the confirmation dialogue. Together with
 * `Field.tsx` these are the whole visual vocabulary of School Setup, which is what makes
 * eight sections look like one application rather than eight.
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
 * An extra per-row button, beside Edit and Delete.
 *
 * Exists for the row actions that are neither: making a school the active one, opening one
 * student's enrolment, jumping to the school that holds a visiting teacher's identity. All
 * of those are navigation or scoping rather than editing, so a form field would be the
 * wrong place for them — and writing a second table to get them would make Stage 0C's
 * screens look unlike the eleven that came before.
 *
 * `blockedBy` disables the button and becomes its `title`. It must say *why*: a disabled
 * button is not focusable, so an explanation only in a tooltip is unreachable by keyboard
 * and by touch. Same rule as `CrudSpec.addBlockedBy`.
 */
export interface RowAction<T> {
  readonly label: string;
  readonly onClick: (row: T) => void;
  /** Why it is unavailable for this row, or `null` when it is available. */
  readonly blockedBy?: (row: T) => string | null;
  /** Renders as a danger-toned link, for an action that takes something away. */
  readonly tone?: "danger";
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
  /** Extra buttons, rendered before Edit. See {@link RowAction}. */
  rowActions?: readonly RowAction<T>[];
  /** Why this row cannot be edited from here, when it cannot. */
  editBlockedBy?: (row: T) => string | null;
}) {
  const {
    caption,
    columns,
    rows,
    busy,
    describe,
    onEdit,
    onDelete,
    activeId,
    rowActions,
    editBlockedBy,
  } = props;

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
                {(rowActions ?? []).map((action) => {
                  const blocked = action.blockedBy?.(row) ?? null;
                  return (
                    <button
                      key={action.label}
                      type="button"
                      className={action.tone === "danger" ? "linkish danger" : "linkish"}
                      aria-label={`${action.label} ${describe(row)}`}
                      disabled={busy || blocked !== null}
                      title={blocked ?? undefined}
                      onClick={() => action.onClick(row)}
                    >
                      {action.label}
                    </button>
                  );
                })}
                {/* `aria-label` rather than a visually-hidden span: a table of twenty
                    rows otherwise has twenty buttons all announced as "Edit", and the
                    label gives each one an unambiguous name without duplicating the
                    row's text into the DOM. */}
                <button
                  type="button"
                  className="linkish"
                  aria-label={`Edit ${describe(row)}`}
                  disabled={busy || (editBlockedBy?.(row) ?? null) !== null}
                  title={editBlockedBy?.(row) ?? undefined}
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
  /**
   * Explains the action instead of offering it.
   *
   * The confirm button goes and Cancel becomes "Close". For an action the backend is
   * certain to refuse — deleting a resource another school is still using, deleting a
   * school that still owns a campus — where the honest interface is an explanation rather
   * than a button that fails. Deliberately a dialogue rather than a disabled button: a
   * disabled button cannot be focused, so its reason is unreachable by keyboard, and the
   * reason is the whole point.
   */
  blocked?: boolean;
  /**
   * Why the action cannot be confirmed *yet*.
   *
   * Disables the confirm button and becomes its `title`. Different from
   * {@link blocked}, which is "this will never work": this is "not until you have chosen
   * something", for a dialogue that asks a question as well as confirming — picking which
   * existing person to add. A button that silently does nothing is worse than one that is
   * visibly unavailable.
   */
  confirmBlockedBy?: string | null;
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
        {props.blocked !== true && (
          <button
            type="button"
            className={props.tone === "danger" ? "danger" : undefined}
            disabled={props.busy || (props.confirmBlockedBy ?? null) !== null}
            title={props.confirmBlockedBy ?? undefined}
            onClick={props.onConfirm}
          >
            {props.busy ? "Working…" : props.confirmLabel}
          </button>
        )}
        <button type="button" className="secondary" disabled={props.busy} onClick={props.onCancel}>
          {props.blocked === true ? "Close" : "Cancel"}
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
