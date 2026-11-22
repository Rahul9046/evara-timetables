/**
 * Project lifecycle, across the IPC boundary.
 *
 * Every command is wrapped exactly once, here. Components call these functions and never
 * `invoke` directly, so the surface stays enumerable and mockable.
 *
 * Failures arrive as a tagged {@link ProjectError}, so the interface branches on `kind`
 * rather than reading English out of a message string.
 */
import { invoke } from "@tauri-apps/api/core";
import type { ProjectError, ProjectView, RecentProjectView } from "@evara/domain";

/**
 * The open project, as the backend describes it.
 *
 * Generated from the Rust since M10 Stage 4. These three types were hand-written here from
 * Phase 1B onwards, which `CLAUDE.md` §2 names as an architecture bug — a TypeScript entity
 * type mirroring a Rust one — and the mirror had already started to drift: `RecentProject`
 * gained `projectId` and `duplicateIdentity` for D13, and `ProjectError` gained eight transfer
 * refusals, neither of which a hand-written union would have known about.
 *
 * Aliased rather than renamed at every call site, because the *name* a component uses is not
 * the part that was wrong.
 */
export type Project = ProjectView;

/** An entry in the recent-project list. */
export type RecentProject = RecentProjectView;

/** Every lifecycle failure the backend distinguishes, transfer included. */
export type { ProjectError };

/** Narrows an unknown rejection to a {@link ProjectError}. */
export function isProjectError(value: unknown): value is ProjectError {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return typeof v["kind"] === "string" && typeof v["message"] === "string";
}

/**
 * Turns any rejection into something showable.
 *
 * A thrown value that is not a {@link ProjectError} means a bug rather than a condition
 * the user caused, so it gets a deliberately generic message.
 */
export function describeError(error: unknown): string {
  if (isProjectError(error)) return error.message;
  return "Something went wrong. Please try again.";
}

/** Creates a project at `path` and opens it. */
export function createProject(path: string): Promise<Project> {
  return invoke<Project>("project_create", { path });
}

/** Opens an existing project. */
export function openProject(path: string): Promise<Project> {
  return invoke<Project>("project_open", { path });
}

/** Closes the open project. Resolves to whether anything was open. */
export function closeProject(): Promise<boolean> {
  return invoke<boolean>("project_close");
}

/** The open project, or null. */
export function currentProject(): Promise<Project | null> {
  return invoke<Project | null>("project_current");
}

/** Recently opened projects, newest first. */
export function recentProjects(): Promise<RecentProject[]> {
  return invoke<RecentProject[]>("project_recent");
}

/** Removes a project from the recent list. The file itself is never touched. */
export function forgetRecentProject(path: string): Promise<boolean> {
  return invoke<boolean>("project_forget_recent", { path });
}

/** The extension a live project must use, so dialog filters have one source. */
export function projectExtension(): Promise<string> {
  return invoke<string>("project_extension");
}

/** A sensible starting folder for a new project, if one can be determined. */
export function defaultProjectFolder(): Promise<string | null> {
  return invoke<string | null>("project_default_folder");
}
