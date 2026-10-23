/**
 * The Timetables area: the selection both of its screens share, and the switch between them.
 *
 * ## One answer about which school, at any moment
 *
 * The selected school is held by the shell and handed down, as `SchoolSetup` holds its own —
 * ADR 0023 scopes the interface to one school, and a selector that scoped some screens and
 * not others would be worse than no selector at all.
 *
 * ### Switching school remounts both screens
 *
 * `key={schoolId}` is the stale-state guarantee, extended here from `SchoolSetup`'s. Both
 * screens hold state that is *about* a school: a chosen cycle, an open timetable, a pending
 * delete confirmation, a generation session. Re-rendering with a new `schoolId` would leave
 * all of it standing, and a confirm-delete opened under Northgate that fired after a switch
 * to Southmoor would be acting on the wrong list.
 *
 * Remounting discards it. The backend refuses a cross-school read regardless, and both
 * layers are wanted: the key stops the user being shown a plausible wrong screen, the
 * backend stops the operation.
 *
 * ## The cycle is chosen, never defaulted
 *
 * ADR 0032 rule 1 scopes generation to one `(school, cycle, term)`, and
 * `SchedulingSelection`'s own documentation says the absence of a default *is* the decision:
 * a solve that picked a cycle would be building a timetable over a grid the user did not
 * choose.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import type { Cycle, School, Term } from "@evara/domain";

import {
  describeSetupError,
  listAcademicYears,
  listCycles,
  listSchools,
  listTerms,
} from "../../ipc/setup";
import type { Selection } from "../../ipc/timetables";
import { Banner, Loading } from "../ui/Shell";
import { Generate } from "./Generate";
import { Timetables } from "./Timetables";

/** Which of the area's two screens is showing. */
type Screen = "list" | "generate";

export function TimetableArea(props: {
  schoolId: string | null;
  onOpenSection?: (section: string) => void;
}) {
  const { schoolId, onOpenSection } = props;
  const [schools, setSchools] = useState<School[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    listSchools()
      .then((found) => {
        if (live) setSchools(found);
      })
      .catch((e: unknown) => {
        if (live) setError(describeSetupError(e));
      });
    return () => {
      live = false;
    };
  }, []);

  if (error !== null) return <Banner tone="error">{error}</Banner>;
  if (schools === null) return <Loading what="timetables" />;

  const school = schools.find((one) => one.id === schoolId) ?? schools[0] ?? null;
  if (school === null) {
    return (
      <div className="ttScreen">
        <h2>Timetables</h2>
        <p className="ttEmpty">
          This campus file describes no school yet. A timetable is generated for one school's
          cycle, so there is nothing to generate over until one exists.{" "}
          <button
            type="button"
            className="linkish"
            onClick={() => onOpenSection?.("schools")}
          >
            Create one under Schools
          </button>
          .
        </p>
      </div>
    );
  }

  // Remounts on a school switch, discarding every piece of per-school state below.
  return <Area key={school.id} school={school} onOpenSection={onOpenSection} />;
}

/** One school's timetables, with the grid selection both screens share. */
function Area(props: { school: School; onOpenSection?: (section: string) => void }) {
  const { school, onOpenSection } = props;

  const [screen, setScreen] = useState<Screen>("list");
  const [openId, setOpenId] = useState<string | null>(null);
  const [cycles, setCycles] = useState<Cycle[] | null>(null);
  const [cycleId, setCycleId] = useState<string | null>(null);
  const [terms, setTerms] = useState<Term[]>([]);
  const [termId, setTermId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    listCycles(school.id)
      .then((found) => {
        if (!live) return;
        setCycles(found);
        // The first cycle is *offered*, not assumed: a school usually has one, and making
        // the user pick from a list of one is friction with no safety value. The picker stays
        // visible, so the choice is always on screen rather than implied.
        setCycleId(found[0]?.id ?? null);
      })
      .catch((e: unknown) => {
        if (live) setError(describeSetupError(e));
      });
    return () => {
      live = false;
    };
  }, [school.id]);

  // Terms belong to academic years, and a cycle names no year — ADR 0017: a cycle carries
  // no start date and is not tied to a year. So every term the school has is offered, and
  // "whole year" (no term at all) is an ordinary selection rather than a missing value.
  useEffect(() => {
    let live = true;
    listAcademicYears(school.id)
      .then(async (years) => {
        const found = await Promise.all(years.map((year) => listTerms(year.id)));
        if (live) setTerms(found.flat());
      })
      .catch(() => {
        if (live) setTerms([]);
      });
    return () => {
      live = false;
    };
  }, [school.id]);

  const selection = useMemo<Selection | null>(
    () => (cycleId === null ? null : { schoolId: school.id, cycleId, termId }),
    [school.id, cycleId, termId],
  );

  const onOpenSaved = useCallback((id: string) => {
    setOpenId(id);
    setScreen("list");
  }, []);

  return (
    <div className="ttArea">
      <div className="ttAreaHead">
        <nav className="ttTabs" aria-label="Timetables">
          <button
            type="button"
            className={`ttTab${screen === "list" ? " ttTab--on" : ""}`}
            aria-current={screen === "list" ? "page" : undefined}
            onClick={() => setScreen("list")}
          >
            Saved timetables
          </button>
          <button
            type="button"
            className={`ttTab${screen === "generate" ? " ttTab--on" : ""}`}
            aria-current={screen === "generate" ? "page" : undefined}
            onClick={() => {
              setOpenId(null);
              setScreen("generate");
            }}
          >
            Generate
          </button>
        </nav>

        {cycles === null ? null : cycles.length === 0 ? (
          <p className="ttEmpty">
            {school.name} has no cycle, so there is no grid to build a timetable over.{" "}
            <button
              type="button"
              className="linkish"
              onClick={() => onOpenSection?.("cycles")}
            >
              Create one under Cycles and cycle days
            </button>
            .
          </p>
        ) : (
          <div className="ttSelection">
            <label className="ttSelect">
              <span>Cycle</span>
              <select
                value={cycleId ?? ""}
                onChange={(event) => setCycleId(event.target.value || null)}
              >
                {cycles.map((cycle) => (
                  <option key={cycle.id} value={cycle.id}>
                    {cycle.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="ttSelect">
              <span>Term</span>
              <select
                value={termId ?? ""}
                onChange={(event) => setTermId(event.target.value || null)}
              >
                <option value="">Whole year</option>
                {terms.map((term) => (
                  <option key={term.id} value={term.id}>
                    {term.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
        )}
      </div>

      {error === null ? null : <Banner tone="error">{error}</Banner>}

      {screen === "generate" ? (
        <Generate
          selection={selection}
          schoolName={school.name}
          onOpenSaved={onOpenSaved}
          onOpenSection={onOpenSection}
        />
      ) : (
        <Timetables
          schoolId={school.id}
          schoolName={school.name}
          openId={openId}
          onOpen={setOpenId}
          onGenerate={() => {
            setOpenId(null);
            setScreen("generate");
          }}
          onOpenSection={onOpenSection}
        />
      )}
    </div>
  );
}
