/**
 * Reading a provider: where its location may and may not be used.
 *
 * A provider may publish contact channels and no location at all. That makes three rules, and this
 * module is where they are stated once so a listing cannot restate one of them wrongly:
 *
 * 1. It is LISTED. Absence of a location is not absence of a provider.
 * 2. It is CONTACTABLE. Its channels are the whole of how it is reached.
 * 3. It is never given a position it does not have - so it carries no distance and no Directions
 *    destination.
 *
 * Rule 3 is the one that decays quietly. `0, 0` is a legal coordinate off the coast of Ghana, so a
 * fallback coordinate substituted anywhere along this path produces a confident pin in the Gulf of
 * Guinea rather than an error - which is exactly the defect that made location optional.
 *
 * These are pure functions over data the caller already holds. Nothing here may acquire I/O or turn
 * a position into a query - the same boundary `packages/ui/src/geo/` sets, for the same reason.
 */

import { isValidLatLng, type LatLng } from "../geo";
import {
  PROVIDER_CONTACT_CHANNELS,
  type DirectoryProvider,
  type ProviderContactChannel,
} from "./types";

/**
 * This provider's position, or `null` when it has none.
 *
 * THE accessor for a provider's location: pass it to `sortByDistance`'s `positionOf` and read it
 * before offering Directions. It re-checks `isValidLatLng` rather than trusting the field, so a
 * provider reaching this code by some path other than the validating source seam - a cached
 * snapshot from an older build, a hand-built fixture - still cannot be ranked as a distance it
 * does not have.
 */
export function providerPosition(provider: DirectoryProvider): LatLng | null {
  const geo = provider.geo;
  return isValidLatLng(geo) ? { lat: geo.lat, lng: geo.lng } : null;
}

/**
 * Can this provider be handed to the OS maps app as a destination?
 *
 * The Directions gate. A location-less provider has no destination, so a listing must render no
 * Directions affordance for it rather than a disabled one pointing at a placeholder.
 */
export function hasDirectionsDestination(provider: DirectoryProvider): boolean {
  return providerPosition(provider) !== null;
}

/** One published channel: its kind and its value. */
export interface ProviderContactEntry {
  channel: ProviderContactChannel;
  value: string;
}

/**
 * The channels this provider actually published, in listing order.
 *
 * Empty for a provider that published none - which, combined with no location, is a provider that
 * cannot be reached at all. `isUnreachableProvider` is what a listing shows that state with; it must
 * not be silently rendered as an ordinary row.
 */
export function providerContactEntries(
  provider: DirectoryProvider,
): readonly ProviderContactEntry[] {
  const entries: ProviderContactEntry[] = [];
  for (const channel of PROVIDER_CONTACT_CHANNELS) {
    const value = provider.contact[channel];
    if (typeof value === "string" && value.trim()) {
      entries.push({ channel, value: value.trim() });
    }
  }
  return entries;
}

/** Did this provider publish any way at all of being reached? */
export function isContactable(provider: DirectoryProvider): boolean {
  return providerContactEntries(provider).length > 0;
}

/**
 * Neither a location nor a contact channel.
 *
 * Not an error and not a reason to drop the row - the provider exists and the source said so. It is
 * a fact worth surfacing, because it is what a directory entry looks like when nobody has filled it
 * in, and it is the state an operator has to fix.
 */
export function isUnreachableProvider(provider: DirectoryProvider): boolean {
  return providerPosition(provider) === null && !isContactable(provider);
}

/**
 * Split a list into the providers that can be placed and the ones that cannot.
 *
 * For a surface that ranks by distance: rank `locatable`, and list `contactOnly` after it, without
 * distances. Both keep their input order.
 *
 * `sortByDistance` already handles this correctly on its own - a `null` position sorts last with
 * `distanceKm: null` - so this is for a caller that wants the two groups presented differently
 * rather than one list with blanks in it. Neither route may invent a coordinate.
 */
export function partitionByLocatability<T extends DirectoryProvider>(
  providers: readonly T[],
): { locatable: T[]; contactOnly: T[] } {
  const locatable: T[] = [];
  const contactOnly: T[] = [];
  for (const provider of providers) {
    (providerPosition(provider) === null ? contactOnly : locatable).push(provider);
  }
  return { locatable, contactOnly };
}
