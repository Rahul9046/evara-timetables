/**
 * What a timetable does not claim: the panels that keep it from being oversold.
 *
 * Every component here exists because a record decided that something should be **visible
 * rather than closed**, and each is written so that it cannot quietly stop saying so.
 *
 * ## Four figures, always four
 *
 * {@link UnverifiedPanel} renders all four `UnverifiedCounts` **including when they are
 * zero**. A panel that disappeared when it had nothing to say would make its absence
 * unreadable, and even at all zeroes the correct phrasing is *"no confirmed clash"* rather
 * than *"conflict-free"* — `SCHEDULING-ARCHITECTURE-DECISIONS.md` §5 rule 4, and
 * `UnverifiedCounts`'s own documentation.
 *
 * ## Three states, never merged
 *
 * A hard violation, a partial timetable and an unplaced lesson are different things:
 *
 * | State | Means |
 * | --- | --- |
 * | a hard violation | the timetable **cannot be run** |
 * | partial | usable, with lessons unplaced — an ordinary state, not an error |
 * | unscheduled | which lessons, and whether anything proves it impossible |
 *
 * Collapsing them into one "problems" list is the failure `is_usable()` is deliberately not
 * called `is_feasible` to avoid.
 *
 * ## The word "impossible" is gated
 *
 * {@link UnscheduledPanel} says *impossible* only where `isImpossible` is true. Everything
 * else is *"this run could not place it"* — because a greedy construction that leaves a
 * lesson unplaced has discovered that **its** arrangement had no room, not that no timetable
 * exists. `CLAUDE.md` §3, and the backend decides it, never this component.
 */
import type {
  FindingCounts,
  FindingView,
  ScoringView,
  SharedEntity,
  UnscheduledView,
  UnverifiedCounts,
} from "@evara/domain";

/** One of the four disclosures, and the sentence it is worth. */
const UNVERIFIED: ReadonlyArray<{
  key: keyof UnverifiedCounts;
  says: (n: number) => string;
}> = [
  {
    key: "indeterminateStudentPairs",
    says: (n) =>
      `${n} pair${n === 1 ? "" : "s"} of student groups have not been declared disjoint, so` +
      " two groups that share children may have been timetabled together.",
  },
  {
    key: "uncheckedCapacities",
    says: (n) =>
      `${n} lesson${n === 1 ? "" : "s"} have no expected attendance, so no room capacity was` +
      " compared.",
  },
  {
    key: "sharedEntities",
    says: (n) =>
      `${n} teacher${n === 1 ? "" : "s"}, room${n === 1 ? "" : "s"} or resource${
        n === 1 ? "" : "s"
      } are also held by another school. Evara cannot compare the two schools' timetables.`,
  },
  {
    key: "unenforcedRestrictions",
    says: (n) =>
      `${n} stored availability restriction${n === 1 ? "" : "s"} are recorded but are not` +
      " enforced by this engine.",
  },
];

/**
 * What this timetable could not check, and who else holds what it uses.
 *
 * `shared` is non-empty only for a compiled view: the names live in the problem, and a
 * stored-only view has the count and nothing more. The panel says which it is showing rather
 * than implying it knows more than it does.
 */
export function UnverifiedPanel(props: {
  counts: UnverifiedCounts;
  shared?: readonly SharedEntity[];
  canName?: boolean;
}) {
  const { counts, shared = [], canName = true } = props;
  const disclosed = UNVERIFIED.filter(({ key }) => counts[key] > 0);

  return (
    <section className="ttPanel ttPanel--disclose" aria-labelledby="ttUnverified">
      <h3 id="ttUnverified">What this timetable could not check</h3>

      {disclosed.length === 0 ? (
        <p className="ttDiscloseNone">
          Nothing undisclosed. Every check this engine implements was applied — which means{" "}
          <strong>no confirmed clash</strong> within this school, not that the timetable is
          conflict-free.
        </p>
      ) : (
        <ul className="ttDiscloseList">
          {disclosed.map(({ key, says }) => (
            <li key={key}>{says(counts[key])}</li>
          ))}
        </ul>
      )}

      {counts.sharedEntities > 0 ? (
        shared.length > 0 ? (
          <div className="ttShared">
            <h4>Shared with another school</h4>
            <ul>
              {shared.map((entity) => (
                <li key={`${entity.kind}-${entity.id}`}>
                  <strong>{entity.name}</strong> ({entity.kind}) is also held by{" "}
                  {entity.alsoAt.join(" and ")}. Evara cannot compare the two schools'
                  timetables and will not detect a clash between them.
                </li>
              ))}
            </ul>
          </div>
        ) : canName ? null : (
          <p className="ttSharedUnnamed">
            {counts.sharedEntities} shared{" "}
            {counts.sharedEntities === 1 ? "entity" : "entities"} — open this timetable to see
            which.
          </p>
        )
      ) : null}
    </section>
  );
}

/**
 * The two scorings, side by side, each labelled with when it was taken.
 *
 * A screen showing one number would have to choose which, and either choice is a lie about
 * the other: a `GENERATION` scoring is what the solver found when it built the timetable and
 * is immutable; a `REVALIDATION` is what the rules find *now*.
 */
export function ScoringPanel(props: { scorings: readonly ScoringView[] }) {
  const { scorings } = props;
  if (scorings.length === 0) {
    return (
      <section className="ttPanel" aria-labelledby="ttScores">
        <h3 id="ttScores">Score</h3>
        <p className="ttDiscloseNone">This timetable has never been scored.</p>
      </section>
    );
  }

  return (
    <section className="ttPanel" aria-labelledby="ttScores">
      <h3 id="ttScores">Score</h3>
      <div className="ttScroll">
        <table className="ttScoreTable">
          <caption className="srOnly">
            Each scoring, with the time it was taken and the penalties it found.
          </caption>
          <thead>
            <tr>
              <th scope="col">Scoring</th>
              <th scope="col">Taken</th>
              <th scope="col">Hard</th>
              <th scope="col">Medium</th>
              <th scope="col">Soft</th>
              <th scope="col">Placed</th>
            </tr>
          </thead>
          <tbody>
            {scorings.map((scoring) => (
              <tr key={scoring.kind}>
                <th scope="row">
                  {scoring.isGeneration ? "As generated" : "As it stands"}
                  <span className="ttScoreKindNote">
                    {scoring.isGeneration
                      ? " — what the solver found, and never changes"
                      : " — what the rules find now"}
                  </span>
                </th>
                <td>{scoring.evaluatedAt}</td>
                <td className="ttNum">{scoring.hard}</td>
                <td className="ttNum">{scoring.medium}</td>
                <td className="ttNum">{scoring.soft}</td>
                <td className="ttNum">
                  {scoring.placed} of {scoring.required}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** Whether a timetable can be run at all, and whether it is complete. */
export function UsabilityLine(props: {
  counts: FindingCounts;
  placed: number;
  required: number;
  schoolName: string;
}) {
  const { counts, placed, required, schoolName } = props;
  const complete = placed >= required;

  return (
    <p className="ttVerdict">
      {counts.hard > 0 ? (
        <span className="ttVerdict--hard">
          <span aria-hidden="true">⚠</span> {counts.hard} hard violation
          {counts.hard === 1 ? "" : "s"} — this timetable cannot be run.
        </span>
      ) : (
        <span className="ttVerdict--ok">
          No confirmed clash within {schoolName}.
        </span>
      )}{" "}
      {complete ? (
        <span>Every lesson is placed.</span>
      ) : (
        <span className="ttVerdict--partial">
          {placed} of {required} lessons placed — usable, with {required - placed} unplaced.
        </span>
      )}
    </p>
  );
}

/** The findings, with the rule's own sentence where one was available. */
export function FindingsPanel(props: {
  findings: readonly FindingView[];
  counts: FindingCounts;
}) {
  const { findings, counts } = props;

  return (
    <section className="ttPanel" aria-labelledby="ttFindings">
      <h3 id="ttFindings">Findings</h3>
      <p className="ttFindingCounts">
        {counts.hard} hard · {counts.medium} medium · {counts.soft} soft
        {counts.unknown > 0 ? ` · ${counts.unknown} from a newer version of Evara` : ""}
      </p>

      {findings.length === 0 ? (
        <p className="ttDiscloseNone">
          Of the rules this version implements, none is broken by this timetable. That is not
          a claim that the timetable is good or complete.
        </p>
      ) : (
        <ul className="ttFindings">
          {findings.map((finding, at) => (
            <li
              key={`${finding.typeKey}-${at}`}
              className={`ttFinding ttFinding--${finding.level.toLowerCase()}`}
            >
              <p className="ttFindingHead">
                <span className="ttFindingLevel">{finding.level}</span>{" "}
                <code>{finding.typeKey}</code>
                {finding.knownType ? null : (
                  <span className="ttFindingUnknown">
                    {" "}
                    — recorded by a newer version of Evara, which this one cannot evaluate
                  </span>
                )}
              </p>
              {/* The rule's own words where a problem was compiled; the stored fact
                  otherwise. Never a sentence this component composed. */}
              <p className="ttFindingBody">{finding.message ?? finding.detail}</p>
              {finding.startsAt === null ? null : (
                <p className="ttFindingWhen">
                  {finding.startsAt}–{finding.endsAt}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** What could not be placed, grouped by whether anything proves it impossible. */
export function UnscheduledPanel(props: { unscheduled: readonly UnscheduledView[] }) {
  const { unscheduled } = props;
  if (unscheduled.length === 0) return null;

  const impossible = unscheduled.filter((one) => one.isImpossible);
  const difficult = unscheduled.filter((one) => !one.isImpossible);

  return (
    <section className="ttPanel" aria-labelledby="ttUnplaced">
      <h3 id="ttUnplaced">
        {unscheduled.length} lesson{unscheduled.length === 1 ? "" : "s"} not placed
      </h3>

      {impossible.length > 0 ? (
        <>
          <h4>No timetable can place these</h4>
          <p className="ttUnplacedNote">
            Each of these is impossible as the school is set up now. The setup has to change
            before any timetable can place them.
          </p>
          <ul className="ttUnplaced">
            {impossible.map((one) => (
              <li key={one.activityId}>
                <strong>{one.label ?? one.activityId}</strong>
                {one.reason === null ? null : <span> — {one.reason}</span>}
                {one.diagnosticCode === null ? null : (
                  <code className="ttUnplacedCode">{one.diagnosticCode}</code>
                )}
              </li>
            ))}
          </ul>
        </>
      ) : null}

      {difficult.length > 0 ? (
        <>
          <h4>This run could not place these</h4>
          <p className="ttUnplacedNote">
            {/* Deliberately not "impossible". A construction that ran out of room has
                learnt that *its* arrangement had none, which is a different claim. */}
            These are not impossible — this run simply did not find room for them. Generating
            again with a different seed may place them.
          </p>
          <ul className="ttUnplaced">
            {difficult.map((one) => (
              <li key={one.activityId}>
                <strong>{one.label ?? one.activityId}</strong>
                {one.reason === null ? (
                  <span className="ttUnplacedCause">
                    {" "}
                    — the reason was recorded as <code>{one.cause}</code>
                  </span>
                ) : (
                  <span> — {one.reason}</span>
                )}
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </section>
  );
}
