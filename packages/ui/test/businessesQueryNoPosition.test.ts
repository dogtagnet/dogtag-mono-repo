import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createCentralClient } from "../src/api/central";

/**
 * The client must not be able to tell the server where the caller is.
 *
 * `GET /v1/businesses` still accepts `near=<lat>,<lng>` and `radius=` and still filters server-side
 * - see the deprecation note on `BusinessesQuery` in `stacks/admin/api/src/routes.rs`. It is mounted
 * on the PUBLIC, UNAUTHENTICATED router, so anything sent there arrives beside the caller's IP with
 * no account attached. dogtag is built on the owner never revealing where they are, so the fix is
 * not to authenticate that route: it is for this client to have no way to send a position at all.
 * Distance is computed on the device instead (`packages/ui/src/geo/`), over a provider set whose
 * request is identical whoever makes it.
 *
 * WHY THIS TEST EXISTS RATHER THAN RELYING ON THE TYPE. Removing `near`/`radius` from
 * `BusinessesQuery` makes `q.near` a compile error today, which is a good first line. But it is one
 * `git revert` away from being undone, and the two halves live in different files - a reviewer
 * looking at a diff that re-adds a field to `types.ts` will not necessarily connect it to the line
 * in `central.ts` that would then start emitting it. This test observes what actually leaves the
 * client, so BOTH halves have to be re-added AND this has to be deleted before a position can reach
 * the wire.
 *
 * That matters because passing `near` is the obvious way to build "nearby". It would work on the
 * first try, and it would turn a latent leak into a live one in a single line.
 */

const originalFetch = globalThis.fetch;
let requested: string[] = [];

beforeEach(() => {
  requested = [];
  globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
    requested.push(String(input));
    return new Response(JSON.stringify({ businesses: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }) as unknown as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

const client = () => createCentralClient({ baseUrl: "https://central.test" });

/** The single URL the call under test produced. */
const soleUrl = (): string => {
  expect(requested).toHaveLength(1);
  const u = requested[0];
  if (u === undefined) throw new Error("no request captured");
  return u;
};

describe("listBusinesses sends no position", () => {
  it("sends a bare path when called with no query - the shape both callers use today", () => {
    // `Businesses.tsx` and `Dashboard.tsx` both call `listBusinesses()` with no arguments.
    void client().listBusinesses();
    expect(soleUrl()).toBe("https://central.test/v1/businesses");
  });

  it("sends only `type` when a query is given", () => {
    void client().listBusinesses({ type: "vet" });
    expect(soleUrl()).toBe("https://central.test/v1/businesses?type=vet");
  });

  it("emits NOTHING position-like even when the caller forces extra fields onto the query", () => {
    // The regression guard. The cast is the point: it simulates a future change that re-adds the
    // fields to `BusinessesQuery`, so that this test fails on the `central.ts` half alone rather
    // than being silently satisfied by the type error that a re-adding change would remove.
    const hostile = {
      type: "vet",
      near: "1.3521,103.8198",
      radius: 5,
      lat: 1.3521,
      lng: 103.8198,
      latitude: 1.3521,
      longitude: 103.8198,
      coords: "1.3521,103.8198",
      location: "1.3521,103.8198",
      bbox: "1,103,2,104",
      geohash: "w21z",
    } as unknown as Parameters<ReturnType<typeof client>["listBusinesses"]>[0];

    void client().listBusinesses(hostile);
    const url = soleUrl();

    // No parameter name that could carry a position.
    const params = new URL(url).searchParams;
    expect([...params.keys()]).toEqual(["type"]);

    // And no coordinate VALUE anywhere in the URL, however it might have been spelled - a check on
    // parameter names alone would miss `?type=vet%3A1.3521`.
    for (const leak of ["1.3521", "103.8198", "geohash", "w21z", "bbox", "near", "radius"]) {
      expect(url, `"${leak}" reached the wire`).not.toContain(leak);
    }
  });

  it("does not smuggle a position through the `type` filter itself", () => {
    // `type` is the one filter that survives, so it is the one remaining channel. It is passed
    // through verbatim and URL-encoded, which is correct - this pins that it is not doing anything
    // else, and that a caller stuffing coordinates into it is visible rather than hidden.
    void client().listBusinesses({ type: "vet & groomer" });
    expect(soleUrl()).toBe("https://central.test/v1/businesses?type=vet+%26+groomer");
  });

  it("omits the query string entirely for an empty or all-absent query", () => {
    void client().listBusinesses({});
    expect(soleUrl()).toBe("https://central.test/v1/businesses");
  });

  it("is an unauthenticated request, so nothing sent here is protected by a session", () => {
    // Stated as a test because it is the reason the parameter mattered: this route carries no
    // bearer, so a position sent on it is attributable only to an IP.
    const c = createCentralClient({
      baseUrl: "https://central.test",
      getAdminToken: () => "admin-token-should-not-be-sent",
    });
    void c.listBusinesses();
    const mock = globalThis.fetch as unknown as { mock: { calls: unknown[][] } };
    const init = mock.mock.calls[0]?.[1] as RequestInit | undefined;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    expect(headers.authorization).toBeUndefined();
  });
});
