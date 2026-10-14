/**
 * Section 5 — Resources.
 *
 * A resource is a shared, countable thing a lesson may need: a projector trolley, a
 * minibus, a set of class instruments.
 *
 * ## Why this section is back
 *
 * It was removed from School Setup when configuring a resource had no effect on anything
 * the user could see — eight steps each fed the grid and this one fed nothing. `V13`
 * changed the answer: [ADR 0022](../../../../../docs/adr/0022-resources-shared-across-campuses.md)
 * gave resources the campus associations buildings and rooms already had, so lending a
 * minibus to another school's campus is now a decision with a consequence, and it is
 * unreachable without a screen. Scheduling *against* resources is still Phase 4's.
 *
 * ## One resource, one identifier, however many campuses
 *
 * This is ADR 0022's binding invariant and the whole reason the association table exists:
 * `resource.id` is the scheduling identity, and a future clash rule is evaluated per
 * `resource_id` across the whole project. Two resource rows would be two availability
 * records, and neither would notice the clash — the failure ADR 0014 rejected for rooms.
 * So nothing on this screen copies a resource to share it.
 *
 * ## A resource need not belong to a campus
 *
 * The one place resources differ from rooms. `resource.campus_id` is nullable and `V3`
 * made it so deliberately: a minibus belongs to the school rather than to a site. A
 * school-wide resource has no associations at all, which is not an error state, and it can
 * still be lent — sharing is additive and gains it a row.
 *
 * ## Three operations, and only one of them destroys anything
 *
 * | Action | Effect |
 * | --- | --- |
 * | Stop sharing with a campus | one association row; the resource and every other campus keep it |
 * | Remove the home campus | refused — it would contradict `resource.campus_id` |
 * | Delete | the row and all its associations, and only while nobody else shares it |
 *
 * A trolley Northgate bought and lent to Southmoor is a trolley Southmoor is timetabling
 * around, so Northgate deleting it would take Southmoor's equipment away without Southmoor
 * being asked. The refusal is read before the action is offered and explained rather than
 * discovered.
 */
import { useCallback, useMemo, useState } from "react";

import type { Resource, ResourceInput, School } from "@evara/domain";
import {
  createResource,
  deleteResource,
  listResourceCampuses,
  listResources,
  shareResource,
  unshareResource,
  updateResource,
} from "../../ipc/setup";
import { CrudPanel, type CrudSpec } from "../ui/CrudPanel";
import type { Choice } from "../ui/Field";
import { Banner } from "../ui/Shell";
import { list, number, text, textOrNull, validateResource } from "../validation";
import {
  campusChoiceLabel,
  campusLabel,
  useCampusIndex,
  useProjectCampuses,
  type CampusOfSchool,
} from "./project";
import type { SetupContext } from "./SchoolSetup";

/**
 * One row of the resources table.
 *
 * The resource's own fields with the campuses it serves alongside, because `CrudPanel`
 * keys rows by `id` and the associations are their own table — there is no
 * `ResourceWithCampuses` projection, so this screen joins them.
 */
type ResourceRow = Resource & { readonly campusIds: readonly string[] };

/** What the form produces: a resource, plus the campuses it should serve besides its home. */
interface ResourceDraft {
  readonly input: ResourceInput;
  readonly sharedCampusIds: readonly string[];
}

export function ResourcesAndSharing(props: { school: School; context?: SetupContext }) {
  const { school } = props;
  const schools = props.context?.schools ?? [school];
  const many = schools.length > 1;

  const campuses = useProjectCampuses(schools);
  const campusOf = useCampusIndex(campuses.all);

  /**
   * The resources available to this school, with their campus associations.
   *
   * `listResources` already returns resources this school **owns or has been lent**, as the
   * same row with the same identifier it has at home — that is ADR 0022's `DISTINCT` join
   * through `resource_campus`. The associations are then read per resource, because there
   * is no projection that carries them and a school has tens of resources rather than
   * thousands.
   */
  const loadResources = useCallback(
    () =>
      listResources(school.id).then((rows) =>
        Promise.all(
          rows.map((resource) =>
            listResourceCampuses(resource.id).then(
              (campusIds): ResourceRow => ({ ...resource, campusIds }),
              // A resource whose associations cannot be read is still shown, with none
              // listed, rather than taking the whole table with it.
              (): ResourceRow => ({ ...resource, campusIds: [] }),
            ),
          ),
        ),
      ),
    [school.id],
  );

  // Held here as well as inside `CrudPanel` so the panel below can count what is shared.
  const [rows, setRows] = useState<readonly ResourceRow[]>([]);

  const spec: CrudSpec<ResourceRow, ResourceDraft> = useMemo(() => {
    const all = campuses.all ?? [];

    /** This school's own campuses: the only ones a resource of its own may be based at. */
    const ownCampuses = all.filter((entry) => entry.school.id === school.id);

    /** A resource's shared campuses, i.e. everything bar its home campus. */
    const sharedOf = (row: ResourceRow): string[] =>
      row.campusIds.filter((id) => id !== row.campusId);

    /** Which other schools can currently reach this resource. */
    const sharersOf = (row: ResourceRow): readonly CampusOfSchool[] =>
      row.campusIds
        .map((id) => campusOf.get(id))
        .filter((entry): entry is CampusOfSchool => entry !== undefined)
        .filter((entry) => entry.school.id !== row.schoolId);

    return {
      noun: "resource",
      plural: "resources",
      title: "Resources available to this school",
      describe: (row) => `${row.name} (${row.code})`,
      columns: [
        { header: "Name", render: (row) => row.name },
        { header: "Code", render: (row) => <code>{row.code}</code> },
        { header: "How many", numeric: true, render: (row) => row.quantity },
        {
          header: "Based at",
          render: (row) => {
            if (row.campusId === null) {
              return (
                <span title="A resource may belong to the school rather than to one of its sites">
                  School-wide
                </span>
              );
            }
            const home = campusOf.get(row.campusId);
            return home === undefined ? "—" : campusLabel(home, many);
          },
        },
        {
          header: "Campuses with access",
          render: (row) => {
            if (row.campusIds.length === 0) {
              return (
                <span className="absent">
                  No campus — available to {row.schoolId === school.id ? "this school" : "its school"}
                </span>
              );
            }
            return (
              <span className="shareList">
                {row.campusIds.map((id) => {
                  const entry = campusOf.get(id);
                  return (
                    <span key={id} className="shareName">
                      {entry === undefined ? "a campus" : campusLabel(entry, many)}
                      {id === row.campusId && (
                        <span className="muted" title="The campus this resource is based at">
                          {" "}
                          (home)
                        </span>
                      )}
                    </span>
                  );
                })}
              </span>
            );
          },
        },
        {
          header: "Ownership",
          render: (row) =>
            row.schoolId === school.id ? (
              sharersOf(row).length === 0 ? (
                "Owned by this school"
              ) : (
                <>
                  <span className="chipBadge">lent</span> Owned by this school
                </>
              )
            ) : (
              <>
                <span className="chipBadge">shared</span> Lent by{" "}
                {schools.find((candidate) => candidate.id === row.schoolId)?.name ??
                  "another school"}
              </>
            ),
        },
      ],
      fields: (draft) => {
        const home = text(draft, "campusId");
        return [
          {
            kind: "text",
            key: "name",
            label: "Resource name",
            required: true,
            placeholder: "Projector trolley",
          },
          {
            kind: "text",
            key: "code",
            label: "Code",
            required: true,
            placeholder: "PROJ-1",
            hint: "Short and unique within the school.",
          },
          {
            kind: "number",
            key: "quantity",
            label: "How many",
            required: true,
            min: 0,
            hint: "How many of this thing exist. One trolley is 1; a set of thirty recorders kept together is 1 set, not 30.",
          },
          {
            kind: "select",
            key: "campusId",
            label: "Based at",
            emptyLabel: "School-wide — no particular campus",
            choices: ownCampuses.map((entry) => ({
              value: entry.campus.id,
              label: campusChoiceLabel(entry, false),
            })),
            hint: "Optional, and only this school's own campuses. A minibus that belongs to the school rather than to a site is school-wide, which is a normal state rather than an unfinished one. Changing this moves where the resource lives: the old campus stops listing it unless you also add it below.",
          },
          {
            kind: "multicombobox",
            key: "sharedCampusIds",
            label: "Also available to campuses",
            choices: shareChoices(all, home, many),
            placeholder: all.length === 0 ? "No campuses yet" : "Search campuses",
            emptyLabel: "No campus matches that",
            noneLabel:
              home.length === 0 ? "This school only" : "Its home campus only",
            hint: many
              ? "Campuses of any school in this campus file. It stays one resource with one identifier, however many campuses list it — which is what makes its availability a single fact when scheduling arrives."
              : "Other campuses of this school that may use it. It stays one resource with one identifier however many list it.",
          },
        ];
      },
      validate: validateResource,
      toDraft: (row) => ({
        name: row?.name ?? "",
        code: row?.code ?? "",
        quantity: row === null ? "1" : String(row.quantity),
        campusId: row?.campusId ?? "",
        sharedCampusIds: row ? sharedOf(row) : [],
      }),
      toInput: (draft) => ({
        input: {
          schoolId: school.id,
          campusId: textOrNull(draft, "campusId"),
          name: text(draft, "name"),
          code: text(draft, "code"),
          quantity: number(draft, "quantity"),
        },
        sharedCampusIds: list(draft, "sharedCampusIds"),
      }),
      list: () =>
        loadResources().then((loaded) => {
          setRows(loaded);
          return loaded;
        }),
      create: async (draft) => {
        const created = await createResource(draft.input);
        // Sharing is a second call per campus because that is what the command surface is
        // — one association at a time, each its own change-log entry. A refusal partway
        // leaves the earlier ones applied, which is why the panel re-reads the list after
        // every mutation rather than patching it.
        for (const campusId of draft.sharedCampusIds) {
          if (campusId !== created.campusId) {
            await shareResource(school.id, created.id, campusId);
          }
        }
        return { ...created, campusIds: [...draft.sharedCampusIds] };
      },
      // Withdrawals first, then the resource, then the new shares — the order
      // `RoomsAndRoomTypes` uses, and for the same reason: moving the home campus *and*
      // changing the share set is one edit to the user, and each step only succeeds once
      // the one before it has.
      //
      // **Neither home campus is touched here.** `update_resource` owns both sides of the
      // move — it withdraws the old home's association and writes the new one, in its own
      // transaction — so the old home is excluded from the withdrawals and the new home
      // from the shares. Including either would send a call the backend refuses by name:
      // withdrawing a resource's home campus is exactly what `unshare_resource` is there
      // to prevent.
      update: async (id, draft) => {
        const existing = (await loadResources()).find((row) => row.id === id);
        const oldHome = existing?.campusId ?? null;
        const newHome = draft.input.campusId;

        // Both homes are excluded from both sides. The old one because
        // `update_resource` withdraws it; the new one because `update_resource` writes it,
        // or keeps the association it already finds rather than rewriting it — so
        // withdrawing it here only to have it written back would spend two revisions
        // recording nothing.
        const before = new Set(existing?.campusIds ?? []);
        if (oldHome !== null) before.delete(oldHome);
        if (newHome !== null) before.delete(newHome);
        const after = new Set(
          draft.sharedCampusIds.filter((campusId) => campusId !== newHome),
        );

        for (const campusId of before) {
          if (!after.has(campusId)) await unshareResource(school.id, id, campusId);
        }
        const saved = await updateResource(id, draft.input);
        for (const campusId of after) {
          if (!before.has(campusId)) await shareResource(school.id, id, campusId);
        }

        const campusIds = [...after];
        if (saved.campusId !== null) campusIds.unshift(saved.campusId);
        return { ...saved, campusIds };
      },
      remove: (id) => deleteResource(school.id, id),
      editBlockedBy: (row) =>
        row.schoolId === school.id
          ? null
          : `${row.name} is lent by ${
              schools.find((candidate) => candidate.id === row.schoolId)?.name ??
              "another school"
            }, which owns it. This school can stop using it — clear its own campuses from the share list there — but its name, code and quantity belong to the owner.`,
      // Withdrawing a share is reversible but it is not nothing: a campus loses equipment
      // it may have been planning around. Asking first is the same courtesy a delete gets.
      //
      // A *move* of the home campus is deliberately not confirmed here. It is one explicit
      // field change whose consequence — the old site stops listing it — is exactly what
      // the user asked for, and the "Based at" hint says how to keep that site's access.
      // So the old home is excluded from this comparison, the way it is excluded from the
      // withdrawals themselves.
      confirmSubmit: (draft, row) => {
        if (!row) return null;
        const after = new Set(list(draft, "sharedCampusIds"));
        const home = textOrNull(draft, "campusId");
        const removed = row.campusIds
          .filter((id) => id !== row.campusId)
          .filter((id) => !after.has(id) && id !== home);
        if (removed.length === 0) return null;

        return (
          <>
            <p>
              <strong>{row.name}</strong> will no longer be available to{" "}
              <strong>
                {removed
                  .map((id) => {
                    const entry = campusOf.get(id);
                    return entry === undefined ? "a campus" : campusLabel(entry, many);
                  })
                  .join(", ")}
              </strong>
              .
            </p>
            <p className="muted">
              The resource itself is not moved, copied or deleted. It keeps its identity and
              every other campus keeps it.
            </p>
          </>
        );
      },
      deleteBlockedBy: (row) => {
        const owner = schools.find((candidate) => candidate.id === row.schoolId);
        if (row.schoolId !== school.id) {
          return (
            <>
              <p>
                <strong>{row.name}</strong> is lent by{" "}
                <strong>{owner?.name ?? "another school"}</strong>, and only the school that
                owns a resource may delete it.
              </p>
              <p className="muted">
                What this school can do is stop using it: edit the resource at{" "}
                {owner?.name ?? "its own school"}, or ask that school to withdraw this
                campus from the share list. Either way the equipment is untouched.
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
              {names.length === 1 ? "still has" : "still have"} access to{" "}
              <strong>{row.name}</strong>, through{" "}
              {sharers.map((entry) => entry.campus.name).join(", ")}.
            </p>
            <p className="muted">
              Owning a resource is not authority to destroy something another school is
              planning around. Withdraw those campuses from &ldquo;Also available to
              campuses&rdquo; first — an ordinary edit, which that school can see — and the
              delete is then offered.
            </p>
          </>
        );
      },
      emptyHint:
        "Resources are optional and nothing schedules against them yet. Add them when you want to record the equipment a lesson may need — and, in a campus file with more than one school, which other schools may use it.",
      deleteNote: (row) =>
        row.campusIds.length > 0 ? (
          <p className="muted">
            Its {row.campusIds.length === 1 ? "campus association" : "campus associations"}{" "}
            {row.campusIds.length === 1 ? "goes" : "go"} with it. No campus is changed.
          </p>
        ) : null,
      blockedBy:
        campuses.all === null
          ? null
          : ownCampuses.length === 0 && all.length === 0
            ? "Add a campus under Campuses and buildings first — a resource can be school-wide, but a campus file with no campus at all has nothing to record equipment against."
            : null,
    };
  }, [campuses.all, campusOf, loadResources, many, school.id, schools]);

  const lent = rows.filter(
    (row) =>
      row.schoolId === school.id &&
      row.campusIds.some((id) => campusOf.get(id)?.school.id !== school.id),
  ).length;
  const borrowed = rows.filter((row) => row.schoolId !== school.id).length;

  return (
    <div className="section">
      <header className="sectionIntro">
        <h2>Resources</h2>
        <p>
          Equipment a lesson may need, counted rather than named individually. A resource
          belongs to one school and may be <strong>based at</strong> one of its campuses or
          belong to the school as a whole — and it can be made available to any number of
          other campuses, including campuses of another school in this campus file.
        </p>
        <p className="muted">
          A shared resource stays <strong>one resource with one identifier</strong>. That is
          the point rather than a detail: when scheduling arrives, the minibus is in use or
          it is not, and no school gets its own private answer.
        </p>
      </header>

      {campuses.error && (
        <Banner tone="warning">
          Some campuses could not be read, so the sharing lists may be incomplete:{" "}
          {campuses.error}
        </Banner>
      )}

      {(lent > 0 || borrowed > 0) && (
        <Banner tone="info">
          {lent > 0 && (
            <>
              This school lends {lent} resource{lent === 1 ? "" : "s"} to another school.{" "}
            </>
          )}
          {borrowed > 0 && (
            <>
              {borrowed} resource{borrowed === 1 ? "" : "s"} on this list{" "}
              {borrowed === 1 ? "is" : "are"} lent to this school by another and{" "}
              {borrowed === 1 ? "is" : "are"} not this school&rsquo;s to rename or delete.
            </>
          )}
        </Banner>
      )}

      <CrudPanel spec={spec} reloadKey={`${school.id}:${(campuses.all ?? []).length}`} />
    </div>
  );
}

/**
 * The campuses a resource can be *additionally* made available to.
 *
 * Its home campus is excluded, because the home association is not a share and cannot be
 * withdrawn as one — the backend refuses it and names the remedy, which is to change
 * "Based at" and let the association follow.
 */
function shareChoices(
  all: readonly CampusOfSchool[],
  homeCampusId: string,
  many: boolean,
): Choice[] {
  return all
    .filter((entry) => entry.campus.id !== homeCampusId)
    .map((entry) => ({ value: entry.campus.id, label: campusChoiceLabel(entry, many) }));
}
