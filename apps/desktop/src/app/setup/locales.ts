/**
 * The locale option list.
 *
 * The school's locale formats dates and numbers in reports. It is a BCP 47 tag, and
 * unlike the timezone list there is no sensible way to enumerate "all" of them: the tag
 * space is combinatorial (any language with any region, plus scripts and variants), and
 * a list of every valid combination would be tens of thousands of rows, almost all of
 * them meaningless. `Intl.supportedValuesOf` does not offer locales for exactly this
 * reason.
 *
 * So this list is **curated, and meant to be extended.** Adding a row is a one-line
 * change and needs no other edit: the label is derived, not written out.
 *
 * ## Why the locale is not inferred from the timezone
 *
 * It would be easy and it would be wrong. A British school in Dubai runs on
 * `Asia/Dubai` and reports in `en-GB`; an international school in Shanghai may report in
 * `en-US`; a school in `Europe/Zurich` could be German, French or Italian. Timezone is a
 * fact about geography and locale is a decision about audience, and conflating them
 * produces a confidently wrong default that a user has to notice before they can fix it.
 * The two fields are independent here, and each one is chosen.
 */

/** The display label and the stored tag, kept apart on purpose. */
export interface LocaleOption {
  /** The BCP 47 tag. This is what gets stored. */
  readonly value: string;
  /** `English (India) — en-IN`. Display only. */
  readonly label: string;
}

/**
 * Locales an international school plausibly reports in.
 *
 * English first and in depth, since that is what most international schools use and the
 * regional variants differ in exactly the ways a timetable cares about — `en-GB` writes
 * 01/09/2027 where `en-US` writes 9/1/2027. Then the languages of instruction and of the
 * host countries those schools are commonly in.
 *
 * Extend this list rather than working around it. No label is stored anywhere, so adding
 * a tag cannot invalidate existing data.
 */
export const LOCALE_TAGS: readonly string[] = [
  // English, by region — the common case for international schools.
  "en-GB",
  "en-US",
  "en-AU",
  "en-CA",
  "en-IE",
  "en-IN",
  "en-NZ",
  "en-SG",
  "en-ZA",
  "en-AE",
  "en-HK",
  "en-MY",
  "en-NG",
  "en-PH",
  "en-KE",
  // Arabic
  "ar-AE",
  "ar-EG",
  "ar-SA",
  "ar-QA",
  "ar-KW",
  "ar-MA",
  // Chinese
  "zh-Hans-CN",
  "zh-Hant-TW",
  "zh-Hant-HK",
  // French
  "fr-FR",
  "fr-BE",
  "fr-CA",
  "fr-CH",
  "fr-MA",
  // German
  "de-DE",
  "de-AT",
  "de-CH",
  // Spanish
  "es-ES",
  "es-MX",
  "es-AR",
  "es-CL",
  "es-CO",
  "es-PE",
  // Portuguese
  "pt-BR",
  "pt-PT",
  // Other European
  "cs-CZ",
  "da-DK",
  "el-GR",
  "fi-FI",
  "hu-HU",
  "it-IT",
  "nb-NO",
  "nl-NL",
  "nl-BE",
  "pl-PL",
  "ro-RO",
  "ru-RU",
  "sv-SE",
  "tr-TR",
  "uk-UA",
  // Asian and other
  "bn-BD",
  "hi-IN",
  "id-ID",
  "ja-JP",
  "ko-KR",
  "ms-MY",
  "ta-IN",
  "te-IN",
  "th-TH",
  "ur-PK",
  "vi-VN",
  "he-IL",
  "sw-KE",
  "af-ZA",
];

/** Display names in English, built once; constructing them per row is measurably slow. */
const NAMES: {
  language: Intl.DisplayNames | null;
  region: Intl.DisplayNames | null;
  script: Intl.DisplayNames | null;
} = (() => {
  try {
    if (typeof Intl.DisplayNames !== "function") {
      return { language: null, region: null, script: null };
    }
    return {
      language: new Intl.DisplayNames("en", { type: "language" }),
      region: new Intl.DisplayNames("en", { type: "region" }),
      script: new Intl.DisplayNames("en", { type: "script" }),
    };
  } catch {
    // A runtime without the display-names data. Labels fall back to the tag itself,
    // which is still selectable and still correct — just terser.
    return { language: null, region: null, script: null };
  }
})();

/** Looks a name up, tolerating a runtime that does not know this particular code. */
function nameOf(names: Intl.DisplayNames | null, code: string): string | null {
  if (!names) return null;
  try {
    const found = names.of(code);
    // `of` echoes the code back when it has no name for it, which is not a name.
    return found && found !== code ? found : null;
  } catch {
    return null;
  }
}

/** Splits a tag without needing `Intl.Locale`, which is not in every runtime. */
function parts(tag: string): { language: string; script?: string; region?: string } {
  const segments = tag.split("-");
  const language = segments[0] ?? tag;
  let script: string | undefined;
  let region: string | undefined;
  for (const segment of segments.slice(1)) {
    if (/^[A-Za-z]{4}$/.test(segment)) script = segment;
    else if (/^(?:[A-Za-z]{2}|\d{3})$/.test(segment)) region = segment;
  }
  return script !== undefined
    ? region !== undefined
      ? { language, script, region }
      : { language, script }
    : region !== undefined
      ? { language, region }
      : { language };
}

/**
 * `English (India)`, or `Chinese (Simplified, Taiwan)`.
 *
 * Built from the pieces rather than asking for the whole tag at once, because
 * `DisplayNames.of("en-IN")` returns "English (India)" on some runtimes and bare
 * "English" on others — and a list where half the English rows are indistinguishable is
 * worse than no list.
 */
export function localeName(tag: string): string {
  const { language, script, region } = parts(tag);
  const base = nameOf(NAMES.language, language) ?? language;

  const qualifiers = [
    script ? nameOf(NAMES.script, script) : null,
    region ? (nameOf(NAMES.region, region) ?? region) : null,
  ].filter((part): part is string => part !== null);

  return qualifiers.length > 0 ? `${base} (${qualifiers.join(", ")})` : base;
}

/**
 * `English (India) — en-IN`.
 *
 * The tag is in the label because it is what gets stored, and because someone who knows
 * they want `en-GB` should be able to type it.
 */
export function localeLabel(tag: string): string {
  const name = localeName(tag);
  return name === tag ? tag : `${name} — ${tag}`;
}

/** Every curated locale, labelled and sorted by label. */
export function localeOptions(): readonly LocaleOption[] {
  const collator = new Intl.Collator("en");
  return [...new Set(LOCALE_TAGS)]
    .map((tag) => ({ value: tag, label: localeLabel(tag) }))
    .sort((a, b) => collator.compare(a.label, b.label));
}

/**
 * The machine's own locale, as a starting suggestion only.
 *
 * Narrowed to a curated tag when the runtime reports something close — a machine set to
 * `en-GB-u-ca-gregory` means `en-GB`, and offering the extension-laden original as a
 * "custom value" would be noise. Falls back to the raw tag, which is still valid BCP 47
 * and still savable.
 */
export function currentLocale(): string {
  try {
    const resolved = Intl.DateTimeFormat().resolvedOptions().locale;
    if (!resolved) return "";
    if (LOCALE_TAGS.includes(resolved)) return resolved;

    // Drop Unicode extensions (`-u-…`) and try the plain language-region tag.
    const plain = resolved.split("-u-")[0] ?? resolved;
    if (LOCALE_TAGS.includes(plain)) return plain;

    const { language, region } = parts(plain);
    const candidate = region ? `${language}-${region}` : language;
    return LOCALE_TAGS.includes(candidate) ? candidate : plain;
  } catch {
    return "";
  }
}
