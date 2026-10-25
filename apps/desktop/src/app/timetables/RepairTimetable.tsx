/**
 * The Repair screen: choose what may move, check the locks, run, review, apply.
 *
 * ## A screen of its own, for the reason editing is
 *
 * A repair session holds a compiled problem, a baseline arrangement and an arrangement stamp
 * from one snapshot. Leaving the screen ends the session, which is why Apply and Discard are
 * both explicit and why Discard is what leaving does.
 *
 * ## The scope is chosen, never defaulted
 *
 * A repair over a scope the user did not pick is a rearrangement they did not ask for. The
 * picker starts with nothing selected and Repair is unavailable until something is.
 *
 * ## The scope is never widened for the user
 *
 * Where a wider scope would plainly help, this screen offers to start a **separate** repair —
 * a new session, a new baseline, a new preview, a new approval. It never grows the movable set
 * of the run in front of them, because a lesson moved outside the chosen scope is a change
 * they had no opportunity to review.
 *
 * ## Two warnings the screen owes the user before anything runs
 *
 * A **positional** scope — a room or a day — cannot schedule an unscheduled lesson, because an
 * unscheduled lesson is in no room and on no day. And lessons outside the scope will not move
 * even where moving one would fix an in-scope problem. Both are stated before Repair, not
 * discovered in the diff.
 *
 * ## What the screen never says
 *
 * Not *"nothing could be improved"* — a greedy construction that changed nothing has
 * discovered that its own walk found the baseline legal, not that no better arrangement
 * exists. The wording is **"This repair attempt made no changes."**
 *
 * Not *"conflict-free"* — `UsabilityLine`'s existing wording, with the unverified counts
 * beside it (ADR 0032 rule 4).
 *
 * And a pre-existing problem outside the scope is labelled as such rather than counted against
 * the repair, because it is not the repair's doing and does not stop it being applied.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  RepairPreflight,
  RepairResult,
  RepairScope,
  RepairScopes,
  ScopeOption,
} from "@evara/domain";

import {
  applyRepair,
  cancelRepair,
  describeRepairError,
  discardRepair,
  isArrangementChanged,
  isInputsChanged,
  isScopeViolation,
  onRepairFinished,
  onRepairProgress,
  repairPreflight,
  repairResult,
  repairScopes,
  startRepair,
} from "../../ipc/repair";
import type { Selection } from "../../ipc/timetables";
import { Banner, Loading, Panel } from "../ui/Shell";
import { UsabilityLine } from "./Disclosure";

/** How far the screen has got. Mirrors the backend's phase, plus "choosing". */
type Stage = "choosing" | "running" | "finished" | "applied";

/** The kinds, in the order the picker offers them. */
const KIND_LABELS: Readonly<Record<string, string>> = {
  whole: "Whole timetable",
  classSection: "One class",
  teacher: "One teacher",
  room: "One room",
  cycleDay: "One day",
  occurrences: "Selected lessons",
};

export function RepairTimetable(props: {
  selection: Selection;
  timetableId: string;
  timetableName: string;
  schoolName: string;
  onApplied: (timetableId: string) => void;
  onDone: () => void;
}) {
  const { selection, timetableId, timetableName, schoolName, onApplied, onDone } = props;

  const [scopes, setScopes] = useState<RepairScopes | null>(null);
  const [chosen, setChosen] = useState<ScopeOption | null>(null);
  const [preflight, setPreflight] = useState<RepairPreflight | null>(null);
  const [checking, setChecking] = useState(false);

  const [stage, setStage] = useState<Stage>("choosing");
  const [runId, setRunId] = useState<string | null>(null);
  const [progress, setProgress] = useState({ placed: 0, total: 0 });
  const [result, setResult] = useState<RepairResult | null>(null);

  const [name, setName] = useState(`${timetableName} (repaired)`);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stale, setStale] = useState<"inputs" | "arrangement" | null>(null);
  const [defect, setDefect] = useState(false);

  const live = useRef<HTMLDivElement | null>(null);

  // ---------------------------------------------------------------- the scope menu

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const found = await repairScopes(selection, timetableId);
        if (!cancelled) setScopes(found);
      } catch (caught) {
        if (!cancelled) setError(describeRepairError(caught));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selection, timetableId]);

  // The pre-flight follows the chosen scope, and writes nothing, so it is safe on every change.
  useEffect(() => {
    if (chosen === null) {
      setPreflight(null);
      return undefined;
    }
    let cancelled = false;
    setChecking(true);
    void (async () => {
      try {
        const checked = await repairPreflight(selection, timetableId, chosen.scope);
        if (!cancelled) setPreflight(checked);
      } catch (caught) {
        if (!cancelled) setError(describeRepairError(caught));
      } finally {
        if (!cancelled) setChecking(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [chosen, selection, timetableId]);

  // ---------------------------------------------------------------- the run

  useEffect(() => {
    if (runId === null) return undefined;
    let unlistenProgress: (() => void) | null = null;
    let unlistenFinished: (() => void) | null = null;
    let cancelled = false;

    void (async () => {
      unlistenProgress = await onRepairProgress(runId, (event) => {
        if (!cancelled) setProgress({ placed: event.placed, total: event.total });
      });
      unlistenFinished = await onRepairFinished(runId, () => {
        if (cancelled) return;
        void (async () => {
          try {
            const found = await repairResult(runId);
            if (!cancelled) {
              setResult(found);
              setStage("finished");
              setDefect(found.defect !== null);
            }
          } catch (caught) {
            if (!cancelled) setError(describeRepairError(caught));
          }
        })();
      });
    })();

    return () => {
      cancelled = true;
      unlistenProgress?.();
      unlistenFinished?.();
    };
  }, [runId]);

  const begin = useCallback(async () => {
    if (chosen === null) return;
    setBusy(true);
    setError(null);
    setStale(null);
    try {
      const started = await startRepair(selection, timetableId, chosen.scope);
      setRunId(started.runId);
      setProgress({ placed: 0, total: started.movable });
      setStage("running");
    } catch (caught) {
      setError(describeRepairError(caught));
    } finally {
      setBusy(false);
    }
  }, [chosen, selection, timetableId]);

  const stop = useCallback(async () => {
    if (runId === null) return;
    try {
      await cancelRepair(runId);
    } catch (caught) {
      setError(describeRepairError(caught));
    }
  }, [runId]);

  const apply = useCallback(async () => {
    if (runId === null) return;
    setBusy(true);
    setError(null);
    setStale(null);
    try {
      const written = await applyRepair(runId, name);
      setStage("applied");
      onApplied(written.timetableId);
    } catch (caught) {
      if (isArrangementChanged(caught)) setStale("arrangement");
      else if (isInputsChanged(caught)) setStale("inputs");
      else if (isScopeViolation(caught)) setDefect(true);
      else setError(describeRepairError(caught));
    } finally {
      setBusy(false);
    }
  }, [runId, name, onApplied]);

  const discard = useCallback(async () => {
    if (runId !== null) {
      try {
        await discardRepair(runId);
      } catch {
        // Discarding a session that is already gone is not a failure worth reporting.
      }
    }
    onDone();
  }, [runId, onDone]);

  // ---------------------------------------------------------------- rendering

  const blocking = useMemo(
    () => preflight?.findings.filter((finding) => finding.blocking) ?? [],
    [preflight],
  );
  const advisory = useMemo(
    () => preflight?.findings.filter((finding) => !finding.blocking) ?? [],
    [preflight],
  );

  if (scopes === null && error === null) {
    return <Loading what="this timetable" />;
  }

  return (
    <div className="ttScreen ttRepair">
      <header className="ttRepairHead">
        <h2>Repair “{timetableName}”</h2>
        <p className="ttRepairLead">
          A repair makes a <strong>new</strong> timetable. “{timetableName}” is left exactly as
          it is.
        </p>
      </header>

      {error !== null && (
        <Banner tone="error">
          <strong>Evara could not do that</strong>{" "}
          {error}
        </Banner>
      )}

      {defect && (
        <Banner tone="error">
          <strong>This is a defect in Evara, not a problem with your timetable</strong>{" "}
          The repair changed a lesson it was told not to touch, so nothing was saved. Your
          timetable is unchanged. Please report this — it is our bug to fix, not your data to
          correct.
        </Banner>
      )}

      {stale === "arrangement" && (
        <Banner tone="warning">
          <strong>This timetable was edited while you were repairing it</strong>{" "}
          The comparison you are looking at describes a timetable that no longer exists, so
          nothing was saved. Run the repair again from the timetable as it now stands.
        </Banner>
      )}

      {stale === "inputs" && (
        <Banner tone="warning">
          <strong>Something the timetable depends on changed</strong>{" "}
          A room, a teacher, an availability rule or part of the grid moved while the repair was
          running, so every judgement it made is out of date and nothing was saved. Run it again.
        </Banner>
      )}

      {stage === "choosing" && (
        <>
          <ScopeRadioGroup
            scopes={scopes}
            chosen={chosen}
            onChoose={setChosen}
          />

          {chosen !== null && (
            <Panel title="Before you repair">
              <ul className="ttRepairWarnings">
                <li>
                  Lessons outside this scope <strong>will not move</strong>, even where moving
                  one would fix a problem inside it. Where that happens the lesson inside the
                  scope is reported as unscheduled rather than fixed by reaching outside.
                </li>
                {chosen.positional && chosen.unreachableUnplaced > 0 && (
                  <li>
                    This scope <strong>cannot schedule your {chosen.unreachableUnplaced}{" "}
                    unscheduled {chosen.unreachableUnplaced === 1 ? "lesson" : "lessons"}</strong>
                    , because an unscheduled lesson is in no room and on no day. Repair the whole
                    timetable, or select those lessons, to place them.
                  </li>
                )}
                <li>
                  {chosen.movable} {chosen.movable === 1 ? "lesson" : "lessons"} may move;{" "}
                  {chosen.frozen} {chosen.frozen === 1 ? "is" : "are"} frozen.
                </li>
              </ul>
            </Panel>
          )}

          {checking && <Loading what="this scope" />}

          {blocking.length > 0 && (
            <Banner tone="error">
          <strong>This timetable cannot be repaired yet</strong>{" "}
              <ul className="ttRepairFindings">
                {blocking.map((finding, at) => (
                  <li key={`${finding.code}-${at}`}>
                    <span className="ttRepairFindingMessage">{finding.message}</span>{" "}
                    <span className="ttRepairFindingRemedy">{finding.remedy}</span>
                  </li>
                ))}
              </ul>
            </Banner>
          )}

          {advisory.length > 0 && (
            <Panel
              title={`Already present, outside this scope (${advisory.length})`}
              note="These are not caused by the repair and do not stop you applying it."
            >
              <ul className="ttRepairFindings">
                {advisory.map((finding, at) => (
                  <li key={`${finding.code}-${at}`}>
                    <span className="ttRepairFindingMessage">{finding.message}</span>{" "}
                    <span className="ttRepairFindingRemedy">{finding.remedy}</span>
                  </li>
                ))}
              </ul>
            </Panel>
          )}

          <div className="ttRepairActions">
            <button
              type="button"
              className="primary"
              disabled={
                chosen === null || checking || blocking.length > 0 || busy
              }
              onClick={() => void begin()}
            >
              Repair
            </button>
            <button type="button" onClick={() => void discard()}>
              Cancel
            </button>
          </div>
        </>
      )}

      {stage === "running" && (
        <Panel title="Repairing">
          <div aria-live="polite" ref={live} className="ttRepairProgress">
            {progress.total > 0
              ? `Reconsidered ${progress.placed} of ${progress.total} lessons.`
              : "Starting…"}
          </div>
          <progress
            max={Math.max(progress.total, 1)}
            value={progress.placed}
            aria-label="Repair progress"
          />
          <p className="ttRepairHint">
            Cancelling asks the repair to stop. It may already have finished, in which case
            nothing is cancelled — and nothing is written either way.
          </p>
          <button type="button" onClick={() => void stop()}>
            Cancel this repair
          </button>
        </Panel>
      )}

      {(stage === "finished" || stage === "applied") && result !== null && (
        <RepairPreview
          result={result}
          schoolName={schoolName}
          name={name}
          busy={busy}
          applied={stage === "applied"}
          onName={setName}
          onApply={() => void apply()}
          onDiscard={() => void discard()}
          onWiderScope={() => {
            setStage("choosing");
            setChosen(null);
            setResult(null);
            setRunId(null);
          }}
        />
      )}
    </div>
  );
}

/**
 * The scope picker, as a real radio group.
 *
 * `role="radiogroup"` with arrow-key navigation and one tab stop, which is what a native
 * radio group gives and what a list of buttons does not. Every count is text before it is
 * anything else.
 */
function ScopeRadioGroup(props: {
  scopes: RepairScopes | null;
  chosen: ScopeOption | null;
  onChoose: (option: ScopeOption) => void;
}) {
  const { scopes, chosen, onChoose } = props;
  const options = scopes?.options ?? [];

  const move = useCallback(
    (from: number, delta: number) => {
      if (options.length === 0) return;
      const next = (from + delta + options.length) % options.length;
      const option = options[next];
      if (option === undefined) return;
      onChoose(option);
      // The newly selected radio is the only tab stop, so focus follows selection — which is
      // what a native radio group does.
      const node = document.getElementById(`ttRepairScope-${next}`);
      node?.focus();
    },
    [options, onChoose],
  );

  if (scopes === null) return null;

  if (options.length === 0) {
    return (
      <Banner tone="info">
          <strong>There is nothing to repair</strong>{" "}
        This timetable holds no lessons that could move.
      </Banner>
    );
  }

  const selectedAt = options.findIndex((option) => sameScope(option.scope, chosen?.scope));

  return (
    <fieldset className="ttRepairScopes">
      <legend>What may move?</legend>
      <p className="ttRepairHint" id="ttRepairScopeHint">
        Nothing is chosen for you. {scopes.total} lessons in total,{" "}
        {scopes.unplaced} unscheduled.
      </p>
      <div role="radiogroup" aria-labelledby="ttRepairScopeHint" className="ttRepairScopeList">
        {options.map((option, at) => {
          const selected = at === selectedAt;
          return (
            <label
              key={`${option.key}-${option.label}-${at}`}
              className={`ttRepairScope${selected ? " ttRepairScope--on" : ""}`}
            >
              <input
                id={`ttRepairScope-${at}`}
                type="radio"
                name="ttRepairScope"
                checked={selected}
                // One tab stop for the group: the selected radio, or the first when none is.
                tabIndex={selected || (selectedAt === -1 && at === 0) ? 0 : -1}
                onChange={() => onChoose(option)}
                onKeyDown={(event) => {
                  if (event.key === "ArrowDown" || event.key === "ArrowRight") {
                    event.preventDefault();
                    move(at, 1);
                  } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
                    event.preventDefault();
                    move(at, -1);
                  }
                }}
              />
              <span className="ttRepairScopeKind">{KIND_LABELS[option.key] ?? option.key}</span>
              <span className="ttRepairScopeLabel">{option.label}</span>
              <span className="ttRepairScopeCounts">
                {option.movable} may move, {option.frozen} frozen
                {option.positional && option.unreachableUnplaced > 0
                  ? `, ${option.unreachableUnplaced} unscheduled out of reach`
                  : ""}
              </span>
            </label>
          );
        })}
      </div>
    </fieldset>
  );
}

/** The before-and-after, and the two explicit decisions that end the session. */
function RepairPreview(props: {
  result: RepairResult;
  schoolName: string;
  name: string;
  busy: boolean;
  applied: boolean;
  onName: (name: string) => void;
  onApply: () => void;
  onDiscard: () => void;
  onWiderScope: () => void;
}) {
  const { result, schoolName, name, busy, applied, onName, onApply, onDiscard, onWiderScope } =
    props;
  const { diff } = result;
  const counts = diff.counts;

  const changes = useMemo(
    () => diff.changes.filter((change) => change.kind !== "unchanged"),
    [diff.changes],
  );
  const blockedInScope = useMemo(
    () =>
      diff.changes.filter(
        (change) => change.kind === "stillUnplaced" && change.inScope,
      ),
    [diff.changes],
  );
  const outOfReach = useMemo(
    () =>
      diff.changes.filter(
        (change) => change.kind === "stillUnplaced" && !change.inScope,
      ),
    [diff.changes],
  );

  return (
    <>
      {counts.moved + counts.newlyScheduled + counts.unscheduled === 0 ? (
        <Banner tone="info">
          <strong>This repair attempt made no changes</strong>{" "}
          {diff.movableCount} {diff.movableCount === 1 ? "lesson was" : "lessons were"}{" "}
          reconsidered; each is still legal where it was. That is not a statement that nothing
          could be improved — only that this attempt found no change to make.
        </Banner>
      ) : (
        <Panel title="What would change">
          <dl className="ttRepairCounts">
            <dt>Moved</dt>
            <dd>{counts.moved}</dd>
            <dt>Newly scheduled</dt>
            <dd>{counts.newlyScheduled}</dd>
            <dt>No longer scheduled</dt>
            <dd>{counts.unscheduled}</dd>
            <dt>Unchanged</dt>
            <dd>{counts.unchanged}</dd>
          </dl>
        </Panel>
      )}

      {result.stoppedAtALimit && (
        <Banner tone="warning">
          <strong>A limit stopped this repair</strong>{" "}
          It reached a built-in budget before it finished looking. What it found is usable and
          can be applied, but it is not a conclusion about what is possible.
        </Banner>
      )}

      {result.notApplyable !== null && (
        <Banner tone="warning">
          <strong>This result cannot be applied</strong>{" "}
          {result.notApplyable}
        </Banner>
      )}

      <Panel
        title="Score"
        note="Three levels, compared in order. Lower is worse; zero is the best reachable."
      >
        <table className="ttRepairScore">
          <caption className="srOnly">
            The timetable&rsquo;s score before and after this repair, at three levels
          </caption>
          <thead>
            <tr>
              <th scope="col">Level</th>
              <th scope="col">Before</th>
              <th scope="col">After</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <th scope="row">Hard</th>
              <td>{diff.scoreBefore.hard}</td>
              <td>{diff.scoreAfter.hard}</td>
            </tr>
            <tr>
              <th scope="row">Medium</th>
              <td>{diff.scoreBefore.medium}</td>
              <td>{diff.scoreAfter.medium}</td>
            </tr>
            <tr>
              <th scope="row">Soft</th>
              <td>{diff.scoreBefore.soft}</td>
              <td>{diff.scoreAfter.soft}</td>
            </tr>
          </tbody>
        </table>
        <p className="ttRepairHint">
          {diff.improved
            ? "This repair improves the score."
            : "This repair does not improve the score."}
        </p>
      </Panel>

      {diff.attribution.eliminated.length > 0 && (
        <Panel title={`Fixed by this repair (${diff.attribution.eliminated.length})`}>
          <ul className="ttRepairFindings">
            {diff.attribution.eliminated.map((finding, at) => (
              <li key={`fixed-${at}`}>{finding.detail}</li>
            ))}
          </ul>
        </Panel>
      )}

      {diff.attribution.preExistingFrozen.length > 0 && (
        <Panel
          title={`Already present, outside the scope you chose (${diff.attribution.preExistingFrozen.length})`}
          note="Not caused by this repair, and not a reason to withhold it."
        >
          <ul className="ttRepairFindings">
            {diff.attribution.preExistingFrozen.map((finding, at) => (
              <li key={`pre-${at}`}>{finding.detail}</li>
            ))}
          </ul>
          <button type="button" className="linkish" onClick={onWiderScope}>
            Start a separate repair with a wider scope
          </button>
        </Panel>
      )}

      {changes.length > 0 && (
        <Panel title="Every change">
          <div className="ttRepairTableWrap">
            <table className="ttRepairChanges">
              <caption className="srOnly">
                Each lesson this repair would change, with where it was and where it would go
              </caption>
              <thead>
                <tr>
                  <th scope="col">Lesson</th>
                  <th scope="col">Change</th>
                  <th scope="col">Was</th>
                  <th scope="col">Would be</th>
                </tr>
              </thead>
              <tbody>
                {changes.map((change) => (
                  <tr
                    key={change.activityId}
                    className={`ttRepairChange ttRepairChange--${change.kind}`}
                  >
                    <th scope="row">
                      {change.lesson ?? change.activityId}
                      {change.isLocked && (
                        <span className="ttRepairLocked"> (locked)</span>
                      )}
                    </th>
                    {/* The change is text first, and the aria-label carries it in full so a
                        screen reader is never relying on the row's tint. */}
                    <td aria-label={describeChange(change.kind)}>
                      {describeChange(change.kind)}
                    </td>
                    <td>{change.before === null ? "—" : placeOf(change.before)}</td>
                    <td>{change.after === null ? "—" : placeOf(change.after)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Panel>
      )}

      {blockedInScope.length > 0 && (
        <Panel
          title={`This scope could not place these (${blockedInScope.length})`}
          note="The scope you chose may be the reason, rather than the timetable."
        >
          <ul className="ttRepairFindings">
            {blockedInScope.map((change) => (
              <li key={change.activityId}>
                {change.lesson ?? change.activityId}
                {change.cause !== null && ` — ${change.cause}`}
              </li>
            ))}
          </ul>
          <button type="button" className="linkish" onClick={onWiderScope}>
            Start a separate repair with a wider scope
          </button>
        </Panel>
      )}

      {outOfReach.length > 0 && (
        <Panel
          title={`Unscheduled, and outside this scope (${outOfReach.length})`}
          note="This repair never attempted these, so it says nothing about them."
        >
          <ul className="ttRepairFindings">
            {outOfReach.map((change) => (
              <li key={change.activityId}>{change.lesson ?? change.activityId}</li>
            ))}
          </ul>
        </Panel>
      )}

      {/* The existing wording, which never says "conflict-free": it says no CONFIRMED clash
          within the school solved. ADR 0032 rule 4. */}
      <UsabilityLine
        counts={diff.countsAfter}
        placed={diff.placedAfter}
        required={diff.required}
        schoolName={schoolName}
      />

      {!applied && (
        <Panel
          title="Keep this repair?"
          note="Applying makes a new timetable and leaves the one you repaired alone."
        >
          <label className="ttRepairName">
            <span>Name for the new timetable</span>
            <input
              type="text"
              value={name}
              onChange={(event) => onName(event.target.value)}
              disabled={busy}
            />
          </label>
          <div className="ttRepairActions">
            <button
              type="button"
              className="primary"
              disabled={busy || result.notApplyable !== null || name.trim() === ""}
              onClick={onApply}
            >
              Apply as a new timetable
            </button>
            <button type="button" disabled={busy} onClick={onDiscard}>
              Discard this repair
            </button>
          </div>
        </Panel>
      )}

      {applied && (
        <Banner tone="ok">
          <strong>Applied</strong>{" "}
          The repaired timetable was saved as “{name}”. The one you repaired is unchanged.
        </Banner>
      )}
    </>
  );
}

/** One change, in words. Never a colour alone. */
function describeChange(kind: RepairResult["diff"]["changes"][number]["kind"]): string {
  switch (kind) {
    case "moved":
      return "Moved";
    case "newlyScheduled":
      return "Newly scheduled";
    case "unscheduled":
      return "No longer scheduled";
    case "stillUnplaced":
      return "Still unscheduled";
    case "unchanged":
    default:
      return "Unchanged";
  }
}

/** One placement, as a reader sees it. */
function placeOf(place: { day: string; period: string; room: string | null }): string {
  const where = `${place.day} ${place.period}`;
  return place.room === null ? where : `${where}, ${place.room}`;
}

/** Whether two scopes name the same thing. */
function sameScope(a: RepairScope | undefined, b: RepairScope | undefined): boolean {
  if (a === undefined || b === undefined) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}
