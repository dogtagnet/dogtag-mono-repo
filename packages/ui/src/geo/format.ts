/**
 * Rendering a distance without over-claiming what it knows.
 *
 * THE RULE. The number of digits shown IS a claim about precision, and it is a claim this input
 * cannot support. A provider's pin is a published business address; the user's end is a GPS fix,
 * typically good to 5-20 m outdoors and far worse indoors. So "823 m" asserts metre resolution that
 * no part of the computation has, and "0.8231 km" is worse. The bands below round to roughly 10 m
 * near the user and degrade from there, which is the most any caller may honestly print.
 *
 * The floor bands (`< 10 m`, `< 25 ft`) exist for the same reason: below the GPS noise the only
 * true statement is "you are essentially there". "3 m" would be fiction dressed as measurement.
 *
 * `null` in, `null` out - and `null` for anything non-finite or negative. A caller that renders the
 * return value directly can never print a distance that was not established, which is what makes
 * `sortByDistance`'s `distanceKm: null` safe to pass straight through.
 *
 * Pure: no `Intl`, no `navigator`, no locale lookup. `unitSystemForRegion` takes the region as an
 * argument rather than reading it, so this file has no ambient dependencies and the unit choice
 * stays the caller's.
 */

/** Which unit system to render in. */
export type UnitSystem = "metric" | "imperial";

/** Exact, by international definition. */
export const KM_PER_MILE = 1.609344;
/** Exact: 1000 m / 0.3048 m per foot. */
export const FEET_PER_KM = 1000 / 0.3048;

/**
 * Regions that use miles for road distance, as uppercase ISO 3166-1 alpha-2.
 *
 * The UK is here deliberately: it is metric for almost everything but signs road distances in miles
 * and yards, and a Nearby list is a road-distance surface.
 */
const IMPERIAL_REGIONS = new Set(["US", "GB", "LR", "MM"]);

/**
 * Unit system for a region code, defaulting to metric.
 *
 * Accepts either a bare region (`"US"`) or a BCP-47 tag whose region subtag it will read
 * (`"en-US"`, `"en-Latn-US"`). Anything unrecognised is metric, which is the right default both by
 * population and because over-reporting in kilometres is a smaller error than the reverse.
 */
export function unitSystemForRegion(regionOrLocale: string | null | undefined): UnitSystem {
  if (!regionOrLocale) return "metric";
  const parts = regionOrLocale.split(/[-_]/);
  // In a BCP-47 tag the FIRST subtag is the language, never a region, so the scan starts at 1. That
  // skip is the rule, not an optimisation: "us-FR" is French written in some language tagged "us",
  // and reading the leading subtag as a region would call it imperial on the strength of a language
  // code. Nothing collides today (no ISO-639 code equals US/GB/LR/MM), which is exactly what makes
  // scanning from 0 a trap - adding `AS` or `MS` to the set would silently make Assamese and Malay
  // imperial, and no existing case would go red.
  for (const part of parts.slice(1)) {
    if (/^[A-Za-z]{2}$/.test(part) && IMPERIAL_REGIONS.has(part.toUpperCase())) return "imperial";
  }
  // A bare two-letter tag is ambiguous between language and region ("in" is Indonesian, not India).
  // With no language subtag ahead of it there is nothing to disambiguate against, so a single-part
  // input is read as a region - which is what a caller passing `"US"` means.
  if (parts.length === 1 && parts[0] && IMPERIAL_REGIONS.has(parts[0].toUpperCase())) {
    return "imperial";
  }
  return "metric";
}

/** Round `value` to the nearest multiple of `step`. */
const roundTo = (value: number, step: number): number => Math.round(value / step) * step;

/**
 * Format a distance in kilometres for display.
 *
 * Metric bands:
 *   < 10 m      -> `"< 10 m"`
 *   < 1 km      -> nearest 10 m, e.g. `"820 m"`
 *   < 10 km     -> one decimal,  e.g. `"3.4 km"`
 *   >= 10 km    -> whole km,     e.g. `"27 km"`
 *
 * Imperial bands (the 25 ft step is ~7.6 m, matching the metric band's ~10 m):
 *   < 25 ft     -> `"< 25 ft"`
 *   < 0.1 mi    -> nearest 25 ft, e.g. `"450 ft"`
 *   < 10 mi     -> one decimal,   e.g. `"2.1 mi"`
 *   >= 10 mi    -> whole miles,   e.g. `"17 mi"`
 *
 * Returns `null` for `null`, `undefined`, non-finite, or negative input.
 */
export function formatDistanceKm(
  km: number | null | undefined,
  unit: UnitSystem = "metric",
): string | null {
  if (km === null || km === undefined) return null;
  if (!Number.isFinite(km) || km < 0) return null;

  if (unit === "imperial") {
    const miles = km / KM_PER_MILE;
    if (miles < 0.1) {
      const feet = km * FEET_PER_KM;
      if (feet < 25) return "< 25 ft";
      return `${roundTo(feet, 25)} ft`;
    }
    // Round BEFORE the band test, so 9.97 mi renders as "10 mi" rather than "10.0 mi" - the
    // one-decimal form would otherwise show a trailing zero it did not measure.
    if (Math.round(miles * 10) / 10 < 10) {
      return `${(Math.round(miles * 10) / 10).toFixed(1)} mi`;
    }
    return `${Math.round(miles)} mi`;
  }

  if (km < 1) {
    const metres = km * 1000;
    if (metres < 10) return "< 10 m";
    const rounded = roundTo(metres, 10);
    // Rounding can push the value out of its own band: 999 m rounds to 1000, which is a kilometre
    // and should read as one. Falling through rather than printing "1000 m" is the same rule the
    // imperial branch and the km band below apply - decide the band AFTER rounding, not before.
    if (rounded < 1000) return `${rounded} m`;
  }
  if (Math.round(km * 10) / 10 < 10) {
    return `${(Math.round(km * 10) / 10).toFixed(1)} km`;
  }
  return `${Math.round(km)} km`;
}

/**
 * Format a bearing in degrees as a short heading, e.g. `"NE"`.
 *
 * A thin pass-through over `compassPoint8` kept here so a caller formatting a row imports one
 * module; `null` propagates, so an undefined bearing renders nothing rather than "N".
 */
export { compassPoint8 as formatBearing } from "./bearing";
