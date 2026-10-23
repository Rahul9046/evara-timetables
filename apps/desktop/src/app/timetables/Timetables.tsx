/**
 * The Timetables area: the versions a school holds, and one of them opened.
 *
 * ## A list row is a version
 *
 * Several timetables coexist for one cycle and term, and that coexistence **is** the
 * versioning. So this screen invents no version number, draws no "current" badge, and orders
 * by when each was generated. Replacement is not an operation: it is a save followed by an
 * explicit delete, which is two decisions by the user rather than one silent one.
 *
 * ## Read-only, and visibly so
 *
 * There is no drag handle, no drop target and no hover validation — **absent** rather than
 * disabled, because a greyed-out affordance teaches a user that editing exists here and in
 * this milestone it does not. The four writes in the whole area are save (on the Generate
 * screen), rename, delete and revalidate.
 *
 * `isLocked` and `isManual` are rendered where set and written nowhere, so that when Phase 8
 * begins writing them the grid already tells the truth.
 *
 * ## A timetable whose grid moved is still worth opening
 *
 * If the problem cannot be compiled, there is no grid — and the screen says why, shows the
 * stored findings, and stops. It never draws a partial grid: deriving spans without a problem
 * would be a second implementation of the span measure.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type {
  ProjectionScope,
  ScopeKind,
  TimetableSummary,
  TimetableView,
} from "@evara/domain";

import {
  deleteTimetable,
  describeTimetableError,
  listTimetables,
  openTimetable,
  renameTimetable,
  revalidateTimetable,
  timetableScope,
} from "../../ipc/timetables";
import { Banner, Loading } from "../ui/Shell";
import {
  FindingsPanel,
  ScoringPanel,
  UnscheduledPanel,
  UnverifiedPanel,
  UsabilityLine,
} from "./Disclosure";
import { ScopePicker, useScopeLoad } from "./ScopeView";
import { TimetableGrid } from "./TimetableGrid";

export function Timetables(props: {
  schoolId: string | null;
  schoolName: string;
  openId: string | null;
  onOpen: (id: string | null) => void;
  onGenerate: () => void;
  onOpenSection?: (section: string) => void;
}) {
  const { schoolId, schoolName, openId, onOpen, onGenerate, onOpenSection } = props;

  const [list, setList] = useState<TimetableSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (schoolId === null) {
      setList(null);
      return;
    }
    try {
      setList(await listTimetables(schoolId));
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    }
  }, [schoolId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (schoolId === null) {
    return (
      <div className="ttScreen">
        <h2>Timetables</h2>
        <p className="ttEmpty">
          Select a school to see its timetables. If this campus file has none,{" "}
          <button
            type="button"
            className="linkish"
            onClick={() => onOpenSection?.("schools")}
          >
            create one under Schools
          </button>
          .
        </p>
      </div>
    );
  }

  if (openId !== null) {
    return (
      <OpenTimetable
        id={openId}
        schoolName={schoolName}
        onBack={() => {
          onOpen(null);
          void refresh();
        }}
        onChanged={() => void refresh()}
      />
    );
  }

  return (
    <div className="ttScreen">
      <h2>Timetables</h2>
      {error === null ? null : <Banner tone="error">{error}</Banner>}

      <div className="ttActions">
        <button type="button" className="primary" onClick={onGenerate}>
          Generate a timetable
        </button>
      </div>

      {list === null ? (
        <Loading what="this timetable" />
      ) : list.length === 0 ? (
        <p className="ttEmpty">
          {schoolName} has no saved timetable yet. Generating one does not replace anything —
          every timetable you save is kept alongside the others.
        </p>
      ) : (
        <div className="ttScroll">
          <table className="ttListTable">
            <caption className="srOnly">
              Every timetable this school holds, newest first. Each row is a separate version.
            </caption>
            <thead>
              <tr>
                <th scope="col">Name</th>
                <th scope="col">Generated</th>
                <th scope="col">Undisclosed</th>
                <th scope="col">
                  <span className="srOnly">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {list.map((timetable) => (
                <tr key={timetable.id}>
                  <th scope="row">
                    <button
                      type="button"
                      className="linkish"
                      onClick={() => onOpen(timetable.id)}
                    >
                      {timetable.name}
                    </button>
                  </th>
                  <td>{timetable.generatedAt}</td>
                  <td className="ttNum">{undisclosed(timetable)}</td>
                  <td>
                    <button
                      type="button"
                      className="secondary"
                      onClick={() => onOpen(timetable.id)}
                    >
                      Open
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** How many of the four disclosures this timetable has something to say about. */
function undisclosed(timetable: TimetableSummary): number {
  const counts = timetable.unverified;
  return [
    counts.indeterminateStudentPairs,
    counts.uncheckedCapacities,
    counts.sharedEntities,
    counts.unenforcedRestrictions,
  ].filter((n) => n > 0).length;
}

/** One opened timetable: its grid where there is one, and everything it discloses. */
function OpenTimetable(props: {
  id: string;
  schoolName: string;
  onBack: () => void;
  onChanged: () => void;
}) {
  const { id, schoolName, onBack, onChanged } = props;

  const [view, setView] = useState<TimetableView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);

  const [scopeKind, setScopeKind] = useState<ScopeKind>("master");
  const [scopeId, setScopeId] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const found = await openTimetable(id);
      setView(found);
      setDraftName(found.timetable.name);
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const projection = view?.projection ?? null;
  const scopes = useMemo<ProjectionScope[]>(() => projection?.scopes ?? [], [projection]);
  // The layout is Rust's, per scope. Nothing here decides a `rowSpan`.
  const loadScope = useMemo(
    () =>
      projection === null
        ? null
        : (kind: ScopeKind, chosen: string | null) => timetableScope(id, kind, chosen),
    [id, projection],
  );
  const { grid, error: gridError } = useScopeLoad(loadScope, scopeKind, scopeId);

  const onRename = useCallback(async () => {
    if (view === null) return;
    setBusy(true);
    setError(null);
    try {
      await renameTimetable(id, view.timetable.rev, draftName);
      setRenaming(false);
      await load();
      onChanged();
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    } finally {
      setBusy(false);
    }
  }, [id, view, draftName, load, onChanged]);

  const onDelete = useCallback(async () => {
    if (view === null) return;
    setBusy(true);
    setError(null);
    try {
      await deleteTimetable(id, view.timetable.rev);
      onChanged();
      onBack();
    } catch (e: unknown) {
      setError(describeTimetableError(e));
      setConfirmDelete(false);
    } finally {
      setBusy(false);
    }
  }, [id, view, onBack, onChanged]);

  const onRevalidate = useCallback(async () => {
    if (view === null) return;
    setBusy(true);
    setError(null);
    try {
      // Carries the revision this screen was holding: the write consumes it, because the
      // timetable's current findings are part of what it says about itself.
      await revalidateTimetable(id, view.timetable.rev);
      await load();
      onChanged();
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    } finally {
      setBusy(false);
    }
  }, [id, view, load, onChanged]);

  if (view === null) {
    return (
      <div className="ttScreen">
        <button type="button" className="linkish" onClick={onBack}>
          ← All timetables
        </button>
        {error === null ? <Loading what="this timetable" /> : <Banner tone="error">{error}</Banner>}
      </div>
    );
  }

  const current = view.scorings.find((scoring) => !scoring.isGeneration) ?? view.scorings[0];

  return (
    <div className="ttScreen">
      <button type="button" className="linkish" onClick={onBack}>
        ← All timetables
      </button>

      <div className="ttOpenHead">
        {renaming ? (
          <div className="ttRenameRow">
            <label htmlFor="ttRename">Name</label>
            <input
              id="ttRename"
              type="text"
              value={draftName}
              onChange={(event) => setDraftName(event.target.value)}
              disabled={busy}
            />
            <button
              type="button"
              className="primary"
              disabled={busy || draftName.trim() === ""}
              onClick={() => void onRename()}
            >
              Save name
            </button>
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => {
                setRenaming(false);
                setDraftName(view.timetable.name);
              }}
            >
              Cancel
            </button>
          </div>
        ) : (
          <h2>{view.timetable.name}</h2>
        )}
        <p className="ttOpenMeta">Generated {view.timetable.generatedAt}</p>
      </div>

      {error === null ? null : <Banner tone="error">{error}</Banner>}

      <div className="ttActions">
        {renaming ? null : (
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => setRenaming(true)}
          >
            Rename
          </button>
        )}
        <button
          type="button"
          className="secondary"
          disabled={busy || view.projection === null}
          onClick={() => void onRevalidate()}
        >
          Re-score against today's inputs
        </button>
        {confirmDelete ? (
          <span className="ttConfirm">
            <span>
              Delete this timetable? It will remove {view.storedEntries} placed lesson
              {view.storedEntries === 1 ? "" : "s"} and everything recorded about how it was
              generated. This cannot be undone.
            </span>
            <button
              type="button"
              className="danger"
              disabled={busy}
              onClick={() => void onDelete()}
            >
              Delete
            </button>
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => setConfirmDelete(false)}
            >
              Keep
            </button>
          </span>
        ) : (
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={() => setConfirmDelete(true)}
          >
            Delete
          </button>
        )}
      </div>

      {current === undefined ? null : (
        <UsabilityLine
          counts={view.counts}
          placed={current.placed}
          required={current.required}
          schoolName={schoolName}
        />
      )}

      {view.unreadable === null ? null : (
        <section className="ttPanel ttPanel--moved" aria-labelledby="ttNoGrid">
          <h3 id="ttNoGrid">No grid could be built</h3>
          <p>
            {view.unreadable}. The findings recorded for this timetable are below, but it
            cannot be drawn until the setup it was built over is restored.
          </p>
        </section>
      )}

      {view.notInProblem.length === 0 ? null : (
        <section className="ttPanel ttPanel--moved" aria-labelledby="ttDropped">
          <h3 id="ttDropped">This timetable is readable but incomplete</h3>
          <p>
            {view.notInProblem.length} of its {view.storedEntries} placed lesson
            {view.storedEntries === 1 ? "" : "s"} name something the campus file no longer
            holds, so they are not in the grid below. What you are looking at is less than
            what was saved.
          </p>
        </section>
      )}

      <ScoringPanel scorings={view.scorings} />
      <UnverifiedPanel
        counts={view.timetable.unverified}
        shared={view.shared}
        canName={view.projection !== null}
      />
      <UnscheduledPanel unscheduled={view.unscheduled} />
      <FindingsPanel findings={view.findings} counts={view.counts} />

      {view.projection === null ? null : (
        <section className="ttPanel" aria-labelledby="ttGridPanel">
          <h3 id="ttGridPanel">Timetable</h3>
          <ScopePicker
            scopes={scopes}
            kind={scopeKind}
            id={scopeId}
            hasEnrolments={view.projection.hasEnrolments}
            onChange={(nextKind, nextId) => {
              setScopeKind(nextKind);
              setScopeId(nextId);
            }}
          />
          {gridError === null ? null : <Banner tone="error">{gridError}</Banner>}
          {grid === null ? null : (
            <TimetableGrid grid={grid} lessons={view.projection.lessons} />
          )}
        </section>
      )}

      {view.run === null ? null : (
        <section className="ttPanel" aria-labelledby="ttRun">
          <h3 id="ttRun">How this was generated</h3>
          <dl className="ttRunFacts">
            <dt>Engine</dt>
            <dd>{view.run.engine}</dd>
            <dt>Seed</dt>
            <dd>{view.run.seed}</dd>
            <dt>Result</dt>
            <dd>
              {view.run.status} — {view.run.termination}
            </dd>
            <dt>Ran</dt>
            <dd>
              {view.run.startedAt} to {view.run.finishedAt}
            </dd>
            <dt>Evara version</dt>
            <dd>{view.run.appVersion}</dd>
          </dl>
        </section>
      )}
    </div>
  );
}
