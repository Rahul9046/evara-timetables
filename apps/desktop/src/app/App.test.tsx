/**
 * The application shell, in the interface's own vocabulary.
 *
 * What a user creates and opens is a **campus**. Everything underneath still calls it a
 * project — the `.evaraproj` extension, the `project_*` commands, `ProjectView` — and this
 * file is where the two meet: it drives the screen by its new labels and asserts the old
 * commands are what actually get called.
 *
 * That pairing is the point. A rename that reached the wiring would be a file-format
 * change wearing a vocabulary change's clothes, and the tests below would not notice on
 * their own. So every one of them checks both halves.
 *
 * ## The collision these tests are guarding
 *
 * A campus file contains one school, and that school has its own campuses — the
 * "Campuses and buildings" section of School Setup. Two different things, one word. The
 * start screen is safe because no school is open there; inside School Setup the file is
 * called a "campus file". `no misleading terminology where both concepts appear` is the
 * test that holds that line.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { Project, RecentProject } from "../ipc/project";

const createProject = vi.fn<(path: string) => Promise<Project>>();
const openProject = vi.fn<(path: string) => Promise<Project>>();
const closeProject = vi.fn<() => Promise<boolean>>();
const currentProject = vi.fn<() => Promise<Project | null>>();
const recentProjects = vi.fn<() => Promise<RecentProject[]>>();
const forgetRecentProject = vi.fn<(path: string) => Promise<boolean>>();
const projectExtension = vi.fn<() => Promise<string>>();
const defaultProjectFolder = vi.fn<() => Promise<string | null>>();

vi.mock("../ipc/project", () => ({
  createProject: (path: string) => createProject(path),
  openProject: (path: string) => openProject(path),
  closeProject: () => closeProject(),
  currentProject: () => currentProject(),
  recentProjects: () => recentProjects(),
  forgetRecentProject: (path: string) => forgetRecentProject(path),
  projectExtension: () => projectExtension(),
  defaultProjectFolder: () => defaultProjectFolder(),
  describeError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : "Something went wrong. Please try again.",
  isProjectError: () => true,
}));

const saveDialog = vi.fn<(options: unknown) => Promise<string | null>>();
const openDialog = vi.fn<(options: unknown) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (options: unknown) => saveDialog(options),
  open: (options: unknown) => openDialog(options),
}));

// School Setup has its own tests and needs a backend of its own; the shell only has to
// put it on screen.
vi.mock("./setup/SchoolSetup", () => ({
  SchoolSetup: () => <div data-testid="school-setup">School setup</div>,
}));

vi.mock("../ipc/setup", () => ({
  getSchoolLogo: vi.fn(() => Promise.resolve(null)),
}));

const { App } = await import("./App");

function project(overrides: Partial<Project> = {}): Project {
  return {
    projectId: "p-1",
    displayName: "Northgate Secondary",
    path: "C:/Schools/Northgate Secondary.evaraproj",
    folder: "C:/Schools",
    schemaVersion: 8,
    createdHere: true,
    syncedFolder: null,
    ...overrides,
  };
}

function recent(name: string, exists = true): RecentProject {
  return {
    path: `C:/Schools/${name}.evaraproj`,
    folder: "C:/Schools",
    displayName: name,
    lastOpenedAt: "2026-01-01T00:00:00.000Z",
    exists,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  currentProject.mockResolvedValue(null);
  recentProjects.mockResolvedValue([]);
  projectExtension.mockResolvedValue("evaraproj");
  defaultProjectFolder.mockResolvedValue("C:/Schools");
  closeProject.mockResolvedValue(true);
  forgetRecentProject.mockResolvedValue(true);
});

/** The start screen, once the first read has landed. */
async function startScreen() {
  render(<App />);
  await screen.findByRole("heading", { name: "Evara Timetables" });
}

/**
 * The button that opens a recent campus.
 *
 * Each row has two — the opener, whose name starts with the campus name, and "Remove X
 * from recent campuses". Anchoring at the start is what tells them apart.
 */
const recentOpener = (name: string) =>
  screen.getByRole("button", { name: new RegExp(`^${name}`) });

/** The shell with a campus open. */
async function opened(overrides: Partial<Project> = {}) {
  currentProject.mockResolvedValue(project(overrides));
  render(<App />);
  await screen.findByTestId("school-setup");
}

describe("the start screen", () => {
  it("asks for a campus, not a project", async () => {
    await startScreen();

    expect(screen.getByRole("button", { name: "New campus" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open campus…" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Recent campuses" })).toBeTruthy();
    expect(screen.getByText(/Create a campus to get started/)).toBeTruthy();
  });

  it("says nothing about projects anywhere on screen", async () => {
    await startScreen();

    // The whole screen, not just the labels that were listed: a stray "project" in an
    // empty state or a tooltip is exactly what this rename is meant to remove.
    expect(document.body.textContent).not.toMatch(/project/i);
  });

  it("names the empty state in the new vocabulary", async () => {
    await startScreen();

    expect(screen.getByText(/No recent campuses/)).toBeTruthy();
  });
});

describe("creating a campus", () => {
  it("offers a campus-worded save dialogue and creates through the unchanged command", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("C:/Schools/New.evaraproj");
    createProject.mockResolvedValue(project());
    await startScreen();

    await user.click(screen.getByRole("button", { name: "New campus" }));

    await waitFor(() => expect(createProject).toHaveBeenCalled());
    const options = saveDialog.mock.calls.at(-1)?.[0] as {
      title: string;
      filters: { name: string; extensions: string[] }[];
    };
    expect(options.title).toBe("New Evara campus");
    expect(options.filters[0]?.name).toBe("Evara campus");
    // The file format is untouched. This is the assertion that keeps the rename to
    // vocabulary: a renamed extension would be a new file format.
    expect(options.filters[0]?.extensions).toEqual(["evaraproj"]);
    expect(createProject).toHaveBeenCalledWith("C:/Schools/New.evaraproj");
  });

  it("does nothing when the dialogue is cancelled", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue(null);
    await startScreen();

    await user.click(screen.getByRole("button", { name: "New campus" }));

    await waitFor(() => expect(saveDialog).toHaveBeenCalled());
    expect(createProject).not.toHaveBeenCalled();
  });

  it("shows the backend's refusal in the new vocabulary", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("C:/Schools/New.evaraproj");
    createProject.mockRejectedValue({
      kind: "alreadyExists",
      message: "C:/Schools/New.evaraproj already exists",
    });
    await startScreen();

    await user.click(screen.getByRole("button", { name: "New campus" }));

    expect(
      await screen.findByText(/C:\/Schools\/New.evaraproj already exists/),
    ).toBeTruthy();
  });
});

describe("opening a campus", () => {
  it("offers a campus-worded open dialogue and opens through the unchanged command", async () => {
    const user = userEvent.setup();
    openDialog.mockResolvedValue("C:/Schools/Northgate Secondary.evaraproj");
    openProject.mockResolvedValue(project());
    await startScreen();

    await user.click(screen.getByRole("button", { name: "Open campus…" }));

    await waitFor(() => expect(openProject).toHaveBeenCalled());
    const options = openDialog.mock.calls.at(-1)?.[0] as {
      title: string;
      filters: { name: string; extensions: string[] }[];
    };
    expect(options.title).toBe("Open Evara campus");
    expect(options.filters[0]?.extensions).toEqual(["evaraproj"]);
  });

  it("opens an existing file written before the rename", async () => {
    // The point of the rename being vocabulary only: a `.evaraproj` created by an earlier
    // build opens with no migration and no conversion.
    const user = userEvent.setup();
    openDialog.mockResolvedValue("C:/Old/Legacy.evaraproj");
    const legacy = project({
      displayName: "Legacy",
      path: "C:/Old/Legacy.evaraproj",
      // A file written long before this rename, at a much older schema version.
      schemaVersion: 4,
    });
    openProject.mockResolvedValue(legacy);
    await startScreen();
    // `run` re-reads the open file after every action, so this is what it will find.
    currentProject.mockResolvedValue(legacy);

    await user.click(screen.getByRole("button", { name: "Open campus…" }));

    await screen.findByTestId("school-setup");
    expect(openProject).toHaveBeenCalledWith("C:/Old/Legacy.evaraproj");
    expect(screen.getByRole("heading", { name: "Legacy" })).toBeTruthy();
  });
});

describe("recent campuses", () => {
  it("lists them and opens one by name", async () => {
    const user = userEvent.setup();
    recentProjects.mockResolvedValue([recent("Northgate"), recent("Southfield")]);
    openProject.mockResolvedValue(project());
    await startScreen();

    expect(screen.getByText("Northgate")).toBeTruthy();
    expect(screen.getByText("Southfield")).toBeTruthy();

    await user.click(recentOpener("Northgate"));
    await waitFor(() =>
      expect(openProject).toHaveBeenCalledWith("C:/Schools/Northgate.evaraproj"),
    );
  });

  it("names the remove action in the new vocabulary", async () => {
    const user = userEvent.setup();
    recentProjects.mockResolvedValue([recent("Northgate")]);
    await startScreen();

    const remove = screen.getByRole("button", {
      name: "Remove Northgate from recent campuses",
    });
    await user.click(remove);

    // Still the same persistence command: the list is stored exactly as before.
    await waitFor(() =>
      expect(forgetRecentProject).toHaveBeenCalledWith("C:/Schools/Northgate.evaraproj"),
    );
  });

  it("marks one whose file has gone, and will not open it", async () => {
    recentProjects.mockResolvedValue([recent("Moved", false)]);
    await startScreen();

    expect(screen.getByText("File not found")).toBeTruthy();
    expect((recentOpener("Moved") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("an open campus", () => {
  it("is labelled a campus in the title bar", async () => {
    await opened();

    expect(screen.getByText("CAMPUS")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Northgate Secondary" })).toBeTruthy();
  });

  it("closes through the unchanged command, and says which campus it means", async () => {
    const user = userEvent.setup();
    await opened();

    const close = screen.getByRole("button", { name: "Close campus" });
    // The title bar sits above a "Campuses and buildings" section, so the button's
    // tooltip has to say it closes the file rather than a campus in that list.
    expect(close.getAttribute("title")).toBe(
      "Close this campus file and return to the start screen",
    );

    currentProject.mockResolvedValue(null);
    await user.click(close);

    await waitFor(() => expect(closeProject).toHaveBeenCalled());
  });

  it("returns to the start screen once closed", async () => {
    const user = userEvent.setup();
    await opened();

    currentProject.mockResolvedValue(null);
    recentProjects.mockResolvedValue([recent("Northgate Secondary")]);
    await user.click(screen.getByRole("button", { name: "Close campus" }));

    expect(await screen.findByRole("button", { name: "New campus" })).toBeTruthy();
    expect(screen.queryByTestId("school-setup")).toBeNull();
  });

  it("can be reopened after being closed", async () => {
    const user = userEvent.setup();
    await opened();

    currentProject.mockResolvedValue(null);
    recentProjects.mockResolvedValue([recent("Northgate Secondary")]);
    await user.click(screen.getByRole("button", { name: "Close campus" }));
    await screen.findByRole("button", { name: "New campus" });

    openProject.mockResolvedValue(project());
    currentProject.mockResolvedValue(project());
    await user.click(recentOpener("Northgate Secondary"));

    await screen.findByTestId("school-setup");
    expect(openProject).toHaveBeenCalledWith("C:/Schools/Northgate Secondary.evaraproj");
  });

  it("warns about a synced folder in the new vocabulary", async () => {
    await opened({ syncedFolder: "OneDrive" });

    const banner = await screen.findByText(/This campus is stored in OneDrive/);
    expect(banner.textContent).toMatch(/This campus is stored in OneDrive/);
    expect(screen.getByText(/An open Evara/).textContent).toMatch(
      /campus is a database written continuously/,
    );
  });
});

describe("no misleading terminology where both concepts appear", () => {
  it("never shortens the open file to bare 'campus' outside the title bar", async () => {
    // The title bar is the one place the file is called a campus while School Setup is on
    // screen, and it is immediately beside the name of the file. Everywhere else inside
    // the shell says "campus file", so a sentence about the file cannot be read as a
    // sentence about one of the school's campuses.
    await opened();

    const titleBar = screen.getByText("CAMPUS").closest("header");
    expect(titleBar).toBeTruthy();
    expect(within(titleBar as HTMLElement).getByRole("button", { name: "Close campus" }))
      .toBeTruthy();
  });

  it("keeps the file format and the commands untouched by the rename", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue("C:/Schools/New.evaraproj");
    createProject.mockResolvedValue(project());
    await startScreen();

    await user.click(screen.getByRole("button", { name: "New campus" }));

    // `projectExtension()` is asked, and whatever it answers is what the dialogue filters
    // on. Nothing here hardcodes or rewrites an extension.
    await waitFor(() => expect(projectExtension).toHaveBeenCalled());
    expect(createProject).toHaveBeenCalledWith("C:/Schools/New.evaraproj");
  });
});
