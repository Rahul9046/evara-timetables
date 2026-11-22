/**
 * The Export dialogue.
 *
 * The Rust tests own the protocol: the snapshot, the archive, the atomic destination claim
 * and every refusal. What only the interface can get wrong is here:
 *
 * - the dialogue filters on the extension the **backend** names, and never a literal;
 * - a cancelled file dialogue starts nothing;
 * - progress comes from `transfer://progress` and an event from another epoch is ignored;
 * - a cancellation and a failure are different outcomes with different words;
 * - an occupied destination is an offer of another name, never an overwrite;
 * - two clicks start one export;
 * - a session that finishes before the listener exists still settles the screen;
 * - a finished session is discarded when the dialogue closes.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type {
  ExportReportView,
  ExportStatus,
  TransferFinishedEvent,
  TransferProgressEvent,
} from "@evara/domain";

const archiveExtension = vi.fn<() => Promise<string>>();
const startExport = vi.fn();
const exportStatus = vi.fn<(runId: string) => Promise<ExportStatus>>();
const cancelExport = vi.fn();
const discardExport = vi.fn();
const onTransferProgress = vi.fn();
const onTransferFinished = vi.fn();

vi.mock("../../ipc/transfer", () => ({
  archiveExtension: () => archiveExtension(),
  startExport: (path: unknown) => startExport(path),
  exportStatus: (runId: string) => exportStatus(runId),
  cancelExport: (runId: unknown) => cancelExport(runId),
  discardExport: (runId: unknown) => discardExport(runId),
  onTransferProgress: (runId: unknown, handler: unknown) => onTransferProgress(runId, handler),
  onTransferFinished: (runId: unknown, handler: unknown) => onTransferFinished(runId, handler),
  isDestinationConflict: (error: unknown) =>
    typeof error === "object" &&
    error !== null &&
    ["alreadyExists", "destinationIsOpen", "transferBlocked"].includes(
      (error as { kind?: string }).kind ?? "",
    ),
}));

vi.mock("../../ipc/project", () => ({
  describeError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : "Something went wrong. Please try again.",
}));

const saveDialog = vi.fn<(options: unknown) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (options: unknown) => saveDialog(options),
  open: vi.fn(),
}));

const { ExportCampus } = await import("./ExportCampus");

const RUN = "run-7";
const DESTINATION = "C:/Shared/Northgate Secondary.evara";

function running(over: Partial<ExportStatus> = {}): ExportStatus {
  return {
    runId: RUN,
    phase: "running",
    step: "snapshotting",
    done: 0,
    total: 4096,
    cancelRequested: false,
    destination: DESTINATION,
    report: null,
    failure: null,
    ...over,
  };
}

function aReport(over: Partial<ExportReportView> = {}): ExportReportView {
  return {
    path: DESTINATION,
    fileName: "Northgate Secondary.evara",
    archiveBytes: 2048,
    payloadBytes: 8192,
    payloadSha256: "a".repeat(64),
    compressionSavingPercent: 75,
    projectId: "p-1",
    schemaVersion: 18,
    schools: [{ id: "s-1", name: "Northgate", campuses: 2 }],
    capabilities: [],
    changeLogEntries: 412,
    changeLogWatermark: 412,
    exportedAt: "2026-11-22T09:14:22.000Z",
    ...over,
  };
}

/** The handler the dialogue registered for one of the two events. */
const handlerOf = (mock: typeof onTransferProgress) =>
  mock.mock.calls.at(-1)![1] as (event: unknown) => void;

function fireProgress(over: Partial<TransferProgressEvent> = {}) {
  handlerOf(onTransferProgress)({
    runId: RUN,
    epoch: 3,
    kind: "export",
    step: "compressing",
    done: 2048,
    total: 4096,
    ...over,
  });
}

function fireFinished(over: Partial<TransferFinishedEvent> = {}) {
  handlerOf(onTransferFinished)({
    runId: RUN,
    epoch: 3,
    kind: "export",
    phase: "finished",
    ok: true,
    ...over,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  archiveExtension.mockResolvedValue("evara");
  saveDialog.mockResolvedValue(DESTINATION);
  startExport.mockResolvedValue({ runId: RUN, totalBytes: 4096 });
  exportStatus.mockResolvedValue(running());
  cancelExport.mockResolvedValue(true);
  discardExport.mockResolvedValue(undefined);
  onTransferProgress.mockResolvedValue(() => undefined);
  onTransferFinished.mockResolvedValue(() => undefined);
});

function open() {
  const onClose = vi.fn();
  render(<ExportCampus open campusName="Northgate Secondary" onClose={onClose} />);
  return { onClose };
}

/** Clicks through to a running export. */
async function startRunning(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Choose a destination…" }));
  await waitFor(() => expect(onTransferProgress).toHaveBeenCalled());
}

describe("before anything is chosen", () => {
  it("says what an export produces and that the campus file is not changed", () => {
    open();

    expect(screen.getByRole("heading", { name: "Export campus" })).toBeTruthy();
    expect(screen.getByText(/one portable file ending in/)).toBeTruthy();
    expect(screen.getByText(/opened read-only and is not changed/)).toBeTruthy();
    expect(screen.getByText(/Northgate Secondary/)).toBeTruthy();
  });

  it("is reachable and operable from the keyboard alone", async () => {
    const user = userEvent.setup();
    open();

    await user.tab();
    const choose = screen.getByRole("button", { name: "Choose a destination…" });
    expect(document.activeElement).toBe(choose);

    await user.keyboard("{Enter}");
    await waitFor(() => expect(saveDialog).toHaveBeenCalled());
  });
});

describe("choosing a destination", () => {
  it("filters the dialogue on the extension the backend names", async () => {
    const user = userEvent.setup();
    open();

    await user.click(screen.getByRole("button", { name: "Choose a destination…" }));

    await waitFor(() => expect(startExport).toHaveBeenCalledWith(DESTINATION));
    expect(archiveExtension).toHaveBeenCalled();
    const options = saveDialog.mock.calls.at(-1)?.[0] as {
      title: string;
      defaultPath: string;
      filters: { name: string; extensions: string[] }[];
    };
    // The extension is whatever the command answered. Nothing here writes ".evara" out.
    expect(options.filters[0]?.extensions).toEqual(["evara"]);
    expect(options.defaultPath).toBe("Northgate Secondary.evara");
  });

  it("starts nothing when the file dialogue is cancelled", async () => {
    const user = userEvent.setup();
    saveDialog.mockResolvedValue(null);
    open();

    await user.click(screen.getByRole("button", { name: "Choose a destination…" }));

    await waitFor(() => expect(saveDialog).toHaveBeenCalled());
    expect(startExport).not.toHaveBeenCalled();
  });

  it("starts one export however fast the button is clicked twice", async () => {
    const user = userEvent.setup();
    let release: (value: { runId: string; totalBytes: number }) => void = () => undefined;
    startExport.mockReturnValue(
      new Promise<{ runId: string; totalBytes: number }>((resolve) => {
        release = resolve;
      }),
    );
    open();

    const choose = screen.getByRole("button", { name: "Choose a destination…" });
    await user.click(choose);
    await waitFor(() => expect(startExport).toHaveBeenCalledTimes(1));
    // The button is unavailable while the start is in flight, so a second click cannot
    // reach it — and even if it could, the session refuses a second start.
    expect((choose as HTMLButtonElement).disabled).toBe(true);
    release({ runId: RUN, totalBytes: 4096 });
    await waitFor(() => expect(onTransferProgress).toHaveBeenCalled());
    expect(startExport).toHaveBeenCalledTimes(1);
  });
});

describe("while it runs", () => {
  it("shows the step the backend reported and the share of the bytes", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    fireProgress();

    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("50");
    const line = screen.getByText(/Writing the portable file/);
    expect(line.getAttribute("aria-live")).toBe("polite");
    expect(line.textContent).toMatch(/50%/);
  });

  it("ignores an event from another epoch", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    // The first event pins the epoch; a later one from a campus file the user has since
    // closed must not drive the screen in front of them.
    fireProgress({ step: "hashing", done: 1024, total: 4096 });
    fireProgress({ epoch: 99, step: "finalising", done: 4096, total: 4096 });

    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.getByText(/Checking the copy/)).toBeTruthy();
  });

  it("asks the worker to stop and says the stop was asked for", async () => {
    const user = userEvent.setup();
    const { onClose } = open();
    await startRunning(user);

    await user.click(screen.getByRole("button", { name: "Stop" }));

    await waitFor(() => expect(cancelExport).toHaveBeenCalledWith(RUN));
    expect(screen.getByText(/stopping after the current step/)).toBeTruthy();
    // The dialogue stays: a transfer must not be left behind with nobody watching it.
    expect(onClose).not.toHaveBeenCalled();
  });

  it("can be left once a stop has been asked for", async () => {
    const user = userEvent.setup();
    const { onClose } = open();
    await startRunning(user);

    await user.click(screen.getByRole("button", { name: "Stop" }));
    // A cancel is a flag the engine reads between steps, so a modal nobody can get out of
    // while that happens would be worse than letting the worker finish on its own — which
    // it does safely, writing no archive.
    const anyway = await screen.findByRole("button", { name: "Close anyway" });
    await user.click(anyway);

    expect(onClose).toHaveBeenCalled();
    expect(cancelExport).toHaveBeenCalledTimes(1);
  });
});

describe("how it ends", () => {
  it("reports a cancellation as nothing written, not as a failure", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    exportStatus.mockResolvedValue(
      running({ phase: "cancelled", cancelRequested: true, report: null }),
    );
    fireFinished({ phase: "cancelled", ok: false });

    expect(await screen.findByText(/The export was stopped/)).toBeTruthy();
    expect(screen.getByText(/Nothing was written/)).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows the backend's own sentence when it fails", async () => {
    const user = userEvent.setup();
    open();
    await startRunning(user);

    exportStatus.mockResolvedValue(
      running({
        phase: "failed",
        failure: { kind: "failed", message: "The disk is full." },
      }),
    );
    fireFinished({ phase: "failed", ok: false });

    expect(await screen.findByText("The disk is full.")).toBeTruthy();
  });

  it("shows what was written, and discards the session when the dialogue closes", async () => {
    const user = userEvent.setup();
    const { onClose } = open();
    await startRunning(user);

    exportStatus.mockResolvedValue(running({ phase: "finished", report: aReport() }));
    fireFinished();

    expect(await screen.findByText(/is ready/)).toBeTruthy();
    expect(screen.getByText(DESTINATION)).toBeTruthy();
    expect(screen.getByText(/2.0 KiB \(from 8.0 KiB, 75.0% smaller\)/)).toBeTruthy();
    expect(screen.getByText("Northgate (2 campuses)")).toBeTruthy();
    expect(screen.getByText(/412 entries/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() => expect(discardExport).toHaveBeenCalledWith(RUN));
    expect(onClose).toHaveBeenCalled();
  });

  it("settles even when the export finishes before the listener exists", async () => {
    // The start race: `startExport` returns with the worker already running, and React
    // subscribes in an effect afterwards. A small campus can finish in between, and the
    // finished event is then delivered to nobody.
    const user = userEvent.setup();
    exportStatus.mockResolvedValue(running({ phase: "finished", report: aReport() }));
    open();

    await user.click(screen.getByRole("button", { name: "Choose a destination…" }));

    expect(await screen.findByText(/is ready/)).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});

describe("a destination that is not available", () => {
  it("offers another name and never an overwrite", async () => {
    const user = userEvent.setup();
    startExport.mockRejectedValue({
      kind: "alreadyExists",
      message:
        "Northgate Secondary.evara already exists. Choose another name — Evara will not overwrite a portable file you may still need.",
    });
    open();

    await user.click(screen.getByRole("button", { name: "Choose a destination…" }));

    expect(await screen.findByText(/already exists/)).toBeTruthy();
    expect(screen.getByText(/Choose another name\./)).toBeTruthy();
    // No overwrite, no replace, nothing that could destroy the file that is there.
    expect(screen.queryByRole("button", { name: /replace/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /overwrite/i })).toBeNull();
    // And the way forward is a second attempt at a different name.
    expect(screen.getByRole("button", { name: "Try another name…" })).toBeTruthy();
  });

  it("shows an unsafe destination as the backend described it", async () => {
    const user = userEvent.setup();
    startExport.mockRejectedValue({
      kind: "destinationUnsafe",
      fileName: "link.evara",
      message: "Evara will not write to link.evara: the file it names is a link.",
    });
    open();

    await user.click(screen.getByRole("button", { name: "Choose a destination…" }));

    expect(
      await screen.findByText(/the file it names is a link/),
    ).toBeTruthy();
  });
});
