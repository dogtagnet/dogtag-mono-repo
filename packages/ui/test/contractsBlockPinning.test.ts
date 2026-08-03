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
// The generation probe is a RAW `call`, deliberately - see `answeredWithExecutionRevert`. Stubbing it
// separately from `readContract` is what lets these cases pin WHICH of the two the probe uses.
const call = vi.fn(async () => ({ data: `0x${"0".repeat(63)}1` }));

vi.mock("viem", async (importActual) => {
  const actual = await importActual<typeof import("viem")>();
  return { ...actual, createPublicClient: () => ({ readContract, getLogs, call }) };
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
  issuerRegistryOf,
  rootIssuedAtLog,
  whitelistGrantHistory,
  sortLogPoints,
  UNPOSITIONED_LOG,
  RIGHT_ISSUE,
} = await import("../src/wallet/contracts");
const { roaxIssuerChainReader } = await import("../src/wallet/verifyCredential");

const ADDR = "0x0000000000000000000000000000000000000001";
const SIGNER = "0x2222222222222222222222222222222222222222" as const;
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

  it("whitelistGrantHistory pins toBlock on its grant query", async () => {
    await whitelistGrantHistory({
      registryAddr: ADDR,
      service: ADDR,
      signer: ADDR,
      rpcUrl: url(),
      toBlock: AT,
    });
    // Grant and withdrawal share one topic (`allowed` is the non-indexed argument), so this is a
    // SINGLE `eth_getLogs` - an unbounded one would admit a withdrawal from beyond the report's
    // own height.
    expect(getLogs.mock.calls.length).toBe(1);
    for (const [call] of getLogs.mock.calls) {
      expect((call as unknown as Record<string, unknown>).toBlock).toBe(AT);
    }
  });

  /**
   * The filter carries the ACCOUNT and no service, because `RightsSet` is indexed on the account
   * alone. Asserted on what reaches `getLogs` rather than on the returned history: a reader that
   * silently kept narrowing by service would still return the right answer for a fixture where both
   * addresses are the same, which is exactly the shape of this test's inputs.
   */
  it("whitelistGrantHistory asks about the ACCOUNT alone, indexed - no service narrows it", async () => {
    await whitelistGrantHistory({ registryAddr: ADDR, service: ADDR, signer: SIGNER, rpcUrl: url() });
    for (const [call] of getLogs.mock.calls) {
      const c = call as unknown as Record<string, unknown>;
      expect(c.address).toBe(ADDR);
      expect(c.args).toEqual({ account: SIGNER });
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

  // `rights` is the account's whole settable mask, so the ISSUE bit is read out of it rather than
  // arriving as a bool of its own.
  const grant = (allowed: boolean, pos: Record<string, unknown>) => ({
    ...pos,
    args: { rights: allowed ? RIGHT_ISSUE : 0n },
  });

  it("whitelistGrantHistory: an unpositioned GRANT log makes the whole history unorderable", async () => {
    getLogs.mockResolvedValueOnce([grant(true, pending)]);
    const h = await whitelistGrantHistory({
      registryAddr: ADDR,
      service: ADDR,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(h).toBe(UNPOSITIONED_LOG);
  });

  it("whitelistGrantHistory: an unpositioned WITHDRAWAL trips it too", async () => {
    // Grant and withdrawal arrive on ONE topic, so a guard that only looked at the first log would
    // leave the withdrawal - the event whose loss flips an answer to authorised - unguarded.
    getLogs.mockResolvedValueOnce([grant(true, positioned), grant(false, pending)]);
    const h = await whitelistGrantHistory({
      registryAddr: ADDR,
      service: ADDR,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(h).toBe(UNPOSITIONED_LOG);
  });

  it("whitelistGrantHistory: positioned logs still fold, and an empty log is still an empty history", async () => {
    // The control. Without it a reader that returned the sentinel unconditionally would pass above.
    getLogs.mockResolvedValueOnce([grant(true, positioned)]);
    const granted = await whitelistGrantHistory({
      registryAddr: ADDR,
      service: ADDR,
      signer: ADDR,
      rpcUrl: url(),
    });
    expect(granted).toEqual([{ kind: "whitelisted", ...positioned }]);

    getLogs.mockResolvedValueOnce([]);
    const none = await whitelistGrantHistory({
      registryAddr: ADDR,
      service: ADDR,
      signer: ADDR,
      rpcUrl: url(),
    });
    // An EMPTY history keeps meaning what it meant: the authority answered and recorded no grant.
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
