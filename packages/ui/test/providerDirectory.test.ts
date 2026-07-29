import { describe, expect, it, vi } from "vitest";
import {
  centralDirectory,
  createMemoryProviderDirectoryCache,
  onchainDirectory,
  withProviderDirectoryCache,
  type DirectoryProvider,
  type ProviderDirectory,
  type ProviderDirectoryResult,
} from "../src/directory";
import type { CentralBusiness } from "../src/api/types";

const CENTRAL_PROVIDER: CentralBusiness = {
  businessId: "business-1",
  type: "vet",
  name: "North Star Veterinary",
  geo: { lat: 1.3521, lng: 103.8198 },
  services: ["vaccination"],
  apiBaseUrl: "https://north-star.test/api",
  domain: "north-star.test",
  documentStores: ["https://north-star.test/documents"],
  hmacKeyId: "hmac-1",
};

const NO_CONTACTS = {
  phone: null,
  whatsapp: null,
  telegram: null,
  email: null,
  website: null,
} as const;

const PROVIDER: DirectoryProvider = {
  providerId: "business-1",
  kind: "vet",
  name: "North Star Veterinary",
  geo: { lat: 1.3521, lng: 103.8198 },
  services: ["vaccination"],
  contact: { ...NO_CONTACTS },
  domain: "north-star.test",
  bindingState: "unavailable",
  active: null,
};

const CONTACT_ONLY_PROVIDER: DirectoryProvider = {
  ...PROVIDER,
  geo: null,
  contact: {
    phone: "+65 6123 4567",
    whatsapp: "+65 9000 0000",
    telegram: "@northstarvet",
    email: "hello@north-star.test",
    website: null,
  },
};

const SECOND_PROVIDER: DirectoryProvider = {
  ...PROVIDER,
  providerId: "business-2",
  name: "Good Dog Grooming",
  kind: "groomer",
};

describe("centralDirectory", () => {
  it("performs one full fetch with no query and reports a live, honestly unanchored result", async () => {
    const listBusinesses = vi.fn(async () => ({
      // The central wire is not a binding oracle. Even if it grows or injects this field, the seam
      // must derive `unavailable` itself rather than accepting an unperformed verification.
      businesses: [{ ...CENTRAL_PROVIDER, bindingState: "verified" }],
    }));
    const result = await centralDirectory(
      { base: "https://central.test", listBusinesses },
      { now: () => 1_000 },
    ).read();

    expect(listBusinesses).toHaveBeenCalledTimes(1);
    expect(listBusinesses.mock.calls[0]).toEqual([]);
    expect(result).toEqual({
      state: "found",
      source: "central",
      providers: [PROVIDER],
      observation: "live",
      blockNumber: null,
      readAt: 1_000,
      expiresAt: null,
    });
  });

  it("returns an explicit empty state only after a successful empty response", async () => {
    const result = await centralDirectory(
      { base: "https://central.test", listBusinesses: async () => ({ businesses: [] }) },
      { now: () => 2_000 },
    ).read();

    expect(result).toEqual({
      state: "empty",
      source: "central",
      providers: [],
      observation: "live",
      blockNumber: null,
      readAt: 2_000,
      expiresAt: null,
    });
  });

  it("turns a source failure into unavailable without manufacturing an empty list", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () => {
          throw new Error("central is offline");
        },
      },
      { now: () => 3_000 },
    ).read();

    expect(result).toEqual({
      state: "unavailable",
      source: "central",
      reason: "sourceUnavailable",
      detail: "central is offline",
      attemptedAt: 3_000,
    });
    expect("providers" in result).toBe(false);
  });

  it("treats a malformed success response as unavailable, not empty", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () =>
          ({ businesses: "not-an-array" }) as unknown as { businesses: DirectoryProvider[] },
      },
      { now: () => 4_000 },
    ).read();

    expect(result.state).toBe("unavailable");
    if (result.state !== "unavailable") throw new Error("expected unavailable");
    expect(result.reason).toBe("malformedResponse");
    expect(result.detail).toContain("not treated as an empty directory");
    expect("providers" in result).toBe(false);
  });

  it("rejects a malformed geo object inside an otherwise array-shaped response", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () =>
          ({
            businesses: [{ ...CENTRAL_PROVIDER, geo: { lat: null, lng: 103.8198 } }],
          }) as unknown as {
            businesses: CentralBusiness[];
          },
      },
      { now: () => 5_000 },
    ).read();

    expect(result).toMatchObject({ state: "unavailable", reason: "malformedResponse" });
    expect("providers" in result).toBe(false);
  });

  it.each(["omitted", "null"] as const)(
    "keeps a contact-only row with %s geo in a successful found snapshot",
    async (geoShape) => {
      const { geo: _geo, ...withoutGeo } = CENTRAL_PROVIDER;
      const row = {
        ...withoutGeo,
        ...(geoShape === "null" ? { geo: null } : {}),
        contact: {
          phone: " +65 6123 4567 ",
          whatsapp: "+65 9000 0000",
          telegram: " @northstarvet ",
          email: " hello@north-star.test ",
        },
      };
      const result = await centralDirectory(
        {
          base: "https://central.test",
          listBusinesses: async () =>
            ({ businesses: [row] }) as unknown as { businesses: CentralBusiness[] },
        },
        { now: () => 5_100 },
      ).read();

      expect(result).toEqual({
        state: "found",
        source: "central",
        providers: [CONTACT_ONLY_PROVIDER],
        observation: "live",
        blockNumber: null,
        readAt: 5_100,
        expiresAt: null,
      });
      if (result.state !== "found") throw new Error("expected found");
      expect(result.providers[0].geo).toBeNull();
      expect(result.providers[0].contact).toEqual(CONTACT_ONLY_PROVIDER.contact);
    },
  );

  it("rejects malformed contact fields rather than partially trusting the row", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () =>
          ({
            businesses: [{ ...CENTRAL_PROVIDER, contact: { phone: 61234567 } }],
          }) as unknown as { businesses: CentralBusiness[] },
      },
      { now: () => 5_200 },
    ).read();

    expect(result).toMatchObject({ state: "unavailable", reason: "malformedResponse" });
  });

  // Mirrors the Kotlin and Swift adapters: `providerId` is the consumer's list identity, so a
  // repeated one is a bad response rather than two rows to render on top of each other.
  it("refuses a response that repeats a provider id rather than rendering both rows", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () => ({
          businesses: [CENTRAL_PROVIDER, { ...CENTRAL_PROVIDER, name: "Impostor Vet" }],
        }),
      },
      { now: () => 5_400 },
    ).read();

    expect(result).toMatchObject({ state: "unavailable", reason: "malformedResponse" });
  });

  it("normalizes a domain-less central row without exposing central-only HMAC metadata", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () => ({
          businesses: [{ ...CENTRAL_PROVIDER, domain: "  " }],
        }),
      },
      { now: () => 5_500 },
    ).read();

    expect(result).toMatchObject({
      state: "found",
      providers: [{ ...PROVIDER, domain: null, bindingState: "noDomainListed" }],
    });
    if (result.state !== "found") throw new Error("expected found");
    expect("hmacKeyId" in result.providers[0]).toBe(false);
  });

  it("accepts a row that omits the central-only fields this seam never projects", async () => {
    const { apiBaseUrl: _apiBaseUrl, documentStores: _stores, hmacKeyId: _key, ...consumed } =
      CENTRAL_PROVIDER;
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () =>
          ({ businesses: [consumed] }) as unknown as { businesses: CentralBusiness[] },
      },
      { now: () => 5_600 },
    ).read();

    expect(result).toMatchObject({ state: "found", providers: [PROVIDER] });
  });

  it("rejects an out-of-range coordinate rather than ranking a provider it cannot place", async () => {
    const result = await centralDirectory(
      {
        base: "https://central.test",
        listBusinesses: async () => ({
          businesses: [{ ...CENTRAL_PROVIDER, geo: { lat: 91, lng: 103.8198 } }],
        }),
      },
      { now: () => 5_700 },
    ).read();

    expect(result).toMatchObject({ state: "unavailable", reason: "malformedResponse" });
    expect("providers" in result).toBe(false);
  });

  it("derives a cache namespace that separates configured bases and joins identical ones", async () => {
    const listBusinesses = async () => ({ businesses: [] });
    const one = centralDirectory({ base: "https://one.test", listBusinesses });
    const two = centralDirectory({ base: "https://two.test", listBusinesses });
    const oneAgain = centralDirectory({ base: "https://one.test", listBusinesses });

    expect(one.cacheNamespace).not.toBe(two.cacheNamespace);
    // The equality half is what catches an instance-scoped namespace, which would disable every
    // replay while still passing the inequality assertion above.
    expect(oneAgain.cacheNamespace).toBe(one.cacheNamespace);

    const relativeOnA = centralDirectory(
      { base: "/api", listBusinesses },
      { documentOrigin: "https://portal-a.test" },
    );
    const relativeOnB = centralDirectory(
      { base: "/api", listBusinesses },
      { documentOrigin: "https://portal-b.test" },
    );

    expect(relativeOnA.cacheNamespace).not.toBe(relativeOnB.cacheNamespace);
    expect(relativeOnA.cacheNamespace).toBe("central:https://portal-a.test/api");

    // A base with nothing to resolve names a fixed sentinel. Resolving it against the page URL
    // instead would namespace one deployment per route, which is a worse collision than sharing
    // one honest sentinel.
    const unresolved = centralDirectory(
      { base: "", listBusinesses },
      { documentOrigin: "https://portal-a.test" },
    );
    expect(unresolved.cacheNamespace).toBe("central:unresolved-base");
  });

  it("ignores even a force-passed viewport or region object and still makes the same full fetch", async () => {
    const listBusinesses = vi.fn(async () => ({ businesses: [] }));
    const directory = centralDirectory({ base: "https://central.test", listBusinesses });
    const hostileRead = directory.read as unknown as (query: unknown) => Promise<unknown>;

    await hostileRead({
      viewport: { north: 2, south: 1, east: 104, west: 103 },
      bbox: "1,103,2,104",
      region: "w21z",
      geohash: "w21z",
    });

    expect(listBusinesses.mock.calls[0]).toEqual([]);
  });
});

describe("onchainDirectory stub", () => {
  it("reports that the provider registry is unavailable and performs no pretend empty read", async () => {
    const result = await onchainDirectory({ now: () => 6_000 }).read();

    expect(result).toEqual({
      state: "unavailable",
      source: "onchain",
      reason: "providerRegistryUnavailable",
      detail:
        "The on-chain provider directory is unavailable: its provider registry and paged-read ABI are not designed or deployed yet",
      attemptedAt: 6_000,
    });
    expect("providers" in result).toBe(false);
  });
});

function scriptedDirectory(
  source: ProviderDirectory["source"],
  results: ProviderDirectoryResult[],
  cacheNamespace = `${source}:fixture`,
): ProviderDirectory & { calls: () => number } {
  let callCount = 0;
  return {
    source,
    cacheNamespace,
    calls: () => callCount,
    async read() {
      const result = results[callCount];
      callCount += 1;
      if (result === undefined) throw new Error("scripted directory exhausted");
      return result;
    },
  };
}

const found = (
  source: ProviderDirectory["source"],
  providers: readonly [DirectoryProvider, ...DirectoryProvider[]],
  readAt: number,
  blockNumber: number | null,
): ProviderDirectoryResult => ({
  state: "found",
  source,
  providers,
  observation: "live",
  blockNumber,
  readAt,
  expiresAt: null,
});

const empty = (
  source: ProviderDirectory["source"],
  readAt: number,
  blockNumber: number | null,
): ProviderDirectoryResult => ({
  state: "empty",
  source,
  providers: [],
  observation: "live",
  blockNumber,
  readAt,
  expiresAt: null,
});

const unavailable = (
  source: ProviderDirectory["source"],
  attemptedAt: number,
): ProviderDirectoryResult => ({
  state: "unavailable",
  source,
  reason: source === "onchain" ? "providerRegistryUnavailable" : "sourceUnavailable",
  detail: "source could not answer",
  attemptedAt,
});

describe("withProviderDirectoryCache", () => {
  it("writes and replays a contact-only provider without manufacturing a pin", async () => {
    let time = 9_000;
    const source = scriptedDirectory("central", [
      found("central", [CONTACT_ONLY_PROVIDER], 9_000, null),
      unavailable("central", 9_100),
    ]);
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(source, {
      cache,
      ttlMs: 1_000,
      now: () => time,
    });

    expect(await directory.read()).toMatchObject({
      state: "found",
      providers: [CONTACT_ONLY_PROVIDER],
      observation: "live",
    });
    expect(cache.read()?.snapshot).toMatchObject({
      state: "found",
      providers: [CONTACT_ONLY_PROVIDER],
    });

    time = 9_100;
    expect(await directory.read()).toMatchObject({
      state: "found",
      providers: [CONTACT_ONLY_PROVIDER],
      observation: "stored",
    });
    expect(CONTACT_ONLY_PROVIDER.geo).toBeNull();
  });

  it("stores a live snapshot with its block anchor and TTL, then replays it as stored", async () => {
    let time = 10_000;
    const source = scriptedDirectory("onchain", [
      found("onchain", [PROVIDER], 9_900, 291_555),
      unavailable("onchain", 10_500),
    ]);
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(source, {
      cache,
      ttlMs: 1_000,
      now: () => time,
    });

    const live = await directory.read();
    expect(live).toMatchObject({
      state: "found",
      observation: "live",
      blockNumber: 291_555,
      readAt: 9_900,
      expiresAt: 10_900,
    });
    expect(cache.read()).toMatchObject({
      namespace: "onchain:fixture",
      expiresAt: 10_900,
      snapshot: { blockNumber: 291_555, readAt: 9_900 },
    });

    time = 10_899;
    const stored = await directory.read();
    expect(stored).toMatchObject({
      state: "found",
      observation: "stored",
      blockNumber: 291_555,
      readAt: 9_900,
      expiresAt: 10_900,
    });
    expect(source.calls()).toBe(2);
  });

  it("does not replay a snapshot at its exact expiry", async () => {
    let time = 20_000;
    const source = scriptedDirectory("central", [
      found("central", [PROVIDER], 20_000, null),
      unavailable("central", 21_000),
    ]);
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(source, {
      cache,
      ttlMs: 1_000,
      now: () => time,
    });

    await directory.read();
    time = 21_000;
    const result = await directory.read();

    expect(result.state).toBe("unavailable");
    expect("providers" in result).toBe(false);
    expect(cache.read()).toBeNull();
  });

  it("can replay a successful empty observation without confusing it with unavailability", async () => {
    let time = 30_000;
    const source = scriptedDirectory("central", [
      empty("central", 30_000, null),
      unavailable("central", 30_100),
    ]);
    const directory = withProviderDirectoryCache(source, {
      cache: createMemoryProviderDirectoryCache(),
      ttlMs: 1_000,
      now: () => time,
    });

    const live = await directory.read();
    expect(live).toMatchObject({ state: "empty", providers: [], observation: "live" });

    time = 30_100;
    const stored = await directory.read();
    expect(stored).toMatchObject({
      state: "empty",
      providers: [],
      observation: "stored",
      blockNumber: null,
    });
  });

  it("replaces a stored snapshot after the next successful live refresh", async () => {
    let time = 40_000;
    const source = scriptedDirectory("onchain", [
      found("onchain", [PROVIDER], 40_000, 100),
      unavailable("onchain", 40_100),
      found("onchain", [SECOND_PROVIDER], 40_200, 101),
    ]);
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(source, {
      cache,
      ttlMs: 1_000,
      now: () => time,
    });

    await directory.read();
    time = 40_100;
    expect(await directory.read()).toMatchObject({ observation: "stored", blockNumber: 100 });
    time = 40_200;
    const refreshed = await directory.read();

    expect(refreshed).toMatchObject({
      state: "found",
      observation: "live",
      providers: [SECOND_PROVIDER],
      blockNumber: 101,
      readAt: 40_200,
      expiresAt: 41_200,
    });
    expect(cache.read()?.snapshot).toMatchObject({
      observation: "live",
      providers: [SECOND_PROVIDER],
      blockNumber: 101,
    });
  });

  it("never replays a central snapshot as an on-chain result after a source swap", async () => {
    let time = 50_000;
    const cache = createMemoryProviderDirectoryCache();
    const central = withProviderDirectoryCache(
      scriptedDirectory("central", [found("central", [PROVIDER], 50_000, null)]),
      { cache, ttlMs: 1_000, now: () => time },
    );
    await central.read();

    time = 50_100;
    const onchain = withProviderDirectoryCache(
      scriptedDirectory("onchain", [unavailable("onchain", 50_100)]),
      { cache, ttlMs: 1_000, now: () => time },
    );
    const result = await onchain.read();

    expect(result.state).toBe("unavailable");
    expect("providers" in result).toBe(false);
    expect(cache.read()).toBeNull();
  });

  it("never replays between two configurations of the same source kind", async () => {
    let time = 55_000;
    const cache = createMemoryProviderDirectoryCache();
    const firstOrigin = withProviderDirectoryCache(
      scriptedDirectory(
        "central",
        [found("central", [PROVIDER], 55_000, null)],
        "central:https://one.test",
      ),
      { cache, ttlMs: 1_000, now: () => time },
    );
    await firstOrigin.read();

    time = 55_100;
    const secondOrigin = withProviderDirectoryCache(
      scriptedDirectory(
        "central",
        [unavailable("central", 55_100)],
        "central:https://two.test",
      ),
      { cache, ttlMs: 1_000, now: () => time },
    );
    const result = await secondOrigin.read();

    expect(result.state).toBe("unavailable");
    expect(cache.read()).toBeNull();
  });

  it("does not renew the hard TTL of a stored result or a nested cache", async () => {
    let time = 60_000;
    const raw = scriptedDirectory("onchain", [
      found("onchain", [PROVIDER], 60_000, 200),
      unavailable("onchain", 60_050),
      unavailable("onchain", 60_100),
    ]);
    const inner = withProviderDirectoryCache(raw, {
      cache: createMemoryProviderDirectoryCache(),
      ttlMs: 100,
      now: () => time,
    });
    const outerCache = createMemoryProviderDirectoryCache();
    const outer = withProviderDirectoryCache(inner, {
      cache: outerCache,
      ttlMs: 1_000,
      now: () => time,
    });

    expect(await outer.read()).toMatchObject({ observation: "live", expiresAt: 60_100 });
    time = 60_050;
    expect(await outer.read()).toMatchObject({ observation: "stored", expiresAt: 60_100 });
    expect(outerCache.read()?.expiresAt).toBe(60_100);

    time = 60_100;
    const expired = await outer.read();
    expect(expired.state).toBe("unavailable");
    expect("providers" in expired).toBe(false);
  });

  it("keeps an initial unavailable read unavailable when there is no cache", async () => {
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(
      scriptedDirectory("central", [unavailable("central", 70_000)]),
      { cache, ttlMs: 1_000, now: () => 70_000 },
    );

    const result = await directory.read();
    expect(result.state).toBe("unavailable");
    expect("providers" in result).toBe(false);
    expect(cache.read()).toBeNull();
  });

  it("rejects contradictory source provenance instead of caching or relabelling it", async () => {
    const cache = createMemoryProviderDirectoryCache();
    const source = scriptedDirectory("central", [unavailable("onchain", 75_000)]);
    const result = await withProviderDirectoryCache(source, {
      cache,
      ttlMs: 1_000,
      now: () => 75_000,
    }).read();

    expect(result).toMatchObject({
      state: "unavailable",
      source: "central",
      reason: "inconsistentSource",
    });
    expect(cache.read()).toBeNull();
  });

  it("rejects invalid stored, timestamp, and block-anchor metadata", async () => {
    const cases: ProviderDirectoryResult[] = [
      {
        ...found("central", [PROVIDER], 80_000, null),
        observation: "stored",
        expiresAt: null,
      } as ProviderDirectoryResult,
      found("central", [PROVIDER], Number.NaN, null),
      found("central", [PROVIDER], 80_000, -1),
      found("central", [PROVIDER], 70_000, null),
    ];

    for (const value of cases) {
      const result = await withProviderDirectoryCache(
        scriptedDirectory("central", [value]),
        {
          cache: createMemoryProviderDirectoryCache(),
          ttlMs: 1_000,
          now: () => 80_000,
        },
      ).read();
      expect(result).toMatchObject({ state: "unavailable", reason: "invalidSnapshot" });
      expect("providers" in result).toBe(false);
    }
  });

  it("rejects a corrupted cache entry whose duplicated deadline disagrees", async () => {
    const cache = createMemoryProviderDirectoryCache();
    const snapshot = {
      ...found("central", [PROVIDER], 85_000, null),
      expiresAt: 86_000,
    } as Extract<ProviderDirectoryResult, { state: "found" }> & { expiresAt: number };
    cache.write({
      namespace: "central:fixture",
      snapshot,
      expiresAt: 86_001,
    });

    const result = await withProviderDirectoryCache(
      scriptedDirectory("central", [unavailable("central", 85_100)]),
      { cache, ttlMs: 1_000, now: () => 85_100 },
    ).read();

    expect(result.state).toBe("unavailable");
    expect(cache.read()).toBeNull();
  });

  it("replays a cached snapshot when the wrapped directory throws instead of resolving", async () => {
    let time = 90_000;
    let shouldThrow = false;
    const source: ProviderDirectory = {
      source: "central",
      cacheNamespace: "central:fixture",
      async read() {
        if (shouldThrow) throw new Error("the source blew up");
        return found("central", [PROVIDER], 90_000, null);
      },
    };
    const directory = withProviderDirectoryCache(source, {
      cache: createMemoryProviderDirectoryCache(),
      ttlMs: 1_000,
      now: () => time,
    });

    await directory.read();
    shouldThrow = true;
    time = 90_500;
    const replayed = await directory.read();

    expect(replayed).toMatchObject({
      state: "found",
      observation: "stored",
      providers: [PROVIDER],
      readAt: 90_000,
      expiresAt: 91_000,
    });
  });

  it("turns a throw with no usable cache into unavailable rather than propagating it", async () => {
    const cache = createMemoryProviderDirectoryCache();
    const result = await withProviderDirectoryCache(
      {
        source: "onchain",
        cacheNamespace: "onchain:fixture",
        async read(): Promise<ProviderDirectoryResult> {
          throw new Error("rpc exploded");
        },
      },
      { cache, ttlMs: 1_000, now: () => 91_000 },
    ).read();

    expect(result).toMatchObject({
      state: "unavailable",
      source: "onchain",
      reason: "sourceUnavailable",
      attemptedAt: 91_000,
    });
    if (result.state !== "unavailable") throw new Error("expected unavailable");
    expect(result.detail).toContain("rpc exploded");
    expect("providers" in result).toBe(false);
    expect(cache.read()).toBeNull();
  });

  it("re-validates a cached entry on replay exactly as it does on write", async () => {
    const corruptions: Array<{ snapshot: Record<string, unknown>; expiresAt: number }> = [
      // A `found` that lost its providers must not replay as found-with-zero-providers.
      { snapshot: { ...found("central", [PROVIDER], 95_000, null), providers: [], expiresAt: 96_000 }, expiresAt: 96_000 },
      { snapshot: { ...empty("central", 95_000, null), providers: [PROVIDER], expiresAt: 96_000 }, expiresAt: 96_000 },
      { snapshot: { ...found("central", [PROVIDER], Number.NaN, null), expiresAt: 96_000 }, expiresAt: 96_000 },
      { snapshot: { ...found("central", [PROVIDER], 95_000, -1), expiresAt: 96_000 }, expiresAt: 96_000 },
      // Read "in the future": the device clock moved backwards, so the stored absolute deadline
      // would silently extend the hard window by however far it jumped.
      { snapshot: { ...found("central", [PROVIDER], 95_400, null), expiresAt: 96_400 }, expiresAt: 96_400 },
      // A deadline longer than this wrapper's own hard TTL allows.
      { snapshot: { ...found("central", [PROVIDER], 95_000, null), expiresAt: 99_000 }, expiresAt: 99_000 },
    ];

    for (const corrupted of corruptions) {
      const cache = createMemoryProviderDirectoryCache();
      cache.write({
        namespace: "central:fixture",
        snapshot: corrupted.snapshot as unknown as Extract<
          ProviderDirectoryResult,
          { state: "found" }
        > & { expiresAt: number },
        expiresAt: corrupted.expiresAt,
      });

      const result = await withProviderDirectoryCache(
        scriptedDirectory("central", [unavailable("central", 95_300)]),
        { cache, ttlMs: 1_000, now: () => 95_300 },
      ).read();

      expect(result.state).toBe("unavailable");
      expect("providers" in result).toBe(false);
      expect(cache.read()).toBeNull();
    }
  });

  it("rejects a missing or nonsensical hard TTL at construction", () => {
    const source = scriptedDirectory("central", []);
    for (const ttlMs of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() =>
        withProviderDirectoryCache(source, {
          cache: createMemoryProviderDirectoryCache(),
          ttlMs,
        }),
      ).toThrow(RangeError);
    }
  });
});
