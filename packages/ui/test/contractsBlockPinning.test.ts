// The block-pinning plumbing, pinned by capturing what the readers actually hand viem.
//
// Against a MUTABLE world an unpinned verdict is not reproducible: clones get superseded and domain
// claims get rewritten, so "this root resolved to clone X" only means something with a height beside
// it. Typecheck alone would accept a `blockNumber` that is declared, threaded halfway, and then
// dropped before `readContract` - which reads as pinned everywhere except where it counts. These tests
// assert the value reaches the eth_call.
import { beforeEach, describe, expect, it, vi } from "vitest";

const readContract = vi.fn(async () => "0x0000000000000000000000000000000000000001");
const getLogs = vi.fn(async () => [] as unknown[]);

vi.mock("viem", async (importActual) => {
  const actual = await importActual<typeof import("viem")>();
  return { ...actual, createPublicClient: () => ({ readContract, getLogs }) };
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
  issuerRegistryOf,
  rootIssuedAtLog,
  whitelistGrantHistory,
  sortLogPoints,
  UNPOSITIONED_LOG,
} = await import("../src/wallet/contracts");
const { roaxIssuerChainReader } = await import("../src/wallet/verifyCredential");

const ADDR = "0x0000000000000000000000000000000000000001";
const ROOT = `0x${"ab".repeat(32)}`;
const AT = 900_100n;

/** A distinct RPC url per call: `roaxPublicClient` caches by url, and a reused entry would hide drift. */
let seq = 0;
const url = () => `http://pin-${++seq}.invalid`;

const lastCall = () => readContract.mock.calls.at(-1)?.[0] as unknown as Record<string, unknown>;
const lastLogQuery = () => getLogs.mock.calls.at(-1)?.[0] as unknown as Record<string, unknown>;

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
    ["issuerRegistryOf", (blockNumber) => issuerRegistryOf({ issuerAddr: ADDR, rpcUrl: url(), blockNumber })],
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

/**
 * The two LOG readers bound the same way, via `toBlock`.
 *
 * They answer the historical whitelist question, so an unbounded upper end would let a grant recorded
 * after the report's block leak into an answer the report claims to have pinned - a verdict that
 * cannot be reproduced at the height it names.
 */
describe("the log readers bound their range to the report's block", () => {
  const lastLogCall = () => getLogs.mock.calls.at(-1)?.[0] as unknown as Record<string, unknown>;
  beforeEach(() => getLogs.mockClear());

  it("rootIssuedAtLog pins toBlock", async () => {
    await rootIssuedAtLog({ issuerAddr: ADDR, root: ROOT, rpcUrl: url(), toBlock: AT });
    expect(lastLogCall()?.toBlock).toBe(AT);
    expect(lastLogCall()?.fromBlock).toBe(0n);
  });

  it("rootIssuedAtLog reads to latest when given no bound", async () => {
    await rootIssuedAtLog({ issuerAddr: ADDR, root: ROOT, rpcUrl: url() });
    // Omitted must stay OMITTED rather than becoming an explicit `undefined` viem would forward.
    expect("toBlock" in (lastLogCall() ?? {})).toBe(false);
  });

  it("whitelistGrantHistory pins toBlock on BOTH event queries", async () => {
    await whitelistGrantHistory({
      registryAddr: ADDR,
      recordTypeKey: ROOT,
      signer: ADDR,
      rpcUrl: url(),
      toBlock: AT,
    });
    // Whitelisted and Delisted are two separate `eth_getLogs`; a bound on only one would silently
    // admit a delisting from beyond the report's own height.
    expect(getLogs.mock.calls.length).toBe(2);
    for (const [call] of getLogs.mock.calls) {
      expect((call as unknown as Record<string, unknown>).toBlock).toBe(AT);
    }
  });

  it("whitelistGrantHistory asks about exactly the (recordType, signer) pair, indexed", async () => {
    await whitelistGrantHistory({ registryAddr: ADDR, recordTypeKey: ROOT, signer: ADDR, rpcUrl: url() });
    for (const [call] of getLogs.mock.calls) {
      const c = call as unknown as Record<string, unknown>;
      expect(c.address).toBe(ADDR);
      expect(c.args).toEqual({ recordType: ROOT, signer: ADDR });
    }
  });
});

/**
 * A log the node returned with NO `(blockNumber, logIndex)` - viem's shape for one it considers
 * pending - must reach the caller as `UNPOSITIONED_LOG`, never as a position.
 *
 * These pin the READER half, and it is the half the original defect lived in: `l.blockNumber ?? 0n`
 * placed such a log at the very start of the chain, which sorts before every anchoring and could turn
 * a delisted-before into an authorised. The consumer half - that the pillar answers indeterminate when
 * handed the sentinel - is pinned in `verifyCredential.test.ts` and `verificationBench.test.ts`, whose
 * fakes bypass these functions entirely and so cannot see a coercion reintroduced here.
 */
describe("a log with no position is reported as such, never placed", () => {
  const positioned = { blockNumber: 700n, logIndex: 2 };
  const pending = { blockNumber: null, logIndex: null };
  beforeEach(() => getLogs.mockClear());

  it("whitelistGrantHistory: an unpositioned WHITELISTED log makes the whole history unorderable", async () => {
    getLogs.mockResolvedValueOnce([pending]).mockResolvedValueOnce([]);
    const h = await whitelistGrantHistory({
      registryAddr: ADDR,
      recordTypeKey: ROOT,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(h).toBe(UNPOSITIONED_LOG);
  });

  it("whitelistGrantHistory: an unpositioned DELISTED log trips it too", async () => {
    // The two events are separate `eth_getLogs`, so a guard applied to only one leaves the delisting -
    // the event whose loss flips an answer to authorised - unguarded. Both sets are checked.
    getLogs.mockResolvedValueOnce([positioned]).mockResolvedValueOnce([pending]);
    const h = await whitelistGrantHistory({
      registryAddr: ADDR,
      recordTypeKey: ROOT,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(h).toBe(UNPOSITIONED_LOG);
  });

  it("whitelistGrantHistory: positioned logs still fold, and an empty log is still an empty history", async () => {
    // The control. Without it a reader that returned the sentinel unconditionally would pass above.
    getLogs.mockResolvedValueOnce([positioned]).mockResolvedValueOnce([]);
    const granted = await whitelistGrantHistory({
      registryAddr: ADDR,
      recordTypeKey: ROOT,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(granted).toEqual([{ kind: "whitelisted", ...positioned }]);

    getLogs.mockResolvedValueOnce([]).mockResolvedValueOnce([]);
    const none = await whitelistGrantHistory({
      registryAddr: ADDR,
      recordTypeKey: ROOT,
      signer: ADDR,
      rpcUrl: url(),
    });
    // An EMPTY history keeps meaning what it meant: the registry answered and recorded no grant.
    expect(none).toEqual([]);
  });

  it("rootIssuedAtLog: an unpositioned anchoring log is NOT block 0, and NOT 'no log'", async () => {
    getLogs.mockResolvedValueOnce([pending]);
    const at = await rootIssuedAtLog({ issuerAddr: ADDR, root: ROOT, rpcUrl: url() });
    expect(at).toBe(UNPOSITIONED_LOG);
    expect(at).not.toBeNull();
    expect(at).not.toEqual({ blockNumber: 0n, logIndex: 0 });
  });

  it("rootIssuedAtLog: a positioned log still answers, and no log at all is still null", async () => {
    getLogs.mockResolvedValueOnce([positioned]);
    expect(await rootIssuedAtLog({ issuerAddr: ADDR, root: ROOT, rpcUrl: url() })).toEqual(
      positioned,
    );
    getLogs.mockResolvedValueOnce([]);
    // `null` says the chain emitted nothing - a different fact from a log we could not place.
    expect(await rootIssuedAtLog({ issuerAddr: ADDR, root: ROOT, rpcUrl: url() })).toBeNull();
  });
});

describe("sortLogPoints", () => {
  it("orders by block, then by logIndex WITHIN a block", () => {
    // `logIndex` is block-scoped, so it is only a tiebreak - comparing it across blocks would put a
    // later block's first log before an earlier block's second.
    const points = [
      { blockNumber: 5n, logIndex: 1 },
      { blockNumber: 3n, logIndex: 9 },
      { blockNumber: 5n, logIndex: 0 },
      { blockNumber: 3n, logIndex: 2 },
    ];
    expect(sortLogPoints(points)).toEqual([
      { blockNumber: 3n, logIndex: 2 },
      { blockNumber: 3n, logIndex: 9 },
      { blockNumber: 5n, logIndex: 0 },
      { blockNumber: 5n, logIndex: 1 },
    ]);
  });

  it("does not mutate its input", () => {
    const points = [
      { blockNumber: 9n, logIndex: 0 },
      { blockNumber: 1n, logIndex: 0 },
    ];
    sortLogPoints(points);
    expect(points[0]?.blockNumber).toBe(9n);
  });
});

describe("roaxIssuerChainReader", () => {
  it("pins every eth_call it makes to the block the bench gave it", async () => {
    const reader = roaxIssuerChainReader(url(), AT);
    await reader.rootIssuer(ADDR, ROOT);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.isValid(ADDR, ROOT);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.recordType(ADDR);
    expect(lastCall()?.blockNumber).toBe(AT);
    await reader.issuerRegistry(ADDR);
    expect(lastCall()?.blockNumber).toBe(AT);
  });

  it("bounds its LOG reads by the same block, so the pillar is one snapshot", async () => {
    // The pillar's answer is a fold over these two, so an unbounded log read would let a grant landing
    // mid-verification change a verdict printed under an earlier block - the anchor beside it would
    // then be a claim about reads that did not all happen there.
    getLogs.mockClear();
    const reader = roaxIssuerChainReader(url(), AT);
    await reader.rootIssuedAt(ADDR, ROOT);
    expect(lastLogQuery()?.toBlock, "the anchoring log read dropped toBlock").toBe(AT);
    await reader.grantHistory(ADDR, ROOT, ADDR);
    // Whitelisted and Delisted are two queries; BOTH must carry the bound.
    for (const call of getLogs.mock.calls.slice(-2)) {
      expect((call[0] as Record<string, unknown>)?.toBlock).toBe(AT);
    }
  });

  it("reads to latest when the head could not be pinned", async () => {
    getLogs.mockClear();
    const reader = roaxIssuerChainReader(url(), undefined);
    await reader.rootIssuedAt(ADDR, ROOT);
    expect(lastLogQuery()).not.toHaveProperty("toBlock");
  });
});
