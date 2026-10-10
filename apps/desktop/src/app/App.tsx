/**
 * The application shell.
 *
 * Two states, and the project decides which: with nothing open there is a start screen,
 * and with a project open there is the School Setup workflow under a title bar. The
 * Phase 1B shell proved the lifecycle worked; this is the first version with something to
 * do once a project is open.
 *
 * The shell holds no rules. It opens and closes projects, and renders whichever of the
 * two states applies.
 */
import { useCallback, useEffect, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import {
  closeProject,
  createProject,
  currentProject,
  defaultProjectFolder,
  describeError,
  forgetRecentProject,
  openProject,
  projectExtension,
  recentProjects,
  type Project,
  type RecentProject,
} from "../ipc/project";
import { SchoolSetup } from "./setup/SchoolSetup";
import { Banner } from "./ui/Shell";

export function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);

  const refresh = useCallback(async () => {
    const [open, list] = await Promise.all([currentProject(), recentProjects()]);
    setProject(open);
    setRecents(list);
  }, []);

  useEffect(() => {
    refresh()
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setReady(true));
  }, [refresh]);

  /** Runs a lifecycle action, surfacing failure rather than leaving the UI stale. */
  const run = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await action();
        await refresh();
      } catch (e: unknown) {
        setError(describeError(e));
        // The backend leaves nothing open after a failed open, so resync rather than
        // trusting what the screen currently shows.
        await refresh().catch(() => undefined);
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );

  const handleNew = () =>
    run(async () => {
      const extension = await projectExtension();
      const path = await saveDialog({
        title: "New Evara project",
        defaultPath: (await defaultProjectFolder()) ?? undefined,
        filters: [{ name: "Evara project", extensions: [extension] }],
      });
      if (!path) return;
      await createProject(path);
    });

  const handleOpen = () =>
    run(async () => {
      const extension = await projectExtension();
      const selected = await openDialog({
        title: "Open Evara project",
        multiple: false,
        directory: false,
        filters: [{ name: "Evara project", extensions: [extension] }],
      });
      if (typeof selected !== "string") return;
      await openProject(selected);
    });

  if (!ready) {
    return (
      <main className="startShell">
        <p>Starting…</p>
      </main>
    );
  }

  if (!project) {
    return (
      <main className="startShell">
        <StartScreen
          recents={recents}
          busy={busy}
          error={error}
          onNew={handleNew}
          onOpen={handleOpen}
          onOpenRecent={(path) => run(() => openProject(path))}
          onForget={(path) => run(() => forgetRecentProject(path))}
        />
      </main>
    );
  }

  return (
    <div className="appShell">
      <TitleBar
        project={project}
        busy={busy}
        error={error}
        onClose={() => run(closeProject)}
        onDismissError={() => setError(null)}
      />
      {/* Keyed by project so switching projects resets every section's state rather
          than showing the previous project's selections against the new one. */}
      <main className="appMain">
        <SchoolSetup key={project.projectId} />
      </main>
    </div>
  );
}

function TitleBar(props: {
  project: Project;
  busy: boolean;
  error: string | null;
  onClose: () => void;
  onDismissError: () => void;
}) {
  const { project, busy, error, onClose, onDismissError } = props;

  return (
    <header className="titleBar">
      <div className="titleBarMain">
        <span className="eyebrow">PROJECT</span>
        <h1 title={project.path}>{project.displayName}</h1>
        <span className="titleBarMeta">
          {project.folder} · schema v{project.schemaVersion}
        </span>
      </div>

      <div className="titleBarActions">
        <button type="button" className="secondary" onClick={onClose} disabled={busy}>
          Close project
        </button>
      </div>

      {project.syncedFolder && (
        <Banner tone="warning">
          <strong>This project is stored in {project.syncedFolder}.</strong> A live Evara
          project is a database written continuously, and sync clients can copy its files
          mid-write and corrupt it. Keeping it in a local folder is safer.
        </Banner>
      )}

      {error && (
        <div className="titleBarError">
          <Banner tone="error">{error}</Banner>
          <button type="button" className="linkish" onClick={onDismissError}>
            Dismiss
          </button>
        </div>
      )}
    </header>
  );
}

function StartScreen(props: {
  recents: RecentProject[];
  busy: boolean;
  error: string | null;
  onNew: () => void;
  onOpen: () => void;
  onOpenRecent: (path: string) => void;
  onForget: (path: string) => void;
}) {
  const { recents, busy, error, onNew, onOpen, onOpenRecent, onForget } = props;

  return (
    <section className="panel">
      <span className="eyebrow">OPEN SOURCE · LOCAL FIRST</span>
      <h1>Evara Timetables</h1>
      <p>
        Create a school project, or open one you already have. Everything stays on this
        machine.
      </p>

      <div className="actions">
        <button type="button" onClick={onNew} disabled={busy}>
          New project
        </button>
        <button type="button" className="secondary" onClick={onOpen} disabled={busy}>
          Open project…
        </button>
      </div>

      {error && <Banner tone="error">{error}</Banner>}

      <h2 className="sectionHead">Recent projects</h2>
      {recents.length === 0 ? (
        <p className="muted">Nothing yet. Projects you create or open will appear here.</p>
      ) : (
        <ul className="recents">
          {recents.map((entry) => (
            <li key={entry.path} className={entry.exists ? undefined : "missing"}>
              <button
                type="button"
                className="recentOpen"
                disabled={busy || !entry.exists}
                onClick={() => onOpenRecent(entry.path)}
                title={entry.path}
              >
                <strong>{entry.displayName}</strong>
                <span>{entry.exists ? entry.folder : "File not found"}</span>
              </button>
              <button
                type="button"
                className="linkish"
                disabled={busy}
                onClick={() => onForget(entry.path)}
                aria-label={`Remove ${entry.displayName} from recent projects`}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
