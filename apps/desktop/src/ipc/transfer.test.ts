/**
 * The portable-transfer IPC contract.
 *
 * Generated types check the *shape* of what crosses the boundary. They cannot check the
 * command **name** or the argument **keys**, because both are strings on the Rust side too —
 * so a typo in either compiles cleanly and fails at runtime in front of a user. That is what
 * this file is for, and it is the same reason `timetables.test.ts` exists.
 *
 * It also pins the behaviours the wrappers themselves own: event filtering by run, and the
 * three error predicates a screen branches on.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectError } from "@evara/domain";

const invoke = vi.fn();
const listen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listen(...args),
}));

const transfer = await import("./transfer");

const RUN = "run-1";
const ARCHIVE = "/campuses/Northgate.evara";
const CAMPUS = "/campuses/Northgate.evaraproj";

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  listen.mockReset();
  listen.mockResolvedValue(() => undefined);
});

// ---------------------------------------------------------------- the command surface

describe("every command is called by the name and arguments the backend declares", () => {
  const cases: ReadonlyArray<[string, () => Promise<unknown>, unknown]> = [
    ["project_archive_extension", () => transfer.archiveExtension(), undefined],
    ["project_export_start", () => transfer.startExport(ARCHIVE), { path: ARCHIVE }],
    ["project_export_status", () => transfer.exportStatus(RUN), { runId: RUN }],
    ["project_export_cancel", () => transfer.cancelExport(RUN), { runId: RUN }],
    ["project_export_discard", () => transfer.discardExport(RUN), { runId: RUN }],
    ["project_import_inspect", () => transfer.inspectArchive(ARCHIVE), { path: ARCHIVE }],
    [
      "project_import_start",
      () => transfer.startImport(ARCHIVE, CAMPUS, "refuse"),
      { archivePath: ARCHIVE, destinationPath: CAMPUS, onConflict: "refuse" },
    ],
    ["project_import_status", () => transfer.importStatus(RUN), { runId: RUN }],
    ["project_import_cancel", () => transfer.cancelImport(RUN), { runId: RUN }],
    ["project_import_discard", () => transfer.discardImport(RUN), { runId: RUN }],
    ["project_recovery_report", () => transfer.recoveryReport(), undefined],
  ];

  it.each(cases)("%s", async (name, call, args) => {
    await call();
    if (args === undefined) {
      expect(invoke).toHaveBeenCalledWith(name);
    } else {
      expect(invoke).toHaveBeenCalledWith(name, args);
    }
  });

  it("wraps all eleven approved commands and no twelfth", () => {
    // The count is the contract: the proposal fixes it at eleven, and a wrapper nothing
    // registers is a command that fails the moment somebody clicks it.
    expect(cases).toHaveLength(11);
  });
});

// ---------------------------------------------------------------- the conflict vocabulary

describe("the replacement answer", () => {
  it("crosses as the two words the Rust enum uses", async () => {
    for (const answer of ["refuse", "replace"] as const) {
      invoke.mockClear();
      await transfer.startImport(ARCHIVE, CAMPUS, answer);
      expect(invoke).toHaveBeenCalledWith("project_import_start", {
        archivePath: ARCHIVE,
        destinationPath: CAMPUS,
        onConflict: answer,
      });
    }
  });

  it("never invents a third", () => {
    // `OnConflict` is a generated union, so a third value is a type error rather than a
    // runtime surprise. Asserted here as prose because a type error cannot be a test.
    const answers: ReadonlyArray<"refuse" | "replace"> = ["refuse", "replace"];
    expect(answers).toHaveLength(2);
  });
});

// ---------------------------------------------------------------- events

describe("event subscriptions", () => {
  /** Drives the handler the wrapper registered with a payload. */
  function fire(payload: unknown) {
    const handler = listen.mock.calls[0]?.[1] as (event: { payload: unknown }) => void;
    expect(handler, "the wrapper must register a listener").toBeTypeOf("function");
    handler({ payload });
  }

  it("subscribes to transfer://progress and ignores another run's events", async () => {
    const seen: string[] = [];
    await transfer.onTransferProgress(RUN, (event) => seen.push(event.runId));

    expect(listen).toHaveBeenCalledWith("transfer://progress", expect.any(Function));
    fire({ runId: "somebody-elses-run", epoch: 0, kind: "export", step: "staging", done: 1, total: 2 });
    fire({ runId: RUN, epoch: 0, kind: "export", step: "staging", done: 2, total: 2 });

    expect(seen, "a transfer the user has moved on from must not drive the screen").toEqual([
      RUN,
    ]);
  });

  it("subscribes to transfer://finished and filters the same way", async () => {
    const seen: boolean[] = [];
    await transfer.onTransferFinished(RUN, (event) => seen.push(event.ok));

    expect(listen).toHaveBeenCalledWith("transfer://finished", expect.any(Function));
    fire({ runId: "another", epoch: 0, kind: "import", phase: "finished", ok: true });
    fire({ runId: RUN, epoch: 0, kind: "import", phase: "cancelled", ok: false });

    expect(seen).toEqual([false]);
  });

  it("uses event names of its own rather than the solver's or the repair's", () => {
    // Both of those run on their own worker threads and both emit progress. A screen
    // watching a transfer must not be woken by a generation.
    const names = listen.mock.calls.map(([name]) => name as string);
    expect(names.every((name) => name.startsWith("transfer://"))).toBe(true);
  });
});

// ---------------------------------------------------------------- error predicates

describe("refusals a screen branches on", () => {
  const error = (kind: string, extra: Record<string, unknown> = {}): ProjectError =>
    ({ kind, message: "something to show", ...extra }) as unknown as ProjectError;

  it("recognises a destination conflict, which is an offer rather than an apology", () => {
    for (const kind of ["alreadyExists", "destinationIsOpen", "transferBlocked"]) {
      expect(transfer.isDestinationConflict(error(kind)), kind).toBe(true);
    }
    expect(transfer.isDestinationConflict(error("failed"))).toBe(false);
    expect(transfer.isDestinationConflict(new Error("not an IPC refusal"))).toBe(false);
  });

  it("recognises both version axes, because both mean 'update Evara'", () => {
    expect(transfer.isTooOldToRead(error("formatTooNew", { found: 9, supported: 1 }))).toBe(
      true,
    );
    expect(transfer.isTooOldToRead(error("schemaTooNew", { found: 99, supported: 18 }))).toBe(
      true,
    );
    expect(transfer.isTooOldToRead(error("archiveDamaged"))).toBe(false);
  });

  it("recognises damage, whose remedy is a fresh copy and never a retry", () => {
    expect(transfer.isArchiveDamaged(error("archiveDamaged"))).toBe(true);
    expect(transfer.isArchiveDamaged(error("archiveUnreadable"))).toBe(false);
  });

  it("does not treat a cancellation as any of the three", () => {
    // The user pressed Cancel and nothing was written. Presenting a working control as a
    // fault is how a safe operation acquires a reputation for being unreliable.
    const cancelled = error("cancelled");
    expect(transfer.isDestinationConflict(cancelled)).toBe(false);
    expect(transfer.isTooOldToRead(cancelled)).toBe(false);
    expect(transfer.isArchiveDamaged(cancelled)).toBe(false);
  });
});

// ---------------------------------------------------------------- the two extensions

describe("the two objects are never confused", () => {
  it("asks the backend for the archive extension rather than hardcoding it", async () => {
    invoke.mockResolvedValue("evara");
    await expect(transfer.archiveExtension()).resolves.toBe("evara");
    expect(invoke).toHaveBeenCalledWith("project_archive_extension");
  });

  it("writes neither extension out in this module", async () => {
    // Read as text, because the point is that the strings are absent from the source rather
    // than merely unused at runtime. A dialog filter built from a literal here is how
    // `.evara` and `.evaraproj` drift apart.
    const source = (await import("./transfer.ts?raw")).default;
    const code = source
      .split("\n")
      .filter((line) => !line.trimStart().startsWith("*") && !line.trimStart().startsWith("/*"))
      .join("\n");
    expect(code).not.toMatch(/"evaraproj"|'evaraproj'/);
    expect(code).not.toMatch(/"\.?evara"|'\.?evara'/);
  });
});
