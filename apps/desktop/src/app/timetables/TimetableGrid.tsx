/**
 * One scope's timetable, as a real table.
 *
 * ## Every layout decision arrives already made
 *
 * This component renders a `ScopeGrid` the backend laid out. It does **not** decide which
 * slots a lesson occupies, which bell schedule it belongs to, whether two lessons in one cell
 * are a clash, or what `rowSpan` a cell may use. Each of those is a scheduling question with
 * exactly one implementation, in Rust, and answering any of them here would be the second
 * copy `CLAUDE.md` §2 forbids.
 *
 * What arrives is a complete rectangle of cells, one per `(day, period)`, each carrying a
 * tagged `CellContent`:
 *
 * | `content.kind` | Rendered as |
 * | --- | --- |
 * | `empty` | an empty `<td>` |
 * | `lesson` | one `<td rowSpan={content.rows}>` holding the lesson |
 * | `continuation` | **nothing** — the span above covers this position |
 * | `pinned` | one `<td>` at `rowSpan` 1, holding every lesson in `content.lessons` |
 *
 * `pinned` is the case that makes this safe. Two lessons in one cell — ordinary in a master
 * grid, a clash in a teacher's — and a run broken by an unmaterialised slot both produce it,
 * and in both cases **every lesson involved is still reachable**. Nothing is discarded to
 * make the table render.
 *
 * ## Why a real table
 *
 * `AvailabilityGrid`'s header says it, and it applies here with more force because a
 * timetable is read far more often than it is configured: *"A grid of `<div>`s with click
 * handlers would have been fewer lines and unusable by half the people who have to configure
 * a school."* A `<td rowSpan>` is also what makes a screen reader announce a double period
 * once rather than twice.
 *
 * ## One panel per bell schedule
 *
 * Two campuses on two bell schedules hold different slots over the same minutes, so their
 * period rows do not align. A scope touching both gets two panels and a sentence saying the
 * two cannot be read as one week — never a merged axis, which would assert that Northgate's
 * Period 3 and Southmoor's Period 3 are the same row. They are not.
 *
 * ## Editing, when it is offered, decides nothing
 *
 * The `editing` prop turns cells into pick-up and drop targets. Every verdict it renders arrives
 * already decided: `editing.candidates` is a `CandidateGrid` the backend probed cell by cell
 * through the constraint registry, so a tinted cell is one Rust called legal and a refused one
 * carries the rule's own sentence. This component chooses no target, computes no clash and
 * compares no interval.
 *
 * The keyboard path reaches **everything** the mouse does and is asserted to, because a grid that
 * can only be edited by dragging is a grid half the people who have to use it cannot edit:
 *
 * | Key | On a cell |
 * | --- | --- |
 * | Enter / Space | pick up the lesson here, or drop the one in hand |
 * | Escape | put down what is in hand, changing nothing |
 * | Delete / Backspace | unschedule the lesson here |
 * | `l` | lock or unlock the lesson here |
 * | arrows, Home, End | move the focus, as in the read-only grid |
 *
 * ## Showing one day is a display choice, and the only one this component makes
 *
 * `shownDay` renders a single column instead of the whole cycle, for a rotation too wide to
 * read. It is safe to do here, and is the one filtering decision that is: hiding a column
 * removes whole cells, never part of a span, so every row still renders the same number of
 * cells as there are visible days and the table stays well-formed. **Which** days exist, and
 * what occupies each, remain entirely the backend's answer.
 */
import { useCallback, useRef, useState } from "react";

import type {
  CandidateGrid,
  GridPanel,
  PinReason,
  ProjectedLesson,
  ScopeGrid,
  TimetableCell,
} from "@evara/domain";

import { verdictAt } from "../../ipc/edit";

/**
 * What the grid needs in order to be editable, and nothing more.
 *
 * Every field is either a fact the backend decided or a callback into it. There is no state here
 * from which legality could be inferred.
 */
export type Editing = {
  /** The occurrence currently in hand, or `null`. */
  pickedUp: string | null;
  /** The verdicts for the occurrence in hand, as the backend probed them. */
  candidates: CandidateGrid | null;
  /** Whether a write is in flight, so the grid stops offering a second one. */
  busy: boolean;
  /** Picks a lesson up, or puts it down when it is already in hand. */
  onPickUp: (activityId: string | null) => void;
  /** Drops what is in hand into one slot. */
  onDrop: (timeslotId: string) => void;
  /** Takes one lesson off the timetable. */
  onUnschedule: (activityId: string) => void;
  /** Locks or unlocks one placement. */
  onToggleLock: (activityId: string) => void;
};

/** What a reader is told about a pinned cell, by why it was pinned. */
const PIN_REASON: Record<PinReason, { label: string; tone: string }> = {
  conflict: { label: "Clash", tone: "ttCell--conflict" },
  shared: { label: "Several lessons", tone: "ttCell--shared" },
  brokenSpan: { label: "Broken run", tone: "ttCell--broken" },
};

/** Where the keyboard is, within one panel. */
type Focus = { panel: number; day: number; period: number };

export function TimetableGrid(props: {
  grid: ScopeGrid;
  lessons: readonly ProjectedLesson[];
  /** One cycle day's `ordinal`, or `null` for the whole cycle. */
  shownDay?: number | null;
  /** Present only when the timetable is open for editing. Absent means read-only. */
  editing?: Editing;
  onSelectLesson?: (index: number) => void;
}) {
  const { grid, lessons, shownDay = null, editing, onSelectLesson } = props;
  const container = useRef<HTMLDivElement>(null);
  const [focus, setFocus] = useState<Focus>({ panel: 0, day: 0, period: 0 });

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent, panelIndex: number, day: number, period: number) => {
      const panel = grid.panels[panelIndex];
      if (panel === undefined) return;

      // The bounds are the *rendered* columns, so arrowing right in a one-day view stops at
      // the one column rather than focusing a cell nobody can see.
      const columns =
        shownDay === null
          ? panel.days.length
          : panel.days.filter((one) => one.ordinal === shownDay).length;

      const moves: Record<string, [number, number]> = {
        ArrowRight: [day + 1, period],
        ArrowLeft: [day - 1, period],
        ArrowDown: [day, period + 1],
        ArrowUp: [day, period - 1],
        Home: [0, period],
        End: [columns - 1, period],
      };
      const move = moves[event.key];
      if (move === undefined) return;

      event.preventDefault();
      const [nextDay, nextPeriod] = move;
      if (nextDay < 0 || nextDay >= columns) return;
      if (nextPeriod < 0 || nextPeriod >= panel.periods.length) return;

      setFocus({ panel: panelIndex, day: nextDay, period: nextPeriod });
      const selector = `[data-cell="${panelIndex}-${nextDay}-${nextPeriod}"]`;
      container.current?.querySelector<HTMLElement>(selector)?.focus();
    },
    [grid.panels, shownDay],
  );

  if (grid.panels.length === 0) {
    return (
      <p className="ttEmpty">
        Nothing is scheduled in this timetable for{" "}
        <strong>{grid.scope.name}</strong>.
        {grid.scope.rosterDerived
          ? " This view is built from class enrolment records, so an empty grid may mean no" +
            " enrolment has been entered rather than that there are no lessons."
          : ""}
      </p>
    );
  }

  return (
    <div className="ttGrid" ref={container}>
      {grid.panels.length > 1 ? (
        <p className="ttNote" role="note">
          This {grid.scope.kind === "master" ? "timetable" : "schedule"} spans{" "}
          {grid.panels.length} bell schedules, whose periods do not line up with one another.
          Each is shown separately below — they cannot be read as a single week.
        </p>
      ) : null}

      {grid.panels.map((panel, panelIndex) => (
        <Panel
          key={panel.scheduleId}
          panel={panel}
          shownDay={shownDay}
          panelIndex={panelIndex}
          lessons={lessons}
          scopeName={grid.scope.name}
          focus={focus}
          editing={editing}
          onKeyDown={onKeyDown}
          onSelectLesson={onSelectLesson}
        />
      ))}
    </div>
  );
}

/** One bell schedule's table. */
function Panel(props: {
  panel: GridPanel;
  shownDay: number | null;
  panelIndex: number;
  lessons: readonly ProjectedLesson[];
  scopeName: string;
  focus: Focus;
  editing?: Editing;
  onKeyDown: (event: React.KeyboardEvent, panel: number, day: number, period: number) => void;
  onSelectLesson?: (index: number) => void;
}) {
  const {
    panel,
    shownDay,
    panelIndex,
    lessons,
    scopeName,
    focus,
    editing,
    onKeyDown,
    onSelectLesson,
  } = props;

  // The candidate grid is the lesson in hand's own bell schedule. A panel of a *different*
  // schedule therefore offers no drop target at all, which is the truthful answer rather than a
  // dead cell: a lesson may only sit in slots of the schedule its class resolved to, and
  // `duration-contiguous` is the rule that owns that. The grid does not restate it — it simply
  // has no verdict to render, and says so.
  const candidatesHere =
    editing?.candidates != null && editing.candidates.scheduleId === panel.scheduleId
      ? editing.candidates
      : null;

  // The columns actually rendered. A filter, not a re-derivation: these are the backend's
  // days, in the backend's order, with the ones the reader asked to hide left out.
  const days =
    shownDay === null ? panel.days : panel.days.filter((day) => day.ordinal === shownDay);

  /** The cell at one position. The backend guarantees exactly one exists. */
  const cellAt = (day: number, period: number): TimetableCell | undefined =>
    panel.cells.find(
      (cell) =>
        cell.dayOrdinal === days[day]?.ordinal &&
        cell.periodOrdinal === panel.periods[period]?.ordinal,
    );

  return (
    <section className="ttPanel">
      <div className="ttPanelHead">
        <h3>{panel.scheduleName}</h3>
        {panel.campusName === null ? null : (
          <p className="ttPanelCampus">{panel.campusName}</p>
        )}
      </div>

      <div className="ttScroll">
        <table className="ttTable">
          <caption className="srOnly">
            {scopeName} — {panel.scheduleName}
            {panel.campusName === null ? "" : ` at ${panel.campusName}`}. Days across,
            periods down.
          </caption>
          <thead>
            <tr>
              <th scope="col" className="ttPeriodHead">
                Period
              </th>
              {days.map((day) => (
                <th key={day.cycleDayId} scope="col">
                  {day.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {panel.periods.map((period, periodIndex) => (
              <tr
                key={period.periodId}
                className={period.isTeaching ? undefined : "ttRow--break"}
              >
                <th scope="row" className="ttPeriodHead">
                  <span className="ttPeriodLabel">{period.label}</span>
                  <span className="ttPeriodTime">
                    {period.startsAt}–{period.endsAt}
                  </span>
                  {period.isTeaching ? null : (
                    <span className="ttPeriodKind"> (no teaching)</span>
                  )}
                </th>
                {days.map((day, dayIndex) => {
                  const cell = cellAt(dayIndex, periodIndex);
                  // A continuation is covered by the span above it. Rendering a `<td>` here
                  // would push the row one column wide and break the table — which is the
                  // whole reason the backend marks it rather than leaving it out.
                  if (cell === undefined || cell.content.kind === "continuation") return null;

                  const isTabStop =
                    focus.panel === panelIndex &&
                    focus.day === dayIndex &&
                    focus.period === periodIndex;

                  return (
                    <Cell
                      key={day.cycleDayId}
                      cell={cell}
                      dayLabel={day.label}
                      periodLabel={period.label}
                      periodStart={period.startsAt}
                      lessons={lessons}
                      panelIndex={panelIndex}
                      dayIndex={dayIndex}
                      periodIndex={periodIndex}
                      isTabStop={isTabStop}
                      isTeaching={period.isTeaching}
                      editing={editing}
                      candidate={verdictAt(candidatesHere, day.ordinal, period.ordinal)}
                      onKeyDown={onKeyDown}
                      onSelectLesson={onSelectLesson}
                    />
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** One `<td>`, whichever of the three rendered kinds it is. */
function Cell(props: {
  cell: TimetableCell;
  dayLabel: string;
  periodLabel: string;
  periodStart: string;
  lessons: readonly ProjectedLesson[];
  panelIndex: number;
  dayIndex: number;
  periodIndex: number;
  isTabStop: boolean;
  isTeaching: boolean;
  editing?: Editing;
  candidate: CandidateGrid["cells"][number] | null;
  onKeyDown: (event: React.KeyboardEvent, panel: number, day: number, period: number) => void;
  onSelectLesson?: (index: number) => void;
}) {
  const {
    cell,
    dayLabel,
    periodLabel,
    lessons,
    panelIndex,
    dayIndex,
    periodIndex,
    isTabStop,
    isTeaching,
    editing,
    candidate,
    onKeyDown,
    onSelectLesson,
  } = props;

  const held = editing?.pickedUp ?? null;
  // Every one of these is read, never computed. `candidate.allowed` is the constraint registry's
  // answer for this exact cell; `candidate.blocking` is the rule's own sentence for refusing it.
  const isTarget = held !== null && candidate !== null;
  const mayDrop = isTarget && candidate.allowed && editing?.busy !== true;
  const dropTone = !isTarget ? "" : candidate.allowed ? " ttCell--legal" : " ttCell--illegal";

  /** The keyboard accelerators a cell carries. Every one has a button equivalent. */
  const onCellKeys = (event: React.KeyboardEvent) => {
    if (editing === undefined) {
      onKeyDown(event, panelIndex, dayIndex, periodIndex);
      return;
    }
    if (event.key === "Escape" && held !== null) {
      event.preventDefault();
      editing.onPickUp(null);
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      if (mayDrop && candidate !== null) {
        event.preventDefault();
        editing.onDrop(candidate.timeslotId);
        return;
      }
      const first = firstActivity(cell, lessons);
      if (held === null && first !== null) {
        event.preventDefault();
        editing.onPickUp(first);
        return;
      }
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      const first = firstActivity(cell, lessons);
      if (first !== null) {
        event.preventDefault();
        editing.onUnschedule(first);
        return;
      }
    }
    if (event.key === "l" || event.key === "L") {
      const first = firstActivity(cell, lessons);
      if (first !== null) {
        event.preventDefault();
        editing.onToggleLock(first);
        return;
      }
    }
    onKeyDown(event, panelIndex, dayIndex, periodIndex);
  };

  const dropProps = {
    onDragOver: (event: React.DragEvent) => {
      if (mayDrop) event.preventDefault();
    },
    onDrop: (event: React.DragEvent) => {
      if (!mayDrop || candidate === null) return;
      event.preventDefault();
      editing?.onDrop(candidate.timeslotId);
    },
  };

  /** What a reader hears about this cell as a target, which is the rules' own words. */
  const targetSays = !isTarget
    ? ""
    : candidate.allowed
      ? " Legal drop target."
      : ` Cannot be dropped here. ${candidate.blocking.join(" ")}`;

  if (cell.content.kind === "empty") {
    return (
      <td
        className={`${isTeaching ? "ttCell ttCell--empty" : "ttCell ttCell--unavailable"}${dropTone}`}
        data-cell={`${panelIndex}-${dayIndex}-${periodIndex}`}
        data-droppable={mayDrop ? "true" : undefined}
        tabIndex={isTabStop ? 0 : -1}
        aria-label={
          (isTeaching
            ? `${dayLabel}, ${periodLabel}: free`
            : `${dayLabel}, ${periodLabel}: not a teaching period`) + targetSays
        }
        onKeyDown={onCellKeys}
        {...(editing === undefined ? {} : dropProps)}
      />
    );
  }

  // `rows` comes from the backend and is the only `rowSpan` this component ever emits.
  //
  // A `switch` over the tag rather than a chain of ternaries, so the compiler proves two
  // things: that a `rowSpan` above 1 can only come from a `lesson` cell, and that the
  // `continuation` case — which renders nothing and is returned above — cannot reach here.
  // If the backend ever gains a fifth kind this stops compiling, which is the right
  // outcome: a cell this component does not understand must not be rendered as a guess.
  const content = cell.content;
  let rowSpan = 1;
  let indexes: number[] = [];
  let pinned: PinReason | null = null;
  switch (content.kind) {
    case "lesson":
      rowSpan = content.rows;
      indexes = [content.lesson];
      break;
    case "pinned":
      indexes = content.lessons;
      pinned = content.reason;
      break;
    // `empty` is handled above, so the compiler has already narrowed it away here.
    case "continuation":
      return null;
  }
  const tone = pinned === null ? "" : ` ${PIN_REASON[pinned].tone}`;

  const inCell: ProjectedLesson[] = indexes
    .map((index: number) => lessons[index])
    .filter((lesson): lesson is ProjectedLesson => lesson !== undefined);

  return (
    <td
      className={`ttCell ttCell--filled${tone}${dropTone}`}
      rowSpan={rowSpan}
      data-cell={`${panelIndex}-${dayIndex}-${periodIndex}`}
      data-droppable={mayDrop ? "true" : undefined}
      tabIndex={isTabStop ? 0 : -1}
      aria-label={describeCell(inCell, dayLabel, periodLabel, rowSpan, pinned) + targetSays}
      onKeyDown={onCellKeys}
      {...(editing === undefined ? {} : dropProps)}
      onClick={() => {
        const first = indexes[0];
        if (first !== undefined) onSelectLesson?.(first);
      }}
    >
      {pinned === null ? null : (
        <p className="ttPinTag">
          {/* Never colour alone: the reason is text and an icon as well as a tint. */}
          <span aria-hidden="true">{pinned === "shared" ? "▦" : "⚠"}</span>{" "}
          {PIN_REASON[pinned].label}
        </p>
      )}
      <ul className="ttLessons">
        {inCell.map((lesson: ProjectedLesson, at: number) => (
          <li
            key={`${lesson.activityId}-${at}`}
            className={`ttLesson${editing?.pickedUp === lesson.activityId ? " ttLesson--held" : ""}`}
            // The mouse path. Its keyboard equivalent is the Pick up button below, so every
            // lesson of a shared cell is reachable both ways rather than only the first.
            draggable={editing !== undefined && !lesson.flags.isLocked && !editing.busy}
            onDragStart={() => editing?.onPickUp(lesson.activityId)}
            onDragEnd={() => editing?.onPickUp(null)}
          >
            <span className="ttLessonName">
              {lesson.label === null ? lesson.sectionId : lesson.label.short}
            </span>
            {lesson.teachers.length > 0 ? (
              <span className="ttLessonTeachers">{lesson.teachers.join(", ")}</span>
            ) : null}
            {lesson.room === null ? null : (
              <span className="ttLessonRoom">{lesson.room.name}</span>
            )}
            <span className="ttLessonTime">
              {lesson.startsAt}–{lesson.endsAt}
            </span>
            {lesson.defect === null ? null : (
              <span className="ttLessonDefect">
                <span aria-hidden="true">⚠</span> {describeDefect(lesson)}
              </span>
            )}
            {lesson.flags.isLocked ? (
              <span className="ttLessonFlag">
                <span aria-hidden="true">🔒</span> Locked
              </span>
            ) : null}
            {lesson.flags.isManual ? (
              <span className="ttLessonFlag">Placed by hand</span>
            ) : null}
            {editing === undefined ? null : (
              <span className="ttLessonTools">
                <button
                  type="button"
                  className="linkish"
                  disabled={editing.busy || lesson.flags.isLocked}
                  aria-pressed={editing.pickedUp === lesson.activityId}
                  onClick={(event) => {
                    event.stopPropagation();
                    editing.onPickUp(
                      editing.pickedUp === lesson.activityId ? null : lesson.activityId,
                    );
                  }}
                >
                  {editing.pickedUp === lesson.activityId ? "Put down" : "Pick up"}
                </button>
                <button
                  type="button"
                  className="linkish"
                  disabled={editing.busy}
                  onClick={(event) => {
                    event.stopPropagation();
                    editing.onToggleLock(lesson.activityId);
                  }}
                >
                  {lesson.flags.isLocked ? "Unlock" : "Lock"}
                </button>
                <button
                  type="button"
                  className="linkish"
                  disabled={editing.busy || lesson.flags.isLocked}
                  onClick={(event) => {
                    event.stopPropagation();
                    editing.onUnschedule(lesson.activityId);
                  }}
                >
                  Unschedule
                </button>
              </span>
            )}
          </li>
        ))}
      </ul>
    </td>
  );
}

/**
 * The first occurrence a cell holds, for the keyboard accelerators.
 *
 * A shortcut, never the only route: every lesson of a shared cell has its own Pick up, Lock and
 * Unschedule button, so the keyboard reaches all of them and not only this one.
 */
function firstActivity(
  cell: TimetableCell,
  lessons: readonly ProjectedLesson[],
): string | null {
  const content = cell.content;
  const index =
    content.kind === "lesson" ? content.lesson : content.kind === "pinned" ? content.lessons[0] : undefined;
  if (index === undefined) return null;
  return lessons[index]?.activityId ?? null;
}

/**
 * The whole sentence a screen reader announces for one cell.
 *
 * Position is not available to somebody hearing the page, so every fact a sighted reader
 * takes from the cell's place in the table is said out loud: which day, which periods, the
 * clock span, the lesson, who teaches it and where.
 */
function describeCell(
  held: readonly ProjectedLesson[],
  dayLabel: string,
  periodLabel: string,
  rowSpan: number,
  pinned: PinReason | null,
): string {
  const where =
    rowSpan > 1
      ? `${dayLabel}, ${rowSpan} periods from ${periodLabel}`
      : `${dayLabel}, ${periodLabel}`;

  const parts = held.map((lesson) => {
    const name = lesson.label === null ? "an unnamed lesson" : lesson.label.short;
    const who = lesson.teachers.length > 0 ? `, ${lesson.teachers.join(" and ")}` : "";
    const room = lesson.room === null ? "" : `, ${lesson.room.name}`;
    const defect = lesson.defect === null ? "" : `. ${describeDefect(lesson)}`;
    return `${name}${who}${room}, ${lesson.startsAt} to ${lesson.endsAt}${defect}`;
  });

  const prefix =
    pinned === "conflict"
      ? "Clash. "
      : pinned === "brokenSpan"
        ? "Broken run. "
        : pinned === "shared"
          ? `${held.length} lessons. `
          : "";

  return `${prefix}${where} — ${parts.join("; ")}`;
}

/**
 * What is wrong with a lesson's run, in words.
 *
 * The backend reports the defect; this names it. Each case is a thing that becomes true when
 * a timetable's grid moves after it was saved, which is precisely when a reader needs to be
 * told rather than shown a quietly shortened lesson.
 */
function describeDefect(lesson: ProjectedLesson): string {
  const defect = lesson.defect;
  if (defect === null) return "";
  const said: string[] = [];
  if (defect.got < defect.wanted) {
    said.push(
      `needs ${defect.wanted} periods but only ${defect.got} are available from here`,
    );
  }
  if (defect.crossesNonTeaching) said.push("runs across a period with no teaching");
  if (defect.startsOutsideTeaching) said.push("starts in a period with no teaching");
  if (defect.wrongBellSchedule) said.push("sits on a bell schedule its class does not use");
  return said.length === 0 ? "" : `This lesson ${said.join(", and ")}.`;
}
