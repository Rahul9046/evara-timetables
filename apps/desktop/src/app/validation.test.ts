/**
 * Form validation, which is pure and therefore cheap to pin down exactly.
 *
 * These tests also guard the *line* the module draws. Several of them assert that
 * validation does **not** reject something — a cycle longer than a week, a day with no
 * room type, a cycle whose `dayCount` exceeds the days that exist — because each of those
 * is a legal state that a plausible-looking client-side rule would wrongly refuse, and
 * the refusal would be invisible from the Rust side where the real rules live.
 */
import { describe, expect, it } from "vitest";

import {
  isoDate,
  isValid,
  optional,
  required,
  validateAcademicYear,
  validateCalendarDay,
  validateCampus,
  validateCycle,
  validateCycleDay,
  validatePeriod,
  validateResource,
  validateRoom,
  validateSchool,
  validateTerm,
  wallClock,
  wholeNumber,
  type Draft,
} from "./validation";

describe("field primitives", () => {
  it("rejects blank and whitespace-only values", () => {
    expect(required("", "Name")).toMatch(/required/);
    expect(required("   ", "Name")).toMatch(/required/);
    expect(required(null, "Name")).toMatch(/required/);
    expect(required(undefined, "Name")).toMatch(/required/);
    expect(required("Northgate", "Name")).toBeNull();
  });

  it("names the field in every message, so a banner is readable on its own", () => {
    expect(required("", "School name")).toBe("School name is required.");
    expect(wholeNumber("x", "Capacity")).toBe("Capacity must be a whole number.");
    expect(wallClock("9:30", "Start time")).toMatch(/^Start time/);
  });

  describe("wholeNumber", () => {
    it("accepts integers and rejects anything else", () => {
      expect(wholeNumber("0", "N")).toBeNull();
      expect(wholeNumber("42", "N")).toBeNull();
      expect(wholeNumber("-3", "N")).toBeNull();
      expect(wholeNumber("3.5", "N")).toMatch(/whole number/);
      expect(wholeNumber("1e3", "N")).toMatch(/whole number/);
      expect(wholeNumber("", "N")).toMatch(/required/);
    });

    it("enforces bounds", () => {
      expect(wholeNumber("0", "Capacity", { min: 0 })).toBeNull();
      expect(wholeNumber("-1", "Capacity", { min: 0 })).toMatch(/cannot be less than 0/);
      expect(wholeNumber("8", "Hint", { min: 1, max: 7 })).toMatch(/cannot be more than 7/);
    });
  });

  describe("wallClock", () => {
    it("requires zero-padded HH:MM, as the column's CHECK does", () => {
      expect(wallClock("09:30", "T")).toBeNull();
      expect(wallClock("00:00", "T")).toBeNull();
      expect(wallClock("23:59", "T")).toBeNull();
      expect(wallClock("9:30", "T")).toMatch(/24-hour time/);
      expect(wallClock("09:30:00", "T")).toMatch(/24-hour time/);
    });

    it("rejects times that match the shape but are not real", () => {
      expect(wallClock("25:00", "T")).toMatch(/not a real time/);
      expect(wallClock("12:60", "T")).toMatch(/not a real time/);
    });
  });

  describe("isoDate", () => {
    it("requires canonical YYYY-MM-DD", () => {
      expect(isoDate("2027-09-01", "D")).toBeNull();
      expect(isoDate("2027-9-1", "D")).toMatch(/date like/);
      expect(isoDate("01/09/2027", "D")).toMatch(/date like/);
    });

    it("rejects a well-shaped date that does not exist", () => {
      expect(isoDate("2027-02-31", "D")).toMatch(/not a real date/);
      expect(isoDate("2027-13-01", "D")).toMatch(/not a real date/);
    });

    it("accepts a leap day in a leap year and rejects it otherwise", () => {
      expect(isoDate("2028-02-29", "D")).toBeNull();
      expect(isoDate("2027-02-29", "D")).toMatch(/not a real date/);
    });
  });

  it("skips an optional field that was left blank", () => {
    expect(optional("", wholeNumber, "Hint")).toBeNull();
    expect(optional("   ", wholeNumber, "Hint")).toBeNull();
    expect(optional("abc", wholeNumber, "Hint")).toMatch(/whole number/);
  });
});

describe("school", () => {
  it("requires all three fields", () => {
    expect(Object.keys(validateSchool({}))).toEqual(["name", "timezone", "locale"]);
  });

  it("accepts a complete record", () => {
    expect(
      isValid(
        validateSchool({
          name: "Northgate Secondary",
          timezone: "Europe/London",
          locale: "en-GB",
        }),
      ),
    ).toBe(true);
  });
});

describe("campus", () => {
  it("requires a name and a code", () => {
    expect(validateCampus({ name: "Main" })).toHaveProperty("code");
    expect(validateCampus({ code: "MAIN" })).toHaveProperty("name");
  });

  it("does not require an address", () => {
    expect(isValid(validateCampus({ name: "Main", code: "MAIN" }))).toBe(true);
  });
});

describe("room", () => {
  const base: Draft = { name: "Lab 1", code: "L1", capacity: "24" };

  it("accepts a room with no building and no room type", () => {
    // Both references are nullable in the model, and clearing them is a normal edit.
    expect(isValid(validateRoom({ ...base, buildingId: "", roomTypeId: "" }))).toBe(true);
  });

  it("accepts a capacity of zero", () => {
    expect(isValid(validateRoom({ ...base, capacity: "0" }))).toBe(true);
  });

  it("rejects a negative capacity and a non-numeric one", () => {
    expect(validateRoom({ ...base, capacity: "-1" })).toHaveProperty("capacity");
    expect(validateRoom({ ...base, capacity: "lots" })).toHaveProperty("capacity");
  });
});

describe("resource", () => {
  it("accepts a quantity of zero and rejects a fraction", () => {
    const base: Draft = { name: "Minibus", code: "BUS" };
    expect(isValid(validateResource({ ...base, quantity: "0" }))).toBe(true);
    expect(validateResource({ ...base, quantity: "1.5" })).toHaveProperty("quantity");
  });
});

describe("academic year and term", () => {
  it("rejects an end before the start", () => {
    const problems = validateAcademicYear({
      name: "2027/28",
      startsOn: "2028-07-20",
      endsOn: "2027-09-01",
    });
    expect(problems["endsOn"]).toMatch(/cannot be before the start/);
  });

  it("accepts a single-day span", () => {
    expect(
      isValid(
        validateAcademicYear({
          name: "Summer school",
          startsOn: "2028-07-03",
          endsOn: "2028-07-03",
        }),
      ),
    ).toBe(true);
  });

  it("does not pile an ordering message on top of a malformed date", () => {
    const problems = validateAcademicYear({
      name: "2027/28",
      startsOn: "nonsense",
      endsOn: "2027-09-01",
    });
    expect(problems["startsOn"]).toBeDefined();
    expect(problems["endsOn"]).toBeUndefined();
  });

  it("requires a term position of at least 1", () => {
    const base: Draft = { name: "Autumn", startsOn: "2027-09-01", endsOn: "2027-12-17" };
    expect(validateTerm({ ...base, ordinal: "0" })).toHaveProperty("ordinal");
    expect(isValid(validateTerm({ ...base, ordinal: "1" }))).toBe(true);
  });
});

describe("cycle", () => {
  it("accepts a one-day cycle", () => {
    expect(
      isValid(validateCycle({ name: "Every day", dayCount: "1", weekCount: "1" })),
    ).toBe(true);
  });

  it("accepts a cycle longer than a week", () => {
    // Nothing in the model treats five or seven days as special, so nothing here may.
    for (const dayCount of ["6", "8", "10", "15"]) {
      expect(
        isValid(validateCycle({ name: "Rotation", dayCount, weekCount: "2" })),
        `a ${dayCount}-day cycle must be accepted`,
      ).toBe(true);
    }
  });

  it("rejects a cycle of zero days", () => {
    expect(validateCycle({ name: "Nothing", dayCount: "0", weekCount: "1" })).toHaveProperty(
      "dayCount",
    );
  });
});

describe("cycle day", () => {
  it("accepts any label — nothing here knows what a weekday is", () => {
    for (const label of ["Monday", "Day A", "Day 1", "Week 2 Thursday", "狗"]) {
      expect(isValid(validateCycleDay({ ordinal: "1", label })), label).toBe(true);
    }
  });

  it("does not check the position against the cycle's declared length", () => {
    // That needs the cycle, so it is the repository's rule. A client-side version would
    // be a second implementation of it — and would have to be kept in step by hand.
    expect(isValid(validateCycleDay({ ordinal: "99", label: "Day 99" }))).toBe(true);
  });

  it("treats the weekday hint as optional but bounded when given", () => {
    expect(isValid(validateCycleDay({ ordinal: "1", label: "Day A", weekdayHint: "" }))).toBe(
      true,
    );
    expect(
      validateCycleDay({ ordinal: "1", label: "Day A", weekdayHint: "8" }),
    ).toHaveProperty("weekdayHint");
  });
});

describe("period", () => {
  const base: Draft = { ordinal: "1", label: "P1", kind: "TEACHING" };

  it("accepts a well-formed period", () => {
    expect(isValid(validatePeriod({ ...base, startsAt: "09:00", endsAt: "09:50" }))).toBe(
      true,
    );
  });

  it("rejects an end at or before the start", () => {
    expect(
      validatePeriod({ ...base, startsAt: "09:00", endsAt: "09:00" })["endsAt"],
    ).toMatch(/must be after the start/);
    expect(
      validatePeriod({ ...base, startsAt: "09:50", endsAt: "09:00" })["endsAt"],
    ).toMatch(/must be after the start/);
  });

  it("does not report ordering on top of a malformed time", () => {
    const problems = validatePeriod({ ...base, startsAt: "9:00", endsAt: "09:50" });
    expect(problems["startsAt"]).toBeDefined();
    expect(problems["endsAt"]).toBeUndefined();
  });

  it("does not require periods to be in clock order relative to each other", () => {
    // Ordering across rows is not this module's business: it never sees the other rows.
    expect(isValid(validatePeriod({ ...base, ordinal: "9", startsAt: "08:00", endsAt: "08:50" }))).toBe(
      true,
    );
  });
});

describe("calendar day", () => {
  it("requires only a date", () => {
    expect(isValid(validateCalendarDay({ date: "2027-09-06" }))).toBe(true);
  });

  it("accepts a date with no cycle day — that is how a non-teaching date is recorded", () => {
    expect(isValid(validateCalendarDay({ date: "2027-12-25", cycleDayId: "" }))).toBe(true);
  });

  it("rejects a malformed date", () => {
    expect(validateCalendarDay({ date: "25/12/2027" })).toHaveProperty("date");
  });
});
