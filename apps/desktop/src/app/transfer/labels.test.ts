/**
 * The transfer screens' formatting.
 *
 * Small, and worth testing because each of these has a case that produces nonsense if it is
 * written the obvious way: a bar at `NaN%`, a size of "1.0 B", an announcement of "nought per
 * cent" for work that has started, or "Invalid Date" where an archive's own timestamp should
 * be.
 */
import { describe, expect, it } from "vitest";

import { bytes, instant, percent, stepLabel } from "./labels";

describe("stepLabel", () => {
  it("names every step the backend can report", () => {
    // The seven `TransferStep` variants. A new one added in Rust without a label here is a
    // type error, which is the point of the exhaustive record.
    expect(stepLabel("snapshotting")).toBe("Taking a copy of the campus");
    expect(stepLabel("hashing")).toBe("Checking the copy");
    expect(stepLabel("compressing")).toBe("Writing the portable file");
    expect(stepLabel("staging")).toBe("Reading the portable file");
    expect(stepLabel("verifying")).toBe("Verifying what arrived");
    expect(stepLabel("migrating")).toBe("Bringing the campus up to date");
    expect(stepLabel("finalising")).toBe("Finishing");
  });

  it("has something to say before the first event", () => {
    expect(stepLabel(null)).toBe("Starting");
  });
});

describe("bytes", () => {
  it("counts bytes exactly below a kibibyte", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(512)).toBe("512 B");
    expect(bytes(1023)).toBe("1023 B");
  });

  it("uses binary units, as a file manager does", () => {
    expect(bytes(1024)).toBe("1.0 KiB");
    expect(bytes(1024 * 1024)).toBe("1.0 MiB");
    expect(bytes(1024 * 1024 * 1024 * 3.5)).toBe("3.5 GiB");
  });

  it("refuses to invent a figure for a number that is not one", () => {
    expect(bytes(Number.NaN)).toBe("—");
    expect(bytes(-1)).toBe("—");
  });
});

describe("percent", () => {
  it("is null when the total is not yet known, rather than zero", () => {
    // `total` is documented as `0` until the backend knows it. A bar that announced "0%"
    // would be claiming no progress on work that has in fact started.
    expect(percent(0, 0)).toBeNull();
    expect(percent(5, 0)).toBeNull();
  });

  it("rounds and clamps", () => {
    expect(percent(1, 3)).toBe(33);
    expect(percent(2, 4)).toBe(50);
    expect(percent(9, 4)).toBe(100);
    expect(percent(-1, 4)).toBe(0);
  });
});

describe("instant", () => {
  it("renders a real timestamp in the viewer's own locale", () => {
    expect(instant("2026-11-22T09:14:22.000Z")).toBe(
      new Date("2026-11-22T09:14:22.000Z").toLocaleString(),
    );
  });

  it("shows an unparseable one as it arrived", () => {
    // It came from whatever wrote the archive. "Invalid Date" tells a user nothing about
    // their file; the string they can quote to whoever sent it does.
    expect(instant("not a date")).toBe("not a date");
  });
});
