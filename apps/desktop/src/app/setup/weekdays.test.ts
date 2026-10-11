/**
 * The weekday vocabulary.
 *
 * `evara_core::calendar` owns the rule — what a legal working week is — and its own tests
 * cover it. This file covers the conversion the interface does on either side of that: the
 * form's `string[]` to the ISO numbers the backend wants, and back, plus the prose a
 * summary reads.
 */
import { describe, expect, it } from "vitest";

import {
  MONDAY_TO_FRIDAY,
  WEEKDAYS,
  describeWeek,
  toWeekdayNumbers,
  toWeekdayValues,
  weekdayName,
} from "./weekdays";

describe("the week itself", () => {
  it("is seven days, Monday first, numbered the way the backend numbers them", () => {
    expect(WEEKDAYS).toHaveLength(7);
    expect(WEEKDAYS.map((day) => day.value)).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(WEEKDAYS[0]?.label).toBe("Monday");
    expect(WEEKDAYS[6]?.label).toBe("Sunday");
  });

  it("gives every day a full name and a two-letter form", () => {
    for (const day of WEEKDAYS) {
      expect(day.label.length).toBeGreaterThan(2);
      expect(day.short).toHaveLength(2);
    }
  });

  it("agrees with the column's own default about Monday to Friday", () => {
    // The migration's `DEFAULT 31` is bits 0-4. A mismatch here would mean a new school
    // and an upgraded school started from different weeks.
    expect(MONDAY_TO_FRIDAY).toEqual([1, 2, 3, 4, 5]);
  });

  it("names a weekday, and falls back to the number for one that does not exist", () => {
    expect(weekdayName(1)).toBe("Monday");
    expect(weekdayName(7)).toBe("Sunday");
    expect(weekdayName(0)).toBe("0");
    expect(weekdayName(8)).toBe("8");
  });
});

describe("toWeekdayNumbers", () => {
  it("converts a form's strings to ascending numbers", () => {
    expect(toWeekdayNumbers(["1", "2", "3", "4", "5"])).toEqual([1, 2, 3, 4, 5]);
  });

  it("sorts, whatever order the boxes were clicked in", () => {
    expect(toWeekdayNumbers(["5", "1", "3"])).toEqual([1, 3, 5]);
    expect(toWeekdayNumbers(["7", "1"])).toEqual([1, 7]);
  });

  it("deduplicates", () => {
    expect(toWeekdayNumbers(["1", "1", "2"])).toEqual([1, 2]);
  });

  it("drops anything that is not a weekday rather than passing it on", () => {
    expect(toWeekdayNumbers(["0", "8", "", "monday", "1"])).toEqual([1]);
  });

  it("returns an empty list for an empty selection, so the validator can object", () => {
    // Emphatically not every day: an empty week is a mistake to report, never a default
    // to invent.
    expect(toWeekdayNumbers([])).toEqual([]);
  });
});

describe("toWeekdayValues", () => {
  it("round-trips a week back to what a draft holds", () => {
    expect(toWeekdayValues([1, 2, 3, 4, 5])).toEqual(["1", "2", "3", "4", "5"]);
    expect(toWeekdayNumbers(toWeekdayValues([1, 4, 7]))).toEqual([1, 4, 7]);
  });

  it("sorts and discards what is not a weekday", () => {
    expect(toWeekdayValues([7, 1, 9, 0])).toEqual(["1", "7"]);
  });
});

describe("describeWeek", () => {
  it("contracts a run of three or more consecutive days", () => {
    expect(describeWeek([1, 2, 3, 4, 5])).toBe("Monday to Friday");
    expect(describeWeek([1, 2, 3])).toBe("Monday to Wednesday");
    expect(describeWeek([1, 2, 3, 4, 5, 6])).toBe("Monday to Saturday");
  });

  it("lists days that are not consecutive", () => {
    expect(describeWeek([1, 3, 5])).toBe("Monday, Wednesday and Friday");
    expect(describeWeek([1, 2])).toBe("Monday and Tuesday");
    expect(describeWeek([3])).toBe("Wednesday");
  });

  it("spells out a week that wraps, rather than inventing a backwards range", () => {
    // Sunday to Thursday is 1,2,3,4,7 in ISO order. "Sunday to Thursday" cannot be read
    // off that ascending run, and a range that read "Monday to Sunday" would be a lie.
    expect(describeWeek([7, 1, 2, 3, 4])).toBe(
      "Monday, Tuesday, Wednesday, Thursday and Sunday",
    );
  });

  it("names the two extremes plainly", () => {
    expect(describeWeek([1, 2, 3, 4, 5, 6, 7])).toBe("Every day");
    expect(describeWeek([])).toBe("No working days");
  });

  it("ignores duplicates and non-weekdays", () => {
    expect(describeWeek([1, 1, 2, 2, 3, 9])).toBe("Monday to Wednesday");
  });
});
