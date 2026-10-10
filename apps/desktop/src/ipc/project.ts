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

/** The open project, as the backend describes it. */
export interface Project {
  readonly projectId: string;
  readonly displayName: string;
  readonly path: string;
  readonly folder: string;
  readonly schemaVersion: number;
  /** Whether this installation created the project. */
  readonly createdHere: boolean;
  /** Cloud provider the file appears to sit under. Advisory only. */
  readonly syncedFolder: string | null;
}

/** An entry in the recent-project list. */
export interface RecentProject {
  readonly path: string;
  readonly folder: string;
  readonly displayName: string;
  readonly lastOpenedAt: string;
  /** False when the file has been moved or deleted outside Evara. */
  readonly exists: boolean;
}

/** Every lifecycle failure the backend distinguishes. */
export type ProjectError =
  | { kind: "schemaTooNew"; found: number; supported: number; message: string }
  | { kind: "notFound"; message: string }
  | { kind: "notAnEvaraProject"; message: string }
  | { kind: "wrongExtension"; expected: string; message: string }
  | { kind: "alreadyExists"; message: string }
  | { kind: "noProjectOpen"; message: string }
  | { kind: "failed"; message: string };

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
