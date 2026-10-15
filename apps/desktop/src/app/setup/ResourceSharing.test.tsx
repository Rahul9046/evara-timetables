/**
 * Stage 0C: the Resources section, and lending equipment across schools.
 *
 * `evara-db`'s `tests_multi_school.rs` owns the rules — either end's school may share or
 * withdraw, the home campus may not be withdrawn, a delete is refused while another school
 * shares. This file owns the four things only the interface can get wrong:
 *
 * - **a shared resource is one resource.** ADR 0022's binding invariant: nothing here
 *   copies a resource to lend it, so one minibus appears in two schools' lists with one
 *   identifier — which is what will make its occupancy one fact when scheduling arrives.
 * - **a resource need not belong to a campus.** `resource.campus_id` is nullable on
 *   purpose, and a form that insisted would force a school to invent a site for a minibus.
 * - **withdrawing access is not deleting.** Different action, different confirmation, and
 *   the home campus cannot be withdrawn at all.
 * - **deletion is explained, not offered.** The owner may not destroy equipment another
 *   school is planning around, and that is read before the action rather than discovered.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Campus, Resource, School } from "@evara/domain";

const listCampuses = vi.fn<(schoolId: string) => Promise<Campus[]>>();
const listResources = vi.fn<(schoolId: string) => Promise<Resource[]>>();
const listResourceCampuses = vi.fn<(resourceId: string) => Promise<string[]>>();
const createResource = vi.fn<(input: unknown) => Promise<Resource>>();
const updateResource =
  vi.fn<(id: string, rev: number, input: unknown) => Promise<Resource>>();
const deleteResource = vi.fn<(schoolId: string, id: string, rev: number) => Promise<void>>();
const shareResource =
  vi.fn<(schoolId: string, resourceId: string, campusId: string) => Promise<void>>();
const unshareResource =
  vi.fn<(schoolId: string, resourceId: string, campusId: string) => Promise<void>>();

vi.mock("../../ipc/setup", () => ({
  listCampuses: (schoolId: string) => listCampuses(schoolId),
  listResources: (schoolId: string) => listResources(schoolId),
  listResourceCampuses: (resourceId: string) => listResourceCampuses(resourceId),
  createResource: (input: unknown) => createResource(input),
  updateResource: (id: string, rev: number, input: unknown) =>
    updateResource(id, rev, input),
  deleteResource: (schoolId: string, id: string, rev: number) =>
    deleteResource(schoolId, id, rev),
  shareResource: (schoolId: string, resourceId: string, campusId: string) =>
    shareResource(schoolId, resourceId, campusId),
  unshareResource: (schoolId: string, resourceId: string, campusId: string) =>
    unshareResource(schoolId, resourceId, campusId),
  // Stage B. Empty by default: a suite that is about something else should not have
  // to describe a curriculum, and one that is about this panel overrides them.
  listDepartments: vi.fn(() => Promise.resolve([])),
  listSubjects: vi.fn(() => Promise.resolve([])),
  listTeacherDepartments: vi.fn(() => Promise.resolve([])),
  listQualifications: vi.fn(() => Promise.resolve([])),
  listSubjectTeachers: vi.fn(() => Promise.resolve([])),
  listDepartmentTeachers: vi.fn(() => Promise.resolve([])),
  departmentDependents: vi.fn(() =>
    Promise.resolve({ total: 0, subjects: 0, teachers: 0 }),
  ),
  subjectDependents: vi.fn(() => Promise.resolve({ total: 0, qualifiedTeachers: 0 })),
  addQualification: vi.fn(() => Promise.resolve({})),
  removeQualification: vi.fn(() => Promise.resolve()),
  addTeacherDepartment: vi.fn(() => Promise.resolve({})),
  removeTeacherDepartment: vi.fn(() => Promise.resolve()),
  createDepartment: vi.fn(() => Promise.resolve({})),
  updateDepartment: vi.fn(() => Promise.resolve({})),
  deleteDepartment: vi.fn(() => Promise.resolve()),
  createSubject: vi.fn(() => Promise.resolve({})),
  updateSubject: vi.fn(() => Promise.resolve({})),
  deleteSubject: vi.fn(() => Promise.resolve()),
  isConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "conflict",
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
  offendingField: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    (error as { kind?: string }).kind === "duplicate"
      ? ((error as { field: string }).field ?? null)
      : null,
}));

const { ResourcesAndSharing } = await import("./ResourcesAndSharing");
type SetupContext = import("./SchoolSetup").SetupContext;

const STAMP = {
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  rev: 1,
};

const NORTH = { id: "school-north", name: "Northgate Secondary", code: "NGS" } as School;
const SOUTH = { id: "school-south", name: "Southmoor College", code: "SMC" } as School;

function campus(id: string, schoolId: string, name: string, code: string): Campus {
  return { id, schoolId, name, code, address: null, ...STAMP } as Campus;
}

const NORTH_MAIN = campus("c-north-main", NORTH.id, "Main Campus", "NM");
const NORTH_JUNIOR = campus("c-north-junior", NORTH.id, "Junior Campus", "NJ");
const SOUTH_MAIN = campus("c-south-main", SOUTH.id, "Riverside", "SR");

function resource(overrides: Partial<Resource> = {}): Resource {
  return {
    id: "res-minibus",
    schoolId: NORTH.id,
    campusId: NORTH_MAIN.id,
    name: "Minibus",
    code: "BUS-1",
    quantity: 1,
    ...STAMP,
    ...overrides,
  } as Resource;
}

const MINIBUS = resource();
/** A minibus that belongs to the school rather than to one of its sites. */
const TROLLEY = resource({
  id: "res-trolley",
  campusId: null,
  name: "Projector trolley",
  code: "PROJ-1",
  quantity: 4,
});

function context(overrides: Partial<SetupContext> = {}): SetupContext {
  return {
    schools: [NORTH, SOUTH],
    selectSchool: vi.fn(),
    openSection: vi.fn(),
    reloadSchools: vi.fn(),
    ...overrides,
  };
}

function mount(school: School, overrides: Partial<SetupContext> = {}) {
  return render(<ResourcesAndSharing school={school} context={context(overrides)} />);
}

beforeEach(() => {
  vi.clearAllMocks();
  listCampuses.mockImplementation((schoolId) =>
    Promise.resolve(schoolId === NORTH.id ? [NORTH_MAIN, NORTH_JUNIOR] : [SOUTH_MAIN]),
  );
  listResources.mockResolvedValue([MINIBUS]);
  listResourceCampuses.mockResolvedValue([NORTH_MAIN.id]);
  createResource.mockImplementation((input) =>
    Promise.resolve(resource({ id: "res-new", ...(input as Partial<Resource>) })),
  );
  updateResource.mockImplementation((id, input) =>
    Promise.resolve(resource({ id, ...(input as Partial<Resource>) })),
  );
  deleteResource.mockResolvedValue(undefined);
  shareResource.mockResolvedValue(undefined);
  unshareResource.mockResolvedValue(undefined);
});

describe("the resources a school can reach", () => {
  it("shows what it owns, where it is based, and who else may use it", async () => {
    mount(NORTH);

    expect(await screen.findByText("Minibus")).toBeTruthy();
    expect(screen.getByText("BUS-1")).toBeTruthy();
    await waitFor(() => expect(screen.getByText("Owned by this school")).toBeTruthy());
  });

  it("says school-wide rather than leaving a blank where a campus would be", async () => {
    listResources.mockResolvedValue([TROLLEY]);
    listResourceCampuses.mockResolvedValue([]);
    mount(NORTH);

    expect(await screen.findByText("Projector trolley")).toBeTruthy();
    // `resource.campus_id` is nullable on purpose; an empty cell would read as unfinished.
    await waitFor(() => expect(screen.getByText("School-wide")).toBeTruthy());
  });

  it("shows a lent resource under the borrowing school, with the same identifier", async () => {
    // One row, one identity, two schools' lists — which is what `listResources` returns
    // through its `DISTINCT` join and what this screen must not duplicate.
    listResources.mockResolvedValue([MINIBUS]);
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(SOUTH);

    expect(await screen.findByText("Minibus")).toBeTruthy();
    expect(screen.getAllByText("Minibus")).toHaveLength(1);
    await waitFor(() =>
      expect(screen.getByText(/Lent by Northgate Secondary/)).toBeTruthy(),
    );
  });

  it("marks a resource this school lends out", async () => {
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(NORTH);

    await screen.findByText("Minibus");
    await waitFor(() => expect(screen.getByText("lent")).toBeTruthy());
    expect(screen.getByText(/This school lends 1 resource/)).toBeTruthy();
  });

  it("warns that a borrowed resource is not this school's to rename", async () => {
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(SOUTH);

    await screen.findByText("Minibus");
    expect(await screen.findByText(/to rename or delete/)).toBeTruthy();
  });

  it("names the campus with access, and marks the home one", async () => {
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(NORTH);

    await screen.findByText("Minibus");
    // Twice on the row: once as "Based at", once in the list of campuses with access.
    await waitFor(() => expect(screen.getAllByText(/Main Campus/)).toHaveLength(2));
    expect(screen.getByText("(home)")).toBeTruthy();
    expect(screen.getByText(/Riverside — Southmoor College/)).toBeTruthy();
  });
});

describe("creating a resource", () => {
  it("creates a school-wide one when no campus is chosen", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: "Add resource" }));
    await user.type(screen.getByLabelText(/Resource name/), "Minibus 2");
    await user.type(screen.getByLabelText(/^Code/), "BUS-2");
    await user.click(screen.getByRole("button", { name: "Add resource" }));

    await waitFor(() => expect(createResource).toHaveBeenCalled());
    expect(createResource).toHaveBeenCalledWith({
      schoolId: NORTH.id,
      campusId: null,
      name: "Minibus 2",
      code: "BUS-2",
      quantity: 1,
    });
    // Nothing shared, so no association beyond the one `create_resource` writes itself.
    expect(shareResource).not.toHaveBeenCalled();
  });

  it("offers only this school's campuses as a home", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: "Add resource" }));

    const labels = within(screen.getByLabelText(/Based at/))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(labels).toContain("Main Campus (NM)");
    // A resource's home campus must belong to its own school; the trigger pair says so.
    expect(labels?.some((label) => label?.includes("Riverside"))).toBe(false);
  });

  it("shares with another school's campus as part of the create", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: "Add resource" }));
    await user.type(screen.getByLabelText(/Resource name/), "Shared minibus");
    await user.type(screen.getByLabelText(/^Code/), "BUS-3");
    await user.selectOptions(screen.getByLabelText(/Based at/), NORTH_MAIN.id);
    await user.click(screen.getByLabelText(/Also available to campuses/));
    await user.click(
      within(await screen.findByRole("listbox", { name: "Options" })).getByRole("option", {
        name: /Riverside/,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Add resource" }));

    await waitFor(() => expect(createResource).toHaveBeenCalled());
    await waitFor(() => expect(shareResource).toHaveBeenCalled());
    // The acting school owns the resource, which is one of the two authorities
    // `require_may_unshare` accepts.
    expect(shareResource).toHaveBeenCalledWith(NORTH.id, "res-new", SOUTH_MAIN.id);
  });

  it("marks a duplicate code on the field, keeping the typed values", async () => {
    const user = userEvent.setup();
    createResource.mockRejectedValue({
      kind: "duplicate",
      entity: "resource",
      field: "code",
      message: "another resource already uses that code",
    });
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: "Add resource" }));
    await user.type(screen.getByLabelText(/Resource name/), "Minibus 2");
    await user.type(screen.getByLabelText(/^Code/), "BUS-1");
    await user.click(screen.getByRole("button", { name: "Add resource" }));

    expect(await screen.findByText(/already uses that code/)).toBeTruthy();
    expect((screen.getByLabelText(/^Code/) as HTMLInputElement).value).toBe("BUS-1");
  });
});

describe("sharing a resource", () => {
  it("adds a campus without touching the resource row", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.click(screen.getByLabelText(/Also available to campuses/));
    await user.click(
      within(await screen.findByRole("listbox", { name: "Options" })).getByRole("option", {
        name: /Riverside/,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(shareResource).toHaveBeenCalled());
    expect(shareResource).toHaveBeenCalledWith(NORTH.id, MINIBUS.id, SOUTH_MAIN.id);
    // One resource, one identifier: nothing is copied to lend it.
    expect(createResource).not.toHaveBeenCalled();
  });

  it("does not offer the home campus as something to share with", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.click(screen.getByLabelText(/Also available to campuses/));

    // The share picker's own list, not the "Based at" select's, which legitimately offers
    // the home campus because that is what it sets.
    const listbox = await screen.findByRole("listbox", { name: "Options" });
    const options = within(listbox)
      .getAllByRole("option")
      .map((option) => option.textContent);
    // The home association is not a share and cannot be withdrawn as one; the remedy is
    // to change "Based at", and the hint says so.
    expect(options.some((label) => label?.includes("Main Campus"))).toBe(false);
    expect(options.some((label) => label?.includes("Riverside"))).toBe(true);
  });
});

describe("moving a resource's home campus", () => {
  /**
   * The home association is `update_resource`'s to move, not this screen's.
   *
   * ADR 0022 made a resource's home campus editable and made the association follow it,
   * in one transaction. A screen that treated the old home as a withdrawn share would send
   * `unshare_resource` for it — which is the one call that operation exists to refuse,
   * because withdrawing a home campus would contradict `resource.campus_id`.
   */
  it("does not try to withdraw the old home campus", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.selectOptions(screen.getByLabelText(/Based at/), NORTH_JUNIOR.id);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateResource).toHaveBeenCalled());
    expect(updateResource.mock.calls[0]?.[2]).toMatchObject({
      campusId: NORTH_JUNIOR.id,
    });
    expect(unshareResource).not.toHaveBeenCalled();
    expect(shareResource).not.toHaveBeenCalled();
  });

  it("does not try to share the new home campus either", async () => {
    const user = userEvent.setup();
    // Already shared with Junior Campus, and now based there. `update_resource` keeps the
    // association it finds rather than rewriting it, so a `share_resource` call would be
    // refused as a duplicate.
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, NORTH_JUNIOR.id]);
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.selectOptions(screen.getByLabelText(/Based at/), NORTH_JUNIOR.id);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    // Junior Campus moves from share to home, which is the one change, so there is
    // nothing to confirm and nothing to share.
    await waitFor(() => expect(updateResource).toHaveBeenCalled());
    expect(shareResource).not.toHaveBeenCalled();
    expect(unshareResource).not.toHaveBeenCalled();
  });

  it("still withdraws a genuine share alongside the move", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.selectOptions(screen.getByLabelText(/Based at/), NORTH_JUNIOR.id);
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareResource).toHaveBeenCalled());
    // Southmoor's share goes; the old home campus does not, because moving it is the
    // update's own business.
    expect(unshareResource).toHaveBeenCalledTimes(1);
    expect(unshareResource).toHaveBeenCalledWith(NORTH.id, MINIBUS.id, SOUTH_MAIN.id);
  });
});

describe("withdrawing access", () => {
  it("asks first, and says the resource itself is untouched", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/no longer be available to/)).toBeTruthy();
    expect(within(dialog).getByText(/not moved, copied or deleted/)).toBeTruthy();
    expect(unshareResource).not.toHaveBeenCalled();
  });

  it("withdraws exactly the campus that was removed", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, NORTH_JUNIOR.id, SOUTH_MAIN.id]);
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(unshareResource).toHaveBeenCalled());
    expect(unshareResource).toHaveBeenCalledTimes(1);
    expect(unshareResource).toHaveBeenCalledWith(NORTH.id, MINIBUS.id, SOUTH_MAIN.id);
    // Junior Campus keeps it, and so does the home campus.
    expect(deleteResource).not.toHaveBeenCalled();
  });

  it("surfaces a refusal rather than reporting a withdrawal that did not happen", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    unshareResource.mockRejectedValue({
      kind: "wrongSchool",
      entity: "resource",
      message: "that resource belongs to a different school",
    });
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Edit Minibus/ }));
    await user.click(screen.getByRole("button", { name: /^Remove Riverside/ }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText(/belongs to a different school/)).toBeTruthy();
  });
});

describe("deleting a resource", () => {
  it("is offered for one nobody else shares", async () => {
    const user = userEvent.setup();
    mount(NORTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Delete Minibus/ }));

    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete resource" }));

    await waitFor(() =>
      expect(deleteResource).toHaveBeenCalledWith(NORTH.id, MINIBUS.id, 1),
    );
  });

  it("is explained rather than offered while another school shares it", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(NORTH);
    await screen.findByText("Minibus");
    await waitFor(() => expect(screen.getByText("lent")).toBeTruthy());

    await user.click(screen.getByRole("button", { name: /^Delete Minibus/ }));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/Southmoor College/)).toBeTruthy();
    expect(within(dialog).getByText(/Riverside/)).toBeTruthy();
    expect(
      within(dialog).getByText(/not authority to destroy something another school/),
    ).toBeTruthy();
    // No destructive button, and nothing is sent.
    expect(within(dialog).queryByRole("button", { name: "Delete resource" })).toBeNull();
    expect(deleteResource).not.toHaveBeenCalled();
  });

  it("is explained rather than offered to a school that merely borrows it", async () => {
    const user = userEvent.setup();
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(SOUTH);
    await screen.findByText("Minibus");

    await user.click(screen.getByRole("button", { name: /^Delete Minibus/ }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/only the school that owns a resource may delete it/),
    ).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Delete resource" })).toBeNull();
  });

  it("does not offer a borrowing school an edit it cannot make", async () => {
    listResourceCampuses.mockResolvedValue([NORTH_MAIN.id, SOUTH_MAIN.id]);
    mount(SOUTH);
    await screen.findByText("Minibus");

    const edit = await screen.findByRole("button", { name: /^Edit Minibus/ });
    await waitFor(() => expect(edit.hasAttribute("disabled")).toBe(true));
    expect(edit.getAttribute("title")).toMatch(/Northgate Secondary/);
  });
});
