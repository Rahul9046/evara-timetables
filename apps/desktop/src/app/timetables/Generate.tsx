/**
 * The Generate screen: choose a grid, check its inputs, build a timetable, keep it or not.
 *
 * ## The order is the gate
 *
 * Pre-solve validation runs **before** Generate is offered, and a blocking finding disables
 * it. That is M3's and M6's rule restated in the interface: *"a problem whose inputs were
 * refused is not searched at all"*. Searching anyway would report the affected lessons as
 * *"could not be placed"*, which reads as a difficulty rather than the unanswered setup
 * question it actually is.
 *
 * ## Saving is a second, explicit action
 *
 * A save **always inserts**. Several timetables coexist for one cycle and term, and that
 * coexistence is the versioning — so this screen never offers "overwrite", and it says how
 * many the grid already holds *before* generating rather than letting a fourth version
 * appear in a list unannounced.
 *
 * ## The one refusal with a remedy
 *
 * If a scheduling input moves between generating and saving, the save is refused and nothing
 * is written. That refusal is **correct** — nothing referential breaks when an availability
 * rule changes, so nothing else would catch it, and the result would be a plausible wrong
 * timetable. The screen therefore shows the remedy as a button rather than an error, because
 * a raw error here would make a working safety check look like a bug.
 *
 * ## The preview is the saved view
 *
 * The grid below the result is rendered from the same Rust projection a stored timetable's
 * view uses. There is no second renderer, so "what I previewed is what I saved" is a property
 * of the code rather than a hope.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  GenerateResult,
  ProjectionScope,
  ScopeKind,
  TimetableSummary,
  ValidationView,
} from "@evara/domain";

import {
  alreadySavedAs,
  cancelGeneration,
  describeTimetableError,
  discardGeneration,
  generationResult,
  generationScope,
  isInputsChanged,
  onSolverFinished,
  onSolverProgress,
  saveGeneration,
  startGeneration,
  validateInputs,
  type Selection,
} from "../../ipc/timetables";
import { Banner, Loading } from "../ui/Shell";
import { ScopePicker, useScopeLoad } from "./ScopeView";
import { UnscheduledPanel, UnverifiedPanel, UsabilityLine } from "./Disclosure";
import { TimetableGrid } from "./TimetableGrid";

/** How far the screen has got. Mirrors the backend's session phase, plus "not started". */
type Stage = "idle" | "running" | "finished" | "saved";

export function Generate(props: {
  selection: Selection | null;
  schoolName: string;
  onOpenSaved: (timetableId: string) => void;
  onOpenSection?: (section: string) => void;
}) {
  const { selection, schoolName, onOpenSaved, onOpenSection } = props;

  const [validation, setValidation] = useState<ValidationView | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [inputsMoved, setInputsMoved] = useState(false);

  const [runId, setRunId] = useState<string | null>(null);
  const [stage, setStage] = useState<Stage>("idle");
  const [progress, setProgress] = useState({ placed: 0, total: 0, elapsedMs: 0 });
  const [cancelling, setCancelling] = useState(false);
  const [result, setResult] = useState<GenerateResult | null>(null);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<TimetableSummary | null>(null);

  const [scopeKind, setScopeKind] = useState<ScopeKind>("master");
  const [scopeId, setScopeId] = useState<string | null>(null);

  // Validation re-runs whenever the chosen grid changes. It writes nothing, so this is safe.
  const key = selection === null ? null : `${selection.cycleId}|${selection.termId ?? ""}`;
  useEffect(() => {
    if (selection === null) {
      setValidation(null);
      return;
    }
    let live = true;
    setChecking(true);
    setError(null);
    validateInputs(selection)
      .then((found) => {
        if (live) setValidation(found);
      })
      .catch((e: unknown) => {
        if (live) setError(describeTimetableError(e));
      })
      .finally(() => {
        if (live) setChecking(false);
      });
    return () => {
      live = false;
    };
    // `key` is the selection's identity; `selection` itself is a fresh object each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  // Progress and completion, filtered to this run inside the IPC wrapper.
  const unlisten = useRef<Array<() => void>>([]);
  useEffect(() => {
    if (runId === null) return;
    let live = true;
    const subscribe = async () => {
      const off = [
        await onSolverProgress(runId, (event) => {
          if (!live) return;
          setProgress({
            placed: event.placed,
            total: event.total,
            elapsedMs: event.elapsedMs,
          });
        }),
        await onSolverFinished(runId, (event) => {
          if (!live) return;
          setProgress((was) => ({ ...was, elapsedMs: event.elapsedMs }));
          if (!event.hasResult) {
            setStage("idle");
            setRunId(null);
            return;
          }
          generationResult(runId)
            .then((found) => {
              if (!live) return;
              setResult(found);
              setStage("finished");
              setName(suggestName(found));
            })
            .catch((e: unknown) => {
              if (live) setError(describeTimetableError(e));
            });
        }),
      ];
      unlisten.current = off;
    };
    void subscribe();
    return () => {
      live = false;
      for (const off of unlisten.current) off();
      unlisten.current = [];
    };
  }, [runId]);

  const onGenerate = useCallback(async () => {
    if (selection === null) return;
    setError(null);
    setInputsMoved(false);
    setResult(null);
    setSaved(null);
    setProgress({ placed: 0, total: validation?.occurrences ?? 0, elapsedMs: 0 });
    setCancelling(false);
    try {
      const started = await startGeneration(selection);
      setRunId(started.runId);
      setStage("running");
      setProgress({ placed: 0, total: started.total, elapsedMs: 0 });
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    }
  }, [selection, validation?.occurrences]);

  const onCancel = useCallback(async () => {
    if (runId === null) return;
    setCancelling(true);
    try {
      await cancelGeneration(runId);
    } catch (e: unknown) {
      setError(describeTimetableError(e));
    }
  }, [runId]);

  const onDiscard = useCallback(async () => {
    if (runId !== null) await discardGeneration(runId).catch(() => undefined);
    setRunId(null);
    setResult(null);
    setSaved(null);
    setStage("idle");
    setInputsMoved(false);
  }, [runId]);

  const onSave = useCallback(async () => {
    if (runId === null) return;
    setSaving(true);
    setError(null);
    setInputsMoved(false);
    try {
      const written = await saveGeneration(runId, name);
      setSaved(written.timetable);
      setStage("saved");
      // M7 §13 item 3, answered: open it. The user asked for a timetable, and a screen that
      // answered with a list entry would make them click again for what they just made.
      onOpenSaved(written.timetable.id);
    } catch (e: unknown) {
      const already = alreadySavedAs(e);
      if (already !== null) {
        // Not a failure: it already succeeded. Offer the thing rather than the error.
        onOpenSaved(already);
        setStage("saved");
      } else if (isInputsChanged(e)) {
        setInputsMoved(true);
      } else {
        setError(describeTimetableError(e));
      }
    } finally {
      setSaving(false);
    }
  }, [runId, name, onOpenSaved]);

  const projection = result?.projection ?? null;
  const scopes = useMemo<ProjectionScope[]>(() => projection?.scopes ?? [], [projection]);
  // The layout comes from Rust, per scope. Nothing here computes a `rowSpan`.
  const loadScope = useMemo(
    () =>
      runId === null || result === null
        ? null
        : (kind: ScopeKind, id: string | null) => generationScope(runId, kind, id),
    [runId, result],
  );
  const { grid, error: gridError } = useScopeLoad(loadScope, scopeKind, scopeId);

  if (selection === null) {
    return (
      <div className="ttScreen">
        <h2>Generate a timetable</h2>
        <p className="ttEmpty">
          Choose a cycle to generate a timetable over. If this school has none,{" "}
          <button
            type="button"
            className="linkish"
            onClick={() => onOpenSection?.("cycles")}
          >
            create one under Cycles and cycle days
          </button>
          .
        </p>
      </div>
    );
  }

  const blocking = validation?.findings.filter((f) => f.blocks) ?? [];
  const advisory = validation?.findings.filter((f) => !f.blocks) ?? [];
  const canGenerate = validation?.maySolve === true && stage !== "running";

  return (
    <div className="ttScreen">
      <h2>Generate a timetable</h2>
      {error === null ? null : <Banner tone="error">{error}</Banner>}

      {checking ? <Loading what="this timetable" /> : null}

      {validation === null ? null : (
        <section className="ttPanel" aria-labelledby="ttInputs">
          <h3 id="ttInputs">Before generating</h3>
          <p className="ttInputsLine">
            {validation.occurrences} lesson
            {validation.occurrences === 1 ? "" : "s"} to place.{" "}
            {validation.existingTimetables === 0
              ? "This grid holds no timetable yet."
              : `This grid already holds ${validation.existingTimetables} timetable${
                  validation.existingTimetables === 1 ? "" : "s"
                } — saving adds another rather than replacing one.`}
          </p>

          {blocking.length > 0 ? (
            <div className="ttBlocking">
              <h4>
                <span aria-hidden="true">⚠</span> These have to be fixed first
              </h4>
              <p className="ttUnplacedNote">
                Each of these leaves a lesson with no legal placement at all, so generating
                would report it as unplaceable rather than as the unanswered setup question it
                is.
              </p>
              <ul>
                {blocking.map((finding, at) => (
                  <li key={`${finding.code}-${at}`}>
                    {finding.message}
                    {finding.remedy === null ? null : (
                      <span className="ttRemedy"> {finding.remedy}</span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          {advisory.length > 0 ? (
            <details className="ttAdvisory">
              <summary>
                {advisory.length} thing{advisory.length === 1 ? "" : "s"} worth knowing —
                generation is still available
              </summary>
              <ul>
                {advisory.map((finding, at) => (
                  <li key={`${finding.code}-${at}`}>
                    {finding.message}
                    {finding.remedy === null ? null : (
                      <span className="ttRemedy"> {finding.remedy}</span>
                    )}
                  </li>
                ))}
              </ul>
            </details>
          ) : null}

          <div className="ttActions">
            <button
              type="button"
              className="primary"
              disabled={!canGenerate}
              onClick={() => void onGenerate()}
            >
              {stage === "running" ? "Generating…" : "Generate"}
            </button>
            {stage === "running" ? (
              <button
                type="button"
                className="secondary"
                disabled={cancelling}
                onClick={() => void onCancel()}
              >
                {cancelling ? "Stopping…" : "Stop"}
              </button>
            ) : null}
            {validation.maySolve ? null : (
              <p className="ttDisabledWhy">
                Generation is unavailable until the items above are corrected.
              </p>
            )}
          </div>
        </section>
      )}

      {stage === "running" ? (
        <section className="ttPanel" aria-labelledby="ttProgress">
          <h3 id="ttProgress">Working</h3>
          <div
            className="ttProgress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={progress.total}
            aria-valuenow={progress.placed}
            aria-labelledby="ttProgress"
          >
            <div
              className="ttProgressFill"
              style={{
                width:
                  progress.total === 0
                    ? "0%"
                    : `${Math.round((progress.placed / progress.total) * 100)}%`,
              }}
            />
          </div>
          {/* Polite, and updated at the coalesced rate rather than per placement, so a
              screen reader is not flooded by 2 000 announcements. */}
          <p className="ttProgressText" aria-live="polite">
            {progress.placed} of {progress.total} lessons placed
            {cancelling ? " — stopping after the current step" : ""}.
          </p>
        </section>
      ) : null}

      {inputsMoved ? (
        <section className="ttPanel ttPanel--moved" aria-labelledby="ttMoved">
          <h3 id="ttMoved">Nothing was saved</h3>
          <p>
            A scheduling input changed since this timetable was generated, so saving it could
            store a wrong timetable. Nothing was written.
          </p>
          <div className="ttActions">
            <button
              type="button"
              className="primary"
              onClick={() => void onGenerate()}
            >
              Generate again
            </button>
            <button type="button" className="secondary" onClick={() => void onDiscard()}>
              Discard this result
            </button>
          </div>
        </section>
      ) : null}

      {result === null ? null : (
        <>
          <section className="ttPanel" aria-labelledby="ttResult">
            <h3 id="ttResult">Result</h3>
            <UsabilityLine
              counts={{
                hard: result.outcome.hardViolations,
                medium: 0,
                soft: 0,
                unknown: 0,
              }}
              placed={result.outcome.placed}
              required={result.outcome.required}
              schoolName={schoolName}
            />
            <p className="ttResultMeta">
              Score {result.outcome.hard} / {result.outcome.medium} / {result.outcome.soft}{" "}
              (hard / medium / soft) · stopped because:{" "}
              <code>{result.outcome.termination}</code>
              {result.outcome.wasCancelled ? (
                <span className="ttResultMeta--cancelled">
                  {" "}
                  — a cancelled run is the one result Evara cannot reproduce from its seed.
                </span>
              ) : (
                <span> · seed {result.outcome.seed}</span>
              )}
            </p>

            {result.notSaveable === null ? (
              <div className="ttSaveRow">
                <label htmlFor="ttName">Name this timetable</label>
                <input
                  id="ttName"
                  type="text"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  disabled={stage === "saved" || saving}
                />
                <button
                  type="button"
                  className="primary"
                  disabled={saving || stage === "saved" || name.trim() === ""}
                  onClick={() => void onSave()}
                >
                  {stage === "saved" ? "Saved" : saving ? "Saving…" : "Save"}
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={saving}
                  onClick={() => void onDiscard()}
                >
                  Discard
                </button>
                <p className="ttSaveNote">
                  Saving adds a new timetable. Nothing existing is replaced.
                </p>
              </div>
            ) : (
              <div className="ttNotSaveable">
                <h4>
                  <span aria-hidden="true">⚠</span> Nothing to save
                </h4>
                <p>{result.notSaveable}</p>
              </div>
            )}

            {saved === null ? null : (
              <p className="ttSaved">
                Saved as <strong>{saved.name}</strong>.
              </p>
            )}
          </section>

          <UnverifiedPanel counts={result.outcome.unverified} />
          <UnscheduledPanel unscheduled={result.outcome.unscheduled} />

          <section className="ttPanel" aria-labelledby="ttPreview">
            <h3 id="ttPreview">Preview</h3>
            <ScopePicker
              scopes={scopes}
              kind={scopeKind}
              id={scopeId}
              hasEnrolments={projection?.hasEnrolments ?? false}
              onChange={(nextKind, nextId) => {
                setScopeKind(nextKind);
                setScopeId(nextId);
              }}
            />
            {gridError === null ? null : <Banner tone="error">{gridError}</Banner>}
            {grid === null ? null : (
              <TimetableGrid grid={grid} lessons={projection?.lessons ?? []} />
            )}
          </section>
        </>
      )}
    </div>
  );
}

/**
 * A default name, so the save box is not empty.
 *
 * Derived from the outcome rather than from a clock the frontend reads: the status and the
 * seed are what distinguish two runs of one grid, and they are what a user comparing
 * versions actually looks at.
 */
function suggestName(result: GenerateResult): string {
  const kind = result.outcome.placed >= result.outcome.required ? "Complete" : "Partial";
  return `${kind} — seed ${result.outcome.seed}`;
}
