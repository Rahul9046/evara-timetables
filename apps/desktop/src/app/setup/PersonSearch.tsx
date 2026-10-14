/**
 * Finding somebody who is already in the project.
 *
 * The one control Stage 0C exists to make unnecessary to live without: a teacher who
 * already teaches at Northgate must be *added* to Southmoor, never typed in again.
 * `V13` made that possible — one row per real person, membership and enrolment as
 * relationships — and only the interface can reintroduce the duplication the model
 * removed.
 *
 * ## Why a search rather than a select
 *
 * A project-wide staff list is as long as the trust is large, and a project-wide student
 * list is longer. More importantly, the thing a user is trying to do here is *tell two
 * similarly named people apart*, which a list of bare names cannot help with. So each
 * candidate shows the identifying detail the model actually holds — a code, an email, the
 * school that entered them, and which schools already hold them — and the query matches
 * all of it.
 *
 * ## Names are never merged
 *
 * Nothing here consolidates two records because they share a name. Two people called
 * J. Smith are two people, and a screen that guessed otherwise would silently merge two
 * teachers' timetables. The user selects exactly one existing record, or creates a new
 * one; there is no third option and no automatic match.
 */
import { useId, useMemo, useState, type ReactNode } from "react";

import { Banner, EmptyState, Loading } from "../ui/Shell";

/** One person the search can offer. */
export interface Candidate {
  readonly id: string;
  /** "Okafor, Ada" — family name first, which is how a staff list is read. */
  readonly name: string;
  /** Short identifying details, shown under the name and matched by the query. */
  readonly details: readonly string[];
  /** Why this person cannot be chosen, when they cannot. */
  readonly blockedBy?: string | null;
}

/** Whether a candidate matches a query. Every detail is searchable, not only the name. */
function matches(candidate: Candidate, query: string): boolean {
  if (query.length === 0) return true;
  const haystack = [candidate.name, ...candidate.details].join(" ").toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word.length > 0)
    .every((word) => haystack.includes(word));
}

/**
 * A searchable list of existing people, with one selected.
 *
 * Real `<input type="radio">`es in a `<fieldset>` with a `<legend>`, not a list of buttons
 * with `aria-selected`. A radio group is reachable by Tab, moves with the arrow keys,
 * announces "3 of 40" without a role being asserted, and is understood by every assistive
 * technology — the same argument `WeekdayPicker` makes for using real checkboxes.
 */
export function PersonSearch(props: {
  legend: string;
  /** The people to offer, or `null` while they are being read. */
  candidates: readonly Candidate[] | null;
  error: string | null;
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** What to say when the project holds nobody to offer. */
  emptyTitle: string;
  emptyHint: ReactNode;
  disabled?: boolean;
}) {
  const { legend, candidates, error, selectedId, onSelect, emptyTitle, emptyHint } = props;
  const [query, setQuery] = useState("");
  const base = useId();

  const shown = useMemo(
    () => (candidates ?? []).filter((candidate) => matches(candidate, query.trim())),
    [candidates, query],
  );

  if (error) return <Banner tone="error">{error}</Banner>;
  if (candidates === null) return <Loading what={legend.toLowerCase()} />;
  if (candidates.length === 0) {
    return <EmptyState title={emptyTitle}>{emptyHint}</EmptyState>;
  }

  return (
    <fieldset className="personSearch" disabled={props.disabled}>
      <legend>{legend}</legend>

      <div className="field">
        <label htmlFor={`${base}-query`}>Search</label>
        <input
          id={`${base}-query`}
          type="text"
          value={query}
          placeholder="Name, code or email"
          onChange={(event) => setQuery(event.currentTarget.value)}
        />
        <p className="hint">
          {shown.length} of {candidates.length} shown. Matching is on every detail below,
          so a school&rsquo;s name narrows the list to the people it already holds.
        </p>
      </div>

      {shown.length === 0 ? (
        <p className="muted">Nobody matches that.</p>
      ) : (
        <ul className="personList">
          {shown.map((candidate) => {
            const id = `${base}-${candidate.id}`;
            const blocked = candidate.blockedBy ?? null;
            return (
              <li key={candidate.id}>
                <label htmlFor={id} className={blocked ? "personRow personRow--off" : "personRow"}>
                  <input
                    id={id}
                    type="radio"
                    name={`${base}-choice`}
                    value={candidate.id}
                    checked={selectedId === candidate.id}
                    disabled={blocked !== null}
                    onChange={() => onSelect(candidate.id)}
                  />
                  <span className="personName">{candidate.name}</span>
                  <span className="personDetails">
                    {candidate.details.length === 0 ? (
                      <span className="absent">No other details recorded</span>
                    ) : (
                      candidate.details.join(" · ")
                    )}
                  </span>
                  {blocked !== null && <span className="personBlocked">{blocked}</span>}
                </label>
              </li>
            );
          })}
        </ul>
      )}
    </fieldset>
  );
}
