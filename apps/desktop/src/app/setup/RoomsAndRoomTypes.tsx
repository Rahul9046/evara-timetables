/**
 * Section 3 — Rooms and Room Types.
 *
 * ## The two nullable references are the interesting part
 *
 * A room's `buildingId` and `roomTypeId` are both optional, and both stay editable and
 * clearable here — the brief asks for exactly that, and the model backs it: deleting a
 * building or a room type does not delete the rooms, it clears the reference through a
 * real repository update so the room's revision stays truthful.
 *
 * So "No building" and "Unclassified" are offered as genuine choices in both selects
 * rather than as a disabled placeholder. A room with no type is a normal state, not an
 * incomplete one.
 *
 * A building choice is restricted to buildings available to the room's own campus — its
 * own and any shared with it — because the
 * schema refuses the cross-campus case with a trigger. Showing an option that is
 * guaranteed to fail is not a choice.
 *
 * ## A room is shared, like a building
 *
 * One physical room can serve several campuses: a sports hall on a shared site, a lecture
 * theatre two campuses timetable into. It stays **one room with one identifier**, listed
 * by every campus it serves — see
 * [ADR 0014](../../../../../docs/adr/0014-rooms-shared-across-campuses.md). That identity
 * is the point: when scheduling arrives, the room is occupied or it is not, and no campus
 * gets its own private answer.
 *
 * Sharing a building does **not** share the rooms inside it, and sharing a room does not
 * share its building. The two settings are independent and this screen keeps them that
 * way. They meet at exactly one rule: a room may serve a campus only if its building —
 * when it has one — is available to that campus too. The repository owns that rule and
 * refuses with a message naming the building and the campus; this screen shows what it
 * said rather than second-guessing it, because the set of buildings a campus can reach is
 * not something a form can recompute correctly while the user is typing.
 *
 * ## Sharing reaches across schools, and the picker now says so
 *
 * `room_campus` associates a room with **campuses** and has never mentioned a school —
 * ADR 0019 left it that way deliberately, and `room-clash` stays a per-`room_id` rule for
 * the whole campus file. Until Stage 0C the share picker offered only the active school's
 * campuses, which made a capability the model has had since `V12` unreachable. It now
 * offers every campus in the campus file, each labelled with the school that owns it.
 *
 * Ownership and access are shown as different things, because the two schools can do
 * different things with the room: either may rename it — it is one room and a rename is
 * one fact about it — but only the school whose campus it was created on may delete it,
 * and only while nobody else shares it.
 *
 * ## Withdrawing a share is not deleting anything
 *
 * Removing a campus from a room is reversible and takes nothing with it. The room keeps
 * its identity and every other campus keeps it; only this campus stops listing it.
 */
import { useCallback, useMemo, useState } from "react";

import type {
  BuildingWithCampuses,
  Campus,
  Room,
  RoomType,
  RoomWithCampuses,
  School,
} from "@evara/domain";
import {
  createRoom,
  createRoomType,
  deleteRoom,
  deleteRoomType,
  listBuildings,
  listCampuses,
  listRoomTypes,
  listRooms,
  shareRoom,
  unshareRoom,
  updateRoom,
  updateRoomType,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import {
  flag,
  list,
  number,
  text,
  textOrNull,
  validateRoom,
  validateRoomType,
} from "../validation";
import { Banner } from "../ui/Shell";
import { Absent, CampusShareList } from "./CampusesAndBuildings";
import { ParentPicker } from "./ParentPicker";
import {
  campusChoiceLabel,
  campusLabel,
  useCampusIndex,
  useProjectCampuses,
  type CampusOfSchool,
} from "./project";
import type { SetupContext } from "./SchoolSetup";
import { AvailabilityPanel } from "./AvailabilityGrid";
import { useList } from "./useList";

/**
 * One row of the rooms table.
 *
 * The room's own fields lifted alongside the campuses it serves, because `CrudPanel` keys
 * rows by `id` and `RoomWithCampuses` keeps it one level down.
 */
type RoomRow = Room & { readonly campusIds: readonly string[] };

function toRow(entry: RoomWithCampuses): RoomRow {
  return { ...entry.room, campusIds: entry.campusIds };
}

/** What `createRoom` and `updateRoom` are given. */
interface RoomDraft {
  readonly campusId: string;
  readonly buildingId: string | null;
  readonly roomTypeId: string | null;
  readonly name: string;
  readonly code: string;
  readonly capacity: number;
  readonly isBookable: boolean;
  readonly notes: string | null;
  /** Every campus the room should serve *besides* its home campus. */
  readonly sharedCampusIds: readonly string[];
}

export function RoomsAndRoomTypes(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const schools = props.context?.schools ?? [school];
  const many = schools.length > 1;
  const [campusId, setCampusId] = useState<string | null>(null);

  const loadCampuses = useCallback(() => listCampuses(school.id), [school.id]);
  const campuses = useList<Campus>(loadCampuses);

  // Every campus in the campus file, for the share picker: a room may serve a campus of
  // another school, and a picker that could not offer one would hide the capability.
  const projectCampuses = useProjectCampuses(schools);
  const campusOf = useCampusIndex(projectCampuses.all);

  const loadRoomTypes = useCallback(() => listRoomTypes(school.id), [school.id]);
  const roomTypes = useList<RoomType>(loadRoomTypes);

  const loadBuildings = useCallback(
    () => (campusId ? listBuildings(campusId) : Promise.resolve([])),
    [campusId],
  );
  const buildings = useList<BuildingWithCampuses>(loadBuildings);

  const roomTypeSpec = useMemo<CrudSpec<RoomType, Parameters<typeof createRoomType>[0]>>(
    () => ({
      noun: "room type",
      plural: "room types",
      title: "Room types",
      describe: (row) => `${row.name} (${row.code})`,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Code", render: (row) => <code>{row.code}</code> },
      ],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Room type name",
          required: true,
          placeholder: "Science Lab",
        },
        { kind: "text", key: "code", label: "Code", required: true, placeholder: "LAB" },
      ],
      validate: validateRoomType,
      toDraft: (row) => ({ name: row?.name ?? "", code: row?.code ?? "" }),
      toInput: (draft) => ({
        schoolId: school.id,
        name: text(draft, "name"),
        code: text(draft, "code"),
      }),
      list: () => listRoomTypes(school.id),
      create: createRoomType,
      update: (row, input) => updateRoomType(row.id, row.rev, input),
      remove: (row) => deleteRoomType(school.id, row.id, row.rev),
      emptyHint:
        "Room types are optional. They become useful when a lesson needs a particular kind of space — a lab, a gym, a music room — rather than a specific room.",
      deleteNote: () => (
        <p className="muted">
          Rooms of this type are <strong>not</strong> deleted. They stay as they are and
          become unclassified.
        </p>
      ),
    }),
    [school.id],
  );

  const loadRooms = useCallback(
    () =>
      campusId
        ? listRooms(campusId).then((entries) => entries.map(toRow))
        : Promise.resolve([]),
    [campusId],
  );

  // The same rows again, held here rather than only inside `CrudPanel`, because the
  // availability picker below needs to offer them and `CrudPanel` keeps its list private.
  const rooms = useList<RoomRow>(loadRooms);

  const roomSpec = useMemo<CrudSpec<RoomRow, RoomDraft>>(() => {
    // Buildings *available to* this campus, which since ADR 0013 includes any shared
    // with it. Offering anything else would be offering a room a building its own campus
    // does not have, which the schema refuses anyway.
    const buildingChoices: Choice[] = (buildings.items ?? []).map((b) => ({
      value: b.building.id,
      label: b.campusIds.length > 1 ? `${b.building.name} (shared)` : b.building.name,
    }));
    const typeChoices: Choice[] = (roomTypes.items ?? []).map((t) => ({
      value: t.id,
      label: `${t.name} (${t.code})`,
    }));
    const buildingName = new Map(buildingChoices.map((c) => [c.value, c.label]));
    const typeName = new Map(typeChoices.map((c) => [c.value, c.label]));

    const all = projectCampuses.all ?? [];
    // School-qualified where that matters, so a campus is named the same way in the table
    // and in the confirmations.
    const campusName = new Map(
      all.map((entry) => [entry.campus.id, campusLabel(entry, many)]),
    );

    /** A room's shared campuses, i.e. everything bar its home campus. */
    const sharedOf = (row: RoomRow): string[] =>
      row.campusIds.filter((id) => id !== row.campusId);

    /** Which other schools can reach this room. */
    const sharersOf = (row: RoomRow): readonly CampusOfSchool[] => {
      const owner = campusOf.get(row.campusId)?.school.id ?? school.id;
      return row.campusIds
        .map((id) => campusOf.get(id))
        .filter((entry): entry is CampusOfSchool => entry !== undefined)
        .filter((entry) => entry.school.id !== owner);
    };

    return {
      noun: "room",
      plural: "rooms",
      title: "Rooms on this campus",
      describe: (row) => `${row.name} (${row.code})`,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Code", render: (row) => <code>{row.code}</code> },
        {
          header: "Building",
          render: (row) =>
            row.buildingId ? (buildingName.get(row.buildingId) ?? <Absent />) : <Absent />,
        },
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
        {
          header: "Type",
          render: (row) =>
            row.roomTypeId ? (typeName.get(row.roomTypeId) ?? <Absent />) : <Absent />,
        },
        { header: "Seats", numeric: true, render: (row) => row.capacity },
        {
          header: "Bookable",
          render: (row) =>
            row.isBookable ? <span className="yes">Yes</span> : <span className="no">No</span>,
        },
        ...(many
          ? [
              {
                header: "Ownership",
                render: (row: RoomRow) => {
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
        // refuses withdrawing the home campus, because a room must never reach zero
        // campuses and vanish from every screen. `homeCampusId` is carried in the draft
        // precisely so this list can exclude the right campus on an edit as well as on a
        // create.
        const home = text(draft, "homeCampusId") || (campusId ?? "");
        const shareChoices: Choice[] = all
          .filter((entry) => entry.campus.id !== home)
          .map((entry) => ({ value: entry.campus.id, label: campusChoiceLabel(entry, many) }));

        return [
        { kind: "text", key: "name", label: "Room name", required: true },
        { kind: "text", key: "code", label: "Code", required: true },
        {
          kind: "select",
          key: "buildingId",
          label: "Building",
          emptyLabel: "No building",
          choices: buildingChoices,
          hint:
            buildingChoices.length === 0
              ? "This campus has no buildings. A room does not need one."
              : "Optional. Only buildings available to this campus are offered, shared ones included.",
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
                ? "For one physical room that serves more than one campus — including a campus of another school in this campus file. It stays a single room: one identity, one set of bookings, however many campuses list it. If it is in a building, only campuses that building reaches can be chosen; the save will say so."
                : "For one physical room that serves more than one campus. It stays a single room — one identity, one set of bookings, however many campuses list it. If it is in a building, only campuses that building reaches can be chosen; the save will say so.",
        },
        {
          kind: "select",
          key: "roomTypeId",
          label: "Room type",
          emptyLabel: "Unclassified",
          choices: typeChoices,
          hint: "Optional. Clearing it later is a normal edit.",
        },
        { kind: "number", key: "capacity", label: "Seats", required: true, min: 0 },
        {
          kind: "checkbox",
          key: "isBookable",
          label: "Lessons may be scheduled here",
          hint: "Clear this for a space that exists but should never be timetabled.",
        },
        { kind: "textarea", key: "notes", label: "Notes", hint: "Optional." },
        ];
      },
      validate: validateRoom,
      toDraft: (row) => ({
        name: row?.name ?? "",
        code: row?.code ?? "",
        buildingId: row?.buildingId ?? "",
        roomTypeId: row?.roomTypeId ?? "",
        capacity: row ? String(row.capacity) : "30",
        isBookable: row ? row.isBookable : true,
        notes: row?.notes ?? "",
        sharedCampusIds: row ? sharedOf(row) : [],
        // Not a field: a room's home campus is immutable, and this is carried so the
        // share picker knows which campus is not a share.
        homeCampusId: row?.campusId ?? campusId ?? "",
      }),
      toInput: (draft) => ({
        campusId: campusId ?? "",
        buildingId: textOrNull(draft, "buildingId"),
        roomTypeId: textOrNull(draft, "roomTypeId"),
        name: text(draft, "name"),
        code: text(draft, "code"),
        capacity: number(draft, "capacity"),
        isBookable: flag(draft, "isBookable"),
        notes: textOrNull(draft, "notes"),
        sharedCampusIds: list(draft, "sharedCampusIds"),
      }),
      list: loadRooms,
      create: (input) =>
        createRoom(withoutShares(input), [...input.sharedCampusIds]).then(
          toNewRow(input),
        ),
      // Withdrawals go first, then the room itself, then the new shares. The order is
      // what makes a legitimate edit possible: moving a room into a campus-local building
      // *and* dropping the campus that cannot reach it is one edit to the user, and it
      // only succeeds if the share is gone before the building changes. Adding shares
      // last for the mirror-image reason — the new building has to be in place before a
      // campus that depends on it is attached.
      //
      // The repository is the authority on each step and refuses the ones that are not
      // allowed; a refusal leaves the earlier steps applied, which is why the panel
      // re-reads the list after every mutation rather than patching it.
      update: async (row, input) => {
        const id = row.id;
        const existing = (await loadRooms()).find((other) => other.id === id);
        // The room's own home campus, not the one being viewed. A room shared *into* this
        // campus is homed on another — possibly another school's — and taking the viewed
        // campus as home would leave a withdrawal of this campus's own access silently
        // undone.
        const home = existing?.campusId ?? input.campusId;
        const before = new Set(existing?.campusIds ?? []);
        before.delete(home);
        const after = new Set(input.sharedCampusIds);

        for (const campus of before) {
          if (!after.has(campus)) await unshareRoom(school.id, id, campus);
        }

        const room = await updateRoom(id, row.rev, withoutShares(input));

        for (const campus of after) {
          if (!before.has(campus)) await shareRoom(id, campus);
        }

        return { ...room, campusIds: [home, ...after] };
      },
      remove: (row) => deleteRoom(school.id, row.id, row.rev),
      // Withdrawing a share is reversible but it is not nothing: a campus loses a room it
      // may have been timetabling into. Asking first is the same courtesy a delete gets.
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
              The room itself is not moved, copied or deleted. It keeps its identity and
              every other campus keeps it.
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
                <strong>{row.name}</strong> is on <strong>{owner.name}</strong>&rsquo;s
                campus, which owns it. Only the school a room was created on may delete it.
              </p>
              <p className="muted">
                This school can stop using it: clear this campus from{" "}
                <strong>Shared with campuses</strong>. The room is untouched and every other
                campus keeps it.
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
              {names.length === 1 ? "shares" : "share"} this room, through{" "}
              {sharers.map((entry) => entry.campus.name).join(", ")}.
            </p>
            <p className="muted">
              Owning a room is not authority to destroy something another school is
              timetabling into. Withdraw those campuses from the share list first — an
              ordinary edit, which that school can see — and the delete is then offered.
            </p>
          </>
        );
      },
      emptyHint:
        "Rooms are what lessons are scheduled into. Add the teaching spaces on this campus — you can classify them and group them by building later.",
      deleteNote: (row) =>
        row.campusIds.length > 1 ? (
          <p className="warning">
            This room serves {row.campusIds.length - 1} other campus
            {row.campusIds.length > 2 ? "es" : ""} of this school. Deleting it removes it
            from all of them, not just this one.
          </p>
        ) : null,
      blockedBy: campusId === null ? "Select a campus first." : null,
    };
  }, [
    campusId,
    campusOf,
    buildings.items,
    roomTypes.items,
    projectCampuses.all,
    loadRooms,
    many,
    school.id,
  ]);

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Rooms and room types</h2>
        <p>
          Rooms belong to a campus. A building and a room type are both optional — a room
          with neither is perfectly normal. One room can serve several campuses: a sports
          hall on a shared site is one room, not two, and stays one room however many
          campuses list it.
        </p>
        {many && (
          <p className="muted">
            Campuses of the other schools in this campus file can be shared with too. Room
            clashes are a property of the room, not of the campus or the school, so a room
            two institutions timetable into has to be <strong>one room</strong> for either
            of them to be scheduled correctly.
          </p>
        )}
      </header>

      {projectCampuses.error && (
        <Banner tone="warning">
          Some campuses could not be read, so the sharing list may be incomplete:{" "}
          {projectCampuses.error}
        </Banner>
      )}

      <CrudPanel spec={roomTypeSpec} reloadKey={school.id} onChanged={roomTypes.reload} />

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
          hint: "Add a campus in Campuses and Buildings first — a room has to be somewhere.",
        }}
      >
        <CrudPanel
          spec={roomSpec}
          reloadKey={`${school.id}:${campusId ?? ""}`}
          onChanged={() => {
            buildings.reload();
            rooms.reload();
          }}
        />
        <AvailabilityPanel
          school={school}
          entityKind="ROOM"
          noun="room"
          onOpenSection={props.context?.openSection}
          options={(rooms.items ?? []).map((room) => ({
            id: room.id,
            name: `${room.name} (${room.code})`,
          }))}
        />
      </ParentPicker>
    </div>
  );
}

/**
 * The draft minus its share set, which is a separate argument rather than part of
 * `RoomInput`.
 *
 * `RoomInput` is generated from the Rust record and has no room for it — the associations
 * are their own table — so the split happens here rather than by inventing a second
 * input shape ([ADR 0012](../../../../../docs/adr/0012-records-are-the-ipc-contract.md)).
 */
function withoutShares(input: RoomDraft) {
  const { sharedCampusIds: _ignored, ...roomInput } = input;
  return roomInput;
}

/**
 * Builds the row a create resolves to.
 *
 * `CrudPanel` re-reads the list straight afterwards, so this value is never what gets
 * rendered — it exists so `create` satisfies the spec's return type without inventing a
 * second source of truth.
 */
function toNewRow(input: RoomDraft): (created: Room) => RoomRow {
  return (created) => ({
    ...created,
    campusIds: [created.campusId, ...input.sharedCampusIds],
  });
}
