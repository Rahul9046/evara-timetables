/**
 * The application shell.
 *
 * Two states, and the open file decides which: with nothing open there is a start screen,
 * and with something open there is the School Setup workflow under a title bar. The
 * Phase 1B shell proved the lifecycle worked; this is the first version with something to
 * do once a file is open.
 *
 * The shell holds no rules. It opens and closes files, and renders whichever of the two
 * states applies.
 *
 * ## "Campus" on screen, `project` in the code
 *
 * What a user creates and opens is called a **campus**. Everything underneath still calls
 * it a project: the `.evaraproj` extension, the `project_*` commands, `ProjectView`,
 * `RecentProject`, the identifiers in this file. That is deliberate — the rename is
 * vocabulary, not architecture, and renaming the model would have meant a migration and a
 * file-format change for a wording decision.
 *
 * **The word is overloaded, and the overload is real.** A campus file contains one school,
 * and that school has its own campuses — the "Campuses and buildings" section of School
 * Setup. They are different things with the same name. So:
 *
 * - on the start screen, where no school is open and no campus entity can be on screen,
 *   plain "campus" is unambiguous and is what is used;
 * - inside School Setup, where a list of the school's campuses is a click away, the file
 *   is called a **"campus file"** rather than a "campus". The extra word is what keeps
 *   "removed from this campus file" from reading as "removed from the campus you have
 *   selected".
 *
 * Anything added later that names the file while a campus entity is visible should follow
 * the second rule. "Export campus file…" in the title bar is the first thing added under it.
 *
 * ## The two transfer actions, and why they are in different places
 *
 * **Export** is about the open campus file, so it is a title-bar action. **Import** produces a
 * new campus file and finishes by offering to open it — which would close whatever is open and
 * discard its edit session — so it is a start-screen action, where nothing is open. That is
 * D9's rule ("an import does not open what it imported, and does not disturb what is open")
 * made true of the interface rather than only of the backend.
 *
 * Neither writes anything itself: both are thin screens over the eleven Tauri transfer
 * commands, and the archive is read and written in Rust.
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
import { TimetableArea } from "./timetables/TimetableArea";
import { LogoMark, evaraBrand, resolveLogo, useSchoolLogo } from "./branding";
import { Banner } from "./ui/Shell";
import { ExportCampus } from "./transfer/ExportCampus";
import { ImportCampus } from "./transfer/ImportCampus";
import { RecoveryNotices } from "./transfer/RecoveryNotices";

export function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  /**
   * Which transfer dialogue is open, if either.
   *
   * The two live at opposite ends of the lifecycle on purpose. An **export** is about the
   * open campus file, so it belongs in the title bar; an **import** produces a new one and
   * finishes by offering to open it, which would close whatever is open — so it belongs on
   * the start screen, where nothing is. That keeps D9's rule ("an import does not open what
   * it imported, and does not disturb what is open") true of the interface and not only of
   * the backend.
   */
  const [transfer, setTransfer] = useState<"export" | "import" | null>(null);

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
        title: "New Evara campus",
        defaultPath: (await defaultProjectFolder()) ?? undefined,
        // The filter's name labels the file type in the OS dialogue. The extension itself
        // is unchanged and is not ours to rename here.
        filters: [{ name: "Evara campus", extensions: [extension] }],
      });
      if (!path) return;
      await createProject(path);
    });

  const handleOpen = () =>
    run(async () => {
      const extension = await projectExtension();
      const selected = await openDialog({
        title: "Open Evara campus",
        multiple: false,
        directory: false,
        filters: [{ name: "Evara campus", extensions: [extension] }],
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
          onImport={() => setTransfer("import")}
          onOpenRecent={(path) => run(() => openProject(path))}
          onForget={(path) => run(() => forgetRecentProject(path))}
        />
        <ImportCampus
          open={transfer === "import"}
          onClose={() => setTransfer(null)}
          onImported={() => void refresh().catch(() => undefined)}
          onOpenImported={(path) => run(() => openProject(path))}
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
        onExport={() => setTransfer("export")}
        onDismissError={() => setError(null)}
      />
      {/* Keyed by project so switching projects resets every area's state rather
          than showing the previous project's selections against the new one. */}
      <Workspace key={project.projectId} />
      {/* Keyed by the open file as well: closing one campus and opening another unmounts
          the dialogue, which unsubscribes its listeners and forgets a finished session
          rather than leaving a report about a file nobody has open any more. */}
      <ExportCampus
        key={`export-${project.projectId}`}
        open={transfer === "export"}
        campusName={project.displayName}
        onClose={() => setTransfer(null)}
      />
    </div>
  );
}

/** The two areas an open campus file has, and which one is showing. */
type Area = "setup" | "timetables";

/**
 * The open campus file's workspace: a rail of areas, and the selected school they share.
 *
 * ## Why the school lives here
 *
 * Both areas are about **one** school, and they have to be about the same one. A rail that
 * switched area while quietly changing which school was in view would be worse than no rail:
 * ADR 0023 scopes the interface to one school, and the whole value of that is that the answer
 * is unambiguous. So the selection is held once, here, and handed to both.
 *
 * `SchoolSetup` keeps its own selection when mounted without one, which is what its own
 * tests do and what Stages A–D were written against.
 *
 * ## Switching school remounts the areas
 *
 * `key={activeSchoolId}` on each area is the same stale-state guarantee `SchoolSetup`'s
 * sections already had, extended to the second area: an open edit form, a pending delete, a
 * generation session and an opened timetable are all *about* a school, and re-rendering with
 * a new one would leave every piece of it standing.
 */
function Workspace() {
  const [area, setArea] = useState<Area>("setup");
  const [schoolId, setSchoolId] = useState<string | null>(null);

  return (
    <>
      <nav className="workspaceRail" aria-label="Areas">
        <button
          type="button"
          className={`railTab${area === "setup" ? " railTab--on" : ""}`}
          aria-current={area === "setup" ? "page" : undefined}
          onClick={() => setArea("setup")}
        >
          School Setup
        </button>
        <button
          type="button"
          className={`railTab${area === "timetables" ? " railTab--on" : ""}`}
          aria-current={area === "timetables" ? "page" : undefined}
          onClick={() => setArea("timetables")}
        >
          Timetables
        </button>
      </nav>
      <main className="appMain">
        {area === "setup" ? (
          <SchoolSetup
            key={`setup-${schoolId ?? "none"}`}
            schoolId={schoolId}
            onSchoolChange={setSchoolId}
          />
        ) : (
          <TimetableArea key={`tt-${schoolId ?? "none"}`} schoolId={schoolId} />
        )}
      </main>
    </>
  );
}

function TitleBar(props: {
  project: Project;
  busy: boolean;
  error: string | null;
  onClose: () => void;
  onExport: () => void;
  onDismissError: () => void;
}) {
  const { project, busy, error, onClose, onExport, onDismissError } = props;
  const { logo } = useSchoolLogo();

  return (
    <header className="titleBar">
      {/* A school-specific view, so the school's own logo when it has one. `schoolLogo`
          is undefined until it has been read, which resolves to the Evara mark -- the
          same thing a school without a logo gets, so nothing flickers between two
          different images on load. */}
      <LogoMark className="titleBarLogo" logo={resolveLogo(logo)} size={36} />
      <div className="titleBarMain">
        <span className="eyebrow">CAMPUS</span>
        <h1 title={project.path}>{project.displayName}</h1>
        <span className="titleBarMeta">
          {project.folder} · schema v{project.schemaVersion}
        </span>
      </div>

      <div className="titleBarActions">
        <button
          type="button"
          className="secondary"
          onClick={onExport}
          disabled={busy}
          // "campus file", not "campus": a "Campuses and buildings" section is one click
          // away, and this exports the file rather than one of those.
          title="Write this campus file to a portable file you can copy or send"
        >
          Export campus file…
        </button>
        <button
          type="button"
          className="secondary"
          onClick={onClose}
          disabled={busy}
          // The title bar sits directly above a "Campuses and buildings" section, so this
          // button is the one place the two meanings of "campus" could be mistaken for
          // each other. The label follows the agreed wording; the tooltip says which one
          // is being closed.
          title="Close this campus file and return to the start screen"
        >
          Close campus
        </button>
      </div>

      {project.syncedFolder && (
        <Banner tone="warning">
          <strong>This campus is stored in {project.syncedFolder}.</strong> An open Evara
          campus is a database written continuously, and sync clients can copy its files
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
  onImport: () => void;
  onOpenRecent: (path: string) => void;
  onForget: (path: string) => void;
}) {
  const { recents, busy, error, onNew, onOpen, onImport, onOpenRecent, onForget } = props;

  return (
    <section className="panel">
      {/* The application's own branding area. Always Evara's mark, whatever campus was
          last open -- a school's logo never stands in for the product's. */}
      <LogoMark className="startLogo" logo={evaraBrand()} size={64} />
      <span className="eyebrow">OPEN SOURCE · LOCAL FIRST</span>
      <h1>Evara Timetables</h1>
      <p>
        Create a campus to get started, or open one you already have. Everything stays on
        this machine.
      </p>

      <div className="actions">
        <button type="button" onClick={onNew} disabled={busy}>
          New campus
        </button>
        <button type="button" className="secondary" onClick={onOpen} disabled={busy}>
          Open campus…
        </button>
        <button type="button" className="secondary" onClick={onImport} disabled={busy}>
          Import campus…
        </button>
      </div>

      {error && <Banner tone="error">{error}</Banner>}

      {/* What startup recovery did about an interrupted import, if anything. It blocks none
          of the three actions above: an interrupted import is about one path. */}
      <RecoveryNotices />

      <h2 className="sectionHead">Recent campuses</h2>
      {recents.length === 0 ? (
        <p className="muted">
          No recent campuses. Ones you create or open will appear here.
        </p>
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
                <span>
                  {entry.exists ? entry.folder : "File not found"}
                  {/* D13: two entries carrying one identity are the same campus, copied or
                      imported twice. Said here as well as at the moment of import, because
                      that is where somebody is choosing between them — and Evara cannot
                      merge them, so disclosure is the whole remedy. */}
                  {entry.duplicateIdentity && (
                    <em className="recentTwin"> · another copy of this campus is listed</em>
                  )}
                </span>
              </button>
              <button
                type="button"
                className="linkish"
                disabled={busy}
                onClick={() => onForget(entry.path)}
                aria-label={`Remove ${entry.displayName} from recent campuses`}
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
