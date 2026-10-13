/**
 * The School Setup workflow: navigation, and the eleven sections behind it.
 *
 * ## Why a sidebar of many sections rather than one long form
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
 * There is exactly one school per campus file and it owns everything else, so until it exists
 * the only thing to do is create it. Rather than letting seven sections each discover
 * that, the shell holds the school record and shows the first-run state once.
 */
import { useCallback, useEffect, useState, type ReactNode } from "react";

import type { School } from "@evara/domain";
import { listSchools } from "../../ipc/setup";
import { Banner, Loading } from "../ui/Shell";
import { AcademicYearsAndTerms } from "./AcademicYearsAndTerms";
import { CalendarMapping } from "./CalendarMapping";
import { CampusesAndBuildings } from "./CampusesAndBuildings";
import { CyclesAndCycleDays } from "./CyclesAndCycleDays";
import { GridPreviewSection } from "./GridPreviewSection";
import { PeriodStructuresAndPeriods } from "./PeriodStructuresAndPeriods";
import { RoomsAndRoomTypes } from "./RoomsAndRoomTypes";
import { SchoolDetails } from "./SchoolDetails";
import { StudentGroups } from "./StudentGroups";
import { StudentsAndYearLevels } from "./StudentsAndYearLevels";
import { Teachers } from "./Teachers";
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
 * The eleven sections, in setup order.
 *
 * Exported so a test can assert the list rather than the rendering: the set of sections
 * and their order is a product decision, and a silently dropped one would otherwise only
 * be noticed by someone looking for it.
 *
 * Resources had a section here and no longer does. Nothing schedules against a resource
 * until constraints arrive, so configuring them was work with no effect on anything the
 * user could see, sitting in the middle of a workflow whose every other step feeds the
 * grid. The `resource` table, its repository and its commands are all still there
 * ([ROADMAP](../../../../../docs/ROADMAP.md) Phase 2 owns them) — only the step is gone.
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
  // People come after the grid because nothing about them depends on it, and because the
  // grid is the first point at which a school can see that the structure it described is
  // the one it meant. Entering staff before that is work done on an unverified foundation.
  {
    id: "teachers",
    label: "Teachers",
    blurb: "The staff a timetable is built around",
    needsSchool: true,
    render: (school) => <Teachers school={school} />,
  },
  {
    id: "students",
    label: "Students and year levels",
    blurb: "Optional — a timetable can be built without them",
    needsSchool: true,
    render: (school) => <StudentsAndYearLevels school={school} />,
  },
  {
    id: "groups",
    label: "Student groups",
    blurb: "Whoever a lesson is taught to",
    needsSchool: true,
    render: (school) => <StudentGroups school={school} />,
  },
];

/**
 * Chooses which school the sections below are about.
 *
 * Shown only when the project holds more than one, so a project with a single school looks
 * and behaves exactly as it did before `V12`. This is the minimum Stage 0A needs to be
 * correct rather than the school-management interface Stage 0C will build: creating,
 * renaming and deleting a school all live in School details for now.
 */
function SchoolPicker(props: {
  schools: readonly School[];
  activeId: string | null;
  onSelect: (id: string) => void;
}) {
  const { schools, activeId, onSelect } = props;
  return (
    <div className="setupSchoolPicker">
      <label className="setupSchoolPickerLabel" htmlFor="setup-active-school">
        School
      </label>
      <select
        id="setup-active-school"
        className="setupSchoolPickerSelect"
        value={activeId ?? ""}
        onChange={(event) => onSelect(event.target.value)}
      >
        {schools.map((school) => (
          <option key={school.id} value={school.id}>
            {school.code ? `${school.name} (${school.code})` : school.name}
          </option>
        ))}
      </select>
    </div>
  );
}

export function SchoolSetup() {
  const load = useCallback(() => listSchools(), []);
  const { value: schools, error, reload } = useValue<School[]>(load);
  const [activeId, setActiveId] = useState<string>("school");
  // Which school every section below is about. Held here rather than in each section so
  // there is exactly one answer at any moment: a selector that scoped some screens and
  // not others would be worse than no selector at all.
  const [activeSchoolId, setActiveSchoolId] = useState<string | null>(null);

  // The active school follows the project: it settles on the only school when there is
  // one, and survives a reload as long as that school still exists.
  useEffect(() => {
    if (schools === undefined) return;
    if (schools.length === 0) {
      setActiveSchoolId(null);
      return;
    }
    const stillThere = schools.some((candidate) => candidate.id === activeSchoolId);
    if (!stillThere) setActiveSchoolId(schools[0]?.id ?? null);
  }, [schools, activeSchoolId]);

  const school = schools?.find((candidate) => candidate.id === activeSchoolId) ?? null;
  const active = SECTIONS.find((section) => section.id === activeId) ?? SECTIONS[0];
  const ready = school !== null;

  return (
    <div className="setup">
      <nav className="setupNav" aria-label="School setup sections">
        <h2 className="setupNavHead">School setup</h2>
        {schools !== undefined && schools.length > 1 && (
          <SchoolPicker
            schools={schools}
            activeId={activeSchoolId}
            onSelect={setActiveSchoolId}
          />
        )}
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
        {schools === undefined ? (
          <Loading what="this campus file" />
        ) : active?.id === "school" ? (
          <SchoolDetails school={school} onChanged={reload} />
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
