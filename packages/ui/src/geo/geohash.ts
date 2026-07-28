/**
 * Geohash encode/decode.
 *
 * A geohash is a base-32 string naming a rectangular CELL of the globe, where each added character
 * refines the cell. That property is why it is here: it is the standard way to talk about a position
 * COARSELY, which is the only way this product should ever talk about a user's position at all.
 *
 * WHY `decodeGeohash` RETURNS A CELL AND NOT A POINT. The obvious signature is
 * `decode(hash) -> {lat, lng}`, and it is a trap. A geohash-4 cell is about 39 x 20 km; returning
 * only its centre hands the caller a coordinate that looks exactly like a GPS fix and is wrong by up
 * to 20 km, with nothing at the call site to say so. So the return type carries the BOUNDS and the
 * cell's real dimensions in kilometres, and the centre sits alongside them clearly labelled as the
 * centre of a box. A caller that wants to treat a cell as a position has to say so.
 *
 * That matters concretely for the two later slices that use this. Coarsening a map request to a
 * snapped grid, and sharding a large directory by prefix, both trade precision for privacy - and
 * both are only honest if the precision given up is legible where the trade is made. The scouting
 * report's figures for the two relevant precisions:
 *
 *     precision 4  ~39 x 19.5 km    coarse enough to name a metro area
 *     precision 5  ~4.9 x 4.9 km    a district
 *
 * Pure: encode/decode arithmetic only. Nothing here chooses a precision or performs a request.
 * Choosing how coarse to be is a privacy decision and belongs to the caller making the trade, not
 * to a default in a utility module.
 */

import { haversineKm, type LatLng } from "./distance";

/**
 * The geohash alphabet: base 32, digits then lowercase letters minus `a`, `i`, `l`, `o` (dropped
 * because they misread as `4`/`1`/`1`/`0`). Order is significant - it IS the bit encoding.
 */
export const GEOHASH_BASE32 = "0123456789bcdefghjkmnpqrstuvwxyz";

/** Longest hash this module will produce. Beyond ~12 the cell is finer than a double can express. */
export const MAX_GEOHASH_PRECISION = 12;

/** Inverse of `GEOHASH_BASE32`, built once. */
const BASE32_INDEX: ReadonlyMap<string, number> = new Map(
  [...GEOHASH_BASE32].map((c, i) => [c, i]),
);

/** A half-open interval on one axis, in degrees. */
export interface Range {
  min: number;
  max: number;
}

/** The rectangle a geohash names, plus everything a caller needs to not mistake it for a point. */
export interface GeohashCell {
  /** The hash that produced this cell, normalised to lowercase. */
  hash: string;
  /** Number of characters - the precision. */
  precision: number;
  lat: Range;
  lng: Range;
  /**
   * The centre of the cell. NOT a position: it is up to `heightKm / 2` and `widthKm / 2` away from
   * whatever real point produced this hash.
   */
  center: LatLng;
  /** North-south extent of the cell in kilometres. */
  heightKm: number;
  /**
   * East-west extent in kilometres, measured along the cell's centre latitude. This SHRINKS toward
   * the poles - a precision-4 cell is ~39 km wide at the equator and ~10 km wide at 75°N - so it is
   * computed rather than taken from a table.
   */
  widthKm: number;
}

/**
 * Encode a position as a geohash of `precision` characters.
 *
 * Returns `null` for an unusable coordinate or an out-of-range precision, rather than a hash of
 * something that was never a place.
 *
 * `precision` is REQUIRED. There is no default: how coarse the hash is decides how much position is
 * disclosed when it is used as a shard or grid key, and a module-level default would make that
 * decision silently on the caller's behalf.
 */
export function encodeGeohash(lat: number, lng: number, precision: number): string | null {
  if (!Number.isFinite(lat) || !Number.isFinite(lng)) return null;
  if (lat < -90 || lat > 90 || lng < -180 || lng > 180) return null;
  if (!Number.isInteger(precision) || precision < 1 || precision > MAX_GEOHASH_PRECISION) {
    return null;
  }

  let latMin = -90;
  let latMax = 90;
  let lngMin = -180;
  let lngMax = 180;

  let hash = "";
  let bits = 0;
  let bitCount = 0;
  // Bits alternate axes, longitude FIRST - that interleave is the format.
  let isLngTurn = true;

  while (hash.length < precision) {
    if (isLngTurn) {
      const mid = (lngMin + lngMax) / 2;
      if (lng >= mid) {
        bits = (bits << 1) | 1;
        lngMin = mid;
      } else {
        bits = bits << 1;
        lngMax = mid;
      }
    } else {
      const mid = (latMin + latMax) / 2;
      if (lat >= mid) {
        bits = (bits << 1) | 1;
        latMin = mid;
      } else {
        bits = bits << 1;
        latMax = mid;
      }
    }
    isLngTurn = !isLngTurn;
    bitCount += 1;

    if (bitCount === 5) {
      hash += GEOHASH_BASE32[bits] ?? "";
      bits = 0;
      bitCount = 0;
    }
  }
  return hash;
}

/**
 * Decode a geohash into the cell it names.
 *
 * Returns `null` for an empty string, a character outside the alphabet, or a hash longer than
 * `MAX_GEOHASH_PRECISION` - an undecodable hash yields no cell rather than a plausible wrong box.
 *
 * Case-insensitive on input (the alphabet is lowercase; uppercase hashes appear in the wild).
 */
export function decodeGeohash(hash: string | null | undefined): GeohashCell | null {
  if (!hash) return null;
  const normalised = hash.toLowerCase();
  if (normalised.length > MAX_GEOHASH_PRECISION) return null;

  let latMin = -90;
  let latMax = 90;
  let lngMin = -180;
  let lngMax = 180;
  let isLngTurn = true;

  for (const ch of normalised) {
    const idx = BASE32_INDEX.get(ch);
    if (idx === undefined) return null;
    for (let bit = 4; bit >= 0; bit -= 1) {
      const isSet = ((idx >> bit) & 1) === 1;
      if (isLngTurn) {
        const mid = (lngMin + lngMax) / 2;
        if (isSet) lngMin = mid;
        else lngMax = mid;
      } else {
        const mid = (latMin + latMax) / 2;
        if (isSet) latMin = mid;
        else latMax = mid;
      }
      isLngTurn = !isLngTurn;
    }
  }

  const center: LatLng = { lat: (latMin + latMax) / 2, lng: (lngMin + lngMax) / 2 };
  return {
    hash: normalised,
    precision: normalised.length,
    lat: { min: latMin, max: latMax },
    lng: { min: lngMin, max: lngMax },
    center,
    heightKm: haversineKm({ lat: latMin, lng: center.lng }, { lat: latMax, lng: center.lng }),
    widthKm: haversineKm({ lat: center.lat, lng: lngMin }, { lat: center.lat, lng: lngMax }),
  };
}

/**
 * Does `cell` contain `point`?
 *
 * Half-open: inclusive on the min edges, exclusive on the max, so adjacent cells tile without a
 * point ever falling in two of them.
 *
 * WITH ONE EXCEPTION, at the top edge of the coordinate domain. Latitude stops at +90 and longitude
 * at +180, so a cell whose max IS that boundary has no neighbour above it to own a point sitting
 * exactly on the edge. Under a strictly half-open rule such a point is in NO cell at all - including
 * the cell `encodeGeohash` just produced for it, so `contains(decode(encode(p)), p)` was `false` for
 * every point at lat 90 or lng 180. The north pole, and the whole antimeridian at +180, are exactly
 * the cases the module is meant to treat as ordinary, so the top edge is closed.
 *
 * The bottom edges need no such case: they are already inclusive, which is why -90 and -180 always
 * worked and hid this.
 */
export function geohashCellContains(cell: GeohashCell, point: LatLng): boolean {
  const inLat =
    point.lat >= cell.lat.min &&
    (point.lat < cell.lat.max || (cell.lat.max === 90 && point.lat === 90));
  const inLng =
    point.lng >= cell.lng.min &&
    (point.lng < cell.lng.max || (cell.lng.max === 180 && point.lng === 180));
  return inLat && inLng;
}
