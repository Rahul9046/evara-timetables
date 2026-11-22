/**
 * The interrupted-import notices.
 *
 * The protocol's own tests own what recovery *does*. What only the interface can get wrong:
 *
 * - an unresolved record cannot be dismissed, because a warning the application quietly
 *   forgets is a warning nobody acts on;
 * - a resolved one can, because it is information;
 * - recovery is never run again from here;
 * - with nothing to report the screen says nothing at all.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { RecoveredView } from "@evara/domain";

const recoveryReport = vi.fn<() => Promise<RecoveredView[]>>();
const recoverPending = vi.fn();

vi.mock("../../ipc/transfer", () => ({
  recoveryReport: () => recoveryReport(),
  // Present only so a test would fail if the component ever reached for it. Recovery runs
  // once, at startup, before any campus file is open — never from React.
  recoverPending: () => recoverPending(),
}));

const { RecoveryNotices } = await import("./RecoveryNotices");

function entry(over: Partial<RecoveredView> = {}): RecoveredView {
  return {
    destination: "C:/Schools/Northgate.evaraproj",
    fileName: "Northgate.evaraproj",
    outcome: "the interrupted import was rolled back; nothing was changed",
    resolved: true,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  recoveryReport.mockResolvedValue([]);
});

describe("with nothing to report", () => {
  it("renders nothing at all", async () => {
    const { container } = render(<RecoveryNotices />);

    await vi.waitFor(() => expect(recoveryReport).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });
});

describe("a record Evara resolved", () => {
  it("says what it did, politely, and can be dismissed", async () => {
    const user = userEvent.setup();
    recoveryReport.mockResolvedValue([entry()]);
    render(<RecoveryNotices />);

    const note = await screen.findByText(/was rolled back; nothing was changed/);
    // `status`, not `alert`: the user did not do anything wrong and it is already handled.
    expect(note.getAttribute("role")).toBe("status");

    await user.click(
      screen.getByRole("button", { name: "Dismiss the note about Northgate.evaraproj" }),
    );

    expect(screen.queryByText(/was rolled back/)).toBeNull();
    // And it was never re-run to find that out.
    expect(recoverPending).not.toHaveBeenCalled();
    expect(recoveryReport).toHaveBeenCalledTimes(1);
  });

  it("names the kept copy when the import had already finished", async () => {
    recoveryReport.mockResolvedValue([
      entry({
        outcome:
          "the import finished; your previous campus file is kept as Northgate.replaced-2026-11-22T09-14-22Z.evaraproj",
      }),
    ]);
    render(<RecoveryNotices />);

    expect(
      await screen.findByText(/kept as Northgate.replaced-2026-11-22T09-14-22Z.evaraproj/),
    ).toBeTruthy();
  });
});

describe("a record Evara could not resolve", () => {
  it("stays on screen, says nothing was changed, and names the folder", async () => {
    recoveryReport.mockResolvedValue([
      entry({
        resolved: false,
        outcome:
          "a campus file, a half-finished import and a previous copy are all present, so Evara cannot tell which is current",
      }),
    ]);
    render(<RecoveryNotices />);

    expect(await screen.findByText(/cannot tell which is current/)).toBeTruthy();
    expect(screen.getByText(/Evara changed nothing here/)).toBeTruthy();
    expect(screen.getByText(/C:\/Schools\/Northgate.evaraproj/)).toBeTruthy();
    // There is no way to make it go away: that is the point of an unresolved record.
    expect(screen.queryByRole("button", { name: /dismiss/i })).toBeNull();
  });

  it("shows both kinds together without confusing them", async () => {
    recoveryReport.mockResolvedValue([
      entry({ destination: "C:/A/One.evaraproj", fileName: "One.evaraproj" }),
      entry({
        destination: "C:/B/Two.evaraproj",
        fileName: "Two.evaraproj",
        resolved: false,
        outcome: "something Evara did not write is at that path",
      }),
    ]);
    render(<RecoveryNotices />);

    await screen.findByText(/something Evara did not write/);
    // One dismiss button, for the resolved one only.
    const dismissals = screen.getAllByRole("button", { name: /dismiss/i });
    expect(dismissals).toHaveLength(1);
    expect(dismissals[0]?.getAttribute("aria-label")).toBe(
      "Dismiss the note about One.evaraproj",
    );
  });
});
