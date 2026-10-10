/**
 * Choosing which parent row a child collection belongs to.
 *
 * Five of the nine sections edit a parent-child pair — campus and buildings, campus and
 * rooms, year and terms, cycle and days, bell schedule and periods — because that is how
 * the model is shaped: `buildings(campus_id)`, `terms(academic_year_id)` and the rest are
 * scoped reads, not whole-table ones.
 *
 * Presenting that as a picker above the child table is what keeps the brief's "avoid one
 * enormous screen" true without hiding the relationship: the heading always says which
 * parent the rows below belong to.
 */
import { useEffect, type ReactNode } from "react";

import { Banner, EmptyState, Loading } from "../ui/Shell";

/** A row that can be picked. */
export interface Pickable {
  readonly id: string;
  readonly label: string;
  /** Shown after the label, e.g. a code or a date range. */
  readonly detail?: string;
  /** Marks the school's default, where the model has one. */
  readonly isDefault?: boolean;
}

/**
 * Renders the picker and keeps a valid selection.
 *
 * Selecting the first row automatically is deliberate: a section whose child table is
 * blank until the user clicks something looks broken, and there is no ambiguity about
 * which row to start on when there is only one.
 */
export function ParentPicker(props: {
  legend: string;
  items: readonly Pickable[] | null;
  error: string | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  /** Shown when the parent collection is empty — must name the next action. */
  empty: { title: string; hint: ReactNode };
  children: ReactNode;
}) {
  const { legend, items, error, selectedId, onSelect, empty, children } = props;

  useEffect(() => {
    if (items === null) return;
    const stillThere = items.some((item) => item.id === selectedId);
    if (stillThere) return;
    // Prefer the model's own default when it has one; it is the row the user means.
    const fallback = items.find((item) => item.isDefault) ?? items[0];
    onSelect(fallback?.id ?? null);
  }, [items, selectedId, onSelect]);

  if (error) return <Banner tone="error">{error}</Banner>;
  if (items === null) return <Loading what={legend.toLowerCase()} />;
  if (items.length === 0) {
    return <EmptyState title={empty.title}>{empty.hint}</EmptyState>;
  }

  return (
    <>
      <div className="picker" role="group" aria-label={legend}>
        <span className="pickerLabel">{legend}</span>
        <div className="pickerOptions">
          {items.map((item) => (
            <button
              key={item.id}
              type="button"
              className={item.id === selectedId ? "chip chip--on" : "chip"}
              aria-pressed={item.id === selectedId}
              onClick={() => onSelect(item.id)}
            >
              {item.label}
              {item.detail && <span className="chipDetail">{item.detail}</span>}
              {item.isDefault && <span className="chipBadge">default</span>}
            </button>
          ))}
        </div>
      </div>
      {selectedId !== null && children}
    </>
  );
}
