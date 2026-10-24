/**
 * One timetable, open for editing.
 *
 * ## Nothing on this screen decides whether an edit is legal
 *
 * Not one line. Every verdict rendered here was produced by the constraint registry in Rust and
 * carries the rule's own sentence: `editing.candidates` is a `CandidateGrid` the backend probed
 * cell by cell, and a refused write comes back as `SetupError` with the rules' words in it. This
 * component computes no span, no clash, no availability and no `rowSpan`, and it never decides
 * which room a lesson may have — it offers the ones the loader resolved through
 * `RoomFact::is_candidate_for` and asks again when the choice changes.
 *
 * A second implementation of any of those is the architecture bug `CLAUDE.md` §2 names in the
 * strongest terms it contains, and an editor is exactly where a codebase grows one.
 *
 * ## The session is this screen's lifetime
 *
 * `edit_open` compiles the problem, loads the arrangement and takes the cross-session stamp —
 * one snapshot, from which the undo stack and every verdict are computed. So the session is
 * opened on mount and closed on unmount, and there is never a second one for the same screen.
 * Leaving the editor discards the undo stack, which is why the screen says so before it does.
 *
 * ## One state, returned by every write
 *
 * Each command answers with the whole `EditStateView` — grid, lessons, unplaced, findings,
 * score, lock count and history together. The screen therefore never patches a field, and
 * cannot end up rendering a lesson list from one moment and a score from another. `version` is
 * what the grid re-fetches on.
 *
 * ## Four refusals, four remedies
 *
 * | Refusal | Means | What this screen offers |
 * | --- | --- | --- |
 * | `locked` | the placement is pinned in this timetable | **Unlock it** |
 * | `conflict` | somebody changed *this* placement | reload |
 * | `arrangementChanged` | somebody changed *another* placement of this timetable | **Reopen** |
 * | `inputsChanged` | a scheduling input moved | **Reopen** |
 *
 * The last two are fatal to the session rather than to the edit: the compiled problem, the
 * verdicts already on screen and the undo stack were computed from one snapshot, so they are
 * stale together and a retry would be answering with the old one. Nothing was written in any of
 * the four, and every message says that first.
 *
 * ## Editing is in place, and the screen says so
 *
 * This does not fork a version. ADR 0042 §5 keeps the `GENERATION` scoring immutable precisely
 * so re-scoring can never misrepresent how a timetable was generated — a guarantee only needed
 * *because* the arrangement beneath it can change. **Duplicate** is the affordance that makes
 * that safe, so it is offered at the top of the screen with the sentence that explains it,
 * before the first drag rather than after it.
 *
 * ## Every answer carries the identity of the question
 *
 * An editor is a screen where the user changes their mind faster than a round trip completes, so
 * four of its refs exist only to stop an old answer landing on a new question:
 *
 * | Ref | Stops |
 * | --- | --- |
 * | `alive` | a state write after the screen has gone — the user left, or the shell remounted the area because the school changed |
 * | `asked` | lesson A's candidate panel landing after lesson B was picked up, or the previous room's panel landing after the room changed |
 * | `writing` | a double-click starting two writes, where the second would come back as a revision conflict and read as the user's fault |
 * | `live` | closing a session other than the one that is actually open |
 *
 * `busy` is **not** one of them. It is state, it drives the disabled attributes, and it takes
 * effect on the next render — which is one render too late to serialise anything. `writing` is
 * the mutex; `busy` is what the mutex looks like.
 *
 * ## The tray exists because an unplaced lesson has no cell
 *
 * An occurrence the timetable does not place is nowhere in the grid, so it cannot be picked up
 * from one. The tray is where it is picked up from, and it is the same pick-up: the grid tints
 * the same candidate cells whether the lesson came from a cell or from the tray.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  CandidateGrid,
  EditStateView,
  ProjectionScope,
  ScopeKind,
} from "@evara/domain";

import {
  candidatesFor,
  closeEditing,
  describeEditError,
  duplicateTimetable,
  editScope,
  editState,
  isArrangementChanged,
  isLocked,
  openForEditing,
  redoEdit,
  setLessonLock,
  undoEdit,
  unscheduleLesson,
  applyMove,
} from "../../ipc/edit";
import { isInputsChanged } from "../../ipc/timetables";
import { Banner, Loading } from "../ui/Shell";
import { FindingsPanel, UsabilityLine } from "./Disclosure";
import { DayStrip, ScopePicker, WHOLE_CYCLE_LIMIT, useScopeLoad } from "./ScopeView";
import { TimetableGrid, type Editing } from "./TimetableGrid";

/**
 * A refusal that ended the session rather than the edit.
 *
 * Held separately from the ordinary error so the screen can stop offering writes: with a stale
 * snapshot every tinted cell on screen is a claim about a timetable that has moved, and the only
 * honest action is to reopen.
 */
type Stale = { message: string } | null;

/**
 * An edit refused because the placement is locked, and the occurrence it was about.
 *
 * Held with its activity so the remedy can be offered rather than described. A lock is a decision
 * the user already recorded, so the answer to *"this is locked"* is **Unlock it**, not a
 * suggestion to go and find the lesson again.
 */
type LockedRefusal = { activityId: string; message: string } | null;

export function EditTimetable(props: {
  /** The stored timetable to edit. */
  id: string;
  schoolName: string;
  /** Back to the read-only view of the same timetable. */
  onDone: () => void;
  /** The list behind this screen has something new to show — a duplicate, or an edit. */
  onChanged: () => void;
}) {
  const { id, schoolName, onDone, onChanged } = props;

  const [state, setState] = useState<EditStateView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stale, setStale] = useState<Stale>(null);
  const [lockedBy, setLockedBy] = useState<LockedRefusal>(null);
  const [busy, setBusy] = useState(false);
  const [opening, setOpening] = useState(true);

  // What is in hand, and the verdicts for it. `candidates` is the backend's answer and is
  // replaced wholesale; it is never edited in place, because a half-updated verdict grid would
  // tint a cell from one question with the answer to another.
  const [pickedUp, setPickedUp] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<CandidateGrid | null>(null);
  const [roomId, setRoomId] = useState<string | null>(null);
  /**
   * The rooms this lesson's class may use, held apart from the verdicts.
   *
   * Two different lifetimes, which is why they are two pieces of state. The room **domain** is a
   * property of the class — `RoomFact::is_candidate_for`, resolved by the loader — and does not
   * change when the user picks a different room. The **verdicts** are per room and are replaced
   * on every change, which is the whole point of re-asking.
   *
   * Reading the domain off `candidates` conflated them, and the symptom was concrete: changing
   * the room cleared `candidates`, which unmounted the room select, which meant the user could
   * not change the room again until the answer came back. The audit's own test is what found it.
   */
  const [rooms, setRooms] = useState<CandidateGrid["rooms"]>([]);
  const [roomless, setRoomless] = useState(false);

  const [scopeKind, setScopeKind] = useState<ScopeKind>("master");
  const [scopeId, setScopeId] = useState<string | null>(null);
  const [shownDay, setShownDay] = useState<number | null>(null);

  const [duplicating, setDuplicating] = useState(false);
  const [duplicateName, setDuplicateName] = useState("");
  const [duplicated, setDuplicated] = useState<string | null>(null);

  // The session the cleanup must close. A ref rather than the state, so unmount closes the
  // session that is actually open without the effect re-running on every edit.
  const live = useRef<string | null>(null);

  /**
   * Whether this screen is still mounted.
   *
   * Every `await` here is a window in which the editor can go away — the user leaves, or the
   * shell remounts the whole area because the campus or the school changed (`TimetableArea`'s
   * `key={school.id}`). An answer that lands after that must not be written into state: at best
   * it is ignored, at worst it is a verdict about a timetable the user is no longer looking at.
   */
  const alive = useRef(true);

  /**
   * Which candidate panel has been asked for, as a monotonic ticket.
   *
   * Picking lesson B up while lesson A's panel is still in flight, and changing the room while
   * the previous room's panel is in flight, both end with **two** answers arriving in an order
   * the network chose. Only the newest question's answer may land; an older one describes cells
   * for a lesson or a room nobody is asking about, and rendering it would tint the wrong cells
   * with a verdict Rust gave about something else.
   *
   * A counter rather than comparing `CandidateGrid.activityId`: the room case has the same
   * activity in both answers, so the identity has to be the *request's*, not the lesson's.
   */
  const asked = useRef(0);

  /**
   * Whether a write is in flight.
   *
   * `busy` is state and drives the disabled attributes; this is the actual mutex. They are not
   * the same thing: `setBusy(true)` takes effect on the next render, so two clicks dispatched
   * before it lands would both pass a `busy` check and start two writes. The database would
   * refuse the second on its revision, which is the backstop working — but the user would see a
   * conflict for having clicked twice, which is not an honest message.
   */
  const writing = useRef(false);

  const open = useCallback(async () => {
    setOpening(true);
    setError(null);
    setStale(null);
    setLockedBy(null);
    setPickedUp(null);
    setCandidates(null);
    try {
      const previous = live.current;
      const next = await openForEditing(id);
      if (!alive.current) {
        // The screen went away while the session was opening. Close what was just opened
        // rather than leaking it: the cleanup below has already run and had nothing to close.
        await closeEditing(next.sessionId).catch(() => undefined);
        return;
      }
      live.current = next.sessionId;
      setState(next);
      setDuplicateName(`${next.timetable.name} (copy)`);
      // Closed after the new one is open, so a failure leaves the screen with a session rather
      // than with neither.
      if (previous !== null && previous !== next.sessionId) {
        await closeEditing(previous).catch(() => undefined);
      }
    } catch (e: unknown) {
      if (!alive.current) return;
      setState(null);
      setError(describeEditError(e));
    } finally {
      if (alive.current) setOpening(false);
    }
  }, [id]);

  useEffect(() => {
    void open();
  }, [open]);

  useEffect(
    () => () => {
      alive.current = false;
      // Any candidate answer still in flight is now about a question nobody is asking.
      asked.current += 1;
      const session = live.current;
      live.current = null;
      // The session holds the compiled problem and the undo stack. Nothing is written by
      // closing it, and a close that arrives after a campus switch has nothing to act on — so a
      // failure here is not worth a banner on a screen that is going away.
      if (session !== null) void closeEditing(session).catch(() => undefined);
    },
    [],
  );

  const sessionId = state?.sessionId ?? null;
  const version = state?.version ?? 0;
  const projection = state?.projection ?? null;
  const scopes = useMemo<ProjectionScope[]>(() => projection?.scopes ?? [], [projection]);

  // The layout is Rust's, through the same `evara-report` call the read-only view uses, so an
  // edited grid and a saved one cannot differ because a different code path drew them. The
  // loader's identity changes with `version`, which is what re-fetches the grid after an edit.
  const loadScope = useMemo(
    () =>
      sessionId === null
        ? null
        : (kind: ScopeKind, chosen: string | null) => editScope(sessionId, kind, chosen),
    // `version` is a dependency and not an argument: the command needs only the session, but a
    // new identity for this function is what makes `useScopeLoad` ask again after an edit.
    [sessionId, version],
  );
  const { grid, error: gridError } = useScopeLoad(loadScope, scopeKind, scopeId);

  /** The entry revision the interface is holding for one occurrence, or `null` when unplaced. */
  const revOf = useCallback(
    (activityId: string): number | null =>
      state?.lessons.find((lesson) => lesson.activityId === activityId)?.rev ?? null,
    [state],
  );

  /**
   * Takes one refusal and decides which of the four it is.
   *
   * The one place that decision is made. `arrangementChanged` and `inputsChanged` are stale
   * snapshots and end the session; `locked` is a decision the user can reverse here; everything
   * else is an edit that did not happen and a message that says why.
   */
  const refused = useCallback((e: unknown, activityId: string | null) => {
    const message = describeEditError(e);
    if (isArrangementChanged(e) || isInputsChanged(e)) {
      // The ticket is bumped as well as the state cleared: a candidate panel still in flight
      // was computed from the snapshot that has just been declared stale, and letting it land
      // would re-tint the grid under the banner saying not to trust it.
      asked.current += 1;
      setStale({ message });
      setPickedUp(null);
      setCandidates(null);
      setRooms([]);
      setRoomless(false);
      return;
    }
    if (isLocked(e) && activityId !== null) {
      setLockedBy({ activityId, message });
      return;
    }
    setError(message);
  }, []);

  /** Accepts the state a write returned, and puts down whatever was in hand. */
  const accept = useCallback(
    (next: EditStateView) => {
      setState(next);
      setError(null);
      setLockedBy(null);
      setPickedUp(null);
      setCandidates(null);
      setRoomId(null);
      setRooms([]);
      setRoomless(false);
      onChanged();
    },
    [onChanged],
  );

  const onPickUp = useCallback(
    (activityId: string | null) => {
      if (sessionId === null || stale !== null) return;
      // Every pick-up and every put-down invalidates whatever was asked before it.
      const ticket = (asked.current += 1);
      if (activityId === null) {
        setPickedUp(null);
        setCandidates(null);
        setRoomId(null);
        setRooms([]);
        setRoomless(false);
        return;
      }
      // The room a placed lesson already has is the room the verdicts are asked with, so
      // picking a lesson up and dropping it one period later is a move and not also a room
      // change. An unplaced one starts with no room, and the picker is how it gets one.
      const current =
        projection?.lessons.find((lesson) => lesson.activityId === activityId)?.roomId ?? null;
      setPickedUp(activityId);
      setRoomId(current);
      setCandidates(null);
      setError(null);
      candidatesFor(sessionId, activityId, current)
        .then((found) => {
          if (!alive.current || asked.current !== ticket) return;
          setCandidates(found);
          setRooms(found.rooms);
          setRoomless(found.roomlessAllowed);
        })
        .catch((e: unknown) => {
          if (!alive.current || asked.current !== ticket) return;
          refused(e, activityId);
        });
    },
    [sessionId, stale, projection, refused],
  );

  /** Re-asks the whole panel with a different room. A room is part of the proposal, not a hint. */
  const onChangeRoom = useCallback(
    (next: string | null) => {
      if (sessionId === null || pickedUp === null) return;
      const ticket = (asked.current += 1);
      setRoomId(next);
      setCandidates(null);
      candidatesFor(sessionId, pickedUp, next)
        .then((found) => {
          // The same activity in both answers, so only the ticket tells them apart.
          if (!alive.current || asked.current !== ticket) return;
          setCandidates(found);
          // The domain is re-recorded rather than assumed unchanged: it is the backend's answer
          // either way, and asserting it cannot have changed would be this layer deciding.
          setRooms(found.rooms);
          setRoomless(found.roomlessAllowed);
        })
        .catch((e: unknown) => {
          if (!alive.current || asked.current !== ticket) return;
          refused(e, pickedUp);
        });
    },
    [sessionId, pickedUp, refused],
  );

  /**
   * One write, with the busy flag and the refusal handling every one of them shares.
   *
   * The occurrence is passed in rather than inferred, because a refusal's remedy depends on which
   * lesson it was about — a locked one is offered its own **Unlock**.
   */
  const write = useCallback(
    async (activityId: string | null, run: (session: string) => Promise<EditStateView>) => {
      if (sessionId === null || stale !== null) return;
      // A second click is not a second edit. Without this the database refuses the second write
      // on its revision — the backstop working — and the user is shown a conflict for having
      // double-clicked, which blames them for something the screen should have absorbed.
      if (writing.current) return;
      writing.current = true;
      setBusy(true);
      try {
        const next = await run(sessionId);
        if (!alive.current) return;
        accept(next);
      } catch (e: unknown) {
        if (!alive.current) return;
        refused(e, activityId);
      } finally {
        writing.current = false;
        if (alive.current) setBusy(false);
      }
    },
    [sessionId, stale, accept, refused],
  );

  const onDrop = useCallback(
    (timeslotId: string) => {
      const activityId = pickedUp;
      if (activityId === null) return;
      void write(activityId, (session) =>
        applyMove(session, { activityId, timeslotId, roomId }, revOf(activityId)),
      );
    },
    [pickedUp, roomId, revOf, write],
  );

  const onUnschedule = useCallback(
    (activityId: string) => {
      const rev = revOf(activityId);
      // An unplaced occurrence has no row to take off the timetable. The grid cannot offer this
      // — it has no cell for one — but the guard is here because the tray does.
      if (rev === null) return;
      void write(activityId, (session) => unscheduleLesson(session, activityId, rev));
    },
    [revOf, write],
  );

  const onToggleLock = useCallback(
    (activityId: string) => {
      const entry = state?.lessons.find((lesson) => lesson.activityId === activityId);
      if (entry === undefined) return;
      void write(activityId, (session) =>
        setLessonLock(session, activityId, !entry.isLocked, entry.rev),
      );
    },
    [state, write],
  );

  const onDuplicate = useCallback(async () => {
    if (state === null) return;
    if (writing.current) return;
    writing.current = true;
    setBusy(true);
    setError(null);
    try {
      const copy = await duplicateTimetable(
        state.timetable.id,
        state.timetable.rev,
        duplicateName,
      );
      if (!alive.current) return;
      setDuplicated(copy.name);
      setDuplicating(false);
      onChanged();
    } catch (e: unknown) {
      if (alive.current) setError(describeEditError(e));
    } finally {
      writing.current = false;
      if (alive.current) setBusy(false);
    }
  }, [state, duplicateName, onChanged]);

  /**
   * Undo and redo.
   *
   * A refused step clears **both** stacks in the session — everything below the one that failed
   * was computed against a state that no longer holds — so the state is re-read rather than
   * patched, and the screen's history buttons tell the truth about what is left.
   */
  const step = useCallback(
    async (direction: "undo" | "redo") => {
      if (sessionId === null || stale !== null) return;
      if (writing.current) return;
      writing.current = true;
      setBusy(true);
      try {
        const next = await (direction === "undo" ? undoEdit(sessionId) : redoEdit(sessionId));
        if (!alive.current) return;
        accept(next);
      } catch (e: unknown) {
        if (!alive.current) return;
        refused(e, null);
        // The session cleared **both** stacks, so what is on screen about undo and redo is
        // wrong. Re-read rather than patch: the session is the only thing that knows.
        await editState(sessionId)
          .then((fresh) => {
            if (alive.current) setState(fresh);
          })
          .catch(() => undefined);
      } finally {
        writing.current = false;
        if (alive.current) setBusy(false);
      }
    },
    [sessionId, stale, accept, refused],
  );

  if (state === null) {
    return (
      <div className="ttScreen">
        <button type="button" className="linkish" onClick={onDone}>
          ← Back to this timetable
        </button>
        {opening ? (
          <Loading what="this timetable for editing" />
        ) : (
          <Banner tone="error">
            {error ??
              "This timetable could not be opened for editing."} There is no partial editing
            mode: an editor with no compiled problem could check nothing, and offering one would
            be offering to write placements nothing had validated.
          </Banner>
        )}
      </div>
    );
  }

  const editing: Editing = {
    pickedUp,
    candidates,
    busy: busy || stale !== null,
    onPickUp,
    onDrop,
    onUnschedule,
    onToggleLock,
  };

  const held =
    pickedUp === null
      ? null
      : (projection?.lessons.find((lesson) => lesson.activityId === pickedUp) ?? null);
  const heldUnplaced =
    pickedUp === null
      ? null
      : (state.unplaced.find((one) => one.activityId === pickedUp) ?? null);

  return (
    <div className="ttScreen">
      <button type="button" className="linkish" onClick={onDone}>
        ← Back to this timetable
      </button>

      <div className="ttOpenHead">
        <h2>Editing {state.timetable.name}</h2>
        <p className="ttOpenMeta">
          Changes are written as you make them, to this timetable. Leaving this screen discards
          the undo history — not the edits.
        </p>
      </div>

      {stale === null ? null : (
        <Banner tone="error">
          {stale.message} Nothing was written. Everything this screen was showing you — the
          legal drop targets, the findings and the undo history — was worked out from the
          timetable as it stood when you opened it, so it has to be read again.{" "}
          <button type="button" className="linkish" disabled={opening} onClick={() => void open()}>
            Reopen it
          </button>
          .
        </Banner>
      )}

      {lockedBy === null ? null : (
        <Banner tone="warning">
          {lockedBy.message} Nothing was written. A lock is a decision you recorded, so this is
          refused before any rule is consulted — unlocking it is not overriding a clash.{" "}
          <button
            type="button"
            className="linkish"
            disabled={busy}
            onClick={() => onToggleLock(lockedBy.activityId)}
          >
            Unlock it
          </button>
          , then try the edit again.
        </Banner>
      )}

      {error === null ? null : <Banner tone="error">{error}</Banner>}

      {duplicated === null ? null : (
        <Banner tone="ok">
          Copied to <strong>{duplicated}</strong>. That copy claims no generation run and no
          generation score, because it was not generated — it is a copy of this arrangement.
        </Banner>
      )}

      <section className="ttPanel ttPanel--keep" aria-labelledby="ttKeep">
        <h3 id="ttKeep">Keeping the generated arrangement</h3>
        <p className="ttSaveNote">
          Editing changes <strong>this</strong> timetable. Nothing here makes a new version, and
          there is no undo once you leave. If you want to keep the arrangement as it was
          generated, copy it first — every timetable a school holds sits alongside the others,
          so a copy costs nothing but a name.
        </p>
        {duplicating ? (
          <div className="ttRenameRow">
            <label htmlFor="ttDupName">Name for the copy</label>
            <input
              id="ttDupName"
              type="text"
              value={duplicateName}
              onChange={(event) => setDuplicateName(event.target.value)}
              disabled={busy}
            />
            <button
              type="button"
              className="primary"
              disabled={busy || duplicateName.trim() === ""}
              onClick={() => void onDuplicate()}
            >
              Make the copy
            </button>
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => setDuplicating(false)}
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="secondary"
            disabled={busy}
            onClick={() => setDuplicating(true)}
          >
            Copy this timetable first
          </button>
        )}
      </section>

      <div className="ttEditBar" role="group" aria-label="Editing">
        <button
          type="button"
          className="secondary"
          disabled={busy || stale !== null || state.history.undo === null}
          onClick={() => void step("undo")}
        >
          Undo
        </button>
        <button
          type="button"
          className="secondary"
          disabled={busy || stale !== null || state.history.redo === null}
          onClick={() => void step("redo")}
        >
          Redo
        </button>
        <span className="ttEditHistory">
          {state.history.undo === null
            ? "Nothing to undo yet."
            : `Undo: ${state.history.undo}.`}
          {state.history.redo === null ? "" : ` Redo: ${state.history.redo}.`}
        </span>
        <span className="ttEditCounts">
          {state.locked} locked · {state.unplaced.length} unplaced · {state.counts.hard} hard
        </span>
      </div>

      {pickedUp === null ? (
        <p className="ttEditHint" role="status">
          Pick a lesson up to see where it can go. Drag it, or move to its cell and press Enter.
          Every legal cell is marked, and every cell that refuses it says which rule refused.
        </p>
      ) : (
        <PickedUp
          label={
            held?.label?.short ??
            held?.activityName ??
            heldUnplaced?.label ??
            "the lesson in hand"
          }
          candidates={candidates}
          rooms={rooms}
          roomlessAllowed={roomless}
          roomId={roomId}
          busy={busy}
          onChangeRoom={onChangeRoom}
          onPutDown={() => onPickUp(null)}
        />
      )}

      <UsabilityLine
        counts={state.counts}
        placed={state.lessons.length}
        required={state.lessons.length + state.unplaced.length}
        schoolName={schoolName}
      />

      {state.notInProblem.length === 0 ? null : (
        <section className="ttPanel ttPanel--moved" aria-labelledby="ttEditDropped">
          <h3 id="ttEditDropped">This timetable is readable but incomplete</h3>
          <p>
            {state.notInProblem.length} placed lesson
            {state.notInProblem.length === 1 ? "" : "s"} name something the campus file no longer
            holds, so {state.notInProblem.length === 1 ? "it is" : "they are"} not in the grid
            below and cannot be edited here. What you are looking at is less than what was saved.
            {state.pinsNotInProblem.length === 0
              ? ""
              : ` ${state.pinsNotInProblem.length} of them ${
                  state.pinsNotInProblem.length === 1 ? "is" : "are"
                } locked, so a later repair cannot be told to preserve ${
                  state.pinsNotInProblem.length === 1 ? "it" : "them"
                }.`}
          </p>
        </section>
      )}

      <Tray
        unplaced={state.unplaced}
        pickedUp={pickedUp}
        busy={busy || stale !== null}
        onPickUp={onPickUp}
      />

      <section className="ttPanel" aria-labelledby="ttEditGrid">
        <h3 id="ttEditGrid">Timetable</h3>
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
        {(projection?.days.length ?? 0) <= WHOLE_CYCLE_LIMIT ? null : (
          <DayStrip days={projection?.days ?? []} shown={shownDay} onChange={setShownDay} />
        )}
        {gridError === null ? null : <Banner tone="error">{gridError}</Banner>}
        {grid === null || projection === null ? (
          <Loading what="this grid" />
        ) : (
          <TimetableGrid
            grid={grid}
            lessons={projection.lessons}
            shownDay={shownDay}
            editing={editing}
          />
        )}
      </section>

      {/*
       * The findings below describe the arrangement **as it now stands**, after this session's
       * edits. They are not the generation record, which ADR 0042 §5 makes immutable precisely
       * so re-scoring can never misrepresent how a timetable was generated — and which this
       * screen therefore does not show rather than showing a second score beside it that a
       * reader could mistake for the first. The sentence is here because a panel headed
       * "Findings" with no date on it is exactly what gets mistaken for the generation score.
       */}
      <p className="ttEditHint">
        Everything below is about this arrangement as it stands now, including the edits you have
        just made. How this timetable scored <em>when it was generated</em> is a separate,
        unchangeable record — it is on the timetable's own screen, under “Scoring”, and nothing
        you do here alters it.
      </p>
      <FindingsPanel findings={state.findings} counts={state.counts} />
    </div>
  );
}

/**
 * What is in hand, where it may go, and which room it would take.
 *
 * ## The room is part of the proposal, and outlives the answer
 *
 * A drop's legality depends on the room — `room-clash`, and the room candidate domain itself —
 * so changing the room re-asks the backend for the whole panel rather than re-tinting what is on
 * screen. The rooms offered come from `CandidateGrid.rooms`, which is the loader's record of
 * `RoomFact::is_candidate_for`; this picker invents none and filters none.
 *
 * They arrive as their **own** prop rather than read off `candidates`, because the two have
 * different lifetimes: the domain belongs to the lesson's class and the verdicts belong to one
 * room. Reading both off the same object meant a room change blanked the picker until the answer
 * returned, so the user could not change their mind twice.
 *
 * *No room* is offered only when `roomlessAllowed`, which is the backend's answer about this
 * occurrence's class and not a guess about whether a lesson needs a room.
 */
function PickedUp(props: {
  label: string;
  candidates: CandidateGrid | null;
  rooms: CandidateGrid["rooms"];
  roomlessAllowed: boolean;
  roomId: string | null;
  busy: boolean;
  onChangeRoom: (roomId: string | null) => void;
  onPutDown: () => void;
}) {
  const { label, candidates, rooms, roomlessAllowed, roomId, busy, onChangeRoom, onPutDown } =
    props;

  const legal = candidates?.cells.filter((cell) => cell.allowed).length ?? 0;
  const total = candidates?.cells.length ?? 0;

  return (
    <section className="ttHeld" aria-labelledby="ttHeldHead" role="status">
      <h3 id="ttHeldHead">
        <strong>{label}</strong> is in hand
      </h3>

      {candidates === null ? (
        <p className="ttEditHint">Working out where it can go…</p>
      ) : candidates.unavailable !== null ? (
        <p className="ttBlocking">{candidates.unavailable}</p>
      ) : (
        <p className="ttEditHint">
          {legal} of {total} cell{total === 1 ? "" : "s"} on its bell schedule will take it. The
          rest say which rule refuses them. Press Escape, or use Put down, to change nothing.
        </p>
      )}

      {rooms.length === 0 && !roomlessAllowed ? (
        candidates === null ? null : (
          <p className="ttEditHint">
            No room is available to this lesson's class, so it is placed without one.
          </p>
        )
      ) : (
        <label className="ttScopeChoose">
          <span>Room</span>
          <select
            value={roomId ?? ""}
            disabled={busy}
            onChange={(event) => onChangeRoom(event.target.value || null)}
          >
            {roomlessAllowed ? <option value="">No room</option> : null}
            {rooms.map((room) => (
              <option key={room.roomId} value={room.roomId}>
                {room.name}
              </option>
            ))}
          </select>
        </label>
      )}

      {/*
       * Named for the lesson rather than left as a bare "Put down". The cell the lesson came from
       * carries one too, so without this there are two identically-named buttons on the page and
       * somebody listening to it cannot tell which they have landed on.
       */}
      <button
        type="button"
        className="secondary"
        disabled={busy}
        aria-label={`Put down ${label}`}
        onClick={onPutDown}
      >
        Put down
      </button>
    </section>
  );
}

/**
 * The occurrences this timetable does not place.
 *
 * An unplaced occurrence is in no cell, so the grid cannot offer it — this is the only place it
 * can be picked up from. Picking one up here is the same pick-up a cell's button makes: the same
 * candidate panel is asked for, and the same cells are tinted.
 *
 * It says what it is rather than whether it is placeable. Only a solver can say a timetable
 * exists, and `CLAUDE.md` §3 forbids this screen from suggesting otherwise in either direction.
 */
function Tray(props: {
  unplaced: EditStateView["unplaced"];
  pickedUp: string | null;
  busy: boolean;
  onPickUp: (activityId: string | null) => void;
}) {
  const { unplaced, pickedUp, busy, onPickUp } = props;

  if (unplaced.length === 0) {
    return (
      <section className="ttPanel ttPanel--tray" aria-labelledby="ttTray">
        <h3 id="ttTray">Unplaced lessons</h3>
        <p className="ttDiscloseNone">
          Every occurrence this timetable covers is on the grid.
        </p>
      </section>
    );
  }

  return (
    <section className="ttPanel ttPanel--tray" aria-labelledby="ttTray">
      <h3 id="ttTray">Unplaced lessons</h3>
      <p className="ttUnplacedNote">
        {unplaced.length} occurrence{unplaced.length === 1 ? "" : "s"} {" "}
        {unplaced.length === 1 ? "is" : "are"} not on the grid. Pick one up to see where it
        could go — an empty tray is what a complete timetable looks like, and a full one is not
        a statement that these cannot be placed.
      </p>
      <ul className="ttTrayList">
        {unplaced.map((one) => (
          <li
            key={one.activityId}
            className={`ttTrayItem${pickedUp === one.activityId ? " ttTrayItem--held" : ""}`}
            draggable={!busy}
            onDragStart={() => onPickUp(one.activityId)}
            onDragEnd={() => onPickUp(null)}
          >
            <span className="ttLessonName">{one.label}</span>
            <span className="ttLessonTime">
              {one.durationPeriods} period{one.durationPeriods === 1 ? "" : "s"} · occurrence{" "}
              {one.splitIndex + 1}
            </span>
            <button
              type="button"
              className="linkish"
              disabled={busy}
              aria-pressed={pickedUp === one.activityId}
              onClick={() => onPickUp(pickedUp === one.activityId ? null : one.activityId)}
            >
              {pickedUp === one.activityId ? "Put down" : "Pick up"}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
