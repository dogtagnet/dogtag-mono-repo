import { createServer, type Server } from "node:http";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
// Imported through the package barrel on purpose: the directory seam's re-export in `src/index.ts`
// is what a portal actually consumes, and no other test loads it.
import {
  centralDirectory,
  createCentralClient,
  createMemoryProviderDirectoryCache,
  onchainDirectory,
  withProviderDirectoryCache,
  type ProviderDirectoryResult,
} from "../src/index";

/**
 * The seam over a REAL HTTP boundary.
 *
 * Every other provider-directory test injects `listBusinesses`, so nothing yet proved what actually
 * leaves the process. This drives `createCentralClient -> centralDirectory ->
 * withProviderDirectoryCache` against a `node:http` server that records every request line, which is
 * the only place the position-free claim can be observed rather than asserted about a mock.
 */

type ServerMode = "two" | "empty" | "error" | "malformed";

const BUSINESSES = [
  {
    businessId: "biz-north-star",
    type: "vet",
    name: "North Star Veterinary",
    geo: { lat: 1.3521, lng: 103.8198 },
    services: ["vaccination", "microchip"],
    apiBaseUrl: "https://north-star.test/api",
    domain: "north-star.test",
    documentStores: ["https://north-star.test/documents"],
    hmacKeyId: "hmac-north-star",
  },
  {
    businessId: "biz-good-dog",
    type: "groomer",
    name: "Good Dog Grooming",
    geo: { lat: 1.3039, lng: 103.8318 },
    services: ["grooming"],
    apiBaseUrl: "https://good-dog.test/api",
    domain: "good-dog.test",
    documentStores: [],
    hmacKeyId: "hmac-good-dog",
  },
];

const TTL_MS = 60_000;
const T0 = 1_800_000_000_000;

const requestLines: string[] = [];
const transcript: string[] = [];
let mode: ServerMode = "two";
let server: Server;
let baseUrl = "";

function say(line: string): void {
  transcript.push(line);
}

function describeResult(label: string, result: ProviderDirectoryResult): void {
  if (result.state === "unavailable") {
    say(
      `${label}\n` +
        `    state=unavailable  source=${result.source}  reason=${result.reason}\n` +
        `    "providers" in result -> ${"providers" in result}   (no empty list to render as "no providers")\n` +
        `    detail: ${result.detail}`,
    );
    return;
  }
  const names = result.providers.map((p) => `${p.name} [${p.kind}] active=${String(p.active)}`);
  say(
    `${label}\n` +
      `    state=${result.state}  source=${result.source}  observation=${result.observation}\n` +
      `    blockNumber=${String(result.blockNumber)}  readAt=${result.readAt}  expiresAt=${String(result.expiresAt)}\n` +
      `    providers(${result.providers.length}): ${names.length ? names.join(" | ") : "(none)"}`,
  );
}

beforeAll(async () => {
  server = createServer((req, res) => {
    requestLines.push(`${req.method} ${req.url}`);
    if (mode === "error") {
      res.writeHead(500, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "central directory is down" }));
      return;
    }
    if (mode === "malformed") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ businesses: [{ businessId: "biz-broken" }] }));
      return;
    }
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ businesses: mode === "two" ? BUSINESSES : [] }));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("no server port");
  baseUrl = `http://127.0.0.1:${address.port}`;
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
  // eslint-disable-next-line no-console
  console.log(`\n${transcript.join("\n")}\n`);
});

describe("provider directory over a real HTTP boundary", () => {
  it("drives found -> stored replay -> expiry -> empty, and never sends a position", async () => {
    let clock = T0;
    const cache = createMemoryProviderDirectoryCache();
    const directory = withProviderDirectoryCache(
      centralDirectory(createCentralClient({ baseUrl }), { now: () => clock }),
      { cache, ttlMs: TTL_MS, now: () => clock },
    );

    say(`ProviderDirectory over real HTTP  (central base ${baseUrl}, hard TTL ${TTL_MS} ms)`);
    say(`  cacheNamespace: ${directory.cacheNamespace}`);
    say("");

    // 1. live read
    mode = "two";
    const live = await directory.read();
    describeResult(`[t=+0ms]      central up, 2 businesses`, live);
    expect(live.state).toBe("found");
    if (live.state !== "found") throw new Error("unreachable");
    expect(live.observation).toBe("live");
    expect(live.blockNumber).toBeNull();
    expect(live.readAt).toBe(T0);
    expect(live.expiresAt).toBe(T0 + TTL_MS);
    expect(live.providers.map((p) => p.providerId)).toEqual(["biz-north-star", "biz-good-dog"]);
    // Central-only wire fields never reach a provider, and no delisting fact is invented.
    expect(Object.keys(live.providers[0]).sort()).toEqual([
      "active",
      "bindingState",
      "contact",
      "domain",
      "geo",
      "kind",
      "name",
      "providerId",
      "services",
    ]);
    expect(live.providers.every((p) => p.active === null)).toBe(true);
    expect(live.providers.every((p) => p.bindingState === "unavailable")).toBe(true);
    expect(live.providers.every((p) => Object.values(p.contact).every((v) => v === null))).toBe(
      true,
    );
    say("");

    // 2. source down mid-window -> stored replay, deadline untouched
    mode = "error";
    clock = T0 + TTL_MS / 2;
    const replayed = await directory.read();
    describeResult(`[t=+${TTL_MS / 2}ms]  central returns HTTP 500`, replayed);
    expect(replayed.state).toBe("found");
    if (replayed.state !== "found") throw new Error("unreachable");
    expect(replayed.observation).toBe("stored");
    expect(replayed.source).toBe("central");
    expect(replayed.readAt).toBe(T0);
    expect(replayed.expiresAt).toBe(T0 + TTL_MS);
    say("");

    // 3. a second replay later still carries the ORIGINAL deadline: stored data never renews
    clock = T0 + TTL_MS - 1;
    const replayedAgain = await directory.read();
    describeResult(`[t=+${TTL_MS - 1}ms]  central still down`, replayedAgain);
    expect(replayedAgain.state).toBe("found");
    if (replayedAgain.state !== "found") throw new Error("unreachable");
    expect(replayedAgain.observation).toBe("stored");
    expect(replayedAgain.readAt).toBe(T0);
    expect(replayedAgain.expiresAt).toBe(T0 + TTL_MS);
    say(
      "    ^ same readAt and expiresAt as the live read: a replay never renews the hard deadline",
    );
    say("");

    // 4. the exact deadline is expired, and expiry reports unavailable rather than "no providers"
    clock = T0 + TTL_MS;
    const expired = await directory.read();
    describeResult(`[t=+${TTL_MS}ms]  exact hard-TTL boundary, central still down`, expired);
    expect(expired.state).toBe("unavailable");
    expect("providers" in expired).toBe(false);
    say("");

    // 5. a genuinely empty directory is a SUCCESSFUL read, and is a different state
    mode = "empty";
    clock = T0 + TTL_MS + 1_000;
    const empty = await directory.read();
    describeResult(`[t=+${TTL_MS + 1_000}ms]  central up, registry genuinely empty`, empty);
    expect(empty.state).toBe("empty");
    if (empty.state !== "empty") throw new Error("unreachable");
    expect(empty.providers).toEqual([]);
    expect(empty.observation).toBe("live");
    say("");

    // 6. a malformed body is never degraded into that same empty. Read straight from the source, so
    //    the still-valid empty snapshot just written above cannot be mistaken for the answer.
    mode = "malformed";
    clock = T0 + TTL_MS + 2_000;
    const uncached = centralDirectory(createCentralClient({ baseUrl }), { now: () => clock });
    const malformed = await uncached.read();
    describeResult(
      `[t=+${TTL_MS + 2_000}ms]  central returns a malformed row (uncached read)`,
      malformed,
    );
    expect(malformed.state).toBe("unavailable");
    if (malformed.state !== "unavailable") throw new Error("unreachable");
    expect(malformed.reason).toBe("malformedResponse");
    say("");

    // The wire evidence: every request is the bare full-set path.
    say("Request lines the central server actually received:");
    for (const line of requestLines) say(`    ${line}`);
    expect(requestLines.length).toBeGreaterThan(0);
    expect(new Set(requestLines)).toEqual(new Set(["GET /v1/businesses"]));
    expect(requestLines.every((line) => !line.includes("?"))).toBe(true);
    for (const forbidden of ["near", "radius", "lat", "lng", "bbox", "viewport", "geohash"]) {
      expect(requestLines.some((line) => line.includes(forbidden))).toBe(false);
    }
    say(
      "    -> no query string at all: no near, radius, lat, lng, bbox, viewport or geohash parameter",
    );
    say("");
  });

  it("resolves the on-chain stub unavailable without performing any network call", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    const before = requestLines.length;
    const result = await onchainDirectory({ now: () => T0 }).read();
    describeResult("[on-chain source]  stub, no registry deployed", result);
    expect(result.state).toBe("unavailable");
    if (result.state !== "unavailable") throw new Error("unreachable");
    expect(result.reason).toBe("providerRegistryUnavailable");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(requestLines.length).toBe(before);
    say("    fetch calls made by the stub: 0   (it asserts nothing about a registry it cannot read)");
    say("");
    fetchSpy.mockRestore();
  });

  it("exposes no query parameter on read(), and no map/viewport surface", async () => {
    const directory = centralDirectory(createCentralClient({ baseUrl }));
    expect(directory.read.length).toBe(0);
    expect(Object.keys(directory).sort()).toEqual(["cacheNamespace", "read", "source"]);
    say("ProviderDirectory shape:");
    say(`    read() arity = ${directory.read.length}   (nowhere to put a position or a viewport)`);
    say(`    members = ${Object.keys(directory).sort().join(", ")}`);
  });
});
