/**
 * The timezone and locale option lists.
 *
 * These exist to make two promises testable:
 *
 * - **the options are available offline.** They come from `Intl` and a checked-in array,
 *   so there is nothing to fetch. The test for it asserts the absence of a fetch rather
 *   than taking the absence of a URL in the source on trust.
 * - **what is stored is the identifier.** A label is a formatting decision that changes
 *   with a CLDR update. If one ever leaked into the stored value, every project written
 *   by that build would hold a display string where a timezone belongs.
 *
 * Label *text* is mostly left unasserted on purpose — it comes from the platform's
 * naming data and will change as that data does, which is the point of using it. The
 * exceptions are the two cases the brief names explicitly, and the invariants that must
 * hold whatever the data says: no offsets, and no empty labels.
 */
import { describe, expect, it } from "vitest";

import {
  FALLBACK_TIMEZONES,
  canonicalTimezone,
  currentTimezone,
  isLegacyTimezone,
  timezoneLabel,
  timezoneName,
  timezoneOptions,
} from "./timezones";
import { LOCALE_TAGS, currentLocale, localeLabel, localeName, localeOptions } from "./locales";

describe("timezone options", () => {
  const options = timezoneOptions();

  it("offers a substantial list", () => {
    // Four hundred-ish from the platform; the floor is the curated fallback.
    expect(options.length).toBeGreaterThanOrEqual(FALLBACK_TIMEZONES.length);
  });

  it("stores IANA identifiers, never labels", () => {
    for (const option of options) {
      // An identifier is `Area/Location` or a bare name like `UTC`. A label contains an
      // em dash, which an identifier never does.
      expect(option.value).not.toContain("—");
      expect(option.value).toMatch(/^[A-Za-z0-9+_\-]+(?:\/[A-Za-z0-9+_\-]+)*$/);
    }
  });

  it("resolves every identifier it offers", () => {
    // An option the platform cannot format is an option that would break the reports it
    // is chosen for.
    for (const option of options) {
      expect(() =>
        new Intl.DateTimeFormat("en", { timeZone: option.value }).format(0),
      ).not.toThrow();
    }
  });

  it("gives every option a non-empty label", () => {
    for (const option of options) {
      expect(option.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("never labels a zone with a bare UTC offset", () => {
    // Offsets move when a government changes its mind. A label that encodes one becomes
    // quietly wrong rather than visibly stale.
    for (const option of options) {
      expect(option.label).not.toMatch(/\b(?:GMT|UTC)[+\-]\d/);
    }
  });

  it("has no duplicate identifiers", () => {
    const values = options.map((o) => o.value);
    expect(new Set(values).size).toBe(values.length);
  });

  it("is sorted by label", () => {
    const labels = options.map((o) => o.label);
    const sorted = [...labels].sort(new Intl.Collator("en").compare);
    expect(labels).toEqual(sorted);
  });

  it("offers the canonical name, not the retired alias", () => {
    const values = options.map((o) => o.value);

    expect(values).toContain("Asia/Kolkata");
    expect(values).not.toContain("Asia/Calcutta");
    expect(values).toContain("Europe/Kyiv");
    expect(values).not.toContain("Europe/Kiev");
  });

  it("labels the brief's example exactly as the brief asks", () => {
    const kolkata = options.find((o) => o.value === "Asia/Kolkata");

    expect(kolkata?.label).toBe("India Standard Time — Asia/Kolkata");
  });

  it("includes every curated fallback, canonicalised", () => {
    const values = new Set(options.map((o) => o.value));
    for (const id of FALLBACK_TIMEZONES) {
      expect(values.has(canonicalTimezone(id))).toBe(true);
    }
  });

  it("covers both hemispheres and the awkward partial-hour zones", () => {
    const values = new Set(options.map((o) => o.value));
    for (const id of [
      "Asia/Kolkata", // +05:30
      "Asia/Kathmandu", // +05:45
      "Australia/Eucla", // +08:45
      "Pacific/Auckland",
      "America/Sao_Paulo",
      "Africa/Nairobi",
      "UTC",
    ]) {
      expect(values.has(id)).toBe(true);
    }
  });
});

describe("timezone naming", () => {
  it("names a zone without reference to the season", () => {
    // `long` would say "Greenwich Mean Time" in January and "British Summer Time" in
    // July. A label that changes under the user twice a year is a bug.
    expect(timezoneName("Europe/London")).toBe("United Kingdom Time");
  });

  it("names a zone as it stands now, not as it stood in 1970", () => {
    // Resolved at the epoch this is "Moscow Standard Time", which is historically
    // accurate and politically obsolete.
    expect(timezoneName("Europe/Kyiv")).toBe("Eastern European Time");
  });

  it("falls back from the generic style when it yields an offset", () => {
    // `longGeneric` renders UTC as "GMT+00:00"; `long` has a real name for it.
    expect(timezoneName("UTC")).toBe("Coordinated Universal Time");
  });

  it("falls back to a readable form of the identifier when there is no name", () => {
    expect(timezoneName("Mars/Olympus_Mons")).toBe("Mars · Olympus Mons");
  });

  it("puts the identifier in the label, because that is what is stored", () => {
    expect(timezoneLabel("Asia/Tokyo")).toContain("Asia/Tokyo");
  });
});

describe("timezone canonicalisation", () => {
  it("maps retired names to current ones", () => {
    expect(canonicalTimezone("Asia/Calcutta")).toBe("Asia/Kolkata");
    expect(canonicalTimezone("Europe/Kiev")).toBe("Europe/Kyiv");
    expect(canonicalTimezone("America/Godthab")).toBe("America/Nuuk");
  });

  it("leaves a current name alone", () => {
    expect(canonicalTimezone("Asia/Kolkata")).toBe("Asia/Kolkata");
    expect(canonicalTimezone("Europe/London")).toBe("Europe/London");
  });

  it("leaves an unknown name alone rather than guessing", () => {
    expect(canonicalTimezone("Mars/Olympus_Mons")).toBe("Mars/Olympus_Mons");
  });

  it("recognises a retired name without rewriting it", () => {
    expect(isLegacyTimezone("Asia/Calcutta")).toBe(true);
    expect(isLegacyTimezone("Asia/Kolkata")).toBe(false);
  });

  it("explains a retired name in its label instead of substituting it", () => {
    const label = timezoneLabel("Asia/Calcutta");

    // The stored value is still what the label is about.
    expect(label).toContain("Asia/Calcutta");
    expect(label).toContain("renamed to Asia/Kolkata");
  });

  it("is not fooled by an inherited Object property", () => {
    expect(isLegacyTimezone("constructor")).toBe(false);
    expect(canonicalTimezone("toString")).toBe("toString");
  });
});

describe("the machine's own timezone", () => {
  it("is a zone the picker can offer", () => {
    const current = currentTimezone();
    if (current === "") return; // A runtime that will not say. Nothing to check.

    expect(timezoneOptions().some((o) => o.value === current)).toBe(true);
  });

  it("is canonical, so a new project is not born holding a retired name", () => {
    const current = currentTimezone();
    if (current === "") return;

    expect(isLegacyTimezone(current)).toBe(false);
  });
});

describe("locale options", () => {
  const options = localeOptions();

  it("offers the curated list", () => {
    expect(options).toHaveLength(new Set(LOCALE_TAGS).size);
  });

  it("stores BCP 47 tags, never labels", () => {
    for (const option of options) {
      expect(option.value).not.toContain("—");
      expect(option.value).toMatch(/^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/);
    }
  });

  it("gives every option a non-empty label", () => {
    for (const option of options) {
      expect(option.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("has no duplicate tags", () => {
    const values = options.map((o) => o.value);
    expect(new Set(values).size).toBe(values.length);
  });

  it("is sorted by label", () => {
    const labels = options.map((o) => o.label);
    const sorted = [...labels].sort(new Intl.Collator("en").compare);
    expect(labels).toEqual(sorted);
  });

  it("labels the brief's example exactly as the brief asks", () => {
    const enIN = options.find((o) => o.value === "en-IN");

    expect(enIN?.label).toBe("English (India) — en-IN");
  });

  it("distinguishes the regional variants of one language", () => {
    // The whole reason en-GB and en-US are separate rows is that they format dates
    // differently, so a list that labels them identically is useless.
    const english = options.filter((o) => o.value.startsWith("en-"));

    expect(english.length).toBeGreaterThan(5);
    expect(new Set(english.map((o) => o.label)).size).toBe(english.length);
  });

  it("names the script where one is in the tag", () => {
    // Script and region together in the qualifier, so the two Chinese rows are
    // distinguishable at a glance.
    expect(localeName("zh-Hans-CN")).toBe("Chinese (Simplified, China)");
    expect(localeName("zh-Hant-TW")).toBe("Chinese (Traditional, Taiwan)");
  });

  it("is accepted by the formatters it exists to configure", () => {
    for (const option of options) {
      expect(() => new Intl.DateTimeFormat(option.value).format(0)).not.toThrow();
      expect(() => new Intl.NumberFormat(option.value).format(1)).not.toThrow();
    }
  });

  it("falls back to the tag when there is no name for it", () => {
    expect(localeLabel("qqq-ZZ")).toContain("qqq");
  });

  it("covers the regions international schools are commonly in", () => {
    const values = new Set(options.map((o) => o.value));
    for (const tag of ["en-GB", "en-US", "en-IN", "en-AE", "ar-AE", "fr-FR", "ja-JP"]) {
      expect(values.has(tag)).toBe(true);
    }
  });
});

describe("the machine's own locale", () => {
  it("is a tag the formatters accept", () => {
    const current = currentLocale();
    if (current === "") return;

    expect(() => new Intl.DateTimeFormat(current).format(0)).not.toThrow();
  });

  it("carries no Unicode extension sequence", () => {
    // `en-GB-u-ca-gregory` means `en-GB`. Offering the extension-laden original as an
    // unrecognised custom value would be noise.
    expect(currentLocale()).not.toContain("-u-");
  });
});

describe("offline availability", () => {
  it("builds both lists with no network access whatsoever", () => {
    // The lists are `Intl` plus a checked-in array, so removing every way to reach the
    // network must not change the outcome. This is the local-first rule from CLAUDE.md
    // asserted rather than assumed.
    const globals = globalThis as Record<string, unknown>;
    const names = ["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "navigator"];
    const saved = new Map(names.map((name) => [name, globals[name]]));

    const trap = () => {
      throw new Error("the option lists must not touch the network");
    };

    try {
      for (const name of names) {
        Object.defineProperty(globals, name, {
          value: trap,
          configurable: true,
          writable: true,
        });
      }

      expect(timezoneOptions().length).toBeGreaterThan(0);
      expect(localeOptions().length).toBeGreaterThan(0);
      expect(timezoneLabel("Asia/Kolkata")).toBe("India Standard Time — Asia/Kolkata");
      expect(localeLabel("en-IN")).toBe("English (India) — en-IN");
    } finally {
      for (const [name, value] of saved) {
        Object.defineProperty(globals, name, {
          value,
          configurable: true,
          writable: true,
        });
      }
    }
  });

  it("still offers a usable timezone list when the runtime will not enumerate", () => {
    // A trimmed-down ICU build, or a runtime older than the ES2022 enumeration API.
    const original = Intl.supportedValuesOf;
    try {
      Object.defineProperty(Intl, "supportedValuesOf", {
        value: undefined,
        configurable: true,
        writable: true,
      });

      const options = timezoneOptions();
      expect(options.length).toBe(new Set(FALLBACK_TIMEZONES.map(canonicalTimezone)).size);
      expect(options.some((o) => o.value === "Asia/Kolkata")).toBe(true);
    } finally {
      Object.defineProperty(Intl, "supportedValuesOf", {
        value: original,
        configurable: true,
        writable: true,
      });
    }
  });

  it("survives a runtime whose enumeration throws", () => {
    const original = Intl.supportedValuesOf;
    try {
      Object.defineProperty(Intl, "supportedValuesOf", {
        value: () => {
          throw new RangeError("unsupported");
        },
        configurable: true,
        writable: true,
      });

      expect(timezoneOptions().length).toBeGreaterThan(0);
    } finally {
      Object.defineProperty(Intl, "supportedValuesOf", {
        value: original,
        configurable: true,
        writable: true,
      });
    }
  });
});
