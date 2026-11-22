/**
 * The Import dialogue.
 *
 * The Rust tests own the reader, the verification order, the staging lifecycle and the
 * replacement protocol. What only the interface can get wrong is here:
 *
 * - choosing a file **inspects** it and imports nothing;
 * - the integrity notice and the "this is not the payload check" sentence are both shown;
 * - an archive the backend refuses offers no import — the screen gets no vote;
 * - D13's identity warning appears *before* anything is staged, and says two copies are
 *   legitimate;
 * - the first attempt is always `"refuse"`, and Replace is a second, explicit click;
 * - the kept copy is named;
 * - a cancellation and a failure read differently;
 * - a successful import **never** opens itself — "Open now" is the user's click.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  ArchiveSummaryView,
  ImportReportView,
  ImportStatus,
  TransferFinishedEvent,
  TransferProgressEvent,
} from "@evara/domain";

const archiveExtension = vi.fn<() => Promise<string>>();
const inspectArchive = vi.fn<(path: string) => Promise<ArchiveSummaryView>>();
const startImport = vi.fn();
const importStatus = vi.fn<(runId: string) => Promise<ImportStatus>>();
const cancelImport = vi.fn();
const discardImport = vi.fn();
const onTransferProgress = vi.fn();
const onTransferFinished = vi.fn();

const hasKind = (error: unknown, ...kinds: string[]) =>
  typeof error === "object" &&
  error !== null &&
  kinds.includes((error as { kind?: string }).kind ?? "");

vi.mock("../../ipc/transfer", () => ({
  archiveExtension: () => archiveExtension(),
  inspectArchive: (path: string) => inspectArchive(path),
  startImport: (archive: unknown, destination: unknown, onConflict: unknown) =>
    startImport(archive, destination, onConflict),
  importStatus: (runId: string) => importStatus(runId),
  cancelImport: (runId: unknown) => cancelImport(runId),
  discardImport: (runId: unknown) => discardImport(runId),
  onTransferProgress: (runId: unknown, handler: unknown) => onTransferProgress(runId, handler),
  onTransferFinished: (runId: unknown, handler: unknown) => onTransferFinished(runId, handler),
  isDestinationConflict: (e: unknown) =>
    hasKind(e, "alreadyExists", "destinationIsOpen", "transferBlocked"),
  isTooOldToRead: (e: unknown) => hasKind(e, "formatTooNew", "schemaTooNew"),
  isArchiveDamaged: (e: unknown) => hasKind(e, "archiveDamaged"),
}));

const projectExtension = vi.fn<() => Promise<string>>();
vi.mock("../../ipc/project", () => ({
  projectExtension: () => projectExtension(),
  isProjectError: (value: unknown) =>
    typeof value === "object" && value !== null && "kind" in value,
  describeError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : "Something went wrong. Please try again.",
}));

const openFileDialog = vi.fn<(options: unknown) => Promise<string | null>>();
const saveDialog = vi.fn<(options: unknown) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (options: unknown) => openFileDialog(options),
  save: (options: unknown) => saveDialog(options),
}));

const { ImportCampus } = await import("./ImportCampus");

const RUN = "run-11";
const ARCHIVE = "C:/Inbox/Northgate.evara";
const DESTINATION = "C:/Schools/Northgate.evaraproj";

const INTEGRITY =
  "The digests in this archive detect corruption, not authenticity. Anyone can rewrite an " +
  "archive and recompute every digest inside it, so a campus file carries exactly the " +
  "authority of the person who sent it to you.";

function aSummary(over: Partial<ArchiveSummaryView> = {}): ArchiveSummaryView {
  return {
    path: ARCHIVE,
    fileName: "Northgate.evara",
    archiveBytes: 4096,
    format: "evara-project",
    formatVersion: 1,
    generatorApp: "Evara Timetables",
    generatorVersion: "0.1.0",
    exportedAt: "2026-11-20T08:00:00.000Z",
    projectId: "p-1",
    createdBySiteId: null,
    exportedBySiteId: null,
    schemaVersion: 18,
    payloadBytes: 16384,
    schools: [{ id: "s-1", name: "Northgate", campuses: 2 }],
    changeLogEntries: 300,
    changeLogWatermark: 300,
    counts: { school: 1 },
    tablesWithRows: 22,
    totalRows: 1840,
    capabilities: [],
    unknownEntries: [],
    readable: true,
    refusal: null,
    suggestedFileName: "Northgate.evaraproj",
    identityConflicts: [],
    identityNotice: null,
    integrityNotice: INTEGRITY,
    ...over,
  };
}

function running(over: Partial<ImportStatus> = {}): ImportStatus {
  return {
    runId: RUN,
    phase: "running",
    step: "staging",
    done: 0,
    total: 16384,
    cancelRequested: false,
    archive: ARCHIVE,
    destination: DESTINATION,
    report: null,
    failure: null,
    ...over,
  };
}

function aReport(over: Partial<ImportReportView> = {}): ImportReportView {
  return {
    path: DESTINATION,
    fileName: "Northgate.evaraproj",
    projectId: "p-1",
    schemaVersionFound: 18,
    schemaVersionNow: 18,
    migrated: false,
    counts: { school: 1 },
    tablesWithRows: 22,
    totalRows: 1840,
    replacedCopyKeptAt: null,
    replacedCopyName: null,
    unknownEntries: [],
    countDisagreements: [],
    remembered: true,
    ...over,
  };
}

const handlerOf = (mock: typeof onTransferProgress) =>
  mock.mock.calls.at(-1)![1] as (event: unknown) => void;

function fireProgress(over: Partial<TransferProgressEvent> = {}) {
  handlerOf(onTransferProgress)({
    runId: RUN,
    epoch: 2,
    kind: "import",
    step: "verifying",
    done: 8192,
    total: 16384,
    ...over,
  });
}

function fireFinished(over: Partial<TransferFinishedEvent> = {}) {
  handlerOf(onTransferFinished)({
    runId: RUN,
    epoch: 2,
    kind: "import",
    phase: "finished",
    ok: true,
    ...over,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  archiveExtension.mockResolvedValue("evara");
  projectExtension.mockResolvedValue("evaraproj");
  openFileDialog.mockResolvedValue(ARCHIVE);
  saveDialog.mockResolvedValue(DESTINATION);
  inspectArchive.mockResolvedValue(aSummary());
  startImport.mockResolvedValue({ runId: RUN, totalBytes: 16384 });
  importStatus.mockResolvedValue(running());
  cancelImport.mockResolvedValue(true);
  discardImport.mockResolvedValue(undefined);
  onTransferProgress.mockResolvedValue(() => undefined);
  onTransferFinished.mockResolvedValue(() => undefined);
});

function open() {
  const onClose = vi.fn();
  const onOpenImported = vi.fn();
  const onImported = vi.fn();
  render(
    <ImportCampus
      open
      onClose={onClose}
      onOpenImported={onOpenImported}
      onImported={onImported}
    />,
  );
  return { onClose, onOpenImported, onImported };
}

/** Chooses the archive and waits for the inspection to land. */
async function inspected(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Choose a portable file…" }));
  await screen.findByRole("button", { name: "Choose where to save…" });
}

/** Chooses the archive and the destination, reaching a running import. */
async function startRunning(user: ReturnType<typeof userEvent.setup>) {
  await inspected(user);
  await user.click(screen.getByRole("button", { name: "Choose where to save…" }));
  await waitFor(() => expect(onTransferProgress).toHaveBeenCalled());
}

describe("choosing an archive", () => {
  it("inspects it and imports nothing", async () => {
    const user = userEvent.setup();
    open();

    await inspected(user);

    expect(inspectArchive).toHaveBeenCalledWith(ARCHIVE);
    expect(startImport).not.toHaveBeenCalled();
    const options = openFileDialog.mock.calls.at(-1)?.[0] as {
      filters: { extensions: string[] }[];
      multiple: boolean;
    };
    expect(options.filters[0]?.extensions).toEqual(["evara"]);
    expect(options.multiple).toBe(false);
  });

  it("shows what the archive says about itself", async () => {
    const user = userEvent.setup();
    open();

    await inspected(user);

    expect(screen.getByText("Northgate.evara")).toBeTruthy();
    expect(screen.getByText("Evara Timetables 0.1.0")).toBeTruthy();
    expect(screen.getByText("Northgate (2 campuses)")).toBeTruthy();
    expect(screen.getByText("v18")).toBeTruthy();
    expect(screen.getByText("1840 in 22 tables")).toBeTruthy();
    expect(screen.getByText(/4.0 KiB \(16.0 KiB inside\)/)).toBeTruthy();
  });

  it("distinguishes a readable container from a verified campus", async () => {
    const user = userEvent.setup();
    open();

    await inspected(user);

    // The sentence that keeps a clean inspection from reading as a guarantee.
    expect(screen.getByText(/Evara can read this file/)).toBeTruthy();
    expect(screen.getByText(/checked against that record during the import — not now/))
      .toBeTruthy();
  });

  it("always shows what the digests do not prove", async () => {
    const user = userEvent.setup();
    open();

    await inspected(user);

    // Verbatim from the backend: one source of this wording, beside the rule.
    expect(screen.getByText(INTEGRITY)).toBeTruthy();
  });

  it("reports an unknown extra entry as ignored rather than as a problem", async () => {
    const user = userEvent.setup();
    inspectArchive.mockResolvedValue(aSummary({ unknownEntries: ["notes.txt"] }));
    open();

    await inspected(user);

    expect(screen.getByText(/also carries notes.txt/)).toBeTruthy();
  });
});

describe("an archive this build cannot read", () => {
  it("shows the backend's refusal and offers no import", async () => {
    const user = userEvent.setup();
    inspectArchive.mockResolvedValue(
      aSummary({
        readable: false,
        refusal: {
          kind: "formatTooNew",
          found: 9,
          supported: 1,
          message:
            "This portable campus file was written by a newer version of Evara (file format v9; this version reads up to v1). Update Evara to import it.",
        },
      }),
    );
    open();

    await user.click(screen.getByRole("button", { name: "Choose a portable file…" }));

    expect(await screen.findByText(/written by a newer version of Evara/)).toBeTruthy();
    // The screen gets no vote on a refusal: there is nothing to click that would import it.
    expect(screen.queryByRole("button", { name: "Choose where to save…" })).toBeNull();
    expect(screen.getByRole("button", { name: "Choose a different file…" })).toBeTruthy();
  });

  it("shows a damaged archive's remedy", async () => {
    const user = userEvent.setup();
    inspectArchive.mockRejectedValue({
      kind: "archiveDamaged",
      message: "`payload/project.evaraproj` in this archive is damaged.",
    });
    open();

    await user.click(screen.getByRole("button", { name: "Choose a portable file…" }));

    expect(await screen.findByText(/is damaged/)).toBeTruthy();
    expect(screen.getByText(/Ask whoever sent it for a fresh copy/)).toBeTruthy();
  });

  it("shows an out-of-date build's remedy", async () => {
    const user = userEvent.setup();
    inspectArchive.mockRejectedValue({
      kind: "schemaTooNew",
      found: 99,
      supported: 18,
      message: "That campus was saved by a newer version of Evara.",
    });
    open();

    await user.click(screen.getByRole("button", { name: "Choose a portable file…" }));

    expect(await screen.findByText(/saved by a newer version/)).toBeTruthy();
    expect(screen.getByText(/Update Evara and try again/)).toBeTruthy();
  });
});

describe("a campus already on this machine — D13", () => {
  it("warns before anything is staged and says two copies are legitimate", async () => {
    const user = userEvent.setup();
    inspectArchive.mockResolvedValue(
      aSummary({
        identityNotice:
          "A campus with this identity is already on this machine: Northgate.evaraproj in Schools. Importing creates a second copy. Evara cannot merge two copies, so changes you make in one will not appear in the other.",
        identityConflicts: [
          {
            path: DESTINATION,
            folder: "C:/Schools",
            displayName: "Northgate",
            lastOpenedAt: "2026-11-01T00:00:00.000Z",
            exists: true,
            projectId: "p-1",
            duplicateIdentity: false,
          },
        ],
      }),
    );
    open();

    await inspected(user);

    expect(screen.getByText(/already on this machine: Northgate.evaraproj in Schools/))
      .toBeTruthy();
    expect(screen.getByText(/Evara cannot merge two copies/)).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Already on this machine" })).toBeTruthy();
    expect(screen.getByText(/can legitimately be the same campus/)).toBeTruthy();
    // Nothing is offered that would deduplicate, delete or renumber.
    expect(screen.queryByRole("button", { name: /merge/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /delete/i })).toBeNull();
    // And it is still importable: a duplicate identity is a disclosure, not a refusal.
    expect(screen.getByRole("button", { name: "Choose where to save…" })).toBeTruthy();
  });
});

describe("importing without replacing", () => {
  it("asks the backend to refuse an occupied destination", async () => {
    const user = userEvent.setup();
    open();
    await inspected(user);

    await user.click(screen.getByRole("button", { name: "Choose where to save…" }));

    await waitFor(() =>
      expect(startImport).toHaveBeenCalledWith(ARCHIVE, DESTINATION, "refuse"),
    );
    const options = saveDialog.mock.calls.at(-1)?.[0] as {
      defaultPath: string;
      filters: { extensions: string[] }[];
    };
    // The name the backend suggested, and the live campus extension — not the archive's.
    expect(options.defaultPath).toBe("Northgate.evaraproj");
    expect(options.filters[0]?.extensions).toEqual(["evaraproj"]);
  });

  it("starts nothing when the destination dialogue is cancelled", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue(null);
    open();
    await inspected(user);

    await user.click(screen.getByRole("button", { name: "Choose where to save…" }));

    await waitFor(() => expect(saveDialog).toHaveBeenCalled());
    expect(startImport).not.toHaveBeenCalled();
  });
});

describe("replacing a campus file", () => {
  const occupied = {
    kind: "alreadyExists",
    message:
      "This replaces Northgate.evaraproj, which is the same campus. Your current copy is kept beside it as Northgate.replaced-<time>.evaraproj.",
  };

  it("needs a second, explicit click, and names the kept copy first", async () => {
    const user = userEvent.setup();
    startImport.mockRejectedValueOnce(occupied);
    open();
    await inspected(user);

    await user.click(screen.getByRole("button", { name: "Choose where to save…" }));

    // The backend's own sentence, which names the campus and the keep-name.
    expect(await screen.findByText(/which is the same campus/)).toBeTruthy();
    expect(screen.getByText(/kept beside it as Northgate.replaced/)).toBeTruthy();
    expect(screen.getByText(/Nothing is deleted/)).toBeTruthy();
    // Nothing has been replaced yet: the refusal was the whole of that attempt.
    expect(startImport).toHaveBeenCalledTimes(1);
    expect(startImport).toHaveBeenLastCalledWith(ARCHIVE, DESTINATION, "refuse");

    await user.click(screen.getByRole("button", { name: "Replace and keep a copy" }));

    await waitFor(() =>
      expect(startImport).toHaveBeenLastCalledWith(ARCHIVE, DESTINATION, "replace"),
    );
  });

  it("offers another name as well as replacing", async () => {
    const user = userEvent.setup();
    startImport.mockRejectedValueOnce(occupied);
    open();
    await inspected(user);
    await user.click(screen.getByRole("button", { name: "Choose where to save…" }));
    await screen.findByRole("button", { name: "Replace and keep a copy" });

    saveDialog.mockResolvedValue("C:/Schools/Northgate (2).evaraproj");
    await user.click(screen.getByRole("button", { name: "Choose another name…" }));

    await waitFor(() =>
      expect(startImport).toHaveBeenLastCalledWith(
        ARCHIVE,
        "C:/Schools/Northgate (2).evaraproj",
        "refuse",
      ),
    );
  });

  it("will not offer to replace a campus file that is open", async () => {
    const user = userEvent.setup();
    startImport.mockRejectedValueOnce({
      kind: "destinationIsOpen",
      fileName: "Northgate.evaraproj",
      message: "Northgate.evaraproj is open. Close it before replacing it.",
    });
    open();
    await inspected(user);

    await user.click(screen.getByRole("button", { name: "Choose where to save…" }));

    expect(await screen.findByText(/is open. Close it before replacing it./)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Replace and keep a copy" })).toBeNull();
    expect(screen.getByText(/Choose another name, or close the campus file first/))
      .toBeTruthy();
  });

  it("names the kept copy in the result", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(
      running({
        phase: "finished",
        report: aReport({
          replacedCopyName: "Northgate.replaced-2026-11-22T09-14-22Z.evaraproj",
          replacedCopyKeptAt: "C:/Schools/Northgate.replaced-2026-11-22T09-14-22Z.evaraproj",
        }),
      }),
    );
    fireFinished();

    // Twice: the name in the sentence, and the full path beside it.
    expect(
      await screen.findAllByText(/Northgate.replaced-2026-11-22T09-14-22Z.evaraproj/),
    ).toHaveLength(2);
    expect(screen.getByText(/Evara never deletes it/)).toBeTruthy();
  });
});

describe("while it runs", () => {
  it("shows the step and the share, and ignores another epoch's event", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    fireProgress();
    fireProgress({ epoch: 77, step: "finalising", done: 16384 });

    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("50");
    expect(screen.getByText(/Verifying what arrived/)).toBeTruthy();
  });

  it("asks the worker to stop", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    await user.click(screen.getByRole("button", { name: "Stop" }));

    await waitFor(() => expect(cancelImport).toHaveBeenCalledWith(RUN));
  });

  it("reports a cancellation as nothing written", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(running({ phase: "cancelled", cancelRequested: true }));
    fireFinished({ phase: "cancelled", ok: false });

    expect(await screen.findByText(/The import was stopped/)).toBeTruthy();
    expect(screen.getByText(/No campus file was written/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("shows the backend's sentence when it fails", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(
      running({
        phase: "failed",
        failure: { kind: "archiveDamaged", message: "That archive is incomplete." },
      }),
    );
    fireFinished({ phase: "failed", ok: false });

    expect(await screen.findByText("That archive is incomplete.")).toBeTruthy();
  });
});

describe("a successful import", () => {
  it("never opens itself, and offers Open now as the user's own click", async () => {
    const user = userEvent.setup();
    const { onOpenImported, onClose } = open();
    await startRunning(user);

    importStatus.mockResolvedValue(running({ phase: "finished", report: aReport() }));
    fireFinished();

    expect(await screen.findByText(/is ready/)).toBeTruthy();
    // D9: nothing was opened by the import itself.
    expect(onOpenImported).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Open now" }));

    expect(onOpenImported).toHaveBeenCalledWith(DESTINATION);
    expect(onClose).toHaveBeenCalled();
    await waitFor(() => expect(discardImport).toHaveBeenCalledWith(RUN));
  });

  it("says when a migration ran", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(
      running({
        phase: "finished",
        report: aReport({ migrated: true, schemaVersionFound: 12, schemaVersionNow: 18 }),
      }),
    );
    fireFinished();

    expect(await screen.findByText(/brought up to date: v12 → v18/)).toBeTruthy();
  });

  it("reports a count disagreement as advisory, not as a failure", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(
      running({
        phase: "finished",
        report: aReport({
          countDisagreements: [{ table: "teacher", declared: 10, found: 9 }],
        }),
      }),
    );
    fireFinished();

    expect(await screen.findByText(/teacher said 10, holds 9/)).toBeTruthy();
    // Still a success: the campus arrived and the report is on screen.
    expect(screen.getByText(/is ready/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open now" })).toBeTruthy();
  });

  it("says so when the recent list could not be updated", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    importStatus.mockResolvedValue(
      running({ phase: "finished", report: aReport({ remembered: false }) }),
    );
    fireFinished();

    expect(await screen.findByText(/could not be added to the recent list/)).toBeTruthy();
  });

  it("refreshes the recent list when the dialogue is closed instead", async () => {
    const user = userEvent.setup();
    const { onImported, onOpenImported } = open();
    await startRunning(user);

    importStatus.mockResolvedValue(running({ phase: "finished", report: aReport() }));
    fireFinished();
    await screen.findByText(/is ready/);

    await user.click(screen.getByRole("button", { name: "Close" }));

    expect(onImported).toHaveBeenCalled();
    expect(onOpenImported).not.toHaveBeenCalled();
  });
});

describe("the keyboard", () => {
  it("reaches the first action with one Tab and runs it with Enter", async () => {
    const user = userEvent.setup();
    open();

    await user.tab();
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "Choose a portable file…" }),
    );

    await user.keyboard("{Enter}");
    await waitFor(() => expect(inspectArchive).toHaveBeenCalled());
  });

  it("closes on the dialogue's own cancel, and discards nothing that is still running", async () => {
    const user = userEvent.setup();
    const { onClose } = open();
    await startRunning(user);

    // The `cancel` event is what Escape fires on a `<dialog>`. A running transfer treats it
    // as a request to stop rather than as a dismissal, so the screen cannot be left with a
    // worker nobody is watching.
    screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));

    await waitFor(() => expect(cancelImport).toHaveBeenCalledWith(RUN));
    expect(onClose).not.toHaveBeenCalled();
  });
});
