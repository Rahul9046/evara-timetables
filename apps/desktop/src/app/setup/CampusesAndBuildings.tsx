/**
 * Section 2 — Campuses and Buildings.
 *
 * A campus is owned by the school; a building is owned by a campus. Both edges are
 * `RESTRICT`, so neither can be deleted out from under its children — the delete fails
 * and says what is in the way. Buildings are the exception on the other side: a room's
 * building is an optional classification, so deleting a building *detaches* its rooms
 * rather than deleting them, and the confirmation says so.
 */
import { useCallback, useMemo, useState } from "react";

import type { Building, Campus, School } from "@evara/domain";
import {
  createBuilding,
  createCampus,
  deleteBuilding,
  deleteCampus,
  listBuildings,
  listCampuses,
  updateBuilding,
  updateCampus,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { text, textOrNull, validateBuilding, validateCampus } from "../validation";
import { ParentPicker } from "./ParentPicker";
import { useList } from "./useList";

export function CampusesAndBuildings(props: { school: School }) {
  const { school } = props;
  const [campusId, setCampusId] = useState<string | null>(null);

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

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
      list: listCampuses,
      create: createCampus,
      update: updateCampus,
      remove: deleteCampus,
      emptyHint:
        "A school needs at least one campus before rooms, bell schedules or resources can be recorded against it. A single-site school has exactly one.",
      deleteNote: () => (
        <p className="muted">
          A campus that still has buildings, rooms or resources cannot be deleted. Move or
          remove those first.
        </p>
      ),
    }),
    [school.id],
  );

  const loadBuildings = useCallback(
    () => (campusId ? listBuildings(campusId) : Promise.resolve([])),
    [campusId],
  );

  const buildingSpec = useMemo<CrudSpec<Building, Parameters<typeof createBuilding>[0]>>(
    () => ({
      noun: "building",
      plural: "buildings",
      title: "Buildings on this campus",
      describe: (row) => row.name,
      columns: [{ header: "Name", render: (row) => row.name }],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Building name",
          required: true,
          hint: "Unique within the campus.",
        },
      ],
      validate: validateBuilding,
      toDraft: (row) => ({ name: row?.name ?? "" }),
      toInput: (draft) => ({ campusId: campusId ?? "", name: text(draft, "name") }),
      list: loadBuildings,
      create: createBuilding,
      update: updateBuilding,
      remove: deleteBuilding,
      emptyHint:
        "Buildings are optional. Add them only if you want rooms grouped by building — a small school can leave this empty and put rooms directly on the campus.",
      deleteNote: () => (
        <p className="muted">
          Rooms in this building are <strong>not</strong> deleted. They stay on the campus
          and simply stop naming a building.
        </p>
      ),
      blockedBy: campusId === null ? "Select a campus first." : null,
    }),
    [campusId, loadBuildings],
  );

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Campuses and buildings</h2>
        <p>
          A campus is a physical site. Buildings group the rooms on a site and are
          optional — rooms can sit directly on a campus.
        </p>
      </header>

      <CrudPanel spec={campusSpec} onChanged={campuses.reload} />

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
        <CrudPanel spec={buildingSpec} reloadKey={campusId ?? ""} />
      </ParentPicker>
    </div>
  );
}

/** A nullable column with nothing in it. Styled so a blank cell is deliberate. */
export function Absent() {
  return <span className="absent">—</span>;
}
