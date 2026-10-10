/**
 * The project IPC wrappers, and the error contract the interface branches on.
 *
 * There is no Tauri runtime under Vitest, so `invoke` is mocked. What matters here is
 * that each command sends the argument shape the backend expects, and that a rejection
 * stays machine-readable rather than collapsing into a string.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const {
  closeProject,
  createProject,
  currentProject,
  describeError,
  forgetRecentProject,
  isProjectError,
  openProject,
  projectExtension,
  recentProjects,
} = await import("./project");

const PROJECT = {
  projectId: "018f3a2c-0000-7000-8000-000000000001",
  displayName: "Northgate",
  path: "/tmp/Northgate.evaraproj",
  folder: "/tmp",
  schemaVersion: 2,
  createdHere: true,
  syncedFolder: null,
};

beforeEach(() => {
  invoke.mockReset();
});

describe("lifecycle commands", () => {
  it("creates a project at the chosen path", async () => {
    invoke.mockResolvedValue(PROJECT);

    await expect(createProject("/tmp/Northgate.evaraproj")).resolves.toEqual(PROJECT);
    expect(invoke).toHaveBeenCalledWith("project_create", {
      path: "/tmp/Northgate.evaraproj",
    });
  });

  it("opens a project at the chosen path", async () => {
    invoke.mockResolvedValue(PROJECT);

    await openProject("/tmp/Northgate.evaraproj");
    expect(invoke).toHaveBeenCalledWith("project_open", {
      path: "/tmp/Northgate.evaraproj",
    });
  });

  it("closes without arguments and reports whether anything was open", async () => {
    invoke.mockResolvedValue(true);

    await expect(closeProject()).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("project_close");
  });

  it("reports no open project as null rather than throwing", async () => {
    invoke.mockResolvedValue(null);
    await expect(currentProject()).resolves.toBeNull();
  });

  it("reads the recent list", async () => {
    const recents = [
      {
        path: "/tmp/a.evaraproj",
        folder: "/tmp",
        displayName: "a",
        lastOpenedAt: "2026-10-06T00:00:00.000Z",
        exists: true,
      },
    ];
    invoke.mockResolvedValue(recents);

    await expect(recentProjects()).resolves.toEqual(recents);
    expect(invoke).toHaveBeenCalledWith("project_recent");
  });

  it("forgets a recent project by path", async () => {
    invoke.mockResolvedValue(true);

    await forgetRecentProject("/tmp/a.evaraproj");
    expect(invoke).toHaveBeenCalledWith("project_forget_recent", {
      path: "/tmp/a.evaraproj",
    });
  });

  it("asks the backend for the project extension instead of hardcoding it", async () => {
    invoke.mockResolvedValue("evaraproj");

    await expect(projectExtension()).resolves.toBe("evaraproj");
  });
});

describe("error contract", () => {
  it("recognises a tagged backend error", () => {
    expect(
      isProjectError({ kind: "schemaTooNew", found: 9, supported: 2, message: "..." }),
    ).toBe(true);
  });

  it("rejects values that are not backend errors", () => {
    for (const value of [null, undefined, "boom", 42, {}, { kind: "x" }]) {
      expect(isProjectError(value)).toBe(false);
    }
  });

  it("keeps the backend message for a known failure", () => {
    expect(
      describeError({
        kind: "notAnEvaraProject",
        message: "notes.txt is not an Evara project",
      }),
    ).toBe("notes.txt is not an Evara project");
  });

  it("does not surface an unexpected throw verbatim", () => {
    // An internal error is a bug, not something the user caused, and its text may carry
    // detail that should not reach the screen.
    const message = describeError(new Error("thread panicked at src/lib.rs:42"));
    expect(message).not.toContain("src/lib.rs");
    expect(message).toBe("Something went wrong. Please try again.");
  });

  it("propagates a rejection unchanged so callers can branch on kind", async () => {
    invoke.mockRejectedValue({
      kind: "schemaTooNew",
      found: 9,
      supported: 2,
      message: "Update Evara to open it.",
    });

    await expect(openProject("/tmp/future.evaraproj")).rejects.toMatchObject({
      kind: "schemaTooNew",
      found: 9,
    });
  });
});
