/**
 * One transfer session's lifecycle, for both directions.
 *
 * ## Why one hook and not two screens' worth of state
 *
 * An export and an import are the same shape on this side of the boundary: start, get a
 * `runId`, watch `transfer://progress`, settle on `transfer://finished`, read the terminal
 * report back with a status call, discard the session. Only the three commands differ, so the
 * hook takes them as an {@link SessionCommands} adapter and holds the state machine once.
 *
 * Two screens holding this separately is the duplication `CLAUDE.md` §2 is about: the
 * interesting parts here — the start race, the epoch pin, the discard — would then have two
 * implementations, and the one that was wrong would be the one nobody looked at.
 *
 * ## The start race, which is real and is not theoretical
 *
 * `project_export_start` returns **after** the snapshot is taken and the worker is running.
 * Subscribing happens in an effect, which React runs after the commit — so a small campus can
 * finish before the listener exists, and `transfer://finished` would be delivered to nobody.
 * So the subscribe effect **also polls the status once**, and settles from it if the session
 * is already terminal. Without that, a fast export leaves a progress bar on screen for ever.
 *
 * ## The epoch pin
 *
 * Both events carry `runId` **and** `epoch`. The IPC wrapper filters on the run; the epoch is
 * pinned here, to the first event seen for this run, and a later event bearing a different one
 * is dropped. No session can legitimately span two epochs — the backend clears its registry
 * when a campus file is created, opened or closed — so a disagreement means the event belongs
 * to a world this screen is no longer in.
 *
 * The epoch is deliberately not fetched from the backend to compare against: there is no
 * command for it, and inventing one would be a second source of a number the events already
 * carry.
 *
 * ## What it never does
 *
 * It forms no opinion about an archive, decides no refusal and rewrites no message. `phase`,
 * `step`, the report and the failure all come from the backend; this holds them and tells a
 * component when they changed.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { ProjectError, TransferPhase, TransferStep } from "@evara/domain";

import { onTransferFinished, onTransferProgress } from "../../ipc/transfer";

/**
 * The fields of `ExportStatus` and `ImportStatus` this hook reads.
 *
 * Both generated types are assignable to it, which is what lets one hook serve both without
 * either type being widened or restated.
 */
export interface TransferStatusLike<R> {
  readonly phase: TransferPhase;
  readonly step: TransferStep;
  readonly done: number;
  readonly total: number;
  readonly cancelRequested: boolean;
  readonly report: R | null;
  readonly failure: ProjectError | null;
}

/** The three commands a session needs, so the hook names none of them itself. */
export interface SessionCommands<R> {
  readonly status: (runId: string) => Promise<TransferStatusLike<R>>;
  readonly cancel: (runId: string) => Promise<boolean>;
  readonly discard: (runId: string) => Promise<void>;
}

/** What a session started with. */
export interface Started {
  readonly runId: string;
  readonly totalBytes: number;
}

/** Everything a screen renders from. */
export interface TransferSession<R> {
  /** The live session, or `null` before one has been started. */
  readonly runId: string | null;
  /** The backend's lifecycle phase, or `null` before a start. */
  readonly phase: TransferPhase | null;
  /** Which part of the work is running. */
  readonly step: TransferStep | null;
  /** How far that part has got. */
  readonly done: number;
  /** How much there is, or `0` when it is not yet known. Never divide by it. */
  readonly total: number;
  /** A cancel has been asked for and the worker has not stopped yet. */
  readonly cancelling: boolean;
  /** `true` between the click and the backend accepting the start. */
  readonly starting: boolean;
  /** The terminal report, once there is one. */
  readonly report: R | null;
  /** The backend's failure, once there is one. */
  readonly failure: ProjectError | null;
  /** Whether the session is running — the one condition that blocks a second start. */
  readonly running: boolean;
  /** Starts one. Rejects with the backend's error so a caller can branch on it. */
  readonly begin: (start: () => Promise<Started>) => Promise<void>;
  /** Asks the worker to stop. Idempotent from the user's point of view. */
  readonly requestCancel: () => Promise<void>;
  /** Forgets the session in the backend and clears this state. */
  readonly reset: () => Promise<void>;
}

/** Whether a phase means the session is over. */
export function isTerminal(phase: TransferPhase | null): boolean {
  return phase !== null && phase !== "running";
}

export function useTransferSession<R>(commands: SessionCommands<R>): TransferSession<R> {
  const [runId, setRunId] = useState<string | null>(null);
  const [phase, setPhase] = useState<TransferPhase | null>(null);
  const [step, setStep] = useState<TransferStep | null>(null);
  const [done, setDone] = useState(0);
  const [total, setTotal] = useState(0);
  const [cancelling, setCancelling] = useState(false);
  const [starting, setStarting] = useState(false);
  const [report, setReport] = useState<R | null>(null);
  const [failure, setFailure] = useState<ProjectError | null>(null);

  // The adapter is a fresh object on every render; the hook only ever needs the latest one,
  // and depending on it would resubscribe the listeners on each render.
  const api = useRef(commands);
  api.current = commands;

  // What unmount has to clean up, read from a ref so the cleanup does not depend on state it
  // would otherwise have to list.
  const live = useRef<{ runId: string | null; phase: TransferPhase | null }>({
    runId: null,
    phase: null,
  });
  live.current = { runId, phase };

  const settle = useCallback((status: TransferStatusLike<R>) => {
    setPhase(status.phase);
    setStep(status.step);
    setDone(status.done);
    setTotal(status.total);
    setCancelling(status.cancelRequested && status.phase === "running");
    setReport(status.report);
    setFailure(status.failure);
  }, []);

  // Progress and completion, filtered to this run inside the IPC wrapper and to this epoch
  // here. One effect, torn down whenever the run changes or the component goes away.
  useEffect(() => {
    if (runId === null) return;
    let alive = true;
    let epoch: number | null = null;
    const offs: Array<() => void> = [];

    /** Drops an event from a world this screen has left. See the module docs. */
    const sameEpoch = (seen: number) => {
      epoch ??= seen;
      return epoch === seen;
    };

    const read = () => {
      void api.current
        .status(runId)
        .then((status) => {
          if (alive) settle(status);
        })
        .catch(() => {
          // A status call that fails on a session the backend has forgotten is not worth
          // showing: the finished event has already said what happened, and the screen's
          // own error line is for the user's action, not for this.
        });
    };

    const subscribe = async () => {
      const progress = await onTransferProgress(runId, (event) => {
        if (!alive || !sameEpoch(event.epoch)) return;
        setStep(event.step);
        setDone(event.done);
        setTotal(event.total);
      });
      const finished = await onTransferFinished(runId, (event) => {
        if (!alive || !sameEpoch(event.epoch)) return;
        // The event's phase first, so cancelled and failed are distinguished the instant
        // the worker returns; the status call then supplies the report or the failure.
        setPhase(event.phase);
        setCancelling(false);
        read();
      });
      if (!alive) {
        progress();
        finished();
        return;
      }
      offs.push(progress, finished);
      // The start race: by the time this runs the worker may already have finished, and
      // nothing was listening when it did.
      read();
    };

    void subscribe();
    return () => {
      alive = false;
      for (const off of offs) off();
      offs.length = 0;
    };
  }, [runId, settle]);

  // A session the user is finished with is forgotten rather than left in the backend's
  // registry. Only a terminal one: a running transfer owns a worker thread and its own
  // cancellation, and discarding it from an unmount would stop work the user did not stop.
  useEffect(
    () => () => {
      const { runId: id, phase: at } = live.current;
      if (id !== null && isTerminal(at)) void api.current.discard(id).catch(() => undefined);
    },
    [],
  );

  const begin = useCallback(
    async (start: () => Promise<Started>) => {
      // Two clicks on one button, or a second click while the first start is in flight.
      // The backend claims its own session too, but a screen that sent two starts would
      // have two run ids and show the wrong one.
      if (starting || phase === "running") return;
      setStarting(true);
      setPhase(null);
      setStep(null);
      setDone(0);
      setTotal(0);
      setCancelling(false);
      setReport(null);
      setFailure(null);
      try {
        const started = await start();
        setRunId(started.runId);
        setTotal(started.totalBytes);
        setPhase("running");
      } finally {
        setStarting(false);
      }
    },
    [starting, phase],
  );

  const requestCancel = useCallback(async () => {
    if (runId === null || phase !== "running") return;
    setCancelling(true);
    // The answer is whether anything was still running. `false` is not an error — there was
    // nothing to stop — and the finished event is what moves the screen on either way.
    await api.current.cancel(runId).catch(() => undefined);
  }, [runId, phase]);

  const reset = useCallback(async () => {
    const id = runId;
    setRunId(null);
    setPhase(null);
    setStep(null);
    setDone(0);
    setTotal(0);
    setCancelling(false);
    setReport(null);
    setFailure(null);
    if (id !== null) await api.current.discard(id).catch(() => undefined);
  }, [runId]);

  return {
    runId,
    phase,
    step,
    done,
    total,
    cancelling,
    starting,
    report,
    failure,
    running: phase === "running",
    begin,
    requestCancel,
    reset,
  };
}
