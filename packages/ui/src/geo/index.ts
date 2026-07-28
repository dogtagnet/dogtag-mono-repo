/**
 * On-device geo core.
 *
 * Pure arithmetic over positions the caller already holds: distance, bearing, display formatting,
 * sorting, and geohash. No I/O of any kind, and no dependency on anything that performs I/O.
 *
 * THE BOUNDARY, which is the reason the module exists. The central directory ships a server-side
 * distance filter (`GET /v1/businesses?near=<lat>,<lng>`, public and unauthenticated) that is told
 * where the caller is. Nothing in this repo sends that parameter, and the client library's ability
 * to do so was removed alongside this module - see the deprecation note on `BusinessesQuery` in
 * `stacks/admin/api/src/routes.rs`. The replacement is: fetch the provider set, which is identical
 * whoever asks for it, and do the arithmetic here.
 *
 * So: nothing in `geo/` may acquire I/O, and nothing here may turn a position into a query
 * parameter, a request path, a cache key that reaches the network, or a log line. Adding a
 * `fetchNearby` here would defeat the module and would be the exact shortcut this slice closed.
 *
 * Everything is a total function over its inputs, and every undefined answer is `null` rather than a
 * plausible number: no bearing between identical points, no distance for an unusable coordinate, no
 * formatted string for a distance that was never computed. That is the same rule the verification
 * verdicts follow - a non-answer must not render as a definite one.
 */

export {
  EARTH_RADIUS_KM,
  MAX_DISTANCE_KM,
  haversineKm,
  isValidLatLng,
  sortByDistance,
  withinRadiusKm,
  type LatLng,
  type Ranked,
} from "./distance";

export {
  COMPASS_POINTS_8,
  COMPASS_POINTS_16,
  compassPoint8,
  compassPoint16,
  initialBearingDeg,
  type CompassPoint8,
  type CompassPoint16,
} from "./bearing";

export {
  FEET_PER_KM,
  KM_PER_MILE,
  formatBearing,
  formatDistanceKm,
  unitSystemForRegion,
  type UnitSystem,
} from "./format";

export {
  GEOHASH_BASE32,
  MAX_GEOHASH_PRECISION,
  decodeGeohash,
  encodeGeohash,
  geohashCellContains,
  type GeohashCell,
  type Range,
} from "./geohash";
