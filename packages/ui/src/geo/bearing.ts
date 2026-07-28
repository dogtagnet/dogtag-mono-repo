/**
 * Initial bearing (forward azimuth) between two points, and its compass rendering.
 *
 * Pairs with `distance.ts` for the list-first Nearby screen: a row says "1.4 km · NE", which is
 * enough to orient without a map - and a map is the one part of this feature that re-opens the
 * location leak the rest of it closes.
 *
 * THE RULE THIS FILE EXISTS TO ENFORCE: a bearing that is undefined must not render as a direction.
 * `Math.atan2(0, 0)` is `0`, which a caller would print as "N" - a definite heading manufactured
 * from no information. Both degenerate cases therefore return `null`:
 *
 *   * IDENTICAL POINTS. There is no direction from a place to itself.
 *   * ORIGIN AT A POLE. "North" is not a direction at the north pole, so an azimuth measured from
 *     there has no meaning. The formula does not fail - it quietly returns a value derived from the
 *     prime-meridian convention, so at 90°N it reports "east" for a target 90° of longitude away
 *     when every direction from that point is in fact south. A plausible wrong answer is worse than
 *     no answer, so this one is caught rather than passed through.
 *
 * Two cases that look similar and are NOT caught, because in each the formula's answer is true:
 *
 *   * The DESTINATION at a pole. The bearing to the north pole is due north from anywhere, which is
 *     exactly what the formula gives.
 *   * ANTIPODES. The bearing is ambiguous - every great circle is a shortest path - but every
 *     candidate is a heading that genuinely reaches the destination. One of several right answers is
 *     a different thing from a manufactured one. See the note at the guard in `initialBearingDeg`.
 */

import { isValidLatLng, type LatLng } from "./distance";

const toRad = (deg: number): number => (deg * Math.PI) / 180;
const toDeg = (rad: number): number => (rad * 180) / Math.PI;

/** The eight-point compass rose. Index = round(bearing / 45) mod 8. */
export const COMPASS_POINTS_8 = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"] as const;
export type CompassPoint8 = (typeof COMPASS_POINTS_8)[number];

/** The sixteen-point rose, for when eight is too coarse to be useful. */
export const COMPASS_POINTS_16 = [
  "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE",
  "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW",
] as const;
export type CompassPoint16 = (typeof COMPASS_POINTS_16)[number];

/** Is this point exactly on a pole, where azimuth has no meaning? */
function isPole(p: LatLng): boolean {
  return Math.abs(p.lat) === 90;
}

/**
 * Initial bearing from `a` to `b`, in degrees clockwise from true north, in `[0, 360)`.
 *
 * Returns `null` when the answer is undefined: identical points, an origin on a pole, or an unusable
 * coordinate on either side. See the file header for why each of those is `null` rather than a
 * number.
 *
 * "Initial" is load-bearing on a sphere: a great-circle path changes heading as it goes, so this is
 * the direction to set off in, not a constant course. Over the few kilometres a Nearby list deals
 * with the difference is invisible; over `london_to_singapore` it is not.
 */
export function initialBearingDeg(a: LatLng, b: LatLng): number | null {
  if (!isValidLatLng(a) || !isValidLatLng(b)) return null;
  if (a.lat === b.lat && a.lng === b.lng) return null;
  if (isPole(a)) return null;

  const phi1 = toRad(a.lat);
  const phi2 = toRad(b.lat);
  const dLambda = toRad(b.lng - a.lng);

  const y = Math.sin(dLambda) * Math.cos(phi2);
  const x = Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(dLambda);

  // ANTIPODES ARE DELIBERATELY NOT CAUGHT, and the distinction from the two cases above is the
  // point. Between antipodes every great circle is a shortest path, so the bearing is ambiguous -
  // but each candidate is a genuinely correct heading that does reach the destination. Returning one
  // of several right answers is not the same defect as `atan2(0, 0)` returning "N" for a pair of
  // points with no path between them at all, which is a wrong answer rather than a non-unique one.
  //
  // Catching it would also need an arbitrary proximity threshold: at exact antipodes the formula
  // does not degenerate cleanly in floating point (for 0,0 -> 0,180, `sin(PI)` is 1.2e-16 rather
  // than 0), so there is no exact test to make. An arbitrary threshold would turn a correct answer
  // into `null` for an arbitrary band of inputs.
  //
  // What the guard below DOES catch is the identical-points case, mathematically rather than by
  // comparing coordinates: when `a` and `b` coincide, `dLambda` is 0 and `phi1 == phi2`, so `y` is
  // `sin(0) * cos(phi)` and `x` is `cos(phi)sin(phi) - sin(phi)cos(phi)` - both EXACTLY zero, at
  // every latitude. It is therefore a second, independent catch for the same case as the early
  // return above rather than dead code, and deleting either one alone leaves the property intact.
  // (Verified by mutation: removing one changes no behaviour; removing both makes
  // `initialBearingDeg(p, p)` return 0 and the caller render "N".)
  if (y === 0 && x === 0) return null;

  // atan2 gives (-180, 180]; the compass convention is [0, 360). The extra `% 360` handles the
  // -0 case, where `(-0 + 360) % 360` is 0 rather than 360.
  return (toDeg(Math.atan2(y, x)) + 360) % 360;
}

/**
 * Render a bearing as an eight-point compass label, or `null` if there is no bearing.
 *
 * `null` in, `null` out - a caller that renders the return value directly can never print a
 * direction that was not established.
 */
export function compassPoint8(bearingDeg: number | null): CompassPoint8 | null {
  if (bearingDeg === null || !Number.isFinite(bearingDeg)) return null;
  const idx = Math.round((((bearingDeg % 360) + 360) % 360) / 45) % 8;
  return COMPASS_POINTS_8[idx] ?? null;
}

/** Render a bearing as a sixteen-point compass label, or `null` if there is no bearing. */
export function compassPoint16(bearingDeg: number | null): CompassPoint16 | null {
  if (bearingDeg === null || !Number.isFinite(bearingDeg)) return null;
  const idx = Math.round((((bearingDeg % 360) + 360) % 360) / 22.5) % 16;
  return COMPASS_POINTS_16[idx] ?? null;
}
