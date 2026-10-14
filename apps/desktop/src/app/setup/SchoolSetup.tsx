/**
 * The School Setup workflow: navigation, the school context, and the sections behind it.
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
 * ## The project, the school, and the sections
 *
 * Since `V12` a campus file holds several schools, and since Stage 0C there is a screen
 * for them: **Schools** is section 1 and is about the project — which institutions it
 * describes, which one is selected, creating and deleting them. Everything after it is
 * about *one* school, the selected one, and until one exists there is nothing for those
 * sections to be about.
 *
 * ## The selected school is held here, once
 *
 * One answer at any moment. A selector that scoped some screens and not others would be
 * worse than no selector at all, so the shell owns it and hands it down.
 *
 * ### Switching schools remounts the sections, on purpose
 *
 * `key={activeSchoolId}` on the section body is not a rendering convenience — it is the
 * stale-state guarantee. Every section holds state that is *about* a school: an open edit
 * form, a pending delete confirmation, a selected campus or group, a half-typed enrolment.
 * Re-rendering with a new `school` prop re-reads the lists, but it leaves that state
 * standing, and a form opened under Northgate that submits after a switch to Southmoor
 * would write one school's values under the other's name.
 *
 * Remounting discards all of it. That is deliberately a blunt instrument: the alternative
 * is for every section to remember to clear every piece of per-school state, which is a
 * rule that holds until the next section is written. The backend refuses a cross-school
 * write regardless (`DbError::WrongSchool`), and both layers are wanted — the key stops
 * the user being shown a plausible wrong screen, the backend stops the write.
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
import { ResourcesAndSharing } from "./ResourcesAndSharing";
import { RoomsAndRoomTypes } from "./RoomsAndRoomTypes";
import { SchoolDetails } from "./SchoolDetails";
import { Schools } from "./Schools";
import { StudentGroups } from "./StudentGroups";
import { Students } from "./Students";
import { Teachers } from "./Teachers";
import { YearLevels } from "./YearLevels";
import { useValue } from "./useList";

/**
 * What a section may know about the project it sits in.
 *
 * Deliberately narrow. A section is about one school and is given that school directly;
 * this is for the few things that are genuinely project-level and that Stage 0C made
 * necessary: naming the school that holds a visiting person's identity, offering a
 * transfer's destination, and sending the user to the school where an edit is allowed.
 *
 * `selectSchool` is the only way a section changes the selection, so switching always goes
 * through the shell and always remounts — see this module's header.
 */
export interface SetupContext {
  /** Every school in the project, by name. */
  readonly schools: readonly School[];
  /** Makes another school the selected one. Remounts every section. */
  readonly selectSchool: (id: string) => void;
  /** Opens another setup section. */
  readonly openSection: (id: string) => void;
  /** Re-reads the project's schools, after one is created, renamed or removed. */
  readonly reloadSchools: () => void;
}

/** One entry in the setup navigation. */
interface Section {
  /**
   * Stable identifier, and the only thing anything else may address a section by.
   *
   * Never a position. Stage A reordered the list and split one section in two, and every
   * caller survived it — `SetupContext.openSection`, the empty states that navigate, the
   * tests — because none of them counts. An index would have made the reorder a silent
   * behaviour change in six places.
   */
  readonly id: string;
  readonly label: string;
  /** One line in the sidebar, so the user can tell the sections apart at a glance. */
  readonly blurb: string;
  /** Whether this section can be used before a school record exists. */
  readonly needsSchool: boolean;
  /**
   * Announced in the navigation, not yet built.
   *
   * Exactly one entry carries this: **Lessons**, which Stage C implements after Stage B
   * brings Subjects and Departments. It is in the list because the *order* is the thing
   * Stage A is for — somebody configuring a school should be able to see that lessons come
   * after year levels and before teachers — and a section that simply appeared later would
   * leave the workflow looking shorter than it is.
   *
   * The entry is disabled and has no `render`. An empty functional screen would be worse
   * than an absent one: it would invite a user to look for a feature that does not exist,
   * and it would need deleting rather than filling in when Stage C arrives.
   */
  readonly comingLater?: string;
  /** Absent for a {@link comingLater} entry, which has nothing to render. */
  readonly render?: (school: School, context: SetupContext) => ReactNode;
}

/**
 * The fifteen sections, in the order a school is actually configured.
 *
 * Exported so a test can assert the list rather than the rendering: the set of sections and
 * their order is a product decision, and a silently dropped or reordered one would
 * otherwise only be noticed by somebody looking for it.
 *
 * ## Stage A reordered this, and the order now means something different
 *
 * It used to be a *dependency* order, and people came last because nothing about them
 * depends on the grid. That reading was defensible and it produced a workflow nobody
 * follows: a school setting itself up knows who it teaches long before it has decided what
 * its rotation looks like, and being asked for a six-day cycle before being asked for a
 * year level is being asked the abstract question first.
 *
 * So the order is now **what a school knows, then when it happens**:
 *
 * | | Sections | |
 * | --- | --- | --- |
 * | 1–2 | Schools, School details | which institution |
 * | 3–5 | Campuses, Rooms, Resources | where, and with what |
 * | 6–7 | Academic years, Year levels | the shape of a school year |
 * | 8 | Lessons | *Stage C* — what is taught |
 * | 9–11 | Teachers, Students, Student groups | who |
 * | 12–15 | Cycles, Bell schedules, Calendar, Grid preview | when |
 *
 * **The one genuine dependency this inverts is availability**, which is set against a
 * cycle's days and a bell schedule's periods and therefore appears on the Teachers, Rooms
 * and Student groups screens before the time model exists. It degrades correctly rather
 * than silently: the grid says "No cycle yet", names the section, and now offers a button
 * that opens it. A school can enter its staff, come back, and fill the grid in afterwards —
 * which is the order it was going to work in anyway.
 *
 * Year levels left Students and became section 7 in the same change, because a year level
 * is part of the structure of a school year rather than an attribute of the children in it,
 * and because Lessons and the Excel imports of Stage D both reference year levels directly.
 */
export const SECTIONS: readonly Section[] = [
  {
    id: "schools",
    label: "Schools",
    blurb: "Every institution this campus file describes",
    needsSchool: false,
    render: () => null, // Rendered specially; see `SchoolSetup`.
  },
  {
    id: "school",
    label: "School details",
    blurb: "Name, code, time zone, locale",
    needsSchool: false,
    render: () => null, // Rendered specially; see `SchoolSetup`.
  },
  // Where the school is, and what is in it.
  {
    id: "campuses",
    label: "Campuses and buildings",
    blurb: "Sites, and how they are divided",
    needsSchool: true,
    render: (school, context) => (
      <CampusesAndBuildings school={school} context={context} />
    ),
  },
  {
    id: "rooms",
    label: "Rooms and room types",
    blurb: "Teaching spaces and their kinds",
    needsSchool: true,
    render: (school, context) => <RoomsAndRoomTypes school={school} context={context} />,
  },
  {
    id: "resources",
    label: "Resources",
    blurb: "Shared equipment — and who else may use it",
    needsSchool: true,
    render: (school, context) => (
      <ResourcesAndSharing school={school} context={context} />
    ),
  },
  // The shape of a school year: how long it runs, and what stages it has.
  {
    id: "years",
    label: "Academic years and terms",
    blurb: "The year, and its divisions",
    needsSchool: true,
    render: (school) => <AcademicYearsAndTerms school={school} />,
  },
  {
    id: "yearLevels",
    label: "Year levels",
    blurb: "The stages this school teaches",
    needsSchool: true,
    render: (school) => <YearLevels school={school} />,
  },
  // What is taught. Stage C; see `Section.comingLater`.
  {
    id: "lessons",
    label: "Lessons",
    blurb: "What is taught, and how often",
    needsSchool: true,
    comingLater:
      "Lessons arrive in Stage C, after Subjects and Departments. The order is shown now so the rest of the workflow reads correctly.",
  },
  // Who. After the year levels they are placed in, and before the rotation they are
  // timetabled into.
  {
    id: "teachers",
    label: "Teachers",
    blurb: "The staff a timetable is built around",
    needsSchool: true,
    render: (school, context) => <Teachers school={school} context={context} />,
  },
  {
    id: "students",
    label: "Students",
    blurb: "Optional — a timetable can be built without them",
    needsSchool: true,
    render: (school, context) => <Students school={school} context={context} />,
  },
  {
    id: "groups",
    label: "Student groups",
    blurb: "Whoever a lesson is taught to",
    needsSchool: true,
    render: (school, context) => <StudentGroups school={school} context={context} />,
  },
  // When. The time model last, because it is the part a school verifies against the grid
  // rather than the part it already knows.
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

/**
 * Chooses which school the sections below are about.
 *
 * Shown only when the project holds more than one, so a project with a single school looks
 * and behaves exactly as it did before `V12`: there is no choice to make, and a selector
 * offering one option is furniture.
 *
 * The value is the school's **identifier**, never its name. A name is unique and would
 * work until the day somebody renames a school mid-session, at which point a name-keyed
 * selection silently points at nothing.
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
  const [activeId, setActiveId] = useState<string>("schools");
  // Which school every section below is about. Held here rather than in each section so
  // there is exactly one answer at any moment.
  const [activeSchoolId, setActiveSchoolId] = useState<string | null>(null);

  // The active school follows the project: it settles on the only school when there is
  // one, and survives a reload as long as that school still exists. A school that has
  // just been deleted therefore hands the selection to another one rather than leaving
  // every section pointed at a row that is gone.
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
  const ready = school !== null;
  const many = (schools?.length ?? 0) > 1;

  /**
   * The section on screen.
   *
   * Resolved by identifier, and a request for one that cannot be shown falls back to the
   * first rather than rendering nothing. An unknown identifier is a caller's mistake; a
   * {@link Section.comingLater} one is reachable only by a disabled button, so neither
   * should happen — but "neither should happen" is not the same as "cannot", and a blank
   * body would be the worst way to find out.
   */
  const requested = SECTIONS.find((section) => section.id === activeId);
  const active =
    requested !== undefined && requested.comingLater === undefined
      ? requested
      : SECTIONS[0];

  const context: SetupContext = {
    schools: schools ?? [],
    selectSchool: setActiveSchoolId,
    openSection: setActiveId,
    reloadSchools: reload,
  };

  return (
    <div className="setup">
      <nav className="setupNav" aria-label="School setup sections">
        <h2 className="setupNavHead">School setup</h2>
        {many && (
          <SchoolPicker
            schools={schools ?? []}
            activeId={activeSchoolId}
            onSelect={setActiveSchoolId}
          />
        )}
        <ol>
          {SECTIONS.map((section, index) => {
            // Two different reasons a step cannot be opened, and they must not be
            // conflated: one is temporary and about this campus file, the other is about
            // the product and will not change by adding a school.
            const later = section.comingLater ?? null;
            const locked = later === null && section.needsSchool && !ready;
            return (
              <li key={section.id}>
                <button
                  type="button"
                  className={
                    section.id === activeId
                      ? "navItem navItem--on"
                      : later !== null
                        ? "navItem navItem--later"
                        : "navItem"
                  }
                  aria-current={section.id === activeId ? "page" : undefined}
                  disabled={locked || later !== null}
                  title={locked ? "Add a school first" : (later ?? undefined)}
                  onClick={() => setActiveId(section.id)}
                >
                  <span className="navIndex" aria-hidden="true">
                    {index + 1}
                  </span>
                  <span className="navText">
                    <span className="navLabel">{section.label}</span>
                    <span className="navBlurb">
                      {later !== null
                        ? "Coming later"
                        : locked
                          ? "Needs a school"
                          : section.blurb}
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      </nav>

      <div className="setupBody">
        {error && <Banner tone="error">{error}</Banner>}
        {/* Which school the body is about, stated above it whenever there is more than
            one. Without this a screen is identified only by the sidebar selector, which
            is the one thing a user scanning a table is not looking at — and "data from
            School A under School B's heading" is the failure this stage exists to
            prevent. Not shown for the project-level Schools section, which is about all
            of them. */}
        {many && school !== null && active?.id !== "schools" && (
          <p className="setupScope" role="note">
            <span className="eyebrow">SCHOOL</span>
            <strong>{school.code ? `${school.name} (${school.code})` : school.name}</strong>
          </p>
        )}
        {schools === undefined ? (
          <Loading what="this campus file" />
        ) : active?.id === "schools" ? (
          <Schools
            activeSchoolId={activeSchoolId}
            onSelect={setActiveSchoolId}
            onOpenSection={setActiveId}
            onChanged={reload}
          />
        ) : active?.id === "school" ? (
          <SchoolDetails
            key={activeSchoolId ?? "none"}
            school={school}
            onChanged={reload}
            onOpenSchools={() => setActiveId("schools")}
          />
        ) : school === null ? (
          <Banner tone="warning">
            This section needs a school. Open <strong>Schools</strong> and add one first.
          </Banner>
        ) : (
          // Keyed by school, so switching discards every form, dialogue and selection the
          // previous school's screen was holding. See this module's header.
          <div key={school.id}>{active?.render?.(school, context) ?? null}</div>
        )}
      </div>
    </div>
  );
}
