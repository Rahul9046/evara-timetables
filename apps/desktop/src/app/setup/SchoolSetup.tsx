/**
 * The School Setup workflow: navigation, and the nine sections behind it.
 *
 * ## Why a sidebar of nine sections rather than one long form
 *
 * The brief asks not to put all configuration on one enormous screen, and the model
 * agrees: the entities fall into natural pairs — a campus and its buildings, a cycle and
 * its days — and the repository reads them that way. One section per pair keeps each
 * screen about one decision.
 *
 * The order is the order a school is actually set up in, and it is also a dependency
 * order: rooms need a campus, terms need a year, the grid needs a cycle and a bell
 * schedule. {@link SECTIONS} says so explicitly, and a section whose prerequisite is
 * missing says what to do first instead of failing at a foreign key.
 *
 * ## Everything except School Details needs a school
 *
 * There is exactly one school per project and it owns everything else, so until it exists
 * the only thing to do is create it. Rather than letting eight sections each discover
 * that, the shell holds the school record and shows the first-run state once.
 */
import { useCallback, useState, type ReactNode } from "react";

import type { School } from "@evara/domain";
import { getSchool } from "../../ipc/setup";
import { Banner, Loading } from "../ui/Shell";
import { AcademicYearsAndTerms } from "./AcademicYearsAndTerms";
import { CalendarMapping } from "./CalendarMapping";
import { CampusesAndBuildings } from "./CampusesAndBuildings";
import { CyclesAndCycleDays } from "./CyclesAndCycleDays";
import { GridPreviewSection } from "./GridPreviewSection";
import { PeriodStructuresAndPeriods } from "./PeriodStructuresAndPeriods";
import { Resources } from "./Resources";
import { RoomsAndRoomTypes } from "./RoomsAndRoomTypes";
import { SchoolDetails } from "./SchoolDetails";
import { useValue } from "./useList";

/** One entry in the setup navigation. */
interface Section {
  readonly id: string;
  readonly label: string;
  /** One line in the sidebar, so the user can tell the sections apart at a glance. */
  readonly blurb: string;
  /** Whether this section can be used before a school record exists. */
  readonly needsSchool: boolean;
  readonly render: (school: School) => ReactNode;
}

/**
 * The nine sections, in setup order.
 *
 * Exported so a test can assert the list rather than the rendering: the set of sections
 * and their order is a product decision, and a silently dropped one would otherwise only
 * be noticed by someone looking for it.
 */
export const SECTIONS: readonly Section[] = [
  {
    id: "school",
    label: "School details",
    blurb: "Name, time zone, locale",
    needsSchool: false,
    render: () => null, // Rendered specially; see `SchoolSetup`.
  },
  {
    id: "campuses",
    label: "Campuses and buildings",
    blurb: "Sites, and how they are divided",
    needsSchool: true,
    render: (school) => <CampusesAndBuildings school={school} />,
  },
  {
    id: "rooms",
    label: "Rooms and room types",
    blurb: "Teaching spaces and their kinds",
    needsSchool: true,
    render: (school) => <RoomsAndRoomTypes school={school} />,
  },
  {
    id: "resources",
    label: "Resources",
    blurb: "Shared equipment a lesson may need",
    needsSchool: true,
    render: (school) => <Resources school={school} />,
  },
  {
    id: "years",
    label: "Academic years and terms",
    blurb: "The year, and its divisions",
    needsSchool: true,
    render: (school) => <AcademicYearsAndTerms school={school} />,
  },
  {
    id: "cycles",
    label: "Cycles and cycle days",
    blurb: "The repeating pattern — not necessarily a week",
    needsSchool: true,
    render: (school) => <CyclesAndCycleDays school={school} />,
  },
  {
    id: "periods",
    label: "Bell schedules and periods",
    blurb: "The shape of one day",
    needsSchool: true,
    render: (school) => <PeriodStructuresAndPeriods school={school} />,
  },
  {
    id: "calendar",
    label: "Calendar mapping",
    blurb: "Which real dates are which cycle day",
    needsSchool: true,
    render: (school) => <CalendarMapping school={school} />,
  },
  {
    id: "grid",
    label: "Timetable grid preview",
    blurb: "Every slot a lesson could occupy",
    needsSchool: true,
    render: (school) => <GridPreviewSection school={school} />,
  },
];

export function SchoolSetup() {
  const load = useCallback(() => getSchool(), []);
  const { value: school, error, reload } = useValue<School | null>(load);
  const [activeId, setActiveId] = useState<string>("school");

  const active = SECTIONS.find((section) => section.id === activeId) ?? SECTIONS[0];
  const ready = school !== undefined && school !== null;

  return (
    <div className="setup">
      <nav className="setupNav" aria-label="School setup sections">
        <h2 className="setupNavHead">School setup</h2>
        <ol>
          {SECTIONS.map((section, index) => {
            const locked = section.needsSchool && !ready;
            return (
              <li key={section.id}>
                <button
                  type="button"
                  className={section.id === activeId ? "navItem navItem--on" : "navItem"}
                  aria-current={section.id === activeId ? "page" : undefined}
                  disabled={locked}
                  title={locked ? "Describe the school first" : undefined}
                  onClick={() => setActiveId(section.id)}
                >
                  <span className="navIndex" aria-hidden="true">
                    {index + 1}
                  </span>
                  <span className="navText">
                    <span className="navLabel">{section.label}</span>
                    <span className="navBlurb">{locked ? "Needs the school record" : section.blurb}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      </nav>

      <div className="setupBody">
        {error && <Banner tone="error">{error}</Banner>}
        {school === undefined ? (
          <Loading what="the project" />
        ) : active?.id === "school" ? (
          <SchoolDetails onChanged={reload} />
        ) : school === null ? (
          <Banner tone="warning">
            This section needs the school record. Open <strong>School details</strong> and
            describe the school first.
          </Banner>
        ) : (
          (active?.render(school) ?? null)
        )}
      </div>
    </div>
  );
}
