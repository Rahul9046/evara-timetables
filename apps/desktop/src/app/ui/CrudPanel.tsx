/**
 * One editable collection: a table, a form, and a confirmed delete.
 *
 * Seven of the eight School Setup sections are the same interaction with different fields,
 * so the interaction is written once here and each section supplies a {@link CrudSpec}.
 * The point is not brevity — it is that "what happens when a save fails", "what a delete
 * asks before it happens" and "what an empty collection says" are decided in one place
 * and are therefore the same everywhere.
 *
 * ## State always comes back from the backend
 *
 * After every mutation the list is re-read rather than patched locally. A create returns
 * the stored record and an update returns the new revision, so patching would usually
 * work — and would quietly stop working the moment a write has a side effect on another
 * row, which several already do: changing the default cycle clears the previous default,
 * and deleting a room type unclassifies rooms. Re-reading means the screen shows what is
 * in the project rather than what this component believed it did.
 *
 * ## Failures are shown, never swallowed
 *
 * A rejected save leaves the form open with the typed values intact, so nothing the user
 * entered is lost. A `duplicate` failure marks the offending field; everything else shows
 * as a banner above the form.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { describeSetupError, offendingField } from "../../ipc/setup";
import { isValid, type Draft, type Problems } from "../validation";
import { EntityForm, type FieldSpec } from "./Field";
import {
  Banner,
  ConfirmDialog,
  DataTable,
  EmptyState,
  Loading,
  type Column,
  type RowAction,
} from "./Shell";

/** Everything one editable collection needs to describe itself. */
export interface CrudSpec<T extends { id: string }, I> {
  /** Singular noun, lowercase: "campus". Used in prompts and messages. */
  readonly noun: string;
  /** Plural noun, lowercase: "campuses". */
  readonly plural: string;
  /** Title shown above the table. */
  readonly title: string;
  readonly columns: readonly Column<T>[];
  /**
   * The form's fields, or a function of the draft when they depend on it.
   *
   * The function form exists for one real case: a choice early in the form that decides
   * which later fields are *meaningful at all*. A cycle's type is the example — a weekly
   * cycle has no length to set, because a week is seven days, so offering the field would
   * invite a number the backend is going to ignore.
   *
   * It is deliberately not a general-purpose "show this if that": a field whose value is
   * merely irrelevant should stay visible and be explained by its hint. Reach for this
   * only when the field would otherwise ask for something the model does not accept.
   */
  readonly fields: readonly FieldSpec[] | ((draft: Draft) => readonly FieldSpec[]);
  /** A human-readable name for one row, for confirmations and screen readers. */
  readonly describe: (row: T) => string;
  readonly validate: (draft: Draft) => Problems;
  /** Builds a form draft. `null` means a new record. */
  readonly toDraft: (row: T | null) => Draft;
  /** Converts a validated draft into the input the backend wants. */
  readonly toInput: (draft: Draft) => I;
  readonly list: () => Promise<T[]>;
  readonly create: (input: I) => Promise<T>;
  readonly update: (id: string, input: I) => Promise<T>;
  readonly remove: (id: string) => Promise<void>;
  /** What the empty state says. Must name the next action. */
  readonly emptyHint: ReactNode;
  /** Extra warning in the delete confirmation, when this row deserves one. */
  readonly deleteNote?: (row: T) => ReactNode;
  /**
   * Why this particular edit needs confirming before it is sent, if it does.
   *
   * Returns the warning to show, or `null` to save straight away. Exists because some
   * edits are as consequential as a delete — withdrawing a building from a campus is the
   * first — and "what a risky change asks before it happens" belongs here with "what a
   * delete asks", not reimplemented per screen.
   *
   * Called with the validated draft and the row being edited, `null` for a create.
   */
  readonly confirmSubmit?: (draft: Draft, row: T | null) => ReactNode | null;
  /**
   * Why this collection cannot be edited yet, if it cannot.
   *
   * Shown instead of the table. Used for the prerequisites the model really has — rooms
   * need a campus, terms need a year — so the user is told what to do first rather than
   * meeting a foreign-key failure.
   */
  readonly blockedBy?: string | null;
  /**
   * Why nothing more can be *added* right now, if nothing can.
   *
   * Deliberately not {@link blockedBy}: that hides the table, which is wrong when the
   * reason adding is unavailable is precisely that the collection is already complete.
   * The rows stay, the Add button is disabled, and this says why.
   *
   * Shown as visible text beside the button as well as on its `title`. A disabled button
   * is not focusable, so a tooltip alone is unreachable by keyboard and by touch — which
   * would make the explanation invisible to exactly the users most likely to need it.
   *
   * The caller decides this from an authoritative answer, never from a rule reimplemented
   * in the view layer. `CyclesAndCycleDays` reads `CycleCoverage.missingOrdinals`, which
   * `evara_core::time` computed.
   */
  readonly addBlockedBy?: string | null;
  /**
   * Why this row cannot be deleted, when the answer is already known.
   *
   * Returns the explanation, or `null` to offer the delete. The confirmation then
   * *explains* rather than asking: no destructive button, and Cancel reads "Close".
   *
   * For the refusals Stage 0C made ordinary — a school that still owns a campus, a
   * resource another school is still using, a person another school still holds — where
   * the answer is read from the backend before the action is offered rather than
   * discovered by pressing Delete. The backend still refuses; this only means the user is
   * not asked to agree to something that was never going to happen.
   *
   * Deliberately a dialogue rather than a disabled Delete button. A disabled button
   * cannot be focused, so its `title` is unreachable by keyboard and by touch, and the
   * reason is the whole value of the interaction.
   */
  readonly deleteBlockedBy?: (row: T) => ReactNode | null;
  /**
   * Why this row cannot be edited in this panel, when it cannot.
   *
   * Disables Edit and becomes its `title`. Unlike {@link deleteBlockedBy} this one *is* a
   * disabled control, because the explanation always has a visible home beside it — the
   * row's own "Belongs to" column says whose record it is — and because an edit form that
   * opened only to refuse every save would be worse.
   */
  readonly editBlockedBy?: (row: T) => string | null;
  /** Extra per-row buttons, before Edit. See {@link RowAction}. */
  readonly rowActions?: readonly RowAction<T>[];
}

/** Renders an editable collection. */
export function CrudPanel<T extends { id: string }, I>(props: {
  spec: CrudSpec<T, I>;
  /** Bumping this re-reads the list — used when a parent selection changes. */
  reloadKey?: string;
  /** Called after any successful mutation, so a parent can refresh dependent data. */
  onChanged?: () => void;
}) {
  const { spec, reloadKey, onChanged } = props;

  const [rows, setRows] = useState<readonly T[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<{ row: T | null } | null>(null);
  const [draft, setDraft] = useState<Draft>({});
  const [problems, setProblems] = useState<Problems>({});
  const [pendingDelete, setPendingDelete] = useState<T | null>(null);
  const [pendingSave, setPendingSave] = useState<ReactNode | null>(null);

  const blocked = spec.blockedBy ?? null;
  const [nonce, setNonce] = useState(0);

  // The spec is rebuilt on every render by most callers — it closes over the selected
  // parent row and whatever lists feed its selects. Depending on its *identity* in the
  // load effect would therefore re-fetch on every render, forever. Holding it in a ref
  // means the effect can call the current `list` without that identity being a trigger,
  // and the triggers become the two things that genuinely change which rows to show.
  const latest = useRef(spec);
  latest.current = spec;

  /** Re-reads the list. Stable, so it is safe to pass to a child or an effect. */
  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (blocked !== null) {
      setRows([]);
      return undefined;
    }

    let live = true;
    setRows(null);
    latest.current
      .list()
      .then((rows) => {
        if (!live) return;
        setRows(rows);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setRows([]);
        setError(describeSetupError(e));
      });

    // A parent selection can change faster than a round trip completes; without this the
    // slower response lands last and shows the previous parent's rows.
    return () => {
      live = false;
    };
    // `reloadKey` is the parent's way of saying "different parent row, different list";
    // `nonce` is this component's own way of saying "read it again".
  }, [blocked, reloadKey, nonce]);

  useEffect(() => {
    // A different parent row means the open form is about the wrong parent.
    setEditing(null);
  }, [reloadKey]);

  const openForm = useCallback(
    (row: T | null) => {
      setEditing({ row });
      setDraft(spec.toDraft(row));
      setProblems({});
      setError(null);
    },
    [spec],
  );

  const closeForm = useCallback(() => {
    setEditing(null);
    setProblems({});
    setPendingSave(null);
  }, []);

  const change = useCallback((key: string, value: string | boolean | string[]) => {
    setDraft((current) => ({ ...current, [key]: value }));
    // Clearing the field's own message as it is retyped keeps a stale "required"
    // sitting under a field the user has just filled in.
    setProblems((current) => {
      if (current[key] === undefined) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  }, []);

  /** Performs the mutation. Reached only once anything that needed confirming has been. */
  const save = useCallback(async () => {
    if (!editing) return;

    setPendingSave(null);
    setBusy(true);
    setError(null);
    try {
      const input = spec.toInput(draft);
      if (editing.row) {
        await spec.update(editing.row.id, input);
      } else {
        await spec.create(input);
      }
      reload();
      onChanged?.();
      closeForm();
    } catch (e: unknown) {
      // The backend is authoritative. If it named a field, mark that field; otherwise
      // show the message. Either way the form stays open with the values intact.
      const field = offendingField(e);
      if (field) {
        const key = fieldKeyFor(fieldsOf(spec, draft), field);
        setProblems({ [key]: describeSetupError(e) });
      } else {
        setError(describeSetupError(e));
      }
    } finally {
      setBusy(false);
    }
  }, [editing, draft, spec, reload, onChanged, closeForm]);

  /**
   * Validates, then either saves or asks first.
   *
   * Validation runs before the confirmation so a user is never asked to confirm a save
   * that was going to be rejected anyway.
   */
  const submit = useCallback(() => {
    if (!editing) return;

    const found = spec.validate(draft);
    setProblems(found);
    if (!isValid(found)) return;

    const warning = spec.confirmSubmit?.(draft, editing.row) ?? null;
    if (warning !== null) {
      setPendingSave(warning);
      return;
    }
    void save();
  }, [draft, editing, save, spec]);

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return;
    setBusy(true);
    setError(null);
    try {
      await spec.remove(pendingDelete.id);
      setPendingDelete(null);
      reload();
      onChanged?.();
    } catch (e: unknown) {
      // A RESTRICT refusal is the common case and its message explains what is in the
      // way, so it belongs on the page rather than inside a dialogue that is closing.
      setPendingDelete(null);
      setError(describeSetupError(e));
    } finally {
      setBusy(false);
    }
  }, [pendingDelete, spec, reload, onChanged]);

  const idPrefix = useMemo(() => `form-${spec.noun.replace(/\s+/g, "-")}`, [spec.noun]);

  if (blocked !== null) {
    return (
      <div className="crudPanel">
        <PanelHead title={spec.title} />
        <EmptyState title={`No ${spec.plural} yet`}>{blocked}</EmptyState>
      </div>
    );
  }

  const addBlocked = spec.addBlockedBy ?? null;
  // Resolved once, so the dialogue's title, its body and whether it offers a destructive
  // button cannot disagree about the same row.
  const deleteBlocked =
    pendingDelete === null ? null : (spec.deleteBlockedBy?.(pendingDelete) ?? null);

  return (
    <div className="crudPanel">
      <PanelHead
        title={spec.title}
        action={
          editing
            ? undefined
            : {
                label: `Add ${spec.noun}`,
                onClick: () => openForm(null),
                disabled: busy || rows === null || addBlocked !== null,
                blockedBy: addBlocked,
              }
        }
      />

      {error && <Banner tone="error">{error}</Banner>}

      {editing && (
        <EntityForm
          idPrefix={idPrefix}
          legend={editing.row ? `Edit ${spec.describe(editing.row)}` : `New ${spec.noun}`}
          fields={fieldsOf(spec, draft)}
          draft={draft}
          problems={problems}
          busy={busy}
          submitLabel={editing.row ? "Save changes" : `Add ${spec.noun}`}
          onChange={change}
          onSubmit={submit}
          onCancel={closeForm}
        />
      )}

      {rows === null ? (
        <Loading what={spec.plural} />
      ) : rows.length === 0 ? (
        editing ? null : (
          <EmptyState
            title={`No ${spec.plural} yet`}
            action={{
              label: `Add ${spec.noun}`,
              onClick: () => openForm(null),
              // Unreachable for a cycle, whose `day_count` is at least 1 and so always
              // leaves a free position when there are no rows. Guarded anyway, so a
              // future caller cannot end up with an empty state offering a button the
              // panel head would have refused.
              disabled: addBlocked !== null,
            }}
          >
            {spec.emptyHint}
          </EmptyState>
        )
      ) : (
        <DataTable
          caption={spec.title}
          columns={spec.columns}
          rows={rows}
          busy={busy}
          describe={spec.describe}
          activeId={editing?.row?.id ?? null}
          onEdit={openForm}
          onDelete={setPendingDelete}
          rowActions={spec.rowActions}
          editBlockedBy={spec.editBlockedBy}
        />
      )}

      <ConfirmDialog
        open={pendingSave !== null}
        title={`Save these changes to this ${spec.noun}?`}
        confirmLabel="Save changes"
        tone="danger"
        busy={busy}
        onConfirm={() => void save()}
        onCancel={() => setPendingSave(null)}
      >
        {pendingSave}
      </ConfirmDialog>

      <ConfirmDialog
        open={pendingDelete !== null}
        title={
          deleteBlocked !== null
            ? `This ${spec.noun} cannot be deleted`
            : `Delete this ${spec.noun}?`
        }
        confirmLabel={`Delete ${spec.noun}`}
        tone="danger"
        busy={busy}
        blocked={deleteBlocked !== null}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setPendingDelete(null)}
      >
        {pendingDelete &&
          (deleteBlocked !== null ? (
            <>
              <p>
                <strong>{spec.describe(pendingDelete)}</strong> is still in use.
              </p>
              {deleteBlocked}
            </>
          ) : (
            <>
              <p>
                <strong>{spec.describe(pendingDelete)}</strong> will be removed from this
                campus file. This cannot be undone.
              </p>
              {spec.deleteNote?.(pendingDelete)}
            </>
          ))}
      </ConfirmDialog>
    </div>
  );
}

function PanelHead(props: {
  title: string;
  action?: {
    label: string;
    onClick: () => void;
    disabled?: boolean;
    /** Why the action is unavailable. Shown as text and as the button's `title`. */
    blockedBy?: string | null;
  };
}) {
  const blockedBy = props.action?.blockedBy ?? null;

  return (
    <div className="panelHead">
      <h3>{props.title}</h3>
      {blockedBy !== null && (
        // `role="note"` rather than `status`: this is a standing explanation of why the
        // button is disabled, not an announcement of something that just happened, and a
        // live region would interrupt a screen reader every time the list is re-read.
        <p className="panelNote" role="note">
          {blockedBy}
        </p>
      )}
      {props.action && (
        <button
          type="button"
          onClick={props.action.onClick}
          disabled={props.action.disabled}
          title={blockedBy ?? undefined}
        >
          {props.action.label}
        </button>
      )}
    </div>
  );
}

/**
 * The fields this draft should be rendered with.
 *
 * One place resolves {@link CrudSpec.fields}, so the form, the validator's error
 * attachment and anything added later cannot disagree about which fields are on screen —
 * which is how a backend message ends up pinned to a field the user cannot see.
 */
function fieldsOf<T extends { id: string }, I>(
  spec: CrudSpec<T, I>,
  draft: Draft,
): readonly FieldSpec[] {
  return typeof spec.fields === "function" ? spec.fields(draft) : spec.fields;
}

/**
 * Maps the backend's field vocabulary onto a form's own field keys.
 *
 * The backend says `code`, `name`, `label`, `position` or `date` — the column that
 * collided. A form's key is usually the same word, but `position` is `ordinal` in the
 * model, and a form that has no such field should show the message somewhere rather than
 * attach it to nothing.
 */
function fieldKeyFor(fields: readonly FieldSpec[], backendField: string): string {
  const candidates = backendField === "position" ? ["ordinal", "position"] : [backendField];
  for (const candidate of candidates) {
    if (fields.some((field) => field.key === candidate)) return candidate;
  }
  return fields[0]?.key ?? backendField;
}
