/**
 * The date arithmetic, on its own.
 *
 * Two things are worth testing here rather than through the component, because they are
 * the ones that fail silently: that a date never moves by a day, and that an impossible
 * date is refused rather than rounded.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  addDays,
  addMonths,
  DATE_FORMAT,
  daysInMonth,
  describeDate,
  fromIso,
  inMonth,
  isRealDate,
  monthGrid,
  monthName,
  parseTypedDate,
  readTypedDate,
  sameDate,
  toDisplay,
  toIso,
  todayDate,
} from "./dates";

describe("the display format", () => {
  it("is the format the placeholder promises, not an example date", () => {
    expect(DATE_FORMAT).toBe("DD/MM/YYYY");
  });
});

describe("toDisplay", () => {
  it("turns storage into something a person reads", () => {
    expect(toDisplay("2027-09-01")).toBe("01/09/2027");
    expect(toDisplay("2026-12-25")).toBe("25/12/2026");
  });

  it("zero-pads both parts, so the columns line up", () => {
    expect(toDisplay("2027-01-02")).toBe("02/01/2027");
  });

  it("leaves a blank field blank", () => {
    expect(toDisplay("")).toBe("");
    expect(toDisplay(null)).toBe("");
    expect(toDisplay(undefined)).toBe("");
  });

  it("passes anything that is not a stored date through untouched", () => {
    // This is what stops a half-typed value being rewritten under the caret.
    expect(toDisplay("01/09/202")).toBe("01/09/202");
    expect(toDisplay("nonsense")).toBe("nonsense");
    // Shaped like storage, but no such day — not a date, so not reformatted.
    expect(toDisplay("2027-02-31")).toBe("2027-02-31");
  });
});

describe("parseTypedDate", () => {
  it("accepts the format the field asks for", () => {
    expect(parseTypedDate("01/09/2027")).toBe("2027-09-01");
  });

  it("accepts a date typed without the padding", () => {
    expect(parseTypedDate("1/9/2027")).toBe("2027-09-01");
  });

  it("accepts the separators a keyboard makes easy", () => {
    expect(parseTypedDate("01-09-2027")).toBe("2027-09-01");
    expect(parseTypedDate("01.09.2027")).toBe("2027-09-01");
  });

  it("reads a leading four-digit group as a year, so a pasted stored date works", () => {
    expect(parseTypedDate("2027-09-01")).toBe("2027-09-01");
    expect(parseTypedDate("2027/9/1")).toBe("2027-09-01");
  });

  it("ignores surrounding whitespace", () => {
    expect(parseTypedDate("  01/09/2027  ")).toBe("2027-09-01");
  });

  it("refuses a date that names no real day, rather than rolling it over", () => {
    expect(parseTypedDate("31/02/2026")).toBeNull();
    expect(parseTypedDate("31/04/2027")).toBeNull();
    expect(parseTypedDate("00/09/2027")).toBeNull();
    expect(parseTypedDate("01/13/2027")).toBeNull();
    expect(parseTypedDate("01/00/2027")).toBeNull();
  });

  it("refuses a date that is only half typed", () => {
    expect(parseTypedDate("01/09/202")).toBeNull();
    expect(parseTypedDate("01/09/")).toBeNull();
    expect(parseTypedDate("01")).toBeNull();
    expect(parseTypedDate("")).toBeNull();
  });

  it("refuses text that is not a date at all", () => {
    expect(parseTypedDate("first day of term")).toBeNull();
    expect(parseTypedDate("1 September 2027")).toBeNull();
  });

  it("gets leap years right in both directions", () => {
    expect(parseTypedDate("29/02/2028")).toBe("2028-02-29");
    expect(parseTypedDate("29/02/2027")).toBeNull();
    // Divisible by 100 but not by 400: not a leap year, and the usual thing to get wrong.
    expect(parseTypedDate("29/02/2100")).toBeNull();
    expect(parseTypedDate("29/02/2000")).toBe("2000-02-29");
  });

  it("round-trips every day of a leap February", () => {
    for (let day = 1; day <= 29; day += 1) {
      const typed = `${String(day).padStart(2, "0")}/02/2028`;
      const iso = parseTypedDate(typed);
      expect(iso).toBe(`2028-02-${String(day).padStart(2, "0")}`);
      expect(toDisplay(iso as string)).toBe(typed);
    }
  });
});

describe("no value shifts by a timezone", () => {
  /**
   * The failure this guards is a day either side of midnight, which only shows up away
   * from UTC — so the check is run against several offsets rather than whichever one the
   * machine running the tests happens to be in.
   *
   * `TZ` is read by Node when it builds a `Date`, so setting it and re-importing is the
   * real thing rather than a stub. The module holds no state, so a fresh import is all
   * that is needed.
   */
  const ZONES = [
    "UTC",
    "Pacific/Kiritimati", // +14, the furthest ahead
    "Pacific/Niue", // −11, a long way behind
    "Asia/Kolkata", // +05:30, not a whole hour
    "America/Los_Angeles", // observes daylight saving
    "Australia/Lord_Howe", // +10:30/+11, a half-hour saving shift
  ];

  /**
   * The process environment, reached without a Node type dependency.
   *
   * The frontend's `tsconfig` has no `node` types on purpose — nothing in `src` may
   * assume a Node runtime — and this is a test asking the host to pretend to be
   * elsewhere, which is the one legitimate reason to look.
   */
  const env = (globalThis as { process?: { env: Record<string, string | undefined> } })
    .process?.env;

  const original = env?.TZ;

  afterEach(() => {
    if (env) env.TZ = original;
    vi.resetModules();
  });

  for (const zone of ZONES) {
    it(`holds in ${zone}`, async () => {
      if (env) env.TZ = zone;
      vi.resetModules();
      const dates = await import("./dates");

      // Both ends of the year, where an off-by-one day is also an off-by-one year.
      expect(dates.parseTypedDate("01/01/2027")).toBe("2027-01-01");
      expect(dates.parseTypedDate("31/12/2027")).toBe("2027-12-31");
      expect(dates.toDisplay("2027-01-01")).toBe("01/01/2027");
      expect(dates.toDisplay("2027-12-31")).toBe("31/12/2027");

      // The day a daylight-saving change lands on, in a zone that has one.
      expect(dates.parseTypedDate("29/03/2026")).toBe("2026-03-29");
      expect(dates.parseTypedDate("25/10/2026")).toBe("2026-10-25");

      // And the arithmetic the calendar does, over the same boundaries.
      expect(dates.toIso(dates.addDays({ year: 2027, month: 12, day: 31 }, 1))).toBe(
        "2028-01-01",
      );
      expect(dates.toIso(dates.addDays({ year: 2028, month: 2, day: 28 }, 1))).toBe(
        "2028-02-29",
      );
      expect(dates.monthGrid(2027, 9)[0] && dates.toIso(dates.monthGrid(2027, 9)[0]!)).toBe(
        "2027-08-30",
      );
    });
  }
});

describe("fromIso", () => {
  it("reads the canonical form only", () => {
    expect(fromIso("2027-09-01")).toEqual({ year: 2027, month: 9, day: 1 });
    expect(fromIso("2027-9-1")).toBeNull();
    expect(fromIso("01/09/2027")).toBeNull();
    expect(fromIso("")).toBeNull();
  });

  it("rejects a canonical spelling of a day that does not exist", () => {
    expect(fromIso("2027-02-31")).toBeNull();
    expect(fromIso("2027-13-01")).toBeNull();
  });
});

describe("isRealDate", () => {
  it("knows how long each month is", () => {
    expect(isRealDate({ year: 2027, month: 1, day: 31 })).toBe(true);
    expect(isRealDate({ year: 2027, month: 4, day: 31 })).toBe(false);
    expect(isRealDate({ year: 2027, month: 6, day: 30 })).toBe(true);
  });

  it("rejects a month, a day or a year outside the calendar", () => {
    expect(isRealDate({ year: 2027, month: 0, day: 1 })).toBe(false);
    expect(isRealDate({ year: 2027, month: 13, day: 1 })).toBe(false);
    expect(isRealDate({ year: 2027, month: 1, day: 0 })).toBe(false);
    expect(isRealDate({ year: 0, month: 1, day: 1 })).toBe(false);
  });
});

describe("readTypedDate", () => {
  it("reports the numbers as typed, without judging them", () => {
    // The split that lets a validator say "not a real date" rather than "wrong format".
    expect(readTypedDate("31/02/2026")).toEqual({ year: 2026, month: 2, day: 31 });
    expect(readTypedDate("hello")).toBeNull();
  });
});

describe("daysInMonth", () => {
  it("counts February correctly in and out of a leap year", () => {
    expect(daysInMonth(2028, 2)).toBe(29);
    expect(daysInMonth(2027, 2)).toBe(28);
    expect(daysInMonth(2100, 2)).toBe(28);
    expect(daysInMonth(2000, 2)).toBe(29);
  });
});

describe("addMonths", () => {
  it("keeps the day where the target month has one", () => {
    expect(addMonths({ year: 2027, month: 9, day: 15 }, 1)).toEqual({
      year: 2027,
      month: 10,
      day: 15,
    });
  });

  it("clamps rather than overflowing into the next month", () => {
    expect(addMonths({ year: 2027, month: 3, day: 31 }, -1)).toEqual({
      year: 2027,
      month: 2,
      day: 28,
    });
    expect(addMonths({ year: 2028, month: 3, day: 31 }, -1)).toEqual({
      year: 2028,
      month: 2,
      day: 29,
    });
  });

  it("crosses a year in both directions", () => {
    expect(addMonths({ year: 2027, month: 12, day: 1 }, 1)).toEqual({
      year: 2028,
      month: 1,
      day: 1,
    });
    expect(addMonths({ year: 2027, month: 1, day: 1 }, -1)).toEqual({
      year: 2026,
      month: 12,
      day: 1,
    });
    expect(addMonths({ year: 2027, month: 6, day: 1 }, -12)).toEqual({
      year: 2026,
      month: 6,
      day: 1,
    });
  });
});

describe("monthGrid", () => {
  it("is always six whole weeks, so the popup never changes height", () => {
    for (const [year, month] of [
      [2027, 9],
      [2026, 2],
      [2028, 2],
      [2027, 1],
    ] as const) {
      expect(monthGrid(year, month)).toHaveLength(42);
    }
  });

  it("starts on the Monday on or before the first of the month", () => {
    // 1 September 2027 is a Wednesday, so the grid opens on Monday 30 August.
    const grid = monthGrid(2027, 9);
    expect(toIso(grid[0] as never)).toBe("2027-08-30");
    expect(toIso(grid[2] as never)).toBe("2027-09-01");
  });

  it("runs without a gap, and covers the whole month", () => {
    const grid = monthGrid(2028, 2);
    for (let index = 1; index < grid.length; index += 1) {
      expect(toIso(addDays(grid[index - 1] as never, 1))).toBe(toIso(grid[index] as never));
    }
    const own = grid.filter((date) => inMonth(date, 2028, 2));
    expect(own).toHaveLength(29);
  });

  it("starts the week on Monday when the first falls on a Monday", () => {
    // 1 February 2027 is a Monday: no leading days from January.
    expect(toIso(monthGrid(2027, 2)[0] as never)).toBe("2027-02-01");
  });

  it("starts the week on Monday when the first falls on a Sunday", () => {
    // 1 August 2027 is a Sunday, the longest possible run of leading days.
    expect(toIso(monthGrid(2027, 8)[0] as never)).toBe("2027-07-26");
  });
});

describe("the small helpers", () => {
  it("names months and spells dates out for a screen reader", () => {
    expect(monthName(9)).toBe("September");
    expect(describeDate({ year: 2027, month: 9, day: 1 })).toBe("1 September 2027");
  });

  it("compares two days without comparing their identities", () => {
    expect(sameDate({ year: 2027, month: 9, day: 1 }, { year: 2027, month: 9, day: 1 })).toBe(
      true,
    );
    expect(sameDate({ year: 2027, month: 9, day: 1 }, { year: 2027, month: 9, day: 2 })).toBe(
      false,
    );
    expect(sameDate(null, { year: 2027, month: 9, day: 1 })).toBe(false);
  });

  it("reads today off the local calendar, not off UTC", () => {
    // Deliberately a moment that is a different date in UTC than it is locally almost
    // everywhere: `todayDate` must agree with the local `Date`, whichever that is.
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2027-09-01T23:30:00Z"));
      const now = new Date();
      expect(todayDate()).toEqual({
        year: now.getFullYear(),
        month: now.getMonth() + 1,
        day: now.getDate(),
      });
    } finally {
      vi.useRealTimers();
    }
  });
});
