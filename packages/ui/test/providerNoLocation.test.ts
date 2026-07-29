/**
 * A location-less provider is first class.
 *
 * Two defects are pinned here, and they fail in opposite directions:
 *
 * 1. `DirectoryProvider.geo` was a non-optional `LatLng`, and the wire's was too, so a provider that
 *    published no location was stored and served as `0, 0`. That is a legal coordinate off the coast
 *    of Ghana, `isValidLatLng` accepts it, and every surface drew a confident pin in the Gulf of
 *    Guinea. Absence had no representation at all.
 *
 * 2. `isDirectoryRow` required `geo`, and `hasDirectoryRows` is all-or-nothing, so the moment
 *    location became optional a SINGLE contact-only provider would have taken the entire directory
 *    to `unavailable` for every consumer - an all-or-nothing failure hiding inside a per-row
 *    validator.
 *
 * The all-or-nothing rule itself is deliberately preserved for a MALFORMED row: absence is a fact
 * this source can state, malformed is a response we cannot trust.
 */

import { describe, expect, it } from "vitest";
import { centralDirectory } from "../src/directory/sources";
import {
  hasDirectionsDestination,
  isContactable,
  isUnreachableProvider,
  partitionByLocatability,
  providerContactEntries,
  providerPosition,
} from "../src/directory/providers";
import type { DirectoryProvider } from "../src/directory/types";
import { sortByDistance } from "../src/geo";

const LOCATED = {
  businessId: "biz-located",
  type: "vet",
  name: "North Star Veterinary",
  geo: { lat: 1.3521, lng: 103.8198 },
  services: ["vaccination"],
  apiBaseUrl: "https://north-star.test/api",
  domain: "north-star.test",
  documentStores: [],
  hmacKeyId: "hmac-1",
};

/** The captain's case: contact channels, no address. */
const CONTACT_ONLY = {
  businessId: "biz-contact-only",
  type: "groomer",
  name: "Pawsh Mobile Grooming",
  geo: null,
  contact: {
    phone: "+65 6123 4567",
    whatsapp: "+65 9123 4567",
    telegram: "@pawshmobile",
    email: "hello@pawsh.test",
    website: "https://pawsh.test",
  },
  services: ["grooming"],
  apiBaseUrl: "https://pawsh.test/api",
  domain: "pawsh.test",
  documentStores: [],
  hmacKeyId: "hmac-2",
};

async function read(businesses: unknown[]) {
  return centralDirectory(
    {
      base: "https://central.test",
      listBusinesses: async () => ({ businesses }) as never,
    },
    { now: () => 1_000, documentOrigin: "https://portal.test" },
  ).read();
}

describe("a geo-less row is accepted", () => {
  it("carries geo as null - never an empty object, and never a 0,0 substitute", async () => {
    const result = await read([CONTACT_ONLY]);
    expect(result.state).toBe("found");
    if (result.state !== "found") return;

    const [provider] = result.providers;
    // Identity, not truthiness. Object spread of `null` yields `{}` rather than `null` and TypeScript
    // drops `null` from a spread's inferred type, so a `{ ...business.geo }` regression would
    // typecheck clean AND pass a truthiness check while handing every consumer an unusable position
    // that is not `null`.
    expect(provider.geo).toBeNull();
    expect(provider.contacts.phone).toBe("+65 6123 4567");
    expect(provider.contacts.whatsapp).toBe("+65 9123 4567");
    expect(provider.contacts.telegram).toBe("@pawshmobile");
    expect(provider.contacts.email).toBe("hello@pawsh.test");
    expect(provider.contacts.website).toBe("https://pawsh.test");
  });

  it("accepts an ABSENT geo key as well as an explicit null", async () => {
    // Our own server emits an explicit null, but a serializer that omits nulls is an ordinary wire
    // difference. Rejecting it would be the same all-or-nothing hazard this slice exists to close.
    const { geo: _omitted, ...noGeoKey } = CONTACT_ONLY;
    const result = await read([noGeoKey]);
    expect(result.state).toBe("found");
    if (result.state !== "found") return;
    expect(result.providers[0].geo).toBeNull();
  });

  it("fills every contact channel with null when the server sends no contact key at all", async () => {
    const result = await read([LOCATED]);
    expect(result.state).toBe("found");
    if (result.state !== "found") return;
    expect(result.providers[0].contacts).toEqual({
      phone: null,
      whatsapp: null,
      telegram: null,
      email: null,
      website: null,
    });
  });

  it("reads a blank contact string as the absent channel it means", async () => {
    const result = await read([
      { ...CONTACT_ONLY, contact: { phone: "   ", email: "", website: " https://pawsh.test " } },
    ]);
    expect(result.state).toBe("found");
    if (result.state !== "found") return;
    expect(result.providers[0].contacts.phone).toBeNull();
    expect(result.providers[0].contacts.email).toBeNull();
    expect(result.providers[0].contacts.website).toBe("https://pawsh.test");
  });
});

describe("ONE geo-less row cannot take down the whole directory", () => {
  it("returns BOTH providers when one has no location and one does", async () => {
    // THE regression. Under the previous validator this batch answered `unavailable`, so a single
    // contact-only provider blanked the directory - including every provider that was fine.
    const result = await read([CONTACT_ONLY, LOCATED]);

    expect(result.state).toBe("found");
    if (result.state !== "found") return;
    expect(result.providers.map((p) => p.providerId)).toEqual(["biz-contact-only", "biz-located"]);
    expect(result.providers[0].geo).toBeNull();
    expect(result.providers[1].geo).toEqual({ lat: 1.3521, lng: 103.8198 });
  });

  it("still takes the whole batch to unavailable for a MALFORMED coordinate", async () => {
    // Preserved on purpose: absence is a fact the source can state; a coordinate that is present and
    // nonsense means the response is not what we asked for, and must not degrade into a successful
    // list that silently omits rows.
    for (const badGeo of [
      { lat: 91, lng: 103.8198 },
      { lat: 1.3521, lng: 181 },
      { lat: null, lng: 103.8198 },
      { lat: "1.3521", lng: 103.8198 },
      "1.3521,103.8198",
      [],
    ]) {
      const result = await read([{ ...LOCATED, geo: badGeo }, CONTACT_ONLY]);
      expect(result.state, `geo ${JSON.stringify(badGeo)} must not be accepted`).toBe("unavailable");
      if (result.state !== "unavailable") return;
      expect(result.reason).toBe("malformedResponse");
    }
  });

  it("still takes the whole batch to unavailable for a wrong-typed contact channel", async () => {
    const result = await read([{ ...CONTACT_ONLY, contact: { phone: 6512345678 } }]);
    expect(result.state).toBe("unavailable");
  });
});

describe("a location-less provider is listed and contactable, but never placed", () => {
  const contactOnly: DirectoryProvider = {
    providerId: "p-contact",
    kind: "groomer",
    name: "Pawsh Mobile Grooming",
    geo: null,
    contacts: {
      phone: "+65 6123 4567",
      whatsapp: null,
      telegram: "@pawshmobile",
      email: null,
      website: null,
    },
    services: ["grooming"],
    domain: "pawsh.test",
    active: null,
  };
  const located: DirectoryProvider = {
    ...contactOnly,
    providerId: "p-located",
    name: "North Star Veterinary",
    geo: { lat: 1.3521, lng: 103.8198 },
  };

  it("has no position and no Directions destination", () => {
    expect(providerPosition(contactOnly)).toBeNull();
    expect(hasDirectionsDestination(contactOnly)).toBe(false);
    expect(hasDirectionsDestination(located)).toBe(true);
  });

  it("is contactable, and lists its channels in listing order", () => {
    expect(isContactable(contactOnly)).toBe(true);
    expect(providerContactEntries(contactOnly)).toEqual([
      { channel: "phone", value: "+65 6123 4567" },
      { channel: "telegram", value: "@pawshmobile" },
    ]);
  });

  it("sorts LAST with a null distance rather than ranking as distance zero", () => {
    // A missing position compared as 0 would rank an unplaceable provider above every real one -
    // the same "absence rendered as a definite value" defect one level up.
    const ranked = sortByDistance({ lat: 1.3, lng: 103.8 }, [contactOnly, located], (p) => p.geo);
    expect(ranked.map((r) => r.item.providerId)).toEqual(["p-located", "p-contact"]);
    expect(ranked[1].distanceKm).toBeNull();
  });

  it("partitions into locatable and contact-only, keeping input order", () => {
    const { locatable, contactOnly: only } = partitionByLocatability([contactOnly, located]);
    expect(locatable.map((p) => p.providerId)).toEqual(["p-located"]);
    expect(only.map((p) => p.providerId)).toEqual(["p-contact"]);
  });

  it("re-checks the coordinate rather than trusting the field", () => {
    // A snapshot replayed from an older build's cache, or a hand-built fixture, has not passed the
    // source seam's validator. An out-of-range pair must not become a distance here either.
    const bogus = { ...located, geo: { lat: 91, lng: 0 } };
    expect(providerPosition(bogus)).toBeNull();
    expect(hasDirectionsDestination(bogus)).toBe(false);
  });

  it("names a provider with neither a location nor a channel, and still lists it", () => {
    const empty: DirectoryProvider = {
      ...contactOnly,
      providerId: "p-empty",
      contacts: { phone: null, whatsapp: null, telegram: null, email: null, website: null },
    };
    expect(isUnreachableProvider(empty)).toBe(true);
    expect(isUnreachableProvider(contactOnly)).toBe(false);
    expect(isUnreachableProvider(located)).toBe(false);
    // It is still a provider the source told us about; a listing shows the state rather than the row
    // vanishing.
    expect(partitionByLocatability([empty]).contactOnly).toHaveLength(1);
  });
});
