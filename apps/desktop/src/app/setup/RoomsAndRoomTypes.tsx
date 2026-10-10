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
 * A building choice is restricted to buildings on the room's own campus, because the
 * schema refuses the cross-campus case with a trigger. Showing an option that is
 * guaranteed to fail is not a choice.
 */
import { useCallback, useMemo, useState } from "react";

import type { Building, Campus, Room, RoomType, School } from "@evara/domain";
import {
  createRoom,
  createRoomType,
  deleteRoom,
  deleteRoomType,
  listBuildings,
  listCampuses,
  listRoomTypes,
  listRooms,
  updateRoom,
  updateRoomType,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import {
  flag,
  number,
  text,
  textOrNull,
  validateRoom,
  validateRoomType,
} from "../validation";
import { Absent } from "./CampusesAndBuildings";
import { ParentPicker } from "./ParentPicker";
import { useList } from "./useList";

export function RoomsAndRoomTypes(props: { school: School }) {
  const { school } = props;
  const [campusId, setCampusId] = useState<string | null>(null);

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

  const loadRoomTypes = useCallback(() => listRoomTypes(), []);
  const roomTypes = useList<RoomType>(loadRoomTypes);

  const loadBuildings = useCallback(
    () => (campusId ? listBuildings(campusId) : Promise.resolve([])),
    [campusId],
  );
  const buildings = useList<Building>(loadBuildings);

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
      list: listRoomTypes,
      create: createRoomType,
      update: updateRoomType,
      remove: deleteRoomType,
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

  const roomSpec = useMemo<CrudSpec<Room, Parameters<typeof createRoom>[0]>>(() => {
    const buildingChoices: Choice[] = (buildings.items ?? []).map((b) => ({
      value: b.id,
      label: b.name,
    }));
    const typeChoices: Choice[] = (roomTypes.items ?? []).map((t) => ({
      value: t.id,
      label: `${t.name} (${t.code})`,
    }));
    const buildingName = new Map(buildingChoices.map((c) => [c.value, c.label]));
    const typeName = new Map(typeChoices.map((c) => [c.value, c.label]));

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
      ],
      fields: [
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
              : "Optional, and only buildings on this campus are offered.",
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
      ],
      validate: validateRoom,
      toDraft: (row) => ({
        name: row?.name ?? "",
        code: row?.code ?? "",
        buildingId: row?.buildingId ?? "",
        roomTypeId: row?.roomTypeId ?? "",
        capacity: row ? String(row.capacity) : "30",
        isBookable: row ? row.isBookable : true,
        notes: row?.notes ?? "",
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
      }),
      list: () => (campusId ? listRooms(campusId) : Promise.resolve([])),
      create: createRoom,
      update: updateRoom,
      remove: deleteRoom,
      emptyHint:
        "Rooms are what lessons are scheduled into. Add the teaching spaces on this campus — you can classify them and group them by building later.",
      blockedBy: campusId === null ? "Select a campus first." : null,
    };
  }, [campusId, buildings.items, roomTypes.items]);

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Rooms and room types</h2>
        <p>
          Rooms belong to a campus. A building and a room type are both optional — a room
          with neither is perfectly normal.
        </p>
      </header>

      <CrudPanel spec={roomTypeSpec} onChanged={roomTypes.reload} />

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
          reloadKey={campusId ?? ""}
          onChanged={buildings.reload}
        />
      </ParentPicker>
    </div>
  );
}
