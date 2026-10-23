/**
 * What a timetable does not claim.
 *
 * Every case here is a sentence a record requires to be on screen, and each is one a
 * reasonable implementation drops in the obvious direction — by hiding a panel that has
 * nothing to say, by merging three states into one list, or by calling a difficulty an
 * impossibility.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import type {
  FindingCounts,
  FindingView,
  ScoringView,
  SharedEntity,
  UnscheduledView,
  UnverifiedCounts,
} from "@evara/domain";

import {
  FindingsPanel,
  ScoringPanel,
  UnscheduledPanel,
  UnverifiedPanel,
  UsabilityLine,
} from "./Disclosure";

const NOTHING: UnverifiedCounts = {
  indeterminateStudentPairs: 0,
  uncheckedCapacities: 0,
  sharedEntities: 0,
  unenforcedRestrictions: 0,
};

// ---------------------------------------------------------------- unverified counts

describe("the unverified counts", () => {
  it("still render when every figure is zero", () => {
    // A panel that disappeared would make its own absence unreadable, and "nothing
    // undisclosed" is a different statement from "conflict-free".
    render(<UnverifiedPanel counts={NOTHING} />);
    expect(screen.getByText(/Nothing undisclosed/)).toBeTruthy();
    expect(screen.getByText(/no confirmed clash/)).toBeTruthy();
  });

  it("never claims a timetable is conflict-free", () => {
    // The phrase does appear, and must: the panel's whole job at all-zeroes is to say that
    // "nothing undisclosed" is *not* the same as "conflict-free". What must never appear is
    // the phrase as a **claim**, so this asserts every occurrence is negated.
    render(<UnverifiedPanel counts={NOTHING} />);
    const said = document.body.textContent ?? "";
    const claims = said.match(/conflict-free/gi) ?? [];
    expect(claims).toHaveLength(1);
    expect(said).toMatch(/not that the timetable is conflict-free/);
  });

  it("says what each non-zero figure means, with its consequence", () => {
    render(
      <UnverifiedPanel
        counts={{
          indeterminateStudentPairs: 3,
          uncheckedCapacities: 2,
          sharedEntities: 1,
          unenforcedRestrictions: 4,
        }}
      />,
    );
    expect(screen.getByText(/3 pairs of student groups have not been declared disjoint/)).toBeTruthy();
    expect(screen.getByText(/2 lessons have no expected attendance/)).toBeTruthy();
    expect(screen.getByText(/Evara cannot compare the two schools' timetables/)).toBeTruthy();
    expect(screen.getByText(/4 stored availability restrictions/)).toBeTruthy();
  });

  it("names a shared entity and both schools when the names are available", () => {
    const shared: SharedEntity[] = [
      { kind: "teacher", id: "t1", name: "A. Okafor", alsoAt: ["Southmoor"] },
    ];
    render(
      <UnverifiedPanel counts={{ ...NOTHING, sharedEntities: 1 }} shared={shared} canName />,
    );
    // ADR 0032 rule 3: name the person, both schools, and the consequence.
    const said = screen.getByText(/is also held by/).textContent ?? "";
    expect(said).toContain("A. Okafor");
    expect(said).toContain("Southmoor");
    expect(said).toContain("will not detect a clash between them");
  });

  it("admits it cannot name them when the problem was not compiled", () => {
    render(
      <UnverifiedPanel
        counts={{ ...NOTHING, sharedEntities: 2 }}
        shared={[]}
        canName={false}
      />,
    );
    expect(screen.getByText(/open this timetable to see which/)).toBeTruthy();
  });
});

// ---------------------------------------------------------------- the three states

describe("usability", () => {
  const counts = (over: Partial<FindingCounts> = {}): FindingCounts => ({
    hard: 0,
    medium: 0,
    soft: 0,
    unknown: 0,
    ...over,
  });

  it("says a hard violation means the timetable cannot be run", () => {
    render(
      <UsabilityLine counts={counts({ hard: 2 })} placed={10} required={10} schoolName="Northgate" />,
    );
    expect(screen.getByText(/2 hard violations — this timetable cannot be run/)).toBeTruthy();
  });

  it("says no confirmed clash within the school, not conflict-free", () => {
    render(
      <UsabilityLine counts={counts()} placed={10} required={10} schoolName="Northgate" />,
    );
    expect(screen.getByText("No confirmed clash within Northgate.")).toBeTruthy();
  });

  it("reports partial as an ordinary state rather than a failure", () => {
    render(<UsabilityLine counts={counts()} placed={8} required={10} schoolName="Northgate" />);
    const said = screen.getByText(/8 of 10 lessons placed/).textContent ?? "";
    expect(said).toContain("usable, with 2 unplaced");
    // The two statements are separate: usable *and* partial.
    expect(screen.getByText("No confirmed clash within Northgate.")).toBeTruthy();
  });
});

// ---------------------------------------------------------------- unscheduled

describe("unplaced lessons", () => {
  const one = (over: Partial<UnscheduledView> = {}): UnscheduledView => ({
    activityId: "a1",
    label: "9 Mathematics B",
    cause: "no-candidate-slot",
    isImpossible: false,
    diagnosticCode: null,
    reason: null,
    ...over,
  });

  it("separates what no timetable can place from what this run did not", () => {
    render(
      <UnscheduledPanel
        unscheduled={[
          one({ activityId: "a1", isImpossible: true, diagnosticCode: "duration-too-long" }),
          one({ activityId: "a2" }),
        ]}
      />,
    );
    expect(screen.getByText("No timetable can place these")).toBeTruthy();
    expect(screen.getByText("This run could not place these")).toBeTruthy();
  });

  it("never calls a merely difficult lesson impossible", () => {
    render(<UnscheduledPanel unscheduled={[one()]} />);
    expect(screen.queryByText("No timetable can place these")).toBeNull();
    expect(screen.getByText(/These are not impossible/)).toBeTruthy();
  });

  it("shows the stored cause when the engine's sentence was not kept", () => {
    render(<UnscheduledPanel unscheduled={[one()]} />);
    expect(screen.getByText(/the reason was recorded as/)).toBeTruthy();
    expect(screen.getByText("no-candidate-slot")).toBeTruthy();
  });

  it("renders nothing at all when everything was placed", () => {
    const { container } = render(<UnscheduledPanel unscheduled={[]} />);
    expect(container.textContent).toBe("");
  });
});

// ---------------------------------------------------------------- scorings

describe("the score panel", () => {
  const scoring = (over: Partial<ScoringView> = {}): ScoringView => ({
    kind: "GENERATION",
    isGeneration: true,
    evaluatedAt: "2026-10-23T14:02:00.000Z",
    hard: 0,
    medium: -40,
    soft: 0,
    placed: 10,
    required: 10,
    ...over,
  });

  it("shows both scorings, each labelled with when it was taken", () => {
    render(
      <ScoringPanel
        scorings={[
          scoring(),
          scoring({
            kind: "REVALIDATION",
            isGeneration: false,
            evaluatedAt: "2026-10-23T16:41:00.000Z",
            hard: -200,
          }),
        ]}
      />,
    );
    expect(screen.getByText("As generated")).toBeTruthy();
    expect(screen.getByText("As it stands")).toBeTruthy();
    expect(screen.getByText("2026-10-23T14:02:00.000Z")).toBeTruthy();
    expect(screen.getByText("2026-10-23T16:41:00.000Z")).toBeTruthy();
    expect(screen.getByText("-200")).toBeTruthy();
  });

  it("says the generation figure never changes and the current one is now", () => {
    render(<ScoringPanel scorings={[scoring()]} />);
    expect(screen.getByText(/what the solver found, and never changes/)).toBeTruthy();
  });

  it("says plainly when a timetable has never been scored", () => {
    render(<ScoringPanel scorings={[]} />);
    expect(screen.getByText("This timetable has never been scored.")).toBeTruthy();
  });
});

// ---------------------------------------------------------------- findings

describe("the findings panel", () => {
  const finding = (over: Partial<FindingView> = {}): FindingView => ({
    typeKey: "teacher-clash",
    knownType: true,
    level: "HARD",
    penalty: -1000,
    message: "A. Okafor already teaches 9 Mathematics B in Day A period 1",
    detail: "two lessons at once",
    cycleDayId: null,
    startsAt: null,
    endsAt: null,
    activityIds: [],
    entities: [],
    source: "explained",
    ...over,
  });

  it("renders the rule's own sentence where there is one", () => {
    render(
      <FindingsPanel
        findings={[finding()]}
        counts={{ hard: 1, medium: 0, soft: 0, unknown: 0 }}
      />,
    );
    expect(
      screen.getByText("A. Okafor already teaches 9 Mathematics B in Day A period 1"),
    ).toBeTruthy();
  });

  it("falls back to the stored fact when no sentence was available", () => {
    render(
      <FindingsPanel
        findings={[finding({ message: null, source: "stored" })]}
        counts={{ hard: 1, medium: 0, soft: 0, unknown: 0 }}
      />,
    );
    expect(screen.getByText("two lessons at once")).toBeTruthy();
  });

  it("reports a rule from a newer version rather than hiding it", () => {
    render(
      <FindingsPanel
        findings={[finding({ typeKey: "from-the-future", knownType: false })]}
        counts={{ hard: 1, medium: 0, soft: 0, unknown: 1 }}
      />,
    );
    expect(screen.getByText("from-the-future")).toBeTruthy();
    expect(screen.getByText(/which this one cannot evaluate/)).toBeTruthy();
    expect(screen.getByText(/1 from a newer version of Evara/)).toBeTruthy();
  });

  it("does not claim a clean timetable is good or complete", () => {
    render(
      <FindingsPanel findings={[]} counts={{ hard: 0, medium: 0, soft: 0, unknown: 0 }} />,
    );
    const said = screen.getByText(/none is broken by this timetable/).textContent ?? "";
    expect(said).toContain("Of the rules this version implements");
    expect(said).toContain("not a claim that the timetable is good or complete");
  });
});
