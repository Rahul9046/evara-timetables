/**
 * The workspace rail, and the school selection the two areas share.
 *
 * Three things only the shell can get wrong, and the third is the one with teeth:
 *
 * - both areas are reachable, and the rail says which is showing;
 * - the selected school is **one** answer: switching area must not quietly change which
 *   school is in view, which is the whole value of ADR 0023's one-school scope;
 * - switching school **remounts** both areas. `SchoolSetup` already had that guarantee for
 *   its sections; M8 extends it, because a generation session and an opened timetable are
 *   just as much "about" a school as a half-typed form is.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Project, RecentProject } from "../../ipc/project";

const currentProject = vi.fn<() => Promise<Project | null>>();
const recentProjects = vi.fn<() => Promise<RecentProject[]>>();

vi.mock("../../ipc/project", () => ({
  createProject: vi.fn(),
  openProject: vi.fn(),
  closeProject: vi.fn(() => Promise.resolve(true)),
  currentProject: () => currentProject(),
  recentProjects: () => recentProjects(),
  forgetRecentProject: vi.fn(),
  projectExtension: vi.fn(() => Promise.resolve("evaraproj")),
  defaultProjectFolder: vi.fn(() => Promise.resolve(null)),
  describeError: () => "Something went wrong.",
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

/**
 * Counts how many times each area was mounted.
 *
 * The remount assertion needs a way to see a mount, and a stub that counts is both the
 * simplest and the least brittle: it does not depend on anything either real area renders.
 */
const setupMounts = vi.fn();
const timetableMounts = vi.fn();

vi.mock("../setup/SchoolSetup", () => ({
  SchoolSetup: (props: { schoolId?: string | null; onSchoolChange?: (id: string | null) => void }) => {
    setupMounts(props.schoolId ?? null);
    return (
      <div>
        <p>School Setup area</p>
        <p>setup sees: {props.schoolId ?? "none"}</p>
        <button type="button" onClick={() => props.onSchoolChange?.("sc-2")}>
          Pick Southmoor
        </button>
      </div>
    );
  },
}));

vi.mock("./TimetableArea", () => ({
  TimetableArea: (props: { schoolId: string | null }) => {
    timetableMounts(props.schoolId);
    return (
      <div>
        <p>Timetables area</p>
        <p>timetables see: {props.schoolId ?? "none"}</p>
      </div>
    );
  },
}));

vi.mock("../branding", () => ({
  LogoMark: () => null,
  evaraBrand: { name: "Evara", initials: "EV" },
  resolveLogo: () => null,
  useSchoolLogo: () => ({ logo: null }),
}));

const { App } = await import("../App");

const OPEN: Project = {
  projectId: "p-1",
  displayName: "northgate",
  path: "C:/schools/northgate.evaraproj",
  folder: "C:/schools",
  schemaVersion: 17,
  createdHere: true,
  syncedFolder: null,
};

beforeEach(() => {
  currentProject.mockReset();
  recentProjects.mockReset();
  setupMounts.mockReset();
  timetableMounts.mockReset();
  currentProject.mockResolvedValue(OPEN);
  recentProjects.mockResolvedValue([]);
});

describe("the workspace rail", () => {
  it("offers both areas and shows School Setup first", async () => {
    render(<App />);
    await screen.findByText("School Setup area");

    const rail = screen.getByRole("navigation", { name: "Areas" });
    expect(rail.textContent).toContain("School Setup");
    expect(rail.textContent).toContain("Timetables");
    // The showing one is marked for a screen reader, not only by colour.
    expect(screen.getByRole("button", { name: "School Setup" }).getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("switches to Timetables and marks it as current", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("School Setup area");

    await user.click(screen.getByRole("button", { name: "Timetables" }));
    expect(await screen.findByText("Timetables area")).toBeTruthy();
    expect(screen.queryByText("School Setup area")).toBeNull();
    expect(screen.getByRole("button", { name: "Timetables" }).getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("hands the same school to both areas", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("School Setup area");

    await user.click(screen.getByRole("button", { name: "Pick Southmoor" }));
    expect(await screen.findByText("setup sees: sc-2")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Timetables" }));
    // The selection survives the area switch, and is the *same* one.
    expect(await screen.findByText("timetables see: sc-2")).toBeTruthy();
  });

  it("remounts both areas when the school changes", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("School Setup area");
    const before = setupMounts.mock.calls.length;

    await user.click(screen.getByRole("button", { name: "Pick Southmoor" }));
    await screen.findByText("setup sees: sc-2");

    // A re-render with a new prop would leave an open form, a pending delete and a
    // generation session standing. The `key` discards them.
    await waitFor(() => expect(setupMounts.mock.calls.length).toBeGreaterThan(before));
    expect(setupMounts.mock.calls.at(-1)).toEqual(["sc-2"]);

    await user.click(screen.getByRole("button", { name: "Timetables" }));
    await screen.findByText("Timetables area");
    expect(timetableMounts.mock.calls.at(-1)).toEqual(["sc-2"]);
  });
});
