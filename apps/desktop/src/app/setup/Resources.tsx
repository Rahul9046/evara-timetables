/**
 * Section 4 — Resources.
 *
 * A resource is a shared, countable thing a lesson may need: a projector trolley, a set
 * of laptops, a minibus. It belongs to the school, and its campus is optional — `null`
 * means school-wide, which is a real choice rather than a missing value, so the select
 * offers it by name.
 */
import { useCallback, useMemo } from "react";

import type { Campus, Resource, School } from "@evara/domain";
import {
  createResource,
  deleteResource,
  listCampuses,
  listResources,
  updateResource,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import { number, text, textOrNull, validateResource } from "../validation";
import { useList } from "./useList";

export function Resources(props: { school: School }) {
  const { school } = props;

  const loadCampuses = useCallback(() => listCampuses(), []);
  const campuses = useList<Campus>(loadCampuses);

  const spec = useMemo<CrudSpec<Resource, Parameters<typeof createResource>[0]>>(() => {
    const choices = (campuses.items ?? []).map((c) => ({ value: c.id, label: c.name }));
    const campusName = new Map(choices.map((c) => [c.value, c.label]));

    return {
      noun: "resource",
      plural: "resources",
      title: "Resources",
      describe: (row) => `${row.name} (${row.code})`,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Code", render: (row) => <code>{row.code}</code> },
        {
          header: "Campus",
          render: (row) =>
            row.campusId ? (
              (campusName.get(row.campusId) ?? <span className="absent">—</span>)
            ) : (
              <span className="muted">School-wide</span>
            ),
        },
        { header: "How many", numeric: true, render: (row) => row.quantity },
      ],
      fields: [
        {
          kind: "text",
          key: "name",
          label: "Resource name",
          required: true,
          placeholder: "Projector trolley",
        },
        { kind: "text", key: "code", label: "Code", required: true, placeholder: "PROJ" },
        {
          kind: "select",
          key: "campusId",
          label: "Campus",
          emptyLabel: "School-wide",
          choices,
          hint: "Leave as school-wide for something that moves between sites.",
        },
        {
          kind: "number",
          key: "quantity",
          label: "How many",
          required: true,
          min: 0,
          hint: "How many exist in total. The solver will treat this as the limit on simultaneous use.",
        },
      ],
      validate: validateResource,
      toDraft: (row) => ({
        name: row?.name ?? "",
        code: row?.code ?? "",
        campusId: row?.campusId ?? "",
        quantity: row ? String(row.quantity) : "1",
      }),
      toInput: (draft) => ({
        schoolId: school.id,
        campusId: textOrNull(draft, "campusId"),
        name: text(draft, "name"),
        code: text(draft, "code"),
        quantity: number(draft, "quantity"),
      }),
      list: listResources,
      create: createResource,
      update: updateResource,
      remove: deleteResource,
      emptyHint:
        "Resources are optional. Add them when a lesson needs equipment that cannot be in two places at once — the solver will respect the quantity once constraints arrive.",
    };
  }, [school.id, campuses.items]);

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Resources</h2>
        <p>
          Shared equipment a lesson may need. Nothing schedules against resources yet —
          that arrives with constraints — but recording them now means the timetable can
          respect them later.
        </p>
      </header>

      <CrudPanel spec={spec} />
    </div>
  );
}
