/**
 * Proves the typed IPC boundary: a command wrapper validates what the backend returns
 * instead of trusting it. There is no Tauri runtime under Vitest, so `invoke` is mocked.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { appInfo } = await import("./index");

describe("appInfo", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("returns the payload when the backend answers correctly", async () => {
    invoke.mockResolvedValue({ version: "0.1.0", debug: true, phase: "0 — infrastructure" });

    await expect(appInfo()).resolves.toEqual({
      version: "0.1.0",
      debug: true,
      phase: "0 — infrastructure",
    });
    expect(invoke).toHaveBeenCalledWith("app_info");
  });

  it("rejects a payload that does not match the contract", async () => {
    invoke.mockResolvedValue({ version: 1, debug: "yes" });

    await expect(appInfo()).rejects.toThrow(/unexpected payload/);
  });

  it("propagates a backend failure", async () => {
    invoke.mockRejectedValue(new Error("command app_info not found"));

    await expect(appInfo()).rejects.toThrow(/not found/);
  });
});
