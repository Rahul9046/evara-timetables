/**
 * Phase 1B application shell.
 *
 * Infrastructure UI, not the real timetable interface. Its only job is to exercise the
 * project lifecycle end to end: create, open, close, recent projects, and the advisory
 * cloud-folder warning. The real chrome arrives with the features it serves.
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
      <main className="shell">
        <p>Starting…</p>
      </main>
    );
  }

  return (
    <main className="shell">
      {project ? (
        <OpenProject
          project={project}
          busy={busy}
          onClose={() => run(closeProject)}
          error={error}
        />
      ) : (
        <StartScreen
          recents={recents}
          busy={busy}
          error={error}
          onNew={handleNew}
          onOpen={handleOpen}
          onOpenRecent={(path) => run(() => openProject(path))}
          onForget={(path) => run(() => forgetRecentProject(path))}
        />
      )}
    </main>
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
      <p>Create a school project, or open one you already have. Everything stays on this machine.</p>

      <div className="actions">
        <button type="button" onClick={onNew} disabled={busy}>
          New project
        </button>
        <button type="button" className="secondary" onClick={onOpen} disabled={busy}>
          Open project…
        </button>
      </div>

      {error && <p className="error" role="alert">{error}</p>}

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

function OpenProject(props: {
  project: Project;
  busy: boolean;
  error: string | null;
  onClose: () => void;
}) {
  const { project, busy, error, onClose } = props;

  return (
    <section className="panel">
      <span className="eyebrow">PROJECT OPEN</span>
      <h1>{project.displayName}</h1>

      {project.syncedFolder && (
        <p className="warning" role="status">
          <strong>This project is stored in {project.syncedFolder}.</strong> A live Evara
          project is a database that is written continuously, and sync clients can copy its
          files mid-write and corrupt it. Keeping it in a local folder is safer.
        </p>
      )}

      {error && <p className="error" role="alert">{error}</p>}

      <dl>
        <dt>Location</dt>
        <dd title={project.path}>{project.folder}</dd>
        <dt>Project ID</dt>
        <dd className="mono">{project.projectId}</dd>
        <dt>Schema</dt>
        <dd>v{project.schemaVersion}</dd>
        <dt>Origin</dt>
        <dd>{project.createdHere ? "Created on this installation" : "Created elsewhere"}</dd>
      </dl>

      <p className="muted">
        No school data can be entered yet — structure, people and timetables arrive in the
        phases after this one.
      </p>

      <div className="actions">
        <button type="button" className="secondary" onClick={onClose} disabled={busy}>
          Close project
        </button>
      </div>
    </section>
  );
}
