// The block-pinning plumbing, pinned by capturing what the readers actually hand viem.
//
// Against a MUTABLE world an unpinned verdict is not reproducible: clones get superseded and domain
// claims get rewritten, so "this root resolved to clone X" only means something with a height beside
// it. Typecheck alone would accept a `blockNumber` that is declared, threaded halfway, and then
// dropped before `readContract` - which reads as pinned everywhere except where it counts. These tests
// assert the value reaches the eth_call.
import { beforeEach, describe, expect, it, vi } from "vitest";

const readContract = vi.fn(async () => "0x0000000000000000000000000000000000000001");

vi.mock("viem", async (importActual) => {
  const actual = await importActual<typeof import("viem")>();
  return { ...actual, createPublicClient: () => ({ readContract }) };
});

// Imported AFTER the mock is registered so the readers bind to the stubbed client.
const {
  isRootValid,
  isRootRevoked,
  issuedAtOf,
  issuedByOf,
  rootIssuerOf,
  recordTypeOf,
  isWhitelistedFor,
  issuerDomainClaimOf,
} = await import("../src/wallet/contracts");
const { roaxIssuerChainReader } = await import("../src/wallet/verifyCredential");

const ADDR = "0x0000000000000000000000000000000000000001";
const ROOT = `0x${"ab".repeat(32)}`;
const AT = 900_100n;

/** A distinct RPC url per call: `roaxPublicClient` caches by url, and a reused entry would hide drift. */
let seq = 0;
const url = () => `http://pin-${++seq}.invalid`;

const lastCall = () => readContract.mock.calls.at(-1)?.[0] as unknown as Record<string, unknown>;

beforeEach(() => readContract.mockClear());

describe("every reader forwards blockNumber to the eth_call", () => {
  const cases: Array<[string, (blockNumber?: bigint) => Promise<unknown>]> = [
    ["isRootValid", (blockNumber) => isRootValid({ issuerAddr: ADDR, root: ROOT, rpcUrl: url(), blockNumber })],
    ["isRootRevoked", (blockNumber) => isRootRevoked({ issuerAddr: ADDR, root: ROOT, rpcUrl: url(), blockNumber })],
    ["issuedAtOf", (blockNumber) => issuedAtOf({ issuerAddr: ADDR, root: ROOT, rpcUrl: url(), blockNumber })],
    ["issuedByOf", (blockNumber) => issuedByOf({ issuerAddr: ADDR, root: ROOT, rpcUrl: url(), blockNumber })],
    ["rootIssuerOf", (blockNumber) => rootIssuerOf({ factoryAddr: ADDR, root: ROOT, rpcUrl: url(), blockNumber })],
    ["recordTypeOf", (blockNumber) => recordTypeOf({ issuerAddr: ADDR, rpcUrl: url(), blockNumber })],
    [
      "isWhitelistedFor",
      (blockNumber) =>
        isWhitelistedFor({ registryAddr: ADDR, recordTypeKey: ROOT, address: ADDR, rpcUrl: url(), blockNumber }),
    ],
  ];

  for (const [name, call] of cases) {
    it(`${name} pins the read to the block it was given`, async () => {
      await call(AT);
      expect(lastCall()?.blockNumber, `${name} dropped blockNumber before readContract`).toBe(AT);
    });

    it(`${name} reads latest when no block was given`, async () => {
      await call(undefined);
      // Omitted must stay omitted: inventing a height here would claim a snapshot never taken.
      expect(lastCall()?.blockNumber).toBeUndefined();
    });
  }

  it("issuerDomainClaimOf pins its read too", async () => {
    readContract.mockResolvedValueOnce({
      domain: "vet.example",
      updatedAt: 1n,
      updatedAtBlock: 5n,
      setBy: ADDR,
    } as never);
    await issuerDomainClaimOf({ domainRegistryAddr: ADDR, cloneAddr: ADDR, rpcUrl: url(), blockNumber: AT });
    expect(lastCall()?.blockNumber).toBe(AT);
  });

  it("treats updatedAt == 0 as 'no claim published' rather than a zero-timestamp binding", async () => {
    readContract.mockResolvedValueOnce({
      domain: "",
      updatedAt: 0n,
      updatedAtBlock: 0n,
      setBy: "0x0000000000000000000000000000000000000000",
    } as never);
    expect(
      await issuerDomainClaimOf({ domainRegistryAddr: ADDR, cloneAddr: ADDR, rpcUrl: url() }),
    ).toBeNull();
  });
});

describe("roaxIssuerChainReader", () => {
  it("pins every read it makes to the block the bench gave it", async () => {
    const reader = roaxIssuerChainReader(url(), AT);
    await reader.rootIssuer(ADDR, ROOT);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.isValid(ADDR, ROOT);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.recordType(ADDR);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.isWhitelistedFor(ADDR, ROOT, ADDR);
    expect(lastCall()?.blockNumber).toBe(AT);
  });
});
