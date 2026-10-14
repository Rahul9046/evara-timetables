/**
 * Section 3 — Campuses and Buildings.
 *
 * A campus is owned by the school. A building is **shared**: it has a home campus, the one
 * it was created on, and may serve any number of others. One physical building is one row
 * with one identifier, listed by every campus it serves — see
 * [ADR 0013](../../../../../docs/adr/0013-buildings-shared-across-campuses.md).
 *
 * Every deletion edge here is `RESTRICT`, so nothing can be removed out from under its
 * children: the delete fails and says what is in the way. Buildings are the exception on
 * the other side — a room's building is an optional classification, so deleting a building
 * *detaches* its rooms rather than deleting them, and the confirmation says so.
 *
 * ## Sharing reaches across schools, and the picker now says so
 *
 * `building_campus` associates a building with **campuses** and has never mentioned a
 * school — ADR 0019 left it that way deliberately, because a sports centre between two
 * institutions on one site is the arrangement multi-school projects exist for. Until
 * Stage 0C the share picker offered only the active school's campuses, which made a
 * capability the model has had since `V12` unreachable from the interface. It now offers
 * every campus in the campus file, each labelled with the school that owns it.
 *
 * Ownership and access are therefore different things and are shown as different things.
 * The home campus is marked, and a building homed on another school's campus is marked as
 * shared *to* this one — which matters because the two schools can do different things
 * with it:
 *
 * | | Rename | Stop sharing with a campus | Delete |
 * | --- | --- | --- | --- |
 * | Home school | yes | yes | only while nobody else shares it |
 * | A school it is shared with | yes — it is one building, and a rename is one fact about it | its own campuses | no |
 *
 * ## Withdrawing a share is not deleting anything
 *
 * Removing a campus from a building is reversible and takes nothing with it. That is why
 * it is refused outright while rooms on that campus are still in the building, rather than
 * detaching them the way a delete does: the user asked to stop sharing a building, not to
 * empty a campus's rooms out of it. The backend decides, names the rooms in the way, and
 * this screen shows what it said.
 */
import { useCallback, useMemo, useState } from "react";

import type { Building, BuildingWithCampuses, Campus, School } from "@evara/domain";
import {
  createBuilding,
  createCampus,
  deleteBuilding,
  deleteCampus,
  listBuildings,
  listCampuses,
  shareBuilding,
  unshareBuilding,
  updateBuilding,
  updateCampus,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner } from "../ui/Shell";
import { list, text, textOrNull, validateBuilding, validateCampus } from "../validation";
import { ParentPicker } from "./ParentPicker";
import {
  campusChoiceLabel,
  campusLabel,
  useCampusIndex,
  useProjectCampuses,
  type CampusOfSchool,
} from "./project";
import type { SetupContext } from "./SchoolSetup";
import { useList } from "./useList";

/**
 * One row of the buildings table.
 *
 * The building's own fields lifted alongside the campuses it serves, because `CrudPanel`
 * keys rows by `id` and `BuildingWithCampuses` keeps it one level down.
 */
type BuildingRow = Building & { readonly campusIds: readonly string[] };

function toRow(entry: BuildingWithCampuses): BuildingRow {
  return { ...entry.building, campusIds: entry.campusIds };
}

/** What `createBuilding` and `updateBuilding` are given. */
interface BuildingDraft {
  readonly campusId: string;
  readonly name: string;
  /** Every campus the building should serve *besides* its home campus. */
  readonly sharedCampusIds: readonly string[];
}

export function CampusesAndBuildings(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const schools = props.context?.schools ?? [school];
  const many = schools.length > 1;
  const [campusId, setCampusId] = useState<string | null>(null);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  // Every campus in the campus file, for the share picker: a building may serve a campus
  // of another school, and a picker that could not offer one would hide the capability.
  const projectCampuses = useProjectCampuses(schools);
  const campusOf = useCampusIndex(projectCampuses.all);

  const campusSpec = useMemo<CrudSpec<Campus, Parameters<typeof createCampus>[0]>>(
    () => ({
      noun: "campus",
      plural: "campuses",
      title: "Campuses",
      describe: (row) => `${row.name} (${row.code})`,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Code", render: (row) => <code>{row.code}</code> },
        { header: "Address", render: (row) => row.address ?? <Absent /> },
      ],
      fields: [
        { kind: "text", key: "name", label: "Campus name", required: true },
        {
          kind: "text",
          key: "code",
          label: "Code",
          required: true,
          hint: "Short and unique within the school — it appears in reports where a full name will not fit.",
        },
        { kind: "textarea", key: "address", label: "Address", hint: "Optional." },
      ],
      validate: validateCampus,
      toDraft: (row) => ({
        name: row?.name ?? "",
        code: row?.code ?? "",
        address: row?.address ?? "",
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        name: text(draft, "name"),
        code: text(draft, "code"),
        address: textOrNull(draft, "address"),
      }),
      list: () => listCampuses(school.id),
      create: createCampus,
      update: updateCampus,
      remove: (id) => deleteCampus(school.id, id),
      emptyHint:
        "A school needs at least one campus before rooms or bell schedules can be recorded against it. A single-site school has exactly one.",
      deleteNote: () => (
        <p className="muted">
          A campus that still has buildings, rooms or resources based on it cannot be
          deleted, and nor can one that a building or a room is shared with. Move, remove or
          unshare those first. Resources another school merely <em>lent</em> to this campus
          are withdrawn automatically — a campus that is closing cannot host anything.
        </p>
      ),
    }),
    [school.id],
  );

  const loadBuildings = useCallback(
    () =>
      campusId
        ? listBuildings(campusId).then((entries) => entries.map(toRow))
        : Promise.resolve([]),
    [campusId],
  );

  const buildingSpec = useMemo<CrudSpec<BuildingRow, BuildingDraft>>(() => {
    const all = projectCampuses.all ?? [];

    /**
     * Campus labels, school-qualified when the campus file holds more than one school.
     *
     * One map for the table and the confirmations, so a campus is named the same way
     * wherever it appears.
     */
    const campusName = new Map(
      all.map((entry) => [entry.campus.id, campusLabel(entry, many)]),
    );

    /** A building's shared campuses, i.e. everything bar its home campus. */
    const sharedOf = (row: BuildingRow): string[] =>
      row.campusIds.filter((id) => id !== row.campusId);

    /** Which other schools can reach this building. */
    const sharersOf = (row: BuildingRow): readonly CampusOfSchool[] => {
      const owner = campusOf.get(row.campusId)?.school.id ?? school.id;
      return row.campusIds
        .map((id) => campusOf.get(id))
        .filter((entry): entry is CampusOfSchool => entry !== undefined)
        .filter((entry) => entry.school.id !== owner);
    };

    return {
      noun: "building",
      plural: "buildings",
      title: "Buildings on this campus",
      describe: (row) => row.name,
      columns: [
        { header: "Name", render: (row) => row.name },
        {
          header: "Campuses",
          render: (row) => (
            <CampusShareList
              campusIds={row.campusIds}
              homeId={row.campusId}
              campusName={campusName}
              viewing={campusId}
            />
          ),
        },
        ...(many
          ? [
              {
                header: "Ownership",
                render: (row: BuildingRow) => {
                  const owner = campusOf.get(row.campusId)?.school;
                  if (owner === undefined || owner.id === school.id) {
                    return sharersOf(row).length === 0 ? (
                      "This school"
                    ) : (
                      <>
                        <span className="chipBadge">lent</span> This school
                      </>
                    );
                  }
                  return (
                    <>
                      <span className="chipBadge">shared</span> {owner.name}
                    </>
                  );
                },
              },
            ]
          : []),
      ],
      fields: (draft) => {
        // Its home campus is never a *share*, so it is not offered as one: the backend
        // refuses withdrawing the home campus, because a building must never reach zero
        // campuses and vanish from every screen. `homeCampusId` is carried in the draft
        // precisely so this list can exclude the right campus on an edit as well as on a
        // create.
        const home = text(draft, "homeCampusId") || (campusId ?? "");
        const shareChoices: Choice[] = all
          .filter((entry) => entry.campus.id !== home)
          .map((entry) => ({ value: entry.campus.id, label: campusChoiceLabel(entry, many) }));

        return [
          {
            kind: "text",
            key: "name",
            label: "Building name",
            required: true,
            hint: "Unique on every campus it serves.",
          },
          {
            kind: "multicombobox",
            key: "sharedCampusIds",
            label: "Shared with campuses",
            choices: shareChoices,
            placeholder: shareChoices.length === 0 ? "No other campuses" : "Search campuses",
            emptyLabel: "No campus matches that",
            noneLabel: "Its home campus only",
            hint:
              shareChoices.length === 0
                ? "A campus file with one campus has nothing to share with. Add another campus first."
                : many
                  ? "For one physical building that serves more than one campus — including a campus of another school in this campus file. It stays a single building: renaming it from any campus renames it everywhere."
                  : "For one physical building that serves more than one campus. It stays a single building — renaming it from any campus renames it everywhere.",
          },
        ];
      },
      validate: validateBuilding,
      toDraft: (row) => ({
        name: row?.name ?? "",
        sharedCampusIds: row ? sharedOf(row) : [],
        // Not a field: the home campus is immutable and is carried so the share picker
        // knows which campus is not a share. A create is homed on the campus being viewed.
        homeCampusId: row?.campusId ?? campusId ?? "",
      }),
      toInput: (draft) => ({
        campusId: campusId ?? "",
        name: text(draft, "name"),
        sharedCampusIds: list(draft, "sharedCampusIds"),
      }),
      list: loadBuildings,
      create: (input) =>
        createBuilding(
          { campusId: input.campusId, name: input.name },
          [...input.sharedCampusIds],
        ).then(toNewRow(input)),
      // A rename and a change to the share set are one edit to the user, so they are one
      // call here. The repository is the authority on each step and refuses the ones that
      // are not allowed; a refusal leaves the earlier steps applied, which is why the
      // panel re-reads the list after every mutation rather than patching it.
      update: async (id, input) => {
        const existing = (await loadBuildings()).find((row) => row.id === id);
        const home = existing?.campusId ?? input.campusId;
        const building = await updateBuilding(id, {
          campusId: home,
          name: input.name,
        });

        const before = new Set(existing?.campusIds ?? []);
        before.delete(home);
        const after = new Set(input.sharedCampusIds);

        for (const campus of after) {
          if (!before.has(campus)) await shareBuilding(id, campus);
        }
        for (const campus of before) {
          if (!after.has(campus)) await unshareBuilding(school.id, id, campus);
        }

        return { ...building, campusIds: [home, ...after] };
      },
      remove: (id) => deleteBuilding(school.id, id),
      // Withdrawing a share is reversible but it is not nothing: a campus loses a building
      // its rooms may be organised around. Asking first is the same courtesy a delete gets.
      confirmSubmit: (draft, row) => {
        if (!row) return null;
        const after = new Set(list(draft, "sharedCampusIds"));
        const removed = sharedOf(row).filter((id) => !after.has(id));
        if (removed.length === 0) return null;

        return (
          <>
            <p>
              <strong>{row.name}</strong> will no longer be available to{" "}
              <strong>
                {removed.map((id) => campusName.get(id) ?? "another campus").join(", ")}
              </strong>
              .
            </p>
            <p className="muted">
              No rooms are deleted or detached. If rooms on those campuses are still in
              this building the change is refused, and the message says how many to move
              first.
            </p>
          </>
        );
      },
      deleteBlockedBy: (row) => {
        const owner = campusOf.get(row.campusId)?.school;
        if (owner !== undefined && owner.id !== school.id) {
          return (
            <>
              <p>
                <strong>{row.name}</strong> is on{" "}
                <strong>{owner.name}</strong>&rsquo;s campus, which owns it. Only the school
                a building was created on may delete it.
              </p>
              <p className="muted">
                This school can stop sharing it: clear this campus from{" "}
                <strong>Shared with campuses</strong> and the building is untouched —
                every other campus keeps it.
              </p>
            </>
          );
        }
        const sharers = sharersOf(row);
        if (sharers.length === 0) return null;
        const names = [...new Set(sharers.map((entry) => entry.school.name))];
        return (
          <>
            <p>
              <strong>{names.join(", ")}</strong>{" "}
              {names.length === 1 ? "shares" : "share"} this building, through{" "}
              {sharers.map((entry) => entry.campus.name).join(", ")}.
            </p>
            <p className="muted">
              Owning a building is not authority to destroy something another school is
              timetabling into. Withdraw those campuses from the share list first — an
              ordinary edit, which that school can see — and the delete is then offered.
            </p>
          </>
        );
      },
      emptyHint:
        "Buildings are optional. Add them only if you want rooms grouped by building — a small school can leave this empty and put rooms directly on the campus.",
      deleteNote: (row) => (
        <>
          <p className="muted">
            Rooms in this building are <strong>not</strong> deleted. They stay on their
            campus and simply stop naming a building.
          </p>
          {row.campusIds.length > 1 && (
            <p className="warning">
              This building serves{" "}
              {row.campusIds.filter((id) => id !== campusId).length} other campus
              {row.campusIds.length > 2 ? "es" : ""}. Deleting it removes it from all of
              them, not just this one.
            </p>
          )}
        </>
      ),
      blockedBy: campusId === null ? "Select a campus first." : null,
    };
  }, [campusId, campusOf, loadBuildings, many, projectCampuses.all, school.id]);

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Campuses and buildings</h2>
        <p>
          A campus is a physical site belonging to this school. Buildings group the rooms on
          a site and are optional — rooms can sit directly on a campus. One building can
          serve several campuses: a sports centre between two sites is one building, not
          two.
        </p>
        {many && (
          <p className="muted">
            Campuses of the other schools in this campus file can be shared with too. A
            building shared across schools stays one building with one identifier, which is
            what will make &ldquo;is the hall free at ten?&rdquo; have a single answer.
          </p>
        )}
      </header>

      {projectCampuses.error && (
        <Banner tone="warning">
          Some campuses could not be read, so the sharing list may be incomplete:{" "}
          {projectCampuses.error}
        </Banner>
      )}

      <CrudPanel
        spec={campusSpec}
        reloadKey={school.id}
        onChanged={() => {
          campuses.reload();
          projectCampuses.reload();
        }}
      />

      <hr />

      <ParentPicker
        legend="Campus"
        items={
          campuses.items?.map((campus) => ({
            id: campus.id,
            label: campus.name,
            detail: campus.code,
          })) ?? null
        }
        error={campuses.error}
        selectedId={campusId}
        onSelect={setCampusId}
        empty={{
          title: "No campuses yet",
          hint: "Add a campus above, then its buildings can be recorded here.",
        }}
      >
        <CrudPanel spec={buildingSpec} reloadKey={`${school.id}:${campusId ?? ""}`} />
      </ParentPicker>
    </div>
  );
}

/**
 * The campuses one shared thing serves, marking the ones other than the campus being
 * viewed and flagging its home campus.
 *
 * Shared by buildings and rooms, because "which campuses is this on" reads the same way
 * for both and the model is the same shape — a home campus plus any number of shares.
 * A row that serves only the current campus says so plainly rather than repeating the
 * campus name the user already selected above.
 *
 * The names come from a map the caller builds, which is how a campus of another school
 * gets its school's name attached without this component knowing anything about schools.
 */
export function CampusShareList(props: {
  /** Every campus the row serves, home included. */
  campusIds: readonly string[];
  /** The campus the row was created on. */
  homeId: string;
  campusName: Map<string, string>;
  /** The campus currently selected above, which is not worth naming again. */
  viewing: string | null;
  /** What to say when this campus is the only one. */
  soleLabel?: string;
}) {
  const { campusIds, homeId, campusName, viewing, soleLabel } = props;
  const others = campusIds.filter((id) => id !== viewing);

  if (others.length === 0) {
    return <span className="absent">{soleLabel ?? "This campus only"}</span>;
  }

  return (
    <span className="shareList">
      <span className="chipBadge">shared</span>
      {others.map((id) => (
        <span key={id} className="shareName">
          {campusName.get(id) ?? "another campus"}
          {id === homeId && (
            <span className="muted" title="The campus this was created on">
              {" "}
              (home)
            </span>
          )}
        </span>
      ))}
    </span>
  );
}

/**
 * Builds the row a create resolves to.
 *
 * `CrudPanel` re-reads the list straight afterwards, so this value is never what gets
 * rendered — it exists so `create` satisfies the spec's return type without inventing a
 * second source of truth.
 */
function toNewRow(input: BuildingDraft): (created: Building) => BuildingRow {
  return (created) => ({
    ...created,
    campusIds: [created.campusId, ...input.sharedCampusIds],
  });
}

/** A nullable column with nothing in it. Styled so a blank cell is deliberate. */
export function Absent() {
  return <span className="absent">—</span>;
}
