/**
 * The timezone option list.
 *
 * Period times are local wall clock, so the school's timezone is what they are local to.
 * It therefore has to be a real IANA identifier, and the user has to be able to find
 * theirs among four hundred of them.
 *
 * ## Where the list comes from
 *
 * `Intl.supportedValuesOf("timeZone")` is the runtime's own copy of the IANA database —
 * the same data the platform uses to format a date, already installed, no network, no
 * bundled snapshot to go stale. It is the right source when it exists.
 *
 * It does not always exist: it is an ES2022 addition, and a runtime can be built with a
 * cut-down ICU that returns a near-empty list. So {@link FALLBACK_TIMEZONES} is a curated
 * set covering the regions an international school is plausibly in, and the enumeration
 * is unioned with it rather than trusted alone. The list is never empty and never needs
 * the network.
 *
 * ## Why there are no UTC offsets in the labels
 *
 * `"Asia/Kolkata (UTC+05:30)"` would be a lie waiting to happen. Offsets move — Egypt
 * reintroduced DST in 2023, Mexico abolished most of it in 2022 — and a hardcoded offset
 * silently becomes wrong rather than visibly stale. Labels come from the platform's own
 * naming data instead, so they track the same updates the timezone rules do.
 *
 * `longGeneric` is used in preference to `long` for the same reason: it yields
 * `"United Kingdom Time"` rather than `"Greenwich Mean Time"` or `"British Summer Time"`
 * depending on when you ask. A label that changes with the season would be a label that
 * changes under the user for no reason they can see.
 */

/** The display label and the stored identifier, kept apart on purpose. */
export interface TimezoneOption {
  /** The canonical IANA identifier. This is what gets stored. */
  readonly value: string;
  /** `India Standard Time — Asia/Kolkata`. Display only. */
  readonly label: string;
}

/**
 * A fixed, modern instant used to resolve zone names.
 *
 * It has to be fixed or the labels would depend on when the app happened to be opened.
 * It has to be recent because `longGeneric` is resolved against history: at the Unix
 * epoch `Europe/Kyiv` was in the Soviet Union and names itself "Moscow Standard Time",
 * which is accurate and useless. A 2024 instant gives the names as they stand now, and
 * because the style is generic rather than seasonal, January and July agree.
 */
const NAMING_INSTANT = Date.UTC(2024, 0, 15);

/**
 * IANA renames that the platform still reports under the old spelling.
 *
 * IANA keeps retired names working as links, and ICU enumerates several of them rather
 * than the current name — so a plain pass-through of `supportedValuesOf` would offer
 * `Asia/Calcutta` to a school in India and `Europe/Kiev` to one in Ukraine. Both are
 * correct identifiers and both are the wrong thing to show someone in 2026.
 *
 * This maps old to current for display and for what new projects store. It is not applied
 * to a value already saved in a project — see {@link isLegacyTimezone} for why.
 *
 * A `Map` rather than an object literal so a lookup cannot walk into
 * `Object.prototype`: with a plain object, `canonicalTimezone("toString")` returns a
 * *function*, which would then be stored as a timezone. Unlikely input, silent failure,
 * free fix.
 */
const RENAMED = new Map<string, string>([
  ["Africa/Asmera", "Africa/Asmara"],
  ["America/Buenos_Aires", "America/Argentina/Buenos_Aires"],
  ["America/Catamarca", "America/Argentina/Catamarca"],
  ["America/Cordoba", "America/Argentina/Cordoba"],
  ["America/Godthab", "America/Nuuk"],
  ["America/Indianapolis", "America/Indiana/Indianapolis"],
  ["America/Jujuy", "America/Argentina/Jujuy"],
  ["America/Louisville", "America/Kentucky/Louisville"],
  ["America/Mendoza", "America/Argentina/Mendoza"],
  ["Asia/Calcutta", "Asia/Kolkata"],
  ["Asia/Katmandu", "Asia/Kathmandu"],
  ["Asia/Rangoon", "Asia/Yangon"],
  ["Asia/Saigon", "Asia/Ho_Chi_Minh"],
  ["Atlantic/Faeroe", "Atlantic/Faroe"],
  ["Europe/Kiev", "Europe/Kyiv"],
  ["Pacific/Enderbury", "Pacific/Kanton"],
  ["Pacific/Ponape", "Pacific/Pohnpei"],
  ["Pacific/Truk", "Pacific/Chuuk"],
]);

/**
 * Enough of the world to run a school from, for when the runtime will not enumerate.
 *
 * Chosen for coverage of international-school regions rather than completeness: every
 * populated continent, both hemispheres, the half-hour and three-quarter-hour zones that
 * catch naive implementations out (`Asia/Kolkata`, `Asia/Kathmandu`, `Australia/Eucla`),
 * and `UTC` for a school that would rather not pick a city. Extend it freely — it is a
 * plain array and nothing derives meaning from its order or length.
 */
export const FALLBACK_TIMEZONES: readonly string[] = [
  "UTC",
  // Africa
  "Africa/Abidjan",
  "Africa/Accra",
  "Africa/Addis_Ababa",
  "Africa/Algiers",
  "Africa/Cairo",
  "Africa/Casablanca",
  "Africa/Johannesburg",
  "Africa/Lagos",
  "Africa/Nairobi",
  "Africa/Tunis",
  // Americas
  "America/Anchorage",
  "America/Argentina/Buenos_Aires",
  "America/Bogota",
  "America/Chicago",
  "America/Denver",
  "America/Halifax",
  "America/Lima",
  "America/Los_Angeles",
  "America/Mexico_City",
  "America/New_York",
  "America/Panama",
  "America/Santiago",
  "America/Sao_Paulo",
  "America/St_Johns",
  "America/Toronto",
  "America/Vancouver",
  // Asia
  "Asia/Almaty",
  "Asia/Amman",
  "Asia/Baghdad",
  "Asia/Bangkok",
  "Asia/Beirut",
  "Asia/Colombo",
  "Asia/Dhaka",
  "Asia/Dubai",
  "Asia/Ho_Chi_Minh",
  "Asia/Hong_Kong",
  "Asia/Jakarta",
  "Asia/Jerusalem",
  "Asia/Kabul",
  "Asia/Karachi",
  "Asia/Kathmandu",
  "Asia/Kolkata",
  "Asia/Kuala_Lumpur",
  "Asia/Kuwait",
  "Asia/Manila",
  "Asia/Qatar",
  "Asia/Riyadh",
  "Asia/Seoul",
  "Asia/Shanghai",
  "Asia/Singapore",
  "Asia/Taipei",
  "Asia/Tashkent",
  "Asia/Tbilisi",
  "Asia/Tehran",
  "Asia/Tokyo",
  "Asia/Yangon",
  // Atlantic and Indian Ocean
  "Atlantic/Azores",
  "Atlantic/Reykjavik",
  "Indian/Maldives",
  "Indian/Mauritius",
  // Australasia and the Pacific
  "Australia/Adelaide",
  "Australia/Brisbane",
  "Australia/Darwin",
  "Australia/Eucla",
  "Australia/Melbourne",
  "Australia/Perth",
  "Australia/Sydney",
  "Pacific/Auckland",
  "Pacific/Fiji",
  "Pacific/Guam",
  "Pacific/Honolulu",
  "Pacific/Port_Moresby",
  // Europe
  "Europe/Amsterdam",
  "Europe/Athens",
  "Europe/Berlin",
  "Europe/Brussels",
  "Europe/Bucharest",
  "Europe/Budapest",
  "Europe/Copenhagen",
  "Europe/Dublin",
  "Europe/Helsinki",
  "Europe/Istanbul",
  "Europe/Kyiv",
  "Europe/Lisbon",
  "Europe/London",
  "Europe/Madrid",
  "Europe/Moscow",
  "Europe/Oslo",
  "Europe/Paris",
  "Europe/Prague",
  "Europe/Rome",
  "Europe/Stockholm",
  "Europe/Vienna",
  "Europe/Warsaw",
  "Europe/Zurich",
];

/** The current IANA spelling of an identifier, where we know it has been renamed. */
export function canonicalTimezone(id: string): string {
  const trimmed = id.trim();
  return RENAMED.get(trimmed) ?? trimmed;
}

/**
 * Whether an identifier is a retired IANA name with a current replacement.
 *
 * Used to explain a stored value rather than to change it. A project holding
 * `Asia/Calcutta` means Kolkata and keeps meaning Kolkata — both resolve identically —
 * so rewriting it on open would be a silent, pointless write that bumps `rev` and
 * teaches the user nothing. The picker says what the current name is and leaves the
 * choice to them.
 */
export function isLegacyTimezone(id: string): boolean {
  return RENAMED.has(id.trim());
}

/** Turns `Asia/Kolkata` into `Asia · Kolkata`, for when the platform offers no name. */
function prettify(id: string): string {
  return id
    .split("/")
    .map((part) => part.replace(/_/g, " "))
    .join(" · ");
}

/** The platform's name for a zone, at {@link NAMING_INSTANT}, or null. */
function zoneName(id: string, style: "longGeneric" | "long"): string | null {
  try {
    const parts = new Intl.DateTimeFormat("en", {
      timeZone: id,
      timeZoneName: style,
    }).formatToParts(NAMING_INSTANT);
    const found = parts.find((part) => part.type === "timeZoneName");
    return found ? found.value : null;
  } catch {
    // An identifier this runtime's database does not know. Not fatal: the caller falls
    // back to the identifier itself, which is still the thing being stored.
    return null;
  }
}

/** Whether a name is really just an offset in disguise, e.g. `GMT+05:30`. */
function isOffset(name: string): boolean {
  return /^(?:GMT|UTC)(?:[+\-−].*)?$/.test(name.trim());
}

/**
 * A human-readable name for a zone, never an offset.
 *
 * `longGeneric` first, because it is the season-independent one. `long` second, which
 * rescues the handful of zones the generic style renders as a bare offset — `UTC` being
 * the obvious one, where `long` says "Coordinated Universal Time". The identifier itself
 * is the last resort, and it is a perfectly honest label.
 */
export function timezoneName(id: string): string {
  const generic = zoneName(id, "longGeneric");
  if (generic && !isOffset(generic)) return generic;

  const long = zoneName(id, "long");
  if (long && !isOffset(long)) return long;

  return prettify(id);
}

/**
 * `India Standard Time — Asia/Kolkata`.
 *
 * The identifier is in the label because it is what gets stored and what a user who
 * knows IANA names will search for. The em dash separates them; neither half is the
 * value.
 */
export function timezoneLabel(id: string): string {
  const name = timezoneName(id);
  const legacy = isLegacyTimezone(id) ? ` — renamed to ${canonicalTimezone(id)}` : "";
  return name === id ? `${id}${legacy}` : `${name} — ${id}${legacy}`;
}

/** The runtime's zone list, or an empty array where it cannot enumerate. */
function enumerated(): readonly string[] {
  try {
    if (typeof Intl.supportedValuesOf !== "function") return [];
    return Intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
}

/**
 * Every zone worth offering, canonical, labelled and sorted by label.
 *
 * Sorted by label rather than by identifier so the list reads as names — which is what
 * the user is scanning — using `Intl.Collator` so accented names land where a speaker
 * expects rather than after `Z`.
 */
export function timezoneOptions(): readonly TimezoneOption[] {
  const ids = new Set<string>();
  for (const id of enumerated()) ids.add(canonicalTimezone(id));
  for (const id of FALLBACK_TIMEZONES) ids.add(canonicalTimezone(id));

  const collator = new Intl.Collator("en");
  return [...ids]
    .map((id) => ({ value: id, label: timezoneLabel(id) }))
    .sort((a, b) => collator.compare(a.label, b.label));
}

/**
 * The machine's own zone, canonicalised, as a starting suggestion only.
 *
 * Reading it is `Intl`, not a network call and not telemetry. It is almost always right
 * and nothing is stored until the user saves.
 */
export function currentTimezone(): string {
  try {
    const resolved = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return resolved ? canonicalTimezone(resolved) : "";
  } catch {
    return "";
  }
}
